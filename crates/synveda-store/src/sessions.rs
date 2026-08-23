//! The session ledger (CPR-10, ADR-0076): sessions, their immutable events,
//! and the context runs composed for them.
//!
//! Three tables, one module, because the three are one aggregate: a session
//! is the root, an event is a child that cannot exist without it, and a
//! context run is an act taken on it. Splitting them across modules would put
//! the transaction that appends an event and stamps its session's
//! `last_observed_at` in two places.
//!
//! ## Governance attaches above this module
//!
//! [`crate::workspaces`]'s rule, verbatim: nothing here decides anything. The
//! PDP decision, the audit event and the ownership 404 are the gateway's, and
//! this module's job is that every read is tenant-filtered in SQL as well as
//! by RLS — because several of these functions run on owner connections in
//! tests and in the reset path, where RLS does not apply.
//!
//! ## The scope is derived here, not received
//!
//! [`create`] reads the workspace (and the project, when there is one) and
//! takes their scope ids from the rows. No caller supplies one. Migration
//! `0044`'s composite keys hold the same rule at the row, so a caller that
//! reached the table another way still cannot anchor a session at a scope its
//! workspace is not in.
//!
//! ## Appends are serialised per session, deliberately
//!
//! [`append_events`] locks its session row before it reads `max(sequence)`.
//! The alternative — computing the next position optimistically and retrying
//! the unique violation — is more code, is only faster when two clients append
//! to *one* session at once, and has to get the retry right to be correct at
//! all. One `select … for update` per batch is the boring version, and the
//! contention it can produce is contention between two writers to the same
//! agent run, which is not a case worth optimising.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor};
use synveda_types::session::{
    ContextRun, MAX_END_REASON_CHARS, MAX_EVENT_PAYLOAD_BYTES, MAX_LABEL_CHARS, MAX_METADATA_BYTES,
    MAX_TASK_SUMMARY_CHARS, Session, SessionEvent, SessionEventType, SessionStatus,
    validate_client_event_id, validate_client_name, validate_json_object, validate_label,
};
use synveda_types::{
    ContextRunId, Error, ProjectId, RepositoryId, Result, ScopeId, SessionEventId, SessionId,
    TenantId, WorkspaceId,
};
use uuid::Uuid;

use crate::workspaces::storage_error;

/// Counter: session-ledger mutations, labelled `table` (`session`, `event`,
/// `context_run`) and `operation` (`create`, `append`, `duplicate`, `end`).
/// Emitted here, described by the gateway where the recorder lives (ADR-0007).
pub const SESSION_MUTATIONS_TOTAL: &str = "synveda_session_mutations_total";

/// The most rows a session listing will consider before the answer is
/// declared truncated.
///
/// A bound rather than a page size: the caller's own `limit` is applied to the
/// rows they may **read**, and this is how many candidates the store will look
/// at to find them. It exists because sessions are the first table on this
/// plane whose volume is not bounded by anything a person does — a fleet of
/// agents writes rows all night — and an unbounded per-row decision over that
/// is a request that gets slower every week.
pub const SCAN_LIMIT: i64 = 500;

/// The most events one `POST …/events` may carry.
///
/// Bounded because the whole batch is one transaction holding one session's
/// row lock, and because a client that wants to deliver ten thousand events
/// should be told to do it in batches rather than discovering the limit as a
/// timeout.
pub const MAX_EVENT_BATCH: usize = 200;

// ── Sessions ─────────────────────────────────────────────────────────────────

/// What [`create`] needs.
///
/// There is deliberately no `tenant_id` from the caller's body and no
/// `principal_id` a client can choose (ADR-0076 decision 8): the gateway fills
/// both from the verified token, and this struct is what it fills.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// The session's identity.
    pub id: SessionId,
    /// Owning tenant, from the resolved token.
    pub tenant_id: TenantId,
    /// The workspace the run is in. Must exist and be active.
    pub workspace_id: WorkspaceId,
    /// The project, when the run is against one. Must be in that workspace
    /// and active.
    pub project_id: Option<ProjectId>,
    /// The token subject opening it.
    pub principal_id: String,
    /// The agent client, as it names itself.
    pub client_name: String,
    /// Its version, when it says one.
    pub client_version: Option<String>,
    /// A stable id for this installation of that client.
    pub client_installation_id: Option<String>,
    /// The harness's own id for this run.
    pub external_session_id: Option<String>,
    /// Which agent ran.
    pub agent_name: Option<String>,
    /// The model, as the client names it.
    pub model_name: Option<String>,
    /// A repository attached to the named project.
    pub repository_id: Option<RepositoryId>,
    /// The branch the run is on.
    pub branch: Option<String>,
    /// What the run is about.
    pub task_summary: Option<String>,
    /// The client's labelling bag.
    pub metadata: serde_json::Value,
}

struct SessionRow {
    id: Uuid,
    tenant_id: Uuid,
    workspace_id: Uuid,
    project_id: Option<Uuid>,
    scope_id: Uuid,
    principal_id: String,
    client_name: String,
    client_version: Option<String>,
    client_installation_id: Option<String>,
    external_session_id: Option<String>,
    agent_name: Option<String>,
    model_name: Option<String>,
    repository_id: Option<Uuid>,
    branch: Option<String>,
    task_summary: Option<String>,
    status: String,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    last_observed_at: Option<DateTime<Utc>>,
    end_reason: Option<String>,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<SessionRow> for Session {
    type Error = Error;

    fn try_from(row: SessionRow) -> Result<Self> {
        Ok(Session {
            id: SessionId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            workspace_id: WorkspaceId::from_uuid(row.workspace_id),
            project_id: row.project_id.map(ProjectId::from_uuid),
            scope_id: ScopeId::from_uuid(row.scope_id),
            principal_id: row.principal_id,
            client_name: row.client_name,
            client_version: row.client_version,
            client_installation_id: row.client_installation_id,
            external_session_id: row.external_session_id,
            agent_name: row.agent_name,
            model_name: row.model_name,
            repository_id: row.repository_id.map(RepositoryId::from_uuid),
            branch: row.branch,
            task_summary: row.task_summary,
            status: row.status.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            started_at: row.started_at,
            ended_at: row.ended_at,
            last_observed_at: row.last_observed_at,
            end_reason: row.end_reason,
            metadata: row.metadata,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn not_found(id: SessionId) -> Error {
    Error::NotFound {
        entity: format!("session {id}"),
    }
}

/// Validates the parts of a new session this crate owns.
fn validate(new: &NewSession) -> Result<()> {
    validate_client_name(&new.client_name)?;
    validate_label(
        "client_version",
        new.client_version.as_deref(),
        MAX_LABEL_CHARS,
    )?;
    validate_label(
        "client_installation_id",
        new.client_installation_id.as_deref(),
        MAX_LABEL_CHARS,
    )?;
    validate_label(
        "external_session_id",
        new.external_session_id.as_deref(),
        MAX_LABEL_CHARS,
    )?;
    validate_label("agent_name", new.agent_name.as_deref(), MAX_LABEL_CHARS)?;
    validate_label("model_name", new.model_name.as_deref(), MAX_LABEL_CHARS)?;
    validate_label("branch", new.branch.as_deref(), MAX_LABEL_CHARS)?;
    validate_label(
        "task_summary",
        new.task_summary.as_deref(),
        MAX_TASK_SUMMARY_CHARS,
    )?;
    validate_json_object("metadata", &new.metadata, MAX_METADATA_BYTES)?;
    Ok(())
}

/// Opens a session in the caller's transaction, resolving its governed scope
/// from the workspace and project it names.
///
/// The workspace must exist in this tenant and be **active**; so must the
/// project, when one is named, and the project must be in that workspace. An
/// archived workspace or project is one somebody retired, and accepting new
/// runs into it would make the retirement advisory.
///
/// # Errors
///
/// [`Error::NotFound`] for a workspace, project or repository that is not this
/// tenant's; [`Error::Conflict`] for an archived one or a duplicate harness
/// reference; [`Error::Invalid`] for a field outside its bounds.
#[tracing::instrument(
    name = "store.sessions.create",
    skip_all,
    fields(
        tenant.id = %new.tenant_id,
        session.id = %new.id,
        workspace.id = %new.workspace_id,
        scope.id = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn create(conn: &mut PgConnection, new: &NewSession) -> Result<Session> {
    validate(new)?;

    let workspace = crate::workspaces::get(&mut *conn, new.tenant_id, new.workspace_id)
        .await?
        .ok_or_else(|| Error::NotFound {
            entity: format!("workspace {}", new.workspace_id),
        })?;
    if !workspace.status.is_active() {
        return Err(Error::Conflict {
            message: format!(
                "workspace {} is {}; a session cannot be opened in it",
                workspace.slug, workspace.status
            ),
        });
    }

    let project = match new.project_id {
        Some(project_id) => {
            let project = crate::projects::get(&mut *conn, new.tenant_id, project_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("project {project_id}"),
                })?;
            if project.workspace_id != new.workspace_id {
                // A 404 rather than a 409: from the caller's side this project
                // is not in the workspace they named, and saying which
                // workspace it *is* in would answer a question they did not
                // ask about a row they may not be able to read.
                return Err(Error::NotFound {
                    entity: format!("project {project_id} in workspace {}", new.workspace_id),
                });
            }
            if !project.status.is_active() {
                return Err(Error::Conflict {
                    message: format!(
                        "project {} is {}; a session cannot be opened in it",
                        project.slug, project.status
                    ),
                });
            }
            Some(project)
        }
        None => None,
    };
    if new.repository_id.is_some() && project.is_none() {
        return Err(Error::Invalid {
            message: "a repository belongs to a project, so a session naming one names a \
                      project too"
                .to_owned(),
        });
    }

    let scope_id = project
        .as_ref()
        .map_or(workspace.scope_id, |project| project.scope_id);
    tracing::Span::current().record("scope.id", tracing::field::display(scope_id));

    let row = sqlx::query_as!(
        SessionRow,
        r#"
        insert into sessions
            (id, tenant_id, workspace_id, project_id, workspace_scope_id,
             project_scope_id, scope_id, principal_id, client_name, client_version,
             client_installation_id, external_session_id, agent_name, model_name,
             repository_id, branch, task_summary, metadata)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                $15, $16, $17, $18)
        returning id, tenant_id, workspace_id, project_id, scope_id, principal_id,
                  client_name, client_version, client_installation_id,
                  external_session_id, agent_name, model_name, repository_id,
                  branch, task_summary, status, started_at, ended_at,
                  last_observed_at, end_reason, metadata, created_at, updated_at
        "#,
        new.id.as_uuid(),
        new.tenant_id.as_uuid(),
        new.workspace_id.as_uuid(),
        new.project_id.map(|id| id.as_uuid()) as Option<Uuid>,
        workspace.scope_id.as_uuid(),
        project.as_ref().map(|project| project.scope_id.as_uuid()) as Option<Uuid>,
        scope_id.as_uuid(),
        new.principal_id,
        new.client_name,
        new.client_version.as_deref() as Option<&str>,
        new.client_installation_id.as_deref() as Option<&str>,
        new.external_session_id.as_deref() as Option<&str>,
        new.agent_name.as_deref() as Option<&str>,
        new.model_name.as_deref() as Option<&str>,
        new.repository_id.map(|id| id.as_uuid()) as Option<Uuid>,
        new.branch.as_deref() as Option<&str>,
        new.task_summary.as_deref() as Option<&str>,
        new.metadata,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        SESSION_MUTATIONS_TOTAL,
        "table" => "session",
        "operation" => "create",
    )
    .increment(1);
    row.try_into()
}

/// Fetches one session.
#[tracing::instrument(
    name = "store.sessions.get",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %id),
    err(Display)
)]
pub async fn get(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: SessionId,
) -> Result<Option<Session>> {
    let row = sqlx::query_as!(
        SessionRow,
        r#"
        select id, tenant_id, workspace_id, project_id, scope_id, principal_id,
               client_name, client_version, client_installation_id,
               external_session_id, agent_name, model_name, repository_id,
               branch, task_summary, status, started_at, ended_at,
               last_observed_at, end_reason, metadata, created_at, updated_at
        from sessions
        where id = $1 and tenant_id = $2
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Finds the session a harness already opened for this run, if any.
///
/// The lookup `sessions_external_unique` exists for: a stateless hook holding
/// only the harness's own id, on a machine that has forgotten everything else.
#[tracing::instrument(
    name = "store.sessions.by_external_ref",
    skip_all,
    fields(tenant.id = %tenant_id),
    err(Display)
)]
pub async fn by_external_ref(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    principal_id: &str,
    client_name: &str,
    external_session_id: &str,
) -> Result<Option<Session>> {
    let row = sqlx::query_as!(
        SessionRow,
        r#"
        select id, tenant_id, workspace_id, project_id, scope_id, principal_id,
               client_name, client_version, client_installation_id,
               external_session_id, agent_name, model_name, repository_id,
               branch, task_summary, status, started_at, ended_at,
               last_observed_at, end_reason, metadata, created_at, updated_at
        from sessions
        where tenant_id = $1
          and principal_id = $2
          and client_name = $3
          and external_session_id = $4
        "#,
        tenant_id.as_uuid(),
        principal_id,
        client_name,
        external_session_id,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// Where a page of the listing resumes (CPR-11, ADR-0077 decision 1).
///
/// A **keyset**, not an offset: the listing's order is `(started_at desc, id
/// desc)`, and this is the last key of the previous page, so the next one is
/// "everything strictly after it in that order". An offset would re-count rows
/// on every page and skip or repeat whenever a run was opened between two
/// requests — which, on a table a fleet of agents writes to all night, is
/// every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCursor {
    /// The last row's `started_at`.
    pub started_at: DateTime<Utc>,
    /// Its id, which breaks ties within one instant.
    pub id: SessionId,
}

/// What a listing filters by. Every field is optional; all of them narrow.
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    /// Only sessions at or under this scope — the subtree, through
    /// `scope_closure`, so a workspace's scope lists its projects' sessions
    /// with no second query and no fan-out.
    pub scope_id: Option<ScopeId>,
    /// Only sessions in this workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// Only sessions in this project.
    pub project_id: Option<ProjectId>,
    /// Only sessions in this state.
    pub status: Option<SessionStatus>,
    /// Only sessions opened by this agent client, matched exactly — the label
    /// the client named itself with, never a prefix.
    pub client_name: Option<String>,
    /// Only sessions opened by this token subject.
    pub principal_id: Option<String>,
    /// Only sessions started at or after this instant.
    pub started_after: Option<DateTime<Utc>>,
    /// Only sessions started strictly before this instant.
    ///
    /// Half-open on purpose: `[after, before)` composes, so two adjacent days
    /// cover every run exactly once and a run started on the stroke of
    /// midnight is in one of them rather than in both.
    pub started_before: Option<DateTime<Utc>>,
    /// Where to resume. `None` starts at the newest.
    pub after: Option<SessionCursor>,
}

/// The candidate sessions a listing should decide about, newest first, with
/// whether the scan bound cut the answer short.
///
/// Returns candidates rather than results: which of them this caller may read
/// is a PDP question, and this crate decides nothing (seed §2.4).
#[tracing::instrument(
    name = "store.sessions.list",
    skip_all,
    fields(tenant.id = %tenant_id, sessions = tracing::field::Empty),
    err(Display)
)]
pub async fn list(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    filter: &SessionFilter,
) -> Result<(Vec<Session>, bool)> {
    // One statement with nullable predicates rather than a built string: a
    // compile-time checked query is the rule (CLAUDE.md), and every `is null
    // or` below is a filter the caller did not name.
    let rows = sqlx::query_as!(
        SessionRow,
        r#"
        select s.id, s.tenant_id, s.workspace_id, s.project_id, s.scope_id,
               s.principal_id, s.client_name, s.client_version,
               s.client_installation_id, s.external_session_id, s.agent_name,
               s.model_name, s.repository_id, s.branch, s.task_summary, s.status,
               s.started_at, s.ended_at, s.last_observed_at, s.end_reason,
               s.metadata, s.created_at, s.updated_at
        from sessions s
        where s.tenant_id = $1
          and ($2::uuid is null or exists (
                  select 1 from scope_closure c
                  where c.tenant_id = s.tenant_id
                    and c.ancestor_id = $2::uuid
                    and c.descendant_id = s.scope_id
              ))
          and ($3::uuid is null or s.workspace_id = $3::uuid)
          and ($4::uuid is null or s.project_id = $4::uuid)
          and ($5::text is null or s.status = $5::text)
          and ($6::text is null or s.client_name = $6::text)
          and ($7::text is null or s.principal_id = $7::text)
          and ($8::timestamptz is null or s.started_at >= $8::timestamptz)
          and ($9::timestamptz is null or s.started_at < $9::timestamptz)
          -- The keyset (CPR-11). A row comparison rather than two disjuncts,
          -- so the (tenant_id, scope_id, started_at desc) index can seek to it.
          and ($10::timestamptz is null
               or (s.started_at, s.id) < ($10::timestamptz, $11::uuid))
        order by s.started_at desc, s.id desc
        limit $12
        "#,
        tenant_id.as_uuid(),
        filter.scope_id.map(|id| id.as_uuid()) as Option<Uuid>,
        filter.workspace_id.map(|id| id.as_uuid()) as Option<Uuid>,
        filter.project_id.map(|id| id.as_uuid()) as Option<Uuid>,
        filter.status.map(|status| status.as_str()) as Option<&str>,
        filter.client_name.as_deref() as Option<&str>,
        filter.principal_id.as_deref() as Option<&str>,
        filter.started_after as Option<DateTime<Utc>>,
        filter.started_before as Option<DateTime<Utc>>,
        filter.after.map(|cursor| cursor.started_at) as Option<DateTime<Utc>>,
        filter.after.map(|cursor| cursor.id.as_uuid()) as Option<Uuid>,
        SCAN_LIMIT + 1,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;

    let truncated = rows.len() as i64 > SCAN_LIMIT;
    let kept: Vec<SessionRow> = rows.into_iter().take(SCAN_LIMIT as usize).collect();
    tracing::Span::current().record("sessions", kept.len());
    let sessions: Vec<Session> = kept
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<_>>()?;
    Ok((sessions, truncated))
}

/// Moves a session's lifecycle forward, optionally replacing its summary.
///
/// The transition is the precondition: the update names the states it will
/// accept, so an already-closed session is refused by the row rather than by a
/// revision the client had to echo back (ADR-0076 decision 3). The database
/// trigger holds the same rule for every writer.
///
/// # Errors
///
/// [`Error::NotFound`] when the session is not this tenant's;
/// [`Error::Conflict`] when it is already past the state being asked for.
#[tracing::instrument(
    name = "store.sessions.transition",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %id, session.status = %status),
    err(Display)
)]
pub async fn transition(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    id: SessionId,
    status: SessionStatus,
    task_summary: Option<&str>,
    end_reason: Option<&str>,
) -> Result<Session> {
    validate_label("task_summary", task_summary, MAX_TASK_SUMMARY_CHARS)?;
    validate_label("end_reason", end_reason, MAX_END_REASON_CHARS)?;
    let from: Vec<String> = SessionStatus::ALL
        .iter()
        .filter(|current| current.may_become(status))
        .map(|current| current.as_str().to_owned())
        .collect();
    if from.is_empty() {
        return Err(Error::Invalid {
            message: format!("a session cannot be moved to {status}"),
        });
    }

    let row = sqlx::query_as!(
        SessionRow,
        r#"
        update sessions
           set status       = $3,
               ended_at     = case when $3 in ('ended', 'abandoned', 'failed')
                                   then now() else null end,
               task_summary = coalesce($4, task_summary),
               end_reason   = coalesce($5, end_reason),
               updated_at   = now()
         where id = $1 and tenant_id = $2 and status = any($6)
        returning id, tenant_id, workspace_id, project_id, scope_id, principal_id,
                  client_name, client_version, client_installation_id,
                  external_session_id, agent_name, model_name, repository_id,
                  branch, task_summary, status, started_at, ended_at,
                  last_observed_at, end_reason, metadata, created_at, updated_at
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
        status.as_str(),
        task_summary as Option<&str>,
        end_reason as Option<&str>,
        &from,
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;

    let Some(row) = row else {
        return Err(match get(&mut *conn, tenant_id, id).await? {
            // The message names the state it is in and the state it was asked
            // for, because "conflict" alone leaves a client that retried after
            // a timeout unable to tell a lost race from a bug.
            Some(current) => Error::Conflict {
                message: format!(
                    "session {id} is {}; it cannot become {status}",
                    current.status
                ),
            },
            None => not_found(id),
        });
    };

    metrics::counter!(
        SESSION_MUTATIONS_TOTAL,
        "table" => "session",
        "operation" => "end",
    )
    .increment(1);
    row.try_into()
}

// ── Session events ───────────────────────────────────────────────────────────

/// One event a client is asking to append.
#[derive(Debug, Clone)]
pub struct NewSessionEvent {
    /// What happened.
    pub event_type: SessionEventType,
    /// The `payload` shape the client declares.
    pub event_schema_version: i32,
    /// The client's own id for it — the idempotency key.
    pub client_event_id: String,
    /// When the client says it happened.
    pub occurred_at: DateTime<Utc>,
    /// The content.
    pub payload: serde_json::Value,
}

/// What appending one event did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    /// The row was written.
    Appended,
    /// This `client_event_id` was already in this session; the stored row is
    /// returned unchanged, and nothing was written.
    Duplicate,
}

impl AppendOutcome {
    /// Stable wire name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            AppendOutcome::Appended => "appended",
            AppendOutcome::Duplicate => "duplicate",
        }
    }
}

/// One appended (or already-present) event, with which of the two it was.
#[derive(Debug, Clone)]
pub struct AppendedEvent {
    /// The stored row.
    pub event: SessionEvent,
    /// Whether this call wrote it.
    pub outcome: AppendOutcome,
}

struct EventRow {
    id: Uuid,
    tenant_id: Uuid,
    session_id: Uuid,
    event_type: String,
    event_schema_version: i32,
    client_event_id: String,
    sequence: i64,
    occurred_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
    payload: serde_json::Value,
    payload_hash: String,
}

impl TryFrom<EventRow> for SessionEvent {
    type Error = Error;

    fn try_from(row: EventRow) -> Result<Self> {
        Ok(SessionEvent {
            id: SessionEventId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            session_id: SessionId::from_uuid(row.session_id),
            event_type: row.event_type.parse().map_err(|err| Error::Internal {
                message: format!("stored value outside vocabulary: {err}"),
            })?,
            event_schema_version: row.event_schema_version,
            client_event_id: row.client_event_id,
            sequence: row.sequence,
            occurred_at: row.occurred_at,
            received_at: row.received_at,
            payload: row.payload,
            payload_hash: row.payload_hash,
        })
    }
}

/// BLAKE3-256 of the payload's canonical encoding, hex.
///
/// Canonical through [`synveda_types::json::canonicalise`] rather than by
/// `to_string()` alone, and that is load-bearing rather than defensive:
/// `cedar-policy-core` turns on `serde_json/preserve_order`, Cargo unifies
/// features across the workspace, and so a `Value`'s object iterates in the
/// order a client happened to write its keys. Without the sort, one event
/// re-sent with its keys in a different order is a different digest — which
/// makes the hash a statement about an HTTP library rather than about the
/// content. Found by the unit test below, not by reading.
fn payload_hash(payload: &serde_json::Value) -> String {
    blake3::hash(
        synveda_types::json::canonicalise(payload)
            .to_string()
            .as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// Appends a batch of events to a session, idempotently.
///
/// Every event is keyed by its `client_event_id`, unique per session: a
/// redelivered batch appends nothing twice and answers `duplicate` for each
/// event it already had. Positions are assigned here, under the session's own
/// row lock, so a batch is contiguous and two writers cannot interleave.
///
/// The session's `last_observed_at` is stamped to the newest `occurred_at` in
/// the batch, in the same transaction — never backwards, which the trigger
/// also holds.
///
/// # Errors
///
/// [`Error::NotFound`] for a session that is not this tenant's;
/// [`Error::Conflict`] for one that is closed; [`Error::Invalid`] for an empty
/// or oversized batch, or an event outside its bounds.
#[tracing::instrument(
    name = "store.sessions.append_events",
    skip_all,
    fields(
        tenant.id = %tenant_id,
        session.id = %session_id,
        events = events.len(),
        appended = tracing::field::Empty,
    ),
    err(Display)
)]
pub async fn append_events(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    session_id: SessionId,
    events: &[NewSessionEvent],
) -> Result<Vec<AppendedEvent>> {
    if events.is_empty() {
        return Err(Error::Invalid {
            message: "an append carries at least one event".to_owned(),
        });
    }
    if events.len() > MAX_EVENT_BATCH {
        return Err(Error::Invalid {
            message: format!(
                "an append carries at most {MAX_EVENT_BATCH} events, got {}",
                events.len()
            ),
        });
    }
    for event in events {
        validate_client_event_id(&event.client_event_id)?;
        validate_json_object("payload", &event.payload, MAX_EVENT_PAYLOAD_BYTES)?;
        if event.event_schema_version < 1 {
            return Err(Error::Invalid {
                message: "`event_schema_version` is at least 1".to_owned(),
            });
        }
    }
    // Two ids in one batch would race each other for a position and one of
    // them would silently become the other's duplicate. Refused by name.
    let mut keys: Vec<&str> = events
        .iter()
        .map(|event| event.client_event_id.as_str())
        .collect();
    keys.sort_unstable();
    let before = keys.len();
    keys.dedup();
    if keys.len() != before {
        return Err(Error::Invalid {
            message: "one batch carries each `client_event_id` at most once".to_owned(),
        });
    }

    // The lock, and the state check under it: a session that closed between
    // the client's decision to append and this statement is refused, and a
    // session closing *during* the append cannot, because the closer needs
    // this same row.
    let locked = sqlx::query!(
        r#"
        select status
        from sessions
        where id = $1 and tenant_id = $2
        for update
        "#,
        session_id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(&mut *conn)
    .await
    .map_err(storage_error)?;
    let Some(locked) = locked else {
        return Err(not_found(session_id));
    };
    let status: SessionStatus = locked.status.parse().map_err(|err| Error::Internal {
        message: format!("stored value outside vocabulary: {err}"),
    })?;
    if !status.accepts_events() {
        return Err(Error::Conflict {
            message: format!("session {session_id} is {status}; it accepts no more events"),
        });
    }

    // The head, read *under* the lock above — which is the whole reason the
    // lock is taken. `max + 1` is only safe while nobody else can be choosing
    // a position for this session at the same time.
    let head = sqlx::query_scalar!(
        r#"
        select coalesce(max(sequence), 0) as "head!"
        from session_events
        where tenant_id = $1 and session_id = $2
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    let mut next = head + 1;
    let mut appended = Vec::with_capacity(events.len());
    let mut written = 0usize;
    let mut newest: Option<DateTime<Utc>> = None;
    for event in events {
        let hash = payload_hash(&event.payload);
        let row = sqlx::query_as!(
            EventRow,
            r#"
            insert into session_events
                (id, tenant_id, session_id, event_type, event_schema_version,
                 client_event_id, sequence, occurred_at, payload, payload_hash)
            values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            on conflict (tenant_id, session_id, client_event_id) do nothing
            returning id, tenant_id, session_id, event_type, event_schema_version,
                      client_event_id, sequence, occurred_at, received_at,
                      payload, payload_hash
            "#,
            SessionEventId::new().as_uuid(),
            tenant_id.as_uuid(),
            session_id.as_uuid(),
            event.event_type.as_str(),
            event.event_schema_version,
            event.client_event_id,
            next,
            event.occurred_at,
            event.payload,
            hash,
        )
        .fetch_optional(&mut *conn)
        .await
        .map_err(storage_error)?;

        match row {
            Some(row) => {
                next += 1;
                written += 1;
                newest = Some(newest.map_or(event.occurred_at, |at: DateTime<Utc>| {
                    at.max(event.occurred_at)
                }));
                appended.push(AppendedEvent {
                    event: row.try_into()?,
                    outcome: AppendOutcome::Appended,
                });
            }
            None => {
                // Already here. The stored row is served rather than the
                // caller's version of it: a retry must be told what this
                // deployment holds, not handed back what it just sent.
                let existing =
                    event_by_client_id(&mut *conn, tenant_id, session_id, &event.client_event_id)
                        .await?
                        .ok_or_else(|| Error::Internal {
                            message: "an event conflicted and then could not be read".to_owned(),
                        })?;
                appended.push(AppendedEvent {
                    event: existing,
                    outcome: AppendOutcome::Duplicate,
                });
            }
        }
    }

    if let Some(newest) = newest {
        sqlx::query!(
            r#"
            update sessions
               set last_observed_at = greatest(coalesce(last_observed_at, $3), $3),
                   updated_at       = now()
             where id = $1 and tenant_id = $2
            "#,
            session_id.as_uuid(),
            tenant_id.as_uuid(),
            newest,
        )
        .execute(&mut *conn)
        .await
        .map_err(storage_error)?;
    }

    tracing::Span::current().record("appended", written);
    metrics::counter!(
        SESSION_MUTATIONS_TOTAL,
        "table" => "event",
        "operation" => "append",
    )
    .increment(written as u64);
    metrics::counter!(
        SESSION_MUTATIONS_TOTAL,
        "table" => "event",
        "operation" => "duplicate",
    )
    .increment((events.len() - written) as u64);
    Ok(appended)
}

/// One event by the client's own id.
async fn event_by_client_id(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    session_id: SessionId,
    client_event_id: &str,
) -> Result<Option<SessionEvent>> {
    let row = sqlx::query_as!(
        EventRow,
        r#"
        select id, tenant_id, session_id, event_type, event_schema_version,
               client_event_id, sequence, occurred_at, received_at, payload,
               payload_hash
        from session_events
        where tenant_id = $1 and session_id = $2 and client_event_id = $3
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
        client_event_id,
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// One event of a session, by its own id.
///
/// Keyed by the session as well as the event, so an event id from another run
/// answers `None` rather than that run's payload — the ownership rule the
/// gateway's 404 depends on, held in the query rather than in a check the
/// caller has to remember to write.
#[tracing::instrument(
    name = "store.sessions.event",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %session_id, event.id = %id),
    err(Display)
)]
pub async fn event(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    session_id: SessionId,
    id: SessionEventId,
) -> Result<Option<SessionEvent>> {
    let row = sqlx::query_as!(
        EventRow,
        r#"
        select id, tenant_id, session_id, event_type, event_schema_version,
               client_event_id, sequence, occurred_at, received_at, payload,
               payload_hash
        from session_events
        where tenant_id = $1 and session_id = $2 and id = $3
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    row.map(TryInto::try_into).transpose()
}

/// A session's events, in order, from `after` exclusive.
///
/// `after` is a sequence rather than an offset, which is what makes a client
/// that is following a live session able to ask for "everything since" without
/// re-reading, and what makes the answer stable while events are still being
/// appended behind it.
#[tracing::instrument(
    name = "store.sessions.events",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %session_id),
    err(Display)
)]
pub async fn events(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    session_id: SessionId,
    after: i64,
    limit: i64,
) -> Result<Vec<SessionEvent>> {
    let rows = sqlx::query_as!(
        EventRow,
        r#"
        select id, tenant_id, session_id, event_type, event_schema_version,
               client_event_id, sequence, occurred_at, received_at, payload,
               payload_hash
        from session_events
        where tenant_id = $1 and session_id = $2 and sequence > $3
        order by sequence
        limit $4
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
        after,
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    rows.into_iter().map(TryInto::try_into).collect()
}

// ── Context runs ─────────────────────────────────────────────────────────────

/// What [`record_context_run`] stores.
#[derive(Debug, Clone)]
pub struct NewContextRun {
    /// The run's identity.
    pub id: ContextRunId,
    /// The session it was composed for.
    pub session_id: SessionId,
    /// The scope it was anchored at — the session's.
    pub scope_id: ScopeId,
    /// The token subject that asked.
    pub principal_id: String,
    /// The task, when one was named.
    pub query: Option<String>,
    /// The composed block.
    pub rendered: String,
    /// The block's identity.
    pub block_hash: String,
    /// Estimated tokens of `rendered`.
    pub tokens: i32,
    /// The budget it composed under.
    pub budget_tokens: i32,
    /// How many records composed.
    pub entry_count: i32,
    /// Which legs degraded.
    pub degraded: Vec<String>,
}

struct ContextRunRow {
    id: Uuid,
    tenant_id: Uuid,
    session_id: Uuid,
    scope_id: Uuid,
    principal_id: String,
    query: Option<String>,
    rendered: String,
    block_hash: String,
    tokens: i32,
    budget_tokens: i32,
    entry_count: i32,
    degraded: Vec<String>,
    created_at: DateTime<Utc>,
}

impl From<ContextRunRow> for ContextRun {
    fn from(row: ContextRunRow) -> Self {
        ContextRun {
            id: ContextRunId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            session_id: SessionId::from_uuid(row.session_id),
            scope_id: ScopeId::from_uuid(row.scope_id),
            principal_id: row.principal_id,
            query: row.query,
            rendered: row.rendered,
            block_hash: row.block_hash,
            tokens: row.tokens,
            budget_tokens: row.budget_tokens,
            entry_count: row.entry_count,
            degraded: row.degraded,
            created_at: row.created_at,
        }
    }
}

/// Records one context run.
///
/// Append-only: the application role holds no UPDATE on this table, so what a
/// session was given at a moment is what the row says forever.
#[tracing::instrument(
    name = "store.sessions.record_context_run",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %new.session_id, run.id = %new.id),
    err(Display)
)]
pub async fn record_context_run(
    conn: &mut PgConnection,
    tenant_id: TenantId,
    new: &NewContextRun,
) -> Result<ContextRun> {
    let row = sqlx::query_as!(
        ContextRunRow,
        r#"
        insert into session_context_runs
            (id, tenant_id, session_id, scope_id, principal_id, query, rendered,
             block_hash, tokens, budget_tokens, entry_count, degraded)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        returning id, tenant_id, session_id, scope_id, principal_id, query,
                  rendered, block_hash, tokens, budget_tokens, entry_count,
                  degraded, created_at
        "#,
        new.id.as_uuid(),
        tenant_id.as_uuid(),
        new.session_id.as_uuid(),
        new.scope_id.as_uuid(),
        new.principal_id,
        new.query.as_deref() as Option<&str>,
        new.rendered,
        new.block_hash,
        new.tokens,
        new.budget_tokens,
        new.entry_count,
        &new.degraded,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(storage_error)?;

    metrics::counter!(
        SESSION_MUTATIONS_TOTAL,
        "table" => "context_run",
        "operation" => "create",
    )
    .increment(1);
    Ok(row.into())
}

/// Fetches one context run.
#[tracing::instrument(
    name = "store.sessions.context_run",
    skip_all,
    fields(tenant.id = %tenant_id, run.id = %id),
    err(Display)
)]
pub async fn context_run(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    id: ContextRunId,
) -> Result<Option<ContextRun>> {
    let row = sqlx::query_as!(
        ContextRunRow,
        r#"
        select id, tenant_id, session_id, scope_id, principal_id, query, rendered,
               block_hash, tokens, budget_tokens, entry_count, degraded, created_at
        from session_context_runs
        where id = $1 and tenant_id = $2
        "#,
        id.as_uuid(),
        tenant_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await
    .map_err(storage_error)?;
    Ok(row.map(Into::into))
}

/// A session's context runs, oldest first.
///
/// `rendered` is deliberately **not** selected: the timeline projection wants
/// to know a run happened, what it asked and how big the answer was, and
/// serving every composed block of a long session in a listing would put a
/// session's whole context history in one response.
#[tracing::instrument(
    name = "store.sessions.context_runs",
    skip_all,
    fields(tenant.id = %tenant_id, session.id = %session_id),
    err(Display)
)]
pub async fn context_runs(
    executor: impl PgExecutor<'_>,
    tenant_id: TenantId,
    session_id: SessionId,
    limit: i64,
) -> Result<Vec<ContextRun>> {
    let rows = sqlx::query_as!(
        ContextRunRow,
        r#"
        select id, tenant_id, session_id, scope_id, principal_id, query,
               '' as "rendered!", block_hash, tokens, budget_tokens, entry_count,
               degraded, created_at
        from session_context_runs
        where tenant_id = $1 and session_id = $2
        order by created_at, id
        limit $3
        "#,
        tenant_id.as_uuid(),
        session_id.as_uuid(),
        limit,
    )
    .fetch_all(executor)
    .await
    .map_err(storage_error)?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest is about the content, not about how a client wrote it.
    ///
    /// This test failed on the first cut, and the reason is worth keeping:
    /// `payload_hash` hashed `to_string()` directly, on the belief that
    /// `serde_json::Map` is a `BTreeMap`. It is an `IndexMap` in this
    /// workspace, because `cedar-policy-core` enables `preserve_order` and
    /// Cargo unifies features — so a re-sent event whose keys arrived in a
    /// different order got a different digest.
    #[test]
    fn a_payload_hash_is_about_content_and_not_about_key_order() {
        let a = serde_json::json!({"a": 1, "b": [2, 3], "c": {"y": 1, "x": 2}});
        let b = serde_json::json!({"c": {"x": 2, "y": 1}, "b": [2, 3], "a": 1});
        assert_eq!(payload_hash(&a), payload_hash(&b));
        assert_ne!(payload_hash(&a), payload_hash(&serde_json::json!({"a": 2})));
        // An array's order *is* content, so reordering one is a different
        // payload and must be a different digest.
        assert_ne!(
            payload_hash(&serde_json::json!({"b": [2, 3]})),
            payload_hash(&serde_json::json!({"b": [3, 2]}))
        );
        assert_eq!(payload_hash(&a).len(), 64);
    }

    #[test]
    fn an_outcome_names_itself_the_way_the_api_reports_it() {
        assert_eq!(AppendOutcome::Appended.as_str(), "appended");
        assert_eq!(AppendOutcome::Duplicate.as_str(), "duplicate");
    }

    /// The `from` set `transition` builds is the lifecycle rule, read off
    /// `may_become` rather than transcribed — so the SQL predicate and
    /// `SessionStatus` cannot disagree.
    #[test]
    fn the_accepted_source_states_are_the_lifecycle_rule() {
        let sources = |status: SessionStatus| -> Vec<&'static str> {
            SessionStatus::ALL
                .iter()
                .filter(|current| current.may_become(status))
                .map(SessionStatus::as_str)
                .collect()
        };
        assert_eq!(sources(SessionStatus::Ending), ["active"]);
        assert_eq!(sources(SessionStatus::Ended), ["active", "ending"]);
        assert_eq!(sources(SessionStatus::Abandoned), ["active", "ending"]);
        assert_eq!(sources(SessionStatus::Failed), ["active", "ending"]);
        assert!(sources(SessionStatus::Active).is_empty());
    }
}

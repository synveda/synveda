//! The session ledger and runtime API (CPR-10, ADR-0076): `/v1/sessions/*`.
//!
//! Seven routes over the CPR-10 store services — open, list, read, append
//! events, end, timeline, compose context — behind tenant resolution like
//! every `/v1` route, behind the PDP like every governed one, and chaining an
//! audit event for every mutation.
//!
//! # Nothing on the wire carries a tenant or an acting principal
//!
//! ADR-0076 decision 8, and it is the one rule to check before adding a field
//! to any body here. Every request body on this plane is
//! `#[serde(deny_unknown_fields)]`, so a client that sends `tenant_id` or
//! `principal_id` is refused rather than quietly ignored — and
//! `a_client_cannot_submit_its_own_tenant_or_principal` in
//! `tests/sessions_api.rs` asserts exactly that, by name, from the outside.
//! Both values come from the verified token.
//!
//! # Two idempotency mechanisms, and they are not redundant
//!
//! Opening a session and composing a context run each take a required
//! `Idempotency-Key` (see [`crate::idempotency`]), because each is a creation
//! whose retry after a timeout would otherwise make a second row.
//!
//! Appending events does **not**. Its unit of idempotency is the *event*, not
//! the request: a batch is a list of things that happened, each carrying the
//! client's own `client_event_id`, and a redelivered batch that overlaps a
//! previous one by three events out of ten must append seven and answer
//! `duplicate` for three. A header guarding the whole request could not
//! express that, and requiring both would be one mechanism doing nothing.
//!
//! # Where the decisions are anchored
//!
//! ADR-0073 decision 3: a decision names what it is about. The **listing** is
//! decided at the scope it is anchored at — a named scope, or the tenant root
//! — and then **again per row against the row** (CPR-9), so a caller granted
//! `member` at one project sees that project's runs and not the workspace's.
//! Every per-object route decides about the `Session` itself, after the
//! ownership check, so a foreign id is a 404 rather than a denial oracle.
//!
//! # The timeline is a projection
//!
//! `GET …/timeline` merges the session's events and its context runs, orders
//! them, and renders them as one sequence. There is no timeline table and
//! there must not be one (ADR-0076 decision 9): a materialised transcript
//! would be a second copy of `session_events`, and the two would disagree the
//! first time one was written and the other was not.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_ingest::embedding::Embedder as _;
use synveda_policy::{Action, Resource, ResourceEntity, ScopeNode};
use synveda_retrieval::{
    CandidateScope, ComposeRequest, MemoryReadInputs, QueryVector, SearchFilter, SearchRequest,
    compose, composition_plan, hybrid_search,
};
use synveda_store::anchors::AnchorSelection;
use synveda_store::sessions::{
    self, AppendOutcome, NewContextRun, NewSession, NewSessionEvent, SessionFilter,
};
use synveda_store::{rls, scopes};
use synveda_types::session::{
    CURRENT_EVENT_SCHEMA_VERSION, ContextRun, Session, SessionEvent, SessionEventType,
    SessionStatus,
};
use synveda_types::{
    ContextRunId, Error, PolicyAssignment, ProjectId, RepositoryId, Result, ScopeId, SessionId,
    TenantId, WorkspaceId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, Authorized};
use crate::error::ApiError;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{body, commit, tenant_id};
use crate::telemetry::SESSION_OPERATIONS_TOTAL;
use crate::workspaces::{ApiErrorBody, Decidable, string_enum, subject};

/// Default rows in a session listing. The scan bound above it is the store's
/// [`sessions::SCAN_LIMIT`].
const DEFAULT_LIST_LIMIT: i64 = 50;

/// Most rows one listing will serve.
const MAX_LIST_LIMIT: i64 = 200;

/// Most timeline entries one response carries, per source.
const MAX_TIMELINE_ENTRIES: i64 = 500;

/// Task cap for a context run. A task is a query, not a document — the same
/// bound `/v1/inject` uses, and for its reason.
const MAX_QUERY_CHARS: usize = 4096;

/// Candidate depth handed to the ranking legs, matching `/v1/inject`'s.
const RELEVANCE_LIMIT: usize = 200;

// ── Views ────────────────────────────────────────────────────────────────────

/// A session, as the API serves it.
///
/// A view rather than `synveda_types::session::Session` itself, for
/// [`crate::workspaces::WorkspaceView`]'s reason: this is the **contract** and
/// the domain type is not. `tenant_id` is deliberately absent — every `/v1`
/// response is already scoped to the caller's tenant.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionView {
    /// The session's stable id.
    #[schema(value_type = String, format = "uuid")]
    pub id: SessionId,
    /// The workspace the run happened in.
    #[schema(value_type = String, format = "uuid")]
    pub workspace_id: WorkspaceId,
    /// The project, when the run was against one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// The governed scope this session is decided at — **derived** from the
    /// workspace and project, never submitted.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// The token subject that opened it.
    pub principal_id: String,
    /// The agent client, as it named itself.
    pub client_name: String,
    /// Its version, when it said one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    /// A stable id for that installation of the client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_installation_id: Option<String>,
    /// The harness's own id for the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_session_id: Option<String>,
    /// Which agent ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    /// The model, as the client named it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// The repository the run was against.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub repository_id: Option<RepositoryId>,
    /// The branch it was on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What the run is about, in the client's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_summary: Option<String>,
    /// Where the run is in its life.
    #[schema(schema_with = session_status_schema)]
    pub status: SessionStatus,
    /// When it began.
    pub started_at: DateTime<Utc>,
    /// When it closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// The newest appended event's own instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_observed_at: Option<DateTime<Utc>>,
    /// The client's labelling bag, echoed back.
    #[schema(value_type = Object)]
    pub metadata: serde_json::Value,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
}

impl From<Session> for SessionView {
    fn from(session: Session) -> Self {
        SessionView {
            id: session.id,
            workspace_id: session.workspace_id,
            project_id: session.project_id,
            scope_id: session.scope_id,
            principal_id: session.principal_id,
            client_name: session.client_name,
            client_version: session.client_version,
            client_installation_id: session.client_installation_id,
            external_session_id: session.external_session_id,
            agent_name: session.agent_name,
            model_name: session.model_name,
            repository_id: session.repository_id,
            branch: session.branch,
            task_summary: session.task_summary,
            status: session.status,
            started_at: session.started_at,
            ended_at: session.ended_at,
            last_observed_at: session.last_observed_at,
            metadata: session.metadata,
            created_at: session.created_at,
            updated_at: session.updated_at,
        }
    }
}

/// The session listing.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionList {
    /// The sessions this caller may read, newest first.
    pub sessions: Vec<SessionView>,
    /// Whether there are more than this answer carries.
    ///
    /// Named rather than hidden. A recency-ordered feed can honestly serve
    /// "the newest N"; what it must never do is serve them as though they were
    /// all of them (ADR-0058 decision 5's rule, one plane over).
    pub truncated: bool,
}

/// One immutable session event, as the API serves it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionEventView {
    /// The event's id in this deployment.
    #[schema(value_type = String, format = "uuid")]
    pub id: synveda_types::SessionEventId,
    /// The session it belongs to.
    #[schema(value_type = String, format = "uuid")]
    pub session_id: SessionId,
    /// What happened.
    #[schema(schema_with = event_type_schema)]
    pub event_type: SessionEventType,
    /// The payload shape the client declared.
    pub event_schema_version: i32,
    /// The client's own id for it.
    pub client_event_id: String,
    /// Position in the session, assigned by the server.
    pub sequence: i64,
    /// When the client says it happened.
    pub occurred_at: DateTime<Utc>,
    /// When the gateway received it.
    pub received_at: DateTime<Utc>,
    /// The content.
    #[schema(value_type = Object)]
    pub payload: serde_json::Value,
    /// BLAKE3-256 of the canonical payload, hex — the server's.
    pub payload_hash: String,
}

impl From<SessionEvent> for SessionEventView {
    fn from(event: SessionEvent) -> Self {
        SessionEventView {
            id: event.id,
            session_id: event.session_id,
            event_type: event.event_type,
            event_schema_version: event.event_schema_version,
            client_event_id: event.client_event_id,
            sequence: event.sequence,
            occurred_at: event.occurred_at,
            received_at: event.received_at,
            payload: event.payload,
            payload_hash: event.payload_hash,
        }
    }
}

/// What one appended event did, and the row it names.
#[derive(Debug, Serialize, ToSchema)]
pub struct AppendedEventView {
    /// `appended` or `duplicate`.
    pub outcome: String,
    /// The stored row — this deployment's version of it, never the caller's,
    /// because a retry must be told what is held rather than handed back what
    /// it just sent.
    pub event: SessionEventView,
}

/// The append response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AppendResponse {
    /// Per-event outcomes, in the order the batch listed them.
    pub events: Vec<AppendedEventView>,
    /// How many were written.
    pub appended: usize,
    /// How many were already here.
    pub duplicates: usize,
}

/// A context run, as the API serves it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ContextRunView {
    /// The run's id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ContextRunId,
    /// The session it was composed for.
    #[schema(value_type = String, format = "uuid")]
    pub session_id: SessionId,
    /// The scope it was anchored at.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// The task, when one was named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// The rendered block, watermark line included. Empty when nothing
    /// composed — a result, not an error.
    pub rendered: String,
    /// BLAKE3 over the composed entries, hex.
    pub block_hash: String,
    /// Estimated tokens of `rendered`.
    pub tokens: i32,
    /// The budget it composed under.
    pub budget_tokens: i32,
    /// How many records composed.
    pub entry_count: i32,
    /// Which retrieval legs degraded — `embedder`, `retrieval`. Empty is the
    /// ordinary answer.
    pub degraded: Vec<String>,
    /// When it was composed.
    pub created_at: DateTime<Utc>,
}

impl From<ContextRun> for ContextRunView {
    fn from(run: ContextRun) -> Self {
        ContextRunView {
            id: run.id,
            session_id: run.session_id,
            scope_id: run.scope_id,
            query: run.query,
            rendered: run.rendered,
            block_hash: run.block_hash,
            tokens: run.tokens,
            budget_tokens: run.budget_tokens,
            entry_count: run.entry_count,
            degraded: run.degraded,
            created_at: run.created_at,
        }
    }
}

/// One entry of the timeline projection.
///
/// Deliberately **not** a union of the two row shapes. A timeline is a reading
/// surface: it answers "what happened, in order, and roughly what was it", and
/// a client that wants an event's full payload fetches the event. Flattening
/// two tables into one wide row with half its fields null per entry would make
/// every consumer branch on which half is populated.
#[derive(Debug, Serialize, ToSchema)]
pub struct TimelineEntry {
    /// `event` or `context_run` — which table this came from.
    pub kind: String,
    /// The entry's own id, as a string, because the two sources have
    /// different id types and a timeline is read rather than joined.
    pub id: String,
    /// When it happened: an event's `occurred_at`, a run's `created_at`.
    pub at: DateTime<Utc>,
    /// The event type, for an event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    /// The event's position, for an event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<i64>,
    /// One line about what this entry is — a run's query, an event's family.
    pub summary: String,
}

/// The timeline projection.
#[derive(Debug, Serialize, ToSchema)]
pub struct TimelineView {
    /// The session this is about.
    #[schema(value_type = String, format = "uuid")]
    pub session_id: SessionId,
    /// The merged entries, oldest first.
    pub entries: Vec<TimelineEntry>,
    /// How many events the session has appended, of every type — the shape of
    /// the run, which is what an auditor reads before any single entry.
    pub event_counts: BTreeMap<String, i64>,
    /// Whether either source hit its bound.
    pub truncated: bool,
}

/// The `status` vocabulary, built from
/// [`synveda_types::session::SessionStatus`] itself rather than transcribed —
/// so the OpenAPI enum and the Rust enum cannot disagree.
fn session_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(SessionStatus::ALL.iter().map(SessionStatus::as_str))
}

/// The `event_type` vocabulary, built the same way.
fn event_type_schema() -> utoipa::openapi::schema::Object {
    string_enum(SessionEventType::ALL.iter().map(SessionEventType::as_str))
}

/// The three states `POST …/end` accepts beside `ending`.
fn end_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        [SessionStatus::Ending]
            .iter()
            .chain(SessionStatus::TERMINAL.iter())
            .map(|status| status.as_str()),
    )
}

// ── Request bodies ───────────────────────────────────────────────────────────

/// `POST /v1/sessions`.
///
/// There is no `tenant_id` and no `principal_id` here, and
/// `deny_unknown_fields` is what makes sending one an error rather than a
/// silent no-op (ADR-0076 decision 8). There is no `scope_id` either: the
/// governed scope is derived from `workspace_id` and `project_id` by the
/// store, because a client that could name the scope could name one its
/// workspace is not in.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenSessionBody {
    /// The workspace the run is in.
    #[schema(value_type = String, format = "uuid")]
    pub workspace_id: WorkspaceId,
    /// The project, when the run is against one.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// The agent client, as it names itself: a lowercase label of letters,
    /// digits, `-` and `.`.
    pub client_name: String,
    /// Its version.
    #[serde(default)]
    pub client_version: Option<String>,
    /// A stable id for this installation of the client.
    #[serde(default)]
    pub client_installation_id: Option<String>,
    /// The harness's own id for this run. Unique per caller and client.
    #[serde(default)]
    pub external_session_id: Option<String>,
    /// Which agent is running.
    #[serde(default)]
    pub agent_name: Option<String>,
    /// The model, as the client names it.
    #[serde(default)]
    pub model_name: Option<String>,
    /// A repository attached to the named project.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub repository_id: Option<RepositoryId>,
    /// The branch the run is on.
    #[serde(default)]
    pub branch: Option<String>,
    /// What the run is about.
    #[serde(default)]
    pub task_summary: Option<String>,
    /// A labelling bag: a JSON object, at most 8 KiB encoded. Never copied
    /// into an audit payload.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub metadata: Option<serde_json::Value>,
}

/// One event of a `POST /v1/sessions/{session_id}/events` batch.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewEventBody {
    /// What happened — one of the twelve names.
    #[schema(schema_with = event_type_schema)]
    pub event_type: SessionEventType,
    /// The payload shape this client declares. Defaults to the current one.
    #[serde(default = "default_schema_version")]
    pub event_schema_version: i32,
    /// The client's own id for this event. **The idempotency unit**: a
    /// redelivered batch appends nothing twice.
    pub client_event_id: String,
    /// When the client says it happened.
    pub occurred_at: DateTime<Utc>,
    /// The content: a JSON object, at most 64 KiB encoded.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub payload: Option<serde_json::Value>,
}

const fn default_schema_version() -> i32 {
    CURRENT_EVENT_SCHEMA_VERSION
}

/// `POST /v1/sessions/{session_id}/events`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AppendEventsBody {
    /// The batch, at most 200 events, each `client_event_id` at most once.
    pub events: Vec<NewEventBody>,
}

/// `POST /v1/sessions/{session_id}/end`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EndSessionBody {
    /// Where to move it: `ending` to announce the close while buffered events
    /// are still arriving, or one of `ended`, `abandoned`, `failed` to close
    /// it.
    #[schema(schema_with = end_status_schema)]
    pub status: SessionStatus,
    /// What the run turned out to be about, when the client only knows at the
    /// end. Replaces whatever was set at open.
    #[serde(default)]
    pub task_summary: Option<String>,
}

/// `POST /v1/sessions/{session_id}/context-runs`.
///
/// The **final shape** of this endpoint (ADR-0076 decision 7). What it does
/// today is call the existing retrieval engine and persist the identity and
/// the rendered block; Prompt 18 adds the explainability — which scopes were
/// considered, which were denied, why each entry made the cut — behind the
/// same request and the same response envelope.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextRunBody {
    /// What the agent is about to do. Ranks the material; omitting it is the
    /// session-start shape — everything pinned, nothing ranked.
    #[serde(default)]
    pub query: Option<String>,
    /// Narrow the block's token budget. The caller may narrow and never
    /// widen: the pack's budget is the ceiling.
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    /// Ceiling on the sensitivity tier that may compose.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub max_sensitivity: Option<synveda_types::Sensitivity>,
}

/// Query parameters for `GET /v1/sessions`.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Only runs at or under this governed scope — the subtree, so a
    /// workspace's scope lists its projects' runs.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Only runs in this workspace.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub workspace_id: Option<WorkspaceId>,
    /// Only runs in this project.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Only runs in this state: `active`, `ending`, `ended`, `abandoned` or
    /// `failed`.
    #[serde(default)]
    pub status: Option<String>,
    /// How many rows to serve: 1–200, default 50.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Query parameters for `GET /v1/sessions/{session_id}/timeline`.
#[derive(Debug, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TimelineParams {
    /// Only entries after this event sequence. A sequence rather than an
    /// offset, so a client following a live run can ask for "everything
    /// since" and get a stable answer while events are still arriving.
    #[serde(default)]
    pub after: Option<i64>,
}

// ── Shared handler plumbing ──────────────────────────────────────────────────

/// Counts the operation and renders the result — the three-outcome taxonomy
/// every plane in this gateway uses.
async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let outcome = match &result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(_) => "error",
    };
    metrics::counter!(SESSION_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// The uniform 404 for a session that is missing *or* another tenant's.
fn session_not_found(id: SessionId) -> Error {
    Error::NotFound {
        entity: format!("session {id}"),
    }
}

/// What a decision on this plane is about.
enum Subject<'a> {
    /// The plane, anchored at a governed scope — a listing's gate, or the
    /// scope a run is about to be opened at.
    ///
    /// Not optional, unlike the workspace plane's: the Cedar schema does not
    /// admit a `Tenant` resource for these actions, because a run always
    /// happens somewhere. A tenant with no scopes at all therefore has no
    /// session question to ask, and the listing answers it without reaching
    /// here.
    At(&'a synveda_types::scope::Scope),
    /// One session.
    Session(&'a Session),
}

/// Takes one decision about `subject`, returning the gathered input beside it
/// so a listing can decide per row without gathering twice.
async fn require(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    action: Action,
    tenant_id: TenantId,
    subject: Subject<'_>,
) -> Result<(Authorized, Resource, authz::DecisionInput)> {
    let (anchor, selection, resources, resource) = match subject {
        Subject::At(scope) => (
            Some(scope.clone()),
            AnchorSelection::none(),
            Vec::new(),
            Resource::Scope(scope.id),
        ),
        Subject::Session(session) => (
            scopes::get(&mut *tx, tenant_id, session.scope_id).await?,
            match session.project_id {
                Some(project_id) => AnchorSelection::project(project_id),
                None => AnchorSelection::workspace(session.workspace_id),
            },
            vec![ResourceEntity::Session {
                id: session.id,
                scope_id: session.scope_id,
            }],
            Resource::Session(session.id),
        ),
    };
    let input = authz::gather(state, tx, anchor.as_ref(), selection, resources).await?;
    let authorized = authz::decide(state, &input, action, resource)?;
    Ok((authorized, resource, input))
}

/// The allowed-read decision event (ADR-0019 decision 4): a read has no
/// semantic event of its own, so the decision itself chains.
async fn read_event(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    op: &'static str,
    action: Action,
    authorized: &Authorized,
    resource: Resource,
    detail: serde_json::Value,
) -> Result<()> {
    audit::record(
        tx,
        tenant_id,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({
            "op": op,
            "authz": audit::decision_context(action, authorized),
            "detail": detail,
        }),
    )
    .await
    .map(|_| ())
}

/// The payload image of a session.
///
/// `metadata` is deliberately **absent** and replaced by a size: an agent's
/// environment is where credentials live, and this is the field a harness
/// would put an environment in (seed: no secret in an audit payload). That
/// there was metadata, and how much, is auditable; what was in it is not.
fn session_image(session: &Session) -> serde_json::Value {
    json!({
        "id": session.id,
        "workspace_id": session.workspace_id,
        "project_id": session.project_id,
        "scope_id": session.scope_id,
        "principal_id": session.principal_id,
        "client_name": session.client_name,
        "client_version": session.client_version,
        "external_session_id": session.external_session_id,
        "agent_name": session.agent_name,
        "model_name": session.model_name,
        "repository_id": session.repository_id,
        "branch": session.branch,
        "status": session.status.as_str(),
        "started_at": session.started_at,
        "metadata_bytes": session.metadata.to_string().len(),
    })
}

/// The sessions this caller may read, decided **one row at a time against the
/// row** (CPR-9's rule, applied from the start rather than retrofitted).
///
/// [`crate::workspaces::decide_each`] does the deciding; this is the mapping
/// from a session to what that function needs. The reason it is per row rather
/// than per distinct scope — which would be cheaper, because every session in
/// one project decides identically under the shipped packs — is that a
/// *stored* pack may name a session by its own entity id, and a cache keyed on
/// the scope would then answer a question it was never asked.
async fn readable(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    input: &authz::DecisionInput,
    sessions: Vec<Session>,
) -> Result<(Vec<Session>, Option<Authorized>)> {
    let rows: Vec<Decidable> = sessions
        .iter()
        .map(|session| Decidable {
            resource: Resource::Session(session.id),
            scope_id: session.scope_id,
            entity: ResourceEntity::Session {
                id: session.id,
                scope_id: session.scope_id,
            },
        })
        .collect();
    let verdicts =
        crate::workspaces::decide_each(state, conn, tenant_id, input, Action::SessionRead, &rows)
            .await?;
    let mut authorized = None;
    let kept = sessions
        .into_iter()
        .zip(verdicts)
        .filter_map(|(session, verdict)| {
            let allowed = verdict?;
            authorized.get_or_insert(allowed);
            Some(session)
        })
        .collect();
    Ok((kept, authorized))
}

/// Reads a session and decides `action` about it, in that order.
///
/// The order is ADR-0012 decision 7's and it is not negotiable on this plane:
/// a session that is not this tenant's is a 404, indistinguishable from an id
/// nobody ever minted, because a caller who can tell the two apart can
/// enumerate another tenant's runs a uuid at a time.
async fn load(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    id: SessionId,
    action: Action,
) -> Result<(Session, Authorized, Resource)> {
    let session = sessions::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| session_not_found(id))?;
    let (authorized, resource, _) =
        require(state, tx, action, tenant_id, Subject::Session(&session)).await?;
    Ok((session, authorized, resource))
}

// ── Open ─────────────────────────────────────────────────────────────────────

/// `POST /v1/sessions` — open a run.
#[utoipa::path(
    post,
    path = "/v1/sessions",
    operation_id = "open_session",
    tag = "sessions",
    request_body = OpenSessionBody,
    params(
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    responses(
        (status = 201, description = "Opened", body = SessionView),
        (status = 200, description = "This key already opened this session", body = SessionView),
        (status = 400, description = "Malformed body, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `session.write`", body = ApiErrorBody),
        (status = 404, description = "No such workspace or project in this tenant", body = ApiErrorBody),
        (status = 409, description = "The workspace is archived, the harness id is taken, or the key was reused for a different request", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn open(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<OpenSessionBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        let claim = Claim::from_headers(
            &headers,
            "session.open",
            &subject,
            &json!({
                "route": "POST /v1/sessions",
                "workspace_id": body.workspace_id,
                "project_id": body.project_id,
                "client_name": body.client_name,
                "external_session_id": body.external_session_id,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(id) => Some(id),
            Dispatch::Create => {
                match open_session(&state, tenant_id, &subject, &body, &claim).await {
                    Ok(session) => {
                        return Ok((StatusCode::CREATED, Json(SessionView::from(session))));
                    }
                    Err(conflict @ Error::Conflict { .. }) => Some(
                        crate::idempotency::resolve_conflict(
                            &state.pool,
                            tenant_id,
                            &claim,
                            conflict,
                        )
                        .await?,
                    ),
                    Err(other) => return Err(other),
                }
            }
        };
        let id = SessionId::from_uuid(replayed.expect("replay id"));
        let session = replay_open(&state, tenant_id, id, &claim).await?;
        Ok((StatusCode::OK, Json(SessionView::from(session))))
    }
    .await;
    respond(&state, "session.open", result).await
}

/// The fresh-open path: decide, create, remember the key, chain, commit —
/// all four in one transaction, for [`crate::workspaces`]'s reason.
async fn open_session(
    state: &AppState,
    tenant_id: TenantId,
    principal_id: &str,
    body: &OpenSessionBody,
    claim: &Claim,
) -> Result<Session> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    // The decision is taken at the scope the run will be anchored at, which
    // means resolving the workspace and project **first** — and a foreign one
    // is a 404 before any decision is taken, so a probe learns nothing.
    let anchor = anchor_scope(&mut tx, tenant_id, body.workspace_id, body.project_id).await?;
    let (authorized, _, _) = require(
        state,
        &mut tx,
        Action::SessionWrite,
        tenant_id,
        Subject::At(&anchor),
    )
    .await?;

    let session = sessions::create(
        &mut tx,
        &NewSession {
            id: SessionId::new(),
            tenant_id,
            workspace_id: body.workspace_id,
            project_id: body.project_id,
            principal_id: principal_id.to_owned(),
            client_name: body.client_name.clone(),
            client_version: body.client_version.clone(),
            client_installation_id: body.client_installation_id.clone(),
            external_session_id: body.external_session_id.clone(),
            agent_name: body.agent_name.clone(),
            model_name: body.model_name.clone(),
            repository_id: body.repository_id,
            branch: body.branch.clone(),
            task_summary: body.task_summary.clone(),
            metadata: body.metadata.clone().unwrap_or_else(|| json!({})),
        },
    )
    .await?;
    claim
        .remember(&mut tx, tenant_id, session.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SessionOpened,
        Resource::Session(session.id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::SessionWrite, &authorized),
            "session": session_image(&session),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(session)
}

/// The replay path: the same decision, then the session the key produced.
///
/// The decision is taken again on purpose. A replay is still a request to open
/// a session, and a caller whose permission was revoked between the first
/// attempt and the retry must be refused — a replay that skipped the PDP would
/// be a cached authorisation, which seed §2.2 forbids.
async fn replay_open(
    state: &AppState,
    tenant_id: TenantId,
    id: SessionId,
    claim: &Claim,
) -> Result<Session> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let session = sessions::get(&mut *tx, tenant_id, id)
        .await?
        .ok_or_else(|| crate::idempotency::vanished(claim, id.as_uuid()))?;
    let (authorized, resource, _) = require(
        state,
        &mut tx,
        Action::SessionWrite,
        tenant_id,
        Subject::Session(&session),
    )
    .await?;
    read_event(
        &mut tx,
        tenant_id,
        "session.open.replay",
        Action::SessionWrite,
        &authorized,
        resource,
        json!({"session_id": id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(session)
}

/// The governed scope a run in this workspace and project would be anchored
/// at, resolved from the rows rather than from the request.
///
/// A missing or foreign workspace or project is a 404 here, before any
/// decision, which is what keeps this route from being an existence oracle for
/// another tenant's inventory.
async fn anchor_scope(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    workspace_id: WorkspaceId,
    project_id: Option<ProjectId>,
) -> Result<synveda_types::scope::Scope> {
    let scope_id = match project_id {
        Some(project_id) => {
            let project = synveda_store::projects::get(&mut *tx, tenant_id, project_id)
                .await?
                .filter(|project| project.workspace_id == workspace_id)
                .ok_or_else(|| Error::NotFound {
                    entity: format!("project {project_id} in workspace {workspace_id}"),
                })?;
            project.scope_id
        }
        None => {
            synveda_store::workspaces::get(&mut *tx, tenant_id, workspace_id)
                .await?
                .ok_or_else(|| Error::NotFound {
                    entity: format!("workspace {workspace_id}"),
                })?
                .scope_id
        }
    };
    scopes::get(&mut *tx, tenant_id, scope_id)
        .await?
        .ok_or_else(|| Error::Internal {
            message: format!("workspace or project {scope_id} has no scope"),
        })
}

// ── List and read ────────────────────────────────────────────────────────────

/// `GET /v1/sessions` — the runs this caller may read, newest first.
#[utoipa::path(
    get,
    path = "/v1/sessions",
    operation_id = "list_sessions",
    tag = "sessions",
    params(ListParams),
    responses(
        (status = 200, description = "The sessions this caller may read", body = SessionList),
        (status = 400, description = "A filter outside its bounds", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `session.read`", body = ApiErrorBody),
        (status = 404, description = "No such scope in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let limit = match params.limit {
            Some(limit) if !(1..=MAX_LIST_LIMIT).contains(&limit) => {
                return Err(Error::Invalid {
                    message: format!("`limit` is 1..={MAX_LIST_LIMIT}, got {limit}"),
                });
            }
            Some(limit) => limit,
            None => DEFAULT_LIST_LIMIT,
        };
        let status = params
            .status
            .as_deref()
            .map(str::parse::<SessionStatus>)
            .transpose()?;

        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        // The gate is decided at the scope the listing is anchored at: the one
        // the caller named, or the tenant root. A named scope that is not this
        // tenant's is a 404 first, so this route is not an existence oracle.
        let anchor = match params.scope_id {
            Some(scope_id) => Some(
                scopes::get(&mut *tx, tenant_id, scope_id)
                    .await?
                    .ok_or_else(|| crate::request::not_found(scope_id))?,
            ),
            None => scopes::tenant_root(&mut *tx, tenant_id).await?,
        };
        // A tenant with no governed scopes at all has no session question to
        // ask: there is no resource for the PDP to decide about, no session
        // row that could exist, and therefore nothing disclosed. It is
        // answered rather than errored — the property CPR-9's audit asserted
        // for a caller who holds nothing — and chains no event, because a
        // disclosure of nothing is not one.
        let Some(anchor) = anchor else {
            commit(tx).await?;
            return Ok(Json(SessionList {
                sessions: Vec::new(),
                truncated: false,
            }));
        };
        // One gather serves both questions: the caller's principal, anchors
        // and groups are properties of the caller, and re-gathering per row
        // could observe them changing mid-response.
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&anchor),
            AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let gate_resource = Resource::Scope(anchor.id);
        let at_gate = authz::decide(&state, &input, Action::SessionRead, gate_resource);

        let (candidates, mut truncated) = sessions::list(
            &mut *tx,
            tenant_id,
            &SessionFilter {
                scope_id: params.scope_id,
                workspace_id: params.workspace_id,
                project_id: params.project_id,
                status,
            },
        )
        .await?;
        let (mut rows, at_row) = readable(&state, &mut tx, tenant_id, &input, candidates).await?;
        if rows.len() as i64 > limit {
            rows.truncate(limit as usize);
            truncated = true;
        }

        // Two questions, not one (CPR-9): the gate answers "may this caller
        // read this plane as such", the per-row verdicts answer "which of
        // these". Refused only when both say no — a caller who holds nothing
        // at the anchor and nothing below it is told so, which is the contract
        // an outsider has always had.
        let (authorized, resource) = match (at_gate, at_row) {
            (Ok(authorized), _) => (authorized, gate_resource),
            (Err(_), Some(authorized)) => (
                authorized,
                Resource::Session(rows.first().expect("a readable row").id),
            ),
            (Err(denial), None) => return Err(denial),
        };
        read_event(
            &mut tx,
            tenant_id,
            "session.list",
            Action::SessionRead,
            &authorized,
            resource,
            json!({"count": rows.len(), "truncated": truncated}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(SessionList {
            sessions: rows.into_iter().map(Into::into).collect(),
            truncated,
        }))
    }
    .await;
    respond(&state, "session.list", result).await
}

/// `GET /v1/sessions/{session_id}`.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}",
    operation_id = "get_session",
    tag = "sessions",
    params(("session_id" = String, Path, description = "The session's id")),
    responses(
        (status = 200, description = "The session", body = SessionView),
        (status = 403, description = "The PDP denied `session.read`", body = ApiErrorBody),
        (status = 404, description = "No such session in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<SessionId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (session, authorized, resource) =
            load(&state, &mut tx, tenant_id, id, Action::SessionRead).await?;
        read_event(
            &mut tx,
            tenant_id,
            "session.get",
            Action::SessionRead,
            &authorized,
            resource,
            json!({"session_id": id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(SessionView::from(session)))
    }
    .await;
    respond(&state, "session.get", result).await
}

// ── Events ───────────────────────────────────────────────────────────────────

/// `POST /v1/sessions/{session_id}/events` — append to the ledger.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/events",
    operation_id = "append_session_events",
    tag = "sessions",
    params(("session_id" = String, Path, description = "The session's id")),
    request_body = AppendEventsBody,
    responses(
        (status = 200, description = "Per-event outcomes", body = AppendResponse),
        (status = 400, description = "An empty or oversized batch, a repeated `client_event_id`, or an event outside its bounds", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `session.write`", body = ApiErrorBody),
        (status = 404, description = "No such session in this tenant", body = ApiErrorBody),
        (status = 409, description = "The session is closed", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn append_events(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    payload: std::result::Result<Json<AppendEventsBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (session, authorized, _) =
            load(&state, &mut tx, tenant_id, id, Action::SessionWrite).await?;

        let events: Vec<NewSessionEvent> = body
            .events
            .iter()
            .map(|event| NewSessionEvent {
                event_type: event.event_type,
                event_schema_version: event.event_schema_version,
                client_event_id: event.client_event_id.clone(),
                occurred_at: event.occurred_at,
                payload: event.payload.clone().unwrap_or_else(|| json!({})),
            })
            .collect();
        let appended = sessions::append_events(&mut tx, tenant_id, id, &events).await?;

        let written = appended
            .iter()
            .filter(|event| event.outcome == AppendOutcome::Appended)
            .count();
        // One event per batch, carrying counts and the sequence range rather
        // than the events themselves: a hundred-turn run would otherwise put
        // its whole transcript in the chain twice, and the chain is not the
        // transcript store. The per-type breakdown rides along, because "what
        // did that agent actually do" should be answerable from the chain
        // without reading the events.
        let mut by_type: BTreeMap<&str, usize> = BTreeMap::new();
        for event in &appended {
            if event.outcome == AppendOutcome::Appended {
                *by_type.entry(event.event.event_type.as_str()).or_default() += 1;
            }
        }
        let sequences: Vec<i64> = appended
            .iter()
            .filter(|event| event.outcome == AppendOutcome::Appended)
            .map(|event| event.event.sequence)
            .collect();
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::SessionEventsAppended,
            Resource::Session(session.id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::SessionWrite, &authorized),
                "session_id": session.id,
                "submitted": appended.len(),
                "appended": written,
                "duplicates": appended.len() - written,
                "by_type": by_type,
                "first_sequence": sequences.first(),
                "last_sequence": sequences.last(),
            }),
        )
        .await?;
        commit(tx).await?;

        Ok(Json(AppendResponse {
            appended: written,
            duplicates: appended.len() - written,
            events: appended
                .into_iter()
                .map(|appended| AppendedEventView {
                    outcome: appended.outcome.as_str().to_owned(),
                    event: appended.event.into(),
                })
                .collect(),
        }))
    }
    .await;
    respond(&state, "session.events.append", result).await
}

// ── End ──────────────────────────────────────────────────────────────────────

/// `POST /v1/sessions/{session_id}/end` — move a run through its close.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/end",
    operation_id = "end_session",
    tag = "sessions",
    params(("session_id" = String, Path, description = "The session's id")),
    request_body = EndSessionBody,
    responses(
        (status = 200, description = "The session, in its new state", body = SessionView),
        (status = 400, description = "A status this session cannot be moved to", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `session.write`", body = ApiErrorBody),
        (status = 404, description = "No such session in this tenant", body = ApiErrorBody),
        (status = 409, description = "The session is already past this state", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn end(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    payload: std::result::Result<Json<EndSessionBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        if body.status == SessionStatus::Active {
            return Err(Error::Invalid {
                message: "a session cannot be reopened; `status` is `ending`, `ended`, \
                          `abandoned` or `failed`"
                    .to_owned(),
            });
        }
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (before, authorized, _) =
            load(&state, &mut tx, tenant_id, id, Action::SessionWrite).await?;
        let after = sessions::transition(
            &mut tx,
            tenant_id,
            id,
            body.status,
            body.task_summary.as_deref(),
        )
        .await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::SessionEnded,
            Resource::Session(after.id).to_string(),
            Outcome::Success,
            json!({
                "authz": audit::decision_context(Action::SessionWrite, &authorized),
                // Both ends of the transition, so the chain says what a run was
                // doing when somebody closed it.
                "from": before.status.as_str(),
                "to": after.status.as_str(),
                "session": session_image(&after),
                "ended_at": after.ended_at,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(SessionView::from(after)))
    }
    .await;
    respond(&state, "session.end", result).await
}

// ── Timeline ─────────────────────────────────────────────────────────────────

/// `GET /v1/sessions/{session_id}/timeline` — the projection.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/timeline",
    operation_id = "get_session_timeline",
    tag = "sessions",
    params(
        ("session_id" = String, Path, description = "The session's id"),
        TimelineParams,
    ),
    responses(
        (status = 200, description = "The merged timeline", body = TimelineView),
        (status = 403, description = "The PDP denied `session.read`", body = ApiErrorBody),
        (status = 404, description = "No such session in this tenant", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn timeline(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Query(params): Query<TimelineParams>,
) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let after = params.after.unwrap_or(0).max(0);
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let (session, authorized, resource) =
            load(&state, &mut tx, tenant_id, id, Action::SessionRead).await?;

        let events =
            sessions::events(&mut *tx, tenant_id, id, after, MAX_TIMELINE_ENTRIES + 1).await?;
        let runs =
            sessions::context_runs(&mut *tx, tenant_id, id, MAX_TIMELINE_ENTRIES + 1).await?;
        let truncated =
            events.len() as i64 > MAX_TIMELINE_ENTRIES || runs.len() as i64 > MAX_TIMELINE_ENTRIES;

        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        let mut event_entries: Vec<TimelineEntry> = Vec::new();
        for event in events.into_iter().take(MAX_TIMELINE_ENTRIES as usize) {
            *counts
                .entry(event.event_type.as_str().to_owned())
                .or_default() += 1;
            event_entries.push(TimelineEntry {
                kind: "event".to_owned(),
                id: event.id.to_string(),
                at: event.occurred_at,
                event_type: Some(event.event_type.as_str().to_owned()),
                sequence: Some(event.sequence),
                summary: event_summary(&event),
            });
        }
        let run_entries: Vec<TimelineEntry> = runs
            .into_iter()
            .take(MAX_TIMELINE_ENTRIES as usize)
            .map(|run| TimelineEntry {
                kind: "context_run".to_owned(),
                id: run.id.to_string(),
                at: run.created_at,
                event_type: None,
                sequence: None,
                summary: match &run.query {
                    Some(query) => format!(
                        "context composed for {:?}: {} entries, {} tokens",
                        truncate(query, 60),
                        run.entry_count,
                        run.tokens
                    ),
                    None => format!(
                        "context composed: {} entries, {} tokens",
                        run.entry_count, run.tokens
                    ),
                },
            })
            .collect();
        let entries = merge_timeline(event_entries, run_entries);

        read_event(
            &mut tx,
            tenant_id,
            "session.timeline",
            Action::SessionRead,
            &authorized,
            resource,
            json!({"session_id": id, "entries": entries.len(), "truncated": truncated}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(TimelineView {
            session_id: session.id,
            entries,
            event_counts: counts,
            truncated,
        }))
    }
    .await;
    respond(&state, "session.timeline", result).await
}

/// Merges the two sources into one reading order.
///
/// A **merge**, not a sort, and the difference is the whole of what this
/// function is for.
///
/// The two instants come from two clocks. An event's `occurred_at` is the
/// *client's* — a buffered adapter delivers an hour late, and a machine whose
/// clock is wrong sends whatever it believes — while a context run's
/// `created_at` is this deployment's. Sorting both by instant therefore lets a
/// skewed client clock **reorder a transcript**, which is the one thing a
/// timeline must never do: `sequence` is server-assigned and monotonic, and it
/// is the only authority on what happened before what inside a run.
///
/// So the events keep their `sequence` order unconditionally, and the runs are
/// placed *among* them by instant. A run landing in the wrong gap because a
/// client's clock is off is a small wrong; a transcript reordered by the same
/// skew is a different account of what an agent did.
///
/// Both inputs arrive ordered — events by `sequence` from the store's own
/// query, runs by `created_at` — so this is one pass.
fn merge_timeline(events: Vec<TimelineEntry>, runs: Vec<TimelineEntry>) -> Vec<TimelineEntry> {
    let mut merged = Vec::with_capacity(events.len() + runs.len());
    let mut runs = runs.into_iter().peekable();
    for event in events {
        while runs.peek().is_some_and(|run| run.at <= event.at) {
            merged.push(runs.next().expect("peeked"));
        }
        merged.push(event);
    }
    merged.extend(runs);
    merged
}

/// One line about an event, from the fields its family actually carries.
///
/// Best-effort by design: a payload is the client's shape, so this reads the
/// two or three keys each family conventionally uses and falls back to the
/// family name. It is a **reading** surface — a client that wants the payload
/// has the payload.
fn event_summary(event: &SessionEvent) -> String {
    let text = |key: &str| event.payload.get(key).and_then(|value| value.as_str());
    match event.event_type {
        SessionEventType::MessageUser | SessionEventType::MessageAssistant => {
            text("text").or_else(|| text("content")).map_or_else(
                || event.event_type.as_str().to_owned(),
                |t| truncate(t, 120),
            )
        }
        SessionEventType::ToolInvoked | SessionEventType::ToolResult => {
            text("tool").or_else(|| text("name")).map_or_else(
                || event.event_type.as_str().to_owned(),
                |t| format!("{}: {t}", event.event_type.as_str()),
            )
        }
        SessionEventType::FileRead | SessionEventType::FileChanged => text("path").map_or_else(
            || event.event_type.as_str().to_owned(),
            |p| format!("{}: {p}", event.event_type.as_str()),
        ),
        SessionEventType::CommandExecuted => {
            text("command").map_or_else(|| "command.executed".to_owned(), |c| truncate(c, 120))
        }
        SessionEventType::SkillLoaded => {
            text("name").map_or_else(|| "skill.loaded".to_owned(), |n| format!("skill: {n}"))
        }
        SessionEventType::AdapterWarning => {
            text("message").map_or_else(|| "adapter.warning".to_owned(), |m| truncate(m, 200))
        }
        _ => event.event_type.as_str().to_owned(),
    }
}

/// `text`, at most `max` characters, with an ellipsis when it was longer.
///
/// Counts characters rather than bytes: slicing a UTF-8 string on a byte
/// boundary panics, and a summary line is exactly the place a multi-byte
/// character arrives.
fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

// ── Context runs ─────────────────────────────────────────────────────────────

/// `POST /v1/sessions/{session_id}/context-runs` — compose context for a run.
///
/// **The final shape of this endpoint** (ADR-0076 decision 7). What it does
/// today is decide `SessionWrite` at the session, call the existing retrieval
/// engine over the session's scope chain and the caller's own, persist the
/// identity and the rendered block, and chain the watermark. Prompt 18 adds
/// the explainability — which scopes were considered, which were denied, why
/// each entry made the cut — behind this same request and this same envelope.
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/context-runs",
    operation_id = "create_context_run",
    tag = "sessions",
    params(
        ("session_id" = String, Path, description = "The session's id"),
        ("Idempotency-Key" = String, Header,
         description = "Required. A unique value per request, reused verbatim on retry."),
    ),
    request_body = ContextRunBody,
    responses(
        (status = 201, description = "Composed", body = ContextRunView),
        (status = 200, description = "This key already composed this run", body = ContextRunView),
        (status = 400, description = "Malformed body, or no `Idempotency-Key`", body = ApiErrorBody),
        (status = 403, description = "The PDP denied `session.write`", body = ApiErrorBody),
        (status = 404, description = "No such session in this tenant", body = ApiErrorBody),
        (status = 409, description = "The key was reused for a different request", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn create_context_run(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<ContextRunBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let tenant_id = tenant_id()?;
        let subject = subject()?;
        if let Some(query) = &body.query {
            let chars = query.chars().count();
            if chars == 0 || chars > MAX_QUERY_CHARS {
                return Err(Error::Invalid {
                    message: format!("`query` is 1..={MAX_QUERY_CHARS} characters"),
                });
            }
        }
        if body.budget_tokens == Some(0) {
            return Err(Error::Invalid {
                message: "`budget_tokens` is at least 1".to_owned(),
            });
        }
        let claim = Claim::from_headers(
            &headers,
            "session.context_run",
            &subject,
            &json!({
                "route": "POST /v1/sessions/{session_id}/context-runs",
                "session_id": id,
                "query": body.query,
                "budget_tokens": body.budget_tokens,
                "max_sensitivity": body.max_sensitivity,
            }),
        )?;

        let replayed = match crate::idempotency::dispatch(&state.pool, tenant_id, &claim).await? {
            Dispatch::Replay(run_id) => Some(run_id),
            Dispatch::Create => {
                match compose_for_session(&state, tenant_id, &subject, id, &body, &claim).await {
                    Ok(run) => return Ok((StatusCode::CREATED, Json(ContextRunView::from(run)))),
                    Err(conflict @ Error::Conflict { .. }) => Some(
                        crate::idempotency::resolve_conflict(
                            &state.pool,
                            tenant_id,
                            &claim,
                            conflict,
                        )
                        .await?,
                    ),
                    Err(other) => return Err(other),
                }
            }
        };
        let run_id = ContextRunId::from_uuid(replayed.expect("replay id"));
        let run = replay_context_run(&state, tenant_id, id, run_id, &claim).await?;
        Ok((StatusCode::OK, Json(ContextRunView::from(run))))
    }
    .await;
    respond(&state, "session.context_run", result).await
}

/// The composition itself.
///
/// Two tenant transactions bracket the embed call, because no transaction
/// spans a network call (the MEM-3 rule `/v1/inject` follows): the first
/// decides and gathers the plan, the second searches, composes, persists and
/// chains.
async fn compose_for_session(
    state: &AppState,
    tenant_id: TenantId,
    principal_id: &str,
    session_id: SessionId,
    body: &ContextRunBody,
    claim: &Claim,
) -> Result<ContextRun> {
    // Transaction 1: the decision and the plan. Read-only, and dropped before
    // the embed call.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let session = sessions::get(&mut *tx, tenant_id, session_id)
        .await?
        .ok_or_else(|| session_not_found(session_id))?;
    let (authorized, _, input) = require(
        state,
        &mut tx,
        Action::SessionWrite,
        tenant_id,
        Subject::Session(&session),
    )
    .await?;

    // The composition universe: the caller's own chain, plus the **session's**
    // scope chain as widened candidates.
    //
    // Two sources, and both are needed. The caller's own chain is what
    // `/v1/inject` composes from, and it is where somebody's private material
    // lives. The session's chain is the project, its workspace and the org
    // above them — the material the run is actually about, which is not on the
    // caller's own chain at all. Handing the session's chain in as `chain`
    // instead would have silently made the caller's own notes unreachable from
    // every project session.
    //
    // `candidates` is the mechanism CTX-5 built for exactly this (ADR-0042
    // decision 2): scopes that could contribute to *this* request, each
    // decided under its own chain and pack. Nothing here widens what a pack
    // permits — every candidate is still decided, per tier, by the same walk.
    let principal_ids: Vec<ScopeId> = input.principal_scopes.iter().map(|node| node.id).collect();
    let mut assignments: Vec<PolicyAssignment> = input.assignments.clone();
    for assignment in
        synveda_store::policy_assignments::for_scopes(&mut *tx, tenant_id, &principal_ids).await?
    {
        if !assignments
            .iter()
            .any(|held| held.scope_id == assignment.scope_id)
        {
            assignments.push(assignment);
        }
    }
    let session_chain: Vec<ScopeNode> = input.chain.to_vec();
    let own_chain: Vec<ScopeNode> = input.principal_scopes.to_vec();
    drop(tx);

    let candidates: Vec<CandidateScope<'_>> = session_chain
        .iter()
        .enumerate()
        // A scope already on the caller's own chain is skipped: the nearer
        // source wins, which is the same rule `MemoryReadInputs::candidates`
        // documents for recall.
        .filter(|(_, node)| !own_chain.iter().any(|own| own.id == node.id))
        .map(|(index, node)| CandidateScope {
            scope_id: node.id,
            chain: &session_chain[index..],
            assignments: &assignments,
        })
        .collect();

    let plan = composition_plan(
        &state.pdp,
        &MemoryReadInputs {
            principal: &input.principal,
            chain: &own_chain,
            anchors: input.anchors.as_slice(),
            groups: &input.groups,
            assignments: &assignments,
            default_pack: input.default_pack.as_deref(),
            // Lapses are not gathered here, which can only ever *narrow* what
            // composes. Widening a session's context by a standing relaxation
            // is Prompt 26's, and doing it half-way now would be a second
            // relaxation path to keep true.
            lapses: &[],
            lapsed: &[],
            candidates: &candidates,
        },
    )?;

    let budget_tokens = match body.budget_tokens {
        Some(requested) => plan.budget_tokens.min(requested),
        None => plan.budget_tokens,
    };
    let as_of = Utc::now();
    let mut degraded: Vec<String> = Vec::new();

    // The embed call: outside any transaction, and a failure drops the dense
    // leg rather than the request — `/v1/inject`'s posture, for its reason.
    let query = body
        .query
        .as_ref()
        .filter(|_| !plan.scopes.is_empty())
        .cloned();
    let vector = match &query {
        Some(task) => {
            match tokio::time::timeout(
                state.inject_embed_timeout,
                state.embedder.embed(std::slice::from_ref(task)),
            )
            .await
            {
                Ok(Ok(mut vectors)) if !vectors.is_empty() => Some(QueryVector {
                    model: state.embedder.model().to_owned(),
                    vector: vectors.remove(0),
                }),
                _ => {
                    tracing::warn!("query embed unavailable; degrading to sparse-only");
                    degraded.push("embedder".to_owned());
                    None
                }
            }
        }
        None => None,
    };

    // Transaction 2: search, compose, persist, chain.
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let relevance = match &query {
        Some(task) => {
            let request = SearchRequest {
                query: task.clone(),
                vector,
                filter: SearchFilter {
                    tiers: plan
                        .scopes
                        .iter()
                        .flat_map(|scope| {
                            synveda_types::ScopeTier::expand(scope.scope_id, &scope.sensitivities)
                        })
                        .collect(),
                },
                limit: RELEVANCE_LIMIT,
                per_leg: RELEVANCE_LIMIT,
                at: as_of,
            };
            match hybrid_search(&mut tx, &state.search_index, tenant_id, &request).await {
                Ok(results) => Some(
                    results
                        .into_iter()
                        .map(|retrieved| retrieved.record.id)
                        .collect(),
                ),
                Err(error) => {
                    tracing::warn!(error = %error, "hybrid search failed; composing unranked");
                    degraded.push("retrieval".to_owned());
                    None
                }
            }
        }
        None => None,
    };

    let mut request = ComposeRequest::new(plan.scopes, budget_tokens, as_of);
    if let Some(ceiling) = body.max_sensitivity {
        request = request.narrowed_to(ceiling);
    }
    request.relevance = relevance;
    let block = compose(&mut tx, tenant_id, &request).await?;

    let run = sessions::record_context_run(
        &mut tx,
        tenant_id,
        &NewContextRun {
            id: ContextRunId::new(),
            session_id,
            scope_id: session.scope_id,
            principal_id: principal_id.to_owned(),
            query: body.query.clone(),
            rendered: block.text.clone(),
            block_hash: block.block_hash.clone(),
            tokens: i32::try_from(block.tokens).unwrap_or(i32::MAX),
            budget_tokens: i32::try_from(block.budget_tokens).unwrap_or(i32::MAX),
            entry_count: i32::try_from(block.entries.len()).unwrap_or(i32::MAX),
            degraded: degraded.clone(),
        },
    )
    .await?;
    claim.remember(&mut tx, tenant_id, run.id.as_uuid()).await?;
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::SessionContextComposed,
        Resource::Session(session_id).to_string(),
        Outcome::Success,
        json!({
            "authz": audit::decision_context(Action::SessionWrite, &authorized),
            "session_id": session_id,
            "context_run_id": run.id,
            // The watermark, never the block: the chain records *what an agent
            // was given*, and the run row holds what that was.
            "block_hash": run.block_hash,
            "entries": run.entry_count,
            "tokens": run.tokens,
            "budget_tokens": run.budget_tokens,
            "degraded": run.degraded,
            "scopes": plan
                .decisions
                .iter()
                .map(|decision| json!({
                    "scope_id": decision.scope_id,
                    "allowed": decision.allowed,
                    "pack": format!("{}@{}", decision.pack_name, decision.pack_version),
                }))
                .collect::<Vec<_>>(),
            "idempotency_key": claim.key,
        }),
    )
    .await?;
    commit(tx).await?;
    Ok(run)
}

/// The replay path: the same decision, then the run the key produced.
async fn replay_context_run(
    state: &AppState,
    tenant_id: TenantId,
    session_id: SessionId,
    run_id: ContextRunId,
    claim: &Claim,
) -> Result<ContextRun> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let (_, authorized, resource) =
        load(state, &mut tx, tenant_id, session_id, Action::SessionWrite).await?;
    let run = sessions::context_run(&mut *tx, tenant_id, run_id)
        .await?
        .filter(|run| run.session_id == session_id)
        .ok_or_else(|| crate::idempotency::vanished(claim, run_id.as_uuid()))?;
    read_event(
        &mut tx,
        tenant_id,
        "session.context_run.replay",
        Action::SessionWrite,
        &authorized,
        resource,
        json!({"session_id": session_id, "context_run_id": run_id, "idempotency_key": claim.key}),
    )
    .await?;
    commit(tx).await?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract's own statement of ADR-0076 decision 8, asserted where a
    /// reader adding a field will meet it: a body that names a tenant or an
    /// acting principal is refused by serde before any handler runs.
    #[test]
    fn a_client_cannot_submit_a_tenant_or_an_acting_principal() {
        let base = json!({"workspace_id": uuid::Uuid::nil(), "client_name": "claude-code"});
        serde_json::from_value::<OpenSessionBody>(base.clone()).expect("the ordinary body parses");
        for forbidden in ["tenant_id", "principal_id", "scope_id", "id"] {
            let mut body = base.clone();
            body[forbidden] = json!("anything");
            let parsed = serde_json::from_value::<OpenSessionBody>(body);
            assert!(
                parsed.is_err(),
                "`{forbidden}` must be refused, not ignored"
            );
        }
    }

    #[test]
    fn an_event_body_defaults_its_schema_version_and_refuses_an_unknown_type() {
        let body: NewEventBody = serde_json::from_value(json!({
            "event_type": "message.user",
            "client_event_id": "e1",
            "occurred_at": "2026-08-23T10:00:00Z",
        }))
        .expect("the minimal event parses");
        assert_eq!(body.event_schema_version, CURRENT_EVENT_SCHEMA_VERSION);
        assert_eq!(body.event_type, SessionEventType::MessageUser);

        assert!(
            serde_json::from_value::<NewEventBody>(json!({
                "event_type": "message.system",
                "client_event_id": "e1",
                "occurred_at": "2026-08-23T10:00:00Z",
            }))
            .is_err(),
            "the event vocabulary is closed"
        );
    }

    fn entry(kind: &str, at: &str, sequence: Option<i64>, id: &str) -> TimelineEntry {
        TimelineEntry {
            kind: kind.to_owned(),
            id: id.to_owned(),
            at: at.parse().expect("an instant"),
            event_type: None,
            sequence,
            summary: String::new(),
        }
    }

    /// The property the merge exists for: a client whose clock is wrong can
    /// misplace a **context run**, and can never reorder the **transcript**.
    ///
    /// The events below arrive in `sequence` order with `occurred_at` values
    /// that run backwards — which is what a machine with a bad clock, or an
    /// adapter replaying a buffer, actually sends. A sort by instant would
    /// reverse them and produce a different account of what the agent did.
    #[test]
    fn a_skewed_client_clock_cannot_reorder_a_transcript() {
        let events = vec![
            entry("event", "2026-08-23T10:00:00Z", Some(1), "e1"),
            entry("event", "2026-08-23T09:00:00Z", Some(2), "e2"),
            entry("event", "2026-08-23T11:00:00Z", Some(3), "e3"),
        ];
        let runs = vec![entry("context_run", "2026-08-23T10:30:00Z", None, "r1")];
        let merged = merge_timeline(events, runs);
        assert_eq!(
            merged
                .iter()
                .filter(|item| item.kind == "event")
                .map(|item| item.sequence.expect("a sequence"))
                .collect::<Vec<_>>(),
            vec![1, 2, 3],
            "events keep the server's own order, whatever the client's clock said"
        );
        // And the run lands in a gap chosen by instant.
        assert_eq!(merged[2].id, "r1");
    }

    #[test]
    fn a_run_before_every_event_leads_and_one_after_them_trails() {
        let events = vec![
            entry("event", "2026-08-23T10:00:00Z", Some(1), "e1"),
            entry("event", "2026-08-23T10:01:00Z", Some(2), "e2"),
        ];
        let runs = vec![
            entry("context_run", "2026-08-23T09:59:00Z", None, "before"),
            entry("context_run", "2026-08-23T10:02:00Z", None, "after"),
        ];
        let merged = merge_timeline(events, runs);
        assert_eq!(
            merged
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["before", "e1", "e2", "after"]
        );
        // Neither source alone loses anything.
        assert_eq!(merge_timeline(Vec::new(), Vec::new()).len(), 0);
        assert_eq!(
            merge_timeline(
                Vec::new(),
                vec![entry("context_run", "2026-08-23T10:00:00Z", None, "only")]
            )
            .len(),
            1
        );
    }

    #[test]
    fn a_summary_never_splits_a_character_and_says_when_it_cut() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 3), "hel…");
        // The case a byte-slicing implementation panics on.
        assert_eq!(truncate("日本語テキスト", 3), "日本語…");
    }

    #[test]
    fn a_summary_reads_the_key_its_family_carries_and_falls_back_by_name() {
        let event = |kind: SessionEventType, payload: serde_json::Value| SessionEvent {
            id: synveda_types::SessionEventId::new(),
            tenant_id: TenantId::new(),
            session_id: SessionId::new(),
            event_type: kind,
            event_schema_version: 1,
            client_event_id: "e".to_owned(),
            sequence: 1,
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            payload,
            payload_hash: "ab".repeat(32),
        };
        assert_eq!(
            event_summary(&event(
                SessionEventType::MessageUser,
                json!({"text": "fix it"})
            )),
            "fix it"
        );
        assert_eq!(
            event_summary(&event(
                SessionEventType::ToolInvoked,
                json!({"tool": "grep"})
            )),
            "tool.invoked: grep"
        );
        assert_eq!(
            event_summary(&event(
                SessionEventType::FileChanged,
                json!({"path": "a.rs"})
            )),
            "file.changed: a.rs"
        );
        // An empty or unexpected payload is never a panic and never a blank
        // line: the family's own name is the honest fallback.
        assert_eq!(
            event_summary(&event(SessionEventType::MessageUser, json!({}))),
            "message.user"
        );
        assert_eq!(
            event_summary(&event(SessionEventType::SessionStarted, json!({}))),
            "session.started"
        );
    }
}

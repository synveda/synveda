//! Deterministic working state from independently authorized exact spans.

use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    response::Response,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::Action;
use synveda_store::{artifact_control, knowledge, rls, sessions, task_state as store};
use synveda_types::session::{SessionEvent, SessionEventType};
use synveda_types::task_state::*;
use synveda_types::{
    ContextRunId, Error, ImportArtifactId, KnowledgeItemId, KnowledgeRevisionId, Result,
    SessionEventId, SessionId, TenantId,
};
use utoipa::ToSchema;

use crate::{app::AppState, audit, request};

/// Exact UTF-8 byte range in already admitted source text.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskSpanBody {
    /// Inclusive byte offset.
    pub start: u32,
    /// Exclusive byte offset; at most 1024 bytes may be copied.
    pub end: u32,
}
impl From<TaskSpanBody> for TaskTextSpan {
    fn from(value: TaskSpanBody) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

/// Closed source pointer. Text and epistemic class are server-derived.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "source_kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskEvidenceBody {
    /// A supported string field in this same Session's admitted event.
    SessionEvent {
        /// Exact server event.
        #[schema(value_type=String,format="uuid")]
        event_id: SessionEventId,
        /// Admitted redacted payload hash.
        payload_hash: String,
        /// `text` or `tool_text`; the latter needs `tool_index`.
        #[schema(value_type=String)]
        field: TaskTextField,
        /// Bounded tool index, only for `tool_text`.
        tool_index: Option<u8>,
        /// Exact source bytes.
        span: TaskSpanBody,
    },
    /// Exact current reviewed Knowledge body.
    KnowledgeRevision {
        /// Stable item.
        #[schema(value_type=String,format="uuid")]
        item_id: KnowledgeItemId,
        /// Exact current revision.
        #[schema(value_type=String,format="uuid")]
        revision_id: KnowledgeRevisionId,
        /// Exact revision content hash.
        content_hash: String,
        /// Exact body bytes.
        span: TaskSpanBody,
    },
    /// Actual admitted import source, independently project-authorized.
    ImportArtifact {
        /// Exact source artifact.
        #[schema(value_type=String,format="uuid")]
        artifact_id: ImportArtifactId,
        /// Exact source content hash.
        content_hash: String,
        /// Exact body bytes.
        span: TaskSpanBody,
    },
}
impl From<TaskEvidenceBody> for TaskEvidenceRef {
    fn from(value: TaskEvidenceBody) -> Self {
        match value {
            TaskEvidenceBody::SessionEvent {
                event_id,
                payload_hash,
                field,
                tool_index,
                span,
            } => Self::SessionEvent {
                event_id,
                payload_hash,
                field,
                tool_index,
                span: span.into(),
            },
            TaskEvidenceBody::KnowledgeRevision {
                item_id,
                revision_id,
                content_hash,
                span,
            } => Self::KnowledgeRevision {
                item_id,
                revision_id,
                content_hash,
                span: span.into(),
            },
            TaskEvidenceBody::ImportArtifact {
                artifact_id,
                content_hash,
                span,
            } => Self::ImportArtifact {
                artifact_id,
                content_hash,
                span: span.into(),
            },
        }
    }
}

/// Declared semantics over exact evidence, without a free-text summary field.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskStatementBody {
    /// objective, constraint, decision, proposal, observation, work, question,
    /// next_action, artifact or verification.
    #[schema(value_type=String)]
    pub kind: TaskStatementKind,
    /// planned, attempted, succeeded, failed or unknown. This is a declaration.
    #[schema(value_type=Option<String>)]
    pub declared_outcome: Option<TaskOutcome>,
    /// Exact source pointer; the server supplies text and evidence class.
    pub evidence: TaskEvidenceBody,
}

/// Exact prior state for serialized replacement, not inherited source evidence.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskStatePrecondition {
    /// Exact preceding server TaskState.
    #[schema(value_type=String,format="uuid")]
    pub event_id: SessionEventId,
    /// Exact payload hash.
    pub payload_hash: String,
}

/// Exact existing context snapshot in this same Session.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskContextBody {
    /// Saved run, independently reauthorized.
    #[schema(value_type=String,format="uuid")]
    pub context_run_id: ContextRunId,
    /// Exact block hash.
    pub block_hash: String,
}

/// Exact event containing the native `context.checkout` observation.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCheckoutBody {
    /// Source event in this Session.
    #[schema(value_type=String,format="uuid")]
    pub event_id: SessionEventId,
    /// Exact redacted payload hash.
    pub payload_hash: String,
}

/// Bounded deterministic replacement of a Session's working state.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskStateBody {
    /// Exactly 1.
    pub schema_version: u8,
    /// Stable retry key for this derivation.
    pub client_event_id: String,
    /// Required to replace an existing available head; exact retries reuse it.
    pub expected_previous: Option<TaskStatePrecondition>,
    /// At most 32 exact-evidence statements.
    pub statements: Vec<TaskStatementBody>,
    /// Optional already saved context, without recursive handoff ancestry.
    pub context_snapshot: Option<TaskContextBody>,
    /// Optional source-bound client checkout observation.
    pub checkout: Option<TaskCheckoutBody>,
}

/// Immutable event receipt. The caller can immediately hand its hash to a receiver.
#[derive(Serialize, ToSchema)]
pub struct TaskStateView {
    /// appended or duplicate.
    pub outcome: String,
    /// Admitted server-derived event, under current source authority.
    pub event: super::SessionEventView,
}

pub(crate) fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}
fn unavailable() -> Error {
    Error::NotFound {
        entity: "Task evidence".to_owned(),
    }
}
fn changed() -> Error {
    Error::Conflict {
        message: "Task state or evidence changed".to_owned(),
    }
}
fn invalid() -> Error {
    Error::Invalid {
        message: "Invalid task-state evidence or bounds".to_owned(),
    }
}
fn hidden_source(error: Error) -> Error {
    match error {
        Error::PolicyDenied { .. } | Error::NotFound { .. } => unavailable(),
        error => error,
    }
}

fn copy_span(text: &str, span: &TaskTextSpan) -> Result<String> {
    let start = span.start as usize;
    let end = span.end as usize;
    if end <= start || end - start > MAX_TASK_STATEMENT_BYTES {
        return Err(invalid());
    }
    text.get(start..end)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(invalid)
}

fn event_text(event: &SessionEvent, field: TaskTextField, index: Option<u8>) -> Result<&str> {
    if !matches!(
        event.event_type,
        SessionEventType::MessageUser
            | SessionEventType::MessageAssistant
            | SessionEventType::ToolResult
            | SessionEventType::CommandExecuted
            | SessionEventType::MemoryAsserted
            | SessionEventType::FileRead
            | SessionEventType::FileChanged
    ) {
        return Err(invalid());
    }
    match (field, index) {
        (TaskTextField::Text, None) => event.payload.get("text").and_then(Value::as_str),
        (TaskTextField::ToolText, Some(index)) if index < 32 => event
            .payload
            .get("tools")
            .and_then(Value::as_array)
            .and_then(|tools| tools.get(index as usize))
            .and_then(|tool| tool.get("text"))
            .and_then(Value::as_str),
        _ => None,
    }
    .ok_or_else(invalid)
}

pub(crate) async fn admitted_event(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    id: SessionEventId,
    hash: &str,
) -> Result<SessionEvent> {
    if !hash_valid(hash) {
        return Err(invalid());
    }
    let event = sessions::event(&mut *tx, tenant, session, id)
        .await?
        .ok_or_else(unavailable)?;
    if event.payload_hash != hash {
        return Err(changed());
    }
    if sessions::payload_hash(&event.payload) != event.payload_hash {
        return Err(changed());
    }
    if !sessions::withheld_event_ids(&mut *tx, tenant, session, &[id])
        .await?
        .is_empty()
    {
        return Err(unavailable());
    }
    Ok(event)
}

async fn resolve_evidence(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    evidence: &TaskEvidenceRef,
) -> Result<(String, TaskEvidenceClass)> {
    match evidence {
        TaskEvidenceRef::SessionEvent {
            event_id,
            payload_hash,
            field,
            tool_index,
            span,
        } => {
            crate::knowledge_api::authorize_historical(
                state,
                tx,
                tenant,
                "session_event",
                event_id.as_uuid(),
            )
            .await?;
            let event = admitted_event(tx, tenant, session, *event_id, payload_hash).await?;
            let text = copy_span(event_text(&event, *field, *tool_index)?, span)?;
            let class = if event.event_type == SessionEventType::MessageUser {
                TaskEvidenceClass::UserDeclared
            } else {
                TaskEvidenceClass::ClientReported
            };
            Ok((text, class))
        }
        TaskEvidenceRef::KnowledgeRevision {
            item_id,
            revision_id,
            content_hash,
            span,
        } => {
            if !hash_valid(content_hash) {
                return Err(invalid());
            }
            let current = knowledge::current(&mut *tx, tenant, *item_id)
                .await?
                .ok_or_else(unavailable)?;
            let allowed = crate::knowledge_api::authorize_snapshot(state, tx, tenant, &current)
                .await
                .map_err(hidden_source)?;
            if current.revision.id != *revision_id || current.revision.content_hash != *content_hash
            {
                return Err(changed());
            }
            if !crate::context_api::current_active_at(&current, Utc::now()) {
                return Err(unavailable());
            }
            audit::record(
                tx,
                tenant,
                AuditAction::AuthzDecision,
                format!("knowledge_item:{item_id}"),
                Outcome::Allow,
                json!({"authz":audit::decision_context(Action::KnowledgeRead,&allowed)}),
            )
            .await?;
            let (_, incomplete) = crate::context_api::visible_sources_with_policy(
                state,
                tx,
                tenant,
                &current.revision,
            )
            .await?;
            if incomplete {
                return Err(unavailable());
            }
            Ok((
                copy_span(&current.revision.content.body_markdown, span)?,
                TaskEvidenceClass::PublishedKnowledge,
            ))
        }
        TaskEvidenceRef::ImportArtifact {
            artifact_id,
            content_hash,
            span,
        } => {
            if !hash_valid(content_hash) {
                return Err(invalid());
            }
            let address = artifact_control::address(&mut *tx, tenant, *artifact_id)
                .await?
                .ok_or_else(unavailable)?;
            crate::artifact_control::authorize(state, tx, tenant, &address, Action::KnowledgeRead)
                .await
                .map_err(hidden_source)?;
            let artifact = store::artifact_text(tx, tenant, *artifact_id)
                .await?
                .ok_or_else(unavailable)?;
            if artifact.content_hash != *content_hash {
                return Err(changed());
            }
            Ok((
                copy_span(&artifact.body_markdown, span)?,
                TaskEvidenceClass::ImportedSource,
            ))
        }
    }
}

async fn resolve_checkout(
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    source: &TaskCheckoutRef,
) -> Result<TaskCheckout> {
    let event = admitted_event(tx, tenant, session, source.event_id, &source.payload_hash).await?;
    let checkout = event
        .payload
        .get("context")
        .and_then(|context| context.get("checkout"))
        .and_then(Value::as_object)
        .ok_or_else(invalid)?;
    let label = |key: &str, max: usize| -> Result<Option<String>> {
        match checkout.get(key) {
            None => Ok(None),
            Some(Value::String(s))
                if !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control) =>
            {
                Ok(Some(s.clone()))
            }
            _ => Err(invalid()),
        }
    };
    let worktree_ref = label("ref", 128)?.ok_or_else(invalid)?;
    let branch = label("branch", 200)?;
    let commit = label("commit", 64)?;
    let tree = label("tree", 64)?;
    let dirty_digest = label("dirty_digest", 64)?;
    if commit.iter().chain(&tree).any(|h| {
        !matches!(h.len(), 40 | 64)
            || !h
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    }) || dirty_digest.as_deref().is_some_and(|h| !hash_valid(h))
    {
        return Err(invalid());
    }
    let dirty = match checkout.get("dirty") {
        None => None,
        Some(Value::Bool(v)) => Some(*v),
        _ => return Err(invalid()),
    };
    Ok(TaskCheckout {
        source: source.clone(),
        worktree_ref,
        branch,
        commit,
        tree,
        dirty,
        dirty_digest,
    })
}

/// Reauthorize exact copied sources; no transcript event is trusted as host proof.
pub(crate) async fn verified(
    state: &AppState,
    tx: &mut PgConnection,
    tenant: TenantId,
    session: SessionId,
    event: &SessionEvent,
) -> Result<Option<TaskState>> {
    if event.event_type != SessionEventType::TaskState || event.session_id != session {
        return Ok(None);
    }
    let result = async {
        crate::knowledge_api::authorize_historical(
            state,
            tx,
            tenant,
            "session_event",
            event.id.as_uuid(),
        )
        .await?;
        let (loaded, _, _) = super::load(state, tx, tenant, session, Action::SessionRead).await?;
        admitted_event(tx, tenant, session, event.id, &event.payload_hash).await?;
        let parsed: TaskState =
            serde_json::from_value(event.payload.clone()).map_err(|_| invalid())?;
        if parsed.schema_version != 1
            || parsed.method != "exact_evidence_task_state_v1"
            || !hash_valid(&parsed.request_hash)
            || parsed.repository_id != loaded.repository_id
            || parsed.statements.is_empty()
            || parsed.statements.len() > MAX_TASK_STATEMENTS
        {
            return Err(invalid());
        }
        for entry in &parsed.statements {
            let (text, class) =
                resolve_evidence(state, tx, tenant, session, &entry.evidence).await?;
            if text != entry.text || class != entry.evidence_class {
                return Err(changed());
            }
        }
        if let Some(checkout) = &parsed.checkout
            && resolve_checkout(tx, tenant, session, &checkout.source).await? != *checkout
        {
            return Err(changed());
        }
        if let Some(snapshot) = &parsed.context_snapshot
            && !Box::pin(crate::context_api::task_snapshot_available(
                state, tx, tenant, session, snapshot,
            ))
            .await?
        {
            return Err(unavailable());
        }
        Ok(parsed)
    }
    .await;
    match result {
        Ok(parsed) => Ok(Some(parsed)),
        Err(
            Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::NotFound { .. }
            | Error::PolicyDenied { .. },
        ) => Ok(None),
        Err(error) => Err(error),
    }
}

#[utoipa::path(post,path="/v1/sessions/{session_id}/task-state",operation_id="derive_session_task_state",tag="sessions",
 request_body=TaskStateBody,params(("session_id"=String,Path)),responses((status=200,description="Exact admitted event receipt",body=TaskStateView),
 (status=400,description="Unsupported declaration, selector or bounds",body=crate::workspaces::ApiErrorBody),
 (status=403,description="Current source or Session authority denied",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable Session or source",body=crate::workspaces::ApiErrorBody),
 (status=409,description="Changed state, source or retry key",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn derive(
    State(state): State<AppState>,
    Path(session): Path<SessionId>,
    payload: std::result::Result<Json<TaskStateBody>, JsonRejection>,
) -> Response {
    let result=async {
        let body=request::body(payload)?;
        synveda_types::session::validate_client_event_id(&body.client_event_id)?;
        if body.schema_version!=1 || body.statements.is_empty() || body.statements.len()>MAX_TASK_STATEMENTS {return Err(invalid());}
        let input=serde_json::to_value(&body).map_err(|_|invalid())?;
        if synveda_types::json::canonicalise(&input).to_string().len()>65_536 {return Err(invalid());}
        let request_hash=sessions::payload_hash(&input);
        let client=format!("task-state:{}",blake3::hash(body.client_event_id.as_bytes()).to_hex());
        let tenant=request::tenant_id()?;let mut tx=rls::begin_tenant_tx(&state.pool,tenant).await?;
        let (loaded,allowed,_)=super::load(&state,&mut tx,tenant,session,Action::SessionWrite).await?;
        let (_,read_allowed,read_resource)=super::load(&state,&mut tx,tenant,session,Action::SessionRead).await?;
        super::read_event(&mut tx,tenant,"session.task_state.source",Action::SessionRead,&read_allowed,read_resource,json!({"session_id":session})).await?;
        store::lock_session(&mut tx,tenant,session).await?;
        if let Some(retry)=store::by_client(&mut tx,tenant,session,&client).await? {
            if retry.request_hash.as_deref()!=Some(&request_hash) {return Err(changed());}
            let event=sessions::event(&mut *tx,tenant,session,SessionEventId::from_uuid(retry.id)).await?.ok_or_else(unavailable)?;
            if verified(&state,&mut tx,tenant,session,&event).await?.is_none() {return Err(unavailable());}
            request::commit(tx).await?;
            return Ok(Json(TaskStateView {outcome:"duplicate".to_owned(),event:event.into()}));
        }
        let head=store::latest(&mut tx,tenant,session,None).await?;
        let head_available=if let Some(head)=&head {
            if let Some(event)=sessions::event(&mut *tx,tenant,session,SessionEventId::from_uuid(head.id)).await? {
                verified(&state,&mut tx,tenant,session,&event).await?.is_some()
            } else {false}
        } else {false};
        match (&body.expected_previous,&head) {
            (Some(expected),Some(head)) if expected.event_id.as_uuid()==head.id && expected.payload_hash==head.payload_hash=>{},
            (None,None)=>{},
            (None,Some(_)) if !head_available=>{},
            _=>return Err(changed()),
        }
        let mut statements=Vec::new();
        for entry in body.statements {
            let evidence=TaskEvidenceRef::from(entry.evidence);
            let (text,evidence_class)=resolve_evidence(&state,&mut tx,tenant,session,&evidence).await?;
            statements.push(TaskStateStatement {kind:entry.kind,declared_outcome:entry.declared_outcome,evidence,text,evidence_class});
        }
        let context_snapshot=body.context_snapshot.map(|s|TaskContextSnapshot {context_run_id:s.context_run_id,block_hash:s.block_hash});
        if let Some(snapshot)=&context_snapshot
            && !crate::context_api::task_snapshot_available(&state,&mut tx,tenant,session,snapshot).await? {return Err(unavailable());}
        let checkout=if let Some(source)=body.checkout {Some(resolve_checkout(&mut tx,tenant,session,&TaskCheckoutRef {event_id:source.event_id,payload_hash:source.payload_hash}).await?)} else {None};
        let parsed=TaskState {schema_version:1,method:"exact_evidence_task_state_v1".to_owned(),request_hash:request_hash.clone(),repository_id:loaded.repository_id,statements,context_snapshot,checkout};
        let payload=serde_json::to_value(&parsed).map_err(|_|invalid())?;
        let scan=synveda_ingest::scan_task_state(&parsed);
        // A new scan boundary may join previously separate safe excerpts. Never
        // persist changed text with an exact-span proof; refuse it content-free.
        // Its validated UUIDs and digests are not prose: scanning them refused
        // valid states whose UUIDv7 groups read as a phone number.
        if !scan.findings.is_empty() {return Err(invalid());}
        let event=sessions::NewSessionEvent {event_type:SessionEventType::TaskState,event_schema_version:1,client_event_id:client,
            occurred_at:Utc::now(),source_payload_hash:sessions::payload_hash(&payload),payload,redactions:None,quarantine:false};
        let appended=sessions::append_events(&mut tx,tenant,session,&[event]).await?.pop().ok_or_else(unavailable)?;
        if appended.event.event_type!=SessionEventType::TaskState {return Err(changed());}
        audit::record(&mut tx,tenant,AuditAction::SessionEventsAppended,format!("session:{session}"),Outcome::Success,
            json!({"session_id":session,"event_id":appended.event.id,"payload_hash":appended.event.payload_hash,
             "request_hash":request_hash,"by_type":{"session.task_state":1},"authz":audit::decision_context(Action::SessionWrite,&allowed)})).await?;
        request::commit(tx).await?;
        Ok(Json(TaskStateView {outcome:appended.outcome.as_str().to_owned(),event:appended.event.into()}))
    }.await;
    super::respond(&state, "session.task_state", result).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_utf8_spans_never_cut_or_invent_source_bytes() {
        assert_eq!(
            copy_span("aλz", &TaskTextSpan { start: 1, end: 3 }).unwrap(),
            "λ"
        );
        assert!(copy_span("aλz", &TaskTextSpan { start: 2, end: 3 }).is_err());
        assert!(copy_span("abc", &TaskTextSpan { start: 0, end: 4 }).is_err());
        assert!(copy_span(" ", &TaskTextSpan { start: 0, end: 1 }).is_err());
    }
}

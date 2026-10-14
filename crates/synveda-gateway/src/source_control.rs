//! Governed Session-source controls; removal is distinct from item Forget.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_policy::Action;
use synveda_store::{rls, source_control as store, tenants};
use synveda_types::knowledge::{KnowledgeCommand, SourceControlMode};
use synveda_types::operation::OperationState;
use synveda_types::{DurableOperationId, Error, ProposalId, Result, SessionEventId, SessionId};
use utoipa::ToSchema;

use crate::{app::AppState, audit, knowledge_api, request};

/// Exact source precondition, reviewed through VedaFlow.
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceControlBody {
    /// Public redacted payload hash previously inspected.
    pub expected_payload_hash: String,
    /// Human reason, 1–1000 characters, never copied to ordinary audit.
    pub reason: String,
}

/// Logical removal and physical cleanup are independently observable.
#[derive(Serialize, ToSchema)]
pub struct SourceControlView {
    /// Original event address.
    #[schema(value_type=String,format="uuid")]
    pub event_id: SessionEventId,
    /// Committed logical removal time.
    pub removed_at: DateTime<Utc>,
    /// Applied VedaFlow source-control effect.
    #[schema(value_type=String,format="uuid")]
    pub change_id: ProposalId,
    /// Durable source-cleanup operation.
    #[schema(value_type=String,format="uuid")]
    pub operation_id: DurableOperationId,
    /// `withdraw` retains inaccessible plaintext; `forget` requests erasure.
    #[schema(schema_with = source_mode_schema)]
    pub mode: String,
    /// Closed durable operation lifecycle.
    #[schema(schema_with = cleanup_state_schema)]
    pub cleanup_state: String,
    /// Fixed cleanup stage; not a byte/row completion estimate.
    #[schema(schema_with = cleanup_stage_schema)]
    pub cleanup_stage: String,
    /// Stable content-free hold/failure reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[utoipa::path(post,path="/v1/sessions/{session_id}/events/{event_id}/withdraw",operation_id="withdraw_session_source",tag="sessions",
 request_body=SourceControlBody,params(("session_id"=String,Path),("event_id"=String,Path),("Idempotency-Key"=String,Header)),
 responses((status=201,description="Governed applied or pending change",body=knowledge_api::KnowledgeMutationView),
 (status=200,description="Exact change replay",body=knowledge_api::KnowledgeMutationView),
 (status=400,description="Invalid precondition or reason",body=crate::workspaces::ApiErrorBody),
 (status=403,description="Current Cedar denied source control",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable Session or source",body=crate::workspaces::ApiErrorBody),
 (status=409,description="Source changed or key conflict",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn withdraw(
    State(state): State<AppState>,
    Path((session, event)): Path<(SessionId, SessionEventId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<SourceControlBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    control(
        &state,
        session,
        event,
        &headers,
        payload,
        SourceControlMode::Withdraw,
    )
    .await
}

#[utoipa::path(post,path="/v1/sessions/{session_id}/events/{event_id}/forget",operation_id="forget_session_source",tag="sessions",
 request_body=SourceControlBody,params(("session_id"=String,Path),("event_id"=String,Path),("Idempotency-Key"=String,Header)),
 responses((status=201,description="Governed applied or pending change",body=knowledge_api::KnowledgeMutationView),
 (status=200,description="Exact change replay",body=knowledge_api::KnowledgeMutationView),
 (status=400,description="Invalid precondition or reason",body=crate::workspaces::ApiErrorBody),
 (status=403,description="Current Cedar denied source control",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable Session or source",body=crate::workspaces::ApiErrorBody),
 (status=409,description="Source changed or key conflict",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn forget(
    State(state): State<AppState>,
    Path((session, event)): Path<(SessionId, SessionEventId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<SourceControlBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    control(
        &state,
        session,
        event,
        &headers,
        payload,
        SourceControlMode::Forget,
    )
    .await
}

async fn control(
    state: &AppState,
    session_id: SessionId,
    event_id: SessionEventId,
    headers: &HeaderMap,
    payload: std::result::Result<Json<SourceControlBody>, axum::extract::rejection::JsonRejection>,
    mode: SourceControlMode,
) -> Response {
    let result = async {
        let body = request::body(payload)?;
        let tenant = request::tenant_id()?;
        let scope_id = {
            let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
            let loaded =
                crate::sessions::load(state, &mut tx, tenant, session_id, Action::SessionWrite)
                    .await;
            let (session, _, _) = match loaded {
                Ok(loaded) => loaded,
                Err(error) => {
                    drop(tx);
                    audit::record_rejection(state, "source_control", &error).await;
                    return Err(hidden_source(error));
                }
            };
            request::commit(tx).await?;
            session.scope_id
        };
        knowledge_api::execute_command(
            state,
            headers,
            "session.source.control",
            json!({"session_id":session_id,"event_id":event_id,"mode":mode,"body":body}),
            || {
                Ok(KnowledgeCommand::ControlSource {
                    session_id,
                    event_id,
                    scope_id,
                    expected_payload_hash: body.expected_payload_hash,
                    mode,
                    reason: body.reason,
                })
            },
        )
        .await
    }
    .await;
    knowledge_api::respond(state, "source_control", result).await
}

#[utoipa::path(get,path="/v1/sessions/{session_id}/events/{event_id}/source-control",operation_id="get_session_source_control",tag="sessions",
 params(("session_id"=String,Path),("event_id"=String,Path)),responses((status=200,description="Applied logical removal and cleanup state",body=SourceControlView),
 (status=403,description="Current Cedar denied the Session read",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable Session or applied source control",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path((session, event)): Path<(SessionId, SessionEventId)>,
) -> Response {
    let result=async {
        let tenant=request::tenant_id()?;
        let mut tx=rls::begin_tenant_tx(&state.pool,tenant).await?;
        let loaded=crate::sessions::load(&state,&mut tx,tenant,session,Action::SessionRead).await;
        let (_,authorized,resource)=match loaded {
            Ok(loaded)=>loaded,
            Err(error)=>{drop(tx);audit::record_rejection(&state,"source_control_get",&error).await;return Err(hidden_source(error));}
        };
        let status=store::status(&mut tx,tenant,session,event).await?.ok_or_else(||Error::NotFound {entity:"Session source".to_owned()})?;
        let state:OperationState=status.state.parse()?;
        let stage=stage(status.phase)?;
        audit::record(&mut tx,tenant,AuditAction::AuthzDecision,resource.to_string(),Outcome::Success,
            json!({"source_event_id":event,"authz":audit::decision_context(Action::SessionRead,&authorized)})).await?;
        request::commit(tx).await?;
        Ok(Json(SourceControlView {event_id:event,removed_at:status.removed_at,change_id:ProposalId::from_uuid(status.change_id),
            operation_id:DurableOperationId::from_uuid(status.operation_id),mode:if status.forgetting {"forget"}else{"withdraw"}.to_owned(),
            cleanup_state:state.as_str().to_owned(),cleanup_stage:stage.to_owned(),error_code:status.last_error_code}))
    }.await;
    knowledge_api::respond(&state, "source_control_get", result).await
}

fn hidden_source(error: Error) -> Error {
    match error {
        Error::PolicyDenied { .. } | Error::NotFound { .. } => Error::NotFound {
            entity: "Session source".to_owned(),
        },
        other => other,
    }
}

pub(crate) fn stage(phase: i16) -> Result<&'static str> {
    match phase {
        0 => Ok("retention_check"),
        1 => Ok("delivery_copies"),
        2 => Ok("capture_copies"),
        3 => Ok("command_copies"),
        4 => Ok("knowledge_copies"),
        5 => Ok("embedding_copies"),
        6 => Ok("relation_metadata"),
        7 => Ok("source_descriptors"),
        8 => Ok("session_copies"),
        _ => Err(Error::Internal {
            message: "unknown source cleanup stage".to_owned(),
        }),
    }
}

/// One bounded worker page; a caller commits its cursor and audit together.
/// Public for real ordinary-role worker acceptance, not exposed as an API.
pub async fn cleanup_page(
    pool: &sqlx::PgPool,
    tenant: synveda_types::TenantId,
    owner: &str,
) -> Result<bool> {
    let mut admission = rls::begin_tenant_tx(pool, tenant).await?;
    let claim = store::claim(&mut admission, tenant, owner).await?;
    let Some(claim) = claim else {
        request::commit(admission).await?;
        return Ok(false);
    };
    audit::record_as(
        &mut admission,
        tenant,
        Actor::system("source-cleanup"),
        AuditAction::SourceCleanupPage,
        format!("operation {}", claim.operation_id),
        Outcome::Success,
        json!({"operation_id":claim.operation_id,"attempt":claim.attempts,"stage":if claim.exhausted {"retry_exhausted"}else{"claimed"}}),
    )
    .await?;
    request::commit(admission).await?;
    if claim.exhausted {
        return Ok(true);
    }
    let mut work = rls::begin_tenant_tx(pool, tenant).await?;
    let page = match store::chunk(&mut work, tenant, &claim, owner).await {
        Ok(page) => page,
        Err(error) => {
            drop(work);
            let mut failed = rls::begin_tenant_tx(pool, tenant).await?;
            if store::fail(&mut failed, tenant, &claim, owner).await? {
                audit::record_as(&mut failed,tenant,Actor::system("source-cleanup"),AuditAction::SourceCleanupPage,
            format!("operation {}",claim.operation_id),Outcome::Failure,json!({"operation_id":claim.operation_id,"attempt":claim.attempts,"stage":"failed","error_code":"cleanup_failed"})).await?;
            }
            request::commit(failed).await?;
            return Err(error);
        }
    };
    audit::record_as(&mut work,tenant,Actor::system("source-cleanup"),AuditAction::SourceCleanupPage,
      format!("operation {}",claim.operation_id),Outcome::Success,json!({"operation_id":claim.operation_id,"attempt":claim.attempts,"progress":page.progress,"terminal":page.done})).await?;
    request::commit(work).await?;
    metrics::counter!("synveda_source_cleanup_pages_total","outcome"=>if page.done {"terminal"}else{"progress"}).increment(1);
    Ok(true)
}

async fn run_inner(pool: sqlx::PgPool, mut stop: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    let owner = format!("source-cleanup:{}", uuid::Uuid::now_v7());
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        for tenant in tenants::active(&pool).await? {
            for _ in 0..16 {
                if *stop.borrow() {
                    return Ok(());
                }
                match cleanup_page(&pool, tenant.id, &owner).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => {
                        tracing::warn!("source cleanup page deferred after failure");
                        break;
                    }
                }
            }
        }
        tokio::select! {_ =tokio::time::sleep(std::time::Duration::from_secs(5))=>{},_=stop.changed()=>{}}
    }
}

pub(crate) async fn run(pool: sqlx::PgPool, stop: tokio::sync::watch::Receiver<bool>) {
    if run_inner(pool, stop).await.is_err() {
        tracing::error!("source cleanup worker stopped after a failed page");
    }
}

pub(crate) fn source_mode_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(["withdraw", "forget"].into_iter())
}
pub(crate) fn cleanup_state_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(OperationState::ALL.iter().map(|state| state.as_str()))
}
fn cleanup_stage_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(
        [
            "retention_check",
            "delivery_copies",
            "capture_copies",
            "command_copies",
            "knowledge_copies",
            "embedding_copies",
            "relation_metadata",
            "source_descriptors",
            "session_copies",
        ]
        .into_iter(),
    )
}

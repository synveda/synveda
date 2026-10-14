//! Governed imported evidence uses the existing Knowledge/VedaFlow boundary.

use crate::{app::AppState, audit, knowledge_api, request};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::Action;
use synveda_store::{artifact_control as store, rls};
use synveda_types::knowledge::{KnowledgeCommand, SourceControlMode};
use synveda_types::{
    DurableOperationId, Error, ImportArtifactId, ImportJobId, ProjectId, ProposalId, Result,
    ScopeId, TenantId,
};
use utoipa::ToSchema;

/// Exact source bytes and reviewed reason.
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactControlBody {
    /// Original admitted content hash previously inspected.
    pub expected_content_hash: String,
    /// Bounded rationale retained in the governed intent.
    pub reason: String,
}

/// Logical removal and physical cleanup are independently observable.
#[derive(Serialize, ToSchema)]
pub struct ArtifactControlView {
    /// Exact inspected imported artifact.
    #[schema(value_type=String,format="uuid")]
    pub artifact_id: ImportArtifactId,
    /// Logical removal commit time.
    pub removed_at: DateTime<Utc>,
    /// Applied governed source control.
    #[schema(value_type=String,format="uuid")]
    pub change_id: ProposalId,
    /// Durable checked cleanup.
    #[schema(value_type=String,format="uuid")]
    pub operation_id: DurableOperationId,
    /// Withdrawal retains inaccessible content; Forget requests erasure.
    #[schema(schema_with=crate::source_control::source_mode_schema)]
    pub mode: String,
    /// Closed durable state.
    #[schema(schema_with=crate::source_control::cleanup_state_schema)]
    pub cleanup_state: String,
    /// Fixed stage, never a row or byte estimate.
    #[schema(schema_with=artifact_stage_schema)]
    pub cleanup_stage: String,
    /// Content-free hold or failure reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

pub(crate) async fn authorize(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    source: &store::ArtifactAddress,
    action: Action,
) -> Result<()> {
    let (project, allowed, resource) = crate::okf::authorize_project(
        state,
        tx,
        tenant,
        ProjectId::from_uuid(source.project_id),
        action,
    )
    .await?;
    if project.scope_id.as_uuid() != source.scope_id {
        return Err(Error::NotFound {
            entity: "Import source".to_owned(),
        });
    }
    audit::record(tx,tenant,AuditAction::AuthzDecision,resource.to_string(),Outcome::Allow,
       json!({"artifact_id":source.artifact_id,"content_hash":source.content_hash,"authz":audit::decision_context(action,&allowed)})).await?;
    Ok(())
}

#[utoipa::path(post,path="/v1/okf/imports/{id}/artifacts/{artifact_id}/withdraw",operation_id="withdraw_import_source",tag="okf",
 request_body=ArtifactControlBody,params(("id"=String,Path),("artifact_id"=String,Path),("Idempotency-Key"=String,Header)),
 responses((status=201,description="Governed applied or pending change",body=knowledge_api::KnowledgeMutationView),
 (status=200,description="Exact intent replay",body=knowledge_api::KnowledgeMutationView),(status=400,description="Invalid source control",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable source",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed source or key conflict",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn withdraw(
    State(state): State<AppState>,
    Path((job, artifact)): Path<(ImportJobId, ImportArtifactId)>,
    headers: HeaderMap,
    payload: std::result::Result<
        Json<ArtifactControlBody>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    control(
        &state,
        job,
        artifact,
        &headers,
        payload,
        SourceControlMode::Withdraw,
    )
    .await
}

#[utoipa::path(post,path="/v1/okf/imports/{id}/artifacts/{artifact_id}/forget",operation_id="forget_import_source",tag="okf",
 request_body=ArtifactControlBody,params(("id"=String,Path),("artifact_id"=String,Path),("Idempotency-Key"=String,Header)),
 responses((status=201,description="Governed applied or pending change",body=knowledge_api::KnowledgeMutationView),
 (status=200,description="Exact intent replay",body=knowledge_api::KnowledgeMutationView),(status=400,description="Invalid source control",body=crate::workspaces::ApiErrorBody),
 (status=404,description="Unavailable source",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed source or key conflict",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn forget(
    State(state): State<AppState>,
    Path((job, artifact)): Path<(ImportJobId, ImportArtifactId)>,
    headers: HeaderMap,
    payload: std::result::Result<
        Json<ArtifactControlBody>,
        axum::extract::rejection::JsonRejection,
    >,
) -> Response {
    control(
        &state,
        job,
        artifact,
        &headers,
        payload,
        SourceControlMode::Forget,
    )
    .await
}

async fn control(
    state: &AppState,
    job: ImportJobId,
    artifact: ImportArtifactId,
    headers: &HeaderMap,
    payload: std::result::Result<
        Json<ArtifactControlBody>,
        axum::extract::rejection::JsonRejection,
    >,
    mode: SourceControlMode,
) -> Response {
    let result = async {
        let body = request::body(payload)?;
        let tenant = request::tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let source = store::address(&mut tx, tenant, artifact)
            .await?
            .filter(|source| source.job_id == job.as_uuid())
            .ok_or_else(missing)?;
        if let Err(error) = authorize(state, &mut tx, tenant, &source, Action::KnowledgeRead).await
        {
            drop(tx);
            audit::record_rejection(state, "artifact_source_control", &error).await;
            return Err(hidden(error));
        }
        request::commit(tx).await?;
        knowledge_api::execute_command(
            state,
            headers,
            "import.source.control",
            json!({"job_id":job,"artifact_id":artifact,"mode":mode,"body":body}),
            || {
                Ok(KnowledgeCommand::ControlArtifact {
                    artifact_id: artifact,
                    scope_id: ScopeId::from_uuid(source.scope_id),
                    expected_content_hash: body.expected_content_hash,
                    mode,
                    reason: body.reason,
                })
            },
        )
        .await
    }
    .await;
    knowledge_api::respond(state, "artifact_source_control", result).await
}

#[utoipa::path(get,path="/v1/okf/imports/{id}/artifacts/{artifact_id}/source-control",operation_id="get_import_source_control",tag="okf",
 params(("id"=String,Path),("artifact_id"=String,Path)),responses((status=200,description="Applied removal and checked cleanup",body=ArtifactControlView),
 (status=404,description="Unavailable source control",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path((job, artifact)): Path<(ImportJobId, ImportArtifactId)>,
) -> Response {
    let result = async {
        let tenant = request::tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let source = store::address(&mut tx, tenant, artifact)
            .await?
            .filter(|source| source.job_id == job.as_uuid())
            .ok_or_else(missing)?;
        if let Err(error) = authorize(&state, &mut tx, tenant, &source, Action::KnowledgeRead).await
        {
            drop(tx);
            audit::record_rejection(&state, "artifact_source_control_get", &error).await;
            return Err(hidden(error));
        }
        let status = store::status(&mut tx, tenant, artifact)
            .await?
            .ok_or_else(missing)?;
        let view = view(artifact, status)?;
        request::commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    knowledge_api::respond(&state, "artifact_source_control_get", result).await
}

pub(crate) fn view(
    artifact_id: ImportArtifactId,
    status: synveda_store::source_control::SourceStatus,
) -> Result<ArtifactControlView> {
    let state: synveda_types::operation::OperationState = status.state.parse()?;
    Ok(ArtifactControlView {
        artifact_id,
        removed_at: status.removed_at,
        change_id: ProposalId::from_uuid(status.change_id),
        operation_id: DurableOperationId::from_uuid(status.operation_id),
        mode: if status.forgetting {
            "forget"
        } else {
            "withdraw"
        }
        .to_owned(),
        cleanup_state: state.as_str().to_owned(),
        cleanup_stage: stage(status.phase)?.to_owned(),
        error_code: status.last_error_code,
    })
}
fn stage(phase: i16) -> Result<&'static str> {
    match phase {
        0..=7 => crate::source_control::stage(phase),
        8 => Ok("import_mappings"),
        9 => Ok("artifact_copies"),
        _ => Err(Error::Internal {
            message: "unknown artifact cleanup stage".to_owned(),
        }),
    }
}
fn artifact_stage_schema() -> utoipa::openapi::schema::Object {
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
            "import_mappings",
            "artifact_copies",
        ]
        .into_iter(),
    )
}
fn missing() -> Error {
    Error::NotFound {
        entity: "Import source".to_owned(),
    }
}
fn hidden(error: Error) -> Error {
    match error {
        Error::PolicyDenied { .. } | Error::NotFound { .. } => missing(),
        other => other,
    }
}

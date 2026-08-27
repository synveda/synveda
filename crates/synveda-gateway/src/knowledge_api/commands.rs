use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use serde_json::json;
use synveda_store::rls;
use synveda_types::knowledge::{KnowledgeCommand, KnowledgeExpectedRevision};
use synveda_types::{Error, KnowledgeItemId, KnowledgeRevisionId, Result, ScopeId};

use super::{
    CreateKnowledgeBody, DeleteKnowledgeBody, EditKnowledgeBody, KnowledgeMutationView,
    LifecycleKnowledgeBody, MergeKnowledgeBody, SupersedeKnowledgeBody, VerifyKnowledgeBody,
    authorize_snapshot, content, execute_command, reject_secrets, respond, snapshot, sources,
};
use crate::app::AppState;
use crate::request::{body, tenant_id};
use crate::workspaces::ApiErrorBody;

async fn mutation_scope(state: &AppState, item_id: KnowledgeItemId) -> Result<ScopeId> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let snapshot = snapshot(&mut tx, tenant_id, item_id).await?;
    authorize_snapshot(state, &mut tx, tenant_id, &snapshot).await?;
    Ok(snapshot.item.scope_id)
}

/// `POST /v1/knowledge` — create one governed aggregate and first revision.
#[utoipa::path(
    post,
    path = "/v1/knowledge",
    operation_id = "create_knowledge",
    tag = "knowledge",
    request_body = CreateKnowledgeBody,
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied Knowledge write or proposal open", body = ApiErrorBody),
        (status = 409, description = "Idempotency key conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.create", skip_all)]
pub(crate) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<CreateKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"route": "POST /v1/knowledge", "body": &body});
        execute_command(&state, &headers, "knowledge.create", canonical, || {
            let now = Utc::now();
            Ok(KnowledgeCommand::Create {
                item_id: KnowledgeItemId::new(),
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                revision_id: KnowledgeRevisionId::new(),
                content: content(&body.content, now)?,
                sources: sources(&body.sources, body.scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.create", result).await
}

/// `PATCH /v1/knowledge/{id}` — append a governed immutable revision.
#[utoipa::path(
    patch,
    path = "/v1/knowledge/{id}",
    operation_id = "edit_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = EditKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.edit", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn edit(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<EditKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let scope_id = mutation_scope(&state, id).await?;
        let canonical = json!({
            "route": "PATCH /v1/knowledge/{id}",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.edit", canonical, || {
            Ok(KnowledgeCommand::Edit {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                revision_id: KnowledgeRevisionId::new(),
                content: content(&body.content, Utc::now())?,
                sources: sources(&body.sources, scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.edit", result).await
}

/// `POST /v1/knowledge/{id}/verify` — append verification evidence.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/verify",
    operation_id = "verify_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = VerifyKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.verify", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn verify(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<VerifyKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        reject_secrets(&body.verification_metadata)?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/verify",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.verify", canonical, || {
            Ok(KnowledgeCommand::Verify {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                revision_id: KnowledgeRevisionId::new(),
                verification_metadata: body.verification_metadata.clone(),
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.verify", result).await
}

/// `POST /v1/knowledge/{id}/supersede` — explicitly replace an item.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/supersede",
    operation_id = "supersede_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Item being replaced"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = SupersedeKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.supersede", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn supersede(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<SupersedeKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({
            "route": "POST /v1/knowledge/{id}/supersede",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, "knowledge.supersede", canonical, || {
            Ok(KnowledgeCommand::Supersede {
                item_id: id,
                expected_revision_id: body.expected_revision_id,
                replacement_item_id: KnowledgeItemId::new(),
                replacement_revision_id: KnowledgeRevisionId::new(),
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                content: content(&body.content, Utc::now())?,
                sources: sources(&body.sources, body.scope_id)?,
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.supersede", result).await
}

/// `POST /v1/knowledge/merge` — combine current items and all provenance.
#[utoipa::path(
    post,
    path = "/v1/knowledge/merge",
    operation_id = "merge_knowledge",
    tag = "knowledge",
    params(("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")),
    request_body = MergeKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied an input or output", body = ApiErrorBody),
        (status = 404, description = "An input is absent in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.merge", skip_all)]
pub(crate) async fn merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: std::result::Result<Json<MergeKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({"route": "POST /v1/knowledge/merge", "body": &body});
        execute_command(&state, &headers, "knowledge.merge", canonical, || {
            Ok(KnowledgeCommand::Merge {
                inputs: body
                    .inputs
                    .iter()
                    .map(|input| KnowledgeExpectedRevision {
                        item_id: input.item_id,
                        revision_id: input.revision_id,
                    })
                    .collect(),
                result_item_id: KnowledgeItemId::new(),
                result_revision_id: KnowledgeRevisionId::new(),
                scope_id: body.scope_id,
                project_id: body.project_id,
                owner_principal_id: body.owner_principal_id.clone(),
                knowledge_type: body.knowledge_type.parse()?,
                origin: body.origin.parse()?,
                content: content(&body.content, Utc::now())?,
                sources: Vec::new(),
            })
        })
        .await
    }
    .await;
    respond(&state, "knowledge.merge", result).await
}

enum LifecycleCommand {
    Archive,
    Restore,
}

async fn lifecycle_command(
    state: &AppState,
    headers: &HeaderMap,
    item_id: KnowledgeItemId,
    body: &LifecycleKnowledgeBody,
    operation: &'static str,
    route: &'static str,
    command: LifecycleCommand,
) -> Result<(StatusCode, Json<KnowledgeMutationView>)> {
    let canonical = json!({
        "route": route,
        "knowledge_item_id": item_id,
        "body": body,
    });
    execute_command(state, headers, operation, canonical, || {
        Ok(match command {
            LifecycleCommand::Archive => KnowledgeCommand::Archive {
                item_id,
                expected_revision_id: body.expected_revision_id,
                reason: body.reason.clone(),
            },
            LifecycleCommand::Restore => KnowledgeCommand::Restore {
                item_id,
                expected_revision_id: body.expected_revision_id,
                reason: body.reason.clone(),
            },
        })
    })
    .await
}

/// `POST /v1/knowledge/{id}/archive`.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/archive",
    operation_id = "archive_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = LifecycleKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.archive", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn archive(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<LifecycleKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        lifecycle_command(
            &state,
            &headers,
            id,
            &body,
            "knowledge.archive",
            "POST /v1/knowledge/{id}/archive",
            LifecycleCommand::Archive,
        )
        .await
    }
    .await;
    respond(&state, "knowledge.archive", result).await
}

/// `POST /v1/knowledge/{id}/restore`.
#[utoipa::path(
    post,
    path = "/v1/knowledge/{id}/restore",
    operation_id = "restore_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = LifecycleKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid body or missing idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied the mutation", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.restore", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn restore(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<LifecycleKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        lifecycle_command(
            &state,
            &headers,
            id,
            &body,
            "knowledge.restore",
            "POST /v1/knowledge/{id}/restore",
            LifecycleCommand::Restore,
        )
        .await
    }
    .await;
    respond(&state, "knowledge.restore", result).await
}

/// `DELETE /v1/knowledge/{id}` — explicit archive or governed forget.
#[utoipa::path(
    delete,
    path = "/v1/knowledge/{id}",
    operation_id = "delete_knowledge",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Stable Knowledge item id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = DeleteKnowledgeBody,
    responses(
        (status = 201, description = "VedaFlow change created", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Missing/invalid mode, body or idempotency key", body = ApiErrorBody),
        (status = 403, description = "The PDP denied archive or forget", body = ApiErrorBody),
        (status = 404, description = "No such item in this tenant", body = ApiErrorBody),
        (status = 409, description = "Idempotency or revision conflict", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.delete", skip_all, fields(knowledge.item.id = %id))]
pub(crate) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<KnowledgeItemId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<DeleteKnowledgeBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let operation = match body.mode.as_str() {
            "archive" => "knowledge.delete.archive",
            "forget" => "knowledge.delete.forget",
            other => {
                return Err(Error::Invalid {
                    message: format!(
                        "DELETE Knowledge mode must be `archive` or `forget`, got {other:?}"
                    ),
                });
            }
        };
        let canonical = json!({
            "route": "DELETE /v1/knowledge/{id}",
            "knowledge_item_id": id,
            "body": &body,
        });
        execute_command(&state, &headers, operation, canonical, || {
            if body.mode == "archive" {
                Ok(KnowledgeCommand::Archive {
                    item_id: id,
                    expected_revision_id: body.expected_revision_id,
                    reason: body.reason.clone(),
                })
            } else {
                Ok(KnowledgeCommand::Forget {
                    item_id: id,
                    expected_revision_id: body.expected_revision_id,
                    reason: body.reason.clone(),
                })
            }
        })
        .await
    }
    .await;
    respond(&state, "knowledge.delete", result).await
}

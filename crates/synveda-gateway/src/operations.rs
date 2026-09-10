//! Customer-safe durable-operation API (CPR-45, ADR-0102).
//!
//! The public identity is always the Synveda operation id. Views intentionally
//! omit tenant, requester, hashes, leases, raw result JSON, outbox state and
//! queue-provider identifiers.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource, ResourceEntity};
use synveda_store::anchors::AnchorSelection;
use synveda_store::operations::{self as store, StoredSkillValidationOperation};
use synveda_store::{projects, rls, scopes};
use synveda_types::operation::{OperationKind, OperationState, SKILL_VALIDATION_OPERATION_VERSION};
use synveda_types::{
    DurableOperationId, Error, IdentityKind, ProjectId, Result, ScopeId, SkillId, SkillTestHarness,
    SkillTestRunId, SkillVersionId, TenantId,
};
use utoipa::{IntoParams, ToSchema};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::idempotency::{Claim, Dispatch};
use crate::request::{commit, tenant_id};
use crate::skills::{self, RunSkillTestBody};
use crate::workspaces::{ApiErrorBody, string_enum, subject};

const DEFAULT_LIMIT: i64 = 25;
const MAX_LIMIT: i64 = 100;
// Per-operation Skill visibility is authoritative and intentionally cannot be
// inferred from project visibility. Bound that work to one maximum page so a
// caller cannot turn a list request into an unbounded Cedar/SQL fan-out.
const MAX_LIST_SCAN: i64 = MAX_LIMIT + 1;

fn operation_state_schema() -> utoipa::openapi::schema::Object {
    string_enum(OperationState::ALL.iter().map(|state| state.as_str()))
}

fn operation_kind_schema() -> utoipa::openapi::schema::Object {
    string_enum(std::iter::once(OperationKind::SkillValidation.as_str()))
}

/// Customer-safe view of one Skill-validation operation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OperationView {
    /// Stable Synveda operation identifier.
    #[schema(value_type = String, format = "uuid")]
    pub id: DurableOperationId,
    /// Closed operation family; currently `skill_validation` only.
    #[schema(schema_with = operation_kind_schema)]
    pub kind: String,
    /// Closed operation contract version.
    pub operation_version: u16,
    /// Current durable lifecycle state.
    #[schema(schema_with = operation_state_schema)]
    pub state: String,
    /// Stable Skill aggregate.
    #[schema(value_type = String, format = "uuid")]
    pub skill_id: SkillId,
    /// Exact immutable Skill version.
    #[schema(value_type = String, format = "uuid")]
    pub skill_version_id: SkillVersionId,
    /// Bounded progress percentage.
    pub progress_percent: u8,
    /// Number of fenced execution claims.
    pub attempts: i32,
    /// Earliest retry time for a retryable failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// When cancellation was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// Stable content-free failure code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Immutable controlled test result after success.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub test_run_id: Option<SkillTestRunId>,
    /// Request creation time.
    pub created_at: DateTime<Utc>,
    /// Last durable transition time.
    pub updated_at: DateTime<Utc>,
    /// First execution start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Terminal completion time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

impl TryFrom<StoredSkillValidationOperation> for OperationView {
    type Error = Error;

    fn try_from(value: StoredSkillValidationOperation) -> Result<Self> {
        let operation = value.operation;
        let skill_version_id = operation.skill_version_id.ok_or_else(|| Error::Internal {
            message: "a Skill-validation operation has no Skill version".to_owned(),
        })?;
        Ok(Self {
            id: operation.id,
            kind: operation.kind.as_str().to_owned(),
            operation_version: operation.operation_version,
            state: operation.state.as_str().to_owned(),
            skill_id: value.skill_id,
            skill_version_id,
            progress_percent: operation.progress_percent,
            attempts: operation.attempts,
            next_attempt_at: operation.next_attempt_at,
            cancel_requested_at: operation.cancel_requested_at,
            error_code: operation.last_error_code,
            test_run_id: value.test_run_id,
            created_at: operation.created_at,
            updated_at: operation.updated_at,
            started_at: operation.started_at,
            completed_at: operation.completed_at,
        })
    }
}

/// Bounded operation page.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OperationListView {
    /// Policy-visible operations.
    pub operations: Vec<OperationView>,
    /// Cursor after the last returned operation, only when another visible
    /// operation was confirmed inside the bounded scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub next_cursor: Option<DurableOperationId>,
}

/// Exact-project operation list query.
#[derive(Debug, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct ListOperationsParams {
    /// Project selected by the caller.
    #[param(value_type = String, format = "uuid")]
    pub project_id: ProjectId,
    /// UUIDv7 cursor returned by the previous page.
    #[param(value_type = Option<String>, format = "uuid")]
    pub cursor: Option<DurableOperationId>,
    /// Page size, 1..=100.
    pub limit: Option<i64>,
}

/// Enqueue the built-in non-executing Skill validation.
#[utoipa::path(
    post,
    path = "/v1/skills/{id}/versions/{version_id}/validation-operations",
    operation_id = "create_skill_validation_operation",
    tag = "operations",
    request_body = RunSkillTestBody,
    params(
        ("id" = String, Path, format = "uuid"),
        ("version_id" = String, Path, format = "uuid"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry.")
    ),
    responses(
        (status = 202, description = "Operation accepted", body = OperationView),
        (status = 200, description = "Idempotent replay", body = OperationView),
        (status = 400, description = "Unsupported harness", body = ApiErrorBody),
        (status = 403, description = "Skill validation denied", body = ApiErrorBody),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "operations.skill_validation.create", skip_all)]
pub(crate) async fn create_skill_validation(
    State(state): State<AppState>,
    Path((skill_id, version_id)): Path<(SkillId, SkillVersionId)>,
    headers: HeaderMap,
    payload: std::result::Result<Json<RunSkillTestBody>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let result: Result<(StatusCode, Json<OperationView>)> = async {
        let body = crate::request::body(payload)?;
        let harness: SkillTestHarness = body.harness.parse()?;
        if harness != SkillTestHarness::ValidationSandbox {
            return Err(Error::Invalid {
                message: "durable Skill validation supports only validation_sandbox".to_owned(),
            });
        }
        let tenant = tenant_id()?;
        let canonical = json!({
            "skill_id": skill_id,
            "version_id": version_id,
            "operation_version": SKILL_VALIDATION_OPERATION_VERSION,
            "body": body,
        });
        let claim = Claim::from_headers(
            &headers,
            "skill.validation_operation",
            &subject()?,
            &canonical,
        )?;
        if let Dispatch::Replay(operation_id) =
            crate::idempotency::dispatch(&state.pool, tenant, &claim).await?
        {
            let view = replay(&state, tenant, DurableOperationId::from_uuid(operation_id)).await?;
            return Ok((StatusCode::OK, Json(view)));
        }
        match create(&state, tenant, skill_id, version_id, &claim).await {
            Ok(view) => Ok((StatusCode::ACCEPTED, Json(view))),
            Err(conflict @ Error::Conflict { .. }) => {
                let operation_id =
                    crate::idempotency::resolve_conflict(&state.pool, tenant, &claim, conflict)
                        .await?;
                let view =
                    replay(&state, tenant, DurableOperationId::from_uuid(operation_id)).await?;
                Ok((StatusCode::OK, Json(view)))
            }
            Err(error) => Err(error),
        }
    }
    .await;
    crate::response::finish(&state, "create_skill_validation_operation", result).await
}

async fn create(
    state: &AppState,
    tenant: TenantId,
    skill_id: SkillId,
    version_id: SkillVersionId,
    claim: &Claim,
) -> Result<OperationView> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let (skill, _, _) =
        skills::exact_visible(state, &mut tx, tenant, skill_id, Some(version_id)).await?;
    let scope = skills::scope_for(&mut tx, tenant, skill.governing_scope_id).await?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&scope),
        AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(state, &input, Action::SkillWrite, Resource::Scope(scope.id))?;
    let identity = input.identity.as_ref().ok_or_else(|| Error::Invalid {
        message: "requesting Skill validation requires a provisioned identity".to_owned(),
    })?;
    if identity.kind != IdentityKind::User {
        return Err(durable_service_identity_refusal(scope.id));
    }
    let digest = blake3::Hash::from_bytes(claim.digest).to_hex().to_string();
    let operation =
        store::create_skill_validation_operation(&mut tx, tenant, version_id, identity.id, &digest)
            .await?;
    claim
        .remember(&mut tx, tenant, operation.id.as_uuid())
        .await?;
    audit::record(
        &mut tx,
        tenant,
        AuditAction::OperationRequested,
        Resource::Scope(scope.id).to_string(),
        Outcome::Success,
        json!({
            "operation_id": operation.id,
            "kind": operation.kind.as_str(),
            "operation_version": operation.operation_version,
            "skill_id": skill.id,
            "skill_version_id": version_id,
            "authz": audit::decision_context(Action::SkillWrite, &authorized),
        }),
    )
    .await?;
    let view = StoredSkillValidationOperation {
        operation,
        skill_id: skill.id,
        scope_id: scope.id,
        test_run_id: None,
    }
    .try_into()?;
    commit(tx).await?;
    Ok(view)
}

async fn replay(
    state: &AppState,
    tenant: TenantId,
    operation_id: DurableOperationId,
) -> Result<OperationView> {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
    let stored = store::read_skill_validation_operation(&mut *tx, tenant, operation_id)
        .await?
        .ok_or_else(|| operation_not_found(operation_id))?;
    authorize_write(state, &mut tx, tenant, &stored).await?;
    commit(tx).await?;
    stored.try_into()
}

/// List policy-visible Skill-validation operations at one project.
#[utoipa::path(
    get,
    path = "/v1/operations",
    operation_id = "list_operations",
    tag = "operations",
    params(ListOperationsParams),
    responses(
        (status = 200, description = "Operations", body = OperationListView),
        (status = 400, description = "Invalid cursor or limit", body = ApiErrorBody),
        (status = 404, description = "Project absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "operations.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListOperationsParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let take = limit(params.limit)?;
        let take_usize = usize::try_from(take).map_err(|_| Error::Internal {
            message: "the validated operation page size is outside the platform range".to_owned(),
        })?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let project = projects::get(&mut *tx, tenant, params.project_id)
            .await?
            .ok_or_else(|| project_not_found(params.project_id))?;
        let scope = scopes::get(&mut *tx, tenant, project.scope_id)
            .await?
            .ok_or_else(|| project_not_found(project.id))?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            AnchorSelection::project(project.id),
            vec![ResourceEntity::Project {
                id: project.id,
                scope_id: project.scope_id,
                workspace_id: project.workspace_id,
            }],
        )
        .await?;
        let project_allowed = authz::decide(
            &state,
            &input,
            Action::ProjectRead,
            Resource::Project(project.id),
        )
        .map_err(|error| match error {
            Error::PolicyDenied { .. } => project_not_found(project.id),
            other => other,
        })?;
        let mut visible: Vec<OperationView> = Vec::new();
        let mut scan_cursor = params.cursor;
        let mut inspected = 0_i64;
        let mut confirmed_more = false;
        while inspected < MAX_LIST_SCAN && !confirmed_more {
            let batch_limit = (MAX_LIST_SCAN - inspected).min(MAX_LIMIT);
            let candidates = store::list_skill_validation_operations(
                &mut *tx,
                tenant,
                scope.id,
                scan_cursor,
                batch_limit,
            )
            .await?;
            let fetched = i64::try_from(candidates.len()).unwrap_or(i64::MAX);
            if candidates.is_empty() {
                break;
            }
            for operation in candidates {
                inspected += 1;
                scan_cursor = Some(operation.operation.id);
                match authorize_read(&state, &mut tx, tenant, &operation).await {
                    Ok(_) if visible.len() == take_usize => {
                        confirmed_more = true;
                        break;
                    }
                    Ok(_) => visible.push(operation.try_into()?),
                    Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                    Err(error) => return Err(error),
                }
            }
            if fetched < batch_limit {
                break;
            }
        }
        let next_cursor = confirmed_more
            .then(|| visible.last().map(|entry| entry.id))
            .flatten();
        audit::record(
            &mut tx,
            tenant,
            AuditAction::AuthzDecision,
            Resource::Project(project.id).to_string(),
            Outcome::Allow,
            json!({
                "op": "operations.list",
                "project_id": project.id,
                "returned": visible.len(),
                "authz": audit::decision_context(Action::ProjectRead, &project_allowed),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(OperationListView {
            operations: visible,
            next_cursor,
        }))
    }
    .await;
    crate::response::finish(&state, "list_operations", result).await
}

/// Get one policy-visible operation.
#[utoipa::path(
    get,
    path = "/v1/operations/{id}",
    operation_id = "get_operation",
    tag = "operations",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Operation", body = OperationView),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "operations.get", skip_all)]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path(operation_id): Path<DurableOperationId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let stored = store::read_skill_validation_operation(&mut *tx, tenant, operation_id)
            .await?
            .ok_or_else(|| operation_not_found(operation_id))?;
        let allowed = authorize_read(&state, &mut tx, tenant, &stored).await?;
        audit::record(
            &mut tx,
            tenant,
            AuditAction::AuthzDecision,
            Resource::Scope(stored.scope_id).to_string(),
            Outcome::Allow,
            json!({
                "op": "operations.get",
                "operation_id": operation_id,
                "authz": audit::decision_context(Action::SkillRead, &allowed),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(OperationView::try_from(stored)?))
    }
    .await;
    crate::response::finish(&state, "get_operation", result).await
}

/// Request idempotent cancellation of one operation.
#[utoipa::path(
    post,
    path = "/v1/operations/{id}/cancel",
    operation_id = "cancel_operation",
    tag = "operations",
    params(("id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "Cancellation state", body = OperationView),
        (status = 403, description = "Cancellation denied", body = ApiErrorBody),
        (status = 404, description = "Absent or denied", body = ApiErrorBody)
    ),
    security(("bearer" = []))
)]
#[tracing::instrument(name = "operations.cancel", skip_all)]
pub(crate) async fn cancel(
    State(state): State<AppState>,
    Path(operation_id): Path<DurableOperationId>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let before = store::read_skill_validation_operation(&mut *tx, tenant, operation_id)
            .await?
            .ok_or_else(|| operation_not_found(operation_id))?;
        let authorized = authorize_write(&state, &mut tx, tenant, &before).await?;
        let cancellation = store::request_cancellation(&mut tx, tenant, operation_id)
            .await?
            .ok_or_else(|| operation_not_found(operation_id))?;
        let after = cancellation.operation;
        if cancellation.transitioned {
            let scope_id = after.scope_id;
            audit::record(
                &mut tx,
                tenant,
                AuditAction::OperationCancellationRequested,
                Resource::Scope(scope_id).to_string(),
                Outcome::Success,
                json!({
                    "operation_id": operation_id,
                    "kind": after.operation.kind.as_str(),
                    "operation_version": after.operation.operation_version,
                    "state": after.operation.state.as_str(),
                    "authz": audit::decision_context(Action::SkillWrite, &authorized),
                }),
            )
            .await?;
            audit::record(
                &mut tx,
                tenant,
                AuditAction::OperationFinished,
                Resource::Scope(scope_id).to_string(),
                Outcome::Success,
                json!({
                    "operation_id": operation_id,
                    "kind": after.operation.kind.as_str(),
                    "operation_version": after.operation.operation_version,
                    "state": after.operation.state.as_str(),
                    "attempts": after.operation.attempts,
                    "error_code": null,
                }),
            )
            .await?;
        }
        commit(tx).await?;
        Ok(Json(OperationView::try_from(after)?))
    }
    .await;
    crate::response::finish(&state, "cancel_operation", result).await
}

async fn authorize_read(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    operation: &StoredSkillValidationOperation,
) -> Result<authz::Authorized> {
    let (skill, _, authorized) = skills::exact_visible(
        state,
        tx,
        tenant,
        operation.skill_id,
        operation.operation.skill_version_id,
    )
    .await?;
    ensure_scope_binding(operation.scope_id, skill.governing_scope_id)?;
    Ok(authorized)
}

async fn authorize_write(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    operation: &StoredSkillValidationOperation,
) -> Result<authz::Authorized> {
    let (skill, _, _) = skills::exact_visible(
        state,
        tx,
        tenant,
        operation.skill_id,
        operation.operation.skill_version_id,
    )
    .await?;
    ensure_scope_binding(operation.scope_id, skill.governing_scope_id)?;
    let scope = skills::scope_for(tx, tenant, skill.governing_scope_id).await?;
    let input = authz::gather(state, tx, Some(&scope), AnchorSelection::none(), Vec::new()).await?;
    let identity = input.identity.as_ref().ok_or_else(|| Error::Invalid {
        message: "operating on Skill validation requires a provisioned identity".to_owned(),
    })?;
    if identity.kind != IdentityKind::User {
        return Err(durable_service_identity_refusal(scope.id));
    }
    authz::decide(state, &input, Action::SkillWrite, Resource::Scope(scope.id))
}

fn ensure_scope_binding(stored_scope_id: ScopeId, actual_scope_id: ScopeId) -> Result<()> {
    if stored_scope_id != actual_scope_id {
        return Err(Error::Internal {
            message: "a Skill-validation operation is bound to the wrong governing scope"
                .to_owned(),
        });
    }
    Ok(())
}

fn durable_service_identity_refusal(scope_id: ScopeId) -> Error {
    Error::PolicyDenied {
        action: Action::SkillWrite.as_str().to_owned(),
        resource: Resource::Scope(scope_id).to_string(),
        reason: "durable execution cannot reconstruct service-token lifetime and confinement"
            .to_owned(),
    }
}

fn operation_not_found(id: DurableOperationId) -> Error {
    Error::NotFound {
        entity: format!("operation {id}"),
    }
}

fn project_not_found(id: ProjectId) -> Error {
    Error::NotFound {
        entity: format!("project {id}"),
    }
}

fn limit(value: Option<i64>) -> Result<i64> {
    let value = value.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_view_excludes_internal_fields_by_construction() {
        let fields = serde_json::to_value(OperationView {
            id: DurableOperationId::new(),
            kind: "skill_validation".to_owned(),
            operation_version: 1,
            state: "pending".to_owned(),
            skill_id: SkillId::new(),
            skill_version_id: SkillVersionId::new(),
            progress_percent: 0,
            attempts: 0,
            next_attempt_at: Some(Utc::now()),
            cancel_requested_at: None,
            error_code: None,
            test_run_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            completed_at: None,
        })
        .expect("serialize operation view");
        let object = fields.as_object().expect("operation view object");
        for forbidden in [
            "tenant_id",
            "requested_by",
            "input_hash",
            "lease_owner",
            "lease_expires_at",
            "result",
            "outbox",
            "provider",
            "task_id",
        ] {
            assert!(!object.contains_key(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn list_limit_is_bounded() {
        assert_eq!(limit(None).unwrap(), DEFAULT_LIMIT);
        assert_eq!(limit(Some(MAX_LIMIT)).unwrap(), MAX_LIMIT);
        assert!(limit(Some(0)).is_err());
        assert!(limit(Some(MAX_LIMIT + 1)).is_err());
    }
}

//! Policy-safe conflict review and freshness-policy projection (CPR-37,
//! ADR-0096).
//!
//! A database conflict set is only a candidate. This surface retains no set,
//! member, classification or count until every exact member is independently
//! readable. Capture challengers repeat both source and proposed-destination
//! decisions. Resolution enters the ordinary Knowledge command service.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_policy::{Action, Resource};
use synveda_store::anchors::AnchorSelection;
use synveda_store::capture as capture_store;
use synveda_store::configuration as configuration_store;
use synveda_store::knowledge as knowledge_store;
use synveda_store::knowledge_conflicts as conflict_store;
use synveda_store::{rls, scopes};
use synveda_types::knowledge::{
    ConflictMemberRole, ConflictResolutionKind, ConflictSet, ConflictSetStatus, FreshnessPolicy,
    KnowledgeCommand, KnowledgeType,
};
use synveda_types::{
    CaptureCandidateId, ConflictMemberId, ConflictSetId, Error, KnowledgeItemId, ProjectId, Result,
    ScopeId, TenantId,
};
use utoipa::ToSchema;

use crate::app::AppState;
use crate::authz;
use crate::knowledge_api::{
    KnowledgeMutationView, KnowledgeRevisionView, execute_command, listing_gate, read_event,
    respond,
};
use crate::request::{body, commit, tenant_id};
use crate::workspaces::{ApiErrorBody, string_enum};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn classification_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        synveda_types::knowledge::ConflictClassification::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

fn conflict_status_schema() -> utoipa::openapi::schema::Object {
    string_enum(ConflictSetStatus::ALL.iter().map(|value| value.as_str()))
}

fn resolution_schema() -> utoipa::openapi::schema::Object {
    string_enum(
        ConflictResolutionKind::ALL
            .iter()
            .map(|value| value.as_str()),
    )
}

/// One exact, independently authorised conflict member.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConflictMemberView {
    /// Stable member evidence id.
    #[schema(value_type = String, format = "uuid")]
    pub id: ConflictMemberId,
    /// `challenger` or `current`.
    pub role: String,
    /// Exact stable Knowledge item, absent for a capture challenger.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub knowledge_item_id: Option<KnowledgeItemId>,
    /// Exact immutable content compared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_revision: Option<KnowledgeRevisionView>,
    /// Reviewable candidate, disclosed only after source and destination PDP
    /// decisions.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub capture_candidate_id: Option<CaptureCandidateId>,
    /// Deterministic proposed classification.
    #[schema(schema_with = classification_schema)]
    pub classification: String,
    /// Integer similarity.
    pub similarity_permille: i32,
    /// Stable content-free reason code.
    pub reason_code: String,
}

/// One fully visible durable conflict set.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConflictSetView {
    /// Stable resolution address.
    #[schema(value_type = String, format = "uuid")]
    pub id: ConflictSetId,
    /// Governing scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Project association.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Dominant classification, disclosed only because every member is
    /// visible.
    #[schema(schema_with = classification_schema)]
    pub classification: String,
    /// Resolution lifecycle.
    #[schema(schema_with = conflict_status_schema)]
    pub status: String,
    /// Revision precondition for a resolution.
    pub revision: i64,
    /// Exact VedaFlow resolution when one has opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub resolution_change_id: Option<synveda_types::ProposalId>,
    /// Terminal/pending resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(schema_with = resolution_schema)]
    pub resolution: Option<String>,
    /// Every member; no denied member is represented or counted.
    pub members: Vec<ConflictMemberView>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    /// Last state transition.
    pub updated_at: DateTime<Utc>,
    /// Resolution time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Cursor-paginated fully visible conflict sets.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConflictSetListView {
    /// Fully visible rows.
    pub conflicts: Vec<ConflictSetView>,
    /// Opaque next candidate position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// At least one candidate set was wholly omitted by policy. No count or
    /// classification is disclosed.
    pub policy_exclusions: bool,
}

/// Public evaluated freshness policy for one Knowledge type.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FreshnessPolicyView {
    /// Scope at which resolution was requested.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Knowledge type.
    pub knowledge_type: String,
    /// Governed default interval; zero means no implicit date.
    pub default_days: u32,
    /// Stable type-specific verification signals.
    pub triggers: Vec<String>,
    /// Exact Configuration aggregate, absent for fail-safe configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub configuration_artifact_id: Option<synveda_types::ConfigurationArtifactId>,
    /// Exact binding selected nearest-first.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub configuration_binding_id: Option<synveda_types::ConfigurationBindingId>,
    /// Exact immutable Configuration version.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub configuration_version_id: Option<synveda_types::ConfigurationVersionId>,
    /// Canonical configuration hash, including fail-safe.
    pub configuration_hash: String,
}

impl From<FreshnessPolicy> for FreshnessPolicyView {
    fn from(policy: FreshnessPolicy) -> Self {
        Self {
            scope_id: policy.scope_id,
            knowledge_type: policy.knowledge_type.as_str().to_owned(),
            default_days: policy.default_days,
            triggers: policy
                .triggers
                .into_iter()
                .map(|trigger| trigger.as_str().to_owned())
                .collect(),
            configuration_artifact_id: policy.configuration_artifact_id,
            configuration_binding_id: policy.configuration_binding_id,
            configuration_version_id: policy.configuration_version_id,
            configuration_hash: policy.configuration_hash,
        }
    }
}

/// Every type-aware policy under one exact effective configuration.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FreshnessPolicyListView {
    /// Closed Knowledge vocabulary in declaration order.
    pub policies: Vec<FreshnessPolicyView>,
}

/// Conflict-list filters.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConflictListParams {
    /// Exact governing scope.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub scope_id: Option<ScopeId>,
    /// Exact project.
    #[serde(default)]
    #[param(value_type = Option<String>, format = "uuid")]
    pub project_id: Option<ProjectId>,
    /// Resolution state. Omission means open and pending-review sets.
    #[serde(default)]
    pub status: Option<String>,
    /// Opaque cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Rows to serve, 1–200.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One governed conflict resolution.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveConflictBody {
    /// Exact conflict-set revision inspected.
    pub expected_revision: i64,
    /// Closed resolution vocabulary.
    #[schema(schema_with = resolution_schema)]
    pub resolution: String,
    /// Exact future valid-time boundary for `transition` only.
    #[serde(default)]
    pub transition_at: Option<DateTime<Utc>>,
    /// Bounded human rationale retained in the VedaFlow command.
    pub reason: String,
}

/// Freshness-policy scope selector.
#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FreshnessPolicyParams {
    /// Exact governed scope.
    #[param(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
}

fn limit(raw: Option<i64>) -> Result<i64> {
    let value = raw.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&value) {
        return Err(Error::Invalid {
            message: format!("limit must be between 1 and {MAX_LIMIT}"),
        });
    }
    Ok(value)
}

fn cursor_digest(params: &ConflictListParams, status: Option<ConflictSetStatus>) -> String {
    blake3::hash(
        json!({
            "scope_id": params.scope_id,
            "project_id": params.project_id,
            "status": status.map(ConflictSetStatus::as_str),
        })
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn encode_cursor(cursor: conflict_store::ConflictCursor, digest: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "kc1|{digest}|{}|{}",
        cursor.updated_at.to_rfc3339(),
        cursor.id
    ))
}

fn decode_cursor(raw: &str, digest: &str) -> Result<conflict_store::ConflictCursor> {
    let invalid = || Error::Invalid {
        message: "cursor was not issued for these conflict filters".to_owned(),
    };
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    let decoded = String::from_utf8(decoded).map_err(|_| invalid())?;
    let parts: Vec<_> = decoded.split('|').collect();
    match parts.as_slice() {
        ["kc1", actual, updated_at, id] if *actual == digest => {
            Ok(conflict_store::ConflictCursor {
                updated_at: DateTime::parse_from_rfc3339(updated_at)
                    .map_err(|_| invalid())?
                    .with_timezone(&Utc),
                id: id.parse().map_err(|_| invalid())?,
            })
        }
        _ => Err(invalid()),
    }
}

async fn visible_set(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    set: ConflictSet,
) -> Result<Option<ConflictSetView>> {
    let members = conflict_store::members(&mut *tx, tenant, set.id).await?;
    let mut visible = Vec::with_capacity(members.len());
    for member in members {
        let (knowledge_revision, capture_candidate_id) = if let (Some(item_id), Some(revision_id)) =
            (member.knowledge_item_id, member.knowledge_revision_id)
        {
            let Some(current) = knowledge_store::current(&mut *tx, tenant, item_id).await? else {
                return Ok(None);
            };
            let Some(revision) =
                knowledge_store::revision(&mut *tx, tenant, item_id, revision_id).await?
            else {
                return Ok(None);
            };
            let exact = knowledge_store::KnowledgeSnapshot {
                item: current.item,
                revision,
                transaction_to: current.transaction_to,
            };
            match crate::knowledge_api::authorize_snapshot(state, tx, tenant, &exact).await {
                Ok(_) => (
                    Some(KnowledgeRevisionView::from_revision(
                        exact.revision,
                        Utc::now(),
                    )),
                    None,
                ),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => return Ok(None),
                Err(error) => return Err(error),
            }
        } else if let Some(candidate_id) = member.capture_candidate_id {
            let Some(candidate) =
                capture_store::get_candidate(&mut *tx, tenant, candidate_id).await?
            else {
                return Ok(None);
            };
            match crate::capture::authorize_context_candidate(state, tx, tenant, &candidate).await {
                Ok(_) => (None, Some(candidate_id)),
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => return Ok(None),
                Err(error) => return Err(error),
            }
        } else {
            return Err(Error::Internal {
                message: format!("conflict member {} has no evidence address", member.id),
            });
        };
        visible.push(ConflictMemberView {
            id: member.id,
            role: member.role.as_str().to_owned(),
            knowledge_item_id: member.knowledge_item_id,
            knowledge_revision,
            capture_candidate_id,
            classification: member.classification.as_str().to_owned(),
            similarity_permille: member.similarity_permille,
            reason_code: member.reason_code,
        });
    }
    if visible.is_empty()
        || !visible
            .iter()
            .any(|member| member.role == ConflictMemberRole::Challenger.as_str())
    {
        return Ok(None);
    }
    Ok(Some(ConflictSetView {
        id: set.id,
        scope_id: set.scope_id,
        project_id: set.project_id,
        classification: set.classification.as_str().to_owned(),
        status: set.status.as_str().to_owned(),
        revision: set.revision,
        resolution_change_id: set.resolution_change_id,
        resolution: set.resolution.map(|value| value.as_str().to_owned()),
        members: visible,
        created_at: set.created_at,
        updated_at: set.updated_at,
        resolved_at: set.resolved_at,
    }))
}

/// List fully policy-visible conflict sets.
#[utoipa::path(
    get,
    path = "/v1/knowledge-conflicts",
    operation_id = "list_knowledge_conflicts",
    tag = "knowledge",
    params(ConflictListParams),
    responses(
        (status = 200, description = "Fully visible durable conflict evidence", body = ConflictSetListView),
        (status = 400, description = "Invalid filter or cursor", body = ApiErrorBody),
        (status = 403, description = "Knowledge reading denied", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.conflicts.list", skip_all)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Query(params): Query<ConflictListParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let limit = limit(params.limit)?;
        let status = params.status.as_deref().map(str::parse).transpose()?;
        let digest = cursor_digest(&params, status);
        let cursor = params
            .cursor
            .as_deref()
            .map(|raw| decode_cursor(raw, &digest))
            .transpose()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let (gate, resource) = listing_gate(&state, &mut tx).await?;
        let scan_limit = (limit * 10).min(500);
        let mut candidates = conflict_store::list(
            &mut tx,
            tenant,
            params.scope_id,
            params.project_id,
            status,
            cursor,
            scan_limit + 1,
        )
        .await?;
        if status.is_none() {
            candidates.retain(|set| {
                matches!(
                    set.status,
                    ConflictSetStatus::Open | ConflictSetStatus::PendingReview
                )
            });
        }
        let more = candidates.len() as i64 > scan_limit;
        candidates.truncate(scan_limit as usize);
        let mut rows = Vec::new();
        let mut last = None;
        let mut policy_exclusions = false;
        for candidate in candidates {
            last = Some(conflict_store::ConflictCursor {
                updated_at: candidate.updated_at,
                id: candidate.id,
            });
            match visible_set(&state, &mut tx, tenant, candidate).await? {
                Some(view) => rows.push(view),
                None => policy_exclusions = true,
            }
            if rows.len() >= limit as usize {
                break;
            }
        }
        let next_cursor = (more || rows.len() >= limit as usize)
            .then(|| last.map(|cursor| encode_cursor(cursor, &digest)))
            .flatten();
        read_event(
            &mut tx,
            tenant,
            "knowledge.conflicts.list",
            &gate,
            resource,
            json!({
                "served": rows.len(),
                "policy_exclusions": policy_exclusions,
                "filter_hash": digest,
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(ConflictSetListView {
            conflicts: rows,
            next_cursor,
            policy_exclusions,
        }))
    }
    .await;
    respond(&state, "knowledge.conflicts.list", result).await
}

/// Read one fully visible conflict set.
#[utoipa::path(
    get,
    path = "/v1/knowledge-conflicts/{id}",
    operation_id = "get_knowledge_conflict",
    tag = "knowledge",
    params(("id" = String, Path, description = "Conflict set id")),
    responses(
        (status = 200, description = "Fully visible conflict evidence", body = ConflictSetView),
        (status = 403, description = "A member is not visible", body = ApiErrorBody),
        (status = 404, description = "No such conflict set", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.conflicts.get", skip_all, fields(knowledge.conflict.id = %id))]
pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<ConflictSetId>) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let set = conflict_store::get(&mut tx, tenant, id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("conflict set {id}"),
            })?;
        let view = visible_set(&state, &mut tx, tenant, set)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("conflict set {id}"),
            })?;
        let (gate, resource) = listing_gate(&state, &mut tx).await?;
        read_event(
            &mut tx,
            tenant,
            "knowledge.conflicts.get",
            &gate,
            resource,
            json!({"conflict_set_id": id}),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "knowledge.conflicts.get", result).await
}

/// Resolve one Knowledge-backed conflict through VedaFlow.
#[utoipa::path(
    post,
    path = "/v1/knowledge-conflicts/{id}/resolve",
    operation_id = "resolve_knowledge_conflict",
    tag = "knowledge",
    params(
        ("id" = String, Path, description = "Conflict set id"),
        ("Idempotency-Key" = String, Header, description = "Required; reuse verbatim on retry."),
    ),
    request_body = ResolveConflictBody,
    responses(
        (status = 201, description = "VedaFlow resolution opened", body = KnowledgeMutationView),
        (status = 200, description = "Idempotent replay", body = KnowledgeMutationView),
        (status = 400, description = "Invalid resolution", body = ApiErrorBody),
        (status = 403, description = "A member or proposal action was denied", body = ApiErrorBody),
        (status = 404, description = "No such set/member", body = ApiErrorBody),
        (status = 409, description = "Conflict revision moved", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.conflicts.resolve", skip_all, fields(knowledge.conflict.id = %id))]
pub(crate) async fn resolve(
    State(state): State<AppState>,
    Path(id): Path<ConflictSetId>,
    headers: HeaderMap,
    payload: std::result::Result<Json<ResolveConflictBody>, JsonRejection>,
) -> Response {
    let result = async {
        let body = body(payload)?;
        let canonical = json!({
            "route": "POST /v1/knowledge-conflicts/{id}/resolve",
            "conflict_set_id": id,
            "body": &body,
        });
        execute_command(
            &state,
            &headers,
            "knowledge.conflicts.resolve",
            canonical,
            || {
                Ok(KnowledgeCommand::ResolveConflict {
                    conflict_set_id: id,
                    expected_conflict_revision: body.expected_revision,
                    resolution: body.resolution.parse()?,
                    transition_at: body.transition_at,
                    reason: body.reason.clone(),
                })
            },
        )
        .await
    }
    .await;
    respond(&state, "knowledge.conflicts.resolve", result).await
}

/// Resolve type-aware policies from one exact governed Configuration.
#[utoipa::path(
    get,
    path = "/v1/knowledge-freshness-policies",
    operation_id = "list_knowledge_freshness_policies",
    tag = "knowledge",
    params(FreshnessPolicyParams),
    responses(
        (status = 200, description = "Exact effective freshness policies", body = FreshnessPolicyListView),
        (status = 403, description = "Knowledge reading denied at the scope", body = ApiErrorBody),
        (status = 404, description = "No such scope", body = ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "knowledge.freshness.list", skip_all)]
pub(crate) async fn freshness_policies(
    State(state): State<AppState>,
    Query(params): Query<FreshnessPolicyParams>,
) -> Response {
    let result = async {
        let tenant = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let scope = scopes::get(&mut *tx, tenant, params.scope_id)
            .await?
            .ok_or_else(|| Error::NotFound {
                entity: format!("scope {}", params.scope_id),
            })?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&scope),
            AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let allowed = authz::decide_knowledge_read(
            &state,
            &input,
            Resource::Scope(params.scope_id),
            synveda_types::Sensitivity::Public,
        )?;
        let effective =
            configuration_store::effective_at_scope(&mut tx, tenant, params.scope_id).await?;
        let policies = KnowledgeType::ALL
            .iter()
            .map(|kind| FreshnessPolicy::from_effective(&effective, *kind).into())
            .collect::<Vec<_>>();
        read_event(
            &mut tx,
            tenant,
            "knowledge.freshness.list",
            &allowed,
            Resource::Scope(params.scope_id),
            json!({
                "scope_id": params.scope_id,
                "configuration_version_id": effective.version_id,
                "configuration_hash": effective.content_hash,
                "policy_count": policies.len(),
                "action": Action::KnowledgeRead.as_str(),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(FreshnessPolicyListView { policies }))
    }
    .await;
    respond(&state, "knowledge.freshness.list", result).await
}

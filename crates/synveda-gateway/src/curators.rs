//! Curator files per scope (FLOW-3, ADR-0032 decisions 13–15):
//! `/v1/admin/scopes/{scope_id}/curators`, beside `/policy` — both
//! re-homed under the scope admin prefix when CPR-7 deleted `/v1/hierarchy`.
//!
//! A curator file names who must **additionally** approve a proposal
//! touching matching assets at this scope. It adds requirements; it never
//! grants authority — a named subject still has to pass `ProposalReview`,
//! so a file naming someone the pack denies makes proposals unsatisfiable
//! rather than making that person an approver.
//!
//! Written through `PolicyAssign` and read through `PolicyRead` rather
//! than a new action pair: the steward who can swap the entire pack — and
//! with it the entire matrix — can obviously edit the file that pack's
//! matrix composes with, and a separate action would imply a separable
//! authority that does not exist.
//!
//! There is no DELETE: refs hold no delete grant (migration 0018), so
//! clearing a scope's requirements is committing an empty file — which
//! also leaves the removal in the history, where a delete would have left
//! nothing.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, Resource};
use synveda_store::{rls, scopes};
use synveda_types::{Error, IdentityId, Result, ScopeId};
use synveda_vedaflow::{self as vedaflow, CuratorFile, PolicySnapshot, Signer};

use crate::app::AppState;
use crate::audit;
use crate::authz;
use crate::error::ApiError;
use crate::request::{body, commit, found, tenant_id};
use crate::telemetry::CURATOR_OPERATIONS_TOTAL;

/// The commit-message cap; mirrors `vedaflow_commits`' CHECK.
const MAX_MESSAGE_CHARS: usize = 4096;

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
    metrics::counter!(CURATOR_OPERATIONS_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// One rule, parsed — rendered beside the source so a console can show
/// what the file *means* without reimplementing the parser.
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = CuratorRuleView)]
pub(crate) struct RuleView {
    pattern: String,
    approvers: Vec<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct CuratorsResponse {
    /// The node asked about.
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// The scope the effective file is committed at — this node, or the
    /// nearest ancestor carrying one (ADR-0032 decision 14). Absent when
    /// no scope on the chain has a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    effective_at: Option<ScopeId>,
    /// The file's exact authored bytes, comments included.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    /// The parsed rules.
    rules: Vec<RuleView>,
    /// The commit the `curators` ref points at.
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    /// The file's content address.
    #[serde(skip_serializing_if = "Option::is_none")]
    object_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    updated_by: Option<IdentityId>,
}

/// `GET /v1/admin/scopes/{scope_id}/curators` — the curator file in force
/// for this node: its own, or the nearest ancestor's.
#[utoipa::path(
    get,
    path = "/v1/admin/scopes/{scope_id}/curators",
    operation_id = "get_scope_curators",
    tag = "policy",
    params(("scope_id" = String, Path, format = "uuid")),
    responses(
        (status = 200, description = "The effective curator file", body = CuratorsResponse),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy read is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The scope is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "curators.get", skip_all)]
pub(crate) async fn get(State(state): State<AppState>, Path(scope_id): Path<ScopeId>) -> Response {
    let result = async {
        let tenant_id = tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
        let node = found(
            scopes::get(&mut *tx, tenant_id, scope_id).await?,
            tenant_id,
            scope_id,
        )?;
        let input = authz::gather(
            &state,
            &mut tx,
            Some(&node),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let authorized = authz::decide(
            &state,
            &input,
            Action::PolicyRead,
            Resource::Scope(scope_id),
        )?;
        let chain: Vec<ScopeId> = input.chain.iter().map(|node| node.id).collect();
        let stored = vedaflow::nearest_curators(&mut tx, tenant_id, &chain).await?;
        audit::record(
            &mut tx,
            tenant_id,
            AuditAction::AuthzDecision,
            Resource::Scope(scope_id).to_string(),
            Outcome::Allow,
            json!({
                "op": "curators.get",
                "authz": audit::decision_context(Action::PolicyRead, &authorized),
                "effective_at": stored.as_ref().map(|stored| stored.scope_id),
            }),
        )
        .await?;
        commit(tx).await?;
        Ok(Json(match stored {
            None => CuratorsResponse {
                scope_id,
                effective_at: None,
                source: None,
                rules: Vec::new(),
                commit: None,
                object_hash: None,
                updated_at: None,
                updated_by: None,
            },
            Some(stored) => CuratorsResponse {
                scope_id,
                effective_at: Some(stored.scope_id),
                source: Some(stored.file.source().to_owned()),
                rules: stored
                    .file
                    .rules()
                    .iter()
                    .map(|rule| RuleView {
                        pattern: rule.pattern.clone(),
                        approvers: rule.approvers.iter().map(ToString::to_string).collect(),
                    })
                    .collect(),
                commit: Some(stored.commit.to_hex()),
                object_hash: Some(stored.object.to_hex()),
                updated_at: Some(stored.updated_at),
                updated_by: Some(stored.updated_by),
            },
        }))
    }
    .await;
    respond(&state, "get", result).await
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = CuratorsPutBody)]
pub(crate) struct PutBody {
    /// The file's text. An empty file clears this scope's requirements —
    /// there is no delete, because the removal is history too.
    source: String,
    /// Why — an auditor reads this.
    message: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = CuratorsPutResponse)]
pub(crate) struct PutResponse {
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    object_hash: String,
    /// Whether the bytes were already stored: a re-commit of an unchanged
    /// file, which still records who re-asserted it and when.
    unchanged: bool,
    rules: usize,
}

/// `PUT /v1/admin/scopes/{scope_id}/curators` — commit this scope's curator
/// file.
#[utoipa::path(
    put,
    path = "/v1/admin/scopes/{scope_id}/curators",
    operation_id = "put_scope_curators",
    tag = "policy",
    params(("scope_id" = String, Path, format = "uuid")),
    request_body = PutBody,
    responses(
        (status = 200, description = "The committed curator file", body = PutResponse),
        (status = 400, description = "The curator file or message is invalid", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 403, description = "Policy management is not permitted", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "The scope is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
#[tracing::instrument(name = "curators.put", skip_all)]
pub(crate) async fn put(
    State(state): State<AppState>,
    Path(scope_id): Path<ScopeId>,
    payload: std::result::Result<Json<PutBody>, JsonRejection>,
) -> Response {
    let result = put_inner(&state, scope_id, payload).await;
    respond(&state, "put", result).await
}

async fn put_inner(
    state: &AppState,
    scope_id: ScopeId,
    payload: std::result::Result<Json<PutBody>, JsonRejection>,
) -> Result<Json<PutResponse>> {
    let body = body(payload)?;
    let chars = body.message.chars().count();
    if chars == 0 || chars > MAX_MESSAGE_CHARS {
        return Err(Error::Invalid {
            message: format!("message must be 1..={MAX_MESSAGE_CHARS} characters"),
        });
    }
    // Parsed before anything is written: a file that does not parse is
    // refused here rather than stored and discovered at review time.
    let file = CuratorFile::parse(&body.source)?;

    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let node = found(
        scopes::get(&mut *tx, tenant_id, scope_id).await?,
        tenant_id,
        scope_id,
    )?;
    let input = authz::gather(
        state,
        &mut tx,
        Some(&node),
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let authorized = authz::decide(
        state,
        &input,
        Action::PolicyAssign,
        Resource::Scope(scope_id),
    )?;
    let author = input
        .identity
        .as_ref()
        .map(|identity| identity.id)
        .ok_or_else(|| Error::Invalid {
            message: "editing a curator file requires a provisioned identity".to_owned(),
        })?;

    let snapshot = PolicySnapshot::new(
        authorized.decision.pack_name.clone(),
        authorized.decision.pack_version,
    );
    let committed = vedaflow::write_curators(
        &mut tx,
        tenant_id,
        &vedaflow::CuratorWrite {
            scope: scope_id,
            file: &file,
            author,
            message: &body.message,
            committed_at: Utc::now(),
            policy_snapshot: &snapshot,
        },
        &Signer::Unsigned,
    )
    .await?;

    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::PolicyNodeAssigned,
        Resource::Scope(scope_id).to_string(),
        Outcome::Success,
        json!({
            "op": "curators.put",
            "authz": audit::decision_context(Action::PolicyAssign, &authorized),
            "message": body.message,
            "commit": committed.commit.to_hex(),
            "parent": committed.parent.map(|parent| parent.to_hex()),
            // The address, not the text: an auditor recomputes the file
            // from the object store, and the rule summary says what
            // changed without duplicating it into the chain.
            "object_hash": committed.object.to_hex(),
            "unchanged": committed.unchanged,
            "rules": file.rules().len(),
            "approvers": file
                .rules()
                .iter()
                .flat_map(|rule| rule.approvers.iter().map(ToString::to_string))
                .collect::<std::collections::BTreeSet<_>>(),
        }),
    )
    .await?;
    commit(tx).await?;

    Ok(Json(PutResponse {
        scope_id,
        commit: committed.commit.to_hex(),
        parent: committed.parent.map(|parent| parent.to_hex()),
        object_hash: committed.object.to_hex(),
        unchanged: committed.unchanged,
        rules: file.rules().len(),
    }))
}

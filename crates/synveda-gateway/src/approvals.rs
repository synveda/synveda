//! The one place the approval matrix is resolved (FLOW-3, ADR-0032
//! decisions 3 and 8).
//!
//! Both paths across the trust boundary come through here: the direct
//! `POST /v1/channels/{scope}/publish`, where the acting principal counts
//! as the only approver, and a proposal's effect, where the recorded
//! approvals are counted. One resolution function is the point — a matrix
//! that governs one path is not a matrix, and if the two surfaces
//! resolved separately they would eventually disagree.
//!
//! Resolution is **live**, at every decision point, never frozen at open
//! time: a pack switch governs the very next request (ADR-0014
//! decision 3), and freezing would create a second, staler answer to a
//! question the product answers one way everywhere else. What each act
//! *records* is the requirement as it stood then, so the audit trail is
//! still readable after the pack changes.

use serde::Serialize;
use serde_json::Value;
use sqlx::PgConnection;
use synveda_policy::{Resource, effective_roles_at};
use synveda_types::{
    ApprovalRequirement, AssetKind, CastApproval, Error, HierarchyNode, Outstanding,
    RequirementOrigin, Result, Sensitivity, TenantId,
};
use synveda_vedaflow as vedaflow;

use crate::app::AppState;
use crate::authz::DecisionInput;
use crate::telemetry::PUBLISH_REVIEW_REQUIRED_TOTAL;

/// The publication a requirement is resolved for: the target node whose
/// channel would move, what is moving, and at what classification.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Requested<'a> {
    /// The target scope. Its kind and its effective pack both feed the
    /// matrix, and its chain carries the curator file.
    pub(crate) target: &'a HierarchyNode,
    /// Which asset type.
    pub(crate) asset: AssetKind,
    /// The **maximum** sensitivity over the set: a set is reviewed as a
    /// set, so it is governed by its most sensitive element.
    pub(crate) sensitivity: Sensitivity,
    /// The member names — record ids for memories. Curator-file patterns
    /// match `{asset-kind}/{entry}`, and a rule matching *any* member
    /// governs the whole set.
    pub(crate) entries: &'a [String],
}

/// What it takes to publish `requested` onto its target's channel, under
/// the pack in force there and the nearest curator file on its chain.
pub(crate) async fn resolve(
    state: &AppState,
    conn: &mut PgConnection,
    tenant_id: TenantId,
    input: &DecisionInput,
    requested: &Requested<'_>,
) -> Result<ApprovalRequirement> {
    let Requested {
        target,
        asset,
        sensitivity,
        entries,
    } = *requested;
    let pack = state
        .pdp
        .effective(tenant_id, Resource::Scope(target.id), &input.context());
    // The floor is merged inside `resolve`; there is no way to ask for a
    // requirement without it (ADR-0032 decision 4).
    let mut requirement = pack.approvals.resolve(asset, sensitivity, target.kind);
    // Nearest-ancestor-first, like pack assignment: the first scope on
    // the chain carrying a file wins outright, no union (ADR-0032
    // decision 14). `input.chain` runs nearest-first (HIER-2's order).
    let chain: Vec<_> = input.chain.iter().map(|node| node.id).collect();
    if let Some(stored) = vedaflow::nearest_curators(conn, tenant_id, &chain).await? {
        stored
            .file
            .apply(stored.scope_id, asset, entries, &mut requirement);
    }
    Ok(requirement)
}

/// The acting principal's effective roles at `target` — the same set the
/// PDP weighed for the decision that got them here.
pub(crate) fn roles_at(input: &DecisionInput, target: &HierarchyNode) -> Vec<synveda_types::Role> {
    effective_roles_at(
        &input.principal,
        Resource::Scope(target.id),
        &input.context(),
    )
}

/// The direct publish route's gate (ADR-0032 decision 8): the publisher
/// counts as one approver holding their effective roles, and the call
/// proceeds only if that single approval satisfies the requirement.
///
/// A curator publishing internal memory under `regulated-strict` still
/// works — the matrix asks for one curator and one curator acted. A
/// `restricted` record refuses, names what is missing, and points at the
/// proposal route, which is the whole point: the direct route did not
/// become a hole to close, it became the degenerate case where one
/// approval is enough.
pub(crate) fn require_single_actor(
    requirement: &ApprovalRequirement,
    actor: &CastApproval,
    surface: &'static str,
) -> Result<()> {
    let outstanding = requirement.outstanding(std::slice::from_ref(actor));
    if outstanding.is_empty() {
        return Ok(());
    }
    metrics::counter!(PUBLISH_REVIEW_REQUIRED_TOTAL, "surface" => surface).increment(1);
    Err(Error::PolicyDenied {
        action: "channel.publish".to_owned(),
        resource: "the approval matrix".to_owned(),
        reason: format!(
            "publishing this here needs {} beyond what one principal can supply; \
             open a proposal (POST /v1/proposals) — still outstanding: {}",
            requirement.describe(),
            outstanding.describe()
        ),
    })
}

/// A requirement as the API and the audit payload render it.
#[derive(Serialize)]
pub(crate) struct RequirementView {
    /// Roles required, with counts.
    pub(crate) roles: Vec<RoleView>,
    /// Distinct identities required.
    pub(crate) distinct_approvers: u8,
    /// Named subjects a curator file requires.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) subjects: Vec<String>,
    /// Where the requirement came from: `floor`, `pack`, and the scope of
    /// any curator file that contributed — so a trail explains what a
    /// proposal needed without reading a pack that has since changed.
    pub(crate) origins: Vec<String>,
}

/// One role line.
#[derive(Serialize)]
pub(crate) struct RoleView {
    pub(crate) role: String,
    pub(crate) count: u8,
}

impl RequirementView {
    /// Renders `requirement` for a response body.
    pub(crate) fn of(requirement: &ApprovalRequirement) -> Self {
        RequirementView {
            roles: requirement
                .roles
                .iter()
                .map(|required| RoleView {
                    role: required.role.as_str().to_owned(),
                    count: required.count,
                })
                .collect(),
            distinct_approvers: requirement.distinct_approvers,
            subjects: requirement.subjects.clone(),
            origins: requirement
                .origins
                .iter()
                .map(RequirementOrigin::label)
                .collect(),
        }
    }
}

/// The audit payload fragment every act against the matrix carries.
/// The shape is [`ApprovalRequirement::audit_view`]'s: since FLOW-4 the
/// rule engine writes these events too, and one shape is the point
/// (ADR-0032's compliance note, ADR-0033 decision 9).
pub(crate) fn audit_context(requirement: &ApprovalRequirement, outstanding: &Outstanding) -> Value {
    serde_json::to_value(requirement.audit_view(outstanding))
        .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
}

//! The capability probe (CNSL-2, ADR-0058): what the **caller** may do at
//! a scope, answered by the PDP rather than re-derived from role bindings.
//!
//! # A forecast, never a grant
//!
//! This is the load-bearing property of the whole surface (ADR-0058
//! decision 2), so it is stated here where somebody reading the code will
//! meet it: **nothing in this product reads a capability answer in order to
//! decide anything.** Every act still takes its own decision at its own
//! seam, under the pack effective *then*. If a probe says yes and a pack
//! changes and the act is refused, the act's decision is the one that
//! decided and the probe was a forecast that aged. Clients use this to
//! choose what to **offer** and never to choose what to **allow** — there
//! is exactly one enforcement point and it is not here.
//!
//! # It answers about the caller and nobody else
//!
//! There is no `subject` parameter and there must not be one (ADR-0058
//! decision 3). "What may I do here" discloses nothing about a third party;
//! "who may do what here" is `RoleRead` on the roles route, with its own
//! denial. An explorer that answered the second question through this route
//! would be an enumeration oracle for an organisation's whole role
//! assignment, one 403 at a time.
//!
//! # One event, not one per pair
//!
//! A probe is a fan-out of decisions and chains **one** summarised
//! `authz.decision` (ADR-0058 decision 4). That is not an exemption from
//! AUD-1: it is ADR-0019 decision 4's second sentence — CTX-2's
//! per-candidate sweep aggregating into the request-level event, with the
//! per-call detail left in traces — arriving on the admin plane, where the
//! first sentence ("every allowed admin-plane read") would otherwise price
//! rendering a tree at thousands of rows.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{Action, EntityBatch, Resource, effective_roles_at};
use synveda_store::{hierarchy, rls};
use synveda_types::{Error, HierarchyNode, Result, Role, ScopeId, Sensitivity};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::hierarchy::{commit, found, tenant_id};
use crate::policy::{OriginView, origin_view};
use crate::telemetry::CAPABILITY_PROBES_TOTAL;

/// The most scopes one batch probe answers.
///
/// A bound a screen can exceed is a bound the screen works around, so this
/// is the API's rather than the client's, and the response says what it did
/// not answer instead of truncating silently (ADR-0058 decision 5;
/// EVAL-5's "no silent caps" one plane over).
pub(crate) const MAX_BATCH_SCOPES: usize = 128;

/// What a probe says about one scope.
#[derive(Serialize)]
pub(crate) struct NodeCapabilities {
    scope_id: ScopeId,
    /// Where the node sits — a fact about the *node*, so it is served only
    /// to a caller who may read it (`HierarchyRead`). Absent otherwise, and
    /// the verdicts beside it are unaffected: they are about the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    scope_path: Option<String>,
    /// The pack the answers were decided under, and where it came from.
    /// Carried because a capability is only true *under a pack*, and a
    /// client comparing a forecast to a later refusal needs to see that the
    /// pack moved underneath it — and withheld, like `scope_path`, from a
    /// caller who may not read the node's governance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pack: Option<PackView>,
    /// The caller's own effective roles here — the caller's, never anyone
    /// else's (decision 3).
    roles: Vec<Role>,
    /// The operand-free actions, by their stable machine name.
    actions: BTreeMap<&'static str, bool>,
    /// The tier-bearing reads: the tiers each permits here, ascending. An
    /// empty list is a real answer — "nothing at this scope, at any tier".
    read_tiers: BTreeMap<&'static str, Vec<Sensitivity>>,
    /// `RoleAssign` per role, because it fails closed without
    /// `context.grant` and because "which roles may I bind here" is the
    /// question an explorer actually asks.
    role_assign: BTreeMap<&'static str, bool>,
}

#[derive(Serialize)]
pub(crate) struct PackView {
    name: String,
    version: i64,
    origin: OriginView,
}

/// The batch response. `not_answered` names the scopes the bound dropped,
/// and `max_scopes` says what the bound is, so a client can page rather
/// than guess.
#[derive(Serialize)]
pub(crate) struct BatchResponse {
    capabilities: Vec<NodeCapabilities>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    not_answered: Vec<ScopeId>,
    max_scopes: usize,
}

#[derive(Deserialize)]
pub(crate) struct BatchParams {
    /// Comma-separated scope ids — the nodes the client actually rendered.
    scopes: String,
}

/// Counts the probe and renders the result, the same outcome taxonomy the
/// hierarchy, policy and role planes use.
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
    metrics::counter!(CAPABILITY_PROBES_TOTAL, "op" => op, "outcome" => outcome).increment(1);
    match result {
        Ok(response) => response.into_response(),
        Err(error) => {
            audit::record_rejection(state, op, &error).await;
            ApiError(error).into_response()
        }
    }
}

/// What the caller may do on the **tenant** plane — `whoami`'s block.
#[derive(Serialize)]
pub(crate) struct TenantCapabilities {
    /// The caller's tenant-wide effective roles. Node bindings are absent
    /// by construction: [`effective_roles_at`] keeps only the tenant-wide
    /// rows for a tenant resource, which is the same rule the decisions
    /// beside it ran under.
    roles: Vec<Role>,
    actions: BTreeMap<&'static str, bool>,
    role_assign: BTreeMap<&'static str, bool>,
}

/// The tenant-plane probe, for `GET /v1/whoami?capabilities=true`.
///
/// Much shorter than a scope's, and honestly so: most of the vocabulary is
/// about a node, and an action only ever taken at a node has no
/// tenant-level answer to give (see [`Action::PROBED_AT_TENANT`]). There
/// are no tiered reads here at all — every one of them applies to `Scope`
/// alone.
///
/// It chains no audit event. That is not an omission: the tenant probe
/// takes no `gather` against a resource node, decides only about the
/// caller, and is attached to a route ADR-0019 decision 6 already keeps
/// off the chain — and a per-request event on the call a console makes on
/// every page load would be the row flood decision 4 exists to prevent,
/// arriving through the other door.
pub(crate) async fn at_tenant(state: &AppState) -> Result<TenantCapabilities> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let input = authz::gather(state, &mut tx, None).await?;
    let resource = Resource::Tenant(tenant_id);
    let context = input.context();
    let batch =
        state
            .pdp
            .materialise(&input.principal, &[&input.chain], &input.principal_scopes)?;

    let mut actions = BTreeMap::new();
    for action in Action::PROBED_AT_TENANT {
        let decision =
            state
                .pdp
                .authorize_with(&batch, &input.principal, action, resource, &context)?;
        actions.insert(action.as_str(), decision.allowed);
    }
    let mut role_assign = BTreeMap::new();
    for role in Role::ALL {
        let decision = state.pdp.authorize_with(
            &batch,
            &input.principal,
            Action::RoleAssign,
            resource,
            &authz::context_granting(&input, role),
        )?;
        role_assign.insert(role.as_str(), decision.allowed);
    }
    let roles = effective_roles_at(&input.principal, resource, &context);
    commit(tx).await?;

    metrics::counter!(CAPABILITY_PROBES_TOTAL, "op" => "at_tenant", "outcome" => "ok").increment(1);
    Ok(TenantCapabilities {
        roles,
        actions,
        role_assign,
    })
}

/// `GET /v1/hierarchy/nodes/{id}/capabilities` — what this caller may do
/// at one node.
pub(crate) async fn at_node(State(state): State<AppState>, Path(id): Path<ScopeId>) -> Response {
    let result = probe(&state, &[id], "at_node").await.map(|(mut batch, _)| {
        // A single-node probe answers about one node or 404s before it gets
        // here, so the batch envelope would be noise.
        Json(batch.capabilities.remove(0))
    });
    respond(&state, "at_node", result).await
}

/// `GET /v1/capabilities?scopes=<id>,<id>,…` — the plural of the same
/// walk, for the nodes a tree actually rendered.
pub(crate) async fn batch(
    State(state): State<AppState>,
    Query(params): Query<BatchParams>,
) -> Response {
    let result = async {
        let requested = parse_scopes(&params.scopes)?;
        let (batch, _) = probe(&state, &requested, "batch").await?;
        Ok(Json(batch))
    }
    .await;
    respond(&state, "batch", result).await
}

/// Splits and parses the `scopes` parameter, refusing an empty ask rather
/// than answering it with an empty list — a client that sent no ids has a
/// bug, and an empty 200 hides it.
fn parse_scopes(raw: &str) -> Result<Vec<ScopeId>> {
    let mut ids = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id: ScopeId = trimmed.parse().map_err(|_| Error::Invalid {
            message: format!("scopes: `{trimmed}` is not a scope id"),
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(Error::Invalid {
            message: "scopes: name at least one scope id".to_owned(),
        });
    }
    Ok(ids)
}

/// The probe itself, shared by both routes.
///
/// Returns the answers and the number of (node, action) pairs decided —
/// the second is what the single summarised audit event reports, and it is
/// the number that would otherwise have been a row count.
async fn probe(
    state: &AppState,
    requested: &[ScopeId],
    op: &'static str,
) -> Result<(BatchResponse, usize)> {
    let tenant_id = tenant_id()?;
    let (answered, not_answered) = split_at_bound(requested);
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;

    // Uniform-404 ownership first, as everywhere: a node this tenant does
    // not own is not found, so a probe can never widen the set of scopes a
    // caller could already enumerate.
    let mut nodes: Vec<HierarchyNode> = Vec::with_capacity(answered.len());
    for id in &answered {
        nodes.push(found(
            hierarchy::node(&mut *tx, *id).await?,
            tenant_id,
            *id,
        )?);
    }

    // **The probe takes no permission of its own beyond ownership**, which
    // is ADR-0058 decision 3 read literally — "no permission beyond the
    // visibility the node already requires ... uniform-404 ownership first,
    // as everywhere". An answer about what *you* may do discloses nothing
    // about anybody else, so there is nothing here for a permission to
    // protect.
    //
    // The first cut required `HierarchyRead` and CNSL-2's own demo found
    // what that costs: under every shipped pack that action is
    // steward/org-admin/auditor only, so a **curator** — the role the
    // proposals inbox exists for — was refused the probe outright and shown
    // no verdict buttons at all. A capability surface that only privileged
    // readers may consult is worse than none: it hides acts from exactly
    // the readers who hold them.
    //
    // What `HierarchyRead` *does* still decide is the **node detail**. The
    // verdicts are about the caller and always served; `scope_path` and the
    // effective pack are facts about the node, so a caller who may not read
    // the node does not receive them and the route cannot become a
    // node-metadata oracle for anyone holding a scope id.
    let mut answers = Vec::with_capacity(nodes.len());
    let mut pairs = 0usize;
    let mut allowed_pairs = 0usize;
    let mut gate = None;
    for node in &nodes {
        let input = authz::gather(state, &mut tx, Some(node)).await?;
        let readable = authz::decide(
            state,
            &input,
            Action::HierarchyRead,
            Resource::Scope(node.id),
            None,
        );
        let may_read_node = readable.is_ok();
        let answer = answer_for(state, &input, node, may_read_node)?;
        pairs += answer.pair_count();
        allowed_pairs += answer.allowed_count();
        answers.push(answer);
        if let Ok(authorized) = readable {
            gate.get_or_insert(authorized);
        }
    }

    // One event for the whole fan-out (decision 4). The payload carries
    // counts and scope ids — never a third party's binding, and never a
    // lapse's reason, which is free text a steward wrote about an incident.
    let mut payload = json!({
        "op": "capabilities",
        "route": op,
        "scopes": answered,
        "scopes_answered": answers.len(),
        "scopes_not_answered": not_answered.len(),
        "pairs_decided": pairs,
        "pairs_allowed": allowed_pairs,
    });
    // The decision context when at least one node was readable. A probe by a
    // caller who may read none of them still chains — somebody sweeping an
    // organisation's admin surface is exactly the reconnaissance an audit
    // log should show, and it is the probes that answer *nothing* that most
    // want recording.
    if let Some(authorized) = gate {
        payload["authz"] = audit::decision_context(Action::HierarchyRead, &authorized);
    }
    audit::record(
        &mut tx,
        tenant_id,
        AuditAction::AuthzDecision,
        Resource::Tenant(tenant_id).to_string(),
        Outcome::Allow,
        payload,
    )
    .await?;
    commit(tx).await?;

    metrics::counter!(CAPABILITY_PROBES_TOTAL, "op" => "pairs", "outcome" => "decided")
        .increment(pairs as u64);

    Ok((
        BatchResponse {
            capabilities: answers.into_iter().map(|answer| answer.node).collect(),
            not_answered,
            max_scopes: MAX_BATCH_SCOPES,
        },
        pairs,
    ))
}

/// Splits the requested ids at [`MAX_BATCH_SCOPES`], keeping the order the
/// caller asked in so paging is predictable.
fn split_at_bound(requested: &[ScopeId]) -> (Vec<ScopeId>, Vec<ScopeId>) {
    if requested.len() <= MAX_BATCH_SCOPES {
        return (requested.to_vec(), Vec::new());
    }
    let (head, tail) = requested.split_at(MAX_BATCH_SCOPES);
    (head.to_vec(), tail.to_vec())
}

/// One node's answers plus the counts the audit event summarises.
struct Answer {
    node: NodeCapabilities,
}

impl Answer {
    fn pair_count(&self) -> usize {
        self.node.actions.len() + self.node.read_tiers.len() + self.node.role_assign.len()
    }

    fn allowed_count(&self) -> usize {
        self.node
            .actions
            .values()
            .filter(|allowed| **allowed)
            .count()
            + self
                .node
                .read_tiers
                .values()
                .filter(|tiers| !tiers.is_empty())
                .count()
            + self
                .node
                .role_assign
                .values()
                .filter(|allowed| **allowed)
                .count()
    }
}

/// Decides every probed pair at one node, against one materialised entity
/// store.
///
/// The entity store is the expensive part of a Cedar decision, not the
/// evaluation (ADR-0042 decision 6 measured it), so it is built once here
/// and reused across ~50 asks. Sharing it changes no verdict: a batch
/// missing a chain produces a *denial*, never a wrong allow.
fn answer_for(
    state: &AppState,
    input: &DecisionInput,
    node: &HierarchyNode,
    may_read_node: bool,
) -> Result<Answer> {
    let resource = Resource::Scope(node.id);
    let context = input.context();
    let batch: EntityBatch =
        state
            .pdp
            .materialise(&input.principal, &[&input.chain], &input.principal_scopes)?;

    let mut actions = BTreeMap::new();
    for action in Action::PROBED_AT_SCOPE {
        let decision =
            state
                .pdp
                .authorize_with(&batch, &input.principal, action, resource, &context)?;
        actions.insert(action.as_str(), decision.allowed);
    }

    // Three of the four tiered reads come back from one pack resolution;
    // `PromptRead` is asked separately because `PermittedTiers` serves the
    // composition path and is not this surface's to widen.
    let tiers = state
        .pdp
        .permitted_read_tiers(&batch, &input.principal, node.id, &context)?;
    let mut read_tiers = BTreeMap::new();
    read_tiers.insert(Action::MemoryRead.as_str(), tiers.memory);
    read_tiers.insert(Action::ContextPackRead.as_str(), tiers.context_pack);
    read_tiers.insert(Action::SkillRead.as_str(), tiers.skill);
    read_tiers.insert(
        Action::PromptRead.as_str(),
        permitted_prompt_tiers(state, input, &batch, resource)?,
    );

    let mut role_assign = BTreeMap::new();
    for role in Role::ALL {
        let decision = state.pdp.authorize_with(
            &batch,
            &input.principal,
            Action::RoleAssign,
            resource,
            &authz::context_granting(input, role),
        )?;
        role_assign.insert(role.as_str(), decision.allowed);
    }

    Ok(Answer {
        node: NodeCapabilities {
            scope_id: node.id,
            scope_path: may_read_node.then(|| node.path.clone()),
            pack: may_read_node.then(|| PackView {
                name: tiers.effective.name.clone(),
                version: tiers.effective.version,
                origin: origin_view(&tiers.effective),
            }),
            roles: effective_roles_at(&input.principal, resource, &context),
            actions,
            read_tiers,
            role_assign,
        },
    })
}

/// The tiers `PromptRead` permits here, ascending.
fn permitted_prompt_tiers(
    state: &AppState,
    input: &DecisionInput,
    batch: &EntityBatch,
    resource: Resource,
) -> Result<Vec<Sensitivity>> {
    let mut permitted = Vec::new();
    for tier in Sensitivity::ALL {
        let decision = state.pdp.authorize_with(
            batch,
            &input.principal,
            Action::PromptRead,
            resource,
            &authz::context_at_tier(input, tier),
        )?;
        if decision.allowed {
            permitted.push(tier);
        }
    }
    Ok(permitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_parse_trims_dedups_and_keeps_order() {
        let a = ScopeId::new();
        let b = ScopeId::new();
        let parsed = parse_scopes(&format!(" {a} , {b},{a} ")).expect("parses");
        assert_eq!(parsed, vec![a, b], "deduped, in the order asked");
    }

    #[test]
    fn an_empty_ask_is_refused_rather_than_answered_emptily() {
        // A client that sent no ids has a bug; a 200 with an empty list is
        // that bug rendered as a successful answer about nothing.
        let error = parse_scopes("  , ,").expect_err("refused");
        assert!(matches!(error, Error::Invalid { .. }));
    }

    #[test]
    fn a_junk_scope_id_names_itself() {
        let error = parse_scopes("not-a-uuid").expect_err("refused");
        let Error::Invalid { message } = error else {
            panic!("expected Invalid");
        };
        assert!(
            message.contains("not-a-uuid"),
            "names the offender: {message}"
        );
    }

    #[test]
    fn the_bound_splits_rather_than_truncates() {
        let ids: Vec<ScopeId> = (0..MAX_BATCH_SCOPES + 3).map(|_| ScopeId::new()).collect();
        let (answered, not_answered) = split_at_bound(&ids);
        assert_eq!(answered.len(), MAX_BATCH_SCOPES);
        assert_eq!(not_answered.len(), 3, "the rest is named, never dropped");
        assert_eq!(
            [answered, not_answered].concat(),
            ids,
            "order is the caller's, so paging is predictable"
        );
    }
}

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
//! "who may do what here" was `RoleRead` on the roles route CPR-7 deleted, with its own
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
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use synveda_audit::{AuditAction, Outcome};
use synveda_policy::{
    Action, AuthzContext, EntityBatch, Resource, ScopeNode, effective_role_keys_at,
};
use synveda_store::{policy_assignments, rls, scopes};
use synveda_types::access::RoleKey;
use synveda_types::scope::Scope;
use synveda_types::{Error, PolicyAssignment, Result, ScopeId, Sensitivity};

use crate::app::AppState;
use crate::audit;
use crate::authz::{self, DecisionInput};
use crate::error::ApiError;
use crate::policy::{OriginView, origin_view};
use crate::request::{commit, found, tenant_id};
use crate::telemetry::CAPABILITY_PROBES_TOTAL;

/// The most scopes one batch probe answers.
///
/// A bound a screen can exceed is a bound the screen works around, so this
/// is the API's rather than the client's, and the response says what it did
/// not answer instead of truncating silently (ADR-0058 decision 5;
/// EVAL-5's "no silent caps" one plane over).
pub(crate) const MAX_BATCH_SCOPES: usize = 128;

/// What a probe says about one scope.
#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct NodeCapabilities {
    #[schema(value_type = String, format = "uuid")]
    scope_id: ScopeId,
    /// Where the node sits — a fact about the *node*, so it is served only
    /// to a caller who may read it (`ScopeRead`). Absent otherwise, and
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
    /// The caller's own effective role keys here — the caller's, never
    /// anyone else's (decision 3; since the cutover, the only roles there
    /// are — CPR-6, ADR-0073 decision 5).
    #[schema(value_type = Vec<String>)]
    roles: Vec<RoleKey>,
    /// The operand-free actions, by their stable machine name.
    #[schema(value_type = BTreeMap<String, bool>)]
    actions: BTreeMap<&'static str, bool>,
    /// The tier-bearing reads: the tiers each permits here, ascending. An
    /// empty list is a real answer — "nothing at this scope, at any tier".
    #[schema(value_type = BTreeMap<String, Vec<String>>)]
    read_tiers: BTreeMap<&'static str, Vec<Sensitivity>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct PackView {
    name: String,
    version: i64,
    origin: OriginView,
}

/// The batch response. `not_answered` names the scopes the bound dropped,
/// and `max_scopes` says what the bound is, so a client can page rather
/// than guess.
#[derive(Serialize, utoipa::ToSchema)]
pub(crate) struct BatchResponse {
    capabilities: Vec<NodeCapabilities>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<String>)]
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

/// What the caller may do on the **tenant** plane — `whoami`'s block, and
/// since CPR-4 `/v1/me`'s.
///
/// Carries a `ToSchema` because `/v1/me` embeds it and that route is on the
/// OpenAPI contract. The three fields are declared to the document by
/// `value_type` rather than derived: `Role` and the `&'static str` map keys
/// live in `synveda-types`, where `utoipa` deliberately does not reach — a
/// contract is a property of the surface, and no crate below the gateway has
/// one.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TenantCapabilities {
    /// The caller's role keys at the **tenant root scope** — the grants
    /// that reach the whole boundary (CPR-6, ADR-0073 decision 5; since
    /// the cutover, the only roles there are).
    #[schema(value_type = Vec<String>)]
    role_keys: Vec<RoleKey>,
    /// Every operand-free tenant-plane action, by its stable machine name.
    #[schema(value_type = BTreeMap<String, bool>)]
    actions: BTreeMap<&'static str, bool>,
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
pub async fn at_tenant(state: &AppState) -> Result<TenantCapabilities> {
    let tenant_id = tenant_id()?;
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant_id).await?;
    let input = authz::gather(
        state,
        &mut tx,
        None,
        synveda_store::anchors::AnchorSelection::none(),
        Vec::new(),
    )
    .await?;
    let resource = Resource::Tenant(tenant_id);
    let context = input.context();
    let batch = state
        .pdp
        .materialise(&input.principal, &[&input.chain], &context)?;

    let mut actions = BTreeMap::new();
    for action in Action::PROBED_AT_TENANT {
        let decision =
            state
                .pdp
                .authorize_with(&batch, &input.principal, action, resource, &context)?;
        actions.insert(action.as_str(), decision.allowed);
    }
    let role_keys = effective_role_keys_at(resource, &context);
    commit(tx).await?;

    metrics::counter!(CAPABILITY_PROBES_TOTAL, "op" => "at_tenant", "outcome" => "ok").increment(1);
    Ok(TenantCapabilities { role_keys, actions })
}

/// The most anchors one `/v1/me` answers capabilities for.
///
/// A bound the response reports rather than hides, exactly like
/// [`MAX_BATCH_SCOPES`]: a caller with two hundred grants gets the most
/// specific thirty-two and is told how many were left. Thirty-two because each
/// anchor costs one chain read, one assignment read and
/// [`Action::PROBED_AT_SCOPE`]'s worth of Cedar evaluations against a shared
/// entity batch, and `/v1/me` is the call a client makes on every page load.
pub(crate) const MAX_ANCHOR_CAPABILITIES: usize = 32;

/// What the caller may do at **one anchor** — a real decision at a real scope,
/// never a shape derived from an edition (CPR-6, ADR-0073 decision 8).
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AnchorCapabilities {
    /// The scope.
    #[schema(value_type = String, format = "uuid")]
    pub scope_id: ScopeId,
    /// Its shape: `tenant`, `org_unit`, `workspace`, `project` or `principal`.
    pub kind: String,
    /// Why it is applicable: `principal_scope`, `selected_project`,
    /// `selected_workspace`, `grant`, `org_unit` or `tenant_root`.
    pub source: String,
    /// Whether a grant is written at this very scope rather than inherited
    /// from an ancestor — the "why" a member list would otherwise have to be
    /// read to answer.
    pub direct: bool,
    /// The role keys effective here.
    #[schema(value_type = Vec<String>)]
    pub roles: Vec<RoleKey>,
    /// Every operand-free scope action, decided here, by its stable machine
    /// name. **A forecast, never a grant** — the whole of this module's first
    /// doc section applies unchanged.
    #[schema(value_type = BTreeMap<String, bool>)]
    pub actions: BTreeMap<&'static str, bool>,
}

/// Decides [`Action::PROBED_AT_SCOPE`] at each of the caller's anchors.
///
/// One `gather` feeds all of them — the principal, the anchors and the groups
/// are properties of the *caller*, not of the scope — but each anchor is
/// decided under **its own chain and its own assignments**, because the
/// effective pack is a property of the resource (ADR-0014 decision 3) and a
/// workspace governed by a stricter profile than the tenant default must
/// answer under that profile.
///
/// Returns the answers and how many anchors the bound dropped.
pub(crate) async fn at_anchors(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    input: &DecisionInput,
) -> Result<(Vec<AnchorCapabilities>, usize)> {
    let tenant_id = input.principal.tenant_id;
    let all = input.anchors.as_slice();
    let answered = all.len().min(MAX_ANCHOR_CAPABILITIES);
    let not_answered = all.len() - answered;

    // Every anchor's chain, read once, so the entity batch covers them all.
    let mut chains: Vec<Vec<ScopeNode>> = Vec::with_capacity(answered);
    let mut assignments: Vec<Vec<PolicyAssignment>> = Vec::with_capacity(answered);
    for anchor in &all[..answered] {
        let Some(scope) = scopes::get(&mut *conn, tenant_id, anchor.scope_id).await? else {
            // Resolved a moment ago and gone now: plan nothing for it rather
            // than answer about a scope that no longer exists.
            chains.push(Vec::new());
            assignments.push(Vec::new());
            continue;
        };
        let mut nodes = vec![ScopeNode::from_scope(&scope, false)];
        for ancestor in scopes::ancestors(&mut *conn, tenant_id, scope.id).await? {
            nodes.push(ScopeNode::from_scope(&ancestor, false));
        }
        let ids: Vec<ScopeId> = nodes.iter().map(|node| node.id).collect();
        assignments.push(policy_assignments::for_scopes(&mut *conn, tenant_id, &ids).await?);
        chains.push(nodes);
    }

    let borrowed: Vec<&[ScopeNode]> = chains.iter().map(Vec::as_slice).collect();
    let seed = anchor_context(input, &[], &[]);
    let batch = state.pdp.materialise(&input.principal, &borrowed, &seed)?;

    let mut out = Vec::with_capacity(answered);
    for (index, anchor) in all[..answered].iter().enumerate() {
        if chains[index].is_empty() {
            continue;
        }
        let context = anchor_context(input, &chains[index], &assignments[index]);
        let resource = Resource::Scope(anchor.scope_id);
        let mut actions = BTreeMap::new();
        for action in Action::PROBED_AT_SCOPE {
            let decision =
                state
                    .pdp
                    .authorize_with(&batch, &input.principal, action, resource, &context)?;
            actions.insert(action.as_str(), decision.allowed);
        }
        out.push(AnchorCapabilities {
            scope_id: anchor.scope_id,
            kind: anchor.kind.as_str().to_owned(),
            source: anchor.source.as_str().to_owned(),
            direct: anchor.is_direct(),
            roles: effective_role_keys_at(resource, &context),
            actions,
        });
    }
    metrics::counter!(CAPABILITY_PROBES_TOTAL, "op" => "at_anchors", "outcome" => "ok")
        .increment(1);
    Ok((out, not_answered))
}

/// The decision context for one anchor: the caller's own principal, anchors
/// and groups, with the anchor's chain and assignments.
fn anchor_context<'a>(
    input: &'a DecisionInput,
    scopes: &'a [ScopeNode],
    assignments: &'a [PolicyAssignment],
) -> AuthzContext<'a> {
    AuthzContext {
        scopes,
        principal_scopes: &input.principal_scopes,
        anchors: input.anchors.as_slice(),
        groups: &input.groups,
        resources: &input.resources,
        assignments,
        default_pack: input.default_pack.as_deref(),
        relaxations: &[],
        sensitivity: None,
    }
}

/// `GET /v1/capabilities?scopes=<id>,<id>,…` — the plural of the same
/// walk, for the nodes a tree actually rendered.
#[utoipa::path(
    get,
    path = "/v1/capabilities",
    operation_id = "get_capabilities",
    tag = "capabilities",
    params(("scopes" = String, Query, description = "Comma-separated governed scope ids, at most 128 answered per request")),
    responses(
        (status = 200, description = "Forecasts for the caller at the requested scopes", body = BatchResponse),
        (status = 400, description = "No valid scope ids were supplied", body = crate::workspaces::ApiErrorBody),
        (status = 401, description = "No usable credential", body = crate::workspaces::ApiErrorBody),
        (status = 404, description = "A scope is absent or outside the tenant", body = crate::workspaces::ApiErrorBody),
    ),
    security(("bearer" = [])),
)]
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
    let mut nodes: Vec<Scope> = Vec::with_capacity(answered.len());
    for id in &answered {
        nodes.push(found(
            scopes::get(&mut *tx, tenant_id, *id).await?,
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
    // The first cut required `ScopeRead` and CNSL-2's own demo found
    // what that costs: under every shipped pack that action is
    // steward/org-admin/auditor only, so a **curator** — the role the
    // proposals inbox exists for — was refused the probe outright and shown
    // no verdict buttons at all. A capability surface that only privileged
    // readers may consult is worse than none: it hides acts from exactly
    // the readers who hold them.
    //
    // What `ScopeRead` *does* still decide is the **node detail**. The
    // verdicts are about the caller and always served; `scope_path` and the
    // effective pack are facts about the node, so a caller who may not read
    // the node does not receive them and the route cannot become a
    // node-metadata oracle for anyone holding a scope id.
    let mut answers = Vec::with_capacity(nodes.len());
    let mut pairs = 0usize;
    let mut allowed_pairs = 0usize;
    let mut gate = None;
    for node in &nodes {
        let input = authz::gather(
            state,
            &mut tx,
            Some(node),
            synveda_store::anchors::AnchorSelection::none(),
            Vec::new(),
        )
        .await?;
        let readable = authz::decide(state, &input, Action::ScopeRead, Resource::Scope(node.id));
        let may_read_node = readable.is_ok();
        // The path is the slug chain from the root — a display fact about
        // the node, priced only when the node is readable.
        let node_path = if may_read_node {
            Some(
                scopes::path(&mut *tx, tenant_id, node.id)
                    .await?
                    .unwrap_or_else(|| node.slug.clone()),
            )
        } else {
            None
        };
        let answer = answer_for(state, &input, node, node_path)?;
        pairs += answer.pair_count();
        allowed_pairs += answer.allowed_count();
        answers.push(answer);
        if let Ok(authorized) = readable {
            gate.get_or_insert(authorized);
        }
    }

    // One event for the whole fan-out (decision 4). The payload carries
    // counts and scope ids — never a third party's binding, and never a
    // relaxation reason, which is free text written about an incident.
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
        payload["authz"] = audit::decision_context(Action::ScopeRead, &authorized);
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
        self.node.actions.len() + self.node.read_tiers.len()
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
    node: &Scope,
    node_path: Option<String>,
) -> Result<Answer> {
    let resource = Resource::Scope(node.id);
    let context = input.context();
    let batch: EntityBatch = state
        .pdp
        .materialise(&input.principal, &[&input.chain], &context)?;

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

    Ok(Answer {
        node: NodeCapabilities {
            scope_id: node.id,
            scope_path: node_path.clone(),
            pack: node_path.is_some().then(|| PackView {
                name: tiers.effective.name.clone(),
                version: tiers.effective.version,
                origin: origin_view(&tiers.effective),
            }),
            roles: effective_role_keys_at(resource, &context),
            actions,
            read_tiers,
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

    /// The same pin, for `whoami?capabilities=true` (CPR-9).
    ///
    /// `synveda whoami --capabilities` parses this block with its own type,
    /// and read `{roles, actions, role_assign}` — CPR-7's vocabulary — until
    /// the foundation audit found the command could not parse a single
    /// response. Plain `synveda whoami` shares the route and never asks for
    /// the block, which is why the break stayed invisible.
    #[test]
    fn the_tenant_capability_block_is_the_shape_the_cli_parses() {
        let block = TenantCapabilities {
            role_keys: vec![RoleKey::Administrator],
            actions: BTreeMap::from([("workspace.read", true)]),
        };
        let rendered = serde_json::to_value(&block).expect("serialises");
        let mut keys: Vec<&str> = rendered
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["actions", "role_keys"],
            "the tenant capability block changed shape. `synveda whoami \
             --capabilities` parses it with its own type (crates/synveda-cli/\
             src/whoami.rs) — update that too."
        );
    }

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

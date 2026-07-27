//! CTX-2 (ADR-0025 decisions 1–3): the composition plan — the CTX-1
//! chain sweep extended with per-scope channel rules and the budget,
//! all resolved from effective packs through the PDP facade, never a
//! bypass (seed §2.2).
//!
//! Fixture chain: alice-user → team-a → eng → org.

use chrono::Utc;
use synveda_policy::{Pdp, Principal};
use synveda_retrieval::{MemoryReadInputs, composition_plan};
use synveda_types::{
    CompositionConfig, HierarchyNode, InjectChannels, PackConfig, PolicyAssignment, ScopeId,
    ScopeKind, Sensitivity, TenantId,
};

struct Fixture {
    tenant: TenantId,
    /// alice's placement chain, nearest-first.
    chain: Vec<HierarchyNode>,
}

fn node(
    tenant: TenantId,
    parent: Option<&HierarchyNode>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    HierarchyNode {
        id: ScopeId::new(),
        tenant_id: tenant,
        parent_id: parent.map(|parent| parent.id),
        kind,
        slug: slug.to_owned(),
        name: slug.to_owned(),
        depth: parent.map_or(0, |parent| parent.depth + 1),
        path: parent.map_or_else(
            || slug.to_owned(),
            |parent| format!("{}/{slug}", parent.path),
        ),
        created_at: Utc::now(),
    }
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    let org = node(tenant, None, ScopeKind::Org, "org");
    let eng = node(tenant, Some(&org), ScopeKind::Department, "eng");
    let team = node(tenant, Some(&eng), ScopeKind::Team, "team-a");
    let alice = node(tenant, Some(&team), ScopeKind::User, "alice-user");
    Fixture {
        tenant,
        chain: vec![alice, team, eng, org],
    }
}

fn principal(fixture: &Fixture, quarantined: bool) -> Principal {
    Principal {
        tenant_id: fixture.tenant,
        subject: "alice".to_owned(),
        quarantined,
        scope_id: Some(fixture.chain[0].id),
        token_scope: None,
    }
}

fn assignment(fixture: &Fixture, scope_id: ScopeId, pack: &str) -> PolicyAssignment {
    PolicyAssignment {
        tenant_id: fixture.tenant,
        scope_id,
        pack_name: pack.to_owned(),
        updated_at: Utc::now(),
    }
}

const BLANKET: &str = "permit (principal, action, resource) when { resource in principal.tenant };";

/// The zero-config path: the strict default pack plans the whole chain
/// nearest-first, both channels everywhere, the seed §4.4 budget.
#[test]
fn default_pack_plans_both_channels_at_the_default_budget() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    let alice = principal(&fixture, false);
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &alice,
            chain: &fixture.chain,
            assignments: &[],
            default_pack: None,
            role_bindings: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("plan");
    assert_eq!(plan.budget_tokens, CompositionConfig::DEFAULT.budget_tokens);
    let planned: Vec<ScopeId> = plan.scopes.iter().map(|scope| scope.scope_id).collect();
    let expected: Vec<ScopeId> = fixture.chain.iter().map(|node| node.id).collect();
    assert_eq!(planned, expected, "own chain, nearest-first");
    assert!(
        plan.scopes.iter().all(|scope| scope.include_derived),
        "the product default composes both channels"
    );
    assert_eq!(plan.scopes[0].kind, ScopeKind::User);
    assert_eq!(plan.scopes[0].path, fixture.chain[0].path);

    // Every planned scope carries the tiers the walk permitted there
    // (AUTHZ-5, ADR-0038 decision 3), and zero-config membership means the
    // working tiers — except at alice's own home, which reads its own
    // `confidential` material with no binding (decision 4).
    assert_eq!(
        plan.scopes[0].sensitivities,
        vec![
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential
        ],
        "own home reaches confidential"
    );
    for scope in &plan.scopes[1..] {
        assert_eq!(
            scope.sensitivities,
            vec![Sensitivity::Public, Sensitivity::Internal],
            "membership above home reads the working tiers"
        );
    }
    // The audit's half: the decisions carry the same sets, so one event
    // can answer "what could this reader see at that scope" (decision 13).
    assert!(
        plan.decisions
            .iter()
            .all(|decision| decision.allowed != decision.sensitivities.is_empty()),
        "an allow is exactly a non-empty tier set"
    );
    assert!(
        plan.decisions
            .iter()
            .all(|decision| !decision.sensitivities.contains(&Sensitivity::Restricted)),
        "and no walk reaches the top tier without a grant that declared it"
    );
}

/// The bank-mode switch (ADR-0025 decision 2): a stored pack with
/// `published-only` assigned at the team governs the team and — by
/// nearest-ancestor inheritance — alice's personal scope below it,
/// while the department and org above stay on both channels. The
/// budget resolves at alice's home scope, so her inject runs under the
/// pack's 900 (decision 3: "per-scope configurable" through the
/// existing assignment machinery).
#[test]
fn published_only_pack_governs_its_subtree_and_the_budget() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    pdp.install_source(
        fixture.tenant,
        "acme-bank",
        1,
        BLANKET,
        PackConfig {
            composition: Some(CompositionConfig {
                budget_tokens: 900,
                channels: InjectChannels::PublishedOnly,
                ..CompositionConfig::DEFAULT
            }),
            ..Default::default()
        },
    )
    .expect("install bank pack");
    let team = fixture.chain[1].id;
    let assignments = [assignment(&fixture, team, "acme-bank")];
    let alice = principal(&fixture, false);
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &alice,
            chain: &fixture.chain,
            assignments: &assignments,
            default_pack: None,
            role_bindings: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("plan");
    assert_eq!(plan.budget_tokens, 900, "the home scope's effective budget");
    let by_scope: Vec<(ScopeKind, bool)> = plan
        .scopes
        .iter()
        .map(|scope| (scope.kind, scope.include_derived))
        .collect();
    assert_eq!(
        by_scope,
        vec![
            (ScopeKind::User, false),
            (ScopeKind::Team, false),
            (ScopeKind::Department, true),
            (ScopeKind::Org, true),
        ],
        "published-only inside the assigned subtree, both channels above"
    );
}

/// An unconfigured stored pack composes under the product config — the
/// fallback narrows nothing (ADR-0025 decision 3).
#[test]
fn unconfigured_stored_pack_gets_the_product_config() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    pdp.install_source(
        fixture.tenant,
        "acme-plain",
        1,
        BLANKET,
        PackConfig::default(),
    )
    .expect("install unconfigured pack");
    let assignments = [assignment(&fixture, fixture.chain[3].id, "acme-plain")];
    let alice = principal(&fixture, false);
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &alice,
            chain: &fixture.chain,
            assignments: &assignments,
            default_pack: None,
            role_bindings: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("plan");
    assert_eq!(plan.budget_tokens, CompositionConfig::DEFAULT.budget_tokens);
    assert!(plan.scopes.iter().all(|scope| scope.include_derived));
}

/// The base layer reaches the plan like every seam: a quarantined
/// principal plans nothing (and the empty chain plans nothing under the
/// default budget).
#[test]
fn quarantine_and_empty_chain_plan_nothing() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    let quarantined = principal(&fixture, true);
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &quarantined,
            chain: &fixture.chain,
            assignments: &[],
            default_pack: None,
            role_bindings: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("plan");
    assert!(plan.scopes.is_empty(), "quarantine denies every scope");

    let unplaced = Principal {
        tenant_id: fixture.tenant,
        subject: "dev".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    let plan = composition_plan(
        &pdp,
        &MemoryReadInputs {
            principal: &unplaced,
            chain: &[],
            assignments: &[],
            default_pack: None,
            role_bindings: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("empty chain plans");
    assert!(plan.scopes.is_empty());
    assert_eq!(plan.budget_tokens, CompositionConfig::DEFAULT.budget_tokens);
}

//! CTX-1 (ADR-0024 decision 1): the authz-derived predicate. Every
//! scope entering the retrieval filter is a per-request PDP
//! `MemoryRead` allow over the caller's placement chain — decided
//! through the same facade production uses, never a bypass (seed §2.2).
//!
//! Fixture chain: alice-user → team-a → eng → org.

use synveda_policy::{Pdp, Principal};
use synveda_retrieval::{MemoryReadInputs, permitted_chain_scopes};
use synveda_types::scope::ScopeKind;
use synveda_types::{ScopeId, ScopeTier, Sensitivity, TenantId};

struct Fixture {
    tenant: TenantId,
    /// alice's placement chain, nearest-first.
    chain: Vec<synveda_policy::ScopeNode>,
}

fn node(
    tenant: TenantId,
    parent: Option<&synveda_policy::ScopeNode>,
    kind: ScopeKind,
    slug: &str,
) -> synveda_policy::ScopeNode {
    synveda_policy::ScopeNode {
        id: ScopeId::new(),
        tenant_id: tenant,
        parent_id: parent.map(|parent| parent.id),
        kind,
        slug: slug.to_owned(),
        sealed: false,
    }
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    let org = node(tenant, None, ScopeKind::Tenant, "org");
    let eng = node(tenant, Some(&org), ScopeKind::OrgUnit, "eng");
    let team = node(tenant, Some(&eng), ScopeKind::OrgUnit, "team-a");
    let alice = node(tenant, Some(&team), ScopeKind::Principal, "alice-user");
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

fn inputs<'a>(
    principal: &'a Principal,
    chain: &'a [synveda_policy::ScopeNode],
) -> MemoryReadInputs<'a> {
    MemoryReadInputs {
        principal,
        chain,
        assignments: &[],
        default_pack: None,
        anchors: &[],
        groups: &[],
        lapses: &[],
        lapsed: &[],
        candidates: &[],
    }
}

/// The distinct scopes a pair set names, in first-seen order — the sweep's
/// old return value, which several assertions here are still about.
fn scopes_of(pairs: &[ScopeTier]) -> Vec<ScopeId> {
    let mut scopes: Vec<ScopeId> = Vec::new();
    for pair in pairs {
        if !scopes.contains(&pair.scope_id) {
            scopes.push(pair.scope_id);
        }
    }
    scopes
}

/// The zero-config path (strict default pack): a placed user composes
/// its whole chain — own personal scope, team, department, org — in
/// chain order, nearest first (the CTX-2 gradient order).
#[test]
fn placed_user_composes_its_whole_chain_nearest_first() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    let alice = principal(&fixture, false);
    let permitted =
        permitted_chain_scopes(&pdp, &inputs(&alice, &fixture.chain)).expect("chain sweep decides");
    let expected: Vec<ScopeId> = fixture.chain.iter().map(|node| node.id).collect();
    assert_eq!(
        scopes_of(&permitted),
        expected,
        "own chain permits, order preserved"
    );
    // The tiers each scope came back with (AUTHZ-5, ADR-0038 decision 4):
    // membership is not an explicit grant, so `confidential` is reached
    // only at the reader's *own home* — the material extracted from their
    // own sessions. Everything above it stops at the working tiers, and
    // that difference along one chain is precisely why the predicate is a
    // pair rather than a ceiling.
    let home = fixture.chain[0].id;
    let tiers_at = |scope: ScopeId| -> Vec<Sensitivity> {
        permitted
            .iter()
            .filter(|pair| pair.scope_id == scope)
            .map(|pair| pair.sensitivity)
            .collect()
    };
    assert_eq!(
        tiers_at(home),
        vec![
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential
        ],
        "own home reaches confidential with no binding"
    );
    for node in &fixture.chain[1..] {
        assert_eq!(
            tiers_at(node.id),
            vec![Sensitivity::Public, Sensitivity::Internal],
            "membership above home reads the working tiers"
        );
    }
    assert!(
        permitted
            .iter()
            .all(|pair| pair.sensitivity < Sensitivity::Restricted),
        "and nothing on a zero-config chain reaches the top tier"
    );
}

/// The base layer's quarantine forbid reaches retrieval like every
/// other seam: a quarantined principal composes nothing.
#[test]
fn quarantined_principal_composes_nothing() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    let alice = principal(&fixture, true);
    let permitted =
        permitted_chain_scopes(&pdp, &inputs(&alice, &fixture.chain)).expect("chain sweep decides");
    assert!(permitted.is_empty(), "quarantine denies every scope");
}

/// An unplaced principal (dev HS256 subject that never provisioned a
/// placement) has an empty chain: the predicate is empty and the
/// engine's mandatory filter then returns nothing (ADR-0014 decision 5).
#[test]
fn unplaced_principal_has_an_empty_predicate() {
    let fixture = fixture();
    let pdp = Pdp::new().expect("embedded packs compile");
    let unplaced = Principal {
        tenant_id: fixture.tenant,
        subject: "dev".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    let permitted =
        permitted_chain_scopes(&pdp, &inputs(&unplaced, &[])).expect("empty chain sweep");
    assert!(permitted.is_empty());
}

/// A service identity's own-chain `MemoryRead` floor (AUTH-3, ADR-0018):
/// the confinement forbid's one carve-out — an agent anchored at team-a
/// composes up its whole chain, ancestors outside its anchor subtree
/// included.
#[test]
fn service_identity_composes_its_own_chain_through_confinement() {
    let tenant = TenantId::new();
    let org = node(tenant, None, ScopeKind::Tenant, "org");
    let eng = node(tenant, Some(&org), ScopeKind::OrgUnit, "eng");
    let team = node(tenant, Some(&eng), ScopeKind::OrgUnit, "team-a");
    let agent_leaf = node(tenant, Some(&team), ScopeKind::Principal, "agent-user");
    let chain = vec![agent_leaf, team, eng, org];
    let agent = Principal {
        tenant_id: tenant,
        subject: "svc-agent".to_owned(),
        quarantined: false,
        scope_id: Some(chain[0].id),
        // Anchored at team-a: the base layer forbids everything outside
        // its subtree except this floor (ADR-0018 decision 4).
        token_scope: Some(chain[1].id),
    };
    let pdp = Pdp::new().expect("embedded packs compile");
    let permitted = permitted_chain_scopes(
        &pdp,
        &MemoryReadInputs {
            principal: &agent,
            chain: &chain,
            assignments: &[],
            default_pack: None,
            anchors: &[],
            groups: &[],
            lapses: &[],
            lapsed: &[],
            candidates: &[],
        },
    )
    .expect("chain sweep decides");
    let expected: Vec<ScopeId> = chain.iter().map(|node| node.id).collect();
    assert_eq!(
        scopes_of(&permitted),
        expected,
        "the own-chain MemoryRead floor survives confinement"
    );
}

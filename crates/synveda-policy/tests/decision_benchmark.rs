//! AUTHZ-1 AC: µs-level decision benchmark (extended by AUTHZ-2).
//!
//! Measures the full facade call — entity materialisation from a
//! realistic 5-level scope chain plus a placement chain, effective-pack
//! resolution from an assignment (ADR-0014), and the Cedar evaluation —
//! since that is what every enforcement point pays per decision. Pure in-process CPU (no
//! I/O), so absolute asserts are meaningful across dev machines and CI;
//! the bound is set an order of magnitude above the expected cost to stay
//! insensitive to scheduler noise while still failing loudly if a
//! millisecond-scale regression (a network hop, a per-call re-parse) ever
//! sneaks in.

use std::time::Instant;

use chrono::Utc;
use synveda_policy::{Action, AuthzContext, Pdp, Principal, Resource, STANDARD};
use synveda_types::{HierarchyNode, PolicyAssignment, ScopeId, ScopeKind, TenantId};

const WARMUP: usize = 1_000;
const SAMPLES: usize = 10_000;
/// "µs-level": the median full-facade decision stays under a quarter of a
/// millisecond (measured reality is single-digit to low tens of µs).
const MEDIAN_BOUND_NANOS: u128 = 250_000;

fn node(
    tenant_id: TenantId,
    id: ScopeId,
    parent_id: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
    depth: i32,
    path: &str,
) -> HierarchyNode {
    HierarchyNode {
        id,
        tenant_id,
        parent_id,
        kind,
        slug: slug.to_owned(),
        name: slug.to_owned(),
        depth,
        path: path.to_owned(),
        created_at: Utc::now(),
    }
}

#[test]
fn ac_decisions_are_microsecond_level() {
    let pdp = Pdp::new().expect("build pdp");
    let tenant = TenantId::new();
    let (org, division, dept, team) = (
        ScopeId::new(),
        ScopeId::new(),
        ScopeId::new(),
        ScopeId::new(),
    );
    // A full-depth chain — deeper than most real tenants use (levels are
    // skippable, ADR-0011), so the entity graph is not flattered.
    let scopes = vec![
        node(tenant, org, None, ScopeKind::Org, "acme", 0, "acme"),
        node(
            tenant,
            division,
            Some(org),
            ScopeKind::Division,
            "emea",
            1,
            "acme/emea",
        ),
        node(
            tenant,
            dept,
            Some(division),
            ScopeKind::Department,
            "payments",
            2,
            "acme/emea/payments",
        ),
        node(
            tenant,
            team,
            Some(dept),
            ScopeKind::Team,
            "core",
            3,
            "acme/emea/payments/core",
        ),
    ];
    // The principal is a placed identity (AUTH-2): its personal scope
    // under the team, its chain materialised alongside the resource's —
    // the shape every governed request pays after AUTHZ-2.
    let user = ScopeId::new();
    let mut principal_scopes = vec![node(
        tenant,
        user,
        Some(team),
        ScopeKind::User,
        "bench",
        4,
        "acme/emea/payments/core/bench",
    )];
    principal_scopes.extend(scopes.iter().cloned());
    let principal = Principal {
        tenant_id: tenant,
        subject: "bench".to_owned(),
        quarantined: false,
        scope_id: Some(user),
    };
    // One assignment at the org: resolution walks the full chain to find
    // it (ADR-0014 decision 3) — the effective-pack cost is measured, not
    // skipped.
    let assignments = [PolicyAssignment {
        tenant_id: tenant,
        scope_id: org,
        pack_name: STANDARD.to_owned(),
        updated_at: Utc::now(),
    }];
    let context = AuthzContext {
        scopes: &scopes,
        principal_scopes: &principal_scopes,
        assignments: &assignments,
        default_pack: None,
    };

    let call = |action: Action| {
        let decision = pdp
            .authorize(&principal, action, Resource::Scope(team), &context)
            .expect("authorize");
        assert!(decision.allowed, "benchmark decisions must be allows");
        decision
    };

    for _ in 0..WARMUP {
        call(Action::HierarchyRead);
    }

    let mut samples: Vec<u128> = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Rotate actions so no single decision path is cached unrealistically.
        let action = match i % 5 {
            0 => Action::HierarchyRead,
            1 => Action::HierarchyCreate,
            2 => Action::HierarchyUpdate,
            3 => Action::MemoryRead,
            _ => Action::HierarchyDelete,
        };
        let start = Instant::now();
        call(action);
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let median = samples[SAMPLES / 2];
    let p99 = samples[SAMPLES * 99 / 100];
    eprintln!(
        "authorize (facade incl. entity materialisation, 4-level chain): \
         median {}µs, p99 {}µs over {SAMPLES} calls",
        median / 1_000,
        p99 / 1_000,
    );
    assert!(
        median < MEDIAN_BOUND_NANOS,
        "median decision took {median}ns; the µs-level AC bound is {MEDIAN_BOUND_NANOS}ns"
    );
}

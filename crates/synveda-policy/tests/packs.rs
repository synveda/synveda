//! AUTHZ-2 golden tests per pack (the AC): for a fixed two-department
//! fixture, the full `MemoryRead` decision matrix of each embedded product
//! pack, the `MemoryWrite` floor (MEM-1, ADR-0020 decision 3 — pack
//! uniform: own home only, role-free; the content-role write grant lives
//! in tests/roles.rs), the shared admin-plane semantics, and the
//! cross-cutting invariants (quarantine, unplaced principals, foreign
//! tenants). Packs are applied through the same assignment-resolution
//! path production uses — never a PDP bypass (CLAUDE.md, seed §2.2).
//!
//! The fixture:
//!
//! ```text
//! org
//! ├── eng (department)
//! │   ├── team-a ── alice-user   ← alice's placement
//! │   └── team-b ── carol-user
//! └── sales (department)
//!     └── team-c ── dave-user
//! ```

use chrono::Utc;
use synveda_policy::{
    Action, AuthzContext, OPEN_COLLABORATION, Pdp, Principal, REGULATED_STRICT, Resource, STANDARD,
    ScopeNode,
};
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::ScopeKind;
use synveda_types::{PackConfig, PolicyAssignment, ScopeId, Sensitivity, TenantId};

/// Every scope of the fixture — the candidate set a composition sweep
/// would consider.
const ALL_SCOPES: [&str; 8] = [
    "org",
    "eng",
    "sales",
    "team-a",
    "team-b",
    "team-c",
    "alice-user",
    "carol-user",
];

struct Fixture {
    tenant: TenantId,
    nodes: Vec<ScopeNode>,
}

impl Fixture {
    fn node(&self, slug: &str) -> &ScopeNode {
        self.nodes
            .iter()
            .find(|node| node.slug == slug)
            .unwrap_or_else(|| panic!("fixture has no node {slug}"))
    }

    /// The node and its ancestors — what a caller reads for the resource
    /// (and what the gateway reads for a principal's placement).
    /// The chain the PDP takes: the old hierarchy's rows projected onto
    /// the shape vocabulary at the caller's edge (CPR-6, ADR-0073
    /// decision 1). The fixture still holds `HierarchyNode`s because the
    /// hierarchy plane still exists; nothing below this line does.
    fn chain(&self, slug: &str) -> Vec<ScopeNode> {
        let mut chain = vec![self.node(slug).clone()];
        let mut current = chain[0].parent_id;
        while let Some(id) = current {
            let parent = self
                .nodes
                .iter()
                .find(|node| node.id == id)
                .expect("parent exists");
            current = parent.parent_id;
            chain.push(parent.clone());
        }
        chain.clone()
    }

    fn assignment(&self, slug: &str, pack: &str) -> PolicyAssignment {
        PolicyAssignment {
            tenant_id: self.tenant,
            scope_id: self.node(slug).id,
            pack_name: pack.to_owned(),
            updated_at: Utc::now(),
        }
    }

    /// A principal placed at `slug`'s personal scope.
    fn placed(&self, subject: &str, slug: &str) -> Principal {
        Principal {
            tenant_id: self.tenant,
            subject: subject.to_owned(),
            quarantined: false,
            scope_id: Some(self.node(slug).id),
            token_scope: None,
        }
    }
}

fn fixture() -> Fixture {
    let tenant = TenantId::new();
    let mut nodes = Vec::new();
    let mut add = |parent: Option<ScopeId>, kind: ScopeKind, slug: &str, _depth: i32| -> ScopeId {
        let id = ScopeId::new();
        nodes.push(ScopeNode {
            id,
            tenant_id: tenant,
            parent_id: parent,
            kind,
            slug: slug.to_owned(),
            sealed: false,
        });
        id
    };
    let org = add(None, ScopeKind::Tenant, "org", 0);
    let eng = add(Some(org), ScopeKind::OrgUnit, "eng", 1);
    let sales = add(Some(org), ScopeKind::OrgUnit, "sales", 1);
    let team_a = add(Some(eng), ScopeKind::OrgUnit, "team-a", 2);
    let team_b = add(Some(eng), ScopeKind::OrgUnit, "team-b", 2);
    let team_c = add(Some(sales), ScopeKind::OrgUnit, "team-c", 2);
    add(Some(team_a), ScopeKind::Principal, "alice-user", 3);
    add(Some(team_b), ScopeKind::Principal, "carol-user", 3);
    add(Some(team_c), ScopeKind::Principal, "dave-user", 3);
    Fixture { tenant, nodes }
}

/// One memory-plane decision for `target`, with the principal's placement
/// chain materialised from `placement` and packs applied via
/// `assignments` — the exact shape the gateway feeds the facade.
fn memory(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    action: Action,
    placement: Option<&str>,
    target: &str,
    assignments: &[PolicyAssignment],
) -> synveda_policy::AuthzDecision {
    let scopes = fx.chain(target);
    let principal_scopes = placement.map(|slug| fx.chain(slug)).unwrap_or_default();
    pdp.authorize(
        principal,
        action,
        Resource::Scope(fx.node(target).id),
        &AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            assignments,
            ..Default::default()
        },
    )
    .expect("authorize")
}

fn read(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    target: &str,
    assignments: &[PolicyAssignment],
) -> synveda_policy::AuthzDecision {
    memory(
        pdp,
        fx,
        principal,
        Action::MemoryRead,
        placement,
        target,
        assignments,
    )
}

/// The composition sweep: which of the fixture's scopes may this
/// principal's inject composition include? (What CTX-2 will ask, ADR-0014
/// decision 5.)
fn composition(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    assignments: &[PolicyAssignment],
) -> Vec<&'static str> {
    ALL_SCOPES
        .into_iter()
        .filter(|target| read(pdp, fx, principal, placement, target, assignments).allowed)
        .collect()
}

/// The write sweep: at which of the fixture's scopes may this principal
/// land memory content? (What observe asks for home, and FLOW will ask
/// beyond it — MEM-1, ADR-0020 decision 3.)
fn write_targets(
    pdp: &Pdp,
    fx: &Fixture,
    principal: &Principal,
    placement: Option<&str>,
    assignments: &[PolicyAssignment],
) -> Vec<&'static str> {
    ALL_SCOPES
        .into_iter()
        .filter(|target| {
            memory(
                pdp,
                fx,
                principal,
                Action::MemoryWrite,
                placement,
                target,
                assignments,
            )
            .allowed
        })
        .collect()
}

/// Asserts one pack's golden `MemoryRead` matrix for alice, plus the
/// invariants every pack shares: version stamping, admin-plane semantics,
/// unplaced/quarantined/foreign denial.
fn assert_pack_golden(pack: &str, version: i64, expected_for_alice: &[&str]) {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    // Applied per node: the org root carries the assignment, the whole
    // tenant inherits it (ADR-0014 decision 4).
    let assignments = [fx.assignment("org", pack)];

    // The golden matrix, target by target — a wrong allow and a wrong
    // deny both fail, and the failure names the pack and target.
    for target in ALL_SCOPES {
        let decision = read(&pdp, &fx, &alice, Some("alice-user"), target, &assignments);
        assert_eq!(
            decision.allowed,
            expected_for_alice.contains(&target),
            "{pack}: alice reading {target} decided {}",
            decision.allowed,
        );
        assert_eq!(decision.pack_name, pack, "{pack}: wrong pack for {target}");
        assert_eq!(decision.pack_version, version);
    }

    // The prompt registry reads exactly where memory does (PRMT-1,
    // ADR-0049 decision 4). Asserted rather than stated: each pack's
    // PromptRead permits are a transcription of its own MemoryRead permits,
    // and a transcription is the kind of thing that drifts silently — a
    // department clause copied into two packs and forgotten in the third
    // would leave a consumer resolving prompts their pack does not share.
    for target in ALL_SCOPES {
        let decision = memory(
            &pdp,
            &fx,
            &alice,
            Action::PromptRead,
            Some("alice-user"),
            target,
            &assignments,
        );
        assert_eq!(
            decision.allowed,
            expected_for_alice.contains(&target),
            "{pack}: alice reading prompts at {target} decided {} — the prompt \
             plane must mirror this pack's memory plane, tier for tier",
            decision.allowed,
        );
    }

    // And the context-pack and skill planes beside it (PRMT-2, ADR-0050
    // decision 7; SKIL-1, ADR-0051 decision 10), asserted for the same
    // reason a third transcription is a third place to drift. What this does
    // *not* say is that they admit the same material: `ContextPackRead`
    // admits pack chunks, `MemoryRead` never does (ADR-0050 decision 8), and
    // `SkillRead` admits neither — a skill composes into no block at all
    // (ADR-0051 decision 9). The claim here is only that the same people are
    // trusted at the same scopes.
    for (action, what) in [
        (Action::ContextPackRead, "context packs"),
        (Action::SkillRead, "skills"),
    ] {
        for target in ALL_SCOPES {
            let decision = memory(
                &pdp,
                &fx,
                &alice,
                action,
                Some("alice-user"),
                target,
                &assignments,
            );
            assert_eq!(
                decision.allowed,
                expected_for_alice.contains(&target),
                "{pack}: alice reading {what} at {target} decided {} — the \
                 {what} plane must mirror this pack's memory plane, tier for tier",
                decision.allowed,
            );
        }
    }

    // Authoring mirrors the write floor, which is pack-uniform: own home
    // and nowhere else without a content role. All three authored asset
    // types.
    for (action, what) in [
        (Action::PromptWrite, "prompts"),
        (Action::ContextPackWrite, "context packs"),
        (Action::SkillWrite, "skills"),
    ] {
        let authorable: Vec<&str> = ALL_SCOPES
            .into_iter()
            .filter(|target| {
                memory(
                    &pdp,
                    &fx,
                    &alice,
                    action,
                    Some("alice-user"),
                    target,
                    &assignments,
                )
                .allowed
            })
            .collect();
        assert_eq!(
            authorable,
            vec!["alice-user"],
            "{pack}: an unbound principal authors {what} at its own home and \
             nowhere else"
        );
    }

    // Since AUTHZ-3 the admin planes require roles (ADR-0015 decision 4):
    // an unbound principal — placed or not — holds no administrative
    // power under any pack. The role×action matrix lives in
    // tests/roles.rs; here we pin the unbound baseline.
    let unplaced = Principal {
        tenant_id: fx.tenant,
        subject: "dev-token".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    for action in [
        Action::ScopeCreate,
        Action::ScopeRead,
        Action::ScopeUpdate,
        Action::PolicyRead,
        Action::PolicyAssign,
    ] {
        for principal in [&alice, &unplaced] {
            let scopes = fx.chain("team-b");
            let decision = pdp
                .authorize(
                    principal,
                    action,
                    Resource::Scope(fx.node("team-b").id),
                    &AuthzContext {
                        sensitivity: Some(Sensitivity::Internal),
                        scopes: &scopes,
                        assignments: &assignments,
                        ..Default::default()
                    },
                )
                .expect("authorize");
            assert!(
                !decision.allowed,
                "{pack}: {action} must be denied to the unbound {}",
                principal.subject
            );
        }
    }

    // The write floor is pack-uniform (MEM-1, ADR-0020 decision 3): a
    // role-free principal writes exactly its own personal scope, under
    // every pack — openness is a read-side property. The content-role
    // write grant is golden-tested in tests/roles.rs.
    assert_eq!(
        write_targets(&pdp, &fx, &alice, Some("alice-user"), &assignments),
        vec!["alice-user"],
        "{pack}: the role-free write floor is home, and home only"
    );

    // An unplaced principal composes nothing, under every pack — and has
    // no home, so it writes nothing either.
    assert_eq!(
        composition(&pdp, &fx, &unplaced, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: an unplaced principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &unplaced, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: an unplaced principal must write nothing"
    );

    // A quarantined principal is forbidden everything, admin included.
    let quarantined = Principal {
        quarantined: true,
        ..alice.clone()
    };
    assert_eq!(
        composition(&pdp, &fx, &quarantined, Some("alice-user"), &assignments),
        Vec::<&str>::new(),
        "{pack}: a quarantined principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &quarantined, Some("alice-user"), &assignments),
        Vec::<&str>::new(),
        "{pack}: a quarantined principal must write nothing"
    );

    // A foreign principal is denied everything, placed or not.
    let intruder = Principal {
        tenant_id: TenantId::new(),
        subject: "intruder".to_owned(),
        quarantined: false,
        scope_id: None,
        token_scope: None,
    };
    assert_eq!(
        composition(&pdp, &fx, &intruder, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: a foreign principal must read nothing"
    );
    assert_eq!(
        write_targets(&pdp, &fx, &intruder, None, &assignments),
        Vec::<&str>::new(),
        "{pack}: a foreign principal must write nothing"
    );
}

/// regulated-strict: own chain only — no cross-team read exists at all
/// (seed §6; lapses, AUTHZ-4, are the sanctioned relaxation).
#[test]
fn golden_regulated_strict() {
    assert_pack_golden(
        REGULATED_STRICT,
        20,
        &["org", "eng", "team-a", "alice-user"],
    );
}

/// standard: own chain, **plus the subtree of everything this caller holds**
/// (CPR-6, ADR-0073). A principal with no grant holds nothing, so this matrix
/// is now identical to `regulated-strict`'s — which is not a regression but
/// the removal of the rank: what used to widen it was
/// `principal.department`, the nearest department-kind ancestor of a
/// placement, and there is no such thing any more. What widens it now is a
/// grant, which `standard_shares_within_what_you_hold` asserts.
#[test]
fn golden_standard() {
    assert_pack_golden(STANDARD, 20, &["org", "eng", "team-a", "alice-user"]);
}

/// open-collaboration: org-wide — only other people's personal scopes
/// stay out (the privacy floor, which AUTHZ-5 left exactly where it was:
/// a tier says how sensitive material is, not whose it is).
#[test]
fn golden_open_collaboration() {
    assert_pack_golden(
        OPEN_COLLABORATION,
        20,
        &[
            "org",
            "eng",
            "sales",
            "team-a",
            "team-b",
            "team-c",
            "alice-user",
        ],
    );
}

/// standard's sharing default, re-cut: **the subtree of what you hold**, not
/// the subtree of where an org chart put you (CPR-6, ADR-0073).
///
/// The old version of this test asserted that dave (sales) gained nothing in
/// eng and that carol (eng) gained her sibling team — both facts about
/// `principal.department`. The property that replaces it says what a reader
/// now has to be told: sharing follows a **grant's neighbourhood**, so carol
/// with a grant at her own team reads the teams beside it, and carol with no
/// grant reads exactly her own chain.
#[test]
fn standard_shares_within_what_you_hold() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let assignments = [fx.assignment("org", STANDARD)];
    let carol = fx.placed("carol", "carol-user");

    // No grant: own chain only.
    assert_eq!(
        composition(&pdp, &fx, &carol, Some("carol-user"), &assignments),
        vec!["org", "eng", "team-b", "carol-user"],
        "with nothing held, standard is regulated-strict's read surface"
    );

    // A grant at carol's own team: its **parent** — eng — is her
    // neighbourhood, so eng's whole subtree joins and sales does not.
    let team_b = fx.node("team-b");
    let anchor = ScopeAnchor {
        scope_id: team_b.id,
        kind: team_b.kind,
        parent_scope_id: team_b.parent_id,
        depth: 0,
        source: AnchorSource::Grant,
        roles: vec![synveda_types::access::RoleKey::Member],
        granted_at: vec![team_b.id],
        via_groups: Vec::new(),
    };
    let anchors = [anchor];
    let held: Vec<&'static str> = ALL_SCOPES
        .into_iter()
        .filter(|target| {
            let scopes = fx.chain(target);
            let principal_scopes = fx.chain("carol-user");
            pdp.authorize(
                &carol,
                Action::MemoryRead,
                Resource::Scope(fx.node(target).id),
                &AuthzContext {
                    sensitivity: Some(Sensitivity::Internal),
                    scopes: &scopes,
                    principal_scopes: &principal_scopes,
                    anchors: &anchors,
                    assignments: &assignments,
                    ..Default::default()
                },
            )
            .expect("authorize")
            .allowed
        })
        .collect();
    assert_eq!(
        held,
        vec!["org", "eng", "team-a", "team-b", "carol-user"],
        "a grant at team-b shares its neighbourhood, and nothing in sales"
    );
}

/// The AC, at the seam: switching one team's pack changes what a
/// principal's composition may include — immediately, because assignment
/// is request-time data (ADR-0014 decision 3), and only for that team's
/// subtree.
#[test]
fn switching_a_teams_pack_changes_the_composition_set() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");

    // Nothing assigned: the tenant runs regulated-strict; alice composes
    // her own chain only.
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &[]),
        vec!["org", "eng", "team-a", "alice-user"]
    );

    // team-b switches to open-collaboration. The effective pack is the
    // resource's: team-b now admits org-wide readers, while every other
    // scope keeps deciding under regulated-strict.
    let switched = [fx.assignment("team-b", OPEN_COLLABORATION)];
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &switched),
        vec!["org", "eng", "team-a", "team-b", "alice-user"],
        "alice's next composition includes team-b and nothing else new"
    );

    // carol's personal scope under team-b keeps its privacy floor even
    // under the team's open pack.
    let carol_scope = read(
        &pdp,
        &fx,
        &alice,
        Some("alice-user"),
        "carol-user",
        &switched,
    );
    assert!(
        !carol_scope.allowed,
        "personal scopes stay excluded under open-collaboration"
    );

    // Switching back restores the strict composition.
    assert_eq!(
        composition(&pdp, &fx, &alice, Some("alice-user"), &[]),
        vec!["org", "eng", "team-a", "alice-user"]
    );
}

/// MEM-2 (ADR-0021 decision 3): the redaction config rides the effective
/// pack — embedded defaults per product pack, a stored pack's explicit
/// config, and the fail-safe strict fallback for an unconfigured stored
/// pack — through the same resolution every decision uses.
#[test]
fn redaction_config_rides_the_effective_pack() {
    use synveda_types::{RedactionConfig, RedactionMode};

    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let team = fx.node("team-a").id;
    let scopes = fx.chain("team-a");

    for (pack, expected) in [
        (REGULATED_STRICT, RedactionConfig::STRICT),
        (STANDARD, RedactionConfig::REDACT_ALL),
        (OPEN_COLLABORATION, RedactionConfig::REDACT_ALL),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = pdp.effective(
            fx.tenant,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        );
        assert_eq!(effective.name, pack);
        assert_eq!(effective.redaction, expected, "{pack}");
    }

    // Nothing assigned anywhere: the embedded default is strict.
    let unassigned = pdp.effective(
        fx.tenant,
        Resource::Scope(team),
        &AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            scopes: &scopes,
            ..Default::default()
        },
    );
    assert_eq!(unassigned.name, REGULATED_STRICT);
    assert_eq!(unassigned.redaction, RedactionConfig::STRICT);

    // A stored pack carries its explicit config; one stored without a
    // config falls back to strict (fail safe).
    const MEMBER_READ: &str = r#"permit (principal, action == Synveda::Action::"MemoryRead", resource)
           when { principal in resource };"#;
    let deny_secrets = RedactionConfig {
        secrets: RedactionMode::Deny,
        pii: RedactionMode::Redact,
    };
    pdp.install_source(
        fx.tenant,
        "acme-deny",
        1,
        MEMBER_READ,
        PackConfig {
            redaction: Some(deny_secrets),
            ..Default::default()
        },
    )
    .expect("install configured pack");
    pdp.install_source(
        fx.tenant,
        "acme-unconfigured",
        1,
        MEMBER_READ,
        PackConfig::default(),
    )
    .expect("install unconfigured pack");
    for (pack, expected) in [
        ("acme-deny", deny_secrets),
        ("acme-unconfigured", RedactionConfig::STRICT),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = pdp.effective(
            fx.tenant,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments: &assignments,
                ..Default::default()
            },
        );
        assert_eq!(effective.name, pack);
        assert_eq!(effective.redaction, expected, "{pack}");
    }
}

/// The skill-quality bar rides the effective pack, and its fail-safe is
/// the **opposite** of every other config on this table (SKIL-3, ADR-0053
/// decision 9).
///
/// This test exists for that inversion and not for the plumbing. Every
/// config since ADR-0025 falls back to the strict reading, because the
/// thing it governs is one a pack that said nothing must still not be able
/// to weaken. Quality is not one of those: there is no floor, a pack that
/// has said nothing has not asked for a gate, and a product that started
/// refusing publications on a rubric nobody opted into would break every
/// tenant on an upgrade.
#[test]
fn the_quality_bar_rides_the_pack_and_an_unconfigured_pack_gates_nothing() {
    use synveda_types::SkillQualityConfig;

    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let team = fx.node("team-a").id;
    let scopes = fx.chain("team-a");

    let effective_under = |assignments: &[synveda_types::PolicyAssignment]| {
        pdp.effective(
            fx.tenant,
            Resource::Scope(team),
            &AuthzContext {
                sensitivity: Some(Sensitivity::Internal),
                scopes: &scopes,
                assignments,
                ..Default::default()
            },
        )
    };

    // The product packs: a real bar and a mandatory checklist for a bank,
    // a bar and no mandatory checklist for an SMB, nothing for an open
    // tenant.
    for (pack, expected) in [
        (REGULATED_STRICT, SkillQualityConfig::STRICT),
        (STANDARD, SkillQualityConfig::MODERATE),
        (OPEN_COLLABORATION, SkillQualityConfig::OPEN),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = effective_under(&assignments);
        assert_eq!(effective.name, pack);
        assert_eq!(effective.quality, expected, "{pack}");
    }

    // Nothing assigned anywhere: the embedded default is the strict pack,
    // so the bar is the strict one — a tenant that has configured nothing
    // still gets the product's opinion, because it is running the
    // product's pack.
    let unassigned = pdp.effective(
        fx.tenant,
        Resource::Scope(team),
        &AuthzContext {
            sensitivity: Some(Sensitivity::Internal),
            scopes: &scopes,
            ..Default::default()
        },
    );
    assert_eq!(unassigned.name, REGULATED_STRICT);
    assert_eq!(unassigned.quality, SkillQualityConfig::STRICT);

    // **The inversion.** A stored pack that configures nothing gates
    // nothing — where the same pack gets `RedactionConfig::STRICT` on the
    // line above, because a secret leaking is a harm and a low-scoring
    // skill is an opinion.
    const MEMBER_READ: &str = r#"permit (principal, action == Synveda::Action::"MemoryRead", resource)
           when { principal in resource };"#;
    let demanding = SkillQualityConfig {
        min_score: 90,
        require_checklist: true,
    };
    pdp.install_source(
        fx.tenant,
        "acme-demanding",
        1,
        MEMBER_READ,
        PackConfig {
            quality: Some(demanding),
            ..Default::default()
        },
    )
    .expect("install configured pack");
    pdp.install_source(
        fx.tenant,
        "acme-quiet",
        1,
        MEMBER_READ,
        PackConfig::default(),
    )
    .expect("install unconfigured pack");

    for (pack, expected) in [
        ("acme-demanding", demanding),
        ("acme-quiet", SkillQualityConfig::OPEN),
    ] {
        let assignments = [fx.assignment("org", pack)];
        let effective = effective_under(&assignments);
        assert_eq!(effective.name, pack);
        assert_eq!(effective.quality, expected, "{pack}");
    }

    // And the security scan is untouched by any of it: a pack may be
    // cheaper about quality and is never cheaper about the critical band
    // (ADR-0052 decision 3). Asserted here because these two configs sit
    // side by side on the same struct and are the two a reader is most
    // likely to conflate.
    let assignments = [fx.assignment("org", "acme-quiet")];
    assert_eq!(
        effective_under(&assignments).scan,
        synveda_types::SkillScanConfig::FLOOR
    );
}

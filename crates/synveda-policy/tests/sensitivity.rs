//! AUTHZ-5: sensitivity as a policy attribute (ADR-0038).
//!
//! The seam decides once per scope with no record in hand — the constraint
//! ADR-0037 decision 6 refused to paper over — and this is the answer: the
//! tier vocabulary is closed and ordered, so the same seam can be asked
//! about each of four tiers before any record is fetched (decision 1).
//!
//! Every case runs through the ordinary `authorize` facade with
//! caller-supplied rows, the shape the gateway feeds it; there is no bypass
//! here and no direct policy evaluation (CLAUDE.md, seed §2.2).
//!
//! The fixture is `packs.rs`'s because the
//! question is what the *same* golden decisions do once a tier is named:
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
use synveda_types::access::RoleKey;
use synveda_types::anchor::{AnchorSource, ScopeAnchor};
use synveda_types::scope::ScopeKind;
use synveda_types::{PolicyAssignment, ScopeId, Sensitivity, TenantId};

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

    /// The chain the PDP takes — governed scopes, nearest first (CPR-7,
    /// ADR-0074: one tree, no projection).
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
        chain
    }

    fn assignment(&self, slug: &str, pack: &str) -> PolicyAssignment {
        PolicyAssignment {
            tenant_id: self.tenant,
            scope_id: self.node(slug).id,
            pack_name: pack.to_owned(),
            updated_at: Utc::now(),
        }
    }

    /// A grant standing over this caller at `slug`, carrying `roles`
    /// (CPR-6; since the cutover the only way a test can give anybody a
    /// role — CPR-7, ADR-0074).
    fn anchor(&self, slug: &str, roles: &[RoleKey]) -> ScopeAnchor {
        let node = self.node(slug);
        ScopeAnchor {
            scope_id: node.id,
            kind: node.kind,
            parent_scope_id: node.parent_id,
            depth: 0,
            source: AnchorSource::Grant,
            roles: roles.to_vec(),
            granted_at: vec![node.id],
            via_groups: Vec::new(),
        }
    }

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

/// Everything one decision needs, so a test names only what it varies.
struct Ask<'a> {
    principal: &'a Principal,
    placement: &'a str,
    target: &'a str,
    sensitivity: Sensitivity,
    assignments: &'a [PolicyAssignment],
    /// The governed-scope grants standing over this caller (CPR-6, ADR-0073).
    /// Empty for every ask about the old hierarchy; `standard`'s sharing
    /// default reads them, because that is what it shares by.
    anchors: &'a [ScopeAnchor],
}

fn read(pdp: &Pdp, fx: &Fixture, ask: &Ask<'_>) -> bool {
    let scopes = fx.chain(ask.target);
    let principal_scopes = fx.chain(ask.placement);
    pdp.authorize(
        ask.principal,
        Action::KnowledgeRead,
        Resource::Scope(fx.node(ask.target).id),
        &AuthzContext {
            scopes: &scopes,
            principal_scopes: &principal_scopes,
            anchors: ask.anchors,
            assignments: ask.assignments,
            relaxations: &[],
            sensitivity: Some(ask.sensitivity),
            ..Default::default()
        },
    )
    .expect("authorize")
    .allowed
}

/// The read path's own shape (ADR-0038 decision 3): ask per tier, keep the
/// answers as a *set*. Ascending, so it reads as a ceiling when it is one.
fn tiers(pdp: &Pdp, fx: &Fixture, ask: &Ask<'_>) -> Vec<Sensitivity> {
    Sensitivity::ALL
        .into_iter()
        .filter(|tier| {
            read(
                pdp,
                fx,
                &Ask {
                    sensitivity: *tier,
                    ..*ask
                },
            )
        })
        .collect()
}

fn ask<'a>(
    principal: &'a Principal,
    placement: &'a str,
    target: &'a str,
    assignments: &'a [PolicyAssignment],
) -> Ask<'a> {
    Ask {
        principal,
        placement,
        target,
        sensitivity: Sensitivity::Internal,
        assignments,
        anchors: &[],
    }
}

/// Zero-config membership reads the working tiers, and belonging to a scope
/// is not a grant: `confidential` is defined as the tier held to explicitly
/// granted scopes, so alice's own team stops below it (decision 4).
#[test]
fn membership_reads_the_working_tiers_and_stops_below_confidential() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", REGULATED_STRICT)];

    for target in ["team-a", "eng", "org"] {
        assert_eq!(
            tiers(&pdp, &fx, &ask(&alice, "alice-user", target, &assignments)),
            [Sensitivity::Public, Sensitivity::Internal],
            "membership at {target} reads the working tiers only"
        );
    }
}

/// The explicit grant is what reaches `confidential` — a content-role
/// binding, which is the mechanism ADR-0015 already called "the seed's
/// explicit grant".
#[test]
fn an_explicit_content_role_binding_is_what_reaches_confidential() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", REGULATED_STRICT)];
    let bound = [fx.anchor("team-b", &[RoleKey::Member])];

    // Without the binding, team-b is closed at every tier.
    assert!(
        tiers(
            &pdp,
            &fx,
            &ask(&alice, "alice-user", "team-b", &assignments)
        )
        .is_empty(),
        "regulated-strict has no cross-team read at any tier"
    );

    assert_eq!(
        tiers(
            &pdp,
            &fx,
            &Ask {
                anchors: &bound,
                ..ask(&alice, "alice-user", "team-b", &assignments)
            }
        ),
        [
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential
        ],
        "a content-role binding is the explicit grant, and it reaches confidential"
    );
}

/// Material extracted from your own sessions, at your own personal scope.
/// Without this, a model's tier proposal could make an author's own memory
/// invisible to them — an accident, not a control (decision 4).
#[test]
fn your_own_home_reaches_confidential_with_no_binding_at_all() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");

    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let assignments = [fx.assignment("org", pack)];
        assert_eq!(
            tiers(
                &pdp,
                &fx,
                &ask(&alice, "alice-user", "alice-user", &assignments)
            ),
            [
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential
            ],
            "{pack}: your own home reaches confidential"
        );
        // And someone else's home reaches nothing, at any tier: the privacy
        // floor is about whose material it is, which no tier can answer.
        assert!(
            tiers(
                &pdp,
                &fx,
                &ask(&alice, "alice-user", "carol-user", &assignments)
            )
            .is_empty(),
            "{pack}: a tier never opens another principal's personal scope"
        );
    }
}

/// **The acceptance criterion at this seam.** No pack, no scope, no
/// membership, and no role reaches `restricted` — including at the reader's
/// own home, where the material may well be their own (decision 7).
#[test]
fn restricted_is_denied_under_every_pack_at_every_scope_including_your_own() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let every_role = [fx.anchor(
        "org",
        &[RoleKey::Curator, RoleKey::Reviewer, RoleKey::Administrator],
    )];

    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let assignments = [fx.assignment("org", pack)];
        for target in ["alice-user", "team-a", "team-b", "eng", "sales", "org"] {
            assert!(
                !read(
                    &pdp,
                    &fx,
                    &Ask {
                        sensitivity: Sensitivity::Restricted,
                        anchors: &every_role,
                        ..ask(&alice, "alice-user", target, &assignments)
                    }
                ),
                "{pack}: restricted material at {target} reached a reader holding every role"
            );
        }
    }
}

/// Seed §6's own sentence, which has been a comment deferring to this
/// feature since ADR-0014: "org-wide read for **non-restricted** content".
#[test]
fn open_collaboration_reads_the_org_at_confidential_and_never_restricted() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", OPEN_COLLABORATION)];

    for target in ["team-b", "team-c", "sales", "org"] {
        assert_eq!(
            tiers(&pdp, &fx, &ask(&alice, "alice-user", target, &assignments)),
            [
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential
            ],
            "open-collaboration reads {target} up to confidential, and no further"
        );
    }
}

/// `standard`'s sharing default is a default, not an explicit grant — so the
/// subtree of what you hold stops at the working tiers, and `confidential`
/// still takes a binding.
///
/// Until CPR-6 the subtree in question was `principal.department`, read off a
/// rank ladder that no longer exists (ADR-0073 decision 4). It is now
/// `principal.anchors` — the scopes a grant reaches this caller at — so the
/// ask below carries one, and the tier ceiling it demonstrates is unchanged.
#[test]
fn standard_shares_what_you_hold_at_the_working_tiers_only() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let assignments = [fx.assignment("org", STANDARD)];
    // A grant at alice's own team. Its **parent** is her neighbourhood, so
    // `standard` shares the sibling team by default; `regulated-strict` would
    // not, because a grant reaches its own subtree and nothing outward.
    let team_a = fx.node("team-a");
    let anchors = [ScopeAnchor {
        scope_id: team_a.id,
        kind: synveda_types::scope::ScopeKind::Workspace,
        parent_scope_id: team_a.parent_id,
        depth: 0,
        source: AnchorSource::Grant,
        roles: vec![RoleKey::Member],
        granted_at: vec![team_a.id],
        via_groups: Vec::new(),
    }];

    assert_eq!(
        tiers(
            &pdp,
            &fx,
            &Ask {
                anchors: &anchors,
                ..ask(&alice, "alice-user", "team-b", &assignments)
            }
        ),
        [Sensitivity::Public, Sensitivity::Internal],
        "the subtree of what alice holds shares by default at the working tiers"
    );
    assert_eq!(
        tiers(
            &pdp,
            &fx,
            &Ask {
                anchors: &[fx.anchor("team-b", &[RoleKey::Curator])],
                ..ask(&alice, "alice-user", "team-b", &assignments)
            }
        ),
        [
            Sensitivity::Public,
            Sensitivity::Internal,
            Sensitivity::Confidential
        ],
        "and confidential still takes the explicit grant"
    );
    // A team outside the department is closed at every tier, as before.
    assert!(
        tiers(
            &pdp,
            &fx,
            &ask(&alice, "alice-user", "team-c", &assignments)
        )
        .is_empty()
    );
}

/// A `KnowledgeRead` decided without naming a tier is refused, not defaulted:
/// the `grant`-on-`RoleAssign` discipline, applied to the attribute the
/// base layer's `restricted` forbid stands on (decision 2).
#[test]
fn a_read_decided_without_a_tier_fails_closed_rather_than_defaulting() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");
    let scopes = fx.chain("team-a");
    let principal_scopes = fx.chain("alice-user");

    let err = pdp
        .authorize(
            &alice,
            Action::KnowledgeRead,
            Resource::Scope(fx.node("team-a").id),
            &AuthzContext {
                scopes: &scopes,
                principal_scopes: &principal_scopes,
                // No tier: the field a caller must fill for this action.
                ..Default::default()
            },
        )
        .expect_err("a read without a tier cannot be decided");
    assert!(
        err.to_string().contains("sensitivity"),
        "the refusal names what was missing: {err}"
    );
}

/// The prompt plane grades the tiers exactly as the memory plane does, and
/// the top one is reachable by nothing (PRMT-1, ADR-0049 decisions 4 and 5).
///
/// The first half is a transcription check with teeth: each pack's
/// `PromptRead` permits were written by copying its own `KnowledgeRead`
/// permits, and this asks both seams the same four questions at the same
/// scopes and requires identical answers.
///
/// The second half is the difference. `restricted` is denied here the way it
/// is denied for memory, but for a different reason and with a different
/// consequence: memory's is the base layer's invariant forbid; a prompt's is
/// that **no pack names the tier at all**. So a `restricted` prompt would
/// be unreadable by everyone, forever — which is why migration 0029 refuses
/// to store one.
#[test]
fn the_prompt_plane_mirrors_the_memory_plane_and_stops_below_restricted() {
    let pdp = Pdp::new().expect("build pdp");
    let fx = fixture();
    let alice = fx.placed("alice", "alice-user");

    let prompt_tiers = |ask: &Ask<'_>| -> Vec<Sensitivity> {
        Sensitivity::ALL
            .into_iter()
            .filter(|tier| {
                let scopes = fx.chain(ask.target);
                let principal_scopes = fx.chain(ask.placement);
                pdp.authorize(
                    ask.principal,
                    Action::PromptRead,
                    Resource::Scope(fx.node(ask.target).id),
                    &AuthzContext {
                        scopes: &scopes,
                        principal_scopes: &principal_scopes,
                        anchors: ask.anchors,
                        assignments: ask.assignments,
                        relaxations: &[],
                        sensitivity: Some(*tier),
                        ..Default::default()
                    },
                )
                .expect("authorize")
                .allowed
            })
            .collect()
    };

    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let assignments = [fx.assignment("org", pack)];
        let grant = [fx.anchor("eng", &[RoleKey::Viewer])];
        for target in ["alice-user", "team-a", "eng", "org", "team-b", "team-c"] {
            for anchors in [&[][..], &grant[..]] {
                let asking = Ask {
                    anchors,
                    ..ask(&alice, "alice-user", target, &assignments)
                };
                assert_eq!(
                    prompt_tiers(&asking),
                    tiers(&pdp, &fx, &asking),
                    "{pack}: the prompt and memory planes disagree at {target} \
                     (anchors: {})",
                    anchors.len()
                );
            }
        }
    }

    // And the top tier is named by nothing, under every pack, at every
    // scope, for a principal holding every content role — including her own
    // home, which is the one place a prompt's author might expect an
    // exception and does not get one.
    let every_role = [fx.anchor(
        "org",
        &[
            RoleKey::Viewer,
            RoleKey::Member,
            RoleKey::Curator,
            RoleKey::Reviewer,
            RoleKey::Administrator,
        ],
    )];
    for pack in [REGULATED_STRICT, STANDARD, OPEN_COLLABORATION] {
        let assignments = [fx.assignment("org", pack)];
        for target in ["alice-user", "team-a", "eng", "org"] {
            let asking = Ask {
                anchors: &every_role,
                sensitivity: Sensitivity::Restricted,
                ..ask(&alice, "alice-user", target, &assignments)
            };
            assert!(
                !prompt_tiers(&asking).contains(&Sensitivity::Restricted),
                "{pack}: nothing may read a restricted prompt at {target}"
            );
        }
    }
}

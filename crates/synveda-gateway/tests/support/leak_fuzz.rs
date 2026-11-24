//! The vocabulary the policy-leak fuzzes share: the packs and roles a case
//! samples, its readers, where it places items, the checks every case makes,
//! and deterministic sampling.
#![allow(dead_code)]

use super::*;

/// Members may not read confidential Knowledge; administrators may.
pub(crate) const MEMBER_CONFIDENTIAL_DENIED: &str = r#"
permit (principal, action, resource) when { resource in principal.tenant };
forbid (principal, action == Synveda::Action::"KnowledgeRead", resource)
when { context.sensitivity == "confidential" && !context.roles.contains("administrator") };
"#;

/// Only administrators may read Knowledge that is not public.
const INTERNAL_DENIED: &str = r#"
permit (principal, action, resource) when { resource in principal.tenant };
forbid (principal, action == Synveda::Action::"KnowledgeRead", resource)
when { context.sensitivity != "public" && !context.roles.contains("administrator") };
"#;

/// The policy a leak case binds for its tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeakPack {
    Standard,
    RegulatedStrict,
    OpenCollaboration,
    ConfidentialDenied,
    InternalDenied,
}

impl LeakPack {
    pub(crate) const ALL: [Self; 5] = [
        Self::Standard,
        Self::RegulatedStrict,
        Self::OpenCollaboration,
        Self::ConfidentialDenied,
        Self::InternalDenied,
    ];
}

/// The extra project roles a leak case may give Bob, or none.
pub(crate) const LEAK_ROLES: [Option<RoleKey>; 5] = [
    None,
    Some(RoleKey::Viewer),
    Some(RoleKey::Reviewer),
    Some(RoleKey::Curator),
    Some(RoleKey::Administrator),
];

/// A fresh tenant bound to `pack`, with Bob's extra project role. Setup
/// binds the pack first, since binding it resets the root document.
pub(crate) async fn leak_world(pack: LeakPack, bob_project_role: Option<RoleKey>) -> World {
    let world = admitted_world().await.expect("database already checked");
    bind_leak_pack(&world, pack).await;
    if let Some(role) = bob_project_role {
        grant_bob_project_role(&world, role).await;
    }
    world
}

/// Gives Bob `role` at the project scope.
async fn grant_bob_project_role(world: &World, role: RoleKey) {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin role grant");
    seed_grant(&mut tx, world.tenant_id, world.project_scope, BOB, role).await;
    tx.commit().await.expect("commit role grant");
}

/// Installs the case's pack when it is not a built-in one, and selects it
/// for the tenant.
async fn bind_leak_pack(world: &World, pack: LeakPack) {
    let (name, source) = match pack {
        LeakPack::Standard => (synveda_policy::STANDARD, None),
        LeakPack::RegulatedStrict => (synveda_policy::REGULATED_STRICT, None),
        LeakPack::OpenCollaboration => (synveda_policy::OPEN_COLLABORATION, None),
        LeakPack::ConfidentialDenied => {
            ("leak-confidential-denied", Some(MEMBER_CONFIDENTIAL_DENIED))
        }
        LeakPack::InternalDenied => ("leak-internal-denied", Some(INTERNAL_DENIED)),
    };
    match source {
        Some(source) => bind_policy(world, name, source, PackConfig::default()).await,
        None => select_pack(world, name).await,
    }
}

/// Where a leak case places an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Root,
    Workspace,
    Project,
    AlicePrivate,
}

/// The scope, project and owner an item in a placement is created with.
pub(crate) struct PlacedScope<'a> {
    pub(crate) scope_id: ScopeId,
    pub(crate) project_id: Option<&'a str>,
    pub(crate) owner_principal_id: Option<&'static str>,
}

impl PlacedScope<'_> {
    /// An authored create request for `content` in this placement.
    pub(crate) fn create_request(&self, knowledge_type: &str, content: Value) -> Value {
        json!({
            "scope_id": self.scope_id,
            "project_id": self.project_id,
            "owner_principal_id": self.owner_principal_id,
            "knowledge_type": knowledge_type,
            "origin": "authored",
            "content": content,
        })
    }
}

impl Placement {
    pub(crate) const ALL: [Self; 4] = [
        Self::Root,
        Self::Workspace,
        Self::Project,
        Self::AlicePrivate,
    ];

    pub(crate) async fn scope<'a>(self, world: &'a World) -> PlacedScope<'a> {
        let unowned = |scope_id| PlacedScope {
            scope_id,
            project_id: None,
            owner_principal_id: None,
        };
        match self {
            Self::Root => {
                let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
                    .await
                    .expect("begin root read");
                let root = scopes::tenant_root(&mut *tx, world.tenant_id)
                    .await
                    .expect("read root")
                    .expect("root exists");
                tx.commit().await.expect("commit root read");
                unowned(root.id)
            }
            Self::Workspace => unowned(world.workspace_scope),
            Self::Project => PlacedScope {
                project_id: Some(world.project_id.as_str()),
                ..unowned(world.project_scope)
            },
            Self::AlicePrivate => PlacedScope {
                owner_principal_id: Some(ALICE),
                ..unowned(world.alice_scope)
            },
        }
    }
}

/// A reader a leak case checks: Alice, who seeds everything, or Bob.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'a> {
    pub(crate) name: &'static str,
    pub(crate) token: &'a str,
    pub(crate) session: &'a str,
}

impl<'a> Reader<'a> {
    pub(crate) fn alice(world: &'a World) -> Self {
        Self {
            name: "alice",
            token: &world.alice_token,
            session: &world.alice_session,
        }
    }

    pub(crate) fn bob(world: &'a World) -> Self {
        Self {
            name: "bob",
            token: &world.bob_token,
            session: &world.bob_session,
        }
    }
}

/// Whether `token`'s reader may open the item with the single-item read.
/// Only the read's documented answers decide: any other fails the run, so a
/// broken read cannot pass for a denial.
pub(crate) async fn readable_by(world: &World, token: &str, item_id: &str) -> bool {
    let (status, item) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item_id}"),
        token,
        None,
        None,
    )
    .await;
    match status {
        StatusCode::OK => true,
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => false,
        _ => panic!("read of {item_id} answered {status}: {item}"),
    }
}

/// Whether a Knowledge command Alice seeds with was applied. `false` when the
/// case's pack refused it, by authorisation or review; any other answer fails
/// the run, so a seeding mistake cannot pass for a refusal.
pub(crate) fn is_applied(label: &str, status: StatusCode, change: &Value) -> bool {
    match (status, change["outcome"].as_str()) {
        (StatusCode::CREATED, Some("applied")) => true,
        (StatusCode::FORBIDDEN, _) | (StatusCode::CREATED, Some("pending_review" | "rejected")) => {
            false
        }
        _ => panic!("{label}: neither applied nor refused by policy: {status} {change}"),
    }
}

/// `text` names none of `traces`.
pub(crate) fn assert_no_traces(label: &str, text: &str, traces: &[&str]) {
    for trace in traces {
        assert!(!text.contains(trace), "{label} left {trace}: {text}");
    }
}

/// Bob opens nothing in Alice's personal scope, whatever the pack permits:
/// a check on the oracle itself. `bob_reads` pairs each item's placement
/// with whether Bob may open it. Answers how many of Alice's private items
/// were checked.
pub(crate) fn assert_bob_kept_from_alice_private(
    label: &str,
    bob_reads: impl IntoIterator<Item = (Placement, bool)>,
) -> usize {
    let mut checked = 0;
    for (index, (placement, readable)) in bob_reads.into_iter().enumerate() {
        if placement == Placement::AlicePrivate {
            assert!(!readable, "{label}: Bob read Alice's private item {index}");
            checked += 1;
        }
    }
    checked
}

/// Deterministic samples, so a failure names a case that reruns identically.
pub(crate) fn sampled<T: std::fmt::Debug>(
    strategy: impl proptest::strategy::Strategy<Value = T>,
    count: usize,
) -> Vec<T> {
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    (0..count)
        .map(|_| {
            strategy
                .new_tree(&mut runner)
                .expect("sample a case")
                .current()
        })
        .collect()
}

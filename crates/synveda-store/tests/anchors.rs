//! CPR-6 at the storage layer: the scope-anchor resolver (ADR-0073).
//!
//! One question — *which scopes is this request decided against, and what does
//! this caller hold at each* — asserted against real rows, real closure walks
//! and the real grant model.
//!
//! What is asserted here, in the order the feature states it: the six inputs
//! each produce the anchor they should; the order is specificity and never a
//! rank; one scope is one anchor however many ways it became applicable; a
//! grant reaches a scope's subtree with no row written there; a project-only
//! grant reaches neither its workspace nor a sibling; a `principal`-shaped
//! scope inherits nothing; a group's grant follows its membership and an
//! archived group resolves to nobody; a revocation is in force on the next
//! resolution; and every read is tenant-filtered so another tenant's rows are
//! absent rather than forbidden.
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`. Isolation is by freshly minted UUIDv7
//! tenants, so a shared dev database is fine.

#[path = "support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::anchors::{self, AnchorSelection};
use synveda_store::{access, identities, projects, scopes, workspaces};
use synveda_types::access::{GrantSource, GrantSubject, GroupSource, RoleKey};
use synveda_types::anchor::{AnchorSet, AnchorSource};
use synveda_types::scope::ScopeKind;
use synveda_types::workspace::LifecycleStatus;
use synveda_types::{
    GrantId, GroupId, IdentityId, IdentityKind, ProjectId, ScopeId, TenantId, TenantStatus,
    WorkspaceId,
};

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping anchor tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                // Two, not six. Every store suite opens its own pool and
                // `cargo test` runs them in parallel; CPR-5 recorded what a
                // generous per-suite pool costs a `--workspace` run, and the
                // tests here open one transaction at a time.
                .max_connections(2)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            synveda_store::epoch::verify(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

macro_rules! db {
    () => {
        match db() {
            Some(db) => db,
            None => return,
        }
    };
}

async fn admit(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("anc-{}", tenant.as_uuid().simple());
    tenant_fixture::create(pool, tenant, &slug, "CPR-6 fixture", TenantStatus::Active)
        .await
        .expect("admit tenant");
    tenant
}

/// ```text
/// tenant root ── org unit ──┬── workspace "payments" ── project "ledger"
///                           └── workspace "risk"     ── project "models"
/// ```
struct Tree {
    tenant: TenantId,
    root: ScopeId,
    unit: ScopeId,
    payments: WorkspaceId,
    payments_scope: ScopeId,
    risk_scope: ScopeId,
    ledger: ProjectId,
    ledger_scope: ScopeId,
    models_scope: ScopeId,
}

async fn seed(pool: &PgPool) -> Tree {
    let tenant = admit(pool).await;
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    // The root is minted by the first thing that needs a parent.
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint the tenant root");
    let unit = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(root.id),
            slug: "platform".to_owned(),
            display_name: "Platform".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create org unit");
    tx.commit().await.expect("commit scopes");

    let mut tx = tenant_fixture::begin(pool, tenant).await;
    let payments = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "payments".to_owned(),
            display_name: "Payments".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create payments");
    let risk = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "risk".to_owned(),
            display_name: "Risk".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create risk");
    // Both workspaces are minted under the root; move them under the org unit
    // so the fixture has an organisation-unit relationship to resolve.
    scopes::move_scope(&mut tx, tenant, payments.scope_id, unit.id)
        .await
        .expect("move payments");
    scopes::move_scope(&mut tx, tenant, risk.scope_id, unit.id)
        .await
        .expect("move risk");
    let ledger = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: payments.id,
            slug: "ledger".to_owned(),
            display_name: "Ledger".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create ledger");
    let models = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: risk.id,
            slug: "models".to_owned(),
            display_name: "Models".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create models");
    tx.commit().await.expect("commit tree");

    Tree {
        tenant,
        root: root.id,
        unit: unit.id,
        payments: payments.id,
        payments_scope: payments.scope_id,
        risk_scope: risk.scope_id,
        ledger: ledger.id,
        ledger_scope: ledger.scope_id,
        models_scope: models.scope_id,
    }
}

async fn resolve(
    pool: &PgPool,
    tree: &Tree,
    principal_id: &str,
    selection: AnchorSelection,
) -> AnchorSet {
    let mut tx = tenant_fixture::begin(pool, tree.tenant).await;
    let identity_id = identities::by_subject(&mut *tx, tree.tenant, principal_id)
        .await
        .expect("look up identity")
        .map(|identity| identity.id);
    let set = anchors::resolve(&mut tx, tree.tenant, principal_id, identity_id, selection)
        .await
        .expect("resolve anchors");
    tx.commit().await.expect("commit resolution");
    set
}

async fn grant_to(
    pool: &PgPool,
    tenant: TenantId,
    scope_id: ScopeId,
    subject: GrantSubject,
    role: RoleKey,
) -> GrantId {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    let grant = access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id,
            subject,
            role_key: role,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: Some("granter".to_owned()),
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
    grant.id
}

fn subject(id: &str) -> GrantSubject {
    GrantSubject::Principal {
        principal_id: id.to_owned(),
    }
}

// ── The six inputs ───────────────────────────────────────────────────────────

/// A caller with nothing at all still resolves the tenant root — it is where a
/// tenant-wide grant is written and where the default profile sits — and holds
/// nothing at it.
#[test]
fn a_caller_with_nothing_resolves_the_root_and_holds_nothing() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let set = resolve(&db.pool, &tree, "nobody", AnchorSelection::none()).await;

        assert_eq!(set.len(), 1, "only the root is applicable");
        let root = set.get(tree.root).expect("the root anchor");
        assert_eq!(root.source, AnchorSource::TenantRoot);
        assert!(!root.is_held(), "applicable is not the same as held");
        assert_eq!(set.held().count(), 0);
        assert_eq!(set.principal_scope(), None, "no own scope has been minted");
    });
}

/// The caller's own scope is an anchor, it sorts first, and it is theirs to
/// hold: a grant written at it applies, and nothing above it does.
#[test]
fn the_callers_own_scope_sorts_first_and_inherits_nothing() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let own = scopes::ensure_principal_scope(&mut tx, tree.tenant, "alice", "Alice")
            .await
            .expect("mint alice's scope");
        tx.commit().await.expect("commit own scope");

        // The widest thing the model can express, at the root — a
        // **different** role from the one minting the scope already
        // wrote there (CPR-7, ADR-0074 decision 8: every principal scope
        // carries its own `owner` grant at itself, in the same
        // transaction as its creation), so a leak and the baseline are
        // still distinguishable by role key.
        grant_to(
            &db.pool,
            tree.tenant,
            tree.root,
            subject("alice"),
            RoleKey::Administrator,
        )
        .await;

        let set = resolve(&db.pool, &tree, "alice", AnchorSelection::none()).await;
        let first = set.iter().next().expect("at least one anchor");
        assert_eq!(first.scope_id, own.id, "the caller's own scope sorts first");
        assert_eq!(first.source, AnchorSource::PrincipalScope);
        assert_eq!(
            first.roles,
            vec![RoleKey::Owner],
            "only the owner grant the scope minted with itself — the \
             tenant-root administrator grant does not reach into \
             somebody's own scope"
        );
        assert_eq!(first.kind, ScopeKind::Principal);
        assert!(first.is_private());

        // A grant written *at* it does — a second role, so its arrival
        // is visible beside the one the scope already held.
        grant_to(
            &db.pool,
            tree.tenant,
            own.id,
            subject("alice"),
            RoleKey::Curator,
        )
        .await;
        let set = resolve(&db.pool, &tree, "alice", AnchorSelection::none()).await;
        let mine = set.get(own.id).expect("still there");
        assert_eq!(mine.roles, vec![RoleKey::Owner, RoleKey::Curator]);
        assert!(mine.is_direct(), "written here, not inherited");
    });
}

/// Selecting a project resolves the project, its workspace, the organisation
/// units above it and the root — the whole applicable path, in specificity
/// order.
#[test]
fn selecting_a_project_resolves_its_whole_path_in_order() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let set = resolve(
            &db.pool,
            &tree,
            "alice",
            AnchorSelection::project(tree.ledger),
        )
        .await;

        let order: Vec<ScopeId> = set.iter().map(|anchor| anchor.scope_id).collect();
        assert_eq!(
            order,
            vec![tree.ledger_scope, tree.payments_scope, tree.unit, tree.root],
            "project, then its workspace, then the org unit, then the root"
        );
        assert_eq!(
            set.get(tree.ledger_scope).expect("project").source,
            AnchorSource::SelectedProject
        );
        // The workspace above the selected project is an organisation-unit
        // relationship rather than a selection: nobody selected it.
        assert_eq!(
            set.get(tree.payments_scope).expect("workspace").source,
            AnchorSource::OrgUnit
        );
        assert_eq!(
            set.get(tree.unit).expect("unit").source,
            AnchorSource::OrgUnit
        );
        assert!(
            set.get(tree.risk_scope).is_none(),
            "the workspace beside it is not applicable"
        );
    });
}

/// Order is depth, not kind. Two org units at different depths sort deepest
/// first, and nothing anywhere asks which *kind* outranks which.
#[test]
fn order_is_depth_and_never_a_rank() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let deeper = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tree.tenant,
                kind: ScopeKind::OrgUnit,
                parent_scope_id: Some(tree.unit),
                slug: "deeper".to_owned(),
                display_name: "Deeper".to_owned(),
                attributes: serde_json::json!({}),
                principal_id: None,
                created_by: None,
            },
        )
        .await
        .expect("create nested org unit");
        tx.commit().await.expect("commit");

        grant_to(
            &db.pool,
            tree.tenant,
            deeper.id,
            subject("alice"),
            RoleKey::Member,
        )
        .await;
        grant_to(
            &db.pool,
            tree.tenant,
            tree.unit,
            subject("alice"),
            RoleKey::Member,
        )
        .await;

        let set = resolve(&db.pool, &tree, "alice", AnchorSelection::none()).await;
        let held: Vec<ScopeId> = set.held().map(|anchor| anchor.scope_id).collect();
        assert_eq!(
            held,
            vec![deeper.id, tree.unit],
            "the deeper org unit sorts first; both are the same kind"
        );
        assert_eq!(set.get(deeper.id).expect("deeper").depth, 2);
        assert_eq!(set.get(tree.unit).expect("unit").depth, 1);
    });
}

/// One scope is one anchor, however many ways it became applicable, and it
/// keeps the more specific source with the union of what reached it.
#[test]
fn one_scope_is_one_anchor() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        grant_to(
            &db.pool,
            tree.tenant,
            tree.payments_scope,
            subject("alice"),
            RoleKey::Owner,
        )
        .await;

        // Selected *and* granted: two candidates for one scope.
        let set = resolve(
            &db.pool,
            &tree,
            "alice",
            AnchorSelection::workspace(tree.payments),
        )
        .await;
        let matching: Vec<_> = set
            .iter()
            .filter(|anchor| anchor.scope_id == tree.payments_scope)
            .collect();
        assert_eq!(matching.len(), 1, "one scope, one anchor");
        assert_eq!(
            matching[0].source,
            AnchorSource::SelectedWorkspace,
            "the more specific source wins the merge"
        );
        assert_eq!(matching[0].roles, vec![RoleKey::Owner]);
    });
}

// ── What a grant reaches ─────────────────────────────────────────────────────

/// A workspace grant is in force at that workspace's projects, and **no row is
/// written there** to say so.
#[test]
fn a_workspace_grant_reaches_its_projects_with_no_row_written() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        grant_to(
            &db.pool,
            tree.tenant,
            tree.payments_scope,
            subject("alice"),
            RoleKey::Owner,
        )
        .await;

        let set = resolve(
            &db.pool,
            &tree,
            "alice",
            AnchorSelection::project(tree.ledger),
        )
        .await;
        let project = set.get(tree.ledger_scope).expect("the project anchor");
        assert_eq!(project.roles, vec![RoleKey::Owner]);
        assert_eq!(
            project.granted_at,
            vec![tree.payments_scope],
            "the grant is written at the workspace"
        );
        assert!(!project.is_direct(), "and not at the project");

        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let at_project: i64 = sqlx::query_scalar(
            "select count(*) from scope_grants where tenant_id = $1 and scope_id = $2",
        )
        .bind(tree.tenant.as_uuid())
        .bind(tree.ledger_scope.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("count");
        tx.commit().await.expect("commit count");
        assert_eq!(at_project, 0, "inheritance writes nothing");
    });
}

/// A project grant reaches the project and stops: not its workspace, not a
/// sibling project, not the org unit.
#[test]
fn a_project_grant_reaches_nothing_above_or_beside_it() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        grant_to(
            &db.pool,
            tree.tenant,
            tree.ledger_scope,
            subject("bob"),
            RoleKey::Member,
        )
        .await;

        let set = resolve(
            &db.pool,
            &tree,
            "bob",
            AnchorSelection::project(tree.ledger),
        )
        .await;
        assert_eq!(
            set.get(tree.ledger_scope).expect("project").roles,
            vec![RoleKey::Member]
        );
        for (name, scope) in [
            ("its workspace", tree.payments_scope),
            ("the org unit", tree.unit),
            ("the tenant root", tree.root),
        ] {
            assert!(
                set.get(scope).expect(name).roles.is_empty(),
                "a project grant must not reach {name}"
            );
        }
        // And the sibling workspace is not even applicable.
        assert!(set.get(tree.risk_scope).is_none());
        assert!(set.get(tree.models_scope).is_none());
    });
}

/// A grant reaches its scope even when the request selected something else —
/// which is what makes project-only access usable at all.
#[test]
fn a_grant_is_applicable_without_being_selected() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        grant_to(
            &db.pool,
            tree.tenant,
            tree.ledger_scope,
            subject("bob"),
            RoleKey::Member,
        )
        .await;

        let set = resolve(&db.pool, &tree, "bob", AnchorSelection::none()).await;
        let held: Vec<ScopeId> = set.held().map(|anchor| anchor.scope_id).collect();
        assert_eq!(held, vec![tree.ledger_scope]);
        assert_eq!(
            set.get(tree.ledger_scope).expect("project").source,
            AnchorSource::Grant,
            "a grant nobody selected is its own source"
        );
    });
}

// ── Groups ───────────────────────────────────────────────────────────────────

/// A grant naming a group reaches its members, records which group reached
/// them, and stops reaching when the group is archived.
#[test]
fn a_group_grant_follows_its_membership() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let carol_scope = scopes::ensure_principal_scope(&mut tx, tree.tenant, "carol", "Carol")
            .await
            .expect("create Carol's scope");
        let carol = identities::create(
            &mut tx,
            IdentityId::new(),
            tree.tenant,
            Some("carol"),
            IdentityKind::User,
            None,
            Some("Carol"),
            carol_scope.id,
        )
        .await
        .expect("create Carol's identity");
        let group = access::create_group(
            &mut *tx,
            &access::NewGroup {
                id: GroupId::new(),
                tenant_id: tree.tenant,
                slug: "reviewers".to_owned(),
                display_name: "Reviewers".to_owned(),
                description: None,
                source: GroupSource::Direct,
                directory_source: None,
                directory_resource_id: None,
                directory_external_id: None,
                created_by: None,
            },
        )
        .await
        .expect("create group");
        access::set_group_members(&mut tx, tree.tenant, group.id, &[carol.id], None)
            .await
            .expect("set members");
        tx.commit().await.expect("commit group");

        grant_to(
            &db.pool,
            tree.tenant,
            tree.payments_scope,
            GrantSubject::Group { group_id: group.id },
            RoleKey::Curator,
        )
        .await;

        let set = resolve(
            &db.pool,
            &tree,
            "carol",
            AnchorSelection::project(tree.ledger),
        )
        .await;
        let project = set.get(tree.ledger_scope).expect("project");
        assert_eq!(project.roles, vec![RoleKey::Curator]);
        assert_eq!(project.via_groups, vec![group.id], "the group is recorded");
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        assert_eq!(
            anchors::groups_of(&mut *tx, tree.tenant, Some(carol.id))
                .await
                .expect("groups"),
            vec![group.id]
        );
        tx.commit().await.expect("commit group read");

        // Archived: it resolves to nobody, on the very next resolution.
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        access::update_group(
            &mut tx,
            tree.tenant,
            group.id,
            group.revision,
            &access::GroupUpdate {
                display_name: None,
                description: None,
                status: Some(LifecycleStatus::Archived),
                members: None,
            },
            None,
        )
        .await
        .expect("archive");
        tx.commit().await.expect("commit archive");

        let set = resolve(
            &db.pool,
            &tree,
            "carol",
            AnchorSelection::project(tree.ledger),
        )
        .await;
        assert!(
            set.get(tree.ledger_scope)
                .expect("project")
                .roles
                .is_empty(),
            "an archived group confers nothing"
        );
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        assert!(
            anchors::groups_of(&mut *tx, tree.tenant, Some(carol.id))
                .await
                .expect("groups")
                .is_empty(),
            "and is not a group the PDP materialises"
        );
        tx.commit().await.expect("commit archived group read");
    });
}

// ── Revocation ───────────────────────────────────────────────────────────────

/// A revoked grant is gone from the next resolution. Nothing runs and nothing
/// is invalidated: the resolution *is* the check.
#[test]
fn revoking_a_grant_is_in_force_on_the_next_resolution() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let id = grant_to(
            &db.pool,
            tree.tenant,
            tree.payments_scope,
            subject("alice"),
            RoleKey::Owner,
        )
        .await;

        let set = resolve(
            &db.pool,
            &tree,
            "alice",
            AnchorSelection::workspace(tree.payments),
        )
        .await;
        assert_eq!(
            set.get(tree.payments_scope).expect("workspace").roles,
            vec![RoleKey::Owner]
        );

        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        access::revoke_grant(&mut tx, tree.tenant, id)
            .await
            .expect("revoke");
        tx.commit().await.expect("commit revocation");

        let set = resolve(
            &db.pool,
            &tree,
            "alice",
            AnchorSelection::workspace(tree.payments),
        )
        .await;
        let workspace = set.get(tree.payments_scope).expect("still applicable");
        assert!(
            workspace.roles.is_empty(),
            "the scope is still applicable and confers nothing"
        );
        assert_eq!(set.held().count(), 0);
    });
}

// ── Tenancy ──────────────────────────────────────────────────────────────────

/// Another tenant's workspace, project and grants are **absent**, not
/// forbidden: a selection naming one contributes no anchor, and a grant in one
/// contributes no role.
#[test]
fn another_tenants_rows_are_absent_from_a_resolution() {
    let db = db!();
    db.rt.block_on(async {
        let ours = seed(&db.pool).await;
        let theirs = seed(&db.pool).await;
        grant_to(
            &db.pool,
            theirs.tenant,
            theirs.payments_scope,
            subject("alice"),
            RoleKey::Owner,
        )
        .await;

        // Their project id, resolved in our tenant.
        let set = resolve(
            &db.pool,
            &ours,
            "alice",
            AnchorSelection::project(theirs.ledger),
        )
        .await;
        assert_eq!(
            set.len(),
            1,
            "a foreign selection contributes nothing but our root"
        );
        assert_eq!(set.iter().next().expect("root").scope_id, ours.root);
        assert_eq!(
            set.held().count(),
            0,
            "and their grant reaches nothing here"
        );
        let mut tx = tenant_fixture::begin(&db.pool, ours.tenant).await;
        assert!(
            anchors::groups_of(&mut *tx, ours.tenant, None)
                .await
                .expect("groups")
                .is_empty()
        );
        tx.commit().await.expect("commit tenant-filtered read");
    });
}

// ── The principal scope's own rules ──────────────────────────────────────────

/// A principal scope names its subject, is minted once, and is found by it.
#[test]
fn a_principal_scope_is_minted_once_and_found_by_its_subject() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let first = scopes::ensure_principal_scope(&mut tx, tree.tenant, "alice", "Alice")
            .await
            .expect("mint");
        let again = scopes::ensure_principal_scope(&mut tx, tree.tenant, "alice", "Alice Again")
            .await
            .expect("idempotent");
        tx.commit().await.expect("commit");

        assert_eq!(first.id, again.id, "one scope per subject");
        assert_eq!(first.principal_id.as_deref(), Some("alice"));
        assert_eq!(first.kind, ScopeKind::Principal);
        assert_eq!(
            first.parent_scope_id,
            Some(tree.root),
            "a principal scope hangs off the tenant root"
        );
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        assert_eq!(
            scopes::principal_scope(&mut *tx, tree.tenant, "alice")
                .await
                .expect("look up")
                .map(|scope| scope.id),
            Some(first.id)
        );
        assert_eq!(
            scopes::principal_scope(&mut *tx, tree.tenant, "nobody")
                .await
                .expect("look up"),
            None
        );
        tx.commit().await.expect("commit principal-scope reads");
    });
}

/// The two structural rules, **against direct SQL**: `principal_id` is present
/// exactly on a principal scope, and it is immutable.
///
/// A rule that only holds for callers who went through a function is not a
/// rule (ADR-0070 decision 2).
#[test]
fn the_principal_id_rules_hold_against_direct_sql() {
    let db = db!();
    db.rt.block_on(async {
        let tree = seed(&db.pool).await;

        // A non-principal scope may not name a subject.
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let forbidden = sqlx::query(
            "insert into scopes (id, tenant_id, kind, parent_scope_id, parent_kind, slug,
                                 display_name, status, attributes, principal_id)
             values ($1, $2, 'org_unit', $3, 'tenant', 'sneaky', 'Sneaky', 'active',
                     '{}'::jsonb, 'alice')",
        )
        .bind(ScopeId::new().as_uuid())
        .bind(tree.tenant.as_uuid())
        .bind(tree.root.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(forbidden.is_err(), "only a principal scope names a subject");
        drop(tx);

        // A principal scope must.
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let missing = sqlx::query(
            "insert into scopes (id, tenant_id, kind, parent_scope_id, parent_kind, slug,
                                 display_name, status, attributes, principal_id)
             values ($1, $2, 'principal', $3, 'tenant', 'anon', 'Anon', 'active',
                     '{}'::jsonb, null)",
        )
        .bind(ScopeId::new().as_uuid())
        .bind(tree.tenant.as_uuid())
        .bind(tree.root.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(missing.is_err(), "a principal scope must name one");
        drop(tx);

        // And whose it is cannot be edited.
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let mine = scopes::ensure_principal_scope(&mut tx, tree.tenant, "alice", "Alice")
            .await
            .expect("mint");
        tx.commit().await.expect("commit");
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let repointed = sqlx::query("update scopes set principal_id = 'mallory' where id = $1")
            .bind(mine.id.as_uuid())
            .execute(&mut *tx)
            .await;
        assert!(
            repointed.is_err(),
            "re-pointing a private scope would hand somebody's material to a new subject"
        );
        drop(tx);

        // Two subjects cannot share one.
        let mut tx = tenant_fixture::begin(&db.pool, tree.tenant).await;
        let second = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tree.tenant,
                kind: ScopeKind::Principal,
                parent_scope_id: Some(tree.root),
                slug: "alice-again".to_owned(),
                display_name: "Alice Again".to_owned(),
                attributes: serde_json::json!({}),
                principal_id: Some("alice".to_owned()),
                created_by: None,
            },
        )
        .await;
        assert!(second.is_err(), "one scope per subject per tenant");
    });
}

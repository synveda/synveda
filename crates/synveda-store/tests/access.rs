//! CPR-5 acceptance criteria at the storage layer: groups, scope grants and
//! invitations, and the resolution that answers "who may act here" (ADR-0072).
//!
//! What is asserted here, in the order the feature states it: a workspace-level
//! grant is in force at that workspace's projects and **no row is written to
//! say so**; a project-only grant is in force nowhere else; a `principal`-shaped
//! scope inherits nothing from anywhere; a group resolves to its members and an
//! archived group resolves to nobody; an invitation is one-time, expires without
//! anything running, and is refused after either terminal state; a token is
//! stored only as a hash; revocation removes exactly what was written here and
//! refuses what a directory owns; and every read is tenant-filtered so another
//! tenant's row is absent rather than forbidden.
//!
//! The structural rules that are database facts are asserted **against direct
//! SQL** as well as through the services, because a rule that only holds for
//! callers who went through a function is not a rule (ADR-0070 decision 2).
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`. Isolation is by freshly minted UUIDv7
//! tenants, so a shared dev database is fine.

#[path = "support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::OnceLock;

use chrono::{Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{access, identities, projects, scopes, workspaces};
use synveda_types::access::{
    GrantSource, GrantSubject, GroupSource, InviteStatus, RoleKey, SubjectKind,
};
use synveda_types::scope::ScopeKind;
use synveda_types::workspace::LifecycleStatus;
use synveda_types::{
    Error, GrantId, GroupId, IdentityId, IdentityKind, InviteId, ProjectId, ScopeId, TenantId,
    TenantStatus, WorkspaceId,
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
                    "skipping access tests: DATABASE_URL is not set \
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
                .max_connections(6)
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

async fn admit(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("acc-{}", tenant.as_uuid().simple());
    tenant_fixture::create(pool, tenant, &slug, "CPR-5 fixture", TenantStatus::Active)
        .await
        .expect("admit tenant");
    tenant
}

async fn begin(pool: &PgPool, tenant: TenantId) -> Transaction<'static, Postgres> {
    tenant_fixture::begin(pool, tenant).await
}

/// A workspace with one project under it, and the scopes both own.
struct Tree {
    tenant: TenantId,
    workspace: WorkspaceId,
    workspace_scope: ScopeId,
    project_scope: ScopeId,
    tenant_scope: ScopeId,
}

async fn seed_tree(pool: &PgPool) -> Tree {
    let tenant = admit(pool).await;
    let mut tx = begin(pool, tenant).await;
    let workspace = workspaces::create(
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
    .expect("create workspace");
    let project = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            slug: "ledger".to_owned(),
            display_name: "Ledger".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create project");
    let root = scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read root")
        .expect("the workspace minted one");
    tx.commit().await.expect("commit tree");
    Tree {
        tenant,
        workspace: workspace.id,
        workspace_scope: workspace.scope_id,
        project_scope: project.scope_id,
        tenant_scope: root.id,
    }
}

/// A `principal`-shaped scope hanging off the tenant root — somebody's own.
async fn seed_principal_scope(pool: &PgPool, tree: &Tree, slug: &str) -> ScopeId {
    let mut tx = begin(pool, tree.tenant).await;
    let scope = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tree.tenant,
            kind: ScopeKind::Principal,
            parent_scope_id: Some(tree.tenant_scope),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: Some(format!("subject-{slug}")),
            created_by: None,
        },
    )
    .await
    .expect("create principal scope");
    tx.commit().await.expect("commit principal scope");
    scope.id
}

async fn grant(
    pool: &PgPool,
    tenant: TenantId,
    scope_id: ScopeId,
    subject: GrantSubject,
    role: RoleKey,
    source: GrantSource,
) -> synveda_types::access::ScopeGrant {
    let mut tx = begin(pool, tenant).await;
    let grant = access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id,
            subject,
            role_key: role,
            source,
            invite_id: None,
            granted_by: Some("granter".to_owned()),
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
    grant
}

async fn claim_initial_admin(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    root_scope: ScopeId,
    subject: &str,
) -> bool {
    let grant_id = GrantId::new();
    let claimed = access::claim_initial_administrator_bootstrap(tx, tenant, grant_id, subject)
        .await
        .expect("claim initial administrator bootstrap");
    if claimed {
        access::create_grant(
            tx,
            &access::NewGrant {
                id: grant_id,
                tenant_id: tenant,
                scope_id: root_scope,
                subject: principal(subject),
                role_key: RoleKey::Administrator,
                source: GrantSource::Automation,
                invite_id: None,
                granted_by: None,
            },
        )
        .await
        .expect("create initial administrator grant");
    }
    claimed
}

async fn tenant_root_grants(
    pool: &PgPool,
    tenant: TenantId,
    root_scope: ScopeId,
) -> Vec<synveda_types::access::ScopeGrant> {
    let mut tx = begin(pool, tenant).await;
    access::list_grants(
        &mut *tx,
        tenant,
        &access::GrantFilter {
            scope_id: Some(root_scope),
            principal_id: None,
        },
    )
    .await
    .expect("list tenant-root grants")
}

fn principal(id: &str) -> GrantSubject {
    GrantSubject::Principal {
        principal_id: id.to_owned(),
    }
}

async fn new_group(pool: &PgPool, tenant: TenantId, slug: &str, members: &[&str]) -> GroupId {
    let mut tx = begin(pool, tenant).await;
    let group = access::create_group(
        &mut *tx,
        &access::NewGroup {
            id: GroupId::new(),
            tenant_id: tenant,
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            description: None,
            source: GroupSource::Direct,
            directory_source: None,
            directory_resource_id: None,
            directory_external_id: None,
            created_by: Some("granter".to_owned()),
        },
    )
    .await
    .expect("create group");
    let mut owned = Vec::with_capacity(members.len());
    for member in members {
        let identity_id = IdentityId::new();
        let root = scopes::ensure_tenant_root(&mut tx, tenant)
            .await
            .expect("create tenant root");
        let scope = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tenant,
                kind: ScopeKind::Principal,
                parent_scope_id: Some(root.id),
                slug: format!("member-{}", identity_id.as_uuid().simple()),
                display_name: (*member).to_owned(),
                attributes: serde_json::json!({}),
                principal_id: Some((*member).to_owned()),
                created_by: None,
            },
        )
        .await
        .expect("create member scope");
        let identity = identities::create(
            &mut tx,
            identity_id,
            tenant,
            Some(member),
            IdentityKind::User,
            None,
            Some(member),
            scope.id,
        )
        .await
        .expect("create member identity");
        owned.push(identity.id);
    }
    access::set_group_members(&mut tx, tenant, group.id, &owned, Some("granter"))
        .await
        .expect("set members");
    tx.commit().await.expect("commit group");
    group.id
}

async fn new_identity(pool: &PgPool, tenant: TenantId, subject: &str) -> IdentityId {
    let mut tx = begin(pool, tenant).await;
    let identity_id = IdentityId::new();
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("create tenant root");
    let scope = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::Principal,
            parent_scope_id: Some(root.id),
            slug: format!("identity-{}", identity_id.as_uuid().simple()),
            display_name: subject.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: Some(subject.to_owned()),
            created_by: None,
        },
    )
    .await
    .expect("create member scope");
    let identity = identities::create(
        &mut tx,
        identity_id,
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        scope.id,
    )
    .await
    .expect("create member identity");
    tx.commit().await.expect("commit identity");
    identity.id
}

async fn members_at(pool: &PgPool, tenant: TenantId, scope: ScopeId) -> Vec<access::AccessEntry> {
    let mut tx = begin(pool, tenant).await;
    let members = access::members_of(&mut *tx, tenant, scope)
        .await
        .expect("resolve members");
    tx.commit().await.expect("commit member resolution");
    members
}

// ── Inheritance ──────────────────────────────────────────────────────────────

/// The feature's first sentence: a grant at a workspace is in force at that
/// workspace's projects, and **nothing is written to say so**.
#[test]
fn a_workspace_grant_reaches_its_projects_without_a_second_row() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;

        let at_project = members_at(&db.pool, tree.tenant, tree.project_scope).await;
        assert_eq!(at_project.len(), 1, "the project inherits the grant");
        assert_eq!(at_project[0].principal_id, "robin");
        assert_eq!(at_project[0].role_key, RoleKey::Member);
        assert!(at_project[0].inherited, "it was not written here");
        assert_eq!(
            at_project[0].scope_id, tree.workspace_scope,
            "the entry names where the grant actually lives, which is what \
             makes 'why can this person see my project' answerable"
        );

        let mut tx = synveda_store::rls::begin_tenant_tx(&db.pool, tree.tenant)
            .await
            .expect("begin tenant transaction");
        let rows = access::list_grants(
            &mut *tx,
            tree.tenant,
            &access::GrantFilter {
                scope_id: Some(tree.project_scope),
                principal_id: None,
            },
        )
        .await
        .expect("list at the project");
        assert!(
            rows.is_empty(),
            "inheritance must not materialise a per-project row: {rows:?}"
        );
        tx.commit().await.expect("commit tenant transaction");
    });
}

/// A project-only grant is in force at the project and nowhere else — not at
/// the workspace above it, not at a sibling project.
#[test]
fn a_project_grant_stays_in_its_project() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let sibling = projects::create(
            &mut tx,
            &projects::NewProject {
                id: ProjectId::new(),
                tenant_id: tree.tenant,
                workspace_id: tree.workspace,
                slug: "reporting".to_owned(),
                display_name: "Reporting".to_owned(),
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("create sibling");
        tx.commit().await.expect("commit sibling");

        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            principal("kim"),
            RoleKey::Viewer,
            GrantSource::Direct,
        )
        .await;

        let at_project = members_at(&db.pool, tree.tenant, tree.project_scope).await;
        assert_eq!(at_project.len(), 1);
        assert!(!at_project[0].inherited, "it was written here");

        assert!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .is_empty(),
            "a project grant must not climb to its workspace"
        );
        assert!(
            members_at(&db.pool, tree.tenant, sibling.scope_id)
                .await
                .is_empty(),
            "a project grant must not reach a sibling project"
        );
    });
}

/// **Principal-private scope isolation.** A grant at the tenant root reaches
/// every workspace and project under it — and does not reach anybody's own
/// scope, which is the point.
#[test]
fn a_principal_scope_inherits_nothing_from_anywhere() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let mine = seed_principal_scope(&db.pool, &tree, "sam").await;

        // A tenant-wide grant: the widest thing this model can say.
        grant(
            &db.pool,
            tree.tenant,
            tree.tenant_scope,
            principal("the-boss"),
            RoleKey::Administrator,
            GrantSource::Direct,
        )
        .await;

        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .len(),
            1,
            "a tenant-root grant reaches a workspace"
        );
        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.project_scope)
                .await
                .len(),
            1,
            "and a project inside it"
        );
        assert!(
            members_at(&db.pool, tree.tenant, mine).await.is_empty(),
            "and must not reach anybody's own scope, however wide it is"
        );

        // A grant written *at* the principal scope still applies: isolation is
        // about inheritance, not about the scope being unreachable.
        grant(
            &db.pool,
            tree.tenant,
            mine,
            principal("sam"),
            RoleKey::Owner,
            GrantSource::Owner,
        )
        .await;
        let mine_members = members_at(&db.pool, tree.tenant, mine).await;
        assert_eq!(mine_members.len(), 1, "only the grant written here");
        assert_eq!(mine_members[0].principal_id, "sam");
        assert!(!mine_members[0].inherited);
    });
}

/// Nearest-first: a reader sees the most specific authority at the top, so a
/// client rendering a member list does not have to sort it itself.
#[test]
fn the_resolution_orders_the_nearest_grant_first() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.tenant_scope,
            principal("aaa"),
            RoleKey::Viewer,
            GrantSource::Direct,
        )
        .await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("bbb"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            principal("ccc"),
            RoleKey::Curator,
            GrantSource::Direct,
        )
        .await;

        let members = members_at(&db.pool, tree.tenant, tree.project_scope).await;
        let order: Vec<&str> = members
            .iter()
            .map(|entry| entry.principal_id.as_str())
            .collect();
        assert_eq!(
            order,
            vec!["ccc", "bbb", "aaa"],
            "nearest scope first, whatever the principals sort to"
        );
        assert_eq!(
            members
                .iter()
                .map(|entry| entry.inherited)
                .collect::<Vec<_>>(),
            vec![false, true, true]
        );
    });
}

// ── Groups ───────────────────────────────────────────────────────────────────

/// A grant to a group resolves to its members, and adding somebody to the
/// group gives them the grant with no second write anywhere.
#[test]
fn a_group_grant_resolves_to_its_members_and_follows_them() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "engineering", &["robin", "kim"]).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            GrantSubject::Group { group_id: group },
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;

        let members = members_at(&db.pool, tree.tenant, tree.project_scope).await;
        assert_eq!(members.len(), 2, "both members hold it, at the project too");
        for entry in &members {
            assert_eq!(
                entry.via_group.as_ref().map(|group| group.slug.as_str()),
                Some("engineering"),
                "the entry says which group it came through"
            );
            assert!(entry.inherited);
        }

        // A third person joins the group; nothing is written on the grant.
        let sam = new_identity(&db.pool, tree.tenant, "sam").await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let mut members: Vec<IdentityId> = access::group_members(&mut *tx, tree.tenant, group)
            .await
            .expect("current members")
            .into_iter()
            .map(|member| member.identity_id)
            .collect();
        members.push(sam);
        access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate {
                members: Some(members),
                ..Default::default()
            },
            Some("granter"),
        )
        .await
        .expect("replace membership");
        tx.commit().await.expect("commit membership");

        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.project_scope)
                .await
                .len(),
            3,
            "joining the group is joining everything it holds"
        );
    });
}

/// An **archived** group confers nothing. Retiring a group is how a deployment
/// withdraws what it grants without hunting down every grant that names it.
#[test]
fn an_archived_group_confers_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "contractors", &["robin"]).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            GrantSubject::Group { group_id: group },
            RoleKey::Viewer,
            GrantSource::Direct,
        )
        .await;
        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .len(),
            1
        );

        let mut tx = begin(&db.pool, tree.tenant).await;
        access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate {
                status: Some(LifecycleStatus::Archived),
                ..Default::default()
            },
            Some("granter"),
        )
        .await
        .expect("archive");
        tx.commit().await.expect("commit archive");

        assert!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .is_empty(),
            "an archived group resolves to nobody"
        );
        let mut tx = begin(&db.pool, tree.tenant).await;
        assert_eq!(
            access::list_grants(&mut *tx, tree.tenant, &access::GrantFilter::default())
                .await
                .expect("list")
                .len(),
            1,
            "and the grant is still there — archiving is not revoking"
        );
        tx.commit().await.expect("commit archived-group read");
    });
}

/// An empty group grants access to nobody, which is the honest answer rather
/// than a row that looks like a member.
#[test]
fn an_empty_group_grants_access_to_nobody() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "empty", &[]).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            GrantSubject::Group { group_id: group },
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        assert!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .is_empty()
        );
    });
}

/// A membership replacement is a replacement: what is not in the list is gone,
/// and a caller who listed somebody twice gets one membership rather than an
/// error about their own duplicate.
#[test]
fn a_membership_replacement_is_the_whole_list() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "eng", &["robin", "kim", "sam"]).await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let sam = access::group_members(&mut *tx, tree.tenant, group)
            .await
            .expect("members")
            .into_iter()
            .find(|member| member.principal_id.as_deref() == Some("sam"))
            .expect("sam")
            .identity_id;
        tx.commit().await.expect("commit member lookup");

        let mut tx = begin(&db.pool, tree.tenant).await;
        access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate {
                members: Some(vec![sam, sam]),
                ..Default::default()
            },
            Some("granter"),
        )
        .await
        .expect("replace");
        tx.commit().await.expect("commit");

        let mut tx = begin(&db.pool, tree.tenant).await;
        let members = access::group_members(&mut *tx, tree.tenant, group)
            .await
            .expect("read members");
        assert_eq!(members.len(), 1, "a duplicate is one membership");
        assert_eq!(members[0].principal_id.as_deref(), Some("sam"));
        tx.commit().await.expect("commit replacement read");
    });
}

/// The revision precondition: a stale update is refused and writes nothing,
/// membership included.
#[test]
fn a_stale_group_update_writes_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "eng", &["robin"]).await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate {
                display_name: Some("Engineering".to_owned()),
                ..Default::default()
            },
            Some("granter"),
        )
        .await
        .expect("first update");
        tx.commit().await.expect("commit");

        let mut tx = begin(&db.pool, tree.tenant).await;
        let stale = access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate {
                members: Some(Vec::new()),
                ..Default::default()
            },
            Some("granter"),
        )
        .await;
        assert!(
            matches!(stale, Err(Error::Conflict { .. })),
            "a stale precondition is a conflict, got {stale:?}"
        );
        drop(tx);

        let mut tx = begin(&db.pool, tree.tenant).await;
        assert_eq!(
            access::group_members(&mut *tx, tree.tenant, group)
                .await
                .expect("read members")
                .len(),
            1,
            "the refused update must not have emptied the group"
        );
        tx.commit().await.expect("commit stale-update read");
    });
}

/// An empty update is a client bug and is said so, rather than answered with a
/// bumped revision that hides one.
#[test]
fn an_empty_group_update_is_refused() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "eng", &[]).await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let result = access::update_group(
            &mut tx,
            tree.tenant,
            group,
            1,
            &access::GroupUpdate::default(),
            None,
        )
        .await;
        assert!(matches!(result, Err(Error::Invalid { .. })), "{result:?}");
    });
}

/// A directory group carries its provider and stable resource address, and a
/// group this product owns carries none — both ways round, in the type layer
/// and behind it in a CHECK.
#[test]
fn a_directory_group_carries_source_identity_and_a_direct_one_does_not() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        for (source, directory_source, resource_id) in [
            (GroupSource::Directory, None, None),
            (
                GroupSource::Direct,
                Some("okta".to_owned()),
                Some("00u1a2b3".to_owned()),
            ),
        ] {
            let mut tx = begin(&db.pool, tenant).await;
            let result = access::create_group(
                &mut *tx,
                &access::NewGroup {
                    id: GroupId::new(),
                    tenant_id: tenant,
                    slug: "eng".to_owned(),
                    display_name: "Eng".to_owned(),
                    description: None,
                    source,
                    directory_source,
                    directory_resource_id: resource_id,
                    directory_external_id: None,
                    created_by: None,
                },
            )
            .await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "{source} with the wrong reference shape must be refused, got {result:?}"
            );
            drop(tx);
        }

        // And the CHECK holds against direct SQL, for a writer that never went
        // through the service.
        let mut tx = begin(&db.pool, tenant).await;
        let err = sqlx::query(
            "insert into groups (id, tenant_id, slug, display_name, source, \
             directory_source, directory_resource_id) \
             values ($1, $2, 'forged', 'Forged', 'directory', null, null)",
        )
        .bind(GroupId::new().as_uuid())
        .bind(tenant.as_uuid())
        .execute(&mut *tx)
        .await
        .expect_err("the CHECK refuses it");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("23514")
        );
    });
}

/// A directory-managed group is not the product's to edit: the refusal names
/// the directory rather than sounding like a permission problem.
#[test]
fn a_directory_group_cannot_be_edited_here() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool, tenant).await;
        let group = access::create_group(
            &mut *tx,
            &access::NewGroup {
                id: GroupId::new(),
                tenant_id: tenant,
                slug: "entra-eng".to_owned(),
                display_name: "Engineering".to_owned(),
                description: None,
                source: GroupSource::Directory,
                directory_source: Some("okta".to_owned()),
                directory_resource_id: Some("00u1a2b3".to_owned()),
                directory_external_id: None,
                created_by: None,
            },
        )
        .await
        .expect("create directory group");
        tx.commit().await.expect("commit directory group");

        let mut tx = begin(&db.pool, tenant).await;
        let result = access::update_group(
            &mut tx,
            tenant,
            group.id,
            1,
            &access::GroupUpdate {
                members: Some(vec![IdentityId::new()]),
                ..Default::default()
            },
            Some("someone"),
        )
        .await;
        let Err(Error::Conflict { message }) = result else {
            panic!("expected a conflict naming the directory, got {result:?}");
        };
        assert!(message.contains("directory"), "{message}");
        assert!(message.contains("entra-eng"), "{message}");
    });
}

/// A group's handle and provenance are immutable, and its revision steps
/// forward by exactly one — against direct SQL, so the rule holds for the owner
/// role that migrations and break-glass psql run as.
#[test]
#[ignore = "serial administrator tamper acceptance"]
fn a_groups_identity_is_immutable_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let group = new_group(&db.pool, tenant, "eng", &[]).await;
        let administrator = tenant_fixture::administrator_pool(&db.pool).await;

        for (statement, what) in [
            (
                "update groups set slug = 'renamed', revision = revision + 1 where id = $1",
                "slug",
            ),
            (
                "update groups set source = 'directory', revision = revision + 1 where id = $1",
                "source",
            ),
            (
                "update groups set revision = revision + 5 where id = $1",
                "a skipped revision",
            ),
            (
                "update groups set revision = revision - 1 where id = $1",
                "a rewound revision",
            ),
            (
                "update groups set created_at = now(), revision = revision + 1 where id = $1",
                "provenance",
            ),
        ] {
            let err = sqlx::query(statement)
                .bind(group.as_uuid())
                .execute(&administrator)
                .await
                .expect_err(what);
            assert_eq!(
                err.as_database_error().and_then(|db| db.code()).as_deref(),
                Some("P0001"),
                "{what} must be refused by the trigger, got {err:?}"
            );
        }
    });
}

// ── Grants ───────────────────────────────────────────────────────────────────

/// Granting the same subject the same role at the same scope twice is a
/// conflict rather than two rows that mean one thing.
#[test]
fn one_subject_holds_one_role_once_per_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let again = access::create_grant(
            &mut tx,
            &access::NewGrant {
                id: GrantId::new(),
                tenant_id: tree.tenant,
                scope_id: tree.workspace_scope,
                subject: principal("robin"),
                role_key: RoleKey::Member,
                source: GrantSource::Direct,
                invite_id: None,
                granted_by: None,
            },
        )
        .await;
        assert!(matches!(again, Err(Error::Conflict { .. })), "{again:?}");
        drop(tx);

        // A *different* role for the same person is a different grant: they
        // are additive, and revoked separately.
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Curator,
            GrantSource::Direct,
        )
        .await;
        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.workspace_scope)
                .await
                .len(),
            2
        );
    });
}

/// A grant has exactly one subject, and the database says so — asserted
/// against direct SQL, because the shape is a CHECK rather than a convention.
#[test]
fn a_grant_has_exactly_one_subject_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let group = new_group(&db.pool, tree.tenant, "eng", &[]).await;

        for (kind, principal_id, group_id, what) in [
            (
                "principal",
                None,
                Some(group),
                "a principal grant naming a group",
            ),
            (
                "group",
                Some("robin"),
                None,
                "a group grant naming a principal",
            ),
            ("principal", None, None, "a grant naming nobody"),
        ] {
            let mut tx = begin(&db.pool, tree.tenant).await;
            let err = sqlx::query(
                "insert into scope_grants \
                 (id, tenant_id, scope_id, subject_kind, principal_id, group_id, role_key, source) \
                 values ($1, $2, $3, $4, $5, $6, 'member', 'direct')",
            )
            .bind(GrantId::new().as_uuid())
            .bind(tree.tenant.as_uuid())
            .bind(tree.workspace_scope.as_uuid())
            .bind(kind)
            .bind(principal_id)
            .bind(group_id.map(|id| id.as_uuid()))
            .execute(&mut *tx)
            .await
            .expect_err(what);
            assert_eq!(
                err.as_database_error().and_then(|db| db.code()).as_deref(),
                Some("23514"),
                "{what} must be refused by a CHECK, got {err:?}"
            );
            drop(tx);
        }
    });
}

/// Provenance that can be claimed without evidence is not provenance: only an
/// `invite`-sourced grant names an invitation, and it must name one.
#[test]
fn only_an_invite_grant_names_an_invitation() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let claimed = access::create_grant(
            &mut tx,
            &access::NewGrant {
                id: GrantId::new(),
                tenant_id: tree.tenant,
                scope_id: tree.workspace_scope,
                subject: principal("robin"),
                role_key: RoleKey::Member,
                source: GrantSource::Invite,
                invite_id: None,
                granted_by: None,
            },
        )
        .await;
        assert!(matches!(claimed, Err(Error::Invalid { .. })), "{claimed:?}");
        drop(tx);

        // And the CHECK behind it, for a writer that never went through the
        // service.
        let mut tx = begin(&db.pool, tree.tenant).await;
        let err = sqlx::query(
            "insert into scope_grants \
             (id, tenant_id, scope_id, subject_kind, principal_id, role_key, source) \
             values ($1, $2, $3, 'principal', 'forged', 'owner', 'invite')",
        )
        .bind(GrantId::new().as_uuid())
        .bind(tree.tenant.as_uuid())
        .bind(tree.workspace_scope.as_uuid())
        .execute(&mut *tx)
        .await
        .expect_err("the CHECK refuses it");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("23514")
        );
    });
}

/// A grant is created and revoked, never edited — against direct SQL, so
/// nothing holding a connection can quietly turn a `viewer` into an `owner`
/// while `created_at` still says "since when".
#[test]
#[ignore = "serial administrator tamper acceptance"]
fn a_grant_is_never_edited_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let existing = grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Viewer,
            GrantSource::Direct,
        )
        .await;
        let administrator = tenant_fixture::administrator_pool(&db.pool).await;
        let err = sqlx::query("update scope_grants set role_key = 'owner' where id = $1")
            .bind(existing.id.as_uuid())
            .execute(&administrator)
            .await
            .expect_err("the trigger refuses every update");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("P0001"),
            "{err:?}"
        );
    });
}

/// A directory-managed grant is refused revocation here, and the refusal says
/// where to make the change.
#[test]
fn a_directory_grant_cannot_be_revoked_here() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let robin = new_identity(&db.pool, tree.tenant, "robin").await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let group = access::sync_directory_group(
            &mut tx,
            GroupId::new(),
            tree.tenant,
            "entra",
            "entra-group-eng",
            None,
            "entra-engineering",
            "Engineering",
            &[robin],
        )
        .await
        .expect("project directory group");
        tx.commit().await.expect("commit directory group");
        let managed = grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            GrantSubject::Group { group_id: group.id },
            RoleKey::Member,
            GrantSource::Directory,
        )
        .await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let result = access::revoke_grant(&mut tx, tree.tenant, managed.id).await;
        let Err(Error::Conflict { message }) = result else {
            panic!("expected a conflict naming the directory, got {result:?}");
        };
        assert!(message.contains("directory"), "{message}");
        drop(tx);

        // And the member listing says so up front, so a client never offers a
        // button the API will refuse.
        let members = members_at(&db.pool, tree.tenant, tree.workspace_scope).await;
        assert!(members[0].directory_managed);

        let mut tx = begin(&db.pool, tree.tenant).await;
        let revoked = access::revoke_directory_grant(&mut tx, tree.tenant, managed.id)
            .await
            .expect("directory surface revokes its own assignment");
        tx.commit().await.expect("commit directory revocation");
        assert_eq!(revoked.directory_source.as_deref(), Some("entra"));
        assert_eq!(
            revoked.directory_resource_id.as_deref(),
            Some("entra-group-eng")
        );
    });
}

/// Revoking removes exactly the authority it names, and the resolution agrees
/// on the very next read.
#[test]
fn revoking_a_grant_removes_what_it_conferred() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let held = grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        assert_eq!(
            members_at(&db.pool, tree.tenant, tree.project_scope)
                .await
                .len(),
            1
        );

        let mut tx = begin(&db.pool, tree.tenant).await;
        let revoked = access::revoke_grant(&mut tx, tree.tenant, held.id)
            .await
            .expect("revoke");
        tx.commit().await.expect("commit revocation");
        assert_eq!(revoked.id, held.id);

        assert!(
            members_at(&db.pool, tree.tenant, tree.project_scope)
                .await
                .is_empty(),
            "the project stops inheriting it immediately"
        );
        let again = {
            let mut tx = begin(&db.pool, tree.tenant).await;
            access::revoke_grant(&mut tx, tree.tenant, held.id).await
        };
        assert!(matches!(again, Err(Error::NotFound { .. })), "{again:?}");
    });
}

#[test]
fn a_root_administrator_grant_permanently_consumes_idp_bootstrap() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let first = grant(
            &db.pool,
            tree.tenant,
            tree.tenant_scope,
            principal("initial-admin"),
            RoleKey::Administrator,
            GrantSource::Direct,
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        assert!(
            !access::claim_initial_administrator_bootstrap(
                &mut tx,
                tree.tenant,
                GrantId::new(),
                "later-admin",
            )
            .await
            .expect("observe bootstrap claimed by trigger"),
            "a governed root administrator grant closes the IdP door"
        );
        access::revoke_grant(&mut tx, tree.tenant, first.id)
            .await
            .expect("revoke first administrator");
        tx.commit().await.expect("commit revocation");

        let mut tx = begin(&db.pool, tree.tenant).await;
        assert!(
            !access::claim_initial_administrator_bootstrap(
                &mut tx,
                tree.tenant,
                GrantId::new(),
                "later-admin",
            )
            .await
            .expect("observe persistent bootstrap marker"),
            "revocation must not return authority to the identity provider"
        );
    });
}

#[test]
fn concurrent_initial_admin_claims_are_single_winner_and_rollback_safe() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let mut winner_tx = begin(&db.pool, tree.tenant).await;
        assert!(
            claim_initial_admin(
                &mut winner_tx,
                tree.tenant,
                tree.tenant_scope,
                "first-claimant",
            )
            .await
        );
        let winner_pid = tenant_fixture::backend_pid(&mut winner_tx).await;

        let pool = db.pool.clone();
        let tenant = tree.tenant;
        let root_scope = tree.tenant_scope;
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
        let contender = tokio::spawn(async move {
            let mut tx = begin(&pool, tenant).await;
            let pid = tenant_fixture::backend_pid(&mut tx).await;
            pid_tx.send(pid).expect("report second claimant pid");
            let claimed = claim_initial_admin(&mut tx, tenant, root_scope, "second-claimant").await;
            tx.commit().await.expect("commit second claimant");
            claimed
        });
        let contender_pid = pid_rx.await.expect("receive second claimant pid");
        let mut observer = begin(&db.pool, tree.tenant).await;
        tenant_fixture::wait_until_blocked_by(&mut observer, contender_pid, winner_pid).await;
        observer
            .rollback()
            .await
            .expect("finish blocker observation");
        winner_tx.commit().await.expect("commit first claimant");
        let contender_claimed = tokio::time::timeout(std::time::Duration::from_secs(5), contender)
            .await
            .expect("second claimant must finish without deadlock")
            .expect("second claimant task");
        assert!(!contender_claimed, "only one claimant may win");
        let grants = tenant_root_grants(&db.pool, tree.tenant, tree.tenant_scope).await;
        assert_eq!(grants.len(), 1, "exactly one root grant commits");
        assert_eq!(grants[0].principal_id.as_deref(), Some("first-claimant"));

        let retry_tree = seed_tree(&db.pool).await;
        let mut abandoned_tx = begin(&db.pool, retry_tree.tenant).await;
        assert!(
            claim_initial_admin(
                &mut abandoned_tx,
                retry_tree.tenant,
                retry_tree.tenant_scope,
                "abandoned-claimant",
            )
            .await
        );
        let abandoned_pid = tenant_fixture::backend_pid(&mut abandoned_tx).await;
        let pool = db.pool.clone();
        let tenant = retry_tree.tenant;
        let root_scope = retry_tree.tenant_scope;
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
        let retry = tokio::spawn(async move {
            let mut tx = begin(&pool, tenant).await;
            let pid = tenant_fixture::backend_pid(&mut tx).await;
            pid_tx.send(pid).expect("report retry claimant pid");
            let claimed = claim_initial_admin(&mut tx, tenant, root_scope, "retry-claimant").await;
            tx.commit().await.expect("commit retry claimant");
            claimed
        });
        let retry_pid = pid_rx.await.expect("receive retry claimant pid");
        let mut observer = begin(&db.pool, retry_tree.tenant).await;
        tenant_fixture::wait_until_blocked_by(&mut observer, retry_pid, abandoned_pid).await;
        observer
            .rollback()
            .await
            .expect("finish blocker observation");
        abandoned_tx
            .rollback()
            .await
            .expect("roll back marker and grant");
        let retry_claimed = tokio::time::timeout(std::time::Duration::from_secs(5), retry)
            .await
            .expect("retry must finish without deadlock")
            .expect("retry claimant task");
        assert!(
            retry_claimed,
            "rollback returns the claim to the next waiter"
        );
        let grants = tenant_root_grants(&db.pool, retry_tree.tenant, retry_tree.tenant_scope).await;
        assert_eq!(grants.len(), 1, "only the retry grant commits");
        assert_eq!(grants[0].principal_id.as_deref(), Some("retry-claimant"));
    });
}

/// Removing a member touches only what was written here, and refuses — with
/// the place to go — when the authority is somewhere else.
#[test]
fn removing_a_member_refuses_what_it_cannot_actually_remove() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;

        // Nothing at all.
        let mut tx = begin(&db.pool, tree.tenant).await;
        let missing =
            access::remove_member(&mut tx, tree.tenant, tree.project_scope, "nobody").await;
        assert!(
            matches!(missing, Err(Error::NotFound { .. })),
            "{missing:?}"
        );
        drop(tx);

        // Inherited from the workspace: removing it at the project would leave
        // the access in place, so it is refused and the refusal names where.
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let inherited =
            access::remove_member(&mut tx, tree.tenant, tree.project_scope, "robin").await;
        let Err(Error::Conflict { message }) = inherited else {
            panic!("expected a conflict naming the scope, got {inherited:?}");
        };
        assert!(
            message.contains(&tree.workspace_scope.to_string()),
            "the refusal names where the grant actually is: {message}"
        );
        drop(tx);

        // Through a group: the refusal names the group instead.
        let group = new_group(&db.pool, tree.tenant, "eng", &["kim"]).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            GrantSubject::Group { group_id: group },
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let via_group =
            access::remove_member(&mut tx, tree.tenant, tree.project_scope, "kim").await;
        let Err(Error::Conflict { message }) = via_group else {
            panic!("expected a conflict naming the group, got {via_group:?}");
        };
        assert!(message.contains("eng"), "{message}");
        drop(tx);

        // And the case it can do: every grant written here for that principal.
        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            principal("sam"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            principal("sam"),
            RoleKey::Reviewer,
            GrantSource::Direct,
        )
        .await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let removed = access::remove_member(&mut tx, tree.tenant, tree.project_scope, "sam")
            .await
            .expect("remove");
        tx.commit().await.expect("commit removal");
        assert_eq!(removed.len(), 2, "both roles written here go");
    });
}

// ── Concurrency ─────────────────────────────────────────────────────────────

/// The principal advisory fence excludes an insert phantom while a lifecycle
/// transaction snapshots and retires that principal's existing authority.
#[test]
fn principal_grant_retirement_serializes_a_concurrent_insert() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let subject = "fenced-principal";
        let old = grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal(subject),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;

        let mut holder = begin(&db.pool, tree.tenant).await;
        access::lock_principal_grants(&mut holder, tree.tenant, subject)
            .await
            .expect("lock principal retirement predicate");
        let snapshot =
            access::principal_grants_bounded(&mut *holder, tree.tenant, subject, None, 16)
                .await
                .expect("snapshot authority under fence");
        assert_eq!(
            snapshot.iter().map(|grant| grant.id).collect::<Vec<_>>(),
            [old.id]
        );
        let holder_pid = tenant_fixture::backend_pid(&mut holder).await;

        let pool = db.pool.clone();
        let tenant = tree.tenant;
        let project_scope = tree.project_scope;
        let waiter_subject = subject.to_owned();
        let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let mut tx = begin(&pool, tenant).await;
            let pid = tenant_fixture::backend_pid(&mut tx).await;
            pid_sender.send(pid).expect("report waiter pid");
            let created = access::create_grant(
                &mut tx,
                &access::NewGrant {
                    id: GrantId::new(),
                    tenant_id: tenant,
                    scope_id: project_scope,
                    subject: principal(&waiter_subject),
                    role_key: RoleKey::Reviewer,
                    source: GrantSource::Direct,
                    invite_id: None,
                    granted_by: Some("concurrent-granter".to_owned()),
                },
            )
            .await
            .expect("create grant after retirement fence");
            tx.commit().await.expect("commit concurrent grant");
            created
        });
        let waiter_pid = pid_receiver.await.expect("receive waiter pid");
        let mut observer = begin(&db.pool, tree.tenant).await;
        tenant_fixture::wait_until_blocked_by(&mut observer, waiter_pid, holder_pid).await;
        observer
            .rollback()
            .await
            .expect("finish blocker observation");

        access::revoke_grant(&mut holder, tree.tenant, old.id)
            .await
            .expect("retire snapshotted grant");
        holder.commit().await.expect("commit retirement");
        let created = tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .expect("concurrent insert completes after fence release")
            .expect("concurrent insert task");

        let mut tx = begin(&db.pool, tree.tenant).await;
        let final_grants =
            access::principal_grants_bounded(&mut *tx, tree.tenant, subject, None, 16)
                .await
                .expect("read linearized authority");
        assert_eq!(final_grants.len(), 1);
        assert_eq!(final_grants[0].id, created.id);
        assert_eq!(final_grants[0].role_key, RoleKey::Reviewer);
        tx.commit().await.expect("commit final authority read");
    });
}

/// Principal-scope creation must wait on the principal predicate fence before
/// it locks the tenant-root parent. Otherwise an ordinary grant that already
/// owns the fence and needs an FK lock on that root forms the opposite edge of
/// a database deadlock.
#[test]
fn principal_scope_creation_locks_the_principal_before_its_parent() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let subject = "ordered-principal";
        let mut holder = begin(&db.pool, tree.tenant).await;
        access::lock_principal_grants(&mut holder, tree.tenant, subject)
            .await
            .expect("hold principal predicate fence");
        let holder_pid = tenant_fixture::backend_pid(&mut holder).await;

        let pool = db.pool.clone();
        let tenant = tree.tenant;
        let parent_scope_id = tree.tenant_scope;
        let waiter_subject = subject.to_owned();
        let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let mut tx = begin(&pool, tenant).await;
            let pid = tenant_fixture::backend_pid(&mut tx).await;
            pid_sender.send(pid).expect("report scope creator pid");
            let scope = scopes::create(
                &mut tx,
                &scopes::NewScope {
                    id: ScopeId::new(),
                    tenant_id: tenant,
                    kind: ScopeKind::Principal,
                    parent_scope_id: Some(parent_scope_id),
                    slug: scopes::principal_slug(&waiter_subject),
                    display_name: "Ordered principal".to_owned(),
                    attributes: serde_json::json!({}),
                    principal_id: Some(waiter_subject.clone()),
                    created_by: None,
                },
            )
            .await
            .expect("create principal scope after fence release");
            tx.commit().await.expect("commit principal scope");
            scope
        });
        let waiter_pid = pid_receiver.await.expect("receive scope creator pid");
        let mut observer = begin(&db.pool, tree.tenant).await;
        tenant_fixture::wait_until_blocked_by(&mut observer, waiter_pid, holder_pid).await;
        observer
            .rollback()
            .await
            .expect("finish blocker observation");

        let parent_grant = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            access::create_grant(
                &mut holder,
                &access::NewGrant {
                    id: GrantId::new(),
                    tenant_id: tree.tenant,
                    scope_id: tree.tenant_scope,
                    subject: principal(subject),
                    role_key: RoleKey::Reviewer,
                    source: GrantSource::Direct,
                    invite_id: None,
                    granted_by: Some("lock-order-probe".to_owned()),
                },
            ),
        )
        .await
        .expect("parent grant cannot wait on the blocked scope creator")
        .expect("create parent grant while holding the principal fence");
        holder.commit().await.expect("release principal fence");

        let scope = tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .expect("scope creation completes after fence release")
            .expect("scope creation task");
        assert_eq!(parent_grant.scope_id, tree.tenant_scope);
        assert_eq!(scope.parent_scope_id, Some(tree.tenant_scope));
        assert_eq!(scope.kind, ScopeKind::Principal);
        assert_eq!(scope.principal_id.as_deref(), Some(subject));
    });
}

/// The principal fence also precedes minting a missing tenant root. Workspace
/// creation takes that fence before it can mint the same root, so reversing
/// those two operations would deadlock on a tenant's first structural write.
#[test]
fn principal_scope_creation_locks_the_principal_before_a_missing_root() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let subject = "fresh-root-owner";
        let mut holder = begin(&db.pool, tenant).await;
        access::lock_principal_grants(&mut holder, tenant, subject)
            .await
            .expect("hold workspace owner principal fence");
        let holder_pid = tenant_fixture::backend_pid(&mut holder).await;

        let pool = db.pool.clone();
        let waiter_subject = subject.to_owned();
        let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let mut tx = begin(&pool, tenant).await;
            let pid = tenant_fixture::backend_pid(&mut tx).await;
            pid_sender.send(pid).expect("report principal creator pid");
            let scope = scopes::ensure_principal_scope(
                &mut tx,
                tenant,
                &waiter_subject,
                "Fresh root owner",
            )
            .await
            .expect("create principal scope after fence release");
            tx.commit().await.expect("commit principal scope");
            scope
        });
        let waiter_pid = pid_receiver.await.expect("receive principal creator pid");
        let mut observer = begin(&db.pool, tenant).await;
        tenant_fixture::wait_until_blocked_by(&mut observer, waiter_pid, holder_pid).await;
        observer
            .rollback()
            .await
            .expect("finish blocker observation");

        let (workspace, owner, root_id) =
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                let workspace = workspaces::create(
                    &mut holder,
                    &workspaces::NewWorkspace {
                        id: WorkspaceId::new(),
                        tenant_id: tenant,
                        slug: "first-workspace".to_owned(),
                        display_name: "First workspace".to_owned(),
                        description: None,
                        created_by: None,
                    },
                )
                .await
                .expect("workspace owner can mint the missing root");
                let owner = access::create_grant(
                    &mut holder,
                    &access::NewGrant {
                        id: GrantId::new(),
                        tenant_id: tenant,
                        scope_id: workspace.scope_id,
                        subject: principal(subject),
                        role_key: RoleKey::Owner,
                        source: GrantSource::Owner,
                        invite_id: None,
                        granted_by: None,
                    },
                )
                .await
                .expect("mint workspace owner grant under the held fence");
                let root_id = scopes::tenant_root(&mut *holder, tenant)
                    .await
                    .expect("read minted tenant root")
                    .expect("workspace minted tenant root")
                    .id;
                (workspace, owner, root_id)
            })
            .await
            .expect("workspace creation cannot wait on the blocked principal creator");
        holder.commit().await.expect("release principal fence");

        let principal_scope = tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
            .await
            .expect("principal creation completes after fence release")
            .expect("principal creation task");
        assert_eq!(owner.scope_id, workspace.scope_id);
        assert_eq!(principal_scope.parent_scope_id, Some(root_id));
        assert_eq!(principal_scope.kind, ScopeKind::Principal);
        assert_eq!(principal_scope.principal_id.as_deref(), Some(subject));

        let mut tx = begin(&db.pool, tenant).await;
        let owner_grants = access::structural_owner_grants(&mut *tx, tenant, principal_scope.id)
            .await
            .expect("read principal structural owner grant");
        assert_eq!(owner_grants.len(), 1);
        assert_eq!(owner_grants[0].principal_id.as_deref(), Some(subject));
        tx.commit().await.expect("commit owner grant read");
    });
}

// ── Invitations ──────────────────────────────────────────────────────────────

async fn invite(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    hash: [u8; 32],
    ttl: Duration,
) -> InviteId {
    let mut tx = begin(pool, tenant).await;
    let invite = access::create_invite(
        &mut *tx,
        &access::NewInvite {
            id: InviteId::new(),
            tenant_id: tenant,
            scope_id: scope,
            role_key: RoleKey::Member,
            email: Some("sam@example.com".to_owned()),
            token_hash: hash,
            expires_at: Utc::now() + ttl,
            created_by: Some("granter".to_owned()),
        },
    )
    .await
    .expect("create invite");
    tx.commit().await.expect("commit invite");
    invite.id
}

/// Redeeming an invitation mints the grant it carries, with the provenance
/// that says where it came from — and the invitation is terminal afterwards.
#[test]
fn redeeming_an_invitation_mints_a_grant_that_says_where_it_came_from() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let hash = [1u8; 32];
        let id = invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            hash,
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let accepted = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", Utc::now())
            .await
            .expect("redeem");
        tx.commit().await.expect("commit");

        assert!(!accepted.replayed);
        assert_eq!(accepted.invite.status, InviteStatus::Accepted);
        assert_eq!(accepted.invite.accepted_by.as_deref(), Some("robin"));
        assert_eq!(accepted.grant.source, GrantSource::Invite);
        assert_eq!(accepted.grant.invite_id, Some(id));
        assert_eq!(accepted.grant.subject_kind, SubjectKind::Principal);
        assert_eq!(accepted.grant.principal_id.as_deref(), Some("robin"));

        let members = members_at(&db.pool, tree.tenant, tree.project_scope).await;
        assert_eq!(members.len(), 1, "and it reaches the project immediately");
        assert_eq!(members[0].source, GrantSource::Invite);
    });
}

/// One-time: the same token redeemed by somebody else is refused, and by the
/// same principal is a replay rather than a punishment for a network retry.
#[test]
fn an_invitation_is_one_time_and_a_retry_is_not_a_second_redemption() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let hash = [2u8; 32];
        invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            hash,
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let first = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", Utc::now())
            .await
            .expect("first redemption");
        tx.commit().await.expect("commit");

        let mut tx = begin(&db.pool, tree.tenant).await;
        let replay = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", Utc::now())
            .await
            .expect("the same principal replays");
        tx.commit().await.expect("commit");
        assert!(replay.replayed);
        assert_eq!(
            replay.grant.id, first.grant.id,
            "the same grant, not a second"
        );

        let mut tx = begin(&db.pool, tree.tenant).await;
        let stolen =
            access::accept_invite(&mut tx, tree.tenant, &hash, "intruder", Utc::now()).await;
        let Err(Error::Conflict { message }) = stolen else {
            panic!("expected a conflict, got {stolen:?}");
        };
        assert!(message.contains("already been accepted"), "{message}");
        drop(tx);

        let mut tx = begin(&db.pool, tree.tenant).await;
        assert_eq!(
            access::list_grants(&mut *tx, tree.tenant, &access::GrantFilter::default())
                .await
                .expect("list")
                .len(),
            1,
            "one invitation minted one grant, whatever was retried"
        );
        tx.commit().await.expect("commit replay read");
    });
}

/// Expiry is a property of the decision rather than of a job: an invitation
/// stops working at the instant it says it will, with nothing having run.
#[test]
fn an_invitation_expires_without_anything_running() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let hash = [3u8; 32];
        let id = invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            hash,
            Duration::seconds(1),
        )
        .await;

        // No sweep, no job: just a later instant.
        let later = Utc::now() + Duration::days(1);
        let mut tx = begin(&db.pool, tree.tenant).await;
        let result = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", later).await;
        let Err(Error::Conflict { message }) = result else {
            panic!("expected an expiry conflict, got {result:?}");
        };
        assert!(message.contains("expired"), "{message}");
        drop(tx);

        // And the stored status is still `pending` — `expired` is derived, so
        // the row does not lie about what happened to it.
        let mut tx = begin(&db.pool, tree.tenant).await;
        let stored = access::get_invite(&mut *tx, tree.tenant, id)
            .await
            .expect("read")
            .expect("still there");
        assert_eq!(stored.status, InviteStatus::Pending);
        assert_eq!(stored.effective_status(later), InviteStatus::Expired);
        tx.commit().await.expect("commit expired-invite read");
    });
}

/// Withdrawing an invitation ends it, and neither terminal state can be
/// reopened — including against direct SQL.
#[test]
#[ignore = "serial administrator tamper acceptance"]
fn a_terminal_invitation_cannot_be_reopened() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let hash = [4u8; 32];
        let id = invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            hash,
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let withdrawn = access::revoke_invite(&mut tx, tree.tenant, id, Some("granter"))
            .await
            .expect("withdraw");
        tx.commit().await.expect("commit");
        assert_eq!(withdrawn.status, InviteStatus::Revoked);

        let mut tx = begin(&db.pool, tree.tenant).await;
        let redeem = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", Utc::now()).await;
        let Err(Error::Conflict { message }) = redeem else {
            panic!("expected a conflict, got {redeem:?}");
        };
        assert!(message.contains("withdrawn"), "{message}");
        drop(tx);

        let mut tx = begin(&db.pool, tree.tenant).await;
        let twice = access::revoke_invite(&mut tx, tree.tenant, id, Some("granter")).await;
        assert!(matches!(twice, Err(Error::Conflict { .. })), "{twice:?}");
        drop(tx);

        let administrator = tenant_fixture::administrator_pool(&db.pool).await;
        let err = sqlx::query("update pending_invites set status = 'pending' where id = $1")
            .bind(id.as_uuid())
            .execute(&administrator)
            .await
            .expect_err("the trigger refuses a reopened invitation");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("P0001"),
            "{err:?}"
        );
    });
}

/// An invitation's terms are immutable: re-pointing one at a different scope or
/// a fatter role is issuing another, not editing this one.
#[test]
#[ignore = "serial administrator tamper acceptance"]
fn an_invitations_terms_are_immutable_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let id = invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            [5u8; 32],
            Duration::days(7),
        )
        .await;
        let administrator = tenant_fixture::administrator_pool(&db.pool).await;
        for (statement, what) in [
            ("update pending_invites set role_key = 'owner' where id = $1", "the role"),
            ("update pending_invites set scope_id = scope_id where id = $1 and false", "a no-op"),
            ("update pending_invites set expires_at = now() + interval '1 year' where id = $1", "the window"),
            ("update pending_invites set token_hash = decode(repeat('00', 32), 'hex') where id = $1", "the token"),
        ] {
            let result = sqlx::query(statement)
                .bind(id.as_uuid())
                .execute(&administrator)
                .await;
            match result {
                // The no-op statement matches no row; the rest must be refused.
                Ok(done) => assert_eq!(done.rows_affected(), 0, "{what} was allowed"),
                Err(err) => assert_eq!(
                    err.as_database_error().and_then(|db| db.code()).as_deref(),
                    Some("P0001"),
                    "{what}: {err:?}"
                ),
            }
        }
    });
}

/// An invitation that expired before it was created is a row nothing can ever
/// redeem; it is refused rather than left there looking like an invitation.
#[test]
fn an_invitation_cannot_be_born_expired() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let mut tx = begin(&db.pool, tree.tenant).await;
        let result = access::create_invite(
            &mut *tx,
            &access::NewInvite {
                id: InviteId::new(),
                tenant_id: tree.tenant,
                scope_id: tree.workspace_scope,
                role_key: RoleKey::Member,
                email: None,
                token_hash: [6u8; 32],
                expires_at: Utc::now() - Duration::days(1),
                created_by: None,
            },
        )
        .await;
        assert!(matches!(result, Err(Error::Invalid { .. })), "{result:?}");
    });
}

/// Redeeming an invitation for access somebody already holds consumes the
/// invitation and hands back the grant they have — the alternative is an error
/// for a person who did nothing wrong.
#[test]
fn redeeming_for_access_already_held_consumes_the_invitation_and_conflicts_with_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        let existing = grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        let hash = [7u8; 32];
        invite(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            hash,
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let accepted = access::accept_invite(&mut tx, tree.tenant, &hash, "robin", Utc::now())
            .await
            .expect("redeem");
        tx.commit().await.expect("commit");
        assert_eq!(accepted.grant.id, existing.id, "the grant they already had");
        assert_eq!(accepted.invite.status, InviteStatus::Accepted);
        let mut tx = begin(&db.pool, tree.tenant).await;
        assert_eq!(
            access::list_grants(&mut *tx, tree.tenant, &access::GrantFilter::default())
                .await
                .expect("list")
                .len(),
            1,
            "and no duplicate row"
        );
        tx.commit().await.expect("commit existing-grant read");
    });
}

/// A token that hashes to nothing is *not found*, and the refusal does not say
/// whether it never existed, belonged to another tenant, or was deleted — a
/// distinguishable refusal is an oracle for guessing tokens.
#[test]
fn an_unknown_token_is_indistinguishable_from_a_foreign_one() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed_tree(&db.pool).await;
        let theirs = seed_tree(&db.pool).await;
        let hash = [8u8; 32];
        invite(
            &db.pool,
            theirs.tenant,
            theirs.workspace_scope,
            hash,
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, mine.tenant).await;
        let foreign = access::accept_invite(&mut tx, mine.tenant, &hash, "robin", Utc::now()).await;
        drop(tx);
        let mut tx = begin(&db.pool, mine.tenant).await;
        let unknown =
            access::accept_invite(&mut tx, mine.tenant, &[99u8; 32], "robin", Utc::now()).await;
        drop(tx);

        let (Err(Error::NotFound { entity: a }), Err(Error::NotFound { entity: b })) =
            (foreign, unknown)
        else {
            panic!("both must be a plain not-found");
        };
        assert_eq!(a, b, "the two refusals must be identical: {a} vs {b}");
    });
}

// ── Tenancy ──────────────────────────────────────────────────────────────────

/// Another tenant's group, grant, invitation and scope are **absent** rather
/// than forbidden, on every read this module has.
#[test]
fn another_tenants_rows_are_absent_on_every_surface() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed_tree(&db.pool).await;
        let theirs = seed_tree(&db.pool).await;
        let their_group = new_group(&db.pool, theirs.tenant, "eng", &["robin"]).await;
        let their_grant = grant(
            &db.pool,
            theirs.tenant,
            theirs.workspace_scope,
            principal("robin"),
            RoleKey::Owner,
            GrantSource::Direct,
        )
        .await;
        let their_invite = invite(
            &db.pool,
            theirs.tenant,
            theirs.workspace_scope,
            [10u8; 32],
            Duration::days(7),
        )
        .await;

        let mut tx = begin(&db.pool, mine.tenant).await;
        assert!(
            access::get_group(&mut *tx, mine.tenant, their_group)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            access::get_grant(&mut *tx, mine.tenant, their_grant.id)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            access::get_invite(&mut *tx, mine.tenant, their_invite)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            access::list_groups(&mut *tx, mine.tenant)
                .await
                .expect("list")
                .is_empty()
        );
        assert!(
            access::list_grants(&mut *tx, mine.tenant, &access::GrantFilter::default())
                .await
                .expect("list")
                .is_empty()
        );
        assert!(
            access::list_invites(&mut *tx, mine.tenant, theirs.workspace_scope)
                .await
                .expect("list")
                .is_empty(),
            "another tenant's scope resolves to no invitations rather than theirs"
        );
        assert!(
            access::members_of(&mut *tx, mine.tenant, theirs.workspace_scope)
                .await
                .expect("resolve foreign members")
                .is_empty(),
            "and to nobody"
        );
        assert!(
            access::group_members(&mut *tx, mine.tenant, their_group)
                .await
                .expect("read")
                .is_empty()
        );
        tx.commit().await.expect("commit cross-tenant reads");

        // The same slug in two tenants is two groups, not a collision.
        new_group(&db.pool, mine.tenant, "eng", &["kim"]).await;
        let mut tx = begin(&db.pool, mine.tenant).await;
        assert_eq!(
            access::list_groups(&mut *tx, mine.tenant)
                .await
                .expect("list")
                .len(),
            1
        );
        tx.commit().await.expect("commit same-slug read");
    });
}

/// The filters select what they say and nothing else: a scope filter lists the
/// rows written **at** that scope, not the authority in force there.
#[test]
fn the_grant_filters_select_rows_rather_than_authority() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tree = seed_tree(&db.pool).await;
        grant(
            &db.pool,
            tree.tenant,
            tree.workspace_scope,
            principal("robin"),
            RoleKey::Member,
            GrantSource::Direct,
        )
        .await;
        grant(
            &db.pool,
            tree.tenant,
            tree.project_scope,
            principal("kim"),
            RoleKey::Viewer,
            GrantSource::Direct,
        )
        .await;

        let mut tx = begin(&db.pool, tree.tenant).await;
        let at_project = access::list_grants(
            &mut *tx,
            tree.tenant,
            &access::GrantFilter {
                scope_id: Some(tree.project_scope),
                principal_id: None,
            },
        )
        .await
        .expect("list");
        assert_eq!(
            at_project.len(),
            1,
            "the row written there, not the two in force"
        );
        assert_eq!(at_project[0].principal_id.as_deref(), Some("kim"));

        let robins = access::list_grants(
            &mut *tx,
            tree.tenant,
            &access::GrantFilter {
                scope_id: None,
                principal_id: Some("robin".to_owned()),
            },
        )
        .await
        .expect("list");
        assert_eq!(robins.len(), 1);

        assert_eq!(
            access::list_grants(&mut *tx, tree.tenant, &access::GrantFilter::default())
                .await
                .expect("list")
                .len(),
            2,
            "and no filter is the tenant's grants"
        );
        tx.commit().await.expect("commit grant-filter reads");
    });
}

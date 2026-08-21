//! CPR-4 acceptance criteria at the storage layer: workspaces, projects and
//! repository attachments as product-level subtypes of a governed scope
//! (ADR-0071).
//!
//! What is asserted here, in the order the feature states it: a subtype and its
//! scope are created in one transaction and a failure leaves **neither**; the
//! scope tree the subtypes produce is the one the model claims (a workspace
//! under the tenant root, a project under its workspace); the structural rules
//! that are database facts hold against direct SQL as well as against the
//! services; revision preconditions refuse a lost update; canonical repository
//! identity collapses the ways one repository can be written and refuses a
//! path; and every read is tenant-filtered so another tenant's row is absent
//! rather than forbidden.
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`. Isolation is by freshly minted UUIDv7
//! tenants, so a shared dev database is fine.

use std::sync::OnceLock;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{idempotency, projects, repositories, scopes, tenants, workspaces};
use synveda_types::repository::{self, RepositoryProvider};
use synveda_types::scope::ScopeKind;
use synveda_types::workspace::LifecycleStatus;
use synveda_types::{Error, ProjectId, Result, ScopeId, TenantId, TenantStatus, WorkspaceId};

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
                    "skipping workspace tests: DATABASE_URL is not set \
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
            synveda_store::migrate(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

/// Admits a tenant. Its slug becomes the tenant root scope's slug when the
/// first workspace mints one.
async fn admit(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("wsp-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "CPR-4 fixture", TenantStatus::Active)
        .await
        .expect("admit tenant");
    tenant
}

async fn begin(pool: &PgPool) -> Transaction<'static, Postgres> {
    pool.begin().await.expect("begin transaction")
}

async fn new_workspace(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    slug: &str,
) -> Result<synveda_types::workspace::Workspace> {
    workspaces::create(
        tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
}

async fn new_project(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
    workspace: WorkspaceId,
    slug: &str,
) -> Result<synveda_types::workspace::Project> {
    projects::create(
        tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: workspace,
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
}

/// Counts scopes of a kind in a tenant. The orphan checks read this.
async fn scope_count(pool: &PgPool, tenant: TenantId, kind: ScopeKind) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from scopes where tenant_id = $1 and kind = $2"#,
        tenant.as_uuid(),
        kind.as_str(),
    )
    .fetch_one(pool)
    .await
    .expect("count scopes")
}

async fn workspace_count(pool: &PgPool, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from workspaces where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(pool)
    .await
    .expect("count workspaces")
}

// ── Creation, and the scope it mints ─────────────────────────────────────────

/// The shape the model claims: a tenant root minted on the way past, a
/// workspace scope under it, and a project scope under the workspace's.
#[test]
fn a_workspace_and_a_project_mint_the_scopes_the_model_claims() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        assert!(
            scopes::tenant_root(&db.pool, tenant)
                .await
                .expect("read root")
                .is_none(),
            "a fresh tenant has no scope tree: nobody has been asked to declare one"
        );

        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("create workspace");
        let project = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("create project");
        tx.commit().await.expect("commit");

        let root = scopes::tenant_root(&db.pool, tenant)
            .await
            .expect("read root")
            .expect("the first workspace minted the tenant root");
        assert_eq!(root.kind, ScopeKind::Tenant);

        let workspace_scope = scopes::get(&db.pool, tenant, workspace.scope_id)
            .await
            .expect("read scope")
            .expect("the workspace's scope exists");
        assert_eq!(workspace_scope.kind, ScopeKind::Workspace);
        assert_eq!(workspace_scope.parent_scope_id, Some(root.id));
        assert_eq!(
            workspace_scope.slug, workspace.slug,
            "the workspace and its scope are one name, held together by a foreign key"
        );

        let project_scope = scopes::get(&db.pool, tenant, project.scope_id)
            .await
            .expect("read scope")
            .expect("the project's scope exists");
        assert_eq!(project_scope.kind, ScopeKind::Project);
        assert_eq!(
            project_scope.parent_scope_id,
            Some(workspace.scope_id),
            "a project's scope sits under its workspace's"
        );

        assert_eq!(
            scopes::path(&db.pool, tenant, project.scope_id)
                .await
                .expect("path"),
            Some(format!("{}/payments/ledger", root.slug)),
            "the scope path reads as the product nouns do"
        );
    });
}

/// **Failure leaves neither.** A workspace whose slug is taken fails after its
/// scope insert, and the transaction takes both away — there is no
/// compensating delete and there must not be one.
#[test]
fn a_failed_creation_leaves_no_orphan_scope_and_no_orphan_subtype() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("first workspace");
        tx.commit().await.expect("commit");

        let before_scopes = scope_count(&db.pool, tenant, ScopeKind::Workspace).await;
        let before_rows = workspace_count(&db.pool, tenant).await;

        let mut tx = begin(&db.pool).await;
        let error = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect_err("a duplicate slug is refused");
        assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
        tx.rollback().await.expect("rollback");

        assert_eq!(
            scope_count(&db.pool, tenant, ScopeKind::Workspace).await,
            before_scopes,
            "a refused workspace must not leave its scope behind"
        );
        assert_eq!(workspace_count(&db.pool, tenant).await, before_rows);
    });
}

/// The same property one level down, and through the *other* failure mode: the
/// project row's own unique index, which fires after its scope insert.
#[test]
fn a_failed_project_leaves_no_orphan_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("project");
        tx.commit().await.expect("commit");

        let before = scope_count(&db.pool, tenant, ScopeKind::Project).await;
        let mut tx = begin(&db.pool).await;
        let error = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect_err("a duplicate slug is refused");
        assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
        tx.rollback().await.expect("rollback");

        assert_eq!(
            scope_count(&db.pool, tenant, ScopeKind::Project).await,
            before,
            "a refused project must not leave its scope behind"
        );
    });
}

/// A project slug is unique inside its workspace and free across workspaces —
/// which is exactly what the scope tree says, so the two agree.
#[test]
fn project_slugs_are_scoped_to_their_workspace() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let one = new_workspace(&mut tx, tenant, "one").await.expect("one");
        let two = new_workspace(&mut tx, tenant, "two").await.expect("two");
        new_project(&mut tx, tenant, one.id, "ledger")
            .await
            .expect("ledger in one");
        new_project(&mut tx, tenant, two.id, "ledger")
            .await
            .expect("ledger in two is a different project");
        tx.commit().await.expect("commit");

        assert_eq!(
            projects::in_workspace(&db.pool, tenant, one.id)
                .await
                .expect("list")
                .len(),
            1
        );
        assert_eq!(
            projects::list(&db.pool, tenant).await.expect("list").len(),
            2
        );
    });
}

/// An archived workspace is retired, and a retirement that still accepted new
/// work would be advisory.
#[test]
fn an_archived_workspace_takes_no_new_projects() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate {
                status: Some(LifecycleStatus::Archived),
                ..Default::default()
            },
        )
        .await
        .expect("archive");
        let error = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect_err("an archived workspace takes no projects");
        assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    });
}

/// A status change is mirrored onto the owned scope in the same transaction: an
/// archived workspace whose scope still read `active` would compose, resolve
/// and accept writes exactly as before.
#[test]
fn archiving_a_subtype_archives_the_scope_it_owns() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        let project = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("project");
        projects::update(
            &mut tx,
            tenant,
            project.id,
            1,
            &projects::ProjectUpdate {
                status: Some(LifecycleStatus::Archived),
                ..Default::default()
            },
        )
        .await
        .expect("archive the project");
        tx.commit().await.expect("commit");

        let scope = scopes::get(&db.pool, tenant, project.scope_id)
            .await
            .expect("read scope")
            .expect("scope");
        assert_eq!(scope.status, synveda_types::scope::ScopeStatus::Archived);

        // And back again — a retirement that could not be undone would be a
        // delete with a nicer name.
        let mut tx = begin(&db.pool).await;
        projects::update(
            &mut tx,
            tenant,
            project.id,
            2,
            &projects::ProjectUpdate {
                status: Some(LifecycleStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("restore");
        tx.commit().await.expect("commit");
        let scope = scopes::get(&db.pool, tenant, project.scope_id)
            .await
            .expect("read scope")
            .expect("scope");
        assert_eq!(scope.status, synveda_types::scope::ScopeStatus::Active);
    });
}

// ── Revision preconditions ───────────────────────────────────────────────────

/// The lost-update protection, from both ends: the stale writer is refused and
/// nothing it sent is applied.
#[test]
fn a_stale_revision_is_refused_and_writes_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        tx.commit().await.expect("commit");
        assert_eq!(workspace.revision, 1, "a fresh subtype is at revision 1");

        // Two readers see revision 1. The first writes.
        let mut tx = begin(&db.pool).await;
        let first = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate {
                display_name: Some("Payments platform".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("the first writer wins");
        tx.commit().await.expect("commit");
        assert_eq!(first.revision, 2);

        // The second still holds revision 1.
        let mut tx = begin(&db.pool).await;
        let error = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate {
                display_name: Some("Payments (old)".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect_err("a stale precondition is refused");
        let Error::Conflict { message } = &error else {
            panic!("expected Conflict, got {error:?}");
        };
        assert!(
            message.contains("revision 2"),
            "the refusal says what the revision actually is: {message}"
        );
        tx.rollback().await.expect("rollback");

        let current = workspaces::get(&db.pool, tenant, workspace.id)
            .await
            .expect("read")
            .expect("workspace");
        assert_eq!(current.display_name, "Payments platform");
        assert_eq!(
            current.revision, 2,
            "a refused update does not bump anything"
        );
    });
}

/// A revision precondition on a workspace that is not this tenant's is a 404,
/// never a revision oracle.
#[test]
fn another_tenants_subtype_is_absent_rather_than_conflicting() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = admit(&db.pool).await;
        let theirs = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, theirs, "payments")
            .await
            .expect("their workspace");
        let project = new_project(&mut tx, theirs, workspace.id, "ledger")
            .await
            .expect("their project");
        tx.commit().await.expect("commit");

        assert!(
            workspaces::get(&db.pool, mine, workspace.id)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            projects::get(&db.pool, mine, project.id)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            workspaces::list(&db.pool, mine)
                .await
                .expect("list")
                .is_empty()
        );
        assert!(
            projects::in_workspace(&db.pool, mine, workspace.id)
                .await
                .expect("list")
                .is_empty()
        );
        assert!(
            repositories::for_project(&db.pool, mine, project.id)
                .await
                .expect("list")
                .is_empty()
        );

        let mut tx = begin(&db.pool).await;
        let error = workspaces::update(
            &mut tx,
            mine,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate {
                display_name: Some("Mine now".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect_err("another tenant's workspace is not found");
        assert!(
            matches!(error, Error::NotFound { .. }),
            "a foreign workspace must be absent, not a conflict: {error:?}"
        );
    });
}

/// An empty update is a client bug, and a 200 with a bumped revision would
/// hide it.
#[test]
fn an_empty_update_is_refused() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        let error = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate::default(),
        )
        .await
        .expect_err("nothing to update");
        assert!(matches!(error, Error::Invalid { .. }), "{error:?}");
    });
}

/// Clearing a description and leaving it alone are different requests, and the
/// store can express both.
#[test]
fn a_description_can_be_set_cleared_and_left_alone() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = workspaces::create(
            &mut tx,
            &workspaces::NewWorkspace {
                id: WorkspaceId::new(),
                tenant_id: tenant,
                slug: "payments".to_owned(),
                display_name: "Payments".to_owned(),
                description: Some("What the payments team knows".to_owned()),
                created_by: None,
            },
        )
        .await
        .expect("workspace");
        assert!(workspace.description.is_some());

        let untouched = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            1,
            &workspaces::WorkspaceUpdate {
                display_name: Some("Payments platform".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("rename");
        assert_eq!(
            untouched.description.as_deref(),
            Some("What the payments team knows"),
            "an absent description leaves it alone"
        );

        let cleared = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            2,
            &workspaces::WorkspaceUpdate {
                description: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("clear");
        assert_eq!(cleared.description, None, "Some(None) clears it");

        let error = workspaces::update(
            &mut tx,
            tenant,
            workspace.id,
            3,
            &workspaces::WorkspaceUpdate {
                description: Some(Some("   ".to_owned())),
                ..Default::default()
            },
        )
        .await
        .expect_err("a blank description is refused rather than stored");
        assert!(matches!(error, Error::Invalid { .. }), "{error:?}");
    });
}

// ── Structural rules, asserted against direct SQL ────────────────────────────

/// The rules ADR-0071 makes database facts hold for anything holding a
/// connection, not only for callers who went through the services.
#[test]
fn the_structural_rules_hold_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        let project = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("project");
        tx.commit().await.expect("commit");

        // A workspace cannot point at a scope of the wrong shape: the composite
        // key has no referent for (tenant, project scope, 'workspace', slug).
        let mut tx = begin(&db.pool).await;
        let wrong_shape = sqlx::query(
            "insert into workspaces (id, tenant_id, scope_id, scope_kind, slug, display_name)
             values ($1, $2, $3, 'workspace', $4, 'Forged')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(tenant.as_uuid())
        .bind(project.scope_id.as_uuid())
        .bind("forged")
        .execute(&mut *tx)
        .await;
        assert!(
            wrong_shape.is_err(),
            "a workspace must not be able to own a project-shaped scope"
        );
        tx.rollback().await.expect("rollback");

        // The slug and the scope's slug are one name.
        let mut tx = begin(&db.pool).await;
        let disagreeing_slug = sqlx::query(
            "insert into workspaces (id, tenant_id, scope_id, scope_kind, slug, display_name)
             values ($1, $2, $3, 'workspace', 'something-else', 'Forged')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(tenant.as_uuid())
        .bind(workspace.scope_id.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(
            disagreeing_slug.is_err(),
            "a workspace's slug and its scope's slug cannot disagree"
        );
        tx.rollback().await.expect("rollback");

        // A revision cannot be rewound or skipped — the precondition would be
        // worth nothing if it could.
        for revision in [1_i64, 3, 7] {
            let mut tx = begin(&db.pool).await;
            let result = sqlx::query("update workspaces set revision = $2 where id = $1")
                .bind(workspace.id.as_uuid())
                .bind(revision)
                .execute(&mut *tx)
                .await;
            assert!(
                result.is_err(),
                "revision {revision} was accepted; revisions step forward by one"
            );
            tx.rollback().await.expect("rollback");
        }

        // A project never moves between workspaces.
        let mut tx = begin(&db.pool).await;
        let other = new_workspace(&mut tx, tenant, "other")
            .await
            .expect("second workspace");
        let moved = sqlx::query(
            "update projects set workspace_id = $2, workspace_scope_id = $3, revision = revision + 1
             where id = $1",
        )
        .bind(project.id.as_uuid())
        .bind(other.id.as_uuid())
        .bind(other.scope_id.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(
            moved.is_err(),
            "a project must not be able to move between workspaces"
        );
        tx.rollback().await.expect("rollback");
    });
}

/// A project's scope cannot be moved out from under its workspace — the
/// referential fact that keeps the subtype graph and the scope tree agreeing.
#[test]
fn a_projects_scope_cannot_leave_its_workspace() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let one = new_workspace(&mut tx, tenant, "one").await.expect("one");
        let two = new_workspace(&mut tx, tenant, "two").await.expect("two");
        let project = new_project(&mut tx, tenant, one.id, "ledger")
            .await
            .expect("project");
        tx.commit().await.expect("commit");

        let mut tx = begin(&db.pool).await;
        let moved = scopes::move_scope(&mut tx, tenant, project.scope_id, two.scope_id).await;
        assert!(
            moved.is_err(),
            "moving a project's scope under another workspace must be refused: {moved:?}"
        );
        tx.rollback().await.expect("rollback");

        let scope = scopes::get(&db.pool, tenant, project.scope_id)
            .await
            .expect("read")
            .expect("scope");
        assert_eq!(scope.parent_scope_id, Some(one.scope_id));
    });
}

// ── Repositories ─────────────────────────────────────────────────────────────

async fn attach(
    pool: &PgPool,
    tenant: TenantId,
    project: ProjectId,
    remote: Option<&str>,
    fingerprint: Option<&str>,
    name: Option<&str>,
) -> Result<synveda_types::repository::ProjectRepository> {
    let identity = repository::identify(remote, fingerprint, name, None)?;
    repositories::attach(
        pool,
        &repositories::NewRepository {
            id: synveda_types::RepositoryId::new(),
            tenant_id: tenant,
            project_id: project,
            identity,
            default_branch: Some("main".to_owned()),
            metadata: serde_json::json!({}),
            created_by: None,
        },
    )
    .await
}

async fn seeded_project(pool: &PgPool) -> (TenantId, ProjectId) {
    let tenant = admit(pool).await;
    let mut tx = begin(pool).await;
    let workspace = new_workspace(&mut tx, tenant, "payments")
        .await
        .expect("workspace");
    let project = new_project(&mut tx, tenant, workspace.id, "ledger")
        .await
        .expect("project");
    tx.commit().await.expect("commit");
    (tenant, project.id)
}

/// The property the identity exists for: two clients describing one repository
/// differently attach the same repository, and the second is a conflict rather
/// than a duplicate row.
#[test]
fn one_repository_written_two_ways_is_one_attachment() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, project) = seeded_project(&db.pool).await;
        let first = attach(
            &db.pool,
            tenant,
            project,
            Some("git@github.com:Acme/payments.git"),
            None,
            None,
        )
        .await
        .expect("attach");
        assert_eq!(first.canonical_uri, "https://github.com/Acme/payments");
        assert_eq!(first.provider, RepositoryProvider::GitHub);
        assert_eq!(first.repository_owner.as_deref(), Some("Acme"));
        assert_eq!(first.repository_name, "payments");

        for other in [
            "https://github.com/Acme/payments",
            "ssh://git@github.com/Acme/payments.git",
            // Case differs, and the unique index is case-insensitive.
            "https://github.com/acme/payments",
        ] {
            let error = attach(&db.pool, tenant, project, Some(other), None, None)
                .await
                .expect_err("the same repository twice is a conflict");
            assert!(
                matches!(error, Error::Conflict { .. }),
                "{other}: {error:?}"
            );
        }

        assert_eq!(
            repositories::for_project(&db.pool, tenant, project)
                .await
                .expect("list")
                .len(),
            1
        );
    });
}

/// A filesystem path never reaches the database, and the refusal is the type
/// layer's rather than a constraint's — so the caller gets a sentence.
#[test]
fn a_filesystem_path_is_refused_before_it_reaches_a_row() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, project) = seeded_project(&db.pool).await;
        let error = attach(
            &db.pool,
            tenant,
            project,
            Some("/Users/sam/src/payments"),
            None,
            None,
        )
        .await
        .expect_err("a path is not an identity");
        let Error::Invalid { message } = &error else {
            panic!("expected Invalid, got {error:?}");
        };
        assert!(message.contains("local_fingerprint"), "{message}");
        assert!(
            repositories::for_project(&db.pool, tenant, project)
                .await
                .expect("list")
                .is_empty()
        );

        // And the constraint behind it: a row that reached the table another
        // way still cannot hold a path.
        let mut tx = begin(&db.pool).await;
        let forged = sqlx::query(
            "insert into project_repositories
                (id, tenant_id, project_id, provider, canonical_uri, repository_name)
             values ($1, $2, $3, 'generic_git', '/Users/sam/src/payments', 'payments')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(tenant.as_uuid())
        .bind(project.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "the CHECK must refuse a path even when the service is bypassed"
        );
    });
}

/// A repository with no remote is identified by its fingerprint, and the row
/// says so in both columns.
#[test]
fn a_local_repository_is_identified_by_its_fingerprint() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, project) = seeded_project(&db.pool).await;
        let oid = "d".repeat(40);
        let attached = attach(
            &db.pool,
            tenant,
            project,
            None,
            Some(&oid),
            Some("payments"),
        )
        .await
        .expect("attach");
        assert_eq!(attached.provider, RepositoryProvider::Local);
        assert_eq!(attached.canonical_uri, format!("git+fingerprint:{oid}"));
        assert_eq!(attached.local_fingerprint.as_deref(), Some(oid.as_str()));

        // The same checkout reported twice is one attachment.
        let error = attach(
            &db.pool,
            tenant,
            project,
            None,
            Some(&oid.to_uppercase()),
            Some("payments"),
        )
        .await
        .expect_err("the same fingerprint twice is a conflict");
        assert!(matches!(error, Error::Conflict { .. }), "{error:?}");
    });
}

/// Detaching is the API's own verb; a repeated detach reports that there was
/// nothing to detach rather than succeeding silently.
#[test]
fn detaching_is_idempotent_in_effect_and_honest_about_it() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, project) = seeded_project(&db.pool).await;
        let attached = attach(
            &db.pool,
            tenant,
            project,
            Some("https://github.com/acme/payments"),
            None,
            None,
        )
        .await
        .expect("attach");
        assert!(
            repositories::detach(&db.pool, tenant, project, attached.id)
                .await
                .expect("detach")
        );
        assert!(
            !repositories::detach(&db.pool, tenant, project, attached.id)
                .await
                .expect("detach again")
        );
    });
}

/// A repository handle from one project cannot address a row in another: the
/// route's path says which project, and a lookup that ignored it would make
/// the path decorative.
#[test]
fn a_repository_handle_is_scoped_to_its_project() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        let one = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("one");
        let two = new_project(&mut tx, tenant, workspace.id, "gateway")
            .await
            .expect("two");
        tx.commit().await.expect("commit");

        let attached = attach(
            &db.pool,
            tenant,
            one.id,
            Some("https://github.com/acme/payments"),
            None,
            None,
        )
        .await
        .expect("attach");

        assert!(
            repositories::get(&db.pool, tenant, two.id, attached.id)
                .await
                .expect("read")
                .is_none(),
            "another project's attachment must read as absent"
        );
        assert!(
            !repositories::detach(&db.pool, tenant, two.id, attached.id)
                .await
                .expect("detach"),
            "another project's attachment must not be detachable"
        );
    });
}

/// The same repository may be attached to two projects: two teams working on
/// one codebase is the ordinary case, not a conflict.
#[test]
fn two_projects_may_be_about_the_same_repository() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let mut tx = begin(&db.pool).await;
        let workspace = new_workspace(&mut tx, tenant, "payments")
            .await
            .expect("workspace");
        let one = new_project(&mut tx, tenant, workspace.id, "ledger")
            .await
            .expect("one");
        let two = new_project(&mut tx, tenant, workspace.id, "gateway")
            .await
            .expect("two");
        tx.commit().await.expect("commit");

        for project in [one.id, two.id] {
            attach(
                &db.pool,
                tenant,
                project,
                Some("https://github.com/acme/monorepo"),
                None,
                None,
            )
            .await
            .expect("attach to both");
        }
    });
}

// ── Idempotency records ──────────────────────────────────────────────────────

/// The record is what makes a retry safe, and the digest is what stops a key
/// from being reused for a different request.
#[test]
fn an_idempotency_record_remembers_one_request() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let resource = ScopeId::new().as_uuid();
        idempotency::remember(
            &db.pool,
            tenant,
            "sam",
            "workspace.create",
            "k-1",
            &[1u8; 32],
            resource,
        )
        .await
        .expect("remember");

        let found = idempotency::find(&db.pool, tenant, "sam", "workspace.create", "k-1")
            .await
            .expect("find")
            .expect("record");
        assert_eq!(found.resource_id, resource);
        assert_eq!(found.request_digest, vec![1u8; 32]);

        // The same key again is refused at the primary key — which is what the
        // gateway turns into a replay.
        let again = idempotency::remember(
            &db.pool,
            tenant,
            "sam",
            "workspace.create",
            "k-1",
            &[1u8; 32],
            resource,
        )
        .await;
        assert!(matches!(again, Err(Error::Conflict { .. })), "{again:?}");

        // A different subject's identical key is a different record: a key is a
        // token a client mints for itself, with no coordination.
        idempotency::remember(
            &db.pool,
            tenant,
            "alex",
            "workspace.create",
            "k-1",
            &[2u8; 32],
            ScopeId::new().as_uuid(),
        )
        .await
        .expect("another subject's key is its own");

        // And a different operation's, likewise.
        idempotency::remember(
            &db.pool,
            tenant,
            "sam",
            "project.create",
            "k-1",
            &[3u8; 32],
            ScopeId::new().as_uuid(),
        )
        .await
        .expect("another operation's key is its own");

        assert!(
            idempotency::find(&db.pool, tenant, "sam", "workspace.create", "k-2")
                .await
                .expect("find")
                .is_none()
        );
    });
}

/// A digest that is not 32 bytes is an application defect, and the store says
/// so rather than storing something a CHECK will reject later.
#[test]
fn a_malformed_digest_is_refused_as_a_defect() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit(&db.pool).await;
        let error = idempotency::remember(
            &db.pool,
            tenant,
            "sam",
            "workspace.create",
            "k-1",
            &[1u8; 16],
            ScopeId::new().as_uuid(),
        )
        .await
        .expect_err("a short digest is refused");
        assert!(matches!(error, Error::Internal { .. }), "{error:?}");
    });
}

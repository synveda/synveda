//! CPR-3 acceptance criteria: the generic governed scope substrate
//! (ADR-0068 decision 4, ADR-0070).
//!
//! What is asserted here, in the order the feature states it: the structural
//! rules (root shape, one root per tenant, the placement rule for every pair
//! of kinds, sibling slugs, no cross-tenant edge, no cycle); arbitrary-depth
//! org units; movement and the closure surgery under it; closure correctness
//! as a property over random operation histories; tenant isolation on every
//! read; and concurrent writers — two creates racing for one slug, two moves
//! racing for one scope, and the one that actually threatens the closure: a
//! create landing inside a subtree that is moving under it.
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make dev-up` then `make db-test`. Isolation is by freshly minted UUIDv7
//! tenants, so a shared dev database is fine.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;
use std::time::Duration;

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{scopes, tenants};
use synveda_types::scope::{
    MAX_ATTRIBUTES_BYTES, MAX_DISPLAY_NAME_CHARS, Scope, ScopeKind, ScopeStatus,
};
use synveda_types::{Error, IdentityId, Result, ScopeId, TenantId, TenantStatus};
use uuid::Uuid;

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

/// Connects (once) to `DATABASE_URL` and applies migrations. `None` = no
/// database configured; every test skips quietly.
fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping scope tests: DATABASE_URL is not set \
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
            // Four is not enough here: the concurrency tests hold two
            // transactions open at once and the invariant check opens its own.
            let pool = PgPoolOptions::new()
                .max_connections(8)
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

/// Guarantees the next statement runs at a strictly later `now()`, so an
/// `updated_at` comparison measures the write rather than the clock's
/// resolution.
async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("cpr3-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "CPR-3 fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    tenant
}

fn new_scope(
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> scopes::NewScope {
    scopes::NewScope {
        id: ScopeId::new(),
        tenant_id: tenant,
        kind,
        parent_scope_id: parent,
        slug: slug.to_owned(),
        display_name: format!("{slug} ({kind})"),
        attributes: serde_json::json!({}),
        created_by: Some(IdentityId::new()),
    }
}

/// Creates a scope in its own committed transaction.
async fn try_add(
    pool: &PgPool,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> Result<Scope> {
    let mut tx = pool.begin().await.expect("begin transaction");
    let created = scopes::create(&mut tx, &new_scope(tenant, parent, kind, slug)).await;
    match created {
        Ok(scope) => {
            tx.commit().await.expect("commit create");
            Ok(scope)
        }
        Err(err) => Err(err),
    }
}

async fn add(
    pool: &PgPool,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> Scope {
    try_add(pool, tenant, parent, kind, slug)
        .await
        .unwrap_or_else(|err| panic!("create {kind} {slug:?}: {err}"))
}

/// Moves a scope in its own committed transaction.
async fn move_to(pool: &PgPool, tenant: TenantId, id: ScopeId, parent: ScopeId) -> Result<Scope> {
    let mut tx = pool.begin().await.expect("begin transaction");
    match scopes::move_scope(&mut tx, tenant, id, parent).await {
        Ok(scope) => {
            tx.commit().await.expect("commit move");
            Ok(scope)
        }
        Err(err) => Err(err),
    }
}

/// A tenant with one scope of every kind:
///
/// ```text
/// acme (tenant)
/// ├── unit      (org_unit)
/// ├── space     (workspace)
/// │   └── proj  (project)
/// └── person    (principal)
/// ```
struct Fixture {
    tenant: TenantId,
    root: Scope,
    unit: Scope,
    space: Scope,
    proj: Scope,
    person: Scope,
}

impl Fixture {
    /// The fixture scope of each kind — the parent side of the placement
    /// matrix.
    fn of_kind(&self, kind: ScopeKind) -> &Scope {
        match kind {
            ScopeKind::Tenant => &self.root,
            ScopeKind::OrgUnit => &self.unit,
            ScopeKind::Workspace => &self.space,
            ScopeKind::Project => &self.proj,
            ScopeKind::Principal => &self.person,
        }
    }
}

async fn seed(pool: &PgPool) -> Fixture {
    let tenant = admit_tenant(pool).await;
    let root = add(pool, tenant, None, ScopeKind::Tenant, "acme").await;
    let unit = add(pool, tenant, Some(root.id), ScopeKind::OrgUnit, "unit").await;
    let space = add(pool, tenant, Some(root.id), ScopeKind::Workspace, "space").await;
    let proj = add(pool, tenant, Some(space.id), ScopeKind::Project, "proj").await;
    let person = add(pool, tenant, Some(root.id), ScopeKind::Principal, "person").await;
    Fixture {
        tenant,
        root,
        unit,
        space,
        proj,
        person,
    }
}

// ── The closure invariant ────────────────────────────────────────────────────

/// Recomputes the closure from the adjacency and asserts the stored one is
/// exactly that — plus the two things the recomputation needs in order to
/// terminate at all: no adjacency cycle, and exactly one root.
///
/// This is the oracle every structural test in this file ends with, and the
/// property test's whole assertion.
async fn assert_closure_matches_adjacency(pool: &PgPool, tenant: TenantId, after: &str) {
    let rows = sqlx::query!(
        "select id, parent_scope_id from scopes where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read the adjacency");

    let parents: HashMap<Uuid, Option<Uuid>> = rows
        .iter()
        .map(|row| (row.id, row.parent_scope_id))
        .collect();
    let roots = parents.values().filter(|parent| parent.is_none()).count();
    assert!(
        roots <= 1,
        "{after}: {roots} root scopes in one tenant — there is exactly one, or none yet"
    );

    let mut expected: BTreeSet<(Uuid, Uuid, i32)> = BTreeSet::new();
    for &id in parents.keys() {
        let mut current = id;
        let mut distance = 0;
        loop {
            assert!(
                distance as usize <= parents.len(),
                "{after}: walking up from {id} did not terminate — an adjacency cycle"
            );
            expected.insert((current, id, distance));
            match parents.get(&current) {
                Some(Some(parent)) => {
                    current = *parent;
                    distance += 1;
                }
                Some(None) => break,
                None => panic!("{after}: scope {current} has a parent outside its own tenant"),
            }
        }
    }

    let stored = sqlx::query!(
        "select ancestor_id, descendant_id, distance from scope_closure where tenant_id = $1",
        tenant.as_uuid(),
    )
    .fetch_all(pool)
    .await
    .expect("read the closure");
    let actual: BTreeSet<(Uuid, Uuid, i32)> = stored
        .into_iter()
        .map(|row| (row.ancestor_id, row.descendant_id, row.distance))
        .collect();

    assert_eq!(
        actual, expected,
        "{after}: the closure disagrees with the adjacency"
    );
}

// ── Creation and the structural rules ────────────────────────────────────────

/// A scope's row, its closure rows and every read over them agree, from the
/// tenant root down.
#[test]
fn create_builds_adjacency_and_closure() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;

        assert_eq!(fx.root.parent_scope_id, None);
        assert_eq!(fx.root.status, ScopeStatus::Active);
        assert_eq!(fx.proj.parent_scope_id, Some(fx.space.id));

        let root = scopes::tenant_root(&db.pool, fx.tenant)
            .await
            .expect("read the tenant root")
            .expect("the tenant has a root");
        assert_eq!(root, fx.root);

        let ancestors = scopes::ancestors(&db.pool, fx.tenant, fx.proj.id)
            .await
            .expect("ancestors");
        assert_eq!(
            ancestors.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![fx.space.id, fx.root.id],
            "ancestors are nearest-first and exclude the scope itself"
        );
        assert!(
            scopes::ancestors(&db.pool, fx.tenant, fx.root.id)
                .await
                .expect("root ancestors")
                .is_empty()
        );

        let descendants = scopes::descendants(&db.pool, fx.tenant, fx.root.id)
            .await
            .expect("descendants");
        assert_eq!(
            descendants.len(),
            4,
            "the root's subtree is everything else"
        );
        assert!(
            scopes::descendants(&db.pool, fx.tenant, fx.proj.id)
                .await
                .expect("leaf descendants")
                .is_empty()
        );

        let children = scopes::children(&db.pool, fx.tenant, fx.root.id)
            .await
            .expect("children");
        assert_eq!(
            children.iter().map(|s| s.slug.as_str()).collect::<Vec<_>>(),
            vec!["person", "space", "unit"],
            "children are ordered by slug"
        );

        assert_eq!(
            scopes::path(&db.pool, fx.tenant, fx.proj.id)
                .await
                .expect("path"),
            Some("acme/space/proj".to_owned())
        );
        assert_eq!(
            scopes::resolve_path(&db.pool, fx.tenant, "acme/space/proj")
                .await
                .expect("resolve"),
            Some(fx.proj.clone())
        );

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "create").await;
    });
}

/// The root is a `tenant` scope, there is exactly one, and no other kind may
/// be parentless.
#[test]
fn the_root_is_a_tenant_scope_and_unique_per_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        assert_eq!(
            scopes::tenant_root(&db.pool, tenant)
                .await
                .expect("read root"),
            None,
            "a tenant with no scopes has no root"
        );

        for kind in ScopeKind::ALL {
            if kind.is_tenant_root() {
                continue;
            }
            let result = try_add(&db.pool, tenant, None, *kind, "orphan").await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "a parentless {kind} must be refused, got {result:?}"
            );
        }

        let root = add(&db.pool, tenant, None, ScopeKind::Tenant, "acme").await;

        let second = try_add(&db.pool, tenant, None, ScopeKind::Tenant, "acme-two").await;
        assert!(
            matches!(second, Err(Error::Conflict { .. })),
            "a second root must conflict, got {second:?}"
        );

        let nested = try_add(
            &db.pool,
            tenant,
            Some(root.id),
            ScopeKind::Tenant,
            "inner-tenant",
        )
        .await;
        assert!(
            matches!(nested, Err(Error::Invalid { .. })),
            "a tenant scope under a parent must be refused, got {nested:?}"
        );

        assert_closure_matches_adjacency(&db.pool, tenant, "root rules").await;
    });
}

/// The placement rule, over the whole product of the vocabulary: a create
/// succeeds exactly when [`ScopeKind::permits_parent`] says it may. Nothing
/// is asserted twice and nothing is left out — the matrix is the rule.
#[test]
fn the_placement_rule_holds_for_every_pair_of_kinds() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;

        for child in ScopeKind::ALL {
            for parent in ScopeKind::ALL {
                let anchor = fx.of_kind(*parent);
                let slug =
                    format!("{}-under-{}", child.as_str(), parent.as_str()).replace('_', "-");
                let result = try_add(&db.pool, fx.tenant, Some(anchor.id), *child, &slug).await;
                if child.permits_parent(*parent) {
                    let created =
                        result.unwrap_or_else(|err| panic!("{child} under {parent}: {err}"));
                    assert_eq!(created.parent_scope_id, Some(anchor.id));
                } else {
                    assert!(
                        matches!(result, Err(Error::Invalid { .. })),
                        "{child} under {parent} must be refused, got {result:?}"
                    );
                }
            }
        }

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "the placement matrix").await;
    });
}

/// Org units nest inside themselves to whatever depth a deployment needs —
/// the property the five-rank vocabulary could not express, and the reason a
/// division/department/team ladder is no longer something anybody declares.
#[test]
fn org_units_nest_to_arbitrary_depth() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        const DEPTH: usize = 40;
        let tenant = admit_tenant(&db.pool).await;
        let root = add(&db.pool, tenant, None, ScopeKind::Tenant, "acme").await;

        let mut current = root.clone();
        let mut expected_path = vec!["acme".to_owned()];
        for level in 0..DEPTH {
            let slug = format!("l{level}");
            current = add(
                &db.pool,
                tenant,
                Some(current.id),
                ScopeKind::OrgUnit,
                &slug,
            )
            .await;
            expected_path.push(slug);
        }

        let ancestors = scopes::ancestors(&db.pool, tenant, current.id)
            .await
            .expect("ancestors of the deepest scope");
        assert_eq!(ancestors.len(), DEPTH, "one per level above the deepest");
        assert_eq!(
            ancestors.last().expect("a root ancestor").id,
            root.id,
            "the farthest ancestor is the tenant root"
        );
        assert_eq!(
            scopes::descendants(&db.pool, tenant, root.id)
                .await
                .expect("descendants")
                .len(),
            DEPTH
        );

        let path = expected_path.join("/");
        assert_eq!(
            scopes::path(&db.pool, tenant, current.id)
                .await
                .expect("path"),
            Some(path.clone())
        );
        assert_eq!(
            scopes::resolve_path(&db.pool, tenant, &path)
                .await
                .expect("resolve the deep path")
                .map(|scope| scope.id),
            Some(current.id)
        );

        // A workspace hangs off any of them, and a project off that: depth is
        // not a rank, so nothing "runs out of levels".
        let space = add(
            &db.pool,
            tenant,
            Some(current.id),
            ScopeKind::Workspace,
            "space",
        )
        .await;
        add(&db.pool, tenant, Some(space.id), ScopeKind::Project, "proj").await;

        assert_closure_matches_adjacency(&db.pool, tenant, "a 40-deep chain").await;
    });
}

/// Sibling slugs are unique under their parent, and only under their parent.
#[test]
fn sibling_slugs_are_unique_within_their_parent() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;

        add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::OrgUnit,
            "a",
        )
        .await;
        let clash = try_add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::OrgUnit,
            "a",
        )
        .await;
        assert!(
            matches!(clash, Err(Error::Conflict { .. })),
            "a duplicate sibling slug must conflict, got {clash:?}"
        );
        // A different kind does not make it a different sibling.
        let clash = try_add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::Workspace,
            "a",
        )
        .await;
        assert!(matches!(clash, Err(Error::Conflict { .. })), "{clash:?}");

        // The same slug under another parent is a different scope.
        add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "a",
        )
        .await;

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "sibling slugs").await;
    });
}

/// Invalid input is refused before anything is written, and refused with a
/// message that says which rule.
#[test]
fn malformed_input_is_refused() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let parent = Some(fx.root.id);

        for bad_slug in ["", "-lead", "Upper", "under_score", "with space"] {
            let result = try_add(&db.pool, fx.tenant, parent, ScopeKind::OrgUnit, bad_slug).await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "slug {bad_slug:?} must be refused, got {result:?}"
            );
        }

        let mut blank = new_scope(fx.tenant, parent, ScopeKind::OrgUnit, "named");
        blank.display_name = "   ".to_owned();
        let mut tx = db.pool.begin().await.expect("begin");
        assert!(matches!(
            scopes::create(&mut tx, &blank).await,
            Err(Error::Invalid { .. })
        ));
        drop(tx);

        let mut long = new_scope(fx.tenant, parent, ScopeKind::OrgUnit, "named");
        long.display_name = "x".repeat(MAX_DISPLAY_NAME_CHARS + 1);
        let mut tx = db.pool.begin().await.expect("begin");
        assert!(matches!(
            scopes::create(&mut tx, &long).await,
            Err(Error::Invalid { .. })
        ));
        drop(tx);

        for bad_attributes in [
            serde_json::json!([]),
            serde_json::json!("labels"),
            serde_json::json!({"blob": "x".repeat(MAX_ATTRIBUTES_BYTES)}),
        ] {
            let mut spec = new_scope(fx.tenant, parent, ScopeKind::OrgUnit, "attributed");
            spec.attributes = bad_attributes;
            let mut tx = db.pool.begin().await.expect("begin");
            let result = scopes::create(&mut tx, &spec).await;
            assert!(matches!(result, Err(Error::Invalid { .. })), "{result:?}");
        }

        assert_eq!(
            scopes::children(&db.pool, fx.tenant, fx.root.id)
                .await
                .expect("children")
                .len(),
            3,
            "nothing was written by any of the refusals"
        );
    });
}

/// Another tenant's scope is indistinguishable from one that does not exist,
/// on every surface — no existence oracle across tenants (ADR-0008).
#[test]
fn another_tenants_scope_is_not_found_rather_than_forbidden() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed(&db.pool).await;
        let theirs = seed(&db.pool).await;

        let stranger = try_add(
            &db.pool,
            mine.tenant,
            Some(theirs.root.id),
            ScopeKind::OrgUnit,
            "borrowed",
        )
        .await;
        assert!(
            matches!(stranger, Err(Error::NotFound { .. })),
            "a parent in another tenant is not found, got {stranger:?}"
        );
        let missing = try_add(
            &db.pool,
            mine.tenant,
            Some(ScopeId::new()),
            ScopeKind::OrgUnit,
            "nowhere",
        )
        .await;
        assert!(
            matches!(missing, Err(Error::NotFound { .. })),
            "{missing:?}"
        );

        assert_eq!(
            scopes::get(&db.pool, mine.tenant, theirs.proj.id)
                .await
                .expect("get"),
            None
        );
        assert!(
            scopes::ancestors(&db.pool, mine.tenant, theirs.proj.id)
                .await
                .expect("ancestors")
                .is_empty()
        );
        assert!(
            scopes::descendants(&db.pool, mine.tenant, theirs.root.id)
                .await
                .expect("descendants")
                .is_empty()
        );
        assert!(
            scopes::children(&db.pool, mine.tenant, theirs.root.id)
                .await
                .expect("children")
                .is_empty()
        );
        assert_eq!(
            scopes::path(&db.pool, mine.tenant, theirs.proj.id)
                .await
                .expect("path"),
            None
        );
        assert_eq!(
            scopes::resolve_path(&db.pool, mine.tenant, "acme/space/proj")
                .await
                .expect("resolve")
                .map(|scope| scope.id),
            Some(mine.proj.id),
            "the same path in two tenants resolves to each tenant's own scope"
        );

        let across = move_to(&db.pool, mine.tenant, mine.unit.id, theirs.root.id).await;
        assert!(
            matches!(across, Err(Error::NotFound { .. })),
            "a destination in another tenant is not found, got {across:?}"
        );
    });
}

/// A scope never moves across tenants. The service refuses it; the database
/// refuses it even to the owner role, which is the role a migration, a
/// break-glass psql session and a restore all run as.
#[test]
fn a_scope_can_never_move_across_tenants() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed(&db.pool).await;
        let theirs = seed(&db.pool).await;

        let forged = sqlx::query!(
            "update scopes set tenant_id = $2 where id = $1",
            mine.unit.id.as_uuid(),
            theirs.tenant.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        let message = forged.expect_err("the trigger must refuse it").to_string();
        assert!(
            message.contains("cannot move across tenants"),
            "the refusal must say what it refused: {message}"
        );

        // The composite parent key makes a cross-tenant edge unrepresentable
        // rather than merely refused, so this cannot be written either.
        let edge = sqlx::query!(
            "update scopes set parent_scope_id = $2, parent_kind = 'tenant' where id = $1",
            mine.unit.id.as_uuid(),
            theirs.root.id.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(edge.is_err(), "a cross-tenant parent edge must not exist");

        assert_closure_matches_adjacency(&db.pool, mine.tenant, "a refused cross-tenant move")
            .await;
    });
}

/// Slug, kind and provenance are immutable, so a path somebody wrote down
/// stays the path of the same scope.
#[test]
fn identity_slug_kind_and_provenance_are_immutable() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let id = fx.unit.id.as_uuid();

        for (what, result) in [
            (
                "slug",
                sqlx::query!("update scopes set slug = 'renamed' where id = $1", id)
                    .execute(&db.pool)
                    .await,
            ),
            (
                "kind",
                sqlx::query!("update scopes set kind = 'workspace' where id = $1", id)
                    .execute(&db.pool)
                    .await,
            ),
            (
                "created_by",
                sqlx::query!("update scopes set created_by = null where id = $1", id)
                    .execute(&db.pool)
                    .await,
            ),
            (
                "created_at",
                sqlx::query!("update scopes set created_at = now() where id = $1", id)
                    .execute(&db.pool)
                    .await,
            ),
        ] {
            assert!(result.is_err(), "scopes.{what} must be immutable");
        }
    });
}

/// A cycle cannot be written, by the service or around it: the closure's own
/// CHECK refuses the row a cycle would need.
#[test]
fn cycles_are_impossible() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let child = add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::OrgUnit,
            "child",
        )
        .await;
        let grandchild = add(
            &db.pool,
            fx.tenant,
            Some(child.id),
            ScopeKind::OrgUnit,
            "grandchild",
        )
        .await;

        for (scope, destination, what) in [
            (fx.unit.id, child.id, "a scope under its own child"),
            (
                fx.unit.id,
                grandchild.id,
                "a scope under its own grandchild",
            ),
            (fx.unit.id, fx.unit.id, "a scope under itself"),
        ] {
            let result = move_to(&db.pool, fx.tenant, scope, destination).await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "{what} must be refused, got {result:?}"
            );
        }

        let self_ancestor = sqlx::query!(
            r#"
            insert into scope_closure (tenant_id, ancestor_id, descendant_id, distance)
            values ($1, $2, $2, 2)
            "#,
            fx.tenant.as_uuid(),
            fx.unit.id.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(
            self_ancestor.is_err(),
            "a scope cannot be its own ancestor at a distance"
        );

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "refused cycles").await;
    });
}

// ── Movement ─────────────────────────────────────────────────────────────────

/// Moving a scope carries its whole subtree: the closure is rewritten for
/// every descendant, and the paths follow.
#[test]
fn move_rewrites_the_closure_for_the_whole_subtree() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let inner = add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::OrgUnit,
            "inner",
        )
        .await;
        let space = add(
            &db.pool,
            fx.tenant,
            Some(inner.id),
            ScopeKind::Workspace,
            "space",
        )
        .await;
        let proj = add(&db.pool, fx.tenant, Some(space.id), ScopeKind::Project, "p").await;
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, proj.id)
                .await
                .expect("path"),
            Some("acme/unit/inner/space/p".to_owned())
        );

        // Move the middle of the chain up to the root.
        let moved = move_to(&db.pool, fx.tenant, inner.id, fx.root.id)
            .await
            .expect("move inner to the root");
        assert_eq!(moved.parent_scope_id, Some(fx.root.id));
        assert!(
            moved.updated_at >= moved.created_at,
            "a move moves updated_at"
        );

        assert_eq!(
            scopes::path(&db.pool, fx.tenant, proj.id)
                .await
                .expect("path"),
            Some("acme/inner/space/p".to_owned()),
            "the whole subtree's paths follow the move"
        );
        assert_eq!(
            scopes::ancestors(&db.pool, fx.tenant, proj.id)
                .await
                .expect("ancestors")
                .iter()
                .map(|s| s.id)
                .collect::<Vec<_>>(),
            vec![space.id, inner.id, fx.root.id]
        );
        assert!(
            scopes::descendants(&db.pool, fx.tenant, fx.unit.id)
                .await
                .expect("descendants")
                .is_empty(),
            "the old parent keeps nothing"
        );
        assert_eq!(
            scopes::resolve_path(&db.pool, fx.tenant, "acme/unit/inner/space/p")
                .await
                .expect("resolve the old path"),
            None,
            "the path the subtree used to have resolves to nothing"
        );

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "a subtree move").await;
    });
}

/// Ineligible moves are refused, each for its own stated reason.
#[test]
fn ineligible_moves_are_refused() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;

        let root_move = move_to(&db.pool, fx.tenant, fx.root.id, fx.unit.id).await;
        assert!(
            matches!(root_move, Err(Error::Invalid { .. })),
            "the tenant root cannot move, got {root_move:?}"
        );

        // The placement rule binds a move exactly as it binds a create.
        let bad_placement = move_to(&db.pool, fx.tenant, fx.proj.id, fx.unit.id).await;
        assert!(
            matches!(bad_placement, Err(Error::Invalid { .. })),
            "a project cannot move under an org unit, got {bad_placement:?}"
        );

        let unknown = move_to(&db.pool, fx.tenant, ScopeId::new(), fx.unit.id).await;
        assert!(
            matches!(unknown, Err(Error::NotFound { .. })),
            "{unknown:?}"
        );
        let nowhere = move_to(&db.pool, fx.tenant, fx.unit.id, ScopeId::new()).await;
        assert!(
            matches!(nowhere, Err(Error::NotFound { .. })),
            "{nowhere:?}"
        );

        // A destination that already holds a sibling with this slug.
        let unit_b = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "b",
        )
        .await;
        add(
            &db.pool,
            fx.tenant,
            Some(fx.unit.id),
            ScopeKind::OrgUnit,
            "b",
        )
        .await;
        let clash = move_to(&db.pool, fx.tenant, unit_b.id, fx.unit.id).await;
        assert!(
            matches!(clash, Err(Error::Conflict { .. })),
            "a slug collision at the destination conflicts, got {clash:?}"
        );

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "refused moves").await;
    });
}

/// A workspace moves between org units, and a project between workspaces —
/// the two moves a deployment actually makes.
#[test]
fn eligible_scopes_move_between_permitted_parents() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let other_unit = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "other",
        )
        .await;
        let other_space = add(
            &db.pool,
            fx.tenant,
            Some(other_unit.id),
            ScopeKind::Workspace,
            "space",
        )
        .await;

        let moved = move_to(&db.pool, fx.tenant, fx.space.id, fx.unit.id)
            .await
            .expect("a workspace moves under an org unit");
        assert_eq!(moved.parent_scope_id, Some(fx.unit.id));
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, fx.proj.id)
                .await
                .expect("path"),
            Some("acme/unit/space/proj".to_owned())
        );

        let moved = move_to(&db.pool, fx.tenant, fx.proj.id, other_space.id)
            .await
            .expect("a project moves between workspaces");
        assert_eq!(moved.parent_scope_id, Some(other_space.id));
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, fx.proj.id)
                .await
                .expect("path"),
            Some("acme/other/space/proj".to_owned())
        );

        // A principal's only permitted parent is the tenant root, which is
        // where it is: its move is a no-op by construction rather than by a
        // special case.
        let stayed = move_to(&db.pool, fx.tenant, fx.person.id, fx.root.id)
            .await
            .expect("a principal moves to the root it is already under");
        assert_eq!(stayed.parent_scope_id, Some(fx.root.id));
        let elsewhere = move_to(&db.pool, fx.tenant, fx.person.id, fx.unit.id).await;
        assert!(
            matches!(elsewhere, Err(Error::Invalid { .. })),
            "a principal cannot sit under an org unit, got {elsewhere:?}"
        );

        assert_closure_matches_adjacency(&db.pool, fx.tenant, "permitted moves").await;
    });
}

// ── Rename and path resolution ───────────────────────────────────────────────

/// Rename changes the display name and nothing else — not the slug, not the
/// path, not what a written-down address resolves to.
#[test]
fn rename_changes_only_the_display_name() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        tick().await;

        let renamed = scopes::rename(&db.pool, fx.tenant, fx.unit.id, "Platform Engineering")
            .await
            .expect("rename");
        assert_eq!(renamed.display_name, "Platform Engineering");
        assert_eq!(renamed.slug, fx.unit.slug);
        assert_eq!(renamed.kind, fx.unit.kind);
        assert_eq!(renamed.parent_scope_id, fx.unit.parent_scope_id);
        assert_eq!(renamed.created_at, fx.unit.created_at);
        assert!(
            renamed.updated_at > fx.unit.updated_at,
            "a rename moves updated_at"
        );
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, fx.unit.id)
                .await
                .expect("path"),
            Some("acme/unit".to_owned())
        );

        assert!(matches!(
            scopes::rename(&db.pool, fx.tenant, fx.unit.id, "  ").await,
            Err(Error::Invalid { .. })
        ));
        assert!(matches!(
            scopes::rename(&db.pool, fx.tenant, ScopeId::new(), "Nobody").await,
            Err(Error::NotFound { .. })
        ));
        // Another tenant's scope is not renameable, and not distinguishable
        // from one that does not exist.
        let theirs = seed(&db.pool).await;
        assert!(matches!(
            scopes::rename(&db.pool, fx.tenant, theirs.unit.id, "Theirs").await,
            Err(Error::NotFound { .. })
        ));
    });
}

/// Every scope's path resolves back to it, a path that names nothing resolves
/// to nothing, and a malformed path is a mistake rather than a miss.
#[test]
fn paths_round_trip_and_refuse_what_is_not_a_path() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let mut all = vec![
            fx.root.clone(),
            fx.unit.clone(),
            fx.space.clone(),
            fx.proj.clone(),
            fx.person.clone(),
        ];
        all.push(
            add(
                &db.pool,
                fx.tenant,
                Some(fx.unit.id),
                ScopeKind::OrgUnit,
                "deep",
            )
            .await,
        );

        for scope in &all {
            let path = scopes::path(&db.pool, fx.tenant, scope.id)
                .await
                .expect("path")
                .expect("every scope has a path");
            let resolved = scopes::resolve_path(&db.pool, fx.tenant, &path)
                .await
                .expect("resolve")
                .expect("a path resolves to the scope it was taken from");
            assert_eq!(resolved.id, scope.id, "path {path:?}");
        }

        for miss in ["nobody", "acme/nobody", "acme/space/proj/deeper"] {
            assert_eq!(
                scopes::resolve_path(&db.pool, fx.tenant, miss)
                    .await
                    .expect("resolve a miss"),
                None,
                "{miss:?} names nothing"
            );
        }
        for malformed in ["", "/acme", "acme/", "acme//space", "acme/Space"] {
            let result = scopes::resolve_path(&db.pool, fx.tenant, malformed).await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "{malformed:?} is not a path, got {result:?}"
            );
        }
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, ScopeId::new())
                .await
                .expect("path of a scope that does not exist"),
            None
        );
    });
}

// ── Property: random operation histories keep the closure honest ─────────────

#[derive(Debug, Clone)]
enum OpSpec {
    /// Create under the scope at `parent` (modulo the tree's size).
    Create { parent: usize, kind: usize },
    /// Move the scope at `scope` under the scope at `parent`.
    Move { scope: usize, parent: usize },
    /// Rename the scope at `scope`.
    Rename { scope: usize },
}

fn ops_strategy() -> impl Strategy<Value = Vec<OpSpec>> {
    let op = prop_oneof![
        3 => (0usize..32, 0usize..ScopeKind::ALL.len())
            .prop_map(|(parent, kind)| OpSpec::Create { parent, kind }),
        2 => (0usize..32, 0usize..32).prop_map(|(scope, parent)| OpSpec::Move { scope, parent }),
        1 => (0usize..32).prop_map(|scope| OpSpec::Rename { scope }),
    ];
    proptest::collection::vec(op, 1..24)
}

/// Applies a random history to a fresh tenant. Every operation is attempted
/// whether or not it is legal — a refusal is a valid outcome and the point of
/// generating them — and the closure invariant is checked after each one.
///
/// This is where "cycles are impossible" and "closure correctness" stop being
/// a list of cases somebody thought of: an illegal move that slipped through
/// would show up as an adjacency walk that does not terminate, and a closure
/// left half-rewritten would show up as a set difference.
async fn check_history(pool: &PgPool, ops: Vec<OpSpec>) {
    let tenant = admit_tenant(pool).await;
    let mut tree = vec![add(pool, tenant, None, ScopeKind::Tenant, "root").await];

    for (seq, op) in ops.into_iter().enumerate() {
        match op {
            OpSpec::Create { parent, kind } => {
                let parent = tree[parent % tree.len()].id;
                let slug = format!("s{seq}");
                if let Ok(created) =
                    try_add(pool, tenant, Some(parent), ScopeKind::ALL[kind], &slug).await
                {
                    tree.push(created);
                }
            }
            OpSpec::Move { scope, parent } => {
                let scope = tree[scope % tree.len()].id;
                let parent = tree[parent % tree.len()].id;
                let _ = move_to(pool, tenant, scope, parent).await;
            }
            OpSpec::Rename { scope } => {
                let scope = tree[scope % tree.len()].id;
                scopes::rename(pool, tenant, scope, &format!("renamed {seq}"))
                    .await
                    .expect("rename an existing scope");
            }
        }
        assert_closure_matches_adjacency(pool, tenant, &format!("operation {seq}")).await;
    }
}

#[test]
fn the_closure_survives_random_operation_histories() {
    let Some(db) = db() else { return };
    let mut runner = TestRunner::new(Config::with_cases(16));
    runner
        .run(&ops_strategy(), |ops| {
            db.rt.block_on(check_history(&db.pool, ops));
            Ok(())
        })
        .unwrap_or_else(|err| panic!("the scope closure property failed: {err}"));
}

// ── Concurrency ──────────────────────────────────────────────────────────────

/// Two writers racing for one sibling slug: exactly one wins, and the loser
/// gets a conflict rather than a second row.
#[test]
fn concurrent_creates_of_one_sibling_slug_admit_exactly_one() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;

        let first = async {
            let mut tx = db.pool.begin().await.expect("begin first");
            let created = scopes::create(
                &mut tx,
                &new_scope(fx.tenant, Some(fx.root.id), ScopeKind::OrgUnit, "race"),
            )
            .await;
            // Hold the row long enough for the second writer to arrive on the
            // unique index and wait there.
            tokio::time::sleep(Duration::from_millis(200)).await;
            if created.is_ok() {
                tx.commit().await.expect("commit first");
            }
            created
        };
        let second = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut tx = db.pool.begin().await.expect("begin second");
            let created = scopes::create(
                &mut tx,
                &new_scope(fx.tenant, Some(fx.root.id), ScopeKind::OrgUnit, "race"),
            )
            .await;
            if created.is_ok() {
                tx.commit().await.expect("commit second");
            }
            created
        };

        let (first, second) = tokio::join!(first, second);
        assert!(first.is_ok(), "the first writer wins: {first:?}");
        assert!(
            matches!(second, Err(Error::Conflict { .. })),
            "the second writer conflicts, got {second:?}"
        );
        assert_eq!(
            scopes::children(&db.pool, fx.tenant, fx.root.id)
                .await
                .expect("children")
                .iter()
                .filter(|scope| scope.slug == "race")
                .count(),
            1
        );
        assert_closure_matches_adjacency(&db.pool, fx.tenant, "a slug race").await;
    });
}

/// Two moves of one scope serialise on its row: both succeed, in an order,
/// and the closure agrees with the last one.
#[test]
fn concurrent_moves_of_one_scope_serialise() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let a = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "a",
        )
        .await;
        let b = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "b",
        )
        .await;
        let mover = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::Workspace,
            "mover",
        )
        .await;

        let first = async {
            let mut tx = db.pool.begin().await.expect("begin first");
            let moved = scopes::move_scope(&mut tx, fx.tenant, mover.id, a.id).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            tx.commit().await.expect("commit first");
            moved
        };
        let second = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut tx = db.pool.begin().await.expect("begin second");
            let moved = scopes::move_scope(&mut tx, fx.tenant, mover.id, b.id).await;
            tx.commit().await.expect("commit second");
            moved
        };

        let (first, second) = tokio::join!(first, second);
        assert_eq!(
            first.expect("the first move").parent_scope_id,
            Some(a.id),
            "the first move lands"
        );
        assert_eq!(
            second.expect("the second move").parent_scope_id,
            Some(b.id),
            "the second move waits for the first and then lands"
        );
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, mover.id)
                .await
                .expect("path"),
            Some("acme/b/mover".to_owned())
        );
        assert_closure_matches_adjacency(&db.pool, fx.tenant, "two moves of one scope").await;
    });
}

/// A create landing under a scope whose ancestry is being rewritten by a move
/// above it: the writer waits, and then inherits the ancestry the move left.
///
/// The create derives its closure rows from its parent's ancestry, so the two
/// writers are touching the same rows from opposite ends. Both must succeed
/// and the closure must agree with the adjacency afterwards — asserted here by
/// the newcomer's ancestors naming the move's *destination*, which is only
/// true if the create read the post-move ancestry.
///
/// What this test does **not** prove is that `move_scope`'s subtree lock is
/// what makes that happen: deleting the lock leaves this test passing, because
/// the relink's foreign keys take a share lock on every subtree member and
/// block the create too. The lock is there so the ordering is stated rather
/// than inherited from a foreign key (see `scopes::lock_subtree`); this test
/// is about the outcome under a real race, which is the part a regression
/// would break either way.
#[test]
fn a_create_inside_a_moving_subtree_waits_for_the_move() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fx = seed(&db.pool).await;
        let outer = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "outer",
        )
        .await;
        let inner = add(
            &db.pool,
            fx.tenant,
            Some(outer.id),
            ScopeKind::OrgUnit,
            "inner",
        )
        .await;
        let destination = add(
            &db.pool,
            fx.tenant,
            Some(fx.root.id),
            ScopeKind::OrgUnit,
            "destination",
        )
        .await;

        let newcomer = ScopeId::new();
        let mover = async {
            let mut tx = db.pool.begin().await.expect("begin the move");
            let moved = scopes::move_scope(&mut tx, fx.tenant, outer.id, destination.id)
                .await
                .expect("move outer under destination");
            tokio::time::sleep(Duration::from_millis(250)).await;
            tx.commit().await.expect("commit the move");
            moved
        };
        let creator = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut tx = db.pool.begin().await.expect("begin the create");
            let mut spec = new_scope(fx.tenant, Some(inner.id), ScopeKind::Workspace, "newcomer");
            spec.id = newcomer;
            let created = scopes::create(&mut tx, &spec)
                .await
                .expect("create under a scope inside the moving subtree");
            tx.commit().await.expect("commit the create");
            created
        };

        let (moved, created) = tokio::join!(mover, creator);
        assert_eq!(moved.parent_scope_id, Some(destination.id));
        assert_eq!(created.id, newcomer);

        let ancestors = scopes::ancestors(&db.pool, fx.tenant, newcomer)
            .await
            .expect("ancestors of the newcomer");
        assert_eq!(
            ancestors.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![inner.id, outer.id, destination.id, fx.root.id],
            "the create waited for the move and inherited the ancestry it left"
        );
        assert_eq!(
            scopes::path(&db.pool, fx.tenant, newcomer)
                .await
                .expect("path"),
            Some("acme/destination/outer/inner/newcomer".to_owned())
        );
        assert_closure_matches_adjacency(&db.pool, fx.tenant, "a create inside a moving subtree")
            .await;
    });
}

// ── Tenancy on every read ────────────────────────────────────────────────────

/// Every read filters on the tenant in SQL, not only through the RLS
/// backstop: these run on an owner connection, where RLS does not bite, and
/// still see nothing of another tenant.
#[test]
fn reads_are_tenant_filtered_even_where_rls_does_not_bite() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed(&db.pool).await;
        let theirs = seed(&db.pool).await;

        assert_eq!(
            scopes::tenant_root(&db.pool, mine.tenant)
                .await
                .expect("root")
                .map(|scope| scope.id),
            Some(mine.root.id)
        );
        assert_eq!(
            scopes::get(&db.pool, theirs.tenant, mine.root.id)
                .await
                .expect("get"),
            None
        );

        // Both tenants used the same slugs; each sees only its own tree.
        let mut tx: Transaction<'static, Postgres> =
            db.pool.begin().await.expect("begin transaction");
        let count = sqlx::query_scalar!(
            r#"select count(*) as "count!" from scopes where tenant_id = $1"#,
            mine.tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count");
        assert_eq!(count, 5);
        drop(tx);

        assert_eq!(
            scopes::descendants(&db.pool, mine.tenant, mine.root.id)
                .await
                .expect("descendants")
                .len(),
            4
        );
        assert_eq!(
            scopes::descendants(&db.pool, theirs.tenant, mine.root.id)
                .await
                .expect("descendants")
                .len(),
            0
        );
    });
}

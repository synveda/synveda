//! AUTH-2 store contract: identities (create/read, the first-login race
//! conflict, placement-derived quarantine), group-mapping overrides, and
//! the convention-candidate hierarchy queries (`teams_matching`,
//! `child_by_slug`) JIT resolution rides on (ADR-0013).
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip
//! with a message when it is unset (CI has no database); run them locally
//! with `make dev-up` then `make db-test`. Isolation is by freshly minted
//! UUIDv7 tenants, so a shared dev database is fine.

use std::sync::OnceLock;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{identities, scopes, tenants};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{Error, IdentityId, IdentityKind, ScopeId, TenantId, TenantStatus};

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
                    "skipping identity tests: DATABASE_URL is not set \
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
                .max_connections(4)
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

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("auth2-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "AUTH-2 fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    tenant
}

async fn tx(pool: &PgPool) -> Transaction<'static, Postgres> {
    pool.begin().await.expect("begin transaction")
}

/// Creates a scope in its own committed transaction.
async fn add(
    pool: &PgPool,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> Scope {
    let mut tx = tx(pool).await;
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure root");
    let scope = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind,
            parent_scope_id: parent.or(Some(root.id)),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .unwrap_or_else(|err| panic!("create {kind} {slug:?}: {err}"));
    tx.commit().await.expect("commit create");
    scope
}

/// root → eng (org_unit) → platform (org_unit): the shape the placement
/// convention used to seed, minus everything the cutover deleted (CPR-7,
/// ADR-0074). Returns (root, eng, platform).
async fn seed(pool: &PgPool, tenant: TenantId) -> (Scope, Scope, Scope) {
    let mut tx = tx(pool).await;
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure root");
    tx.commit().await.expect("commit root");
    let eng = add(pool, tenant, Some(root.id), ScopeKind::OrgUnit, "eng").await;
    let platform = add(pool, tenant, Some(eng.id), ScopeKind::OrgUnit, "platform").await;
    (root, eng, platform)
}

/// The cutover's provisioning shape: the subject's own principal scope,
/// minted in the same transaction as the identity row that binds it.
async fn provision(
    pool: &PgPool,
    tenant: TenantId,
    subject: &str,
    _parent: ScopeId,
) -> synveda_types::Identity {
    let mut tx = tx(pool).await;
    let personal = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint own scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        Some(&format!("{subject}@example.test")),
        Some(subject),
        personal.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit provision");
    identity
}

// ── Identities ───────────────────────────────────────────────────────────────

#[test]
fn create_then_by_subject_roundtrips_bound_to_the_own_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (root, _, _) = seed(&db.pool, tenant).await;

        let alice = provision(&db.pool, tenant, "alice", root.id).await;
        let read = identities::by_subject(&db.pool, tenant, "alice")
            .await
            .expect("read alice")
            .expect("alice exists");
        assert_eq!(read, alice);
        assert_eq!(read.email.as_deref(), Some("alice@example.test"));
        // The identity's scope is its own principal scope at the root —
        // placement is identity, not convention (CPR-7, ADR-0074).
        assert_eq!(
            alice.scope_id,
            scopes::principal_scope(&db.pool, tenant, "alice")
                .await
                .expect("read own scope")
                .expect("alice has a scope")
                .id
        );

        // A second subject gets a second own scope; nobody is quarantined,
        // because quarantine is only ever "not provisioned" now.
        let bob = provision(&db.pool, tenant, "bob", root.id).await;
        assert_ne!(alice.scope_id, bob.scope_id);

        // Unknown subjects and foreign tenants read as absent.
        assert_eq!(
            identities::by_subject(&db.pool, tenant, "nobody")
                .await
                .expect("read nobody"),
            None
        );
        assert_eq!(
            identities::by_subject(&db.pool, TenantId::new(), "alice")
                .await
                .expect("read cross-tenant"),
            None
        );
    });
}

/// The first-login race resolves at the unique constraint: the second
/// insert for a subject is a Conflict the caller retries into a read.
#[test]
fn duplicate_subject_is_a_conflict() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (root, _, _) = seed(&db.pool, tenant).await;
        provision(&db.pool, tenant, "alice", root.id).await;

        let mut tx = tx(&db.pool).await;
        let second_scope = scopes::ensure_principal_scope(&mut tx, tenant, "alice-second", "alice")
            .await
            .expect("mint second own scope");
        let result = identities::create(
            &mut tx,
            IdentityId::new(),
            tenant,
            Some("alice"),
            IdentityKind::User,
            None,
            None,
            second_scope.id,
        )
        .await;
        assert!(
            matches!(result, Err(Error::Conflict { .. })),
            "a duplicate subject must be a Conflict, got {result:?}"
        );
    });
}

// ── Deleted with the placement convention (CPR-7, ADR-0074) ─────────────────
//
// `teams_matching`, `child_by_slug` and the `group_mappings` roundtrip
// tested the `synveda-{dept}-{team}` convention and its override table;
// both left with the hierarchy, and AUTH-2's placement story is the
// own-scope minting the tests above cover.

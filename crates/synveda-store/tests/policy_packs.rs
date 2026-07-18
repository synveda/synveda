//! AUTHZ-1 store contract for `policy_packs`: apply upserts and owns the
//! version bump, `active` reads the current row, `clear` drops back to the
//! embedded default. The adversarial RLS coverage lives in `tests/rls.rs`
//! (ADR-0009 structural rule).
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::{policy_packs, rls, tenants};
use synveda_types::{Error, TenantId, TenantStatus};

/// Connects and migrates. `None` = no database configured; the test skips
/// quietly.
async fn db() -> Option<PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping policy pack tests: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    Some(pool)
}

async fn admit_tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    let slug = format!("pack-{}", id.as_uuid().simple());
    tenants::create(pool, id, &slug, "AUTHZ-1 pack test", TenantStatus::Active)
        .await
        .expect("admit tenant");
    id
}

#[tokio::test]
async fn apply_bumps_versions_and_clear_removes() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    // One tenant transaction, dropped at the end: the fixture leaves no
    // pack rows behind on the shared dev database.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");

    assert_eq!(
        policy_packs::active(&mut *tx, tenant)
            .await
            .expect("read empty"),
        None,
        "a fresh tenant runs the embedded default"
    );

    let first = policy_packs::apply(&mut *tx, tenant, "authz1-test", "permit (p) v1;")
        .await
        .expect("first apply");
    assert_eq!(first.version, 1);
    assert_eq!(first.name, "authz1-test");

    // Every apply is a new version, even with identical content — the
    // reloader's unchanged-skip and the decision log both see the change.
    let second = policy_packs::apply(&mut *tx, tenant, "authz1-strict", "permit (p) v1;")
        .await
        .expect("second apply");
    assert_eq!(second.version, 2);
    assert_eq!(second.name, "authz1-strict");

    let active = policy_packs::active(&mut *tx, tenant)
        .await
        .expect("read active")
        .expect("a pack is stored");
    assert_eq!(active, second);

    assert!(
        policy_packs::clear(&mut *tx, tenant).await.expect("clear"),
        "clear removes the stored pack"
    );
    assert!(
        !policy_packs::clear(&mut *tx, tenant)
            .await
            .expect("second clear"),
        "second clear is a no-op"
    );
    assert_eq!(
        policy_packs::active(&mut *tx, tenant)
            .await
            .expect("read after clear"),
        None
    );
}

#[tokio::test]
async fn constraints_map_onto_the_taxonomy() {
    let Some(pool) = db().await else { return };
    let tenant = admit_tenant(&pool).await;
    // One transaction per failing statement: a constraint violation aborts
    // the whole Postgres transaction, so cases cannot share one.

    // Malformed pack name (slug grammar).
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let bad_name = policy_packs::apply(&mut *tx, tenant, "Not A Slug!", "permit;").await;
    assert!(
        matches!(bad_name, Err(Error::Invalid { .. })),
        "malformed name must be Invalid, got {bad_name:?}"
    );
    drop(tx);

    // Empty source.
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("begin tenant tx");
    let empty = policy_packs::apply(&mut *tx, tenant, "authz1-empty", "").await;
    assert!(
        matches!(empty, Err(Error::Invalid { .. })),
        "empty source must be Invalid, got {empty:?}"
    );
    drop(tx);

    // Unknown tenant: the FK reports it as NotFound.
    let ghost = TenantId::new();
    let mut tx = rls::begin_tenant_tx(&pool, ghost)
        .await
        .expect("begin ghost tx");
    let orphan = policy_packs::apply(&mut *tx, ghost, "authz1-ghost", "permit;").await;
    assert!(
        matches!(orphan, Err(Error::NotFound { .. })),
        "unknown tenant must be NotFound, got {orphan:?}"
    );
}

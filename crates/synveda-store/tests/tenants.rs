//! TEN-1 store tests: tenant admit/resolve round-trip and conflict handling.
//!
//! These tests need a live Postgres. They read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database); run them locally with
//! `make db-test`. Isolation is by freshly minted UUIDv7 ids and per-run
//! slugs, so a shared dev database is fine.

use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::tenants;
use synveda_types::{Error, TenantId, TenantStatus};

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
                    "skipping tenant tests: DATABASE_URL is not set \
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

/// A slug that is unique per test run (the table has a global unique
/// constraint and the dev database is shared).
fn fresh_slug(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::now_v7().simple())
}

#[test]
fn create_then_resolve_by_id_roundtrips() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let id = TenantId::new();
        let slug = fresh_slug("acme");
        let created = tenants::create(&db.pool, id, &slug, "ACME Bank", TenantStatus::Active)
            .await
            .expect("create tenant");
        assert_eq!(created.id, id);
        assert_eq!(created.slug, slug);
        assert_eq!(created.name, "ACME Bank");
        assert_eq!(created.status, TenantStatus::Active);

        let resolved = tenants::by_id(&db.pool, id)
            .await
            .expect("resolve tenant")
            .expect("tenant exists");
        assert_eq!(resolved, created);
    });
}

#[test]
fn unknown_id_resolves_to_none() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let missing = tenants::by_id(&db.pool, TenantId::new())
            .await
            .expect("query ok");
        assert_eq!(missing, None);
    });
}

#[test]
fn suspended_status_is_stored_and_returned() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let id = TenantId::new();
        let created = tenants::create(
            &db.pool,
            id,
            &fresh_slug("frozen"),
            "Frozen Corp",
            TenantStatus::Suspended,
        )
        .await
        .expect("create suspended tenant");
        assert_eq!(created.status, TenantStatus::Suspended);
    });
}

#[test]
fn duplicate_id_and_duplicate_slug_are_conflicts() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let id = TenantId::new();
        let slug = fresh_slug("dup");
        tenants::create(&db.pool, id, &slug, "First", TenantStatus::Active)
            .await
            .expect("create tenant");

        let dup_id = tenants::create(
            &db.pool,
            id,
            &fresh_slug("dup"),
            "Same id",
            TenantStatus::Active,
        )
        .await;
        assert!(
            matches!(dup_id, Err(Error::Conflict { .. })),
            "duplicate id must be a conflict, got {dup_id:?}"
        );

        let dup_slug = tenants::create(
            &db.pool,
            TenantId::new(),
            &slug,
            "Same slug",
            TenantStatus::Active,
        )
        .await;
        assert!(
            matches!(dup_slug, Err(Error::Conflict { .. })),
            "duplicate slug must be a conflict, got {dup_slug:?}"
        );
    });
}

#[test]
fn malformed_slug_is_invalid_not_storage() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        for bad in ["", "-leading-hyphen", "Has-Uppercase", "spaced out"] {
            let result =
                tenants::create(&db.pool, TenantId::new(), bad, "Bad", TenantStatus::Active).await;
            assert!(
                matches!(result, Err(Error::Invalid { .. })),
                "slug {bad:?} must be rejected as invalid, got {result:?}"
            );
        }
    });
}

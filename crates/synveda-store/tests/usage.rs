//! Durable tenant usage under the ordinary gateway role and forced RLS.
//!
//! A charge is one conditional statement inside the caller's transaction: it
//! lands only within the effective daily limit, which an unexpired override
//! can narrow but never raise, and concurrent charges serialise on the day's
//! row. The runtime role can neither delete usage nor read another tenant's.

#[path = "support/tenant_fixture.rs"]
mod tenant_fixture;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::rls;
use synveda_store::usage::{self, Meter, NewOverride};
use synveda_types::{Error, TenantId, TenantStatus};

async fn pool() -> Option<PgPool> {
    let url = tenant_fixture::database_url("tenant usage tests")?;
    Some(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .expect("connect to DATABASE_URL"),
    )
}

async fn tenant(pool: &PgPool) -> TenantId {
    let id = TenantId::new();
    tenant_fixture::create(
        pool,
        id,
        &format!("usage-{}", id.as_uuid().simple()),
        "usage",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    id
}

async fn charge(
    pool: &PgPool,
    tenant: TenantId,
    units: i64,
    ceiling: i64,
    enforce: bool,
) -> Option<usage::Charged> {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let charged = usage::charge(&mut tx, tenant, Meter::ContextRuns, units, ceiling, enforce)
        .await
        .expect("charge");
    tx.commit().await.expect("commit");
    charged
}

async fn used(pool: &PgPool, tenant: TenantId, ceiling: i64) -> usage::Standing {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let standing = usage::standing(&mut tx, tenant, Meter::ContextRuns, ceiling)
        .await
        .expect("standing");
    tx.commit().await.expect("commit");
    standing
}

async fn narrow(pool: &PgPool, tenant: TenantId, daily_limit: i64, ttl_secs: i64) {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    usage::set_override(
        &mut tx,
        tenant,
        &NewOverride {
            meter: Meter::ContextRuns,
            daily_limit,
            reason: "incident drill",
            set_by: "ops@example.test",
            ttl_secs,
        },
    )
    .await
    .expect("override");
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn a_charge_lands_only_within_the_effective_limit() {
    let Some(pool) = pool().await else { return };
    let tenant = tenant(&pool).await;
    assert_eq!(
        charge(&pool, tenant, 3, 5, true).await.map(|c| c.units),
        Some(3)
    );
    assert_eq!(
        charge(&pool, tenant, 2, 5, true).await.map(|c| c.units),
        Some(5)
    );
    assert!(
        charge(&pool, tenant, 1, 5, true).await.is_none(),
        "over the ceiling"
    );
    assert_eq!(
        used(&pool, tenant, 5).await.used,
        5,
        "a refusal writes nothing"
    );

    let landed = charge(&pool, tenant, 2, 5, false).await.expect("observed");
    assert_eq!((landed.units, landed.over_budget), (7, true));

    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let error = usage::charge(&mut tx, tenant, Meter::ContextRuns, 0, 5, true)
        .await
        .expect_err("a charge is positive");
    assert!(matches!(error, Error::Invalid { .. }));
}

#[tokio::test]
async fn an_override_only_narrows_and_stops_applying_when_it_expires() {
    let Some(pool) = pool().await else { return };
    let tenant = tenant(&pool).await;
    narrow(&pool, tenant, 1_000, 3_600).await;
    assert_eq!(
        used(&pool, tenant, 10).await.daily_limit,
        10,
        "never above the ceiling"
    );
    narrow(&pool, tenant, 2, 3_600).await;
    assert_eq!(used(&pool, tenant, 10).await.daily_limit, 2);
    assert!(charge(&pool, tenant, 2, 10, true).await.is_some());
    assert!(charge(&pool, tenant, 1, 10, true).await.is_none());
    narrow(&pool, tenant, 0, 3_600).await;
    assert!(
        charge(&pool, tenant, 1, 10, true).await.is_none(),
        "zero pauses the meter"
    );

    let migrator = tenant_fixture::migrator_pool(&pool).await;
    let mut expire = tenant_fixture::begin(&migrator, tenant).await;
    sqlx::query(
        "update tenant_usage_budgets
            set set_at = statement_timestamp() - interval '2 hours',
                expires_at = statement_timestamp() - interval '1 hour'
          where tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .execute(&mut *expire)
    .await
    .expect("age the override");
    expire.commit().await.expect("commit");
    migrator.close().await;
    assert_eq!(used(&pool, tenant, 10).await.daily_limit, 10);
    assert!(charge(&pool, tenant, 1, 10, true).await.is_some());
    let mut tx = rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    assert!(
        usage::overrides(&mut tx, tenant)
            .await
            .expect("read")
            .is_empty()
    );
}

#[tokio::test]
async fn concurrent_charges_sum_exactly_and_stop_at_the_limit() {
    let Some(pool) = pool().await else { return };
    let tenant = tenant(&pool).await;
    let attempts = (0..24).map(|_| {
        let pool = pool.clone();
        tokio::spawn(async move { charge(&pool, tenant, 1, 16, true).await.is_some() })
    });
    let mut landed = 0;
    for attempt in attempts {
        landed += usize::from(attempt.await.expect("joined"));
    }
    assert_eq!(landed, 16);
    assert_eq!(used(&pool, tenant, 16).await.used, 16);
}

#[tokio::test]
async fn usage_is_tenant_private_and_undeletable_by_the_runtime_role() {
    let Some(pool) = pool().await else { return };
    let (acme, globex) = (tenant(&pool).await, tenant(&pool).await);
    charge(&pool, acme, 4, 100, true).await.expect("acme");
    narrow(&pool, acme, 3, 600).await;

    let mut tx = rls::begin_tenant_tx(&pool, globex)
        .await
        .expect("globex tx");
    let (_, days) = usage::recent(&mut tx, globex, 31).await.expect("recent");
    assert!(days.is_empty(), "another tenant's usage is invisible");
    assert!(
        usage::overrides(&mut tx, globex)
            .await
            .expect("read")
            .is_empty()
    );
    let visible: i64 = sqlx::query_scalar("select count(*) from tenant_usage")
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(visible, 0);
    tx.rollback().await.expect("rollback");

    let mut tx = rls::begin_tenant_tx(&pool, acme).await.expect("acme tx");
    let denied = sqlx::query("delete from tenant_usage where tenant_id = $1")
        .bind(acme.as_uuid())
        .execute(&mut *tx)
        .await
        .expect_err("the runtime role cannot delete usage");
    assert!(denied.to_string().contains("permission denied"), "{denied}");
    tx.rollback().await.expect("rollback");

    let mut tx = rls::begin_tenant_tx(&pool, acme).await.expect("acme tx");
    let (today, days) = usage::recent(&mut tx, acme, 1).await.expect("recent");
    assert_eq!(days.len(), 1);
    assert_eq!((days[0].day, days[0].units), (today, 4));
    let cleared = usage::clear_override(&mut tx, acme, Meter::ContextRuns)
        .await
        .expect("clear")
        .expect("an override was in force");
    assert_eq!(cleared.daily_limit, 3);
    tx.commit().await.expect("commit");
    assert_eq!(used(&pool, acme, 100).await.used, 4, "clearing keeps usage");
}

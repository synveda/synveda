//! TEN-2 acceptance criteria: adversarial suite proving the RLS backstop —
//! direct SQL with the wrong tenant GUC returns zero rows on every
//! tenant-scoped table (ADR-0009).
//!
//! These tests need a live Postgres and a connection role allowed to
//! `SET ROLE synveda_app` (the dev compose superuser is; any role with
//! membership works). They read `DATABASE_URL` and skip with a message when
//! it is unset (CI has no database); run them locally with `make db-test`.
//! Isolation is by freshly minted UUIDv7 tenants, so a shared dev database
//! is fine.
//!
//! Every adversarial check runs inside one transaction as `synveda_app`
//! (non-superuser, no BYPASSRLS — enforcement actually bites, unlike the
//! compose superuser) with the GUC set transaction-locally via
//! `rls::begin_tenant_tx`, exactly the shape data-path code must use.

use std::sync::OnceLock;

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::records::{self, RecordState};
use synveda_store::{rls, tenants};
use synveda_types::{
    Error, IdentityId, RecordClass, RecordId, RecordKind, ScopeId, Sensitivity, TenantId,
    TenantStatus,
};

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
                    "skipping RLS tests: DATABASE_URL is not set \
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

/// Guarantees the next statement runs at a strictly later `now()`, so the
/// archive trigger records a non-empty transaction period (same rationale as
/// the FND-4 tests; 5ms is ample for microsecond resolution).
async fn tick() {
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
}

fn state(content: &str) -> RecordState {
    RecordState {
        scope_id: ScopeId::new(),
        owner_id: IdentityId::new(),
        kind: RecordKind::Derived,
        class: RecordClass::Fact,
        content: content.to_owned(),
        sensitivity: Sensitivity::Internal,
        provenance: serde_json::json!({"source": "ten-2 acceptance test"}),
        valid_from: chrono::Utc::now(),
        valid_to: None,
    }
}

/// Admits a fresh tenant and seeds one record with one archived version, so
/// `records`, `records_history`, and `records_versions` all hold rows for
/// it. Runs on the (RLS-exempt) test connection — the fixture is the world
/// the backstop must then hide.
async fn seed_tenant(pool: &PgPool) -> (TenantId, RecordId) {
    let tenant = TenantId::new();
    let slug = format!("rls-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "RLS fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    let record = RecordId::new();
    records::insert(pool, record, tenant, &state("v1"))
        .await
        .expect("insert record");
    tick().await;
    records::update(pool, record, &state("v2"))
        .await
        .expect("update record")
        .expect("record is current");
    (tenant, record)
}

/// Begins a transaction with the GUC set for `tenant` (unset when `None`),
/// then demotes it to `synveda_app` for the rest of the transaction. `SET
/// LOCAL ROLE` reverts with the transaction, like the GUC itself.
async fn app_tx(pool: &PgPool, tenant: Option<TenantId>) -> Transaction<'static, Postgres> {
    let mut tx = match tenant {
        Some(tenant) => rls::begin_tenant_tx(pool, tenant)
            .await
            .expect("begin tenant transaction"),
        None => pool.begin().await.expect("begin transaction"),
    };
    sqlx::raw_sql("set local role synveda_app")
        .execute(&mut *tx)
        .await
        .expect("SET ROLE synveda_app (the test connection must be allowed to)");
    tx
}

/// Rows of `tenant` visible through each tenant-scoped relation, in the
/// order (records, records_history, records_versions).
async fn visible_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64) {
    let current = sqlx::query_scalar!(
        r#"select count(*) as "count!" from records where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count records");
    let history = sqlx::query_scalar!(
        r#"select count(*) as "count!" from records_history where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count records_history");
    let versions = sqlx::query_scalar!(
        r#"select count(*) as "count!" from records_versions where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count records_versions");
    (current, history, versions)
}

// ── Completeness guard ───────────────────────────────────────────────────────

/// The tables this suite adversarially covers. Extending the schema with a
/// tenant-scoped table means extending this suite; the guard below turns
/// forgetting into a test failure.
const COVERED: &[&str] = &["records", "records_history"];

/// Discovers every tenant-scoped table (structural definition, ADR-0009: any
/// public base table with a `tenant_id` column) and fails unless each is
/// covered here and carries enabled + FORCED row security with at least one
/// policy. Also pins `records_versions` to `security_invoker`, without which
/// the view would evaluate RLS as its owner and bypass the backstop.
#[test]
fn every_tenant_scoped_table_is_covered_and_forced() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tables = sqlx::query!(
            r#"
            select c.relname as "table_name!",
                   c.relrowsecurity as "rls_enabled!",
                   c.relforcerowsecurity as "rls_forced!",
                   (select count(*) from pg_policy p where p.polrelid = c.oid)
                       as "policy_count!"
            from pg_class c
            join pg_namespace n on n.oid = c.relnamespace
            where n.nspname = 'public'
              and c.relkind = 'r'
              and exists (
                  select from pg_attribute a
                  where a.attrelid = c.oid
                    and a.attname = 'tenant_id'
                    and not a.attisdropped
              )
            order by c.relname
            "#
        )
        .fetch_all(&db.pool)
        .await
        .expect("discover tenant-scoped tables");

        let discovered: Vec<&str> = tables.iter().map(|t| t.table_name.as_str()).collect();
        assert_eq!(
            discovered, COVERED,
            "tenant-scoped tables (any table with a tenant_id column) and this \
             suite's covered list have drifted: enable+force RLS, add the \
             policy and grants in the same migration (ADR-0009), then extend \
             this suite"
        );
        for table in &tables {
            assert!(
                table.rls_enabled && table.rls_forced,
                "{} must have row security enabled AND forced (owners are not \
                 exempt), got enabled={} forced={}",
                table.table_name,
                table.rls_enabled,
                table.rls_forced
            );
            assert!(
                table.policy_count > 0,
                "{} has row security but no policy — default-deny would block \
                 the app entirely rather than isolate tenants",
                table.table_name
            );
        }

        let invoker = sqlx::query_scalar!(
            r#"
            select coalesce((
                select lower(opt.option_value) in ('on', 'true', '1', 'yes')
                from pg_options_to_table(c.reloptions) opt
                where opt.option_name = 'security_invoker'
            ), false) as "security_invoker!"
            from pg_class c
            join pg_namespace n on n.oid = c.relnamespace
            where n.nspname = 'public' and c.relname = 'records_versions'
              and c.relkind = 'v'
            "#
        )
        .fetch_one(&db.pool)
        .await
        .expect("inspect records_versions");
        assert!(
            invoker,
            "records_versions must be security_invoker, or as-of queries \
             evaluate RLS as the view owner and bypass the backstop"
        );
    });
}

// ── The headline acceptance test ─────────────────────────────────────────────

/// AC: direct SQL with the wrong tenant GUC returns zero rows on every
/// tenant-scoped table — while the right GUC sees exactly its own rows.
#[test]
fn wrong_tenant_guc_returns_zero_rows_on_every_table() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_tenant(&db.pool).await;
        let (adversary, _) = seed_tenant(&db.pool).await;

        // Adversary's GUC, victim's rows: nothing, on every relation.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_rows(&mut tx, victim).await,
            (0, 0, 0),
            "rows leaked across tenants under the wrong GUC"
        );
        // The adversary still sees its own world — isolation, not denial of
        // service: 1 current row, 1 archived version, 2 versions in the view.
        assert_eq!(visible_rows(&mut tx, adversary).await, (1, 1, 2));
        drop(tx);

        // And the victim's GUC sees the victim's rows.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert_eq!(visible_rows(&mut tx, victim).await, (1, 1, 2));
    });
}

/// A connection that never set the GUC sees nothing at all: the backstop
/// fails closed, it does not fall open.
#[test]
fn unset_guc_returns_zero_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_tenant(&db.pool).await;
        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_rows(&mut tx, tenant).await,
            (0, 0, 0),
            "rows visible without any tenant GUC"
        );
    });
}

/// A malformed GUC value makes tenant-scoped queries error — fail closed —
/// rather than being treated as "no tenant" or, worse, admitting rows.
#[test]
fn malformed_guc_fails_closed() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (_, record) = seed_tenant(&db.pool).await;
        let mut tx = app_tx(&db.pool, None).await;
        sqlx::query_scalar!(
            "select set_config('synveda.tenant_id', $1, true)",
            "not-a-uuid",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("set a malformed GUC");
        let result = records::current(&mut *tx, record).await;
        assert!(
            matches!(result, Err(Error::Storage { .. })),
            "a malformed tenant GUC must error, got {result:?}"
        );
    });
}

/// Writes are checked too: inserting a row for another tenant than the GUC's
/// trips the policy's WITH CHECK (SQLSTATE 42501), surfaced as the internal
/// application-defect error.
#[test]
fn cross_tenant_insert_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_tenant(&db.pool).await;
        let (other, _) = seed_tenant(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = records::insert(&mut *tx, RecordId::new(), other, &state("forged")).await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant insert must be rejected by RLS as an internal \
             defect, got {result:?}"
        );
    });
}

/// Another tenant's record is invisible to update, delete, and every read —
/// the store reports "no such current version", indistinguishable from a
/// record that never existed (no existence oracle across tenants).
#[test]
fn cross_tenant_update_delete_and_reads_see_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_tenant(&db.pool).await;
        let (_, foreign_record) = seed_tenant(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let current = records::current(&mut *tx, foreign_record).await.unwrap();
        assert_eq!(current, None, "cross-tenant read leaked a record");
        let updated = records::update(&mut *tx, foreign_record, &state("hijack"))
            .await
            .unwrap();
        assert_eq!(updated, None, "cross-tenant update found a row");
        let deleted = records::delete(&mut *tx, foreign_record).await.unwrap();
        assert!(!deleted, "cross-tenant delete removed a row");
        let as_of = records::as_of(&mut *tx, foreign_record, chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(as_of, None, "cross-tenant as-of leaked history");
    });
}

/// The backstop must not break legitimate use: the full record lifecycle —
/// including the trigger that archives into `records_history` under the
/// caller's rights — works as `synveda_app`. One tenant transaction per
/// step, the shape a data-path request takes (and `now()` is frozen inside
/// a transaction, so history can only accrue across them).
#[test]
fn same_tenant_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = TenantId::new();
        let slug = format!("rls-{}", tenant.as_uuid().simple());
        tenants::create(
            &db.pool,
            tenant,
            &slug,
            "RLS lifecycle",
            TenantStatus::Active,
        )
        .await
        .expect("create tenant");
        let record = RecordId::new();

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        records::insert(&mut *tx, record, tenant, &state("v1"))
            .await
            .expect("insert under RLS");
        tx.commit().await.expect("commit insert");
        tick().await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        records::update(&mut *tx, record, &state("v2"))
            .await
            .expect("update under RLS (archive trigger runs as synveda_app)")
            .expect("record is current");
        tx.commit().await.expect("commit update");
        tick().await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let versions = records::versions(&mut *tx, record)
            .await
            .expect("versions under RLS");
        assert_eq!(versions.len(), 2, "history must accrue under RLS");
        assert!(
            records::delete(&mut *tx, record).await.expect("delete"),
            "delete must work in-tenant"
        );
        tx.commit().await.expect("commit delete");

        // Deleted from the present, still in history: the as-of surface keeps
        // working for the tenant that owns it.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let current = records::current(&mut *tx, record).await.expect("re-read");
        assert_eq!(current, None);
        let versions = records::versions(&mut *tx, record)
            .await
            .expect("versions after delete");
        assert_eq!(versions.len(), 2);
    });
}

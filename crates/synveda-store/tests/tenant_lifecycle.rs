//! Tenant lifecycle: inventory coverage against the live catalogue, and
//! suspension fencing under ordinary runtime roles (ADR-0157).
//!
//! Needs a live database (`make db-test`). Suspension and resumption run as
//! the exact test migrator; every refusal and canary is observed through the
//! ordinary `DATABASE_URL` role.

#[path = "support/tenant_fixture.rs"]
mod tenant_fixture;

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::tenant_inventory::{DEPLOYMENT_TABLES, TENANT_TABLES};
use synveda_store::tenant_lifecycle::{self, TENANT_BUSY, Transition};
use synveda_store::{rls, scopes, tenant_inventory, tenants};
use synveda_types::{Error, TenantId, TenantStatus};

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = tenant_fixture::database_url("tenant lifecycle tests")?;
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
                .expect("verify the schema epoch");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

async fn admitted(pool: &PgPool, prefix: &str) -> TenantId {
    let id = TenantId::new();
    let slug = format!("{prefix}-{}", uuid::Uuid::now_v7().simple());
    tenant_fixture::create(pool, id, &slug, "lifecycle fixture", TenantStatus::Active)
        .await
        .expect("admit fixture tenant");
    let mut tx = tenant_fixture::begin(pool, id).await;
    scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("create the tenant root");
    tx.commit().await.expect("commit the tenant root");
    id
}

fn roles() -> synveda_store::runtime_role::DatabaseRoles {
    let path = std::env::var_os("SYNVEDA_DATABASE_ROLES_FILE")
        .expect("SYNVEDA_DATABASE_ROLES_FILE names the test role contract");
    let json = std::fs::read_to_string(path).expect("read the test role contract");
    synveda_store::runtime_role::DatabaseRoles::parse_json(json.trim_end())
        .expect("parse the test role contract")
}

/// Performs one operator transition exactly as the CLI does.
async fn transition(
    migrator: &PgPool,
    tenant: TenantId,
    transition: Transition,
) -> synveda_types::Result<bool> {
    let mut tx = rls::begin_migrator_tenant_tx(migrator, tenant, &roles()).await?;
    tenant_lifecycle::lock_exclusive(&mut tx, tenant).await?;
    let changed = tenant_lifecycle::apply(&mut tx, tenant, transition).await?;
    tx.commit().await.map_err(|error| Error::Storage {
        message: error.to_string(),
    })?;
    Ok(changed)
}

fn assert_unavailable<T>(result: synveda_types::Result<T>, what: &str) {
    match result {
        Err(error) => assert!(
            rls::is_tenant_unavailable(&error),
            "{what} refused with the wrong error: {error}"
        ),
        Ok(_) => panic!("{what} was admitted for a tenant that is not active"),
    }
}

// ── Inventory coverage ──────────────────────────────────────────────────────

#[test]
fn the_inventory_classifies_exactly_the_catalogue_tables() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let rows = sqlx::query!(
            r#"
            select c.relname as "name!",
                   exists (
                       select from pg_catalog.pg_attribute a
                       where a.attrelid = c.oid and a.attname = 'tenant_id'
                         and not a.attisdropped
                   ) as "tenant_bearing!",
                   c.relrowsecurity and c.relforcerowsecurity as "forced!"
            from pg_catalog.pg_class c
            join pg_catalog.pg_namespace n on n.oid = c.relnamespace
            where n.nspname = 'public' and c.relkind in ('r', 'p')
            order by c.relname
            "#
        )
        .fetch_all(&db.pool)
        .await
        .expect("read the live catalogue");
        let tenant_bearing: HashSet<&str> = rows
            .iter()
            .filter(|row| row.tenant_bearing)
            .map(|row| row.name.as_str())
            .collect();
        let deployment: HashSet<&str> = rows
            .iter()
            .filter(|row| !row.tenant_bearing)
            .map(|row| row.name.as_str())
            .collect();
        let classified: HashSet<&str> = TENANT_TABLES.iter().map(|table| table.name).collect();
        let named: HashSet<&str> = DEPLOYMENT_TABLES.iter().map(|table| table.name).collect();
        assert_eq!(
            tenant_bearing.difference(&classified).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "classify every new tenant-bearing table in the tenant inventory"
        );
        assert_eq!(
            classified.difference(&tenant_bearing).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "the tenant inventory names a table the catalogue does not hold"
        );
        assert_eq!(deployment, named, "deployment tables drifted");
        for row in rows.iter().filter(|row| row.tenant_bearing) {
            assert!(row.forced, "{} lacks forced row security", row.name);
        }
    });
}

#[test]
fn restore_order_puts_every_immediate_parent_first() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let edges = sqlx::query!(
            r#"
            select child.relname as "child!", parent.relname as "parent!",
                   k.conname as "name!", k.condeferrable as "deferrable!"
            from pg_catalog.pg_constraint k
            join pg_catalog.pg_class child on child.oid = k.conrelid
            join pg_catalog.pg_class parent on parent.oid = k.confrelid
            where k.contype = 'f' and k.connamespace = 'public'::regnamespace
            order by k.conname
            "#
        )
        .fetch_all(&db.pool)
        .await
        .expect("read foreign keys");
        let position: HashMap<&str, usize> = TENANT_TABLES
            .iter()
            .enumerate()
            .map(|(index, table)| (table.name, index))
            .collect();
        let mut checked = 0;
        for edge in &edges {
            let (Some(child), Some(parent)) = (
                position.get(edge.child.as_str()),
                position.get(edge.parent.as_str()),
            ) else {
                continue;
            };
            if edge.deferrable {
                continue;
            }
            assert!(
                parent < child,
                "{} needs {} restored first, or the key must be deferrable",
                edge.child,
                edge.parent
            );
            checked += 1;
        }
        assert!(checked > 100, "only {checked} tenant foreign keys checked");
    });
}

// ── Suspension fencing ──────────────────────────────────────────────────────

#[test]
fn suspension_refuses_later_ordinary_work_and_keeps_custody_and_canaries() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "suspend").await;
        let canary = admitted(&db.pool, "canary").await;

        assert!(
            transition(&migrator, tenant, Transition::Suspend)
                .await
                .unwrap()
        );
        assert!(
            !transition(&migrator, tenant, Transition::Suspend)
                .await
                .unwrap(),
            "a repeated suspension must change nothing"
        );
        assert_unavailable(
            rls::begin_tenant_tx(&db.pool, tenant).await,
            "shared admission",
        );
        assert_unavailable(
            rls::begin_tenant_write_tx(&db.pool, tenant).await,
            "exclusive admission",
        );
        rls::begin_tenant_custody_tx(&db.pool, tenant)
            .await
            .expect("key custody stays available while suspended")
            .rollback()
            .await
            .unwrap();
        let mut snapshot = rls::begin_tenant_snapshot_tx(&db.pool, tenant)
            .await
            .expect("a read-only snapshot stays available while suspended");
        assert_eq!(
            tenants::by_id(&mut *snapshot, tenant)
                .await
                .unwrap()
                .map(|t| t.status),
            Some(TenantStatus::Suspended)
        );
        snapshot.rollback().await.unwrap();
        rls::begin_tenant_tx(&db.pool, canary)
            .await
            .expect("another tenant keeps working")
            .rollback()
            .await
            .unwrap();

        assert!(
            transition(&migrator, tenant, Transition::Resume)
                .await
                .unwrap()
        );
        rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("a resumed tenant is admitted again")
            .rollback()
            .await
            .unwrap();
    });
}

#[test]
fn an_unknown_tenant_is_refused_by_every_admission() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let unknown = TenantId::new();
        assert_unavailable(rls::begin_tenant_tx(&db.pool, unknown).await, "shared");
        assert_unavailable(
            rls::begin_tenant_custody_tx(&db.pool, unknown).await,
            "custody",
        );
    });
}

#[test]
fn suspension_waits_for_admitted_work_then_fences_its_waiters() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "fence").await;
        let mut holder = tenant_fixture::begin(&db.pool, tenant).await;
        let holder_pid = tenant_fixture::backend_pid(&mut holder).await;

        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
        let suspender = tokio::spawn({
            let migrator = migrator.clone();
            async move {
                let mut tx = rls::begin_migrator_tenant_tx(&migrator, tenant, &roles())
                    .await
                    .expect("begin migrator transaction");
                let pid: i32 = sqlx::query_scalar("select pg_catalog.pg_backend_pid()")
                    .fetch_one(&mut *tx)
                    .await
                    .expect("read suspender pid");
                pid_tx.send(pid).expect("report suspender pid");
                tenant_lifecycle::lock_exclusive(&mut tx, tenant)
                    .await
                    .expect("suspension acquires the lock once admitted work ends");
                let changed = tenant_lifecycle::apply(&mut tx, tenant, Transition::Suspend)
                    .await
                    .expect("suspend");
                tx.commit().await.expect("commit suspension");
                changed
            }
        });
        let suspender_pid = pid_rx.await.expect("suspender pid");
        let mut observer = db.pool.begin().await.expect("observer transaction");
        tenant_fixture::wait_until_blocked_by(&mut observer, suspender_pid, holder_pid).await;

        let late = tokio::spawn({
            let pool = db.pool.clone();
            async move { rls::begin_tenant_tx(&pool, tenant).await.map(|_| ()) }
        });
        wait_for_advisory_waiters(&mut observer, tenant, 2).await;

        sqlx::query_scalar::<_, i64>("select count(*) from public.scopes")
            .fetch_one(&mut *holder)
            .await
            .expect("admitted work still reads after suspension was requested");
        holder.commit().await.expect("admitted work completes");
        assert!(
            suspender.await.expect("suspender task"),
            "suspension applied"
        );
        assert_unavailable(late.await.expect("late task"), "a transaction that waited");
        observer.rollback().await.unwrap();
    });
}

/// Polls until `expected` sessions wait on this tenant's admission lock.
/// Bounded; the lock graph is the assertion, never a sleep.
async fn wait_for_advisory_waiters(
    observer: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    expected: i64,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "select count(*) from pg_catalog.pg_locks \
                 where locktype = 'advisory' and not granted and objsubid = 1 \
                   and ((classid::int8 << 32) | objid::int8) \
                       = hashtextextended('synveda.admission:' || $1, 0)",
            )
            .bind(tenant.as_uuid().to_string())
            .fetch_one(&mut **observer)
            .await
            .expect("inspect advisory waiters");
            if waiting >= expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the late transaction did not queue behind the suspension");
}

#[test]
fn a_busy_tenant_refuses_suspension_and_stays_active() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "busy").await;
        let holder = tenant_fixture::begin(&db.pool, tenant).await;
        match transition(&migrator, tenant, Transition::Suspend).await {
            Err(Error::Conflict { message }) => assert_eq!(message, TENANT_BUSY),
            other => panic!("expected {TENANT_BUSY}, got {other:?}"),
        }
        holder.rollback().await.unwrap();
        assert_eq!(
            tenants::by_id(&db.pool, tenant)
                .await
                .unwrap()
                .map(|t| t.status),
            Some(TenantStatus::Active)
        );
    });
}

#[test]
fn the_dry_run_counts_only_the_named_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "inventory").await;
        let canary = admitted(&db.pool, "inventory-canary").await;
        assert!(
            transition(&migrator, tenant, Transition::Suspend)
                .await
                .unwrap()
        );
        let mut tx = rls::begin_tenant_snapshot_tx(&db.pool, tenant)
            .await
            .unwrap();
        let counts = tenant_inventory::count_tenant_rows(&mut tx, tenant, 10)
            .await
            .expect("count a suspended tenant");
        tx.rollback().await.unwrap();
        assert_eq!(counts.len(), TENANT_TABLES.len());
        let scope_rows = counts.iter().find(|count| count.table == "scopes").unwrap();
        assert_eq!((scope_rows.rows, scope_rows.capped), (1, false));

        let mut tx = rls::begin_tenant_snapshot_tx(&db.pool, canary)
            .await
            .unwrap();
        let canary_counts = tenant_inventory::count_tenant_rows(&mut tx, canary, 1)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        let canary_scopes = canary_counts.iter().find(|count| count.table == "scopes");
        assert_eq!(canary_scopes.map(|count| count.rows), Some(1));
        assert!(
            tenant_inventory::count_tenant_rows(&mut db.pool.acquire().await.unwrap(), tenant, 0)
                .await
                .is_err(),
            "a zero cap is refused"
        );
    });
}

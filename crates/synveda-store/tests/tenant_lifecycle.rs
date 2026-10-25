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
use synveda_store::tenant_inventory::{DEPLOYMENT_TABLES, Erasure, TENANT_TABLES};
use synveda_store::tenant_lifecycle::{self, TENANT_BUSY, Transition};
use synveda_store::{rls, scopes, tenant_erasure, tenant_inventory, tenants};
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

// ── Erasure ─────────────────────────────────────────────────────────────────

/// One erasure transaction exactly as the CLI opens it.
async fn locked(migrator: &PgPool, tenant: TenantId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = rls::begin_migrator_tenant_tx(migrator, tenant, &roles())
        .await
        .expect("migrator tenant transaction");
    tenant_lifecycle::lock_exclusive(&mut tx, tenant)
        .await
        .expect("exclusive admission");
    tx
}

const LIMITS: tenant_erasure::StepLimits = tenant_erasure::StepLimits {
    chunk: 1,
    lease: std::time::Duration::from_secs(1),
    statement_timeout: std::time::Duration::from_secs(30),
};

#[test]
fn the_erasure_plan_deletes_every_referencing_row_before_its_parent() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mut connection = db.pool.acquire().await.unwrap();
        let plan = tenant_erasure::plan(&mut connection)
            .await
            .expect("the live catalogue has only declared cycles");
        let position: HashMap<&str, usize> = plan
            .units()
            .iter()
            .enumerate()
            .flat_map(|(index, unit)| unit.tables().into_iter().map(move |table| (table, index)))
            .collect();
        let erased = TENANT_TABLES
            .iter()
            .filter(|table| table.erasure != Erasure::RetainEvidence)
            .count();
        assert_eq!(position.len(), erased, "every erased table is planned once");
        let edges: Vec<(String, String)> = sqlx::query_as(
            "select child.relname::text, parent.relname::text
               from pg_catalog.pg_constraint fk
               join pg_catalog.pg_class child on child.oid = fk.conrelid
               join pg_catalog.pg_class parent on parent.oid = fk.confrelid
              where fk.contype = 'f' and fk.connamespace = 'public'::regnamespace",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        let mut checked = 0;
        for (child_table, parent_table) in &edges {
            if let (Some(child), Some(parent)) = (
                position.get(child_table.as_str()),
                position.get(parent_table.as_str()),
            ) {
                assert!(
                    child <= parent,
                    "{child_table} references {parent_table} but is planned after it"
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "the plan was checked against {checked} keys");
    });
}

#[test]
fn erasure_guards_admit_deletion_only_for_an_erasing_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "erasure-guard").await;
        assert!(
            transition(&migrator, tenant, Transition::Suspend)
                .await
                .unwrap()
        );
        // The owner naming a suspended tenant still meets the append-only guard.
        let mut tx = locked(&migrator, tenant).await;
        sqlx::query("select set_config('synveda.tenant_erasure', $1, true)")
            .bind(tenant.as_uuid().to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        let refused = sqlx::query("delete from durable_operations where tenant_id = $1")
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("a suspended tenant is not erasing");
        assert!(refused.to_string().contains("append-only"), "{refused}");
        tx.rollback().await.unwrap();

        // The ordinary role naming an erasing tenant is not the owner.
        let mut tx = locked(&migrator, tenant).await;
        let plan = tenant_erasure::plan(&mut tx).await.unwrap();
        let start = tenant_erasure::start(&mut tx, tenant, &plan, "guard-test", LIMITS.lease)
            .await
            .unwrap();
        assert!(matches!(start, tenant_erasure::Start::Started(_)));
        tx.commit().await.unwrap();
        let mut ordinary = db.pool.begin().await.unwrap();
        sqlx::query(
            "select set_config('synveda.tenant_id', $1, true), set_config('synveda.tenant_erasure', $1, true)",
        )
        .bind(tenant.as_uuid().to_string())
        .execute(&mut *ordinary)
        .await
        .unwrap();
        let authorized: bool =
            sqlx::query_scalar("select public.synveda_tenant_erasure_authorized($1)")
                .bind(tenant.as_uuid())
                .fetch_one(&mut *ordinary)
                .await
                .unwrap();
        assert!(!authorized, "only the database owner is authorized");
        ordinary.rollback().await.unwrap();
        assert_unavailable(
            rls::begin_tenant_custody_tx(&db.pool, tenant).await,
            "custody of an erasing tenant",
        );
        let custody = tenants::custody_ids(&db.pool, 100_000).await.unwrap();
        assert!(!custody.contains(&tenant), "KEK replacement skips an erasing tenant");
        assert!(tenants::all_ids(&db.pool, 100_000).await.unwrap().contains(&tenant));
    });
}

#[test]
fn an_interrupted_erasure_resumes_under_a_reclaimed_lease() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let migrator = tenant_fixture::migrator_pool(&db.pool).await;
        let tenant = admitted(&db.pool, "erasure-resume").await;
        let mut tx = tenant_fixture::begin(&db.pool, tenant).await;
        scopes::ensure_tenant_root(&mut tx, tenant).await.unwrap();
        scopes::ensure_principal_scope(&mut tx, tenant, "first", "First")
            .await
            .unwrap();
        scopes::ensure_principal_scope(&mut tx, tenant, "second", "Second")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert!(
            transition(&migrator, tenant, Transition::Suspend)
                .await
                .unwrap()
        );
        let mut tx = rls::begin_tenant_snapshot_tx(&db.pool, tenant)
            .await
            .unwrap();
        let before: u64 = tenant_inventory::count_tenant_rows(&mut tx, tenant, 1_000)
            .await
            .unwrap()
            .iter()
            .filter(|count| count.erasure != Erasure::RetainEvidence)
            .map(|count| count.rows)
            .sum();
        tx.rollback().await.unwrap();
        assert!(before >= 3, "a root and two principal scopes: {before}");

        let mut tx = locked(&migrator, tenant).await;
        let plan = tenant_erasure::plan(&mut tx).await.unwrap();
        let tenant_erasure::Start::Started(first) =
            tenant_erasure::start(&mut tx, tenant, &plan, "worker-a", LIMITS.lease)
                .await
                .unwrap()
        else {
            panic!("a suspended tenant starts erasing");
        };
        tx.commit().await.unwrap();
        // Worker A deletes until it has removed one scope, then stops without
        // releasing its lease, as a crash would.
        let mut deleted = 0;
        for _ in 0..plan.units().len() * 4 {
            let mut tx = locked(&migrator, tenant).await;
            let progress = tenant_erasure::step(&mut tx, tenant, &plan, &first, LIMITS)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            deleted += progress.deleted;
            if progress.label.as_deref() == Some("leaves:scopes") && progress.deleted > 0 {
                break;
            }
        }
        assert!(deleted >= 1);

        let mut tx = locked(&migrator, tenant).await;
        let busy = tenant_erasure::start(&mut tx, tenant, &plan, "worker-b", LIMITS.lease)
            .await
            .expect_err("a live lease is not reclaimed");
        assert!(
            busy.to_string()
                .contains(tenant_erasure::ERASURE_IN_PROGRESS),
            "{busy}"
        );
        tx.rollback().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        let mut tx = locked(&migrator, tenant).await;
        let tenant_erasure::Start::Resumed(second) =
            tenant_erasure::start(&mut tx, tenant, &plan, "worker-b", LIMITS.lease)
                .await
                .unwrap()
        else {
            panic!("an expired lease is reclaimed");
        };
        tx.commit().await.unwrap();
        assert_eq!(second.attempt, 2);
        let mut tx = locked(&migrator, tenant).await;
        let lost = tenant_erasure::step(&mut tx, tenant, &plan, &first, LIMITS)
            .await
            .expect_err("the first worker lost its lease");
        assert!(
            lost.to_string()
                .contains(tenant_erasure::ERASURE_LEASE_LOST),
            "{lost}"
        );
        tx.rollback().await.unwrap();

        let mut finished = false;
        for _ in 0..plan.units().len() * 8 {
            let mut tx = locked(&migrator, tenant).await;
            let progress = tenant_erasure::step(&mut tx, tenant, &plan, &second, LIMITS)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            if progress.finished {
                finished = true;
                break;
            }
        }
        assert!(finished, "every unit ran within its bound");
        let mut tx = locked(&migrator, tenant).await;
        let receipt = tenant_erasure::finish(&mut tx, tenant, &plan, &second, LIMITS)
            .await
            .expect("an empty residual inventory");
        tx.commit().await.unwrap();
        assert_eq!(receipt["attempts"], 2);
        assert_eq!(receipt["deleted_rows"], before, "no row was counted twice");
        assert_eq!(receipt["deleted"]["scopes"], 3);
        assert_eq!(
            tenants::by_id(&db.pool, tenant)
                .await
                .unwrap()
                .unwrap()
                .status,
            TenantStatus::Erased
        );
        let mut tx = locked(&migrator, tenant).await;
        let again = tenant_erasure::start(&mut tx, tenant, &plan, "worker-c", LIMITS.lease)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(again, tenant_erasure::Start::Erased(receipt));
    });
}

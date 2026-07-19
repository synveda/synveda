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
use synveda_store::{
    group_mappings, hierarchy, identities, policy_assignments, policy_packs, rls, role_bindings,
    tenants,
};
use synveda_types::{
    Error, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Role, ScopeId, ScopeKind,
    Sensitivity, TenantId, TenantStatus,
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
const COVERED: &[&str] = &[
    "audit_chain_heads",
    "audit_log",
    "group_mappings",
    "hierarchy_closure",
    "hierarchy_nodes",
    "identities",
    "policy_pack_assignments",
    "policy_pack_defaults",
    "policy_packs",
    "records",
    "records_history",
    "role_bindings",
];

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

// ── Hierarchy tables (HIER-1, ADR-0011) ─────────────────────────────────────

/// Rows of `tenant` visible through the hierarchy tables, in the order
/// (hierarchy_nodes, hierarchy_closure).
async fn visible_hierarchy_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let nodes = sqlx::query_scalar!(
        r#"select count(*) as "count!" from hierarchy_nodes where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count hierarchy_nodes");
    let closure = sqlx::query_scalar!(
        r#"select count(*) as "count!" from hierarchy_closure where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count hierarchy_closure");
    (nodes, closure)
}

/// Admits a tenant with an org root and one team: 2 nodes, 3 closure rows
/// (two self-rows + one edge). Runs on the (RLS-exempt) test connection.
async fn seed_hierarchy(pool: &PgPool) -> (TenantId, ScopeId) {
    let tenant = TenantId::new();
    let slug = format!("rlsh-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS hierarchy fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "acme",
        "ACME",
    )
    .await
    .expect("create org");
    hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        "core",
        "Core",
    )
    .await
    .expect("create team");
    tx.commit().await.expect("commit hierarchy");
    (tenant, org.id)
}

/// The wrong (or absent) tenant GUC sees zero hierarchy rows; the right one
/// sees exactly its own.
#[test]
fn wrong_tenant_guc_sees_no_hierarchy_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_hierarchy(&db.pool).await;
        let (adversary, _) = seed_hierarchy(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_hierarchy_rows(&mut tx, victim).await,
            (0, 0),
            "hierarchy rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_hierarchy_rows(&mut tx, adversary).await, (2, 3));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_hierarchy_rows(&mut tx, victim).await,
            (0, 0),
            "hierarchy rows visible without any tenant GUC"
        );
    });
}

/// Writing hierarchy rows for another tenant than the GUC's trips the
/// policies' WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_hierarchy_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_hierarchy(&db.pool).await;
        let (other, _) = seed_hierarchy(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            other,
            None,
            ScopeKind::Org,
            "forged",
            "Forged",
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant hierarchy insert must be rejected by RLS as an \
             internal defect, got {result:?}"
        );
    });
}

/// The full hierarchy lifecycle — create, move (closure surgery needs no
/// UPDATE on the closure table), delete — works as `synveda_app` with the
/// right GUC: the backstop isolates, it does not deny service.
#[test]
fn same_tenant_hierarchy_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, org) = seed_hierarchy(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let dept = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(org),
            ScopeKind::Department,
            "pay",
            "Payments",
        )
        .await
        .expect("create under RLS");
        let team = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(org),
            ScopeKind::Team,
            "platform",
            "Platform",
        )
        .await
        .expect("create second child under RLS");
        let moved = hierarchy::move_node(&mut tx, team.id, dept.id)
            .await
            .expect("move under RLS");
        assert_eq!(moved.parent_id, Some(dept.id));
        assert_eq!(moved.path, "acme/pay/platform");
        assert!(
            hierarchy::delete(&mut tx, moved.id)
                .await
                .expect("delete under RLS"),
            "delete must work in-tenant"
        );
        tx.commit().await.expect("commit lifecycle");
    });
}

// ── Policy packs (AUTHZ-1, ADR-0012) ────────────────────────────────────────

/// Admits a tenant with one stored pack. Runs on the (RLS-exempt) test
/// connection.
async fn seed_policy_pack(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("rlsp-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS pack fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    policy_packs::apply(
        pool,
        tenant,
        "rls-fixture",
        "permit (principal, action, resource);",
    )
    .await
    .expect("apply pack");
    tenant
}

async fn visible_pack_rows(tx: &mut Transaction<'static, Postgres>, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from policy_packs where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count policy_packs")
}

/// The wrong (or absent) tenant GUC sees zero pack rows — a tenant can
/// never read another tenant's policy source; the right one sees its own.
#[test]
fn wrong_tenant_guc_sees_no_policy_pack_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_policy_pack(&db.pool).await;
        let adversary = seed_policy_pack(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_pack_rows(&mut tx, victim).await,
            0,
            "policy pack rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_pack_rows(&mut tx, adversary).await, 1);
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_pack_rows(&mut tx, victim).await,
            0,
            "policy pack rows visible without any tenant GUC"
        );
    });
}

/// Writing a pack for another tenant than the GUC's trips the policy's
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_policy_pack_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_policy_pack(&db.pool).await;
        let other = seed_policy_pack(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = policy_packs::apply(&mut *tx, other, "forged", "permit;").await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant pack write must be rejected by RLS as an internal \
             defect, got {result:?}"
        );
    });
}

/// The full pack lifecycle — apply (insert, then version-bumping update
/// of the same name), read, clear — works as `synveda_app` with the right
/// GUC: the shape the gateway's refresher and the CLI take.
#[test]
fn same_tenant_policy_pack_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_policy_pack(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let first = policy_packs::apply(&mut *tx, tenant, "rls-lifecycle", "forbid;")
            .await
            .expect("apply under RLS");
        assert_eq!(first.version, 1, "a new name starts at v1");
        let bumped = policy_packs::apply(&mut *tx, tenant, "rls-lifecycle", "permit;")
            .await
            .expect("re-apply under RLS");
        assert_eq!(bumped.version, 2, "re-applying the name must bump to v2");
        let stored = policy_packs::get(&mut *tx, tenant, "rls-lifecycle")
            .await
            .expect("read under RLS");
        assert_eq!(stored.as_ref(), Some(&bumped));
        assert_eq!(
            policy_packs::stored(&mut *tx, tenant)
                .await
                .expect("list under RLS")
                .len(),
            2,
            "the seeded pack and the lifecycle pack are both stored"
        );
        assert!(
            policy_packs::clear(&mut tx, tenant, "rls-lifecycle")
                .await
                .expect("clear under RLS"),
            "clear must work in-tenant"
        );
    });
}

// ── Policy assignments & defaults (AUTHZ-2, ADR-0014) ───────────────────────

/// Admits a tenant with an org root carrying a pack assignment and a
/// tenant default. Runs on the (RLS-exempt) test connection.
async fn seed_policy_assignment(pool: &PgPool) -> (TenantId, ScopeId) {
    let (tenant, root) = seed_hierarchy(pool).await;
    policy_assignments::assign(pool, tenant, root, "open-collaboration")
        .await
        .expect("assign pack");
    policy_assignments::set_default(pool, tenant, "standard")
        .await
        .expect("set default");
    (tenant, root)
}

async fn visible_assignment_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let assignments = sqlx::query_scalar!(
        r#"select count(*) as "count!" from policy_pack_assignments where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count policy_pack_assignments");
    let defaults = sqlx::query_scalar!(
        r#"select count(*) as "count!" from policy_pack_defaults where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count policy_pack_defaults");
    (assignments, defaults)
}

/// The wrong (or absent) tenant GUC sees zero assignment/default rows —
/// which pack governs which node is itself tenant-private.
#[test]
fn wrong_tenant_guc_sees_no_policy_assignment_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_policy_assignment(&db.pool).await;
        let (adversary, _) = seed_policy_assignment(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_assignment_rows(&mut tx, victim).await,
            (0, 0),
            "assignment rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_assignment_rows(&mut tx, adversary).await, (1, 1));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_assignment_rows(&mut tx, victim).await,
            (0, 0),
            "assignment rows visible without any tenant GUC"
        );
    });
}

/// Writing an assignment or default for another tenant than the GUC's
/// trips the policy's WITH CHECK — an application defect, surfaced as
/// internal.
#[test]
fn cross_tenant_policy_assignment_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_policy_assignment(&db.pool).await;
        let (other, other_root) = seed_policy_assignment(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let forged = policy_assignments::assign(&mut *tx, other, other_root, "standard").await;
        assert!(
            matches!(forged, Err(Error::Internal { .. })),
            "cross-tenant assignment write must be rejected by RLS as an \
             internal defect, got {forged:?}"
        );
        drop(tx);
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let forged_default = policy_assignments::set_default(&mut *tx, other, "standard").await;
        assert!(
            matches!(forged_default, Err(Error::Internal { .. })),
            "cross-tenant default write must be rejected by RLS as an \
             internal defect, got {forged_default:?}"
        );
    });
}

/// The full assignment lifecycle — assign (insert and replacing update),
/// chain lookup, unassign, default set/get/clear — works as `synveda_app`
/// with the right GUC: the shape the gateway's policy routes take.
#[test]
fn same_tenant_policy_assignment_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, root) = seed_policy_assignment(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let replaced = policy_assignments::assign(&mut *tx, tenant, root, "standard")
            .await
            .expect("re-assign under RLS");
        assert_eq!(replaced.pack_name, "standard");
        let for_chain = policy_assignments::for_scopes(&mut *tx, tenant, &[root])
            .await
            .expect("chain lookup under RLS");
        assert_eq!(for_chain, vec![replaced]);
        assert_eq!(
            policy_assignments::default_pack(&mut *tx, tenant)
                .await
                .expect("read default under RLS"),
            Some("standard".to_owned())
        );
        assert!(
            policy_assignments::unassign(&mut *tx, tenant, root)
                .await
                .expect("unassign under RLS"),
            "unassign must work in-tenant"
        );
        assert!(
            policy_assignments::clear_default(&mut *tx, tenant)
                .await
                .expect("clear default under RLS"),
            "clearing the default must work in-tenant"
        );
    });
}

// ── Role bindings (AUTHZ-3, ADR-0015) ───────────────────────────────────────

/// Admits a tenant with an org root carrying a node binding plus a
/// tenant-wide binding. Runs on the (RLS-exempt) test connection.
async fn seed_role_bindings(pool: &PgPool) -> (TenantId, ScopeId) {
    let (tenant, root) = seed_hierarchy(pool).await;
    role_bindings::bind(pool, tenant, "rls-steward", Some(root), Role::Steward)
        .await
        .expect("bind steward at root");
    role_bindings::bind(pool, tenant, "rls-admin", None, Role::OrgAdmin)
        .await
        .expect("bind tenant-wide org-admin");
    (tenant, root)
}

async fn visible_binding_rows(tx: &mut Transaction<'static, Postgres>, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from role_bindings where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count role_bindings")
}

/// The wrong (or absent) tenant GUC sees zero binding rows — who holds
/// which role where is itself tenant-private.
#[test]
fn wrong_tenant_guc_sees_no_role_binding_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_role_bindings(&db.pool).await;
        let (adversary, _) = seed_role_bindings(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_binding_rows(&mut tx, victim).await,
            0,
            "binding rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_binding_rows(&mut tx, adversary).await, 2);
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_binding_rows(&mut tx, victim).await,
            0,
            "binding rows visible without any tenant GUC"
        );
    });
}

/// Writing a binding for another tenant than the GUC's trips the policy's
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_role_binding_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_role_bindings(&db.pool).await;
        let (other, other_root) = seed_role_bindings(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let forged =
            role_bindings::bind(&mut *tx, other, "mallory", Some(other_root), Role::OrgAdmin).await;
        assert!(
            matches!(forged, Err(Error::Internal { .. })),
            "cross-tenant binding write must be rejected by RLS as an \
             internal defect, got {forged:?}"
        );
    });
}

/// The full binding lifecycle — bind (node and tenant-wide), the chain
/// query, per-node and tenant listings, unbind — works as `synveda_app`
/// with the right GUC: the shape the gateway's roles routes take.
#[test]
fn same_tenant_role_binding_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, root) = seed_role_bindings(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let bound = role_bindings::bind(&mut *tx, tenant, "rls-viewer", Some(root), Role::Viewer)
            .await
            .expect("bind under RLS");
        assert_eq!(bound.role, Role::Viewer);
        let for_chain =
            role_bindings::for_subject_on_scopes(&mut *tx, tenant, "rls-viewer", &[root])
                .await
                .expect("chain query under RLS");
        assert_eq!(for_chain, vec![bound]);
        assert_eq!(
            role_bindings::for_scope(&mut *tx, tenant, root)
                .await
                .expect("node listing under RLS")
                .len(),
            2,
            "the seeded steward and the fresh viewer"
        );
        assert_eq!(
            role_bindings::all(&mut *tx, tenant)
                .await
                .expect("tenant listing under RLS")
                .len(),
            3,
            "both node bindings plus the tenant-wide admin"
        );
        assert!(
            role_bindings::unbind(&mut *tx, tenant, "rls-viewer", Some(root), Role::Viewer)
                .await
                .expect("unbind under RLS"),
            "unbind must work in-tenant"
        );
    });
}

// ── Identities & group mappings (AUTH-2, ADR-0013) ──────────────────────────

/// Admits a tenant with an org root, a provisioned identity under it, and
/// one group-mapping override. Runs on the (RLS-exempt) test connection.
async fn seed_identity(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let slug = format!("rlsi-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS identity fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let org = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        None,
        ScopeKind::Org,
        "acme",
        "ACME",
    )
    .await
    .expect("create org");
    let personal = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::User,
        "alice",
        "Alice",
    )
    .await
    .expect("create personal scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        "alice",
        IdentityKind::User,
        None,
        None,
        personal.id,
    )
    .await
    .expect("create identity");
    group_mappings::upsert(&mut *tx, tenant, "synveda-eng-core", org.id)
        .await
        .expect("create mapping");
    tx.commit().await.expect("commit identity fixture");
    tenant
}

/// Rows of `tenant` visible through the AUTH-2 tables, in the order
/// (identities, group_mappings).
async fn visible_identity_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let identities = sqlx::query_scalar!(
        r#"select count(*) as "count!" from identities where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count identities");
    let mappings = sqlx::query_scalar!(
        r#"select count(*) as "count!" from group_mappings where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count group_mappings");
    (identities, mappings)
}

/// The wrong (or absent) tenant GUC sees zero identity and mapping rows —
/// who works where is itself tenant-confidential; the right one sees its
/// own.
#[test]
fn wrong_tenant_guc_sees_no_identity_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_identity(&db.pool).await;
        let adversary = seed_identity(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_identity_rows(&mut tx, victim).await,
            (0, 0),
            "identity/mapping rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_identity_rows(&mut tx, adversary).await, (1, 1));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_identity_rows(&mut tx, victim).await,
            (0, 0),
            "identity/mapping rows visible without any tenant GUC"
        );
    });
}

/// Provisioning rows for another tenant than the GUC's trips the policies'
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_identity_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_identity(&db.pool).await;
        let other = seed_identity(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let foreign_root = hierarchy::root(&mut *tx, other)
            .await
            .expect("query foreign root");
        assert_eq!(foreign_root, None, "the foreign root must not even read");
        let result = identities::create(
            &mut tx,
            IdentityId::new(),
            other,
            "mallory",
            IdentityKind::User,
            None,
            None,
            ScopeId::new(),
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant identity insert must be rejected by RLS as an \
             internal defect, got {result:?}"
        );
    });
}

/// The provisioning shape — read subject, create identity, read mappings —
/// works as `synveda_app` with the right GUC, including the placement-
/// derived quarantine flag.
#[test]
fn same_tenant_identity_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_identity(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;

        let alice = identities::by_subject(&mut *tx, tenant, "alice")
            .await
            .expect("read under RLS")
            .expect("alice is provisioned");
        assert!(
            !alice.quarantined,
            "alice sits under the org, not quarantine"
        );

        let root = hierarchy::root(&mut *tx, tenant)
            .await
            .expect("read root")
            .expect("root exists");
        let quarantine = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(root.id),
            ScopeKind::Team,
            identities::QUARANTINE_SLUG,
            "Quarantine",
        )
        .await
        .expect("create quarantine under RLS");
        let personal = hierarchy::create(
            &mut tx,
            ScopeId::new(),
            tenant,
            Some(quarantine.id),
            ScopeKind::User,
            "bob",
            "Bob",
        )
        .await
        .expect("create personal scope under RLS");
        let bob = identities::create(
            &mut tx,
            IdentityId::new(),
            tenant,
            "bob",
            IdentityKind::User,
            Some("bob@example.test"),
            Some("Bob"),
            personal.id,
        )
        .await
        .expect("provision under RLS");
        assert!(bob.quarantined, "bob's placement derives quarantined=true");

        let mappings = group_mappings::for_groups(
            &mut *tx,
            tenant,
            &["synveda-eng-core".to_owned(), "unmapped".to_owned()],
        )
        .await
        .expect("read mappings under RLS");
        assert_eq!(mappings.len(), 1);
        assert!(
            group_mappings::remove(&mut *tx, tenant, "synveda-eng-core")
                .await
                .expect("remove mapping under RLS"),
            "remove must work in-tenant"
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

// ── Audit chain tables (AUD-1, ADR-0019) ────────────────────────────────────

/// Seeds one audit chain: the head row and two events, structurally-valid
/// rows via raw SQL on the (RLS-exempt) test connection — the store crate
/// sits beside `synveda-audit`, so this suite fabricates chain rows rather
/// than importing append. Chain *semantics* are the audit crate's tamper
/// suite; this suite covers isolation and grants only.
async fn seed_audit_chain(pool: &PgPool) -> TenantId {
    let tenant = TenantId::new();
    let hash = [0xabu8; 32];
    for seq in 1i64..=2 {
        sqlx::query!(
            r#"
            insert into audit_log
                (tenant_id, seq, occurred_at, actor_kind, actor_subject,
                 action, resource, outcome, payload, prev_hash, hash)
            values ($1, $2, now(), 'subject', 'rls-fixture',
                    'authz.decision', 'tenant fixture', 'allow', '{}', $3, $3)
            "#,
            tenant.as_uuid(),
            seq,
            &hash[..],
        )
        .execute(pool)
        .await
        .expect("insert audit event");
    }
    sqlx::query!(
        "insert into audit_chain_heads (tenant_id, seq, head_hash) values ($1, 2, $2)",
        tenant.as_uuid(),
        &hash[..],
    )
    .execute(pool)
    .await
    .expect("insert chain head");
    tenant
}

/// Rows of `tenant` visible through the audit tables, in the order
/// (audit_log, audit_chain_heads).
async fn visible_audit_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let events = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_log where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count audit_log");
    let heads = sqlx::query_scalar!(
        r#"select count(*) as "count!" from audit_chain_heads where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count audit_chain_heads");
    (events, heads)
}

/// The wrong (or absent) tenant GUC sees zero audit rows; the right one
/// sees exactly its own chain.
#[test]
fn wrong_tenant_guc_sees_no_audit_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_audit_chain(&db.pool).await;
        let adversary = seed_audit_chain(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_audit_rows(&mut tx, victim).await,
            (0, 0),
            "audit rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_audit_rows(&mut tx, adversary).await, (2, 1));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_audit_rows(&mut tx, victim).await,
            (0, 0),
            "audit rows visible without any tenant GUC"
        );
    });
}

/// The app role cannot rewrite history even inside its own tenant scope:
/// UPDATE and DELETE on `audit_log` were never granted (ADR-0019
/// decision 3), and DELETE on the chain head neither. 42501 whether the
/// grant or the append-only trigger answers first.
#[test]
fn audit_log_is_append_only_for_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_audit_chain(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let update = sqlx::raw_sql("update audit_log set resource = 'rewritten'")
            .execute(&mut *tx)
            .await;
        assert!(
            update.is_err(),
            "the app role must not hold UPDATE on audit_log"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let delete = sqlx::raw_sql("delete from audit_log")
            .execute(&mut *tx)
            .await;
        assert!(
            delete.is_err(),
            "the app role must not hold DELETE on audit_log"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let behead = sqlx::raw_sql("delete from audit_chain_heads")
            .execute(&mut *tx)
            .await;
        assert!(
            behead.is_err(),
            "the app role must not hold DELETE on audit_chain_heads"
        );
    });
}

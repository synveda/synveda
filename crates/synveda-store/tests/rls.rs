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
    group_mappings, hierarchy, identities, observe, policy_assignments, policy_packs, quarantine,
    rls, role_bindings, tenants,
};
use synveda_types::{
    Error, IdentityId, IdentityKind, ObserveKind, PackConfig, RecordClass, RecordId, RecordKind,
    Role, ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
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

/// A fixed embedding for every write: this suite exercises tenant
/// isolation, not vectors, but embed-or-fail (MEM-4, ADR-0023) makes an
/// embedding-less write unrepresentable.
fn embed() -> records::RecordEmbedding {
    records::RecordEmbedding {
        model: "test@1".to_owned(),
        vector: vec![0.25, -0.5, 0.75],
    }
}

/// [`records::insert`] with the fixed test embedding.
async fn insert(
    executor: impl sqlx::PgExecutor<'_>,
    id: RecordId,
    tenant: TenantId,
    state: &RecordState,
) -> synveda_types::Result<records::RecordVersion> {
    records::insert(executor, id, tenant, state, &embed()).await
}

/// [`records::update`] with the fixed test embedding.
async fn update(
    executor: impl sqlx::PgExecutor<'_>,
    id: RecordId,
    state: &RecordState,
) -> synveda_types::Result<Option<records::RecordVersion>> {
    records::update(executor, id, state, &embed()).await
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
    insert(pool, record, tenant, &state("v1"))
        .await
        .expect("insert record");
    tick().await;
    update(pool, record, &state("v2"))
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
    "graph_edges",
    "graph_edges_history",
    "graph_vertices",
    "group_mappings",
    "hierarchy_closure",
    "hierarchy_nodes",
    "identities",
    "memory_usage",
    "observe_events",
    "observe_quarantine",
    "policy_lapses",
    "policy_pack_assignments",
    "policy_pack_defaults",
    "policy_packs",
    "promotion_watermarks",
    "record_embeddings",
    "record_signatures",
    "record_supersessions",
    "records",
    "records_history",
    "role_bindings",
    "vedaflow_commit_parents",
    "vedaflow_commits",
    "vedaflow_objects",
    "vedaflow_proposal_approvals",
    "vedaflow_proposals",
    "vedaflow_refs",
    "vedaflow_tree_entries",
    "vedaflow_trees",
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

        // Both as-of surfaces: the corpus's (ADR-0006) and the graph's
        // (ADR-0043 decision 3, the same pair shape over `graph_edges`).
        for view in ["records_versions", "graph_edges_versions"] {
            let invoker = sqlx::query_scalar!(
                r#"
                select coalesce((
                    select lower(opt.option_value) in ('on', 'true', '1', 'yes')
                    from pg_options_to_table(c.reloptions) opt
                    where opt.option_name = 'security_invoker'
                ), false) as "security_invoker!"
                from pg_class c
                join pg_namespace n on n.oid = c.relnamespace
                where n.nspname = 'public' and c.relname = $1
                  and c.relkind = 'v'
                "#,
                view,
            )
            .fetch_one(&db.pool)
            .await
            .unwrap_or_else(|_| panic!("inspect {view}"));
            assert!(
                invoker,
                "{view} must be security_invoker, or as-of queries evaluate \
                 RLS as the view owner and bypass the backstop"
            );
        }
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
        &PackConfig::default(),
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
        let result =
            policy_packs::apply(&mut *tx, other, "forged", "permit;", &PackConfig::default()).await;
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
        let first = policy_packs::apply(
            &mut *tx,
            tenant,
            "rls-lifecycle",
            "forbid;",
            &PackConfig::default(),
        )
        .await
        .expect("apply under RLS");
        assert_eq!(first.version, 1, "a new name starts at v1");
        let bumped = policy_packs::apply(
            &mut *tx,
            tenant,
            "rls-lifecycle",
            "permit;",
            &PackConfig::default(),
        )
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
        let result = insert(&mut *tx, RecordId::new(), other, &state("forged")).await;
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
        let updated = update(&mut *tx, foreign_record, &state("hijack"))
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
        insert(&mut *tx, record, tenant, &state("v1"))
            .await
            .expect("insert under RLS");
        tx.commit().await.expect("commit insert");
        tick().await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        update(&mut *tx, record, &state("v2"))
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

// ── Record embeddings (MEM-4, ADR-0023) ─────────────────────────────────────

/// Embeddings are content-derived vectors and sit squarely under the
/// backstop: the wrong tenant GUC sees zero rows, a forged-tenant write
/// trips WITH CHECK, and the app role holds no DELETE — an embedding
/// leaves only through its record's FK cascade (which fires under the
/// app role because FK actions bypass RLS by Postgres semantics).
#[test]
fn record_embeddings_are_tenant_isolated_and_undeletable_by_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_record) = seed_tenant(&db.pool).await;
        let (adversary, _) = seed_tenant(&db.pool).await;

        // Cross-tenant reads see nothing — the raw table and the store
        // surface agree.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_embeddings
               where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count embeddings");
        assert_eq!(visible, 0, "embedding rows leaked across tenants");
        assert_eq!(
            records::embedding_meta(&mut *tx, victim_record)
                .await
                .expect("embedding meta across tenants"),
            None,
            "cross-tenant embedding metadata leaked"
        );
        drop(tx);

        // A forged-tenant write trips the policy's WITH CHECK.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            r#"insert into record_embeddings
                   (record_id, tenant_id, model, dim, embedding)
               values ($1, $2, 'test@1', 3, '[1,0,0]'::vector)"#,
            RecordId::new().as_uuid(),
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant embedding write must be rejected"
        );
        drop(tx);

        // No DELETE grant, even in-tenant: deleting an embedding while
        // its record lives would strand the record embedding-less.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let delete = sqlx::raw_sql("delete from record_embeddings")
            .execute(&mut *tx)
            .await;
        assert!(
            delete.is_err(),
            "the app role must not hold DELETE on record_embeddings"
        );
        drop(tx);

        // In-tenant the surface works, and the record's temporal delete
        // cascades the embedding away under the app role.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let meta = records::embedding_meta(&mut *tx, victim_record)
            .await
            .expect("read own embedding meta")
            .expect("the seeded record is embedded");
        assert_eq!(meta.model, "test@1");
        assert_eq!(meta.dim, 3);
        assert!(
            records::delete(&mut *tx, victim_record)
                .await
                .expect("delete record"),
            "delete must work in-tenant"
        );
        let orphaned = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_embeddings
               where record_id = $1"#,
            victim_record.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count after cascade");
        assert_eq!(
            orphaned, 0,
            "the record's cascade must remove its embedding"
        );
        tx.commit().await.expect("commit");
    });
}

// ── Dedup & supersession (MEM-5, ADR-0039) ──────────────────────────────────

/// The signature sidecar is content-derived and sits under the backstop
/// exactly as the embedding does: the wrong tenant GUC nominates nothing,
/// a forged-tenant write trips WITH CHECK, and no DELETE grant exists — a
/// signature leaves through its record's cascade.
///
/// The nomination *leak* is the interesting one. LSH buckets are a
/// similarity oracle: if a band query crossed tenants, a competitor could
/// learn that somebody else holds a document like theirs without ever
/// reading a row. The band the two tenants share here is identical by
/// construction, so a leak would be certain rather than probable.
#[test]
fn record_signatures_are_tenant_isolated_and_nominate_nothing_across_tenants() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_record) = seed_tenant(&db.pool).await;
        let (adversary, _) = seed_tenant(&db.pool).await;
        // Both fixtures store the same content, so their bands are equal.
        let bands = synveda_store::dedup::signature("v2").bands;
        // The victim's *own* group, so nothing but the backstop is left to
        // stop the nomination — a query filtered to a scope the adversary
        // guessed wrong would prove nothing.
        let placement = sqlx::query!(
            "select scope_id, owner_id from records where id = $1",
            victim_record.as_uuid(),
        )
        .fetch_one(&db.pool)
        .await
        .expect("read the victim's placement");

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_signatures where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count signatures");
        assert_eq!(visible, 0, "signature rows leaked across tenants");

        let nominated = synveda_store::dedup::nominate_lexical(
            &mut tx,
            &synveda_store::dedup::CandidateGroup {
                tenant_id: victim,
                scope_id: ScopeId::from_uuid(placement.scope_id),
                owner_id: IdentityId::from_uuid(placement.owner_id),
                class: RecordClass::Fact,
                at: chrono::Utc::now() - chrono::Duration::days(365),
            },
            &bands,
            16,
        )
        .await
        .expect("nominate across tenants");
        assert!(
            nominated.is_empty(),
            "the LSH nominator is a similarity oracle; it must not answer \
             for another tenant's corpus"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            r#"insert into record_signatures (record_id, tenant_id, signature, bands)
               values ($1, $2, array[1::bigint], array[1::bigint])"#,
            RecordId::new().as_uuid(),
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant signature write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let delete = sqlx::raw_sql("delete from record_signatures")
            .execute(&mut *tx)
            .await;
        assert!(
            delete.is_err(),
            "the app role must not hold DELETE on record_signatures"
        );
        drop(tx);

        // In-tenant it works, and the record's cascade takes it away.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let own = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_signatures where record_id = $1"#,
            victim_record.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count own signature");
        assert_eq!(own, 1, "every record written through the API is signed");
        records::delete(&mut *tx, victim_record)
            .await
            .expect("delete record");
        let orphaned = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_signatures where record_id = $1"#,
            victim_record.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count after cascade");
        assert_eq!(
            orphaned, 0,
            "the record's cascade must remove its signature"
        );
        tx.commit().await.expect("commit");
    });
}

/// Supersession edges say which of a tenant's facts replaced which — a
/// change log of what an organisation believed and when. Isolated like
/// every other tenant-scoped table, and append-only by grant: an edge is a
/// record of a decision that was taken, and a decision that can be deleted
/// is one an auditor cannot rely on.
#[test]
fn record_supersessions_are_tenant_isolated_and_append_only() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_record) = seed_tenant(&db.pool).await;
        let (adversary, adversary_record) = seed_tenant(&db.pool).await;

        let edge = |superseded, superseding| synveda_store::dedup::Supersession {
            superseded_id: superseded,
            superseding_id: superseding,
            method: "deterministic".to_owned(),
            reason: "contradiction".to_owned(),
            jaccard_permille: Some(600),
            cosine_permille: None,
            closed_at: chrono::Utc::now(),
        };

        // A second record to point at, in the victim's own tenant.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let successor = RecordId::new();
        insert(&mut *tx, successor, victim, &state("v3"))
            .await
            .expect("insert successor");
        synveda_store::dedup::record_supersession(&mut tx, victim, &edge(victim_record, successor))
            .await
            .expect("record the edge in-tenant");
        tx.commit().await.expect("commit edge");

        // The adversary sees nothing, through the raw table or the surface.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_supersessions where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count edges");
        assert_eq!(visible, 0, "supersession edges leaked across tenants");
        assert!(
            synveda_store::dedup::supersessions_for(&mut tx, victim, victim_record)
                .await
                .expect("read edges across tenants")
                .is_empty(),
            "and the read surface agrees"
        );

        // A forged-tenant edge trips WITH CHECK.
        let forged = synveda_store::dedup::record_supersession(
            &mut tx,
            victim,
            &edge(adversary_record, adversary_record),
        )
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant edge write must be rejected"
        );
        drop(tx);

        // Append-only: no DELETE, no UPDATE.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert!(
            sqlx::raw_sql("delete from record_supersessions")
                .execute(&mut *tx)
                .await
                .is_err(),
            "the app role must not hold DELETE on record_supersessions"
        );
        drop(tx);
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert!(
            sqlx::raw_sql("update record_supersessions set reason = 'near-duplicate'")
                .execute(&mut *tx)
                .await
                .is_err(),
            "nor UPDATE: an edge records a decision that was taken"
        );
        drop(tx);

        // In-tenant the surface reads its own, and the record's cascade
        // takes the edge with it.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let edges = synveda_store::dedup::supersessions_for(&mut tx, victim, victim_record)
            .await
            .expect("read own edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].superseding_id, successor);
        assert_eq!(edges[0].jaccard_permille, Some(600));
        records::delete(&mut *tx, victim_record)
            .await
            .expect("delete record");
        let orphaned = sqlx::query_scalar!(
            r#"select count(*) as "count!" from record_supersessions
               where superseded_id = $1"#,
            victim_record.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count after cascade");
        assert_eq!(orphaned, 0, "the record's cascade must remove its edges");
        tx.commit().await.expect("commit");
    });
}

// ── Observe buffer (MEM-1, ADR-0020) ────────────────────────────────────────

fn observe_event(key: &str) -> observe::NewObserveEvent {
    observe::NewObserveEvent {
        idempotency_key: key.to_owned(),
        kind: ObserveKind::TranscriptDelta,
        payload: serde_json::json!({"text": "rls fixture"}),
        occurred_at: chrono::Utc::now(),
        redactions: None,
        quarantine: false,
    }
}

/// An event staged behind the review queue (MEM-2, ADR-0021 decision 5).
fn quarantined_event(key: &str) -> observe::NewObserveEvent {
    observe::NewObserveEvent {
        idempotency_key: key.to_owned(),
        kind: ObserveKind::TranscriptDelta,
        payload: serde_json::json!({"text": "[REDACTED:aws-access-key-id] fixture"}),
        occurred_at: chrono::Utc::now(),
        redactions: Some(serde_json::json!([
            {"rule": "aws-access-key-id", "category": "secret", "count": 1}
        ])),
        quarantine: true,
    }
}

/// Admits a tenant with an org root, a personal scope, an identity, and
/// two buffered observe events. Runs on the (RLS-exempt) test connection.
async fn seed_observe(pool: &PgPool) -> (TenantId, ScopeId, IdentityId) {
    let tenant = TenantId::new();
    let slug = format!("rlso-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS observe fixture",
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
    let identity = identities::create(
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
    observe::buffer_batch(
        &mut tx,
        tenant,
        personal.id,
        identity.id,
        "rls-session",
        &[observe_event("rls-1"), observe_event("rls-2")],
    )
    .await
    .expect("buffer events");
    tx.commit().await.expect("commit observe fixture");
    (tenant, personal.id, identity.id)
}

async fn visible_observe_rows(tx: &mut Transaction<'static, Postgres>, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count observe_events")
}

/// The wrong (or absent) tenant GUC sees zero staged events — raw
/// pre-redaction session content is exactly what the backstop exists to
/// protect (ADR-0020 decision 1); the right one sees its own.
#[test]
fn wrong_tenant_guc_sees_no_observe_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, _) = seed_observe(&db.pool).await;
        let (adversary, _, _) = seed_observe(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_observe_rows(&mut tx, victim).await,
            0,
            "observe rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_observe_rows(&mut tx, adversary).await, 2);
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_observe_rows(&mut tx, victim).await,
            0,
            "observe rows visible without any tenant GUC"
        );
    });
}

/// Buffering events for another tenant than the GUC's trips the policy's
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_observe_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, _) = seed_observe(&db.pool).await;
        let (other, other_scope, other_identity) = seed_observe(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = observe::buffer_batch(
            &mut tx,
            other,
            other_scope,
            other_identity,
            "forged-session",
            &[observe_event("forged")],
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant observe write must be rejected by RLS as an \
             internal defect, got {result:?}"
        );
    });
}

/// The app role cannot rewrite what was observed even inside its own tenant
/// scope: UPDATE on `observe_events` was never granted — staging rows are
/// provenance (ADR-0020 decision 1).
///
/// DELETE *is* granted since migration 0025, and deliberately: disposal is
/// the obligation ADR-0020 parked on MEM-6 and migration 0012 said would
/// "bring its own grants" (ADR-0040 decision 7). What bounds it is the
/// horizon the sweep reads from the pack, not the absence of a grant — so
/// what this test still holds is the immutability of a staged payload,
/// which is the property the provenance doctrine was ever about.
#[test]
fn observe_events_are_immutable_and_only_retention_removes_them() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, _) = seed_observe(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let update = sqlx::raw_sql("update observe_events set payload = '{}'")
            .execute(&mut *tx)
            .await;
        assert!(
            update.is_err(),
            "the app role must not hold UPDATE on observe_events"
        );
        drop(tx);

        // A payload cannot be edited into something else...
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let rewritten = sqlx::raw_sql("update observe_events set occurred_at = now()")
            .execute(&mut *tx)
            .await;
        assert!(
            rewritten.is_err(),
            "nor may its stamps move: a staging row is what was observed"
        );
        drop(tx);

        // ...and disposal removes it whole, which is a different act.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let disposed = sqlx::raw_sql("delete from observe_events")
            .execute(&mut *tx)
            .await
            .expect("disposal is granted since MEM-6")
            .rows_affected();
        assert!(disposed > 0, "and it takes whole rows, never part of one");
        let left = sqlx::query_scalar!(
            r#"select count(*) as "count!" from observe_events where tenant_id = $1"#,
            tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count staging");
        assert_eq!(left, 0, "the tenant's own staging plane, and only its own");
    });
}

/// The full admission shape — insert, duplicate suppression, enqueue —
/// works as `synveda_app` with the right GUC, PGMQ grants included: the
/// backstop isolates, it does not deny service.
#[test]
fn same_tenant_observe_admission_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, scope, identity) = seed_observe(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let admitted = observe::buffer_batch(
            &mut tx,
            tenant,
            scope,
            identity,
            "rls-session",
            // rls-1 was admitted by the seed; rls-3 is new.
            &[observe_event("rls-1"), observe_event("rls-3")],
        )
        .await
        .expect("buffer under RLS (pgmq grants included)");
        assert_eq!(
            admitted
                .iter()
                .map(|event| event.duplicate)
                .collect::<Vec<_>>(),
            vec![true, false],
            "the redelivered key must be reported, the fresh one admitted"
        );
        tx.commit().await.expect("commit admission");

        // Exactly one queue signal per admitted event: the seed's two plus
        // rls-3 — the duplicate enqueued nothing (messages carry only ids,
        // and this count is per-tenant via the message body).
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let signals = sqlx::query_scalar!(
            r#"select count(*) as "count!" from pgmq.q_observe
               where message ->> 'tenant_id' = $1"#,
            tenant.to_string(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count queue signals as synveda_app");
        assert_eq!(
            signals, 3,
            "one signal per admitted event, none per duplicate"
        );
    });
}

// ── Quarantine review queue (MEM-2, ADR-0021) ───────────────────────────────

/// Admits a tenant with one quarantined observe event; returns its ids.
async fn seed_quarantined(pool: &PgPool) -> (TenantId, synveda_types::ObserveEventId) {
    let (tenant, scope, identity) = seed_observe(pool).await;
    let mut tx = pool.begin().await.expect("begin");
    let admitted = observe::buffer_batch(
        &mut tx,
        tenant,
        scope,
        identity,
        "rls-quarantine-session",
        &[quarantined_event("rls-q1")],
    )
    .await
    .expect("buffer quarantined event");
    tx.commit().await.expect("commit quarantine fixture");
    (tenant, admitted[0].id)
}

/// The wrong (or absent) tenant GUC sees zero quarantine rows, and a
/// cross-tenant review resolves nothing — the review queue is content
/// (redacted, but content) and sits squarely under the backstop.
#[test]
fn wrong_tenant_guc_sees_no_quarantine_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, event_id) = seed_quarantined(&db.pool).await;
        let (adversary, _) = seed_quarantined(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from observe_quarantine
               where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count quarantine rows");
        assert_eq!(visible, 0, "quarantine rows leaked across tenants");
        // The store surfaces reach nothing either: get is None, review
        // touches no row — the gateway's uniform 404.
        assert_eq!(
            quarantine::get(&mut tx, victim, event_id)
                .await
                .expect("get across tenants"),
            None
        );
        let review = quarantine::review(
            &mut tx,
            victim,
            event_id,
            quarantine::ReviewDecision::Release,
            "adversary",
            None,
        )
        .await;
        assert!(
            matches!(review, Ok(None)),
            "a cross-tenant review must resolve nothing, got {review:?}"
        );
    });
}

/// The app role's write power over the review queue is exactly the
/// one-shot review: findings/provenance columns are not updatable, rows
/// are not deletable, and a reviewed row cannot be re-reviewed — column
/// grants and the transition trigger, exercised as `synveda_app`.
#[test]
fn quarantine_review_is_one_shot_and_column_bound_for_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, event_id) = seed_quarantined(&db.pool).await;

        // Rewriting findings: no column grant.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let rewrite = sqlx::raw_sql("update observe_quarantine set findings = '[]'")
            .execute(&mut *tx)
            .await;
        assert!(
            rewrite.is_err(),
            "the app role must not hold UPDATE on findings"
        );
        drop(tx);

        // Deleting: no grant, and the trigger raises even for owners.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let delete = sqlx::raw_sql("delete from observe_quarantine")
            .execute(&mut *tx)
            .await;
        assert!(
            delete.is_err(),
            "the app role must not hold DELETE on observe_quarantine"
        );
        drop(tx);

        // The sanctioned path works: pending → rejected, one-shot.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let reviewed = quarantine::review(
            &mut tx,
            tenant,
            event_id,
            quarantine::ReviewDecision::Reject,
            "rls-reviewer",
            Some("rls suite"),
        )
        .await
        .expect("review under RLS")
        .expect("the quarantined event exists");
        assert_eq!(reviewed.state, synveda_types::QuarantineState::Rejected);
        tx.commit().await.expect("commit review");

        // A second verdict is a conflict, not a rewrite.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let second = quarantine::review(
            &mut tx,
            tenant,
            event_id,
            quarantine::ReviewDecision::Release,
            "rls-reviewer",
            None,
        )
        .await;
        assert!(
            matches!(second, Err(Error::Conflict { .. })),
            "review must be one-shot, got {second:?}"
        );
        drop(tx);

        // Even a raw update aimed back at pending trips the transition
        // trigger — the state machine is schema-enforced.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let unreview = sqlx::query!(
            "update observe_quarantine set state = 'pending', \
             reviewer_subject = null, reviewed_at = null, review_reason = null \
             where event_id = $1",
            event_id.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            unreview.is_err(),
            "a reviewed row must never return to pending"
        );
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

// ── VedaFlow object store (FLOW-1, ADR-0030) ────────────────────────────────

/// Seeds one tenant's worth of VedaFlow history: an object, a tree holding
/// an entry that points at it, two commits linked parent-to-child, and a ref.
///
/// Raw SQL with arbitrary 32-byte addresses on purpose. `synveda-store` sits
/// beside `synveda-vedaflow` and cannot import it (seed §8), and this suite
/// is about the backstop rather than about content addressing — FLOW-1's own
/// property tests own that half, and RLS does not care whether a hash is
/// honest.
async fn seed_vedaflow(pool: &PgPool) -> (TenantId, ScopeId) {
    let tenant = TenantId::new();
    let slug = format!("rls-vf-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "VedaFlow RLS fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let scope = ScopeId::new();
    let author = IdentityId::new();

    sqlx::query!(
        "insert into vedaflow_objects (tenant_id, hash, kind, content, size_bytes)
         values ($1, $2, 'memory', $3, 3)",
        tenant.as_uuid(),
        &[1u8; 32][..],
        &b"abc"[..],
    )
    .execute(pool)
    .await
    .expect("seed object");
    sqlx::query!(
        "insert into vedaflow_trees (tenant_id, hash) values ($1, $2)",
        tenant.as_uuid(),
        &[2u8; 32][..],
    )
    .execute(pool)
    .await
    .expect("seed tree");
    sqlx::query!(
        "insert into vedaflow_tree_entries (tenant_id, tree_hash, name, object_hash)
         values ($1, $2, 'note.md', $3)",
        tenant.as_uuid(),
        &[2u8; 32][..],
        &[1u8; 32][..],
    )
    .execute(pool)
    .await
    .expect("seed tree entry");
    for hash in [[3u8; 32], [4u8; 32]] {
        sqlx::query!(
            "insert into vedaflow_commits
                 (tenant_id, hash, tree_hash, author_id, message, committed_at,
                  policy_snapshot_hash)
             values ($1, $2, $3, $4, 'seed', now(), $5)",
            tenant.as_uuid(),
            &hash[..],
            &[2u8; 32][..],
            author.as_uuid(),
            &[5u8; 32][..],
        )
        .execute(pool)
        .await
        .expect("seed commit");
    }
    sqlx::query!(
        "insert into vedaflow_commit_parents (tenant_id, commit_hash, ordinal, parent_hash)
         values ($1, $2, 0, $3)",
        tenant.as_uuid(),
        &[4u8; 32][..],
        &[3u8; 32][..],
    )
    .execute(pool)
    .await
    .expect("seed commit parent");
    sqlx::query!(
        "insert into vedaflow_refs (tenant_id, scope_id, name, commit_hash, updated_by)
         values ($1, $2, 'published', $3, $4)",
        tenant.as_uuid(),
        scope.as_uuid(),
        &[4u8; 32][..],
        author.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed ref");
    (tenant, scope)
}

/// Rows of `tenant` visible through the six VedaFlow tables, in the order
/// (objects, trees, entries, commits, parents, refs).
async fn visible_vedaflow_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64, i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from vedaflow_objects where tenant_id = $1) as "objects!",
             (select count(*) from vedaflow_trees where tenant_id = $1) as "trees!",
             (select count(*) from vedaflow_tree_entries where tenant_id = $1) as "entries!",
             (select count(*) from vedaflow_commits where tenant_id = $1) as "commits!",
             (select count(*) from vedaflow_commit_parents where tenant_id = $1) as "parents!",
             (select count(*) from vedaflow_refs where tenant_id = $1) as "refs!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count vedaflow rows");
    (
        row.objects,
        row.trees,
        row.entries,
        row.commits,
        row.parents,
        row.refs,
    )
}

#[test]
fn wrong_tenant_guc_sees_no_vedaflow_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_vedaflow(&db.pool).await;
        let (adversary, _) = seed_vedaflow(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert_eq!(
            visible_vedaflow_rows(&mut tx, victim).await,
            (1, 1, 1, 2, 1, 1),
            "a tenant must see its own governed history"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_vedaflow_rows(&mut tx, victim).await,
            (0, 0, 0, 0, 0, 0),
            "another tenant's knowledge history leaked"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_vedaflow_rows(&mut tx, victim).await,
            (0, 0, 0, 0, 0, 0),
            "an unset GUC must see nothing at all"
        );
    });
}

/// Forging another tenant's id on the way in trips each policy's WITH CHECK,
/// on every one of the six tables.
#[test]
fn cross_tenant_vedaflow_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_scope) = seed_vedaflow(&db.pool).await;
        let (adversary, _) = seed_vedaflow(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_objects (tenant_id, hash, kind, content, size_bytes)
             values ($1, $2, 'memory', $3, 3)",
            victim.as_uuid(),
            &[9u8; 32][..],
            &b"xyz"[..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant object write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_trees (tenant_id, hash) values ($1, $2)",
            victim.as_uuid(),
            &[9u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant tree write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_tree_entries (tenant_id, tree_hash, name, object_hash)
             values ($1, $2, 'stolen.md', $3)",
            victim.as_uuid(),
            &[2u8; 32][..],
            &[1u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant tree entry write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_commits
                 (tenant_id, hash, tree_hash, author_id, message, committed_at,
                  policy_snapshot_hash)
             values ($1, $2, $3, $4, 'forged', now(), $5)",
            victim.as_uuid(),
            &[9u8; 32][..],
            &[2u8; 32][..],
            IdentityId::new().as_uuid(),
            &[5u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant commit write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_commit_parents
                 (tenant_id, commit_hash, ordinal, parent_hash)
             values ($1, $2, 1, $3)",
            victim.as_uuid(),
            &[4u8; 32][..],
            &[3u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant commit parent write must be rejected"
        );
        drop(tx);

        // And the one mutable table, both ways in: a forged insert, and a
        // cross-tenant move of a ref the adversary cannot even see.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_refs (tenant_id, scope_id, name, commit_hash, updated_by)
             values ($1, $2, 'hijacked', $3, $4)",
            victim.as_uuid(),
            victim_scope.as_uuid(),
            &[4u8; 32][..],
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant ref write must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let moved = sqlx::query!(
            "update vedaflow_refs set commit_hash = $3
             where tenant_id = $1 and scope_id = $2",
            victim.as_uuid(),
            victim_scope.as_uuid(),
            &[3u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant ref move");
        assert_eq!(
            moved.rows_affected(),
            0,
            "another tenant's published ref must be unreachable, not merely unwritten"
        );
    });
}

/// The five history tables hold no UPDATE or DELETE grant for the app role
/// (ADR-0030 decision 6). 42501 whether the withheld grant or the
/// append-only trigger answers first.
#[test]
fn vedaflow_history_is_append_only_for_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_vedaflow(&db.pool).await;

        for statement in [
            "update vedaflow_objects set content = 'tampered'",
            "delete from vedaflow_objects",
            "update vedaflow_trees set created_at = now()",
            "delete from vedaflow_trees",
            "update vedaflow_tree_entries set name = 'renamed'",
            "delete from vedaflow_tree_entries",
            "update vedaflow_commits set message = 'rewritten'",
            "delete from vedaflow_commits",
            "update vedaflow_commit_parents set ordinal = 9",
            "delete from vedaflow_commit_parents",
        ] {
            let mut tx = app_tx(&db.pool, Some(tenant)).await;
            let outcome = sqlx::raw_sql(statement).execute(&mut *tx).await;
            assert!(
                outcome.is_err(),
                "the app role must not be able to run: {statement}"
            );
        }
    });
}

/// FLOW-7's one deletion (migration 0021, ADR-0036 decision 8): a pin can
/// be released, and a channel pointer still never disappears.
///
/// Both halves of the narrowing are asserted, because they answer to
/// different attackers. The **restrictive policy** is what the product
/// runs under: the app role's delete of a channel ref is a legal statement
/// that matches nothing, even with the right tenant GUC set and even
/// without a `where` clause. The **trigger** is what someone bypassing RLS
/// meets: the superuser pool — no GUC, no policies — gets an exception
/// naming the rule instead of a quiet success. That is migration 0018's
/// own split, extended to the first ref that is a decision rather than a
/// pointer into history.
#[test]
fn only_pins_can_be_deleted_and_only_in_their_own_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, scope) = seed_vedaflow(&db.pool).await;
        let (other_tenant, other_scope) = seed_vedaflow(&db.pool).await;
        let pinner = IdentityId::new();

        // A pin at each tenant, on the seeded channel's own commit.
        for (tenant, scope) in [(tenant, scope), (other_tenant, other_scope)] {
            sqlx::query!(
                "insert into vedaflow_refs (tenant_id, scope_id, name, commit_hash, updated_by)
                 values ($1, $2, 'pin/memory/published', $3, $4)",
                tenant.as_uuid(),
                scope.as_uuid(),
                &[4u8; 32][..],
                pinner.as_uuid(),
            )
            .execute(&db.pool)
            .await
            .expect("seed pin");
        }

        let mut tx = app_tx(&db.pool, Some(tenant)).await;

        // A channel pointer: legal statement, zero rows. Deliberately
        // unqualified — if the policy were permissive this would take
        // every channel in the tenant with it.
        let channels = sqlx::query!("delete from vedaflow_refs where name not like 'pin/%'")
            .execute(&mut *tx)
            .await
            .expect("deleting a channel ref is a legal statement")
            .rows_affected();
        assert_eq!(
            channels, 0,
            "a channel pointer must be unreachable to DELETE, not merely unwritten"
        );

        // Another tenant's pin: the ordinary isolation, still in force on
        // the one path that can now remove a row.
        let forged = sqlx::query!(
            "delete from vedaflow_refs where tenant_id = $1 and name = 'pin/memory/published'",
            other_tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant unpin runs")
        .rows_affected();
        assert_eq!(forged, 0, "another tenant's pin must be unreachable");

        // Its own pin: released.
        let released = sqlx::query!(
            "delete from vedaflow_refs where scope_id = $1 and name = 'pin/memory/published'",
            scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("unpin own channel")
        .rows_affected();
        assert_eq!(released, 1, "a pin must be releasable in its own tenant");
        tx.commit().await.expect("commit");

        // The other tenant's pin is untouched, and its channel is too.
        let survivors = sqlx::query_scalar!(
            "select count(*) as \"count!\" from vedaflow_refs where tenant_id = $1",
            other_tenant.as_uuid(),
        )
        .fetch_one(&db.pool)
        .await
        .expect("count surviving refs");
        assert_eq!(
            survivors, 2,
            "the other tenant keeps its channel and its pin"
        );

        // And the trigger, for whoever is not running under RLS: the
        // superuser pool deleting a channel pointer must raise rather than
        // silently succeed.
        let raised = sqlx::query!(
            "delete from vedaflow_refs where tenant_id = $1 and name = 'published'",
            tenant.as_uuid(),
        )
        .execute(&db.pool)
        .await
        .expect_err("the delete guard must raise for a channel pointer");
        assert!(
            raised.to_string().contains("channel pointer"),
            "unexpected error: {raised}"
        );
    });
}

/// In-tenant, the app role can do everything the object store needs: write
/// content, link it, commit it, and move a ref forward.
#[test]
fn same_tenant_vedaflow_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, scope) = seed_vedaflow(&db.pool).await;
        let author = IdentityId::new();

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            "insert into vedaflow_objects (tenant_id, hash, kind, content, size_bytes)
             values ($1, $2, 'prompt', $3, 4)",
            tenant.as_uuid(),
            &[10u8; 32][..],
            &b"tips"[..],
        )
        .execute(&mut *tx)
        .await
        .expect("write own object");
        sqlx::query!(
            "insert into vedaflow_trees (tenant_id, hash) values ($1, $2)",
            tenant.as_uuid(),
            &[11u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("write own tree");
        sqlx::query!(
            "insert into vedaflow_tree_entries (tenant_id, tree_hash, name, object_hash)
             values ($1, $2, 'style.md', $3)",
            tenant.as_uuid(),
            &[11u8; 32][..],
            &[10u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("write own tree entry");
        sqlx::query!(
            "insert into vedaflow_commits
                 (tenant_id, hash, tree_hash, author_id, message, committed_at,
                  policy_snapshot_hash)
             values ($1, $2, $3, $4, 'own commit', now(), $5)",
            tenant.as_uuid(),
            &[12u8; 32][..],
            &[11u8; 32][..],
            author.as_uuid(),
            &[5u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("write own commit");
        sqlx::query!(
            "insert into vedaflow_commit_parents
                 (tenant_id, commit_hash, ordinal, parent_hash)
             values ($1, $2, 0, $3)",
            tenant.as_uuid(),
            &[12u8; 32][..],
            &[4u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("write own commit parent");

        // The ref moves — the one UPDATE the app role holds on this schema.
        let moved = sqlx::query!(
            "update vedaflow_refs set commit_hash = $4, updated_at = now(), updated_by = $5
             where tenant_id = $1 and scope_id = $2 and name = 'published' and commit_hash = $3",
            tenant.as_uuid(),
            scope.as_uuid(),
            &[4u8; 32][..],
            &[12u8; 32][..],
            author.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("move own ref");
        assert_eq!(moved.rows_affected(), 1, "a ref must move in-tenant");

        assert_eq!(
            visible_vedaflow_rows(&mut tx, tenant).await,
            (2, 2, 2, 3, 2, 1),
            "everything written in-tenant is visible in-tenant"
        );
        tx.commit().await.expect("commit");
    });
}

// ── VedaFlow proposals (FLOW-3, ADR-0032) ───────────────────────────────────

/// Seeds one tenant's proposal: the commit it names (the FK insists on real
/// history), the row, and one recorded approval.
///
/// Raw SQL again, and for the same reason as [`seed_vedaflow`]: this suite is
/// about the backstop, not about whether an address is honest.
async fn seed_proposal(pool: &PgPool) -> (TenantId, ScopeId, uuid::Uuid) {
    let (tenant, scope) = seed_vedaflow(pool).await;
    let proposal = uuid::Uuid::now_v7();
    let approver = IdentityId::new();
    sqlx::query!(
        "insert into vedaflow_proposals
             (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
              target_channel, commit_hash, sensitivity, title, proposer_id,
              proposer_subject)
         values ($1, $2, $3, $3, 'memory', 'published', $4, 'internal',
                 'rls fixture proposal', $5, 'rls-fixture')",
        tenant.as_uuid(),
        proposal,
        scope.as_uuid(),
        &[4u8; 32][..],
        approver.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed proposal");
    sqlx::query!(
        "insert into vedaflow_proposal_approvals
             (tenant_id, proposal_id, approver_id, commit_hash, verdict, roles,
              approver_subject)
         values ($1, $2, $3, $4, 'approve', array['curator']::text[], 'rls-fixture')",
        tenant.as_uuid(),
        proposal,
        approver.as_uuid(),
        &[4u8; 32][..],
    )
    .execute(pool)
    .await
    .expect("seed approval");
    (tenant, scope, proposal)
}

/// Rows of `tenant` visible through the proposal tables, in the order
/// (proposals, approvals).
async fn visible_proposal_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from vedaflow_proposals where tenant_id = $1)
                 as "proposals!",
             (select count(*) from vedaflow_proposal_approvals where tenant_id = $1)
                 as "approvals!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count proposal rows");
    (row.proposals, row.approvals)
}

#[test]
fn wrong_tenant_guc_sees_no_proposal_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, _) = seed_proposal(&db.pool).await;
        let (adversary, _, _) = seed_proposal(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert_eq!(
            visible_proposal_rows(&mut tx, victim).await,
            (1, 1),
            "a tenant must see its own proposals and their review log"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_proposal_rows(&mut tx, victim).await,
            (0, 0),
            "another tenant's proposals leaked — including who approved what"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_proposal_rows(&mut tx, victim).await,
            (0, 0),
            "an unset GUC must see nothing at all"
        );
    });
}

/// Forging another tenant's id trips each policy's WITH CHECK, and a
/// cross-tenant close affects zero rows — a proposal at another tenant is
/// unreachable, not merely unwritten.
#[test]
fn cross_tenant_proposal_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_scope, victim_proposal) = seed_proposal(&db.pool).await;
        let (adversary, _, _) = seed_proposal(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_proposals
                 (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
                  target_channel, commit_hash, sensitivity, title, proposer_id,
                  proposer_subject)
             values ($1, $2, $3, $3, 'memory', 'published', $4, 'internal',
                     'forged', $5, 'intruder')",
            victim.as_uuid(),
            uuid::Uuid::now_v7(),
            victim_scope.as_uuid(),
            &[4u8; 32][..],
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant proposal write must be rejected"
        );
        drop(tx);

        // An approval forged onto someone else's proposal is the attack
        // that would fabricate a review: the FK cannot see the victim's
        // proposal from here, and the policy's WITH CHECK refuses the row.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into vedaflow_proposal_approvals
                 (tenant_id, proposal_id, approver_id, commit_hash, verdict, roles,
                  approver_subject)
             values ($1, $2, $3, $4, 'approve', array['compliance']::text[], 'intruder')",
            victim.as_uuid(),
            victim_proposal,
            IdentityId::new().as_uuid(),
            &[4u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged approval on another tenant's proposal must be rejected"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let closed = sqlx::query!(
            "update vedaflow_proposals
             set state = 'published', closed_at = now(), closed_by = $3
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            victim_proposal,
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant proposal close");
        assert_eq!(
            closed.rows_affected(),
            0,
            "another tenant's proposal must be unreachable, not merely unwritten"
        );
    });
}

/// The review log is history: no UPDATE, no DELETE, for anyone. The
/// proposal row holds no DELETE either, and its one permitted change is
/// open → closed — a second close raises, and so does editing anything
/// else about it (ADR-0032 decision 1).
#[test]
fn the_proposal_review_log_is_append_only_and_the_row_closes_once() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, proposal) = seed_proposal(&db.pool).await;

        for statement in [
            "update vedaflow_proposal_approvals set verdict = 'approve'",
            "delete from vedaflow_proposal_approvals",
            "delete from vedaflow_proposals",
        ] {
            let mut tx = app_tx(&db.pool, Some(tenant)).await;
            let outcome = sqlx::raw_sql(statement).execute(&mut *tx).await;
            assert!(
                outcome.is_err(),
                "the app role must not be able to run: {statement}"
            );
        }

        // Retitling an open proposal — the attack that would change what a
        // recorded approval was about — is refused by the transition
        // trigger, table owner included.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let retitled = sqlx::raw_sql("update vedaflow_proposals set title = 'something else'")
            .execute(&mut *tx)
            .await;
        assert!(
            retitled.is_err(),
            "a proposal is immutable except for its closure"
        );
        drop(tx);

        // The one permitted transition works once...
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let closed = sqlx::query!(
            "update vedaflow_proposals
             set state = 'withdrawn', closed_at = now(), closed_by = $3
             where tenant_id = $1 and id = $2 and state = 'open'",
            tenant.as_uuid(),
            proposal,
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("close an open proposal");
        assert_eq!(closed.rows_affected(), 1);
        tx.commit().await.expect("commit the close");

        // ...and never again, whatever the state column is set to.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let reopened = sqlx::query!(
            "update vedaflow_proposals set state = 'published'
             where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            proposal,
        )
        .execute(&mut *tx)
        .await;
        assert!(
            reopened.is_err(),
            "a closed proposal must never be reopened or re-closed"
        );
    });
}

/// In-tenant, the app role can do everything the review flow needs: open a
/// proposal, record verdicts against it, and close it exactly once.
#[test]
fn same_tenant_proposal_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, scope, _) = seed_proposal(&db.pool).await;
        let proposal = uuid::Uuid::now_v7();
        let proposer = IdentityId::new();

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            "insert into vedaflow_proposals
                 (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
                  target_channel, commit_hash, sensitivity, title, proposer_id,
                  proposer_subject)
             values ($1, $2, $3, $3, 'memory', 'published', $4, 'restricted',
                     'own proposal', $5, 'own-subject')",
            tenant.as_uuid(),
            proposal,
            scope.as_uuid(),
            &[3u8; 32][..],
            proposer.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("open own proposal");

        // Two distinct approvers, which is what `restricted` takes.
        for (approver, role) in [
            (IdentityId::new(), "curator"),
            (IdentityId::new(), "compliance"),
        ] {
            sqlx::query!(
                "insert into vedaflow_proposal_approvals
                     (tenant_id, proposal_id, approver_id, commit_hash, verdict,
                      roles, approver_subject)
                 values ($1, $2, $3, $4, 'approve', array[$5]::text[], $6)",
                tenant.as_uuid(),
                proposal,
                approver.as_uuid(),
                &[3u8; 32][..],
                role,
                format!("{role}-subject"),
            )
            .execute(&mut *tx)
            .await
            .expect("record own approval");
        }

        let closed = sqlx::query!(
            "update vedaflow_proposals
             set state = 'published', closed_at = now(), closed_by = $3
             where tenant_id = $1 and id = $2 and state = 'open'",
            tenant.as_uuid(),
            proposal,
            proposer.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("publish own proposal");
        assert_eq!(closed.rows_affected(), 1);
        tx.commit().await.expect("commit the lifecycle");
    });
}

// ── FLOW-4: the usage projection and the sweeper's watermark ─────────────────

/// Seeds one tenant with a usage row and a watermark. The projection is
/// deliberately un-FK'd on `records` (migration 0020), so a bare record id
/// is a faithful fixture: the sweeper folds ids out of the audit chain
/// before anything checks whether they still exist.
async fn seed_usage(pool: &PgPool) -> (TenantId, uuid::Uuid) {
    let (tenant, _) = seed_tenant(pool).await;
    let record = uuid::Uuid::now_v7();
    sqlx::query!(
        "insert into memory_usage
             (tenant_id, record_id, subject, recalls, first_recall_at, last_recall_at)
         values ($1, $2, 'rls-fixture', 3, now(), now())",
        tenant.as_uuid(),
        record,
    )
    .execute(pool)
    .await
    .expect("seed usage");
    sqlx::query!(
        "insert into promotion_watermarks (tenant_id, last_seq) values ($1, 7)",
        tenant.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed watermark");
    (tenant, record)
}

/// Rows of `tenant` visible through the promotion tables, in the order
/// (usage, watermarks).
async fn visible_promotion_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from memory_usage where tenant_id = $1) as "usage!",
             (select count(*) from promotion_watermarks where tenant_id = $1)
                 as "watermarks!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count promotion rows");
    (row.usage, row.watermarks)
}

/// Who recalled what is a behavioural record of named people. It leaks
/// nothing across a tenant boundary, and nothing at all without a GUC.
#[test]
fn wrong_tenant_guc_sees_no_promotion_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_usage(&db.pool).await;
        let (adversary, _) = seed_usage(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert_eq!(
            visible_promotion_rows(&mut tx, victim).await,
            (1, 1),
            "a tenant must see its own usage projection and watermark"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_promotion_rows(&mut tx, victim).await,
            (0, 0),
            "another tenant's usage leaked — which names who recalled what"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_promotion_rows(&mut tx, victim).await,
            (0, 0),
            "an unset GUC must see nothing at all"
        );
    });
}

/// A forged tenant id trips each policy's WITH CHECK, and a cross-tenant
/// watermark advance affects zero rows — which matters more here than it
/// looks: rewinding a victim's watermark would refold their chain and
/// double every count in their evidence.
#[test]
fn cross_tenant_promotion_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_record) = seed_usage(&db.pool).await;
        let (adversary, _) = seed_usage(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into memory_usage
                 (tenant_id, record_id, subject, recalls, first_recall_at, last_recall_at)
             values ($1, $2, 'forged', 99, now(), now())",
            victim.as_uuid(),
            uuid::Uuid::now_v7(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "an adversary wrote a usage row into another tenant's projection"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let rewound = sqlx::query!(
            "update promotion_watermarks set last_seq = 0 where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("the statement runs; RLS makes it match nothing");
        assert_eq!(
            rewound.rows_affected(),
            0,
            "an adversary rewound another tenant's sweep watermark"
        );
        let inflated = sqlx::query!(
            "update memory_usage set recalls = 10_000 where tenant_id = $1 and record_id = $2",
            victim.as_uuid(),
            victim_record,
        )
        .execute(&mut *tx)
        .await
        .expect("the statement runs; RLS makes it match nothing");
        assert_eq!(
            inflated.rows_affected(),
            0,
            "an adversary inflated another tenant's promotion evidence"
        );
    });
}

/// The projection is derived state, and the app role holds the DELETE that
/// says so: ADR-0033 decision 3's rebuild has to be an operation the
/// product can actually perform, unlike the governed-history tables beside
/// it, where the same grant is deliberately absent.
#[test]
fn the_projection_is_rebuildable_by_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_usage(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let cleared = sqlx::query!(
            "delete from memory_usage where tenant_id = $1",
            tenant.as_uuid()
        )
        .execute(&mut *tx)
        .await
        .expect("the app role may discard the projection");
        assert_eq!(cleared.rows_affected(), 1);
        let cleared = sqlx::query!(
            "delete from promotion_watermarks where tenant_id = $1",
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("the app role may discard the watermark");
        assert_eq!(cleared.rows_affected(), 1);
        assert_eq!(
            visible_promotion_rows(&mut tx, tenant).await,
            (0, 0),
            "a reset leaves nothing to refold from"
        );
        tx.commit().await.expect("commit the reset");
    });
}

// ── AUTHZ-4: standing lapse grants (ADR-0037) ───────────────────────────────

/// Seeds a granted lapse: the proposal whose effect it was, and a standing
/// window an hour wide.
async fn seed_lapse(pool: &PgPool) -> (TenantId, ScopeId, uuid::Uuid) {
    let (tenant, target, proposal) = seed_proposal(pool).await;
    let lapse = uuid::Uuid::now_v7();
    sqlx::query!(
        "insert into policy_lapses
             (tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
              action, max_sensitivity, reason, expires_at, granted_by)
         values ($1, $2, $3, $4, $5, 'memory.read', 'internal',
                 'joint incident review', now() + interval '1 hour', $6)",
        tenant.as_uuid(),
        lapse,
        proposal,
        ScopeId::new().as_uuid(),
        target.as_uuid(),
        IdentityId::new().as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed lapse");
    (tenant, target, lapse)
}

/// The three attacks a standing grant invites, all of which the schema has
/// to refuse rather than the handler (ADR-0037's compliance notes).
///
/// The third is the one this table exists to make impossible: an UPDATE
/// that pushes `expires_at` forward turns a 30-day grant into a permanent
/// one *without a second approval*, and it would leave the proposal, the
/// approvals, and the audit chain all still saying "30 days".
#[test]
fn a_grant_cannot_be_forged_resurrected_or_extended() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_target, victim_lapse) = seed_lapse(&db.pool).await;
        let (adversary, _, adversary_lapse) = seed_lapse(&db.pool).await;

        // 1. A grant forged onto another tenant's scope: the ordinary
        //    isolation, on the one table whose rows widen access.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into policy_lapses
                 (tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
                  action, max_sensitivity, reason, expires_at, granted_by)
             values ($1, $2, $3, $4, $5, 'memory.read', 'internal', 'forged',
                     now() + interval '1 hour', $6)",
            victim.as_uuid(),
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            ScopeId::new().as_uuid(),
            victim_target.as_uuid(),
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant grant must be rejected: it would widen another \
             tenant's reads with no proposal behind it"
        );
        drop(tx);

        // A cross-tenant revocation is a legal statement matching nothing —
        // unreachable rather than merely unwritten.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let reached = sqlx::query!(
            "update policy_lapses set revoked_at = now(), revoked_by = $2,
                                      revoke_reason = 'not yours'
             where tenant_id = $1",
            victim.as_uuid(),
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant revoke runs")
        .rows_affected();
        assert_eq!(reached, 0, "another tenant's grant must be unreachable");
        drop(tx);

        // 2. A revocation is terminal: it cannot be undone, and it cannot
        //    be re-cast to move the reason or the actor.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query!(
            "update policy_lapses set revoked_at = now(), revoked_by = $2,
                                      revoke_reason = 'incident closed'
             where tenant_id = $1 and id = $3",
            adversary.as_uuid(),
            IdentityId::new().as_uuid(),
            adversary_lapse,
        )
        .execute(&mut *tx)
        .await
        .expect("revoke own grant");
        let resurrected = sqlx::query!(
            "update policy_lapses set revoked_at = null, revoked_by = null,
                                      revoke_reason = null
             where tenant_id = $1 and id = $2",
            adversary.as_uuid(),
            adversary_lapse,
        )
        .execute(&mut *tx)
        .await;
        assert!(
            resurrected.is_err(),
            "a revoked grant must not be un-revoked: reinstating access is a \
             new proposal, not an UPDATE"
        );
        drop(tx);

        // 3. The window is immutable. This is the attack the trigger exists
        //    for: everything else about the grant still reads as approved.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let extended = sqlx::query!(
            "update policy_lapses set expires_at = now() + interval '3650 days'
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            victim_lapse,
        )
        .execute(&mut *tx)
        .await;
        let message = extended
            .expect_err("expires_at must be immutable")
            .to_string();
        assert!(
            message.contains("second approval"),
            "the refusal must say why, got: {message}"
        );
        drop(tx);

        // 4. The declared tier is immutable, and it is the same attack in
        //    the other dimension (AUTHZ-5, ADR-0038): raised after approval,
        //    an `internal` grant two stewards approved becomes a
        //    `restricted` one no compliance approver ever saw — while the
        //    proposal, the approvals and the chain all still say `internal`.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let widened = sqlx::query!(
            "update policy_lapses set max_sensitivity = 'restricted'
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            victim_lapse,
        )
        .execute(&mut *tx)
        .await;
        let message = widened
            .expect_err("max_sensitivity must be immutable")
            .to_string();
        assert!(
            message.contains("approvers signed"),
            "the refusal must say why, got: {message}"
        );
        drop(tx);

        // And a tier outside the vocabulary is refused by the CHECK, not
        // stored and puzzled over later.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let nonsense = sqlx::query!(
            "insert into policy_lapses
                 (tenant_id, id, proposal_id, grantee_scope_id, target_scope_id,
                  action, max_sensitivity, reason, expires_at, granted_by)
             values ($1, $2, $3, $4, $5, 'memory.read', 'top-secret', 'invented tier',
                     now() + interval '1 hour', $6)",
            victim.as_uuid(),
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7(),
            ScopeId::new().as_uuid(),
            victim_target.as_uuid(),
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            nonsense.is_err(),
            "a tier outside the vocabulary must be refused at the column"
        );

        drop(tx);

        // And the rest of the terms with it — a moved target scope would
        // point an approved grant at material nobody reviewed, and a moved
        // grantee would hand it to somebody nobody approved.
        //
        // One transaction per attempt: a refused statement aborts the
        // transaction it ran in, so sharing one would prove only that the
        // first refusal happened.
        for column in [
            "target_scope_id = gen_random_uuid()",
            "grantee_scope_id = gen_random_uuid()",
            "reason = 'something else'",
            "granted_at = now() - interval '1 day'",
            "proposal_id = gen_random_uuid()",
            "granted_by = gen_random_uuid()",
        ] {
            let mut tx = app_tx(&db.pool, Some(victim)).await;
            let statement =
                format!("update policy_lapses set {column} where id = '{victim_lapse}'");
            let outcome = sqlx::raw_sql(&statement).execute(&mut *tx).await;
            assert!(outcome.is_err(), "{column} must be immutable");
            drop(tx);
        }

        // The app role holds no DELETE: a grant is the record of why an
        // inject composed what it composed, and the outcome is rendered
        // from the row rather than by removing it.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let deleted = sqlx::raw_sql("delete from policy_lapses")
            .execute(&mut *tx)
            .await;
        assert!(
            deleted.is_err(),
            "the app role must hold no DELETE on grants"
        );
        drop(tx);

        // The trigger, for whoever is not running under RLS at all.
        let owner = sqlx::query!(
            "delete from policy_lapses where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(
            owner.is_err(),
            "the table owner must not be able to delete a grant either"
        );
    });
}

/// The expiry stamp is bookkeeping and is written once: two overlapping
/// sweeps cannot chain one expiry twice.
///
/// The row keeps deciding nothing either way — `expires_at` passed — so
/// this guards the audit chain against a duplicate, not access against a
/// leak.
#[test]
fn an_expiry_can_only_be_chained_once() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, lapse) = seed_lapse(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;

        let first = sqlx::query!(
            "update policy_lapses set expiry_recorded_at = now()
             where tenant_id = $1 and id = $2 and expiry_recorded_at is null",
            tenant.as_uuid(),
            lapse,
        )
        .execute(&mut *tx)
        .await
        .expect("stamp the expiry")
        .rows_affected();
        assert_eq!(first, 1);

        // The loser of the race matches nothing rather than double-chaining.
        let second = sqlx::query!(
            "update policy_lapses set expiry_recorded_at = now()
             where tenant_id = $1 and id = $2 and expiry_recorded_at is null",
            tenant.as_uuid(),
            lapse,
        )
        .execute(&mut *tx)
        .await
        .expect("the second sweep runs")
        .rows_affected();
        assert_eq!(second, 0, "one expiry, one event");

        // And an unguarded restamp raises rather than moving the record.
        let restamped = sqlx::query!(
            "update policy_lapses set expiry_recorded_at = now() + interval '1 day'
             where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            lapse,
        )
        .execute(&mut *tx)
        .await;
        assert!(restamped.is_err(), "a chained expiry must not move");
    });
}

// ── MEM-6: the destruction path (ADR-0040 decision 6) ───────────────────────

/// The one statement in the product that removes recorded content, and the
/// three things that must stay true of it.
///
/// Migration 0025 opens `records_history` to DELETE only while a named
/// flag is set, because migration 0001's own comment says the append-only
/// trigger "is not a security boundary … defence in depth against
/// application bugs". The boundary that *is* one is RLS, and this test is
/// what says so: with the flag on and the app role's new grant in hand, a
/// purge still cannot reach another tenant's history — however the flag is
/// set, and whatever tenant the statement names.
#[test]
fn a_purge_is_flag_gated_scoped_to_its_tenant_and_never_a_rewrite() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_tenant(&db.pool).await;
        let (adversary, _) = seed_tenant(&db.pool).await;
        // Each fixture's update archived one version.
        let archived = |tenant: TenantId| async move {
            sqlx::query_scalar!(
                r#"select count(*) as "count!" from records_history where tenant_id = $1"#,
                tenant.as_uuid(),
            )
            .fetch_one(&db.pool)
            .await
            .expect("count history")
        };
        assert_eq!(archived(victim).await, 1);
        assert_eq!(archived(adversary).await, 1);

        // 1. Without the flag, the grant buys nothing: the trigger refuses.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let unflagged = sqlx::query!(
            "delete from records_history where tenant_id = $1",
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            unflagged.is_err(),
            "history is append-only until something deliberately says otherwise"
        );
        drop(tx);

        // 2. With the flag, a purge naming the victim's tenant is a legal
        //    statement that matches nothing. This is the attack: the
        //    adversary knows the flag, holds the grant, and names the rows.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query_scalar!("select set_config('synveda.retention_purge', 'on', true)")
            .fetch_one(&mut *tx)
            .await
            .expect("set the purge flag");
        let reached = sqlx::query!(
            "delete from records_history where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant purge runs")
        .rows_affected();
        assert_eq!(
            reached, 0,
            "another tenant's history must be unreachable — the flag opens the \
             trigger, never the isolation policy"
        );
        // Unqualified, it destroys exactly its own.
        let own = sqlx::query!("delete from records_history")
            .execute(&mut *tx)
            .await
            .expect("own purge runs")
            .rows_affected();
        assert_eq!(
            own, 1,
            "the adversary can only ever destroy its own history"
        );
        tx.commit().await.expect("commit purge");
        assert_eq!(archived(victim).await, 1, "the victim's history stands");
        assert_eq!(archived(adversary).await, 0);

        // 3. The flag opens a DELETE and nothing else: a rewrite of history
        //    is refused with the flag on, which is what keeps "destroyed"
        //    and "altered" different words.
        let (rewriter, _) = seed_tenant(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(rewriter)).await;
        sqlx::query_scalar!("select set_config('synveda.retention_purge', 'on', true)")
            .fetch_one(&mut *tx)
            .await
            .expect("set the purge flag");
        let rewritten = sqlx::query!(
            "update records_history set content = 'never happened' where tenant_id = $1",
            rewriter.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            rewritten.is_err(),
            "retention destroys rows; it never edits one"
        );
    });
}

/// The staging plane's new DELETE grants (migration 0025), under the same
/// adversarial reading: disposal is per tenant, and the marker cannot
/// outlive the row it points at.
#[test]
fn staging_disposal_is_scoped_to_its_tenant_and_takes_its_markers_with_it() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_event) = seed_quarantined(&db.pool).await;
        let (adversary, _) = seed_quarantined(&db.pool).await;

        // A disposal naming another tenant's staging rows matches nothing.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query_scalar!("select set_config('synveda.retention_purge', 'on', true)")
            .fetch_one(&mut *tx)
            .await
            .expect("declare the disposal");
        let reached = sqlx::query!(
            "delete from observe_quarantine where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant marker disposal runs")
        .rows_affected();
        assert_eq!(reached, 0);
        let reached = sqlx::query!(
            "delete from observe_events where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant staging disposal runs")
        .rows_affected();
        assert_eq!(reached, 0, "another tenant's payloads are unreachable");
        drop(tx);

        // And the FK is the order: the staging row cannot go first, which
        // is why the sweep disposes of markers before payloads (ADR-0040
        // decision 7).
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let orphaned = sqlx::query!(
            "delete from observe_events where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            victim_event.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            orphaned.is_err(),
            "a quarantine marker must never point at a payload that is gone"
        );
        drop(tx);

        // Migration 0013's trigger refuses a marker delete outright until
        // the transaction says it is a retention disposal — the same flag
        // the history purge sets, and the reason a handler cannot retire a
        // pending review by accident.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let undeclared = sqlx::query!(
            "delete from observe_quarantine where tenant_id = $1 and event_id = $2",
            victim.as_uuid(),
            victim_event.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            undeclared.is_err(),
            "a marker delete outside a declared disposal must raise"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        sqlx::query_scalar!("select set_config('synveda.retention_purge', 'on', true)")
            .fetch_one(&mut *tx)
            .await
            .expect("declare the disposal");
        sqlx::query!(
            "delete from observe_quarantine where tenant_id = $1 and event_id = $2",
            victim.as_uuid(),
            victim_event.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("dispose of the marker");
        let disposed = sqlx::query!(
            "delete from observe_events where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            victim_event.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("dispose of the payload")
        .rows_affected();
        assert_eq!(disposed, 1, "marker first, then the payload it named");
        tx.commit().await.expect("commit disposal");
    });
}

// ── Graph (GRPH-1, ADR-0043) ────────────────────────────────────────────────

/// Seeds a tenant with a two-vertex entity graph and one edge that has
/// already been superseded once, so `graph_vertices`, `graph_edges`,
/// `graph_edges_history` and `graph_edges_versions` all hold rows for it.
/// One vertex is backed by the tenant's record, which is what the cascade
/// test below pulls on.
///
/// Raw SQL because the traversal API is GRPH-1's next commit and this suite
/// is about the backstop, not about the query surface — the VedaFlow seed
/// above sets the precedent. Returns (tenant, record, source vertex, edge).
async fn seed_graph(pool: &PgPool) -> (TenantId, RecordId, uuid::Uuid, uuid::Uuid) {
    let (tenant, record) = seed_tenant(pool).await;
    let src = uuid::Uuid::now_v7();
    let dst = uuid::Uuid::now_v7();
    let edge = uuid::Uuid::now_v7();

    sqlx::query!(
        "insert into graph_vertices (id, tenant_id, graph, kind, key, label, record_id)
         values ($1, $3, 'entity', 'person', 'alice', 'Alice', $4),
                ($2, $3, 'entity', 'org', 'acme', 'Acme Corp.', null)",
        src,
        dst,
        tenant.as_uuid(),
        record.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed vertices");

    sqlx::query!(
        "insert into graph_edges
             (id, tenant_id, graph, kind, src_id, dst_id, method,
              confidence_permille, valid_from)
         values ($1, $2, 'entity', 'works_for', $3, $4, 'deterministic', 900, now())",
        edge,
        tenant.as_uuid(),
        src,
        dst,
    )
    .execute(pool)
    .await
    .expect("seed edge");

    // A supersession, so history holds the version this one closed
    // (ADR-0043 decision 4). The tick makes the replaced version's
    // transaction period non-empty, exactly as the records fixture does.
    tick().await;
    sqlx::query!(
        "update graph_edges set confidence_permille = 1000 where id = $1",
        edge,
    )
    .execute(pool)
    .await
    .expect("supersede edge");

    (tenant, record, src, edge)
}

/// Rows of `tenant` visible through the graph relations, in the order
/// (vertices, edges, edge history, edge versions).
async fn visible_graph_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from graph_vertices where tenant_id = $1) as "vertices!",
             (select count(*) from graph_edges where tenant_id = $1) as "edges!",
             (select count(*) from graph_edges_history where tenant_id = $1) as "history!",
             (select count(*) from graph_edges_versions where tenant_id = $1) as "versions!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count graph rows");
    (row.vertices, row.edges, row.history, row.versions)
}

/// The backstop over the graph. An edge is a disclosure that its endpoints
/// exist and are related, so a graph that leaked across tenants would leak
/// the shape of another organisation's world without ever showing a record
/// body — which is why ADR-0043 keeps both the structural guarantee and this
/// one (decisions 7 and 8).
#[test]
fn wrong_tenant_guc_sees_no_graph_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, ..) = seed_graph(&db.pool).await;
        let (adversary, ..) = seed_graph(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        assert_eq!(
            visible_graph_rows(&mut tx, victim).await,
            (2, 1, 1, 2),
            "the victim sees its own graph"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_graph_rows(&mut tx, victim).await,
            (0, 0, 0, 0),
            "the graph leaked across tenants"
        );
        drop(tx);

        // No GUC at all: the connection that forgot to declare its tenant
        // sees nothing, including through the as-of view.
        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_graph_rows(&mut tx, victim).await,
            (0, 0, 0, 0),
            "an undeclared tenant must see no graph rows"
        );
    });
}

/// The write side: a forged tenant trips WITH CHECK on both tables, and an
/// edge cannot name a vertex outside its own tenant *or* outside its own
/// named graph — the composite `(tenant_id, graph, id)` foreign keys make
/// both unrepresentable rather than merely refused (ADR-0043 decisions 6
/// and 7, answering ADR-0004 option 2's leak-by-omission objection).
#[test]
fn graph_writes_cannot_forge_a_tenant_or_cross_a_boundary() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, victim_src, _) = seed_graph(&db.pool).await;
        let (adversary, _, adversary_src, _) = seed_graph(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            "insert into graph_vertices (id, tenant_id, graph, kind, key, label)
             values ($1, $2, 'entity', 'person', 'mallory', 'Mallory')",
            uuid::Uuid::now_v7(),
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant vertex write must be rejected"
        );
        drop(tx);

        // An edge of the adversary's own tenant, reaching for the victim's
        // vertex: the composite foreign key has no such row to point at.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let cross_tenant = sqlx::query!(
            "insert into graph_edges
                 (id, tenant_id, graph, kind, src_id, dst_id, method,
                  confidence_permille, valid_from)
             values ($1, $2, 'entity', 'knows', $3, $4, 'deterministic', 500, now())",
            uuid::Uuid::now_v7(),
            adversary.as_uuid(),
            adversary_src,
            victim_src,
        )
        .execute(&mut *tx)
        .await;
        assert!(
            cross_tenant.is_err(),
            "a cross-tenant edge must be unrepresentable, not merely invisible"
        );
        drop(tx);

        // Same tenant, different named graph: an episode vertex is not an
        // entity vertex, and the discriminator is in the key.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let episode = uuid::Uuid::now_v7();
        sqlx::query!(
            "insert into graph_vertices (id, tenant_id, graph, kind, key, label)
             values ($1, $2, 'episode', 'meeting', 'q3-review', 'Q3 review')",
            episode,
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("an episode vertex is ordinary in-tenant data");
        let cross_graph = sqlx::query!(
            "insert into graph_edges
                 (id, tenant_id, graph, kind, src_id, dst_id, method,
                  confidence_permille, valid_from)
             values ($1, $2, 'entity', 'attended', $3, $4, 'deterministic', 500, now())",
            uuid::Uuid::now_v7(),
            adversary.as_uuid(),
            adversary_src,
            episode,
        )
        .execute(&mut *tx)
        .await;
        assert!(
            cross_graph.is_err(),
            "an edge must not join two named graphs"
        );
    });
}

/// Least privilege over the pair. The app role may close an edge's window
/// and insert its replacement (ADR-0043 decision 4) but may not delete one:
/// direct authorship or deletion of an edge is reserved for "a new action, a
/// new grant and a new ADR". History is append-only by grant as well as by
/// trigger, and destruction past a horizon stays retention's to add.
#[test]
fn the_app_role_cannot_delete_edges_or_rewrite_graph_history() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, ..) = seed_graph(&db.pool).await;

        for statement in [
            "delete from graph_edges",
            "delete from graph_vertices",
            "delete from graph_edges_history",
            "update graph_edges_history set confidence_permille = 1",
        ] {
            let mut tx = app_tx(&db.pool, Some(tenant)).await;
            let refused = sqlx::raw_sql(statement).execute(&mut *tx).await;
            assert!(
                refused.is_err(),
                "the app role must not be able to run `{statement}`"
            );
        }

        // Truncate is refused even where the grant would allow it, because
        // it would take rows out without archiving them.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let truncated = sqlx::raw_sql("truncate graph_edges")
            .execute(&mut *tx)
            .await;
        assert!(truncated.is_err(), "truncate must not bypass the archive");
    });
}

/// The one path rows do leave by: a record destroyed by retention takes its
/// backed vertex with it, the vertex takes its claims, and every claim lands
/// in history on the way out. Foreign-key actions bypass grants and RLS by
/// Postgres semantics, so this is the cascade the missing DELETE grant above
/// deliberately leaves intact.
#[test]
fn destroying_a_record_cascades_through_the_graph_into_history() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, record, ..) = seed_graph(&db.pool).await;
        tick().await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        assert_eq!(visible_graph_rows(&mut tx, tenant).await, (2, 1, 1, 2));
        records::delete(&mut *tx, record)
            .await
            .expect("delete the backing record");

        let (vertices, edges, history, versions) = visible_graph_rows(&mut tx, tenant).await;
        assert_eq!(
            (vertices, edges),
            (1, 0),
            "the record's cascade must remove its vertex and that vertex's edges"
        );
        assert_eq!(
            (history, versions),
            (2, 2),
            "and the cascaded edge must be archived, not dropped — the \
             history holds both the superseded version and the closed one"
        );
        tx.commit().await.expect("commit");
    });
}

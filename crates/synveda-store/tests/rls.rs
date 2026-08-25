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
    access, configuration, context, idempotency, identities, knowledge, policy_packs, projects,
    quarantine, relaxations, repositories, rls, scopes, sessions, tenants, workspaces,
};
// The generic scope vocabulary, reached through its module because the old
// hierarchy still owns the root name until Prompt 6 (CPR-3, ADR-0070).
use synveda_types::access::{GrantSource, GrantSubject, GroupSource, RoleKey};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType,
    KnowledgeType,
};
use synveda_types::repository;
use synveda_types::scope;
use synveda_types::session::{SessionEventType, SessionStatus};
use synveda_types::{
    ArtifactFamily, ArtifactReference, ContextCandidateId, ContextCompletionStatus,
    ContextFeedbackId, ContextFeedbackType, ContextReasonCode, ContextRunId, ContextSelectionId,
    Error, GrantId, GroupId, IdentityId, IdentityKind, InviteId, KnowledgeItemId,
    KnowledgeRevisionId, KnowledgeSourceId, PackConfig, ProjectId, ProposalId, RecordClass,
    RecordId, RecordKind, RelaxationId, RelaxationVersionId, RepositoryId, ScopeId, Sensitivity,
    SessionId, TenantId, TenantStatus, TraceRetentionMode, WorkspaceId,
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
///
/// CPR-2 (ADR-0069) adds `schema_metadata`, and it is deliberately absent for
/// the reason `deployment_keys` and `console_sessions` are: it carries no
/// `tenant_id`, so the guard does not discover it and no exemption was needed.
/// Structural rather than granted, and structurally *necessary* here — the
/// epoch guard runs before a tenant is resolved, which is exactly when a
/// tenant-keyed predicate would evaluate to false and hide the marker from
/// the check that exists to read it.
const COVERED: &[&str] = &[
    "audit_chain_heads",
    "audit_log",
    // CPR-25 (ADR-0086): trusted MCP catalogue metadata, immutable discovery
    // snapshots, exact project bindings, typed VedaFlow changes and read-only
    // test evidence are all tenant-confidential and owner-bypass-proof.
    "capability_snapshots",
    // CPR-18 (ADR-0083): frozen session evidence, its reviewable proposals,
    // visible match hints and durable decisions all disclose transcript- or
    // Knowledge-derived state and therefore share the tenant boundary.
    "capture_batch_events",
    "capture_batches",
    "capture_candidate_decisions",
    "capture_candidate_events",
    // CPR-27 (ADR-0087): a candidate's immutable OKF artifact evidence is a
    // separate source family, never a fabricated session event.
    "capture_candidate_import_artifacts",
    "capture_candidate_matches",
    "capture_candidates",
    // CPR-30 (ADR-0089): Configuration heads, immutable versions, revisioned
    // bindings and typed VedaFlow effects can reveal policy and runtime
    // posture, so all four are tenant-bound and forced through RLS.
    "configuration_artifacts",
    "configuration_bindings",
    "configuration_changes",
    "configuration_versions",
    // CPR-20 (ADR-0084): policy-visible planner evidence and explicit
    // feedback are tenant-bound, even when retention keeps hashes only.
    "context_candidates",
    "context_feedback",
    "context_pack_chunks",
    "context_pack_documents",
    "context_packs",
    "context_selections",
    "directory_sync_state",
    // CPR-16 (ADR-0081): the reusable operation ledger is tenant-bound just
    // like the effect it executes; a guessed job id must not reveal whether
    // another tenant is erasing anything.
    "durable_operations",
    "graph_edges",
    "graph_edges_history",
    "graph_vertices",
    // CPR-5 (ADR-0072): the access plane. A group is a tenant's own set of
    // principals, and `group_members` is its content — both tenant-bound, both
    // forced, because "who is in engineering" is exactly the kind of fact one
    // tenant must not learn about another.
    "group_members",
    "groups",
    // CPR-4 (ADR-0071): the record that makes a creation retryable, and the
    // three product-level subtype tables below. `idempotency_records` is
    // tenant-bound like the rest — a key is a client's claim about a request
    // to *this* tenant, and a cross-tenant lookup would let one tenant learn
    // that another used a key it guessed.
    "idempotency_records",
    "identities",
    // CPR-27 (ADR-0087): immutable import plans retain source paths,
    // extension metadata and proposed Knowledge until a reviewer decides a
    // candidate. Every row is therefore tenant-confidential.
    "import_artifacts",
    "import_jobs",
    "import_mappings",
    // CPR-15/16 (ADR-0080/0081): stable Knowledge heads, immutable revisions,
    // independently governed sources, explicit relation claims, typed
    // VedaFlow effects and content-free erasure/index evidence. Neither
    // provenance, governance state nor the fact that erasure happened may
    // become a cross-tenant side channel.
    "knowledge_changes",
    "knowledge_erasure_tombstones",
    "knowledge_index_invalidations",
    "knowledge_items",
    "knowledge_items_history",
    "knowledge_relations",
    "knowledge_revision_embeddings",
    "knowledge_revision_sources",
    "knowledge_revisions",
    "knowledge_sources",
    "memory_usage",
    // CPR-5 (ADR-0072): an outstanding invitation is a live credential's
    // shadow. Tenant-bound so a hash lookup runs inside one tenant's own row
    // policy — the shape ADR-0059 decision 13 set for the provisioning
    // credential, and for the same threat model.
    "pending_invites",
    "policy_packs",
    // CPR-31 (ADR-0090): the stable head, immutable reviewed versions and
    // typed VedaFlow command projection are independently tenant-bound.
    "policy_relaxation_changes",
    "policy_relaxation_versions",
    "policy_relaxations",
    "project_repositories",
    "projects",
    "promotion_watermarks",
    "prompts",
    "record_embeddings",
    "record_signatures",
    "record_supersessions",
    "records",
    "records_history",
    "scim_credentials",
    "scim_users",
    // CPR-3 (ADR-0070): the generic scope substrate, and since CPR-7
    // (ADR-0074) the only scope tree there is — `hierarchy_nodes`,
    // `hierarchy_closure` and `role_bindings` left this inventory with the
    // model they belonged to, and nothing unforced replaced them.
    "scope_closure",
    // CPR-5 (ADR-0072): who holds what, where. The table a cross-tenant read
    // would turn into an org chart.
    "scope_grants",
    "scopes",
    // CPR-10 (ADR-0076): the session ledger. A run's events are a transcript
    // of what somebody and their agent said, read and changed, and a context
    // run holds the composed block itself — so all three carry material one
    // tenant must never see of another, and all three are forced.
    "session_context_runs",
    "session_event_quarantine",
    "session_events",
    "sessions",
    "skill_bindings",
    "skill_changes",
    "skill_test_runs",
    "skill_usage_events",
    "skill_version_files",
    "skill_versions",
    "skills",
    // TEN-4 (ADR-0064). `deployment_keys` is deliberately absent: it carries
    // no `tenant_id`, so this guard does not discover it and no exemption was
    // needed — the same structural satisfaction `console_sessions` has, and
    // for the same reason (decision 5).
    "tenant_keys",
    "tenant_secrets",
    "tool_bindings",
    "tool_changes",
    "tool_server_versions",
    "tool_servers",
    "tool_test_runs",
    "vedaflow_commit_parents",
    "vedaflow_commits",
    "vedaflow_objects",
    "vedaflow_proposal_approvals",
    "vedaflow_proposals",
    "vedaflow_refs",
    "vedaflow_tree_entries",
    "vedaflow_trees",
    "workspaces",
];

/// Discovers every tenant-scoped table (structural definition, ADR-0009: any
/// public base table with a `tenant_id` column) and fails unless each is
/// covered here and carries enabled + FORCED row security with at least one
/// policy. Also pins every current/as-of view to `security_invoker`, without
/// which the view would evaluate RLS as its owner and bypass the backstop.
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

        // Every composed current/as-of surface: the old corpus's (ADR-0006),
        // the graph's (ADR-0043 decision 3), and CPR-15's Knowledge pair
        // (ADR-0080 decisions 2 and 6).
        for view in [
            "records_versions",
            "graph_edges_versions",
            "knowledge_item_versions",
            "knowledge_current",
        ] {
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

// ── Scope substrate (CPR-3, ADR-0070) ───────────────────────────────────────

/// Rows of `tenant` visible through the scope tables, in the order
/// (scopes, scope_closure).
async fn visible_scope_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64) {
    let scopes = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scopes where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count scopes");
    let closure = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scope_closure where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count scope_closure");
    (scopes, closure)
}

fn new_scope(
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: scope::ScopeKind,
    slug: &str,
) -> scopes::NewScope {
    scopes::NewScope {
        id: ScopeId::new(),
        tenant_id: tenant,
        kind,
        parent_scope_id: parent,
        slug: slug.to_owned(),
        display_name: slug.to_owned(),
        attributes: serde_json::json!({}),
        principal_id: (kind == scope::ScopeKind::Principal).then(|| format!("subject-{slug}")),
        created_by: None,
    }
}

/// Admits a tenant with a root scope and one workspace: 2 scopes, 3 closure
/// rows (two self-rows + one edge). Runs on the (RLS-exempt) test connection.
async fn seed_scopes(pool: &PgPool) -> (TenantId, ScopeId) {
    let tenant = TenantId::new();
    let slug = format!("rlss-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS scope fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let root = scopes::create(
        &mut tx,
        &new_scope(tenant, None, scope::ScopeKind::Tenant, "acme"),
    )
    .await
    .expect("create the tenant root");
    scopes::create(
        &mut tx,
        &new_scope(
            tenant,
            Some(root.id),
            scope::ScopeKind::Workspace,
            "workspace",
        ),
    )
    .await
    .expect("create a workspace");
    tx.commit().await.expect("commit scopes");
    (tenant, root.id)
}

/// The wrong (or absent) tenant GUC sees zero scope rows; the right one sees
/// exactly its own.
#[test]
fn wrong_tenant_guc_sees_no_scope_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_scopes(&db.pool).await;
        let (adversary, _) = seed_scopes(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_scope_rows(&mut tx, victim).await,
            (0, 0),
            "scope rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_scope_rows(&mut tx, adversary).await, (2, 3));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_scope_rows(&mut tx, victim).await,
            (0, 0),
            "scope rows visible without any tenant GUC"
        );
    });
}

/// Writing scope rows for another tenant than the GUC's trips the policies'
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_scope_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_scopes(&db.pool).await;
        let (other, _) = seed_scopes(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = scopes::create(
            &mut tx,
            &new_scope(other, None, scope::ScopeKind::Tenant, "forged"),
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant scope insert must be rejected by RLS as an internal \
             defect, got {result:?}"
        );
    });
}

/// The full scope lifecycle — create, move (closure surgery needs no UPDATE
/// on the closure table), rename — works as `synveda_app` with the right GUC:
/// the backstop isolates, it does not deny service.
#[test]
fn same_tenant_scope_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, root) = seed_scopes(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let unit = scopes::create(
            &mut tx,
            &new_scope(tenant, Some(root), scope::ScopeKind::OrgUnit, "unit"),
        )
        .await
        .expect("create under RLS");
        let space = scopes::create(
            &mut tx,
            &new_scope(tenant, Some(root), scope::ScopeKind::Workspace, "space"),
        )
        .await
        .expect("create a second child under RLS");
        let moved = scopes::move_scope(&mut tx, tenant, space.id, unit.id)
            .await
            .expect("move under RLS");
        assert_eq!(moved.parent_scope_id, Some(unit.id));
        assert_eq!(
            scopes::path(&mut *tx, tenant, moved.id)
                .await
                .expect("path under RLS"),
            Some("acme/unit/space".to_owned())
        );
        let renamed = scopes::rename(&mut *tx, tenant, moved.id, "Shared space")
            .await
            .expect("rename under RLS");
        assert_eq!(renamed.display_name, "Shared space");
        tx.commit().await.expect("commit lifecycle");
    });
}

/// The application role holds no DELETE on `scopes`: nothing in the product
/// removes a scope, and a grant for a path that does not exist is a grant
/// nobody reviewed.
#[test]
fn the_app_role_cannot_delete_a_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, root) = seed_scopes(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = sqlx::query!("delete from scopes where id = $1", root.as_uuid())
            .execute(&mut *tx)
            .await;
        let err = result.expect_err("the app role must not be able to delete a scope");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("42501"),
            "expected insufficient_privilege, got {err:?}"
        );
    });
}

// ── Principal scopes (CPR-6, ADR-0073) ──────────────────────────────────────

/// A `principal`-shaped scope is the one row in this schema that is *somebody's
/// own*, and its `principal_id` is what makes it findable. The forced-RLS
/// backstop has to hold for that lookup specifically: a subject is not a
/// secret, so a resolver that could find another tenant's row by subject would
/// be an existence oracle for who works where.
#[test]
fn a_principal_scope_is_not_findable_across_tenants() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (ours, _) = seed_scopes(&db.pool).await;
        let (theirs, _) = seed_scopes(&db.pool).await;

        // The same subject, in both tenants — the case a shared-subject
        // deployment actually produces.
        for tenant in [ours, theirs] {
            let mut tx = app_tx(&db.pool, Some(tenant)).await;
            scopes::ensure_principal_scope(&mut tx, tenant, "alice", "Alice")
                .await
                .expect("mint under RLS");
            tx.commit().await.expect("commit");
        }

        let mut tx = app_tx(&db.pool, Some(ours)).await;
        let mine = scopes::principal_scope(&mut *tx, ours, "alice")
            .await
            .expect("read ours")
            .expect("ours exists");
        // Asking *this* tenant's connection for the other tenant's row: the
        // SQL filter says no and the row policy says no, independently.
        assert_eq!(
            scopes::principal_scope(&mut *tx, theirs, "alice")
                .await
                .expect("read theirs"),
            None,
            "another tenant's own scope must be absent, not forbidden"
        );
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from scopes where principal_id = 'alice'"#
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count principal scopes");
        assert_eq!(
            visible, 1,
            "an unfiltered query still sees exactly this tenant's row"
        );
        assert_eq!(mine.tenant_id, ours);
        tx.commit().await.expect("commit");
    });
}

/// Whose a private scope is cannot be edited — by the **owner** role either,
/// which is what migrations, break-glass psql and a restore run as and what
/// forced RLS does not bind. Re-pointing one would hand somebody's material to
/// a new subject without a single grant row changing.
#[test]
fn a_principal_scope_cannot_be_re_pointed_even_by_the_owner() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_scopes(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let mine = scopes::ensure_principal_scope(&mut tx, tenant, "alice", "Alice")
            .await
            .expect("mint");
        tx.commit().await.expect("commit");

        // The owner connection: no RLS, no application grants, nothing but the
        // trigger between this and somebody else's notes.
        let result = sqlx::query!(
            "update scopes set principal_id = 'mallory' where id = $1",
            mine.id.as_uuid()
        )
        .execute(&db.pool)
        .await;
        assert!(
            result.is_err(),
            "the owner must not be able to re-point a private scope"
        );
    });
}

// ── Workspaces, projects, repositories (CPR-4, ADR-0071) ────────────────────

/// Rows of `tenant` visible through the four CPR-4 tables, in the order
/// (workspaces, projects, project_repositories, idempotency_records).
async fn visible_subtype_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64) {
    let workspaces = sqlx::query_scalar!(
        r#"select count(*) as "count!" from workspaces where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count workspaces");
    let projects = sqlx::query_scalar!(
        r#"select count(*) as "count!" from projects where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count projects");
    let repositories = sqlx::query_scalar!(
        r#"select count(*) as "count!" from project_repositories where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count project_repositories");
    let keys = sqlx::query_scalar!(
        r#"select count(*) as "count!" from idempotency_records where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count idempotency_records");
    (workspaces, projects, repositories, keys)
}

/// Admits a tenant with one workspace, one project, one repository and one
/// idempotency record. Runs on the (RLS-exempt) test connection.
async fn seed_subtypes(pool: &PgPool) -> (TenantId, WorkspaceId, ProjectId) {
    let tenant = TenantId::new();
    let slug = format!("rlsw-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS workspace fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "payments".to_owned(),
            display_name: "Payments".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace");
    let project = projects::create(
        &mut tx,
        &projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            slug: "ledger".to_owned(),
            display_name: "Ledger".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create project");
    repositories::attach(
        &mut *tx,
        &repositories::NewRepository {
            id: RepositoryId::new(),
            tenant_id: tenant,
            project_id: project.id,
            identity: repository::identify(
                Some("https://github.com/acme/ledger.git"),
                None,
                None,
                None,
            )
            .expect("canonical identity"),
            default_branch: Some("main".to_owned()),
            metadata: serde_json::json!({}),
            created_by: None,
        },
    )
    .await
    .expect("attach repository");
    idempotency::remember(
        &mut *tx,
        tenant,
        "rls-subject",
        "workspace.create",
        "rls-key",
        &[7u8; 32],
        workspace.id.as_uuid(),
    )
    .await
    .expect("remember an idempotency key");
    tx.commit().await.expect("commit subtypes");
    (tenant, workspace.id, project.id)
}

/// The wrong (or absent) tenant GUC sees zero rows in all four tables; the
/// right one sees exactly its own.
#[test]
fn wrong_tenant_guc_sees_no_workspace_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, _) = seed_subtypes(&db.pool).await;
        let (adversary, _, _) = seed_subtypes(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_subtype_rows(&mut tx, victim).await,
            (0, 0, 0, 0),
            "workspace-plane rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_subtype_rows(&mut tx, adversary).await, (1, 1, 1, 1));
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_subtype_rows(&mut tx, victim).await,
            (0, 0, 0, 0),
            "workspace-plane rows visible without any tenant GUC"
        );
    });
}

/// Writing a workspace for another tenant than the GUC's trips the policy's
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_workspace_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, _) = seed_subtypes(&db.pool).await;
        let (other, _, _) = seed_subtypes(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let result = workspaces::create(
            &mut tx,
            &workspaces::NewWorkspace {
                id: WorkspaceId::new(),
                tenant_id: other,
                slug: "forged".to_owned(),
                display_name: "Forged".to_owned(),
                description: None,
                created_by: None,
            },
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant workspace insert must be rejected by RLS as an \
             internal defect, got {result:?}"
        );
    });
}

/// An idempotency key is not a cross-tenant oracle: the same (subject,
/// operation, key) triple in two tenants is two records, and neither tenant
/// can see the other's.
#[test]
fn an_idempotency_key_is_scoped_to_its_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (one, first_workspace, _) = seed_subtypes(&db.pool).await;
        let (two, second_workspace, _) = seed_subtypes(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(two)).await;
        // The fixture already stored ("rls-subject", "workspace.create",
        // "rls-key") in *both* tenants; each sees only its own resource.
        let found = idempotency::find(&mut *tx, two, "rls-subject", "workspace.create", "rls-key")
            .await
            .expect("find under RLS")
            .expect("this tenant's record");
        assert_eq!(found.resource_id, second_workspace.as_uuid());
        assert_ne!(
            found.resource_id,
            first_workspace.as_uuid(),
            "one tenant's key must never resolve to another tenant's resource"
        );
        assert!(
            idempotency::find(&mut *tx, one, "rls-subject", "workspace.create", "rls-key")
                .await
                .expect("find under RLS")
                .is_none(),
            "another tenant's key must read as absent"
        );
    });
}

/// The full subtype lifecycle works as `synveda_app` with the right GUC: the
/// backstop isolates, it does not deny service. Including the two grants that
/// are deliberately narrower than the rest — `project_repositories` has
/// DELETE (detaching is the API's own verb) and `workspaces` does not.
#[test]
fn same_tenant_workspace_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, workspace, project) = seed_subtypes(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let updated = workspaces::update(
            &mut tx,
            tenant,
            workspace,
            1,
            &workspaces::WorkspaceUpdate {
                display_name: Some("Payments platform".to_owned()),
                ..Default::default()
            },
        )
        .await
        .expect("update under RLS");
        assert_eq!(updated.revision, 2);

        let attached = repositories::for_project(&mut *tx, tenant, project)
            .await
            .expect("list under RLS");
        assert_eq!(attached.len(), 1);
        assert!(
            repositories::detach(&mut *tx, tenant, project, attached[0].id)
                .await
                .expect("detach under RLS"),
            "detach must work in-tenant"
        );
        tx.commit().await.expect("commit lifecycle");
    });
}

/// The application role holds no DELETE on `workspaces` or `projects`:
/// retiring one is a status transition, and a grant for a path that does not
/// exist is a grant nobody reviewed.
#[test]
fn the_app_role_cannot_delete_a_workspace_or_a_project() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, workspace, project) = seed_subtypes(&db.pool).await;

        // A transaction each: the first refusal aborts its transaction, so a
        // second statement inside it would fail with 25P02 and the test would
        // be asserting that Postgres noticed the first failure.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let err = sqlx::query!("delete from workspaces where id = $1", workspace.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("the app role must not delete a workspace");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("42501"),
            "workspaces: expected insufficient_privilege, got {err:?}"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let err = sqlx::query!("delete from projects where id = $1", project.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("the app role must not delete a project");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("42501"),
            "projects: expected insufficient_privilege, got {err:?}"
        );
    });
}

// ── Groups, grants, invitations (CPR-5, ADR-0072) ───────────────────────────

/// Rows of `tenant` visible through the four CPR-5 tables, in the order
/// (groups, group_members, scope_grants, pending_invites).
async fn visible_access_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64) {
    let groups = sqlx::query_scalar!(
        r#"select count(*) as "count!" from groups where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count groups");
    let members = sqlx::query_scalar!(
        r#"select count(*) as "count!" from group_members where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count group_members");
    let grants = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scope_grants where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count scope_grants");
    let invites = sqlx::query_scalar!(
        r#"select count(*) as "count!" from pending_invites where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count pending_invites");
    (groups, members, grants, invites)
}

/// What [`seed_access`] built, so a test can name the rows it wants.
struct AccessFixture {
    tenant: TenantId,
    scope: ScopeId,
    group: GroupId,
    grant: GrantId,
    invite: InviteId,
    member: IdentityId,
    second_member: IdentityId,
}

/// Admits a tenant with a workspace, a group holding one member, a grant to
/// that group at the workspace's scope, and one outstanding invitation. Runs on
/// the (RLS-exempt) test connection.
async fn seed_access(pool: &PgPool) -> AccessFixture {
    let tenant = TenantId::new();
    let slug = format!("rlsa-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS access fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "payments".to_owned(),
            display_name: "Payments".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace");
    let root = scopes::tenant_root(&mut *tx, tenant)
        .await
        .expect("read tenant root")
        .expect("workspace created root");
    let mut member_ids = Vec::new();
    for (subject, display_name) in [("rls-member", "RLS Member"), ("rls-second", "RLS Second")] {
        let identity_id = IdentityId::new();
        let principal_scope = scopes::create(
            &mut tx,
            &scopes::NewScope {
                id: ScopeId::new(),
                tenant_id: tenant,
                kind: scope::ScopeKind::Principal,
                parent_scope_id: Some(root.id),
                slug: format!("member-{}", identity_id.as_uuid().simple()),
                display_name: display_name.to_owned(),
                attributes: serde_json::json!({}),
                principal_id: Some(subject.to_owned()),
                created_by: None,
            },
        )
        .await
        .expect("create member scope");
        identities::create(
            &mut tx,
            identity_id,
            tenant,
            Some(subject),
            IdentityKind::User,
            None,
            Some(display_name),
            principal_scope.id,
        )
        .await
        .expect("create member identity");
        member_ids.push(identity_id);
    }
    let group = access::create_group(
        &mut *tx,
        &access::NewGroup {
            id: GroupId::new(),
            tenant_id: tenant,
            slug: "engineering".to_owned(),
            display_name: "Engineering".to_owned(),
            description: None,
            source: GroupSource::Direct,
            directory_source: None,
            directory_resource_id: None,
            directory_external_id: None,
            created_by: Some("rls-subject".to_owned()),
        },
    )
    .await
    .expect("create group");
    access::set_group_members(
        &mut tx,
        tenant,
        group.id,
        &member_ids[..1],
        Some("rls-subject"),
    )
    .await
    .expect("set members");
    let grant = access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: workspace.scope_id,
            subject: GrantSubject::Group { group_id: group.id },
            role_key: RoleKey::Member,
            source: GrantSource::Direct,
            invite_id: None,
            granted_by: Some("rls-subject".to_owned()),
        },
    )
    .await
    .expect("create grant");
    let invite = access::create_invite(
        &mut *tx,
        &access::NewInvite {
            id: InviteId::new(),
            tenant_id: tenant,
            scope_id: workspace.scope_id,
            role_key: RoleKey::Viewer,
            email: Some("sam@example.com".to_owned()),
            token_hash: [9u8; 32],
            expires_at: chrono::Utc::now() + chrono::Duration::days(7),
            created_by: Some("rls-subject".to_owned()),
        },
    )
    .await
    .expect("create invite");
    tx.commit().await.expect("commit access fixture");
    AccessFixture {
        tenant,
        scope: workspace.scope_id,
        group: group.id,
        grant: grant.id,
        invite: invite.id,
        member: member_ids[0],
        second_member: member_ids[1],
    }
}

/// The wrong (or absent) tenant GUC sees zero rows in all four tables; the
/// right one sees exactly its own.
#[test]
fn wrong_tenant_guc_sees_no_access_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_access(&db.pool).await;
        let adversary = seed_access(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary.tenant)).await;
        assert_eq!(
            visible_access_rows(&mut tx, victim.tenant).await,
            (0, 0, 0, 0),
            "access-plane rows leaked across tenants under the wrong GUC"
        );
        // Two grants in the adversary's own tenant: the workspace's `owner`
        // grant is minted by the API, not by this fixture, so what is here is
        // the group grant alone.
        assert_eq!(
            visible_access_rows(&mut tx, adversary.tenant).await,
            (1, 1, 1, 1)
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_access_rows(&mut tx, victim.tenant).await,
            (0, 0, 0, 0),
            "access-plane rows visible without any tenant GUC"
        );
    });
}

/// Granting into another tenant than the GUC's trips the policy's WITH CHECK
/// — an application defect, surfaced as internal. This is the adversarial
/// case that matters most on this plane: a forged grant is a forged authority.
#[test]
fn cross_tenant_grant_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed_access(&db.pool).await;
        let theirs = seed_access(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(mine.tenant)).await;
        let result = access::create_grant(
            &mut *tx,
            &access::NewGrant {
                id: GrantId::new(),
                tenant_id: theirs.tenant,
                scope_id: theirs.scope,
                subject: GrantSubject::Principal {
                    principal_id: "intruder".to_owned(),
                },
                role_key: RoleKey::Owner,
                source: GrantSource::Direct,
                invite_id: None,
                granted_by: Some("intruder".to_owned()),
            },
        )
        .await;
        assert!(
            matches!(result, Err(Error::Internal { .. })),
            "cross-tenant grant must be rejected by RLS as an internal defect, \
             got {result:?}"
        );
    });
}

/// An invitation token is not a cross-tenant key: the same hash in two tenants
/// is two invitations, and redeeming inside one tenant never reaches the
/// other's — which is what makes the `(tenant_id, token_hash)` lookup safe.
#[test]
fn an_invitation_is_scoped_to_its_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let one = seed_access(&db.pool).await;
        let two = seed_access(&db.pool).await;
        // Both fixtures store the same hash, deliberately.
        let mut tx = app_tx(&db.pool, Some(two.tenant)).await;
        let accepted = access::accept_invite(
            &mut tx,
            two.tenant,
            &[9u8; 32],
            "rls-acceptor",
            chrono::Utc::now(),
        )
        .await
        .expect("redeem this tenant's invitation");
        assert_eq!(accepted.invite.id, two.invite);
        assert_ne!(
            accepted.invite.id, one.invite,
            "one tenant's token must never resolve to another tenant's invitation"
        );
        assert_eq!(accepted.grant.scope_id, two.scope);
        tx.commit().await.expect("commit redemption");

        // And the other tenant's invitation is untouched by it.
        let mut tx = app_tx(&db.pool, Some(one.tenant)).await;
        let still = access::get_invite(&mut *tx, one.tenant, one.invite)
            .await
            .expect("read under RLS")
            .expect("still there");
        assert!(
            still.is_redeemable(chrono::Utc::now()),
            "redeeming in one tenant consumed another tenant's invitation"
        );
    });
}

/// The full access lifecycle works as `synveda_app` with the right GUC: the
/// backstop isolates, it does not deny service.
#[test]
fn same_tenant_access_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_access(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let members = access::members_of(&mut *tx, fixture.tenant, fixture.scope)
            .await
            .expect("resolve members under RLS");
        assert_eq!(members.len(), 1, "the group's one member holds the grant");
        assert_eq!(members[0].principal_id, "rls-member");

        access::update_group(
            &mut tx,
            fixture.tenant,
            fixture.group,
            1,
            &access::GroupUpdate {
                members: Some(vec![fixture.member, fixture.second_member]),
                ..Default::default()
            },
            Some("rls-subject"),
        )
        .await
        .expect("replace membership under RLS");
        assert_eq!(
            access::members_of(&mut *tx, fixture.tenant, fixture.scope)
                .await
                .expect("resolve again")
                .len(),
            2
        );

        access::revoke_grant(&mut tx, fixture.tenant, fixture.grant)
            .await
            .expect("revoke under RLS");
        assert!(
            access::members_of(&mut *tx, fixture.tenant, fixture.scope)
                .await
                .expect("resolve after revocation")
                .is_empty(),
            "revoking the group's grant removes what it conferred"
        );
        tx.commit().await.expect("commit lifecycle");
    });
}

/// The application role holds no DELETE on `groups` and no UPDATE on
/// `scope_grants`: retiring a group is a status transition, and a grant is
/// created and revoked rather than edited. A grant for a path that does not
/// exist is a grant nobody reviewed.
#[test]
fn the_app_role_cannot_delete_a_group_or_edit_a_grant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_access(&db.pool).await;

        // A transaction each: the first refusal aborts its transaction.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let err = sqlx::query!("delete from groups where id = $1", fixture.group.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("the app role must not delete a group");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("42501"),
            "groups: expected insufficient_privilege, got {err:?}"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let err = sqlx::query!(
            "update scope_grants set role_key = 'owner' where id = $1",
            fixture.grant.as_uuid()
        )
        .execute(&mut *tx)
        .await
        .expect_err("the app role must not edit a grant");
        assert_eq!(
            err.as_database_error().and_then(|db| db.code()).as_deref(),
            Some("42501"),
            "scope_grants: expected insufficient_privilege, got {err:?}"
        );
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

// ── Governed Configuration (CPR-30, ADR-0089) ─────────────────────────────

fn encoded_artifact_reference(reference: ArtifactReference) -> serde_json::Value {
    serde_json::to_value([reference]).expect("encode typed artifact reference")
}

fn fixture_configuration_reference(
    command: &synveda_types::configuration::ConfigurationCommand,
    payload_hash: &str,
) -> ArtifactReference {
    use synveda_types::configuration::ConfigurationCommand;

    match command {
        ConfigurationCommand::Create {
            artifact_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        ),
        ConfigurationCommand::Publish {
            artifact_id,
            expected_current_version_id,
            version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            artifact_id.to_string(),
            command.kind(),
            version_id.to_string(),
            Some(expected_current_version_id.to_string()),
        ),
        ConfigurationCommand::Bind {
            binding_id,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.to_owned(), |id| id.to_string()),
            None,
        ),
        ConfigurationCommand::SetBinding {
            binding_id,
            expected_revision,
            pinned_version_id,
            ..
        } => ArtifactReference::new(
            ArtifactFamily::Configuration,
            binding_id.to_string(),
            command.kind(),
            pinned_version_id.map_or_else(|| payload_hash.to_owned(), |id| id.to_string()),
            Some(expected_revision.to_string()),
        ),
    }
    .expect("valid Configuration fixture reference")
}

fn fixture_memory_references(
    proposal: uuid::Uuid,
    operation: &str,
    commit_hash: [u8; 32],
) -> serde_json::Value {
    encoded_artifact_reference(
        ArtifactReference::new(
            ArtifactFamily::Memory,
            proposal.to_string(),
            operation,
            blake3::Hash::from_bytes(commit_hash).to_hex().to_string(),
            None,
        )
        .expect("valid Memory fixture reference"),
    )
}

async fn fixture_configuration_command(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    scope: ScopeId,
    command: &synveda_types::configuration::ConfigurationCommand,
) -> configuration::AppliedConfiguration {
    let proposal = synveda_types::ProposalId::new();
    let actor = IdentityId::new();
    let payload_hash = blake3::hash(
        synveda_types::json::canonicalise(
            &serde_json::to_value(command).expect("encode Configuration command"),
        )
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let artifact_references =
        encoded_artifact_reference(fixture_configuration_reference(command, &payload_hash));
    sqlx::query!(
        "insert into vedaflow_proposals
             (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
              target_channel, commit_hash, sensitivity, title, proposer_id,
              proposer_subject, artifact_references)
         values ($1, $2, $3, $3, 'configuration', 'apply', $4, 'internal',
                 'RLS Configuration fixture', $5, 'rls-fixture', $6)",
        tenant.as_uuid(),
        proposal.as_uuid(),
        scope.as_uuid(),
        &[4_u8; 32][..],
        actor.as_uuid(),
        artifact_references,
    )
    .execute(&mut *tx)
    .await
    .expect("open Configuration fixture proposal");
    configuration::insert_change(&mut *tx, tenant, proposal, command, &payload_hash)
        .await
        .expect("store Configuration change");
    let applied = configuration::apply(&mut *tx, tenant, proposal, "rls-fixture", command)
        .await
        .expect("apply Configuration fixture");
    configuration::complete_change(&mut *tx, tenant, proposal, applied)
        .await
        .expect("complete Configuration fixture");
    applied
}

async fn seed_configuration(
    pool: &PgPool,
) -> (
    TenantId,
    ScopeId,
    synveda_types::ConfigurationArtifactId,
    synveda_types::ConfigurationVersionId,
) {
    // `seed_vedaflow` supplies genuine immutable object/tree/commit history;
    // this RLS fixture binds typed Configuration changes to that commit. The
    // semantic gateway lifecycle is covered by CPR-30's API acceptance test.
    let (tenant, _) = seed_vedaflow(pool).await;
    let mut tx = pool.begin().await.expect("begin Configuration seed");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("create Configuration root");
    let artifact_id = synveda_types::ConfigurationArtifactId::new();
    let version_id = synveda_types::ConfigurationVersionId::new();
    let binding_id = synveda_types::ConfigurationBindingId::new();
    let document = synveda_types::configuration::ConfigurationDocument::template(
        synveda_types::configuration::ConfigurationTemplate::Personal,
    );
    let content_hash = document.content_hash().expect("hash Configuration");
    fixture_configuration_command(
        &mut tx,
        tenant,
        root.id,
        &synveda_types::configuration::ConfigurationCommand::Create {
            artifact_id,
            version_id,
            governing_scope_id: root.id,
            name: "rls-runtime".to_owned(),
            document,
            content_hash,
            source_template: Some(synveda_types::configuration::ConfigurationTemplate::Personal),
        },
    )
    .await;
    fixture_configuration_command(
        &mut tx,
        tenant,
        root.id,
        &synveda_types::configuration::ConfigurationCommand::Bind {
            binding_id,
            scope_id: root.id,
            artifact_id,
            pinned_version_id: None,
            enabled: true,
        },
    )
    .await;
    tx.commit().await.expect("commit Configuration seed");
    (tenant, root.id, artifact_id, version_id)
}

async fn visible_configuration_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from configuration_artifacts where tenant_id = $1) as "artifacts!",
             (select count(*) from configuration_versions where tenant_id = $1) as "versions!",
             (select count(*) from configuration_bindings where tenant_id = $1) as "bindings!",
             (select count(*) from configuration_changes where tenant_id = $1) as "changes!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count Configuration rows");
    (row.artifacts, row.versions, row.bindings, row.changes)
}

#[test]
fn wrong_or_absent_tenant_guc_sees_no_configuration_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, _, _) = seed_configuration(&db.pool).await;
        let (adversary, _, _, _) = seed_configuration(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_configuration_rows(&mut tx, victim).await,
            (0, 0, 0, 0)
        );
        assert_eq!(
            visible_configuration_rows(&mut tx, adversary).await,
            (1, 1, 1, 2)
        );
        drop(tx);
        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_configuration_rows(&mut tx, victim).await,
            (0, 0, 0, 0)
        );
    });
}

#[test]
fn cross_tenant_configuration_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, _, _) = seed_configuration(&db.pool).await;
        let (other, other_root, other_artifact, other_version) = seed_configuration(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let forged = sqlx::query(
            "insert into configuration_bindings
                 (id, tenant_id, scope_id, artifact_id, pinned_version_id,
                  enabled, created_by, updated_by)
             values ($1, $2, $3, $4, $5, true, 'forged', 'forged')",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(other.as_uuid())
        .bind(other_root.as_uuid())
        .bind(other_artifact.as_uuid())
        .bind(other_version.as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(
            forged.as_ref().is_err_and(|error| error
                .as_database_error()
                .and_then(|database| database.code())
                .as_deref()
                == Some("42501")),
            "forced RLS must reject a cross-tenant Configuration write: {forged:?}"
        );
    });
}

#[test]
fn same_tenant_configuration_projection_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, root, _, _) = seed_configuration(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let effective = configuration::effective_at_scope(&mut tx, tenant, root)
            .await
            .expect("resolve Configuration under RLS");
        assert_eq!(effective.document.policy_pack, "open-collaboration");
        assert!(effective.version_id.is_some());
        assert_eq!(
            visible_configuration_rows(&mut tx, tenant).await,
            (1, 1, 1, 2)
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
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure tenant root");
    let personal = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: synveda_types::scope::ScopeKind::Principal,
            parent_scope_id: Some(root.id),
            slug: "alice".to_owned(),
            display_name: "Alice".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: Some("rls-alice".to_owned()),
            created_by: None,
        },
    )
    .await
    .expect("create own scope");
    identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some("alice"),
        IdentityKind::User,
        None,
        None,
        personal.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit identity fixture");
    tenant
}

/// Rows of `tenant` visible through `identities` (the AUTH-2 table the
/// cutover left — group mappings left with the placement convention,
/// CPR-7).
async fn visible_identity_rows(tx: &mut Transaction<'static, Postgres>, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from identities where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count identities")
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
            0,
            "identity rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_identity_rows(&mut tx, adversary).await, 1);
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_identity_rows(&mut tx, victim).await,
            0,
            "identity rows visible without any tenant GUC"
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
        let foreign_root = scopes::tenant_root(&mut *tx, other)
            .await
            .expect("query foreign root");
        assert_eq!(foreign_root, None, "the foreign root must not even read");
        let result = identities::create(
            &mut tx,
            IdentityId::new(),
            other,
            Some("mallory"),
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

/// The provisioning shape — read subject, mint own scope, create
/// identity — works as `synveda_app` with the right GUC (CPR-7: placement
/// is a principal scope minted in the provisioning transaction).
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
        // The seeded fixture's own scope is what the identity is bound to.
        assert_eq!(
            alice.scope_id,
            scopes::principal_scope(&mut *tx, tenant, "rls-alice")
                .await
                .expect("read own scope")
                .expect("alice has a scope")
                .id
        );

        // A second identity mints its own scope under RLS — the cutover's
        // whole placement story (CPR-7, ADR-0074 decision 3).
        let bob_scope = scopes::ensure_principal_scope(&mut tx, tenant, "bob", "Bob")
            .await
            .expect("mint own scope under RLS");
        let bob = identities::create(
            &mut tx,
            IdentityId::new(),
            tenant,
            Some("bob"),
            IdentityKind::User,
            Some("bob@example.test"),
            Some("Bob"),
            bob_scope.id,
        )
        .await
        .expect("create identity under RLS");
        let read_back = identities::by_subject(&mut *tx, tenant, "bob")
            .await
            .expect("read bob")
            .expect("bob is provisioned");
        assert_eq!(read_back.id, bob.id);
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

// ── Session ingestion (CPR-12, ADR-0078) ────────────────────────────────────
//
// The observe staging buffer and its review queue lived here until the observe
// cutover. `session_events` carries the ingestion plane now, so these are the
// same properties over the table that replaced it: cross-tenant blindness, an
// immutability that is a *privilege* rather than a discipline, explicit
// capture eligibility, and a review queue whose verdict is one-shot.
//
// The seeding fixture is [`seed_session`], shared with the session-ledger
// block below: a run is what an event now hangs off, so there is one fixture
// rather than two.

/// A tenant with a run holding one quarantined event, and that event's id.
async fn seed_quarantined(pool: &PgPool) -> (TenantId, SessionId, synveda_types::SessionEventId) {
    let fixture = seed_session(pool).await;
    let mut tx = pool.begin().await.expect("begin");
    let admitted = sessions::append_events(
        &mut tx,
        fixture.tenant,
        fixture.session,
        &[sessions::NewSessionEvent {
            event_type: SessionEventType::MessageUser,
            event_schema_version: 1,
            client_event_id: "rls-q1".to_owned(),
            occurred_at: chrono::Utc::now(),
            payload: serde_json::json!({"text": "[REDACTED:aws-access-key-id] fixture"}),
            redactions: Some(serde_json::json!([
                {"rule": "aws-access-key-id", "category": "secret", "count": 1}
            ])),
            quarantine: true,
        }],
    )
    .await
    .expect("append a quarantined event");
    assert!(
        admitted[0].quarantined,
        "the fixture must actually quarantine"
    );
    tx.commit().await.expect("commit quarantine fixture");
    (fixture.tenant, fixture.session, admitted[0].event.id)
}

async fn visible_event_rows(tx: &mut Transaction<'static, Postgres>, tenant: TenantId) -> i64 {
    sqlx::query_scalar!(
        r#"select count(*) as "count!" from session_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count session_events")
}

/// The wrong (or absent) tenant GUC sees zero events — raw session content is
/// exactly what the backstop exists to protect.
#[test]
fn wrong_tenant_guc_sees_no_session_event_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_session(&db.pool).await;
        let adversary = seed_session(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary.tenant)).await;
        assert_eq!(
            visible_event_rows(&mut tx, victim.tenant).await,
            0,
            "session events leaked across tenants under the wrong GUC"
        );
        assert_eq!(visible_event_rows(&mut tx, adversary.tenant).await, 2);
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_event_rows(&mut tx, victim.tenant).await,
            0,
            "session events visible without any tenant GUC"
        );
    });
}

/// Appending events to another tenant's run trips the policy's WITH CHECK —
/// an application defect, surfaced as internal or as a missing row.
#[test]
fn cross_tenant_session_append_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = seed_session(&db.pool).await;
        let other = seed_session(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant.tenant)).await;
        let result = sessions::append_events(
            &mut tx,
            other.tenant,
            other.session,
            &[sessions::NewSessionEvent {
                event_type: SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: "forged".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({"text": "forged"}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await;
        assert!(
            matches!(
                result,
                Err(Error::NotFound { .. }) | Err(Error::Internal { .. })
            ),
            "a cross-tenant append must be refused, got {result:?}"
        );
    });
}

/// The app role cannot rewrite what was recorded even inside its own tenant:
/// UPDATE on `session_events` was never granted, so immutability is a
/// privilege rather than a discipline (migration 0044).
///
/// DELETE *is* granted since migration 0046, and deliberately: disposal is the
/// obligation 0044 parked on the retention plane. What bounds it is the
/// transaction-local `synveda.retention_purge` flag, not the absence of a
/// grant — so a handler that has not declared itself a disposal still cannot
/// retire a run's transcript.
#[test]
fn session_events_are_immutable_and_only_retention_removes_them() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let update = sqlx::raw_sql("update session_events set payload = '{}'")
            .execute(&mut *tx)
            .await;
        assert!(
            update.is_err(),
            "the app role must not hold UPDATE on session_events"
        );
        drop(tx);

        // A delete that has not declared itself a disposal is refused by the
        // trigger, grant or no grant.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let undeclared = sqlx::raw_sql("delete from session_events")
            .execute(&mut *tx)
            .await;
        assert!(
            undeclared.is_err(),
            "a delete outside a declared retention purge must be refused"
        );
        drop(tx);

        // The sanctioned path removes whole rows, and only this tenant's.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        sqlx::raw_sql("set local synveda.retention_purge = 'on'")
            .execute(&mut *tx)
            .await
            .expect("declare the purge");
        let disposed = sqlx::raw_sql("delete from session_events")
            .execute(&mut *tx)
            .await
            .expect("disposal is granted since CPR-12")
            .rows_affected();
        assert!(disposed > 0, "and it takes whole rows, never part of one");
        let left = visible_event_rows(&mut tx, fixture.tenant).await;
        assert_eq!(left, 0, "the tenant's own events, and only its own");
    });
}

/// The full admission shape — insert, duplicate suppression and capture
/// eligibility — works as `synveda_app` with the right GUC: the backstop
/// isolates, it does not deny service.
#[test]
fn same_tenant_session_admission_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let admitted = sessions::append_events(
            &mut tx,
            fixture.tenant,
            fixture.session,
            // e1 was appended by the seed; e3 is new.
            &[
                sessions::NewSessionEvent {
                    event_type: SessionEventType::MessageUser,
                    event_schema_version: 1,
                    client_event_id: "e1".to_owned(),
                    occurred_at: chrono::Utc::now(),
                    payload: serde_json::json!({"text": "redelivered"}),
                    redactions: None,
                    quarantine: false,
                },
                sessions::NewSessionEvent {
                    event_type: SessionEventType::MessageUser,
                    event_schema_version: 1,
                    client_event_id: "e3".to_owned(),
                    occurred_at: chrono::Utc::now(),
                    payload: serde_json::json!({"text": "fresh"}),
                    redactions: None,
                    quarantine: false,
                },
            ],
        )
        .await
        .expect("append under RLS (pgmq grants included)");
        assert_eq!(
            admitted
                .iter()
                .map(|event| event.outcome)
                .collect::<Vec<_>>(),
            vec![
                sessions::AppendOutcome::Duplicate,
                sessions::AppendOutcome::Appended
            ],
            "the redelivered id must be reported, the fresh one appended"
        );
        tx.commit().await.expect("commit admission");

        // One eligible row per content-carrying event actually appended: the
        // seed's `message.user` and `tool.invoked`, plus e3. The duplicate
        // created no second row.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let eligible = sqlx::query_scalar!(
            r#"select count(*) as "count!"
               from session_events event
               left join session_event_quarantine quarantine
                 on quarantine.tenant_id = event.tenant_id
                and quarantine.event_id = event.id
               where event.tenant_id = $1
                 and event.event_type in (
                     'message.user', 'message.assistant', 'tool.invoked',
                     'tool.result', 'file.changed', 'command.executed',
                     'memory.asserted'
                 )
                 and (quarantine.event_id is null or quarantine.state = 'released')"#,
            fixture.tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count capture-eligible events as synveda_app");
        assert_eq!(
            eligible, 3,
            "one eligible row per appended content event, none per duplicate"
        );
    });
}

/// A type that carries no memory is recorded and **not capture-eligible**: the
/// timeline holds it, the extractor never sees it (ADR-0078 decision 2).
#[test]
fn bookkeeping_events_are_recorded_without_a_work_signal() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;
        let before = {
            let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
            sqlx::query_scalar!(
                r#"select count(*) as "count!" from session_events
                   where tenant_id = $1 and event_type in (
                       'message.user', 'message.assistant', 'tool.invoked',
                       'tool.result', 'file.changed', 'command.executed',
                       'memory.asserted'
                   )"#,
                fixture.tenant.as_uuid(),
            )
            .fetch_one(&mut *tx)
            .await
            .expect("count before")
        };

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        sessions::append_events(
            &mut tx,
            fixture.tenant,
            fixture.session,
            &[sessions::NewSessionEvent {
                event_type: SessionEventType::AdapterWarning,
                event_schema_version: 1,
                client_event_id: "warn-1".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({"text": "dropped a batch"}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await
        .expect("append a warning");
        tx.commit().await.expect("commit");

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let after = sqlx::query_scalar!(
            r#"select count(*) as "count!" from session_events
               where tenant_id = $1 and event_type in (
                   'message.user', 'message.assistant', 'tool.invoked',
                   'tool.result', 'file.changed', 'command.executed',
                   'memory.asserted'
               )"#,
            fixture.tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count after");
        assert_eq!(
            after, before,
            "an adapter warning must reach the timeline and never the extractor"
        );
    });
}

// ── Quarantine review queue (MEM-2, ADR-0021; ADR-0078 decision 4) ──────────

/// A quarantined event is recorded like any other and is simply **not capture
/// eligible**: the review state lives in its own table, because `session_events`
/// has no UPDATE grant and must not acquire one.
#[test]
fn a_quarantined_event_is_recorded_and_withheld_from_the_pipeline() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, session, event_id) = seed_quarantined(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let stored = sessions::event(&mut *tx, tenant, session, event_id)
            .await
            .expect("read the event")
            .expect("the quarantined event is stored like any other");
        assert!(
            stored.redactions.is_some(),
            "the finding summary rides the row as immutable provenance"
        );
        let eligible = sqlx::query_scalar!(
            r#"select count(*) as "count!"
               from session_events event
               left join session_event_quarantine quarantine
                 on quarantine.tenant_id = event.tenant_id
                and quarantine.event_id = event.id
               where event.tenant_id = $1 and event.id = $2
                 and (quarantine.event_id is null or quarantine.state = 'released')"#,
            tenant.as_uuid(),
            event_id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count eligibility for the withheld event");
        assert_eq!(eligible, 0, "a quarantined event must not be eligible");
    });
}

/// The wrong (or absent) tenant GUC sees zero quarantine rows, and a
/// cross-tenant review resolves nothing — the review queue is content
/// (redacted, but content) and sits squarely under the backstop.
#[test]
fn wrong_tenant_guc_sees_no_quarantine_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, event_id) = seed_quarantined(&db.pool).await;
        let (adversary, _, _) = seed_quarantined(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from session_event_quarantine
               where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count quarantine rows");
        assert_eq!(visible, 0, "quarantine rows leaked across tenants");
        // The store surfaces reach nothing either: get is None, review touches
        // no row — the gateway's uniform 404.
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

/// The app role's write power over the review queue is exactly the one-shot
/// review: findings/provenance columns are not updatable, rows are not
/// deletable outside a declared purge, and a reviewed row cannot be
/// re-reviewed — column grants and the transition trigger, exercised as
/// `synveda_app`.
#[test]
fn quarantine_review_is_one_shot_and_column_bound_for_the_app_role() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, event_id) = seed_quarantined(&db.pool).await;

        // Rewriting findings: no column grant.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let rewrite = sqlx::raw_sql("update session_event_quarantine set findings = '[]'")
            .execute(&mut *tx)
            .await;
        assert!(
            rewrite.is_err(),
            "the app role must not hold UPDATE on findings"
        );
        drop(tx);

        // Deleting outside a declared purge: the trigger raises.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let delete = sqlx::raw_sql("delete from session_event_quarantine")
            .execute(&mut *tx)
            .await;
        assert!(
            delete.is_err(),
            "a quarantine row is retired by retention disposal, never by a handler"
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

        // Even a raw update aimed back at pending trips the transition trigger
        // — the state machine is schema-enforced.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let unreview = sqlx::query!(
            "update session_event_quarantine set state = 'pending', \
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

/// A release makes the exact admitted row eligible, so a frozen batch treats
/// it like an event that never needed review.
#[test]
fn releasing_a_quarantined_event_makes_it_capture_eligible() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, event_id) = seed_quarantined(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        quarantine::review(
            &mut tx,
            tenant,
            event_id,
            quarantine::ReviewDecision::Release,
            "rls-reviewer",
            None,
        )
        .await
        .expect("release under RLS")
        .expect("the quarantined event exists");
        tx.commit().await.expect("commit release");

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let eligible = sqlx::query_scalar!(
            r#"select count(*) as "count!"
               from session_events event
               left join session_event_quarantine quarantine
                 on quarantine.tenant_id = event.tenant_id
                and quarantine.event_id = event.id
               where event.tenant_id = $1 and event.id = $2
                 and (quarantine.event_id is null or quarantine.state = 'released')"#,
            tenant.as_uuid(),
            event_id.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count eligibility after release");
        assert_eq!(
            eligible, 1,
            "a release makes exactly the admitted row eligible"
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
    let artifact_references = fixture_memory_references(proposal, "publish", [4_u8; 32]);
    sqlx::query!(
        "insert into vedaflow_proposals
             (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
              target_channel, commit_hash, sensitivity, title, proposer_id,
              proposer_subject, artifact_references)
         values ($1, $2, $3, $3, 'memory', 'published', $4, 'internal',
                 'rls fixture proposal', $5, 'rls-fixture', $6)",
        tenant.as_uuid(),
        proposal,
        scope.as_uuid(),
        &[4u8; 32][..],
        approver.as_uuid(),
        artifact_references,
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
        let forged_proposal = uuid::Uuid::now_v7();
        let artifact_references = fixture_memory_references(forged_proposal, "publish", [4_u8; 32]);
        let forged = sqlx::query!(
            "insert into vedaflow_proposals
                 (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
                  target_channel, commit_hash, sensitivity, title, proposer_id,
                  proposer_subject, artifact_references)
             values ($1, $2, $3, $3, 'memory', 'published', $4, 'internal',
                     'forged', $5, 'intruder', $6)",
            victim.as_uuid(),
            forged_proposal,
            victim_scope.as_uuid(),
            &[4u8; 32][..],
            IdentityId::new().as_uuid(),
            artifact_references,
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
        let artifact_references = fixture_memory_references(proposal, "publish", [3_u8; 32]);

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            "insert into vedaflow_proposals
                 (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
                  target_channel, commit_hash, sensitivity, title, proposer_id,
                  proposer_subject, artifact_references)
             values ($1, $2, $3, $3, 'memory', 'published', $4, 'restricted',
                     'own proposal', $5, 'own-subject', $6)",
            tenant.as_uuid(),
            proposal,
            scope.as_uuid(),
            &[3u8; 32][..],
            proposer.as_uuid(),
            artifact_references,
        )
        .execute(&mut *tx)
        .await
        .expect("open own proposal");

        // Two distinct approvers, which is what `restricted` takes.
        for (approver, role) in [
            (IdentityId::new(), "curator"),
            (IdentityId::new(), "administrator"),
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

// ── CPR-31: governed, versioned policy relaxations (ADR-0090) ──────────

async fn seed_relaxation(
    pool: &PgPool,
    lifetime: chrono::TimeDelta,
) -> (
    TenantId,
    ScopeId,
    IdentityId,
    String,
    RelaxationId,
    RelaxationVersionId,
    ProposalId,
) {
    let (tenant, target, _, _) = seed_configuration(pool).await;
    let mut tx = pool.begin().await.expect("begin Relaxation fixture");
    let principal = scopes::create(
        &mut tx,
        &new_scope(
            tenant,
            Some(target),
            scope::ScopeKind::Principal,
            "relaxation-subject",
        ),
    )
    .await
    .expect("create Relaxation subject scope");
    let actor = IdentityId::new();
    let subject = "subject-relaxation-subject".to_owned();
    identities::create(
        &mut tx,
        actor,
        tenant,
        Some(&subject),
        IdentityKind::User,
        None,
        Some("Relaxation subject"),
        principal.id,
    )
    .await
    .expect("create Relaxation subject identity");

    let relaxation_id = RelaxationId::new();
    let version_id = RelaxationVersionId::new();
    let proposal_id = ProposalId::new();
    let now = chrono::Utc::now();
    let command = synveda_types::relaxation::RelaxationCommand::Create {
        relaxation_id,
        version_id,
        terms: synveda_types::relaxation::RelaxationTerms {
            subject_identity_id: actor,
            target_scope_id: target,
            action: synveda_types::relaxation::RelaxationAction::KnowledgeRead,
            max_sensitivity: Sensitivity::Internal,
            requested_start_at: now - chrono::TimeDelta::seconds(1),
            requested_end_at: now + lifetime,
            reason: "time-boxed incident investigation".to_owned(),
        },
    };
    let artifact_references = encoded_artifact_reference(
        ArtifactReference::new(
            ArtifactFamily::PolicyRelaxation,
            relaxation_id.to_string(),
            command.kind(),
            version_id.to_string(),
            None,
        )
        .expect("valid Relaxation fixture reference"),
    );
    sqlx::query!(
        "insert into vedaflow_proposals
             (tenant_id, id, target_scope_id, source_scope_id, asset_kind,
              target_channel, commit_hash, sensitivity, title, proposer_id,
              proposer_subject, artifact_references)
         values ($1, $2, $3, $3, 'policy', 'apply', $4, 'internal',
                 'RLS Relaxation fixture', $5, $6, $7)",
        tenant.as_uuid(),
        proposal_id.as_uuid(),
        target.as_uuid(),
        &[4_u8; 32][..],
        actor.as_uuid(),
        &subject,
        artifact_references,
    )
    .execute(&mut *tx)
    .await
    .expect("open Relaxation fixture proposal");
    let payload_hash = blake3::hash(
        synveda_types::json::canonicalise(
            &serde_json::to_value(&command).expect("encode Relaxation command"),
        )
        .to_string()
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    relaxations::insert_change(&mut tx, tenant, proposal_id, &command, &payload_hash)
        .await
        .expect("store Relaxation change");
    let effective = configuration::effective_at_scope(&mut tx, tenant, target)
        .await
        .expect("resolve governed Configuration");
    let applied = relaxations::apply(
        &mut tx,
        tenant,
        proposal_id,
        actor,
        &[],
        &effective,
        &command,
    )
    .await
    .expect("apply Relaxation fixture");
    relaxations::complete_change(&mut tx, tenant, proposal_id, applied)
        .await
        .expect("complete Relaxation fixture");
    tx.commit().await.expect("commit Relaxation fixture");
    (
        tenant,
        target,
        actor,
        subject,
        relaxation_id,
        version_id,
        proposal_id,
    )
}

async fn visible_relaxation_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64) {
    let row = sqlx::query!(
        r#"select
             (select count(*) from policy_relaxations where tenant_id = $1) as "aggregates!",
             (select count(*) from policy_relaxation_versions where tenant_id = $1) as "versions!",
             (select count(*) from policy_relaxation_changes where tenant_id = $1) as "changes!""#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count Relaxation rows");
    (row.aggregates, row.versions, row.changes)
}

#[test]
fn relaxation_rows_are_tenant_isolated_and_versions_are_immutable() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, _, _, relaxation_id, version_id, _) =
            seed_relaxation(&db.pool, chrono::TimeDelta::hours(1)).await;
        let (adversary, _, _, _, _, _, _) =
            seed_relaxation(&db.pool, chrono::TimeDelta::hours(1)).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(visible_relaxation_rows(&mut tx, victim).await, (0, 0, 0));
        assert_eq!(visible_relaxation_rows(&mut tx, adversary).await, (1, 1, 1));
        let reached = sqlx::query!(
            "update policy_relaxations set expiry_recorded_at = now()
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            relaxation_id.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant bookkeeping update is a legal no-op")
        .rows_affected();
        assert_eq!(reached, 0, "another tenant's Relaxation is unreachable");
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(visible_relaxation_rows(&mut tx, victim).await, (0, 0, 0));
        drop(tx);

        let changed = sqlx::query!(
            "update policy_relaxation_versions set reason = 'rewritten after approval'
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            version_id.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(
            changed
                .expect_err("a reviewed version must be immutable")
                .to_string()
                .contains("immutable"),
            "the schema trigger must refuse privileged rewrites too"
        );
        let moved = sqlx::query!(
            "update policy_relaxations set governing_scope_id = gen_random_uuid()
             where tenant_id = $1 and id = $2",
            victim.as_uuid(),
            relaxation_id.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(
            moved
                .expect_err("aggregate identity must be immutable")
                .to_string()
                .contains("identity is immutable")
        );

        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let deleted = sqlx::raw_sql("delete from policy_relaxation_versions")
            .execute(&mut *tx)
            .await;
        assert!(deleted.is_err(), "the app role holds no version DELETE");
    });
}

#[test]
fn relaxation_expiry_is_authoritative_and_chained_once() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _, _, subject, relaxation_id, _, _) =
            seed_relaxation(&db.pool, chrono::TimeDelta::milliseconds(250)).await;
        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        assert!(
            relaxations::active_for_subject(&mut tx, tenant, &subject)
                .await
                .expect("resolve active Relaxations")
                .is_empty(),
            "database time, not the sweep, ends access"
        );
        assert!(
            relaxations::record_expiry(&mut tx, tenant, relaxation_id, 1)
                .await
                .expect("record first expiry")
        );
        assert!(
            !relaxations::record_expiry(&mut tx, tenant, relaxation_id, 1)
                .await
                .expect("the losing sweep is a no-op"),
            "one expiry produces one audit event"
        );
        let restamped = sqlx::query!(
            "update policy_relaxations set expiry_recorded_at = now() + interval '1 day'
             where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            relaxation_id.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(restamped.is_err(), "a recorded expiry cannot move");
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

/// The ingestion plane's DELETE grants (migration 0046), under the same
/// adversarial reading: disposal is per tenant, and the marker cannot outlive
/// the row it points at.
#[test]
fn staging_disposal_is_scoped_to_its_tenant_and_takes_its_markers_with_it() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _, victim_event) = seed_quarantined(&db.pool).await;
        let (adversary, _, _) = seed_quarantined(&db.pool).await;

        // A disposal naming another tenant's staging rows matches nothing.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query_scalar!("select set_config('synveda.retention_purge', 'on', true)")
            .fetch_one(&mut *tx)
            .await
            .expect("declare the disposal");
        let reached = sqlx::query!(
            "delete from session_event_quarantine where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant marker disposal runs")
        .rows_affected();
        assert_eq!(reached, 0);
        let reached = sqlx::query!(
            "delete from session_events where tenant_id = $1",
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
            "delete from session_events where tenant_id = $1 and id = $2",
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

        // Migration 0046's trigger refuses a marker delete outright until the
        // transaction says it is a retention disposal — the same flag the
        // history purge sets, and the reason a handler cannot retire a pending
        // review by accident.
        let mut tx = app_tx(&db.pool, Some(victim)).await;
        let undeclared = sqlx::query!(
            "delete from session_event_quarantine where tenant_id = $1 and event_id = $2",
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
            "delete from session_event_quarantine where tenant_id = $1 and event_id = $2",
            victim.as_uuid(),
            victim_event.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("dispose of the marker");
        let disposed = sqlx::query!(
            "delete from session_events where tenant_id = $1 and id = $2",
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

// ── PRMT-1: the prompt registry's draft table ────────────────────────────────

/// A tenant with one prompt draft at one scope, seeded on the RLS-exempt
/// test connection — the world the backstop must then hide.
///
/// The object it references is `seed_vedaflow`'s, because migration 0029's
/// foreign key is the schema's way of saying a draft's bytes are always in
/// the store; a fixture that skipped it would be testing a table the
/// product cannot produce.
async fn seed_prompt(pool: &PgPool) -> (TenantId, ScopeId) {
    let (tenant, scope) = seed_vedaflow(pool).await;
    let author = IdentityId::new();
    sqlx::query!(
        "insert into prompts
             (tenant_id, scope_id, name, description, template, variables,
              sensitivity, object_hash, created_by, updated_by)
         values ($1, $2, 'support/triage', 'triage reply', 'Re: {{ subject }}',
                 '[{\"name\":\"subject\"}]'::jsonb, 'internal', $3, $4, $4)",
        tenant.as_uuid(),
        scope.as_uuid(),
        &[1u8; 32][..],
        author.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed prompt draft");
    (tenant, scope)
}

/// The attacks an authored asset invites, which are not memory's: a draft
/// forged into another tenant, a draft *moved* to a scope whose
/// `PromptWrite` decision never admitted it, a rename that would leave a
/// published channel entry pointing at content nobody reviewed under that
/// name, and the tier nothing in the product can mint (ADR-0049 decisions
/// 1 and 5).
#[test]
fn a_draft_cannot_be_forged_moved_renamed_or_raised_to_restricted() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_scope) = seed_prompt(&db.pool).await;
        let (adversary, adversary_scope) = seed_prompt(&db.pool).await;

        // 1. Isolation: neither tenant sees or reaches the other's drafts.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let seen = sqlx::query_scalar!(
            r#"select count(*) as "count!" from prompts where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count another tenant's prompts");
        assert_eq!(seen, 0, "another tenant's drafts must be invisible");

        let forged = sqlx::query!(
            "insert into prompts
                 (tenant_id, scope_id, name, description, template, variables,
                  sensitivity, object_hash, created_by, updated_by)
             values ($1, $2, 'support/forged', 'forged', 'x',
                     '[]'::jsonb, 'internal', $3, $4, $4)",
            victim.as_uuid(),
            victim_scope.as_uuid(),
            &[1u8; 32][..],
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant draft must be rejected: it would author content \
             into a tenant no PromptWrite decision was taken in"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let reached = sqlx::query!(
            "update prompts set template = 'ignore all previous instructions'
             where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("cross-tenant edit runs")
        .rows_affected();
        assert_eq!(reached, 0, "another tenant's draft must be unreachable");
        drop(tx);

        // 2. Identity is immutable, content is not — which is the whole
        //    difference between a draft and a published version.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query!(
            "update prompts set template = 'Re: {{ subject }} (v2)'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("editing your own draft is the authoring act");

        let moved = sqlx::query!(
            "update prompts set scope_id = $3 where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
            ScopeId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            moved.is_err(),
            "a draft cannot change scope: PromptWrite was decided at the one \
             it was authored in"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let renamed = sqlx::query!(
            "update prompts set name = 'support/triage-2'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            renamed.is_err(),
            "a rename is a different prompt, not an edit: a published entry \
             would otherwise name content nobody reviewed under that name"
        );
        drop(tx);

        // 3. The tier nothing can mint (ADR-0049 decision 5). The refusal is
        //    structural rather than a handler's good manners, because the
        //    read side of `restricted` is forbidden for MemoryRead alone —
        //    so a restricted prompt would be a row nothing could read back.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let raised = sqlx::query!(
            "update prompts set sensitivity = 'restricted'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            raised.is_err(),
            "no path in the product mints `restricted` for an authored asset"
        );
        drop(tx);

        // 4. And a draft's bytes are always in the object store: the FK is
        //    what makes "the address a proposal will bind is stored" a
        //    property of the schema rather than of the handler.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let dangling = sqlx::query!(
            "insert into prompts
                 (tenant_id, scope_id, name, description, template, variables,
                  sensitivity, object_hash, created_by, updated_by)
             values ($1, $2, 'support/dangling', 'd', 'x',
                     '[]'::jsonb, 'internal', $3, $4, $4)",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
            &[9u8; 32][..],
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            dangling.is_err(),
            "a draft naming bytes the store does not hold must be rejected"
        );
        drop(tx);

        // 5. The app role holds no DELETE (ADR-0049): retracting a published
        //    prompt is FLOW-7's rewind, and replacing a draft is an
        //    overwrite — neither needs the statement that could erase who
        //    authored what.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let deleted = sqlx::query!(
            "delete from prompts where tenant_id = $1",
            adversary.as_uuid()
        )
        .execute(&mut *tx)
        .await;
        assert!(
            deleted.is_err(),
            "the app role must hold no DELETE on prompts"
        );
    });
}

// ── PRMT-2: the context-pack registry and its chunk mapping ─────────────────

/// A tenant with one pack, one document, one pinned record, and the chunk
/// row that ties the record to the document address it was cut from.
///
/// The object is `seed_vedaflow`'s for migration 0029's reason, and the
/// record is a real `records::insert` — a fixture that faked either would be
/// testing a table the product cannot produce.
async fn seed_context_pack(pool: &PgPool) -> (TenantId, ScopeId, RecordId) {
    let (tenant, scope) = seed_vedaflow(pool).await;
    let author = IdentityId::new();
    sqlx::query!(
        "insert into context_packs
             (tenant_id, scope_id, name, description, created_by, updated_by)
         values ($1, $2, 'payments', 'payment conventions', $3, $3)",
        tenant.as_uuid(),
        scope.as_uuid(),
        author.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed pack");
    sqlx::query!(
        "insert into context_pack_documents
             (tenant_id, scope_id, pack_name, document_name, title, sensitivity,
              object_hash, chunks, created_by, updated_by)
         values ($1, $2, 'payments', 'runbooks/refunds.md', 'Refunds runbook',
                 'internal', $3, 1, $4, $4)",
        tenant.as_uuid(),
        scope.as_uuid(),
        &[1u8; 32][..],
        author.as_uuid(),
    )
    .execute(pool)
    .await
    .expect("seed document");

    let record = RecordId::new();
    let mut chunk = state("Escalate refunds over £500.");
    chunk.scope_id = scope;
    // A pack chunk is a pinned record — that is the decision the whole
    // feature hangs on (ADR-0050 decision 2), and it is also what makes the
    // FK below safe: the retention sweep's own SQL exempts `pinned`.
    chunk.kind = RecordKind::Pinned;
    insert(pool, record, tenant, &chunk)
        .await
        .expect("seed chunk record");
    sqlx::query!(
        "insert into context_pack_chunks
             (tenant_id, record_id, scope_id, pack_name, document_name, title,
              document_hash, ordinal, heading)
         values ($1, $2, $3, 'payments', 'runbooks/refunds.md', 'Refunds runbook',
                 $4, 0, 'Refunds')",
        tenant.as_uuid(),
        record.as_uuid(),
        scope.as_uuid(),
        &[1u8; 32][..],
    )
    .execute(pool)
    .await
    .expect("seed chunk mapping");
    (tenant, scope, record)
}

/// The attacks a pack invites, which are the prompt registry's plus one
/// that is entirely new: **the chunk mapping decides what composes as
/// published**, so a forged or edited chunk row is a way to put unreviewed
/// text into somebody's session under a reviewed document's name
/// (ADR-0050 decision 3).
#[test]
fn a_pack_cannot_be_forged_moved_renamed_raised_or_have_its_chunks_relabelled() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_scope, victim_record) = seed_context_pack(&db.pool).await;
        let (adversary, adversary_scope, _) = seed_context_pack(&db.pool).await;

        // 1. Isolation across all three tables.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        for table in [
            "context_packs",
            "context_pack_documents",
            "context_pack_chunks",
        ] {
            let seen: i64 = sqlx::query_scalar(&format!(
                "select count(*) from {table} where tenant_id = $1"
            ))
            .bind(victim.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count another tenant's rows");
            assert_eq!(seen, 0, "another tenant's {table} rows must be invisible");
        }

        let forged = sqlx::query!(
            "insert into context_packs
                 (tenant_id, scope_id, name, description, created_by, updated_by)
             values ($1, $2, 'forged', 'forged', $3, $3)",
            victim.as_uuid(),
            victim_scope.as_uuid(),
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a forged-tenant pack must be rejected: it would author content \
             into a tenant no ContextPackWrite decision was taken in"
        );
        drop(tx);

        // 2. Identity is immutable, content is not.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query!(
            "update context_pack_documents set title = 'Refunds runbook (v2)'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("editing your own document is the authoring act");

        let moved = sqlx::query!(
            "update context_packs set scope_id = $3 where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
            ScopeId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            moved.is_err(),
            "a pack cannot change scope: ContextPackWrite was decided at the \
             one it was authored in"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let renamed = sqlx::query!(
            "update context_pack_documents set document_name = 'runbooks/other.md'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            renamed.is_err(),
            "a rename is a different document, not an edit: a published entry \
             would otherwise name content nobody reviewed under that name"
        );
        drop(tx);

        // 3. The tier nothing can mint (ADR-0050 decision 12).
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let raised = sqlx::query!(
            "update context_pack_documents set sensitivity = 'restricted'
             where tenant_id = $1 and scope_id = $2",
            adversary.as_uuid(),
            adversary_scope.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            raised.is_err(),
            "no path in the product mints `restricted` for an authored asset"
        );
        drop(tx);

        // 4. **The new attack.** A chunk row's `document_hash` is what
        //    decides whether its record composes as published, so relabelling
        //    one would move unreviewed text under a reviewed document's
        //    address. There is no UPDATE grant and a trigger behind it, which
        //    is one step stricter than the draft tables because nothing about
        //    a chunk mapping can legitimately change.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let relabelled = sqlx::query!(
            "update context_pack_chunks set document_hash = $2 where tenant_id = $1",
            adversary.as_uuid(),
            &[3u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            relabelled.is_err(),
            "a chunk cannot be re-pointed at another document version: that is \
             how unreviewed text would compose under a reviewed address"
        );
        drop(tx);

        // 5. A chunk pointed at another tenant's record composes nothing.
        //    `records_pk` is the id alone, so the FK does not carry a tenant
        //    and the insert is accepted — the mapping is a claim, and what
        //    makes the claim worthless is that composition resolves it
        //    against `records` inside this tenant's transaction, where the
        //    victim's row is invisible (ADR-0009). The chunk is a name for a
        //    record, never a capability over one; this is that sentence
        //    tested rather than asserted.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        sqlx::query!(
            "insert into context_pack_chunks
                 (tenant_id, record_id, scope_id, pack_name, document_name, title,
                  document_hash, ordinal)
             values ($1, $2, $3, 'payments', 'runbooks/forged.md', 'f', $4, 7)",
            adversary.as_uuid(),
            victim_record.as_uuid(),
            adversary_scope.as_uuid(),
            &[1u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("the mapping table cannot check another tenant's records");
        assert!(
            records::current(&mut *tx, victim_record)
                .await
                .expect("resolve the pointed-at record")
                .is_none(),
            "the record a forged chunk names must stay unreadable, so the \
             chunk resolves to nothing"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let dangling = sqlx::query!(
            "insert into context_pack_chunks
                 (tenant_id, record_id, scope_id, pack_name, document_name, title,
                  document_hash, ordinal)
             values ($1, $2, $3, 'payments', 'runbooks/dangling.md', 'd', $4, 9)",
            adversary.as_uuid(),
            RecordId::new().as_uuid(),
            adversary_scope.as_uuid(),
            &[9u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            dangling.is_err(),
            "a chunk naming a record or an address the store does not hold \
             must be rejected"
        );
        drop(tx);

        // 6. The app role holds no DELETE on any of the three (ADR-0050
        //    decision 14): retracting a published pack is FLOW-7's rewind,
        //    and replacing a draft is an overwrite.
        for table in [
            "context_packs",
            "context_pack_documents",
            "context_pack_chunks",
        ] {
            let mut tx = app_tx(&db.pool, Some(adversary)).await;
            let deleted = sqlx::query(&format!("delete from {table} where tenant_id = $1"))
                .bind(adversary.as_uuid())
                .execute(&mut *tx)
                .await;
            assert!(
                deleted.is_err(),
                "the app role must hold no DELETE on {table}"
            );
        }
    });
}

// ── CPR-23: immutable Skill versions and bindings ──────────────────────────

/// Seed one stable Skill, one immutable version with two content-addressed
/// files, and one principal-scope binding. The fixture uses privileged setup;
/// every assertion below runs as the forced-RLS application role.
async fn seed_skill(pool: &PgPool) -> (TenantId, ScopeId) {
    let (tenant, _) = seed_vedaflow(pool).await;
    let author = IdentityId::new();
    let mut tx = pool.begin().await.expect("begin Skill fixture");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure tenant root");
    let principal = scopes::create(
        &mut tx,
        &new_scope(
            tenant,
            Some(root.id),
            scope::ScopeKind::Principal,
            "skill-owner",
        ),
    )
    .await
    .expect("create principal binding scope");
    let skill_id = uuid::Uuid::now_v7();
    let version_id = uuid::Uuid::now_v7();
    let binding_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "insert into skills
             (id, tenant_id, governing_scope_id, name, current_version_id,
              created_by, updated_by)
         values ($1, $2, $3, 'code-review', $4, $5, $5)",
        skill_id,
        tenant.as_uuid(),
        principal.id.as_uuid(),
        version_id,
        author.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("seed Skill aggregate");
    sqlx::query!(
        r#"insert into skill_versions
             (id, tenant_id, skill_id, ordinal, bundle_digest, sensitivity,
              manifest, source_kind, provenance, scan_report,
              scan_ruleset_version, quality_score, rubric_version, created_by)
         values ($1, $2, $3, 1, $4, 'internal',
                 '{"name":"code-review","description":"Reviews a diff."}'::jsonb,
                 'authored', '{"source":"rls-fixture"}'::jsonb,
                 '{"findings":[]}'::jsonb, 1, 80, 1, $5)"#,
        version_id,
        tenant.as_uuid(),
        skill_id,
        &[7u8; 32][..],
        author.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("seed immutable Skill version");
    for path in ["SKILL.md", "scripts/check.py"] {
        sqlx::query!(
            "insert into skill_version_files
                 (tenant_id, version_id, path, object_hash, chars)
             values ($1, $2, $3, $4, 3)",
            tenant.as_uuid(),
            version_id,
            path,
            &[1u8; 32][..],
        )
        .execute(&mut *tx)
        .await
        .expect("seed immutable Skill file");
    }
    sqlx::query!(
        "insert into skill_bindings
             (id, tenant_id, scope_id, skill_id, pinned_version_id, enabled,
              created_by, updated_by)
         values ($1, $2, $3, $4, $5, true, $6, $6)",
        binding_id,
        tenant.as_uuid(),
        principal.id.as_uuid(),
        skill_id,
        version_id,
        author.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("seed Skill binding");
    tx.commit().await.expect("commit Skill fixture");
    (tenant, principal.id)
}

/// Forced RLS isolates every active Skill table, and transition guards make
/// versions/files immutable while aggregate and binding identity cannot move.
#[test]
fn versioned_skills_are_tenant_isolated_and_immutable() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_skill(&db.pool).await;
        let (adversary, adversary_scope) = seed_skill(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        for table in [
            "skills",
            "skill_versions",
            "skill_version_files",
            "skill_bindings",
            "skill_usage_events",
            "skill_test_runs",
        ] {
            let seen: i64 = sqlx::query_scalar(&format!(
                "select count(*) from {table} where tenant_id = $1"
            ))
            .bind(victim.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("count another tenant's Skill rows");
            assert_eq!(seen, 0, "another tenant's {table} rows must be invisible");
        }
        let forged = sqlx::query!(
            "insert into skills
                 (id, tenant_id, governing_scope_id, name, current_version_id,
                  created_by, updated_by)
             values ($1, $2, $3, 'forged', $4, $5, $5)",
            uuid::Uuid::now_v7(),
            victim.as_uuid(),
            adversary_scope.as_uuid(),
            uuid::Uuid::now_v7(),
            IdentityId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(forged.is_err(), "a forged-tenant Skill must be rejected");
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let renamed = sqlx::query!(
            "update skills set name = 'renamed' where tenant_id = $1",
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(renamed.is_err(), "Skill aggregate identity is immutable");
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let rewritten = sqlx::query!(
            "update skill_versions set manifest = '{}'::jsonb where tenant_id = $1",
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(rewritten.is_err(), "Skill versions are immutable");
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let repathed = sqlx::query!(
            "update skill_version_files set path = 'scripts/other.py'
             where tenant_id = $1 and path = 'scripts/check.py'",
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(repathed.is_err(), "version file paths are immutable");
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let moved = sqlx::query!(
            "update skill_bindings set scope_id = $2 where tenant_id = $1",
            adversary.as_uuid(),
            ScopeId::new().as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(moved.is_err(), "a binding cannot move to another scope");
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let dangling = sqlx::query!(
            "insert into skill_version_files
                 (tenant_id, version_id, path, object_hash, chars)
             select tenant_id, id, 'references/missing.md', $2, 3
               from skill_versions where tenant_id = $1 limit 1",
            adversary.as_uuid(),
            &[9u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            dangling.is_err(),
            "a version file must name content-addressed bytes the tenant holds"
        );
        drop(tx);

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let deleted = sqlx::query!(
            "delete from skill_versions where tenant_id = $1",
            adversary.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(deleted.is_err(), "immutable versions cannot be deleted");
    });
}

// ── AUTH-4: the directory mirror and the provisioning credential ────────────

/// Seeds a tenant with a directory user, a shared group membership and a
/// provisioning credential.
async fn seed_directory(pool: &PgPool) -> (TenantId, synveda_types::ScimCredentialId) {
    let tenant = seed_identity(pool).await;
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let user = synveda_store::directory::create_user(
        &mut *tx,
        synveda_types::DirectoryUserId::new(),
        tenant,
        &synveda_store::directory::UserAttributes {
            directory_source: "scim".to_owned(),
            external_id: Some("ext-1".to_owned()),
            user_name: "person@example.test".to_owned(),
            active: true,
            display_name: None,
            given_name: None,
            family_name: None,
            work_email: None,
        },
    )
    .await
    .expect("create mirror user");
    let identity = identities::by_subject(&mut *tx, tenant, "alice")
        .await
        .expect("read identity")
        .expect("fixture identity");
    synveda_store::directory::link_identity(&mut *tx, tenant, "scim", user.id, identity.id)
        .await
        .expect("link directory user");
    access::sync_directory_group(
        &mut tx,
        GroupId::new(),
        tenant,
        "scim",
        "group-1",
        None,
        "synveda-eng-core",
        "synveda-eng-core",
        &[identity.id],
    )
    .await
    .expect("project shared group");
    let credential_id = synveda_types::ScimCredentialId::new();
    // A distinct hash per tenant, because `scim_credentials_hash_unique` is
    // **global**: one presented token identifies at most one credential
    // anywhere, so a hash can never be ambiguous across tenants. The
    // fixture has to respect that to seed two tenants at all.
    let hash: Vec<u8> = credential_id.as_uuid().as_bytes().repeat(2);
    synveda_store::directory::issue_credential(
        &mut *tx,
        credential_id,
        tenant,
        &hash,
        "rls-suite",
        chrono::Utc::now() + chrono::Duration::days(1),
        "operator",
    )
    .await
    .expect("issue credential");
    tx.commit().await.expect("commit directory fixture");
    (tenant, credential_id)
}

async fn visible_directory_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64, i64) {
    let users = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scim_users where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count scim_users");
    let groups = sqlx::query_scalar!(
        r#"select count(*) as "count!" from groups
            where tenant_id = $1 and source = 'directory'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count directory groups");
    let members = sqlx::query_scalar!(
        r#"select count(*) as "count!" from group_members
            where tenant_id = $1 and source = 'directory'"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count directory memberships");
    let credentials = sqlx::query_scalar!(
        r#"select count(*) as "count!" from scim_credentials where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count scim_credentials");
    (users, groups, members, credentials)
}

/// The directory plane is tenant-confidential in both directions.
///
/// Who a tenant employs, which groups they are in, and **which credentials
/// can provision them** are all things another tenant must not be able to
/// count, let alone read. The credential table matters most: it is the
/// lookup a request authenticates against, so a policy that leaked across
/// tenants would make the token's own tenant claim decorative.
#[test]
fn wrong_tenant_guc_sees_no_directory_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, victim_credential) = seed_directory(&db.pool).await;
        let (adversary, _) = seed_directory(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        assert_eq!(
            visible_directory_rows(&mut tx, victim).await,
            (0, 0, 0, 0),
            "another tenant's directory plane is invisible"
        );
        assert_eq!(
            visible_directory_rows(&mut tx, adversary).await,
            (1, 1, 1, 1),
            "its own is not"
        );
        tx.rollback().await.expect("rollback");

        // The victim's own credential, looked up by its exact hash from
        // inside the adversary's tenant, is not there. This is the property
        // the token's tenant prefix rests on: naming a tenant selects whose
        // rows the hash is checked against, and naming the wrong one finds
        // nothing rather than somebody else's key.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let hash: Vec<u8> = victim_credential.as_uuid().as_bytes().repeat(2);
        let found = sqlx::query_scalar!(
            r#"select count(*) as "count!" from scim_credentials where token_hash = $1"#,
            &hash,
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count by hash");
        assert_eq!(found, 0, "another tenant's credential is not reachable");
        tx.rollback().await.expect("rollback");
    });
}

/// A forged write into another tenant's directory plane is refused by the
/// `with check` half of every policy.
#[test]
fn cross_tenant_directory_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_directory(&db.pool).await;
        let (adversary, _) = seed_directory(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            r#"
            insert into scim_users (id, tenant_id, directory_source, user_name)
            values ($1, $2, 'scim', 'forged@example.test')
            "#,
            uuid::Uuid::now_v7(),
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a mirror row forged into another tenant must be refused"
        );
        tx.rollback().await.expect("rollback");

        // And the one that matters most: a credential minted into somebody
        // else's tenant would be a key to their directory plane.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            r#"
            insert into scim_credentials
                (id, tenant_id, token_hash, label, expires_at, created_by)
            values ($1, $2, $3, 'forged', now() + interval '1 day', 'attacker')
            "#,
            uuid::Uuid::now_v7(),
            victim.as_uuid(),
            &[9u8; 32][..],
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a credential forged into another tenant must be refused"
        );
        tx.rollback().await.expect("rollback");
    });
}

/// The grants migration 0036 chose, asserted as behaviour.
///
/// The mirror is fully mutable because the directory authors it; the
/// credential is append-and-stamp because **which credential sealed which
/// identity has to stay answerable after the credential is gone**. A
/// `delete` on `scim_credentials` is the one operation that would take that
/// answer away, so the app role does not hold it.
#[test]
fn a_credential_can_be_revoked_but_never_erased() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, credential) = seed_directory(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let revoked = synveda_store::directory::revoke_credential(&mut *tx, tenant, credential)
            .await
            .expect("revoke");
        assert!(revoked, "revocation is an update the app role holds");
        tx.commit().await.expect("commit revoke");

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let deleted = sqlx::query!(
            "delete from scim_credentials where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            credential.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            deleted.is_err(),
            "no DELETE grant: a credential's history is the point"
        );
        tx.rollback().await.expect("rollback");
    });
}

// ── AUTH-5: the pull sync's own state (migration 0037, ADR-0060) ─────────────

/// A tenant with a directory mirror and one sync-state row.
///
/// The insert is direct SQL because AUTH-5's store module does not exist
/// yet: migration 0037 lands the schema and its invariants ahead of the loop
/// that will write them, so until that loop arrives this suite is the only
/// thing holding them.
async fn seed_sync_state(pool: &PgPool) -> (TenantId, synveda_types::DirectoryUserId) {
    let (tenant, _) = seed_directory(pool).await;
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let user = synveda_store::directory::user_by_user_name(
        &mut *tx,
        tenant,
        "scim",
        "person@example.test",
    )
    .await
    .expect("read directory user")
    .expect("the directory fixture's user");
    sqlx::query!(
        r#"
        insert into directory_sync_state
            (tenant_id, connector, passes_completed, last_pass_at, last_complete_pass_at)
        values ($1, 'entra', 3, now(), now())
        "#,
        tenant.as_uuid(),
    )
    .execute(&mut *tx)
    .await
    .expect("seed sync state");
    tx.commit().await.expect("commit sync fixture");
    (tenant, user.id)
}

/// The pull sync's state is tenant-confidential, and for a sharper reason
/// than "it is a table with a `tenant_id`".
///
/// `passes_completed` and `last_complete_pass_at` are the completeness proof
/// ADR-0060 decision 3.1 rests on — they are what says a pass is entitled to
/// conclude that somebody has left. Read across tenants they disclose a
/// customer's directory health and their headcount churn; the write half is
/// the next test, and it is worse.
#[test]
fn wrong_tenant_guc_sees_no_directory_sync_state() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_sync_state(&db.pool).await;
        let (adversary, _) = seed_sync_state(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let visible = sqlx::query_scalar!(
            r#"select count(*) as "count!" from directory_sync_state where tenant_id = $1"#,
            victim.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count directory_sync_state");
        assert_eq!(visible, 0, "another tenant's sync state is invisible");

        let own = sqlx::query_scalar!(
            r#"select count(*) as "count!" from directory_sync_state where tenant_id = $1"#,
            adversary.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count own directory_sync_state");
        assert_eq!(own, 1, "its own is not");
        tx.rollback().await.expect("rollback");
    });
}

/// A forged sync-state row is ADR-0060 decision 3 defeated from outside the
/// tenant it protects.
///
/// `passes_completed` counts passes that **completed**, and nothing else may
/// advance it. An adversary who could insert or update one in somebody
/// else's tenant would be handing their next pass a completeness proof it
/// never earned — and a pass that believes it enumerated the whole directory
/// is precisely the pass that seals everyone it did not see.
#[test]
fn cross_tenant_sync_state_write_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (victim, _) = seed_sync_state(&db.pool).await;
        let (adversary, _) = seed_sync_state(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let forged = sqlx::query!(
            r#"
            insert into directory_sync_state (tenant_id, connector, passes_completed)
            values ($1, 'forged', 99)
            "#,
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a sync state forged into another tenant must be refused"
        );
        tx.rollback().await.expect("rollback");

        // The update half fails differently and the difference is the point:
        // the `using` clause hides the row rather than refusing the
        // statement, so an attacker learns nothing about whether it existed.
        let mut tx = app_tx(&db.pool, Some(adversary)).await;
        let touched = sqlx::query!(
            "update directory_sync_state set passes_completed = 99 where tenant_id = $1",
            victim.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("an update over invisible rows is not an error");
        assert_eq!(
            touched.rows_affected(),
            0,
            "another tenant's completeness proof is not advanceable"
        );
        tx.rollback().await.expect("rollback");
    });
}

/// The grant migration 0037 withheld, asserted as behaviour.
///
/// "Why has nobody been sealed for three days" is answerable only from a row
/// that outlives the passes it describes. A loop that could delete its own
/// state could also delete the evidence that it had stopped working — the
/// silence would look exactly like a directory in which nobody had left. So
/// the app role advances this row and never removes it, which is
/// `scim_credentials`' rule (migration 0036) reached by a different route.
#[test]
fn sync_state_advances_but_is_never_erased() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_sync_state(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let advanced = sqlx::query!(
            r#"
            update directory_sync_state
               set passes_completed = passes_completed + 1,
                   last_complete_pass_at = now(),
                   updated_at = now()
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("advancing a pass count is an update the app role holds");
        assert_eq!(advanced.rows_affected(), 1);
        tx.commit().await.expect("commit advance");

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let deleted = sqlx::query!(
            "delete from directory_sync_state where tenant_id = $1",
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            deleted.is_err(),
            "no DELETE grant: a sync state that can vanish takes the record \
             of a stalled connector with it"
        );
        tx.rollback().await.expect("rollback");
    });
}

/// An absence hypothesis cannot be half-reset.
///
/// `missing_passes` is the condition and `missing_since` is the record
/// (ADR-0060 decision 3.2). A write that cleared one and left the other
/// standing would leave somebody who is present by the counter and gone
/// since Tuesday by the timestamp — and because the counter is what seals,
/// the disagreement would surface much later as a seal nobody can account
/// for, or as a leaver nobody ever seals. The constraint is what makes "the
/// two columns say one thing" a property of the schema rather than a
/// convention every future writer is trusted to keep.
#[test]
fn an_absence_hypothesis_cannot_be_half_reset() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, user) = seed_sync_state(&db.pool).await;

        // A complete pass that did not list this person: both columns move.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            r#"
            update scim_users
               set missing_passes = 1, missing_since = now()
             where tenant_id = $1 and id = $2
            "#,
            tenant.as_uuid(),
            user.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("an absence is recordable");
        tx.commit().await.expect("commit absence");

        // Clearing the counter alone says present-and-missing at once.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let half = sqlx::query!(
            "update scim_users set missing_passes = 0 where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            user.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            half.is_err(),
            "clearing the condition without the record must be refused"
        );
        tx.rollback().await.expect("rollback");

        // And clearing the record alone says the same thing the other way.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let half = sqlx::query!(
            "update scim_users set missing_since = null where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            user.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            half.is_err(),
            "clearing the record without the condition must be refused"
        );
        tx.rollback().await.expect("rollback");

        // The directory listing them again clears both, which is the only
        // shape a reset comes in.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            r#"
            update scim_users
               set missing_passes = 0, missing_since = null
             where tenant_id = $1 and id = $2
            "#,
            tenant.as_uuid(),
            user.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("a person the directory lists again is present");
        tx.commit().await.expect("commit reset");

        // A negative count is not a smaller hypothesis; it is a broken one.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let negative = sqlx::query!(
            "update scim_users set missing_passes = -1 where tenant_id = $1 and id = $2",
            tenant.as_uuid(),
            user.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(negative.is_err(), "a negative absence count is refused");
        tx.rollback().await.expect("rollback");
    });
}

/// A pass that did not complete cannot claim to have completed, and a
/// breaker trip that sealed nobody cannot claim to have tripped.
///
/// Both constraints guard a value that decides whether anybody is sealed. A
/// `last_complete_pass_at` with no completed pass behind it is an incomplete
/// pass wearing the completeness proof of decision 3.1; a breaker trip
/// recorded without the count it refused is a refusal an operator cannot
/// size, which is the one thing decision 3.3 exists to tell them.
#[test]
fn an_incomplete_pass_cannot_claim_the_completeness_proof() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_directory(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let forged = sqlx::query!(
            r#"
            insert into directory_sync_state
                (tenant_id, connector, passes_completed, last_complete_pass_at)
            values ($1, 'entra', 0, now())
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a completed-pass timestamp with no completed pass behind it is refused"
        );
        tx.rollback().await.expect("rollback");

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let unsized_trip = sqlx::query!(
            r#"
            insert into directory_sync_state
                (tenant_id, connector, passes_completed, breaker_tripped_at)
            values ($1, 'entra', 1, now())
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            unsized_trip.is_err(),
            "a breaker trip without the count it refused is refused"
        );
        tx.rollback().await.expect("rollback");

        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let countless = sqlx::query!(
            r#"
            insert into directory_sync_state
                (tenant_id, connector, passes_completed, breaker_would_have_sealed)
            values ($1, 'entra', 1, 5)
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            countless.is_err(),
            "a count with no trip behind it is refused"
        );
        tx.rollback().await.expect("rollback");

        // A trip that would have sealed nobody is not a trip; it is a pass
        // in which nobody had left, and recording it as a refusal would put
        // a breaker event in front of an operator on a quiet week.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let empty_trip = sqlx::query!(
            r#"
            insert into directory_sync_state
                (tenant_id, connector, passes_completed,
                 breaker_tripped_at, breaker_would_have_sealed)
            values ($1, 'entra', 1, now(), 0)
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            empty_trip.is_err(),
            "a trip that would have sealed nobody is refused"
        );
        tx.rollback().await.expect("rollback");
    });
}

/// A seal authorisation arrives whole or not at all.
///
/// ADR-0060 decision 10: releasing a breaker trip is reasoned, time-boxed,
/// bounded by a ceiling and signed by a named human. A row able to carry
/// three of those five would be an authorisation with no ceiling, or with
/// nobody's name against it — assembled by whichever writer got halfway, at
/// the one moment it matters, which is mid-incident with a mass departure on
/// the wire and nobody reading carefully. This is where "whole or not at
/// all" stops depending on the writer.
///
/// The two malformed-but-complete cases go in with all five columns set, so
/// each is refused by the constraint written for it rather than swallowed by
/// the pair check.
#[test]
fn a_seal_authorisation_arrives_whole_or_not_at_all() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant, _) = seed_sync_state(&db.pool).await;

        // Whole: a future window, a positive ceiling, a name and a reason.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            r#"
            update directory_sync_state
               set seal_authorised_at = now(),
                   seal_authorised_until = now() + interval '2 hours',
                   seal_authorised_ceiling = 300,
                   seal_authorised_by = 'alice@example.test',
                   seal_authorised_reason = 'Q3 restructure, ticket OPS-1123'
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("a complete authorisation is storable");
        tx.commit().await.expect("commit authorisation");

        // Spending it clears all five, which is the only other legal shape.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        sqlx::query!(
            r#"
            update directory_sync_state
               set seal_authorised_at = null, seal_authorised_until = null,
                   seal_authorised_ceiling = null, seal_authorised_by = null,
                   seal_authorised_reason = null
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await
        .expect("a spent authorisation clears whole");
        tx.commit().await.expect("commit spend");

        // A ceiling on its own permits a number and names nobody for it.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let partial = sqlx::query!(
            "update directory_sync_state set seal_authorised_ceiling = 300 where tenant_id = $1",
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            partial.is_err(),
            "a ceiling with no grantor, reason or window is refused"
        );
        tx.rollback().await.expect("rollback");

        // A ceiling of zero permits no seals, so it is not permission —
        // and a pass that consulted it would clear it having done nothing.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let empty = sqlx::query!(
            r#"
            update directory_sync_state
               set seal_authorised_at = now(),
                   seal_authorised_until = now() + interval '2 hours',
                   seal_authorised_ceiling = 0,
                   seal_authorised_by = 'alice@example.test',
                   seal_authorised_reason = 'authorising nothing'
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(empty.is_err(), "a ceiling of zero is not an authorisation");
        tx.rollback().await.expect("rollback");

        // An authorisation that expired before it was granted is refused
        // rather than left sitting there looking like permission.
        let mut tx = app_tx(&db.pool, Some(tenant)).await;
        let expired = sqlx::query!(
            r#"
            update directory_sync_state
               set seal_authorised_at = now(),
                   seal_authorised_until = now() - interval '1 minute',
                   seal_authorised_ceiling = 300,
                   seal_authorised_by = 'alice@example.test',
                   seal_authorised_reason = 'window closed before it opened'
             where tenant_id = $1
            "#,
            tenant.as_uuid(),
        )
        .execute(&mut *tx)
        .await;
        assert!(
            expired.is_err(),
            "an authorisation expiring before it is granted is refused"
        );
        tx.rollback().await.expect("rollback");
    });
}

// ── The session ledger (CPR-10, ADR-0076) ───────────────────────────────────

/// What [`seed_session`] built.
struct SessionFixture {
    tenant: TenantId,
    workspace: WorkspaceId,
    scope: ScopeId,
    session: SessionId,
    run: ContextRunId,
}

/// Admits a tenant with a workspace, one session in it, two events and one
/// context run. Runs on the (RLS-exempt) test connection.
async fn seed_session(pool: &PgPool) -> SessionFixture {
    let tenant = TenantId::new();
    let slug = format!("rlss-{}", tenant.as_uuid().simple());
    tenants::create(
        pool,
        tenant,
        &slug,
        "RLS session fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin transaction");
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "runs".to_owned(),
            display_name: "Runs".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace");
    let session = sessions::create(
        &mut tx,
        &sessions::NewSession {
            id: SessionId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            project_id: None,
            principal_id: "rls-agent".to_owned(),
            client_name: "claude-code".to_owned(),
            client_version: None,
            client_installation_id: None,
            external_session_id: Some("harness-1".to_owned()),
            agent_name: None,
            model_name: None,
            repository_id: None,
            branch: None,
            task_summary: None,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("open session");
    sessions::append_events(
        &mut tx,
        tenant,
        session.id,
        &[
            sessions::NewSessionEvent {
                event_type: SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: "e1".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({"text": "a secret plan"}),
                redactions: None,
                quarantine: false,
            },
            sessions::NewSessionEvent {
                event_type: SessionEventType::ToolInvoked,
                event_schema_version: 1,
                client_event_id: "e2".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({"tool": "grep"}),
                redactions: None,
                quarantine: false,
            },
        ],
    )
    .await
    .expect("append events");
    let run = sessions::record_context_run(
        &mut tx,
        tenant,
        &sessions::NewContextRun {
            id: ContextRunId::new(),
            skills: serde_json::json!([]),
            session_id: session.id,
            workspace_id: workspace.id,
            project_id: None,
            scope_id: workspace.scope_id,
            principal_id: "rls-agent".to_owned(),
            configuration_version_id: None,
            configuration_hash: synveda_types::configuration::ConfigurationDocument::fail_safe()
                .content_hash()
                .expect("hash fail-safe Configuration"),
            query: Some("what do we know".to_owned()),
            query_hash: Some(blake3::hash(b"what do we know").to_hex().to_string()),
            rendered: "composed material".to_owned(),
            block_hash: blake3::hash(b"composed material").to_hex().to_string(),
            tokens: 4,
            budget_tokens: 1500,
            requested_budget_tokens: None,
            entry_count: 0,
            candidate_count: 0,
            selection_count: 0,
            degraded: Vec::new(),
            as_of: chrono::Utc::now(),
            retrieval_version: "rls-test-v1".to_owned(),
            embedding_model: None,
            index_version: "rls-test-index-v1".to_owned(),
            graph_version: None,
            trace_retention: TraceRetentionMode::Full,
            completion_status: ContextCompletionStatus::Completed,
            policy_exclusion: false,
        },
    )
    .await
    .expect("record context run");

    let item_id = KnowledgeItemId::new();
    let revision_id = KnowledgeRevisionId::new();
    let source = knowledge::create_source(
        &mut tx,
        &knowledge::NewKnowledgeSource {
            id: KnowledgeSourceId::new(),
            tenant_id: tenant,
            scope_id: workspace.scope_id,
            source_type: KnowledgeSourceType::Manual,
            session_event_id: None,
            locator: None,
            source_revision: None,
            content_hash: None,
            metadata: serde_json::json!({"fixture": "CPR-20"}),
            created_by: Some("rls-agent".to_owned()),
        },
    )
    .await
    .expect("create provenance fixture");
    let content = KnowledgeRevisionContent {
        title: "RLS context fixture".to_owned(),
        body_markdown: "tenant-private planner evidence".to_owned(),
        summary: "planner evidence".to_owned(),
        tags: vec!["rls".to_owned()],
        sensitivity: Sensitivity::Internal,
        confidence_permille: 900,
        valid_from: chrono::Utc::now(),
        valid_to: None,
        stale_after: None,
        verification_metadata: serde_json::json!({}),
        metadata: serde_json::json!({"fixture": "CPR-20"}),
    };
    let snapshot = knowledge::create_item(
        &mut tx,
        &knowledge::NewKnowledgeItem {
            id: item_id,
            tenant_id: tenant,
            scope_id: workspace.scope_id,
            project_id: None,
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Fact,
            origin: KnowledgeOrigin::Authored,
            created_by: Some("rls-agent".to_owned()),
        },
        &knowledge::NewKnowledgeRevision {
            id: revision_id,
            content,
            created_by: Some("rls-agent".to_owned()),
        },
        &[source.id],
    )
    .await
    .expect("create Knowledge fixture");
    let content_hash = snapshot.revision.content_hash;
    context::insert_candidate(
        &mut tx,
        tenant,
        &context::NewContextCandidate {
            id: ContextCandidateId::new(),
            context_run_id: run.id,
            ordinal: 0,
            channel: synveda_types::configuration::ConfigurationContextChannel::CurrentKnowledge,
            knowledge_item_id: Some(item_id),
            knowledge_revision_id: Some(revision_id),
            capture_candidate_id: None,
            content_hash: content_hash.clone(),
            scope_id: Some(workspace.scope_id),
            lifecycle_state: Some(KnowledgeLifecycleState::Active),
            keyword_score_micros: 500_000,
            semantic_score_micros: 0,
            freshness_score_micros: 100_000,
            pin_score_micros: 0,
            current_state_score_micros: 100_000,
            final_score_micros: 700_000,
            reason_codes: vec![ContextReasonCode::KeywordMatch],
            exclusion_reason: None,
        },
    )
    .await
    .expect("record candidate fixture");
    let selection = context::insert_selection(
        &mut tx,
        tenant,
        &context::NewContextSelection {
            id: ContextSelectionId::new(),
            context_run_id: run.id,
            rank: 1,
            channel: synveda_types::configuration::ConfigurationContextChannel::CurrentKnowledge,
            knowledge_item_id: Some(item_id),
            knowledge_revision_id: Some(revision_id),
            capture_candidate_id: None,
            content_hash,
            token_count: 8,
            reason_codes: vec![ContextReasonCode::KeywordMatch],
        },
    )
    .await
    .expect("record selection fixture");
    context::insert_feedback(
        &mut tx,
        tenant,
        &context::NewContextFeedback {
            id: ContextFeedbackId::new(),
            context_run_id: run.id,
            context_selection_id: selection.id,
            knowledge_revision_id: revision_id,
            feedback_type: ContextFeedbackType::Helpful,
            principal_id: "rls-agent".to_owned(),
            idempotency_key: "rls-feedback".to_owned(),
        },
    )
    .await
    .expect("record feedback fixture");
    tx.commit().await.expect("commit session fixture");
    SessionFixture {
        tenant,
        workspace: workspace.id,
        scope: workspace.scope_id,
        session: session.id,
        run: run.id,
    }
}

/// Rows of `tenant` visible through the three tables, in the order
/// (sessions, session_events, session_context_runs).
async fn visible_session_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64) {
    let sessions = sqlx::query_scalar!(
        r#"select count(*) as "count!" from sessions where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count sessions");
    let events = sqlx::query_scalar!(
        r#"select count(*) as "count!" from session_events where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count session_events");
    let runs = sqlx::query_scalar!(
        r#"select count(*) as "count!" from session_context_runs where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count session_context_runs");
    (sessions, events, runs)
}

/// Rows of `tenant` visible through the three CPR-20 trace tables.
async fn visible_context_rows(
    tx: &mut Transaction<'static, Postgres>,
    tenant: TenantId,
) -> (i64, i64, i64) {
    let candidates = sqlx::query_scalar!(
        r#"select count(*) as "count!" from context_candidates where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count context_candidates");
    let selections = sqlx::query_scalar!(
        r#"select count(*) as "count!" from context_selections where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count context_selections");
    let feedback = sqlx::query_scalar!(
        r#"select count(*) as "count!" from context_feedback where tenant_id = $1"#,
        tenant.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await
    .expect("count context_feedback");
    (candidates, selections, feedback)
}

/// The wrong (or absent) tenant GUC sees zero rows in all three tables; the
/// right one sees exactly its own.
#[test]
fn wrong_tenant_guc_sees_no_session_rows() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let victim = seed_session(&db.pool).await;
        let adversary = seed_session(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(adversary.tenant)).await;
        assert_eq!(
            visible_session_rows(&mut tx, victim.tenant).await,
            (0, 0, 0),
            "session-ledger rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(
            visible_session_rows(&mut tx, adversary.tenant).await,
            (1, 2, 1)
        );
        assert_eq!(
            visible_context_rows(&mut tx, victim.tenant).await,
            (0, 0, 0),
            "planner trace rows leaked across tenants under the wrong GUC"
        );
        assert_eq!(
            visible_context_rows(&mut tx, adversary.tenant).await,
            (1, 1, 1)
        );
        // Not a count: the *content*. An event payload is a transcript and a
        // context run holds composed material, so a leak here is the leak this
        // whole product exists to prevent.
        let leaked = sqlx::query_scalar!(
            r#"select count(*) as "count!" from session_events
               where payload::text like '%a secret plan%'
                 and tenant_id = $1"#,
            victim.tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("search for the victim's transcript");
        assert_eq!(leaked, 0, "another tenant's transcript was readable");
        drop(tx);

        let mut tx = app_tx(&db.pool, None).await;
        assert_eq!(
            visible_session_rows(&mut tx, victim.tenant).await,
            (0, 0, 0),
            "session-ledger rows visible without any tenant GUC"
        );
        assert_eq!(
            visible_context_rows(&mut tx, victim.tenant).await,
            (0, 0, 0),
            "planner trace rows visible without any tenant GUC"
        );
    });
}

/// Opening a session into another tenant than the GUC's trips the policy's
/// WITH CHECK — an application defect, surfaced as internal.
#[test]
fn cross_tenant_session_is_rejected() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let mine = seed_session(&db.pool).await;
        let theirs = seed_session(&db.pool).await;
        let mut tx = app_tx(&db.pool, Some(mine.tenant)).await;
        let result = sessions::create(
            &mut tx,
            &sessions::NewSession {
                id: SessionId::new(),
                tenant_id: theirs.tenant,
                workspace_id: theirs.workspace,
                project_id: None,
                principal_id: "intruder".to_owned(),
                client_name: "claude-code".to_owned(),
                client_version: None,
                client_installation_id: None,
                external_session_id: None,
                agent_name: None,
                model_name: None,
                repository_id: None,
                branch: None,
                task_summary: None,
                metadata: serde_json::json!({}),
            },
        )
        .await;
        assert!(
            matches!(
                result,
                Err(Error::NotFound { .. }) | Err(Error::Internal { .. })
            ),
            "a cross-tenant session must be refused, got {result:?}"
        );
    });
}

/// The application role holds **no UPDATE and no DELETE** on `session_events`
/// or `session_context_runs`: "immutable" is a privilege here, not a
/// discipline, so a defect in this crate cannot rewrite a transcript.
#[test]
fn the_app_role_cannot_rewrite_or_delete_a_transcript() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;

        for (statement, id) in [
            (
                "update session_events set payload = '{}'::jsonb where session_id = $1",
                fixture.session.as_uuid(),
            ),
            (
                "delete from session_events where session_id = $1",
                fixture.session.as_uuid(),
            ),
            (
                "update session_context_runs set rendered = '' where id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "delete from session_context_runs where id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "update context_candidates set final_score_micros = 0 where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "delete from context_candidates where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "update context_selections set token_count = 0 where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "delete from context_selections where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "update context_feedback set feedback_type = 'unhelpful' where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
            (
                "delete from context_feedback where context_run_id = $1",
                fixture.run.as_uuid(),
            ),
        ] {
            let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
            let err = sqlx::query(statement)
                .bind(id)
                .execute(&mut *tx)
                .await
                .expect_err(&format!("{statement} must be refused"));
            // Two mechanisms, one property. An UPDATE is refused by the
            // missing grant (`42501`); a DELETE from `session_events` is
            // refused by migration 0046's trigger (`P0001`), because retention
            // disposal does need the grant and declares itself with a
            // transaction-local flag. What matters is that neither reaches a
            // row, and that a handler which has not said it is retention
            // cannot retire a transcript.
            let code = err
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .map(std::borrow::Cow::into_owned);
            assert!(
                matches!(code.as_deref(), Some("42501" | "P0001")),
                "{statement}: expected a refusal, got {err:?}"
            );
            tx.rollback().await.expect("rollback");
        }

        // And no DELETE on `sessions` either: a run is what events, candidates,
        // knowledge provenance and audit events name, so disposal belongs to
        // the retention plane.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let err = sqlx::query("delete from sessions where id = $1")
            .bind(fixture.session.as_uuid())
            .execute(&mut *tx)
            .await
            .expect_err("sessions must not be deletable");
        assert!(
            err.as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref()
                == Some("42501"),
            "sessions: expected insufficient_privilege, got {err:?}"
        );
    });
}

/// CPR-20 trace immutability also binds the migration owner. Missing app-role
/// grants are not enough: break-glass SQL must be unable to rewrite the
/// historical explanation without an explicit future retention design.
#[test]
fn the_database_owner_cannot_rewrite_context_trace_history() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;
        for statement in [
            "update session_context_runs set rendered = '' where id = $1",
            "delete from session_context_runs where id = $1",
            "update context_candidates set final_score_micros = 0 where context_run_id = $1",
            "delete from context_candidates where context_run_id = $1",
            "update context_selections set token_count = 0 where context_run_id = $1",
            "delete from context_selections where context_run_id = $1",
            "update context_feedback set feedback_type = 'unhelpful' where context_run_id = $1",
            "delete from context_feedback where context_run_id = $1",
        ] {
            let mut tx = db.pool.begin().await.expect("begin owner trace probe");
            let err = sqlx::query(statement)
                .bind(fixture.run.as_uuid())
                .execute(&mut *tx)
                .await
                .expect_err(&format!("{statement} must trip immutable trigger"));
            assert_eq!(
                err.as_database_error()
                    .and_then(sqlx::error::DatabaseError::code)
                    .as_deref(),
                Some("P0001"),
                "{statement}: owner bypassed immutable trace trigger: {err:?}"
            );
            tx.rollback().await.expect("roll back owner trace probe");
        }
    });
}

/// The structural rules the migration makes facts, checked against **direct
/// SQL** rather than through the store — because a rule that only the store
/// enforces holds only for callers who went through the store.
#[test]
fn the_session_row_rules_hold_against_direct_sql() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;

        // 1. The anchor is derived, not chosen: a scope that is not the
        //    workspace's is refused by `sessions_anchor_check`.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let forged = sqlx::query(
            "insert into sessions (id, tenant_id, workspace_id, workspace_scope_id, scope_id, \
                                   principal_id, client_name) \
             values ($1, $2, $3, $4, $5, 'forger', 'claude-code')",
        )
        .bind(SessionId::new().as_uuid())
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.workspace.as_uuid())
        .bind(fixture.scope.as_uuid())
        .bind(ScopeId::new().as_uuid())
        .execute(&mut *tx)
        .await;
        assert!(
            forged.is_err(),
            "a session must not be anchored at a scope its workspace does not own"
        );
        tx.rollback().await.expect("rollback");

        // 2. A closed run never reopens, and never changes how it closed.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        sessions::transition(
            &mut tx,
            fixture.tenant,
            fixture.session,
            SessionStatus::Ended,
            None,
            None,
        )
        .await
        .expect("close it");
        let reopened = sqlx::query("update sessions set status = 'active' where id = $1")
            .bind(fixture.session.as_uuid())
            .execute(&mut *tx)
            .await;
        assert!(
            reopened.is_err(),
            "a closed run reopened through direct SQL"
        );
        tx.rollback().await.expect("rollback");

        // 3. `ended_at` and the status agree in both directions.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let dangling = sqlx::query("update sessions set ended_at = now() where id = $1")
            .bind(fixture.session.as_uuid())
            .execute(&mut *tx)
            .await;
        assert!(
            dangling.is_err(),
            "an active run must not carry an end time"
        );
        tx.rollback().await.expect("rollback");

        // 4. One position per session, and one client event id per session.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let duplicate = sqlx::query(
            "insert into session_events (id, tenant_id, session_id, event_type, \
                                         client_event_id, sequence, occurred_at, payload_hash) \
             values ($1, $2, $3, 'message.user', 'e1', 99, now(), $4)",
        )
        .bind(synveda_types::SessionEventId::new().as_uuid())
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.session.as_uuid())
        .bind("ab".repeat(32))
        .execute(&mut *tx)
        .await;
        assert!(duplicate.is_err(), "one client event id per session");
        tx.rollback().await.expect("rollback");

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let collision = sqlx::query(
            "insert into session_events (id, tenant_id, session_id, event_type, \
                                         client_event_id, sequence, occurred_at, payload_hash) \
             values ($1, $2, $3, 'message.user', 'e99', 1, now(), $4)",
        )
        .bind(synveda_types::SessionEventId::new().as_uuid())
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.session.as_uuid())
        .bind("ab".repeat(32))
        .execute(&mut *tx)
        .await;
        assert!(collision.is_err(), "one position per session");
        tx.rollback().await.expect("rollback");

        // 5. The event vocabulary is closed at the row, not only in Rust.
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        let unknown = sqlx::query(
            "insert into session_events (id, tenant_id, session_id, event_type, \
                                         client_event_id, sequence, occurred_at, payload_hash) \
             values ($1, $2, $3, 'message.system', 'e100', 100, now(), $4)",
        )
        .bind(synveda_types::SessionEventId::new().as_uuid())
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.session.as_uuid())
        .bind("ab".repeat(32))
        .execute(&mut *tx)
        .await;
        assert!(unknown.is_err(), "the event vocabulary is closed");
        tx.rollback().await.expect("rollback");
    });
}

/// The ledger works end to end as `synveda_app` under the right GUC: the
/// backstop isolates, it does not deny service.
#[test]
fn same_tenant_session_lifecycle_works_under_rls() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed_session(&db.pool).await;

        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;
        // A harness that forgot everything but its own id finds its run again.
        let found = sessions::by_external_ref(
            &mut *tx,
            fixture.tenant,
            "rls-agent",
            "claude-code",
            "harness-1",
        )
        .await
        .expect("look up by harness id")
        .expect("found");
        assert_eq!(found.id, fixture.session);

        // A redelivered batch appends nothing twice and answers with the
        // stored rows.
        let again = sessions::append_events(
            &mut tx,
            fixture.tenant,
            fixture.session,
            &[sessions::NewSessionEvent {
                event_type: SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: "e1".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({"text": "a secret plan"}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await
        .expect("re-append under RLS");
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].outcome, sessions::AppendOutcome::Duplicate);
        assert_eq!(
            again[0].event.sequence, 1,
            "the stored position, not a new one"
        );

        // `last_observed_at` moved with the first append and does not move
        // back for a duplicate.
        let session = sessions::get(&mut *tx, fixture.tenant, fixture.session)
            .await
            .expect("read under RLS")
            .expect("still there");
        assert!(session.last_observed_at.is_some());

        let (listed, truncated) = sessions::list(
            &mut *tx,
            fixture.tenant,
            &sessions::SessionFilter {
                scope_id: Some(fixture.scope),
                ..Default::default()
            },
        )
        .await
        .expect("list under RLS");
        assert_eq!(listed.len(), 1);
        assert!(!truncated);

        // CPR-11: a reason is part of a close, so an `active` row carrying one
        // is a state nothing wrote — and that is a CHECK rather than a
        // service's discipline, which is why this asserts it against **direct
        // SQL** rather than through `transition`. A rule that lives in a
        // function holds only for callers who went through that function.
        let while_running = sqlx::query("update sessions set end_reason = $2 where id = $1")
            .bind(fixture.session.as_uuid())
            .bind("it has not ended")
            .execute(&mut *tx)
            .await;
        assert!(
            while_running.is_err(),
            "an active run may not carry an end reason, got {while_running:?}"
        );
        // A failed statement poisons the transaction, so the rest of this test
        // continues on a fresh one — the same shape every negative direct-SQL
        // assertion in this file uses.
        tx.rollback().await.expect("roll back the refused write");
        let mut tx = app_tx(&db.pool, Some(fixture.tenant)).await;

        sessions::transition(
            &mut tx,
            fixture.tenant,
            fixture.session,
            SessionStatus::Ending,
            None,
            None,
        )
        .await
        .expect("begin closing");
        // `ending` still accepts the events already buffered — the whole
        // reason there are five states rather than three.
        sessions::append_events(
            &mut tx,
            fixture.tenant,
            fixture.session,
            &[sessions::NewSessionEvent {
                event_type: SessionEventType::SessionEnded,
                event_schema_version: 1,
                client_event_id: "e3".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await
        .expect("a buffered event still lands while ending");
        let closed = sessions::transition(
            &mut tx,
            fixture.tenant,
            fixture.session,
            SessionStatus::Ended,
            Some("done"),
            Some("the agent finished its task"),
        )
        .await
        .expect("close it");
        assert_eq!(closed.status, SessionStatus::Ended);
        assert!(closed.ended_at.is_some());
        assert_eq!(closed.task_summary.as_deref(), Some("done"));
        // CPR-11: how it ended, beside what it was about. Two fields because
        // they answer two questions, and a client that set both must get both
        // back.
        assert_eq!(
            closed.end_reason.as_deref(),
            Some("the agent finished its task")
        );

        let refused = sessions::append_events(
            &mut tx,
            fixture.tenant,
            fixture.session,
            &[sessions::NewSessionEvent {
                event_type: SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: "e4".to_owned(),
                occurred_at: chrono::Utc::now(),
                payload: serde_json::json!({}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await;
        assert!(
            matches!(refused, Err(Error::Conflict { .. })),
            "a closed run takes no more events, got {refused:?}"
        );
    });
}

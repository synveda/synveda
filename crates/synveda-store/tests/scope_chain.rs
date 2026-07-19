//! HIER-2 acceptance criteria (ADR-0016): the scope chain resolver serves
//! the ordered user→…→org chain from cache, a hierarchy change invalidates
//! it (the cache invalidation test), and a warm resolve answers in well
//! under 0.5ms at p99. Plus the contract around it: no negative caching,
//! and tenant-scoped keys backed by a tenant-filtered read.
//!
//! These tests need a live Postgres; they read `DATABASE_URL` and skip
//! with a message when it is unset (CI has no database); run them locally
//! with `make dev-up` then `make db-test`. Isolation is by freshly minted
//! UUIDv7 tenants, so a shared dev database is fine.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use synveda_store::{ScopeChainCache, hierarchy, tenants};
use synveda_types::{HierarchyNode, ScopeId, ScopeKind, TenantId, TenantStatus};

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
                    "skipping scope chain tests: DATABASE_URL is not set \
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
    let slug = format!("hier2-{}", tenant.as_uuid().simple());
    tenants::create(pool, tenant, &slug, "HIER-2 fixture", TenantStatus::Active)
        .await
        .expect("create tenant");
    tenant
}

async fn tx(pool: &PgPool) -> Transaction<'static, Postgres> {
    pool.begin().await.expect("begin transaction")
}

/// Creates a node in its own committed transaction.
async fn add(
    pool: &PgPool,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    let mut tx = tx(pool).await;
    let node = hierarchy::create(&mut tx, ScopeId::new(), tenant, parent, kind, slug, slug)
        .await
        .unwrap_or_else(|err| panic!("create {kind} {slug:?}: {err}"));
    tx.commit().await.expect("commit create");
    node
}

fn ids(chain: &[HierarchyNode]) -> Vec<ScopeId> {
    chain.iter().map(|node| node.id).collect()
}

/// A five-level fixture: org → division → department → team → user,
/// returned root-last so destructuring reads like the chain.
async fn five_levels(
    pool: &PgPool,
    tenant: TenantId,
) -> (
    HierarchyNode,
    HierarchyNode,
    HierarchyNode,
    HierarchyNode,
    HierarchyNode,
) {
    let org = add(pool, tenant, None, ScopeKind::Org, "acme").await;
    let div = add(pool, tenant, Some(org.id), ScopeKind::Division, "emea").await;
    let dept = add(pool, tenant, Some(div.id), ScopeKind::Department, "pay").await;
    let team = add(pool, tenant, Some(dept.id), ScopeKind::Team, "core").await;
    let user = add(pool, tenant, Some(team.id), ScopeKind::User, "alice").await;
    (org, div, dept, team, user)
}

// ── The resolver contract ────────────────────────────────────────────────────

/// The identity→chain shape (HIER-2): the user node first, then every
/// ancestor nearest-first, org root last — matching what the composition
/// gradient consumes (seed §4.4).
#[test]
fn resolves_the_ordered_chain_user_first_org_last() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (org, div, dept, team, user) = five_levels(&db.pool, tenant).await;
        let cache = ScopeChainCache::new();

        let chain = cache
            .resolve(&db.pool, tenant, user.id)
            .await
            .expect("resolve")
            .expect("user chain exists");
        assert_eq!(
            ids(&chain),
            vec![user.id, team.id, dept.id, div.id, org.id],
            "node first, ancestors nearest-first, org root last"
        );

        // Interior nodes resolve too — the resource chain rides the same
        // resolver as the placement chain.
        let chain = cache
            .resolve(&db.pool, tenant, dept.id)
            .await
            .expect("resolve")
            .expect("department chain exists");
        assert_eq!(ids(&chain), vec![dept.id, div.id, org.id]);
    });
}

/// The AC: a hierarchy change invalidates the cache. The middle
/// assertion — a stale chain served *before* invalidation — is what
/// proves the cache is a cache; the post-invalidation resolve must
/// reflect the committed move.
#[test]
fn ac_invalidation_serves_the_fresh_chain_after_a_move() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (org, div_a, dept, team, user) = five_levels(&db.pool, tenant).await;
        let div_b = add(&db.pool, tenant, Some(org.id), ScopeKind::Division, "apac").await;
        let cache = ScopeChainCache::new();

        let warm = cache
            .resolve(&db.pool, tenant, user.id)
            .await
            .expect("resolve")
            .expect("user chain exists");
        assert!(
            ids(&warm).contains(&div_a.id),
            "warm chain runs through emea"
        );

        // Move the department (and its subtree) to the other division.
        let mut mv = tx(&db.pool).await;
        hierarchy::move_node(&mut mv, dept.id, div_b.id)
            .await
            .expect("move department");
        mv.commit().await.expect("commit move");

        // Not yet invalidated: the cache still serves the pre-move chain.
        let stale = cache
            .resolve(&db.pool, tenant, user.id)
            .await
            .expect("resolve")
            .expect("cached chain");
        assert!(
            ids(&stale).contains(&div_a.id),
            "before invalidation the cached (pre-move) chain is served — it is a cache"
        );

        // The gateway invalidates post-commit; the next resolve re-reads.
        cache.invalidate(tenant);
        let fresh = cache
            .resolve(&db.pool, tenant, user.id)
            .await
            .expect("resolve")
            .expect("fresh chain");
        assert_eq!(
            ids(&fresh),
            vec![user.id, team.id, dept.id, div_b.id, org.id],
            "after invalidation the chain reflects the committed move"
        );
    });
}

/// Negative results are never cached (ADR-0016 decision 2): a scope that
/// does not exist yet resolves to `None`, and once created it is visible
/// with no invalidation in between.
#[test]
fn unknown_scopes_are_not_cached_negatively() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let org = add(&db.pool, tenant, None, ScopeKind::Org, "acme").await;
        let cache = ScopeChainCache::new();

        let id = ScopeId::new();
        assert!(
            cache
                .resolve(&db.pool, tenant, id)
                .await
                .expect("resolve")
                .is_none(),
            "an unknown scope has no chain"
        );

        let mut create = tx(&db.pool).await;
        hierarchy::create(
            &mut create,
            id,
            tenant,
            Some(org.id),
            ScopeKind::Team,
            "core",
            "Core",
        )
        .await
        .expect("create team");
        create.commit().await.expect("commit create");

        let chain = cache
            .resolve(&db.pool, tenant, id)
            .await
            .expect("resolve")
            .expect("the fresh node resolves without any invalidation");
        assert_eq!(ids(&chain), vec![id, org.id]);
    });
}

/// Tenant correctness lives in the key *and* the read: another tenant
/// probing a real scope id resolves nothing and caches nothing, even
/// though the probed chain is warm in the cache — and even on dev
/// connections where the RLS backstop does not bite (ADR-0016 decision 2;
/// the pool here is the compose superuser, exactly the connection RLS
/// exempts).
#[test]
fn cross_tenant_probes_resolve_nothing() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let owner = admit_tenant(&db.pool).await;
        let prober = admit_tenant(&db.pool).await;
        let (_, _, _, _, user) = five_levels(&db.pool, owner).await;
        let cache = ScopeChainCache::new();

        cache
            .resolve(&db.pool, owner, user.id)
            .await
            .expect("resolve")
            .expect("owner chain warms the cache");

        assert!(
            cache
                .resolve(&db.pool, prober, user.id)
                .await
                .expect("resolve")
                .is_none(),
            "a foreign tenant must not see the chain, cached or not"
        );
    });
}

// ── The performance AC ───────────────────────────────────────────────────────

/// The AC: p99 < 0.5ms warm. A warm resolve is a read lock and an `Arc`
/// clone — no database round-trip — so the bound is asserted absolutely
/// (the delta-over-baseline discipline exists for queries that cross the
/// Docker link; a hit never does).
#[test]
fn ac_warm_resolves_answer_under_half_a_millisecond_p99() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let tenant = admit_tenant(&db.pool).await;
        let (_, _, _, _, user) = five_levels(&db.pool, tenant).await;
        let cache = ScopeChainCache::new();

        const WARMUP: usize = 100;
        const SAMPLES: usize = 10_000;
        let mut samples = Vec::with_capacity(SAMPLES);
        for i in 0..WARMUP + SAMPLES {
            let clock = Instant::now();
            let chain = cache
                .resolve(&db.pool, tenant, user.id)
                .await
                .expect("resolve")
                .expect("warm chain");
            let elapsed = clock.elapsed();
            assert_eq!(chain.len(), 5);
            if i >= WARMUP {
                samples.push(elapsed);
            }
        }
        samples.sort();
        let median = samples[SAMPLES / 2];
        let p99 = samples[SAMPLES * 99 / 100];
        println!("warm resolve: median {median:?}, p99 {p99:?} over {SAMPLES} samples");
        assert!(
            p99 < Duration::from_micros(500),
            "warm p99 {p99:?} must be under 0.5ms"
        );
    });
}

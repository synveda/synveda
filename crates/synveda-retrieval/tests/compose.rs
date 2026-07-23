//! CTX-2 AC tests (ADR-0025): deterministic given same inputs; every
//! block watermarked with version hashes + record ids;
//! `tokens_per_inject` emitted — plus the gradient, pinned-first,
//! conflict, channel, budget, valid-time, sensitivity, and relevance
//! rules the feature names.
//!
//! These tests need a live Postgres (the bitemporal-suite harness:
//! `DATABASE_URL` or quiet skip). Tenant isolation is by freshly minted
//! ids.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, Utc};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_retrieval::{ComposeRequest, ComposeScope, ComposedBlock, compose};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::{
    IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeKind, Sensitivity, TenantId,
};

// ── Harness ──────────────────────────────────────────────────────────────────

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping composition tests: DATABASE_URL is not set \
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

/// The process-global recorder the metric AC asserts against (the
/// gateway test suites' pattern).
fn metrics_handle() -> &'static PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install prometheus recorder")
    })
}

/// A four-level chain of fresh scopes, nearest-first, both channels on.
struct Chain {
    user: ScopeId,
    team: ScopeId,
    dept: ScopeId,
    org: ScopeId,
}

impl Chain {
    fn new() -> Self {
        Chain {
            user: ScopeId::new(),
            team: ScopeId::new(),
            dept: ScopeId::new(),
            org: ScopeId::new(),
        }
    }

    fn scopes(&self) -> Vec<ComposeScope> {
        [
            (self.user, ScopeKind::User, "acme/eng/team-a/alice"),
            (self.team, ScopeKind::Team, "acme/eng/team-a"),
            (self.dept, ScopeKind::Department, "acme/eng"),
            (self.org, ScopeKind::Org, "acme"),
        ]
        .into_iter()
        .map(|(scope_id, kind, path)| ComposeScope {
            scope_id,
            kind,
            path: path.to_owned(),
            include_derived: true,
        })
        .collect()
    }
}

struct Insert<'a> {
    scope: ScopeId,
    kind: RecordKind,
    content: &'a str,
    sensitivity: Sensitivity,
    valid_from: DateTime<Utc>,
    valid_to: Option<DateTime<Utc>>,
}

impl<'a> Insert<'a> {
    fn derived(scope: ScopeId, content: &'a str, valid_from: DateTime<Utc>) -> Self {
        Insert {
            scope,
            kind: RecordKind::Derived,
            content,
            sensitivity: Sensitivity::Internal,
            valid_from,
            valid_to: None,
        }
    }

    fn pinned(scope: ScopeId, content: &'a str, valid_from: DateTime<Utc>) -> Self {
        Insert {
            kind: RecordKind::Pinned,
            ..Insert::derived(scope, content, valid_from)
        }
    }
}

async fn insert(pool: &PgPool, tenant: TenantId, spec: Insert<'_>) -> RecordId {
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: spec.scope,
            owner_id: IdentityId::new(),
            kind: spec.kind,
            class: RecordClass::Fact,
            content: spec.content.to_owned(),
            sensitivity: spec.sensitivity,
            provenance: serde_json::json!({"source": "ctx-2 acceptance test"}),
            valid_from: spec.valid_from,
            valid_to: spec.valid_to,
        },
        &RecordEmbedding {
            model: "test@1".to_owned(),
            vector: vec![0.5; 16],
        },
    )
    .await
    .expect("insert record");
    id
}

/// Runs one compose inside a fresh tenant transaction (the RLS
/// discipline production follows).
fn run(db: &Db, tenant: TenantId, request: &ComposeRequest) -> ComposedBlock {
    metrics_handle();
    db.rt.block_on(async {
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        let block = compose(&mut tx, tenant, request).await.expect("compose");
        drop(tx);
        block
    })
}

fn ids(block: &ComposedBlock) -> Vec<RecordId> {
    block.entries.iter().map(|entry| entry.record_id).collect()
}

// ── The gradient and pinned-first (seed §4.4) ────────────────────────────────

/// Assembly order is user > team > department > org, pinned before
/// derived within each level, and derived — never pinned — is marked
/// unreviewed in the rendered text.
#[test]
fn gradient_assembles_nearest_first_pinned_first() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    // Inserted deliberately out of gradient order.
    let org_pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, "org convention: request reviews early", now),
    ));
    let user_derived = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers tabs", now),
    ));
    let team_derived = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "team-a deploys with make deploy", now),
    ));
    let team_pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "team-a owns the payments service", now),
    ));

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    assert_eq!(
        ids(&block),
        vec![user_derived, team_pin, team_derived, org_pin],
        "gradient order, pinned first within the level"
    );
    assert!(
        block.text.contains("alice prefers tabs [unreviewed]"),
        "derived is marked unreviewed: {}",
        block.text
    );
    assert!(
        block
            .text
            .contains("- [fact] team-a owns the payments service\n"),
        "pinned carries no unreviewed mark: {}",
        block.text
    );
    let user_at = block.text.find("alice prefers tabs").expect("user entry");
    let team_at = block.text.find("team-a owns").expect("team entry");
    let org_at = block.text.find("org convention").expect("org entry");
    assert!(user_at < team_at && team_at < org_at, "sections in order");
}

// ── The determinism AC ───────────────────────────────────────────────────────

/// Deterministic given same inputs: the same plan, instant, and
/// database state re-compose to a byte-identical block — hash included
/// — even after unrelated writes elsewhere.
#[test]
fn deterministic_given_same_inputs() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    for content in ["fact one", "fact two", "fact three"] {
        db.rt.block_on(insert(
            &db.pool,
            tenant,
            Insert::derived(chain.user, content, now),
        ));
    }
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "the pinned fact", now),
    ));
    let request = ComposeRequest::new(chain.scopes(), 1_500, now);

    let first = run(db, tenant, &request);
    // An unrelated tenant's write must not perturb the block.
    let other = TenantId::new();
    db.rt.block_on(insert(
        &db.pool,
        other,
        Insert::derived(ScopeId::new(), "someone else's fact", now),
    ));
    let second = run(db, tenant, &request);

    assert_eq!(first.text, second.text, "byte-identical text");
    assert_eq!(first.block_hash, second.block_hash);
    assert_eq!(first.entries, second.entries);
    assert_eq!(first.tokens, second.tokens);
}

// ── The watermark AC ─────────────────────────────────────────────────────────

/// Every block is watermarked: per-entry BLAKE3 version hashes, a block
/// hash recomputable from them, and the rendered watermark line carries
/// the block hash and every composed record id.
#[test]
fn watermark_carries_hashes_and_record_ids() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "watermarked fact", now),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, "watermarked convention", now),
    ));

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    assert_eq!(block.entries.len(), 2);
    for entry in &block.entries {
        assert_eq!(entry.version_hash.len(), 64, "BLAKE3 hex");
        assert!(
            entry.version_hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hex-encoded"
        );
        assert!(
            block.text.contains(&entry.record_id.to_string()),
            "record id {} on the watermark line",
            entry.record_id
        );
    }
    // The block hash is BLAKE3 over the ordered entry hashes —
    // recomputable by any consumer holding the watermark.
    let mut hasher = blake3::Hasher::new();
    for entry in &block.entries {
        hasher.update(entry.version_hash.as_bytes());
    }
    let recomputed = hasher.finalize().to_hex().to_string();
    assert_eq!(block.block_hash, recomputed, "block hash recomputes");
    assert!(
        block
            .text
            .contains(&format!("synveda:watermark v1 blake3={}", block.block_hash)),
        "watermark line carries the block hash: {}",
        block.text
    );
}

// ── The budget ───────────────────────────────────────────────────────────────

/// The budget is enforced on estimated tokens of the whole rendered
/// block; nearer-scope material was placed first, so the broad long
/// record is what gets skipped.
#[test]
fn budget_is_enforced_nearest_first() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let short = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "the short user fact", now),
    ));
    let long_content = "org lore ".repeat(200);
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, long_content.trim(), now),
    ));

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 120, now));

    assert_eq!(ids(&block), vec![short], "the near record composed");
    assert_eq!(block.skipped_over_budget, 1, "the long broad one skipped");
    assert!(
        block.tokens <= 120,
        "estimated tokens {} within budget",
        block.tokens
    );
    // And the block still ends watermarked.
    assert!(block.text.contains("synveda:watermark"));
}

// ── Channel rules ────────────────────────────────────────────────────────────

/// A published-only scope (its effective pack's bank-mode switch)
/// composes pinned material only; other scopes keep both channels.
#[test]
fn published_only_scope_excludes_derived() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let user_derived = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "user derived fact", now),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "team derived fact", now),
    ));
    let team_pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "team pinned procedure", now),
    ));

    let mut scopes = chain.scopes();
    scopes[1].include_derived = false;
    let block = run(db, tenant, &ComposeRequest::new(scopes, 1_500, now));

    assert_eq!(
        ids(&block),
        vec![user_derived, team_pin],
        "team derived is out; team pinned and user derived compose"
    );
}

// ── Conflict rules (seed §4.4) ───────────────────────────────────────────────

/// The three conflict rules, each proven against the tempting wrong
/// winner: pinned beats derived even from a broader scope and an older
/// valid time; specificity beats recency among equals; newer valid time
/// wins within a scope. Losers vanish from block and watermark alike.
#[test]
fn conflicts_resolve_by_kind_then_scope_then_valid_time() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let earlier = now - Duration::hours(2);

    // Pinned at org (older) vs derived at user (newer, whitespace-
    // padded — the trimmed-content predicate still groups them).
    let org_pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, "the deploy window is friday", earlier),
    ));
    let user_dup = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "  the deploy window is friday  ", now),
    ));

    // Derived at user (older) vs derived at org (newer).
    let user_near = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "the oncall rotation is weekly", earlier),
    ));
    let org_dup = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.org, "the oncall rotation is weekly", now),
    ));

    // Two derived at the same scope: newer valid time wins.
    let team_old = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "the retro is on mondays", earlier),
    ));
    let team_new = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "the retro is on mondays", now),
    ));

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    let composed = ids(&block);
    assert!(composed.contains(&org_pin), "pinned beats derived");
    assert!(!composed.contains(&user_dup));
    assert!(composed.contains(&user_near), "specific beats broad");
    assert!(!composed.contains(&org_dup));
    assert!(composed.contains(&team_new), "newer valid time wins");
    assert!(!composed.contains(&team_old));
    assert_eq!(block.dropped_conflicts, 3);
    for loser in [user_dup, org_dup, team_old] {
        assert!(
            !block.text.contains(&loser.to_string()),
            "loser {loser} must not reach the watermark"
        );
    }
}

// ── Valid time ───────────────────────────────────────────────────────────────

/// Records compose only when their valid window covers the explicit
/// instant — and composing at an earlier instant surfaces what held
/// then (the valid-time half of CTX-5's as-of).
#[test]
fn valid_time_window_is_applied_at_the_explicit_instant() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let handover = now - Duration::hours(1);
    let old = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert {
            valid_to: Some(handover),
            ..Insert::derived(chain.team, "bob owns the pager", now - Duration::hours(3))
        },
    ));
    let new = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "carol owns the pager", handover),
    ));

    let current = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert_eq!(ids(&current), vec![new], "only the currently valid fact");

    let before = run(
        db,
        tenant,
        &ComposeRequest::new(chain.scopes(), 1_500, handover - Duration::minutes(30)),
    );
    assert_eq!(ids(&before), vec![old], "as of then, the old fact held");
}

// ── Sensitivity ──────────────────────────────────────────────────────────────

/// The ceiling is inclusive and clamped: `restricted` never composes,
/// whatever the caller asks for (ADR-0024 decision 2 reused).
#[test]
fn sensitivity_ceiling_clamps_below_restricted() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let mut by_level = Vec::new();
    for (level, content) in [
        (Sensitivity::Public, "public fact"),
        (Sensitivity::Internal, "internal fact"),
        (Sensitivity::Confidential, "confidential fact"),
        (Sensitivity::Restricted, "restricted fact"),
    ] {
        by_level.push((
            level,
            db.rt.block_on(insert(
                &db.pool,
                tenant,
                Insert {
                    sensitivity: level,
                    ..Insert::derived(chain.team, content, now)
                },
            )),
        ));
    }

    let mut internal = ComposeRequest::new(chain.scopes(), 1_500, now);
    internal.max_sensitivity = Sensitivity::Internal;
    let block = run(db, tenant, &internal);
    assert_eq!(block.entries.len(), 2, "public + internal under the floor");

    let mut asks_restricted = ComposeRequest::new(chain.scopes(), 1_500, now);
    asks_restricted.max_sensitivity = Sensitivity::Restricted;
    let block = run(db, tenant, &asks_restricted);
    let composed = ids(&block);
    assert_eq!(composed.len(), 3, "clamped to confidential");
    let restricted = by_level
        .iter()
        .find(|(level, _)| *level == Sensitivity::Restricted)
        .map(|(_, id)| *id)
        .expect("restricted fixture");
    assert!(!composed.contains(&restricted), "restricted never composes");
}

// ── Relevance ────────────────────────────────────────────────────────────────

/// Under a relevance ranking (the hybrid engine's output), unranked
/// derived records do not compose and ranked ones follow rank order —
/// while pinned material composes regardless of the task.
#[test]
fn relevance_ranks_derived_and_never_gates_pinned() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let chain = Chain::new();
    let now = Utc::now();
    let a = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "derived alpha", now),
    ));
    let b = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "derived beta", now),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "derived gamma — unranked", now),
    ));
    let pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "pinned delta — never gated", now),
    ));

    let mut request = ComposeRequest::new(chain.scopes(), 1_500, now);
    request.relevance = Some(vec![b, a]);
    let block = run(db, tenant, &request);

    assert_eq!(
        ids(&block),
        vec![pin, b, a],
        "pinned first, then derived in rank order, unranked excluded"
    );
}

// ── The metric AC and the empty compose ──────────────────────────────────────

/// `synveda_tokens_per_inject` is recorded on every compose — the
/// empty permitted set records 0 (an inject that composed nothing is
/// data), a composed block records its estimated tokens.
#[test]
fn tokens_per_inject_is_emitted_including_zero() {
    let Some(db) = db() else { return };
    let tenant = TenantId::new();
    let now = Utc::now();

    let empty = run(db, tenant, &ComposeRequest::new(Vec::new(), 1_500, now));
    assert!(empty.text.is_empty());
    assert!(empty.entries.is_empty());
    assert_eq!(empty.tokens, 0);

    let chain = Chain::new();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "a fact worth some tokens", now),
    ));
    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert!(block.tokens > 0);

    let exposition = metrics_handle().render();
    let count: u64 = exposition
        .lines()
        .find_map(|line| line.strip_prefix("synveda_tokens_per_inject_count "))
        .expect("histogram count in exposition")
        .trim()
        .parse()
        .expect("count parses");
    assert!(count >= 2, "both composes recorded (count {count})");
    let sum: f64 = exposition
        .lines()
        .find_map(|line| line.strip_prefix("synveda_tokens_per_inject_sum "))
        .expect("histogram sum in exposition")
        .trim()
        .parse()
        .expect("sum parses");
    assert!(
        sum >= f64::from(block.tokens),
        "the composed block's tokens are in the sum"
    );
}

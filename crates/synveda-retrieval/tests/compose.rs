//! CTX-2 AC tests (ADR-0025): deterministic given same inputs; every
//! block watermarked with content addresses + record ids;
//! `tokens_per_inject` emitted — plus the gradient, conflict, channel,
//! budget, valid-time, sensitivity, and relevance rules the feature
//! names.
//!
//! Since FLOW-2 (ADR-0031) the channel rules are real, and the suite
//! says so: material composes as *published* only where a scope's
//! `memory/published` tree names it at the content it now holds, so the
//! fixtures publish deliberately. `RecordKind::Pinned` here means what
//! seed §4.2 says — authored — and an unpublished authored record is
//! still unreviewed.
//!
//! These tests need a live Postgres (the bitemporal-suite harness:
//! `DATABASE_URL` or quiet skip). Tenants are admitted per test — the
//! VedaFlow tables carry a tenant foreign key.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, Utc};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_retrieval::{ComposeRequest, ComposeScope, ComposedBlock, compose};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::{
    Channel, IdentityId, RecordClass, RecordId, RecordKind, ScopeId, ScopeKind, Sensitivity,
    TenantId,
};
use synveda_vedaflow::{
    self as vedaflow, ChannelRef, ChannelWrite, MemoryAsset, PolicySnapshot, Signer,
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

/// Admits a tenant. Records need none, but the VedaFlow tables carry a
/// tenant foreign key, so a test that publishes needs one.
fn admit(db: &Db) -> TenantId {
    let tenant = TenantId::new();
    db.rt.block_on(async {
        sqlx::query("insert into tenants (id, slug, name, status) values ($1, $2, $3, 'active')")
            .bind(tenant.as_uuid())
            .bind(format!("ctx2-{}", tenant.as_uuid().simple()))
            .bind("CTX-2 / FLOW-2 acceptance test")
            .execute(&db.pool)
            .await
            .expect("admit tenant");
    });
    tenant
}

/// Publishes records onto a scope's `memory/published` channel — the
/// fixture form of what `POST /v1/channels/{scope}/publish` does under
/// the PDP (the governed route is proven in the gateway suite; seeding
/// here is the same standing `records::insert` has).
fn publish(db: &Db, tenant: TenantId, scope: ScopeId, ids: &[RecordId]) {
    db.rt.block_on(async {
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        let mut members = Vec::with_capacity(ids.len());
        for id in ids {
            let version = records::current(&mut *tx, *id)
                .await
                .expect("read record")
                .expect("record exists");
            let asset = MemoryAsset {
                id: version.id,
                scope_id: version.state.scope_id,
                owner_id: version.state.owner_id,
                kind: version.state.kind,
                class: version.state.class,
                content: version.state.content.clone(),
                sensitivity: version.state.sensitivity,
                valid_from: version.state.valid_from,
                valid_to: version.state.valid_to,
            };
            let object = vedaflow::put_memory(&mut tx, tenant, &asset)
                .await
                .expect("put memory object");
            members.push((asset.entry_name(), object.hash));
        }
        vedaflow::publish(
            &mut tx,
            tenant,
            &ChannelWrite {
                scope,
                channel: ChannelRef::memory(Channel::Published),
                members: &members,
                author: IdentityId::new(),
                message: "ctx-2 fixture publication",
                committed_at: Utc::now(),
                policy_snapshot: &PolicySnapshot::new("regulated-strict", 6),
            },
            &Signer::Unsigned,
        )
        .await
        .expect("publish");
        tx.commit().await.expect("commit publication");
    });
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

// ── The gradient and published-first (seed §4.4) ─────────────────────────────

/// Assembly order is user > team > department > org, published before
/// unpublished within each level, and everything not published is
/// marked unreviewed in the rendered text.
#[test]
fn gradient_assembles_nearest_first_published_first() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
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
    publish(db, tenant, chain.team, &[team_pin]);
    publish(db, tenant, chain.org, &[org_pin]);

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    assert_eq!(
        ids(&block),
        vec![user_derived, team_pin, team_derived, org_pin],
        "gradient order, published first within the level"
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
        "published carries no unreviewed mark: {}",
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
    let tenant = admit(db);
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

/// Every block is watermarked: per-entry VedaFlow object addresses, a
/// block hash recomputable from them, the rendered watermark line
/// carrying the block hash and every composed record id, and — since
/// FLOW-2 — the published channel commit each scope was read at
/// (ADR-0031 decision 11).
#[test]
fn watermark_carries_hashes_and_record_ids() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "watermarked fact", now),
    ));
    let convention = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, "watermarked convention", now),
    ));
    publish(db, tenant, chain.org, &[convention]);

    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    assert_eq!(block.entries.len(), 2);
    for entry in &block.entries {
        assert_eq!(entry.object_hash.len(), 64, "BLAKE3 hex");
        assert!(
            entry.object_hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hex-encoded"
        );
        assert!(
            block.text.contains(&entry.record_id.to_string()),
            "record id {} on the watermark line",
            entry.record_id
        );
    }
    // The block hash is BLAKE3 over the ordered entry addresses and the
    // channel each composed from — recomputable by any consumer holding
    // the watermark, and different for two blocks that read differently.
    let mut hasher = blake3::Hasher::new();
    for entry in &block.entries {
        hasher.update(entry.object_hash.as_bytes());
        hasher.update(entry.channel.as_str().as_bytes());
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

    // The channel watermark: the org's published ref, cited on the block
    // and (in the product path) in the inject audit event — never in the
    // rendered text, which the budget pays for.
    let cited = block
        .channels
        .iter()
        .find(|channel| channel.scope_id == chain.org)
        .expect("the org's published channel is cited");
    assert_eq!(cited.channel, "memory/published");
    assert_eq!(cited.commit.len(), 64, "a commit hash, hex");
    assert!(
        !block.text.contains(&cited.commit),
        "channel commits stay out of the budgeted text"
    );
}

// ── The budget ───────────────────────────────────────────────────────────────

/// The budget is enforced on estimated tokens of the whole rendered
/// block; nearer-scope material was placed first, so the broad long
/// record is what gets skipped.
#[test]
fn budget_is_enforced_nearest_first() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
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
/// composes that scope's *published* material only; other scopes keep
/// both channels.
#[test]
fn published_only_scope_excludes_derived() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
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
    let team_published = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "team published procedure", now),
    ));
    publish(db, tenant, chain.team, &[team_published]);

    let mut scopes = chain.scopes();
    scopes[1].include_derived = false;
    let block = run(db, tenant, &ComposeRequest::new(scopes, 1_500, now));

    assert_eq!(
        ids(&block),
        vec![user_derived, team_published],
        "team derived is out; the team's published record and user derived compose"
    );
}

/// The behaviour change FLOW-2 exists for (ADR-0031 decision 7):
/// authorship is not review. An authored (`pinned`) record nobody
/// published is unreviewed material — marked as such, and gone under
/// bank mode.
#[test]
fn authored_material_nobody_published_does_not_survive_bank_mode() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let authored = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "authored but never reviewed", now),
    ));

    let both = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert_eq!(
        ids(&both),
        vec![authored],
        "it composes while derived is on"
    );
    assert!(
        both.text
            .contains("authored but never reviewed [unreviewed]"),
        "and says it is unreviewed: {}",
        both.text
    );

    let mut bank = chain.scopes();
    for scope in &mut bank {
        scope.include_derived = false;
    }
    let block = run(db, tenant, &ComposeRequest::new(bank, 1_500, now));
    assert!(
        ids(&block).is_empty(),
        "bank mode composes nothing nobody published"
    );

    // Publish it, and the very same block composes it as reviewed.
    publish(db, tenant, chain.team, &[authored]);
    let mut bank = chain.scopes();
    for scope in &mut bank {
        scope.include_derived = false;
    }
    let after = run(db, tenant, &ComposeRequest::new(bank, 1_500, now));
    assert_eq!(ids(&after), vec![authored]);
    assert!(!after.text.contains("[unreviewed]"));
    // Same record, same content, different block: publishing changed
    // what the block says, so it changed the block's identity too.
    assert_ne!(
        both.block_hash, after.block_hash,
        "the channel is part of the block hash"
    );
}

/// Publication binds bytes, not ids (ADR-0031 decision 5): a record
/// edited after publication no longer matches the address its scope
/// admitted, so it composes as unreviewed again rather than laundering
/// the edit through a published id.
#[test]
fn editing_published_content_demotes_it_to_unreviewed() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let record = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.team, "deploy on fridays", now),
    ));
    publish(db, tenant, chain.team, &[record]);

    let before = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert!(
        !before.text.contains("[unreviewed]"),
        "published as written"
    );

    // The same id, different bytes — the whole point of the check.
    db.rt.block_on(async {
        let current = records::current(&db.pool, record)
            .await
            .expect("read record")
            .expect("record exists");
        records::update(
            &db.pool,
            record,
            &RecordState {
                content: "deploy whenever you like".to_owned(),
                ..current.state
            },
            &RecordEmbedding {
                model: "test@1".to_owned(),
                vector: vec![0.5; 16],
            },
        )
        .await
        .expect("rewrite record");
    });

    let after = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert!(
        after.text.contains("deploy whenever you like [unreviewed]"),
        "the edited version is unreviewed again: {}",
        after.text
    );
    let mut bank = chain.scopes();
    for scope in &mut bank {
        scope.include_derived = false;
    }
    let banked = run(db, tenant, &ComposeRequest::new(bank, 1_500, now));
    assert!(
        ids(&banked).is_empty(),
        "and bank mode drops it: the reviewed bytes no longer exist"
    );
}

// ── Conflict rules (seed §4.4) ───────────────────────────────────────────────

/// The conflict rules, each proven against the tempting wrong winner:
/// published beats unpublished even from a broader scope and an older
/// valid time (ADR-0031 decision 8's tier 0); specificity beats recency
/// among equals; newer valid time wins within a scope. Losers vanish
/// from block and watermark alike.
#[test]
fn conflicts_resolve_by_channel_then_scope_then_valid_time() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let earlier = now - Duration::hours(2);

    // Published at org (older) vs unpublished at user (newer,
    // whitespace-padded — the trimmed-content predicate still groups
    // them).
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
    publish(db, tenant, chain.org, &[org_pin]);

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
    assert!(
        composed.contains(&org_pin),
        "published beats nearer unpublished"
    );
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
    let tenant = admit(db);
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
    let tenant = admit(db);
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
/// while published material composes regardless of the task (ADR-0031
/// decision 9: the rule was always about the trusted channel).
#[test]
fn relevance_ranks_derived_and_never_gates_published() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
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
        Insert::pinned(chain.team, "published delta — never gated", now),
    ));
    publish(db, tenant, chain.team, &[pin]);

    let mut request = ComposeRequest::new(chain.scopes(), 1_500, now);
    request.relevance = Some(vec![b, a]);
    let block = run(db, tenant, &request);

    assert_eq!(
        ids(&block),
        vec![pin, b, a],
        "published first, then derived in rank order, unranked excluded"
    );
}

// ── The metric AC and the empty compose ──────────────────────────────────────

/// `synveda_tokens_per_inject` is recorded on every compose — the
/// empty permitted set records 0 (an inject that composed nothing is
/// data), a composed block records its estimated tokens.
#[test]
fn tokens_per_inject_is_emitted_including_zero() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
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

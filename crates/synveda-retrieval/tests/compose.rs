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
use synveda_retrieval::{ComposeRequest, ComposeScope, ComposedBlock, compose, estimated_tokens};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::rls;
use synveda_types::scope::ScopeKind;
use synveda_types::{
    Channel, ClassTtl, CompositionConfig, EntryTier, IdentityId, IndexTier, RecordClass, RecordId,
    RecordKind, RetentionConfig, ScopeId, Sensitivity, TenantId,
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

    /// The plan a placed reader gets with no explicit grant anywhere: the
    /// whole chain at the working tiers (AUTHZ-5, ADR-0038 decision 4).
    fn scopes(&self) -> Vec<ComposeScope> {
        self.scopes_at(&[Sensitivity::Public, Sensitivity::Internal])
    }

    /// The same plan with a stated tier set — what a binding, an own-home
    /// read, or a lapse that declared a ceiling produces.
    fn scopes_at(&self, sensitivities: &[Sensitivity]) -> Vec<ComposeScope> {
        [
            (self.user, ScopeKind::Principal, "acme/eng/team-a/alice"),
            (self.team, ScopeKind::OrgUnit, "acme/eng/team-a"),
            (self.dept, ScopeKind::OrgUnit, "acme/eng"),
            (self.org, ScopeKind::Tenant, "acme"),
        ]
        .into_iter()
        .map(|(scope_id, kind, path)| ComposeScope {
            scope_id,
            kind,
            path: path.to_owned(),
            include_derived: true,
            sensitivities: sensitivities.to_vec(),
            // The same tiers for pack material: these fixtures are about
            // memory, and a plan that permitted packs nowhere would leave
            // the pack path untested by construction rather than by
            // intent (PRMT-2, ADR-0050 decision 8).
            pack_sensitivities: sensitivities.to_vec(),
            // And for skills, on the same reasoning one feature later
            // (SKIL-4, ADR-0054 decision 10): these fixtures publish no
            // skill channel, so the advertisement finds nothing — which is
            // the state a plan permitting skills nowhere could not tell
            // apart from a bug.
            skill_sensitivities: sensitivities.to_vec(),
            // The product config: the index tier on, so these fixtures
            // exercise what a real chain composes under (ADR-0041
            // decision 11). Nothing here is long enough to demote, which
            // is decision 2 doing its job rather than an accident.
            index_tier: CompositionConfig::DEFAULT.index_tier,
            index_entry_chars: CompositionConfig::DEFAULT.index_entry_chars,
            skill_index: CompositionConfig::DEFAULT.skill_index,
            // The caller's own chain: nothing here arrives by a grant.
            lapse: None,
            // The product config: the machinery on, no record horizon,
            // so these fixtures compose exactly as they did before MEM-6
            // (ADR-0040 decision 13).
            retention: RetentionConfig::DEFAULT,
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
                merge_parents: &[],
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

// ── Climbed material (FLOW-5, ADR-0034 decision 6) ───────────────────────────

/// A scope's published channel may name a record that lives *below* it,
/// and composition reads it that way: the entry takes the publishing
/// scope's position in the gradient and the publishing scope's section,
/// not the residence its row records.
///
/// This is the read-path half of a cross-scope promotion, and it is
/// asserted from the side that makes it matter — the reader's. The record
/// lives at a team this caller is not in and can never read; what admits
/// it is the department's publication, and nothing else. Before that
/// publication the same record composes nothing at all, which is the
/// control: residence alone never admits anything.
#[test]
fn an_ancestors_published_channel_admits_a_record_living_below_it() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    // A team this caller has no chain relationship with — a sibling under
    // the same department, which is exactly where a climb starts.
    let sibling = ScopeId::new();
    let now = Utc::now();
    let runbook = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(sibling, "rotate the signing key every 90 days", now),
    ));

    // Published at the sibling team only: not on this caller's chain, so
    // the department's channel says nothing about it and neither does the
    // block.
    publish(db, tenant, sibling, &[runbook]);
    let before = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert!(
        ids(&before).is_empty(),
        "another team's material must not compose: {}",
        before.text
    );

    // The climb lands: the department publishes it, and the record has not
    // moved an inch.
    publish(db, tenant, chain.dept, &[runbook]);
    let after = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert_eq!(
        ids(&after),
        vec![runbook],
        "the department's publication admits it: {}",
        after.text
    );
    assert!(
        !after.text.contains("[unreviewed]"),
        "and it composes as reviewed, not derived: {}",
        after.text
    );

    // The watermark and the rendering both name the scope it composed
    // *from*. A reader is never shown a section for a scope they cannot
    // see, and an auditor asking "why was this in that block" is pointed
    // at the decision that admitted it.
    let entry = &after.entries[0];
    assert_eq!(
        entry.scope_id, chain.dept,
        "the entry belongs to the publishing scope"
    );
    assert_eq!(entry.channel, Channel::Published);
    assert!(
        after.text.contains("## acme/eng (org_unit)"),
        "sectioned under the department: {}",
        after.text
    );

    // Bank mode is the sharpest form of the same statement: with derived
    // material switched off entirely, the climbed record is all there is.
    let mut bank = chain.scopes();
    for scope in &mut bank {
        scope.include_derived = false;
    }
    let banked = run(db, tenant, &ComposeRequest::new(bank, 1_500, now));
    assert_eq!(
        ids(&banked),
        vec![runbook],
        "a climbed record survives published-only composition: {}",
        banked.text
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
fn the_plans_tiers_are_what_composes() {
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

    // The plan carries the tiers, so "what composes" is what the PDP said
    // and nothing else (AUTHZ-5, ADR-0038 decision 3). The working-tier
    // plan is what every reader gets with no explicit grant anywhere.
    let internal = ComposeRequest::new(chain.scopes(), 1_500, now);
    let block = run(db, tenant, &internal);
    assert_eq!(block.entries.len(), 2, "public + internal under the floor");

    // A plan that permits every tier composes every tier: there is no
    // clamp here any more, because a clamp is a decision nobody took. What
    // keeps `restricted` out of a real block is the PDP, which needs a
    // compliance-signed lapse to put it in a plan at all.
    let restricted_plan = ComposeRequest::new(chain.scopes_at(&Sensitivity::ALL), 1_500, now);
    let block = run(db, tenant, &restricted_plan);
    let composed = ids(&block);
    assert_eq!(composed.len(), 4, "every tier the plan named");
    let restricted = by_level
        .iter()
        .find(|(level, _)| *level == Sensitivity::Restricted)
        .map(|(_, id)| *id)
        .expect("restricted fixture");
    assert!(composed.contains(&restricted), "including the top tier");
    assert!(
        block.text.contains("[restricted]") && block.text.contains("[confidential]"),
        "and the block says which lines carry them: {}",
        block.text
    );

    // A caller narrowing takes tiers back out, never adds one
    // (ADR-0038 decision 12).
    let narrowed = ComposeRequest::new(chain.scopes_at(&Sensitivity::ALL), 1_500, now)
        .narrowed_to(Sensitivity::Internal);
    let block = run(db, tenant, &narrowed);
    assert_eq!(block.entries.len(), 2, "narrowing is not widening");
}

/// The property a single ceiling could not express, and the reason the
/// predicate is a pair (AUTHZ-5, ADR-0038 decision 3): one scope of a chain
/// admits `confidential` while the next admits only the working tiers —
/// which is exactly what the packs produce, since an explicit binding or
/// one's own home reaches the tier and mere membership does not.
#[test]
fn one_scopes_tier_set_never_leaks_into_another() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();

    let home_confidential = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert {
            sensitivity: Sensitivity::Confidential,
            ..Insert::derived(chain.user, "my own confidential note", now)
        },
    ));
    let team_confidential = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert {
            sensitivity: Sensitivity::Confidential,
            ..Insert::derived(chain.team, "the team's confidential note", now)
        },
    ));
    let team_internal = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.team, "the team's ordinary note", now),
    ));

    // The plan a placed reader with no binding actually gets: confidential
    // at home, the working tiers above it.
    let mut scopes = chain.scopes();
    scopes[0].sensitivities = vec![
        Sensitivity::Public,
        Sensitivity::Internal,
        Sensitivity::Confidential,
    ];
    let block = run(db, tenant, &ComposeRequest::new(scopes, 1_500, now));
    let composed = ids(&block);
    assert!(
        composed.contains(&home_confidential),
        "the reader's own confidential material composes"
    );
    assert!(
        composed.contains(&team_internal),
        "and the team's ordinary material"
    );
    assert!(
        !composed.contains(&team_confidential),
        "but not the team's confidential material, which no grant reached"
    );
    // The one that did compose says what it is, so the harness knows what
    // it is holding (decision 11).
    assert!(
        block.text.contains("[confidential]"),
        "the tier is marked in the line: {}",
        block.text
    );
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

// ── Retention and staleness (MEM-6, ADR-0040) ────────────────────────────────

/// The read cut: a scope's own pack decides what that scope serves, and it
/// decides it in the query that asks (ADR-0040 decision 2). Nothing is
/// stamped on a record, so the same corpus composes differently under two
/// plans built a line apart.
#[test]
fn a_scopes_horizon_removes_its_own_material_from_the_block() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let fresh = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "a fact from yesterday", now - Duration::days(1)),
    ));
    let old = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(
            chain.user,
            "a fact from last year",
            now - Duration::days(300),
        ),
    ));

    let both = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));
    assert_eq!(
        ids(&both),
        vec![fresh, old],
        "under the product default nothing expires"
    );

    let mut scoped = chain.scopes();
    scoped[0].retention = RetentionConfig {
        ttl: ClassTtl {
            fact: 90,
            ..ClassTtl::KEEP
        },
        ..RetentionConfig::DEFAULT
    };
    let narrowed = run(db, tenant, &ComposeRequest::new(scoped, 1_500, now));
    assert_eq!(
        ids(&narrowed),
        vec![fresh],
        "the horizon removed the old fact and nothing else"
    );
}

/// Pinned material is exempt from the read cut — seed §4.2, and a clause in
/// the candidate query rather than a pack setting (ADR-0040 decision 8).
#[test]
fn a_horizon_never_reaches_pinned_material() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let ancient_pin = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.user, "canonical, and old", now - Duration::days(900)),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "derived, and old", now - Duration::days(900)),
    ));

    let mut scoped = chain.scopes();
    scoped[0].retention = RetentionConfig {
        ttl: ClassTtl {
            fact: 30,
            ..ClassTtl::KEEP
        },
        ..RetentionConfig::DEFAULT
    };
    let block = run(db, tenant, &ComposeRequest::new(scoped, 1_500, now));
    assert_eq!(
        ids(&block),
        vec![ancient_pin],
        "the derived record went; the pinned one of the same age did not"
    );
}

/// Staleness decays *relevance*, and only within a gradient position: a
/// stale record two ranks ahead of a fresh one loses its place, and the
/// budget then drops it rather than the fresh one (ADR-0040 decision 12).
///
/// Both records live at the same scope, so seed §4.4's ordering is not in
/// play here — which is the point: freshness reorders inside a position and
/// never across one.
#[test]
fn staleness_ages_a_ranked_record_out_of_its_place() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let stale = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "the stale answer", now - Duration::days(720)),
    ));
    let fresh = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "the fresh answer", now - Duration::days(1)),
    ));

    // The hybrid engine ranked the stale record first.
    let ranked = |scopes: Vec<ComposeScope>| ComposeRequest {
        relevance: Some(vec![stale, fresh]),
        ..ComposeRequest::new(scopes, 1_500, now)
    };

    // With no half-life the rank is the order, exactly as before MEM-6.
    let mut undecayed = chain.scopes();
    undecayed[0].retention = RetentionConfig {
        staleness_half_life_days: 0,
        ..RetentionConfig::DEFAULT
    };
    let block = run(db, tenant, &ranked(undecayed));
    assert_eq!(ids(&block), vec![stale, fresh], "rank alone decides");

    // With one, two years of silence costs the stale record its place.
    let mut decayed = chain.scopes();
    decayed[0].retention = RetentionConfig {
        staleness_half_life_days: 90,
        ..RetentionConfig::DEFAULT
    };
    let block = run(db, tenant, &ranked(decayed));
    assert_eq!(
        ids(&block),
        vec![fresh, stale],
        "the fresh record composes first"
    );
    let scores: Vec<u16> = block
        .entries
        .iter()
        .map(|entry| entry.staleness_permille)
        .collect();
    assert!(
        scores[0] >= 990,
        "one day at a 90-day half-life is all but fresh: {scores:?}"
    );
    assert!(
        scores[1] < 50,
        "two years at a 90-day half-life is nearly nothing left: {scores:?}"
    );
}

/// A MEM-5 merge is the staleness clock's other input: retention runs from
/// first assertion and staleness from last, so a fact somebody restated
/// yesterday scores fresh even though its window opened long ago (ADR-0040
/// decisions 3 and 12).
#[test]
fn a_restatement_refreshes_the_staleness_clock_without_moving_the_retention_one() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let old = now - Duration::days(400);
    let id = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "a fact stated long ago", old),
    ));
    // Exactly what `records::reinforce` writes when a restatement is
    // absorbed (ADR-0039 decision 10).
    db.rt.block_on(async {
        let mut tx = rls::begin_tenant_tx(&db.pool, tenant)
            .await
            .expect("tenant tx");
        records::reinforce(
            &mut *tx,
            tenant,
            id,
            synveda_types::ObserveEventId::new(),
            now - Duration::days(1),
        )
        .await
        .expect("reinforce");
        tx.commit().await.expect("commit");
    });

    let mut scoped = chain.scopes();
    scoped[0].retention = RetentionConfig {
        staleness_half_life_days: 90,
        ttl: ClassTtl {
            fact: 200,
            ..ClassTtl::KEEP
        },
        ..RetentionConfig::DEFAULT
    };
    let block = run(db, tenant, &ComposeRequest::new(scoped, 1_500, now));
    assert!(
        block.entries.is_empty(),
        "the retention clock did not move: 400 days old under a 200-day \
         horizon is still expired, however often it was restated"
    );

    let mut kept = chain.scopes();
    kept[0].retention = RetentionConfig {
        staleness_half_life_days: 90,
        ..RetentionConfig::DEFAULT
    };
    let block = run(db, tenant, &ComposeRequest::new(kept, 1_500, now));
    assert!(
        block.entries[0].staleness_permille >= 990,
        "and the staleness clock did: last asserted yesterday, so it scores \
         as a day old rather than as four hundred ({})",
        block.entries[0].staleness_permille
    );
}

// ── The index tier (CTX-4, ADR-0041) ─────────────────────────────────────────

/// A helper that narrows every planned scope's index configuration —
/// the pack knob of ADR-0041 decision 11, applied at the fixture level.
fn with_index(
    mut scopes: Vec<ComposeScope>,
    tier: IndexTier,
    entry_chars: u32,
) -> Vec<ComposeScope> {
    for scope in &mut scopes {
        scope.index_tier = tier;
        scope.index_entry_chars = entry_chars;
    }
    scopes
}

/// A record long enough that naming it is cheaper than showing it.
fn long_content(marker: &str) -> String {
    format!("{marker} {}", "a paragraph of runbook prose ".repeat(40))
}

/// The feature, at the composition seam: material that does not fit the
/// budget is **named** rather than dropped in silence, and the name comes
/// with the handle that fetches the rest (ADR-0041 decisions 2 and 3).
#[test]
fn material_that_does_not_fit_is_named_rather_than_dropped() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let near = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers pnpm", now),
    ));
    let runbook = long_content("payments incident runbook:");
    let far = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, &runbook, now),
    ));

    // Room for the near record in full and the far one only by name.
    let budget = 220;
    let block = run(
        db,
        tenant,
        &ComposeRequest::new(chain.scopes(), budget, now),
    );

    assert_eq!(
        ids(&block),
        vec![near, far],
        "both records reached the block; the gradient is unchanged"
    );
    assert_eq!(
        block.entries[0].tier,
        EntryTier::Body,
        "the near one in full"
    );
    assert_eq!(
        block.entries[1].tier,
        EntryTier::Index,
        "the far one by name — the whole feature"
    );
    assert_eq!(block.index_entries, 1);
    assert_eq!(
        block.skipped_over_budget, 0,
        "nothing was dropped in silence, which is the defect CTX-4 fixes"
    );
    assert!(
        block.text.contains(&format!("(recall {far})")),
        "the index line carries the handle that fetches the body:\n{}",
        block.text
    );
    assert!(
        block.text.contains('…'),
        "and says it is elided:\n{}",
        block.text
    );
    assert!(
        !block.text.contains(runbook.trim_end()),
        "the body itself never composed"
    );
    assert!(
        block.text.contains("synveda recall"),
        "and the block says how to navigate:\n{}",
        block.text
    );
    // An index entry is a disclosure, so it is watermarked like any other
    // (ADR-0041 decision 10).
    assert!(
        block.text.contains(&far.to_string()),
        "the named record is in the watermark"
    );
    assert!(
        block.tokens <= budget,
        "the budget still bounds the block: {} > {budget}",
        block.tokens
    );
    // The measurement the AC asks for, produced by the product rather
    // than re-derived by the test (decision 14).
    assert_eq!(
        block.index_tokens,
        block.entries[1].tokens
            + estimated_tokens(
                "Summarised entries end with a recall handle; \
                 `synveda recall <id>` fetches the full text.\n",
            ),
        "the index tier's cost is its lines plus the legend it had to place"
    );
}

/// ADR-0041 decision 2, which is what keeps a mechanism built for assets
/// that do not exist yet from making today's corpus worse: a record short
/// enough that naming it costs what showing it costs is **not** demoted.
/// It is skipped, exactly as it was before CTX-4.
#[test]
fn a_short_record_is_never_demoted() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let near = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers pnpm", now),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        // Short — the shape MEM-3's write-time summarisation produces.
        Insert::derived(chain.org, "the org uses UTC in logs", now),
    ));

    // Room for the first entry and nothing else. Raised from 80 to 94 on
    // 2026-07-31 (EVAL-5, ADR-0048 decision 10): the preamble gained the
    // line saying its entries are recorded material and not instructions,
    // which is 14 estimated tokens of fixed overhead. What this test is
    // about is ENTRY room — one entry fits, the next does not, and the
    // short one is skipped rather than demoted — so the budget moves with
    // the overhead or the test starts measuring the preamble. It caught
    // the change by failing: at 80 the nearest scope's section header no
    // longer fitted and the block composed the org record instead, which
    // is first-fit working and this assertion measuring something else.
    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 94, now));

    assert_eq!(ids(&block), vec![near]);
    assert_eq!(
        block.index_entries, 0,
        "demoting a short record would spend budget to say less"
    );
    assert_eq!(block.skipped_over_budget, 1);
    assert!(
        !block.text.contains("(recall "),
        "and no legend or handle was paid for:\n{}",
        block.text
    );
}

/// Decision 11: a pack that turns the tier off gets the pre-CTX-4 product
/// back — the same corpus, the same budget, the long record dropped in
/// silence and counted, with no legend and no handle anywhere.
///
/// And the other half, which is the one that keeps every CTX-2 test
/// honest: where nothing demotes, `off` and `demote` compose the same
/// bytes.
#[test]
fn the_tier_off_composes_what_ctx_2_composed() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers pnpm", now),
    ));
    let far = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, &long_content("runbook:"), now),
    ));

    let off = run(
        db,
        tenant,
        &ComposeRequest::new(with_index(chain.scopes(), IndexTier::Off, 320), 220, now),
    );
    assert!(!ids(&off).contains(&far), "the long record did not compose");
    assert_eq!(off.skipped_over_budget, 1, "it was dropped and counted");
    assert_eq!(off.index_entries, 0);
    assert_eq!(off.index_tokens, 0);
    assert!(!off.text.contains("(recall "));
    assert!(!off.text.contains("synveda recall"));

    // The same plan with the tier on, over a corpus with nothing to
    // demote, is byte-identical: the feature costs nothing where it does
    // nothing.
    let short_only = Chain::new();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(short_only.user, "alice prefers pnpm", now),
    ));
    let quiet_off = run(
        db,
        tenant,
        &ComposeRequest::new(
            with_index(short_only.scopes(), IndexTier::Off, 320),
            1_500,
            now,
        ),
    );
    let quiet_on = run(
        db,
        tenant,
        &ComposeRequest::new(
            with_index(short_only.scopes(), IndexTier::Demote, 320),
            1_500,
            now,
        ),
    );
    assert_eq!(
        quiet_off.text, quiet_on.text,
        "a block with nothing to demote is the block CTX-2 rendered"
    );
    assert_eq!(quiet_off.block_hash, quiet_on.block_hash);
}

/// An index entry is the same trust statement as a body, rendered
/// shallower: the unreviewed marker and the tier marker both survive the
/// elision (ADR-0041 decision 3, and its compliance note).
#[test]
fn an_index_entry_keeps_every_trust_marker() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert {
            sensitivity: Sensitivity::Confidential,
            ..Insert::derived(chain.user, &long_content("the acquisition memo:"), now)
        },
    ));

    let block = run(
        db,
        tenant,
        &ComposeRequest::new(
            chain.scopes_at(&[
                Sensitivity::Public,
                Sensitivity::Internal,
                Sensitivity::Confidential,
            ]),
            200,
            now,
        ),
    );

    assert_eq!(block.index_entries, 1, "it composed by name");
    let line = block
        .text
        .lines()
        .find(|line| line.starts_with("- ["))
        .expect("an entry line");
    assert!(
        line.contains("[confidential]"),
        "a harness cannot know what it is holding unless the block says so — \
         and that holds for a name as much as for a body: {line}"
    );
    assert!(
        line.contains("[unreviewed]"),
        "nobody published it, elision notwithstanding: {line}"
    );
    assert!(line.contains("(recall "), "and it is navigable: {line}");
}

/// Decision 1, stated as a test rather than a paragraph: the index tier
/// renders the permitted set, it never widens it. Material above the
/// tiers the plan permits is not named, not elided, and not hinted at —
/// it is simply not there, exactly as before CTX-4.
#[test]
fn the_index_tier_never_names_what_the_plan_excluded() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    let readable = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers pnpm", now),
    ));
    let secret = db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert {
            sensitivity: Sensitivity::Confidential,
            ..Insert::derived(chain.user, &long_content("the acquisition memo:"), now)
        },
    ));

    // A working-tier plan: `confidential` was never permitted here.
    let block = run(db, tenant, &ComposeRequest::new(chain.scopes(), 1_500, now));

    assert_eq!(ids(&block), vec![readable]);
    assert_eq!(block.index_entries, 0, "nothing was named");
    assert!(
        !block.text.contains(&secret.to_string()),
        "not even the id, which would be a handle to ask for it:\n{}",
        block.text
    );
    assert!(!block.text.contains("acquisition"));
}

/// The CTX-2 acceptance criterion — byte-identical re-composition at the
/// same instant — asserted with the index tier actually demoting, because
/// a determinism proof over a path the feature does not take proves
/// nothing about the feature.
#[test]
fn determinism_holds_while_the_tier_demotes() {
    let Some(db) = db() else { return };
    let tenant = admit(db);
    let chain = Chain::new();
    let now = Utc::now();
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::derived(chain.user, "alice prefers pnpm", now),
    ));
    db.rt.block_on(insert(
        &db.pool,
        tenant,
        Insert::pinned(chain.org, &long_content("runbook:"), now),
    ));

    let request = ComposeRequest::new(chain.scopes(), 220, now);
    let first = run(db, tenant, &request);
    let second = run(db, tenant, &request);

    assert_eq!(first.index_entries, 1, "the tier is doing something");
    assert_eq!(first.text, second.text, "byte-identical");
    assert_eq!(first.block_hash, second.block_hash);
    assert_eq!(first.index_tokens, second.index_tokens);
}

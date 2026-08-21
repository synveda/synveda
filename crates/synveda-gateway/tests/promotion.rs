//! FLOW-4's acceptance criteria (ADR-0033): **a rule fires in a soak
//! test**, and **the proposal it opens carries evidence for the
//! reviewer** — over the product's own surfaces, against a live Postgres.
//!
//! The signal is real throughout. Nothing here writes a usage counter:
//! every recall in these tests is an actual `POST /v1/inject` that
//! composed the record into a block and chained a `context.injected`
//! event, and the engine folds those events out of the audit chain
//! exactly as it does in production. That matters for the AC's second
//! half — the evidence claims a count over an audit range, and
//! `evidence_is_checkable_against_the_chain` re-derives that count from
//! the chain itself rather than trusting the projection that produced it.
//!
//! Around the AC: the content-address idempotency key (a rejection binds
//! bytes; an edit frees them), the rebuild property that makes the
//! projection derived state, the owner's `ProposalOpen` decision standing
//! between a rule and a proposal, and the structural fact ADR-0033
//! decision 8 rests on — that one member's material can never accrue a
//! second member, because composition never leaves the caller's own
//! chain.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder};
use synveda_ingest::promotion::{self, SweepConfig, SweepDeps};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_retrieval::indexer::{self, IndexerConfig};
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{access, identities, policy_packs, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, Identity, IdentityId, IdentityKind, PackConfig, PromotionConfig, PromotionRule,
    RecordClass, RecordId, RecordKind, ScopeId, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"flow-4-test-secret";

/// A pack that permits within the tenant, so these tests exercise the
/// *engine* rather than re-litigating the matrix the FLOW-3 suite already
/// covers. Deliberately blanket: it makes
/// `another_members_recalls_never_reach_a_personal_record` prove the
/// layer that cannot be configured away — composition never leaves the
/// caller's own chain — rather than the pack clause that could.
const BLANKET: &str = "permit (principal, action, resource) when { resource in principal.tenant };";

/// The procedure a member keeps needing — the material a usage rule is
/// meant to notice.
const PROCEDURE: &str = "restart the ingest worker with a drained queue before deploying";

/// A rule that fires on one member's own well-used procedures.
///
/// `min_distinct_members: 1` is not a weakened threshold, it is the only
/// one that can fire on material the write path produces: every record
/// `observe` → extraction writes lands on a user-kind personal leaf —
/// a service identity's included, since ADR-0018 places one "like a
/// user" — and no non-self `MemoryRead` permit in any pack admits a
/// user-kind scope (ADR-0033 decision 8).
fn well_used_procedures() -> PromotionConfig {
    PromotionConfig {
        rules: vec![PromotionRule {
            name: "well-used-procedures".to_owned(),
            asset: synveda_types::AssetKind::Memory,
            classes: vec![RecordClass::Procedure],
            max_sensitivity: Sensitivity::Internal,
            min_recalls: 3,
            min_distinct_members: 1,
            min_age_hours: 0,
            recency_hours: Some(24),
            target_channel: synveda_types::Channel::Published,
        }],
    }
}

// ── Harness ──────────────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-flow4-tests")
        .join(TenantId::new().to_string())
}

fn state_with(url: &str, search_index: Arc<SearchIndex>) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index,
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
        // TEN-4 (ADR-0064): a fixed test KEK, so a suite that touches a
        // sealed column seals rather than skipping. `Kms::Disabled` is the
        // production default when no key is configured.
        keys: std::sync::Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Local(
                synveda_crypto::LocalKms::from_hex(&"11".repeat(32), "local:test")
                    .expect("test kek"),
            ),
        )),
    }
}

/// The engine, sharing the app's PDP and chain cache exactly as the
/// gateway binary wires it.
fn engine(state: &AppState) -> SweepDeps {
    SweepDeps {
        pool: state.pool.clone(),
        pdp: Arc::clone(&state.pdp),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping FLOW-4 promotion test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("flow4-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-4 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// The tenant root with an `eng` org unit and a platform unit under it.
async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Scope {
    let mut tx = pool.begin().await.expect("begin");
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(org.id),
            slug: "eng".to_owned(),
            display_name: "Engineering".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope");
    let platform = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(eng.id),
            slug: "platform".to_owned(),
            display_name: "Platform".to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create scope");
    tx.commit().await.expect("commit hierarchy");
    platform
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

/// One record at `scope`, owned by `owner`, with its vector — the state
/// extraction leaves behind (MEM-4's one-statement API).
async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    class: RecordClass,
    content: &str,
) -> RecordId {
    let embedder = DeterministicEmbedder::new();
    let vector = embedder
        .embed(std::slice::from_ref(&content.to_owned()))
        .await
        .expect("deterministic embed")
        .remove(0);
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: scope,
            owner_id: owner,
            kind: RecordKind::Derived,
            class,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "flow-4 acceptance test"}),
            valid_from: chrono::Utc::now(),
            valid_to: None,
        },
        &RecordEmbedding {
            model: embedder.model().to_owned(),
            vector,
        },
    )
    .await
    .expect("insert record");
    id
}

/// Rewrites a record's content — new bytes, therefore a new content
/// address (ADR-0032 decision 6, ADR-0033 decision 11).
async fn edit_record(pool: &PgPool, id: RecordId, content: &str) {
    let current = records::current(pool, id)
        .await
        .expect("read record")
        .expect("record exists");
    let embedder = DeterministicEmbedder::new();
    let vector = embedder
        .embed(std::slice::from_ref(&content.to_owned()))
        .await
        .expect("deterministic embed")
        .remove(0);
    records::update(
        pool,
        id,
        &RecordState {
            content: content.to_owned(),
            ..current.state
        },
        &RecordEmbedding {
            model: embedder.model().to_owned(),
            vector,
        },
    )
    .await
    .expect("update record");
}

async fn install_rules(pool: &PgPool, tenant: TenantId, state: &AppState, rules: PromotionConfig) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    policy_packs::apply(
        &mut *tx,
        tenant,
        "flow4-promoting",
        BLANKET,
        &PackConfig {
            promotion: Some(rules),
            ..PackConfig::default()
        },
    )
    .await
    .expect("store pack");
    synveda_store::policy_assignments::set_default(&mut *tx, tenant, "flow4-promoting")
        .await
        .expect("make it the tenant default");
    tx.commit().await.expect("commit pack");
    assert_eq!(
        synveda_gateway::authz::refresh_tenant_packs(&state.pool, &state.pdp, tenant).await,
        "installed",
        "the pack must be in force before the engine reads its rules"
    );
}

async fn converge(pool: &PgPool, index: &SearchIndex, tenant: TenantId) {
    let config = IndexerConfig {
        overlap: Duration::ZERO,
        ..IndexerConfig::default()
    };
    indexer::sweep_tenant(pool, index, tenant, &config)
        .await
        .expect("sweep sidecar");
}

/// One real inject: composes a block and chains one `context.injected`.
async fn inject(app: &Router, token: &str, session: &str) -> Value {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/inject")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"task": "ingest worker deploy", "session_id": session}).to_string(),
        ))
        .expect("build request");
    let response = app.clone().oneshot(request).await.expect("route responds");
    assert_eq!(response.status(), StatusCode::OK, "inject must succeed");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

/// `count` real injects as `subject`, each its own session.
async fn recall_times(app: &Router, tenant: TenantId, subject: &str, count: usize) {
    let token = issue(subject, tenant);
    for round in 0..count {
        inject(app, &token, &format!("{subject}-sess-{round}")).await;
    }
}

async fn api(app: &Router, method: &str, uri: &str, token: &str, body: Option<Value>) -> Value {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(value) => request
            .header("content-type", "application/json")
            .body(Body::from(value.to_string())),
        None => request.body(Body::empty()),
    }
    .expect("build request");
    let response = app.clone().oneshot(request).await.expect("route responds");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    }
}

/// The tenant's audit events of one action, oldest first.
async fn events(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read audit tail");
    all.reverse();
    all.retain(|event| event.action == action);
    all
}

async fn open_proposals(pool: &PgPool, tenant: TenantId) -> Vec<synveda_vedaflow::StoredProposal> {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant tx");
    synveda_vedaflow::proposals::list(
        &mut tx,
        tenant,
        synveda_vedaflow::ProposalFilter {
            limit: 100,
            ..Default::default()
        },
    )
    .await
    .expect("list proposals")
}

/// Drives one pass of the engine over **this tenant only**, and reports
/// how many proposals it opened.
///
/// `run_tenant` rather than `run_once` deliberately: this suite shares a
/// database with every other suite, and a cross-tenant pass from a
/// neighbouring test would fold this tenant's events out from under it —
/// the sweeper that folds is the one that evaluates, so the fold has to
/// belong to the test that asserts on it.
async fn run_engine(fx: &Fixture) -> usize {
    run_engine_for(&fx.state, fx.tenant).await
}

async fn run_engine_for(state: &AppState, tenant: TenantId) -> usize {
    promotion::run_tenant(&engine(state), &SweepConfig::default(), tenant)
        .await
        .expect("promotion pass")
        .proposals_opened
}

/// The full fixture: a team, one member, one well-used procedure, the
/// rules installed, and the sidecar converged.
struct Fixture {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    state: AppState,
    alice: Identity,
    record: RecordId,
}

async fn fixture() -> Option<Fixture> {
    let (pool, tenant) = admitted_tenant().await?;
    let _platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    // The material lives on alice's own personal leaf, which is where
    // extraction puts everything it produces.
    let record = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordClass::Procedure,
        PROCEDURE,
    )
    .await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    converge(&pool, &index, tenant).await;
    let state = state_with(&database_url(), index);
    let app = router(state.clone());
    install_rules(&pool, tenant, &state, well_used_procedures()).await;
    Some(Fixture {
        pool,
        tenant,
        app,
        state,
        alice,
        record,
    })
}

// ── The acceptance criteria ──────────────────────────────────────────────────

/// **AC (both halves).** Sustained real use — one member, one procedure,
/// enough injects to cross the threshold — makes a rule fire without
/// anybody deciding to, and the proposal it opens carries the usage stats
/// that justify it.
#[tokio::test]
async fn a_rule_fires_on_real_use_and_its_proposal_carries_evidence() {
    let Some(fx) = fixture().await else { return };

    // Below the threshold, nothing fires: two recalls against min 3.
    recall_times(&fx.app, fx.tenant, "alice", 2).await;
    assert_eq!(
        run_engine(&fx).await,
        0,
        "two recalls is under the rule's threshold of three"
    );
    assert!(
        open_proposals(&fx.pool, fx.tenant).await.is_empty(),
        "nothing may be proposed below the threshold"
    );

    // The third recall crosses it.
    recall_times(&fx.app, fx.tenant, "alice", 1).await;
    assert_eq!(
        run_engine(&fx).await,
        1,
        "the third recall crosses the threshold and the rule fires"
    );

    let proposals = open_proposals(&fx.pool, fx.tenant).await;
    assert_eq!(proposals.len(), 1, "exactly one proposal, batched");
    let proposal = &proposals[0];
    assert_eq!(
        proposal.target_scope_id, fx.alice.scope_id,
        "FLOW-4 is same-scope: the target is where the material already lives"
    );
    assert_eq!(
        proposal.source_scope_id, proposal.target_scope_id,
        "source and target are the same node until FLOW-5"
    );
    assert_eq!(
        proposal.proposer_id, fx.alice.id,
        "the proposal rides the material owner's authority, not a system principal"
    );

    // The AC's second half: the evidence a reviewer reads.
    let evidence = proposal
        .evidence
        .as_ref()
        .expect("an auto-opened proposal carries evidence");
    assert_eq!(evidence.rule, "well-used-procedures");
    assert_eq!(evidence.pack_name, "flow4-promoting");
    assert_eq!(
        evidence.actions,
        vec!["context.injected".to_owned(), "context.recalled".to_owned()],
        "the set grew when CTX-5 landed (ADR-0042 decision 16), and the evidence \
         says so — which is what keeps a proposal opened before that day from \
         reading as though it had counted an explicit recall"
    );
    assert_eq!(evidence.members.len(), 1);
    let member = &evidence.members[0];
    assert_eq!(member.record_id, fx.record);
    assert_eq!(member.recalls, 3, "three injects, three recalls");
    assert_eq!(
        member.distinct_members, 1,
        "one member recalled it — the only count this material can reach"
    );
    assert!(
        evidence.from_seq >= 1 && evidence.to_seq >= evidence.from_seq,
        "the evidence names the chain range it was folded from: {evidence:?}"
    );

    // The reviewer's surface shows it. Alice holds no review role, so
    // the curator reads it — the person who would act on it.
    seed_user(&fx.pool, fx.tenant, "carol").await;
    role_bindings_bind(
        &fx.pool,
        fx.tenant,
        "carol",
        fx.alice.scope_id,
        RoleKey::Curator,
    )
    .await;
    let detail = api(
        &fx.app,
        "GET",
        &format!("/v1/proposals/{}", proposal.id),
        &issue("carol", fx.tenant),
        None,
    )
    .await;
    assert_eq!(
        detail["promotion"]["rule"], "well-used-procedures",
        "the reviewer sees why this was raised: {detail}"
    );
    assert_eq!(detail["promotion"]["members"][0]["recalls"], 3);
    assert_eq!(
        detail["title"],
        "auto-promotion (well-used-procedures): 1 record, 3 recalls by up to 1 member",
        "the title reads as a summary in a queue: {detail}"
    );

    // One `ProposalOpened`, attributed to the component that acted, with
    // the owner whose authority it rode named in the payload.
    let opened = events(&fx.pool, fx.tenant, "vedaflow.proposal.opened").await;
    assert_eq!(opened.len(), 1, "exactly one open event");
    assert_eq!(
        opened[0].actor_kind, "system",
        "a rule opened it, and the trail says so"
    );
    assert_eq!(opened[0].actor_subject, "promotion");
    assert_eq!(
        opened[0].payload["proposer"]["subject"], "alice",
        "whose authority it rode is a different fact from who acted: {}",
        opened[0].payload
    );
    assert!(
        opened[0].payload["approvals"]["required"].is_object(),
        "the requirement as resolved is recorded exactly as a human's open records it: {}",
        opened[0].payload
    );
    assert_eq!(
        opened[0].payload["promotion"]["rule"], "well-used-procedures",
        "the evidence is under the chain's hash too"
    );
}

/// **The evidence is checkable, not merely present** (ADR-0033 decision
/// 4). A reviewer who does not believe the count re-derives it from the
/// hash-chained events in the range the evidence names — which is what
/// this test does, without consulting the projection at all.
#[tokio::test]
async fn evidence_is_checkable_against_the_chain() {
    let Some(fx) = fixture().await else { return };
    recall_times(&fx.app, fx.tenant, "alice", 4).await;
    run_engine(&fx).await;

    let proposals = open_proposals(&fx.pool, fx.tenant).await;
    let evidence = proposals[0].evidence.as_ref().expect("evidence").clone();
    let claimed = &evidence.members[0];

    // Re-derive the claim from the chain itself.
    let injected = events(&fx.pool, fx.tenant, "context.injected").await;
    let mut counted = 0;
    let mut subjects = std::collections::BTreeSet::new();
    for event in &injected {
        if event.seq < evidence.from_seq || event.seq > evidence.to_seq {
            continue;
        }
        let names_record = event.payload["entries"]
            .as_array()
            .map(|entries| {
                entries.iter().any(|entry| {
                    entry["record_id"].as_str() == Some(&claimed.record_id.to_string())
                })
            })
            .unwrap_or(false);
        if names_record {
            counted += 1;
            subjects.insert(event.actor_subject.clone());
        }
    }
    assert_eq!(
        counted, claimed.recalls,
        "the chain must agree with the evidence's recall count"
    );
    assert_eq!(
        subjects.len() as u64,
        claimed.distinct_members,
        "the chain must agree with the evidence's distinct-member count"
    );
}

/// **A soak does not open the same proposal twice.** The idempotency key
/// is the content address, so repeated passes over growing usage add
/// nothing while the same bytes stand under review (ADR-0033 decision
/// 11) — the property that keeps `MAX_OPEN_PROPOSALS` from being reached
/// by an engine arguing with itself.
#[tokio::test]
async fn a_soak_never_opens_the_same_bytes_twice() {
    let Some(fx) = fixture().await else { return };
    recall_times(&fx.app, fx.tenant, "alice", 3).await;
    assert_eq!(run_engine(&fx).await, 1);

    // Ten more passes over ten more recalls: the material keeps
    // qualifying, and keeps being the same bytes already under review.
    for _ in 0..10 {
        recall_times(&fx.app, fx.tenant, "alice", 1).await;
        assert_eq!(
            run_engine(&fx).await,
            0,
            "the same bytes must not be proposed while a proposal stands open"
        );
    }
    assert_eq!(
        open_proposals(&fx.pool, fx.tenant).await.len(),
        1,
        "one proposal after a soak, not eleven"
    );
}

/// **A rejection binds bytes; an edit frees them** (ADR-0033 decision
/// 11). A human's "no" is durable against an engine that would otherwise
/// ask again on the next pass — and it is durable *about those bytes*,
/// so revised material is a new review rather than a suppressed one.
#[tokio::test]
async fn a_rejection_binds_bytes_and_an_edit_frees_them() {
    let Some(fx) = fixture().await else { return };
    seed_user(&fx.pool, fx.tenant, "carol").await;
    role_bindings_bind(
        &fx.pool,
        fx.tenant,
        "carol",
        fx.alice.scope_id,
        RoleKey::Curator,
    )
    .await;
    let carol = issue("carol", fx.tenant);

    recall_times(&fx.app, fx.tenant, "alice", 3).await;
    run_engine(&fx).await;
    let proposal = open_proposals(&fx.pool, fx.tenant).await.remove(0);

    let rejected = api(
        &fx.app,
        "POST",
        &format!("/v1/proposals/{}/reject", proposal.id),
        &carol,
        Some(json!({"reason": "not canonical yet — the queue drain step is wrong"})),
    )
    .await;
    assert_eq!(
        rejected["state"], "rejected",
        "the curator's rejection must land: {rejected}"
    );

    recall_times(&fx.app, fx.tenant, "alice", 3).await;
    assert_eq!(
        run_engine(&fx).await,
        0,
        "re-proposing bytes a reviewer refused is the pile-up the cap exists to survive"
    );

    // The same record, revised: different bytes, therefore a new review.
    edit_record(
        &fx.pool,
        fx.record,
        "restart the ingest worker only after the queue reports zero visible messages",
    )
    .await;
    recall_times(&fx.app, fx.tenant, "alice", 3).await;
    assert_eq!(
        run_engine(&fx).await,
        1,
        "an edited record is new material and may be proposed again"
    );
}

/// **The projection is derived state** (ADR-0033 decision 3): discard it
/// and the watermark, refold the chain from seq 1, and the counts come
/// back identical. That is what lets it carry no audit trail of its own.
#[tokio::test]
async fn the_projection_rebuilds_from_the_chain() {
    let Some(fx) = fixture().await else { return };
    recall_times(&fx.app, fx.tenant, "alice", 5).await;
    run_engine(&fx).await;

    let before = {
        let mut tx = rls::begin_tenant_tx(&fx.pool, fx.tenant)
            .await
            .expect("begin");
        synveda_store::promotion::usage_for(&mut *tx, fx.tenant, &[fx.record])
            .await
            .expect("read usage")
    };
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].recalls, 5);

    {
        let mut tx = rls::begin_tenant_tx(&fx.pool, fx.tenant)
            .await
            .expect("begin");
        synveda_store::promotion::reset(&mut tx, fx.tenant)
            .await
            .expect("discard the projection");
        tx.commit().await.expect("commit reset");
    }

    run_engine(&fx).await;
    let after = {
        let mut tx = rls::begin_tenant_tx(&fx.pool, fx.tenant)
            .await
            .expect("begin");
        synveda_store::promotion::usage_for(&mut *tx, fx.tenant, &[fx.record])
            .await
            .expect("read usage")
    };
    assert_eq!(
        after, before,
        "a rebuild from the chain must reproduce the projection exactly"
    );
}

/// **An unconfigured pack promotes nothing.** Unlike the approval matrix,
/// whose fail-safe is the invariant floor, a trigger's fail-safe is
/// silence — and no embedded pack carries rules, so auto-promotion never
/// arrives through an upgrade (ADR-0033 decision 6).
#[tokio::test]
async fn an_unconfigured_pack_promotes_nothing() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let _platform = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice").await;
    let record = seed_record(
        &pool,
        tenant,
        alice.scope_id,
        alice.id,
        RecordClass::Procedure,
        PROCEDURE,
    )
    .await;
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    converge(&pool, &index, tenant).await;
    let state = state_with(&database_url(), index);
    let app = router(state.clone());

    recall_times(&app, tenant, "alice", 10).await;
    assert_eq!(
        run_engine_for(&state, tenant).await,
        0,
        "the product's own packs carry no promotion rules"
    );
    let usage = {
        let mut tx = rls::begin_tenant_tx(&pool, tenant).await.expect("begin");
        synveda_store::promotion::usage_for(&mut *tx, tenant, &[record])
            .await
            .expect("read usage")
    };
    assert_eq!(
        usage.first().map(|row| row.recalls),
        Some(10),
        "usage is still swept — the projection is not gated on rules, \
         so turning rules on later sees the history"
    );
}

/// **One member's material can never accrue a second member** — the
/// structural fact ADR-0033 decision 8 rests on, pinned so a future
/// change to the read path cannot quietly invalidate the ADR.
///
/// Composition never leaves the caller's own chain
/// (`permitted_chain_scopes` decides `MemoryRead` once per *chain node*),
/// and under that, no non-self permit in any pack admits a user-kind
/// scope. Bob injecting all day cannot make Alice's note a shared fact.
#[tokio::test]
async fn another_members_recalls_never_reach_a_personal_record() {
    let Some(fx) = fixture().await else { return };
    // Same team as alice: the closest bob can structurally get.
    let bob = seed_user(&fx.pool, fx.tenant, "bob").await;
    assert_ne!(bob.scope_id, fx.alice.scope_id);

    recall_times(&fx.app, fx.tenant, "alice", 2).await;
    recall_times(&fx.app, fx.tenant, "bob", 20).await;
    run_engine(&fx).await;

    let usage = {
        let mut tx = rls::begin_tenant_tx(&fx.pool, fx.tenant)
            .await
            .expect("begin");
        synveda_store::promotion::usage_for(&mut *tx, fx.tenant, &[fx.record])
            .await
            .expect("read usage")
    };
    assert_eq!(usage.len(), 1);
    assert_eq!(
        usage[0].distinct_members, 1,
        "twenty injects by a teammate added no member to a personal record's usage"
    );
    assert_eq!(
        usage[0].recalls, 2,
        "and added no recalls to it either — it was never a candidate to compose"
    );
}

/// **A quarantined owner proposes nothing.** The rule engine re-decides
/// `ProposalOpen` as the owner, under their *current* state, so the
/// quarantine that stops them writing stops the engine acting for them —
/// the MEM-3 property, one action over (ADR-0033 decision 9).
#[tokio::test]
async fn a_quarantined_owner_proposes_nothing() {
    let Some(fx) = fixture().await else { return };
    recall_times(&fx.app, fx.tenant, "alice", 3).await;

    // Quarantine is departure now — the one shape it takes (CPR-7,
    // ADR-0074 decision 3): a sealed identity, refused by the base
    // layer's forbid.
    {
        let mut tx = rls::begin_tenant_tx(&fx.pool, fx.tenant)
            .await
            .expect("begin");
        identities::depart(&mut tx, fx.tenant, fx.alice.id)
            .await
            .expect("depart alice");
        tx.commit().await.expect("commit departure");
    }

    assert_eq!(
        run_engine(&fx).await,
        0,
        "a rule cannot propose what its owner could not"
    );
    assert!(open_proposals(&fx.pool, fx.tenant).await.is_empty());
}

/// The CLI break-glass shape: a direct grant write, silent in the chain.
async fn role_bindings_bind(
    pool: &PgPool,
    tenant: TenantId,
    subject: &str,
    scope: ScopeId,
    role: RoleKey,
) {
    let mut tx = pool.begin().await.expect("begin");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: role,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
}

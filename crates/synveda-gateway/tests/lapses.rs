//! AUTHZ-4's acceptance criterion (ADR-0037): **a lapse grants cross-team
//! read, expiry restores denial, and the audit shows the full story** —
//! over the product's own HTTP surfaces, under the PDP, against a live
//! Postgres.
//!
//! The walk is the feature's own sentence made true, and it is asserted
//! from the *reader's* side because that is what makes it a grant rather
//! than a row: a payments engineer who cannot read the platform team's
//! material receives it in her own `POST /v1/inject` while a two-steward
//! lapse stands, and stops receiving it when the window closes — with
//! nobody revoking anything, no restart, and nothing to wait out but the
//! clock.
//!
//! **The expiry here is real.** The lapse runs for a few seconds and the
//! test sleeps through it. There is no injected clock and no simulated
//! instant: a duration is seconds with no minimum precisely so this
//! assertion can be about the product rather than about a fixture
//! (ADR-0037 decision 4).
//!
//! Around the AC: the refusals a proposer meets in the product's own words
//! (a personal-scope target, a target the grantee already composes, an
//! action outside the vocabulary, a window past the pack's ceiling), the
//! separation that makes the two Cedar actions worth having (a
//! security-reviewer revokes what they could never grant), and the two
//! honest limits — a lapse discloses only what the target *published*, and
//! one steward alone cannot open the door.
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
use synveda_audit::StoredEvent;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Role,
    ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"authz-4-test-secret";

/// The platform team's material — what a lapse discloses, and only because
/// platform published it.
const RUNBOOK: &str = "page the on-call via the incident bridge, never by direct message";

/// The window the AC's lapse runs for. Short enough that a test can wait
/// it out, long enough that the assertions before it are not racing.
const WINDOW_SECS: u32 = 4;

// ── Harness ──────────────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-authz4-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open sidecar"),
        ),
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

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

async fn admitted_tenant() -> Option<(PgPool, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping AUTHZ-4 lapse test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("authz4-{}", id.as_uuid().simple());
    tenants::create(
        &pool,
        id,
        &slug,
        "AUTHZ-4 test tenant",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// The org the lapse happens in:
///
/// ```text
/// acme (org)
/// └── eng (department)
///     ├── platform (team)   ← the runbook lives and is published here
///     └── payments (team)   ← reads it only while a lapse stands
/// ```
struct Org {
    org: HierarchyNode,
    eng: HierarchyNode,
    platform: HierarchyNode,
    payments: HierarchyNode,
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    let seeded = {
        let mut node = async |parent: Option<ScopeId>, kind, slug: &str, name: &str| {
            hierarchy::create(&mut tx, ScopeId::new(), tenant, parent, kind, slug, name)
                .await
                .expect("create node")
        };
        let org = node(None, ScopeKind::Org, "acme", "ACME").await;
        let eng = node(Some(org.id), ScopeKind::Department, "eng", "Engineering").await;
        let platform = node(Some(eng.id), ScopeKind::Team, "platform", "Platform").await;
        let payments = node(Some(eng.id), ScopeKind::Team, "payments", "Payments").await;
        node(
            Some(org.id),
            ScopeKind::Team,
            identities::QUARANTINE_SLUG,
            "Quarantine",
        )
        .await;
        Org {
            org,
            eng,
            platform,
            payments,
        }
    };
    tx.commit().await.expect("commit hierarchy");
    seeded
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str, parent: ScopeId) -> Identity {
    let mut tx = pool.begin().await.expect("begin");
    let id = IdentityId::new();
    let leaf = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(parent),
        ScopeKind::User,
        &personal_slug(None, subject, id),
        subject,
    )
    .await
    .expect("create personal scope");
    let identity = identities::create(
        &mut tx,
        id,
        tenant,
        Some(subject),
        IdentityKind::User,
        None,
        None,
        leaf.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit user");
    identity
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: ScopeId, role: Role) {
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, subject, Some(scope), role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
) -> RecordId {
    let id = RecordId::new();
    records::insert(
        pool,
        id,
        tenant,
        &RecordState {
            scope_id: scope,
            owner_id: owner,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "authz-4 acceptance test"}),
            valid_from: chrono::Utc::now() - chrono::Duration::hours(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("insert record");
    id
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, body)
}

async fn post(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

async fn get(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");
    call(app, request).await
}

async fn inject(app: &Router, token: &str) -> Value {
    let (status, body) = post(app, token, "/v1/inject", json!({"session_id": "authz-4"})).await;
    assert_eq!(status, StatusCode::OK, "inject failed: {body}");
    body
}

/// The caller's own composed block, as text.
async fn block(app: &Router, token: &str) -> String {
    inject(app, token).await["text"]
        .as_str()
        .expect("block text")
        .to_owned()
}

async fn events(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read chain");
    all.reverse();
    all.into_iter()
        .filter(|event| event.action == action)
        .collect()
}

/// Opens a lapse proposal.
async fn propose(
    app: &Router,
    token: &str,
    target: ScopeId,
    grantee: ScopeId,
    duration_secs: u32,
    reason: &str,
) -> (StatusCode, Value) {
    post(
        app,
        token,
        "/v1/lapses",
        json!({
            "scope_id": target,
            "grantee_scope_id": grantee,
            "action": "memory.read",
            "duration_secs": duration_secs,
            "reason": reason,
        }),
    )
    .await
}

/// Opens a lapse proposal that must succeed, returning its proposal id.
async fn proposed(
    app: &Router,
    token: &str,
    target: ScopeId,
    grantee: ScopeId,
    duration_secs: u32,
    reason: &str,
) -> String {
    let (status, body) = propose(app, token, target, grantee, duration_secs, reason).await;
    assert_eq!(status, StatusCode::OK, "opening the lapse failed: {body}");
    body["proposal_id"]
        .as_str()
        .expect("proposal id")
        .to_owned()
}

async fn approve(app: &Router, token: &str, proposal: &str) {
    let (status, body) = post(
        app,
        token,
        &format!("/v1/proposals/{proposal}/approve"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approval failed: {body}");
}

// ── The acceptance criterion ─────────────────────────────────────────────────

/// **A lapse grants cross-team read, expiry restores denial, and the audit
/// shows the full story.**
///
/// Every claim is made from the reader's side. Priya is on payments;
/// `regulated-strict` gives her her own chain and nothing else, so the
/// platform team's runbook is invisible to her — before the lapse and
/// after it, with the identical request in between returning it.
#[tokio::test(flavor = "multi_thread")]
async fn a_lapse_grants_cross_team_read_and_its_expiry_restores_the_denial() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;

    // Platform: an engineer who owns the runbook, and a curator who
    // publishes it. A lapse discloses what the target *stands behind*, so
    // the publication is what gives it anything to disclose at all.
    let sam = seed_user(&pool, tenant, "sam", org.platform.id).await;
    seed_user(&pool, tenant, "cara", org.platform.id).await;
    bind(&pool, tenant, "cara", org.platform.id, Role::Curator).await;
    let runbook = seed_record(&pool, tenant, org.platform.id, sam.id, RUNBOOK).await;
    let cara = issue("cara", tenant);
    let (status, body) = post(
        &app,
        &cara,
        &format!("/v1/channels/{}/publish", org.platform.id),
        json!({"record_ids": [runbook], "message": "reviewed at the incident retro"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "platform's publish failed: {body}");

    // Payments: the reader the whole feature is about.
    seed_user(&pool, tenant, "priya", org.payments.id).await;
    let priya = issue("priya", tenant);

    // Two stewards at the department, because `regulated-strict`'s `policy`
    // cell asks for two *distinct* steward approvers — tech plan §2.4's
    // lapse row, carried in the matrix since FLOW-3.
    seed_user(&pool, tenant, "nadia", org.eng.id).await;
    seed_user(&pool, tenant, "omar", org.eng.id).await;
    bind(&pool, tenant, "nadia", org.eng.id, Role::Steward).await;
    bind(&pool, tenant, "omar", org.eng.id, Role::Steward).await;
    let nadia = issue("nadia", tenant);
    let omar = issue("omar", tenant);

    // ── Before ──────────────────────────────────────────────────────────
    assert!(
        !block(&app, &priya).await.contains(RUNBOOK),
        "regulated-strict has no cross-team read: that is what a lapse is for"
    );

    // ── The proposal, opened on the disclosing side ─────────────────────
    let proposal = proposed(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        WINDOW_SECS,
        "joint incident review: payments is on the bridge for the outage",
    )
    .await;

    // One steward is not enough, and the refusal says what is missing
    // rather than leaving someone to read a pack.
    approve(&app, &nadia, &proposal).await;
    let (status, body) = post(
        &app,
        &nadia,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "one steward must not open the door: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("distinct approver"),
        "the refusal must name what is outstanding: {body}"
    );

    // ── The grant ───────────────────────────────────────────────────────
    approve(&app, &omar, &proposal).await;
    let (status, granted) = post(
        &app,
        &omar,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "granting the lapse failed: {granted}"
    );
    assert_eq!(granted["outcome"].as_str(), Some("active"));
    let lapse_id = granted["id"].as_str().expect("lapse id").to_owned();

    // ── While it stands ─────────────────────────────────────────────────
    let during = block(&app, &priya).await;
    assert!(
        during.contains(RUNBOOK),
        "the lapse must reach the reader's own inject: {during}"
    );
    // And the block says so. A reader who is not a member of platform
    // must not be shown its material under a header identical to their own
    // team's (ADR-0037 decision 12).
    assert!(
        during.contains("[lapse]"),
        "a lapsed section has to declare itself: {during}"
    );
    assert!(
        during.contains(&org.platform.path),
        "the section names the scope the material came from: {during}"
    );

    // ── Expiry: nothing is revoked, nothing runs, the window just closes ─
    tokio::time::sleep(Duration::from_secs(u64::from(WINDOW_SECS) + 1)).await;
    let after = block(&app, &priya).await;
    assert!(
        !after.contains(RUNBOOK),
        "expiry must restore the denial with no act by anyone: {after}"
    );
    assert!(!after.contains("[lapse]"), "and no empty lapsed section");

    // The sweep is bookkeeping: it chains the event, and the access was
    // already gone before it ran. Driving it directly rather than waiting
    // on the background loop is the FLOW-4 discipline — a test asserts the
    // pass, not the scheduler.
    //
    // What is asserted is the event on *this tenant's* chain, never the
    // sweep's own return value: `expire_once` is tenant-wide, so a
    // concurrent test's pass can chain this grant first and hand this call
    // a zero. The property that matters is unchanged by who ran the pass —
    // one window, exactly one expiry event, ever — and asserting the count
    // a scheduler happened to produce would be asserting the scheduler.
    synveda_gateway::lapses::expire_once(&pool)
        .await
        .expect("expiry sweep");
    // Twice is once: the stamp is the idempotency key.
    synveda_gateway::lapses::expire_once(&pool)
        .await
        .expect("second sweep");
    let expired = events(&pool, tenant, "policy.lapse.expired").await;
    assert_eq!(expired.len(), 1, "one window, one expiry event");

    // ── The full story, on one chain, in order ──────────────────────────
    let opened = events(&pool, tenant, "vedaflow.proposal.opened").await;
    let approvals = events(&pool, tenant, "vedaflow.proposal.approved").await;
    let grants = events(&pool, tenant, "policy.lapse.granted").await;
    assert_eq!(opened.len(), 1, "the proposal was opened once");
    assert_eq!(approvals.len(), 2, "two distinct stewards approved");
    assert_eq!(grants.len(), 1, "the effect ran once");

    let grant = &grants[0];
    assert_eq!(grant.payload["lapse_id"].as_str(), Some(lapse_id.as_str()));
    assert_eq!(
        grant.payload["lapse"]["target_scope_id"].as_str(),
        Some(org.platform.id.to_string().as_str())
    );
    assert_eq!(
        grant.payload["lapse"]["grantee_scope_id"].as_str(),
        Some(org.payments.id.to_string().as_str())
    );
    assert!(
        grant.payload["lapse"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("joint incident review"),
        "the mandatory reason rides the event: {}",
        grant.payload
    );
    // The window is on the grant event, which is what keeps the trail
    // complete even when the sweep never runs: when the grant stopped
    // deciding is arithmetic over these two instants.
    assert!(grant.payload["granted_at"].is_string());
    assert!(grant.payload["expires_at"].is_string());
    assert_eq!(
        grant.payload["authz"]["action"].as_str(),
        Some("lapse.grant")
    );
    // Never the material it disclosed.
    assert!(
        !grant.payload.to_string().contains(RUNBOOK),
        "an audit payload must never carry record content"
    );

    // The expiry event is the sweep's, under system attribution.
    assert_eq!(expired[0].actor_kind, "system");
    assert_eq!(
        expired[0].payload["lapse_id"].as_str(),
        Some(lapse_id.as_str())
    );

    // Every act in order on one chain, and the chain verifies over all of
    // it.
    let ordering: Vec<i64> = [
        opened[0].seq,
        approvals[0].seq,
        approvals[1].seq,
        grants[0].seq,
        expired[0].seq,
    ]
    .to_vec();
    let mut sorted = ordering.clone();
    sorted.sort_unstable();
    assert_eq!(ordering, sorted, "the story reads in the order it happened");

    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let report = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(report, synveda_audit::ChainVerification::Valid { .. }),
        "audit chain broke: {report}"
    );
}

// ── What a lapse discloses ───────────────────────────────────────────────────

/// A lapse admits the target's **published** channel and nothing else.
///
/// The honest half of the feature, and the reason two stewards can know
/// what they are consenting to: unreviewed extraction output that nobody at
/// the target has looked at does not travel on an override (ADR-0037
/// decision 11).
#[tokio::test(flavor = "multi_thread")]
async fn a_lapse_discloses_only_what_the_target_published() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;

    let sam = seed_user(&pool, tenant, "sam", org.platform.id).await;
    // Two records at platform: one published, one merely derived.
    let published = seed_record(&pool, tenant, org.platform.id, sam.id, RUNBOOK).await;
    let draft = "the draft nobody has reviewed: restart the broker by hand";
    seed_record(&pool, tenant, org.platform.id, sam.id, draft).await;
    seed_user(&pool, tenant, "cara", org.platform.id).await;
    bind(&pool, tenant, "cara", org.platform.id, Role::Curator).await;
    let (status, body) = post(
        &app,
        &issue("cara", tenant),
        &format!("/v1/channels/{}/publish", org.platform.id),
        json!({"record_ids": [published], "message": "reviewed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {body}");

    seed_user(&pool, tenant, "priya", org.payments.id).await;
    let priya = issue("priya", tenant);
    for subject in ["nadia", "omar"] {
        seed_user(&pool, tenant, subject, org.eng.id).await;
        bind(&pool, tenant, subject, org.eng.id, Role::Steward).await;
    }
    let nadia = issue("nadia", tenant);
    let omar = issue("omar", tenant);

    let proposal = proposed(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        600,
        "joint incident review",
    )
    .await;
    approve(&app, &nadia, &proposal).await;
    approve(&app, &omar, &proposal).await;
    let (status, body) = post(
        &app,
        &omar,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "grant failed: {body}");

    let composed = block(&app, &priya).await;
    assert!(
        composed.contains(RUNBOOK),
        "the published record travels: {composed}"
    );
    assert!(
        !composed.contains(draft),
        "unreviewed material must not travel on an override: {composed}"
    );
    // And what did travel is not marked unreviewed, because it is not:
    // platform published it.
    assert!(
        composed.contains("[lapse]"),
        "the section still declares the grant: {composed}"
    );
}

// ── Revocation ───────────────────────────────────────────────────────────────

/// A security-reviewer ends a grant they could never have opened.
///
/// This is the whole reason `LapseGrant` and `LapseRevoke` are two actions
/// rather than one (ADR-0037 decision 15): the responder who ends a
/// disclosure at 3am is not the steward who authorises one, and a pack has
/// to be able to say so.
#[tokio::test(flavor = "multi_thread")]
async fn a_security_reviewer_revokes_what_they_could_never_grant() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;

    let sam = seed_user(&pool, tenant, "sam", org.platform.id).await;
    let runbook = seed_record(&pool, tenant, org.platform.id, sam.id, RUNBOOK).await;
    seed_user(&pool, tenant, "cara", org.platform.id).await;
    bind(&pool, tenant, "cara", org.platform.id, Role::Curator).await;
    post(
        &app,
        &issue("cara", tenant),
        &format!("/v1/channels/{}/publish", org.platform.id),
        json!({"record_ids": [runbook], "message": "reviewed"}),
    )
    .await;

    seed_user(&pool, tenant, "priya", org.payments.id).await;
    let priya = issue("priya", tenant);
    for subject in ["nadia", "omar"] {
        seed_user(&pool, tenant, subject, org.eng.id).await;
        bind(&pool, tenant, subject, org.eng.id, Role::Steward).await;
    }
    // The responder: security-reviewer at the org, and nothing else.
    seed_user(&pool, tenant, "raj", org.org.id).await;
    bind(&pool, tenant, "raj", org.org.id, Role::SecurityReviewer).await;
    let nadia = issue("nadia", tenant);
    let omar = issue("omar", tenant);
    let raj = issue("raj", tenant);

    // Raj cannot open one. `ProposalOpen` at platform is floored on
    // membership plus contributor-and-above, and security-reviewer is
    // neither.
    let (status, body) = propose(
        &app,
        &raj,
        org.platform.id,
        org.payments.id,
        600,
        "I would like to open this myself",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a security-reviewer must not be able to open a lapse: {body}"
    );

    let proposal = proposed(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        600,
        "joint incident review",
    )
    .await;
    approve(&app, &nadia, &proposal).await;
    approve(&app, &omar, &proposal).await;
    let (_, granted) = post(
        &app,
        &omar,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    let lapse_id = granted["id"].as_str().expect("lapse id").to_owned();
    assert!(block(&app, &priya).await.contains(RUNBOOK));

    // A revocation needs its reason, exactly as the grant does.
    let (status, body) = post(
        &app,
        &raj,
        &format!("/v1/lapses/{lapse_id}/revoke"),
        json!({"reason": "   "}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a reasonless revocation is not a governed act: {body}"
    );

    let (status, revoked) = post(
        &app,
        &raj,
        &format!("/v1/lapses/{lapse_id}/revoke"),
        json!({"reason": "the bridge closed; access no longer needed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "revocation failed: {revoked}");
    assert_eq!(revoked["outcome"].as_str(), Some("revoked"));

    // The very next request, with no restart and nothing to wait out.
    assert!(
        !block(&app, &priya).await.contains(RUNBOOK),
        "a revocation reaches the reader on their next session"
    );

    // Terminal: a second revocation finds no standing grant.
    let (status, _) = post(
        &app,
        &raj,
        &format!("/v1/lapses/{lapse_id}/revoke"),
        json!({"reason": "again"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a revocation is terminal");

    // A revoked grant gets no expiry event: its ending is already on the
    // chain, and two events asserting one fact is something an auditor
    // would have to reconcile.
    synveda_gateway::lapses::expire_once(&pool)
        .await
        .expect("sweep");
    assert!(
        events(&pool, tenant, "policy.lapse.expired")
            .await
            .is_empty(),
        "a revoked grant must not also expire"
    );
    let revocations = events(&pool, tenant, "policy.lapse.revoked").await;
    assert_eq!(revocations.len(), 1);
    assert!(
        revocations[0].payload["would_have_expired_at"].is_string(),
        "the event records the window the revocation cut short"
    );
}

// ── Refusals, in the product's own words ─────────────────────────────────────

/// The four ways a lapse proposal is refused before it exists, each naming
/// what is wrong rather than leaving someone to read a pack.
#[tokio::test(flavor = "multi_thread")]
async fn a_lapse_that_cannot_mean_anything_is_refused_at_the_surface() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;
    let sam = seed_user(&pool, tenant, "sam", org.platform.id).await;
    seed_user(&pool, tenant, "nadia", org.eng.id).await;
    bind(&pool, tenant, "nadia", org.eng.id, Role::Steward).await;
    let nadia = issue("nadia", tenant);

    // 1. A personal scope, twice over — the privacy floor holds at two
    //    independent layers, and the test asserts both.
    //
    //    A steward is stopped by the PDP before the surface sees the
    //    request at all: every pack's `ProposalOpen` role branch excludes
    //    user-kind scopes, so nobody can even *propose* against somebody
    //    else's personal memory.
    let personal = {
        let mut tx = pool.begin().await.expect("begin");
        let node = hierarchy::node(&mut *tx, sam.scope_id)
            .await
            .expect("read sam's scope")
            .expect("sam has a scope");
        tx.commit().await.expect("commit");
        node
    };
    let (status, body) = propose(
        &app,
        &nadia,
        personal.id,
        org.payments.id,
        600,
        "let us read sam's notes",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a steward must not reach another principal's personal scope at all: {body}"
    );

    //    The one principal the membership floor *does* let in there is the
    //    owner, and this is the surface's own refusal: sam cannot lapse his
    //    own memory to a team either. No pack permits a lapse over a
    //    personal scope, so a proposal for one could only ever be reviewed
    //    and then refused at its effect — refusing it here is the ADR-0032
    //    discipline of failing at install rather than at review.
    let (status, body) = propose(
        &app,
        &issue("sam", tenant),
        personal.id,
        org.payments.id,
        600,
        "my own notes, to the payments team",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("personal scope"),
        "the refusal must name the floor: {body}"
    );

    // 2. A target the grantee already composes through its own chain: two
    //    stewards' review to change nothing.
    let (status, body) = propose(
        &app,
        &nadia,
        org.eng.id,
        org.payments.id,
        600,
        "payments should read engineering",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("already composes"),
        "the refusal must say the grant would do nothing: {body}"
    );

    // 3. An action outside the closed vocabulary. Widening the admin plane
    //    on a timer is a different product.
    let (status, body) = post(
        &app,
        &nadia,
        "/v1/lapses",
        json!({
            "scope_id": org.platform.id,
            "grantee_scope_id": org.payments.id,
            "action": "policy.assign",
            "duration_secs": 600,
            "reason": "let us administer platform for a while",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unlapsable action must be refused: {body}"
    );

    // 4. A window past the pack's ceiling. `regulated-strict` grants seed
    //    §6's own 30 days, and the refusal names both numbers.
    let (status, body) = propose(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        60 * 60 * 24 * 45,
        "forty-five days, please",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("ceiling"),
        "the refusal must name the ceiling: {body}"
    );

    // And a reasonless one, because the reason is the point.
    let (status, _) = propose(&app, &nadia, org.platform.id, org.payments.id, 600, "  ").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The listing answers "who could read this scope's material, and when" —
/// including grants that have since ended, which is the only way it can.
#[tokio::test(flavor = "multi_thread")]
async fn the_listing_keeps_grants_that_have_ended() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state(&database_url()));
    let org = seed_hierarchy(&pool, tenant).await;
    seed_user(&pool, tenant, "sam", org.platform.id).await;
    for subject in ["nadia", "omar"] {
        seed_user(&pool, tenant, subject, org.eng.id).await;
        bind(&pool, tenant, subject, org.eng.id, Role::Steward).await;
    }
    let nadia = issue("nadia", tenant);
    let omar = issue("omar", tenant);

    let proposal = proposed(
        &app,
        &nadia,
        org.platform.id,
        org.payments.id,
        WINDOW_SECS,
        "joint incident review",
    )
    .await;
    approve(&app, &nadia, &proposal).await;
    approve(&app, &omar, &proposal).await;
    post(
        &app,
        &omar,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;

    let (status, body) = get(
        &app,
        &nadia,
        &format!("/v1/lapses?scope_id={}", org.platform.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "listing failed: {body}");
    let listed = body["lapses"].as_array().expect("lapses array");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["outcome"].as_str(), Some("active"));

    tokio::time::sleep(Duration::from_secs(u64::from(WINDOW_SECS) + 1)).await;
    let (_, body) = get(
        &app,
        &nadia,
        &format!("/v1/lapses?scope_id={}", org.platform.id),
    )
    .await;
    let listed = body["lapses"].as_array().expect("lapses array");
    assert_eq!(
        listed.len(),
        1,
        "an ended grant stays: 'who could read this in March' is the question"
    );
    assert_eq!(
        listed[0]["outcome"].as_str(),
        Some("expired"),
        "and it renders as ended rather than being deleted"
    );
}

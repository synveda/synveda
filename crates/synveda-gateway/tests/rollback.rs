//! FLOW-7 acceptance criteria (ADR-0036): **a rewind reaches every agent
//! under the scope on their next session**, **it can only install a state
//! the channel has already held**, and **a pin holds what a channel serves
//! while publications keep landing**.
//!
//! The AC test is shaped around the way bad content actually becomes
//! trusted: a runbook line is authored at a team, climbs to the
//! department through review (FLOW-5), and from there reaches two teams'
//! agents as reviewed material. One curator then rewinds the department,
//! and the next session of every reader is different — with nobody
//! convening, no approvals, and no restart.
//!
//! What the two readers see afterwards is deliberately *not* the same
//! thing, and asserting both is the point. For the payments engineer the
//! line is simply gone: the record lives at a team outside her chain, so
//! without the department's tree naming it there is nothing to compose.
//! For the platform engineer it comes back as `[unreviewed]` — the record
//! still lives in her chain, and what a rewind removed is its *trust*,
//! which is exactly what a channel is. A demo that only showed the first
//! reader would be claiming the feature deletes content. It does not; it
//! moves the boundary.
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

const SECRET: &[u8] = b"flow-7-test-secret";

/// The line that should be there.
const GOOD: &str = "deploys go out on tuesdays after the release review";
/// The line that should not: a real-shaped operational instruction, which
/// is what "bad prompt" means before PRMT-1 makes prompts assets.
const BAD: &str = "skip the staging soak when the release is running late";

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
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-flow7-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open sidecar"),
        ),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
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
                "skipping FLOW-7 rollback test: DATABASE_URL is not set \
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
    let slug = format!("flow7-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-7 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// acme → eng → {platform, payments}, plus the quarantine scope every
/// tenant needs.
struct Estate {
    org: HierarchyNode,
    eng: HierarchyNode,
    platform: HierarchyNode,
    payments: HierarchyNode,
}

async fn seed_estate(pool: &PgPool, tenant: TenantId) -> Estate {
    let mut tx = pool.begin().await.expect("begin");
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
    let eng = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Department,
        "eng",
        "Engineering",
    )
    .await
    .expect("create dept");
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(eng.id),
        ScopeKind::Team,
        "platform",
        "Platform",
    )
    .await
    .expect("create platform");
    let payments = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(eng.id),
        ScopeKind::Team,
        "payments",
        "Payments",
    )
    .await
    .expect("create payments");
    hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
        "Quarantine",
    )
    .await
    .expect("create quarantine scope");
    tx.commit().await.expect("commit hierarchy");
    Estate {
        org,
        eng,
        platform,
        payments,
    }
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
        subject,
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
            kind: RecordKind::Pinned,
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "flow-7 acceptance test"}),
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

async fn post(app: &Router, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    call(app, request).await
}

async fn get(app: &Router, token: &str, path: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build request");
    call(app, request).await
}

async fn inject(app: &Router, token: &str) -> Value {
    let (status, body) = post(app, token, "/v1/inject", json!({"session_id": "flow-7"})).await;
    assert_eq!(status, StatusCode::OK, "inject: {body}");
    body
}

fn text(block: &Value) -> String {
    block["text"].as_str().expect("block text").to_owned()
}

/// Whether the block presents `line` as **reviewed** material: present,
/// and without the unreviewed marker the renderer puts on everything the
/// trust boundary has not admitted.
fn reads_as_reviewed(block: &Value, line: &str) -> bool {
    text(block).contains(&format!("- [procedure] {line}\n"))
}

/// Whether the block carries `line` at all, however it is labelled.
fn mentions(block: &Value, line: &str) -> bool {
    text(block).contains(line)
}

/// Publishes `records` onto `scope`'s published channel directly. Legal
/// at a team under `regulated-strict`, where the matrix asks for one
/// curator and one curator acted (ADR-0032 decision 8).
async fn publish(
    app: &Router,
    token: &str,
    scope: ScopeId,
    records: &[RecordId],
    message: &str,
) -> Value {
    let (status, body) = post(
        app,
        token,
        &format!("/v1/channels/{scope}/publish"),
        json!({"record_ids": records, "message": message}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish at {scope}: {body}");
    body
}

/// The whole FLOW-3/FLOW-5 path in one call: tara opens a climb from the
/// platform team to Engineering, the department's approvers approve, and
/// cora runs the effect. Returns the publication's commit.
///
/// `regulated-strict` asks for a curator **and** a steward at a department
/// — two distinct people — which is why this takes two approvals, and why
/// cora is the one who publishes: steward reads no content in any pack,
/// and the effect takes the same read a direct publication does.
impl Fixture {
    async fn promote(&self, records: &[RecordId], title: &str) -> String {
        let body = json!({
            "scope_id": self.estate.eng.id,
            "source_scope_id": self.estate.platform.id,
            "record_ids": records,
            "title": title,
        });
        let (status, opened) = post(&self.app, &self.tara, "/v1/proposals", body).await;
        assert_eq!(status, StatusCode::OK, "open proposal: {opened}");
        let id = opened["id"].as_str().expect("proposal id").to_owned();

        for approver in [&self.cora, &self.steve] {
            let (status, approved) = post(
                &self.app,
                approver,
                &format!("/v1/proposals/{id}/approve"),
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "approve {id}: {approved}");
        }
        let (status, published) = post(
            &self.app,
            &self.cora,
            &format!("/v1/proposals/{id}/publish"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "publish {id}: {published}");
        published["commit"]
            .as_str()
            .expect("publication commit")
            .to_owned()
    }
}

async fn history(app: &Router, token: &str, scope: ScopeId) -> Value {
    let (status, body) = get(app, token, &format!("/v1/channels/{scope}/history")).await;
    assert_eq!(status, StatusCode::OK, "history at {scope}: {body}");
    body
}

async fn rollback(
    app: &Router,
    token: &str,
    scope: ScopeId,
    from: &str,
    to: &str,
) -> (StatusCode, Value) {
    post(
        app,
        token,
        &format!("/v1/channels/{scope}/rollback"),
        json!({"from_commit": from, "to_commit": to, "message": "retract: unsafe instruction"}),
    )
    .await
}

async fn events(pool: &PgPool, tenant: TenantId, action: &str) -> Vec<StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let mut all = synveda_audit::tail(&mut tx, tenant, 400)
        .await
        .expect("read chain");
    all.reverse();
    all.into_iter()
        .filter(|event| event.action == action)
        .collect()
}

async fn chain_verifies(pool: &PgPool, tenant: TenantId) -> bool {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    matches!(
        synveda_audit::verify(&mut tx, tenant)
            .await
            .expect("verify chain"),
        synveda_audit::ChainVerification::Valid { .. }
    )
}

/// The whole estate the AC needs, wired: a runbook line published at the
/// department through review, and two readers in different teams.
struct Fixture {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    estate: Estate,
    tara: String,
    cora: String,
    steve: String,
    alice: String,
    bea: String,
    good: RecordId,
    bad: RecordId,
}

async fn fixture() -> Option<Fixture> {
    let (pool, tenant) = admitted_tenant().await?;
    let estate = seed_estate(&pool, tenant).await;
    let app = router(state(&database_url()));

    // Tara curates the platform team, where the material is authored.
    let tara_identity = seed_user(&pool, tenant, "tara", estate.platform.id).await;
    bind(&pool, tenant, "tara", estate.platform.id, Role::Curator).await;
    // Cora curates Engineering and Steve stewards it: the two distinct
    // approvers `regulated-strict` asks for at a department.
    seed_user(&pool, tenant, "cora", estate.eng.id).await;
    bind(&pool, tenant, "cora", estate.eng.id, Role::Curator).await;
    // Steve stewards Engineering from outside it: placed under the org,
    // so the `regulated-strict` own-chain MemoryRead floor does not hand
    // him the content read his *role* deliberately does not carry. That
    // is the same shape FLOW-5's fixture uses, and it is what makes
    // "the steward cannot run the effect" true rather than incidental.
    seed_user(&pool, tenant, "steve", estate.org.id).await;
    bind(&pool, tenant, "steve", estate.eng.id, Role::Steward).await;
    // Two readers with no roles at all — the fleet.
    seed_user(&pool, tenant, "alice", estate.platform.id).await;
    seed_user(&pool, tenant, "bea", estate.payments.id).await;

    let good = seed_record(&pool, tenant, estate.platform.id, tara_identity.id, GOOD).await;
    let bad = seed_record(&pool, tenant, estate.platform.id, tara_identity.id, BAD).await;

    Some(Fixture {
        tara: issue("tara", tenant),
        cora: issue("cora", tenant),
        steve: issue("steve", tenant),
        alice: issue("alice", tenant),
        bea: issue("bea", tenant),
        pool,
        tenant,
        app,
        estate,
        good,
        bad,
    })
}

// ── The acceptance criterion ─────────────────────────────────────────────────

/// **A rewind reaches every agent under the scope on their next session.**
///
/// The bad line gets in the way bad lines get in: authored at a team,
/// climbed to the department under that department's approvers, and from
/// there composed as reviewed material by everyone below. One curator
/// rewinds; the next inject of both readers is different, and nothing else
/// happened in between — no second approval, no restart, no cache to wait
/// out.
#[test]
fn a_rewind_reaches_every_agent_under_the_scope_on_the_next_session() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;

        // Two publications at the department, each through review, so the
        // channel has a state to go back to.
        fx.promote(&[fx.good], "release runbook").await;
        let bad_commit = fx
            .promote(&[fx.bad], "runbook: late-release exception")
            .await;

        // The fleet, before. Both readers hold no roles and configured
        // nothing; the department's channel is simply on their chain.
        for (who, token) in [("alice", &fx.alice), ("bea", &fx.bea)] {
            let block = inject(app, token).await;
            assert!(
                reads_as_reviewed(&block, BAD),
                "{who} must be receiving the bad line as reviewed material: {}",
                text(&block)
            );
            assert!(reads_as_reviewed(&block, GOOD), "{who} reads the runbook");
        }

        // The operator's two calls: read the states this channel has held,
        // then install the one before the mistake.
        let states = history(app, &fx.cora, fx.estate.eng.id).await;
        let entries = states["history"].as_array().expect("history array");
        assert_eq!(
            entries[0]["commit"].as_str(),
            Some(bad_commit.as_str()),
            "the newest state is the publication that went wrong: {states}"
        );
        assert_eq!(
            entries[0]["head"].as_bool(),
            Some(true),
            "the head is marked, because it is the one entry a rewind cannot name"
        );
        let previous = entries[1]["commit"].as_str().expect("previous state");

        let (status, rewound) =
            rollback(app, &fx.cora, fx.estate.eng.id, &bad_commit, previous).await;
        assert_eq!(status, StatusCode::OK, "rewind: {rewound}");
        assert_eq!(
            rewound["removed"].as_array().map(Vec::len),
            Some(1),
            "exactly the bad record stopped being published: {rewound}"
        );
        assert_eq!(
            rewound["removed"][0].as_str(),
            Some(fx.bad.to_string().as_str()),
            "and it is the one named: {rewound}"
        );

        // The fleet, after — one call later, with nothing else done.
        //
        // Bea's block loses the line entirely: the record lives at the
        // platform team, which is not on her chain, so with the
        // department's tree no longer naming it there is nothing to
        // compose. Alice's keeps it and marks it `[unreviewed]`: it still
        // lives in her chain, and what the rewind took away is its trust.
        let bea = inject(app, &fx.bea).await;
        assert!(
            !mentions(&bea, BAD),
            "a payments engineer must not see the retracted line at all: {}",
            text(&bea)
        );
        assert!(reads_as_reviewed(&bea, GOOD), "the good runbook survives");

        let alice = inject(app, &fx.alice).await;
        assert!(
            !reads_as_reviewed(&alice, BAD),
            "a platform engineer must not see the retracted line as reviewed: {}",
            text(&alice)
        );
        assert!(
            text(&alice).contains(&format!("- [procedure] {BAD} [unreviewed]")),
            "…it comes back as unreviewed material, which is what a channel is: {}",
            text(&alice)
        );
        assert!(reads_as_reviewed(&alice, GOOD), "the good runbook survives");

        // And the trail. One event, carrying both commits, the record that
        // left the boundary, and the operator's reason — never content.
        let chained = events(&fx.pool, fx.tenant, "vedaflow.channel.rolled_back").await;
        assert_eq!(chained.len(), 1, "one rewind, one event");
        let payload = &chained[0].payload;
        assert_eq!(payload["from"].as_str(), Some(bad_commit.as_str()));
        assert_eq!(payload["to"].as_str(), Some(previous));
        assert_eq!(
            payload["removed"][0].as_str(),
            Some(fx.bad.to_string().as_str())
        );
        assert_eq!(
            payload["message"].as_str(),
            Some("retract: unsafe instruction")
        );
        assert_eq!(
            payload["authz"]["action"].as_str(),
            Some("channel.rollback"),
            "the decision that permitted it rides the event: {payload}"
        );
        assert!(
            !chained[0].payload.to_string().contains(BAD),
            "an audit payload carries ids and addresses, never record content"
        );
        assert!(chain_verifies(&fx.pool, fx.tenant).await, "chain verifies");
    });
}

// ── What a rewind may install (ADR-0036 decisions 1 and 2) ───────────────────

/// **A proposal commit is reachable from the head and is still not a
/// rewind target.**
///
/// This is the distinction the feature turns on. Since FLOW-3 a
/// publication through review is a merge commit whose second parent is the
/// proposal commit, so FLOW-1's `is_ancestor` — the fast-forward test, run
/// backwards — would happily accept it. Its tree is the *proposed* member
/// set, which nobody ever published.
#[test]
fn a_proposal_commit_is_reachable_and_is_still_refused() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;

        fx.promote(&[fx.good], "release runbook").await;
        let head = fx
            .promote(&[fx.bad], "runbook: late-release exception")
            .await;

        let states = history(app, &fx.cora, fx.estate.eng.id).await;
        let entries = states["history"].as_array().expect("history array");
        let proposal_commit = entries[0]["merge_parents"][0]
            .as_str()
            .expect("a reviewed publication names its proposal commit")
            .to_owned();

        // It is on the listing as the *provenance* of the head, and it is
        // deliberately not one of the states listed.
        assert!(
            entries
                .iter()
                .all(|entry| entry["commit"].as_str() != Some(proposal_commit.as_str())),
            "a proposal commit is not a state the channel held: {states}"
        );

        let (status, refused) =
            rollback(app, &fx.cora, fx.estate.eng.id, &head, &proposal_commit).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "refusal: {refused}");
        let message = refused["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("never a state it held"),
            "the refusal must name why a reachable commit is not a target: {message}"
        );
        assert!(
            message.contains("may never have been approved"),
            "…and say what would be installed if it were: {message}"
        );

        // And the case that nearly slipped through, because it is the one
        // where ordinal 0 is not enough: a channel's **first** publication
        // has no head to be its first parent, so a reviewed one puts the
        // proposal commit at ordinal 0 (ADR-0032 decision 10, which
        // FLOW-3's own AC pins). It is still not a state the ref held.
        let oldest = entries.last().expect("the channel's first state");
        assert!(
            oldest["parent"].is_null(),
            "a channel's first state replaced nothing: {oldest}"
        );
        let first_proposal = oldest["merge_parents"][0]
            .as_str()
            .expect("its proposal is provenance, not a parent state")
            .to_owned();
        assert!(
            entries
                .iter()
                .all(|entry| entry["commit"].as_str() != Some(first_proposal.as_str())),
            "the walk stops at it rather than listing it: {states}"
        );
        let (status, refused) =
            rollback(app, &fx.cora, fx.estate.eng.id, &head, &first_proposal).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a proposal commit at ordinal 0 is refused like any other: {refused}"
        );
    });
}

/// **A rewind rewinds; it never advances**, and a stale `from` is a
/// conflict rather than a silent re-decision.
#[test]
fn a_rewind_never_advances_and_never_guesses_what_it_is_leaving() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;

        let first =
            publish(app, &fx.tara, fx.estate.platform.id, &[fx.good], "runbook").await["commit"]
                .as_str()
                .expect("commit")
                .to_owned();
        let second = publish(
            app,
            &fx.tara,
            fx.estate.platform.id,
            &[fx.bad],
            "late-release exception",
        )
        .await["commit"]
            .as_str()
            .expect("commit")
            .to_owned();

        // Where the channel already is.
        let (status, refused) =
            rollback(app, &fx.tara, fx.estate.platform.id, &second, &second).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "no-op rewind: {refused}");
        assert!(
            refused["message"]
                .as_str()
                .unwrap_or_default()
                .contains("already points at"),
            "a rewind to the head says so: {refused}"
        );

        // A `from` that is not the head: the world moved under a
        // well-formed request, which is a conflict and names both.
        let (status, stale) = rollback(app, &fx.tara, fx.estate.platform.id, &first, &second).await;
        assert_eq!(status, StatusCode::CONFLICT, "stale from: {stale}");
        assert!(
            stale["message"]
                .as_str()
                .unwrap_or_default()
                .contains("re-read the history"),
            "…and says what to do about it: {stale}"
        );

        // The rewind itself, then the attempt to undo it by rewinding
        // forward. Recovery is publishing, which resolves the matrix
        // again — a rewind that could be undone by another rewind would be
        // a way to reinstate a member set without ever resolving one.
        let (status, done) = rollback(app, &fx.tara, fx.estate.platform.id, &second, &first).await;
        assert_eq!(status, StatusCode::OK, "rewind: {done}");

        let (status, forward) =
            rollback(app, &fx.tara, fx.estate.platform.id, &first, &second).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "advance: {forward}");
        let message = forward["message"].as_str().unwrap_or_default();
        assert!(
            message.contains("re-admitting content is a publication"),
            "the refusal names the way back: {message}"
        );
    });
}

/// A log channel has no membership to rewind, and an asset kind with no
/// read action cannot be governed by memory's.
#[test]
fn only_set_channels_of_readable_assets_rewind() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;
        let head =
            publish(app, &fx.tara, fx.estate.platform.id, &[fx.good], "runbook").await["commit"]
                .as_str()
                .expect("commit")
                .to_owned();

        let (status, refused) = post(
            app,
            &fx.tara,
            &format!("/v1/channels/{}/rollback", fx.estate.platform.id),
            json!({
                "channel": "derived",
                "from_commit": head,
                "to_commit": head,
                "message": "rewind the pipeline log",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "derived rewind: {refused}");
        assert!(
            refused["message"]
                .as_str()
                .unwrap_or_default()
                .contains("log channel"),
            "the refusal names the shape: {refused}"
        );

        // PRMT-1 (ADR-0049 decision 4) supplied the read action this
        // route deferred to it by name, so a prompt channel is now
        // *decidable*: the refusal below is the ordinary one for a channel
        // that was never written, not the "this asset kind has no reader"
        // one it used to be.
        let (status, refused) = post(
            app,
            &fx.tara,
            &format!("/v1/channels/{}/rollback", fx.estate.platform.id),
            json!({
                "asset": "prompt",
                "from_commit": head,
                "to_commit": head,
                "message": "rewind the prompts",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "prompt rewind: {refused}");
        assert!(
            !refused.to_string().contains("no read action yet"),
            "PromptRead exists now; the deferral is discharged: {refused}"
        );
        assert!(
            refused["message"]
                .as_str()
                .unwrap_or_default()
                .contains("already points at"),
            "the request gets as far as the rewind's own rules — from == to \
             is a malformed rewind rather than an ungoverned asset kind: {refused}"
        );

        // The two kinds that still have no reader are still refused by
        // name — the deferral shrank rather than closing.
        for asset in ["skill", "context-pack"] {
            let (status, refused) = post(
                app,
                &fx.tara,
                &format!("/v1/channels/{}/rollback", fx.estate.platform.id),
                json!({
                    "asset": asset,
                    "from_commit": head,
                    "to_commit": head,
                    "message": "rewind something with no reader",
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{asset} rewind: {refused}");
            assert!(
                refused["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("no read action yet"),
                "an asset kind without a reader is refused by name: {refused}"
            );
        }
    });
}

// ── Who may rewind (ADR-0036 decision 3) ─────────────────────────────────────

/// A rewind takes `ChannelRollback` **and** the asset kind's read action,
/// which is publishing's own rule and is what keeps a curator out of a
/// teammate's personal channel with no clause about personal scopes
/// anywhere in the feature.
#[test]
fn rewinding_takes_the_action_and_the_read_that_publishing_takes() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;

        let first =
            publish(app, &fx.tara, fx.estate.platform.id, &[fx.good], "runbook").await["commit"]
                .as_str()
                .expect("commit")
                .to_owned();
        let head = publish(
            app,
            &fx.tara,
            fx.estate.platform.id,
            &[fx.bad],
            "late-release exception",
        )
        .await["commit"]
            .as_str()
            .expect("commit")
            .to_owned();

        // A reader with no roles: denied on the action itself.
        let (status, denied) = rollback(app, &fx.alice, fx.estate.platform.id, &head, &first).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "member rewind: {denied}");
        assert_eq!(denied["action"].as_str(), Some("channel.rollback"));

        // A steward: holds the action, reads no content in any pack, and
        // so cannot rewind — the same reason a steward cannot publish.
        let (status, denied) = rollback(app, &fx.steve, fx.estate.eng.id, &head, &first).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "steward rewind: {denied}");
        assert_eq!(
            denied["action"].as_str(),
            Some("memory.read"),
            "the second decision is the one that refuses: {denied}"
        );

        // A teammate's personal channel. Alice curates her own scope and
        // publishes her own note; Tara curates the team above it, holds
        // `ChannelRollback` there by inheritance, and is still refused —
        // by the privacy floor, through the read decision.
        let alice_identity = {
            let mut tx = fx.pool.begin().await.expect("begin");
            let identity = synveda_store::identities::by_subject(&mut *tx, fx.tenant, "alice")
                .await
                .expect("read alice")
                .expect("alice exists");
            tx.commit().await.expect("commit");
            identity
        };
        let alice_scope = alice_identity.scope_id;
        bind(&fx.pool, fx.tenant, "alice", alice_scope, Role::Curator).await;
        let note = seed_record(
            &fx.pool,
            fx.tenant,
            alice_scope,
            alice_identity.id,
            "my own note",
        )
        .await;
        let hers = publish(app, &fx.alice, alice_scope, &[note], "my note").await["commit"]
            .as_str()
            .expect("commit")
            .to_owned();

        let (status, denied) = rollback(app, &fx.tara, alice_scope, &hers, &hers).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a curator must not rewind a teammate's personal channel: {denied}"
        );
        assert_eq!(denied["action"].as_str(), Some("memory.read"));

        // Every refusal above is a denial, so the chain carries them and
        // none of them moved anything.
        let states = history(app, &fx.tara, fx.estate.platform.id).await;
        assert_eq!(
            states["head"].as_str(),
            Some(head.as_str()),
            "nothing a denial touched moved: {states}"
        );
        assert!(chain_verifies(&fx.pool, fx.tenant).await, "chain verifies");
    });
}

// ── Pinning (ADR-0036 decisions 6, 7 and 10) ─────────────────────────────────

/// **A pin holds what a channel serves while publications keep landing**,
/// the block says it is held, and releasing it catches every reader up on
/// their next session.
#[test]
fn a_pin_holds_what_readers_compose_while_the_channel_keeps_moving() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;
        let platform = fx.estate.platform.id;

        let held = publish(app, &fx.tara, platform, &[fx.good], "runbook").await["commit"]
            .as_str()
            .expect("commit")
            .to_owned();

        let (status, pinned) = post(
            app,
            &fx.tara,
            &format!("/v1/channels/{platform}/pin"),
            json!({"commit": held, "reason": "freeze the runbook through the migration"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "pin: {pinned}");
        assert_eq!(pinned["commit"].as_str(), Some(held.as_str()));

        // Publishing onto a pinned channel lands, moves the ref, and says
        // that readers did not move — a curator who publishes and sees no
        // effect must be told why.
        let published = publish(app, &fx.tara, platform, &[fx.bad], "late-release exception").await;
        let head = published["commit"].as_str().expect("commit").to_owned();
        assert_ne!(head, held, "the channel itself advanced");
        assert_eq!(
            published["pinned"]["commit"].as_str(),
            Some(held.as_str()),
            "the publish response names the standing pin: {published}"
        );

        // The reader is held, and the block says so rather than implying
        // it holds the latest reviewed material.
        let block = inject(app, &fx.alice).await;
        assert!(reads_as_reviewed(&block, GOOD), "the pinned state composes");
        assert!(
            !reads_as_reviewed(&block, BAD),
            "what landed after the pin does not: {}",
            text(&block)
        );
        let citation = block["channels"]
            .as_array()
            .expect("channel citations")
            .iter()
            .find(|channel| channel["scope_id"].as_str() == Some(platform.to_string().as_str()))
            .expect("the platform channel is cited")
            .clone();
        assert_eq!(citation["commit"].as_str(), Some(held.as_str()));
        assert_eq!(
            citation["pinned"].as_bool(),
            Some(true),
            "a frozen citation says it is frozen: {citation}"
        );

        // The listing shows the pin beside the channel it holds.
        let (status, listed) = get(app, &fx.tara, &format!("/v1/channels/{platform}")).await;
        assert_eq!(status, StatusCode::OK, "list: {listed}");
        let channel = listed["channels"]
            .as_array()
            .expect("channels")
            .iter()
            .find(|channel| channel["name"].as_str() == Some("memory/published"))
            .expect("published channel")
            .clone();
        assert_eq!(channel["commit"].as_str(), Some(head.as_str()));
        assert_eq!(channel["pin"]["commit"].as_str(), Some(held.as_str()));

        // A rewind under a pin would reach nobody, so it refuses and names
        // the pin rather than returning 200 to a fleet-wide act that had
        // no fleet-wide effect.
        let (status, refused) = rollback(app, &fx.tara, platform, &head, &held).await;
        assert_eq!(status, StatusCode::CONFLICT, "pinned rewind: {refused}");
        assert!(
            refused["message"]
                .as_str()
                .unwrap_or_default()
                .contains("readers would not heal"),
            "the refusal says why: {refused}"
        );

        // Release, and the very next session catches up.
        let (status, released) = post(
            app,
            &fx.tara,
            &format!("/v1/channels/{platform}/unpin"),
            json!({"reason": "migration done"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "unpin: {released}");
        assert_eq!(released["released"].as_str(), Some(held.as_str()));

        let block = inject(app, &fx.alice).await;
        assert!(reads_as_reviewed(&block, BAD), "the reader caught up");
        let citation = block["channels"]
            .as_array()
            .expect("channel citations")
            .iter()
            .find(|channel| channel["scope_id"].as_str() == Some(platform.to_string().as_str()))
            .expect("the platform channel is cited")
            .clone();
        assert_eq!(citation["pinned"].as_bool(), Some(false));

        // Both acts are chained, with the reason each carries.
        let pinned_events = events(&fx.pool, fx.tenant, "vedaflow.channel.pinned").await;
        assert_eq!(pinned_events.len(), 1, "one pin, one event");
        assert_eq!(
            pinned_events[0].payload["reason"].as_str(),
            Some("freeze the runbook through the migration")
        );
        let unpinned = events(&fx.pool, fx.tenant, "vedaflow.channel.unpinned").await;
        assert_eq!(unpinned.len(), 1, "one release, one event");
        assert_eq!(
            unpinned[0].payload["released"].as_str(),
            Some(held.as_str())
        );
        assert!(chain_verifies(&fx.pool, fx.tenant).await, "chain verifies");
    });
}

/// A pin obeys the same rule a rewind does: it can only hold a state the
/// channel has held. A pin at a proposal commit would serve a member set
/// nobody approved, indefinitely.
#[test]
fn a_pin_can_only_hold_a_state_the_channel_has_held() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;

        fx.promote(&[fx.good], "release runbook").await;
        let states = history(app, &fx.cora, fx.estate.eng.id).await;
        let proposal_commit = states["history"][0]["merge_parents"][0]
            .as_str()
            .expect("proposal commit")
            .to_owned();

        let (status, refused) = post(
            app,
            &fx.cora,
            &format!("/v1/channels/{}/pin", fx.estate.eng.id),
            json!({"commit": proposal_commit, "reason": "hold here"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "pin: {refused}");
        assert!(
            refused["message"]
                .as_str()
                .unwrap_or_default()
                .contains("never a state it held"),
            "the same refusal a rewind gives: {refused}"
        );

        // Releasing a channel nobody pinned is the answer, not an error:
        // it serves its head either way, and an operator asserting that
        // is a fact worth chaining.
        let (status, nothing) = post(
            app,
            &fx.cora,
            &format!("/v1/channels/{}/unpin", fx.estate.eng.id),
            json!({"reason": "make sure nothing holds this"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "unpin: {nothing}");
        assert!(nothing["released"].is_null(), "nothing was held: {nothing}");
        assert_eq!(
            events(&fx.pool, fx.tenant, "vedaflow.channel.unpinned")
                .await
                .len(),
            1,
            "the assertion is still an act with an actor and a time"
        );
    });
}

// ── The climb interaction (ADR-0034 reversal trigger c) ──────────────────────

/// **A climbed record survives its source's rollback.**
///
/// The department admitted the record under its own approvers; a team
/// curator rewinding their own channel does not get to undo that. What
/// changes for the reader is which scope's section the line appears under,
/// which is true rather than cosmetic: the department is the scope that
/// stands behind it now.
#[test]
fn a_climbed_record_survives_its_sources_rewind() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let Some(fx) = fixture().await else { return };
        let app = &fx.app;
        let platform = fx.estate.platform.id;

        // Two states at the team, so there is one to go back to, and the
        // runbook only enters on the second.
        let before =
            publish(app, &fx.tara, platform, &[fx.bad], "late-release exception").await["commit"]
                .as_str()
                .expect("commit")
                .to_owned();
        let with_runbook = publish(app, &fx.tara, platform, &[fx.good], "runbook").await["commit"]
            .as_str()
            .expect("commit")
            .to_owned();

        // Then the runbook climbs to the department, under the
        // department's own approvers.
        fx.promote(&[fx.good], "the runbook is everyone's").await;

        // Before the rewind: composed under the team, the nearest scope
        // whose tree names it.
        let block = inject(app, &fx.alice).await;
        assert!(reads_as_reviewed(&block, GOOD));
        assert!(
            text(&block).contains("## acme/eng/platform (team)"),
            "the nearest publisher sections it: {}",
            text(&block)
        );

        // The team takes the runbook off its own channel.
        let (status, rewound) = rollback(app, &fx.tara, platform, &with_runbook, &before).await;
        assert_eq!(status, StatusCode::OK, "rewind at the source: {rewound}");
        assert_eq!(
            rewound["removed"][0].as_str(),
            Some(fx.good.to_string().as_str()),
            "the team stopped publishing it: {rewound}"
        );

        // The department is untouched. Its approvers made that decision
        // and a team curator does not get to undo it — which is
        // ADR-0034 decision 5's refusal of a ladder, pointed downhill.
        let dept = history(app, &fx.cora, fx.estate.eng.id).await;
        assert_eq!(
            dept["history"][0]["members"].as_u64(),
            Some(1),
            "the department's publication stands: {dept}"
        );

        // And the reader still has the line — now under the department's
        // section, which is true rather than cosmetic: the department is
        // the scope that stands behind it now.
        let block = inject(app, &fx.alice).await;
        assert!(
            reads_as_reviewed(&block, GOOD),
            "a climbed record survives its source's rewind: {}",
            text(&block)
        );
        assert!(
            text(&block).contains("## acme/eng (department)"),
            "…and is sectioned under the scope that still publishes it: {}",
            text(&block)
        );
        assert!(chain_verifies(&fx.pool, fx.tenant).await, "chain verifies");
    });
}

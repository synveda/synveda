//! FLOW-5's acceptance criteria (ADR-0034): **knowledge climbing two
//! levels with distinct approver sets**, and **denial at any level
//! audited with reason** — over the product's own HTTP surfaces, under
//! the PDP, against a live Postgres.
//!
//! The walk is the feature's own sentence made true: a team's procedure
//! reaches the department under the department's approvers, then the org
//! under the org's, and at each level the level below's curator is
//! refused by the PDP because role bindings inherit downward and never
//! up. What makes it a *promotion* rather than a row in a table is the
//! last assertion of each hop: a member of another team, and then a
//! member of another department, receives the procedure in their own
//! `inject` — people who could not read the team it lives at, and still
//! cannot.
//!
//! Around the AC walk: the direction rule (a climb goes up, never
//! sideways or down), the disclosure rule that lets a user climb their
//! own personal memory and stops anyone else from climbing it for them,
//! and the two senses in which a scope holds material — the second of
//! which is what lets the department propose onward what the team
//! climbed into it, with nothing stored to make the hop possible.
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

const SECRET: &[u8] = b"flow-5-test-secret";

/// The team knowledge that climbs.
const RUNBOOK: &str = "rotate the signing key every 90 days, on the first tuesday";

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
                    .join("synveda-flow5-tests")
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
                "skipping FLOW-5 cross-scope test: DATABASE_URL is not set \
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
    let slug = format!("flow5-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-5 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// The org the climb happens in:
///
/// ```text
/// acme (org)
/// ├── eng (department)
/// │   ├── platform (team)   ← the runbook lives here
/// │   └── payments (team)   ← reads it after the first hop, never before
/// └── sales (department)
///     └── field (team)      ← reads it after the second hop, never before
/// ```
///
/// Two departments, because "the org" has to mean something a department
/// publication does not: the second hop is proven by a reader outside
/// `eng` entirely.
struct Org {
    org: HierarchyNode,
    eng: HierarchyNode,
    platform: HierarchyNode,
    payments: HierarchyNode,
    field: HierarchyNode,
}

async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> Org {
    let mut tx = pool.begin().await.expect("begin");
    // The closure borrows the transaction, so it lives in a block that
    // ends before the commit.
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
        let sales = node(Some(org.id), ScopeKind::Department, "sales", "Sales").await;
        let field = node(Some(sales.id), ScopeKind::Team, "field", "Field").await;
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
            field,
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
            provenance: json!({"source": "flow-5 acceptance test"}),
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

async fn inject(app: &Router, token: &str) -> Value {
    let (status, body) = post(app, token, "/v1/inject", json!({"session_id": "flow-5"})).await;
    assert_eq!(status, StatusCode::OK, "inject failed: {body}");
    body
}

/// Whether a caller's own context block carries the runbook.
async fn reads_runbook(app: &Router, token: &str) -> bool {
    inject(app, token).await["text"]
        .as_str()
        .expect("block text")
        .contains(RUNBOOK)
}

/// Opens a proposal at `target`, optionally naming where the material is.
async fn open_climb(
    app: &Router,
    token: &str,
    target: ScopeId,
    source: Option<ScopeId>,
    records: &[RecordId],
    title: &str,
) -> (StatusCode, Value) {
    let mut body = json!({
        "scope_id": target,
        "record_ids": records,
        "title": title,
    });
    if let Some(source) = source {
        body["source_scope_id"] = json!(source);
    }
    post(app, token, "/v1/proposals", body).await
}

/// Opens a climb that must succeed, returning its id.
async fn climb(
    app: &Router,
    token: &str,
    target: ScopeId,
    source: ScopeId,
    records: &[RecordId],
    title: &str,
) -> String {
    let (status, body) = open_climb(app, token, target, Some(source), records, title).await;
    assert_eq!(status, StatusCode::OK, "opening the climb failed: {body}");
    assert_eq!(
        body["source_scope_id"].as_str(),
        Some(source.to_string().as_str()),
        "the proposal must record where the material came from: {body}"
    );
    body["id"].as_str().expect("proposal id").to_owned()
}

/// The human-readable half of an error body, whichever variant it is.
fn detail(body: &Value) -> String {
    ["message", "reason", "error"]
        .iter()
        .filter_map(|field| body.get(field).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
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

// ── AC: two levels, each level's approvers, denial audited with reason ───────

/// The headline walk, and the only test in this file that asserts on the
/// whole feature at once.
///
/// A platform-team runbook climbs to Engineering and then to ACME. Each
/// hop is refused for the level below's curator — bindings inherit
/// downward, so a team curator holds nothing at the department and a
/// department curator holds nothing at the org — and carried by that
/// level's own. Between the hops, the payments team (a sibling that
/// cannot read platform, and never will) starts receiving the runbook;
/// after the second, so does a team in a different department entirely.
/// That is what makes it a promotion rather than a row: the audience
/// widened, at each level, under that level's approvers.
///
/// The approver *sets* are the pack's, not the test's: under
/// `regulated-strict` a team publication takes one curator, and a
/// department or org publication takes a curator **and** a steward, two
/// distinct people (the FLOW-3 matrix golden, rows `memory internal
/// team|department|org`). So the two hops need four principals between
/// them and none of them is the team's curator — which is what "each
/// level's approvers" means when the levels differ in kind as well as in
/// place.
#[tokio::test]
async fn knowledge_climbs_two_levels_under_each_levels_approvers() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test", org.platform.id).await;
    seed_user(&pool, tenant, "tara@acme.test", org.platform.id).await;
    seed_user(&pool, tenant, "dana@acme.test", org.eng.id).await;
    seed_user(&pool, tenant, "evan@acme.test", org.eng.id).await;
    seed_user(&pool, tenant, "olive@acme.test", org.org.id).await;
    seed_user(&pool, tenant, "owen@acme.test", org.org.id).await;
    seed_user(&pool, tenant, "pia@acme.test", org.payments.id).await;
    seed_user(&pool, tenant, "sam@acme.test", org.field.id).await;
    // Each principal bound at exactly one level. Roles inherit downward,
    // so binding the org's people at the org would let them approve at the
    // department too; the direction the AC cares about is the other one,
    // and it is proven by denial below: a binding never climbs.
    bind(
        &pool,
        tenant,
        "tara@acme.test",
        org.platform.id,
        Role::Curator,
    )
    .await;
    bind(&pool, tenant, "dana@acme.test", org.eng.id, Role::Curator).await;
    bind(&pool, tenant, "evan@acme.test", org.eng.id, Role::Steward).await;
    bind(&pool, tenant, "olive@acme.test", org.org.id, Role::Curator).await;
    bind(&pool, tenant, "owen@acme.test", org.org.id, Role::Steward).await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        Role::Contributor,
    )
    .await;

    let (ravi_t, tara, dana, evan, olive, owen, pia, sam) = (
        issue("ravi@acme.test", tenant),
        issue("tara@acme.test", tenant),
        issue("dana@acme.test", tenant),
        issue("evan@acme.test", tenant),
        issue("olive@acme.test", tenant),
        issue("owen@acme.test", tenant),
        issue("pia@acme.test", tenant),
        issue("sam@acme.test", tenant),
    );
    let app = router(state(&database_url()));
    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;

    // Baseline: the runbook is the platform team's, and nobody outside it
    // reads it — not the sibling team, not the other department.
    assert!(
        !reads_runbook(&app, &pia).await,
        "the runbook must not reach payments before it climbs"
    );
    assert!(
        !reads_runbook(&app, &sam).await,
        "the runbook must not reach sales before it climbs"
    );

    // ── Hop 1: platform → eng, under Engineering's curator ──────────────
    let hop1 = climb(
        &app,
        &ravi_t,
        org.eng.id,
        org.platform.id,
        &[runbook],
        "promote the signing-key runbook to Engineering",
    )
    .await;

    // Tara curates the platform team. She holds nothing at Engineering,
    // because role bindings inherit downward and never up — so the level
    // below cannot approve the level above's publication.
    let (status, denied) = post(
        &app,
        &tara,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a team curator must not review at the department: {denied}"
    );

    let (status, approved) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the dept curator's approval: {approved}"
    );
    assert_eq!(
        approved["state"].as_str(),
        Some("open"),
        "a department publication is not one curator's to make: {approved}"
    );
    assert!(
        approved["outstanding"]
            .as_str()
            .is_some_and(|outstanding| outstanding.contains("steward")),
        "and the response says what it still lacks: {approved}"
    );
    // The department's steward is the second of the two distinct
    // approvers the matrix asks for at this scope kind.
    let (status, approved) = post(
        &app,
        &evan,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the dept steward's approval: {approved}"
    );
    assert_eq!(
        approved["state"].as_str(),
        Some("approved"),
        "curator + steward, two distinct people, satisfies a department: {approved}"
    );
    // Dana runs the effect, not Evan: publishing takes `MemoryRead` too,
    // and `steward` is an authority role that reads no content in any pack
    // (ADR-0031 decision 12 — nobody publishes what they cannot read).
    let (status, published) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing hop 1: {published}");
    assert_eq!(
        published["scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str()),
        "the department's channel is what moved: {published}"
    );

    // The promotion, from the reader's side. Pia is in payments; she has
    // never been able to read platform and still cannot — what changed is
    // that Engineering published the runbook, and Engineering is on her
    // chain.
    assert!(
        reads_runbook(&app, &pia).await,
        "after the climb, the sibling team must receive the runbook"
    );
    assert!(
        !reads_runbook(&app, &sam).await,
        "a department publication must not reach another department"
    );
    // It composes as reviewed, under the department's own section — the
    // reader is never shown a scope they cannot see.
    let block = inject(&app, &pia).await;
    let text = block["text"].as_str().expect("block text");
    assert!(
        !text.contains("[unreviewed]"),
        "climbed material composes as published, not derived: {text}"
    );
    assert!(
        text.contains("acme/eng") && !text.contains("platform"),
        "a climbed entry sits in the publishing scope's section: {text}"
    );

    // ── Hop 2: eng → org, on material eng holds only by publishing it ───
    // The record still lives at the platform team. Engineering can propose
    // it onward because its published tree names it at exactly its current
    // address, which is the second sense of holding material.
    let hop2 = climb(
        &app,
        &dana,
        org.org.id,
        org.eng.id,
        &[runbook],
        "promote the signing-key runbook to ACME",
    )
    .await;
    let (status, denied) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop2}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a department curator must not review at the org: {denied}"
    );

    // Denial at this level, with a reason an auditor reads.
    let (status, rejected) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2}/reject"),
        json!({"reason": "org-wide runbooks need the platform team named as owner"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rejecting hop 2: {rejected}");
    assert_eq!(rejected["state"].as_str(), Some("rejected"));
    assert!(
        !reads_runbook(&app, &sam).await,
        "a rejected climb must change nothing"
    );

    // A revision is a new proposal (ADR-0032 decision 12), and this one
    // carries the org's approval.
    let hop2b = climb(
        &app,
        &dana,
        org.org.id,
        org.eng.id,
        &[runbook],
        "promote the signing-key runbook to ACME (owner named)",
    )
    .await;
    let (status, approved) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2b}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the org curator's approval: {approved}"
    );
    let (status, approved) = post(
        &app,
        &owen,
        &format!("/v1/proposals/{hop2b}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the org steward's approval: {approved}"
    );
    assert_eq!(
        approved["state"].as_str(),
        Some("approved"),
        "the org asks for its own curator and its own steward: {approved}"
    );
    let (status, published) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2b}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing hop 2: {published}");

    assert!(
        reads_runbook(&app, &sam).await,
        "after the second climb, another department must receive the runbook"
    );

    // ── The trail: two climbs, one denial, both levels named ────────────
    let opened = events(&pool, tenant, "vedaflow.proposal.opened").await;
    let climbs: Vec<&StoredEvent> = opened
        .iter()
        .filter(|event| event.payload.get("climb").is_some_and(|c| !c.is_null()))
        .collect();
    assert_eq!(
        climbs.len(),
        3,
        "three climbs were opened (one was rejected)"
    );
    assert_eq!(
        climbs[0].payload["source_scope_id"].as_str(),
        Some(org.platform.id.to_string().as_str())
    );
    assert_eq!(
        climbs[0].payload["target_scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str())
    );
    assert_eq!(
        climbs[0].payload["climb"]["levels"].as_u64(),
        Some(1),
        "platform → eng is one level"
    );
    // The disclosure decision is on the event beside the open decision:
    // two governed decisions, two recorded contexts (ADR-0034 decision 9).
    assert!(
        climbs[0].payload["climb"]["source_read"]["pack"].is_string(),
        "the proposer's read at the source must be recorded: {}",
        climbs[0].payload
    );

    let rejections = events(&pool, tenant, "vedaflow.proposal.rejected").await;
    assert_eq!(rejections.len(), 1, "exactly one denial");
    let denial = &rejections[0];
    assert_eq!(
        denial.payload["target_scope_id"].as_str(),
        Some(org.org.id.to_string().as_str()),
        "the denial names the level it happened at"
    );
    assert_eq!(
        denial.payload["source_scope_id"].as_str(),
        Some(org.eng.id.to_string().as_str()),
        "and what it refused to take"
    );
    assert_eq!(
        denial.payload["reason"].as_str(),
        Some("org-wide runbooks need the platform team named as owner"),
        "a denial an auditor cannot read the reason for is not a review"
    );

    let publications = events(&pool, tenant, "vedaflow.channel.published").await;
    let climbed: Vec<(String, String)> = publications
        .iter()
        .filter_map(|event| {
            Some((
                event.payload.get("source_scope_id")?.as_str()?.to_owned(),
                event.payload.get("target_scope_id")?.as_str()?.to_owned(),
            ))
        })
        .collect();
    assert_eq!(
        climbed,
        [
            (org.platform.id.to_string(), org.eng.id.to_string()),
            (org.eng.id.to_string(), org.org.id.to_string()),
        ],
        "the chain shows the runbook climbing, hop by hop"
    );

    // The chain still verifies with all of it in.
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

// ── The direction rule ───────────────────────────────────────────────────────

/// A promotion climbs the hierarchy; it does not cross it and it does not
/// descend. Both refusals name the rule rather than collapsing into a
/// generic "invalid", because a caller that meant to climb needs to be
/// told which way is up.
#[tokio::test]
async fn a_climb_goes_up_never_sideways_and_never_down() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test", org.platform.id).await;
    // Curator at both teams *and* the department, so every refusal below
    // is the direction rule talking and not the PDP.
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        Role::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.payments.id,
        Role::Curator,
    )
    .await;
    bind(&pool, tenant, "ravi@acme.test", org.eng.id, Role::Curator).await;
    let token = issue("ravi@acme.test", tenant);
    let app = router(state(&database_url()));
    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;

    // Sideways: payments is a sibling of platform, not an ancestor.
    let (status, sideways) = open_climb(
        &app,
        &token,
        org.payments.id,
        Some(org.platform.id),
        &[runbook],
        "climb sideways",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "sideways slipped through: {sideways}"
    );
    assert!(
        detail(&sideways).contains("not an ancestor"),
        "the refusal must name the direction rule: {sideways}"
    );

    // Downward: proposing the department's material into a team under it
    // would put content behind *fewer* approvers than it already has.
    let (status, down) = open_climb(
        &app,
        &token,
        org.platform.id,
        Some(org.eng.id),
        &[runbook],
        "climb down",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a descent slipped through: {down}"
    );
    assert!(
        detail(&down).contains("not an ancestor"),
        "the refusal must name the direction rule: {down}"
    );

    // And straight up is fine, from the same principal on the same record:
    // the three refusals above were about direction, nothing else.
    let (status, up) = open_climb(
        &app,
        &token,
        org.eng.id,
        Some(org.platform.id),
        &[runbook],
        "climb up",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the climb itself must work: {up}");
}

// ── The disclosure rule ──────────────────────────────────────────────────────

/// The rule that makes a climb safe is a read the proposer already holds
/// at the source (ADR-0034 decision 1), and the privacy floor is what
/// enforces it — with no clause about personal scopes anywhere in the
/// promotion code.
///
/// So a team's own curator cannot climb a teammate's personal memory (no
/// pack permits `MemoryRead` there), and the teammate can climb their own
/// (the self permit does). That second half is the sentence the feature
/// exists for: this is how tribal knowledge leaves one person's head.
#[tokio::test]
async fn only_a_principal_who_can_read_the_source_may_climb_it() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test", org.platform.id).await;
    seed_user(&pool, tenant, "tara@acme.test", org.platform.id).await;
    bind(
        &pool,
        tenant,
        "tara@acme.test",
        org.platform.id,
        Role::Curator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        Role::Contributor,
    )
    .await;
    let (ravi_t, tara) = (
        issue("ravi@acme.test", tenant),
        issue("tara@acme.test", tenant),
    );
    let app = router(state(&database_url()));

    // Ravi's own note, at his own personal leaf — where every extracted
    // memory the write path produces lands (ADR-0033 decision 8).
    let note = seed_record(&pool, tenant, ravi.scope_id, ravi.id, RUNBOOK).await;

    // Tara curates the team and can publish there. She still cannot climb
    // Ravi's note into it, because she cannot read it.
    let (status, denied) = open_climb(
        &app,
        &tara,
        org.platform.id,
        Some(ravi.scope_id),
        &[note],
        "promote a teammate's note",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a curator climbed a teammate's personal memory: {denied}"
    );

    // Ravi can, and that is the whole point.
    let (status, opened) = open_climb(
        &app,
        &ravi_t,
        org.platform.id,
        Some(ravi.scope_id),
        &[note],
        "promote my runbook to the team",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the owner's own climb: {opened}");
    assert_eq!(
        opened["source_scope_id"].as_str(),
        Some(ravi.scope_id.to_string().as_str())
    );

    // The disclosure is real and it is the point: Tara reviews content she
    // cannot read at its source. Requiring otherwise would make the
    // product's own floor unsatisfiable — `compliance` reads no memory in
    // any pack — and would leave personal material unclimbable forever.
    let id = opened["id"].as_str().expect("proposal id");
    let request = Request::builder()
        .method("GET")
        .uri(format!("/v1/proposals/{id}"))
        .header("authorization", format!("Bearer {tara}"))
        .body(Body::empty())
        .expect("build request");
    let (status, detail_body) = call(&app, request).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the reviewer must see it: {detail_body}"
    );
    assert!(
        detail_body["members"][0]["content"]
            .as_str()
            .is_some_and(|content| content.contains(RUNBOOK)),
        "a reviewer who cannot read the source reviews what the proposal shows them: {detail_body}"
    );
}

// ── What a source holds ──────────────────────────────────────────────────────

/// A climb carries only material its source scope holds, in one of the two
/// senses ADR-0034 decision 3 names — and an edit takes material out of
/// both at once, because the address moves with the content.
#[tokio::test]
async fn a_climb_carries_only_what_its_source_holds() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let org = seed_hierarchy(&pool, tenant).await;
    let ravi = seed_user(&pool, tenant, "ravi@acme.test", org.platform.id).await;
    seed_user(&pool, tenant, "dana@acme.test", org.eng.id).await;
    seed_user(&pool, tenant, "evan@acme.test", org.eng.id).await;
    bind(
        &pool,
        tenant,
        "ravi@acme.test",
        org.platform.id,
        Role::Curator,
    )
    .await;
    bind(&pool, tenant, "dana@acme.test", org.eng.id, Role::Curator).await;
    // A department publication takes a curator and a steward, two distinct
    // people, whichever route it arrives by (the FLOW-3 matrix golden).
    bind(&pool, tenant, "evan@acme.test", org.eng.id, Role::Steward).await;
    let (ravi_t, dana, evan) = (
        issue("ravi@acme.test", tenant),
        issue("dana@acme.test", tenant),
        issue("evan@acme.test", tenant),
    );
    let app = router(state(&database_url()));

    let runbook = seed_record(&pool, tenant, org.platform.id, ravi.id, RUNBOOK).await;
    let elsewhere = seed_record(
        &pool,
        tenant,
        org.payments.id,
        ravi.id,
        "payments closes the ledger on the last business day",
    )
    .await;

    // Not held at the named source: platform neither has the payments
    // record nor publishes it.
    let (status, refused) = open_climb(
        &app,
        &ravi_t,
        org.eng.id,
        Some(org.platform.id),
        &[runbook, elsewhere],
        "climb someone else's record along with mine",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unheld material climbed: {refused}"
    );
    assert!(
        detail(&refused).contains(&elsewhere.to_string()),
        "the refusal must name the record it refused, not the request: {refused}"
    );

    // The department cannot propose the runbook onward yet either: it
    // neither holds nor publishes it until the first hop lands.
    let (status, early) = open_climb(
        &app,
        &dana,
        org.org.id,
        Some(org.eng.id),
        &[runbook],
        "climb from a scope that has not received it",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty source climbed: {early}"
    );

    // Land the first hop, then the second sense opens up.
    let hop1 = climb(
        &app,
        &ravi_t,
        org.eng.id,
        org.platform.id,
        &[runbook],
        "promote the runbook to Engineering",
    )
    .await;
    post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    post(
        &app,
        &evan,
        &format!("/v1/proposals/{hop1}/approve"),
        Value::Null,
    )
    .await;
    let (status, published) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{hop1}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publishing hop 1: {published}");

    let hop2 = climb(
        &app,
        &dana,
        org.org.id,
        org.eng.id,
        &[runbook],
        "the department proposes onward what the team climbed into it",
    )
    .await;

    // Now edit the record. It falls out of *both* senses at once — the
    // address it was approved at no longer matches, and the department's
    // tree no longer names its current content — so the publication is
    // refused by name rather than carried up on a stale review.
    let mut current = records::current(&pool, runbook)
        .await
        .expect("read record")
        .expect("record exists");
    current.state.content = format!("{RUNBOOK} (superseded)");
    records::update(
        &pool,
        runbook,
        &current.state,
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("edit record");

    seed_user(&pool, tenant, "olive@acme.test", org.org.id).await;
    seed_user(&pool, tenant, "owen@acme.test", org.org.id).await;
    bind(&pool, tenant, "olive@acme.test", org.org.id, Role::Curator).await;
    bind(&pool, tenant, "owen@acme.test", org.org.id, Role::Steward).await;
    let (olive, owen) = (
        issue("olive@acme.test", tenant),
        issue("owen@acme.test", tenant),
    );
    post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2}/approve"),
        Value::Null,
    )
    .await;
    let (status, approved) = post(
        &app,
        &owen,
        &format!("/v1/proposals/{hop2}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        approved["state"].as_str(),
        Some("approved"),
        "the org's approvers signed off (status {status}): {approved}"
    );
    let (status, conflict) = post(
        &app,
        &olive,
        &format!("/v1/proposals/{hop2}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an edited record published on a stale review: {conflict}"
    );
    assert!(
        detail(&conflict).contains(&runbook.to_string()),
        "the refusal must name the record that moved: {conflict}"
    );
}

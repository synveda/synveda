//! FLOW-3's other two acceptance criteria (ADR-0032): **a memory
//! promotion team→published E2E with 1 curator**, and **a restricted
//! asset requires compliance + dual approval** — both over the product's
//! own HTTP surfaces, under the PDP, against a live Postgres.
//!
//! The full matrix golden lives in `synveda-policy/tests/approvals.rs`;
//! this suite is about what happens when a real principal pushes real
//! records across the trust boundary.
//!
//! Around the two AC walks: the direct publish route resolving the *same*
//! matrix and refusing a restricted record by name (ADR-0032 decision 8),
//! approvals binding bytes so an edit after approval refuses (decision 6),
//! the merge commit whose second parent is the proposal (decision 10), a
//! curator file adding a named approver without granting them anything
//! (decision 13), the PDP gates on every verb, rejection and withdrawal,
//! and the audit chain that carries the requirement as it was resolved.
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

const SECRET: &[u8] = b"flow-3-test-secret";

/// The team knowledge a contributor wants promoted.
const PROCEDURE: &str = "rotate the signing key every 90 days, on the first tuesday";
/// Material classified `restricted`: the floor's cell.
const RESTRICTED: &str = "the incident bridge for a sev-1 is opened by the on-call lead";

// ── Harness ──────────────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-flow3-tests")
        .join(TenantId::new().to_string())
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
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
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
                "skipping FLOW-3 proposal test: DATABASE_URL is not set \
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
    let slug = format!("flow3-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-3 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// org → dept → two teams, plus the quarantine scope AUTH-2 needs.
async fn seed_hierarchy(pool: &PgPool, tenant: TenantId) -> (HierarchyNode, HierarchyNode) {
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
    .expect("create team");
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
    .expect("create second team");
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
    (platform, payments)
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
    sensitivity: Sensitivity,
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
            sensitivity,
            provenance: json!({"source": "flow-3 acceptance test"}),
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

async fn put(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("PUT")
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

async fn open_proposal(
    app: &Router,
    token: &str,
    scope: ScopeId,
    records: &[RecordId],
    title: &str,
) -> (StatusCode, Value) {
    post(
        app,
        token,
        "/v1/proposals",
        json!({
            "scope_id": scope,
            "record_ids": records,
            "title": title,
        }),
    )
    .await
}

/// The human-readable half of an [`synveda_gateway::error::ApiError`]
/// body, whichever variant it is: `Invalid` renders `message`,
/// `PolicyDenied` renders `reason`.
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
    let mut all = synveda_audit::tail(&mut tx, tenant, 400)
        .await
        .expect("read chain");
    all.reverse();
    all.into_iter()
        .filter(|event| event.action == action)
        .collect()
}

/// Every commit reachable as a parent of `commit`, one hop.
async fn parents_of(pool: &PgPool, tenant: TenantId, commit: &str) -> Vec<String> {
    let bytes = (0..commit.len() / 2)
        .map(|index| u8::from_str_radix(&commit[index * 2..index * 2 + 2], 16).expect("hex commit"))
        .collect::<Vec<u8>>();
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("tenant tx");
    sqlx::query_scalar!(
        "select parent_hash from vedaflow_commit_parents
         where tenant_id = $1 and commit_hash = $2
         order by ordinal",
        tenant.as_uuid(),
        &bytes[..],
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read parents")
    .into_iter()
    .map(|parent| {
        parent
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    })
    .collect()
}

// ── AC 1: a memory promotion team→published, with one curator ────────────────

/// The headline walk. A contributor at the platform team proposes one of
/// the team's records for publication; the team's curator reviews it and
/// approves; the curator runs the effect. Along the way every FLOW-3
/// property that makes the walk mean anything is asserted: the
/// requirement resolved from the pack, the proposal that is not yet
/// approved, the approval that carries the roles it counted under, the
/// merge commit whose second parent is the proposal, and one audit chain
/// carrying all of it.
#[tokio::test]
async fn a_memory_promotion_needs_one_curator_and_publishes_through_the_proposal() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    // A contributor writes; a curator reviews. That separation is the
    // whole point of the feature.
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let app = router(state(&database_url()));

    // ── Open ────────────────────────────────────────────────────────────
    let (status, opened) = open_proposal(
        &app,
        &dana,
        team.id,
        &[record],
        "promote the key rotation runbook",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open failed: {opened}");
    let proposal_id = opened["id"].as_str().expect("proposal id").to_owned();
    let proposal_commit = opened["commit"].as_str().expect("commit").to_owned();
    assert_eq!(
        opened["state"], "open",
        "a fresh proposal is not approved: {opened}"
    );
    // regulated-strict at a team asks for one curator (the golden table).
    assert_eq!(
        opened["required"]["roles"],
        json!([{"role": "curator", "count": 1}]),
        "the requirement came from the pack: {opened}"
    );
    assert_eq!(opened["required"]["distinct_approvers"], 1);
    assert_eq!(opened["required"]["origins"], json!(["pack"]));
    assert_eq!(opened["outstanding"], "curator × 1, 1 distinct approver(s)");

    // The proposer cannot publish it: their own roles satisfy nothing.
    let (status, refused) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{proposal_id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a contributor holds no ChannelPublish: {refused}"
    );

    // ── Review ──────────────────────────────────────────────────────────
    // The curator sees the proposal's content — that is what a review is.
    let (status, detail) = get(&app, &cora, &format!("/v1/proposals/{proposal_id}")).await;
    assert_eq!(status, StatusCode::OK, "curator cannot read it: {detail}");
    assert_eq!(detail["members"][0]["content"], PROCEDURE);
    assert_eq!(
        detail["members"][0]["unchanged"], true,
        "the record still hashes to what was proposed"
    );

    let (status, approved) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{proposal_id}/approve"),
        json!({"comment": "matches the runbook we agreed"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "approve failed: {approved}");
    assert_eq!(
        approved["state"], "approved",
        "one curator satisfies the requirement: {approved}"
    );
    assert_eq!(approved["outstanding"], "nothing");
    assert_eq!(approved["counted_roles"], json!(["curator"]));

    // ── Effect ──────────────────────────────────────────────────────────
    let (status, published) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{proposal_id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {published}");
    assert_eq!(published["members"], 1);
    assert_eq!(published["added"], 1);
    assert_eq!(published["proposal_commit"], proposal_commit);

    // The published commit is a merge: head first (there is none — this is
    // the channel's first commit), then the proposal. Lineage is a fact
    // about the graph, not a join (ADR-0032 decision 10).
    let channel_commit = published["commit"].as_str().expect("commit").to_owned();
    let parents = parents_of(&pool, tenant, &channel_commit).await;
    assert_eq!(
        parents,
        vec![proposal_commit.clone()],
        "the publication must descend from the proposal it is the effect of"
    );

    // ── State ───────────────────────────────────────────────────────────
    let (_, after) = get(&app, &cora, &format!("/v1/proposals/{proposal_id}")).await;
    assert_eq!(after["state"], "published");
    assert_eq!(after["approvals"][0]["verdict"], "approve");
    assert_eq!(after["approvals"][0]["roles"], json!(["curator"]));
    assert_eq!(after["approvals"][0]["counts"], true);

    // Publishing twice is refused: the proposal is history now.
    let (status, again) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{proposal_id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "double publish: {again}");

    // ── The trail ───────────────────────────────────────────────────────
    let opened_events = events(&pool, tenant, "vedaflow.proposal.opened").await;
    assert_eq!(opened_events.len(), 1);
    assert_eq!(opened_events[0].payload["proposal_id"], proposal_id);
    assert_eq!(
        opened_events[0].payload["approvals"]["required"]["distinct_approvers"], 1,
        "the trail records the requirement as it was resolved"
    );
    // Ids and addresses, never content.
    let opened_json = opened_events[0].payload.to_string();
    assert!(
        !opened_json.contains(PROCEDURE),
        "the audit payload must not carry record content"
    );

    let approvals = events(&pool, tenant, "vedaflow.proposal.approved").await;
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].payload["roles"], json!(["curator"]));
    assert_eq!(approvals[0].payload["approvals"]["satisfied"], true);

    // The effect is the same action a direct publish emits, with the
    // proposal named (ADR-0032 decision 18) — not a second action.
    let publishes = events(&pool, tenant, "vedaflow.channel.published").await;
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].payload["proposal_id"], proposal_id);
    assert_eq!(publishes[0].payload["commit"], channel_commit);
    assert_eq!(
        publishes[0].payload["approved_by"][0]["roles"],
        json!(["curator"])
    );

    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("tenant tx");
    let verification = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify chain");
    assert!(
        matches!(verification, synveda_audit::ChainVerification::Valid { .. }),
        "chain broken: {verification}"
    );
}

// ── AC 2: restricted needs compliance and two distinct approvers ─────────────

/// The floor, on the product surfaces. A `restricted` record cannot be
/// published by one curator through *either* route: the direct one
/// refuses and names the proposal path, and the proposal itself refuses
/// to publish until a compliance reviewer has also approved — two
/// distinct identities, whatever roles either of them holds.
#[tokio::test]
async fn a_restricted_record_takes_compliance_and_two_distinct_approvers() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    seed_user(&pool, tenant, "quinn@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    // Quinn is compliance *and* a curator, which is the interesting case:
    // holding both roles satisfies both role lines and still counts as one
    // identity, so dual approval binds them exactly as intended.
    bind(&pool, tenant, "quinn@acme.test", team.id, Role::Compliance).await;
    bind(&pool, tenant, "quinn@acme.test", team.id, Role::Curator).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let quinn = issue("quinn@acme.test", tenant);

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Restricted,
        RESTRICTED,
    )
    .await;
    let app = router(state(&database_url()));

    // ── The direct route refuses, and says why ──────────────────────────
    let (status, refused) = post(
        &app,
        &cora,
        &format!("/v1/channels/{}/publish", team.id),
        json!({"record_ids": [record], "message": "straight to published"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a lone curator published restricted material: {refused}"
    );
    let reason = detail(&refused);
    assert!(
        reason.contains("compliance") && reason.contains("proposal"),
        "the refusal must name what is missing and where to go: {reason}"
    );

    // Even the double-role holder cannot do it alone: two *distinct*
    // identities is the requirement, and one person is one identity.
    let (status, refused) = post(
        &app,
        &quinn,
        &format!("/v1/channels/{}/publish", team.id),
        json!({"record_ids": [record], "message": "i hold both roles"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "one person holding curator and compliance published alone: {refused}"
    );

    // ── The proposal route: two approvals, one of them compliance ───────
    let (status, opened) = open_proposal(
        &app,
        &dana,
        team.id,
        &[record],
        "publish the sev-1 bridge procedure",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "open failed: {opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    assert_eq!(opened["required"]["distinct_approvers"], 2);
    assert_eq!(
        opened["required"]["origins"],
        json!(["floor", "pack"]),
        "the floor and the pack both contributed: {opened}"
    );
    assert!(
        opened["required"]["roles"]
            .as_array()
            .expect("roles")
            .iter()
            .any(|role| role["role"] == "compliance"),
        "compliance is required: {opened}"
    );

    // One curator is not enough — and the response says exactly what is
    // still missing.
    let (status, first) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "first approval failed: {first}");
    assert_eq!(first["state"], "open", "still short: {first}");
    assert_eq!(
        first["outstanding"],
        "compliance × 1, 1 distinct approver(s)"
    );

    let (status, early) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an unapproved proposal published: {early}"
    );

    // A second approval by the *same* identity is refused, not counted
    // twice.
    let (status, again) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "double vote counted: {again}");

    // Compliance closes it out.
    let (status, second) = post(
        &app,
        &quinn,
        &format!("/v1/proposals/{id}/approve"),
        json!({"comment": "classification reviewed"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "compliance approval failed: {second}"
    );
    assert_eq!(second["state"], "approved", "still not approved: {second}");
    assert!(
        second["counted_roles"]
            .as_array()
            .expect("roles")
            .contains(&json!("compliance"))
    );

    // The effect still needs `ChannelPublish`: compliance alone holds none
    // in any pack, which is exactly why the deciding approval does not
    // publish (ADR-0032 decision 9). Quinn holds curator too, so quinn can.
    let (status, published) = post(
        &app,
        &quinn,
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {published}");
    assert_eq!(published["members"], 1);

    let publishes = events(&pool, tenant, "vedaflow.channel.published").await;
    assert_eq!(publishes.len(), 1);
    let approvers = publishes[0].payload["approved_by"]
        .as_array()
        .expect("approved_by");
    assert_eq!(approvers.len(), 2, "the trail names both approvers");
    assert_eq!(
        publishes[0].payload["approvals"]["required"]["distinct_approvers"],
        2
    );
    assert_eq!(publishes[0].payload["sensitivity"], "restricted");
}

/// A `compliance` reviewer with no publish grant reaches the deciding vote
/// and stops there: the proposal is `approved`, and its effect refuses.
/// This is the case that decided against auto-publishing.
#[tokio::test]
async fn compliance_can_approve_but_not_publish() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    seed_user(&pool, tenant, "cleo@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    bind(&pool, tenant, "cleo@acme.test", team.id, Role::Compliance).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let cleo = issue("cleo@acme.test", tenant);

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Restricted,
        RESTRICTED,
    )
    .await;
    let app = router(state(&database_url()));
    let (_, opened) = open_proposal(&app, &dana, team.id, &[record], "restricted promotion").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    let (status, decided) = post(
        &app,
        &cleo,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "compliance approval failed: {decided}"
    );
    assert_eq!(decided["state"], "approved");

    // Compliance cast the deciding vote and holds no ChannelPublish. Had
    // the approval published, this call would have run under system
    // authority — which is the PDP bypass ADR-0032 decision 9 refuses.
    let (status, refused) = post(
        &app,
        &cleo,
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "compliance published without ChannelPublish: {refused}"
    );

    // The curator runs the effect instead.
    let (status, published) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish failed: {published}");
}

// ── Approvals bind bytes ─────────────────────────────────────────────────────

/// The attack the whole design exists to stop: approve a proposal, edit
/// the record, publish. Publication recomputes every member's address from
/// the record as it stands now and refuses when it moved (ADR-0032
/// decision 6).
#[tokio::test]
async fn editing_a_record_after_approval_refuses_the_publish() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let app = router(state(&database_url()));
    let (_, opened) = open_proposal(&app, &dana, team.id, &[record], "promote the runbook").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;

    // The content moves under the completed review.
    records::update(
        &pool,
        record,
        &RecordState {
            scope_id: team.id,
            owner_id: author.id,
            kind: RecordKind::Derived,
            class: RecordClass::Procedure,
            content: "rotate the signing key whenever you feel like it".to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "flow-3 acceptance test"}),
            valid_from: chrono::Utc::now() - chrono::Duration::hours(1),
            valid_to: None,
        },
        &RecordEmbedding {
            model: DeterministicEmbedder::MODEL.to_owned(),
            vector: vec![0.25; 16],
        },
    )
    .await
    .expect("edit the record");

    let (status, refused) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an edited record published under a stale approval: {refused}"
    );

    // And the review surface says so before anyone tries.
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{id}")).await;
    assert_eq!(
        detail["members"][0]["unchanged"], false,
        "the drift must be visible in the review: {detail}"
    );
}

// ── Curator files ────────────────────────────────────────────────────────────

/// A curator file adds a named approver and grants them nothing: the file
/// is written by a steward, the named subject's approval becomes required,
/// and a subject the pack denies makes the proposal unsatisfiable rather
/// than an approver (ADR-0032 decision 13).
#[tokio::test]
async fn a_curator_file_adds_a_required_approver_without_granting_anything() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    seed_user(&pool, tenant, "sam@acme.test", team.id).await;
    seed_user(&pool, tenant, "vic@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    bind(&pool, tenant, "sam@acme.test", team.id, Role::Steward).await;
    // Vic is named in the file but holds only viewer: the file cannot make
    // them an approver.
    bind(&pool, tenant, "vic@acme.test", team.id, Role::Viewer).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let sam = issue("sam@acme.test", tenant);
    let vic = issue("vic@acme.test", tenant);
    let app = router(state(&database_url()));

    // A contributor cannot edit it; a steward can.
    let file = "# platform owns its runbooks\nmemory/* @sam@acme.test\n";
    let (status, refused) = put(
        &app,
        &dana,
        &format!("/v1/hierarchy/nodes/{}/curators", team.id),
        json!({"source": file, "message": "seize the review"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a contributor edited the curator file: {refused}"
    );

    let (status, written) = put(
        &app,
        &sam,
        &format!("/v1/hierarchy/nodes/{}/curators", team.id),
        json!({"source": file, "message": "platform runbooks need sam"}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "curator file write failed: {written}"
    );
    assert_eq!(written["rules"], 1);

    // It reads back as authored, bytes and all — FLOW-6 diffs this.
    let (status, read) = get(
        &app,
        &sam,
        &format!("/v1/hierarchy/nodes/{}/curators", team.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "curator file read failed: {read}");
    assert_eq!(read["source"], file);
    assert_eq!(read["effective_at"], json!(team.id));

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let (_, opened) = open_proposal(&app, &dana, team.id, &[record], "promote the runbook").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    assert_eq!(
        opened["required"]["subjects"],
        json!(["sam@acme.test"]),
        "the file's named approver joined the requirement: {opened}"
    );
    // The named-subject line is what forces a second person here: sam is
    // a steward, not a curator, so the pack's `curator × 1` and the file's
    // `@sam` cannot be satisfied by one identity. (Had sam held curator,
    // one approval would rightly satisfy both — the requirement is the
    // lines, not the count.)
    assert!(
        opened["required"]["distinct_approvers"]
            .as_u64()
            .expect("distinct_approvers")
            >= 1
    );
    assert!(
        opened["required"]["origins"]
            .as_array()
            .expect("origins")
            .iter()
            .any(|origin| origin.as_str().is_some_and(|o| o.starts_with("curators:"))),
        "the trail must say where the extra requirement came from: {opened}"
    );

    // The curator alone no longer suffices.
    let (_, first) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(first["state"], "open", "the file was ignored: {first}");
    assert!(
        first["outstanding"]
            .as_str()
            .expect("outstanding")
            .contains("@sam@acme.test"),
        "the named approver must be outstanding: {first}"
    );

    // Vic is not named and holds only viewer: no route to a vote.
    let (status, denied) = post(
        &app,
        &vic,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a viewer cast a verdict: {denied}"
    );

    // Sam, who is named *and* passes ProposalReview, closes it.
    let (status, second) = post(
        &app,
        &sam,
        &format!("/v1/proposals/{id}/approve"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "named approval failed: {second}");
    assert_eq!(second["state"], "approved");

    // And the file inherits: a proposal at the *department* above it is
    // unaffected, because resolution is nearest-ancestor-first from the
    // target, not a union.
    let (_, dept_read) = get(
        &app,
        &sam,
        &format!("/v1/hierarchy/nodes/{}/curators", team.id),
    )
    .await;
    assert_eq!(dept_read["effective_at"], json!(team.id));
}

// ── Lifecycle and gates ──────────────────────────────────────────────────────

/// Rejection is terminal and carries its reason; withdrawal is the
/// proposer's act alone; and a closed proposal admits nothing further.
#[tokio::test]
async fn rejection_is_terminal_and_withdrawal_belongs_to_the_proposer() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let app = router(state(&database_url()));

    // ── Rejection ───────────────────────────────────────────────────────
    let first = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let (_, opened) = open_proposal(&app, &dana, team.id, &[first], "promote this").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    // A reason is mandatory: a rejection nobody can read is not a review.
    let (status, empty) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/reject"),
        json!({"reason": ""}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "reasonless reject: {empty}"
    );

    let (status, rejected) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/reject"),
        json!({"reason": "supersedes an existing published runbook"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "reject failed: {rejected}");
    assert_eq!(rejected["state"], "rejected");
    assert_eq!(
        rejected["close_reason"],
        "supersedes an existing published runbook"
    );

    for verb in ["approve", "publish", "withdraw"] {
        let (status, after) = post(
            &app,
            &cora,
            &format!("/v1/proposals/{id}/{verb}"),
            Value::Null,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{verb} on a rejected proposal: {after}"
        );
    }
    let rejections = events(&pool, tenant, "vedaflow.proposal.rejected").await;
    assert_eq!(rejections.len(), 1);
    assert_eq!(
        rejections[0].payload["reason"],
        "supersedes an existing published runbook"
    );

    // ── Withdrawal ──────────────────────────────────────────────────────
    let second = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        "a second procedure worth promoting",
    )
    .await;
    let (_, opened) = open_proposal(&app, &dana, team.id, &[second], "and this one").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    // A curator cannot withdraw someone else's proposal — they reject it,
    // with a reason, which is what leaves a trail.
    let (status, refused) = post(
        &app,
        &cora,
        &format!("/v1/proposals/{id}/withdraw"),
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a reviewer withdrew someone else's proposal: {refused}"
    );

    let (status, withdrawn) = post(
        &app,
        &dana,
        &format!("/v1/proposals/{id}/withdraw"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "withdraw failed: {withdrawn}");
    assert_eq!(withdrawn["state"], "withdrawn");
    assert_eq!(
        events(&pool, tenant, "vedaflow.proposal.withdrawn")
            .await
            .len(),
        1
    );
}

/// The PDP gates, and the uniform 404 that keeps a cross-tenant probe from
/// learning anything: a proposal at another tenant is not found, never
/// denied.
#[tokio::test]
async fn the_pdp_gates_every_verb_and_foreign_proposals_are_not_found() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let Some((_, other)) = admitted_tenant().await else {
        return;
    };
    let (team, payments) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    seed_user(&pool, tenant, "nobody@acme.test", payments.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    // Cora curates both teams, so the cross-scope refusal below is the
    // *scope* rule talking rather than the PDP: a principal who may open
    // a proposal at payments still cannot name a platform record in it.
    bind(&pool, tenant, "cora@acme.test", payments.id, Role::Curator).await;
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let outsider = issue("dana@acme.test", other);
    let app = router(state(&database_url()));

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let (_, opened) = open_proposal(&app, &dana, team.id, &[record], "promote the runbook").await;
    let id = opened["id"].as_str().expect("proposal id").to_owned();

    // A curator cannot open a proposal naming a record of a scope that is
    // not the source: payments neither holds the platform record nor
    // publishes it, and FLOW-5's climb would not help — payments is a
    // sibling, not an ancestor (ADR-0034 decisions 2 and 3).
    let (status, cross) =
        open_proposal(&app, &cora, payments.id, &[record], "climb sideways").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "cross-scope promotion slipped through: {cross}"
    );
    assert!(
        detail(&cross).contains("neither holds nor publishes")
            && detail(&cross).contains("source_scope_id"),
        "the refusal must name the rule and the field that expresses a climb: {cross}"
    );

    // Another tenant sees a uniform 404, not a policy denial oracle.
    for uri in [
        format!("/v1/proposals/{id}"),
        format!("/v1/proposals/{id}/approve"),
    ] {
        let (status, body) = if uri.ends_with("approve") {
            post(&app, &outsider, &uri, Value::Null).await
        } else {
            get(&app, &outsider, &uri).await
        };
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a foreign proposal must be not found, not denied: {uri} → {body}"
        );
    }

    // The listing decides too, and the tenant-wide inbox is a
    // tenant-resource read: a *node* binding does not reach it (ADR-0015
    // decision 3), which is the same rule the quarantine queue follows.
    // So neither the contributor nor the node-bound curator sees it...
    for (who, token) in [("contributor", &dana), ("node-bound curator", &cora)] {
        let (status, denied) = get(&app, token, "/v1/proposals").await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a {who} read the tenant-wide inbox: {denied}"
        );
    }

    // ...and a tenant-wide auditor binding does, read-only, which is what
    // FLOW-6's `proposal list` and CNSL-1's hero screen stand on.
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, "cora@acme.test", None, Role::Auditor)
        .await
        .expect("bind tenant-wide auditor");
    tx.commit().await.expect("commit binding");
    let (status, listed) = get(&app, &cora, "/v1/proposals").await;
    assert_eq!(status, StatusCode::OK, "inbox listing failed: {listed}");
    assert_eq!(listed["proposals"].as_array().expect("proposals").len(), 1);
    assert_eq!(listed["proposals"][0]["id"], id);

    // Scoped, a contributor reads their own team's queue (the membership
    // floor MemoryRead has, deliberately mirrored).
    let (status, scoped) = get(
        &app,
        &dana,
        &format!("/v1/proposals?scope_id={}&state=open", team.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "scoped listing failed: {scoped}");
    assert_eq!(scoped["proposals"][0]["id"], id);
}

/// The direct route still works where the matrix is satisfiable by one
/// actor — which is the whole shape of ADR-0032 decision 8, and the
/// reason FLOW-2's own acceptance walk still passes.
#[tokio::test]
async fn a_curator_still_publishes_internal_memory_directly() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, _) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    bind(&pool, tenant, "cora@acme.test", team.id, Role::Curator).await;
    let cora = issue("cora@acme.test", tenant);

    let record = seed_record(
        &pool,
        tenant,
        team.id,
        author.id,
        Sensitivity::Internal,
        PROCEDURE,
    )
    .await;
    let app = router(state(&database_url()));

    let (status, published) = post(
        &app,
        &cora,
        &format!("/v1/channels/{}/publish", team.id),
        json!({"record_ids": [record], "message": "the team's runbook"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "direct publish failed: {published}");
    assert_eq!(
        published["required"]["roles"],
        json!([{"role": "curator", "count": 1}]),
        "the response says what the matrix asked for: {published}"
    );

    // The trail records the single approval that satisfied it.
    let publishes = events(&pool, tenant, "vedaflow.channel.published").await;
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0].payload["approvals"]["satisfied"], true);
    assert_eq!(
        publishes[0].payload["approved_by"][0]["roles"],
        json!(["curator"])
    );
    assert!(
        publishes[0].payload.get("proposal_id").is_none(),
        "a direct publish names no proposal"
    );
}

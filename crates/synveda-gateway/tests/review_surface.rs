//! FLOW-6's gateway half (ADR-0035): what a reviewer is *shown*.
//!
//! The acceptance criterion — full review possible without console — is
//! demonstrated end to end by `demos/flow-6-cli-review.sh`, which drives a
//! whole promotion with nothing but `synveda proposal ...`. This suite
//! pins the half the CLI cannot invent: the proposal's **effect on the
//! target's published channel**, per member, with the bytes on both sides.
//!
//! Every case here is a claim about a rendering that would otherwise be a
//! guess:
//!
//! - `add`/`update`/`none` are the three things publishing can do to a
//!   member, and they are read off the target's tree rather than off the
//!   record's row;
//! - the old side is the object the tree names *now*, so a reviewer sees
//!   what would be overwritten;
//! - the new side is the object the **proposal** names, not the record as
//!   it stands, which is the only honest answer once a record has been
//!   edited under its own review (ADR-0032 decision 6);
//! - a `compliance` reviewer, who holds no `MemoryRead` in any pack, is
//!   shown both sides — the widening ADR-0035 decision 8 makes on purpose,
//!   asserted so it stays a decision rather than becoming an accident;
//! - a climb renders both scope paths, which is what turns a listing of
//!   UUIDs into a review surface.
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

const SECRET: &[u8] = b"flow-6-test-secret";

/// A procedure with lines, because the point of the diff is that an edit
/// to one of them is an edit to one of them.
const RUNBOOK: &str = "check the on-call rota\nrotate the signing key\nfile the change record";
const REVISED: &str =
    "check the on-call rota\nrotate the signing key every 90 days\nfile the change record";

// ── Harness ──────────────────────────────────────────────────────────────────

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-flow6-tests")
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
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(Pdp::new().expect("build the embedded PDP")),
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
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
                "skipping FLOW-6 review-surface test: DATABASE_URL is not set \
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
    let slug = format!("flow6-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "FLOW-6 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

/// acme → eng → platform, plus the quarantine scope AUTH-2 needs.
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
    (platform, eng)
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

fn record_state(scope: ScopeId, owner: IdentityId, content: &str) -> RecordState {
    RecordState {
        scope_id: scope,
        owner_id: owner,
        kind: RecordKind::Derived,
        class: RecordClass::Procedure,
        content: content.to_owned(),
        sensitivity: Sensitivity::Internal,
        provenance: json!({"source": "flow-6 acceptance test"}),
        valid_from: chrono::Utc::now() - chrono::Duration::hours(1),
        valid_to: None,
    }
}

fn embedding() -> RecordEmbedding {
    RecordEmbedding {
        model: DeterministicEmbedder::MODEL.to_owned(),
        vector: vec![0.25; 16],
    }
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
        &record_state(scope, owner, content),
        &embedding(),
    )
    .await
    .expect("insert record");
    id
}

async fn rewrite(
    pool: &PgPool,
    record: RecordId,
    scope: ScopeId,
    owner: IdentityId,
    content: &str,
) {
    records::update(
        pool,
        record,
        &record_state(scope, owner, content),
        &embedding(),
    )
    .await
    .expect("rewrite record")
    .expect("the record exists");
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

/// Opens a proposal and returns its id.
async fn open_proposal(
    app: &Router,
    token: &str,
    target: ScopeId,
    source: Option<ScopeId>,
    records: &[RecordId],
    title: &str,
) -> (Value, String) {
    let mut body = json!({
        "scope_id": target,
        "record_ids": records,
        "title": title,
    });
    if let Some(source) = source {
        body["source_scope_id"] = json!(source);
    }
    let (status, opened) = post(app, token, "/v1/proposals", body).await;
    assert_eq!(status, StatusCode::OK, "open refused: {opened}");
    let id = opened["id"].as_str().expect("proposal id").to_owned();
    (opened, id)
}

/// Approves as each of `approvers` and then publishes as the **first**,
/// which is the two-act sequence ADR-0032 decision 9 requires.
///
/// The approver set is the pack's, not this test's convenience: under
/// `regulated-strict` a team asks for one curator and a department asks
/// for a curator *and* a steward, and the steward cannot run the effect
/// because steward reads no content in any pack (ADR-0034). So the first
/// approver is the curator, and the publish goes to them.
async fn approve_and_publish(app: &Router, approvers: &[&str], id: &str) {
    for token in approvers {
        let (status, approved) = post(
            app,
            token,
            &format!("/v1/proposals/{id}/approve"),
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "approve refused: {approved}");
    }
    let (status, published) = post(
        app,
        approvers[0],
        &format!("/v1/proposals/{id}/publish"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "publish refused: {published}");
}

/// The member of a proposal detail naming `record`.
fn member_for(detail: &Value, record: RecordId) -> &Value {
    detail["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|member| member["record_id"] == json!(record))
        .unwrap_or_else(|| panic!("no member for {record} in {detail}"))
}

/// The `content` field of a canonical memory object, as text.
fn content_of(object: &str) -> String {
    serde_json::from_str::<Value>(object)
        .unwrap_or_else(|err| panic!("object is not canonical JSON ({err}): {object}"))["content"]
        .as_str()
        .unwrap_or_else(|| panic!("object has no content: {object}"))
        .to_owned()
}

// ── The three effects ────────────────────────────────────────────────────────

/// The heart of it: publishing a member can add it, replace an older
/// version, or do nothing, and the review surface says which — with the
/// bytes of both sides for the one that replaces.
///
/// The walk is the one a curator actually performs. A runbook is proposed
/// and published, so the channel holds it; the runbook is then edited and
/// proposed again, which is the *only* way to republish edited content
/// (approvals bind bytes, so an in-flight edit refuses); and a second,
/// untouched record is proposed alongside it. One `GET` then has to
/// distinguish all three.
#[tokio::test]
async fn a_proposal_renders_its_effect_on_the_channel_member_by_member() {
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

    let runbook = seed_record(&pool, tenant, team.id, author.id, RUNBOOK).await;
    let rota = seed_record(&pool, tenant, team.id, author.id, "the rota is weekly").await;

    // Round one puts the runbook on the channel.
    let (_, first) = open_proposal(
        &app,
        &dana,
        team.id,
        None,
        &[runbook],
        "publish the key-rotation runbook",
    )
    .await;
    // Before anything is published, every member is an addition.
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{first}")).await;
    let member = member_for(&detail, runbook);
    assert_eq!(
        member["effect"], "add",
        "an empty channel holds nothing to replace: {detail}"
    );
    assert!(
        member["baseline"].is_null(),
        "an addition has no old side: {member}"
    );
    assert_eq!(
        content_of(member["proposed"].as_str().expect("proposed bytes")),
        RUNBOOK,
        "the proposed side is the object under review"
    );
    approve_and_publish(&app, &[&cora], &first).await;

    // Round two: the runbook is edited, and proposed again beside an
    // untouched second record and the *unedited* published one.
    rewrite(&pool, runbook, team.id, author.id, REVISED).await;
    let (_, second) = open_proposal(
        &app,
        &dana,
        team.id,
        None,
        &[runbook, rota],
        "revise the rotation interval",
    )
    .await;

    let (status, detail) = get(&app, &cora, &format!("/v1/proposals/{second}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");

    // The edited record: an update, with the published version as the old
    // side and the proposed version as the new one.
    let updated = member_for(&detail, runbook);
    assert_eq!(updated["effect"], "update", "{detail}");
    let baseline = &updated["baseline"];
    assert!(!baseline.is_null(), "an update must carry what it replaces");
    assert_eq!(
        content_of(baseline["text"].as_str().expect("baseline bytes")),
        RUNBOOK,
        "the old side is what the channel names today"
    );
    assert_eq!(
        content_of(updated["proposed"].as_str().expect("proposed bytes")),
        REVISED,
        "the new side is what this proposal names"
    );
    assert_ne!(
        baseline["object_hash"], updated["object_hash"],
        "an update replaces one address with another: {updated}"
    );

    // The untouched second record: an addition, on the same channel.
    let added = member_for(&detail, rota);
    assert_eq!(added["effect"], "add", "{detail}");
    assert!(added["baseline"].is_null(), "{added}");

    // And re-proposing what is already published, unchanged, is a no-op
    // the reviewer can see before voting.
    let (_, third) = open_proposal(
        &app,
        &dana,
        team.id,
        None,
        &[rota],
        "the rota again, unchanged",
    )
    .await;
    approve_and_publish(&app, &[&cora], &second).await;
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{third}")).await;
    let repeated = member_for(&detail, rota);
    assert_eq!(
        repeated["effect"], "none",
        "the channel already names this record at this address: {detail}"
    );
    assert!(
        repeated["baseline"].is_null(),
        "a no-op replaces nothing: {repeated}"
    );
}

/// The new side is the **proposal's** object, not the record's row.
///
/// Once a record has been edited under its own review, the row and the
/// approved bytes are two different things, and a review surface that
/// showed the row would be showing content nobody proposed. `unchanged`
/// marks the drift; `proposed` keeps naming what the approvals bind
/// (ADR-0032 decision 6, ADR-0035 decision 6).
#[tokio::test]
async fn an_edited_record_still_renders_the_bytes_that_were_proposed() {
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

    let runbook = seed_record(&pool, tenant, team.id, author.id, RUNBOOK).await;
    let (_, id) = open_proposal(&app, &dana, team.id, None, &[runbook], "promote it").await;
    rewrite(&pool, runbook, team.id, author.id, REVISED).await;

    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{id}")).await;
    let member = member_for(&detail, runbook);
    assert_eq!(
        member["unchanged"], false,
        "the drift must be visible: {detail}"
    );
    assert_eq!(
        content_of(member["proposed"].as_str().expect("proposed bytes")),
        RUNBOOK,
        "the review is of the proposed bytes, not of the record as it now stands"
    );
    assert_eq!(
        member["content"].as_str().expect("current content"),
        REVISED,
        "`content` is the record as it now stands — that is what makes the drift legible"
    );
}

// ── The disclosure ───────────────────────────────────────────────────────────

/// A `compliance` reviewer holds no `MemoryRead` in any pack, and is shown
/// both sides of the change anyway (ADR-0035 decision 8).
///
/// Asserted rather than assumed, because it is a deliberate widening of
/// ADR-0034 decision 1's carve-out: the alternative is that the one role
/// the invariant floor requires on everything `restricted` must approve
/// replacements sight unseen.
#[tokio::test]
async fn compliance_sees_both_sides_of_the_change_it_must_approve() {
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
    let app = router(state(&database_url()));

    let runbook = seed_record(&pool, tenant, team.id, author.id, RUNBOOK).await;
    let (_, first) = open_proposal(&app, &dana, team.id, None, &[runbook], "publish it").await;
    approve_and_publish(&app, &[&cora], &first).await;
    rewrite(&pool, runbook, team.id, author.id, REVISED).await;
    let (_, second) = open_proposal(&app, &dana, team.id, None, &[runbook], "revise it").await;

    // The premise: compliance genuinely cannot read this scope's memory.
    let (status, denied) = post(
        &app,
        &cleo,
        "/v1/inject",
        json!({"task": "what is the rotation interval"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{denied}");
    assert!(
        !denied["block"]
            .as_str()
            .unwrap_or_default()
            .contains("rotate the signing key"),
        "compliance holds no MemoryRead, so inject composes nothing for it: {denied}"
    );

    // And yet the review shows the change, both sides.
    let (status, detail) = get(&app, &cleo, &format!("/v1/proposals/{second}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let member = member_for(&detail, runbook);
    assert_eq!(member["effect"], "update", "{detail}");
    assert_eq!(
        content_of(member["proposed"].as_str().expect("proposed bytes")),
        REVISED
    );
    assert_eq!(
        content_of(member["baseline"]["text"].as_str().expect("baseline bytes")),
        RUNBOOK,
        "a reviewer who cannot see one side cannot review a replacement"
    );
}

// ── Reading a listing ────────────────────────────────────────────────────────

/// A climb renders both scope paths, and a same-scope proposal renders one
/// (ADR-0035 decision 9). Two UUIDs are not a review surface: "this came
/// from acme/eng/platform and is asking to land at acme/eng" is the
/// sentence FLOW-5 exists to make true.
#[tokio::test]
async fn a_listing_names_the_scopes_a_proposal_moves_between() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, eng) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    // Tenant-wide, so the same principal can read both scopes' listings
    // and the tenant-wide one — the case the packs grant to review roles.
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, "cora@acme.test", None, Role::Curator)
        .await
        .expect("bind tenant-wide curator");
    tx.commit().await.expect("commit binding");
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let app = router(state(&database_url()));

    let runbook = seed_record(&pool, tenant, team.id, author.id, RUNBOOK).await;
    let (same_scope, _) = open_proposal(
        &app,
        &dana,
        team.id,
        None,
        &[runbook],
        "publish at the team",
    )
    .await;
    assert_eq!(
        same_scope["target_scope_path"], "acme/eng/platform",
        "{same_scope}"
    );
    assert_eq!(
        same_scope["source_scope_path"], "acme/eng/platform",
        "a same-scope proposal names one scope twice: {same_scope}"
    );

    let (climb, climb_id) = open_proposal(
        &app,
        &dana,
        eng.id,
        Some(team.id),
        &[runbook],
        "climb it to the department",
    )
    .await;
    assert_eq!(climb["target_scope_path"], "acme/eng", "{climb}");
    assert_eq!(
        climb["source_scope_path"], "acme/eng/platform",
        "a climb names where it came from: {climb}"
    );

    // The paths survive the round trip through both read surfaces, which
    // is where a reviewer meets them.
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{climb_id}")).await;
    assert_eq!(detail["target_scope_path"], "acme/eng", "{detail}");
    assert_eq!(detail["source_scope_path"], "acme/eng/platform", "{detail}");

    let (status, listing) = get(&app, &cora, "/v1/proposals").await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    let rows = listing["proposals"].as_array().expect("proposals");
    assert_eq!(rows.len(), 2, "both proposals list: {listing}");
    assert!(
        rows.iter().all(|row| row["target_scope_path"].is_string()),
        "every row names its target: {listing}"
    );
    assert!(
        rows.iter()
            .any(|row| row["source_scope_path"] == "acme/eng/platform"
                && row["target_scope_path"] == "acme/eng"),
        "the climb is readable from the listing alone: {listing}"
    );
}

/// A climb's baseline is the **target's** channel, not the source's.
///
/// The proposal moves the ancestor's published set, so that is what the
/// diff must be against — a team that has published a record still makes
/// an `add` at the department, because the department has not.
#[tokio::test]
async fn a_climbs_baseline_is_the_scope_it_would_land_on() {
    let Some((pool, tenant)) = admitted_tenant().await else {
        return;
    };
    let (team, eng) = seed_hierarchy(&pool, tenant).await;
    let author = seed_user(&pool, tenant, "dana@acme.test", team.id).await;
    seed_user(&pool, tenant, "cora@acme.test", team.id).await;
    seed_user(&pool, tenant, "sam@acme.test", team.id).await;
    bind(&pool, tenant, "dana@acme.test", team.id, Role::Contributor).await;
    // Tenant-wide, so one curator and one steward can act at both levels.
    // The pair is the pack's: `regulated-strict` asks for a curator at a
    // team and a curator *and* a steward at a department (ADR-0034).
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, "cora@acme.test", None, Role::Curator)
        .await
        .expect("bind tenant-wide curator");
    role_bindings::bind(&mut *tx, tenant, "sam@acme.test", None, Role::Steward)
        .await
        .expect("bind tenant-wide steward");
    tx.commit().await.expect("commit binding");
    let dana = issue("dana@acme.test", tenant);
    let cora = issue("cora@acme.test", tenant);
    let sam = issue("sam@acme.test", tenant);
    let app = router(state(&database_url()));

    let runbook = seed_record(&pool, tenant, team.id, author.id, RUNBOOK).await;
    let (_, at_team) = open_proposal(
        &app,
        &dana,
        team.id,
        None,
        &[runbook],
        "publish at the team",
    )
    .await;
    approve_and_publish(&app, &[&cora], &at_team).await;

    let (_, climb) = open_proposal(
        &app,
        &dana,
        eng.id,
        Some(team.id),
        &[runbook],
        "climb it to the department",
    )
    .await;
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{climb}")).await;
    let member = member_for(&detail, runbook);
    assert_eq!(
        member["effect"], "add",
        "the department has published nothing, so the climb adds: {detail}"
    );
    assert!(
        member["baseline"].is_null(),
        "the source's own channel is not the baseline: {member}"
    );

    // And once the department holds it, a second climb of the same bytes
    // is the no-op — read off the target's tree, one level up. Two
    // approvers here, and the curator runs the effect: a steward reads no
    // content in any pack, so it cannot publish what it approved.
    approve_and_publish(&app, &[&cora, &sam], &climb).await;
    let (_, again) = open_proposal(
        &app,
        &dana,
        eng.id,
        Some(team.id),
        &[runbook],
        "climb it again",
    )
    .await;
    let (_, detail) = get(&app, &cora, &format!("/v1/proposals/{again}")).await;
    assert_eq!(
        member_for(&detail, runbook)["effect"],
        "none",
        "the department already names it at this address: {detail}"
    );
}

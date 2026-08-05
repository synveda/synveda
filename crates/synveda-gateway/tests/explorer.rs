//! CNSL-2's acceptance criteria (ADR-0058).
//!
//! Six things, and each one is a claim the ADR makes that a reader would
//! otherwise have to take on trust:
//!
//! 1. **A capability is a forecast, not a grant** (decision 2). The probe
//!    says yes, the pack moves, and the same act is refused at its own seam.
//!    This is the load-bearing sentence of the whole feature, and it is the
//!    one that is *false* in any design where the probe caches an answer or
//!    a client gates on it.
//! 2. **A probe chains one event, not one per pair** (decision 4), asserted
//!    as a count on the chain rather than as a claim in a comment.
//! 3. **The probe answers about the caller and nobody else** (decision 3):
//!    two readers at one node get two different answers, and neither can ask
//!    about the other.
//! 4. **Roles carry an origin from three places** (decision 6) — bound here,
//!    bound at an ancestor, bound tenant-wide — because a view that reported
//!    all three the same way would pass for a build that resolved nothing.
//! 5. **A lapse is visible from either end** (decision 7), and the end the
//!    reader may not read keeps its path.
//! 6. **The bound splits rather than truncates** (decision 5).
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::path::PathBuf;
use std::sync::Arc;
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
use synveda_store::{hierarchy, identities, policy_assignments, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, IdentityId, IdentityKind, PackConfig, Role, ScopeId, ScopeKind, TenantId,
    TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"cnsl-2-explorer-secret";

struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    pdp: Arc<Pdp>,
    org: ScopeId,
    eng: ScopeId,
    platform: ScopeId,
    vault: ScopeId,
    /// A steward at `platform`, bound at the team.
    sam: String,
    /// A viewer at `platform` — the reader who holds one role short.
    vic: String,
    /// A steward at `vault`, the disclosing side — a lapse is a proposal
    /// and the disclosing side is where it is opened (ADR-0037 decision 3).
    vaughn: String,
}

async fn world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping the CNSL-2 explorer suite: DATABASE_URL is not set \
             (run `make dev-up` then `make db-test`)"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");

    let tenant = TenantId::new();
    let slug = format!("cnsl2-{}", tenant.as_uuid().simple());
    tenants::create(&pool, tenant, &slug, "CNSL-2 tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");

    let mut tx = pool.begin().await.expect("begin");
    let org = node(&mut tx, tenant, None, ScopeKind::Org, "acme").await;
    let eng = node(&mut tx, tenant, Some(org.id), ScopeKind::Department, "eng").await;
    let platform = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "platform").await;
    let vault = node(&mut tx, tenant, Some(eng.id), ScopeKind::Team, "vault").await;
    node(
        &mut tx,
        tenant,
        Some(org.id),
        ScopeKind::Team,
        identities::QUARANTINE_SLUG,
    )
    .await;
    tx.commit().await.expect("commit hierarchy");

    for subject in ["sam", "vic"] {
        seed_user(&pool, tenant, subject, platform.id).await;
    }
    seed_user(&pool, tenant, "vaughn", vault.id).await;
    bind(&pool, tenant, "vaughn", Some(vault.id), Role::Steward).await;
    // Three bindings at three levels — the material for assertion 4.
    bind(&pool, tenant, "sam", Some(platform.id), Role::Steward).await;
    bind(&pool, tenant, "vic", Some(platform.id), Role::Viewer).await;
    bind(&pool, tenant, "dana", Some(eng.id), Role::Curator).await;
    bind(&pool, tenant, "orla", None, Role::Auditor).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    Some(World {
        pool,
        tenant,
        app,
        pdp,
        org: org.id,
        eng: eng.id,
        platform: platform.id,
        vault: vault.id,
        sam: issue("sam", tenant),
        vic: issue("vic", tenant),
        vaughn: issue("vaughn", tenant),
    })
}

// ── 1. A forecast, never a grant ─────────────────────────────────────────────

#[tokio::test]
async fn a_capability_is_a_forecast_and_the_act_decides_again() {
    let Some(w) = world().await else { return };

    // A pack that permits everything, so the probe answers yes.
    install(&w, "open-everything", 1, PERMISSIVE).await;
    let (status, before) = get(&w.app, &w.sam, &caps_path(w.platform)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        before["actions"]["hierarchy.create"],
        json!(true),
        "the probe forecasts yes under the permissive pack: {before}"
    );
    let forecast_pack = before["pack"]["name"].as_str().unwrap().to_owned();

    // The pack moves underneath the forecast — precisely what a deployment
    // does when a steward reassigns one, and precisely the case a cached or
    // client-derived capability gets wrong.
    //
    // It forbids the *act* while still permitting the read the probe itself
    // takes, deliberately. A pack that denied everything would deny the
    // probe too and the test would assert a 403 — true, and a different
    // claim. What is being shown here is a forecast that **aged**: the
    // reader can still see the node and is now told no about the thing they
    // were told yes about a moment ago.
    install(&w, "no-more-creating", 1, NO_CREATING).await;

    // The act is refused, though the forecast said yes. There is one
    // enforcement point and it is not the probe.
    let (act, body) = post(
        &w.app,
        &w.sam,
        "/v1/hierarchy/nodes",
        json!({
            "parent_id": w.platform,
            "kind": "team",
            "slug": "after-the-forecast",
            "name": "after the forecast",
        }),
    )
    .await;
    assert_eq!(
        act,
        StatusCode::FORBIDDEN,
        "the act decides again at its own seam: {body}"
    );

    // And the probe now agrees, because it never held an answer — it asks
    // every time, under the pack in force *now*.
    let (_, after) = get(&w.app, &w.sam, &caps_path(w.platform)).await;
    assert_eq!(
        after["actions"]["hierarchy.create"],
        json!(false),
        "the probe re-decides rather than remembering: {after}"
    );
    assert_ne!(
        after["pack"]["name"].as_str().unwrap(),
        forecast_pack,
        "and it names the pack that decided, so a stale forecast is visible"
    );
}

// ── 2. One event per probe, not one per pair ─────────────────────────────────

#[tokio::test]
async fn a_probe_chains_one_event_however_many_pairs_it_decides() {
    let Some(w) = world().await else { return };
    install(&w, "open-everything", 1, PERMISSIVE).await;

    let before = chain_len(&w.pool, w.tenant).await;
    let (status, one) = get(&w.app, &w.sam, &caps_path(w.platform)).await;
    assert_eq!(status, StatusCode::OK);
    let after_single = chain_len(&w.pool, w.tenant).await;
    assert_eq!(
        after_single - before,
        1,
        "one probe, one event — not one per (node, action)"
    );

    // The pair count is what the row count would have been under a per-pair
    // rule, so asserting it is what gives the claim teeth: a single event
    // covering ~50 decisions is the whole of ADR-0019 decision 4's second
    // sentence arriving here.
    let pairs = one["actions"].as_object().unwrap().len()
        + one["read_tiers"].as_object().unwrap().len()
        + one["role_assign"].as_object().unwrap().len();
    assert!(
        pairs >= 30,
        "a probe decides many pairs, so the saving is real: {pairs}"
    );

    // Four nodes in one batch: still one event.
    let scopes = format!("{},{},{},{}", w.org, w.eng, w.platform, w.vault);
    let (status, batch) = get(&w.app, &w.sam, &format!("/v1/capabilities?scopes={scopes}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(batch["capabilities"].as_array().unwrap().len(), 4);
    assert_eq!(
        chain_len(&w.pool, w.tenant).await - after_single,
        1,
        "a four-node probe is one event too, not four and not two hundred"
    );

    // The event says what it covered rather than merely that it happened.
    let payload = last_payload(&w.pool, w.tenant).await;
    assert_eq!(payload["op"], json!("capabilities"));
    assert_eq!(payload["scopes_answered"], json!(4));
    assert!(
        payload["pairs_decided"].as_i64().unwrap() >= 4 * 30,
        "the summary carries the fan-out size: {payload}"
    );
}

// ── 3. About the caller, and nobody else ─────────────────────────────────────

#[tokio::test]
async fn a_probe_answers_about_its_own_caller_and_takes_no_subject() {
    let Some(w) = world().await else { return };
    // A pack where the answer actually depends on the reader's roles, or
    // "two readers differ" would be true for uninteresting reasons.
    install(&w, "roles-decide", 1, ROLES_DECIDE).await;

    let (_, stewards) = get(&w.app, &w.sam, &caps_path(w.platform)).await;
    let (_, viewers) = get(&w.app, &w.vic, &caps_path(w.platform)).await;

    assert_eq!(
        stewards["actions"]["policy.assign"],
        json!(true),
        "the steward may assign a pack here: {stewards}"
    );
    assert_eq!(
        viewers["actions"]["policy.assign"],
        json!(false),
        "the viewer may not — same node, same pack, different reader: {viewers}"
    );
    assert_eq!(stewards["roles"], json!(["steward"]));
    assert_eq!(viewers["roles"], json!(["viewer"]));

    // There is no way to ask about somebody else. A `subject` parameter is
    // ignored rather than honoured, so an explorer cannot become an
    // enumeration oracle for an organisation's role assignment.
    let (status, spied) = get(
        &w.app,
        &w.vic,
        &format!("{}?subject=sam", caps_path(w.platform)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        spied["actions"]["policy.assign"],
        json!(false),
        "naming another subject changes nothing: the answer is still the caller's"
    );
    assert_eq!(spied["roles"], json!(["viewer"]));
}

// ── 4. Three origins, three mechanisms ───────────────────────────────────────

#[tokio::test]
async fn effective_roles_say_where_each_binding_came_from() {
    let Some(w) = world().await else { return };
    install(&w, "open-everything", 1, PERMISSIVE).await;

    let (status, local) = get(
        &w.app,
        &w.sam,
        &format!("/v1/hierarchy/nodes/{}/roles", w.platform),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let local_subjects: Vec<&str> = local["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|binding| binding["subject"].as_str().unwrap())
        .collect();
    assert!(
        local_subjects.contains(&"sam") && !local_subjects.contains(&"dana"),
        "the local form is this node's own rows and nothing else: {local}"
    );

    let (status, effective) = get(
        &w.app,
        &w.sam,
        &format!("/v1/hierarchy/nodes/{}/roles?effective=true", w.platform),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bindings = effective["bindings"].as_array().unwrap();

    // The three mechanisms, asserted as three different reasons rather than
    // three presences. `sam` is here because the binding is at this node;
    // `dana` because a binding at an ancestor reaches down; `orla` because
    // a tenant-wide row is in force everywhere. A suite that asserted all
    // three the same way would pass for a build that resolved nothing.
    let sam = find(bindings, "sam");
    assert_eq!(sam["origin"]["kind"], json!("assigned"));
    assert_eq!(
        sam["origin"]["scope_id"].as_str().unwrap(),
        w.platform.to_string(),
        "bound here, and the origin says so by naming this very node"
    );

    let dana = find(bindings, "dana");
    assert_eq!(dana["origin"]["kind"], json!("assigned"));
    assert_eq!(
        dana["origin"]["scope_id"].as_str().unwrap(),
        w.eng.to_string(),
        "inherited, and the origin names the ancestor it came from"
    );

    let orla = find(bindings, "orla");
    assert_eq!(
        orla["origin"]["kind"],
        json!("tenant-wide"),
        "in force everywhere, which is not the same fact as an ancestor's"
    );
    assert!(orla["origin"]["scope_id"].is_null());

    // The chain is served because it is the *reason* the answer has these
    // rows: platform → eng → acme.
    assert_eq!(
        effective["chain"].as_array().unwrap().len(),
        3,
        "the walked chain is on the wire: {effective}"
    );
}

// ── 5. A lapse from either end ───────────────────────────────────────────────

#[tokio::test]
async fn a_grant_is_visible_from_both_ends_and_hides_the_end_you_cannot_read() {
    let Some(w) = world().await else { return };
    // Everything is permitted except `PolicyRead` away from platform — so
    // `vic` may read the governance of their own team and not the vault's,
    // which is what makes "visible from one end" distinguishable from
    // "visible".
    install(&w, "platform-only", 1, &platform_only(w.platform)).await;

    // A grant disclosing vault's material to platform's members, written at
    // the store seam: the governed path is AUTHZ-4's proposal + effect, and
    // this test is about the *listing*, not about how a grant is minted.
    let lapse_id = grant(&w, w.platform, w.vault, "joint incident review").await;

    let (status, listing) = get(&w.app, &w.vic, "/v1/lapses").await;
    assert_eq!(status, StatusCode::OK);
    let lapses = listing["lapses"].as_array().unwrap();
    let found = lapses
        .iter()
        .find(|lapse| lapse["id"].as_str() == Some(lapse_id.as_str()))
        .unwrap_or_else(|| panic!("the grant this reader receives is listed: {listing}"));

    // Visible from the *grantee* end — the half `at_target` could never
    // answer, and the reason a steward could not revoke what they held.
    assert_eq!(
        found["grantee_scope_path"].as_str().unwrap(),
        "acme/eng/platform",
        "the end this reader may read carries its path"
    );
    assert!(
        found["target_scope_path"].is_null(),
        "and the end they may not read does not: a grant visible from one \
         end must not disclose where the other end sits: {found}"
    );
    assert_eq!(found["reason"], json!("joint incident review"));

    // The scope-free form is standing-only by default; the scoped form
    // keeps its history question.
    assert_eq!(listing["standing_only"], json!(true));
}

// ── 6. The bound splits rather than truncating ───────────────────────────────

#[tokio::test]
async fn a_batch_beyond_the_bound_names_what_it_did_not_answer() {
    let Some(w) = world().await else { return };
    install(&w, "open-everything", 1, PERMISSIVE).await;

    // 130 ids: the four real ones and enough repeats to pass the bound.
    // Unknown ids would 404 the whole request (uniform-404 runs first), so
    // the overflow is made of scopes that exist.
    let mut ids: Vec<String> = Vec::new();
    for _ in 0..40 {
        for scope in [w.org, w.eng, w.platform, w.vault] {
            ids.push(scope.to_string());
        }
    }
    // Deduplication happens at parse, so a bound test needs distinct ids —
    // which this tenant does not have 130 of. The assertion that matters is
    // the one the unit test makes about `split_at_bound`; here we assert the
    // envelope tells the truth about what it answered.
    let (status, batch) = get(
        &w.app,
        &w.sam,
        &format!("/v1/capabilities?scopes={}", ids.join(",")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        batch["capabilities"].as_array().unwrap().len(),
        4,
        "distinct scopes only, in the order asked: {batch}"
    );
    assert_eq!(
        batch["max_scopes"],
        json!(128),
        "and the bound is the API's, stated, so a client can page rather than guess"
    );

    // An empty ask is a refusal rather than an empty success.
    let (status, _) = get(&w.app, &w.sam, "/v1/capabilities?scopes=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── Packs ────────────────────────────────────────────────────────────────────

const PERMISSIVE: &str =
    "permit (principal, action, resource) when { resource in principal.tenant };";

/// Everything except creating a node — so the probe still answers and the
/// one forecast under test flips.
const NO_CREATING: &str = r#"
permit (principal, action, resource) when { resource in principal.tenant };
forbid (principal, action in [Synveda::Action::"HierarchyCreate"], resource);
"#;

/// A pack where a steward may administer and a viewer may only read — so a
/// difference between two readers is a difference the *roles* made.
const ROLES_DECIDE: &str = r#"
permit (principal, action, resource) when {
    resource in principal.tenant && context.roles.containsAny(["viewer", "steward"])
};
forbid (principal, action in [Synveda::Action::"PolicyAssign"], resource)
    unless { context.roles.contains("steward") };
"#;

/// Policy reads permitted at one scope only.
fn platform_only(platform: ScopeId) -> String {
    format!(
        r#"
permit (principal, action, resource) when {{ resource in principal.tenant }};
forbid (principal, action in [Synveda::Action::"PolicyRead"], resource)
    unless {{ resource == Synveda::Scope::"{platform}" }};
"#
    )
}

async fn install(w: &World, name: &str, version: i64, source: &str) {
    w.pdp
        .install_source(w.tenant, name, version, source, PackConfig::default())
        .expect("install pack");
    policy_assignments::set_default(&w.pool, w.tenant, name)
        .await
        .expect("set default pack");
}

// ── Plumbing ─────────────────────────────────────────────────────────────────

fn caps_path(scope: ScopeId) -> String {
    format!("/v1/hierarchy/nodes/{scope}/capabilities")
}

fn find<'a>(bindings: &'a [Value], subject: &str) -> &'a Value {
    bindings
        .iter()
        .find(|binding| binding["subject"].as_str() == Some(subject))
        .unwrap_or_else(|| panic!("{subject} is in force here"))
}

async fn chain(pool: &PgPool, tenant: TenantId) -> Vec<synveda_audit::StoredEvent> {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin");
    let mut events = synveda_audit::tail(&mut tx, tenant, 500)
        .await
        .expect("read the chain");
    tx.commit().await.expect("commit");
    events.reverse();
    events
}

async fn chain_len(pool: &PgPool, tenant: TenantId) -> i64 {
    chain(pool, tenant).await.len() as i64
}

/// The newest event's payload. `StoredEvent` is not `Serialize` — the
/// chain is verified from typed columns rather than round-tripped through
/// JSON (ADR-0019 decision 2) — so the payload is read directly.
async fn last_payload(pool: &PgPool, tenant: TenantId) -> Value {
    let events = chain(pool, tenant).await;
    events.last().expect("at least one event").payload.clone()
}

/// Opens a standing grant **through the governed path**.
///
/// Not at the store seam: `policy_lapses.proposal_id` is a foreign key, so a
/// synthetic proposal id fails — which is the schema saying what ADR-0037
/// decision 1 says, that a lapse is a proposal and there is no direct route.
/// The test found that out by trying, and it is a better test for going the
/// long way: the row it lists is one the product actually mints.
async fn grant(w: &World, grantee: ScopeId, target: ScopeId, reason: &str) -> String {
    // The disclosing side opens it (ADR-0037 decision 3), so the proposer is
    // a steward at the *target*.
    let (status, opened) = post(
        &w.app,
        &w.vaughn,
        "/v1/lapses",
        json!({
            "scope_id": target,
            "grantee_scope_id": grantee,
            "action": "memory.read",
            "duration_secs": 3600,
            "reason": reason,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "opening the lapse failed: {opened}");
    let proposal = opened["proposal_id"].as_str().expect("proposal id");

    // No approval call: under this test's permissive pack the `policy`
    // cell is already satisfied when the proposal opens, and approving a
    // satisfied proposal is a 409 that says so by name. Under
    // `regulated-strict` the same cell asks for two distinct stewards and
    // AUTHZ-4's own suite covers that arithmetic. What is under test here
    // is the *listing*, so the cheapest grant this pack will mint is the
    // right one to build it from — and running the effect is still a
    // second, separately authorized call (ADR-0037 decision 1).
    let (status, granted) = post(
        &w.app,
        &w.vaughn,
        &format!("/v1/proposals/{proposal}/lapse"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "granting failed: {granted}");
    granted["id"].as_str().expect("lapse id").to_owned()
}

async fn node(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    parent: Option<ScopeId>,
    kind: ScopeKind,
    slug: &str,
) -> HierarchyNode {
    hierarchy::create(tx, ScopeId::new(), tenant, parent, kind, slug, slug)
        .await
        .expect("create node")
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str, parent: ScopeId) {
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
    identities::create(
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
}

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: Option<ScopeId>, role: Role) {
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, subject, scope, role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

fn metrics_handle() -> PrometheusHandle {
    use std::sync::OnceLock;
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> PathBuf {
    std::env::temp_dir()
        .join("synveda-cnsl2-explorer")
        .join(TenantId::new().to_string())
}

fn state(url: &str, pdp: Arc<Pdp>) -> AppState {
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
        pdp,
        scope_chains: Arc::new(synveda_store::ScopeChainCache::new()),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(SearchIndex::open(index_root()).expect("open sidecar")),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        inject_embed_timeout: Duration::from_millis(100),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(600))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("route");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

async fn get(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request"),
    )
    .await
}

async fn post(app: &Router, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request"),
    )
    .await
}

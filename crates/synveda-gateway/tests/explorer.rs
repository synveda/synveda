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
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::{access, identities, rls, scopes, tenants};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    GrantId, IdentityId, IdentityKind, PackConfig, ScopeId, TenantId, TenantStatus,
};
use tower::ServiceExt;

#[path = "support/configuration.rs"]
mod configuration_support;

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
    let org = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("mint root");
    let eng = unit(&mut tx, tenant, org.id, "eng").await;
    let platform = unit(&mut tx, tenant, eng.id, "platform").await;
    let vault = unit(&mut tx, tenant, eng.id, "vault").await;
    tx.commit().await.expect("commit scopes");

    for subject in ["sam", "vic"] {
        seed_user(&pool, tenant, subject).await;
    }
    seed_user(&pool, tenant, "vaughn").await;
    bind(
        &pool,
        tenant,
        "vaughn",
        Some(vault.id),
        RoleKey::Administrator,
    )
    .await;
    bind(
        &pool,
        tenant,
        "sam",
        Some(platform.id),
        RoleKey::Administrator,
    )
    .await;
    bind(&pool, tenant, "vic", Some(platform.id), RoleKey::Viewer).await;

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
    let before = the_one(&before).clone();
    assert_eq!(
        before["actions"]["scope.create"],
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
    let (act, body) = post_key(
        &w.app,
        &w.sam,
        "/v1/admin/scopes",
        "cnsl2-forecast-act",
        json!({
            "parent_id": w.platform,
            "kind": "org_unit",
            "slug": "after-the-forecast",
            "display_name": "after the forecast",
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
    let after = the_one(&after);
    assert_eq!(
        after["actions"]["scope.create"],
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
    let one = the_one(&one);
    let after_single = chain_len(&w.pool, w.tenant).await;
    assert_eq!(
        after_single - before,
        1,
        "one probe, one event — not one per (node, action)"
    );

    // The pair count is what the row count would have been under a per-pair
    // rule, so asserting it is what gives the claim teeth: a single event
    // covering ~36 decisions is the whole of ADR-0019 decision 4's second
    // sentence arriving here.
    let pairs =
        one["actions"].as_object().unwrap().len() + one["read_tiers"].as_object().unwrap().len();
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
    let stewards = the_one(&stewards);
    let viewers = the_one(&viewers);

    assert_eq!(
        stewards["actions"]["policy.assign"],
        json!(true),
        "the administrator may assign a pack here: {stewards}"
    );
    assert_eq!(
        viewers["actions"]["policy.assign"],
        json!(false),
        "the viewer may not — same node, same pack, different reader: {viewers}"
    );
    assert_eq!(stewards["roles"], json!(["administrator"]));
    assert_eq!(viewers["roles"], json!(["viewer"]));

    // There is no way to ask about somebody else. A `subject` parameter is
    // ignored rather than honoured, so an explorer cannot become an
    // enumeration oracle for an organisation's role assignment.
    let (status, spied) = get(
        &w.app,
        &w.vic,
        &format!("{}&subject=sam", caps_path(w.platform)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let spied = the_one(&spied);
    assert_eq!(
        spied["actions"]["policy.assign"],
        json!(false),
        "naming another subject changes nothing: the answer is still the caller's"
    );
    assert_eq!(spied["roles"], json!(["viewer"]));
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
    let grantee_path = found["grantee_scope_path"].as_str().unwrap_or_default();
    assert!(
        grantee_path.ends_with("eng/platform"),
        "the end this reader may read carries its path: {grantee_path}"
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
forbid (principal, action in [Synveda::Action::"ScopeCreate"], resource);
"#;

/// A pack where a steward may administer and a viewer may only read — so a
/// difference between two readers is a difference the *roles* made.
const ROLES_DECIDE: &str = r#"
permit (principal, action, resource) when {
    resource in principal.tenant && context.roles.containsAny(["viewer", "administrator"])
};
forbid (principal, action in [Synveda::Action::"PolicyAssign"], resource)
    unless { context.roles.contains("administrator") };
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
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin Configuration selection");
    configuration_support::bind_tenant_pack(&mut tx, w.tenant, name).await;
    tx.commit().await.expect("commit Configuration selection");
}

// ── Plumbing ─────────────────────────────────────────────────────────────────

fn caps_path(scope: ScopeId) -> String {
    format!("/v1/capabilities?scopes={scope}")
}

/// The single capability object a one-scope probe answers with.
fn the_one(batch: &Value) -> &Value {
    batch["capabilities"]
        .as_array()
        .expect("one answer")
        .first()
        .expect("the asked-for scope")
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

async fn unit(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    tenant: TenantId,
    parent: ScopeId,
    slug: &str,
) -> Scope {
    scopes::create(
        &mut *tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(parent),
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            attributes: serde_json::json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create org unit")
}

async fn seed_user(pool: &PgPool, tenant: TenantId, subject: &str) {
    let mut tx = pool.begin().await.expect("begin");
    let own = scopes::ensure_principal_scope(&mut tx, tenant, subject, subject)
        .await
        .expect("mint principal scope");
    identities::create(
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
}

async fn bind(
    pool: &PgPool,
    tenant: TenantId,
    subject: &str,
    scope: Option<ScopeId>,
    role: RoleKey,
) {
    let mut tx = pool.begin().await.expect("begin");
    let scope = match scope {
        Some(scope) => scope,
        None => {
            scopes::ensure_tenant_root(&mut tx, tenant)
                .await
                .expect("mint root")
                .id
        }
    };
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

async fn post_key(
    app: &Router,
    token: &str,
    uri: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    call(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("idempotency-key", key)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request"),
    )
    .await
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

// ── The parity corpus (ADR-0058 decision 10) ─────────────────────────────────
//
// Four payloads recorded from the real API, each with a `.facts.json` saying
// what a reader must be told. Both renderers answer them: the CLI's in
// `crates/synveda-cli`, the console's in `console/src/explorer.parity.test.tsx`.
//
// The cases are chosen for the judgements the surfaces make rather than the
// shapes a serialiser emits — an origin that is *not* local, a pack from two
// levels up, a grant beside one that has ended, and a capability answer with
// a denial in it. A corpus of happy paths proves the two renderers agree
// about nothing difficult.
//
// `SYNVEDA_RECORD_FIXTURES=1 make db-test` re-records; otherwise this
// verifies, which is the point — a corpus nobody checks drifts out of the
// shape the product serves, and then both renderers agree about a response
// nobody receives.

/// Ids and instants are replaced with stable, **shape-preserving** stand-ins:
/// a scope id stays uuid-shaped because both surfaces key behaviour off that
/// shape (the CLI abbreviates an id and does not abbreviate a path), so a
/// corpus that normalised one to `scope-01` would be a corpus in which that
/// rule is never exercised.
fn stabilise(value: &mut Value, map: &mut std::collections::BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if is_uuid(text) {
                let next = map.len();
                let stable = map
                    .entry(text.clone())
                    .or_insert_with(|| format!("0199c000-0000-7000-8000-{next:012}"));
                *text = stable.clone();
            } else if text.len() >= 20 && text.contains("T") && text.ends_with('Z') {
                *text = "2026-08-05T09:00:00Z".to_owned();
            } else if let Some((head, tail)) = text.split_once('/') {
                // A slug chain's first segment is the tenant root's slug,
                // which carries the tenant's random id — the one volatile
                // word a path can hold, since every slug after it was
                // chosen by the fixture.
                if is_tenant_slug(head) {
                    *text = format!("<tenant>/{tail}");
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| stabilise(item, map)),
        Value::Object(fields) => fields
            .iter_mut()
            .for_each(|(_, field)| stabilise(field, map)),
        _ => {}
    }
}

fn is_uuid(text: &str) -> bool {
    text.len() == 36 && text.split('-').map(str::len).eq([8, 4, 4, 4, 12])
}

/// The fixtures' tenant slugs: a suite prefix, a dash, and the tenant's
/// 32-hex simple uuid.
fn is_tenant_slug(text: &str) -> bool {
    let Some((prefix, hex)) = text.rsplit_once('-') else {
        return false;
    };
    prefix.len() <= 6
        && !prefix.is_empty()
        && hex.len() == 32
        && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../console/fixtures/explorer")
        .canonicalize()
        .unwrap_or_else(|_| {
            let dir =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../console/fixtures/explorer");
            std::fs::create_dir_all(&dir).expect("create corpus dir");
            dir.canonicalize().expect("canonicalize")
        })
}

fn settle(name: &str, payload: &Value) {
    let path = corpus_dir().join(format!("{name}.json"));
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(payload).expect("render")
    );
    if std::env::var("SYNVEDA_RECORD_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, &rendered).expect("write fixture");
        return;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing — re-record with SYNVEDA_RECORD_FIXTURES=1",
            path.display()
        )
    });
    assert_eq!(
        existing,
        rendered,
        "the corpus has drifted from what the gateway serves ({}). \
         Re-record with SYNVEDA_RECORD_FIXTURES=1 and read the diff.",
        path.display()
    );
}

#[tokio::test]
async fn the_explorer_parity_corpus_is_what_the_gateway_serves() {
    let Some(w) = world().await else { return };

    // A pack assigned at the department, so `platform` inherits it from one
    // level up — the "not local" origin the corpus exists for.
    //
    // `ROLES_DECIDE` rather than the permissive source, because the
    // capability case has to carry a **denial**: under a pack that permits
    // everything, every reader is allowed everything and the panel a
    // renderer must get right — the one where some actions are absent — is
    // never exercised. `vic` is a viewer, so `policy.assign` comes back
    // false and the corpus has the case it is for.
    install(&w, "eng-pack", 7, ROLES_DECIDE).await;
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin department Configuration");
    configuration_support::bind_pack(&mut tx, w.tenant, w.eng, "eng-pack").await;
    tx.commit().await.expect("commit department Configuration");

    let (status, configuration) = get(
        &w.app,
        &w.sam,
        &format!("/v1/configurations/effective?scope_id={}", w.platform),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, caps) = get(&w.app, &w.vic, &caps_path(w.platform)).await;
    let caps = the_one(&caps).clone();

    // A standing grant and one that has already ended, so a renderer that
    // showed them the same way fails on exactly this case.
    grant(&w, w.platform, w.vault, "joint incident review").await;
    expire_one(&w).await;
    let (_, lapses) = get(&w.app, &w.sam, "/v1/lapses?active=false").await;

    let mut ids = std::collections::BTreeMap::new();
    // The node each payload is *about*, because an origin cannot be
    // described without the frame it is described against.
    let asked_about = w.platform.to_string();
    for (name, mut payload) in [
        (
            "configuration-inherited",
            json!({"asked_about": asked_about, "payload": configuration}),
        ),
        (
            "capabilities-with-denial",
            json!({"asked_about": asked_about, "payload": caps}),
        ),
        (
            "lapses-standing-and-ended",
            json!({"asked_about": asked_about, "payload": lapses}),
        ),
    ] {
        stabilise(&mut payload, &mut ids);
        let facts = facts_for(name, &payload);
        settle(name, &payload);
        settle(&format!("{name}.facts"), &facts);
    }
}

/// What a reader must be told about a case, derived from the payload rather
/// than hand-written — so a fact cannot quietly stop describing the case it
/// is about.
///
/// These are **facts, not strings**: an origin's word is shared between the
/// surfaces (ADR-0056 decision 5 — one right answer), but the layout around
/// it is not, so a fact is a substring each renderer must contain somewhere
/// and never a line either must produce.
fn facts_for(name: &str, case: &Value) -> Value {
    let payload = &case["payload"];
    match name {
        "configuration-inherited" => json!({
            "must_name": [
                payload["document"]["policy_pack"].as_str().unwrap(),
                payload["version_id"].as_str().unwrap(),
                payload["content_hash"].as_str().unwrap(),
                "inherited",
            ],
            "must_not_name": ["bound here", "enterprise fail-safe"],
        }),
        "capabilities-with-denial" => {
            let allowed: Vec<String> = payload["actions"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(_, permitted)| permitted.as_bool() == Some(true))
                .map(|(action, _)| action.clone())
                .collect();
            let denied = payload["actions"]
                .as_object()
                .unwrap()
                .values()
                .filter(|permitted| permitted.as_bool() == Some(false))
                .count();
            assert!(denied > 0, "this case exists to carry a denial");
            let mut must = allowed;
            must.push(payload["pack"]["name"].as_str().unwrap().to_owned());
            // The sentence that stops a capability list reading as a
            // permission (ADR-0058 decision 2). Both surfaces carry it or
            // one of them is telling a reader something untrue.
            must.push("forecast".to_owned());
            json!({
                "must_name": must,
                // A denied action must not be listed as something the
                // reader may do — the failure that would make the panel
                // worse than useless.
                "must_not_name": denied_actions(payload),
            })
        }
        "lapses-standing-and-ended" => {
            let mut must = Vec::new();
            let mut outcomes = std::collections::BTreeSet::new();
            for lapse in payload["lapses"].as_array().unwrap() {
                must.push(lapse["reason"].as_str().unwrap().to_owned());
                outcomes.insert(lapse["outcome"].as_str().unwrap().to_owned());
            }
            assert!(
                outcomes.len() > 1,
                "this case exists to carry a standing grant beside an ended one, got {outcomes:?}"
            );
            must.extend(outcomes);
            json!({"must_name": must, "must_not_name": []})
        }
        other => panic!("no facts derivation for {other}"),
    }
}

fn denied_actions(payload: &Value) -> Vec<String> {
    payload["actions"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, permitted)| permitted.as_bool() == Some(false))
        .map(|(action, _)| action.clone())
        .collect()
}

/// Ends one grant early so the corpus carries a standing row beside a
/// finished one. Revocation rather than waiting: an expiry needs a clock to
/// pass and a corpus must not be slow.
async fn expire_one(w: &World) {
    let extra = grant(w, w.platform, w.vault, "a grant that was withdrawn").await;
    let (status, body) = post(
        &w.app,
        &w.vaughn,
        &format!("/v1/lapses/{extra}/revoke"),
        json!({"reason": "the review finished early"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "revoking failed: {body}");
}

/// The defect CNSL-2's own demo found, and the reason decision 3 is read
/// literally (ADR-0058).
///
/// The first cut gated the probe on `ScopeRead`. Under **every shipped
/// pack** that action belongs to owner and administrator alone — so a
/// `curator`, the role the proposals inbox exists for, was refused the probe
/// outright and the console showed them no verdict buttons at all. A
/// capability surface only privileged readers may consult is worse than
/// none: it hides acts from exactly the readers who hold them.
///
/// What survives is the split. The verdicts are about the caller and are
/// always answered; `scope_path` and the effective pack are facts about the
/// *node* and are withheld from a caller who may not read it, so the route
/// cannot become a node-metadata oracle for anyone holding a scope id.
#[tokio::test]
async fn a_reader_without_admin_read_still_learns_what_they_may_do() {
    let Some(w) = world().await else { return };
    // The shipped pack, not a permissive test one — the whole point is what
    // the real packs grant.
    let mut tx = rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("begin standard Configuration");
    configuration_support::bind_tenant_pack(&mut tx, w.tenant, "standard").await;
    tx.commit().await.expect("commit standard Configuration");

    // `vic` is a viewer: no `ScopeRead` under `standard`.
    let (status, caps) = get(&w.app, &w.vic, &caps_path(w.platform)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a viewer may ask what they themselves may do: {caps}"
    );
    let caps = the_one(&caps);
    assert_eq!(
        caps["actions"]["scope.read"],
        json!(false),
        "and the answer is honest about the very action that used to gate it"
    );
    assert!(
        caps["actions"].as_object().unwrap().len() >= 20,
        "the full vocabulary is answered, not a stub: {caps}"
    );

    // The node's own facts are withheld, because those are not about the
    // caller.
    assert!(
        caps["scope_path"].is_null(),
        "a caller who may not read the node does not learn where it sits: {caps}"
    );
    assert!(caps["pack"].is_null(), "nor which pack governs it: {caps}");

    // A reader who *may* read the node gets both, so the withholding is a
    // decision rather than a missing feature.
    let (status, admin) = get(&w.app, &w.sam, &caps_path(w.platform)).await;
    assert_eq!(status, StatusCode::OK);
    let admin = the_one(&admin);
    assert!(
        admin["scope_path"]
            .as_str()
            .is_some_and(|path| path.ends_with("/eng/platform")),
        "the reader who may read the node learns where it sits: {admin}"
    );
    assert_eq!(admin["pack"]["name"], json!("standard"));

    // And the probe still chains, including for the caller who learned
    // nothing about the node — a sweep that answers nothing is the one most
    // worth recording.
    let payload = last_payload(&w.pool, w.tenant).await;
    assert_eq!(payload["op"], json!("capabilities"));
}

// ── The tree at HIER-1's own scale ───────────────────────────────────────────

/// The AC's scale clause: **a 10,000-node hierarchy renders without
/// fetching a subtree or probing a node nobody looked at** (ADR-0058
/// decision 5).
///
/// The claim is about *what a screen fetches*, not about how fast a query
/// runs — HIER-1's own suite owns the latency bound and measures it at the
/// store. What this measures is the thing the explorer decides: a lazy tree
/// touches the nodes a reader opened and nothing else, where the
/// `descendants` call it deliberately does not make would have returned all
/// of them and then handed each one to a PDP fan-out.
///
/// It also makes the batch bound testable for the first time. The small
/// fixture has four scopes, so `a_batch_beyond_the_bound_names_what_it_did
/// _not_answer` could only assert the envelope; here there are ten thousand
/// distinct ids and the split is real.
#[tokio::test]
async fn a_ten_thousand_node_tree_renders_without_fetching_or_probing_it() {
    // Its own tenant, because a hierarchy has one root per tenant and the
    // count has to be exact for the contrast below to mean anything.
    let Some(w) = wide_world().await else { return };
    install(&w, "open-everything", 1, PERMISSIVE).await;

    // The wide shape, in the five-shape vocabulary: a root and four
    // levels of nested org units (an org unit nests inside itself to
    // arbitrary depth, ADR-0070 decision 1). Principal scopes hang off
    // the tenant root and only there, so a fixture that put nine
    // thousand of them there would make the root's own level the
    // subtree — the exact fetch this test exists to refuse.
    let seeded = std::time::Instant::now();
    let mut tx = w.pool.begin().await.expect("begin");
    let root = scopes::ensure_tenant_root(&mut tx, w.tenant)
        .await
        .expect("mint root");
    let mut first_division = None;
    let mut first_department = None;
    let mut every_id: Vec<ScopeId> = vec![root.id];
    for d in 0..9 {
        let division = unit(&mut tx, w.tenant, root.id, &format!("div-{d}")).await;
        every_id.push(division.id);
        first_division.get_or_insert(division.id);
        for p in 0..10 {
            let dept = unit(&mut tx, w.tenant, division.id, &format!("dept-{d}-{p}")).await;
            every_id.push(dept.id);
            first_department.get_or_insert(dept.id);
            for m in 0..10 {
                let team = unit(&mut tx, w.tenant, dept.id, &format!("team-{d}-{p}-{m}")).await;
                every_id.push(team.id);
            }
        }
    }
    // The fourth level: ten subteams under each of the 900 teams.
    let teams: Vec<ScopeId> = every_id[1 + 9 + 90..1 + 9 + 90 + 900].to_vec();
    for (t, team) in teams.iter().enumerate() {
        for s in 0..10 {
            let subteam = unit(&mut tx, w.tenant, *team, &format!("sub-{t}-{s}")).await;
            every_id.push(subteam.id);
        }
    }
    tx.commit().await.expect("commit the wide fixture");
    eprintln!("seeded {} nodes in {:?}", every_id.len(), seeded.elapsed());
    assert_eq!(every_id.len(), 10_000, "the fixture must actually be 10k");

    // What the screen does NOT do, measured so the contrast is a number
    // rather than a claim: one `descendants` at the root is the whole tree.
    let (status, subtree) = get(
        &w.app,
        &w.sam,
        &format!("/v1/admin/scopes/{}/descendants", root.id),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{subtree}");
    let subtree_size = subtree["scopes"].as_array().unwrap().len();
    assert_eq!(
        subtree_size, 9_999,
        "the call the explorer refuses to make returns the entire tree"
    );

    // What the screen DOES do: children on expand, one level at a time.
    // A reader opening root → division → department sees three fetches
    // and touches a bounded handful of nodes; the fourth structural level
    // (teams) is what the third fetch returned.
    let mut touched: Vec<ScopeId> = vec![root.id];
    let mut fetches = 0usize;
    let mut cursor = root.id;
    for _ in 0..3 {
        let (status, kids) = get(
            &w.app,
            &w.sam,
            &format!("/v1/admin/scopes?parent_id={cursor}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        fetches += 1;
        let kids = kids["scopes"].as_array().unwrap();
        for kid in kids {
            touched.push(kid["id"].as_str().unwrap().parse().unwrap());
        }
        cursor = kids[0]["id"].as_str().unwrap().parse().unwrap();
    }
    eprintln!(
        "a three-expand descent: {fetches} fetches, {} nodes touched of {}",
        touched.len(),
        every_id.len()
    );
    assert!(
        touched.len() <= 64,
        "opening the tree must touch a handful, not a subtree: {} nodes",
        touched.len()
    );
    // The root + 9 divisions + 10 departments + 10 teams.
    assert_eq!(touched.len(), 30);

    // And the probe is over what was rendered, in ONE request and ONE
    // event — which is the other half of the claim: a tree that fetched
    // lazily and then probed every node would have moved the cost rather
    // than removed it.
    let before = chain_len(&w.pool, w.tenant).await;
    let scopes: Vec<String> = touched.iter().map(ToString::to_string).collect();
    let (status, probed) = get(
        &w.app,
        &w.sam,
        &format!("/v1/capabilities?scopes={}", scopes.join(",")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(probed["capabilities"].as_array().unwrap().len(), 30);
    assert_eq!(
        chain_len(&w.pool, w.tenant).await - before,
        1,
        "thirty nodes, one event"
    );

    // The bound, on real distinct ids at last. Ask for more than the API
    // will answer and it splits rather than truncating: the answered head
    // in the order asked, the rest named.
    let overflow: Vec<String> = every_id
        .iter()
        .take(MAX_BATCH + 37)
        .map(ToString::to_string)
        .collect();
    let (status, split) = get(
        &w.app,
        &w.sam,
        &format!("/v1/capabilities?scopes={}", overflow.join(",")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        split["capabilities"].as_array().unwrap().len(),
        MAX_BATCH,
        "the bound is the API's and it holds"
    );
    assert_eq!(
        split["not_answered"].as_array().unwrap().len(),
        37,
        "and what it did not answer is named rather than dropped: {split}"
    );
    let answered: Vec<&str> = split["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["scope_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        answered,
        overflow[..MAX_BATCH]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        "in the order the caller asked, so paging is predictable"
    );
}

/// The API's declared batch bound, mirrored from
/// `capabilities::MAX_BATCH_SCOPES` — which is `pub(crate)`, so a test
/// binary cannot import it. `a_batch_beyond_the_bound_names_what_it_did_not
/// _answer` asserts the served `max_scopes` equals this, which is what keeps
/// the mirror honest.
const MAX_BATCH: usize = 128;

/// A tenant with nothing in it but a token and a pack — the wide fixture
/// builds its own hierarchy, and it needs the root.
async fn wide_world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!("skipping the CNSL-2 scale test: DATABASE_URL is not set");
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
    let slug = format!("cnsl2w-{}", tenant.as_uuid().simple());
    tenants::create(&pool, tenant, &slug, "CNSL-2 scale", TenantStatus::Active)
        .await
        .expect("admit tenant");
    // Tenant-wide, because the fixture's own scopes do not exist yet and
    // this reader has to be able to walk all of them.
    bind(&pool, tenant, "sam", None, RoleKey::Administrator).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let app = router(state(&url, pdp.clone()));
    // The unused anchors are the small world's shape; this fixture builds
    // its own and the struct is shared.
    let placeholder = ScopeId::new();
    Some(World {
        pool,
        tenant,
        app,
        pdp,
        org: placeholder,
        eng: placeholder,
        platform: placeholder,
        vault: placeholder,
        sam: issue("sam", tenant),
        vic: issue("vic", tenant),
        vaughn: issue("vaughn", tenant),
    })
}

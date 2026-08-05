//! AUD-2 acceptance criteria (ADR-0045): "both questions answerable via
//! one API call each (uses bitemporal + refs)."
//!
//! Both questions over the real product surfaces, and — this is the point
//! — **never a seeded audit row**. Disclosures exist here because people
//! were actually served material: alice and bob each call
//! `POST /v1/inject` and the chain records what they got, exactly as it
//! would in a live session. The audit surface then answers from those
//! events and nothing else.
//!
//! The suite runs under the **real embedded packs** — `regulated-strict`
//! is the zero-config default and nothing here installs a permissive one.
//! That is load-bearing: a blanket pack would grant `AuditRead` to
//! everyone and make every refusal in this file vacuous.
//!
//! Tests need a live Postgres: they read `DATABASE_URL` and skip with a
//! message when it is unset (CI has no database), the house convention.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::{Hs256Verifier, personal_slug};
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder, Embedder as _};
use synveda_policy::Pdp;
use synveda_retrieval::index::SearchIndex;
use synveda_store::records::{self, RecordEmbedding, RecordState};
use synveda_store::{hierarchy, identities, role_bindings, tenants};
use synveda_types::{
    HierarchyNode, Identity, IdentityId, IdentityKind, RecordClass, RecordId, RecordKind, Role,
    ScopeId, ScopeKind, Sensitivity, TenantId, TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"aud-2-test-secret";

/// The material the disclosure question is asked about. Distinctive enough
/// that a leak sweep over a response body cannot miss it.
const RUNBOOK: &str = "Platform incident runbook: freeze the reconciliation job, compare the \
     ledger tail against the acquirer statement, then page the on-call and the controller \
     together.";

/// alice's own note — a second record, so "what did alice know" has more
/// than one row and the fold has something to fold.
const NOTE: &str = "Alice prefers rebase-and-merge for her feature branches.";

/// Material only the payments team holds. Never disclosed to anyone in
/// this suite, which is what makes it the honest half of the
/// existence-oracle test: a record that exists and was never served must
/// answer exactly like one that does not exist at all.
const PAYMENTS_ONLY: &str = "Payments settlement cutover checklist, revision four.";

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn index_root() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("synveda-aud2-tests")
        .join(TenantId::new().to_string())
}

fn state_with(url: &str, search_index: Arc<SearchIndex>, pdp: Arc<Pdp>) -> AppState {
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
        search_index,
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
                "skipping AUD-2 test: DATABASE_URL is not set \
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
    let slug = format!("aud2-{}", id.as_uuid().simple());
    tenants::create(&pool, id, &slug, "AUD-2 test tenant", TenantStatus::Active)
        .await
        .expect("admit tenant");
    Some((pool, id))
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("checked by admitted_tenant")
}

async fn seed_hierarchy(
    pool: &PgPool,
    tenant: TenantId,
) -> (HierarchyNode, HierarchyNode, HierarchyNode) {
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
    let platform = hierarchy::create(
        &mut tx,
        ScopeId::new(),
        tenant,
        Some(org.id),
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
        Some(org.id),
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
    .expect("create quarantine");
    tx.commit().await.expect("commit hierarchy");
    (org, platform, payments)
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

async fn seed_record(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    owner: IdentityId,
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
            class: RecordClass::Procedure,
            content: content.to_owned(),
            sensitivity: Sensitivity::Internal,
            provenance: json!({"source": "aud-2 test fixture"}),
            valid_from: Utc::now(),
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

async fn bind(pool: &PgPool, tenant: TenantId, subject: &str, scope: Option<ScopeId>, role: Role) {
    let mut tx = pool.begin().await.expect("begin");
    role_bindings::bind(&mut *tx, tenant, subject, scope, role)
        .await
        .expect("bind role");
    tx.commit().await.expect("commit binding");
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("send request");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, value)
}

async fn get(app: &Router, path: &str, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("build get request");
    call(app, request).await
}

/// Bind a role through the product surface, so the act lands on the chain
/// as `role.bound` — which is what makes the authority half of a
/// disclosure answer non-empty. A binding written straight to the store
/// (the AUTHZ-3 bootstrap, and what most fixtures do) chains nothing, and
/// an audit answer can only ever report what was recorded.
async fn bind_over_http(
    app: &Router,
    token: &str,
    scope: Option<ScopeId>,
    subject: &str,
    role: Role,
) -> StatusCode {
    let uri = match scope {
        None => "/v1/roles/bindings".to_owned(),
        Some(id) => format!("/v1/hierarchy/nodes/{id}/roles"),
    };
    let request = Request::builder()
        .method("PUT")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"subject": subject, "role": role}).to_string(),
        ))
        .expect("build bind request");
    call(app, request).await.0
}

async fn inject(app: &Router, token: &str, session: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/inject")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"session_id": session}).to_string()))
        .expect("build inject request");
    call(app, request).await
}

/// RFC 3339 with a `Z` offset — no `+` to survive a query string.
fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// The world every test starts in: a platform runbook and alice's own
/// note, both served to alice; the runbook also served to bob; a payments
/// record nobody is ever served; and two auditors — dana tenant-wide,
/// erin at platform only.
struct World {
    pool: PgPool,
    tenant: TenantId,
    app: Router,
    alice: String,
    dana: String,
    erin: String,
    runbook: RecordId,
    note: RecordId,
    payments_only: RecordId,
    /// An instant before any inject ran — the "what did alice know
    /// *then*" end of the bitemporal pair.
    before: DateTime<Utc>,
}

async fn world() -> Option<World> {
    let (pool, tenant) = admitted_tenant().await?;
    let (_org, platform, payments) = seed_hierarchy(&pool, tenant).await;
    let alice = seed_user(&pool, tenant, "alice", platform.id).await;
    let bob = seed_user(&pool, tenant, "bob", platform.id).await;
    let carol = seed_user(&pool, tenant, "carol", payments.id).await;
    seed_user(&pool, tenant, "dana", platform.id).await;
    seed_user(&pool, tenant, "erin", platform.id).await;
    seed_user(&pool, tenant, "olive", platform.id).await;
    // The bootstrap is a direct write, exactly as AUTHZ-3 describes it:
    // `synveda role bind` is how a tenant gets its first org-admin, and it
    // chains as break-glass rather than as a governed act.
    bind(&pool, tenant, "olive", None, Role::OrgAdmin).await;

    let runbook = seed_record(&pool, tenant, platform.id, alice.id, RUNBOOK).await;
    let note = seed_record(&pool, tenant, alice.scope_id, alice.id, NOTE).await;
    let payments_only = seed_record(&pool, tenant, payments.id, carol.id, PAYMENTS_ONLY).await;

    let pdp = Arc::new(Pdp::new().expect("build the embedded PDP"));
    let index = Arc::new(SearchIndex::open(index_root()).expect("open sidecar"));
    let app = router(state_with(&database_url(), index, pdp));

    // The two auditors, bound through the product surface by the
    // org-admin: dana tenant-wide, erin at platform only. Going through
    // the route is what puts `role.bound` on the chain, which is where
    // historical authority lives — `role_bindings` is a current-state
    // table and an unbound role leaves no row behind.
    let olive = issue("olive", tenant);
    assert_eq!(
        bind_over_http(&app, &olive, None, "dana", Role::Auditor).await,
        StatusCode::OK,
        "dana is bound auditor tenant-wide"
    );
    assert_eq!(
        bind_over_http(&app, &olive, Some(platform.id), "erin", Role::Auditor).await,
        StatusCode::OK,
        "erin is bound the same role at platform only"
    );

    let before = Utc::now();
    let alice_token = issue("alice", tenant);
    let bob_token = issue("bob", tenant);

    // The disclosures this suite answers about are made here, by the
    // product, through the surface a real session uses.
    let (status, _) = inject(&app, &alice_token, "alice-1").await;
    assert_eq!(status, StatusCode::OK, "alice's session start");
    let (status, _) = inject(&app, &bob_token, "bob-1").await;
    assert_eq!(status, StatusCode::OK, "bob's session start");
    let (status, _) = inject(&app, &alice_token, "alice-2").await;
    assert_eq!(status, StatusCode::OK, "alice's second session");

    let _ = bob;
    Some(World {
        pool,
        tenant,
        app,
        alice: alice_token,
        dana: issue("dana", tenant),
        erin: issue("erin", tenant),
        runbook,
        note,
        payments_only,
        before,
    })
}

/// Today's window, as the route takes it: `from` and an explicit `until`.
fn day_window() -> (String, String) {
    let now = Utc::now();
    (
        stamp(now - ChronoDuration::days(1)),
        stamp(now + ChronoDuration::days(1)),
    )
}

fn subjects(disclosed: &Value) -> Vec<String> {
    disclosed
        .as_array()
        .expect("disclosed is an array")
        .iter()
        .map(|row| row["actor_subject"].as_str().expect("subject").to_owned())
        .collect()
}

// ── The acceptance criterion, first question ─────────────────────────

/// **"Who could see X on date D" — one API call.**
///
/// alice and bob were each served the platform runbook by `inject`; carol
/// never was. One `GET /v1/audit/disclosures` names exactly those two, with
/// what each of them actually got, and the answer is stamped with the chain
/// it was taken against so it can be re-derived (ADR-0045 decisions 4
/// and 9).
#[tokio::test]
async fn who_could_see_this_record_is_one_call_answered_from_the_chain() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (status, body) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?record={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "the auditor's one call: {body}");

    let mut who = subjects(&body["disclosed"]);
    who.sort();
    who.dedup();
    assert_eq!(
        who,
        vec!["alice".to_owned(), "bob".to_owned()],
        "exactly the two the chain records being served the runbook: {body}"
    );
    assert!(
        !who.contains(&"carol".to_owned()),
        "carol is on payments and was served nothing of platform's"
    );

    // Each disclosure carries what that reader got, not merely that they
    // got something (ADR-0041 decision 9).
    let first = &body["disclosed"][0];
    assert_eq!(
        first["record_id"].as_str(),
        Some(w.runbook.to_string()).as_deref()
    );
    assert_eq!(
        first["action"].as_str(),
        Some("context.injected"),
        "being *given* material is its own act, kept apart from asking for it"
    );
    assert!(
        first["version_hash"].is_string() || first["object_hash"].is_string(),
        "a disclosure names the version served: {first}"
    );
    assert!(first["seq"].is_i64(), "and the chain row that proves it");

    // The completeness stamp (decision 9).
    assert!(body["head_seq"].as_i64().expect("head_seq") > 0);
    assert_eq!(
        body["head_hash"].as_str().map(str::len),
        Some(64),
        "a BLAKE3 head hash in hex"
    );
    assert_eq!(body["truncated"].as_bool(), Some(false));
}

/// **The two lists are never merged** (ADR-0045 decision 4).
///
/// `disclosed` is evidence; `authority` is the state that governed the
/// window. Collapsing them into one "could see" set would mean deciding
/// over reconstructed inputs, which is the replay ADR-0042 option 5
/// refused — so the response keeps them apart and says why in itself,
/// not only in the ADR.
#[tokio::test]
async fn disclosure_and_authority_arrive_as_two_lists_with_the_reason_in_the_response() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (status, body) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?record={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(body["disclosed"].is_array(), "evidence");
    assert!(body["authority"].is_array(), "inputs");
    assert!(
        body.get("could_see").is_none(),
        "there is deliberately no merged set: {body}"
    );

    // The authority half is the events that opened and closed authority —
    // here, the two role bindings the world established. They exist
    // nowhere else: `role_bindings` is a current-state table, so the
    // chain is the only record that dana held `auditor` today.
    let actions: Vec<&str> = body["authority"]
        .as_array()
        .expect("authority array")
        .iter()
        .map(|event| event["action"].as_str().expect("action"))
        .collect();
    assert!(
        actions.contains(&"role.bound"),
        "the bindings that granted authority are on the chain: {actions:?}"
    );

    let note = body["note"]
        .as_str()
        .expect("the note is part of the answer");
    assert!(
        note.contains("not merged") || note.contains("not merged:"),
        "the response states why the two lists are separate: {note}"
    );
}

// ── The acceptance criterion, second question ────────────────────────

/// **"What did agent A know at time T" — one API call, over bitemporal +
/// refs.**
///
/// One `GET /v1/audit/knowledge` returns what alice was *served*, folded
/// to one row per record with the version last delivered. The AC's
/// "uses bitemporal + refs" is asserted rather than asserted-about: every
/// id in the answer resolves in the bitemporal pair at the instant asked
/// at, and each row names the version by hash.
#[tokio::test]
async fn what_did_this_agent_know_is_one_call_and_its_ids_resolve_bitemporally() {
    let Some(w) = world().await else { return };
    let at = Utc::now();

    let (status, body) = get(
        &w.app,
        &format!("/v1/audit/knowledge?subject=alice&at={}", stamp(at)),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the auditor's one call: {body}");

    let known = body["known"].as_array().expect("known array");
    let ids: Vec<String> = known
        .iter()
        .map(|row| row["record_id"].as_str().expect("record_id").to_owned())
        .collect();
    assert!(
        ids.contains(&w.runbook.to_string()),
        "alice was served the runbook: {body}"
    );
    assert!(
        ids.contains(&w.note.to_string()),
        "and her own note: {body}"
    );
    assert!(
        !ids.contains(&w.payments_only.to_string()),
        "and never payments material"
    );

    // One row per record, not one per delivery: alice injected twice, so
    // the runbook was served twice and folds to a single row that says so.
    assert_eq!(
        ids.len(),
        known
            .iter()
            .map(|row| row["record_id"].as_str().expect("id"))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "the fold is one row per record: {body}"
    );
    let runbook_row = known
        .iter()
        .find(|row| row["record_id"].as_str() == Some(&w.runbook.to_string()))
        .expect("the runbook row");
    assert_eq!(
        runbook_row["occasions"].as_u64(),
        Some(2),
        "alice was served the runbook in both her sessions: {runbook_row}"
    );

    // **refs**: the row names the version it delivered.
    assert!(
        runbook_row["version_hash"].is_string() || runbook_row["object_hash"].is_string(),
        "the answer names a version, which is what makes it checkable: {runbook_row}"
    );

    // **bitemporal**: every id the answer names resolves to the version
    // that was current at the instant asked at. The audit answer and the
    // corpus agree, which is the whole "uses bitemporal + refs" clause.
    let mut tx = synveda_store::rls::begin_tenant_tx(&w.pool, w.tenant)
        .await
        .expect("tenant tx");
    for id in &ids {
        let record_id = RecordId::from_uuid(id.parse().expect("uuid"));
        let version = records::as_of(&mut *tx, record_id, at)
            .await
            .expect("as-of read");
        let version = version.unwrap_or_else(|| {
            panic!("{id} was disclosed at {at} and must resolve in the bitemporal pair")
        });
        assert_eq!(
            version.tenant_id, w.tenant,
            "and it resolves inside the asking tenant"
        );
    }
    drop(tx);

    // And the answer says what it is, in itself.
    assert!(
        body["note"]
            .as_str()
            .expect("note")
            .contains("not what they"),
        "the response states that this is what A was served, not what A \
         was permitted to ask for: {body}"
    );
}

/// The instant is load-bearing: asked *before* any session ran, alice knew
/// nothing. Same call, same subject, same corpus — only `at` differs.
#[tokio::test]
async fn the_instant_decides_what_the_answer_contains() {
    let Some(w) = world().await else { return };

    let (status, before) = get(
        &w.app,
        &format!("/v1/audit/knowledge?subject=alice&at={}", stamp(w.before)),
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        before["known"].as_array().expect("known").is_empty(),
        "before her first session alice had been served nothing: {before}"
    );

    let (status, now) = get(&w.app, "/v1/audit/knowledge?subject=alice", &w.dana).await;
    assert_eq!(status, StatusCode::OK, "`at` defaults to now");
    assert!(
        !now["known"].as_array().expect("known").is_empty(),
        "and by now she has been: {now}"
    );
}

// ── The refusals ─────────────────────────────────────────────────────

/// **A subtree-bound auditor is refused rather than served a subset**
/// (ADR-0045 decision 2).
///
/// erin holds exactly the role dana holds — `auditor` — bound at platform
/// instead of tenant-wide. There is one chain per tenant and no way to
/// answer for part of it without silently omitting the events whose
/// `resource` string does not name a scope, so the answer is a refusal.
/// The Cedar action has no `Scope` in its `appliesTo`, so this holds for
/// every route without any of them checking.
#[tokio::test]
async fn a_subtree_bound_auditor_is_refused_rather_than_served_a_subset() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    for path in [
        "/v1/audit/events".to_owned(),
        format!(
            "/v1/audit/disclosures?record={}&from={from}&until={until}",
            w.runbook
        ),
        "/v1/audit/knowledge?subject=alice".to_owned(),
        "/v1/audit/verify".to_owned(),
    ] {
        let (status, body) = get(&w.app, &path, &w.erin).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "erin's platform binding must not reach the tenant's chain at {path}: {body}"
        );
    }

    // And the same four are open to the same role held tenant-wide, so
    // the refusal is about the *binding* and not about the role.
    let (status, _) = get(&w.app, "/v1/audit/verify", &w.dana).await;
    assert_eq!(status, StatusCode::OK, "dana holds auditor tenant-wide");
}

/// An ordinary member holds nothing on the audit plane: alice can be
/// *asked about* and cannot ask (ADR-0015 decision 4 — no role, no
/// administrative power, under every pack).
#[tokio::test]
async fn the_subject_of_an_audit_answer_cannot_read_the_audit_plane() {
    let Some(w) = world().await else { return };

    for path in ["/v1/audit/events", "/v1/audit/knowledge?subject=alice"] {
        let (status, body) = get(&w.app, path, &w.alice).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "an unbound member reads no trail at {path}: {body}"
        );
    }
}

// ── The properties that hold across every route ──────────────────────

/// **No content reaches an audit answer** (ADR-0045 decision 6).
///
/// Swept over every route's full response body: an auditor holds
/// `AuditRead` and no `MemoryRead`, and the surface has no content path to
/// forget to gate. Record bodies are what an auditor would otherwise
/// acquire here, and this is the last route by which they could.
#[tokio::test]
async fn no_record_content_reaches_any_audit_answer() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    for path in [
        "/v1/audit/events?limit=500".to_owned(),
        format!(
            "/v1/audit/disclosures?record={}&from={from}&until={until}",
            w.runbook
        ),
        "/v1/audit/knowledge?subject=alice".to_owned(),
        "/v1/audit/verify".to_owned(),
    ] {
        let (status, body) = get(&w.app, &path, &w.dana).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let rendered = body.to_string();
        for content in [RUNBOOK, NOTE, PAYMENTS_ONLY] {
            assert!(
                !rendered.contains(content),
                "record content reached {path} — an auditor reads no content (seed §5)"
            );
        }
        // The distinctive middles too, in case a renderer ever truncates.
        for fragment in [
            "acquirer statement",
            "rebase-and-merge",
            "cutover checklist",
        ] {
            assert!(
                !rendered.contains(fragment),
                "a fragment of record content reached {path}: {fragment}"
            );
        }
    }
}

/// **The surface is not an existence oracle** (ADR-0045 compliance notes).
///
/// A record that does not exist and one that exists but was never served
/// answer identically — same status, same shape, same empty list. An
/// auditor who could tell them apart would have a membership oracle over
/// the corpus, which is exactly what `recall`'s uniform refusal exists to
/// prevent (ADR-0041 decision 6).
#[tokio::test]
async fn a_disclosure_answer_is_not_an_existence_oracle() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();
    let nonexistent = RecordId::new();

    let mut answers = Vec::new();
    for record in [nonexistent, w.payments_only] {
        let (status, body) = get(
            &w.app,
            &format!("/v1/audit/disclosures?record={record}&from={from}&until={until}"),
            &w.dana,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "both answer, neither errors");
        assert!(
            body["disclosed"].as_array().expect("disclosed").is_empty(),
            "neither was ever served: {body}"
        );
        answers.push(body);
    }

    assert_eq!(
        answers[0]["disclosed"], answers[1]["disclosed"],
        "a record that never existed and one that was never served are \
         indistinguishable in the answer"
    );
    assert_eq!(
        answers[0]["truncated"], answers[1]["truncated"],
        "including in the completeness stamp"
    );
}

/// **A truncated page says so, and its cursor advances** (ADR-0045
/// decision 9).
///
/// Walked at `limit=1` over the runbook's three-plus disclosures: each
/// page reports itself truncated with a cursor, and the walk collects
/// every disclosure the single-page answer contains. This is the property
/// the page arithmetic exists for — a page whose rows filter out must
/// still advance rather than report itself complete.
#[tokio::test]
async fn a_truncated_page_reports_itself_and_its_cursor_walks_the_whole_answer() {
    let Some(w) = world().await else { return };
    let (from, until) = day_window();

    let (_, whole) = get(
        &w.app,
        &format!(
            "/v1/audit/disclosures?record={}&from={from}&until={until}",
            w.runbook
        ),
        &w.dana,
    )
    .await;
    let total = whole["disclosed"].as_array().expect("disclosed").len();
    assert!(total >= 2, "the world serves the runbook more than once");

    let mut walked = Vec::new();
    let mut after = 0_i64;
    for _ in 0..(total + 2) {
        let (status, page) = get(
            &w.app,
            &format!(
                "/v1/audit/disclosures?record={}&from={from}&until={until}&limit=1&after={after}",
                w.runbook
            ),
            &w.dana,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        walked.extend(subjects(&page["disclosed"]));
        match page["next_cursor"].as_i64() {
            Some(next) => {
                assert_eq!(
                    page["truncated"].as_bool(),
                    Some(true),
                    "a page with a cursor is a truncated page: {page}"
                );
                assert!(
                    next > after,
                    "the cursor must advance: {next} after {after}"
                );
                after = next;
            }
            None => {
                assert_eq!(
                    page["truncated"].as_bool(),
                    Some(false),
                    "the last page is not truncated: {page}"
                );
                break;
            }
        }
    }

    assert_eq!(
        walked.len(),
        total,
        "walking the cursor collects exactly what one page contained"
    );
}

/// **Reading the trail is itself on the trail** (ADR-0045 decision 8).
///
/// An allowed admin-plane read chains its decision, so an audit query
/// appears in the next audit query's results. That is a property rather
/// than an accident: "who has been reading the trail" is a question a
/// regulator asks.
#[tokio::test]
async fn reading_the_trail_appears_in_the_next_reading_of_it() {
    let Some(w) = world().await else { return };

    let (status, first) = get(&w.app, "/v1/audit/events?limit=5", &w.dana).await;
    assert_eq!(status, StatusCode::OK);
    let head_after_first = first["head_seq"].as_i64().expect("head_seq");

    let (status, second) = get(
        &w.app,
        "/v1/audit/events?action=authz.decision&outcome=allow&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let read_by_dana = second["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| {
            event["actor_subject"].as_str() == Some("dana")
                && event["payload"]["authz"]["action"].as_str() == Some("audit.read")
        });
    assert!(
        read_by_dana,
        "dana's first query is on the chain the second one reads: {second}"
    );

    assert!(
        second["head_seq"].as_i64().expect("head_seq") > head_after_first,
        "the chain grew because it was read — which is why the pages are \
         cursor-paginated and not offset-paginated"
    );
}

/// The chain check, reachable by an auditor holding no `DATABASE_URL`
/// (ADR-0045 decision 1) — the surface `synveda audit verify` was standing
/// in for since AUD-1.
#[tokio::test]
async fn the_chain_verifies_over_everything_this_suite_wrote() {
    let Some(w) = world().await else { return };

    let (status, body) = get(&w.app, "/v1/audit/verify", &w.dana).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["valid"].as_bool(),
        Some(true),
        "the chain this suite built verifies: {body}"
    );
    assert!(body["events"].as_i64().expect("events") > 0);
    assert!(body["broken_at"].is_null(), "nothing is broken: {body}");
}

/// A search filters by the whole vocabulary the AC names — actor, action,
/// time — and **denials are an ordinary filter value**, which is the AC's
/// "auditor role read-only incl. denials".
#[tokio::test]
async fn the_search_filters_by_actor_action_and_outcome_including_denials() {
    let Some(w) = world().await else { return };

    // Produce a denial to find: alice asking for the audit plane.
    let (status, _) = get(&w.app, "/v1/audit/events", &w.alice).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, denials) = get(
        &w.app,
        "/v1/audit/events?outcome=deny&action=authz.decision&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let found = denials["events"]
        .as_array()
        .expect("events")
        .iter()
        .any(|event| event["actor_subject"].as_str() == Some("alice"));
    assert!(found, "alice's refusal is readable as a denial: {denials}");

    // By actor: every row is the actor asked for.
    let (status, by_actor) = get(&w.app, "/v1/audit/events?actor=bob&limit=200", &w.dana).await;
    assert_eq!(status, StatusCode::OK);
    for event in by_actor["events"].as_array().expect("events") {
        assert_eq!(event["actor_subject"].as_str(), Some("bob"));
    }
    assert!(
        !by_actor["events"].as_array().expect("events").is_empty(),
        "bob injected, so bob is on the chain"
    );

    // By action: every row is the action asked for.
    let (status, injects) = get(
        &w.app,
        "/v1/audit/events?action=context.injected&limit=200",
        &w.dana,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for event in injects["events"].as_array().expect("events") {
        assert_eq!(event["action"].as_str(), Some("context.injected"));
    }
}

/// A misspelled action is a 400, not an empty answer: "no events" and "you
/// spelled it wrong" are different facts, and only one of them is an audit
/// finding. Same for a limit over the cap, which is refused rather than
/// silently trimmed — an audit surface must not quietly return less than
/// it was asked for.
#[tokio::test]
async fn a_typo_is_refused_rather_than_answered_with_nothing() {
    let Some(w) = world().await else { return };

    for path in [
        "/v1/audit/events?action=context.injcted",
        "/v1/audit/events?outcome=denied",
        "/v1/audit/events?limit=1001",
        "/v1/audit/events?limit=0",
    ] {
        let (status, body) = get(&w.app, path, &w.dana).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{path} must be refused, never answered emptily: {body}"
        );
    }
}

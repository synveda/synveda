//! CPR-10 acceptance criteria at the HTTP surface (ADR-0076): the session
//! ledger and the runtime API.
//!
//! What only the HTTP surface has is what this suite proves: status codes, the
//! PDP on every route, the audit events, the two idempotency mechanisms, and
//! the rules that are about *what a client may say* — no tenant, no acting
//! principal, no governed scope.
//!
//! The store-level contract — the anchor keys, the lifecycle trigger, the
//! append's ordering under its lock — is
//! `crates/synveda-store/tests/sessions.rs`. The tenancy backstop is
//! `crates/synveda-store/tests/rls.rs`.
//!
//! Tests that need a live Postgres read `DATABASE_URL` and skip with a message
//! when it is unset (CI has no database); run them locally with
//! `make db-test`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::telemetry;
use synveda_identity::Hs256Verifier;
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, TenantId};
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-10-sessions-test-secret";
/// Holds `administrator` at the tenant root — the bootstrap grant a first
/// login mints, and what every shipped pack prices the admin planes at.
const ADMIN: &str = "cpr10-admin";
/// A subject with no grant anywhere: every "denies without the action"
/// assertion's caller.
const OUTSIDER: &str = "cpr10-outsider";
/// A subject granted `member` at one project and nothing else.
const MEMBER: &str = "cpr10-member";

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

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
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build the embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr10-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search index"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

fn issue(subject: &str, tenant_id: TenantId) -> String {
    Hs256Verifier::new(SECRET).issue(subject, tenant_id, Duration::from_secs(300))
}

/// Connects, migrates, admits a tenant and seeds the administrator's grant at
/// the tenant root. The seeding is the one break-glass act, and it is the row
/// a first login would mint (CPR-7's `synveda-admins` convention); everything
/// after it goes through the API under the PDP.
async fn admitted_tenant() -> Option<(AppState, TenantId)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping session API test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("cpr10-{}", id.as_uuid().simple());
    synveda_store::tenants::create(
        &pool,
        id,
        &slug,
        "CPR-10 API test",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    let mut tx = synveda_store::rls::begin_tenant_tx(&pool, id)
        .await
        .expect("begin tenant tx");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, id)
        .await
        .expect("mint root");
    grant(&mut tx, id, root.id, ADMIN, RoleKey::Administrator).await;
    tx.commit().await.expect("commit grant");
    Some((state(&url), id))
}

async fn grant(
    tx: &mut sqlx::PgConnection,
    tenant_id: TenantId,
    scope_id: synveda_types::ScopeId,
    principal_id: &str,
    role_key: RoleKey,
) {
    synveda_store::access::create_grant(
        &mut *tx,
        &synveda_store::access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id,
            subject: GrantSubject::Principal {
                principal_id: principal_id.to_owned(),
            },
            role_key,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("write the grant");
}

/// One API call. `key` is the `Idempotency-Key`, when the route takes one.
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("build request"),
        None => request.body(Body::empty()).expect("build request"),
    };
    let response = app.clone().oneshot(request).await.expect("router responds");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A workspace and a project, through the public routes.
async fn seed_place(app: &Router, token: &str, slug: &str) -> (String, String) {
    let (status, workspace) = call(
        app,
        "POST",
        "/v1/workspaces",
        Some(token),
        Some(&format!("ws-{slug}")),
        Some(json!({"slug": slug, "display_name": slug})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{workspace}");
    let workspace_id = workspace["id"].as_str().expect("workspace id").to_owned();
    let (status, project) = call(
        app,
        "POST",
        &format!("/v1/workspaces/{workspace_id}/projects"),
        Some(token),
        Some(&format!("pr-{slug}")),
        Some(json!({"slug": format!("{slug}-app"), "display_name": "App"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let project_id = project["id"].as_str().expect("project id").to_owned();
    (workspace_id, project_id)
}

async fn open_session(app: &Router, token: &str, key: &str, body: Value) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/v1/sessions",
        Some(token),
        Some(key),
        Some(body),
    )
    .await
}

fn event(client_event_id: &str, event_type: &str, payload: Value) -> Value {
    at_event(client_event_id, event_type, "2020-01-01T10:00:00Z", payload)
}

/// An event with an explicit instant. Deliberately in the **past**: a
/// timeline merges a client-asserted `occurred_at` against the server's own
/// clock for a context run, and a fixture dated in the future would place
/// every event after every run and hide whichever ordering bug it had.
fn at_event(client_event_id: &str, event_type: &str, at: &str, payload: Value) -> Value {
    json!({
        "event_type": event_type,
        "client_event_id": client_event_id,
        "occurred_at": at,
        "payload": payload,
    })
}

/// Every audit action in the tenant's chain, in order.
async fn chain_actions(state: &AppState, tenant_id: TenantId) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "select action from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&state.pool)
    .await
    .expect("read the chain")
}

/// The payload of the newest event with this action.
async fn newest_payload(state: &AppState, tenant_id: TenantId, action: &str) -> Value {
    sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 and action = $2 \
         order by seq desc limit 1",
    )
    .bind(tenant_id.as_uuid())
    .bind(action)
    .fetch_one(&state.pool)
    .await
    .expect("read the payload")
}

// ── The whole path, once ─────────────────────────────────────────────────────

/// The end-to-end shape this feature exists for: an agent opens a run, appends
/// what it did, asks for context, ends — and every step is a governed record
/// with an audit event, a timeline that projects over them, and no transcript
/// table anywhere.
#[tokio::test]
async fn an_agent_run_is_a_governed_record_from_open_to_end() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed_place(&app, &token, "payments").await;

    let (status, session) = open_session(
        &app,
        &token,
        "run-1",
        json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "claude-code",
            "client_version": "2.1.0",
            "external_session_id": "harness-abc",
            "agent_name": "reviewer",
            "model_name": "claude-opus-5",
            "branch": "main",
            "task_summary": "Refactor the ledger",
            "metadata": {"cwd": "/work/payments"},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    let session_id = session["id"].as_str().expect("session id").to_owned();
    assert_eq!(session["status"], "active");
    assert_eq!(
        session["principal_id"], ADMIN,
        "the token decides, not a body"
    );
    // The governed scope is derived: it is the *project's*, not the
    // workspace's, and no request said so.
    let (_, project) = call(
        &app,
        "GET",
        &format!("/v1/projects/{project_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(session["scope_id"], project["scope_id"]);

    // Append a batch covering five of the eight families.
    let (status, appended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("e1", "message.user", json!({"text": "refactor the ledger"})),
            event("e2", "tool.invoked", json!({"tool": "grep"})),
            event("e3", "file.changed", json!({"path": "ledger.rs"})),
            event("e4", "command.executed", json!({"command": "cargo test"})),
            event("e5", "adapter.warning", json!({"message": "one hook did not fire"})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
    assert_eq!(appended["appended"], 5);
    assert_eq!(appended["duplicates"], 0);
    // Positions are the server's, contiguous from one.
    let sequences: Vec<i64> = appended["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|entry| entry["event"]["sequence"].as_i64().expect("sequence"))
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    // The payload hash is the server's and is a real digest.
    let hash = appended["events"][0]["event"]["payload_hash"]
        .as_str()
        .expect("hash");
    assert_eq!(hash.len(), 64, "blake3-256, hex");

    // A context run over the same session.
    let (status, run) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        Some(&token),
        Some("ctx-1"),
        Some(json!({"query": "ledger rounding"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(run["session_id"], session_id);
    assert_eq!(run["scope_id"], session["scope_id"]);
    assert!(run["block_hash"].as_str().is_some_and(|h| !h.is_empty()));

    // The timeline projects over both, in one order.
    let (status, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{session_id}/timeline"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let entries = timeline["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 6, "five events and one context run");
    let kinds: Vec<&str> = entries
        .iter()
        .map(|entry| entry["kind"].as_str().expect("kind"))
        .collect();
    assert_eq!(kinds.iter().filter(|kind| **kind == "event").count(), 5);
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "context_run").count(),
        1
    );
    // The events keep the server's `sequence` order and the run is placed
    // among them by instant — a merge, not a sort, so a client's clock can
    // misplace a run and can never reorder a transcript.
    let sequences: Vec<i64> = entries
        .iter()
        .filter(|entry| entry["kind"] == "event")
        .map(|entry| entry["sequence"].as_i64().expect("sequence"))
        .collect();
    assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    assert_eq!(
        entries.last().expect("last")["kind"],
        "context_run",
        "the run's own clock is now, and every event is dated 2020"
    );
    assert_eq!(timeline["event_counts"]["message.user"], 1);
    assert_eq!(timeline["event_counts"]["tool.invoked"], 1);

    // Two-phase close: `ending` first, then `ended`.
    let (status, ending) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/end"),
        Some(&token),
        None,
        Some(json!({"status": "ending"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ending}");
    assert_eq!(ending["status"], "ending");
    assert!(ending.get("ended_at").is_none(), "`ending` is not closed");
    // Buffered events still land while `ending` — the whole reason the state
    // exists.
    let (status, late) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [event("e6", "session.ended", json!({}))]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{late}");
    assert_eq!(late["appended"], 1);

    let (status, ended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/end"),
        Some(&token),
        None,
        Some(json!({"status": "ended", "task_summary": "Ledger rounding fixed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ended}");
    assert_eq!(ended["status"], "ended");
    assert!(ended["ended_at"].as_str().is_some());
    assert_eq!(ended["task_summary"], "Ledger rounding fixed");

    // A closed run takes no more events.
    let (status, refused) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [event("e7", "message.user", json!({"text": "late"}))]})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    // And the chain records every act, in order, once each.
    let actions = chain_actions(&state, tenant_id).await;
    for expected in [
        "session.opened",
        "session.events.appended",
        "session.context.composed",
        "session.ended",
    ] {
        assert!(
            actions.iter().any(|action| action == expected),
            "the chain should carry {expected}: {actions:?}"
        );
    }
    assert_eq!(
        actions
            .iter()
            .filter(|action| *action == "session.opened")
            .count(),
        1,
        "one open, one event"
    );
}

// ── What a client may not say ────────────────────────────────────────────────

/// ADR-0076 decision 8 at the wire: a body naming a tenant, an acting
/// principal or the governed scope is **refused**, not ignored.
///
/// Refused rather than ignored is the whole assertion. A server that silently
/// dropped `principal_id` would behave correctly and teach every client author
/// that the field works.
#[tokio::test]
async fn a_client_cannot_submit_its_own_tenant_or_acting_principal() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed_place(&app, &token, "identity").await;
    let other = TenantId::new();

    for (field, value) in [
        ("tenant_id", json!(other.to_string())),
        ("principal_id", json!("somebody-else")),
        ("scope_id", json!(TenantId::new().to_string())),
        ("status", json!("ended")),
        ("started_at", json!("2020-01-01T00:00:00Z")),
    ] {
        let mut body = json!({
            "workspace_id": workspace_id,
            "client_name": "claude-code",
        });
        body[field] = value;
        let (status, error) = open_session(&app, &token, &format!("id-{field}"), body).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{field}` must be refused, not ignored: {error}"
        );
    }

    // And the ordinary body still works, so the assertion above is about the
    // extra fields and not about the route being broken.
    let (status, session) = open_session(
        &app,
        &token,
        "id-ok",
        json!({"workspace_id": workspace_id, "client_name": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    assert_eq!(session["principal_id"], ADMIN);
}

// ── Idempotency, twice, two different ways ───────────────────────────────────

/// Opening is idempotent by the header; appending is idempotent by the
/// **event**. Both in one test, because the pair is the design: a batch that
/// overlaps a previous one appends what is new and reports `duplicate` for the
/// rest, which a request-level key could not express.
#[tokio::test]
async fn a_retry_creates_nothing_twice_and_a_partial_redelivery_appends_the_rest() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed_place(&app, &token, "retry").await;
    let body = json!({
        "workspace_id": workspace_id,
        "project_id": project_id,
        "client_name": "claude-code",
    });

    let (first, session) = open_session(&app, &token, "same-key", body.clone()).await;
    assert_eq!(first, StatusCode::CREATED);
    let (second, replay) = open_session(&app, &token, "same-key", body.clone()).await;
    assert_eq!(second, StatusCode::OK, "a replay is 200, not 201");
    assert_eq!(replay["id"], session["id"], "and the same session");

    // Same key, different request: a conflict rather than the wrong resource
    // reported as success.
    let mut different = body.clone();
    different["client_name"] = json!("cursor");
    let (status, error) = open_session(&app, &token, "same-key", different).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");

    // No `Idempotency-Key` at all is a 400 naming the header.
    let (status, error) = call(
        &app,
        "POST",
        "/v1/sessions",
        Some(&token),
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|m| m.contains("Idempotency-Key")),
        "the refusal names the header: {error}"
    );

    let session_id = session["id"].as_str().expect("id");
    let (status, first_batch) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("a", "message.user", json!({"text": "one"})),
            event("b", "message.assistant", json!({"text": "two"})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first_batch}");
    assert_eq!(first_batch["appended"], 2);

    // A redelivery that overlaps by one and carries one new event.
    let (status, second_batch) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("b", "message.assistant", json!({"text": "two"})),
            event("c", "tool.invoked", json!({"tool": "grep"})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second_batch}");
    assert_eq!(second_batch["appended"], 1);
    assert_eq!(second_batch["duplicates"], 1);
    assert_eq!(second_batch["events"][0]["outcome"], "duplicate");
    assert_eq!(second_batch["events"][1]["outcome"], "appended");
    // The duplicate serves **the stored row**, at its original position — not
    // the caller's version of it at a new one.
    assert_eq!(second_batch["events"][0]["event"]["sequence"], 2);
    assert_eq!(second_batch["events"][1]["event"]["sequence"], 3);

    // Three events, not five.
    let (_, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{session_id}/timeline"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(timeline["entries"].as_array().expect("entries").len(), 3);

    // One batch that repeats an id inside itself is refused by name: the two
    // would race for a position and one would silently become the other's
    // duplicate.
    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("d", "message.user", json!({})),
            event("d", "message.user", json!({})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

// ── The PDP, on every route ──────────────────────────────────────────────────

/// Every route on this plane refuses a caller who holds nothing, and the
/// refusal is a 403 with the action named — never a 200 with an empty answer,
/// and never a 500.
#[tokio::test]
async fn every_route_refuses_a_caller_who_holds_nothing() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let outsider = issue(OUTSIDER, tenant_id);
    let (workspace_id, project_id) = seed_place(&app, &token, "closed").await;
    let (_, session) = open_session(
        &app,
        &token,
        "closed-1",
        json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "claude-code",
        }),
    )
    .await;
    let session_id = session["id"].as_str().expect("id");

    let probes: Vec<(&str, String, Option<&str>, Option<Value>)> = vec![
        (
            "POST",
            "/v1/sessions".to_owned(),
            Some("outsider-open"),
            Some(json!({"workspace_id": workspace_id, "client_name": "cursor"})),
        ),
        ("GET", "/v1/sessions".to_owned(), None, None),
        ("GET", format!("/v1/sessions/{session_id}"), None, None),
        (
            "POST",
            format!("/v1/sessions/{session_id}/events"),
            None,
            Some(json!({"events": [event("x", "message.user", json!({}))]})),
        ),
        (
            "POST",
            format!("/v1/sessions/{session_id}/end"),
            None,
            Some(json!({"status": "ended"})),
        ),
        (
            "GET",
            format!("/v1/sessions/{session_id}/timeline"),
            None,
            None,
        ),
        (
            "POST",
            format!("/v1/sessions/{session_id}/context-runs"),
            Some("outsider-ctx"),
            Some(json!({})),
        ),
    ];
    for (method, path, key, body) in probes {
        let (status, error) = call(&app, method, &path, Some(&outsider), key, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} must refuse an outsider: {error}"
        );
        assert_eq!(error["kind"], "policy_denied", "{method} {path}: {error}");
        assert!(
            error["action"]
                .as_str()
                .is_some_and(|action| action.starts_with("session.")),
            "{method} {path} names the action it refused: {error}"
        );
    }
}

/// A grant at one project reaches that project's runs and no others, and the
/// listing decides **per row against the row** — CPR-9's rule, applied to this
/// plane from the start rather than retrofitted to it.
#[tokio::test]
async fn a_project_member_sees_that_project_s_runs_and_no_others() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (mine_ws, mine_pr) = seed_place(&app, &token, "mine").await;
    let (theirs_ws, theirs_pr) = seed_place(&app, &token, "theirs").await;

    let (_, mine) = open_session(
        &app,
        &token,
        "mine-1",
        json!({"workspace_id": mine_ws, "project_id": mine_pr, "client_name": "claude-code"}),
    )
    .await;
    let (_, theirs) = open_session(
        &app,
        &token,
        "theirs-1",
        json!({"workspace_id": theirs_ws, "project_id": theirs_pr, "client_name": "claude-code"}),
    )
    .await;

    // The member is granted at *one project's* scope and nothing above it.
    let (_, project) = call(
        &app,
        "GET",
        &format!("/v1/projects/{mine_pr}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let scope_id: synveda_types::ScopeId = project["scope_id"]
        .as_str()
        .expect("scope id")
        .parse()
        .expect("parse scope id");
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin tx");
    grant(&mut tx, tenant_id, scope_id, MEMBER, RoleKey::Member).await;
    tx.commit().await.expect("commit grant");

    let member = issue(MEMBER, tenant_id);
    let (status, listed) = call(&app, "GET", "/v1/sessions", Some(&member), None, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|session| session["id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        ids,
        vec![mine["id"].as_str().expect("id")],
        "exactly the run in the project they hold: {listed}"
    );
    assert_eq!(listed["truncated"], false);

    // And the per-object route agrees with the listing, both ways round —
    // the disagreement CPR-9 found on the workspace plane, asserted absent
    // here.
    let (status, _) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{}", mine["id"].as_str().expect("id")),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, error) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{}", theirs["id"].as_str().expect("id")),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{error}");
}

/// A session id from another tenant is **indistinguishable from one nobody
/// ever minted**: same status, same error kind. CPR-9's rule, on the new
/// plane — a caller who can tell the two apart can enumerate another tenant's
/// runs a uuid at a time.
#[tokio::test]
async fn another_tenant_s_session_id_is_a_404_like_any_other() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let Some((_, other_tenant)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let other_token = issue(ADMIN, other_tenant);

    let (other_ws, other_pr) = seed_place(&app, &other_token, "foreign").await;
    let (_, foreign) = open_session(
        &app,
        &other_token,
        "foreign-1",
        json!({"workspace_id": other_ws, "project_id": other_pr, "client_name": "claude-code"}),
    )
    .await;
    let foreign_id = foreign["id"].as_str().expect("id");
    let fictional = synveda_types::SessionId::new().to_string();

    // The caller holds `administrator` at their own tenant root — the caller
    // who reaches everything else — so a leak here would be the PDP's and not
    // a missing grant's.
    for path in [
        format!("/v1/sessions/{foreign_id}"),
        format!("/v1/sessions/{foreign_id}/timeline"),
    ] {
        let control = path.replace(foreign_id, &fictional);
        let (foreign_status, foreign_error) =
            call(&app, "GET", &path, Some(&token), None, None).await;
        let (control_status, control_error) =
            call(&app, "GET", &control, Some(&token), None, None).await;
        assert_eq!(foreign_status, StatusCode::NOT_FOUND, "{foreign_error}");
        assert_eq!(
            foreign_status, control_status,
            "{path}: a foreign id and a fictional one answer alike"
        );
        assert_eq!(
            foreign_error["kind"], control_error["kind"],
            "{path}: and with the same error kind"
        );
    }

    // Nothing about the foreign run reaches this tenant's listing.
    let (_, listed) = call(&app, "GET", "/v1/sessions", Some(&token), None, None).await;
    assert!(
        !listed.to_string().contains(foreign_id),
        "the listing must not name another tenant's run: {listed}"
    );
}

// ── The rules that are about the rows ────────────────────────────────────────

/// A session names a project it was told about, in the workspace it was told
/// about — and cannot name a project from another workspace, an archived
/// place, or a repository the project does not have.
#[tokio::test]
async fn a_run_can_only_be_opened_somewhere_it_belongs() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws_a, _) = seed_place(&app, &token, "alpha").await;
    let (_, pr_b) = seed_place(&app, &token, "beta").await;

    // A project from another workspace: a 404, not a 409 — from the caller's
    // side that project is not in the workspace they named, and saying which
    // workspace it *is* in answers a question they did not ask.
    let (status, error) = open_session(
        &app,
        &token,
        "cross-1",
        json!({"workspace_id": ws_a, "project_id": pr_b, "client_name": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    // A repository with no project at all.
    let (status, error) = open_session(
        &app,
        &token,
        "repo-1",
        json!({
            "workspace_id": ws_a,
            "client_name": "claude-code",
            "repository_id": synveda_types::RepositoryId::new().to_string(),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // An archived workspace takes no new runs: retiring it would otherwise be
    // advisory.
    let (_, workspace) = call(
        &app,
        "GET",
        &format!("/v1/workspaces/{ws_a}"),
        Some(&token),
        None,
        None,
    )
    .await;
    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/v1/workspaces/{ws_a}"),
        Some(&token),
        None,
        Some(json!({"expected_revision": workspace["revision"], "status": "archived"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, error) = open_session(
        &app,
        &token,
        "archived-1",
        json!({"workspace_id": ws_a, "client_name": "claude-code"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
}

/// The lifecycle is forward-only at the surface as well as at the row: a
/// closed run never reopens, never changes how it closed, and never goes back
/// to `ending`.
#[tokio::test]
async fn a_closed_run_never_reopens() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed_place(&app, &token, "lifecycle").await;
    let (_, session) = open_session(
        &app,
        &token,
        "life-1",
        json!({"workspace_id": workspace_id, "client_name": "claude-code"}),
    )
    .await;
    let session_id = session["id"].as_str().expect("id");
    let end = |status: &str| {
        let path = format!("/v1/sessions/{session_id}/end");
        let body = json!({"status": status});
        let app = app.clone();
        let token = token.clone();
        async move { call(&app, "POST", &path, Some(&token), None, Some(body)).await }
    };

    let (status, failed) = end("failed").await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(failed["status"], "failed");

    for attempt in ["ended", "abandoned", "ending", "failed"] {
        let (status, error) = end(attempt).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a failed run must not become {attempt}: {error}"
        );
        assert!(
            error["message"]
                .as_str()
                .is_some_and(|message| message.contains("failed")),
            "the refusal names the state it is in: {error}"
        );
    }

    // `active` is refused by name rather than by the transition table, because
    // "reopen this" is the request most worth answering clearly.
    let (status, error) = end("active").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

// ── What the chain says, and what it does not ────────────────────────────────

/// The audit payload records that a session carried metadata and how much —
/// and **never what was in it**.
///
/// The rule is the seed's (no secret in an audit payload) and the reason is
/// specific: an agent's environment is where credentials live, and `metadata`
/// is the field a harness would put an environment in. Asserted by putting a
/// credential-shaped value in and looking for it in the chain.
#[tokio::test]
async fn a_session_s_metadata_never_reaches_the_audit_chain() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed_place(&app, &token, "secrets").await;
    let secret = "ghp_ThisLooksExactlyLikeAToken00000000";

    let (status, session) = open_session(
        &app,
        &token,
        "secret-1",
        json!({
            "workspace_id": workspace_id,
            "client_name": "claude-code",
            "metadata": {"env": {"GITHUB_TOKEN": secret}},
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    // It is echoed on the API — it is the caller's own bag, and they sent it.
    assert_eq!(session["metadata"]["env"]["GITHUB_TOKEN"], secret);

    let payload = newest_payload(&state, tenant_id, "session.opened").await;
    assert!(
        !payload.to_string().contains(secret),
        "the chain must not carry a session's metadata: {payload}"
    );
    assert!(
        payload["session"]["metadata_bytes"].as_u64().is_some(),
        "but it records that there was some, and how much: {payload}"
    );

    // The whole chain, not just this event — a second writer would be the
    // failure this test exists to catch.
    let everything: Vec<Value> = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&state.pool)
    .await
    .expect("read the chain");
    assert!(
        !everything.iter().any(|p| p.to_string().contains(secret)),
        "no event anywhere carries it"
    );
}

/// One audit event per batch, carrying counts and the sequence range rather
/// than the events — because the chain is not the transcript store, and a
/// hundred-turn run would otherwise be written twice.
#[tokio::test]
async fn an_append_chains_one_event_however_many_it_carried() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, _) = seed_place(&app, &token, "batching").await;
    let (_, session) = open_session(
        &app,
        &token,
        "batch-1",
        json!({"workspace_id": workspace_id, "client_name": "claude-code"}),
    )
    .await;
    let session_id = session["id"].as_str().expect("id");

    let events: Vec<Value> = (0..20)
        .map(|n| {
            event(
                &format!("e{n}"),
                if n % 2 == 0 {
                    "message.user"
                } else {
                    "tool.invoked"
                },
                json!({"text": format!("turn {n}"), "tool": "grep"}),
            )
        })
        .collect();
    let (status, appended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/events"),
        Some(&token),
        None,
        Some(json!({"events": events})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
    assert_eq!(appended["appended"], 20);

    let actions = chain_actions(&state, tenant_id).await;
    assert_eq!(
        actions
            .iter()
            .filter(|action| *action == "session.events.appended")
            .count(),
        1,
        "one event for twenty: {actions:?}"
    );
    let payload = newest_payload(&state, tenant_id, "session.events.appended").await;
    assert_eq!(payload["appended"], 20);
    assert_eq!(payload["duplicates"], 0);
    assert_eq!(payload["first_sequence"], 1);
    assert_eq!(payload["last_sequence"], 20);
    // The shape of the run, so "what did that agent actually do" is answerable
    // from the chain without reading the events.
    assert_eq!(payload["by_type"]["message.user"], 10);
    assert_eq!(payload["by_type"]["tool.invoked"], 10);
    // And not the events themselves.
    assert!(
        !payload.to_string().contains("turn 7"),
        "the chain carries counts, never content: {payload}"
    );
}

/// The context run is decided, persisted and chained — and its rendered block
/// stays out of the audit payload, exactly as `/v1/inject`'s does.
#[tokio::test]
async fn a_context_run_is_governed_persisted_and_watermarked() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (workspace_id, project_id) = seed_place(&app, &token, "context").await;
    let (_, session) = open_session(
        &app,
        &token,
        "ctx-session",
        json!({
            "workspace_id": workspace_id,
            "project_id": project_id,
            "client_name": "claude-code",
        }),
    )
    .await;
    let session_id = session["id"].as_str().expect("id");

    let (status, run) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        Some(&token),
        Some("ctx-key"),
        Some(json!({"query": "how do we round money", "budget_tokens": 500})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    // An empty corpus composes an empty block, which is a *result* rather than
    // an error — the posture `/v1/inject` has always had.
    assert_eq!(run["entry_count"], 0);
    assert!(run["budget_tokens"].as_i64().is_some_and(|b| b <= 500));
    assert!(run["degraded"].as_array().expect("degraded").is_empty());

    // A replay serves the same run with 200 and composes nothing again.
    let (status, replay) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        Some(&token),
        Some("ctx-key"),
        Some(json!({"query": "how do we round money", "budget_tokens": 500})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["id"], run["id"]);

    let payload = newest_payload(&state, tenant_id, "session.context.composed").await;
    assert_eq!(payload["context_run_id"], run["id"]);
    assert_eq!(payload["block_hash"], run["block_hash"]);
    assert!(
        payload.get("rendered").is_none(),
        "the chain carries the watermark, never the block: {payload}"
    );
    // The per-scope decisions ride along, which is what makes "why was this
    // thin" answerable — the half of explainability that costs nothing now.
    assert!(
        payload["scopes"].as_array().is_some_and(|s| !s.is_empty()),
        "the chain names the scopes the walk decided: {payload}"
    );
    // A query is bounded like every other text input on this product.
    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{session_id}/context-runs"),
        Some(&token),
        Some("ctx-long"),
        Some(json!({"query": "x".repeat(4097)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

/// A brand-new tenant — no scopes, no workspaces — is **answered** rather than
/// errored when it lists sessions.
///
/// The property CPR-9's audit asserted for a caller who holds nothing, applied
/// to a plane whose Cedar action deliberately admits no `Tenant` resource: a
/// run always happens somewhere, so there is nothing to decide about and
/// nothing disclosed.
#[tokio::test]
async fn a_tenant_with_no_scopes_is_answered_rather_than_errored() {
    let _guard = serial().await;
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => return,
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect");
    synveda_store::migrate(&pool).await.expect("migrate");
    let tenant_id = TenantId::new();
    synveda_store::tenants::create(
        &pool,
        tenant_id,
        &format!("cpr10-bare-{}", tenant_id.as_uuid().simple()),
        "CPR-10 bare tenant",
        synveda_types::TenantStatus::Active,
    )
    .await
    .expect("admit tenant");

    let app = router(state(&url));
    let token = issue(ADMIN, tenant_id);
    let (status, listed) = call(&app, "GET", "/v1/sessions", Some(&token), None, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(listed["sessions"].as_array().expect("sessions").is_empty());
    assert_eq!(listed["truncated"], false);
}

/// The listing's filters narrow, and the caller's `limit` is bounded.
#[tokio::test]
async fn the_listing_filters_narrow_and_the_limit_is_bounded() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, pr) = seed_place(&app, &token, "filters").await;

    for n in 0..3 {
        let (status, _) = open_session(
            &app,
            &token,
            &format!("filter-{n}"),
            json!({
                "workspace_id": ws,
                "project_id": if n == 0 { json!(pr) } else { Value::Null },
                "client_name": "claude-code",
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (_, ended) = open_session(
        &app,
        &token,
        "filter-ended",
        json!({"workspace_id": ws, "client_name": "cursor"}),
    )
    .await;
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{}/end", ended["id"].as_str().expect("id")),
        Some(&token),
        None,
        Some(json!({"status": "abandoned"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let count = |listed: &Value| listed["sessions"].as_array().expect("sessions").len();

    let (_, all) = call(&app, "GET", "/v1/sessions", Some(&token), None, None).await;
    assert_eq!(count(&all), 4);

    let (_, in_project) = call(
        &app,
        "GET",
        &format!("/v1/sessions?project_id={pr}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&in_project), 1);

    let (_, abandoned) = call(
        &app,
        "GET",
        "/v1/sessions?status=abandoned",
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&abandoned), 1);
    assert_eq!(abandoned["sessions"][0]["client_name"], "cursor");

    // A `limit` under the number available truncates and **says so**.
    let (_, limited) = call(
        &app,
        "GET",
        "/v1/sessions?limit=2",
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&limited), 2);
    assert_eq!(limited["truncated"], true);

    // Out-of-range and unknown values are refused rather than clamped: a
    // silently clamped limit is a client bug nobody ever finds.
    for query in ["limit=0", "limit=201", "limit=-1", "status=closed"] {
        let (status, error) = call(
            &app,
            "GET",
            &format!("/v1/sessions?{query}"),
            Some(&token),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "?{query}: {error}");
    }
}

/// A `principal`-shaped scope is somebody's own, and the base layer's privacy
/// forbid reaches this plane too: a session opened at a person's own scope is
/// not readable by the tenant administrator — the one caller who reaches
/// everything else.
///
/// This is the assertion that would fail if `SessionRead` had been added to the
/// governance carve-out in `base.cedar`, which is exactly the mistake worth
/// pinning.
#[tokio::test]
async fn a_run_at_somebody_s_own_scope_is_not_the_administrator_s_to_read() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let admin = issue(ADMIN, tenant_id);

    // The member's own principal scope, and a workspace they own, so they can
    // open a run at all.
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin tx");
    let own = synveda_store::scopes::ensure_principal_scope(&mut tx, tenant_id, MEMBER, MEMBER)
        .await
        .expect("mint the principal scope");
    tx.commit().await.expect("commit");

    // Nothing this test asserts depends on a session actually existing at the
    // own scope — what it asserts is what the *decision* says about it, which
    // is the capability probe's own question.
    let (status, probe) = call(
        &app,
        "GET",
        &format!("/v1/capabilities?scopes={}", own.id),
        Some(&admin),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{probe}");
    let node = &probe["capabilities"][0];
    assert_eq!(
        node["actions"]["session.read"], false,
        "an administrator is offered nothing at somebody else's own scope: {probe}"
    );
    assert_eq!(node["actions"]["session.write"], false, "{probe}");
}

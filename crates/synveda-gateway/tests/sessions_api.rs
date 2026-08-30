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

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, behavior_test_router as router};
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
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        context_embed_timeout: Duration::from_millis(100),
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("apply migrations");
    let id = TenantId::new();
    let slug = format!("cpr10-{}", id.as_uuid().simple());
    tenant_fixture::create(
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
        tx,
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
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let actions = sqlx::query_scalar::<_, String>(
        "select action from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .expect("read the chain");
    tx.commit().await.expect("commit chain read");
    actions
}

/// The payload of the newest event with this action.
async fn newest_payload(state: &AppState, tenant_id: TenantId, action: &str) -> Value {
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let payload = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 and action = $2 \
         order by seq desc limit 1",
    )
    .bind(tenant_id.as_uuid())
    .bind(action)
    .fetch_one(&mut *tx)
    .await
    .expect("read the payload");
    tx.commit().await.expect("commit payload read");
    payload
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
    assert!(
        listed["next_cursor"].is_null(),
        "one page held everything: {listed}"
    );

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

/// The three global routes are **gone**, not deprecated (CPR-12, ADR-0078).
///
/// Asserted rather than assumed, and asserted for the reason CPR-7 asserted
/// the hierarchy 404s: a route that is deleted from a handler module but left
/// mounted somewhere — a stale `.route(...)`, a fallback that swallows the
/// path, a router someone re-added while merging — answers 200 to a caller
/// that has not been updated, and the two planes are both live again with
/// nobody told. The cheapest way to know is to ask.
///
/// **A 405 would fail this too, and should.** `405 Method Not Allowed` means
/// the path is still mounted for some other verb, which is a route that
/// exists. Only "there is nothing here" is the right answer.
#[tokio::test]
async fn the_three_global_routes_are_deleted_rather_than_deprecated() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);

    // A body each route would once have accepted, so a 404 cannot be a
    // rejected payload wearing the wrong status.
    let bodies = [
        (
            "/v1/observe",
            json!({"session_id": "s1", "events": [{
                "idempotency_key": "k1",
                "kind": "transcript_delta",
                "payload": {"text": "hello"},
                "occurred_at": "2026-08-23T10:00:00Z"
            }]}),
        ),
        (
            "/v1/inject",
            json!({"session_id": "s1", "task": "why retries"}),
        ),
        (
            "/v1/recall",
            json!({"session_id": "s1", "query": "why retries"}),
        ),
    ];

    for (path, body) in bodies {
        let (status, error) = call(&app, "POST", path, Some(&token), None, Some(body)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} must be gone, not merely unhappy with the body: {error}"
        );
    }

    // And they are not hiding behind another verb.
    for path in ["/v1/observe", "/v1/inject", "/v1/recall"] {
        let (status, error) = call(&app, "GET", path, Some(&token), None, None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{path} answers GET, so the path is still mounted: {error}"
        );
    }
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
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let everything: Vec<Value> = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .expect("read the chain");
    tx.commit().await.expect("commit chain read");
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
/// stays out of the audit payload.
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
    assert_eq!(
        run["degraded"],
        json!(["embedder"]),
        "the deterministic test embedder is honestly reported as non-semantic"
    );

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
    assert_eq!(payload["authz"]["action"], json!("session.write"));
    assert_eq!(payload["knowledge"], json!([]));
    assert!(payload["retrieval_version"].is_string());
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
    synveda_store::epoch::verify(&pool)
        .await
        .expect("verify current schema");
    let tenant_id = TenantId::new();
    tenant_fixture::create(
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
    assert!(listed["next_cursor"].is_null());
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

    // A `limit` under the number available fills one page and **says where
    // the next one starts** — CPR-11 replaced CPR-10's `truncated`, which
    // could say that an answer was cut short and could not say where to
    // continue.
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
    assert!(limited["next_cursor"].is_string(), "{limited}");
    assert!(
        limited.get("truncated").is_none(),
        "the boolean it replaced is gone rather than kept beside it: {limited}"
    );

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

// ── CPR-11: the product surface over the ledger (ADR-0077) ───────────────────

/// Every page of a listing is reachable, and the walk terminates.
///
/// This is the property CPR-10 could not offer: `truncated` said *that* an
/// answer was cut short and never where to continue, so a run from last
/// Tuesday was unreachable through the API. The assertion is deliberately the
/// whole walk rather than one hop — a cursor that repeats a row, skips one, or
/// never clears is a bug that only a full traversal shows.
#[tokio::test]
async fn a_listing_pages_through_every_run_exactly_once() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "paging").await;

    let mut opened: Vec<String> = Vec::new();
    for n in 0..5 {
        let (status, session) = open_session(
            &app,
            &token,
            &format!("page-{n}"),
            json!({"workspace_id": ws, "client_name": "claude-code"}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{session}");
        opened.push(session["id"].as_str().expect("id").to_owned());
    }

    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    // Bounded: a walk that cannot terminate must fail this test rather than
    // hang the suite.
    for _ in 0..10 {
        let path = match &cursor {
            Some(cursor) => format!("/v1/sessions?limit=2&cursor={cursor}"),
            None => "/v1/sessions?limit=2".to_owned(),
        };
        let (status, page) = call(&app, "GET", &path, Some(&token), None, None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        for session in page["sessions"].as_array().expect("sessions") {
            seen.push(session["id"].as_str().expect("id").to_owned());
        }
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => {
                cursor = None;
                break;
            }
        }
    }
    assert!(cursor.is_none(), "the walk ended rather than running out");

    // Every run, once, newest first — which is the reverse of the order they
    // were opened in.
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), seen.len(), "a row was served twice: {seen:?}");
    opened.reverse();
    assert_eq!(seen, opened, "every run, newest first");

    // A cursor this listing did not issue is refused rather than silently
    // restarting from the newest row, which is how an infinite scroll becomes
    // one nobody notices.
    for bad in ["not-base64!!", "YWJj", ""] {
        let (status, error) = call(
            &app,
            "GET",
            &format!("/v1/sessions?cursor={bad}"),
            Some(&token),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "cursor={bad}: {error}");
    }
}

/// The four filters this surface adds, and the half-open date range.
#[tokio::test]
async fn the_listing_narrows_by_client_by_who_and_by_day() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "narrowing").await;

    // A second principal, so the `principal_id` filter has something to
    // separate. They need a grant to open a run at all.
    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .expect("begin tx");
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("the tenant root");
    grant(&mut tx, tenant_id, root.id, MEMBER, RoleKey::Member).await;
    tx.commit().await.expect("commit grant");
    let member = issue(MEMBER, tenant_id);

    for (key, token, client) in [
        ("narrow-1", &token, "claude-code"),
        ("narrow-2", &token, "zed"),
        ("narrow-3", &member, "claude-code"),
    ] {
        let (status, session) = open_session(
            &app,
            token,
            key,
            json!({"workspace_id": ws, "client_name": client}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{session}");
    }

    let count = |listed: &Value| listed["sessions"].as_array().expect("sessions").len();

    let (_, by_client) = call(
        &app,
        "GET",
        "/v1/sessions?client_name=zed",
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&by_client), 1);
    assert_eq!(by_client["sessions"][0]["client_name"], "zed");

    // An exact match, never a prefix: `zed` and `zed-nightly` are two clients.
    let (_, prefix) = call(
        &app,
        "GET",
        "/v1/sessions?client_name=ze",
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&prefix), 0);

    let (_, by_who) = call(
        &app,
        "GET",
        &format!("/v1/sessions?principal_id={MEMBER}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&by_who), 1);
    assert_eq!(by_who["sessions"][0]["principal_id"], MEMBER);

    // The day these runs were opened is today, and the range is half-open —
    // so a window that ends at this instant excludes them and one that ends
    // tomorrow includes them.
    let now = chrono::Utc::now();
    // `Z`, not `+00:00`: `to_rfc3339` renders the offset with a `+`, and a
    // bare `+` in a query string decodes to a space — which the route refuses
    // as "input contains invalid characters". A real client either encodes it
    // or sends `Z`; this sends `Z`.
    let instant =
        |at: chrono::DateTime<chrono::Utc>| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let tomorrow = now + chrono::Duration::days(1);
    let yesterday = now - chrono::Duration::days(1);
    let (_, windowed) = call(
        &app,
        "GET",
        &format!(
            "/v1/sessions?started_after={}&started_before={}",
            instant(yesterday),
            instant(tomorrow)
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&windowed), 3);

    let (_, before_them) = call(
        &app,
        "GET",
        &format!("/v1/sessions?started_before={}", instant(yesterday)),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(count(&before_them), 0);

    // A window that ends before it starts is refused rather than answered
    // with an empty list, which would read as "no runs" instead of "that is
    // not a window".
    let (status, error) = call(
        &app,
        "GET",
        &format!(
            "/v1/sessions?started_after={}&started_before={}",
            instant(tomorrow),
            instant(yesterday)
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

/// A run says **how** it ended, not only that it did — and the reason reaches
/// the chain.
#[tokio::test]
async fn a_close_carries_a_reason_and_the_chain_records_it() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "reasons").await;
    let (_, session) = open_session(
        &app,
        &token,
        "reason-1",
        json!({"workspace_id": ws, "client_name": "claude-code"}),
    )
    .await;
    let id = session["id"].as_str().expect("id").to_owned();
    assert!(
        session["end_reason"].is_null(),
        "an open run has no reason yet: {session}"
    );

    let (status, closed) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/end"),
        Some(&token),
        None,
        Some(json!({"status": "failed", "end_reason": "the SessionEnd hook timed out"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{closed}");
    assert_eq!(closed["status"], "failed");
    assert_eq!(closed["end_reason"], "the SessionEnd hook timed out");

    // It survives a re-read, and it is on the chain: "why did that run fail"
    // is the question an auditor asks and the status alone cannot answer.
    let (_, reread) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(reread["end_reason"], "the SessionEnd hook timed out");
    let chained = newest_payload(&state, tenant_id, "session.ended").await;
    assert_eq!(chained["end_reason"], "the SessionEnd hook timed out");
    assert_eq!(chained["to"], "failed");

    // A reason over its bound is refused rather than truncated: a silently
    // shortened explanation is worse than none.
    let (_, another) = open_session(
        &app,
        &token,
        "reason-2",
        json!({"workspace_id": ws, "client_name": "claude-code"}),
    )
    .await;
    let (status, error) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{}/end", another["id"].as_str().expect("id")),
        Some(&token),
        None,
        Some(json!({"status": "failed", "end_reason": "x".repeat(501)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

/// The timeline carries **both clocks**, and says which entries did not
/// arrive live.
///
/// The reason this is on the server rather than left to each client: a spooled
/// batch, a replay after a crash and a machine with a wrong clock all look
/// identical from here, and one definition of "late" is what keeps the
/// console, the CLI and anything else that reads a timeline agreeing about
/// what they are looking at.
#[tokio::test]
async fn a_timeline_reports_both_clocks_and_marks_what_arrived_late() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "clocks").await;
    let (_, session) = open_session(
        &app,
        &token,
        "clocks-1",
        json!({"workspace_id": ws, "client_name": "claude-code"}),
    )
    .await;
    let id = session["id"].as_str().expect("id").to_owned();

    // One event that happened just now, and one that happened two hours ago
    // and is only being delivered now — which is exactly what an adapter
    // flushing a spool sends.
    let now = chrono::Utc::now();
    let long_ago = now - chrono::Duration::hours(2);
    let (status, appended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            at_event("live", "message.user", &now.to_rfc3339(), json!({"text": "fix the rounding"})),
            at_event(
                "spooled",
                "message.assistant",
                &long_ago.to_rfc3339(),
                json!({"text": "here is the patch"}),
            ),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");

    let (status, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/timeline"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let entries = timeline["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert!(
            entry["received_at"].is_string(),
            "every event carries the second clock: {entry}"
        );
    }
    // Ordered by `sequence`, not by instant — the merge's own property, and
    // the reason the delivered-two-hours-ago event is second.
    assert!(!entries[0]["delayed"].as_bool().expect("delayed"));
    assert!(
        entries[1]["delayed"].as_bool().expect("delayed"),
        "an event two hours behind its own timestamp did not arrive live: {}",
        entries[1]
    );

    // A context run has one clock and cannot be late: it is composed here.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/context-runs"),
        Some(&token),
        Some("clocks-ctx"),
        Some(json!({"query": "the ledger"})),
    )
    .await;
    // A fresh composition is a creation; only a replayed `Idempotency-Key`
    // answers 200.
    assert_eq!(status, StatusCode::CREATED);
    let (_, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/timeline"),
        Some(&token),
        None,
        None,
    )
    .await;
    let run = timeline["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["kind"] == "context_run")
        .expect("the context run");
    assert!(run["received_at"].is_null(), "{run}");
    assert_eq!(run["delayed"], false);
}

/// An adapter warning is an event like any other and is **counted** like one,
/// so a reader can see that a client could not do something without reading
/// every line of the run.
#[tokio::test]
async fn a_failed_delivery_is_a_warning_the_timeline_counts() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "warnings").await;
    let (_, session) = open_session(
        &app,
        &token,
        "warn-1",
        json!({"workspace_id": ws, "client_name": "claude-code"}),
    )
    .await;
    let id = session["id"].as_str().expect("id").to_owned();

    let (status, _) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("w-1", "adapter.warning", json!({
                "message": "could not deliver 4 events; spooled to disk",
            })),
            event("m-1", "message.user", json!({"text": "carry on"})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/timeline"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(timeline["event_counts"]["adapter.warning"], 1);
    let warning = timeline["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["event_type"] == "adapter.warning")
        .expect("the warning");
    // The summary carries the client's own sentence: the console renders it
    // and does not compose a second one.
    assert!(
        warning["summary"]
            .as_str()
            .expect("summary")
            .contains("could not deliver 4 events"),
        "{warning}"
    );
}

/// A raw payload is a different disclosure from a timeline, so it is a
/// different authority — and the split is asserted with **one caller** who
/// holds one and not the other.
#[tokio::test]
async fn a_payload_takes_diagnostics_and_a_timeline_does_not() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, pr) = seed_place(&app, &token, "diagnostics").await;
    let (_, session) = open_session(
        &app,
        &token,
        "diag-1",
        json!({"workspace_id": ws, "project_id": pr, "client_name": "claude-code"}),
    )
    .await;
    let id = session["id"].as_str().expect("id").to_owned();
    let (status, appended) = call(
        &app,
        "POST",
        &format!("/v1/sessions/{id}/events"),
        Some(&token),
        None,
        Some(json!({"events": [
            event("d-1", "message.user", json!({"text": "the API key is hunter2"})),
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
    let event_id = appended["events"][0]["event"]["id"]
        .as_str()
        .expect("event id")
        .to_owned();

    // The member holds `member` at the project and nothing above it. Under the
    // shipped `regulated-strict` pack that reads a timeline and does **not**
    // read a payload, which is the whole of what this action is for.
    let (_, project) = call(
        &app,
        "GET",
        &format!("/v1/projects/{pr}"),
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

    let (status, timeline) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/timeline"),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the member reads the run: {timeline}"
    );
    // And the timeline it reads carries no payload at all — a summary is not
    // the content, and this is the assertion that would fail if somebody put
    // one on the entry "for convenience".
    for entry in timeline["entries"].as_array().expect("entries") {
        assert!(entry.get("payload").is_none(), "{entry}");
        assert!(
            !entry.to_string().contains("hunter2"),
            "a timeline is not a transcript: {entry}"
        );
    }

    let (status, refusal) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/events/{event_id}"),
        Some(&member),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refusal}");
    assert_eq!(refusal["action"], "session.diagnostics");

    // The administrator holds it, and gets the bytes.
    let (status, expanded) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{id}/events/{event_id}"),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    assert_eq!(expanded["payload"]["text"], "the API key is hunter2");
    assert_eq!(expanded["client_event_id"], "d-1");

    // The chain says which event was expanded and **never what was in it**:
    // an audit log that copied every prompt somebody read would be a second
    // transcript store with weaker access rules than the first.
    let mut tx = tenant_fixture::begin(&state.pool, tenant_id).await;
    let chain = sqlx::query_scalar::<_, Value>(
        "select payload from audit_log where tenant_id = $1 order by seq",
    )
    .bind(tenant_id.as_uuid())
    .fetch_all(&mut *tx)
    .await
    .expect("read the chain");
    tx.commit().await.expect("commit diagnostic chain read");
    let whole = serde_json::to_string(&chain).expect("serialise");
    assert!(!whole.contains("hunter2"), "the chain carries no payload");
    assert!(
        whole.contains("session.event.diagnostic"),
        "and it does record that somebody expanded one"
    );
}

/// An event id from another run, another tenant, or nowhere at all is the
/// same 404 — the ownership check before the decision, on the newest route.
#[tokio::test]
async fn an_event_of_another_run_is_missing_rather_than_refused() {
    let _guard = serial().await;
    let Some((state, tenant_id)) = admitted_tenant().await else {
        return;
    };
    let app = router(state.clone());
    let token = issue(ADMIN, tenant_id);
    let (ws, _) = seed_place(&app, &token, "ownership").await;

    let mut ids = Vec::new();
    for n in 0..2 {
        let (_, session) = open_session(
            &app,
            &token,
            &format!("own-{n}"),
            json!({"workspace_id": ws, "client_name": "claude-code"}),
        )
        .await;
        let id = session["id"].as_str().expect("id").to_owned();
        let (status, appended) = call(
            &app,
            "POST",
            &format!("/v1/sessions/{id}/events"),
            Some(&token),
            None,
            Some(json!({"events": [event(&format!("e-{n}"), "message.user", json!({}))]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        ids.push((
            id,
            appended["events"][0]["event"]["id"]
                .as_str()
                .expect("event id")
                .to_owned(),
        ));
    }

    // The first run's event, asked for under the second run's id.
    let (status, error) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{}/events/{}", ids[1].0, ids[0].1),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");

    // And an id nobody ever minted answers identically, so the route is not
    // an existence oracle for another run's ledger.
    let (fictional_status, fictional) = call(
        &app,
        "GET",
        &format!(
            "/v1/sessions/{}/events/{}",
            ids[1].0,
            synveda_types::SessionEventId::new()
        ),
        Some(&token),
        None,
        None,
    )
    .await;
    assert_eq!(fictional_status, status);
    assert_eq!(fictional["kind"], error["kind"]);

    // A caller who holds nothing is refused on this route like every other.
    let (status, refusal) = call(
        &app,
        "GET",
        &format!("/v1/sessions/{}/events/{}", ids[0].0, ids[0].1),
        Some(&issue(OUTSIDER, tenant_id)),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refusal}");
}

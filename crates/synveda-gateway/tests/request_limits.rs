//! Request admission and durable tenant usage through the public API.
//!
//! Ordinary tenant identities drive enforcing limits on one gateway router:
//! independent principal and tenant budgets, refusal before any database
//! work, a source budget that unverified credentials cannot escape, permits
//! that span body extraction and are released on cancellation, exactly-once
//! usage under duplicates and replay, expiring narrowing overrides governed
//! by Cedar with content-free audit, observe mode, and what a restart keeps.

#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use synveda_gateway::app::behavior_test_router_with_limits;
use synveda_gateway::limits::{ClassLimits, Limiter, Limits, OperationClass, RateLimit};

const PAYLOAD_TEXT: &str = "A request-limits assistant turn about the tern-7 rollout.";

fn limited(state: &AppState, limits: Limits) -> (Router, Limiter) {
    let limiter = Limiter::new(limits).expect("valid limits");
    (
        behavior_test_router_with_limits(state.clone(), limiter.clone()),
        limiter,
    )
}

fn compose_budget(principal_burst: u32, tenant_burst: u32) -> Limits {
    let mut limits = Limits::default();
    limits.set_class(
        OperationClass::ContextCompose,
        ClassLimits {
            principal_rate: RateLimit {
                per_minute: 1,
                burst: principal_burst,
            },
            tenant_rate: RateLimit {
                per_minute: 1,
                burst: tenant_burst,
            },
            ..*Limits::default().class(OperationClass::ContextCompose)
        },
    );
    limits
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

async fn send(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<Value>,
    peer: Option<&str>,
) -> Reply {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    let mut request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request"),
        None => request.body(Body::empty()).expect("request"),
    };
    if let Some(peer) = peer {
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(
            peer.parse().expect("ip"),
            40_000,
        )));
    }
    let response = app.clone().oneshot(request).await.expect("router responds");
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    Reply {
        status,
        headers,
        body,
    }
}

fn assert_limited(reply: &Reply) {
    assert_eq!(
        reply.status,
        StatusCode::TOO_MANY_REQUESTS,
        "{}",
        reply.body
    );
    assert_eq!(reply.body["kind"], "rate_limited", "{}", reply.body);
    let retry: u64 = reply.headers["retry-after"]
        .to_str()
        .expect("ascii")
        .parse()
        .expect("seconds");
    assert!((1..=60).contains(&retry), "retry-after {retry}");
}

async fn compose(app: &Router, token: &str, session: &str, key: &str) -> Reply {
    send(
        app,
        "POST",
        &format!("/v1/sessions/{session}/context-runs"),
        Some(token),
        Some(key),
        Some(json!({"query": "tern-7 rollout", "budget_tokens": 800})),
        None,
    )
    .await
}

fn events(ids: &[&str]) -> (Value, i64) {
    let payload = json!({"text": PAYLOAD_TEXT});
    let bytes = serde_json::to_string(&payload).expect("encode").len() as i64;
    let events: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({
                "event_type": "message.assistant",
                "client_event_id": id,
                "occurred_at": "2026-10-27T12:00:00Z",
                "payload": payload,
            })
        })
        .collect();
    (json!({"events": events}), bytes * ids.len() as i64)
}

async fn append(app: &Router, token: &str, session: &str, ids: &[&str]) -> Reply {
    send(
        app,
        "POST",
        &format!("/v1/sessions/{session}/events"),
        Some(token),
        None,
        Some(events(ids).0),
        None,
    )
    .await
}

async fn usage_units(state: &AppState, tenant: TenantId, meter: &str) -> i64 {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("tenant tx");
    let units: Option<i64> = sqlx::query_scalar(
        "select units from tenant_usage where tenant_id = $1 and meter = $2
           and usage_day = (statement_timestamp() at time zone 'UTC')::date",
    )
    .bind(tenant.as_uuid())
    .bind(meter)
    .fetch_optional(&mut *tx)
    .await
    .expect("usage");
    tx.commit().await.expect("commit");
    units.unwrap_or(0)
}

async fn count(state: &AppState, tenant: TenantId, table: &str) -> i64 {
    let mut tx = rls::begin_tenant_tx(&state.pool, tenant)
        .await
        .expect("tenant tx");
    let sql = format!("select count(*) from {table} where tenant_id = $1");
    let rows: i64 = sqlx::query_scalar(&sql)
        .bind(tenant.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    tx.commit().await.expect("commit");
    rows
}

#[tokio::test]
async fn principal_and_tenant_budgets_are_independent_and_refuse_before_database_work() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let Some(globex) = admitted_world().await else {
        return;
    };
    let (app, _) = limited(&acme.state, compose_budget(2, 3));
    for key in ["acme-alice-1", "acme-alice-2"] {
        let reply = compose(&app, &acme.alice_token, &acme.alice_session, key).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }
    let refused = compose(&app, &acme.alice_token, &acme.alice_session, "acme-alice-3").await;
    assert_limited(&refused);
    let bob = compose(&app, &acme.bob_token, &acme.bob_session, "acme-bob-1").await;
    assert_eq!(
        bob.status,
        StatusCode::CREATED,
        "bob has his own principal budget"
    );
    let tenant_spent = compose(&app, &acme.bob_token, &acme.bob_session, "acme-bob-2").await;
    assert_limited(&tenant_spent);
    assert_eq!(
        tenant_spent.body, refused.body,
        "principal and tenant refusals are indistinguishable"
    );
    for key in ["globex-alice-1", "globex-alice-2"] {
        let reply = compose(&app, &globex.alice_token, &globex.alice_session, key).await;
        assert_eq!(
            reply.status,
            StatusCode::CREATED,
            "another tenant's budget is untouched: {}",
            reply.body
        );
    }
    let other_class = send(
        &app,
        "GET",
        "/v1/me",
        Some(&acme.alice_token),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(
        other_class.status,
        StatusCode::OK,
        "reads have their own budget"
    );
    assert_eq!(
        count(&acme.state, acme.tenant_id, "session_context_runs").await,
        3
    );
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "context_runs").await,
        3
    );

    // The same limiter over an unreachable database: a refused caller still
    // gets 429, so admission ran before any database work, while an admitted
    // caller reaches the database and fails as storage.
    let mut offline = acme.state.clone();
    offline.pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .expect("lazy pool");
    let (_, limiter) = limited(&acme.state, compose_budget(2, 3));
    let offline_app = behavior_test_router_with_limits(offline, limiter.clone());
    let live_app = behavior_test_router_with_limits(acme.state.clone(), limiter);
    for key in ["acme-carol-1", "acme-carol-2"] {
        let reply = compose(&live_app, &acme.alice_token, &acme.alice_session, key).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }
    assert_limited(
        &compose(
            &offline_app,
            &acme.alice_token,
            &acme.alice_session,
            "acme-carol-3",
        )
        .await,
    );
    let admitted = compose(
        &offline_app,
        &globex.alice_token,
        &globex.alice_session,
        "g-3",
    )
    .await;
    assert_eq!(
        admitted.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        admitted.body
    );
}

#[tokio::test]
async fn unverified_credentials_spend_only_their_source_and_allocate_no_identity() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let mut limits = Limits::default();
    limits.sources.per_source = RateLimit {
        per_minute: 1,
        burst: 5,
    };
    limits.capacity.sources = 64;
    let (app, limiter) = limited(&acme.state, limits);
    let forged = Hs256Verifier::new(b"not-the-gateway-secret").issue(
        ALICE,
        acme.tenant_id,
        Duration::from_secs(300),
    );
    for token in ["garbage", "a.b.c", &forged, &forged, "garbage"] {
        let reply = send(
            &app,
            "GET",
            "/v1/me",
            Some(token),
            None,
            None,
            Some("203.0.113.7"),
        )
        .await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    for token in ["garbage", acme.alice_token.as_str()] {
        assert_limited(
            &send(
                &app,
                "GET",
                "/v1/me",
                Some(token),
                None,
                None,
                Some("203.0.113.7"),
            )
            .await,
        );
    }
    assert_eq!(
        limiter.remembered(),
        (0, 0, 1),
        "no principal or tenant state for unverified credentials"
    );
    let elsewhere = send(
        &app,
        "GET",
        "/v1/me",
        Some(&acme.alice_token),
        None,
        None,
        Some("198.51.100.4"),
    )
    .await;
    assert_eq!(
        elsewhere.status,
        StatusCode::OK,
        "the tenant itself is unaffected"
    );
    for octet in 0..=255_u8 {
        let peer = format!("192.0.2.{octet}");
        send(
            &app,
            "GET",
            "/v1/me",
            Some("garbage"),
            None,
            None,
            Some(&peer),
        )
        .await;
    }
    let (principals, tenants, sources) = limiter.remembered();
    assert_eq!((principals, tenants), (1, 1));
    assert!(sources <= 64, "{sources} sources remembered");
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "context_runs").await,
        0
    );
}

#[tokio::test]
async fn permits_span_body_extraction_and_cancellation_releases_them() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let mut limits = Limits::default();
    limits.set_class(
        OperationClass::SessionIngest,
        ClassLimits {
            principal_in_flight: 1,
            tenant_in_flight: 1,
            process_in_flight: 2,
            max_wait: Duration::from_millis(300),
            ..*Limits::default().class(OperationClass::SessionIngest)
        },
    );
    let (app, _) = limited(&acme.state, limits);
    let reading = Arc::new(tokio::sync::Notify::new());
    let stalled = {
        let (app, reading) = (app.clone(), Arc::clone(&reading));
        let path = format!("/v1/sessions/{}/events", acme.alice_session);
        let token = acme.alice_token.clone();
        tokio::spawn(async move {
            let body = futures_util::stream::once(async move {
                reading.notify_one();
                std::future::pending::<std::result::Result<axum::body::Bytes, std::io::Error>>()
                    .await
            });
            let request = Request::post(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from_stream(body))
                .expect("request");
            app.oneshot(request).await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), reading.notified())
        .await
        .expect("the admitted handler is reading the body");
    let started = Instant::now();
    let queued = append(&app, &acme.bob_token, &acme.bob_session, &["queued-1"]).await;
    assert_limited(&queued);
    assert!(
        started.elapsed() >= Duration::from_millis(300),
        "it waited first"
    );

    stalled.abort();
    assert!(stalled.await.is_err(), "the stalled request was cancelled");
    let admitted = append(&app, &acme.bob_token, &acme.bob_session, &["after-cancel"]).await;
    assert_eq!(admitted.status, StatusCode::OK, "{}", admitted.body);
    assert_eq!(admitted.body["appended"], 1);
}

#[tokio::test]
async fn accepted_work_is_charged_exactly_once_and_rejected_work_never() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let (app, _) = limited(&acme.state, Limits::default());
    let bytes = |ids: &[&str]| events(ids).1;

    let first = append(&app, &acme.alice_token, &acme.alice_session, &["e1", "e2"]).await;
    assert_eq!(first.body["appended"], 2, "{}", first.body);
    let replay = append(&app, &acme.alice_token, &acme.alice_session, &["e1", "e2"]).await;
    assert_eq!(replay.body["duplicates"], 2, "{}", replay.body);
    let (left, right) = tokio::join!(
        append(&app, &acme.alice_token, &acme.alice_session, &["e3", "e4"]),
        append(&app, &acme.alice_token, &acme.alice_session, &["e3", "e4"]),
    );
    assert_eq!(left.status, StatusCode::OK);
    assert_eq!(right.status, StatusCode::OK);
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "session_event_bytes").await,
        bytes(&["e1", "e2", "e3", "e4"]),
        "duplicates and a concurrent identical batch charge once"
    );

    let created = compose(&app, &acme.alice_token, &acme.alice_session, "once-1").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let replayed = compose(&app, &acme.alice_token, &acme.alice_session, "once-1").await;
    assert_eq!(
        replayed.status,
        StatusCode::OK,
        "a lost reply is retried as a replay"
    );
    assert_eq!(replayed.body["id"], created.body["id"]);
    let racers = futures_util::future::join_all(
        (0..4).map(|_| compose(&app, &acme.alice_token, &acme.alice_session, "once-2")),
    )
    .await;
    let ids: std::collections::HashSet<String> = racers
        .iter()
        .map(|reply| reply.body["id"].as_str().expect("run id").to_owned())
        .collect();
    assert_eq!(ids.len(), 1, "concurrent completion delivered one run");
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "context_runs").await,
        2
    );

    let before = usage_units(&acme.state, acme.tenant_id, "session_event_bytes").await;
    let denied = append(&app, &acme.mallory_token, &acme.alice_session, &["m1"]).await;
    assert!(denied.status.is_client_error(), "{}", denied.body);
    let ended = send(
        &app,
        "POST",
        &format!("/v1/sessions/{}/end", acme.bob_session),
        Some(&acme.bob_token),
        None,
        Some(json!({"status": "ended"})),
        None,
    )
    .await;
    assert_eq!(ended.status, StatusCode::OK, "{}", ended.body);
    let closed = append(&app, &acme.bob_token, &acme.bob_session, &["late"]).await;
    assert_eq!(closed.status, StatusCode::CONFLICT, "{}", closed.body);
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "session_event_bytes").await,
        before,
        "denied and refused work is not usage"
    );
}

async fn set_budget(app: &Router, token: &str, meter: &str, body: Value) -> Reply {
    send(
        app,
        "PUT",
        &format!("/v1/usage/budgets/{meter}"),
        Some(token),
        None,
        Some(body),
        None,
    )
    .await
}

#[tokio::test]
async fn a_spent_daily_budget_refuses_and_rolls_back_the_work() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let (app, _) = limited(&acme.state, Limits::default());
    let created = compose(&app, &acme.alice_token, &acme.alice_session, "budget-1").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let narrowed = set_budget(
        &app,
        &acme.alice_token,
        "context_runs",
        json!({"daily_limit": 2, "reason": "incident 7: runaway agent", "expires_in_secs": 3600}),
    )
    .await;
    assert_eq!(narrowed.status, StatusCode::OK, "{}", narrowed.body);
    let within = compose(&app, &acme.alice_token, &acme.alice_session, "budget-2").await;
    assert_eq!(within.status, StatusCode::CREATED, "{}", within.body);
    let spent = compose(&app, &acme.alice_token, &acme.alice_session, "budget-3").await;
    assert_limited(&spent);
    assert_eq!(spent.headers["retry-after"], "60");
    assert!(
        spent.body["message"]
            .as_str()
            .expect("message")
            .contains("context_runs")
    );
    assert_eq!(
        count(&acme.state, acme.tenant_id, "session_context_runs").await,
        2
    );
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "context_runs").await,
        2
    );

    let paused = set_budget(
        &app,
        &acme.alice_token,
        "session_event_bytes",
        json!({"daily_limit": 0, "reason": "pause ingestion", "expires_in_secs": 600}),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{}", paused.body);
    let events_before = count(&acme.state, acme.tenant_id, "session_events").await;
    assert_limited(&append(&app, &acme.alice_token, &acme.alice_session, &["p1"]).await);
    assert_eq!(
        count(&acme.state, acme.tenant_id, "session_events").await,
        events_before
    );
    let cleared = send(
        &app,
        "DELETE",
        "/v1/usage/budgets/session_event_bytes",
        Some(&acme.alice_token),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT, "{}", cleared.body);
    let resumed = append(&app, &acme.alice_token, &acme.alice_session, &["p1"]).await;
    assert_eq!(resumed.body["appended"], 1, "{}", resumed.body);
}

#[tokio::test]
async fn usage_and_budgets_are_cedar_governed_tenant_scoped_and_audited() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let Some(globex) = admitted_world().await else {
        return;
    };
    let (app, _) = limited(&acme.state, Limits::default());
    append(&app, &acme.alice_token, &acme.alice_session, &["v1"]).await;
    compose(&app, &acme.alice_token, &acme.alice_session, "view-1").await;

    let view = send(
        &app,
        "GET",
        "/v1/usage?days=2",
        Some(&acme.alice_token),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.body);
    assert_eq!(view.body["enforced"], true);
    let meters = view.body["meters"].as_array().expect("meters");
    let runs = meters
        .iter()
        .find(|meter| meter["meter"] == "context_runs")
        .expect("context_runs");
    assert_eq!(runs["used_today"], 1);
    assert_eq!(runs["daily_limit"], runs["ceiling"]);
    let foreign = send(
        &app,
        "GET",
        "/v1/usage",
        Some(&globex.alice_token),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(foreign.status, StatusCode::OK);
    assert!(
        foreign.body["days"].as_array().expect("days").is_empty(),
        "another tenant sees none of this usage: {}",
        foreign.body
    );

    for (token, method, path, body) in [
        (&acme.bob_token, "GET", "/v1/usage", None),
        (&acme.mallory_token, "GET", "/v1/usage", None),
        (
            &acme.bob_token,
            "PUT",
            "/v1/usage/budgets/context_runs",
            Some(json!({"daily_limit": 0, "reason": "x", "expires_in_secs": 600})),
        ),
        (
            &acme.bob_token,
            "DELETE",
            "/v1/usage/budgets/context_runs",
            None,
        ),
    ] {
        let reply = send(&app, method, path, Some(token), None, body, None).await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            reply.body
        );
    }
    for (meter, body, expected) in [
        (
            "context_runs",
            json!({"daily_limit": 5, "reason": "x", "expires_in_secs": 7_776_001}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "context_runs",
            json!({"daily_limit": -1, "reason": "x", "expires_in_secs": 600}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "context_runs",
            json!({"daily_limit": 5, "reason": " ", "expires_in_secs": 600}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "tokens",
            json!({"daily_limit": 5, "reason": "x", "expires_in_secs": 600}),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let reply = set_budget(&app, &acme.alice_token, meter, body).await;
        assert_eq!(reply.status, expected, "{}", reply.body);
    }

    let globex_pause = set_budget(
        &app,
        &globex.alice_token,
        "context_runs",
        json!({"daily_limit": 0, "reason": "globex drill", "expires_in_secs": 600}),
    )
    .await;
    assert_eq!(globex_pause.status, StatusCode::OK);
    let acme_run = compose(&app, &acme.alice_token, &acme.alice_session, "view-2").await;
    assert_eq!(
        acme_run.status,
        StatusCode::CREATED,
        "a foreign override never applies here: {}",
        acme_run.body
    );
    let set = set_budget(
        &app,
        &acme.alice_token,
        "context_runs",
        json!({"daily_limit": 9, "reason": "change 12: rollout guard", "expires_in_secs": 900}),
    )
    .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.body);
    assert_eq!(set.body["set_by"], ALICE);

    let audit = audit_events(&acme).await;
    let changes: Vec<_> = audit
        .iter()
        .filter(|event| event.action == "tenant.usage.budget_changed")
        .collect();
    assert_eq!(changes.len(), 1, "one audited change in this tenant");
    let payload = &changes[0].payload;
    assert_eq!(payload["meter"], "context_runs");
    assert_eq!(payload["daily_limit"], 9);
    assert_eq!(payload["reason"], "change 12: rollout guard");
    assert!(
        audit
            .iter()
            .any(|event| event.action == "authz.decision" && event.outcome == "deny"),
        "the refused budget change is a recorded PDP denial"
    );
    let chain = serde_json::to_string(&audit.iter().map(|e| &e.payload).collect::<Vec<_>>())
        .expect("audit payloads");
    assert!(
        !chain.contains(PAYLOAD_TEXT),
        "no event content in the audit chain"
    );
    assert!(
        !chain.contains("globex drill"),
        "no foreign override in this chain"
    );
}

#[tokio::test]
async fn observe_mode_admits_and_records_usage_beyond_every_budget() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let mut limits = compose_budget(1, 1);
    limits.mode = synveda_gateway::limits::Mode::Observe;
    let (app, _) = limited(&acme.state, limits);
    let paused = set_budget(
        &app,
        &acme.alice_token,
        "context_runs",
        json!({"daily_limit": 0, "reason": "observe drill", "expires_in_secs": 600}),
    )
    .await;
    assert_eq!(paused.status, StatusCode::OK, "{}", paused.body);
    for key in ["observe-1", "observe-2", "observe-3"] {
        let reply = compose(&app, &acme.alice_token, &acme.alice_session, key).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    }
    assert_eq!(
        usage_units(&acme.state, acme.tenant_id, "context_runs").await,
        3
    );
    let view = send(
        &app,
        "GET",
        "/v1/usage",
        Some(&acme.alice_token),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(view.body["enforced"], false);
}

#[tokio::test]
async fn a_restart_resets_hot_budgets_and_keeps_durable_usage_and_overrides() {
    let _guard = serial().await;
    let Some(acme) = admitted_world().await else {
        return;
    };
    let (before, _) = limited(&acme.state, compose_budget(1, 2));
    let first = compose(&before, &acme.alice_token, &acme.alice_session, "restart-1").await;
    assert_eq!(first.status, StatusCode::CREATED);
    assert_limited(&compose(&before, &acme.alice_token, &acme.alice_session, "restart-2").await);
    let narrowed = set_budget(
        &before,
        &acme.alice_token,
        "session_event_bytes",
        json!({"daily_limit": 4096, "reason": "kept across restart", "expires_in_secs": 600}),
    )
    .await;
    assert_eq!(narrowed.status, StatusCode::OK);

    let (after, _) = limited(&acme.state, compose_budget(1, 2));
    let second = compose(&after, &acme.alice_token, &acme.alice_session, "restart-2").await;
    assert_eq!(
        second.status,
        StatusCode::CREATED,
        "hot budgets reset: {}",
        second.body
    );
    let view = send(
        &after,
        "GET",
        "/v1/usage",
        Some(&acme.alice_token),
        None,
        None,
        None,
    )
    .await;
    let meters = view.body["meters"].as_array().expect("meters");
    let runs = meters
        .iter()
        .find(|m| m["meter"] == "context_runs")
        .expect("runs");
    assert_eq!(runs["used_today"], 2, "durable usage survives");
    let bytes = meters
        .iter()
        .find(|m| m["meter"] == "session_event_bytes")
        .expect("bytes");
    assert_eq!(bytes["daily_limit"], 4096, "the override survives");
}

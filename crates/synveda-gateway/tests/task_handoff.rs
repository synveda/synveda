//! Exact working state and cross-interface handoff acceptance.
#[path = "support/context_world.rs"]
mod context_world;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use context_world::*;

async fn open(world: &World, key: &str, client: &str) -> String {
    let (status, body) = call(&world.app, "POST", "/v1/sessions", &world.alice_token, Some(key),
        Some(json!({"workspace_id":world.workspace_id,"project_id":world.project_id,"client_name":client}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_owned()
}
async fn append(
    world: &World,
    session: &str,
    key: &str,
    event_type: &str,
    payload: Value,
) -> Value {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let (status, body) = call(&world.app,"POST",&format!("/v1/sessions/{session}/events"),&world.alice_token,Some(key),
        Some(json!({"events":[{"client_event_id":key,"event_type":event_type,"occurred_at":"2026-10-14T00:00:00Z","payload":payload}]}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["events"][0]["event"].clone()
}
fn statement(event: &Value, text: &str) -> Value {
    json!({"kind":"objective","evidence":{"source_kind":"session_event","event_id":event["id"],
        "payload_hash":event["payload_hash"],"field":"text","span":{"start":0,"end":text.len()}}})
}
fn state_body(key: &str, statements: Vec<Value>, previous: Option<&Value>) -> Value {
    json!({"schema_version":1,"client_event_id":key,"statements":statements,
        "expected_previous":previous.map(|e|json!({"event_id":e["id"],"payload_hash":e["payload_hash"]}))})
}
async fn derive(world: &World, session: &str, body: Value) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/task-state"),
        &world.alice_token,
        None,
        Some(body),
    )
    .await
}
async fn checkpoint(world: &World, session: &str, key: &str) -> Value {
    append(
        world,
        session,
        key,
        "session.compaction_boundary",
        json!({"schema_version":1,"local_high_sequence":1,
        "expected_client_event_ids":[],"expected_ids_truncated":false}),
    )
    .await;
    let (status, timeline) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{session}/timeline"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let id = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|e| e["event_type"] == "session.checkpoint")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let (status, event) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{session}/events/{id}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{event}");
    event
}
fn handoff(source: &str, snapshot: &Value) -> Value {
    json!({"source_session_id":source,"snapshot_event_id":snapshot["id"],"payload_hash":snapshot["payload_hash"]})
}
fn delivered_statement_texts(run: &Value) -> Vec<String> {
    run["rendered"]
        .as_str()
        .unwrap()
        .lines()
        .filter_map(|line| {
            let value: Value = serde_json::from_str(line.strip_prefix("- ")?).ok()?;
            (value["kind"] == "task_state_statement")
                .then(|| value["statement"]["text"].as_str().unwrap().to_owned())
        })
        .collect()
}
async fn deliver(world: &World, receiver: &str, key: &str, body: Value) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{receiver}/context-runs"),
        &world.alice_token,
        Some(key),
        Some(body),
    )
    .await
}
async fn remove(world: &World, session: &str, source: &Value, mode: &str) -> Value {
    let (status, result)=call(&world.app,"POST",&format!("/v1/sessions/{session}/events/{}/{mode}",source["id"].as_str().unwrap()),
        &world.alice_token,Some(&format!("{mode}-{}",source["id"].as_str().unwrap())),
        Some(json!({"expected_payload_hash":source["payload_hash"],"reason":"Retire fictional handoff evidence"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["outcome"], "applied", "{result}");
    result
}

#[tokio::test]
async fn exact_spans_retry_and_serial_replacement_do_not_accept_invented_proof() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let text = "Build the λ retry guard.";
    let source = append(
        &world,
        &world.alice_session,
        "exact-source",
        "message.user",
        json!({"text":text}),
    )
    .await;
    let request = state_body("first-state", vec![statement(&source, text)], None);
    let (status, first) = derive(&world, &world.alice_session, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["event"]["payload"]["statements"][0]["text"], text);
    assert_eq!(
        first["event"]["payload"]["statements"][0]["evidence_class"],
        "user_declared"
    );
    let (status, retry) = derive(&world, &world.alice_session, request).await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["outcome"], "duplicate");
    assert_eq!(retry["event"]["id"], first["event"]["id"]);
    let a = state_body(
        "race-a",
        vec![statement(&source, text)],
        Some(&first["event"]),
    );
    let b = state_body(
        "race-b",
        vec![statement(&source, text)],
        Some(&first["event"]),
    );
    let (a, b) = tokio::join!(
        derive(&world, &world.alice_session, a),
        derive(&world, &world.alice_session, b)
    );
    assert!(
        matches!(
            (a.0, b.0),
            (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)
        ),
        "{a:?} {b:?}"
    );
    let latest = if a.0 == StatusCode::OK { a.1 } else { b.1 };
    for mutate in ["hash", "utf8", "text", "verified"] {
        let mut invalid = state_body(
            mutate,
            vec![statement(&source, text)],
            Some(&latest["event"]),
        );
        match mutate {
            "hash" => invalid["statements"][0]["evidence"]["payload_hash"] = json!("0".repeat(64)),
            "utf8" => invalid["statements"][0]["evidence"]["span"] = json!({"start":11,"end":12}),
            "text" => invalid["statements"][0]["text"] = json!("Invented objective"),
            _ => invalid["statements"][0]["evidence_class"] = json!("host_verified"),
        }
        let (status, error) = derive(&world, &world.alice_session, invalid).await;
        assert!(
            matches!(
                status,
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::CONFLICT
            ),
            "{mutate}: {error}"
        );
    }
    let (status, error) = derive(
        &world,
        &world.bob_session,
        state_body("foreign-source", vec![statement(&source, text)], None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    let (status,error)=call(&world.app,"POST",&format!("/v1/sessions/{}/events",world.alice_session),&world.alice_token,None,
        Some(json!({"events":[{"client_event_id":"forged-state","event_type":"session.task_state","occurred_at":"2026-10-14T00:00:00Z","payload":{}}]}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
}

#[tokio::test]
async fn latest_withdrawn_state_never_resurrects_an_older_objective_on_native_restart() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    for client in ["claude-code", "codex"] {
        let session = open(&world, &format!("native-{client}"), client).await;
        let old = "The obsolete objective is amber.";
        let new = "The replacement objective is indigo.";
        let a = append(
            &world,
            &session,
            "native-a",
            "message.user",
            json!({"text":old}),
        )
        .await;
        let (status, first) = derive(
            &world,
            &session,
            state_body("native-first", vec![statement(&a, old)], None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        checkpoint(&world, &session, "native-boundary-a").await;
        let b = append(
            &world,
            &session,
            "native-b",
            "message.user",
            json!({"text":new}),
        )
        .await;
        let (status, second) = derive(
            &world,
            &session,
            state_body(
                "native-second",
                vec![statement(&b, new)],
                Some(&first["event"]),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second}");
        let latest = checkpoint(&world, &session, "native-boundary-b").await;
        assert_eq!(latest["payload"]["schema_version"], 2, "{latest}");
        let (status, before) = deliver(
            &world,
            &session,
            &format!("native-before-{client}"),
            json!({"restart":true,"budget_tokens":2400}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{before}");
        assert!(
            before["rendered"].as_str().unwrap().contains(new),
            "{before}"
        );
        assert!(
            !before["rendered"].as_str().unwrap().contains(old),
            "{before}"
        );
        remove(&world, &session, &b, "withdraw").await;
        let (status, after) = deliver(
            &world,
            &session,
            &format!("native-after-{client}"),
            json!({"restart":true,"budget_tokens":2400}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{after}");
        assert!(
            !after.to_string().contains(old) && !after.to_string().contains(new),
            "{after}"
        );
        let (status, replay) = deliver(
            &world,
            &session,
            &format!("native-before-{client}"),
            json!({"restart":true,"budget_tokens":2400}),
        )
        .await;
        assert!(
            matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
            "{replay}"
        );
    }
}

#[tokio::test]
async fn cross_interface_handoff_is_atomic_historical_and_checkout_sensitive() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let source = open(&world, "handoff-source", "claude-code").await;
    let receiver = open(&world, "handoff-receiver", "codex").await;
    let text = "Deployment was attempted; the external outcome remains unknown.";
    let event=append(&world,&source,"handoff-report","tool.result",json!({"text":text,"context":{"checkout":{
        "ref":"worktree-a","branch":"main","commit":"a".repeat(40),"tree":"b".repeat(40),"dirty":false}}})).await;
    let mut reported = statement(&event, text);
    reported["kind"] = json!("work");
    reported["declared_outcome"] = json!("unknown");
    let mut body = state_body("handoff-state", vec![reported], None);
    body["checkout"] = json!({"event_id":event["id"],"payload_hash":event["payload_hash"]});
    let (status, state) = derive(&world, &source, body).await;
    assert_eq!(status, StatusCode::OK, "{state}");
    let snapshot = checkpoint(&world, &source, "handoff-boundary").await;
    let request = json!({"budget_tokens":2600,"handoff":handoff(&source,&snapshot),"checkout_observation":{
        "ref":"worktree-b","observed_at":"2026-10-14T00:00:01Z","branch":"fix/retry","commit":"c".repeat(40),"tree":"d".repeat(40),"dirty":false}});
    let (status, run) = deliver(&world, &receiver, "handoff-run", request.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let rendered = run["rendered"].as_str().unwrap();
    assert!(
        rendered.contains(text)
            && rendered.contains("client_reported")
            && rendered.contains("changed_revalidation_required"),
        "{run}"
    );
    assert!(
        rendered.contains("unknown outcome remains unknown"),
        "{run}"
    );
    assert!(
        run["restart_checkpoint_event_id"].is_null(),
        "foreign snapshot must remain manifest-only: {run}"
    );
    let (status, native) = deliver(
        &world,
        &source,
        "fresh-native-checkout",
        json!({"restart":true,"budget_tokens":2600,
        "checkout_observation":request["checkout_observation"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{native}");
    assert!(
        native["rendered"]
            .as_str()
            .unwrap()
            .contains("changed_revalidation_required"),
        "{native}"
    );
    let mut tight = request.clone();
    tight["budget_tokens"] = json!(64);
    let (status, error) = deliver(&world, &receiver, "handoff-tight", tight).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert!(!error.to_string().contains(text));
    remove(&world, &source, &event, "forget").await;
    for key in ["handoff-run", "handoff-after"] {
        let (status, error) = deliver(&world, &receiver, key, request.clone()).await;
        assert!(
            matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
            "{error}"
        );
        assert!(!error.to_string().contains(text));
    }
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "handoff-worker",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, history) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{source}/events/{}",
            state["event"]["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{history}");
    assert!(!history.to_string().contains(text));
}

#[tokio::test]
async fn copied_knowledge_respects_classification_and_item_forget_without_erasing_primary_events() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let source = open(&world, "knowledge-source", "claude-code").await;
    let private = "The confidential fictional metric excludes refunded trials.";
    let mut body = content("Private metric", private, None);
    body["sensitivity"] = json!("confidential");
    let (item, revision) = create_knowledge_with_content(
        &world,
        "handoff-knowledge",
        world.project_scope,
        Some(&world.project_id),
        None,
        body,
    )
    .await;
    let (status, detail) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let evidence = json!({"kind":"constraint","evidence":{"source_kind":"knowledge_revision","item_id":item,"revision_id":revision,
        "content_hash":detail["current_revision"]["content_hash"],"span":{"start":0,"end":private.len()}}});
    let primary = append(
        &world,
        &source,
        "independent-primary",
        "message.user",
        json!({"text":"Independent current instructions remain valid."}),
    )
    .await;
    let (status, state) = derive(
        &world,
        &source,
        state_body("private-state", vec![evidence], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{state}");
    let snapshot = checkpoint(&world, &source, "private-boundary").await;
    let request = json!({"budget_tokens":2600,"handoff":handoff(&source,&snapshot)});
    let (status, run) = deliver(
        &world,
        &world.bob_session,
        "private-delivery",
        request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert!(run["rendered"].as_str().unwrap().contains(private));
    let mut narrow = request.clone();
    narrow["max_sensitivity"] = json!("public");
    let (status, error) = deliver(&world, &world.bob_session, "narrow-private", narrow).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert!(!error.to_string().contains(private));
    let (status, native) = deliver(
        &world,
        &source,
        "narrow-native",
        json!({"restart":true,"budget_tokens":2600,"max_sensitivity":"public"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{native}");
    assert!(!native.to_string().contains(private));
    let (status,forgotten)=call(&world.app,"DELETE",&format!("/v1/knowledge/{item}"),&world.alice_token,Some("forget-copy"),
        Some(json!({"mode":"forget","expected_revision_id":revision,"reason":"Remove fictional metric"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied");
    for id in [&state["event"]["id"], &snapshot["id"]] {
        let (status, error) = call(
            &world.app,
            "GET",
            &format!("/v1/sessions/{source}/events/{}", id.as_str().unwrap()),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
        assert!(!error.to_string().contains(private));
    }
    let (status, retained) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{source}/events/{}",
            primary["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retained}");
    let (status, error) = deliver(&world, &world.bob_session, "private-delivery", request).await;
    assert!(
        matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
        "{error}"
    );
    assert!(!error.to_string().contains(private));
}

#[tokio::test]
async fn retained_v1_handoff_inherits_every_delivered_chain_root() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let source = open(&world, "v1-source", "claude-code").await;
    let first = "The first checkpoint carries a cobalt marker.";
    let a = append(
        &world,
        &source,
        "v1-a",
        "message.user",
        json!({"text":first}),
    )
    .await;
    checkpoint(&world, &source, "v1-boundary-a").await;
    append(
        &world,
        &source,
        "v1-b",
        "message.user",
        json!({"text":"The second checkpoint carries a jade marker."}),
    )
    .await;
    let latest = checkpoint(&world, &source, "v1-boundary-b").await;
    assert_eq!(latest["payload"]["schema_version"], 1, "{latest}");
    let request = json!({"budget_tokens":3000,"handoff":handoff(&source,&latest)});
    let (status, run) = deliver(&world, &world.bob_session, "v1-delivery", request.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert!(run["rendered"].as_str().unwrap().contains(first), "{run}");
    remove(&world, &source, &a, "forget").await;
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "v1-handoff-worker",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, error) = deliver(&world, &world.bob_session, "v1-delivery", request).await;
    assert!(
        matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
        "{error}"
    );
    assert!(!error.to_string().contains(first));
}

#[tokio::test]
async fn handoff_replay_and_detail_reauthorize_sources_in_every_trace_mode() {
    let _guard = serial().await;
    for mode in TraceRetentionMode::ALL {
        let Some(world) = admitted_world().await else {
            return;
        };
        set_trace_mode(&world, mode).await;
        let text = "The fictional public review requires a copper marker.";
        let source = append(
            &world,
            &world.alice_session,
            "trace-source",
            "message.user",
            json!({"text":text}),
        )
        .await;
        let (status, state) = derive(
            &world,
            &world.alice_session,
            state_body("trace-state", vec![statement(&source, text)], None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{state}");
        let request =
            json!({"budget_tokens":2400,"handoff":handoff(&world.alice_session,&state["event"])});
        let (status, run) =
            deliver(&world, &world.bob_session, "trace-handoff", request.clone()).await;
        assert_eq!(status, StatusCode::CREATED, "{mode:?}: {run}");
        remove(&world, &world.alice_session, &source, "withdraw").await;
        let (status, error) = deliver(&world, &world.bob_session, "trace-handoff", request).await;
        assert!(
            matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
            "{mode:?}: {error}"
        );
        assert!(!error.to_string().contains(text));
        let (status, detail) = call(
            &world.app,
            "GET",
            &format!("/v1/context-runs/{}", run["id"].as_str().unwrap()),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert!(
            matches!(status, StatusCode::OK | StatusCode::NOT_FOUND),
            "{detail}"
        );
        assert!(!detail.to_string().contains(text), "{mode:?}: {detail}");
    }
}

#[tokio::test]
async fn current_session_policy_and_foreign_tenants_cannot_launder_handoff_sources() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let text = "The governed fictional source contains a violet marker.";
    let source = append(
        &world,
        &world.alice_session,
        "policy-source",
        "message.user",
        json!({"text":text}),
    )
    .await;
    let (status, state) = derive(
        &world,
        &world.alice_session,
        state_body("policy-state", vec![statement(&source, text)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{state}");
    let request =
        json!({"budget_tokens":2400,"handoff":handoff(&world.alice_session,&state["event"])});
    let other = admitted_world().await.unwrap();
    let foreign = other.alice_token;
    let endpoint = format!("/v1/sessions/{}/task-state", world.alice_session);
    let (status, error) = call(
        &world.app,
        "POST",
        &endpoint,
        &foreign,
        None,
        Some(state_body(
            "foreign-state",
            vec![statement(&source, text)],
            None,
        )),
    )
    .await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{error}"
    );
    assert!(!error.to_string().contains(text));
    world.install_policy("handoff-source-deny",1,&format!("{BLANKET}\nforbid (principal, action == Synveda::Action::\"SessionRead\", resource);"),PackConfig::default()).await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "handoff-source-deny").await;
    tx.commit().await.unwrap();
    let (status, error) = deliver(&world, &world.bob_session, "policy-handoff", request).await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{error}"
    );
    assert!(!error.to_string().contains(text));
}

#[tokio::test]
async fn replacement_before_one_boundary_never_restores_or_inherits_the_old_objective() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let session = open(&world, "single-window", "codex").await;
    let old = "The obsolete objective is to ship the violet prototype.";
    let new = "The current objective is to review the copper production guard.";
    let a = append(
        &world,
        &session,
        "window-a",
        "message.user",
        json!({"text":old}),
    )
    .await;
    let (status, first) = derive(
        &world,
        &session,
        state_body("window-first", vec![statement(&a, old)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let b = append(
        &world,
        &session,
        "window-b",
        "message.user",
        json!({"text":new}),
    )
    .await;
    let (status, second) = derive(
        &world,
        &session,
        state_body(
            "window-second",
            vec![statement(&b, new)],
            Some(&first["event"]),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let snapshot = checkpoint(&world, &session, "window-boundary").await;
    assert!(snapshot["payload"]["previous_checkpoint_event_id"].is_null());
    remove(&world, &session, &a, "withdraw").await;
    for (receiver, key, request) in [
        (
            session.as_str(),
            "window-native",
            json!({"restart":true,"budget_tokens":2600}),
        ),
        (
            world.bob_session.as_str(),
            "window-handoff",
            json!({"handoff":handoff(&session,&snapshot),"budget_tokens":2600}),
        ),
    ] {
        let (status, run) = deliver(&world, receiver, key, request).await;
        assert_eq!(status, StatusCode::CREATED, "{run}");
        assert!(
            delivered_statement_texts(&run)
                .iter()
                .any(|text| text == new),
            "{run}"
        );
        assert!(!run.to_string().contains(old), "{run}");
    }
}

#[tokio::test]
async fn scoped_retention_allows_an_independent_new_head_after_disposing_old_state() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let old = "The retained old objective is a fictional violet marker.";
    let source = append(
        &world,
        &world.alice_session,
        "retained-source",
        "message.user",
        json!({"text":old}),
    )
    .await;
    let (status, state) = derive(
        &world,
        &world.alice_session,
        state_body("retained-state", vec![statement(&source, old)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{state}");
    let owner = tenant_fixture::migrator_pool(&world.state.pool).await;
    let mut tx = rls::begin_tenant_tx(&owner, world.tenant_id).await.unwrap();
    sqlx::raw_sql("set local synveda.retention_purge='on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let id: uuid::Uuid = state["event"]["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        sqlx::query("delete from session_events where tenant_id=$1 and id=$2")
            .bind(world.tenant_id.as_uuid())
            .bind(id)
            .execute(&mut *tx)
            .await
            .unwrap()
            .rows_affected(),
        1
    );
    tx.commit().await.unwrap();
    owner.close().await;
    let fresh = "The independent fresh objective is the copper guard.";
    let source = append(
        &world,
        &world.alice_session,
        "fresh-after-retention",
        "message.user",
        json!({"text":fresh}),
    )
    .await;
    let (status, new) = derive(
        &world,
        &world.alice_session,
        state_body("fresh-state", vec![statement(&source, fresh)], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{new}");
    assert_ne!(new["event"]["id"], state["event"]["id"]);
    assert_eq!(new["event"]["payload"]["statements"][0]["text"], fresh);
}

#[tokio::test]
async fn actual_import_artifact_handoff_is_invalidated_and_cleaned_without_synthetic_events() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let document = "---\ntype: Procedure\ntitle: Iris retries\ndescription: Fictional approved retry procedure.\n---\n\nThe fictional Iris retry cap is 3 attempts.\n";
    let (status,plan)=call(&world.app,"POST",&format!("/v1/projects/{}/okf/imports",world.project_id),&world.alice_token,Some("handoff-import"),Some(json!({
        "source_kind":"git","source_locator":"https://example.test/iris.git","source_revision":"fixture-v1","encoding":"entries","entries":[
            {"logical_path":"index.md","kind":"file","content_base64":STANDARD.encode("---\nokf_version: \"0.2\"\n---\n\n# Fictional Iris\n")},
            {"logical_path":"procedures/retries.md","kind":"file","content_base64":STANDARD.encode(document)}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    let artifact = plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["logical_path"] == "procedures/retries.md")
        .unwrap();
    let body = artifact["body_markdown"].as_str().unwrap();
    let entry = json!({"kind":"constraint","evidence":{"source_kind":"import_artifact","artifact_id":artifact["id"],
        "content_hash":artifact["content_hash"],"span":{"start":0,"end":body.len()}}});
    let (status, state) = derive(
        &world,
        &world.alice_session,
        state_body("artifact-state", vec![entry], None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{state}");
    let request =
        json!({"budget_tokens":2600,"handoff":handoff(&world.alice_session,&state["event"])});
    let (status, run) = deliver(
        &world,
        &world.bob_session,
        "artifact-handoff",
        request.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert!(
        delivered_statement_texts(&run)
            .iter()
            .any(|text| text == body),
        "{run}"
    );
    let (status,removed)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/forget",plan["job"]["id"].as_str().unwrap(),artifact["id"].as_str().unwrap()),
        &world.alice_token,Some("artifact-handoff-forget"),Some(json!({"expected_content_hash":artifact["content_hash"],"reason":"Remove fictional artifact evidence"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "artifact-handoff-worker",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, operation) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/operations/{}",
            removed["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["cleanup_state"], "succeeded", "{operation}");
    let (status, error) = deliver(&world, &world.bob_session, "artifact-handoff", request).await;
    assert!(
        matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
        "{error}"
    );
    assert!(!error.to_string().contains(body));
    let (status, error) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{}/events/{}",
            world.alice_session,
            state["event"]["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
}

//! Exact source removal through public APIs and ordinary-role PostgreSQL.
#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;

async fn append_source(world: &World, key: &str, text: &str) -> Value {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let (status,body)=call(&world.app,"POST",&format!("/v1/sessions/{}/events",world.alice_session),&world.alice_token,Some(key),
      Some(json!({"events":[{"client_event_id":key,"event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":text}}]}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["appended"], 1);
    assert_eq!(body["events"][0]["outcome"], "appended");
    body["events"][0]["event"].clone()
}

async fn source_knowledge(world: &World, key: &str, event: &Value, text: &str) -> (String, String) {
    let (status,created)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some(key),Some(json!({
      "scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"convention","origin":"observed",
      "content":content("Iris retry policy",text,None),"sources":[{"scope_id":world.project_scope,"source_type":"session_event","session_event_id":event["id"],"metadata":{}}]
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    (
        created["knowledge_item_id"].as_str().unwrap().to_owned(),
        created["revision_id"].as_str().unwrap().to_owned(),
    )
}

async fn remove_source(world: &World, event: &Value, mode: &str, key: &str) -> Value {
    let (status,removed)=call(&world.app,"POST",&format!("/v1/sessions/{}/events/{}/{mode}",world.alice_session,event["id"].as_str().unwrap()),
      &world.alice_token,Some(key),Some(json!({"expected_payload_hash":event["payload_hash"],"reason":"Retire this fictional source"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    assert_eq!(removed["outcome"], "applied", "{removed}");
    removed
}

#[tokio::test]
async fn withdrawal_blocks_raw_history_delivery_and_unchanged_append() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let secret = "The fictional Iris retry cap is 3 attempts.";
    let event = append_source(&world, "source-withdraw-event", secret).await;
    let (item, revision) =
        source_knowledge(&world, "source-withdraw-knowledge", &event, secret).await;
    let path = format!("/v1/sessions/{}/context-runs", world.alice_session);
    let request = json!({"query":"Iris retry","budget_tokens":1800,"required_knowledge_revisions":[{"item_id":item,"revision_id":revision}]});
    let (status, receipt) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("source-receipt"),
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{receipt}");
    assert!(receipt["rendered"].as_str().unwrap().contains(secret));
    remove_source(&world, &event, "withdraw", "source-withdraw").await;
    for endpoint in [
        format!("/v1/knowledge/{item}"),
        format!(
            "/v1/sessions/{}/events/{}",
            world.alice_session,
            event["id"].as_str().unwrap()
        ),
    ] {
        let (status, body) =
            call(&world.app, "GET", &endpoint, &world.alice_token, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert!(!body.to_string().contains(secret));
    }
    let (history_status, history) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}/history"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(history_status, StatusCode::OK, "{history}");
    assert_eq!(history["revisions"], json!([]));
    let (status, replay) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("source-receipt"),
        Some(request),
    )
    .await;
    assert!(
        matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
        "{replay}"
    );
    assert!(!replay.to_string().contains(secret));
    let (status,duplicate)=call(&world.app,"POST",&format!("/v1/sessions/{}/events",world.alice_session),&world.alice_token,Some("new-append-key"),
      Some(json!({"events":[{"client_event_id":"source-withdraw-event","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":secret}}]}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    assert!(!duplicate.to_string().contains(secret));
    // One statement checks the whole batch: a withdrawn source anywhere in it,
    // by client id or by unchanged payload, refuses every event in it.
    let fresh = json!({"client_event_id":"source-withdraw-fresh","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":"A fictional unrelated note."}});
    for (key, suppressed) in [
        (
            "mixed-by-client-id",
            json!({"client_event_id":"source-withdraw-event","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":"Changed fictional text."}}),
        ),
        (
            "mixed-by-payload",
            json!({"client_event_id":"source-withdraw-renamed","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":secret}}),
        ),
    ] {
        let (status, mixed) = call(
            &world.app,
            "POST",
            &format!("/v1/sessions/{}/events", world.alice_session),
            &world.alice_token,
            Some(key),
            Some(json!({"events":[fresh.clone(),suppressed]})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{key}: {mixed}");
        // The suppression refusal, not the later duplicate-identity conflict.
        assert_eq!(
            mixed["message"], "Source evidence is unavailable under current source state",
            "{key}: {mixed}"
        );
        assert!(!mixed.to_string().contains(secret));
    }
    let (status, alone) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/events", world.alice_session),
        &world.alice_token,
        Some("fresh-alone"),
        Some(json!({"events":[fresh]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{alone}");
    assert_eq!(
        alone["events"][0]["outcome"], "appended",
        "a refused batch wrote nothing"
    );
    let (status, view) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{}/events/{}/source-control",
            world.alice_session,
            event["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["mode"], "withdraw");
    assert_eq!(view["cleanup_state"], "blocked");
}

#[tokio::test]
async fn source_forget_preserves_an_independently_edited_head_and_its_delivery() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let old = "The fictional Iris retry cap is 3 attempts.";
    let event = append_source(&world, "source-forget-event", old).await;
    let (item, revision) = source_knowledge(&world, "source-forget-knowledge", &event, old).await;
    remove_source(&world, &event, "withdraw", "source-forget-withdraw").await;
    let clean = "The independent approved Iris retry cap is 5 attempts.";
    let (status,edited)=call(&world.app,"PATCH",&format!("/v1/knowledge/{item}"),&world.alice_token,Some("source-clean-edit"),Some(json!({
      "expected_revision_id":revision,"content":content("Independent Iris retry policy",clean,None),
      "sources":[{"scope_id":world.project_scope,"source_type":"manual","metadata":{"fixture":"independent"}}]
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{edited}");
    assert_eq!(edited["outcome"], "applied", "{edited}");
    let path = format!("/v1/sessions/{}/context-runs", world.alice_session);
    let request = json!({"query":"Iris retry","budget_tokens":1800,"required_knowledge_revisions":[{"item_id":item,"revision_id":edited["revision_id"]}]});
    let (status, receipt) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("clean-source-receipt"),
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{receipt}");
    assert!(receipt["rendered"].as_str().unwrap().contains(clean));
    remove_source(&world, &event, "forget", "source-forget").await;
    for _ in 0..16 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "source-test-worker",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, view) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{}/events/{}/source-control",
            world.alice_session,
            event["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["cleanup_state"], "succeeded", "{view}");
    let (status, current) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert!(current.to_string().contains(clean));
    assert!(!current.to_string().contains(old));
    let (status, replay) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("clean-source-receipt"),
        Some(request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, receipt);
}

#[tokio::test]
async fn removal_boundaries_and_exact_operation_receipts_are_content_free() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "receipt-source",
        "Fictional secret phrase to retire",
    )
    .await;
    let endpoint = format!(
        "/v1/sessions/{}/events/{}/withdraw",
        world.alice_session,
        event["id"].as_str().unwrap()
    );
    for (index, body) in [
        json!({"expected_payload_hash":42,"reason":"retire"}),
        json!({"expected_payload_hash":"bad","reason":"retire"}),
        json!({"expected_payload_hash":event["payload_hash"],"reason":"retire","unknown":true}),
    ]
    .into_iter()
    .enumerate()
    {
        let (status, response) = call(
            &world.app,
            "POST",
            &endpoint,
            &world.alice_token,
            Some(&format!("bad-source-{index}")),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    }
    let withdrawal = remove_source(&world, &event, "withdraw", "receipt-withdraw").await;
    let old_operation = withdrawal["operation_id"].as_str().unwrap();
    let (status, old) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{old_operation}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{old}");
    assert_eq!(old["kind"], "source_cleanup");
    assert_eq!(old["mode"], "withdraw");
    let forgetting = remove_source(&world, &event, "forget", "receipt-forget").await;
    let (status, again) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{old_operation}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again, old);
    let (status, new) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/operations/{}",
            forgetting["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{new}");
    assert_eq!(new["mode"], "forget");
    for response in [old, again, new] {
        let text = response.to_string();
        assert!(!text.contains("Fictional secret phrase"));
        assert!(!text.contains("actor_hash"));
        assert!(!text.contains("affected_count"));
    }
}

#[tokio::test]
async fn source_forget_and_then_item_forget_clear_suppressed_history() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(&world, "item-after-source", "Old fictional private source").await;
    let (item, revision) = source_knowledge(
        &world,
        "item-after-source-create",
        &event,
        "Old fictional private source",
    )
    .await;
    remove_source(&world, &event, "forget", "item-after-source-forget").await;
    for _ in 0..20 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "item-source-worker",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    let (status, result) = call(
        &world.app,
        "DELETE",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        Some("item-forget-after-source"),
        Some(json!({"mode":"forget","expected_revision_id":revision,"reason":"Remove the entire retired item"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["outcome"], "applied", "{result}");
    let (status, body) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn queued_saved_delivery_observes_the_policy_committed_before_admission() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(&world, "gate-source", "The fictional Iris retry cap is 3.").await;
    let (item, revision) = source_knowledge(
        &world,
        "gate-knowledge",
        &event,
        "The fictional Iris retry cap is 3.",
    )
    .await;
    let path = format!("/v1/sessions/{}/context-runs", world.alice_session);
    let request = json!({"query":"Iris","budget_tokens":1800,"required_knowledge_revisions":[{"item_id":item,"revision_id":revision}]});
    let (status, receipt) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("gate-receipt"),
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{receipt}");
    let mut writer = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let holder = tenant_fixture::backend_pid(&mut writer).await;
    let app = world.app.clone();
    let token = world.alice_token.clone();
    let pending = tokio::spawn(async move {
        call(
            &app,
            "POST",
            &path,
            &token,
            Some("gate-receipt"),
            Some(request),
        )
        .await
    });
    wait_for_admission_waiter(&world, holder).await;
    assert!(
        !pending.is_finished(),
        "saved read must wait for writer admission"
    );
    let source = format!(
        "{BLANKET}\nforbid (principal, action == Synveda::Action::\"KnowledgeRead\", resource);"
    );
    synveda_store::policy_packs::apply(
        &mut *writer,
        world.tenant_id,
        "default",
        &source,
        &PackConfig::default(),
    )
    .await
    .unwrap();
    configuration_support::bind_tenant_pack(&mut writer, world.tenant_id, "default").await;
    writer.commit().await.unwrap();
    let (status, body) = pending.await.unwrap();
    assert!(
        matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
        "{body}"
    );
    assert!(!body.to_string().contains("fictional Iris retry cap"));
}

#[tokio::test]
async fn source_writer_waits_for_an_admitted_reader() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "reader-gate-source",
        "Already admitted fictional evidence",
    )
    .await;
    let mut reader = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let holder = tenant_fixture::backend_pid(&mut reader).await;
    let app = world.app.clone();
    let token = world.alice_token.clone();
    let path = format!(
        "/v1/sessions/{}/events/{}/withdraw",
        world.alice_session,
        event["id"].as_str().unwrap()
    );
    let hash = event["payload_hash"].clone();
    let pending = tokio::spawn(async move {
        call(
            &app,
            "POST",
            &path,
            &token,
            Some("reader-gate-withdraw"),
            Some(json!({"expected_payload_hash":hash,"reason":"Retire source"})),
        )
        .await
    });
    wait_for_admission_waiter(&world, holder).await;
    assert!(
        !pending.is_finished(),
        "writer must wait for admitted readers"
    );
    assert!(
        synveda_store::source_control::available(
            &mut reader,
            world.tenant_id,
            "session_event",
            event["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
    );
    reader.commit().await.unwrap();
    let (status, result) = pending.await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "{result}");
}

async fn wait_for_admission_waiter(world: &World, holder: i32) {
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {
            let blocked:bool=sqlx::query_scalar("select exists(select 1 from pg_catalog.pg_stat_activity where $1=any(pg_catalog.pg_blocking_pids(pid)))")
                .bind(holder).fetch_one(&world.state.pool).await.unwrap();
            if blocked {break}
            tokio::task::yield_now().await;
        }
    }).await.expect("request did not wait at PostgreSQL admission gate");
}

#[tokio::test]
async fn bounded_cleanup_continues_the_fifth_attempt_and_fences_stale_owners() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(&world, "many-source", "Fictional shared retry source").await;
    for index in 0..102 {
        source_knowledge(
            &world,
            &format!("many-knowledge-{index}"),
            &event,
            &format!("Fictional shared retry source, assertion {index}."),
        )
        .await;
    }
    let removal = remove_source(&world, &event, "forget", "many-forget").await;
    let operation: uuid::Uuid = removal["operation_id"].as_str().unwrap().parse().unwrap();
    let mut stale = None;
    for attempt in 1..=4 {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .unwrap();
        let claim =
            synveda_store::source_control::claim(&mut tx, world.tenant_id, "failed-source-worker")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(claim.attempts, attempt);
        assert!(!claim.exhausted);
        sqlx::query("update durable_operations set lease_expires_at=clock_timestamp()-interval '1 second' where tenant_id=$1 and id=$2")
            .bind(world.tenant_id.as_uuid()).bind(operation).execute(&mut *tx).await.unwrap();
        stale = Some(claim);
        tx.commit().await.unwrap();
    }
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let fifth =
        synveda_store::source_control::claim(&mut tx, world.tenant_id, "fifth-source-worker")
            .await
            .unwrap()
            .unwrap();
    assert_eq!(fifth.attempts, 5);
    tx.commit().await.unwrap();
    let mut forged = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    sqlx::query("select set_config('synveda.source_cleanup_operation',$1,true),set_config('synveda.source_cleanup_attempt','5',true)")
        .bind(operation.to_string()).execute(&mut *forged).await.unwrap();
    assert!(
        !synveda_store::source_control::available(
            &mut forged,
            world.tenant_id,
            "session_event",
            event["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap(),
        "a valid operation GUC cannot confer owner authority"
    );
    forged.commit().await.unwrap();
    let mut old = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    assert!(
        synveda_store::source_control::chunk(
            &mut old,
            world.tenant_id,
            &stale.unwrap(),
            "failed-source-worker"
        )
        .await
        .is_err()
    );
    old.rollback().await.unwrap();
    let mut null_bound = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let invalid = sqlx::query(
        "select * from synveda_source_cleanup_chunk($1,$2,'fifth-source-worker',5,null)",
    )
    .bind(world.tenant_id.as_uuid())
    .bind(operation)
    .fetch_all(&mut *null_bound)
    .await;
    assert!(
        invalid.is_err(),
        "NULL must never mean an unbounded cleanup page"
    );
    null_bound.rollback().await.unwrap();
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages <= 30, "bounded fixture cleanup did not finish");
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .unwrap();
        let claim =
            synveda_store::source_control::claim(&mut tx, world.tenant_id, "fifth-source-worker")
                .await
                .unwrap()
                .unwrap();
        assert_eq!(claim.attempts, 5);
        assert!(!claim.exhausted);
        let page = synveda_store::source_control::chunk(
            &mut tx,
            world.tenant_id,
            &claim,
            "fifth-source-worker",
        )
        .await
        .unwrap();
        assert!(page.erased <= 100, "one page exceeded the target bound");
        tx.commit().await.unwrap();
        if page.done {
            break;
        }
    }
    assert!(
        pages > 9,
        "fixture must require continuation across multi-page stages"
    );
    let (status, view) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{operation}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["cleanup_state"], "succeeded", "{view}");
}

#[tokio::test]
async fn all_trace_modes_withhold_source_backed_saved_bytes() {
    let _guard = serial().await;
    for mode in TraceRetentionMode::ALL {
        let Some(world) = admitted_world().await else {
            return;
        };
        set_trace_mode(&world, mode).await;
        let event = append_source(
            &world,
            "trace-source",
            "Fictional all-mode withdrawal evidence",
        )
        .await;
        let (item, revision) = source_knowledge(
            &world,
            "trace-knowledge",
            &event,
            "Fictional all-mode withdrawal evidence",
        )
        .await;
        let path = format!("/v1/sessions/{}/context-runs", world.alice_session);
        let body = json!({"query":"withdrawal","budget_tokens":1800,"required_knowledge_revisions":[{"item_id":item,"revision_id":revision}]});
        let (status, receipt) = call(
            &world.app,
            "POST",
            &path,
            &world.alice_token,
            Some("trace-receipt"),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{receipt}");
        remove_source(&world, &event, "withdraw", "trace-withdraw").await;
        let (status, replay) = call(
            &world.app,
            "POST",
            &path,
            &world.alice_token,
            Some("trace-receipt"),
            Some(body),
        )
        .await;
        assert!(
            matches!(status, StatusCode::CONFLICT | StatusCode::NOT_FOUND),
            "{mode:?}: {replay}"
        );
        assert!(!replay.to_string().contains("Fictional all-mode"));
    }
}

#[tokio::test]
async fn paused_embedding_cannot_repopulate_a_forgotten_source() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "late-vector-source",
        "Fictional late embedding evidence",
    )
    .await;
    source_knowledge(
        &world,
        "late-vector-knowledge",
        &event,
        "Fictional late embedding evidence",
    )
    .await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let signals = (Arc::clone(&entered), Arc::clone(&release));
    let stub = Router::new().route(
        "/embed",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let (entered, release) = signals.clone();
            async move {
                entered.notify_one();
                release.notified().await;
                axum::Json(vec![
                    vec![0.25_f32, 0.75];
                    body["inputs"].as_array().unwrap().len()
                ])
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let embedder = synveda_ingest::embedding::AnyEmbedder::Tei(
        synveda_ingest::embedding::TeiEmbedder::new(
            "source-barrier-stub".to_owned(),
            format!("http://{address}"),
        )
        .unwrap(),
    );
    let pool = world.state.pool.clone();
    let tenant = world.tenant_id;
    let worker = tokio::spawn(async move {
        synveda_gateway::knowledge_index::sweep_tenant(&pool, &embedder, tenant, 10).await
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    remove_source(&world, &event, "forget", "late-vector-forget").await;
    for _ in 0..20 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "late-vector-cleanup",
        )
        .await
        .unwrap()
        {
            break;
        }
    }
    release.notify_one();
    let result = worker.await.unwrap().unwrap();
    assert_eq!(result.inserted, 0, "late vector was admitted after cleanup");
    server.abort();
}

#[tokio::test]
async fn paused_capture_rejects_the_original_frozen_input_after_withdrawal() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "late-capture-source",
        "The fictional retry cap is 3.",
    )
    .await;
    let (status, batch) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/capture-batches", world.alice_session),
        &world.alice_token,
        Some("late-capture-batch"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{batch}");
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let signals = (Arc::clone(&entered), Arc::clone(&release));
    let stub=Router::new().route("/v1/chat/completions",axum::routing::post(move || {
        let (entered,release)=signals.clone();async move {
            entered.notify_one();release.notified().await;
            axum::Json(json!({"model":"source-capture-stub","choices":[{"message":{"content":json!({"candidates":[{
                "knowledge_type":"convention","title":"Old retry policy","body_markdown":"The fictional retry cap is 3.",
                "summary":"Retry cap 3.","tags":["retry"],"confidence":0.9}]}).to_string()}}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
    let deps = synveda_ingest::capture_worker::Deps {
        pool: world.state.pool.clone(),
        pdp: Arc::clone(&world.state.pdp),
        extractor: Arc::new(synveda_ingest::extraction::AnyExtractor::Vllm(
            synveda_ingest::extraction::VllmExtractor::new(
                "source-capture-stub".to_owned(),
                format!("http://{address}"),
            )
            .unwrap(),
        )),
    };
    let tenant = world.tenant_id;
    let worker = tokio::spawn(async move {
        synveda_ingest::capture_worker::sweep_tenant_once(
            &deps,
            &synveda_ingest::capture_worker::Config {
                poll_interval: Duration::from_secs(1),
                lease_duration: Duration::from_secs(30),
                batches_per_tenant: 1,
                lease_owner: "late-capture-worker".to_owned(),
            },
            tenant,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    remove_source(&world, &event, "withdraw", "late-capture-withdraw").await;
    release.notify_one();
    let result = worker.await.unwrap().unwrap();
    assert_eq!(result.completed, 0, "{result:?}");
    let (status, after) = call(
        &world.app,
        "GET",
        &format!("/v1/capture-batches/{}", batch["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["state"], "failed", "{after}");
    assert_eq!(after["error_code"], "source_unavailable");
    assert_eq!(after["input_hash"], batch["input_hash"]);
    assert_eq!(after["event_count"], batch["event_count"]);
    assert_eq!(after["candidate_count"], 0);
    server.abort();
}

#[tokio::test]
async fn a_hold_on_a_later_page_blocks_before_any_content_cleanup() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "held-source",
        "Fictional retained source under a hold",
    )
    .await;
    for index in 0..101 {
        let mut data = content(
            "Held retry source",
            &format!("Fictional held assertion {index}"),
            None,
        );
        if index == 100 {
            data["metadata"]["legal_hold"] = json!(true)
        }
        let (status,created)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some(&format!("held-knowledge-{index}")),
            Some(json!({"scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"convention","origin":"observed","content":data,
                "sources":[{"scope_id":world.project_scope,"source_type":"session_event","session_event_id":event["id"],"metadata":{}}]}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["outcome"], "applied", "{created}");
    }
    let removal = remove_source(&world, &event, "forget", "held-source-forget").await;
    let operation: uuid::Uuid = removal["operation_id"].as_str().unwrap().parse().unwrap();
    for _ in 0..3 {
        synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "held-source-worker",
        )
        .await
        .unwrap();
    }
    let (status, view) = call(
        &world.app,
        "GET",
        &format!("/v1/operations/{operation}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["cleanup_state"], "blocked", "{view}");
    assert_eq!(view["cleanup_stage"], "retention_check");
    assert_eq!(view["error_code"], "retention_hold");
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let processed:i64=sqlx::query_scalar("select processed_targets from source_cleanup_targets where tenant_id=$1 and operation_id=$2")
        .bind(world.tenant_id.as_uuid()).bind(operation).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(
        processed, 0,
        "preflight must not erase any target before all holds are checked"
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn retained_address_suppresses_an_event_even_after_raw_retention() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let text = "Fictional disposed source evidence";
    let event = append_source(&world, "disposed-source", text).await;
    // Exercise the existing owner-only retention disposal guard with an
    // ordinary tenant transaction; no RLS or immutable trigger is disabled.
    let owner = tenant_fixture::migrator_pool(&world.state.pool).await;
    let mut disposal = rls::begin_tenant_tx(&owner, world.tenant_id).await.unwrap();
    sqlx::query("select set_config('synveda.retention_purge','on',true)")
        .execute(&mut *disposal)
        .await
        .unwrap();
    let removed = sqlx::query("delete from session_events where tenant_id=$1 and id=$2")
        .bind(world.tenant_id.as_uuid())
        .bind(event["id"].as_str().unwrap().parse::<uuid::Uuid>().unwrap())
        .execute(&mut *disposal)
        .await
        .unwrap();
    assert_eq!(removed.rows_affected(), 1);
    disposal.commit().await.unwrap();
    remove_source(&world, &event, "withdraw", "disposed-source-withdraw").await;
    let (status,body)=call(&world.app,"POST",&format!("/v1/sessions/{}/events",world.alice_session),&world.alice_token,None,
        Some(json!({"events":[{"client_event_id":"disposed-source","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":text}}]}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(!body.to_string().contains(text));
}

#[tokio::test]
async fn item_forget_clears_incident_relations_asserted_by_another_withdrawn_item() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (original, revision) = create_knowledge(
        &world,
        "incident-original",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Earlier retry procedure",
        "Independent earlier retry procedure.",
        None,
    )
    .await;
    let event = append_source(&world, "incident-source", "Fictional superseding source").await;
    let (status,replacement)=call(&world.app,"POST",&format!("/v1/knowledge/{original}/supersede"),&world.alice_token,Some("incident-supersede"),
        Some(json!({"expected_revision_id":revision,"scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"convention","origin":"observed",
            "content":content("Superseding procedure","Fictional superseding source",None),
            "sources":[{"scope_id":world.project_scope,"source_type":"session_event","session_event_id":event["id"],"metadata":{}}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{replacement}");
    assert_eq!(replacement["outcome"], "applied");
    remove_source(&world, &event, "withdraw", "incident-withdraw").await;
    let (status,forgotten)=call(&world.app,"DELETE",&format!("/v1/knowledge/{original}"),&world.alice_token,Some("incident-original-forget"),
        Some(json!({"mode":"forget","expected_revision_id":revision,"reason":"Retire the independent earlier item"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied", "{forgotten}");
}

#[tokio::test]
async fn denied_source_controls_match_absence_and_retain_denial_audit() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let event = append_source(
        &world,
        "denied-control-source",
        "Fictional inaccessible source",
    )
    .await;
    remove_source(&world, &event, "withdraw", "denied-control-withdraw").await;
    let source = format!(
        "{BLANKET}\nforbid (principal, action in [Synveda::Action::\"SessionRead\",Synveda::Action::\"SessionWrite\"], resource);"
    );
    world
        .install_policy("source-control-denied", 1, &source, PackConfig::default())
        .await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "source-control-denied")
        .await;
    tx.commit().await.unwrap();
    let known = format!(
        "/v1/sessions/{}/events/{}/source-control",
        world.alice_session,
        event["id"].as_str().unwrap()
    );
    let absent = format!(
        "/v1/sessions/{}/events/{}/source-control",
        synveda_types::SessionId::new(),
        synveda_types::SessionEventId::new()
    );
    let denied = call(&world.app, "GET", &known, &world.alice_token, None, None).await;
    let missing = call(&world.app, "GET", &absent, &world.alice_token, None, None).await;
    assert_eq!(denied.0, StatusCode::NOT_FOUND);
    assert_eq!(denied, missing);
    let events = audit_events(&world).await;
    assert!(events.iter().any(|entry| entry.action
        == synveda_audit::AuditAction::AuthzDecision.as_str()
        && entry.outcome == synveda_audit::Outcome::Deny.as_str()));
}

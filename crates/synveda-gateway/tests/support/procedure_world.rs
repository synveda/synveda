//! Ordinary-role public learning acceptance.
#![allow(dead_code)]
use super::context_world::*;
use synveda_types::{ApprovalMatrix, ApprovalRule, AssetKind, Sensitivity};

pub(crate) struct Learning {
    pub(crate) world: World,
    pub(crate) procedure: Value,
    pub(crate) runner: String,
    pub(crate) evaluator: String,
    pub(crate) suite: Value,
}
pub(crate) async fn setup() -> Option<Learning> {
    Some(setup_world(admitted_world().await?, None).await)
}
pub(crate) async fn setup_world(world: World, existing: Option<Value>) -> Learning {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    let procedure = if let Some(procedure) = existing {
        procedure
    } else {
        let (status,created)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some("learning-baseline"),Some(json!({"scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"procedure","origin":"authored","content":content("Iris retry guard","Retry at most 3 times.\n\nNever retry an unknown commit outcome.",None),"sources":[{"scope_id":world.project_scope,"source_type":"manual","metadata":{"fixture":"owned-learning"}}]}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let item = created["knowledge_item_id"].as_str().unwrap();
        let (status, itemview) = call(
            &world.app,
            "GET",
            &format!("/v1/knowledge/{item}"),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{itemview}");
        json!({"item_id":item,"revision_id":created["revision_id"],"content_hash":itemview["current_revision"]["content_hash"]})
    };
    let mut reporter_ids = Vec::new();
    let mut tokens = Vec::new();
    for subject in ["owned-procedure-runner", "owned-procedure-evaluator"] {
        let (status, registered) = call(
            &world.app,
            "POST",
            "/v1/service-identities",
            &world.alice_token,
            None,
            Some(json!({"scope_id":world.workspace_scope,"subject":subject})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{registered}");
        reporter_ids.push(registered["id"].clone());
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .unwrap();
        seed_grant(
            &mut tx,
            world.tenant_id,
            world.project_scope,
            subject,
            RoleKey::Curator,
        )
        .await;
        tx.commit().await.unwrap();
        tokens.push(Hs256Verifier::new(SECRET).issue(
            subject,
            world.tenant_id,
            Duration::from_secs(300),
        ));
    }
    use sha2::{Digest, Sha256};
    let digest = |v: Value| format!("{:x}", Sha256::digest(serde_json::to_vec(&v).unwrap()));
    let suite = json!({"suite_id":"owned-held-out-v1","evaluator_id":reporter_ids[1],"harness":"closed-iris-retry","harness_version":"v1",
        "configuration_digest":digest(json!({"max_attempts":5,"case_version":1})),"cases":[
        {"case_id":"held-out-delayed-ack","digest":digest(json!({"kind":"safe_delayed_ack","ack_after_ms":350})),"required":true},
        {"case_id":"held-out-unknown-commit","digest":digest(json!({"kind":"unknown_commit","ack_after_ms":350})),"required":true}]});
    let policy = PackConfig {
        approvals: Some(ApprovalMatrix {
            rules: vec![ApprovalRule {
                asset: Some(AssetKind::Knowledge),
                min_sensitivity: Sensitivity::Public,
                scope_kinds: None,
                roles: vec![],
                distinct_approvers: 1,
                forbid_author_approval: true,
                separate_effect_actor: false,
            }],
        }),
        ..PackConfig::default()
    };
    world
        .install_policy("learning-review", 1, BLANKET, policy)
        .await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "learning-review").await;
    tx.commit().await.unwrap();
    let (status, effective) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/configurations/effective?scope_id={}",
            world.project_scope
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{effective}");
    let mut document = effective["document"].clone();
    document["policy_pack"] = json!("learning-review");
    reporter_ids.sort_by_key(Value::to_string);
    document["context"]["channels"] = json!(["current_knowledge"]);
    document["procedure_learning"] =
        json!({"reporters":reporter_ids,"evaluation_suites":[suite.clone()]});
    let artifact = effective["artifact_id"].as_str().unwrap();
    let (status, configured) = call(
        &world.app,
        "POST",
        &format!("/v1/configurations/{artifact}/versions"),
        &world.alice_token,
        Some("learning-custody"),
        Some(json!({"expected_current_version_id":effective["version_id"],"document":document})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{configured}");
    if configured["outcome"] == "pending_review" {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .unwrap();
        seed_grant(
            &mut tx,
            world.tenant_id,
            world.project_scope,
            BOB,
            RoleKey::Administrator,
        )
        .await;
        tx.commit().await.unwrap();
        review_world(&world, configured["change_id"].as_str().unwrap()).await;
    } else {
        assert_eq!(configured["outcome"], "applied", "{configured}");
    }
    Learning {
        world,
        procedure,
        runner: tokens[0].clone(),
        evaluator: tokens[1].clone(),
        suite,
    }
}
pub(crate) fn outcome_body(l: &Learning, key: &str) -> Value {
    json!({"client_event_id":key,"procedure":l.procedure,"skill_versions":[],"environment":{"task_class":"retry-guard","environment_id":"owned-fixture","environment_digest":"d".repeat(64)},"outcome":"failed","supporting_events":[],"result_digest":"e".repeat(64)})
}
pub(crate) async fn observe(l: &Learning, token: &str, key: &str) -> Value {
    let (status, result) = call(
        &l.world.app,
        "POST",
        &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
        token,
        None,
        Some(outcome_body(l, key)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    result["event"].clone()
}
pub(crate) fn improvement_body(l: &Learning, event: &Value) -> Value {
    json!({"procedure":l.procedure,"outcomes":[{"event_id":event["id"],"payload_hash":event["payload_hash"]}],"proposed_append":"Use a 200 ms delay before each safe retry.","uncertainty":"Controlled fixture evidence; external provider behavior remains unqualified.","development_case":"development-retry"})
}
pub(crate) async fn candidate(l: &Learning, event: &Value) -> (Value, Value) {
    let body = improvement_body(l, event);
    let path = format!(
        "/v1/sessions/{}/procedure-improvements",
        l.world.alice_session
    );
    let (a, b) = tokio::join!(
        call(
            &l.world.app,
            "POST",
            &path,
            &l.world.alice_token,
            None,
            Some(body.clone())
        ),
        call(
            &l.world.app,
            "POST",
            &path,
            &l.world.alice_token,
            None,
            Some(body)
        )
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["event"]["id"], b.1["event"]["id"]);
    assert_eq!(a.1["batch_id"], b.1["batch_id"]);
    let deps = synveda_ingest::capture_worker::Deps {
        pool: l.world.state.pool.clone(),
        pdp: l.world.state.pdp.clone(),
        extractor: Arc::new(synveda_ingest::extraction::AnyExtractor::Deterministic(
            synveda_ingest::extraction::DeterministicExtractor::new(),
        )),
    };
    let swept = synveda_ingest::capture_worker::sweep_tenant_once(
        &deps,
        &synveda_ingest::capture_worker::Config::default(),
        l.world.tenant_id,
    )
    .await
    .unwrap();
    assert_eq!(swept.failed_attempts, 0, "{swept:?}");
    assert!(swept.completed >= 1, "{swept:?}");
    let (status, batch) = call(
        &l.world.app,
        "GET",
        &format!(
            "/v1/capture-candidates?batch_id={}",
            a.1["batch_id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    let candidate = batch["candidates"][0].clone();
    assert_eq!(
        candidate["content_hash"],
        a.1["event"]["payload"]["content_hash"]
    );
    assert_eq!(candidate["content"], a.1["event"]["payload"]["content"]);
    (a.1, candidate)
}
pub(crate) async fn freeze(l: &Learning, candidate: &Value) -> Value {
    let (status, attempt) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/procedure-evaluations",
            candidate["id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        None,
        Some(json!({"candidate_hash":candidate["content_hash"],"suite_id":l.suite["suite_id"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{attempt}");
    assert!(attempt["qualified"].is_null());
    attempt
}
pub(crate) fn observations(l: &Learning, pass: bool) -> Value {
    l.suite["cases"].as_array().unwrap().iter().map(|case|json!({"case_id":case["case_id"],"digest":case["digest"],"baseline":"passed","candidate":if pass{"passed"}else{"failed"},"result_digest":"f".repeat(64)})).collect()
}
pub(crate) async fn results(l: &Learning, attempt: &Value, pass: bool) -> Value {
    let body = json!({"candidate_hash":attempt["candidate_hash"],"specification_hash":attempt["specification_hash"],"results":observations(l,pass)});
    let (status, result) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/procedure-evaluations/{}/results",
            attempt["attempt_id"].as_str().unwrap()
        ),
        &l.evaluator,
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["qualified"], pass);
    result
}
pub(crate) async fn review(l: &Learning, change: &str) {
    review_world(&l.world, change).await;
}
async fn review_world(world: &World, change: &str) {
    let (status, p) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{change}"),
        &world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{p}");
    let (status, approved) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/approve"),
        &world.bob_token,
        None,
        Some(json!({"expected_commit":p["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let (status, applied) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change}/apply"),
        &world.alice_token,
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
}

//! CPR-40 / SKIL-6 authorized diagnostics and the separate owned-fixture oracle.
#[path = "support/context_world.rs"]
mod context_world;
#[path = "support/procedure_world.rs"]
mod procedure_world;
use context_world::*;
use procedure_world::*;

fn finding<'a>(detail: &'a Value, dimension: &str) -> &'a Value {
    detail["health"]["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["dimension"] == dimension)
        .unwrap()
}

#[tokio::test]
async fn frozen_delivery_diagnoses_four_causes_without_an_oracle_in_production() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    // Frozen private expectations precede the production request. The missing
    // source deliberately has no published Knowledge or production query hint.
    let expected = l.procedure.clone();
    let unpublished_source = "owned fixture: sandbox delivery exclusion";
    let query = "response signalling completion";
    let path = format!("/v1/sessions/{}/context-runs", l.world.alice_session);
    let (status, ordinary) = call(
        &l.world.app,
        "POST",
        &path,
        &l.world.alice_token,
        Some("diagnostic-query-first"),
        Some(json!({"query":query,"budget_tokens":1400})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ordinary}");
    let frozen_hash = ordinary["block_hash"].clone();
    // The diagnostic lens is read only and runs after the production freeze.
    let (status, exact) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/sessions/{}/knowledge-evaluation",
            l.world.alice_session
        ),
        &l.world.alice_token,
        None,
        Some(json!({"ids":[expected["item_id"]],"limit":1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{exact}");
    let exact_item = &exact["items"][0]["knowledge"];
    assert_eq!(exact_item["id"], expected["item_id"]);
    assert_eq!(
        exact_item["current_revision"]["id"],
        expected["revision_id"]
    );
    assert_eq!(
        exact_item["current_revision"]["content_hash"],
        expected["content_hash"]
    );
    assert!(!exact.to_string().contains(unpublished_source));
    let query_detail = detail(
        &l.world,
        &l.world.alice_token,
        ordinary["id"].as_str().unwrap(),
    )
    .await;
    assert!(
        !query_detail["selections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["knowledge_revision_id"] == expected["revision_id"]),
        "fixture query must actually miss"
    );
    let (status, supplied) = call(&l.world.app,"POST",&path,&l.world.alice_token,
        Some("diagnostic-exact-delivery"),Some(json!({"budget_tokens":1400,
        "required_knowledge_revisions":[{"item_id":expected["item_id"],"revision_id":expected["revision_id"]}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{supplied}");
    let rendering = supplied["rendered"].as_str().unwrap();
    let qualifier = "Never retry an unknown commit outcome.";
    assert!(rendering.contains(qualifier));
    let injected = rendering.replace(qualifier, "");
    assert!(!injected.contains(qualifier));
    // A controlled runner executes a deliberately failing local task after
    // receiving the complete proposition; its failed result is not causal credit.
    let task = std::process::Command::new("sh")
        .args(["-c", "test 1 -eq 2"])
        .status()
        .unwrap();
    assert!(!task.success());
    let mut body = outcome_body(&l, "diagnostic-owned-failure");
    body["context"] = json!({"context_run_id":supplied["id"],"block_hash":supplied["block_hash"]});
    let (status, observation) = call(
        &l.world.app,
        "POST",
        &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
        &l.runner,
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{observation}");
    let evidence_class = &observation["event"]["payload"]["evidence_class"];
    let cases = json!([
        {"evidence":{"source_published":false}},
        {"evidence":{"source_published":true,"exact_eligible":true,"query_retrieved":false}},
        {"fault_injected_output":true,"evidence":{"query_retrieved":true,"qualifier_preserved":false}},
        {"evidence":{"delivered_exact":true,"evidence_class":evidence_class,"task_outcome":"failed"}}
    ]);
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("frozen-oracle.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&json!({"hash":frozen_hash,"cases":cases,
        "limits":query_detail["run"]["degradation"]}))
        .unwrap(),
    )
    .unwrap();
    let module = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../evals/product/health-diagnosis.mjs");
    let output=tokio::process::Command::new("node").args(["--input-type=module","-e",
        "import{readFileSync}from'node:fs';import{pathToFileURL}from'node:url';const{diagnoseContextFixture}=await import(pathToFileURL(process.argv[1]));const x=JSON.parse(readFileSync(process.argv[2]));process.stdout.write(JSON.stringify(x.cases.map(c=>diagnoseContextFixture({...c,scope:'owned_fixture_oracle',frozen_delivery_hash:x.hash,observed_retrieval_limits:x.limits}))));"])
        .arg(module).arg(input).output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let diagnosis: Value = serde_json::from_slice(&output.stdout).unwrap();
    for (i, stage) in ["representation", "retrieval", "assembly", "delivery_use"]
        .iter()
        .enumerate()
    {
        assert_eq!(diagnosis[i]["stage"], *stage);
        assert_eq!(diagnosis[i]["causal_attribution"], "unknown");
        assert_eq!(diagnosis[i]["production_ranking_input"], false);
    }
    assert_eq!(diagnosis[2]["fault_injected_output"], true);
    let health = detail(
        &l.world,
        &l.world.alice_token,
        supplied["id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(finding(&health, "outcomes")["status"], "needs_attention");
    assert!(
        finding(&health, "outcomes")["explanation"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );
    assert!(
        health["health"]["pipeline"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["stage"] == "representation" && s["status"] == "unknown")
    );
}

#[tokio::test]
async fn newer_success_cannot_hide_failure_and_reduced_traces_hide_outcome_addresses() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let (status,run)=call(&l.world.app,"POST",&format!("/v1/sessions/{}/context-runs",l.world.alice_session),&l.world.alice_token,Some("health-outcomes"),
        Some(json!({"budget_tokens":1400,"required_knowledge_revisions":[{"item_id":l.procedure["item_id"],"revision_id":l.procedure["revision_id"]}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let mut failure = Value::Null;
    for (key, outcome, token) in [
        ("old-failure", "failed", &l.runner),
        ("new-success", "succeeded", &l.runner),
        ("unsupported-success", "succeeded", &l.world.alice_token),
    ] {
        let mut body = outcome_body(&l, key);
        body["outcome"] = json!(outcome);
        body["context"] = json!({"context_run_id":run["id"],"block_hash":run["block_hash"]});
        let (status, reported) = call(
            &l.world.app,
            "POST",
            &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
            token,
            None,
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reported}");
        if outcome == "failed" {
            failure = reported["event"].clone();
        }
    }
    let inspected = detail(&l.world, &l.world.alice_token, run["id"].as_str().unwrap()).await;
    assert_eq!(finding(&inspected, "outcomes")["status"], "needs_attention");
    assert_eq!(
        finding(&inspected, "outcomes")["evidence"]["event_id"],
        failure["id"]
    );
    for mode in [TraceRetentionMode::HashesOnly, TraceRetentionMode::Disabled] {
        let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
            .await
            .unwrap();
        let root = scopes::tenant_root(&mut *tx, l.world.tenant_id)
            .await
            .unwrap()
            .unwrap();
        configuration_support::set_trace_retention(&mut tx, l.world.tenant_id, root.id, mode).await;
        tx.commit().await.unwrap();
        let (status, reduced) = call(&l.world.app, "POST", &format!("/v1/sessions/{}/context-runs",l.world.alice_session), &l.world.alice_token, Some(mode.as_str()), Some(json!({"budget_tokens":1400,"required_knowledge_revisions":[{"item_id":l.procedure["item_id"],"revision_id":l.procedure["revision_id"]}]}))).await;
        assert_eq!(status, StatusCode::CREATED, "{reduced}");
        let mut body = outcome_body(&l, mode.as_str());
        body["context"] =
            json!({"context_run_id":reduced["id"],"block_hash":reduced["block_hash"]});
        let (status, reported) = call(
            &l.world.app,
            "POST",
            &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
            &l.runner,
            None,
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reported}");
        let inspected = detail(
            &l.world,
            &l.world.alice_token,
            reduced["id"].as_str().unwrap(),
        )
        .await;
        assert!(
            inspected["health"]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f.get("evidence").is_none_or(Value::is_null))
        );
        assert!(
            !inspected["health"]
                .to_string()
                .contains(failure["id"].as_str().unwrap())
        );
    }
}

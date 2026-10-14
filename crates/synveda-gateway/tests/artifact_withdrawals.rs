//! MEM-8 actual imported evidence, without synthetic Session provenance.
#[path = "support/context_world.rs"]
mod context_world;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use context_world::*;

fn entry(path: &str, text: &str) -> Value {
    json!({"logical_path":path,"kind":"file","content_base64":STANDARD.encode(text)})
}
fn bundle(cap: u8, extra: bool) -> Value {
    let mut entries = vec![
        entry(
            "index.md",
            "---\nokf_version: \"0.2\"\n---\n\n# Fictional Iris\n",
        ),
        entry(
            "procedures/retries.md",
            &format!(
                "---\ntype: Procedure\ntitle: Iris retry policy\ndescription: The approved fictional retry procedure.\n---\n\nThe fictional Iris retry cap is {cap} attempts. See [missing note](missing.md).\n"
            ),
        ),
        entry(
            "conventions/trace.md",
            "---\ntype: Convention\ntitle: Iris tracing\ndescription: Independent fictional tracing evidence.\n---\n\nUse traceparent for fictional Iris requests.\n",
        ),
    ];
    if extra {
        entries.push(entry("index-notes.md","---\ntype: Reference\ntitle: Extra import note\ndescription: An unrelated note changes the bundle digest.\n---\n\nThis note is independent.\n"));
    }
    json!({"source_kind":"git","source_locator":"https://example.test/iris.git","source_revision":if extra {"commit-b"}else{"commit-a"},"encoding":"entries","entries":entries})
}
async fn plan(world: &World, key: &str, cap: u8, extra: bool) -> Value {
    let (status, body) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{}/okf/imports", world.project_id),
        &world.alice_token,
        Some(key),
        Some(bundle(cap, extra)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}
async fn materialize(world: &World, plan: &Value, key: &str) -> Value {
    let (status, body) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/okf/imports/{}/materialize",
            plan["job"]["id"].as_str().unwrap()
        ),
        &world.alice_token,
        Some(key),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}
fn artifact(plan: &Value, path: &str) -> Value {
    plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["logical_path"] == path)
        .unwrap()
        .clone()
}
async fn accept(world: &World, batch: &Value, title: &str, key: &str) -> Value {
    let candidate = batch["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["content"]["title"] == title)
        .unwrap();
    let (status, body) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/accept",
            candidate["id"].as_str().unwrap()
        ),
        &world.alice_token,
        Some(key),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["candidate"]["resulting_outcome"], "applied", "{body}");
    body["candidate"].clone()
}
async fn remove(world: &World, plan: &Value, source: &Value, mode: &str, key: &str) -> Value {
    let (status,body)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/{mode}",plan["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),
      &world.alice_token,Some(key),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Retire this fictional imported evidence"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["outcome"], "applied", "{body}");
    body
}

#[tokio::test]
async fn artifact_withdrawal_hides_exact_derivatives_and_shared_aliases() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-plan-a", 3, false).await;
    let batch = materialize(&world, &first, "artifact-materialize-a").await;
    let retry = accept(&world, &batch, "Iris retry policy", "artifact-accept-a").await;
    let clean = accept(&world, &batch, "Iris tracing", "artifact-accept-clean").await;
    let duplicate = plan(&world, "artifact-plan-b", 3, true).await;
    let source = artifact(&first, "procedures/retries.md");
    let item = retry["resulting_knowledge_item_id"].as_str().unwrap();
    let revision = retry["resulting_revision_id"].as_str().unwrap();
    let request = json!({"query":"Iris retry","budget_tokens":1800,"required_knowledge_revisions":[{"item_id":item,"revision_id":revision}]});
    let path = format!("/v1/sessions/{}/context-runs", world.alice_session);
    let (status, receipt) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("artifact-saved"),
        Some(request.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{receipt}");
    assert!(receipt["rendered"].as_str().unwrap().contains("3 attempts"));
    let removal = remove(&world, &first, &source, "withdraw", "artifact-withdraw").await;
    let (status, withheld) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}"),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{withheld}");
    let (status, remaining) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/knowledge/{}",
            clean["resulting_knowledge_item_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{remaining}");
    assert!(remaining.to_string().contains("traceparent"));
    let (status, replay) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        Some("artifact-saved"),
        Some(request),
    )
    .await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::CONFLICT),
        "{replay}"
    );
    assert!(!replay.to_string().contains("3 attempts"));
    for current in [&first, &duplicate] {
        let (status, view) = call(
            &world.app,
            "GET",
            &format!("/v1/okf/imports/{}", current["job"]["id"].as_str().unwrap()),
            &world.alice_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(view["job"]["notices"], json!([]));
        assert!(!view.to_string().contains("3 attempts"));
        assert!(
            view["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["logical_path"] == "conventions/trace.md")
        );
    }
    let (status, blocked) = call(
        &world.app,
        "POST",
        &format!("/v1/projects/{}/okf/imports", world.project_id),
        &world.alice_token,
        Some("artifact-plan-a"),
        Some(bundle(3, false)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert!(!blocked.to_string().contains("3 attempts"));
    let (status, blocked) = call(
        &world.app,
        "POST",
        &format!(
            "/v1/okf/imports/{}/materialize",
            duplicate["job"]["id"].as_str().unwrap()
        ),
        &world.alice_token,
        Some("artifact-materialize-b"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    let changed = plan(&world, "artifact-changed", 5, false).await;
    assert!(changed.to_string().contains("5 attempts"));
    let (status, operation) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/operations/{}",
            removal["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["mode"], "withdraw");
}

#[tokio::test]
async fn artifact_forget_cleans_service_copies_and_preserves_item_forget() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-forget-plan", 3, false).await;
    let batch = materialize(&world, &first, "artifact-forget-materialize").await;
    let retry = accept(
        &world,
        &batch,
        "Iris retry policy",
        "artifact-forget-accept",
    )
    .await;
    let source = artifact(&first, "procedures/retries.md");
    let removal = remove(&world, &first, &source, "forget", "artifact-forget").await;
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "artifact-acceptance-worker",
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
            removal["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["cleanup_state"], "succeeded", "{operation}");
    let item = retry["resulting_knowledge_item_id"].as_str().unwrap();
    let (status,forgotten)=call(&world.app,"DELETE",&format!("/v1/knowledge/{item}"),&world.alice_token,Some("artifact-then-item-forget"),
       Some(json!({"mode":"forget","expected_revision_id":retry["resulting_revision_id"],"reason":"Retire the fictional item too"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied", "{forgotten}");
}

#[tokio::test]
async fn item_forget_retains_inert_mapping_replay_before_artifact_forget() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "item-first-plan", 3, false).await;
    let batch = materialize(&world, &first, "item-first-materialize").await;
    let retry = accept(&world, &batch, "Iris retry policy", "item-first-accept").await;
    let item = retry["resulting_knowledge_item_id"].as_str().unwrap();
    let (status,forgotten)=call(&world.app,"DELETE",&format!("/v1/knowledge/{item}"),&world.alice_token,Some("item-first-forget"),
       Some(json!({"mode":"forget","expected_revision_id":retry["resulting_revision_id"],"reason":"Retire this fictional item before its source"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied", "{forgotten}");
    let (status, view) = call(
        &world.app,
        "GET",
        &format!("/v1/okf/imports/{}", first["job"]["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let mapping = view["mappings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|mapping| mapping["artifact_id"] == artifact(&first, "procedures/retries.md")["id"])
        .unwrap();
    assert_eq!(mapping["content_erased"], true);
    assert_eq!(mapping["okf_type"], "[erased]");
    for key in ["item-first-plan", "item-first-new-key"] {
        let (status, replayed) = call(
            &world.app,
            "POST",
            &format!("/v1/projects/{}/okf/imports", world.project_id),
            &world.alice_token,
            Some(key),
            Some(bundle(3, false)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replayed}");
        assert_eq!(replayed["job"]["id"], first["job"]["id"]);
        assert!(
            replayed["mappings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|mapping| mapping["content_erased"] == true)
        );
    }
    let source = artifact(&first, "procedures/retries.md");
    let removal = remove(
        &world,
        &first,
        &source,
        "forget",
        "item-first-source-forget",
    )
    .await;
    for _ in 0..30 {
        if !synveda_gateway::source_control::cleanup_page(
            &world.state.pool,
            world.tenant_id,
            "item-first-cleanup",
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
            removal["operation_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["cleanup_state"], "succeeded", "{operation}");
}

#[tokio::test]
async fn typed_artifact_provenance_is_exact_and_generic_metadata_stays_generic() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "typed-source-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    let locator = "https://example.test/iris.git#procedures/retries.md";
    let (status,created)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some("typed-source-write"),Some(json!({
      "scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"fact","origin":"asserted",
      "content":content("Inspected Iris procedure","Use the inspected fictional procedure as evidence.",None),
      "sources":[{"scope_id":world.project_scope,"source_type":"okf","import_artifact_id":source["id"],"locator":locator,"source_revision":"commit-a","content_hash":source["content_hash"],"metadata":{}}]
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    let (status, proposal) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{}", created["change_id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    let references = proposal["artifact_references"].as_array().unwrap();
    let reference = references
        .iter()
        .find(|reference| reference["family"] == "okf_import")
        .unwrap();
    assert_eq!(reference["artifact_id"], source["id"]);
    assert!(
        !references
            .iter()
            .any(|reference| reference.to_string().contains(locator))
    );
    let (status,generic)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some("generic-external-source"),Some(json!({
      "scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"reference","origin":"asserted",
      "content":content("External document identity","This independent external document is ordinary evidence.",None),
      "sources":[{"scope_id":world.project_scope,"source_type":"document","locator":"external:fictional-document","metadata":{"artifact_id":"external-system-id","import_job_id":"external-job"}}]
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{generic}");
    assert_eq!(generic["outcome"], "applied", "{generic}");
    remove(&world, &first, &source, "withdraw", "typed-source-withdraw").await;
    let (status, visible) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/knowledge/{}",
            generic["knowledge_item_id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{visible}");
    assert!(visible.to_string().contains("ordinary evidence"));
}

#[tokio::test]
async fn pending_artifact_control_retains_source_until_review() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "pending-source-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    let config = PackConfig {
        approvals: Some(synveda_types::ApprovalMatrix {
            rules: vec![synveda_types::ApprovalRule {
                asset: Some(synveda_types::AssetKind::Knowledge),
                min_sensitivity: synveda_types::Sensitivity::Public,
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
        .install_policy("artifact-review", 1, BLANKET, config)
        .await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "artifact-review").await;
    tx.commit().await.unwrap();
    let (status,pending)=call(&world.app,"POST",&format!("/v1/okf/imports/{}/artifacts/{}/withdraw",first["job"]["id"].as_str().unwrap(),source["id"].as_str().unwrap()),
       &world.alice_token,Some("pending-artifact-withdraw"),Some(json!({"expected_content_hash":source["content_hash"],"reason":"Ask an independent reviewer"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["outcome"], "pending_review", "{pending}");
    let (status, view) = call(
        &world.app,
        "GET",
        &format!("/v1/okf/imports/{}", first["job"]["id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert!(view.to_string().contains("3 attempts"));
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    assert!(
        synveda_store::artifact_control::status(
            &mut tx,
            world.tenant_id,
            source["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
        .is_none()
    );
    tx.commit().await.unwrap();
}

async fn artifact_knowledge(world: &World, source: &Value, key: &str, held: bool) {
    let mut data = content(
        "Inspected Iris evidence",
        "Fictional evidence derived from this exact import.",
        None,
    );
    if held {
        data["metadata"]["legal_hold"] = json!(true);
    }
    let (status, created) = call(&world.app, "POST", "/v1/knowledge", &world.alice_token, Some(key), Some(json!({
        "scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"fact","origin":"asserted","content":data,
        "sources":[{"scope_id":world.project_scope,"source_type":"okf","import_artifact_id":source["id"],
          "locator":"https://example.test/iris.git#procedures/retries.md","source_revision":"commit-a","content_hash":source["content_hash"],"metadata":{}}]
    }))).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
}

#[tokio::test]
async fn artifact_hold_blocks_before_erasure_and_preserves_suppression() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-held-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    artifact_knowledge(&world, &source, "artifact-held-knowledge", true).await;
    let removal = remove(&world, &first, &source, "forget", "artifact-held-forget").await;
    synveda_gateway::source_control::cleanup_page(
        &world.state.pool,
        world.tenant_id,
        "artifact-held-worker",
    )
    .await
    .unwrap();
    let operation: uuid::Uuid = removal["operation_id"].as_str().unwrap().parse().unwrap();
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
    let processed: i64 = sqlx::query_scalar("select processed_targets from artifact_cleanup_targets where tenant_id=$1 and operation_id=$2")
        .bind(world.tenant_id.as_uuid()).bind(operation).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(processed, 0);
    assert!(
        !synveda_store::source_control::available(
            &mut tx,
            world.tenant_id,
            "import_artifact",
            source["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn denied_artifact_controls_match_absence_and_retain_denial_audit() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-denied-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    remove(
        &world,
        &first,
        &source,
        "withdraw",
        "artifact-denied-withdraw",
    )
    .await;
    let policy = format!(
        "{BLANKET}\nforbid (principal, action == Synveda::Action::\"KnowledgeRead\", resource);"
    );
    world
        .install_policy("artifact-control-denied", 1, &policy, PackConfig::default())
        .await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "artifact-control-denied")
        .await;
    tx.commit().await.unwrap();
    let known = format!(
        "/v1/okf/imports/{}/artifacts/{}/source-control",
        first["job"]["id"].as_str().unwrap(),
        source["id"].as_str().unwrap()
    );
    let absent = format!(
        "/v1/okf/imports/{}/artifacts/{}/source-control",
        uuid::Uuid::now_v7(),
        uuid::Uuid::now_v7()
    );
    let denied = call(&world.app, "GET", &known, &world.alice_token, None, None).await;
    let missing = call(&world.app, "GET", &absent, &world.alice_token, None, None).await;
    assert_eq!(denied.0, StatusCode::NOT_FOUND);
    assert_eq!(denied, missing);
    assert!(audit_events(&world).await.iter().any(|entry| entry.action
        == synveda_audit::AuditAction::AuthzDecision.as_str()
        && entry.outcome == synveda_audit::Outcome::Deny.as_str()));
}

#[tokio::test]
async fn artifact_cleanup_pages_and_fifth_attempt_fence_stale_owners() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-many-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    for index in 0..102 {
        artifact_knowledge(
            &world,
            &source,
            &format!("artifact-many-knowledge-{index}"),
            false,
        )
        .await;
    }
    let removal = remove(&world, &first, &source, "forget", "artifact-many-forget").await;
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
    sqlx::query("select set_config('synveda.artifact_cleanup_operation',$1,true),set_config('synveda.artifact_cleanup_attempt','5',true)")
        .bind(operation.to_string()).execute(&mut *forged).await.unwrap();
    assert!(
        !synveda_store::source_control::available(
            &mut forged,
            world.tenant_id,
            "import_artifact",
            source["id"].as_str().unwrap().parse().unwrap()
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
        "select * from synveda_artifact_cleanup_chunk($1,$2,'fifth-source-worker',5,null)",
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
async fn artifact_withdrawal_waits_for_admitted_reader() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let first = plan(&world, "artifact-gate-plan", 3, false).await;
    let source = artifact(&first, "procedures/retries.md");
    let mut reader = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .unwrap();
    let holder = tenant_fixture::backend_pid(&mut reader).await;
    let app = world.app.clone();
    let token = world.alice_token.clone();
    let path = format!(
        "/v1/okf/imports/{}/artifacts/{}/withdraw",
        first["job"]["id"].as_str().unwrap(),
        source["id"].as_str().unwrap()
    );
    let hash = source["content_hash"].clone();
    let pending = tokio::spawn(async move {
        call(
            &app,
            "POST",
            &path,
            &token,
            Some("artifact-gate-withdraw"),
            Some(json!({"expected_content_hash":hash,"reason":"Retire the inspected artifact"})),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        loop {
            let blocked: bool = sqlx::query_scalar("select exists(select 1 from pg_catalog.pg_stat_activity where $1=any(pg_catalog.pg_blocking_pids(pid)))")
                .bind(holder).fetch_one(&world.state.pool).await.unwrap();
            if blocked { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("artifact writer did not wait at admission");
    assert!(!pending.is_finished());
    assert!(
        synveda_store::source_control::available(
            &mut reader,
            world.tenant_id,
            "import_artifact",
            source["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
    );
    reader.commit().await.unwrap();
    let (status, removed) = pending.await.unwrap();
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    assert_eq!(removed["outcome"], "applied", "{removed}");
}

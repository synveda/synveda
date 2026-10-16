//! SKIL-6 / SKIL-5 / MEM-8 ordinary-role public learning acceptance.
#[path = "support/context_world.rs"]
mod context_world;
#[path = "support/procedure_world.rs"]
mod procedure_world;
use context_world::*;
use procedure_world::*;
use synveda_types::CaptureCandidateId;

#[tokio::test]
async fn client_reports_and_raw_reserved_events_never_become_runner_evidence() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.world.alice_token, "client-report").await;
    assert_eq!(event["payload"]["evidence_class"], "client_reported");
    let (status, denied) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/sessions/{}/procedure-improvements",
            l.world.alice_session
        ),
        &l.world.alice_token,
        None,
        Some(improvement_body(&l, &event)),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{denied}");
    for kind in ["task.outcome", "procedure.improvement"] {
        let (status,body)=call(&l.world.app,"POST",&format!("/v1/sessions/{}/events",l.world.alice_session),&l.world.alice_token,Some(kind),Some(json!({"events":[{"client_event_id":kind,"event_type":kind,"occurred_at":"2026-10-14T00:00:00Z","payload":{"evidence_class":"authenticated_runner"}}]}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}
#[tokio::test]
async fn exact_candidate_qualification_review_and_historical_retry_use_one_public_workflow() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.runner, "owned-runner").await;
    assert_eq!(event["payload"]["evidence_class"], "authenticated_runner");
    let (request, candidate) = candidate(&l, &event).await;
    let id = candidate["id"].as_str().unwrap();
    let (status, denied) = call(
        &l.world.app,
        "POST",
        &format!("/v1/capture-candidates/{id}/accept"),
        &l.world.alice_token,
        Some("invalid-generic-accept"),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{denied}");
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    assert!(
        synveda_store::capture::get_decision(
            &mut *tx,
            l.world.tenant_id,
            id.parse::<CaptureCandidateId>().unwrap()
        )
        .await
        .unwrap()
        .is_none()
    );
    tx.commit().await.unwrap();
    let attempt = freeze(&l, &candidate).await;
    let path = format!("/v1/capture-candidates/{id}/procedure-submit");
    let body = json!({"candidate_hash":candidate["content_hash"]});
    let (status, denied) = call(
        &l.world.app,
        "POST",
        &path,
        &l.world.alice_token,
        Some("qualified-submit"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
    // Owned delayed-ack and unknown-commit cases execute against the exact
    // frozen content. Their answers are never supplied to retrieval/generation.
    assert!(
        candidate["content"]["body_markdown"]
            .as_str()
            .unwrap()
            .contains("200 ms")
    );
    assert!(
        candidate["content"]["body_markdown"]
            .as_str()
            .unwrap()
            .contains("Never retry an unknown commit outcome")
    );
    results(&l, &attempt, true).await;
    let (status, pending) = call(
        &l.world.app,
        "POST",
        &path,
        &l.world.alice_token,
        Some("qualified-submit"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(
        pending["candidate"]["resulting_outcome"], "pending_review",
        "{pending}"
    );
    let change = pending["candidate"]["resulting_change_id"]
        .as_str()
        .unwrap();
    review(&l, change).await;
    let (status, item) = call(
        &l.world.app,
        "GET",
        &format!("/v1/knowledge/{}", l.procedure["item_id"].as_str().unwrap()),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{item}");
    assert_eq!(
        item["current_revision"]["content_hash"],
        candidate["content_hash"]
    );
    let (status, retry) = call(
        &l.world.app,
        "POST",
        &path,
        &l.world.alice_token,
        Some("qualified-submit"),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{retry}");
    assert_eq!(retry["candidate"]["resulting_change_id"], change);
    assert_eq!(retry["replayed"], true);
    let old = observe(&l, &l.runner, "owned-runner").await;
    assert_eq!(old["id"], event["id"]);
    let (status, retry) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/sessions/{}/procedure-improvements",
            l.world.alice_session
        ),
        &l.world.alice_token,
        None,
        Some(improvement_body(&l, &event)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["event"]["id"], request["event"]["id"]);
}
#[tokio::test]
async fn failed_and_changed_fixed_results_stay_unpublishable() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.runner, "failed-runner").await;
    let (_, candidate) = candidate(&l, &event).await;
    let attempt = freeze(&l, &candidate).await;
    results(&l, &attempt, false).await;
    let (status,changed)=call(&l.world.app,"POST",&format!("/v1/procedure-evaluations/{}/results",attempt["attempt_id"].as_str().unwrap()),&l.evaluator,None,Some(json!({"candidate_hash":attempt["candidate_hash"],"specification_hash":attempt["specification_hash"],"results":observations(&l,true)}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{changed}");
    let (status, denied) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/procedure-submit",
            candidate["id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        Some("failed-submit"),
        Some(json!({"candidate_hash":candidate["content_hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
}

#[tokio::test]
async fn runner_evidence_revocation_blocks_pending_apply_only_for_the_revoked_actor() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.runner, "revocation-runner").await;
    let (_, c) = candidate(&l, &event).await;
    let a = freeze(&l, &c).await;
    results(&l, &a, true).await;
    let path = format!(
        "/v1/capture-candidates/{}/procedure-submit",
        c["id"].as_str().unwrap()
    );
    let (status, p) = call(
        &l.world.app,
        "POST",
        &path,
        &l.world.alice_token,
        Some("revocation-pending"),
        Some(json!({"candidate_hash":c["content_hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    let change = p["candidate"]["resulting_change_id"].as_str().unwrap();
    let (status, proposal) = call(
        &l.world.app,
        "GET",
        &format!("/v1/proposals/{change}"),
        &l.world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{proposal}");
    let (status, approved) = call(
        &l.world.app,
        "POST",
        &format!("/v1/proposals/{change}/approve"),
        &l.world.bob_token,
        None,
        Some(json!({"expected_commit":proposal["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let source = format!(
        "{BLANKET}\nforbid (principal == Synveda::Principal::\"{}/{ALICE}\", action == Synveda::Action::\"SessionRead\", resource);",
        l.world.tenant_id
    );
    l.world
        .install_policy(
            "learning-review",
            2,
            &source,
            PackConfig {
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
            },
        )
        .await;
    let (status, denied) = call(
        &l.world.app,
        "POST",
        &format!("/v1/proposals/{change}/apply"),
        &l.world.alice_token,
        None,
        Some(json!({})),
    )
    .await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{denied}"
    );
    assert!(!denied.to_string().contains("200 ms"));
    let (status, independent) = call(
        &l.world.app,
        "GET",
        &format!("/v1/proposals/{change}"),
        &l.world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{independent}");
    assert!(independent.to_string().contains("200 ms"));
    let (status, unchanged) = call(
        &l.world.app,
        "GET",
        &format!("/v1/knowledge/{}", l.procedure["item_id"].as_str().unwrap()),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(
        unchanged["current_revision"]["id"],
        l.procedure["revision_id"]
    );
}

#[tokio::test]
async fn forgetting_baseline_scrubs_learning_copies_pending_proposals_and_fixed_results() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let (status, independent) = call(&l.world.app,"POST","/v1/knowledge",&l.world.alice_token,Some("learning-independent"),Some(json!({"scope_id":l.world.project_scope,"project_id":l.world.project_id,"knowledge_type":"convention","origin":"authored","content":content("Independent Iris convention","Keep traceparent independently.",None),"sources":[{"scope_id":l.world.project_scope,"source_type":"manual","metadata":{}}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{independent}");
    if independent["outcome"] == "pending_review" {
        review(&l, independent["change_id"].as_str().unwrap()).await;
    }
    let event = observe(&l, &l.runner, "forget-runner").await;
    let (request, c) = candidate(&l, &event).await;
    let a = freeze(&l, &c).await;
    results(&l, &a, true).await;
    let (status, p) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/procedure-submit",
            c["id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        Some("forget-pending"),
        Some(json!({"candidate_hash":c["content_hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{p}");
    let item = l.procedure["item_id"].as_str().unwrap();
    let (status,removed)=call(&l.world.app,"DELETE",&format!("/v1/knowledge/{item}"),&l.world.alice_token,Some("forget-procedure-baseline"),Some(json!({"mode":"forget","expected_revision_id":l.procedure["revision_id"],"reason":"Remove this owned baseline and its retained copies"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    if removed["outcome"] == "pending_review" {
        review(&l, removed["change_id"].as_str().unwrap()).await;
    }
    for path in [
        format!("/v1/knowledge/{item}"),
        format!(
            "/v1/sessions/{}/events/{}",
            l.world.alice_session,
            event["id"].as_str().unwrap()
        ),
        format!(
            "/v1/sessions/{}/events/{}",
            l.world.alice_session,
            request["event"]["id"].as_str().unwrap()
        ),
        format!("/v1/capture-candidates/{}", c["id"].as_str().unwrap()),
        format!(
            "/v1/procedure-evaluations/{}",
            a["attempt_id"].as_str().unwrap()
        ),
    ] {
        let (status, v) = call(&l.world.app, "GET", &path, &l.world.alice_token, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {v}");
        assert!(!v.to_string().contains("200 ms"));
    }
    let (status, survivor) = call(
        &l.world.app,
        "GET",
        &format!(
            "/v1/knowledge/{}",
            independent["knowledge_item_id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{survivor}");
    assert!(survivor.to_string().contains("traceparent"));
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    let payload = synveda_store::sessions::event(
        &mut *tx,
        l.world.tenant_id,
        l.world.alice_session.parse().unwrap(),
        request["event"]["id"].as_str().unwrap().parse().unwrap(),
    )
    .await
    .unwrap();
    assert!(
        payload.is_none_or(|e| !e.payload.to_string().contains("200 ms")),
        "retained event must be physically scrubbed"
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn fixed_attempt_bound_preserves_terminal_retry_and_unknown_baseline_never_qualifies() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let path = format!(
        "/v1/configurations/effective?scope_id={}",
        l.world.project_scope
    );
    let (status, effective) =
        call(&l.world.app, "GET", &path, &l.world.alice_token, None, None).await;
    assert_eq!(status, StatusCode::OK, "{effective}");
    let mut document = effective["document"].clone();
    let mut suites = Vec::new();
    for i in 0..8 {
        let mut suite = l.suite.clone();
        suite["suite_id"] = json!(format!("fixed-suite-{i}"));
        suites.push(suite);
    }
    document["procedure_learning"]["evaluation_suites"] = json!(suites);
    let config_path = format!(
        "/v1/configurations/{}/versions",
        effective["artifact_id"].as_str().unwrap()
    );
    let (status, configured) = call(
        &l.world.app,
        "POST",
        &config_path,
        &l.world.alice_token,
        Some("fixed-eight-suites"),
        Some(json!({"expected_current_version_id":effective["version_id"],"document":document})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{configured}");
    assert_eq!(configured["outcome"], "applied");
    let outcome = observe(&l, &l.runner, "fixed-bound-outcome").await;
    let mut oversized = improvement_body(&l, &outcome);
    oversized["proposed_append"] = json!("x".repeat(1025));
    let (status, refused) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/sessions/{}/procedure-improvements",
            l.world.alice_session
        ),
        &l.world.alice_token,
        None,
        Some(oversized),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (_, candidate) = candidate(&l, &outcome).await;
    let evaluate = format!(
        "/v1/capture-candidates/{}/procedure-evaluations",
        candidate["id"].as_str().unwrap()
    );
    let mut terminal = Value::Null;
    for i in 0..8 {
        let (status,attempt)=call(&l.world.app,"POST",&evaluate,&l.world.alice_token,None,Some(json!({"candidate_hash":candidate["content_hash"],"suite_id":format!("fixed-suite-{i}")}))).await;
        assert_eq!(status, StatusCode::OK, "{attempt}");
        terminal = attempt;
    }
    let (status, retry) = call(
        &l.world.app,
        "POST",
        &evaluate,
        &l.world.alice_token,
        None,
        Some(json!({"candidate_hash":candidate["content_hash"],"suite_id":"fixed-suite-7"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(retry["attempt_id"], terminal["attempt_id"]);
    let mut observations = observations(&l, true);
    observations[0]["baseline"] = json!("unknown");
    let (status,result)=call(&l.world.app,"POST",&format!("/v1/procedure-evaluations/{}/results",terminal["attempt_id"].as_str().unwrap()),&l.evaluator,None,Some(json!({"candidate_hash":terminal["candidate_hash"],"specification_hash":terminal["specification_hash"],"results":observations}))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["qualified"], false);
    document["procedure_learning"]["evaluation_suites"][7]["configuration_digest"] =
        json!("a".repeat(64));
    let (status, changed) = call(
        &l.world.app,
        "POST",
        &config_path,
        &l.world.alice_token,
        Some("fixed-ninth-spec"),
        Some(json!({"expected_current_version_id":configured["version_id"],"document":document})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{changed}");
    let (status, refused) = call(
        &l.world.app,
        "POST",
        &evaluate,
        &l.world.alice_token,
        None,
        Some(json!({"candidate_hash":candidate["content_hash"],"suite_id":"fixed-suite-7"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

#[tokio::test]
async fn ordinary_role_sql_rejects_plaintext_fields_in_retained_evaluation_metadata() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.runner, "closed-sql-outcome").await;
    let (_, candidate) = candidate(&l, &event).await;
    let frozen = freeze(&l, &candidate).await;
    let id = frozen["attempt_id"].as_str().unwrap().parse().unwrap();
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    let attempt = synveda_store::procedure_learning::attempt(&mut tx, l.world.tenant_id, id)
        .await
        .unwrap()
        .unwrap();
    let mut specification = attempt.specification.clone();
    specification["extra"] = json!("must never survive forgetting as evaluation metadata");
    let error = synveda_store::procedure_learning::freeze(
        &mut tx,
        l.world.tenant_id,
        &synveda_store::procedure_learning::FreezeEvaluation {
            candidate: candidate["id"].as_str().unwrap().parse().unwrap(),
            candidate_hash: &attempt.candidate_hash,
            request_hash: &attempt.request_hash,
            suite_id: &attempt.suite_id,
            evaluator: synveda_types::IdentityId::from_uuid(attempt.evaluator_id),
            configuration_hash: &attempt.configuration_hash,
            specification_hash: &"a".repeat(64),
            specification: &specification,
        },
    )
    .await
    .err()
    .expect("closed retained metadata must be rejected");
    assert!(
        error.to_string().contains("invalid frozen evaluation"),
        "{error}"
    );
    tx.rollback().await.unwrap();
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    let mut observations = observations(&l, true);
    observations[0]["extra"] = json!("must never survive forgetting as result metadata");
    let error = synveda_store::procedure_learning::record_result(
        &mut tx,
        l.world.tenant_id,
        id,
        synveda_types::IdentityId::from_uuid(attempt.evaluator_id),
        &"b".repeat(64),
        true,
        &observations,
    )
    .await
    .err()
    .expect("closed retained metadata must be rejected");
    assert!(
        error.to_string().contains("fixed case identity mismatch"),
        "{error}"
    );
    tx.rollback().await.unwrap();
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    assert_eq!(
        synveda_store::procedure_learning::attempts(
            &mut tx,
            l.world.tenant_id,
            candidate["id"].as_str().unwrap().parse().unwrap()
        )
        .await
        .unwrap()
        .len(),
        1
    );
    assert!(
        synveda_store::procedure_learning::result(&mut tx, l.world.tenant_id, id)
            .await
            .unwrap()
            .is_none()
    );
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn interrupted_capture_decision_resumes_an_already_applied_exact_knowledge_receipt() {
    let _guard = serial().await;
    let Some(l) = setup().await else { return };
    let event = observe(&l, &l.runner, "recovery-runner").await;
    let (request, candidate) = candidate(&l, &event).await;
    let attempt = freeze(&l, &candidate).await;
    results(&l, &attempt, true).await;
    let candidate_id = candidate["id"].as_str().unwrap().parse().unwrap();
    let payload = json!({"candidate_hash":candidate["content_hash"]});
    let key = "interrupted-exact-submit";
    // Retain precisely the intent that survives the crash window. No product
    // effect is seeded: its publication below takes the actual API and review.
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    synveda_store::capture::begin_decision(
        &mut tx,
        l.world.tenant_id,
        candidate_id,
        synveda_types::capture::CaptureDecisionAction::EditAndAccept,
        ALICE,
        key,
        &payload,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (status,published)=call(&l.world.app,"PATCH",&format!("/v1/knowledge/{}",l.procedure["item_id"].as_str().unwrap()),&l.world.alice_token,Some("recovery-authorized-effect"),Some(json!({"expected_revision_id":l.procedure["revision_id"],"content":candidate["content"],"sources":[{"scope_id":l.world.project_scope,"source_type":"session_event","session_event_id":request["event"]["id"],"metadata":{"capture_candidate_id":candidate["id"]}}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{published}");
    assert_eq!(published["outcome"], "pending_review");
    review(&l, published["change_id"].as_str().unwrap()).await;
    let change = published["change_id"]
        .as_str()
        .unwrap()
        .parse::<synveda_types::ProposalId>()
        .unwrap();
    let mut tx = rls::begin_tenant_tx(&l.world.state.pool, l.world.tenant_id)
        .await
        .unwrap();
    let digest = blake3::hash(
        synveda_types::json::canonicalise(&payload)
            .to_string()
            .as_bytes(),
    );
    synveda_store::idempotency::remember(
        &mut *tx,
        l.world.tenant_id,
        ALICE,
        "capture.candidate.edit_accept",
        key,
        digest.as_bytes(),
        change.as_uuid(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    for _ in 0..2 {
        let (status, recovered) = call(
            &l.world.app,
            "POST",
            &format!("/v1/capture-candidates/{candidate_id}/procedure-submit"),
            &l.world.alice_token,
            Some(key),
            Some(payload.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{recovered}");
        assert_eq!(
            recovered["candidate"]["resulting_change_id"],
            change.to_string()
        );
        assert_eq!(
            recovered["candidate"]["content_hash"],
            candidate["content_hash"]
        );
    }
    let (status, history) = call(
        &l.world.app,
        "GET",
        &format!(
            "/v1/knowledge/{}/history",
            l.procedure["item_id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    assert_eq!(
        history["revisions"].as_array().unwrap().len(),
        2,
        "{history}"
    );
}

#[tokio::test]
async fn held_historical_descendant_blocks_baseline_forget_before_any_scrub() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (a, ar) = create_knowledge(
        &world,
        "held-historical-a",
        world.project_scope,
        Some(&world.project_id),
        None,
        "Independent execution constraint",
        "Iris provider reconciliation remains an independent constraint.",
        None,
    )
    .await;
    let mut held = content(
        "Held retry Procedure",
        "Retry at most 3 times.\n\nNever retry an unknown commit outcome.",
        None,
    );
    held["metadata"]["legal_hold"] = json!(true);
    let (status,b)=call(&world.app,"POST","/v1/knowledge",&world.alice_token,Some("held-historical-b"),Some(json!({"scope_id":world.project_scope,"project_id":world.project_id,"knowledge_type":"procedure","origin":"authored","content":held,"sources":[{"scope_id":world.project_scope,"source_type":"manual","metadata":{}}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{b}");
    let (status, b) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{}", b["knowledge_item_id"].as_str().unwrap()),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{b}");
    let l=setup_world(world,Some(json!({"item_id":b["id"],"revision_id":b["current_revision"]["id"],"content_hash":b["current_revision"]["content_hash"]}))).await;
    let (status,run)=call(&l.world.app,"POST",&format!("/v1/sessions/{}/context-runs",l.world.alice_session),&l.world.alice_token,Some("held-historical-context"),Some(json!({"budget_tokens":2400,"required_knowledge_revisions":[{"item_id":a,"revision_id":ar},{"item_id":l.procedure["item_id"],"revision_id":l.procedure["revision_id"]}]}))).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let mut body = outcome_body(&l, "held-historical-observation");
    body["context"] = json!({"context_run_id":run["id"],"block_hash":run["block_hash"]});
    let (status, event) = call(
        &l.world.app,
        "POST",
        &format!("/v1/sessions/{}/outcomes", l.world.alice_session),
        &l.runner,
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{event}");
    let (request, c) = candidate(&l, &event["event"]).await;
    let attempt = freeze(&l, &c).await;
    results(&l, &attempt, true).await;
    let (status, pending) = call(
        &l.world.app,
        "POST",
        &format!(
            "/v1/capture-candidates/{}/procedure-submit",
            c["id"].as_str().unwrap()
        ),
        &l.world.alice_token,
        Some("held-historical-publish"),
        Some(json!({"candidate_hash":c["content_hash"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    review(
        &l,
        pending["candidate"]["resulting_change_id"]
            .as_str()
            .unwrap(),
    )
    .await;
    let (status,removed)=call(&l.world.app,"DELETE",&format!("/v1/knowledge/{a}"),&l.world.alice_token,Some("held-historical-forget-a"),Some(json!({"mode":"forget","expected_revision_id":ar,"reason":"Attempt exact owned constraint removal while a descendant is held"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
    if removed["outcome"] == "pending_review" {
        let change = removed["change_id"].as_str().unwrap();
        let (status, p) = call(
            &l.world.app,
            "GET",
            &format!("/v1/proposals/{change}"),
            &l.world.bob_token,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{p}");
        let (status, p) = call(
            &l.world.app,
            "POST",
            &format!("/v1/proposals/{change}/approve"),
            &l.world.bob_token,
            None,
            Some(json!({"expected_commit":p["commit"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{p}");
        let (status, p) = call(
            &l.world.app,
            "POST",
            &format!("/v1/proposals/{change}/apply"),
            &l.world.alice_token,
            None,
            Some(json!({})),
        )
        .await;
        assert!(
            matches!(status, StatusCode::BAD_REQUEST | StatusCode::CONFLICT)
                || p["outcome"] == "rejected",
            "{status}: {p}"
        );
    } else {
        assert_eq!(removed["outcome"], "rejected", "{removed}");
    }
    for path in [
        format!("/v1/knowledge/{a}"),
        format!("/v1/knowledge/{}", l.procedure["item_id"].as_str().unwrap()),
        format!(
            "/v1/sessions/{}/events/{}",
            l.world.alice_session,
            request["event"]["id"].as_str().unwrap()
        ),
        format!("/v1/context-runs/{}", run["id"].as_str().unwrap()),
    ] {
        let (status, v) = call(&l.world.app, "GET", &path, &l.world.alice_token, None, None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {v}");
    }
    let (status, learned) = call(
        &l.world.app,
        "GET",
        &format!("/v1/knowledge/{}", l.procedure["item_id"].as_str().unwrap()),
        &l.world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{learned}");
    let text = learned["current_revision"]["body_markdown"]
        .as_str()
        .unwrap();
    let state_path = format!("/v1/sessions/{}/task-state", l.world.alice_session);
    let (status,old)=call(&l.world.app,"POST",&state_path,&l.world.alice_token,None,Some(json!({"schema_version":1,"client_event_id":"held-old-state","statements":[{"kind":"constraint","evidence":{"source_kind":"knowledge_revision","item_id":learned["id"],"revision_id":learned["current_revision"]["id"],"content_hash":learned["current_revision"]["content_hash"],"span":{"start":0,"end":text.len()}}}]}))).await;
    assert_eq!(status, StatusCode::OK, "{old}");
    l.world.install_policy("learning-review",2,&format!("{BLANKET}\nforbid (principal, action == Synveda::Action::\"KnowledgeRead\", resource == Synveda::KnowledgeItem::\"{a}\");"),PackConfig::default()).await;
    let clean = "Iris next objective uses independently admitted evidence.";
    let (status,event)=call(&l.world.app,"POST",&format!("/v1/sessions/{}/events",l.world.alice_session),&l.world.alice_token,Some("held-clean-source"),Some(json!({"events":[{"client_event_id":"held-clean-source","event_type":"message.user","occurred_at":"2026-10-14T00:00:00Z","payload":{"text":clean}}]}))).await;
    assert_eq!(status, StatusCode::OK, "{event}");
    let event = &event["events"][0]["event"];
    let (status,replaced)=call(&l.world.app,"POST",&state_path,&l.world.alice_token,None,Some(json!({"schema_version":1,"client_event_id":"held-independent-state","expected_previous":null,"statements":[{"kind":"objective","evidence":{"source_kind":"session_event","event_id":event["id"],"payload_hash":event["payload_hash"],"field":"text","span":{"start":0,"end":clean.len()}}}]}))).await;
    assert_eq!(status, StatusCode::OK, "{replaced}");
    assert!(replaced["event"]["payload"].to_string().contains(clean));
    assert!(!replaced["event"]["payload"].to_string().contains("200 ms"));
}

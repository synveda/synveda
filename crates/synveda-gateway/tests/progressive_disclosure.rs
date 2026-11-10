//! Progressive context disclosure acceptance.
#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;
use synveda_types::configuration::ContextDisclosureMode;

const BODY_MARK: &str = "BODYMARK";
const MEMBER_CONFIDENTIAL_DENIED: &str = r#"
permit (principal, action, resource) when { resource in principal.tenant };
forbid (principal, action == Synveda::Action::"KnowledgeRead", resource)
when { context.sensitivity == "confidential" && !context.roles.contains("administrator") };
"#;

fn distinct_words(seed: &str) -> String {
    [
        "alpha", "bravo", "cobalt", "dune", "ember", "fjord", "garnet", "harbor",
    ]
    .iter()
    .map(|word| format!("{word}{seed}"))
    .collect::<Vec<_>>()
    .join(" ")
}

fn authored(title: &str, summary: &str, body: &str) -> Value {
    let mut value = content(title, body, None);
    value["summary"] = json!(summary);
    value
}

async fn zephyr_note(world: &World, key: &str, body_mark: &str) -> (String, String) {
    let words = distinct_words(key);
    create_knowledge_with_content(
        world,
        key,
        world.project_scope,
        Some(&world.project_id),
        None,
        authored(
            &format!("Zephyr {words}"),
            &format!("Zephyr summary {words}"),
            &format!("{body_mark} Zephyr body {words}"),
        ),
    )
    .await
}

async fn deny_confidential_to_members(world: &World) {
    world
        .install_policy(
            "disclosure-denial",
            1,
            MEMBER_CONFIDENTIAL_DENIED,
            PackConfig::default(),
        )
        .await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin policy Configuration");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "disclosure-denial").await;
    tx.commit().await.expect("commit policy Configuration");
}

async fn run_with(
    world: &World,
    token: &str,
    session: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/context-runs"),
        token,
        Some(key),
        Some(body),
    )
    .await
}

async fn manifest(world: &World, token: &str, session: &str, key: &str, body: Value) -> Value {
    let (status, run) = run_with(world, token, session, key, body).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(run["disclosure"], "manifest", "{run}");
    run
}

fn rendered(run: &Value) -> &str {
    run["rendered"].as_str().expect("rendered block")
}

fn entries(run: &Value) -> Vec<Value> {
    rendered(run)
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .map(|line| serde_json::from_str(line).expect("entry is one JSON object"))
        .collect()
}

fn entry_ids(run: &Value) -> Vec<String> {
    let mut ids: Vec<String> = entries(run)
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_manifest_names_knowledge_and_carries_no_bodies() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (first, _) = zephyr_note(&world, "manifesta", BODY_MARK).await;
    let (second, _) = zephyr_note(&world, "manifestb", BODY_MARK).await;

    let run = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "manifest-run",
        json!({"query": "zephyr", "disclosure": "manifest", "budget_tokens": 1500}),
    )
    .await;
    let block = rendered(&run);
    assert!(!block.contains(BODY_MARK), "{block}");
    assert!(
        block.contains(&format!(
            "\"manifest_id\":\"{}\"",
            run["id"].as_str().expect("run id")
        )),
        "{block}"
    );
    assert!(block.contains("\"truncated\":false"), "{block}");
    assert!(block.contains("an estimate"), "{block}");
    assert!(!block.contains("[Synveda Knowledge:"), "{block}");

    let found = entries(&run);
    let mut expected = vec![first.clone(), second.clone()];
    expected.sort();
    assert_eq!(entry_ids(&run), expected, "{block}");
    for entry in &found {
        assert_eq!(entry["kind"], "manifest_entry", "{entry}");
        assert_eq!(entry["type"], "convention", "{entry}");
        assert_eq!(entry["channel"], "current_knowledge", "{entry}");
        assert_eq!(entry["sensitivity"], "internal", "{entry}");
        assert!(entry["expand_cost_tokens"].as_u64().unwrap() > 0, "{entry}");
        assert!(
            entry["title"].as_str().unwrap().chars().count() <= 80,
            "{entry}"
        );
        assert!(
            entry["description"].as_str().unwrap().chars().count() <= 120,
            "{entry}"
        );
    }
    assert!(run["tokens"].as_i64().unwrap() <= run["budget_tokens"].as_i64().unwrap());

    let read = detail(&world, &world.alice_token, run["id"].as_str().unwrap()).await;
    let selections = read["selections"].as_array().expect("selections");
    assert_eq!(selections.len(), 2, "{read}");
    for selection in selections {
        assert!(
            selection["reason_codes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "manifest_entry"),
            "{selection}"
        );
    }
}

#[tokio::test]
async fn full_delivery_still_renders_bodies() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    zephyr_note(&world, "fulla", BODY_MARK).await;
    let (status, run) = run_with(
        &world,
        &world.alice_token,
        &world.alice_session,
        "full-run",
        json!({"query": "zephyr"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(run["disclosure"], "full", "{run}");
    assert_eq!(run["retrieval_version"], "knowledge-planner-v2", "{run}");
    assert!(rendered(&run).contains(BODY_MARK), "{run}");
    assert!(!rendered(&run).contains("manifest_id"), "{run}");
    assert!(rendered(&run).contains("[Synveda Knowledge:"), "{run}");
}

#[tokio::test]
async fn a_request_cannot_widen_a_governed_manifest() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    zephyr_note(&world, "governa", BODY_MARK).await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin Configuration");
    let root = scopes::tenant_root(&mut *tx, world.tenant_id)
        .await
        .expect("read root")
        .expect("root exists");
    configuration_support::set_disclosure_mode(
        &mut tx,
        world.tenant_id,
        root.id,
        ContextDisclosureMode::Manifest,
    )
    .await;
    tx.commit().await.expect("commit Configuration");

    let run = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "govern-run",
        json!({"query": "zephyr", "disclosure": "full"}),
    )
    .await;
    assert!(!rendered(&run).contains(BODY_MARK), "{run}");
}

#[tokio::test]
async fn required_revisions_stay_verbatim_inside_a_manifest() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (required, revision) = zephyr_note(&world, "reqa", BODY_MARK).await;
    let (other, _) = zephyr_note(&world, "reqb", "OTHERMARK").await;
    let run = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "req-run",
        json!({
            "query": "zephyr",
            "disclosure": "manifest",
            "required_knowledge_revisions": [{"item_id": required, "revision_id": revision}],
        }),
    )
    .await;
    assert!(rendered(&run).contains(BODY_MARK), "{run}");
    assert!(!rendered(&run).contains("OTHERMARK"), "{run}");
    assert!(entry_ids(&run).contains(&other), "{run}");
}

#[tokio::test]
async fn truncation_is_stated_and_points_at_recall() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    for index in 0..12 {
        zephyr_note(&world, &format!("trunc{index}"), "NOTE").await;
    }
    let run = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "trunc-run",
        json!({"query": "zephyr", "disclosure": "manifest", "budget_tokens": 400}),
    )
    .await;
    let block = rendered(&run);
    assert!(block.contains("\"truncated\":true"), "{block}");
    assert!(block.contains("recall tool"), "{block}");
    assert!(!entries(&run).is_empty(), "{block}");
    assert!(entries(&run).len() < 12, "{block}");
    assert!(run["tokens"].as_i64().unwrap() <= 400, "{run}");
}

#[tokio::test]
async fn an_unauthorised_principal_sees_no_trace_in_a_manifest() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    deny_confidential_to_members(&world).await;
    let (visible, _) = zephyr_note(&world, "tracevisible", "NOTE").await;
    let request = json!({"query": "zephyr", "disclosure": "manifest", "budget_tokens": 1500});
    let baseline = manifest(
        &world,
        &world.bob_token,
        &world.bob_session,
        "trace-baseline",
        request.clone(),
    )
    .await;
    assert_eq!(entry_ids(&baseline), vec![visible.clone()], "{baseline}");

    let words = distinct_words("tracehidden");
    let mut restricted = authored(
        &format!("Zephyr HIDDENTITLE {words}"),
        &format!("Zephyr HIDDENSUMMARY {words}"),
        &format!("Zephyr HIDDENBODY {words}"),
    );
    restricted["sensitivity"] = json!("confidential");
    let (hidden, _) = create_knowledge_with_content(
        &world,
        "trace-hidden",
        world.project_scope,
        Some(&world.project_id),
        None,
        restricted,
    )
    .await;

    let control = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "trace-control",
        request.clone(),
    )
    .await;
    assert!(entry_ids(&control).contains(&hidden), "{control}");

    let denied = manifest(
        &world,
        &world.bob_token,
        &world.bob_session,
        "trace-denied",
        request,
    )
    .await;
    assert_eq!(entry_ids(&denied), vec![visible], "{denied}");
    let text = denied.to_string();
    assert!(!text.contains(&hidden), "{text}");
    assert!(!text.contains("HIDDEN"), "{text}");
    assert!(denied.get("policy_exclusion_message").is_none(), "{denied}");
    assert_eq!(denied["candidate_count"], baseline["candidate_count"]);
    assert_eq!(denied["selection_count"], baseline["selection_count"]);
    assert!(
        rendered(&denied).contains("\"truncated\":false"),
        "{denied}"
    );
}

#[tokio::test]
async fn a_replayed_manifest_returns_the_same_delivery() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    zephyr_note(&world, "replaya", "NOTE").await;
    let request = json!({"query": "zephyr", "disclosure": "manifest"});
    let first = manifest(
        &world,
        &world.alice_token,
        &world.alice_session,
        "replay-run",
        request.clone(),
    )
    .await;
    assert_eq!(entries(&first).len(), 1, "{first}");
    let (status, second) = run_with(
        &world,
        &world.alice_token,
        &world.alice_session,
        "replay-run",
        request,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["id"], first["id"]);
    assert_eq!(second["rendered"], first["rendered"]);
}

async fn expand(world: &World, token: &str, session: &str, body: Value) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/knowledge-expand"),
        token,
        None,
        Some(body),
    )
    .await
}

async fn alice_manifest(world: &World, key: &str) -> Value {
    manifest(
        world,
        &world.alice_token,
        &world.alice_session,
        key,
        json!({"query": "zephyr", "disclosure": "manifest"}),
    )
    .await
}

async fn expansion_events(world: &World, session: &str) -> Vec<Value> {
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    let rows: Vec<Value> = sqlx::query_scalar(
        "select payload from session_events
          where tenant_id = $1 and session_id = $2
            and event_type = 'context.knowledge_expanded' order by sequence",
    )
    .bind(world.tenant_id.as_uuid())
    .bind(uuid::Uuid::parse_str(session).expect("session uuid"))
    .fetch_all(&mut *tx)
    .await
    .expect("read expansion events");
    tx.commit().await.expect("commit expansion read");
    rows
}

fn refusal_shape(item: &Value) -> Value {
    let mut shape = item.clone();
    shape.as_object_mut().expect("item").remove("id");
    shape
}

#[tokio::test]
async fn expansion_serves_the_body_and_records_one_event() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, revision) = zephyr_note(&world, "expanda", BODY_MARK).await;
    let run = alice_manifest(&world, "expand-run").await;
    let forged = uuid::Uuid::now_v7().to_string();
    let body = json!({"ids": [forged, item], "manifest_id": run["id"]});

    let (status, first) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["resolve"], "pinned", "{first}");
    assert_eq!(first["items"][0]["id"], forged, "request order is kept");
    assert_eq!(first["items"][0]["status"], "not_found_at_as_of");
    assert_eq!(first["items"][1]["status"], "served", "{first}");
    let served = &first["items"][1];
    assert!(
        served["knowledge"]["current_revision"]["body_markdown"]
            .as_str()
            .unwrap()
            .contains(BODY_MARK),
        "{served}"
    );
    assert!(
        served["sources"]
            .as_array()
            .is_some_and(|all| !all.is_empty())
    );
    assert!(served["tokens"].as_u64().unwrap() > 0, "{served}");
    assert_eq!(first["tokens_returned"], served["tokens"], "{first}");

    let (status, retried) = expand(&world, &world.alice_token, &world.alice_session, body).await;
    assert_eq!(status, StatusCode::OK, "{retried}");
    let events = expansion_events(&world, &world.alice_session).await;
    assert_eq!(events.len(), 1, "a retry converges: {events:?}");
    assert_eq!(events[0]["manifest_id"], run["id"]);
    assert_eq!(events[0]["items"][0]["knowledge_item_id"], item);
    assert_eq!(events[0]["items"][0]["knowledge_revision_id"], revision);
    assert!(!events[0].to_string().contains(BODY_MARK));
    assert!(!events[0].to_string().contains(&forged));

    let audited = audit_events(&world)
        .await
        .into_iter()
        .rev()
        .find(|event| event.payload["op"] == "context.knowledge_expand")
        .expect("expansion audit");
    let detail = &audited.payload["detail"];
    assert_eq!(detail["served"], 1, "{detail}");
    assert_eq!(detail["not_found_at_as_of"], 1, "{detail}");
    assert_eq!(detail["resolve"], "pinned", "{detail}");
    assert!(detail["tokens_returned"].as_u64().unwrap() > 0, "{detail}");
    assert!(!audited.payload.to_string().contains(&forged), "{detail}");
}

#[tokio::test]
async fn concurrent_twin_expansions_converge_on_one_event() {
    let _guard = serial().await;
    let Some(mut world) = admitted_world().await else {
        return;
    };
    replace_gateway_pool(&mut world, 8);
    let (item, _) = zephyr_note(&world, "twina", "NOTE").await;
    let run = alice_manifest(&world, "twin-run").await;
    let body = json!({"ids": [item], "manifest_id": run["id"]});
    let attempt = || {
        expand(
            &world,
            &world.alice_token,
            &world.alice_session,
            body.clone(),
        )
    };
    let (first, second, third, fourth) = tokio::join!(attempt(), attempt(), attempt(), attempt());
    for (status, response) in [first, second, third, fourth] {
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["items"][0]["status"], "served", "{response}");
    }
    assert_eq!(
        expansion_events(&world, &world.alice_session).await.len(),
        1
    );
}

#[tokio::test]
async fn an_expansion_that_serves_nothing_records_no_event() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (status, refused) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [uuid::Uuid::now_v7()]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refused}");
    assert_eq!(refused["resolve"], "current", "{refused}");
    assert!(
        expansion_events(&world, &world.alice_session)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn every_refusal_reads_the_same() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    deny_confidential_to_members(&world).await;
    let words = distinct_words("uniformhidden");
    let mut restricted = authored(
        &format!("Zephyr {words}"),
        &format!("Zephyr summary {words}"),
        &format!("Zephyr body {words}"),
    );
    restricted["sensitivity"] = json!("confidential");
    let (denied, _) = create_knowledge_with_content(
        &world,
        "uniform-hidden",
        world.project_scope,
        Some(&world.project_id),
        None,
        restricted,
    )
    .await;
    let (private, _) = create_knowledge(
        &world,
        "uniform-private",
        world.alice_scope,
        None,
        Some(ALICE),
        "Zephyr private",
        "Zephyr private body.",
        None,
    )
    .await;
    let (archived, archived_revision) = zephyr_note(&world, "uniformarchived", "NOTE").await;
    let (status, archive) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{archived}/archive"),
        &world.alice_token,
        Some("uniform-archive"),
        Some(json!({"expected_revision_id": archived_revision, "reason": "retired"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archive}");
    let forged = uuid::Uuid::now_v7().to_string();

    let (status, response) = expand(
        &world,
        &world.bob_token,
        &world.bob_session,
        json!({"ids": [forged, denied, private, archived]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let items = response["items"].as_array().expect("items");
    assert_eq!(items.len(), 4, "{response}");
    let shape = refusal_shape(&items[0]);
    assert_eq!(shape, json!({"status": "not_found_at_as_of"}), "{response}");
    for item in items {
        assert_eq!(refusal_shape(item), shape, "{response}");
    }
    assert!(!response.to_string().contains("Zephyr"), "{response}");
}

#[tokio::test]
async fn a_foreign_or_unknown_manifest_is_the_unknown_session_response() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, _) = zephyr_note(&world, "foreigna", "NOTE").await;
    let alice_run = alice_manifest(&world, "foreign-run").await;
    let request = |manifest: Value| json!({"ids": [item], "manifest_id": manifest});

    let (forged_status, forged) = expand(
        &world,
        &world.bob_token,
        &world.bob_session,
        request(json!(uuid::Uuid::now_v7())),
    )
    .await;
    let (foreign_status, foreign) = expand(
        &world,
        &world.bob_token,
        &world.bob_session,
        request(alice_run["id"].clone()),
    )
    .await;
    assert_eq!(forged_status, StatusCode::NOT_FOUND, "{forged}");
    assert_eq!(foreign_status, StatusCode::NOT_FOUND, "{foreign}");
    assert_eq!(forged, foreign);

    let unknown_session = uuid::Uuid::now_v7().to_string();
    let (status, unknown) = expand(
        &world,
        &world.bob_token,
        &unknown_session,
        request(json!(uuid::Uuid::now_v7())),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(
        unknown.to_string().replace(&unknown_session, "ID"),
        forged.to_string().replace(&world.bob_session, "ID"),
    );

    let (status, outsider) = expand(
        &world,
        &world.mallory_token,
        &world.alice_session,
        json!({"ids": [item]}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{outsider}");
}

#[tokio::test]
async fn a_pin_refuses_what_changed_after_the_manifest() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (retired, retired_revision) = zephyr_note(&world, "rewindretired", "NOTE").await;
    let (edited, edited_revision) = zephyr_note(&world, "rewindedited", "ORIGINAL").await;
    let run = alice_manifest(&world, "rewind-run").await;

    let (status, archive) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{retired}/archive"),
        &world.alice_token,
        Some("rewind-archive"),
        Some(json!({"expected_revision_id": retired_revision, "reason": "retracted"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archive}");
    let words = distinct_words("rewindedited");
    let (status, patched) = call(
        &world.app,
        "PATCH",
        &format!("/v1/knowledge/{edited}"),
        &world.alice_token,
        Some("rewind-edit"),
        Some(json!({
            "expected_revision_id": edited_revision,
            "content": authored(
                &format!("Zephyr {words}"),
                &format!("Zephyr summary {words}"),
                &format!("REPLACEMENT Zephyr body {words}"),
            ),
            "sources": [{"scope_id": world.project_scope, "source_type": "manual"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{patched}");

    let (status, pinned) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [retired, edited], "manifest_id": run["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{pinned}");
    assert_eq!(
        pinned["items"][0]["status"], "not_found_at_as_of",
        "{pinned}"
    );
    assert_eq!(
        pinned["items"][1]["status"], "not_found_at_as_of",
        "{pinned}"
    );
    assert!(!pinned.to_string().contains("ORIGINAL"), "{pinned}");

    let (status, current) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [retired, edited], "manifest_id": run["id"], "resolve": "current"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{current}");
    assert_eq!(
        current["items"][0]["status"], "not_found_at_as_of",
        "{current}"
    );
    assert_eq!(current["items"][1]["status"], "served", "{current}");
    assert!(current.to_string().contains("REPLACEMENT"), "{current}");
    assert!(!current.to_string().contains("ORIGINAL"), "{current}");
}

#[tokio::test]
async fn a_grant_revoked_after_the_manifest_reads_like_a_forged_id() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let words = distinct_words("revoked");
    let mut body = authored(
        &format!("Zephyr {words}"),
        &format!("Zephyr summary {words}"),
        &format!("Zephyr body {words}"),
    );
    body["sensitivity"] = json!("confidential");
    let (item, _) = create_knowledge_with_content(
        &world,
        "revoke-item",
        world.project_scope,
        Some(&world.project_id),
        None,
        body,
    )
    .await;
    let run = manifest(
        &world,
        &world.bob_token,
        &world.bob_session,
        "revoke-run",
        json!({"query": "zephyr", "disclosure": "manifest"}),
    )
    .await;
    assert_eq!(entry_ids(&run), vec![item.clone()], "{run}");

    deny_confidential_to_members(&world).await;
    let forged = uuid::Uuid::now_v7().to_string();
    let (status, response) = expand(
        &world,
        &world.bob_token,
        &world.bob_session,
        json!({"ids": [item, forged], "manifest_id": run["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        refusal_shape(&response["items"][0]),
        refusal_shape(&response["items"][1]),
        "{response}"
    );
    assert_eq!(response["items"][0]["status"], "not_found_at_as_of");
}

#[tokio::test]
async fn recall_defaults_to_summaries_and_full_is_explicit() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, revision) = zephyr_note(&world, "recalla", BODY_MARK).await;
    let path = format!("/v1/sessions/{}/knowledge-query", world.alice_session);
    let (status, summary) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        None,
        Some(json!({"query": "zephyr"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["depth"], "summary", "{summary}");
    assert_eq!(summary["items"], json!([]), "{summary}");
    assert_eq!(summary["truncated"], false, "{summary}");
    let hit = &summary["summaries"][0];
    assert_eq!(hit["id"], item, "{summary}");
    assert_eq!(hit["revision_id"], revision, "{summary}");
    assert!(hit["expand_cost_tokens"].as_u64().unwrap() > 0, "{summary}");
    assert!(hit["sources"].as_array().is_some_and(|all| !all.is_empty()));
    assert!(!summary.to_string().contains(BODY_MARK), "{summary}");

    let (status, full) = call(
        &world.app,
        "POST",
        &path,
        &world.alice_token,
        None,
        Some(json!({"query": "zephyr", "depth": "full"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{full}");
    assert_eq!(full["depth"], "full", "{full}");
    assert!(full.get("summaries").is_none(), "{full}");
    assert!(full.to_string().contains(BODY_MARK), "{full}");

    let audited = audit_events(&world)
        .await
        .into_iter()
        .filter(|event| event.payload["op"] == "context.knowledge_query")
        .map(|event| event.payload["detail"]["depth"].clone())
        .collect::<Vec<_>>();
    assert_eq!(audited, vec![json!("summary"), json!("full")]);
}

#[tokio::test]
async fn seeded_pii_never_reaches_any_tier() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let email = "dana.seeded@example.com";
    let words = distinct_words("pii");
    let (item, _) = create_knowledge_with_content(
        &world,
        "pii-item",
        world.project_scope,
        Some(&world.project_id),
        None,
        authored(
            &format!("Zephyr {email} {words}"),
            &format!("Zephyr owner {email} {words}"),
            &format!("Zephyr escalate to {email} {words}"),
        ),
    )
    .await;
    let run = alice_manifest(&world, "pii-run").await;
    assert_eq!(entry_ids(&run), vec![item.clone()], "{run}");
    let (_, summary) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"query": "zephyr"})),
    )
    .await;
    assert_eq!(summary["summaries"][0]["id"], item, "{summary}");
    let (_, expanded) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [item], "manifest_id": run["id"]}),
    )
    .await;
    assert_eq!(expanded["items"][0]["status"], "served", "{expanded}");
    for tier in [&run, &summary, &expanded] {
        assert!(!tier.to_string().contains(email), "{tier}");
    }
    assert!(rendered(&run).contains("[REDACTED:email]"), "{run}");
}

#[tokio::test]
async fn retention_masks_expansion_addresses_for_readers() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, _) = zephyr_note(&world, "maska", "NOTE").await;
    set_trace_mode(&world, TraceRetentionMode::HashesOnly).await;
    let (status, expanded) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [item]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    assert_eq!(expanded["items"][0]["status"], "served", "{expanded}");

    let (status, timeline) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{}/timeline", world.alice_session),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let entry = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["event_type"] == "context.knowledge_expanded")
        .expect("expansion on the timeline");
    assert!(!entry.to_string().contains(&item), "{entry}");
    let (status, event) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{}/events/{}",
            world.alice_session,
            entry["id"].as_str().unwrap()
        ),
        &world.alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{event}");
    assert!(!event["payload"].to_string().contains(&item), "{event}");
    assert!(
        event["payload"]["items"][0]["content_hash"].is_string(),
        "{event}"
    );

    let audited = audit_events(&world)
        .await
        .into_iter()
        .rev()
        .find(|event| event.payload["op"] == "context.knowledge_expand")
        .expect("expansion audit");
    assert!(
        !audited.payload.to_string().contains(&item),
        "{:?}",
        audited.payload
    );
}

#[tokio::test]
async fn a_client_cannot_forge_expansion_evidence() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (status, refused) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/events", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"events": [{
            "event_type": "context.knowledge_expanded",
            "client_event_id": "forged",
            "occurred_at": "2026-08-01T10:00:00Z",
            "payload": {"items": []},
        }]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
}

async fn claude_session(world: &World, token: &str, key: &str) -> String {
    let (status, opened) = call(
        &world.app,
        "POST",
        "/v1/sessions",
        token,
        Some(key),
        Some(json!({
            "workspace_id": world.workspace_id,
            "project_id": world.project_id,
            "client_name": "claude-code",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{opened}");
    opened["id"].as_str().expect("session id").to_owned()
}

async fn compact(world: &World, token: &str, session: &str) {
    let (status, appended) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/events"),
        token,
        None,
        Some(json!({"events": [
            {"event_type": "message.user", "client_event_id": "rehydrate-source",
                "occurred_at": "2026-10-01T10:00:00Z", "payload": {"text": "carry on"}},
            {"event_type": "session.compaction_boundary", "client_event_id": "rehydrate-boundary",
                "occurred_at": "2026-10-01T10:00:01Z", "payload": {"schema_version": 1,
                    "local_high_sequence": 2, "expected_client_event_ids": ["rehydrate-source"],
                    "expected_ids_truncated": false}},
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{appended}");
}

#[tokio::test]
async fn a_restart_restores_expanded_bodies_under_fresh_policy() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let session = claude_session(&world, &world.bob_token, "rehydrate-session").await;
    let (kept, _) = zephyr_note(&world, "rehydratekept", "KEPTBODY").await;
    let (retired, retired_revision) = zephyr_note(&world, "rehydrateretired", "RETIREDBODY").await;
    let (unexpanded, _) = zephyr_note(&world, "rehydrateother", "UNEXPANDEDBODY").await;
    let words = distinct_words("rehydrateconfidential");
    let mut body = authored(
        &format!("Zephyr {words}"),
        &format!("Zephyr summary {words}"),
        &format!("REVOKEDBODY Zephyr body {words}"),
    );
    body["sensitivity"] = json!("confidential");
    let (revoked, _) = create_knowledge_with_content(
        &world,
        "rehydrate-confidential",
        world.project_scope,
        Some(&world.project_id),
        None,
        body,
    )
    .await;

    let run = manifest(
        &world,
        &world.bob_token,
        &session,
        "rehydrate-manifest",
        json!({"query": "zephyr", "disclosure": "manifest"}),
    )
    .await;
    let (status, expanded) = expand(
        &world,
        &world.bob_token,
        &session,
        json!({"ids": [kept, retired, revoked], "manifest_id": run["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    assert_eq!(
        expanded["tokens_returned"].as_u64().map(|n| n > 0),
        Some(true)
    );
    for item in expanded["items"].as_array().unwrap() {
        assert_eq!(item["status"], "served", "{expanded}");
    }

    let (status, archive) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{retired}/archive"),
        &world.alice_token,
        Some("rehydrate-archive"),
        Some(json!({"expected_revision_id": retired_revision, "reason": "retracted"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archive}");
    deny_confidential_to_members(&world).await;
    compact(&world, &world.bob_token, &session).await;

    let restarted = manifest(
        &world,
        &world.bob_token,
        &session,
        "rehydrate-restart",
        json!({"query": "zephyr", "disclosure": "manifest", "restart": true, "budget_tokens": 1500}),
    )
    .await;
    let block = rendered(&restarted);
    assert!(block.contains("KEPTBODY"), "{block}");
    assert!(!block.contains("RETIREDBODY"), "{block}");
    assert!(!block.contains("REVOKEDBODY"), "{block}");
    assert!(!block.contains(&revoked), "{block}");
    assert!(!block.contains("UNEXPANDEDBODY"), "{block}");
    assert!(entry_ids(&restarted).contains(&unexpanded), "{block}");
}

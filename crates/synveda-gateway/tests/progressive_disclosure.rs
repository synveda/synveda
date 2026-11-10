//! Progressive context disclosure acceptance.
#[path = "support/context_world.rs"]
mod context_world;
use context_world::*;
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use synveda_types::Sensitivity;
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
async fn a_policy_pack_narrows_expansion_evidence_under_a_full_configuration() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, _) = zephyr_note(&world, "packa", "NOTE").await;
    set_trace_modes(
        &world,
        TraceRetentionMode::HashesOnly,
        TraceRetentionMode::Full,
    )
    .await;
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
    let event_id = timeline["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["event_type"] == "context.knowledge_expanded")
        .and_then(|entry| entry["id"].as_str())
        .expect("expansion on the timeline")
        .to_owned();
    let (status, event) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{}/events/{event_id}", world.alice_session),
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

    let (status, recalled) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/knowledge-query", world.alice_session),
        &world.alice_token,
        None,
        Some(json!({"query": "zephyr"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{recalled}");
    assert_eq!(recalled["summaries"][0]["id"], item, "{recalled}");

    let audit = audit_events(&world).await;
    for op in ["context.knowledge_expand", "context.knowledge_query"] {
        let audited = audit
            .iter()
            .rev()
            .find(|event| event.payload["op"] == op)
            .unwrap_or_else(|| panic!("{op} audit"));
        assert!(
            !audited.payload.to_string().contains(&item),
            "{op}: {:?}",
            audited.payload
        );
    }
    let recall_audit = audit
        .iter()
        .rev()
        .find(|event| event.payload["op"] == "context.knowledge_query")
        .expect("recall audit");
    assert_eq!(
        recall_audit.payload["detail"]["result_count"], 1,
        "{:?}",
        recall_audit.payload
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

#[tokio::test]
async fn forgotten_knowledge_reads_like_a_forged_id_and_never_returns_on_restart() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let session = claude_session(&world, &world.bob_token, "forget-session").await;
    let (forgotten, forgotten_revision) = zephyr_note(&world, "forgetgone", "FORGOTTENBODY").await;
    let (kept, _) = zephyr_note(&world, "forgetkept", "KEPTBODY").await;
    let run = manifest(
        &world,
        &world.bob_token,
        &session,
        "forget-manifest",
        json!({"query": "zephyr", "disclosure": "manifest"}),
    )
    .await;
    let (status, expanded) = expand(
        &world,
        &world.bob_token,
        &session,
        json!({"ids": [forgotten, kept], "manifest_id": run["id"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    for item in expanded["items"].as_array().expect("items") {
        assert_eq!(item["status"], "served", "{expanded}");
    }

    let (status, erased) = call(
        &world.app,
        "DELETE",
        &format!("/v1/knowledge/{forgotten}"),
        &world.alice_token,
        Some("forget-erasure"),
        Some(json!({
            "mode": "forget",
            "expected_revision_id": forgotten_revision,
            "reason": "erasure after expansion",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{erased}");
    assert_eq!(erased["outcome"], "applied", "{erased}");

    let forged = uuid::Uuid::now_v7().to_string();
    for resolve in ["pinned", "current"] {
        let (status, after) = expand(
            &world,
            &world.bob_token,
            &session,
            json!({"ids": [forgotten, forged], "manifest_id": run["id"], "resolve": resolve}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{after}");
        assert_eq!(
            refusal_shape(&after["items"][0]),
            json!({"status": "not_found_at_as_of"}),
            "{resolve}: {after}"
        );
        assert_eq!(
            refusal_shape(&after["items"][0]),
            refusal_shape(&after["items"][1]),
            "{resolve}: {after}"
        );
        assert!(!after.to_string().contains("FORGOTTENBODY"), "{after}");
    }

    compact(&world, &world.bob_token, &session).await;
    let restarted = manifest(
        &world,
        &world.bob_token,
        &session,
        "forget-restart",
        json!({"query": "zephyr", "disclosure": "manifest", "restart": true, "budget_tokens": 1500}),
    )
    .await;
    let block = rendered(&restarted);
    assert!(block.contains("KEPTBODY"), "{block}");
    assert!(!block.contains("FORGOTTENBODY"), "{block}");
    assert!(!entry_ids(&restarted).contains(&forgotten), "{block}");
}

/// The expansion's timeline entry and its reader view.
async fn expansion_views(world: &World, token: &str, session: &str) -> (Value, Value) {
    let (status, timeline) = call(
        &world.app,
        "GET",
        &format!("/v1/sessions/{session}/timeline"),
        token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{timeline}");
    let entry = timeline["entries"]
        .as_array()
        .expect("timeline entries")
        .iter()
        .find(|entry| entry["event_type"] == "context.knowledge_expanded")
        .cloned()
        .unwrap_or_else(|| panic!("expansion on the timeline: {timeline}"));
    let (status, event) = call(
        &world.app,
        "GET",
        &format!(
            "/v1/sessions/{session}/events/{}",
            entry["id"].as_str().expect("event id")
        ),
        token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{event}");
    (entry, event)
}

#[tokio::test]
async fn disabled_retention_counts_expansions_without_naming_them() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let (item, revision) = zephyr_note(&world, "disableda", "NOTE").await;
    set_trace_mode(&world, TraceRetentionMode::Disabled).await;
    let (status, expanded) = expand(
        &world,
        &world.alice_token,
        &world.alice_session,
        json!({"ids": [item]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{expanded}");
    assert_eq!(expanded["items"][0]["status"], "served", "{expanded}");

    let (entry, event) = expansion_views(&world, &world.alice_token, &world.alice_session).await;
    for view in [&entry, &event] {
        let text = view.to_string();
        assert!(!text.contains(&item), "{text}");
        assert!(!text.contains(&revision), "{text}");
    }
    assert_eq!(event["payload"]["item_count"], 1, "{event}");
    assert!(event["payload"].get("items").is_none(), "{event}");

    let audited = audit_events(&world)
        .await
        .into_iter()
        .rev()
        .find(|event| event.payload["op"] == "context.knowledge_expand")
        .expect("expansion audit");
    let detail = &audited.payload["detail"];
    assert_eq!(detail["served"], 1, "{detail}");
    assert_eq!(detail["tokens_returned"], expanded["tokens_returned"]);
    assert_eq!(detail["addresses"], Value::Null, "{detail}");
    assert!(
        !audited.payload.to_string().contains(&item),
        "{:?}",
        audited.payload
    );

    let stored = expansion_events(&world, &world.alice_session).await;
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(
        stored[0]["items"][0]["knowledge_item_id"], item,
        "the stored event keeps what restart re-reads: {stored:?}"
    );
}

#[tokio::test]
async fn a_restart_restores_expansions_under_every_narrowed_retention_mode() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    for mode in [TraceRetentionMode::HashesOnly, TraceRetentionMode::Disabled] {
        let tag = mode.as_str().replace('_', "");
        set_trace_mode(&world, mode).await;
        let session = claude_session(
            &world,
            &world.bob_token,
            &format!("retention-session-{tag}"),
        )
        .await;
        let mark = format!("KEPT{}BODY", tag.to_uppercase());
        let (kept, kept_revision) =
            zephyr_note(&world, &format!("retentionkept{tag}"), &mark).await;
        let run = manifest(
            &world,
            &world.bob_token,
            &session,
            &format!("retention-manifest-{tag}"),
            json!({"query": "zephyr", "disclosure": "manifest"}),
        )
        .await;
        assert!(!rendered(&run).contains(&mark), "{run}");
        let (status, expanded) = expand(
            &world,
            &world.bob_token,
            &session,
            json!({"ids": [kept], "manifest_id": run["id"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{expanded}");
        assert_eq!(expanded["items"][0]["status"], "served", "{expanded}");
        compact(&world, &world.bob_token, &session).await;

        let restarted = manifest(
            &world,
            &world.bob_token,
            &session,
            &format!("retention-restart-{tag}"),
            json!({"query": "zephyr", "disclosure": "manifest", "restart": true, "budget_tokens": 1500}),
        )
        .await;
        assert!(
            rendered(&restarted).contains(&mark),
            "{mode:?}: {restarted}"
        );
        let trace = detail(
            &world,
            &world.bob_token,
            restarted["id"].as_str().expect("run id"),
        )
        .await;
        assert_eq!(
            trace["run"]["trace_retention_mode"],
            mode.as_str(),
            "{trace}"
        );
        assert_eq!(trace["run"]["rendered"], Value::Null, "{trace}");
        let retained = trace.to_string();
        for forbidden in [&kept, &kept_revision, &mark] {
            assert!(
                !retained.contains(forbidden.as_str()),
                "{mode:?} retained {forbidden}: {retained}"
            );
        }
        let composed = audit_events(&world)
            .await
            .into_iter()
            .rev()
            .find(|event| event.action == "session.context.composed")
            .expect("restart composition audit");
        let audited = composed.payload.to_string();
        assert!(!audited.contains(&kept), "{mode:?}: {audited}");
        assert!(!audited.contains(&mark), "{mode:?}: {audited}");
    }
}

// ── Policy-leak fuzz across manifest entries and summary recall ──────────────

const LEAK_CASES: usize = 24;
const INTERNAL_DENIED: &str = r#"
permit (principal, action, resource) when { resource in principal.tenant };
forbid (principal, action == Synveda::Action::"KnowledgeRead", resource)
when { context.sensitivity != "public" && !context.roles.contains("administrator") };
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeakPack {
    Standard,
    RegulatedStrict,
    OpenCollaboration,
    ConfidentialDenied,
    InternalDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    Root,
    Workspace,
    Project,
    AlicePrivate,
}

#[derive(Debug, Clone)]
struct LeakCase {
    pack: LeakPack,
    bob_project_role: Option<RoleKey>,
    items: Vec<(Placement, Sensitivity)>,
}

fn leak_case() -> impl Strategy<Value = LeakCase> {
    (
        prop::sample::select(vec![
            LeakPack::Standard,
            LeakPack::RegulatedStrict,
            LeakPack::OpenCollaboration,
            LeakPack::ConfidentialDenied,
            LeakPack::InternalDenied,
        ]),
        prop::sample::select(vec![
            None,
            Some(RoleKey::Viewer),
            Some(RoleKey::Reviewer),
            Some(RoleKey::Curator),
            Some(RoleKey::Administrator),
        ]),
        prop::collection::vec(
            (
                prop::sample::select(vec![
                    Placement::Root,
                    Placement::Workspace,
                    Placement::Project,
                    Placement::AlicePrivate,
                ]),
                prop::sample::select(Sensitivity::ALL.to_vec()),
            ),
            2..8,
        ),
    )
        .prop_map(|(pack, bob_project_role, items)| LeakCase {
            pack,
            bob_project_role,
            items,
        })
}

/// Deterministic samples, so a failure names a case that reruns identically.
fn sampled<T: std::fmt::Debug>(strategy: impl Strategy<Value = T>, count: usize) -> Vec<T> {
    let mut runner = TestRunner::deterministic();
    (0..count)
        .map(|_| {
            strategy
                .new_tree(&mut runner)
                .expect("sample a case")
                .current()
        })
        .collect()
}

struct LeakItem {
    id: String,
    revision: String,
    mark: String,
    placement: Placement,
}

async fn bind_leak_pack(world: &World, pack: LeakPack) {
    let name = match pack {
        LeakPack::Standard => synveda_policy::STANDARD.to_owned(),
        LeakPack::RegulatedStrict => synveda_policy::REGULATED_STRICT.to_owned(),
        LeakPack::OpenCollaboration => synveda_policy::OPEN_COLLABORATION.to_owned(),
        LeakPack::ConfidentialDenied | LeakPack::InternalDenied => {
            let (name, source) = if pack == LeakPack::ConfidentialDenied {
                ("leak-confidential-denied", MEMBER_CONFIDENTIAL_DENIED)
            } else {
                ("leak-internal-denied", INTERNAL_DENIED)
            };
            world
                .install_policy(name, 1, source, PackConfig::default())
                .await;
            name.to_owned()
        }
    };
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin leak pack");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, &name).await;
    tx.commit().await.expect("commit leak pack");
}

async fn seed_leak_item(
    world: &World,
    case: usize,
    index: usize,
    placement: Placement,
    sensitivity: Sensitivity,
) -> Option<LeakItem> {
    let mark = format!("leak{case:02}i{index:02}z");
    let words = distinct_words(&mark);
    let mut body = authored(
        &format!("Zephyr {words}"),
        &format!("Zephyr summary {words}"),
        &format!("Zephyr body {words}"),
    );
    body["sensitivity"] = json!(sensitivity.as_str());
    let root = {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .expect("begin root read");
        let root = scopes::tenant_root(&mut *tx, world.tenant_id)
            .await
            .expect("read root")
            .expect("root exists");
        tx.commit().await.expect("commit root read");
        root.id
    };
    let (scope, project, owner) = match placement {
        Placement::Root => (root, None, None),
        Placement::Workspace => (world.workspace_scope, None, None),
        Placement::Project => (world.project_scope, Some(world.project_id.as_str()), None),
        Placement::AlicePrivate => (world.alice_scope, None, Some(ALICE)),
    };
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some(&format!("leak-{mark}")),
        Some(json!({
            "scope_id": scope,
            "project_id": project,
            "owner_principal_id": owner,
            "knowledge_type": "convention",
            "origin": "authored",
            "content": body,
            "sources": [{"scope_id": scope, "source_type": "manual"}],
        })),
    )
    .await;
    if status != StatusCode::CREATED || created["outcome"] != "applied" {
        return None;
    }
    Some(LeakItem {
        id: created["knowledge_item_id"].as_str()?.to_owned(),
        revision: created["revision_id"].as_str()?.to_owned(),
        mark: format!("alpha{mark}"),
        placement,
    })
}

async fn readable_by(world: &World, token: &str, item: &LeakItem) -> bool {
    let (status, _) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{}", item.id),
        token,
        None,
        None,
    )
    .await;
    status == StatusCode::OK
}

async fn summary_recall(world: &World, token: &str, session: &str) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/knowledge-query"),
        token,
        None,
        Some(json!({"query": "zephyr", "limit": 100})),
    )
    .await
}

fn summary_ids(recalled: &Value) -> Vec<String> {
    let mut ids: Vec<String> = recalled["summaries"]
        .as_array()
        .map(|all| {
            all.iter()
                .filter_map(|hit| hit["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn assert_no_trace(label: &str, response: &Value, hidden: &[&LeakItem]) {
    let text = response.to_string();
    for item in hidden {
        for trace in [&item.id, &item.revision, &item.mark] {
            assert!(
                !text.contains(trace.as_str()),
                "{label}: unreadable {} ({:?}) left {trace}: {text}",
                item.id,
                item.placement
            );
        }
    }
}

/// The parts of a manifest a reader could compare between two runs.
fn manifest_shape(run: &Value) -> Value {
    json!({
        "entries": entries(run),
        "candidate_count": run["candidate_count"],
        "selection_count": run["selection_count"],
        "entry_count": run["entry_count"],
        "truncated": rendered(run).contains("\"truncated\":true"),
        "policy_exclusion_message": run.get("policy_exclusion_message"),
    })
}

fn recall_shape(recalled: &Value) -> Value {
    json!({
        "summaries": recalled["summaries"],
        "truncated": recalled["truncated"],
        "depth": recalled["depth"],
    })
}

/// One reader's results before anything is archived.
struct ReaderView {
    manifest_status: StatusCode,
    manifest: Value,
    recall_status: StatusCode,
    recall: Value,
    readable: usize,
    hidden: usize,
}

fn leak_manifest_request() -> Value {
    json!({"query": "zephyr", "disclosure": "manifest", "budget_tokens": 6000})
}

/// A fresh tenant bound to the case's pack, with Bob's extra role and the
/// case's Knowledge. An item the pack keeps from being created is left out.
async fn seed_leak_case(case_index: usize, case: &LeakCase) -> (World, Vec<LeakItem>) {
    let world = admitted_world().await.expect("database already checked");
    bind_leak_pack(&world, case.pack).await;
    if let Some(role) = case.bob_project_role {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .expect("begin role grant");
        seed_grant(&mut tx, world.tenant_id, world.project_scope, BOB, role).await;
        tx.commit().await.expect("commit role grant");
    }
    let mut items = Vec::new();
    for (index, (placement, sensitivity)) in case.items.iter().enumerate() {
        if let Some(item) =
            seed_leak_item(&world, case_index, index, *placement, *sensitivity).await
        {
            items.push(item);
        }
    }
    (world, items)
}

/// A listing names only readable items and, when nothing was cut, every
/// readable item in the reader's universe.
fn assert_lists_only_readable(
    reader: &str,
    what: &str,
    listed: &[String],
    readable: &[&LeakItem],
    expected: &[String],
    complete: bool,
    response: &Value,
) {
    assert!(
        listed
            .iter()
            .all(|id| readable.iter().any(|item| item.id == *id)),
        "{reader}: {what} named an unreadable item: {response}"
    );
    if complete {
        assert_eq!(
            listed, expected,
            "{reader}: {what} omitted readable Knowledge: {response}"
        );
    }
}

/// One reader's manifest and summary recall, checked against what the
/// single-item read lets that reader open.
async fn check_reader(
    world: &World,
    items: &[LeakItem],
    reader: &str,
    name: &str,
    token: &str,
    session: &str,
    key: &str,
) -> ReaderView {
    let mut readable = Vec::new();
    let mut hidden = Vec::new();
    for item in items {
        if readable_by(world, token, item).await {
            readable.push(item);
        } else {
            hidden.push(item);
        }
    }
    let (manifest_status, manifest) =
        run_with(world, token, session, key, leak_manifest_request()).await;
    assert_no_trace(&format!("{reader} manifest"), &manifest, &hidden);
    let (recall_status, recall) = summary_recall(world, token, session).await;
    assert_no_trace(&format!("{reader} recall"), &recall, &hidden);

    let mut expected: Vec<String> = readable
        .iter()
        .filter(|item| item.placement != Placement::AlicePrivate || name == "alice")
        .map(|item| item.id.clone())
        .collect();
    expected.sort();
    if manifest_status == StatusCode::CREATED {
        assert!(
            manifest.get("policy_exclusion_message").is_none(),
            "{reader}: {manifest}"
        );
        let complete = !rendered(&manifest).contains("\"truncated\":true");
        let listed = entry_ids(&manifest);
        assert_lists_only_readable(
            reader, "manifest", &listed, &readable, &expected, complete, &manifest,
        );
    }
    if recall_status == StatusCode::OK {
        let listed = summary_ids(&recall);
        assert_lists_only_readable(
            reader, "recall", &listed, &readable, &expected, true, &recall,
        );
    }
    ReaderView {
        manifest_status,
        manifest,
        recall_status,
        recall,
        readable: readable.len(),
        hidden: hidden.len(),
    }
}

/// Archives, as Alice, what Bob may not read, and answers whether anything
/// was archived. An item Alice may not read either stays in place, which
/// keeps the comparison sound: it is present in both runs.
async fn archive_unreadable_by_bob(world: &World, items: &[LeakItem], label: &str) -> bool {
    let mut archived_any = false;
    for item in items {
        if readable_by(world, &world.bob_token, item).await
            || !readable_by(world, &world.alice_token, item).await
        {
            continue;
        }
        let (status, archived) = call(
            &world.app,
            "POST",
            &format!("/v1/knowledge/{}/archive", item.id),
            &world.alice_token,
            Some(&format!("leak-archive-{}", item.mark)),
            Some(json!({"expected_revision_id": item.revision, "reason": "fuzz"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{label}: {archived}");
        archived_any = true;
    }
    archived_any
}

/// Bob's manifest and recall after the archive match what he saw before it.
async fn assert_bob_unchanged(world: &World, label: &str, key: &str, before: &ReaderView) {
    let (status, after) = run_with(
        world,
        &world.bob_token,
        &world.bob_session,
        key,
        leak_manifest_request(),
    )
    .await;
    assert_eq!(status, before.manifest_status, "{label}: {after}");
    if status == StatusCode::CREATED {
        assert_eq!(
            manifest_shape(&after),
            manifest_shape(&before.manifest),
            "{label}: Bob's manifest changed when only unreadable Knowledge was archived"
        );
    }
    let (status, after) = summary_recall(world, &world.bob_token, &world.bob_session).await;
    assert_eq!(status, before.recall_status, "{label}: {after}");
    if status == StatusCode::OK {
        assert_eq!(
            recall_shape(&after),
            recall_shape(&before.recall),
            "{label}: Bob's recall changed when only unreadable Knowledge was archived"
        );
    }
}

#[tokio::test]
async fn unreadable_knowledge_leaves_no_trace_in_manifests_or_summaries() {
    let _guard = serial().await;
    if tenant_fixture::database_url("context tests").is_none() {
        return;
    }
    let mut compared = 0;
    let mut hidden_checks = 0;
    for (case_index, case) in sampled(leak_case(), LEAK_CASES).into_iter().enumerate() {
        let (world, items) = seed_leak_case(case_index, &case).await;
        let label = format!("case {case_index} {case:?}");
        let alice = check_reader(
            &world,
            &items,
            &format!("{label} reader alice"),
            "alice",
            &world.alice_token,
            &world.alice_session,
            &format!("leak-{case_index}-alice-before"),
        )
        .await;
        let bob = check_reader(
            &world,
            &items,
            &format!("{label} reader bob"),
            "bob",
            &world.bob_token,
            &world.bob_session,
            &format!("leak-{case_index}-bob-before"),
        )
        .await;
        hidden_checks += alice.hidden + bob.hidden;

        // Archiving what Bob may not read must not change anything Bob sees.
        if bob.hidden == 0 || !archive_unreadable_by_bob(&world, &items, &label).await {
            continue;
        }
        assert_bob_unchanged(
            &world,
            &label,
            &format!("leak-{case_index}-bob-after"),
            &bob,
        )
        .await;
        if bob.manifest_status == StatusCode::CREATED && bob.readable > 0 {
            compared += 1;
        }
    }
    assert!(
        compared > 0,
        "no sampled case held Knowledge Bob could and could not read, so nothing was compared"
    );
    eprintln!(
        "policy-leak fuzz: {LEAK_CASES} cases, {hidden_checks} unreadable item checks, \
         {compared} archive comparisons"
    );
}

// ── Cross-tenant fuzz on expansion ──────────────────────────────────────────

const EXPAND_CASES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpandTarget {
    Readable(usize),
    Denied,
    Archived,
    AlicePrivate,
    Foreign(usize),
    OwnRevision,
    ManifestAsItem,
    Forged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpandManifest {
    Absent,
    Own,
    OtherSession,
    Foreign,
    Forged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpandCaller {
    Owner,
    OwnerOnForeignSession,
    OwnerOnForgedSession,
    ForeignTenantOnOwnSession,
}

#[derive(Debug, Clone)]
struct ExpandCase {
    caller: ExpandCaller,
    manifest: ExpandManifest,
    resolve: Option<&'static str>,
    targets: Vec<ExpandTarget>,
}

fn expand_case() -> impl Strategy<Value = ExpandCase> {
    let target = prop_oneof![
        (0_usize..3).prop_map(ExpandTarget::Readable),
        Just(ExpandTarget::Denied),
        Just(ExpandTarget::Archived),
        Just(ExpandTarget::AlicePrivate),
        (0_usize..2).prop_map(ExpandTarget::Foreign),
        Just(ExpandTarget::OwnRevision),
        Just(ExpandTarget::ManifestAsItem),
        Just(ExpandTarget::Forged),
    ];
    (
        prop_oneof![
            6 => Just(ExpandCaller::Owner),
            1 => Just(ExpandCaller::OwnerOnForeignSession),
            1 => Just(ExpandCaller::OwnerOnForgedSession),
            1 => Just(ExpandCaller::ForeignTenantOnOwnSession),
        ],
        prop_oneof![
            3 => Just(ExpandManifest::Absent),
            3 => Just(ExpandManifest::Own),
            1 => Just(ExpandManifest::OtherSession),
            1 => Just(ExpandManifest::Foreign),
            1 => Just(ExpandManifest::Forged),
        ],
        prop::sample::select(vec![None, Some("pinned"), Some("current")]),
        prop::collection::vec(target, 1..10),
    )
        .prop_map(|(caller, manifest, resolve, mut targets)| {
            let mut seen = Vec::new();
            targets.retain(|target| {
                *target == ExpandTarget::Forged || {
                    let fresh = !seen.contains(target);
                    seen.push(*target);
                    fresh
                }
            });
            ExpandCase {
                caller,
                manifest,
                resolve,
                targets,
            }
        })
}

fn without_session(body: &Value, session: &str) -> String {
    body.to_string().replace(session, "SESSION")
}

/// Everything the cross-tenant cases address: a home tenant whose Bob
/// expands, and a foreign tenant whose ids and manifest he must not reach.
struct CrossFixture {
    home: World,
    foreign: World,
    readable: Vec<(String, String)>,
    denied: String,
    archived: String,
    private: String,
    foreign_items: Vec<(String, String)>,
    own_run: Value,
    other_run: Value,
    foreign_run: Value,
    /// The unknown-Session answer with its Session id removed.
    unknown: String,
}

#[derive(Default)]
struct CrossTally {
    served: std::collections::BTreeSet<String>,
    outcomes: std::collections::BTreeMap<u16, usize>,
    refusals: usize,
}

/// The home tenant's ids Bob must not be served: policy-denied, archived
/// and Alice's private Knowledge.
async fn seed_home_withheld(home: &World) -> (String, String, String) {
    let words = distinct_words("crossdenied");
    let mut restricted = authored(
        &format!("Zephyr {words}"),
        &format!("Zephyr summary {words}"),
        &format!("DENIEDBODY Zephyr body {words}"),
    );
    restricted["sensitivity"] = json!("confidential");
    let (denied, _) = create_knowledge_with_content(
        home,
        "cross-denied",
        home.project_scope,
        Some(&home.project_id),
        None,
        restricted,
    )
    .await;
    let (archived, archived_revision) = zephyr_note(home, "crossarchived", "ARCHIVEDBODY").await;
    let (status, archive) = call(
        &home.app,
        "POST",
        &format!("/v1/knowledge/{archived}/archive"),
        &home.alice_token,
        Some("cross-archive"),
        Some(json!({"expected_revision_id": archived_revision, "reason": "retired"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archive}");
    let (private, _) = create_knowledge(
        home,
        "cross-private",
        home.alice_scope,
        None,
        Some(ALICE),
        "Zephyr private",
        "PRIVATEBODY Zephyr private body.",
        None,
    )
    .await;
    (denied, archived, private)
}

async fn seed_cross_fixture(home: World, foreign: World) -> CrossFixture {
    deny_confidential_to_members(&home).await;
    let mut readable = Vec::new();
    for index in 0..3 {
        readable.push(zephyr_note(&home, &format!("crossreadable{index}"), "HOMEBODY").await);
    }
    let (denied, archived, private) = seed_home_withheld(&home).await;
    let mut foreign_items = Vec::new();
    for index in 0..2 {
        foreign_items
            .push(zephyr_note(&foreign, &format!("crossforeign{index}"), "FOREIGNBODY").await);
    }
    let foreign_run = alice_manifest(&foreign, "cross-foreign-run").await;
    let own_run = manifest(
        &home,
        &home.bob_token,
        &home.bob_session,
        "cross-own-run",
        json!({"query": "zephyr", "disclosure": "manifest"}),
    )
    .await;
    let other_run = alice_manifest(&home, "cross-other-run").await;
    let forged_session = uuid::Uuid::now_v7().to_string();
    let (status, unknown) = expand(
        &home,
        &home.bob_token,
        &forged_session,
        json!({"ids": [uuid::Uuid::now_v7()]}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    CrossFixture {
        unknown: without_session(&unknown, &forged_session),
        home,
        foreign,
        readable,
        denied,
        archived,
        private,
        foreign_items,
        own_run,
        other_run,
        foreign_run,
    }
}

impl CrossFixture {
    fn target_id(&self, target: ExpandTarget) -> String {
        match target {
            ExpandTarget::Readable(slot) => self.readable[slot].0.clone(),
            ExpandTarget::Denied => self.denied.clone(),
            ExpandTarget::Archived => self.archived.clone(),
            ExpandTarget::AlicePrivate => self.private.clone(),
            ExpandTarget::Foreign(slot) => self.foreign_items[slot].0.clone(),
            ExpandTarget::OwnRevision => self.readable[0].1.clone(),
            ExpandTarget::ManifestAsItem => self.own_run["id"].as_str().unwrap().to_owned(),
            ExpandTarget::Forged => uuid::Uuid::now_v7().to_string(),
        }
    }

    fn body(&self, case: &ExpandCase, ids: &[String]) -> Value {
        let mut body = json!({"ids": ids});
        match case.manifest {
            ExpandManifest::Absent => {}
            ExpandManifest::Own => body["manifest_id"] = self.own_run["id"].clone(),
            ExpandManifest::OtherSession => body["manifest_id"] = self.other_run["id"].clone(),
            ExpandManifest::Foreign => body["manifest_id"] = self.foreign_run["id"].clone(),
            ExpandManifest::Forged => body["manifest_id"] = json!(uuid::Uuid::now_v7()),
        }
        if let Some(resolve) = case.resolve {
            body["resolve"] = json!(resolve);
        }
        body
    }

    fn caller(&self, caller: ExpandCaller) -> (&World, &str, String) {
        let (home, foreign) = (&self.home, &self.foreign);
        match caller {
            ExpandCaller::Owner => (home, &home.bob_token, home.bob_session.clone()),
            ExpandCaller::OwnerOnForeignSession => {
                (home, &home.bob_token, foreign.alice_session.clone())
            }
            ExpandCaller::OwnerOnForgedSession => {
                (home, &home.bob_token, uuid::Uuid::now_v7().to_string())
            }
            ExpandCaller::ForeignTenantOnOwnSession => {
                (foreign, &foreign.alice_token, home.bob_session.clone())
            }
        }
    }

    fn home_ids(&self) -> Vec<&str> {
        self.readable
            .iter()
            .map(|(id, _)| id.as_str())
            .chain([
                self.denied.as_str(),
                self.archived.as_str(),
                self.private.as_str(),
            ])
            .collect()
    }
}

/// A served item for every readable target and the one uniform refusal for
/// everything else, in request order.
fn check_item_outcomes(
    label: &str,
    case: &ExpandCase,
    ids: &[String],
    response: &Value,
    tally: &mut CrossTally,
) {
    let items = response["items"].as_array().expect("items");
    assert_eq!(items.len(), case.targets.len(), "{label}: {response}");
    for ((target, id), item) in case.targets.iter().zip(ids).zip(items) {
        assert_eq!(item["id"], json!(id), "{label}: request order");
        if let ExpandTarget::Readable(_) = target {
            assert_eq!(item["status"], "served", "{label}: {response}");
            assert_eq!(item["knowledge"]["id"], json!(id), "{label}");
            tally.served.insert(id.clone());
        } else {
            tally.refusals += 1;
            assert_eq!(
                refusal_shape(item),
                json!({"status": "not_found_at_as_of"}),
                "{label}: {target:?} did not read like a forged id: {response}"
            );
        }
    }
}

async fn run_expand_case(
    fixture: &CrossFixture,
    index: usize,
    case: &ExpandCase,
    tally: &mut CrossTally,
) {
    let label = format!("case {index} {case:?}");
    let ids: Vec<String> = case
        .targets
        .iter()
        .map(|target| fixture.target_id(*target))
        .collect();
    let (world, token, session) = fixture.caller(case.caller);
    let (status, response) = expand(world, token, &session, fixture.body(case, &ids)).await;
    *tally.outcomes.entry(status.as_u16()).or_default() += 1;
    for marker in ["FOREIGNBODY", "DENIEDBODY", "ARCHIVEDBODY", "PRIVATEBODY"] {
        assert!(
            !response.to_string().contains(marker),
            "{label}: {response}"
        );
    }

    // The body is validated before any Session is looked up, so this answer
    // is the same whether or not the Session exists.
    if case.manifest == ExpandManifest::Absent && case.resolve == Some("pinned") {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {response}");
        return;
    }
    let unknown_manifest = matches!(
        case.manifest,
        ExpandManifest::OtherSession | ExpandManifest::Foreign | ExpandManifest::Forged
    );
    if case.caller != ExpandCaller::Owner || unknown_manifest {
        assert_eq!(status, StatusCode::NOT_FOUND, "{label}: {response}");
        assert_eq!(
            without_session(&response, &session),
            fixture.unknown,
            "{label}"
        );
        return;
    }
    assert_eq!(status, StatusCode::OK, "{label}: {response}");
    check_item_outcomes(&label, case, &ids, &response, tally);
}

/// The home audit and events name only what was served, and the foreign
/// tenant records nothing about the home one.
async fn assert_evidence_names_only_served(
    fixture: &CrossFixture,
    served: &std::collections::BTreeSet<String>,
) {
    let (home, foreign) = (&fixture.home, &fixture.foreign);
    for event in audit_events(home)
        .await
        .into_iter()
        .filter(|event| event.payload["op"] == "context.knowledge_expand")
    {
        let addresses = event.payload["detail"]["addresses"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for address in addresses {
            let id = address["knowledge_item_id"].as_str().unwrap_or_default();
            assert!(
                served.contains(id),
                "the audit named an id it did not serve: {:?}",
                event.payload
            );
        }
    }
    for event in expansion_events(home, &home.bob_session).await {
        for item in event["items"].as_array().expect("items") {
            let id = item["knowledge_item_id"].as_str().unwrap_or_default();
            assert!(
                served.contains(id),
                "an event named an unserved id: {event}"
            );
        }
    }
    assert!(
        expansion_events(foreign, &foreign.alice_session)
            .await
            .is_empty(),
        "nothing aimed at the home tenant may record evidence in the foreign one"
    );
    let home_ids = fixture.home_ids();
    for event in audit_events(foreign).await {
        let text = event.payload.to_string();
        for id in &home_ids {
            assert!(
                !text.contains(id),
                "the foreign tenant's audit named a home id: {text}"
            );
        }
    }
}

#[tokio::test]
async fn forged_and_foreign_expansions_all_read_alike() {
    let _guard = serial().await;
    let Some(home) = admitted_world().await else {
        return;
    };
    let foreign = admitted_world().await.expect("a second tenant");
    let fixture = seed_cross_fixture(home, foreign).await;
    let mut tally = CrossTally::default();
    for (index, case) in sampled(expand_case(), EXPAND_CASES).into_iter().enumerate() {
        run_expand_case(&fixture, index, &case, &mut tally).await;
    }
    assert!(!tally.served.is_empty(), "no sampled case served anything");
    assert!(tally.refusals > 0, "no sampled case refused anything");
    eprintln!(
        "cross-tenant expansion fuzz: {EXPAND_CASES} cases, statuses {:?}, {} refused ids",
        tally.outcomes, tally.refusals
    );
    assert_evidence_names_only_served(&fixture, &tally.served).await;
}

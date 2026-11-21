//! Business links between Knowledge items (ADR-0164): governed admission,
//! refusals that reveal nothing, ending, bitemporal reads, erasure and review.
#[path = "support/context_world.rs"]
mod context_world;
use chrono::{DateTime, SecondsFormat, Utc};
use context_world::*;
use synveda_types::knowledge::{EntityClass, EntityPredicate};

const SUBJECT: &str = "dcterms:subject";
const WAS_ATTRIBUTED_TO: &str = "prov:wasAttributedTo";
const WORKS_FOR: &str = "schema:worksFor";

async fn enable_vocabulary(
    world: &World,
    classes: Vec<EntityClass>,
    predicates: Vec<EntityPredicate>,
) {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin vocabulary Configuration");
    configuration_support::set_tenant_entity_vocabulary(
        &mut tx,
        world.tenant_id,
        classes,
        predicates,
    )
    .await;
    tx.commit().await.expect("commit vocabulary Configuration");
}

/// An item and its current revision.
struct Item {
    id: String,
    revision: String,
}

async fn create(
    world: &World,
    key: &str,
    scope: ScopeId,
    knowledge_type: &str,
    title: &str,
    metadata: Value,
) -> Item {
    let mut body = json!({
        "scope_id": scope,
        "knowledge_type": knowledge_type,
        "origin": "authored",
        "content": {
            "title": title,
            "body_markdown": format!("{title}."),
            "summary": title,
            "tags": [],
            "sensitivity": "internal",
            "confidence_permille": 900,
            "metadata": metadata,
        },
    });
    if scope == world.project_scope {
        body["project_id"] = json!(world.project_id);
    }
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some(key),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    Item {
        id: created["knowledge_item_id"]
            .as_str()
            .expect("item id")
            .to_owned(),
        revision: created["revision_id"]
            .as_str()
            .expect("revision id")
            .to_owned(),
    }
}

fn declares(class: &str) -> Value {
    json!({"synveda_entity": {"schema_version": 1, "class": class}})
}

/// A project `entity` item declaring `class`.
async fn entity(world: &World, key: &str, title: &str, class: &str) -> Item {
    create(
        world,
        key,
        world.project_scope,
        "entity",
        title,
        declares(class),
    )
    .await
}

/// A project item of a type without a class.
async fn typed(world: &World, key: &str, knowledge_type: &str, title: &str) -> Item {
    create(
        world,
        key,
        world.project_scope,
        knowledge_type,
        title,
        json!({}),
    )
    .await
}

async fn link(
    world: &World,
    token: &str,
    key: &str,
    source: &Item,
    target: &str,
    predicate: &str,
    valid_from: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({
        "expected_revision_id": source.revision,
        "target_item_id": target,
        "predicate": predicate,
    });
    if let Some(valid_from) = valid_from {
        body["valid_from"] = json!(valid_from);
    }
    call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/links", source.id),
        token,
        Some(key),
        Some(body),
    )
    .await
}

fn changed_link(change: &Value) -> String {
    change["entity_link_id"]
        .as_str()
        .expect("link id")
        .to_owned()
}

async fn linked(world: &World, key: &str, source: &Item, target: &Item, predicate: &str) -> String {
    let (status, created) = link(
        world,
        &world.alice_token,
        key,
        source,
        &target.id,
        predicate,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied", "{created}");
    changed_link(&created)
}

async fn end(
    world: &World,
    token: &str,
    key: &str,
    source: &Item,
    link_id: &str,
    valid_to: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({"reason": "The relationship changed."});
    if let Some(valid_to) = valid_to {
        body["valid_to"] = json!(valid_to);
    }
    call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/links/{link_id}/end", source.id),
        token,
        Some(key),
        Some(body),
    )
    .await
}

async fn links(world: &World, token: &str, item: &str, lens: &str) -> Vec<Value> {
    let (status, listed) = call(
        &world.app,
        "GET",
        &format!("/v1/knowledge/{item}/links{lens}"),
        token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    listed["links"].as_array().expect("links").clone()
}

async fn refused(
    world: &World,
    key: &str,
    source: &Item,
    target: &str,
    predicate: &str,
    message: &str,
) {
    let (status, refusal) = link(
        world,
        &world.alice_token,
        key,
        source,
        target,
        predicate,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refusal}");
    assert!(refusal.to_string().contains(message), "{refusal}");
}

async fn stored_rows(world: &World, table: &str) -> i64 {
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    let rows = sqlx::query_scalar(&format!(
        "select count(*) from {table} where tenant_id = $1"
    ))
    .bind(world.tenant_id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count link rows");
    tx.commit().await.expect("commit link count");
    rows
}

#[tokio::test]
async fn a_link_needs_an_enabled_predicate_and_ends_that_fit_it() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    enable_vocabulary(&world, classes.clone(), Vec::new()).await;
    let decision = typed(&world, "rule-decision", "decision", "Stop retrying charges").await;
    let fact = typed(&world, "rule-fact", "fact", "Charges settle nightly").await;
    let service = entity(&world, "rule-service", "Ledgerline", "schema:Service").await;
    let person = entity(&world, "rule-person", "Dana Ortiz", "schema:Person").await;

    refused(
        &world,
        "rule-off",
        &decision,
        &service.id,
        SUBJECT,
        "is not enabled",
    )
    .await;
    let predicates = vec![EntityPredicate::WasAttributedTo, EntityPredicate::Subject];
    enable_vocabulary(&world, classes, predicates.clone()).await;
    let about = linked(&world, "rule-about", &decision, &service, SUBJECT).await;
    let by = linked(&world, "rule-by", &decision, &person, WAS_ATTRIBUTED_TO).await;
    refuses_ends_that_do_not_fit(&world, &decision, &fact, &service).await;
    enable_vocabulary(&world, vec![EntityClass::Person], predicates).await;
    refused(
        &world,
        "rule-class-off",
        &decision,
        &service.id,
        SUBJECT,
        "schema:Service is not enabled",
    )
    .await;

    let outgoing = links(&world, &world.alice_token, &decision.id, "").await;
    let ids: Vec<&str> = outgoing
        .iter()
        .map(|link| link["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [about.as_str(), by.as_str()]);
    assert_eq!(outgoing[0]["predicate"], SUBJECT);
    assert_eq!(
        outgoing[0]["asserting_revision_id"],
        decision.revision.as_str()
    );
    let incoming = links(&world, &world.alice_token, &service.id, "").await;
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0]["source_item_id"], decision.id.as_str());

    a_stale_source_is_rejected(&world, &decision, &person).await;
    an_outsider_learns_nothing(&world, &decision, &person).await;
}

async fn refuses_ends_that_do_not_fit(world: &World, decision: &Item, fact: &Item, service: &Item) {
    refused(
        world,
        "rule-fact-source",
        fact,
        &service.id,
        SUBJECT,
        "does not join",
    )
    .await;
    refused(
        world,
        "rule-range",
        decision,
        &service.id,
        WAS_ATTRIBUTED_TO,
        "does not join",
    )
    .await;
    refused(
        world,
        "rule-self",
        decision,
        &decision.id,
        SUBJECT,
        "two different",
    )
    .await;
    refused(
        world,
        "rule-unknown",
        decision,
        &service.id,
        "org:hasMember",
        "unknown business link predicate",
    )
    .await;
}

async fn a_stale_source_is_rejected(world: &World, decision: &Item, person: &Item) {
    let (status, verified) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/verify", decision.id),
        &world.alice_token,
        Some("rule-verify"),
        Some(json!({
            "expected_revision_id": decision.revision,
            "verification_metadata": {"method": "reviewed"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{verified}");
    let (status, stale) = link(
        world,
        &world.alice_token,
        "rule-stale",
        decision,
        &person.id,
        WAS_ATTRIBUTED_TO,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{stale}");
    assert_eq!(stale["outcome"], "rejected", "the source moved on: {stale}");
}

async fn an_outsider_learns_nothing(world: &World, decision: &Item, person: &Item) {
    let (status, outsider) = link(
        world,
        &world.mallory_token,
        "rule-outsider",
        decision,
        &person.id,
        WAS_ATTRIBUTED_TO,
        None,
    )
    .await;
    assert!(status.is_client_error(), "{outsider}");
    for word in ["predicate", "class", "enabled", "join"] {
        assert!(!outsider.to_string().contains(word), "{outsider}");
    }
}

/// A body with the probed id replaced, so refusals of two ids compare.
fn shape(body: &Value, id: &str) -> String {
    body.to_string().replace(id, "<id>")
}

#[tokio::test]
async fn an_unreadable_end_is_refused_exactly_like_an_absent_one() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let predicates = vec![EntityPredicate::Subject];
    enable_vocabulary(&world, vec![EntityClass::Service], predicates).await;
    let decision = typed(&world, "hide-decision", "decision", "Pin the ledger region").await;
    let visible = entity(&world, "hide-visible", "Ledgerline", "schema:Service").await;
    let hidden = create(
        &world,
        "hide-hidden",
        world.alice_scope,
        "entity",
        "Payroll export",
        declares("schema:Service"),
    )
    .await;

    let absent = uuid::Uuid::now_v7().to_string();
    let (absent_status, absent_refusal) =
        link_as_bob(&world, "hide-absent", &decision, &absent).await;
    let (hidden_status, hidden_refusal) =
        link_as_bob(&world, "hide-hidden", &decision, &hidden.id).await;
    assert_eq!(absent_status, StatusCode::NOT_FOUND, "{absent_refusal}");
    assert_eq!(hidden_status, absent_status, "{hidden_refusal}");
    assert_eq!(
        shape(&hidden_refusal, &hidden.id),
        shape(&absent_refusal, &absent)
    );
    let (control, admitted) = link_as_bob(&world, "hide-control", &decision, &visible.id).await;
    assert_eq!(
        control,
        StatusCode::CREATED,
        "a readable target is admitted: {admitted}"
    );

    let concealed =
        a_link_to_an_unreadable_end_is_hidden(&world, &decision, &visible, &hidden).await;
    ending_it_is_refused_like_an_absent_link(&world, &decision, &concealed).await;
}

/// Bob, a workspace member, links `source` to `target` with `dcterms:subject`.
async fn link_as_bob(world: &World, key: &str, source: &Item, target: &str) -> (StatusCode, Value) {
    link(world, &world.bob_token, key, source, target, SUBJECT, None).await
}

async fn ending_it_is_refused_like_an_absent_link(world: &World, decision: &Item, concealed: &str) {
    let bob = &world.bob_token;
    let forged = uuid::Uuid::now_v7().to_string();
    let (status, ended) = end(world, bob, "hide-end", decision, concealed, None).await;
    let (forged_status, forged_refusal) =
        end(world, bob, "hide-end-forged", decision, &forged, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{ended}");
    assert_eq!(forged_status, status, "{forged_refusal}");
    assert_eq!(shape(&ended, concealed), shape(&forged_refusal, &forged));
}

/// Links the decision to the hidden item and returns that link's id.
async fn a_link_to_an_unreadable_end_is_hidden(
    world: &World,
    decision: &Item,
    visible: &Item,
    hidden: &Item,
) -> String {
    let concealed = linked(world, "hide-alice", decision, hidden, SUBJECT).await;
    assert_eq!(
        links(world, &world.alice_token, &decision.id, "")
            .await
            .len(),
        2
    );
    let seen = links(world, &world.bob_token, &decision.id, "").await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["target_item_id"], visible.id.as_str());
    let listed = Value::Array(seen).to_string();
    assert!(
        !listed.contains(&hidden.id) && !listed.contains(&concealed),
        "{listed}"
    );
    concealed
}

fn instant(value: &Value) -> DateTime<Utc> {
    value
        .as_str()
        .expect("instant")
        .parse()
        .expect("RFC 3339 instant")
}

fn lens(as_of: &str, as_known_at: DateTime<Utc>) -> String {
    format!(
        "?as_of={as_of}&as_known_at={}",
        as_known_at.to_rfc3339_opts(SecondsFormat::Micros, true)
    )
}

#[tokio::test]
async fn an_ended_link_leaves_the_present_and_stays_readable_as_known() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Organization];
    enable_vocabulary(&world, classes, vec![EntityPredicate::WorksFor]).await;
    let alice = &world.alice_token;
    let dana = entity(&world, "time-dana", "Dana Ortiz", "schema:Person").await;
    let acme = entity(&world, "time-acme", "Acme Freight", "schema:Organization").await;
    let (status, created) = link(
        &world,
        alice,
        "time-link",
        &dana,
        &acme.id,
        WORKS_FOR,
        Some("2026-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let link_id = changed_link(&created);
    let open = links(&world, alice, &dana.id, "").await;
    assert_eq!(open.len(), 1);
    assert!(open[0].get("valid_to").is_none(), "{open:?}");

    let until = Some("2026-06-01T00:00:00Z");
    let (status, ended) = end(&world, alice, "time-end", &dana, &link_id, until).await;
    assert_eq!(status, StatusCode::CREATED, "{ended}");
    assert_eq!(ended["outcome"], "applied", "{ended}");
    assert_eq!(ended["entity_link_id"], link_id.as_str());
    assert!(links(&world, alice, &dana.id, "").await.is_empty());

    the_lens_reads_the_ended_link(&world, &dana).await;
    a_link_ends_once_and_after_it_starts(&world, &dana, &acme, &link_id).await;
}

async fn the_lens_reads_the_ended_link(world: &World, dana: &Item) {
    let alice = &world.alice_token;
    let march = "2026-03-01T00:00:00Z";
    let during = links(world, alice, &dana.id, &format!("?as_of={march}")).await;
    assert_eq!(during.len(), 1);
    assert_eq!(
        instant(&during[0]["valid_to"]),
        instant(&json!("2026-06-01T00:00:00Z"))
    );
    let recorded = instant(&during[0]["created_at"]);
    assert!(instant(&during[0]["ended_at"]) > recorded, "{during:?}");
    let known_open = links(world, alice, &dana.id, &lens(march, recorded)).await;
    assert_eq!(known_open.len(), 1);
    assert!(known_open[0].get("valid_to").is_none(), "{known_open:?}");
    let unrecorded = recorded - chrono::Duration::microseconds(1);
    assert!(
        links(world, alice, &dana.id, &lens(march, unrecorded))
            .await
            .is_empty()
    );
    for as_of in ["2025-12-31T23:59:59Z", "2026-06-01T00:00:00Z"] {
        let outside = links(world, alice, &dana.id, &format!("?as_of={as_of}")).await;
        assert!(outside.is_empty(), "{as_of}: {outside:?}");
    }
    let ahead = Utc::now() + chrono::Duration::hours(1);
    let path = format!("/v1/knowledge/{}/links{}", dana.id, lens(march, ahead));
    let (status, future) = call(&world.app, "GET", &path, alice, None, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{future}");
}

async fn a_link_ends_once_and_after_it_starts(
    world: &World,
    dana: &Item,
    acme: &Item,
    link_id: &str,
) {
    let alice = &world.alice_token;
    let (status, again) = end(world, alice, "time-end-again", dana, link_id, None).await;
    assert_eq!(status, StatusCode::CREATED, "{again}");
    assert_eq!(again["outcome"], "rejected", "a link ends once: {again}");
    let august = Some("2026-08-01T00:00:00Z");
    let (status, later) = link(
        world,
        alice,
        "time-rejoin",
        dana,
        &acme.id,
        WORKS_FOR,
        august,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a link may be asserted again: {later}"
    );
    let (status, early) = end(
        world,
        alice,
        "time-end-early",
        dana,
        &changed_link(&later),
        august,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{early}");
    assert!(
        early.to_string().contains("must end after it starts"),
        "{early}"
    );
}

async fn forget(world: &World, key: &str, target: &Item) {
    let (status, forgotten) = call(
        &world.app,
        "DELETE",
        &format!("/v1/knowledge/{}", target.id),
        &world.alice_token,
        Some(key),
        Some(json!({
            "mode": "forget",
            "expected_revision_id": target.revision,
            "reason": "Erase the fictional system.",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{forgotten}");
    assert_eq!(forgotten["outcome"], "applied", "{forgotten}");
}

#[tokio::test]
async fn forgetting_either_end_erases_its_links_and_their_ends() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    enable_vocabulary(
        &world,
        vec![EntityClass::Service, EntityClass::System],
        vec![EntityPredicate::DependsOn],
    )
    .await;
    let ledger = entity(&world, "erase-ledger", "Ledgerline", "schema:Service").await;
    let cluster = entity(&world, "erase-cluster", "Ledger cluster", "synveda:System").await;
    let invoicer = entity(&world, "erase-invoicer", "Invoicer", "schema:Service").await;
    linked(
        &world,
        "erase-ledger-cluster",
        &ledger,
        &cluster,
        "synveda:dependsOn",
    )
    .await;
    let incoming = linked(
        &world,
        "erase-invoicer-ledger",
        &invoicer,
        &ledger,
        "synveda:dependsOn",
    )
    .await;
    let (status, ended) = end(
        &world,
        &world.alice_token,
        "erase-end",
        &invoicer,
        &incoming,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ended}");
    assert_eq!(
        (
            stored_rows(&world, "entity_links").await,
            stored_rows(&world, "entity_link_ends").await
        ),
        (2, 1)
    );

    forget(&world, "erase-target", &cluster).await;
    assert_eq!(
        stored_rows(&world, "entity_links").await,
        1,
        "the link to the target is gone"
    );
    forget(&world, "erase-source", &invoicer).await;
    assert_eq!(
        stored_rows(&world, "entity_links").await,
        0,
        "the link from the source is gone"
    );
    assert_eq!(
        stored_rows(&world, "entity_link_ends").await,
        0,
        "its end went with it"
    );
}

async fn review(world: &World, change: &Value) -> Value {
    let change_id = change["change_id"].as_str().expect("change id");
    let (status, detail) = call(
        &world.app,
        "GET",
        &format!("/v1/proposals/{change_id}"),
        &world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    let (status, approved) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change_id}/approve"),
        &world.bob_token,
        None,
        Some(json!({"expected_commit": detail["commit"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approved}");
    let (status, applied) = call(
        &world.app,
        "POST",
        &format!("/v1/proposals/{change_id}/apply"),
        &world.bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{applied}");
    applied
}

/// Requires one independent approval for every Knowledge change, then
/// restores the vocabulary the fresh document drops.
async fn require_review(
    world: &World,
    classes: Vec<EntityClass>,
    predicates: Vec<EntityPredicate>,
) {
    let approvals = PackConfig {
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
        .install_policy("link-review", 1, BLANKET, approvals)
        .await;
    let mut tx = rls::begin_tenant_write_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin review binding");
    configuration_support::bind_tenant_pack(&mut tx, world.tenant_id, "link-review").await;
    tx.commit().await.expect("commit review binding");
    enable_vocabulary(world, classes, predicates).await;
}

#[tokio::test]
async fn a_reviewed_link_applies_as_reviewed_unless_an_end_moved() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    let predicates = vec![EntityPredicate::Subject, EntityPredicate::AccountableFor];
    enable_vocabulary(&world, classes.clone(), predicates.clone()).await;
    let decision = typed(
        &world,
        "review-decision",
        "decision",
        "Retire the nightly batch",
    )
    .await;
    let service = entity(&world, "review-service", "Ledgerline", "schema:Service").await;
    let owner = entity(&world, "review-owner", "Dana Ortiz", "schema:Person").await;
    require_review(&world, classes, predicates).await;

    let alice = &world.alice_token;
    let (status, pending) = link(
        &world,
        alice,
        "review-link",
        &decision,
        &service.id,
        SUBJECT,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{pending}");
    assert_eq!(pending["outcome"], "pending_review", "{pending}");
    let link_id = changed_link(&pending);
    assert!(links(&world, alice, &decision.id, "").await.is_empty());
    let applied = review(&world, &pending).await;
    assert_eq!(applied["outcome"], "applied", "{applied}");
    assert_eq!(applied["entity_link_id"], link_id.as_str());
    let reviewed = links(&world, alice, &decision.id, "").await;
    assert_eq!(reviewed.len(), 1);
    assert_eq!(reviewed[0]["id"], link_id.as_str());

    a_second_reviewed_end_is_rejected(&world, &decision, &link_id).await;
    a_target_archived_in_review_is_not_linked(&world, &owner, &service).await;
}

async fn a_second_reviewed_end_is_rejected(world: &World, decision: &Item, link_id: &str) {
    let alice = &world.alice_token;
    let (_, first) = end(world, alice, "review-end-first", decision, link_id, None).await;
    let (_, second) = end(world, alice, "review-end-second", decision, link_id, None).await;
    assert_eq!(first["outcome"], "pending_review", "{first}");
    assert_eq!(second["outcome"], "pending_review", "{second}");
    assert_eq!(review(world, &first).await["outcome"], "applied");
    assert_eq!(
        review(world, &second).await["outcome"],
        "rejected",
        "a link ends once"
    );
}

async fn a_target_archived_in_review_is_not_linked(world: &World, owner: &Item, service: &Item) {
    let alice = &world.alice_token;
    let (_, pending) = link(
        world,
        alice,
        "review-owner-link",
        owner,
        &service.id,
        "synveda:accountableFor",
        None,
    )
    .await;
    assert_eq!(pending["outcome"], "pending_review", "{pending}");
    let (status, archived) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/archive", service.id),
        alice,
        Some("review-archive"),
        Some(json!({"expected_revision_id": service.revision, "reason": "Retired."})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archived}");
    assert_eq!(review(world, &archived).await["outcome"], "applied");
    let rejected = review(world, &pending).await;
    assert_eq!(
        rejected["outcome"], "rejected",
        "an archived target cannot be linked"
    );
    assert!(links(world, alice, &owner.id, "").await.is_empty());
}

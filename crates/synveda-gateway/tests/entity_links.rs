//! Business links between Knowledge items (ADR-0164): governed admission,
//! refusals that reveal nothing, ending, bitemporal reads, erasure, review,
//! recall, graph expansion, and a policy-leak fuzz over all of them.
#[path = "support/context_world.rs"]
mod context_world;
#[path = "support/leak_fuzz.rs"]
mod leak_fuzz;
use chrono::{DateTime, SecondsFormat, Utc};
use context_world::*;
use leak_fuzz::*;
use proptest::prelude::*;
use std::cell::Cell;
use synveda_store::entity_links::MAX_ITEM_LINKS;
use synveda_types::knowledge::{EntityClass, EntityPredicate, LinkEndpoint};
use synveda_types::{
    ContextGraphEdge, ContextGraphStep, EntityLinkId, KnowledgeItemId, Sensitivity,
};

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

/// Internal content titled `title`, whose body is the title as a sentence.
fn internal_content(title: &str, metadata: Value) -> Value {
    json!({
        "title": title,
        "body_markdown": format!("{title}."),
        "summary": title,
        "tags": [],
        "sensitivity": "internal",
        "confidence_permille": 900,
        "metadata": metadata,
    })
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
        "content": internal_content(title, metadata),
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

/// An `entity` Service in alice's personal scope, which bob cannot read.
async fn private_service(world: &World, key: &str) -> Item {
    create(
        world,
        key,
        world.alice_scope,
        "entity",
        "Payroll export",
        declares("schema:Service"),
    )
    .await
}

/// `item`, titled `title`, revised to its content with `changes` applied.
async fn revise(world: &World, key: &str, item: &Item, title: &str, changes: Value) -> Item {
    let mut revised = internal_content(title, json!({}));
    for (field, value) in changes.as_object().expect("content changes") {
        revised[field] = value.clone();
    }
    let (status, edited) = call(
        &world.app,
        "PATCH",
        &format!("/v1/knowledge/{}", item.id),
        &world.alice_token,
        Some(key),
        Some(json!({"expected_revision_id": item.revision, "content": revised})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{edited}");
    assert_eq!(edited["outcome"], "applied", "{edited}");
    Item {
        id: item.id.clone(),
        revision: edited["revision_id"]
            .as_str()
            .expect("revision id")
            .to_owned(),
    }
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

/// Alice verifies `item`, which writes it a new revision.
async fn verify(world: &World, key: &str, item: &Item) -> Value {
    let (status, verified) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/verify", item.id),
        &world.alice_token,
        Some(key),
        Some(json!({
            "expected_revision_id": item.revision,
            "verification_metadata": {"method": "reviewed"},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{verified}");
    verified
}

async fn a_stale_source_is_rejected(world: &World, decision: &Item, person: &Item) {
    verify(world, "rule-verify", decision).await;
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
    let unknown = Item {
        id: decision.id.clone(),
        revision: uuid::Uuid::now_v7().to_string(),
    };
    let (status, refusal) = link(
        world,
        &world.alice_token,
        "rule-unknown-revision",
        &unknown,
        &person.id,
        WAS_ATTRIBUTED_TO,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a revision admission never saw: {refusal}"
    );
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
    let hidden = private_service(&world, "hide-hidden").await;

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

/// Records `count` links from `source` to `target` straight into the store,
/// as that many applied link commands would.
async fn seed_links(world: &World, source: &Item, target: &Item, count: i64) {
    let id = |item: &Item| item.id.parse::<uuid::Uuid>().expect("item id");
    let revision = source.revision.parse::<uuid::Uuid>().expect("revision id");
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    sqlx::query(
        "insert into entity_links
             (id, tenant_id, source_item_id, target_item_id, asserting_revision_id,
              predicate, valid_from, created_by)
         select gen_random_uuid(), $1, $2, $3, $4, $5, now() - interval '1 day', 'seed'
           from generate_series(1, $6)",
    )
    .bind(world.tenant_id.as_uuid())
    .bind(id(source))
    .bind(id(target))
    .bind(revision)
    .bind(SUBJECT)
    .bind(count)
    .execute(&mut *tx)
    .await
    .expect("seed links");
    tx.commit().await.expect("commit seeded links");
}

#[tokio::test]
async fn an_item_holds_a_thousand_links_and_hidden_ones_leave_no_trace_in_its_read() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let predicates = vec![EntityPredicate::Subject];
    enable_vocabulary(&world, vec![EntityClass::Service], predicates).await;
    let decision = typed(&world, "cap-decision", "decision", "Merge the ledgers").await;
    let visible = entity(&world, "cap-visible", "Ledgerline", "schema:Service").await;
    let spare = entity(&world, "cap-spare", "Billing hub", "schema:Service").await;
    let hidden = private_service(&world, "cap-hidden").await;
    seed_links(&world, &decision, &hidden, MAX_ITEM_LINKS - 1).await;
    assert!(
        links(&world, &world.bob_token, &decision.id, "")
            .await
            .is_empty(),
        "links to an end bob cannot read leave no trace"
    );

    linked(&world, "cap-last", &decision, &visible, SUBJECT).await;
    assert_eq!(
        links(&world, &world.alice_token, &decision.id, "")
            .await
            .len(),
        MAX_ITEM_LINKS as usize,
        "a read at the bound returns every link"
    );
    assert_eq!(
        links(&world, &world.bob_token, &decision.id, "")
            .await
            .len(),
        1
    );
    let (status, over) = link(
        &world,
        &world.alice_token,
        "cap-over",
        &decision,
        &spare.id,
        SUBJECT,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{over}");
    assert_eq!(over["outcome"], "rejected", "the decision is full: {over}");
    assert_eq!(stored_rows(&world, "entity_links").await, MAX_ITEM_LINKS);
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
    bind_policy(world, "link-review", BLANKET, approvals).await;
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
    a_target_verified_in_review_is_not_linked(&world, &decision, &owner).await;
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
    let archived = archive(world, "review-archive", service).await;
    assert_eq!(review(world, &archived).await["outcome"], "applied");
    let rejected = review(world, &pending).await;
    assert_eq!(
        rejected["outcome"], "rejected",
        "an archived target cannot be linked"
    );
    assert!(links(world, alice, &owner.id, "").await.is_empty());
}

/// Alice archives `item` at its current revision.
async fn archive(world: &World, key: &str, item: &Item) -> Value {
    let (status, archived) = call(
        &world.app,
        "POST",
        &format!("/v1/knowledge/{}/archive", item.id),
        &world.alice_token,
        Some(key),
        Some(json!({"expected_revision_id": item.revision, "reason": "Retired."})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{archived}");
    archived
}

/// Admission checked the target's class at the revision it read, so a target
/// that moves on before the change applies is not linked.
async fn a_target_verified_in_review_is_not_linked(world: &World, decision: &Item, owner: &Item) {
    let (_, pending) = link(
        world,
        &world.alice_token,
        "review-moved-link",
        decision,
        &owner.id,
        SUBJECT,
        None,
    )
    .await;
    assert_eq!(pending["outcome"], "pending_review", "{pending}");
    let verified = verify(world, "review-moved-verify", owner).await;
    assert_eq!(review(world, &verified).await["outcome"], "applied");
    assert_eq!(
        review(world, &pending).await["outcome"],
        "rejected",
        "the target moved on after admission"
    );
}

/// The summary hit for `item` in `token`'s recall of `query` through
/// `session`.
async fn recall_hit(world: &World, token: &str, session: &str, query: &str, item: &Item) -> Value {
    let (status, recalled) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{session}/knowledge-query"),
        token,
        None,
        Some(json!({"query": query})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{recalled}");
    recalled["summaries"]
        .as_array()
        .expect("summaries")
        .iter()
        .find(|hit| hit["id"] == item.id.as_str())
        .cloned()
        .unwrap_or_else(|| panic!("no hit for {}: {recalled}", item.id))
}

fn hit_links(hit: &Value) -> Vec<Value> {
    hit.get("links")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn link_ids<'a>(links: impl IntoIterator<Item = &'a Value>, key: &str) -> Vec<String> {
    let mut ids: Vec<String> = links
        .into_iter()
        .map(|link| link[key].as_str().expect("link id").to_owned())
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn recall_shows_each_hits_current_links_both_ways() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    let predicates = vec![
        EntityPredicate::Owns,
        EntityPredicate::WasAttributedTo,
        EntityPredicate::Subject,
    ];
    enable_vocabulary(&world, classes, predicates).await;
    let decision = typed(
        &world,
        "recall-decision",
        "decision",
        "Stop zephyrine retries",
    )
    .await;
    let service = entity(
        &world,
        "recall-service",
        "Quillfeather billing",
        "schema:Service",
    )
    .await;
    let dana = entity(&world, "recall-dana", "Dana Ortiz", "schema:Person").await;
    let about = linked(&world, "recall-about", &decision, &service, SUBJECT).await;
    let by = linked(&world, "recall-by", &decision, &dana, WAS_ATTRIBUTED_TO).await;
    let owns = linked(&world, "recall-owns", &dana, &service, "schema:owns").await;
    let (alice, session) = (&world.alice_token, &world.alice_session);

    let hit = recall_hit(&world, alice, session, "zephyrine", &decision).await;
    assert_eq!(
        hit_links(&hit),
        vec![
            json!({"link_id": about, "predicate": SUBJECT, "direction": "outbound",
                   "item_id": service.id, "knowledge_type": "entity",
                   "entity_class": "schema:Service", "title": "Quillfeather billing"}),
            json!({"link_id": by, "predicate": WAS_ATTRIBUTED_TO, "direction": "outbound",
                   "item_id": dana.id, "knowledge_type": "entity",
                   "entity_class": "schema:Person", "title": "Dana Ortiz"}),
        ],
        "{hit}"
    );
    assert!(hit.get("links_truncated").is_none(), "{hit}");
    let hit = recall_hit(&world, alice, session, "Quillfeather", &service).await;
    let incoming = hit_links(&hit);
    assert_eq!(incoming[0]["direction"], "inbound", "{hit}");
    assert_eq!(incoming[0]["title"], "Stop zephyrine retries", "{hit}");
    assert!(incoming[0].get("entity_class").is_none(), "{hit}");
    assert_eq!(incoming[1]["link_id"], owns.as_str(), "{hit}");

    for (token, session) in [(alice, session), (&world.bob_token, &world.bob_session)] {
        let recalled = recall_hit(&world, token, session, "Quillfeather", &service).await;
        let read = links(&world, token, &service.id, "").await;
        assert_eq!(
            link_ids(&hit_links(&recalled), "link_id"),
            link_ids(&read, "id")
        );
    }
    let (status, ended) = end(&world, alice, "recall-end", &dana, &owns, None).await;
    assert_eq!(status, StatusCode::CREATED, "{ended}");
    let hit = recall_hit(&world, alice, session, "Quillfeather", &service).await;
    assert_eq!(link_ids(&hit_links(&hit), "link_id"), vec![about]);
}

#[tokio::test]
async fn hidden_links_leave_no_trace_in_recall() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let predicates = vec![EntityPredicate::Subject];
    enable_vocabulary(&world, vec![EntityClass::Service], predicates).await;
    let decision = typed(
        &world,
        "trace-decision",
        "decision",
        "Merge zephyrine ledgers",
    )
    .await;
    let visible = entity(&world, "trace-visible", "Ledgerline", "schema:Service").await;
    let hidden = private_service(&world, "trace-hidden").await;
    linked(&world, "trace-visible-link", &decision, &visible, SUBJECT).await;
    let (bob, session) = (&world.bob_token, &world.bob_session);

    let before = recall_hit(&world, bob, session, "zephyrine", &decision).await;
    let concealed = linked(&world, "trace-hidden-link", &decision, &hidden, SUBJECT).await;
    let after = recall_hit(&world, bob, session, "zephyrine", &decision).await;
    assert_eq!(after, before, "a hidden link changes nothing bob sees");
    let details = audit_details(&world, "context.knowledge_query").await;
    assert_eq!(details.len(), 2);
    assert_eq!(details[1], details[0], "nor anything audit records of him");
    assert_eq!(details[1]["visible_links"], 1, "{}", details[1]);
    assert!(
        !details[1].to_string().contains(&concealed),
        "{}",
        details[1]
    );
    let alice = &world.alice_token;
    let seen = recall_hit(&world, alice, &world.alice_session, "zephyrine", &decision).await;
    assert_eq!(hit_links(&seen).len(), 2, "{seen}");

    // Bob's sixteen visible links all show, whatever hides among them.
    seed_links(&world, &decision, &hidden, 20).await;
    seed_links(&world, &decision, &visible, 15).await;
    let full = recall_hit(&world, bob, session, "zephyrine", &decision).await;
    assert_eq!(hit_links(&full).len(), 16, "{full}");
    assert!(full.get("links_truncated").is_none(), "{full}");
    let seen = recall_hit(&world, alice, &world.alice_session, "zephyrine", &decision).await;
    assert_eq!(hit_links(&seen).len(), 16, "{seen}");
    assert_eq!(seen["links_truncated"], true, "{seen}");
    seed_links(&world, &decision, &visible, 1).await;
    let over = recall_hit(&world, bob, session, "zephyrine", &decision).await;
    assert_eq!(hit_links(&over).len(), 16, "{over}");
    assert_eq!(over["links_truncated"], true, "{over}");
}

#[tokio::test]
async fn recall_leaves_out_links_to_archived_ends_and_disabled_predicates() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    let predicates = vec![EntityPredicate::WasAttributedTo, EntityPredicate::Subject];
    enable_vocabulary(&world, classes.clone(), predicates).await;
    let decision = typed(
        &world,
        "inert-decision",
        "decision",
        "Keep zephyrine receipts",
    )
    .await;
    let service = entity(&world, "inert-service", "Receipt vault", "schema:Service").await;
    let dana = entity(&world, "inert-dana", "Dana Ortiz", "schema:Person").await;
    linked(&world, "inert-about", &decision, &service, SUBJECT).await;
    let by = linked(&world, "inert-by", &decision, &dana, WAS_ATTRIBUTED_TO).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let archived = archive(&world, "inert-archive", &service).await;
    assert_eq!(archived["outcome"], "applied", "{archived}");

    let hit = recall_hit(&world, alice, session, "zephyrine", &decision).await;
    assert_eq!(link_ids(&hit_links(&hit), "link_id"), vec![by]);
    enable_vocabulary(&world, classes, vec![EntityPredicate::Subject]).await;
    let hit = recall_hit(&world, alice, session, "zephyrine", &decision).await;
    assert!(hit.get("links").is_none(), "{hit}");
    assert_eq!(
        links(&world, alice, &decision.id, "").await.len(),
        2,
        "the read keeps the recorded history"
    );
}

/// The entry for `item` in the detail's `list`.
fn listed<'a>(detail: &'a Value, list: &str, item: &Item) -> Option<&'a Value> {
    detail[list]
        .as_array()
        .expect("listed entries")
        .iter()
        .find(|entry| entry["knowledge_item_id"] == item.id.as_str())
}

fn candidate<'a>(detail: &'a Value, item: &Item) -> Option<&'a Value> {
    listed(detail, "candidates", item)
}

fn path(candidate: &Value) -> Vec<Value> {
    candidate["graph_path"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Whether the run left out edges past an item's fan-out.
fn is_fanout_truncated(run: &Value) -> bool {
    run["degraded"]
        .as_array()
        .expect("degraded")
        .iter()
        .any(|value| value == "graph_fanout_truncated")
}

#[tokio::test]
async fn expansion_follows_business_links_only_when_configuration_includes_them() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let predicates = vec![EntityPredicate::WasAttributedTo, EntityPredicate::Subject];
    enable_vocabulary(
        &world,
        vec![EntityClass::Person, EntityClass::Service],
        predicates,
    )
    .await;
    let service = entity(
        &world,
        "walk-service",
        "Quillfeather billing",
        "schema:Service",
    )
    .await;
    let decision = typed(
        &world,
        "walk-decision",
        "decision",
        "Stop zephyrine retries",
    )
    .await;
    let dana = entity(&world, "walk-dana", "Dana Ortiz", "schema:Person").await;
    let about = linked(&world, "walk-about", &decision, &service, SUBJECT).await;
    let by = linked(&world, "walk-by", &decision, &dana, WAS_ATTRIBUTED_TO).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);

    set_traversal(&world, false, 2, 8).await;
    let (run, off) = traverse(&world, alice, session, "walk-off", "Quillfeather").await;
    assert_eq!(run["graph_version"], "knowledge-relations-v1", "{run}");
    assert!(candidate(&off, &service).is_some(), "the anchor: {off}");
    assert!(candidate(&off, &decision).is_none(), "{off}");

    set_traversal(&world, true, 2, 8).await;
    let (run, on) = traverse(&world, alice, session, "walk-on", "Quillfeather").await;
    assert_eq!(run["graph_version"], "knowledge-relations-entity-links-v1");
    let rendered = run["rendered"].as_str().expect("rendered");
    assert!(rendered.contains("Stop zephyrine retries."), "{run}");
    assert!(rendered.contains("Dana Ortiz."), "{run}");
    let reached = candidate(&on, &decision).expect("what was decided about it");
    assert_eq!(reached["reason_codes"][0], "graph_expansion", "{reached}");
    let step = &path(reached)[0];
    assert_eq!(step["predicate"], SUBJECT, "{reached}");
    assert_eq!(step["direction"], "inbound", "{reached}");
    assert_eq!(step["entity_link_id"], about.as_str(), "{reached}");
    assert_eq!(step["asserting_revision_id"], decision.revision.as_str());
    assert!(step.get("relation_type").is_none(), "{reached}");
    assert!(step.get("relation_id").is_none(), "{reached}");
    assert_eq!(step["edge_weight_micros"], 500_000, "{reached}");
    let delivered = listed(&on, "selections", &decision).expect("the decision is delivered");
    assert_eq!(delivered["graph_path"][0], *step, "{delivered}");
    let named = candidate(&on, &dana).expect("who decided");
    let steps = path(named);
    assert_eq!(steps.len(), 2, "{named}");
    assert_eq!(steps[0], *step, "both share the first step");
    assert_eq!(steps[1]["predicate"], WAS_ATTRIBUTED_TO, "{named}");
    assert_eq!(steps[1]["direction"], "outbound", "{named}");
    assert_eq!(steps[1]["entity_link_id"], by.as_str(), "{named}");
    assert_eq!(named["scores"]["edge_weight_micros"], 1_000_000, "{named}");
    assert_eq!(named["scores"]["hop_penalty_micros"], 200_000, "{named}");

    hashes_only_keeps_each_link_steps_predicate(&world).await;
}

/// A hashes-only trace keeps each step's predicate and evidence hash and no
/// address.
async fn hashes_only_keeps_each_link_steps_predicate(world: &World) {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin hashes-only trace");
    let root = scopes::tenant_root(&mut *tx, world.tenant_id)
        .await
        .expect("read tenant root")
        .expect("tenant root");
    configuration_support::set_trace_retention(
        &mut tx,
        world.tenant_id,
        root.id,
        TraceRetentionMode::HashesOnly,
    )
    .await;
    tx.commit().await.expect("commit hashes-only trace");
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let (_, hashed) = traverse(world, alice, session, "walk-hashes", "Quillfeather").await;
    let steps: Vec<Value> = hashed["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .flat_map(path)
        .collect();
    assert_eq!(steps.len(), 3, "{hashed}");
    assert!(steps.iter().all(|step| {
        step["predicate"].is_string()
            && step.get("entity_link_id").is_none()
            && step.get("from_item_id").is_none()
            && step["relation_hash"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
    }));
}

#[tokio::test]
async fn a_path_mixes_links_and_relations_and_the_best_score_wins() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let predicates = vec![EntityPredicate::Subject];
    enable_vocabulary(&world, vec![EntityClass::Service], predicates).await;
    let gateway = entity(
        &world,
        "mix-gateway",
        "Brindlemoor gateway",
        "schema:Service",
    )
    .await;
    let paused = typed(&world, "mix-paused", "decision", "Pause gateway deploys").await;
    let freeze = typed(&world, "mix-freeze", "fact", "Deploy freeze evidence").await;
    let budget = typed(&world, "mix-budget", "fact", "Gateway retry budget").await;
    let capped = typed(&world, "mix-capped", "decision", "Cap gateway retries").await;
    linked(&world, "mix-paused-about", &paused, &gateway, SUBJECT).await;
    linked(&world, "mix-capped-about", &capped, &gateway, SUBJECT).await;
    let pair = |item: &Item| (item.id.clone(), item.revision.clone());
    for (key, from, to) in [
        ("mix-paused-freeze", &paused, &freeze),
        ("mix-gateway-budget", &gateway, &budget),
        ("mix-budget-capped", &budget, &capped),
    ] {
        let (from, to) = (pair(from), pair(to));
        support_relation(&world, key, (&from.0, &from.1), (&to.0, &to.1)).await;
    }
    set_traversal(&world, true, 2, 8).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let (_, run) = traverse(&world, alice, session, "mix-run", "Brindlemoor").await;

    let mixed = path(candidate(&run, &freeze).expect("reached past a link"));
    assert_eq!(mixed.len(), 2, "{run}");
    assert_eq!(mixed[0]["predicate"], SUBJECT, "{run}");
    assert_eq!(mixed[1]["relation_type"], "supports", "{run}");
    // Reached by its link at one hop (0.50) and by two `supports` relations
    // at two hops (1.40, less a further hop penalty): the relations win.
    let best = candidate(&run, &capped).expect("reached both ways");
    let steps = path(best);
    assert_eq!(steps.len(), 2, "{best}");
    assert!(
        steps.iter().all(|step| step["relation_type"] == "supports"),
        "{best}"
    );
    assert_eq!(best["scores"]["edge_weight_micros"], 1_400_000, "{best}");
}

#[tokio::test]
async fn an_ended_link_or_a_disabled_predicate_stops_expansion_and_its_replay() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    let predicates = vec![EntityPredicate::Owns, EntityPredicate::Subject];
    enable_vocabulary(&world, classes, predicates).await;
    let service = entity(
        &world,
        "inert-walk-service",
        "Wrenfield ledger",
        "schema:Service",
    )
    .await;
    let decision = typed(
        &world,
        "inert-walk-decision",
        "decision",
        "Freeze ledger exports",
    )
    .await;
    // Dana's links are governed by alice's own scope, outside the run's.
    let dana = create(
        &world,
        "inert-walk-dana",
        world.alice_scope,
        "entity",
        "Dana Ortiz",
        declares("schema:Person"),
    )
    .await;
    let about = linked(&world, "inert-walk-about", &decision, &service, SUBJECT).await;
    linked(&world, "inert-walk-owns", &dana, &service, "schema:owns").await;
    set_traversal(&world, true, 2, 8).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);

    let (status, original) = post_run(&world, "inert-walk-run", "Wrenfield").await;
    assert_eq!(status, StatusCode::CREATED, "{original}");
    assert!(
        original["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("Freeze ledger exports.")),
        "{original}"
    );
    let (status, replayed) = post_run(&world, "inert-walk-run", "Wrenfield").await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed, original, "the saved delivery replays exactly");

    let (status, ended) = end(&world, alice, "inert-walk-end", &decision, &about, None).await;
    assert_eq!(status, StatusCode::CREATED, "{ended}");
    let (status, refused) = post_run(&world, "inert-walk-run", "Wrenfield").await;
    let kept = detail(&world, alice, original["id"].as_str().expect("run id")).await;
    let reached = candidate(&kept, &decision).expect("the inspector keeps the run's path");
    assert_eq!(path(reached)[0]["entity_link_id"], about.as_str(), "{kept}");
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a delivery over an ended link: {refused}"
    );
    let (_, after) = traverse(&world, alice, session, "inert-walk-ended", "Wrenfield").await;
    assert!(candidate(&after, &decision).is_none(), "{after}");
    assert!(candidate(&after, &dana).is_some(), "{after}");

    a_disabled_predicate_refuses_its_replay(&world, &service, &dana).await;
}

/// Disabling `schema:owns` where Dana's links are governed refuses a saved
/// delivery over her link, though the run's own Configuration is unchanged.
async fn a_disabled_predicate_refuses_its_replay(world: &World, service: &Item, dana: &Item) {
    let (status, owned) = post_run(world, "inert-walk-owned", "Wrenfield").await;
    assert_eq!(status, StatusCode::CREATED, "{owned}");
    assert!(
        owned["rendered"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("Dana Ortiz.")),
        "{owned}"
    );
    let (status, replayed) = post_run(world, "inert-walk-owned", "Wrenfield").await;
    assert_eq!(status, StatusCode::OK, "{replayed}");

    let run_configuration = effective_hash(world, world.project_scope).await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin alice's own Configuration");
    // A Personal document of its own, under the tenant's pack, enables no
    // predicate.
    configuration_support::bind_pack(
        &mut tx,
        world.tenant_id,
        world.alice_scope,
        synveda_policy::STANDARD,
    )
    .await;
    tx.commit().await.expect("commit alice's own Configuration");
    assert_eq!(
        effective_hash(world, world.project_scope).await,
        run_configuration,
        "the run's Configuration is unchanged"
    );
    let (status, refused) = post_run(world, "inert-walk-owned", "Wrenfield").await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a delivery over a disabled predicate: {refused}"
    );
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let (_, disabled) = traverse(world, alice, session, "inert-walk-disabled", "Wrenfield").await;
    assert!(candidate(&disabled, dana).is_none(), "{disabled}");
    assert!(candidate(&disabled, service).is_some(), "{disabled}");
}

/// The digest of the Configuration in effect at `scope`.
async fn effective_hash(world: &World, scope: ScopeId) -> String {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin effective Configuration read");
    let effective =
        synveda_store::configuration::effective_at_scope(&mut tx, world.tenant_id, scope)
            .await
            .expect("read effective Configuration");
    tx.commit().await.expect("end effective Configuration read");
    effective.content_hash
}

/// Sorts every `determining` policy list, which the PDP reports in no fixed
/// order.
fn sort_determining(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, entry) in object.iter_mut() {
                match entry {
                    Value::Array(policies) if key == "determining" => {
                        policies.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
                    }
                    _ => sort_determining(entry),
                }
            }
        }
        Value::Array(entries) => entries.iter_mut().for_each(sort_determining),
        _ => {}
    }
}

/// `value` without the object keys named.
fn without(value: &Value, keys: &[&str]) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in keys {
            object.remove(*key);
        }
    }
    value
}

/// A rendered line as two runs compare it: a manifest's header without the
/// run's own id and instant, after checking it names `run` and its instant,
/// and any other line as it is.
fn rendered_line(line: &str, run: &Value) -> Value {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(mut header)) if header.contains_key("manifest_id") => {
            let named = header.remove("manifest_id");
            assert_eq!(
                named.as_ref(),
                Some(&run["id"]),
                "the manifest names its run: {line}"
            );
            let instant = |value: &Value| {
                let text = value.as_str().expect("an instant");
                DateTime::parse_from_rfc3339(text).expect("an RFC 3339 instant")
            };
            let as_of = header.remove("as_of").expect("the manifest's instant");
            assert_eq!(
                instant(&as_of),
                instant(&run["as_of"]),
                "the manifest is as of its run: {line}"
            );
            Value::Object(header)
        }
        _ => json!(line),
    }
}

/// What a run shows its reader and what audit records of it, without the
/// run's own ids and instants.
fn run_shape(run: &Value, detail: &Value, audit: &Value) -> Value {
    let each = |values: &Value, keys: &[&str]| {
        values
            .as_array()
            .expect("array")
            .iter()
            .map(|value| without(value, keys))
            .collect::<Vec<_>>()
    };
    let rendered = run["rendered"].as_str().expect("rendered");
    // The rendered block opens with the run's own instant, which its hash
    // covers.
    let body = rendered.split_once('\n').map_or("", |(_, rest)| rest);
    let lines: Vec<Value> = body.lines().map(|line| rendered_line(line, run)).collect();
    let mut shown = without(run, &["id", "as_of", "created_at", "block_hash"]);
    shown["rendered"] = json!(lines);
    let mut audit = without(audit, &["session_id", "context_run_id"]);
    sort_determining(&mut audit);
    json!({
        "run": shown,
        "candidates": each(&detail["candidates"], &["id"]),
        "selections": each(&detail["selections"], &["id", "context_candidate_id"]),
        "policy_exclusion_message": detail["policy_exclusion_message"],
        "audit": audit,
    })
}

async fn shaped_run(world: &World, key: &str, query: &str) -> Value {
    let (run, detail) = traverse(world, &world.bob_token, &world.bob_session, key, query).await;
    let audit = run_audit(world, &run).await;
    run_shape(&run, &detail, &audit)
}

/// What audit recorded of the candidates `run` retrieved.
async fn run_audit(world: &World, run: &Value) -> Value {
    audit_events(world)
        .await
        .into_iter()
        .find(|event| {
            event.action == "context.candidates.retrieved"
                && event.payload["context_run_id"] == run["id"]
        })
        .expect("this run's candidate audit")
        .payload
}

#[tokio::test]
async fn hidden_link_ends_leave_no_trace_in_expansion() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    enable_vocabulary(
        &world,
        vec![EntityClass::Service],
        vec![EntityPredicate::Subject],
    )
    .await;
    let decision = typed(
        &world,
        "hidden-walk-decision",
        "decision",
        "Merge zephyrine ledgers",
    )
    .await;
    let visible = entity(
        &world,
        "hidden-walk-visible",
        "Ledgerline",
        "schema:Service",
    )
    .await;
    let hidden = private_service(&world, "hidden-walk-hidden").await;
    linked(
        &world,
        "hidden-walk-visible-link",
        &decision,
        &visible,
        SUBJECT,
    )
    .await;
    set_traversal(&world, true, 1, 2).await;

    let before = shaped_run(&world, "hidden-walk-before", "zephyrine").await;
    let concealed = linked(
        &world,
        "hidden-walk-hidden-link",
        &decision,
        &hidden,
        SUBJECT,
    )
    .await;
    let after = shaped_run(&world, "hidden-walk-after", "zephyrine").await;
    assert_eq!(
        after, before,
        "a hidden link changes nothing bob sees or audit records"
    );
    assert!(!after.to_string().contains(&concealed), "{after}");
    assert!(!after.to_string().contains(&hidden.id), "{after}");
    let reached = candidate(&after, &visible).expect("the visible end is reached");
    assert_eq!(path(reached).len(), 1, "{after}");

    fan_out_counts_only_visible_links(&world, &decision, &visible, &hidden).await;
}

#[tokio::test]
async fn a_link_asserted_by_a_revision_the_reader_cannot_see_leaves_no_trace() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    // Bob reads anything in the tenant but a confidential revision.
    let policy = format!(
        "{BLANKET}\nforbid (principal == Synveda::Principal::\"{}/{BOB}\", \
         action == Synveda::Action::\"KnowledgeRead\", resource) \
         when {{ context.sensitivity == \"confidential\" }};",
        world.tenant_id,
    );
    bind_policy(
        &world,
        "mem11-hidden-asserter",
        &policy,
        PackConfig::default(),
    )
    .await;
    let services = vec![EntityClass::Service];
    enable_vocabulary(&world, services, vec![EntityPredicate::Subject]).await;
    let relay = entity(
        &world,
        "asserter-relay",
        "Quillmarsh relay",
        "schema:Service",
    )
    .await;
    let rotate = typed(
        &world,
        "asserter-rotate",
        "decision",
        "Rotate relay certificates",
    )
    .await;
    linked(&world, "asserter-rotate-about", &rotate, &relay, SUBJECT).await;
    let title = "Retire legacy ciphers";
    let retire = typed(&world, "asserter-retire", "decision", title).await;
    let confidential = json!({"sensitivity": "confidential"});
    let hidden = revise(&world, "asserter-hidden", &retire, title, confidential).await;
    revise(&world, "asserter-shown", &hidden, title, json!({})).await;
    set_traversal(&world, true, 1, 1).await;

    let before = shaped_run(&world, "asserter-before", "Quillmarsh").await;
    // A link that the confidential revision asserted, which bob cannot read
    // though he reads both ends.
    seed_links(&world, &hidden, &relay, 1).await;
    let after = shaped_run(&world, "asserter-after", "Quillmarsh").await;
    assert_eq!(
        after, before,
        "a link bob cannot fully read changes nothing he sees or audit records"
    );
    assert!(!is_fanout_truncated(&after["run"]), "{after}");
    assert!(!after.to_string().contains(&retire.id), "{after}");
}

#[tokio::test]
async fn expansion_leaves_a_link_end_outside_its_valid_window() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let services = vec![EntityClass::Service];
    enable_vocabulary(&world, services, vec![EntityPredicate::Subject]).await;
    let gateway = entity(
        &world,
        "window-gateway",
        "Ostravel gateway",
        "schema:Service",
    )
    .await;
    let title = "Adopt mutual TLS";
    let adopt = typed(&world, "window-adopt", "decision", title).await;
    linked(&world, "window-adopt-about", &adopt, &gateway, SUBJECT).await;
    set_traversal(&world, true, 1, 8).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let (_, held) = traverse(&world, alice, session, "window-held", "Ostravel").await;
    assert!(candidate(&held, &adopt).is_some(), "{held}");

    let tomorrow =
        (Utc::now() + chrono::Duration::days(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
    let upcoming = json!({"valid_from": tomorrow});
    revise(&world, "window-upcoming", &adopt, title, upcoming).await;
    let (_, early) = traverse(&world, alice, session, "window-early", "Ostravel").await;
    assert!(
        candidate(&early, &adopt).is_none(),
        "not yet valid: {early}"
    );
    assert!(candidate(&early, &gateway).is_some(), "{early}");
}

/// With a fan-out of two, bob's limit counts only the links he can see.
async fn fan_out_counts_only_visible_links(
    world: &World,
    decision: &Item,
    visible: &Item,
    hidden: &Item,
) {
    let bob = (&world.bob_token, &world.bob_session);
    seed_links(world, decision, hidden, 3).await;
    seed_links(world, decision, visible, 1).await;
    let (run, _) = traverse(world, bob.0, bob.1, "fan-out-bob", "zephyrine").await;
    assert!(!is_fanout_truncated(&run), "{run}");
    let alice = (&world.alice_token, &world.alice_session);
    let (run, _) = traverse(world, alice.0, alice.1, "fan-out-alice", "zephyrine").await;
    assert!(is_fanout_truncated(&run), "{run}");
    seed_links(world, decision, visible, 1).await;
    let (run, _) = traverse(world, bob.0, bob.1, "fan-out-bob-over", "zephyrine").await;
    assert!(is_fanout_truncated(&run), "{run}");
}

/// The business-link steps a run retained.
async fn link_steps(world: &World, run: &Value) -> Vec<ContextGraphStep> {
    let run_id = run["id"].as_str().expect("run id").parse().expect("run id");
    let mut tx = tenant_fixture::begin(&world.state.pool, world.tenant_id).await;
    let steps = synveda_store::context::graph_steps_for_run(&mut *tx, world.tenant_id, run_id)
        .await
        .expect("read graph steps");
    tx.commit().await.expect("commit graph-step read");
    steps
        .into_iter()
        .filter(|step| step.edge.predicate().is_some())
        .collect()
}

#[tokio::test]
async fn erasing_either_end_scrubs_the_link_steps_a_run_retained() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    enable_vocabulary(
        &world,
        vec![EntityClass::Service],
        vec![EntityPredicate::Subject],
    )
    .await;
    set_traversal(&world, true, 1, 8).await;
    let (alice, session) = (&world.alice_token, &world.alice_session);
    let cases = [
        (
            "source",
            "scrub-source",
            "Tallowmere",
            "Rotate signing keys",
        ),
        (
            "target",
            "scrub-target",
            "Quenbridge",
            "Pause nightly exports",
        ),
    ];
    for (end, key, name, decided) in cases {
        let service = entity(&world, &format!("{key}-service"), name, "schema:Service").await;
        let decision = typed(&world, &format!("{key}-decision"), "decision", decided).await;
        linked(&world, &format!("{key}-link"), &decision, &service, SUBJECT).await;
        let (run, _) = traverse(&world, alice, session, &format!("{key}-run"), name).await;
        let retained = link_steps(&world, &run).await;
        assert_eq!(retained.len(), 1, "{run}");
        assert!(retained[0].edge.entity_link_id().is_some(), "{retained:?}");
        assert!(retained[0].from_item_id.is_some(), "{retained:?}");

        forget(
            &world,
            &format!("{key}-forget"),
            if end == "source" { &decision } else { &service },
        )
        .await;
        let scrubbed = link_steps(&world, &run).await;
        assert_eq!(scrubbed.len(), 1, "{scrubbed:?}");
        assert_eq!(
            scrubbed[0].edge,
            ContextGraphEdge::EntityLink {
                predicate: EntityPredicate::Subject,
                entity_link_id: None,
            },
            "erasing the {end} keeps the step's predicate and clears its link"
        );
        assert_eq!(
            (scrubbed[0].from_item_id, scrubbed[0].to_item_id),
            (None, None),
            "and its addresses"
        );
        detail(&world, alice, run["id"].as_str().expect("run id")).await;
    }
}

// ── Policy-leak fuzz across classes, predicates, roles and scopes ───────────

const LINK_LEAK_CASES: usize = 32;

/// Every kind some predicate's endpoint rule names.
fn endpoint_kinds() -> Vec<LinkEndpoint> {
    let mut kinds = Vec::new();
    for predicate in EntityPredicate::ALL {
        for kind in predicate.sources().iter().chain(predicate.targets()) {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
    }
    kinds
}

/// Which end of a link the hub is.
#[derive(Clone, Copy)]
enum HubEnd {
    Source,
    Target,
}

/// A predicate whose rule admits the hub at one end, and the kinds its other
/// end admits.
struct HubRule {
    predicate: EntityPredicate,
    hub_end: HubEnd,
    far_kinds: &'static [LinkEndpoint],
}

fn hub_rules(hub: LinkEndpoint) -> Vec<HubRule> {
    let mut rules = Vec::new();
    for predicate in EntityPredicate::ALL.iter().copied() {
        if predicate.sources().contains(&hub) {
            rules.push(HubRule {
                predicate,
                hub_end: HubEnd::Source,
                far_kinds: predicate.targets(),
            });
        }
        if predicate.targets().contains(&hub) {
            rules.push(HubRule {
                predicate,
                hub_end: HubEnd::Target,
                far_kinds: predicate.sources(),
            });
        }
    }
    rules
}

/// One link a case seeds from a hub. `rule` and `kind` pick, modulo what the
/// hub's kind admits, the predicate and the far end's kind.
#[derive(Debug)]
struct FarSpec {
    rule: usize,
    kind: usize,
    placement: Placement,
    sensitivity: Sensitivity,
}

#[derive(Debug)]
struct HubSpec {
    kind: LinkEndpoint,
    links: Vec<FarSpec>,
}

#[derive(Debug)]
struct LinkLeakCase {
    pack: LeakPack,
    bob_project_role: Option<RoleKey>,
    fan_out: u32,
    manifest: bool,
    hubs: Vec<HubSpec>,
}

/// Weighted so that many far ends are ones Bob may not read. Restricted
/// Knowledge is left out: nobody may read it, so nobody can link it.
fn link_leak_case() -> impl Strategy<Value = LinkLeakCase> {
    let placement = prop_oneof![
        1 => Just(Placement::Root),
        1 => Just(Placement::Workspace),
        2 => Just(Placement::Project),
        3 => Just(Placement::AlicePrivate),
    ];
    let sensitivity = prop::sample::select(vec![
        Sensitivity::Public,
        Sensitivity::Internal,
        Sensitivity::Confidential,
    ]);
    let far = (0..64usize, 0..64usize, placement, sensitivity).prop_map(
        |(rule, kind, placement, sensitivity)| FarSpec {
            rule,
            kind,
            placement,
            sensitivity,
        },
    );
    let hub = (
        prop::sample::select(endpoint_kinds()),
        prop::collection::vec(far, 2..=5),
    )
        .prop_map(|(kind, links)| HubSpec { kind, links });
    (
        prop::sample::select(LeakPack::ALL.to_vec()),
        prop::sample::select(LEAK_ROLES.to_vec()),
        prop_oneof![2 => Just(1u32), 2 => Just(2), 1 => Just(8)],
        any::<bool>(),
        prop::collection::vec(hub, 1..=2),
    )
        .prop_map(
            |(pack, bob_project_role, fan_out, manifest, hubs)| LinkLeakCase {
                pack,
                bob_project_role,
                fan_out,
                manifest,
                hubs,
            },
        )
}

/// An active item of `kind` titled `title`, placed and classified as given,
/// or `None` when the case's pack keeps Alice from creating or reading it.
/// An item Alice may read must be active, so a seeding collision that
/// admission takes for a conflict fails the run instead of thinning it.
async fn placed(
    world: &World,
    key: &str,
    kind: LinkEndpoint,
    placement: Placement,
    sensitivity: Sensitivity,
    title: &str,
) -> Option<Item> {
    let (knowledge_type, metadata) = match kind {
        LinkEndpoint::Class(class) => ("entity", declares(class.as_str())),
        LinkEndpoint::Type(knowledge_type) => (knowledge_type.as_str(), json!({})),
    };
    let mut content = internal_content(title, metadata);
    content["sensitivity"] = json!(sensitivity.as_str());
    let placed_scope = placement.scope(world).await;
    let request = placed_scope.create_request(knowledge_type, content);
    let (status, created) = call(
        &world.app,
        "POST",
        "/v1/knowledge",
        &world.alice_token,
        Some(key),
        Some(request),
    )
    .await;
    if !is_applied(key, status, &created) {
        return None;
    }
    let id = created["knowledge_item_id"]
        .as_str()
        .expect("created item id")
        .to_owned();
    let read = format!("/v1/knowledge/{id}");
    let (status, item) = call(&world.app, "GET", &read, &world.alice_token, None, None).await;
    match status {
        StatusCode::OK => {}
        StatusCode::FORBIDDEN => return None,
        _ => panic!("{key}: Alice's read of her item answered {status}: {item}"),
    }
    assert_eq!(item["lifecycle_state"], "active", "{key} seeded as {item}");
    Some(Item {
        id,
        revision: created["revision_id"]
            .as_str()
            .expect("created revision id")
            .to_owned(),
    })
}

/// A link the case applied, oldest first within its hub.
struct SeededLink {
    id: String,
    /// The hub's position in `SeededCase::hubs`.
    hub: usize,
    hub_end: HubEnd,
    predicate: EntityPredicate,
    far: Item,
    far_mark: String,
    far_placement: Placement,
}

impl SeededLink {
    /// The end that asserted the link, which ends it.
    fn source<'c>(&'c self, case: &'c SeededCase) -> &'c Item {
        match self.hub_end {
            HubEnd::Source => &case.hubs[self.hub],
            HubEnd::Target => &self.far,
        }
    }
}

struct SeededCase {
    world: World,
    query: String,
    hubs: Vec<Item>,
    links: Vec<SeededLink>,
    /// Items or links the case's pack kept from being applied.
    kept_out: usize,
    /// Recalls made so far; audit records each once.
    recalls: Cell<usize>,
}

/// Seeds the case's hub `hub_index` and its links: every far end and link in
/// sampled order, so links a reader cannot see sit among those it can. Keys,
/// titles and marks follow the hub's place in the spec, so a hub the pack
/// keeps out lends none of them to the next.
async fn seed_hub(case: &mut SeededCase, case_index: usize, hub_index: usize, spec: &HubSpec) {
    let words = distinct_words(&format!("lfzh{case_index:02}h{hub_index}"));
    let title = format!("Hub {} {words}", case.query);
    let key = format!("lfz-{case_index}-hub{hub_index}");
    let world = &case.world;
    let (in_project, public) = (Placement::Project, Sensitivity::Public);
    let Some(hub) = placed(world, &key, spec.kind, in_project, public, &title).await else {
        case.kept_out += 1;
        return;
    };
    let hub_at = case.hubs.len();
    let rules = hub_rules(spec.kind);
    assert!(!rules.is_empty(), "no predicate admits {:?}", spec.kind);
    for (link_index, far_spec) in spec.links.iter().enumerate() {
        let rule = &rules[far_spec.rule % rules.len()];
        let far_kind = rule.far_kinds[far_spec.kind % rule.far_kinds.len()];
        let far_mark = format!("lfzm{case_index:02}h{hub_index}l{link_index}x");
        let far_key = format!("lfz-{case_index}-far{hub_index}-{link_index}");
        let far_title = format!("Far end {}", distinct_words(&far_mark));
        let (placement, sensitivity) = (far_spec.placement, far_spec.sensitivity);
        let Some(far) = placed(
            world,
            &far_key,
            far_kind,
            placement,
            sensitivity,
            &far_title,
        )
        .await
        else {
            case.kept_out += 1;
            continue;
        };
        let (source, target) = match rule.hub_end {
            HubEnd::Source => (&hub, &far),
            HubEnd::Target => (&far, &hub),
        };
        let key = format!("lfz-{case_index}-link{hub_index}-{link_index}");
        let (alice, predicate) = (&world.alice_token, rule.predicate.as_str());
        let (status, linked) = link(world, alice, &key, source, &target.id, predicate, None).await;
        if !is_applied(&key, status, &linked) {
            case.kept_out += 1;
            continue;
        }
        case.links.push(SeededLink {
            id: changed_link(&linked),
            hub: hub_at,
            hub_end: rule.hub_end,
            predicate: rule.predicate,
            far,
            far_mark,
            far_placement: placement,
        });
    }
    case.hubs.push(hub);
}

/// A fresh tenant bound to the case's pack, with Bob's extra role, every
/// class and predicate enabled, expansion following links, and the case's
/// hubs.
async fn seed_link_leak_case(case_index: usize, spec: &LinkLeakCase) -> SeededCase {
    let world = leak_world(spec.pack, spec.bob_project_role).await;
    let (classes, predicates) = (EntityClass::ALL.to_vec(), EntityPredicate::ALL.to_vec());
    enable_vocabulary(&world, classes, predicates).await;
    set_traversal(&world, true, 1, spec.fan_out).await;
    let mut case = SeededCase {
        world,
        query: format!("lfzq{case_index:02}"),
        hubs: Vec::new(),
        links: Vec::new(),
        kept_out: 0,
        recalls: Cell::new(0),
    };
    for (hub_index, hub) in spec.hubs.iter().enumerate() {
        seed_hub(&mut case, case_index, hub_index, hub).await;
    }
    case
}

/// What a reader may open of a case, by the single-item read: an oracle
/// independent of the bulk decisions the links read, recall and expansion
/// take.
struct Readable {
    hubs: Vec<bool>,
    fars: Vec<bool>,
}

impl Readable {
    /// Everything: `placed` kept only items Alice's own read returned.
    fn all(case: &SeededCase) -> Self {
        Self {
            hubs: vec![true; case.hubs.len()],
            fars: vec![true; case.links.len()],
        }
    }

    async fn decide(case: &SeededCase, reader: Reader<'_>) -> Self {
        let mut hubs = Vec::with_capacity(case.hubs.len());
        for hub in &case.hubs {
            hubs.push(readable_by(&case.world, reader.token, &hub.id).await);
        }
        let mut fars = Vec::with_capacity(case.links.len());
        for link in &case.links {
            fars.push(readable_by(&case.world, reader.token, &link.far.id).await);
        }
        Self { hubs, fars }
    }

    /// Whether the reader may see a link: both its ends are readable.
    fn sees(&self, case: &SeededCase, link: usize) -> bool {
        self.fars[link] && self.hubs[case.links[link].hub]
    }

    /// The links the reader may see on hub `h`, oldest first.
    fn visible(&self, case: &SeededCase, h: usize) -> Vec<usize> {
        (0..case.links.len())
            .filter(|link| case.links[*link].hub == h && self.sees(case, *link))
            .collect()
    }

    /// Hubs holding links the reader may and may not see, more of them than
    /// `fan_out`: where a walk that counted links before deciding them would
    /// cut or flag differently.
    fn mixed_past_fan_out(&self, case: &SeededCase, fan_out: usize) -> usize {
        (0..case.hubs.len())
            .filter(|h| {
                let on_hub = case.links.iter().filter(|link| link.hub == *h).count();
                let seen = self.visible(case, *h).len();
                on_hub > fan_out && 0 < seen && seen < on_hub
            })
            .count()
    }
}

/// One reader's view of a case: every hub's links read, recall over the hubs
/// with its audit, and an expanded run shaped with its detail and audit.
/// `text` holds the raw responses, before anything is stripped.
struct LinkView {
    links_read: Vec<Value>,
    recall: Value,
    run: Value,
    run_status: StatusCode,
    text: String,
}

/// Recall over the case's hubs as `reader`, shaped with what audit recorded
/// of this recall. Appends the raw response and audit to `text`.
async fn recall_view(case: &SeededCase, reader: Reader<'_>, text: &mut String) -> Value {
    let world = &case.world;
    let query = format!("/v1/sessions/{}/knowledge-query", reader.session);
    let request = json!({"query": case.query, "limit": 100});
    let (status, recalled) = call(
        &world.app,
        "POST",
        &query,
        reader.token,
        None,
        Some(request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{} recall: {recalled}", reader.name);
    // Each recall is audited once, so this one's event is the case's latest
    // and its count matches the recalls made.
    case.recalls.set(case.recalls.get() + 1);
    let mut recorded: Vec<Value> = audit_events(world)
        .await
        .into_iter()
        .filter(|event| event.payload["op"] == "context.knowledge_query")
        .map(|event| event.payload)
        .collect();
    assert_eq!(
        recorded.len(),
        case.recalls.get(),
        "{} recall audited once",
        reader.name
    );
    let mut audit = recorded.pop().expect("recall audit");
    text.push_str(&format!("{recalled}{audit}"));
    sort_determining(&mut audit);
    json!({"response": without(&recalled, &["as_of"]), "audit": audit})
}

async fn link_view(case: &SeededCase, reader: Reader<'_>, key: &str, manifest: bool) -> LinkView {
    let world = &case.world;
    let mut links_read = Vec::new();
    let mut text = String::new();
    for hub in &case.hubs {
        let lens = format!("/v1/knowledge/{}/links", hub.id);
        let (status, listed) = call(&world.app, "GET", &lens, reader.token, None, None).await;
        text.push_str(&listed.to_string());
        links_read.push(json!({"status": status.as_u16(), "links": listed["links"]}));
    }
    let recall = recall_view(case, reader, &mut text).await;
    let mut body = json!({"query": case.query});
    if manifest {
        body["disclosure"] = json!("manifest");
    }
    let (run_status, run) = run_with(world, reader.token, reader.session, key, body).await;
    text.push_str(&run.to_string());
    let run = if run_status == StatusCode::CREATED {
        let run_detail = detail(world, reader.token, run["id"].as_str().expect("run id")).await;
        let audit = run_audit(world, &run).await;
        text.push_str(&format!("{run_detail}{audit}"));
        run_shape(&run, &run_detail, &audit)
    } else {
        run
    };
    LinkView {
        links_read,
        recall,
        run,
        run_status,
        text,
    }
}

/// Each hub's links read and recall hit list exactly the links the reader
/// may see, and a hub it may not read is refused.
fn assert_links_and_recall(label: &str, case: &SeededCase, readable: &Readable, view: &LinkView) {
    for (h, hub) in case.hubs.iter().enumerate() {
        let read = &view.links_read[h];
        if !readable.hubs[h] {
            assert_ne!(
                read["status"], 200,
                "{label}: read an unreadable hub's links: {read}"
            );
            continue;
        }
        let mut expected: Vec<String> = readable
            .visible(case, h)
            .iter()
            .map(|link| case.links[*link].id.clone())
            .collect();
        expected.sort();
        let listed = read["links"].as_array().expect("links read");
        assert_eq!(
            link_ids(listed, "id"),
            expected,
            "{label}: links read of hub {h}"
        );
        let hit = view.recall["response"]["summaries"]
            .as_array()
            .expect("summaries")
            .iter()
            .find(|hit| hit["id"] == hub.id.as_str())
            .unwrap_or_else(|| panic!("{label}: no recall hit for hub {h}"));
        let shown = link_ids(&hit_links(hit), "link_id");
        assert_eq!(shown, expected, "{label}: recall links of hub {h}");
    }
}

/// Expansion reaches the first `fan_out` links the reader may see on each
/// hub, and flags fan-out only when more are visible. Answers whether it
/// was checked: not when the run was refused or another bound degraded it.
fn assert_expansion(
    label: &str,
    case: &SeededCase,
    readable: &Readable,
    view: &LinkView,
    fan_out: usize,
) -> bool {
    if view.run_status != StatusCode::CREATED {
        return false;
    }
    let degraded = view.run["run"]["degraded"].as_array().expect("degraded");
    let other_bound = degraded.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|value| value.starts_with("graph_") && value != "graph_fanout_truncated")
    });
    if other_bound {
        return false;
    }
    let mut cut = false;
    let mut reached = Vec::new();
    for h in 0..case.hubs.len() {
        let visible = readable.visible(case, h);
        cut |= visible.len() > fan_out;
        let first = visible.iter().take(fan_out);
        reached.extend(first.map(|link| case.links[*link].far.id.clone()));
    }
    let flagged = is_fanout_truncated(&view.run["run"]);
    assert_eq!(flagged, cut, "{label}: fan-out truncation: {degraded:?}");
    let mut found: Vec<String> = view.run["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .filter_map(|candidate| candidate["knowledge_item_id"].as_str())
        .filter(|id| case.links.iter().any(|link| link.far.id == *id))
        .map(str::to_owned)
        .collect();
    found.sort();
    reached.sort();
    assert_eq!(found, reached, "{label}: far ends expansion reached");
    true
}

/// Nothing in the reader's view names a link it may not see, or an end it
/// may not read.
fn assert_no_link_trace(label: &str, case: &SeededCase, readable: &Readable, view: &LinkView) {
    for (link_index, link) in case.links.iter().enumerate() {
        if readable.sees(case, link_index) {
            continue;
        }
        let mut traces = vec![link.id.as_str()];
        if !readable.fars[link_index] {
            traces.extend([
                link.far.id.as_str(),
                link.far.revision.as_str(),
                link.far_mark.as_str(),
            ]);
        }
        let hidden = format!("{label}: hidden link {link_index}");
        assert_no_traces(&hidden, &view.text, &traces);
    }
    for (h, hub) in case.hubs.iter().enumerate() {
        if !readable.hubs[h] {
            let hidden = format!("{label}: hidden hub {h}");
            assert_no_traces(&hidden, &view.text, &[&hub.id, &hub.revision]);
        }
    }
}

/// A response with `id` replaced, so two refusals compare by shape.
fn refusal((status, body): &(StatusCode, Value), id: &str) -> (StatusCode, String) {
    (*status, shape(body, id))
}

/// Bob linking his readable hub to the hidden far end of link `l` is refused
/// as linking it to an item that does not exist.
async fn assert_link_refused_like_absent(label: &str, case: &SeededCase, l: usize) {
    let (world, seeded) = (&case.world, &case.links[l]);
    let (bob, hub) = (&world.bob_token, &case.hubs[seeded.hub]);
    let predicate = seeded.predicate.as_str();
    let absent = KnowledgeItemId::new().to_string();
    let (key, absent_key) = (format!("lfz-probe-{l}"), format!("lfz-probe-{l}-absent"));
    let hidden = link(world, bob, &key, hub, &seeded.far.id, predicate, None).await;
    let missing = link(world, bob, &absent_key, hub, &absent, predicate, None).await;
    assert_eq!(
        refusal(&hidden, &seeded.far.id),
        refusal(&missing, &absent),
        "{label}: Bob linking to hidden far end {l}"
    );
}

/// Bob ending hidden link `l` through the end that asserted it is refused as
/// ending a link that does not exist there.
async fn assert_end_refused_like_absent(label: &str, case: &SeededCase, l: usize) {
    let (world, seeded) = (&case.world, &case.links[l]);
    let (bob, source) = (&world.bob_token, seeded.source(case));
    let forged = EntityLinkId::new().to_string();
    let (key, forged_key) = (
        format!("lfz-probe-end-{l}"),
        format!("lfz-probe-end-{l}-forged"),
    );
    let hidden = end(world, bob, &key, source, &seeded.id, None).await;
    let missing = end(world, bob, &forged_key, source, &forged, None).await;
    assert_eq!(
        refusal(&hidden, &seeded.id),
        refusal(&missing, &forged),
        "{label}: Bob ending hidden link {l}"
    );
}

/// The links read of an item Bob may not open refuses exactly as that
/// item's own read does.
async fn assert_links_read_refused_like_item(label: &str, world: &World, item_id: &str) {
    let item = format!("/v1/knowledge/{item_id}");
    let read = call(&world.app, "GET", &item, &world.bob_token, None, None).await;
    assert_ne!(read.0, StatusCode::OK, "{label}: item read: {}", read.1);
    let links = format!("{item}/links");
    let listed = call(&world.app, "GET", &links, &world.bob_token, None, None).await;
    assert_eq!(listed, read, "{label}: links read");
}

/// Every refusal Bob meets over what is hidden from him answers as its
/// absent counterpart does: the links read of each hub he may not read, and
/// each link hidden from him on a hub he may. Answers how many refusals were
/// compared.
async fn assert_refusals_hide_links(label: &str, case: &SeededCase, bob: &Readable) -> usize {
    let mut compared = 0;
    for (h, hub) in case.hubs.iter().enumerate() {
        if !bob.hubs[h] {
            let hidden = format!("{label}: hidden hub {h}");
            assert_links_read_refused_like_item(&hidden, &case.world, &hub.id).await;
            compared += 1;
        }
    }
    for (l, seeded) in case.links.iter().enumerate() {
        if bob.sees(case, l) || !bob.hubs[seeded.hub] {
            continue;
        }
        // On a hub Bob reads, a link he may not see has a far end he may not.
        assert_link_refused_like_absent(label, case, l).await;
        assert_end_refused_like_absent(label, case, l).await;
        let hidden = format!("{label}: hidden far end {l}");
        assert_links_read_refused_like_item(&hidden, &case.world, &seeded.far.id).await;
        compared += 3;
    }
    compared
}

/// Ends, as Alice, the links Bob may not see. Answers whether every end
/// applied; the case's pack may refuse one.
async fn end_hidden_links(case: &SeededCase, hidden: &[usize], case_index: usize) -> bool {
    let world = &case.world;
    for l in hidden {
        let link = &case.links[*l];
        let key = format!("lfz-{case_index}-end{l}");
        let alice = &world.alice_token;
        let (status, change) = end(world, alice, &key, link.source(case), &link.id, None).await;
        if !is_applied(&key, status, &change) {
            return false;
        }
    }
    true
}

/// What the fuzz compared, so its floor can require real coverage.
#[derive(Default)]
struct LinkLeakTally {
    links: usize,
    hidden_from_bob: usize,
    refusals_compared: usize,
    compared: usize,
    compared_with_visible: usize,
    mixed_past_fan_out: usize,
    expansion_checked: usize,
    alice_private_checked: usize,
    kept_out: usize,
    end_refused: usize,
}

/// One reader's view of the case, checked against the oracle and for traces
/// of what that reader may not see.
async fn checked_view(
    case: &SeededCase,
    readable: &Readable,
    reader: Reader<'_>,
    case_label: &str,
    key: &str,
    spec: &LinkLeakCase,
    tally: &mut LinkLeakTally,
) -> LinkView {
    let label = format!("{case_label} {}", reader.name);
    let view = link_view(case, reader, key, spec.manifest).await;
    assert_links_and_recall(&label, case, readable, &view);
    let fan_out = spec.fan_out as usize;
    let checked = assert_expansion(&label, case, readable, &view, fan_out);
    tally.expansion_checked += usize::from(checked);
    assert_no_link_trace(&label, case, readable, &view);
    view
}

async fn run_link_leak_case(case_index: usize, spec: &LinkLeakCase, tally: &mut LinkLeakTally) {
    let case = seed_link_leak_case(case_index, spec).await;
    let (alice, bob) = (Reader::alice(&case.world), Reader::bob(&case.world));
    let label = format!("case {case_index} {spec:?}");
    tally.links += case.links.len();
    tally.kept_out += case.kept_out;

    // Alice sees every link, so her views are the positive control for each
    // link hidden from Bob.
    let alice_reads = Readable::all(&case);
    let alice_key = format!("lfz-{case_index}-alice");
    checked_view(&case, &alice_reads, alice, &label, &alice_key, spec, tally).await;
    let bob_reads = Readable::decide(&case, bob).await;
    let far_ends = case.links.iter().map(|link| link.far_placement);
    let bob_far_reads = far_ends.zip(bob_reads.fars.iter().copied());
    tally.alice_private_checked += assert_bob_kept_from_alice_private(&label, bob_far_reads);
    let before_key = format!("lfz-{case_index}-bob-before");
    let before = checked_view(&case, &bob_reads, bob, &label, &before_key, spec, tally).await;
    let bob_label = format!("{label} bob");
    tally.refusals_compared += assert_refusals_hide_links(&bob_label, &case, &bob_reads).await;

    let hidden: Vec<usize> = (0..case.links.len())
        .filter(|link| !bob_reads.sees(&case, *link))
        .collect();
    tally.hidden_from_bob += hidden.len();
    tally.mixed_past_fan_out += bob_reads.mixed_past_fan_out(&case, spec.fan_out as usize);
    if hidden.is_empty() {
        return;
    }
    // Bob's views with every hidden link ended match his views with them
    // live: they changed nothing he sees, counts and audit included.
    if !end_hidden_links(&case, &hidden, case_index).await {
        tally.end_refused += 1;
        return;
    }
    let after_key = format!("lfz-{case_index}-bob-after");
    let after = link_view(&case, bob, &after_key, spec.manifest).await;
    assert_same_view(&bob_label, &before, &after);
    tally.compared += 1;
    tally.compared_with_visible += usize::from(hidden.len() < case.links.len());
}

fn assert_same_view(label: &str, before: &LinkView, after: &LinkView) {
    assert_eq!(
        after.links_read, before.links_read,
        "{label}: links read changed"
    );
    assert_eq!(after.recall, before.recall, "{label}: recall changed");
    assert_eq!(
        after.run_status, before.run_status,
        "{label}: run status changed: {}",
        after.run
    );
    assert_eq!(after.run, before.run, "{label}: expansion changed");
}

#[tokio::test]
async fn links_leave_no_trace_across_classes_predicates_roles_and_scopes() {
    let _guard = serial().await;
    if tenant_fixture::database_url("context tests").is_none() {
        return;
    }
    let mut tally = LinkLeakTally::default();
    for (case_index, spec) in sampled(link_leak_case(), LINK_LEAK_CASES)
        .into_iter()
        .enumerate()
    {
        run_link_leak_case(case_index, &spec, &mut tally).await;
    }
    let LinkLeakTally {
        links,
        hidden_from_bob,
        refusals_compared,
        compared,
        compared_with_visible,
        mixed_past_fan_out,
        expansion_checked,
        alice_private_checked,
        kept_out,
        end_refused,
    } = tally;
    let summary = format!(
        "{LINK_LEAK_CASES} cases, {links} links, {hidden_from_bob} hidden from Bob, \
         {refusals_compared} refusals compared, {compared} comparisons \
         ({compared_with_visible} beside visible links), {mixed_past_fan_out} hubs mixing \
         them past fan-out, {expansion_checked} expansion checks, {alice_private_checked} of \
         Alice's private far ends kept from Bob, {kept_out} items or links \
         the pack kept out, {end_refused} cases whose ends the pack refused"
    );
    // Without these, the fuzz could pass while comparing nothing: no hidden
    // link beside a visible one, no hub where counting before deciding would
    // show, no expansion checked, no refusal probed, or no private end of
    // Alice's held back from Bob.
    assert!(
        compared_with_visible > 0
            && mixed_past_fan_out > 0
            && expansion_checked > 0
            && refusals_compared > 0
            && alice_private_checked > 0,
        "link policy-leak fuzz covered too little: {summary}"
    );
    eprintln!("link policy-leak fuzz: {summary}");
}

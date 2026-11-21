//! Business links between Knowledge items (ADR-0164): governed admission,
//! refusals that reveal nothing, ending, bitemporal reads, erasure, review,
//! recall and graph expansion.
#[path = "support/context_world.rs"]
mod context_world;
use chrono::{DateTime, SecondsFormat, Utc};
use context_world::*;
use synveda_store::entity_links::MAX_ITEM_LINKS;
use synveda_types::configuration::GraphRetrievalConfiguration;
use synveda_types::knowledge::{EntityClass, EntityPredicate};
use synveda_types::{ContextGraphEdge, ContextGraphStep};

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

/// Selects two-hop expansion at the tenant root, following business links
/// when `links`, with `fan_out` edges of each kind per item.
async fn set_traversal(world: &World, links: bool, max_hops: u8, fan_out: u32) {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin traversal Configuration");
    let root = scopes::tenant_root(&mut *tx, world.tenant_id)
        .await
        .expect("read tenant root")
        .expect("tenant root");
    let graph = GraphRetrievalConfiguration {
        enabled: true,
        max_hops,
        fan_out_per_node: fan_out,
        max_expanded_candidates: 32,
        time_budget_ms: 5_000,
        token_budget: 512,
        include_entity_links: links,
    };
    configuration_support::set_graph_retrieval(&mut tx, world.tenant_id, root.id, graph).await;
    tx.commit().await.expect("commit traversal Configuration");
}

/// A context run and the reader's detail of it.
async fn traverse(
    world: &World,
    token: &str,
    session: &str,
    key: &str,
    query: &str,
) -> (Value, Value) {
    let run = context_run(world, token, session, key, query, None).await;
    let detail = detail(world, token, run["id"].as_str().expect("run id")).await;
    (run, detail)
}

fn candidate<'a>(detail: &'a Value, item: &Item) -> Option<&'a Value> {
    detail["candidates"]
        .as_array()
        .expect("candidates")
        .iter()
        .find(|candidate| candidate["knowledge_item_id"] == item.id.as_str())
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

/// Alice's context run under `key`, which replays a saved delivery when the
/// key was used before.
async fn post_run(world: &World, key: &str, query: &str) -> (StatusCode, Value) {
    call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/context-runs", world.alice_session),
        &world.alice_token,
        Some(key),
        Some(json!({"query": query})),
    )
    .await
}

#[tokio::test]
async fn an_ended_link_or_a_disabled_predicate_stops_expansion_and_its_replay() {
    let _guard = serial().await;
    let Some(world) = admitted_world().await else {
        return;
    };
    let classes = vec![EntityClass::Person, EntityClass::Service];
    let predicates = vec![EntityPredicate::Owns, EntityPredicate::Subject];
    enable_vocabulary(&world, classes.clone(), predicates).await;
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
    let dana = entity(&world, "inert-walk-dana", "Dana Ortiz", "schema:Person").await;
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

    enable_vocabulary(&world, classes, vec![EntityPredicate::Subject]).await;
    let (_, disabled) = traverse(&world, alice, session, "inert-walk-disabled", "Wrenfield").await;
    assert!(candidate(&disabled, &dana).is_none(), "{disabled}");
    assert!(candidate(&disabled, &service).is_some(), "{disabled}");
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

/// What a run shows its reader and what audit records of it, without the
/// run's own ids and instants.
fn run_shape(run: &Value, detail: &Value, audit: &Value) -> Value {
    let strip = |value: &Value, keys: &[&str]| {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            for key in keys {
                object.remove(*key);
            }
        }
        value
    };
    let each = |values: &Value, keys: &[&str]| {
        values
            .as_array()
            .expect("array")
            .iter()
            .map(|value| strip(value, keys))
            .collect::<Vec<_>>()
    };
    // The rendered block opens with the run's own instant, which its hash
    // covers.
    let mut run = strip(run, &["id", "as_of", "created_at", "block_hash"]);
    let rendered = run["rendered"].as_str().expect("rendered");
    let body = rendered
        .split_once('\n')
        .map_or("", |(_, rest)| rest)
        .to_owned();
    run["rendered"] = json!(body);
    let mut audit = strip(audit, &["session_id", "context_run_id"]);
    sort_determining(&mut audit);
    json!({
        "run": run,
        "candidates": each(&detail["candidates"], &["id"]),
        "selections": each(&detail["selections"], &["id", "context_candidate_id"]),
        "policy_exclusion_message": detail["policy_exclusion_message"],
        "audit": audit,
    })
}

async fn shaped_run(world: &World, key: &str, query: &str) -> Value {
    let (run, detail) = traverse(world, &world.bob_token, &world.bob_session, key, query).await;
    let audit = audit_events(world)
        .await
        .into_iter()
        .rfind(|event| event.action == "context.candidates.retrieved")
        .expect("the run's candidate audit");
    run_shape(&run, &detail, &audit.payload)
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

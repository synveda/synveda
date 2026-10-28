//! Clustered 1024-dimension Knowledge corpora and plan instruments for the
//! dense-leg plan evidence.
//!
//! Items are created through ordinary tenant transactions under the runtime
//! role; withdrawn session sources go through the public withdrawal route.
//! Vectors come from a seeded Gaussian mixture, so the same seed rebuilds
//! the same geometry and query set.
#![allow(dead_code)]

use super::*;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use synveda_store::knowledge::{self, NewKnowledgeItem, NewKnowledgeRevision, NewKnowledgeSource};
use synveda_store::knowledge_search::{self, Filters};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType,
    KnowledgeType,
};
use synveda_types::{
    KnowledgeItemId, KnowledgeRevisionId, KnowledgeSourceId, ProjectId, Sensitivity, SessionEventId,
};

pub(crate) const DIMENSION: usize = 1024;
pub(crate) const MODEL: &str = "dense-plan/clustered-1024";
pub(crate) const SEMANTIC_SQL: &str =
    include_str!("../../../synveda-store/queries/knowledge_semantic_1024.sql");
const TOPICS: usize = 48;
const ITEM_SPREAD: f32 = 0.35;
const QUERY_SPREAD: f32 = 0.30;
const BATCH: usize = 200;
const SEED_LANES: usize = 6;

/// SplitMix64: a reproducible stream with no dependency.
#[derive(Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn gaussian(&mut self) -> f32 {
        let (u, v) = (self.unit(), self.unit());
        ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()) as f32
    }
}

fn normalised(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

/// Topic centroids shared by every tenant, so tenants compete for the same
/// neighbourhoods of the one HNSW graph.
pub(crate) struct Geometry {
    centroids: Vec<Vec<f32>>,
}

impl Geometry {
    pub(crate) fn new(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let centroids = (0..TOPICS)
            .map(|_| normalised((0..DIMENSION).map(|_| rng.gaussian()).collect()))
            .collect();
        Self { centroids }
    }

    pub(crate) fn topics(&self) -> usize {
        self.centroids.len()
    }

    fn around(&self, topic: usize, spread: f32, rng: &mut Rng) -> Vec<f32> {
        let scale = spread / (DIMENSION as f32).sqrt();
        normalised(
            self.centroids[topic]
                .iter()
                .map(|value| value + scale * rng.gaussian())
                .collect(),
        )
    }

    pub(crate) fn item(&self, topic: usize, rng: &mut Rng) -> Vec<f32> {
        self.around(topic, ITEM_SPREAD, rng)
    }

    pub(crate) fn query(&self, topic: usize, rng: &mut Rng) -> Vec<f32> {
        self.around(topic, QUERY_SPREAD, rng)
    }
}

/// Where one item lives. Most Knowledge is project-specific; a smaller share
/// is workspace-wide, tenant-wide or personal.
#[derive(Clone, Copy)]
enum Placement {
    Project(usize),
    Workspace,
    Root,
    Personal,
}

fn placement(rng: &mut Rng, projects: usize) -> Placement {
    match rng.below(100) {
        0..=89 => Placement::Project(rng.below(projects)),
        90..=95 => Placement::Workspace,
        96..=97 => Placement::Root,
        _ => Placement::Personal,
    }
}

#[derive(Clone)]
pub(crate) struct ScopeMap {
    pub(crate) root: ScopeId,
    pub(crate) workspace: ScopeId,
    pub(crate) personal: ScopeId,
    pub(crate) projects: Vec<(ProjectId, ScopeId)>,
}

impl ScopeMap {
    fn resolve(&self, placement: Placement) -> (ScopeId, Option<ProjectId>) {
        match placement {
            Placement::Project(index) => (self.projects[index].1, Some(self.projects[index].0)),
            Placement::Workspace => (self.workspace, None),
            Placement::Root => (self.root, None),
            Placement::Personal => (self.personal, None),
        }
    }

    /// The exact admissible scopes a session in `project` hands the planner.
    pub(crate) fn chain(&self, project: usize) -> Vec<ScopeId> {
        vec![
            self.projects[project].1,
            self.workspace,
            self.root,
            self.personal,
        ]
    }
}

/// Counts of what the seeded tenant holds.
#[derive(Default, Clone, serde::Serialize)]
pub(crate) struct CorpusStats {
    pub(crate) items: usize,
    pub(crate) revisions: usize,
    pub(crate) revised_items: usize,
    pub(crate) archived: usize,
    pub(crate) stale: usize,
    pub(crate) session_sourced: usize,
    pub(crate) withdrawn_sources: usize,
    pub(crate) projects: usize,
}

/// Facts recorded while seeding, independent of the SQL under test.
#[derive(Default)]
pub(crate) struct Ledger {
    pub(crate) scope_of: HashMap<KnowledgeItemId, ScopeId>,
    pub(crate) inactive: std::collections::HashSet<KnowledgeItemId>,
    pub(crate) withdrawn: std::collections::HashSet<KnowledgeItemId>,
    /// Created after `as_known_mid`.
    pub(crate) late: std::collections::HashSet<KnowledgeItemId>,
}

/// What seeding counted and recorded as it went.
#[derive(Default)]
struct Tracking {
    stats: CorpusStats,
    ledger: Ledger,
}

pub(crate) struct DenseCorpus {
    pub(crate) scopes: ScopeMap,
    pub(crate) stats: CorpusStats,
    pub(crate) ledger: Ledger,
    /// Database clock after the first revisions committed and before any
    /// second revision, lifecycle change or withdrawal.
    pub(crate) as_known_mid: DateTime<Utc>,
}

/// The fixture's own bearers last five minutes; seeding a large corpus takes
/// longer, so each public call carries a freshly issued one.
fn fresh_token(world: &World) -> String {
    Hs256Verifier::new(SECRET).issue(ALICE, world.tenant_id, Duration::from_secs(600))
}

pub(crate) async fn create_projects(world: &World, count: usize) -> Vec<(ProjectId, ScopeId)> {
    let mut projects = Vec::with_capacity(count);
    for index in 0..count {
        let (status, project) = call(
            &world.app,
            "POST",
            &format!("/v1/workspaces/{}/projects", world.workspace_id),
            &fresh_token(world),
            Some(&format!("dense-project-{index}")),
            Some(
                json!({"slug": format!("dense-{index}"), "display_name": format!("Dense {index}")}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{project}");
        let id: ProjectId = project["id"]
            .as_str()
            .expect("project id")
            .parse()
            .expect("parse project");
        let scope: ScopeId = project["scope_id"]
            .as_str()
            .expect("scope")
            .parse()
            .expect("parse scope");
        projects.push((id, scope));
    }
    projects
}

pub(crate) async fn scope_map(world: &World, projects: usize) -> ScopeMap {
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin root lookup");
    let root = scopes::ensure_tenant_root(&mut tx, world.tenant_id)
        .await
        .expect("tenant root");
    tx.commit().await.expect("commit root lookup");
    let mut projects_list = vec![(
        world.project_id.parse().expect("fixture project"),
        world.project_scope,
    )];
    projects_list.extend(create_projects(world, projects.saturating_sub(1)).await);
    ScopeMap {
        root: root.id,
        workspace: world.workspace_scope,
        personal: world.alice_scope,
        projects: projects_list,
    }
}

fn revision_content(title: String, valid_from: DateTime<Utc>) -> KnowledgeRevisionContent {
    KnowledgeRevisionContent {
        summary: title.clone(),
        body_markdown: format!("Synthetic dense-plan fixture text for {title}."),
        title,
        tags: vec!["dense-plan".to_owned()],
        sensitivity: Sensitivity::Internal,
        confidence_permille: 900,
        valid_from,
        valid_to: None,
        stale_after: None,
        verification_metadata: json!({"method": "fixture"}),
        metadata: json!({}),
    }
}

fn new_revision(title: String, valid_from: DateTime<Utc>) -> NewKnowledgeRevision {
    NewKnowledgeRevision {
        id: KnowledgeRevisionId::new(),
        content: revision_content(title, valid_from),
        created_by: Some(ALICE.to_owned()),
    }
}

fn manual_source(tenant_id: TenantId, scope_id: ScopeId) -> NewKnowledgeSource {
    NewKnowledgeSource {
        id: KnowledgeSourceId::new(),
        tenant_id,
        scope_id,
        source_type: KnowledgeSourceType::Manual,
        session_event_id: None,
        import_artifact_id: None,
        locator: None,
        source_revision: None,
        content_hash: None,
        metadata: json!({"method": "dense_plan_fixture"}),
        created_by: Some(ALICE.to_owned()),
    }
}

/// Words that name each topic, so lexical search and the stub embedder agree
/// on what a query is about.
pub(crate) const TOPIC_WORDS: [&str; 24] = [
    "release",
    "checklist",
    "rollback",
    "correlation",
    "header",
    "migration",
    "review",
    "convention",
    "retry",
    "budget",
    "latency",
    "schema",
    "deploy",
    "incident",
    "runbook",
    "cache",
    "queue",
    "policy",
    "audit",
    "session",
    "context",
    "fixture",
    "gateway",
    "worker",
];

/// The two topic words for one of the geometry's topics.
pub(crate) fn topic_words(topic: usize) -> (&'static str, &'static str) {
    (
        TOPIC_WORDS[topic % TOPIC_WORDS.len()],
        TOPIC_WORDS[(topic / TOPIC_WORDS.len() + topic * 7 + 3) % TOPIC_WORDS.len()],
    )
}

fn topic_title(topic: usize, ordinal: usize) -> String {
    let (first, second) = topic_words(topic);
    format!("{first} {second} note {ordinal}")
}

const TYPES: [KnowledgeType; 4] = [
    KnowledgeType::Fact,
    KnowledgeType::Convention,
    KnowledgeType::Decision,
    KnowledgeType::Procedure,
];

/// One item to create, with its first vector.
#[derive(Clone)]
pub(crate) struct Planned {
    pub(crate) item: KnowledgeItemId,
    pub(crate) revision: KnowledgeRevisionId,
    pub(crate) scope: ScopeId,
    pub(crate) project: Option<ProjectId>,
    pub(crate) topic: usize,
    pub(crate) ordinal: usize,
}

fn plan_items(scopes: &ScopeMap, topics: usize, count: usize, rng: &mut Rng) -> Vec<Planned> {
    (0..count)
        .map(|ordinal| {
            let (scope, project) = scopes.resolve(placement(rng, scopes.projects.len()));
            Planned {
                item: KnowledgeItemId::new(),
                revision: KnowledgeRevisionId::new(),
                scope,
                project,
                topic: rng.below(topics),
                ordinal,
            }
        })
        .collect()
}

pub(crate) async fn manual_sources(
    world: &World,
    scopes: &ScopeMap,
) -> HashMap<ScopeId, KnowledgeSourceId> {
    let mut all = vec![scopes.root, scopes.workspace, scopes.personal];
    all.extend(scopes.projects.iter().map(|(_, scope)| *scope));
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin source fixture");
    let mut sources = HashMap::new();
    for scope in all {
        let source = manual_source(world.tenant_id, scope);
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create manual source");
        sources.insert(scope, source.id);
    }
    tx.commit().await.expect("commit source fixture");
    sources
}

pub(crate) use std::collections::HashMap;

async fn create_batch(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    batch: &[Planned],
    sources: &HashMap<ScopeId, KnowledgeSourceId>,
    vectors: &[Vec<f32>],
    valid_from: DateTime<Utc>,
) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin item batch");
    for (planned, vector) in batch.iter().zip(vectors) {
        let item = NewKnowledgeItem {
            id: planned.item,
            tenant_id: tenant,
            scope_id: planned.scope,
            project_id: planned.project,
            owner_principal_id: None,
            knowledge_type: TYPES[planned.ordinal % TYPES.len()],
            origin: KnowledgeOrigin::Authored,
            created_by: Some(ALICE.to_owned()),
        };
        let mut revision = new_revision(topic_title(planned.topic, planned.ordinal), valid_from);
        revision.id = planned.revision;
        knowledge::create_item(&mut tx, &item, &revision, &[sources[&planned.scope]])
            .await
            .expect("create dense item");
        knowledge_search::insert_embedding(&mut tx, tenant, planned.revision, MODEL, vector)
            .await
            .expect("embed dense item");
    }
    tx.commit().await.expect("commit item batch");
}

/// Creates `planned` items over parallel ordinary-role connections.
async fn create_items(
    world: &World,
    planned: &[Planned],
    sources: &HashMap<ScopeId, KnowledgeSourceId>,
    geometry: &Geometry,
    seed: u64,
    valid_from: DateTime<Utc>,
) {
    let pool = PgPoolOptions::new()
        .max_connections(SEED_LANES as u32)
        .connect(&world.database_url)
        .await
        .expect("connect seeding pool");
    let mut rng = Rng::new(seed);
    let vectors: Vec<Vec<f32>> = planned
        .iter()
        .map(|p| geometry.item(p.topic, &mut rng))
        .collect();
    let chunks: Vec<(Vec<Planned>, Vec<Vec<f32>>)> = planned
        .chunks(BATCH)
        .zip(vectors.chunks(BATCH))
        .map(|(items, vectors)| (items.to_vec(), vectors.to_vec()))
        .collect();
    let queue = Arc::new(tokio::sync::Mutex::new(chunks));
    let mut lanes = Vec::new();
    for _ in 0..SEED_LANES {
        let (pool, queue, sources) = (pool.clone(), Arc::clone(&queue), sources.clone());
        let tenant = world.tenant_id;
        lanes.push(tokio::spawn(async move {
            loop {
                let next = queue.lock().await.pop();
                let Some((items, vectors)) = next else { break };
                create_batch(&pool, tenant, &items, &sources, &vectors, valid_from).await;
            }
        }));
    }
    for lane in lanes {
        lane.await.expect("seeding lane joined");
    }
    pool.close().await;
}

pub(crate) async fn database_now(pool: &sqlx::PgPool) -> DateTime<Utc> {
    sqlx::query_scalar("select clock_timestamp()")
        .fetch_one(pool)
        .await
        .expect("read database clock")
}

/// Revises a share of items (each revision keeps its predecessor's vector in
/// the sidecar and the HNSW graph) and moves another share out of `active`.
async fn evolve(
    world: &World,
    planned: &[Planned],
    sources: &HashMap<ScopeId, KnowledgeSourceId>,
    geometry: &Geometry,
    rng: &mut Rng,
    tracking: &mut Tracking,
    valid_from: DateTime<Utc>,
) {
    let Tracking { stats, ledger } = tracking;
    for chunk in planned.chunks(BATCH) {
        let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
            .await
            .expect("begin evolution batch");
        for planned in chunk {
            let roll = rng.below(100);
            let mut head = planned.revision;
            for round in 0..usize::from(roll < 40) + usize::from(roll < 10) {
                let revision = new_revision(
                    format!("Dense item {} r{}", planned.ordinal, round + 2),
                    valid_from,
                );
                let next = revision.id;
                knowledge::append_revision(
                    &mut tx,
                    world.tenant_id,
                    planned.item,
                    head,
                    &revision,
                    &[sources[&planned.scope]],
                )
                .await
                .expect("revise dense item")
                .expect("dense item exists");
                let vector = geometry.item(planned.topic, rng);
                knowledge_search::insert_embedding(&mut tx, world.tenant_id, next, MODEL, &vector)
                    .await
                    .expect("embed revision");
                head = next;
                stats.revisions += 1;
            }
            stats.revised_items += usize::from(roll < 40);
            let lifecycle = match rng.below(100) {
                0..=4 => Some(KnowledgeLifecycleState::Archived),
                5..=9 => Some(KnowledgeLifecycleState::Stale),
                _ => None,
            };
            if let Some(lifecycle) = lifecycle {
                knowledge::set_lifecycle(
                    &mut tx,
                    world.tenant_id,
                    planned.item,
                    head,
                    lifecycle,
                    Some(ALICE),
                )
                .await
                .expect("move dense lifecycle")
                .expect("dense item exists");
                match lifecycle {
                    KnowledgeLifecycleState::Archived => stats.archived += 1,
                    _ => stats.stale += 1,
                }
                ledger.inactive.insert(planned.item);
            }
        }
        tx.commit().await.expect("commit evolution batch");
    }
}

const SOURCE_BATCH: usize = 50;

/// Appends real session events through the public API and returns each
/// event's id and payload hash.
async fn append_source_events(
    world: &World,
    count: usize,
    tag: &str,
) -> Vec<(SessionEventId, String)> {
    let mut appended = Vec::with_capacity(count);
    for start in (0..count).step_by(SOURCE_BATCH) {
        let events: Vec<Value> = (start..(start + SOURCE_BATCH).min(count))
            .map(|index| {
                json!({"client_event_id": format!("{tag}-{index}"), "event_type": "message.user",
                       "occurred_at": "2026-10-20T00:00:00Z",
                       "payload": {"text": format!("Fictional dense-plan source note {index}.")}})
            })
            .collect();
        let (status, body) = call(
            &world.app,
            "POST",
            &format!("/v1/sessions/{}/events", world.alice_session),
            &fresh_token(world),
            Some(&format!("{tag}-batch-{start}")),
            Some(json!({"events": events})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for event in body["events"].as_array().expect("appended events") {
            let event = &event["event"];
            appended.push((
                event["id"]
                    .as_str()
                    .expect("event id")
                    .parse()
                    .expect("parse event id"),
                event["payload_hash"]
                    .as_str()
                    .expect("payload hash")
                    .to_owned(),
            ));
        }
    }
    appended
}

async fn withdraw(world: &World, event: SessionEventId, payload_hash: &str, key: &str) {
    let (status, removed) = call(
        &world.app,
        "POST",
        &format!("/v1/sessions/{}/events/{event}/withdraw", world.alice_session),
        &fresh_token(world),
        Some(key),
        Some(json!({"expected_payload_hash": payload_hash, "reason": "Retire this fictional source"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{removed}");
}

/// Items whose only provenance is a session event, half of which are then
/// withdrawn, so the restrictive source-eligibility policies hide them.
async fn session_sourced(
    world: &World,
    planned: &[Planned],
    geometry: &Geometry,
    rng: &mut Rng,
    tracking: &mut Tracking,
    valid_from: DateTime<Utc>,
) {
    let Tracking { stats, ledger } = tracking;
    let events = append_source_events(world, planned.len(), "dense-source").await;
    let mut tx = rls::begin_tenant_tx(&world.state.pool, world.tenant_id)
        .await
        .expect("begin session-sourced items");
    for (planned, (event, _)) in planned.iter().zip(&events) {
        let mut source = manual_source(world.tenant_id, world.project_scope);
        source.source_type = KnowledgeSourceType::SessionEvent;
        source.session_event_id = Some(*event);
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create session source");
        let item = NewKnowledgeItem {
            id: planned.item,
            tenant_id: world.tenant_id,
            scope_id: planned.scope,
            project_id: planned.project,
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Fact,
            origin: KnowledgeOrigin::Observed,
            created_by: Some(ALICE.to_owned()),
        };
        let mut revision = new_revision(format!("Sourced item {}", planned.ordinal), valid_from);
        revision.id = planned.revision;
        knowledge::create_item(&mut tx, &item, &revision, &[source.id])
            .await
            .expect("create sourced item");
        let vector = geometry.item(planned.topic, rng);
        knowledge_search::insert_embedding(
            &mut tx,
            world.tenant_id,
            planned.revision,
            MODEL,
            &vector,
        )
        .await
        .expect("embed sourced item");
    }
    tx.commit().await.expect("commit session-sourced items");
    stats.session_sourced = planned.len();
    for (index, (event, hash)) in events.iter().enumerate().step_by(2) {
        withdraw(world, *event, hash, &format!("dense-withdraw-{index}")).await;
        stats.withdrawn_sources += 1;
        ledger.withdrawn.insert(planned[index].item);
    }
}

/// What to build in one tenant.
pub(crate) struct Spec {
    pub(crate) items: usize,
    pub(crate) projects: usize,
    pub(crate) sourced: usize,
    pub(crate) seed: u64,
}

pub(crate) async fn seed(world: &World, spec: &Spec, geometry: &Geometry) -> DenseCorpus {
    let scopes = scope_map(world, spec.projects).await;
    let sources = manual_sources(world, &scopes).await;
    let mut rng = Rng::new(spec.seed);
    let planned = plan_items(&scopes, geometry.topics(), spec.items, &mut rng);
    let valid_from = Utc::now() - chrono::Duration::days(30);
    create_items(
        world,
        &planned,
        &sources,
        geometry,
        spec.seed ^ 0x5EED,
        valid_from,
    )
    .await;
    let as_known_mid = database_now(&world.state.pool).await;
    let mut tracking = Tracking {
        stats: CorpusStats {
            items: spec.items + spec.sourced,
            revisions: spec.items + spec.sourced,
            projects: scopes.projects.len(),
            ..CorpusStats::default()
        },
        ledger: Ledger::default(),
    };
    evolve(
        world,
        &planned,
        &sources,
        geometry,
        &mut rng,
        &mut tracking,
        valid_from,
    )
    .await;
    let sourced = plan_items(&scopes, geometry.topics(), spec.sourced, &mut rng);
    session_sourced(
        world,
        &sourced,
        geometry,
        &mut rng,
        &mut tracking,
        valid_from,
    )
    .await;
    let Tracking { stats, mut ledger } = tracking;
    for item in planned.iter().chain(&sourced) {
        ledger.scope_of.insert(item.item, item.scope);
    }
    ledger.late.extend(sourced.iter().map(|item| item.item));
    DenseCorpus {
        scopes,
        stats,
        ledger,
        as_known_mid,
    }
}

/// Refreshes planner statistics as the table owner, outside any tenant.
pub(crate) async fn analyze(world: &World) {
    let migrator = tenant_fixture::migrator_pool(&world.state.pool).await;
    for table in [
        "knowledge_items",
        "knowledge_items_history",
        "knowledge_revisions",
        "knowledge_revision_embeddings",
        "knowledge_revision_sources",
        "knowledge_sources",
        "knowledge_relations",
        "scope_closure",
        "workspaces",
        "source_dependencies",
        "source_suppressions",
    ] {
        sqlx::query(&format!("vacuum (analyze) public.{table}"))
            .execute(&migrator)
            .await
            .unwrap_or_else(|error| panic!("vacuum analyze {table}: {error}"));
    }
    migrator.close().await;
}

/// One retrieval request shape the product actually issues.
#[derive(Clone)]
pub(crate) struct Shape {
    pub(crate) name: &'static str,
    pub(crate) filters: Filters,
    pub(crate) limit: i64,
}

pub(crate) fn base_filters(
    scope_ids: Vec<ScopeId>,
    lifecycle: Option<KnowledgeLifecycleState>,
    at: DateTime<Utc>,
) -> Filters {
    Filters {
        scope_ids,
        workspace_id: None,
        project_id: None,
        scope_id: None,
        owner_principal_id: None,
        knowledge_type: None,
        origin: None,
        lifecycle,
        tag: None,
        source_type: None,
        updated_from: None,
        updated_before: None,
        stale: None,
        at,
        as_known_at: None,
        include_history: false,
        include_transitional: false,
    }
}

/// The context planner's exact chain, one project's scope alone, the public
/// Knowledge search depth and a transaction-time history read.
pub(crate) fn shapes(corpus: &DenseCorpus, at: DateTime<Utc>) -> Vec<Shape> {
    let active = Some(KnowledgeLifecycleState::Active);
    let mut history = base_filters(corpus.scopes.chain(1), None, at);
    history.as_known_at = Some(corpus.as_known_mid);
    history.include_history = true;
    vec![
        Shape {
            name: "planner_chain",
            filters: base_filters(corpus.scopes.chain(1), active, at),
            limit: 96,
        },
        Shape {
            name: "one_project",
            filters: base_filters(vec![corpus.scopes.projects[1].1], active, at),
            limit: 96,
        },
        Shape {
            name: "search_broad",
            filters: base_filters(Vec::new(), None, at),
            limit: 1_000,
        },
        Shape {
            name: "history_chain",
            filters: history,
            limit: 96,
        },
    ]
}

/// Planner and pgvector settings applied to one tenant transaction.
#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct Arm {
    pub(crate) name: &'static str,
    pub(crate) plan_cache_mode: &'static str,
    pub(crate) iterative_scan: &'static str,
    pub(crate) ef_search: u32,
    pub(crate) max_scan_tuples: u32,
    pub(crate) exact: bool,
}

/// pgvector's own defaults, which the shipped dense leg runs at.
pub(crate) const DEFAULT_EF_SEARCH: u32 = 40;
pub(crate) const DEFAULT_MAX_SCAN_TUPLES: u32 = 20_000;

pub(crate) const fn arm(
    name: &'static str,
    plan_cache_mode: &'static str,
    iterative_scan: &'static str,
    ef_search: u32,
    max_scan_tuples: u32,
) -> Arm {
    Arm {
        name,
        plan_cache_mode,
        iterative_scan,
        ef_search,
        max_scan_tuples,
        exact: false,
    }
}

pub(crate) const EXACT: Arm = Arm {
    name: "exact_truth",
    plan_cache_mode: "force_custom_plan",
    iterative_scan: "off",
    ef_search: DEFAULT_EF_SEARCH,
    max_scan_tuples: DEFAULT_MAX_SCAN_TUPLES,
    exact: true,
};

pub(crate) async fn apply_arm(conn: &mut PgConnection, arm: &Arm) {
    sqlx::query(
        "select set_config('plan_cache_mode', $1, true), set_config('hnsw.iterative_scan', $2, true), \
                set_config('hnsw.ef_search', $3, true), set_config('hnsw.max_scan_tuples', $4, true), \
                set_config('enable_indexscan', $5, true)",
    )
    .bind(arm.plan_cache_mode)
    .bind(arm.iterative_scan)
    .bind(arm.ef_search.to_string())
    .bind(arm.max_scan_tuples.to_string())
    .bind(if arm.exact { "off" } else { "on" })
    .execute(&mut *conn)
    .await
    .expect("apply arm settings");
}

/// A pool of exactly one connection, so every checkout is the same backend.
pub(crate) async fn one_connection(world: &World) -> sqlx::PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&world.database_url)
        .await
        .expect("connect single-connection pool")
}

pub(crate) async fn run_dense(
    pool: &sqlx::PgPool,
    tenant: TenantId,
    shape: &Shape,
    vector: &[f32],
    arm: Option<&Arm>,
) -> (Vec<KnowledgeItemId>, Duration) {
    let mut tx = rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin dense read");
    if let Some(arm) = arm {
        apply_arm(&mut tx, arm).await;
    }
    let started = Instant::now();
    let rows = knowledge_search::semantic_candidates(
        &mut tx,
        tenant,
        &shape.filters,
        MODEL,
        vector,
        shape.limit,
    )
    .await
    .expect("dense candidates");
    let elapsed = started.elapsed();
    tx.commit().await.expect("commit dense read");
    (rows.into_iter().map(|row| row.item_id).collect(), elapsed)
}

pub(crate) fn recall(truth: &[KnowledgeItemId], got: &[KnowledgeItemId], k: usize) -> f64 {
    let expected: std::collections::HashSet<_> = truth.iter().take(k).collect();
    if expected.is_empty() {
        return if got.is_empty() { 1.0 } else { 0.0 };
    }
    let hits = got
        .iter()
        .take(k)
        .filter(|id| expected.contains(id))
        .count();
    hits as f64 / expected.len() as f64
}

pub(crate) fn quantiles(mut values: Vec<f64>) -> Value {
    if values.is_empty() {
        return json!({"n": 0});
    }
    values.sort_by(f64::total_cmp);
    let at = |quantile: f64| {
        let index = ((values.len() as f64) * quantile).ceil() as usize;
        values[index.saturating_sub(1).min(values.len() - 1)]
    };
    json!({"n": values.len(), "p50": at(0.50), "p95": at(0.95), "p99": at(0.99), "max": values[values.len() - 1]})
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn minimum(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::INFINITY, f64::min)
}

const PARAMETER_TYPES: &str = "uuid, text, real[], uuid, uuid, uuid, text, text, text, text, text, \
    text, timestamptz, timestamptz, bool, timestamptz, int8, uuid[], timestamptz, bool, bool";

fn literal_uuid(value: Option<uuid::Uuid>) -> String {
    value.map_or_else(
        || "null::uuid".to_owned(),
        |value| format!("'{value}'::uuid"),
    )
}

fn literal_text(value: Option<&str>) -> String {
    value.map_or_else(
        || "null::text".to_owned(),
        |value| format!("'{}'", value.replace('\'', "''")),
    )
}

fn literal_time(value: Option<DateTime<Utc>>) -> String {
    value.map_or_else(
        || "null::timestamptz".to_owned(),
        |value| {
            format!(
                "'{}'::timestamptz",
                value.to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
            )
        },
    )
}

fn literal_bool(value: Option<bool>) -> String {
    value.map_or_else(|| "null::bool".to_owned(), |value| value.to_string())
}

/// The product statement's 21 arguments as SQL literals, in query order.
fn execute_arguments(tenant: TenantId, shape: &Shape, vector: &[f32]) -> String {
    let filters = &shape.filters;
    let vector = vector
        .iter()
        .map(f32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let scopes = filters
        .scope_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    [
        literal_uuid(Some(tenant.as_uuid())),
        literal_text(Some(MODEL)),
        format!("'{{{vector}}}'::real[]"),
        literal_uuid(filters.workspace_id.map(|id| id.as_uuid())),
        literal_uuid(filters.project_id.map(|id| id.as_uuid())),
        literal_uuid(filters.scope_id.map(|id| id.as_uuid())),
        literal_text(filters.owner_principal_id.as_deref()),
        literal_text(filters.knowledge_type.map(|value| value.as_str())),
        literal_text(filters.origin.map(|value| value.as_str())),
        literal_text(filters.lifecycle.map(|value| value.as_str())),
        literal_text(filters.tag.as_deref()),
        literal_text(filters.source_type.map(|value| value.as_str())),
        literal_time(filters.updated_from),
        literal_time(filters.updated_before),
        literal_bool(filters.stale),
        literal_time(Some(filters.at)),
        shape.limit.max(1).to_string(),
        format!("'{{{scopes}}}'::uuid[]"),
        literal_time(filters.as_known_at),
        filters.include_history.to_string(),
        filters.include_transitional.to_string(),
    ]
    .join(", ")
}

/// Content-free facts read from one `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`.
#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct PlanFacts {
    pub(crate) plan_class: String,
    pub(crate) root: String,
    pub(crate) nodes: Vec<String>,
    pub(crate) indexes: Vec<String>,
    pub(crate) uses_hnsw: bool,
    pub(crate) planning_ms: f64,
    pub(crate) execution_ms: f64,
    pub(crate) jit_ms: f64,
    pub(crate) rows: f64,
    pub(crate) rows_removed_by_filter: f64,
    pub(crate) shared_hit_blocks: f64,
    pub(crate) shared_read_blocks: f64,
}

fn plan_facts(explained: &Value) -> PlanFacts {
    let top = &explained[0];
    let plan = &top["Plan"];
    let mut facts = PlanFacts {
        root: plan["Node Type"].as_str().unwrap_or_default().to_owned(),
        planning_ms: top["Planning Time"].as_f64().unwrap_or_default(),
        execution_ms: top["Execution Time"].as_f64().unwrap_or_default(),
        jit_ms: top["JIT"]["Timing"]["Total"].as_f64().unwrap_or_default(),
        rows: plan["Actual Rows"].as_f64().unwrap_or_default(),
        shared_hit_blocks: plan["Shared Hit Blocks"].as_f64().unwrap_or_default(),
        shared_read_blocks: plan["Shared Read Blocks"].as_f64().unwrap_or_default(),
        ..PlanFacts::default()
    };
    let mut stack = vec![plan];
    let mut visited = 0_usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        assert!(visited < 10_000, "EXPLAIN tree exceeded its bound");
        let kind = node["Node Type"].as_str().unwrap_or_default().to_owned();
        if !facts.nodes.contains(&kind) {
            facts.nodes.push(kind);
        }
        if let Some(index) = node["Index Name"].as_str()
            && !facts.indexes.iter().any(|known| known == index)
        {
            facts.indexes.push(index.to_owned());
        }
        facts.rows_removed_by_filter += node["Rows Removed by Filter"].as_f64().unwrap_or_default()
            * node["Actual Loops"].as_f64().unwrap_or(1.0);
        stack.extend(node["Plans"].as_array().into_iter().flatten());
    }
    facts.nodes.sort();
    facts.indexes.sort();
    facts.uses_hnsw = facts.indexes.iter().any(|index| index.contains("hnsw"));
    facts
}

/// Plan-cache counters for one named prepared statement.
async fn plan_counts(conn: &mut PgConnection, name: &str) -> (i64, i64) {
    sqlx::query_as("select generic_plans, custom_plans from pg_prepared_statements where name = $1")
        .bind(name)
        .fetch_one(&mut *conn)
        .await
        .expect("read plan-cache counters")
}

/// Prepares the exact product SQL on this session under `name`.
pub(crate) async fn prepare_probe(conn: &mut PgConnection, name: &str) {
    sqlx::raw_sql(&format!(
        "prepare {name}({PARAMETER_TYPES}) as {SEMANTIC_SQL}"
    ))
    .execute(&mut *conn)
    .await
    .expect("prepare the product dense statement");
}

/// Explains one execution of a prepared probe and names the plan class the
/// plan cache chose for it.
pub(crate) async fn explain_execution(
    conn: &mut PgConnection,
    name: &str,
    tenant: TenantId,
    shape: &Shape,
    vector: &[f32],
) -> PlanFacts {
    let before = plan_counts(conn, name).await;
    let sql = format!(
        "explain (analyze, buffers, format json) execute {name}({})",
        execute_arguments(tenant, shape, vector)
    );
    let (explained,): (Value,) = sqlx::query_as(&sql)
        .persistent(false)
        .fetch_one(&mut *conn)
        .await
        .expect("explain the prepared dense statement");
    let after = plan_counts(conn, name).await;
    let mut facts = plan_facts(&explained);
    facts.plan_class = match (after.0 - before.0, after.1 - before.1) {
        (1, 0) => "generic",
        (0, 1) => "custom",
        _ => "unknown",
    }
    .to_owned();
    facts
}

/// Plan-cache counters for the product's own protocol-level statement.
pub(crate) async fn product_plan_counts(conn: &mut PgConnection) -> (i64, i64) {
    sqlx::query_as(
        "select coalesce(sum(generic_plans), 0)::int8, coalesce(sum(custom_plans), 0)::int8 \
         from pg_prepared_statements where not from_sql \
         and statement like '%knowledge_item_versions%' and statement not like '%pg_prepared_statements%'",
    )
    .persistent(false)
    .fetch_one(&mut *conn)
    .await
    .expect("read product plan-cache counters")
}

/// Exact ordered results for every query, from a plan that cannot use HNSW.
pub(crate) async fn ground_truth(
    world: &World,
    shape: &Shape,
    queries: &[Vec<f32>],
) -> (Vec<Vec<KnowledgeItemId>>, Vec<f64>) {
    let pool = one_connection(world).await;
    let mut truth = Vec::with_capacity(queries.len());
    let mut latency = Vec::with_capacity(queries.len());
    for query in queries {
        let (ids, elapsed) = run_dense(&pool, world.tenant_id, shape, query, Some(&EXACT)).await;
        truth.push(ids);
        latency.push(elapsed.as_secs_f64() * 1_000.0);
    }
    pool.close().await;
    (truth, latency)
}

/// Recall, latency, ordering stability and one explained plan for an arm.
pub(crate) async fn measure_arm(
    world: &World,
    shape: &Shape,
    arm: &Arm,
    queries: &[Vec<f32>],
    truth: &[Vec<KnowledgeItemId>],
    repetitions: usize,
) -> Value {
    let pool = one_connection(world).await;
    let mut first = Vec::with_capacity(queries.len());
    for query in queries {
        first.push(
            run_dense(&pool, world.tenant_id, shape, query, Some(arm))
                .await
                .0,
        );
    }
    let mut latency = Vec::new();
    let mut stable = true;
    for _ in 0..repetitions {
        for (query, expected) in queries.iter().zip(&first) {
            let (ids, elapsed) = run_dense(&pool, world.tenant_id, shape, query, Some(arm)).await;
            stable &= &ids == expected;
            latency.push(elapsed.as_secs_f64() * 1_000.0);
        }
    }
    let k10: Vec<f64> = truth
        .iter()
        .zip(&first)
        .map(|(t, g)| recall(t, g, 10))
        .collect();
    let k_limit: Vec<f64> = truth
        .iter()
        .zip(&first)
        .map(|(t, g)| recall(t, g, shape.limit as usize))
        .collect();
    let returned: Vec<f64> = first.iter().map(|ids| ids.len() as f64).collect();
    let expected: Vec<f64> = truth.iter().map(|ids| ids.len() as f64).collect();
    let mut tx = rls::begin_tenant_tx(&pool, world.tenant_id)
        .await
        .expect("begin explain");
    apply_arm(&mut tx, arm).await;
    prepare_probe(&mut tx, "dense_arm_probe").await;
    let plan = explain_execution(
        &mut tx,
        "dense_arm_probe",
        world.tenant_id,
        shape,
        &queries[0],
    )
    .await;
    tx.rollback().await.expect("end explain");
    pool.close().await;
    json!({
        "arm": arm,
        "recall_at_10": {"mean": mean(&k10), "min": minimum(&k10)},
        "recall_at_limit": {"mean": mean(&k_limit), "min": minimum(&k_limit)},
        "returned": {"mean": mean(&returned), "min": minimum(&returned)},
        "exact_returned": {"mean": mean(&expected), "min": minimum(&expected)},
        "latency_ms": quantiles(latency),
        "stable_ordering": stable,
        "plan": plan,
    })
}

/// Executions one through six of the unmodified product statement, and of
/// an identical SQL-level prepared probe, on one connection each.
pub(crate) async fn switch_study(world: &World, shape: &Shape, query: &[f32]) -> Value {
    let pool = one_connection(world).await;
    let mut executions = Vec::new();
    let mut previous = (0, 0);
    let mut first: Option<Vec<KnowledgeItemId>> = None;
    for execution in 1..=6 {
        let (ids, elapsed) = run_dense(&pool, world.tenant_id, shape, query, None).await;
        let mut conn = pool.acquire().await.expect("reacquire the same connection");
        let counts = product_plan_counts(&mut conn).await;
        drop(conn);
        let class = match (counts.0 - previous.0, counts.1 - previous.1) {
            (1, 0) => "generic",
            (0, 1) => "custom",
            _ => "unknown",
        };
        previous = counts;
        let same = first.get_or_insert_with(|| ids.clone()) == &ids;
        executions.push(json!({"execution": execution, "plan_class": class,
            "latency_ms": elapsed.as_secs_f64() * 1_000.0, "returned": ids.len(), "same_as_first": same}));
    }
    let mut tx = rls::begin_tenant_tx(&pool, world.tenant_id)
        .await
        .expect("begin probe");
    prepare_probe(&mut tx, "dense_switch_probe").await;
    let mut probe = Vec::new();
    for _ in 1..=6 {
        probe.push(
            explain_execution(&mut tx, "dense_switch_probe", world.tenant_id, shape, query).await,
        );
    }
    tx.rollback().await.expect("end probe");
    pool.close().await;
    json!({"product": executions, "probe": probe})
}

/// Server facts that bound what a measurement may claim.
pub(crate) async fn environment(world: &World) -> Value {
    let pool = &world.state.pool;
    let mut settings = serde_json::Map::new();
    for name in [
        "server_version",
        "shared_buffers",
        "work_mem",
        "effective_cache_size",
        "random_page_cost",
        "jit",
        "jit_above_cost",
        "max_parallel_workers_per_gather",
        "plan_cache_mode",
        "max_connections",
    ] {
        let value: String = sqlx::query_scalar("select current_setting($1)")
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("read setting");
        settings.insert(name.to_owned(), json!(value));
    }
    let vector: String =
        sqlx::query_scalar("select extversion from pg_extension where extname = 'vector'")
            .fetch_one(pool)
            .await
            .expect("read pgvector version");
    let index: (Option<Vec<String>>, i64) = sqlx::query_as(
        "select reloptions, pg_relation_size(oid) from pg_class where relname = 'knowledge_revision_embeddings_hnsw_1024'",
    )
    .fetch_one(pool)
    .await
    .expect("read HNSW index facts");
    let rows: i64 = sqlx::query_scalar(
        "select reltuples::int8 from pg_class where relname = 'knowledge_revision_embeddings'",
    )
    .fetch_one(pool)
    .await
    .expect("estimate indexed vectors across tenants");
    json!({"settings": settings, "pgvector": vector,
           "hnsw_1024": {"reloptions": index.0, "bytes": index.1, "analyzed_rows_all_tenants": rows}})
}

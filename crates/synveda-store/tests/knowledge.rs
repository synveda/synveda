//! CPR-15 acceptance evidence: stable Knowledge aggregates, immutable
//! revisions, normalised provenance, bitemporal current-state correctness and
//! forced-RLS tenant isolation (ADR-0080).
//!
//! These tests need Postgres and skip when `DATABASE_URL` is absent. The
//! database-backed gate runs them through `make db-test`.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::knowledge::{
    self, NewKnowledgeItem, NewKnowledgeRelation, NewKnowledgeRevision, NewKnowledgeSource,
};
use synveda_store::sessions::{NewSession, NewSessionEvent};
use synveda_store::workspaces::NewWorkspace;
use synveda_store::{rls, scopes, sessions, tenants, workspaces};
use synveda_types::knowledge::{
    KnowledgeLifecycleState, KnowledgeOrigin, KnowledgeRelationType, KnowledgeRevisionContent,
    KnowledgeSourceType, KnowledgeType,
};
use synveda_types::session::SessionEventType;
use synveda_types::{
    Error, KnowledgeItemId, KnowledgeRelationId, KnowledgeRevisionId, KnowledgeSourceId, ScopeId,
    Sensitivity, SessionId, TenantId, TenantStatus, WorkspaceId,
};

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = match std::env::var("DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "skipping Knowledge tests: DATABASE_URL is not set \
                     (run `make dev-up` then `make db-test`)"
                );
                return None;
            }
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let pool = rt.block_on(async {
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .expect("connect to DATABASE_URL");
            synveda_store::migrate(&pool)
                .await
                .expect("apply migrations");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

async fn tick() {
    tokio::time::sleep(Duration::from_millis(5)).await;
}

async fn new_tenant(pool: &PgPool) -> (TenantId, ScopeId) {
    let tenant_id = TenantId::new();
    let slug = format!("knowledge-{}", tenant_id.as_uuid().simple());
    tenants::create(
        pool,
        tenant_id,
        &slug,
        "Knowledge acceptance fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = pool.begin().await.expect("begin root transaction");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("ensure tenant root");
    tx.commit().await.expect("commit tenant root");
    (tenant_id, root.id)
}

fn manual_source(tenant_id: TenantId, scope_id: ScopeId) -> NewKnowledgeSource {
    NewKnowledgeSource {
        id: KnowledgeSourceId::new(),
        tenant_id,
        scope_id,
        source_type: KnowledgeSourceType::Manual,
        session_event_id: None,
        locator: None,
        source_revision: None,
        content_hash: None,
        metadata: serde_json::json!({"method": "acceptance_test"}),
        created_by: Some("test:knowledge".to_owned()),
    }
}

fn item(tenant_id: TenantId, scope_id: ScopeId, knowledge_type: KnowledgeType) -> NewKnowledgeItem {
    NewKnowledgeItem {
        id: KnowledgeItemId::new(),
        tenant_id,
        scope_id,
        project_id: None,
        owner_principal_id: None,
        knowledge_type,
        origin: KnowledgeOrigin::Authored,
        created_by: Some("test:knowledge".to_owned()),
    }
}

fn revision(title: &str, body: &str) -> NewKnowledgeRevision {
    NewKnowledgeRevision {
        id: KnowledgeRevisionId::new(),
        content: KnowledgeRevisionContent {
            title: title.to_owned(),
            body_markdown: body.to_owned(),
            summary: title.to_owned(),
            tags: vec!["http".to_owned(), "observability".to_owned()],
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: Utc::now(),
            valid_to: None,
            stale_after: None,
            verification_metadata: serde_json::json!({"method": "reviewed"}),
            metadata: serde_json::json!({}),
        },
        created_by: Some("test:knowledge".to_owned()),
    }
}

async fn create_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    new: &NewKnowledgeItem,
    new_revision: &NewKnowledgeRevision,
    source_id: KnowledgeSourceId,
) -> knowledge::KnowledgeSnapshot {
    knowledge::create_item(&mut *tx, new, new_revision, &[source_id])
        .await
        .expect("create Knowledge item")
}

#[test]
fn revisions_are_immutable_and_current_projection_is_bitemporal() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant_id, root_scope_id) = new_tenant(&db.pool).await;
        let source = manual_source(tenant_id, root_scope_id);
        let new_item = item(tenant_id, root_scope_id, KnowledgeType::Convention);
        let first_revision = revision("Request correlation", "Use `X-Request-Id`.");

        let mut tx = db.pool.begin().await.expect("begin create");
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create source");
        let first = create_item(&mut tx, &new_item, &first_revision, source.id).await;
        tx.commit().await.expect("commit first revision");

        tick().await;
        let second_revision = revision("Trace propagation", "Use `traceparent`.");
        let mut tx = db.pool.begin().await.expect("begin revision");
        let second = knowledge::append_revision(
            &mut tx,
            tenant_id,
            new_item.id,
            first_revision.id,
            &second_revision,
            &[source.id],
        )
        .await
        .expect("append revision")
        .expect("item exists");
        tx.commit().await.expect("commit second revision");

        let current = knowledge::current(&db.pool, tenant_id, new_item.id)
            .await
            .expect("read current")
            .expect("current item");
        assert_eq!(current, second);
        assert_eq!(current.revision.id, second_revision.id);
        assert_eq!(current.revision.revision_number, 2);
        assert_eq!(current.revision.content.body_markdown, "Use `traceparent`.");

        let history = knowledge::as_known_at(
            &db.pool,
            tenant_id,
            new_item.id,
            first.item.transaction_from,
        )
        .await
        .expect("read transaction history")
        .expect("first head state");
        assert_eq!(history.revision.id, first_revision.id);
        assert_eq!(
            history.revision.content.body_markdown,
            "Use `X-Request-Id`."
        );
        assert!(history.transaction_to.is_some());

        let revisions = knowledge::revisions(&db.pool, tenant_id, new_item.id)
            .await
            .expect("list revisions");
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].id, first_revision.id);
        assert_eq!(revisions[1].id, second_revision.id);
        assert_ne!(revisions[0].content_hash, revisions[1].content_hash);

        let stale = knowledge::append_revision(
            &mut db.pool.begin().await.expect("begin stale append"),
            tenant_id,
            new_item.id,
            first_revision.id,
            &revision("Lost update", "This must not be stored."),
            &[source.id],
        )
        .await
        .expect_err("stale precondition is rejected");
        assert!(matches!(stale, Error::Conflict { .. }));

        let mutation = sqlx::query!(
            "update knowledge_revisions set title = 'mutated' where tenant_id = $1 and id = $2",
            tenant_id.as_uuid(),
            first_revision.id.as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(
            mutation.is_err(),
            "revision rows are append-only for every role"
        );
    });
}

#[test]
fn all_source_shapes_are_real_and_disclosed_by_their_own_scope() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant_id, root_scope_id) = new_tenant(&db.pool).await;
        let mut tx = db.pool.begin().await.expect("begin fixture");
        let workspace = workspaces::create(
            &mut tx,
            &NewWorkspace {
                id: WorkspaceId::new(),
                tenant_id,
                slug: "source-workspace".to_owned(),
                display_name: "Source workspace".to_owned(),
                description: None,
                created_by: None,
            },
        )
        .await
        .expect("create workspace");
        let session = sessions::create(
            &mut tx,
            &NewSession {
                id: SessionId::new(),
                tenant_id,
                workspace_id: workspace.id,
                project_id: None,
                principal_id: "principal:alice".to_owned(),
                client_name: "knowledge-acceptance".to_owned(),
                client_version: Some("1.0.0".to_owned()),
                client_installation_id: None,
                external_session_id: None,
                agent_name: Some("fixture".to_owned()),
                model_name: None,
                repository_id: None,
                branch: None,
                task_summary: Some("Exercise provenance".to_owned()),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("create session");
        let event = sessions::append_events(
            &mut tx,
            tenant_id,
            session.id,
            &[NewSessionEvent {
                event_type: SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: "knowledge-source-event".to_owned(),
                occurred_at: Utc::now(),
                payload: serde_json::json!({"text": "Use provider event IDs"}),
                redactions: None,
                quarantine: false,
            }],
        )
        .await
        .expect("append source event")
        .remove(0)
        .event;

        let shapes = [
            NewKnowledgeSource {
                id: KnowledgeSourceId::new(),
                tenant_id,
                scope_id: workspace.scope_id,
                source_type: KnowledgeSourceType::SessionEvent,
                session_event_id: Some(event.id),
                locator: None,
                source_revision: None,
                content_hash: Some(event.payload_hash.clone()),
                metadata: serde_json::json!({}),
                created_by: Some("test:knowledge".to_owned()),
            },
            manual_source(tenant_id, root_scope_id),
            located_source(
                tenant_id,
                root_scope_id,
                KnowledgeSourceType::Document,
                "docs/runbook.md",
            ),
            located_source(
                tenant_id,
                workspace.scope_id,
                KnowledgeSourceType::Repository,
                "src/http.rs",
            ),
            located_source(
                tenant_id,
                workspace.scope_id,
                KnowledgeSourceType::Url,
                "https://example.test/design",
            ),
            located_source(
                tenant_id,
                workspace.scope_id,
                KnowledgeSourceType::Okf,
                "knowledge/request-ids.md",
            ),
            located_source(
                tenant_id,
                workspace.scope_id,
                KnowledgeSourceType::SystemDerived,
                "extractor@1:event-set",
            ),
        ];
        for source in &shapes {
            knowledge::create_source(&mut tx, source)
                .await
                .unwrap_or_else(|error| panic!("create {} source: {error}", source.source_type));
        }
        let new_item = item(tenant_id, root_scope_id, KnowledgeType::Fact);
        let new_revision = revision("Provider idempotency", "Deduplicate by provider event ID.");
        let source_ids: Vec<_> = shapes.iter().map(|source| source.id).collect();
        knowledge::create_item(&mut tx, &new_item, &new_revision, &source_ids)
            .await
            .expect("create item with all source families");
        tx.commit().await.expect("commit source fixture");

        let root_visible =
            knowledge::visible_sources(&db.pool, tenant_id, new_revision.id, &[root_scope_id])
                .await
                .expect("root-visible sources");
        assert_eq!(
            root_visible
                .iter()
                .map(|source| source.source_type)
                .collect::<Vec<_>>(),
            [KnowledgeSourceType::Manual, KnowledgeSourceType::Document]
        );
        let workspace_visible =
            knowledge::visible_sources(&db.pool, tenant_id, new_revision.id, &[workspace.scope_id])
                .await
                .expect("workspace-visible sources");
        assert_eq!(workspace_visible.len(), 5);
        assert_eq!(workspace_visible[0].session_event_id, Some(event.id));
        assert_eq!(
            knowledge::visible_sources(
                &db.pool,
                tenant_id,
                new_revision.id,
                &[root_scope_id, workspace.scope_id],
            )
            .await
            .expect("all visible sources")
            .len(),
            KnowledgeSourceType::ALL.len()
        );

        let mut tx = db.pool.begin().await.expect("begin scope-confusion check");
        let confused = NewKnowledgeSource {
            id: KnowledgeSourceId::new(),
            tenant_id,
            scope_id: root_scope_id,
            source_type: KnowledgeSourceType::SessionEvent,
            session_event_id: Some(event.id),
            locator: None,
            source_revision: None,
            content_hash: Some(event.payload_hash),
            metadata: serde_json::json!({}),
            created_by: None,
        };
        let error = knowledge::create_source(&mut tx, &confused)
            .await
            .expect_err("an event cannot be relabelled into a broader scope");
        assert!(matches!(error, Error::Invalid { .. }));
    });
}

fn located_source(
    tenant_id: TenantId,
    scope_id: ScopeId,
    source_type: KnowledgeSourceType,
    locator: &str,
) -> NewKnowledgeSource {
    NewKnowledgeSource {
        id: KnowledgeSourceId::new(),
        tenant_id,
        scope_id,
        source_type,
        session_event_id: None,
        locator: Some(locator.to_owned()),
        source_revision: Some("fixture-revision".to_owned()),
        content_hash: Some("a".repeat(64)),
        metadata: serde_json::json!({"extension": {"kept": true}}),
        created_by: Some("test:knowledge".to_owned()),
    }
}

#[test]
fn relation_vocabulary_is_append_only() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant_id, root_scope_id) = new_tenant(&db.pool).await;
        let source = manual_source(tenant_id, root_scope_id);
        let source_item = item(tenant_id, root_scope_id, KnowledgeType::Decision);
        let target_item = item(tenant_id, root_scope_id, KnowledgeType::Convention);
        let source_revision = revision("New convention", "Use `traceparent`.");
        let target_revision = revision("Old convention", "Use `X-Request-Id`.");
        let mut tx = db.pool.begin().await.expect("begin relation fixture");
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create relation source");
        create_item(&mut tx, &source_item, &source_revision, source.id).await;
        create_item(&mut tx, &target_item, &target_revision, source.id).await;
        let mut relation_ids = Vec::new();
        for relation_type in KnowledgeRelationType::ALL {
            let new = NewKnowledgeRelation {
                id: KnowledgeRelationId::new(),
                tenant_id,
                source_item_id: source_item.id,
                target_item_id: target_item.id,
                asserting_revision_id: source_revision.id,
                relation_type: *relation_type,
                metadata: serde_json::json!({}),
                created_by: Some("test:knowledge".to_owned()),
            };
            knowledge::add_relation(&mut tx, &new)
                .await
                .unwrap_or_else(|error| panic!("create {relation_type}: {error}"));
            relation_ids.push(new.id);
        }
        tx.commit().await.expect("commit relations");

        let relations = knowledge::relations(&db.pool, tenant_id, source_item.id)
            .await
            .expect("list relations");
        assert_eq!(relations.len(), KnowledgeRelationType::ALL.len());
        let actual: HashSet<_> = relations
            .iter()
            .map(|relation| relation.relation_type)
            .collect();
        let expected: HashSet<_> = KnowledgeRelationType::ALL.iter().copied().collect();
        assert_eq!(actual, expected);

        let mutation = sqlx::query!(
            "update knowledge_relations set relation_type = 'supports' where tenant_id = $1 and id = $2",
            tenant_id.as_uuid(),
            relation_ids[1].as_uuid(),
        )
        .execute(&db.pool)
        .await;
        assert!(mutation.is_err(), "relation claims are append-only");
    });
}

#[test]
fn forced_rls_hides_every_knowledge_relation_from_another_tenant() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (hidden_tenant, root_scope_id) = new_tenant(&db.pool).await;
        let source = manual_source(hidden_tenant, root_scope_id);
        let first_item = item(hidden_tenant, root_scope_id, KnowledgeType::Fact);
        let second_item = item(hidden_tenant, root_scope_id, KnowledgeType::Fact);
        let first_revision = revision("First", "First revision.");
        let second_revision = revision("Second", "Second item.");
        let mut tx = db.pool.begin().await.expect("begin hidden fixture");
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create hidden source");
        create_item(&mut tx, &first_item, &first_revision, source.id).await;
        create_item(&mut tx, &second_item, &second_revision, source.id).await;
        let next_revision = revision("First updated", "Second revision.");
        knowledge::append_revision(
            &mut tx,
            hidden_tenant,
            first_item.id,
            first_revision.id,
            &next_revision,
            &[source.id],
        )
        .await
        .expect("append hidden revision")
        .expect("hidden item exists");
        knowledge::add_relation(
            &mut tx,
            &NewKnowledgeRelation {
                id: KnowledgeRelationId::new(),
                tenant_id: hidden_tenant,
                source_item_id: first_item.id,
                target_item_id: second_item.id,
                asserting_revision_id: next_revision.id,
                relation_type: KnowledgeRelationType::RelatedTo,
                metadata: serde_json::json!({}),
                created_by: None,
            },
        )
        .await
        .expect("create hidden relation");
        tx.commit().await.expect("commit hidden fixture");

        let (other_tenant, _) = new_tenant(&db.pool).await;
        let mut tx = rls::begin_tenant_tx(&db.pool, other_tenant)
            .await
            .expect("begin other-tenant transaction");
        sqlx::raw_sql("set local role synveda_app")
            .execute(&mut *tx)
            .await
            .expect("demote to application role");
        let counts = sqlx::query!(
            r#"
            select
                (select count(*) from knowledge_items where tenant_id = $1) as "items!",
                (select count(*) from knowledge_items_history where tenant_id = $1) as "history!",
                (select count(*) from knowledge_revisions where tenant_id = $1) as "revisions!",
                (select count(*) from knowledge_sources where tenant_id = $1) as "sources!",
                (select count(*) from knowledge_revision_sources where tenant_id = $1) as "links!",
                (select count(*) from knowledge_relations where tenant_id = $1) as "relations!",
                (select count(*) from knowledge_item_versions where tenant_id = $1) as "versions_view!",
                (select count(*) from knowledge_current where tenant_id = $1) as "current_view!"
            "#,
            hidden_tenant.as_uuid(),
        )
        .fetch_one(&mut *tx)
        .await
        .expect("query hidden Knowledge relations");
        assert_eq!(
            [
                counts.items,
                counts.history,
                counts.revisions,
                counts.sources,
                counts.links,
                counts.relations,
                counts.versions_view,
                counts.current_view,
            ],
            [0; 8]
        );
    });
}

#[test]
fn lifecycle_changes_preserve_content_and_head_history() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant_id, root_scope_id) = new_tenant(&db.pool).await;
        let source = manual_source(tenant_id, root_scope_id);
        let new_item = item(tenant_id, root_scope_id, KnowledgeType::Warning);
        let new_revision = revision("Temporary warning", "Do not deploy during maintenance.");
        let mut tx = db.pool.begin().await.expect("begin lifecycle fixture");
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create lifecycle source");
        create_item(&mut tx, &new_item, &new_revision, source.id).await;
        tx.commit().await.expect("commit lifecycle fixture");

        tick().await;
        let mut tx = db.pool.begin().await.expect("begin lifecycle update");
        let archived = knowledge::set_lifecycle(
            &mut tx,
            tenant_id,
            new_item.id,
            new_revision.id,
            KnowledgeLifecycleState::Archived,
            Some("test:knowledge"),
        )
        .await
        .expect("archive lifecycle")
        .expect("item exists");
        tx.commit().await.expect("commit lifecycle update");
        assert_eq!(
            archived.item.lifecycle_state,
            KnowledgeLifecycleState::Archived
        );
        assert_eq!(archived.revision.id, new_revision.id);
        assert_eq!(
            knowledge::revisions(&db.pool, tenant_id, new_item.id)
                .await
                .expect("revisions after lifecycle update")
                .len(),
            1,
            "a lifecycle transition does not invent a content revision"
        );
    });
}

#[test]
fn sealed_export_projection_contains_complete_knowledge_history_and_provenance() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let (tenant_id, root_scope_id) = new_tenant(&db.pool).await;
        let source = manual_source(tenant_id, root_scope_id);
        let former = item(tenant_id, root_scope_id, KnowledgeType::Convention);
        let current = item(tenant_id, root_scope_id, KnowledgeType::Convention);
        let former_revision = revision("Request id", "Use `X-Request-Id`.");
        let current_revision = revision("Trace context", "Use `traceparent`.");
        let mut tx = db.pool.begin().await.expect("begin export fixture");
        knowledge::create_source(&mut tx, &source)
            .await
            .expect("create export source");
        create_item(&mut tx, &former, &former_revision, source.id).await;
        create_item(&mut tx, &current, &current_revision, source.id).await;
        let amended = revision(
            "Trace context everywhere",
            "Use `traceparent` on public APIs.",
        );
        knowledge::append_revision(
            &mut tx,
            tenant_id,
            current.id,
            current_revision.id,
            &amended,
            &[source.id],
        )
        .await
        .expect("append export revision")
        .expect("current item");
        knowledge::add_relation(
            &mut tx,
            &NewKnowledgeRelation {
                id: KnowledgeRelationId::new(),
                tenant_id,
                source_item_id: current.id,
                target_item_id: former.id,
                asserting_revision_id: amended.id,
                relation_type: KnowledgeRelationType::Supersedes,
                metadata: serde_json::json!({}),
                created_by: Some("test:knowledge".to_owned()),
            },
        )
        .await
        .expect("create export relation");
        tx.commit().await.expect("commit export fixture");

        let mut tx = rls::begin_tenant_tx(&db.pool, tenant_id)
            .await
            .expect("begin export snapshot");
        let exported = knowledge::export_tenant(&mut tx, tenant_id)
            .await
            .expect("export Knowledge snapshot");
        tx.commit().await.expect("commit export snapshot");

        assert_eq!(exported.item_count, 2);
        assert_eq!(exported.revision_count, 3);
        assert_eq!(exported.source_count, 1);
        assert_eq!(exported.relation_count, 1);
        assert_eq!(exported.head_history.as_array().map(Vec::len), Some(3));
        assert_eq!(exported.revisions.as_array().map(Vec::len), Some(3));
        assert_eq!(exported.sources.as_array().map(Vec::len), Some(1));
        assert_eq!(exported.revision_sources.as_array().map(Vec::len), Some(3));
        assert_eq!(exported.relations.as_array().map(Vec::len), Some(1));
        let encoded = serde_json::to_string(&exported.revisions).expect("encode revisions");
        assert!(encoded.contains("X-Request-Id"));
        assert!(encoded.contains("traceparent"));
    });
}

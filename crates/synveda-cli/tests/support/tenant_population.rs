//! A populated tenant shared by the archive and erasure acceptance tests.
#![allow(dead_code)]

use sqlx::PgPool;
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::*;

/// What [`seed`] created.
pub struct Seeded {
    /// The tenant root scope.
    pub root: ScopeId,
    /// The first and the current Knowledge revision.
    pub revisions: [KnowledgeRevisionId; 2],
}

/// Admits a populated tenant through the ordinary runtime role: a tenant root,
/// workspace, project, principal scope and identity, one Session with three
/// events, one Knowledge item with two revisions, a source and an embedding,
/// a decision linked to a service by an ended business link, a context run
/// whose retained path follows that link, and one policy pack. Returns the
/// root scope and the first item's two revision ids.
pub async fn seed(pool: &PgPool, tenant: TenantId) -> Seeded {
    let mut tx = synveda_store::rls::begin_tenant_write_tx(pool, tenant)
        .await
        .unwrap();
    let root = synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .unwrap();
    let workspace = synveda_store::workspaces::create(
        &mut tx,
        &synveda_store::workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "archive".to_owned(),
            display_name: "Archive".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .unwrap();
    let project = synveda_store::projects::create(
        &mut tx,
        &synveda_store::projects::NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            slug: "round-trip".to_owned(),
            display_name: "Round trip".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .unwrap();
    let principal =
        synveda_store::scopes::ensure_principal_scope(&mut tx, tenant, "archivist", "Archivist")
            .await
            .unwrap();
    synveda_store::identities::create(
        &mut tx,
        IdentityId::new(),
        tenant,
        Some("archivist"),
        IdentityKind::User,
        None,
        None,
        principal.id,
    )
    .await
    .unwrap();
    let session = seed_session(&mut tx, tenant, workspace.id).await;
    let revisions = seed_knowledge(&mut tx, tenant, &session, project.scope_id, project.id).await;
    synveda_store::policy_packs::apply(
        &mut *tx,
        tenant,
        "archive-pack",
        "permit (principal, action == Synveda::Action::\"ScopeRead\", resource) when { resource in principal.tenant };",
        &PackConfig::default(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Seeded {
        root: root.id,
        revisions,
    }
}

async fn seed_session(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    workspace: WorkspaceId,
) -> synveda_types::session::Session {
    let session = synveda_store::sessions::create(
        tx,
        &synveda_store::sessions::NewSession {
            id: SessionId::new(),
            tenant_id: tenant,
            workspace_id: workspace,
            project_id: None,
            principal_id: "archivist".to_owned(),
            client_name: "claude-code".to_owned(),
            client_version: None,
            client_installation_id: None,
            external_session_id: None,
            agent_name: None,
            model_name: None,
            repository_id: None,
            branch: None,
            task_summary: None,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .unwrap();
    for index in 0..3 {
        let payload =
            serde_json::json!({"schema_version": 1, "text": format!("fictional note {index}")});
        synveda_store::sessions::append_events(
            tx,
            tenant,
            session.id,
            &[synveda_store::sessions::NewSessionEvent {
                event_type: synveda_types::session::SessionEventType::MessageUser,
                event_schema_version: 1,
                client_event_id: format!("note-{index}"),
                occurred_at: chrono::Utc::now(),
                source_payload_hash: synveda_store::sessions::payload_hash(&payload),
                payload,
                redactions: None,
                quarantine: false,
            }],
        )
        .await
        .unwrap();
    }
    session
}

async fn seed_knowledge(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    session: &synveda_types::session::Session,
    scope: ScopeId,
    project: ProjectId,
) -> [KnowledgeRevisionId; 2] {
    let content = |title: &str| KnowledgeRevisionContent {
        title: title.to_owned(),
        body_markdown: format!("{title}: retry the fictional import five times."),
        summary: title.to_owned(),
        tags: vec!["archive".to_owned()],
        sensitivity: Sensitivity::Internal,
        confidence_permille: 900,
        valid_from: chrono::Utc::now(),
        valid_to: None,
        stale_after: None,
        verification_metadata: serde_json::json!({"method": "fixture"}),
        metadata: serde_json::json!({}),
    };
    let source = synveda_store::knowledge::create_source(
        tx,
        &synveda_store::knowledge::NewKnowledgeSource {
            id: KnowledgeSourceId::new(),
            tenant_id: tenant,
            scope_id: scope,
            source_type: KnowledgeSourceType::Document,
            session_event_id: None,
            import_artifact_id: None,
            locator: Some("fictional:archive-procedure".to_owned()),
            source_revision: Some("fixture-v1".to_owned()),
            content_hash: None,
            metadata: serde_json::json!({}),
            created_by: Some("archivist".to_owned()),
        },
    )
    .await
    .unwrap();
    let first = synveda_store::knowledge::NewKnowledgeRevision {
        id: KnowledgeRevisionId::new(),
        content: content("Archived retry procedure"),
        created_by: Some("archivist".to_owned()),
    };
    let item = synveda_store::knowledge::create_item(
        tx,
        &synveda_store::knowledge::NewKnowledgeItem {
            id: KnowledgeItemId::new(),
            tenant_id: tenant,
            scope_id: scope,
            project_id: Some(project),
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Procedure,
            origin: KnowledgeOrigin::Authored,
            created_by: Some("archivist".to_owned()),
        },
        &first,
        &[source.id],
    )
    .await
    .unwrap();
    let second = synveda_store::knowledge::NewKnowledgeRevision {
        id: KnowledgeRevisionId::new(),
        content: content("Archived retry procedure, revised"),
        created_by: Some("archivist".to_owned()),
    };
    synveda_store::knowledge::append_revision(
        tx,
        tenant,
        item.item.id,
        first.id,
        &second,
        &[source.id],
    )
    .await
    .unwrap()
    .unwrap();
    let item = SeededItem {
        tenant,
        scope,
        project,
        source: source.id,
    };
    seed_link(tx, &item, session).await;
    synveda_store::knowledge_search::insert_embedding(
        tx,
        tenant,
        second.id,
        "archive-fixture-16",
        &[
            1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ],
    )
    .await
    .unwrap();
    [first.id, second.id]
}

/// A decision about an import service, linked and then unlinked, so both
/// business-link tables hold a row.
async fn seed_link(
    tx: &mut sqlx::PgConnection,
    item: &SeededItem,
    session: &synveda_types::session::Session,
) {
    let tenant = item.tenant;
    let (decision, decision_revision) = item
        .create(
            tx,
            KnowledgeType::Decision,
            "Stop retrying the fictional import",
            serde_json::json!({}),
        )
        .await;
    let (service, _) = item
        .create(
            tx,
            KnowledgeType::Entity,
            "Fictional import service",
            serde_json::json!({"synveda_entity": {"schema_version": 1, "class": "schema:Service"}}),
        )
        .await;
    let link = EntityLinkId::new();
    synveda_store::entity_links::insert(
        tx,
        &synveda_store::entity_links::NewEntityLink {
            id: link,
            tenant_id: tenant,
            source_item_id: decision,
            target_item_id: service,
            asserting_revision_id: decision_revision,
            predicate: synveda_types::knowledge::EntityPredicate::Subject,
            valid_from: chrono::Utc::now() - chrono::Duration::days(1),
            created_by: "archivist",
        },
    )
    .await
    .unwrap();
    seed_link_step(tx, tenant, session, link, decision, service).await;
    assert!(
        synveda_store::entity_links::end(tx, tenant, link, chrono::Utc::now(), "archivist")
            .await
            .unwrap(),
        "the open link ends"
    );
}

/// A context run whose one candidate was reached across the business link,
/// so archive and erasure carry a retained step that names a link.
async fn seed_link_step(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    session: &synveda_types::session::Session,
    link: EntityLinkId,
    decision: KnowledgeItemId,
    service: KnowledgeItemId,
) {
    let from = synveda_store::knowledge::current(&mut *tx, tenant, decision)
        .await
        .unwrap()
        .unwrap();
    let to = synveda_store::knowledge::current(&mut *tx, tenant, service)
        .await
        .unwrap()
        .unwrap();
    let run = seed_run(tx, tenant, session).await;
    let candidate = ContextCandidateId::new();
    synveda_store::context::insert_candidate(
        tx,
        tenant,
        &synveda_store::context::NewContextCandidate {
            id: candidate,
            context_run_id: run,
            ordinal: 0,
            channel: configuration::ConfigurationContextChannel::CurrentKnowledge,
            knowledge_item_id: Some(to.item.id),
            knowledge_revision_id: Some(to.revision.id),
            capture_candidate_id: None,
            content_hash: to.revision.content_hash.clone(),
            scope_id: Some(to.item.scope_id),
            lifecycle_state: Some(knowledge::KnowledgeLifecycleState::Active),
            keyword_score_micros: 0,
            semantic_score_micros: 0,
            anchor_score_micros: 500_000,
            edge_weight_micros: 500_000,
            hop_penalty_micros: 100_000,
            freshness_score_micros: 0,
            pin_score_micros: 0,
            current_state_score_micros: 100_000,
            final_score_micros: 1_000_000,
            reason_codes: vec![context::ContextReasonCode::GraphExpansion],
            exclusion_reason: None,
        },
    )
    .await
    .unwrap();
    synveda_store::context::insert_graph_step(
        tx,
        tenant,
        &synveda_store::context::NewContextGraphStep {
            context_run_id: run,
            context_candidate_id: candidate,
            ordinal: 0,
            hop: 1,
            edge: ContextGraphEdge::EntityLink {
                predicate: knowledge::EntityPredicate::Subject,
                entity_link_id: Some(link),
            },
            relation_hash: blake3::hash(link.to_string().as_bytes())
                .to_hex()
                .to_string(),
            direction: ContextGraphDirection::Outbound,
            from_item_id: Some(from.item.id),
            from_revision_id: Some(from.revision.id),
            to_item_id: Some(to.item.id),
            to_revision_id: Some(to.revision.id),
            asserting_revision_id: Some(from.revision.id),
            from_content_hash: from.revision.content_hash,
            to_content_hash: to.revision.content_hash,
            edge_weight_micros: 500_000,
            supporting: true,
        },
    )
    .await
    .unwrap();
}

/// A completed context run in the Session that delivered nothing, so it
/// needs no delivery proof.
async fn seed_run(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    session: &synveda_types::session::Session,
) -> ContextRunId {
    let effective =
        synveda_store::configuration::effective_at_scope(&mut *tx, tenant, session.scope_id)
            .await
            .unwrap();
    let rendered = "";
    let run = synveda_store::sessions::record_context_run(
        tx,
        tenant,
        &synveda_store::sessions::NewContextRun {
            id: ContextRunId::new(),
            session_id: session.id,
            checkpoint_event_id: None,
            restart_event_ids: Vec::new(),
            workspace_id: session.workspace_id,
            project_id: session.project_id,
            scope_id: session.scope_id,
            principal_id: "archivist".to_owned(),
            configuration_version_id: effective.version_id,
            configuration_hash: effective.content_hash,
            query: Some("fictional import".to_owned()),
            query_hash: Some(blake3::hash(b"fictional import").to_hex().to_string()),
            rendered: rendered.to_owned(),
            block_hash: blake3::hash(rendered.as_bytes()).to_hex().to_string(),
            tokens: 0,
            budget_tokens: 256,
            requested_budget_tokens: None,
            entry_count: 0,
            candidate_count: 1,
            selection_count: 0,
            skills: serde_json::json!([]),
            degraded: Vec::new(),
            as_of: chrono::Utc::now(),
            retrieval_version: "knowledge-planner-v2".to_owned(),
            embedding_model: None,
            index_version: "postgres-fts-v1".to_owned(),
            graph_version: Some("knowledge-relations-entity-links-v1".to_owned()),
            trace_retention: TraceRetentionMode::Full,
            completion_status: ContextCompletionStatus::Completed,
            policy_exclusion: false,
        },
    )
    .await
    .unwrap();
    run.id
}

/// Where [`seed_link`] places its items.
struct SeededItem {
    tenant: TenantId,
    scope: ScopeId,
    project: ProjectId,
    source: KnowledgeSourceId,
}

impl SeededItem {
    async fn create(
        &self,
        tx: &mut sqlx::PgConnection,
        knowledge_type: KnowledgeType,
        title: &str,
        metadata: serde_json::Value,
    ) -> (KnowledgeItemId, KnowledgeRevisionId) {
        let revision = synveda_store::knowledge::NewKnowledgeRevision {
            id: KnowledgeRevisionId::new(),
            content: KnowledgeRevisionContent {
                title: title.to_owned(),
                body_markdown: format!("{title}."),
                summary: title.to_owned(),
                tags: Vec::new(),
                sensitivity: Sensitivity::Internal,
                confidence_permille: 900,
                valid_from: chrono::Utc::now(),
                valid_to: None,
                stale_after: None,
                verification_metadata: serde_json::json!({}),
                metadata,
            },
            created_by: Some("archivist".to_owned()),
        };
        let item = synveda_store::knowledge::create_item(
            tx,
            &synveda_store::knowledge::NewKnowledgeItem {
                id: KnowledgeItemId::new(),
                tenant_id: self.tenant,
                scope_id: self.scope,
                project_id: Some(self.project),
                owner_principal_id: None,
                knowledge_type,
                origin: KnowledgeOrigin::Authored,
                created_by: Some("archivist".to_owned()),
            },
            &revision,
            &[self.source],
        )
        .await
        .unwrap();
        (item.item.id, revision.id)
    }
}

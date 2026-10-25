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
/// and one policy pack. Returns the root scope and both revision ids.
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
    seed_session(&mut tx, tenant, workspace.id).await;
    let revisions = seed_knowledge(&mut tx, tenant, project.scope_id, project.id).await;
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

async fn seed_session(tx: &mut sqlx::PgConnection, tenant: TenantId, workspace: WorkspaceId) {
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
}

async fn seed_knowledge(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
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

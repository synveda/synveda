//! OPS-11 store tests: OKF import plans stay idempotent for every nullable
//! repository/source key pattern on the PostgreSQL 14 compatible expression
//! index that replaced NULLS NOT DISTINCT uniqueness (ADR-0152).
//!
//! Runs through `make db-test` as the ordinary gateway role; without
//! `DATABASE_URL` every test skips.

use std::sync::OnceLock;

use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::imports::{self, NewImportArtifact, NewImportMapping, NewImportPlan};
use synveda_store::projects::{self, NewProject};
use synveda_store::repositories::{self, NewRepository};
use synveda_store::workspaces::{self, NewWorkspace};
use synveda_types::import::{ImportArtifactKind, ImportMappingClassification};
use synveda_types::knowledge::{KnowledgeRevisionContent, KnowledgeType};
use synveda_types::{
    ImportArtifactId, ImportJobId, ImportMappingId, ProjectId, RepositoryId, ScopeId, Sensitivity,
    TenantId, TenantStatus, WorkspaceId,
};

#[path = "support/tenant_fixture.rs"]
mod tenant_fixture;

struct Db {
    rt: tokio::runtime::Runtime,
    pool: PgPool,
}

fn db() -> Option<&'static Db> {
    static DB: OnceLock<Option<Db>> = OnceLock::new();
    DB.get_or_init(|| {
        let url = tenant_fixture::database_url("import tests")?;
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
            synveda_store::epoch::verify(&pool)
                .await
                .expect("a current database");
            pool
        });
        Some(Db { rt, pool })
    })
    .as_ref()
}

struct Project {
    tenant: TenantId,
    workspace: WorkspaceId,
    project: ProjectId,
    scope: ScopeId,
}

async fn seed(pool: &PgPool) -> Project {
    let tenant = TenantId::new();
    tenant_fixture::create(
        pool,
        tenant,
        &format!("imports-{}", tenant.as_uuid().simple()),
        "Import idempotency fixture",
        TenantStatus::Active,
    )
    .await
    .expect("create tenant");
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("ensure tenant root");
    let workspace = workspaces::create(
        &mut tx,
        &NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: "imports".to_owned(),
            display_name: "Imports".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace");
    let project = projects::create(
        &mut tx,
        &NewProject {
            id: ProjectId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            slug: "imports".to_owned(),
            display_name: "Imports".to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create project");
    tx.commit().await.expect("commit import fixture");
    Project {
        tenant,
        workspace: workspace.id,
        project: project.id,
        scope: project.scope_id,
    }
}

fn plan(fixture: &Project, locator: &str, repository_id: Option<RepositoryId>) -> NewImportPlan {
    let artifact_id = ImportArtifactId::new();
    let content = KnowledgeRevisionContent {
        title: "Imported convention".to_owned(),
        body_markdown: "Imported body.".to_owned(),
        summary: "Imported convention".to_owned(),
        tags: Vec::new(),
        sensitivity: Sensitivity::Internal,
        confidence_permille: 800,
        valid_from: chrono::Utc::now(),
        valid_to: None,
        stale_after: None,
        verification_metadata: json!({}),
        metadata: json!({}),
    };
    let content_hash = synveda_types::knowledge::knowledge_revision_content_hash(&content);
    NewImportPlan {
        id: ImportJobId::new(),
        tenant_id: fixture.tenant,
        project_id: fixture.project,
        scope_id: fixture.scope,
        workspace_id: fixture.workspace,
        principal_id: "importer".to_owned(),
        format: "okf".to_owned(),
        format_version: "0.2".to_owned(),
        specification_commit: "ad30107c31c06aec8a7d5636e0d1058118604e6f".to_owned(),
        source_kind: "directory".to_owned(),
        source_locator: locator.to_owned(),
        source_revision: None,
        repository_id,
        managed_source_id: None,
        bundle_digest: blake3::hash(b"ops11-import-bundle").to_hex().to_string(),
        notices: Vec::new(),
        artifacts: vec![NewImportArtifact {
            id: artifact_id,
            ordinal: 1,
            logical_path: "concepts/convention.md".to_owned(),
            kind: ImportArtifactKind::Concept,
            content_hash: blake3::hash(b"Imported body.").to_hex().to_string(),
            frontmatter: json!({}),
            body_markdown: "Imported body.".to_owned(),
        }],
        mappings: vec![NewImportMapping {
            id: ImportMappingId::new(),
            artifact_id,
            ordinal: 1,
            okf_type: "concept".to_owned(),
            knowledge_type: KnowledgeType::Convention,
            content,
            content_hash,
            classification: ImportMappingClassification::Addition,
            matched_item_id: None,
            matched_revision_id: None,
            proposed_relations: json!([]),
            materializable: true,
        }],
    }
}

/// Two writers plan the same bundle for each nullable key pattern: exactly
/// one inserts, the other blocks on the unique index and then replays that
/// job. A different locator or repository is a distinct plan.
#[test]
fn concurrent_duplicate_plans_replay_one_job_per_nullable_key_pattern() {
    let Some(db) = db() else { return };
    db.rt.block_on(async {
        let fixture = seed(&db.pool).await;
        let mut tx = tenant_fixture::begin(&db.pool, fixture.tenant).await;
        let identity = synveda_types::repository::identify(
            Some("https://github.com/synveda/import-fixture.git"),
            None,
            None,
            None,
        )
        .expect("identify fixture repository");
        let repository = repositories::attach(
            &mut *tx,
            &NewRepository {
                id: RepositoryId::new(),
                tenant_id: fixture.tenant,
                project_id: fixture.project,
                identity,
                default_branch: None,
                metadata: json!({}),
                created_by: None,
            },
        )
        .await
        .expect("attach fixture repository");
        tx.commit().await.expect("commit repository");

        for repository_id in [None, Some(repository.id)] {
            let mut holder = tenant_fixture::begin(&db.pool, fixture.tenant).await;
            let first = imports::create_plan(&mut holder, &plan(&fixture, "race", repository_id))
                .await
                .expect("hold the first plan uncommitted");
            assert!(first.created);
            let holder_pid = tenant_fixture::backend_pid(&mut holder).await;

            let pool = db.pool.clone();
            let tenant = fixture.tenant;
            let duplicate = plan(&fixture, "race", repository_id);
            let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel();
            let waiter = tokio::spawn(async move {
                let mut tx = tenant_fixture::begin(&pool, tenant).await;
                pid_sender
                    .send(tenant_fixture::backend_pid(&mut tx).await)
                    .expect("report waiter pid");
                let planned = imports::create_plan(&mut tx, &duplicate).await;
                tx.commit().await.expect("commit replayed plan");
                planned
            });
            let waiter_pid = pid_receiver.await.expect("receive waiter pid");
            let mut observer = tenant_fixture::begin(&db.pool, fixture.tenant).await;
            tenant_fixture::wait_until_blocked_by(&mut observer, waiter_pid, holder_pid).await;
            observer
                .rollback()
                .await
                .expect("finish blocker observation");
            holder.commit().await.expect("commit the first plan");
            let replayed = tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
                .await
                .expect("the duplicate completes after the first commit")
                .expect("duplicate plan task")
                .expect("a duplicate plan replays the committed job");
            assert!(!replayed.created, "{repository_id:?}");
            assert_eq!(replayed.job.id, first.job.id, "{repository_id:?}");

            let mut tx = tenant_fixture::begin(&db.pool, fixture.tenant).await;
            let distinct = imports::create_plan(&mut tx, &plan(&fixture, "other", repository_id))
                .await
                .expect("a different locator is a distinct plan");
            assert!(distinct.created);
            tx.commit().await.expect("commit distinct plan");
        }
        let mut tx = tenant_fixture::begin(&db.pool, fixture.tenant).await;
        let jobs: i64 = sqlx::query_scalar(
            "select count(*) from import_jobs where tenant_id = $1 and project_id = $2",
        )
        .bind(fixture.tenant.as_uuid())
        .bind(fixture.project.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("count import jobs");
        assert_eq!(jobs, 4, "one raced and one distinct plan per key pattern");
        tx.commit().await.expect("finish import job count");
    });
}

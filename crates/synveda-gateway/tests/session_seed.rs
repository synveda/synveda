//! The shared fixture seam for suites that drive the memory pipeline
//! (CPR-12, ADR-0078).
//!
//! Every one of these suites used to write with `POST /v1/observe` and read
//! with `POST /v1/inject`. Both routes are deleted, and what replaced them
//! names the run it belongs to — so a suite that wants to put a statement into
//! the corpus needs a workspace and a session before it can write one.
//!
//! That bootstrap is the same two store calls in every suite, so it lives here
//! once. Included with `#[path = "session_seed.rs"] mod session_seed;` because
//! each integration test is its own crate and there is no other way to share
//! code between them.
//!
//! # Store-level, deliberately
//!
//! The workspace and the run are **fixtures**, seeded the same way these
//! suites already seed scopes, identities and grants. What stays on the public
//! routes is the thing under test: the append that admits a statement, and the
//! context run that reads it back. A fixture that went through the API would
//! need its own grants and its own idempotency keys in fifteen places, and
//! would test the workspace plane rather than the pipeline.
//!
//! # What changed for a suite that used to call `observe`
//!
//! - The write lands at the **run's** scope rather than the caller's own home
//!   (ADR-0078 decision 3). A suite that asserted a record's `scope_id` is the
//!   submitter's personal scope now asserts it is the workspace's, and needs
//!   its grants there rather than on a personal leaf.
//! - `kind` is gone. [`synveda_types::session::SessionEventType`] decides
//!   routing, and only the types that answer `capture_eligible` reach the
//!   extractor at all — so a fixture that wants a memory must use one.
//! - The read is a **context run**: `rendered` rather than `text`, and no
//!   `record_ids`.

#![allow(dead_code)]

use sqlx::PgPool;
use synveda_store::{access, sessions, workspaces};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::{GrantId, ScopeId, SessionId, TenantId, WorkspaceId};

/// A seeded workspace and the run opened in it.
pub struct SeededRun {
    pub workspace_id: WorkspaceId,
    /// The workspace's own governed scope — where this run's memories land,
    /// and therefore where a suite's grants have to reach.
    pub workspace_scope_id: ScopeId,
    pub session_id: SessionId,
}

/// Creates a workspace and opens a run in one ordinary tenant transaction.
///
/// `slug` must be unique per tenant; suites derive it from whatever they
/// already use to keep fixtures apart. `principal` is the token subject the
/// run is attributed to — the same subject the suite's bearer carries, because
/// extraction re-decides `KnowledgeWrite` for it at commit time.
pub async fn seed_run(pool: &PgPool, tenant: TenantId, slug: &str, principal: &str) -> SeededRun {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant-scoped session fixture");
    let workspace = workspaces::create(
        &mut tx,
        &workspaces::NewWorkspace {
            id: WorkspaceId::new(),
            tenant_id: tenant,
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            description: None,
            created_by: None,
        },
    )
    .await
    .expect("create the fixture workspace");
    let session = sessions::create(
        &mut tx,
        &sessions::NewSession {
            id: SessionId::new(),
            tenant_id: tenant,
            workspace_id: workspace.id,
            project_id: None,
            principal_id: principal.to_owned(),
            client_name: "test-harness".to_owned(),
            client_version: None,
            client_installation_id: None,
            external_session_id: Some(slug.to_owned()),
            agent_name: None,
            model_name: None,
            repository_id: None,
            branch: None,
            task_summary: None,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("open the fixture run");
    tx.commit().await.expect("commit session fixture");
    SeededRun {
        workspace_id: workspace.id,
        workspace_scope_id: workspace.scope_id,
        session_id: session.id,
    }
}

/// Opens a second run in an existing workspace — for suites that need two.
pub async fn open_run(
    pool: &PgPool,
    tenant: TenantId,
    workspace_id: WorkspaceId,
    slug: &str,
    principal: &str,
) -> SessionId {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant-scoped run fixture");
    let session = sessions::create(
        &mut tx,
        &sessions::NewSession {
            id: SessionId::new(),
            tenant_id: tenant,
            workspace_id,
            project_id: None,
            principal_id: principal.to_owned(),
            client_name: "test-harness".to_owned(),
            client_version: None,
            client_installation_id: None,
            external_session_id: Some(slug.to_owned()),
            agent_name: None,
            model_name: None,
            repository_id: None,
            branch: None,
            task_summary: None,
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("open the fixture run");
    tx.commit().await.expect("commit run fixture");
    session.id
}

/// Grants `subject` a role at `scope`.
///
/// Needed by every suite that used to lean on the role-free own-home write
/// floor: a memory lands at the run's scope now, and a write beyond your own
/// personal scope has always taken an explicit grant (ADR-0020 decision 3).
pub async fn grant_at(
    pool: &PgPool,
    tenant: TenantId,
    subject: &str,
    scope: ScopeId,
    role: RoleKey,
) {
    let mut tx = synveda_store::rls::begin_tenant_tx(pool, tenant)
        .await
        .expect("begin tenant-scoped grant");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key: role,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("write the grant");
    tx.commit().await.expect("commit grant");
}

/// Seeds a workspace, opens a run in it, and grants `principal` `member`
/// there — the whole bootstrap a pipeline suite needs, in one call.
pub async fn seed_run_for(
    pool: &PgPool,
    tenant: TenantId,
    slug: &str,
    principal: &str,
) -> SeededRun {
    let run = seed_run(pool, tenant, slug, principal).await;
    grant_at(
        pool,
        tenant,
        principal,
        run.workspace_scope_id,
        RoleKey::Member,
    )
    .await;
    run
}

/// The append body for one statement, as `POST /v1/sessions/{id}/events`
/// takes it.
///
/// `message.user` because that is what a person saying something is, and it is
/// one of the types that carries memory — a bookkeeping type would be recorded
/// and never signalled, so a fixture built on one would silently extract
/// nothing.
pub fn one_event(event_id: &str, at: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "events": [{
            "event_type": "message.user",
            "client_event_id": event_id,
            "occurred_at": at,
            "payload": {"text": text},
        }],
    })
}

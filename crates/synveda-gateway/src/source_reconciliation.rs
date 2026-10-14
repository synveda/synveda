//! MEM-9 operational inbound reads; only ordinary Capture review publishes.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_git::{LocalSourceReader, SourcePin, SourceSnapshot};
use synveda_okf::{
    BundleInput, InputEntry, InputEntryKind, KnowledgeFormatAdapter, OkfAdapter, SourceDescriptor,
    SourceKind,
};
use synveda_policy::{Action, Pdp, Resource, ResourceEntity};
use synveda_store::{
    anchors::AnchorSelection, configuration, identities, imports, knowledge, projects,
    repositories, rls, scopes, source_reconciliation as store,
};
use synveda_types::configuration::EffectiveConfiguration;
use synveda_types::source::GitSourceDescriptor;
use synveda_types::{
    DurableOperationId, Error, Identity, IdentityId, IdentityKind, ImportJobId, ProjectId, Result,
    TenantId,
};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    app::AppState,
    audit, authz,
    idempotency::{Claim, Dispatch},
    request,
    workspaces::subject,
};

const COMPONENT: &str = "source-reconcile";

/// Public content-free refresh receipt; import expansion uses its own API.
#[derive(Serialize, ToSchema)]
pub struct SourceReconcileView {
    /// Stable runtime operation address.
    #[schema(value_type=String,format="uuid")]
    pub id: Uuid,
    /// Closed operational family.
    #[schema(schema_with=source_kind_schema)]
    pub kind: String,
    /// Current immutable contract version.
    pub operation_version: u16,
    /// Actual project.
    #[schema(value_type=String,format="uuid")]
    pub project_id: Uuid,
    /// Stable configured source.
    pub source_id: String,
    /// Closed durable lifecycle.
    #[schema(schema_with=crate::source_control::cleanup_state_schema)]
    pub state: String,
    /// Persisted bounded attempt count.
    pub attempts: i32,
    /// Exact once-only observed commit when resolved.
    pub pinned_revision: Option<String>,
    /// Committed complete observation, absent for incomplete work.
    #[schema(value_type=Option<String>,format="uuid")]
    pub observation_id: Option<Uuid>,
    /// Existing import review address, when changed bytes produced candidates.
    #[schema(value_type=Option<String>,format="uuid")]
    pub import_job_id: Option<Uuid>,
    /// Safe failure code, never provider output.
    pub error_code: Option<String>,
    /// Durable database request time.
    pub created_at: DateTime<Utc>,
}

fn source_kind_schema() -> utoipa::openapi::schema::Object {
    crate::workspaces::string_enum(["source_reconcile"].into_iter())
}

impl From<store::SourceRequest> for SourceReconcileView {
    fn from(r: store::SourceRequest) -> Self {
        Self {
            id: r.operation_id,
            kind: "source_reconcile".to_owned(),
            operation_version: 1,
            project_id: r.project_id,
            source_id: r.source_id,
            state: r.state,
            attempts: r.attempts,
            pinned_revision: r.commit_oid,
            observation_id: r
                .result
                .get("observation_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok()),
            import_job_id: r
                .result
                .get("import_job_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok()),
            error_code: r.error_code,
            created_at: r.created_at,
        }
    }
}

/// Configured source and its last independently inspectable refresh receipt.
#[derive(Serialize, ToSchema)]
pub struct SourceView {
    /// Stable configured identity.
    pub source_id: String,
    /// Actual project repository attachment.
    #[schema(value_type=String,format="uuid")]
    pub repository_id: Uuid,
    /// Exact configured branch reference.
    pub git_ref: String,
    /// Exact approved coverage; no inferred source inventory.
    pub allowed_paths: Vec<String>,
    /// Disconnect retains previous approved Knowledge.
    pub enabled: bool,
    /// Last fully committed snapshot.
    #[schema(value_type=Option<String>,format="uuid")]
    pub observation_id: Option<Uuid>,
    /// Latest runtime work, including failed or blocked work.
    pub operation: Option<SourceReconcileView>,
}

/// Bounded source list from effective Configuration.
#[derive(Serialize, ToSchema)]
pub struct SourceListView {
    /// At most sixteen authorized descriptors belonging to this project.
    pub sources: Vec<SourceView>,
}

fn unavailable() -> Error {
    Error::NotFound {
        entity: "Source".to_owned(),
    }
}
fn changed() -> Error {
    Error::Conflict {message:"Source refresh configuration or custody changed; inspect current configuration and request a new refresh".to_owned()}
}
fn descriptor_hash(descriptor: &GitSourceDescriptor) -> Result<String> {
    let value = serde_json::to_value(descriptor).map_err(|_| changed())?;
    Ok(blake3::hash(
        synveda_types::json::canonicalise(&value)
            .to_string()
            .as_bytes(),
    )
    .to_hex()
    .to_string())
}
fn custody(
    reader: &LocalSourceReader,
    tenant: TenantId,
    scope: synveda_types::ScopeId,
    target: &str,
) -> Result<String> {
    let path = reader.repository(tenant, scope, target)?;
    let value = json!(["synveda.git.source.local.v1", tenant, scope, target, path]);
    Ok(blake3::hash(value.to_string().as_bytes())
        .to_hex()
        .to_string())
}

pub(crate) async fn authorize_project(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    project: ProjectId,
    action: Action,
) -> Result<synveda_types::workspace::Project> {
    let (project, allowed, resource) =
        crate::okf::authorize_project(state, tx, tenant, project, action).await?;
    audit::record(
        tx,
        tenant,
        AuditAction::AuthzDecision,
        resource.to_string(),
        Outcome::Allow,
        json!({"op":"sources", "authz":audit::decision_context(action, &allowed)}),
    )
    .await?;
    Ok(project)
}

async fn configured(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    project: ProjectId,
    source: &str,
) -> Result<(
    synveda_types::workspace::Project,
    EffectiveConfiguration,
    GitSourceDescriptor,
)> {
    synveda_types::source::validate_source_id(source)?;
    let project = projects::get(&mut *tx, tenant, project)
        .await?
        .ok_or_else(unavailable)?;
    let configuration = configuration::effective_at_scope(tx, tenant, project.scope_id).await?;
    let descriptor = configuration
        .document
        .git_sources
        .iter()
        .find(|s| s.source_id == source)
        .cloned()
        .ok_or_else(unavailable)?;
    let repository = repositories::get(&mut *tx, tenant, project.id, descriptor.repository_id)
        .await?
        .ok_or_else(unavailable)?;
    if repository.project_id != project.id {
        return Err(unavailable());
    }
    descriptor.validate()?;
    Ok((project, configuration, descriptor))
}

async fn source_view(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    project: ProjectId,
    descriptor: GitSourceDescriptor,
) -> Result<SourceView> {
    let head = store::head(tx, tenant, project, &descriptor.source_id).await?;
    let operation = if let Some(id) = head.as_ref().and_then(|h| h.last_operation_id) {
        store::request(tx, tenant, DurableOperationId::from_uuid(id))
            .await?
            .map(Into::into)
    } else {
        None
    };
    Ok(SourceView {
        source_id: descriptor.source_id,
        repository_id: descriptor.repository_id.as_uuid(),
        git_ref: descriptor.git_ref,
        allowed_paths: descriptor.allowed_paths,
        enabled: descriptor.enabled,
        observation_id: head.and_then(|h| h.observation_id),
        operation,
    })
}

async fn respond<T: IntoResponse>(
    state: &AppState,
    op: &'static str,
    result: Result<T>,
) -> Response {
    let result = match result {
        Err(error @ Error::PolicyDenied { .. }) => {
            audit::record_rejection(state, op, &error).await;
            Err(unavailable())
        }
        Err(Error::NotFound { .. }) => Err(unavailable()),
        other => other,
    };
    crate::response::finish(state, op, result).await
}

#[utoipa::path(get,path="/v1/projects/{project_id}/sources",operation_id="list_sources",tag="sources",
 params(("project_id"=String,Path)),responses((status=200,description="Authorized configured sources",body=SourceListView),
 (status=404,description="Absent or denied",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn list(
    State(state): State<AppState>,
    Path(project): Path<ProjectId>,
) -> Response {
    let result = async {
        let tenant = request::tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let state = crate::context_api::policy_snapshot::current(&state, &mut tx, tenant).await?;
        let actual =
            authorize_project(&state, &mut tx, tenant, project, Action::SourceRead).await?;
        let config = configuration::effective_at_scope(&mut tx, tenant, actual.scope_id).await?;
        let mut sources = Vec::new();
        for descriptor in config.document.git_sources {
            if repositories::get(&mut *tx, tenant, project, descriptor.repository_id)
                .await?
                .is_some_and(|r| r.project_id == project)
            {
                sources.push(source_view(&mut tx, tenant, project, descriptor).await?);
            }
        }
        request::commit(tx).await?;
        Ok(Json(SourceListView { sources }))
    }
    .await;
    respond(&state, "sources.list", result).await
}

#[utoipa::path(get,path="/v1/projects/{project_id}/sources/{source_id}",operation_id="get_source",tag="sources",
 params(("project_id"=String,Path),("source_id"=String,Path)),responses((status=200,description="Source and refresh state",body=SourceView),
 (status=404,description="Absent or denied",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn get(
    State(state): State<AppState>,
    Path((project, source)): Path<(ProjectId, String)>,
) -> Response {
    let result = async {
        let tenant = request::tenant_id()?;
        let mut tx = rls::begin_tenant_tx(&state.pool, tenant).await?;
        let state = crate::context_api::policy_snapshot::current(&state, &mut tx, tenant).await?;
        authorize_project(&state, &mut tx, tenant, project, Action::SourceRead).await?;
        let (_, _, descriptor) = configured(&mut tx, tenant, project, &source).await?;
        let view = source_view(&mut tx, tenant, project, descriptor).await?;
        request::commit(tx).await?;
        Ok(Json(view))
    }
    .await;
    respond(&state, "sources.get", result).await
}

#[utoipa::path(post,path="/v1/projects/{project_id}/sources/{source_id}/refresh",operation_id="refresh_source",tag="sources",
 params(("project_id"=String,Path),("source_id"=String,Path),("Idempotency-Key"=String,Header)),
 responses((status=202,description="Durable refresh queued",body=SourceReconcileView),(status=200,description="Exact receipt replay",body=SourceReconcileView),
 (status=404,description="Absent or denied",body=crate::workspaces::ApiErrorBody),(status=409,description="Changed request or active refresh",body=crate::workspaces::ApiErrorBody)),security(("bearer"=[])))]
pub(crate) async fn refresh(
    State(state): State<AppState>,
    Path((project, source)): Path<(ProjectId, String)>,
    headers: HeaderMap,
) -> Response {
    let result=async {
        let tenant=request::tenant_id()?;
        let claim=Claim::from_headers(&headers,"source.refresh",&subject()?,&json!({"project_id":project,"source_id":source,"operation_version":1}))?;
        let replay=crate::idempotency::dispatch(&state.pool,tenant,&claim).await?;
        let mut tx=rls::begin_tenant_write_tx(&state.pool,tenant).await?;
        let current=crate::context_api::policy_snapshot::current(&state,&mut tx,tenant).await?;
        authorize_project(&current,&mut tx,tenant,project,Action::SourceRead).await?;
        authorize_project(&current,&mut tx,tenant,project,Action::KnowledgeWrite).await?;
        if let Dispatch::Replay(id)=replay {
            let view: SourceReconcileView=store::request(&mut tx,tenant,DurableOperationId::from_uuid(id)).await?.ok_or_else(unavailable)?.into();
            request::commit(tx).await?;return Ok((StatusCode::OK,Json(view)));
        }
        // Re-read the key after exclusive admission to cover concurrent retries.
        if let Some(old)=synveda_store::idempotency::find(&mut *tx,tenant,&claim.subject,claim.operation,&claim.key).await? {
            if old.request_digest!=claim.digest {return Err(changed());}
            let view: SourceReconcileView=store::request(&mut tx,tenant,DurableOperationId::from_uuid(old.resource_id)).await?.ok_or_else(unavailable)?.into();
            request::commit(tx).await?;return Ok((StatusCode::OK,Json(view)));
        }
        let (actual,configuration,descriptor)=configured(&mut tx,tenant,project,&source).await?;
        if !descriptor.enabled {return Err(changed());}
        let identity=identities::by_subject(&mut *tx,tenant,&claim.subject).await?.ok_or_else(unavailable)?;
        if identity.kind!=IdentityKind::User || identity.sealed() {return Err(Error::Invalid {message:"Durable source refresh requires an active User requester".to_owned()});}
        let reader=state.git_sources.as_deref().ok_or_else(changed)?;
        let version=configuration.version_id.ok_or_else(changed)?;
        let digest=descriptor_hash(&descriptor)?;
        let custody=custody(reader,tenant,actual.scope_id,&descriptor.read_target)?;
        let id=store::enqueue(&mut tx,tenant,project,actual.scope_id,&source,descriptor.repository_id,version,&digest,&custody,None,
            i32::try_from(descriptor.allowed_paths.len()).map_err(|_|changed())?,identity.id,blake3::Hash::from_bytes(claim.digest).to_hex().as_ref()).await?;
        claim.remember(&mut tx,tenant,id.as_uuid()).await?;
        audit::record(&mut tx,tenant,AuditAction::OperationRequested,Resource::Scope(actual.scope_id).to_string(),Outcome::Success,
            json!({"operation_id":id,"kind":"source_reconcile","operation_version":1,"configuration_version_id":version,"descriptor_hash":digest})).await?;
        let view: SourceReconcileView=store::request(&mut tx,tenant,id).await?.ok_or_else(unavailable)?.into();
        request::commit(tx).await?;Ok((StatusCode::ACCEPTED,Json(view)))
    }.await;
    respond(&state, "sources.refresh", result).await
}

/// Resolve the public runtime receipt under current source authority.
pub(crate) async fn operation_view(
    state: &AppState,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    id: DurableOperationId,
) -> Result<Option<SourceReconcileView>> {
    let Some(request) = store::request(tx, tenant, id).await? else {
        return Ok(None);
    };
    let state = crate::context_api::policy_snapshot::current(state, tx, tenant).await?;
    authorize_project(
        &state,
        tx,
        tenant,
        ProjectId::from_uuid(request.project_id),
        Action::SourceRead,
    )
    .await?;
    Ok(Some(request.into()))
}

/// Narrow supervised worker dependencies, with no fabricated HTTP claims.
#[derive(Clone)]
pub struct Runtime {
    pool: sqlx::PgPool,
    pdp: Arc<Pdp>,
    reader: Arc<LocalSourceReader>,
    worker_id: String,
}

impl Runtime {
    /// Assemble a one-at-a-time producer; deployment custody remains separate.
    #[must_use]
    pub fn new(pool: sqlx::PgPool, pdp: Arc<Pdp>, reader: Arc<LocalSourceReader>) -> Self {
        Self {
            pool,
            pdp,
            reader,
            worker_id: format!("{COMPONENT}:{}", Uuid::now_v7()),
        }
    }
}

struct Admission {
    project: synveda_types::workspace::Project,
    configuration: EffectiveConfiguration,
    descriptor: GitSourceDescriptor,
    identity: Identity,
    pdp: Arc<Pdp>,
}

async fn admit(
    runtime: &Runtime,
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    target: &store::SourceRequest,
) -> Result<Admission> {
    let (project, configuration, descriptor) = configured(
        tx,
        tenant,
        ProjectId::from_uuid(target.project_id),
        &target.source_id,
    )
    .await?;
    if !descriptor.enabled
        || descriptor.repository_id.as_uuid() != target.repository_id
        || configuration.version_id.map(|v| v.as_uuid()) != Some(target.configuration_version_id)
        || descriptor_hash(&descriptor)? != target.descriptor_hash
        || custody(
            &runtime.reader,
            tenant,
            project.scope_id,
            &descriptor.read_target,
        )? != target.custody_digest
    {
        return Err(changed());
    }
    let identity = identities::by_id(&mut *tx, tenant, IdentityId::from_uuid(target.requested_by))
        .await?
        .ok_or_else(unavailable)?;
    let scope = scopes::get(&mut *tx, tenant, project.scope_id)
        .await?
        .ok_or_else(unavailable)?;
    let pdp = crate::context_api::policy_snapshot::current_pdp(&runtime.pdp, tx, tenant)
        .await
        .map_err(|error| match error {
            Error::Conflict { .. } => Error::Dependency {
                service: "policy-snapshot".to_owned(),
                message: "Current source policy snapshot is temporarily unavailable".to_owned(),
            },
            other => other,
        })?;
    let input = authz::gather_for_operation_identity(
        tx,
        tenant,
        identity.clone(),
        Some(&scope),
        AnchorSelection::project(project.id),
        Vec::new(),
    )
    .await?;
    for action in [Action::SourceRead, Action::KnowledgeWrite] {
        let decision =
            authz::decide_with_pdp(&pdp, &input, action, Resource::Scope(project.scope_id))?;
        audit::record_as(tx,tenant,Actor::system(COMPONENT),AuditAction::AuthzDecision,Resource::Scope(project.scope_id).to_string(),Outcome::Allow,
            json!({"operation_id":target.operation_id,"authz":audit::decision_context(action,&decision)})).await?;
    }
    Ok(Admission {
        project,
        configuration,
        descriptor,
        identity,
        pdp,
    })
}

async fn matching(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    admission: &Admission,
    paths: &[String],
) -> Result<BTreeMap<String, Vec<crate::okf::ManagedSourceMatch>>> {
    let mut matched = BTreeMap::new();
    let mut inspected = 0usize;
    for path in paths {
        let mut visible = Vec::new();
        for item in store::current_items(
            tx,
            tenant,
            admission.project.id,
            &admission.descriptor.source_id,
            path,
        )
        .await?
        {
            let id = synveda_types::KnowledgeItemId::from_uuid(item.id);
            inspected += 1;
            if inspected > 96 {
                return Err(changed());
            }
            let Some(snapshot) = knowledge::current(&mut *tx, tenant, id).await? else {
                continue;
            };
            let scope = scopes::get(&mut *tx, tenant, snapshot.item.scope_id)
                .await?
                .ok_or_else(unavailable)?;
            let input = authz::gather_for_operation_identity(
                tx,
                tenant,
                admission.identity.clone(),
                Some(&scope),
                snapshot
                    .item
                    .project_id
                    .map_or_else(AnchorSelection::none, AnchorSelection::project),
                vec![ResourceEntity::KnowledgeItem {
                    id,
                    scope_id: snapshot.item.scope_id,
                }],
            )
            .await?;
            match authz::decide_knowledge_read_with_pdp(
                &admission.pdp,
                &input,
                Resource::KnowledgeItem(id),
                snapshot.revision.content.sensitivity,
            ) {
                Ok(allowed) => {
                    audit::record_as(
                        tx,
                        tenant,
                        Actor::system(COMPONENT),
                        AuditAction::AuthzDecision,
                        Resource::KnowledgeItem(id).to_string(),
                        Outcome::Allow,
                        json!({"authz":audit::decision_context(Action::KnowledgeRead,&allowed)}),
                    )
                    .await?;
                    visible.push(crate::okf::ManagedSourceMatch {
                        snapshot,
                        source_hash: item.content_hash,
                    });
                }
                Err(Error::PolicyDenied { .. } | Error::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        matched.insert(path.clone(), visible);
    }
    Ok(matched)
}

async fn execute_claim(
    runtime: &Runtime,
    tenant: TenantId,
    target: &store::SourceRequest,
) -> Result<()> {
    let id = DurableOperationId::from_uuid(target.operation_id);
    let mut before = rls::begin_tenant_tx(&runtime.pool, tenant).await?;
    store::fence(&mut before, tenant, id, target.attempts, &runtime.worker_id).await?;
    let admission = admit(runtime, &mut before, tenant, target).await?;
    let mut omitted = BTreeMap::new();
    for path in &admission.descriptor.allowed_paths {
        if let Some(prior) =
            store::prior_path(&mut before, tenant, target.previous_observation_id, path).await?
            && let (Some(hash), Some(blob)) = (prior.content_hash, prior.blob_oid)
            && store::artifact_state(
                &mut before,
                tenant,
                admission.project.id,
                &admission.descriptor.source_id,
                path,
                &hash,
            )
            .await?
            .1
        {
            omitted.insert(path.clone(), blob);
        }
    }
    request::commit(before).await?;
    // Every Git wait is outside a database transaction and its admission gate.
    let pin = match (&target.commit_oid, &target.tree_oid) {
        (Some(commit), Some(tree)) => SourcePin {
            commit: commit.clone(),
            tree: tree.clone(),
        },
        (None, None) => {
            let pin = runtime
                .reader
                .resolve(
                    tenant,
                    admission.project.scope_id,
                    &admission.descriptor.read_target,
                    &admission.descriptor.git_ref,
                )
                .await?;
            let mut pinned = rls::begin_tenant_tx(&runtime.pool, tenant).await?;
            store::fence(&mut pinned, tenant, id, target.attempts, &runtime.worker_id).await?;
            admit(runtime, &mut pinned, tenant, target).await?;
            store::pin(&mut pinned, tenant, id, &pin.commit, &pin.tree).await?;
            request::commit(pinned).await?;
            pin
        }
        _ => return Err(changed()),
    };
    let snapshot = runtime
        .reader
        .read_at_omitting(
            tenant,
            admission.project.scope_id,
            &admission.descriptor.read_target,
            &pin,
            &admission.descriptor.allowed_paths,
            &omitted,
        )
        .await?;
    commit_snapshot(runtime, tenant, target, snapshot).await
}

async fn commit_snapshot(
    runtime: &Runtime,
    tenant: TenantId,
    target: &store::SourceRequest,
    snapshot: SourceSnapshot,
) -> Result<()> {
    let id = DurableOperationId::from_uuid(target.operation_id);
    let mut tx = rls::begin_tenant_write_tx(&runtime.pool, tenant).await?;
    store::fence(&mut tx, tenant, id, target.attempts, &runtime.worker_id).await?;
    let target = store::request(&mut tx, tenant, id)
        .await?
        .ok_or_else(unavailable)?;
    let admission = admit(runtime, &mut tx, tenant, &target).await?;
    if target.commit_oid.as_deref() != Some(&snapshot.pin.commit)
        || target.tree_oid.as_deref() != Some(&snapshot.pin.tree)
    {
        return Err(changed());
    }
    let mut paths = Vec::new();
    let mut entries = Vec::new();
    let current = matching(
        &mut tx,
        tenant,
        &admission,
        &admission.descriptor.allowed_paths,
    )
    .await?;
    for file in snapshot.files {
        if file.omitted {
            let prior =
                store::prior_path(&mut tx, tenant, target.previous_observation_id, &file.path)
                    .await?
                    .ok_or_else(changed)?;
            let hash = prior.content_hash.ok_or_else(changed)?;
            if prior.blob_oid != file.blob_oid {
                return Err(changed());
            }
            let (artifact, suppressed) = store::artifact_state(
                &mut tx,
                tenant,
                admission.project.id,
                &admission.descriptor.source_id,
                &file.path,
                &hash,
            )
            .await?;
            if !suppressed {
                return Err(changed());
            }
            paths.push(store::ObservedPath {
                path: file.path,
                state: "suppressed",
                blob_oid: file.blob_oid,
                content_hash: Some(hash),
                artifact_id: artifact,
            });
            continue;
        }
        let Some(bytes) = file.bytes else {
            paths.push(store::ObservedPath {
                path: file.path,
                state: "missing",
                blob_oid: None,
                content_hash: None,
                artifact_id: None,
            });
            continue;
        };
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let (artifact, suppressed) = store::artifact_state(
            &mut tx,
            tenant,
            admission.project.id,
            &admission.descriptor.source_id,
            &file.path,
            &hash,
        )
        .await?;

        paths.push(store::ObservedPath {
            path: file.path.clone(),
            state: if suppressed { "suppressed" } else { "present" },
            blob_oid: file.blob_oid,
            content_hash: Some(hash.clone()),
            artifact_id: artifact,
        });
        let needs_review = current
            .get(&file.path)
            .is_some_and(|items| items.iter().any(|item| item.source_hash != hash));
        if !suppressed && (artifact.is_none() || needs_review) {
            entries.push(InputEntry {
                logical_path: file.path,
                kind: InputEntryKind::File,
                bytes,
            });
        }
    }
    let mut job: Option<ImportJobId> = None;
    let has_concept = entries.iter().any(|entry| {
        !matches!(
            entry.logical_path.rsplit('/').next(),
            Some("index.md" | "log.md")
        )
    });
    if has_concept {
        let repository = repositories::get(
            &mut *tx,
            tenant,
            admission.project.id,
            admission.descriptor.repository_id,
        )
        .await?
        .ok_or_else(unavailable)?;
        let inspection = OkfAdapter.inspect(
            SourceDescriptor {
                kind: SourceKind::Git,
                locator: repository.canonical_uri,
                revision: Some(snapshot.pin.commit),
            },
            BundleInput::Entries(entries),
            target.created_at,
        )?;
        if inspection.concepts.len()
            > usize::try_from(
                admission
                    .configuration
                    .document
                    .capture
                    .maximum_candidates_per_batch,
            )
            .map_err(|_| changed())?
        {
            return Err(changed());
        }
        let plan = crate::okf::new_managed_plan(
            tenant,
            &admission.project,
            admission.identity.subject.clone().ok_or_else(unavailable)?,
            inspection,
            &admission.descriptor,
            &current,
        )?;
        let planned = imports::create_plan(&mut tx, &plan).await?;
        imports::materialize(&mut tx, tenant, planned.job.id, &admission.configuration).await?;
        for artifact in imports::artifacts(&mut *tx, tenant, planned.job.id).await? {
            if let Some(path) = paths.iter_mut().find(|p| p.path == artifact.logical_path) {
                path.artifact_id = Some(artifact.id);
            }
        }
        audit::record_as(&mut tx,tenant,Actor::system(COMPONENT),AuditAction::OkfImportMaterialized,Resource::Scope(admission.project.scope_id).to_string(),Outcome::Success,
            json!({"operation_id":id,"import_job_id":planned.job.id,"bundle_digest":planned.job.bundle_digest})).await?;
        job = Some(planned.job.id);
    }
    store::fence(&mut tx, tenant, id, target.attempts, &runtime.worker_id).await?;
    let observation = store::complete(&mut tx, tenant, &target, job, &paths).await?;
    record_finished(
        &mut tx,
        tenant,
        &target,
        Outcome::Success,
        Some(observation),
    )
    .await?;
    request::commit(tx).await?;
    Ok(())
}

async fn record_finished(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    target: &store::SourceRequest,
    outcome: Outcome,
    observation: Option<Uuid>,
) -> Result<()> {
    audit::record_as(tx,tenant,Actor::system(COMPONENT),AuditAction::OperationFinished,format!("operation {}",target.operation_id),outcome,
        json!({"operation_id":target.operation_id,"kind":"source_reconcile","operation_version":1,"attempt":target.attempts,"observation_id":observation})).await.map(|_|())
}

/// Execute one due refresh, returning false when this tenant has no due work.
pub async fn execute_page(runtime: &Runtime, tenant: TenantId) -> Result<bool> {
    let mut tx = rls::begin_tenant_tx(&runtime.pool, tenant).await?;
    let Some(claim) = store::claim(&mut tx, tenant, &runtime.worker_id).await? else {
        return Ok(false);
    };
    if claim.exhausted {
        record_finished(&mut tx, tenant, &claim.request, Outcome::Failure, None).await?;
        request::commit(tx).await?;
        return Ok(true);
    }
    request::commit(tx).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        execute_claim(runtime, tenant, &claim.request),
    )
    .await
    .unwrap_or_else(|_| {
        Err(Error::Internal {
            message: "Source read deadline expired".to_owned(),
        })
    });
    if let Err(error) = result {
        let permanent = matches!(
            error,
            Error::PolicyDenied { .. } | Error::NotFound { .. } | Error::Conflict { .. }
        );
        let code = if permanent {
            "source_admission_changed"
        } else {
            "source_snapshot_incomplete"
        };
        let mut failed = rls::begin_tenant_write_tx(&runtime.pool, tenant).await?;
        match store::fail(
            &mut failed,
            tenant,
            &claim.request,
            &runtime.worker_id,
            code,
            permanent,
        )
        .await
        {
            Ok(()) => {
                if permanent || claim.request.attempts >= 3 {
                    record_finished(&mut failed, tenant, &claim.request, Outcome::Failure, None)
                        .await?;
                }
                if matches!(error, Error::PolicyDenied { .. }) {
                    audit::record_as(&mut failed,tenant,Actor::system(COMPONENT),AuditAction::AuthzDecision,
                        format!("operation {}",claim.request.operation_id),Outcome::Deny,
                        json!({"operation_id":claim.request.operation_id,"action":"source.read","error_code":"source_admission_changed"})).await?;
                }
                request::commit(failed).await?;
            }
            Err(Error::Conflict { .. }) => {
                metrics::counter!("synveda_source_reconcile_operations_total","outcome"=>"lease_lost").increment(1);
                return Err(error);
            }
            Err(failure) => return Err(failure),
        }
        metrics::counter!("synveda_source_reconcile_operations_total","outcome"=>if permanent {"blocked"}else{"failed"}).increment(1);

        return Err(error);
    } else {
        metrics::counter!("synveda_source_reconcile_operations_total","outcome"=>"succeeded")
            .increment(1);
    }
    Ok(true)
}

/// Existing worker supervision runs one bounded source operation at a time.
pub(crate) async fn run(runtime: Runtime, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut cursor = None;
    loop {
        if *stop.borrow() {
            return;
        }
        let page = match store::active_tenants(&runtime.pool, cursor).await {
            Ok(page) => page,
            Err(_) => {
                tracing::warn!("source worker tenant sweep deferred");
                Vec::new()
            }
        };
        if page.is_empty() {
            cursor = None;
        } else {
            for tenant in page {
                cursor = Some(tenant);
                tokio::select! {
                    result=execute_page(&runtime,tenant)=>{if result.is_err(){tracing::warn!("source refresh deferred after failure");}},
                    _=stop.changed()=>{return;},
                }
            }
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_secs(5))=>{},_=stop.changed()=>{return;}}
    }
}

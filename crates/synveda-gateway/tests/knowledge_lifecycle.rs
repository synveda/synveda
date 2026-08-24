//! CPR-16 acceptance evidence: every Knowledge mutation is a PDP decision
//! followed by one VedaFlow change, and only the live approval matrix may
//! execute its typed effect. The suite covers the complete command vocabulary
//! against Postgres, including immutable history, provenance-preserving merge,
//! explicit supersession and content erasure.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use chrono::Utc;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_gateway::app::{AppState, router};
use synveda_gateway::{knowledge, telemetry};
use synveda_identity::{Claims, Hs256Verifier, TenantContext, with_tenant};
use synveda_ingest::embedding::Embedder as _;
use synveda_store::{
    access, identities, knowledge as stored, knowledge_search, policy_assignments, rls, scopes,
    tenants,
};
use synveda_types::access::{GrantSource, GrantSubject, RoleKey};
use synveda_types::knowledge::{
    KnowledgeCommand, KnowledgeExpectedRevision, KnowledgeLifecycleState, KnowledgeMutationOutcome,
    KnowledgeMutationResult, KnowledgeOrigin, KnowledgeRelationType, KnowledgeRevisionContent,
    KnowledgeSourceDraft, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::operation::OperationState;
use synveda_types::scope::{Scope, ScopeKind};
use synveda_types::{
    Error, GrantId, Identity, IdentityId, IdentityKind, KnowledgeItemId, KnowledgeRevisionId,
    KnowledgeSourceId, ProposalId, ProposalState, ScopeId, Sensitivity, Tenant, TenantId,
    TenantStatus,
};
use tower::ServiceExt;

const SECRET: &[u8] = b"cpr-16-knowledge-lifecycle";

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    LOCK.lock().await
}

fn metrics_handle() -> PrometheusHandle {
    static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| telemetry::init_metrics().expect("install prometheus recorder"))
        .clone()
}

fn state(url: &str) -> AppState {
    AppState {
        pool: PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_lazy(url)
            .expect("parse database url"),
        metrics: metrics_handle(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".to_owned(),
        pdp: Arc::new(synveda_policy::Pdp::new().expect("build embedded PDP")),
        service_token_max_ttl: Duration::from_secs(3600),
        search_index: Arc::new(
            synveda_retrieval::SearchIndex::open(
                std::env::temp_dir()
                    .join("synveda-cpr16-tests")
                    .join(TenantId::new().to_string()),
            )
            .expect("open search sidecar"),
        ),
        embedder: Arc::new(synveda_ingest::embedding::AnyEmbedder::Deterministic(
            synveda_ingest::embedding::DeterministicEmbedder::new(),
        )),
        inject_embed_timeout: Duration::from_millis(100),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    }
}

async fn admitted_tenant() -> Option<(AppState, Tenant)> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping CPR-16 Knowledge lifecycle test: DATABASE_URL is not set \
                 (run `make dev-up` then `make db-test`)"
            );
            return None;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::migrate(&pool)
        .await
        .expect("apply migrations");
    let tenant_id = TenantId::new();
    let slug = format!("cpr16-{}", tenant_id.as_uuid().simple());
    let tenant = synveda_store::tenants::create(
        &pool,
        tenant_id,
        &slug,
        "CPR-16 Knowledge lifecycle",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    Some((state(&url), tenant))
}

async fn seed_user(pool: &PgPool, tenant_id: TenantId, subject: &str) -> Identity {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let own = scopes::ensure_principal_scope(&mut tx, tenant_id, subject, subject)
        .await
        .expect("mint principal scope");
    let identity = identities::create(
        &mut tx,
        IdentityId::new(),
        tenant_id,
        Some(subject),
        IdentityKind::User,
        None,
        Some(subject),
        own.id,
    )
    .await
    .expect("create identity");
    tx.commit().await.expect("commit identity");
    identity
}

async fn seed_workspace(pool: &PgPool, tenant_id: TenantId) -> Scope {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, tenant_id)
        .await
        .expect("mint tenant root");
    let workspace = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id,
            kind: ScopeKind::Workspace,
            parent_scope_id: Some(root.id),
            slug: format!("knowledge-{}", ScopeId::new().as_uuid().simple()),
            display_name: "Knowledge lifecycle".to_owned(),
            attributes: json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .expect("create workspace scope");
    tx.commit().await.expect("commit workspace");
    workspace
}

async fn grant(
    pool: &PgPool,
    tenant_id: TenantId,
    scope_id: ScopeId,
    subject: &str,
    role_key: RoleKey,
) {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    access::create_grant(
        &mut *tx,
        &access::NewGrant {
            id: GrantId::new(),
            tenant_id,
            scope_id,
            subject: GrantSubject::Principal {
                principal_id: subject.to_owned(),
            },
            role_key,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .expect("create grant");
    tx.commit().await.expect("commit grant");
}

async fn use_standard(pool: &PgPool, tenant_id: TenantId) {
    use_policy(pool, tenant_id, synveda_policy::STANDARD).await;
}

async fn use_policy(pool: &PgPool, tenant_id: TenantId, name: &str) {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant tx");
    policy_assignments::set_default(&mut *tx, tenant_id, name)
        .await
        .expect("select policy profile");
    tx.commit().await.expect("commit profile");
}

fn tenant_context(tenant: &Tenant, subject: &str) -> TenantContext {
    TenantContext {
        tenant: tenant.clone(),
        claims: Claims {
            subject: subject.to_owned(),
            tenant_id: tenant.id,
            provisioning: None,
            lifetime: None,
        },
    }
}

async fn command_as(
    state: &AppState,
    tenant: &Tenant,
    subject: &str,
    command: KnowledgeCommand,
) -> synveda_types::Result<synveda_types::knowledge::KnowledgeMutationResult> {
    with_tenant(
        tenant_context(tenant, subject),
        knowledge::command(state, command),
    )
    .await
}

async fn apply_as(
    state: &AppState,
    tenant: &Tenant,
    subject: &str,
    change_id: ProposalId,
) -> synveda_types::Result<synveda_types::knowledge::KnowledgeMutationResult> {
    with_tenant(
        tenant_context(tenant, subject),
        knowledge::apply_reviewed(state, change_id),
    )
    .await
}

async fn result_as(
    state: &AppState,
    tenant: &Tenant,
    subject: &str,
    change_id: ProposalId,
) -> synveda_types::knowledge::KnowledgeMutationResult {
    with_tenant(
        tenant_context(tenant, subject),
        knowledge::result(state, change_id),
    )
    .await
    .expect("read Knowledge change result")
}

fn content(title: &str, body: &str, metadata: Value) -> KnowledgeRevisionContent {
    KnowledgeRevisionContent {
        title: title.to_owned(),
        body_markdown: body.to_owned(),
        summary: body.to_owned(),
        tags: vec!["knowledge".to_owned(), "test".to_owned()],
        sensitivity: Sensitivity::Internal,
        confidence_permille: 925,
        valid_from: Utc::now(),
        valid_to: None,
        stale_after: None,
        verification_metadata: json!({}),
        metadata,
    }
}

async fn api(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: &str,
    idempotency_key: Option<&str>,
    payload: Option<&Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    if let Some(key) = idempotency_key {
        request = request.header("idempotency-key", key);
    }
    let body = if let Some(payload) = payload {
        request = request.header("content-type", "application/json");
        Body::from(serde_json::to_vec(payload).expect("encode API request"))
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).expect("build API request"))
        .await
        .expect("Knowledge API route responds");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read API response");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("decode API response")
    };
    (status, headers, value)
}

fn api_content(title: &str, body: &str) -> Value {
    json!({
        "title": title,
        "body_markdown": body,
        "summary": body,
        "tags": ["PulseBoard", "Delivery"],
        "sensitivity": "internal",
        "confidence_permille": 940,
        "verification_metadata": {},
        "metadata": {"fixture": "CPR-17"}
    })
}

fn source(scope_id: ScopeId) -> KnowledgeSourceDraft {
    KnowledgeSourceDraft {
        id: KnowledgeSourceId::new(),
        scope_id,
        source_type: KnowledgeSourceType::Manual,
        session_event_id: None,
        locator: None,
        source_revision: None,
        content_hash: None,
        metadata: json!({"fixture": "CPR-16"}),
    }
}

fn create_command(
    scope_id: ScopeId,
    owner: Option<&str>,
    title: &str,
    body: &str,
    metadata: Value,
) -> (
    KnowledgeCommand,
    KnowledgeItemId,
    KnowledgeRevisionId,
    KnowledgeSourceId,
) {
    let item_id = KnowledgeItemId::new();
    let revision_id = KnowledgeRevisionId::new();
    let provenance = source(scope_id);
    let source_id = provenance.id;
    (
        KnowledgeCommand::Create {
            item_id,
            scope_id,
            project_id: None,
            owner_principal_id: owner.map(str::to_owned),
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Authored,
            revision_id,
            content: content(title, body, metadata),
            sources: vec![provenance],
        },
        item_id,
        revision_id,
        source_id,
    )
}

async fn snapshot(
    pool: &PgPool,
    tenant_id: TenantId,
    item_id: KnowledgeItemId,
) -> Option<stored::KnowledgeSnapshot> {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant read");
    let value = stored::current(&mut *tx, tenant_id, item_id)
        .await
        .expect("read current Knowledge");
    tx.commit().await.expect("commit tenant read");
    value
}

async fn proposal_state(pool: &PgPool, tenant_id: TenantId, id: ProposalId) -> ProposalState {
    let mut tx = rls::begin_tenant_tx(pool, tenant_id)
        .await
        .expect("begin tenant read");
    let proposal = synveda_vedaflow::proposals::read(&mut tx, tenant_id, id)
        .await
        .expect("read proposal")
        .expect("proposal exists");
    tx.commit().await.expect("commit tenant read");
    proposal.state
}

#[tokio::test]
async fn personal_changes_auto_apply_but_still_create_vedaflow_history() {
    let _guard = serial().await;
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    use_standard(&state.pool, tenant.id).await;
    let alice = seed_user(&state.pool, tenant.id, "alice@pulseboard.test").await;
    let outsider = seed_user(&state.pool, tenant.id, "mallory@pulseboard.test").await;

    let (create, item_id, revision_1, _) = create_command(
        alice.scope_id,
        alice.subject.as_deref(),
        "Request correlation",
        "Public requests use X-Request-Id.",
        json!({}),
    );
    let created = command_as(&state, &tenant, "alice@pulseboard.test", create)
        .await
        .expect("create Knowledge");
    assert_eq!(created.outcome, KnowledgeMutationOutcome::Applied);
    assert_eq!(created.knowledge_item_id, Some(item_id));
    assert_eq!(created.revision_id, Some(revision_1));
    assert_eq!(
        proposal_state(&state.pool, tenant.id, created.change_id).await,
        ProposalState::Applied,
        "auto-apply still closes a real VedaFlow change"
    );

    let revision_2 = KnowledgeRevisionId::new();
    let edited = command_as(
        &state,
        &tenant,
        "alice@pulseboard.test",
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id: revision_1,
            revision_id: revision_2,
            content: content(
                "Request correlation",
                "Public requests use traceparent.",
                json!({}),
            ),
            sources: vec![source(alice.scope_id)],
        },
    )
    .await
    .expect("edit Knowledge");
    assert_eq!(edited.outcome, KnowledgeMutationOutcome::Applied);

    let stale = command_as(
        &state,
        &tenant,
        "alice@pulseboard.test",
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id: revision_1,
            revision_id: KnowledgeRevisionId::new(),
            content: content("Stale write", "This must not land.", json!({})),
            sources: vec![source(alice.scope_id)],
        },
    )
    .await
    .expect("stale revision is a governed rejection");
    assert_eq!(stale.outcome, KnowledgeMutationOutcome::Rejected);
    assert_eq!(
        proposal_state(&state.pool, tenant.id, stale.change_id).await,
        ProposalState::Rejected
    );

    let denied = command_as(
        &state,
        &tenant,
        outsider.subject.as_deref().expect("outsider subject"),
        KnowledgeCommand::Verify {
            item_id,
            expected_revision_id: revision_2,
            revision_id: KnowledgeRevisionId::new(),
            verification_metadata: json!({"method": "forged"}),
        },
    )
    .await
    .expect_err("another principal cannot mutate private Knowledge");
    assert!(matches!(denied, Error::PolicyDenied { .. }));

    let revision_3 = KnowledgeRevisionId::new();
    command_as(
        &state,
        &tenant,
        "alice@pulseboard.test",
        KnowledgeCommand::Verify {
            item_id,
            expected_revision_id: revision_2,
            revision_id: revision_3,
            verification_metadata: json!({"method": "repository-check", "passed": true}),
        },
    )
    .await
    .expect("verify Knowledge");

    command_as(
        &state,
        &tenant,
        "alice@pulseboard.test",
        KnowledgeCommand::Archive {
            item_id,
            expected_revision_id: revision_3,
            reason: "temporarily retired".to_owned(),
        },
    )
    .await
    .expect("archive Knowledge");
    assert_eq!(
        snapshot(&state.pool, tenant.id, item_id)
            .await
            .expect("archived item")
            .item
            .lifecycle_state,
        KnowledgeLifecycleState::Archived
    );
    command_as(
        &state,
        &tenant,
        "alice@pulseboard.test",
        KnowledgeCommand::Restore {
            item_id,
            expected_revision_id: revision_3,
            reason: "valid again".to_owned(),
        },
    )
    .await
    .expect("restore Knowledge");

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id)
        .await
        .expect("begin history read");
    let revisions = stored::revisions(&mut *tx, tenant.id, item_id)
        .await
        .expect("read immutable revisions");
    tx.commit().await.expect("commit history read");
    assert_eq!(revisions.len(), 3);
    assert_eq!(
        revisions[0].content.body_markdown,
        "Public requests use X-Request-Id."
    );
    assert_eq!(
        revisions[1].content.body_markdown,
        "Public requests use traceparent."
    );
    assert_eq!(
        revisions[2].content.body_markdown,
        "Public requests use traceparent."
    );
    assert_eq!(
        snapshot(&state.pool, tenant.id, item_id)
            .await
            .expect("restored item")
            .item
            .lifecycle_state,
        KnowledgeLifecycleState::Active
    );
}

#[tokio::test]
async fn supersession_and_merge_are_explicit_and_retain_every_source() {
    let _guard = serial().await;
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    use_standard(&state.pool, tenant.id).await;
    let alice = seed_user(&state.pool, tenant.id, "alice-merge@pulseboard.test").await;
    let subject = alice.subject.as_deref().expect("subject");

    let (first, first_id, first_revision, _) = create_command(
        alice.scope_id,
        Some(subject),
        "Old correlation convention",
        "Use X-Request-Id.",
        json!({}),
    );
    command_as(&state, &tenant, subject, first)
        .await
        .expect("create first");

    let replacement_id = KnowledgeItemId::new();
    let replacement_revision = KnowledgeRevisionId::new();
    command_as(
        &state,
        &tenant,
        subject,
        KnowledgeCommand::Supersede {
            item_id: first_id,
            expected_revision_id: first_revision,
            replacement_item_id: replacement_id,
            replacement_revision_id: replacement_revision,
            scope_id: alice.scope_id,
            project_id: None,
            owner_principal_id: Some(subject.to_owned()),
            knowledge_type: KnowledgeType::Convention,
            origin: KnowledgeOrigin::Authored,
            content: content(
                "Current correlation convention",
                "Use traceparent.",
                json!({}),
            ),
            sources: vec![source(alice.scope_id)],
        },
    )
    .await
    .expect("supersede Knowledge");
    assert_eq!(
        snapshot(&state.pool, tenant.id, first_id)
            .await
            .expect("superseded source")
            .item
            .lifecycle_state,
        KnowledgeLifecycleState::Superseded
    );

    let (second, second_id, second_revision, _) = create_command(
        alice.scope_id,
        Some(subject),
        "Webhook identity",
        "Deduplicate by provider event ID.",
        json!({}),
    );
    let (third, third_id, third_revision, _) = create_command(
        alice.scope_id,
        Some(subject),
        "Webhook retries",
        "Retries preserve the provider event ID.",
        json!({}),
    );
    command_as(&state, &tenant, subject, second)
        .await
        .expect("create second");
    command_as(&state, &tenant, subject, third)
        .await
        .expect("create third");

    let merged_id = KnowledgeItemId::new();
    let merged_revision = KnowledgeRevisionId::new();
    command_as(
        &state,
        &tenant,
        subject,
        KnowledgeCommand::Merge {
            inputs: vec![
                KnowledgeExpectedRevision {
                    item_id: second_id,
                    revision_id: second_revision,
                },
                KnowledgeExpectedRevision {
                    item_id: third_id,
                    revision_id: third_revision,
                },
            ],
            result_item_id: merged_id,
            result_revision_id: merged_revision,
            scope_id: alice.scope_id,
            project_id: None,
            owner_principal_id: Some(subject.to_owned()),
            knowledge_type: KnowledgeType::Procedure,
            origin: KnowledgeOrigin::Authored,
            content: content(
                "Webhook delivery",
                "Deduplicate and retry by provider event ID.",
                json!({}),
            ),
            sources: Vec::new(),
        },
    )
    .await
    .expect("merge Knowledge");

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id)
        .await
        .expect("begin relation read");
    let replacement_relations = stored::relations(&mut *tx, tenant.id, replacement_id)
        .await
        .expect("read supersession relation");
    let merge_relations = stored::relations(&mut *tx, tenant.id, merged_id)
        .await
        .expect("read merge relations");
    let merged_sources =
        stored::visible_sources(&mut *tx, tenant.id, merged_revision, &[alice.scope_id])
            .await
            .expect("read merged provenance");
    tx.commit().await.expect("commit relation read");
    assert!(replacement_relations.iter().any(|relation| {
        relation.source_item_id == replacement_id
            && relation.target_item_id == first_id
            && relation.relation_type == KnowledgeRelationType::Supersedes
    }));
    assert_eq!(
        merge_relations
            .iter()
            .filter(|relation| relation.relation_type == KnowledgeRelationType::DerivedFrom)
            .count(),
        2
    );
    assert_eq!(
        merged_sources.len(),
        2,
        "merge retains both source descriptors"
    );
    for input in [second_id, third_id] {
        assert_eq!(
            snapshot(&state.pool, tenant.id, input)
                .await
                .expect("merge input retained")
                .item
                .lifecycle_state,
            KnowledgeLifecycleState::Superseded
        );
    }
}

#[tokio::test]
async fn review_is_live_and_forget_leaves_only_content_free_evidence() {
    let _guard = serial().await;
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    let workspace = seed_workspace(&state.pool, tenant.id).await;
    seed_user(&state.pool, tenant.id, "member@pulseboard.test").await;
    seed_user(&state.pool, tenant.id, "curator@pulseboard.test").await;
    grant(
        &state.pool,
        tenant.id,
        workspace.id,
        "member@pulseboard.test",
        RoleKey::Member,
    )
    .await;
    grant(
        &state.pool,
        tenant.id,
        workspace.id,
        "curator@pulseboard.test",
        RoleKey::Curator,
    )
    .await;

    let (shared, shared_id, shared_revision, _) = create_command(
        workspace.id,
        None,
        "Shared convention",
        "Provider event IDs are idempotency keys.",
        json!({}),
    );
    let pending = command_as(&state, &tenant, "member@pulseboard.test", shared)
        .await
        .expect("open reviewed change");
    assert_eq!(pending.outcome, KnowledgeMutationOutcome::PendingReview);
    assert!(snapshot(&state.pool, tenant.id, shared_id).await.is_none());

    let app = router(state.clone());
    let token = Hs256Verifier::new(SECRET).issue(
        "curator@pulseboard.test",
        tenant.id,
        Duration::from_secs(300),
    );
    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1/proposals/{}", pending.change_id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build proposal detail request"),
        )
        .await
        .expect("proposal detail route responds");
    assert_eq!(detail.status(), StatusCode::OK);
    let detail: Value = serde_json::from_slice(
        &to_bytes(detail.into_body(), usize::MAX)
            .await
            .expect("read proposal detail"),
    )
    .expect("decode proposal detail");
    assert_eq!(detail["asset"], "knowledge");
    assert_eq!(detail["members"][0]["effect"], "apply");
    assert!(
        detail["members"][0]["proposed"]
            .as_str()
            .unwrap_or_default()
            .contains("Provider event IDs"),
        "an authorised reviewer sees the exact typed payload: {detail}"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/proposals/{}/approve", pending.change_id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("build approval request"),
        )
        .await
        .expect("approval route responds");
    assert_eq!(response.status(), StatusCode::OK);
    let member_token = Hs256Verifier::new(SECRET).issue(
        "member@pulseboard.test",
        tenant.id,
        Duration::from_secs(300),
    );
    let applied = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/proposals/{}/apply", pending.change_id))
                .header("authorization", format!("Bearer {member_token}"))
                .body(Body::empty())
                .expect("build Knowledge apply request"),
        )
        .await
        .expect("Knowledge apply route responds");
    assert_eq!(applied.status(), StatusCode::OK);
    let applied: KnowledgeMutationResult = serde_json::from_slice(
        &to_bytes(applied.into_body(), usize::MAX)
            .await
            .expect("read Knowledge apply response"),
    )
    .expect("decode Knowledge apply result");
    assert_eq!(applied.outcome, KnowledgeMutationOutcome::Applied);
    assert!(snapshot(&state.pool, tenant.id, shared_id).await.is_some());

    let reviewed_revision = KnowledgeRevisionId::new();
    let reviewed_edit = command_as(
        &state,
        &tenant,
        "member@pulseboard.test",
        KnowledgeCommand::Edit {
            item_id: shared_id,
            expected_revision_id: shared_revision,
            revision_id: reviewed_revision,
            content: content(
                "Shared convention",
                "The reviewed edit must not overwrite a newer head.",
                json!({}),
            ),
            sources: vec![source(workspace.id)],
        },
    )
    .await
    .expect("open reviewed edit");
    assert_eq!(
        reviewed_edit.outcome,
        KnowledgeMutationOutcome::PendingReview
    );

    use_standard(&state.pool, tenant.id).await;
    let newer_revision = KnowledgeRevisionId::new();
    command_as(
        &state,
        &tenant,
        "member@pulseboard.test",
        KnowledgeCommand::Edit {
            item_id: shared_id,
            expected_revision_id: shared_revision,
            revision_id: newer_revision,
            content: content(
                "Shared convention",
                "A newer governed edit won the revision race.",
                json!({}),
            ),
            sources: vec![source(workspace.id)],
        },
    )
    .await
    .expect("apply newer edit under live standard profile");
    let rejected = apply_as(
        &state,
        &tenant,
        "member@pulseboard.test",
        reviewed_edit.change_id,
    )
    .await
    .expect("reviewed stale edit closes as rejected");
    assert_eq!(rejected.outcome, KnowledgeMutationOutcome::Rejected);
    assert_eq!(
        snapshot(&state.pool, tenant.id, shared_id)
            .await
            .expect("shared item remains")
            .revision
            .id,
        newer_revision
    );

    let alice = seed_user(&state.pool, tenant.id, "alice-forget@pulseboard.test").await;
    let subject = alice.subject.as_deref().expect("subject");
    let sentinel = "ERASE-ME-CPR16-PLAINTEXT";
    let (erasable, item_id, revision_id, source_id) = create_command(
        alice.scope_id,
        Some(subject),
        "Disposable secret",
        sentinel,
        json!({}),
    );
    command_as(&state, &tenant, subject, erasable)
        .await
        .expect("create erasable Knowledge");
    use_policy(&state.pool, tenant.id, synveda_policy::REGULATED_STRICT).await;
    let pending_sentinel = "ERASE-ME-CPR16-PENDING-COMMAND";
    let invalidated = command_as(
        &state,
        &tenant,
        subject,
        KnowledgeCommand::Edit {
            item_id,
            expected_revision_id: revision_id,
            revision_id: KnowledgeRevisionId::new(),
            content: content("Pending disposable edit", pending_sentinel, json!({})),
            sources: vec![source(alice.scope_id)],
        },
    )
    .await
    .expect("open pending edit before erasure");
    assert_eq!(invalidated.outcome, KnowledgeMutationOutcome::PendingReview);
    use_standard(&state.pool, tenant.id).await;
    let forgotten = command_as(
        &state,
        &tenant,
        subject,
        KnowledgeCommand::Forget {
            item_id,
            expected_revision_id: revision_id,
            reason: "the user requested erasure".to_owned(),
        },
    )
    .await
    .expect("forget Knowledge");
    assert_eq!(forgotten.outcome, KnowledgeMutationOutcome::Applied);
    assert_eq!(forgotten.knowledge_item_id, Some(item_id));
    let operation_id = forgotten.operation_id.expect("erasure operation");
    assert!(snapshot(&state.pool, tenant.id, item_id).await.is_none());

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id)
        .await
        .expect("begin erasure evidence read");
    let operation =
        synveda_store::knowledge_lifecycle::read_operation(&mut *tx, tenant.id, operation_id)
            .await
            .expect("read erasure operation")
            .expect("operation exists");
    let source_count = sqlx::query_scalar!(
        r#"select count(*) as "count!" from knowledge_sources
           where tenant_id = $1 and id = $2"#,
        tenant.id.as_uuid(),
        source_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count erased source");
    let tombstones = sqlx::query_scalar!(
        r#"select count(*) as "count!" from knowledge_erasure_tombstones
           where tenant_id = $1 and knowledge_item_id = $2"#,
        tenant.id.as_uuid(),
        item_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count tombstone");
    let invalidations = sqlx::query_scalar!(
        r#"select count(*) as "count!" from knowledge_index_invalidations
           where tenant_id = $1 and operation_id = $2"#,
        tenant.id.as_uuid(),
        operation_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count invalidations");
    let retained_payloads = sqlx::query_scalar!(
        r#"select coalesce(string_agg(payload::text, ' '), '') as "payloads!"
           from knowledge_changes where tenant_id = $1"#,
        tenant.id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read retained change payloads");
    tx.commit().await.expect("commit erasure evidence read");
    assert_eq!(operation.state, OperationState::Succeeded);
    assert_eq!(source_count, 0);
    assert_eq!(tombstones, 1);
    assert_eq!(invalidations, 1);
    assert!(!retained_payloads.contains(sentinel));
    assert!(!retained_payloads.contains(pending_sentinel));
    assert_eq!(
        result_as(&state, &tenant, subject, invalidated.change_id)
            .await
            .outcome,
        KnowledgeMutationOutcome::Rejected,
        "erasure closes a pending effect before clearing its payload"
    );
    assert_eq!(
        result_as(&state, &tenant, subject, forgotten.change_id)
            .await
            .operation_id,
        Some(operation_id)
    );

    let foreign_tenant_id = TenantId::new();
    tenants::create(
        &state.pool,
        foreign_tenant_id,
        &format!("cpr16-foreign-{}", foreign_tenant_id.as_uuid().simple()),
        "CPR-16 foreign RLS probe",
        TenantStatus::Active,
    )
    .await
    .expect("create foreign tenant");
    let mut foreign = rls::begin_tenant_tx(&state.pool, foreign_tenant_id)
        .await
        .expect("begin foreign tenant transaction");
    sqlx::raw_sql("set local role synveda_app")
        .execute(&mut *foreign)
        .await
        .expect("demote RLS probe to app role");
    let hidden = sqlx::query!(
        r#"
        select
          (select count(*) from knowledge_changes where tenant_id = $1) as "changes!",
          (select count(*) from durable_operations where tenant_id = $1) as "operations!",
          (select count(*) from knowledge_erasure_tombstones where tenant_id = $1) as "tombstones!",
          (select count(*) from knowledge_index_invalidations where tenant_id = $1) as "invalidations!"
        "#,
        tenant.id.as_uuid(),
    )
    .fetch_one(&mut *foreign)
    .await
    .expect("probe foreign lifecycle rows");
    assert_eq!(
        (
            hidden.changes,
            hidden.operations,
            hidden.tombstones,
            hidden.invalidations,
        ),
        (0, 0, 0, 0),
        "forced RLS hides the complete lifecycle and erasure trail"
    );
    foreign.rollback().await.expect("roll back foreign probe");

    let (held, held_id, held_revision, _) = create_command(
        alice.scope_id,
        Some(subject),
        "Held record",
        "This content remains while the legal hold is active.",
        json!({"legal_hold": true}),
    );
    command_as(&state, &tenant, subject, held)
        .await
        .expect("create held Knowledge");
    let blocked = command_as(
        &state,
        &tenant,
        subject,
        KnowledgeCommand::Forget {
            item_id: held_id,
            expected_revision_id: held_revision,
            reason: "requested while held".to_owned(),
        },
    )
    .await
    .expect("hold is a governed rejection result");
    assert_eq!(blocked.outcome, KnowledgeMutationOutcome::Rejected);
    let blocked_operation = blocked.operation_id.expect("blocked operation");
    let rendered = result_as(&state, &tenant, subject, blocked.change_id).await;
    assert_eq!(rendered.outcome, KnowledgeMutationOutcome::Rejected);
    assert_eq!(rendered.operation_id, Some(blocked_operation));
    assert!(snapshot(&state.pool, tenant.id, held_id).await.is_some());
    assert_eq!(
        proposal_state(&state.pool, tenant.id, blocked.change_id).await,
        ProposalState::Rejected
    );

    let mut tx = rls::begin_tenant_tx(&state.pool, tenant.id)
        .await
        .expect("begin audit read");
    let operation =
        synveda_store::knowledge_lifecycle::read_operation(&mut *tx, tenant.id, blocked_operation)
            .await
            .expect("read blocked operation")
            .expect("blocked operation exists");
    let audit = synveda_audit::tail(&mut tx, tenant.id, 500)
        .await
        .expect("read audit chain");
    tx.commit().await.expect("commit audit read");
    assert_eq!(operation.state, OperationState::Blocked);
    let audit_text = audit
        .iter()
        .map(|event| {
            format!(
                "{} {} {} {}",
                event.actor_subject, event.action, event.resource, event.payload
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !audit_text.contains(sentinel),
        "ordinary audit evidence must retain hashes and IDs, not erased content"
    );
}

/// CPR-17's public acceptance seam. This deliberately enters mutations only
/// through HTTP and proves that listing/search, immutable detail, provenance,
/// erasure and the revision-vector sidecar all stay on the Knowledge model.
#[tokio::test]
async fn public_knowledge_api_is_current_governed_paginated_and_tenant_safe() {
    let _guard = serial().await;
    let Some((state, tenant)) = admitted_tenant().await else {
        return;
    };
    use_standard(&state.pool, tenant.id).await;
    let workspace = seed_workspace(&state.pool, tenant.id).await;
    let alice = seed_user(&state.pool, tenant.id, "alice-api@pulseboard.test").await;
    let _bob = seed_user(&state.pool, tenant.id, "bob-api@pulseboard.test").await;
    for subject in ["alice-api@pulseboard.test", "bob-api@pulseboard.test"] {
        grant(
            &state.pool,
            tenant.id,
            workspace.id,
            subject,
            RoleKey::Member,
        )
        .await;
    }

    let app = router(state.clone());
    let verifier = Hs256Verifier::new(SECRET);
    let alice_token = verifier.issue(
        "alice-api@pulseboard.test",
        tenant.id,
        Duration::from_secs(300),
    );
    let bob_token = verifier.issue(
        "bob-api@pulseboard.test",
        tenant.id,
        Duration::from_secs(300),
    );
    let removed_route = api(
        &app,
        Method::POST,
        &format!("/v1/proposals/{}/classify", ProposalId::new()),
        &alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(removed_route.0, StatusCode::NOT_FOUND);
    let removed_payload = json!({
        "scope_id": workspace.id,
        "record_ids": [KnowledgeItemId::new()],
        "title": "removed raw-record proposal"
    });
    let refused_record_proposal = api(
        &app,
        Method::POST,
        "/v1/proposals",
        &alice_token,
        None,
        Some(&removed_payload),
    )
    .await;
    assert_eq!(refused_record_proposal.0, StatusCode::BAD_REQUEST);
    let refused_effect_alias = api(
        &app,
        Method::POST,
        "/v1/proposals",
        &alice_token,
        None,
        Some(&json!({
            "scope_id": workspace.id,
            "prompt_names": ["removed-effect"],
            "title": "removed effect field",
            "effect": "classify"
        })),
    )
    .await;
    assert_eq!(refused_effect_alias.0, StatusCode::BAD_REQUEST);
    let refused_record_channel = api(
        &app,
        Method::POST,
        &format!("/v1/channels/{}/publish", workspace.id),
        &alice_token,
        None,
        Some(&json!({
            "record_ids": [KnowledgeItemId::new()],
            "message": "removed direct record publication"
        })),
    )
    .await;
    assert_eq!(refused_record_channel.0, StatusCode::BAD_REQUEST);
    let refused_memory_history = api(
        &app,
        Method::GET,
        &format!(
            "/v1/channels/{}/history?asset=memory&channel=published",
            workspace.id
        ),
        &alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(refused_memory_history.0, StatusCode::BAD_REQUEST);

    let shared_body = json!({
        "scope_id": workspace.id,
        "knowledge_type": "convention",
        "origin": "authored",
        "content": api_content(
            "Webhook delivery identity",
            "Webhook deliveries are deduplicated by provider event ID."
        ),
        "sources": [
            {"scope_id": workspace.id, "source_type": "manual", "metadata": {"kind": "team-note"}},
            {"scope_id": alice.scope_id, "source_type": "manual", "metadata": {"kind": "private-draft"}}
        ]
    });
    let (created_status, _, created) = api(
        &app,
        Method::POST,
        "/v1/knowledge",
        &alice_token,
        Some("cpr17-create-shared"),
        Some(&shared_body),
    )
    .await;
    assert_eq!(created_status, StatusCode::CREATED, "{created}");
    assert_eq!(created["outcome"], "applied");
    let shared_id: KnowledgeItemId = created["knowledge_item_id"]
        .as_str()
        .expect("created item id")
        .parse()
        .expect("parse item id");
    let first_revision: KnowledgeRevisionId = created["revision_id"]
        .as_str()
        .expect("created revision id")
        .parse()
        .expect("parse revision id");

    let (replay_status, _, replay) = api(
        &app,
        Method::POST,
        "/v1/knowledge",
        &alice_token,
        Some("cpr17-create-shared"),
        Some(&shared_body),
    )
    .await;
    assert_eq!(replay_status, StatusCode::OK, "{replay}");
    assert_eq!(replay["change_id"], created["change_id"]);
    assert_eq!(replay["knowledge_item_id"], created["knowledge_item_id"]);

    let (detail_status, detail_headers, detail) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{shared_id}"),
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(detail_status, StatusCode::OK, "{detail}");
    assert_eq!(detail["current_revision"]["id"], first_revision.to_string());
    assert_eq!(
        detail_headers
            .get("etag")
            .expect("revision ETag")
            .to_str()
            .expect("ETag text"),
        format!("\"{first_revision}\"")
    );

    let (_, _, sources) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{shared_id}/sources"),
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(
        sources["sources"].as_array().expect("source array").len(),
        1,
        "Bob may read the shared descriptor but not Alice's personal source: {sources}"
    );
    assert_eq!(sources["sources"][0]["scope_id"], workspace.id.to_string());

    let edit_body = json!({
        "expected_revision_id": first_revision,
        "content": api_content(
            "Webhook delivery identity",
            "Provider event ID is the webhook delivery idempotency key."
        )
    });
    let (edit_status, _, edited) = api(
        &app,
        Method::PATCH,
        &format!("/v1/knowledge/{shared_id}"),
        &alice_token,
        Some("cpr17-edit-shared"),
        Some(&edit_body),
    )
    .await;
    assert_eq!(edit_status, StatusCode::CREATED, "{edited}");
    let second_revision: KnowledgeRevisionId = edited["revision_id"]
        .as_str()
        .expect("edited revision id")
        .parse()
        .expect("parse edited revision");

    let (history_status, _, history) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{shared_id}/history?limit=1"),
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(history_status, StatusCode::OK, "{history}");
    assert_eq!(history["revisions"][0]["id"], second_revision.to_string());
    let history_cursor = history["next_cursor"]
        .as_str()
        .expect("second history page");
    let (_, _, older) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{shared_id}/history?limit=1&cursor={history_cursor}"),
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(older["revisions"][0]["id"], first_revision.to_string());

    let (search_status, _, search) = api(
        &app,
        Method::GET,
        "/v1/knowledge?query=provider%20event%20id&tag=pulseboard&source=manual&limit=1",
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(search_status, StatusCode::OK, "{search}");
    assert_eq!(search["retrieval_mode"], "lexical");
    assert_eq!(
        search["degradation"],
        "deterministic_embedder_is_not_semantic"
    );
    assert_eq!(search["items"][0]["id"], shared_id.to_string());

    let private_body = json!({
        "scope_id": alice.scope_id,
        "owner_principal_id": "alice-api@pulseboard.test",
        "knowledge_type": "preference",
        "content": api_content("Quick test", "My local quick-test command is just test-fast.")
    });
    let (private_status, _, private_created) = api(
        &app,
        Method::POST,
        "/v1/knowledge",
        &alice_token,
        Some("cpr17-create-private"),
        Some(&private_body),
    )
    .await;
    assert_eq!(private_status, StatusCode::CREATED, "{private_created}");
    let private_id: KnowledgeItemId = private_created["knowledge_item_id"]
        .as_str()
        .expect("private item")
        .parse()
        .expect("parse private item");
    let private_revision: KnowledgeRevisionId = private_created["revision_id"]
        .as_str()
        .expect("private revision")
        .parse()
        .expect("parse private revision");
    let (_, _, hidden) = api(
        &app,
        Method::GET,
        "/v1/knowledge?query=quick-test",
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(
        hidden["items"],
        json!([]),
        "private Knowledge leaked: {hidden}"
    );

    let delete_without_mode = api(
        &app,
        Method::DELETE,
        &format!("/v1/knowledge/{shared_id}"),
        &alice_token,
        Some("cpr17-delete-without-mode"),
        None,
    )
    .await;
    assert_eq!(delete_without_mode.0, StatusCode::BAD_REQUEST);
    let archive_body = json!({
        "mode": "archive",
        "expected_revision_id": second_revision,
        "reason": "acceptance archive"
    });
    let (archive_status, _, archived) = api(
        &app,
        Method::DELETE,
        &format!("/v1/knowledge/{shared_id}"),
        &alice_token,
        Some("cpr17-archive-shared"),
        Some(&archive_body),
    )
    .await;
    assert_eq!(archive_status, StatusCode::CREATED, "{archived}");
    let (_, _, active_search) = api(
        &app,
        Method::GET,
        "/v1/knowledge?query=provider%20event%20id",
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(active_search["items"], json!([]));
    let (_, _, archived_search) = api(
        &app,
        Method::GET,
        "/v1/knowledge?query=provider%20event%20id&lifecycle_state=archived",
        &bob_token,
        None,
        None,
    )
    .await;
    assert_eq!(archived_search["items"][0]["id"], shared_id.to_string());
    let restore_body = json!({
        "expected_revision_id": second_revision,
        "reason": "acceptance restore"
    });
    let (restore_status, _, restored) = api(
        &app,
        Method::POST,
        &format!("/v1/knowledge/{shared_id}/restore"),
        &alice_token,
        Some("cpr17-restore-shared"),
        Some(&restore_body),
    )
    .await;
    assert_eq!(restore_status, StatusCode::CREATED, "{restored}");

    let embed = synveda_gateway::knowledge_index::sweep_tenant(
        &state.pool,
        state.embedder.as_ref(),
        tenant.id,
        64,
    )
    .await
    .expect("index immutable Knowledge revisions");
    assert!(
        embed.inserted >= 3,
        "every new revision gets a sidecar: {embed:?}"
    );
    let query_vector = state
        .embedder
        .embed(&["webhook provider event id".to_owned()])
        .await
        .expect("embed semantic test query")
        .pop()
        .expect("one query vector");
    let mut search_tx = rls::begin_tenant_tx(&state.pool, tenant.id)
        .await
        .expect("begin semantic read");
    let semantic = knowledge_search::semantic_candidates(
        &mut search_tx,
        tenant.id,
        &knowledge_search::Filters {
            workspace_id: None,
            project_id: None,
            scope_id: Some(workspace.id),
            owner_principal_id: None,
            knowledge_type: None,
            origin: None,
            lifecycle: None,
            tag: None,
            source_type: None,
            updated_from: None,
            updated_before: None,
            stale: None,
            at: Utc::now(),
        },
        state.embedder.model(),
        &query_vector,
        10,
    )
    .await
    .expect("run Knowledge semantic candidate query");
    search_tx.commit().await.expect("commit semantic read");
    assert!(
        semantic
            .iter()
            .any(|candidate| candidate.item_id == shared_id)
    );

    let forget_body = json!({
        "mode": "forget",
        "expected_revision_id": private_revision,
        "reason": "acceptance erasure"
    });
    let (forget_status, _, forgotten) = api(
        &app,
        Method::DELETE,
        &format!("/v1/knowledge/{private_id}"),
        &alice_token,
        Some("cpr17-forget-private"),
        Some(&forget_body),
    )
    .await;
    assert_eq!(forget_status, StatusCode::CREATED, "{forgotten}");
    let (gone_status, _, _) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{private_id}"),
        &alice_token,
        None,
        None,
    )
    .await;
    assert_eq!(gone_status, StatusCode::NOT_FOUND);

    let foreign_id = TenantId::new();
    let foreign = tenants::create(
        &state.pool,
        foreign_id,
        &format!("cpr17-foreign-{}", foreign_id.as_uuid().simple()),
        "CPR-17 foreign API tenant",
        TenantStatus::Active,
    )
    .await
    .expect("create foreign tenant");
    use_standard(&state.pool, foreign.id).await;
    seed_user(&state.pool, foreign.id, "foreign-api@pulseboard.test").await;
    let foreign_token = verifier.issue(
        "foreign-api@pulseboard.test",
        foreign.id,
        Duration::from_secs(300),
    );
    let (foreign_status, _, foreign_body) = api(
        &app,
        Method::GET,
        &format!("/v1/knowledge/{shared_id}"),
        &foreign_token,
        None,
        None,
    )
    .await;
    assert_eq!(foreign_status, StatusCode::NOT_FOUND, "{foreign_body}");
    let mut foreign_tx = rls::begin_tenant_tx(&state.pool, foreign.id)
        .await
        .expect("begin foreign embedding probe");
    sqlx::raw_sql("set local role synveda_app")
        .execute(&mut *foreign_tx)
        .await
        .expect("demote foreign probe");
    let leaked_embeddings = sqlx::query_scalar!(
        r#"select count(*) as "count!" from knowledge_revision_embeddings
           where tenant_id = $1"#,
        tenant.id.as_uuid(),
    )
    .fetch_one(&mut *foreign_tx)
    .await
    .expect("probe foreign embeddings");
    assert_eq!(leaked_embeddings, 0, "forced RLS hides revision vectors");
    foreign_tx
        .rollback()
        .await
        .expect("roll back foreign probe");
}

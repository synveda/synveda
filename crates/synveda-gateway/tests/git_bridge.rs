//! FLOW-8: ordinary tenant transactions, fresh Cedar, durable receipts and Git.
#![cfg(unix)]

#[path = "support/configuration.rs"]
mod configuration_support;
#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use http_body_util::BodyExt;
use metrics_exporter_prometheus::PrometheusHandle;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, postgres::PgPoolOptions};
use synveda_gateway::app::{AppState, behavior_test_router};
use synveda_git::{
    CommitEvidence, EntryEvidence, LocalTransport, ObjectEvidence, Projection, Snapshot, Transport,
    TreeEvidence, verify_repository,
};
use synveda_identity::Hs256Verifier;
use synveda_ingest::embedding::{AnyEmbedder, DeterministicEmbedder};
use synveda_policy::Pdp;
use synveda_store::{access, identities, scopes};
use synveda_types::{
    AssetKind, Channel, GrantId, IdentityId, IdentityKind, PackConfig, PackDocument,
    PromptTemplate, ScopeId, Sensitivity, TenantId, TenantStatus,
    access::{GrantSource, GrantSubject, RoleKey},
    scope::ScopeKind,
};
use synveda_vedaflow::{self as flow, ChannelRef, CommitHash, PolicySnapshot, Signer, TreeEntry};
use tower::ServiceExt;

const SECRET: &[u8] = b"flow-8-test-secret";
const CONTENT: &str = "FLOW-8 source content must stay out of audit and responses.";

struct World {
    pool: PgPool,
    tenant: TenantId,
    scope: ScopeId,
    actor: IdentityId,
    grant: GrantId,
    app: Router,
    state: AppState,
    pdp: Arc<Pdp>,
    transport: Arc<LocalTransport>,
    _root: tempfile::TempDir,
}

async fn world() -> Option<World> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "skipping FLOW-8 database acceptance: DATABASE_URL is unset; run demos/flow-8-git-export.sh"
        );
        return None;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    synveda_store::epoch::verify(&pool).await.unwrap();
    let tenant = TenantId::new();
    tenant_fixture::create(
        &pool,
        tenant,
        &format!("flow8-{}", tenant.as_uuid().simple()),
        "FLOW-8",
        TenantStatus::Active,
    )
    .await
    .unwrap();
    let mut tx = tenant_fixture::begin(&pool, tenant).await;
    let scope = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .unwrap()
        .id;
    let own = scopes::ensure_principal_scope(&mut tx, tenant, "admin", "Admin")
        .await
        .unwrap();
    let actor = IdentityId::new();
    identities::create(
        &mut tx,
        actor,
        tenant,
        Some("admin"),
        IdentityKind::User,
        None,
        None,
        own.id,
    )
    .await
    .unwrap();
    let grant = GrantId::new();
    access::create_grant(
        &mut tx,
        &access::NewGrant {
            id: grant,
            tenant_id: tenant,
            scope_id: scope,
            subject: GrantSubject::Principal {
                principal_id: "admin".into(),
            },
            role_key: RoleKey::Administrator,
            source: GrantSource::Automation,
            invite_id: None,
            granted_by: None,
        },
    )
    .await
    .unwrap();
    configuration_support::bind_pack(&mut tx, tenant, scope, "standard").await;
    configuration_support::set_git_export_targets(&mut tx, tenant, scope, vec!["review".into()])
        .await;
    tx.commit().await.unwrap();
    let root = tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let transport = Arc::new(LocalTransport::new(root.path().to_owned()).unwrap());
    let pdp = Arc::new(Pdp::new().unwrap());
    static METRICS: OnceLock<PrometheusHandle> = OnceLock::new();
    let state = AppState {
        pool: pool.clone(),
        metrics: METRICS
            .get_or_init(|| synveda_gateway::telemetry::init_metrics().unwrap())
            .clone(),
        verifier: Arc::new(Hs256Verifier::new(SECRET)),
        login: None,
        public_origin: "http://127.0.0.1:8120".into(),
        pdp: pdp.clone(),
        service_token_max_ttl: Duration::from_secs(3600),
        embedder: Arc::new(AnyEmbedder::Deterministic(DeterministicEmbedder::new())),
        context_embed_timeout: Duration::from_millis(100),
        git_exports: Some(Arc::new(synveda_git::ExportTransport::new(
            Some((*transport).clone()),
            None,
        ))),
        keys: Arc::new(synveda_store::keys::KeyRing::new(
            synveda_crypto::Kms::Disabled,
        )),
    };
    let app = behavior_test_router(state.clone());
    Some(World {
        pool,
        tenant,
        scope,
        actor,
        grant,
        app,
        state,
        pdp,
        transport,
        _root: root,
    })
}

async fn source_commit(
    w: &World,
    asset: AssetKind,
    parents: Vec<CommitHash>,
    content: &str,
    object_scope: ScopeId,
) -> CommitHash {
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    let (name, bytes) = match asset {
        AssetKind::Prompt => {
            let object = flow::PromptAsset {
                scope_id: object_scope,
                sensitivity: Sensitivity::Internal,
                template: PromptTemplate {
                    name: "review/reply".parse().unwrap(),
                    description: "Fixture".into(),
                    template: content.into(),
                    variables: vec![],
                },
            };
            (object.entry_name(), object.canonical_bytes())
        }
        AssetKind::ContextPack => {
            let object = flow::ContextPackAsset {
                scope_id: object_scope,
                sensitivity: Sensitivity::Internal,
                pack: "review".parse().unwrap(),
                document: PackDocument {
                    name: "guide".parse().unwrap(),
                    title: "Fixture".into(),
                    content: content.into(),
                },
            };
            (object.entry_name(), object.canonical_bytes())
        }
        _ => unreachable!(),
    };
    let object = flow::put_object(&mut tx, w.tenant, asset, &bytes)
        .await
        .unwrap();
    let tree = flow::put_tree(&mut tx, w.tenant, &[TreeEntry::object(name, object.hash)])
        .await
        .unwrap();
    let receipt = flow::commit(
        &mut tx,
        w.tenant,
        &flow::NewCommit {
            tree: tree.hash,
            parents,
            author: w.actor,
            message: "FLOW-8 source fixture".into(),
            committed_at: Utc::now(),
            policy_snapshot: PolicySnapshot::new("standard", 1),
        },
        &Signer::Unsigned,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    receipt.hash
}

async fn set_head(w: &World, asset: AssetKind, channel: Channel, head: CommitHash) {
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    let name = ChannelRef::new(asset, channel).name();
    let previous = flow::read_ref(&mut tx, w.tenant, w.scope, &name)
        .await
        .unwrap();
    let result = match previous {
        Some(previous) => flow::force_update_ref(
            &mut tx,
            w.tenant,
            w.scope,
            &name,
            previous.commit_hash,
            head,
            w.actor,
        )
        .await
        .unwrap(),
        None => flow::create_ref(&mut tx, w.tenant, w.scope, &name, head, w.actor)
            .await
            .unwrap(),
    };
    assert_eq!(result, flow::RefUpdate::Updated);
    tx.commit().await.unwrap();
}

async fn call(
    w: &World,
    tenant: TenantId,
    asset: AssetKind,
    channel: Channel,
    target: &str,
) -> (StatusCode, Value) {
    let token = Hs256Verifier::new(SECRET).issue("admin", tenant, Duration::from_secs(300));
    let response = w
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/channels/{}/git-export", w.scope))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"asset":asset,"channel":channel,"target":target}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains(CONTENT));
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn cursor(tx: &mut PgConnection, w: &World, asset: AssetKind) -> Value {
    let reference = flow::read_ref(
        tx,
        w.tenant,
        w.scope,
        &format!("git-export/review/{asset}/published"),
    )
    .await
    .unwrap()
    .unwrap();
    let commit = flow::read_commit(tx, w.tenant, reference.commit_hash)
        .await
        .unwrap()
        .unwrap();
    let tree = flow::read_tree(tx, w.tenant, commit.tree)
        .await
        .unwrap()
        .unwrap();
    let flow::TreeTarget::Object(hash) = tree[0].target else {
        panic!("cursor blob")
    };
    let object = flow::read_object(tx, w.tenant, hash)
        .await
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&object.content).contains(CONTENT));
    serde_json::from_slice(&object.content).unwrap()
}

#[tokio::test]
async fn export_clone_verify_replay_rollback_and_content_free_audit() {
    let Some(w) = world().await else { return };
    for asset in [AssetKind::Prompt, AssetKind::ContextPack] {
        let root = source_commit(&w, asset, vec![], CONTENT, w.scope).await;
        let left = source_commit(&w, asset, vec![root], CONTENT, w.scope).await;
        let right = source_commit(&w, asset, vec![root], "Reviewed source edit", w.scope).await;
        let merge =
            source_commit(&w, asset, vec![right, left], "Merged source edit", w.scope).await;
        set_head(&w, asset, Channel::Published, merge).await;
        let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
        flow::pin(
            &mut tx,
            w.tenant,
            w.scope,
            ChannelRef::new(asset, Channel::Published),
            root,
            w.actor,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let (status, result) = call(&w, w.tenant, asset, Channel::Published, "review").await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["outcome"], "completed");
        assert_eq!(result["provider"], "local");
        assert!(result["repository_id"].is_null());
        assert_eq!(result["destination_digest"].as_str().unwrap().len(), 64);
        assert_eq!(result["commits"], 4);
        assert_eq!(result["source_pin"], root.to_hex());
        let repo = w.transport.repository(w.tenant, w.scope, "review").unwrap();
        let reference = result["git_ref"].as_str().unwrap();
        let manifest = verify_repository(&repo, reference, &BTreeMap::new())
            .await
            .unwrap();
        let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
        assert!(matches!(
            synveda_store::git_exports::read_commit(&mut tx, w.tenant, merge.as_slice(), 1).await,
            Err(synveda_types::Error::Invalid { .. })
        ));
        let merge_evidence =
            synveda_store::git_exports::read_commit(&mut tx, w.tenant, merge.as_slice(), 128)
                .await
                .unwrap()
                .unwrap();
        assert!(matches!(
            synveda_store::git_exports::read_tree(&mut tx, w.tenant, &merge_evidence.tree_hash, 0)
                .await,
            Err(synveda_types::Error::Invalid { .. })
        ));
        let bounded_members =
            synveda_store::git_exports::read_tree(&mut tx, w.tenant, &merge_evidence.tree_hash, 1)
                .await
                .unwrap();
        assert!(matches!(
            synveda_store::git_exports::read_object(
                &mut tx,
                w.tenant,
                bounded_members[0].object_hash.as_ref().unwrap(),
                0
            )
            .await,
            Err(synveda_types::Error::Invalid { .. })
        ));
        for evidence in &manifest.snapshot.commits {
            let stored = flow::read_commit(&mut tx, w.tenant, evidence.hash.parse().unwrap())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(stored.hash, flow::verify::recompute_commit(&stored));
            assert_eq!(
                evidence.parents,
                stored
                    .parents
                    .iter()
                    .map(|p| p.to_hex())
                    .collect::<Vec<_>>()
            );
        }
        let receipt = cursor(&mut tx, &w, asset).await;
        assert_eq!(receipt["phase"], "completed");
        assert_eq!(receipt["destination_digest"], result["destination_digest"]);
        assert_eq!(receipt["export_states"], 1);
        assert_eq!(
            receipt["manifest_bytes"],
            serde_json::to_vec(&manifest).unwrap().len()
        );
        tx.commit().await.unwrap();
        let clone = w._root.path().join(format!("clone-{asset}"));
        let output = tokio::process::Command::new("git")
            .args(["clone", "--quiet", "--no-local", "--branch"])
            .arg(reference.strip_prefix("refs/heads/").unwrap())
            .arg(&repo)
            .arg(&clone)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            verify_repository(&clone, reference, &BTreeMap::new())
                .await
                .unwrap()
                .snapshot
                .head,
            merge.to_hex()
        );
        let (status, replay) = call(&w, w.tenant, asset, Channel::Published, "review").await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay["outcome"], "no_op");
        assert_eq!(replay["git_head"], result["git_head"]);
        set_head(&w, asset, Channel::Published, root).await;
        let (status, rollback) = call(&w, w.tenant, asset, Channel::Published, "review").await;
        assert_eq!(status, StatusCode::OK, "{rollback}");
        assert_eq!(rollback["source_head"], root.to_hex());
        assert_eq!(
            verify_repository(&repo, reference, &BTreeMap::new())
                .await
                .unwrap()
                .previous_state
                .as_deref(),
            result["git_head"].as_str()
        );
    }
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    let events = synveda_audit::search(
        &mut tx,
        w.tenant,
        &synveda_audit::EventFilter {
            actions: vec![synveda_audit::AuditAction::GitExported],
            ..Default::default()
        },
        0,
        100,
    )
    .await
    .unwrap();
    assert!(!events.items.is_empty());
    for event in events.items {
        assert_eq!(event.actor_subject, "admin");
        let payload = event.payload.to_string();
        assert!(!payload.contains(CONTENT));
        assert!(!payload.contains(w._root.path().to_str().unwrap()));
        assert_eq!(event.payload["target_id"], "review");
        assert_eq!(event.payload["provider"], "local");
        assert!(event.payload["secret_id"].is_null());
        assert!(event.payload["mapping_digest"].as_str().unwrap().len() == 64);
    }
    tx.commit().await.unwrap();
}

fn select_remote(w: &mut World, secret_id: synveda_types::TenantSecretId) {
    let target = synveda_git::GitHubTarget {
        tenant_id: w.tenant,
        scope_id: w.scope,
        target: "review".into(),
        owner: "fixture-unconfigured".into(),
        repository: "export".into(),
        repository_id: 123,
        secret_reference: synveda_types::secret::tenant_secret_reference(secret_id),
    };
    w.state.git_exports = Some(Arc::new(synveda_git::ExportTransport::new(
        Some((*w.transport).clone()),
        Some(synveda_git::GitHubTransport::new(vec![target]).unwrap()),
    )));
    w.app = behavior_test_router(w.state.clone());
}

#[tokio::test]
async fn github_disclosure_requires_current_provider_and_exact_active_sealed_secret() {
    use synveda_crypto::{KeyScope, Purpose, RowKey};
    use synveda_types::{TenantSecretId, secret::TenantSecretKind};
    let Some(mut w) = world().await else { return };
    let head = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, w.scope).await;
    set_head(&w, AssetKind::Prompt, Channel::Published, head).await;
    let missing = TenantSecretId::new();
    select_remote(&mut w, missing);
    let (status, disabled) = call(
        &w,
        w.tenant,
        AssetKind::Prompt,
        Channel::Published,
        "review",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        disabled
            .to_string()
            .contains("disabled by effective governed Configuration")
    );
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    configuration_support::set_github_exports(&mut tx, w.tenant, w.scope, true).await;
    tx.commit().await.unwrap();
    let (status, absent) = call(
        &w,
        w.tenant,
        AssetKind::Prompt,
        Channel::Published,
        "review",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        absent
            .to_string()
            .contains("destination or credential is unavailable")
    );
    let ring = Arc::new(synveda_store::keys::KeyRing::new(
        synveda_crypto::Kms::Local(
            synveda_crypto::LocalKms::from_hex(&"42".repeat(32), "local:flow8").unwrap(),
        ),
    ));
    ring.provision(&w.pool, KeyScope::Tenant(w.tenant))
        .await
        .unwrap();
    w.state.keys = ring.clone();
    let key = ring
        .sealing_key(&w.pool, KeyScope::Tenant(w.tenant))
        .await
        .unwrap();
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    let own = scopes::ensure_principal_scope(&mut tx, w.tenant, "admin", "Admin")
        .await
        .unwrap()
        .id;
    tx.commit().await.unwrap();
    // Every candidate fails before HTTP. All are actual sealed/store values;
    // no live provider or real credential participates in database acceptance.
    for case in [
        "wrong_kind",
        "wrong_scope",
        "wrong_provider",
        "wrong_repository",
        "wrong_aad",
        "corrupt",
        "revoked",
    ] {
        let id = TenantSecretId::new();
        let document =
            serde_json::to_vec(&json!({ "format": "synveda-github-export-credential-v1",
            "repository_id": if case == "wrong_repository" { 124 } else { 123 },
            "token": "github_pat_fixture_source_secret" }))
            .unwrap();
        let aad = if case == "wrong_aad" {
            TenantSecretId::new()
        } else {
            id
        };
        let mut sealed = key
            .seal(
                Purpose::TenantSecret,
                RowKey::Uuid(aad.as_uuid()),
                &document,
            )
            .unwrap();
        if case == "corrupt" {
            *sealed.last_mut().unwrap() ^= 1;
        }
        let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
        synveda_store::tenant_secrets::put(
            &mut tx,
            id,
            w.tenant,
            if case == "wrong_scope" { own } else { w.scope },
            if case == "wrong_kind" {
                TenantSecretKind::ToolServer
            } else {
                TenantSecretKind::ImportExport
            },
            &format!("flow8.{case}").replace('_', "."),
            Some(if case == "wrong_provider" {
                "gitlab"
            } else {
                "github"
            }),
            key.version(),
            &sealed,
        )
        .await
        .unwrap();
        if case == "revoked" {
            synveda_store::tenant_secrets::revoke(&mut tx, w.tenant, id)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        select_remote(&mut w, id);
        let (status, failure) = call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {failure}");
        assert_eq!(
            failure, absent,
            "credential failures must not reveal reference availability"
        );
        assert!(!failure.to_string().contains("source_secret"));
    }
    let Some(foreign) = world().await else {
        panic!("same database fixture")
    };
    ring.provision(&foreign.pool, KeyScope::Tenant(foreign.tenant))
        .await
        .unwrap();
    let foreign_key = ring
        .sealing_key(&foreign.pool, KeyScope::Tenant(foreign.tenant))
        .await
        .unwrap();
    let foreign_id = TenantSecretId::new();
    let sealed = foreign_key
        .seal(
            Purpose::TenantSecret,
            RowKey::Uuid(foreign_id.as_uuid()),
            b"foreign source-secret",
        )
        .unwrap();
    let mut tx = tenant_fixture::begin(&foreign.pool, foreign.tenant).await;
    synveda_store::tenant_secrets::put(
        &mut tx,
        foreign_id,
        foreign.tenant,
        foreign.scope,
        TenantSecretKind::ImportExport,
        "flow8.foreign",
        Some("github"),
        foreign_key.version(),
        &sealed,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    select_remote(&mut w, foreign_id);
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await,
        (StatusCode::BAD_REQUEST, absent)
    );
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    assert!(
        flow::read_ref(
            &mut tx,
            w.tenant,
            w.scope,
            "git-export/review/prompt/published"
        )
        .await
        .unwrap()
        .is_none()
    );
    access::revoke_grant(&mut tx, w.tenant, w.grant)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &w,
            foreign.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        !w.transport
            .repository(w.tenant, w.scope, "review")
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn changing_destination_custody_cannot_retarget_a_completed_receipt() {
    let Some(mut w) = world().await else { return };
    let head = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, w.scope).await;
    set_head(&w, AssetKind::Prompt, Channel::Published, head).await;
    let (status, completed) = call(
        &w,
        w.tenant,
        AssetKind::Prompt,
        Channel::Published,
        "review",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    configuration_support::set_github_exports(&mut tx, w.tenant, w.scope, true).await;
    tx.commit().await.unwrap();
    select_remote(&mut w, synveda_types::TenantSecretId::new());
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let repo = w.transport.repository(w.tenant, w.scope, "review").unwrap();
    let manifest = verify_repository(
        &repo,
        completed["git_ref"].as_str().unwrap(),
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(manifest.snapshot.head, head.to_hex());
}

#[tokio::test]
async fn prepared_receipt_resumes_frozen_source_and_rechecks_configuration() {
    let Some(w) = world().await else { return };
    let first = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, w.scope).await;
    set_head(&w, AssetKind::Prompt, Channel::Published, first).await;
    let repo = w.transport.repository(w.tenant, w.scope, "review").unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    for path in [
        &repo,
        repo.parent().unwrap(),
        repo.parent().unwrap().parent().unwrap(),
    ] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(repo.join("retained"), "keep").unwrap();
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    assert_eq!(
        cursor(&mut tx, &w, AssetKind::Prompt).await["phase"],
        "prepared"
    );
    configuration_support::set_git_export_targets(&mut tx, w.tenant, w.scope, vec![]).await;
    tx.commit().await.unwrap();
    std::fs::remove_file(repo.join("retained")).unwrap();
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert!(!repo.join("HEAD").exists());
    let next = source_commit(
        &w,
        AssetKind::Prompt,
        vec![first],
        "Later source edit",
        w.scope,
    )
    .await;
    set_head(&w, AssetKind::Prompt, Channel::Published, next).await;
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    configuration_support::set_git_export_targets(
        &mut tx,
        w.tenant,
        w.scope,
        vec!["review".into()],
    )
    .await;
    let pending = cursor(&mut tx, &w, AssetKind::Prompt).await;
    let commit = flow::read_commit(&mut tx, w.tenant, first)
        .await
        .unwrap()
        .unwrap();
    let members = flow::read_tree(&mut tx, w.tenant, commit.tree)
        .await
        .unwrap()
        .unwrap();
    let flow::TreeTarget::Object(hash) = members[0].target else {
        panic!("source object")
    };
    let object = flow::read_object(&mut tx, w.tenant, hash)
        .await
        .unwrap()
        .unwrap();
    let snapshot = Snapshot {
        tenant: w.tenant,
        scope: w.scope,
        target: "review".into(),
        asset: AssetKind::Prompt,
        channel: Channel::Published,
        head: first.to_hex(),
        updated_at: serde_json::from_value(pending["source"]["updated_at"].clone()).unwrap(),
        updated_by: w.actor,
        pin: None,
        commits: vec![CommitEvidence::from_stored(&commit)],
        trees: vec![TreeEvidence {
            hash: commit.tree.to_hex(),
            entries: vec![EntryEvidence {
                name: members[0].name.clone(),
                object: hash.to_hex(),
            }],
        }],
        objects: vec![ObjectEvidence {
            hash: hash.to_hex(),
            kind: object.kind,
            content: String::from_utf8(object.content).unwrap(),
        }],
    };
    tx.commit().await.unwrap();
    let prepared_projection = Projection::render(snapshot, None).unwrap();
    assert_eq!(
        prepared_projection.head,
        pending["git_head"].as_str().unwrap()
    );
    // Simulate a crash after Git succeeds but before the prepared receipt is
    // completed. The next HTTP request must finish that same exact export.
    assert!(
        w.transport
            .advance(&prepared_projection, None)
            .await
            .unwrap()
    );
    let (status, resumed) = call(
        &w,
        w.tenant,
        AssetKind::Prompt,
        Channel::Published,
        "review",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(resumed["outcome"], "resumed");
    assert_eq!(resumed["source_head"], first.to_hex());
    let (status, advanced) = call(
        &w,
        w.tenant,
        AssetKind::Prompt,
        Channel::Published,
        "review",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{advanced}");
    assert_eq!(advanced["source_head"], next.to_hex());
    // Reconstruct a prepared receipt after the native effect, as a lost DB
    // completion would see it. The transport recognizes the exact new head.
    let manifest = verify_repository(
        &repo,
        advanced["git_ref"].as_str().unwrap(),
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    let projection =
        Projection::render(manifest.snapshot, manifest.previous_state.clone()).unwrap();
    assert!(
        !w.transport
            .advance(&projection, manifest.previous_state.as_deref())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn fresh_cedar_and_rls_refuse_revoked_and_foreign_history() {
    let Some(w) = world().await else { return };
    let foreign = world().await.unwrap();
    let head = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, w.scope).await;
    set_head(&w, AssetKind::Prompt, Channel::Published, head).await;
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "unconfigured"
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &w,
            foreign.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    access::revoke_grant(&mut tx, w.tenant, w.grant)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut foreign = tenant_fixture::begin(&w.pool, foreign.tenant).await;
    assert!(
        synveda_store::git_exports::read_commit(&mut foreign, w.tenant, head.as_slice(), 128)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        flow::read_ref(
            &mut foreign,
            w.tenant,
            w.scope,
            "git-export/review/prompt/published"
        )
        .await
        .unwrap()
        .is_none()
    );
    foreign.commit().await.unwrap();
}

#[tokio::test]
async fn every_historical_asset_uses_its_original_scope_and_current_policy() {
    let Some(w) = world().await else { return };
    let mut tx = tenant_fixture::begin(&w.pool, w.tenant).await;
    let own = scopes::ensure_principal_scope(&mut tx, w.tenant, "other", "Other")
        .await
        .unwrap();
    let child = scopes::create(
        &mut tx,
        &scopes::NewScope {
            id: ScopeId::new(),
            tenant_id: w.tenant,
            kind: ScopeKind::OrgUnit,
            parent_scope_id: Some(w.scope),
            slug: "denied-source".into(),
            display_name: "Denied source".into(),
            attributes: json!({}),
            principal_id: None,
            created_by: None,
        },
    )
    .await
    .unwrap();
    let source = "permit(principal, action, resource); forbid(principal, action == Synveda::Action::\"PromptRead\", resource);";
    w.pdp
        .install_source(w.tenant, "flow8-deny", 1, source, PackConfig::default())
        .unwrap();
    configuration_support::bind_pack(&mut tx, w.tenant, child.id, "flow8-deny").await;
    tx.commit().await.unwrap();
    let private = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, own.id).await;
    let allowed_head = source_commit(
        &w,
        AssetKind::Prompt,
        vec![private],
        "Visible current head",
        w.scope,
    )
    .await;
    set_head(&w, AssetKind::Prompt, Channel::Published, allowed_head).await;
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let denied = source_commit(&w, AssetKind::Prompt, vec![], CONTENT, child.id).await;
    set_head(&w, AssetKind::Prompt, Channel::Published, denied).await;
    assert_eq!(
        call(
            &w,
            w.tenant,
            AssetKind::Prompt,
            Channel::Published,
            "review"
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert!(
        !w.transport
            .repository(w.tenant, w.scope, "review")
            .unwrap()
            .exists()
    );
}

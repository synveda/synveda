//! OPS-11 retained pgvector transition evidence (ADR-0153).
//!
//! `scripts/db-test.sh` seeds one populated installation on the legacy native
//! package, replaces the package on the same data directory and verifies the
//! same content-free receipt at every transition boundary. Each case runs only
//! through that fixture; they are ignored in an ordinary test run.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_audit::{Actor, AuditAction, AuditEvent, ChainVerification, Outcome};
use synveda_crypto::{KeyScope, Kms, LocalKms, Purpose, RowKey};
use synveda_store::keys::KeyRing;
use synveda_store::knowledge::{self, NewKnowledgeItem, NewKnowledgeRevision, NewKnowledgeSource};
use synveda_store::{epoch, knowledge_search, rls, runtime_role, scopes, tenant_secrets};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::secret::TenantSecretKind;
use synveda_types::{
    KnowledgeItemId, KnowledgeRevisionId, KnowledgeSourceId, ScopeId, Sensitivity, TenantId,
    TenantSecretId, TenantStatus,
};

const ITEMS: usize = 12;
const DIMENSIONS: [usize; 2] = [16, 1024];
const SECRET_LABEL: &str = "fixture.vector-transition";
const SECRET_PLAINTEXT: &[u8] = b"vector transition sealed fixture payload";
const MAX_PRIVATE_FILE_BYTES: u64 = 64 * 1024;
/// SHA-256 of the extension fingerprint shipped before the 0.8.7 profiles.
/// Adding or removing a profile row changes the splice below; update both.
const PRIOR_FINGERPRINT_SHA256: &str =
    "4bbeeffe18609bf72b7d9527f73fe790b8f0ccc396654d89d6402b26ea0a732b";
const CURRENT_FINGERPRINT: &str = include_str!("../../synveda-store/sql/extension_fingerprint.sql");
const LAST_LEGACY_ROW_END: &str =
    "'5b1552a857b437d8a0c3274d3344feaed14a4033ce0ebcdcef238ea99f84b980')";
const LAST_PATCHED_ROW_END: &str =
    "'7ffe1da9f74811daba20d0923a6233b23f04badee88627630fd21bd4e704f9e0')";

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("the transition fixture requires {name}"))
}

fn private_file(name: &str) -> String {
    let path = setting(name);
    let metadata = std::fs::metadata(&path).expect("inspect a private fixture file");
    assert!(
        metadata.len() <= MAX_PRIVATE_FILE_BYTES,
        "{name} is too large"
    );
    std::fs::read_to_string(&path)
        .expect("read a private fixture file")
        .trim()
        .to_owned()
}

async fn pool_from(label: &str, url: &str) -> PgPool {
    let options = synveda_store::database_url::parse(label, url).expect("parse a fixture URL");
    PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect a fixture pool")
}

async fn gateway_pool() -> PgPool {
    pool_from("DATABASE_URL", &setting("DATABASE_URL")).await
}

async fn migrator_pool() -> PgPool {
    let url = private_file("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE");
    pool_from("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE", &url).await
}

fn roles() -> runtime_role::DatabaseRoles {
    runtime_role::DatabaseRoles::parse_json(&private_file("SYNVEDA_DATABASE_ROLES_FILE"))
        .expect("parse the fixture role contract")
}

fn ring() -> KeyRing {
    let kek = private_file("SYNVEDA_VECTOR_TRANSITION_KEK_FILE");
    KeyRing::new(Kms::Local(
        LocalKms::from_hex(&kek, "local:vector-transition").expect("fixture KEK"),
    ))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
}

/// Two-dimensional structure padded to the indexed width, so cosine order
/// from each probe is strict and identical for exact and HNSW scans.
fn embedding(dimension: usize, first: f32, second: f32) -> Vec<f32> {
    let mut vector = vec![0.0; dimension];
    vector[0] = first;
    vector[1] = second;
    vector
}

fn position(item: usize, revision: usize) -> f32 {
    let base = (item as f32 + 1.0) / 32.0;
    if revision == 2 {
        base + 1.0 / 64.0
    } else {
        base
    }
}

fn model(dimension: usize) -> String {
    format!("vector-transition-{dimension}")
}

fn revision(item: usize, revision: usize) -> NewKnowledgeRevision {
    NewKnowledgeRevision {
        id: KnowledgeRevisionId::new(),
        content: KnowledgeRevisionContent {
            title: format!("Transition fixture {item} revision {revision}"),
            body_markdown: format!("Retained transition fixture item {item}, revision {revision}."),
            summary: format!("Transition fixture {item}"),
            tags: vec!["transition".to_owned()],
            sensitivity: Sensitivity::Internal,
            confidence_permille: 900,
            valid_from: chrono::Utc::now(),
            valid_to: None,
            stale_after: None,
            verification_metadata: serde_json::json!({"method": "fixture"}),
            metadata: serde_json::json!({}),
        },
        created_by: Some("fixture:vector-transition".to_owned()),
    }
}

async fn admit(pool: &PgPool, label: &str) -> (TenantId, ScopeId) {
    let tenant = TenantId::new();
    let slug = format!("{label}-{}", tenant.as_uuid().simple());
    tenant_fixture::create(
        pool,
        tenant,
        &slug,
        "Vector transition fixture",
        TenantStatus::Active,
    )
    .await
    .expect("admit a fixture tenant");
    let mut tx = rls::begin_tenant_write_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let root = scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("tenant root");
    tx.commit().await.expect("commit tenant root");
    (tenant, root.id)
}

async fn create_item(
    pool: &PgPool,
    tenant: TenantId,
    scope: ScopeId,
    index: usize,
) -> KnowledgeItemId {
    let mut tx = rls::begin_tenant_write_tx(pool, tenant)
        .await
        .expect("tenant tx");
    let source = NewKnowledgeSource {
        id: KnowledgeSourceId::new(),
        tenant_id: tenant,
        scope_id: scope,
        source_type: KnowledgeSourceType::Manual,
        session_event_id: None,
        import_artifact_id: None,
        locator: None,
        source_revision: None,
        content_hash: None,
        metadata: serde_json::json!({"method": "fixture"}),
        created_by: Some("fixture:vector-transition".to_owned()),
    };
    knowledge::create_source(&mut tx, &source)
        .await
        .expect("create source");
    let new_item = NewKnowledgeItem {
        id: KnowledgeItemId::new(),
        tenant_id: tenant,
        scope_id: scope,
        project_id: None,
        owner_principal_id: None,
        knowledge_type: KnowledgeType::Fact,
        origin: KnowledgeOrigin::Authored,
        created_by: Some("fixture:vector-transition".to_owned()),
    };
    let first = revision(index, 1);
    knowledge::create_item(&mut tx, &new_item, &first, &[source.id])
        .await
        .expect("create Knowledge item");
    embed(&mut tx, tenant, first.id, position(index, 1)).await;
    if index.is_multiple_of(2) {
        let second = revision(index, 2);
        knowledge::append_revision(
            &mut tx,
            tenant,
            new_item.id,
            first.id,
            &second,
            &[source.id],
        )
        .await
        .expect("append Knowledge revision")
        .expect("item exists");
        embed(&mut tx, tenant, second.id, position(index, 2)).await;
    }
    tx.commit().await.expect("commit Knowledge item");
    new_item.id
}

async fn embed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: TenantId,
    revision: KnowledgeRevisionId,
    second: f32,
) {
    for dimension in DIMENSIONS {
        knowledge_search::insert_embedding(
            tx,
            tenant,
            revision,
            &model(dimension),
            &embedding(dimension, 1.0, second),
        )
        .await
        .expect("insert revision embedding");
    }
}

fn filters(scope: ScopeId) -> knowledge_search::Filters {
    knowledge_search::Filters {
        scope_ids: vec![scope],
        workspace_id: None,
        project_id: None,
        scope_id: None,
        owner_principal_id: None,
        knowledge_type: None,
        origin: None,
        lifecycle: None,
        tag: None,
        source_type: None,
        updated_from: None,
        updated_before: None,
        stale: None,
        at: chrono::Utc::now(),
        as_known_at: None,
        include_history: false,
        include_transitional: false,
    }
}

/// Ordered nearest items from two opposite probes at both indexed widths,
/// plus the lexical match set.
async fn retrieval(pool: &PgPool, tenant: TenantId, scope: ScopeId) -> serde_json::Value {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let filters = filters(scope);
    let mut semantic = serde_json::Map::new();
    for dimension in DIMENSIONS {
        let mut probes = Vec::new();
        for probe in [
            embedding(dimension, 1.0, 0.0),
            embedding(dimension, 0.0, 1.0),
        ] {
            let candidates = knowledge_search::semantic_candidates(
                &mut tx,
                tenant,
                &filters,
                &model(dimension),
                &probe,
                ITEMS as i64,
            )
            .await
            .expect("semantic candidates");
            probes.push(
                candidates
                    .iter()
                    .map(|candidate| candidate.item_id.to_string())
                    .collect::<Vec<_>>(),
            );
        }
        semantic.insert(dimension.to_string(), serde_json::json!(probes));
    }
    let mut lexical: Vec<String> =
        knowledge_search::lexical_candidates(&mut tx, tenant, &filters, "transition", 50)
            .await
            .expect("lexical candidates")
            .iter()
            .map(|candidate| candidate.item_id.to_string())
            .collect();
    lexical.sort();
    tx.commit().await.expect("finish retrieval");
    serde_json::json!({"semantic": semantic, "lexical": lexical})
}

async fn knowledge_state(
    pool: &PgPool,
    tenant: TenantId,
    items: &[KnowledgeItemId],
) -> serde_json::Value {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let mut state = Vec::with_capacity(items.len());
    for &item in items {
        let current = knowledge::current(&mut *tx, tenant, item)
            .await
            .expect("current Knowledge")
            .expect("retained item");
        let revisions: Vec<_> = knowledge::revisions(&mut *tx, tenant, item)
            .await
            .expect("Knowledge revisions")
            .into_iter()
            .map(|revision| serde_json::json!([revision.id.to_string(), revision.content_hash]))
            .collect();
        state.push(serde_json::json!({
            "item": item.to_string(),
            "current": current.revision.id.to_string(),
            "revisions": revisions,
        }));
    }
    tx.commit().await.expect("finish Knowledge read");
    serde_json::json!(state)
}

async fn audit_head(pool: &PgPool, tenant: TenantId) -> serde_json::Value {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let report = synveda_audit::verify_report(&mut tx, tenant)
        .await
        .expect("verify audit chain");
    tx.commit().await.expect("finish audit verification");
    assert!(
        matches!(report.verification, ChainVerification::Valid { .. }),
        "{:?}",
        report.verification
    );
    serde_json::json!({"seq": report.head_seq, "hash": hex(&report.head_hash)})
}

async fn opened_secret_digest(pool: &PgPool, tenant: TenantId, id: TenantSecretId) -> String {
    let mut tx = rls::begin_tenant_tx(pool, tenant).await.expect("tenant tx");
    let stored = tenant_secrets::get(&mut *tx, tenant, id)
        .await
        .expect("read sealed secret")
        .expect("retained sealed secret");
    tx.commit().await.expect("finish sealed read");
    let sealed = stored.sealed.expect("sealed payload");
    let ring = ring();
    let opened = ring
        .opening_key(pool, KeyScope::Tenant(tenant), &sealed)
        .await
        .expect("opening key")
        .open(Purpose::TenantSecret, RowKey::Uuid(id.as_uuid()), &sealed)
        .expect("open the retained sealed secret");
    blake3::hash(&opened).to_hex().to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn prior_fingerprint() -> String {
    let start = CURRENT_FINGERPRINT
        .find(LAST_LEGACY_ROW_END)
        .expect("the last legacy profile row")
        + LAST_LEGACY_ROW_END.len();
    let end = CURRENT_FINGERPRINT
        .find(LAST_PATCHED_ROW_END)
        .expect("the last patched profile row")
        + LAST_PATCHED_ROW_END.len();
    let prior = format!(
        "{}{}",
        &CURRENT_FINGERPRINT[..start],
        &CURRENT_FINGERPRINT[end..]
    );
    assert_eq!(
        hex(&Sha256::digest(prior.as_bytes())),
        PRIOR_FINGERPRINT_SHA256,
        "the derived query must be byte-identical to the prior shipped fingerprint"
    );
    prior
}

async fn extension_facts(pool: &PgPool) -> (String, String) {
    sqlx::query_as(
        "select extension.extversion, available.default_version \
           from pg_catalog.pg_extension extension \
           join pg_catalog.pg_available_extensions available on available.name = extension.extname \
          where extension.extname = 'vector'",
    )
    .fetch_one(pool)
    .await
    .expect("read the vector catalogue and installed package versions")
}

async fn lineage(pool: &PgPool) -> &'static str {
    let versions: Vec<i64> =
        sqlx::query_scalar("select version from public._sqlx_migrations order by version")
            .fetch_all(pool)
            .await
            .expect("read the migration ledger");
    match versions.first() {
        Some(1) => "published",
        Some(11) => "portable",
        _ => panic!("unexpected migration ledger {versions:?}"),
    }
}

/// Recreates the exact published v0.4.6 ledger head (`0010`) in the empty
/// fixture database, so normal migration then converges a published lineage.
#[test]
#[ignore = "run only through the pgvector transition fixture"]
fn install_the_published_v046_head() {
    runtime().block_on(async {
        let pool = migrator_pool().await;
        let mut connection = pool.acquire().await.expect("migrator connection");
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..10].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .expect("install the published 0010 head");
        epoch::stamp_connection(&mut connection, "0.4.6")
            .await
            .expect("stamp the published 0010 head");
        drop(connection);
        pool.close().await;
    });
}

/// Populates a retained installation and writes its content-free receipt.
#[test]
#[ignore = "run only through the pgvector transition fixture"]
fn seed_a_retained_vector_installation() {
    runtime().block_on(async {
        let pool = gateway_pool().await;
        let (tenant, scope) = admit(&pool, "vector-transition").await;
        let mut items = Vec::with_capacity(ITEMS);
        for index in 0..ITEMS {
            items.push(create_item(&pool, tenant, scope, index).await);
        }
        let mut tx = rls::begin_tenant_write_tx(&pool, tenant).await.expect("tenant tx");
        for index in 0..3 {
            let event = AuditEvent {
                occurred_at: chrono::Utc::now(),
                actor: Actor::subject("fixture:vector-transition"),
                action: AuditAction::AuthzDecision,
                resource: format!("scope {scope}"),
                outcome: Outcome::Allow,
                payload: serde_json::json!({"fixture": "vector-transition", "n": index}),
                trace_id: None,
            };
            synveda_audit::append(&mut tx, tenant, &event).await.expect("append audit");
        }
        tx.commit().await.expect("commit audit");

        let ring = ring();
        let key_scope = KeyScope::Tenant(tenant);
        ring.provision(&pool, key_scope).await.expect("provision tenant key");
        let secret = TenantSecretId::new();
        let key = ring.sealing_key(&pool, key_scope).await.expect("sealing key");
        let sealed = key
            .seal(Purpose::TenantSecret, RowKey::Uuid(secret.as_uuid()), SECRET_PLAINTEXT)
            .expect("seal");
        let mut tx = rls::begin_tenant_write_tx(&pool, tenant).await.expect("tenant tx");
        tenant_secrets::put(
            &mut tx,
            secret,
            tenant,
            scope,
            TenantSecretKind::Directory,
            SECRET_LABEL,
            Some("fixture"),
            key.version(),
            &sealed,
        )
        .await
        .expect("store sealed secret");
        tx.commit().await.expect("commit sealed secret");

        let migrator = migrator_pool().await;
        let receipt = serde_json::json!({
            "tenant": tenant.to_string(),
            "scope": scope.to_string(),
            "lineage": lineage(&migrator).await,
            "items": items.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "knowledge": knowledge_state(&pool, tenant, &items).await,
            "retrieval": retrieval(&pool, tenant, scope).await,
            "audit": audit_head(&pool, tenant).await,
            "secret": {"id": secret.to_string(), "digest": blake3::hash(SECRET_PLAINTEXT).to_hex().to_string()},
        });
        let expected_order: Vec<String> = items.iter().map(ToString::to_string).collect();
        assert_eq!(receipt["retrieval"]["semantic"]["16"][0], serde_json::json!(expected_order));
        std::fs::write(
            setting("SYNVEDA_VECTOR_TRANSITION_RECEIPT_FILE"),
            serde_json::to_vec_pretty(&receipt).expect("serialise receipt"),
        )
        .expect("write the content-free receipt");
        migrator.close().await;
        pool.close().await;
    });
}

/// Re-reads the retained installation at one transition boundary and checks
/// the receipt, the boundary's catalogue/package versions and whether the
/// prior shipped extension admission still accepts the catalogue.
#[test]
#[ignore = "run only through the pgvector transition fixture"]
fn verify_a_retained_vector_installation() {
    let expected_catalogue = setting("SYNVEDA_VECTOR_TRANSITION_EXPECT_CATALOGUE");
    let expected_package = setting("SYNVEDA_VECTOR_TRANSITION_EXPECT_PACKAGE");
    let expected_prior = setting("SYNVEDA_VECTOR_TRANSITION_EXPECT_PRIOR") == "admitted";
    let receipt: serde_json::Value =
        serde_json::from_str(&private_file("SYNVEDA_VECTOR_TRANSITION_RECEIPT_FILE"))
            .expect("parse the receipt");
    runtime().block_on(async {
        let pool = gateway_pool().await;
        let migrator = migrator_pool().await;
        let tenant: TenantId = receipt["tenant"].as_str().and_then(|v| v.parse().ok()).expect("tenant");
        let scope: ScopeId = receipt["scope"].as_str().and_then(|v| v.parse().ok()).expect("scope");
        let items: Vec<KnowledgeItemId> = receipt["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item.as_str().and_then(|v| v.parse().ok()).expect("item id"))
            .collect();
        let secret: TenantSecretId =
            receipt["secret"]["id"].as_str().and_then(|v| v.parse().ok()).expect("secret");

        assert_eq!(
            extension_facts(&pool).await,
            (expected_catalogue.clone(), expected_package.clone())
        );
        synveda_store::check_migration_compatibility(&migrator, &roles())
            .await
            .expect("this build admits the retained catalogue");
        let prior_admits: Option<bool> = sqlx::query_scalar(&prior_fingerprint())
            .fetch_one(&pool)
            .await
            .expect("evaluate the prior shipped extension admission");
        assert_eq!(prior_admits == Some(true), expected_prior);

        assert_eq!(lineage(&migrator).await, receipt["lineage"].as_str().expect("lineage"));
        assert_eq!(knowledge_state(&pool, tenant, &items).await, receipt["knowledge"]);
        assert_eq!(retrieval(&pool, tenant, scope).await, receipt["retrieval"]);
        assert_eq!(audit_head(&pool, tenant).await, receipt["audit"]);
        assert_eq!(
            opened_secret_digest(&pool, tenant, secret).await,
            receipt["secret"]["digest"].as_str().expect("secret digest")
        );

        // A new tenant writes and retrieves through the transitioned index
        // and cannot see the retained tenant's Knowledge.
        let (probe, probe_scope) = admit(&pool, "vector-transition-probe").await;
        let written = create_item(&pool, probe, probe_scope, 0).await;
        let mut tx = rls::begin_tenant_tx(&pool, probe).await.expect("probe tx");
        for &item in &items {
            assert!(
                knowledge::current(&mut *tx, tenant, item)
                    .await
                    .expect("cross-tenant read")
                    .is_none(),
                "forced RLS must hide another tenant's Knowledge"
            );
        }
        tx.commit().await.expect("finish isolation probe");
        let probe_retrieval = retrieval(&pool, probe, probe_scope).await;
        for dimension in DIMENSIONS {
            assert_eq!(
                probe_retrieval["semantic"][dimension.to_string()][0],
                serde_json::json!([written.to_string()])
            );
        }
        println!(
            "vector-transition lineage={} catalogue={expected_catalogue} package={expected_package} prior_admission={} items={} audit_seq={}",
            receipt["lineage"].as_str().unwrap_or_default(),
            if expected_prior { "admitted" } else { "refused" },
            items.len(),
            receipt["audit"]["seq"],
        );
        migrator.close().await;
        pool.close().await;
    });
}

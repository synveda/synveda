//! A populated tenant restores exactly into fresh storage (ADR-0157).
//!
//! Runs in the serial lifecycle phase: source and target are scratch
//! databases on the isolated lifecycle cluster. The CLI child exports and
//! imports under the exact migrator; every comparison reads through the
//! ordinary gateway role. Damaged, oversized, wrong-key, wrong-schema and
//! colliding archives are refused without changing the target.

#[path = "../../synveda-store/tests/support/epoch_fixture.rs"]
mod epoch_fixture;

use std::path::Path;
use std::process::{Command, Output};

use epoch_fixture::*;
use sqlx::PgPool;
use synveda_crypto::{KeyScope, Purpose, RowKey};
use synveda_store::tenant_archive as rows;
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::*;

const KEK: &str = "3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c";
const OTHER_KEK: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";
const SECRET_VALUE: &[u8] = b"fictional archive round-trip credential";

fn cli(url: Option<&str>, kek: Option<&str>, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
    command
        .args(arguments)
        .env_remove("DATABASE_URL_FILE")
        .env_remove("SYNVEDA_KMS_KEY_FILE");
    match url {
        Some(url) => command.env("DATABASE_URL", url),
        None => command.env_remove("DATABASE_URL"),
    };
    match kek {
        Some(kek) => command.env("SYNVEDA_KMS_KEY", kek),
        None => command.env_remove("SYNVEDA_KMS_KEY"),
    };
    command.output().expect("run the synveda CLI")
}

fn receipt(output: &Output) -> serde_json::Value {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("JSON receipt")
}

fn refused(output: &Output, reason: &str) {
    assert!(!output.status.success(), "expected {reason}: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(reason), "expected {reason}: {stderr}");
}

fn gateway(scratch: &Scratch) -> sqlx::postgres::PgConnectOptions {
    let url = private_database_url("SYNVEDA_EPOCH_TEST_GATEWAY_DATABASE_URL_FILE")
        .expect("lifecycle gateway URL");
    synveda_store::database_url::parse("lifecycle gateway", &url)
        .expect("gateway URL")
        .database(&scratch.name)
}

async fn seed(pool: &PgPool, tenant: TenantId) -> ScopeId {
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
    seed_knowledge(&mut tx, tenant, project.scope_id, project.id).await;
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
    root.id
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
) {
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
}

/// Every portable table's ordered COPY digest, read as the ordinary role.
async fn digests(pool: &PgPool, tenant: TenantId) -> Vec<(String, u64, [u8; 32])> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .unwrap();
    rows::pin_copy_format(&mut tx).await.unwrap();
    let mut digests = Vec::new();
    for table in rows::portable_tables() {
        let shape = rows::table_shape(&mut tx, table).await.unwrap();
        let (count, digest) = rows::table_digest(&mut tx, &shape, tenant, 1 << 25)
            .await
            .unwrap();
        digests.push((table.name.to_owned(), count, digest));
    }
    tx.rollback().await.unwrap();
    digests
}

async fn audit(
    pool: &PgPool,
    tenant: TenantId,
) -> (synveda_audit::VerificationReport, Vec<String>) {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .unwrap();
    let report = synveda_audit::verify_report(&mut tx, tenant).await.unwrap();
    let mut events = synveda_audit::tail(&mut tx, tenant, 1000).await.unwrap();
    tx.rollback().await.unwrap();
    events.reverse();
    (
        report,
        events.into_iter().map(|event| hex(&event.hash)).collect(),
    )
}

async fn opened_secret(
    pool: &PgPool,
    tenant: TenantId,
    id: TenantSecretId,
    kek: &str,
) -> Option<Vec<u8>> {
    let mut tx = synveda_store::rls::begin_tenant_custody_tx(pool, tenant)
        .await
        .unwrap();
    let stored = synveda_store::tenant_secrets::get(&mut *tx, tenant, id)
        .await
        .unwrap()?;
    tx.rollback().await.unwrap();
    let sealed = stored.sealed?;
    let ring = synveda_store::keys::KeyRing::new(synveda_crypto::Kms::Local(
        synveda_crypto::LocalKms::from_hex(kek, "local:default").unwrap(),
    ));
    let key = ring
        .opening_key(pool, KeyScope::Tenant(tenant), &sealed)
        .await
        .ok()?;
    key.open(Purpose::TenantSecret, RowKey::Uuid(id.as_uuid()), &sealed)
        .ok()
        .map(|opened| opened.to_vec())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A copy of `archive` with `change` applied, for refusal cases.
fn damaged(archive: &Path, name: &str, change: impl FnOnce(&mut Vec<u8>)) -> std::path::PathBuf {
    let mut bytes = std::fs::read(archive).unwrap();
    change(&mut bytes);
    let path = archive.with_file_name(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_populated_tenant_restores_exactly_into_fresh_storage() {
    let Some(server) = server() else { return };
    // The runtime roles may reach one application database at a time, so the
    // source is exported and dropped before fresh storage is created.
    let source = Scratch::empty(&server);
    let scratch_dir =
        std::env::temp_dir().join(format!("synveda-archive-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&scratch_dir).unwrap();

    let (tenant, canary, root, source_gateway) = source.block_on(async {
        let owner = connect_pool(&source.options).await;
        synveda_store::migrate(&owner, &source.roles).await.unwrap();
        let tenant = admit_a_tenant(&owner).await;
        let canary = admit_a_tenant(&owner).await;
        let gateway_pool = connect_pool(&gateway(&source)).await;
        let root = seed(&gateway_pool, tenant).await;
        seed(&gateway_pool, canary).await;
        owner.close().await;
        (tenant, canary, root, gateway_pool)
    });
    let tenant_arg = tenant.to_string();
    let source_url = source.url();
    let source_admin = replace_database(&source.admin_url, &source.name);
    assert!(
        cli(
            Some(&source_url),
            Some(KEK),
            &["tenant", "key", "provision", "--tenant", &tenant_arg]
        )
        .status
        .success()
    );
    let secret_file = scratch_dir.join("secret");
    std::fs::write(&secret_file, SECRET_VALUE).unwrap();
    let secret = receipt(&cli(
        Some(&source_url),
        Some(KEK),
        &[
            "tenant",
            "secret",
            "put",
            "--tenant",
            &tenant_arg,
            "--scope",
            &root.to_string(),
            "--kind",
            "import_export",
            "--label",
            "archive.round-trip",
            "--from",
            secret_file.to_str().unwrap(),
        ],
    ));
    let secret_id: TenantSecretId = secret["id"].as_str().unwrap().parse().unwrap();

    let archive = scratch_dir.join("tenant.svarc");
    let archive_arg = archive.to_str().unwrap();
    let export = [
        "tenant",
        "archive",
        "export",
        "--tenant",
        &tenant_arg,
        "--out",
        archive_arg,
    ];
    refused(
        &cli(Some(&source_admin), Some(OTHER_KEK), &export),
        "key_unavailable",
    );
    refused(
        &cli(
            Some(&source_admin),
            Some(KEK),
            &[&export[..], &["--max-bytes", "1024"]].concat(),
        ),
        "archive_too_large",
    );
    assert!(!archive.exists(), "a refused export publishes nothing");
    refused(
        &cli(Some(&source_url), Some(KEK), &export),
        "configured database administrator",
    );
    let exported = receipt(&cli(Some(&source_admin), Some(KEK), &export));
    refused(
        &cli(Some(&source_admin), Some(KEK), &export),
        "archive_exists",
    );
    let (source_digests, (source_audit, source_events)) = source.block_on(async {
        let administrator = connect_pool(&source.admin.clone().database(&source.name)).await;
        let complete = digests(&administrator, tenant).await;
        administrator.close().await;
        (complete, audit(&source_gateway, tenant).await)
    });
    assert_eq!(
        exported["audit_head_seq"],
        source_audit.head_seq - 1,
        "tenant.archived follows the frozen prefix"
    );
    eprintln!("tenant-archive-roundtrip export {exported}");
    let verified = receipt(&cli(
        None,
        Some(KEK),
        &["tenant", "archive", "verify", "--archive", archive_arg],
    ));
    assert_eq!(verified["manifest_digest"], exported["manifest_digest"]);

    let length = std::fs::metadata(&archive).unwrap().len() as usize;
    for (name, reason, change) in [
        (
            "flipped.svarc",
            "archive_corrupt",
            Box::new(|bytes: &mut Vec<u8>| {
                let at = bytes.len() / 2;
                bytes[at] ^= 1;
            }) as Box<dyn FnOnce(&mut Vec<u8>)>,
        ),
        (
            "truncated.svarc",
            "archive_truncated",
            Box::new(move |bytes: &mut Vec<u8>| bytes.truncate(length - 7)),
        ),
        (
            "appended.svarc",
            "archive_corrupt",
            Box::new(|bytes: &mut Vec<u8>| bytes.extend_from_slice(b"D\0\0\0\x01x")),
        ),
        (
            "header.svarc",
            "archive_corrupt",
            Box::new(|bytes: &mut Vec<u8>| {
                let at = bytes.iter().position(|b| *b == b'{').unwrap() + 2;
                bytes[at] ^= 0x20;
            }),
        ),
    ] {
        let path = damaged(&archive, name, change);
        refused(
            &cli(
                None,
                Some(KEK),
                &[
                    "tenant",
                    "archive",
                    "verify",
                    "--archive",
                    path.to_str().unwrap(),
                ],
            ),
            reason,
        );
    }
    refused(
        &cli(
            None,
            Some(KEK),
            &[
                "tenant",
                "archive",
                "verify",
                "--archive",
                archive_arg,
                "--max-bytes",
                "4096",
            ],
        ),
        "archive_too_large",
    );
    source.block_on(async { source_gateway.close().await });
    drop(source);

    let import = ["tenant", "archive", "import", "--archive", archive_arg];
    let stale = Scratch::empty(&server);
    stale.block_on(async {
        let owner = connect_pool(&stale.options).await;
        let mut connection = owner.acquire().await.unwrap();
        sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(synveda_store::MIGRATOR.migrations[..13].to_vec()),
            ..sqlx::migrate::Migrator::DEFAULT
        }
        .run(&mut *connection)
        .await
        .unwrap();
        synveda_store::epoch::stamp_connection(&mut connection, "0.4.7")
            .await
            .unwrap();
        drop(connection);
        owner.close().await;
    });
    refused(
        &cli(
            Some(&replace_database(&stale.admin_url, &stale.name)),
            Some(KEK),
            &import,
        ),
        "schema_incompatible",
    );
    drop(stale);

    let target = Scratch::empty(&server);
    let target_url = replace_database(&target.admin_url, &target.name);
    let target_gateway = target.block_on(async {
        let owner = connect_pool(&target.options).await;
        synveda_store::migrate(&owner, &target.roles).await.unwrap();
        owner.close().await;
        connect_pool(&gateway(&target)).await
    });
    let flipped = scratch_dir.join("flipped.svarc");
    refused(
        &cli(
            Some(&target_url),
            Some(KEK),
            &[
                "tenant",
                "archive",
                "import",
                "--archive",
                flipped.to_str().unwrap(),
            ],
        ),
        "archive_corrupt",
    );
    refused(
        &cli(Some(&target_url), Some(OTHER_KEK), &import),
        "key_unavailable",
    );
    refused(
        &cli(
            Some(&target_url),
            Some(KEK),
            &[&import[..], &["--writer-drain-seconds", "1"]].concat(),
        ),
        "writers_connected",
    );
    target.block_on(async {
        assert_eq!(
            tenant_count(&target_gateway).await,
            0,
            "refusals write nothing"
        );
        target_gateway.close().await;
    });

    let restored = receipt(&cli(Some(&target_url), Some(KEK), &import));
    assert_eq!(restored["outcome"], "restored");
    eprintln!("tenant-archive-roundtrip restore {restored}");
    assert_eq!(restored["manifest_digest"], exported["manifest_digest"]);
    assert_eq!(
        receipt(&cli(Some(&target_url), Some(KEK), &import))["outcome"],
        "already_restored"
    );

    let target_gateway = target.block_on(async { connect_pool(&gateway(&target)).await });
    target.block_on(async {
        let status = synveda_store::tenants::by_id(&target_gateway, tenant)
            .await
            .unwrap()
            .unwrap()
            .status;
        assert_eq!(
            status,
            TenantStatus::Suspended,
            "a restored tenant waits for an operator"
        );
        assert!(
            synveda_store::tenants::by_id(&target_gateway, canary)
                .await
                .unwrap()
                .is_none()
        );
        let administrator = connect_pool(&target.admin.clone().database(&target.name)).await;
        let restored_digests = digests(&administrator, tenant).await;
        administrator.close().await;
        assert_eq!(restored_digests.len(), source_digests.len());
        for (restored, original) in restored_digests.iter().zip(&source_digests) {
            if restored.0 == "audit_log" || restored.0 == "audit_chain_heads" {
                continue;
            }
            assert_eq!(restored, original, "{} differs after restore", restored.0);
        }
        let (target_audit, target_events) = audit(&target_gateway, tenant).await;
        assert!(matches!(
            target_audit.verification,
            synveda_audit::ChainVerification::Valid { .. }
        ));
        let frozen = usize::try_from(source_audit.head_seq - 1).unwrap();
        assert_eq!(
            &target_events[..frozen],
            &source_events[..frozen],
            "the frozen audit prefix is exact"
        );
        assert_eq!(
            target_events.len(),
            frozen + 1,
            "tenant.restored follows the frozen prefix"
        );
        assert_eq!(
            opened_secret(&target_gateway, tenant, secret_id, KEK)
                .await
                .as_deref(),
            Some(SECRET_VALUE)
        );
        assert!(
            opened_secret(&target_gateway, tenant, secret_id, OTHER_KEK)
                .await
                .is_none()
        );
        let embeddings: i64 =
            sqlx::query_scalar("select count(*) from knowledge_revision_embeddings")
                .fetch_one(
                    &mut *synveda_store::rls::begin_tenant_snapshot_tx(&target_gateway, tenant)
                        .await
                        .unwrap(),
                )
                .await
                .unwrap();
        assert_eq!(
            embeddings, 0,
            "derived embeddings are rebuilt, never archived"
        );
        let searchable: i64 = sqlx::query_scalar(
            "select count(*) from knowledge_revisions \
              where search_document @@ plainto_tsquery('simple', 'retry')",
        )
        .fetch_one(
            &mut *synveda_store::rls::begin_tenant_snapshot_tx(&target_gateway, tenant)
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            searchable, 2,
            "the generated lexical index is recomputed on restore"
        );
    });

    let inventory = receipt(&cli(
        Some(&target.url()),
        None,
        &["tenant", "inventory", "--tenant", &tenant_arg, "--json"],
    ));
    assert_eq!(inventory["status"], "suspended");
    let resumed = receipt(&cli(
        Some(&target.url()),
        Some(KEK),
        &[
            "tenant",
            "resume",
            "--tenant",
            &tenant_arg,
            "--reason",
            "recovery",
        ],
    ));
    assert_eq!(resumed["status"], "active");
    target.block_on(async {
        synveda_store::rls::begin_tenant_tx(&target_gateway, tenant)
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap();
        target_gateway.close().await;
    });
    std::fs::remove_dir_all(&scratch_dir).ok();
}

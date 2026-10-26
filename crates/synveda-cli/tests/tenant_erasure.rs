//! `synveda tenant erase` as separate processes (ADR-0157).
//!
//! A populated tenant and a populated canary share the workspace database.
//! The CLI child erases under the exact test migrator; every observation is
//! made through the ordinary `DATABASE_URL` role. Only the hidden revision is
//! created as the migrator: an unproven-lineage row hides it from every role,
//! so a residual count through any product role could not see it. An
//! interrupted run resumes without counting a row twice, a held revision stops
//! erasure before any deletion, and the canary is unchanged.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;
#[path = "support/tenant_population.rs"]
mod tenant_population;

use std::collections::BTreeMap;
use std::process::{Command, Output};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::tenant_inventory::{self, Erasure, TENANT_TABLES};
use synveda_types::knowledge::{
    KnowledgeOrigin, KnowledgeRevisionContent, KnowledgeSourceType, KnowledgeType,
};
use synveda_types::*;

const KEK: &str = "7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e";

/// Runs the CLI as the exact test migrator.
fn synveda(arguments: &[&str]) -> Output {
    let migrator = std::env::var_os("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE")
        .expect("test migrator URL file");
    Command::new(env!("CARGO_BIN_EXE_synveda"))
        .args(arguments)
        .env_remove("DATABASE_URL")
        .env_remove("SYNVEDA_KMS_KEY_FILE")
        .env("DATABASE_URL_FILE", migrator)
        .env("SYNVEDA_KMS_KEY", KEK)
        .output()
        .expect("run the synveda CLI")
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

fn erase(tenant: &str, extra: &[&str]) -> Output {
    let base = [
        "tenant",
        "erase",
        "--tenant",
        tenant,
        "--reason",
        "offboarding",
        "--force",
        "--fixture-rules",
    ];
    synveda(&[&base[..], extra].concat())
}

fn transition(command: &str, tenant: &str) -> Output {
    synveda(&[
        "tenant",
        command,
        "--tenant",
        tenant,
        "--reason",
        "offboarding",
    ])
}

async fn connect() -> Option<PgPool> {
    let url = tenant_fixture::database_url("CLI tenant erasure test")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("current schema");
    Some(pool)
}

async fn admit(pool: &PgPool, prefix: &str) -> TenantId {
    let id = TenantId::new();
    let slug = format!("{prefix}-{}", id.as_uuid().simple());
    tenant_fixture::create(pool, id, &slug, "CLI erasure", TenantStatus::Active)
        .await
        .expect("admit tenant");
    id
}

async fn tenant_row(pool: &PgPool, tenant: TenantId) -> Tenant {
    synveda_store::tenants::by_id(pool, tenant)
        .await
        .expect("read tenant")
        .expect("tenant exists")
}

/// Rows per tenant table as the ordinary role reads them.
async fn inventory(pool: &PgPool, tenant: TenantId) -> BTreeMap<&'static str, u64> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .expect("snapshot");
    let counts = tenant_inventory::count_tenant_rows(&mut tx, tenant, 1_000_000)
        .await
        .expect("count tenant rows");
    tx.rollback().await.expect("end snapshot");
    counts
        .into_iter()
        .map(|count| (count.table, count.rows))
        .collect()
}

async fn lifecycle_events(pool: &PgPool, tenant: TenantId) -> Vec<(String, serde_json::Value)> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .expect("snapshot");
    let verified = synveda_audit::verify(&mut tx, tenant)
        .await
        .expect("verify the retained chain");
    assert!(
        matches!(verified, synveda_audit::ChainVerification::Valid { .. }),
        "the retained audit chain verifies: {verified:?}"
    );
    let mut events = synveda_audit::tail(&mut tx, tenant, 100)
        .await
        .expect("read chain");
    tx.rollback().await.expect("end snapshot");
    events.reverse();
    events
        .into_iter()
        .filter(|event| event.action.starts_with("tenant.") && event.action != "tenant.created")
        .map(|event| (event.action, event.payload))
        .collect()
}

/// One console login's custody and its tenant index row.
async fn console_login(pool: &PgPool, tenant: TenantId) -> [u8; 32] {
    let custody: [u8; 32] = *uuid::Uuid::now_v7()
        .as_bytes()
        .repeat(2)
        .first_chunk()
        .unwrap();
    let expires = chrono::Utc::now() + chrono::Duration::hours(1);
    let mut tx = synveda_store::rls::begin_tenant_write_tx(pool, tenant)
        .await
        .expect("tenant write");
    synveda_store::console_sessions::create(
        &mut *tx,
        &custody,
        "https://issuer.example",
        &[0x5a; 64],
        None,
        None,
        expires,
    )
    .await
    .expect("console custody");
    synveda_store::session_index::insert(
        &mut *tx,
        &synveda_store::session_index::NewEntry {
            tenant_id: tenant,
            id: uuid::Uuid::now_v7(),
            subject: "archivist",
            issuer: "https://issuer.example",
            custody_hash: &custody,
            family_digest: None,
            expires_at: expires,
        },
    )
    .await
    .expect("console index");
    tx.commit().await.expect("commit console login");
    custody
}

async fn custody_exists(pool: &PgPool, custody: &[u8; 32]) -> bool {
    sqlx::query_scalar("select exists (select 1 from console_sessions where token_hash = $1)")
        .bind(&custody[..])
        .fetch_one(pool)
        .await
        .expect("read console custody")
}

#[tokio::test]
async fn a_populated_tenant_is_erased_and_its_canary_is_untouched() {
    let Some(pool) = connect().await else { return };
    let tenant = admit(&pool, "cli-erase").await;
    let canary = admit(&pool, "cli-erase-canary").await;
    let seeded = tenant_population::seed(&pool, tenant).await;
    tenant_population::seed(&pool, canary).await;
    console_login(&pool, tenant).await;
    let canary_custody = console_login(&pool, canary).await;
    let tenant_arg = tenant.to_string();
    let provision = synveda(&["tenant", "key", "provision", "--tenant", &tenant_arg]);
    assert!(provision.status.success(), "{provision:?}");
    let scratch = std::env::temp_dir().join(format!("synveda-erase-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&scratch).expect("scratch directory");
    let secret_file = scratch.join("secret");
    std::fs::write(&secret_file, b"fictional erasure credential").expect("write secret");
    let secret = synveda(&[
        "tenant",
        "secret",
        "put",
        "--tenant",
        &tenant_arg,
        "--scope",
        &seeded.root.to_string(),
        "--kind",
        "import_export",
        "--label",
        "erasure.fixture",
        "--from",
        secret_file.to_str().expect("path"),
    ]);
    assert!(secret.status.success(), "{secret:?}");

    // Unproven lineage hides the first revision from every role, the owner
    // included; only the erasure exception may read and delete it.
    let migrator = tenant_fixture::migrator_pool(&pool).await;
    let mut hide = tenant_fixture::begin(&migrator, tenant).await;
    sqlx::query(
        "insert into unproven_artifact_dependencies (tenant_id, dependent_kind, dependent_id)
         values ($1, 'knowledge_revision', $2)",
    )
    .bind(tenant.as_uuid())
    .bind(seeded.revisions[0].as_uuid())
    .execute(&mut *hide)
    .await
    .expect("hide the first revision");
    hide.commit().await.expect("commit the hidden revision");
    migrator.close().await;

    let before = inventory(&pool, tenant).await;
    assert_eq!(before["knowledge_revisions"], 1, "one revision is hidden");
    let canary_before = inventory(&pool, canary).await;

    let unacknowledged = synveda(&[
        "tenant",
        "erase",
        "--tenant",
        &tenant_arg,
        "--reason",
        "offboarding",
        "--force",
    ]);
    refused(&unacknowledged, "erasure_unacknowledged");
    refused(&erase(&tenant_arg, &[]), "tenant_not_suspended");
    assert_eq!(tenant_row(&pool, tenant).await.status, TenantStatus::Active);
    assert!(transition("suspend", &tenant_arg).status.success());

    // A run cut short commits whole steps and hands its lease back.
    refused(
        &erase(
            &tenant_arg,
            &["--chunk-rows", "1", "--timeout-seconds", "1"],
        ),
        "erasure_timeout",
    );
    assert_eq!(
        tenant_row(&pool, tenant).await.status,
        TenantStatus::Erasing
    );
    refused(&transition("resume", &tenant_arg), "tenant_erasing");
    refused(&transition("suspend", &tenant_arg), "tenant_erasing");
    let archive = scratch.join("erasing.svarc");
    refused(
        &synveda(&[
            "tenant",
            "archive",
            "export",
            "--tenant",
            &tenant_arg,
            "--out",
            archive.to_str().expect("path"),
        ]),
        "tenant_erasing",
    );
    assert!(!archive.exists());

    let done = receipt(&erase(&tenant_arg, &["--chunk-rows", "3"]));
    assert_eq!(done["status"], "erased");
    assert_eq!(done["changed"], true);
    let erased = &done["receipt"];
    assert_eq!(erased["format"], "synveda.tenant-erasure-receipt.v1");
    assert_eq!(erased["attempts"], 2, "the rerun reclaimed the lease once");
    assert_eq!(erased["residual_rows"], 0);
    assert_eq!(erased["deployment_deleted"]["console_sessions"], 1);
    let mut expected_rows = 1_u64;
    for table in &TENANT_TABLES {
        if table.erasure == Erasure::RetainEvidence {
            assert!(
                erased["deleted"].get(table.name).is_none(),
                "{}",
                table.name
            );
            continue;
        }
        let hidden = u64::from(table.name == "knowledge_revisions");
        assert_eq!(
            erased["deleted"][table.name],
            before[table.name] + hidden,
            "measured deletions for {}",
            table.name
        );
        expected_rows += before[table.name];
    }
    assert_eq!(erased["deleted_rows"], expected_rows);
    println!(
        "tenant erasure receipt: deleted_rows={} attempts={} console_sessions={} retained={} digest={}",
        erased["deleted_rows"],
        erased["attempts"],
        erased["deployment_deleted"]["console_sessions"],
        erased["retained"],
        erased["digest"]
    );
    assert!(before["session_events"] >= 3 && before["scopes"] >= 3);
    assert_eq!(before["tenant_secrets"], 1);

    let row = tenant_row(&pool, tenant).await;
    assert_eq!(row.status, TenantStatus::Erased);
    assert_eq!(row.name, "", "the tombstone keeps only id, slug and status");
    let after = inventory(&pool, tenant).await;
    for table in &TENANT_TABLES {
        let rows = after[table.name];
        match (table.erasure, table.name) {
            (Erasure::RetainEvidence, name) => {
                // The receipt is measured before `tenant.erased` is chained.
                let chained = u64::from(name == "audit_log");
                assert_eq!(erased["retained"][name], rows - chained, "{name}");
                assert!(rows > 0, "{name} retains evidence");
            }
            (Erasure::RetainLifecycleRows, "durable_operations") => assert_eq!(rows, 1),
            (Erasure::RetainLifecycleRows, _) => assert_eq!(rows, 2, "both attempts remain"),
            (Erasure::Erase, name) => assert_eq!(rows, 0, "{name} is empty"),
        }
    }
    let actions: Vec<String> = lifecycle_events(&pool, tenant)
        .await
        .into_iter()
        .map(|(action, payload)| {
            if action == "tenant.erased" {
                assert_eq!(payload["receipt_digest"], erased["digest"]);
                assert_eq!(payload["deleted_rows"], erased["deleted_rows"]);
            }
            action
        })
        .collect();
    assert_eq!(
        actions,
        [
            "tenant.key.provisioned",
            "tenant.secret.stored",
            "tenant.suspended",
            "tenant.erasure.started",
            "tenant.erased"
        ]
    );

    assert_eq!(inventory(&pool, canary).await, canary_before);
    assert_eq!(tenant_row(&pool, canary).await.status, TenantStatus::Active);
    assert!(custody_exists(&pool, &canary_custody).await);

    let again = receipt(&erase(&tenant_arg, &[]));
    assert_eq!(again["changed"], false);
    assert_eq!(again["receipt"]["digest"], erased["digest"]);
    refused(&transition("resume", &tenant_arg), "tenant_erased");
    refused(&transition("suspend", &tenant_arg), "tenant_erased");
    std::fs::remove_dir_all(&scratch).ok();
}

#[tokio::test]
async fn a_held_revision_stops_erasure_before_anything_is_deleted() {
    let Some(pool) = connect().await else { return };
    let tenant = admit(&pool, "cli-erase-held").await;
    let seeded = tenant_population::seed(&pool, tenant).await;
    let mut tx = synveda_store::rls::begin_tenant_write_tx(&pool, tenant)
        .await
        .expect("tenant write");
    let source = synveda_store::knowledge::create_source(
        &mut tx,
        &synveda_store::knowledge::NewKnowledgeSource {
            id: KnowledgeSourceId::new(),
            tenant_id: tenant,
            scope_id: seeded.root,
            source_type: KnowledgeSourceType::Document,
            session_event_id: None,
            import_artifact_id: None,
            locator: Some("fictional:held-procedure".to_owned()),
            source_revision: Some("fixture-v1".to_owned()),
            content_hash: None,
            metadata: serde_json::json!({}),
            created_by: Some("archivist".to_owned()),
        },
    )
    .await
    .expect("source");
    synveda_store::knowledge::create_item(
        &mut tx,
        &synveda_store::knowledge::NewKnowledgeItem {
            id: KnowledgeItemId::new(),
            tenant_id: tenant,
            scope_id: seeded.root,
            project_id: None,
            owner_principal_id: None,
            knowledge_type: KnowledgeType::Procedure,
            origin: KnowledgeOrigin::Authored,
            created_by: Some("archivist".to_owned()),
        },
        &synveda_store::knowledge::NewKnowledgeRevision {
            id: KnowledgeRevisionId::new(),
            content: KnowledgeRevisionContent {
                title: "Held procedure".to_owned(),
                body_markdown: "A fictional procedure under legal hold.".to_owned(),
                summary: "Held".to_owned(),
                tags: Vec::new(),
                sensitivity: Sensitivity::Internal,
                confidence_permille: 900,
                valid_from: chrono::Utc::now(),
                valid_to: None,
                stale_after: None,
                verification_metadata: serde_json::json!({}),
                metadata: serde_json::json!({ "legal_hold": true }),
            },
            created_by: Some("archivist".to_owned()),
        },
        &[source.id],
    )
    .await
    .expect("held item");
    tx.commit().await.expect("commit the held item");
    let tenant_arg = tenant.to_string();
    assert!(transition("suspend", &tenant_arg).status.success());
    let before = inventory(&pool, tenant).await;

    refused(&erase(&tenant_arg, &[]), "erasure_held");
    assert_eq!(
        tenant_row(&pool, tenant).await.status,
        TenantStatus::Suspended
    );
    let after = inventory(&pool, tenant).await;
    for (table, rows) in &before {
        let added = match *table {
            "durable_operations" | "audit_log" => 1,
            _ => 0,
        };
        assert_eq!(after[table], rows + added, "{table} is unchanged");
    }
    let events = lifecycle_events(&pool, tenant).await;
    let held = events
        .iter()
        .find(|(action, _)| action == "tenant.erasure.held")
        .expect("the hold is on the chain");
    assert_eq!(held.1["held_revisions"], 1);
    assert!(
        transition("resume", &tenant_arg).status.success(),
        "a held erasure leaves the tenant resumable"
    );
}

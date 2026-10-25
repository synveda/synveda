//! `synveda tenant suspend|resume|inventory` as separate processes (ADR-0157).
//!
//! The CLI child runs suspension and resumption under the exact test migrator
//! credential; every effect is observed here through the ordinary
//! `DATABASE_URL` role. Resumption refuses an uncompilable stored pack and a
//! KEK that cannot open the tenant's key, both repairable while suspended.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::process::{Command, Output};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_types::{PackConfig, TenantId, TenantStatus};

const KEK: &str = "5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";
const OTHER_KEK: &str = "a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5";
const VALID_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };
"#;

/// Runs the CLI as the migrator (`operator`) or the ordinary role.
fn synveda(operator: bool, kek: Option<&str>, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
    command.args(arguments).env_remove("SYNVEDA_KMS_KEY_FILE");
    if operator {
        let migrator = std::env::var_os("SYNVEDA_TEST_MIGRATOR_DATABASE_URL_FILE")
            .expect("test migrator URL file");
        command
            .env_remove("DATABASE_URL")
            .env("DATABASE_URL_FILE", migrator);
    } else {
        command.env_remove("DATABASE_URL_FILE");
    }
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

fn refusal(output: &Output) -> String {
    assert!(!output.status.success(), "{output:?}");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

async fn status(pool: &PgPool, tenant: TenantId) -> TenantStatus {
    synveda_store::tenants::by_id(pool, tenant)
        .await
        .expect("read tenant")
        .expect("tenant exists")
        .status
}

async fn lifecycle_events(
    pool: &PgPool,
    tenant: TenantId,
) -> Vec<(String, String, serde_json::Value)> {
    let mut tx = synveda_store::rls::begin_tenant_snapshot_tx(pool, tenant)
        .await
        .expect("snapshot");
    let mut events = synveda_audit::tail(&mut tx, tenant, 100)
        .await
        .expect("read chain");
    tx.rollback().await.expect("end snapshot");
    events.reverse();
    events
        .into_iter()
        .filter(|event| event.action.starts_with("tenant.") && event.action != "tenant.created")
        .map(|event| (event.action, event.actor_kind, event.payload))
        .collect()
}

#[tokio::test]
async fn operator_suspension_and_revalidated_resumption_across_processes() {
    let Some(url) = tenant_fixture::database_url("CLI tenant lifecycle test") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("current schema");
    let tenant = TenantId::new();
    let canary = TenantId::new();
    for (id, prefix) in [(tenant, "cli-life"), (canary, "cli-canary")] {
        let slug = format!("{prefix}-{}", id.as_uuid().simple());
        tenant_fixture::create(&pool, id, &slug, "CLI lifecycle", TenantStatus::Active)
            .await
            .expect("admit tenant");
    }
    let mut tx = tenant_fixture::begin(&pool, tenant).await;
    synveda_store::scopes::ensure_tenant_root(&mut tx, tenant)
        .await
        .expect("root");
    synveda_store::policy_packs::apply(
        &mut *tx,
        tenant,
        "cli-life-pack",
        "permit (principal",
        &PackConfig::default(),
    )
    .await
    .expect("store an uncompilable pack out of band");
    tx.commit().await.expect("commit fixture");
    let tenant_arg = tenant.to_string();
    let provision = synveda(
        true,
        Some(KEK),
        &["tenant", "key", "provision", "--tenant", &tenant_arg],
    );
    assert!(provision.status.success(), "{provision:?}");

    let held = tenant_fixture::begin(&pool, tenant).await;
    let busy = synveda(
        true,
        None,
        &[
            "tenant",
            "suspend",
            "--tenant",
            &tenant_arg,
            "--reason",
            "incident",
        ],
    );
    assert!(refusal(&busy).contains("tenant_busy"), "{busy:?}");
    held.rollback()
        .await
        .expect("release the admitted transaction");
    assert_eq!(status(&pool, tenant).await, TenantStatus::Active);

    let suspend = [
        "tenant",
        "suspend",
        "--tenant",
        &tenant_arg,
        "--reason",
        "incident",
    ];
    let first = receipt(&synveda(true, None, &suspend));
    assert_eq!(
        (first["status"].as_str(), first["changed"].as_bool()),
        (Some("suspended"), Some(true))
    );
    let again = receipt(&synveda(true, None, &suspend));
    assert_eq!(
        again["changed"], false,
        "a repeated suspension changes nothing"
    );
    let refused = synveda_store::rls::begin_tenant_tx(&pool, tenant).await;
    assert!(matches!(&refused, Err(error) if synveda_store::rls::is_tenant_unavailable(error)));
    synveda_store::rls::begin_tenant_tx(&pool, canary)
        .await
        .expect("the canary keeps working")
        .rollback()
        .await
        .unwrap();

    let inventory = receipt(&synveda(
        false,
        None,
        &["tenant", "inventory", "--tenant", &tenant_arg, "--json"],
    ));
    assert_eq!(inventory["status"], "suspended");
    let scopes = inventory["tables"]
        .as_array()
        .and_then(|tables| tables.iter().find(|table| table["table"] == "scopes"))
        .expect("scopes counted");
    assert_eq!(scopes["rows"], 1, "{inventory}");

    let resume = [
        "tenant",
        "resume",
        "--tenant",
        &tenant_arg,
        "--reason",
        "recovery",
    ];
    assert!(refusal(&synveda(true, Some(KEK), &resume)).contains("policy_invalid"));
    assert_eq!(status(&pool, tenant).await, TenantStatus::Suspended);
    let scratch = std::env::temp_dir().join(format!("synveda-cli-lifecycle-{tenant}"));
    std::fs::create_dir_all(&scratch).expect("scratch directory");
    let pack = scratch.join("valid.cedar");
    std::fs::write(&pack, VALID_PACK).expect("write pack");
    let repaired = synveda(
        false,
        None,
        &[
            "policy",
            "apply",
            "--tenant",
            &tenant_arg,
            "--name",
            "cli-life-pack",
            pack.to_str().unwrap(),
        ],
    );
    assert!(
        repaired.status.success(),
        "a suspended tenant's pack is repairable: {repaired:?}"
    );

    assert!(refusal(&synveda(true, Some(OTHER_KEK), &resume)).contains("key_unavailable"));
    assert!(refusal(&synveda(true, None, &resume)).contains("key_unavailable"));
    assert_eq!(status(&pool, tenant).await, TenantStatus::Suspended);
    let resumed = receipt(&synveda(true, Some(KEK), &resume));
    assert_eq!(
        (resumed["status"].as_str(), resumed["changed"].as_bool()),
        (Some("active"), Some(true))
    );
    synveda_store::rls::begin_tenant_tx(&pool, tenant)
        .await
        .expect("a resumed tenant is admitted")
        .rollback()
        .await
        .unwrap();

    let events = lifecycle_events(&pool, tenant).await;
    let transitions: Vec<_> = events
        .iter()
        .filter(|(action, _, _)| action == "tenant.suspended" || action == "tenant.resumed")
        .collect();
    assert_eq!(transitions.len(), 2, "{events:?}");
    assert_eq!(transitions[0].0, "tenant.suspended");
    assert_eq!(transitions[0].1, "break_glass");
    assert_eq!(transitions[0].2["reason"], "incident");
    assert_eq!(transitions[1].0, "tenant.resumed");
    assert_eq!(transitions[1].2["policy_packs"], 1);
    assert_eq!(transitions[1].2["key_generations"], 1);
    std::fs::remove_dir_all(&scratch).ok();
}

//! AUTHZ-2: `synveda policy apply` and `clear` are the operator repair for a
//! tenant's stored policy. A malformed file is refused before any write; a
//! compiling replacement and a clear each leave break-glass audit evidence.

#[path = "../../synveda-store/tests/support/tenant_fixture.rs"]
mod tenant_fixture;

use std::path::Path;
use std::process::{Command, Output};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use synveda_store::policy_packs;
use synveda_types::{PackConfig, TenantId, TenantStatus};

const VALID_PACK: &str = r#"
permit (
    principal,
    action == Synveda::Action::"ScopeRead",
    resource
) when { resource in principal.tenant };
"#;
const MALFORMED_PACK: &str = "permit (principal";

fn policy(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_synveda"))
        .arg("policy")
        .args(arguments)
        .env_remove("DATABASE_URL_FILE")
        .output()
        .expect("run the synveda CLI")
}

async fn stored(pool: &PgPool, tenant: TenantId, name: &str) -> Option<(i64, String)> {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    policy_packs::get(&mut *tx, tenant, name)
        .await
        .expect("read stored pack")
        .map(|pack| (pack.version, pack.source))
}

async fn store_out_of_band(pool: &PgPool, tenant: TenantId, name: &str, source: &str) {
    let mut tx = tenant_fixture::begin(pool, tenant).await;
    policy_packs::apply(&mut *tx, tenant, name, source, &PackConfig::default())
        .await
        .expect("store pack without a compile check");
    tx.commit().await.expect("commit pack");
}

fn write(directory: &Path, name: &str, source: &str) -> String {
    let path = directory.join(name);
    std::fs::write(&path, source).expect("write pack file");
    path.to_str().expect("UTF-8 scratch path").to_owned()
}

#[tokio::test]
async fn policy_repair_is_compile_checked_and_audited() {
    let Some(url) = tenant_fixture::database_url("CLI policy repair test") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("connect to DATABASE_URL");
    synveda_store::epoch::verify(&pool)
        .await
        .expect("current schema epoch");
    let tenant = TenantId::new();
    let slug = format!("cli-policy-{}", tenant.as_uuid().simple());
    tenant_fixture::create(
        &pool,
        tenant,
        &slug,
        "CLI policy repair",
        TenantStatus::Active,
    )
    .await
    .expect("admit tenant");
    store_out_of_band(&pool, tenant, "cli-repaired", MALFORMED_PACK).await;
    store_out_of_band(&pool, tenant, "cli-orphan", MALFORMED_PACK).await;

    let scratch = std::env::temp_dir().join(format!("synveda-cli-policy-repair-{tenant}"));
    std::fs::create_dir_all(&scratch).expect("create scratch directory");
    let malformed = write(&scratch, "malformed.cedar", MALFORMED_PACK);
    let valid = write(&scratch, "valid.cedar", VALID_PACK);
    let tenant_arg = tenant.to_string();
    let apply = |file: &str| {
        policy(&[
            "apply",
            "--tenant",
            &tenant_arg,
            "--name",
            "cli-repaired",
            file,
        ])
    };

    let refused = apply(&malformed);
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(
        stored(&pool, tenant, "cli-repaired").await,
        Some((1, MALFORMED_PACK.to_owned())),
        "a refused repair must not write"
    );

    let repaired = apply(&valid);
    assert!(repaired.status.success(), "{repaired:?}");
    let receipt: serde_json::Value =
        serde_json::from_slice(&repaired.stdout).expect("JSON receipt");
    assert_eq!(receipt["version"], 2, "{receipt}");
    assert_eq!(
        stored(&pool, tenant, "cli-repaired").await,
        Some((2, VALID_PACK.to_owned()))
    );

    let cleared = policy(&["clear", "--tenant", &tenant_arg, "--name", "cli-orphan"]);
    assert!(cleared.status.success(), "{cleared:?}");
    assert_eq!(stored(&pool, tenant, "cli-orphan").await, None);
    std::fs::remove_dir_all(&scratch).expect("remove scratch directory");

    let mut tx = tenant_fixture::begin(&pool, tenant).await;
    let mut events = synveda_audit::tail(&mut tx, tenant, 50)
        .await
        .expect("read audit chain");
    events.reverse();
    let repairs: Vec<_> = events
        .iter()
        .filter(|event| event.action.starts_with("policy.pack."))
        .collect();
    let recorded: Vec<_> = repairs
        .iter()
        .map(|event| (event.actor_kind.as_str(), event.action.as_str()))
        .collect();
    assert_eq!(
        recorded,
        [
            ("break_glass", "policy.pack.applied"),
            ("break_glass", "policy.pack.cleared"),
        ]
    );
    assert_eq!(repairs[0].payload["version"], 2);
    assert_eq!(
        synveda_audit::verify(&mut tx, tenant)
            .await
            .expect("verify audit chain"),
        synveda_audit::ChainVerification::Valid {
            events: i64::try_from(events.len()).expect("bounded chain")
        }
    );
}

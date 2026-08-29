//! Process acceptance for the CLI database direct/file boundary (CPR-45).

use std::path::Path;
use std::process::Command;

fn database_preflight_command(role_contract: &Path, migrator_url: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
    command.args(["db", "preflight"]);
    for setting in [
        "SYNVEDA_DATABASE_ROLES",
        "SYNVEDA_MIGRATOR_DATABASE_URL",
        "SYNVEDA_GATEWAY_DATABASE_URL",
        "SYNVEDA_WORKER_DATABASE_URL",
    ] {
        command.env_remove(setting);
    }
    command
        .env("SYNVEDA_DATABASE_ROLES_FILE", role_contract)
        .env("SYNVEDA_MIGRATOR_DATABASE_URL_FILE", migrator_url)
        .env("SYNVEDA_GATEWAY_DATABASE_URL_FILE", migrator_url)
        .env("SYNVEDA_WORKER_DATABASE_URL_FILE", migrator_url);
    command
}

#[test]
fn migrate_uses_database_url_file_and_keeps_failures_content_free() {
    const FILE_SENTINEL: &str = "SYNVEDA_DATABASE_URL_FILE_PROCESS_SECRET";
    const DIRECT_SENTINEL: &str = "SYNVEDA_DATABASE_URL_DIRECT_PROCESS_SECRET";
    let scratch = std::env::temp_dir().join(format!(
        "synveda-cli-database-url-file-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join(format!("{FILE_SENTINEL}.txt"));
    std::fs::write(
        &path,
        format!("https://admin:{FILE_SENTINEL}@db.example.test/synveda\n"),
    )
    .unwrap();

    let file_only = Command::new(env!("CARGO_BIN_EXE_synveda"))
        .args(["db", "migrate"])
        .env_remove("DATABASE_URL")
        .env("DATABASE_URL_FILE", &path)
        .output()
        .unwrap();
    assert!(!file_only.status.success());
    let file_output = format!(
        "{}{}",
        String::from_utf8_lossy(&file_only.stdout),
        String::from_utf8_lossy(&file_only.stderr)
    );
    assert!(
        file_output.contains("DATABASE_URL is not a valid PostgreSQL connection URL"),
        "{file_output}"
    );
    assert!(!file_output.contains(FILE_SENTINEL), "{file_output}");
    assert!(
        !file_output.contains(&path.display().to_string()),
        "{file_output}"
    );

    let ambiguous = Command::new(env!("CARGO_BIN_EXE_synveda"))
        .args(["db", "migrate"])
        .env(
            "DATABASE_URL",
            format!("postgres://admin:{DIRECT_SENTINEL}@db.example.test/synveda"),
        )
        .env("DATABASE_URL_FILE", &path)
        .output()
        .unwrap();
    assert!(!ambiguous.status.success());
    let ambiguous_output = format!(
        "{}{}",
        String::from_utf8_lossy(&ambiguous.stdout),
        String::from_utf8_lossy(&ambiguous.stderr)
    );
    assert!(
        ambiguous_output.contains(
            "DATABASE_URL and DATABASE_URL_FILE are mutually exclusive; configure exactly one"
        ),
        "{ambiguous_output}"
    );
    assert!(
        !ambiguous_output.contains(FILE_SENTINEL),
        "{ambiguous_output}"
    );
    assert!(
        !ambiguous_output.contains(DIRECT_SENTINEL),
        "{ambiguous_output}"
    );
    assert!(
        !ambiguous_output.contains(&path.display().to_string()),
        "{ambiguous_output}"
    );

    std::fs::remove_dir_all(scratch).ok();
}

#[test]
fn database_preflight_process_failures_are_content_free() {
    const FILE_SENTINEL: &str = "CPR45_PREFLIGHT_FILE_SECRET";
    const DIRECT_SENTINEL: &str = "CPR45_PREFLIGHT_DIRECT_SECRET";
    let scratch = std::env::temp_dir().join(format!(
        "synveda-cli-database-preflight-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();
    let role_contract = scratch.join("roles.json");
    std::fs::write(
        &role_contract,
        r#"{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["synveda_owner"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}
"#,
    )
    .unwrap();
    let migrator_url = scratch.join("migrator-url");

    std::fs::write(
        &migrator_url,
        format!("https://synveda_migrator:{FILE_SENTINEL}@db.invalid/synveda\n"),
    )
    .unwrap();
    let invalid = database_preflight_command(&role_contract, &migrator_url)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert_eq!(invalid.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&invalid.stderr),
        "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE is not a valid PostgreSQL URL\n"
    );
    assert!(
        !invalid
            .stderr
            .windows(FILE_SENTINEL.len())
            .any(|window| { window == FILE_SENTINEL.as_bytes() })
    );
    assert!(
        !String::from_utf8_lossy(&invalid.stderr).contains(&migrator_url.display().to_string())
    );

    std::fs::write(
        &migrator_url,
        format!("postgresql://synveda_migrator:{FILE_SENTINEL}@127.0.0.1:1/synveda\n"),
    )
    .unwrap();
    let unavailable = database_preflight_command(&role_contract, &migrator_url)
        .output()
        .unwrap();
    assert!(!unavailable.status.success());
    assert_eq!(unavailable.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&unavailable.stderr),
        "synveda: SYNVEDA_MIGRATOR_DATABASE_URL_FILE connection failed\n"
    );
    assert!(!String::from_utf8_lossy(&unavailable.stderr).contains(FILE_SENTINEL));
    assert!(
        !String::from_utf8_lossy(&unavailable.stderr).contains(&migrator_url.display().to_string())
    );

    let ambiguous = database_preflight_command(&role_contract, &migrator_url)
        .env(
            "SYNVEDA_MIGRATOR_DATABASE_URL",
            format!("postgresql://synveda_migrator:{DIRECT_SENTINEL}@127.0.0.1:1/synveda"),
        )
        .output()
        .unwrap();
    assert!(!ambiguous.status.success());
    assert_eq!(ambiguous.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&ambiguous.stderr),
        "synveda: SYNVEDA_MIGRATOR_DATABASE_URL is forbidden for database preflight; use SYNVEDA_MIGRATOR_DATABASE_URL_FILE\n"
    );
    let ambiguous_stderr = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(!ambiguous_stderr.contains(FILE_SENTINEL));
    assert!(!ambiguous_stderr.contains(DIRECT_SENTINEL));
    assert!(!ambiguous_stderr.contains(&migrator_url.display().to_string()));

    std::fs::remove_dir_all(scratch).ok();
}

#[test]
fn tenant_admission_refuses_an_implicit_database_target() {
    let scratch = std::env::temp_dir().join(format!(
        "synveda-cli-tenant-explicit-database-{}",
        std::process::id()
    ));
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();
    let role_contract = scratch.join("roles.json");
    std::fs::write(
        &role_contract,
        r#"{"migrator":"synveda_migrator","gateway":"synveda_gateway","worker":"synveda_worker","administrators":["synveda_owner"],"administrative_memberships":[],"forbidden_databases":["postgres"],"isolated_peer_roles":[]}
"#,
    )
    .unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
    command.args([
        "tenant",
        "create",
        "--slug",
        "cpr45-explicit-database",
        "--name",
        "CPR-45 explicit database refusal",
    ]);
    for setting in [
        "DATABASE_URL",
        "DATABASE_URL_FILE",
        "SYNVEDA_DATABASE_ROLES",
        "SYNVEDA_EVAL_MIGRATOR_DATABASE_URL_FILE",
        "SYNVEDA_EVAL_GATEWAY_DATABASE_URL_FILE",
        "SYNVEDA_EVAL_WORKER_DATABASE_URL_FILE",
    ] {
        command.env_remove(setting);
    }
    let refusal = command
        .env("SYNVEDA_DATABASE_ROLES_FILE", &role_contract)
        .output()
        .unwrap();
    assert!(!refusal.status.success());
    assert_eq!(refusal.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&refusal.stderr),
        "synveda: tenant create requires explicit DATABASE_URL or DATABASE_URL_FILE for the configured migrator\n"
    );

    std::fs::remove_dir_all(scratch).ok();
}

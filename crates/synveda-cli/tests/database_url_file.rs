//! Process acceptance for the CLI database direct/file boundary (CPR-45).

use std::process::Command;

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

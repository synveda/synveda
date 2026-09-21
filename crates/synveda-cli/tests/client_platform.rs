//! OPS-12 executable platform boundaries. No issuer or harness claim.

use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "synveda-client-platform-{}",
            synveda_types::TenantId::new()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
        command
            .current_dir(&self.0)
            .env_remove("HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("LOCALAPPDATA")
            .env_remove("SYNVEDA_TOKEN")
            .env_remove("SYNVEDA_GATEWAY")
            .env_remove("SYNVEDA_DATABASE_PEER_WITNESS_FILE");
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn version_and_help_need_no_private_home_or_service() {
    let scratch = Scratch::new();
    for argument in ["--version", "--help"] {
        let output = scratch.command().arg(argument).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn missing_home_never_uses_a_repository_relative_spool() {
    let scratch = Scratch::new();
    // A misleading empty backlog would silently abandon undelivered events.
    // The command must instead report the missing root and leave these bytes.
    let spool = scratch.0.join(".local/state/synveda/spool");
    std::fs::create_dir_all(&spool).unwrap();
    let retained = spool.join("retained.json");
    std::fs::write(&retained, b"retained observation").unwrap();
    for args in [
        vec!["session", "spool", "status", "--json"],
        vec!["session", "flush"],
    ] {
        let output = scratch.command().args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("HOME is not set"));
        assert_eq!(std::fs::read(&retained).unwrap(), b"retained observation");
    }
}

#[cfg(windows)]
#[test]
fn private_commands_refuse_before_issuer_access_or_filesystem_mutation() {
    let scratch = Scratch::new();
    let retained = scratch.0.join("retained.json");
    std::fs::write(&retained, b"retained private state").unwrap();
    let commands = [
        vec!["login", "--gateway", "http://127.0.0.1:1", "--no-browser"],
        vec!["auth", "token", "--json"],
        vec!["auth", "logout", "--all"],
        vec!["session", "spool", "status", "--json"],
        vec!["session", "flush"],
        vec!["session", "spool", "purge", "--acknowledged"],
        vec!["demo", "status"],
        vec!["mcp", "install", "--client", "cursor"],
    ];
    for args in commands {
        let output = scratch
            .command()
            .args(&args)
            .env("LOCALAPPDATA", scratch.0.join("Local"))
            .env("XDG_CONFIG_HOME", scratch.0.join("config"))
            .env("XDG_STATE_HOME", scratch.0.join("state"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("native ACL"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "private bytes must not be printed"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
        assert_eq!(std::fs::read(&retained).unwrap(), b"retained private state");
    }
    let output = scratch
        .command()
        .args(["session", "spool", "status", "--dir"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("native ACL"));
    assert_eq!(std::fs::read(&retained).unwrap(), b"retained private state");
}

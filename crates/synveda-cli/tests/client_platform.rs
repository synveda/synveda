//! OPS-12 executable platform boundaries. No issuer or harness claim.

use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

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
        // Keep native private-state refusals identical for source and packaged CLIs.
        let binary = std::env::var_os("SYNVEDA_PACKAGED_CLI")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_synveda").into());
        let mut command = Command::new(binary);
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
fn unported_commands_refuse_by_name_before_filesystem_mutation() {
    let scratch = Scratch::new();
    let retained = scratch.0.join("retained.json");
    std::fs::write(&retained, b"retained private state").unwrap();
    let commands = [
        (vec!["status"], "Compose deployment"),
        (vec!["up"], "Compose deployment"),
        (vec!["down"], "Compose deployment"),
    ];
    for (args, surface) in commands {
        let output = scratch
            .command()
            .args(&args)
            .env("LOCALAPPDATA", scratch.0.join("Local"))
            .env("XDG_CONFIG_HOME", scratch.0.join("config"))
            .env("XDG_STATE_HOME", scratch.0.join("state"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        for expected in [surface, "not supported on native Windows", "OPS-12"] {
            assert!(stderr.contains(expected), "{args:?}: {stderr}");
        }
        assert!(
            output.stdout.is_empty(),
            "private bytes must not be printed"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
        assert_eq!(std::fs::read(&retained).unwrap(), b"retained private state");
    }
    // A broad ACL still refuses the now-implemented spool read before bytes
    // reach status/flush/purge, including an explicit --dir override.
    windows_private::private(&scratch.0);
    windows_private::allow(&scratch.0, "S-1-1-0", "Read", "None");
    let output = scratch
        .command()
        .args(["session", "spool", "status", "--dir"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ACL"));
    assert_eq!(std::fs::read(&retained).unwrap(), b"retained private state");
}

#[cfg(windows)]
#[test]
fn unsafe_credentials_refuse_before_issuer_access_without_repair_or_disclosure() {
    let scratch = Scratch::new();
    let config = scratch.0.join("config/synveda");
    std::fs::create_dir_all(&config).unwrap();
    windows_private::private(&config);
    let path = config.join("credentials.json");
    std::fs::write(&path, b"retained private credential").unwrap();
    windows_private::allow(&path, "S-1-1-0", "Read", "None");
    for args in [
        vec!["login", "--gateway", "http://127.0.0.1:1", "--no-browser"],
        vec!["auth", "token", "--json"],
        vec!["auth", "logout", "--all"],
    ] {
        let output = scratch
            .command()
            .args(&args)
            .env("XDG_CONFIG_HOME", scratch.0.join("config"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("ACL"),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("retained private credential"));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"retained private credential"
        );
    }
}

#[cfg(windows)]
#[path = "support/loopback_gateway.rs"]
mod loopback_gateway;
#[cfg(windows)]
use loopback_gateway::{PROJECT, WORKSPACE, gateway};

#[cfg(windows)]
fn native_command(scratch: &Scratch) -> Command {
    let mut command = scratch.command();
    command
        .env("LOCALAPPDATA", scratch.0.join("Local"))
        .env("XDG_CONFIG_HOME", scratch.0.join("config"))
        .env("XDG_STATE_HOME", scratch.0.join("state"));
    command
}

#[cfg(windows)]
fn leftovers(directory: &std::path::Path) -> Vec<std::ffi::OsString> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().starts_with(".synveda-"))
        .collect()
}

#[cfg(windows)]
#[test]
fn mcp_install_and_uninstall_edit_vendor_configuration_natively() {
    let scratch = Scratch::new();
    // Receipts and the operation lock need private ancestry; the vendor file
    // itself keeps whatever ACL its directory gives it.
    windows_private::private(&scratch.0);
    let vendor = scratch.0.join("vendor dir 文");
    let config = vendor.join("mcp.json");
    let run = |verb: &str| {
        let output = native_command(&scratch)
            .args(["mcp", verb, "--client", "cursor", "--config"])
            .arg(&config)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{verb}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(run("install").contains("wrote"));
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    let command = written["mcpServers"]["synveda"]["command"]
        .as_str()
        .unwrap();
    assert!(
        command.ends_with("synveda.exe") && !command.starts_with(r"\\?\"),
        "{command}"
    );
    assert!(run("install").contains("already names this binary"));
    std::fs::write(
        &config,
        r#"{"mcpServers":{"other":{"command":"other.exe"},"synveda":{"command":"stale.exe"}}}"#,
    )
    .unwrap();
    run("uninstall");
    let kept: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        kept,
        serde_json::json!({"mcpServers":{"other":{"command":"other.exe"}}})
    );
    assert!(leftovers(&vendor).is_empty());
}

#[cfg(windows)]
#[test]
fn setup_edits_project_configuration_and_hook_visible_receipts_natively() {
    use sha2::{Digest, Sha256};

    let scratch = Scratch::new();
    windows_private::private(&scratch.0);
    let project = scratch.0.join("project λ");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join(".synveda")).unwrap();
    let config = project.join(".synveda/config.json");
    std::fs::write(&config, br#"{"budget_tokens":900}"#).unwrap();
    let (url, served) = gateway(4);
    for observation in ["off", "on"] {
        let output = native_command(&scratch)
            .current_dir(&project)
            .env("SYNVEDA_GATEWAY", &url)
            .env("SYNVEDA_TOKEN", "fixture-bearer")
            .args(["setup", "--project", PROJECT, "--observation", observation])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(served.join().unwrap(), 4);
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        written,
        serde_json::json!({"budget_tokens":900,"project_id":PROJECT,"workspace_id":WORKSPACE,"observe":true,"managed_observation":true})
    );
    // Hooks key the receipt by the conventional `C:\...` root that Node's
    // realpath reports, never Rust's verbatim `\?\C:\...` spelling.
    let canonical = std::fs::canonicalize(&project).unwrap();
    let root = canonical.to_str().unwrap().trim_start_matches(r"\\?\");
    let key = format!("{:x}", Sha256::digest(root.as_bytes()));
    let receipts = scratch.0.join("config/synveda/consumer");
    let receipt = receipts.join(format!("setup-{key}.json"));
    assert!(
        receipt.is_file(),
        "no receipt for {root}: {:?}",
        std::fs::read_dir(&receipts)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>()
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(saved["root"], root);
    assert!(
        !std::fs::read_to_string(&receipt)
            .unwrap()
            .contains("fixture-bearer")
    );
    assert!(leftovers(&project.join(".synveda")).is_empty());
}

#[cfg(windows)]
#[test]
fn demo_receipts_use_native_private_state() {
    let scratch = Scratch::new();
    windows_private::private(&scratch.0);
    let status = || {
        native_command(&scratch)
            .args(["demo", "status", "--json"])
            .output()
            .unwrap()
    };
    let output = status();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no PulseBoard demo receipt"), "{stderr}");
    // A receipt whose ACL was broadened is refused before any byte is shown.
    let receipts = scratch.0.join("state").join("synveda");
    std::fs::create_dir_all(&receipts).unwrap();
    let receipt = receipts.join("pulseboard-demo.json");
    std::fs::write(
        &receipt,
        br#"{"receipt_version":1,"secret":"retained-demo-bytes"}"#,
    )
    .unwrap();
    windows_private::allow(&receipt, "S-1-1-0", "Read", "None");
    let output = status();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ACL") || stderr.contains("S-1-1-0"),
        "{stderr}"
    );
    assert!(!stderr.contains("retained-demo-bytes"));
    assert!(output.stdout.is_empty());
}

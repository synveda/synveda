//! OPS-12: native installer process boundaries, without touching a user's
//! Claude configuration. These fixtures prove commands and reconciliation on
//! Unix and native Windows; real vendor registration/loading is recorded
//! separately.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

#[path = "support/vendor.rs"]
mod vendor;
#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    bundle: PathBuf,
    before: Vec<Value>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "synveda plugin scope λ-{name}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&root).ok();
        let project = root.join("project");
        let bundle = root.join("marketplace");
        for directory in [
            root.join("bin"),
            project.join(".git"),
            project.join("nested"),
            root.join("other project"),
            bundle.join(".claude-plugin"),
            bundle.join("synveda/.claude-plugin"),
        ] {
            std::fs::create_dir_all(directory).unwrap();
        }
        // Native Windows keeps the operation lock in private state.
        #[cfg(windows)]
        windows_private::private(&root);
        let root = vendor::conventional(root.canonicalize().unwrap());
        let project = vendor::conventional(project.canonicalize().unwrap());
        let bundle = vendor::conventional(bundle.canonicalize().unwrap());
        let entry = |id, scope, path: Option<&PathBuf>| {
            json!({"id":id, "version":"0.4.0", "scope":scope,
                "enabled":true, "projectPath":path})
        };
        let before = vec![
            entry("unrelated@other", "user", None),
            entry("synveda@synveda", "user", None),
            entry("synveda@synveda", "project", Some(&project)),
            entry("synveda@synveda", "local", Some(&project)),
            entry(
                "synveda@synveda",
                "project",
                Some(&root.join("other project")),
            ),
        ];
        let fixture = Self {
            root,
            project,
            bundle,
            before,
        };
        fixture.write("state.json", &json!(fixture.before));
        fixture.write("installed.json", &json!(fixture.before));
        let remaining: Vec<_> = fixture
            .before
            .iter()
            .filter(|entry| {
                !(entry["scope"] == "project"
                    && entry["projectPath"] == fixture.project.to_str().unwrap())
            })
            .collect();
        fixture.write("removed.json", &json!(remaining));
        fixture.write(
            "marketplaces.json",
            &json!([{"name":"synveda", "source":"directory", "path":fixture.bundle}]),
        );
        std::fs::write(
            fixture.bundle.join(".claude-plugin/marketplace.json"),
            r#"{"name":"synveda","plugins":[{"name":"synveda","source":"./synveda"}]}"#,
        )
        .unwrap();
        std::fs::write(
            fixture.bundle.join("synveda/.claude-plugin/plugin.json"),
            r#"{"name":"synveda","version":"0.4.0"}"#,
        )
        .unwrap();
        vendor::install(&fixture.root.join("bin"), "claude");
        vendor::rules(
            &fixture.root,
            &[
                (
                    "plugin list --json",
                    &["exit-if inventory-failure 19", "cat state.json"],
                ),
                (
                    "plugin uninstall --help",
                    &["exit-if old-cli 0", "print --scope --keep-data"],
                ),
                ("plugin marketplace list --json", &["cat marketplaces.json"]),
                ("plugin marketplace update synveda", &["log"]),
                (
                    "plugin uninstall synveda@synveda --scope project --keep-data",
                    &["log", "copy removed.json state.json"],
                ),
                (
                    "plugin install synveda@synveda --scope project",
                    &[
                        "log",
                        "exit-if install-failure 20",
                        "copy installed.json state.json",
                    ],
                ),
            ],
        );
        fixture
    }

    fn write(&self, file: &str, value: &Value) {
        std::fs::write(self.root.join(file), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn command(&self, operation: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
        command
            .current_dir(self.project.join("nested"))
            .env("PATH", self.root.join("bin"))
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("SYNVEDA_FAKE_VENDOR", &self.root)
            .args([
                "plugin",
                operation,
                "--client",
                "claude-code",
                "--scope",
                "project",
            ]);
        if operation == "install" {
            command.arg("--from").arg(&self.bundle);
        }
        command
    }

    fn mutations(&self) -> String {
        std::fs::read_to_string(self.root.join("mutations")).unwrap_or_default()
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join("state.json")).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn scoped_uninstall_preserves_other_registrations_and_repeats_without_mutation() {
    let fixture = Fixture::new("uninstall");
    let output = fixture.command("uninstall").output().unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert_eq!(
        fixture.mutations(),
        format!(
            "{}: plugin uninstall synveda@synveda --scope project --keep-data\n",
            fixture.project.display()
        )
    );
    assert_eq!(fixture.state().as_array().unwrap().len(), 4);
    let again = fixture.command("uninstall").output().unwrap();
    assert!(again.status.success(), "{}", output_text(&again));
    assert_eq!(fixture.mutations().lines().count(), 1);
    assert!(output_text(&output).contains("Restart Claude to unload"));
}

#[test]
fn current_install_is_idempotent_and_force_replaces_only_requested_scope() {
    let fixture = Fixture::new("replace");
    let output = fixture.command("install").output().unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert!(fixture.mutations().is_empty());
    assert!(output_text(&output).contains("review required"));
    let forced = fixture.command("install").arg("--force").output().unwrap();
    assert!(forced.status.success(), "{}", output_text(&forced));
    let mutations = fixture.mutations();
    assert_eq!(mutations.lines().count(), 3);
    assert!(mutations.contains("uninstall synveda@synveda --scope project --keep-data"));
    assert!(mutations.ends_with("plugin install synveda@synveda --scope project\n"));
    assert_eq!(fixture.state(), json!(fixture.before));
}

#[test]
fn failed_or_ambiguous_inventory_and_foreign_marketplace_never_mutate() {
    let fixture = Fixture::new("inventory");
    std::fs::write(fixture.root.join("inventory-failure"), "").unwrap();
    for operation in ["install", "uninstall"] {
        let output = fixture.command(operation).output().unwrap();
        assert!(!output.status.success());
        assert!(output_text(&output).contains("inventory failed"));
    }
    std::fs::remove_file(fixture.root.join("inventory-failure")).unwrap();
    fixture.write("state.json", &json!({"unknown":"format"}));
    assert!(
        !fixture
            .command("uninstall")
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut duplicate = fixture.before.clone();
    duplicate.push(duplicate[2].clone());
    fixture.write("state.json", &json!(duplicate));
    let output = fixture.command("uninstall").output().unwrap();
    assert!(output_text(&output).contains("ambiguous"));
    let mut missing_owner = fixture.before.clone();
    missing_owner[2]["projectPath"] = Value::Null;
    fixture.write("state.json", &json!(missing_owner));
    let output = fixture.command("uninstall").output().unwrap();
    assert!(output_text(&output).contains("project ownership"));
    fixture.write("state.json", &json!(fixture.before));
    fixture.write(
        "marketplaces.json",
        &json!([{"name":"synveda","source":"github","path":null}]),
    );
    let output = fixture.command("install").output().unwrap();
    assert!(output_text(&output).contains("another source"));
    assert!(fixture.mutations().is_empty());
}

#[test]
fn missing_or_unsupported_vendor_cli_does_not_claim_successful_removal() {
    let fixture = Fixture::new("unavailable");
    std::fs::write(fixture.root.join("old-cli"), "").unwrap();
    let output = fixture.command("uninstall").output().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("cannot preserve scope"));
    std::fs::remove_file(
        fixture
            .root
            .join("bin")
            .join(format!("claude{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
    let output = fixture.command("uninstall").output().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("state cannot be verified"));
    assert!(fixture.mutations().is_empty());
    assert_eq!(fixture.state(), json!(fixture.before));
}

#[test]
fn partial_replacement_reports_repair_and_detects_other_scope_damage() {
    let fixture = Fixture::new("partial");
    std::fs::write(fixture.root.join("install-failure"), "").unwrap();
    let output = fixture.command("install").arg("--force").output().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("setup may be partial in scope project"));
    assert_eq!(fixture.state().as_array().unwrap().len(), 4);
    std::fs::remove_file(fixture.root.join("install-failure")).unwrap();
    let repair = fixture.command("install").arg("--force").output().unwrap();
    assert!(repair.status.success(), "{}", output_text(&repair));
    assert_eq!(fixture.state(), json!(fixture.before));
    fixture.write("removed.json", &json!([]));
    let output = fixture.command("uninstall").output().unwrap();
    assert!(!output.status.success());
    assert!(output_text(&output).contains("changed another registration"));
}

#[cfg(windows)]
#[test]
fn windows_runs_only_the_native_vendor_executable() {
    let fixture = Fixture::new("shim");
    let bin = fixture.root.join("bin");
    std::fs::remove_file(bin.join("claude.exe")).unwrap();
    // npm installs a cmd/ps1 pair and an extensionless POSIX script.
    for shim in ["claude.cmd", "claude.ps1", "claude"] {
        std::fs::write(bin.join(shim), "exit 0\r\n").unwrap();
    }
    for operation in ["install", "uninstall"] {
        let output = fixture.command(operation).output().unwrap();
        assert!(!output.status.success(), "{operation}");
        assert!(
            output_text(&output).contains("native claude.exe is not on PATH"),
            "{}",
            output_text(&output)
        );
    }
    assert!(fixture.mutations().is_empty());
    assert_eq!(fixture.state(), json!(fixture.before));
}

#[cfg(windows)]
#[test]
fn windows_project_path_spelling_variants_select_the_same_scope() {
    let fixture = Fixture::new("spelling");
    // Claude may report the repository with other casing or separators.
    let variant = fixture
        .project
        .to_str()
        .unwrap()
        .to_lowercase()
        .replace('\\', "/");
    let mut state = fixture.before.clone();
    state[2]["projectPath"] = json!(variant);
    fixture.write("state.json", &json!(state));
    fixture.write(
        "removed.json",
        &json!(
            state
                .iter()
                .filter(|entry| entry["projectPath"] != json!(variant))
                .collect::<Vec<_>>()
        ),
    );
    let output = fixture.command("uninstall").output().unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
    assert_eq!(fixture.mutations().lines().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_installed_client_is_selected_through_current_json() {
    use sha2::{Digest, Sha256};

    let fixture = Fixture::new("installed");
    let home = fixture.root.join("SynvedaClient");
    let manifest = br#"{"version":"fixture"}"#;
    let digest = format!("{:x}", Sha256::digest(manifest));
    let release = home.join("client").join("releases").join(&digest);
    std::fs::create_dir_all(release.join("plugin").join(".claude-plugin")).unwrap();
    std::fs::write(release.join("client.json"), manifest).unwrap();
    std::fs::write(
        release
            .join("plugin")
            .join(".claude-plugin")
            .join("marketplace.json"),
        "{}",
    )
    .unwrap();
    std::fs::write(
        home.join("client").join("current.json"),
        format!(r#"{{"schema_version":1,"release":"releases/{digest}"}}"#),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_synveda"))
        .current_dir(fixture.project.join("nested"))
        .env("PATH", fixture.root.join("bin"))
        .env("SYNVEDA_HOME", &home)
        .env("XDG_CONFIG_HOME", fixture.root.join("config"))
        .env("XDG_STATE_HOME", fixture.root.join("state"))
        .env("SYNVEDA_FAKE_VENDOR", &fixture.root)
        .args([
            "plugin",
            "install",
            "--client",
            "claude-code",
            "--scope",
            "project",
            "--dry-run",
        ])
        .output()
        .unwrap();
    let text = output_text(&output);
    assert!(output.status.success(), "{text}");
    assert!(
        text.contains(&release.join("plugin").display().to_string()),
        "{text}"
    );
    assert!(
        text.contains("SetEnvironmentVariable('SYNVEDA_CLI'"),
        "{text}"
    );
    assert!(fixture.mutations().is_empty());
}

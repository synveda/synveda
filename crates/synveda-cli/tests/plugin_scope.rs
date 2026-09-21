//! OPS-12: native installer process boundaries, without touching a user's
//! Claude configuration. These fixtures prove commands and reconciliation;
//! real vendor registration/loading is recorded separately.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

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
        let root = root.canonicalize().unwrap();
        let project = project.canonicalize().unwrap();
        let bundle = bundle.canonicalize().unwrap();
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
        let driver = fixture.root.join("bin/claude");
        std::fs::write(
            &driver,
            r#"#!/bin/sh
set -eu
case "$*" in
  'plugin list --json')
    test ! -e "$PLUGIN_FIXTURE/inventory-failure" || exit 19
    /bin/cat "$PLUGIN_FIXTURE/state.json" ;;
  'plugin uninstall --help')
    test ! -e "$PLUGIN_FIXTURE/old-cli" || exit 0
    printf '%s\n' '--scope --keep-data' ;;
  'plugin marketplace list --json') /bin/cat "$PLUGIN_FIXTURE/marketplaces.json" ;;
  'plugin marketplace update synveda')
    printf '%s\n' "$PWD: $*" >> "$PLUGIN_FIXTURE/mutations" ;;
  'plugin uninstall synveda@synveda --scope project --keep-data')
    printf '%s\n' "$PWD: $*" >> "$PLUGIN_FIXTURE/mutations"
    /bin/cp "$PLUGIN_FIXTURE/removed.json" "$PLUGIN_FIXTURE/state.json" ;;
  'plugin install synveda@synveda --scope project')
    printf '%s\n' "$PWD: $*" >> "$PLUGIN_FIXTURE/mutations"
    test ! -e "$PLUGIN_FIXTURE/install-failure" || exit 20
    /bin/cp "$PLUGIN_FIXTURE/installed.json" "$PLUGIN_FIXTURE/state.json" ;;
  *) printf 'unexpected native command: %s\n' "$*" >&2; exit 99 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(driver, std::fs::Permissions::from_mode(0o700)).unwrap();
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
            .env("PLUGIN_FIXTURE", &self.root)
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
    std::fs::remove_file(fixture.root.join("bin/claude")).unwrap();
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

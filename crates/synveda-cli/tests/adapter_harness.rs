//! Managed Codex and Copilot CLI registration: repository hook files plus the
//! vendor's own `mcp add`. A scripted vendor and a loopback gateway stand in
//! for the clients and Synveda; no live harness claim.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

#[path = "support/loopback_gateway.rs"]
mod loopback_gateway;
#[path = "support/vendor.rs"]
mod vendor;
#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

use loopback_gateway::{PROJECT, gateway};

const EXE: &str = std::env::consts::EXE_SUFFIX;

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    release: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "synveda harness λ {}",
            synveda_types::TenantId::new()
        ));
        for directory in ["bin", "project/.git", "project/nested"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        #[cfg(windows)]
        windows_private::private(&root);
        let root = vendor::conventional(root.canonicalize().unwrap());
        vendor::install(&root.join("bin"), "codex");
        vendor::install(&root.join("bin"), "copilot");
        Self {
            project: root.join("project"),
            release: release(&root, "release"),
            root,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
        command
            .current_dir(self.project.join("nested"))
            .env("PATH", self.root.join("bin"))
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("SYNVEDA_FAKE_VENDOR", &self.root)
            .env_remove("SYNVEDA_GATEWAY")
            .env_remove("SYNVEDA_TOKEN");
        command
    }

    fn setup(&self) {
        let (url, served) = gateway(2);
        success(
            self.command()
                .env("SYNVEDA_GATEWAY", &url)
                .env("SYNVEDA_TOKEN", "fixture-bearer")
                .args(["setup", "--project", PROJECT, "--observation", "on"])
                .output()
                .unwrap(),
        );
        assert_eq!(served.join().unwrap(), 2);
    }

    fn adapter(&self, action: &str, client: &str) -> Command {
        let mut command = self.command();
        command.args(["adapter", action, "--client", client]);
        if action == "install" {
            command.arg("--from").arg(&self.release);
        }
        command
    }

    fn cli(&self) -> String {
        cli_of(&self.release)
    }

    fn mutations(&self) -> Vec<String> {
        std::fs::read_to_string(self.root.join("mutations"))
            .unwrap_or_default()
            .lines()
            .map(|line| line.rsplit_once(": ").unwrap().1.to_owned())
            .collect()
    }

    fn read(&self, relative: &str) -> Value {
        serde_json::from_slice(&std::fs::read(self.project.join(relative)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).ok();
    }
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn success(output: Output) -> String {
    assert!(output.status.success(), "{}", text(&output));
    text(&output)
}

fn failure(output: Output, expected: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        text(&output)
    );
    assert!(text(&output).contains(expected), "{}", text(&output));
}

/// The files `synveda adapter install --from` needs from an extracted client.
fn release(root: &Path, name: &str) -> PathBuf {
    let release = root.join(name);
    for file in [
        format!("plugin/synveda/runtime/node{EXE}"),
        "plugin/codex/dist/hook.mjs".to_owned(),
        "plugin/copilot-cli/dist/hook.mjs".to_owned(),
        format!("bin/synveda{EXE}"),
    ] {
        let path = release.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "fixture\n").unwrap();
    }
    release
}

fn cli_of(release: &Path) -> String {
    release
        .join("bin")
        .join(format!("synveda{EXE}"))
        .to_str()
        .unwrap()
        .to_owned()
}

fn rules(f: &Fixture) {
    let add = format!("mcp add synveda -- {} mcp --writes host", f.cli());
    vendor::rules(
        &f.root,
        &[(add.as_str(), &["log"]), ("mcp remove synveda", &["log"])],
    );
}

#[test]
fn copilot_install_status_repeat_and_uninstall_own_only_their_entries() {
    let f = Fixture::new();
    failure(
        f.adapter("install", "copilot-cli").output().unwrap(),
        "synveda setup",
    );
    f.setup();
    rules(&f);

    let out = success(f.adapter("install", "copilot-cli").output().unwrap());
    assert!(out.contains("Registered the `synveda` MCP server"), "{out}");
    let hooks = f.read(".github/hooks/synveda.json");
    assert_eq!(hooks["version"], 1);
    let start = &hooks["hooks"]["sessionStart"][0];
    assert_eq!(start["env"]["SYNVEDA_CLI"], f.cli());
    assert_eq!(start["args"][1], "sessionStart");
    assert_eq!(
        f.mutations(),
        [format!("mcp add synveda -- {} mcp --writes host", f.cli())]
    );

    let status = success(f.adapter("status", "copilot-cli").output().unwrap());
    assert!(status.contains("receipt matches"), "{status}");
    assert!(status.contains("registered by this command"), "{status}");

    let again = success(f.adapter("install", "copilot-cli").output().unwrap());
    assert!(again.contains("already registered"), "{again}");
    assert_eq!(
        f.mutations().len(),
        1,
        "a repeat install must not re-add MCP"
    );

    success(f.adapter("uninstall", "copilot-cli").output().unwrap());
    assert!(!f.project.join(".github/hooks/synveda.json").exists());
    assert_eq!(f.mutations().last().unwrap(), "mcp remove synveda");
    success(f.adapter("uninstall", "copilot-cli").output().unwrap());
}

#[test]
fn codex_keeps_unrelated_hooks_and_refuses_edited_entries() {
    let f = Fixture::new();
    f.setup();
    rules(&f);
    let other =
        json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "./lint.sh"}]}]}});
    std::fs::create_dir_all(f.project.join(".codex")).unwrap();
    std::fs::write(
        f.project.join(".codex/hooks.json"),
        serde_json::to_vec(&other).unwrap(),
    )
    .unwrap();

    success(f.adapter("install", "codex").output().unwrap());
    let hooks = f.read(".codex/hooks.json");
    assert_eq!(hooks["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(hooks["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"], 3);

    let mut edited = hooks.clone();
    edited["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(99);
    std::fs::write(
        f.project.join(".codex/hooks.json"),
        serde_json::to_vec(&edited).unwrap(),
    )
    .unwrap();
    failure(
        f.adapter("uninstall", "codex").output().unwrap(),
        "differs from the owned entry",
    );
    std::fs::write(
        f.project.join(".codex/hooks.json"),
        serde_json::to_vec(&hooks).unwrap(),
    )
    .unwrap();

    success(f.adapter("uninstall", "codex").output().unwrap());
    assert_eq!(f.read(".codex/hooks.json"), other);
}

#[test]
fn unowned_hook_file_and_wrong_scope_are_refused() {
    let f = Fixture::new();
    f.setup();
    rules(&f);
    std::fs::create_dir_all(f.project.join(".github/hooks")).unwrap();
    std::fs::write(f.project.join(".github/hooks/synveda.json"), b"{}").unwrap();
    failure(
        f.adapter("install", "copilot-cli").output().unwrap(),
        "no managed ownership receipt",
    );
    failure(
        f.command()
            .args(["adapter", "install", "--client", "codex", "--scope", "user"])
            .output()
            .unwrap(),
        "--scope project",
    );
    assert!(f.mutations().is_empty());
}

#[test]
fn an_upgraded_release_refreshes_owned_hooks_and_the_mcp_entry() {
    let f = Fixture::new();
    f.setup();
    let next = release(&f.root, "release-next");
    let first = format!("mcp add synveda -- {} mcp --writes host", f.cli());
    let second = format!("mcp add synveda -- {} mcp --writes host", cli_of(&next));
    vendor::rules(
        &f.root,
        &[
            (first.as_str(), &["log"]),
            (second.as_str(), &["log"]),
            ("mcp remove synveda", &["log"]),
        ],
    );
    success(f.adapter("install", "copilot-cli").output().unwrap());
    success(
        f.command()
            .args(["adapter", "install", "--client", "copilot-cli", "--from"])
            .arg(&next)
            .output()
            .unwrap(),
    );
    let hooks = f.read(".github/hooks/synveda.json");
    assert_eq!(
        hooks["hooks"]["agentStop"][0]["env"]["SYNVEDA_CLI"],
        cli_of(&next)
    );
    assert_eq!(
        f.mutations(),
        [first, "mcp remove synveda".to_owned(), second]
    );
    let status = success(f.adapter("status", "copilot-cli").output().unwrap());
    assert!(status.contains("receipt matches"), "{status}");
}

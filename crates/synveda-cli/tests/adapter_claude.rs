//! ADR-0116: managed Claude Code registration through the native
//! vendor CLI, on Unix and native Windows. A scripted vendor and a loopback
//! gateway stand in for Claude and Synveda; no live harness claim.

use std::path::PathBuf;
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

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    marketplace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "synveda adapter λ {}",
            synveda_types::TenantId::new()
        ));
        for directory in ["bin", "project/.git", "project/nested", "claude"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        #[cfg(windows)]
        windows_private::private(&root);
        let root = vendor::conventional(root.canonicalize().unwrap());
        vendor::install(&root.join("bin"), "claude");
        Self {
            project: root.join("project"),
            marketplace: root.join("marketplace"),
            root,
        }
    }

    fn put(&self, name: &str, value: &Value) {
        let path = self.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_synveda"));
        command
            .current_dir(self.project.join("nested"))
            .env("PATH", self.root.join("bin"))
            .env("HOME", &self.root)
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("SYNVEDA_FAKE_VENDOR", &self.root)
            .env_remove("SYNVEDA_GATEWAY")
            .env_remove("SYNVEDA_TOKEN");
        command
    }

    fn adapter(&self, action: &str) -> Command {
        let mut command = self.command();
        command.args([
            "adapter",
            action,
            "--client",
            "claude-code",
            "--scope",
            "project",
        ]);
        if action == "install" {
            command.arg("--from").arg(&self.marketplace);
        }
        command
    }

    fn receipts(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.root.join("config/synveda/consumer"))
            .into_iter()
            .flatten()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("adapter-")
            })
            .collect()
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join("native-state.json")).unwrap())
            .unwrap()
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

#[test]
fn managed_claude_resumes_partial_install_and_removes_only_matching_native_scope() {
    use sha2::{Digest, Sha256};

    let f = Fixture::new();
    let (url, served) = gateway(2);
    success(
        f.command()
            .env("SYNVEDA_GATEWAY", &url)
            .env("SYNVEDA_TOKEN", "fixture-bearer")
            .args(["setup", "--project", PROJECT])
            .output()
            .unwrap(),
    );
    assert_eq!(served.join().unwrap(), 2);
    f.put(
        "marketplace/.claude-plugin/marketplace.json",
        &json!({"name":"synveda","plugins":[{"name":"synveda","source":"./synveda"}]}),
    );
    f.put(
        "marketplace/synveda/.claude-plugin/plugin.json",
        &json!({"name":"synveda","version":env!("CARGO_PKG_VERSION")}),
    );
    f.put(
        "marketplaces.json",
        &json!([{"name":"synveda","source":"directory","path":f.marketplace}]),
    );
    let other =
        json!({"id":"other@other","version":"1","scope":"user","enabled":true,"projectPath":null});
    let ours = json!({"id":"synveda@synveda","version":env!("CARGO_PKG_VERSION"),"scope":"project","enabled":true,"projectPath":f.project});
    f.put("native-state.json", &json!([other]));
    f.put("native-installed.json", &json!([other, ours]));
    f.put("native-removed.json", &json!([other]));
    vendor::rules(
        &f.root,
        &[
            ("plugin list --json", &["cat native-state.json"]),
            ("plugin uninstall --help", &["print --scope --keep-data"]),
            ("plugin marketplace list --json", &["cat marketplaces.json"]),
            ("plugin marketplace update synveda", &["log"]),
            (
                "plugin install synveda@synveda --scope project",
                &[
                    "copy native-installed.json native-state.json",
                    "exit-if native-failure 20",
                ],
            ),
            (
                "plugin uninstall synveda@synveda --scope project --keep-data",
                &["copy native-removed.json native-state.json"],
            ),
        ],
    );
    failure(
        f.adapter("install").output().unwrap(),
        "consumer hook contract",
    );
    assert!(f.receipts().is_empty());
    let config = b"export const managed_observation = true;\n";
    std::fs::create_dir_all(f.marketplace.join("synveda/dist")).unwrap();
    std::fs::write(f.marketplace.join("synveda/dist/config.mjs"), config).unwrap();
    f.put(
        "marketplace/synveda/consumer-setup.json",
        &json!({"version":1,"contract":"ADR-0116","config_sha256":format!("{:x}",Sha256::digest(config))}),
    );
    std::fs::write(f.root.join("native-failure"), "").unwrap();
    failure(
        f.adapter("install").output().unwrap(),
        "setup may be partial",
    );
    assert_eq!(f.receipts().len(), 1);
    // The receipt names the marketplace in the spelling Claude reports.
    let receipt = std::fs::read_to_string(&f.receipts()[0]).unwrap();
    assert!(!receipt.contains(r"\\\\?\\"), "{receipt}");
    std::fs::remove_file(f.root.join("native-failure")).unwrap();
    success(f.adapter("install").output().unwrap());
    assert!(success(f.adapter("status").output().unwrap()).contains("receipt matches"));
    let mut changed = ours.clone();
    changed["version"] = json!("foreign");
    f.put("native-state.json", &json!([other, changed]));
    failure(
        f.adapter("uninstall").output().unwrap(),
        "differs from the owned entry",
    );
    f.put("native-state.json", &json!([other, ours]));
    success(f.adapter("uninstall").output().unwrap());
    assert_eq!(f.state(), json!([other]));
    assert!(f.receipts().is_empty());
}

#[test]
fn managed_claude_refuses_user_scope_and_foreign_configuration() {
    let f = Fixture::new();
    failure(
        f.command()
            .args([
                "adapter",
                "install",
                "--client",
                "claude-code",
                "--scope",
                "user",
            ])
            .output()
            .unwrap(),
        "requires project or local",
    );
    failure(
        f.command()
            .args([
                "adapter", "install", "--client", "cursor", "--scope", "project", "--config",
            ])
            .arg(f.root.join("foreign.json"))
            .output()
            .unwrap(),
        "inside this repository",
    );
}

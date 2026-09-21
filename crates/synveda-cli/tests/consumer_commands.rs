//! OPS-12 built-CLI boundary tests. Docker/vendor fixtures do not claim live
//! deployment or harness qualification; every file belongs to this test.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};

const PROJECT: &str = "0198e4c1-0000-7000-8000-000000000002";
const WORKSPACE: &str = "0198e4c1-0000-7000-8000-000000000001";

struct Fixture {
    root: PathBuf,
    bundle: PathBuf,
    project: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "synveda consumer λ {}",
            synveda_types::TenantId::new()
        ));
        for directory in [
            "bin",
            "bundle/deploy/compose",
            "project/.git",
            "project/nested",
        ] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let fixture = Self {
            bundle: root.join("bundle"),
            project: root.join("project"),
            root,
        };
        fixture.put("bundle/environment.json", &json!({"schema_version":1,"release_version":env!("CARGO_PKG_VERSION"),"deployment_contract":"CPR-45/ADR-0102"}));
        fixture.put("bundle/evaluation.json", &json!({"port":8080}));
        fixture.put("bundle/compose.yaml", &json!({"name":"synveda-local"}));
        fixture.put(
            "bundle/deploy/compose/consumer-runtime.yaml",
            &json!({"services":{}}),
        );
        let driver = fixture.root.join("bin/docker");
        std::fs::write(&driver, r#"#!/bin/sh
set -eu
test -z "${COMPOSE_FILE:-}${COMPOSE_PROFILES:-}${COMPOSE_ENV_FILES:-}${SYNVEDA_TOKEN:-}${SYNVEDA_RECOVERY_SOURCE:-}"
case "$1 $2" in
  'context inspect') printf 'unix:///fixture/docker.sock\n' ;;
  'info --format')
    if test -f "$HOME/other-engine"; then printf 'linux other\n'; else printf 'linux fixture\n'; fi ;;
  'compose version') printf '2.38.2\n' ;;
  'ps -aq'|'volume ls'|'network ls') if test -f "$HOME/retained"; then printf 'retained-resource\n'; fi ;;
  'compose --env-file')
    printf '%s\n' "$@" >> "$HOME/commands"
    for arg do
      if test "$arg" = up; then
        : > "$HOME/retained"
        test ! -f "$HOME/fail-start" || exit 23
      fi
    done ;;
  *) exit 99 ;;
esac
"#).unwrap();
        std::fs::set_permissions(driver, std::fs::Permissions::from_mode(0o700)).unwrap();
        fixture
    }
    fn put(&self, name: &str, value: &Value) {
        let path = self.root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_synveda"));
        cmd.env_clear()
            .current_dir(self.project.join("nested"))
            .env("PATH", self.root.join("bin"))
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"));
        cmd
    }
    fn lifecycle(&self, name: &str) -> Command {
        let mut cmd = self.command();
        cmd.args([name, "--bundle"]).arg(&self.bundle);
        cmd
    }
    fn adapter(&self, name: &str) -> Command {
        let mut cmd = self.command();
        cmd.args([
            "adapter", name, "--client", "zed", "--scope", "project", "--config",
        ])
        .arg(self.project.join(".zed/settings.json"));
        cmd
    }
    fn setup(&self, gateway: &Gateway, extra: &[&str]) -> Output {
        self.command()
            .env("SYNVEDA_GATEWAY", &gateway.url)
            .env("SYNVEDA_TOKEN", "fixture-bearer")
            .args(["setup", "--project", PROJECT])
            .args(extra)
            .output()
            .unwrap()
    }
    fn receipts(&self, kind: &str) -> Vec<PathBuf> {
        std::fs::read_dir(self.root.join("config/synveda/consumer"))
            .into_iter()
            .flatten()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(kind)
            })
            .collect()
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

struct Gateway {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Gateway {
    fn new(deny: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") {
                    let mut part = [0];
                    stream.read_exact(&mut part).unwrap();
                    bytes.push(part[0]);
                    assert!(bytes.len() < 8192);
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("GET /v1/"));
                assert!(request.contains("authorization: Bearer fixture-bearer"));
                let body = if deny {
                    json!({"kind":"policy_denied","reason":"no policy permits it"})
                } else if request.starts_with(&format!("GET /v1/projects/{PROJECT} ")) {
                    json!({"id":PROJECT,"workspace_id":WORKSPACE,"status":"active"})
                } else if request.starts_with(&format!("GET /v1/workspaces/{WORKSPACE} ")) {
                    json!({"id":WORKSPACE,"status":"active"})
                } else if request.starts_with("GET /v1/me ") {
                    json!({"projects":[]})
                } else {
                    panic!("unexpected request")
                };
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", if deny { "403 Forbidden" } else { "200 OK" }, body.len(), body).unwrap();
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

#[test]
fn native_lifecycle_retries_owned_state_retains_volumes_and_clears_ambient_overrides() {
    let f = Fixture::new();
    success(f.lifecycle("up").arg("--dry-run").output().unwrap());
    assert!(f.receipts("deployment-").is_empty());
    assert!(!f.root.join("commands").exists());
    std::fs::write(f.root.join("fail-start"), "").unwrap();
    failure(
        f.lifecycle("up").output().unwrap(),
        "state and receipt were retained",
    );
    assert_eq!(f.receipts("deployment-").len(), 1);
    std::fs::remove_file(f.root.join("fail-start")).unwrap();
    success(
        f.lifecycle("up")
            .env("COMPOSE_FILE", "foreign.yaml")
            .env("COMPOSE_PROFILES", "foreign")
            .env("COMPOSE_ENV_FILES", "foreign.env")
            .env("SYNVEDA_TOKEN", "never-forward")
            .output()
            .unwrap(),
    );
    success(f.lifecycle("status").output().unwrap());
    success(f.lifecycle("logs").args(["--tail", "17"]).output().unwrap());
    success(f.lifecycle("doctor").output().unwrap());
    success(f.lifecycle("down").output().unwrap());
    let commands = std::fs::read_to_string(f.root.join("commands")).unwrap();
    assert!(commands.contains("--wait-timeout\n600\n"));
    assert!(commands.contains("logs\n--no-color\n--tail\n17\n"));
    assert!(commands.contains("--profile\n*\ndown\n"));
    assert!(!commands.contains("--volumes"));
    assert!(f.root.join("retained").exists());
    let receipt = &f.receipts("deployment-")[0];
    assert_eq!(
        std::fs::metadata(receipt).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !std::fs::read_to_string(receipt)
            .unwrap()
            .contains("never-forward")
    );
}

#[test]
fn native_lifecycle_refuses_unowned_resources_bundle_drift_and_other_engines() {
    let f = Fixture::new();
    std::fs::write(f.root.join("retained"), "").unwrap();
    for command in ["up", "down"] {
        failure(
            f.lifecycle(command).output().unwrap(),
            "without this CLI's ownership receipt",
        );
    }
    assert!(!f.root.join("commands").exists());
    std::fs::remove_file(f.root.join("retained")).unwrap();
    success(f.lifecycle("up").output().unwrap());
    let before = std::fs::read(f.root.join("commands")).unwrap();
    std::fs::write(f.root.join("other-engine"), "").unwrap();
    failure(f.lifecycle("down").output().unwrap(), "receipt differs");
    std::fs::remove_file(f.root.join("other-engine")).unwrap();
    f.put("bundle/evaluation.json", &json!({"port":8081}));
    failure(f.lifecycle("up").output().unwrap(), "receipt differs");
    assert_eq!(std::fs::read(f.root.join("commands")).unwrap(), before);
    failure(
        f.lifecycle("up")
            .args(["--project-name", "foreign"])
            .output()
            .unwrap(),
        "project must be",
    );
}

#[test]
fn setup_validates_public_api_preserves_other_settings_and_defaults_observation_off() {
    let f = Fixture::new();
    let gateway = Gateway::new(false);
    f.put(
        "project/.synveda/config.json",
        &json!({"budget_tokens":900}),
    );
    let path = f.project.join(".synveda/config.json");
    let before = std::fs::read(&path).unwrap();
    success(f.setup(&gateway, &["--dry-run"]));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(f.receipts("setup-").is_empty());
    success(f.setup(&gateway, &[]));
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        config,
        json!({"budget_tokens":900,"project_id":PROJECT,"workspace_id":WORKSPACE,"observe":false,"managed_observation":true})
    );
    success(f.setup(&gateway, &["--observation", "on"]));
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(config["observe"], true);
    assert_eq!(f.receipts("setup-").len(), 1);
    assert!(
        !std::fs::read_to_string(&f.receipts("setup-")[0])
            .unwrap()
            .contains("fixture-bearer")
    );
    // Bare setup is a read-only public inventory, even with the default choice.
    success(
        f.command()
            .env("SYNVEDA_GATEWAY", &gateway.url)
            .env("SYNVEDA_TOKEN", "fixture-bearer")
            .arg("setup")
            .output()
            .unwrap(),
    );
}

#[test]
fn setup_denial_and_conflicting_or_linked_config_never_change_selection() {
    let f = Fixture::new();
    let denied = Gateway::new(true);
    failure(f.setup(&denied, &[]), "403");
    assert!(!f.project.join(".synveda").exists());
    assert!(f.receipts("setup-").is_empty());
    let gateway = Gateway::new(false);
    f.put(
        "project/.synveda/config.json",
        &json!({"project_id":"someone-else", "observe":true}),
    );
    let path = f.project.join(".synveda/config.json");
    let before = std::fs::read(&path).unwrap();
    failure(f.setup(&gateway, &[]), "conflicts");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_file(&path).unwrap();
    f.put("private.json", &json!({"secret":"do-not-replace"}));
    symlink(f.root.join("private.json"), &path).unwrap();
    assert!(!f.setup(&gateway, &[]).status.success());
    assert!(
        std::fs::read_to_string(f.root.join("private.json"))
            .unwrap()
            .contains("do-not-replace")
    );
}

#[test]
fn managed_mcp_round_trip_preserves_unrelated_jsonc_and_refuses_entry_drift() {
    let f = Fixture::new();
    let gateway = Gateway::new(false);
    success(f.setup(&gateway, &[]));
    let path = f.project.join(".zed/settings.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = "{\n  // keep this comment\n  \"context_servers\": {\"other\": {\"command\": \"keep\"}},\n  \"theme\": \"mine\"\n}\n";
    std::fs::write(&path, before).unwrap();
    success(f.adapter("install").arg("--dry-run").output().unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    success(f.adapter("install").output().unwrap());
    let installed = std::fs::read(&path).unwrap();
    success(f.adapter("install").output().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), installed);
    assert!(success(f.adapter("status").output().unwrap()).contains("receipt matches"));
    let changed = String::from_utf8(installed.clone())
        .unwrap()
        .replace("--writes", "--foreign");
    std::fs::write(&path, &changed).unwrap();
    failure(
        f.adapter("uninstall").output().unwrap(),
        "differs from the owned entry",
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    std::fs::write(&path, installed).unwrap();
    success(f.adapter("uninstall").output().unwrap());
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("// keep this comment"));
    assert!(after.contains("\"other\": {\"command\": \"keep\"}"));
    assert!(after.contains("\"theme\": \"mine\""));
    assert!(!after.contains("\"synveda\""));
    success(f.adapter("uninstall").output().unwrap());
    assert!(f.receipts("adapter-").is_empty());
    assert!(f.project.join(".synveda/config.json").is_file());
}

#[test]
fn managed_adapter_refuses_unowned_entries_and_unsupported_scope() {
    let f = Fixture::new();
    let gateway = Gateway::new(false);
    success(f.setup(&gateway, &[]));
    f.put("project/.zed/settings.json", &json!({"context_servers":{"synveda":{"command":"foreign","env":{"SECRET":"never-print-this"}}}}));
    for action in ["install", "uninstall"] {
        let output = f.adapter(action).output().unwrap();
        assert!(!text(&output).contains("never-print-this"));
        assert!(!output.status.success());
        assert!(text(&output).contains("ownership receipt"));
    }
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
                "adapter", "install", "--client", "zed", "--scope", "project", "--config",
            ])
            .arg(f.root.join("foreign.json"))
            .output()
            .unwrap(),
        "inside this repository",
    );
}

#[test]
fn lifecycle_waits_for_the_shared_os_lock_and_rechecks_state() {
    let f = Fixture::new();
    let directory = f.root.join("config/synveda/consumer");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("operations.lock");
    let lock = std::fs::File::create(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    lock.lock().unwrap();
    let mut child = f
        .lifecycle("up")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(child.try_wait().unwrap().is_none());
    assert!(!f.root.join("commands").exists());
    // The waiter must inspect this resource after it acquires the lock.
    std::fs::write(f.root.join("retained"), "").unwrap();
    drop(lock);
    failure(
        child.wait_with_output().unwrap(),
        "without this CLI's ownership receipt",
    );
}

#[test]
fn managed_claude_resumes_partial_install_and_removes_only_matching_native_scope() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let gateway = Gateway::new(false);
    success(f.setup(&gateway, &[]));
    let marketplace = f.root.join("marketplace");
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
        &json!([{"name":"synveda","source":"directory","path":marketplace}]),
    );
    let other =
        json!({"id":"other@other","version":"1","scope":"user","enabled":true,"projectPath":null});
    let ours = json!({"id":"synveda@synveda","version":env!("CARGO_PKG_VERSION"),"scope":"project","enabled":true,"projectPath":f.project});
    f.put("native-state.json", &json!([other]));
    f.put("native-installed.json", &json!([other, ours]));
    f.put("native-removed.json", &json!([other]));
    let driver = f.root.join("bin/claude");
    std::fs::write(
        &driver,
        r#"#!/bin/sh
set -eu
case "$*" in
 'plugin list --json') /bin/cat "$HOME/native-state.json" ;;
 'plugin uninstall --help') printf '%s\n' '--scope --keep-data' ;;
 'plugin marketplace list --json') /bin/cat "$HOME/marketplaces.json" ;;
 'plugin marketplace update synveda') : ;;
 'plugin install synveda@synveda --scope project')
   /bin/cp "$HOME/native-installed.json" "$HOME/native-state.json"
   test ! -f "$HOME/native-failure" || exit 20 ;;
 'plugin uninstall synveda@synveda --scope project --keep-data')
   /bin/cp "$HOME/native-removed.json" "$HOME/native-state.json" ;;
 *) exit 99 ;;
esac
"#,
    )
    .unwrap();
    std::fs::set_permissions(driver, std::fs::Permissions::from_mode(0o700)).unwrap();
    let command = |action: &str| {
        let mut cmd = f.command();
        cmd.args([
            "adapter",
            action,
            "--client",
            "claude-code",
            "--scope",
            "project",
        ]);
        if action == "install" {
            cmd.arg("--from").arg(&marketplace);
        }
        cmd
    };
    failure(
        command("install").output().unwrap(),
        "updated OPS-12 plugin bundle",
    );
    assert!(f.receipts("adapter-").is_empty());
    let config = b"export const managed_observation = true;\n";
    std::fs::create_dir_all(marketplace.join("synveda/dist")).unwrap();
    std::fs::write(marketplace.join("synveda/dist/config.mjs"), config).unwrap();
    f.put("marketplace/synveda/consumer-setup.json", &json!({"version":1,"contract":"OPS-12/ADR-0116","config_sha256":format!("{:x}",Sha256::digest(config))}));
    std::fs::write(f.root.join("native-failure"), "").unwrap();
    failure(command("install").output().unwrap(), "setup may be partial");
    assert_eq!(f.receipts("adapter-").len(), 1);
    std::fs::remove_file(f.root.join("native-failure")).unwrap();
    success(command("install").output().unwrap());
    assert!(success(command("status").output().unwrap()).contains("receipt matches"));
    let mut changed = ours.clone();
    changed["version"] = json!("foreign");
    f.put("native-state.json", &json!([other, changed]));
    failure(
        command("uninstall").output().unwrap(),
        "differs from the owned entry",
    );
    f.put("native-state.json", &json!([other, ours]));
    success(command("uninstall").output().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(f.root.join("native-state.json")).unwrap())
            .unwrap(),
        json!([other])
    );
    assert!(f.receipts("adapter-").is_empty());
}

//! Codex and Copilot CLI registration: repository hook files that run the
//! installed client's private Node, plus the vendor's own `mcp add`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::{credentials::DEFAULT_PROFILE, local_state, setup::Selection};

const SERVER: &str = "synveda";
const VENDOR_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Harness {
    Codex,
    Copilot,
}

impl Harness {
    pub(crate) fn parse(client: &str) -> Option<Self> {
        match client {
            "codex" => Some(Self::Codex),
            "copilot-cli" => Some(Self::Copilot),
            _ => None,
        }
    }

    fn vendor(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Copilot => "copilot",
        }
    }

    fn runtime(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Copilot => "copilot-cli",
        }
    }

    /// Hook timeouts are the qualified values; Codex caps SessionEnd at 3 s.
    fn events(self) -> &'static [(&'static str, u64)] {
        match self {
            Self::Codex => &[
                ("SessionStart", 12),
                ("PreCompact", 12),
                ("Stop", 12),
                ("SessionEnd", 3),
            ],
            Self::Copilot => &[("sessionStart", 12), ("agentStop", 12), ("sessionEnd", 5)],
        }
    }

    /// The repository hook file this harness loads.
    pub(crate) fn target(self, root: &Path) -> PathBuf {
        match self {
            Self::Codex => root.join(".codex").join("hooks.json"),
            Self::Copilot => root.join(".github").join("hooks").join("synveda.json"),
        }
    }
}

/// Absolute paths inside one installed client release.
pub(crate) struct Runtime {
    node: String,
    hook: String,
    cli: String,
}

/// `from` is an extracted client archive; otherwise the selected installed
/// release. Unix paths go through the `client/current` link so an upgrade
/// keeps working hooks; Windows paths name the verified release.
pub(crate) fn runtime(harness: Harness, from: Option<&Path>) -> Result<Runtime, String> {
    let release = match from {
        Some(path) => local_state::absolute(path)?,
        None => crate::plugin::installed_client_release()?,
    };
    let (node, cli) = if cfg!(windows) {
        ("node.exe", "synveda.exe")
    } else {
        ("node", "synveda")
    };
    let node = release.join("plugin/synveda/runtime").join(node);
    let hook = release
        .join("plugin")
        .join(harness.runtime())
        .join("dist/hook.mjs");
    let cli = release.join("bin").join(cli);
    for path in [&node, &hook, &cli] {
        if !path.is_file() {
            return Err(format!(
                "{} is missing; install the Synveda client (see docs/INSTALL.md) or pass --from with an extracted client archive",
                path.display()
            ));
        }
    }
    let text = |path: PathBuf| {
        path.into_os_string()
            .into_string()
            .map_err(|_| "client release path must be UTF-8".to_owned())
    };
    Ok(Runtime {
        node: text(node)?,
        hook: text(hook)?,
        cli: text(cli)?,
    })
}

/// The hook entries this command owns, keyed by event.
pub(crate) fn desired(harness: Harness, runtime: &Runtime, selection: &Selection) -> Value {
    let profile = (selection.profile != DEFAULT_PROFILE).then_some(selection.profile.as_str());
    let mut events = Map::new();
    for &(event, timeout) in harness.events() {
        let entry = match harness {
            Harness::Codex => json!({"hooks": [{
                "type": "command",
                "command": codex_command(runtime, profile),
                "timeout": timeout,
            }]}),
            Harness::Copilot => {
                let mut env = json!({"SYNVEDA_CLI": runtime.cli});
                if let Some(profile) = profile {
                    env["SYNVEDA_PROFILE"] = json!(profile);
                }
                json!({
                    "type": "command",
                    "exec": runtime.node,
                    "args": [runtime.hook, event],
                    "env": env,
                    "timeoutSec": timeout,
                })
            }
        };
        events.insert(event.to_owned(), json!([entry]));
    }
    Value::Object(events)
}

/// Codex runs a command string. On Unix it is a POSIX shell line, so the CLI
/// path travels in the environment; Windows hooks find it through the
/// user-level `SYNVEDA_CLI` that the PowerShell installer sets.
fn codex_command(runtime: &Runtime, profile: Option<&str>) -> String {
    if cfg!(windows) {
        return format!("\"{}\" \"{}\"", runtime.node, runtime.hook);
    }
    let mut line = format!("SYNVEDA_CLI={} ", shell_quote(&runtime.cli));
    if let Some(profile) = profile {
        line.push_str(&format!("SYNVEDA_PROFILE={} ", shell_quote(profile)));
    }
    line.push_str(&format!(
        "{} {}",
        shell_quote(&runtime.node),
        shell_quote(&runtime.hook)
    ));
    line
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// What this command owns in the hook file now, if anything.
pub(crate) fn registration(harness: Harness, path: &Path) -> Result<Option<Value>, String> {
    let Some(document) = read(path)? else {
        return Ok(None);
    };
    let hooks = &document["hooks"];
    let mut owned = Map::new();
    for &(event, _) in harness.events() {
        let entries: Vec<Value> = hooks[event]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| harness == Harness::Copilot || is_ours(entry))
            .cloned()
            .collect();
        if !entries.is_empty() {
            owned.insert(event.to_owned(), Value::Array(entries));
        }
    }
    // The Copilot file is Synveda's by name, so any content counts as present.
    Ok((!owned.is_empty() || harness == Harness::Copilot).then_some(Value::Object(owned)))
}

/// A Codex matcher group whose command runs the Synveda Codex hook.
fn is_ours(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|hook| {
            hook["command"].as_str().is_some_and(|command| {
                command
                    .replace('\\', "/")
                    .contains("/plugin/codex/dist/hook.mjs")
            })
        })
    })
}

/// Write `desired`, replacing only entries this command owns.
pub(crate) fn write(harness: Harness, path: &Path, desired: &Value) -> Result<(), String> {
    let before = local_state::read(path)?;
    let document = match harness {
        Harness::Copilot => json!({"version": 1, "hooks": desired}),
        Harness::Codex => {
            let mut document = parse(before.as_deref(), path)?;
            strip_ours(&mut document, harness);
            let hooks = hooks_object(&mut document, path)?;
            for (event, entries) in desired.as_object().into_iter().flatten() {
                let list = hooks.entry(event.clone()).or_insert_with(|| json!([]));
                let list = list
                    .as_array_mut()
                    .ok_or_else(|| format!("{}: hooks.{event} must be an array", path.display()))?;
                list.extend(entries.as_array().into_iter().flatten().cloned());
            }
            document
        }
    };
    let parent = path.parent().ok_or("hook file has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    local_state::replace(path, before.as_deref(), &bytes, false)
}

/// Remove only owned entries; delete the file when nothing else remains.
pub(crate) fn remove(harness: Harness, path: &Path) -> Result<(), String> {
    let Some(before) = local_state::read(path)? else {
        return Ok(());
    };
    let mut document = parse(Some(&before), path)?;
    if harness == Harness::Copilot {
        return local_state::remove_unchanged(path, &before);
    }
    strip_ours(&mut document, harness);
    let empty = document
        .as_object()
        .is_some_and(|object| object.keys().all(|key| key == "hooks"))
        && document["hooks"].as_object().is_none_or(Map::is_empty);
    if empty {
        return local_state::remove_unchanged(path, &before);
    }
    let mut bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    local_state::replace(path, Some(&before), &bytes, false)
}

fn strip_ours(document: &mut Value, harness: Harness) {
    let Some(hooks) = document["hooks"].as_object_mut() else {
        return;
    };
    for &(event, _) in harness.events() {
        if let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            list.retain(|group| !is_ours(group));
            if list.is_empty() {
                hooks.remove(event);
            }
        }
    }
}

fn hooks_object<'a>(
    document: &'a mut Value,
    path: &Path,
) -> Result<&'a mut Map<String, Value>, String> {
    let object = document
        .as_object_mut()
        .ok_or_else(|| format!("{} must be a JSON object", path.display()))?;
    object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("{}: `hooks` must be an object", path.display()))
}

fn read(path: &Path) -> Result<Option<Value>, String> {
    local_state::read(path)?
        .map(|bytes| parse(Some(&bytes), path))
        .transpose()
}

fn parse(bytes: Option<&[u8]>, path: &Path) -> Result<Value, String> {
    let Some(bytes) = bytes.filter(|bytes| !bytes.iter().all(u8::is_ascii_whitespace)) else {
        return Ok(json!({}));
    };
    let document: Value = serde_json::from_slice(bytes).map_err(|_| {
        format!(
            "{} is not valid JSON; it was left unchanged",
            path.display()
        )
    })?;
    if !document.is_object() {
        return Err(format!("{} must be a JSON object", path.display()));
    }
    Ok(document)
}

/// The vendor command that registers the shared, unbound MCP server.
pub(crate) fn mcp_add_args(runtime: &Runtime, selection: &Selection) -> Vec<String> {
    let mut args = [
        "mcp",
        "add",
        SERVER,
        "--",
        &runtime.cli,
        "mcp",
        "--writes",
        "host",
    ]
    .map(str::to_owned)
    .to_vec();
    if selection.profile != DEFAULT_PROFILE {
        args.extend(["--profile".to_owned(), selection.profile.clone()]);
    }
    args
}

pub(crate) fn mcp_remove_args() -> Vec<String> {
    ["mcp", "remove", SERVER].map(str::to_owned).to_vec()
}

/// Run `<vendor> args`; `Err` carries the exact command to run by hand.
pub(crate) fn run_vendor(harness: Harness, args: &[String]) -> Result<(), String> {
    let manual = format!("{} {}", harness.vendor(), args.join(" "));
    let Some(program) = which(harness.vendor()) else {
        return Err(format!(
            "`{}` is not on PATH. Run: {manual}",
            harness.vendor()
        ));
    };
    let mut child = Command::new(&program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("start {}: {e}. Run: {manual}", program.display()))?;
    let deadline = Instant::now() + VENDOR_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{manual} did not finish within 30 seconds"));
            }
            Err(e) => return Err(format!("wait for {manual}: {e}")),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut detail = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        let _ = (&mut stderr).take(2048).read_to_string(&mut detail);
    }
    Err(format!("{manual} failed ({status}): {}", detail.trim()))
}

/// The vendor executable from an absolute PATH entry. Windows accepts the
/// npm `.cmd` launcher these clients ship with; std quotes its arguments
/// for cmd.exe and refuses ones it cannot quote safely.
fn which(program: &str) -> Option<PathBuf> {
    let names: &[String] = if cfg!(windows) {
        &[format!("{program}.exe"), format!("{program}.cmd")]
    } else {
        &[program.to_owned()]
    };
    let path = std::env::var_os("PATH")?;
    names.iter().find_map(|name| {
        std::env::split_paths(&path)
            .take(1024)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use synveda_types::{ProjectId, WorkspaceId};

    fn selection(profile: &str) -> Selection {
        Selection {
            profile: profile.to_owned(),
            project: ProjectId::new(),
            workspace: WorkspaceId::new(),
            observation: crate::setup::Observation::On,
        }
    }

    fn runtime() -> Runtime {
        Runtime {
            node: "/home/a b/.synveda/client/current/plugin/synveda/runtime/node".to_owned(),
            hook: "/home/a b/.synveda/client/current/plugin/codex/dist/hook.mjs".to_owned(),
            cli: "/home/a b/.synveda/client/current/bin/synveda".to_owned(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("synveda-harness-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copilot_hooks_carry_the_cli_and_profile_without_a_shell() {
        let hooks = desired(Harness::Copilot, &runtime(), &selection("work"));
        let start = &hooks["sessionStart"][0];
        assert_eq!(start["exec"], runtime().node);
        assert_eq!(start["args"], json!([runtime().hook, "sessionStart"]));
        assert_eq!(start["env"]["SYNVEDA_CLI"], runtime().cli);
        assert_eq!(start["env"]["SYNVEDA_PROFILE"], "work");
        assert_eq!(hooks["sessionEnd"][0]["timeoutSec"], 5);
    }

    #[cfg(not(windows))]
    #[test]
    fn codex_command_quotes_paths_with_spaces() {
        let hooks = desired(Harness::Codex, &runtime(), &selection(DEFAULT_PROFILE));
        let command = hooks["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(command.starts_with("SYNVEDA_CLI='/home/a b/"), "{command}");
        assert!(!command.contains("SYNVEDA_PROFILE"), "{command}");
        assert_eq!(hooks["SessionEnd"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn codex_install_keeps_other_hooks_and_uninstall_restores_them() {
        let dir = scratch("codex");
        let path = dir.join(".codex/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let other =
            json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "./lint.sh"}]}]}});
        std::fs::write(&path, serde_json::to_vec(&other).unwrap()).unwrap();

        let wanted = desired(Harness::Codex, &runtime(), &selection(DEFAULT_PROFILE));
        write(Harness::Codex, &path, &wanted).unwrap();
        assert_eq!(
            registration(Harness::Codex, &path).unwrap(),
            Some(wanted.clone())
        );
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["hooks"]["Stop"].as_array().unwrap().len(), 2);

        write(Harness::Codex, &path, &wanted).unwrap();
        let rewritten: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            rewritten, written,
            "a repeated install must not duplicate entries"
        );

        remove(Harness::Codex, &path).unwrap();
        let restored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, other);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copilot_file_is_created_and_removed_whole() {
        let dir = scratch("copilot");
        let path = Harness::Copilot.target(&dir);
        let wanted = desired(Harness::Copilot, &runtime(), &selection(DEFAULT_PROFILE));
        assert_eq!(registration(Harness::Copilot, &path).unwrap(), None);
        write(Harness::Copilot, &path, &wanted).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["version"], 1);
        assert_eq!(registration(Harness::Copilot, &path).unwrap(), Some(wanted));
        remove(Harness::Copilot, &path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_hook_json_is_refused_and_preserved() {
        let dir = scratch("invalid");
        let path = dir.join(".codex/hooks.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        let wanted = desired(Harness::Codex, &runtime(), &selection(DEFAULT_PROFILE));
        assert!(
            write(Harness::Codex, &path, &wanted)
                .unwrap_err()
                .contains("not valid JSON")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mcp_registration_is_unbound_host_writes() {
        let args = mcp_add_args(&runtime(), &selection("work"));
        assert_eq!(
            args,
            [
                "mcp",
                "add",
                "synveda",
                "--",
                &runtime().cli,
                "mcp",
                "--writes",
                "host",
                "--profile",
                "work"
            ]
        );
    }
}

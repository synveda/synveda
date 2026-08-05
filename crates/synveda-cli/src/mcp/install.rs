//! `synveda mcp install` — the client's own config, written rather than
//! documented (ADPT-2, ADR-0057 decision 10).
//!
//! The acceptance criterion is "works in Claude Desktop + one
//! non-Anthropic client", and "paste this JSON into that file" is exactly
//! where two clients diverge into a support burden nobody can test. So the
//! product writes the file, says what it wrote, and can be asked what it
//! would do first.
//!
//! # Writing another application's file
//!
//! ADR-0057 records this among the costs, and it is the reason for every
//! restraint below:
//!
//! - **One key changes.** The file is read, `mcpServers.synveda` is set,
//!   and everything else is written back as it was found. A user's other
//!   servers are not this command's to touch.
//! - **It refuses to clobber.** An existing `synveda` entry that differs
//!   is a conflict, not an opportunity: it is shown and `--force` is
//!   named. An entry that already matches is left alone and the file is
//!   not rewritten at all.
//! - **The write is atomic.** A temporary file beside the target, then a
//!   rename, so a crash cannot leave another application's config
//!   truncated. The original's permissions are carried over.
//! - **Nothing secret is written.** The generated entry names a binary and
//!   its arguments; the bearer stays where `synveda login` put it, and the
//!   server reads it per call.
//!
//! One thing this cannot preserve: **key order**. `serde_json` orders
//! object keys alphabetically unless built with `preserve_order`, and that
//! feature would unify across the workspace and change the byte order of
//! every JSON the gateway emits — including the payloads AUD-1 hash-chains.
//! A tidier diff in someone's editor is not worth reaching into the audit
//! log's hash inputs, so the keys get sorted, once, on the install that
//! first adds ours. The JSON is identical in meaning and no value moves.
//!
//! # Why the absolute path
//!
//! ADR-0057 decision 1: "the config line both AC clients get is the
//! absolute path to a binary they already have after `synveda init`". Not
//! a convenience — Claude Desktop is a GUI application launched by the
//! window manager, so it inherits none of the `PATH` a shell would have,
//! and a bare `synveda` there fails at spawn with nothing useful said.
//! `current_exe()` is the binary the user just ran, which is the one they
//! meant.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::credentials::DEFAULT_PROFILE;

/// The clients `--client` knows, and what each calls its config.
///
/// The per-client difference is the path and nothing else: both take the
/// ecosystem's `mcpServers` object, with `command` and `args`, which is
/// why decision 10's reversal trigger is "a third client arrives with
/// another config format → `install` grows a generic print-the-JSON mode
/// rather than a branch per vendor". `--print` is that mode, here before
/// the third client is, because it is also the honest answer on a platform
/// whose path we do not know.
const CLIENTS: [&str; 2] = ["claude-desktop", "cursor"];

/// The key this server takes in `mcpServers`. The same name the Claude
/// Code plugin's manifest uses, so one product is one entry everywhere.
const SERVER_KEY: &str = "synveda";

/// What `install` was asked to do.
pub struct Plan {
    /// Which client's config to write.
    pub client: String,
    /// Write this file instead of the client's own.
    pub config: Option<PathBuf>,
    /// The credential profile the generated entry names.
    pub profile: String,
    /// Report what would change and write nothing.
    pub dry_run: bool,
    /// Replace an existing `synveda` entry that differs.
    pub force: bool,
    /// Print the entry and write nothing — for a client this does not
    /// know, or a config kept somewhere unusual.
    pub print: bool,
}

/// `synveda mcp install --client <name>`.
pub fn install(plan: &Plan) -> Result<(), String> {
    let entry = entry(&plan.profile)?;

    if plan.print {
        // The whole object rather than the inner entry: what a person
        // pastes is a config file, and one that is already correctly
        // shaped saves them guessing at the wrapper.
        println!(
            "{}",
            render(&json!({ "mcpServers": { SERVER_KEY: entry } }))
        );
        return Ok(());
    }

    let path = match &plan.config {
        Some(path) => path.clone(),
        None => config_path(&plan.client)?,
    };

    let mut document = read(&path)?;
    let servers = servers_mut(&mut document, &path)?;

    match servers.get(SERVER_KEY) {
        Some(existing) if *existing == entry => {
            println!(
                "{} already names this binary; nothing to do",
                path.display()
            );
            describe(&entry);
            return Ok(());
        }
        // A differing entry is somebody's decision — an older install, a
        // hand edit, a second checkout — and overwriting it silently is
        // how a user's working setup disappears without a message.
        Some(existing) if !plan.force => {
            return Err(format!(
                "{} already has an `mcpServers.{SERVER_KEY}` entry, and it is not this one:\n\
                 \n{}\n\n\
                 it would become:\n\n{}\n\n\
                 pass --force to replace it, or --print to see the entry and place it yourself",
                path.display(),
                indent(&render(existing)),
                indent(&render(&entry)),
            ));
        }
        _ => {}
    }

    let replaced = servers
        .insert(SERVER_KEY.to_owned(), entry.clone())
        .is_some();
    let others = servers.len() - 1;

    if plan.dry_run {
        println!("would write {}", path.display());
    } else {
        write(&path, &document)?;
        println!("wrote {}", path.display());
    }
    describe(&entry);
    if replaced {
        println!("  (replaced the entry that was there)");
    }
    if others > 0 {
        // Said out loud because this command opened a file it does not
        // own: the user should be told their other servers survived.
        println!("  ({others} other MCP server(s) left as they were)");
    }
    if !plan.dry_run {
        println!("{}", restart_hint(&plan.client));
    }
    Ok(())
}

/// The entry a client will exec.
fn entry(profile: &str) -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|err| format!("locate this binary: {err}"))?;
    // Through symlinks, because a client that cannot read this path is a
    // server that never starts, and `current_exe` may hand back a shim.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let exe = exe
        .to_str()
        .ok_or_else(|| format!("this binary's path is not valid UTF-8: {}", exe.display()))?
        .to_owned();

    // `--writes tool` is stated rather than left to the default. Both
    // clients here are model-driven — nothing else in them writes, so the
    // tool must (ADR-0057 decision 6) — and a generated file should say
    // what it means so a later change of default cannot quietly take
    // `remember` away from every config already on disk.
    let mut args = vec![json!("mcp"), json!("--writes"), json!("tool")];
    if profile != DEFAULT_PROFILE {
        args.push(json!("--profile"));
        args.push(json!(profile));
    }
    Ok(json!({ "command": exe, "args": args }))
}

fn describe(entry: &Value) {
    println!(
        "  command  {}",
        entry["command"].as_str().unwrap_or_default()
    );
    let args: Vec<&str> = entry["args"]
        .as_array()
        .map(|args| args.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    println!("  args     {}", args.join(" "));
}

/// Where `client` keeps the config that names its MCP servers.
///
/// Both paths are the vendors' own, and both are per-user rather than
/// per-project on purpose: a bearer is bound to a machine by `synveda
/// login`, so the server belongs beside it rather than in a checkout that
/// travels.
fn config_path(client: &str) -> Result<PathBuf, String> {
    match client {
        // modelcontextprotocol.io, "Connect to local MCP servers": macOS
        // and Windows are the two Claude Desktop ships for, and the two
        // its documentation gives a path for.
        "claude-desktop" => {
            if cfg!(target_os = "macos") {
                Ok(home()?.join("Library/Application Support/Claude/claude_desktop_config.json"))
            } else if cfg!(windows) {
                let appdata =
                    std::env::var_os("APPDATA").ok_or_else(|| "APPDATA is not set".to_owned())?;
                Ok(PathBuf::from(appdata).join("Claude/claude_desktop_config.json"))
            } else {
                // Guessing a path on a platform the vendor documents no
                // path for is how this command writes a file nothing
                // reads. Say so, and hand over the JSON instead.
                Err(format!(
                    "Claude Desktop documents a config path for macOS and Windows only, \
                     and this is {}. Use --print and place the entry yourself, or --config \
                     <path> if you know where it lives here.",
                    std::env::consts::OS,
                ))
            }
        }
        // cursor.com/docs: `~/.cursor/mcp.json` is the global one;
        // `.cursor/mcp.json` in a project is the per-project form, which
        // `--config` reaches.
        "cursor" => Ok(home()?.join(".cursor/mcp.json")),
        other => Err(format!(
            "unknown client {other:?}; known clients are {}",
            clients(),
        )),
    }
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "neither HOME nor USERPROFILE is set".to_owned())
}

fn restart_hint(client: &str) -> &'static str {
    match client {
        "claude-desktop" => "quit Claude Desktop completely and reopen it to pick this up",
        _ => "restart Cursor to pick this up",
    }
}

// ── The file ───────────────────────────────────────────────────────────

/// The config as it stands, or an empty object when there is none yet.
fn read(path: &Path) -> Result<Value, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(err) => return Err(format!("read {}: {err}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    // A file we cannot parse is a file we must not rewrite: whatever is in
    // it, the user put it there, and a "fix" that discards it is worse
    // than a refusal.
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "{} is not valid JSON ({err}); fix it, or use --print and edit it by hand",
            path.display(),
        )
    })
}

/// The `mcpServers` object, created if absent.
fn servers_mut<'a>(
    document: &'a mut Value,
    path: &Path,
) -> Result<&'a mut Map<String, Value>, String> {
    let top = kind(document);
    let root = document.as_object_mut().ok_or_else(|| {
        format!(
            "{} holds {top} at the top level, not an object",
            path.display()
        )
    })?;
    let servers = root
        .entry("mcpServers".to_owned())
        .or_insert_with(|| json!({}));
    let found = kind(servers);
    servers.as_object_mut().ok_or_else(|| {
        format!(
            "{}'s `mcpServers` holds {found}, not an object",
            path.display()
        )
    })
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Atomically, because this is somebody else's file: a temporary beside it
/// and then a rename, so an interrupted write cannot truncate a config the
/// user's client needs to start.
fn write(path: &Path, document: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    if !parent.exists() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("create {}: {err}", parent.display()))?;
        println!("created {}", parent.display());
    }

    let body = format!("{}\n", render(document));
    let temporary = path.with_extension("synveda-tmp");
    std::fs::write(&temporary, &body)
        .map_err(|err| format!("write {}: {err}", temporary.display()))?;
    // The client's own permissions, kept: this command has no opinion
    // about how another application's config should be readable.
    if let Ok(existing) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temporary, existing.permissions());
    }
    std::fs::rename(&temporary, path).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        format!("write {}: {err}", path.display())
    })
}

fn render(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The names `--client` takes, for a usage message.
#[must_use]
pub fn clients() -> String {
    CLIENTS.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(config: &Path) -> Plan {
        Plan {
            client: "cursor".to_owned(),
            config: Some(config.to_path_buf()),
            profile: DEFAULT_PROFILE.to_owned(),
            dry_run: false,
            force: false,
            print: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("synveda-mcp-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir.join("mcp.json")
    }

    fn read_back(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("written")).expect("json")
    }

    #[test]
    fn a_fresh_config_gets_the_entry_and_nothing_else() {
        let path = scratch("fresh");
        install(&plan(&path)).expect("install");
        let written = read_back(&path);
        assert_eq!(written.as_object().expect("object").len(), 1, "{written}");
        let entry = &written["mcpServers"]["synveda"];
        assert!(
            entry["command"].as_str().expect("command").starts_with('/'),
            "a GUI client inherits no PATH, so the path must be absolute: {entry}",
        );
        assert_eq!(entry["args"], json!(["mcp", "--writes", "tool"]));
    }

    /// The property this whole module is arranged around: a user's other
    /// MCP servers are not ours to touch, and neither is anything else in
    /// a file we did not create.
    #[test]
    fn every_other_server_and_every_other_key_survives() {
        let path = scratch("preserve");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "mcpServers": {
                    "filesystem": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"] },
                    "github": { "command": "gh-mcp", "args": [], "env": { "GH_TOKEN": "secret" } },
                },
                "someOtherSetting": { "nested": [1, 2, 3] },
            }))
            .expect("json"),
        )
        .expect("seed");

        install(&plan(&path)).expect("install");

        let written = read_back(&path);
        assert_eq!(written["mcpServers"]["filesystem"]["command"], json!("npx"));
        assert_eq!(
            written["mcpServers"]["github"]["env"]["GH_TOKEN"],
            json!("secret")
        );
        assert_eq!(written["someOtherSetting"], json!({ "nested": [1, 2, 3] }));
        assert!(written["mcpServers"]["synveda"].is_object());
    }

    #[test]
    fn an_entry_that_differs_is_a_conflict_and_names_the_flag() {
        let path = scratch("conflict");
        std::fs::write(
            &path,
            json!({ "mcpServers": { "synveda": { "command": "/elsewhere/synveda", "args": ["mcp"] } } })
                .to_string(),
        )
        .expect("seed");

        let error = install(&plan(&path)).expect_err("a differing entry must not be overwritten");
        assert!(error.contains("--force"), "{error}");
        assert!(
            error.contains("/elsewhere/synveda"),
            "the conflict must show what is there: {error}"
        );

        // And the file is untouched until someone says so.
        assert_eq!(
            read_back(&path)["mcpServers"]["synveda"]["command"],
            json!("/elsewhere/synveda"),
        );

        let forced = Plan {
            force: true,
            ..plan(&path)
        };
        install(&forced).expect("--force replaces it");
        assert_ne!(
            read_back(&path)["mcpServers"]["synveda"]["command"],
            json!("/elsewhere/synveda"),
        );
    }

    /// A re-run must be free. Rewriting a file to produce identical
    /// content still churns its mtime and its key order, and a command
    /// that is safe to repeat is one people will repeat.
    #[test]
    fn an_identical_entry_rewrites_nothing() {
        let path = scratch("idempotent");
        install(&plan(&path)).expect("first");
        let first = std::fs::read_to_string(&path).expect("written");
        let stamp = std::fs::metadata(&path)
            .expect("meta")
            .modified()
            .expect("mtime");

        install(&plan(&path)).expect("second");
        assert_eq!(std::fs::read_to_string(&path).expect("written"), first);
        assert_eq!(
            std::fs::metadata(&path)
                .expect("meta")
                .modified()
                .expect("mtime"),
            stamp,
            "a second install must not touch the file at all",
        );
    }

    #[test]
    fn dry_run_reports_and_writes_nothing() {
        let path = scratch("dry");
        let dry = Plan {
            dry_run: true,
            ..plan(&path)
        };
        install(&dry).expect("dry run");
        assert!(!path.exists(), "--dry-run must not create the file");
    }

    /// Whatever is in an unparseable file, the user put it there. A
    /// command that "fixes" it by writing a fresh one has destroyed
    /// something it was never asked to.
    #[test]
    fn an_unparseable_config_is_refused_rather_than_replaced() {
        let path = scratch("garbage");
        std::fs::write(&path, "{ this is not json").expect("seed");
        let error = install(&plan(&path)).expect_err("must refuse");
        assert!(error.contains("not valid JSON"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "{ this is not json"
        );
    }

    #[test]
    fn a_config_shaped_wrongly_says_what_it_found() {
        let path = scratch("shape");
        std::fs::write(
            &path,
            json!({ "mcpServers": ["not", "an", "object"] }).to_string(),
        )
        .expect("seed");
        let error = install(&plan(&path)).expect_err("must refuse");
        assert!(error.contains("an array"), "{error}");
    }

    /// An empty file is not a broken one — a client may have created it
    /// and never written to it.
    #[test]
    fn an_empty_file_is_treated_as_no_config_yet() {
        let path = scratch("empty");
        std::fs::write(&path, "   \n").expect("seed");
        install(&plan(&path)).expect("install");
        assert!(read_back(&path)["mcpServers"]["synveda"].is_object());
    }

    #[test]
    fn a_non_default_profile_is_carried_into_the_entry() {
        let default = entry(DEFAULT_PROFILE).expect("entry");
        assert_eq!(default["args"], json!(["mcp", "--writes", "tool"]));

        let work = entry("work").expect("entry");
        assert_eq!(
            work["args"],
            json!(["mcp", "--writes", "tool", "--profile", "work"])
        );
    }

    /// Decision 6, at the one place a config file decides it: both clients
    /// this command knows are model-driven, so the entry it generates must
    /// advertise the write tool — and must say so rather than rely on a
    /// default that a later release could move.
    #[test]
    fn the_generated_entry_states_writes_tool_rather_than_assuming_it() {
        let args = entry(DEFAULT_PROFILE).expect("entry")["args"].clone();
        let args: Vec<String> = serde_json::from_value(args).expect("strings");
        let at = args
            .iter()
            .position(|arg| arg == "--writes")
            .expect("--writes is stated");
        assert_eq!(args[at + 1], "tool");
    }

    #[test]
    fn an_unknown_client_names_the_ones_it_knows() {
        let error = config_path("windsurf").expect_err("unknown");
        assert!(error.contains("claude-desktop"), "{error}");
        assert!(error.contains("cursor"), "{error}");
    }

    /// The vendors' own documented locations. Pinned because these are
    /// claims about somebody else's product: if one moves, this test is
    /// where the move gets recorded rather than where a user's config
    /// silently lands in the wrong place.
    #[test]
    fn the_client_paths_are_the_documented_ones() {
        let cursor = config_path("cursor").expect("cursor");
        assert!(cursor.ends_with(".cursor/mcp.json"), "{}", cursor.display());

        let desktop = config_path("claude-desktop");
        if cfg!(target_os = "macos") {
            let path = desktop.expect("macOS has a documented path");
            assert!(
                path.ends_with("Library/Application Support/Claude/claude_desktop_config.json"),
                "{}",
                path.display(),
            );
        } else if !cfg!(windows) {
            // Claude Desktop ships for macOS and Windows, and documents a
            // path for those two. Inventing one elsewhere writes a file
            // nothing reads.
            let error = desktop.expect_err("no path should be invented");
            assert!(error.contains("--print"), "{error}");
        }
    }
}

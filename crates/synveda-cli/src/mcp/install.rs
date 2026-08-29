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
//! - **One key changes.** The file is read, the client's own server map
//!   gains a `synveda` entry, and everything else is written back as it was
//!   found. A user's other servers are not this command's to touch. Which
//!   key that is comes from the product adapter registry: `mcpServers`
//!   for most, `context_servers` for Zed.
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
//! One thing the **`json` path** cannot preserve: **key order**.
//! `serde_json` orders object keys alphabetically unless built with
//! `preserve_order`, and that feature would unify across the workspace and
//! change the byte order of every JSON the gateway emits — including the
//! payloads AUD-1 hash-chains. A tidier diff in someone's editor is not
//! worth reaching into the audit log's hash inputs, so the keys get sorted,
//! once, on the install that first adds ours. The JSON is identical in
//! meaning and no value moves.
//!
//! The **`jsonc` path has no such caveat**, and that asymmetry is the point
//! rather than an accident. A file this command generated may be reordered
//! because nothing in it came from anywhere else; a file its owner
//! maintains may not, so that one is spliced through a concrete syntax tree
//! and every byte outside the entry — comments, blank lines, key order,
//! their indentation — comes back exactly as it went in. Which path a
//! client takes is *declared* in the registry, never sniffed from the
//! bytes: a settings file that happens to carry no comments today is still
//! not ours to reformat tomorrow.
//!
//! # Why the absolute path
//!
//! ADR-0057 decision 1 requires both AC clients to receive the absolute path
//! to the binary whose installer the user just invoked. This is not
//! a convenience — Claude Desktop is a GUI application launched by the
//! window manager, so it inherits none of the `PATH` a shell would have,
//! and a bare `synveda` there fails at spawn with nothing useful said.
//! `current_exe()` is the binary the user just ran, which is the one they
//! meant.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::credentials::DEFAULT_PROFILE;

/// The built-in client registry — data, not code.
///
/// This started as "the per-client difference is the path and nothing
/// else", and Zed ended it: it keeps its servers under `context_servers`
/// rather than the ecosystem's `mcpServers`, and its `settings.json` is
/// JSONC that its owner edits by hand. The first fix was a `match` arm per
/// vendor, which is precisely what seed §2 principle 6 forbids — "the
/// harness is a guest; supporting a new harness must never require touching
/// the core" — and the same mistake ADR-0057 decision 6 refused one level
/// up when it made `--writes` describe a capability instead of naming
/// harnesses.
///
/// So the product registry owns the built-ins and a user's own file is read
/// through the same configuration loader. This also answers decision 10's
/// reversal trigger ("a third client arrives with another config format →
/// `install` grows a generic print-the-JSON mode rather than a branch per
/// vendor") better than the trigger's own remedy did: `--print` still
/// exists, but a third client no longer needs it, and a fourth needs
/// neither it nor us.
const BUILT_IN: &str = include_str!("../../../../adapters/registry.json");

/// Where a user adds clients we have never heard of, or overrides ours.
const USER_REGISTRY: &str = ".config/synveda/mcp-clients.jsonc";

/// How a client's config file must be written back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Syntax {
    /// An app-generated file: a formatter may render it whole.
    Json,
    /// A file its owner maintains: splice the one key we own and leave
    /// every other byte alone.
    Jsonc,
}

/// One client's entry in the registry.
#[derive(Clone, Debug, serde::Deserialize)]
struct Client {
    /// The top-level key holding this client's server map.
    key: String,
    syntax: Syntax,
    restart: String,
    /// Config location by `std::env::consts::OS`, plus `any`.
    path: std::collections::BTreeMap<String, String>,
}

/// The product-level registry owns support evidence as well as configuration;
/// the CLI deliberately projects only the latter. A recipe grants no support
/// level and no tool authority.
#[derive(serde::Deserialize)]
struct BuiltInRegistry {
    clients: Vec<BuiltInClient>,
}

#[derive(serde::Deserialize)]
struct BuiltInClient {
    id: String,
    configuration: Option<Client>,
}

/// The built-in table, then the user's over the top of it.
///
/// The user's file wins on a name collision, which is what "override"
/// means: someone whose Cursor keeps its config somewhere ours does not
/// expect should be able to say so once rather than pass `--config` every
/// time.
fn registry() -> Result<std::collections::BTreeMap<String, Client>, String> {
    // The unit tests must not read the developer's home directory. A
    // machine that happens to carry an `mcp-clients.jsonc` overriding
    // `zed` would fail tests a clean machine passes, which is a test suite
    // that reports the developer rather than the code. The merge itself is
    // covered directly, by handing `merge` both halves.
    #[cfg(test)]
    let user: Option<String> = None;
    // No file is the ordinary case and not an error — which is why this
    // reads rather than checks first, and drops the read failing. A
    // registry that exists but does not parse is a different matter and
    // does fail, loudly: someone wrote it meaning it to be used.
    #[cfg(not(test))]
    let user: Option<String> = home()
        .ok()
        .and_then(|home| std::fs::read_to_string(home.join(USER_REGISTRY)).ok());
    merge(BUILT_IN, user.as_deref())
}

/// The merge itself, off the filesystem so it can be tested without a
/// `$HOME` to write into — the tests below own no user's home directory
/// and setting the variable would race every other test in this binary.
fn merge(
    built_in: &str,
    user: Option<&str>,
) -> Result<std::collections::BTreeMap<String, Client>, String> {
    let registry: BuiltInRegistry = serde_json::from_str(built_in)
        .map_err(|err| format!("the built-in client registry does not match the schema: {err}"))?;
    let mut clients: std::collections::BTreeMap<String, Client> = registry
        .clients
        .into_iter()
        .filter_map(|client| client.configuration.map(|config| (client.id, config)))
        .collect();
    if let Some(raw) = user {
        clients.extend(parse_registry(raw, &format!("~/{USER_REGISTRY}"))?);
    }
    Ok(clients)
}

fn parse_registry(
    raw: &str,
    label: &str,
) -> Result<std::collections::BTreeMap<String, Client>, String> {
    let value = parse_jsonc(raw)
        .map_err(|err| format!("{label} is not valid JSONC: {err}"))?
        .unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|err| format!("{label} does not match the schema: {err}"))
}

/// JSONC text as a plain value, for anything that only wants to read it.
fn parse_jsonc(raw: &str) -> Result<Option<Value>, String> {
    let root = jsonc_parser::cst::CstRootNode::parse(raw, &jsonc_parser::ParseOptions::default())
        .map_err(|err| err.to_string())?;
    Ok(root.to_serde_value())
}

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
#[cfg(test)]
pub fn install(plan: &Plan) -> Result<(), String> {
    install_for(plan, None, None)
}

/// Install a configuration that pins the runtime workspace/project selection.
///
/// Kept beside the ordinary entry point so existing callers and tests retain
/// the default no-selection shape, while the CLI can faithfully carry its
/// outer `mcp --workspace/--project` arguments into the file a client owns.
pub fn install_for(
    plan: &Plan,
    workspace: Option<&str>,
    project: Option<&str>,
) -> Result<(), String> {
    let entry = entry_for(&plan.profile, workspace, project)?;
    let client = lookup(&plan.client)?;
    let key = client.key.as_str();

    if plan.print {
        // The whole object rather than the inner entry: what a person
        // pastes is a config file, and one that is already correctly
        // shaped saves them guessing at the wrapper — including which
        // wrapper, now that it is not the same word for every client.
        println!("{}", render(&json!({ key: { SERVER_KEY: entry } })));
        return Ok(());
    }

    let path = match &plan.config {
        Some(path) => path.clone(),
        None => config_path(&plan.client)?,
    };

    let mut document = read(&path, client.syntax)?;

    let present = existing(&document, key, &path)?;
    match &present {
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
                "{} already has a `{key}.{SERVER_KEY}` entry, and it is not this one:\n\
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

    let replaced = present.is_some();
    upsert(&mut document, key, &entry, &path)?;
    let others = other_servers(&document, key);

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
        println!("{}", client.restart);
    }
    Ok(())
}

/// What one `uninstall` asks for. Deliberately smaller than [`Plan`]:
/// removal needs no profile (it is not generating an entry) and no `--force`
/// (it takes out whatever is under our key, whoever wrote it).
pub struct RemovePlan {
    /// Which client's config to edit.
    pub client: String,
    /// Edit this file instead of the client's own.
    pub config: Option<PathBuf>,
    /// Report what would change and write nothing.
    pub dry_run: bool,
}

/// `synveda mcp uninstall --client <name>` — the exact mirror of
/// [`install`] (OPS-10, ADR-0067 decision 3).
///
/// It removes the one key we own and nothing else. That is the same promise
/// `install` makes in the other direction, and half a promise is not one: a
/// user who let us write into Zed's settings is owed their comments, their
/// layout and their other context servers back exactly as they were.
pub fn uninstall(plan: &RemovePlan) -> Result<(), String> {
    let client = lookup(&plan.client)?;
    let key = client.key.as_str();
    let path = match &plan.config {
        Some(path) => path.clone(),
        None => config_path(&plan.client)?,
    };

    if !path.exists() {
        // Not an error. An uninstaller that fails on what is already gone is
        // one nobody runs twice, and the moment somebody runs it twice is
        // the moment the first run went wrong (ADR-0067 decision 5).
        println!("{} does not exist; nothing to remove", path.display());
        return Ok(());
    }

    let mut document = read(&path, client.syntax)?;
    if existing(&document, key, &path)?.is_none() {
        println!("{} has no `{key}.{SERVER_KEY}` entry", path.display());
        return Ok(());
    }

    remove_entry(&mut document, key, &path)?;
    let others = other_servers(&document, key);

    if plan.dry_run {
        println!("would rewrite {} without our entry", path.display());
    } else {
        write(&path, &document)?;
        println!("removed the `{SERVER_KEY}` entry from {}", path.display());
    }
    // Said out loud for install's reason, and more so here: this command
    // opened a file it does not own in order to delete from it, and the
    // user is owed the count that survived.
    println!("  ({others} other MCP server(s) left as they were)");
    if !plan.dry_run {
        println!("{}", client.restart);
    }
    Ok(())
}

/// Takes our entry out, leaving the containing map — and, for JSONC, every
/// other byte of the file — as it was.
///
/// The empty map is deliberately **not** pruned. A `mcpServers: {}` that we
/// emptied is the user's key, and deciding it is now litter is the same
/// class of judgement as replacing a differing entry without asking.
fn remove_entry(document: &mut Document, key: &str, path: &Path) -> Result<(), String> {
    match document {
        Document::Json(value) => {
            let servers = value
                .as_object_mut()
                .and_then(|root| root.get_mut(key))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| format!("{}'s `{key}` is not an object", path.display()))?;
            servers.remove(SERVER_KEY);
            Ok(())
        }
        Document::Jsonc(root) => {
            let prop = root
                .object_value()
                .and_then(|root| root.object_value(key))
                .and_then(|servers| servers.get(SERVER_KEY))
                .ok_or_else(|| format!("{}'s `{key}.{SERVER_KEY}` is not there", path.display()))?;
            prop.remove();
            Ok(())
        }
    }
}

/// The entry a client will exec.
#[cfg(test)]
fn entry(profile: &str) -> Result<Value, String> {
    entry_for(profile, None, None)
}

fn entry_for(
    profile: &str,
    workspace: Option<&str>,
    project: Option<&str>,
) -> Result<Value, String> {
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
    if let Some(workspace) = workspace {
        args.push(json!("--workspace"));
        args.push(json!(workspace));
    }
    if let Some(project) = project {
        args.push(json!("--project"));
        args.push(json!(project));
    }
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

/// The registry entry for `client`, or the message that names the ones we
/// do have.
fn lookup(client: &str) -> Result<Client, String> {
    let clients = registry()?;
    clients.get(client).cloned().ok_or_else(|| {
        format!(
            "unknown client {client:?}; known clients are {}.\n\
             Add your own to ~/{USER_REGISTRY} — see `synveda mcp install --help`",
            names(&clients),
        )
    })
}

/// Where `client` keeps the config that names its MCP servers.
///
/// Every path in the registry is the vendor's own, and per-user rather than
/// per-project on purpose: a bearer is bound to a machine by `synveda
/// login`, so the server belongs beside it rather than in a checkout that
/// travels. `--config` is the per-project form for the clients that have
/// one.
fn config_path(client: &str) -> Result<PathBuf, String> {
    let entry = lookup(client)?;
    let os = std::env::consts::OS;
    let template = entry
        .path
        .get(os)
        .or_else(|| entry.path.get("any"))
        .ok_or_else(|| {
            // Guessing a path on a platform the vendor documents no path
            // for is how this command writes a file nothing reads. Say so,
            // and hand over the JSON instead.
            let known: Vec<&str> = entry.path.keys().map(String::as_str).collect();
            format!(
                "{client} has no documented config path for {os}; the registry has {}. \
                 Use --print and place the entry yourself, or --config <path> if you know \
                 where it lives here.",
                known.join(", "),
            )
        })?;
    expand(template)
}

/// `~` and `$VAR` in a registry path.
///
/// Deliberately the whole of the language: a registry entry is a location,
/// not a program, and every construct here is one a user can predict from
/// having seen a shell.
fn expand(template: &str) -> Result<PathBuf, String> {
    let mut path = PathBuf::new();
    for (index, segment) in template.split('/').enumerate() {
        match segment {
            "~" if index == 0 => path.push(home()?),
            _ if segment.starts_with('$') => {
                let name = segment
                    .trim_start_matches('$')
                    .trim_start_matches('{')
                    .trim_end_matches('}');
                let value = std::env::var_os(name)
                    .ok_or_else(|| format!("{template} needs ${name}, which is not set"))?;
                path.push(value);
            }
            _ => path.push(segment),
        }
    }
    Ok(path)
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "neither HOME nor USERPROFILE is set".to_owned())
}

// ── The file ───────────────────────────────────────────────────────────

/// A client's config, and how it has to be written back.
///
/// The split is not cosmetic. A file this command *generated* may be
/// rendered whole by a formatter, because there is nothing in it that did
/// not come from here. A file its owner maintains may not: it carries
/// comments, trailing commas and a layout that are the point of the file,
/// and `serde_json` can represent none of them.
enum Document {
    /// Strict JSON, or a file that does not exist yet — an app-managed
    /// config (Claude Desktop, Cursor) the formatter may render whole.
    Json(Value),
    /// JSONC its owner maintains (Zed). Held as a concrete syntax tree, so
    /// writing back changes the bytes of the one key we own and leaves
    /// every other byte — comments included — exactly as it found them.
    Jsonc(Box<jsonc_parser::cst::CstRootNode>),
}

/// The config as it stands, or an empty object when there is none yet.
///
/// The syntax is the registry's declaration, **not** a sniff of the bytes.
/// A `jsonc` client whose file happens to carry no comments today still
/// parses as strict JSON, and sniffing would send it down the formatter
/// path and hand its owner back a reflowed settings file. What makes a
/// config unsafe to reformat is who maintains it, which is a fact about the
/// client and not about today's contents.
fn read(path: &Path, syntax: Syntax) -> Result<Document, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document::Json(json!({})));
        }
        Err(err) => return Err(format!("read {}: {err}", path.display())),
    };
    // A file with nothing in it has no formatting to keep, and the CST has
    // no root to splice into; both syntaxes start from the same blank.
    if raw.trim().is_empty() {
        return Ok(Document::Json(json!({})));
    }
    // A file we cannot parse is a file we must not rewrite: whatever is in
    // it, the user put it there, and a "fix" that discards it is worse than
    // a refusal.
    match syntax {
        Syntax::Json => serde_json::from_str(&raw)
            .map(Document::Json)
            .map_err(|err| {
                format!(
                    "{} is not valid JSON ({err}); fix it, or use --print and edit it by hand",
                    path.display(),
                )
            }),
        Syntax::Jsonc => {
            match jsonc_parser::cst::CstRootNode::parse(
                &raw,
                &jsonc_parser::ParseOptions::default(),
            ) {
                Ok(root) => Ok(Document::Jsonc(Box::new(root))),
                Err(err) => Err(format!(
                    "{} is not valid JSONC ({err}); fix it, or use --print and edit it by hand",
                    path.display(),
                )),
            }
        }
    }
}

/// What the config holds at `key`.`SERVER_KEY` today, if anything.
///
/// Read through `serde_json` in both representations so the compare, the
/// refusal and the diff below are one piece of logic rather than two that
/// have to agree.
fn existing(document: &Document, key: &str, path: &Path) -> Result<Option<Value>, String> {
    match document {
        Document::Json(value) => {
            let top = kind(value);
            let root = value.as_object().ok_or_else(|| {
                format!(
                    "{} holds {top} at the top level, not an object",
                    path.display()
                )
            })?;
            match root.get(key) {
                None => Ok(None),
                Some(servers) => {
                    let servers = servers.as_object().ok_or_else(|| {
                        format!(
                            "{}'s `{key}` holds {}, not an object",
                            path.display(),
                            kind(&root[key]),
                        )
                    })?;
                    Ok(servers.get(SERVER_KEY).cloned())
                }
            }
        }
        Document::Jsonc(root) => {
            let root_object = root.object_value().ok_or_else(|| {
                format!(
                    "{} does not hold an object at the top level",
                    path.display()
                )
            })?;
            match root_object.object_value(key) {
                None => Ok(None),
                Some(servers) => Ok(servers
                    .get(SERVER_KEY)
                    .and_then(|prop| prop.value())
                    .and_then(|value| value.to_serde_value())),
            }
        }
    }
}

/// How many *other* servers the config names, for the line that tells the
/// user their existing setup survived.
fn other_servers(document: &Document, key: &str) -> usize {
    let total = match document {
        Document::Json(value) => value
            .get(key)
            .and_then(Value::as_object)
            .map(Map::len)
            .unwrap_or(0),
        Document::Jsonc(root) => root
            .object_value()
            .and_then(|root| root.object_value(key))
            .map(|servers| servers.properties().len())
            .unwrap_or(0),
    };
    total.saturating_sub(1)
}

/// Put our entry in, creating the servers object if it is not there.
fn upsert(document: &mut Document, key: &str, entry: &Value, path: &Path) -> Result<(), String> {
    match document {
        Document::Json(value) => {
            let top = kind(value);
            let root = value.as_object_mut().ok_or_else(|| {
                format!(
                    "{} holds {top} at the top level, not an object",
                    path.display()
                )
            })?;
            let servers = root.entry(key.to_owned()).or_insert_with(|| json!({}));
            let found = kind(servers);
            let servers = servers.as_object_mut().ok_or_else(|| {
                format!("{}'s `{key}` holds {found}, not an object", path.display())
            })?;
            servers.insert(SERVER_KEY.to_owned(), entry.clone());
            Ok(())
        }
        Document::Jsonc(root) => {
            let root_object = root.object_value().ok_or_else(|| {
                format!(
                    "{} does not hold an object at the top level",
                    path.display()
                )
            })?;
            // `or_create` rather than `or_set`: a `context_servers` that
            // holds something other than an object is the user's, and
            // replacing it is the silent clobber this command refuses
            // everywhere else.
            let servers = root_object.object_value_or_create(key).ok_or_else(|| {
                format!(
                    "{}'s `{key}` holds something other than an object; \
                     fix it, or use --print and edit it by hand",
                    path.display(),
                )
            })?;
            let input = cst_value(entry);
            match servers.get(SERVER_KEY) {
                Some(prop) => prop.set_value(input),
                None => {
                    servers.append(SERVER_KEY, input);
                }
            }
            Ok(())
        }
    }
}

/// A `serde_json` value as the CST's input form.
///
/// Numbers go through their own text rather than `as_f64` so an integer
/// stays an integer; nothing this command writes is a number today, and
/// the day one is, `1` should not land in somebody's settings as `1.0`.
fn cst_value(value: &Value) -> jsonc_parser::cst::CstInputValue {
    use jsonc_parser::cst::CstInputValue;
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(flag) => CstInputValue::Bool(*flag),
        Value::Number(number) => CstInputValue::Number(number.to_string()),
        Value::String(text) => CstInputValue::String(text.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(cst_value).collect()),
        Value::Object(fields) => CstInputValue::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), cst_value(value)))
                .collect(),
        ),
    }
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
fn write(path: &Path, document: &Document) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    if !parent.exists() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("create {}: {err}", parent.display()))?;
        println!("created {}", parent.display());
    }

    let body = match document {
        Document::Json(value) => format!("{}\n", render(value)),
        // Already the whole file, byte-for-byte outside the key we
        // changed — including the trailing newline it arrived with, which
        // is why this branch adds none.
        Document::Jsonc(root) => root.to_string(),
    };
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
///
/// Built from the registry rather than a constant, so a user who added a
/// client sees it named here too — a list that knows less than the loader
/// is a list that teaches the wrong thing.
fn names(clients: &std::collections::BTreeMap<String, Client>) -> String {
    clients.keys().cloned().collect::<Vec<_>>().join(", ")
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

    fn zed_plan(config: &Path) -> Plan {
        Plan {
            client: "zed".to_owned(),
            ..plan(config)
        }
    }

    fn read_back(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("written")).expect("json")
    }

    fn remove_plan(config: &Path, client: &str) -> RemovePlan {
        RemovePlan {
            client: client.to_owned(),
            config: Some(config.to_owned()),
            dry_run: false,
        }
    }

    /// The mirror of the install test above, and the criterion OPS-10
    /// exists for: we opened somebody's file to delete from it, so
    /// everything that is not ours has to come back byte-identical.
    #[test]
    fn uninstall_leaves_a_hand_maintained_config_exactly_as_it_found_it() {
        let path = scratch("jsonc-uninstall");
        let before = "// Zed settings\n\
                      //\n\
                      // Keep the theme, I like it.\n\
                      {\n\
                      \x20 \"telemetry\": { \"metrics\": false },\n\
                      \x20 \"ui_font_size\": 16,\n\
                      \x20 \"theme\": {\n\
                      \x20   \"mode\": \"system\", // follows the OS\n\
                      \x20   \"dark\": \"One Dark\",\n\
                      \x20 },\n\
                      }\n";
        std::fs::write(&path, before).expect("seed");

        install(&zed_plan(&path)).expect("install");
        let with_ours = std::fs::read_to_string(&path).expect("written");
        assert!(with_ours.contains("synveda"), "install did not land");

        uninstall(&remove_plan(&path, "zed")).expect("uninstall");
        let after = std::fs::read_to_string(&path).expect("written");

        assert!(
            !after.contains("synveda"),
            "our entry survived the uninstall:\n{after}"
        );
        for kept in [
            "// Zed settings",
            "// Keep the theme, I like it.",
            "// follows the OS",
            "\"ui_font_size\": 16,",
            "\"dark\": \"One Dark\",",
        ] {
            assert!(
                after.contains(kept),
                "the removal lost {kept:?} from a file it does not own:\n{after}",
            );
        }
    }

    /// A user's other MCP servers are not ours to remove, and this is the
    /// assertion that says so — the failure it guards against is a splice
    /// that takes the whole map instead of one key in it.
    #[test]
    fn uninstall_leaves_another_server_alone() {
        let path = scratch("other-server-uninstall");
        std::fs::write(
            &path,
            r#"{"mcpServers":{"other":{"command":"/usr/bin/other"}}}"#,
        )
        .expect("seed");

        install(&plan(&path)).expect("install");
        uninstall(&remove_plan(&path, "claude-desktop")).expect("uninstall");

        let after = read_back(&path);
        assert!(
            after["mcpServers"]["synveda"].is_null(),
            "ours survived: {after}"
        );
        assert_eq!(
            after["mcpServers"]["other"]["command"],
            json!("/usr/bin/other"),
            "somebody else's server did not survive: {after}"
        );
    }

    /// Idempotent, and quiet about it (ADR-0067 decision 5). Failing on
    /// what is already gone makes an uninstaller nobody runs twice, and the
    /// moment somebody runs it twice is the moment the first run went wrong.
    #[test]
    fn uninstalling_twice_is_not_an_error() {
        let path = scratch("twice-uninstall");
        std::fs::write(&path, r#"{"mcpServers":{}}"#).expect("seed");
        install(&plan(&path)).expect("install");
        uninstall(&remove_plan(&path, "claude-desktop")).expect("first");
        uninstall(&remove_plan(&path, "claude-desktop")).expect("second must not fail");
    }

    /// A config that was never written is not an error either — the
    /// ordinary case when somebody uninstalls a client they never hooked up.
    #[test]
    fn uninstalling_an_absent_config_is_not_an_error() {
        let path = scratch("absent-uninstall").with_file_name("never-written.json");
        assert!(!path.exists());
        uninstall(&remove_plan(&path, "claude-desktop")).expect("must not fail");
        assert!(!path.exists(), "it created the file it was asked to clean");
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

    #[test]
    fn an_exact_project_selection_is_carried_into_the_entry() {
        let selected =
            entry_for("default", Some("workspace-1"), Some("project-2")).expect("selected entry");
        assert_eq!(
            selected["args"],
            json!([
                "mcp",
                "--writes",
                "tool",
                "--workspace",
                "workspace-1",
                "--project",
                "project-2"
            ])
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

    /// The name here is deliberately one no vendor will ever ship.
    ///
    /// It used to be `windsurf`, and OPS-9 adding Windsurf to the registry
    /// turned this test red — correctly, and usefully: a test that asserts
    /// the *unknown* path must not name a client that growing the table can
    /// make known, or it fails for a reason that has nothing to do with the
    /// behaviour it covers.
    #[test]
    fn an_unknown_client_names_the_ones_it_knows() {
        let error = config_path("not-a-real-mcp-client").expect_err("unknown");
        assert!(error.contains("claude-desktop"), "{error}");
        assert!(error.contains("cursor"), "{error}");
    }

    /// The clients OPS-9 added parse and resolve a path (ADR-0066
    /// decision 7). This asserts they are *reachable*, not that they are
    /// correct: none has been replayed against a running client, which
    /// The client-support registry states that limitation.
    #[test]
    fn the_clients_ops_9_added_resolve_somewhere() {
        for client in ["vscode", "windsurf", "continue"] {
            let path = config_path(client)
                .unwrap_or_else(|err| panic!("{client} should resolve on this OS: {err}"));
            assert!(
                path.is_absolute(),
                "{client} resolved to a relative path: {}",
                path.display()
            );
        }
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

    // ── The registry ───────────────────────────────────────────────────

    /// The file that ships in the binary is the one thing here nobody
    /// edits with a compiler watching, so it gets a test of its own: a
    /// typo in `adapters/registry.json` should fail here rather than at a user's
    /// first `install`.
    #[test]
    fn the_built_in_registry_parses_and_every_client_can_be_located_somewhere() {
        let clients = merge(BUILT_IN, None).expect("the built-in registry must parse");
        assert!(
            clients.contains_key("claude-desktop") && clients.contains_key("zed"),
            "the two clients ADR-0057 decision 11 records must be installable: {}",
            names(&clients),
        );
        for (name, client) in &clients {
            assert!(
                !client.path.is_empty(),
                "{name} names no config path on any OS, so `install` could never write it",
            );
            assert!(
                !client.key.is_empty() && !client.restart.is_empty(),
                "{name} is missing the key or the restart hint",
            );
            for template in client.path.values() {
                assert!(
                    template.starts_with('~') || template.starts_with('$'),
                    "{name}'s path {template:?} is absolute; a registry path is per-user",
                );
            }
        }
    }

    /// The point of the whole registry: a client we have never heard of
    /// needs a file, not a release (seed §2 principle 6).
    #[test]
    fn a_user_can_add_a_client_this_release_has_never_heard_of() {
        let user = r#"{
            // Windsurf, which shipped after we did.
            "windsurf": {
                "key": "mcpServers",
                "syntax": "json",
                "restart": "restart Windsurf",
                "path": { "any": "~/.codeium/windsurf/mcp_config.json" }
            }
        }"#;
        let clients = merge(BUILT_IN, Some(user)).expect("merge");
        let windsurf = clients.get("windsurf").expect("the user's client");
        assert_eq!(windsurf.key, "mcpServers");
        assert!(
            clients.contains_key("zed"),
            "adding a client must not drop the built-ins",
        );
    }

    #[test]
    fn a_user_entry_overrides_the_built_in_of_the_same_name() {
        let user = r#"{
            "zed": {
                "key": "context_servers",
                "syntax": "jsonc",
                "restart": "restart Zed",
                "path": { "any": "~/somewhere/else/settings.json" }
            }
        }"#;
        let clients = merge(BUILT_IN, Some(user)).expect("merge");
        assert_eq!(
            clients["zed"].path["any"], "~/somewhere/else/settings.json",
            "a user who says where their config lives should be believed",
        );
    }

    /// A registry that exists but does not parse is somebody's mistake,
    /// and silently ignoring it installs into the wrong file.
    #[test]
    fn a_broken_user_registry_fails_rather_than_being_skipped() {
        let error = merge(BUILT_IN, Some(r#"{ "zed": { "key": 7 } }"#))
            .expect_err("a malformed registry must be refused");
        assert!(
            error.contains(USER_REGISTRY),
            "the message must name the file to fix: {error}",
        );
    }

    #[test]
    fn a_registry_path_expands_home_and_environment_variables() {
        let home = home().expect("HOME");
        assert_eq!(
            expand("~/.cursor/mcp.json").expect("expand"),
            home.join(".cursor/mcp.json"),
        );
        // Whatever the platform, `$HOME`/`$USERPROFILE` is a variable that
        // is set, so this needs no fixture of its own.
        let named = if cfg!(windows) {
            "$USERPROFILE"
        } else {
            "$HOME"
        };
        assert_eq!(
            expand(&format!("{named}/x")).expect("expand"),
            home.join("x"),
        );
        let missing =
            expand("$SYNVEDA_NO_SUCH_VARIABLE/x").expect_err("an unset variable must be refused");
        assert!(missing.contains("SYNVEDA_NO_SUCH_VARIABLE"), "{missing}");
    }

    #[test]
    fn an_unknown_client_is_told_which_ones_exist() {
        let error = lookup("emacs").expect_err("unknown");
        assert!(error.contains("zed"), "{error}");
        assert!(
            error.contains(USER_REGISTRY),
            "the message must say how to add one: {error}",
        );
    }

    // ── The JSONC splice ───────────────────────────────────────────────

    /// The headline: a config its owner maintains comes back with their
    /// file in it, not ours.
    #[test]
    fn a_hand_maintained_config_keeps_its_comments_and_its_layout() {
        let path = scratch("jsonc");
        let before = "// Zed settings\n\
                      //\n\
                      // Keep the theme, I like it.\n\
                      {\n\
                      \x20 \"telemetry\": { \"metrics\": false },\n\
                      \x20 \"ui_font_size\": 16,\n\
                      \x20 \"theme\": {\n\
                      \x20   \"mode\": \"system\", // follows the OS\n\
                      \x20   \"dark\": \"One Dark\",\n\
                      \x20 },\n\
                      }\n";
        std::fs::write(&path, before).expect("seed");
        install(&zed_plan(&path)).expect("install");

        let after = std::fs::read_to_string(&path).expect("written");
        for kept in [
            "// Zed settings",
            "// Keep the theme, I like it.",
            "// follows the OS",
            "\"ui_font_size\": 16,",
            "\"dark\": \"One Dark\",",
        ] {
            assert!(
                after.contains(kept),
                "the splice lost {kept:?} from a file it does not own:\n{after}",
            );
        }
        // And the entry actually landed, under Zed's key rather than the
        // ecosystem's — a config written under `mcpServers` is one Zed
        // reads and finds nothing in.
        let parsed = parse_jsonc(&after).expect("jsonc").expect("value");
        assert!(
            parsed["context_servers"]["synveda"]["command"].is_string(),
            "{after}",
        );
        assert!(
            parsed.get("mcpServers").is_none(),
            "the ecosystem's key must not appear in a Zed config: {after}",
        );
    }

    /// The bug the *declared* syntax fixes, and the reason it is not
    /// sniffed: this file is valid strict JSON today, so a sniff would
    /// send it to the formatter and hand its owner back a reflowed
    /// settings file with their indentation gone.
    #[test]
    fn a_jsonc_config_with_no_comments_in_it_is_still_not_reformatted() {
        let path = scratch("jsonc-plain");
        let before = "{\n\"ui_font_size\":16,\n    \"theme\":{\"mode\":\"system\"}\n}\n";
        std::fs::write(&path, before).expect("seed");
        assert!(
            serde_json::from_str::<Value>(before).is_ok(),
            "the premise of this test is that a sniff would call this strict JSON",
        );

        install(&zed_plan(&path)).expect("install");
        let after = std::fs::read_to_string(&path).expect("written");
        assert!(
            after.contains("\"ui_font_size\":16,"),
            "the owner's spacing was reflowed:\n{after}",
        );
        assert!(
            after.contains("\"theme\":{\"mode\":\"system\"}"),
            "the owner's layout was reflowed:\n{after}",
        );
    }

    /// The refusal has to work the same on this path, or the splice
    /// becomes the way to clobber a config that the JSON path refuses to.
    #[test]
    fn a_differing_entry_in_a_jsonc_config_is_refused_without_force() {
        let path = scratch("jsonc-clobber");
        std::fs::write(
            &path,
            "{\n  // mine\n  \"context_servers\": { \"synveda\": { \"command\": \"/old/synveda\" } }\n}\n",
        )
        .expect("seed");

        let error = install(&zed_plan(&path)).expect_err("must refuse");
        assert!(error.contains("--force"), "{error}");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .contains("/old/synveda"),
            "a refusal must leave the file alone",
        );

        let forced = Plan {
            force: true,
            ..zed_plan(&path)
        };
        install(&forced).expect("forced");
        let after = std::fs::read_to_string(&path).expect("read");
        assert!(!after.contains("/old/synveda"), "{after}");
        assert!(
            after.contains("// mine"),
            "even --force keeps the file: {after}"
        );
    }

    #[test]
    fn another_context_server_survives_the_splice() {
        let path = scratch("jsonc-neighbour");
        std::fs::write(
            &path,
            "{\n  \"context_servers\": {\n    // theirs, not ours\n    \"other\": { \"command\": \"/usr/bin/other\" }\n  }\n}\n",
        )
        .expect("seed");
        install(&zed_plan(&path)).expect("install");
        let after = std::fs::read_to_string(&path).expect("read");
        assert!(after.contains("/usr/bin/other"), "{after}");
        assert!(after.contains("// theirs, not ours"), "{after}");
        let parsed = parse_jsonc(&after).expect("jsonc").expect("value");
        assert!(
            parsed["context_servers"]["synveda"]["command"].is_string(),
            "{after}"
        );
    }
}

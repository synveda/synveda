//! Managed ownership around the native/plugin, hook and MCP installers.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::harness::{self, Harness};
use crate::{local_state, mcp::install as mcp, plugin, setup};

#[derive(Args)]
pub(crate) struct Target {
    /// claude-code, codex, copilot-cli, or an MCP client such as cursor.
    #[arg(long)]
    client: String,
    /// Defaults to project for claude-code, codex and copilot-cli, and to user
    /// for MCP clients. Claude also accepts local.
    #[arg(long, value_parser = ["user", "project", "local"])]
    scope: Option<String>,
    /// Explicit project config for an MCP client; must remain inside this repository.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Connect an agent client to the project selected by `synveda setup`.
    Install {
        #[command(flatten)]
        target: Target,
        /// An extracted client archive (or, for Claude, a packaged
        /// marketplace); defaults to the installed client.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Show the intended registration without changing files or native state.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show whether the managed registration is present and unchanged.
    Status {
        #[command(flatten)]
        target: Target,
    },
    /// Remove only an unchanged managed registration, keeping data and consent.
    Uninstall {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        dry_run: bool,
    },
}

impl Command {
    pub(crate) fn locks(&self) -> bool {
        matches!(
            self,
            Self::Install { dry_run: false, .. } | Self::Uninstall { dry_run: false, .. }
        )
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Claude,
    Hooks(Harness),
    Mcp,
}

impl Kind {
    fn of(client: &str) -> Self {
        if client == "claude-code" {
            return Self::Claude;
        }
        Harness::parse(client).map_or(Self::Mcp, Self::Hooks)
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Identity {
    client: String,
    scope: String,
    root: PathBuf,
    target: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u8,
    identity: Identity,
    registration_hash: String,
    marketplace: Option<PathBuf>,
    // Persist intent before the vendor/file mutation; retry rechecks actual state.
    configured: bool,
    /// Whether this command registered the vendor's MCP server entry.
    #[serde(default)]
    mcp: bool,
}

pub(crate) fn run(command: Command) -> Result<(), String> {
    let (target, from, dry_run, install, status) = match &command {
        Command::Install {
            target,
            from,
            dry_run,
        } => (target, from.as_deref(), *dry_run, true, false),
        Command::Uninstall { target, dry_run } => (target, None, *dry_run, false, false),
        Command::Status { target } => (target, None, true, false, true),
    };
    let kind = Kind::of(&target.client);
    let identity = identity(target, kind)?;
    let path = local_state::receipt(
        "adapter",
        &serde_json::to_string(&identity).map_err(|e| e.to_string())?,
    )?;
    let receipt: Option<Receipt> = local_state::read_json(&path)?;
    if receipt
        .as_ref()
        .is_some_and(|r| r.version != 1 || r.identity != identity)
    {
        return Err("adapter receipt identity differs; no registration was changed".to_owned());
    }
    let current = registration(&identity, kind)?;
    if current.is_some()
        && let Some(marketplace) = receipt.as_ref().and_then(|r| r.marketplace.as_deref())
    {
        plugin::verify_managed_source(marketplace)?;
    }
    let current_hash = current.as_ref().map(hash);
    if status {
        report(&identity, kind, receipt.as_ref(), current_hash.as_deref());
        return Ok(());
    }
    if install {
        return run_install(identity, kind, from, dry_run, receipt, current_hash, &path);
    }
    run_uninstall(
        identity,
        kind,
        dry_run,
        receipt,
        current.is_some(),
        current_hash,
        &path,
    )
}

fn report(identity: &Identity, kind: Kind, receipt: Option<&Receipt>, current: Option<&str>) {
    let state = match (receipt, current) {
        (_, None) => "absent",
        (Some(r), Some(h)) if r.registration_hash == h => "configured (receipt matches)",
        (Some(_), Some(_)) => "conflict (registration changed)",
        (None, Some(_)) => "configured outside this managed route (no receipt)",
    };
    println!(
        "{} scope {}: {state}. Trust/loading/replay/live evidence is not established by registration.",
        identity.client, identity.scope
    );
    if let Kind::Hooks(_) = kind {
        println!("  hooks: {}", identity.target.display());
        let mcp = if receipt.is_some_and(|r| r.mcp) {
            "registered by this command"
        } else {
            "not registered by this command"
        };
        println!("  MCP server `synveda`: {mcp}");
    }
}

fn run_install(
    identity: Identity,
    kind: Kind,
    from: Option<&std::path::Path>,
    dry_run: bool,
    receipt: Option<Receipt>,
    current_hash: Option<String>,
    path: &std::path::Path,
) -> Result<(), String> {
    let selection = setup::selection(&identity.root)?;
    let mut runtime = None;
    let (desired, marketplace) = match kind {
        Kind::Claude => {
            let (marketplace, version) = plugin::managed_bundle(from)?;
            (
                json!({"version":version, "scope":identity.scope, "enabled":true, "root":identity.root}),
                Some(marketplace),
            )
        }
        Kind::Hooks(harness) => {
            let resolved = harness::runtime(harness, from)?;
            let desired = harness::desired(harness, &resolved, &selection);
            runtime = Some(resolved);
            (desired, None)
        }
        Kind::Mcp => {
            if from.is_some() {
                return Err("--from applies only to claude-code, codex and copilot-cli".to_owned());
            }
            (
                mcp::managed_entry(
                    &selection.profile,
                    &selection.workspace.to_string(),
                    &selection.project.to_string(),
                )?,
                None,
            )
        }
    };
    let wanted_hash = hash(&desired);
    check_ownership(
        kind,
        receipt.as_ref(),
        current_hash.as_deref(),
        &wanted_hash,
        marketplace.as_ref(),
    )?;
    if dry_run {
        println!(
            "Would configure {} scope {} at {}; observation {:?} for this project. No trust or loading approval is inferred.",
            identity.client,
            identity.scope,
            identity.target.display(),
            selection.observation
        );
        if let (Kind::Hooks(_), Some(runtime)) = (kind, &runtime) {
            println!(
                "Would register the MCP server with: {}",
                harness::mcp_add_args(runtime, &selection).join(" ")
            );
        }
        return Ok(());
    }
    let unchanged = receipt
        .as_ref()
        .is_some_and(|r| r.configured && r.registration_hash == wanted_hash)
        && current_hash.as_deref() == Some(wanted_hash.as_str());
    let had_mcp = receipt.as_ref().is_some_and(|r| r.mcp);
    // A refresh keeps the old receipt until the new entries are written, so a
    // failed write still matches the file it left behind.
    let refresh = matches!(kind, Kind::Hooks(_)) && receipt.is_some();
    let mut receipt = Receipt {
        version: 1,
        identity,
        registration_hash: wanted_hash,
        marketplace,
        configured: unchanged,
        mcp: had_mcp,
    };
    if !unchanged {
        if !refresh {
            local_state::save(path, &receipt)?;
        }
        apply(&receipt, kind, &desired, &selection)?;
        if registration(&receipt.identity, kind)?
            .as_ref()
            .map(hash)
            .as_deref()
            != Some(&receipt.registration_hash)
        {
            return Err("installer did not confirm the intended registration; receipt intent retained for inspection".to_owned());
        }
        receipt.configured = true;
        local_state::save(path, &receipt)?;
    }
    if let (Kind::Hooks(harness), Some(runtime)) = (kind, &runtime) {
        register_mcp(harness, runtime, &selection, &mut receipt, unchanged);
        local_state::save(path, &receipt)?;
        next_steps(harness, &receipt.identity.target);
        return Ok(());
    }
    println!(
        "Configured; ownership receipt saved. Review vendor trust and start a new session to verify loading."
    );
    Ok(())
}

/// Refuse to adopt or overwrite a registration this command does not own.
/// Hook files written by this command are replaced in place while unchanged,
/// so an upgraded client can refresh them.
fn check_ownership(
    kind: Kind,
    receipt: Option<&Receipt>,
    current_hash: Option<&str>,
    wanted_hash: &str,
    marketplace: Option<&PathBuf>,
) -> Result<(), String> {
    let Some(receipt) = receipt else {
        if current_hash.is_some() {
            return Err("an existing registration has no managed ownership receipt; it was not adopted or replaced; inspect it with the existing plugin/MCP commands, or remove it and retry".to_owned());
        }
        return Ok(());
    };
    if current_hash.is_some_and(|h| h != receipt.registration_hash) {
        return Err(
            "adapter registration changed outside this command; refusing replacement".to_owned(),
        );
    }
    let refreshable = matches!(kind, Kind::Hooks(_));
    if !refreshable
        && (receipt.registration_hash != wanted_hash || receipt.marketplace.as_ref() != marketplace)
    {
        return Err("adapter selection or bundle differs from its receipt; uninstall the unchanged managed entry before selecting new registration".to_owned());
    }
    Ok(())
}

fn apply(
    receipt: &Receipt,
    kind: Kind,
    desired: &Value,
    selection: &setup::Selection,
) -> Result<(), String> {
    let identity = &receipt.identity;
    match kind {
        Kind::Claude => plugin::install(&plugin::Plan {
            client: identity.client.clone(),
            from: receipt.marketplace.clone(),
            scope: identity.scope.clone(),
            force: false,
            dry_run: false,
        }),
        Kind::Hooks(harness) => harness::write(harness, &identity.target, desired),
        Kind::Mcp => mcp::install_for(
            &mcp::Plan {
                client: identity.client.clone(),
                config: Some(identity.target.clone()),
                profile: selection.profile.clone(),
                dry_run: false,
                force: false,
                print: false,
            },
            Some(&selection.workspace.to_string()),
            Some(&selection.project.to_string()),
            None,
            None,
        ),
    }
}

/// MCP registration is the vendor's own command; a failure leaves the hooks
/// in place and prints the exact command to run by hand.
fn register_mcp(
    harness: Harness,
    runtime: &harness::Runtime,
    selection: &setup::Selection,
    receipt: &mut Receipt,
    unchanged: bool,
) {
    if receipt.mcp && unchanged {
        println!("MCP server `synveda` is already registered.");
        return;
    }
    if receipt.mcp
        && let Err(error) = harness::run_vendor(harness, &harness::mcp_remove_args())
    {
        println!("Could not replace the earlier MCP server entry: {error}");
        return;
    }
    match harness::run_vendor(harness, &harness::mcp_add_args(runtime, selection)) {
        Ok(()) => {
            receipt.mcp = true;
            println!("Registered the `synveda` MCP server.");
        }
        Err(error) => {
            receipt.mcp = false;
            println!("MCP server not registered: {error}");
        }
    }
}

fn next_steps(harness: Harness, hooks: &std::path::Path) {
    println!("Wrote {}.", hooks.display());
    match harness {
        Harness::Codex => println!(
            "Next: start `codex` in this repository, trust the project when asked and approve the Synveda hooks with /hooks."
        ),
        Harness::Copilot => println!(
            "Next: start `copilot` in this repository and trust the folder when asked. The hook file names paths on this machine; keep it out of version control."
        ),
    }
}

fn run_uninstall(
    identity: Identity,
    kind: Kind,
    dry_run: bool,
    receipt: Option<Receipt>,
    present: bool,
    current_hash: Option<String>,
    path: &std::path::Path,
) -> Result<(), String> {
    let Some(receipt) = receipt else {
        if !present {
            println!("No registration or receipt; nothing to remove.");
            return Ok(());
        }
        return Err("registration has no ownership receipt; no entry was removed".to_owned());
    };
    if current_hash
        .as_ref()
        .is_some_and(|hash| hash != &receipt.registration_hash)
    {
        return Err("registration differs from the owned entry; no entry was removed".to_owned());
    }
    if dry_run {
        println!(
            "Would remove the unchanged {} registration in scope {}; retain data, project consent and unrelated entries.",
            identity.client, identity.scope
        );
        return Ok(());
    }
    if present {
        match kind {
            Kind::Claude => plugin::uninstall(&plugin::RemovePlan {
                client: identity.client.clone(),
                scope: identity.scope.clone(),
                dry_run: false,
            })?,
            Kind::Hooks(harness) => harness::remove(harness, &identity.target)?,
            Kind::Mcp => mcp::uninstall(&mcp::RemovePlan {
                client: identity.client.clone(),
                config: Some(identity.target.clone()),
                dry_run: false,
            })?,
        }
    }
    if let Kind::Hooks(harness) = kind
        && receipt.mcp
        && let Err(error) = harness::run_vendor(harness, &harness::mcp_remove_args())
    {
        println!("MCP server entry left in place: {error}");
    }
    if registration(&identity, kind)?.is_some() {
        return Err("registration remains; receipt retained".to_owned());
    }
    local_state::remove_receipt(path)?;
    println!(
        "Removed the owned registration; project configuration, credentials and spool data are retained."
    );
    Ok(())
}

fn identity(target: &Target, kind: Kind) -> Result<Identity, String> {
    let root = plugin::project_root()?;
    let scope = target.scope.clone().unwrap_or_else(|| {
        match kind {
            Kind::Mcp => "user",
            Kind::Claude | Kind::Hooks(_) => "project",
        }
        .to_owned()
    });
    if target.config.is_some() && !matches!(kind, Kind::Mcp) {
        return Err(format!(
            "{} configuration is owned by its native installer; --config is not supported",
            target.client
        ));
    }
    let config = match kind {
        Kind::Claude => {
            if scope == "user" {
                return Err("managed Claude registration requires project or local scope; project observation consent cannot authorize every repository".to_owned());
            }
            match std::env::var_os("CLAUDE_CONFIG_DIR") {
                Some(path) if !path.is_empty() => PathBuf::from(path),
                Some(_) => return Err("CLAUDE_CONFIG_DIR must not be empty".to_owned()),
                None => crate::client_paths::vendor_home()?.join(".claude"),
            }
        }
        Kind::Hooks(harness) => {
            if scope != "project" {
                return Err(format!(
                    "{} hooks are registered per repository; use --scope project",
                    target.client
                ));
            }
            if !root.join(".git").exists() {
                return Err(format!(
                    "{} loads hooks from the Git repository root; run this inside a Git repository",
                    target.client
                ));
            }
            harness.target(&root)
        }
        Kind::Mcp => match (scope.as_str(), target.config.as_deref()) {
            ("user", None) => mcp::managed_path(&target.client, None)?,
            ("project", Some(path)) => mcp::managed_path(&target.client, Some(path))?,
            _ => return Err("MCP adapters require --scope user without --config, or --scope project with an explicit --config inside this repository; local scope is vendor-specific".to_owned()),
        },
    };
    let config = local_state::absolute(&config)?;
    if matches!(kind, Kind::Mcp) && scope == "project" && !config.starts_with(&root) {
        return Err("project MCP config must be inside this repository".to_owned());
    }
    Ok(Identity {
        client: target.client.clone(),
        scope,
        root,
        target: config,
    })
}

fn registration(identity: &Identity, kind: Kind) -> Result<Option<Value>, String> {
    match kind {
        Kind::Claude => plugin::registration(&identity.scope),
        Kind::Hooks(harness) => harness::registration(harness, &identity.target),
        Kind::Mcp => mcp::registration(&identity.client, &identity.target),
    }
}

fn hash(value: &Value) -> String {
    local_state::digest(value.to_string().as_bytes())
}

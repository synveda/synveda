//! Managed ownership around the existing native/plugin and MCP installers.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{local_state, mcp::install as mcp, plugin, setup};

#[derive(Args)]
pub(crate) struct Target {
    /// claude-code, or a client supported by the existing MCP config registry.
    #[arg(long)]
    client: String,
    /// Exact native scope. Claude supports project/local; MCP supports user/project.
    #[arg(long, value_parser = ["user", "project", "local"])]
    scope: String,
    /// Explicit project config for an MCP client; must remain inside this repository.
    #[arg(long)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Register the configured project's adapter and retain an ownership receipt.
    Install {
        #[command(flatten)]
        target: Target,
        /// Packaged Claude marketplace; otherwise use the installed bundle.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Show the intended registration without changing files or native state.
        #[arg(long)]
        dry_run: bool,
    },
    /// Inspect configured state; does not claim vendor trust, loading or live execution.
    Status {
        #[command(flatten)]
        target: Target,
    },
    /// Remove only an unchanged receipt-owned registration, retaining data and consent.
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
    let identity = identity(target)?;
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
    let current = registration(&identity)?;
    if current.is_some()
        && let Some(marketplace) = receipt.as_ref().and_then(|r| r.marketplace.as_deref())
    {
        plugin::verify_managed_source(marketplace)?;
    }
    let current_hash = current.as_ref().map(hash);
    if status {
        let state = match (&receipt, &current_hash) {
            (_, None) => "absent",
            (Some(r), Some(h)) if &r.registration_hash == h => "configured (receipt matches)",
            (Some(_), Some(_)) => "conflict (registration changed)",
            (None, Some(_)) => "configured outside this managed route (no receipt)",
        };
        println!(
            "{} scope {}: {state}. Trust/loading/replay/live evidence is not established by registration.",
            identity.client, identity.scope
        );
        return Ok(());
    }
    if install {
        let selection = setup::selection(&identity.root)?;
        let (desired, marketplace) = if identity.client == "claude-code" {
            let (marketplace, version) = plugin::managed_bundle(from)?;
            (
                json!({"version":version, "scope":identity.scope, "enabled":true, "root":identity.root}),
                Some(marketplace),
            )
        } else {
            if from.is_some() {
                return Err("--from applies only to claude-code".to_owned());
            }
            (
                mcp::managed_entry(
                    &selection.profile,
                    &selection.workspace.to_string(),
                    &selection.project.to_string(),
                )?,
                None,
            )
        };
        let wanted_hash = hash(&desired);
        if let Some(receipt) = &receipt {
            if receipt.registration_hash != wanted_hash || receipt.marketplace != marketplace {
                return Err("adapter selection or bundle differs from its receipt; uninstall the unchanged managed entry before selecting new registration".to_owned());
            }
            if current_hash
                .as_ref()
                .is_some_and(|h| h != &receipt.registration_hash)
            {
                return Err(
                    "adapter registration changed outside this command; refusing replacement"
                        .to_owned(),
                );
            }
        } else if current.is_some() {
            return Err("an existing registration has no managed ownership receipt; it was not adopted or replaced; inspect it with the existing plugin/MCP commands".to_owned());
        }
        if dry_run {
            println!(
                "Would configure {} scope {} at {}; observation {:?} for this project. No trust or loading approval is inferred.",
                identity.client,
                identity.scope,
                identity.target.display(),
                selection.observation
            );
            return Ok(());
        }
        let mut receipt = Receipt {
            version: 1,
            identity,
            registration_hash: wanted_hash,
            marketplace,
            configured: false,
        };
        local_state::save(&path, &receipt)?;
        if receipt.identity.client == "claude-code" {
            plugin::install(&plugin::Plan {
                client: receipt.identity.client.clone(),
                from: receipt.marketplace.clone(),
                scope: receipt.identity.scope.clone(),
                force: false,
                dry_run: false,
            })?;
        } else {
            mcp::install_for(
                &mcp::Plan {
                    client: receipt.identity.client.clone(),
                    config: Some(receipt.identity.target.clone()),
                    profile: selection.profile,
                    dry_run: false,
                    force: false,
                    print: false,
                },
                Some(&selection.workspace.to_string()),
                Some(&selection.project.to_string()),
                None,
                None,
            )?;
        }
        if registration(&receipt.identity)?
            .as_ref()
            .map(hash)
            .as_deref()
            != Some(&receipt.registration_hash)
        {
            return Err("installer did not confirm the intended registration; receipt intent retained for inspection".to_owned());
        }
        receipt.configured = true;
        local_state::save(&path, &receipt)?;
        println!(
            "Configured; ownership receipt saved. Review vendor trust and start a new session to verify loading."
        );
        return Ok(());
    }
    let Some(receipt) = receipt else {
        if current.is_none() {
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
    if current.is_some() {
        if identity.client == "claude-code" {
            plugin::uninstall(&plugin::RemovePlan {
                client: identity.client.clone(),
                scope: identity.scope.clone(),
                dry_run: false,
            })?;
        } else {
            mcp::uninstall(&mcp::RemovePlan {
                client: identity.client.clone(),
                config: Some(identity.target.clone()),
                dry_run: false,
            })?;
        }
    }
    if registration(&identity)?.is_some() {
        return Err("registration remains; receipt retained".to_owned());
    }
    std::fs::remove_file(path).map_err(|e| format!("remove completed adapter receipt: {e}"))?;
    println!(
        "Removed the owned registration; project configuration, credentials and spool data are retained."
    );
    Ok(())
}

fn identity(target: &Target) -> Result<Identity, String> {
    let root = plugin::project_root()?;
    let config = if target.client == "claude-code" {
        if target.scope == "user" {
            return Err("managed Claude registration requires project or local scope; project observation consent cannot authorize every repository".to_owned());
        }
        if target.config.is_some() {
            return Err(
                "Claude configuration is owned by its native installer; --config is not supported"
                    .to_owned(),
            );
        }
        match std::env::var_os("CLAUDE_CONFIG_DIR") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            Some(_) => return Err("CLAUDE_CONFIG_DIR must not be empty".to_owned()),
            None => {
                PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".claude")
            }
        }
    } else {
        match (target.scope.as_str(), target.config.as_deref()) {
            ("user", None) => mcp::managed_path(&target.client, None)?,
            ("project", Some(path)) => mcp::managed_path(&target.client, Some(path))?,
            _ => return Err("MCP adapters require --scope user without --config, or --scope project with an explicit --config inside this repository; local scope is vendor-specific".to_owned()),
        }
    };
    let config = local_state::absolute(&config)?;
    if target.client != "claude-code" && target.scope == "project" && !config.starts_with(&root) {
        return Err("project MCP config must be inside this repository".to_owned());
    }
    Ok(Identity {
        client: target.client.clone(),
        scope: target.scope.clone(),
        root,
        target: config,
    })
}

fn registration(identity: &Identity) -> Result<Option<Value>, String> {
    if identity.client == "claude-code" {
        plugin::registration(&identity.scope)
    } else {
        mcp::registration(&identity.client, &identity.target)
    }
}

fn hash(value: &Value) -> String {
    local_state::digest(value.to_string().as_bytes())
}

//! `synveda plugin install` — the Claude Code plugin, put where the harness
//! actually reads it (OPS-8, ADR-0065 amendment 2).
//!
//! # Why this is not `mcp install`
//!
//! `mcp install` writes another application's config file, and ADR-0057
//! decision 10 justifies that by the absence of an alternative: Claude
//! Desktop ships no CLI, so a JSON file is the only interface it has.
//!
//! Claude Code ships one. `claude plugin marketplace add` and `claude plugin
//! install` are its own supported entry points, and its plugin state is
//! three JSON files that reference each other — `known_marketplaces.json`,
//! `installed_plugins.json`, and a versioned cache directory. Reproducing
//! that by hand would be a second implementation of somebody else's
//! installer, wrong the first time the format moves. So this command drives
//! theirs, and the harness stays a guest we ask rather than a directory we
//! edit (seed §2 principle 6).
//!
//! # What "install a plugin" turns out to mean
//!
//! Not what this repository said it did. The adapter README said "point
//! Claude Code at this directory as a plugin", and `demos/adpt-1-claude-code.sh`
//! copies three directories into `~/.claude/plugins/synveda/`. **Claude Code
//! does not read that path.** Its plugins live in a *marketplace* — a
//! directory carrying `.claude-plugin/marketplace.json` — which is added,
//! and from which named plugins are installed into a cache it owns.
//!
//! Nothing caught it because ADPT-1's demo is its own harness: it reads
//! `hooks/hooks.json` itself, substitutes `${CLAUDE_PLUGIN_ROOT}`, and
//! invokes node. That proves the hooks do their job — which is what ADPT-1
//! is about — and proves nothing about whether Claude Code ever loads them.
//!
//! # Restraint, the same as `mcp install`'s
//!
//! - **It refuses to clobber.** An installed `synveda@synveda` is reported
//!   rather than replaced; `--force` reinstalls.
//! - **It can be asked first.** `--dry-run` prints the two commands and runs
//!   neither.
//! - **Nothing secret is written.** The plugin reads its bearer from what
//!   `synveda login` stored, per call.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `<plugin>@<marketplace>`, which is how `claude plugin install` names one.
/// Both halves are `synveda` because the marketplace we ship carries a
/// single plugin. Both come from `adapters/claude-code/marketplace.json`,
/// and changing that file without changing this installs nothing — which a
/// test below refuses to let happen quietly.
const PLUGIN_ID: &str = "synveda@synveda";

/// The clients this knows how to install into.
///
/// One, and the shape is deliberately the same as `skill.rs`'s table rather
/// than a `match`: a second harness with a plugin system is a row here, not
/// a rewrite. It is not the `mcp install` registry because that one
/// describes *config files*, and this describes *a CLI to drive* — the same
/// data would be two unrelated meanings under one name.
const CLIENTS: [&str; 1] = ["claude-code"];

pub struct Plan {
    pub client: String,
    /// Install a bundle from somewhere other than the installed release —
    /// a checkout's `adapters/claude-code`, wrapped, or an unpacked
    /// tarball.
    pub from: Option<PathBuf>,
    pub dry_run: bool,
    pub force: bool,
    /// Claude Code's own installation scope: `user`, `project` or `local`.
    pub scope: String,
}

pub fn install(plan: &Plan) -> Result<(), String> {
    if !CLIENTS.contains(&plan.client.as_str()) {
        return Err(format!(
            "unknown client {:?}; known clients are {}",
            plan.client,
            CLIENTS.join(", ")
        ));
    }

    let marketplace = locate(plan.from.as_deref())?;
    // Looked up before the dry run rather than inside it: `--dry-run` is
    // what somebody runs *before* they have decided anything, including on
    // a machine with no Claude Code, and refusing to describe the plan
    // because the tool is absent would be a worse answer than describing it.
    let claude = which("claude");

    let add = vec![
        "plugin".to_owned(),
        "marketplace".to_owned(),
        "add".to_owned(),
        marketplace.display().to_string(),
    ];
    let mut install = vec![
        "plugin".to_owned(),
        "install".to_owned(),
        PLUGIN_ID.to_owned(),
        "--scope".to_owned(),
        plan.scope.clone(),
    ];
    if plan.force {
        install.push("--force".to_owned());
    }

    if plan.dry_run {
        println!("synveda plugin install --dry-run");
        println!();
        println!("  marketplace  {}", marketplace.display());
        println!(
            "  claude       {}",
            match &claude {
                Some(path) => path.display().to_string(),
                None => "not on PATH — install Claude Code to run this".to_owned(),
            }
        );
        println!("  scope        {}", plan.scope);
        println!();
        println!("  would run    claude {}", add.join(" "));
        println!("               claude {}", install.join(" "));
        println!();
        println!("  writes nothing outside Claude Code's own plugin state");
        return Ok(());
    }

    let claude = claude.ok_or_else(|| {
        format!(
            "the `claude` CLI is not on PATH, and it is what installs a Claude Code plugin.\n\
             \n\
             The bundle is ready at {}. With Claude Code installed, either re-run\n\
             this command or do it yourself:\n\
             \n\
             \x20 claude plugin marketplace add {}\n\
             \x20 claude plugin install {PLUGIN_ID} --scope {}",
            marketplace.display(),
            marketplace.display(),
            plan.scope,
        )
    })?;

    // Adding a marketplace that is already known re-points it at this path,
    // which is what a reinstall from a new release should do.
    run(&claude, &add)?;

    if !plan.force && installed(&claude) {
        println!("    {PLUGIN_ID} is already installed — leaving it alone");
        println!(
            "    (`--force` reinstalls it from {})",
            marketplace.display()
        );
        return Ok(());
    }
    run(&claude, &install)?;

    println!();
    println!("    {PLUGIN_ID} installed, scope {}", plan.scope);
    println!("    start a new Claude Code session to pick it up.");
    println!();
    println!("    It needs a login to do anything: `synveda login` stores the");
    println!("    bearer, and the plugin reads it per call. Check it loaded with");
    println!("    `claude plugin list`.");
    Ok(())
}

/// The marketplace directory to install from.
///
/// `--from` wins. Otherwise an installed release's `$SYNVEDA_HOME/plugin`,
/// then a checkout — and a checkout has to be *wrapped* at package time, so
/// what is named there is the packaged bundle rather than
/// `adapters/claude-code` itself. Saying so is the whole point: a plugin
/// directory is not a marketplace, and pointing Claude Code at one installs
/// nothing.
fn locate(from: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = from {
        return validate(path).map(Path::to_path_buf);
    }
    let home = crate::init::synveda_home()?;
    let installed = home.join("plugin");
    if installed.join(".claude-plugin/marketplace.json").is_file() {
        return Ok(installed);
    }
    Err(format!(
        "no plugin bundle to install. Looked in:\n\
         \x20 {}\n\
         \n\
         An installed release carries one. From a checkout, build and package it:\n\
         \n\
         \x20 pnpm --filter @synveda/claude-code-adapter build\n\
         \x20 scripts/package-plugin.sh $(synveda --version | cut -d' ' -f2) /tmp/synveda-plugin\n\
         \x20 synveda plugin install --client claude-code --from /tmp/synveda-plugin/plugin",
        installed.display(),
    ))
}

fn validate(path: &Path) -> Result<&Path, String> {
    if path.join(".claude-plugin/marketplace.json").is_file() {
        return Ok(path);
    }
    // The specific wrong thing somebody will do, named rather than
    // reported as "not found": hand it the plugin.
    if path.join(".claude-plugin/plugin.json").is_file() {
        return Err(format!(
            "{} is a plugin, not a marketplace — Claude Code installs marketplaces.\n\
             Package it first:  scripts/package-plugin.sh <version> <outdir>",
            path.display()
        ));
    }
    Err(format!(
        "{} has no .claude-plugin/marketplace.json",
        path.display()
    ))
}

fn installed(claude: &Path) -> bool {
    Command::new(claude)
        .args(["plugin", "list"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .is_some_and(|out| String::from_utf8_lossy(&out.stdout).contains(PLUGIN_ID))
}

fn run(claude: &Path, args: &[String]) -> Result<(), String> {
    println!("    claude {}", args.join(" "));
    let status = Command::new(claude)
        .args(args)
        .status()
        .map_err(|err| format!("run claude {}: {err}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("claude {} failed ({status})", args.join(" ")));
    }
    Ok(())
}

/// `command -v`, without a dependency. Absolute paths are taken as given.
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(what: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("synveda-plugin-{what}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_marketplace_is_accepted_and_a_bare_plugin_is_named_as_the_mistake() {
        let root = scratch("locate");

        let market = root.join("market");
        std::fs::create_dir_all(market.join(".claude-plugin")).unwrap();
        std::fs::write(market.join(".claude-plugin/marketplace.json"), "{}").unwrap();
        assert_eq!(validate(&market).unwrap(), market.as_path());

        // The mistake this repository's own docs invited for a year: hand
        // Claude Code the plugin directory. It is not a marketplace, and
        // nothing installs.
        let plugin = root.join("plugin");
        std::fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        std::fs::write(plugin.join(".claude-plugin/plugin.json"), "{}").unwrap();
        let err = validate(&plugin).expect_err("a plugin is not a marketplace");
        assert!(err.contains("is a plugin, not a marketplace"), "{err}");
        assert!(err.contains("package-plugin.sh"), "{err}");

        let neither = root.join("neither");
        std::fs::create_dir_all(&neither).unwrap();
        assert!(validate(&neither).is_err());

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unknown_client_names_the_ones_it_knows() {
        let err = install(&Plan {
            client: "emacs".to_owned(),
            from: None,
            dry_run: true,
            force: false,
            scope: "user".to_owned(),
        })
        .expect_err("emacs has no Claude Code plugin system");
        assert!(err.contains("emacs"), "{err}");
        assert!(err.contains("claude-code"), "{err}");
    }

    #[test]
    fn the_marketplace_and_the_plugin_id_agree() {
        // `claude plugin install <plugin>@<marketplace>` — if the id and
        // the manifest drift apart the install names something that is not
        // there, and Claude Code's error is about a missing plugin rather
        // than about us.
        let (plugin, marketplace) = PLUGIN_ID.split_once('@').expect("an id of the right shape");
        let manifest = include_str!("../../../adapters/claude-code/marketplace.json");
        let parsed: serde_json::Value = serde_json::from_str(manifest).unwrap();
        assert_eq!(parsed["name"], marketplace);
        assert_eq!(parsed["plugins"][0]["name"], plugin);
        // And the plugin it names has to be the directory the packager
        // stages beside it.
        assert_eq!(parsed["plugins"][0]["source"], "./synveda");
    }
}

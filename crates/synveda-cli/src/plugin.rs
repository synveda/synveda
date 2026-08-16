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
//! - **It refuses to clobber, but not to upgrade.** An installed
//!   `synveda@synveda` *at the bundle's version* is reported rather than
//!   replaced. A different version is replaced, because "already installed"
//!   is not "installed at this version" and Claude Code caches its own copy
//!   — so an upgraded release otherwise leaves the previous plugin running.
//!   `--force` replaces regardless.
//! - **It can be asked first.** `--dry-run` prints the commands and runs
//!   none of them.
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

/// The marketplace's own name, which is the second half of [`PLUGIN_ID`] and
/// what `claude plugin marketplace update` takes as its argument.
const MARKETPLACE: &str = "synveda";

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
    // Re-reads the marketplace manifest from the path it points at. `add` on
    // a known marketplace re-points it and reports "already on disk" without
    // re-reading, so an upgrade that replaced the bundle would otherwise be
    // installed from the previous release's manifest.
    let refresh = vec![
        "plugin".to_owned(),
        "marketplace".to_owned(),
        "update".to_owned(),
        MARKETPLACE.to_owned(),
    ];
    // No `--force` here: `claude plugin install` does not take one, and
    // passing it made `synveda plugin install --force` fail outright with
    // `error: unknown option '--force'` — the escape hatch our own message
    // advertised. Replacing an installed plugin is `uninstall` then
    // `install`, below.
    let install = vec![
        "plugin".to_owned(),
        "install".to_owned(),
        PLUGIN_ID.to_owned(),
        "--scope".to_owned(),
        plan.scope.clone(),
    ];
    let remove = vec![
        "plugin".to_owned(),
        "uninstall".to_owned(),
        PLUGIN_ID.to_owned(),
    ];

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
        println!("               claude {}", refresh.join(" "));
        println!(
            "               claude {} (if a different version is installed)",
            remove.join(" ")
        );
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
    run(&claude, &refresh)?;

    // "Already installed" is not "installed at this version", and the
    // difference is the whole upgrade story. Claude Code copies a plugin
    // into a cache it owns at install time, so replacing
    // `$SYNVEDA_HOME/plugin` leaves the *running* plugin on whatever
    // release installed it. Measured after upgrading a machine 0.1.0 →
    // 0.1.2: the bundle on disk said 0.1.2 and `claude plugin list` said
    // **0.1.0**, two releases behind, reported healthy and enabled.
    //
    // This is the same fault `binary_stamp` fixed for the gateway one
    // release earlier (ADR-0065 amendment 5): a convergence check that
    // compares identity when it has to compare the artefact.
    let want = bundle_version(&marketplace);
    match installed_version(&claude) {
        Some(have) if !plan.force && Some(&have) == want.as_ref() => {
            println!("    {PLUGIN_ID} {have} is already installed — leaving it alone");
            return Ok(());
        }
        Some(have) => {
            match &want {
                Some(want) => println!("    installed {have}, bundle {want} — replacing"),
                None => println!("    installed {have} — replacing"),
            }
            // Claude Code has no update verb, and `install` on an installed
            // plugin reports success while changing nothing — which is how
            // this stayed invisible.
            run(&claude, &remove)?;
        }
        None => {}
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
/// What one `uninstall` asks for. No `from` and no `scope`: removal names
/// the plugin Claude Code already has, and where its bundle came from is
/// not a question that has an answer any more.
pub struct RemovePlan {
    pub client: String,
    pub dry_run: bool,
}

/// `synveda plugin uninstall` — the mirror of [`install`] (OPS-10,
/// ADR-0067 decision 4).
///
/// Two steps, in this order, and the order is the finding ADR-0065
/// amendment 8 paid for: Claude Code copies a plugin into a **versioned
/// cache it owns** at install time, so removing our marketplace does not
/// remove the running plugin. `plugin uninstall` takes the plugin out;
/// `marketplace remove` takes out the source it came from. Doing only the
/// second leaves a plugin loaded from a marketplace that no longer exists.
///
/// It asserts against `claude plugin list` rather than the filesystem for
/// that amendment's other half: installing and *loading* are different
/// events, and so are removing and unloading.
pub fn uninstall(plan: &RemovePlan) -> Result<(), String> {
    if !CLIENTS.contains(&plan.client.as_str()) {
        return Err(format!(
            "unknown client {:?}; known clients are {}",
            plan.client,
            CLIENTS.join(", ")
        ));
    }
    let claude = which("claude");

    let remove_plugin = vec![
        "plugin".to_owned(),
        "uninstall".to_owned(),
        PLUGIN_ID.to_owned(),
    ];
    let remove_marketplace = vec![
        "plugin".to_owned(),
        "marketplace".to_owned(),
        "remove".to_owned(),
        MARKETPLACE.to_owned(),
    ];

    if plan.dry_run {
        println!("synveda plugin uninstall --dry-run");
        println!();
        println!(
            "  claude       {}",
            match &claude {
                Some(path) => path.display().to_string(),
                None => "not on PATH".to_owned(),
            }
        );
        println!("  would run    claude {}", remove_plugin.join(" "));
        println!("               claude {}", remove_marketplace.join(" "));
        return Ok(());
    }

    let Some(claude) = claude else {
        // Not an error. Claude Code being absent is the ordinary state of a
        // machine somebody is cleaning up, and failing here would stop an
        // uninstall over a tool the user has already removed.
        println!("claude is not on PATH; nothing to remove from Claude Code");
        return Ok(());
    };

    match installed_version(&claude) {
        None => {
            println!("Claude Code does not have {PLUGIN_ID} installed; nothing to do");
            return Ok(());
        }
        Some(version) => println!("removing {PLUGIN_ID} {version} from Claude Code"),
    }

    run(&claude, &remove_plugin)?;
    // The marketplace may already be gone, or never added by us; either way
    // the plugin is out, which is the thing that mattered. Reported rather
    // than fatal.
    if let Err(error) = run(&claude, &remove_marketplace) {
        println!("    (leaving the marketplace: {error})");
    }

    // Ask the vendor, not the filesystem.
    if let Some(still) = installed_version(&claude) {
        return Err(format!(
            "claude still lists {PLUGIN_ID} at {still} after uninstalling it.\n  \
             `claude plugin list` is the authority here, so this is a real \
             failure rather than\n  a stale cache — remove it by hand with \
             `claude plugin uninstall {PLUGIN_ID}`."
        ));
    }
    println!("claude plugin list no longer names it");
    Ok(())
}

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

/// The version Claude Code has installed, if it has this plugin at all.
///
/// Parsed from `claude plugin list`, which prints an id line followed by
/// indented `Key: value` lines:
///
/// ```text
///   ❯ synveda@synveda
///     Version: 0.1.2
///     Scope: user
/// ```
///
/// The presence of the id was all this used to look at, and presence is the
/// wrong question after an upgrade — see the call site.
fn installed_version(claude: &Path) -> Option<String> {
    let out = Command::new(claude)
        .args(["plugin", "list"])
        .output()
        .ok()
        .filter(|out| out.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut lines = text.lines().skip_while(|line| !line.contains(PLUGIN_ID));
    // The id line itself, so the search starts at this plugin's own fields.
    lines.next()?;
    lines
        // `@` starts the next plugin's id line, and stopping there keeps a
        // plugin with no version from borrowing the following one's.
        .take_while(|line| !line.contains('@'))
        .find_map(|line| {
            line.trim()
                .strip_prefix("Version:")
                .map(|version| version.trim().to_owned())
        })
}

/// The version the bundle on disk carries, from the plugin manifest the
/// marketplace points at.
fn bundle_version(marketplace: &Path) -> Option<String> {
    let manifest = marketplace
        .join(MARKETPLACE)
        .join(".claude-plugin/plugin.json");
    let text = std::fs::read_to_string(manifest).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("version")?.as_str().map(str::to_owned)
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

    /// `MARKETPLACE` is what `claude plugin marketplace update` is given, and
    /// it is also where `bundle_version` looks for the manifest. Drift here
    /// is silent in both: the refresh updates nothing by that name, and the
    /// version read comes back `None`, which reads as "no version to compare"
    /// rather than as a wrong path.
    #[test]
    fn the_marketplace_name_is_the_second_half_of_the_id() {
        let (_, marketplace) = PLUGIN_ID.split_once('@').expect("an id of the right shape");
        assert_eq!(MARKETPLACE, marketplace);
    }

    /// The version comparison that decides whether an upgrade replaces the
    /// installed plugin, read from the manifest the marketplace points at.
    #[test]
    fn the_bundle_version_comes_from_the_plugin_manifest() {
        let root = scratch("bundle-version");
        let manifest = root.join(MARKETPLACE).join(".claude-plugin");
        std::fs::create_dir_all(&manifest).unwrap();
        std::fs::write(
            manifest.join("plugin.json"),
            r#"{"name": "synveda", "version": "9.9.9"}"#,
        )
        .unwrap();
        assert_eq!(bundle_version(&root).as_deref(), Some("9.9.9"));

        // A bundle with no manifest is `None` rather than a panic: it is
        // what `--from` pointed at something wrong looks like, and the
        // install still has to reach `validate`'s error rather than die here.
        assert_eq!(bundle_version(&scratch("bundle-empty")), None);
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

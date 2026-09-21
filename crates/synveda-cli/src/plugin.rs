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

    validate_scope(&plan.scope)?;
    let root = project_root()?;
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
    let remove = removal_args(&plan.scope);

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

    // Inventory errors must fail before mutating native registration.
    let before = installed_plugins(&claude, &root)?;
    let have = scoped_plugin(&before, &plan.scope, &root)?;
    require_scoped_removal(&claude)?;
    let want = bundle_version(&marketplace)
        .ok_or_else(|| "the Synveda bundle has no readable version".to_owned())?;
    let marketplace_exists = verify_marketplace_source(&claude, &root, &marketplace)?;
    if let Some(have) = have.filter(|have| !plan.force && have.version == want) {
        report_registration(have);
        return Ok(());
    }
    if !marketplace_exists {
        run(&claude, &root, &add)?;
    }
    run(&claude, &root, &refresh)?;

    if let Some(have) = have {
        println!(
            "    installed {}, bundle {want} — replacing scope {}",
            have.version, plan.scope
        );
        run(&claude, &root, &remove)?;
    }
    run(&claude, &root, &install).map_err(|error| format!(
        "{error}; setup may be partial in scope {}. Persistent data and marketplace are retained; rerun the same install command to repair it",
        plan.scope,
    ))?;
    let after = installed_plugins(&claude, &root)?;
    verify_other_scopes(&before, &after, &plan.scope, &root)?;
    let registered = scoped_plugin(&after, &plan.scope, &root)?
        .filter(|plugin| plugin.version == want)
        .ok_or_else(|| "Claude did not confirm the requested plugin version and scope; inspect `claude plugin list --json`".to_owned())?;
    report_registration(registered);

    println!();
    println!("    start a new Claude Code session and review hook/MCP trust to load it.");
    println!();
    println!("    It needs a login to do anything: `synveda login` stores the");
    println!("    bearer, and the plugin reads it per call. Check registration with");
    println!("    `claude plugin list`.");
    Ok(())
}

/// Removal names one native registration, never every scope of a plugin.
pub struct RemovePlan {
    pub client: String,
    pub dry_run: bool,
    pub scope: String,
}

/// Remove one native registration, preserving persistent data and the shared
/// marketplace. Native inventory is configuration evidence, not live unloading.
pub fn uninstall(plan: &RemovePlan) -> Result<(), String> {
    if !CLIENTS.contains(&plan.client.as_str()) {
        return Err(format!(
            "unknown client {:?}; known clients are {}",
            plan.client,
            CLIENTS.join(", ")
        ));
    }
    validate_scope(&plan.scope)?;
    let root = project_root()?;
    let claude = which("claude");
    let remove_plugin = removal_args(&plan.scope);
    if plan.dry_run {
        println!("synveda plugin uninstall --dry-run");
        println!("  repository   {}", root.display());
        println!("  would run    claude {}", remove_plugin.join(" "));
        println!("  retain the shared marketplace and persistent plugin data");
        return Ok(());
    }
    let claude = claude.ok_or_else(||
        "Claude CLI is unavailable; native installation state cannot be verified. No plugin assets or registration were removed".to_owned()
    )?;
    let before = installed_plugins(&claude, &root)?;
    let Some(plugin) = scoped_plugin(&before, &plan.scope, &root)? else {
        println!(
            "Claude lists no {PLUGIN_ID} registration in scope {} for this repository",
            plan.scope
        );
        return Ok(());
    };
    require_scoped_removal(&claude)?;
    println!(
        "removing {PLUGIN_ID} {} in scope {}",
        plugin.version, plan.scope
    );
    run(&claude, &root, &remove_plugin)?;
    let after = installed_plugins(&claude, &root)?;
    verify_other_scopes(&before, &after, &plan.scope, &root)?;
    if scoped_plugin(&after, &plan.scope, &root)?.is_some() {
        return Err(format!(
            "Claude still lists {PLUGIN_ID} in scope {} after uninstall; inspect `claude plugin list --json`",
            plan.scope
        ));
    }
    println!(
        "Removed the {} registration; retained marketplace and persistent data. Restart Claude to unload active sessions.",
        plan.scope
    );
    Ok(())
}

/// `--from` selects a packaged marketplace; the installed default is
/// `$SYNVEDA_HOME/plugin`. A bare plugin directory cannot be registered.
fn locate(from: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = from {
        return validate(path).map(Path::to_path_buf);
    }
    let home = synveda_home()?;
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

pub(crate) fn synveda_home() -> Result<PathBuf, String> {
    synveda_home_from(std::env::var_os("SYNVEDA_HOME"), std::env::var_os("HOME"))
}

fn synveda_home_from(
    explicit: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    if let Some(path) = explicit.filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    home.filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".synveda"))
        .ok_or_else(|| {
            "neither SYNVEDA_HOME nor HOME is set, so there is nowhere to look for an installed release"
                .to_owned()
        })
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

/// Native registration evidence, distinct from a running session's load state.
#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct InstalledPlugin {
    id: String,
    version: String,
    scope: String,
    enabled: bool,
    #[serde(rename = "projectPath")]
    project_path: Option<PathBuf>,
}

fn validate_scope(scope: &str) -> Result<(), String> {
    if ["user", "project", "local"].contains(&scope) {
        Ok(())
    } else {
        Err("plugin scope must be user, project or local".to_owned())
    }
}

pub(crate) fn project_root() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|error| format!("resolve the current repository: {error}"))?;
    Ok(cwd
        .ancestors()
        .find(|directory| directory.join(".git").exists())
        .unwrap_or(&cwd)
        .to_path_buf())
}

fn is_scope(plugin: &InstalledPlugin, scope: &str, root: &Path) -> bool {
    plugin.id == PLUGIN_ID
        && plugin.scope == scope
        && (scope == "user"
            || plugin
                .project_path
                .as_ref()
                .and_then(|path| path.canonicalize().ok())
                .as_deref()
                == Some(root))
}

fn scoped_plugin<'a>(
    plugins: &'a [InstalledPlugin],
    scope: &str,
    root: &Path,
) -> Result<Option<&'a InstalledPlugin>, String> {
    // A project/local record without a usable path is not proof of absence in this
    // repository. Refuse unknown native schema rather than guessing its owner.
    if plugins.iter().any(|plugin| {
        plugin.id == PLUGIN_ID
            && plugin.scope == scope
            && scope != "user"
            && plugin
                .project_path
                .as_ref()
                .is_none_or(|path| !path.is_absolute() || path.canonicalize().is_err())
    }) {
        return Err(
            "Claude inventory omits project ownership; cannot safely select a scope".to_owned(),
        );
    }
    let mut matches = plugins
        .iter()
        .filter(|plugin| is_scope(plugin, scope, root));
    let selected = matches.next();
    if matches.next().is_some() {
        return Err("ambiguous Claude registration in the selected scope".to_owned());
    }
    Ok(selected)
}

fn installed_plugins(claude: &Path, root: &Path) -> Result<Vec<InstalledPlugin>, String> {
    let output = Command::new(claude)
        .current_dir(root)
        .args(["plugin", "list", "--json"])
        .output()
        .map_err(|error| format!("read Claude plugin inventory: {error}"))?;
    if !output.status.success() {
        return Err(
            "Claude plugin inventory failed; no successful installation/removal can be inferred"
                .to_owned(),
        );
    }
    serde_json::from_slice(&output.stdout).map_err(|_| {
        "Claude plugin inventory is not the supported JSON shape; no registration was selected"
            .to_owned()
    })
}

fn verify_marketplace_source(
    claude: &Path,
    root: &Path,
    marketplace: &Path,
) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Marketplace {
        name: String,
        source: String,
        path: Option<PathBuf>,
    }
    let output = Command::new(claude)
        .current_dir(root)
        .args(["plugin", "marketplace", "list", "--json"])
        .output()
        .map_err(|error| format!("read Claude marketplace inventory: {error}"))?;
    if !output.status.success() {
        return Err(
            "Claude marketplace inventory failed; its source cannot be safely replaced".to_owned(),
        );
    }
    let entries: Vec<Marketplace> = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Claude marketplace inventory is not the supported JSON shape".to_owned())?;
    let matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry.name == MARKETPLACE)
        .collect();
    if matches.is_empty() {
        return Ok(false);
    }
    let expected = marketplace
        .canonicalize()
        .map_err(|error| format!("resolve marketplace: {error}"))?;
    if matches.len() != 1
        || matches[0].source != "directory"
        || matches[0]
            .path
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            != Some(&expected)
    {
        return Err("the Synveda marketplace name belongs to another source; inspect it in Claude before changing registration".to_owned());
    }
    Ok(true)
}

fn verify_other_scopes(
    before: &[InstalledPlugin],
    after: &[InstalledPlugin],
    scope: &str,
    root: &Path,
) -> Result<(), String> {
    let before: Vec<_> = before
        .iter()
        .filter(|plugin| !is_scope(plugin, scope, root))
        .collect();
    let after: Vec<_> = after
        .iter()
        .filter(|plugin| !is_scope(plugin, scope, root))
        .collect();
    if before.len() != after.len() || before.iter().any(|entry| !after.contains(entry)) {
        return Err("Claude changed another registration during setup; inspect native plugin state before retrying. Synveda did not edit the cache".to_owned());
    }
    Ok(())
}

fn removal_args(scope: &str) -> Vec<String> {
    [
        "plugin",
        "uninstall",
        PLUGIN_ID,
        "--scope",
        scope,
        "--keep-data",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn require_scoped_removal(claude: &Path) -> Result<(), String> {
    let output = Command::new(claude)
        .args(["plugin", "uninstall", "--help"])
        .output()
        .map_err(|error| format!("inspect Claude removal capabilities: {error}"))?;
    let help = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !help.contains("--scope") || !help.contains("--keep-data") {
        return Err("this Claude version cannot preserve scope and persistent data; use a supported Claude Code CLI (verified with 2.1.241)".to_owned());
    }
    Ok(())
}

fn report_registration(plugin: &InstalledPlugin) {
    println!(
        "{PLUGIN_ID} {} configured in scope {}; {}",
        plugin.version,
        plugin.scope,
        if plugin.enabled {
            "enabled; review required to verify hook/MCP loading"
        } else {
            "disabled; enable it in Claude when ready"
        }
    );
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

/// The managed route compares vendor-owned registration, never cache files.
pub(crate) fn registration(scope: &str) -> Result<Option<serde_json::Value>, String> {
    validate_scope(scope)?;
    let root = project_root()?;
    let claude =
        which("claude").ok_or("Claude CLI is unavailable; registration cannot be verified")?;
    let inventory = installed_plugins(&claude, &root)?;
    Ok(scoped_plugin(&inventory, scope, &root)?.map(|plugin| {
        serde_json::json!({
            "version": plugin.version, "scope": plugin.scope, "enabled": plugin.enabled,
            "root": if scope == "user" { None } else { Some(&root) },
        })
    }))
}

pub(crate) fn managed_bundle(from: Option<&Path>) -> Result<(PathBuf, String), String> {
    let marketplace = locate(from)?
        .canonicalize()
        .map_err(|e| format!("resolve marketplace: {e}"))?;
    let version = bundle_version(&marketplace).ok_or("plugin bundle has no readable version")?;
    if version != env!("CARGO_PKG_VERSION") {
        return Err("managed registration requires matching CLI and plugin versions".to_owned());
    }
    let runtime = marketplace.join("synveda");
    let contract: serde_json::Value = serde_json::from_slice(
        &crate::local_state::read(&runtime.join("consumer-setup.json"))?
            .ok_or("managed registration requires the updated OPS-12 plugin bundle; published v0.4.0 does not carry receipt-aware observation")?,
    ).map_err(|_| "invalid consumer hook contract")?;
    let config = crate::local_state::read(&runtime.join("dist/config.mjs"))?
        .ok_or("plugin config runtime is missing")?;
    if contract["version"] != 1
        || contract["contract"] != "OPS-12/ADR-0116"
        || contract["config_sha256"].as_str() != Some(crate::local_state::digest(&config).as_str())
    {
        return Err("plugin observation runtime differs from its consumer contract; re-extract the matching bundle".to_owned());
    }
    Ok((marketplace, version))
}

pub(crate) fn verify_managed_source(marketplace: &Path) -> Result<(), String> {
    let claude =
        which("claude").ok_or("Claude CLI is unavailable; marketplace cannot be verified")?;
    if !verify_marketplace_source(&claude, &project_root()?, marketplace)? {
        return Err(
            "the receipt's native marketplace is absent; registration source cannot be verified"
                .to_owned(),
        );
    }
    Ok(())
}

fn run(claude: &Path, root: &Path, args: &[String]) -> Result<(), String> {
    println!("    claude {}", args.join(" "));
    let status = Command::new(claude)
        .current_dir(root)
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
    use std::ffi::OsString;

    fn scratch(what: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("synveda-plugin-{what}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn installed_home_never_falls_back_to_a_relative_empty_path() {
        assert_eq!(
            synveda_home_from(
                Some(OsString::from("/srv/synveda")),
                Some(OsString::from("/home/operator")),
            )
            .unwrap(),
            PathBuf::from("/srv/synveda"),
        );
        assert_eq!(
            synveda_home_from(
                Some(OsString::new()),
                Some(OsString::from("/home/operator")),
            )
            .unwrap(),
            PathBuf::from("/home/operator/.synveda"),
        );
        assert!(synveda_home_from(Some(OsString::new()), Some(OsString::new())).is_err());
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

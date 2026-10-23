//! Local client paths shared with the hooks through client-paths.json.

use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(crate) enum Directory {
    Config,
    State,
}

/// Unix private-state roots; native Windows callers use `resolve_directory`
/// with the ACL backend instead.
#[cfg(not(windows))]
pub(crate) fn directory(kind: Directory) -> Result<PathBuf, String> {
    require_private_state()?;
    resolve_directory(kind)
}

// Native Windows callers resolve here and use the ACL backend; `directory`
// keeps the refusal for state that has no native implementation.
pub(crate) fn resolve_directory(kind: Directory) -> Result<PathBuf, String> {
    resolve(kind, cfg!(windows), |name| match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8")),
    })
    .map(PathBuf::from)
}

fn resolve(
    kind: Directory,
    windows: bool,
    env: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<String, String> {
    let variable = match kind {
        Directory::Config => "XDG_CONFIG_HOME",
        Directory::State => "XDG_STATE_HOME",
    };
    if let Some(value) = env(variable)? {
        if absolute(&value, windows) {
            return Ok(append(&value, "synveda", windows));
        }
        // A Windows network/device path is not an ignored relative override:
        // it could reach another machine while resolving private state.
        if windows && (value.starts_with(['/', '\\']) || value.contains(':')) {
            return Err(format!(
                "{variable} must be a fully qualified local drive path"
            ));
        }
    }
    let variable = if windows { "LOCALAPPDATA" } else { "HOME" };
    let base = env(variable)?.ok_or_else(|| format!("{variable} is not set"))?;
    if !absolute(&base, windows) {
        return Err(format!(
            "{variable} must be an absolute {}path",
            if windows { "local drive " } else { "" }
        ));
    }
    let suffix = match (windows, kind) {
        (true, Directory::Config) => "synveda/config",
        (true, Directory::State) => "synveda/state",
        (false, Directory::Config) => ".config/synveda",
        (false, Directory::State) => ".local/state/synveda",
    };
    Ok(append(&base, suffix, windows))
}

fn absolute(value: &str, windows: bool) -> bool {
    if !windows {
        return value.starts_with('/');
    }
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
        && !value[2..].contains(':')
}

fn append(base: &str, suffix: &str, windows: bool) -> String {
    let base = if windows {
        base.replace('\\', "/")
    } else {
        base.to_owned()
    };
    format!("{}/{suffix}", base.trim_end_matches('/'))
}

/// Rust canonicalizes a Windows drive path to `\\?\C:\...`; hooks and vendor
/// clients name the same directory `C:\...`. Drop only that verbatim drive
/// prefix so receipts and generated commands match what those programs see.
pub(crate) fn conventional(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        if let Some(Component::Prefix(prefix)) = components.next()
            && let Prefix::VerbatimDisk(drive) = prefix.kind()
        {
            let mut plain = PathBuf::from(format!("{}:\\", char::from(drive)));
            plain.push(components.as_path());
            return plain;
        }
    }
    path
}

/// This binary's absolute path, in the spelling a vendor client or hook will
/// launch. Through symlinks, because a client that cannot read this path is a
/// server that never starts, and `current_exe` may hand back a shim.
pub(crate) fn current_executable() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|err| format!("locate this binary: {err}"))?;
    let exe = conventional(std::fs::canonicalize(&exe).unwrap_or(exe));
    exe.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("this binary's path is not valid UTF-8: {}", exe.display()))
}

/// The account home a vendor client resolves: `USERPROFILE` on Windows, where
/// Node's `os.homedir()` reads it, and `HOME` elsewhere.
pub(crate) fn vendor_home() -> Result<PathBuf, String> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{variable} is not set"))?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(format!("{variable} must be an absolute path"));
    }
    Ok(home)
}

/// Compose deployment: the native Windows client never runs the server stack.
pub(crate) const DEPLOYMENT: &str = "Compose deployment (`up`, `down`, `status`, `logs`, `doctor`)";

/// Refuse a surface that has no native implementation here, naming it.
/// Native Windows covers every client surface except the Compose deployment
/// commands (ADR-0117 amendments 1-4); portable locks and chmod stubs are never
/// treated as Windows privacy enforcement.
pub(crate) fn require_unix(surface: &str) -> Result<(), String> {
    if cfg!(unix) {
        Ok(())
    } else if cfg!(windows) {
        Err(format!(
            "{surface} is not supported on native Windows: this client does not run \
             the Synveda server there. Use the client to connect to a deployment that runs on \
             Linux or macOS, or run this command inside WSL2"
        ))
    } else {
        Err(format!(
            "{surface} is unavailable: this build has no private client-state backend for {}",
            std::env::consts::OS
        ))
    }
}

/// State that still uses the portable Unix descriptor/mode implementation.
pub(crate) fn require_private_state() -> Result<(), String> {
    require_unix("this command's private local state")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn conventional_drops_only_the_verbatim_drive_prefix() {
        assert_eq!(
            conventional(PathBuf::from(r"\\?\C:\Users\a b\文")),
            PathBuf::from(r"C:\Users\a b\文")
        );
        for unchanged in [r"\\?\UNC\server\share\x", r"C:\Users\x", r"\\.\pipe\x"] {
            assert_eq!(
                conventional(PathBuf::from(unchanged)),
                PathBuf::from(unchanged)
            );
        }
    }

    #[test]
    fn cli_and_hooks_share_the_platform_path_contract() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../adapters/fixtures/client-paths.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            for (kind, key) in [(Directory::Config, "config"), (Directory::State, "state")] {
                let result = resolve(kind, case["platform"] == "win32", |name| {
                    Ok(case["env"][name].as_str().map(str::to_owned))
                });
                if let Some(error) = case["error"].as_str() {
                    assert!(result.unwrap_err().contains(error), "{}", case["name"]);
                } else {
                    assert_eq!(
                        result.unwrap(),
                        case[key].as_str().unwrap(),
                        "{}",
                        case["name"]
                    );
                }
            }
        }
    }
}

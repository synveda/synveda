//! Local client paths shared with the hooks through client-paths.json (OPS-12).

use std::path::PathBuf;

#[derive(Clone, Copy)]
pub(crate) enum Directory {
    Config,
    State,
}

pub(crate) fn directory(kind: Directory) -> Result<PathBuf, String> {
    require_private_state()?;
    resolve_directory(kind)
}

// Only credential storage has native Windows privacy enforcement so far.
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

/// Portable locks and chmod stubs are not Windows privacy enforcement.
pub(crate) fn require_private_state() -> Result<(), String> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err("private client state is unavailable on this platform until native ACL and atomic replacement support is qualified (OPS-12)".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

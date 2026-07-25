//! The credentials file (ADPT-1, ADR-0027 decision 6).
//!
//! `$XDG_CONFIG_HOME/synveda/credentials.json`, mode 0600, keyed by
//! profile: gateway URL, issuer, tenant, subject, access token, expiry,
//! refresh token. `synveda login` writes it; `synveda auth token` reads
//! it and rewrites it when a refresh renews the access token; the Claude
//! Code adapter never opens it at all — it shells out to the CLI, which
//! is the sole credential authority (ADR-0027 decision 4).
//!
//! It never enters `settings.json`, the environment, or a transcript.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The default profile name, for the overwhelmingly common case of one
/// user against one gateway.
pub const DEFAULT_PROFILE: &str = "default";

/// The on-disk file. Versioned so a later format change is a migration
/// rather than a parse failure the user has to debug.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    /// Format version; 1 is the ADPT-1 shape.
    pub version: u32,
    /// Stored logins by profile name.
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

/// One logged-in identity against one gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// The gateway this login belongs to. It is also the gateway the
    /// adapter posts to: `synveda login` is what binds a machine to a
    /// gateway, and a project file must not be able to redirect a bearer
    /// somewhere else.
    pub gateway_url: String,
    /// The issuer that authenticated the login — a refresh names the same
    /// token endpoint and client.
    pub issuer: String,
    /// The tenant the login resolved to (TEN-1).
    pub tenant_id: String,
    /// Display-only tenant handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_slug: Option<String>,
    /// The token subject.
    pub subject: String,
    /// The `/v1` bearer.
    pub access_token: String,
    /// Token type as the IdP reported it (`Bearer`).
    pub token_type: String,
    /// When the access token expires. `None` for an issuer that reported
    /// no lifetime: the token is then used until the gateway rejects it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The refresh token, when the issuer granted one. Its absence is
    /// what makes a login eventually need repeating (ADR-0027 decision 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

impl Profile {
    /// Whether the access token is still usable `skew` from now. An
    /// unknown expiry counts as valid: the gateway is the authority on
    /// that, and refusing to try would be worse than a 401.
    pub fn valid_for(&self, skew: chrono::Duration) -> bool {
        self.expires_at
            .is_none_or(|expires_at| expires_at > Utc::now() + skew)
    }
}

/// `$XDG_CONFIG_HOME/synveda`, else `~/.config/synveda` — the same rule
/// the adapter's `paths.mts` applies, so both agree on where the file is.
pub fn config_dir() -> Result<PathBuf, String> {
    if let Ok(configured) = std::env::var("XDG_CONFIG_HOME")
        && configured.starts_with('/')
    {
        return Ok(PathBuf::from(configured).join("synveda"));
    }
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_owned())?;
    Ok(PathBuf::from(home).join(".config").join("synveda"))
}

/// The credentials file path.
pub fn path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("credentials.json"))
}

/// Reads the file. A missing file is an empty set, not an error — that is
/// the state of a machine that has never logged in.
pub fn load() -> Result<Credentials, String> {
    let path = path()?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Credentials {
                version: 1,
                profiles: BTreeMap::new(),
            });
        }
        Err(err) => return Err(format!("read {}: {err}", path.display())),
    };
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "{} is not a valid credentials file ({err}); \
             remove it and run `synveda login`",
            path.display()
        )
    })
}

/// Reads one profile, or reports what to do about its absence.
pub fn profile(name: &str) -> Result<Profile, String> {
    load()?
        .profiles
        .remove(name)
        .ok_or_else(|| format!("no credentials for profile `{name}`; run `synveda login` first"))
}

/// Writes one profile, leaving every other profile as it was.
pub fn store(name: &str, profile: Profile) -> Result<(), String> {
    let mut credentials = load()?;
    credentials.profiles.insert(name.to_owned(), profile);
    save(&credentials)
}

/// Writes the whole file. It goes to a 0600 temporary alongside the real
/// path and is renamed into place, so a crash mid-write cannot leave a
/// half-written credentials file — and so the secret is never briefly
/// world-readable.
pub fn save(credentials: &Credentials) -> Result<(), String> {
    let credentials = Credentials {
        version: 1,
        profiles: credentials.profiles.clone(),
    };
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
    restrict_dir(&dir)?;

    let body = serde_json::to_string_pretty(&credentials)
        .map_err(|err| format!("serialize credentials: {err}"))?;

    let path = path()?;
    let temporary = path.with_extension("json.tmp");
    write_private(&temporary, &body)?;
    std::fs::rename(&temporary, &path).map_err(|err| {
        // Leave nothing behind holding a token if the rename failed.
        let _ = std::fs::remove_file(&temporary);
        format!("write {}: {err}", path.display())
    })
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, body: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| format!("open {}: {err}", path.display()))?;
    file.write_all(body.as_bytes())
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    file.sync_all()
        .map_err(|err| format!("sync {}: {err}", path.display()))
}

/// Windows has no mode bits; the file inherits the user profile
/// directory's ACL, which is already user-only. Called out rather than
/// silently skipped: "0600" is a promise this platform keeps differently.
#[cfg(not(unix))]
fn write_private(path: &std::path::Path, body: &str) -> Result<(), String> {
    std::fs::write(path, body).map_err(|err| format!("write {}: {err}", path.display()))
}

#[cfg(unix)]
fn restrict_dir(dir: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("chmod {}: {err}", dir.display()))
}

#[cfg(not(unix))]
fn restrict_dir(_dir: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points HOME and XDG_CONFIG_HOME at a scratch directory for one
    /// test. Serialised, because the environment is process-global.
    struct Scratch {
        dir: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let dir = std::env::temp_dir().join(format!("synveda-cli-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            // SAFETY: the lock makes this the only thread touching the
            // environment for the duration of the test.
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", &dir);
            }
            Self { dir, _guard: guard }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn sample(gateway: &str) -> Profile {
        Profile {
            gateway_url: gateway.to_owned(),
            issuer: "http://idp.test".to_owned(),
            tenant_id: "0198f000-0000-7000-8000-000000000000".to_owned(),
            tenant_slug: Some("acme".to_owned()),
            subject: "alice@example.test".to_owned(),
            access_token: "at".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + chrono::Duration::seconds(600)),
            refresh_token: Some("rt".to_owned()),
        }
    }

    #[test]
    fn a_missing_file_reads_as_no_credentials() {
        let _scratch = Scratch::new("missing");
        assert!(load().expect("load").profiles.is_empty());
        let err = profile(DEFAULT_PROFILE).expect_err("no profile");
        assert!(err.contains("synveda login"), "unhelpful message: {err}");
    }

    #[test]
    fn profiles_round_trip_and_do_not_clobber_each_other() {
        let _scratch = Scratch::new("profiles");
        store(DEFAULT_PROFILE, sample("http://127.0.0.1:8120")).expect("store default");
        store("work", sample("https://synveda.corp.test")).expect("store work");

        let loaded = load().expect("load");
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(
            loaded.profiles[DEFAULT_PROFILE].gateway_url,
            "http://127.0.0.1:8120"
        );
        assert_eq!(
            profile("work").expect("work").gateway_url,
            "https://synveda.corp.test"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_credentials_file_is_never_readable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt;

        let _scratch = Scratch::new("modes");
        store(DEFAULT_PROFILE, sample("http://127.0.0.1:8120")).expect("store");
        // Rewrite: the mode must hold on the replacement too, not only on
        // first creation.
        store(DEFAULT_PROFILE, sample("http://127.0.0.1:8120")).expect("re-store");

        let file = std::fs::metadata(path().expect("path")).expect("stat file");
        assert_eq!(file.permissions().mode() & 0o777, 0o600);
        let dir = std::fs::metadata(config_dir().expect("dir")).expect("stat dir");
        assert_eq!(dir.permissions().mode() & 0o777, 0o700);
        // And no temporary is left holding the same secret.
        assert!(
            !path().expect("path").with_extension("json.tmp").exists(),
            "the temporary must not survive the rename"
        );
    }

    #[test]
    fn a_corrupt_file_says_what_to_do_about_it() {
        let _scratch = Scratch::new("corrupt");
        std::fs::create_dir_all(config_dir().expect("dir")).expect("mkdir");
        std::fs::write(path().expect("path"), "{ not json").expect("write");
        let err = load().expect_err("corrupt file must not parse");
        assert!(err.contains("synveda login"), "unhelpful message: {err}");
    }

    #[test]
    fn validity_accounts_for_skew_and_unknown_expiry() {
        let skew = chrono::Duration::seconds(60);
        let mut profile = sample("http://127.0.0.1:8120");
        assert!(profile.valid_for(skew), "ten minutes of life is valid");

        // Inside the skew window: treat as expired, so the refresh happens
        // before the call rather than after a 401.
        profile.expires_at = Some(Utc::now() + chrono::Duration::seconds(30));
        assert!(!profile.valid_for(skew));

        profile.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        assert!(!profile.valid_for(skew));

        // An issuer that reported no lifetime: the gateway decides.
        profile.expires_at = None;
        assert!(profile.valid_for(skew));
    }
}

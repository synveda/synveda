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
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub(crate) mod windows;
#[cfg(any(windows, test))]
mod windows_acl;

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
    crate::client_paths::directory(crate::client_paths::Directory::Config)
}

/// The credentials file path.
pub fn path() -> Result<PathBuf, String> {
    Ok(storage_dir()?.join("credentials.json"))
}

fn storage_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        crate::client_paths::resolve_directory(crate::client_paths::Directory::Config)
    }
    #[cfg(not(windows))]
    {
        config_dir()
    }
}

/// Refuse unsafe storage before starting an issuer/browser round trip.
pub(crate) fn preflight() -> Result<(), String> {
    #[cfg(windows)]
    {
        load().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        crate::client_paths::require_private_state()
    }
}

/// Reads the file. A missing file is an empty set, not an error — that is
/// the state of a machine that has never logged in.
pub fn load() -> Result<Credentials, String> {
    load_at(&path()?)
}

fn load_at(path: &Path) -> Result<Credentials, String> {
    #[cfg(windows)]
    let raw = match windows::read(path).map_err(|err| format!("read private credentials: {err}"))? {
        Some(raw) => raw,
        None => {
            return Ok(Credentials {
                version: 1,
                profiles: BTreeMap::new(),
            });
        }
    };
    #[cfg(not(windows))]
    crate::client_paths::require_private_state()?;
    #[cfg(not(windows))]
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Credentials {
                version: 1,
                profiles: BTreeMap::new(),
            });
        }
        Err(err) => return Err(format!("read {}: {err}", path.display())),
    };
    serde_json::from_slice(&raw).map_err(|err| {
        format!(
            "{} is not a valid credentials file (line {}, column {}); \
             preserve it for repair or restore a private backup before `synveda login`",
            path.display(),
            err.line(),
            err.column()
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
pub async fn store(name: &str, profile: Profile) -> Result<(), String> {
    lock().await?.store(name, profile)
}

/// OPS-12: one stable lock for the whole profile file, including the refresh
/// request. Locking credentials.json itself would lose exclusion on rename.
pub(crate) struct LockedCredentials {
    _lock: File,
    path: PathBuf,
    #[cfg(windows)]
    _directory: windows::Directory,
}

pub(crate) async fn lock() -> Result<LockedCredentials, String> {
    lock_with_timeout(Duration::from_secs(20)).await
}

async fn lock_with_timeout(timeout: Duration) -> Result<LockedCredentials, String> {
    let dir = storage_dir()?;
    #[cfg(windows)]
    let directory = windows::Directory::open(&dir, true)
        .map_err(|err| format!("prepare private credentials: {err}"))?
        .ok_or("private credential directory is absent")?;
    #[cfg(windows)]
    let file = directory
        .lock_file()
        .map_err(|err| format!("open credential lock: {err}"))?;
    let lock_path = dir.join("credentials.lock");
    #[cfg(not(windows))]
    let file = {
        std::fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
        let metadata = std::fs::symlink_metadata(&dir)
            .map_err(|err| format!("inspect credential directory: {err}"))?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(
                "credential directory must be a private directory, not a symlink".to_owned(),
            );
        }
        restrict_dir(&dir)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        options
            .open(&lock_path)
            .map_err(|err| format!("open credential lock: {err}"))?
    };
    validate_lock(&file, &lock_path)?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if started.elapsed() < timeout => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(TryLockError::WouldBlock) => {
                return Err("credentials are in use by another Synveda process; retry after it finishes; do not remove credentials.lock".to_owned());
            }
            Err(TryLockError::Error(err)) => return Err(format!("lock credentials: {err}")),
        }
    }
    validate_lock(&file, &lock_path)?;
    Ok(LockedCredentials {
        _lock: file,
        path: dir.join("credentials.json"),
        #[cfg(windows)]
        _directory: directory,
    })
}

fn validate_lock(file: &File, path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let parent = path.parent().ok_or("credential lock has no parent")?;
        let directory = windows::Directory::open(parent, false)
            .map_err(|err| format!("inspect credential directory: {err}"))?
            .ok_or("credential directory disappeared")?;
        directory
            .validate(file, "credentials.lock")
            .map_err(|err| format!("inspect credential lock: {err}"))?;
    }
    let opened = file
        .metadata()
        .map_err(|err| format!("inspect credential lock: {err}"))?;
    let named = std::fs::symlink_metadata(path)
        .map_err(|err| format!("inspect credential lock path: {err}"))?;
    if !opened.is_file() || !named.is_file() || named.is_symlink() || opened.len() != 0 {
        return Err("credential lock must be an empty regular file".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.uid() != rustix::process::geteuid().as_raw()
            || opened.mode() & 0o7777 != 0o600
            || opened.nlink() != 1
            || opened.dev() != named.dev()
            || opened.ino() != named.ino()
        {
            return Err(
                "credential lock ownership, privacy or file identity was refused".to_owned(),
            );
        }
    }
    Ok(())
}

impl LockedCredentials {
    pub(crate) fn profile(&self, name: &str) -> Result<Profile, String> {
        load_at(&self.path)?.profiles.remove(name).ok_or_else(|| {
            format!("no credentials for profile `{name}`; run `synveda login` first")
        })
    }

    pub(crate) fn store(&self, name: &str, profile: Profile) -> Result<(), String> {
        let mut credentials = load_at(&self.path)?;
        credentials.profiles.insert(name.to_owned(), profile);
        save_at(&self.path, &credentials)
    }

    pub(crate) fn forget(&self, name: Option<&str>) -> Result<usize, String> {
        let mut credentials = load_at(&self.path)?;
        let count = if let Some(name) = name {
            credentials
                .profiles
                .remove(name)
                .ok_or_else(|| format!("no credentials for profile `{name}`"))?;
            1
        } else {
            let count = credentials.profiles.len();
            credentials.profiles.clear();
            count
        };
        save_at(&self.path, &credentials)?;
        Ok(count)
    }
}

/// Writes the whole file. It goes to a 0600 temporary alongside the real
/// path and is renamed into place, so a crash mid-write cannot leave a
/// half-written credentials file — and so the secret is never briefly
/// world-readable.
fn save_at(path: &Path, credentials: &Credentials) -> Result<(), String> {
    let credentials = Credentials {
        version: 1,
        profiles: credentials.profiles.clone(),
    };
    let body = serde_json::to_string_pretty(&credentials)
        .map_err(|err| format!("serialize credentials: {err}"))?;

    #[cfg(windows)]
    {
        windows::write(path, body.as_bytes())
            .map_err(|err| format!("write private credentials: {err}"))
    }

    #[cfg(not(windows))]
    {
        save_private(path, &body)
    }
}

#[cfg(not(windows))]
fn save_private(path: &Path, body: &str) -> Result<(), String> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|err| format!("credential temporary name: {err}"))?;
    let nonce = u128::from_be_bytes(nonce);
    let temporary = path.with_extension(format!("json.{nonce:032x}.tmp"));
    write_private(&temporary, body)?;
    std::fs::rename(&temporary, path).map_err(|err| {
        // Leave nothing behind holding a token if the rename failed.
        let _ = std::fs::remove_file(&temporary);
        format!("write {}: {err}", path.display())
    })
}

#[cfg(not(windows))]
fn write_private(path: &std::path::Path, body: &str) -> Result<(), String> {
    use std::io::Write;

    crate::client_paths::require_private_state()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| format!("open {}: {err}", path.display()))?;
    let result = file
        .write_all(body.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|err| format!("write private credentials: {err}"));
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

#[cfg(unix)]
fn restrict_dir(dir: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("chmod {}: {err}", dir.display()))
}

#[cfg(not(any(unix, windows)))]
fn restrict_dir(_dir: &std::path::Path) -> Result<(), String> {
    crate::client_paths::require_private_state()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("credential test runtime")
    }

    fn store(name: &str, profile: Profile) -> Result<(), String> {
        runtime().block_on(super::store(name, profile))
    }

    /// Points HOME and XDG_CONFIG_HOME at a scratch directory for one
    /// test. Serialised, because the environment is process-global.
    struct Scratch {
        dir: PathBuf,
        _guard: tokio::sync::MutexGuard<'static, ()>,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            // The binary's one environment lock rather than a private one:
            // `XDG_CONFIG_HOME` is read by the same credential resolution
            // `api::tests` reaches through, so a lock only this module
            // knows about is a lock that does not serialise the pair. See
            // `crate::testing`.
            let guard = crate::testing::ENV.blocking_lock();
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

    #[cfg(unix)]
    #[test]
    fn credential_path_refuses_non_unicode_environment_input() {
        use std::os::unix::ffi::OsStringExt as _;

        let _guard = crate::testing::ENV.blocking_lock();
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::set_var(
                "XDG_CONFIG_HOME",
                std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
            );
        }
        let error = config_dir().expect_err("non-Unicode XDG path must be refused");
        assert_eq!(error, "XDG_CONFIG_HOME must be valid UTF-8");
        unsafe {
            match previous {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn credential_path_never_falls_back_to_the_working_directory() {
        let _guard = crate::testing::ENV.blocking_lock();
        let previous_xdg = std::env::var_os("XDG_CONFIG_HOME");
        let previous_home = std::env::var_os("HOME");
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
        for unsafe_home in ["", "relative/home"] {
            unsafe { std::env::set_var("HOME", unsafe_home) };
            assert_eq!(
                config_dir().expect_err("relative credential root must be refused"),
                "HOME must be an absolute path"
            );
        }
        unsafe {
            match previous_xdg {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
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
            std::fs::read_dir(config_dir().expect("dir"))
                .expect("entries")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
        let lock = std::fs::metadata(config_dir().expect("dir").join("credentials.lock"))
            .expect("lock file survives credential replacement");
        assert_eq!(lock.permissions().mode() & 0o777, 0o600);
        assert_eq!(lock.len(), 0, "lock contains no private material");
    }

    #[test]
    fn credential_lock_wait_is_bounded_and_cancellation_releases_it() {
        let _scratch = Scratch::new("bounded-lock");
        runtime().block_on(async {
            let held = lock().await.expect("first lock");
            let error = match lock_with_timeout(Duration::from_millis(10)).await {
                Ok(_) => panic!("second handle acquired the held lock"),
                Err(error) => error,
            };
            assert!(error.contains("do not remove credentials.lock"));
            assert!(config_dir().unwrap().join("credentials.lock").exists());
            drop(held);
            lock().await.expect("release after cancellation/drop");
        });
    }

    #[cfg(unix)]
    #[test]
    fn lock_links_and_broadened_permissions_refuse_without_changing_credentials() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let _scratch = Scratch::new("unsafe-lock");
        store(DEFAULT_PROFILE, sample("http://127.0.0.1:8120")).unwrap();
        let before = std::fs::read(path().unwrap()).unwrap();
        let lock_path = config_dir().unwrap().join("credentials.lock");
        std::fs::remove_file(&lock_path).unwrap();
        symlink(path().unwrap(), &lock_path).unwrap();
        assert!(store("work", sample("http://127.0.0.1:8121")).is_err());
        std::fs::remove_file(&lock_path).unwrap();
        let other = config_dir().unwrap().join("other-empty-file");
        std::fs::write(&other, "").unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::hard_link(other, &lock_path).unwrap();
        assert!(store("work", sample("http://127.0.0.1:8121")).is_err());
        std::fs::remove_file(&lock_path).unwrap();
        std::fs::write(&lock_path, "").unwrap();
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store("work", sample("http://127.0.0.1:8121")).is_err());
        assert_eq!(std::fs::read(path().unwrap()).unwrap(), before);
    }

    #[test]
    fn private_writes_never_truncate_existing_paths() {
        let scratch = Scratch::new("exclusive-write");
        let target = scratch.dir.join("unrelated");
        std::fs::write(&target, "keep these bytes").unwrap();
        assert!(write_private(&target, "new private material").is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "keep these bytes");
    }

    #[test]
    fn a_corrupt_file_says_what_to_do_about_it() {
        let _scratch = Scratch::new("corrupt");
        std::fs::create_dir_all(config_dir().expect("dir")).expect("mkdir");
        std::fs::write(path().expect("path"), "{ not json").expect("write");
        let err = load().expect_err("corrupt file must not parse");
        assert!(err.contains("synveda login"), "unhelpful message: {err}");
        std::fs::write(
            path().expect("path"),
            r#"{"version":"secret-fixture-value"}"#,
        )
        .expect("write invalid field");
        assert!(!load().unwrap_err().contains("secret-fixture-value"));
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

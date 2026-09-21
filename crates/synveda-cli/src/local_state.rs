//! OPS-12 local ownership evidence. This never supplies product authority.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

pub(crate) const MAX_FILE: u64 = 2 * 1024 * 1024;

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn directory() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        Ok(
            crate::client_paths::resolve_directory(crate::client_paths::Directory::Config)?
                .join("consumer"),
        )
    }
    #[cfg(not(windows))]
    {
        Ok(crate::credentials::config_dir()?.join("consumer"))
    }
}

pub(crate) fn receipt(kind: &str, identity: &str) -> Result<PathBuf, String> {
    Ok(directory()?.join(format!("{kind}-{}.json", digest(identity.as_bytes()))))
}

/// Resolve the existing ancestor without guessing a different config target.
pub(crate) fn absolute(path: &Path) -> Result<PathBuf, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if path
        .components()
        .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("configuration paths must not contain ..".to_owned());
    }
    let mut ancestor = path.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.is_symlink() {
                    return Err("configuration target must not be a symlink".to_owned());
                }
                let mut resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
                for part in suffix.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or("configuration path has no filename")?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or("configuration path has no parent")?;
            }
            Err(e) => return Err(format!("resolve configuration: {e}")),
        }
    }
}

/// Do not unlink an OS lock: a waiting process must acquire the same inode.
pub(crate) struct Lock {
    _file: File,
    #[cfg(windows)]
    _directory: crate::credentials::windows::Directory,
}

pub(crate) async fn lock() -> Result<Lock, String> {
    #[cfg(windows)]
    {
        let directory = crate::credentials::windows::Directory::open(&directory()?, true)
            .map_err(|e| format!("private receipts: {e}"))?
            .ok_or("receipt directory is absent")?;
        let file = directory
            .named_lock("operations.lock")
            .map_err(|e| e.to_string())?;
        wait_for_lock(&file).await?;
        directory
            .validate(&file, "operations.lock")
            .map_err(|e| e.to_string())?;
        Ok(Lock {
            _file: file,
            _directory: directory,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(Lock {
            _file: lock_unix().await?,
        })
    }
}

#[cfg(not(windows))]
async fn lock_unix() -> Result<File, String> {
    let dir = directory()?;
    private_directory(&dir)?;
    let path = dir.join("operations.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    secure_options(&mut options);
    let file = options
        .open(&path)
        .map_err(|e| format!("open consumer lock: {e}"))?;
    validate_file(&file, &path, true)?;
    if file.metadata().map_err(|e| e.to_string())?.len() != 0 {
        return Err("consumer lock must be empty".to_owned());
    }
    wait_for_lock(&file).await?;
    validate_file(&file, &path, true)?;
    Ok(file)
}

async fn wait_for_lock(file: &File) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if started.elapsed() < Duration::from_secs(20) => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(TryLockError::WouldBlock) => return Err(
                "another Synveda setup or lifecycle command is active; retry after it finishes; do not remove operations.lock".to_owned(),
            ),
            Err(TryLockError::Error(e)) => return Err(format!("lock consumer state: {e}")),
        }
    }
    Ok(())
}

pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    read_checked(path, false)
}

fn read_checked(path: &Path, private: bool) -> Result<Option<Vec<u8>>, String> {
    #[cfg(windows)]
    if private {
        let Some(dir) = crate::credentials::windows::Directory::open(
            path.parent().ok_or("receipt has no parent")?,
            false,
        )
        .map_err(|e| format!("private receipts: {e}"))?
        else {
            return Ok(None);
        };
        return Ok(dir
            .read(leaf(path)?)
            .map_err(|e| format!("private receipts: {e}"))?
            .map(|(_, bytes)| bytes));
    }
    crate::client_paths::require_private_state()?;
    let mut options = OpenOptions::new();
    options.read(true);
    secure_options(&mut options);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink is not an absent file that we may replace.
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(format!("{} is not a regular file", path.display()));
            }
            return Ok(None);
        }
        Err(e) => return Err(format!("open {}: {e}", path.display())),
    };
    validate_file(&file, path, private)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("{} exceeds the local file bound", path.display()));
    }
    Ok(Some(bytes))
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    read_checked(path, true)?
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|_| {
                format!(
                    "{} has an invalid document; preserve it for inspection",
                    path.display()
                )
            })
        })
        .transpose()
}

pub(crate) fn save<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let before = read_checked(path, true)?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    private_directory(path.parent().ok_or("receipt has no parent")?)?;
    replace(path, before.as_deref(), &bytes, true)
}

/// Compare again immediately before rename. The OS lock covers participating
/// CLI processes; an editor/vendor process must still be quiescent during edits.
pub(crate) fn replace(
    path: &Path,
    before: Option<&[u8]>,
    bytes: &[u8],
    private: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    if private {
        let dir = crate::credentials::windows::Directory::open(
            path.parent().ok_or("receipt has no parent")?,
            true,
        )
        .map_err(|e| format!("private receipts: {e}"))?
        .ok_or("receipt directory is absent")?;
        return dir
            .replace_if(leaf(path)?, bytes, before.map(digest).as_deref())
            .map_err(|e| format!("private receipts: {e}"));
    }
    crate::client_paths::require_private_state()?;
    let parent = path.parent().ok_or("configuration has no parent")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("create configuration directory: {e}"))?;
    let temporary = parent.join(format!(".synveda-{}.tmp", synveda_types::TenantId::new()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        secure_options(&mut options);
        let mut file = options
            .open(&temporary)
            .map_err(|e| format!("create temporary configuration: {e}"))?;
        if !private && let Ok(metadata) = std::fs::symlink_metadata(path) {
            file.set_permissions(metadata.permissions())
                .map_err(|e| format!("preserve configuration permissions: {e}"))?;
        }
        file.write_all(bytes)
            .map_err(|e| format!("write temporary configuration: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("sync temporary configuration: {e}"))?;
        if read(path)?.as_deref() != before {
            return Err(
                "configuration changed during the operation; nothing was replaced".to_owned(),
            );
        }
        std::fs::rename(&temporary, path).map_err(|e| format!("replace configuration: {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn private_directory(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::credentials::windows::Directory::open(path, true)
            .map_err(|e| format!("private receipts: {e}"))?
            .ok_or("receipt directory is absent")?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        private_directory_unix(path)
    }
}

#[cfg(not(windows))]
fn private_directory_unix(path: &Path) -> Result<(), String> {
    crate::client_paths::require_private_state()?;
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && (!metadata.is_dir() || metadata.is_symlink())
    {
        return Err("consumer state must be a private directory, not a symlink".to_owned());
    }
    std::fs::create_dir_all(path).map_err(|e| format!("create consumer directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err("consumer directory belongs to another user".to_owned());
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn remove_receipt(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let dir = crate::credentials::windows::Directory::open(
            path.parent().ok_or("receipt has no parent")?,
            false,
        )
        .map_err(|e| e.to_string())?
        .ok_or("receipt directory is absent")?;
        let before = dir.read(leaf(path)?).map_err(|e| e.to_string())?;
        dir.remove_if(
            leaf(path)?,
            before.as_ref().map(|(_, b)| digest(b)).as_deref(),
        )
        .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::fs::remove_file(path).map_err(|e| format!("remove completed receipt: {e}"))
    }
}

#[cfg(windows)]
fn leaf(path: &Path) -> Result<&str, String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "invalid receipt filename".to_owned())
}

fn secure_options(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    let _ = options;
}

fn validate_file(file: &File, path: &Path, private: bool) -> Result<(), String> {
    let opened = file.metadata().map_err(|e| e.to_string())?;
    let named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !opened.is_file() || !named.is_file() || named.is_symlink() || opened.len() > MAX_FILE {
        return Err(format!("{} must be a bounded regular file", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1
            || opened.dev() != named.dev()
            || opened.ino() != named.ino()
            || (private
                && (opened.uid() != rustix::process::geteuid().as_raw()
                    || opened.mode() & 0o7777 != 0o600))
        {
            return Err("local file ownership, privacy or identity was refused".to_owned());
        }
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(())
}

#[cfg(all(test, windows))]
#[path = "../tests/support/windows_private.rs"]
mod windows_fixture;

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[tokio::test]
    async fn receipts_use_native_private_creation_replacement_and_conflict_refusal() {
        let root = std::env::temp_dir().join(format!(
            "synveda receipts '文' {}",
            synveda_types::TenantId::new()
        ));
        std::fs::create_dir(&root).unwrap();
        windows_fixture::private(&root);
        let path = root.join("consumer/setup-fixture.json");
        let first = serde_json::json!({ "version": 1, "selection": "first" });
        let second = serde_json::json!({ "version": 1, "selection": "second" });
        save(&path, &first).unwrap();
        let before = read_checked(&path, true).unwrap().unwrap();
        save(&path, &second).unwrap();
        assert_eq!(
            read_json::<serde_json::Value>(&path).unwrap(),
            Some(second.clone())
        );
        assert!(replace(&path, Some(&before), b"stale", true).is_err());
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap(), Some(second));
        let alias = root.join("consumer/alias.json");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(read_json::<serde_json::Value>(&path).is_err());
        assert!(save(&path, &first).is_err());
        assert!(remove_receipt(&path).is_err());
        std::fs::remove_file(alias).unwrap();
        remove_receipt(&path).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

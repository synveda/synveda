//! Ordinary Windows configuration files (ADR-0117 amendment 3).
//!
//! Vendor and repository files belong to their owner, so replacement keeps
//! their DACL instead of sealing them as private state. The exclusively
//! created temporary receives the destination's DACL, compared through SDDL,
//! before any byte is written; a first creation is sealed private like the
//! Unix 0600 temporary. Publication uses `std::fs::rename`, which on the
//! pinned toolchain is `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` with a POSIX
//! `FileRenameInfoEx` fallback: the name always denotes a complete version.
//! `ReplaceFileW` is not used because native tests observed readers finding
//! the name absent between its internal renames. A first creation uses
//! `MoveFileExW` without replacement, so concurrent creators cannot clobber.
//!
//! Visibility is atomic at the name. Data is flushed before publication; the
//! directory entry itself is not promised to survive power loss.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use windows_permissions::constants::{SeObjectType, SecurityInformation};
use windows_permissions::{SecurityDescriptor, wrappers};
use winsafe::co;

use super::{Identity, OPEN_REPARSE_POINT, identity, options, refused, seal, user_sid};

/// Sharing/access refusals are retried at most 20 times, 25 ms apart.
const RETRIES: usize = 20;
const WAIT: Duration = Duration::from_millis(25);
/// Read, write and delete sharing: a brief read never blocks the owner's own
/// atomic save, and the identity recheck detects a rename during the read.
const SHARE_ALL: u32 = 7;

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(SHARE_ALL)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

/// Reads a bounded ordinary file without following a link at its name.
pub(crate) fn read(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let mut file = match open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let opened = current(&file)?;
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    // The name must still denote the object whose bytes were read.
    if current(&open(path)?)? != opened {
        return Err(moved());
    }
    if bytes.len() as u64 > limit {
        return Err(refused("configuration exceeds the local file bound"));
    }
    Ok(Some(bytes))
}

/// The identity of an opened ordinary file. A concurrent replacement can hand
/// a reader the previous object already unlinked (zero links, observed in
/// native tests); that is a changed file, not an unsafe one. Once unlinked an
/// object stays so, which makes the distinction stable.
fn current(file: &File) -> io::Result<Identity> {
    identity(file).map_err(|error| {
        let unlinked =
            winapi_util::file::information(file).is_ok_and(|info| info.number_of_links() == 0);
        if unlinked { moved() } else { error }
    })
}

fn moved() -> io::Error {
    refused("configuration changed while it was read; retry the command")
}

/// Publishes `bytes` at `path` only while it still holds `before`. The live
/// file is never truncated, deleted first or copied over.
pub(crate) fn replace(
    path: &Path,
    before: Option<&[u8]>,
    bytes: &[u8],
    limit: u64,
) -> io::Result<()> {
    if bytes.len() as u64 > limit {
        return Err(refused("configuration exceeds the local file bound"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| refused("configuration has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".synveda-{}.tmp", nonce()?));
    let created = write_temporary(&temporary, path, before.is_some(), bytes)?;
    let result = publish(path, &temporary, before, limit);
    if result.is_err() {
        discard(&temporary, created);
    }
    result
}

fn nonce() -> io::Result<String> {
    let mut nonce = [0; 16];
    getrandom::fill(&mut nonce)
        .map_err(|_| io::Error::other("configuration temporary name failed"))?;
    Ok(format!("{:032x}", u128::from_be_bytes(nonce)))
}

/// Protect the empty temporary first, then write and flush the bytes.
fn write_temporary(
    temporary: &Path,
    destination: &Path,
    existing: bool,
    bytes: &[u8],
) -> io::Result<Identity> {
    let user = user_sid()?;
    let mut file = options(true, true).open(temporary)?;
    let created = identity(&file)?;
    let protected = if existing {
        copy_dacl(destination, &mut file)
    } else {
        seal(&mut file, temporary, &user, false)
    };
    let written = protected
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        discard(temporary, created);
        return Err(error);
    }
    Ok(created)
}

/// Apply the destination's DACL unchanged and confirm the OS kept it.
fn copy_dacl(destination: &Path, temporary: &mut File) -> io::Result<()> {
    let source = open(destination)?;
    let descriptor = wrappers::GetSecurityInfo(
        &source,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl,
    )?;
    let dacl = descriptor.dacl().ok_or_else(|| {
        refused(&format!(
            "{} has a null DACL, which grants every account access; it is not reproduced",
            destination.display()
        ))
    })?;
    let expected = dacl_sddl(&descriptor)?;
    let protection = if access(&expected).0 {
        SecurityInformation::ProtectedDacl
    } else {
        SecurityInformation::UnprotectedDacl
    };
    wrappers::SetSecurityInfo(
        temporary,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl | protection,
        None,
        None,
        Some(dacl),
        None,
    )?;
    let applied = wrappers::GetSecurityInfo(
        &*temporary,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl,
    )?;
    if access(&dacl_sddl(&applied)?) != access(&expected) {
        return Err(refused(&format!(
            "{} has an ACL its replacement cannot reproduce (for example stale inherited \
             entries); nothing was written",
            destination.display()
        )));
    }
    Ok(())
}

/// Protection and entries of a DACL's SDDL. The `AI`/`AR` control flags record
/// how inheritance was last computed, not who has access, so they are ignored.
fn access(sddl: &str) -> (bool, &str) {
    let dacl = sddl.strip_prefix("D:").unwrap_or(sddl);
    let (control, entries) = dacl.split_at(dacl.find('(').unwrap_or(dacl.len()));
    (control.contains('P'), entries)
}

fn dacl_sddl(descriptor: &SecurityDescriptor) -> io::Result<String> {
    wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(
        descriptor,
        SecurityInformation::Dacl,
    )?
    .into_string()
    .map_err(|_| refused("Windows security descriptor is not valid Unicode"))
}

fn publish(path: &Path, temporary: &Path, before: Option<&[u8]>, limit: u64) -> io::Result<()> {
    for attempt in 0..=RETRIES {
        if read(path, limit)?.as_deref() != before {
            return Err(changed());
        }
        let result = match before {
            None => create(temporary, path),
            Some(_) => std::fs::rename(temporary, path),
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) && attempt < RETRIES => {
                std::thread::sleep(WAIT);
            }
            // A concurrent creator won; its complete version is retained.
            Err(error) if matches!(error.raw_os_error(), Some(80 | 183)) => return Err(changed()),
            Err(error) => return Err(error),
        }
    }
    Err(refused("configuration replacement retry bound exceeded"))
}

/// First publication never replaces a name that appeared meanwhile.
fn create(temporary: &Path, path: &Path) -> io::Result<()> {
    let (source, target) = (unicode(temporary)?, unicode(path)?);
    winsafe::MoveFileEx(source, Some(target), co::MOVEFILE::WRITE_THROUGH).map_err(|error| {
        io::Error::from_raw_os_error(i32::try_from(error.raw()).unwrap_or(i32::MAX))
    })
}

/// Removes a temporary only while its name still denotes the created object.
fn discard(path: &Path, created: Identity) {
    let ours = open(path)
        .and_then(|file| identity(&file))
        .is_ok_and(|found| found == created);
    if ours {
        let _ = std::fs::remove_file(path);
    }
}

fn changed() -> io::Error {
    refused("configuration changed during the operation; nothing was replaced")
}

fn unicode(path: &Path) -> io::Result<&str> {
    path.to_str()
        .ok_or_else(|| refused("Windows configuration path must be valid Unicode"))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

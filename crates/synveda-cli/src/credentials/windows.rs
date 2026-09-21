//! Native Windows private files (OPS-12 / ADR-0117).
//!
//! Directory handles deny delete sharing through the complete operation. New
//! children inherit a checked private ACL, then receive a protected DACL before
//! any payload is written. Existing ACLs are inspected, never silently repaired.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use windows_permissions::constants::{SeObjectType, SecurityInformation};
use windows_permissions::{LocalBox, SecurityDescriptor, wrappers};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
// Win32 file access/attribute constants; safe std OpenOptions consumes these.
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const READ_ATTRIBUTES: u32 = 0x80;
const LIST_DIRECTORY: u32 = 1;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u64 = 0x400;
const DIRECTORY: u64 = 0x10;
const SHARE_READ_WRITE: u32 = 3;

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn user_sid() -> io::Result<String> {
    windows_permissions::utilities::current_process_sid().map(|sid| sid.to_string())
}

fn acl(file: &File) -> io::Result<String> {
    let information = SecurityInformation::Owner | SecurityInformation::Dacl;
    let descriptor = wrappers::GetSecurityInfo(file, SeObjectType::SE_FILE_OBJECT, information)?;
    let rendered =
        wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(&descriptor, information)?
            .into_string()
            .map_err(|_| refused("Windows security descriptor is not valid Unicode"))?;
    let owner = descriptor
        .owner()
        .ok_or_else(|| refused("Windows private ACL has no owner"))?;
    let dacl = descriptor
        .dacl()
        .ok_or_else(|| refused("Windows private ACL is absent"))?;
    let count = wrappers::GetAclInformationSize(dacl)?.AceCount;
    if count > 128 || rendered.len() > 65_536 {
        return Err(refused("Windows private ACL exceeds its bound"));
    }
    let text = rendered
        .split_once("D:")
        .ok_or_else(|| refused("Windows private ACL is absent"))?
        .1;
    let (control, mut entries) = text.split_at(text.find('(').unwrap_or(text.len()));
    // SDDL may abbreviate a machine/domain account (for example LA). Preserve
    // the actual descriptor's numeric SIDs instead of guessing an alias domain.
    let mut canonical = format!("O:{owner}D:{control}");
    for index in 0..count {
        let (entry, rest) = entries
            .strip_prefix('(')
            .and_then(|s| s.split_once(')'))
            .ok_or_else(|| refused("Windows private ACL entry is unsupported"))?;
        entries = rest;
        let fields: Vec<_> = entry.split(';').collect();
        if fields.len() != 6 || fields[0] != "A" || !fields[3].is_empty() || !fields[4].is_empty() {
            return Err(refused("Windows private ACL entry is unsupported"));
        }
        let sid = wrappers::GetAce(dacl, index)?
            .sid()
            .ok_or_else(|| refused("Windows private ACL entry has no identity"))?;
        canonical.push_str(&format!("(A;{};{};;;{sid})", fields[1], fields[2]));
    }
    if !entries.is_empty() {
        return Err(refused("Windows private ACL entry count changed"));
    }
    Ok(canonical)
}

fn validate_acl(file: &File, user: &str, directory: bool) -> io::Result<()> {
    super::windows_acl::validate(&acl(file)?, user, directory).map_err(refused)
}

fn seal(file: &mut File, user: &str, directory: bool) -> io::Result<()> {
    let flags = if directory { "OICI" } else { "" };
    let descriptor: LocalBox<SecurityDescriptor> =
        format!("O:{user}D:P(A;{flags};FA;;;{user})(A;{flags};FA;;;SY)(A;{flags};FA;;;BA)")
            .parse()?;
    wrappers::SetSecurityInfo(
        file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
        descriptor.owner(),
        None,
        descriptor.dacl(),
        None,
    )?;
    validate_acl(file, user, directory)
}

fn open_directory(path: &Path, new: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        // Metadata-only handles do not engage Windows delete-sharing checks.
        .access_mode(
            LIST_DIRECTORY
                | READ_CONTROL
                | READ_ATTRIBUTES
                | if new { WRITE_DAC | WRITE_OWNER } else { 0 },
        )
        .share_mode(SHARE_READ_WRITE)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)?;
    let info = winapi_util::file::information(&file)?;
    if !winapi_util::file::typ(&file)?.is_disk()
        || info.file_attributes() & (DIRECTORY | REPARSE_POINT) != DIRECTORY
    {
        return Err(refused(
            "Windows private path must contain only ordinary disk directories",
        ));
    }
    Ok(file)
}

fn options(write: bool, create: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .create_new(create)
        .share_mode(SHARE_READ_WRITE)
        .custom_flags(OPEN_REPARSE_POINT);
    if create {
        // GENERIC_READ | GENERIC_WRITE, plus permission to seal the empty file.
        options.access_mode(0xc000_0000 | WRITE_DAC | WRITE_OWNER);
    }
    options
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Identity(u64, u64);

fn identity(file: &File) -> io::Result<Identity> {
    let info = winapi_util::file::information(file)?;
    if !winapi_util::file::typ(file)?.is_disk()
        || info.file_attributes() & (DIRECTORY | REPARSE_POINT) != 0
        || info.number_of_links() != 1
    {
        return Err(refused(
            "Windows private file must be bounded, ordinary and singly linked",
        ));
    }
    Ok(Identity(info.volume_serial_number(), info.file_index()))
}

pub(crate) struct Directory {
    path: PathBuf,
    user: String,
    limit: u64,
    // These handles prevent ancestors from being renamed or replaced by a junction.
    _ancestors: Vec<File>,
}

impl Directory {
    pub(crate) fn open(path: &Path, create: bool) -> io::Result<Option<Self>> {
        Self::bounded(path, create, MAX_BYTES)
    }

    pub(crate) fn bounded(path: &Path, create: bool, limit: u64) -> io::Result<Option<Self>> {
        if limit > 16 * 1024 * 1024 {
            return Err(refused("Windows private file limit exceeds its bound"));
        }
        let path_text = path
            .to_str()
            .ok_or_else(|| refused("Windows private path must be Unicode"))?;
        let normalized = path_text.replace('\\', "/");
        let bytes = normalized.as_bytes();
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":/" {
            return Err(refused(
                "Windows private path must be a fully qualified local drive path",
            ));
        }
        let components: Vec<_> = normalized[3..].trim_end_matches('/').split('/').collect();
        for part in &components {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            if part.is_empty()
                || matches!(*part, "." | "..")
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"|?*".contains(c))
                || matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                )
                || ["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|n| {
                        matches!(
                            n,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                })
            {
                return Err(refused(
                    "Windows private path contains an ambiguous or device component",
                ));
            }
        }
        let root = normalized[..3].replace('/', "\\");
        if winsafe::GetDriveType(Some(&root)) != winsafe::co::DRIVE::FIXED {
            return Err(refused(
                "Windows private state requires a local fixed drive",
            ));
        }
        let user = user_sid()?;
        let mut current = PathBuf::from(root);
        let root_file = open_directory(&current, false)?;
        super::windows_acl::validate_ancestor(&acl(&root_file)?, &user).map_err(refused)?;
        let mut ancestors = vec![root_file];
        for component in components {
            current.push(component);
            let file = match open_directory(&current, false) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    // No private child may inherit public permissions, even
                    // briefly. Existing roots are never chmod/ACL-repaired.
                    let parent = ancestors
                        .last()
                        .ok_or_else(|| refused("private parent is absent"))?;
                    validate_acl(parent, &user, true)?;
                    if !create {
                        return Ok(None);
                    }
                    match std::fs::create_dir(&current) {
                        Ok(()) => {
                            let mut file = open_directory(&current, true)?;
                            seal(&mut file, &user, true)?;
                            file
                        }
                        // Another updated CLI may have created this private
                        // component while both commands were preparing a lock.
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            open_directory(&current, false)?
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            };
            // Holding a directory against deletion does not stop someone with
            // WRITE_DAC from propagating a broader ACL into unprotected children.
            super::windows_acl::validate_ancestor(&acl(&file)?, &user).map_err(refused)?;
            ancestors.push(file);
        }
        validate_acl(
            ancestors
                .last()
                .ok_or_else(|| refused("private directory is absent"))?,
            &user,
            true,
        )?;
        Ok(Some(Self {
            path: current,
            user,
            limit,
            _ancestors: ancestors,
        }))
    }

    pub(crate) fn validate(&self, file: &File, name: &str) -> io::Result<()> {
        validate_name(name)?;
        let opened = identity(file)?;
        if file.metadata()?.len() > self.limit {
            return Err(refused("Windows private file exceeds its bound"));
        }
        validate_acl(file, &self.user, false)?;
        let named = options(false, false).open(self.path.join(name))?;
        if opened != identity(&named)? {
            return Err(refused("Windows private file identity changed"));
        }
        Ok(())
    }

    pub(crate) fn read(&self, name: &str) -> io::Result<Option<(Identity, Vec<u8>)>> {
        validate_name(name)?;
        let mut file = match options(false, false).open(self.path.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        self.validate(&file, name)?;
        let before = identity(&file)?;
        let mut bytes = Vec::new();
        (&mut file).take(self.limit + 1).read_to_end(&mut bytes)?;
        self.validate(&file, name)?;
        if bytes.len() as u64 > self.limit || identity(&file)? != before {
            return Err(refused(
                "Windows private file changed or exceeded its bound",
            ));
        }
        Ok(Some((before, bytes)))
    }

    pub(crate) fn lock_file(&self) -> io::Result<File> {
        self.named_lock("credentials.lock")
    }

    pub(crate) fn named_lock(&self, name: &str) -> io::Result<File> {
        validate_name(name)?;
        let path = self.path.join(name);
        let file = match options(true, true).open(&path) {
            Ok(mut file) => {
                seal(&mut file, &self.user, false)?;
                file
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                options(true, false).open(&path)?
            }
            Err(error) => return Err(error),
        };
        self.validate(&file, name)?;
        if file.metadata()?.len() != 0 {
            return Err(refused("private lock must be an empty regular file"));
        }
        Ok(file)
    }

    pub(crate) fn replace(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let before = self.read(name)?;
        self.replace_snapshot(name, bytes, before)
    }

    pub(crate) fn replace_if(
        &self,
        name: &str,
        bytes: &[u8],
        expected: Option<&str>,
    ) -> io::Result<()> {
        let before = self.read(name)?;
        check_digest(&before, expected)?;
        self.replace_snapshot(name, bytes, before)
    }

    fn replace_snapshot(
        &self,
        name: &str,
        bytes: &[u8],
        before: Option<(Identity, Vec<u8>)>,
    ) -> io::Result<()> {
        if bytes.len() as u64 > self.limit {
            return Err(refused("private bytes exceed the local file bound"));
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| io::Error::other("credential temporary name failed"))?;
        let temporary = format!(".synveda-{:032x}.tmp", u128::from_be_bytes(nonce));
        let temporary_path = self.path.join(&temporary);
        let mut file = options(true, true).open(&temporary_path)?;
        let created = identity(&file)?;
        let result = (|| {
            seal(&mut file, &self.user, false)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            self.validate(&file, &temporary)?;
            Ok(())
        })();
        drop(file);
        let result = result.and_then(|()| {
            // Readers deny delete sharing while validating a snapshot. A brief
            // read must not discard a newly rotated token. Retry only Windows
            // sharing/access refusals (MoveFileEx can report either), with at
            // most 500 ms total retry delay. Never delete/truncate the target.
            for attempt in 0..=20 {
                if self.read(name)? != before {
                    return Err(refused(
                        "private file changed during replacement; nothing was replaced",
                    ));
                }
                match std::fs::rename(&temporary_path, self.path.join(name)) {
                    Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) && attempt < 20 => {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    result => return result,
                }
            }
            Err(refused("credential replacement retry bound exceeded"))
        });
        if result.is_err() {
            // Never remove a replacement at the temporary name. No token is
            // included in diagnostics; failed cleanup keeps private evidence.
            let unchanged = options(false, false)
                .open(&temporary_path)
                .and_then(|file| identity(&file))
                .is_ok_and(|id| id == created);
            if unchanged {
                let _ = std::fs::remove_file(&temporary_path);
            }
        }
        result
    }

    /// The caller holds the stable mutation lock. Never retire a newer snapshot.
    pub(crate) fn remove_if(&self, name: &str, expected: Option<&str>) -> io::Result<()> {
        let before = self.read(name)?;
        check_digest(&before, expected)?;
        if before.is_none() {
            return Ok(());
        }
        for attempt in 0..=20 {
            if self.read(name)? != before {
                return Err(refused("private file changed; nothing was removed"));
            }
            match std::fs::remove_file(self.path.join(name)) {
                Err(error) if matches!(error.raw_os_error(), Some(5 | 32)) && attempt < 20 => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                result => return result,
            }
        }
        Err(refused("private removal retry bound exceeded"))
    }

    pub(crate) fn names(&self) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(&self.path)? {
            if names.len() == 4096 {
                return Err(refused("private directory exceeds its entry bound"));
            }
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| refused("private filename is not Unicode"))?;
            validate_name(&name)?;
            names.push(name);
        }
        names.sort();
        Ok(names)
    }
}

fn check_digest(before: &Option<(Identity, Vec<u8>)>, expected: Option<&str>) -> io::Result<()> {
    if before
        .as_ref()
        .map(|(_, bytes)| crate::local_state::digest(bytes))
        .as_deref()
        != expected
    {
        return Err(refused("private file changed; stale operation refused"));
    }
    Ok(())
}

/// Every operation is relative to an already admitted directory, never a path.
fn validate_name(name: &str) -> io::Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 200
        || matches!(name, "." | "..")
        || name.ends_with(['.', ' '])
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        })
    {
        return Err(refused("private filename is not an ordinary leaf name"));
    }
    Ok(())
}

pub(super) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let parent = path
        .parent()
        .ok_or_else(|| refused("credentials have no parent"))?;
    let Some(directory) = Directory::open(parent, false)? else {
        return Ok(None);
    };
    Ok(directory.read("credentials.json")?.map(|(_, bytes)| bytes))
}

pub(super) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| refused("credentials have no parent"))?;
    let directory =
        Directory::open(parent, true)?.ok_or_else(|| refused("credential directory is absent"))?;
    directory.replace("credentials.json", bytes)
}

#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;

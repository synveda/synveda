//! Native Windows private credential files (OPS-12 / ADR-0117).
//!
//! Directory handles deny delete sharing through the complete operation. New
//! children inherit a checked private ACL, then receive a protected DACL before
//! any payload is written. Existing ACLs are inspected, never silently repaired.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use windows_permissions::constants::{SeObjectType, SecurityInformation};
use windows_permissions::{LocalBox, SecurityDescriptor, WindowsSecure, wrappers};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
// Win32 file access/attribute constants; safe std OpenOptions consumes these.
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const READ_ATTRIBUTES: u32 = 0x80;
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
    let descriptor = file.security_descriptor(information)?;
    wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(&descriptor, information)?
        .into_string()
        .map_err(|_| refused("Windows security descriptor is not valid Unicode"))
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
        .access_mode(READ_CONTROL | READ_ATTRIBUTES | if new { WRITE_DAC | WRITE_OWNER } else { 0 })
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
struct Identity(u64, u64);

fn identity(file: &File) -> io::Result<Identity> {
    let info = winapi_util::file::information(file)?;
    if !winapi_util::file::typ(file)?.is_disk()
        || info.file_attributes() & (DIRECTORY | REPARSE_POINT) != 0
        || info.number_of_links() != 1
        || info.file_size() > MAX_BYTES
    {
        return Err(refused(
            "Windows private file must be bounded, ordinary and singly linked",
        ));
    }
    Ok(Identity(info.volume_serial_number(), info.file_index()))
}

pub(super) struct Directory {
    path: PathBuf,
    user: String,
    // These handles prevent ancestors from being renamed or replaced by a junction.
    _ancestors: Vec<File>,
}

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Option<Self>> {
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
        let mut ancestors = vec![open_directory(&current, false)?];
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
            _ancestors: ancestors,
        }))
    }

    pub(super) fn validate(&self, file: &File, name: &str) -> io::Result<()> {
        let opened = identity(file)?;
        validate_acl(file, &self.user, false)?;
        let named = options(false, false).open(self.path.join(name))?;
        if opened != identity(&named)? {
            return Err(refused("Windows private file identity changed"));
        }
        Ok(())
    }

    fn read(&self, name: &str) -> io::Result<Option<(Identity, Vec<u8>)>> {
        let mut file = match options(false, false).open(self.path.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        self.validate(&file, name)?;
        let before = identity(&file)?;
        let mut bytes = Vec::new();
        (&mut file).take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        self.validate(&file, name)?;
        if bytes.len() as u64 > MAX_BYTES || identity(&file)? != before {
            return Err(refused(
                "Windows private file changed or exceeded its bound",
            ));
        }
        Ok(Some((before, bytes)))
    }

    pub(super) fn lock_file(&self) -> io::Result<File> {
        let name = "credentials.lock";
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
            return Err(refused("credential lock must be an empty regular file"));
        }
        Ok(file)
    }

    fn replace(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err(refused("credentials exceed the local file bound"));
        }
        let before = self.read(name)?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce)
            .map_err(|_| io::Error::other("credential temporary name failed"))?;
        let temporary = format!(".credentials-{:032x}.tmp", u128::from_be_bytes(nonce));
        let temporary_path = self.path.join(&temporary);
        let mut file = options(true, true).open(&temporary_path)?;
        let created = identity(&file)?;
        let result = (|| {
            seal(&mut file, &self.user, false)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            self.validate(&file, &temporary)?;
            if self.read(name)? != before {
                return Err(refused(
                    "credentials changed during replacement; nothing was replaced",
                ));
            }
            Ok(())
        })();
        drop(file);
        let result = result.and_then(|()| std::fs::rename(&temporary_path, self.path.join(name)));
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

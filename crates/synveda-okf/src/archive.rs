use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use synveda_types::{Error, Result};

use crate::format::{BundleInput, InputEntry, InputEntryKind};
use crate::{MAX_ARCHIVE_BYTES, MAX_ARTIFACT_BYTES, MAX_ARTIFACTS, MAX_EXPANDED_BYTES};

pub(crate) fn entries(input: BundleInput) -> Result<Vec<InputEntry>> {
    match input {
        BundleInput::Entries(entries) => Ok(entries),
        BundleInput::Zip(bytes) => zip_entries(&bytes),
        BundleInput::Tar(bytes) => tar_entries(Cursor::new(bounded_archive(bytes)?)),
        BundleInput::TarGzip(bytes) => {
            let bytes = bounded_archive(bytes)?;
            tar_entries(GzDecoder::new(Cursor::new(bytes)))
        }
        BundleInput::Directory(root) => directory_entries(&root),
    }
}

fn bounded_archive(bytes: Vec<u8>) -> Result<Vec<u8>> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(Error::Invalid {
            message: format!("OKF archive exceeds {MAX_ARCHIVE_BYTES} bytes"),
        });
    }
    Ok(bytes)
}

fn zip_entries(bytes: &[u8]) -> Result<Vec<InputEntry>> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(Error::Invalid {
            message: format!("OKF zip exceeds {MAX_ARCHIVE_BYTES} bytes"),
        });
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| Error::Invalid {
        message: "OKF zip is malformed or encrypted".to_owned(),
    })?;
    if archive.len() > MAX_ARTIFACTS {
        return Err(Error::Invalid {
            message: format!("OKF zip exceeds {MAX_ARTIFACTS} entries"),
        });
    }
    let mut output = Vec::new();
    let mut total = 0usize;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|_| Error::Invalid {
            message: "OKF zip contains an unreadable or encrypted entry".to_owned(),
        })?;
        let logical_path = normalise_path(file.name())?;
        if file.is_dir() {
            output.push(InputEntry {
                logical_path,
                kind: InputEntryKind::Directory,
                bytes: Vec::new(),
            });
            continue;
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
        {
            return Err(Error::Invalid {
                message: format!("OKF zip contains a link or special entry: {logical_path}"),
            });
        }
        let size = usize::try_from(file.size()).map_err(|_| Error::Invalid {
            message: format!("OKF zip entry size overflows: {logical_path}"),
        })?;
        if size > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF zip entry exceeds byte limit: {logical_path}"),
            });
        }
        let compressed = file.compressed_size().max(1);
        if file.size() > compressed.saturating_mul(100) + 1024 {
            return Err(Error::Invalid {
                message: format!("OKF zip entry exceeds expansion ratio: {logical_path}"),
            });
        }
        total = total.checked_add(size).ok_or_else(|| Error::Invalid {
            message: "OKF zip expanded byte total overflowed".to_owned(),
        })?;
        if total > MAX_EXPANDED_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF zip exceeds {MAX_EXPANDED_BYTES} expanded bytes"),
            });
        }
        let mut content = Vec::with_capacity(size);
        file.take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut content)
            .map_err(|_| Error::Invalid {
                message: format!("OKF zip entry cannot be read: {logical_path}"),
            })?;
        if content.len() != size || content.len() > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF zip entry size is inconsistent: {logical_path}"),
            });
        }
        output.push(InputEntry {
            logical_path,
            kind: InputEntryKind::File,
            bytes: content,
        });
    }
    Ok(output)
}

fn tar_entries(reader: impl Read) -> Result<Vec<InputEntry>> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|_| Error::Invalid {
        message: "OKF tar is malformed or compressed with an unsupported format".to_owned(),
    })?;
    let mut output = Vec::new();
    let mut total = 0usize;
    for entry in entries {
        if output.len() >= MAX_ARTIFACTS {
            return Err(Error::Invalid {
                message: format!("OKF tar exceeds {MAX_ARTIFACTS} entries"),
            });
        }
        let entry = entry.map_err(|_| Error::Invalid {
            message: "OKF tar contains an unreadable entry".to_owned(),
        })?;
        let path = entry.path().map_err(|_| Error::Invalid {
            message: "OKF tar contains an invalid path".to_owned(),
        })?;
        let logical_path = path.to_str().ok_or_else(|| Error::Invalid {
            message: "OKF tar path is not UTF-8".to_owned(),
        })?;
        let logical_path = normalise_path(logical_path)?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            output.push(InputEntry {
                logical_path,
                kind: InputEntryKind::Directory,
                bytes: Vec::new(),
            });
            continue;
        }
        if !kind.is_file() {
            return Err(Error::Invalid {
                message: format!("OKF tar contains a link or special entry: {logical_path}"),
            });
        }
        let size = usize::try_from(entry.size()).map_err(|_| Error::Invalid {
            message: format!("OKF tar entry size overflows: {logical_path}"),
        })?;
        if size > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF tar entry exceeds byte limit: {logical_path}"),
            });
        }
        total = total.checked_add(size).ok_or_else(|| Error::Invalid {
            message: "OKF tar expanded byte total overflowed".to_owned(),
        })?;
        if total > MAX_EXPANDED_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF tar exceeds {MAX_EXPANDED_BYTES} expanded bytes"),
            });
        }
        let mut content = Vec::with_capacity(size);
        entry
            .take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut content)
            .map_err(|_| Error::Invalid {
                message: format!("OKF tar entry cannot be read: {logical_path}"),
            })?;
        if content.len() != size || content.len() > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF tar entry size is inconsistent: {logical_path}"),
            });
        }
        output.push(InputEntry {
            logical_path,
            kind: InputEntryKind::File,
            bytes: content,
        });
    }
    Ok(output)
}

pub(crate) fn directory_entries(root: &Path) -> Result<Vec<InputEntry>> {
    if fs::symlink_metadata(root)
        .map_err(|_| Error::Invalid {
            message: "OKF directory cannot be opened".to_owned(),
        })?
        .file_type()
        .is_symlink()
    {
        return Err(Error::Invalid {
            message: "OKF directory source cannot be a symlink".to_owned(),
        });
    }
    let root = fs::canonicalize(root).map_err(|_| Error::Invalid {
        message: "OKF directory cannot be opened".to_owned(),
    })?;
    if !root.is_dir() {
        return Err(Error::Invalid {
            message: "OKF directory source is not a directory".to_owned(),
        });
    }
    let mut output = Vec::new();
    let mut pending = vec![root.clone()];
    let mut entry_count = 0usize;
    let mut total_bytes = 0usize;
    while let Some(path) = pending.pop() {
        if path == root {
            enqueue_children(&path, &mut pending, &mut entry_count)?;
            continue;
        }
        let relative = path.strip_prefix(&root).map_err(|_| Error::Invalid {
            message: "OKF directory entry escapes its root".to_owned(),
        })?;
        let logical_path = relative.to_str().ok_or_else(|| Error::Invalid {
            message: "OKF directory path is not UTF-8".to_owned(),
        })?;
        let logical_path = normalise_path(logical_path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|_| Error::Invalid {
            message: "OKF directory entry metadata cannot be read".to_owned(),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(Error::Invalid {
                message: format!("OKF directory contains a symlink: {logical_path}"),
            });
        }
        if metadata.is_dir() {
            enqueue_children(&path, &mut pending, &mut entry_count)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(Error::Invalid {
                message: format!("OKF directory contains a special entry: {logical_path}"),
            });
        }
        let canonical = fs::canonicalize(&path).map_err(|_| Error::Invalid {
            message: format!("OKF directory entry cannot be resolved: {logical_path}"),
        })?;
        if !canonical.starts_with(&root) {
            return Err(Error::Invalid {
                message: format!("OKF directory entry escapes its root: {logical_path}"),
            });
        }
        if metadata.len() > MAX_ARTIFACT_BYTES as u64 {
            return Err(Error::Invalid {
                message: format!("OKF directory entry exceeds byte limit: {logical_path}"),
            });
        }
        let mut bytes = Vec::new();
        fs::File::open(canonical)
            .map_err(|_| Error::Invalid {
                message: format!("OKF directory entry cannot be read: {logical_path}"),
            })?
            .take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Invalid {
                message: format!("OKF directory entry cannot be read: {logical_path}"),
            })?;
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF directory entry exceeds byte limit: {logical_path}"),
            });
        }
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| Error::Invalid {
                message: "OKF directory expanded byte total overflowed".to_owned(),
            })?;
        if total_bytes > MAX_EXPANDED_BYTES {
            return Err(Error::Invalid {
                message: format!("OKF directory exceeds {MAX_EXPANDED_BYTES} expanded bytes"),
            });
        }
        output.push(InputEntry {
            logical_path,
            kind: InputEntryKind::File,
            bytes,
        });
    }
    Ok(output)
}

fn enqueue_children(
    current: &Path,
    pending: &mut Vec<std::path::PathBuf>,
    entry_count: &mut usize,
) -> Result<()> {
    let mut children = Vec::new();
    for child in fs::read_dir(current).map_err(|_| Error::Invalid {
        message: "OKF directory cannot be enumerated".to_owned(),
    })? {
        let child = child.map_err(|_| Error::Invalid {
            message: "OKF directory contains an unreadable entry".to_owned(),
        })?;
        if child.file_name() == ".git" {
            continue;
        }
        *entry_count = entry_count.checked_add(1).ok_or_else(|| Error::Invalid {
            message: "OKF directory entry count overflowed".to_owned(),
        })?;
        if *entry_count > MAX_ARTIFACTS {
            return Err(Error::Invalid {
                message: format!("OKF directory exceeds {MAX_ARTIFACTS} entries"),
            });
        }
        children.push(child.path());
    }
    children.sort();
    for child in children.into_iter().rev() {
        pending.push(child);
    }
    Ok(())
}

pub(crate) fn normalise_path(value: &str) -> Result<String> {
    if value.is_empty()
        || value.contains(['\\', '\0'])
        || value.starts_with('/')
        || value.chars().count() > 1_000
    {
        return Err(Error::Invalid {
            message: "OKF logical paths must be non-empty relative UTF-8 slash paths".to_owned(),
        });
    }
    // Path::components removes interior `.` segments before we can reject them.
    if value.split('/').any(|part| part == "." || part == "..") {
        return Err(Error::Invalid {
            message: "OKF logical paths must not contain dot segments".to_owned(),
        });
    }
    let path = Path::new(value);
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_str().ok_or_else(|| Error::Invalid {
                    message: "OKF logical path is not UTF-8".to_owned(),
                })?;
                if segment.is_empty() || segment == "." || segment == ".." {
                    return Err(Error::Invalid {
                        message: "OKF logical paths must not contain dot segments".to_owned(),
                    });
                }
                parts.push(segment);
            }
            _ => {
                return Err(Error::Invalid {
                    message: "OKF logical paths must not be absolute or traverse parents"
                        .to_owned(),
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(Error::Invalid {
            message: "OKF logical path has no file name".to_owned(),
        });
    }
    Ok(parts.join("/"))
}

pub(crate) fn resolve_relative(parent: Option<&str>, target: &str) -> Result<String> {
    if target.contains(['\\', '\0']) || target.starts_with('/') {
        return Err(Error::Invalid {
            message: "OKF relation target is not a relative path".to_owned(),
        });
    }
    let mut parts: Vec<&str> = parent
        .map(|parent| parent.split('/').filter(|part| !part.is_empty()).collect())
        .unwrap_or_default();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(Error::Invalid {
                        message: "OKF relation target escapes the bundle root".to_owned(),
                    });
                }
            }
            segment => parts.push(segment),
        }
    }
    normalise_path(&parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_refuse_every_escape_and_resolve_bounded_parent_links() {
        for invalid in [
            "",
            "/root.md",
            "../escape.md",
            "a/../b.md",
            "a/./b.md",
            "a\\b.md",
        ] {
            assert!(normalise_path(invalid).is_err(), "{invalid}");
        }
        assert_eq!(normalise_path("a/b.md").unwrap(), "a/b.md");
        assert_eq!(resolve_relative(Some("a/b"), "../c.md").unwrap(), "a/c.md");
        assert!(resolve_relative(None, "../c.md").is_err());
    }

    #[test]
    fn local_directory_counts_empty_directories_before_allocating_all_entries() {
        let root = tempfile::tempdir().expect("temporary OKF directory");
        for index in 0..=MAX_ARTIFACTS {
            fs::create_dir(root.path().join(format!("empty-{index:04}")))
                .expect("create empty directory");
        }
        let error = directory_entries(root.path()).expect_err("empty directories count");
        assert!(error.to_string().contains("entries"));
    }

    #[test]
    fn local_directory_bounds_total_bytes_before_returning_entries() {
        let root = tempfile::tempdir().expect("temporary OKF directory");
        let content = vec![b'x'; MAX_ARTIFACT_BYTES];
        for index in 0..16 {
            fs::write(root.path().join(format!("entry-{index:02}.md")), &content)
                .expect("write bounded file");
        }
        let error = directory_entries(root.path()).expect_err("total must be bounded");
        assert!(error.to_string().contains("expanded bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn local_directory_refuses_symlink_root() {
        let root = tempfile::tempdir().expect("temporary OKF directory");
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(root.path(), &alias).expect("create symlink");
        assert!(directory_entries(&alias).is_err());
    }
}

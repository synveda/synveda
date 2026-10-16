//! Inbound Git object reads. Transport carries bytes, never product authority.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use synveda_types::source::{validate_source_paths, validate_source_ref};
use synveda_types::{Error, Result, ScopeId, TenantId};

use crate::LocalTransport;
use crate::transport::{command, private_directory};

/// Maximum inert Markdown bytes per configured source path.
pub const MAX_SOURCE_BLOB_BYTES: usize = 131_072;
/// Maximum cumulative object bytes admitted by one complete source read.
pub const MAX_SOURCE_BYTES: usize = 2_097_152;
/// Maximum Git object reads, including cached parent trees and blobs.
pub const MAX_SOURCE_REQUESTS: usize = 128;
const MAX_TREE_ENTRIES: usize = 2_000;
const SOURCE_DEADLINE: Duration = Duration::from_secs(30);

/// Exact immutable commit/tree pair pinned once for a durable refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePin {
    /// Full lower-case SHA-1 commit id.
    pub commit: String,
    /// Full lower-case SHA-1 root-tree id.
    pub tree: String,
}

impl SourcePin {
    /// Refuse abbreviated, substituted or malformed object identities.
    pub fn validate(&self) -> Result<()> {
        if !oid(&self.commit) || !oid(&self.tree) {
            return Err(incomplete());
        }
        Ok(())
    }
}

/// An exact configured path, including a positively established absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Validated literal relative Markdown path.
    pub path: String,
    /// Actual Git blob id; absent only after complete parent-tree inspection.
    pub blob_oid: Option<String>,
    /// Inert original bytes; never executed or written by transport.
    pub bytes: Option<Vec<u8>>,
    /// A known suppressed blob was deliberately not read; distinct from absence.
    pub omitted: bool,
}

/// A complete bounded snapshot. An error never returns partial deletion proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    /// Durable immutable pin, independent of later branch movement.
    pub pin: SourcePin,
    /// Every configured path in canonical order, present or proven absent.
    pub files: Vec<SourceFile>,
}

/// Private deployment custody for inbound local bare repositories.
#[derive(Clone)]
pub struct LocalSourceReader {
    custody: LocalTransport,
}

impl LocalSourceReader {
    /// Admit an existing private owned root. No repository is created here.
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            custody: LocalTransport::new(root)?,
        })
    }

    /// Derive the operator-owned location without admitting caller paths.
    pub fn repository(&self, tenant: TenantId, scope: ScopeId, target: &str) -> Result<PathBuf> {
        self.custody.repository(tenant, scope, target)
    }

    fn admitted_repository(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
    ) -> Result<PathBuf> {
        let repository = self.repository(tenant, scope, target)?;
        let mut current = self.custody.repository(tenant, scope, target)?;
        for _ in 0..3 {
            private_directory(&current).map_err(|_| incomplete())?;
            current = current.parent().ok_or_else(incomplete)?.to_owned();
        }
        Ok(repository)
    }

    /// Resolve one branch to a verified immutable commit/tree before file reads.
    pub async fn resolve(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
        reference: &str,
    ) -> Result<SourcePin> {
        validate_source_ref(reference)?;
        let repository = self.admitted_repository(tenant, scope, target)?;
        tokio::time::timeout(SOURCE_DEADLINE, async {
            if command(
                &repository,
                &["rev-parse", "--is-bare-repository"],
                &[],
                128,
            )
            .await
            .map_err(|_| incomplete())?
                != b"true\n"
                || command(
                    &repository,
                    &["rev-parse", "--show-object-format"],
                    &[],
                    128,
                )
                .await
                .map_err(|_| incomplete())?
                    != b"sha1\n"
            {
                return Err(incomplete());
            }
            let resolved = command(
                &repository,
                &["show-ref", "--verify", "--hash", reference],
                &[],
                128,
            )
            .await
            .map_err(|_| incomplete())?;
            let commit = std::str::from_utf8(&resolved)
                .map_err(|_| incomplete())?
                .trim_end_matches('\n')
                .to_owned();
            if !oid(&commit) {
                return Err(incomplete());
            }
            let bytes = command(&repository, &["cat-file", "commit", &commit], &[], 65_536)
                .await
                .map_err(|_| incomplete())?;
            verify_object("commit", &commit, &bytes)?;
            let first = bytes.split(|b| *b == b'\n').next().ok_or_else(incomplete)?;
            let tree = std::str::from_utf8(first)
                .map_err(|_| incomplete())?
                .strip_prefix("tree ")
                .ok_or_else(incomplete)?
                .to_owned();
            let pin = SourcePin { commit, tree };
            pin.validate()?;
            Ok(pin)
        })
        .await
        .map_err(|_| incomplete())?
    }

    /// Read only the durable pin. Missing objects, links or bounds yield no snapshot.
    pub async fn read_at(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
        pin: &SourcePin,
        paths: &[String],
    ) -> Result<SourceSnapshot> {
        self.read_at_omitting(tenant, scope, target, pin, paths, &BTreeMap::new())
            .await
    }

    /// Account for known suppressed immutable blobs without fetching their bytes.
    /// Only an exact previously admitted blob id may be omitted.
    pub async fn read_at_omitting(
        &self,
        tenant: TenantId,
        scope: ScopeId,
        target: &str,
        pin: &SourcePin,
        paths: &[String],
        omitted: &BTreeMap<String, String>,
    ) -> Result<SourceSnapshot> {
        pin.validate()?;
        validate_source_paths(paths)?;
        if omitted
            .iter()
            .any(|(path, object)| !paths.contains(path) || !oid(object))
        {
            return Err(incomplete());
        }
        let repository = self.admitted_repository(tenant, scope, target)?;
        tokio::time::timeout(SOURCE_DEADLINE, async {
            let mut trees = BTreeMap::<String, Vec<TreeEntry>>::new();
            let mut budget = ReadBudget::default();
            budget.request()?;
            let commit_bytes = command(
                &repository,
                &["cat-file", "commit", &pin.commit],
                &[],
                65_536,
            )
            .await
            .map_err(|_| incomplete())?;
            budget.bytes(commit_bytes.len())?;
            verify_object("commit", &pin.commit, &commit_bytes)?;
            let tree_header = format!("tree {}", pin.tree);
            if commit_bytes.split(|b| *b == b'\n').next() != Some(tree_header.as_bytes()) {
                return Err(incomplete());
            }
            let mut files = Vec::with_capacity(paths.len());
            for path in paths {
                let mut tree = pin.tree.clone();
                let parts: Vec<_> = path.split('/').collect();
                let mut found = None;
                for (index, part) in parts.iter().enumerate() {
                    if !trees.contains_key(&tree) {
                        budget.request()?;
                        let bytes = command(
                            &repository,
                            &["cat-file", "tree", &tree],
                            &[],
                            MAX_SOURCE_BLOB_BYTES,
                        )
                        .await
                        .map_err(|_| incomplete())?;
                        budget.bytes(bytes.len())?;
                        verify_object("tree", &tree, &bytes)?;
                        trees.insert(tree.clone(), parse_tree(&bytes)?);
                    }
                    let Some(entry) = trees
                        .get(&tree)
                        .and_then(|entries| {
                            entries.iter().find(|entry| entry.name == part.as_bytes())
                        })
                        .cloned()
                    else {
                        break;
                    };
                    if index + 1 < parts.len() {
                        if entry.mode != "40000" {
                            return Err(incomplete());
                        }
                        tree = entry.oid;
                    } else {
                        if !matches!(entry.mode.as_str(), "100644" | "100755") {
                            return Err(incomplete());
                        }
                        if omitted.get(path) == Some(&entry.oid) {
                            found = Some((entry.oid, None));
                            break;
                        }
                        budget.request()?;
                        let bytes = command(
                            &repository,
                            &["cat-file", "blob", &entry.oid],
                            &[],
                            MAX_SOURCE_BLOB_BYTES,
                        )
                        .await
                        .map_err(|_| incomplete())?;
                        budget.bytes(bytes.len())?;
                        verify_object("blob", &entry.oid, &bytes)?;
                        found = Some((entry.oid, Some(bytes)));
                    }
                }
                files.push(match found {
                    Some((object, bytes)) => SourceFile {
                        path: path.clone(),
                        blob_oid: Some(object),
                        omitted: bytes.is_none(),
                        bytes,
                    },
                    None => SourceFile {
                        path: path.clone(),
                        blob_oid: None,
                        bytes: None,
                        omitted: false,
                    },
                });
            }
            Ok(SourceSnapshot {
                pin: pin.clone(),
                files,
            })
        })
        .await
        .map_err(|_| incomplete())?
    }
}

#[derive(Clone)]
struct TreeEntry {
    mode: String,
    name: Vec<u8>,
    oid: String,
}

fn parse_tree(mut bytes: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    while !bytes.is_empty() {
        if entries.len() == MAX_TREE_ENTRIES {
            return Err(incomplete());
        }
        let space = bytes
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(incomplete)?;
        let nul = bytes.iter().position(|b| *b == 0).ok_or_else(incomplete)?;
        if space >= nul || nul + 21 > bytes.len() {
            return Err(incomplete());
        }
        let mode = std::str::from_utf8(&bytes[..space])
            .map_err(|_| incomplete())?
            .to_owned();
        let name = bytes[space + 1..nul].to_vec();
        if name.is_empty()
            || name.contains(&b'/')
            || entries.iter().any(|e: &TreeEntry| e.name == name)
        {
            return Err(incomplete());
        }
        let object = bytes[nul + 1..nul + 21]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        entries.push(TreeEntry {
            mode,
            name,
            oid: object,
        });
        bytes = &bytes[nul + 21..];
    }
    Ok(entries)
}

#[derive(Default)]
struct ReadBudget {
    requests: usize,
    bytes: usize,
}
impl ReadBudget {
    fn request(&mut self) -> Result<()> {
        self.requests += 1;
        if self.requests > MAX_SOURCE_REQUESTS {
            Err(incomplete())
        } else {
            Ok(())
        }
    }
    fn bytes(&mut self, bytes: usize) -> Result<()> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(incomplete)?;
        if self.bytes > MAX_SOURCE_BYTES {
            Err(incomplete())
        } else {
            Ok(())
        }
    }
}

pub(crate) fn verify_object(kind: &str, object: &str, bytes: &[u8]) -> Result<()> {
    let mut digest = Sha1::new();
    digest.update(format!("{kind} {}\0", bytes.len()).as_bytes());
    digest.update(bytes);
    if format!("{:x}", digest.finalize()) != object {
        return Err(incomplete());
    }
    Ok(())
}

pub(crate) fn oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn incomplete() -> Error {
    Error::Invalid { message:"Git source snapshot is incomplete or exceeds its admitted bounds; no successful cursor was advanced".to_owned() }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    fn git(directory: &std::path::Path, args: &[&str]) {
        let result = Command::new("git")
            .args([
                "-c",
                "user.name=Fictional fixture",
                "-c",
                "user.email=fixture@example.test",
            ])
            .args(args)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(result.status.success(), "fixture Git failed");
    }

    #[tokio::test]
    async fn immutable_reads_prove_deletion_and_do_not_follow_a_later_force_push() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let tenant = TenantId::new();
        let scope = ScopeId::new();
        let reader = LocalSourceReader::new(root.clone()).unwrap();
        let repository = reader.repository(tenant, scope, "fictional").unwrap();
        std::fs::create_dir_all(repository.parent().unwrap()).unwrap();
        for directory in [
            root.join(tenant.to_string()),
            repository.parent().unwrap().to_owned(),
        ] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let work = root.join("work");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "--template=", "--initial-branch=main"]);
        std::fs::write(work.join("index.md"), "Fictional retry cap 3.").unwrap();
        git(&work, &["add", "index.md"]);
        git(&work, &["commit", "-m", "fixture"]);
        git(
            &root,
            &[
                "clone",
                "--bare",
                work.to_str().unwrap(),
                repository.to_str().unwrap(),
            ],
        );
        std::fs::set_permissions(&repository, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pin = reader
            .resolve(tenant, scope, "fictional", "refs/heads/main")
            .await
            .unwrap();
        let paths = vec!["index.md".into(), "missing.md".into()];
        let first = reader
            .read_at(tenant, scope, "fictional", &pin, &paths)
            .await
            .unwrap();
        assert_eq!(
            first.files[0].bytes.as_deref(),
            Some(b"Fictional retry cap 3.".as_slice())
        );
        assert_eq!(first.files[1].bytes, None);
        let omit = BTreeMap::from([(
            "index.md".to_owned(),
            first.files[0].blob_oid.clone().unwrap(),
        )]);
        let suppressed = reader
            .read_at_omitting(tenant, scope, "fictional", &pin, &paths, &omit)
            .await
            .unwrap();
        assert!(suppressed.files[0].omitted);
        assert!(suppressed.files[0].bytes.is_none());
        assert_eq!(suppressed.files[0].blob_oid, first.files[0].blob_oid);
        assert!(
            !suppressed.files[1].omitted,
            "suppression cannot fabricate absence"
        );
        std::fs::remove_file(work.join("index.md")).unwrap();
        std::fs::write(work.join("unrelated.md"), "Unrelated bytes").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-m", "deletion"]);
        git(&work, &["push", repository.to_str().unwrap(), "main:main"]);
        let second_pin = reader
            .resolve(tenant, scope, "fictional", "refs/heads/main")
            .await
            .unwrap();
        assert_ne!(pin, second_pin);
        let mixed = SourcePin {
            commit: pin.commit.clone(),
            tree: second_pin.tree.clone(),
        };
        assert!(
            reader
                .read_at(tenant, scope, "fictional", &mixed, &paths)
                .await
                .is_err()
        );
        assert_eq!(
            reader
                .read_at(tenant, scope, "fictional", &pin, &paths)
                .await
                .unwrap(),
            first
        );
        assert!(
            reader
                .read_at(tenant, scope, "fictional", &second_pin, &paths)
                .await
                .unwrap()
                .files
                .iter()
                .all(|file| file.bytes.is_none())
        );
        let invalid = SourcePin {
            commit: pin.commit.clone(),
            tree: "0".repeat(40),
        };
        assert!(
            reader
                .read_at(tenant, scope, "fictional", &invalid, &paths)
                .await
                .is_err()
        );
        std::os::unix::fs::symlink("unrelated.md", work.join("index.md")).unwrap();
        git(&work, &["add", "index.md"]);
        git(&work, &["commit", "-m", "link"]);
        git(&work, &["push", repository.to_str().unwrap(), "main:main"]);
        let linked = reader
            .resolve(tenant, scope, "fictional", "refs/heads/main")
            .await
            .unwrap();
        assert!(
            reader
                .read_at(tenant, scope, "fictional", &linked, &paths)
                .await
                .is_err()
        );
    }

    #[test]
    fn object_hash_and_cumulative_bounds_are_independent_of_git_output() {
        assert!(
            verify_object("blob", "0d153be0e34e44db81f0e7b57e8c09028bd00ec7", b"test").is_err()
        );
        let mut budget = ReadBudget::default();
        assert!(budget.bytes(MAX_SOURCE_BYTES).is_ok());
        assert!(budget.bytes(1).is_err());
        for _ in 0..MAX_SOURCE_REQUESTS {
            budget.request().unwrap();
        }
        assert!(budget.request().is_err());
        assert!(parse_tree(b"100644 invalid\0short").is_err());
    }
}

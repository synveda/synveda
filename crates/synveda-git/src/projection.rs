use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use chrono::{DateTime, Utc};
use flate2::{Compression, write::ZlibEncoder};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use synveda_types::{AssetKind, Channel, Error, IdentityId, Result, ScopeId, TenantId};
use synveda_vedaflow::{CommitHash, CommitSignature, StoredCommit, TreeEntry};

/// Maximum complete source DAG, including merge and pinned ancestry.
pub const MAX_COMMITS: usize = 128;
/// Maximum distinct immutable blobs in one snapshot.
pub const MAX_OBJECTS: usize = 1024;
/// Maximum members of any source tree.
pub const MAX_ENTRIES: usize = 2048;
/// Maximum source bytes; rendered Git objects have a separate 4x ceiling.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const FORMAT: &str = "synveda-git-export-v1";

pub(crate) fn invalid() -> Error {
    Error::Invalid {
        message: "Git export evidence is invalid or exceeds its bounds".into(),
    }
}

/// Validate a credential-free destination identifier; paths never come from it.
pub fn validate_target(target: &str) -> Result<()> {
    if target.is_empty()
        || target.len() > 48
        || !target
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !target.as_bytes()[0].is_ascii_alphanumeric()
    {
        return Err(Error::Invalid { message: "Git export target must be 1..=48 lowercase ASCII letters, digits or hyphens, starting with a letter or digit".into() });
    }
    Ok(())
}

/// Exact source object bytes, represented as UTF-8 canonical authored JSON.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectEvidence {
    /// BLAKE3 typed object address.
    pub hash: String,
    /// Only Prompt or ContextPack is admitted.
    pub kind: AssetKind,
    /// Unmodified canonical source bytes.
    pub content: String,
}

/// Flat authored-channel tree membership. Names are data, never Git paths.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryEvidence {
    /// Source tree entry name.
    pub name: String,
    /// Exact immutable object address.
    pub object: String,
}

/// Source tree evidence in bytewise entry-name order.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeEvidence {
    /// BLAKE3 tree address.
    pub hash: String,
    /// Complete flat authored membership.
    pub entries: Vec<EntryEvidence>,
}

/// Immutable source commit fields and retained Ed25519 evidence.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitEvidence {
    /// BLAKE3 source commit address.
    pub hash: String,
    /// Source tree address.
    pub tree: String,
    /// Ordered source parent addresses.
    pub parents: Vec<String>,
    /// Source identity ID; no inferred email or display-name attribution.
    pub author: IdentityId,
    /// Exact source message.
    pub message: String,
    /// Source timestamp, including microseconds in evidence.
    pub committed_at: DateTime<Utc>,
    /// Original governing policy fingerprint.
    pub policy_snapshot_hash: String,
    /// Ed25519 signature bytes; absent means unsigned.
    pub signature: Option<Vec<u8>>,
    /// Independently trusted key lookup identifier.
    pub signer_key_id: Option<String>,
}

impl CommitEvidence {
    /// Project a stored source commit without altering evidentiary fields.
    #[must_use]
    pub fn from_stored(commit: &StoredCommit) -> Self {
        Self {
            hash: commit.hash.to_hex(),
            tree: commit.tree.to_hex(),
            parents: commit.parents.iter().map(|p| p.to_hex()).collect(),
            author: commit.author,
            message: commit.message.clone(),
            committed_at: commit.committed_at,
            policy_snapshot_hash: commit.policy_snapshot_hash.to_hex(),
            signature: commit.signature.as_ref().map(|s| s.signature.clone()),
            signer_key_id: commit.signature.as_ref().map(|s| s.key_id.clone()),
        }
    }

    fn stored(&self) -> Result<StoredCommit> {
        let signature = match (&self.signature, &self.signer_key_id) {
            (None, None) => None,
            (Some(bytes), Some(key_id))
                if bytes.len() == 64 && !key_id.is_empty() && key_id.len() <= 128 =>
            {
                Some(CommitSignature {
                    signature: bytes.clone(),
                    key_id: key_id.clone(),
                })
            }
            _ => return Err(invalid()),
        };
        Ok(StoredCommit {
            hash: self.hash.parse()?,
            tree: self.tree.parse()?,
            parents: self
                .parents
                .iter()
                .map(|p| p.parse())
                .collect::<Result<_>>()?,
            author: self.author,
            message: self.message.clone(),
            committed_at: self.committed_at,
            policy_snapshot_hash: self.policy_snapshot_hash.parse()?,
            signature,
        })
    }
}

/// Exact pin state, separate from the channel's moving head.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinEvidence {
    /// Source commit served by the pin.
    pub commit: String,
    /// Original pin time.
    pub pinned_at: DateTime<Utc>,
    /// Original pin actor.
    pub pinned_by: IdentityId,
}

/// A bounded, fully authorized frozen source graph.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    /// Tenant derived from the caller's credential.
    pub tenant: TenantId,
    /// Channel governing scope.
    pub scope: ScopeId,
    /// Governed destination identifier.
    pub target: String,
    /// Authored asset family.
    pub asset: AssetKind,
    /// Published or staged source channel.
    pub channel: Channel,
    /// Actual source ref head.
    pub head: String,
    /// Actual source ref update time.
    pub updated_at: DateTime<Utc>,
    /// Actual source ref updater.
    pub updated_by: IdentityId,
    /// Standing pin, when present.
    pub pin: Option<PinEvidence>,
    /// All reachable source commits, hash-sorted.
    pub commits: Vec<CommitEvidence>,
    /// All referenced source trees, hash-sorted.
    pub trees: Vec<TreeEvidence>,
    /// All referenced source objects, hash-sorted.
    pub objects: Vec<ObjectEvidence>,
}

impl Snapshot {
    /// Stable branch name; repository identity already isolates tenant/scope/target.
    #[must_use]
    pub fn git_ref(&self) -> String {
        format!("refs/heads/synveda/{}/{}", self.asset, self.channel)
    }

    /// Canonical source-state digest used to recognize no-op replay.
    pub fn digest(&self) -> Result<String> {
        Ok(blake3::hash(&json_bytes(self)?).to_hex().to_string())
    }

    /// Verify every hash, graph edge, authored encoding and resource bound.
    pub fn validate(&self) -> Result<()> {
        validate_target(&self.target)?;
        if !self.asset.has_channels()
            || !matches!(self.channel, Channel::Published | Channel::Staged)
            || self.commits.is_empty()
            || self.commits.len() > MAX_COMMITS
            || self.objects.len() > MAX_OBJECTS
            || self.trees.len() > MAX_COMMITS
            || json_bytes(self)?.len() > MAX_BYTES
        {
            return Err(invalid());
        }
        sorted_unique(self.commits.iter().map(|c| c.hash.as_str()))?;
        sorted_unique(self.trees.iter().map(|t| t.hash.as_str()))?;
        sorted_unique(self.objects.iter().map(|o| o.hash.as_str()))?;
        let objects: BTreeMap<_, _> = self.objects.iter().map(|o| (o.hash.as_str(), o)).collect();
        for object in &self.objects {
            if object.kind != self.asset
                || synveda_vedaflow::hash::object_hash(object.kind, object.content.as_bytes())
                    .to_hex()
                    != object.hash
            {
                return Err(invalid());
            }
            let canonical = match object.kind {
                AssetKind::Prompt => {
                    synveda_vedaflow::PromptAsset::from_bytes(object.content.as_bytes())?
                        .canonical_bytes()
                }
                AssetKind::ContextPack => {
                    synveda_vedaflow::ContextPackAsset::from_bytes(object.content.as_bytes())?
                        .canonical_bytes()
                }
                _ => return Err(invalid()),
            };
            if canonical != object.content.as_bytes() {
                return Err(invalid());
            }
        }
        let mut used_objects = BTreeSet::new();
        for tree in &self.trees {
            if tree.entries.len() > MAX_ENTRIES {
                return Err(invalid());
            }
            sorted_unique(tree.entries.iter().map(|e| e.name.as_str()))?;
            let mut entries = Vec::new();
            for entry in &tree.entries {
                let object = objects.get(entry.object.as_str()).ok_or_else(invalid)?;
                let name = match object.kind {
                    AssetKind::Prompt => {
                        synveda_vedaflow::PromptAsset::from_bytes(object.content.as_bytes())?
                            .entry_name()
                    }
                    AssetKind::ContextPack => {
                        synveda_vedaflow::ContextPackAsset::from_bytes(object.content.as_bytes())?
                            .entry_name()
                    }
                    _ => return Err(invalid()),
                };
                if entry.name != name || entry.name.len() > 255 {
                    return Err(invalid());
                }
                used_objects.insert(entry.object.as_str());
                entries.push(TreeEntry::object(&entry.name, entry.object.parse()?));
            }
            if synveda_vedaflow::verify::recompute_tree(&entries).to_hex() != tree.hash {
                return Err(invalid());
            }
        }
        let tree_ids: BTreeSet<_> = self.trees.iter().map(|t| t.hash.as_str()).collect();
        let mut used_trees = BTreeSet::new();
        for commit in &self.commits {
            if !tree_ids.contains(commit.tree.as_str())
                || commit.parents.len() > MAX_COMMITS
                || commit.message.is_empty()
                || commit.message.chars().count() > 4096
                || commit.committed_at.timestamp_subsec_nanos() % 1000 != 0
                || commit.parents.iter().collect::<BTreeSet<_>>().len() != commit.parents.len()
                || synveda_vedaflow::verify::recompute_commit(&commit.stored()?).to_hex()
                    != commit.hash
            {
                return Err(invalid());
            }
            used_trees.insert(commit.tree.as_str());
        }
        let order = self.order()?;
        if order.len() != self.commits.len()
            || used_trees.len() != tree_ids.len()
            || used_objects.len() != objects.len()
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn order(&self) -> Result<Vec<&CommitEvidence>> {
        let commits: BTreeMap<_, _> = self.commits.iter().map(|c| (c.hash.as_str(), c)).collect();
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut out = Vec::new();
        visit(&self.head, &commits, &mut visiting, &mut visited, &mut out)?;
        if let Some(pin) = &self.pin {
            visit(&pin.commit, &commits, &mut visiting, &mut visited, &mut out)?;
        }
        Ok(out)
    }
}

fn visit<'a>(
    hash: &'a str,
    commits: &BTreeMap<&str, &'a CommitEvidence>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
    out: &mut Vec<&'a CommitEvidence>,
) -> Result<()> {
    if visited.contains(hash) {
        return Ok(());
    }
    if !visiting.insert(hash) {
        return Err(invalid());
    }
    let commit = *commits.get(hash).ok_or_else(invalid)?;
    for parent in &commit.parents {
        visit(parent, commits, visiting, visited, out)?;
    }
    visiting.remove(hash);
    visited.insert(hash);
    out.push(commit);
    Ok(())
}

fn sorted_unique<'a>(iter: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut previous = None;
    for value in iter {
        if previous.is_some_and(|p| p >= value) {
            return Err(invalid());
        }
        previous = Some(value);
    }
    Ok(())
}

/// Independently reproducible manifest retained in the export-state tree.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Fixed format discriminator.
    pub format: String,
    /// Complete source evidence.
    pub snapshot: Snapshot,
    /// Previous export-state Git commit, preserving rollback history.
    pub previous_state: Option<String>,
    /// VedaFlow object address to Git blob address.
    pub objects: BTreeMap<String, String>,
    /// VedaFlow tree address to Git assets subtree address.
    pub trees: BTreeMap<String, String>,
    /// VedaFlow commit address to Git commit address.
    pub commits: BTreeMap<String, String>,
    /// BLAKE3 of the canonical mapping and source-state evidence.
    pub mapping_digest: String,
}

pub(crate) struct GitObject {
    pub(crate) kind: &'static str,
    pub(crate) bytes: Vec<u8>,
}

/// An inert, deterministic set of Git objects ready for an authorized transport.
pub struct Projection {
    /// Complete mapping and source evidence.
    pub manifest: Manifest,
    /// Resulting unsigned export-state Git commit.
    pub head: String,
    pub(crate) objects: BTreeMap<String, GitObject>,
}

impl Projection {
    /// Render a snapshot, preserving source parent order and exact blob bytes.
    pub fn render(snapshot: Snapshot, previous: Option<String>) -> Result<Self> {
        snapshot.validate().map_err(|_| invalid())?;
        if previous.as_ref().is_some_and(|oid| !valid_oid(oid)) {
            return Err(invalid());
        }
        let mut objects = BTreeMap::new();
        let mut blobs = BTreeMap::new();
        for object in &snapshot.objects {
            blobs.insert(
                object.hash.clone(),
                put(&mut objects, "blob", object.content.as_bytes().to_vec()),
            );
        }
        let mut trees = BTreeMap::new();
        for tree in &snapshot.trees {
            let members = tree
                .entries
                .iter()
                .map(|entry| {
                    Ok((
                        asset_path(&entry.name),
                        blobs.get(&entry.object).ok_or_else(invalid)?.clone(),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            trees.insert(tree.hash.clone(), asset_tree(&mut objects, members)?);
        }
        let tree_evidence: BTreeMap<_, _> = snapshot
            .trees
            .iter()
            .map(|t| (t.hash.as_str(), t))
            .collect();
        let mut commits = BTreeMap::new();
        for commit in snapshot.order()? {
            let commit_blob = put(&mut objects, "blob", json_bytes(commit)?);
            let tree_blob = put(
                &mut objects,
                "blob",
                json_bytes(
                    tree_evidence
                        .get(commit.tree.as_str())
                        .ok_or_else(invalid)?,
                )?,
            );
            let evidence = git_tree(
                &mut objects,
                BTreeMap::from([
                    ("commit.json".into(), ("100644", commit_blob)),
                    ("tree.json".into(), ("100644", tree_blob)),
                ]),
            )?;
            let root = git_tree(
                &mut objects,
                BTreeMap::from([
                    (".synveda".into(), ("40000", evidence)),
                    (
                        "assets".into(),
                        (
                            "40000",
                            trees.get(&commit.tree).ok_or_else(invalid)?.clone(),
                        ),
                    ),
                ]),
            )?;
            let parents = commit
                .parents
                .iter()
                .map(|p| commits.get(p).cloned().ok_or_else(invalid))
                .collect::<Result<Vec<String>>>()?;
            let who = format!(
                "Synveda {} <{}@identity.synveda.invalid>",
                commit.author, commit.author
            );
            let bytes = git_commit(&root, &parents, &who, commit.committed_at, &commit.message);
            commits.insert(commit.hash.clone(), put(&mut objects, "commit", bytes));
        }
        let digest = blake3::hash(&json_bytes(&(
            &snapshot, &previous, &blobs, &trees, &commits,
        ))?)
        .to_hex()
        .to_string();
        let manifest = Manifest {
            format: FORMAT.into(),
            snapshot,
            previous_state: previous,
            objects: blobs,
            trees,
            commits,
            mapping_digest: digest,
        };
        let manifest_blob = put(&mut objects, "blob", json_bytes(&manifest)?);
        let head_source = manifest
            .snapshot
            .commits
            .iter()
            .find(|c| c.hash == manifest.snapshot.head)
            .ok_or_else(invalid)?;
        let head_tree = tree_evidence_for(&manifest.snapshot, &head_source.tree)?;
        let commit_blob = put(&mut objects, "blob", json_bytes(head_source)?);
        let tree_blob = put(&mut objects, "blob", json_bytes(head_tree)?);
        let evidence = git_tree(
            &mut objects,
            BTreeMap::from([
                ("commit.json".into(), ("100644", commit_blob)),
                ("tree.json".into(), ("100644", tree_blob)),
                ("export.json".into(), ("100644", manifest_blob)),
            ]),
        )?;
        let root = git_tree(
            &mut objects,
            BTreeMap::from([
                (".synveda".into(), ("40000", evidence)),
                (
                    "assets".into(),
                    (
                        "40000",
                        manifest
                            .trees
                            .get(&head_source.tree)
                            .ok_or_else(invalid)?
                            .clone(),
                    ),
                ),
            ]),
        )?;
        let mut parents: Vec<String> = manifest.previous_state.iter().cloned().collect();
        parents.push(
            manifest
                .commits
                .get(&manifest.snapshot.head)
                .ok_or_else(invalid)?
                .clone(),
        );
        if let Some(pin) = &manifest.snapshot.pin {
            let pinned = manifest.commits.get(&pin.commit).ok_or_else(invalid)?;
            if !parents.contains(pinned) {
                parents.push(pinned.clone());
            }
        }
        let at = manifest
            .snapshot
            .pin
            .as_ref()
            .map_or(manifest.snapshot.updated_at, |p| {
                p.pinned_at.max(manifest.snapshot.updated_at)
            });
        let head = put(
            &mut objects,
            "commit",
            git_commit(
                &root,
                &parents,
                "Synveda Git bridge <bridge@synveda.invalid>",
                at,
                &format!("Synveda export {}\n", manifest.mapping_digest),
            ),
        );
        if objects.values().map(|o| o.bytes.len()).sum::<usize>() > MAX_BYTES * 4 {
            return Err(invalid());
        }
        Ok(Self {
            manifest,
            head,
            objects,
        })
    }

    /// Serialize a standard Git v2 pack, independently checked by Git index-pack.
    pub(crate) fn pack(&self) -> Result<Vec<u8>> {
        let mut pack = b"PACK\0\0\0\x02".to_vec();
        pack.extend_from_slice(&(self.objects.len() as u32).to_be_bytes());
        for object in self.objects.values() {
            let tag = match object.kind {
                "commit" => 1,
                "tree" => 2,
                "blob" => 3,
                _ => return Err(invalid()),
            };
            let mut remaining = object.bytes.len();
            let mut byte = (tag << 4) | (remaining as u8 & 15);
            remaining >>= 4;
            if remaining != 0 {
                byte |= 128;
            }
            pack.push(byte);
            while remaining != 0 {
                byte = remaining as u8 & 127;
                remaining >>= 7;
                if remaining != 0 {
                    byte |= 128;
                }
                pack.push(byte);
            }
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&object.bytes).map_err(|_| invalid())?;
            pack.extend_from_slice(&encoder.finish().map_err(|_| invalid())?);
        }
        let checksum = Sha1::digest(&pack);
        pack.extend_from_slice(&checksum);
        if pack.len() > MAX_BYTES * 5 {
            return Err(invalid());
        }
        Ok(pack)
    }
}

fn tree_evidence_for<'a>(snapshot: &'a Snapshot, hash: &str) -> Result<&'a TreeEvidence> {
    snapshot
        .trees
        .iter()
        .find(|t| t.hash == hash)
        .ok_or_else(invalid)
}

fn put(objects: &mut BTreeMap<String, GitObject>, kind: &'static str, bytes: Vec<u8>) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{kind} {}\0", bytes.len()).as_bytes());
    hasher.update(&bytes);
    let oid = hex(&hasher.finalize());
    objects
        .entry(oid.clone())
        .or_insert(GitObject { kind, bytes });
    oid
}

fn git_tree(
    objects: &mut BTreeMap<String, GitObject>,
    entries: BTreeMap<String, (&str, String)>,
) -> Result<String> {
    let mut bytes = Vec::new();
    // Every entry here has a generated ASCII name; directory ordering uses
    // Git's trailing slash convention rather than the source tree collation.
    let mut sorted: Vec<_> = entries.into_iter().collect();
    sorted.sort_by_key(|(name, (mode, _))| {
        if *mode == "40000" {
            format!("{name}/")
        } else {
            name.clone()
        }
    });
    for (name, (mode, oid)) in sorted {
        bytes.extend_from_slice(format!("{mode} {name}\0").as_bytes());
        bytes.extend_from_slice(&decode_oid(&oid)?);
    }
    Ok(put(objects, "tree", bytes))
}

// Segment the injective hex encoding so every checkout component fits under
// NAME_MAX, including the longest admitted source tree name.
fn asset_path(name: &str) -> String {
    let encoded = hex(name.as_bytes());
    let components: Vec<_> = encoded
        .as_bytes()
        .chunks(120)
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect();
    format!("{}.json", components.join("/"))
}

fn asset_tree(
    objects: &mut BTreeMap<String, GitObject>,
    files: BTreeMap<String, String>,
) -> Result<String> {
    let mut entries = BTreeMap::new();
    let mut directories: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (path, oid) in files {
        if let Some((directory, tail)) = path.split_once('/') {
            directories
                .entry(directory.into())
                .or_default()
                .insert(tail.into(), oid);
        } else {
            entries.insert(path, ("100644", oid));
        }
    }
    for (name, files) in directories {
        if entries.contains_key(&name) {
            return Err(invalid());
        }
        entries.insert(name, ("40000", asset_tree(objects, files)?));
    }
    git_tree(objects, entries)
}

fn git_commit(
    tree: &str,
    parents: &[String],
    who: &str,
    at: DateTime<Utc>,
    message: &str,
) -> Vec<u8> {
    let mut content = format!("tree {tree}\n");
    for parent in parents {
        content.push_str(&format!("parent {parent}\n"));
    }
    content.push_str(&format!(
        "author {who} {} +0000\ncommitter {who} {} +0000\n\n{message}",
        at.timestamp(),
        at.timestamp()
    ));
    content.into_bytes()
}

pub(crate) fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| invalid())
}

pub(crate) fn valid_oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode_oid(value: &str) -> Result<[u8; 20]> {
    if !valid_oid(value) {
        return Err(invalid());
    }
    let mut bytes = [0; 20];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).map_err(|_| invalid())?;
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Verify signatures only against independently supplied public keys.
pub(crate) fn verify_signatures(
    snapshot: &Snapshot,
    keys: &BTreeMap<String, [u8; 32]>,
) -> Result<()> {
    for commit in &snapshot.commits {
        if let (Some(bytes), Some(id)) = (&commit.signature, &commit.signer_key_id) {
            let key = keys.get(id).ok_or_else(|| Error::Invalid {
                message: "Git export signature has no trusted public key".into(),
            })?;
            if !synveda_vedaflow::verify_ed25519(commit.hash.parse::<CommitHash>()?, bytes, key) {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

//! MEM-9 fixed-origin, GET-only GitHub source reads (ADR-0149).

use crate::sources::{oid, verify_object};
use crate::{
    MAX_SOURCE_BLOB_BYTES, MAX_SOURCE_BYTES, MAX_SOURCE_REQUESTS, SourceFile, SourcePin,
    SourceSnapshot,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, ClientBuilder};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use synveda_types::{Error, Result, ScopeId, TenantId, TenantSecretId};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

fn incomplete() -> Error {
    Error::Dependency {
        service: "github-source".to_owned(),
        message:
            "GitHub source snapshot is incomplete or unavailable; no successful cursor was advanced"
                .to_owned(),
    }
}
pub(crate) fn unavailable() -> Error {
    Error::Conflict {
        message:
            "GitHub source identity or read custody is unavailable; inspect its configured target"
                .to_owned(),
    }
}

/// Distinct deployment assertion for private inbound reads; never export custody.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubSourceTarget {
    /// Exact tenant.
    pub tenant_id: TenantId,
    /// Actual project scope and required credential scope.
    pub scope_id: ScopeId,
    /// Governed read target slug.
    pub target: String,
    /// Canonical lowercase owner.
    pub owner: String,
    /// Canonical lowercase repository, without `.git`.
    pub repository: String,
    /// Independently pinned provider identity.
    pub repository_id: u64,
    /// Exact scoped SourceRead tenant-secret reference.
    pub secret_reference: String,
}
impl GitHubSourceTarget {
    /// Parse internal custody without returning credential content.
    pub fn secret_id(&self) -> Result<TenantSecretId> {
        synveda_types::secret::parse_tenant_secret_reference(&self.secret_reference)
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)
    }
    fn validate(&self) -> Result<()> {
        crate::validate_target(&self.target)?;
        if self.owner.is_empty()
            || self.owner.len() > 39
            || !self
                .owner
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || !self.owner.as_bytes()[0].is_ascii_alphanumeric()
            || !self.owner.as_bytes()[self.owner.len() - 1].is_ascii_alphanumeric()
            || self.owner.contains("--")
            || self.repository.is_empty()
            || self.repository.len() > 100
            || !self
                .repository
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
            || !self.repository.as_bytes()[0].is_ascii_alphanumeric()
            || self.repository.ends_with(".git")
            || self.repository_id == 0
            || self.repository_id > i64::MAX as u64
        {
            return Err(unavailable());
        }
        self.secret_id()?;
        Ok(())
    }
    fn name(&self) -> String {
        format!("{}/{}", self.owner, self.repository)
    }
    /// Canonical actual attachment required by admission.
    #[must_use]
    pub fn repository_uri(&self) -> String {
        format!("https://github.com/{}", self.name())
    }
    fn api_url(&self) -> String {
        format!("https://api.github.com/repos/{}", self.name())
    }
}

/// Bounded, versioned deployment allowlist for inbound custody.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubSourceTargets {
    format: String,
    targets: Vec<GitHubSourceTarget>,
}
impl GitHubSourceTargets {
    /// Decode without exposing untrusted document details in errors.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 65_536 {
            return Err(unavailable());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| unavailable())?;
        if value.format != "synveda-github-source-targets-v1" {
            return Err(unavailable());
        }
        validate_targets(&value.targets)?;
        Ok(value)
    }
    /// Consume only validated descriptors.
    #[must_use]
    pub fn into_targets(self) -> Vec<GitHubSourceTarget> {
        self.targets
    }
}
fn validate_targets(targets: &[GitHubSourceTarget]) -> Result<()> {
    if targets.len() > 128 {
        return Err(unavailable());
    }
    let mut identities = BTreeSet::new();
    let mut names = BTreeMap::new();
    for target in targets {
        target.validate()?;
        if !identities.insert((target.tenant_id, target.scope_id, &target.target)) {
            return Err(unavailable());
        }
        if let Some(old) = names.insert(target.name(), target.repository_id)
            && old != target.repository_id
        {
            return Err(unavailable());
        }
    }
    Ok(())
}

/// Opened read credential; token is wiped and cannot be serialized.
pub struct GitHubSourceCredential {
    repository_id: u64,
    token: Zeroizing<String>,
}
impl std::fmt::Debug for GitHubSourceCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GitHubSourceCredential([REDACTED])")
    }
}
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    format: String,
    repository_id: u64,
    token: String,
}
impl GitHubSourceCredential {
    /// Refuse export documents, mismatched provider IDs and malformed tokens.
    pub fn parse(bytes: &[u8], repository_id: u64) -> Result<Self> {
        if bytes.len() > 4096 {
            return Err(unavailable());
        }
        let mut document: CredentialDocument =
            serde_json::from_slice(bytes).map_err(|_| unavailable())?;
        if document.format != "synveda-github-source-credential-v1"
            || document.repository_id != repository_id
            || repository_id == 0
            || !document.token.starts_with("github_pat_")
            || !(20..=512).contains(&document.token.len())
            || !document
                .token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(unavailable());
        }
        Ok(Self {
            repository_id,
            token: Zeroizing::new(std::mem::take(&mut document.token)),
        })
    }
}

pub(crate) fn client_builder() -> ClientBuilder {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent("synveda-git-source/1")
}
/// Fixed-origin reader; test DNS/CA overrides are confined to its private tests.
#[derive(Clone)]
pub struct GitHubSourceReader {
    pub(crate) targets: Vec<GitHubSourceTarget>,
    client: Client,
}
#[derive(Default)]
struct Budget {
    requests: usize,
    bytes: usize,
}
impl GitHubSourceReader {
    /// Validate deployment custody and build an isolated HTTPS client.
    pub fn new(targets: Vec<GitHubSourceTarget>) -> Result<Self> {
        validate_targets(&targets)?;
        Ok(Self {
            targets,
            client: client_builder().build().map_err(|_| incomplete())?,
        })
    }
    /// Controlled HTTPS fixture trust; production construction retains fixed CA/DNS.
    #[cfg(feature = "test-support")]
    pub fn for_controlled_https(
        targets: Vec<GitHubSourceTarget>,
        certificate_pem: &[u8],
        address: std::net::SocketAddr,
    ) -> Result<Self> {
        validate_targets(&targets)?;
        let certificate =
            reqwest::Certificate::from_pem(certificate_pem).map_err(|_| incomplete())?;
        Ok(Self {
            targets,
            client: client_builder()
                .add_root_certificate(certificate)
                .resolve("api.github.com", address)
                .build()
                .map_err(|_| incomplete())?,
        })
    }
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        target: &GitHubSourceTarget,
        credential: &GitHubSourceCredential,
        suffix: &str,
        limit: usize,
        budget: &mut Budget,
    ) -> Result<T> {
        if credential.repository_id != target.repository_id {
            return Err(unavailable());
        }
        budget.requests = budget.requests.checked_add(1).ok_or_else(incomplete)?;
        if budget.requests > MAX_SOURCE_REQUESTS {
            return Err(incomplete());
        }
        let mut response = self
            .client
            .get(format!("{}{suffix}", target.api_url()))
            .bearer_auth(credential.token.as_str())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|_| incomplete())?;
        if response.status() != reqwest::StatusCode::OK
            || response.headers().contains_key("location")
            || !response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("application/json"))
            || response
                .content_length()
                .is_some_and(|length| length > limit as u64)
        {
            return Err(incomplete());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| incomplete())? {
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|n| n > limit)
            {
                return Err(incomplete());
            }
            budget.bytes = budget
                .bytes
                .checked_add(chunk.len())
                .ok_or_else(incomplete)?;
            if budget.bytes > MAX_SOURCE_BYTES {
                return Err(incomplete());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| incomplete())
    }
    async fn metadata(
        &self,
        target: &GitHubSourceTarget,
        credential: &GitHubSourceCredential,
        budget: &mut Budget,
    ) -> Result<()> {
        #[derive(Deserialize)]
        struct Repository {
            id: u64,
            full_name: String,
            private: bool,
            visibility: String,
            archived: bool,
            disabled: bool,
            fork: bool,
        }
        let r: Repository = self.get(target, credential, "", 16_384, budget).await?;
        if r.id != target.repository_id
            || !r.full_name.eq_ignore_ascii_case(&target.name())
            || !r.private
            || r.visibility != "private"
            || r.archived
            || r.disabled
            || r.fork
        {
            return Err(unavailable());
        }
        Ok(())
    }
    async fn commit(
        &self,
        target: &GitHubSourceTarget,
        credential: &GitHubSourceCredential,
        commit: &str,
        budget: &mut Budget,
    ) -> Result<SourcePin> {
        #[derive(Deserialize)]
        struct Tree {
            sha: String,
        }
        #[derive(Deserialize)]
        struct Commit {
            sha: String,
            tree: Tree,
        }
        if !oid(commit) {
            return Err(incomplete());
        }
        let r: Commit = self
            .get(
                target,
                credential,
                &format!("/git/commits/{commit}"),
                65_536,
                budget,
            )
            .await?;
        if r.sha != commit || !oid(&r.tree.sha) {
            return Err(incomplete());
        }
        Ok(SourcePin {
            commit: r.sha,
            tree: r.tree.sha,
        })
    }
    /// Resolve one exact branch after independently verifying repository identity.
    pub async fn resolve(
        &self,
        target: &GitHubSourceTarget,
        credential: &GitHubSourceCredential,
        reference: &str,
    ) -> Result<SourcePin> {
        synveda_types::source::validate_source_ref(reference)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut budget = Budget::default();
            self.metadata(target, credential, &mut budget).await?;
            #[derive(Deserialize)]
            struct Object {
                sha: String,
                #[serde(rename = "type")]
                kind: String,
            }
            #[derive(Deserialize)]
            struct Ref {
                #[serde(rename = "ref")]
                reference: String,
                object: Object,
            }
            let r: Ref = self
                .get(
                    target,
                    credential,
                    &format!(
                        "/git/ref/{}",
                        reference.strip_prefix("refs/").ok_or_else(incomplete)?
                    ),
                    16_384,
                    &mut budget,
                )
                .await?;
            if r.reference != reference || r.object.kind != "commit" {
                return Err(incomplete());
            }
            self.commit(target, credential, &r.object.sha, &mut budget)
                .await
        })
        .await
        .map_err(|_| incomplete())?
    }
    /// Read every exact path at the immutable pin; partial trees prove no absence.
    pub async fn read_at_omitting(
        &self,
        target: &GitHubSourceTarget,
        credential: &GitHubSourceCredential,
        pin: &SourcePin,
        paths: &[String],
        omitted: &BTreeMap<String, String>,
    ) -> Result<SourceSnapshot> {
        pin.validate()?;
        synveda_types::source::validate_source_paths(paths)?;
        if omitted
            .iter()
            .any(|(path, object)| !paths.contains(path) || !oid(object))
        {
            return Err(incomplete());
        }
        tokio::time::timeout(Duration::from_secs(30), async {
            // Charge resolve's maximum metadata/ref/commit work even on a retry.
            let mut budget = Budget {
                requests: 3,
                bytes: 98_304,
            };
            self.metadata(target, credential, &mut budget).await?;
            if self
                .commit(target, credential, &pin.commit, &mut budget)
                .await?
                != *pin
            {
                return Err(incomplete());
            }
            let mut trees = BTreeMap::<String, Vec<TreeEntry>>::new();
            let mut files = Vec::new();
            for path in paths {
                let parts: Vec<_> = path.split('/').collect();
                let mut tree = pin.tree.clone();
                let mut found = None;
                for (index, part) in parts.iter().enumerate() {
                    if !trees.contains_key(&tree) {
                        let r: TreeResponse = self
                            .get(
                                target,
                                credential,
                                &format!("/git/trees/{tree}"),
                                262_144,
                                &mut budget,
                            )
                            .await?;
                        trees.insert(tree.clone(), verify_tree(r, &tree)?);
                    }
                    let Some(entry) = trees
                        .get(&tree)
                        .and_then(|entries| entries.iter().find(|entry| entry.path == *part))
                        .cloned()
                    else {
                        break;
                    };
                    if index + 1 < parts.len() {
                        if entry.mode != "040000" || entry.kind != "tree" {
                            return Err(incomplete());
                        }
                        tree = entry.sha;
                    } else {
                        if !matches!(entry.mode.as_str(), "100644" | "100755")
                            || entry.kind != "blob"
                        {
                            return Err(incomplete());
                        }
                        if omitted.get(path) == Some(&entry.sha) {
                            found = Some((entry.sha, None));
                            break;
                        }
                        if entry.size.is_none_or(|size| size > MAX_SOURCE_BLOB_BYTES) {
                            return Err(incomplete());
                        }
                        #[derive(Deserialize)]
                        struct Blob {
                            sha: String,
                            size: usize,
                            encoding: String,
                            content: String,
                        }
                        let r: Blob = self
                            .get(
                                target,
                                credential,
                                &format!("/git/blobs/{}", entry.sha),
                                196_608,
                                &mut budget,
                            )
                            .await?;
                        if r.sha != entry.sha
                            || r.encoding != "base64"
                            || Some(r.size) != entry.size
                            || r.size > MAX_SOURCE_BLOB_BYTES
                        {
                            return Err(incomplete());
                        }
                        let encoded: Vec<_> = r
                            .content
                            .bytes()
                            .filter(|byte| !byte.is_ascii_whitespace())
                            .collect();
                        let bytes = STANDARD.decode(encoded).map_err(|_| incomplete())?;
                        if bytes.len() != r.size {
                            return Err(incomplete());
                        }
                        verify_object("blob", &entry.sha, &bytes).map_err(|_| incomplete())?;
                        found = Some((entry.sha, Some(bytes)));
                    }
                }
                files.push(match found {
                    Some((oid, bytes)) => SourceFile {
                        path: path.clone(),
                        blob_oid: Some(oid),
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
#[derive(Clone, Deserialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    size: Option<usize>,
}
#[derive(Deserialize)]
struct TreeResponse {
    sha: String,
    truncated: bool,
    tree: Vec<TreeEntry>,
}
fn verify_tree(mut value: TreeResponse, wanted: &str) -> Result<Vec<TreeEntry>> {
    if value.sha != wanted || value.truncated || value.tree.len() > 2000 {
        return Err(incomplete());
    }
    let mut names = BTreeSet::new();
    for e in &value.tree {
        if e.path.is_empty()
            || e.path.len() > 255
            || e.path.contains('/')
            || e.path.contains('\0')
            || !oid(&e.sha)
            || !names.insert(&e.path)
            || !matches!(
                (e.mode.as_str(), e.kind.as_str()),
                ("040000", "tree")
                    | ("100644" | "100755" | "120000", "blob")
                    | ("160000", "commit")
            )
        {
            return Err(incomplete());
        }
    }
    value.tree.sort_by_cached_key(|entry| {
        format!(
            "{}{}",
            entry.path,
            if entry.mode == "040000" { "/" } else { "" }
        )
    });
    let mut raw = Vec::new();
    for e in &value.tree {
        let mode = if e.mode == "040000" { "40000" } else { &e.mode };
        raw.extend_from_slice(format!("{mode} {}\0", e.path).as_bytes());
        for pair in e.sha.as_bytes().chunks_exact(2) {
            let hex = std::str::from_utf8(pair).map_err(|_| incomplete())?;
            raw.push(u8::from_str_radix(hex, 16).map_err(|_| incomplete())?);
        }
        if raw.len() > MAX_SOURCE_BLOB_BYTES {
            return Err(incomplete());
        }
    }
    verify_object("tree", wanted, &raw).map_err(|_| incomplete())?;
    Ok(value.tree)
}

//! FLOW-8 private GitHub.com export. Only fixed HTTPS origins and smart-HTTP
//! receive-pack are supported; credentials never enter a native Git process.

use std::collections::BTreeSet;
use std::time::Duration;

use reqwest::{Client, ClientBuilder, Response, StatusCode};
use serde::Deserialize;
use synveda_types::{Error, Result, ScopeId, TenantId, TenantSecretId};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Projection, validate_target, wire};

/// Maximum deployment allowlist bytes, before parsing untrusted JSON.
const MAX_TARGET_BYTES: usize = 64 * 1024;
const MAX_TARGETS: usize = 128;
const MAX_CREDENTIAL_BYTES: usize = 4096;

pub(crate) fn failure() -> Error {
    Error::Internal {
        message: "GitHub export transport failed; retry to recover the prepared receipt".into(),
    }
}

pub(crate) fn unavailable() -> Error {
    Error::Invalid {
        message: "GitHub export destination or credential is unavailable".into(),
    }
}

pub(crate) fn divergence() -> Error {
    Error::Conflict { message: "GitHub export destination diverged from its recorded identity or cursor; retain it and enable a governed new target identifier".into() }
}

/// One credential-free deployment mapping. Repository IDs cannot be shared
/// between mappings, even across tenants or scopes (ADR-0139).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubTarget {
    /// Exact admitted tenant.
    pub tenant_id: TenantId,
    /// Exact channel scope, also required of the stored secret.
    pub scope_id: ScopeId,
    /// Governed destination slug.
    pub target: String,
    /// Canonical lowercase GitHub.com owner slug.
    pub owner: String,
    /// Canonical lowercase repository name without `.git`.
    pub repository: String,
    /// Independently obtained immutable GitHub repository ID.
    pub repository_id: u64,
    /// Canonical stable `synveda-secret://` reference, never a credential.
    pub secret_reference: String,
}

impl GitHubTarget {
    /// Parse the exact internal secret identity. External references refuse.
    pub fn secret_id(&self) -> Result<TenantSecretId> {
        synveda_types::secret::parse_tenant_secret_reference(&self.secret_reference)
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)
    }

    fn validate(&self) -> Result<()> {
        validate_target(&self.target)?;
        let owner = self.owner.as_bytes();
        let repository = self.repository.as_bytes();
        if owner.is_empty()
            || owner.len() > 39
            || !owner
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
            || !owner[0].is_ascii_alphanumeric()
            || !owner[owner.len() - 1].is_ascii_alphanumeric()
            || self.owner.contains("--")
            || repository.is_empty()
            || repository.len() > 100
            || !repository
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(b))
            || !repository[0].is_ascii_alphanumeric()
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
    fn api_url(&self) -> String {
        format!("https://api.github.com/repos/{}", self.name())
    }
    fn git_url(&self) -> String {
        format!("https://github.com/{}.git", self.name())
    }
}

/// Versioned, bounded operator allowlist. No caller-selected hosts, paths,
/// credentials, repository creation or fallback credential is admitted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubTargets {
    format: String,
    targets: Vec<GitHubTarget>,
}

impl GitHubTargets {
    /// Decode and validate the deployment file without echoing JSON details.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_TARGET_BYTES {
            return Err(unavailable());
        }
        let config: Self = serde_json::from_slice(bytes).map_err(|_| unavailable())?;
        if config.format != "synveda-github-export-targets-v1" {
            return Err(unavailable());
        }
        validate_targets(&config.targets)?;
        Ok(config)
    }

    /// Consume validated descriptors to construct the provider transport.
    #[must_use]
    pub fn into_targets(self) -> Vec<GitHubTarget> {
        self.targets
    }
}

fn validate_targets(targets: &[GitHubTarget]) -> Result<()> {
    if targets.len() > MAX_TARGETS {
        return Err(unavailable());
    }
    let mut identities = BTreeSet::new();
    let mut repositories = BTreeSet::new();
    let mut names = BTreeSet::new();
    for target in targets {
        target.validate()?;
        if !identities.insert((target.tenant_id, target.scope_id, &target.target))
            || !repositories.insert(target.repository_id)
            || !names.insert(target.name())
        {
            return Err(unavailable());
        }
    }
    Ok(())
}

/// An opened, repository-bound fine-grained PAT. Its value is wiped on drop
/// and cannot be serialized or printed through the public adapter interface.
pub struct GitHubCredential {
    repository_id: u64,
    token: Zeroizing<String>,
}

impl std::fmt::Debug for GitHubCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubCredential")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    format: String,
    repository_id: u64,
    token: String,
}

impl GitHubCredential {
    /// Admit only the v1 sealed document bound to this immutable repository.
    /// Parse failures deliberately omit serde errors, which may echo a token.
    pub fn parse(bytes: &[u8], repository_id: u64) -> Result<Self> {
        if bytes.len() > MAX_CREDENTIAL_BYTES {
            return Err(unavailable());
        }
        let mut document: CredentialDocument =
            serde_json::from_slice(bytes).map_err(|_| unavailable())?;
        if document.format != "synveda-github-export-credential-v1"
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
        .timeout(Duration::from_secs(15))
        .user_agent("synveda-git-export/1")
}

/// Private GitHub.com transport with a deployment allowlist and isolated TLS
/// client. Neither native Git configuration nor environment proxies affect it.
#[derive(Clone)]
pub struct GitHubTransport {
    pub(crate) targets: Vec<GitHubTarget>,
    client: Client,
}

impl GitHubTransport {
    /// Validate fixed provider descriptors and build the verified HTTPS client.
    pub fn new(targets: Vec<GitHubTarget>) -> Result<Self> {
        validate_targets(&targets)?;
        Ok(Self {
            targets,
            client: client_builder().build().map_err(|_| failure())?,
        })
    }

    async fn metadata(&self, target: &GitHubTarget, credential: &GitHubCredential) -> Result<()> {
        if credential.repository_id != target.repository_id {
            return Err(unavailable());
        }
        let response = self
            .client
            .get(target.api_url())
            .bearer_auth(credential.token.as_str())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|_| failure())?;
        let bytes = bounded(response, "application/json", wire::MAX_RESPONSE).await?;
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
        let repository: Repository = serde_json::from_slice(&bytes).map_err(|_| failure())?;
        if repository.id != target.repository_id
            || !repository.full_name.eq_ignore_ascii_case(&target.name())
        {
            return Err(divergence());
        }
        if !repository.private
            || repository.visibility != "private"
            || repository.archived
            || repository.disabled
            || repository.fork
        {
            return Err(unavailable());
        }
        Ok(())
    }

    async fn discover(
        &self,
        target: &GitHubTarget,
        credential: &GitHubCredential,
        reference: &str,
    ) -> Result<wire::Advertisement> {
        let response = self
            .client
            .get(format!(
                "{}/info/refs?service=git-receive-pack",
                target.git_url()
            ))
            .basic_auth("x-access-token", Some(credential.token.as_str()))
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .map_err(|_| failure())?;
        let bytes = bounded(
            response,
            "application/x-git-receive-pack-advertisement",
            wire::MAX_RESPONSE,
        )
        .await?;
        wire::advertisement(&bytes, reference)
    }

    pub(crate) async fn advance(
        &self,
        target: &GitHubTarget,
        credential: &GitHubCredential,
        projection: &Projection,
        previous: Option<&str>,
        completed: bool,
    ) -> Result<bool> {
        let source = &projection.manifest.snapshot;
        if source.tenant != target.tenant_id
            || source.scope != target.scope_id
            || source.target != target.target
            || projection.manifest.previous_state.as_deref() != previous
        {
            return Err(divergence());
        }
        self.metadata(target, credential).await?;
        let reference = source.git_ref();
        let observed = self.discover(target, credential, &reference).await?;
        if observed.head.as_deref() == Some(&projection.head) {
            return Ok(false);
        }
        if completed || observed.head.as_deref() != previous {
            return Err(divergence());
        }
        let mut request = wire::update(
            previous,
            &projection.head,
            &reference,
            observed.object_format,
        )?;
        request.extend(projection.pack()?);
        // Repeat identity/privacy immediately before content leaves the process.
        // External owners must still govern visibility; the API and push cannot
        // atomically lock a forge's repository settings.
        self.metadata(target, credential).await?;
        let response = self
            .client
            .post(format!("{}/git-receive-pack", target.git_url()))
            // Pack upload and processing need more time than metadata; leave
            // room for final checks within the gateway's 60-second deadline.
            .timeout(Duration::from_secs(45))
            .basic_auth("x-access-token", Some(credential.token.as_str()))
            .header("Content-Type", "application/x-git-receive-pack-request")
            .header("Accept", "application/x-git-receive-pack-result")
            .body(request)
            .send()
            .await
            .map_err(|_| failure())?;
        wire::report(
            &bounded(
                response,
                "application/x-git-receive-pack-result",
                wire::MAX_RESPONSE,
            )
            .await?,
            &reference,
        )?;
        self.metadata(target, credential).await?;
        if self
            .discover(target, credential, &reference)
            .await?
            .head
            .as_deref()
            != Some(&projection.head)
        {
            return Err(divergence());
        }
        Ok(true)
    }
}

async fn bounded(mut response: Response, content_type: &str, limit: usize) -> Result<Vec<u8>> {
    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err(unavailable());
        }
        _ => return Err(failure()),
    }
    if response
        .headers()
        .get("Content-Type")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(str::trim)
        != Some(content_type)
        || response
            .content_length()
            .is_some_and(|length| length > limit as u64)
    {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failure())? {
        if chunk.len() > limit - bytes.len() {
            return Err(failure());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(all(test, unix))]
#[path = "github_tests.rs"]
mod tests;

#[cfg(test)]
mod validation_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn allowlist_and_credentials_bind_only_exact_private_provider_custody() {
        let descriptor = json!({ "tenant_id": TenantId::new(), "scope_id": ScopeId::new(), "target": "review",
            "owner": "fixture-owner", "repository": "export", "repository_id": 123,
            "secret_reference": synveda_types::secret::tenant_secret_reference(TenantSecretId::new()) });
        let encode = |targets: Vec<serde_json::Value>| {
            serde_json::to_vec(&json!({
            "format": "synveda-github-export-targets-v1", "targets": targets }))
            .unwrap()
        };
        GitHubTargets::parse(&encode(vec![descriptor.clone()])).unwrap();
        assert!(
            GitHubTargets::parse(&encode(vec![descriptor.clone(), descriptor.clone()])).is_err()
        );
        for (key, value) in [
            ("owner", "https://source-secret@other"),
            ("repository", "../source-secret"),
            ("repository", "upperCase"),
            ("secret_reference", "https://source-secret"),
        ] {
            let mut malformed = descriptor.clone();
            malformed[key] = value.into();
            let error = GitHubTargets::parse(&encode(vec![malformed]))
                .err()
                .unwrap();
            assert!(!error.to_string().contains("source-secret"));
        }
        let mut foreign = descriptor.clone();
        foreign["tenant_id"] = TenantId::new().to_string().into();
        assert!(GitHubTargets::parse(&encode(vec![descriptor.clone(), foreign])).is_err());
        assert!(GitHubTargets::parse(&vec![b'x'; MAX_TARGET_BYTES + 1]).is_err());
        let document = json!({ "format": "synveda-github-export-credential-v1", "repository_id": 123,
            "token": "github_pat_fixture_source_secret" });
        let bytes = serde_json::to_vec(&document).unwrap();
        let credential = GitHubCredential::parse(&bytes, 123).unwrap();
        assert!(!format!("{credential:?}").contains("source_secret"));
        assert!(GitHubCredential::parse(&bytes, 124).is_err());
        for token in [
            "ghp_legacy_source_secret",
            "github_pat_fixture_source_secret\nInjected: yes",
        ] {
            let mut invalid = document.clone();
            invalid["token"] = token.into();
            let error = GitHubCredential::parse(&serde_json::to_vec(&invalid).unwrap(), 123)
                .err()
                .unwrap();
            assert!(!error.to_string().contains("source_secret"));
        }
        assert!(GitHubCredential::parse(&vec![b'x'; MAX_CREDENTIAL_BYTES + 1], 123).is_err());
    }
}

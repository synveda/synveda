//! Canonical repository identity (CPR-4, ADR-0071 decision 4).
//!
//! A project is *about* code, and the question this module answers is the one
//! everything downstream depends on: **when are two clients talking about the
//! same repository?** Two agents on two laptops, a CI runner and a container
//! all see the same repository at four different filesystem paths, and one
//! person's checkout moves between `~/src`, `~/work` and `/tmp` in a week. A
//! path is where a repository happens to be sitting; it is not what it is.
//!
//! So: **the canonical remote URI is the identity when one exists**, and a
//! local filesystem path is never the identity at all — not as a fallback, not
//! as a tiebreak, not when nothing else is available. What a repository with no
//! remote gets instead is a **fingerprint**: a stable content identifier the
//! client computes (a git root-commit object id is the obvious one), which
//! survives every move the path does not. [`identify`] refuses a path-shaped
//! `remote_uri` by name rather than accepting it and normalising it into
//! something that looks canonical.
//!
//! ## What canonicalisation does
//!
//! `git@github.com:Acme/payments.git`, `https://github.com/Acme/payments`,
//! `ssh://git@github.com/Acme/payments.git/` and
//! `https://x-token:secret@github.com:443/Acme/payments.git` are one
//! repository, and all four canonicalise to
//! `https://github.com/Acme/payments`:
//!
//! - the transport is dropped (`ssh`, `git`, `http` and `https` all become
//!   `https`) — a repository is not two repositories because two people clone
//!   it differently;
//! - credentials in the authority are dropped, which is also why a canonical
//!   URI is safe to store, log and return (seed: no secret in an ordinary API
//!   response);
//! - the host is lower-cased and a default port removed;
//! - a `.git` suffix and any trailing slash are removed.
//!
//! Path **case is preserved**, because a generic git server's paths may be
//! case-sensitive; uniqueness is enforced case-insensitively by the store, so
//! `Acme/payments` and `acme/payments` are one row that displays the
//! capitalisation the first caller used.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Error, IdentityId, ProjectId, RepositoryId, Result, TenantId};

/// Longest canonical URI, in characters.
pub const MAX_CANONICAL_URI_CHARS: usize = 512;

/// Longest owner path (`acme`, or `acme/platform` for a GitLab subgroup).
pub const MAX_OWNER_CHARS: usize = 255;

/// Longest repository name.
pub const MAX_NAME_CHARS: usize = 255;

/// Longest branch name.
pub const MAX_BRANCH_CHARS: usize = 255;

/// Shortest accepted local fingerprint, in hex characters — a full SHA-1
/// object id. Abbreviations are refused: an identity that collides is not one.
pub const MIN_FINGERPRINT_CHARS: usize = 40;

/// Longest accepted local fingerprint, in hex characters. Comfortably past
/// SHA-256's 64.
pub const MAX_FINGERPRINT_CHARS: usize = 128;

/// Largest `metadata` document, in bytes of its compact JSON encoding.
pub const MAX_METADATA_BYTES: usize = 8 * 1024;

/// The URI scheme minted for a repository that has no remote — a fingerprint,
/// never a path.
pub const FINGERPRINT_SCHEME: &str = "git+fingerprint";

/// Where a repository is hosted.
///
/// Derived from the canonical URI's host, and overridable by the caller for
/// the self-hosted case (a GitHub Enterprise or GitLab instance on a company
/// domain is still GitHub or GitLab). The vocabulary is closed so a UI can
/// render a provider and a future adapter can dispatch on one; it decides
/// nothing about authorisation.
/// The wire names are spelled out per variant rather than derived: serde's
/// `snake_case` rule renders `GitHub` as `git_hub`, and the stored value, the
/// OpenAPI enum and what somebody types are all this string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RepositoryProvider {
    /// github.com or a GitHub Enterprise instance.
    #[serde(rename = "github")]
    GitHub,
    /// gitlab.com or a self-hosted GitLab.
    #[serde(rename = "gitlab")]
    GitLab,
    /// bitbucket.org or a Bitbucket Data Center instance.
    #[serde(rename = "bitbucket")]
    Bitbucket,
    /// Azure DevOps (`dev.azure.com`, or a legacy `*.visualstudio.com`).
    #[serde(rename = "azure_devops")]
    AzureDevOps,
    /// A git remote on a host the product has no special knowledge of.
    #[serde(rename = "generic_git")]
    GenericGit,
    /// No remote at all: identified by a fingerprint the client computed.
    #[serde(rename = "local")]
    Local,
}

impl RepositoryProvider {
    /// Every provider, in declaration order.
    pub const ALL: &'static [RepositoryProvider] = &[
        RepositoryProvider::GitHub,
        RepositoryProvider::GitLab,
        RepositoryProvider::Bitbucket,
        RepositoryProvider::AzureDevOps,
        RepositoryProvider::GenericGit,
        RepositoryProvider::Local,
    ];

    /// Stable wire name, identical to the serde form and to the stored value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            RepositoryProvider::GitHub => "github",
            RepositoryProvider::GitLab => "gitlab",
            RepositoryProvider::Bitbucket => "bitbucket",
            RepositoryProvider::AzureDevOps => "azure_devops",
            RepositoryProvider::GenericGit => "generic_git",
            RepositoryProvider::Local => "local",
        }
    }

    /// Whether this provider describes a repository with no remote.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self, RepositoryProvider::Local)
    }

    /// The provider a host implies, before any caller override.
    #[must_use]
    fn from_host(host: &str) -> RepositoryProvider {
        match host {
            "github.com" | "www.github.com" => RepositoryProvider::GitHub,
            "gitlab.com" | "www.gitlab.com" => RepositoryProvider::GitLab,
            "bitbucket.org" | "www.bitbucket.org" => RepositoryProvider::Bitbucket,
            "dev.azure.com" | "ssh.dev.azure.com" => RepositoryProvider::AzureDevOps,
            other if other.ends_with(".visualstudio.com") => RepositoryProvider::AzureDevOps,
            _ => RepositoryProvider::GenericGit,
        }
    }
}

impl fmt::Display for RepositoryProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RepositoryProvider {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "github" => Ok(RepositoryProvider::GitHub),
            "gitlab" => Ok(RepositoryProvider::GitLab),
            "bitbucket" => Ok(RepositoryProvider::Bitbucket),
            "azure_devops" => Ok(RepositoryProvider::AzureDevOps),
            "generic_git" => Ok(RepositoryProvider::GenericGit),
            "local" => Ok(RepositoryProvider::Local),
            other => Err(Error::Invalid {
                message: format!("unknown repository provider: {other:?}"),
            }),
        }
    }
}

/// One repository attached to a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRepository {
    /// The attachment's identity.
    pub id: RepositoryId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// The project this repository belongs to.
    pub project_id: ProjectId,
    /// Where it is hosted.
    pub provider: RepositoryProvider,
    /// **The identity.** Canonical, credential-free, and never a filesystem
    /// path — see the module docs.
    pub canonical_uri: String,
    /// The owning path on the host (`acme`, `acme/platform`), when the URI has
    /// one. `None` for a fingerprint-identified repository and for a remote
    /// whose path is a single segment.
    pub repository_owner: Option<String>,
    /// The repository's own name — the last path segment of the remote, or
    /// the name the caller gave a repository with no remote.
    pub repository_name: String,
    /// The branch a client should read when it is not told otherwise. Advisory
    /// metadata: nothing in the product resolves it.
    pub default_branch: Option<String>,
    /// The stable content fingerprint of a local checkout, when the client
    /// computed one. Identity only when there is no remote; a hint beside the
    /// remote otherwise.
    pub local_fingerprint: Option<String>,
    /// Open labelling bag, caller-supplied. Never an authorisation input, and
    /// never read to decide identity.
    pub metadata: serde_json::Value,
    /// The identity that attached it, when one did.
    pub created_by: Option<IdentityId>,
    /// When it was attached.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
}

/// What [`identify`] resolved: the fields that are *derived* rather than
/// supplied, so a caller cannot disagree with the canonicalisation and a
/// second client computing the same identity gets the same row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    /// The resolved provider.
    pub provider: RepositoryProvider,
    /// The canonical URI — the identity.
    pub canonical_uri: String,
    /// The owning path, when the remote had one.
    pub repository_owner: Option<String>,
    /// The repository's name.
    pub repository_name: String,
    /// The normalised fingerprint, when one was supplied.
    pub local_fingerprint: Option<String>,
}

/// Resolves the canonical identity of a repository from what a client knows
/// about it.
///
/// `remote_uri` wins whenever it is present — that is the whole rule. When it
/// is absent, `local_fingerprint` identifies the repository and `name` must
/// say what to call it. A path-shaped `remote_uri` is refused rather than
/// normalised: accepting one would make the identity of a project depend on
/// which machine last reported it.
///
/// # Errors
///
/// [`Error::Invalid`] when neither a remote nor a fingerprint is supplied;
/// when the remote is a filesystem path, has no host, has no path, uses a
/// transport this product does not recognise, or is over the length bound;
/// when the fingerprint is not 40–128 hex characters; when a `local` provider
/// is claimed for a repository that has a remote (or a remote provider for one
/// that has none); or when the resolved name or owner is malformed.
pub fn identify(
    remote_uri: Option<&str>,
    local_fingerprint: Option<&str>,
    name: Option<&str>,
    provider_override: Option<RepositoryProvider>,
) -> Result<RepositoryIdentity> {
    let remote = remote_uri.map(str::trim).filter(|uri| !uri.is_empty());
    let fingerprint = local_fingerprint
        .map(str::trim)
        .filter(|print| !print.is_empty())
        .map(normalise_fingerprint)
        .transpose()?;

    match remote {
        Some(remote) => {
            let parsed = parse_remote(remote)?;
            let provider = match provider_override {
                Some(RepositoryProvider::Local) => {
                    return Err(invalid(
                        "provider `local` describes a repository with no remote; \
                         drop `remote_uri` or choose a hosted provider"
                            .to_owned(),
                    ));
                }
                Some(explicit) => explicit,
                None => RepositoryProvider::from_host(&parsed.host),
            };
            let repository_name = match name.map(str::trim).filter(|name| !name.is_empty()) {
                Some(given) => given.to_owned(),
                None => parsed.name.clone(),
            };
            validate_name(&repository_name)?;
            if let Some(owner) = &parsed.owner {
                validate_owner(owner)?;
            }
            Ok(RepositoryIdentity {
                provider,
                canonical_uri: parsed.canonical,
                repository_owner: parsed.owner,
                repository_name,
                local_fingerprint: fingerprint,
            })
        }
        None => {
            let Some(fingerprint) = fingerprint else {
                return Err(invalid(
                    "a repository is identified by its canonical remote URI: supply \
                     `remote_uri`, or `local_fingerprint` when the repository has no \
                     remote. A filesystem path is never an identity."
                        .to_owned(),
                ));
            };
            if let Some(explicit) = provider_override
                && !explicit.is_local()
            {
                return Err(invalid(format!(
                    "provider `{explicit}` describes a hosted repository, but no \
                     `remote_uri` was supplied"
                )));
            }
            let Some(repository_name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
                return Err(invalid(
                    "a repository with no remote needs a `name`: there is no URI to take \
                     one from"
                        .to_owned(),
                ));
            };
            validate_name(repository_name)?;
            Ok(RepositoryIdentity {
                provider: RepositoryProvider::Local,
                canonical_uri: format!("{FINGERPRINT_SCHEME}:{fingerprint}"),
                repository_owner: None,
                repository_name: repository_name.to_owned(),
                local_fingerprint: Some(fingerprint),
            })
        }
    }
}

/// Checks a default branch name: non-blank, bounded, and free of the
/// characters git itself refuses in a ref.
///
/// # Errors
///
/// [`Error::Invalid`] when the branch is blank, too long, or malformed.
pub fn validate_branch(branch: Option<&str>) -> Result<()> {
    let Some(branch) = branch else {
        return Ok(());
    };
    if branch.trim().is_empty() {
        return Err(invalid(
            "a default branch cannot be blank; omit it instead".to_owned(),
        ));
    }
    let len = branch.chars().count();
    if len > MAX_BRANCH_CHARS {
        return Err(invalid(format!(
            "a default branch is at most {MAX_BRANCH_CHARS} characters, got {len}"
        )));
    }
    // git-check-ref-format's rules, in the subset a stored, displayed and
    // logged string needs: no whitespace, no control characters, none of the
    // pathspec metacharacters, and no `..` sequence.
    if branch.contains("..")
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with(".lock")
        || branch.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        return Err(invalid(format!("{branch:?} is not a usable branch name")));
    }
    Ok(())
}

/// Checks a repository metadata bag: a JSON object, at most
/// [`MAX_METADATA_BYTES`] encoded.
///
/// # Errors
///
/// [`Error::Invalid`] when the value is not an object or is over the bound.
pub fn validate_metadata(metadata: &serde_json::Value) -> Result<()> {
    if !metadata.is_object() {
        return Err(invalid(
            "repository metadata must be a JSON object".to_owned(),
        ));
    }
    let encoded = metadata.to_string().len();
    if encoded > MAX_METADATA_BYTES {
        return Err(invalid(format!(
            "repository metadata is at most {MAX_METADATA_BYTES} bytes encoded, got {encoded}"
        )));
    }
    Ok(())
}

fn invalid(message: String) -> Error {
    Error::Invalid { message }
}

/// A remote URI taken apart: everything the canonical form is built from.
struct ParsedRemote {
    host: String,
    owner: Option<String>,
    name: String,
    canonical: String,
}

/// The transports that name the same repository. All four canonicalise to
/// `https`, because how somebody clones a repository is not part of what it
/// is.
const TRANSPORTS: &[&str] = &["https", "http", "ssh", "git"];

fn parse_remote(remote: &str) -> Result<ParsedRemote> {
    if remote.chars().count() > MAX_CANONICAL_URI_CHARS {
        return Err(invalid(format!(
            "a repository URI is at most {MAX_CANONICAL_URI_CHARS} characters, got {}",
            remote.chars().count()
        )));
    }
    if remote.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid(format!(
            "{remote:?} is not a repository URI: it holds whitespace or control characters"
        )));
    }
    refuse_filesystem_path(remote)?;

    let (authority, path) = split_authority(remote)?;
    let host = normalise_host(&authority)?;
    let segments = path_segments(&path, remote)?;
    let (owner, name) = split_owner_and_name(&segments);
    let canonical = format!("https://{host}/{}", segments.join("/"));
    if canonical.chars().count() > MAX_CANONICAL_URI_CHARS {
        return Err(invalid(format!(
            "the canonical form of {remote:?} is over {MAX_CANONICAL_URI_CHARS} characters"
        )));
    }
    Ok(ParsedRemote {
        host,
        owner,
        name,
        canonical,
    })
}

/// Refuses the shapes a filesystem path takes, by name.
///
/// This is decision 4 enforced rather than documented. Each refusal says what
/// to send instead, because a caller holding a path genuinely does have
/// something better to send — `git remote get-url origin`, or the root-commit
/// id when there is no remote.
fn refuse_filesystem_path(remote: &str) -> Result<()> {
    let path_like = remote.starts_with('/')
        || remote.starts_with("./")
        || remote.starts_with("../")
        || remote.starts_with('~')
        || remote.starts_with('.')
        || remote.starts_with("\\\\")
        || remote.to_ascii_lowercase().starts_with("file:")
        // A Windows drive letter: `C:\src\repo` or `C:/src/repo`. The `:` also
        // starts scp-like syntax, so the discriminator is the single-character
        // authority, which is never a hostname.
        || windows_drive(remote);
    if path_like {
        return Err(invalid(format!(
            "{remote:?} is a filesystem path, and a path is never a repository identity: \
             it differs per machine and changes when somebody moves a directory. Send the \
             remote (`git remote get-url origin`), or `local_fingerprint` — a stable \
             content id such as the root commit — when there is no remote."
        )));
    }
    Ok(())
}

fn windows_drive(remote: &str) -> bool {
    let mut chars = remote.chars();
    let (Some(letter), Some(colon), Some(separator)) = (chars.next(), chars.next(), chars.next())
    else {
        return false;
    };
    letter.is_ascii_alphabetic() && colon == ':' && matches!(separator, '/' | '\\')
}

/// Splits `remote` into its authority and its path, accepting both URL syntax
/// (`ssh://git@host:22/acme/repo.git`) and git's scp-like shorthand
/// (`git@host:acme/repo.git`).
fn split_authority(remote: &str) -> Result<(String, String)> {
    if let Some((scheme, rest)) = remote.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !TRANSPORTS.contains(&scheme.as_str()) {
            return Err(invalid(format!(
                "{remote:?} uses the {scheme:?} transport; repository URIs use one of {}",
                TRANSPORTS.join(", ")
            )));
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        return Ok((authority.to_owned(), path.to_owned()));
    }
    if remote.contains("//") || remote.contains(':') {
        // scp-like: `[user@]host:path`. Everything before the first `:` is the
        // authority; a `//` outside a scheme is a malformed URL.
        if let Some((authority, path)) = remote.split_once(':')
            && !remote.contains("//")
        {
            return Ok((authority.to_owned(), path.to_owned()));
        }
        return Err(invalid(format!(
            "{remote:?} is not a repository URI: expected `https://host/owner/name` or \
             `git@host:owner/name`"
        )));
    }
    Err(invalid(format!(
        "{remote:?} is not a repository URI: it names no host. Expected \
         `https://host/owner/name` or `git@host:owner/name`."
    )))
}

/// Drops credentials and the port, and lower-cases what is left.
///
/// Dropping the credential is why a canonical URI is safe to store and return:
/// a caller that pasted `https://x-access-token:ghp_…@github.com/acme/repo`
/// has handed us a live token, and the row must not keep it.
///
/// **The port goes too, and that is a decision rather than an omission.** A
/// canonical URI is an identity, not a clone endpoint: the same repository is
/// reached on 443 over https and on 22 (or 2222) over ssh, and a form that
/// kept whichever number the caller happened to use would make one repository
/// two. What a client needs to *clone* is the URI it already has; what it
/// needs from us is the answer to "is this the same one you saw yesterday".
/// A deployment running two git servers on one host and path, distinguished
/// only by port, is the case this loses — it can keep the raw URI in
/// `metadata`, and no product surface reads it.
fn normalise_host(authority: &str) -> Result<String> {
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => host_port,
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return Err(invalid(format!(
            "{authority:?} names no host; a repository URI needs one"
        )));
    }
    // Names only. An IPv6 literal (`[::1]`) is refused rather than mangled:
    // the canonical form has one grammar, the database mirrors it as a CHECK,
    // and a bracketed authority is not it.
    if !host.starts_with(|c: char| c.is_ascii_alphanumeric())
        || host
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    {
        return Err(invalid(format!(
            "{host:?} is not a repository hostname; a name is expected"
        )));
    }
    Ok(host)
}

/// The path, split into segments with the `.git` suffix, empty segments and a
/// trailing slash removed.
fn path_segments(path: &str, remote: &str) -> Result<Vec<String>> {
    let trimmed = path.trim_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let segments: Vec<String> = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if segments.is_empty() {
        return Err(invalid(format!(
            "{remote:?} names a host but no repository path"
        )));
    }
    if segments
        .iter()
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(invalid(format!("{remote:?} holds a relative path segment")));
    }
    Ok(segments)
}

/// Everything but the last segment is the owner — so a GitLab subgroup path
/// survives intact rather than being flattened into a name nobody typed.
fn split_owner_and_name(segments: &[String]) -> (Option<String>, String) {
    let (name, owner) = segments
        .split_last()
        .expect("path_segments refuses an empty path");
    let owner = (!owner.is_empty()).then(|| owner.join("/"));
    (owner, name.clone())
}

fn normalise_fingerprint(fingerprint: &str) -> Result<String> {
    let lowered = fingerprint.to_ascii_lowercase();
    let len = lowered.chars().count();
    if !(MIN_FINGERPRINT_CHARS..=MAX_FINGERPRINT_CHARS).contains(&len)
        || !lowered.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid(format!(
            "a local fingerprint is {MIN_FINGERPRINT_CHARS}–{MAX_FINGERPRINT_CHARS} hex \
             characters — a stable content id such as a git root-commit object id, never \
             a path. Got {fingerprint:?}."
        )));
    }
    Ok(lowered)
}

fn validate_name(name: &str) -> Result<()> {
    let len = name.chars().count();
    if name.trim().is_empty() || len > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "a repository name is 1–{MAX_NAME_CHARS} characters, got {len}"
        )));
    }
    if name.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid(format!(
            "{name:?} is not a repository name: it holds whitespace or control characters"
        )));
    }
    Ok(())
}

fn validate_owner(owner: &str) -> Result<()> {
    let len = owner.chars().count();
    if len > MAX_OWNER_CHARS {
        return Err(invalid(format!(
            "a repository owner is at most {MAX_OWNER_CHARS} characters, got {len}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(uri: &str) -> RepositoryIdentity {
        identify(Some(uri), None, None, None).unwrap_or_else(|err| panic!("{uri}: {err}"))
    }

    /// The property the whole module exists for: four ways of writing one
    /// repository produce one identity.
    #[test]
    fn every_transport_for_one_repository_is_one_identity() {
        let canonical = "https://github.com/Acme/payments";
        for uri in [
            "https://github.com/Acme/payments",
            "https://github.com/Acme/payments.git",
            "https://github.com/Acme/payments/",
            "http://github.com/Acme/payments",
            "git://github.com/Acme/payments.git",
            "ssh://git@github.com/Acme/payments.git",
            "ssh://git@github.com:22/Acme/payments.git",
            "git@github.com:Acme/payments.git",
            "git@GitHub.com:Acme/payments",
            "https://github.com:443/Acme/payments.git",
        ] {
            assert_eq!(remote(uri).canonical_uri, canonical, "{uri}");
        }
    }

    /// The port is part of how you reach a repository, not of which one it
    /// is — so a non-default one unifies with the rest rather than minting a
    /// second identity.
    #[test]
    fn a_non_default_port_does_not_mint_a_second_repository() {
        assert_eq!(
            remote("ssh://git@git.acme.internal:2222/acme/payments.git").canonical_uri,
            remote("https://git.acme.internal/acme/payments").canonical_uri
        );
    }

    /// An IPv6 literal is refused rather than mangled: the canonical grammar
    /// is one shape, mirrored by a CHECK constraint, and a bracketed
    /// authority is not it.
    #[test]
    fn an_ipv6_literal_host_is_refused() {
        assert!(identify(Some("https://[::1]/acme/payments"), None, None, None).is_err());
    }

    /// A credential pasted into a remote never reaches the stored row.
    #[test]
    fn credentials_are_stripped_from_the_canonical_form() {
        let identity =
            remote("https://x-access-token:ghp_secretsecret@github.com/acme/payments.git");
        assert_eq!(identity.canonical_uri, "https://github.com/acme/payments");
        assert!(
            !identity.canonical_uri.contains("ghp_"),
            "a canonical URI must never carry a credential: {}",
            identity.canonical_uri
        );
    }

    #[test]
    fn owner_and_name_come_off_the_path() {
        let identity = remote("https://github.com/acme/payments.git");
        assert_eq!(identity.repository_owner.as_deref(), Some("acme"));
        assert_eq!(identity.repository_name, "payments");

        // A GitLab subgroup path survives whole rather than being flattened.
        let nested = remote("https://gitlab.com/acme/platform/payments.git");
        assert_eq!(nested.repository_owner.as_deref(), Some("acme/platform"));
        assert_eq!(nested.repository_name, "payments");
        assert_eq!(nested.provider, RepositoryProvider::GitLab);

        // A single-segment path has a name and no owner.
        let flat = remote("https://git.example.com/payments.git");
        assert_eq!(flat.repository_owner, None);
        assert_eq!(flat.repository_name, "payments");
        assert_eq!(flat.provider, RepositoryProvider::GenericGit);
    }

    #[test]
    fn providers_are_derived_from_the_host_and_overridable() {
        assert_eq!(
            remote("https://github.com/a/b").provider,
            RepositoryProvider::GitHub
        );
        assert_eq!(
            remote("https://bitbucket.org/a/b").provider,
            RepositoryProvider::Bitbucket
        );
        assert_eq!(
            remote("https://dev.azure.com/a/b").provider,
            RepositoryProvider::AzureDevOps
        );
        assert_eq!(
            remote("https://acme.visualstudio.com/a/b").provider,
            RepositoryProvider::AzureDevOps
        );
        assert_eq!(
            remote("https://git.acme.internal/a/b").provider,
            RepositoryProvider::GenericGit
        );
        // Self-hosted GitHub Enterprise: the host says nothing, the caller does.
        let enterprise = identify(
            Some("https://git.acme.internal/a/b"),
            None,
            None,
            Some(RepositoryProvider::GitHub),
        )
        .expect("override accepted");
        assert_eq!(enterprise.provider, RepositoryProvider::GitHub);
    }

    /// ADR-0071 decision 4, as an assertion: every path shape is refused, and
    /// the refusal says what to send instead.
    #[test]
    fn a_filesystem_path_is_never_an_identity() {
        for path in [
            "/Users/sam/src/payments",
            "./payments",
            "../payments",
            "~/src/payments",
            ".git",
            "file:///Users/sam/src/payments",
            "FILE:///Users/sam/src/payments",
            "C:\\src\\payments",
            "c:/src/payments",
            "\\\\server\\share\\payments",
        ] {
            let error = identify(Some(path), None, None, None)
                .expect_err(&format!("{path:?} should be refused"));
            let Error::Invalid { message } = error else {
                panic!("{path:?}: expected Invalid");
            };
            assert!(
                message.contains("local_fingerprint"),
                "{path:?}: the refusal must say what to send instead: {message}"
            );
        }
    }

    #[test]
    fn a_repository_with_no_remote_is_identified_by_its_fingerprint() {
        let oid = "9".repeat(40);
        let identity = identify(None, Some(&oid.to_uppercase()), Some("payments"), None)
            .expect("fingerprint accepted");
        assert_eq!(identity.provider, RepositoryProvider::Local);
        assert_eq!(identity.canonical_uri, format!("git+fingerprint:{oid}"));
        assert_eq!(identity.local_fingerprint.as_deref(), Some(oid.as_str()));
        assert_eq!(identity.repository_name, "payments");
        assert_eq!(identity.repository_owner, None);
    }

    #[test]
    fn a_fingerprint_must_be_a_content_id_and_not_a_path() {
        for bad in [
            "/Users/sam/src/payments",
            "payments",
            "deadbeef",
            &"z".repeat(40),
            &"a".repeat(MAX_FINGERPRINT_CHARS + 1),
        ] {
            assert!(
                identify(None, Some(bad), Some("payments"), None).is_err(),
                "{bad:?} should be refused as a fingerprint"
            );
        }
    }

    #[test]
    fn the_remote_wins_when_both_are_known() {
        let oid = "a".repeat(64);
        let identity = identify(
            Some("git@github.com:acme/payments.git"),
            Some(&oid),
            None,
            None,
        )
        .expect("both accepted");
        assert_eq!(identity.canonical_uri, "https://github.com/acme/payments");
        assert_eq!(identity.provider, RepositoryProvider::GitHub);
        assert_eq!(
            identity.local_fingerprint.as_deref(),
            Some(oid.as_str()),
            "the checkout is recorded beside the remote, never instead of it"
        );
    }

    #[test]
    fn knowing_nothing_is_refused_rather_than_guessed() {
        let error = identify(None, None, Some("payments"), None).expect_err("refused");
        let Error::Invalid { message } = error else {
            panic!("expected Invalid");
        };
        assert!(message.contains("remote_uri"), "{message}");
    }

    #[test]
    fn a_local_repository_must_be_named() {
        let oid = "b".repeat(40);
        let error = identify(None, Some(&oid), None, None).expect_err("refused");
        let Error::Invalid { message } = error else {
            panic!("expected Invalid");
        };
        assert!(message.contains("name"), "{message}");
    }

    #[test]
    fn provider_and_remote_must_agree() {
        let oid = "c".repeat(40);
        assert!(
            identify(
                Some("https://github.com/a/b"),
                None,
                None,
                Some(RepositoryProvider::Local)
            )
            .is_err(),
            "`local` with a remote is a contradiction"
        );
        assert!(
            identify(
                None,
                Some(&oid),
                Some("b"),
                Some(RepositoryProvider::GitHub)
            )
            .is_err(),
            "a hosted provider with no remote is a contradiction"
        );
    }

    #[test]
    fn malformed_remotes_are_refused() {
        for bad in [
            "",
            "github.com",
            "https://",
            "https:///acme/payments",
            "https://github.com",
            "https://github.com/",
            "ftp://github.com/a/b",
            "https://github.com/a/../b",
            "git@github.com:",
            "https://git hub.com/a/b",
            &format!("https://github.com/acme/{}", "x".repeat(600)),
        ] {
            assert!(
                identify(Some(bad), None, None, None).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn providers_round_trip_through_the_wire_name() {
        for provider in RepositoryProvider::ALL {
            assert_eq!(
                provider.as_str().parse::<RepositoryProvider>().unwrap(),
                *provider
            );
            let json = serde_json::to_string(provider).unwrap();
            assert_eq!(json, format!("\"{}\"", provider.as_str()));
        }
        assert!("gitea".parse::<RepositoryProvider>().is_err());
    }

    #[test]
    fn branches_are_optional_bounded_and_ref_shaped() {
        validate_branch(None).unwrap();
        validate_branch(Some("main")).unwrap();
        validate_branch(Some("release/2026-08")).unwrap();
        for bad in [
            "",
            "  ",
            "with space",
            "-leading",
            "/leading",
            "trailing/",
            "a..b",
            "wip.lock",
            "star*",
            "colon:",
        ] {
            assert!(validate_branch(Some(bad)).is_err(), "{bad:?}");
        }
        assert!(validate_branch(Some(&"x".repeat(MAX_BRANCH_CHARS + 1))).is_err());
    }

    #[test]
    fn metadata_is_a_bounded_object() {
        validate_metadata(&serde_json::json!({})).unwrap();
        validate_metadata(&serde_json::json!({"language": "rust"})).unwrap();
        assert!(validate_metadata(&serde_json::json!([])).is_err());
        assert!(validate_metadata(&serde_json::Value::Null).is_err());
        let oversized = serde_json::json!({"blob": "x".repeat(MAX_METADATA_BYTES)});
        assert!(validate_metadata(&oversized).is_err());
    }

    #[test]
    fn a_repository_round_trips_through_json() {
        let repository = ProjectRepository {
            id: RepositoryId::new(),
            tenant_id: TenantId::new(),
            project_id: ProjectId::new(),
            provider: RepositoryProvider::GitHub,
            canonical_uri: "https://github.com/acme/payments".to_owned(),
            repository_owner: Some("acme".to_owned()),
            repository_name: "payments".to_owned(),
            default_branch: Some("main".to_owned()),
            local_fingerprint: None,
            metadata: serde_json::json!({"language": "rust"}),
            created_by: Some(IdentityId::new()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&repository).unwrap();
        assert_eq!(
            serde_json::from_str::<ProjectRepository>(&json).unwrap(),
            repository
        );
    }
}

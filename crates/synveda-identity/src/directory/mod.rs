//! Reading a directory we do not own (AUTH-5, ADR-0060).
//!
//! AUTH-4 made this product a SCIM *server*: the directory authenticated to
//! us and stated facts as acts. This is the other direction — we authenticate
//! to the directory, we choose what to ask, and what comes back is a
//! snapshot. Nothing in a snapshot is an act, and everything in this module
//! exists to keep that distinction intact on the way to the reconciler.
//!
//! ## Why it lives in this crate
//!
//! The reconciler is the gateway's and takes `AppState`, so the *loop* is the
//! gateway's too (ADR-0060 decision 1). The connector needs none of that: it
//! needs an HTTP client, an issuer's configuration and a credential, all of
//! which are already here. Placing it beside [`crate::oidc`] also makes
//! ADR-0060 decision 8 **structural** rather than a promise. This crate is a
//! sibling of `synveda-store`, not a dependent, so a connector cannot name a
//! scope, a role, a pack or a record even if somebody wanted it to — the
//! types do not exist here. That is why [`DirectoryUserRecord`] mirrors the
//! mirror's shape rather than importing it.
//!
//! ## Completeness is a type, not a flag
//!
//! [`enumerate`](DirectoryConnector::enumerate) returns [`Enumeration`] and
//! **not** `Result<Enumeration>`, which is deliberate and is the one piece of
//! this module's shape worth defending. ADR-0060 decision 3.1 says an
//! incomplete pass still establishes *presence* — seeing somebody is not
//! conditional on seeing everybody — but may never conclude that anyone is
//! gone. A `Result` invites `?`, and `?` throws away the users we did see;
//! the second page failing would silently discard the first. So a failure is
//! [`Enumeration::Partial`], it carries everything read before the failure,
//! and the only way to reach a conclusion about absence is to match on
//! [`Enumeration::Complete`], which no failing path can construct.

use std::collections::HashSet;
use std::fmt;

use async_trait::async_trait;
use serde::Deserialize;
use url::{ParseError, Url};

pub mod entra;
pub mod okta;

/// Maximum requests one directory enumeration may make.
///
/// This and the other enumeration limits are resource-safety invariants, not
/// vendor support-size claims. They are deliberately well above an
/// individual or small team's directory while keeping a corrupt or hostile
/// continuation finite.
const MAX_ENUMERATION_PAGES: usize = 10_000;

/// Maximum decoded response bytes retained across one complete enumeration.
const MAX_ENUMERATION_BYTES: usize = 128 * 1024 * 1024;

/// Maximum user objects retained across one enumeration.
const MAX_ENUMERATION_USERS: usize = 100_000;

/// Maximum group objects retained across one enumeration.
const MAX_ENUMERATION_GROUPS: usize = 25_000;

/// Maximum membership objects retained across one enumeration.
const MAX_ENUMERATION_MEMBERS: usize = 1_000_000;

/// Maximum user, group and membership objects retained in total across one
/// enumeration.
const MAX_ENUMERATION_ITEMS: usize = 1_000_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct EnumerationLimits {
    pages: usize,
    bytes: usize,
    users: usize,
    groups: usize,
    members: usize,
    items: usize,
}

impl EnumerationLimits {
    pub(crate) const DEFAULT: Self = Self {
        pages: MAX_ENUMERATION_PAGES,
        bytes: MAX_ENUMERATION_BYTES,
        users: MAX_ENUMERATION_USERS,
        groups: MAX_ENUMERATION_GROUPS,
        members: MAX_ENUMERATION_MEMBERS,
        items: MAX_ENUMERATION_ITEMS,
    };
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DirectoryItemKind {
    Users,
    Groups,
    Members,
}

/// One mutable resource budget shared by every users/groups/member walk in a
/// pass. A fresh budget per collection would make the advertised bound a
/// multiple of the number of groups, which is exactly where membership walks
/// become unbounded.
pub(crate) struct EnumerationBudget {
    limits: EnumerationLimits,
    pages: usize,
    bytes: usize,
    users: usize,
    groups: usize,
    members: usize,
    items: usize,
    seen_urls: HashSet<String>,
}

impl EnumerationBudget {
    pub(crate) fn with_limits(limits: EnumerationLimits) -> Self {
        Self {
            limits,
            pages: 0,
            bytes: 0,
            users: 0,
            groups: 0,
            members: 0,
            items: 0,
            seen_urls: HashSet::new(),
        }
    }

    /// Accounts for a page before its request is built. In particular, the
    /// caller must do this before attaching a bearer or API token.
    pub(crate) fn begin_page(&mut self, url: &Url) -> Result<(), String> {
        let key = url.as_str().to_owned();
        if self.seen_urls.contains(&key) {
            return Err("directory pagination repeated a URL (cycle refused)".to_owned());
        }
        if self.pages >= self.limits.pages {
            return Err(format!(
                "directory enumeration exceeded its {}-page bound",
                self.limits.pages
            ));
        }
        self.seen_urls.insert(key);
        self.pages += 1;
        Ok(())
    }

    fn add_bytes(&mut self, count: usize) -> Result<(), String> {
        let next = self
            .bytes
            .checked_add(count)
            .ok_or_else(|| "directory response byte count overflowed".to_owned())?;
        if next > self.limits.bytes {
            return Err(format!(
                "directory enumeration exceeded its {}-byte response bound",
                self.limits.bytes
            ));
        }
        self.bytes = next;
        Ok(())
    }

    /// Accounts for decoded objects before retaining the page that carries
    /// them. If the page crosses a bound, earlier pages remain available and
    /// this page is not partially appended.
    pub(crate) fn retain(&mut self, kind: DirectoryItemKind, count: usize) -> Result<(), String> {
        let (current, limit, label) = match kind {
            DirectoryItemKind::Users => (&mut self.users, self.limits.users, "user"),
            DirectoryItemKind::Groups => (&mut self.groups, self.limits.groups, "group"),
            DirectoryItemKind::Members => (&mut self.members, self.limits.members, "membership"),
        };
        let next = current
            .checked_add(count)
            .ok_or_else(|| format!("directory {label} count overflowed"))?;
        if next > limit {
            return Err(format!(
                "directory enumeration exceeded its {limit}-{label} bound"
            ));
        }
        let next_items = self
            .items
            .checked_add(count)
            .ok_or_else(|| "directory item count overflowed".to_owned())?;
        if next_items > self.limits.items {
            return Err(format!(
                "directory enumeration exceeded its {}-item bound",
                self.limits.items
            ));
        }
        *current = next;
        self.items = next_items;
        Ok(())
    }
}

/// The configured origin to which a connector may present credentials.
/// Continuations resolve relative to the current page, but must remain on
/// this exact scheme, host and effective port.
pub(crate) struct ApiOrigin {
    configured: Url,
}

impl ApiOrigin {
    pub(crate) fn parse(raw: &str, label: &str) -> synveda_types::Result<Self> {
        let configured = Url::parse(raw).map_err(|_| synveda_types::Error::Invalid {
            message: format!("{label} must be an absolute HTTP(S) URL"),
        })?;
        if !matches!(configured.scheme(), "http" | "https")
            || configured.host_str().is_none()
            || !configured.username().is_empty()
            || configured.password().is_some()
            || configured.fragment().is_some()
        {
            return Err(synveda_types::Error::Invalid {
                message: format!(
                    "{label} must be an HTTP(S) origin without userinfo or a fragment"
                ),
            });
        }
        Ok(Self { configured })
    }

    pub(crate) fn resolve(
        &self,
        current_page: Option<&Url>,
        continuation: &str,
    ) -> Result<Url, String> {
        let url = match Url::parse(continuation) {
            Ok(url) => url,
            Err(ParseError::RelativeUrlWithoutBase) => current_page
                .unwrap_or(&self.configured)
                .join(continuation)
                .map_err(|_| "directory continuation is not a valid URL".to_owned())?,
            Err(_) => return Err("directory continuation is not a valid URL".to_owned()),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err("directory continuation contains forbidden userinfo".to_owned());
        }
        if url.fragment().is_some() {
            return Err("directory continuation contains a forbidden fragment".to_owned());
        }
        if url.scheme() != self.configured.scheme()
            || url.host_str() != self.configured.host_str()
            || url.port_or_known_default() != self.configured.port_or_known_default()
        {
            return Err("directory continuation changes the configured API origin".to_owned());
        }
        Ok(url)
    }
}

/// Incrementally reads and deserializes one response under the pass-wide byte
/// budget. Errors report classification and location only; response content
/// is never copied into an error.
pub(crate) async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
    budget: &mut EnumerationBudget,
    url: &Url,
) -> Result<T, String> {
    let mut body = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|err| format!("reading {url}: {err}"))?;
        let Some(chunk) = chunk else {
            break;
        };
        budget.add_bytes(chunk.len())?;
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|err| {
        format!(
            "decoding {url}: invalid JSON at line {}, column {}",
            err.line(),
            err.column()
        )
    })
}

/// One person as the directory describes them.
///
/// Deliberately not `synveda_store::directory::UserAttributes`: this crate is
/// that one's sibling under the dependency rule (seed §8), so the projection
/// onto product state is the gateway's to make and the vocabulary here is
/// directory facts and nothing else (ADR-0060 decision 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryUserRecord {
    /// The vendor's own stable id, which becomes `externalId` on the mirror
    /// and is the anchor the correspondence rule matches on first.
    pub external_id: String,
    /// `userName` — a UPN for Entra, a login for Okta.
    pub user_name: String,
    /// Whether the directory considers them active. `false` is an act and
    /// seals on the first complete pass that sees it; being *absent* from
    /// the enumeration is not this field and never becomes it here.
    pub active: bool,
    /// `displayName`, when the directory has one.
    pub display_name: Option<String>,
    /// `name.givenName`.
    pub given_name: Option<String>,
    /// `name.familyName`.
    pub family_name: Option<String>,
    /// The work address, which the correspondence rule prefers over
    /// `userName` (ADR-0059 decision 4, as the AUTH-4 demo corrected it).
    pub work_email: Option<String>,
}

/// One group and its complete membership as the directory describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryGroupRecord {
    /// Stable vendor resource id (Entra object id or Okta group id).
    pub external_id: String,
    /// Human-facing directory name.
    pub display_name: String,
    /// Stable vendor user ids belonging to the group.
    pub member_external_ids: Vec<String>,
}

/// Everything one pass read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirectorySnapshot {
    /// Every user the enumeration reached, in the order the directory
    /// listed them.
    pub users: Vec<DirectoryUserRecord>,
    /// Every group reached by the enumeration, including empty groups.
    pub groups: Vec<DirectoryGroupRecord>,
}

/// What an enumeration produced, and whether it may be trusted about who is
/// missing.
///
/// The two variants carry the same data and differ only in what the caller is
/// entitled to conclude from it, which is the whole of ADR-0060 decision 3.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enumeration {
    /// Every page, no error. The only variant from which absence means
    /// anything.
    Complete(DirectorySnapshot),
    /// Something failed part-way through. What was read is still true —
    /// those people exist and are present — and what was not read is
    /// evidence of nothing at all.
    Partial {
        /// Everything gathered before the failure, which may be empty.
        snapshot: DirectorySnapshot,
        /// Why it stopped, for a log and a metric. Never carries the
        /// credential; see [`redact`].
        failure: String,
    },
}

impl Enumeration {
    /// The users read, however the pass ended. Presence survives failure.
    #[must_use]
    pub fn snapshot(&self) -> &DirectorySnapshot {
        match self {
            Enumeration::Complete(snapshot) | Enumeration::Partial { snapshot, .. } => snapshot,
        }
    }

    /// Whether this pass may support a conclusion about absence.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self, Enumeration::Complete(_))
    }
}

/// A directory this product can read.
///
/// One method, because a pass is one act. Pagination, token acquisition and
/// the vendor's own shapes are each implementation's business; what crosses
/// this boundary is people and the groups they are in.
#[async_trait]
pub trait DirectoryConnector: Send + Sync {
    /// The connector's name, stored on `directory_sync_state.connector` so a
    /// deployment that re-points a tenant invalidates its absence counts.
    fn name(&self) -> &'static str;

    /// Reads the whole directory.
    ///
    /// **Full enumeration, never a delta feed** (ADR-0060 decision 6): a
    /// change feed states what changed and never what still exists, so it
    /// cannot carry the completeness proof absence is built from.
    async fn enumerate(&self) -> Enumeration;
}

/// The client every connector uses.
///
/// [`crate::oidc`]'s settings, for its reasons: bounded connect and request
/// timeouts so a hung vendor cannot hold a pass open indefinitely, and
/// **no redirect following**, because a credential must go to the host a
/// deployment configured and not to wherever that host points next.
///
/// # Errors
/// If the client cannot be constructed.
pub(crate) fn http_client() -> synveda_types::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| synveda_types::Error::Internal {
            message: format!("building the directory HTTP client: {err}"),
        })
}

/// Builds the connector a configuration names.
///
/// # Errors
/// If the HTTP client cannot be constructed.
pub fn connector(
    config: &DirectorySyncConfig,
) -> synveda_types::Result<Box<dyn DirectoryConnector>> {
    match config {
        DirectorySyncConfig::Entra {
            tenant_id,
            client_id,
            client_secret,
            graph_base,
            login_base,
        } => Ok(Box::new(entra::EntraConnector::new(
            tenant_id.clone(),
            client_id.clone(),
            client_secret.clone(),
            graph_base.clone(),
            login_base.clone(),
        )?)),
        DirectorySyncConfig::Okta { org_url, api_token } => Ok(Box::new(okta::OktaConnector::new(
            org_url.clone(),
            api_token.clone(),
        )?)),
    }
}

/// Removes a secret from anything about to be logged, stored or returned.
///
/// Called on every failure string this module produces. A connector error
/// normally carries a URL and a status, neither of which holds a credential —
/// Entra's client secret rides in a POST body and Okta's token in a header,
/// and `reqwest` renders neither. This exists because "normally" is not a
/// property, and because ADR-0060 decision 7 makes the outbound credential
/// the first secret in this product that has to be recoverable: it is the one
/// thing here that must never reach a log, a span, an error or the chain.
fn redact(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        return message.to_owned();
    }
    message.replace(secret, "«redacted»")
}

/// A credential that does not print itself.
///
/// `Debug` is derived all over this workspace and configuration ends up in
/// logs by accident rather than by decision, so the secret is wrapped in a
/// type whose `Debug` cannot leak it.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Wraps a secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, at the one place that has to present it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(«redacted»)")
    }
}

/// The stable operator label of a tenant's directory secret aggregate
/// (CPR-35, ADR-0094). Its UUID, rather than this label, binds the envelope
/// and is the immutable reference identity.
pub const CREDENTIAL_SECRET_NAME: &str = "directory.credential";

/// How a deployment configures the pull sync for one issuer.
///
/// **Two sources since TEN-4, and the per-tenant one wins.** A tenant that has
/// never created a `directory.credential` aggregate may fall back to this,
/// configured beside the issuer in `SYNVEDA_OIDC_ISSUERS`. A revoked or
/// unusable stable aggregate fails closed and does not fall back (ADR-0094).
///
/// ADR-0060 decision 7 put the credential here alone and named the cost: one
/// deployment could not pull two tenants from two directories, because one
/// environment configures one connector per issuer and an issuer binds one
/// tenant. The stored form removes that — each tenant's row carries a whole
/// configuration of its own — and its reasoning for waiting was exactly this
/// feature: "a per-tenant table holding a secret we can read back … wants
/// TEN-4's per-tenant encryption keys".
///
/// The environment form is retained rather than migrated because it is the
/// right shape for the single-tenant deployment that is most of the installed
/// base, and because a deployment that has one working directory should not
/// have to do anything on upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "connector", rename_all = "lowercase", deny_unknown_fields)]
pub enum DirectorySyncConfig {
    /// Microsoft Entra ID through Microsoft Graph.
    Entra {
        /// The Entra tenant the token is minted against.
        tenant_id: String,
        /// The application registered for this integration.
        client_id: String,
        /// Its secret. Never printed, never logged.
        client_secret: Secret,
        /// Graph's base URL. Overridable so tests can point at a local
        /// mock; deployments leave it unset.
        #[serde(default)]
        graph_base: Option<String>,
        /// The login host that mints the token. Overridable for the same
        /// reason.
        #[serde(default)]
        login_base: Option<String>,
    },
    /// Okta through its Users and Groups APIs.
    Okta {
        /// The org's base URL, e.g. `https://example.okta.com`.
        org_url: String,
        /// An SSWS API token. Never printed, never logged.
        api_token: Secret,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_does_not_print_itself() {
        // The failure this prevents is a configuration struct reaching a log
        // line through a derived `Debug` on something that contains it —
        // which is how a credential leaks without anybody writing a line
        // that leaks a credential.
        let config = DirectorySyncConfig::Okta {
            org_url: "https://example.okta.com".to_owned(),
            api_token: Secret::new("00SUPERSECRETTOKEN"),
        };
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("00SUPERSECRETTOKEN"),
            "a config's Debug must not carry the credential: {rendered}"
        );
        assert!(rendered.contains("«redacted»"));
        // And the one caller that needs it still gets it.
        let DirectorySyncConfig::Okta { api_token, .. } = &config else {
            unreachable!()
        };
        assert_eq!(api_token.expose(), "00SUPERSECRETTOKEN");
    }

    #[test]
    fn redaction_removes_a_secret_wherever_it_appears() {
        let message = "POST https://login.example/token failed: shhh in body";
        assert_eq!(
            redact(message, "shhh"),
            "POST https://login.example/token failed: «redacted» in body"
        );
        // An empty secret must not turn every message into redaction marks.
        assert_eq!(redact(message, ""), message);
    }

    #[test]
    fn a_partial_enumeration_keeps_what_it_read() {
        // The property the type exists for: presence survives an incomplete
        // pass, and only `Complete` can support a claim about absence.
        let snapshot = DirectorySnapshot {
            users: vec![DirectoryUserRecord {
                external_id: "1".to_owned(),
                user_name: "a@example.test".to_owned(),
                active: true,
                display_name: None,
                given_name: None,
                family_name: None,
                work_email: None,
            }],
            groups: Vec::new(),
        };
        let partial = Enumeration::Partial {
            snapshot: snapshot.clone(),
            failure: "429 from the second page".to_owned(),
        };
        assert!(!partial.is_complete());
        assert_eq!(partial.snapshot().users.len(), 1);
        assert!(Enumeration::Complete(snapshot).is_complete());
    }

    #[test]
    fn continuations_are_relative_or_same_origin_with_effective_ports() {
        let origin =
            ApiOrigin::parse("https://directory.example.test", "test origin").expect("origin");
        let current = Url::parse("https://directory.example.test/v1/users?page=1").unwrap();
        assert_eq!(
            origin
                .resolve(Some(&current), "?page=2")
                .expect("relative query")
                .as_str(),
            "https://directory.example.test/v1/users?page=2"
        );
        assert!(
            origin
                .resolve(
                    Some(&current),
                    "https://directory.example.test:443/v1/users?page=3"
                )
                .is_ok(),
            "an explicit default port is the same effective origin"
        );
        for refused in [
            "http://directory.example.test/v1/users",
            "https://directory.example.test:444/v1/users",
            "https://other.example.test/v1/users",
            "https://user@directory.example.test/v1/users",
            "/v1/users#fragment",
        ] {
            assert!(
                origin.resolve(Some(&current), refused).is_err(),
                "accepted forbidden continuation {refused:?}"
            );
        }
    }

    #[test]
    fn every_retained_collection_has_its_own_pass_wide_bound() {
        for (kind, expected) in [
            (DirectoryItemKind::Users, "user"),
            (DirectoryItemKind::Groups, "group"),
            (DirectoryItemKind::Members, "membership"),
        ] {
            let mut budget = EnumerationBudget::with_limits(EnumerationLimits {
                users: 1,
                groups: 1,
                members: 1,
                items: 10,
                ..EnumerationLimits::DEFAULT
            });
            budget.retain(kind, 1).expect("at the boundary");
            let failure = budget.retain(kind, 1).expect_err("over the boundary");
            assert!(failure.contains(expected), "got {failure:?}");
        }
    }
}

//! OIDC bearer-token verification (AUTH-1, ADR-0010).
//!
//! [`OidcVerifier`] verifies IdP-issued JWTs against per-issuer trust
//! entries. Discovery and JWKS are fetched lazily and cached; keys are
//! replaced wholesale on refresh; a token with an unknown `kid` triggers a
//! refetch rate-limited to one per issuer per [`REFRESH_MIN_INTERVAL`] —
//! that is the rotation-handling contract: rotation heals on the next
//! request without letting an attacker drive fetch load.
//!
//! Fail-closed throughout (seed §2.3): unknown issuer, unknown key,
//! algorithm outside the per-issuer allowlist, or any claim mismatch is the
//! uniform [`Error::Unauthenticated`]. The *unverified* `iss` claim is read
//! only to select a trust entry; every other claim is consulted only after
//! signature verification under that entry's keys.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::{
    AlgorithmParameters, Jwk, JwkSet, KeyAlgorithm, KeyOperations, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use rsa::traits::PublicKeyParts as _;
use serde::Deserialize;
use synveda_types::{Error, Result, TenantId};

use crate::token::{Claims, CredentialClass, ProvisioningClaims, TokenVerifier, issued_lifetime};

/// Token verifications by issuer and outcome (`ok`, `rejected`, `error`).
/// Emitted here, described by the gateway's recorder (ADR-0007 layering).
pub const TOKEN_VERIFICATIONS_TOTAL: &str = "synveda_token_verifications_total";

/// JWKS refreshes by issuer and outcome (`ok`, `error`).
pub const JWKS_REFRESHES_TOTAL: &str = "synveda_jwks_refreshes_total";

/// Explicit issuer diagnostics by issuer and outcome (`ok`, `error`).
pub const OIDC_DIAGNOSTICS_TOTAL: &str = "synveda_oidc_diagnostics_total";

/// Minimum interval between JWKS refetches per issuer (ADR-0010 §3).
const REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// Clock skew tolerated on `exp` and `nbf`.
const LEEWAY: Duration = Duration::from_secs(30);

/// Discovery and JWKS are configuration metadata, not an unbounded content
/// channel. One MiB accommodates ordinary multi-key JWKS documents while
/// keeping a malicious provider from making startup allocate without limit.
const MAX_METADATA_BYTES: usize = 1_048_576;

/// A deployment may trust several issuers, but the list is configuration,
/// not an unbounded tenant-controlled registry.
const MAX_ISSUERS: usize = 16;

/// Maximum number of configured OAuth scope tokens for one issuer.
const MAX_LOGIN_SCOPES: usize = 32;

/// Maximum length of one OAuth scope token in bytes.
const MAX_LOGIN_SCOPE_BYTES: usize = 256;

/// Provider metadata is untrusted and must not expand into unbounded scope
/// comparison work.
const MAX_ADVERTISED_SCOPES: usize = 256;

/// Each protocol-vocabulary array in discovery is bounded before any scan or
/// set construction. The wire document's byte bound alone is insufficient:
/// it can still encode hundreds of thousands of empty strings and monopolise
/// the startup task before its async deadline can run.
const MAX_DISCOVERY_VALUES: usize = 64;

/// One discovery vocabulary entry is a compact protocol token or response
/// type, never an open content channel.
const MAX_DISCOVERY_VALUE_BYTES: usize = 256;

/// Additional bearer audiences are deployment configuration, not a dynamic
/// client registry.
const MAX_SERVICE_AUDIENCES: usize = 64;

/// A bounded JWKS prevents a small metadata document containing thousands of
/// tiny keys from turning startup or rotation into unbounded verification
/// work.
const MAX_JWKS_KEYS: usize = 128;

/// Gateway and deployment one-shot use the same closed startup policy. The
/// HTTP client has tighter per-request bounds; this caps the complete
/// multi-issuer retry window.
const INITIAL_DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(45);
const INITIAL_DIAGNOSTIC_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Whether an explicit deployment diagnostic failure can heal without an
/// operator changing configuration or provider policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcDiagnosticDisposition {
    /// Transport interruption or a provider status explicitly intended for
    /// retry may heal inside the bounded deployment window.
    Retryable,
    /// Configuration, metadata, key or protocol contract drift must be
    /// refused immediately.
    Refused,
}

/// Content-free OIDC diagnostic failure.
///
/// Provider responses, URLs, client identifiers and keys never enter this
/// value. The one-shot process can therefore report it without turning
/// deployment logs into a metadata or secret channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcDiagnosticError {
    stage: &'static str,
    disposition: OidcDiagnosticDisposition,
}

impl OidcDiagnosticError {
    fn retryable(stage: &'static str) -> Self {
        Self {
            stage,
            disposition: OidcDiagnosticDisposition::Retryable,
        }
    }

    fn refused(stage: &'static str) -> Self {
        Self {
            stage,
            disposition: OidcDiagnosticDisposition::Refused,
        }
    }

    /// Whether a fresh verifier may retry this failure inside the total
    /// deployment deadline.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.disposition == OidcDiagnosticDisposition::Retryable
    }

    /// Closed failure disposition for metrics and process exit policy.
    #[must_use]
    pub fn disposition(&self) -> OidcDiagnosticDisposition {
        self.disposition
    }

    /// Closed, static failure stage suitable for operator logs. Provider
    /// response bodies, URLs, identifiers and credentials can never enter it.
    #[must_use]
    pub fn stage(&self) -> &'static str {
        self.stage
    }
}

impl fmt::Display for OidcDiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = match self.disposition {
            OidcDiagnosticDisposition::Retryable => "is temporarily unavailable",
            OidcDiagnosticDisposition::Refused => "was refused",
        };
        write!(formatter, "OIDC {} diagnostic {outcome}", self.stage)
    }
}

impl std::error::Error for OidcDiagnosticError {}

/// The signature algorithms this build can verify. The bundled provider and
/// supported external-provider configurations use the RSA family; the
/// RustCrypto `rsa` backend is the only one compiled in (deny.toml).
const SUPPORTED_ALGORITHMS: [Algorithm; 3] = [Algorithm::RS256, Algorithm::RS384, Algorithm::RS512];

/// How a verified token binds to a Synveda tenant (ADR-0010 §4). Both modes
/// end at TEN-1's unchanged active-tenant lookup in the gateway.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TenantBinding {
    /// A named claim carries the tenant UUID (`tid` — Entra's native shape
    /// and the ADR-0008 internal convention).
    Claim {
        /// The claim name holding the tenant UUID.
        name: String,
    },
    /// Every subject from this issuer belongs to one configured tenant —
    /// the natural shape for a single-organisation provider.
    Static {
        /// The tenant every login from this issuer resolves to.
        tenant_id: TenantId,
    },
}

fn default_tenant_binding() -> TenantBinding {
    TenantBinding::Claim {
        name: "tid".to_owned(),
    }
}

fn default_algorithms() -> Vec<Algorithm> {
    vec![Algorithm::RS256]
}

fn default_groups_claim() -> String {
    "groups".to_owned()
}

/// `sub` — right for every issuer whose subject is stable across
/// applications, and wrong for Entra, which is why it is configurable
/// (AUTH-4, ADR-0059 decision 4).
fn default_external_id_claim() -> String {
    "sub".to_owned()
}

fn default_login_scopes() -> Vec<String> {
    // `openid` is what makes it OIDC; profile and email feed AUTH-2's
    // provisioning claims. Providers that gate the groups claim behind a
    // scope add "groups" in config; providers that reject unknown scopes keep
    // the default (ADR-0013 decision 1).
    vec![
        "openid".to_owned(),
        "profile".to_owned(),
        "email".to_owned(),
    ]
}

/// One issuer the gateway trusts (ADR-0010 §2). Deserialized from the
/// `SYNVEDA_OIDC_ISSUERS` JSON array; unknown fields are config typos and
/// rejected.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssuerConfig {
    /// Issuer URL, compared byte-for-byte with the discovery document and
    /// the `iss` claim.
    pub issuer: String,
    /// OAuth2 client id registered at the IdP; also the ID-token audience.
    pub client_id: String,
    /// Expected bearer/API-token audience. This must be distinct from
    /// `client_id`, which is reserved for ID tokens returned by the login
    /// flow. Requiring separate audiences prevents an ID token from being
    /// replayed as an API bearer token.
    pub audience: String,
    /// Allowed signature algorithms. Defaults to `["RS256"]`.
    #[serde(default = "default_algorithms")]
    pub algorithms: Vec<Algorithm>,
    /// How tokens from this issuer bind to a tenant. Defaults to the `tid`
    /// claim.
    #[serde(default = "default_tenant_binding")]
    pub tenant: TenantBinding,
    /// The claim carrying group names for JIT provisioning (AUTH-2,
    /// ADR-0013). Defaults to `groups`.
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    /// How this issuer's directory is **read**, when it cannot push
    /// (AUTH-5, ADR-0060 decision 7).
    ///
    /// Beside the issuer rather than in a per-tenant table because the
    /// outbound credential is the first secret in this product that has to
    /// be recoverable, and a table holding one would want TEN-4's keys —
    /// which do not exist yet. The issuer whose tokens this entry already
    /// verifies is the issuer this connector reads, so it adds no new
    /// configuration surface and inherits the deployment-level scope the
    /// issuer list already has.
    ///
    /// Only meaningful with [`TenantBinding::Static`]: a pull sync runs on
    /// a timer with no request in front of it, so there is no token whose
    /// claim could say which tenant it is for. The core worker refuses to
    /// start work on the other combination rather than syncing the wrong tenant or
    /// silently syncing none.
    #[serde(default)]
    pub directory_sync: Option<crate::directory::DirectorySyncReferenceConfig>,
    /// The claim carrying this issuer's stable directory anchor (AUTH-4,
    /// ADR-0059 decision 4) — matched against a SCIM mirror row's
    /// `externalId` at first login. Defaults to `sub`.
    ///
    /// **Set this to `oid` for Entra.** Entra's `sub` is pairwise per
    /// application and never equals the object id its provisioning agent
    /// sends as `externalId`, so the default would match nothing there and
    /// the login would fall through to the weaker email match — or, for
    /// somebody whose address the directory never sent, to a second
    /// identity.
    #[serde(default = "default_external_id_claim")]
    pub external_id_claim: String,
    /// Scopes requested at login. Defaults to `openid profile email`;
    /// must include `openid` (no ID token without it).
    #[serde(default = "default_login_scopes")]
    pub login_scopes: Vec<String>,
    /// Additional audiences accepted on *bearer* tokens (never ID tokens):
    /// client-credentials access tokens carry the service client's own
    /// audience, not the login client's (AUTH-3, ADR-0018 decision 1).
    /// Typically the registered service clients' ids, or one shared API
    /// audience (Entra's `api://...` shape). Defaults to empty.
    #[serde(default)]
    pub service_audiences: Vec<String>,
}

impl IssuerConfig {
    /// Every audience a bearer token may carry: the primary bearer
    /// audience plus the service audiences (ADR-0018 decision 1).
    fn bearer_audiences(&self) -> Vec<&str> {
        let mut audiences = vec![self.audience.as_str()];
        audiences.extend(self.service_audiences.iter().map(String::as_str));
        audiences
    }
}

/// Parses the `SYNVEDA_OIDC_ISSUERS` JSON array.
pub fn parse_issuers(json: &str) -> Result<Vec<IssuerConfig>> {
    serde_json::from_str(json).map_err(|err| Error::Invalid {
        message: format!("SYNVEDA_OIDC_ISSUERS is not a valid issuer list: {err}"),
    })
}

/// The subset of the discovery document Synveda uses.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoveryDocument {
    issuer: String,
    pub(crate) authorization_endpoint: String,
    pub(crate) token_endpoint: String,
    jwks_uri: String,
    /// RFC 8414 / RFC 7636 metadata. The field is optional in the wire
    /// format. Omission makes no claim about support; when present, the
    /// provider must advertise the S256 method Synveda always sends.
    #[serde(default, deserialize_with = "deserialize_present_vec")]
    code_challenge_methods_supported: Option<Vec<String>>,
    /// OIDC discovery signing-algorithm metadata. A provider passes the
    /// diagnostic only when this intersects both the configured allowlist
    /// and an actually usable JWKS key.
    #[serde(default)]
    id_token_signing_alg_values_supported: Vec<String>,
    /// OIDC providers used for interactive login must advertise the
    /// authorization-code response type.
    #[serde(default)]
    response_types_supported: Vec<String>,
    /// OAuth discovery makes this optional. When present, it must include
    /// the authorization-code grant Synveda uses.
    #[serde(default)]
    grant_types_supported: Option<Vec<String>>,
    /// RFC 8414's optional `scopes_supported`. Absent means the issuer
    /// published no list, which is not the same as publishing an empty
    /// one: both read as "do not ask for scopes it never advertised"
    /// (ADR-0027 decision 6).
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
}

/// Preserves the distinction between absent optional metadata and a present
/// value. Serde normally maps both a missing field and explicit `null` to
/// `None`; discovery `null` is an ill-shaped claim and must be refused.
fn deserialize_present_vec<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<String>::deserialize(deserializer).map(Some)
}

/// A cached verification key: the decoded key plus the algorithms it may
/// verify (the per-issuer allowlist narrowed by the JWK's own `kty`/`alg`).
struct VerifyingKey {
    key: DecodingKey,
    algorithms: Vec<Algorithm>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetadataEndpointTransport {
    HttpsOnly,
    LoopbackHttpOnly,
    InsecureDevelopmentHttp,
}

/// Immutable snapshot swapped wholesale on refresh; readers clone the `Arc`
/// under a short read lock and verify without further synchronisation.
pub(crate) struct IssuerState {
    pub(crate) discovery: DiscoveryDocument,
    keys: HashMap<String, VerifyingKey>,
    id_token_algorithms: Vec<Algorithm>,
}

impl IssuerState {
    /// Whether the issuer's discovery document advertises `scope`. The
    /// login flow asks this before requesting `offline_access` (ADR-0027
    /// decision 6).
    pub(crate) fn advertises_scope(&self, scope: &str) -> bool {
        self.discovery
            .scopes_supported
            .as_ref()
            .is_some_and(|supported| supported.iter().any(|entry| entry == scope))
    }
}

struct IssuerEntry {
    config: IssuerConfig,
    /// Derived from the parsed, normalized issuer and the explicit
    /// development relaxation. A loopback issuer may advertise only
    /// loopback plaintext endpoints; an HTTPS issuer never downgrades.
    metadata_endpoint_transport: MetadataEndpointTransport,
    state: RwLock<Option<Arc<IssuerState>>>,
    /// Serialises refreshes and carries the rate-limit clock. `tokio::sync`
    /// because it is held across the discovery/JWKS fetches.
    refresh: tokio::sync::Mutex<Option<Instant>>,
}

/// Multi-issuer OIDC verifier (ADR-0010). The gateway installs it as the
/// [`TokenVerifier`] whenever `SYNVEDA_OIDC_ISSUERS` is configured.
pub struct OidcVerifier {
    http: reqwest::Client,
    issuers: HashMap<String, IssuerEntry>,
    refresh_min_interval: Duration,
}

impl OidcVerifier {
    /// Builds a verifier over the configured trust entries. Rejects empty
    /// and duplicate issuer lists and algorithms this build cannot verify.
    pub fn new(configs: Vec<IssuerConfig>) -> Result<Self> {
        Self::new_with_insecure_development_http(configs, false)
    }

    /// Builds a verifier with the one explicit plaintext relaxation used by
    /// an explicitly labelled development deployment. Callers that own a
    /// deployment setting pass its closed boolean here; `false` is identical
    /// to [`Self::new`] and does not weaken issuer, redirect, signature,
    /// audience or token validation.
    pub fn new_with_insecure_development_http(
        configs: Vec<IssuerConfig>,
        insecure_development_http: bool,
    ) -> Result<Self> {
        if configs.is_empty() {
            return Err(Error::Invalid {
                message: "at least one OIDC issuer must be configured".to_owned(),
            });
        }
        if configs.len() > MAX_ISSUERS {
            return Err(Error::Invalid {
                message: format!("at most {MAX_ISSUERS} OIDC issuers may be configured"),
            });
        }
        let mut issuers = HashMap::new();
        for config in configs {
            let issuer_url = validate_issuer_identifier(&config.issuer)?;
            if issuer_url.scheme() == "http"
                && !insecure_development_http
                && !url_is_loopback(&issuer_url)
            {
                return Err(Error::Invalid {
                    message: "plaintext OIDC issuers require the explicit insecure-development HTTP relaxation"
                        .to_owned(),
                });
            }
            validate_client_and_audiences(&config)?;
            if config.algorithms.is_empty() || config.algorithms.len() > SUPPORTED_ALGORITHMS.len()
            {
                return Err(Error::Invalid {
                    message: format!(
                        "issuer {}: algorithms must contain between 1 and {} unique entries",
                        config.issuer,
                        SUPPORTED_ALGORITHMS.len()
                    ),
                });
            }
            let mut configured_algorithms = Vec::with_capacity(config.algorithms.len());
            for algorithm in &config.algorithms {
                if !SUPPORTED_ALGORITHMS.contains(algorithm)
                    || configured_algorithms.contains(algorithm)
                {
                    return Err(Error::Invalid {
                        message: format!(
                            "issuer {}: algorithms must be unique and limited to \
                             RS256/RS384/RS512 (ADR-0010)",
                            config.issuer
                        ),
                    });
                }
                configured_algorithms.push(*algorithm);
            }
            validate_login_scopes(&config.issuer, &config.login_scopes)?;
            if !config.login_scopes.iter().any(|scope| scope == "openid") {
                return Err(Error::Invalid {
                    message: format!(
                        "issuer {}: login_scopes must include \"openid\" \
                         (no ID token without it)",
                        config.issuer
                    ),
                });
            }
            let issuer = config.issuer.clone();
            let metadata_endpoint_transport = if issuer_url.scheme() == "https" {
                MetadataEndpointTransport::HttpsOnly
            } else if insecure_development_http {
                MetadataEndpointTransport::InsecureDevelopmentHttp
            } else {
                MetadataEndpointTransport::LoopbackHttpOnly
            };
            let entry = IssuerEntry {
                config,
                metadata_endpoint_transport,
                state: RwLock::new(None),
                refresh: tokio::sync::Mutex::new(None),
            };
            if issuers.insert(issuer.clone(), entry).is_some() {
                return Err(Error::Invalid {
                    message: format!("issuer {issuer} is configured twice"),
                });
            }
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            // Ambient proxy variables are not part of the OIDC trust
            // contract. In particular, a loopback-HTTP issuer must never be
            // made remote by HTTP_PROXY/ALL_PROXY.
            .no_proxy()
            // IdP metadata, keys, and tokens must come from the configured
            // host, not wherever it redirects.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|err| Error::Internal {
                message: format!("building the OIDC HTTP client: {err}"),
            })?;
        Ok(Self {
            http,
            issuers,
            refresh_min_interval: REFRESH_MIN_INTERVAL,
        })
    }

    /// Overrides the JWKS refresh rate-limit. For tests that rotate keys
    /// faster than the production interval; deployments keep the default.
    #[must_use]
    pub fn with_refresh_min_interval(mut self, interval: Duration) -> Self {
        self.refresh_min_interval = interval;
        self
    }

    /// The shared HTTP client, for the login flow's token exchange.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// The configured issuer URLs.
    pub fn issuers(&self) -> impl Iterator<Item = &str> {
        self.issuers.keys().map(String::as_str)
    }

    /// The sole configured issuer, if exactly one.
    pub fn sole_issuer(&self) -> Option<&str> {
        let mut keys = self.issuers.keys();
        match (keys.next(), keys.next()) {
            (Some(issuer), None) => Some(issuer),
            _ => None,
        }
    }

    /// Eagerly proves every configured provider's discovery and JWKS
    /// contract. This is the provider-neutral deployment gate: it checks the
    /// byte-exact issuer, bounded metadata endpoints, PKCE S256, an allowed
    /// advertised signing algorithm and a usable key under that algorithm.
    /// It does not mint a token or make a provider-specific administration
    /// call.
    pub async fn diagnose(&self) -> std::result::Result<(), OidcDiagnosticError> {
        let mut entries = self.issuers.values().collect::<Vec<_>>();
        entries.sort_unstable_by(|left, right| left.config.issuer.cmp(&right.config.issuer));
        for entry in entries {
            let outcome = self.diagnose_entry(entry).await;
            metrics::counter!(
                OIDC_DIAGNOSTICS_TOTAL,
                "issuer" => entry.config.issuer.clone(),
                "outcome" => match &outcome {
                    Ok(()) => "ok",
                    Err(error) if error.is_retryable() => "unavailable",
                    Err(_) => "refused",
                },
            )
            .increment(1);
            outcome?;
        }
        Ok(())
    }

    /// Proves and installs every configured issuer snapshot before a product
    /// process opens readiness. Retryable provider outages get one bounded
    /// window; permanent metadata or configuration failures return at once.
    pub async fn initialize(&self) -> std::result::Result<(), OidcDiagnosticError> {
        self.initialize_with_deadline(
            INITIAL_DIAGNOSTIC_TIMEOUT,
            INITIAL_DIAGNOSTIC_RETRY_INTERVAL,
        )
        .await
    }

    async fn initialize_with_deadline(
        &self,
        timeout: Duration,
        retry_interval: Duration,
    ) -> std::result::Result<(), OidcDiagnosticError> {
        let attempts = async {
            loop {
                match self.diagnose().await {
                    Ok(()) => return Ok(()),
                    Err(error) if error.is_retryable() => {
                        tokio::time::sleep(retry_interval).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        tokio::time::timeout(timeout, attempts)
            .await
            .map_err(|_| OidcDiagnosticError::retryable("deadline"))?
    }

    #[tracing::instrument(
        name = "oidc.diagnose",
        skip_all,
        fields(oidc.issuer = %entry.config.issuer)
    )]
    async fn diagnose_entry(
        &self,
        entry: &IssuerEntry,
    ) -> std::result::Result<(), OidcDiagnosticError> {
        let mut last_attempt = entry.refresh.lock().await;
        let state = Arc::new(self.fetch_contract_state(entry).await?);
        *entry.state.write().expect("issuer state lock") = Some(state);
        *last_attempt = Some(Instant::now());
        Ok(())
    }

    pub(crate) fn config(&self, issuer: &str) -> Result<&IssuerConfig> {
        self.issuers
            .get(issuer)
            .map(|entry| &entry.config)
            .ok_or_else(|| Error::Invalid {
                message: format!("issuer {issuer} is not configured"),
            })
    }

    /// Discovery-backed endpoints for `issuer`, fetching on first use.
    pub(crate) async fn issuer_state(&self, issuer: &str) -> Result<Arc<IssuerState>> {
        let entry = self.entry(issuer)?;
        match cached(entry) {
            Some(state) => Ok(state),
            None => self.refresh(entry).await,
        }
    }

    /// Verifies an ID token from `issuer`: audience must be the client id
    /// alone — never a service audience (ADR-0018 decision 1) — and the
    /// `nonce` claim must match the login's nonce (ADR-0010 §5).
    pub(crate) async fn verify_id_token(
        &self,
        issuer: &str,
        token: &str,
        nonce: &str,
    ) -> Result<Claims> {
        let entry = self.entry(issuer)?;
        self.verify_against(entry, token, &[&entry.config.client_id], Some(nonce))
            .await
    }

    fn entry(&self, issuer: &str) -> Result<&IssuerEntry> {
        self.issuers
            .get(issuer)
            .ok_or_else(|| unauthenticated("unknown token issuer"))
    }

    /// Full verification under one trust entry. `expected_nonce` is set for
    /// ID tokens during login completion.
    #[tracing::instrument(name = "oidc.verify", skip_all, fields(oidc.issuer = %entry.config.issuer))]
    async fn verify_against(
        &self,
        entry: &IssuerEntry,
        token: &str,
        audiences: &[&str],
        expected_nonce: Option<&str>,
    ) -> Result<Claims> {
        let outcome = self
            .verify_inner(entry, token, audiences, expected_nonce)
            .await;
        let label = match &outcome {
            Ok(_) => "ok",
            Err(Error::Unauthenticated { .. }) => "rejected",
            Err(_) => "error",
        };
        metrics::counter!(
            TOKEN_VERIFICATIONS_TOTAL,
            "issuer" => entry.config.issuer.clone(),
            "outcome" => label,
        )
        .increment(1);
        outcome
    }

    async fn verify_inner(
        &self,
        entry: &IssuerEntry,
        token: &str,
        audiences: &[&str],
        expected_nonce: Option<&str>,
    ) -> Result<Claims> {
        let header = decode_header(token).map_err(|_| unauthenticated("malformed token header"))?;
        if !entry.config.algorithms.contains(&header.alg) {
            return Err(unauthenticated("token algorithm not allowed for issuer"));
        }
        let kid = header
            .kid
            .ok_or_else(|| unauthenticated("token has no key id"))?;

        // Cached snapshot, refreshing once on an unknown kid (rotation).
        let mut state = match cached(entry) {
            Some(state) => state,
            None => self.refresh(entry).await?,
        };
        if !state.keys.contains_key(&kid) {
            state = self.refresh(entry).await?;
        }
        let key = state
            .keys
            .get(&kid)
            .ok_or_else(|| unauthenticated("token signed with an unknown key"))?;
        if !key.algorithms.contains(&header.alg) {
            return Err(unauthenticated(
                "token algorithm does not match signing key",
            ));
        }
        if expected_nonce.is_some() && !state.id_token_algorithms.contains(&header.alg) {
            return Err(unauthenticated(
                "ID token algorithm is not advertised by the issuer",
            ));
        }

        let mut validation = Validation::new(header.alg);
        validation.leeway = LEEWAY.as_secs();
        validation.set_audience(audiences);
        validation.set_issuer(&[&entry.config.issuer]);
        if expected_nonce.is_some() {
            validation.set_required_spec_claims(&["exp", "iss", "aud", "iat"]);
        } else {
            validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        }
        let data = decode::<serde_json::Value>(token, &key.key, &validation)
            .map_err(|err| unauthenticated(&verification_failure(&err)))?;
        let claims = data.claims;

        let credential_class = if expected_nonce.is_some() {
            // `jsonwebtoken` deliberately accepts `iss` as either a string
            // or an array containing an allowed value. OIDC ID Tokens have
            // one StringOrURI issuer, and accepting a signed array would make
            // the login path less strict than discovery and bearer dispatch.
            validate_id_token_issuer(&claims, &entry.config.issuer)?;
            validate_id_token_authorized_party(&claims, &entry.config.client_id)?;
            if claims
                .get("iat")
                .and_then(serde_json::Value::as_u64)
                .is_none()
            {
                return Err(unauthenticated("ID token issued-at claim is malformed"));
            }
            CredentialClass::Interactive
        } else {
            classify_bearer_audience(&claims, &entry.config)?
        };

        // jsonwebtoken validates nbf only when asked to require it; check it
        // ourselves so an IdP that sets it (Entra does) is honoured.
        if let Some(value) = claims.get("nbf") {
            let nbf = value
                .as_u64()
                .ok_or_else(|| unauthenticated("token not-before claim is malformed"))?;
            if nbf > (now() + LEEWAY).as_secs() {
                return Err(unauthenticated("token not yet valid"));
            }
        }

        let subject = verified_subject(&claims, credential_class)?;

        match (
            expected_nonce,
            claims.get("nonce").and_then(serde_json::Value::as_str),
        ) {
            (None, _) => {}
            (Some(expected), Some(actual)) if expected == actual => {}
            (Some(_), _) => return Err(unauthenticated("login nonce mismatch")),
        }

        let tenant_id = match &entry.config.tenant {
            TenantBinding::Static { tenant_id } => *tenant_id,
            TenantBinding::Claim { name } => claims
                .get(name.as_str())
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| unauthenticated("token has no tenant claim"))?
                .parse()
                .map_err(|_| unauthenticated("tenant claim is not a UUID"))?,
        };

        // ID tokens require numeric `iat`; bearer tokens may omit it and then
        // carry an unknown lifetime, which the service-identity enforcement
        // seam refuses (ADR-0018 decision 5).
        let lifetime = oidc_lifetime(&claims, now().as_secs())?;

        Ok(Claims {
            subject,
            tenant_id,
            // Always Some for IdP-verified tokens, even with no groups:
            // presence marks the subject as IdP-backed (ADR-0013).
            provisioning: Some(provisioning_claims(
                &claims,
                &entry.config.groups_claim,
                &entry.config.external_id_claim,
            )),
            lifetime,
            credential_class,
        })
    }

    /// Refetches JWKS (and discovery on first use), rate-limited per issuer.
    /// Returns the freshest available snapshot; when rate-limited, that is
    /// the existing one.
    #[tracing::instrument(name = "oidc.jwks.refresh", skip_all, fields(oidc.issuer = %entry.config.issuer))]
    async fn refresh(&self, entry: &IssuerEntry) -> Result<Arc<IssuerState>> {
        let mut last_attempt = entry.refresh.lock().await;

        // Another request may have refreshed while this one waited; a
        // snapshot newer than the rate window is as fresh as a fetch.
        if let Some(at) = *last_attempt
            && at.elapsed() < self.refresh_min_interval
        {
            return cached(entry).ok_or_else(|| Error::Dependency {
                service: "oidc-jwks".to_owned(),
                message: "issuer keys unavailable; refresh rate-limited".to_owned(),
            });
        }
        *last_attempt = Some(Instant::now());

        let result = self.fetch_state(entry).await;
        metrics::counter!(
            JWKS_REFRESHES_TOTAL,
            "issuer" => entry.config.issuer.clone(),
            "outcome" => if result.is_ok() { "ok" } else { "error" },
        )
        .increment(1);
        let state = Arc::new(result?);
        *entry.state.write().expect("issuer state lock") = Some(Arc::clone(&state));
        Ok(state)
    }

    async fn fetch_state(&self, entry: &IssuerEntry) -> Result<IssuerState> {
        self.fetch_contract_state(entry)
            .await
            .map_err(|error| Error::Dependency {
                service: "oidc-metadata".to_owned(),
                message: error.to_string(),
            })
    }

    /// Fetches one complete runtime trust snapshot and validates the same
    /// contract as the deployment diagnostic before it can enter the cache.
    /// Discovery is refetched together with JWKS on rotation so direct binary
    /// launches and post-start provider drift cannot bypass the one-shot.
    async fn fetch_contract_state(
        &self,
        entry: &IssuerEntry,
    ) -> std::result::Result<IssuerState, OidcDiagnosticError> {
        let discovery = self.fetch_discovery(&entry.config.issuer).await?;
        validate_metadata_endpoint(
            &discovery.authorization_endpoint,
            "authorization endpoint",
            entry.metadata_endpoint_transport,
        )?;
        validate_metadata_endpoint(
            &discovery.token_endpoint,
            "token endpoint",
            entry.metadata_endpoint_transport,
        )?;
        validate_metadata_endpoint(
            &discovery.jwks_uri,
            "JWKS endpoint",
            entry.metadata_endpoint_transport,
        )?;
        let jwks: JwkSet = self.fetch_json(&discovery.jwks_uri, "JWKS").await?;
        if jwks.keys.len() > MAX_JWKS_KEYS {
            return Err(OidcDiagnosticError::refused("JWKS"));
        }
        let mut keys = HashMap::new();
        let mut key_ids = HashSet::new();
        for jwk in &jwks.keys {
            let Some(kid) = jwk.common.key_id.clone() else {
                continue;
            };
            if kid.is_empty() || kid.len() > 256 || !key_ids.insert(kid.clone()) {
                return Err(OidcDiagnosticError::refused("JWKS"));
            }
            let Some(verifying) = verifying_key(jwk, &entry.config.algorithms) else {
                tracing::debug!("skipping JWKS key unusable under the issuer allowlist");
                continue;
            };
            keys.insert(kid, verifying);
        }
        if keys.is_empty() {
            return Err(OidcDiagnosticError::refused("JWKS"));
        }
        let id_token_algorithms = advertised_id_token_algorithms(&entry.config, &discovery)?;
        let state = IssuerState {
            discovery,
            keys,
            id_token_algorithms,
        };
        validate_diagnostic(&entry.config, &state)?;
        Ok(state)
    }

    #[tracing::instrument(name = "oidc.discovery", skip_all, fields(oidc.issuer = %issuer))]
    async fn fetch_discovery(
        &self,
        issuer: &str,
    ) -> std::result::Result<DiscoveryDocument, OidcDiagnosticError> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let document: DiscoveryDocument = self.fetch_json(&url, "discovery").await?;
        // Byte-for-byte per ADR-0010: a document that names another issuer
        // is misconfiguration or an attack, never something to normalise.
        if document.issuer != issuer {
            return Err(OidcDiagnosticError::refused("discovery"));
        }
        Ok(document)
    }

    async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        stage: &'static str,
    ) -> std::result::Result<T, OidcDiagnosticError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| OidcDiagnosticError::retryable(stage))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            return Err(if matches!(status, 408 | 425 | 429 | 500..=599) {
                OidcDiagnosticError::retryable(stage)
            } else {
                OidcDiagnosticError::refused(stage)
            });
        }
        let mut response = response;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
        {
            return Err(OidcDiagnosticError::refused(stage));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| OidcDiagnosticError::retryable(stage))?
        {
            let remaining = MAX_METADATA_BYTES.saturating_sub(body.len());
            if chunk.len() > remaining {
                return Err(OidcDiagnosticError::refused(stage));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| OidcDiagnosticError::refused(stage))
    }
}

#[async_trait::async_trait]
impl TokenVerifier for OidcVerifier {
    async fn verify(&self, token: &str) -> Result<Claims> {
        // The unverified `iss` selects the trust entry — nothing more.
        let issuer = unverified_issuer(token)?;
        let entry = self.entry(&issuer)?;
        let audiences = entry.config.bearer_audiences();
        self.verify_against(entry, token, &audiences, None).await
    }
}

fn cached(entry: &IssuerEntry) -> Option<Arc<IssuerState>> {
    entry.state.read().expect("issuer state lock").clone()
}

fn validate_diagnostic(
    config: &IssuerConfig,
    state: &IssuerState,
) -> std::result::Result<(), OidcDiagnosticError> {
    let methods = state
        .discovery
        .code_challenge_methods_supported
        .as_ref()
        .ok_or_else(|| OidcDiagnosticError::refused("PKCE"))?;
    validate_discovery_values(methods, "PKCE metadata")?;
    if !methods.iter().any(|method| method == "S256") {
        return Err(OidcDiagnosticError::refused("PKCE"));
    }
    validate_discovery_values(
        &state.discovery.id_token_signing_alg_values_supported,
        "signing-algorithm metadata",
    )?;
    validate_discovery_values(
        &state.discovery.response_types_supported,
        "response-type metadata",
    )?;
    if let Some(grants) = &state.discovery.grant_types_supported {
        validate_discovery_values(grants, "grant-type metadata")?;
    }
    if let Some(scopes) = &state.discovery.scopes_supported {
        if scopes.len() > MAX_ADVERTISED_SCOPES {
            return Err(OidcDiagnosticError::refused("scope metadata"));
        }
        let mut unique = HashSet::with_capacity(scopes.len());
        for scope in scopes {
            if !valid_scope_token(scope) || !unique.insert(scope.as_str()) {
                return Err(OidcDiagnosticError::refused("scope metadata"));
            }
        }
    }
    if !state
        .discovery
        .response_types_supported
        .iter()
        .any(|response_type| response_type == "code")
    {
        return Err(OidcDiagnosticError::refused("authorization-code"));
    }
    if state
        .discovery
        .grant_types_supported
        .as_ref()
        .is_some_and(|grant_types| {
            !grant_types
                .iter()
                .any(|grant_type| grant_type == "authorization_code")
        })
    {
        return Err(OidcDiagnosticError::refused("authorization-code"));
    }
    if config
        .login_scopes
        .iter()
        .any(|scope| scope == "offline_access")
        && !state.advertises_scope("offline_access")
    {
        return Err(OidcDiagnosticError::refused("offline-access"));
    }

    let advertised = &state.id_token_algorithms;
    if advertised.is_empty() {
        return Err(OidcDiagnosticError::refused("signing-algorithm"));
    }
    if !state.keys.values().any(|key| {
        key.algorithms
            .iter()
            .any(|algorithm| advertised.contains(algorithm))
    }) {
        return Err(OidcDiagnosticError::refused("signing-key"));
    }
    Ok(())
}

fn advertised_id_token_algorithms(
    config: &IssuerConfig,
    discovery: &DiscoveryDocument,
) -> std::result::Result<Vec<Algorithm>, OidcDiagnosticError> {
    validate_discovery_values(
        &discovery.id_token_signing_alg_values_supported,
        "signing-algorithm metadata",
    )?;
    let mut supported = [false; SUPPORTED_ALGORITHMS.len()];
    for name in &discovery.id_token_signing_alg_values_supported {
        match name.as_str() {
            "RS256" => supported[0] = true,
            "RS384" => supported[1] = true,
            "RS512" => supported[2] = true,
            _ => {}
        }
    }
    Ok(config
        .algorithms
        .iter()
        .copied()
        .filter(|algorithm| {
            SUPPORTED_ALGORITHMS
                .iter()
                .position(|supported_algorithm| supported_algorithm == algorithm)
                .is_some_and(|index| supported[index])
        })
        .collect())
}

fn validate_discovery_values(
    values: &[String],
    stage: &'static str,
) -> std::result::Result<(), OidcDiagnosticError> {
    if values.len() > MAX_DISCOVERY_VALUES {
        return Err(OidcDiagnosticError::refused(stage));
    }
    let mut unique = HashSet::with_capacity(values.len());
    for value in values {
        if value.is_empty()
            || value.len() > MAX_DISCOVERY_VALUE_BYTES
            || value.chars().any(char::is_control)
            || !unique.insert(value.as_str())
        {
            return Err(OidcDiagnosticError::refused(stage));
        }
    }
    Ok(())
}

fn validate_metadata_endpoint(
    raw: &str,
    stage: &'static str,
    transport: MetadataEndpointTransport,
) -> std::result::Result<(), OidcDiagnosticError> {
    let url = url::Url::parse(raw).map_err(|_| OidcDiagnosticError::refused(stage))?;
    let transport_allowed = match transport {
        MetadataEndpointTransport::HttpsOnly => url.scheme() == "https",
        MetadataEndpointTransport::LoopbackHttpOnly => {
            url.scheme() == "https" || url.scheme() == "http" && url_is_loopback(&url)
        }
        MetadataEndpointTransport::InsecureDevelopmentHttp => {
            matches!(url.scheme(), "http" | "https")
        }
    };
    if !transport_allowed
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(OidcDiagnosticError::refused(stage));
    }
    Ok(())
}

fn validate_issuer_identifier(raw: &str) -> Result<url::Url> {
    let invalid = || {
        Error::Invalid {
        message: "configured OIDC issuer must be an absolute credential-free HTTP(S) URL without a query or fragment"
            .to_owned(),
    }
    };
    let url = url::Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}

fn url_is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn validate_login_scopes(issuer: &str, scopes: &[String]) -> Result<()> {
    if scopes.is_empty() || scopes.len() > MAX_LOGIN_SCOPES {
        return Err(Error::Invalid {
            message: format!(
                "issuer {issuer}: login_scopes must contain between 1 and {MAX_LOGIN_SCOPES} entries"
            ),
        });
    }
    let mut unique = HashSet::new();
    for scope in scopes {
        if !valid_scope_token(scope) || !unique.insert(scope.as_str()) {
            return Err(Error::Invalid {
                message: format!(
                    "issuer {issuer}: login_scopes must be unique RFC 6749 scope tokens"
                ),
            });
        }
    }
    Ok(())
}

fn valid_scope_token(scope: &str) -> bool {
    !scope.is_empty()
        && scope.len() <= MAX_LOGIN_SCOPE_BYTES
        && scope
            .bytes()
            .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}

fn validate_client_and_audiences(config: &IssuerConfig) -> Result<()> {
    if config.service_audiences.len() > MAX_SERVICE_AUDIENCES {
        return Err(Error::Invalid {
            message: format!(
                "issuer {}: at most {MAX_SERVICE_AUDIENCES} service audiences may be configured",
                config.issuer
            ),
        });
    }
    let valid = |value: &str| {
        !value.is_empty()
            && value.len() <= 512
            && value
                .chars()
                .all(|character| !character.is_control() && !character.is_whitespace())
    };
    if !valid(&config.client_id)
        || !valid(&config.audience)
        || config.service_audiences.iter().any(|value| !valid(value))
    {
        return Err(Error::Invalid {
            message: format!(
                "issuer {}: client and audience identifiers must be nonempty bounded tokens",
                config.issuer
            ),
        });
    }
    let mut unique = HashSet::from([config.audience.as_str()]);
    if config.audience == config.client_id
        || config
            .service_audiences
            .iter()
            .any(|audience| audience == &config.client_id)
    {
        return Err(Error::Invalid {
            message: format!(
                "issuer {}: bearer audiences must be distinct from the login client id",
                config.issuer
            ),
        });
    }
    if config
        .service_audiences
        .iter()
        .any(|audience| !unique.insert(audience.as_str()))
    {
        return Err(Error::Invalid {
            message: format!("issuer {}: audiences must be unique", config.issuer),
        });
    }
    Ok(())
}

fn classify_bearer_audience(
    claims: &serde_json::Value,
    config: &IssuerConfig,
) -> Result<CredentialClass> {
    let audiences: Vec<&str> = match claims.get("aud") {
        Some(serde_json::Value::String(audience)) if !audience.is_empty() => vec![audience],
        Some(serde_json::Value::Array(audiences))
            if !audiences.is_empty() && audiences.len() <= MAX_SERVICE_AUDIENCES + 1 =>
        {
            audiences
                .iter()
                .map(serde_json::Value::as_str)
                .collect::<Option<Vec<_>>>()
                .filter(|values| values.iter().all(|value| !value.is_empty()))
                .ok_or_else(|| unauthenticated("token audience claim is malformed"))?
        }
        _ => return Err(unauthenticated("token audience claim is malformed")),
    };
    let mut seen = HashSet::with_capacity(audiences.len());
    let mut service = false;
    for audience in audiences {
        if !seen.insert(audience) {
            return Err(unauthenticated("token audience claim is malformed"));
        }
        if audience == config.client_id {
            return Err(unauthenticated(
                "bearer token includes the login client audience",
            ));
        }
        if audience == config.audience {
            continue;
        }
        if config
            .service_audiences
            .iter()
            .any(|accepted| accepted == audience)
        {
            service = true;
            continue;
        }
        // `jsonwebtoken` deliberately uses any-match audience semantics. A
        // closed set here prevents an ID token for an unknown client from
        // becoming a bearer merely because a resource audience was appended.
        return Err(unauthenticated("token includes an unknown audience"));
    }
    if service {
        Ok(CredentialClass::ServiceBearer)
    } else {
        Ok(CredentialClass::PrimaryBearer)
    }
}

fn verified_subject(claims: &serde_json::Value, class: CredentialClass) -> Result<String> {
    let claimed = |name: &str| {
        claims
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
    };
    claimed("sub")
        .or_else(|| {
            (class == CredentialClass::ServiceBearer)
                .then(|| claimed("azp"))
                .flatten()
        })
        .map(str::to_owned)
        .ok_or_else(|| unauthenticated("token has no subject"))
}

/// Reads the `iss` claim without verifying anything. The result is only
/// ever used to pick which trust entry must then fully verify the token.
fn unverified_issuer(token: &str) -> Result<String> {
    let mut parts = token.split('.');
    let (Some(_), Some(payload), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unauthenticated("malformed token"));
    };
    URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|claims| {
            claims
                .get("iss")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| unauthenticated("token has no issuer"))
}

/// Applies OIDC Core §2's authorized-party rule after signature, issuer and
/// audience verification. A multi-audience ID token must identify this login
/// client as `azp`; when `azp` is present on any ID token it must name that
/// same client.
fn validate_id_token_issuer(claims: &serde_json::Value, expected: &str) -> Result<()> {
    if claims.get("iss").and_then(serde_json::Value::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(unauthenticated("ID token issuer claim is malformed"))
    }
}

fn validate_id_token_authorized_party(claims: &serde_json::Value, client_id: &str) -> Result<()> {
    let audience_count = match claims.get("aud") {
        Some(serde_json::Value::String(audience)) if !audience.is_empty() => 1,
        Some(serde_json::Value::Array(audiences))
            if !audiences.is_empty()
                && audiences.iter().all(|audience| {
                    audience
                        .as_str()
                        .is_some_and(|audience| !audience.is_empty())
                }) =>
        {
            audiences.len()
        }
        _ => return Err(unauthenticated("ID token audience is malformed")),
    };
    let authorized_party = match claims.get("azp") {
        None => None,
        Some(serde_json::Value::String(value)) if !value.is_empty() => Some(value.as_str()),
        Some(_) => return Err(unauthenticated("ID token authorized party is malformed")),
    };
    if authorized_party.is_some_and(|party| party != client_id)
        || audience_count > 1 && authorized_party != Some(client_id)
    {
        return Err(unauthenticated("ID token authorized party mismatch"));
    }
    Ok(())
}

/// Builds the cached key for one JWK, or `None` if the key cannot verify
/// anything under the issuer's algorithm allowlist.
fn verifying_key(jwk: &Jwk, allowed: &[Algorithm]) -> Option<VerifyingKey> {
    if jwk
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| usage != &PublicKeyUse::Signature)
    {
        return None;
    }
    if let Some(operations) = &jwk.common.key_operations {
        if operations.len() > 2 {
            return None;
        }
        let unique = operations.iter().collect::<HashSet<_>>();
        if unique.len() != operations.len()
            || !operations.contains(&KeyOperations::Verify)
            || operations
                .iter()
                .any(|operation| !matches!(operation, KeyOperations::Sign | KeyOperations::Verify))
        {
            return None;
        }
    }
    let family: &[Algorithm] = match &jwk.algorithm {
        AlgorithmParameters::RSA(parameters) => {
            let modulus = URL_SAFE_NO_PAD.decode(&parameters.n).ok()?;
            let exponent = URL_SAFE_NO_PAD.decode(&parameters.e).ok()?;
            if modulus.is_empty() || exponent.is_empty() {
                return None;
            }
            let key = rsa::RsaPublicKey::new(
                rsa::BigUint::from_bytes_be(&modulus),
                rsa::BigUint::from_bytes_be(&exponent),
            )
            .ok()?;
            if !(2048..=4096).contains(&key.n().bits()) {
                return None;
            }
            &SUPPORTED_ALGORITHMS
        }
        // EC/OKP/oct keys verify algorithms this build does not compile.
        _ => return None,
    };
    // Narrow by the JWK's own alg when it declares one.
    let declared = match jwk.common.key_algorithm {
        None => None,
        Some(KeyAlgorithm::RS256) => Some(Algorithm::RS256),
        Some(KeyAlgorithm::RS384) => Some(Algorithm::RS384),
        Some(KeyAlgorithm::RS512) => Some(Algorithm::RS512),
        // A present encryption, asymmetric-family or unknown algorithm is
        // a prohibition, not an omitted signing-algorithm hint.
        Some(_) => return None,
    };
    let algorithms: Vec<Algorithm> = allowed
        .iter()
        .copied()
        .filter(|alg| family.contains(alg))
        .filter(|alg| declared.is_none_or(|declared| declared == *alg))
        .collect();
    if algorithms.is_empty() {
        return None;
    }
    let key = DecodingKey::from_jwk(jwk).ok()?;
    Some(VerifyingKey { key, algorithms })
}

/// Harvests the provisioning claims (AUTH-2, ADR-0013) from a verified
/// token's payload. Absent or ill-shaped claims degrade to empty/`None` —
/// they gate placement, never verification; non-string group entries
/// (some IdPs mix formats) are skipped.
fn provisioning_claims(
    claims: &serde_json::Value,
    groups_claim: &str,
    external_id_claim: &str,
) -> ProvisioningClaims {
    let text = |name: &str| {
        claims
            .get(name)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let groups = claims
        .get(groups_claim)
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|group| !group.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let verified_email = claims
        .get("email_verified")
        .and_then(serde_json::Value::as_bool)
        .filter(|verified| *verified)
        .and_then(|_| text("email"));
    let external_id = if external_id_claim == "email" {
        verified_email.clone()
    } else {
        text(external_id_claim)
    };
    ProvisioningClaims {
        groups,
        email: verified_email,
        display_name: text("name"),
        external_id,
    }
}

fn oidc_lifetime(claims: &serde_json::Value, now: u64) -> Result<Option<Duration>> {
    let exp = claims
        .get("exp")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| unauthenticated("token expiry is not a valid timestamp"))?;
    let iat = match claims.get("iat") {
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| unauthenticated("token issued-at is not a valid timestamp"))?,
        ),
        None => None,
    };
    issued_lifetime(exp, iat, now)
}

/// Collapses jsonwebtoken's error detail into caller-safe messages.
fn verification_failure(err: &jsonwebtoken::errors::Error) -> String {
    use jsonwebtoken::errors::ErrorKind;
    match err.kind() {
        ErrorKind::ExpiredSignature => "token expired".to_owned(),
        ErrorKind::InvalidAudience => "token audience mismatch".to_owned(),
        ErrorKind::InvalidIssuer => "token issuer mismatch".to_owned(),
        ErrorKind::MissingRequiredClaim(claim) => {
            format!("token is missing the {claim} claim")
        }
        _ => "token verification failed".to_owned(),
    }
}

fn unauthenticated(message: &str) -> Error {
    Error::Unauthenticated {
        message: message.to_owned(),
    }
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issuer_config_defaults_apply() {
        let configs = parse_issuers(
            r#"[{"issuer":"http://localhost:8100/auth/v1","client_id":"synveda","audience":"synveda-api"}]"#,
        )
        .expect("parse");
        assert_eq!(configs.len(), 1);
        let config = &configs[0];
        assert_eq!(config.bearer_audiences(), ["synveda-api"]);
        assert_eq!(config.algorithms, vec![Algorithm::RS256]);
        assert_eq!(
            config.tenant,
            TenantBinding::Claim {
                name: "tid".to_owned()
            }
        );
        assert_eq!(config.groups_claim, "groups");
        assert_eq!(config.login_scopes, ["openid", "profile", "email"]);
    }

    #[test]
    fn login_scopes_without_openid_are_rejected_at_construction() {
        let configs = parse_issuers(
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","login_scopes":["profile","groups"]}]"#,
        )
        .expect("parse");
        let err = OidcVerifier::new(configs)
            .err()
            .expect("scopes without openid must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn login_scopes_are_unique_rfc6749_tokens() {
        for scopes in [
            r#"["openid","profile offline_access"]"#,
            r#"["openid","profile\toffline_access"]"#,
            r#"["openid","profile\noffline_access"]"#,
            r#"["openid","openid"]"#,
            r#"["openid","bad\\scope"]"#,
            r#"["openid","bad\"scope"]"#,
        ] {
            let configs = parse_issuers(&format!(
                r#"[{{"issuer":"https://idp","client_id":"c","audience":"api","login_scopes":{scopes}}}]"#
            ))
            .expect("scope fixture parses");
            let error = OidcVerifier::new(configs)
                .err()
                .expect("scope token must be refused");
            assert!(
                matches!(error, Error::Invalid { .. }),
                "{scopes}: {error:?}"
            );
        }
    }

    #[test]
    fn client_and_audience_identifiers_are_nonempty_and_unique() {
        let missing = parse_issuers(r#"[{"issuer":"http://idp","client_id":"c"}]"#)
            .expect_err("the bearer audience is required");
        assert!(matches!(missing, Error::Invalid { .. }));

        for config in [
            r#"[{"issuer":"https://idp","client_id":"","audience":"api"}]"#,
            r#"[{"issuer":"https://idp","client_id":"c","audience":""}]"#,
            r#"[{"issuer":"https://idp","client_id":"c","audience":"c"}]"#,
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","service_audiences":["c"]}]"#,
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","service_audiences":["a","a"]}]"#,
        ] {
            let issuers = parse_issuers(config).expect("identifier fixture parses");
            assert!(OidcVerifier::new(issuers).is_err(), "accepted {config}");
        }
    }

    #[test]
    fn provisioning_claims_harvest_groups_email_and_name() {
        let claims = serde_json::json!({
            "sub": "alice",
            "groups": ["synveda-eng-platform", "", 42, "everyone"],
            "email": "alice@example.test",
            "email_verified": true,
            "name": "Alice Example",
        });
        let harvested = provisioning_claims(&claims, "groups", "sub");
        assert_eq!(harvested.groups, ["synveda-eng-platform", "everyone"]);
        assert_eq!(harvested.email.as_deref(), Some("alice@example.test"));
        assert_eq!(harvested.display_name.as_deref(), Some("Alice Example"));
    }

    #[test]
    fn provisioning_claims_ignore_email_without_strict_verification() {
        for email_verified in [
            None,
            Some(serde_json::Value::Bool(false)),
            Some(serde_json::Value::String("true".to_owned())),
            Some(serde_json::Value::Number(1.into())),
        ] {
            let mut claims = serde_json::json!({
                "sub": "alice",
                "email": "alice@example.test"
            });
            if let Some(value) = email_verified {
                claims["email_verified"] = value;
            }
            let harvested = provisioning_claims(&claims, "groups", "sub");
            assert_eq!(harvested.email, None, "from {claims}");
            let email_anchor = provisioning_claims(&claims, "groups", "email");
            assert_eq!(
                email_anchor.external_id, None,
                "an unverified email must not regain authority as the configured anchor: {claims}"
            );
        }
        let verified = serde_json::json!({
            "email": "alice@example.test",
            "email_verified": true,
        });
        assert_eq!(
            provisioning_claims(&verified, "groups", "email")
                .external_id
                .as_deref(),
            Some("alice@example.test")
        );
    }

    #[test]
    fn provisioning_claims_degrade_to_empty_when_absent_or_ill_shaped() {
        for claims in [
            serde_json::json!({ "sub": "alice" }),
            serde_json::json!({ "sub": "alice", "groups": "not-an-array", "email": 7 }),
        ] {
            let harvested = provisioning_claims(&claims, "groups", "sub");
            assert_eq!(
                ProvisioningClaims {
                    external_id: None,
                    ..harvested.clone()
                },
                ProvisioningClaims::default(),
                "from {claims}"
            );
            // The one thing these tokens *do* carry: the default anchor is
            // `sub`, so a subject that is stable across applications needs
            // no per-issuer configuration at all (AUTH-4, ADR-0059
            // decision 4).
            assert_eq!(harvested.external_id.as_deref(), Some("alice"));
        }
        // A configured claim name other than the default is honoured — for
        // groups, and for the anchor.
        let entra_style = serde_json::json!({
            "sub": "pairwise-per-application",
            "oid": "9f2c1b70-object-id",
            "wids": ["role-a"],
            "groups": ["ignored"]
        });
        let harvested = provisioning_claims(&entra_style, "wids", "oid");
        assert_eq!(harvested.groups, ["role-a"]);
        // The Entra case in one assertion: the anchor is the object id, and
        // it is *not* the subject. A server that joined a SCIM mirror row on
        // `sub` here would match nothing and provision a second identity.
        assert_eq!(harvested.external_id.as_deref(), Some("9f2c1b70-object-id"));
        assert_ne!(
            harvested.external_id.as_deref(),
            Some("pairwise-per-application")
        );
    }

    #[test]
    fn issuer_config_parses_static_tenant_and_audience() {
        let tenant = TenantId::new();
        let configs = parse_issuers(&format!(
            r#"[{{"issuer":"http://idp","client_id":"c","audience":"api://synveda",
                 "tenant":{{"static":{{"tenant_id":"{tenant}"}}}}}}]"#,
        ))
        .expect("parse");
        assert_eq!(configs[0].bearer_audiences(), ["api://synveda"]);
        assert_eq!(
            configs[0].tenant,
            TenantBinding::Static { tenant_id: tenant }
        );
    }

    #[test]
    fn service_audiences_extend_bearer_audiences_only() {
        let configs = parse_issuers(
            r#"[{"issuer":"http://idp","client_id":"synveda",
                 "audience":"synveda-api",
                 "service_audiences":["ci-agent","api://agents"]}]"#,
        )
        .expect("parse");
        // Bearer tokens may carry the API audience or any service audience;
        // ID-token verification passes client_id alone (ADR-0018 decision 1).
        assert_eq!(
            configs[0].bearer_audiences(),
            ["synveda-api", "ci-agent", "api://agents"]
        );
    }

    #[test]
    fn bearer_audience_classification_is_closed_and_service_tainted() {
        let config = parse_issuers(
            r#"[{"issuer":"https://idp","client_id":"interactive",
                 "audience":"synveda-api","service_audiences":["service-client"]}]"#,
        )
        .expect("config")
        .remove(0);
        assert_eq!(
            classify_bearer_audience(&serde_json::json!({"aud": "synveda-api"}), &config)
                .expect("primary audience"),
            CredentialClass::PrimaryBearer
        );
        for aud in [
            serde_json::json!("service-client"),
            serde_json::json!(["synveda-api", "service-client"]),
        ] {
            assert_eq!(
                classify_bearer_audience(&serde_json::json!({"aud": aud}), &config)
                    .expect("service-tainted audience"),
                CredentialClass::ServiceBearer
            );
        }
        for aud in [
            serde_json::json!("interactive"),
            serde_json::json!(["synveda-api", "unknown-client"]),
            serde_json::json!(["synveda-api", "synveda-api"]),
            serde_json::json!([]),
            serde_json::json!(["synveda-api", 7]),
        ] {
            classify_bearer_audience(&serde_json::json!({"aud": aud}), &config)
                .expect_err("open or malformed audience set must be refused");
        }
    }

    #[test]
    fn authorized_party_fallback_is_service_bearer_only() {
        let claims = serde_json::json!({"sub": null, "azp": "client-subject"});
        assert!(
            verified_subject(&claims, CredentialClass::PrimaryBearer).is_err(),
            "a primary API bearer must not turn an OAuth client id into a user subject"
        );
        assert!(
            verified_subject(&claims, CredentialClass::Interactive).is_err(),
            "an ID token must carry its OIDC subject"
        );
        assert_eq!(
            verified_subject(&claims, CredentialClass::ServiceBearer)
                .expect("client-credentials fallback"),
            "client-subject"
        );
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        let err = parse_issuers(
            r#"[{"issuer":"http://idp","client_id":"c","audience":"api","cliett_id":"typo"}]"#,
        )
        .expect_err("typo must not parse");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn issuer_file_rejects_inline_directory_credentials_without_echoing_values() {
        for inline in [
            r#"{"connector":"okta","org_url":"https://okta.example.test","api_token":"never-log-directory-secret"}"#,
            r#"{"connector":"entra","tenant_id":"t","client_id":"c","client_secret":"never-log-directory-secret"}"#,
        ] {
            let error = parse_issuers(&format!(
                r#"[{{"issuer":"https://idp.example.test","client_id":"c","audience":"api","directory_sync":{inline}}}]"#
            ))
            .expect_err("inline credential must be refused");
            assert!(matches!(error, Error::Invalid { .. }));
            assert!(!error.to_string().contains("never-log-directory-secret"));
        }

        parse_issuers(
            r#"[{"issuer":"https://idp.example.test","client_id":"c","audience":"api","directory_sync":{"connector":"okta","org_url":"https://okta.example.test","api_token_file":"/run/secrets/oidc_directory/okta_token"}}]"#,
        )
        .expect("credential file reference");
    }

    #[test]
    fn unsupported_algorithms_are_rejected_at_construction() {
        let configs = parse_issuers(
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","algorithms":["HS256"]}]"#,
        )
        .expect("HS256 parses as an algorithm name");
        let err = OidcVerifier::new(configs)
            .err()
            .expect("HS256 must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn configured_algorithms_are_unique_and_fixed_set_bounded() {
        for config in [
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","algorithms":["RS256","RS256"]}]"#,
            r#"[{"issuer":"https://idp","client_id":"c","audience":"api","algorithms":["RS256","RS384","RS512","RS256"]}]"#,
        ] {
            let issuers = parse_issuers(config).expect("algorithm fixture parses");
            let error = OidcVerifier::new(issuers)
                .err()
                .expect("duplicate or oversized algorithm allowlist must be refused");
            assert!(matches!(error, Error::Invalid { .. }), "{error:?}");
        }
    }

    #[test]
    fn duplicate_issuers_are_rejected() {
        let configs = parse_issuers(
            r#"[{"issuer":"https://idp","client_id":"a","audience":"api-a"},
                {"issuer":"https://idp","client_id":"b","audience":"api-b"}]"#,
        )
        .expect("parse");
        let err = OidcVerifier::new(configs)
            .err()
            .expect("duplicate issuer must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn empty_issuer_list_is_rejected() {
        let err = OidcVerifier::new(Vec::new())
            .err()
            .expect("empty config must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn unverified_issuer_reads_iss_only_from_well_formed_tokens() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"iss":"http://idp","sub":"alice"}"#);
        let token = format!("h.{payload}.s");
        assert_eq!(unverified_issuer(&token).expect("iss"), "http://idp");
        for garbage in ["", "a.b", "a.b.c.d", "h.!!!.s"] {
            assert!(unverified_issuer(garbage).is_err(), "accepted {garbage:?}");
        }
        let no_iss = format!("h.{}.s", URL_SAFE_NO_PAD.encode(r#"{"sub":"a"}"#));
        assert!(unverified_issuer(&no_iss).is_err());
    }

    #[tokio::test]
    async fn sole_issuer_is_only_reported_for_single_entry_configs() {
        let one = OidcVerifier::new(
            parse_issuers(r#"[{"issuer":"https://a","client_id":"c","audience":"api"}]"#).unwrap(),
        )
        .unwrap();
        assert_eq!(one.sole_issuer(), Some("https://a"));
        let two = OidcVerifier::new(
            parse_issuers(
                r#"[{"issuer":"https://a","client_id":"c","audience":"api-a"},
                    {"issuer":"https://b","client_id":"c","audience":"api-b"}]"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(two.sole_issuer(), None);
    }

    #[tokio::test]
    async fn tokens_from_unknown_issuers_are_rejected_without_io() {
        let verifier = OidcVerifier::new(
            parse_issuers(r#"[{"issuer":"https://a","client_id":"c","audience":"api"}]"#).unwrap(),
        )
        .unwrap();
        let payload = URL_SAFE_NO_PAD.encode(r#"{"iss":"http://evil","sub":"x"}"#);
        let err = verifier
            .verify(&format!("h.{payload}.s"))
            .await
            .expect_err("unknown issuer must be rejected");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }

    #[test]
    fn remote_plaintext_issuer_requires_explicit_development_relaxation() {
        for sentinel in [
            "http://never-log-plaintext-issuer.example.test",
            "http://auth.synveda.localhost/realms/synveda",
        ] {
            let json = format!(r#"[{{"issuer":"{sentinel}","client_id":"c","audience":"api"}}]"#);
            let error = OidcVerifier::new(parse_issuers(&json).expect("parse"))
                .err()
                .expect("remote plaintext must be refused");
            assert!(!error.to_string().contains(sentinel));

            OidcVerifier::new_with_insecure_development_http(
                parse_issuers(&json).expect("parse"),
                true,
            )
            .expect("explicit development relaxation");
        }
    }

    #[test]
    fn loopback_plaintext_issuers_do_not_require_the_relaxation() {
        for issuer in [
            "http://localhost:8100/realms/synveda",
            "http://127.0.0.1:8100/realms/synveda",
            "http://[::1]:8100/realms/synveda",
        ] {
            let json = format!(r#"[{{"issuer":"{issuer}","client_id":"c","audience":"api"}}]"#);
            OidcVerifier::new(parse_issuers(&json).expect("parse"))
                .unwrap_or_else(|error| panic!("refused {issuer}: {error}"));
        }
    }

    fn diagnostic_fixture(
        pkce: &[&str],
        advertised_algorithms: &[&str],
        key_algorithms: Vec<Algorithm>,
    ) -> (IssuerConfig, IssuerState) {
        let config = parse_issuers(
            r#"[{"issuer":"https://auth.example.test/realms/synveda","client_id":"synveda","audience":"synveda-api"}]"#,
        )
        .expect("issuer config")
        .remove(0);
        let discovery = DiscoveryDocument {
            issuer: config.issuer.clone(),
            authorization_endpoint: format!("{}/protocol/openid-connect/auth", config.issuer),
            token_endpoint: format!("{}/protocol/openid-connect/token", config.issuer),
            jwks_uri: format!("{}/protocol/openid-connect/certs", config.issuer),
            code_challenge_methods_supported: Some(
                pkce.iter().map(|value| (*value).to_owned()).collect(),
            ),
            id_token_signing_alg_values_supported: advertised_algorithms
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            response_types_supported: vec!["code".to_owned()],
            grant_types_supported: Some(vec!["authorization_code".to_owned()]),
            scopes_supported: Some(vec!["openid".to_owned()]),
        };
        let keys = HashMap::from([(
            "test-key".to_owned(),
            VerifyingKey {
                key: DecodingKey::from_secret(b"diagnostic-only"),
                algorithms: key_algorithms,
            },
        )]);
        let id_token_algorithms =
            advertised_id_token_algorithms(&config, &discovery).expect("bounded fixture metadata");
        (
            config,
            IssuerState {
                discovery,
                keys,
                id_token_algorithms,
            },
        )
    }

    #[test]
    fn diagnostic_requires_explicit_s256_pkce_metadata() {
        let (config, valid) = diagnostic_fixture(&["S256"], &["RS256"], vec![Algorithm::RS256]);
        validate_diagnostic(&config, &valid).expect("complete diagnostic contract");

        let (_, mixed) = diagnostic_fixture(&["plain", "S256"], &["RS256"], vec![Algorithm::RS256]);
        validate_diagnostic(&config, &mixed).expect("S256 remains usable in a mixed method list");

        let mut omitted = diagnostic_fixture(&["plain"], &["RS256"], vec![Algorithm::RS256]).1;
        omitted.discovery.code_challenge_methods_supported = None;
        let error = validate_diagnostic(&config, &omitted)
            .expect_err("the deployment gate cannot prove S256 when metadata omits it");
        assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);

        let (_, no_s256) = diagnostic_fixture(&["plain"], &["RS256"], vec![Algorithm::RS256]);
        let error = validate_diagnostic(&config, &no_s256)
            .expect_err("an advertised method list that excludes S256 must be refused");
        assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);

        let (_, empty) = diagnostic_fixture(&[], &["RS256"], vec![Algorithm::RS256]);
        let error = validate_diagnostic(&config, &empty)
            .expect_err("a present but empty method list explicitly advertises no S256");
        assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);
    }

    #[test]
    fn pkce_metadata_distinguishes_omission_from_explicit_null() {
        let metadata = serde_json::json!({
            "issuer": "https://auth.example.test/realms/synveda",
            "authorization_endpoint": "https://auth.example.test/realms/synveda/auth",
            "token_endpoint": "https://auth.example.test/realms/synveda/token",
            "jwks_uri": "https://auth.example.test/realms/synveda/certs",
            "id_token_signing_alg_values_supported": ["RS256"],
            "response_types_supported": ["code"]
        });
        let omitted: DiscoveryDocument =
            serde_json::from_value(metadata.clone()).expect("optional field may be omitted");
        assert!(omitted.code_challenge_methods_supported.is_none());

        let mut explicit_null = metadata;
        explicit_null["code_challenge_methods_supported"] = serde_json::Value::Null;
        serde_json::from_value::<DiscoveryDocument>(explicit_null)
            .expect_err("a present null is malformed metadata, not omission");
    }

    #[test]
    fn diagnostic_requires_an_advertised_algorithm_and_matching_key() {
        let (config, _) = diagnostic_fixture(&["S256"], &["RS256"], vec![Algorithm::RS256]);
        let (_, no_advertised_allowlist) =
            diagnostic_fixture(&["S256"], &["ES256"], vec![Algorithm::RS256]);
        let error = validate_diagnostic(&config, &no_advertised_allowlist)
            .expect_err("discovery and configuration must intersect");
        assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);

        let (_, no_matching_key) =
            diagnostic_fixture(&["S256"], &["RS256"], vec![Algorithm::RS384]);
        let error = validate_diagnostic(&config, &no_matching_key)
            .expect_err("advertised algorithm needs a usable key");
        assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);
    }

    #[test]
    fn diagnostic_metadata_arrays_are_bounded_before_projection() {
        let (config, mut state) = diagnostic_fixture(&["S256"], &["RS256"], vec![Algorithm::RS256]);

        state.discovery.id_token_signing_alg_values_supported =
            vec!["RS256".to_owned(); MAX_DISCOVERY_VALUES + 1];
        let error = advertised_id_token_algorithms(&config, &state.discovery)
            .expect_err("oversized signing metadata must be refused before projection");
        assert_eq!(error.stage(), "signing-algorithm metadata");

        state.discovery.id_token_signing_alg_values_supported = vec!["RS256".to_owned()];
        state.discovery.response_types_supported =
            vec!["code".to_owned(); MAX_DISCOVERY_VALUES + 1];
        let error = validate_diagnostic(&config, &state)
            .expect_err("oversized response metadata must be refused");
        assert_eq!(error.stage(), "response-type metadata");

        state.discovery.response_types_supported = vec!["code".to_owned()];
        state.discovery.scopes_supported =
            Some(vec!["openid".to_owned(); MAX_ADVERTISED_SCOPES + 1]);
        let error = validate_diagnostic(&config, &state)
            .expect_err("oversized scope metadata must be refused before hashing");
        assert_eq!(error.stage(), "scope metadata");
    }

    #[tokio::test]
    async fn retryable_initialization_has_one_total_deadline() {
        let verifier = OidcVerifier::new(
            parse_issuers(
                r#"[{"issuer":"http://127.0.0.1:1/idp","client_id":"c","audience":"api"}]"#,
            )
            .expect("issuer fixture"),
        )
        .expect("verifier");
        let error = verifier
            .initialize_with_deadline(Duration::from_millis(20), Duration::from_millis(1))
            .await
            .expect_err("an unavailable issuer must stop at the total deadline");
        assert!(error.is_retryable());
        assert_eq!(error.stage(), "deadline");
    }

    #[test]
    fn discovery_algorithm_intersection_is_retained_for_id_token_verification() {
        let (mut config, mut state) = diagnostic_fixture(
            &["S256"],
            &["RS256"],
            vec![Algorithm::RS256, Algorithm::RS512],
        );
        config.algorithms = vec![Algorithm::RS256, Algorithm::RS512];
        state.id_token_algorithms = advertised_id_token_algorithms(&config, &state.discovery)
            .expect("bounded fixture metadata");
        assert_eq!(state.id_token_algorithms, [Algorithm::RS256]);
        assert!(!state.id_token_algorithms.contains(&Algorithm::RS512));
        validate_diagnostic(&config, &state).expect("RS256 remains usable");
    }

    #[test]
    fn diagnostic_requires_the_authorization_code_contract_and_explicit_offline_scope() {
        let (mut config, mut state) =
            diagnostic_fixture(&["S256"], &["RS256"], vec![Algorithm::RS256]);
        state.discovery.response_types_supported = vec!["token".to_owned()];
        assert!(validate_diagnostic(&config, &state).is_err());

        state.discovery.response_types_supported = vec!["code".to_owned()];
        state.discovery.grant_types_supported = Some(vec!["client_credentials".to_owned()]);
        assert!(validate_diagnostic(&config, &state).is_err());

        state.discovery.grant_types_supported = None;
        config.login_scopes.push("offline_access".to_owned());
        assert!(validate_diagnostic(&config, &state).is_err());
        state
            .discovery
            .scopes_supported
            .get_or_insert_default()
            .push("offline_access".to_owned());
        validate_diagnostic(&config, &state).expect("complete offline contract");
    }

    #[test]
    fn discovery_metadata_endpoints_obey_the_issuer_transport_policy() {
        for (accepted, transport) in [
            (
                "http://auth.example.test/authorize",
                MetadataEndpointTransport::InsecureDevelopmentHttp,
            ),
            (
                "http://127.0.0.1:8100/authorize",
                MetadataEndpointTransport::LoopbackHttpOnly,
            ),
            (
                "https://keys.example.test/jwks?version=2",
                MetadataEndpointTransport::HttpsOnly,
            ),
        ] {
            validate_metadata_endpoint(accepted, "test", transport).expect(accepted);
        }
        for refused in [
            "",
            "/relative",
            "file:///run/secrets/token",
            "https://user:secret@auth.example.test/token",
            "https://auth.example.test/jwks#fragment",
        ] {
            let error = validate_metadata_endpoint(
                refused,
                "test",
                MetadataEndpointTransport::InsecureDevelopmentHttp,
            )
            .expect_err("unsafe metadata endpoint must be refused");
            assert_eq!(error.disposition(), OidcDiagnosticDisposition::Refused);
            assert!(
                !error.to_string().contains("secret"),
                "credential value reached the error: {error}"
            );
        }
        for (endpoint, transport) in [
            (
                "http://auth.example.test/token",
                MetadataEndpointTransport::HttpsOnly,
            ),
            (
                "http://auth.example.test/token",
                MetadataEndpointTransport::LoopbackHttpOnly,
            ),
            (
                "http://127.0.0.1:8100/token",
                MetadataEndpointTransport::HttpsOnly,
            ),
        ] {
            let downgrade = validate_metadata_endpoint(endpoint, "test", transport)
                .expect_err("metadata endpoint transport must stay within issuer policy");
            assert_eq!(downgrade.disposition(), OidcDiagnosticDisposition::Refused);
        }
    }

    #[test]
    fn parsed_https_scheme_cannot_downgrade_even_when_the_raw_scheme_is_uppercase() {
        let issuer = validate_issuer_identifier("HTTPS://auth.example.test/realms/synveda")
            .expect("URL schemes are case-insensitive");
        assert_eq!(issuer.scheme(), "https");
        assert!(
            validate_metadata_endpoint(
                "http://auth.example.test/realms/synveda/protocol/openid-connect/token",
                "token endpoint",
                MetadataEndpointTransport::HttpsOnly,
            )
            .is_err()
        );
    }

    #[test]
    fn jwk_declared_use_operations_and_algorithm_must_allow_signature_verification() {
        let fixture = include_str!("../tests/fixtures/idp_rsa_2048.jwk.json");
        let valid: Jwk = serde_json::from_str(fixture).expect("JWK fixture");
        assert!(verifying_key(&valid, &[Algorithm::RS256]).is_some());

        let mut encryption_algorithm = valid.clone();
        encryption_algorithm.common.key_algorithm = Some(KeyAlgorithm::RSA_OAEP);
        assert!(verifying_key(&encryption_algorithm, &[Algorithm::RS256]).is_none());

        let mut encryption_use = valid.clone();
        encryption_use.common.public_key_use = Some(PublicKeyUse::Encryption);
        assert!(verifying_key(&encryption_use, &[Algorithm::RS256]).is_none());

        let mut encryption_operation = valid.clone();
        encryption_operation.common.key_operations = Some(vec![KeyOperations::Encrypt]);
        assert!(verifying_key(&encryption_operation, &[Algorithm::RS256]).is_none());

        let mut duplicate_operation = valid.clone();
        duplicate_operation.common.key_operations =
            Some(vec![KeyOperations::Verify, KeyOperations::Verify]);
        assert!(verifying_key(&duplicate_operation, &[Algorithm::RS256]).is_none());

        let mut invalid_exponent = valid.clone();
        if let AlgorithmParameters::RSA(parameters) = &mut invalid_exponent.algorithm {
            parameters.e = URL_SAFE_NO_PAD.encode([2]);
        }
        assert!(verifying_key(&invalid_exponent, &[Algorithm::RS256]).is_none());

        let mut weak_modulus = valid.clone();
        if let AlgorithmParameters::RSA(parameters) = &mut weak_modulus.algorithm {
            parameters.n = URL_SAFE_NO_PAD.encode([0xff; 128]);
        }
        assert!(verifying_key(&weak_modulus, &[Algorithm::RS256]).is_none());

        let mut oversized_modulus = valid.clone();
        if let AlgorithmParameters::RSA(parameters) = &mut oversized_modulus.algorithm {
            parameters.n = URL_SAFE_NO_PAD.encode([0xff; 513]);
        }
        assert!(verifying_key(&oversized_modulus, &[Algorithm::RS256]).is_none());

        let mut unspecified = valid;
        unspecified.common.public_key_use = None;
        unspecified.common.key_operations = None;
        unspecified.common.key_algorithm = None;
        assert!(verifying_key(&unspecified, &[Algorithm::RS256]).is_some());
    }

    #[test]
    fn id_token_authorized_party_is_required_for_multiple_audiences() {
        validate_id_token_authorized_party(&serde_json::json!({"aud": "synveda"}), "synveda")
            .expect("single audience needs no azp");
        validate_id_token_authorized_party(
            &serde_json::json!({"aud": ["synveda", "account"], "azp": "synveda"}),
            "synveda",
        )
        .expect("matching multi-audience azp");
        for claims in [
            serde_json::json!({"aud": ["synveda", "account"]}),
            serde_json::json!({"aud": ["synveda", "account"], "azp": "other"}),
            serde_json::json!({"aud": "synveda", "azp": "other"}),
            serde_json::json!({"aud": "synveda", "azp": 7}),
        ] {
            assert!(
                validate_id_token_authorized_party(&claims, "synveda").is_err(),
                "accepted {claims}"
            );
        }
    }

    #[test]
    fn id_token_issuer_is_exactly_one_matching_string() {
        let expected = "https://auth.example.test/realms/synveda";
        validate_id_token_issuer(&serde_json::json!({"iss": expected}), expected)
            .expect("exact StringOrURI issuer");
        for claims in [
            serde_json::json!({}),
            serde_json::json!({"iss": "https://other.example.test"}),
            serde_json::json!({"iss": [expected]}),
            serde_json::json!({"iss": [expected, "https://other.example.test"]}),
            serde_json::json!({"iss": {"value": expected}}),
            serde_json::json!({"iss": 7}),
            serde_json::json!({"iss": null}),
        ] {
            assert!(
                validate_id_token_issuer(&claims, expected).is_err(),
                "accepted {claims}"
            );
        }
    }

    #[test]
    fn oidc_time_claims_are_ordered_and_future_bounded() {
        let after_expiry = oidc_lifetime(&serde_json::json!({"exp": 1_060, "iat": 1_061}), 1_000)
            .expect_err("iat after exp must be rejected");
        assert!(matches!(after_expiry, Error::Unauthenticated { .. }));

        let too_future = oidc_lifetime(&serde_json::json!({"exp": 1_120, "iat": 1_031}), 1_000)
            .expect_err("iat beyond skew must be rejected");
        assert!(matches!(too_future, Error::Unauthenticated { .. }));

        assert_eq!(
            oidc_lifetime(&serde_json::json!({"exp": 1_090, "iat": 1_030}), 1_000)
                .expect("the skew boundary is valid"),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            oidc_lifetime(&serde_json::json!({"exp": 1_090}), 1_000)
                .expect("missing iat keeps an unknown lifetime"),
            None
        );
        for malformed in [
            serde_json::json!({"exp": "1090", "iat": 1_000}),
            serde_json::json!({"exp": 1_090, "iat": -1}),
        ] {
            let failure = oidc_lifetime(&malformed, 1_000)
                .expect_err("present malformed timestamps must fail closed");
            assert!(matches!(failure, Error::Unauthenticated { .. }));
        }
    }
}

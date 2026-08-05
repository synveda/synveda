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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use synveda_types::{Error, Result, TenantId};

use crate::token::{Claims, ProvisioningClaims, TokenVerifier};

/// Token verifications by issuer and outcome (`ok`, `rejected`, `error`).
/// Emitted here, described by the gateway's recorder (ADR-0007 layering).
pub const TOKEN_VERIFICATIONS_TOTAL: &str = "synveda_token_verifications_total";

/// JWKS refreshes by issuer and outcome (`ok`, `error`).
pub const JWKS_REFRESHES_TOTAL: &str = "synveda_jwks_refreshes_total";

/// Minimum interval between JWKS refetches per issuer (ADR-0010 §3).
const REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// Clock skew tolerated on `exp` and `nbf`.
const LEEWAY: Duration = Duration::from_secs(30);

/// The signature algorithms this build can verify. RS256 is what Entra,
/// Rauthy (as provisioned by us), and every mainstream IdP sign with; the
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
    /// the natural shape for a single-org IdP like the dev Rauthy.
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
    // provisioning claims. IdPs that gate the groups claim behind a scope
    // (Rauthy) add "groups" in config; IdPs that reject unknown scopes
    // (Entra) keep the default (ADR-0013 decision 1).
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
    /// Expected bearer-token audience. Defaults to `client_id`.
    #[serde(default)]
    pub audience: Option<String>,
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
        let mut audiences = vec![self.audience.as_deref().unwrap_or(&self.client_id)];
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
    /// RFC 8414's optional `scopes_supported`. Absent means the issuer
    /// published no list, which is not the same as publishing an empty
    /// one: both read as "do not ask for scopes it never advertised"
    /// (ADR-0027 decision 6).
    #[serde(default)]
    scopes_supported: Option<Vec<String>>,
}

/// A cached verification key: the decoded key plus the algorithms it may
/// verify (the per-issuer allowlist narrowed by the JWK's own `kty`/`alg`).
struct VerifyingKey {
    key: DecodingKey,
    algorithms: Vec<Algorithm>,
}

/// Immutable snapshot swapped wholesale on refresh; readers clone the `Arc`
/// under a short read lock and verify without further synchronisation.
pub(crate) struct IssuerState {
    pub(crate) discovery: DiscoveryDocument,
    keys: HashMap<String, VerifyingKey>,
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
        if configs.is_empty() {
            return Err(Error::Invalid {
                message: "at least one OIDC issuer must be configured".to_owned(),
            });
        }
        let mut issuers = HashMap::new();
        for config in configs {
            for algorithm in &config.algorithms {
                if !SUPPORTED_ALGORITHMS.contains(algorithm) {
                    return Err(Error::Invalid {
                        message: format!(
                            "issuer {}: algorithm {algorithm:?} is not supported \
                             (RS256/RS384/RS512 only, ADR-0010)",
                            config.issuer
                        ),
                    });
                }
            }
            if config.algorithms.is_empty() {
                return Err(Error::Invalid {
                    message: format!("issuer {}: empty algorithm list", config.issuer),
                });
            }
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
            let entry = IssuerEntry {
                config,
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

        let mut validation = Validation::new(header.alg);
        validation.leeway = LEEWAY.as_secs();
        validation.set_audience(audiences);
        validation.set_issuer(&[&entry.config.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        let data = decode::<serde_json::Value>(token, &key.key, &validation)
            .map_err(|err| unauthenticated(&verification_failure(&err)))?;
        let claims = data.claims;

        // jsonwebtoken validates nbf only when asked to require it; check it
        // ourselves so an IdP that sets it (Entra does) is honoured.
        if let Some(nbf) = claims.get("nbf").and_then(serde_json::Value::as_u64)
            && nbf > (now() + LEEWAY).as_secs()
        {
            return Err(unauthenticated("token not yet valid"));
        }

        // Bearer tokens without a subject fall back to `azp` — the
        // authorized party, i.e. the OAuth client the token was issued
        // to: client-credentials access tokens are minted *as* the
        // client, and some IdPs (Rauthy) set `sub: null` there
        // (ADR-0018 decision 1). ID tokens never fall back: OIDC
        // requires `sub` on them, and login must not admit a token
        // that names no end user.
        let claimed = |name: &str| {
            claims
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
        };
        let subject = claimed("sub")
            .or_else(|| expected_nonce.is_none().then(|| claimed("azp")).flatten())
            .ok_or_else(|| unauthenticated("token has no subject"))?
            .to_owned();

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

        // `exp` is validation-required above; `iat` is optional in the
        // spec, so a token without it has an unknown lifetime — the seam
        // fails closed on that for service identities (ADR-0018
        // decision 5).
        let lifetime = match (
            claims.get("exp").and_then(serde_json::Value::as_u64),
            claims.get("iat").and_then(serde_json::Value::as_u64),
        ) {
            (Some(exp), Some(iat)) => Some(Duration::from_secs(exp.saturating_sub(iat))),
            _ => None,
        };

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
        // Endpoints are stable per issuer; refetch discovery only on first
        // use, keys on every refresh.
        let discovery = match cached(entry) {
            Some(state) => state.discovery.clone(),
            None => self.fetch_discovery(&entry.config.issuer).await?,
        };
        let jwks: JwkSet = self.fetch_json(&discovery.jwks_uri, "oidc-jwks").await?;
        let mut keys = HashMap::new();
        for jwk in &jwks.keys {
            let Some(kid) = jwk.common.key_id.clone() else {
                continue;
            };
            let Some(verifying) = verifying_key(jwk, &entry.config.algorithms) else {
                tracing::debug!(
                    kid,
                    "skipping JWKS key: unusable under the issuer allowlist"
                );
                continue;
            };
            keys.insert(kid, verifying);
        }
        if keys.is_empty() {
            return Err(Error::Dependency {
                service: "oidc-jwks".to_owned(),
                message: format!(
                    "issuer {} published no usable verification keys",
                    entry.config.issuer
                ),
            });
        }
        Ok(IssuerState { discovery, keys })
    }

    #[tracing::instrument(name = "oidc.discovery", skip_all, fields(oidc.issuer = %issuer))]
    async fn fetch_discovery(&self, issuer: &str) -> Result<DiscoveryDocument> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let document: DiscoveryDocument = self.fetch_json(&url, "oidc-discovery").await?;
        // Byte-for-byte per ADR-0010: a document that names another issuer
        // is misconfiguration or an attack, never something to normalise.
        if document.issuer != issuer {
            return Err(Error::Dependency {
                service: "oidc-discovery".to_owned(),
                message: format!(
                    "discovery document names issuer {:?}, configured {issuer:?}",
                    document.issuer
                ),
            });
        }
        Ok(document)
    }

    async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        service: &str,
    ) -> Result<T> {
        let dependency = |message: String| Error::Dependency {
            service: service.to_owned(),
            message,
        };
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|err| dependency(format!("GET {url}: {err}")))?;
        if !response.status().is_success() {
            return Err(dependency(format!("GET {url}: HTTP {}", response.status())));
        }
        response
            .json()
            .await
            .map_err(|err| dependency(format!("GET {url}: invalid body: {err}")))
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

/// Builds the cached key for one JWK, or `None` if the key cannot verify
/// anything under the issuer's algorithm allowlist.
fn verifying_key(jwk: &Jwk, allowed: &[Algorithm]) -> Option<VerifyingKey> {
    let family: &[Algorithm] = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => &SUPPORTED_ALGORITHMS,
        // EC/OKP/oct keys verify algorithms this build does not compile.
        _ => return None,
    };
    // Narrow by the JWK's own alg when it declares one.
    let declared = jwk.common.key_algorithm.and_then(|ka| match ka {
        KeyAlgorithm::RS256 => Some(Algorithm::RS256),
        KeyAlgorithm::RS384 => Some(Algorithm::RS384),
        KeyAlgorithm::RS512 => Some(Algorithm::RS512),
        _ => None,
    });
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
    ProvisioningClaims {
        groups,
        email: text("email"),
        display_name: text("name"),
        external_id: text(external_id_claim),
    }
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
        let configs =
            parse_issuers(r#"[{"issuer":"http://localhost:8100/auth/v1","client_id":"synveda"}]"#)
                .expect("parse");
        assert_eq!(configs.len(), 1);
        let config = &configs[0];
        assert_eq!(config.bearer_audiences(), ["synveda"]);
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
            r#"[{"issuer":"http://idp","client_id":"c","login_scopes":["profile","groups"]}]"#,
        )
        .expect("parse");
        let err = OidcVerifier::new(configs)
            .err()
            .expect("scopes without openid must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn provisioning_claims_harvest_groups_email_and_name() {
        let claims = serde_json::json!({
            "sub": "alice",
            "groups": ["synveda-eng-platform", "", 42, "everyone"],
            "email": "alice@example.test",
            "name": "Alice Example",
        });
        let harvested = provisioning_claims(&claims, "groups", "sub");
        assert_eq!(harvested.groups, ["synveda-eng-platform", "everyone"]);
        assert_eq!(harvested.email.as_deref(), Some("alice@example.test"));
        assert_eq!(harvested.display_name.as_deref(), Some("Alice Example"));
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
                 "service_audiences":["ci-agent","api://agents"]}]"#,
        )
        .expect("parse");
        // Bearer tokens may carry the login client's audience or any
        // service audience; ID-token verification passes client_id alone
        // (ADR-0018 decision 1).
        assert_eq!(
            configs[0].bearer_audiences(),
            ["synveda", "ci-agent", "api://agents"]
        );
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        let err = parse_issuers(r#"[{"issuer":"http://idp","client_id":"c","cliett_id":"typo"}]"#)
            .expect_err("typo must not parse");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn unsupported_algorithms_are_rejected_at_construction() {
        let configs =
            parse_issuers(r#"[{"issuer":"http://idp","client_id":"c","algorithms":["HS256"]}]"#)
                .expect("HS256 parses as an algorithm name");
        let err = OidcVerifier::new(configs)
            .err()
            .expect("HS256 must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[test]
    fn duplicate_issuers_are_rejected() {
        let configs = parse_issuers(
            r#"[{"issuer":"http://idp","client_id":"a"},
                {"issuer":"http://idp","client_id":"b"}]"#,
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
        let one =
            OidcVerifier::new(parse_issuers(r#"[{"issuer":"http://a","client_id":"c"}]"#).unwrap())
                .unwrap();
        assert_eq!(one.sole_issuer(), Some("http://a"));
        let two = OidcVerifier::new(
            parse_issuers(
                r#"[{"issuer":"http://a","client_id":"c"},
                    {"issuer":"http://b","client_id":"c"}]"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(two.sole_issuer(), None);
    }

    #[tokio::test]
    async fn tokens_from_unknown_issuers_are_rejected_without_io() {
        let verifier =
            OidcVerifier::new(parse_issuers(r#"[{"issuer":"http://a","client_id":"c"}]"#).unwrap())
                .unwrap();
        let payload = URL_SAFE_NO_PAD.encode(r#"{"iss":"http://evil","sub":"x"}"#);
        let err = verifier
            .verify(&format!("h.{payload}.s"))
            .await
            .expect_err("unknown issuer must be rejected");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }
}

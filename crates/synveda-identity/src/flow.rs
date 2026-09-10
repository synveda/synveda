//! The authorization-code + PKCE login flow (AUTH-1, ADR-0010 §5), and the
//! CLI-mediated variant of it that `synveda login` drives (ADPT-1,
//! ADR-0027 decisions 5 and 6).
//!
//! [`LoginFlow::begin`] builds the IdP authorization URL (S256 challenge,
//! `state`, `nonce`) and parks a [`PendingLogin`]; [`LoginFlow::complete`]
//! consumes the `state` (single use — replay is a 401), exchanges the code
//! at the token endpoint with the `code_verifier`, and verifies the ID
//! token including the nonce. Pending logins live in a bounded in-memory
//! store with a 10-minute TTL: single-replica only until OPS-2 (ADR-0010).
//!
//! A CLI-initiated login carries a [`CliHandoff`] through the same flow
//! untouched — the point of ADR-0027 decision 5 is that `synveda login`
//! reuses AUTH-1 end to end rather than talking to the IdP itself, so
//! there is no second code path here, only a parked return address. What
//! the gateway does with it afterwards ([`LoginFlow::park_handoff`],
//! [`LoginFlow::redeem_handoff`]) is a one-time, 60-second, state-bound
//! code — the only thing that ever travels to the loopback listener.
//!
//! Tenant activeness is deliberately not checked here: the store tier is a
//! sibling crate (seed §8), so the gateway's callback handler runs TEN-1's
//! active-tenant lookup on the returned claims. For the same reason the
//! handoff payload is opaque `serde_json::Value` here: what a session
//! contains — tenant, placement — is the gateway tier's vocabulary.

use std::collections::HashMap;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use synveda_types::{Error, Result};

use crate::oidc::OidcVerifier;
use crate::token::Claims;

/// Login flows by issuer and outcome (`started`, `completed`, `rejected`,
/// `error`). An AUD-1 emission point (ADR-0010 compliance notes).
pub const OIDC_LOGINS_TOTAL: &str = "synveda_oidc_logins_total";

/// Refresh-token redemptions by issuer and outcome (`completed`,
/// `rejected`, `error`) — ADR-0027 decision 6. The gateway is the OAuth
/// client, so this is where "log in once" is actually kept true.
pub const OIDC_REFRESHES_TOTAL: &str = "synveda_oidc_refreshes_total";

/// A scope requested only when an issuer entry explicitly configures it and
/// the destination can retain a refresh token — the CLI (ADR-0027 decision 6,
/// as amended by ADR-0102) and the console (ADR-0056 decision 3). Provider-wide
/// discovery advertising is necessary but cannot grant this client the scope.
const OFFLINE_ACCESS: &str = "offline_access";

/// Token responses contain a handful of compact credentials and claims. A
/// larger body is a provider failure, not useful login data.
const MAX_TOKEN_RESPONSE_BYTES: usize = 65_536;

/// How long a pending login may wait between redirect and callback.
const PENDING_TTL: Duration = Duration::from_secs(600);

/// How long a handoff code may wait between the loopback redirect and its
/// redemption (ADR-0027 decision 5). The CLI redeems it in the same
/// breath; a minute is generous for that and short for anything else.
const HANDOFF_TTL: Duration = Duration::from_secs(60);

/// Upper bound on parked logins; beyond it new logins are rate-limited
/// rather than letting an unauthenticated caller grow memory.
const PENDING_CAP: usize = 10_000;

/// The fixed path a CLI loopback listener must serve (ADR-0027 decision 5).
const CLI_REDIRECT_PATH: &str = "/callback";

/// `synveda login` mints 32 random bytes and encodes them without padding.
/// The gateway accepts exactly that closed state shape rather than parking an
/// attacker-controlled, unbounded callback value.
const CLI_STATE_LENGTH: usize = 43;

struct PendingLogin {
    issuer: String,
    code_verifier: String,
    nonce: String,
    expires_at: Instant,
    /// Where to hand the completed session back.
    destination: LoginDestination,
}

impl PendingLogin {
    fn matches_correlation(&self, presented_secret: Option<&str>) -> bool {
        match &self.destination {
            LoginDestination::Console(binding) => binding.matches(presented_secret),
            LoginDestination::Json | LoginDestination::Cli(_) => true,
        }
    }
}

/// Where a completed login is delivered. The OIDC exchange is identical for
/// all three — same PKCE, same JWKS verification, same TEN-1 active-tenant
/// rule, same AUTH-2 provisioning — and this decides only the last step.
///
/// An enum rather than a pair of optional fields, on ADR-0027 decision 6's
/// own reasoning: a struct with `cli: Option<..>` beside `console: bool`
/// can represent "a CLI login that is also a console login", and the way
/// that gets fixed is somebody noticing. Here it cannot be written.
#[derive(Clone)]
pub enum LoginDestination {
    /// AUTH-1's browser login (ADR-0010 §1): the session comes back as
    /// JSON on the callback response.
    Json,
    /// ADPT-1's CLI login (ADR-0027 decision 5): a one-time, state-bound
    /// code 302'd to the CLI's loopback listener; the session material
    /// waits on the gateway until the CLI redeems it.
    Cli(CliHandoff),
    /// CNSL-1's console login (ADR-0056 decision 2): the gateway keeps the
    /// tokens, sets an `HttpOnly` cookie naming them, and 302s into the
    /// app. Carries no return address — the console is served from the
    /// gateway's own origin, so there is nowhere else for it to land, and
    /// an operator-supplied redirect target here would be an open
    /// redirector attached to a login.
    Console(ConsoleLoginBinding),
}

/// Digest-only binding between a console login and the browser that started
/// it.
///
/// The gateway gives the browser the independent random secret in a
/// host-only cookie and parks only its SHA-256 here. It is therefore not
/// enough to learn or forward an OIDC callback URL: the callback must also
/// arrive from the initiating browser.
#[derive(Clone)]
pub struct ConsoleLoginBinding {
    correlation_hash: [u8; 32],
}

impl ConsoleLoginBinding {
    /// Binds a pending console login to the hash of its browser secret.
    pub const fn new(correlation_hash: [u8; 32]) -> Self {
        Self { correlation_hash }
    }

    fn matches(&self, presented_secret: Option<&str>) -> bool {
        let Some(presented_hash) = presented_secret.and_then(crate::console::presented_hash) else {
            return false;
        };
        // Fixed-work comparison. The input is already a SHA-256 digest, but
        // avoiding an early-exit equality keeps the binding independent of
        // optimiser/library comparison details.
        self.correlation_hash
            .iter()
            .zip(presented_hash)
            .fold(0u8, |difference, (expected, actual)| {
                difference | (*expected ^ actual)
            })
            == 0
    }
}

/// A CLI-initiated login's return address (ADR-0027 decision 5): the
/// loopback URI `synveda login` is listening on, and the CSRF state it
/// minted. Both round-trip through the IdP untouched — the gateway parks
/// them, and only ever sends a one-time code back to that address.
#[derive(Clone)]
pub struct CliHandoff {
    /// Loopback return URI, already checked against the allowlist by
    /// [`validate_cli_redirect_uri`].
    pub redirect_uri: String,
    /// The CLI's own random state. It comes back on the loopback redirect
    /// so the CLI can tell its own callback from any other local process
    /// hitting its listener, and it is required again at redemption.
    pub state: String,
}

impl fmt::Debug for LoginDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json => formatter.write_str("Json"),
            Self::Cli(handoff) => formatter.debug_tuple("Cli").field(handoff).finish(),
            Self::Console(_) => formatter.write_str("Console(<redacted>)"),
        }
    }
}

impl fmt::Debug for CliHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliHandoff")
            .field("redirect_uri", &self.redirect_uri)
            .field("state", &"<redacted>")
            .finish()
    }
}

/// The `cli_redirect_uri` allowlist (ADR-0027 decision 5), and it is
/// absolute: scheme `http`, host literal `127.0.0.1` or `[::1]`, any port,
/// the fixed path `/callback`, and nothing else — no userinfo, no query,
/// no fragment. The `localhost` *name* is deliberately not accepted: it
/// resolves through the resolver, and a resolver is something an attacker
/// can sometimes reach.
pub fn validate_cli_redirect_uri(raw: &str) -> Result<()> {
    let refuse = |why: &str| {
        Err(Error::Invalid {
            message: format!("cli_redirect_uri {why} (ADR-0027 decision 5)"),
        })
    };
    let Ok(url) = url::Url::parse(raw) else {
        return refuse("is not a URL");
    };
    if url.scheme() != "http" {
        return refuse("must use the http scheme");
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) if ip == Ipv4Addr::LOCALHOST => {}
        Some(url::Host::Ipv6(ip)) if ip == Ipv6Addr::LOCALHOST => {}
        _ => return refuse("must be the literal loopback host 127.0.0.1 or [::1]"),
    }
    if url.path() != CLI_REDIRECT_PATH {
        return refuse("must have the path /callback");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return refuse("must carry no userinfo");
    }
    if url.query().is_some() || url.fragment().is_some() {
        return refuse("must carry no query or fragment");
    }
    Ok(())
}

/// The completed login: verified claims plus the session material the
/// caller uses as its bearer token (ADR-0010 §1).
pub struct LoginSession {
    /// Verified subject and tenant from the ID token.
    pub claims: Claims,
    /// The issuer that authenticated this login. The CLI stores it so a
    /// later refresh names the same token endpoint and client.
    pub issuer: String,
    /// The IdP-issued access token — the `/v1` bearer credential.
    pub access_token: String,
    /// Token type as reported by the IdP (`Bearer`).
    pub token_type: String,
    /// Seconds until the access token expires, when the IdP reports it.
    pub expires_in: Option<u64>,
    /// The refresh token, when the issuer grants one. It leaves the
    /// gateway on the CLI handoff exchange and nowhere else (ADR-0027
    /// decision 6) — never in the browser-facing response. A console login
    /// keeps it on the server entirely (ADR-0056 decision 3).
    pub refresh_token: Option<String>,
    /// Where this login asked to be delivered.
    pub destination: LoginDestination,
}

impl fmt::Debug for LoginSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginSession")
            .field("claims", &"<verified>")
            .field("issuer", &self.issuer)
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("destination", &self.destination)
            .finish()
    }
}

/// A refreshed credential (ADR-0027 decision 6). No claims: a refresh
/// response need not carry an ID token, and the access token is verified
/// where every bearer is — at the `/v1` seam, on the next request.
pub struct RefreshedSession {
    /// The new bearer.
    pub access_token: String,
    /// Token type as reported by the IdP (`Bearer`).
    pub token_type: String,
    /// Seconds until the new access token expires, when reported.
    pub expires_in: Option<u64>,
    /// The rotated refresh token, for issuers that rotate them. Absent
    /// means the caller keeps the one it has.
    pub refresh_token: Option<String>,
}

impl fmt::Debug for RefreshedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshedSession")
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("expires_in", &self.expires_in)
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Session material parked under a one-time handoff code.
struct Handoff {
    /// The CLI state this code is bound to: a code alone redeems nothing.
    state: String,
    payload: serde_json::Value,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn decode_token_response(body: &[u8]) -> Result<TokenResponse> {
    let mut tokens: TokenResponse =
        serde_json::from_slice(body).map_err(|_| Error::Dependency {
            service: "oidc-token-endpoint".to_owned(),
            message: "token response is not valid JSON".to_owned(),
        })?;
    if !tokens.token_type.eq_ignore_ascii_case("Bearer") {
        return Err(Error::Dependency {
            service: "oidc-token-endpoint".to_owned(),
            message: "token response did not return a Bearer credential".to_owned(),
        });
    }
    // Every downstream Synveda client uses the RFC 6750 Bearer scheme. Keep
    // one canonical spelling rather than persisting provider casing.
    tokens.token_type = "Bearer".to_owned();
    Ok(tokens)
}

/// Drives the code+PKCE flow against the verifier's configured issuers.
pub struct LoginFlow {
    verifier: Arc<OidcVerifier>,
    redirect_uri: String,
    pending: Mutex<HashMap<String, PendingLogin>>,
    handoffs: Mutex<HashMap<String, Handoff>>,
}

impl LoginFlow {
    /// Builds a flow that sends callbacks to `redirect_uri`
    /// (`{SYNVEDA_PUBLIC_URL}/auth/callback`).
    pub fn new(verifier: Arc<OidcVerifier>, redirect_uri: String) -> Self {
        Self {
            verifier,
            redirect_uri,
            pending: Mutex::new(HashMap::new()),
            handoffs: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves the issuer to run a flow against: the named one, or the
    /// sole configured one when the caller named none.
    fn resolve_issuer(&self, issuer: Option<&str>) -> Result<String> {
        match (issuer, self.verifier.sole_issuer()) {
            (Some(named), _) => Ok(self.verifier.config(named)?.issuer.clone()),
            (None, Some(sole)) => Ok(sole.to_owned()),
            (None, None) => Err(Error::Invalid {
                message: "several issuers are configured; pass ?issuer=".to_owned(),
            }),
        }
    }

    /// Starts a login and returns the IdP authorization URL to redirect to.
    /// `issuer` may be omitted when exactly one is configured.
    /// `destination` decides only where the completed session is handed
    /// back (ADR-0027 decision 5, ADR-0056 decision 2); the OIDC exchange
    /// is identical for all three.
    pub async fn begin(
        &self,
        issuer: Option<&str>,
        destination: LoginDestination,
    ) -> Result<String> {
        if let LoginDestination::Cli(handoff) = &destination {
            validate_cli_redirect_uri(&handoff.redirect_uri)?;
            if !valid_cli_state(&handoff.state) {
                return Err(Error::Invalid {
                    message: "cli_state must be a 43-character base64url value".to_owned(),
                });
            }
        }
        let issuer = self.resolve_issuer(issuer)?;
        let state = random_urlsafe()?;
        let nonce = random_urlsafe()?;
        let code_verifier = random_urlsafe()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

        let issuer_state = self.verifier.issuer_state(&issuer).await?;
        let config = self.verifier.config(&issuer)?;
        let client_id = config.client_id.clone();
        // Per-issuer (ADR-0013 decision 1): IdPs that gate the groups claim
        // behind a scope add it in config; the default stays universal.
        let mut scope_list = config.login_scopes.clone();
        // Discovery describes provider-wide scopes, not what this client is
        // authorised to request. The configured list is therefore
        // authoritative: a bundled Keycloak realm can advertise
        // `offline_access` while Synveda's public client deliberately has it
        // disabled. A JSON browser login cannot retain a refresh token and
        // strips an explicitly configured offline scope. CLI/console may ask
        // for it only when both configuration and discovery agree.
        if matches!(destination, LoginDestination::Json) {
            scope_list.retain(|scope| scope != OFFLINE_ACCESS);
        } else if scope_list.iter().any(|scope| scope == OFFLINE_ACCESS)
            && !issuer_state.advertises_scope(OFFLINE_ACCESS)
        {
            return Err(Error::Dependency {
                service: "oidc-discovery".to_owned(),
                message: format!(
                    "issuer {issuer}: configured offline_access is not advertised by discovery"
                ),
            });
        }
        let requests_offline_access = scope_list.iter().any(|scope| scope == OFFLINE_ACCESS);
        let scopes = scope_list.join(" ");
        let mut url =
            url::Url::parse(&issuer_state.discovery.authorization_endpoint).map_err(|err| {
                Error::Dependency {
                    service: "oidc-discovery".to_owned(),
                    message: format!("issuer {issuer}: bad authorization_endpoint: {err}"),
                }
            })?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &client_id)
                .append_pair("redirect_uri", &self.redirect_uri)
                .append_pair("scope", &scopes)
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            if requests_offline_access {
                // OIDC Core requires explicit consent when requesting the
                // offline scope; without it some providers silently omit the
                // refresh contract while others refuse the request.
                query.append_pair("prompt", "consent");
            }
        }

        {
            let mut pending = self.pending.lock().expect("pending login lock");
            let now = Instant::now();
            pending.retain(|_, login| login.expires_at > now);
            if pending.len() >= PENDING_CAP {
                return Err(Error::RateLimited {
                    message: "too many logins in flight; retry shortly".to_owned(),
                });
            }
            pending.insert(
                state,
                PendingLogin {
                    issuer: issuer.clone(),
                    code_verifier,
                    nonce,
                    expires_at: now + PENDING_TTL,
                    destination,
                },
            );
        }
        metrics::counter!(OIDC_LOGINS_TOTAL, "issuer" => issuer, "outcome" => "started")
            .increment(1);
        Ok(url.into())
    }

    /// The destination parked under `state`, without consuming the pending
    /// login. The gateway reads it *before* completing so that every way a
    /// login can fail still lands where the caller is waiting — the
    /// terminal `synveda login` is sitting in (ADR-0027 decision 5), or the
    /// console's own error page rather than a JSON body rendered into a
    /// browser tab (ADR-0056). A correctly bound expired entry is still
    /// visible here so its terminal callback can clear the browser cookie;
    /// [`Self::complete`] remains the authority that refuses its expiry.
    pub fn peek_destination(
        &self,
        state: &str,
        presented_correlation: Option<&str>,
    ) -> Option<LoginDestination> {
        let pending = self.pending.lock().expect("pending login lock");
        pending
            .get(state)
            .filter(|login| login.matches_correlation(presented_correlation))
            .map(|login| login.destination.clone())
    }

    /// Discards a pending login without completing it — the IdP-reported
    /// failure path, where there is no code to exchange and nothing to
    /// keep parked for ten minutes.
    pub fn abandon(&self, state: &str, presented_correlation: Option<&str>) -> bool {
        let mut pending = self.pending.lock().expect("pending login lock");
        if !pending
            .get(state)
            .is_some_and(|login| login.matches_correlation(presented_correlation))
        {
            return false;
        }
        pending.remove(state).is_some()
    }

    /// Completes a login from the IdP callback. `state` is single-use;
    /// replaying it — or presenting one the gateway never issued — is a 401.
    pub async fn complete(
        &self,
        state: &str,
        code: &str,
        presented_correlation: Option<&str>,
    ) -> Result<LoginSession> {
        let login = {
            let mut pending = self.pending.lock().expect("pending login lock");
            if !pending
                .get(state)
                .is_some_and(|login| login.matches_correlation(presented_correlation))
            {
                None
            } else {
                pending.remove(state)
            }
        }
        .ok_or_else(|| Error::Unauthenticated {
            message: "unknown or already-used login state".to_owned(),
        })?;
        let issuer = login.issuer.clone();
        let outcome = self.complete_inner(login, code).await;
        let label = match &outcome {
            Ok(_) => "completed",
            Err(Error::Unauthenticated { .. }) => "rejected",
            Err(_) => "error",
        };
        metrics::counter!(OIDC_LOGINS_TOTAL, "issuer" => issuer, "outcome" => label).increment(1);
        outcome
    }

    async fn complete_inner(&self, login: PendingLogin, code: &str) -> Result<LoginSession> {
        if login.expires_at <= Instant::now() {
            return Err(Error::Unauthenticated {
                message: "login expired; start again".to_owned(),
            });
        }
        let tokens = self.exchange(&login, code).await?;
        let id_token = tokens.id_token.ok_or_else(|| Error::Dependency {
            service: "oidc-token-endpoint".to_owned(),
            message: "token response carried no id_token (openid scope missing?)".to_owned(),
        })?;
        let claims = self
            .verifier
            .verify_id_token(&login.issuer, &id_token, &login.nonce)
            .await?;
        Ok(LoginSession {
            claims,
            issuer: login.issuer,
            access_token: tokens.access_token,
            token_type: tokens.token_type,
            expires_in: tokens.expires_in,
            refresh_token: tokens.refresh_token,
            destination: login.destination,
        })
    }

    /// Redeems a refresh token for a fresh access token (ADR-0027
    /// decision 6). The gateway stays the OAuth client — that is precisely
    /// what lets `synveda login` hold no client id, no client secret, and
    /// no per-issuer configuration at all.
    pub async fn refresh(
        &self,
        issuer: Option<&str>,
        refresh_token: &str,
    ) -> Result<RefreshedSession> {
        let issuer = self.resolve_issuer(issuer)?;
        let outcome = self.refresh_inner(&issuer, refresh_token).await;
        let label = match &outcome {
            Ok(_) => "completed",
            Err(Error::Unauthenticated { .. }) => "rejected",
            Err(_) => "error",
        };
        metrics::counter!(OIDC_REFRESHES_TOTAL, "issuer" => issuer, "outcome" => label)
            .increment(1);
        outcome
    }

    #[tracing::instrument(name = "oidc.refresh", skip_all, fields(oidc.issuer = %issuer))]
    async fn refresh_inner(&self, issuer: &str, refresh_token: &str) -> Result<RefreshedSession> {
        let client_id = self.verifier.config(issuer)?.client_id.clone();
        let tokens = self
            .post_token(
                issuer,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token),
                    ("client_id", &client_id),
                ],
            )
            .await?;
        Ok(RefreshedSession {
            access_token: tokens.access_token,
            token_type: tokens.token_type,
            expires_in: tokens.expires_in,
            refresh_token: tokens.refresh_token,
        })
    }

    /// Parks completed session material under a one-time handoff code and
    /// returns the code (ADR-0027 decision 5). `state` is the CLI's own,
    /// and redemption requires it: the code that travels to the loopback
    /// listener is worth nothing to anything that did not start the login.
    pub fn park_handoff(&self, state: &str, payload: serde_json::Value) -> Result<String> {
        let code = random_urlsafe()?;
        let mut handoffs = self.handoffs.lock().expect("handoff lock");
        let now = Instant::now();
        handoffs.retain(|_, handoff| handoff.expires_at > now);
        if handoffs.len() >= PENDING_CAP {
            return Err(Error::RateLimited {
                message: "too many logins in flight; retry shortly".to_owned(),
            });
        }
        handoffs.insert(
            code.clone(),
            Handoff {
                state: state.to_owned(),
                payload,
                expires_at: now + HANDOFF_TTL,
            },
        );
        Ok(code)
    }

    /// Redeems a handoff code for its parked session material. Single use
    /// — the entry is removed on the first attempt, valid or not — 60
    /// seconds, and bound to the state the CLI minted.
    pub fn redeem_handoff(&self, code: &str, state: &str) -> Result<serde_json::Value> {
        let rejected = || Error::Unauthenticated {
            message: "unknown, expired, or already-redeemed handoff code".to_owned(),
        };
        let handoff = {
            let mut handoffs = self.handoffs.lock().expect("handoff lock");
            handoffs.remove(code)
        }
        .ok_or_else(rejected)?;
        if handoff.expires_at <= Instant::now() || handoff.state != state {
            return Err(rejected());
        }
        Ok(handoff.payload)
    }

    /// The authorization-code exchange: a public client authenticating the
    /// redemption with the PKCE verifier alone (no client secret).
    #[tracing::instrument(name = "oidc.exchange", skip_all, fields(oidc.issuer = %login.issuer))]
    async fn exchange(&self, login: &PendingLogin, code: &str) -> Result<TokenResponse> {
        let client_id = self.verifier.config(&login.issuer)?.client_id.clone();
        self.post_token(
            &login.issuer,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &client_id),
                ("code_verifier", &login.code_verifier),
            ],
        )
        .await
    }

    /// One POST to an issuer's token endpoint, with the failure taxonomy
    /// both grants share: a 4xx is the caller's problem (a bad code, a
    /// revoked refresh token) and stays a uniform 401; anything else is
    /// the IdP's.
    async fn post_token(&self, issuer: &str, form: &[(&str, &str)]) -> Result<TokenResponse> {
        let issuer_state = self.verifier.issuer_state(issuer).await?;
        let response = self
            .verifier
            .http()
            .post(&issuer_state.discovery.token_endpoint)
            .form(form)
            .send()
            .await
            .map_err(|_| Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: "token endpoint request failed".to_owned(),
            })?;
        let status = response.status();
        if matches!(status.as_u16(), 408 | 425 | 429) {
            return Err(Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: "token endpoint is temporarily unavailable".to_owned(),
            });
        }
        if status.is_client_error() {
            // The IdP refused the grant — a caller problem. Its body is an
            // untrusted content channel and is neither read nor traced.
            tracing::debug!(%status, "token exchange refused");
            return Err(Error::Unauthenticated {
                message: "the identity provider rejected the login".to_owned(),
            });
        }
        if !status.is_success() {
            return Err(Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: format!("token exchange: HTTP {status}"),
            });
        }
        let mut response = response;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_TOKEN_RESPONSE_BYTES as u64)
        {
            return Err(Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: "token response exceeds the byte bound".to_owned(),
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Dependency {
            service: "oidc-token-endpoint".to_owned(),
            message: "token response body failed".to_owned(),
        })? {
            let remaining = MAX_TOKEN_RESPONSE_BYTES.saturating_sub(body.len());
            if chunk.len() > remaining {
                return Err(Error::Dependency {
                    service: "oidc-token-endpoint".to_owned(),
                    message: "token response exceeds the byte bound".to_owned(),
                });
            }
            body.extend_from_slice(&chunk);
        }
        decode_token_response(&body)
    }
}

/// 32 bytes of CSPRNG entropy, base64url — used for `state`, `nonce`, and
/// the PKCE `code_verifier` (43 chars, within RFC 7636's 43–128).
fn random_urlsafe() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| Error::Internal {
        message: format!("system CSPRNG unavailable: {err}"),
    })?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn valid_cli_state(state: &str) -> bool {
    state.len() == CLI_STATE_LENGTH
        && state
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc::parse_issuers;

    fn flow() -> LoginFlow {
        let verifier = Arc::new(
            OidcVerifier::new(
                parse_issuers(
                    r#"[{"issuer":"http://127.0.0.1:1/idp","client_id":"synveda","audience":"synveda-api"}]"#,
                )
                    .unwrap(),
            )
            .unwrap(),
        );
        LoginFlow::new(verifier, "http://127.0.0.1:8120/auth/callback".to_owned())
    }

    #[test]
    fn random_urlsafe_is_pkce_shaped_and_unique() {
        let a = random_urlsafe().unwrap();
        let b = random_urlsafe().unwrap();
        assert_eq!(a.len(), 43, "32 bytes must encode to 43 chars");
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[tokio::test]
    async fn unknown_login_state_is_rejected_without_io() {
        let err = flow()
            .complete("never-issued", "code", None)
            .await
            .expect_err("unknown state must be rejected");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn named_but_unconfigured_issuer_is_invalid() {
        let err = flow()
            .begin(Some("http://other-idp"), LoginDestination::Json)
            .await
            .expect_err("unconfigured issuer must be rejected");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    // ── The CLI loopback allowlist (ADR-0027 decision 5) ────────────────

    #[test]
    fn loopback_redirect_uris_are_accepted_on_any_port() {
        for uri in [
            "http://127.0.0.1:1/callback",
            "http://127.0.0.1:49152/callback",
            "http://[::1]:8080/callback",
        ] {
            validate_cli_redirect_uri(uri).unwrap_or_else(|err| panic!("{uri}: {err:?}"));
        }
    }

    #[test]
    fn every_other_redirect_target_is_refused() {
        for uri in [
            // The name, not the literal: it resolves, and resolution is
            // something an attacker can sometimes reach.
            "http://localhost:8080/callback",
            "https://127.0.0.1:8080/callback",
            "http://127.0.0.2:8080/callback",
            "http://10.0.0.1:8080/callback",
            "http://evil.test/callback",
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/callback/../elsewhere",
            "http://127.0.0.1:8080/callback?next=http://evil.test",
            "http://127.0.0.1:8080/callback#fragment",
            "http://user:pass@127.0.0.1:8080/callback",
            "file:///callback",
            "not-a-url",
            "",
        ] {
            let err =
                validate_cli_redirect_uri(uri).expect_err("must not be an accepted handoff target");
            assert!(matches!(err, Error::Invalid { .. }), "{uri}: got {err:?}");
        }
    }

    #[tokio::test]
    async fn a_disallowed_redirect_fails_before_any_io() {
        // The unreachable issuer proves the check runs first: a flow that
        // reached discovery would fail as a Dependency, not an Invalid.
        let err = flow()
            .begin(
                None,
                LoginDestination::Cli(CliHandoff {
                    redirect_uri: "http://evil.test/callback".to_owned(),
                    state: "s".to_owned(),
                }),
            )
            .await
            .expect_err("a non-loopback redirect must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn cli_state_is_one_exact_bounded_base64url_shape() {
        for state in [
            "",
            "short",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA!",
        ] {
            let err = flow()
                .begin(
                    None,
                    LoginDestination::Cli(CliHandoff {
                        redirect_uri: "http://127.0.0.1:49152/callback".to_owned(),
                        state: state.to_owned(),
                    }),
                )
                .await
                .expect_err("malformed CLI state must be refused before discovery");
            assert!(matches!(err, Error::Invalid { .. }), "{state:?}: {err:?}");
        }
    }

    #[test]
    fn completed_session_debug_output_redacts_credentials() {
        let login = LoginSession {
            claims: Claims {
                subject: "alice".to_owned(),
                tenant_id: synveda_types::TenantId::new(),
                provisioning: None,
                lifetime: Some(Duration::from_secs(60)),
                credential_class: crate::token::CredentialClass::Interactive,
            },
            issuer: "https://idp.example.test".to_owned(),
            access_token: "never-log-access-token".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: Some(60),
            refresh_token: Some("never-log-refresh-token".to_owned()),
            destination: LoginDestination::Cli(CliHandoff {
                redirect_uri: "http://127.0.0.1:49152/callback".to_owned(),
                state: "never-log-cli-state".to_owned(),
            }),
        };
        let refreshed = RefreshedSession {
            access_token: "never-log-new-access-token".to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: Some(60),
            refresh_token: Some("never-log-new-refresh-token".to_owned()),
        };
        let rendered = format!("{login:?} {refreshed:?}");
        assert!(!rendered.contains("never-log"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn token_responses_accept_only_canonicalizable_bearer_credentials() {
        for token_type in ["Bearer", "bearer", "BEARER"] {
            let body = serde_json::json!({
                "access_token": "never-log-token",
                "token_type": token_type,
            });
            let decoded = decode_token_response(body.to_string().as_bytes()).expect("Bearer");
            assert_eq!(decoded.token_type, "Bearer");
        }
        for body in [
            serde_json::json!({"access_token": "never-log-token", "token_type": "DPoP"}),
            serde_json::json!({"access_token": "never-log-token", "token_type": "MAC"}),
            serde_json::json!({"access_token": "never-log-token", "token_type": ""}),
            serde_json::json!({"access_token": "never-log-token"}),
        ] {
            let error = match decode_token_response(body.to_string().as_bytes()) {
                Ok(_) => panic!("non-Bearer token response must be refused"),
                Err(error) => error,
            };
            assert!(matches!(error, Error::Dependency { .. }), "{error:?}");
            assert!(!format!("{error:?}").contains("never-log-token"));
        }
    }

    #[tokio::test]
    async fn console_state_is_invisible_and_unconsumed_without_its_browser_secret() {
        let flow = flow();
        let correlation = crate::console::mint().expect("correlation");
        let state = "console-state";
        flow.pending.lock().expect("pending login lock").insert(
            state.to_owned(),
            PendingLogin {
                issuer: "http://127.0.0.1:1/idp".to_owned(),
                code_verifier: "verifier".to_owned(),
                nonce: "nonce".to_owned(),
                expires_at: Instant::now() + PENDING_TTL,
                destination: LoginDestination::Console(ConsoleLoginBinding::new(correlation.hash)),
            },
        );

        for wrong in [
            None,
            Some("malformed"),
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        ] {
            assert!(flow.peek_destination(state, wrong).is_none());
            assert!(!flow.abandon(state, wrong));
            let error = flow
                .complete(state, "code", wrong)
                .await
                .expect_err("wrong correlation must fail before exchange");
            assert!(matches!(error, Error::Unauthenticated { .. }));
        }

        assert!(matches!(
            flow.peek_destination(state, Some(&correlation.secret)),
            Some(LoginDestination::Console(_))
        ));
        assert!(flow.abandon(state, Some(&correlation.secret)));
        assert!(
            flow.peek_destination(state, Some(&correlation.secret))
                .is_none()
        );

        let expired = crate::console::mint().expect("expired correlation");
        flow.pending.lock().expect("pending login lock").insert(
            "expired-console-state".to_owned(),
            PendingLogin {
                issuer: "http://127.0.0.1:1/idp".to_owned(),
                code_verifier: "verifier".to_owned(),
                nonce: "nonce".to_owned(),
                expires_at: Instant::now() - Duration::from_secs(1),
                destination: LoginDestination::Console(ConsoleLoginBinding::new(expired.hash)),
            },
        );
        assert!(matches!(
            flow.peek_destination("expired-console-state", Some(&expired.secret)),
            Some(LoginDestination::Console(_))
        ));
        let error = flow
            .complete("expired-console-state", "code", Some(&expired.secret))
            .await
            .expect_err("expiry must be refused before token exchange");
        assert!(matches!(error, Error::Unauthenticated { .. }));
        assert!(
            flow.peek_destination("expired-console-state", Some(&expired.secret))
                .is_none(),
            "the terminal expired callback consumes its state"
        );
    }

    #[test]
    fn console_binding_debug_never_exposes_its_digest() {
        let correlation = crate::console::mint().expect("correlation");
        let rendered = format!(
            "{:?}",
            LoginDestination::Console(ConsoleLoginBinding::new(correlation.hash))
        );
        assert_eq!(rendered, "Console(<redacted>)");
        assert!(!rendered.contains(&correlation.secret));
    }

    // ── The handoff code (ADR-0027 decision 5) ──────────────────────────

    #[test]
    fn a_handoff_code_redeems_once_and_only_with_its_state() {
        let flow = flow();
        let payload = serde_json::json!({ "access_token": "at" });

        let code = flow
            .park_handoff("cli-state", payload.clone())
            .expect("park");
        let wrong = flow
            .redeem_handoff(&code, "another-state")
            .expect_err("a code alone must redeem nothing");
        assert!(
            matches!(wrong, Error::Unauthenticated { .. }),
            "got {wrong:?}"
        );
        // The mismatched attempt consumed it: a wrong state is a failed
        // redemption, not a free retry.
        let replay = flow
            .redeem_handoff(&code, "cli-state")
            .expect_err("a consumed code must not redeem");
        assert!(
            matches!(replay, Error::Unauthenticated { .. }),
            "got {replay:?}"
        );

        let code = flow
            .park_handoff("cli-state", payload.clone())
            .expect("park");
        assert_eq!(
            flow.redeem_handoff(&code, "cli-state").expect("redeem"),
            payload
        );
        let replay = flow
            .redeem_handoff(&code, "cli-state")
            .expect_err("single use");
        assert!(
            matches!(replay, Error::Unauthenticated { .. }),
            "got {replay:?}"
        );
    }

    #[test]
    fn an_expired_handoff_code_redeems_nothing() {
        let flow = flow();
        let code = flow
            .park_handoff("cli-state", serde_json::json!({}))
            .expect("park");
        // Reach past the clock: the TTL is a minute, and a test that slept
        // one would be a minute of nothing.
        flow.handoffs
            .lock()
            .expect("handoff lock")
            .get_mut(&code)
            .expect("parked")
            .expires_at = Instant::now() - Duration::from_secs(1);
        let err = flow
            .redeem_handoff(&code, "cli-state")
            .expect_err("an expired code must not redeem");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }

    #[test]
    fn unknown_handoff_codes_are_rejected() {
        let err = flow()
            .redeem_handoff("never-issued", "cli-state")
            .expect_err("unknown code must be rejected");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }
}

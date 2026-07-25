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

/// The scope that asks an issuer for a refresh token. Requested only for
/// CLI logins, and only where discovery advertises it (ADR-0027 decision
/// 6): an issuer that rejects unknown scopes must not have its browser
/// logins broken by a scope only the CLI needs.
const OFFLINE_ACCESS: &str = "offline_access";

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

struct PendingLogin {
    issuer: String,
    code_verifier: String,
    nonce: String,
    expires_at: Instant,
    /// Where to hand the completed session back, for a CLI-initiated
    /// login; `None` for a browser login, which reads it as JSON.
    cli: Option<CliHandoff>,
}

/// A CLI-initiated login's return address (ADR-0027 decision 5): the
/// loopback URI `synveda login` is listening on, and the CSRF state it
/// minted. Both round-trip through the IdP untouched — the gateway parks
/// them, and only ever sends a one-time code back to that address.
#[derive(Debug, Clone)]
pub struct CliHandoff {
    /// Loopback return URI, already checked against the allowlist by
    /// [`validate_cli_redirect_uri`].
    pub redirect_uri: String,
    /// The CLI's own random state. It comes back on the loopback redirect
    /// so the CLI can tell its own callback from any other local process
    /// hitting its listener, and it is required again at redemption.
    pub state: String,
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
#[derive(Debug)]
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
    /// decision 6) — never in the browser-facing response.
    pub refresh_token: Option<String>,
    /// The parked CLI return address, when this login was CLI-initiated.
    pub cli: Option<CliHandoff>,
}

/// A refreshed credential (ADR-0027 decision 6). No claims: a refresh
/// response need not carry an ID token, and the access token is verified
/// where every bearer is — at the `/v1` seam, on the next request.
#[derive(Debug)]
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
    /// `issuer` may be omitted when exactly one is configured. `cli` is
    /// `Some` when `synveda login` started this flow (ADR-0027 decision 5);
    /// it changes nothing about the OIDC exchange, only where the completed
    /// session is handed back.
    pub async fn begin(&self, issuer: Option<&str>, cli: Option<CliHandoff>) -> Result<String> {
        if let Some(handoff) = &cli {
            validate_cli_redirect_uri(&handoff.redirect_uri)?;
            if handoff.state.is_empty() {
                return Err(Error::Invalid {
                    message: "cli_state must not be empty".to_owned(),
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
        // "Log in once" needs a refresh token, and only the CLI keeps a
        // credential long enough to need one (ADR-0027 decision 6). Ask
        // only where the issuer says it understands the scope: an IdP that
        // rejects unknown scopes would otherwise fail the whole login.
        if cli.is_some()
            && issuer_state.advertises_scope(OFFLINE_ACCESS)
            && !scope_list.iter().any(|scope| scope == OFFLINE_ACCESS)
        {
            scope_list.push(OFFLINE_ACCESS.to_owned());
        }
        let scopes = scope_list.join(" ");
        let mut url =
            url::Url::parse(&issuer_state.discovery.authorization_endpoint).map_err(|err| {
                Error::Dependency {
                    service: "oidc-discovery".to_owned(),
                    message: format!("issuer {issuer}: bad authorization_endpoint: {err}"),
                }
            })?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &self.redirect_uri)
            .append_pair("scope", &scopes)
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");

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
                    cli,
                },
            );
        }
        metrics::counter!(OIDC_LOGINS_TOTAL, "issuer" => issuer, "outcome" => "started")
            .increment(1);
        Ok(url.into())
    }

    /// The CLI return address parked under `state`, without consuming the
    /// pending login. The gateway reads it *before* completing so that
    /// every way a login can fail still lands back in the terminal
    /// `synveda login` is waiting in, rather than on a page nobody is
    /// looking at (ADR-0027 decision 5).
    pub fn peek_cli(&self, state: &str) -> Option<CliHandoff> {
        let pending = self.pending.lock().expect("pending login lock");
        pending
            .get(state)
            .filter(|login| login.expires_at > Instant::now())
            .and_then(|login| login.cli.clone())
    }

    /// Discards a pending login without completing it — the IdP-reported
    /// failure path, where there is no code to exchange and nothing to
    /// keep parked for ten minutes.
    pub fn abandon(&self, state: &str) {
        self.pending
            .lock()
            .expect("pending login lock")
            .remove(state);
    }

    /// Completes a login from the IdP callback. `state` is single-use;
    /// replaying it — or presenting one the gateway never issued — is a 401.
    pub async fn complete(&self, state: &str, code: &str) -> Result<LoginSession> {
        let login = {
            let mut pending = self.pending.lock().expect("pending login lock");
            pending.remove(state)
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
            cli: login.cli,
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
            .map_err(|err| Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: format!("token exchange: {err}"),
            })?;
        let status = response.status();
        if status.is_client_error() {
            // The IdP refused the grant — a caller problem, and the detail
            // stays in the trace, not the response.
            let body = response.text().await.unwrap_or_default();
            tracing::debug!(%status, body, "token exchange refused");
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
        response.json().await.map_err(|err| Error::Dependency {
            service: "oidc-token-endpoint".to_owned(),
            message: format!("token exchange: invalid body: {err}"),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oidc::parse_issuers;

    fn flow() -> LoginFlow {
        let verifier = Arc::new(
            OidcVerifier::new(
                parse_issuers(r#"[{"issuer":"http://127.0.0.1:1/idp","client_id":"synveda"}]"#)
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
            .complete("never-issued", "code")
            .await
            .expect_err("unknown state must be rejected");
        assert!(matches!(err, Error::Unauthenticated { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn named_but_unconfigured_issuer_is_invalid() {
        let err = flow()
            .begin(Some("http://other-idp"), None)
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
                Some(CliHandoff {
                    redirect_uri: "http://evil.test/callback".to_owned(),
                    state: "s".to_owned(),
                }),
            )
            .await
            .expect_err("a non-loopback redirect must be refused");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
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

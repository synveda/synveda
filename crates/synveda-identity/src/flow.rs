//! The authorization-code + PKCE login flow (AUTH-1, ADR-0010 §5).
//!
//! [`LoginFlow::begin`] builds the IdP authorization URL (S256 challenge,
//! `state`, `nonce`) and parks a [`PendingLogin`]; [`LoginFlow::complete`]
//! consumes the `state` (single use — replay is a 401), exchanges the code
//! at the token endpoint with the `code_verifier`, and verifies the ID
//! token including the nonce. Pending logins live in a bounded in-memory
//! store with a 10-minute TTL: single-replica only until OPS-2 (ADR-0010).
//!
//! Tenant activeness is deliberately not checked here: the store tier is a
//! sibling crate (seed §8), so the gateway's callback handler runs TEN-1's
//! active-tenant lookup on the returned claims.

use std::collections::HashMap;
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

/// How long a pending login may wait between redirect and callback.
const PENDING_TTL: Duration = Duration::from_secs(600);

/// Upper bound on parked logins; beyond it new logins are rate-limited
/// rather than letting an unauthenticated caller grow memory.
const PENDING_CAP: usize = 10_000;

/// Scopes requested from the IdP. `openid` is what makes it OIDC; profile
/// and email feed AUTH-2's JIT provisioning claims.
const SCOPES: &str = "openid profile email";

struct PendingLogin {
    issuer: String,
    code_verifier: String,
    nonce: String,
    expires_at: Instant,
}

/// The completed login: verified claims plus the session material the
/// caller uses as its bearer token (ADR-0010 §1).
#[derive(Debug)]
pub struct LoginSession {
    /// Verified subject and tenant from the ID token.
    pub claims: Claims,
    /// The IdP-issued access token — the `/v1` bearer credential.
    pub access_token: String,
    /// Token type as reported by the IdP (`Bearer`).
    pub token_type: String,
    /// Seconds until the access token expires, when the IdP reports it.
    pub expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Drives the code+PKCE flow against the verifier's configured issuers.
pub struct LoginFlow {
    verifier: Arc<OidcVerifier>,
    redirect_uri: String,
    pending: Mutex<HashMap<String, PendingLogin>>,
}

impl LoginFlow {
    /// Builds a flow that sends callbacks to `redirect_uri`
    /// (`{SYNVEDA_PUBLIC_URL}/auth/callback`).
    pub fn new(verifier: Arc<OidcVerifier>, redirect_uri: String) -> Self {
        Self {
            verifier,
            redirect_uri,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Starts a login and returns the IdP authorization URL to redirect to.
    /// `issuer` may be omitted when exactly one is configured.
    pub async fn begin(&self, issuer: Option<&str>) -> Result<String> {
        let issuer = match (issuer, self.verifier.sole_issuer()) {
            (Some(named), _) => self.verifier.config(named)?.issuer.clone(),
            (None, Some(sole)) => sole.to_owned(),
            (None, None) => {
                return Err(Error::Invalid {
                    message: "several issuers are configured; pass ?issuer=".to_owned(),
                });
            }
        };
        let state = random_urlsafe()?;
        let nonce = random_urlsafe()?;
        let code_verifier = random_urlsafe()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));

        let issuer_state = self.verifier.issuer_state(&issuer).await?;
        let client_id = self.verifier.config(&issuer)?.client_id.clone();
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
            .append_pair("scope", SCOPES)
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
                },
            );
        }
        metrics::counter!(OIDC_LOGINS_TOTAL, "issuer" => issuer, "outcome" => "started")
            .increment(1);
        Ok(url.into())
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
            access_token: tokens.access_token,
            token_type: tokens.token_type,
            expires_in: tokens.expires_in,
        })
    }

    /// The authorization-code exchange: a public client authenticating the
    /// redemption with the PKCE verifier alone (no client secret).
    #[tracing::instrument(name = "oidc.exchange", skip_all, fields(oidc.issuer = %login.issuer))]
    async fn exchange(&self, login: &PendingLogin, code: &str) -> Result<TokenResponse> {
        let issuer_state = self.verifier.issuer_state(&login.issuer).await?;
        let client_id = self.verifier.config(&login.issuer)?.client_id.clone();
        let response = self
            .verifier
            .http()
            .post(&issuer_state.discovery.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", &self.redirect_uri),
                ("client_id", &client_id),
                ("code_verifier", &login.code_verifier),
            ])
            .send()
            .await
            .map_err(|err| Error::Dependency {
                service: "oidc-token-endpoint".to_owned(),
                message: format!("token exchange: {err}"),
            })?;
        let status = response.status();
        if status.is_client_error() {
            // The IdP refused the code/verifier — a caller problem, and the
            // detail stays in the trace, not the response.
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
            .begin(Some("http://other-idp"))
            .await
            .expect_err("unconfigured issuer must be rejected");
        assert!(matches!(err, Error::Invalid { .. }), "got {err:?}");
    }
}

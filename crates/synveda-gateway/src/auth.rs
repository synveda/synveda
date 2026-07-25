//! The unauthenticated auth plane (AUTH-1, ADR-0010): `GET /auth/login`
//! redirects to the IdP with a code+PKCE challenge; `GET /auth/callback`
//! completes the login and returns the Synveda session material (subject,
//! tenant, and the access token to present as the `/v1` bearer).
//!
//! Since ADPT-1 (ADR-0027 decisions 5 and 6) the same two routes also
//! serve `synveda login`. A login started with a `cli_redirect_uri` runs
//! AUTH-1 unchanged — same PKCE, same JWKS verification, same TEN-1
//! active-tenant rule, same AUTH-2 provisioning — and differs only at the
//! last step: instead of returning the session as JSON to a browser, the
//! callback 302s to the CLI's loopback listener with a one-time,
//! 60-second, state-bound handoff code. The CLI redeems that code at
//! `POST /auth/cli/exchange`, and renews the resulting access token at
//! `POST /auth/refresh`. Tokens never travel in a URL or a browser
//! history; only the single-use code does.
//!
//! Both CLI routes are unauthenticated for the same reason `/auth/login`
//! is: they are how a caller becomes authenticated. Neither reads or
//! writes governed data, so neither involves the PDP; the credential they
//! hand back carries exactly the authority the IdP and AUTH-2 gave it.
//!
//! AUD-1 wiring point: login completions and rejections become audit events
//! when the hash-chained log lands; until then they are visible in traces
//! and `synveda_oidc_logins_total` only (ADR-0010 compliance notes).

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Redirect, Response};
use serde::{Deserialize, Serialize};
use synveda_identity::CliHandoff;
use synveda_types::{Error, IdentityId, ScopeId, Tenant};

use crate::app::AppState;
use crate::error::ApiError;
use crate::provision;
use crate::tenant;

/// CLI-mediated login stages by outcome (`started`, `handed_off`,
/// `exchanged`, `rejected`) — the ADPT-1 half of the AUTH-1 metric
/// contract (ADR-0027 decision 5).
pub const CLI_LOGINS_TOTAL: &str = "synveda_cli_logins_total";

/// Query parameters for `GET /auth/login`.
#[derive(Deserialize)]
pub struct LoginParams {
    /// Which configured issuer to log in against; optional when exactly
    /// one is configured.
    issuer: Option<String>,
    /// The loopback URI `synveda login` is listening on (ADR-0027
    /// decision 5). Its presence is what makes this a CLI login; the
    /// allowlist that governs it is absolute and lives in
    /// `synveda_identity::validate_cli_redirect_uri`.
    cli_redirect_uri: Option<String>,
    /// The CLI's own CSRF state, returned to the loopback listener and
    /// required again at redemption. Given with `cli_redirect_uri` or not
    /// at all.
    cli_state: Option<String>,
}

/// `POST /auth/cli/exchange`: redeem a handoff code for session material.
#[derive(Deserialize)]
pub struct CliExchangeRequest {
    code: String,
    state: String,
}

/// `POST /auth/refresh`: renew an access token (ADR-0027 decision 6).
#[derive(Deserialize)]
pub struct RefreshRequest {
    refresh_token: String,
    /// Optional when exactly one issuer is configured, same rule as
    /// `/auth/login`.
    #[serde(default)]
    issuer: Option<String>,
}

/// The refreshed credential. No claims and no identity: a refresh is an
/// OAuth-client operation, and the new bearer is verified where every
/// bearer is — at the `/v1` seam, on the next request.
#[derive(Serialize)]
struct RefreshResponse {
    access_token: String,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
    /// Present only for issuers that rotate refresh tokens; absent means
    /// the CLI keeps the one it has.
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

/// Query parameters the IdP sends to `GET /auth/callback` (RFC 6749 §4.1.2:
/// either `code`+`state` or `error`).
#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// The Synveda session (ADR-0010 §1): who the login resolved to, plus the
/// bearer credential for `/v1`. Since AUTH-2 it also says where JIT
/// provisioning placed the subject (ADR-0013 decision 2).
#[derive(Serialize)]
struct SessionResponse {
    subject: String,
    tenant: Tenant,
    identity: IdentitySummary,
    access_token: String,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
}

/// The same session, as a CLI login receives it: the browser-facing shape
/// plus the two things only a long-lived client needs — the issuer to
/// refresh against, and the refresh token itself. Kept a separate type on
/// purpose. [`SessionResponse`] is then structurally incapable of carrying
/// a refresh token to a browser (ADR-0027 decision 6), which an
/// `Option` field on one shared struct would leave one edit away.
#[derive(Serialize)]
struct CliSessionResponse {
    #[serde(flatten)]
    session: SessionResponse,
    issuer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

/// The provisioning result a fresh session reports.
#[derive(Serialize)]
struct IdentitySummary {
    id: IdentityId,
    scope_id: ScopeId,
    /// Display-only slug chain of the personal scope (ADR-0011).
    scope_path: String,
    quarantined: bool,
}

/// Starts a login: 302 to the IdP's authorization endpoint. A
/// `cli_redirect_uri` (with its `cli_state`) makes it a CLI login and
/// changes nothing but where the completed session is handed back.
#[tracing::instrument(name = "auth.login", skip_all)]
pub async fn login(State(state): State<AppState>, Query(params): Query<LoginParams>) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    let cli = match (params.cli_redirect_uri, params.cli_state) {
        (None, None) => None,
        (Some(redirect_uri), Some(cli_state)) => Some(CliHandoff {
            redirect_uri,
            state: cli_state,
        }),
        _ => {
            return ApiError(Error::Invalid {
                message: "cli_redirect_uri and cli_state must be given together".to_owned(),
            })
            .into_response();
        }
    };
    let is_cli = cli.is_some();
    match flow.begin(params.issuer.as_deref(), cli).await {
        Ok(url) => {
            if is_cli {
                metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "started").increment(1);
            }
            Redirect::temporary(&url).into_response()
        }
        Err(error) => {
            if is_cli {
                metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "rejected").increment(1);
            }
            ApiError(error).into_response()
        }
    }
}

/// Completes a login: code exchange, ID-token verification, and TEN-1's
/// active-tenant rule — a login for a suspended or unknown tenant is the
/// same uniform 401 as a bearer request for one.
#[tracing::instrument(name = "auth.callback", skip_all)]
pub async fn callback(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    // Read the CLI's return address before anything consumes the pending
    // login: a login can fail in half a dozen ways below, and all of them
    // have to land back in the terminal, not on a page nobody sees.
    let cli = params
        .state
        .as_deref()
        .and_then(|login_state| flow.peek_cli(login_state));
    let refuse = |error: Error| match &cli {
        Some(handoff) => cli_error_redirect(
            handoff,
            "login_failed",
            &crate::error::caller_facing(&error).to_string(),
        ),
        None => ApiError(error).into_response(),
    };

    if let Some(error) = params.error {
        // The IdP refused (user denied, policy, ...). The description is
        // trace detail; the caller gets the classification.
        tracing::debug!(
            error,
            description = params.error_description.as_deref().unwrap_or_default(),
            "authorization error returned by the IdP"
        );
        // Nothing will complete this login; do not leave it parked for the
        // rest of its TTL.
        if let Some(login_state) = &params.state {
            flow.abandon(login_state);
        }
        return refuse(Error::Unauthenticated {
            message: format!("the identity provider reported: {error}"),
        });
    }
    let (Some(code), Some(login_state)) = (params.code, params.state) else {
        return refuse(Error::Invalid {
            message: "callback requires code and state".to_owned(),
        });
    };
    let session = match flow.complete(&login_state, &code).await {
        Ok(session) => session,
        Err(error) => return refuse(error),
    };
    let context = match tenant::active_tenant(&state, &session.claims).await {
        Ok(context) => context,
        Err(error) => return refuse(error),
    };
    // A completed login always carries IdP claims (the ID token was just
    // verified); JIT provisioning places first-time subjects (AUTH-2,
    // ADR-0013) and is a read for everyone else.
    let Some(provisioning) = &session.claims.provisioning else {
        return refuse(Error::Internal {
            message: "login completed without provisioning claims".to_owned(),
        });
    };
    let provisioned = match provision::provision(
        &state,
        &context.tenant,
        &session.claims.subject,
        provisioning,
    )
    .await
    {
        Ok(provisioned) => provisioned,
        Err(error) => return refuse(error),
    };
    let completed = SessionResponse {
        subject: session.claims.subject,
        tenant: context.tenant,
        identity: IdentitySummary {
            id: provisioned.identity.id,
            scope_id: provisioned.identity.scope_id,
            scope_path: provisioned.scope.path,
            quarantined: provisioned.identity.quarantined,
        },
        access_token: session.access_token,
        token_type: session.token_type,
        expires_in: session.expires_in,
    };
    match &cli {
        // A browser login reads its session here, as AUTH-1 always has.
        None => Json(completed).into_response(),
        // A CLI login gets a code, and only a code: the session material
        // waits on the gateway until the CLI redeems it (ADR-0027
        // decision 5).
        Some(handoff) => hand_off(
            flow,
            handoff,
            completed,
            session.issuer,
            session.refresh_token,
        ),
    }
}

/// Parks the completed session and 302s to the CLI's loopback listener
/// with the one-time code. A failure to park is still a redirect: the CLI
/// is waiting on that listener, and leaving it to time out would turn a
/// transient gateway problem into a hung terminal.
fn hand_off(
    flow: &synveda_identity::LoginFlow,
    handoff: &CliHandoff,
    session: SessionResponse,
    issuer: String,
    refresh_token: Option<String>,
) -> Response {
    let payload = match serde_json::to_value(CliSessionResponse {
        session,
        issuer,
        refresh_token,
    }) {
        Ok(payload) => payload,
        Err(error) => {
            return cli_error_redirect(handoff, "server_error", &format!("{error}"));
        }
    };
    match flow.park_handoff(&handoff.state, payload) {
        Ok(code) => {
            metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "handed_off").increment(1);
            let mut url = handoff.redirect_uri.clone();
            // The allowlist guarantees no existing query string, so the
            // first separator is always '?'.
            url.push_str(&format!(
                "?code={}&state={}",
                urlencode(&code),
                urlencode(&handoff.state)
            ));
            Redirect::temporary(&url).into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "could not park a CLI login handoff");
            cli_error_redirect(
                handoff,
                "server_error",
                "the gateway could not park the login",
            )
        }
    }
}

/// Tells the waiting CLI that the login failed, in the same place it is
/// already listening. The description is caller-safe by construction —
/// nothing here reads a token.
fn cli_error_redirect(handoff: &CliHandoff, error: &str, description: &str) -> Response {
    metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "rejected").increment(1);
    let url = format!(
        "{}?error={}&error_description={}&state={}",
        handoff.redirect_uri,
        urlencode(error),
        urlencode(description),
        urlencode(&handoff.state)
    );
    Redirect::temporary(&url).into_response()
}

/// Redeems a one-time handoff code for the session material of a
/// CLI-mediated login (ADR-0027 decision 5). Single use, 60 seconds, and
/// bound to the state the CLI minted: everything a caller needs is
/// something only the CLI that started the login has.
#[tracing::instrument(name = "auth.cli.exchange", skip_all)]
pub async fn cli_exchange(
    State(state): State<AppState>,
    Json(request): Json<CliExchangeRequest>,
) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    match flow.redeem_handoff(&request.code, &request.state) {
        Ok(payload) => {
            metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "exchanged").increment(1);
            Json(payload).into_response()
        }
        Err(error) => {
            metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "rejected").increment(1);
            ApiError(error).into_response()
        }
    }
}

/// Renews an access token from a refresh token (ADR-0027 decision 6). The
/// gateway remains the OAuth client — which is exactly what lets the CLI
/// hold no client id, no client secret, and no issuer configuration.
#[tracing::instrument(name = "auth.refresh", skip_all)]
pub async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    match flow
        .refresh(request.issuer.as_deref(), &request.refresh_token)
        .await
    {
        Ok(refreshed) => Json(RefreshResponse {
            access_token: refreshed.access_token,
            token_type: refreshed.token_type,
            expires_in: refreshed.expires_in,
            refresh_token: refreshed.refresh_token,
        })
        .into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}

/// Percent-encodes one query-parameter value. The values are base64url
/// codes and fixed error slugs, so this is belt-and-braces — but a
/// redirect target is the last place to assume an input is already safe.
fn urlencode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn not_configured() -> Response {
    ApiError(Error::NotFound {
        entity: "OIDC login (no issuers configured on this gateway)".to_owned(),
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::urlencode;

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(urlencode("aB9-_.~"), "aB9-_.~");
        assert_eq!(urlencode("a b&c=d#e"), "a%20b%26c%3Dd%23e");
        // A code that tried to smuggle a second parameter cannot.
        assert_eq!(urlencode("x&state=forged"), "x%26state%3Dforged");
    }
}

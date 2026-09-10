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
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Json, Redirect, Response};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use synveda_identity::{
    CliHandoff, ConsoleLoginBinding, LoginDestination, validate_cli_redirect_uri,
};
use synveda_types::{Error, IdentityId, ScopeId, Tenant};

use crate::app::{AppState, ConsoleCookieMode};
use crate::error::ApiError;
use crate::provision;
use crate::tenant;

/// CLI-mediated login stages by outcome (`started`, `handed_off`,
/// `exchanged`, `rejected`) — the ADPT-1 half of the AUTH-1 metric
/// contract (ADR-0027 decision 5).
pub const CLI_LOGINS_TOTAL: &str = "synveda_cli_logins_total";

/// Console session lifecycle by outcome (`opened`, `closed`, `absent`,
/// `rejected`, `error`) — CNSL-1's half of the AUTH-1 metric contract.
pub const CONSOLE_SESSIONS_TOTAL: &str = "synveda_console_sessions_total";

/// Where a console login lands, win or lose. A fixed path rather than
/// anything a caller supplies (ADR-0056 decision 2): the console is served
/// from this origin, and a login that redirects wherever it is told is an
/// open redirector with an audience.
const CONSOLE_HOME: &str = "/console/";

/// The browser-correlation cookie expires with the in-memory pending login.
const PENDING_LOGIN_MAX_SECS: i64 = 10 * 60;

/// The console session's hard cap — 12 hours, one working day. A refresh
/// token an IdP never rotates would otherwise make the session immortal;
/// this is the ceiling migration 0034's `absolute_expires_at` enforces, and
/// past it the operator logs in again.
const CONSOLE_SESSION_MAX_SECS: i64 = 12 * 60 * 60;

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
    /// `?console=true` makes this a console login (CNSL-1, ADR-0056
    /// decision 2): the gateway keeps the tokens and hands the browser a
    /// cookie naming them. A flag rather than a redirect target on
    /// purpose — the console is served from this origin, so there is
    /// nowhere else for the login to land, and a caller-supplied return
    /// address would be an open redirector bolted to a login.
    #[serde(default)]
    console: bool,
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
    /// Display-only slug chain of the caller's own principal scope, from
    /// the tenant root (CPR-3, ADR-0070).
    scope_path: String,
}

/// Starts a login: 302 to the IdP's authorization endpoint. A
/// `cli_redirect_uri` (with its `cli_state`) makes it a CLI login and
/// changes nothing but where the completed session is handed back.
#[tracing::instrument(name = "auth.login", skip_all)]
pub async fn login(State(state): State<AppState>, Query(params): Query<LoginParams>) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    let mut console_correlation_cookie = None;
    let destination = match (params.cli_redirect_uri, params.cli_state, params.console) {
        (None, None, false) => LoginDestination::Json,
        (None, None, true) => {
            let correlation = match synveda_identity::console::mint() {
                Ok(correlation) => correlation,
                Err(error) => return private_response(ApiError(error).into_response()),
            };
            let cookie = match login_cookie_header(
                &correlation.secret,
                PENDING_LOGIN_MAX_SECS,
                flow.cookie_mode(),
            ) {
                Ok(cookie) => cookie,
                Err(error) => return private_response(ApiError(error).into_response()),
            };
            let destination = LoginDestination::Console(ConsoleLoginBinding::new(correlation.hash));
            console_correlation_cookie = Some(cookie);
            destination
        }
        (Some(redirect_uri), Some(cli_state), false) => LoginDestination::Cli(CliHandoff {
            redirect_uri,
            state: cli_state,
        }),
        // A CLI login that is also a console login is not a thing, and
        // guessing which one the caller meant is how a credential ends up
        // delivered somewhere nobody intended.
        (Some(_), Some(_), true) => {
            return ApiError(Error::Invalid {
                message: "console and cli_redirect_uri are mutually exclusive".to_owned(),
            })
            .into_response();
        }
        _ => {
            return ApiError(Error::Invalid {
                message: "cli_redirect_uri and cli_state must be given together".to_owned(),
            })
            .into_response();
        }
    };
    let cli_handoff = match &destination {
        LoginDestination::Cli(handoff) => {
            if let Err(error) = validate_cli_redirect_uri(&handoff.redirect_uri) {
                return ApiError(error).into_response();
            }
            Some(handoff.clone())
        }
        _ => None,
    };
    match flow.begin(params.issuer.as_deref(), destination).await {
        Ok(url) => {
            if cli_handoff.is_some() {
                metrics::counter!(CLI_LOGINS_TOTAL, "outcome" => "started").increment(1);
            }
            let mut response = Redirect::temporary(&url).into_response();
            if let Some(cookie) = console_correlation_cookie {
                response.headers_mut().append(header::SET_COOKIE, cookie);
            }
            private_response(response)
        }
        Err(error) => {
            if let Some(handoff) = &cli_handoff {
                return cli_error_redirect(
                    handoff,
                    "login_unavailable",
                    "the gateway could not start the identity-provider login",
                );
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
    headers: HeaderMap,
    Query(params): Query<CallbackParams>,
) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    // Read the CLI's return address before anything consumes the pending
    // login: a login can fail in half a dozen ways below, and all of them
    // have to land back in the terminal, not on a page nobody sees.
    let cookie_mode = flow.cookie_mode();
    let presented_correlation = login_correlation_cookie(&headers, cookie_mode);
    let destination = params
        .state
        .as_deref()
        .and_then(|login_state| flow.peek_destination(login_state, presented_correlation));
    let matched_console = matches!(destination, Some(LoginDestination::Console(_)));
    let refuse = |error: Error| match &destination {
        Some(LoginDestination::Cli(handoff)) => cli_error_redirect(
            handoff,
            "login_failed",
            &crate::error::caller_facing(&error).to_string(),
        ),
        // A console login lands back in the console, which can say so in
        // its own words. The classification rides the query string; the
        // error itself never does, on `caller_facing`'s usual rule.
        Some(LoginDestination::Console(_)) => console_error_redirect(&error),
        _ => ApiError(error).into_response(),
    };

    if let Some(error) = params.error.as_deref() {
        // Both fields are an untrusted browser/IdP-controlled content
        // channel. Record only their presence; callers receive a closed
        // classification.
        tracing::debug!(
            has_error = !error.is_empty(),
            has_description = params.error_description.is_some(),
            "authorization error returned by the IdP"
        );
        // Nothing will complete this login; do not leave it parked for the
        // rest of its TTL.
        let consumed = params
            .state
            .as_deref()
            .is_some_and(|login_state| flow.abandon(login_state, presented_correlation));
        return finish_callback(
            refuse(Error::Unauthenticated {
                message: "the identity provider rejected the authorization request".to_owned(),
            }),
            matched_console && consumed,
            cookie_mode,
        );
    }
    let (Some(code), Some(login_state)) = (params.code.as_deref(), params.state.as_deref()) else {
        let consumed = params
            .state
            .as_deref()
            .is_some_and(|login_state| flow.abandon(login_state, presented_correlation));
        return finish_callback(
            refuse(Error::Invalid {
                message: "callback requires code and state".to_owned(),
            }),
            matched_console && consumed,
            cookie_mode,
        );
    };
    let session = match flow
        .complete(login_state, code, presented_correlation)
        .await
    {
        Ok(session) => session,
        Err(error) => return finish_callback(refuse(error), matched_console, cookie_mode),
    };
    let context = match tenant::active_tenant(&state, &session.claims).await {
        Ok(context) => context,
        Err(error) => return finish_callback(refuse(error), matched_console, cookie_mode),
    };
    // A completed login always carries IdP claims (the ID token was just
    // verified); JIT provisioning places first-time subjects (AUTH-2,
    // ADR-0013) and is a read for everyone else.
    let Some(provisioning) = &session.claims.provisioning else {
        return finish_callback(
            refuse(Error::Internal {
                message: "login completed without provisioning claims".to_owned(),
            }),
            matched_console,
            cookie_mode,
        );
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
        Err(error) => return finish_callback(refuse(error), matched_console, cookie_mode),
    };
    let completed = SessionResponse {
        subject: session.claims.subject,
        tenant: context.tenant,
        identity: IdentitySummary {
            id: provisioned.identity.id,
            scope_id: provisioned.identity.scope_id,
            scope_path: provisioned.scope_path.clone(),
        },
        access_token: session.access_token,
        token_type: session.token_type,
        expires_in: session.expires_in,
    };
    let response = match session.destination {
        // A browser login reads its session here, as AUTH-1 always has.
        LoginDestination::Json => private_response(Json(completed).into_response()),
        // A CLI login gets a code, and only a code: the session material
        // waits on the gateway until the CLI redeems it (ADR-0027
        // decision 5).
        LoginDestination::Cli(handoff) => hand_off(
            flow,
            &handoff,
            completed,
            session.issuer,
            session.refresh_token,
        ),
        // A console login gets a cookie, and only a cookie: the tokens
        // stay here (ADR-0056 decisions 2 and 3).
        LoginDestination::Console(_) => {
            open_console_session(
                &state,
                completed,
                session.issuer,
                session.refresh_token,
                cookie_mode,
            )
            .await
        }
    };
    finish_callback(response, matched_console, cookie_mode)
}

/// Opens a console session: mint a secret, store what it names, set the
/// cookie, and 302 into the app.
///
/// The response body carries nothing. That is the point of the whole
/// arrangement — the browser leaves this handler holding an opaque string
/// and no credential, and every fact about who it is comes from verifying
/// the stored access token on the next request (ADR-0056 decision 2).
async fn open_console_session(
    state: &AppState,
    session: SessionResponse,
    issuer: String,
    refresh_token: Option<String>,
    cookie_mode: ConsoleCookieMode,
) -> Response {
    let secret = match synveda_identity::console::mint() {
        Ok(secret) => secret,
        Err(error) => return console_error_redirect(&error),
    };
    let access_expires_at = session
        .expires_in
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| Utc::now().checked_add_signed(Duration::seconds(seconds)));
    let absolute_expires_at = Utc::now() + Duration::seconds(CONSOLE_SESSION_MAX_SECS);

    // Sealed before it is stored (TEN-4, ADR-0064): the deployment key, bound
    // to this session's own hash. A key that is not configured fails the
    // login rather than storing a plaintext credential — which is the whole
    // reason `Kms::Disabled` refuses instead of passing bytes through.
    let access_token_sealed = match state
        .seal_console_token(
            &secret.hash,
            synveda_crypto::Purpose::ConsoleAccessToken,
            &session.access_token,
        )
        .await
    {
        Ok(sealed) => sealed,
        Err(error) => {
            tracing::warn!(%error, "could not seal a console session's access token");
            return console_error_redirect(&error);
        }
    };
    let refresh_token_sealed = match refresh_token.as_deref() {
        Some(token) => match state
            .seal_console_token(
                &secret.hash,
                synveda_crypto::Purpose::ConsoleRefreshToken,
                token,
            )
            .await
        {
            Ok(sealed) => Some(sealed),
            Err(error) => {
                tracing::warn!(%error, "could not seal a console session's refresh token");
                return console_error_redirect(&error);
            }
        },
        None => None,
    };

    if let Err(error) = synveda_store::console_sessions::create(
        &state.pool,
        &secret.hash,
        &issuer,
        &access_token_sealed,
        access_expires_at,
        refresh_token_sealed.as_deref(),
        absolute_expires_at,
    )
    .await
    {
        tracing::warn!(%error, "could not open a console session");
        return console_error_redirect(&error);
    }

    metrics::counter!(CONSOLE_SESSIONS_TOTAL, "outcome" => "opened").increment(1);
    let mut response = Redirect::temporary(CONSOLE_HOME).into_response();
    match set_cookie_header(&secret.secret, CONSOLE_SESSION_MAX_SECS, cookie_mode) {
        Ok(value) => {
            response.headers_mut().insert(header::SET_COOKIE, value);
            private_response(response)
        }
        Err(error) => {
            tracing::warn!(%error, "could not render the console session cookie");
            console_error_redirect(&Error::Internal {
                message: "could not render the session cookie".to_owned(),
            })
        }
    }
}

/// Ends a console session. Idempotent by design: the cookie is cleared
/// either way, so a second click, a replayed request, or a session the
/// gateway already reaped all end in the same place — signed out.
#[tracing::instrument(name = "auth.console.logout", skip_all)]
pub async fn console_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let cookie_mode = state
        .login
        .as_ref()
        .map_or(ConsoleCookieMode::Https, |login| login.cookie_mode());
    if let Some(secret) = console_cookie(&headers, cookie_mode) {
        let hash = synveda_identity::console::hash(secret);
        match synveda_store::console_sessions::delete(&state.pool, &hash).await {
            Ok(existed) => {
                let outcome = if existed { "closed" } else { "absent" };
                metrics::counter!(CONSOLE_SESSIONS_TOTAL, "outcome" => outcome).increment(1);
            }
            // The row may outlive its cookie. Say so in traces, still clear
            // the cookie: a sign-out that reports failure and leaves the
            // browser holding a live session is the worst of both.
            Err(error) => {
                tracing::warn!(%error, "could not delete a console session");
                metrics::counter!(CONSOLE_SESSIONS_TOTAL, "outcome" => "error").increment(1);
            }
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = set_cookie_header("", 0, cookie_mode) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

/// Reads the console cookie off a request.
pub(crate) fn console_cookie(headers: &HeaderMap, cookie_mode: ConsoleCookieMode) -> Option<&str> {
    named_cookie(headers, console_cookie_name(cookie_mode))
}

/// Reads the short-lived console-login correlation cookie. Missing,
/// malformed and duplicated values all deliberately collapse to absence.
fn login_correlation_cookie(headers: &HeaderMap, cookie_mode: ConsoleCookieMode) -> Option<&str> {
    named_cookie(headers, login_cookie_name(cookie_mode))
        .filter(|secret| synveda_identity::console::presented_hash(secret).is_some())
}

fn console_cookie_name(cookie_mode: ConsoleCookieMode) -> &'static str {
    match cookie_mode {
        ConsoleCookieMode::Https => synveda_identity::console::CONSOLE_COOKIE,
        ConsoleCookieMode::ExplicitDevelopmentHttp => {
            synveda_identity::console::DEVELOPMENT_CONSOLE_COOKIE
        }
    }
}

fn login_cookie_name(cookie_mode: ConsoleCookieMode) -> &'static str {
    match cookie_mode {
        ConsoleCookieMode::Https => synveda_identity::console::LOGIN_COOKIE,
        ConsoleCookieMode::ExplicitDevelopmentHttp => {
            synveda_identity::console::DEVELOPMENT_LOGIN_COOKIE
        }
    }
}

/// Reads exactly one named cookie across every Cookie header field.
///
/// Selecting a first or last duplicate lets intermediaries and application
/// code disagree about which credential was presented. Refuse the ambiguous
/// request instead.
fn named_cookie<'a>(headers: &'a HeaderMap, expected_name: &str) -> Option<&'a str> {
    let mut found = None;
    for raw in headers.get_all(header::COOKIE) {
        let raw = raw.to_str().ok()?;
        for pair in raw.split(';') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if name.trim() != expected_name {
                continue;
            }
            let value = value.trim();
            if value.is_empty() || found.replace(value).is_some() {
                return None;
            }
        }
    }
    found
}

/// Renders the `Set-Cookie` value. `max_age` of 0 with an empty secret is
/// the clear.
///
/// HTTPS uses the browser-enforced `__Host-`/`Secure` pair. Only the startup-
/// validated explicit development HTTP mode uses a distinct host-only name,
/// retaining every other session and CSRF attribute.
fn set_cookie_header(
    secret: &str,
    max_age: i64,
    cookie_mode: ConsoleCookieMode,
) -> Result<header::HeaderValue, Error> {
    let secure = match cookie_mode {
        ConsoleCookieMode::Https => "; Secure",
        ConsoleCookieMode::ExplicitDevelopmentHttp => "",
    };
    let value = format!(
        "{}={secret}; Max-Age={max_age}; Path=/{secure}; HttpOnly; SameSite=Strict",
        console_cookie_name(cookie_mode),
    );
    header::HeaderValue::from_str(&value).map_err(|err| Error::Internal {
        message: format!("cookie value is not a valid header: {err}"),
    })
}

/// Renders the short-lived browser binding for a console OIDC round trip.
fn login_cookie_header(
    secret: &str,
    max_age: i64,
    cookie_mode: ConsoleCookieMode,
) -> Result<header::HeaderValue, Error> {
    let secure = match cookie_mode {
        ConsoleCookieMode::Https => "; Secure",
        ConsoleCookieMode::ExplicitDevelopmentHttp => "",
    };
    let value = format!(
        "{}={secret}; Max-Age={max_age}; Path=/{secure}; HttpOnly; SameSite=Lax",
        login_cookie_name(cookie_mode),
    );
    header::HeaderValue::from_str(&value).map_err(|err| Error::Internal {
        message: format!("login cookie value is not a valid header: {err}"),
    })
}

/// Applies the cache and browser-secret cleanup contract to every callback
/// response. A matching console callback is terminal even when token
/// exchange, tenant admission or provisioning later fails.
fn finish_callback(
    mut response: Response,
    clear_login_cookie: bool,
    cookie_mode: ConsoleCookieMode,
) -> Response {
    if clear_login_cookie && let Ok(value) = clear_login_cookie_header(cookie_mode) {
        // `append`, not `insert`: a successful console callback already has
        // the final session cookie and must carry both fields.
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    private_response(response)
}

fn clear_login_cookie_header(cookie_mode: ConsoleCookieMode) -> Result<header::HeaderValue, Error> {
    let secure = match cookie_mode {
        ConsoleCookieMode::Https => "; Secure",
        ConsoleCookieMode::ExplicitDevelopmentHttp => "",
    };
    let value = format!(
        "{}=; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Path=/{secure}; HttpOnly; SameSite=Lax",
        login_cookie_name(cookie_mode),
    );
    header::HeaderValue::from_str(&value).map_err(|err| Error::Internal {
        message: format!("login cookie deletion is not a valid header: {err}"),
    })
}

/// Sends a failed console login back to the console with a classification
/// and nothing else. Never the error's own text: `caller_facing` governs
/// what a caller learns, and a query string is the most quotable place a
/// message can land.
fn console_error_redirect(error: &Error) -> Response {
    metrics::counter!(CONSOLE_SESSIONS_TOTAL, "outcome" => "rejected").increment(1);
    let classification = match error {
        Error::Unauthenticated { .. } => "unauthenticated",
        Error::Invalid { .. } => "invalid_request",
        _ => "server_error",
    };
    Redirect::temporary(&format!("{CONSOLE_HOME}?error={classification}")).into_response()
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
            private_response(Redirect::temporary(&url).into_response())
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
            private_response(Json(payload).into_response())
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
        Ok(refreshed) => private_response(
            Json(RefreshResponse {
                access_token: refreshed.access_token,
                token_type: refreshed.token_type,
                expires_in: refreshed.expires_in,
                refresh_token: refreshed.refresh_token,
            })
            .into_response(),
        ),
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

/// Token-bearing JSON, one-time handoff redirects and session cookies must
/// never enter browser, proxy or intermediary caches (OIDC Core §3.1.3.3).
fn private_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, header::HeaderValue::from_static("no-cache"));
    response
}

fn not_configured() -> Response {
    ApiError(Error::NotFound {
        entity: "OIDC login (no issuers configured on this gateway)".to_owned(),
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, header};

    use super::{
        IdentitySummary, clear_login_cookie_header, console_cookie, login_cookie_header,
        set_cookie_header, urlencode,
    };
    use crate::app::ConsoleCookieMode;

    /// **The other half of the CLI's `the_session_shape_is_the_one_the_gateway
    /// _serves`** (CPR-9).
    ///
    /// `synveda login` hand-writes the type it parses this with, because
    /// `/auth/cli/exchange` is not on the OpenAPI contract yet (Prompt 19).
    /// Until this test, nothing anywhere checked the two agreed — and they had
    /// not since CPR-7 deleted `quarantined` from this struct, which made
    /// every CLI login fail after a successful exchange.
    ///
    /// So this pins the served key set exactly. A field added here is a field
    /// this assertion makes somebody look at the CLI for; a field removed is
    /// one it stops them shipping.
    #[test]
    fn the_cli_session_shape_is_the_one_the_cli_parses() {
        let summary = IdentitySummary {
            id: synveda_types::IdentityId::new(),
            scope_id: synveda_types::ScopeId::new(),
            scope_path: "acme/alice".to_owned(),
        };
        let rendered = serde_json::to_value(&summary).expect("serialises");
        let mut keys: Vec<&str> = rendered
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["id", "scope_id", "scope_path"],
            "the session's `identity` block changed shape. `synveda login` \
             parses it with its own `IdentitySummary` (crates/synveda-cli/src/\
             login.rs) — update that too, or the next login fails after the \
             browser round trip."
        );
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(urlencode("aB9-_.~"), "aB9-_.~");
        assert_eq!(urlencode("a b&c=d#e"), "a%20b%26c%3Dd%23e");
        // A code that tried to smuggle a second parameter cannot.
        assert_eq!(urlencode("x&state=forged"), "x%26state%3Dforged");
    }

    #[test]
    fn explicit_development_http_uses_distinct_host_only_cookie_contracts() {
        let session = set_cookie_header(
            "session",
            43_200,
            ConsoleCookieMode::ExplicitDevelopmentHttp,
        )
        .expect("development session cookie")
        .to_str()
        .expect("ASCII cookie")
        .to_owned();
        assert_eq!(
            session,
            "synveda_console_dev=session; Max-Age=43200; Path=/; HttpOnly; SameSite=Strict"
        );

        let login = login_cookie_header(
            "correlation",
            600,
            ConsoleCookieMode::ExplicitDevelopmentHttp,
        )
        .expect("development login cookie")
        .to_str()
        .expect("ASCII cookie")
        .to_owned();
        assert_eq!(
            login,
            "synveda_login_dev=correlation; Max-Age=600; Path=/; HttpOnly; SameSite=Lax"
        );
        assert_eq!(
            clear_login_cookie_header(ConsoleCookieMode::ExplicitDevelopmentHttp)
                .expect("development login clear")
                .to_str()
                .expect("ASCII cookie"),
            "synveda_login_dev=; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Path=/; HttpOnly; SameSite=Lax"
        );
    }

    #[test]
    fn https_cookie_contract_remains_host_prefixed_and_secure() {
        assert_eq!(
            set_cookie_header("session", 43_200, ConsoleCookieMode::Https)
                .expect("HTTPS session cookie")
                .to_str()
                .expect("ASCII cookie"),
            "__Host-synveda_console=session; Max-Age=43200; Path=/; Secure; HttpOnly; SameSite=Strict"
        );
        assert_eq!(
            login_cookie_header("correlation", 600, ConsoleCookieMode::Https)
                .expect("HTTPS login cookie")
                .to_str()
                .expect("ASCII cookie"),
            "__Host-synveda_login=correlation; Max-Age=600; Path=/; Secure; HttpOnly; SameSite=Lax"
        );
    }

    #[test]
    fn duplicated_console_cookie_is_rejected_in_both_modes() {
        for (mode, name) in [
            (ConsoleCookieMode::Https, "__Host-synveda_console"),
            (
                ConsoleCookieMode::ExplicitDevelopmentHttp,
                "synveda_console_dev",
            ),
        ] {
            let mut headers = HeaderMap::new();
            headers.append(
                header::COOKIE,
                format!("{name}=first").parse().expect("cookie header"),
            );
            headers.append(
                header::COOKIE,
                format!("{name}=second").parse().expect("cookie header"),
            );
            assert!(console_cookie(&headers, mode).is_none(), "mode: {mode:?}");
        }
    }
}

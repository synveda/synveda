//! The unauthenticated auth plane (AUTH-1, ADR-0010): `GET /auth/login`
//! redirects to the IdP with a code+PKCE challenge; `GET /auth/callback`
//! completes the login and returns the Synveda session material (subject,
//! tenant, and the access token to present as the `/v1` bearer).
//!
//! AUD-1 wiring point: login completions and rejections become audit events
//! when the hash-chained log lands; until then they are visible in traces
//! and `synveda_oidc_logins_total` only (ADR-0010 compliance notes).

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Redirect, Response};
use serde::{Deserialize, Serialize};
use synveda_types::{Error, Tenant};

use crate::app::AppState;
use crate::error::ApiError;
use crate::tenant;

/// Query parameters for `GET /auth/login`.
#[derive(Deserialize)]
pub struct LoginParams {
    /// Which configured issuer to log in against; optional when exactly
    /// one is configured.
    issuer: Option<String>,
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
/// bearer credential for `/v1`.
#[derive(Serialize)]
struct SessionResponse {
    subject: String,
    tenant: Tenant,
    access_token: String,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
}

/// Starts a login: 302 to the IdP's authorization endpoint.
#[tracing::instrument(name = "auth.login", skip_all)]
pub async fn login(State(state): State<AppState>, Query(params): Query<LoginParams>) -> Response {
    let Some(flow) = &state.login else {
        return not_configured();
    };
    match flow.begin(params.issuer.as_deref()).await {
        Ok(url) => Redirect::temporary(&url).into_response(),
        Err(error) => ApiError(error).into_response(),
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
    if let Some(error) = params.error {
        // The IdP refused (user denied, policy, ...). The description is
        // trace detail; the caller gets the classification.
        tracing::debug!(
            error,
            description = params.error_description.as_deref().unwrap_or_default(),
            "authorization error returned by the IdP"
        );
        return ApiError(Error::Unauthenticated {
            message: format!("the identity provider reported: {error}"),
        })
        .into_response();
    }
    let (Some(code), Some(login_state)) = (params.code, params.state) else {
        return ApiError(Error::Invalid {
            message: "callback requires code and state".to_owned(),
        })
        .into_response();
    };
    let session = match flow.complete(&login_state, &code).await {
        Ok(session) => session,
        Err(error) => return ApiError(error).into_response(),
    };
    match tenant::active_tenant(&state, &session.claims).await {
        Ok(context) => Json(SessionResponse {
            subject: context.subject,
            tenant: context.tenant,
            access_token: session.access_token,
            token_type: session.token_type,
            expires_in: session.expires_in,
        })
        .into_response(),
        Err(error) => ApiError(error).into_response(),
    }
}

fn not_configured() -> Response {
    ApiError(Error::NotFound {
        entity: "OIDC login (no issuers configured on this gateway)".to_owned(),
    })
    .into_response()
}

//! Tenant resolution middleware (TEN-1, ADR-0008) — the second stage of the
//! gateway chain (seed §7: AuthN → tenant resolution → PDP → rate limits →
//! audit). Every `/v1` route runs behind it: a request either acquires an
//! ambient [`TenantContext`] or is rejected with a uniform 401.
//!
//! Audited since AUD-1 (ADR-0019 decision 6): a verified token naming a
//! tenant that refuses resolution (suspended) chains
//! `tenant.resolution.denied` on that tenant's log. Successful resolutions
//! are not events — every subsequent chained event proves resolution — and
//! unauthenticated failures without both a verified subject and a resolvable
//! tenant stay in traces and the counter. A verified service-audience token
//! that resolves to an active tenant but not an active service identity is
//! attributable, so that refusal is also chained without revealing why the
//! identity did not resolve.
//!
//! Since CNSL-1 (ADR-0056 decision 2) a request may arrive with a console
//! session cookie instead of an `Authorization` header. That changes how
//! the bearer is *found* and nothing else: the cookie names a stored access
//! token, and that token goes through the same [`TokenVerifier`] and the
//! same [`active_tenant`] as one a client presented itself. No `Claims`
//! value in this product is ever constructed from a session row.
//!
//! The one thing a cookie does change is that authority becomes ambient —
//! a browser attaches it to cross-site requests the user never intended —
//! so a cookie-authenticated mutation additionally has to prove intent
//! (decision 4, [`enforce_origin`]). A bearer never needs to: a header is
//! not something a cross-site form can set.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use serde_json::json;
use synveda_audit::{Actor, AuditAction, Outcome};
use synveda_identity::{Claims, CredentialClass, TenantContext, with_tenant};
use synveda_types::{Error, IdentityKind, Result, TenantStatus};

/// How close to expiry a stored access token gets renewed rather than
/// presented. Wide enough that a token does not expire between this check
/// and the verifier reading it.
const EXPIRY_SKEW_SECS: i64 = 30;

/// How stale `last_seen_at` has to be before a read advances it. A review
/// screen polls; a row rewritten on every poll turns the read path into a
/// write path for a column nothing reads in real time.
const TOUCH_STALENESS_SECS: i64 = 300;

use crate::app::{AppState, ConsoleCookieMode};
use crate::audit;
use crate::error::ApiError;
use crate::telemetry::{SERVICE_TOKEN_REJECTIONS_TOTAL, TENANT_RESOLUTIONS_TOTAL};

/// One non-oracular reason for every service-identity admission failure.
const SERVICE_IDENTITY_REJECTION_REASON: &str = "identity_unresolved";

/// Resolves the request's tenant from its bearer token, records the tenant id
/// on the request span (the traces half of the TEN-1 AC), and runs the rest
/// of the stack inside the task-local tenant scope. Any failure short-circuits
/// with the taxonomy's transport mapping — 401 for everything unresolvable.
pub async fn resolve_tenant(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    // Only the headers cross the await: `Body` is !Sync, so borrowing the
    // whole request here would make this future non-Send.
    match resolve_with_transport(&state, request.method(), request.headers()).await {
        Ok(context) => {
            // The middleware runs inside the TraceLayer request span, so
            // this lands on the `tenant.id` field declared there.
            tracing::Span::current()
                .record("tenant.id", tracing::field::display(context.tenant.id));
            metrics::counter!(TENANT_RESOLUTIONS_TOTAL, "outcome" => "resolved").increment(1);
            with_tenant(context, next.run(request)).await
        }
        Err(error) => {
            let outcome = match &error {
                Error::Unauthenticated { .. } => "rejected",
                _ => "error",
            };
            metrics::counter!(TENANT_RESOLUTIONS_TOTAL, "outcome" => outcome).increment(1);
            ApiError(error).into_response()
        }
    }
}

/// Bearer token → verified claims → active tenant row. Unknown, suspended,
/// and missing are deliberately the same 401: the gateway is not an
/// existence oracle (ADR-0008).
#[tracing::instrument(name = "tenant.resolve", skip_all)]
async fn resolve_with_transport(
    state: &AppState,
    method: &Method,
    headers: &HeaderMap,
) -> Result<TenantContext> {
    // A presented header wins over a cookie. The console is the only caller
    // that has both to offer, and a client that went to the trouble of
    // sending a bearer meant that bearer; silently preferring an ambient
    // cookie would make "which credential did this act under" a question
    // the answer to depends on header order.
    match bearer_token(headers) {
        Ok(token) => {
            let claims = state.verifier.verify(token).await?;
            active_tenant(state, &claims).await
        }
        Err(missing) => {
            let cookie_mode = state
                .login
                .as_ref()
                .map_or(ConsoleCookieMode::Https, |login| login.cookie_mode());
            let Some(secret) = crate::auth::console_cookie(headers, cookie_mode) else {
                return Err(missing);
            };
            // Ambient authority: prove intent before the credential is
            // worth anything (ADR-0056 decision 4).
            enforce_origin(state, method, headers)?;
            let token = console_bearer(state, secret).await?;
            let claims = state.verifier.verify(&token).await?;
            active_tenant(state, &claims).await
        }
    }
}

/// Resolves a console cookie to the access token it names, renewing that
/// token first if it is at or past its expiry (ADR-0056 decision 3).
///
/// Returns the same [`Error::Unauthenticated`] for an unknown session, an
/// expired one, and one whose refresh the issuer refused. A caller learning
/// which of the three it hit learns whether a session id it guessed exists.
async fn console_bearer(state: &AppState, secret: &str) -> Result<String> {
    let unauthenticated = || Error::Unauthenticated {
        message: "console session is not valid".to_owned(),
    };
    let hash = synveda_identity::console::hash(secret);
    let session = synveda_store::console_sessions::by_hash(&state.pool, &hash)
        .await?
        .ok_or_else(unauthenticated)?;

    // Still good: hand back what is stored. `None` means the issuer
    // reported no lifetime, in which case the gateway is the authority on
    // expiry and finds out by verifying (the ADPT-1 rule).
    let expiring = session
        .access_expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now() + Duration::seconds(EXPIRY_SKEW_SECS));
    if !expiring {
        touch(state, &hash, &session).await;
        // A token that does not open is the same 401 as a session that does
        // not exist. The distinction is in the metric and the log
        // (`AppState::open_console_token`), not in the response — a caller
        // learning that a session id it guessed exists but is corrupt has
        // still learned that it exists.
        return state
            .open_console_token(
                &hash,
                synveda_crypto::Purpose::ConsoleAccessToken,
                &session.access_token_sealed,
            )
            .await
            .map_err(|_| unauthenticated());
    }

    let (Some(refresh_sealed), Some(flow)) =
        (session.refresh_token_sealed.as_deref(), &state.login)
    else {
        // Expired with no way to renew. Not an error worth distinguishing:
        // the operator logs in again either way.
        return Err(unauthenticated());
    };
    let refresh_token = state
        .open_console_token(
            &hash,
            synveda_crypto::Purpose::ConsoleRefreshToken,
            refresh_sealed,
        )
        .await
        .map_err(|_| unauthenticated())?;
    let renewed = flow
        .refresh(Some(&session.issuer), &refresh_token)
        .await
        .map_err(|error| {
            tracing::debug!(%error, "console session refresh refused");
            unauthenticated()
        })?;
    let access_expires_at = renewed
        .expires_in
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| Utc::now().checked_add_signed(Duration::seconds(seconds)));
    // Re-sealed under whatever the *current* key generation is, which is how
    // a rotation reaches this column without a rewrite pass: a session that
    // refreshes moves forward, one that does not ages out under its own cap
    // (ADR-0064 decision 6).
    let access_token_sealed = state
        .seal_console_token(
            &hash,
            synveda_crypto::Purpose::ConsoleAccessToken,
            &renewed.access_token,
        )
        .await?;
    let refresh_token_sealed = match renewed.refresh_token.as_deref() {
        Some(token) => Some(
            state
                .seal_console_token(&hash, synveda_crypto::Purpose::ConsoleRefreshToken, token)
                .await?,
        ),
        None => None,
    };
    synveda_store::console_sessions::renew(
        &state.pool,
        &hash,
        &access_token_sealed,
        access_expires_at,
        refresh_token_sealed.as_deref(),
    )
    .await?;
    metrics::counter!(crate::auth::CONSOLE_SESSIONS_TOTAL, "outcome" => "renewed").increment(1);
    Ok(renewed.access_token)
}

/// Advances `last_seen_at` on a coarse cadence. Best-effort: a session that
/// works is not made not-to-work by a bookkeeping write that failed.
async fn touch(
    state: &AppState,
    hash: &[u8; 32],
    session: &synveda_store::console_sessions::ConsoleSession,
) {
    if session.last_seen_at > Utc::now() - Duration::seconds(TOUCH_STALENESS_SECS) {
        return;
    }
    if let Err(error) = synveda_store::console_sessions::touch(
        &state.pool,
        hash,
        Duration::seconds(TOUCH_STALENESS_SECS),
    )
    .await
    {
        tracing::debug!(%error, "could not advance a console session's last_seen_at");
    }
}

/// The CSRF defence for cookie-authenticated mutations (ADR-0056
/// decision 4).
///
/// `SameSite=Strict` is the first line and this is the second, because the
/// first is a promise made by software we do not ship. Safe methods are
/// exempt — a cross-site `GET` discloses nothing a browser will let the
/// attacker read, and requiring a header on them would break ordinary
/// navigation to the console itself.
///
/// A **missing** `Origin` on a mutation is refused rather than allowed.
/// Every browser has sent `Origin` on cross-origin requests for years and
/// on same-origin non-GET requests since 2020; the callers that omit it are
/// not browsers, and a caller that is not a browser has no business
/// authenticating with a cookie when a bearer is right there.
fn enforce_origin(state: &AppState, method: &Method, headers: &HeaderMap) -> Result<()> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let refuse = || Error::Unauthenticated {
        message: "cookie-authenticated request must carry a matching Origin".to_owned(),
    };
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(refuse)?;
    if origin == state.public_origin {
        Ok(())
    } else {
        metrics::counter!(crate::auth::CONSOLE_SESSIONS_TOTAL, "outcome" => "origin_refused")
            .increment(1);
        Err(refuse())
    }
}

/// Verified claims → active tenant row → context. Shared by this middleware
/// and the login callback (AUTH-1): TEN-1's uniform-401 doctrine applies to
/// both entry points identically.
pub(crate) async fn active_tenant(state: &AppState, claims: &Claims) -> Result<TenantContext> {
    let unresolved = || Error::Unauthenticated {
        message: "token does not resolve to an active tenant".to_owned(),
    };
    let Some(tenant) = synveda_store::tenants::by_id(&state.pool, claims.tenant_id).await? else {
        // Unknown tenant: nothing attributable, no chain to write to
        // (ADR-0019 decision 6). The uniform 401 stays uniform.
        return Err(unresolved());
    };
    if tenant.status != TenantStatus::Active {
        // A verified subject named a real but suspended tenant — that
        // tenant's auditors get the attempt on their chain. Best-effort:
        // the uniform 401 is returned either way.
        audit::record_detached(
            state,
            tenant.id,
            Actor::subject(claims.subject.clone()),
            AuditAction::TenantResolutionDenied,
            format!("tenant {}", tenant.id),
            Outcome::Deny,
            json!({"status": tenant.status}),
        )
        .await;
        return Err(unresolved());
    }
    if claims.credential_class == CredentialClass::ServiceBearer {
        let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant.id).await?;
        let service = synveda_store::identities::by_subject(&mut *tx, tenant.id, &claims.subject)
            .await?
            .is_some_and(|identity| identity.kind == IdentityKind::Service && !identity.sealed());
        // Detached audit opens its own tenant transaction. Release this
        // read-only lookup first so a one-connection pool cannot hold and
        // wait on itself while recording the refusal.
        drop(tx);
        if !service {
            metrics::counter!(
                SERVICE_TOKEN_REJECTIONS_TOTAL,
                "reason" => SERVICE_IDENTITY_REJECTION_REASON,
            )
            .increment(1);
            tracing::debug!(
                tenant.id = %tenant.id,
                "service-audience token did not resolve to an active service identity"
            );
            audit::record_detached(
                state,
                tenant.id,
                Actor::subject(claims.subject.clone()),
                AuditAction::TokenRejected,
                format!("tenant {}", tenant.id),
                Outcome::Deny,
                json!({
                    "op": "tenant.resolve",
                    "reason": SERVICE_IDENTITY_REJECTION_REASON,
                }),
            )
            .await;
            return Err(unresolved());
        }
    }
    Ok(TenantContext {
        tenant,
        claims: claims.clone(),
    })
}

fn bearer_token(headers: &HeaderMap) -> Result<&str> {
    let unauthenticated = |message: &str| Error::Unauthenticated {
        message: message.to_owned(),
    };
    let value = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| unauthenticated("missing Authorization header"))?
        .to_str()
        .map_err(|_| unauthenticated("malformed Authorization header"))?;
    // RFC 9110: the auth scheme name is case-insensitive.
    let (scheme, token) = value
        .split_once(' ')
        .ok_or_else(|| unauthenticated("malformed Authorization header"))?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(unauthenticated("Authorization scheme must be Bearer"));
    }
    let token = token.trim();
    if token.is_empty() {
        return Err(unauthenticated("empty bearer token"));
    }
    Ok(token)
}

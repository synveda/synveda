//! Tenant resolution middleware (TEN-1, ADR-0008) — the second stage of the
//! gateway chain (seed §7: AuthN → tenant resolution → PDP → rate limits →
//! audit). Every `/v1` route runs behind it: a request either acquires an
//! ambient [`TenantContext`] or is rejected with a uniform 401.
//!
//! AUD-1 wiring point: resolution decisions (allow and reject) become audit
//! events when the hash-chained log lands; until then they are visible in
//! traces and the `synveda_tenant_resolutions_total` counter only.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use synveda_identity::{Claims, TenantContext, with_tenant};
use synveda_types::{Error, Result, TenantStatus};

use crate::app::AppState;
use crate::error::ApiError;
use crate::telemetry::TENANT_RESOLUTIONS_TOTAL;

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
    match resolve(&state, request.headers()).await {
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
async fn resolve(state: &AppState, headers: &HeaderMap) -> Result<TenantContext> {
    let token = bearer_token(headers)?;
    let claims = state.verifier.verify(token).await?;
    active_tenant(state, &claims).await
}

/// Verified claims → active tenant row → context. Shared by this middleware
/// and the login callback (AUTH-1): TEN-1's uniform-401 doctrine applies to
/// both entry points identically.
pub(crate) async fn active_tenant(state: &AppState, claims: &Claims) -> Result<TenantContext> {
    let tenant = synveda_store::tenants::by_id(&state.pool, claims.tenant_id)
        .await?
        .filter(|tenant| tenant.status == TenantStatus::Active)
        .ok_or_else(|| Error::Unauthenticated {
            message: "token does not resolve to an active tenant".to_owned(),
        })?;
    Ok(TenantContext {
        tenant,
        subject: claims.subject.clone(),
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

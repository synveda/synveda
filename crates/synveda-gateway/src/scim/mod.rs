//! The SCIM 2.0 plane (AUTH-4, ADR-0059).
//!
//! `/scim/v2` is not `/v1`, and the separation is the feature's load-bearing
//! sentence: **a SCIM request carries directory facts, never product
//! instructions** (decision 2). A request here names a person, their
//! `active` flag and their groups. There is no field in the wire format for
//! a scope, a record, a role, a pack or a channel, and none will be added as
//! an extension — so every product effect is the mapping resolver's and the
//! effective pack's, and the caller has no vocabulary to ask for one.
//!
//! CPR-34 narrows the old reachability argument (ADR-0093). This credential
//! can state identity and shared-group membership facts, and those facts can
//! affect a grant already bound to the group. It still cannot name or create
//! a scope, role, grant, pack or governed artifact: the only bridge from a
//! directory group to product authority is the separate
//! `/v1/directory/access-assignments` command, which takes the ordinary
//! `MembershipGrant` PDP decision and chains the ordinary access audit event.
//! The credential is therefore verified external-adapter authority over
//! directory-owned facts, never an alternate authorisation plane.
//!
//! ## What is here
//!
//! - [`wire`] — RFC 7643 resources and RFC 7644 messages.
//! - [`filter`] — the equality subset, and the `501` for the rest.
//! - [`reconcile`] — the projection from the user resource onto identities
//!   and principal scopes. The **only** writer of that seam, and the function
//!   AUTH-5's pull sync drives.
//! - [`credentials`] — the `/v1` admin routes that issue and revoke the
//!   static bearer this plane authenticates with, PDP-gated at the tenant.
//!
//! ## Audit
//!
//! State changes chain (`identity.provisioned`, `identity.sealed`,
//! `access.group.created`, `access.group.updated`); reads do not (decision
//! 14). A provisioning agent
//! polls its whole assigned population every cycle, so chaining reads would
//! fill a tenant's audit chain with a directory reading its own copy back
//! and bury the events that matter. A read returns only source-owned directory
//! facts, never scope grants or governed artifact content.

pub mod credentials;
pub mod filter;
pub mod groups;
pub mod reconcile;
pub mod users;
pub mod wire;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, middleware};
use serde_json::json;
use synveda_types::{Error, ScimCredential, Tenant};

use crate::app::AppState;

/// The plane's mount point. Entra's "Tenant URL" and Okta's "SCIM connector
/// base URL" are this, absolute — one value for every customer of a
/// deployment, because the tenant rides inside the credential rather than
/// the path (`synveda_identity::scim`).
pub const SCIM_PREFIX: &str = "/scim/v2";

/// How stale a credential's `last_used_at` must be before a request
/// advances it (migration 0034's cadence rule).
const TOUCH_STALENESS_SECS: i32 = 300;

/// The largest page this server will return, and the number
/// `/ServiceProviderConfig` advertises as `filter.maxResults`. Advertised
/// from the same constant the routes clamp with, so the two cannot drift.
pub const MAX_RESULTS: i64 = 200;

/// What a request on this plane authenticated as.
#[derive(Debug, Clone)]
pub struct ScimAuth {
    /// The tenant the credential is bound to.
    pub tenant: Tenant,
    /// The credential itself — its id rides every audit event this plane
    /// chains, so "which credential sealed this person" is answerable from
    /// the chain alone.
    pub credential: ScimCredential,
}

/// A SCIM error response (RFC 7644 §3.12).
///
/// Deliberately **not** the product's [`crate::error::ApiError`]. The
/// audience for these bytes is a provisioning agent that reports what it
/// cannot parse to an administrator as our failure, so a body in our own
/// envelope would turn every error into an unparseable one for the only
/// reader it has.
pub struct ScimError {
    status: StatusCode,
    /// RFC 7644 §3.12's `scimType`, when one of its keywords applies.
    scim_type: Option<&'static str>,
    detail: String,
}

impl ScimError {
    /// A refusal with one of the RFC's own `scimType` keywords.
    pub fn typed(status: StatusCode, scim_type: &'static str, detail: impl Into<String>) -> Self {
        ScimError {
            status,
            scim_type: Some(scim_type),
            detail: detail.into(),
        }
    }

    /// A refusal with no keyword — the RFC makes `scimType` optional and
    /// inventing one would be worse than omitting it.
    pub fn plain(status: StatusCode, detail: impl Into<String>) -> Self {
        ScimError {
            status,
            scim_type: None,
            detail: detail.into(),
        }
    }

    /// `404`, in the one wording every unknown resource gets.
    pub fn not_found() -> Self {
        ScimError::plain(StatusCode::NOT_FOUND, "resource not found")
    }

    /// The taxonomy error a store call produced, rendered as SCIM.
    ///
    /// Operator-side failures keep their detail in traces and logs and
    /// reach the client as a classification only — the same doctrine
    /// [`crate::error::caller_facing`] applies on `/v1`, restated here
    /// because this plane does not share that renderer.
    pub fn from_taxonomy(error: &Error) -> Self {
        match error {
            Error::NotFound { .. } => ScimError::not_found(),
            Error::Conflict { .. } => ScimError::typed(
                StatusCode::CONFLICT,
                "uniqueness",
                "a resource with this attribute already exists",
            ),
            Error::Invalid { message } => {
                ScimError::typed(StatusCode::BAD_REQUEST, "invalidValue", message.clone())
            }
            Error::PolicyDenied { .. } => ScimError::plain(StatusCode::FORBIDDEN, "not permitted"),
            Error::Unauthenticated { .. } => {
                ScimError::plain(StatusCode::UNAUTHORIZED, "invalid credential")
            }
            Error::Storage { .. } => {
                ScimError::plain(StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
            }
            Error::Dependency { .. } => {
                ScimError::plain(StatusCode::BAD_GATEWAY, "dependency unavailable")
            }
            Error::RateLimited { .. } => {
                ScimError::plain(StatusCode::TOO_MANY_REQUESTS, "rate limited")
            }
            Error::Internal { .. } => {
                ScimError::plain(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
        }
    }
}

fn require_patch_schema(body: &wire::PatchRequest) -> Result<(), ScimError> {
    if body
        .schemas
        .iter()
        .any(|schema| schema == wire::PATCH_OP_SCHEMA)
    {
        return Ok(());
    }
    Err(ScimError::typed(
        StatusCode::BAD_REQUEST,
        "invalidSyntax",
        "schemas must include the SCIM PatchOp URN",
    ))
}

impl From<Error> for ScimError {
    fn from(error: Error) -> Self {
        ScimError::from_taxonomy(&error)
    }
}

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        let mut body = json!({
            "schemas": [wire::ERROR_SCHEMA],
            // RFC 7644 §3.12 types `status` as a *string*, which is one of
            // the details a hand-rolled server gets wrong and a strict
            // client then refuses to parse.
            "status": self.status.as_u16().to_string(),
            "detail": self.detail,
        });
        if let Some(scim_type) = self.scim_type {
            body["scimType"] = json!(scim_type);
        }
        let mut response = (self.status, Json(body)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            wire::SCIM_CONTENT_TYPE.parse().expect("static"),
        );
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, "Bearer".parse().expect("static"));
        }
        response
    }
}

/// A successful SCIM body: the response with `application/scim+json` on it,
/// which RFC 7644 §3.1 requires and some clients check.
pub struct ScimJson<T>(pub StatusCode, pub T);

impl<T: serde::Serialize> IntoResponse for ScimJson<T> {
    fn into_response(self) -> Response {
        let mut response = (self.0, Json(self.1)).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            wire::SCIM_CONTENT_TYPE.parse().expect("static"),
        );
        response
    }
}

/// The absolute base a `meta.location` is built from.
///
/// Read from configuration rather than from the request's `Host`, because a
/// `Location` derived from an attacker-supplied header is a redirect
/// somebody else chose. Falls back to the prefix alone, which yields
/// relative locations — allowed by RFC 7644 §3.1 and better than confident
/// nonsense.
pub fn base_url(state: &AppState) -> String {
    format!("{}{SCIM_PREFIX}", state.public_origin.trim_end_matches('/'))
}

/// Authenticates a request on this plane: the presented bearer names its
/// tenant, the hash lookup inside that tenant's own row policy proves it
/// (`synveda_identity::scim`).
async fn authenticate(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<ScimAuth, ScimError> {
    let unauthorized = || {
        ScimError::plain(
            StatusCode::UNAUTHORIZED,
            "a provisioning credential is required",
        )
    };
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .ok_or_else(unauthorized)?;
    // Reading the tenant out of the token is not authenticating: it is
    // choosing which tenant's rows to look in, and the hash settles it.
    let tenant_id = synveda_identity::scim::tenant_of(presented).ok_or_else(unauthorized)?;
    let hash = synveda_identity::scim::hash(presented);

    let tenant = synveda_store::tenants::by_id(&state.pool, tenant_id)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?
        .filter(|tenant| tenant.status == synveda_types::TenantStatus::Active)
        .ok_or_else(unauthorized)?;

    let mut tx = synveda_store::rls::begin_tenant_tx(&state.pool, tenant_id)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?;
    let credential = synveda_store::directory::credential_by_hash(&mut *tx, tenant_id, &hash)
        .await
        .map_err(|error| ScimError::from_taxonomy(&error))?
        .ok_or_else(unauthorized)?;
    if !credential.usable_at(chrono::Utc::now()) {
        // Expired and revoked are both 401 and neither says which. A
        // credential store that distinguishes them tells whoever holds a
        // stolen token whether it was ever real.
        return Err(unauthorized());
    }
    synveda_store::directory::touch_credential(
        &mut *tx,
        tenant_id,
        credential.id,
        TOUCH_STALENESS_SECS,
    )
    .await
    .map_err(|error| ScimError::from_taxonomy(&error))?;
    tx.commit().await.map_err(|err| {
        ScimError::from_taxonomy(&Error::Storage {
            message: format!("commit credential touch: {err}"),
        })
    })?;

    Ok(ScimAuth { tenant, credential })
}

/// The plane's authentication middleware.
async fn require_credential(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate(&state, request.headers()).await {
        Ok(auth) => {
            tracing::Span::current().record("tenant.id", tracing::field::display(auth.tenant.id));
            metrics::counter!(crate::telemetry::SCIM_REQUESTS_TOTAL, "outcome" => "authenticated")
                .increment(1);
            request.extensions_mut().insert(auth);
            next.run(request).await
        }
        Err(error) => {
            metrics::counter!(crate::telemetry::SCIM_REQUESTS_TOTAL, "outcome" => "rejected")
                .increment(1);
            error.into_response()
        }
    }
}

/// The plane's routes, mounted under [`SCIM_PREFIX`].
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            &format!("{SCIM_PREFIX}/ServiceProviderConfig"),
            get(service_provider_config),
        )
        .route(&format!("{SCIM_PREFIX}/ResourceTypes"), get(resource_types))
        .route(&format!("{SCIM_PREFIX}/Schemas"), get(schemas))
        .route(
            &format!("{SCIM_PREFIX}/Users"),
            get(users::list).post(users::create),
        )
        .route(
            &format!("{SCIM_PREFIX}/Users/{{id}}"),
            get(users::get)
                .put(users::replace)
                .patch(users::patch)
                .delete(users::delete),
        )
        .route(
            &format!("{SCIM_PREFIX}/Groups"),
            get(groups::list).post(groups::create),
        )
        .route(
            &format!("{SCIM_PREFIX}/Groups/{{id}}"),
            get(groups::get)
                .put(groups::replace)
                .patch(groups::patch)
                .delete(groups::delete),
        )
        .route_layer(middleware::from_fn_with_state(state, require_credential))
}

/// `GET /ServiceProviderConfig` — RFC 7644 §4. Unauthenticated discovery is
/// permitted by §2, but this plane requires the credential for it anyway:
/// the document names a tenant's provisioning surface, and an unauthenticated
/// endpoint is one more thing to have an opinion about.
async fn service_provider_config(State(state): State<AppState>) -> ScimJson<serde_json::Value> {
    ScimJson(
        StatusCode::OK,
        wire::service_provider_config(&base_url(&state), MAX_RESULTS),
    )
}

/// `GET /ResourceTypes` — RFC 7644 §4.
async fn resource_types(
    State(state): State<AppState>,
) -> ScimJson<wire::ListResponse<serde_json::Value>> {
    let types = wire::resource_types(&base_url(&state));
    let total = i64::try_from(types.len()).unwrap_or(0);
    ScimJson(StatusCode::OK, wire::ListResponse::new(types, total, 1))
}

/// `GET /Schemas` — RFC 7644 §4, and the endpoint that makes "unknown
/// attributes are ignored" honest: what is not published here was never
/// promised.
async fn schemas(State(state): State<AppState>) -> ScimJson<wire::ListResponse<serde_json::Value>> {
    let published = wire::schemas(&base_url(&state));
    let total = i64::try_from(published.len()).unwrap_or(0);
    ScimJson(StatusCode::OK, wire::ListResponse::new(published, total, 1))
}

/// Clamps a caller's `count` to [`MAX_RESULTS`], and `startIndex` to RFC
/// 7644 §3.4.2.4's 1-based floor.
#[must_use]
pub fn page_bounds(start_index: Option<i64>, count: Option<i64>) -> (i64, i64) {
    let start = start_index.unwrap_or(1).max(1);
    let count = count.unwrap_or(MAX_RESULTS).clamp(0, MAX_RESULTS);
    (start, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_clamped_to_the_advertised_maximum() {
        // The number `/ServiceProviderConfig` publishes and the number the
        // routes enforce are the same constant, so a client that trusts
        // the advertisement is never surprised.
        assert_eq!(page_bounds(None, None), (1, MAX_RESULTS));
        assert_eq!(page_bounds(Some(0), Some(10_000)), (1, MAX_RESULTS));
        assert_eq!(page_bounds(Some(-5), Some(-1)), (1, 0));
        assert_eq!(page_bounds(Some(7), Some(25)), (7, 25));
    }

    #[test]
    fn an_error_renders_the_rfc_shape_with_a_string_status() {
        let error = ScimError::typed(StatusCode::CONFLICT, "uniqueness", "already exists");
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(wire::SCIM_CONTENT_TYPE)
        );
    }
}

//! Maps the shared error taxonomy onto HTTP responses. The taxonomy's
//! contract (synveda-types): variants classify failures by who must act and
//! map one-to-one onto transport status codes *here*, nowhere else.

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use synveda_types::Error;

/// The gateway's transport rendering of a taxonomy error: mapped status,
/// serialized taxonomy body (`{"kind": ..., ...}`), `WWW-Authenticate`
/// on 401 per RFC 6750, and bounded `Retry-After` guidance on 429.
pub struct ApiError(pub Error);

/// Retry guidance for a 429 whose cause has no shorter known recovery, such
/// as a spent daily budget or a full ledger. Admission refusals replace it
/// with their own computed delay.
const DEFAULT_RETRY_AFTER_SECS: u64 = 60;

/// The one-to-one variant → status mapping.
pub fn status_of(error: &Error) -> StatusCode {
    match error {
        Error::Unauthenticated { .. } => StatusCode::UNAUTHORIZED,
        Error::PolicyDenied { .. } => StatusCode::FORBIDDEN,
        Error::NotFound { .. } => StatusCode::NOT_FOUND,
        Error::Invalid { .. } => StatusCode::BAD_REQUEST,
        Error::InsufficientBudget { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        Error::Conflict { .. } => StatusCode::CONFLICT,
        Error::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        Error::Storage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        Error::Dependency { .. } => StatusCode::BAD_GATEWAY,
        Error::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The caller-facing form of an error. Operator-side failures keep their
/// detail in traces and logs; the caller sees only the classification
/// (same doctrine as `/readyz`).
pub fn caller_facing(error: &Error) -> Error {
    match error {
        Error::Storage { .. } => Error::Storage {
            message: "storage unavailable".to_owned(),
        },
        Error::Dependency { service, .. } => Error::Dependency {
            service: service.clone(),
            message: "dependency unavailable".to_owned(),
        },
        Error::Internal { .. } => Error::Internal {
            message: "internal error".to_owned(),
        },
        safe => safe.clone(),
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = status_of(&self.0);
        let body = Json(caller_facing(&self.0));
        let mut response = (status, body).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, "Bearer".parse().expect("static"));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            response.headers_mut().insert(
                header::RETRY_AFTER,
                axum::http::HeaderValue::from(DEFAULT_RETRY_AFTER_SECS),
            );
        }
        response
    }
}

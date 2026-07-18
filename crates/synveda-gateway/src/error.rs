//! Maps the shared error taxonomy onto HTTP responses. The taxonomy's
//! contract (synveda-types): variants classify failures by who must act and
//! map one-to-one onto transport status codes *here*, nowhere else.

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use synveda_types::Error;

/// The gateway's transport rendering of a taxonomy error: mapped status,
/// serialized taxonomy body (`{"kind": ..., ...}`), and `WWW-Authenticate`
/// on 401 per RFC 6750.
pub struct ApiError(pub Error);

/// The one-to-one variant → status mapping.
pub fn status_of(error: &Error) -> StatusCode {
    match error {
        Error::Unauthenticated { .. } => StatusCode::UNAUTHORIZED,
        Error::PolicyDenied { .. } => StatusCode::FORBIDDEN,
        Error::NotFound { .. } => StatusCode::NOT_FOUND,
        Error::Invalid { .. } => StatusCode::BAD_REQUEST,
        Error::Conflict { .. } => StatusCode::CONFLICT,
        Error::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
        Error::Storage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        Error::Dependency { .. } => StatusCode::BAD_GATEWAY,
        Error::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = status_of(&self.0);
        // Operator-side failures keep their detail in traces and logs; the
        // caller sees only the classification (same doctrine as /readyz).
        let body = match &self.0 {
            Error::Storage { .. } => Json(Error::Storage {
                message: "storage unavailable".to_owned(),
            }),
            Error::Dependency { service, .. } => Json(Error::Dependency {
                service: service.clone(),
                message: "dependency unavailable".to_owned(),
            }),
            Error::Internal { .. } => Json(Error::Internal {
                message: "internal error".to_owned(),
            }),
            caller_facing => Json(caller_facing.clone()),
        };
        let mut response = (status, body).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, "Bearer".parse().expect("static"));
        }
        response
    }
}

//! Common response finishing for authenticated application handlers.

use axum::response::{IntoResponse, Response};
use synveda_types::{Error, Result};

use crate::app::AppState;
use crate::{audit, error::ApiError};

/// Classifies an operation result for each feature's bounded-cardinality
/// counter. Metric names and label keys remain at the feature call site.
pub(crate) fn outcome<T>(result: &Result<T>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(
            Error::Unauthenticated { .. }
            | Error::PolicyDenied { .. }
            | Error::NotFound { .. }
            | Error::Invalid { .. }
            | Error::Conflict { .. }
            | Error::RateLimited { .. },
        ) => "rejected",
        Err(Error::Storage { .. } | Error::Dependency { .. } | Error::Internal { .. }) => "error",
    }
}

/// Converts a handler result after its feature metric has been recorded.
pub(crate) async fn finish<T: IntoResponse>(
    state: &AppState,
    operation: &'static str,
    result: Result<T>,
) -> Response {
    match result {
        Ok(value) => value.into_response(),
        Err(error) => {
            audit::record_rejection(state, operation, &error).await;
            ApiError(error).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_outcomes_follow_the_error_taxonomy() {
        let rejected = [
            Error::Unauthenticated {
                message: String::new(),
            },
            Error::PolicyDenied {
                action: String::new(),
                resource: String::new(),
                reason: String::new(),
            },
            Error::NotFound {
                entity: String::new(),
            },
            Error::Invalid {
                message: String::new(),
            },
            Error::Conflict {
                message: String::new(),
            },
            Error::RateLimited {
                message: String::new(),
            },
        ];
        let failed = [
            Error::Storage {
                message: String::new(),
            },
            Error::Dependency {
                service: String::new(),
                message: String::new(),
            },
            Error::Internal {
                message: String::new(),
            },
        ];

        assert_eq!(outcome(&Ok::<_, Error>(())), "ok");
        for error in rejected {
            assert_eq!(outcome(&Err::<(), _>(error)), "rejected");
        }
        for error in failed {
            assert_eq!(outcome(&Err::<(), _>(error)), "error");
        }
    }
}

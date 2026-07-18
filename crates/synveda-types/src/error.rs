//! The error taxonomy shared by every Synveda crate.

use serde::{Deserialize, Serialize};

/// Result alias used across the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The Synveda error taxonomy.
///
/// Variants are deliberately coarse: they classify a failure by who must act
/// (the caller, an operator, or us) and map one-to-one onto transport status
/// codes at the gateway. Layer-specific detail belongs in the message fields,
/// not in new variants.
///
/// Every variant is plain data — no source-error chaining — so errors
/// serialize losslessly into audit events and across process boundaries.
/// Lower layers convert their native errors (sqlx, HTTP, ...) at the boundary
/// and put the detail in the message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Error {
    /// The caller's identity could not be established (missing, expired, or
    /// invalid credentials).
    #[error("unauthenticated: {message}")]
    Unauthenticated {
        /// What was wrong with the credentials, safe to return to the caller.
        message: String,
    },

    /// The PDP evaluated the request and denied it (seed §2.2). Fields carry
    /// the attempted action so the audit event can record the decision
    /// verbatim; denial reasons are policy names, never content.
    #[error("policy denied {action} on {resource}: {reason}")]
    PolicyDenied {
        /// The action that was attempted, e.g. `inject`, `recall`, `promote`.
        action: String,
        /// The resource acted upon, e.g. a record or scope reference.
        resource: String,
        /// Which policy produced the denial.
        reason: String,
    },

    /// The referenced entity does not exist — or is not visible to this
    /// caller. The gateway never distinguishes the two cases, so an ID probe
    /// cannot be used to confirm existence across scopes.
    #[error("not found: {entity}")]
    NotFound {
        /// What was looked up, e.g. `record 0198…`, `scope team-billing`.
        entity: String,
    },

    /// The request was understood but is invalid (bad field, unknown enum
    /// value, budget out of range, ...).
    #[error("invalid: {message}")]
    Invalid {
        /// What failed validation and why.
        message: String,
    },

    /// The operation conflicts with current state: duplicate creation,
    /// concurrent modification, or a stale ref in a VedaFlow proposal.
    #[error("conflict: {message}")]
    Conflict {
        /// The conflicting state.
        message: String,
    },

    /// The caller exceeded a rate or quota limit and should back off.
    #[error("rate limited: {message}")]
    RateLimited {
        /// Which limit was hit.
        message: String,
    },

    /// A storage backend failed (Postgres, vector index, graph). Operator
    /// concern, never the caller's fault.
    #[error("storage: {message}")]
    Storage {
        /// The failure, with the backend's own error rendered into text.
        message: String,
    },

    /// An upstream dependency failed (IdP, embedding service, Temporal, ...).
    #[error("dependency {service}: {message}")]
    Dependency {
        /// Which dependency, e.g. `oidc-provider`, `tei`.
        service: String,
        /// The failure as reported by the dependency.
        message: String,
    },

    /// A broken internal invariant — always a bug in Synveda.
    #[error("internal: {message}")]
    Internal {
        /// The violated invariant.
        message: String,
    },
}

impl Error {
    /// Stable machine-readable code, identical to the serde `kind` tag.
    /// Audit events and API error bodies carry this string; changing an
    /// existing code is a breaking change.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Error::Unauthenticated { .. } => "unauthenticated",
            Error::PolicyDenied { .. } => "policy_denied",
            Error::NotFound { .. } => "not_found",
            Error::Invalid { .. } => "invalid",
            Error::Conflict { .. } => "conflict",
            Error::RateLimited { .. } => "rate_limited",
            Error::Storage { .. } => "storage",
            Error::Dependency { .. } => "dependency",
            Error::Internal { .. } => "internal",
        }
    }
}

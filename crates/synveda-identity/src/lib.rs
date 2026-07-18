//! Identity: OIDC client (code + PKCE), SCIM 2.0, directory sync, and automatic
//! hierarchy provisioning from IdP claims and groups (seed §5).
//!
//! What exists today (TEN-1, ADR-0008): the [`TokenVerifier`] AuthN seam with
//! the pre-AUTH-1 HS256 dev implementation, and the tenant context task-local
//! the gateway propagates per request. OIDC/JWKS verification lands with
//! AUTH-1; provisioning with AUTH-2.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod context;
mod token;

pub use context::{TenantContext, current_tenant, with_tenant};
pub use token::{Claims, DisabledVerifier, Hs256Verifier, TokenVerifier};

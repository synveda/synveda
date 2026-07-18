//! Identity: OIDC client (code + PKCE), SCIM 2.0, directory sync, and automatic
//! hierarchy provisioning from IdP claims and groups (seed §5).
//!
//! What exists today: the [`TokenVerifier`] AuthN seam (TEN-1, ADR-0008)
//! with the [`OidcVerifier`] JWKS implementation and the code+PKCE
//! [`LoginFlow`] (AUTH-1, ADR-0010), the HS256 dev verifier for CLI/demo
//! bootstrap, and the tenant context task-local the gateway propagates per
//! request. JIT provisioning lands with AUTH-2.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod context;
mod flow;
mod oidc;
mod token;

pub use context::{TenantContext, current_tenant, with_tenant};
pub use flow::{LoginFlow, LoginSession, OIDC_LOGINS_TOTAL};
pub use oidc::{
    IssuerConfig, JWKS_REFRESHES_TOTAL, OidcVerifier, TOKEN_VERIFICATIONS_TOTAL, TenantBinding,
    parse_issuers,
};
pub use token::{Claims, DisabledVerifier, Hs256Verifier, TokenVerifier};

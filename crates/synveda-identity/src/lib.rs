//! Identity: OIDC client (code + PKCE), SCIM 2.0, directory sync, and
//! automatic provisioning from IdP claims and groups (seed §5).
//!
//! What exists today: the [`TokenVerifier`] AuthN seam (TEN-1, ADR-0008)
//! with the [`OidcVerifier`] JWKS implementation and the code+PKCE
//! [`LoginFlow`] (AUTH-1, ADR-0010), its CLI-mediated loopback variant and
//! refresh-token redemption (ADPT-1, ADR-0027 decisions 5 and 6), the
//! HS256 dev verifier for CLI/demo bootstrap, the tenant context
//! task-local the gateway propagates per request, and the
//! provisioning-claims contract plus the one IdP-group convention JIT
//! provisioning rides on (AUTH-2, ADR-0013; CPR-7, ADR-0074 decisions 3
//! and 4 — the gateway orchestrates the storage half, and placement is
//! identity rather than a group-name convention).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod console;
mod context;
pub mod directory;
mod flow;
// The invitation token (CPR-5, ADR-0072 decision 5): the same mint/hash/
// show-once shape as `scim`, for the same reason — it is a bearer credential
// that mints access.
pub mod invite;
mod mapping;
mod oidc;
pub mod scim;
mod token;

pub use context::{TenantContext, current_tenant, with_tenant};
pub use flow::{
    CliHandoff, LoginDestination, LoginFlow, LoginSession, OIDC_LOGINS_TOTAL, OIDC_REFRESHES_TOTAL,
    RefreshedSession, validate_cli_redirect_uri,
};
pub use mapping::{ADMIN_GROUP, contains_admin_group};
pub use oidc::{
    IssuerConfig, JWKS_REFRESHES_TOTAL, OidcVerifier, TOKEN_VERIFICATIONS_TOTAL, TenantBinding,
    parse_issuers,
};
pub use token::{Claims, DisabledVerifier, Hs256Verifier, ProvisioningClaims, TokenVerifier};

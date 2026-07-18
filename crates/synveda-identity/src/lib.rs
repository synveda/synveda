//! Identity: OIDC client (code + PKCE), SCIM 2.0, directory sync, and automatic
//! hierarchy provisioning from IdP claims and groups (seed §5).
//!
//! What exists today: the [`TokenVerifier`] AuthN seam (TEN-1, ADR-0008)
//! with the [`OidcVerifier`] JWKS implementation and the code+PKCE
//! [`LoginFlow`] (AUTH-1, ADR-0010), the HS256 dev verifier for CLI/demo
//! bootstrap, the tenant context task-local the gateway propagates per
//! request, and the provisioning-claims contract plus convention-mapping
//! rules JIT provisioning rides on (AUTH-2, ADR-0013 — the gateway
//! orchestrates the storage half).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod context;
mod flow;
mod mapping;
mod oidc;
mod token;

pub use context::{TenantContext, current_tenant, with_tenant};
pub use flow::{LoginFlow, LoginSession, OIDC_LOGINS_TOTAL};
pub use mapping::{CONVENTION_PREFIX, ConventionCandidate, convention_candidates};
pub use oidc::{
    IssuerConfig, JWKS_REFRESHES_TOTAL, OidcVerifier, TOKEN_VERIFICATIONS_TOTAL, TenantBinding,
    parse_issuers,
};
pub use token::{Claims, DisabledVerifier, Hs256Verifier, ProvisioningClaims, TokenVerifier};

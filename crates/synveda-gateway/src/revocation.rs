//! Request-time bearer and session-family revocation (ADR-0130).
//!
//! Runs after signature, issuer, audience and active-tenant verification and
//! before any handler uses the credential's authority. Only an issuer with a
//! revocation profile is checked; without one the issuer keeps its plain
//! authentication contract and this module adds no database read.
//!
//! Every outcome is a closed metric label. Token identifiers, digests and
//! subjects never reach a log, trace field or label.

use synveda_identity::revocation::{
    self, BearerKeys, IdentifierDigest, RevocationMode, RevocationProfile,
};
use synveda_identity::{Claims, CredentialClass, OidcTokenIdentity, TenantContext};
use synveda_types::{Error, Result, TenantId};

use crate::app::AppState;
use crate::telemetry::TOKEN_REVOCATION_CHECKS_TOTAL;

/// Verified identifiers of a bearer whose issuer has a profile.
struct Profiled<'a> {
    identity: &'a OidcTokenIdentity,
    profile: RevocationProfile,
}

fn profiled(claims: &Claims) -> Option<Profiled<'_>> {
    let identity = claims.oidc_token.as_ref()?;
    let profile = identity.revocation?;
    Some(Profiled { identity, profile })
}

fn count(mode: RevocationMode, outcome: &'static str) {
    metrics::counter!(
        TOKEN_REVOCATION_CHECKS_TOTAL,
        "mode" => mode.as_str(),
        "outcome" => outcome,
    )
    .increment(1);
}

fn revoked() -> Error {
    Error::Unauthenticated {
        message: "the credential has been revoked".to_owned(),
    }
}

fn outside_profile() -> Error {
    Error::Unauthenticated {
        message:
            "the credential does not carry the identifiers its issuer's revocation profile requires"
                .to_owned(),
    }
}

/// Admits or refuses one verified bearer for its resolved tenant.
///
/// `enforce` refuses a token outside the profile, a revoked token or family,
/// and any ledger failure; the failure stays a storage error, distinguishable
/// from a revocation. `observe` reports the same outcomes and admits.
pub(crate) async fn admit_bearer(state: &AppState, context: &TenantContext) -> Result<()> {
    let Some(profiled) = profiled(&context.claims) else {
        return Ok(());
    };
    let mode = profiled.profile.mode;
    let keys = match revocation::bearer_keys(
        context.tenant.id,
        profiled.identity,
        context.claims.credential_class,
    ) {
        Ok(keys) => keys,
        Err(refusal) => {
            count(mode, refusal.as_str());
            return match mode {
                RevocationMode::Enforce => Err(outside_profile()),
                RevocationMode::Observe => Ok(()),
            };
        }
    };
    check_ledger(state, context.tenant.id, &keys, mode).await
}

async fn check_ledger(
    state: &AppState,
    tenant: TenantId,
    keys: &BearerKeys,
    mode: RevocationMode,
) -> Result<()> {
    let mut digests = vec![*keys.token.as_bytes()];
    if let Some(family) = keys.family {
        digests.push(*family.as_bytes());
    }
    match lookup(state, tenant, &digests).await {
        Ok(false) => {
            count(mode, "admitted");
            Ok(())
        }
        Ok(true) => {
            count(mode, "revoked");
            match mode {
                RevocationMode::Enforce => Err(revoked()),
                RevocationMode::Observe => Ok(()),
            }
        }
        Err(error) => {
            count(mode, "unavailable");
            match mode {
                RevocationMode::Enforce => Err(error),
                RevocationMode::Observe => {
                    tracing::warn!(%error, "revocation ledger lookup failed in observe mode");
                    Ok(())
                }
            }
        }
    }
}

async fn lookup(state: &AppState, tenant: TenantId, digests: &[[u8; 32]]) -> Result<bool> {
    let mut tx = synveda_store::rls::begin_tenant_read_tx(&state.pool, tenant).await?;
    let revoked = synveda_store::token_revocations::any_live(&mut *tx, tenant, digests).await?;
    crate::request::commit(tx).await?;
    Ok(revoked)
}

/// The session family of a just-verified login, for the console index.
///
/// The ID token's `sid` names the issuer session that every bearer from
/// this login shares. Under `enforce`, a login without one is refused, and a
/// login into an already revoked family is refused so the caller signs out
/// at the issuer and starts a fresh session.
pub(crate) async fn login_family(
    state: &AppState,
    tenant: TenantId,
    claims: &Claims,
) -> Result<Option<IdentifierDigest>> {
    let Some(profiled) = profiled(claims) else {
        return Ok(None);
    };
    debug_assert_eq!(claims.credential_class, CredentialClass::Interactive);
    let mode = profiled.profile.mode;
    let family = match revocation::family_key(tenant, profiled.identity) {
        Ok(family) => family,
        Err(refusal) => {
            count(mode, refusal.as_str());
            return match mode {
                RevocationMode::Enforce => Err(outside_profile()),
                RevocationMode::Observe => Ok(None),
            };
        }
    };
    match (lookup(state, tenant, &[*family.as_bytes()]).await, mode) {
        (Ok(false), _) => count(mode, "admitted"),
        (Ok(true), RevocationMode::Enforce) => {
            count(mode, "revoked");
            return Err(family_revoked());
        }
        (Ok(true), RevocationMode::Observe) => count(mode, "revoked"),
        (Err(error), RevocationMode::Enforce) => {
            count(mode, "unavailable");
            return Err(error);
        }
        (Err(error), RevocationMode::Observe) => {
            count(mode, "unavailable");
            tracing::warn!(%error, "revocation ledger lookup failed in observe mode");
        }
    }
    Ok(Some(family))
}

/// Distinguishes a login into a revoked issuer session from every other
/// login refusal, so the console can tell the person to sign out at the
/// identity provider.
pub(crate) const SESSION_FAMILY_REVOKED_MESSAGE: &str = "this sign-in belongs to a revoked session; sign out at the identity provider and sign in again";

fn family_revoked() -> Error {
    Error::Unauthenticated {
        message: SESSION_FAMILY_REVOKED_MESSAGE.to_owned(),
    }
}

/// Verifies an access token minted by a refresh before it is stored or
/// returned: signature and claims, active tenant, then the bearer profile
/// and ledger. A refreshed token of a revoked family never leaves the
/// gateway under `enforce`.
pub(crate) async fn verify_refreshed(state: &AppState, access_token: &str) -> Result<()> {
    let claims = state.verifier.verify(access_token).await?;
    let context = crate::tenant::active_tenant(state, &claims).await?;
    admit_bearer(state, &context).await
}

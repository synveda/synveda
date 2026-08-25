//! The provisioning credential (AUTH-4, ADR-0059 decision 13).
//!
//! A static bearer token, which is not the credential this product would
//! choose — AUTH-3 exists because short-lived scoped tokens are better — and
//! is the credential Entra can be configured to send for a non-gallery
//! application. Confinement does the work instead: the token authenticates
//! the `/scim/v2` plane and nothing else. That plane may state directory-owned
//! identity and group facts, but cannot name a scope, role or grant; the
//! separately PDP-governed directory access-assignment command is the only
//! bridge from a group to product authority (CPR-34, ADR-0093).
//!
//! ## The shape, and why the tenant is inside it
//!
//! ```text
//! synveda_scim_v1.<tenant-uuid>.<43-char base64url secret>
//! ```
//!
//! The gateway must know which tenant to look the credential up in *before*
//! it can look it up, and `scim_credentials` is tenant-scoped under forced
//! RLS like everything else. Naming the tenant in the token makes that the
//! same shape a bearer's `tid` claim has (TEN-1, ADR-0008): **the caller
//! names the tenant, the secret proves it**. The lookup runs inside that
//! tenant's own row policy, so a credential presented against another
//! tenant is not denied so much as absent.
//!
//! What is hashed is the **whole presented string**, prefix included, so a
//! secret pasted behind a different tenant's prefix hashes to nothing. The
//! tenant in the token is therefore a claim the hash check settles, never a
//! selector the caller gets to steer.
//!
//! This amends ADR-0059 decision 13's "there is no tenant-selecting
//! parameter on the wire" — there is one, it is inside the credential, and
//! the alternative was a credential table that held tenant data with no
//! tenant policy over it (migration 0036, amendment 1).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use synveda_types::{Error, Result, TenantId};

/// Bytes of entropy in the secret half. 256 bits, the console secret's
/// budget: this is the only thing between a leaked configuration field and
/// a tenant's whole directory plane.
const SECRET_BYTES: usize = 32;

/// The token's fixed prefix. Versioned so that a future format is
/// distinguishable at a glance rather than by length, and greppable so
/// that one leaked into a log or a support ticket can be found and
/// revoked.
pub const TOKEN_PREFIX: &str = "synveda_scim_v1";

/// A freshly minted credential: the token to show once, and the hash to
/// store for it.
pub struct MintedCredential {
    /// The value an operator pastes into Entra or Okta. Never stored,
    /// never logged, shown exactly once.
    pub token: String,
    /// SHA-256 of the whole token — the database key.
    pub hash: [u8; 32],
}

/// Mints a credential for `tenant_id`.
///
/// # Errors
///
/// [`Error::Internal`] when the OS random source fails.
pub fn mint(tenant_id: TenantId) -> Result<MintedCredential> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|err| Error::Internal {
        message: format!("failed to generate a provisioning credential: {err}"),
    })?;
    let token = format!(
        "{TOKEN_PREFIX}.{tenant_id}.{}",
        URL_SAFE_NO_PAD.encode(bytes)
    );
    let hash = hash(&token);
    Ok(MintedCredential { token, hash })
}

/// Hashes a presented token to its storage key — the whole string, prefix
/// and tenant included.
#[must_use]
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// The tenant a presented token names, or `None` when the string is not
/// one of this product's provisioning credentials at all.
///
/// Reading this is not authenticating: the tenant here is the caller's
/// claim, settled by the hash lookup that follows. What it buys is the
/// ability to run that lookup inside the right tenant's row policy.
#[must_use]
pub fn tenant_of(token: &str) -> Option<TenantId> {
    let mut parts = token.split('.');
    let prefix = parts.next()?;
    if prefix != TOKEN_PREFIX {
        return None;
    }
    let tenant = parts.next()?;
    let secret = parts.next()?;
    // A third separator means a shape this version does not define; refuse
    // it rather than ignore the tail.
    if parts.next().is_some() || secret.is_empty() {
        return None;
    }
    tenant.parse::<TenantId>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_names_its_tenant_and_hashes_to_its_key() {
        let tenant = TenantId::new();
        let minted = mint(tenant).expect("mint");
        assert_eq!(tenant_of(&minted.token), Some(tenant));
        assert_eq!(minted.hash, hash(&minted.token));
        assert!(minted.token.starts_with(TOKEN_PREFIX));
    }

    #[test]
    fn the_hash_covers_the_prefix_so_a_secret_cannot_be_re_pointed() {
        // The property the whole-string hash exists for: lifting a valid
        // secret behind another tenant's prefix must not authenticate
        // anything. Without it the secret alone would be the credential
        // and the tenant would be the caller's to choose.
        let tenant = TenantId::new();
        let other = TenantId::new();
        let minted = mint(tenant).expect("mint");
        let secret = minted.token.rsplit('.').next().expect("secret");
        let forged = format!("{TOKEN_PREFIX}.{other}.{secret}");
        assert_eq!(tenant_of(&forged), Some(other));
        assert_ne!(hash(&forged), minted.hash);
    }

    #[test]
    fn two_credentials_are_never_the_same() {
        let tenant = TenantId::new();
        let a = mint(tenant).expect("mint");
        let b = mint(tenant).expect("mint");
        assert_ne!(a.token, b.token);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn anything_that_is_not_this_format_names_no_tenant() {
        let tenant = TenantId::new().to_string();
        for text in [
            "",
            "synveda_scim_v1",
            &format!("synveda_scim_v1.{tenant}"),
            &format!("synveda_scim_v1.{tenant}."),
            &format!("synveda_scim_v2.{tenant}.secret"),
            "synveda_scim_v1.not-a-uuid.secret",
            &format!("synveda_scim_v1.{tenant}.secret.extra"),
            "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.body.signature",
        ] {
            assert_eq!(tenant_of(text), None, "{text:?} must name no tenant");
        }
    }
}

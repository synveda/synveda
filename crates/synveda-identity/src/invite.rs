//! The invitation token (CPR-5, ADR-0072 decision 5).
//!
//! An invitation is a **bearer credential that mints access**: whoever presents
//! it, inside the tenant it names, gets the role it carries. So it is minted,
//! hashed and treated exactly as the provisioning credential is
//! ([`crate::scim`], ADR-0059 decision 13) — the same 256 bits of entropy, the
//! same whole-string hash, the same "shown once and never stored".
//!
//! ## The shape, and why the tenant is inside it
//!
//! ```text
//! synveda_invite_v1.<tenant-uuid>.<43-char base64url secret>
//! ```
//!
//! What is hashed is the **whole presented string**, prefix and tenant
//! included, so a secret lifted behind another tenant's prefix hashes to
//! nothing. That is not the isolation mechanism — forced RLS and the
//! `(tenant_id, token_hash)` lookup are — it is the second one, and it costs a
//! format decision.
//!
//! The prefix is versioned so a future format is distinguishable at a glance
//! rather than by length, and **greppable**, which is the property that matters
//! most for this particular secret: an invitation is pasted into chat windows,
//! ticket comments and support threads, and one that leaks has to be findable.
//!
//! ## It is not a login
//!
//! Redeeming an invitation takes the recipient's *own* credential — the token
//! says which access to add, never who is asking. An invitation that
//! authenticated would be a password somebody emailed.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use synveda_types::{Error, Result, TenantId};

/// Bytes of entropy in the secret half. 256 bits, the console secret's and the
/// provisioning credential's budget, because this is the same kind of thing:
/// the only barrier between a leaked string and somebody else's workspace.
const SECRET_BYTES: usize = 32;

/// The token's fixed prefix.
pub const TOKEN_PREFIX: &str = "synveda_invite_v1";

/// A freshly minted invitation: the token to show once, and the hash to store.
pub struct MintedInvite {
    /// The value the inviter copies. Never stored, never logged, never in an
    /// audit payload, shown exactly once.
    pub token: String,
    /// SHA-256 of the whole token — the database key.
    pub hash: [u8; 32],
}

/// Mints an invitation token for `tenant_id`.
///
/// # Errors
///
/// [`Error::Internal`] when the OS random source fails.
pub fn mint(tenant_id: TenantId) -> Result<MintedInvite> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|err| Error::Internal {
        message: format!("failed to generate an invitation token: {err}"),
    })?;
    let token = format!(
        "{TOKEN_PREFIX}.{tenant_id}.{}",
        URL_SAFE_NO_PAD.encode(bytes)
    );
    let hash = hash(&token);
    Ok(MintedInvite { token, hash })
}

/// Hashes a presented token to its storage key — the whole string, prefix and
/// tenant included.
#[must_use]
pub fn hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// The tenant a presented token names, or `None` when the string is not one of
/// this product's invitations at all.
///
/// Reading this is not authenticating, and on the redeem path it is not even
/// how the tenant is chosen: the caller's own bearer decides that. It is here
/// so a malformed string is refused as *malformed* rather than looked up, and
/// so a token pasted at the wrong deployment fails with a sentence rather than
/// a bare 404.
#[must_use]
pub fn tenant_of(token: &str) -> Option<TenantId> {
    let mut parts = token.split('.');
    if parts.next()? != TOKEN_PREFIX {
        return None;
    }
    let tenant = parts.next()?;
    let secret = parts.next()?;
    // A third separator means a shape this version does not define; refuse it
    // rather than ignore the tail.
    if parts.next().is_some() || secret.is_empty() {
        return None;
    }
    tenant.parse::<TenantId>().ok()
}

/// Checks that a presented string is one of this product's invitation tokens
/// before anything looks it up.
///
/// # Errors
///
/// [`Error::Invalid`] naming the prefix, so somebody who pasted the wrong half
/// of a URL learns what an invitation looks like — without the message ever
/// echoing what they pasted, because what they pasted may well be a secret.
pub fn parse(token: &str) -> Result<TenantId> {
    tenant_of(token).ok_or_else(|| Error::Invalid {
        message: format!(
            "that is not an invitation token: one starts with `{TOKEN_PREFIX}.` and \
             carries the tenant it belongs to"
        ),
    })
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

    /// The property the whole-string hash exists for: lifting a valid secret
    /// behind another tenant's prefix must not redeem anything.
    #[test]
    fn the_hash_covers_the_prefix_so_a_secret_cannot_be_re_pointed() {
        let tenant = TenantId::new();
        let other = TenantId::new();
        let minted = mint(tenant).expect("mint");
        let secret = minted.token.rsplit('.').next().expect("secret");
        let forged = format!("{TOKEN_PREFIX}.{other}.{secret}");
        assert_eq!(tenant_of(&forged), Some(other));
        assert_ne!(hash(&forged), minted.hash);
    }

    #[test]
    fn two_invitations_are_never_the_same() {
        let tenant = TenantId::new();
        assert_ne!(
            mint(tenant).expect("mint").token,
            mint(tenant).expect("mint").token
        );
    }

    #[test]
    fn a_string_that_is_not_an_invitation_is_refused_by_shape() {
        for bad in [
            "",
            "hunter2",
            "synveda_scim_v1.00000000-0000-0000-0000-000000000000.abc",
            "synveda_invite_v1.not-a-uuid.abc",
            "synveda_invite_v1.00000000-0000-0000-0000-000000000000.",
            "synveda_invite_v1.00000000-0000-0000-0000-000000000000.abc.extra",
        ] {
            assert!(tenant_of(bad).is_none(), "{bad:?} parsed as an invitation");
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    /// A refusal must never quote the thing it refused: the caller may have
    /// pasted a live token from another deployment, and an error message is a
    /// log line.
    #[test]
    fn the_refusal_never_echoes_what_was_presented() {
        let tenant = TenantId::new();
        let minted = mint(tenant).expect("mint");
        let mangled = format!("{}x.{}", TOKEN_PREFIX, minted.token);
        let error = parse(&mangled).expect_err("refused");
        let message = error.to_string();
        assert!(message.contains(TOKEN_PREFIX));
        assert!(
            !message.contains(&minted.token),
            "the refusal quoted the presented secret: {message}"
        );
    }
}

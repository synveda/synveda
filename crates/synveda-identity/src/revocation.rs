//! Issuer-specific bearer and session-family revocation profiles (ADR-0130).
//!
//! A profile is selected per issuer. Without one, an issuer keeps its plain
//! authentication contract and no revocation promise. With one, verified
//! token identifiers become domain-separated digests that the gateway checks
//! against its ledger; in `enforce` mode a token outside the profile's shape
//! is refused rather than accepted as unrevocable.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use synveda_types::TenantId;

use crate::token::{CredentialClass, OidcTokenIdentity};

/// The longest access-token lifetime the bundled Keycloak profile admits.
pub const MAX_ACCESS_TOKEN_LIFETIME: Duration = Duration::from_secs(300);

/// How long a revoked session family stays in the ledger. Longer than the
/// bundled realm's eight-hour maximum SSO session.
pub const FAMILY_RETENTION: Duration = Duration::from_secs(12 * 60 * 60);

/// Added to a revoked token's own expiry so verifier leeway cannot outlive
/// its ledger row.
pub const TOKEN_RETENTION_LEEWAY: Duration = Duration::from_secs(60);

const DIGEST_DOMAIN: &[u8] = b"synveda.token-revocation.v1";

/// The selected issuer contract. Only the bundled Keycloak realm has one;
/// another issuer needs its own claim mapping and live evidence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationProfileKind {
    /// Signed `jti` per access token, stable `sid` per interactive session,
    /// numeric `iat`/`exp` at most five minutes apart.
    BundledKeycloak,
}

/// Whether a profile refuses or only reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationMode {
    /// Look up and report, but admit tokens outside the profile or in the
    /// ledger.
    Observe,
    /// Refuse a malformed, oversized-lifetime or revoked token, and refuse
    /// when the ledger cannot be read.
    Enforce,
}

impl RevocationMode {
    /// Stable metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }
}

/// One issuer's revocation selection, from `SYNVEDA_OIDC_ISSUERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevocationProfile {
    /// Which issuer contract the identifiers follow.
    pub profile: RevocationProfileKind,
    /// Report-only or refusing.
    pub mode: RevocationMode,
}

/// Which identifier a ledger row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierKind {
    /// One bearer, by `jti`.
    Token,
    /// Every bearer of one interactive session, by `sid`.
    Family,
}

impl IdentifierKind {
    /// Stable ledger and audit value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Family => "family",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Token => 1,
            Self::Family => 2,
        }
    }
}

/// SHA-256 over tenant, exact issuer, kind and opaque value. Not secret, but
/// never rendered: it is still a stable handle on one credential.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdentifierDigest([u8; 32]);

impl IdentifierDigest {
    /// The raw digest bytes, for the ledger.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Rebuilds a digest read back from the ledger or index.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for IdentifierDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdentifierDigest(<redacted>)")
    }
}

/// Domain-separated digest of one identifier. Every variable-length field is
/// length-prefixed, so no two distinct inputs share an encoding.
#[must_use]
pub fn identifier_digest(
    tenant: TenantId,
    issuer: &str,
    kind: IdentifierKind,
    value: &str,
) -> IdentifierDigest {
    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update(tenant.as_uuid().as_bytes());
    hasher.update(
        u64::try_from(issuer.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(issuer.as_bytes());
    hasher.update([kind.tag()]);
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value.as_bytes());
    IdentifierDigest(hasher.finalize().into())
}

/// Why a verified token falls outside its issuer's profile. Closed, so it can
/// label a metric without carrying token content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileRefusal {
    /// No bounded, nonempty `jti`.
    MissingTokenId,
    /// An interactive token without a bounded, nonempty `sid`.
    MissingSessionId,
    /// No numeric `iat`.
    MissingIssuedAt,
    /// `exp - iat` is zero or longer than the profile admits.
    Lifetime,
}

impl ProfileRefusal {
    /// Stable metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingTokenId => "missing_token_id",
            Self::MissingSessionId => "missing_session_id",
            Self::MissingIssuedAt => "missing_issued_at",
            Self::Lifetime => "lifetime",
        }
    }
}

/// The ledger keys that can revoke one verified bearer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BearerKeys {
    /// This bearer's own identifier.
    pub token: IdentifierDigest,
    /// Its interactive session family. Service bearers never carry one: a
    /// service identity is not a human session and is withdrawn by removal.
    pub family: Option<IdentifierDigest>,
    /// The bearer's `exp`, which bounds how long its token row must live.
    pub expires_at: u64,
}

/// Applies the bundled Keycloak profile to a verified access token.
pub fn bearer_keys(
    tenant: TenantId,
    identity: &OidcTokenIdentity,
    class: CredentialClass,
) -> Result<BearerKeys, ProfileRefusal> {
    let token_id = identity
        .token_id
        .as_deref()
        .ok_or(ProfileRefusal::MissingTokenId)?;
    let issued_at = identity.issued_at.ok_or(ProfileRefusal::MissingIssuedAt)?;
    let lifetime = identity.expires_at.saturating_sub(issued_at);
    if lifetime == 0 || lifetime > MAX_ACCESS_TOKEN_LIFETIME.as_secs() {
        return Err(ProfileRefusal::Lifetime);
    }
    let family = match class {
        CredentialClass::ServiceBearer => None,
        CredentialClass::PrimaryBearer | CredentialClass::Interactive => {
            Some(family_key(tenant, identity)?)
        }
    };
    Ok(BearerKeys {
        token: identifier_digest(tenant, &identity.issuer, IdentifierKind::Token, token_id),
        family,
        expires_at: identity.expires_at,
    })
}

/// The session-family key of a verified interactive token: the ID token at
/// login, or a primary bearer.
pub fn family_key(
    tenant: TenantId,
    identity: &OidcTokenIdentity,
) -> Result<IdentifierDigest, ProfileRefusal> {
    let session_id = identity
        .session_id
        .as_deref()
        .ok_or(ProfileRefusal::MissingSessionId)?;
    Ok(identifier_digest(
        tenant,
        &identity.issuer,
        IdentifierKind::Family,
        session_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> OidcTokenIdentity {
        OidcTokenIdentity {
            issuer: "https://auth.example.test/realms/synveda".to_owned(),
            token_id: Some("token-1".to_owned()),
            session_id: Some("session-1".to_owned()),
            issued_at: Some(1_000),
            expires_at: 1_300,
            revocation: None,
        }
    }

    #[test]
    fn digests_bind_tenant_issuer_kind_and_value() {
        let tenant = TenantId::new();
        let issuer = "https://auth.example.test/realms/synveda";
        let base = identifier_digest(tenant, issuer, IdentifierKind::Token, "a");
        assert_eq!(
            base,
            identifier_digest(tenant, issuer, IdentifierKind::Token, "a")
        );
        for other in [
            identifier_digest(TenantId::new(), issuer, IdentifierKind::Token, "a"),
            identifier_digest(tenant, "https://other.test", IdentifierKind::Token, "a"),
            identifier_digest(tenant, issuer, IdentifierKind::Family, "a"),
            identifier_digest(tenant, issuer, IdentifierKind::Token, "A"),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn length_prefixes_keep_issuer_and_value_apart() {
        let tenant = TenantId::new();
        assert_ne!(
            identifier_digest(tenant, "ab", IdentifierKind::Token, "c"),
            identifier_digest(tenant, "a", IdentifierKind::Token, "bc"),
        );
    }

    #[test]
    fn primary_bearers_carry_token_and_family_keys() {
        let tenant = TenantId::new();
        let keys = bearer_keys(tenant, &identity(), CredentialClass::PrimaryBearer)
            .expect("profile shape");
        assert_eq!(keys.family, Some(family_key(tenant, &identity()).unwrap()));
        assert_eq!(keys.expires_at, 1_300);
    }

    #[test]
    fn a_refreshed_token_keeps_its_family_and_changes_its_token_key() {
        let tenant = TenantId::new();
        let first = bearer_keys(tenant, &identity(), CredentialClass::PrimaryBearer).unwrap();
        let refreshed = OidcTokenIdentity {
            token_id: Some("token-2".to_owned()),
            ..identity()
        };
        let second = bearer_keys(tenant, &refreshed, CredentialClass::PrimaryBearer).unwrap();
        assert_eq!(first.family, second.family);
        assert_ne!(first.token, second.token);
    }

    #[test]
    fn service_bearers_never_join_a_human_family() {
        let keys = bearer_keys(TenantId::new(), &identity(), CredentialClass::ServiceBearer)
            .expect("service shape");
        assert_eq!(keys.family, None);
        let without_sid = OidcTokenIdentity {
            session_id: None,
            ..identity()
        };
        assert!(
            bearer_keys(
                TenantId::new(),
                &without_sid,
                CredentialClass::ServiceBearer
            )
            .is_ok()
        );
    }

    #[test]
    fn tokens_outside_the_profile_are_refused_with_a_closed_reason() {
        let tenant = TenantId::new();
        let cases = [
            (
                OidcTokenIdentity {
                    token_id: None,
                    ..identity()
                },
                ProfileRefusal::MissingTokenId,
            ),
            (
                OidcTokenIdentity {
                    session_id: None,
                    ..identity()
                },
                ProfileRefusal::MissingSessionId,
            ),
            (
                OidcTokenIdentity {
                    issued_at: None,
                    ..identity()
                },
                ProfileRefusal::MissingIssuedAt,
            ),
            (
                OidcTokenIdentity {
                    expires_at: 1_301,
                    ..identity()
                },
                ProfileRefusal::Lifetime,
            ),
            (
                OidcTokenIdentity {
                    expires_at: 1_000,
                    ..identity()
                },
                ProfileRefusal::Lifetime,
            ),
        ];
        for (shape, refusal) in cases {
            assert_eq!(
                bearer_keys(tenant, &shape, CredentialClass::PrimaryBearer),
                Err(refusal)
            );
        }
    }

    #[test]
    fn digests_never_render() {
        let digest = identifier_digest(TenantId::new(), "i", IdentifierKind::Token, "v");
        assert_eq!(format!("{digest:?}"), "IdentifierDigest(<redacted>)");
    }

    #[test]
    fn profiles_parse_closed_values_only() {
        let parsed: RevocationProfile =
            serde_json::from_str(r#"{"profile":"bundled_keycloak","mode":"enforce"}"#)
                .expect("closed profile");
        assert_eq!(parsed.mode, RevocationMode::Enforce);
        for refused in [
            r#"{"profile":"entra","mode":"enforce"}"#,
            r#"{"profile":"bundled_keycloak","mode":"strict"}"#,
            r#"{"profile":"bundled_keycloak"}"#,
            r#"{"profile":"bundled_keycloak","mode":"observe","cache":true}"#,
        ] {
            assert!(serde_json::from_str::<RevocationProfile>(refused).is_err());
        }
    }
}

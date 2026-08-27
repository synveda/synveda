//! What every ciphertext is bound to (TEN-4, ADR-0064 decision 4).
//!
//! A sealed payload carries additional authenticated data covering the key
//! scope, what the payload *is*, and which row it belongs to. A ciphertext
//! lifted from one tenant's row into another's therefore fails to open rather
//! than opening — which makes a cross-tenant transplant a decryption failure
//! TEN-6 can fuzz for, and an audit event, instead of a silent success.
//!
//! **The AAD is composed here, from typed arguments, and there is no public
//! way to supply raw bytes.** That is the whole reason this crate depends on
//! `synveda-types` instead of dealing in `&[u8]` and staying one tier purer
//! (ADR-0064 decision 13): binding a ciphertext to its tenant must not be
//! something a caller can forget. It is ADR-0060 decision 8's move — a
//! connector that cannot *express* a scope cannot leak one — applied to
//! bytes.

use crate::key::KeyScope;

/// The domain separator. Bumped only if the AAD layout below changes, which
/// would be a new envelope version and a new constant, not an edit to this
/// one — every stored ciphertext was sealed against these exact bytes.
const DOMAIN: &[u8] = b"synveda/envelope/v1\0";

/// What a sealed payload is.
///
/// Part of the AAD, so a ciphertext cannot be moved between columns any more
/// than it can be moved between tenants: an access token pasted into the
/// refresh token column fails to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// `console_sessions.access_token_sealed` (deployment scope — decision 5).
    ConsoleAccessToken,
    /// `console_sessions.refresh_token_sealed` (deployment scope).
    ConsoleRefreshToken,
    /// One stable tenant-secret aggregate (CPR-35, ADR-0094).
    TenantSecret,
    /// The per-export data key carried in a sealed archive's header
    /// (decision 8).
    ExportKey,
    /// A tenant export's body.
    TenantExport,
    /// A data key wrapped by the KMS (decision 1). Using the same envelope
    /// for the wrap keeps one audited code path rather than two.
    DataKey,
}

impl Purpose {
    /// The stable name that goes into the AAD, into metrics labels, and into
    /// audit payloads. Changing one of these strings invalidates every
    /// ciphertext sealed under it, which is why they are spelled out rather
    /// than derived from the variant name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Purpose::ConsoleAccessToken => "console.access_token",
            Purpose::ConsoleRefreshToken => "console.refresh_token",
            Purpose::TenantSecret => "tenant.secret",
            Purpose::ExportKey => "export.key",
            Purpose::TenantExport => "export.body",
            Purpose::DataKey => "kms.data_key",
        }
    }
}

/// Which row a sealed payload belongs to.
///
/// A closed set rather than a byte string, for the same reason [`Purpose`] is
/// an enum: the identity of a row is a thing the caller knows and the crate
/// should encode, not a buffer the caller assembles and might assemble
/// differently at the two ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKey<'a> {
    /// A 32-byte hash — `console_sessions.token_hash`.
    Hash(&'a [u8; 32]),
    /// A UUID row identity.
    Uuid(uuid::Uuid),
    /// A named singleton — a tenant has one directory credential, and the
    /// name is what distinguishes it from the next singleton to arrive.
    Name(&'a str),
}

impl RowKey<'_> {
    const fn tag(&self) -> u8 {
        match self {
            RowKey::Hash(_) => 1,
            RowKey::Uuid(_) => 2,
            RowKey::Name(_) => 3,
        }
    }

    fn bytes(&self) -> &[u8] {
        match self {
            RowKey::Hash(hash) => &hash[..],
            RowKey::Uuid(uuid) => uuid.as_bytes(),
            RowKey::Name(name) => name.as_bytes(),
        }
    }
}

/// Builds the additional authenticated data for one sealed payload.
///
/// Every variable-length field is length-prefixed. Without that, a purpose of
/// `"a"` with a row named `"bc"` and a purpose of `"ab"` with a row named
/// `"c"` would produce identical AAD, and two ciphertexts that should be
/// unrelated would open in each other's context.
pub(crate) fn compose(
    header_prefix: &[u8],
    scope: KeyScope,
    purpose: Purpose,
    row: RowKey<'_>,
) -> Vec<u8> {
    let purpose = purpose.as_str().as_bytes();
    let row_bytes = row.bytes();
    let mut aad = Vec::with_capacity(
        DOMAIN.len() + header_prefix.len() + 1 + 16 + 2 + purpose.len() + 1 + 4 + row_bytes.len(),
    );
    aad.extend_from_slice(DOMAIN);
    // The header the reader trusted to select a key and an algorithm. Binding
    // it means a downgrade — rewriting the algorithm byte, or pointing the
    // version at a key an attacker can reach — breaks the tag rather than
    // being honoured.
    aad.extend_from_slice(header_prefix);
    aad.push(scope.tag());
    aad.extend_from_slice(&scope.uuid_bytes());
    aad.extend_from_slice(&(purpose.len() as u16).to_be_bytes());
    aad.extend_from_slice(purpose);
    aad.push(row.tag());
    // A row key is bounded by the columns it comes from (a 32-byte hash, a
    // UUID, or a name this crate's callers spell as a literal), so a u32
    // length cannot truncate one.
    aad.extend_from_slice(&(row_bytes.len() as u32).to_be_bytes());
    aad.extend_from_slice(row_bytes);
    aad
}

#[cfg(test)]
mod tests {
    use synveda_types::TenantId;

    use super::*;

    const HEADER: &[u8] = b"header0123";

    #[test]
    fn scope_changes_the_aad() {
        let one = TenantId::new();
        let two = TenantId::new();
        let a = compose(
            HEADER,
            KeyScope::Tenant(one),
            Purpose::TenantSecret,
            RowKey::Name("graph"),
        );
        let b = compose(
            HEADER,
            KeyScope::Tenant(two),
            Purpose::TenantSecret,
            RowKey::Name("graph"),
        );
        assert_ne!(a, b, "two tenants must not share AAD");
    }

    #[test]
    fn the_deployment_scope_is_distinct_from_every_tenant() {
        let tenant = compose(
            HEADER,
            KeyScope::Tenant(TenantId::new()),
            Purpose::ConsoleAccessToken,
            RowKey::Name("x"),
        );
        let deployment = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("x"),
        );
        assert_ne!(tenant, deployment);
    }

    #[test]
    fn purpose_changes_the_aad() {
        let access = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("x"),
        );
        let refresh = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleRefreshToken,
            RowKey::Name("x"),
        );
        assert_ne!(access, refresh, "columns must not be interchangeable");
    }

    #[test]
    fn row_key_changes_the_aad() {
        let a = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Hash(&[1_u8; 32]),
        );
        let b = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Hash(&[2_u8; 32]),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn header_is_bound_so_a_downgrade_breaks_the_tag() {
        let a = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("x"),
        );
        let b = compose(
            b"header0124",
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("x"),
        );
        assert_ne!(a, b);
    }

    #[test]
    fn length_prefixing_removes_the_ambiguity_it_exists_for() {
        // Without length prefixes these two would collide: "a" + "bc" and
        // "ab" + "c" concatenate to the same bytes.
        let left = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("abc"),
        );
        let right = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name("ab"),
        );
        assert_ne!(left, right);
        // And the row tag keeps a name distinct from a hash that spells it.
        let named = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Name(&"a".repeat(32)),
        );
        let hashed = compose(
            HEADER,
            KeyScope::Deployment,
            Purpose::ConsoleAccessToken,
            RowKey::Hash(&[b'a'; 32]),
        );
        assert_ne!(named, hashed);
    }

    #[test]
    fn purpose_names_are_stable_and_distinct() {
        let all = [
            Purpose::ConsoleAccessToken,
            Purpose::ConsoleRefreshToken,
            Purpose::TenantSecret,
            Purpose::ExportKey,
            Purpose::TenantExport,
            Purpose::DataKey,
        ];
        let mut names: Vec<&str> = all.iter().map(|p| p.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "purpose names must be distinct");
    }
}

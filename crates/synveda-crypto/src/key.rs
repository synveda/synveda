//! Key material, key scopes, and key versions (TEN-4, ADR-0064).

use std::fmt;

use synveda_types::{Error, Result, TenantId};
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The length of every key in this crate. XChaCha20-Poly1305 takes a 256-bit
/// key and so does the local KEK that wraps one.
pub const KEY_LEN: usize = 32;

/// 256 bits of key material, wiped on drop.
///
/// There is no `Debug` that prints bytes, no `Display`, no `Serialize` and no
/// `Clone` — a key that can be copied casually is a key that outlives the
/// scope somebody reasoned about. Getting the bytes out is deliberately
/// awkward and crate-private ([`DataKey::expose`]).
#[derive(ZeroizeOnDrop)]
pub struct DataKey([u8; KEY_LEN]);

// The key never reaches a log line, a span field, or an error message —
// the discipline `Ed25519Signer` already models (ADR-0030 decision 9).
impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey(<redacted>)")
    }
}

impl DataKey {
    /// Mints a key from the operating system's CSPRNG.
    ///
    /// This is the one place in the product that invents key material. It
    /// takes `getrandom` directly rather than a seeded RNG for the same
    /// reason `ed25519-dalek` is taken without `rand_core` (ADR-0030
    /// decision 9): nothing here can be made deterministic by a caller, in a
    /// test or otherwise.
    pub fn generate() -> Result<Self> {
        let mut bytes = [0_u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|err| Error::Internal {
            message: format!("system CSPRNG unavailable: {err}"),
        })?;
        Ok(DataKey(bytes))
    }

    /// Adopts existing key material — an unwrapped DEK, or a KEK read from
    /// configuration.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        DataKey(bytes)
    }

    /// Parses the 64-character hex form used by configuration.
    ///
    /// The buffer is wiped before returning on every path, including the
    /// error ones — a malformed key is still key-shaped.
    pub fn from_hex(hex: &str) -> Result<Self> {
        let hex = hex.trim();
        if hex.len() != KEY_LEN * 2 {
            return Err(Error::Invalid {
                message: format!(
                    "key must be {} hex characters ({KEY_LEN} bytes), got {}",
                    KEY_LEN * 2,
                    hex.len()
                ),
            });
        }
        let mut bytes = [0_u8; KEY_LEN];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_nibble(pair[0]);
            let low = decode_nibble(pair[1]);
            match (high, low) {
                (Some(high), Some(low)) => bytes[index] = (high << 4) | low,
                _ => {
                    bytes.zeroize();
                    return Err(Error::Invalid {
                        message: "key must be hex characters only".to_string(),
                    });
                }
            }
        }
        Ok(DataKey(bytes))
    }

    /// The raw bytes, for the cipher and for wrapping. Crate-private: no
    /// caller outside this crate has a reason to hold key bytes.
    pub(crate) const fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

const fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Which key seals a payload.
///
/// Two scopes, and the second one is the finding this feature turned up
/// (ADR-0064 decision 5). `console_sessions` is read *before* a tenant
/// exists — reading it is one of the steps that establishes the tenant — so
/// selecting a per-tenant key for it would require deriving a tenant from
/// the session row, which is exactly the derivation ADR-0056's schema exists
/// to make impossible. It is a scope in the key plane rather than an
/// exemption in the guard, so that nobody "fixes" the asymmetry by giving
/// that table a `tenant_id` back and trading a real isolation invariant for
/// a cosmetic one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyScope {
    /// One tenant's data key. Everything tenant-scoped.
    Tenant(TenantId),
    /// The deployment's data key, for rows that structurally cannot name a
    /// tenant.
    Deployment,
}

/// The tag byte identifying a tenant-scoped key in an envelope header.
pub(crate) const SCOPE_TAG_TENANT: u8 = 1;
/// The tag byte identifying the deployment key in an envelope header.
pub(crate) const SCOPE_TAG_DEPLOYMENT: u8 = 2;

impl KeyScope {
    /// The stable label recorded in metrics and audit payloads. Never
    /// carries the tenant id — a metric label with a tenant in it is a
    /// cardinality problem and a disclosure at the same time.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            KeyScope::Tenant(_) => "tenant",
            KeyScope::Deployment => "deployment",
        }
    }

    pub(crate) const fn tag(&self) -> u8 {
        match self {
            KeyScope::Tenant(_) => SCOPE_TAG_TENANT,
            KeyScope::Deployment => SCOPE_TAG_DEPLOYMENT,
        }
    }

    /// The 16 bytes naming this scope. The deployment scope is all zeros,
    /// which is not a UUID any tenant can have (`tenants_pk` is a UUIDv7 and
    /// the nil UUID is never a valid identifier — `define_id!` refuses a
    /// `Default` for exactly that reason).
    pub(crate) fn uuid_bytes(&self) -> [u8; 16] {
        match self {
            KeyScope::Tenant(id) => *id.as_uuid().as_bytes(),
            KeyScope::Deployment => *Uuid::nil().as_bytes(),
        }
    }

    /// The tenant this scope names, if any. The store uses it to pick the
    /// row; nothing else should need it.
    #[must_use]
    pub const fn tenant(&self) -> Option<TenantId> {
        match self {
            KeyScope::Tenant(id) => Some(*id),
            KeyScope::Deployment => None,
        }
    }
}

impl fmt::Display for KeyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyScope::Tenant(id) => write!(f, "tenant:{id}"),
            KeyScope::Deployment => f.write_str("deployment"),
        }
    }
}

/// Which generation of a scope's data key sealed a payload.
///
/// Carried in every envelope header so rotation is lazy rather than
/// stop-the-world (ADR-0064 decision 6): a reader peeks the version, selects
/// that key, and opens. Re-sealing under a newer version can then happen on
/// write, in a background pass, or never — `console_sessions` rotates by
/// expiry, because every row already ages out under its own absolute cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyVersion(u32);

impl KeyVersion {
    /// The version a scope's first key gets.
    pub const FIRST: KeyVersion = KeyVersion(1);

    /// Wraps a version read back from storage.
    ///
    /// Zero is refused: a stored version of zero is far more likely to be an
    /// unset column than a real generation, and a key plane that silently
    /// accepts it would select the wrong key on the day somebody adds a
    /// column with a default.
    pub fn new(version: u32) -> Result<Self> {
        if version == 0 {
            return Err(Error::Invalid {
                message: "key version must be 1 or greater".to_string(),
            });
        }
        Ok(KeyVersion(version))
    }

    /// The next generation.
    #[must_use]
    pub const fn next(&self) -> KeyVersion {
        KeyVersion(self.0.saturating_add(1))
    }

    /// The wire and storage form.
    #[must_use]
    pub const fn get(&self) -> u32 {
        self.0
    }

    /// The storage form. Postgres has no unsigned integers and the column is
    /// `integer`, so this is where the range is checked rather than at every
    /// call site.
    pub fn from_i32(version: i32) -> Result<Self> {
        u32::try_from(version)
            .map_err(|_| Error::Invalid {
                message: format!("key version must be positive, got {version}"),
            })
            .and_then(KeyVersion::new)
    }

    /// The storage form. Infallible: a `u32` that came from
    /// [`KeyVersion::new`] is bounded by the same `integer` column it was
    /// read from, and `next()` saturates.
    #[must_use]
    pub const fn as_i32(&self) -> i32 {
        // A version that reached i32::MAX is a rotation loop, not a
        // deployment; clamping keeps this total without inventing a
        // fallible conversion nobody can handle.
        if self.0 > i32::MAX as u32 {
            i32::MAX
        } else {
            self.0 as i32
        }
    }
}

impl fmt::Display for KeyVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let key = DataKey::generate().expect("generate");
        let hex: String = key.expose().iter().map(|b| format!("{b:02x}")).collect();
        let parsed = DataKey::from_hex(&hex).expect("parse");
        assert_eq!(parsed.expose(), key.expose());
    }

    #[test]
    fn hex_rejects_wrong_length_and_non_hex() {
        assert!(DataKey::from_hex("abcd").is_err());
        assert!(DataKey::from_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn hex_accepts_upper_and_lower_case() {
        let lower = DataKey::from_hex(&"ab".repeat(32)).expect("lower");
        let upper = DataKey::from_hex(&"AB".repeat(32)).expect("upper");
        assert_eq!(lower.expose(), upper.expose());
    }

    #[test]
    fn debug_does_not_print_key_material() {
        let key = DataKey::from_bytes([7_u8; KEY_LEN]);
        let rendered = format!("{key:?}");
        assert_eq!(rendered, "DataKey(<redacted>)");
        assert!(!rendered.contains('7'));
    }

    #[test]
    fn generated_keys_differ() {
        let first = DataKey::generate().expect("first");
        let second = DataKey::generate().expect("second");
        assert_ne!(first.expose(), second.expose());
    }

    #[test]
    fn deployment_scope_is_the_nil_uuid_which_no_tenant_can_hold() {
        assert_eq!(KeyScope::Deployment.uuid_bytes(), [0_u8; 16]);
        let tenant = KeyScope::Tenant(TenantId::new());
        assert_ne!(tenant.uuid_bytes(), [0_u8; 16]);
    }

    #[test]
    fn scope_labels_never_carry_the_tenant() {
        let scope = KeyScope::Tenant(TenantId::new());
        assert_eq!(scope.label(), "tenant");
        assert_eq!(KeyScope::Deployment.label(), "deployment");
    }

    #[test]
    fn key_version_refuses_zero() {
        assert!(KeyVersion::new(0).is_err());
        assert!(KeyVersion::from_i32(0).is_err());
        assert!(KeyVersion::from_i32(-1).is_err());
        assert_eq!(KeyVersion::new(1).expect("one"), KeyVersion::FIRST);
    }

    #[test]
    fn key_version_round_trips_through_storage() {
        let version = KeyVersion::new(9).expect("nine");
        assert_eq!(
            KeyVersion::from_i32(version.as_i32()).expect("round trip"),
            version
        );
        assert_eq!(version.next().get(), 10);
    }
}

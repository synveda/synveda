//! The sealed-payload format (TEN-4, ADR-0064 decisions 3 and 6).
//!
//! ```text
//!   0..4    magic       b"SVE1"
//!   4       algorithm   1 = XChaCha20-Poly1305
//!   5       scope tag   1 = tenant, 2 = deployment
//!   6..10   key version u32, big-endian
//!  10..34   nonce       24 bytes
//!  34..     ciphertext || 16-byte Poly1305 tag
//! ```
//!
//! Bytes `0..10` — everything the header *asserts* — are bound into the AAD,
//! so rewriting the algorithm byte or pointing the version at a different key
//! breaks the tag rather than being honoured. The nonce is not in the AAD
//! because the cipher already binds it.
//!
//! **The version is in the header so rotation can be lazy** (decision 6). A
//! reader peeks it with [`envelope_version`], selects that generation's key,
//! and opens; re-sealing under a newer key then happens on write, in a
//! background pass, or never. The algorithm byte is there for the same kind
//! of reason: a customer with a FIPS-140 requirement gets an AES-256-GCM
//! variant beside XChaCha rather than a migration, because the header names
//! which one sealed each payload.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use synveda_types::{Error, Result};
use zeroize::Zeroizing;

use crate::aad::{self, Purpose, RowKey};
use crate::key::{DataKey, KeyScope, KeyVersion, SCOPE_TAG_DEPLOYMENT, SCOPE_TAG_TENANT};

/// Magic bytes: Synveda Envelope, format 1.
const MAGIC: &[u8; 4] = b"SVE1";
/// XChaCha20-Poly1305, the only algorithm this version defines.
pub const ALGORITHM_XCHACHA20_POLY1305: u8 = 1;
/// The part of the header bound into the AAD: magic, algorithm, scope, version.
const HEADER_PREFIX_LEN: usize = 10;
/// XChaCha20's nonce.
const NONCE_LEN: usize = 24;
/// Where the ciphertext starts.
const HEADER_LEN: usize = HEADER_PREFIX_LEN + NONCE_LEN;
/// Poly1305's tag, which the AEAD appends to the ciphertext.
const TAG_LEN: usize = 16;

/// Reads the key version an envelope was sealed under, without a key.
///
/// This is what makes lazy rotation possible: a key ring peeks the version,
/// loads that generation, and only then opens.
pub fn envelope_version(envelope: &[u8]) -> Result<KeyVersion> {
    let header = parse_header(envelope)?;
    Ok(header.version)
}

/// Reads the scope tag an envelope declares, without a key. `None` for a tag
/// this version does not define — a caller deciding which key ring to ask
/// should treat that as "not mine" rather than guessing.
#[must_use]
pub fn envelope_is_deployment_scoped(envelope: &[u8]) -> Option<bool> {
    let header = parse_header(envelope).ok()?;
    match header.scope_tag {
        SCOPE_TAG_DEPLOYMENT => Some(true),
        SCOPE_TAG_TENANT => Some(false),
        _ => None,
    }
}

struct Header {
    scope_tag: u8,
    version: KeyVersion,
}

fn parse_header(envelope: &[u8]) -> Result<Header> {
    if envelope.len() < HEADER_LEN + TAG_LEN {
        return Err(Error::Invalid {
            message: format!(
                "sealed payload is {} bytes, shorter than the {} a header and tag need",
                envelope.len(),
                HEADER_LEN + TAG_LEN
            ),
        });
    }
    if &envelope[0..4] != MAGIC {
        return Err(Error::Invalid {
            message: "sealed payload does not carry the envelope magic".to_string(),
        });
    }
    let algorithm = envelope[4];
    if algorithm != ALGORITHM_XCHACHA20_POLY1305 {
        return Err(Error::Invalid {
            message: format!("sealed payload names unknown algorithm {algorithm}"),
        });
    }
    let version = u32::from_be_bytes([envelope[6], envelope[7], envelope[8], envelope[9]]);
    Ok(Header {
        scope_tag: envelope[5],
        version: KeyVersion::new(version)?,
    })
}

/// One generation of one scope's data key, and the only thing that seals.
///
/// It holds its own scope and version, so [`seal`](SealingKey::seal) takes
/// neither: it is not possible to seal with one tenant's key while claiming
/// another tenant's scope, because the scope is not a parameter. That is
/// decision 4's binding made structural rather than remembered.
pub struct SealingKey {
    scope: KeyScope,
    version: KeyVersion,
    key: DataKey,
}

impl std::fmt::Debug for SealingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SealingKey")
            .field("scope", &self.scope)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl SealingKey {
    /// Adopts an unwrapped data key for one scope and generation.
    #[must_use]
    pub const fn new(scope: KeyScope, version: KeyVersion, key: DataKey) -> Self {
        SealingKey {
            scope,
            version,
            key,
        }
    }

    /// Which scope this key belongs to.
    #[must_use]
    pub const fn scope(&self) -> KeyScope {
        self.scope
    }

    /// Which generation this key is.
    #[must_use]
    pub const fn version(&self) -> KeyVersion {
        self.version
    }

    /// Seals `plaintext`, binding it to this key's scope and to the purpose
    /// and row given.
    ///
    /// The nonce is 192 random bits from the OS, which is the reason
    /// decision 3 chose XChaCha: at this size a random nonce needs no counter
    /// and no coordination between processes, so OPS-7's second gateway
    /// replica changes nothing here.
    pub fn seal(&self, purpose: Purpose, row: RowKey<'_>, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|err| Error::Internal {
            message: format!("system CSPRNG unavailable: {err}"),
        })?;

        let mut envelope = Vec::with_capacity(HEADER_LEN + plaintext.len() + TAG_LEN);
        envelope.extend_from_slice(MAGIC);
        envelope.push(ALGORITHM_XCHACHA20_POLY1305);
        envelope.push(self.scope.tag());
        envelope.extend_from_slice(&self.version.get().to_be_bytes());
        debug_assert_eq!(envelope.len(), HEADER_PREFIX_LEN);
        let associated = aad::compose(&envelope, self.scope, purpose, row);
        envelope.extend_from_slice(&nonce);

        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.expose()));
        let sealed = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &associated,
                },
            )
            // The AEAD's error carries nothing to leak, and neither does
            // this: no plaintext, no key, no nonce.
            .map_err(|_| Error::Internal {
                message: format!("sealing {} failed", purpose.as_str()),
            })?;
        envelope.extend_from_slice(&sealed);
        Ok(envelope)
    }

    /// Seals another key under this one.
    ///
    /// The KMS wraps a data key this way, and a tenant export wraps its own
    /// per-archive key this way (ADR-0064 decision 8). It exists as its own
    /// method so that key material never has to leave this crate to be
    /// sealed — a caller with a `DataKey` can pass it, and cannot read it.
    pub fn seal_data_key(
        &self,
        purpose: Purpose,
        row: RowKey<'_>,
        key: &DataKey,
    ) -> Result<Vec<u8>> {
        self.seal(purpose, row, key.expose())
    }

    /// Opens a key sealed by [`Self::seal_data_key`].
    pub fn open_data_key(
        &self,
        purpose: Purpose,
        row: RowKey<'_>,
        envelope: &[u8],
    ) -> Result<DataKey> {
        let opened = self.open(purpose, row, envelope)?;
        let bytes: [u8; crate::key::KEY_LEN] =
            opened.as_slice().try_into().map_err(|_| Error::Invalid {
                message: format!(
                    "a sealed key opened to {} bytes, not {}",
                    opened.len(),
                    crate::key::KEY_LEN
                ),
            })?;
        Ok(DataKey::from_bytes(bytes))
    }

    /// Opens an envelope sealed by this key, under the same purpose and row.
    ///
    /// Fails — rather than returning the wrong plaintext — when the envelope
    /// was sealed for another tenant, another column, another row, or another
    /// key generation. A caller that sees this on a row it wrote is looking
    /// at corruption or a transplant, which is why decision 12 makes it an
    /// audit event.
    ///
    /// The plaintext comes back in a [`Zeroizing`] buffer: everything this
    /// crate opens is a credential.
    pub fn open(
        &self,
        purpose: Purpose,
        row: RowKey<'_>,
        envelope: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        let header = parse_header(envelope)?;
        if header.scope_tag != self.scope.tag() {
            return Err(Error::Invalid {
                message: format!(
                    "sealed payload is {}-scoped and this key is {}-scoped",
                    scope_tag_name(header.scope_tag),
                    self.scope.label()
                ),
            });
        }
        if header.version != self.version {
            return Err(Error::Invalid {
                message: format!(
                    "sealed payload names key version {} and this key is version {}",
                    header.version, self.version
                ),
            });
        }
        let associated = aad::compose(&envelope[..HEADER_PREFIX_LEN], self.scope, purpose, row);
        let nonce = &envelope[HEADER_PREFIX_LEN..HEADER_LEN];
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.expose()));
        let opened = cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: &envelope[HEADER_LEN..],
                    aad: &associated,
                },
            )
            // Deliberately uniform: an attacker probing which of scope,
            // purpose or row was wrong learns nothing from this message, and
            // the operator learns what they need from the audit event the
            // caller emits (decision 12).
            .map_err(|_| Error::Invalid {
                message: format!(
                    "sealed payload for {} did not open under this key",
                    purpose.as_str()
                ),
            })?;
        Ok(Zeroizing::new(opened))
    }
}

fn scope_tag_name(tag: u8) -> &'static str {
    match tag {
        SCOPE_TAG_TENANT => "tenant",
        SCOPE_TAG_DEPLOYMENT => "deployment",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use synveda_types::TenantId;

    use super::*;

    fn key_for(scope: KeyScope) -> SealingKey {
        SealingKey::new(
            scope,
            KeyVersion::FIRST,
            DataKey::generate().expect("generate"),
        )
    }

    #[test]
    fn round_trips() {
        let key = key_for(KeyScope::Tenant(TenantId::new()));
        let sealed = key
            .seal(
                Purpose::DirectoryCredential,
                RowKey::Name("graph"),
                b"s3cr3t",
            )
            .expect("seal");
        let opened = key
            .open(Purpose::DirectoryCredential, RowKey::Name("graph"), &sealed)
            .expect("open");
        assert_eq!(&opened[..], b"s3cr3t");
    }

    #[test]
    fn ciphertext_does_not_contain_the_plaintext() {
        let key = key_for(KeyScope::Deployment);
        let sealed = key
            .seal(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[3_u8; 32]),
                b"bearer-token-value",
            )
            .expect("seal");
        assert!(
            !sealed
                .windows(b"bearer".len())
                .any(|window| window == b"bearer"),
            "plaintext must not survive in the envelope"
        );
    }

    #[test]
    fn a_ciphertext_moved_between_tenants_does_not_open() {
        // The property TEN-6 fuzzes for, and the reason decision 4 exists.
        let one = TenantId::new();
        let two = TenantId::new();
        let material = DataKey::generate().expect("generate");
        let bytes = *material.expose();
        // Same key material, different scope: only the AAD differs, which is
        // what makes this test about the binding rather than about the key.
        let first = SealingKey::new(
            KeyScope::Tenant(one),
            KeyVersion::FIRST,
            DataKey::from_bytes(bytes),
        );
        let second = SealingKey::new(
            KeyScope::Tenant(two),
            KeyVersion::FIRST,
            DataKey::from_bytes(bytes),
        );
        let sealed = first
            .seal(Purpose::DirectoryCredential, RowKey::Name("graph"), b"x")
            .expect("seal");
        assert!(
            second
                .open(Purpose::DirectoryCredential, RowKey::Name("graph"), &sealed)
                .is_err(),
            "a ciphertext must not open under another tenant's scope"
        );
    }

    #[test]
    fn a_ciphertext_moved_between_columns_does_not_open() {
        let key = key_for(KeyScope::Deployment);
        let sealed = key
            .seal(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[1_u8; 32]),
                b"token",
            )
            .expect("seal");
        assert!(
            key.open(
                Purpose::ConsoleRefreshToken,
                RowKey::Hash(&[1_u8; 32]),
                &sealed
            )
            .is_err()
        );
    }

    #[test]
    fn a_ciphertext_moved_between_rows_does_not_open() {
        let key = key_for(KeyScope::Deployment);
        let sealed = key
            .seal(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[1_u8; 32]),
                b"token",
            )
            .expect("seal");
        assert!(
            key.open(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[2_u8; 32]),
                &sealed
            )
            .is_err()
        );
    }

    #[test]
    fn a_different_key_does_not_open() {
        let scope = KeyScope::Tenant(TenantId::new());
        let sealed = key_for(scope)
            .seal(Purpose::TenantExport, RowKey::Name("archive"), b"rows")
            .expect("seal");
        assert!(
            key_for(scope)
                .open(Purpose::TenantExport, RowKey::Name("archive"), &sealed)
                .is_err(),
            "this is the AC in miniature: no key, no plaintext"
        );
    }

    #[test]
    fn the_wrong_generation_is_refused_by_version_rather_than_by_the_tag() {
        let scope = KeyScope::Tenant(TenantId::new());
        let material = DataKey::generate().expect("generate");
        let bytes = *material.expose();
        let first = SealingKey::new(scope, KeyVersion::FIRST, DataKey::from_bytes(bytes));
        let second = SealingKey::new(scope, KeyVersion::FIRST.next(), DataKey::from_bytes(bytes));
        let sealed = first
            .seal(Purpose::TenantExport, RowKey::Name("archive"), b"rows")
            .expect("seal");
        let err = second
            .open(Purpose::TenantExport, RowKey::Name("archive"), &sealed)
            .expect_err("must refuse");
        assert!(
            err.to_string().contains("key version"),
            "a caller reading this needs to know to fetch another generation: {err}"
        );
    }

    #[test]
    fn the_version_is_readable_without_a_key() {
        let key = SealingKey::new(
            KeyScope::Deployment,
            KeyVersion::new(7).expect("seven"),
            DataKey::generate().expect("generate"),
        );
        let sealed = key
            .seal(Purpose::ConsoleAccessToken, RowKey::Hash(&[0_u8; 32]), b"t")
            .expect("seal");
        assert_eq!(
            envelope_version(&sealed).expect("peek"),
            KeyVersion::new(7).expect("seven")
        );
        assert_eq!(envelope_is_deployment_scoped(&sealed), Some(true));
    }

    #[test]
    fn a_tampered_header_breaks_the_tag_rather_than_being_honoured() {
        let key = key_for(KeyScope::Deployment);
        let sealed = key
            .seal(Purpose::ConsoleAccessToken, RowKey::Hash(&[0_u8; 32]), b"t")
            .expect("seal");

        // Flip the scope tag: caught by the explicit check.
        let mut scope_edited = sealed.clone();
        scope_edited[5] = SCOPE_TAG_TENANT;
        assert!(
            key.open(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[0_u8; 32]),
                &scope_edited
            )
            .is_err()
        );

        // Flip a byte of the ciphertext: caught by Poly1305.
        let mut body_edited = sealed.clone();
        let last = body_edited.len() - 1;
        body_edited[last] ^= 0x01;
        assert!(
            key.open(
                Purpose::ConsoleAccessToken,
                RowKey::Hash(&[0_u8; 32]),
                &body_edited
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_envelopes_are_refused_before_any_key_work() {
        assert!(envelope_version(b"").is_err());
        assert!(envelope_version(&[0_u8; HEADER_LEN + TAG_LEN]).is_err());
        let mut wrong_algorithm = vec![0_u8; HEADER_LEN + TAG_LEN];
        wrong_algorithm[0..4].copy_from_slice(MAGIC);
        wrong_algorithm[4] = 99;
        let err = envelope_version(&wrong_algorithm).expect_err("unknown algorithm");
        assert!(err.to_string().contains("algorithm"));
    }

    #[test]
    fn two_seals_of_the_same_plaintext_differ() {
        // Nonce reuse would show up here first.
        let key = key_for(KeyScope::Deployment);
        let a = key
            .seal(Purpose::ConsoleAccessToken, RowKey::Hash(&[0_u8; 32]), b"t")
            .expect("seal");
        let b = key
            .seal(Purpose::ConsoleAccessToken, RowKey::Hash(&[0_u8; 32]), b"t")
            .expect("seal");
        assert_ne!(a, b);
    }

    #[test]
    fn debug_does_not_print_key_material() {
        let key = key_for(KeyScope::Deployment);
        let rendered = format!("{key:?}");
        assert!(rendered.contains("Deployment"), "{rendered}");
        assert!(!rendered.contains("DataKey("), "{rendered}");
    }

    #[test]
    fn an_empty_payload_still_seals_and_opens() {
        let key = key_for(KeyScope::Deployment);
        let sealed = key
            .seal(Purpose::ConsoleRefreshToken, RowKey::Hash(&[0_u8; 32]), b"")
            .expect("seal");
        let opened = key
            .open(
                Purpose::ConsoleRefreshToken,
                RowKey::Hash(&[0_u8; 32]),
                &sealed,
            )
            .expect("open");
        assert!(opened.is_empty());
    }
}

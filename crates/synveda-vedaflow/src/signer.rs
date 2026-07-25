//! Commit signatures (FLOW-1, ADR-0030 decision 9).
//!
//! A signature covers the 32-byte commit hash, which already covers the tree,
//! the parents, the author, the timestamp, the message, and the policy
//! snapshot. That means a verifier needs the hash and a public key — no
//! re-encoding, no schema knowledge — which is what lets a FLOW-8 git mirror
//! stay verifiable outside this database.
//!
//! The default is [`Signer::Unsigned`], which writes NULL. A commit nobody
//! signed says so; a column that always held a signature over nothing would
//! be worse than an empty one. Key *management* is deferred — TEN-4's
//! per-tenant keys are its natural home — so for now a key arrives as
//! configuration and is named by `signer_key_id` so rotation is expressible
//! and a verifier can find the right public key.

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use synveda_types::{Error, Result};

use crate::hash::CommitHash;

/// What a signer produced for one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSignature {
    /// The signature bytes.
    pub signature: Vec<u8>,
    /// Which key produced it — stored beside the signature so rotation is
    /// expressible and a verifier knows which public key to fetch.
    pub key_id: String,
}

/// Signs commit hashes.
///
/// The seam mirrors `Extractor`/`Embedder` (ADR-0022/ADR-0023): a trait for
/// the contract, an enum for static dispatch, and a `method()` name that
/// shows up in metrics and logs.
pub trait CommitSigner {
    /// The stable method name (`unsigned`, `ed25519`).
    fn method(&self) -> &'static str;

    /// Signs `hash`, or returns `None` when this signer signs nothing.
    fn sign(&self, hash: CommitHash) -> Option<CommitSignature>;
}

/// The configured signer, dispatched statically.
///
/// Boxed key material: an `Ed25519Signer` is two hundred-odd bytes of
/// expanded key, and this enum is passed by reference on every commit.
#[derive(Debug, Clone, Default)]
pub enum Signer {
    /// Signs nothing; commits record a NULL signature. The default.
    #[default]
    Unsigned,
    /// Ed25519 over the commit hash.
    Ed25519(Box<Ed25519Signer>),
}

impl CommitSigner for Signer {
    fn method(&self) -> &'static str {
        match self {
            Signer::Unsigned => "unsigned",
            Signer::Ed25519(signer) => signer.method(),
        }
    }

    fn sign(&self, hash: CommitHash) -> Option<CommitSignature> {
        match self {
            Signer::Unsigned => None,
            Signer::Ed25519(signer) => signer.sign(hash),
        }
    }
}

/// Ed25519 signing over commit hashes.
#[derive(Clone)]
pub struct Ed25519Signer {
    key: SigningKey,
    key_id: String,
}

// The key never reaches a log line, a span field, or an error message.
impl std::fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ed25519Signer")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl Ed25519Signer {
    /// Builds a signer from a 32-byte seed and the id this key is known by.
    ///
    /// The seed is configuration, never generated here: `ed25519-dalek` is
    /// taken without its `rand_core` feature precisely so no key can be
    /// invented in-process and then lost on restart.
    pub fn new(seed: [u8; 32], key_id: impl Into<String>) -> Result<Self> {
        let key_id = key_id.into();
        if key_id.is_empty() || key_id.len() > 128 {
            return Err(Error::Invalid {
                message: format!(
                    "signing key id must be 1..=128 characters, got {}",
                    key_id.len()
                ),
            });
        }
        Ok(Ed25519Signer {
            key: SigningKey::from_bytes(&seed),
            key_id,
        })
    }

    /// Builds a signer from a 64-character hex seed — the configuration form.
    pub fn from_hex_seed(seed: &str, key_id: impl Into<String>) -> Result<Self> {
        let seed = decode_hex_32(seed).ok_or_else(|| Error::Invalid {
            message: "signing key seed must be 64 hex characters (32 bytes)".to_string(),
        })?;
        Ed25519Signer::new(seed, key_id)
    }

    /// The public key a verifier needs, as raw bytes.
    #[must_use]
    pub fn verifying_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// This signer's key id.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl CommitSigner for Ed25519Signer {
    fn method(&self) -> &'static str {
        "ed25519"
    }

    fn sign(&self, hash: CommitHash) -> Option<CommitSignature> {
        Some(CommitSignature {
            signature: self.key.sign(hash.as_slice()).to_vec(),
            key_id: self.key_id.clone(),
        })
    }
}

/// Verifies an Ed25519 commit signature against a public key.
///
/// The whole verification surface: a commit hash, a signature, and a key.
/// Nothing here reads the database, so an offline verifier — or the FLOW-8
/// mirror — can use it unchanged.
#[must_use]
pub fn verify_ed25519(hash: CommitHash, signature: &[u8], verifying_key: &[u8; 32]) -> bool {
    let (Ok(key), Ok(signature)) = (
        VerifyingKey::from_bytes(verifying_key),
        Signature::from_slice(signature),
    ) else {
        return false;
    };
    key.verify(hash.as_slice(), &signature).is_ok()
}

/// Decodes exactly 32 bytes of hex. Sixteen lines beats a dependency.
fn decode_hex_32(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(input.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [42u8; 32];

    fn hash(byte: u8) -> CommitHash {
        CommitHash::from_bytes([byte; 32])
    }

    #[test]
    fn the_default_signer_signs_nothing_and_says_so() {
        let signer = Signer::default();
        assert_eq!(signer.method(), "unsigned");
        assert!(signer.sign(hash(1)).is_none());
    }

    #[test]
    fn ed25519_signatures_verify_and_bind_the_commit() {
        let signer = Ed25519Signer::new(SEED, "dev-key-1").unwrap();
        let signed = signer.sign(hash(1)).expect("ed25519 signs");
        assert_eq!(signed.key_id, "dev-key-1");
        assert!(verify_ed25519(
            hash(1),
            &signed.signature,
            &signer.verifying_key()
        ));
        // A signature over one commit does not verify another.
        assert!(!verify_ed25519(
            hash(2),
            &signed.signature,
            &signer.verifying_key()
        ));
        // Nor under a different key.
        let other = Ed25519Signer::new([7u8; 32], "dev-key-2").unwrap();
        assert!(!verify_ed25519(
            hash(1),
            &signed.signature,
            &other.verifying_key()
        ));
    }

    #[test]
    fn signing_is_deterministic_so_the_same_commit_signs_identically() {
        let signer = Ed25519Signer::new(SEED, "k").unwrap();
        assert_eq!(signer.sign(hash(3)), signer.sign(hash(3)));
    }

    #[test]
    fn malformed_signatures_fail_closed_rather_than_panicking() {
        let signer = Ed25519Signer::new(SEED, "k").unwrap();
        assert!(!verify_ed25519(hash(1), &[], &signer.verifying_key()));
        assert!(!verify_ed25519(
            hash(1),
            &[0u8; 63],
            &signer.verifying_key()
        ));
    }

    #[test]
    fn hex_seeds_round_trip_and_reject_the_wrong_length() {
        let hex = "2a".repeat(32);
        let from_hex = Ed25519Signer::from_hex_seed(&hex, "k").unwrap();
        let from_bytes = Ed25519Signer::new(SEED, "k").unwrap();
        assert_eq!(from_hex.verifying_key(), from_bytes.verifying_key());
        assert!(Ed25519Signer::from_hex_seed("2a", "k").is_err());
        assert!(Ed25519Signer::from_hex_seed(&"zz".repeat(32), "k").is_err());
    }

    #[test]
    fn a_key_id_is_required_and_bounded() {
        assert!(Ed25519Signer::new(SEED, "").is_err());
        assert!(Ed25519Signer::new(SEED, "x".repeat(129)).is_err());
        assert!(Ed25519Signer::new(SEED, "x".repeat(128)).is_ok());
    }

    #[test]
    fn the_signing_key_never_appears_in_debug_output() {
        let signer = Ed25519Signer::new(SEED, "dev-key-1").unwrap();
        let rendered = format!("{signer:?}");
        assert!(rendered.contains("dev-key-1"));
        assert!(!rendered.contains("42"), "{rendered}");
    }
}

//! Content addressing (FLOW-1, ADR-0030 decision 2).
//!
//! Every hash is `BLAKE3(domain ‖ length-prefixed fields)`: git's
//! `type ‖ len ‖ NUL ‖ payload` generalised. Two properties do the work:
//!
//! - **One domain separator per object kind.** The three hash spaces are
//!   disjoint by construction, so a tree hash can never collide with a
//!   commit hash even before the type system gets involved.
//! - **Every variable-length field is preceded by its length** as a
//!   big-endian `u64`. Without that, `("ab", "c")` and `("a", "bc")` hash
//!   alike, and whoever controls two adjacent fields controls the address.
//!
//! The tenant is deliberately absent from every encoding: an auditor holding
//! the bytes, or the FLOW-8 git mirror holding an exported object,
//! recomputes the same address with no access to our schema. Isolation is
//! RLS's job (ADR-0009); storage is keyed `(tenant_id, hash)` regardless, so
//! identical content dedups inside a tenant and never across one (ADR-0030
//! decision 3).
//!
//! The `-v1` suffixes are the only migration path this encoding has. Changing
//! any of the routines below without bumping one silently re-addresses
//! history.

use std::fmt;
use std::str::FromStr;

use blake3::Hasher;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use synveda_types::{AssetKind, Error, IdentityId, Result};

/// Domain separator for object (blob) hashes.
const OBJECT_DOMAIN: &[u8] = b"synveda-vedaflow-object-v1";
/// Domain separator for tree hashes.
const TREE_DOMAIN: &[u8] = b"synveda-vedaflow-tree-v1";
/// Domain separator for commit hashes.
const COMMIT_DOMAIN: &[u8] = b"synveda-vedaflow-commit-v1";
/// Domain separator for policy-snapshot hashes.
const POLICY_SNAPSHOT_DOMAIN: &[u8] = b"synveda-vedaflow-policy-snapshot-v1";

/// Tag distinguishing a tree entry that points at an object.
pub(crate) const TARGET_TAG_OBJECT: u8 = 0x01;
/// Tag distinguishing a tree entry that points at a subtree.
pub(crate) const TARGET_TAG_TREE: u8 = 0x02;

/// Defines one 32-byte content-address newtype.
///
/// Separate types per object kind for the reason `define_id!` gives in
/// `synveda-types`: a `TreeHash` can never be passed where a `CommitHash` is
/// expected. On the wire and in logs all of them are lowercase hex.
macro_rules! define_hash {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name([u8; 32]);

        impl $name {
            /// Wraps 32 raw bytes, e.g. a freshly computed digest.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// The raw digest.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// The digest as a byte slice, for binding into SQL.
            #[must_use]
            pub const fn as_slice(&self) -> &[u8] {
                &self.0
            }

            /// Lowercase hex — the rendering used in logs, CLI output, and
            /// injected watermarks.
            #[must_use]
            pub fn to_hex(&self) -> String {
                blake3::Hash::from_bytes(self.0).to_hex().to_string()
            }

            /// Reads a digest back from a stored `bytea`. A wrong length
            /// means schema and code have drifted, which is our bug.
            pub fn from_slice(bytes: &[u8]) -> Result<Self> {
                <[u8; 32]>::try_from(bytes)
                    .map(Self)
                    .map_err(|_| Error::Internal {
                        message: format!(
                            concat!(stringify!($name), " must be 32 bytes, got {}"),
                            bytes.len()
                        ),
                    })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<Self> {
                blake3::Hash::from_hex(s)
                    .map(|hash| Self(*hash.as_bytes()))
                    .map_err(|err| Error::Invalid {
                        message: format!(
                            concat!("not a valid ", stringify!($name), ": {}"), err
                        ),
                    })
            }
        }
    };
}

define_hash!(
    /// The content address of an immutable blob — `BLAKE3` over its asset
    /// kind and its bytes.
    ObjectHash
);

define_hash!(
    /// The content address of a tree — `BLAKE3` over its name-sorted entries.
    TreeHash
);

define_hash!(
    /// The content address of a commit — `BLAKE3` over its tree, parents,
    /// author, timestamp, message, and policy snapshot.
    CommitHash
);

define_hash!(
    /// The address of the policy pack that governed a commit (ADR-0030
    /// decision 8). Not an object in the store; a fingerprint the caller
    /// computes from the pack it already resolved.
    PolicySnapshotHash
);

/// A length-prefixing hasher. Every variable-length field goes in through
/// [`Writer::field`]; fixed-width values go in raw.
pub(crate) struct Writer(Hasher);

impl Writer {
    /// Starts a hash in `domain`'s space.
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(domain);
        Self(hasher)
    }

    /// Absorbs a variable-length field, length-prefixed.
    pub(crate) fn field(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(&(bytes.len() as u64).to_be_bytes());
        self.0.update(bytes);
        self
    }

    /// Absorbs a fixed-width count (element counts, tags with known width).
    pub(crate) fn count(&mut self, value: u64) -> &mut Self {
        self.0.update(&value.to_be_bytes());
        self
    }

    /// Absorbs a fixed-width byte string — a 32-byte digest, a UUID, a tag.
    pub(crate) fn fixed(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self
    }

    /// Finishes the hash.
    pub(crate) fn finish(&self) -> [u8; 32] {
        *self.0.finalize().as_bytes()
    }
}

/// An object's content address: `kind` then `content`, both length-prefixed.
///
/// `kind` is inside the address on purpose (ADR-0030 decision 4) — identical
/// bytes registered as a prompt and as a skill are two different objects,
/// because they are governed differently.
#[must_use]
pub fn object_hash(kind: AssetKind, content: &[u8]) -> ObjectHash {
    let mut writer = Writer::new(OBJECT_DOMAIN);
    writer.field(kind.as_str().as_bytes()).field(content);
    ObjectHash::from_bytes(writer.finish())
}

/// One entry of a tree as it enters the hash: a name and what it points at.
pub(crate) struct HashedEntry<'a> {
    pub(crate) name: &'a str,
    pub(crate) tag: u8,
    pub(crate) target: &'a [u8; 32],
}

/// A tree's content address: the entry count, then each entry's name, target
/// tag, and target hash — entries in bytewise name order.
///
/// Because a tree's address covers its children's addresses, a tree cannot
/// contain itself without a preimage attack: cycles are impossible by
/// construction, not by a check (ADR-0030 decision 5).
pub(crate) fn tree_hash_from(entries: &[HashedEntry<'_>]) -> TreeHash {
    let mut writer = Writer::new(TREE_DOMAIN);
    writer.count(entries.len() as u64);
    for entry in entries {
        writer
            .field(entry.name.as_bytes())
            .fixed(&[entry.tag])
            .fixed(entry.target);
    }
    TreeHash::from_bytes(writer.finish())
}

/// A commit's content address.
///
/// `committed_at` is hashed as RFC 3339 UTC with exactly six fractional
/// digits — the AUD-1 canonical timestamp rule (ADR-0019 decision 2) — so a
/// recomputation from the stored `timestamptz` is byte-exact. Callers pass an
/// instant already truncated by [`truncate_to_micros`].
pub(crate) fn commit_hash_from(
    tree: TreeHash,
    parents: &[CommitHash],
    author: IdentityId,
    committed_at: DateTime<Utc>,
    message: &str,
    policy_snapshot: PolicySnapshotHash,
) -> CommitHash {
    let mut writer = Writer::new(COMMIT_DOMAIN);
    writer.fixed(tree.as_slice()).count(parents.len() as u64);
    for parent in parents {
        writer.fixed(parent.as_slice());
    }
    writer
        .fixed(author.as_uuid().as_bytes())
        .field(canonical_timestamp(committed_at).as_bytes())
        .field(message.as_bytes())
        .fixed(policy_snapshot.as_slice());
    CommitHash::from_bytes(writer.finish())
}

/// Starts a policy-snapshot hash. The fields go in from
/// [`crate::policy::PolicySnapshot`], which owns their canonical form.
pub(crate) fn policy_snapshot_writer() -> Writer {
    Writer::new(POLICY_SNAPSHOT_DOMAIN)
}

/// RFC 3339 UTC with exactly six fractional digits — the one timestamp
/// rendering the hash and the stored column agree on.
fn canonical_timestamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// Truncates to whole microseconds, so the value hashed is the value
/// `timestamptz` stores — no rounding on insert, no drift on read.
#[must_use]
pub fn truncate_to_micros(at: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_micros(at.timestamp_micros())
        .expect("a valid DateTime survives microsecond truncation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn object_addresses_bind_content_and_kind() {
        let a = object_hash(AssetKind::Prompt, b"hello");
        assert_eq!(a, object_hash(AssetKind::Prompt, b"hello"), "recomputable");
        assert_ne!(a, object_hash(AssetKind::Prompt, b"hellp"), "content-bound");
        // The decision that makes decision 4 real: same bytes, different
        // governance, different object.
        assert_ne!(a, object_hash(AssetKind::Skill, b"hello"), "kind-bound");
    }

    #[test]
    fn length_prefixing_stops_field_boundaries_from_sliding() {
        // Without length prefixes these two would hash identically, and an
        // author who controls a name could forge a target hash.
        let target = [7u8; 32];
        let ab_c = tree_hash_from(&[
            HashedEntry {
                name: "ab",
                tag: TARGET_TAG_OBJECT,
                target: &target,
            },
            HashedEntry {
                name: "c",
                tag: TARGET_TAG_OBJECT,
                target: &target,
            },
        ]);
        let a_bc = tree_hash_from(&[
            HashedEntry {
                name: "a",
                tag: TARGET_TAG_OBJECT,
                target: &target,
            },
            HashedEntry {
                name: "bc",
                tag: TARGET_TAG_OBJECT,
                target: &target,
            },
        ]);
        assert_ne!(ab_c, a_bc);
    }

    #[test]
    fn tree_entry_target_kind_is_part_of_the_address() {
        let target = [9u8; 32];
        let as_object = tree_hash_from(&[HashedEntry {
            name: "x",
            tag: TARGET_TAG_OBJECT,
            target: &target,
        }]);
        let as_subtree = tree_hash_from(&[HashedEntry {
            name: "x",
            tag: TARGET_TAG_TREE,
            target: &target,
        }]);
        assert_ne!(as_object, as_subtree);
    }

    #[test]
    fn empty_tree_has_a_stable_address_distinct_from_an_empty_object() {
        let empty = tree_hash_from(&[]);
        assert_eq!(empty, tree_hash_from(&[]));
        // Disjoint domains: no encoding of an object can collide with a tree.
        assert_ne!(
            empty.as_bytes(),
            object_hash(AssetKind::Memory, b"").as_bytes()
        );
    }

    #[test]
    fn commit_addresses_bind_every_field() {
        let tree = TreeHash::from_bytes([1u8; 32]);
        let other_tree = TreeHash::from_bytes([2u8; 32]);
        let parent = CommitHash::from_bytes([3u8; 32]);
        let author = IdentityId::new();
        let at = Utc.with_ymd_and_hms(2026, 7, 25, 9, 0, 0).unwrap();
        let pack = PolicySnapshotHash::from_bytes([4u8; 32]);
        let base = commit_hash_from(tree, &[], author, at, "msg", pack);

        assert_eq!(base, commit_hash_from(tree, &[], author, at, "msg", pack));
        assert_ne!(
            base,
            commit_hash_from(other_tree, &[], author, at, "msg", pack)
        );
        assert_ne!(
            base,
            commit_hash_from(tree, &[parent], author, at, "msg", pack)
        );
        assert_ne!(
            base,
            commit_hash_from(tree, &[], IdentityId::new(), at, "msg", pack)
        );
        assert_ne!(
            base,
            commit_hash_from(
                tree,
                &[],
                author,
                at + chrono::Duration::microseconds(1),
                "msg",
                pack
            )
        );
        assert_ne!(base, commit_hash_from(tree, &[], author, at, "other", pack));
        assert_ne!(
            base,
            commit_hash_from(
                tree,
                &[],
                author,
                at,
                "msg",
                PolicySnapshotHash::from_bytes([5u8; 32])
            )
        );
    }

    #[test]
    fn parent_order_is_part_of_the_commit_address() {
        // First parent is the mainline, as in git; swapping them is a
        // different commit.
        let tree = TreeHash::from_bytes([1u8; 32]);
        let (a, b) = (
            CommitHash::from_bytes([2u8; 32]),
            CommitHash::from_bytes([3u8; 32]),
        );
        let author = IdentityId::new();
        let at = Utc.with_ymd_and_hms(2026, 7, 25, 9, 0, 0).unwrap();
        let pack = PolicySnapshotHash::from_bytes([4u8; 32]);
        assert_ne!(
            commit_hash_from(tree, &[a, b], author, at, "m", pack),
            commit_hash_from(tree, &[b, a], author, at, "m", pack)
        );
    }

    #[test]
    fn hex_round_trips_and_rejects_garbage() {
        let hash = object_hash(AssetKind::Memory, b"content");
        assert_eq!(hash.to_hex().parse::<ObjectHash>().unwrap(), hash);
        assert!(matches!(
            "not-hex".parse::<ObjectHash>(),
            Err(Error::Invalid { .. })
        ));
    }

    #[test]
    fn short_stored_digests_are_an_internal_error_not_a_silent_pad() {
        assert!(matches!(
            CommitHash::from_slice(&[0u8; 31]),
            Err(Error::Internal { .. })
        ));
        assert!(CommitHash::from_slice(&[0u8; 32]).is_ok());
    }

    #[test]
    fn timestamps_truncate_to_microseconds_idempotently() {
        let at = Utc.with_ymd_and_hms(2026, 7, 25, 9, 0, 0).unwrap()
            + chrono::Duration::nanoseconds(1_999);
        let truncated = truncate_to_micros(at);
        assert_eq!(truncated.timestamp_subsec_nanos(), 1_000);
        assert_eq!(truncate_to_micros(truncated), truncated);
        assert_eq!(
            canonical_timestamp(truncated),
            "2026-07-25T09:00:00.000001Z"
        );
    }
}

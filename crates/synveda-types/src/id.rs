//! UUID-backed identifier newtypes.
//!
//! All identifiers are UUID version 7 (ADR-0005): time-ordered for b-tree
//! locality in Postgres, mintable anywhere without coordination. Per-concept
//! newtypes exist so a `RecordId` can never be passed where a `TenantId` is
//! expected; on the wire all of them are canonical hyphenated UUID strings.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Defines one identifier newtype. All four identifiers are deliberately
/// identical in behaviour; only the concept they name differs.
macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        // No `Default` on purpose: a nil UUID is never a valid identifier, and
        // a `Default` that mints a fresh random one would let `#[derive(Default)]`
        // on containing structs invent identifiers silently.
        #[allow(clippy::new_without_default)]
        impl $name {
            /// Mints a new time-ordered (UUIDv7) identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wraps an existing UUID, e.g. one read back from storage.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.as_hyphenated().fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::from_str(s).map(Self)
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }
    };
}

define_id!(
    /// Identifies a tenant — the root isolation boundary (seed §4.1).
    TenantId
);

define_id!(
    /// Identifies a node in the tenancy hierarchy (org, division, department,
    /// team, or user level). Every scope is an attachment point for memories,
    /// skills, prompts, and policies (seed §4.1).
    ScopeId
);

define_id!(
    /// Identifies an actor: a user or a service identity. Every agent runs
    /// *as* an identity in the hierarchy, never as a shared key (seed §5).
    IdentityId
);

define_id!(
    /// Identifies a memory record, the atomic unit of stored knowledge
    /// (seed §4.2).
    RecordId
);

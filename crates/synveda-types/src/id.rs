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

/// Defines one identifier newtype. All identifiers are deliberately
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

define_id!(
    /// Identifies a buffered observe event — one entry in the ingestion
    /// staging table, admitted once per idempotency key (MEM-1, ADR-0020).
    ObserveEventId
);

define_id!(
    /// Identifies a VedaFlow proposal — one governed request to move a
    /// reviewed set of assets onto a scope's published channel (FLOW-3,
    /// ADR-0032).
    ProposalId
);

define_id!(
    /// Identifies a policy lapse — one time-boxed, dual-approved grant of a
    /// single action from one scope to another (AUTHZ-4, ADR-0037).
    LapseId
);

define_id!(
    /// Identifies a graph vertex — one thing the knowledge graph can talk
    /// about, which is identity rather than a claim and therefore carries
    /// no history (GRPH-1, ADR-0043 decision 5).
    GraphVertexId
);

define_id!(
    /// Identifies a graph edge — one claim about a relation, bitemporal
    /// like a record and superseded the same way (GRPH-1, ADR-0043
    /// decisions 3 and 4). Stable across the versions of that claim.
    GraphEdgeId
);

define_id!(
    /// Identifies a directory user — one row of the SCIM mirror, and the
    /// `id` a provisioning agent stores and addresses that resource by
    /// forever (AUTH-4, ADR-0059 decision 3). Deliberately distinct from
    /// the [`IdentityId`] it projects onto: a rehire is a new identity and
    /// a new personal scope, and the client must still be able to fetch
    /// the resource by the id it holds.
    DirectoryUserId
);
define_id!(
    /// Identifies a directory group — one row of the SCIM mirror, whose
    /// `displayName` is what the AUTH-2 mapping resolver sees (AUTH-4,
    /// ADR-0059 decision 6).
    DirectoryGroupId
);
define_id!(
    /// Identifies a provisioning credential — the static bearer a SCIM
    /// client authenticates with (AUTH-4, ADR-0059 decision 13).
    ScimCredentialId
);

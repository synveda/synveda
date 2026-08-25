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
    /// Identifies a node of the governed scope tree (CPR-3, ADR-0070): a
    /// named node with a parent and a subtree, shaped `tenant`, `org_unit`,
    /// `workspace`, `project` or `principal`. A shape, never a rank — the
    /// `org`/`division`/`department`/`team`/`user` ladder this doc used to
    /// name left with the hierarchy (CPR-7, ADR-0074). Every scope is an
    /// attachment point for memories, skills, prompts, and policies.
    ScopeId
);

define_id!(
    /// Identifies an actor: a user or a service identity. Every agent runs
    /// *as* an identity holding its own `principal` scope, never as a shared
    /// key (seed §5; CPR-7, ADR-0074 decision 3).
    IdentityId
);

define_id!(
    /// Identifies a memory record, the atomic unit of stored knowledge
    /// (seed §4.2).
    RecordId
);

define_id!(
    /// Identifies one stable Knowledge aggregate (CPR-15, ADR-0080).
    ///
    /// The id survives edits, verification and lifecycle changes; callers
    /// that need exact content cite a [`KnowledgeRevisionId`] instead.
    KnowledgeItemId
);

define_id!(
    /// Identifies one immutable content revision of a Knowledge item
    /// (CPR-15, ADR-0080).
    KnowledgeRevisionId
);

define_id!(
    /// Identifies one normalised provenance source (CPR-15, ADR-0080).
    KnowledgeSourceId
);

define_id!(
    /// Identifies one explicit relation between stable Knowledge items
    /// (CPR-15, ADR-0080).
    KnowledgeRelationId
);

define_id!(
    /// Identifies one durable set of exact Knowledge/capture conflict evidence
    /// (CPR-37, ADR-0096).
    ConflictSetId
);

define_id!(
    /// Identifies one exact member retained in a Knowledge conflict set
    /// (CPR-37, ADR-0096).
    ConflictMemberId
);

define_id!(
    /// Identifies one durable external knowledge-format import plan (CPR-27,
    /// ADR-0087).
    ImportJobId
);

define_id!(
    /// Identifies one immutable admitted artifact inside an import job
    /// (CPR-27, ADR-0087).
    ImportArtifactId
);

define_id!(
    /// Identifies one deterministic artifact-to-candidate mapping in an
    /// import plan (CPR-27, ADR-0087).
    ImportMappingId
);

define_id!(
    /// Identifies one durable background operation (CPR-16, ADR-0081).
    ///
    /// The operation is a retry-safe address for work whose transaction may
    /// outlive the request that proposed it, beginning with Knowledge erasure.
    DurableOperationId
);

define_id!(
    /// Identifies one stable tenant-secret aggregate (CPR-35, ADR-0094).
    ///
    /// Credential and data-key rotation preserve this address so immutable
    /// artifacts can cite it without embedding or being rewritten with the
    /// secret value.
    TenantSecretId
);

define_id!(
    /// Identifies one durable tenant-secret DEK re-encryption job (CPR-35,
    /// ADR-0094).
    TenantSecretReencryptionJobId
);

define_id!(
    /// Identifies a VedaFlow proposal — one governed request to move a
    /// reviewed set of assets onto a scope's published channel (FLOW-3,
    /// ADR-0032).
    ProposalId
);

define_id!(
    /// Identifies one stable governed policy relaxation aggregate (CPR-31,
    /// ADR-0090).
    RelaxationId
);

define_id!(
    /// Identifies one immutable version of a governed policy relaxation
    /// (CPR-31, ADR-0090).
    RelaxationVersionId
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
    /// Identifies a provisioning credential — the static bearer a SCIM
    /// client authenticates with (AUTH-4, ADR-0059 decision 13).
    ScimCredentialId
);

define_id!(
    /// Identifies a workspace — the product-level collaboration space that
    /// owns a `workspace`-shaped governed scope (CPR-4, ADR-0071).
    ///
    /// Distinct from the [`ScopeId`] it carries, deliberately: the scope is
    /// what policy, bindings and assets attach to, and the workspace is the
    /// product noun a person names. One is not the other's alias, and a
    /// route that took a scope id where a workspace id belongs would be
    /// admitting a project's scope by mistake.
    WorkspaceId
);

define_id!(
    /// Identifies a project — a unit of work inside a workspace, owning a
    /// `project`-shaped governed scope (CPR-4, ADR-0071).
    ProjectId
);

define_id!(
    /// Identifies one repository attached to a project (CPR-4, ADR-0071).
    ///
    /// The row's *identity* to a human is its canonical URI; this is the
    /// handle the API deletes by, so re-attaching a repository somebody
    /// detached is a new row rather than a resurrection.
    RepositoryId
);

define_id!(
    /// Identifies a group — a named set of principals a grant can be made to
    /// (CPR-5, ADR-0072).
    ///
    /// A group is not a scope and never becomes one: a scope is a place
    /// authority applies *at*, a group is a set authority applies *to*, and
    /// collapsing the two is how a membership list turns into a second tree.
    GroupId
);

define_id!(
    /// Identifies one scope grant — one subject's one role at one scope
    /// (CPR-5, ADR-0072). The handle a revocation names.
    GrantId
);

define_id!(
    /// Identifies one outstanding invitation (CPR-5, ADR-0072).
    ///
    /// Deliberately distinct from the token that redeems it: the id is safe
    /// to list, log and audit, and the token is a bearer credential that
    /// exists once in one response.
    InviteId
);

define_id!(
    /// Identifies one agent session — a run of an agent in a workspace
    /// (CPR-10, ADR-0076). The root of runtime activity: every session
    /// event, context run and audit entry for that run names it.
    SessionId
);

define_id!(
    /// Identifies one immutable session event (CPR-10, ADR-0076).
    ///
    /// Distinct from the client's own `client_event_id`, deliberately: that
    /// one is the caller's idempotency key and is unique only inside a
    /// session, while this is the row's identity in this deployment.
    SessionEventId
);

define_id!(
    /// Identifies one stable Agent Skill aggregate (CPR-23, ADR-0085).
    SkillId
);

define_id!(
    /// Identifies one immutable version of an Agent Skill bundle (CPR-23,
    /// ADR-0085).
    SkillVersionId
);

define_id!(
    /// Identifies one revisioned project- or principal-scope skill binding
    /// (CPR-23, ADR-0085).
    SkillBindingId
);

define_id!(
    /// Identifies one append-only, evidence-labelled skill usage event
    /// (CPR-23, ADR-0085).
    SkillUsageEventId
);

define_id!(
    /// Identifies one immutable controlled-harness skill test run (CPR-23,
    /// ADR-0085).
    SkillTestRunId
);

define_id!(
    /// Identifies one stable trusted MCP server catalogue entry (CPR-25,
    /// ADR-0086).
    ToolServerId
);

define_id!(
    /// Identifies one immutable MCP server version (CPR-25, ADR-0086).
    ToolServerVersionId
);

define_id!(
    /// Identifies one immutable raw and normalised MCP capability snapshot
    /// (CPR-25, ADR-0086).
    CapabilitySnapshotId
);

define_id!(
    /// Identifies one revisioned exact-version project tool binding (CPR-25,
    /// ADR-0086).
    ToolBindingId
);

define_id!(
    /// Identifies one immutable read-only MCP connection test (CPR-25,
    /// ADR-0086).
    ToolTestRunId
);

define_id!(
    /// Identifies one stable governed runtime-configuration aggregate
    /// (CPR-30, ADR-0089).
    ConfigurationArtifactId
);

define_id!(
    /// Identifies one immutable governed runtime-configuration version
    /// (CPR-30, ADR-0089).
    ConfigurationVersionId
);

define_id!(
    /// Identifies one revisioned governed-scope configuration binding
    /// (CPR-30, ADR-0089).
    ConfigurationBindingId
);

define_id!(
    /// Identifies one context run — one act of composing context for a
    /// session (CPR-10, ADR-0076 decision 7).
    ContextRunId
);

define_id!(
    /// Identifies one visible candidate considered by a context run
    /// (CPR-20, ADR-0084).
    ContextCandidateId
);

define_id!(
    /// Identifies one immutable Knowledge-revision selection in a context run
    /// (CPR-20, ADR-0084).
    ContextSelectionId
);

define_id!(
    /// Identifies one feedback assertion about an exact context selection
    /// (CPR-20, ADR-0084).
    ContextFeedbackId
);

define_id!(
    /// Identifies one immutable snapshot of session evidence selected for
    /// candidate extraction (CPR-18, ADR-0083).
    CaptureBatchId
);

define_id!(
    /// Identifies one reviewable learning proposed by a capture batch
    /// (CPR-18, ADR-0083).
    CaptureCandidateId
);

define_id!(
    /// Identifies the durable intent/result record for one candidate decision
    /// (CPR-18, ADR-0083).
    CaptureCandidateDecisionId
);

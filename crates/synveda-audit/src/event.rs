//! The audit event vocabulary (AUD-1, ADR-0019).
//!
//! An [`AuditEvent`] is what a seam hands to [`crate::append`]; the chain
//! columns it becomes are described in `migrations/0011_audit_log.sql`.
//! Actions are a closed enum in-process so a typo cannot mint a new event
//! type silently, while the column stays open text so later features add
//! actions without schema churn.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Who performed the audited operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    /// How the actor was established.
    pub kind: ActorKind,
    /// The acting subject: the verified token subject for
    /// [`ActorKind::Subject`], the OS username (best effort) for
    /// [`ActorKind::BreakGlass`].
    pub subject: String,
}

impl Actor {
    /// An authenticated bearer — the verified token subject. Whether it is
    /// a user or a service identity is the identities table's knowledge,
    /// joined at query time (AUD-2), not duplicated per event.
    #[must_use]
    pub fn subject(subject: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Subject,
            subject: subject.into(),
        }
    }

    /// Store-level CLI access (ADR-0019 decision 7). Attribution is honest
    /// about being weaker: whoever holds the database credentials names
    /// themselves only as well as the OS does.
    #[must_use]
    pub fn break_glass(os_user: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::BreakGlass,
            subject: os_user.into(),
        }
    }
}

/// The two attribution strengths an event can carry (ADR-0019 decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    /// An authenticated bearer subject (user or service identity).
    Subject,
    /// Unauthenticated store-level access via the CLI break-glass.
    BreakGlass,
}

impl ActorKind {
    /// The stable column value; mirrors `audit_log_actor_kind_check`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ActorKind::Subject => "subject",
            ActorKind::BreakGlass => "break_glass",
        }
    }
}

/// The audited operation, as a stable dotted name. One event per audited
/// operation (ADR-0019 decision 4): mutations use their semantic action and
/// embed the authorizing decision in the payload; [`AuditAction::AuthzDecision`]
/// stands alone only for denials and allowed admin-plane reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditAction {
    /// A PDP decision with no semantic event of its own: every denial, and
    /// every allowed admin-plane read.
    AuthzDecision,
    /// A verified token named a tenant that refused resolution (suspended).
    /// Unauthenticated failures are not chain events (ADR-0019 decision 6).
    TenantResolutionDenied,
    /// A service identity's token was refused at the enforcement seam
    /// (lifetime unknown or over the cap, ADR-0018 decision 5).
    TokenRejected,
    /// The RLS backstop denied a statement (TEN-2, ADR-0009) — an internal
    /// isolation invariant broke; always accompanied by an error response.
    RlsBackstopTripped,
    /// JIT provisioning created an identity row (mapped, admin, or
    /// quarantined placement — not repeat logins).
    IdentityProvisioned,
    /// A hierarchy node was created.
    HierarchyNodeCreated,
    /// A hierarchy node was renamed and/or moved.
    HierarchyNodeUpdated,
    /// A hierarchy node was deleted.
    HierarchyNodeDeleted,
    /// The tenant's default policy pack was set.
    PolicyDefaultSet,
    /// The tenant's default policy pack was cleared.
    PolicyDefaultCleared,
    /// A policy pack was assigned to a hierarchy node.
    PolicyNodeAssigned,
    /// A node's policy pack assignment was removed.
    PolicyNodeUnassigned,
    /// A stored policy pack was applied (CLI break-glass; the reviewed
    /// product surface arrives with VedaFlow).
    PolicyPackApplied,
    /// A stored policy pack was removed (CLI break-glass).
    PolicyPackCleared,
    /// A role binding was created (admin API, JIT admin-group first
    /// establishment, or break-glass).
    RoleBound,
    /// A role binding was removed.
    RoleUnbound,
    /// A service identity was registered at an anchor node.
    ServiceIdentityRegistered,
    /// A service identity was revoked (row and personal leaf deleted).
    ServiceIdentityRevoked,
    /// An observe batch was admitted to the ingestion buffer — one event
    /// per batch, counts and id range in the payload, never one row per
    /// event (MEM-1, ADR-0020 decision 5; ADR-0019 decision 4). Since
    /// MEM-2 the payload also carries quarantined/denied counts and the
    /// finding rule summary — never matched text (ADR-0021).
    MemoryObserved,
    /// A reviewer released a quarantined observe event into the pipeline
    /// (MEM-2, ADR-0021 decision 7).
    QuarantineReleased,
    /// A reviewer rejected a quarantined observe event; its staging row
    /// stays provenance-only, forever signal-less.
    QuarantineRejected,
    /// A tenant was admitted (CLI break-glass; TEN-5 owns the product
    /// lifecycle surface).
    TenantCreated,
}

impl AuditAction {
    /// The stable dotted name stored in the `action` column. Renaming an
    /// existing value is a breaking change to every consumer of the log.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AuditAction::AuthzDecision => "authz.decision",
            AuditAction::TenantResolutionDenied => "tenant.resolution.denied",
            AuditAction::TokenRejected => "auth.token.rejected",
            AuditAction::RlsBackstopTripped => "store.rls.denied",
            AuditAction::IdentityProvisioned => "identity.provisioned",
            AuditAction::HierarchyNodeCreated => "hierarchy.node.created",
            AuditAction::HierarchyNodeUpdated => "hierarchy.node.updated",
            AuditAction::HierarchyNodeDeleted => "hierarchy.node.deleted",
            AuditAction::PolicyDefaultSet => "policy.default.set",
            AuditAction::PolicyDefaultCleared => "policy.default.cleared",
            AuditAction::PolicyNodeAssigned => "policy.node.assigned",
            AuditAction::PolicyNodeUnassigned => "policy.node.unassigned",
            AuditAction::PolicyPackApplied => "policy.pack.applied",
            AuditAction::PolicyPackCleared => "policy.pack.cleared",
            AuditAction::RoleBound => "role.bound",
            AuditAction::RoleUnbound => "role.unbound",
            AuditAction::ServiceIdentityRegistered => "service_identity.registered",
            AuditAction::ServiceIdentityRevoked => "service_identity.revoked",
            AuditAction::MemoryObserved => "memory.observed",
            AuditAction::QuarantineReleased => "memory.quarantine.released",
            AuditAction::QuarantineRejected => "memory.quarantine.rejected",
            AuditAction::TenantCreated => "tenant.created",
        }
    }
}

/// How the audited operation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The PDP allowed (decision events).
    Allow,
    /// The PDP denied (decision events).
    Deny,
    /// The operation completed (semantic events).
    Success,
    /// The operation failed after being allowed (e.g. an RLS trip).
    Failure,
}

impl Outcome {
    /// The stable column value; mirrors `audit_log_outcome_check`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Outcome::Allow => "allow",
            Outcome::Deny => "deny",
            Outcome::Success => "success",
            Outcome::Failure => "failure",
        }
    }
}

/// One audit event, ready to append to its tenant's chain.
///
/// `occurred_at` is truncated to whole microseconds by [`crate::append`]
/// before hashing and storage, so the timestamptz round-trip is exact
/// (ADR-0019 decision 2). The payload must contain no non-integer numbers;
/// append rejects violations rather than hash a value jsonb could reshape.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    /// When the operation happened (append time is the honest choice).
    pub occurred_at: DateTime<Utc>,
    /// Who did it.
    pub actor: Actor,
    /// What they did.
    pub action: AuditAction,
    /// What they did it to, e.g. `scope:0198…`, `tenant:0198…`,
    /// `binding:alice@scope:0198…`. Freeform but consistent per action.
    pub resource: String,
    /// How it ended.
    pub outcome: Outcome,
    /// Event-specific detail: the authorizing decision's context
    /// (pack name@version, determining policies, roles), pre/post images,
    /// denial reasons. `{}` when there is nothing to add.
    pub payload: Value,
    /// The OTel trace id live at emission, when there was one.
    pub trace_id: Option<String>,
}

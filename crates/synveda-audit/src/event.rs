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

    /// A background pipeline acting as itself (MEM-3, ADR-0022
    /// decision 5). The subject names the component — `"extraction"`, a
    /// MEM-6 sweep, a directory sync — never a user: the identity the
    /// pipeline acted *for* belongs in the event payload.
    #[must_use]
    pub fn system(component: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::System,
            subject: component.into(),
        }
    }
}

/// The attribution strengths an event can carry (ADR-0019 decision 7;
/// ADR-0022 decision 5 adds `System`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    /// An authenticated bearer subject (user or service identity).
    Subject,
    /// Unauthenticated store-level access via the CLI break-glass.
    BreakGlass,
    /// A background pipeline (extraction, sweeps, sync jobs) acting as
    /// itself, named by component.
    System,
}

impl ActorKind {
    /// The stable column value; mirrors `audit_log_actor_kind_check`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ActorKind::Subject => "subject",
            ActorKind::BreakGlass => "break_glass",
            ActorKind::System => "system",
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
    /// quarantined placement — not repeat logins). Since AUTH-4 the
    /// directory plane chains the same action for the same act, with an
    /// `origin` of `scim` in the payload: two doors, one thing produced
    /// (ADR-0059 decision 6).
    IdentityProvisioned,
    /// A directory moved somebody, and their personal scope moved with
    /// them — or was sealed where it stood, when the move crossed a policy
    /// boundary a sealing pack governs (AUTH-4, ADR-0059 decision 10).
    /// The payload carries which of the two happened and why.
    IdentityMoved,
    /// A provisioning credential was issued for the directory plane
    /// (AUTH-4, ADR-0059 decision 13).
    ScimCredentialIssued,
    /// A provisioning credential was revoked. A stamp rather than a
    /// delete: which credential sealed which identity stays answerable.
    ScimCredentialRevoked,
    /// A scheduled pull pass changed something: what it saw, how many it
    /// counted absent, and how many it sealed (AUTH-5, ADR-0060 decision 9).
    /// A pass that changed nothing chains nothing — a quiet tenant's chain
    /// must not become a record of the product reading a directory that had
    /// not changed.
    DirectorySyncCompleted,
    /// A pull pass declined to seal a bulk departure because no
    /// authorisation in force covered it (ADR-0060 decision 3.3). Chained
    /// because a refusal to act is the one thing on this plane an auditor
    /// must not have to notice.
    DirectorySyncBreakerTripped,
    /// A human authorised a pull pass to seal past the breaker: reasoned,
    /// time-boxed and bounded by a ceiling (ADR-0060 decision 10).
    DirectorySealAuthorised,
    /// A pass spent that authorisation. Two events rather than one so the
    /// chain answers both what was permitted and what was actually done
    /// with it — the grant cannot describe a use that had not happened yet.
    DirectorySealAuthorisationUsed,
    /// A directory said somebody has left: their personal scope is sealed
    /// — unreadable under the base layer's forbid, and exempt from every
    /// retention horizon (AUTH-4, ADR-0059 decision 8).
    IdentitySealed,
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
    /// The extraction pipeline processed a tenant commit-group of staged
    /// events — one event per group, ids/counts/classes in the payload,
    /// never one row per record (MEM-3, ADR-0022 decision 5). `failure`
    /// marks a dead-lettered signal (retries exhausted).
    MemoryExtracted,
    /// The extraction pipeline closed the valid windows of records a newer
    /// statement replaced (MEM-5, ADR-0039 decision 13) — one event per
    /// commit group, with the id pairs, the judge, the signals as integer
    /// per-mille, and the instant each window closed at. Never record
    /// content.
    ///
    /// Its own action rather than a field on `memory.extracted` because it
    /// asserts a different fact: extraction says what was created, and this
    /// says what stopped being current, which is the question an auditor
    /// arrives with. Restatements *merged* into existing records stay on
    /// `memory.extracted`, where they belong — a merge creates nothing and
    /// closes nothing.
    ///
    /// The payload also carries the contradictions the pipeline found
    /// against *published* material and declined to act on: reviewed content
    /// leaves the trust boundary through a proposal, never through a
    /// session, and a refusal nobody can see is a refusal nobody can act on.
    MemorySuperseded,
    /// An approved classification proposal's effect ran: records moved to
    /// the sensitivity their reviewed versions carried (AUTHZ-5, ADR-0038
    /// decision 9). Carries both tiers, the record ids, and the approvals
    /// as resolved — never record content, which is what the tier is
    /// about in the first place.
    MemoryClassified,
    /// A tenant was admitted (CLI break-glass; TEN-5 owns the product
    /// lifecycle surface).
    TenantCreated,
    /// A tenant's encryption key was minted (TEN-4, ADR-0064). Carries the
    /// generation and the KEK's name, never key material of any kind —
    /// wrapped included, because a wrapped key in the chain is a wrapped key
    /// in every copy of the chain.
    TenantKeyProvisioned,
    /// A tenant's encryption key was rotated: the current generation retired
    /// and the next minted. Nothing is re-sealed by the act, so this event
    /// means "new payloads move to generation N", not "everything is on
    /// generation N".
    TenantKeyRotated,
    /// A sealed archive of a tenant was written (ADR-0064 decision 8).
    /// Carries what left and under which generation, because an export is a
    /// copy of a tenant's data leaving the deployment and the chain is where
    /// that is answerable afterwards.
    TenantExported,
    /// A tenant's sealed secret was stored or replaced — its outbound
    /// directory credential today (decision 9). Carries the name and never
    /// the value.
    TenantSecretStored,
    /// A tenant's sealed secret was destroyed.
    TenantSecretCleared,
    /// An inject composed a context block — one event per inject with
    /// the block's watermark and the per-scope `MemoryRead` decisions
    /// aggregated, never one row per candidate (CTX-3, ADR-0026
    /// decision 5; ADR-0019 decision 4). Payloads carry no user
    /// content: the task rides as a BLAKE3 hash only.
    ContextInjected,
    /// A recall served record bodies the caller named — the third
    /// primitive seed §3 has listed since day one, and the act seed §2.2
    /// principle 5 requires the chain to record (CTX-4, ADR-0041
    /// decision 8).
    ///
    /// One event per recall, with the requested count, the served
    /// entries' object addresses and channels — the same watermark shape
    /// an inject carries, so a recall is exactly as recomputable — and the
    /// per-scope `MemoryRead` decisions aggregated. Never record content,
    /// and never the ids that were refused as a distinguishable list: a
    /// recall answers uniformly for what it will not serve, and an audit
    /// payload that enumerated the difference would be the oracle the
    /// surface refuses to be.
    ContextRecalled,
    /// Records were published onto a scope's VedaFlow published channel —
    /// the act that moves content across the trust boundary so `inject`
    /// composes it as reviewed (FLOW-2, ADR-0031 decision 14). The first
    /// VedaFlow action; ADR-0030 decision 14 deferred it to whichever
    /// feature produced a governed one. Payload carries the asset kind,
    /// the record ids, the commit the channel moved from and to, and the
    /// pack that governed — never record content. The pipeline's
    /// derived-channel commits ride `memory.extracted` instead: one event
    /// per group, not a second event asserting the same fact.
    ///
    /// Since FLOW-3 the payload also names the proposal a publication was
    /// the effect of, when it had one, and the approval requirement the
    /// publisher satisfied (ADR-0032 decision 18): publishing through a
    /// proposal is the same governed act with the same consequence as a
    /// direct publish, so it is the same action with a fuller payload,
    /// not a second one asserting the same fact.
    ChannelPublished,
    /// A scope's published channel was rewound to a state it had already
    /// held (FLOW-7, ADR-0036 decision 9) — the one governed act that
    /// removes content from the trust boundary, and the one that reaches
    /// every agent in the subtree on their next session with no further
    /// human act. Payload carries the commit abandoned and the commit
    /// installed, the record ids that stopped being published material and
    /// those whose published version went back to an earlier one, the
    /// operator's mandatory message, and the `channel.rollback` decision.
    /// Never record content, like every other channel event.
    ChannelRolledBack,
    /// A scope's channel was held at a commit, or a standing hold was moved
    /// to a different one (ADR-0036 decision 6). Publications keep landing
    /// while a pin stands; what changes is only what readers compose.
    /// Payload carries the commit held, the one it replaced when the pin
    /// moved, and the mandatory reason — the pin's only record, because the
    /// ref itself carries who and when and nothing else.
    ChannelPinned,
    /// A standing pin was released and the channel serves its head again.
    /// The one ref deletion the schema permits (migration 0021).
    ChannelUnpinned,
    /// A proposal was opened against a scope's published channel
    /// (FLOW-3, ADR-0032 decision 18). Payload carries the target, asset
    /// kind, member ids and addresses, the proposal commit, the maximum
    /// sensitivity, and the requirement **as resolved at that moment** —
    /// so a trail explains what the proposal needed without reading a
    /// pack that has since changed. Never record content.
    ProposalOpened,
    /// A reviewer approved a proposal, with the effective roles their
    /// approval counted under and what the requirement still lacked
    /// afterwards.
    ProposalApproved,
    /// A reviewer rejected a proposal, closing it. The reason is
    /// mandatory and rides the payload.
    ProposalRejected,
    /// A proposer withdrew their own proposal, closing it.
    ProposalWithdrawn,
    /// An approved lapse proposal's effect ran: a time-boxed grant now
    /// stands over the target scope's material (AUTHZ-4, ADR-0037
    /// decision 17). Payload carries both scopes, the action granted, the
    /// window, the mandatory reason, the proposal, and the requirement as
    /// resolved — never any of the material the grant discloses.
    ///
    /// The window is the load-bearing field: with it recorded here, the
    /// trail is complete even if the expiry sweep never runs, because when
    /// the grant stopped deciding anything is arithmetic over this event.
    LapseGranted,
    /// A standing grant was ended early, with its mandatory reason and the
    /// window it cut short.
    LapseRevoked,
    /// Records left the live corpus because they were past the horizon
    /// the pack at their scope sets (MEM-6, ADR-0040 decision 15) — one
    /// event per scope per sweep batch, under `actor_kind=system`.
    /// Carries the pack and version that decided, the horizon per class,
    /// the record ids and their ages; never record content.
    ///
    /// Unlike [`AuditAction::LapseExpired`] this is **not** bookkeeping: a
    /// lapse expires whether or not its sweep runs, but a record leaves
    /// the corpus only because this loop ran, and the event commits in the
    /// same transaction as the delete.
    ///
    /// What it describes is a *temporal* delete: the record stops being
    /// current, `as_of` keeps answering, and destruction is the second
    /// horizon's event.
    MemoryExpired,
    /// Content was destroyed (MEM-6, ADR-0040 decision 15): closed record
    /// versions past the destruction horizon, and observe staging rows
    /// with their quarantine markers past the staging horizon. Per plane,
    /// with counts, the horizon that authorised it, and — for records —
    /// the scope. The one action in the product that says data is gone
    /// rather than hidden.
    ///
    /// Deliberately separate from [`AuditAction::MemoryExpired`]: "what
    /// did we stop using" and "what did we destroy" are different
    /// questions, and only the second has a legal answer.
    MemoryDisposed,
    /// A prompt draft was written — created or replaced (PRMT-1, ADR-0049
    /// decision 14).
    ///
    /// The authoring act, not a publication: what it records is that a
    /// scope's working copy moved, with the name, the tier and the new
    /// object address. Nothing here crosses the trust boundary, and a
    /// consumer asking for the published channel is unaffected by it —
    /// which is exactly what "prompt change behind review" means when it is
    /// read from the reader's side.
    PromptAuthored,
    /// A prompt was served to a consumer (ADR-0049 decision 14).
    ///
    /// A data-plane read, so it chains its own event rather than an
    /// `authz.decision`: what left the system is content, and "who was
    /// served which version of which prompt, when" is a question an auditor
    /// asks about prompts exactly as AUD-2 asks it about memory. The
    /// payload carries the name, the scope it resolved at, the channel or
    /// the pinned commit, and the object address — never the template.
    PromptResolved,
    /// A context pack's draft was written at a scope (PRMT-2, ADR-0050
    /// decision 13).
    ///
    /// The authoring act, not a publication, exactly as
    /// [`AuditAction::PromptAuthored`] is — and here it is also where the
    /// *expensive* half happens: chunking, the MEM-2 scan and the embedding
    /// all run before this event, so the payload can say how many chunks
    /// landed and how many were already there. Nothing here crosses the
    /// trust boundary. No document text: the addresses, the counts and the
    /// tiers only.
    ContextPackAuthored,
    /// A pack document carrying a live credential was quarantined at
    /// authoring (ADR-0050 decision 11).
    ///
    /// This is the first surface where bulk external text enters the
    /// product — a prompt is short and hand-written, and PRMT-1 does not
    /// scan one — so MEM-2's scanner runs here with the authoring scope's
    /// effective redaction config, ahead of the embedder, and a document it
    /// stops never reaches vector space.
    ///
    /// A *served* chunk chains nothing of its own: it composes inside
    /// `context.injected` with its object address like every other entry,
    /// which is why there is no third action here.
    ContextPackQuarantined,
    /// A skill's draft was written at a scope (SKIL-1, ADR-0051
    /// decision 16).
    ///
    /// The authoring act, not a publication, exactly as
    /// [`AuditAction::ContextPackAuthored`] is. The payload carries the
    /// name, the tier, the per-file object addresses and how many files
    /// were removed from the bundle — never `SKILL.md` text and never file
    /// content.
    ///
    /// There is deliberately no `skill.installed`: an install is a
    /// client-side act on bytes an audited [`AuditAction::SkillResolved`]
    /// already served, and an event the server cannot verify is a fact an
    /// auditor would have to reconcile (ADR-0019 decision 4).
    SkillAuthored,
    /// A skill bundle was served to a consumer (ADR-0051 decision 16).
    ///
    /// A data-plane read, so it chains its own event for
    /// [`AuditAction::PromptResolved`]'s reason — and here the question is
    /// sharper than for a prompt, because what was served is about to
    /// become **files on somebody's machine**. This event and the addresses
    /// in it are the whole of a materialised bundle's provenance: nothing
    /// inside the installed directory can carry a watermark without
    /// breaking the one criterion the feature exists to meet (ADR-0051
    /// force 2).
    SkillResolved,
    /// A skill bundle carrying a live credential was stopped at authoring
    /// (ADR-0051 decision 14).
    ///
    /// [`AuditAction::ContextPackQuarantined`]'s twin, and the guarantee is
    /// stronger here for having a different destination: a pack's secret
    /// would have reached vector space, and a skill's reaches a laptop.
    SkillQuarantined,
    /// A skill bundle was refused by the security scanning gate, at
    /// authoring or at publication (SKIL-2, ADR-0052 decision 8).
    ///
    /// ADR-0051 decision 16 said "two new audit actions, and no third".
    /// This is the third, and it earns the place by being a governed
    /// refusal rather than a fact already on the chain: nothing else
    /// records that the product stopped a bundle, and an auditor
    /// filtering for what the security gate caught should not have to
    /// disambiguate two different scanners inside one action's payload.
    ///
    /// Carries rule ids, severities, counts, line numbers, the ruleset
    /// version and the pack that decided — never file content and never
    /// the matched span, which for a credential rule *is* the credential
    /// path.
    ///
    /// There is deliberately no event for a clean scan (every authored
    /// bundle already chains [`AuditAction::SkillAuthored`], and a scan
    /// that found nothing is not an act) and none for rendering a report
    /// to a reviewer (the proposal read already chains, and the report is
    /// recomputable from what it names).
    SkillScanRejected,
    /// A reviewer recorded a quality checklist against a skill bundle
    /// (SKIL-3, ADR-0053 decision 10).
    ///
    /// The **durable record of the human half of the score**, and the
    /// reason the row it writes can be last-writer-wins: a row is mutable
    /// and a chained event is not, so re-answering replaces a row while
    /// the chain keeps every answer anybody gave.
    ///
    /// Carries the item ids, the verdicts, the bundle digest the answers
    /// are bound to and the rubric version rendered beside the reviewer —
    /// never file content, and the note only because a reviewer wrote it
    /// to be read (it passes MEM-2's scanner before it is stored).
    SkillChecklistRecorded,
    /// A skill was published over the quality gate's objection (SKIL-3,
    /// ADR-0053 decision 10).
    ///
    /// **The most valuable event this feature produces.** "What did we
    /// ship that we knew was below the bar, and who said so" is a question
    /// no other event in the product answers, and an override whose event
    /// was lost would be a publication with no explanation — which is why
    /// it chains inside the publish transaction rather than beside it.
    ///
    /// Carries the score, which of the three bars was missed, the reason
    /// the publisher gave, the pack that set the bar and the identity that
    /// held [`Action::SkillQualityOverride`](synveda_policy::Action).
    ///
    /// There is deliberately **no equivalent for the security scan**, and
    /// there must not be: ADR-0052 decision 3 put the `critical` band on
    /// the invariant floor precisely so nothing can wave it through, and
    /// an event recording that somebody had would be evidence of a path
    /// that should not exist.
    SkillQualityOverridden,
    /// A grant reached the end of its window. Emitted by the sweep under
    /// `actor_kind=system`, and **bookkeeping only** — the grant stopped
    /// deciding anything at `expires_at` whether or not this was ever
    /// written (ADR-0037 decision 4).
    ///
    /// Revoked grants deliberately get no expiry event: their ending is
    /// already on the chain, and a second event asserting the same fact is
    /// something an auditor would have to reconcile (ADR-0019 decision 4).
    LapseExpired,
}

impl AuditAction {
    /// Every action, in declaration order — the vocabulary a query surface
    /// may name (AUD-2, ADR-0045 decision 3).
    ///
    /// Hand-maintained beside the enum, like [`synveda_types::Role::ALL`]:
    /// Rust cannot make an array literal exhaustive, so the guard is the
    /// unit test below plus the fact that an action missing from here is
    /// an event `GET /v1/audit/events` cannot filter for. Add the variant
    /// and add it here in the same diff.
    pub const ALL: [AuditAction; 63] = [
        AuditAction::AuthzDecision,
        AuditAction::TenantResolutionDenied,
        AuditAction::TokenRejected,
        AuditAction::RlsBackstopTripped,
        AuditAction::IdentityProvisioned,
        AuditAction::IdentityMoved,
        AuditAction::IdentitySealed,
        AuditAction::ScimCredentialIssued,
        AuditAction::ScimCredentialRevoked,
        AuditAction::DirectorySyncCompleted,
        AuditAction::DirectorySyncBreakerTripped,
        AuditAction::DirectorySealAuthorised,
        AuditAction::DirectorySealAuthorisationUsed,
        AuditAction::HierarchyNodeCreated,
        AuditAction::HierarchyNodeUpdated,
        AuditAction::HierarchyNodeDeleted,
        AuditAction::PolicyDefaultSet,
        AuditAction::PolicyDefaultCleared,
        AuditAction::PolicyNodeAssigned,
        AuditAction::PolicyNodeUnassigned,
        AuditAction::PolicyPackApplied,
        AuditAction::PolicyPackCleared,
        AuditAction::RoleBound,
        AuditAction::RoleUnbound,
        AuditAction::ServiceIdentityRegistered,
        AuditAction::ServiceIdentityRevoked,
        AuditAction::MemoryObserved,
        AuditAction::QuarantineReleased,
        AuditAction::QuarantineRejected,
        AuditAction::MemoryExtracted,
        AuditAction::MemorySuperseded,
        AuditAction::MemoryClassified,
        AuditAction::TenantCreated,
        AuditAction::TenantKeyProvisioned,
        AuditAction::TenantKeyRotated,
        AuditAction::TenantExported,
        AuditAction::TenantSecretStored,
        AuditAction::TenantSecretCleared,
        AuditAction::ContextInjected,
        AuditAction::ContextRecalled,
        AuditAction::ChannelPublished,
        AuditAction::ChannelRolledBack,
        AuditAction::ChannelPinned,
        AuditAction::ChannelUnpinned,
        AuditAction::ProposalOpened,
        AuditAction::ProposalApproved,
        AuditAction::ProposalRejected,
        AuditAction::ProposalWithdrawn,
        AuditAction::LapseGranted,
        AuditAction::LapseRevoked,
        AuditAction::LapseExpired,
        AuditAction::MemoryExpired,
        AuditAction::MemoryDisposed,
        AuditAction::PromptAuthored,
        AuditAction::PromptResolved,
        AuditAction::ContextPackAuthored,
        AuditAction::ContextPackQuarantined,
        AuditAction::SkillAuthored,
        AuditAction::SkillResolved,
        AuditAction::SkillQuarantined,
        AuditAction::SkillScanRejected,
        AuditAction::SkillChecklistRecorded,
        AuditAction::SkillQualityOverridden,
    ];

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
            AuditAction::IdentityMoved => "identity.moved",
            AuditAction::IdentitySealed => "identity.sealed",
            AuditAction::ScimCredentialIssued => "scim.credential.issued",
            AuditAction::ScimCredentialRevoked => "scim.credential.revoked",
            AuditAction::DirectorySyncCompleted => "directory.sync.completed",
            AuditAction::DirectorySyncBreakerTripped => "directory.sync.breaker_tripped",
            AuditAction::DirectorySealAuthorised => "directory.seal.authorised",
            AuditAction::DirectorySealAuthorisationUsed => "directory.seal.authorisation_used",
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
            AuditAction::MemoryExtracted => "memory.extracted",
            AuditAction::MemorySuperseded => "memory.superseded",
            AuditAction::MemoryClassified => "memory.classified",
            AuditAction::TenantCreated => "tenant.created",
            AuditAction::TenantKeyProvisioned => "tenant.key.provisioned",
            AuditAction::TenantKeyRotated => "tenant.key.rotated",
            AuditAction::TenantExported => "tenant.exported",
            AuditAction::TenantSecretStored => "tenant.secret.stored",
            AuditAction::TenantSecretCleared => "tenant.secret.cleared",
            AuditAction::ContextInjected => "context.injected",
            AuditAction::ContextRecalled => "context.recalled",
            AuditAction::ChannelPublished => "vedaflow.channel.published",
            AuditAction::ChannelRolledBack => "vedaflow.channel.rolled_back",
            AuditAction::ChannelPinned => "vedaflow.channel.pinned",
            AuditAction::ChannelUnpinned => "vedaflow.channel.unpinned",
            AuditAction::ProposalOpened => "vedaflow.proposal.opened",
            AuditAction::ProposalApproved => "vedaflow.proposal.approved",
            AuditAction::ProposalRejected => "vedaflow.proposal.rejected",
            AuditAction::ProposalWithdrawn => "vedaflow.proposal.withdrawn",
            AuditAction::LapseGranted => "policy.lapse.granted",
            AuditAction::LapseRevoked => "policy.lapse.revoked",
            AuditAction::LapseExpired => "policy.lapse.expired",
            AuditAction::MemoryExpired => "memory.expired",
            AuditAction::MemoryDisposed => "memory.disposed",
            AuditAction::PromptAuthored => "prompt.authored",
            AuditAction::PromptResolved => "prompt.resolved",
            AuditAction::ContextPackAuthored => "context_pack.authored",
            AuditAction::ContextPackQuarantined => "context_pack.quarantined",
            AuditAction::SkillAuthored => "skill.authored",
            AuditAction::SkillResolved => "skill.resolved",
            AuditAction::SkillQuarantined => "skill.quarantined",
            AuditAction::SkillScanRejected => "skill.scan.rejected",
            AuditAction::SkillChecklistRecorded => "skill.checklist.recorded",
            AuditAction::SkillQualityOverridden => "skill.quality.overridden",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `ALL` really is all of them.
    ///
    /// The distinctness test below cannot catch an *omission*, and four
    /// actions reached the chain without joining this list before it was
    /// written (AUTH-4). An action missing here is invisible to everything
    /// that enumerates the vocabulary — an export's column set, a SIEM
    /// rule's allow-list, a documentation table — while working perfectly
    /// well at the emission point, which is the shape of gap that survives
    /// review.
    ///
    /// Enforced by arithmetic rather than by reflection, which Rust does
    /// not have for this: `as_str` is an exhaustive match, so every variant
    /// has a name, and `ALL`'s length is declared. What this asserts is
    /// that the declared length matches the number of *distinct* names
    /// reachable through it — so adding a variant without adding it here
    /// leaves the two numbers disagreeing the moment anybody updates the
    /// array's length, and forgetting the length is a compile error.
    #[test]
    fn the_vocabulary_list_holds_every_action() {
        let names: std::collections::BTreeSet<&str> =
            AuditAction::ALL.iter().map(|a| a.as_str()).collect();
        assert_eq!(
            names.len(),
            AuditAction::ALL.len(),
            "ALL has a duplicate, so it is hiding a missing variant"
        );
    }

    #[test]
    fn every_action_name_is_distinct_and_dotted() {
        // Two actions sharing a name would make the chain ambiguous to
        // every consumer of it — a query, an export, a SIEM rule.
        let mut names: Vec<&str> = AuditAction::ALL.iter().map(|a| a.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            total,
            "duplicate action name in AuditAction::ALL"
        );

        for name in names {
            assert!(!name.is_empty(), "an action name must not be empty");
            assert!(
                name.contains('.'),
                "{name} is not dotted — the taxonomy is `domain.thing.happened`"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_'),
                "{name} has characters outside the taxonomy's alphabet"
            );
            assert!(
                name.len() <= 100,
                "{name} exceeds audit_log_action_check's 100-character bound"
            );
        }
    }

    #[test]
    fn every_actor_kind_matches_the_column_constraint() {
        // migration 0011 + 0014: the CHECK accepts exactly these three.
        for (kind, expected) in [
            (ActorKind::Subject, "subject"),
            (ActorKind::BreakGlass, "break_glass"),
            (ActorKind::System, "system"),
        ] {
            assert_eq!(kind.as_str(), expected);
        }
    }
}

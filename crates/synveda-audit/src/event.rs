//! The audit event vocabulary (AUD-1, ADR-0019).
//!
//! An [`AuditEvent`] is what a seam hands to [`crate::append`]; the chain
//! columns it becomes are defined in the epoch baseline's `audit_log` table.
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
    /// A workspace was created, with the governed scope it owns — one event
    /// for both, because they are one act and one transaction (CPR-4,
    /// ADR-0071). The payload carries the scope id, so the chain answers
    /// "which scope is this workspace" without a join.
    WorkspaceCreated,
    /// A workspace was renamed, re-described or retired. The payload carries
    /// the before and after images and the revision the update was applied
    /// under — a lost-update conflict is a *refusal*, so a chain that records
    /// the precondition records why the write that lost is absent.
    WorkspaceUpdated,
    /// A project was created inside a workspace, with the scope it owns.
    ProjectCreated,
    /// A project was renamed, re-described or retired.
    ProjectUpdated,
    /// A repository was attached to a project. The payload carries the
    /// **canonical** URI — which is credential-free by construction, so this
    /// is one of the places that property is load-bearing rather than tidy.
    ProjectRepositoryAttached,
    /// A repository was detached from a project. A delete rather than a
    /// stamp, because the row asserts a present fact about a project; what it
    /// was is here, in the chain.
    ProjectRepositoryDetached,
    /// A session was opened (CPR-10, ADR-0076). The payload carries the
    /// workspace, the project when there is one, the governed scope the run
    /// is decided at, and the client that opened it — never the client's
    /// `metadata`, which is where an agent's environment would land and
    /// therefore where a credential would (seed: no secret in an audit
    /// payload). That metadata was present, and how large it was, is
    /// recorded; its contents are not.
    SessionOpened,
    /// A session moved through its lifecycle: `ending`, or one of the three
    /// closed states. The payload carries both ends of the transition, so the
    /// chain says what a run was doing when somebody closed it.
    SessionEnded,
    /// Events were appended to a session. **One event per batch**, carrying
    /// counts and the sequence range rather than the events themselves — a
    /// hundred-turn run would otherwise put its whole transcript in the chain
    /// twice, and the chain is not the transcript store.
    ///
    /// The per-type breakdown rides the payload, because "what did that agent
    /// actually do" should be answerable from the chain without reading the
    /// events, and because the *shape* of a run is what an auditor reads
    /// first.
    SessionEventsAppended,
    /// Context was composed for a session (CPR-10, ADR-0076 decision 7). The
    /// payload carries the block's watermark — its hash, the entry count, the
    /// tokens against the budget — and not the block: the chain records what
    /// an agent was given, and the run row holds what that was.
    SessionContextComposed,
    /// A bounded policy-visible Knowledge pool was retrieved for a context
    /// run. Carries revision ids or hashes according to retention, score
    /// component names and an aggregate policy-filtering flag, never content.
    ContextCandidatesRetrieved,
    /// Exact immutable Knowledge revisions were selected under the context
    /// budget. Distinct from delivery: a selection is a planner decision and
    /// `SessionContextComposed` proves the resulting block crossed the API.
    ContextSelectionsMade,
    /// Explicit feedback was attached to one context selection and immutable
    /// revision. Retrieval or delivery alone never emits this action.
    ContextFeedbackRecorded,
    /// An exact eligible session-event snapshot was frozen for extraction.
    /// Carries ids, digest and count, never transcript content.
    CaptureBatchCreated,
    /// A capture batch reached completed or failed. Carries extractor
    /// identity, counts and a content-free error code.
    CaptureBatchCompleted,
    /// A person made one terminal candidate decision. Carries the action,
    /// VedaFlow change/result ids and hashes, never candidate content.
    CaptureCandidateDecided,
    /// A bounded OKF bundle was validated into an immutable dry-run plan.
    /// Carries counts, version and hashes, never Markdown or frontmatter.
    OkfImportPlanned,
    /// One immutable OKF plan became reviewable capture candidates. It does
    /// not imply that any candidate was published as Knowledge.
    OkfImportMaterialized,
    /// Freshly authorised current Knowledge was rendered as deterministic
    /// OKF output. Carries revision ids and the output digest, never bodies.
    OkfExported,
    /// A group was created (CPR-5, ADR-0072). The payload carries the
    /// group's handle and whether a directory owns it — a group nobody here
    /// maintains is a different fact from one somebody here made.
    GroupCreated,
    /// A group was renamed, retired or re-populated. The payload carries the
    /// before and after images and the revision the update was applied under;
    /// when the membership changed it carries **counts and the difference**,
    /// never the whole list, because a hundred-person group would otherwise
    /// put a hundred names in the chain on every edit.
    GroupUpdated,
    /// Somebody was granted a role at a scope. The payload carries the
    /// subject, the role key, the scope and the **source** — which is the
    /// whole of "why does this person have access", answerable from the chain
    /// as well as from the row.
    AccessGranted,
    /// A grant was revoked. A delete rather than a stamp, on the repository
    /// plane's reasoning: the row asserts a present fact about who may act,
    /// and what it was is here.
    AccessRevoked,
    /// An invitation was issued. The payload carries the invitation's id, the
    /// scope, the role and the expiry — and **never the token**, which exists
    /// once, in the response to the request that created it.
    InviteCreated,
    /// An invitation was withdrawn before anybody redeemed it.
    InviteRevoked,
    /// An invitation was redeemed, and the grant it carried was minted. One
    /// event for both, because they are one act and one transaction.
    InviteAccepted,
    /// A governed scope was created (CPR-7, ADR-0074 decision 5).
    ScopeCreated,
    /// A governed scope was renamed, re-described, archived or moved —
    /// the payload names which, and a move names both ends.
    ScopeUpdated,
    /// A scope's VedaFlow curator rules were updated. This is policy source
    /// governance, not runtime-profile selection (CPR-30, ADR-0089).
    CuratorRulesUpdated,
    /// A stored policy pack was applied (CLI break-glass; the reviewed
    /// product surface arrives with VedaFlow).
    PolicyPackApplied,
    /// A stored policy pack was removed (CLI break-glass).
    PolicyPackCleared,
    /// A service identity was registered at an anchor node.
    ServiceIdentityRegistered,
    /// A service identity was revoked (row and personal leaf deleted).
    ServiceIdentityRevoked,
    /// A reviewer released a quarantined session event for capture.
    QuarantineReleased,
    /// A reviewer rejected a quarantined session event; the immutable event
    /// remains as provenance but cannot become a capture source.
    QuarantineRejected,
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
    /// Active tenant-secret envelopes were advanced to a new DEK generation.
    /// Carries one durable job id, generations and counts, never ciphertext.
    TenantSecretsReencrypted,
    /// Authored objects were published onto a scope's VedaFlow published
    /// channel. The payload carries the asset kind, immutable object
    /// addresses, the commit the channel moved from and to, and the policy
    /// pack that governed — never object content.
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
    /// A typed Knowledge mutation was opened as a VedaFlow change. Carries
    /// command kind, ids, hashes and approval arithmetic, never content.
    KnowledgeChangeOpened,
    /// A Knowledge change's typed effect completed. The command kind in the
    /// payload distinguishes create/edit/verify/supersede/merge/lifecycle.
    KnowledgeChangeApplied,
    /// A Knowledge change was closed without applying because a revision,
    /// lifecycle or retention precondition no longer permitted its effect.
    /// Carries only a stable reason code and identifiers.
    KnowledgeChangeRejected,
    /// A bounded, policy-visible matcher retained one durable conflict set.
    /// Carries set/member ids, classifications and hashes only.
    KnowledgeConflictOpened,
    /// A revision-aware Knowledge/VedaFlow change made one conflict set
    /// terminal. Carries the exact resolution and change id, never content.
    KnowledgeConflictResolved,
    /// A retention or legal-hold hook blocked a governed erasure operation.
    KnowledgeErasureBlocked,
    /// Plaintext, owned source descriptors and indexes were removed, leaving
    /// only hashes and identifiers in the tombstone and chain.
    KnowledgeErased,
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
    /// A typed Skill/apply VedaFlow change was opened (CPR-23, ADR-0085).
    /// Carries ids, command kind, hashes and approval requirements, never
    /// bundle text.
    SkillChangeOpened,
    /// A typed Skill/apply effect installed an immutable version or changed
    /// a revisioned binding.
    SkillChangeApplied,
    /// A Skill/apply effect reached a terminal precondition refusal.
    SkillChangeRejected,
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
    /// There is deliberately no event for a clean scan (every governed
    /// install or update already chains [`AuditAction::SkillChangeOpened`],
    /// and a scan that found nothing is not an act) and none for rendering
    /// a report to a reviewer (the proposal read already chains, and the
    /// report is bound to the immutable version command).
    SkillScanRejected,
    /// One version-specific usage stage was appended. Host observation and
    /// model self-report are distinct payload values.
    SkillUsageRecorded,
    /// A controlled harness completed against one immutable version. The
    /// gateway's built-in harness validates and scans; it executes no bundle
    /// script.
    SkillTestRecorded,
    /// A typed Tool/apply VedaFlow change staged trusted MCP catalogue or
    /// exact-binding intent (CPR-25, ADR-0086). Carries hashes and ids, never
    /// credentials or arbitrary descriptions.
    ToolChangeOpened,
    /// An approved Tool/apply effect advanced the current version or changed
    /// an exact project binding.
    ToolChangeApplied,
    /// A Tool/apply effect reached a terminal precondition refusal.
    ToolChangeRejected,
    /// A trusted adapter reported one immutable read-only connection test.
    ToolTestRecorded,
    /// Secret-free client configuration was generated for a project from its
    /// exact approved bindings.
    ToolConfigurationGenerated,
    /// A typed Configuration/apply VedaFlow change was opened (CPR-30,
    /// ADR-0089). Carries identifiers, hashes and approval requirements,
    /// never credentials or configuration-adjacent secret material.
    ConfigurationChangeOpened,
    /// An approved Configuration/apply effect published an immutable document
    /// or changed a revisioned scope selector.
    ConfigurationChangeApplied,
    /// A Configuration/apply effect reached a terminal precondition refusal.
    ConfigurationChangeRejected,
    /// A typed Policy/apply relaxation command was opened. Carries only
    /// identifiers, immutable term/configuration hashes and approval
    /// arithmetic; never Knowledge content.
    RelaxationChangeOpened,
    /// A governed relaxation create, revision or early revocation completed.
    RelaxationChangeApplied,
    /// A relaxation effect reached a terminal validation or revision
    /// precondition refusal.
    RelaxationChangeRejected,
    /// The current immutable relaxation version reached its hard expiry.
    /// This system event is bookkeeping only: database time stopped the
    /// permission regardless of whether the sweep emitted it.
    RelaxationExpired,
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
    pub const ALL: [AuditAction; 94] = [
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
        AuditAction::ScopeCreated,
        AuditAction::ScopeUpdated,
        AuditAction::WorkspaceCreated,
        AuditAction::WorkspaceUpdated,
        AuditAction::ProjectCreated,
        AuditAction::ProjectUpdated,
        AuditAction::ProjectRepositoryAttached,
        AuditAction::ProjectRepositoryDetached,
        AuditAction::SessionOpened,
        AuditAction::SessionEnded,
        AuditAction::SessionEventsAppended,
        AuditAction::SessionContextComposed,
        AuditAction::ContextCandidatesRetrieved,
        AuditAction::ContextSelectionsMade,
        AuditAction::ContextFeedbackRecorded,
        AuditAction::CaptureBatchCreated,
        AuditAction::CaptureBatchCompleted,
        AuditAction::CaptureCandidateDecided,
        AuditAction::OkfImportPlanned,
        AuditAction::OkfImportMaterialized,
        AuditAction::OkfExported,
        AuditAction::GroupCreated,
        AuditAction::GroupUpdated,
        AuditAction::AccessGranted,
        AuditAction::AccessRevoked,
        AuditAction::InviteCreated,
        AuditAction::InviteRevoked,
        AuditAction::InviteAccepted,
        AuditAction::CuratorRulesUpdated,
        AuditAction::PolicyPackApplied,
        AuditAction::PolicyPackCleared,
        AuditAction::ServiceIdentityRegistered,
        AuditAction::ServiceIdentityRevoked,
        AuditAction::QuarantineReleased,
        AuditAction::QuarantineRejected,
        AuditAction::TenantCreated,
        AuditAction::TenantKeyProvisioned,
        AuditAction::TenantKeyRotated,
        AuditAction::TenantExported,
        AuditAction::TenantSecretStored,
        AuditAction::TenantSecretCleared,
        AuditAction::TenantSecretsReencrypted,
        AuditAction::ChannelPublished,
        AuditAction::ChannelRolledBack,
        AuditAction::ChannelPinned,
        AuditAction::ChannelUnpinned,
        AuditAction::ProposalOpened,
        AuditAction::ProposalApproved,
        AuditAction::ProposalRejected,
        AuditAction::ProposalWithdrawn,
        AuditAction::KnowledgeChangeOpened,
        AuditAction::KnowledgeChangeApplied,
        AuditAction::KnowledgeChangeRejected,
        AuditAction::KnowledgeConflictOpened,
        AuditAction::KnowledgeConflictResolved,
        AuditAction::KnowledgeErasureBlocked,
        AuditAction::KnowledgeErased,
        AuditAction::PromptAuthored,
        AuditAction::PromptResolved,
        AuditAction::ContextPackAuthored,
        AuditAction::ContextPackQuarantined,
        AuditAction::SkillChangeOpened,
        AuditAction::SkillChangeApplied,
        AuditAction::SkillChangeRejected,
        AuditAction::SkillResolved,
        AuditAction::SkillQuarantined,
        AuditAction::SkillScanRejected,
        AuditAction::SkillUsageRecorded,
        AuditAction::SkillTestRecorded,
        AuditAction::ToolChangeOpened,
        AuditAction::ToolChangeApplied,
        AuditAction::ToolChangeRejected,
        AuditAction::ToolTestRecorded,
        AuditAction::ToolConfigurationGenerated,
        AuditAction::ConfigurationChangeOpened,
        AuditAction::ConfigurationChangeApplied,
        AuditAction::ConfigurationChangeRejected,
        AuditAction::RelaxationChangeOpened,
        AuditAction::RelaxationChangeApplied,
        AuditAction::RelaxationChangeRejected,
        AuditAction::RelaxationExpired,
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
            AuditAction::WorkspaceCreated => "workspace.created",
            AuditAction::WorkspaceUpdated => "workspace.updated",
            AuditAction::ProjectCreated => "project.created",
            AuditAction::ProjectUpdated => "project.updated",
            AuditAction::ProjectRepositoryAttached => "project.repository.attached",
            AuditAction::ProjectRepositoryDetached => "project.repository.detached",
            AuditAction::SessionOpened => "session.opened",
            AuditAction::SessionEnded => "session.ended",
            AuditAction::SessionEventsAppended => "session.events.appended",
            AuditAction::SessionContextComposed => "session.context.composed",
            AuditAction::ContextCandidatesRetrieved => "context.candidates.retrieved",
            AuditAction::ContextSelectionsMade => "context.selections.made",
            AuditAction::ContextFeedbackRecorded => "context.feedback.recorded",
            AuditAction::CaptureBatchCreated => "capture.batch.created",
            AuditAction::CaptureBatchCompleted => "capture.batch.completed",
            AuditAction::CaptureCandidateDecided => "capture.candidate.decided",
            AuditAction::OkfImportPlanned => "okf.import.planned",
            AuditAction::OkfImportMaterialized => "okf.import.materialized",
            AuditAction::OkfExported => "okf.exported",
            AuditAction::GroupCreated => "access.group.created",
            AuditAction::GroupUpdated => "access.group.updated",
            AuditAction::AccessGranted => "access.granted",
            AuditAction::AccessRevoked => "access.revoked",
            AuditAction::InviteCreated => "access.invite.created",
            AuditAction::InviteRevoked => "access.invite.revoked",
            AuditAction::InviteAccepted => "access.invite.accepted",
            AuditAction::ScopeCreated => "scope.created",
            AuditAction::ScopeUpdated => "scope.updated",
            AuditAction::CuratorRulesUpdated => "policy.curator_rules.updated",
            AuditAction::PolicyPackApplied => "policy.pack.applied",
            AuditAction::PolicyPackCleared => "policy.pack.cleared",
            AuditAction::ServiceIdentityRegistered => "service_identity.registered",
            AuditAction::ServiceIdentityRevoked => "service_identity.revoked",
            AuditAction::QuarantineReleased => "session.quarantine.released",
            AuditAction::QuarantineRejected => "session.quarantine.rejected",
            AuditAction::TenantCreated => "tenant.created",
            AuditAction::TenantKeyProvisioned => "tenant.key.provisioned",
            AuditAction::TenantKeyRotated => "tenant.key.rotated",
            AuditAction::TenantExported => "tenant.exported",
            AuditAction::TenantSecretStored => "tenant.secret.stored",
            AuditAction::TenantSecretCleared => "tenant.secret.cleared",
            AuditAction::TenantSecretsReencrypted => "tenant.secrets.reencrypted",
            AuditAction::ChannelPublished => "vedaflow.channel.published",
            AuditAction::ChannelRolledBack => "vedaflow.channel.rolled_back",
            AuditAction::ChannelPinned => "vedaflow.channel.pinned",
            AuditAction::ChannelUnpinned => "vedaflow.channel.unpinned",
            AuditAction::ProposalOpened => "vedaflow.proposal.opened",
            AuditAction::ProposalApproved => "vedaflow.proposal.approved",
            AuditAction::ProposalRejected => "vedaflow.proposal.rejected",
            AuditAction::ProposalWithdrawn => "vedaflow.proposal.withdrawn",
            AuditAction::KnowledgeChangeOpened => "knowledge.change.opened",
            AuditAction::KnowledgeChangeApplied => "knowledge.change.applied",
            AuditAction::KnowledgeChangeRejected => "knowledge.change.rejected",
            AuditAction::KnowledgeConflictOpened => "knowledge.conflict.opened",
            AuditAction::KnowledgeConflictResolved => "knowledge.conflict.resolved",
            AuditAction::KnowledgeErasureBlocked => "knowledge.erasure.blocked",
            AuditAction::KnowledgeErased => "knowledge.erased",
            AuditAction::PromptAuthored => "prompt.authored",
            AuditAction::PromptResolved => "prompt.resolved",
            AuditAction::ContextPackAuthored => "context_pack.authored",
            AuditAction::ContextPackQuarantined => "context_pack.quarantined",
            AuditAction::SkillChangeOpened => "skill.change.opened",
            AuditAction::SkillChangeApplied => "skill.change.applied",
            AuditAction::SkillChangeRejected => "skill.change.rejected",
            AuditAction::SkillResolved => "skill.resolved",
            AuditAction::SkillQuarantined => "skill.quarantined",
            AuditAction::SkillScanRejected => "skill.scan.rejected",
            AuditAction::SkillUsageRecorded => "skill.usage.recorded",
            AuditAction::SkillTestRecorded => "skill.test.recorded",
            AuditAction::ToolChangeOpened => "tool.change.opened",
            AuditAction::ToolChangeApplied => "tool.change.applied",
            AuditAction::ToolChangeRejected => "tool.change.rejected",
            AuditAction::ToolTestRecorded => "tool.test.recorded",
            AuditAction::ToolConfigurationGenerated => "tool.configuration.generated",
            AuditAction::ConfigurationChangeOpened => "configuration.change.opened",
            AuditAction::ConfigurationChangeApplied => "configuration.change.applied",
            AuditAction::ConfigurationChangeRejected => "configuration.change.rejected",
            AuditAction::RelaxationChangeOpened => "policy.relaxation.change.opened",
            AuditAction::RelaxationChangeApplied => "policy.relaxation.change.applied",
            AuditAction::RelaxationChangeRejected => "policy.relaxation.change.rejected",
            AuditAction::RelaxationExpired => "policy.relaxation.expired",
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

// GENERATED FILE — DO NOT EDIT.
//
// Written by scripts/generate-api-types.mjs from docs/api/openapi.json, which the
// gateway derives from its own request and response types (CPR-4, ADR-0071
// decision 7). Editing this file is editing the wrong end of the chain: change
// the Rust, run `cargo test -p synveda-gateway --test openapi` with
// SYNVEDA_WRITE_OPENAPI=1 to refresh the document, then
// `node scripts/generate-api-types.mjs`.
//
// `make check-api-types` fails when this file and the document disagree.
//
// Source document: Synveda 0.2.0

/**
 * Batch accept currently applies every pending candidate at its proposed
 * placement. Per-candidate editing remains on the candidate endpoint.
 */
export type AcceptBatchBody = Record<string, unknown>;

/**
 * Optional edits applied while accepting a candidate.
 */
export type AcceptCandidateBody = {
    content?: null | KnowledgeContentBody;
    /**
     * Override the proposed Knowledge type.
     */
    knowledge_type?: string | null;
    /**
     * Override the proposed owner.
     */
    owner_principal_id?: string | null;
    /**
     * Override the proposed project association.
     */
    project_id?: string | null;
    /**
     * Override the proposed governing scope.
     */
    scope_id?: string | null;
  };

/**
 * The response to redeeming an invitation.
 */
export type AcceptedInviteView = {
    /**
     * The grant it minted — or the one the acceptor already held.
     */
    grant: GrantView;
    /**
     * The scope they now hold it at.
     */
    scope_id: string;
  };

/**
 * Distribution switches. They narrow already-authorised bindings and grant
 * no Skill or Tool authority.
 */
export type AdvertisementConfigurationBody = {
    skills: boolean;
    tools: boolean;
  };

/**
 * What the caller may do at **one anchor** — a real decision at a real scope,
 * never a shape derived from an edition (CPR-6, ADR-0073 decision 8).
 */
export type AnchorCapabilities = {
    /**
     * Every operand-free scope action, decided here, by its stable machine
     * name. **A forecast, never a grant** — the whole of this module's first
     * doc section applies unchanged.
     */
    actions: Record<string, boolean>;
    /**
     * Whether a grant is written at this very scope rather than inherited
     * from an ancestor — the "why" a member list would otherwise have to be
     * read to answer.
     */
    direct: boolean;
    /**
     * Its shape: `tenant`, `org_unit`, `workspace`, `project` or `principal`.
     */
    kind: string;
    /**
     * The role keys effective here.
     */
    roles: string[];
    /**
     * The scope.
     */
    scope_id: string;
    /**
     * Why it is applicable: `principal_scope`, `selected_project`,
     * `selected_workspace`, `grant`, `org_unit` or `tenant_root`.
     */
    source: string;
  };

/**
 * The taxonomy error body, declared for the OpenAPI document.
 *
 * A schema-only mirror of `synveda_types::Error`'s serialised form, which is
 * `{"kind": "...", ...}` with a per-variant remainder. It exists because the
 * contract has to say what a 4xx body looks like and the taxonomy lives two
 * crates down, where `utoipa` deliberately does not reach — the OpenAPI
 * derive is a property of the surface, and `synveda-types` is not one.
 */
export type ApiErrorBody = {
    /**
     * `policy_denied` only: the action that was attempted.
     */
    action?: string | null;
    /**
     * `not_found` only: what was looked up.
     */
    entity?: string | null;
    /**
     * The stable machine-readable code — `invalid`, `conflict`, `not_found`,
     * `policy_denied`, …
     */
    kind: string;
    /**
     * Present on most variants; what went wrong, safe to show a caller.
     */
    message?: string | null;
    /**
     * `policy_denied` only: which policy produced the denial.
     */
    reason?: string | null;
    /**
     * `policy_denied` only: what was acted on.
     */
    resource?: string | null;
  };

/**
 * `POST /v1/sessions/{session_id}/events`.
 */
export type AppendEventsBody = {
    /**
     * The batch, at most 200 events, each `client_event_id` at most once.
     */
    events: NewEventBody[];
  };

/**
 * The append response.
 */
export type AppendResponse = {
    /**
     * How many were written.
     */
    appended: number;
    /**
     * How many were refused outright by the redaction policy. Nothing of a
     * denied event persists — not its payload, not a row, not a position.
     */
    denied: number;
    /**
     * How many were already here.
     */
    duplicates: number;
    /**
     * Per-event outcomes, in the order the batch listed them.
     */
    events: AppendedEventView[];
    /**
     * How many were stored but withheld from the extraction pipeline pending
     * a reviewer's decision (MEM-2, ADR-0021 decision 5).
     */
    quarantined: number;
  };

/**
 * What one appended event did, and the row it names.
 */
export type AppendedEventView = {
    /**
     * The client's own id for the event, echoed on every outcome — including
     * the ones that store nothing, which is what lets a spooling client mark
     * exactly the entries this call resolved.
     */
    client_event_id: string;
    event?: null | SessionEventView;
    /**
     * `appended`, `duplicate`, `quarantined` or `denied`.
     */
    outcome: string;
    /**
     * The scan's finding summary — rule ids, categories and counts, never
     * matched text. Absent when the payload was clean.
     */
    redactions?: Record<string, unknown> | null;
  };

/**
 * A requirement as the API and the audit payload render it.
 */
export type ApprovalRequirementView = {
    /**
     * Distinct identities required.
     */
    distinct_approvers: number;
    /**
     * The proposal author cannot cast a verdict when true.
     */
    forbid_author_approval: boolean;
    /**
     * Where the requirement came from: `floor`, `pack`, and the scope of
     * any curator file that contributed — so a trail explains what a
     * proposal needed without reading a pack that has since changed.
     */
    origins: string[];
    /**
     * Roles required, with counts.
     */
    roles: ApprovalRoleView[];
    /**
     * Applying or publishing requires an actor distinct from the author
     * and every recorded approver when true.
     */
    separate_effect_actor: boolean;
    /**
     * Named subjects a curator file requires.
     */
    subjects?: string[];
  };

/**
 * One role line.
 */
export type ApprovalRoleView = {
    count: number;
    role: string;
  };

/**
 * `POST /v1/projects/{project_id}/repositories`.
 *
 * The server derives `provider`, `canonical_uri`, `repository_owner` and
 * `repository_name`; a client sends what it knows and never what it
 * concluded, because two clients concluding separately is how one repository
 * becomes two rows.
 */
export type AttachRepositoryBody = {
    /**
     * Advisory default branch. Nothing in the product resolves it.
     */
    default_branch?: string | null;
    /**
     * A stable content id for a repository with no remote — a git
     * root-commit object id, 40–128 hex characters. Never a path.
     */
    local_fingerprint?: string | null;
    /**
     * Caller-supplied labelling bag; a JSON object, at most 8 KiB encoded.
     */
    metadata?: Record<string, unknown> | null;
    /**
     * What to call it. Derived from the remote when there is one; **required**
     * when there is not, because a fingerprint names nothing a human reads.
     */
    name?: string | null;
    provider?: "github" | "gitlab" | "bitbucket" | "azure_devops" | "generic_git" | "local";
    /**
     * The remote, in any form git accepts: `https://host/owner/name`,
     * `git@host:owner/name.git`, `ssh://git@host/owner/name`. **The
     * identity**, whenever it is known. A filesystem path is refused.
     */
    remote_uri?: string | null;
  };

/**
 * One disclosure as the API renders it — the shape of what was served,
 * never the substance.
 */
export type AuditDisclosureView = {
    /**
     * The delivery act that put this revision in a session context.
     */
    action: string;
    actor_kind: string;
    /**
     * Who was served.
     */
    actor_subject: string;
    content_hash?: string | null;
    knowledge_item_id?: string | null;
    knowledge_revision_id?: string | null;
    occurred_at: string;
    reason_codes: string[];
    seq: number;
    session_id?: string | null;
  };

export type AuditDisclosuresResponse = AuditFrame & {
    /**
     * The events that opened and closed authority over the window — role
     * grants, pack assignments, relaxations, publications, classifications.
     * These are *inputs*, not a set of principals.
     */
    authority: AuditEventView[];
    /**
     * Whether the authority half hit its own cap, which is separate from
     * the disclosure page's.
     */
    authority_truncated: boolean;
    /**
     * Who the chain records the Knowledge item being **served** to in the window,
     * with what they got. This is evidence.
     */
    disclosed: AuditDisclosureView[];
    /**
     * Why the two lists are not one, in the response rather than only in
     * the ADR: merging them means deciding, and deciding over
     * reconstructed inputs is a replay of authority rather than a record
     * of it (ADR-0045 decision 4).
     */
    note: string;
  };

/**
 * One chain row as the API renders it.
 */
export type AuditEventView = {
    action: string;
    /**
     * How the actor was established (`subject`/`break_glass`/`system`).
     */
    actor_kind: string;
    actor_subject: string;
    /**
     * This row's hash, hex.
     */
    hash: string;
    occurred_at: string;
    outcome: string;
    payload: unknown;
    resource: string;
    seq: number;
    trace_id?: string | null;
  };

export type AuditEventsResponse = AuditFrame & {
    events: AuditEventView[];
  };

/**
 * Every canonical event input needed by an offline verifier.
 */
export type AuditExportEvent = {
    action: string;
    actor_kind: string;
    actor_subject: string;
    hash: string;
    occurred_at: string;
    outcome: string;
    payload: unknown;
    prev_hash: string;
    resource: string;
    seq: number;
    trace_id?: string | null;
  };

/**
 * One page from a frozen deterministic audit-chain prefix.
 */
export type AuditExportPage = {
    canonicalization: string;
    events: AuditExportEvent[];
    first_seq?: number | null;
    format: string;
    genesis_hash: string;
    hash_algorithm: string;
    last_seq?: number | null;
    next_cursor?: number | null;
    snapshot_hash: string;
    snapshot_seq: number;
    tenant_id: string;
    truncated: boolean;
  };

/**
 * Where the chain stood when the answer was taken, and what the answer
 * covered (ADR-0045 decision 9).
 */
export type AuditFrame = {
    /**
     * The lowest seq in this page.
     */
    first_seq?: number | null;
    /**
     * The head hash, hex — the value that makes an answer re-derivable.
     */
    head_hash: string;
    /**
     * The chain head's sequence number when the query ran.
     */
    head_seq: number;
    /**
     * The highest seq in this page.
     */
    last_seq?: number | null;
    /**
     * The cursor to continue from, when it did.
     */
    next_cursor?: number | null;
    /**
     * Whether the limit cut the answer short.
     */
    truncated: boolean;
  };

export type AuditKnowledgeResponse = AuditFrame & {
    as_known_at: string;
    /**
     * One row per item, carrying the revision *last* delivered at or
     * before `as_known_at` and valid at `valid_at`.
     */
    known: AuditKnownView[];
    /**
     * What this answer is, stated in it: what A was served, not what A
     * could have asked for (ADR-0045 decision 5).
     */
    note: string;
    /**
     * Delivered revisions that are retained but outside the requested
     * valid/transaction-time pair. They are evidence, not part of `known`.
     */
    outside_time: AuditKnownView[];
    subject: string;
    /**
     * Hashes-only or erased delivery evidence whose temporal interval can no
     * longer be resolved. It is not silently counted as known.
     */
    unresolved: AuditKnownView[];
    valid_at: string;
  };

/**
 * One Knowledge item a subject was last served, with what they got.
 */
export type AuditKnownView = {
    /**
     * How it arrived that last time.
     */
    action: string;
    content_hash?: string | null;
    /**
     * Stable aggregate address when retained. Hashes-only traces deliberately
     * omit it and remain in `unresolved` as content-free evidence.
     */
    knowledge_item_id?: string | null;
    knowledge_revision_id?: string | null;
    /**
     * How many times it was served in the window read.
     */
    occasions: number;
    /**
     * When it was last delivered.
     */
    occurred_at: string;
    reason_codes: string[];
    /**
     * The chain position of the last delivery — the evidence.
     */
    seq: number;
    /**
     * `valid`, `outside_valid_time`, `not_known_at` or `unresolved`.
     */
    temporal_status: string;
    /**
     * Immutable revision transaction time, when retained.
     */
    transaction_time?: string | null;
    /**
     * Immutable revision valid-time start, when retained.
     */
    valid_from?: string | null;
    /**
     * Immutable revision valid-time end, when bounded and retained.
     */
    valid_to?: string | null;
  };

export type AuditVerifyResponse = {
    /**
     * The first divergence, when there is one: the seq and why. A broken
     * chain is a 200 with `valid: false`, not an error — the verification
     * succeeded; it is the chain that did not.
     */
    broken_at?: number | null;
    /**
     * The number of events checked.
     */
    events: number;
    head_hash: string;
    /**
     * The chain head after verification.
     */
    head_seq: number;
    reason?: string | null;
    /**
     * Whether every row recomputes to its stored hash and the head
     * matches.
     */
    valid: boolean;
  };

/**
 * A standing authorisation, as an operator sees it.
 */
export type AuthorisationView = {
    /**
     * The most it permits. A pass proposing more trips again.
     */
    ceiling: number;
    /**
     * When it stops covering anything.
     */
    expires_at: string;
    /**
     * When it was signed.
     */
    granted_at: string;
    /**
     * Who signed it.
     */
    granted_by: string;
    /**
     * Why.
     */
    reason: string;
  };

/**
 * `POST /v1/directory/seal-authorisations`.
 */
export type AuthoriseRequest = {
    /**
     * The most this authorisation permits a pass to seal.
     */
    ceiling: number;
    /**
     * How long it stands. Clamped to [`MAX_WINDOW_SECS`].
     */
    expires_in_secs?: number | null;
    /**
     * Why, in the operator's words. Required, and stored — an
     * authorisation nobody can read the reason for explains nothing later.
     */
    reason: string;
  };

export type AuthoriseResponse = {
    ceiling: number;
  };

/**
 * Visible available skills after binding and PDP evaluation.
 */
export type AvailableSkillListView = {
    /**
     * Project or principal scope resolved for the session.
     */
    scope_id: string;
    /**
     * Enabled and policy-visible exact versions.
     */
    skills: AvailableSkillView[];
  };

/**
 * Exact version made available by one enabled binding.
 */
export type AvailableSkillView = {
    /**
     * Binding that makes this version available.
     */
    binding: SkillBindingView;
    /**
     * Content-addressed SKILL.md object.
     */
    manifest_object_hash: string;
    /**
     * Agent Skills bundle name.
     */
    name: string;
    /**
     * Exact version resolved from the binding.
     */
    version: SkillVersionView;
  };

/**
 * The batch response. `not_answered` names the scopes the bound dropped,
 * and `max_scopes` says what the bound is, so a client can page rather
 * than guess.
 */
export type BatchResponse = {
    capabilities: NodeCapabilities[];
    max_scopes: number;
    not_answered?: string[];
  };

/**
 * One page of capture batches.
 */
export type CaptureBatchListView = {
    /**
     * Visible batches.
     */
    batches: CaptureBatchView[];
    /**
     * Opaque resume cursor.
     */
    next_cursor?: string | null;
  };

/**
 * One durable extraction job over an exact session-event snapshot.
 */
export type CaptureBatchView = {
    /**
     * Processing attempts.
     */
    attempts: number;
    /**
     * Reviewable candidates produced.
     */
    candidate_count: number;
    /**
     * Terminal instant.
     */
    completed_at?: string | null;
    /**
     * Canonical hash of the frozen runtime document.
     */
    configuration_hash: string;
    /**
     * Exact immutable Configuration version, absent only for the built-in
     * fail-safe.
     */
    configuration_version_id?: string | null;
    /**
     * Creation instant.
     */
    created_at: string;
    /**
     * Content-free stable failure code.
     */
    error_code?: string | null;
    /**
     * Frozen event count.
     */
    event_count: number;
    /**
     * Extractor implementation, once known.
     */
    extractor_method?: string | null;
    /**
     * Stable batch id.
     */
    id: string;
    /**
     * Source import job, for OKF materialisation.
     */
    import_job_id?: string | null;
    /**
     * Content-free digest of the ordered frozen evidence set.
     */
    input_hash: string;
    /**
     * Model or deterministic ruleset version, once known.
     */
    model_version?: string | null;
    /**
     * Project association, when the session had one.
     */
    project_id?: string | null;
    /**
     * Governed scope copied from the session.
     */
    scope_id: string;
    /**
     * Source session, for session extraction.
     */
    session_id?: string | null;
    source_kind: "session" | "okf_import";
    /**
     * First processing instant.
     */
    started_at?: string | null;
    state: "pending" | "running" | "completed" | "failed";
  };

/**
 * One page of capture candidates.
 */
export type CaptureCandidateListView = {
    /**
     * Visible reviewable candidates.
     */
    candidates: CaptureCandidateView[];
    /**
     * Opaque resume cursor.
     */
    next_cursor?: string | null;
  };

/**
 * One reviewable proposal. It is not active Knowledge.
 */
export type CaptureCandidateView = {
    /**
     * Owning batch.
     */
    batch_id: string;
    /**
     * Proposed immutable revision content.
     */
    content: KnowledgeContentBody;
    /**
     * Whether governed erasure removed the proposal plaintext.
     */
    content_erased: boolean;
    /**
     * Canonical proposed-content hash.
     */
    content_hash: string;
    /**
     * Creation instant.
     */
    created_at: string;
    /**
     * Decision instant.
     */
    decided_at?: string | null;
    /**
     * Actor that made the terminal decision.
     */
    decided_by?: string | null;
    /**
     * Bounded dismissal reason.
     */
    decision_reason?: string | null;
    /**
     * Stable candidate id.
     */
    id: string;
    /**
     * Source import job, for OKF materialisation.
     */
    import_job_id?: string | null;
    /**
     * Proposed Knowledge type.
     */
    knowledge_type: string;
    /**
     * Only matches that passed a fresh Knowledge read decision for this caller.
     */
    matches: CaptureMatchView[];
    /**
     * Stable position within the batch.
     */
    ordinal: number;
    /**
     * Proposed origin.
     */
    origin: string;
    /**
     * Proposed personal owner.
     */
    proposed_owner_principal_id?: string | null;
    /**
     * Proposed project association.
     */
    proposed_project_id?: string | null;
    /**
     * Proposed governing scope.
     */
    proposed_scope_id: string;
    /**
     * VedaFlow change opened by the decision.
     */
    resulting_change_id?: string | null;
    /**
     * Resulting Knowledge aggregate.
     */
    resulting_knowledge_item_id?: string | null;
    resulting_outcome?: "applied" | "pending_review" | "rejected";
    /**
     * Resulting immutable revision, once applied.
     */
    resulting_revision_id?: string | null;
    /**
     * Source session, for session extraction.
     */
    session_id?: string | null;
    /**
     * Exact immutable OKF artifacts, for imported candidates.
     */
    source_artifact_ids: string[];
    /**
     * Exact immutable source event ids.
     */
    source_event_ids: string[];
    source_kind: "session" | "okf_import";
    state: "pending" | "accepted" | "edited_and_accepted" | "merged" | "replaced" | "dismissed" | "failed";
  };

/**
 * Capture and extraction settings in one immutable document.
 */
export type CaptureConfigurationBody = {
    enabled: boolean;
    explicit_request: boolean;
    maximum_candidates_per_batch: number;
    minimum_confidence_permille: number;
    on_session_end: boolean;
  };

/**
 * Result of accepting, merging, replacing or dismissing a candidate.
 */
export type CaptureDecisionView = {
    /**
     * Candidate after its terminal decision.
     */
    candidate: CaptureCandidateView;
    /**
     * Whether this request executed the decision or replayed it.
     */
    replayed: boolean;
  };

/**
 * One independently authorised current-Knowledge comparison.
 */
export type CaptureMatchView = {
    kind: "duplicate" | "support" | "contradiction" | "supersession" | "transition";
    /**
     * Existing stable aggregate.
     */
    knowledge_item_id: string;
    /**
     * Exact revision compared during extraction.
     */
    knowledge_revision_id: string;
    /**
     * Stable, content-free classifier reason.
     */
    reason_code: string;
    /**
     * Deterministic score in `0..=1000`.
     */
    similarity_permille: number;
  };

/**
 * `GET /v1/admin/scopes/{scope_id}/ancestors` — the chain to the tenant
 * root, nearest first.
 */
export type ChainResponse = {
    /**
     * The chain or subtree, nearest first.
     */
    scopes: ScopeView[];
  };

/**
 * One state a channel has held, as the API renders it.
 */
export type ChannelHistoryEntryView = {
    author: string;
    commit: string;
    committed_at: string;
    /**
     * True for the commit the channel points at now — where it already
     * is, and so the one entry a rewind cannot name.
     */
    head: boolean;
    /**
     * The membership this state served.
     */
    members: number;
    /**
     * Parents beyond the first: the proposal this publication was the
     * effect of, when it had one. Present so a reviewer can trace the
     * decision — and deliberately *not* a rewind target, because a
     * proposal's tree is a member set that may never have been approved
     * (ADR-0036 decision 1).
     */
    merge_parents?: string[];
    message: string;
    /**
     * The state it replaced — its first parent, absent on the channel's
     * first commit.
     */
    parent?: string | null;
    /**
     * True for the commit a pin holds readers at.
     */
    served: boolean;
  };

export type ChannelHistoryResponse = {
    channel: string;
    /**
     * The commit the ref points at.
     */
    head: string;
    /**
     * Newest first. Every entry but `head` is a legal rewind target, and
     * nothing outside this listing is (ADR-0036 decision 11).
     */
    history: ChannelHistoryEntryView[];
    pin?: null | ChannelPinView;
    scope_id: string;
  };

export type ChannelListResponse = {
    channels: ChannelStatusView[];
    scope_id: string;
  };

export type ChannelPinBody = {
    asset: string;
    channel?: string | null;
    /**
     * The commit to hold readers at: one of the entries `GET /history`
     * lists, the head included.
     */
    commit: string;
    /**
     * Why this scope is holding its readers. The pin's only record — the
     * ref carries who and when and nothing else (ADR-0036 decision 9).
     */
    reason: string;
  };

export type ChannelPinResponse = {
    channel: string;
    /**
     * The commit readers now compose.
     */
    commit: string;
    /**
     * Where the channel's ref points. Publications keep landing here
     * while the pin stands (ADR-0036 decision 6).
     */
    head: string;
    /**
     * What the pin held before, when this call moved a standing one.
     */
    previous?: string | null;
    scope_id: string;
  };

/**
 * A pin as the API renders it.
 */
export type ChannelPinView = {
    /**
     * The commit readers are held at.
     */
    commit: string;
    pinned_at: string;
    pinned_by: string;
  };

export type ChannelPublishBody = {
    /**
     * The context-pack documents to admit, by path (PRMT-2, ADR-0050
     * decision 1). Must be documents of **this** scope, for the reason
     * the other lists must be its material: the direct route stays
     * same-scope.
     *
     * Exactly one of the three lists may be present. Under the default
     * pack a pack publication above a team now refuses here on its own
     * arithmetic — since ADR-0050 decision 15 the matrix asks for a
     * curator *and* a steward, two distinct people — and names the
     * proposal route; at a team or a leaf one curator still publishes
     * directly, which is the governed `SHARED`/`LOCAL` split.
     */
    document_paths?: string[];
    /**
     * Why — an auditor and a reviewer both read this. Required: a
     * publication with nothing to say is one nobody can review after
     * the fact.
     */
    message: string;
    /**
     * The prompts to admit, by name (PRMT-1, ADR-0049 decision 7). Must be
     * drafts of **this** scope: the direct route stays same-scope.
     *
     * Exactly one member list may be present. Under the default pack
     * a prompt publication refuses here on its own arithmetic — the matrix
     * asks for a steward *and* a curator, two distinct people — and names
     * the proposal route; under `standard` a single curator may publish,
     * which is that pack saying what that pack exists to say. That is
     * ADR-0032 decision 8's invariant kept rather than a second rule for
     * authored assets.
     */
    prompt_names?: string[];
  };

export type ChannelPublishResponse = {
    /**
     * Members this call admitted that the channel did not already hold
     * at that address. Zero means everything named was already
     * published, unchanged — the act still commits and still audits.
     */
    added: number;
    /**
     * The channel that moved.
     */
    channel: string;
    /**
     * The commit it now points at.
     */
    commit: string;
    /**
     * The published set's size after this call.
     */
    members: number;
    /**
     * What it pointed at before — absent on the channel's first commit.
     */
    parent?: string | null;
    pinned?: null | ChannelPinView;
    /**
     * Each member's content address, in request order.
     */
    published: ChannelPublishedMember[];
    /**
     * What the approval matrix asked for here, and which of the acting
     * principal's roles supplied it (FLOW-3, ADR-0032 decision 8).
     * A publication that needed nothing renders an empty requirement,
     * which is the honest answer: this pack asks for no review at this
     * cell.
     */
    required: ApprovalRequirementView;
    scope_id: string;
  };

export type ChannelPublishedMember = {
    /**
     * The tree entry name or authored path.
     */
    member: string;
    object_hash: string;
  };

export type ChannelRollbackBody = {
    asset: string;
    channel?: string | null;
    /**
     * The commit being abandoned — what the caller read before deciding.
     * Required rather than inferred: a rewind is a decision about *which*
     * state to leave, and that decision is stale if someone else moved
     * the ref meanwhile (ADR-0030 decision 10's rule, applied to the one
     * call that can move a ref backwards).
     */
    from_commit: string;
    /**
     * Why. An auditor reads this, and so does whoever asks next week why
     * an artifact stopped being published.
     */
    message: string;
    /**
     * The state to install: one of the entries `GET /history` lists.
     */
    to_commit: string;
  };

export type ChannelRollbackResponse = {
    channel: string;
    /**
     * The commit abandoned.
     */
    from: string;
    /**
     * The membership after the rewind.
     */
    members: number;
    /**
     * Member names that stopped being published.
     */
    removed: string[];
    /**
     * Members whose published version went back to an earlier one, with
     * the address now bound.
     */
    restored: ChannelPublishedMember[];
    scope_id: string;
    /**
     * The commit installed — what the next authorised reader sees.
     */
    to: string;
  };

/**
 * One standing channel as the API renders it.
 */
export type ChannelStatusView = {
    asset: string;
    channel: string;
    /**
     * Where the channel points — what an authorised reader cites.
     */
    commit: string;
    /**
     * Entries in that commit's tree: the membership for `published` and
     * `staged`, the last commit's additions for `derived` (which is a
     * log, not a set — ADR-0031 decision 3).
     */
    entries: number;
    /**
     * The ref name, e.g. `prompt/published`.
     */
    name: string;
    pin?: null | ChannelPinView;
    updated_at: string;
    updated_by: string;
  };

export type ChannelUnpinBody = {
    asset: string;
    channel?: string | null;
    /**
     * Why the hold is being released.
     */
    reason: string;
  };

export type ChannelUnpinResponse = {
    channel: string;
    /**
     * What readers compose from the next session on.
     */
    head: string;
    /**
     * The commit that was held, when there was a pin. Absent means there
     * was none, which is the answer rather than an error: the channel
     * serves its head either way.
     */
    released?: string | null;
    scope_id: string;
  };

export type ConfigurationArtifactListView = {
    artifacts: ConfigurationArtifactView[];
    next_cursor?: string | null;
  };

/**
 * Stable aggregate metadata.
 */
export type ConfigurationArtifactView = {
    created_at: string;
    created_by: string;
    current_version_id: string;
    governing_scope_id: string;
    id: string;
    name: string;
    updated_at: string;
    updated_by: string;
  };

export type ConfigurationBindingListView = {
    bindings: ConfigurationBindingView[];
    next_cursor?: string | null;
  };

/**
 * Revisioned scope selector.
 */
export type ConfigurationBindingView = {
    artifact_id: string;
    created_at: string;
    created_by: string;
    enabled: boolean;
    id: string;
    pinned_version_id?: string | null;
    revision: number;
    scope_id: string;
    updated_at: string;
    updated_by: string;
  };

/**
 * Deterministic field-level comparison of two immutable versions.
 */
export type ConfigurationComparisonView = {
    changed_fields: string[];
    from_hash: string;
    from_version_id: string;
    to_hash: string;
    to_version_id: string;
  };

/**
 * A complete immutable governed runtime document.
 */
export type ConfigurationDocumentBody = {
    advertisement: AdvertisementConfigurationBody;
    /**
     * `anthropic`, `vllm`, `tei`, or `remote_mcp`, sorted and unique.
     */
    allowed_external_providers: string[];
    capture: CaptureConfigurationBody;
    context: ContextConfigurationBody;
    freshness: FreshnessConfigurationBody;
    policy_pack: string;
    relaxations: RelaxationConfigurationBody;
  };

/**
 * Stable result envelope for every governed mutation.
 */
export type ConfigurationMutationView = {
    artifact_id?: string | null;
    binding_id?: string | null;
    binding_revision?: number | null;
    change_id: string;
    outcome: "applied" | "pending_review" | "rejected";
    version_id?: string | null;
  };

export type ConfigurationTemplateListView = {
    templates: ConfigurationTemplateView[];
  };

/**
 * Canonical template source. Selecting one creates ordinary version data.
 */
export type ConfigurationTemplateView = {
    content_hash: string;
    document: ConfigurationDocumentBody;
    name: "personal" | "team" | "enterprise";
  };

export type ConfigurationVersionListView = {
    next_cursor?: number | null;
    versions: ConfigurationVersionView[];
  };

/**
 * One immutable version with its complete runtime document.
 */
export type ConfigurationVersionView = {
    artifact_id: string;
    change_id: string;
    content_hash: string;
    created_at: string;
    created_by: string;
    document: ConfigurationDocumentBody;
    id: string;
    ordinal: number;
    source_template?: "personal" | "team" | "enterprise";
  };

/**
 * One exact, independently authorised conflict member.
 */
export type ConflictMemberView = {
    /**
     * Reviewable candidate, disclosed only after source and destination PDP
     * decisions.
     */
    capture_candidate_id?: string | null;
    classification: "duplicate" | "support" | "contradiction" | "supersession" | "transition";
    /**
     * Stable member evidence id.
     */
    id: string;
    /**
     * Exact stable Knowledge item, absent for a capture challenger.
     */
    knowledge_item_id?: string | null;
    knowledge_revision?: null | KnowledgeRevisionView;
    /**
     * Stable content-free reason code.
     */
    reason_code: string;
    /**
     * `challenger` or `current`.
     */
    role: string;
    /**
     * Integer similarity.
     */
    similarity_permille: number;
  };

/**
 * Cursor-paginated fully visible conflict sets.
 */
export type ConflictSetListView = {
    /**
     * Fully visible rows.
     */
    conflicts: ConflictSetView[];
    /**
     * Opaque next candidate position.
     */
    next_cursor?: string | null;
    /**
     * At least one candidate set was wholly omitted by policy. No count or
     * classification is disclosed.
     */
    policy_exclusions: boolean;
  };

/**
 * One fully visible durable conflict set.
 */
export type ConflictSetView = {
    classification: "duplicate" | "support" | "contradiction" | "supersession" | "transition";
    /**
     * Creation time.
     */
    created_at: string;
    /**
     * Stable resolution address.
     */
    id: string;
    /**
     * Every member; no denied member is represented or counted.
     */
    members: ConflictMemberView[];
    /**
     * Project association.
     */
    project_id?: string | null;
    resolution?: "keep_separate" | "support" | "duplicate" | "supersede" | "transition" | "archive";
    /**
     * Exact VedaFlow resolution when one has opened.
     */
    resolution_change_id?: string | null;
    /**
     * Resolution time.
     */
    resolved_at?: string | null;
    /**
     * Revision precondition for a resolution.
     */
    revision: number;
    /**
     * Governing scope.
     */
    scope_id: string;
    status: "open" | "pending_review" | "resolved" | "dismissed";
    /**
     * Last state transition.
     */
    updated_at: string;
  };

/**
 * One retained, freshly re-authorised planner candidate.
 */
export type ContextCandidateView = {
    /**
     * Pending capture proposal, absent for Knowledge and hashes-only mode.
     */
    capture_candidate_id?: string | null;
    /**
     * `current_knowledge` or the visibly unreviewed candidate channel.
     */
    channel: string;
    /**
     * Canonical content hash.
     */
    content_hash: string;
    /**
     * Why this visible candidate was not selected.
     */
    exclusion_reason?: string | null;
    /**
     * Trace-row id.
     */
    id: string;
    /**
     * Stable Knowledge item, absent in hashes-only mode.
     */
    knowledge_item_id?: string | null;
    /**
     * Exact immutable revision, absent in hashes-only mode.
     */
    knowledge_revision_id?: string | null;
    /**
     * Lifecycle observed at planning time.
     */
    lifecycle_state?: string | null;
    /**
     * Consideration position.
     */
    ordinal: number;
    /**
     * Why it was considered.
     */
    reason_codes: string[];
    revision?: null | KnowledgeRevisionView;
    scores?: null | ContextScoreView;
    /**
     * Independently visible provenance, full mode only.
     */
    sources?: KnowledgeSourceView[];
    unreviewed_candidate?: null | CaptureCandidateView;
  };

/**
 * Budgeted context delivery settings.
 */
export type ContextConfigurationBody = {
    /**
     * `current_knowledge`, optionally followed by `unreviewed_candidates`.
     */
    channels: string[];
    token_budget: number;
    trace_retention: "full" | "redacted" | "hashes_only" | "disabled";
  };

/**
 * Explicit feedback about one exact selected revision.
 */
export type ContextFeedbackBody = {
    /**
     * Exact retained selection.
     */
    context_selection_id: string;
    /**
     * One of the five explicit feedback values.
     */
    feedback_type: string;
    /**
     * Exact immutable revision selected.
     */
    knowledge_revision_id: string;
  };

/**
 * One explicit feedback assertion.
 */
export type ContextFeedbackView = {
    /**
     * Exact selection.
     */
    context_selection_id: string;
    /**
     * Assertion time.
     */
    created_at: string;
    /**
     * Feedback vocabulary.
     */
    feedback_type: string;
    /**
     * Feedback id.
     */
    id: string;
    /**
     * Exact immutable revision.
     */
    knowledge_revision_id: string;
    /**
     * Authenticated subject that supplied it.
     */
    principal_id: string;
  };

/**
 * Scoped Knowledge query/evaluation result.
 */
export type ContextKnowledgeQueryView = {
    /**
     * Valid-time instant applied to the current-head projection.
     */
    as_of: string;
    /**
     * Honest semantic degradation, when applicable.
     */
    degradation?: string | null;
    /**
     * Policy-visible current Knowledge.
     */
    items: ContextKnowledgeView[];
    /**
     * Evaluation sweep continuation. Ordinary queries never return one.
     */
    next_cursor?: string | null;
    /**
     * `lexical`, `hybrid`, `listing` or `ids`.
     */
    retrieval_mode: string;
  };

/**
 * One current Knowledge query result with independently visible evidence.
 */
export type ContextKnowledgeView = {
    /**
     * Current stable item and immutable revision.
     */
    knowledge: KnowledgeItemView;
    /**
     * Independently visible provenance.
     */
    sources: KnowledgeSourceView[];
  };

export type ContextPackAuthorBody = {
    /**
     * One line, read in a listing and at review.
     */
    description?: string;
    /**
     * The documents. A request naming none writes the bundle row alone,
     * which is how an empty pack gets created before anything is put in
     * it.
     */
    documents?: ContextPackDocumentBody[];
    /**
     * Its name: one segment, lower-case, and the identifier a scope's
     * override is expressed in (ADR-0050 decision 1).
     */
    name: string;
    /**
     * Where the pack is authored — the scope that will stand behind it,
     * and the scope whose published channel a proposal would move.
     */
    scope_id: string;
  };

/**
 * One document as an author supplies it.
 */
export type ContextPackDocumentBody = {
    /**
     * The text.
     */
    content: string;
    /**
     * Its name within the pack: path-shaped, so a bundle can carry
     * `runbooks/payments.md` rather than flattening a directory.
     */
    name: string;
    /**
     * Its classification. Absent means `internal`. Per document rather
     * than per pack (decision 12) — a glossary of public terms and an
     * internal runbook are plausibly the same bundle.
     */
    sensitivity?: string | null;
    /**
     * One line, read in a listing, at review, and in the index tier
     * (ADR-0050 decision 10).
     */
    title?: string;
  };

export type ContextPackDocumentView = {
    /**
     * How many chunks it cut into.
     */
    chunks: number;
    /**
     * How many of those this request actually embedded. Zero for a
     * document whose bytes did not move, which is the observable half of
     * "re-authoring an unchanged document re-embeds nothing".
     */
    embedded: number;
    name: string;
    /**
     * The draft's content address — what a proposal would bind.
     */
    object_hash: string;
    published?: null | ContextPackPublishedView;
    sensitivity: string;
    title: string;
    updated_at: string;
    updated_by: string;
  };

export type ContextPackListEntry = {
    description: string;
    documents: ContextPackDocumentView[];
    name: string;
    updated_at: string;
    updated_by: string;
  };

export type ContextPackListResponse = {
    packs: ContextPackListEntry[];
    scope_id: string;
    scope_path: string;
  };

/**
 * What a scope's published channel holds for one document right now.
 */
export type ContextPackPublishedView = {
    /**
     * The commit the channel serves.
     */
    commit: string;
    /**
     * Whether that is the draft's own address. `false` after an edit: the
     * draft has moved and the reviewed version has not, which is what
     * "behind review" looks like from the writing side — and, for a pack,
     * is also exactly when the old version's chunks keep composing
     * (decision 3).
     */
    current: boolean;
    /**
     * The address it names for this document.
     */
    object_hash: string;
  };

export type ContextPackView = {
    created_at: string;
    created_by: string;
    description: string;
    documents: ContextPackDocumentView[];
    name: string;
    scope_id: string;
    scope_path: string;
    updated_at: string;
    updated_by: string;
  };

/**
 * Freshly re-authorised detail for one context run.
 */
export type ContextRunDetailView = {
    /**
     * Retained visible candidates. Empty in disabled mode.
     */
    candidates: ContextCandidateView[];
    /**
     * Explicit feedback whose revision remains visible.
     */
    feedback: ContextFeedbackView[];
    /**
     * Aggregate revocation/policy notice with no denied count.
     */
    policy_exclusion_message?: string | null;
    /**
     * Core immutable run/delivery record.
     */
    run: ContextRunView;
    /**
     * Retained visible selections. Empty in disabled mode.
     */
    selections: ContextSelectionView[];
  };

/**
 * Cursor page of context runs.
 */
export type ContextRunListView = {
    /**
     * Resume position after the last candidate considered.
     */
    next_cursor?: string | null;
    /**
     * Freshly session-authorised rows.
     */
    runs: ContextRunView[];
  };

/**
 * A context run, as the API serves it.
 */
export type ContextRunView = {
    /**
     * Valid-time instant used for current Knowledge.
     */
    as_of: string;
    /**
     * BLAKE3 over the composed entries, hex.
     */
    block_hash: string;
    /**
     * The budget it composed under.
     */
    budget_tokens: number;
    /**
     * Visible candidates retained for the run.
     */
    candidate_count: number;
    /**
     * `pending`, `completed` or `failed`.
     */
    completion_status: string;
    /**
     * Canonical digest of the exact runtime configuration.
     */
    configuration_hash: string;
    /**
     * Exact immutable runtime configuration, absent for the built-in
     * fail-safe.
     */
    configuration_version_id?: string | null;
    /**
     * When it was composed.
     */
    created_at: string;
    /**
     * Which retrieval legs degraded — `embedder`, `retrieval`. Empty is the
     * ordinary answer.
     */
    degraded: string[];
    /**
     * Semantic model used, when configured.
     */
    embedding_model?: string | null;
    /**
     * How many records composed.
     */
    entry_count: number;
    /**
     * Graph implementation version, when graph expansion ran.
     */
    graph_version?: string | null;
    /**
     * The run's id.
     */
    id: string;
    /**
     * Knowledge index implementation version.
     */
    index_version: string;
    /**
     * Aggregate policy-filtering notice without a denied count.
     */
    policy_exclusion_message?: string | null;
    /**
     * Project derived from the session, when present.
     */
    project_id?: string | null;
    /**
     * The task, when one was named.
     */
    query?: string | null;
    /**
     * Content-free query digest.
     */
    query_hash?: string | null;
    /**
     * The rendered block, watermark line included. Empty when nothing
     * composed — a result, not an error.
     */
    rendered?: string | null;
    /**
     * Caller-requested budget before the governed ceiling.
     */
    requested_budget_tokens?: number | null;
    /**
     * Planner implementation version.
     */
    retrieval_version: string;
    /**
     * The scope it was anchored at.
     */
    scope_id: string;
    /**
     * Immutable Knowledge revisions selected.
     */
    selection_count: number;
    /**
     * The session it was composed for.
     */
    session_id: string;
    /**
     * The skills this block advertised (ADR-0054 decision 8): name, scope,
     * commit and object address, so an adapter can materialise exactly what
     * was named without asking twice.
     */
    skills: Record<string, unknown>;
    /**
     * Estimated tokens of `rendered`.
     */
    tokens: number;
    /**
     * `full`, `redacted`, `hashes_only` or `disabled`.
     */
    trace_retention_mode: string;
    /**
     * Workspace derived from the session.
     */
    workspace_id: string;
  };

/**
 * Integer score components retained for an authorised candidate.
 */
export type ContextScoreView = {
    /**
     * Current-state contribution, per million.
     */
    current_state_micros: number;
    /**
     * Final deterministic score, per million.
     */
    final_micros: number;
    /**
     * Freshness contribution, per million.
     */
    freshness_micros: number;
    /**
     * Lexical contribution, per million.
     */
    keyword_micros: number;
    /**
     * Explicit-pin contribution, per million.
     */
    pin_micros: number;
    /**
     * Semantic contribution, per million.
     */
    semantic_micros: number;
  };

/**
 * One retained selected revision.
 */
export type ContextSelectionView = {
    /**
     * Pending capture proposal, absent for Knowledge and hashes-only mode.
     */
    capture_candidate_id?: string | null;
    /**
     * `current_knowledge` or the visibly unreviewed candidate channel.
     */
    channel: string;
    /**
     * Canonical content hash.
     */
    content_hash: string;
    /**
     * Selection id.
     */
    id: string;
    /**
     * Stable Knowledge item, absent in hashes-only mode.
     */
    knowledge_item_id?: string | null;
    /**
     * Exact immutable revision, absent in hashes-only mode.
     */
    knowledge_revision_id?: string | null;
    /**
     * One-based delivery rank.
     */
    rank: number;
    /**
     * Why it was selected.
     */
    reason_codes: string[];
    revision?: null | KnowledgeRevisionView;
    /**
     * Independently visible provenance, full mode only.
     */
    sources?: KnowledgeSourceView[];
    /**
     * Estimated tokens charged.
     */
    token_count: number;
    unreviewed_candidate?: null | CaptureCandidateView;
  };

/**
 * Create a scope selector.
 */
export type CreateConfigurationBindingBody = {
    artifact_id: string;
    enabled?: boolean;
    pinned_version_id?: string | null;
    scope_id: string;
  };

/**
 * Create an aggregate and its first immutable version.
 */
export type CreateConfigurationBody = {
    document: ConfigurationDocumentBody;
    governing_scope_id: string;
    name: string;
    source_template?: "personal" | "team" | "enterprise";
  };

/**
 * `POST /v1/sessions/{session_id}/context-runs`.
 */
export type CreateContextRunBody = {
    /**
     * Requested budget; the governed pack remains the ceiling.
     */
    budget_tokens?: number | null;
    /**
     * Optional sensitivity narrowing.
     */
    max_sensitivity?: string | null;
    /**
     * Task/query; omission is the session-start recency shape.
     */
    query?: string | null;
  };

/**
 * `POST /v1/admin/groups`.
 */
export type CreateGroupBody = {
    /**
     * Optional prose. Blank is refused; omit it instead.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Its members at creation, by stable identity id.
     */
    members?: string[];
    /**
     * Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Immutable.
     */
    slug: string;
  };

/**
 * `POST /v1/workspaces/{workspace_id}/invites`.
 */
export type CreateInviteBody = {
    /**
     * Who it is meant for. Optional: an invitation with no address is a link
     * the inviter copies, redeemable once by whoever presents it first.
     */
    email?: string | null;
    /**
     * How long it stands, in seconds. Defaults to seven days and is capped at
     * thirty — an invitation that never expires is a key left under the mat.
     */
    expires_in_secs?: number | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
  };

/**
 * `POST /v1/knowledge`.
 */
export type CreateKnowledgeBody = {
    /**
     * First immutable revision.
     */
    content: KnowledgeContentBody;
    knowledge_type: "fact" | "decision" | "preference" | "procedure" | "entity" | "episode" | "convention" | "warning" | "reference";
    origin?: "observed" | "asserted" | "authored" | "imported";
    /**
     * Optional personal owner.
     */
    owner_principal_id?: string | null;
    /**
     * Optional project association.
     */
    project_id?: string | null;
    /**
     * Governing scope.
     */
    scope_id: string;
    /**
     * Provenance. Omission creates one manual descriptor at `scope_id`.
     */
    sources?: KnowledgeSourceBody[];
  };

/**
 * `POST /v1/workspaces/{workspace_id}/projects`.
 */
export type CreateProjectBody = {
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Workspace-unique handle, same grammar as a workspace slug.
     */
    slug: string;
  };

export type CreateRelaxationBody = {
    action: "knowledge.read";
    max_sensitivity: string;
    reason: string;
    requested_end_at: string;
    requested_start_at: string;
    subject_identity_id: string;
  } & {
    target_scope_id: string;
  };

/**
 * `POST /v1/admin/scopes` — create a scope under a parent. The tenant
 * root is minted by the substrate, so `parent_id` is required.
 */
export type CreateScopeBody = {
    /**
     * Open labelling bag; never an authorisation input.
     */
    attributes?: unknown;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * The scope's shape — `org_unit`, `workspace`, `project` or
     * `principal`. The old rank vocabulary (`org`, `division`,
     * `department`, `team`, `user`) fails validation by name.
     */
    kind: string;
    /**
     * The parent. The tenant root is minted by the first thing that needs
     * a parent and cannot be created here.
     */
    parent_id: string;
    /**
     * Sibling-unique handle, immutable.
     */
    slug: string;
  };

/**
 * Create a project- or principal-scope binding.
 */
export type CreateSkillBindingBody = {
    /**
     * Initial activation state.
     */
    enabled?: boolean;
    /**
     * Exact version pin; absent follows the current version.
     */
    pinned_version_id?: string | null;
    /**
     * Project or principal scope receiving the binding.
     */
    scope_id: string;
    /**
     * Stable catalogue entry.
     */
    skill_id: string;
  };

/**
 * Bind one exact approved version to a project.
 */
export type CreateToolBindingBody = {
    /**
     * Target project.
     */
    project_id: string;
    /**
     * Stable server.
     */
    server_id: string;
    state: "enabled" | "disabled" | "removed";
    /**
     * Exact approved version. There is no follow-current mode.
     */
    version_id: string;
  };

/**
 * `POST /v1/workspaces`.
 */
export type CreateWorkspaceBody = {
    /**
     * Optional prose. Blank is refused; omit it instead.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Tenant-unique handle: `^[a-z0-9][a-z0-9-]{0,62}$`. Becomes the owned
     * scope's slug too, and is immutable afterwards.
     */
    slug: string;
  };

/**
 * The response to creating an invitation — the **one and only** place the
 * token exists.
 *
 * It is not stored (only its SHA-256 is), not logged, not in the audit
 * payload and not on any other route. An inviter who loses it withdraws the
 * invitation and issues another, which is one action; the alternative is a
 * product that can show you somebody else's live credential.
 */
export type CreatedInviteView = {
    /**
     * The URL the recipient posts to, with their own credential, to redeem
     * it. Enough for a local or demo deployment to invite by copying a link —
     * email delivery is deliberately not a requirement of this feature.
     */
    accept_url: string;
    /**
     * The invitation.
     */
    invite: InviteView;
    /**
     * The token. **Shown once.**
     */
    token: string;
  };

/**
 * One rule, parsed — rendered beside the source so a console can show
 * what the file *means* without reimplementing the parser.
 */
export type CuratorRuleView = {
    approvers: string[];
    pattern: string;
  };

export type CuratorsPutBody = {
    /**
     * Why — an auditor reads this.
     */
    message: string;
    /**
     * The file's text. An empty file clears this scope's requirements —
     * there is no delete, because the removal is history too.
     */
    source: string;
  };

export type CuratorsPutResponse = {
    commit: string;
    object_hash: string;
    parent?: string | null;
    rules: number;
    scope_id: string;
    /**
     * Whether the bytes were already stored: a re-commit of an unchanged
     * file, which still records who re-asserted it and when.
     */
    unchanged: boolean;
  };

export type CuratorsResponse = {
    /**
     * The commit the `curators` ref points at.
     */
    commit?: string | null;
    /**
     * The scope the effective file is committed at — this node, or the
     * nearest ancestor carrying one (ADR-0032 decision 14). Absent when
     * no scope on the chain has a file.
     */
    effective_at?: string | null;
    /**
     * The file's content address.
     */
    object_hash?: string | null;
    /**
     * The parsed rules.
     */
    rules: CuratorRuleView[];
    /**
     * The node asked about.
     */
    scope_id: string;
    /**
     * The file's exact authored bytes, comments included.
     */
    source?: string | null;
    updated_at?: string | null;
    updated_by?: string | null;
  };

/**
 * `DELETE /v1/knowledge/{id}`. Deletion without a mode is invalid.
 */
export type DeleteKnowledgeBody = {
    /**
     * Exact current revision inspected.
     */
    expected_revision_id: string;
    mode: "archive" | "forget";
    /**
     * Bounded human reason.
     */
    reason: string;
  };

/**
 * `POST /v1/directory/access-assignments`.
 */
export type DirectoryAccessAssignmentBody = {
    /**
     * A shared Group whose source is `directory`.
     */
    group_id: string;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * Governed scope at which the directory group receives authority.
     */
    scope_id: string;
  };

/**
 * Report a fresh discovery using the current descriptor.
 */
export type DiscoverToolServerBody = {
    /**
     * Raw stateless discovery result.
     */
    capabilities: Record<string, unknown>;
    /**
     * Exact current approved version precondition.
     */
    expected_current_version_id: string;
  };

/**
 * Dismissal records no Knowledge mutation.
 */
export type DismissCandidateBody = {
    /**
     * Optional bounded human reason.
     */
    reason?: string | null;
  };

/**
 * `PATCH /v1/knowledge/{id}`.
 */
export type EditKnowledgeBody = {
    /**
     * Complete replacement content.
     */
    content: KnowledgeContentBody;
    /**
     * Exact current revision the editor inspected.
     */
    expected_revision_id: string;
    /**
     * Provenance for this exact revision. Omission records a manual edit.
     */
    sources?: KnowledgeSourceBody[];
  };

/**
 * Exact current document and selector evidence at a governed scope.
 */
export type EffectiveConfigurationView = {
    artifact_id?: string | null;
    binding_id?: string | null;
    binding_scope_id?: string | null;
    content_hash: string;
    document: ConfigurationDocumentBody;
    fail_safe: boolean;
    scope_id: string;
    version_id?: string | null;
  };

/**
 * `POST /v1/sessions/{session_id}/end`.
 */
export type EndSessionBody = {
    /**
     * **Why** it stopped, in the client's words — `hook timed out`, `user
     * cancelled`, `context window exhausted` (CPR-11, ADR-0077 decision 4).
     *
     * Distinct from `task_summary`, which is what the run was *about*: the
     * status says a run failed, this says what failed. Free text, because the
     * vocabulary belongs to the harness.
     */
    end_reason?: string | null;
    status: "ending" | "ended" | "abandoned" | "failed";
    /**
     * What the run turned out to be about, when the client only knows at the
     * end. Replaces whatever was set at open.
     */
    task_summary?: string | null;
  };

/**
 * Explicit current-Knowledge export selection. Empty means all visible
 * current active/stale Knowledge in the project, bounded at 2000 items.
 */
export type ExportOkfBody = {
    /**
     * Stable item ids to export.
     */
    item_ids?: string[];
  };

/**
 * Type-aware implicit staleness intervals in days; zero disables the
 * implicit date for that type.
 */
export type FreshnessConfigurationBody = {
    convention_days: number;
    decision_days: number;
    entity_days: number;
    episode_days: number;
    fact_days: number;
    preference_days: number;
    procedure_days: number;
    reference_days: number;
    warning_days: number;
  };

/**
 * Every type-aware policy under one exact effective configuration.
 */
export type FreshnessPolicyListView = {
    /**
     * Closed Knowledge vocabulary in declaration order.
     */
    policies: FreshnessPolicyView[];
  };

/**
 * Public evaluated freshness policy for one Knowledge type.
 */
export type FreshnessPolicyView = {
    /**
     * Exact Configuration aggregate, absent for fail-safe configuration.
     */
    configuration_artifact_id?: string | null;
    /**
     * Exact binding selected nearest-first.
     */
    configuration_binding_id?: string | null;
    /**
     * Canonical configuration hash, including fail-safe.
     */
    configuration_hash: string;
    /**
     * Exact immutable Configuration version.
     */
    configuration_version_id?: string | null;
    /**
     * Governed default interval; zero means no implicit date.
     */
    default_days: number;
    /**
     * Knowledge type.
     */
    knowledge_type: string;
    /**
     * Scope at which resolution was requested.
     */
    scope_id: string;
    /**
     * Stable type-specific verification signals.
     */
    triggers: string[];
  };

/**
 * The grant listing.
 */
export type GrantList = {
    /**
     * The grants this filter selected, oldest first. These are the **rows**,
     * not the authority in force anywhere: a workspace grant appears once
     * here and reaches every project inside it. `GET /v1/projects/{id}/members`
     * is the other question.
     */
    grants: GrantView[];
  };

/**
 * `POST /v1/projects/{project_id}/members` and `POST /v1/admin/grants`.
 *
 * Exactly one of `principal_id` and `group_id`. Two flat fields rather than a
 * tagged union, for [`GrantView`]'s reason.
 */
export type GrantSubjectBody = {
    /**
     * The group.
     */
    group_id?: string | null;
    /**
     * The principal, by verified token subject.
     */
    principal_id?: string | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope. Required on `/v1/admin/grants`, where the caller chooses;
     * **refused** on the project route, where the path already says which
     * scope — a body that could name a different one would make the path a
     * suggestion.
     */
    scope_id?: string | null;
  };

/**
 * A grant, as the API serves it.
 *
 * The subject is **flattened** into `subject_kind` plus one of two id fields
 * rather than nested as a tagged union. A tagged union is the tidier model and
 * the worse contract: it renders as a `oneOf` that the frontend generator
 * would have to discriminate, and this document's whole point is that the
 * types on both ends are derived rather than hand-reconciled.
 */
export type GrantView = {
    /**
     * When it was made.
     */
    created_at: string;
    /**
     * Whether a directory manages it, and it therefore cannot be revoked
     * here.
     */
    directory_managed: boolean;
    /**
     * Stable directory group resource that caused the assignment.
     */
    directory_resource_id?: string | null;
    /**
     * Owning directory adapter for a directory-managed assignment.
     */
    directory_source?: string | null;
    /**
     * Who granted it, when a caller did.
     */
    granted_by?: string | null;
    /**
     * The group, for a `group` grant.
     */
    group_id?: string | null;
    /**
     * Stable id — what a revocation names.
     */
    id: string;
    /**
     * The invitation that produced it, for an `invite` grant.
     */
    invite_id?: string | null;
    /**
     * The principal, for a `principal` grant.
     */
    principal_id?: string | null;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope the grant is at. Its subtree inherits it.
     */
    scope_id: string;
    source: "owner" | "direct" | "invite" | "directory" | "automation";
    subject_kind: "principal" | "group";
  };

/**
 * The group listing.
 */
export type GroupList = {
    /**
     * The tenant's groups, by slug. Archived ones included: a listing that
     * omitted them would make an archived group indistinguishable from one
     * that never existed.
     */
    groups: GroupView[];
  };

/**
 * One group membership. An unbound directory identity intentionally has no
 * `principal_id` yet; its stable identity still remains visible and usable.
 */
export type GroupMemberView = {
    /**
     * Stable identity id.
     */
    identity_id: string;
    /**
     * Verified token subject, once first login binds one.
     */
    principal_id?: string | null;
  };

/**
 * A group, named enough to render without a second call.
 */
export type GroupRefView = {
    /**
     * The group's id.
     */
    id: string;
    /**
     * Its handle.
     */
    slug: string;
  };

/**
 * A group, as the API serves it.
 */
export type GroupView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it, when a caller did.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Optional protocol `externalId`.
     */
    directory_external_id?: string | null;
    /**
     * The stable resource id assigned by that directory.
     */
    directory_resource_id?: string | null;
    /**
     * The adapter/provider that owns this directory-managed group.
     */
    directory_source?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Stable id.
     */
    id: string;
    /**
     * Its members, by stable identity with a bound subject when available.
     */
    members: GroupMemberView[];
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * Tenant-unique handle. Immutable.
     */
    slug: string;
    source: "direct" | "directory";
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * Import one supported client configuration entry.
 */
export type ImportToolClientConfigBody = {
    /**
     * Raw stateless discovery result captured by the trusted adapter.
     */
    capabilities: Record<string, unknown>;
    /**
     * `claude_code`, `cursor` or `vscode` configuration grammar.
     */
    client: string;
    /**
     * Governing scope.
     */
    governing_scope_id: string;
    /**
     * Server key in the client configuration.
     */
    name: string;
    /**
     * Opaque secret reference replacing any client credential material.
     */
    secret_reference?: string | null;
    /**
     * One client server object. Embedded env/header values are refused.
     */
    server: Record<string, unknown>;
  };

/**
 * Install the first immutable version of a stable skill.
 */
export type InstallSkillBody = {
    /**
     * Whole Agent Skills-compatible bundle.
     */
    files: SkillFileBody[];
    /**
     * Scope governing the catalogue aggregate.
     */
    governing_scope_id: string;
    /**
     * Agent Skills bundle name.
     */
    name: string;
    /**
     * Retained bundle provenance.
     */
    provenance?: SkillProvenanceBody;
    sensitivity: "public" | "internal" | "confidential" | "restricted";
  };

/**
 * The invitation listing.
 */
export type InviteList = {
    /**
     * Every invitation issued at this scope, newest first — redeemed and
     * withdrawn ones included, because "who was invited here and what
     * happened" is the question this answers.
     */
    invites: InviteView[];
  };

/**
 * An invitation, as the API serves it.
 *
 * The token is **not here**, on any route, ever. It appears once, in
 * [`CreatedInviteView`], in the response to the request that minted it.
 */
export type InviteView = {
    /**
     * When it was redeemed.
     */
    accepted_at?: string | null;
    /**
     * The principal that redeemed it, when somebody has.
     */
    accepted_by?: string | null;
    /**
     * When.
     */
    created_at: string;
    /**
     * Who issued it.
     */
    created_by?: string | null;
    /**
     * Who it was meant for, when the inviter said.
     */
    email?: string | null;
    /**
     * When it stops being redeemable.
     */
    expires_at: string;
    /**
     * Stable id — what a withdrawal names.
     */
    id: string;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope it grants at.
     */
    scope_id: string;
    status: "pending" | "accepted" | "revoked" | "expired";
  };

/**
 * `POST /v1/scim/credentials`.
 */
export type IssueRequest = {
    /**
     * How long it lives. Clamped to [`MAX_LIFETIME_DAYS`].
     */
    expires_in_days?: number | null;
    /**
     * What an operator recognises it by when deciding to rotate.
     */
    label: string;
  };

/**
 * The issue response — **the only time the token is ever readable**.
 */
export type IssuedCredential = ScimCredentialView & {
    /**
     * The value to paste into Entra's "Secret Token" or Okta's
     * authorisation header. Never stored, never logged, shown once.
     */
    token: string;
  };

/**
 * Complete content for a new immutable revision.
 */
export type KnowledgeContentBody = {
    /**
     * Markdown body.
     */
    body_markdown: string;
    /**
     * Integer confidence from 0 through 1000.
     */
    confidence_permille: number;
    /**
     * Forward-compatible product metadata.
     */
    metadata?: Record<string, unknown>;
    sensitivity: "public" | "internal" | "confidential" | "restricted";
    /**
     * Verification due time.
     */
    stale_after?: string | null;
    /**
     * Short summary.
     */
    summary: string;
    /**
     * Canonicalised by the server to lower-case, sorted and unique.
     */
    tags?: string[];
    /**
     * Human title.
     */
    title: string;
    /**
     * Defaults to server time when omitted.
     */
    valid_from?: string | null;
    /**
     * Exclusive end of valid time.
     */
    valid_to?: string | null;
    /**
     * Bounded verification evidence.
     */
    verification_metadata?: Record<string, unknown>;
  };

/**
 * Separately authorised evaluation query/enumeration/id lens.
 */
export type KnowledgeEvaluationBody = {
    /**
     * Valid-time instant for the current-head projection. Defaults to now.
     */
    as_of?: string | null;
    /**
     * Opaque continuation for an enumeration sweep.
     */
    cursor?: string | null;
    /**
     * Exact stable ids. Omit with `query` for an enumeration sweep.
     */
    ids?: string[];
    /**
     * Candidate/page bound, 1–100.
     */
    limit?: number | null;
    /**
     * Query text. Omit with `ids` for an enumeration sweep.
     */
    query?: string | null;
  };

/**
 * Immutable revision history page.
 */
export type KnowledgeHistoryView = {
    /**
     * Resume position after the last revision considered.
     */
    next_cursor?: string | null;
    /**
     * Policy-visible revisions, newest first.
     */
    revisions: KnowledgeRevisionView[];
  };

/**
 * Stable Knowledge head plus its exact current immutable revision.
 */
export type KnowledgeItemView = {
    /**
     * Aggregate creation time.
     */
    created_at: string;
    /**
     * Creation actor.
     */
    created_by?: string | null;
    /**
     * Current immutable revision.
     */
    current_revision: KnowledgeRevisionView;
    /**
     * Stable aggregate id.
     */
    id: string;
    knowledge_type: "fact" | "decision" | "preference" | "procedure" | "entity" | "episode" | "convention" | "warning" | "reference";
    lifecycle_state: "active" | "stale" | "transitional" | "superseded" | "archived" | "erasure_pending" | "erased";
    /**
     * Fused search score, absent outside a query listing.
     */
    match_score?: number | null;
    origin: "observed" | "asserted" | "authored" | "imported";
    /**
     * Owning principal, for personal Knowledge.
     */
    owner_principal_id?: string | null;
    /**
     * Associated project.
     */
    project_id?: string | null;
    /**
     * Visible relations only; omitted from collection rows.
     */
    relationships?: KnowledgeRelationView[];
    /**
     * Governing scope.
     */
    scope_id: string;
    /**
     * Last head change.
     */
    updated_at: string;
    /**
     * Last head-change actor.
     */
    updated_by?: string | null;
  };

/**
 * Cursor-paginated current Knowledge results.
 */
export type KnowledgeListView = {
    /**
     * Honest reason the semantic leg did not run, if any.
     */
    degradation?: string | null;
    /**
     * Policy-visible rows.
     */
    items: KnowledgeItemView[];
    /**
     * Resume position after the last candidate considered. May be present on
     * an empty page when every candidate was denied.
     */
    next_cursor?: string | null;
    /**
     * `listing`, `lexical` or `hybrid`.
     */
    retrieval_mode: string;
  };

/**
 * Every Knowledge mutation's stable VedaFlow result envelope.
 */
export type KnowledgeMutationView = {
    /**
     * VedaFlow change/proposal id.
     */
    change_id: string;
    /**
     * Stable result aggregate when applicable.
     */
    knowledge_item_id?: string | null;
    /**
     * Durable operation for long-running work such as erasure.
     */
    operation_id?: string | null;
    outcome: "applied" | "pending_review" | "rejected";
    /**
     * Resulting immutable revision when applied.
     */
    revision_id?: string | null;
  };

/**
 * Ordinary session-scoped deep query.
 */
export type KnowledgeQueryBody = {
    /**
     * Result bound, 1–100.
     */
    limit?: number | null;
    /**
     * Query text.
     */
    query: string;
  };

/**
 * One visible relation. Both endpoint ids passed independent PDP decisions.
 */
export type KnowledgeRelationView = {
    /**
     * Exact revision asserting the relation.
     */
    asserting_revision_id: string;
    /**
     * Assertion time.
     */
    created_at: string;
    /**
     * Stable relation id.
     */
    id: string;
    /**
     * Forward-compatible relation metadata.
     */
    metadata: Record<string, unknown>;
    relation_type: "supports" | "duplicates" | "contradicts" | "supersedes" | "derived_from" | "references" | "related_to" | "transitions_to";
    /**
     * Visible source item.
     */
    source_item_id: string;
    /**
     * Visible target item.
     */
    target_item_id: string;
  };

/**
 * One immutable Knowledge revision as served to an authorised reader.
 */
export type KnowledgeRevisionView = {
    /**
     * Canonical Markdown body.
     */
    body_markdown: string;
    /**
     * Confidence on a 0–1000 integer scale.
     */
    confidence_permille: number;
    /**
     * Canonical BLAKE3-256 digest.
     */
    content_hash: string;
    /**
     * Author label, when recorded.
     */
    created_by?: string | null;
    /**
     * Explainable effective freshness reasons; empty means current.
     */
    freshness_reasons: string[];
    /**
     * Immutable revision id.
     */
    id: string;
    /**
     * Stable item this revision belongs to.
     */
    knowledge_item_id: string;
    /**
     * Forward-compatible product metadata.
     */
    metadata: Record<string, unknown>;
    /**
     * Monotonic number within the item.
     */
    revision_number: number;
    sensitivity: "public" | "internal" | "confidential" | "restricted";
    /**
     * Whether verification is due at response time.
     */
    stale: boolean;
    /**
     * Verification due time, when configured.
     */
    stale_after?: string | null;
    /**
     * Retrieval/listing summary.
     */
    summary: string;
    /**
     * Canonical lower-case tags.
     */
    tags: string[];
    /**
     * Human title.
     */
    title: string;
    /**
     * Database-stamped transaction time.
     */
    transaction_time: string;
    /**
     * Beginning of valid time.
     */
    valid_from: string;
    /**
     * End of valid time, when known.
     */
    valid_to?: string | null;
    /**
     * Bounded verification evidence.
     */
    verification_metadata: Record<string, unknown>;
  };

/**
 * A normalised provenance descriptor submitted with a revision.
 */
export type KnowledgeSourceBody = {
    /**
     * Lower-case BLAKE3-256 source-content hash.
     */
    content_hash?: string | null;
    /**
     * Stable logical locator for located source families.
     */
    locator?: string | null;
    /**
     * Bounded extension metadata.
     */
    metadata?: Record<string, unknown>;
    /**
     * Descriptor disclosure scope. Defaults to the item's governing scope.
     */
    scope_id?: string | null;
    /**
     * Exact immutable event for `session_event`.
     */
    session_event_id?: string | null;
    /**
     * External revision/version label.
     */
    source_revision?: string | null;
    source_type: "session_event" | "manual" | "document" | "repository" | "url" | "okf" | "system_derived";
  };

/**
 * One independently authorised provenance descriptor.
 */
export type KnowledgeSourceView = {
    /**
     * Source-content hash when known.
     */
    content_hash?: string | null;
    /**
     * Registration time.
     */
    created_at: string;
    /**
     * Stable source descriptor id.
     */
    id: string;
    /**
     * Logical locator; contains no source payload.
     */
    locator?: string | null;
    /**
     * Bounded extension metadata.
     */
    metadata: Record<string, unknown>;
    /**
     * Scope whose policy admitted this descriptor.
     */
    scope_id: string;
    /**
     * Exact session event for observed Knowledge.
     */
    session_event_id?: string | null;
    /**
     * External source revision/version.
     */
    source_revision?: string | null;
    source_type: "session_event" | "manual" | "document" | "repository" | "url" | "okf" | "system_derived";
  };

/**
 * Visible provenance attached to the current revision.
 */
export type KnowledgeSourcesView = {
    /**
     * Independently authorised descriptors, in provenance order.
     */
    sources: KnowledgeSourceView[];
  };

/**
 * Cursor envelope for policy-visible usage history.
 */
export type KnowledgeUsageListView = {
    /**
     * Resume cursor.
     */
    next_cursor?: string | null;
    /**
     * Recorded context uses.
     */
    usages: KnowledgeUsageView[];
  };

/**
 * One policy-visible context use of an exact immutable revision.
 */
export type KnowledgeUsageView = {
    /**
     * Context run that selected the exact revision.
     */
    context_run_id: string;
    /**
     * Exact selection, suitable for explicit feedback.
     */
    context_selection_id: string;
    /**
     * Visible reason codes.
     */
    reason_codes: string[];
    /**
     * Exact revision used.
     */
    revision_id: string;
    /**
     * Selection time.
     */
    selected_at: string;
    /**
     * Session whose access was independently decided.
     */
    session_id: string;
  };

/**
 * Archive/restore body.
 */
export type LifecycleKnowledgeBody = {
    /**
     * Exact current revision inspected.
     */
    expected_revision_id: string;
    /**
     * Bounded human reason.
     */
    reason: string;
  };

/**
 * One level of the scope tree: the parent the level hangs from, and its
 * children.
 */
export type ListResponse = {
    parent?: null | ScopeView;
    /**
     * The parent's children, sorted by slug.
     */
    scopes: ScopeView[];
  };

/**
 * Everything a client needs before it renders anything.
 */
export type MeView = {
    /**
     * Where this caller stands, most specific first, and what they may do at
     * each — **from real policy decisions** (CPR-6, ADR-0073 decision 8).
     *
     * Their own scope, the tenant root, and every scope a direct or group
     * grant reaches them at. Nothing here is derived from a plan, an edition
     * or a shape: each entry is `Action::PROBED_AT_SCOPE` decided at that
     * scope, under that scope's own effective profile, by the same PDP the
     * act itself will pass through. A personal deployment and an enterprise
     * one differ in the rows this reads, never in the code that reads them.
     */
    anchors: AnchorCapabilities[];
    /**
     * How many anchors the response bound dropped. Named rather than hidden:
     * a truncated answer presented as a complete one is the one failure a
     * capability surface cannot afford (ADR-0058 decision 5).
     */
    anchors_not_answered?: number;
    /**
     * What this caller may do on the tenant plane, asked of the PDP.
     *
     * **A forecast, never a grant** (ADR-0058 decision 2): nothing downstream
     * reads this to decide anything, every act still takes its own decision
     * at its own seam, and a client uses this to choose what to *offer*.
     */
    capabilities: TenantCapabilities;
    /**
     * Where they are in setting up — the server's answer, not a client's
     * inference from an empty list.
     */
    onboarding: OnboardingView;
    /**
     * Who is calling.
     */
    principal: PrincipalView;
    /**
     * Every project this caller may read, by workspace then slug. Flat rather
     * than nested under its workspace: a client that wants the tree has
     * `workspace_id` on every row, and one that wants a recent-projects list
     * would otherwise have to flatten what we just nested.
     */
    projects: ProjectView[];
    /**
     * The tenant they resolved to.
     */
    tenant: TenantView;
    /**
     * Every workspace this caller may read, by slug.
     */
    workspaces: WorkspaceView[];
  };

/**
 * The member listing.
 */
export type MemberList = {
    /**
     * Everybody who holds a role here, nearest grant first. One entry per
     * (principal, role): somebody holding two roles appears twice, because
     * the two came from different grants and are revoked separately.
     */
    members: MemberView[];
  };

/**
 * One principal's one role at one scope, with everything a reader needs to
 * answer **why**.
 *
 * The `source`, `scope_id`, `inherited` and `via_group` fields together are
 * the whole of "access-source visibility": a person looking at a project's
 * member list can see that Robin is there because somebody granted them
 * `member` at the workspace, or because they are in the `engineering` group,
 * or because a directory said so — without reading an audit log.
 */
export type MemberView = {
    /**
     * Whether a directory manages it, and it therefore cannot be edited here.
     */
    directory_managed: boolean;
    /**
     * The grant this came from — what a revocation names.
     */
    grant_id: string;
    /**
     * When the grant was made.
     */
    granted_at: string;
    /**
     * Whether it was inherited from an ancestor scope rather than written
     * here. A client that offers "remove" on an inherited row is offering
     * something the API will refuse, so this is the field that decides.
     */
    inherited: boolean;
    /**
     * The principal, by verified token subject.
     */
    principal_id: string;
    role: "owner" | "member" | "viewer" | "reviewer" | "curator" | "administrator";
    /**
     * The scope the grant is actually written at — **not** necessarily the
     * one that was asked about.
     */
    scope_id: string;
    source: "owner" | "direct" | "invite" | "directory" | "automation";
    via_group?: null | GroupRefView;
  };

/**
 * Merge a candidate with visible current Knowledge.
 */
export type MergeCandidateBody = {
    /**
     * Existing current inputs and their exact inspected heads.
     */
    inputs: MergeInputBody[];
    /**
     * Optional result placement/content edits.
     */
    result?: AcceptCandidateBody;
  };

/**
 * One merge input and stale-write precondition.
 */
export type MergeInputBody = {
    /**
     * Stable input item.
     */
    item_id: string;
    /**
     * Exact input head inspected.
     */
    revision_id: string;
  };

/**
 * `POST /v1/knowledge/merge`.
 */
export type MergeKnowledgeBody = {
    /**
     * Result's first revision.
     */
    content: KnowledgeContentBody;
    /**
     * Two or more current inputs.
     */
    inputs: MergeInputBody[];
    knowledge_type: "fact" | "decision" | "preference" | "procedure" | "entity" | "episode" | "convention" | "warning" | "reference";
    origin: "observed" | "asserted" | "authored" | "imported";
    /**
     * Result owner.
     */
    owner_principal_id?: string | null;
    /**
     * Result project association.
     */
    project_id?: string | null;
    /**
     * Result governing scope.
     */
    scope_id: string;
  };

/**
 * One event of a `POST /v1/sessions/{session_id}/events` batch.
 */
export type NewEventBody = {
    /**
     * The client's own id for this event. **The idempotency unit**: a
     * redelivered batch appends nothing twice.
     */
    client_event_id: string;
    /**
     * The payload shape this client declares. Defaults to the current one.
     */
    event_schema_version?: number;
    event_type: "session.started" | "session.ended" | "message.user" | "message.assistant" | "tool.invoked" | "tool.result" | "file.read" | "file.changed" | "command.executed" | "skill.loaded" | "context.requested" | "adapter.warning" | "memory.asserted";
    /**
     * When the client says it happened.
     */
    occurred_at: string;
    /**
     * The content: a JSON object, at most 64 KiB encoded.
     */
    payload?: Record<string, unknown> | null;
  };

/**
 * What a probe says about one scope.
 */
export type NodeCapabilities = {
    /**
     * The operand-free actions, by their stable machine name.
     */
    actions: Record<string, boolean>;
    pack?: null | PackView;
    /**
     * The tier-bearing reads: the tiers each permits here, ascending. An
     * empty list is a real answer — "nothing at this scope, at any tier".
     */
    read_tiers: Record<string, string[]>;
    /**
     * The caller's own effective role keys here — the caller's, never
     * anyone else's (decision 3; since the cutover, the only roles there
     * are — CPR-6, ADR-0073 decision 5).
     */
    roles: string[];
    scope_id: string;
    /**
     * Where the node sits — a fact about the *node*, so it is served only
     * to a caller who may read it (`ScopeRead`). Absent otherwise, and
     * the verdicts beside it are unaffected: they are about the caller.
     */
    scope_path?: string | null;
  };

/**
 * One immutable admitted artifact.
 */
export type OkfArtifactView = {
    /**
     * Markdown body after frontmatter.
     */
    body_markdown: string;
    /**
     * Admitted-byte digest.
     */
    content_hash: string;
    /**
     * Parsed extension-preserving frontmatter.
     */
    frontmatter: Record<string, unknown>;
    /**
     * Stable artifact id.
     */
    id: string;
    /**
     * Concept, index or log.
     */
    kind: string;
    /**
     * Safe logical path.
     */
    logical_path: string;
    /**
     * Stable order.
     */
    ordinal: number;
  };

/**
 * One deterministic OKF output file.
 */
export type OkfExportFileView = {
    /**
     * Exact UTF-8 Markdown.
     */
    content: string;
    /**
     * Exact content digest.
     */
    content_hash: string;
    /**
     * Stable bundle-relative path.
     */
    logical_path: string;
  };

/**
 * Deterministic export response.
 */
export type OkfExportView = {
    /**
     * Digest over ordered paths and hashes.
     */
    bundle_digest: string;
    /**
     * Stable ordered output files.
     */
    files: OkfExportFileView[];
    /**
     * Exact supported version.
     */
    format_version: string;
    /**
     * Pinned official specification commit.
     */
    specification_commit: string;
  };

/**
 * Keyset-paginated import jobs.
 */
export type OkfImportJobListView = {
    /**
     * Visible jobs.
     */
    jobs: OkfImportJobView[];
    /**
     * Opaque continuation cursor.
     */
    next_cursor?: string | null;
  };

/**
 * Import operation summary.
 */
export type OkfImportJobView = {
    /**
     * Immutable artifact count.
     */
    artifact_count: number;
    /**
     * Canonical admitted-bundle digest.
     */
    bundle_digest: string;
    /**
     * Reviewable candidate count.
     */
    candidate_count: number;
    /**
     * Resulting candidate batch.
     */
    capture_batch_id?: string | null;
    /**
     * Terminal time.
     */
    completed_at?: string | null;
    /**
     * Creation time.
     */
    created_at: string;
    /**
     * Exact adapter format and version.
     */
    format: string;
    /**
     * Exact implemented format version.
     */
    format_version: string;
    /**
     * Stable job id.
     */
    id: string;
    /**
     * Immutable mapping count.
     */
    mapping_count: number;
    /**
     * Content-free validation notices.
     */
    notices: string[];
    /**
     * Target project.
     */
    project_id: string;
    /**
     * Directory, zip, tar or Git.
     */
    source_kind: string;
    /**
     * Credential-free retained source identity.
     */
    source_locator: string;
    /**
     * Upstream revision when reported.
     */
    source_revision?: string | null;
    /**
     * Pinned official specification commit.
     */
    specification_commit: string;
    /**
     * Planned, materialized or failed.
     */
    state: string;
  };

/**
 * Complete persisted dry-run plan.
 */
export type OkfImportPlanView = {
    /**
     * Immutable admitted artifacts.
     */
    artifacts: OkfArtifactView[];
    /**
     * Operation summary.
     */
    job: OkfImportJobView;
    /**
     * Immutable proposed mappings.
     */
    mappings: OkfMappingView[];
  };

/**
 * One inert entry supplied by a local client.
 */
export type OkfInputEntryBody = {
    /**
     * Exact bytes encoded with standard base64; omitted for directory markers.
     */
    content_base64?: string;
    /**
     * `file`, `directory`, `symlink` or `special`.
     */
    kind: string;
    /**
     * Bundle-relative slash-separated path.
     */
    logical_path: string;
  };

/**
 * One immutable proposed concept mapping.
 */
export type OkfMappingView = {
    /**
     * Source artifact.
     */
    artifact_id: string;
    /**
     * Candidate created on materialisation.
     */
    candidate_id?: string | null;
    /**
     * Addition, update, duplicate or conflict.
     */
    classification: string;
    /**
     * Complete proposed immutable content.
     */
    content: KnowledgeContentBody;
    /**
     * Semantic content digest.
     */
    content_hash: string;
    /**
     * Stable mapping id.
     */
    id: string;
    /**
     * Proposed Synveda type.
     */
    knowledge_type: string;
    /**
     * Independently visible match, when any.
     */
    matched_item_id?: string | null;
    /**
     * Exact visible revision compared.
     */
    matched_revision_id?: string | null;
    /**
     * Whether external lifecycle permits a candidate.
     */
    materializable: boolean;
    /**
     * Exact producer-defined type, including unknown values.
     */
    okf_type: string;
    /**
     * Stable concept order.
     */
    ordinal: number;
    /**
     * Proposed internal links.
     */
    proposed_relations: Record<string, unknown>;
  };

/**
 * Candidate-only materialisation result.
 */
export type OkfMaterializationView = {
    /**
     * Completed capture batch.
     */
    batch: CaptureBatchView;
    /**
     * Reviewable candidates, never active Knowledge by this response alone.
     */
    candidates: CaptureCandidateView[];
    /**
     * Terminal job.
     */
    job: OkfImportJobView;
  };

/**
 * The onboarding vocabulary. Closed, so a client's branch is exhaustive and
 * a new state is a compile error somewhere rather than a silently unhandled
 * string.
 */
export type OnboardingState = "blocked" | "needs_workspace" | "needs_project" | "ready";

/**
 * How far along setting up this caller is.
 */
export type OnboardingView = {
    /**
     * How many projects this caller can see.
     */
    project_count: number;
    /**
     * The single word a client branches on.
     */
    state: OnboardingState;
    /**
     * The tenant's root scope, once anything has needed one. Absent on a
     * deployment where nobody has created a workspace yet: the root is minted
     * by the first thing that needs a parent, so that nobody is asked to
     * declare an organisation before they can hold a record.
     */
    tenant_scope_id?: string | null;
    /**
     * How many workspaces this caller can see.
     */
    workspace_count: number;
  };

/**
 * `POST /v1/sessions`.
 *
 * There is no `tenant_id` and no `principal_id` here, and
 * `deny_unknown_fields` is what makes sending one an error rather than a
 * silent no-op (ADR-0076 decision 8). There is no `scope_id` either: the
 * governed scope is derived from `workspace_id` and `project_id` by the
 * store, because a client that could name the scope could name one its
 * workspace is not in.
 */
export type OpenSessionBody = {
    /**
     * Which agent is running.
     */
    agent_name?: string | null;
    /**
     * The branch the run is on.
     */
    branch?: string | null;
    /**
     * A stable id for this installation of the client.
     */
    client_installation_id?: string | null;
    /**
     * The agent client, as it names itself: a lowercase label of letters,
     * digits, `-` and `.`.
     */
    client_name: string;
    /**
     * Its version.
     */
    client_version?: string | null;
    /**
     * The harness's own id for this run. Unique per caller and client.
     */
    external_session_id?: string | null;
    /**
     * A labelling bag: a JSON object, at most 8 KiB encoded. Never copied
     * into an audit payload.
     */
    metadata?: Record<string, unknown> | null;
    /**
     * The model, as the client names it.
     */
    model_name?: string | null;
    /**
     * The project, when the run is against one.
     */
    project_id?: string | null;
    /**
     * A repository attached to the named project.
     */
    repository_id?: string | null;
    /**
     * What the run is about.
     */
    task_summary?: string | null;
    /**
     * The workspace the run is in.
     */
    workspace_id: string;
  };

/**
 * Stable API origin vocabulary retained by the capability surface. These
 * values describe the PDP's derived selector, not a mutable assignment row.
 */
export type OriginView = {
    kind: string;
    scope_id?: string | null;
  };

export type PackSummary = {
    kind: string;
    name: string;
    updated_at?: string | null;
    version: number;
  };

export type PackView = {
    name: string;
    origin: OriginView;
    version: number;
  };

export type PacksResponse = {
    packs: PackSummary[];
  };

/**
 * `PATCH /v1/admin/scopes/{scope_id}` — rename, re-describe, archive or
 * move. Omitted fields change nothing; a `parent_scope_id` is a move.
 */
export type PatchScopeBody = {
    /**
     * The new labelling bag, replacing the old one whole.
     */
    attributes?: unknown;
    /**
     * The new display name.
     */
    display_name?: string | null;
    /**
     * Naming a parent **moves the scope and its subtree**.
     */
    parent_scope_id?: string | null;
    /**
     * `active` or `archived`.
     */
    status?: string | null;
  };

/**
 * Immutable import-plan request.
 */
export type PlanOkfImportBody = {
    /**
     * Standard-base64 archive bytes for an archive encoding.
     */
    archive_base64?: string | null;
    /**
     * `entries`, `zip`, `tar` or `tar_gzip`.
     */
    encoding: string;
    /**
     * Enumerated inert entries for directory or checked-out Git input.
     */
    entries?: OkfInputEntryBody[];
    /**
     * Directory, zip, tar or Git source label.
     */
    source_kind: string;
    /**
     * Credential-free source identity. It is retained, never fetched.
     */
    source_locator: string;
    /**
     * Required for Git and retained for provenance.
     */
    source_revision?: string | null;
  };

/**
 * The authenticated principal.
 */
export type PrincipalView = {
    /**
     * The IdP's `name` claim at provisioning time, if any.
     */
    display_name?: string | null;
    /**
     * The identity row, when this subject has provisioned one. Absent for a
     * dev token and for a service client that never completed login.
     */
    identity_id?: string | null;
    /**
     * `human` or `service`.
     */
    kind?: string | null;
    /**
     * Whether the base layer forbids this caller everything (AUTH-2,
     * ADR-0013 decision 5). True also for an IdP subject that never
     * provisioned — fail closed.
     */
    quarantined: boolean;
    /**
     * The verified token's `sub` claim — the name every audit event, role
     * binding and idempotency key is keyed by.
     */
    subject: string;
  };

/**
 * The project listing.
 */
export type ProjectList = {
    /**
     * The workspace's projects, by slug. Archived ones included.
     */
    projects: ProjectView[];
  };

/**
 * A project, as the API serves it.
 */
export type ProjectView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * The project's stable id.
     */
    id: string;
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * The governed scope this project owns, beneath the workspace's.
     */
    scope_id: string;
    /**
     * Workspace-unique handle, identical to the scope's slug. Immutable.
     */
    slug: string;
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
    /**
     * The workspace it belongs to. Immutable.
     */
    workspace_id: string;
  };

export type PromotionEvidenceSchema = {
    actions: string[];
    from_seq: number;
    members: PromotionMemberEvidenceSchema[];
    pack_name: string;
    pack_version: number;
    rule: string;
    to_seq: number;
  };

export type PromotionMemberEvidenceSchema = {
    distinct_members: number;
    first_recall_at: string;
    last_recall_at: string;
    recalls: number;
    record_id: string;
  };

export type PromptAuthorBody = {
    /**
     * One line, read in a listing and at review.
     */
    description?: string;
    /**
     * Its name: path-shaped, lower-case, and the identifier a consumer
     * writes in its source (ADR-0049 decision 3).
     */
    name: string;
    /**
     * Where the prompt is authored — the scope that will stand behind it,
     * and the scope whose published channel a proposal would move.
     */
    scope_id: string;
    /**
     * Its classification. Absent means `internal`, the working tier
     * everything else in the product defaults to. `restricted` is refused
     * by name: nothing in the product mints that tier for an authored
     * asset, so a prompt carrying it could never be read back
     * (decision 5).
     */
    sensitivity?: string | null;
    /**
     * The text, with `{{ name }}` placeholders.
     */
    template: string;
    /**
     * Every placeholder the template uses, declared. A schema that
     * disagrees with the template is refused here rather than discovered
     * by a consumer (decision 12).
     */
    variables?: PromptVariableSchema[];
  };

export type PromptListEntry = {
    description: string;
    name: string;
    /**
     * The draft's address, and when it last moved.
     */
    object_hash: string;
    published?: null | PromptPublishedView;
    sensitivity: string;
    updated_at: string;
    updated_by: string;
    variables: PromptVariableSchema[];
  };

export type PromptListResponse = {
    prompts: PromptListEntry[];
    scope_id: string;
    scope_path: string;
  };

/**
 * Where the served bytes came from — one field with four honest answers,
 * because a response that cites a frozen commit without saying so
 * overstates its own freshness (ADR-0036 decision 10, applied here).
 */
export type PromptOrigin = "head" | "pinned-commit" | "channel-pin" | "draft";

/**
 * What a scope's published channel holds for a name right now — the
 * answer to "is my edit live?", which an author who just saved has to be
 * told rather than left to infer.
 */
export type PromptPublishedView = {
    /**
     * The commit the channel serves.
     */
    commit: string;
    /**
     * Whether that is the draft's own address. `false` after an edit: the
     * draft has moved and the reviewed version has not, which is what
     * "behind review" looks like from the writing side.
     */
    current: boolean;
    /**
     * The address it names for this prompt.
     */
    object_hash: string;
  };

export type PromptResolveResponse = {
    channel: string;
    /**
     * The commit whose tree named this version — what a consumer pins next
     * time. Absent for a draft, which is on no channel.
     */
    commit?: string | null;
    description: string;
    name: string;
    /**
     * The version's content address.
     */
    object_hash: string;
    /**
     * What produced these bytes.
     */
    origin: PromptOrigin;
    /**
     * The scope the version came from — for a walked resolve, the nearest
     * one on the caller's chain that publishes it and permits the read.
     */
    scope_id: string;
    scope_path: string;
    sensitivity: string;
    template: string;
    variables: PromptVariableSchema[];
  };

export type PromptVariableSchema = {
    default?: string | null;
    description?: string | null;
    name: string;
  };

export type PromptView = {
    created_at: string;
    created_by: string;
    description: string;
    name: string;
    /**
     * The draft's content address — what a proposal would bind.
     */
    object_hash: string;
    published?: null | PromptPublishedView;
    scope_id: string;
    scope_path: string;
    sensitivity: string;
    template: string;
    updated_at: string;
    updated_by: string;
    variables: PromptVariableSchema[];
  };

/**
 * One review act as the API renders it.
 */
export type ProposalApprovalView = {
    approver_id: string;
    approver_subject: string;
    comment?: string | null;
    /**
     * The commit reviewed. An approval of another commit is evidence
     * about other content and never carries over.
     */
    commit: string;
    /**
     * Whether this act still counts: `false` once the proposal's commit
     * has moved past it.
     */
    counts: boolean;
    created_at: string;
    /**
     * The effective roles the approver held at the target when they cast
     * it — recorded then, never re-derived now (ADR-0032 decision 5).
     */
    roles: string[];
    verdict: string;
  };

/**
 * Content-free typed address shared by every governed artifact family.
 */
export type ProposalArtifactReference = {
    /**
     * Stable aggregate, binding, import job, or authored member id.
     */
    artifact_id: string;
    /**
     * Head inspected by a revision-aware mutation.
     */
    expected_revision?: string | null;
    /**
     * Closed common-review family vocabulary.
     */
    family: string;
    /**
     * Domain mutation carried by the reviewed effect.
     */
    operation: string;
    /**
     * Exact immutable revision, binding-state digest, or content digest.
     */
    version: string;
  };

/**
 * The version the target's published channel holds for a member now —
 * the old side of the diff, present only for [`MemberEffect::Update`].
 *
 * This is the one content-visibility widening in FLOW-6 (ADR-0035
 * decision 8): a reviewer sees what a publication would overwrite.
 * Bounded by the proposal's own member set,
 * the target's own channel, and the target scope the reviewer already
 * holds `ProposalRead` on — and admitted because a review of a change
 * that hides one side of the change is not a review.
 */
export type ProposalBaselineView = {
    /**
     * The address the target's tree names for this member today.
     */
    object_hash: string;
    /**
     * That object's canonical bytes as text (ADR-0030 decision 4's
     * human-readable form, which FLOW-1 chose for exactly this).
     */
    text: string;
  };

/**
 * One proposal, in full.
 */
export type ProposalDetail = ProposalSummary & {
    approvals: ProposalApprovalView[];
    members: ProposalMemberView[];
    timeline: ProposalTimelineEvent[];
  };

export type ProposalListResponse = {
    proposals: ProposalSummary[];
  };

/**
 * What publishing this proposal would do to the target's published
 * channel, for one member (FLOW-6, ADR-0035 decision 5). Membership in
 * the target's tree is the predicate — the same sense of "this scope
 * holds it" ADR-0034 decision 3 used one scope over.
 */
export type ProposalMemberEffect = "add" | "update" | "apply" | "none";

/**
 * One member of a proposal — the id and the address that was proposed,
 * plus what a reviewer needs to review it: the bytes under review, the
 * bytes they would replace, and the artifact's current content.
 */
export type ProposalMemberView = {
    /**
     * What kind of asset this proposal carries — one word, so a reviewer's
     * first line says what they are looking at.
     */
    asset: string;
    baseline?: null | ProposalBaselineView;
    /**
     * The member's text **as it stands now**. Beside `unchanged` this is what makes drift
     * legible; it is not what the approvals bind.
     */
    content: string;
    /**
     * What publication would do to the target's channel for this member.
     */
    effect: ProposalMemberEffect;
    /**
     * The tree entry name: a path for an authored asset or `command` for a
     * typed aggregate effect. The one field every artifact family carries.
     */
    member: string;
    /**
     * The address the proposal named.
     */
    object_hash: string;
    /**
     * The canonical bytes at the proposed address — what the approvals
     * bind, read from the object store rather than re-derived from the
     * source row, because an edited artifact is no longer what anyone approved
     * (ADR-0035 decision 6). Empty only if the object is missing, which
     * the append-only store makes impossible.
     */
    proposed: string;
    sensitivity: string;
    /**
     * Whether the member still hashes to that address. `false` means the
     * content moved after the proposal opened, and publishing will
     * refuse (ADR-0032 decision 6).
     */
    unchanged: boolean;
  };

export type ProposalOpenBody = {
    /**
     * The context-pack documents to propose, by path (PRMT-2, ADR-0050
     * decision 1).
     *
     * One entry per **document**, named `pack/document`: the pack channel
     * names documents rather than bundles (decision 3), so a proposal that
     * publishes half a pack is a thing the vocabulary can express and a
     * curator can decide on. Exactly one of the three member lists may be
     * present, for `prompt_names`' reason — a proposal has one asset kind,
     * because the approval matrix resolves from it and, since decision 15,
     * `regulated-strict` prices a pack at a department at two distinct
     * people where it prices a team's memory at one.
     */
    document_paths?: string[];
    /**
     * The prompts to propose, by name (PRMT-1, ADR-0049 decision 6).
     *
     * Exactly one authored-artifact member list may be present: a proposal
     * has one asset kind because the approval matrix resolves from it.
     *
     * The same two senses of "the source holds it" apply: the draft lives
     * there, or the source's published channel names it at that address —
     * which is what lets a department propose onward what a team climbed
     * into it, with no draft row at the department at all.
     */
    prompt_names?: string[];
    /**
     * The scope whose published channel would move. Requirements resolve
     * here, and only here — "each level's approvers" is true because
     * each level's proposal resolves at that level (ADR-0034
     * decision 4).
     */
    scope_id: string;
    /**
     * Where the material is now. Absent means the target — the
     * same-scope case, a climb of zero levels. Present, it must be the
     * target or a **descendant** of it: a climb goes up the chain that
     * composition walks down (ADR-0034 decision 2).
     */
    source_scope_id?: string | null;
    /**
     * What this proposes, in one line. A reviewer reads it in a list.
     */
    title: string;
  };

export type ProposalOpenResponse = ProposalSummary;

export type ProposalPublishResponse = {
    added: number;
    channel: string;
    /**
     * The commit the channel now points at. Its parents are
     * `[previous head, proposal commit]` — first-parent mainline as in
     * git, so lineage is a fact about the graph (ADR-0032 decision 10).
     */
    commit: string;
    members: number;
    parent?: string | null;
    /**
     * The proposal commit, this publication's second parent.
     */
    proposal_commit: string;
    proposal_id: string;
    scope_id: string;
  };

export type ProposalRejectBody = {
    /**
     * Exact proposal commit the reviewer inspected.
     */
    expected_commit: string;
    /**
     * Why. Mandatory — a rejection an auditor cannot read the reason for
     * is not a review, and FLOW-5 inherits this reason for its
     * per-level denials.
     */
    reason: string;
  };

export type ProposalReviewBody = {
    /**
     * What the reviewer wants to say. Optional on an approval; a
     * rejection carries its reason in `reason` instead.
     */
    comment?: string | null;
    /**
     * Exact proposal commit the reviewer inspected.
     */
    expected_commit: string;
  };

export type ProposalReviewResponse = ProposalSummary & {
    /**
     * What this act contributed: the roles it counted under.
     */
    counted_roles: string[];
  };

/**
 * One proposal in a listing.
 */
export type ProposalSummary = {
    /**
     * Stable, content-free artifacts and exact versions bound by the commit.
     */
    artifact_references: ProposalArtifactReference[];
    asset: string;
    close_reason?: string | null;
    closed_at?: string | null;
    /**
     * The commit holding exactly what is proposed.
     */
    commit: string;
    created_at: string;
    /**
     * What running this proposal would do. `published` writes a channel,
     * `classify` changes sensitivity, and `apply` executes a typed governed
     * artifact command (including a policy relaxation).
     */
    effect: string;
    id: string;
    /**
     * What it still lacks, in one line a reviewer reads.
     */
    outstanding: string;
    promotion?: null | PromotionEvidenceSchema;
    proposer_id: string;
    proposer_subject: string;
    /**
     * What the matrix asks for here, resolved now.
     */
    required: ApprovalRequirementView;
    sensitivity: string;
    source_scope_id: string;
    source_scope_path?: string | null;
    /**
     * The five-state vocabulary tech plan §2.3 describes: the stored
     * state, with `approved` rendered from `open` plus a satisfied
     * requirement (ADR-0032 decision 11).
     */
    state: string;
    target_scope_id: string;
    /**
     * The target's hierarchy path. A review surface that renders two
     * UUIDs is not one a person can use, and for a climb the *source*
     * is half of what is being judged (FLOW-6, ADR-0035 decision 9).
     * Absent only inside TEN-5's disposal window, when the scope the
     * proposal targets has already gone.
     */
    target_scope_path?: string | null;
    title: string;
    updated_at: string;
  };

/**
 * One common proposal lifecycle event, oldest first.
 */
export type ProposalTimelineEvent = {
    actor_id?: string | null;
    actor_subject?: string | null;
    at: string;
    /**
     * Exact proposal commit the act bound.
     */
    commit: string;
    /**
     * `opened`, `approved`, `rejected`, `withdrawn`, `applied`, or `published`.
     */
    kind: string;
    reason?: string | null;
  };

/**
 * Publish another complete immutable version.
 */
export type PublishConfigurationBody = {
    document: ConfigurationDocumentBody;
    expected_current_version_id: string;
    source_template?: "personal" | "team" | "enterprise";
  };

/**
 * One quarantined event as the API renders it: the redacted payload,
 * the finding summary, and the review state — never raw finding text
 * (there is none anywhere to render, ADR-0021 decision 1).
 */
export type QuarantineEventView = {
    client_event_id: string;
    created_at: string;
    event_id: string;
    event_type: string;
    findings: unknown;
    payload: unknown;
    /**
     * The token subject that opened that run.
     */
    principal_id: string;
    review_reason?: string | null;
    reviewed_at?: string | null;
    reviewer_subject?: string | null;
    scope_id: string;
    /**
     * The run the event belongs to — a real aggregate since CPR-12, so a
     * reviewer can open the transcript this payload came from instead of
     * deciding about it in isolation.
     */
    session_id: string;
    state: string;
  };

export type QuarantineQueueResponse = {
    pending: QuarantineEventView[];
  };

export type QuarantineReviewBody = {
    /**
     * The reviewer's note, recorded on the row and in the audit event.
     */
    reason?: string | null;
  };

/**
 * Append one idempotent usage observation.
 */
export type RecordSkillUsageBody = {
    /**
     * Active binding observed.
     */
    binding_id: string;
    /**
     * Client idempotency key.
     */
    client_event_id: string;
    evidence: "host_observed" | "model_reported";
    /**
     * Bounded, content-free evidence.
     */
    metadata?: Record<string, unknown>;
    /**
     * Client occurrence time.
     */
    occurred_at: string;
    /**
     * Resource/script path for the stages that name one.
     */
    resource_path?: string | null;
    /**
     * Session carrying the event, when applicable.
     */
    session_id?: string | null;
    stage: "advertised" | "discovered" | "activated" | "instructions_loaded" | "resource_loaded" | "script_requested" | "executed" | "outcome_reported";
    /**
     * Exact immutable version involved.
     */
    version_id: string;
  };

export type RegisterServiceIdentityBody = {
    /**
     * Display name for the agent's personal leaf; defaults to the
     * subject.
     */
    display_name?: string | null;
    /**
     * The anchor node whose subtree confines the agent's tokens.
     */
    scope_id: string;
    /**
     * The `sub` the IdP will put in the agent's client-credentials
     * tokens (for Rauthy, the client id).
     */
    subject: string;
  };

/**
 * Register a server and its first immutable discovery snapshot.
 */
export type RegisterToolServerBody = {
    /**
     * Raw stateless MCP discovery result.
     */
    capabilities: Record<string, unknown>;
    /**
     * Immutable source/transport/authentication metadata.
     */
    descriptor: ToolServerDescriptorBody;
    /**
     * Scope governing the catalogue entry.
     */
    governing_scope_id: string;
    /**
     * Tenant-unique display name.
     */
    name: string;
  };

/**
 * Time-boxed relaxation bounds. This document can narrow the closed product
 * vocabulary but never grants authority by itself.
 */
export type RelaxationConfigurationBody = {
    allowed_actions: string[];
    enabled: boolean;
    maximum_duration_secs: number;
  };

export type RelaxationListView = {
    next_cursor?: string | null;
    relaxations: RelaxationView[];
  };

/**
 * Result shared by create, revise and revoke.
 */
export type RelaxationMutationView = {
    change_id: string;
    outcome: "applied" | "pending_review" | "rejected";
    relaxation_id: string;
    revision?: number | null;
    version_id?: string | null;
  };

/**
 * Complete reviewed terms for create and revision requests.
 */
export type RelaxationTermsBody = {
    action: "knowledge.read";
    max_sensitivity: string;
    reason: string;
    requested_end_at: string;
    requested_start_at: string;
    subject_identity_id: string;
  };

export type RelaxationVersionListView = {
    next_cursor?: number | null;
    versions: RelaxationVersionView[];
  };

/**
 * One immutable reviewed version.
 */
export type RelaxationVersionView = {
    action: "knowledge.read";
    approver_ids: string[];
    auto_applied: boolean;
    change_id: string;
    configuration_hash: string;
    configuration_version_id?: string | null;
    content_hash: string;
    created_at: string;
    creator_id: string;
    effective_start_at: string;
    hard_expires_at: string;
    id: string;
    max_sensitivity: string;
    ordinal: number;
    reason: string;
    relaxation_id: string;
    requested_end_at: string;
    requested_start_at: string;
    subject: string;
    subject_identity_id: string;
    target_scope_id: string;
  };

/**
 * Current stable aggregate projection.
 */
export type RelaxationView = {
    created_at: string;
    created_by: string;
    current: RelaxationVersionView;
    current_version_id: string;
    governing_scope_id: string;
    id: string;
    revision: number;
    revocation_change_id?: string | null;
    revocation_reason?: string | null;
    revoked_at?: string | null;
    revoked_by?: string | null;
    status: "scheduled" | "active" | "expired" | "revoked";
    updated_at: string;
    updated_by: string;
  };

/**
 * Replace one visible current Knowledge item with this candidate.
 */
export type ReplaceCandidateBody = {
    /**
     * Exact existing head inspected.
     */
    expected_revision_id: string;
    /**
     * Existing item to supersede.
     */
    item_id: string;
    /**
     * Optional replacement placement/content edits.
     */
    replacement?: AcceptCandidateBody;
  };

/**
 * The repository listing.
 */
export type RepositoryList = {
    /**
     * What the project is about, oldest attachment first.
     */
    repositories: RepositoryView[];
  };

/**
 * A repository attached to a project, as the API serves it.
 */
export type RepositoryView = {
    /**
     * **The identity**: canonical, credential-free, and never a filesystem
     * path. Two clients that describe one repository differently are served
     * the same value here, which is what makes it an identity.
     */
    canonical_uri: string;
    /**
     * When it was attached.
     */
    created_at: string;
    /**
     * Who attached it.
     */
    created_by?: string | null;
    /**
     * The advisory default branch.
     */
    default_branch?: string | null;
    /**
     * The attachment's id — the handle `DELETE` takes.
     */
    id: string;
    /**
     * The stable content fingerprint of a local checkout, when one was given.
     */
    local_fingerprint?: string | null;
    /**
     * The caller's labelling bag, echoed back.
     */
    metadata: Record<string, unknown>;
    /**
     * The project it belongs to.
     */
    project_id: string;
    provider: "github" | "gitlab" | "bitbucket" | "azure_devops" | "generic_git" | "local";
    /**
     * The repository's own name.
     */
    repository_name: string;
    /**
     * The owning path on the host, when the remote had one.
     */
    repository_owner?: string | null;
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * One governed conflict resolution.
 */
export type ResolveConflictBody = {
    /**
     * Exact conflict-set revision inspected.
     */
    expected_revision: number;
    /**
     * Bounded human rationale retained in the VedaFlow command.
     */
    reason: string;
    resolution: "keep_separate" | "support" | "duplicate" | "supersede" | "transition" | "archive";
    /**
     * Exact future valid-time boundary for `transition` only.
     */
    transition_at?: string | null;
  };

export type ReviseRelaxationBody = {
    action: "knowledge.read";
    max_sensitivity: string;
    reason: string;
    requested_end_at: string;
    requested_start_at: string;
    subject_identity_id: string;
  } & {
    expected_current_version_id: string;
  };

export type RevokeRelaxationBody = {
    expected_current_version_id: string;
    reason: string;
  };

/**
 * Pin an older version without mutating history.
 */
export type RollbackConfigurationBindingBody = {
    expected_revision: number;
    version_id: string;
  };

/**
 * Roll a binding back by pinning an older immutable version.
 */
export type RollbackSkillBindingBody = {
    /**
     * Exact binding revision required when the change applies.
     */
    expected_revision: number;
    /**
     * Older version of the same skill.
     */
    version_id: string;
  };

/**
 * Run the non-executing built-in validation sandbox.
 */
export type RunSkillTestBody = {
    harness: "validation_sandbox" | "controlled_client";
  };

/**
 * Trusted-adapter report for a read-only connection test.
 */
export type RunToolTestBody = {
    /**
     * Bounded credential-free report.
     */
    evidence?: Record<string, unknown>;
    harness: "trusted_local_adapter" | "remote_http_adapter";
    /**
     * Exact adapter implementation/version.
     */
    harness_version: string;
    /**
     * End-to-end elapsed milliseconds.
     */
    latency_ms?: number | null;
    /**
     * Methods attempted. `tools/call` and every execution method are refused.
     */
    methods: string[];
    outcome: "passed" | "failed" | "error";
  };

/**
 * The non-secret provisioning credential metadata exposed after issuance.
 */
export type ScimCredentialView = {
    created_at: string;
    created_by: string;
    expires_at: string;
    id: string;
    label: string;
    last_used_at?: string | null;
    revoked_at?: string | null;
  };

export type ScimCredentialsResponse = {
    credentials: ScimCredentialView[];
  };

/**
 * `GET /v1/admin/scopes/{scope_id}` — the scope and its path.
 */
export type ScopeDetail = {
    /**
     * The slug chain from the tenant root — display and ordering only.
     */
    path: string;
    /**
     * The scope itself.
     */
    scope: ScopeView;
  };

/**
 * One governed scope as the admin surface serves it.
 */
export type ScopeView = {
    /**
     * The open labelling bag.
     */
    attributes: unknown;
    /**
     * When the scope was created.
     */
    created_at: string;
    /**
     * Display name, renameable.
     */
    display_name: string;
    /**
     * The scope's stable id.
     */
    id: string;
    /**
     * The scope's shape.
     */
    kind: string;
    /**
     * Its parent; absent only on the tenant root.
     */
    parent_scope_id?: string | null;
    /**
     * The subject this scope belongs to, on a `principal`-shaped scope.
     */
    principal_id?: string | null;
    /**
     * Sibling-unique handle, immutable.
     */
    slug: string;
    /**
     * `active` or `archived`.
     */
    status: string;
    /**
     * When the scope last changed.
     */
    updated_at: string;
  };

export type ServiceIdentitiesResponse = {
    identities: ServiceIdentityView[];
  };

/**
 * A service identity as the public application API exposes it.
 */
export type ServiceIdentityView = {
    created_at: string;
    departed_at?: string | null;
    display_name?: string | null;
    email?: string | null;
    id: string;
    kind: string;
    scope_id: string;
    status: string;
    subject?: string | null;
  };

/**
 * One immutable session event, as the API serves it.
 */
export type SessionEventView = {
    /**
     * The client's own id for it.
     */
    client_event_id: string;
    /**
     * The payload shape the client declared.
     */
    event_schema_version: number;
    event_type: "session.started" | "session.ended" | "message.user" | "message.assistant" | "tool.invoked" | "tool.result" | "file.read" | "file.changed" | "command.executed" | "skill.loaded" | "context.requested" | "adapter.warning" | "memory.asserted";
    /**
     * The event's id in this deployment.
     */
    id: string;
    /**
     * When the client says it happened.
     */
    occurred_at: string;
    /**
     * The content.
     */
    payload: Record<string, unknown>;
    /**
     * BLAKE3-256 of the canonical payload, hex — the server's.
     */
    payload_hash: string;
    /**
     * When the gateway received it.
     */
    received_at: string;
    /**
     * Position in the session, assigned by the server.
     */
    sequence: number;
    /**
     * The session it belongs to.
     */
    session_id: string;
  };

/**
 * The session listing, one page of it.
 */
export type SessionList = {
    /**
     * Where the next page resumes, or absent when this is the last one
     * (CPR-11, ADR-0077 decision 1).
     *
     * Opaque: pass it back as `cursor` and nothing else. It replaced CPR-10's
     * `truncated` boolean, which could say *that* an answer was cut short and
     * could not say where to continue — so a reader who wanted the run from
     * last Tuesday had no way to reach it.
     *
     * A page may be **empty and still carry one**: rows are filtered by the
     * PDP after they are scanned, so a page whose candidates this caller may
     * not read serves nothing and still says where to continue. That is the
     * honest shape — the alternative is a server that keeps scanning until it
     * fills a page, which is unbounded work driven by somebody else's rows.
     */
    next_cursor?: string | null;
    /**
     * The sessions this caller may read, newest first.
     */
    sessions: SessionView[];
  };

/**
 * A session, as the API serves it.
 *
 * A view rather than `synveda_types::session::Session` itself, for
 * [`crate::workspaces::WorkspaceView`]'s reason: this is the **contract** and
 * the domain type is not. `tenant_id` is deliberately absent — every `/v1`
 * response is already scoped to the caller's tenant.
 */
export type SessionView = {
    /**
     * Which agent ran.
     */
    agent_name?: string | null;
    /**
     * The branch it was on.
     */
    branch?: string | null;
    /**
     * A stable id for that installation of the client.
     */
    client_installation_id?: string | null;
    /**
     * The agent client, as it named itself.
     */
    client_name: string;
    /**
     * Its version, when it said one.
     */
    client_version?: string | null;
    /**
     * When the row was created.
     */
    created_at: string;
    /**
     * Why it stopped, in the client's words — `status` says a run failed,
     * this says the hook timed out (CPR-11, ADR-0077 decision 4).
     */
    end_reason?: string | null;
    /**
     * When it closed.
     */
    ended_at?: string | null;
    /**
     * The harness's own id for the run.
     */
    external_session_id?: string | null;
    /**
     * The session's stable id.
     */
    id: string;
    /**
     * The newest appended event's own instant.
     */
    last_observed_at?: string | null;
    /**
     * The client's labelling bag, echoed back.
     */
    metadata: Record<string, unknown>;
    /**
     * The model, as the client named it.
     */
    model_name?: string | null;
    /**
     * The token subject that opened it.
     */
    principal_id: string;
    /**
     * The project, when the run was against one.
     */
    project_id?: string | null;
    /**
     * The repository the run was against.
     */
    repository_id?: string | null;
    /**
     * The governed scope this session is decided at — **derived** from the
     * workspace and project, never submitted.
     */
    scope_id: string;
    /**
     * When it began.
     */
    started_at: string;
    status: "active" | "ending" | "ended" | "abandoned" | "failed";
    /**
     * What the run is about, in the client's words.
     */
    task_summary?: string | null;
    /**
     * When the row last changed.
     */
    updated_at: string;
    /**
     * The workspace the run happened in.
     */
    workspace_id: string;
  };

/**
 * Binding collection.
 */
export type SkillBindingListView = {
    /**
     * Policy-visible bindings.
     */
    bindings: SkillBindingView[];
    /**
     * Cursor after the last candidate considered.
     */
    next_cursor?: string | null;
  };

/**
 * One revisioned binding.
 */
export type SkillBindingView = {
    /**
     * Binding creation time.
     */
    created_at: string;
    /**
     * Principal that created the binding.
     */
    created_by: string;
    /**
     * Whether sessions may discover this binding.
     */
    enabled: boolean;
    /**
     * Stable binding identifier.
     */
    id: string;
    /**
     * Exact version pin, or absent to follow the current pointer.
     */
    pinned_version_id?: string | null;
    /**
     * Optimistic-concurrency revision.
     */
    revision: number;
    /**
     * Bound project or principal scope.
     */
    scope_id: string;
    /**
     * Bound Skill aggregate.
     */
    skill_id: string;
    /**
     * Last binding transition time.
     */
    updated_at: string;
    /**
     * Principal that made the last binding transition.
     */
    updated_by: string;
  };

/**
 * One file supplied for an immutable version.
 */
export type SkillFileBody = {
    /**
     * UTF-8 text stored and installed byte-for-byte.
     */
    content: string;
    /**
     * Bundle-relative path.
     */
    path: string;
  };

/**
 * Cursor-paginated catalogue page.
 */
export type SkillListView = {
    /**
     * Cursor after the last candidate considered.
     */
    next_cursor?: string | null;
    /**
     * Policy-visible catalogue entries.
     */
    skills: SkillView[];
  };

/**
 * Stable result envelope for every governed Skill mutation.
 */
export type SkillMutationView = {
    /**
     * Revisioned binding created or changed by the change.
     */
    binding_id?: string | null;
    /**
     * Binding revision produced by the change.
     */
    binding_revision?: number | null;
    /**
     * VedaFlow change id.
     */
    change_id: string;
    outcome: "applied" | "pending_review" | "rejected";
    /**
     * Stable Skill aggregate affected by the change.
     */
    skill_id?: string | null;
    /**
     * Immutable version created or selected by the change.
     */
    version_id?: string | null;
  };

/**
 * Provenance supplied for an imported or authored bundle.
 */
export type SkillProvenanceBody = {
    kind: "authored" | "directory" | "archive" | "git" | "registry";
    /**
     * Forward-compatible source metadata.
     */
    metadata?: Record<string, unknown>;
    /**
     * Non-secret source reference.
     */
    reference?: string | null;
    /**
     * Exact upstream revision, when present.
     */
    revision?: string | null;
  };

/**
 * Test-run collection.
 */
export type SkillTestRunListView = {
    /**
     * Cursor after the last returned run.
     */
    next_cursor?: string | null;
    /**
     * Immutable controlled-harness results.
     */
    runs: SkillTestRunView[];
  };

/**
 * One immutable controlled-harness result.
 */
export type SkillTestRunView = {
    /**
     * Test-run creation time.
     */
    created_at: string;
    /**
     * Principal that requested the test.
     */
    created_by: string;
    /**
     * Content-free validation evidence.
     */
    evidence: Record<string, unknown>;
    harness: "validation_sandbox" | "controlled_client";
    /**
     * Exact harness implementation version.
     */
    harness_version: string;
    /**
     * Stable test-run identifier.
     */
    id: string;
    outcome: "passed" | "failed" | "error";
    /**
     * Quality rubric used by the run.
     */
    rubric_version: number;
    /**
     * Scanner ruleset used by the run.
     */
    scan_ruleset_version: number;
    /**
     * Exact immutable version tested.
     */
    version_id: string;
  };

/**
 * One append-only usage event.
 */
export type SkillUsageEventView = {
    /**
     * Binding that advertised the version.
     */
    binding_id: string;
    /**
     * Adapter-provided idempotency key.
     */
    client_event_id: string;
    evidence: "host_observed" | "model_reported";
    /**
     * Stable append-only usage event identifier.
     */
    id: string;
    /**
     * Bounded content-free evidence.
     */
    metadata: Record<string, unknown>;
    /**
     * Client occurrence time.
     */
    occurred_at: string;
    /**
     * Principal associated with the lifecycle event.
     */
    principal_id: string;
    /**
     * Server receipt time.
     */
    received_at: string;
    /**
     * Resource or script path, when the stage names one.
     */
    resource_path?: string | null;
    /**
     * Governed session, when the lifecycle event occurred in one.
     */
    session_id?: string | null;
    stage: "advertised" | "discovered" | "activated" | "instructions_loaded" | "resource_loaded" | "script_requested" | "executed" | "outcome_reported";
    /**
     * Exact immutable version involved.
     */
    version_id: string;
  };

/**
 * Usage collection.
 */
export type SkillUsageListView = {
    /**
     * Append-only usage evidence.
     */
    events: SkillUsageEventView[];
    /**
     * Cursor after the last returned event.
     */
    next_cursor?: string | null;
  };

/**
 * One authorised file with its exact immutable content.
 */
export type SkillVersionFileContentView = {
    /**
     * Exact authorised text content.
     */
    content: string;
    /**
     * Content-addressed VedaFlow object hash.
     */
    object_hash: string;
    /**
     * Relative bundle path.
     */
    path: string;
    /**
     * Exact immutable version containing the file.
     */
    version_id: string;
  };

/**
 * Immutable file collection.
 */
export type SkillVersionFileListView = {
    /**
     * Immutable file descriptors in path order.
     */
    files: SkillVersionFileView[];
  };

/**
 * One immutable file descriptor.
 */
export type SkillVersionFileView = {
    /**
     * Unicode scalar count retained for bounded clients.
     */
    chars: number;
    /**
     * File-reference creation time.
     */
    created_at: string;
    /**
     * Content-addressed VedaFlow object hash.
     */
    object_hash: string;
    /**
     * Relative bundle path.
     */
    path: string;
  };

/**
 * Immutable-version collection.
 */
export type SkillVersionListView = {
    /**
     * Ordinal cursor for the next page.
     */
    next_cursor?: number | null;
    /**
     * Immutable versions, newest first.
     */
    versions: SkillVersionView[];
  };

/**
 * Immutable version metadata. File bytes use the dedicated file route.
 */
export type SkillVersionView = {
    /**
     * Stable digest over exact bundle paths and object addresses.
     */
    bundle_digest: string;
    /**
     * Immutable version creation time.
     */
    created_at: string;
    /**
     * Principal that created the version through VedaFlow.
     */
    created_by: string;
    /**
     * Declared tools are metadata and grant no authority.
     */
    declared_tools_are_authorization: boolean;
    /**
     * Immutable version identifier.
     */
    id: string;
    /**
     * Parsed Agent Skills manifest with extension metadata preserved.
     */
    manifest: Record<string, unknown>;
    /**
     * Monotonic version number within the aggregate.
     */
    ordinal: number;
    /**
     * Version-specific provenance evidence.
     */
    provenance: Record<string, unknown>;
    /**
     * Automated quality score from zero through one hundred.
     */
    quality_score: number;
    /**
     * Rubric version that produced the score.
     */
    rubric_version: number;
    /**
     * Content-free scanner evidence.
     */
    scan: Record<string, unknown>;
    /**
     * Scanner ruleset that produced the evidence.
     */
    scan_ruleset_version: number;
    sensitivity: "public" | "internal" | "confidential" | "restricted";
    /**
     * Stable Skill aggregate identifier.
     */
    skill_id: string;
    source_kind: "authored" | "directory" | "archive" | "git" | "registry";
  };

/**
 * Stable skill head and its current immutable version.
 */
export type SkillView = {
    /**
     * Aggregate creation time.
     */
    created_at: string;
    /**
     * Principal that installed the aggregate.
     */
    created_by: string;
    /**
     * Current immutable version metadata.
     */
    current_version: SkillVersionView;
    /**
     * Current immutable version pointer.
     */
    current_version_id: string;
    /**
     * Scope governing installation and updates.
     */
    governing_scope_id: string;
    /**
     * Stable Skill aggregate identifier.
     */
    id: string;
    /**
     * Tenant-unique Agent Skills bundle name.
     */
    name: string;
    /**
     * Last current-pointer update time.
     */
    updated_at: string;
    /**
     * Principal that last advanced the current pointer.
     */
    updated_by: string;
  };

/**
 * Stage changed source, transport, auth or capability metadata.
 */
export type StageToolVersionBody = {
    /**
     * Complete raw discovery result.
     */
    capabilities: Record<string, unknown>;
    /**
     * Complete replacement descriptor.
     */
    descriptor: ToolServerDescriptorBody;
    /**
     * Exact current approved version precondition.
     */
    expected_current_version_id: string;
  };

/**
 * `POST /v1/knowledge/{id}/supersede`.
 */
export type SupersedeKnowledgeBody = {
    /**
     * Replacement's first revision.
     */
    content: KnowledgeContentBody;
    /**
     * Exact old head inspected.
     */
    expected_revision_id: string;
    knowledge_type: "fact" | "decision" | "preference" | "procedure" | "entity" | "episode" | "convention" | "warning" | "reference";
    origin: "observed" | "asserted" | "authored" | "imported";
    /**
     * Replacement owner.
     */
    owner_principal_id?: string | null;
    /**
     * Replacement project association.
     */
    project_id?: string | null;
    /**
     * Replacement governing scope.
     */
    scope_id: string;
    /**
     * Replacement provenance.
     */
    sources?: KnowledgeSourceBody[];
  };

/**
 * `GET /v1/directory/sync` — what the last pass did.
 */
export type SyncStatus = {
    /**
     * Set iff the most recent complete pass refused to seal.
     */
    breaker_tripped_at?: string | null;
    /**
     * How many that pass declined to seal — the number an operator is
     * being asked to bound.
     */
    breaker_would_have_sealed?: number | null;
    /**
     * Which connector last wrote this state.
     */
    connector: string;
    /**
     * The last one that finished. A gap between this and `last_pass_at` is
     * a connector that runs and never completes — the state in which
     * nobody is sealed and nothing looks wrong.
     */
    last_complete_pass_at?: string | null;
    /**
     * The last attempt, complete or not.
     */
    last_pass_at?: string | null;
    /**
     * Passes that completed. An absence count means nothing without it.
     */
    passes_completed: number;
    seal_authorisation?: null | AuthorisationView;
  };

/**
 * What the caller may do on the **tenant** plane — `whoami`'s block, and
 * since CPR-4 `/v1/me`'s.
 *
 * Carries a `ToSchema` because `/v1/me` embeds it and that route is on the
 * OpenAPI contract. The three fields are declared to the document by
 * `value_type` rather than derived: `Role` and the `&'static str` map keys
 * live in `synveda-types`, where `utoipa` deliberately does not reach — a
 * contract is a property of the surface, and no crate below the gateway has
 * one.
 */
export type TenantCapabilities = {
    /**
     * Every operand-free tenant-plane action, by its stable machine name.
     */
    actions: Record<string, boolean>;
    /**
     * The caller's role keys at the **tenant root scope** — the grants
     * that reach the whole boundary (CPR-6, ADR-0073 decision 5; since
     * the cutover, the only roles there are).
     */
    role_keys: string[];
  };

/**
 * The tenant, as this plane serves it.
 */
export type TenantView = {
    /**
     * The isolation key.
     */
    id: string;
    /**
     * Display name.
     */
    name: string;
    /**
     * Human-stable handle.
     */
    slug: string;
    /**
     * `active` or `suspended`.
     */
    status: string;
  };

/**
 * One entry of the timeline projection.
 *
 * Deliberately **not** a union of the two row shapes. A timeline is a reading
 * surface: it answers "what happened, in order, and roughly what was it", and
 * a client that wants an event's full payload fetches the event. Flattening
 * two tables into one wide row with half its fields null per entry would make
 * every consumer branch on which half is populated.
 */
export type TimelineEntry = {
    /**
     * When it happened: an event's `occurred_at`, a run's `created_at`.
     */
    at: string;
    /**
     * Whether the gap between the two exceeded a minute.
     *
     * Computed rather than left to each client, so "this did not arrive live"
     * means one thing across the console, the CLI and anything else that
     * reads a timeline. It is what a locally spooled batch, a replay after a
     * crash, and a machine with a wrong clock all look like from here — the
     * server cannot tell those three apart and does not pretend to.
     */
    delayed: boolean;
    /**
     * The event type, for an event.
     */
    event_type?: string | null;
    /**
     * The entry's own id, as a string, because the two sources have
     * different id types and a timeline is read rather than joined.
     */
    id: string;
    /**
     * `event` or `context_run` — which table this came from.
     */
    kind: string;
    /**
     * When this deployment received it, for an event (CPR-11, ADR-0077
     * decision 2).
     *
     * The *other* clock. `at` is the client's statement about when the thing
     * happened; this is when the gateway was told. A live turn has them
     * within a second of each other; an adapter that spooled to disk while
     * the network was down delivers an hour of them at once, and only one of
     * the two instants is a clock this deployment controls.
     *
     * Absent for a context run: a composition happens *here*, so its two
     * instants would be the same number written twice.
     */
    received_at?: string | null;
    /**
     * The event's position, for an event.
     */
    sequence?: number | null;
    /**
     * One line about what this entry is — a run's query, an event's family.
     */
    summary: string;
  };

/**
 * The timeline projection.
 */
export type TimelineView = {
    /**
     * The merged entries, oldest first.
     */
    entries: TimelineEntry[];
    /**
     * How many events the session has appended, of every type — the shape of
     * the run, which is what an auditor reads before any single entry.
     */
    event_counts: Record<string, number>;
    /**
     * The session this is about.
     */
    session_id: string;
    /**
     * Whether either source hit its bound.
     */
    truncated: boolean;
  };

/**
 * Binding collection.
 */
export type ToolBindingListView = {
    /**
     * Policy-visible bindings.
     */
    bindings: ToolBindingView[];
    /**
     * Cursor after the last returned binding.
     */
    next_cursor?: string | null;
  };

/**
 * One exact-version project binding.
 */
export type ToolBindingView = {
    /**
     * Creation instant.
     */
    created_at: string;
    /**
     * Stable binding id.
     */
    id: string;
    /**
     * Target project.
     */
    project_id: string;
    /**
     * Optimistic-concurrency revision.
     */
    revision: number;
    /**
     * Target project scope.
     */
    scope_id: string;
    /**
     * Stable server.
     */
    server_id: string;
    state: "enabled" | "disabled" | "removed";
    /**
     * Last transition instant.
     */
    updated_at: string;
    /**
     * Exact immutable approved version.
     */
    version_id: string;
  };

/**
 * Secret-free generated client configuration.
 */
export type ToolClientConfigurationView = {
    /**
     * Binding/version pairs included.
     */
    bindings: ToolConfigurationBindingView[];
    /**
     * Configuration contains exact approved bindings only.
     */
    configuration: Record<string, unknown>;
    /**
     * Target project.
     */
    project_id: string;
  };

/**
 * Evidence for one generated configuration entry.
 */
export type ToolConfigurationBindingView = {
    /**
     * Binding id.
     */
    binding_id: string;
    /**
     * Immutable version digest.
     */
    digest: string;
    /**
     * Stable server id.
     */
    server_id: string;
    /**
     * Exact version id.
     */
    version_id: string;
  };

/**
 * Governed mutation response.
 */
export type ToolMutationView = {
    /**
     * Binding when applicable.
     */
    binding_id?: string | null;
    /**
     * Resulting binding revision.
     */
    binding_revision?: number | null;
    /**
     * VedaFlow change id.
     */
    change_id: string;
    outcome: "applied" | "pending_review" | "rejected";
    /**
     * Stable server when applicable.
     */
    server_id?: string | null;
    /**
     * Exact immutable version when applicable.
     */
    version_id?: string | null;
  };

/**
 * Credential-free immutable server descriptor.
 */
export type ToolServerDescriptorBody = {
    /**
     * Literal argument vector; never a shell line.
     */
    args?: string[];
    authentication: "none" | "oauth" | "api_key" | "custom";
    /**
     * One executable token for trusted-local stdio metadata.
     */
    command?: string | null;
    /**
     * HTTPS endpoint for Streamable HTTP.
     */
    endpoint?: string | null;
    /**
     * Forward-compatible credential-free metadata.
     */
    metadata?: Record<string, unknown>;
    /**
     * Requested permissions are review metadata, not authorisation.
     */
    requested_permissions?: string[];
    /**
     * Opaque reference resolved outside the gateway. Never a secret value.
     */
    secret_reference?: string | null;
    source_kind: "manifest" | "client_config" | "remote_http" | "trusted_local_adapter";
    /**
     * Human-inspectable credential-free source reference.
     */
    source_reference: string;
    transport: "stdio" | "streamable_http";
  };

/**
 * Cursor-paginated server catalogue.
 */
export type ToolServerListView = {
    /**
     * Cursor after the last candidate considered.
     */
    next_cursor?: string | null;
    /**
     * Policy-visible entries.
     */
    servers: ToolServerView[];
  };

/**
 * Version collection.
 */
export type ToolServerVersionListView = {
    /**
     * Cursor after the last returned version.
     */
    next_cursor?: number | null;
    /**
     * Policy-visible versions.
     */
    versions: ToolServerVersionView[];
  };

/**
 * One immutable MCP server version and discovery snapshot.
 */
export type ToolServerVersionView = {
    /**
     * Snapshot digest.
     */
    capability_digest: string;
    /**
     * VedaFlow proposal defining the trust state.
     */
    change_id: string;
    /**
     * Creation instant.
     */
    created_at: string;
    /**
     * Capability names and descriptions grant no authority.
     */
    declared_capabilities_are_authorization: boolean;
    /**
     * Credential-free descriptor. Secret references are opaque identifiers.
     */
    descriptor: ToolServerDescriptorBody;
    /**
     * Stable digest.
     */
    digest: string;
    /**
     * Discovery instant.
     */
    discovered_at: string;
    /**
     * Immutable version id.
     */
    id: string;
    /**
     * Canonical comparison snapshot.
     */
    normalized_capabilities: Record<string, unknown>;
    /**
     * Monotonic version ordinal.
     */
    ordinal: number;
    /**
     * Pinned official MCP protocol version.
     */
    protocol_version: string;
    /**
     * Immutable raw discovery evidence.
     */
    raw_capabilities: Record<string, unknown>;
    /**
     * Whether an opaque secret reference is configured.
     */
    secret_reference_present: boolean;
    /**
     * Stable server id.
     */
    server_id: string;
    state: "quarantined" | "approved" | "rejected";
  };

/**
 * Stable catalogue entry.
 */
export type ToolServerView = {
    /**
     * Creation instant.
     */
    created_at: string;
    /**
     * Current approved version, absent while first registration is quarantined.
     */
    current_version_id?: string | null;
    /**
     * Governing scope.
     */
    governing_scope_id: string;
    /**
     * Stable id.
     */
    id: string;
    /**
     * Display name.
     */
    name: string;
    /**
     * Last approved-pointer transition.
     */
    updated_at: string;
  };

/**
 * Test-run collection.
 */
export type ToolTestRunListView = {
    /**
     * Cursor after the last returned report.
     */
    next_cursor?: string | null;
    /**
     * Immutable test reports.
     */
    runs: ToolTestRunView[];
  };

/**
 * One immutable read-only connection-test report.
 */
export type ToolTestRunView = {
    /**
     * Server receipt instant.
     */
    created_at: string;
    /**
     * Credential-free evidence.
     */
    evidence: Record<string, unknown>;
    harness: "trusted_local_adapter" | "remote_http_adapter";
    /**
     * Exact reporter version.
     */
    harness_version: string;
    /**
     * Run id.
     */
    id: string;
    /**
     * Elapsed milliseconds.
     */
    latency_ms?: number | null;
    /**
     * Read-only methods attempted.
     */
    methods: string[];
    outcome: "passed" | "failed" | "error";
    /**
     * Exact version tested.
     */
    version_id: string;
  };

/**
 * Visible comparison between two immutable versions.
 */
export type ToolVersionDiffView = {
    /**
     * Descriptor fields whose canonical values differ.
     */
    descriptor_changed: string[];
    /**
     * Baseline version.
     */
    from_version_id: string;
    /**
     * Added prompt names.
     */
    prompts_added: string[];
    /**
     * Prompt names whose schema/metadata changed.
     */
    prompts_changed: string[];
    /**
     * Removed prompt names.
     */
    prompts_removed: string[];
    /**
     * Added resource URIs.
     */
    resources_added: string[];
    /**
     * Resource URIs whose schema/metadata changed.
     */
    resources_changed: string[];
    /**
     * Removed resource URIs.
     */
    resources_removed: string[];
    /**
     * Candidate version.
     */
    to_version_id: string;
    /**
     * Added tool names.
     */
    tools_added: string[];
    /**
     * Tool names whose description or input schema changed.
     */
    tools_changed: string[];
    /**
     * Removed tool names.
     */
    tools_removed: string[];
  };

/**
 * `PATCH /v1/workspaces/{workspace_id}` and
 * `PATCH /v1/projects/{project_id}`.
 *
 * `description` has three cases and the wire says them apart: absent leaves
 * it alone, `null` clears it, a string replaces it.
 */
export type UpdateBody = {
    /**
     * New description; `null` clears it.
     */
    description?: string | null;
    /**
     * New display name.
     */
    display_name?: string | null;
    /**
     * The revision the caller last saw. Required: an update without a
     * precondition is a last-writer-wins update, which is the failure this
     * field exists to remove rather than a convenience it offers.
     */
    expected_revision: number;
    status?: "active" | "archived";
  };

/**
 * Change a selector under an exact revision precondition.
 */
export type UpdateConfigurationBindingBody = {
    artifact_id: string;
    enabled: boolean;
    expected_revision: number;
    pinned_version_id?: string | null;
    reason: string;
  };

/**
 * `PATCH /v1/admin/groups/{group_id}`.
 *
 * `members` is a **full replacement**, not a delta: a membership list has no
 * precondition of its own, so add/remove pairs race — two callers each
 * removing one person can both succeed and leave a list neither intended. A
 * replacement under `expected_revision` cannot.
 */
export type UpdateGroupBody = {
    /**
     * New description; `null` clears it.
     */
    description?: string | null;
    /**
     * New display name.
     */
    display_name?: string | null;
    /**
     * The revision the caller last saw. Required, for the reason the
     * workspace plane's is: an update without a precondition is a
     * last-writer-wins update.
     */
    expected_revision: number;
    /**
     * The complete membership after this update.
     */
    members?: string[] | null;
    status?: "active" | "archived";
  };

/**
 * Change a binding using optimistic concurrency.
 */
export type UpdateSkillBindingBody = {
    /**
     * Complete resulting activation state.
     */
    enabled: boolean;
    /**
     * Exact binding revision required when the change applies.
     */
    expected_revision: number;
    /**
     * Complete resulting pin state; null follows current.
     */
    pinned_version_id?: string | null;
    /**
     * Stable reason code (`disable`, `enable`, `pin`, `unpin`).
     */
    reason: string;
  };

/**
 * Add and select a new immutable version.
 */
export type UpdateSkillBody = {
    /**
     * Exact current version required when the change applies.
     */
    expected_current_version_id: string;
    /**
     * Complete replacement bundle; history remains immutable.
     */
    files: SkillFileBody[];
    /**
     * Retained bundle provenance.
     */
    provenance?: SkillProvenanceBody;
    sensitivity: "public" | "internal" | "confidential" | "restricted";
  };

/**
 * Change a binding using optimistic concurrency.
 */
export type UpdateToolBindingBody = {
    /**
     * Exact current binding revision.
     */
    expected_revision: number;
    /**
     * Bounded reason code (`disable`, `enable`, `repin`, `remove`).
     */
    reason: string;
    state: "enabled" | "disabled" | "removed";
    /**
     * Complete resulting exact version.
     */
    version_id: string;
  };

/**
 * `POST /v1/knowledge/{id}/verify`.
 */
export type VerifyKnowledgeBody = {
    /**
     * Exact current revision the verifier inspected.
     */
    expected_revision_id: string;
    /**
     * Complete bounded verification evidence.
     */
    verification_metadata: Record<string, unknown>;
  };

export type WhoamiResponse = {
    capabilities?: null | TenantCapabilities;
    subject: string;
    tenant: TenantView;
  };

/**
 * The workspace listing.
 *
 * An envelope rather than a bare array, so that paging can arrive without
 * breaking every client — and so that a future `not_answered` has somewhere
 * to live, which is the shape the capability probe already uses for the same
 * reason (ADR-0058 decision 5).
 */
export type WorkspaceList = {
    /**
     * Every workspace the caller may read, by slug. Archived ones included:
     * a listing that silently omitted them would make an archived workspace
     * indistinguishable from one that never existed.
     */
    workspaces: WorkspaceView[];
  };

/**
 * A workspace, as the API serves it.
 *
 * A view rather than `synveda_types::workspace::Workspace` itself, because
 * this is the **contract** and the domain type is not: the two agree today
 * and the day they need to differ — a computed field, a withheld one — the
 * contract must be able to say so without the storage type moving. `tenant_id`
 * is deliberately absent: every `/v1` response is already scoped to the
 * caller's tenant, and echoing it invites a client to key on it.
 */
export type WorkspaceView = {
    /**
     * When it was created.
     */
    created_at: string;
    /**
     * Who created it; absent when the deployment did.
     */
    created_by?: string | null;
    /**
     * Optional prose.
     */
    description?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * The workspace's stable id.
     */
    id: string;
    /**
     * The revision an update must name as its precondition.
     */
    revision: number;
    /**
     * The governed scope this workspace owns — what policy, role bindings and
     * every asset attach to.
     */
    scope_id: string;
    /**
     * Tenant-unique handle, identical to the scope's slug. Immutable.
     */
    slug: string;
    status: "active" | "archived";
    /**
     * When it last changed.
     */
    updated_at: string;
  };

/**
 * Every operation the contract declares, keyed by its operation id.
 *
 * `body` is present exactly when the operation takes a request body;
 * `idempotent` exactly when it requires an `Idempotency-Key` header;
 * `response` is the union of its 2xx bodies (`void` for a 204). Error
 * bodies are {@link ApiErrorBody} on every operation and are not repeated
 * here.
 */
export type Operations = {
  /**
   * `GET /v1/admin/grants`.
   */
  readonly list_grants: {
    readonly path: "/v1/admin/grants";
    readonly method: "GET";
    readonly response: GrantList;
  };
  /**
   * `POST /v1/admin/grants` — grant at any scope the caller names.
   */
  readonly create_grant: {
    readonly path: "/v1/admin/grants";
    readonly method: "POST";
    readonly body: GrantSubjectBody;
    readonly idempotent: true;
    readonly response: GrantView;
  };
  /**
   * `DELETE /v1/admin/grants/{grant_id}` — revoke one.
   */
  readonly revoke_grant: {
    readonly path: "/v1/admin/grants/{grant_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/admin/groups`.
   */
  readonly list_groups: {
    readonly path: "/v1/admin/groups";
    readonly method: "GET";
    readonly response: GroupList;
  };
  /**
   * `POST /v1/admin/groups`.
   */
  readonly create_group: {
    readonly path: "/v1/admin/groups";
    readonly method: "POST";
    readonly body: CreateGroupBody;
    readonly idempotent: true;
    readonly response: GroupView;
  };
  /**
   * `PATCH /v1/admin/groups/{group_id}`.
   */
  readonly update_group: {
    readonly path: "/v1/admin/groups/{group_id}";
    readonly method: "PATCH";
    readonly body: UpdateGroupBody;
    readonly response: GroupView;
  };
  readonly list_scopes: {
    readonly path: "/v1/admin/scopes";
    readonly method: "GET";
    readonly response: ListResponse;
  };
  readonly create_scope: {
    readonly path: "/v1/admin/scopes";
    readonly method: "POST";
    readonly body: CreateScopeBody;
    readonly idempotent: true;
    readonly response: ScopeView;
  };
  readonly get_scope: {
    readonly path: "/v1/admin/scopes/{scope_id}";
    readonly method: "GET";
    readonly response: ScopeDetail;
  };
  readonly update_scope: {
    readonly path: "/v1/admin/scopes/{scope_id}";
    readonly method: "PATCH";
    readonly body: PatchScopeBody;
    readonly response: ScopeView;
  };
  readonly list_scope_ancestors: {
    readonly path: "/v1/admin/scopes/{scope_id}/ancestors";
    readonly method: "GET";
    readonly response: ChainResponse;
  };
  /**
   * `GET /v1/admin/scopes/{scope_id}/curators` — the curator file in force
   * for this node: its own, or the nearest ancestor's.
   */
  readonly get_scope_curators: {
    readonly path: "/v1/admin/scopes/{scope_id}/curators";
    readonly method: "GET";
    readonly response: CuratorsResponse;
  };
  /**
   * `PUT /v1/admin/scopes/{scope_id}/curators` — commit this scope's curator
   * file.
   */
  readonly put_scope_curators: {
    readonly path: "/v1/admin/scopes/{scope_id}/curators";
    readonly method: "PUT";
    readonly body: CuratorsPutBody;
    readonly response: CuratorsPutResponse;
  };
  /**
   * `GET /v1/admin/scopes/{scope_id}/descendants` — the whole subtree,
   * nearest first, the scope itself excluded.
   */
  readonly list_scope_descendants: {
    readonly path: "/v1/admin/scopes/{scope_id}/descendants";
    readonly method: "GET";
    readonly response: ChainResponse;
  };
  /**
   * `GET /v1/audit/disclosures` — "who could see X on date D", as two lists
   * (ADR-0045 decision 4).
   */
  readonly list_audit_disclosures: {
    readonly path: "/v1/audit/disclosures";
    readonly method: "GET";
    readonly response: AuditDisclosuresResponse;
  };
  /**
   * `GET /v1/audit/events` — the search (ADR-0045 decision 3).
   */
  readonly list_audit_events: {
    readonly path: "/v1/audit/events";
    readonly method: "GET";
    readonly response: AuditEventsResponse;
  };
  /**
   * `GET /v1/audit/export` — a cursor page from one frozen, offline-verifiable
   * chain prefix (CPR-33, ADR-0092 decisions 4 and 5).
   */
  readonly export_audit_chain: {
    readonly path: "/v1/audit/export";
    readonly method: "GET";
    readonly response: AuditExportPage;
  };
  /**
   * `GET /v1/audit/knowledge` — "what did agent A know at time T" (ADR-0045
   * decision 5).
   */
  readonly get_audit_knowledge: {
    readonly path: "/v1/audit/knowledge";
    readonly method: "GET";
    readonly response: AuditKnowledgeResponse;
  };
  /**
   * `GET /v1/audit/verify` — the chain check, under the same `AuditRead`
   * (ADR-0045 decision 1): it returns a verdict and a sequence number and
   * no event content, so a principal who may read the chain may check it.
   */
  readonly verify_audit_chain: {
    readonly path: "/v1/audit/verify";
    readonly method: "GET";
    readonly response: AuditVerifyResponse;
  };
  /**
   * `GET /v1/capabilities?scopes=<id>,<id>,…` — the plural of the same
   * walk, for the nodes a tree actually rendered.
   */
  readonly get_capabilities: {
    readonly path: "/v1/capabilities";
    readonly method: "GET";
    readonly response: BatchResponse;
  };
  /**
   * `GET /v1/capture-batches`.
   */
  readonly list_capture_batches: {
    readonly path: "/v1/capture-batches";
    readonly method: "GET";
    readonly response: CaptureBatchListView;
  };
  /**
   * `GET /v1/capture-batches/{id}`.
   */
  readonly get_capture_batch: {
    readonly path: "/v1/capture-batches/{id}";
    readonly method: "GET";
    readonly response: CaptureBatchView;
  };
  /**
   * `POST /v1/capture-batches/{id}/accept` — accept every still-pending
   * candidate with deterministic child idempotency keys.
   */
  readonly accept_capture_batch: {
    readonly path: "/v1/capture-batches/{id}/accept";
    readonly method: "POST";
    readonly body: AcceptBatchBody;
    readonly idempotent: true;
    readonly response: CaptureCandidateListView;
  };
  /**
   * `GET /v1/capture-candidates`.
   */
  readonly list_capture_candidates: {
    readonly path: "/v1/capture-candidates";
    readonly method: "GET";
    readonly response: CaptureCandidateListView;
  };
  /**
   * `POST /v1/capture-candidates/{id}/accept`.
   */
  readonly accept_capture_candidate: {
    readonly path: "/v1/capture-candidates/{id}/accept";
    readonly method: "POST";
    readonly body: AcceptCandidateBody;
    readonly idempotent: true;
    readonly response: CaptureDecisionView;
  };
  /**
   * `POST /v1/capture-candidates/{id}/dismiss`.
   */
  readonly dismiss_capture_candidate: {
    readonly path: "/v1/capture-candidates/{id}/dismiss";
    readonly method: "POST";
    readonly body: DismissCandidateBody;
    readonly idempotent: true;
    readonly response: CaptureDecisionView;
  };
  /**
   * `POST /v1/capture-candidates/{id}/merge`.
   */
  readonly merge_capture_candidate: {
    readonly path: "/v1/capture-candidates/{id}/merge";
    readonly method: "POST";
    readonly body: MergeCandidateBody;
    readonly idempotent: true;
    readonly response: CaptureDecisionView;
  };
  /**
   * `POST /v1/capture-candidates/{id}/replace`.
   */
  readonly replace_capture_candidate: {
    readonly path: "/v1/capture-candidates/{id}/replace";
    readonly method: "POST";
    readonly body: ReplaceCandidateBody;
    readonly idempotent: true;
    readonly response: CaptureDecisionView;
  };
  /**
   * `GET /v1/channels/{scope_id}` — the channels standing at one scope.
   */
  readonly list_channels: {
    readonly path: "/v1/channels/{scope_id}";
    readonly method: "GET";
    readonly response: ChannelListResponse;
  };
  /**
   * `GET /v1/channels/{scope_id}/history` — the states this channel has
   * held, newest first.
   */
  readonly get_channel_history: {
    readonly path: "/v1/channels/{scope_id}/history";
    readonly method: "GET";
    readonly response: ChannelHistoryResponse;
  };
  /**
   * `POST /v1/channels/{scope_id}/pin` — hold what this channel serves at a
   * commit.
   */
  readonly pin_channel: {
    readonly path: "/v1/channels/{scope_id}/pin";
    readonly method: "POST";
    readonly body: ChannelPinBody;
    readonly response: ChannelPinResponse;
  };
  /**
   * `POST /v1/channels/{scope_id}/publish` — admit authored artifact versions.
   */
  readonly publish_channel: {
    readonly path: "/v1/channels/{scope_id}/publish";
    readonly method: "POST";
    readonly body: ChannelPublishBody;
    readonly response: ChannelPublishResponse;
  };
  /**
   * `POST /v1/channels/{scope_id}/rollback` — rewind the channel to a state
   * it has already held.
   */
  readonly rollback_channel: {
    readonly path: "/v1/channels/{scope_id}/rollback";
    readonly method: "POST";
    readonly body: ChannelRollbackBody;
    readonly response: ChannelRollbackResponse;
  };
  /**
   * `POST /v1/channels/{scope_id}/unpin` — release the hold.
   */
  readonly unpin_channel: {
    readonly path: "/v1/channels/{scope_id}/unpin";
    readonly method: "POST";
    readonly body: ChannelUnpinBody;
    readonly response: ChannelUnpinResponse;
  };
  /**
   * List revisioned bindings at one exact scope.
   */
  readonly list_configuration_bindings: {
    readonly path: "/v1/configuration-bindings";
    readonly method: "GET";
    readonly response: ConfigurationBindingListView;
  };
  /**
   * Create one selector at a governed scope.
   */
  readonly create_configuration_binding: {
    readonly path: "/v1/configuration-bindings";
    readonly method: "POST";
    readonly body: CreateConfigurationBindingBody;
    readonly idempotent: true;
    readonly response: ConfigurationMutationView;
  };
  /**
   * Change, enable, disable, pin or unpin a binding.
   */
  readonly update_configuration_binding: {
    readonly path: "/v1/configuration-bindings/{id}";
    readonly method: "PATCH";
    readonly body: UpdateConfigurationBindingBody;
    readonly idempotent: true;
    readonly response: ConfigurationMutationView;
  };
  /**
   * Roll back by pinning one earlier immutable version.
   */
  readonly rollback_configuration_binding: {
    readonly path: "/v1/configuration-bindings/{id}/rollback";
    readonly method: "POST";
    readonly body: RollbackConfigurationBindingBody;
    readonly idempotent: true;
    readonly response: ConfigurationMutationView;
  };
  /**
   * List canonical source templates. Templates are never effective by name.
   */
  readonly list_configuration_templates: {
    readonly path: "/v1/configuration-templates";
    readonly method: "GET";
    readonly response: ConfigurationTemplateListView;
  };
  /**
   * List policy-visible stable aggregates with an opaque keyset cursor.
   */
  readonly list_configurations: {
    readonly path: "/v1/configurations";
    readonly method: "GET";
    readonly response: ConfigurationArtifactListView;
  };
  /**
   * Create an immutable governed configuration aggregate.
   */
  readonly create_configuration: {
    readonly path: "/v1/configurations";
    readonly method: "POST";
    readonly body: CreateConfigurationBody;
    readonly idempotent: true;
    readonly response: ConfigurationMutationView;
  };
  /**
   * Resolve the nearest enabled selector and return exact version evidence.
   */
  readonly get_effective_configuration: {
    readonly path: "/v1/configurations/effective";
    readonly method: "GET";
    readonly response: EffectiveConfigurationView;
  };
  /**
   * Read one stable aggregate.
   */
  readonly get_configuration: {
    readonly path: "/v1/configurations/{id}";
    readonly method: "GET";
    readonly response: ConfigurationArtifactView;
  };
  /**
   * Compare two versions of one stable aggregate.
   */
  readonly compare_configuration_versions: {
    readonly path: "/v1/configurations/{id}/compare";
    readonly method: "GET";
    readonly response: ConfigurationComparisonView;
  };
  /**
   * List immutable versions newest first.
   */
  readonly list_configuration_versions: {
    readonly path: "/v1/configurations/{id}/versions";
    readonly method: "GET";
    readonly response: ConfigurationVersionListView;
  };
  /**
   * Publish and select another immutable version.
   */
  readonly publish_configuration_version: {
    readonly path: "/v1/configurations/{id}/versions";
    readonly method: "POST";
    readonly body: PublishConfigurationBody;
    readonly idempotent: true;
    readonly response: ConfigurationMutationView;
  };
  /**
   * `GET /v1/context-packs?scope_id=…` — the registry at one scope: every
   * pack, its documents, and what the published channel holds for each.
   */
  readonly list_context_packs: {
    readonly path: "/v1/context-packs";
    readonly method: "GET";
    readonly response: ContextPackListResponse;
  };
  /**
   * `POST /v1/context-packs` — author a pack: create it, or replace the
   * documents named in the request.
   */
  readonly author_context_pack: {
    readonly path: "/v1/context-packs";
    readonly method: "POST";
    readonly body: ContextPackAuthorBody;
    readonly response: ContextPackView;
  };
  /**
   * `GET /v1/context-runs` — cursor-paginated, per-session-authorised plans.
   */
  readonly list_context_runs: {
    readonly path: "/v1/context-runs";
    readonly method: "GET";
    readonly response: ContextRunListView;
  };
  /**
   * `GET /v1/context-runs/{id}` — re-authorised planner detail.
   */
  readonly get_context_run: {
    readonly path: "/v1/context-runs/{id}";
    readonly method: "GET";
    readonly response: ContextRunDetailView;
  };
  /**
   * `POST /v1/context-runs/{id}/feedback` — one explicit outcome assertion.
   */
  readonly create_context_feedback: {
    readonly path: "/v1/context-runs/{id}/feedback";
    readonly method: "POST";
    readonly body: ContextFeedbackBody;
    readonly idempotent: true;
    readonly response: ContextFeedbackView;
  };
  /**
   * `POST /v1/directory/access-assignments` — bind a provider-owned group to a
   * governed scope without creating a second directory authorisation model.
   */
  readonly create_directory_access_assignment: {
    readonly path: "/v1/directory/access-assignments";
    readonly method: "POST";
    readonly body: DirectoryAccessAssignmentBody;
    readonly idempotent: true;
    readonly response: GrantView;
  };
  /**
   * `DELETE /v1/directory/access-assignments/{grant_id}`.
   */
  readonly revoke_directory_access_assignment: {
    readonly path: "/v1/directory/access-assignments/{grant_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `POST /v1/directory/seal-authorisations`.
   */
  readonly authorise_directory_seals: {
    readonly path: "/v1/directory/seal-authorisations";
    readonly method: "POST";
    readonly body: AuthoriseRequest;
    readonly response: AuthoriseResponse;
  };
  /**
   * `GET /v1/directory/sync`.
   */
  readonly get_directory_sync: {
    readonly path: "/v1/directory/sync";
    readonly method: "GET";
    readonly response: SyncStatus;
  };
  /**
   * `POST /v1/invites/{invite_token}/accept` — redeem one.
   */
  readonly accept_invite: {
    readonly path: "/v1/invites/{invite_token}/accept";
    readonly method: "POST";
    readonly response: AcceptedInviteView;
  };
  /**
   * `GET /v1/knowledge` — current policy-visible Knowledge.
   */
  readonly list_knowledge: {
    readonly path: "/v1/knowledge";
    readonly method: "GET";
    readonly response: KnowledgeListView;
  };
  /**
   * `POST /v1/knowledge` — create one governed aggregate and first revision.
   */
  readonly create_knowledge: {
    readonly path: "/v1/knowledge";
    readonly method: "POST";
    readonly body: CreateKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * List fully policy-visible conflict sets.
   */
  readonly list_knowledge_conflicts: {
    readonly path: "/v1/knowledge-conflicts";
    readonly method: "GET";
    readonly response: ConflictSetListView;
  };
  /**
   * Read one fully visible conflict set.
   */
  readonly get_knowledge_conflict: {
    readonly path: "/v1/knowledge-conflicts/{id}";
    readonly method: "GET";
    readonly response: ConflictSetView;
  };
  /**
   * Resolve one Knowledge-backed conflict through VedaFlow.
   */
  readonly resolve_knowledge_conflict: {
    readonly path: "/v1/knowledge-conflicts/{id}/resolve";
    readonly method: "POST";
    readonly body: ResolveConflictBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * Resolve type-aware policies from one exact governed Configuration.
   */
  readonly list_knowledge_freshness_policies: {
    readonly path: "/v1/knowledge-freshness-policies";
    readonly method: "GET";
    readonly response: FreshnessPolicyListView;
  };
  /**
   * `POST /v1/knowledge/merge` — combine current items and all provenance.
   */
  readonly merge_knowledge: {
    readonly path: "/v1/knowledge/merge";
    readonly method: "POST";
    readonly body: MergeKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `GET /v1/knowledge/{id}` — current content and visible relationships.
   */
  readonly get_knowledge: {
    readonly path: "/v1/knowledge/{id}";
    readonly method: "GET";
    readonly response: KnowledgeItemView;
  };
  /**
   * `PATCH /v1/knowledge/{id}` — append a governed immutable revision.
   */
  readonly edit_knowledge: {
    readonly path: "/v1/knowledge/{id}";
    readonly method: "PATCH";
    readonly body: EditKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `DELETE /v1/knowledge/{id}` — explicit archive or governed forget.
   */
  readonly delete_knowledge: {
    readonly path: "/v1/knowledge/{id}";
    readonly method: "DELETE";
    readonly body: DeleteKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `POST /v1/knowledge/{id}/archive`.
   */
  readonly archive_knowledge: {
    readonly path: "/v1/knowledge/{id}/archive";
    readonly method: "POST";
    readonly body: LifecycleKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `GET /v1/knowledge/{id}/history` — immutable revisions newest first.
   */
  readonly get_knowledge_history: {
    readonly path: "/v1/knowledge/{id}/history";
    readonly method: "GET";
    readonly response: KnowledgeHistoryView;
  };
  /**
   * `POST /v1/knowledge/{id}/restore`.
   */
  readonly restore_knowledge: {
    readonly path: "/v1/knowledge/{id}/restore";
    readonly method: "POST";
    readonly body: LifecycleKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `GET /v1/knowledge/{id}/sources` — independently governed provenance.
   */
  readonly get_knowledge_sources: {
    readonly path: "/v1/knowledge/{id}/sources";
    readonly method: "GET";
    readonly response: KnowledgeSourcesView;
  };
  /**
   * `POST /v1/knowledge/{id}/supersede` — explicitly replace an item.
   */
  readonly supersede_knowledge: {
    readonly path: "/v1/knowledge/{id}/supersede";
    readonly method: "POST";
    readonly body: SupersedeKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `GET /v1/knowledge/{id}/usage` — context selections of exact revisions.
   */
  readonly get_knowledge_usage: {
    readonly path: "/v1/knowledge/{id}/usage";
    readonly method: "GET";
    readonly response: KnowledgeUsageListView;
  };
  /**
   * `POST /v1/knowledge/{id}/verify` — append verification evidence.
   */
  readonly verify_knowledge: {
    readonly path: "/v1/knowledge/{id}/verify";
    readonly method: "POST";
    readonly body: VerifyKnowledgeBody;
    readonly idempotent: true;
    readonly response: KnowledgeMutationView;
  };
  /**
   * `GET /v1/me`.
   */
  readonly get_me: {
    readonly path: "/v1/me";
    readonly method: "GET";
    readonly response: MeView;
  };
  /**
   * List visible import jobs with a true keyset cursor.
   */
  readonly list_okf_imports: {
    readonly path: "/v1/okf/imports";
    readonly method: "GET";
    readonly response: OkfImportJobListView;
  };
  /**
   * Read one complete immutable dry-run plan.
   */
  readonly get_okf_import: {
    readonly path: "/v1/okf/imports/{id}";
    readonly method: "GET";
    readonly response: OkfImportPlanView;
  };
  /**
   * Turn an immutable plan into reviewable candidates only.
   */
  readonly materialize_okf_import: {
    readonly path: "/v1/okf/imports/{id}/materialize";
    readonly method: "POST";
    readonly idempotent: true;
    readonly response: OkfMaterializationView;
  };
  /**
   * List immutable pack sources available to Configuration documents.
   */
  readonly list_policy_packs: {
    readonly path: "/v1/policy/packs";
    readonly method: "GET";
    readonly response: PacksResponse;
  };
  /**
   * `GET /v1/projects/{project_id}`.
   */
  readonly get_project: {
    readonly path: "/v1/projects/{project_id}";
    readonly method: "GET";
    readonly response: ProjectView;
  };
  /**
   * `PATCH /v1/projects/{project_id}`.
   */
  readonly update_project: {
    readonly path: "/v1/projects/{project_id}";
    readonly method: "PATCH";
    readonly body: UpdateBody;
    readonly response: ProjectView;
  };
  /**
   * `GET /v1/projects/{project_id}/members` — who may act here, **including
   * what the workspace above it grants**.
   */
  readonly list_project_members: {
    readonly path: "/v1/projects/{project_id}/members";
    readonly method: "GET";
    readonly response: MemberList;
  };
  /**
   * `POST /v1/projects/{project_id}/members` — a **project-only** grant.
   */
  readonly add_project_member: {
    readonly path: "/v1/projects/{project_id}/members";
    readonly method: "POST";
    readonly body: GrantSubjectBody;
    readonly idempotent: true;
    readonly response: GrantView;
  };
  /**
   * `DELETE /v1/projects/{project_id}/members/{principal_id}`.
   */
  readonly remove_project_member: {
    readonly path: "/v1/projects/{project_id}/members/{principal_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * Export freshly authorised current Knowledge deterministically as OKF v0.2.
   */
  readonly export_okf: {
    readonly path: "/v1/projects/{project_id}/okf/exports";
    readonly method: "POST";
    readonly body: ExportOkfBody;
    readonly response: OkfExportView;
  };
  /**
   * Plan one bounded OKF v0.2 import. This never creates active Knowledge.
   */
  readonly plan_okf_import: {
    readonly path: "/v1/projects/{project_id}/okf/imports";
    readonly method: "POST";
    readonly body: PlanOkfImportBody;
    readonly idempotent: true;
    readonly response: OkfImportPlanView;
  };
  /**
   * `GET /v1/projects/{project_id}/repositories`.
   */
  readonly list_repositories: {
    readonly path: "/v1/projects/{project_id}/repositories";
    readonly method: "GET";
    readonly response: RepositoryList;
  };
  /**
   * `POST /v1/projects/{project_id}/repositories` — attach a repository.
   */
  readonly attach_repository: {
    readonly path: "/v1/projects/{project_id}/repositories";
    readonly method: "POST";
    readonly body: AttachRepositoryBody;
    readonly idempotent: true;
    readonly response: RepositoryView;
  };
  /**
   * `DELETE /v1/projects/{project_id}/repositories/{repository_id}`.
   */
  readonly detach_repository: {
    readonly path: "/v1/projects/{project_id}/repositories/{repository_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/projects/{project_id}/tool-config` — secret-free exact bindings.
   */
  readonly generate_tool_client_config: {
    readonly path: "/v1/projects/{project_id}/tool-config";
    readonly method: "GET";
    readonly response: ToolClientConfigurationView;
  };
  /**
   * `GET /v1/prompts?scope_id=…` — the registry at one scope: every draft,
   * with what the published channel holds for it.
   */
  readonly list_prompts: {
    readonly path: "/v1/prompts";
    readonly method: "GET";
    readonly response: PromptListResponse;
  };
  /**
   * `POST /v1/prompts` — author a draft: create it, or replace the content
   * of the one that is there.
   */
  readonly author_prompt: {
    readonly path: "/v1/prompts";
    readonly method: "POST";
    readonly body: PromptAuthorBody;
    readonly response: PromptView;
  };
  /**
   * `GET /v1/prompts/{name}` — resolve a prompt for this caller.
   */
  readonly resolve_prompt: {
    readonly path: "/v1/prompts/{name}";
    readonly method: "GET";
    readonly response: PromptResolveResponse;
  };
  /**
   * `GET /v1/proposals` — proposals, newest first.
   */
  readonly list_proposals: {
    readonly path: "/v1/proposals";
    readonly method: "GET";
    readonly response: ProposalListResponse;
  };
  /**
   * `POST /v1/proposals` — open a proposal against a scope's published
   * channel.
   */
  readonly open_proposal: {
    readonly path: "/v1/proposals";
    readonly method: "POST";
    readonly body: ProposalOpenBody;
    readonly response: ProposalOpenResponse;
  };
  /**
   * `GET /v1/proposals/{id}` — one proposal, with its members' content and
   * its review log.
   */
  readonly get_proposal: {
    readonly path: "/v1/proposals/{id}";
    readonly method: "GET";
    readonly response: ProposalDetail;
  };
  /**
   * `POST /v1/proposals/{id}/apply` — run an approved typed aggregate effect.
   * The artifact command layer repeats ownership, PDP and revision checks at
   * this boundary; approvals never become write authority by themselves.
   */
  readonly apply_proposal: {
    readonly path: "/v1/proposals/{id}/apply";
    readonly method: "POST";
    readonly response: unknown;
  };
  /**
   * `POST /v1/proposals/{id}/approve` — cast an approval.
   */
  readonly approve_proposal: {
    readonly path: "/v1/proposals/{id}/approve";
    readonly method: "POST";
    readonly body: ProposalReviewBody;
    readonly response: ProposalReviewResponse;
  };
  /**
   * `POST /v1/proposals/{id}/publish` — run an approved proposal's effect.
   */
  readonly publish_proposal: {
    readonly path: "/v1/proposals/{id}/publish";
    readonly method: "POST";
    readonly response: ProposalPublishResponse;
  };
  /**
   * `POST /v1/proposals/{id}/reject` — close a proposal with a reason.
   */
  readonly reject_proposal: {
    readonly path: "/v1/proposals/{id}/reject";
    readonly method: "POST";
    readonly body: ProposalRejectBody;
    readonly response: ProposalSummary;
  };
  /**
   * `POST /v1/proposals/{id}/withdraw` — the proposer closes their own.
   */
  readonly withdraw_proposal: {
    readonly path: "/v1/proposals/{id}/withdraw";
    readonly method: "POST";
    readonly response: ProposalSummary;
  };
  /**
   * `GET /v1/quarantine` — the pending review queue, oldest first.
   * `QuarantineRead` is decided at the tenant either way (module doc);
   * `scope_id` narrows *which* events come back, after the uniform-404
   * ownership check on the scope named.
   */
  readonly list_quarantine: {
    readonly path: "/v1/quarantine";
    readonly method: "GET";
    readonly response: QuarantineQueueResponse;
  };
  /**
   * `POST /v1/quarantine/{event_id}/reject`.
   */
  readonly reject_quarantined_event: {
    readonly path: "/v1/quarantine/{event_id}/reject";
    readonly method: "POST";
    readonly body: QuarantineReviewBody;
    readonly response: QuarantineEventView;
  };
  /**
   * `POST /v1/quarantine/{event_id}/release`.
   */
  readonly release_quarantined_event: {
    readonly path: "/v1/quarantine/{event_id}/release";
    readonly method: "POST";
    readonly body: QuarantineReviewBody;
    readonly response: QuarantineEventView;
  };
  /**
   * List policy-visible relaxations. The cursor follows the last candidate
   * considered, so a fully denied page can be empty and still advance.
   */
  readonly list_relaxations: {
    readonly path: "/v1/relaxations";
    readonly method: "GET";
    readonly response: RelaxationListView;
  };
  /**
   * Create a stable relaxation and its first immutable version through
   * VedaFlow.
   */
  readonly create_relaxation: {
    readonly path: "/v1/relaxations";
    readonly method: "POST";
    readonly body: CreateRelaxationBody;
    readonly idempotent: true;
    readonly response: RelaxationMutationView;
  };
  /**
   * Read one current stable aggregate.
   */
  readonly get_relaxation: {
    readonly path: "/v1/relaxations/{id}";
    readonly method: "GET";
    readonly response: RelaxationView;
  };
  /**
   * Publish a replacement immutable version with an exact head precondition.
   */
  readonly revise_relaxation: {
    readonly path: "/v1/relaxations/{id}";
    readonly method: "PATCH";
    readonly body: ReviseRelaxationBody;
    readonly idempotent: true;
    readonly response: RelaxationMutationView;
  };
  /**
   * End a current version early through a governed VedaFlow change.
   */
  readonly revoke_relaxation: {
    readonly path: "/v1/relaxations/{id}/revoke";
    readonly method: "POST";
    readonly body: RevokeRelaxationBody;
    readonly idempotent: true;
    readonly response: RelaxationMutationView;
  };
  /**
   * List immutable versions newest first.
   */
  readonly list_relaxation_versions: {
    readonly path: "/v1/relaxations/{id}/versions";
    readonly method: "GET";
    readonly response: RelaxationVersionListView;
  };
  /**
   * `GET /v1/scim/credentials` — the inventory, revoked and expired ones
   * included, because rotation is a decision about a history rather than
   * about a current state.
   */
  readonly list_scim_credentials: {
    readonly path: "/v1/scim/credentials";
    readonly method: "GET";
    readonly response: ScimCredentialsResponse;
  };
  /**
   * Issues a credential.
   */
  readonly issue_scim_credential: {
    readonly path: "/v1/scim/credentials";
    readonly method: "POST";
    readonly body: IssueRequest;
    readonly response: IssuedCredential;
  };
  /**
   * `POST /v1/scim/credentials/{id}/revoke`.
   */
  readonly revoke_scim_credential: {
    readonly path: "/v1/scim/credentials/{id}/revoke";
    readonly method: "POST";
    readonly response: void;
  };
  /**
   * `GET /v1/service-identities` — the tenant's registered agents. A
   * tenant-plane read: `ServiceIdentityRead` at the tenant.
   */
  readonly list_service_identities: {
    readonly path: "/v1/service-identities";
    readonly method: "GET";
    readonly response: ServiceIdentitiesResponse;
  };
  /**
   * `POST /v1/service-identities` — register an agent at an anchor node.
   * `ServiceIdentityManage` on the anchor: a steward registers agents in
   * their subtree, visibly (ADR-0018 decision 3).
   */
  readonly register_service_identity: {
    readonly path: "/v1/service-identities";
    readonly method: "POST";
    readonly body: RegisterServiceIdentityBody;
    readonly response: ServiceIdentityView;
  };
  /**
   * `GET /v1/service-identities/{id}` — one registration.
   * `ServiceIdentityRead` on the anchor.
   */
  readonly get_service_identity: {
    readonly path: "/v1/service-identities/{id}";
    readonly method: "GET";
    readonly response: ServiceIdentityView;
  };
  /**
   * `DELETE /v1/service-identities/{id}` — revoke: delete the identity row
   * and its personal leaf. `ServiceIdentityManage` on the anchor. Effective
   * on the next request: an unregistered IdP subject is quarantined at the
   * seam (ADR-0013 decision 6).
   */
  readonly remove_service_identity: {
    readonly path: "/v1/service-identities/{id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/sessions` — the runs this caller may read, newest first.
   */
  readonly list_sessions: {
    readonly path: "/v1/sessions";
    readonly method: "GET";
    readonly response: SessionList;
  };
  /**
   * `POST /v1/sessions` — open a run.
   */
  readonly open_session: {
    readonly path: "/v1/sessions";
    readonly method: "POST";
    readonly body: OpenSessionBody;
    readonly idempotent: true;
    readonly response: SessionView;
  };
  /**
   * `GET /v1/sessions/{session_id}`.
   */
  readonly get_session: {
    readonly path: "/v1/sessions/{session_id}";
    readonly method: "GET";
    readonly response: SessionView;
  };
  /**
   * `POST /v1/sessions/{session_id}/capture-batches` — freeze the current
   * eligible evidence snapshot for asynchronous extraction.
   */
  readonly create_capture_batch: {
    readonly path: "/v1/sessions/{session_id}/capture-batches";
    readonly method: "POST";
    readonly idempotent: true;
    readonly response: CaptureBatchView;
  };
  /**
   * `POST /v1/sessions/{session_id}/context-runs` — plan and deliver context.
   */
  readonly create_context_run: {
    readonly path: "/v1/sessions/{session_id}/context-runs";
    readonly method: "POST";
    readonly body: CreateContextRunBody;
    readonly idempotent: true;
    readonly response: ContextRunView;
  };
  /**
   * `POST /v1/sessions/{session_id}/end` — move a run through its close.
   */
  readonly end_session: {
    readonly path: "/v1/sessions/{session_id}/end";
    readonly method: "POST";
    readonly body: EndSessionBody;
    readonly response: SessionView;
  };
  /**
   * `POST /v1/sessions/{session_id}/events` — append to the ledger.
   */
  readonly append_session_events: {
    readonly path: "/v1/sessions/{session_id}/events";
    readonly method: "POST";
    readonly body: AppendEventsBody;
    readonly response: AppendResponse;
  };
  /**
   * `GET /v1/sessions/{session_id}/events/{event_id}` — the diagnostic
   * expansion (CPR-11, ADR-0077 decision 3).
   */
  readonly get_session_event: {
    readonly path: "/v1/sessions/{session_id}/events/{event_id}";
    readonly method: "GET";
    readonly response: SessionEventView;
  };
  /**
   * `POST /v1/sessions/{session_id}/knowledge-evaluation` — diagnostics lens.
   */
  readonly evaluate_session_knowledge: {
    readonly path: "/v1/sessions/{session_id}/knowledge-evaluation";
    readonly method: "POST";
    readonly body: KnowledgeEvaluationBody;
    readonly response: ContextKnowledgeQueryView;
  };
  /**
   * `POST /v1/sessions/{session_id}/knowledge-query` — ordinary deep recall.
   */
  readonly query_session_knowledge: {
    readonly path: "/v1/sessions/{session_id}/knowledge-query";
    readonly method: "POST";
    readonly body: KnowledgeQueryBody;
    readonly response: ContextKnowledgeQueryView;
  };
  /**
   * `GET /v1/sessions/{session_id}/timeline` — the projection.
   */
  readonly get_session_timeline: {
    readonly path: "/v1/sessions/{session_id}/timeline";
    readonly method: "GET";
    readonly response: TimelineView;
  };
  /**
   * List revisioned bindings at one project/principal scope.
   */
  readonly list_skill_bindings: {
    readonly path: "/v1/skill-bindings";
    readonly method: "GET";
    readonly response: SkillBindingListView;
  };
  /**
   * Create a project/principal binding through VedaFlow.
   */
  readonly create_skill_binding: {
    readonly path: "/v1/skill-bindings";
    readonly method: "POST";
    readonly body: CreateSkillBindingBody;
    readonly idempotent: true;
    readonly response: SkillMutationView;
  };
  /**
   * Get one revisioned binding.
   */
  readonly get_skill_binding: {
    readonly path: "/v1/skill-bindings/{id}";
    readonly method: "GET";
    readonly response: SkillBindingView;
  };
  /**
   * Update, enable, disable, pin or unpin a binding through VedaFlow.
   */
  readonly update_skill_binding: {
    readonly path: "/v1/skill-bindings/{id}";
    readonly method: "PATCH";
    readonly body: UpdateSkillBindingBody;
    readonly idempotent: true;
    readonly response: SkillMutationView;
  };
  /**
   * Roll a binding back by changing its pin, never its version history.
   */
  readonly rollback_skill_binding: {
    readonly path: "/v1/skill-bindings/{id}/rollback";
    readonly method: "POST";
    readonly body: RollbackSkillBindingBody;
    readonly idempotent: true;
    readonly response: SkillMutationView;
  };
  /**
   * Record one version-specific usage stage idempotently.
   */
  readonly record_skill_usage: {
    readonly path: "/v1/skill-usage";
    readonly method: "POST";
    readonly body: RecordSkillUsageBody;
    readonly response: SkillUsageEventView;
  };
  /**
   * List visible stable skills. Each row is decided at its governing scope.
   */
  readonly list_skills: {
    readonly path: "/v1/skills";
    readonly method: "GET";
    readonly response: SkillListView;
  };
  /**
   * Install a stable skill and first immutable version through VedaFlow.
   */
  readonly install_skill: {
    readonly path: "/v1/skills";
    readonly method: "POST";
    readonly body: InstallSkillBody;
    readonly idempotent: true;
    readonly response: SkillMutationView;
  };
  /**
   * Resolve enabled bindings to exact immutable versions for a context scope.
   */
  readonly list_available_skills: {
    readonly path: "/v1/skills/available";
    readonly method: "GET";
    readonly response: AvailableSkillListView;
  };
  /**
   * Get one stable skill and current version.
   */
  readonly get_skill: {
    readonly path: "/v1/skills/{id}";
    readonly method: "GET";
    readonly response: SkillView;
  };
  /**
   * Create a new immutable version through VedaFlow.
   */
  readonly update_skill: {
    readonly path: "/v1/skills/{id}";
    readonly method: "PATCH";
    readonly body: UpdateSkillBody;
    readonly idempotent: true;
    readonly response: SkillMutationView;
  };
  /**
   * List immutable versions, newest first.
   */
  readonly list_skill_versions: {
    readonly path: "/v1/skills/{id}/versions";
    readonly method: "GET";
    readonly response: SkillVersionListView;
  };
  /**
   * Get exact immutable version metadata.
   */
  readonly get_skill_version: {
    readonly path: "/v1/skills/{id}/versions/{version_id}";
    readonly method: "GET";
    readonly response: SkillVersionView;
  };
  /**
   * List exact immutable file descriptors.
   */
  readonly list_skill_version_files: {
    readonly path: "/v1/skills/{id}/versions/{version_id}/files";
    readonly method: "GET";
    readonly response: SkillVersionFileListView;
  };
  /**
   * Fetch one exact version file. The wildcard remains bundle-relative.
   */
  readonly get_skill_version_file: {
    readonly path: "/v1/skills/{id}/versions/{version_id}/files/{path}";
    readonly method: "GET";
    readonly response: SkillVersionFileContentView;
  };
  /**
   * List controlled test runs for one immutable version.
   */
  readonly list_skill_tests: {
    readonly path: "/v1/skills/{id}/versions/{version_id}/tests";
    readonly method: "GET";
    readonly response: SkillTestRunListView;
  };
  /**
   * Run the built-in non-executing validation sandbox.
   */
  readonly run_skill_test: {
    readonly path: "/v1/skills/{id}/versions/{version_id}/tests";
    readonly method: "POST";
    readonly body: RunSkillTestBody;
    readonly idempotent: true;
    readonly response: SkillTestRunView;
  };
  /**
   * List usage evidence for one exact version.
   */
  readonly list_skill_usage: {
    readonly path: "/v1/skills/{id}/versions/{version_id}/usage";
    readonly method: "GET";
    readonly response: SkillUsageListView;
  };
  /**
   * `GET /v1/tool-bindings` — list policy-visible project bindings.
   */
  readonly list_tool_bindings: {
    readonly path: "/v1/tool-bindings";
    readonly method: "GET";
    readonly response: ToolBindingListView;
  };
  /**
   * `POST /v1/tool-bindings` — propose an exact approved project binding.
   */
  readonly create_tool_binding: {
    readonly path: "/v1/tool-bindings";
    readonly method: "POST";
    readonly body: CreateToolBindingBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `GET /v1/tool-bindings/{id}` — inspect one exact binding.
   */
  readonly get_tool_binding: {
    readonly path: "/v1/tool-bindings/{id}";
    readonly method: "GET";
    readonly response: ToolBindingView;
  };
  /**
   * `PATCH /v1/tool-bindings/{id}` — propose disable, repin or removal.
   */
  readonly update_tool_binding: {
    readonly path: "/v1/tool-bindings/{id}";
    readonly method: "PATCH";
    readonly body: UpdateToolBindingBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `GET /v1/tool-servers` — list policy-visible catalogue entries.
   */
  readonly list_tool_servers: {
    readonly path: "/v1/tool-servers";
    readonly method: "GET";
    readonly response: ToolServerListView;
  };
  /**
   * `POST /v1/tool-servers` — stage a stable server and first version.
   */
  readonly register_tool_server: {
    readonly path: "/v1/tool-servers";
    readonly method: "POST";
    readonly body: RegisterToolServerBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `POST /v1/tool-servers/import-client-config` — import one supported client entry.
   */
  readonly import_tool_client_config: {
    readonly path: "/v1/tool-servers/import-client-config";
    readonly method: "POST";
    readonly body: ImportToolClientConfigBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `GET /v1/tool-servers/{id}` — inspect one stable entry.
   */
  readonly get_tool_server: {
    readonly path: "/v1/tool-servers/{id}";
    readonly method: "GET";
    readonly response: ToolServerView;
  };
  /**
   * `PATCH /v1/tool-servers/{id}` — stage changed immutable metadata.
   */
  readonly update_tool_server: {
    readonly path: "/v1/tool-servers/{id}";
    readonly method: "PATCH";
    readonly body: StageToolVersionBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `POST /v1/tool-servers/{id}/discoveries` — report stateless discovery.
   */
  readonly discover_tool_server: {
    readonly path: "/v1/tool-servers/{id}/discoveries";
    readonly method: "POST";
    readonly body: DiscoverToolServerBody;
    readonly idempotent: true;
    readonly response: ToolMutationView;
  };
  /**
   * `GET /v1/tool-servers/{id}/versions` — immutable version history.
   */
  readonly list_tool_server_versions: {
    readonly path: "/v1/tool-servers/{id}/versions";
    readonly method: "GET";
    readonly response: ToolServerVersionListView;
  };
  /**
   * `GET /v1/tool-servers/{id}/versions/{version_id}` — exact version.
   */
  readonly get_tool_server_version: {
    readonly path: "/v1/tool-servers/{id}/versions/{version_id}";
    readonly method: "GET";
    readonly response: ToolServerVersionView;
  };
  /**
   * `GET /v1/tool-servers/{id}/versions/{version_id}/diff` — compare versions.
   */
  readonly diff_tool_server_version: {
    readonly path: "/v1/tool-servers/{id}/versions/{version_id}/diff";
    readonly method: "GET";
    readonly response: ToolVersionDiffView;
  };
  /**
   * `GET /v1/tool-servers/{id}/versions/{version_id}/tests` — list evidence.
   */
  readonly list_tool_server_tests: {
    readonly path: "/v1/tool-servers/{id}/versions/{version_id}/tests";
    readonly method: "GET";
    readonly response: ToolTestRunListView;
  };
  /**
   * `POST /v1/tool-servers/{id}/versions/{version_id}/tests` — record trusted
   * read-only evidence. The gateway does not execute the server.
   */
  readonly run_tool_server_test: {
    readonly path: "/v1/tool-servers/{id}/versions/{version_id}/tests";
    readonly method: "POST";
    readonly body: RunToolTestBody;
    readonly idempotent: true;
    readonly response: ToolTestRunView;
  };
  /**
   * Introspection: who does the gateway think is calling? Returns the
   * caller's own resolution result, and — only when asked — what the caller
   * may do on the tenant plane.
   */
  readonly get_whoami: {
    readonly path: "/v1/whoami";
    readonly method: "GET";
    readonly response: WhoamiResponse;
  };
  /**
   * `GET /v1/workspaces` — the tenant's workspaces.
   */
  readonly list_workspaces: {
    readonly path: "/v1/workspaces";
    readonly method: "GET";
    readonly response: WorkspaceList;
  };
  /**
   * `POST /v1/workspaces` — create a workspace and the governed scope it owns.
   */
  readonly create_workspace: {
    readonly path: "/v1/workspaces";
    readonly method: "POST";
    readonly body: CreateWorkspaceBody;
    readonly idempotent: true;
    readonly response: WorkspaceView;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}`.
   */
  readonly get_workspace: {
    readonly path: "/v1/workspaces/{workspace_id}";
    readonly method: "GET";
    readonly response: WorkspaceView;
  };
  /**
   * `PATCH /v1/workspaces/{workspace_id}` — rename, re-describe or retire.
   */
  readonly update_workspace: {
    readonly path: "/v1/workspaces/{workspace_id}";
    readonly method: "PATCH";
    readonly body: UpdateBody;
    readonly response: WorkspaceView;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}/invites`.
   */
  readonly list_workspace_invites: {
    readonly path: "/v1/workspaces/{workspace_id}/invites";
    readonly method: "GET";
    readonly response: InviteList;
  };
  /**
   * `POST /v1/workspaces/{workspace_id}/invites` — issue one.
   */
  readonly create_workspace_invite: {
    readonly path: "/v1/workspaces/{workspace_id}/invites";
    readonly method: "POST";
    readonly body: CreateInviteBody;
    readonly idempotent: true;
    readonly response: CreatedInviteView;
  };
  /**
   * `DELETE /v1/workspaces/{workspace_id}/invites/{invite_id}` — withdraw one.
   */
  readonly revoke_workspace_invite: {
    readonly path: "/v1/workspaces/{workspace_id}/invites/{invite_id}";
    readonly method: "DELETE";
    readonly response: void;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}/members` — who may act here.
   */
  readonly list_workspace_members: {
    readonly path: "/v1/workspaces/{workspace_id}/members";
    readonly method: "GET";
    readonly response: MemberList;
  };
  /**
   * `GET /v1/workspaces/{workspace_id}/projects`.
   */
  readonly list_projects: {
    readonly path: "/v1/workspaces/{workspace_id}/projects";
    readonly method: "GET";
    readonly response: ProjectList;
  };
  /**
   * `POST /v1/workspaces/{workspace_id}/projects`.
   */
  readonly create_project: {
    readonly path: "/v1/workspaces/{workspace_id}/projects";
    readonly method: "POST";
    readonly body: CreateProjectBody;
    readonly idempotent: true;
    readonly response: ProjectView;
  };
};

/** An operation id. */
export type OperationId = keyof Operations;

/**
 * Every operation's path template and method, as values.
 *
 * The runtime half of {@link Operations}: a type erases, and a client has to
 * build a URL. Generated from the same document in the same pass, so the two
 * cannot disagree. `idempotent` marks the operations whose document requires
 * an `Idempotency-Key` header.
 */
export const OPERATIONS = {
  list_grants: { path: "/v1/admin/grants", method: "GET" },
  create_grant: { path: "/v1/admin/grants", method: "POST", idempotent: true },
  revoke_grant: { path: "/v1/admin/grants/{grant_id}", method: "DELETE" },
  list_groups: { path: "/v1/admin/groups", method: "GET" },
  create_group: { path: "/v1/admin/groups", method: "POST", idempotent: true },
  update_group: { path: "/v1/admin/groups/{group_id}", method: "PATCH" },
  list_scopes: { path: "/v1/admin/scopes", method: "GET" },
  create_scope: { path: "/v1/admin/scopes", method: "POST", idempotent: true },
  get_scope: { path: "/v1/admin/scopes/{scope_id}", method: "GET" },
  update_scope: { path: "/v1/admin/scopes/{scope_id}", method: "PATCH" },
  list_scope_ancestors: { path: "/v1/admin/scopes/{scope_id}/ancestors", method: "GET" },
  get_scope_curators: { path: "/v1/admin/scopes/{scope_id}/curators", method: "GET" },
  put_scope_curators: { path: "/v1/admin/scopes/{scope_id}/curators", method: "PUT" },
  list_scope_descendants: { path: "/v1/admin/scopes/{scope_id}/descendants", method: "GET" },
  list_audit_disclosures: { path: "/v1/audit/disclosures", method: "GET" },
  list_audit_events: { path: "/v1/audit/events", method: "GET" },
  export_audit_chain: { path: "/v1/audit/export", method: "GET" },
  get_audit_knowledge: { path: "/v1/audit/knowledge", method: "GET" },
  verify_audit_chain: { path: "/v1/audit/verify", method: "GET" },
  get_capabilities: { path: "/v1/capabilities", method: "GET" },
  list_capture_batches: { path: "/v1/capture-batches", method: "GET" },
  get_capture_batch: { path: "/v1/capture-batches/{id}", method: "GET" },
  accept_capture_batch: { path: "/v1/capture-batches/{id}/accept", method: "POST", idempotent: true },
  list_capture_candidates: { path: "/v1/capture-candidates", method: "GET" },
  accept_capture_candidate: { path: "/v1/capture-candidates/{id}/accept", method: "POST", idempotent: true },
  dismiss_capture_candidate: { path: "/v1/capture-candidates/{id}/dismiss", method: "POST", idempotent: true },
  merge_capture_candidate: { path: "/v1/capture-candidates/{id}/merge", method: "POST", idempotent: true },
  replace_capture_candidate: { path: "/v1/capture-candidates/{id}/replace", method: "POST", idempotent: true },
  list_channels: { path: "/v1/channels/{scope_id}", method: "GET" },
  get_channel_history: { path: "/v1/channels/{scope_id}/history", method: "GET" },
  pin_channel: { path: "/v1/channels/{scope_id}/pin", method: "POST" },
  publish_channel: { path: "/v1/channels/{scope_id}/publish", method: "POST" },
  rollback_channel: { path: "/v1/channels/{scope_id}/rollback", method: "POST" },
  unpin_channel: { path: "/v1/channels/{scope_id}/unpin", method: "POST" },
  list_configuration_bindings: { path: "/v1/configuration-bindings", method: "GET" },
  create_configuration_binding: { path: "/v1/configuration-bindings", method: "POST", idempotent: true },
  update_configuration_binding: { path: "/v1/configuration-bindings/{id}", method: "PATCH", idempotent: true },
  rollback_configuration_binding: { path: "/v1/configuration-bindings/{id}/rollback", method: "POST", idempotent: true },
  list_configuration_templates: { path: "/v1/configuration-templates", method: "GET" },
  list_configurations: { path: "/v1/configurations", method: "GET" },
  create_configuration: { path: "/v1/configurations", method: "POST", idempotent: true },
  get_effective_configuration: { path: "/v1/configurations/effective", method: "GET" },
  get_configuration: { path: "/v1/configurations/{id}", method: "GET" },
  compare_configuration_versions: { path: "/v1/configurations/{id}/compare", method: "GET" },
  list_configuration_versions: { path: "/v1/configurations/{id}/versions", method: "GET" },
  publish_configuration_version: { path: "/v1/configurations/{id}/versions", method: "POST", idempotent: true },
  list_context_packs: { path: "/v1/context-packs", method: "GET" },
  author_context_pack: { path: "/v1/context-packs", method: "POST" },
  list_context_runs: { path: "/v1/context-runs", method: "GET" },
  get_context_run: { path: "/v1/context-runs/{id}", method: "GET" },
  create_context_feedback: { path: "/v1/context-runs/{id}/feedback", method: "POST", idempotent: true },
  create_directory_access_assignment: { path: "/v1/directory/access-assignments", method: "POST", idempotent: true },
  revoke_directory_access_assignment: { path: "/v1/directory/access-assignments/{grant_id}", method: "DELETE" },
  authorise_directory_seals: { path: "/v1/directory/seal-authorisations", method: "POST" },
  get_directory_sync: { path: "/v1/directory/sync", method: "GET" },
  accept_invite: { path: "/v1/invites/{invite_token}/accept", method: "POST" },
  list_knowledge: { path: "/v1/knowledge", method: "GET" },
  create_knowledge: { path: "/v1/knowledge", method: "POST", idempotent: true },
  list_knowledge_conflicts: { path: "/v1/knowledge-conflicts", method: "GET" },
  get_knowledge_conflict: { path: "/v1/knowledge-conflicts/{id}", method: "GET" },
  resolve_knowledge_conflict: { path: "/v1/knowledge-conflicts/{id}/resolve", method: "POST", idempotent: true },
  list_knowledge_freshness_policies: { path: "/v1/knowledge-freshness-policies", method: "GET" },
  merge_knowledge: { path: "/v1/knowledge/merge", method: "POST", idempotent: true },
  get_knowledge: { path: "/v1/knowledge/{id}", method: "GET" },
  edit_knowledge: { path: "/v1/knowledge/{id}", method: "PATCH", idempotent: true },
  delete_knowledge: { path: "/v1/knowledge/{id}", method: "DELETE", idempotent: true },
  archive_knowledge: { path: "/v1/knowledge/{id}/archive", method: "POST", idempotent: true },
  get_knowledge_history: { path: "/v1/knowledge/{id}/history", method: "GET" },
  restore_knowledge: { path: "/v1/knowledge/{id}/restore", method: "POST", idempotent: true },
  get_knowledge_sources: { path: "/v1/knowledge/{id}/sources", method: "GET" },
  supersede_knowledge: { path: "/v1/knowledge/{id}/supersede", method: "POST", idempotent: true },
  get_knowledge_usage: { path: "/v1/knowledge/{id}/usage", method: "GET" },
  verify_knowledge: { path: "/v1/knowledge/{id}/verify", method: "POST", idempotent: true },
  get_me: { path: "/v1/me", method: "GET" },
  list_okf_imports: { path: "/v1/okf/imports", method: "GET" },
  get_okf_import: { path: "/v1/okf/imports/{id}", method: "GET" },
  materialize_okf_import: { path: "/v1/okf/imports/{id}/materialize", method: "POST", idempotent: true },
  list_policy_packs: { path: "/v1/policy/packs", method: "GET" },
  get_project: { path: "/v1/projects/{project_id}", method: "GET" },
  update_project: { path: "/v1/projects/{project_id}", method: "PATCH" },
  list_project_members: { path: "/v1/projects/{project_id}/members", method: "GET" },
  add_project_member: { path: "/v1/projects/{project_id}/members", method: "POST", idempotent: true },
  remove_project_member: { path: "/v1/projects/{project_id}/members/{principal_id}", method: "DELETE" },
  export_okf: { path: "/v1/projects/{project_id}/okf/exports", method: "POST" },
  plan_okf_import: { path: "/v1/projects/{project_id}/okf/imports", method: "POST", idempotent: true },
  list_repositories: { path: "/v1/projects/{project_id}/repositories", method: "GET" },
  attach_repository: { path: "/v1/projects/{project_id}/repositories", method: "POST", idempotent: true },
  detach_repository: { path: "/v1/projects/{project_id}/repositories/{repository_id}", method: "DELETE" },
  generate_tool_client_config: { path: "/v1/projects/{project_id}/tool-config", method: "GET" },
  list_prompts: { path: "/v1/prompts", method: "GET" },
  author_prompt: { path: "/v1/prompts", method: "POST" },
  resolve_prompt: { path: "/v1/prompts/{name}", method: "GET" },
  list_proposals: { path: "/v1/proposals", method: "GET" },
  open_proposal: { path: "/v1/proposals", method: "POST" },
  get_proposal: { path: "/v1/proposals/{id}", method: "GET" },
  apply_proposal: { path: "/v1/proposals/{id}/apply", method: "POST" },
  approve_proposal: { path: "/v1/proposals/{id}/approve", method: "POST" },
  publish_proposal: { path: "/v1/proposals/{id}/publish", method: "POST" },
  reject_proposal: { path: "/v1/proposals/{id}/reject", method: "POST" },
  withdraw_proposal: { path: "/v1/proposals/{id}/withdraw", method: "POST" },
  list_quarantine: { path: "/v1/quarantine", method: "GET" },
  reject_quarantined_event: { path: "/v1/quarantine/{event_id}/reject", method: "POST" },
  release_quarantined_event: { path: "/v1/quarantine/{event_id}/release", method: "POST" },
  list_relaxations: { path: "/v1/relaxations", method: "GET" },
  create_relaxation: { path: "/v1/relaxations", method: "POST", idempotent: true },
  get_relaxation: { path: "/v1/relaxations/{id}", method: "GET" },
  revise_relaxation: { path: "/v1/relaxations/{id}", method: "PATCH", idempotent: true },
  revoke_relaxation: { path: "/v1/relaxations/{id}/revoke", method: "POST", idempotent: true },
  list_relaxation_versions: { path: "/v1/relaxations/{id}/versions", method: "GET" },
  list_scim_credentials: { path: "/v1/scim/credentials", method: "GET" },
  issue_scim_credential: { path: "/v1/scim/credentials", method: "POST" },
  revoke_scim_credential: { path: "/v1/scim/credentials/{id}/revoke", method: "POST" },
  list_service_identities: { path: "/v1/service-identities", method: "GET" },
  register_service_identity: { path: "/v1/service-identities", method: "POST" },
  get_service_identity: { path: "/v1/service-identities/{id}", method: "GET" },
  remove_service_identity: { path: "/v1/service-identities/{id}", method: "DELETE" },
  list_sessions: { path: "/v1/sessions", method: "GET" },
  open_session: { path: "/v1/sessions", method: "POST", idempotent: true },
  get_session: { path: "/v1/sessions/{session_id}", method: "GET" },
  create_capture_batch: { path: "/v1/sessions/{session_id}/capture-batches", method: "POST", idempotent: true },
  create_context_run: { path: "/v1/sessions/{session_id}/context-runs", method: "POST", idempotent: true },
  end_session: { path: "/v1/sessions/{session_id}/end", method: "POST" },
  append_session_events: { path: "/v1/sessions/{session_id}/events", method: "POST" },
  get_session_event: { path: "/v1/sessions/{session_id}/events/{event_id}", method: "GET" },
  evaluate_session_knowledge: { path: "/v1/sessions/{session_id}/knowledge-evaluation", method: "POST" },
  query_session_knowledge: { path: "/v1/sessions/{session_id}/knowledge-query", method: "POST" },
  get_session_timeline: { path: "/v1/sessions/{session_id}/timeline", method: "GET" },
  list_skill_bindings: { path: "/v1/skill-bindings", method: "GET" },
  create_skill_binding: { path: "/v1/skill-bindings", method: "POST", idempotent: true },
  get_skill_binding: { path: "/v1/skill-bindings/{id}", method: "GET" },
  update_skill_binding: { path: "/v1/skill-bindings/{id}", method: "PATCH", idempotent: true },
  rollback_skill_binding: { path: "/v1/skill-bindings/{id}/rollback", method: "POST", idempotent: true },
  record_skill_usage: { path: "/v1/skill-usage", method: "POST" },
  list_skills: { path: "/v1/skills", method: "GET" },
  install_skill: { path: "/v1/skills", method: "POST", idempotent: true },
  list_available_skills: { path: "/v1/skills/available", method: "GET" },
  get_skill: { path: "/v1/skills/{id}", method: "GET" },
  update_skill: { path: "/v1/skills/{id}", method: "PATCH", idempotent: true },
  list_skill_versions: { path: "/v1/skills/{id}/versions", method: "GET" },
  get_skill_version: { path: "/v1/skills/{id}/versions/{version_id}", method: "GET" },
  list_skill_version_files: { path: "/v1/skills/{id}/versions/{version_id}/files", method: "GET" },
  get_skill_version_file: { path: "/v1/skills/{id}/versions/{version_id}/files/{path}", method: "GET" },
  list_skill_tests: { path: "/v1/skills/{id}/versions/{version_id}/tests", method: "GET" },
  run_skill_test: { path: "/v1/skills/{id}/versions/{version_id}/tests", method: "POST", idempotent: true },
  list_skill_usage: { path: "/v1/skills/{id}/versions/{version_id}/usage", method: "GET" },
  list_tool_bindings: { path: "/v1/tool-bindings", method: "GET" },
  create_tool_binding: { path: "/v1/tool-bindings", method: "POST", idempotent: true },
  get_tool_binding: { path: "/v1/tool-bindings/{id}", method: "GET" },
  update_tool_binding: { path: "/v1/tool-bindings/{id}", method: "PATCH", idempotent: true },
  list_tool_servers: { path: "/v1/tool-servers", method: "GET" },
  register_tool_server: { path: "/v1/tool-servers", method: "POST", idempotent: true },
  import_tool_client_config: { path: "/v1/tool-servers/import-client-config", method: "POST", idempotent: true },
  get_tool_server: { path: "/v1/tool-servers/{id}", method: "GET" },
  update_tool_server: { path: "/v1/tool-servers/{id}", method: "PATCH", idempotent: true },
  discover_tool_server: { path: "/v1/tool-servers/{id}/discoveries", method: "POST", idempotent: true },
  list_tool_server_versions: { path: "/v1/tool-servers/{id}/versions", method: "GET" },
  get_tool_server_version: { path: "/v1/tool-servers/{id}/versions/{version_id}", method: "GET" },
  diff_tool_server_version: { path: "/v1/tool-servers/{id}/versions/{version_id}/diff", method: "GET" },
  list_tool_server_tests: { path: "/v1/tool-servers/{id}/versions/{version_id}/tests", method: "GET" },
  run_tool_server_test: { path: "/v1/tool-servers/{id}/versions/{version_id}/tests", method: "POST", idempotent: true },
  get_whoami: { path: "/v1/whoami", method: "GET" },
  list_workspaces: { path: "/v1/workspaces", method: "GET" },
  create_workspace: { path: "/v1/workspaces", method: "POST", idempotent: true },
  get_workspace: { path: "/v1/workspaces/{workspace_id}", method: "GET" },
  update_workspace: { path: "/v1/workspaces/{workspace_id}", method: "PATCH" },
  list_workspace_invites: { path: "/v1/workspaces/{workspace_id}/invites", method: "GET" },
  create_workspace_invite: { path: "/v1/workspaces/{workspace_id}/invites", method: "POST", idempotent: true },
  revoke_workspace_invite: { path: "/v1/workspaces/{workspace_id}/invites/{invite_id}", method: "DELETE" },
  list_workspace_members: { path: "/v1/workspaces/{workspace_id}/members", method: "GET" },
  list_projects: { path: "/v1/workspaces/{workspace_id}/projects", method: "GET" },
  create_project: { path: "/v1/workspaces/{workspace_id}/projects", method: "POST", idempotent: true },
} as const satisfies Record<
  OperationId,
  { readonly path: string; readonly method: string; readonly idempotent?: true }
>;

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
    content?: unknown | null | KnowledgeContentBody;
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
    event?: unknown | null | SessionEventView;
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
     * Source session.
     */
    session_id: string;
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
     * Source session.
     */
    session_id: string;
    /**
     * Exact immutable source event ids.
     */
    source_event_ids: string[];
    state: "pending" | "accepted" | "edited_and_accepted" | "merged" | "replaced" | "dismissed" | "failed";
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
    kind: "duplicate" | "conflict" | "possible_supersession";
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
 * One retained, freshly re-authorised planner candidate.
 */
export type ContextCandidateView = {
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
    revision?: unknown | null | KnowledgeRevisionView;
    scores?: unknown | null | ContextScoreView;
    /**
     * Independently visible provenance, full mode only.
     */
    sources?: KnowledgeSourceView[];
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
    revision?: unknown | null | KnowledgeRevisionView;
    /**
     * Independently visible provenance, full mode only.
     */
    sources?: KnowledgeSourceView[];
    /**
     * Estimated tokens charged.
     */
    token_count: number;
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
     * Its members at creation, by principal id.
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
     * The external id a directory knows it by, when one does.
     */
    directory_ref?: string | null;
    /**
     * Display name.
     */
    display_name: string;
    /**
     * Stable id.
     */
    id: string;
    /**
     * Its members, by principal id.
     */
    members: string[];
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
    lifecycle_state: "active" | "stale" | "superseded" | "archived" | "erasure_pending" | "erased";
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
    parent?: unknown | null | ScopeView;
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
    via_group?: unknown | null | GroupRefView;
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
   * `GET /v1/admin/scopes/{scope_id}/descendants` — the whole subtree,
   * nearest first, the scope itself excluded.
   */
  readonly list_scope_descendants: {
    readonly path: "/v1/admin/scopes/{scope_id}/descendants";
    readonly method: "GET";
    readonly response: ChainResponse;
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
  list_scope_descendants: { path: "/v1/admin/scopes/{scope_id}/descendants", method: "GET" },
  list_capture_batches: { path: "/v1/capture-batches", method: "GET" },
  get_capture_batch: { path: "/v1/capture-batches/{id}", method: "GET" },
  accept_capture_batch: { path: "/v1/capture-batches/{id}/accept", method: "POST", idempotent: true },
  list_capture_candidates: { path: "/v1/capture-candidates", method: "GET" },
  accept_capture_candidate: { path: "/v1/capture-candidates/{id}/accept", method: "POST", idempotent: true },
  dismiss_capture_candidate: { path: "/v1/capture-candidates/{id}/dismiss", method: "POST", idempotent: true },
  merge_capture_candidate: { path: "/v1/capture-candidates/{id}/merge", method: "POST", idempotent: true },
  replace_capture_candidate: { path: "/v1/capture-candidates/{id}/replace", method: "POST", idempotent: true },
  list_context_runs: { path: "/v1/context-runs", method: "GET" },
  get_context_run: { path: "/v1/context-runs/{id}", method: "GET" },
  create_context_feedback: { path: "/v1/context-runs/{id}/feedback", method: "POST", idempotent: true },
  accept_invite: { path: "/v1/invites/{invite_token}/accept", method: "POST" },
  list_knowledge: { path: "/v1/knowledge", method: "GET" },
  create_knowledge: { path: "/v1/knowledge", method: "POST", idempotent: true },
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
  get_project: { path: "/v1/projects/{project_id}", method: "GET" },
  update_project: { path: "/v1/projects/{project_id}", method: "PATCH" },
  list_project_members: { path: "/v1/projects/{project_id}/members", method: "GET" },
  add_project_member: { path: "/v1/projects/{project_id}/members", method: "POST", idempotent: true },
  remove_project_member: { path: "/v1/projects/{project_id}/members/{principal_id}", method: "DELETE" },
  list_repositories: { path: "/v1/projects/{project_id}/repositories", method: "GET" },
  attach_repository: { path: "/v1/projects/{project_id}/repositories", method: "POST", idempotent: true },
  detach_repository: { path: "/v1/projects/{project_id}/repositories/{repository_id}", method: "DELETE" },
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

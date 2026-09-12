// GENERATED FILE — DO NOT EDIT.
//
// Written by scripts/generate-api-types.mjs from docs/api/openapi.json, which the
// gateway derives from its own request and response types (CPR-4, ADR-0071
// decision 7). Editing this file is editing the wrong end of the chain: change
// the Rust, run `cargo test -p synveda-gateway --test openapi` with
// SYNVEDA_WRITE_OPENAPI=1 to refresh the document, then
// `node scripts/generate-sdk-contract.mjs`.
//
// `make sdk-check` fails when this file and the document disagree.
//
// Source document: Synveda 0.2.0

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
 * `POST /v1/sessions/{session_id}/events`.
 */
export type AppendEventsBody = {
    /**
     * The batch, at most 200 events, each `client_event_id` at most once.
     */
    events: NewEventBody[];
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

export type AuditEventsResponse = AuditFrame & {
    events: AuditEventView[];
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
     * act itself will pass through. Configuration and policy bindings change
     * the rows this reads, never the code that reads them.
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
 * One proposal, in full.
 */
export type ProposalDetail = ProposalSummary & {
    approvals: ProposalApprovalView[];
    members: ProposalMemberView[];
    timeline: ProposalTimelineEvent[];
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
 * Immutable file collection.
 */
export type SkillVersionFileListView = {
    /**
     * Immutable file descriptors in path order.
     */
    files: SkillVersionFileView[];
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

/**
 * The onboarding vocabulary. Closed, so a client's branch is exhaustive and
 * a new state is a compile error somewhere rather than a silently unhandled
 * string.
 */
export type OnboardingState = "blocked" | "needs_workspace" | "needs_project" | "ready";

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
 * What publishing this proposal would do to the target's published
 * channel, for one member (FLOW-6, ADR-0035 decision 5). Membership in
 * the target's tree is the predicate — the same sense of "this scope
 * holds it" ADR-0034 decision 3 used one scope over.
 */
export type ProposalMemberEffect = "add" | "update" | "apply" | "none";

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
 * One role line.
 */
export type ApprovalRoleView = {
    count: number;
    role: string;
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
   * `GET /v1/audit/events` — the search (ADR-0045 decision 3).
   */
  readonly list_audit_events: {
    readonly path: "/v1/audit/events";
    readonly method: "GET";
    readonly response: AuditEventsResponse;
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
   * `GET /v1/me`.
   */
  readonly get_me: {
    readonly path: "/v1/me";
    readonly method: "GET";
    readonly response: MeView;
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
   * `POST /v1/sessions/{session_id}/knowledge-query` — ordinary deep recall.
   */
  readonly query_session_knowledge: {
    readonly path: "/v1/sessions/{session_id}/knowledge-query";
    readonly method: "POST";
    readonly body: KnowledgeQueryBody;
    readonly response: ContextKnowledgeQueryView;
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
  list_audit_events: { path: "/v1/audit/events", method: "GET" },
  create_knowledge: { path: "/v1/knowledge", method: "POST", idempotent: true },
  get_me: { path: "/v1/me", method: "GET" },
  get_proposal: { path: "/v1/proposals/{id}", method: "GET" },
  open_session: { path: "/v1/sessions", method: "POST", idempotent: true },
  get_session: { path: "/v1/sessions/{session_id}", method: "GET" },
  create_capture_batch: { path: "/v1/sessions/{session_id}/capture-batches", method: "POST", idempotent: true },
  create_context_run: { path: "/v1/sessions/{session_id}/context-runs", method: "POST", idempotent: true },
  end_session: { path: "/v1/sessions/{session_id}/end", method: "POST" },
  append_session_events: { path: "/v1/sessions/{session_id}/events", method: "POST" },
  query_session_knowledge: { path: "/v1/sessions/{session_id}/knowledge-query", method: "POST" },
  list_available_skills: { path: "/v1/skills/available", method: "GET" },
  get_skill_version: { path: "/v1/skills/{id}/versions/{version_id}", method: "GET" },
  list_skill_version_files: { path: "/v1/skills/{id}/versions/{version_id}/files", method: "GET" },
  get_skill_version_file: { path: "/v1/skills/{id}/versions/{version_id}/files/{path}", method: "GET" },
} as const satisfies Record<
  OperationId,
  { readonly path: string; readonly method: string; readonly idempotent?: true }
>;

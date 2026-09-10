import {
  BrowserContractError,
  allowedRequest,
} from "./console-login-contract.mjs";

const OIDC_VALUE = /^[A-Za-z0-9_-]{43}$/;
const CLI_LOGIN_PARAMETERS = Object.freeze([
  "cli_redirect_uri",
  "cli_state",
  "issuer",
]);
const HANDOFF_PARAMETERS = Object.freeze(["code", "state"]);
const REQUIRED_RESOURCES = Object.freeze([
  "workspace",
  "project",
  "bob_invite",
  "bob_member",
  "bob_principal",
  "first_session",
  "first_events",
  "first_capture",
  "first_candidates",
  "webhook_knowledge",
  "private_knowledge",
  "reuse_session",
  "reuse_context",
  "private_isolation",
  "current_session",
  "current_context",
  "release_skill",
]);
const STATUS_RESOURCES = Object.freeze([
  "workspace",
  "project",
  "first_session",
  "first_capture",
  "reuse_session",
  "current_session",
  "webhook_knowledge",
  "reuse_context",
  "current_context",
]);
const RETRY_FIXTURE = "cpr45-retry-review-v1";
const RETRY_RULE =
  "Retried ingestion requests must reuse the original Idempotency-Key. While the original request is running, the retry returns 409; after completion, it replays the stored response without starting a second ingestion.";
const RETRY_BASE_RESOURCES = Object.freeze([
  "workspace",
  "project",
  "repository",
  "grant_reviewer",
  "grant_administrator",
  "grant_viewer",
  "baseline_knowledge",
  "baseline_provenance",
  "curator_rule",
  "skill_install",
  "source_session",
  "source_event",
]);

function refuse(stage) {
  throw new BrowserContractError(stage);
}

function one(search, name, stage) {
  const values = search.getAll(name);
  if (values.length !== 1 || values[0] === "") refuse(stage);
  return values[0];
}

function exactNames(search, expected, stage) {
  const names = [...search.keys()].sort();
  if (
    names.length !== expected.length ||
    names.some((name, index) => name !== expected[index])
  ) refuse(stage);
}

export function validateCliLoginUrl(raw, settings) {
  if (typeof raw !== "string" || raw.length > 4096) refuse("cli-login-url");
  let url;
  try {
    url = new URL(raw);
  } catch {
    refuse("cli-login-url");
  }
  if (
    url.origin !== settings.appOrigin ||
    url.pathname !== "/auth/login" ||
    url.hash !== "" ||
    url.username !== "" ||
    url.password !== ""
  ) refuse("cli-login-url");
  exactNames(url.searchParams, CLI_LOGIN_PARAMETERS, "cli-login-url");
  const state = one(url.searchParams, "cli_state", "cli-login-url");
  if (!OIDC_VALUE.test(state)) refuse("cli-login-url");
  if (one(url.searchParams, "issuer", "cli-login-url") !== settings.issuer) {
    refuse("cli-login-url");
  }

  let redirect;
  try {
    redirect = new URL(
      one(url.searchParams, "cli_redirect_uri", "cli-login-url"),
    );
  } catch {
    refuse("cli-login-url");
  }
  if (
    redirect.protocol !== "http:" ||
    redirect.hostname !== "127.0.0.1" ||
    redirect.port === "" ||
    redirect.pathname !== "/callback" ||
    redirect.search !== "" ||
    redirect.hash !== "" ||
    redirect.username !== "" ||
    redirect.password !== ""
  ) refuse("cli-login-url");
  const port = Number(redirect.port);
  if (!Number.isSafeInteger(port) || port < 1 || port > 65535) {
    refuse("cli-login-url");
  }
  return Object.freeze({
    startUrl: url.href,
    state,
    redirect: redirect.href,
    redirectOrigin: redirect.origin,
  });
}

export function validateCliHandoffUrl(raw, login) {
  if (typeof raw !== "string" || raw.length > 8192) refuse("cli-handoff");
  let url;
  try {
    url = new URL(raw);
  } catch {
    refuse("cli-handoff");
  }
  exactNames(url.searchParams, HANDOFF_PARAMETERS, "cli-handoff");
  if (
    url.origin !== login.redirectOrigin ||
    `${url.origin}${url.pathname}` !== login.redirect ||
    url.hash !== "" ||
    url.username !== "" ||
    url.password !== "" ||
    one(url.searchParams, "state", "cli-handoff") !== login.state
  ) refuse("cli-handoff");
  const code = one(url.searchParams, "code", "cli-handoff");
  if (code.length > 4096) refuse("cli-handoff");
  return true;
}

export function allowedCliRequest(raw, settings, login) {
  if (allowedRequest(raw, settings)) return true;
  if (typeof raw !== "string" || raw.length > 8192) return false;
  try {
    const url = new URL(raw);
    return (
      url.origin === login.redirectOrigin &&
      `${url.origin}${url.pathname}` === login.redirect &&
      url.hash === "" &&
      url.username === "" &&
      url.password === ""
    );
  } catch {
    return false;
  }
}

function object(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

const UUID_V7 =
  /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

function uuid(value) {
  return typeof value === "string" && UUID_V7.test(value);
}

function text(value) {
  return typeof value === "string" && value.length > 0;
}

function activeResource(value, slug) {
  return (
    object(value) &&
    uuid(value.id) &&
    uuid(value.scope_id) &&
    value.slug === slug &&
    value.status === "active"
  );
}

function sessionOnProject(value, receipt, principal) {
  const { workspace, project } = receipt.resources;
  return (
    object(value) &&
    uuid(value.id) &&
    value.workspace_id === workspace.id &&
    value.project_id === project.id &&
    value.scope_id === project.scope_id &&
    value.principal_id === principal &&
    value.client_name === "synveda-demo"
  );
}

function appliedKnowledge(value) {
  return (
    object(value) &&
    uuid(value.id) &&
    uuid(value.revision_id) &&
    uuid(value.candidate_id) &&
    value.outcome === "applied"
  );
}

function pendingKnowledge(value) {
  return (
    object(value) &&
    uuid(value.id) &&
    uuid(value.candidate_id) &&
    uuid(value.change_id) &&
    value.revision_id === undefined &&
    value.outcome === "pending_review"
  );
}

function completedContext(value, receipt, session) {
  const { workspace, project } = receipt.resources;
  return (
    object(value) &&
    uuid(value.id) &&
    value.session_id === session.id &&
    value.workspace_id === workspace.id &&
    value.project_id === project.id &&
    value.scope_id === project.scope_id &&
    value.completion_status === "completed" &&
    Number.isInteger(value.selection_count) &&
    value.selection_count > 0
  );
}

export function validateDemoReceipt(value) {
  if (
    !object(value) ||
    value.receipt_version !== 1 ||
    value.profile !== "team" ||
    value.state !== "active" ||
    !object(value.resources) ||
    REQUIRED_RESOURCES.some((name) => !object(value.resources[name]))
  ) refuse("product-demo");

  const resources = value.resources;
  const workspace = resources.workspace;
  const project = resources.project;
  const bob = resources.bob_principal;
  const invite = resources.bob_invite.invite;
  const accepted = resources.bob_member;
  const grant = accepted?.grant;
  if (
    !text(value.actor_subject) ||
    !activeResource(workspace, "pulseboard-demo") ||
    !activeResource(project, "delivery-api") ||
    project.workspace_id !== workspace.id ||
    !object(bob) ||
    !text(bob.subject) ||
    bob.subject === value.actor_subject ||
    bob.credential_profile !== "bob" ||
    !object(invite) ||
    !uuid(invite.id) ||
    invite.scope_id !== workspace.scope_id ||
    invite.role !== "member" ||
    invite.status !== "pending" ||
    !object(grant) ||
    !uuid(grant.id) ||
    grant.scope_id !== workspace.scope_id ||
    grant.subject_kind !== "principal" ||
    grant.principal_id !== bob.subject ||
    grant.role !== "member" ||
    grant.source !== "invite" ||
    grant.invite_id !== invite.id ||
    grant.directory_managed !== false ||
    accepted.scope_id !== workspace.scope_id
  ) refuse("product-demo");

  const firstSession = resources.first_session;
  const reuseSession = resources.reuse_session;
  const currentSession = resources.current_session;
  if (
    !sessionOnProject(firstSession, value, value.actor_subject) ||
    !sessionOnProject(reuseSession, value, bob.subject) ||
    !sessionOnProject(currentSession, value, bob.subject)
  ) refuse("product-demo");

  const eventEntries = resources.first_events.events;
  if (
    !Array.isArray(eventEntries) ||
    eventEntries.length < 4 ||
    eventEntries.some(
      (entry) =>
        !object(entry) ||
        !["appended", "duplicate"].includes(entry.outcome) ||
        !object(entry.event) ||
        !uuid(entry.event.id),
    )
  ) refuse("product-demo");
  const eventIds = new Set(eventEntries.map((entry) => entry.event.id));

  const batch = resources.first_capture;
  const candidates = resources.first_candidates.candidates;
  if (
    !object(batch) ||
    !uuid(batch.id) ||
    batch.source_kind !== "session" ||
    batch.session_id !== firstSession.id ||
    batch.project_id !== project.id ||
    batch.scope_id !== project.scope_id ||
    batch.state !== "completed" ||
    !Number.isInteger(batch.event_count) ||
    batch.event_count < 4 ||
    !Number.isInteger(batch.candidate_count) ||
    batch.candidate_count < 1 ||
    !Array.isArray(candidates) ||
    candidates.length !== batch.candidate_count ||
    candidates.some(
      (candidate) =>
        !object(candidate) ||
        !uuid(candidate.id) ||
        candidate.batch_id !== batch.id ||
        candidate.session_id !== firstSession.id ||
        !Array.isArray(candidate.source_event_ids) ||
        candidate.source_event_ids.length < 1 ||
        candidate.source_event_ids.some((id) => !eventIds.has(id)),
    )
  ) refuse("product-demo");

  const webhook = resources.webhook_knowledge;
  const privateKnowledge = resources.private_knowledge;
  const byId = new Map(candidates.map((candidate) => [candidate.id, candidate]));
  const webhookCandidate = byId.get(webhook.candidate_id);
  const privateCandidate = byId.get(privateKnowledge.candidate_id);
  const privateApplied = appliedKnowledge(privateKnowledge);
  const privatePending = pendingKnowledge(privateKnowledge);
  if (
    !appliedKnowledge(webhook) ||
    (!privateApplied && !privatePending) ||
    (privatePending &&
      (!Array.isArray(value.notices) ||
        !value.notices.includes(
          "private quick-test preference remains pending in Advanced Reviews; the demo does not claim it as active Knowledge",
        ))) ||
    !object(webhookCandidate) ||
    webhookCandidate.proposed_scope_id !== project.scope_id ||
    webhookCandidate.proposed_project_id !== project.id ||
    webhookCandidate.proposed_owner_principal_id !== undefined ||
    !object(privateCandidate) ||
    privateCandidate.knowledge_type !== "preference" ||
    privateCandidate.proposed_scope_id === project.scope_id ||
    privateCandidate.proposed_project_id !== undefined ||
    privateCandidate.proposed_owner_principal_id !== value.actor_subject
  ) refuse("product-demo");

  const reuse = resources.reuse_context;
  const current = resources.current_context;
  if (
    !completedContext(reuse, value, reuseSession) ||
    !text(reuse.rendered) ||
    !reuse.rendered.includes("provider event ID") ||
    reuse.rendered.includes("test-fast") ||
    !completedContext(current, value, currentSession) ||
    (text(current.rendered) && current.rendered.includes("test-fast"))
  ) refuse("product-demo");

  const isolation = resources.private_isolation;
  if (
    isolation.session_id !== reuseSession.id ||
    isolation.private_knowledge_id !== privateKnowledge.id ||
    !Number.isInteger(isolation.inspected_count) ||
    isolation.inspected_count < 0 ||
    isolation.private_knowledge_absent !== true ||
    isolation.evidence_kind !==
      (privatePending ? "pending_review_not_published" : "owner_scope")
  ) refuse("product-demo");

  const skill = resources.release_skill;
  const validation = resources.release_skill_validation;
  const skillApplied = object(skill) && skill.outcome === "applied";
  const skillPending =
    object(skill) && skill.outcome === "pending_review" && uuid(skill.change_id);
  if (
    (!skillApplied && !skillPending) ||
    !uuid(skill.skill_id) ||
    !uuid(skill.version_id)
  ) refuse("product-demo");
  if (skillApplied) {
    if (
      !object(validation) ||
      !uuid(validation.id) ||
      validation.kind !== "skill_validation" ||
      validation.operation_version !== 1 ||
      validation.state !== "succeeded" ||
      validation.skill_id !== skill.skill_id ||
      validation.skill_version_id !== skill.version_id ||
      validation.progress_percent !== 100 ||
      !Number.isInteger(validation.attempts) ||
      validation.attempts < 1 ||
      !uuid(validation.test_run_id) ||
      validation.error_code !== undefined
    ) refuse("product-demo");
  } else if (
    validation !== undefined ||
    !Array.isArray(value.notices) ||
    !value.notices.includes(
      "Release Skill installation is in Advanced > Reviews; no unreviewed version was advertised or pinned",
    ) ||
    !value.notices.includes(
      "Release Skill validation remains pending until its governed version is applied",
    )
  ) refuse("product-demo");
  return true;
}

export function validateDemoStatus(value) {
  if (!object(value) || !object(value.receipt) || !object(value.live)) {
    refuse("product-status");
  }
  validateDemoReceipt(value.receipt);
  if (
    STATUS_RESOURCES.some(
      (name) =>
        !object(value.live[name]) || value.live[name].status !== "visible",
    )
  ) refuse("product-status");

  const resources = value.receipt.resources;
  const privatePending = pendingKnowledge(resources.private_knowledge);
  const skillPending = resources.release_skill.outcome === "pending_review";
  if (
    !object(value.live.private_knowledge) ||
    value.live.private_knowledge.status !==
      (privatePending ? "unavailable" : "visible")
  ) refuse("product-status");
  if (
    (skillPending && value.live.release_skill_validation !== undefined) ||
    (!skillPending &&
      (!object(value.live.release_skill_validation) ||
        value.live.release_skill_validation.status !== "visible"))
  ) refuse("product-status");
  const visible = (name) => value.live[name].value;
  const workspace = visible("workspace");
  const project = visible("project");
  const firstSession = visible("first_session");
  const firstCapture = visible("first_capture");
  const reuseSession = visible("reuse_session");
  const currentSession = visible("current_session");
  if (
    workspace.id !== resources.workspace.id ||
    workspace.status !== "active" ||
    project.id !== resources.project.id ||
    project.workspace_id !== workspace.id ||
    project.status !== "active" ||
    firstSession.id !== resources.first_session.id ||
    firstSession.status !== "ended" ||
    reuseSession.id !== resources.reuse_session.id ||
    reuseSession.status !== "ended" ||
    currentSession.id !== resources.current_session.id ||
    currentSession.status !== "ended" ||
    firstCapture.id !== resources.first_capture.id ||
    firstCapture.state !== "completed" ||
    firstCapture.candidate_count !== resources.first_capture.candidate_count
  ) refuse("product-status");
  if (!skillPending) {
    const skillValidation = visible("release_skill_validation");
    if (
      skillValidation.id !== resources.release_skill_validation.id ||
      skillValidation.state !== "succeeded" ||
      skillValidation.test_run_id !== resources.release_skill_validation.test_run_id
    ) refuse("product-status");
  }

  const assertLiveKnowledge = (name, owner) => {
    const handle = resources[name];
    const item = visible(name);
    if (
      !object(item) ||
      item.id !== handle.id ||
      item.lifecycle_state !== "active" ||
      !object(item.current_revision) ||
      item.current_revision.id !== handle.revision_id ||
      item.current_revision.knowledge_item_id !== handle.id ||
      (owner === undefined
        ? item.project_id !== project.id ||
          item.owner_principal_id !== undefined
        : item.project_id !== undefined || item.owner_principal_id !== owner)
    ) refuse("product-status");
  };
  assertLiveKnowledge("webhook_knowledge", undefined);
  if (!privatePending) {
    assertLiveKnowledge("private_knowledge", value.receipt.actor_subject);
  }

  const reuse = visible("reuse_context");
  const current = visible("current_context");
  if (
    !object(reuse) ||
    !completedContext(reuse.run, value.receipt, resources.reuse_session) ||
    reuse.run.trace_retention_mode !== "redacted" ||
    !Array.isArray(reuse.selections) ||
    reuse.selections.length !== reuse.run.selection_count ||
    !reuse.selections.some(
      (selection) =>
        selection.channel === "current_knowledge" &&
        selection.knowledge_item_id === resources.webhook_knowledge.id &&
        selection.knowledge_revision_id ===
          resources.webhook_knowledge.revision_id,
    ) ||
    reuse.selections.some(
      (selection) =>
        selection.knowledge_item_id === resources.private_knowledge.id,
    ) ||
    !object(current) ||
    !completedContext(current.run, value.receipt, resources.current_session) ||
    current.run.trace_retention_mode !== "redacted"
  ) refuse("product-status");
  return true;
}

function retryIdentity(value, label, profile) {
  return (
    object(value) &&
    value.label === label &&
    text(value.subject) &&
    value.credential_profile === profile
  );
}

function retryHandle(value, expectedOutcome) {
  return (
    object(value) &&
    uuid(value.change_id) &&
    uuid(value.skill_id) &&
    uuid(value.version_id) &&
    value.outcome === expectedOutcome
  );
}

function retryBase(value, expectedState) {
  if (
    !object(value) ||
    value.receipt_version !== 1 ||
    value.fixture !== RETRY_FIXTURE ||
    value.gateway_url !== "http://app.synveda.test:8080" ||
    value.state !== expectedState ||
    !retryIdentity(value.author, "Avery Author", "author") ||
    !retryIdentity(value.reviewer, "Riley Reviewer", "reviewer") ||
    !retryIdentity(value.viewer, "Vera Restricted Viewer", "viewer") ||
    new Set([
      value.author.subject,
      value.reviewer.subject,
      value.viewer.subject,
    ]).size !== 3 ||
    !object(value.resources) ||
    RETRY_BASE_RESOURCES.some((name) => !object(value.resources[name]))
  ) refuse("retry-review");

  const resources = value.resources;
  const workspace = resources.workspace;
  const project = resources.project;
  const repository = resources.repository;
  if (
    !activeResource(workspace, "northstar-delivery-demo") ||
    !activeResource(project, "ingestion-api") ||
    project.workspace_id !== workspace.id ||
    !uuid(repository.id) ||
    repository.project_id !== project.id ||
    repository.canonical_uri !==
      "https://github.com/northstar-demo/ingestion-api"
  ) refuse("retry-review");

  for (const [name, role, subject] of [
    ["grant_reviewer", "reviewer", value.reviewer.subject],
    ["grant_administrator", "administrator", value.reviewer.subject],
    ["grant_viewer", "viewer", value.viewer.subject],
  ]) {
    const grant = resources[name];
    if (
      !uuid(grant.grant_id) ||
      grant.scope_id !== project.scope_id ||
      grant.principal_id !== subject ||
      grant.role !== role ||
      grant.inherited !== false ||
      grant.directory_managed !== false
    ) refuse("retry-review");
  }

  const baseline = resources.baseline_knowledge;
  const baselineSources = resources.baseline_provenance.sources;
  if (
    baseline.outcome !== "applied" ||
    !uuid(baseline.change_id) ||
    !uuid(baseline.knowledge_item_id) ||
    !uuid(baseline.revision_id) ||
    !Array.isArray(baselineSources) ||
    !baselineSources.some(
      (source) =>
        object(source) &&
        source.source_type === "repository" &&
        source.source_revision === "northstar-demo-baseline-v1",
    ) ||
    resources.curator_rule.effective_at !== project.scope_id ||
    resources.curator_rule.source !==
      `knowledge/* @${value.reviewer.subject}\n` ||
    !retryHandle(
      resources.skill_install,
      ["binding_pending", "verified"].includes(expectedState)
        ? "applied"
        : "pending_review",
    )
  ) refuse("retry-review");

  const session = resources.source_session;
  const event = resources.source_event;
  if (
    !uuid(session.id) ||
    session.workspace_id !== workspace.id ||
    session.project_id !== project.id ||
    session.scope_id !== project.scope_id ||
    session.principal_id !== value.author.subject ||
    session.client_name !== "synveda-demo" ||
    session.external_session_id !== RETRY_FIXTURE ||
    !uuid(event.id) ||
    event.session_id !== session.id ||
    event.payload?.text !== RETRY_RULE ||
    event.payload?.synthetic !== true ||
    event.payload?.replay !== true
  ) refuse("retry-review");
  return resources;
}

export function validateRetryReviewReceipt(value, expectedState = "seeded") {
  const resources = retryBase(value, expectedState);
  if (expectedState === "seeded") return true;
  const batch = resources.capture_batch;
  const learning = resources.learning;
  if (
    !object(batch) ||
    !uuid(batch.id) ||
    batch.session_id !== resources.source_session.id ||
    batch.project_id !== resources.project.id ||
    batch.scope_id !== resources.project.scope_id ||
    batch.state !== "completed" ||
    !object(resources.capture_candidates) ||
    !Array.isArray(resources.capture_candidates.candidates) ||
    !object(learning) ||
    !uuid(learning.id) ||
    !uuid(learning.candidate_id) ||
    !uuid(learning.change_id) ||
    learning.outcome !== "pending_review" ||
    resources.source_session_closed?.id !== resources.source_session.id ||
    resources.source_session_closed?.status !== "ended"
  ) refuse("retry-review");
  if (expectedState === "learning_pending") return true;
  const binding = resources.skill_binding;
  const bindingOutcome =
    expectedState === "verified"
      ? ["pending_review", "applied"].includes(binding?.outcome)
      : binding?.outcome === "pending_review";
  if (
    !object(binding) ||
    !uuid(binding.change_id) ||
    !uuid(binding.binding_id) ||
    binding.skill_id !== resources.skill_install.skill_id ||
    binding.version_id !== resources.skill_install.version_id ||
    !bindingOutcome
  ) refuse("retry-review");
  if (expectedState === "binding_pending") return true;
  if (
    expectedState !== "verified" ||
    !object(resources.verification) ||
    resources.verification.knowledge_item_id !== learning.id ||
    resources.verification.source_event_id !== resources.source_event.id ||
    resources.verification.skill_id !== resources.skill_install.skill_id ||
    resources.verification.skill_version_id !== resources.skill_install.version_id ||
    resources.verification.skill_binding_id !== binding.binding_id ||
    !uuid(resources.verification.knowledge_revision_id) ||
    !uuid(resources.verification.context_run_id) ||
    !Number.isSafeInteger(resources.verification.audit_head_seq) ||
    resources.verification.audit_head_seq < 1
  ) refuse("retry-review");
  return true;
}

export function validateRetryReviewRerun(first, second) {
  validateRetryReviewReceipt(first, "seeded");
  validateRetryReviewReceipt(second, "seeded");
  for (const name of RETRY_BASE_RESOURCES) {
    if (JSON.stringify(first.resources[name]) !== JSON.stringify(second.resources[name])) {
      refuse("retry-review-rerun");
    }
  }
  return true;
}

export function validateRetryReviewInspection(value, receipt) {
  validateRetryReviewReceipt(receipt, "seeded");
  if (
    !object(value) ||
    value.synthetic !== true ||
    value.session?.id !== receipt.resources.source_session.id ||
    !object(value.timeline) ||
    value.source_event?.id !== receipt.resources.source_event.id ||
    value.source_event?.payload?.text !== RETRY_RULE
  ) refuse("retry-review-inspection");
  return true;
}

export function validateRetryReviewProposal(value, id) {
  if (
    !object(value) ||
    value.id !== id ||
    value.state !== "open" ||
    !Array.isArray(value.artifact_references) ||
    value.artifact_references.length < 1
  ) refuse("retry-review-proposal");
  return true;
}

export function validateRetryReviewVerification(value, receipt) {
  validateRetryReviewReceipt(receipt, "binding_pending");
  const resources = receipt.resources;
  const revision = value?.knowledge?.current_revision;
  const sources = value?.provenance?.sources;
  const selections = value?.context?.selections;
  if (
    !object(value) ||
    value.synthetic !== true ||
    value.knowledge?.id !== resources.learning.id ||
    !uuid(revision?.id) ||
    revision?.body_markdown !== RETRY_RULE ||
    !Array.isArray(sources) ||
    !sources.some(
      (source) =>
        source?.source_type === "session_event" &&
        source?.session_event_id === resources.source_event.id,
    ) ||
    value.skill_version?.id !== resources.skill_install.version_id ||
    value.skill_binding?.id !== resources.skill_binding.binding_id ||
    value.skill_binding?.pinned_version_id !== resources.skill_install.version_id ||
    value.skill_binding?.enabled !== true ||
    !Array.isArray(value.available_skills?.skills) ||
    !value.available_skills.skills.some(
      (entry) =>
        entry?.binding?.id === resources.skill_binding.binding_id &&
        entry?.version?.id === resources.skill_install.version_id,
    ) ||
    !Array.isArray(selections) ||
    !selections.some(
      (selection) =>
        selection?.knowledge_item_id === resources.learning.id &&
        selection?.knowledge_revision_id === revision.id,
    ) ||
    value.audit?.chain?.valid !== true ||
    !Number.isSafeInteger(value.audit?.chain?.head_seq) ||
    ["knowledge", "skill_binding", "session", "context"].some(
      (name) => !Array.isArray(value.audit?.[name]?.events) || value.audit[name].events.length < 1,
    )
  ) refuse("retry-review-verification");
  return true;
}

export function validateRetryReviewStatus(value) {
  if (!object(value) || !object(value.receipt) || !object(value.live)) {
    refuse("retry-review-status");
  }
  validateRetryReviewReceipt(value.receipt, "verified");
  const required = [
    "workspace",
    "project",
    "source_session",
    "capture_batch",
    "context_session",
    "context_run",
    "baseline_knowledge",
    "learning",
    "learning_provenance",
    "skill_install",
    "skill_binding",
    "audit_chain",
  ];
  const receipt = value.receipt;
  const resources = receipt.resources;
  const verification = resources.verification;
  const live = (name) => value.live[name]?.value;
  if (
    required.some(
      (name) =>
        !object(value.live[name]) ||
        value.live[name].status !== "visible" ||
        !object(value.live[name].value),
    ) ||
    live("workspace").id !== resources.workspace.id ||
    live("project").id !== resources.project.id ||
    live("source_session").id !== resources.source_session.id ||
    live("source_session").status !== "ended" ||
    live("capture_batch").id !== resources.capture_batch.id ||
    live("capture_batch").state !== "completed" ||
    live("baseline_knowledge").id !==
      resources.baseline_knowledge.knowledge_item_id ||
    live("baseline_knowledge").current_revision?.id !==
      resources.baseline_knowledge.revision_id ||
    live("learning").id !== resources.learning.id ||
    live("learning").current_revision?.id !==
      verification.knowledge_revision_id ||
    !Array.isArray(live("learning_provenance").sources) ||
    !live("learning_provenance").sources.some(
      (source) =>
        source?.source_type === "session_event" &&
        source?.session_event_id === resources.source_event.id,
    ) ||
    live("skill_install").id !== resources.skill_install.skill_id ||
    live("skill_install").current_version_id !==
      resources.skill_install.version_id ||
    live("skill_binding").id !== resources.skill_binding.binding_id ||
    live("skill_binding").pinned_version_id !==
      resources.skill_install.version_id ||
    live("skill_binding").enabled !== true ||
    live("context_run").run?.id !== verification.context_run_id ||
    !Array.isArray(live("context_run").selections) ||
    !live("context_run").selections.some(
      (selection) =>
        selection?.knowledge_item_id === resources.learning.id &&
        selection?.knowledge_revision_id === verification.knowledge_revision_id,
    ) ||
    live("audit_chain").valid !== true ||
    !Number.isSafeInteger(live("audit_chain").head_seq) ||
    live("audit_chain").head_seq < verification.audit_head_seq
  ) refuse("retry-review-status");
  return true;
}

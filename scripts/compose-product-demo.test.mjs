import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { PassThrough } from "node:stream";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  BrowserContractError,
  validateSettings,
} from "../deploy/compose/browser/console-login-contract.mjs";
import {
  allowedCliRequest,
  validateCliHandoffUrl,
  validateCliLoginUrl,
  validateDemoReceipt,
  validateDemoStatus,
  validateRetryReviewInspection,
  validateRetryReviewProposal,
  validateRetryReviewReceipt,
  validateRetryReviewRerun,
  validateRetryReviewStatus,
  validateRetryReviewVerification,
} from "../deploy/compose/browser/product-demo-contract.mjs";
import { runConsoleProductCheckpoint } from "../deploy/compose/browser/console-product-runner.mjs";
import { runProductAcceptance } from "../deploy/compose/browser/product-demo-runner.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SETTINGS = validateSettings(
  "http://app.synveda.test:8080",
  "http://auth.synveda.test:8080/realms/synveda",
);
const STATE = "s".repeat(43);
const REDIRECT = "http://127.0.0.1:43127/callback";
const RESOURCE_NAMES = [
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
];
const STATUS_NAMES = [
  "workspace",
  "project",
  "first_session",
  "first_capture",
  "reuse_session",
  "current_session",
  "webhook_knowledge",
  "private_knowledge",
  "reuse_context",
  "current_context",
];

const uuid = (value) =>
  `019b53c0-7c00-7${value.toString(16).padStart(3, "0")}-8000-${value
    .toString(16)
    .padStart(12, "0")}`;

function startUrl(overrides = {}, extra = []) {
  const value = new URL(`${SETTINGS.appOrigin}/auth/login`);
  for (const [name, entry] of Object.entries({
    cli_redirect_uri: REDIRECT,
    cli_state: STATE,
    issuer: SETTINGS.issuer,
    ...overrides,
  })) {
    if (entry !== undefined) value.searchParams.append(name, entry);
  }
  for (const [name, entry] of extra) value.searchParams.append(name, entry);
  return value.href;
}

function refuse(operation, stage) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof BrowserContractError);
    assert.equal(error.stage, stage);
    return true;
  });
}

function receipt() {
  const ids = {
    workspace: uuid(1),
    workspaceScope: uuid(2),
    project: uuid(3),
    projectScope: uuid(4),
    invite: uuid(5),
    grant: uuid(6),
    firstSession: uuid(7),
    firstCapture: uuid(8),
    firstEvent: uuid(9),
    privateEvent: uuid(10),
    webhookCandidate: uuid(11),
    privateCandidate: uuid(12),
    webhookKnowledge: uuid(13),
    webhookRevision: uuid(14),
    privateKnowledge: uuid(15),
    privateRevision: uuid(16),
    reuseSession: uuid(17),
    reuseContext: uuid(18),
    currentSession: uuid(19),
    currentContext: uuid(20),
    releaseSkill: uuid(24),
    releaseSkillVersion: uuid(25),
    releaseSkillValidation: uuid(26),
    releaseSkillTestRun: uuid(27),
  };
  const workspace = {
    id: ids.workspace,
    scope_id: ids.workspaceScope,
    slug: "pulseboard-demo",
    status: "active",
  };
  const project = {
    id: ids.project,
    workspace_id: ids.workspace,
    scope_id: ids.projectScope,
    slug: "delivery-api",
    status: "active",
  };
  const session = (id, principal) => ({
    id,
    workspace_id: ids.workspace,
    project_id: ids.project,
    scope_id: ids.projectScope,
    principal_id: principal,
    client_name: "synveda-demo",
    status: "active",
  });
  const context = (id, sessionId, rendered) => ({
    id,
    session_id: sessionId,
    workspace_id: ids.workspace,
    project_id: ids.project,
    scope_id: ids.projectScope,
    completion_status: "completed",
    selection_count: 1,
    rendered,
  });
  return {
    receipt_version: 1,
    profile: "team",
    state: "active",
    actor_subject: "alice-subject",
    resources: {
      workspace,
      project,
      bob_invite: {
        invite: {
          id: ids.invite,
          scope_id: ids.workspaceScope,
          role: "member",
          status: "pending",
        },
      },
      bob_member: {
        scope_id: ids.workspaceScope,
        grant: {
          id: ids.grant,
          scope_id: ids.workspaceScope,
          subject_kind: "principal",
          principal_id: "bob-subject",
          role: "member",
          source: "invite",
          invite_id: ids.invite,
          directory_managed: false,
        },
      },
      bob_principal: { subject: "bob-subject", credential_profile: "bob" },
      first_session: session(ids.firstSession, "alice-subject"),
      first_events: {
        events: [ids.firstEvent, ids.privateEvent, uuid(21), uuid(22)].map(
          (id) => ({ outcome: "appended", event: { id } }),
        ),
      },
      first_capture: {
        id: ids.firstCapture,
        source_kind: "session",
        session_id: ids.firstSession,
        project_id: ids.project,
        scope_id: ids.projectScope,
        state: "completed",
        event_count: 4,
        candidate_count: 2,
      },
      first_candidates: {
        candidates: [
          {
            id: ids.webhookCandidate,
            batch_id: ids.firstCapture,
            session_id: ids.firstSession,
            proposed_scope_id: ids.projectScope,
            proposed_project_id: ids.project,
            source_event_ids: [ids.firstEvent],
          },
          {
            id: ids.privateCandidate,
            batch_id: ids.firstCapture,
            session_id: ids.firstSession,
            proposed_scope_id: uuid(23),
            proposed_owner_principal_id: "alice-subject",
            knowledge_type: "preference",
            source_event_ids: [ids.privateEvent],
          },
        ],
      },
      webhook_knowledge: {
        id: ids.webhookKnowledge,
        revision_id: ids.webhookRevision,
        candidate_id: ids.webhookCandidate,
        outcome: "applied",
      },
      private_knowledge: {
        id: ids.privateKnowledge,
        revision_id: ids.privateRevision,
        candidate_id: ids.privateCandidate,
        outcome: "applied",
      },
      reuse_session: session(ids.reuseSession, "bob-subject"),
      reuse_context: context(
        ids.reuseContext,
        ids.reuseSession,
        "Webhook deliveries are deduplicated by provider event ID.",
      ),
      private_isolation: {
        session_id: ids.reuseSession,
        private_knowledge_id: ids.privateKnowledge,
        inspected_count: 1,
        private_knowledge_absent: true,
        evidence_kind: "owner_scope",
      },
      current_session: session(ids.currentSession, "bob-subject"),
      current_context: context(
        ids.currentContext,
        ids.currentSession,
        "PulseBoard public requests use the W3C traceparent header.",
      ),
      release_skill: {
        outcome: "applied",
        skill_id: ids.releaseSkill,
        version_id: ids.releaseSkillVersion,
      },
      release_skill_validation: {
        id: ids.releaseSkillValidation,
        kind: "skill_validation",
        operation_version: 1,
        state: "succeeded",
        skill_id: ids.releaseSkill,
        skill_version_id: ids.releaseSkillVersion,
        progress_percent: 100,
        attempts: 1,
        test_run_id: ids.releaseSkillTestRun,
      },
    },
  };
}

function status() {
  const currentReceipt = receipt();
  const resources = currentReceipt.resources;
  const ended = (session) => ({ ...session, status: "ended" });
  const knowledge = (handle, owner) => ({
    id: handle.id,
    ...(owner === undefined
      ? { project_id: resources.project.id }
      : { owner_principal_id: owner }),
    lifecycle_state: "active",
    current_revision: {
      id: handle.revision_id,
      knowledge_item_id: handle.id,
    },
  });
  const detail = (run, selections) => ({
    run: {
      ...run,
      rendered: undefined,
      trace_retention_mode: "redacted",
      selection_count: selections.length,
    },
    candidates: [],
    selections,
    feedback: [],
  });
  const sharedSelection = {
    channel: "current_knowledge",
    knowledge_item_id: resources.webhook_knowledge.id,
    knowledge_revision_id: resources.webhook_knowledge.revision_id,
  };
  return {
    receipt: currentReceipt,
    live: {
      workspace: { status: "visible", value: resources.workspace },
      project: { status: "visible", value: resources.project },
      first_session: {
        status: "visible",
        value: ended(resources.first_session),
      },
      first_capture: {
        status: "visible",
        value: resources.first_capture,
      },
      reuse_session: {
        status: "visible",
        value: ended(resources.reuse_session),
      },
      current_session: {
        status: "visible",
        value: ended(resources.current_session),
      },
      webhook_knowledge: {
        status: "visible",
        value: knowledge(resources.webhook_knowledge, undefined),
      },
      private_knowledge: {
        status: "visible",
        value: knowledge(resources.private_knowledge, "alice-subject"),
      },
      reuse_context: {
        status: "visible",
        value: detail(resources.reuse_context, [sharedSelection]),
      },
      current_context: {
        status: "visible",
        value: detail(resources.current_context, [sharedSelection]),
      },
      release_skill_validation: {
        status: "visible",
        value: { ...resources.release_skill_validation },
      },
    },
  };
}

function pendingPrivateReceipt() {
  const value = receipt();
  delete value.resources.private_knowledge.revision_id;
  value.resources.private_knowledge.change_id = uuid(28);
  value.resources.private_knowledge.outcome = "pending_review";
  value.resources.private_isolation.evidence_kind =
    "pending_review_not_published";
  value.notices = [
    "private quick-test preference remains pending in Advanced Reviews; the demo does not claim it as active Knowledge",
  ];
  return value;
}

function pendingPrivateStatus() {
  const value = status();
  value.receipt = pendingPrivateReceipt();
  value.live.private_knowledge = {
    status: "unavailable",
    reason: "not published",
  };
  return value;
}

function pendingSkillReceipt(value = receipt()) {
  value.resources.release_skill.outcome = "pending_review";
  value.resources.release_skill.change_id = uuid(29);
  delete value.resources.release_skill_validation;
  value.notices ??= [];
  value.notices.push(
    "Release Skill installation is in Advanced > Reviews; no unreviewed version was advertised or pinned",
    "Release Skill validation remains pending until its governed version is applied",
  );
  return value;
}

function governedStatus() {
  const value = pendingPrivateStatus();
  value.receipt = pendingSkillReceipt(value.receipt);
  delete value.live.release_skill_validation;
  return value;
}

const RETRY_RULE =
  "Retried ingestion requests must reuse the original Idempotency-Key. While the original request is running, the retry returns 409; after completion, it replays the stored response without starting a second ingestion.";

function retryReceipt(state = "seeded") {
  const ids = {
    tenantScope: uuid(100),
    workspace: uuid(101),
    workspaceScope: uuid(102),
    project: uuid(103),
    projectScope: uuid(104),
    repository: uuid(105),
    reviewerGrant: uuid(106),
    administratorGrant: uuid(107),
    approverGrant: uuid(126),
    viewerGrant: uuid(108),
    baselineChange: uuid(109),
    baselineKnowledge: uuid(110),
    baselineRevision: uuid(111),
    skillChange: uuid(112),
    skill: uuid(113),
    skillVersion: uuid(114),
    session: uuid(115),
    event: uuid(116),
    capture: uuid(117),
    candidate: uuid(118),
    learning: uuid(119),
    learningChange: uuid(120),
    bindingChange: uuid(121),
    binding: uuid(122),
    learningRevision: uuid(123),
    contextSession: uuid(124),
    contextRun: uuid(125),
  };
  const author = {
    label: "Avery Author",
    subject: "author-subject",
    credential_profile: "author",
  };
  const reviewer = {
    label: "Riley Reviewer",
    subject: "reviewer-subject",
    credential_profile: "reviewer",
  };
  const approver = {
    label: "Morgan Approver",
    subject: "approver-subject",
    credential_profile: "approver",
  };
  const viewer = {
    label: "Vera Restricted Viewer",
    subject: "viewer-subject",
    credential_profile: "viewer",
  };
  const workspace = {
    id: ids.workspace,
    scope_id: ids.workspaceScope,
    slug: "northstar-delivery-demo",
    display_name: "Northstar Delivery",
    description: "Northstar delivery workspace",
    status: "active",
    revision: 1,
    created_by: author.subject,
    created_at: "2026-09-10T09:00:00Z",
    updated_at: "2026-09-10T09:00:00Z",
  };
  const project = {
    id: ids.project,
    workspace_id: ids.workspace,
    scope_id: ids.projectScope,
    slug: "ingestion-api",
    status: "active",
  };
  const grant = (id, principal_id, role) => ({
    id,
    subject_kind: "principal",
    principal_id,
    role,
    scope_id: ids.workspaceScope,
    source: "direct",
    directory_managed: false,
  });
  const sourceSession = {
    id: ids.session,
    workspace_id: ids.workspace,
    project_id: ids.project,
    scope_id: ids.projectScope,
    repository_id: ids.repository,
    principal_id: author.subject,
    client_name: "synveda-demo",
    external_session_id: "cpr45-retry-review-v1",
    status: "active",
    created_at: "2026-09-10T09:01:00Z",
    last_observed_at: "2026-09-10T09:00:00Z",
    updated_at: "2026-09-10T09:01:01Z",
  };
  const sourceEvent = {
    id: ids.event,
    session_id: ids.session,
    payload: { text: RETRY_RULE, synthetic: true, replay: true },
  };
  const resources = {
    workspace,
    project,
    repository: {
      id: ids.repository,
      project_id: ids.project,
      canonical_uri: "https://github.com/northstar-demo/ingestion-api",
    },
    grant_reviewer: grant(ids.reviewerGrant, reviewer.subject, "reviewer"),
    grant_administrator: grant(
      ids.administratorGrant,
      reviewer.subject,
      "administrator",
    ),
    grant_approver: grant(
      ids.approverGrant,
      approver.subject,
      "administrator",
    ),
    grant_viewer: grant(ids.viewerGrant, viewer.subject, "viewer"),
    baseline_knowledge: {
      outcome: "applied",
      change_id: ids.baselineChange,
      knowledge_item_id: ids.baselineKnowledge,
      revision_id: ids.baselineRevision,
    },
    baseline_provenance: {
      sources: [
        {
          source_type: "repository",
          source_revision: "northstar-demo-baseline-v1",
        },
      ],
    },
    curator_rule: {
      effective_at: ids.projectScope,
      source: `knowledge/* @${reviewer.subject}\n`,
    },
    skill_install: {
      outcome: ["binding_pending", "verified"].includes(state)
        ? "applied"
        : "pending_review",
      change_id: ids.skillChange,
      skill_id: ids.skill,
      version_id: ids.skillVersion,
    },
    source_session: sourceSession,
    source_event: sourceEvent,
  };
  if (state !== "seeded") {
    Object.assign(resources, {
      capture_batch: {
        id: ids.capture,
        session_id: ids.session,
        project_id: ids.project,
        scope_id: ids.projectScope,
        state: "completed",
      },
      capture_candidates: {
        candidates: [
          {
            id: ids.candidate,
            source_event_ids: [ids.event],
          },
        ],
      },
      learning: {
        id: ids.learning,
        candidate_id: ids.candidate,
        change_id: ids.learningChange,
        outcome: "pending_review",
      },
      source_session_closed: { ...sourceSession, status: "ended" },
    });
  }
  if (["binding_pending", "verified"].includes(state)) {
    resources.skill_binding = {
      outcome: "pending_review",
      change_id: ids.bindingChange,
      binding_id: ids.binding,
      skill_id: ids.skill,
      version_id: ids.skillVersion,
    };
  }
  if (state === "verified") {
    resources.context_session = { id: ids.contextSession };
    resources.context_run = { id: ids.contextRun };
    resources.verification = {
      knowledge_item_id: ids.learning,
      knowledge_revision_id: ids.learningRevision,
      source_event_id: ids.event,
      skill_id: ids.skill,
      skill_version_id: ids.skillVersion,
      skill_binding_id: ids.binding,
      context_run_id: ids.contextRun,
      audit_head_seq: 42,
    };
  }
  return {
    receipt_version: 2,
    fixture: "cpr45-retry-review-v1",
    gateway_url: SETTINGS.appOrigin,
    state,
    author,
    reviewer,
    approver,
    viewer,
    resources,
  };
}

function retryRerunReceipt() {
  const value = retryReceipt();
  value.resources.workspace.description =
    "Operator-edited CPR-45 acceptance description retained across seed replay.";
  value.resources.workspace.revision = 2;
  value.resources.workspace.updated_at = "2026-09-10T09:01:00Z";
  return value;
}

function retryInspection(seed = retryReceipt()) {
  return {
    synthetic: true,
    session: seed.resources.source_session,
    timeline: { events: [seed.resources.source_event] },
    source_event: seed.resources.source_event,
  };
}

function retryProposal(id) {
  return {
    id,
    state: "open",
    commit: "c".repeat(64),
    artifact_references: [{ family: "knowledge", artifact_id: uuid(130) }],
  };
}

function retryVerification(binding = retryReceipt("binding_pending")) {
  const resources = binding.resources;
  const revision = uuid(123);
  return {
    synthetic: true,
    knowledge: {
      id: resources.learning.id,
      current_revision: { id: revision, body_markdown: RETRY_RULE },
    },
    provenance: {
      sources: [
        {
          source_type: "session_event",
          session_event_id: resources.source_event.id,
        },
      ],
    },
    skill_version: { id: resources.skill_install.version_id },
    skill_binding: {
      id: resources.skill_binding.binding_id,
      pinned_version_id: resources.skill_install.version_id,
      enabled: true,
    },
    available_skills: {
      skills: [
        {
          binding: { id: resources.skill_binding.binding_id },
          version: { id: resources.skill_install.version_id },
        },
      ],
    },
    context: {
      selections: [
        {
          knowledge_item_id: resources.learning.id,
          knowledge_revision_id: revision,
        },
      ],
    },
    audit: {
      knowledge: { events: [{ id: uuid(131) }] },
      skill_binding: { events: [{ id: uuid(132) }] },
      session: { events: [{ id: uuid(133) }] },
      context: { events: [{ id: uuid(134) }] },
      chain: { valid: true, head_seq: 42 },
    },
  };
}

function retryStatus() {
  const receipt = retryReceipt("verified");
  const value = (id) => ({ id });
  const revision = receipt.resources.verification.knowledge_revision_id;
  return {
    receipt,
    live: {
      workspace: { status: "visible", value: value(receipt.resources.workspace.id) },
      project: { status: "visible", value: value(receipt.resources.project.id) },
      source_session: {
        status: "visible",
        value: {
          id: receipt.resources.source_session.id,
          status: "ended",
        },
      },
      capture_batch: {
        status: "visible",
        value: {
          id: receipt.resources.capture_batch.id,
          state: "completed",
        },
      },
      context_session: {
        status: "visible",
        value: value(receipt.resources.context_session.id),
      },
      context_run: {
        status: "visible",
        value: {
          run: value(receipt.resources.verification.context_run_id),
          selections: [
            {
              knowledge_item_id: receipt.resources.learning.id,
              knowledge_revision_id: revision,
            },
          ],
        },
      },
      baseline_knowledge: {
        status: "visible",
        value: {
          id: receipt.resources.baseline_knowledge.knowledge_item_id,
          current_revision: {
            id: receipt.resources.baseline_knowledge.revision_id,
          },
        },
      },
      learning: {
        status: "visible",
        value: {
          id: receipt.resources.learning.id,
          current_revision: { id: revision },
        },
      },
      learning_provenance: {
        status: "visible",
        value: {
          sources: [
            {
              source_type: "session_event",
              session_event_id: receipt.resources.source_event.id,
            },
          ],
        },
      },
      skill_install: {
        status: "visible",
        value: {
          id: receipt.resources.skill_install.skill_id,
          current_version_id: receipt.resources.skill_install.version_id,
        },
      },
      skill_binding: {
        status: "visible",
        value: {
          id: receipt.resources.skill_binding.binding_id,
          pinned_version_id: receipt.resources.skill_install.version_id,
          enabled: true,
        },
      },
      audit_chain: {
        status: "visible",
        value: {
          valid: true,
          head_seq: receipt.resources.verification.audit_head_seq,
        },
      },
    },
  };
}

function fakeChild() {
  const child = new EventEmitter();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  return child;
}

function successfulSpawn(outputs, calls) {
  return (file, args, options) => {
    const child = fakeChild();
    const output = outputs.shift();
    const result = output?.__result ?? { code: 0, stdout: JSON.stringify(output), stderr: "" };
    calls.push({ file, args, options });
    child.kill = () => true;
    queueMicrotask(() => {
      child.emit("exit", result.code, null);
      child.stdout.write(result.stdout ?? "");
      child.stdout.end();
      child.stderr.write(result.stderr ?? "");
      child.stderr.end();
      setImmediate(() => child.emit("close", result.code, null));
    });
    return child;
  };
}

test("CLI login and loopback handoff URLs are exact and secret-free", () => {
  const login = validateCliLoginUrl(startUrl(), SETTINGS);
  assert.equal(login.redirect, REDIRECT);
  assert.equal(login.state, STATE);
  const handoff = `${REDIRECT}?code=opaque-one-time-code&state=${STATE}`;
  assert.equal(validateCliHandoffUrl(handoff, login), true);
  assert.equal(allowedCliRequest(handoff, SETTINGS, login), true);
  assert.equal(
    allowedCliRequest(`${SETTINGS.appOrigin}/auth/login`, SETTINGS, login),
    true,
  );

  for (const mutant of [
    startUrl({ cli_redirect_uri: "http://localhost:43127/callback" }),
    startUrl({ cli_redirect_uri: "http://127.0.0.1/callback" }),
    startUrl({ cli_redirect_uri: "http://127.0.0.1:43127/other" }),
    startUrl({ cli_redirect_uri: "https://127.0.0.1:43127/callback" }),
    startUrl({ cli_state: "short" }),
    startUrl({ issuer: "http://other.invalid/realms/synveda" }),
    startUrl({ issuer: undefined }),
    startUrl({}, [["token", "private"]]),
    startUrl().replace("app.synveda.test", "other.synveda.test"),
  ]) refuse(() => validateCliLoginUrl(mutant, SETTINGS), "cli-login-url");

  for (const mutant of [
    `${REDIRECT}?code=opaque-one-time-code&state=wrong`,
    `${REDIRECT}?code=opaque-one-time-code&state=${STATE}&token=private`,
    `${REDIRECT}?state=${STATE}`,
    `http://127.0.0.1:43128/callback?code=opaque&state=${STATE}`,
    `http://user@127.0.0.1:43127/callback?code=opaque&state=${STATE}`,
  ]) refuse(() => validateCliHandoffUrl(mutant, login), "cli-handoff");
  assert.equal(
    allowedCliRequest("http://127.0.0.1:43127/favicon.ico", SETTINGS, login),
    false,
  );
  assert.equal(
    allowedCliRequest("http://127.0.0.1:43128/callback", SETTINGS, login),
    false,
  );
});

test("the product receipt requires the real team, Capture, Knowledge and reuse legs", () => {
  assert.equal(validateDemoReceipt(receipt()), true);
  assert.equal(validateDemoStatus(status()), true);
  assert.equal(validateDemoReceipt(pendingPrivateReceipt()), true);
  assert.equal(validateDemoStatus(pendingPrivateStatus()), true);
  assert.equal(validateDemoReceipt(pendingSkillReceipt()), true);
  assert.equal(validateDemoReceipt(governedStatus().receipt), true);
  assert.equal(validateDemoStatus(governedStatus()), true);
  for (const name of RESOURCE_NAMES) {
    const mutant = receipt();
    delete mutant.resources[name];
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }
  for (const name of STATUS_NAMES) {
    const mutant = status();
    mutant.live[name].status = "unavailable";
    refuse(() => validateDemoStatus(mutant), "product-status");
  }

  for (const mutate of [
    (value) => { value.resources.bob_member.grant.principal_id = "alice-subject"; },
    (value) => { value.resources.first_capture.state = "running"; },
    (value) => {
      value.resources.webhook_knowledge.candidate_id =
        value.resources.private_knowledge.candidate_id;
    },
    (value) => { value.resources.reuse_context.rendered = "test-fast"; },
    (value) => { value.resources.private_isolation.private_knowledge_absent = false; },
    (value) => { value.resources.private_isolation.evidence_kind = "pending_review_not_published"; },
    (value) => { value.resources.release_skill_validation.state = "dead_lettered"; },
  ]) {
    const mutant = receipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => { value.notices = []; },
    (value) => {
      value.resources.release_skill_validation =
        receipt().resources.release_skill_validation;
    },
    (value) => { delete value.resources.release_skill.change_id; },
  ]) {
    const mutant = pendingSkillReceipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => { value.notices = []; },
    (value) => { value.resources.private_knowledge.revision_id = uuid(29); },
    (value) => { value.resources.private_isolation.evidence_kind = "owner_scope"; },
  ]) {
    const mutant = pendingPrivateReceipt();
    mutate(mutant);
    refuse(() => validateDemoReceipt(mutant), "product-demo");
  }

  for (const mutate of [
    (value) => {
      value.live.webhook_knowledge.value.current_revision.id = uuid(99);
    },
    (value) => {
      value.live.reuse_context.value.selections[0].knowledge_item_id =
        value.receipt.resources.private_knowledge.id;
    },
    (value) => { value.live.first_session.value.status = "active"; },
    (value) => { value.live.release_skill_validation.value.test_run_id = uuid(99); },
  ]) {
    const mutant = status();
    mutate(mutant);
    refuse(() => validateDemoStatus(mutant), "product-status");
  }
});

test("the retry-review contract proves first run, rerun and persisted evidence", () => {
  const seeded = retryReceipt();
  const rerun = retryRerunReceipt();
  const captured = retryReceipt("learning_pending");
  const binding = retryReceipt("binding_pending");
  assert.equal(validateRetryReviewReceipt(seeded), true);
  assert.equal(validateRetryReviewRerun(seeded, rerun), true);
  for (const mutate of [
    (value) => { value.resources.workspace.description = seeded.resources.workspace.description; },
    (value) => { value.resources.workspace.revision = 3; },
    (value) => { value.resources.workspace.updated_at = seeded.resources.workspace.updated_at; },
    (value) => { value.resources.workspace.display_name = "Repointed local fixture"; },
    (value) => { value.resources.project.description = "Changed by the second seed"; },
    (value) => { value.resources.source_session.updated_at = "2026-09-12T10:00:00Z"; },
  ]) {
    const mutant = structuredClone(rerun);
    mutate(mutant);
    refuse(() => validateRetryReviewRerun(seeded, mutant), "retry-review-rerun");
  }
  assert.equal(validateRetryReviewInspection(retryInspection(seeded), seeded), true);
  assert.equal(validateRetryReviewReceipt(captured, "learning_pending"), true);
  assert.equal(
    validateRetryReviewProposal(
      retryProposal(captured.resources.learning.change_id),
      captured.resources.learning.change_id,
    ),
    true,
  );
  assert.equal(validateRetryReviewReceipt(binding, "binding_pending"), true);
  assert.equal(
    validateRetryReviewVerification(retryVerification(binding), binding),
    true,
  );
  assert.equal(validateRetryReviewStatus(retryStatus()), true);
  const repeatedBinding = retryStatus();
  repeatedBinding.receipt.resources.skill_binding.outcome = "applied";
  assert.equal(validateRetryReviewStatus(repeatedBinding), true);

  const repointed = retryReceipt();
  repointed.resources.workspace.display_name = "Repointed local fixture";
  refuse(
    () => validateRetryReviewRerun(seeded, repointed),
    "retry-review-rerun",
  );

  const groupGrant = retryReceipt();
  groupGrant.resources.grant_viewer.subject_kind = "group";
  refuse(() => validateRetryReviewReceipt(groupGrant), "retry-review");
});

test("console checkpoints keep author reviewer and viewer browser identities separate", async () => {
  const receipt = retryReceipt("learning_pending");
  const proposal = retryProposal(receipt.resources.learning.change_id);
  const status = retryStatus();
  const sessions = [];
  const runSession = async (options) => {
    sessions.push(options);
    return true;
  };

  for (const checkpoint of [
    "seeded-edit",
    "seeded-preserved",
    "viewer-denial",
    "reviewer-open",
    "approved-not-applied",
  ]) {
    assert.equal(
      await runConsoleProductCheckpoint({
        chromium: {},
        checkpoint,
        proposal,
        receipt,
        runSession,
      }),
      true,
    );
  }
  assert.equal(
    await runConsoleProductCheckpoint({
      chromium: {},
      checkpoint: "verified",
      runSession,
      status,
    }),
    true,
  );

  assert.deepEqual(
    sessions.map(({ admissionStage, expectedSubject, passwordFile, requiredRoleKeys, username }) => ({
      admissionStage,
      expectedSubject,
      passwordFile,
      requiredRoleKeys,
      username,
    })),
    [
      ...Array.from({ length: 2 }, () => ({
        admissionStage: "author-admission",
        expectedSubject: receipt.author.subject,
        passwordFile: "/run/secrets/keycloak_demo_admin_password",
        requiredRoleKeys: ["administrator"],
        username: "synveda-demo-admin",
      })),
      {
        admissionStage: "viewer-admission",
        expectedSubject: receipt.viewer.subject,
        passwordFile: "/run/secrets/keycloak_demo_viewer_password",
        requiredRoleKeys: [],
        username: "synveda-demo-viewer",
      },
      {
        admissionStage: "reviewer-admission",
        expectedSubject: receipt.reviewer.subject,
        passwordFile: "/run/secrets/keycloak_demo_member_password",
        requiredRoleKeys: [],
        username: "synveda-demo-member",
      },
      {
        admissionStage: "author-admission",
        expectedSubject: receipt.author.subject,
        passwordFile: "/run/secrets/keycloak_demo_admin_password",
        requiredRoleKeys: ["administrator"],
        username: "synveda-demo-admin",
      },
      {
        admissionStage: "author-admission",
        expectedSubject: status.receipt.author.subject,
        passwordFile: "/run/secrets/keycloak_demo_admin_password",
        requiredRoleKeys: ["administrator"],
        username: "synveda-demo-admin",
      },
    ],
  );
});

test("seed replays the staged fixture through four real identities and one denied review", async () => {
  const calls = [];
  const passwords = [
    Buffer.from("a".repeat(64)),
    Buffer.from("b".repeat(64)),
    Buffer.from("c".repeat(64)),
    Buffer.from("d".repeat(64)),
  ];
  let passwordIndex = 0;
  let seedCount = 0;
  const command = async (args, _environment, _timeout, _spawn, expectation = "json") => {
    calls.push(["command", expectation, ...args]);
    if (args[0] === "proposal") {
      return args[1] === "show" ? retryProposal(args[2]) : true;
    }
    switch (args[2]) {
      case "seed":
        seedCount += 1;
        return seedCount === 1 ? retryReceipt() : retryRerunReceipt();
      case "inspect": return retryInspection();
      case "capture": return retryReceipt("learning_pending");
      case "bind-skill": return retryReceipt("binding_pending");
      case "verify": return retryVerification();
      case "status": return retryStatus();
      default: throw new Error(`unexpected command ${args.join(" ")}`);
    }
  };
  const login = async ({ profile, username, password }) => {
    calls.push(["login", profile, username]);
    assert.equal(password.length, 64);
  };
  const browserCheckpoint = async ({ checkpoint, receipt, proposal, status }) => {
    calls.push([
      "browser",
      checkpoint,
      receipt?.fixture ?? status?.receipt?.fixture,
      proposal?.id,
    ]);
    return true;
  };
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => passwords[passwordIndex++],
      login,
      command,
      browserCheckpoint,
    }),
    true,
  );
  assert.deepEqual(calls.slice(0, 4), [
    ["login", "author", "synveda-demo-admin"],
    ["login", "reviewer", "synveda-demo-member"],
    ["login", "approver", "synveda-demo-approver"],
    ["login", "viewer", "synveda-demo-viewer"],
  ]);
  const commands = calls.filter(([kind]) => kind === "command");
  assert.equal(commands.length, 18);
  assert.equal(commands.filter((call) => call[4] === "seed").length, 2);
  assert.equal(
    commands.filter(
      (call) =>
        call[2] === "proposal" && call[3] === "approve" && call.includes("viewer"),
    ).length,
    0,
  );
  assert.equal(
    commands.filter(
      (call) => call[2] === "proposal" && call[3] === "approve" && call.includes("reviewer"),
    ).length,
    3,
  );
  assert.equal(
    commands.filter(
      (call) => call[2] === "proposal" && call[3] === "approve" && call.includes("approver"),
    ).length,
    2,
  );
  assert.equal(
    commands.filter(
      (call) => call[2] === "proposal" && call[3] === "apply" && call.includes("author"),
    ).length,
    3,
  );
  assert.deepEqual(commands.at(-1).slice(1, 5), [
    "json",
    "demo",
    "retry-review",
    "status",
  ]);
  assert.deepEqual(
    calls.filter(([kind]) => kind === "browser").map((call) => call[1]),
    [
      "seeded-edit",
      "seeded-preserved",
      "viewer-denial",
      "reviewer-open",
      "approved-not-applied",
      "verified",
    ],
  );
  for (const password of passwords) {
    assert.ok(password.every((value) => value === 0));
  }
});

test("post-restart verification requires the persisted receipt and only the author login", async () => {
  const calls = [];
  const adminPassword = Buffer.from("a".repeat(64));
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      phase: "verify",
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: (path) => {
        calls.push(["password", path]);
        return adminPassword;
      },
      login: async ({ profile, username, password }) => {
        calls.push(["login", profile, username]);
        password.fill(0);
      },
      command: async (args) => {
        calls.push(["command", ...args]);
        return retryStatus();
      },
      browserCheckpoint: async () => true,
    }),
    true,
  );
  assert.deepEqual(calls, [
    ["password", "/run/secrets/keycloak_demo_admin_password"],
    ["login", "author", "synveda-demo-admin"],
    [
      "command",
      "demo",
      "retry-review",
      "status",
      "--author-credentials",
      "author",
      "--json",
    ],
  ]);
  assert.ok(adminPassword.every((value) => value === 0));
});

test("the reference acceptance path retains the existing portable product receipt", async () => {
  const reference = validateSettings(
    "https://synveda.example.com",
    "https://identity.example.com/realms/synveda",
  );
  const calls = [];
  const passwords = [Buffer.from("a".repeat(64)), Buffer.from("b".repeat(64))];
  const command = async (args) => {
    calls.push(["command", ...args]);
    return args[1] === "status" ? status() : receipt();
  };
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        SYNVEDA_BROWSER_APP_URL: reference.appOrigin,
        SYNVEDA_BROWSER_ISSUER: reference.issuer,
      },
      readPassword: () => passwords.shift(),
      login: async ({ profile, username, password }) => {
        calls.push(["login", profile, username]);
        password.fill(0);
      },
      command,
    }),
    true,
  );
  assert.deepEqual(calls.slice(0, 2), [
    ["login", "alice", "synveda-demo-admin"],
    ["login", "bob", "synveda-demo-member"],
  ]);
  assert.equal(
    calls.filter((call) => call[0] === "command" && call[2] === "start").length,
    2,
  );
  assert.deepEqual(calls.at(-1), [
    "command",
    "demo",
    "status",
    "--credentials",
    "alice",
    "--json",
  ]);
});

test("the product runner refuses unknown phases before reading a secret", async () => {
  await assert.rejects(
    runProductAcceptance({
      chromium: { launch: async () => {} },
      phase: "other",
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => {
        throw new Error("must not read");
      },
    }),
    (error) => {
      assert.ok(error instanceof BrowserContractError);
      assert.equal(error.stage, "configuration");
      return true;
    },
  );
});

test("a partial password read failure clears the first secret and stops", async () => {
  const adminPassword = Buffer.from("a".repeat(64));
  const memberPassword = Buffer.from("b".repeat(64));
  let reads = 0;
  await assert.rejects(
    runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
      },
      readPassword: () => {
        reads += 1;
        if (reads === 1) return adminPassword;
        if (reads === 2) return memberPassword;
        throw new Error("viewer secret unavailable");
      },
      login: async () => {
        throw new Error("login must not run");
      },
      command: async () => {
        throw new Error("command must not run");
      },
    }),
  );
  assert.ok(adminPassword.every((value) => value === 0));
  assert.ok(memberPassword.every((value) => value === 0));
});

test("the exact CLI is spawned with a closed environment and waits for close", async () => {
  const calls = [];
  const passwords = [
    Buffer.from("a".repeat(64)),
    Buffer.from("b".repeat(64)),
    Buffer.from("c".repeat(64)),
    Buffer.from("d".repeat(64)),
  ];
  const seeded = retryReceipt();
  const captured = retryReceipt("learning_pending");
  const binding = retryReceipt("binding_pending");
  const ok = { __result: { code: 0, stdout: "", stderr: "" } };
  const outputs = [
    seeded,
    retryRerunReceipt(),
    retryInspection(seeded),
    captured,
    retryProposal(captured.resources.learning.change_id),
    ok,
    ok,
    retryProposal(captured.resources.skill_install.change_id),
    ok,
    ok,
    ok,
    binding,
    retryProposal(binding.resources.skill_binding.change_id),
    ok,
    ok,
    ok,
    retryVerification(binding),
    retryStatus(),
  ];
  assert.equal(
    await runProductAcceptance({
      chromium: { launch: async () => {} },
      environment: {
        HOME: "/tmp/browser-home",
        HTTP_PROXY: "http://private-proxy.invalid",
        NODE_OPTIONS: "--require=/private/injection.cjs",
        SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
        SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
        SYNVEDA_INSECURE_DEVELOPMENT_HTTP: "true",
        SYNVEDA_TOKEN: "private-bearer",
        XDG_CONFIG_HOME: "/tmp/browser-home/config",
        XDG_STATE_HOME: "/tmp/browser-home/state",
      },
      readPassword: () => passwords.shift(),
      login: async ({ password }) => password.fill(0),
      browserCheckpoint: async () => true,
      spawnProcess: successfulSpawn(outputs, calls),
    }),
    true,
  );
  assert.equal(calls.length, 18);
  for (const call of calls) {
    assert.equal(call.file, "/usr/local/bin/synveda");
    assert.deepEqual(Object.keys(call.options.env).sort(), [
      "HOME",
      "LANG",
      "LC_ALL",
      "PATH",
      "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
      "TZ",
      "XDG_CONFIG_HOME",
      "XDG_STATE_HOME",
    ]);
    assert.doesNotMatch(JSON.stringify(call.options), /private-bearer|private-proxy|injection/);
  }
});

test("CLI timeout escalates to SIGKILL and oversized output fails closed", async () => {
  for (const mode of ["timeout", "oversized"]) {
    const signals = [];
    const spawnProcess = () => {
      const child = fakeChild();
      let closed = false;
      const close = (signal) => {
        if (closed) return;
        closed = true;
        child.stdout.end();
        child.stderr.end();
        queueMicrotask(() => child.emit("close", null, signal));
      };
      child.kill = (signal) => {
        signals.push(signal);
        if (mode === "oversized" || signal === "SIGKILL") close(signal);
        return true;
      };
      if (mode === "oversized") {
        queueMicrotask(() => child.stdout.write(Buffer.alloc(2 * 1024 * 1024 + 1)));
      }
      return child;
    };
    const passwords = [
      Buffer.from("a".repeat(64)),
      Buffer.from("b".repeat(64)),
      Buffer.from("c".repeat(64)),
      Buffer.from("d".repeat(64)),
    ];
    await assert.rejects(
      runProductAcceptance({
        chromium: { launch: async () => {} },
        environment: {
          SYNVEDA_BROWSER_APP_URL: SETTINGS.appOrigin,
          SYNVEDA_BROWSER_ISSUER: SETTINGS.issuer,
        },
        readPassword: () => passwords.shift(),
        login: async ({ password }) => password.fill(0),
        spawnProcess,
        demoTimeout: 10,
      }),
      (error) => {
        assert.ok(error instanceof BrowserContractError);
        assert.equal(error.stage, "product-demo");
        return true;
      },
    );
    assert.ok(signals.includes("SIGTERM"));
    if (mode === "timeout") assert.ok(signals.includes("SIGKILL"));
  }
});

test("the fixture copies the product CLI and exposes no token-transfer shortcut", () => {
  const dockerfile = readFileSync(
    join(ROOT, "deploy/compose/product/Dockerfile"),
    "utf8",
  );
  const runner = readFileSync(
    join(ROOT, "deploy/compose/browser/product-demo-runner.mjs"),
    "utf8",
  );
  const consoleRunner = readFileSync(
    join(ROOT, "deploy/compose/browser/console-product-runner.mjs"),
    "utf8",
  );
  const lifecycle = readFileSync(
    join(ROOT, "deploy/compose/scripts/compose.sh"),
    "utf8",
  );
  assert.match(
    dockerfile,
    /^COPY --from=build \/src\/target\/release\/synveda \/usr\/local\/bin\/synveda$/m,
  );
  assert.match(
    dockerfile,
    /^FROM debian:bookworm-slim@sha256:[0-9a-f]{64} AS runtime$/m,
  );
  for (const forbidden of [
    /SYNVEDA_TOKEN/,
    /password_grant/,
    /grant_type[^\n]*password/,
    /client_secret/,
    /docker\.sock/,
    /\.screenshot\s*\(/,
    /storageState\s*[:(]/,
    /ignoreHTTPSErrors/,
    /--no-sandbox/,
  ]) {
    assert.doesNotMatch(runner, forbidden);
    assert.doesNotMatch(consoleRunner, forbidden);
  }
  assert.match(runner, /"login",\s*"--gateway"/);
  assert.match(runner, /"demo",\s*"retry-review",\s*"seed"/);
  assert.match(runner, /profile:\s*"viewer"/);
  assert.match(runner, /checkpoint:\s*"viewer-denial"/);
  assert.match(consoleRunner, /expectedSubject:\s*receipt\?\.viewer\?\.subject/);
  assert.match(
    consoleRunner,
    /\^Synthetic replay: determine ingestion retry behaviour\\s\+running\$/,
  );
  assert.match(consoleRunner, /atProductStage\("console-seeded-session"/);
  assert.match(consoleRunner, /atProductStage\("console-seeded-review-page"/);
  assert.match(consoleRunner, /atProductStage\("console-seeded-review-state"/);
  assert.match(consoleRunner, /atProductStage\("console-confirmation-open"/);
  assert.match(consoleRunner, /atProductStage\("console-confirmation-dismiss"/);
  assert.match(consoleRunner, /getByRole\("combobox", \{ name: \/\^workspace\$\/i \}\)/);
  assert.match(consoleRunner, /getByRole\("textbox", \{ name: \/\^description\$\/i \}\)/);
  assert.match(
    lifecycle,
    /run_product_acceptance seed "\$@"[\s\S]*restart_service_name=postgres[\s\S]*rerun_browser_acceptance "\$@"[\s\S]*run_product_acceptance verify "\$@"/,
  );
  assert.doesNotMatch(lifecycle, /product_acceptance_state|product-demo\.mjs start/);
});

import assert from "node:assert/strict";
import { test } from "node:test";

import { describe } from "./client.mjs";
import {
  EMPTY_LEARNINGS_FILTERS,
  batchProgress,
  batchQuery,
  candidateQuery,
  decisionMessage,
  dismissBody,
  editAndAcceptBody,
  groupCandidates,
  mergeBody,
  placementEdits,
  proposedPublishScope,
  publishableScopes,
  replaceBody,
  type PublishScope,
} from "./learnings.mjs";
import { forecastAt, type Forecast } from "./forecast.mjs";
import type {
  CaptureBatchView,
  CaptureCandidateView,
  CaptureMatchView,
  MeView,
  ProjectView,
  WorkspaceView,
} from "./generated/api.js";

const PRINCIPAL_SCOPE = "scope-principal";

/** Publication choices answered by `/v1/me` anchors alone, as a fixture. */
function scopesFor(view: MeView, context: { projectId?: string; sourceScopeId?: string }) {
  return publishableScopes(view, context, (scopeId, action) =>
    forecastAt(view, { status: "loading" }, scopeId, action),
  );
}
const WORKSPACE_SCOPE = "scope-workspace";
const PROJECT_SCOPE = "scope-project";

function workspace(): WorkspaceView {
  return {
    id: "workspace-1",
    scope_id: WORKSPACE_SCOPE,
    slug: "payments",
    display_name: "Payments",
    status: "active",
    revision: 1,
    created_at: "2026-08-24T09:00:00Z",
    updated_at: "2026-08-24T09:00:00Z",
  };
}

function project(): ProjectView {
  return {
    id: "project-1",
    workspace_id: "workspace-1",
    scope_id: PROJECT_SCOPE,
    slug: "pulseboard",
    display_name: "PulseBoard",
    status: "active",
    revision: 1,
    created_at: "2026-08-24T09:00:00Z",
    updated_at: "2026-08-24T09:00:00Z",
  };
}

function me(writes: { principal?: boolean; project?: boolean; workspace?: boolean } = {}): MeView {
  return {
    principal: { subject: "alice@example.test", display_name: "Alice", quarantined: false },
    tenant: { id: "tenant-1", slug: "acme", name: "ACME", status: "active" },
    onboarding: {
      state: "ready",
      workspace_count: 1,
      project_count: 1,
      workspace_create: false,
      project_create_workspace_ids: [],
    },
    capabilities: { actions: {}, role_keys: ["member"] },
    workspaces: [workspace()],
    projects: [project()],
    anchors: [
      {
        scope_id: PRINCIPAL_SCOPE,
        kind: "principal",
        source: "principal_scope",
        direct: true,
        roles: ["member"],
        actions: { "knowledge.write": writes.principal ?? true },
      },
      {
        scope_id: PROJECT_SCOPE,
        kind: "project",
        source: "selected_project",
        direct: true,
        roles: ["member"],
        actions: { "knowledge.write": writes.project ?? true },
      },
      {
        scope_id: WORKSPACE_SCOPE,
        kind: "workspace",
        source: "selected_workspace",
        direct: false,
        roles: ["member"],
        actions: { "knowledge.write": writes.workspace ?? true },
      },
    ],
  };
}

function candidate(overrides: Partial<CaptureCandidateView> = {}): CaptureCandidateView {
  return {
    id: "candidate-1",
    batch_id: "batch-1",
    source_kind: "session",
    session_id: "session-1",
    ordinal: 1,
    proposed_scope_id: PROJECT_SCOPE,
    proposed_project_id: "project-1",
    knowledge_type: "convention",
    origin: "observed",
    content: {
      title: "Request correlation",
      summary: "Public requests use X-Request-Id.",
      body_markdown: "Public requests use `X-Request-Id`.",
      tags: ["http"],
      sensitivity: "internal",
      confidence_permille: 940,
    },
    content_hash: "hash-1",
    state: "pending",
    source_event_ids: ["event-1"],
    source_artifact_ids: [],
    matches: [],
    content_erased: false,
    created_at: "2026-08-24T10:00:00Z",
    ...overrides,
  };
}

function batch(overrides: Partial<CaptureBatchView> = {}): CaptureBatchView {
  return {
    id: "batch-1",
    source_kind: "session",
    session_id: "session-1",
    scope_id: PROJECT_SCOPE,
    project_id: "project-1",
    input_hash: "batch-hash",
    event_count: 4,
    state: "completed",
    extractor_method: "deterministic",
    model_version: "builtin@3",
    attempts: 1,
    max_attempts: 5,
    manual_retries: 0,
    candidate_count: 3,
    configuration_hash: "configuration-hash",
    configuration_version_id: "configuration-version-1",
    created_at: "2026-08-24T10:00:00Z",
    ...overrides,
  };
}

function matched(overrides: Partial<CaptureMatchView> = {}): CaptureMatchView {
  return {
    knowledge_item_id: "knowledge-old",
    knowledge_revision_id: "revision-old",
    kind: "supersession",
    similarity_permille: 860,
    reason_code: "shared_terms_with_polarity_change",
    ...overrides,
  };
}

test("project and session filters shape both cursor-paginated collection calls", () => {
  const filters = {
    ...EMPTY_LEARNINGS_FILTERS,
    projectId: " project-1 ",
    sessionId: " session-1 ",
    state: "pending" as const,
  };
  assert.deepEqual(candidateQuery(filters, "candidate-cursor"), {
    project_id: "project-1",
    session_id: "session-1",
    cursor: "candidate-cursor",
    limit: "200",
  });
  assert.deepEqual(batchQuery(filters, "batch-cursor"), {
    project_id: "project-1",
    session_id: "session-1",
    cursor: "batch-cursor",
    limit: "100",
  });
});

test("publication choices are private, project and workspace placements filtered by policy", () => {
  const all = scopesFor(me(), { projectId: "project-1", sourceScopeId: PROJECT_SCOPE });
  assert.deepEqual(
    all.map((scope) => [scope.visibility, scope.label]),
    [
      ["private", "Private to me"],
      ["project", "Shared with project · PulseBoard"],
      ["workspace", "Shared at workspace · Payments"],
    ],
  );

  const denied = scopesFor(me({ project: false, workspace: false }), {
    projectId: "project-1",
    sourceScopeId: PROJECT_SCOPE,
  });
  assert.deepEqual(denied.map((scope) => scope.id), [PRINCIPAL_SCOPE]);
  assert.equal(proposedPublishScope(candidate(), denied), null);
  assert.equal(
    denied.some((scope) => scope.id === PROJECT_SCOPE || scope.id === WORKSPACE_SCOPE),
    false,
    "a denied destination never becomes a selectable option",
  );
});

test("an inherited project is offered from its own forecast, not from a missing anchor", () => {
  // A workspace member's project has no anchor: the selected-target probe
  // answers there, and nothing is offered while it is still unknown.
  const view = me();
  view.anchors = view.anchors.filter((anchor) => anchor.scope_id !== PROJECT_SCOPE);
  const context = { projectId: "project-1", sourceScopeId: PROJECT_SCOPE };
  const answered = (project: Forecast) => (scopeId: string, action: string): Forecast =>
    scopeId === PROJECT_SCOPE ? project : forecastAt(view, { status: "loading" }, scopeId, action);
  assert.ok(
    publishableScopes(view, context, answered("allowed")).some((scope) => scope.id === PROJECT_SCOPE),
  );
  for (const unsettled of ["loading", "failed", "unanswered", "denied"] as const) {
    assert.ok(
      !publishableScopes(view, context, answered(unsettled)).some((scope) => scope.id === PROJECT_SCOPE),
      unsettled,
    );
  }
});

test("plain accept stays plain, while changing scope sends explicit placement nulls", () => {
  const scopes = scopesFor(me(), { projectId: "project-1" });
  const proposed = proposedPublishScope(candidate(), scopes);
  assert.ok(proposed);
  assert.deepEqual(placementEdits(candidate(), proposed), {});

  const personal = scopes.find((scope) => scope.visibility === "private") as PublishScope;
  assert.deepEqual(placementEdits(candidate(), personal), {
    scope_id: PRINCIPAL_SCOPE,
    project_id: null,
    owner_principal_id: "alice@example.test",
  });
});

test("edit-and-accept sends complete immutable content and a changed type", () => {
  const target = scopesFor(me(), { projectId: "project-1" }).find(
    (scope) => scope.visibility === "project",
  ) as PublishScope;
  const content = {
    ...candidate().content,
    title: "Trace correlation",
    body_markdown: "Use `traceparent`.",
    summary: "Use traceparent.",
  };
  assert.deepEqual(editAndAcceptBody(candidate(), target, { knowledgeType: "decision", content }), {
    content,
    knowledge_type: "decision",
  });
});

test("merge and replace carry exact inspected revision preconditions", () => {
  const target = scopesFor(me(), { projectId: "project-1" }).find(
    (scope) => scope.visibility === "project",
  ) as PublishScope;
  assert.deepEqual(mergeBody(candidate(), target, matched()), {
    inputs: [{ item_id: "knowledge-old", revision_id: "revision-old" }],
  });
  assert.deepEqual(replaceBody(candidate(), target, matched()), {
    item_id: "knowledge-old",
    expected_revision_id: "revision-old",
  });
});

test("all five decisions use generated idempotent public operations", () => {
  const target = scopesFor(me(), { projectId: "project-1" })[1] as PublishScope;
  const operations = [
    describe("accept_capture_candidate", {
      path: { id: "candidate-1" },
      body: placementEdits(candidate(), target),
      idempotencyKey: "accept-key",
    }),
    describe("accept_capture_candidate", {
      path: { id: "candidate-1" },
      body: editAndAcceptBody(candidate(), target, {
        knowledgeType: "convention",
        content: { ...candidate().content, title: "Edited" },
      }),
      idempotencyKey: "edit-key",
    }),
    describe("merge_capture_candidate", {
      path: { id: "candidate-1" },
      body: mergeBody(candidate(), target, matched()),
      idempotencyKey: "merge-key",
    }),
    describe("replace_capture_candidate", {
      path: { id: "candidate-1" },
      body: replaceBody(candidate(), target, matched()),
      idempotencyKey: "replace-key",
    }),
    describe("dismiss_capture_candidate", {
      path: { id: "candidate-1" },
      body: dismissBody("  incidental detail  "),
      idempotencyKey: "dismiss-key",
    }),
  ];
  assert.deepEqual(
    operations.map((operation) => [operation.path, operation.init.method]),
    [
      ["/capture-candidates/candidate-1/accept", "POST"],
      ["/capture-candidates/candidate-1/accept", "POST"],
      ["/capture-candidates/candidate-1/merge", "POST"],
      ["/capture-candidates/candidate-1/replace", "POST"],
      ["/capture-candidates/candidate-1/dismiss", "POST"],
    ],
  );
  assert.deepEqual(JSON.parse(operations[4]?.init.body as string), {
    reason: "incidental detail",
  });
  for (const operation of operations) {
    assert.ok(new Headers(operation.init.headers).has("idempotency-key"));
  }
});

test("batch progress and grouping distinguish pending, decided and partially loaded", () => {
  const rows = [
    candidate(),
    candidate({ id: "candidate-2", ordinal: 2, state: "dismissed" }),
  ];
  assert.equal(batchProgress(batch(), rows), "1 reviewed · 1 pending · 2 of 3 loaded");
  assert.deepEqual(
    groupCandidates([batch()], rows, "pending")[0]?.candidates.map((entry) => entry.id),
    ["candidate-1"],
  );
});

test("pending review and dismissal remain visibly separate from published Knowledge", () => {
  assert.match(
    decisionMessage(
      candidate({
        state: "accepted",
        resulting_outcome: "pending_review",
        resulting_change_id: "change-1",
      }),
    ) ?? "",
    /waiting in Advanced Reviews.*not active Knowledge yet/,
  );
  assert.equal(
    decisionMessage(candidate({ state: "dismissed", decision_reason: "incidental" })),
    "Dismissed. No Knowledge was published.",
  );
});

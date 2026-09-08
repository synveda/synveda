/** Customer-visible acceptance for the bounded CPR-45 Operations page. */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { describe } from "./client.mjs";
import { Operations, operationsReadPlan } from "./Operations.js";
import { AppProvider, appContext } from "./Shell.js";
import { reconcile } from "./selection.mjs";
import { toText } from "./text.mjs";
import type {
  CaptureBatchView,
  ContextRunView,
  MeView,
  ProjectView,
  SessionView,
  WorkspaceView,
} from "./generated/api.js";

const WORKSPACE_ID = "workspace-1";
const PROJECT_ID = "project-1";
const PROJECT_SCOPE = "scope-project-1";

function workspace(): WorkspaceView {
  return {
    id: WORKSPACE_ID,
    scope_id: "scope-workspace-1",
    slug: "payments",
    display_name: "Payments",
    status: "active",
    revision: 1,
    created_at: "2026-09-08T09:00:00Z",
    updated_at: "2026-09-08T09:00:00Z",
  };
}

function project(): ProjectView {
  return {
    id: PROJECT_ID,
    workspace_id: WORKSPACE_ID,
    scope_id: PROJECT_SCOPE,
    slug: "pulseboard",
    display_name: "PulseBoard",
    status: "active",
    revision: 1,
    created_at: "2026-09-08T09:00:00Z",
    updated_at: "2026-09-08T09:00:00Z",
  };
}

function me(withProject = true): MeView {
  return {
    principal: { subject: "alice@example.test", display_name: "Alice", quarantined: false },
    tenant: { id: "tenant-1", slug: "acme", name: "ACME", status: "active" },
    onboarding: {
      state: withProject ? "ready" : "needs_project",
      workspace_count: 1,
      project_count: withProject ? 1 : 0,
    },
    capabilities: { actions: {}, role_keys: ["member"] },
    workspaces: [workspace()],
    projects: withProject ? [project()] : [],
    anchors: [],
  };
}

function render(view = me()): { markup: string; text: string } {
  const selection = reconcile(
    { workspaceId: WORKSPACE_ID, projectId: view.projects.length > 0 ? PROJECT_ID : null },
    view,
  );
  const markup = renderToStaticMarkup(
    <AppProvider value={appContext(view, selection, () => {})}>
      <Operations />
    </AppProvider>,
  );
  return { markup, text: toText(markup) };
}

const plan = operationsReadPlan(PROJECT_ID, PROJECT_SCOPE);
const ok = (body: unknown): Outcome => ({ kind: "ok", body });

async function seed(key: string, outcome: Outcome): Promise<void> {
  await cache.ensure(key, async () => outcome);
}

function session(): SessionView {
  return {
    id: "session-1",
    workspace_id: WORKSPACE_ID,
    project_id: PROJECT_ID,
    scope_id: PROJECT_SCOPE,
    principal_id: "PRIVATE-PRINCIPAL",
    client_name: "claude-code",
    client_installation_id: "PRIVATE-INSTALLATION",
    external_session_id: "PRIVATE-EXTERNAL-SESSION",
    task_summary: "PRIVATE-SESSION-CONTENT",
    end_reason: "PRIVATE-END-REASON",
    metadata: { secret: "PRIVATE-SESSION-METADATA" },
    repository_id: "PRIVATE-REPOSITORY",
    agent_name: "PRIVATE-AGENT",
    model_name: "PRIVATE-MODEL",
    branch: "PRIVATE-BRANCH",
    status: "active",
    started_at: "2026-09-08T10:00:00Z",
    last_observed_at: "2026-09-08T10:03:00Z",
    created_at: "2026-09-08T10:00:00Z",
    updated_at: "2026-09-08T10:03:00Z",
  };
}

function contextRun(): ContextRunView {
  return {
    id: "context-run-1",
    session_id: "PRIVATE-CONTEXT-SESSION",
    workspace_id: WORKSPACE_ID,
    project_id: PROJECT_ID,
    scope_id: PROJECT_SCOPE,
    query: "PRIVATE-CONTEXT-QUERY",
    rendered: "PRIVATE-RENDERED-CONTEXT",
    skills: { secret: "PRIVATE-SKILL-CONTENT" },
    block_hash: "PRIVATE-BLOCK-HASH",
    configuration_hash: "PRIVATE-CONFIGURATION-HASH",
    retrieval_version: "PRIVATE-RETRIEVAL-VERSION",
    index_version: "PRIVATE-INDEX-VERSION",
    embedding_model: "PRIVATE-EMBEDDING-MODEL",
    as_of: "2026-09-08T10:04:00Z",
    created_at: "2026-09-08T10:04:00Z",
    completion_status: "completed",
    degraded: ["embedder", "PRIVATE-DEGRADED-DETAIL"],
    budget_tokens: 99001,
    tokens: 99002,
    candidate_count: 99003,
    selection_count: 99004,
    entry_count: 99005,
    trace_retention_mode: "full",
  };
}

function captureBatch(): CaptureBatchView {
  return {
    id: "capture-1",
    source_kind: "session",
    session_id: "PRIVATE-CAPTURE-SESSION",
    project_id: PROJECT_ID,
    scope_id: PROJECT_SCOPE,
    input_hash: "PRIVATE-INPUT-HASH",
    configuration_hash: "PRIVATE-CAPTURE-CONFIGURATION",
    configuration_version_id: "PRIVATE-CAPTURE-CONFIGURATION-VERSION",
    extractor_method: "PRIVATE-EXTRACTOR",
    model_version: "PRIVATE-CAPTURE-MODEL",
    event_count: 2,
    candidate_count: 99006,
    attempts: 3,
    state: "failed",
    error_code: "dependency_unavailable",
    created_at: "2026-09-08T10:05:00Z",
    started_at: "2026-09-08T10:05:01Z",
    completed_at: "2026-09-08T10:05:02Z",
  };
}

beforeEach(() => cache.clear());

test("the page makes exactly three bounded generated public reads", () => {
  assert.equal(
    describe("list_sessions", { query: plan.sessions.query }).path,
    `/sessions?scope_id=${PROJECT_SCOPE}&project_id=${PROJECT_ID}&limit=8`,
  );
  assert.equal(
    describe("list_context_runs", { query: plan.contextRuns.query }).path,
    `/context-runs?project_id=${PROJECT_ID}&limit=8`,
  );
  assert.equal(
    describe("list_capture_batches", { query: plan.captureBatches.query }).path,
    `/capture-batches?project_id=${PROJECT_ID}&limit=8`,
  );
});

test("no selected project is an explicit empty state without a tenant fallback", () => {
  const { text } = render(me(false));
  assert.match(text, /Select a project/);
  assert.match(text, /no tenant-wide fallback is inferred/);
  assert.doesNotMatch(text, /Reading recent|Refresh all|dependency health/);
});

test("loading and unavailable signal boundaries are explicit", () => {
  const { text } = render();
  assert.match(text, /Reading recent sessions/);
  assert.match(text, /Reading recent context runs/);
  assert.match(text, /Reading recent Capture work/);
  assert.match(text, /Refresh all/);
  assert.match(text, /Sections may have different fetch times and can already be stale/);
  assert.match(text, /Not available through the public API yet/);
  assert.match(text, /worker last-seen and durable operation retry or dead-letter state/);
  assert.match(text, /latest backup and isolated-restore result/);
});

test("empty authorised pages remain three separate honest answers", async () => {
  await seed(plan.sessions.key, ok({ sessions: [], next_cursor: null }));
  await seed(plan.contextRuns.key, ok({ runs: [], next_cursor: null }));
  await seed(plan.captureBatches.key, ok({ batches: [], next_cursor: null }));

  const { text } = render();
  assert.match(text, /No policy-visible recent sessions were returned/);
  assert.match(text, /No policy-visible recent context runs were returned/);
  assert.match(text, /No policy-visible recent Capture work was returned/);
  assert.match(text, /Latest response loaded \d{4}-\d{2}-\d{2} \d{2}:\d{2} UTC/);
  assert.match(text, /can already be stale/);
  assert.doesNotMatch(text, /nothing happened|no sessions exist|tenant total/i);
});

test("one failed section does not hide the other authorised activity", async () => {
  await seed(plan.sessions.key, ok({ sessions: [session()], next_cursor: null }));
  await seed(plan.contextRuns.key, {
    kind: "unavailable",
    message: "temporary context dependency failure",
  });
  await seed(plan.captureBatches.key, ok({ batches: [captureBatch()], next_cursor: null }));

  const { markup, text } = render();
  assert.match(text, /claude-code/);
  assert.match(text, /temporary context dependency failure/);
  assert.match(text, /2 frozen events · 3 attempts/);
  assert.match(text, /Safe failure code: dependency_unavailable/);
  assert.match(markup, /href="\/console\/sessions\/session-1"/);
});

test("degraded context is visible without content, identifiers or zeroed list fields", async () => {
  await seed(plan.sessions.key, ok({ sessions: [session()], next_cursor: null }));
  await seed(plan.contextRuns.key, ok({ runs: [contextRun()], next_cursor: null }));
  await seed(plan.captureBatches.key, ok({ batches: [captureBatch()], next_cursor: null }));

  const { markup, text } = render();
  assert.match(text, /Context composition completed degraded/);
  assert.match(text, /affected legs: embedder, another recorded leg/);
  assert.match(markup, /href="\/console\/context-runs\/context-run-1"/);
  for (const privateValue of [
    "PRIVATE-PRINCIPAL",
    "PRIVATE-INSTALLATION",
    "PRIVATE-EXTERNAL-SESSION",
    "PRIVATE-SESSION-CONTENT",
    "PRIVATE-END-REASON",
    "PRIVATE-SESSION-METADATA",
    "PRIVATE-REPOSITORY",
    "PRIVATE-AGENT",
    "PRIVATE-MODEL",
    "PRIVATE-BRANCH",
    "PRIVATE-CONTEXT-SESSION",
    "PRIVATE-CONTEXT-QUERY",
    "PRIVATE-RENDERED-CONTEXT",
    "PRIVATE-SKILL-CONTENT",
    "PRIVATE-BLOCK-HASH",
    "PRIVATE-CONFIGURATION-HASH",
    "PRIVATE-RETRIEVAL-VERSION",
    "PRIVATE-INDEX-VERSION",
    "PRIVATE-EMBEDDING-MODEL",
    "PRIVATE-DEGRADED-DETAIL",
    "PRIVATE-CAPTURE-SESSION",
    "PRIVATE-INPUT-HASH",
    "PRIVATE-CAPTURE-CONFIGURATION",
    "PRIVATE-CAPTURE-CONFIGURATION-VERSION",
    "PRIVATE-EXTRACTOR",
    "PRIVATE-CAPTURE-MODEL",
    "99001",
    "99002",
    "99003",
    "99004",
    "99005",
    "99006",
  ]) {
    assert.ok(!markup.includes(privateValue), `${privateValue} was rendered`);
  }
});

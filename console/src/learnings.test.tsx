/**
 * Reader-visible acceptance for New Learnings (CPR-19).
 *
 * The cache is primed with generated-contract responses and the real page is
 * rendered to static markup. These assertions pin facts and offered actions,
 * not layout: candidate/published separation, evidence, comparisons, scope
 * denial, VedaFlow outcomes and the resulting Knowledge address.
 */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { Learnings } from "./Learnings.js";
import { AppProvider, appContext } from "./Shell.js";
import {
  EMPTY_LEARNINGS_FILTERS,
  batchQuery,
  candidateQuery,
} from "./learnings.mjs";
import { reconcile } from "./selection.mjs";
import { toText } from "./text.mjs";
import type {
  CaptureBatchView,
  CaptureCandidateView,
  KnowledgeItemView,
  MeView,
  ProjectView,
  TimelineView,
  WorkspaceView,
} from "./generated/api.js";

const PROJECT_SCOPE = "scope-project";
const WORKSPACE_SCOPE = "scope-workspace";
const PRINCIPAL_SCOPE = "scope-principal";

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

function me(
  options: {
    projectWrite?: boolean;
    workspaceWrite?: boolean;
    sessionWrite?: boolean;
    diagnostics?: boolean;
  } = {},
): MeView {
  return {
    principal: { subject: "alice@example.test", display_name: "Alice", quarantined: false },
    tenant: { id: "tenant-1", slug: "acme", name: "ACME", status: "active" },
    onboarding: { state: "ready", workspace_count: 1, project_count: 1 },
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
        actions: { "knowledge.write": true },
      },
      {
        scope_id: PROJECT_SCOPE,
        kind: "project",
        source: "selected_project",
        direct: true,
        roles: ["member"],
        actions: {
          "session.read": true,
          "session.write": options.sessionWrite ?? true,
          "session.diagnostics": options.diagnostics ?? false,
          "knowledge.write": options.projectWrite ?? true,
        },
      },
      {
        scope_id: WORKSPACE_SCOPE,
        kind: "workspace",
        source: "selected_workspace",
        direct: false,
        roles: ["member"],
        actions: { "knowledge.write": options.workspaceWrite ?? true },
      },
    ],
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
    candidate_count: 1,
    configuration_hash: "configuration-hash",
    configuration_version_id: "configuration-version-1",
    created_at: "2026-08-24T10:00:00Z",
    completed_at: "2026-08-24T10:00:01Z",
    ...overrides,
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
    content_hash: "candidate-hash",
    state: "pending",
    source_event_ids: ["event-1"],
    source_artifact_ids: [],
    matches: [
      {
        knowledge_item_id: "knowledge-old",
        knowledge_revision_id: "revision-old",
        kind: "supersession",
        similarity_permille: 860,
        reason_code: "shared_terms_with_polarity_change",
      },
    ],
    content_erased: false,
    created_at: "2026-08-24T10:00:01Z",
    ...overrides,
  };
}

function existing(): KnowledgeItemView {
  return {
    id: "knowledge-old",
    scope_id: PROJECT_SCOPE,
    project_id: "project-1",
    knowledge_type: "convention",
    origin: "observed",
    lifecycle_state: "active",
    current_revision: {
      id: "revision-old",
      knowledge_item_id: "knowledge-old",
      revision_number: 1,
      title: "Legacy request correlation",
      summary: "Public requests use X-Request-Id.",
      body_markdown: "Public requests use `X-Request-Id`.",
      tags: ["http"],
      sensitivity: "internal",
      confidence_permille: 900,
      stale: false,
      valid_from: "2026-08-20T00:00:00Z",
      transaction_time: "2026-08-20T00:00:00Z",
      content_hash: "old-hash",
      metadata: {},
      verification_metadata: {},
    },
    created_at: "2026-08-20T00:00:00Z",
    updated_at: "2026-08-20T00:00:00Z",
  } as KnowledgeItemView;
}

function timeline(): TimelineView {
  return {
    session_id: "session-1",
    event_counts: { "message.user": 1 },
    truncated: false,
    entries: [
      {
        id: "event-1",
        kind: "event",
        event_type: "message.user",
        sequence: 1,
        at: "2026-08-24T09:59:00Z",
        received_at: "2026-08-24T09:59:01Z",
        delayed: false,
        summary: "message.user (43 characters)",
      },
    ],
  };
}

const filters = { ...EMPTY_LEARNINGS_FILTERS, projectId: "project-1" };
const batchKey = `learnings/batches/${JSON.stringify(batchQuery(filters, null))}`;
const candidateKey = `learnings/candidates/${JSON.stringify(candidateQuery(filters, null))}`;

async function seed(key: string, body: unknown): Promise<void> {
  const outcome: Outcome = { kind: "ok", body };
  await cache.ensure(key, async () => outcome);
}

async function seedPage(
  rows: CaptureCandidateView[],
  sourceBatch: CaptureBatchView = batch({ candidate_count: rows.length }),
): Promise<void> {
  await seed(batchKey, { batches: [sourceBatch], next_cursor: null });
  await seed(candidateKey, { candidates: rows, next_cursor: null });
  if (sourceBatch.session_id) await seed(`sessions/timeline/${sourceBatch.session_id}`, timeline());
  for (const row of rows) {
    for (const match of row.matches) {
      await seed(`knowledge/item/${match.knowledge_item_id}`, existing());
    }
  }
}

function render(view: MeView): { markup: string; text: string } {
  const selection = reconcile({ workspaceId: "workspace-1", projectId: "project-1" }, view);
  const markup = renderToStaticMarkup(
    <AppProvider value={appContext(view, selection, () => {})}>
      <Learnings />
    </AppProvider>,
  );
  return { markup, text: toText(markup) };
}

beforeEach(() => cache.clear());

test("the primary review page groups a batch and exposes evidence, comparisons and every action", async () => {
  await seedPage([candidate()]);
  const { text, markup } = render(me({ diagnostics: true }));

  for (const expected of [
    "PulseBoard",
    "0 reviewed · 1 pending · 1 total",
    "Request correlation",
    "Shared with the project",
    "Possible replacement",
    "Legacy request correlation",
    "Source conversation preview",
    "message.user (43 characters)",
    "Show authorised source payload",
    "Accept",
    "Edit and accept",
    "Merge with existing",
    "Replace existing",
    "Dismiss",
    "Private to me",
    "Shared with project · PulseBoard",
    "Shared at workspace · Payments",
  ]) {
    assert.ok(text.includes(expected), `missing reader-visible fact/action ${JSON.stringify(expected)}`);
  }
  assert.ok(markup.includes('href="/console/sessions/session-1"'));
  assert.match(
    text,
    /suggestions extracted from sessions, not active Knowledge/,
    "the trust boundary is stated before any decision control",
  );
});

test("an OKF import is review input with artifact provenance, not a fictional session", async () => {
  await seedPage(
    [
      candidate({
        source_kind: "okf_import",
        session_id: undefined,
        import_job_id: "import-1",
        source_event_ids: [],
        source_artifact_ids: ["artifact-1"],
        matches: [],
      }),
    ],
    batch({
      source_kind: "okf_import",
      session_id: undefined,
      import_job_id: "import-1",
      event_count: 1,
      candidate_count: 1,
    }),
  );
  const { text, markup } = render(me());

  assert.match(text, /OKF import import-1/);
  assert.match(text, /1 immutable source artifact/);
  assert.match(text, /remains review input until this candidate is accepted/);
  assert.match(text, /Decide this learning/);
  assert.ok(!markup.includes("/console/sessions/"));
});

test("a denied destination is absent and the page explains the required scope change", async () => {
  await seedPage([candidate()]);
  const { text, markup } = render(me({ projectWrite: false, workspaceWrite: false }));

  assert.match(text, /proposed scope is readable but not publishable/);
  assert.match(text, /Private to me/);
  assert.match(text, /Change scope and accept/);
  assert.ok(!markup.includes("Shared with project · PulseBoard"));
  assert.ok(!markup.includes("Shared at workspace · Payments"));
});

test("pending-review acceptance links Advanced Reviews and never claims publication", async () => {
  await seedPage([
    candidate({
      state: "accepted",
      resulting_change_id: "change-pending",
      resulting_outcome: "pending_review",
      resulting_knowledge_item_id: "knowledge-not-published",
      resulting_revision_id: undefined,
    }),
  ]);
  const { text, markup } = render(me());

  assert.match(text, /waiting in Advanced Reviews and not active Knowledge yet/);
  assert.ok(markup.includes('href="/console/advanced/reviews"'));
  assert.doesNotMatch(text, /Open resulting Knowledge/);
  assert.ok(!markup.includes("/console/knowledge/knowledge-not-published"));
});

test("an applied replacement links the resulting Knowledge while retaining supersession wording", async () => {
  await seedPage([
    candidate({
      state: "replaced",
      resulting_change_id: "change-applied",
      resulting_outcome: "applied",
      resulting_knowledge_item_id: "knowledge-new",
      resulting_revision_id: "revision-new",
    }),
  ]);
  const { text, markup } = render(me());

  assert.match(text, /replaced and published through VedaFlow change change-applied/);
  assert.match(text, /Open resulting Knowledge/);
  assert.ok(markup.includes('href="/console/knowledge/knowledge-new"'));
  assert.doesNotMatch(text, /Decide this learning/);
});

test("a read-only session shows candidates without controls and names the missing authority", async () => {
  await seedPage([candidate()]);
  const { text } = render(me({ sessionWrite: false }));

  assert.match(text, /does not currently offer session\.write/);
  assert.match(text, /Request correlation/);
  assert.doesNotMatch(text, /Decide this learning/);
});

test("dismissed candidates state that no Knowledge was published", async () => {
  await seedPage([
    candidate({
      state: "dismissed",
      decision_reason: "incidental detail",
      decided_at: "2026-08-24T10:02:00Z",
    }),
  ]);
  const { text } = render(me());
  assert.match(text, /Dismissed\. No Knowledge was published\./);
  assert.doesNotMatch(text, /Decide this learning/);
});

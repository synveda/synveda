/** Reader-visible acceptance for Context Inspector retention and disclosure (CPR-21). */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { ContextInspector } from "./Context.js";
import { toText } from "./text.mjs";
import type {
  ContextCandidateView,
  ContextRunDetailView,
  ContextRunView,
  ContextSelectionView,
  KnowledgeRevisionView,
  KnowledgeSourceView,
} from "./generated/api.js";

const RUN_ID = "context-run-1";
const CACHE_KEY = `context-runs/${RUN_ID}`;

function revision(overrides: Partial<KnowledgeRevisionView> = {}): KnowledgeRevisionView {
  return {
    id: "revision-current",
    knowledge_item_id: "knowledge-current",
    revision_number: 3,
    title: "Current request correlation",
    summary: "Public requests use traceparent.",
    body_markdown: "Public requests use `traceparent`.",
    tags: ["http"],
    sensitivity: "internal",
    confidence_permille: 960,
    stale: false,
    valid_from: "2026-08-24T09:00:00Z",
    transaction_time: "2026-08-24T09:01:00Z",
    content_hash: "hash-current",
    metadata: {},
    verification_metadata: { method: "team-review" },
    ...overrides,
  };
}

function source(): KnowledgeSourceView {
  return {
    id: "source-1",
    source_type: "session_event",
    scope_id: "scope-project",
    session_event_id: "event-17",
    content_hash: "source-hash",
    metadata: {},
    created_at: "2026-08-24T09:02:00Z",
  };
}

function run(overrides: Partial<ContextRunView> = {}): ContextRunView {
  return {
    id: RUN_ID,
    session_id: "session-1",
    workspace_id: "workspace-1",
    project_id: "project-1",
    scope_id: "scope-project",
    query: "Which correlation header does PulseBoard use?",
    query_hash: "query-hash",
    rendered: "# Synveda Knowledge\nPublic requests use traceparent.",
    block_hash: "rendered-hash",
    tokens: 37,
    budget_tokens: 256,
    requested_budget_tokens: 300,
    entry_count: 1,
    candidate_count: 3,
    selection_count: 1,
    skills: {},
    degraded: ["embedder"],
    as_of: "2026-08-24T10:00:00Z",
    retrieval_version: "knowledge-planner-v1",
    embedding_model: "bge-small-en-v1.5",
    index_version: "knowledge-search-v1",
    graph_version: undefined,
    trace_retention_mode: "full",
    completion_status: "completed",
    created_at: "2026-08-24T10:00:01Z",
    ...overrides,
  };
}

function selectedCandidate(): ContextCandidateView {
  return {
    id: "candidate-current",
    ordinal: 0,
    knowledge_item_id: "knowledge-current",
    knowledge_revision_id: "revision-current",
    content_hash: "hash-current",
    lifecycle_state: "active",
    reason_codes: ["keyword_match", "freshness_boost"],
    scores: {
      keyword_micros: 420_000,
      semantic_micros: 310_000,
      freshness_micros: 80_000,
      pin_micros: 0,
      current_state_micros: 100_000,
      final_micros: 910_000,
    },
    revision: revision(),
    sources: [source()],
  };
}

function excluded(
  id: string,
  title: string,
  reason: string,
  lifecycle = "active",
): ContextCandidateView {
  return {
    id,
    ordinal: id === "candidate-old" ? 1 : 2,
    knowledge_item_id: `knowledge-${id}`,
    knowledge_revision_id: `revision-${id}`,
    content_hash: `hash-${id}`,
    lifecycle_state: lifecycle,
    reason_codes: [reason],
    exclusion_reason: reason,
    scores: {
      keyword_micros: 250_000,
      semantic_micros: 0,
      freshness_micros: 0,
      pin_micros: 0,
      current_state_micros: lifecycle === "superseded" ? -1_000_000 : 100_000,
      final_micros: 10_000,
    },
    revision: revision({
      id: `revision-${id}`,
      knowledge_item_id: `knowledge-${id}`,
      title,
      content_hash: `hash-${id}`,
    }),
    sources: [],
  };
}

function selection(overrides: Partial<ContextSelectionView> = {}): ContextSelectionView {
  return {
    id: "selection-1",
    rank: 1,
    knowledge_item_id: "knowledge-current",
    knowledge_revision_id: "revision-current",
    content_hash: "hash-current",
    token_count: 31,
    reason_codes: ["keyword_match", "freshness_boost"],
    revision: revision(),
    sources: [source()],
    ...overrides,
  };
}

function detail(overrides: Partial<ContextRunDetailView> = {}): ContextRunDetailView {
  return {
    run: run(),
    candidates: [
      selectedCandidate(),
      excluded("candidate-old", "Obsolete X-Request-Id convention", "superseded", "superseded"),
      excluded("candidate-budget", "Long incident appendix", "token_budget"),
    ],
    selections: [selection()],
    feedback: [
      {
        id: "feedback-1",
        context_selection_id: "selection-1",
        knowledge_revision_id: "revision-current",
        feedback_type: "referenced_by_agent",
        principal_id: "bob@example.test",
        created_at: "2026-08-24T10:02:00Z",
      },
    ],
    ...overrides,
  };
}

async function seed(outcome: Outcome): Promise<void> {
  await cache.ensure(CACHE_KEY, async () => outcome);
}

function render(): { markup: string; text: string } {
  const markup = renderToStaticMarkup(<ContextInspector contextRunId={RUN_ID} />);
  return { markup, text: toText(markup) };
}

beforeEach(() => cache.clear());

test("a full trace explains selection, evidence, scores, exclusions, versions and exact feedback", async () => {
  await seed({ kind: "ok", body: detail() });
  const { markup, text } = render();

  for (const expected of [
    "Which correlation header does PulseBoard use?",
    "Current request correlation",
    "Public requests use `traceparent`.",
    "current at planning time",
    "keyword match",
    "freshness boost",
    "Rank 1 · 31 tokens",
    "Keyword",
    "42%",
    "Embedding",
    "Source evidence",
    "session event event-17",
    "300 requested · 256 governed · 37 used",
    "knowledge-planner-v1",
    "knowledge-search-v1",
    "bge-small-en-v1.5",
    "Graph",
    "not run",
    "rendered-hash",
    "Degraded retrieval: embedder",
    "Obsolete X-Request-Id convention",
    "superseded at planning time",
    "Long incident appendix",
    "Excluded because token budget",
    "Referenced by agent",
    "Selection alone records no positive outcome",
    "Caused correction",
  ]) {
    assert.ok(text.includes(expected), `missing inspector fact ${JSON.stringify(expected)}`);
  }
  assert.ok(markup.includes('href="/console/sessions/session-1"'));
  assert.ok(markup.includes('href="/console/knowledge/knowledge-current"'));
});

test("redacted mode keeps exact reasons and feedback targets without task or Knowledge content", async () => {
  const redactedSelection = selection({ revision: undefined, sources: [] });
  const redactedCandidate = {
    ...selectedCandidate(),
    revision: undefined,
    scores: undefined,
    sources: [],
  };
  await seed({
    kind: "ok",
    body: detail({
      run: run({ query: undefined, rendered: undefined, trace_retention_mode: "redacted" }),
      candidates: [redactedCandidate],
      selections: [redactedSelection],
      feedback: [],
    }),
  });
  const { markup, text } = render();
  assert.match(text, /Redacted trace/);
  assert.match(text, /original task is unavailable/);
  assert.match(text, /Knowledge content was not retained in this redacted trace/);
  assert.match(text, /Referenced by agent/);
  assert.ok(markup.includes('href="/console/knowledge/knowledge-current"'));
  assert.doesNotMatch(text, /traceparent/);
  assert.doesNotMatch(text, /session event event-17/);
});

test("hashes-only mode displays hashes and reasons but no address, content, source or feedback control", async () => {
  const hashSelection = selection({
    knowledge_item_id: undefined,
    knowledge_revision_id: undefined,
    revision: undefined,
    sources: [],
  });
  const hashCandidate: ContextCandidateView = {
    ...selectedCandidate(),
    knowledge_item_id: undefined,
    knowledge_revision_id: undefined,
    lifecycle_state: undefined,
    revision: undefined,
    scores: undefined,
    sources: [],
  };
  await seed({
    kind: "ok",
    body: detail({
      run: run({ query: undefined, rendered: undefined, trace_retention_mode: "hashes_only" }),
      candidates: [hashCandidate],
      selections: [hashSelection],
      feedback: [],
    }),
  });
  const { markup, text } = render();
  assert.match(text, /Hashes-only trace/);
  assert.match(text, /Content hash-current/);
  assert.match(text, /feedback is unavailable because this trace retained no exact revision address/i);
  assert.ok(!markup.includes("/console/knowledge/knowledge-current"));
  assert.doesNotMatch(text, /Current request correlation/);
  assert.doesNotMatch(text, /traceparent/);
  assert.doesNotMatch(text, /session event event-17/);
  assert.doesNotMatch(text, /Caused correction/);
});

test("disabled mode says detail was not retained and never claims nothing was selected", async () => {
  await seed({
    kind: "ok",
    body: detail({
      run: run({
        query: undefined,
        rendered: undefined,
        trace_retention_mode: "disabled",
        candidate_count: 0,
        selection_count: 0,
        entry_count: 0,
      }),
      candidates: [],
      selections: [],
      feedback: [],
    }),
  });
  const { text } = render();
  assert.match(text, /Trace retention was disabled/);
  assert.match(text, /This is not a claim that the delivery selected nothing/);
  assert.match(text, /Candidate exclusions were not retained/);
  assert.doesNotMatch(text, /No policy-visible Knowledge revision is available/);
});

test("a denied context run renders only the refusal supplied by the gateway", async () => {
  await seed({ kind: "forbidden", message: "policy denied session.read on this context run" });
  const { markup, text } = render();
  assert.match(text, /Your roles do not allow this/);
  assert.match(text, /session\.read/);
  for (const secret of ["traceparent", "X-Request-Id", "knowledge-current", "event-17", "rendered-hash"]) {
    assert.ok(!text.includes(secret), `denied page leaked ${secret}`);
    assert.ok(!markup.includes(secret), `denied markup leaked ${secret}`);
  }
});

test("an aggregate policy exclusion carries no hidden candidate address, reason or count", async () => {
  await seed({
    kind: "ok",
    body: detail({
      policy_exclusion_message: "Some context detail is unavailable under current policy.",
      candidates: [],
      selections: [],
      feedback: [],
      run: run({ candidate_count: 0, selection_count: 0, entry_count: 0, rendered: undefined }),
    }),
  });
  const { text } = render();
  assert.match(text, /Some context detail is unavailable under current policy/);
  assert.match(text, /No hidden candidate address, title, reason or count is shown/);
  for (const hidden of ["hidden-item", "hidden-title", "token_budget", "denied: 1"]) {
    assert.doesNotMatch(text, new RegExp(hidden));
  }
});

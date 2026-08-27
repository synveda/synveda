/** Pure Context Inspector rules and its exact feedback wire address (CPR-21). */

import assert from "node:assert/strict";
import { test } from "node:test";

import { describe } from "./client.mjs";
import {
  FEEDBACK_TYPES,
  candidateForSelection,
  canGiveFeedback,
  excludedCandidates,
  feedbackBody,
  feedbackLabel,
  retentionDescription,
  scorePercent,
  selectionState,
} from "./context.mjs";
import type {
  ContextCandidateView,
  ContextSelectionView,
  KnowledgeRevisionView,
} from "./generated/api.js";

function selection(overrides: Partial<ContextSelectionView> = {}): ContextSelectionView {
  return {
    id: "selection-1",
    context_candidate_id: "candidate-1",
    rank: 1,
    channel: "current_knowledge",
    knowledge_item_id: "knowledge-1",
    knowledge_revision_id: "revision-7",
    content_hash: "hash-current",
    token_count: 31,
    reason_codes: ["keyword_match"],
    ...overrides,
  };
}

function candidate(overrides: Partial<ContextCandidateView> = {}): ContextCandidateView {
  return {
    id: "candidate-1",
    ordinal: 0,
    channel: "current_knowledge",
    knowledge_item_id: "knowledge-1",
    knowledge_revision_id: "revision-7",
    content_hash: "hash-current",
    lifecycle_state: "active",
    reason_codes: ["keyword_match"],
    ...overrides,
  };
}

test("feedback names one context selection and its exact immutable revision", () => {
  for (const feedbackType of FEEDBACK_TYPES) {
    const body = feedbackBody(selection(), feedbackType);
    assert.deepEqual(body, {
      context_selection_id: "selection-1",
      knowledge_revision_id: "revision-7",
      feedback_type: feedbackType,
    });
    const request = describe("create_context_feedback", {
      path: { id: "run-3" },
      body,
      idempotencyKey: "feedback-attempt-1",
    });
    assert.equal(request.path, "/context-runs/run-3/feedback");
    assert.equal(request.init.method, "POST");
    assert.equal(
      (request.init.headers as Record<string, string>)["idempotency-key"],
      "feedback-attempt-1",
    );
    assert.deepEqual(JSON.parse(request.init.body as string), body);
    assert.ok(feedbackLabel(feedbackType).length > 0);
  }
});

test("hashes-only selections cannot manufacture a feedback target", () => {
  const retained = selection({ knowledge_item_id: undefined, knowledge_revision_id: undefined });
  assert.equal(canGiveFeedback(retained), false);
  assert.equal(feedbackBody(retained, "helpful"), null);
});

test("unreviewed candidates match by capture address and cannot receive revision feedback", () => {
  const retained = selection({
    channel: "unreviewed_candidates",
    capture_candidate_id: "capture-9",
    knowledge_item_id: undefined,
    knowledge_revision_id: undefined,
  });
  const exact = candidate({
    channel: "unreviewed_candidates",
    capture_candidate_id: "capture-9",
    knowledge_item_id: undefined,
    knowledge_revision_id: undefined,
  });
  assert.equal(candidateForSelection([exact], retained)?.id, exact.id);
  assert.equal(selectionState(exact, null), "unreviewed at planning time");
  assert.equal(canGiveFeedback(retained), false);
  assert.equal(feedbackBody(retained, "helpful"), null);
});

test("a selection matches its exact revision before a same-content fallback", () => {
  const sameHash = candidate({ id: "same-hash", knowledge_revision_id: "revision-other" });
  const exact = candidate({ id: "exact", content_hash: "another-hash" });
  assert.equal(candidateForSelection([sameHash, exact], selection())?.id, "exact");
  assert.equal(
    candidateForSelection([sameHash], selection({ knowledge_revision_id: undefined }))?.id,
    "same-hash",
  );
});

test("current, stale and superseded are never collapsed into one label", () => {
  const revision = { stale: false } as KnowledgeRevisionView;
  assert.equal(selectionState(candidate(), revision), "current at planning time");
  assert.equal(
    selectionState(candidate({ lifecycle_state: "superseded" }), revision),
    "superseded at planning time",
  );
  assert.equal(selectionState(candidate(), { stale: true } as KnowledgeRevisionView), "stale at planning time");
});

test("token-budget and supersession exclusions stay explicit and visible", () => {
  const excluded = excludedCandidates([
    candidate(),
    candidate({ id: "budget", exclusion_reason: "token_budget" }),
    candidate({ id: "old", exclusion_reason: "superseded" }),
  ]);
  assert.deepEqual(excluded.map((entry) => entry.exclusion_reason), ["token_budget", "superseded"]);
});

test("every trace-retention mode explains what its absence means", () => {
  for (const mode of ["full", "redacted", "hashes_only", "disabled"]) {
    assert.match(retentionDescription(mode), new RegExp(mode === "hashes_only" ? "Hashes-only" : mode, "i"));
  }
  assert.match(retentionDescription("disabled"), /only the immutable delivery envelope/);
  assert.match(retentionDescription("redacted"), /were not retained/);
});

test("per-million score contributions render as percentages", () => {
  assert.equal(scorePercent(420_000), "42%");
  assert.equal(scorePercent(12_500), "1.25%");
  assert.equal(scorePercent(-5_000), "-0.5%");
});

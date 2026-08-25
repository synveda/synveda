/** Reader-visible CPR-37 conflict, transition and freshness evidence. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { ConflictReview, KnowledgeRow } from "./Knowledge.js";
import { toText } from "./text.mjs";
import type {
  ConflictMemberView,
  ConflictSetView,
  KnowledgeItemView,
  KnowledgeRevisionView,
} from "./generated/api.js";

function revision(id: string, title: string, body: string): KnowledgeRevisionView {
  return {
    id,
    knowledge_item_id: `item-${id}`,
    revision_number: 1,
    title,
    body_markdown: body,
    summary: body,
    tags: ["pulseboard"],
    sensitivity: "internal",
    confidence_permille: 940,
    valid_from: "2026-08-25T10:00:00Z",
    stale: false,
    freshness_reasons: [],
    verification_metadata: {},
    content_hash: "a".repeat(64),
    metadata: {},
    transaction_time: "2026-08-25T10:00:00Z",
  };
}

function member(
  role: "challenger" | "current",
  exact: KnowledgeRevisionView,
): ConflictMemberView {
  return {
    id: `member-${role}`,
    role,
    knowledge_item_id: exact.knowledge_item_id,
    knowledge_revision: exact,
    classification: "contradiction",
    similarity_permille: 712,
    reason_code: "shared_subject_opposite_polarity",
  };
}

function conflict(): ConflictSetView {
  return {
    id: "conflict-1",
    scope_id: "scope-project",
    project_id: "project-1",
    classification: "contradiction",
    status: "open",
    revision: 3,
    members: [
      member(
        "challenger",
        revision("revision-new", "Trace context", "Public requests use traceparent."),
      ),
      member(
        "current",
        revision("revision-old", "Request ID", "Public requests use X-Request-Id."),
      ),
    ],
    created_at: "2026-08-25T10:01:00Z",
    updated_at: "2026-08-25T10:01:00Z",
  };
}

test("conflict review compares exact revisions and offers governed future resolution", () => {
  const markup = renderToStaticMarkup(<ConflictReview conflict={conflict()} />);
  const text = toText(markup);
  assert.match(text, /Conflict review|contradiction/i);
  assert.match(text, /Public requests use traceparent/);
  assert.match(text, /Public requests use X-Request-Id/);
  assert.match(text, /shared subject opposite polarity/);
  assert.match(text, /Resolve through VedaFlow/);
  for (const resolution of ["keep_separate", "support", "duplicate", "supersede", "transition", "archive"]) {
    assert.ok(markup.includes(`value="${resolution}"`), resolution);
  }
  assert.ok(!markup.includes("expected_revision"), "precondition stays generated request state");
});

test("capture challengers remain in New Learnings instead of a second publication path", () => {
  const value = conflict();
  value.members[0] = {
    ...value.members[0],
    knowledge_item_id: undefined,
    knowledge_revision: undefined,
    capture_candidate_id: "candidate-1",
  };
  const markup = renderToStaticMarkup(<ConflictReview conflict={value} />);
  assert.match(toText(markup), /still a capture candidate.*New Learnings/i);
  assert.ok(markup.includes('href="/console/learnings"'));
  assert.ok(!markup.includes("Resolve through VedaFlow"));
});

test("stale rows name the exact effective freshness reasons", () => {
  const exact = revision("revision-stale", "Webhook procedure", "Retry by provider event ID.");
  exact.stale = true;
  exact.freshness_reasons = ["configured_interval", "failed_use"];
  const item = {
    id: exact.knowledge_item_id,
    scope_id: "scope-project",
    project_id: "project-1",
    knowledge_type: "procedure",
    origin: "authored",
    lifecycle_state: "active",
    current_revision: exact,
    created_at: "2026-08-25T10:00:00Z",
    updated_at: "2026-08-25T10:00:00Z",
  } as KnowledgeItemView;
  const markup = renderToStaticMarkup(<KnowledgeRow item={item} />);
  assert.match(toText(markup), /verification due/i);
});

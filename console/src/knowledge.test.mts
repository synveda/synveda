import assert from "node:assert/strict";
import { test } from "node:test";

import {
  EMPTY_KNOWLEDGE_FILTERS,
  knowledgeIsFiltered,
  knowledgeQuery,
  mutationMessage,
  visibilityLabel,
} from "./knowledge.mjs";
import type { KnowledgeItemView } from "./generated/api.js";

test("Knowledge filters map exactly to the public collection contract", () => {
  const filters = {
    ...EMPTY_KNOWLEDGE_FILTERS,
    query: "  webhook retry  ",
    projectId: "project-1",
    lifecycle: "stale",
    source: "repository",
    stale: "true" as const,
    updatedFrom: "2026-08-24T10:30",
    asOf: "2026-08-25T12:00",
    asKnownAt: "2026-08-25T11:00",
    includeHistory: true,
    includeTransitional: true,
  };
  assert.deepEqual(knowledgeQuery(filters, "next"), {
    query: "webhook retry",
    workspace_id: undefined,
    project_id: "project-1",
    scope_id: undefined,
    owner: undefined,
    knowledge_type: undefined,
    origin: undefined,
    lifecycle_state: "stale",
    tag: undefined,
    source: "repository",
    updated_from: new Date("2026-08-24T10:30").toISOString(),
    updated_before: undefined,
    stale: "true",
    as_of: new Date("2026-08-25T12:00").toISOString(),
    as_known_at: new Date("2026-08-25T11:00").toISOString(),
    include_history: "true",
    include_transitional: "true",
    cursor: "next",
    limit: "50",
  });
  assert.ok(knowledgeIsFiltered(filters));
  assert.ok(!knowledgeIsFiltered(EMPTY_KNOWLEDGE_FILTERS));
});

test("scope and governance outcomes stay explicit", () => {
  const item = {
    owner_principal_id: "alice@example.test",
  } as KnowledgeItemView;
  assert.equal(visibilityLabel(item), "Private to alice@example.test");
  assert.match(
    mutationMessage({ change_id: "change-1", outcome: "pending_review" }),
    /Waiting for review.*change-1/,
  );
  assert.match(
    mutationMessage({ change_id: "change-2", outcome: "rejected" }),
    /no Knowledge state changed/,
  );
});

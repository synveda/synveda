/** CPR-30 console acceptance for generated Configuration operations. */

import assert from "node:assert/strict";
import { test } from "node:test";

import { describe } from "./client.mjs";
import {
  configurationSummary,
  configurationTarget,
  mutationMessage,
  parseConfiguration,
  renderConfiguration,
} from "./configuration.mjs";
import type { ConfigurationDocumentBody, MeView } from "./generated/api.js";

const document: ConfigurationDocumentBody = {
  policy_pack: "standard",
  capture: {
    enabled: true,
    explicit_request: true,
    on_session_end: true,
    maximum_candidates_per_batch: 24,
    minimum_confidence_permille: 600,
  },
  context: {
    token_budget: 1500,
    channels: ["current_knowledge"],
    trace_retention: "redacted",
  },
  freshness: {
    fact_days: 30,
    decision_days: 0,
    preference_days: 0,
    procedure_days: 90,
    entity_days: 60,
    episode_days: 0,
    convention_days: 30,
    warning_days: 14,
    reference_days: 30,
  },
  advertisement: { skills: true, tools: false },
  allowed_external_providers: ["anthropic", "remote_mcp"],
};

test("the complete immutable document round-trips without a second DTO", () => {
  assert.deepEqual(parseConfiguration(renderConfiguration(document)), document);
  assert.match(configurationSummary(document), /standard · 1500 tokens · redacted traces/);
  assert.throws(() => parseConfiguration("[]"), /complete JSON object/);
});

test("the selected project is the nearest configuration target", () => {
  const me = {
    anchors: [{ source: "principal_scope", scope_id: "personal-scope" }],
  } as unknown as MeView;
  assert.deepEqual(
    configurationTarget(
      me,
      { id: "w", scope_id: "workspace-scope", slug: "pulseboard" } as never,
      { id: "p", scope_id: "project-scope", slug: "api" } as never,
    ),
    { id: "project-scope", label: "project api" },
  );
  assert.deepEqual(configurationTarget(me, null, null), {
    id: "personal-scope",
    label: "your private scope",
  });
});

test("mutation outcomes never imply a pending change is effective", () => {
  assert.match(
    mutationMessage({ change_id: "change-1", outcome: "pending_review" }),
    /runtime selection is unchanged/,
  );
  assert.match(mutationMessage({ change_id: "change-2", outcome: "applied" }), /VedaFlow/);
});

test("generated operations carry idempotency keys and revision preconditions", () => {
  const publish = describe("publish_configuration_version", {
    path: { id: "configuration-1" },
    idempotencyKey: "publish-1",
    body: { expected_current_version_id: "version-1", document },
  });
  assert.equal(publish.path, "/configurations/configuration-1/versions");
  assert.equal(publish.init.method, "POST");
  assert.equal((publish.init.headers as Record<string, string>)["idempotency-key"], "publish-1");
  assert.match(String(publish.init.body), /expected_current_version_id/);

  const binding = describe("update_configuration_binding", {
    path: { id: "binding-1" },
    idempotencyKey: "binding-2",
    body: {
      expected_revision: 7,
      artifact_id: "configuration-1",
      pinned_version_id: "version-1",
      enabled: true,
      reason: "rollback",
    },
  });
  assert.match(String(binding.init.body), /"expected_revision":7/);
});

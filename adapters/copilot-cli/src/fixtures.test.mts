/** ADPT-9: authentic native evidence stays distinct from public-API conformance. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const capture = JSON.parse(readFileSync(new URL("../fixtures/lifecycle.json", import.meta.url), "utf8"));
const transcript = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
const events = transcript.trim().split("\n").map((line) => JSON.parse(line));
const invocation = capture.invocations[0];

test("native fixture bytes are pinned and omit private paths and hidden model content", () => {
  const registry = JSON.parse(readFileSync(new URL("../../registry.json", import.meta.url), "utf8"));
  const client = registry.clients.find((entry: { id: string }) => entry.id === "copilot-cli");
  assert.equal(client.support_level, "experimental");
  assert.deepEqual(client.tested_versions, ["1.0.83"]);
  for (const name of ["lifecycle.json", "transcript.jsonl"]) {
    const file = readFileSync(new URL(`../fixtures/${name}`, import.meta.url));
    const digest = createHash("sha256").update(file).digest("hex");
    assert.equal(client.authentic_fixtures.find((entry: { path: string }) =>
      entry.path === `adapters/copilot-cli/fixtures/${name}`).sha256, digest);
    assert.doesNotMatch(file.toString("utf8"), /\/Users\/|\/private\/|"(?:access_token|refresh_token|reasoningText|reasoningOpaque|reasoningBlocks|encryptedContent)"/);
  }
});

test("native new-session order and accepted hook output do not turn a failed marker into a pass", () => {
  assert.equal(capture.provenance.kind, "captured-real-client");
  assert.equal(capture.client.version, "1.0.83");
  assert.deepEqual(invocation.frames.map((frame: { event: string }) => frame.event),
    ["userPromptSubmitted", "sessionStart", "preToolUse", "postToolUse", "agentStop", "sessionEnd"]);
  assert.equal(new Set(invocation.frames.map((frame: { input: { sessionId: string } }) => frame.input.sessionId)).size, 1);
  assert.equal(invocation.frames[1].input.source, "new");
  assert.equal(invocation.frames.at(-1).input.reason, "complete");
  assert.equal(invocation.session_start_result.data.success, true);
  assert.ok(invocation.session_start_result.data.output.additionalContext.includes(invocation.result.expected_marker));
  assert.equal(invocation.result.exit_code, 0);
  assert.equal(invocation.result.marker_assertion, "failed");
  assert.notEqual(invocation.result.actual_response, invocation.result.expected_marker);
  assert.equal(invocation.result.synveda_context_delivery, "not_verified");
  assert.equal(events[0].data.sessionId, invocation.result.session_id);
  assert.equal(events[0].data.sessionLimits.maxAiCredits, 30);
  assert.equal(events.at(-1).data.totalPremiumRequests, 1);
});

test("native Skill invocation has host evidence but no governed Synveda binding attribution", () => {
  const before = invocation.frames[2].input;
  const after = invocation.frames[3].input;
  assert.equal(before.toolName, "skill");
  assert.equal(after.toolName, before.toolName);
  assert.deepEqual(after.toolArgs, before.toolArgs);
  const start = events.find((event) => event.type === "tool.execution_start").data;
  const complete = events.find((event) => event.type === "tool.execution_complete").data;
  assert.equal(start.toolCallId, complete.toolCallId);
  assert.equal(complete.success, true);
  const skill = events.find((event) => event.type === "skill.invoked").data;
  assert.equal(skill.name, before.toolArgs.skill);
  assert.equal(skill.source, "project");
  assert.equal(skill.trigger, "agent-invoked");
  assert.ok(skill.content.includes(invocation.result.actual_response));
  assert.equal(invocation.result.governed_skill_activation, "not_exercised");
});

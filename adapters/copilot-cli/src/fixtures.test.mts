/** ADPT-9: authentic native evidence stays distinct from public-API conformance. */
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const capture = JSON.parse(readFileSync(new URL("../fixtures/lifecycle.json", import.meta.url), "utf8"));
const transcript = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
const events = transcript.trim().split("\n").map((line) => JSON.parse(line));
const invocation = capture.invocations[0];
const resume = JSON.parse(readFileSync(new URL("../fixtures/resume-lifecycle.json", import.meta.url), "utf8"));
const resumedEvents = readFileSync(new URL("../fixtures/resume-transcript.jsonl", import.meta.url), "utf8")
  .trim().split("\n").map((line) => JSON.parse(line));
const resumedInvocation = resume.invocations[0];
const governed = JSON.parse(readFileSync(new URL("../fixtures/governed-probe.json", import.meta.url), "utf8"));
const governedEvents = readFileSync(new URL("../fixtures/governed-transcript.jsonl", import.meta.url), "utf8")
  .trim().split("\n").map((line) => JSON.parse(line));

test("native fixture bytes are pinned and omit private paths and hidden model content", () => {
  const registry = JSON.parse(readFileSync(new URL("../../registry.json", import.meta.url), "utf8"));
  const client = registry.clients.find((entry: { id: string }) => entry.id === "copilot-cli");
  assert.equal(client.support_level, "experimental");
  assert.deepEqual(client.tested_versions, ["1.0.83"]);
  assert.equal(client.conformance.evidence_level, "native-probe");
  assert.deepEqual(client.conformance.checks, {}, "partial probes do not establish a complete native lifecycle");
  for (const name of ["lifecycle.json", "transcript.jsonl", "resume-lifecycle.json", "resume-transcript.jsonl",
    "governed-probe.json", "governed-transcript.jsonl"]) {
    const file = readFileSync(new URL(`../fixtures/${name}`, import.meta.url));
    const digest = createHash("sha256").update(file).digest("hex");
    assert.equal(client.authentic_fixtures.find((entry: { path: string }) =>
      entry.path === `adapters/copilot-cli/fixtures/${name}`).sha256, digest);
    assert.doesNotMatch(file.toString("utf8"), /\/Users\/|\/private\/|"(?:access_token|refresh_token|reasoningText|reasoningOpaque|reasoningBlocks|encryptedContent)"/);
  }
});

test("governed native calls use the injected task and preserve the original delivery failure", () => {
  const user = governedEvents.find((event) => event.type === "user.message").data.content;
  assert.ok(!user.includes(governed.synveda_session_id), "the user prompt must not supply the Synveda task");
  const calls = governedEvents.filter((event) => event.type === "tool.execution_start");
  const allowed = calls.find((event) => event.data.arguments.session_id === governed.synveda_session_id);
  assert.equal(allowed.data.toolName, "synveda-recall");
  const result = governedEvents.find((event) => event.type === "tool.execution_complete" &&
    event.data.toolCallId === allowed.data.toolCallId);
  assert.equal(result.data.success, true);
  assert.ok(result.data.result.content.includes(governed.native_mcp.knowledge_id));
  const denied = governedEvents.find((event) => event.id === governed.native_mcp.denied_completion_event_id);
  assert.equal(denied.data.success, false);
  assert.equal(denied.data.result, undefined);
  assert.equal(denied.data.error.code, "failure");
  assert.equal(calls.find((event) => event.data.toolCallId === denied.data.toolCallId).data.arguments.session_id,
    governed.native_mcp.denied_session_id);
  const skill = governedEvents.find((event) => event.type === "skill.invoked").data;
  assert.equal(skill.path, governed.governed_skill.native_path);
  assert.equal(skill.name, governed.governed_skill.name);
  assert.equal(skill.trigger, "agent-invoked");
  assert.equal(governed.observation_delivery.original_status, "failed");
  assert.equal(governed.observation_delivery.recorded_spool_entries, 0);
  assert.equal(governed.native_invocation.resume_prompt, "not_run");
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

test("native resume consumes a fresh hook-only marker without invoking tools", () => {
  assert.equal(resume.provenance.kind, "captured-real-client");
  assert.equal(resumedInvocation.result.exit_code, 0);
  assert.equal(resumedInvocation.result.timed_out, false);
  assert.equal(resumedInvocation.result.marker_assertion, "passed");
  const marker = resumedInvocation.result.expected_marker;
  assert.ok(!JSON.stringify(capture).includes(marker));
  assert.ok(!transcript.includes(marker), "the marker must be fresh to this resumed invocation");
  const user = resumedEvents.find((event) => event.type === "user.message").data;
  const assistant = resumedEvents.find((event) => event.type === "assistant.message").data;
  assert.ok(!user.content.includes(marker), "the model prompt must not supply the answer");
  assert.equal(assistant.content, marker);
  assert.equal(assistant.content, resumedInvocation.result.actual_response);
  assert.deepEqual(assistant.toolRequests, []);
  assert.equal(assistant.phase, "final_answer");
  const hook = resumedInvocation.session_start_result.data;
  assert.equal(hook.success, true);
  assert.ok(hook.output.additionalContext.includes(marker));
  assert.deepEqual(hook.output, resumedInvocation.frames[1].output);
  assert.deepEqual(resumedInvocation.frames.map((frame: { event: string }) => frame.event),
    ["userPromptSubmitted", "sessionStart", "agentStop", "sessionEnd"]);
  assert.equal(resumedInvocation.result.tool_invocations, 0);
  assert.equal(resumedInvocation.result.synveda_context_delivery, "not_verified");
});

test("native resume retains Session identity and limits after the prior runtime completed", () => {
  assert.equal(invocation.frames.at(-1).input.reason, "complete");
  assert.equal(resumedInvocation.frames[1].input.source, "resume");
  assert.equal(resumedInvocation.frames.at(-1).input.reason, "complete");
  assert.equal(resumedInvocation.result.session_id, invocation.result.session_id);
  for (const frame of resumedInvocation.frames) {
    assert.equal(frame.input.sessionId, invocation.result.session_id);
  }
  assert.equal(resumedEvents[0].type, "session.resume");
  assert.equal(resumedEvents[0].parentId, events.at(-1).id);
  assert.equal(resumedEvents[0].data.sessionLimits.maxAiCredits, 30);
  const shutdown = resumedEvents.at(-1);
  assert.equal(shutdown.type, "session.shutdown");
  assert.equal(shutdown.data.totalPremiumRequests, resumedInvocation.result.cumulative_premium_requests);
  assert.equal(shutdown.data.totalPremiumRequests - events.at(-1).data.totalPremiumRequests, 1);
  assert.equal(resumedInvocation.result.premium_request_delta, 1);
  assert.deepEqual(shutdown.data.codeChanges.filesModified, []);
});

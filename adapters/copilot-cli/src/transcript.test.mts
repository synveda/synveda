/** ADPT-9: captured native records plus explicitly authored fault cases. */
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { fileURLToPath } from "node:url";
import { MAX_TRANSCRIPT_BYTES } from "@synveda/claude-code-adapter/session-runtime";
import { toSessionEvents } from "../../claude-code/dist/events.mjs";
import { readCopilotTranscript } from "./transcript.mjs";

const nativeId = "a634658f-178c-4477-b448-632bc4ab4724";
const captured = readFileSync(new URL("../fixtures/transcript.jsonl", import.meta.url), "utf8");
const resumed = readFileSync(new URL("../fixtures/resume-transcript.jsonl", import.meta.url), "utf8");
const records = captured.trim().split("\n").map((line) => JSON.parse(line));
const governed = readFileSync(new URL("../fixtures/governed-transcript.jsonl", import.meta.url), "utf8");
const governedRecords = governed.trim().split("\n").map((line) => JSON.parse(line));
const governedId = governedRecords[0].data.sessionId;

test("shared native transcript replays all sixteen persisted observations with stable ids", () => {
  const path = new URL("../fixtures/shared-workflow-transcript.jsonl", import.meta.url);
  const records = readFileSync(path, "utf8").trim().split("\n").map((line) => JSON.parse(line));
  const events = toSessionEvents(readCopilotTranscript(fileURLToPath(path), records[0].data.sessionId), undefined);
  assert.equal(events.length, 16);
  assert.equal(new Set(events.map((event) => event.client_event_id)).size, 16);
  assert.deepEqual(events, toSessionEvents(readCopilotTranscript(fileURLToPath(path), records[0].data.sessionId), undefined));
  assert.equal(events.at(-1)?.event_type, "message.assistant");
  assert.equal(events.filter((event) => event.event_type === "tool.result").length, 6);
  assert.ok(!events.some((event) => event.event_type.startsWith("skill.")));
});

function fixture(t: TestContext) {
  const root = mkdtempSync(join(tmpdir(), "synveda-copilot-reader-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return { root, path: join(root, "events.jsonl") };
}

test("captured new/resume records yield six stable events with one actual tool pair", (t) => {
  const { path } = fixture(t);
  writeFileSync(path, captured + resumed);
  const events = toSessionEvents(readCopilotTranscript(path, nativeId), undefined);
  assert.deepEqual(events.map((event) => event.event_type), [
    "message.user", "tool.invoked", "tool.result", "message.assistant", "message.user", "message.assistant",
  ]);
  const expectedIds = [...records, ...resumed.trim().split("\n").map((line) => JSON.parse(line))]
    .filter((event) => event.type === "user.message" || event.type.startsWith("tool.execution_") ||
      event.type === "assistant.message" && event.data.content.length > 0).map((event) => event.id);
  assert.deepEqual(events.map((event) => event.client_event_id), expectedIds);
  assert.deepEqual(events, toSessionEvents(readCopilotTranscript(path, nativeId), undefined));
  const call = events[1].payload as { calls: { tool_use_id: string; name: string; input: string }[] };
  const result = events[2].payload as { tools: { tool_use_id: string; is_error: boolean; text: string }[] };
  assert.equal(call.calls[0].tool_use_id, result.tools[0].tool_use_id);
  assert.equal(call.calls[0].name, "skill");
  assert.deepEqual(JSON.parse(call.calls[0].input), { skill: "synveda-qualification" });
  assert.equal(result.tools[0].is_error, false);
  assert.equal(result.tools[0].text, records[4].data.result.content);
  assert.ok(!JSON.stringify(events).includes("Skill loaded successfully ✅"), "detailed Skill instructions are not copied");
  assert.ok(!events.some((event) => event.event_type.startsWith("skill.")), "native activation is not a governed binding");
});

test("unknown metadata and injected/hidden content never become observations", (t) => {
  const { path } = fixture(t);
  const fault = "private-prompt-marker";
  const altered = structuredClone(records);
  altered[1].data.attachments = [{ content: fault }];
  altered[6].data.reasoningText = fault;
  altered[6].data.reasoningOpaque = fault;
  altered[6].data.encryptedContent = fault;
  altered[5].data.content = fault;
  altered.splice(2, 0, { type: "system.message", data: { content: fault } },
    { type: "hook.end", data: { output: { additionalContext: fault } } });
  writeFileSync(path, altered.map((event) => JSON.stringify(event)).join("\n"));
  const events = toSessionEvents(readCopilotTranscript(path, nativeId), undefined);
  assert.equal(events.length, 4);
  assert.ok(!JSON.stringify(events).includes(fault));
});

test("foreign, absent, repeated and unsupported transcript headers are held", (t) => {
  const { path } = fixture(t);
  const variants = [
    { raw: captured.replace(nativeId, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"), reason: /session_mismatch/ },
    { raw: records.slice(1).map((event) => JSON.stringify(event)).join("\n"), reason: /missing_identity/ },
    { raw: captured + JSON.stringify(records[0]), reason: /session_mismatch/ },
    { raw: captured.replace('"version":1', '"version":2'), reason: /session_schema_unknown/ },
    { raw: captured + "{partial-private-prompt-marker", reason: /invalid_json/ },
    { raw: captured + "null", reason: /record_shape_unknown/ },
    { raw: captured + "\n".repeat(20_000), reason: /record_limit/ },
  ];
  for (const { raw, reason } of variants) {
    writeFileSync(path, raw);
    assert.throws(() => readCopilotTranscript(path, nativeId), reason);
    assert.equal(readFileSync(path, "utf8"), raw);
  }
});

test("ambiguous event identities, messages and tool results are held instead of losing the cursor", (t) => {
  const { path } = fixture(t);
  const variants: { mutate: (value: typeof records) => void; reason: RegExp }[] = [
    { mutate: (value) => { value[1].id = ""; }, reason: /event_identity/ },
    { mutate: (value) => { value[1].timestamp = "bad"; }, reason: /event_identity/ },
    { mutate: (value) => { value.push(value[1]); }, reason: /duplicate_event/ },
    { mutate: (value) => { value[1].data.content = []; }, reason: /message_shape_unknown/ },
    { mutate: (value) => { value[6].data.phase = "analysis"; }, reason: /message_shape_unknown/ },
    { mutate: (value) => { value[3].data.arguments = "unknown-arguments"; }, reason: /tool_call_shape_unknown/ },
    { mutate: (value) => { delete value[4].data.success; }, reason: /tool_result_shape_unknown/ },
    { mutate: (value) => { value[4].data.result.content = [{ type: "image" }]; }, reason: /tool_result_shape_unknown/ },
    { mutate: (value) => { value[4].data.toolCallId = "foreign-call"; }, reason: /tool_pair_unknown/ },
    { mutate: (value) => { value.splice(3, 1); }, reason: /tool_pair_unknown/ },
  ];
  for (const { mutate, reason } of variants) {
    const value = structuredClone(records);
    mutate(value);
    writeFileSync(path, value.map((event) => JSON.stringify(event)).join("\n"));
    assert.throws(() => readCopilotTranscript(path, nativeId), reason);
  }
});

test("an authored negative completion preserves the host boolean for the known text shape", (t) => {
  const { path } = fixture(t);
  const altered = structuredClone(records);
  altered[4].data.success = false;
  writeFileSync(path, altered.map((event) => JSON.stringify(event)).join("\n"));
  const events = toSessionEvents(readCopilotTranscript(path, nativeId), undefined);
  const result = events[2].payload as { tools: { is_error: boolean }[] };
  assert.equal(result.tools[0].is_error, true);
});

test("native MCP denial preserves all eight observations and out-of-order tool correlation", (t) => {
  const { path } = fixture(t);
  writeFileSync(path, governed);
  const events = toSessionEvents(readCopilotTranscript(path, governedId), undefined);
  assert.deepEqual(events.map((event) => event.event_type), [
    "message.user", "tool.invoked", "tool.result", "tool.invoked", "tool.invoked",
    "tool.result", "tool.result", "message.assistant",
  ]);
  const denied = governedRecords.find((record) => record.type === "tool.execution_complete" && record.data.success === false);
  const result = events.find((event) => event.client_event_id === denied.id)?.payload as {
    tools: { tool_use_id: string; is_error: boolean; text: string }[];
  };
  assert.equal(result.tools[0].tool_use_id, denied.data.toolCallId);
  assert.equal(result.tools[0].is_error, true);
  assert.equal(result.tools[0].text, denied.data.error.message);
  assert.equal(new Set(events.map((event) => event.client_event_id)).size, 8);
  assert.deepEqual(events, toSessionEvents(readCopilotTranscript(path, governedId), undefined));
});

test("only the captured failure shape is admitted and unrelated error fields are excluded", (t) => {
  const { path } = fixture(t);
  const altered = structuredClone(governedRecords);
  const failure = altered.find((record) => record.type === "tool.execution_complete" && record.data.success === false);
  failure.data.error.stack = "private-error-details";
  writeFileSync(path, altered.map((record) => JSON.stringify(record)).join("\n"));
  assert.ok(!JSON.stringify(readCopilotTranscript(path, governedId)).includes("private-error-details"));
  for (const fields of [
    { success: true }, { result: { content: "ambiguous" } }, { error: { code: "unknown", message: "future" } },
    { error: { code: "failure", message: { unknown: true } } }, { error: undefined },
  ]) {
    const variant = structuredClone(governedRecords);
    Object.assign(variant.find((record) => record.id === failure.id).data, fields);
    writeFileSync(path, variant.map((record) => JSON.stringify(record)).join("\n"));
    assert.throws(() => readCopilotTranscript(path, governedId), /tool_result_shape_unknown/);
  }
});

test("the shared file reader refuses oversize, symlink and special-file input", (t) => {
  const { root, path } = fixture(t);
  assert.deepEqual(readCopilotTranscript(path, nativeId), []);
  writeFileSync(path, "");
  assert.deepEqual(readCopilotTranscript(path, nativeId), []);
  truncateSync(path, MAX_TRANSCRIPT_BYTES + 1);
  assert.throws(() => readCopilotTranscript(path, nativeId), /size_or_type/);
  symlinkSync(path, join(root, "linked"));
  assert.throws(() => readCopilotTranscript(join(root, "linked"), nativeId), /unreadable/);
  assert.throws(() => readCopilotTranscript(root, nativeId), /size_or_type/);
});

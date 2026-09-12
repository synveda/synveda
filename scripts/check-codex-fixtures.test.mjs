// CPR-39: authentic protocol evidence; no gateway/lifecycle support claim.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const capture = JSON.parse(readFileSync(new URL("../adapters/codex/fixtures/lifecycle.json", import.meta.url), "utf8"));
const [initial, resumed] = capture.invocations;

test("native Codex hooks correlate the prompt, tool pair and assistant turn", () => {
  assert.equal(capture.client.version, "0.152.0");
  assert.equal(capture.provenance.kind, "captured-real-client");
  assert.deepEqual(initial.frames.map((frame) => frame.hook_event_name),
    ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "SessionEnd"]);
  const [start, prompt, before, after, stop] = initial.frames;
  assert.equal(start.source, "startup");
  assert.equal(new Set(initial.frames.map((frame) => frame.session_id)).size, 1);
  assert.equal(new Set([prompt, before, after, stop].map((frame) => frame.turn_id)).size, 1);
  assert.equal(before.tool_name, "Bash");
  assert.equal(before.tool_use_id, after.tool_use_id);
  assert.deepEqual(before.tool_input, after.tool_input);
  assert.equal(after.tool_response, "Synveda lifecycle qualification fixture.\n");
  assert.equal(stop.last_assistant_message, "Synveda lifecycle capture complete.");
});

test("a native runtime end is followed by resume of the same Codex session", () => {
  assert.equal(initial.frames.at(-1).reason, "other");
  assert.deepEqual(resumed.frames.map((frame) => frame.hook_event_name),
    ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"]);
  assert.equal(resumed.frames[0].source, "resume");
  assert.equal(new Set([...initial.frames, ...resumed.frames].map((frame) => frame.session_id)).size, 1);
  assert.notEqual(initial.frames[1].turn_id, resumed.frames[1].turn_id);
  assert.equal(resumed.frames.at(-1).reason, "other");
});

test("native manual compaction requests fresh context before the next prompt", () => {
  const compact = JSON.parse(readFileSync(new URL("../adapters/codex/fixtures/compaction.json", import.meta.url), "utf8"));
  assert.equal(compact.client.version, "0.152.0");
  assert.equal(compact.provenance.kind, "captured-real-client");
  const [before, after, resume, start, prompt, stop] = compact.frames;
  assert.deepEqual(compact.frames.map((frame) => frame.hook_event_name),
    ["PreCompact", "PostCompact", "SessionStart", "SessionStart", "UserPromptSubmit", "Stop"]);
  assert.equal(before.trigger, "manual");
  assert.equal(after.trigger, "manual");
  assert.equal(before.turn_id, after.turn_id);
  assert.equal(resume.source, "resume");
  assert.equal(start.source, "compact");
  assert.equal(prompt.turn_id, stop.turn_id);
  assert.equal(new Set(compact.frames.map((frame) => frame.session_id)).size, 1);
});

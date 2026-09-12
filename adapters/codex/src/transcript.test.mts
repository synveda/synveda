import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, symlinkSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { MAX_TRANSCRIPT_BYTES, readCodexTranscript } from "./transcript.mjs";
import { toSessionEvents } from "../../claude-code/dist/events.mjs";

const fixture = new URL("../fixtures/transcript.jsonl", import.meta.url);
const sessionId = "01a09672-099f-7f30-929d-8a5b72389a67";

test("native transcript preserves six user/assistant/tool events and excludes injected context", () => {
  const entries = readCodexTranscript(fixture.pathname, sessionId);
  const events = toSessionEvents(entries, "gpt-5.5");
  assert.deepEqual(events.map((event) => event.event_type), [
    "message.user", "tool.invoked", "tool.result", "message.assistant",
    "message.user", "message.assistant",
  ]);
  const text = JSON.stringify(events);
  assert.ok(!text.includes("environment_context"));
  assert.ok(!text.includes("base_instructions"));
  assert.equal(new Set(events.map((event) => event.client_event_id)).size, 6);
  assert.deepEqual(events, toSessionEvents(readCodexTranscript(fixture.pathname, sessionId), "gpt-5.5"));
  assert.ok(text.includes("call_Kn1FWjstIIz43WFfedeXfxs2"));
});

test("foreign, malformed, oversized and symlinked transcripts are refused without losing source bytes", () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-codex-reader-"));
  try {
    const path = join(root, "transcript.jsonl");
    const raw = readFileSync(fixture, "utf8");
    writeFileSync(path, raw);
    assert.throws(() => readCodexTranscript(path, "another-session"), /session_mismatch/);
    writeFileSync(path, raw + "{partial");
    assert.throws(() => readCodexTranscript(path, sessionId), /invalid_json/);
    assert.equal(readFileSync(path, "utf8"), raw + "{partial");
    writeFileSync(path, raw.split("\n").filter((line) => !line || JSON.parse(line).type !== "event_msg").join("\n"));
    assert.throws(() => readCodexTranscript(path, sessionId), /tool_result_status_unknown/);
    writeFileSync(path, raw.replace('"exit_code":0', '"exit_code":1'));
    const failed = toSessionEvents(readCodexTranscript(path, sessionId), "gpt-5.5");
    assert.ok(JSON.stringify(failed.find((event) => event.event_type === "tool.result")).includes('"is_error":true'));
    writeFileSync(path, raw.split("\n").slice(1).join("\n"));
    assert.throws(() => readCodexTranscript(path, sessionId), /missing_identity/);
    truncateSync(path, MAX_TRANSCRIPT_BYTES + 1);
    assert.throws(() => readCodexTranscript(path, sessionId), /size_or_type/);
    symlinkSync(path, join(root, "link"));
    assert.throws(() => readCodexTranscript(join(root, "link"), sessionId), /unreadable/);
    assert.throws(() => readCodexTranscript(root, sessionId), /size_or_type/);
    assert.deepEqual(readCodexTranscript(join(root, "missing"), sessionId), []);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

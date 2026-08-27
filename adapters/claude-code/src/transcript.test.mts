/**
 * The defensive transcript parser (ADR-0027 decision 9) and the task
 * derivation of decision 11.
 */

import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import {
  entriesAfter,
  lastUserPrompt,
  messageText,
  readTranscript,
  toolInvocations,
  toolResults,
  truncateChars,
} from "./transcript.mjs";

function transcript(lines: unknown[]): string {
  const file = join(mkdtempSync(join(tmpdir(), "synveda-transcript-")), "session.jsonl");
  writeFileSync(file, lines.map((line) => JSON.stringify(line)).join("\n"));
  return file;
}

function entry(overrides: Record<string, unknown>): Record<string, unknown> {
  return {
    type: "user",
    uuid: "u1",
    timestamp: "2026-07-24T10:00:00.000Z",
    message: { role: "user", content: "hello" },
    ...overrides,
  };
}

test("keeps user and assistant entries and drops harness bookkeeping", () => {
  const file = transcript([
    { type: "mode", mode: "default", sessionId: "s1" },
    entry({ uuid: "u1" }),
    entry({ uuid: "a1", type: "assistant" }),
    { type: "file-history-snapshot", messageId: "x" },
    entry({ uuid: "meta", isMeta: true }),
    entry({ uuid: "side", isSidechain: true }),
  ]);
  assert.deepEqual(
    readTranscript(file).map((parsed) => parsed.uuid),
    ["u1", "a1"],
  );
});

test("skips a line it cannot parse rather than failing the flush", () => {
  const file = join(mkdtempSync(join(tmpdir(), "synveda-transcript-")), "session.jsonl");
  writeFileSync(
    file,
    [
      JSON.stringify(entry({ uuid: "u1" })),
      "{ this is not json",
      "[]",
      "null",
      JSON.stringify(entry({ uuid: "u2" })),
      "",
    ].join("\n"),
  );
  assert.deepEqual(
    readTranscript(file).map((parsed) => parsed.uuid),
    ["u1", "u2"],
  );
});

test("requires the fields the adapter actually uses", () => {
  const file = transcript([
    entry({ uuid: 7 }),
    entry({ uuid: "u2", timestamp: 1234 }),
    entry({ uuid: "u3" }),
  ]);
  assert.deepEqual(
    readTranscript(file).map((parsed) => parsed.uuid),
    ["u3"],
  );
});

test("a missing transcript is empty, not an error", () => {
  assert.deepEqual(readTranscript("/nonexistent/session.jsonl"), []);
});

test("entriesAfter returns everything past the cursor", () => {
  const file = transcript([entry({ uuid: "u1" }), entry({ uuid: "u2" }), entry({ uuid: "u3" })]);
  const entries = readTranscript(file);
  const delta = entriesAfter(entries, "u1");
  assert.equal(delta.resynced, false);
  assert.deepEqual(
    delta.entries.map((parsed) => parsed.uuid),
    ["u2", "u3"],
  );
});

test("a cursor that no longer appears resynchronises from the beginning", () => {
  const file = transcript([entry({ uuid: "u9" })]);
  const delta = entriesAfter(readTranscript(file), "compacted-away");
  assert.equal(delta.resynced, true);
  assert.deepEqual(
    delta.entries.map((parsed) => parsed.uuid),
    ["u9"],
  );
});

test("no cursor means the whole transcript, and is not a resync", () => {
  const file = transcript([entry({ uuid: "u1" })]);
  const delta = entriesAfter(readTranscript(file), undefined);
  assert.equal(delta.resynced, false);
  assert.equal(delta.entries.length, 1);
});

test("messageText concatenates text blocks and ignores the rest", () => {
  assert.equal(messageText({ content: "plain" }), "plain");
  assert.equal(
    messageText({
      content: [
        { type: "text", text: "first" },
        { type: "tool_use", name: "Bash", input: { command: "ls" } },
        { type: "text", text: "second" },
      ],
    }),
    "first\nsecond",
  );
  assert.equal(messageText(undefined), "");
  assert.equal(messageText({ content: 42 }), "");
});

test("toolResults reads both string and block content", () => {
  const results = toolResults({
    content: [
      { type: "tool_result", tool_use_id: "t1", content: "output" },
      { type: "tool_result", tool_use_id: "t2", is_error: true, content: [{ type: "text", text: "boom" }] },
      { type: "text", text: "not a tool result" },
    ],
  });
  assert.equal(results.length, 2);
  assert.deepEqual(results[0], { tool_use_id: "t1", is_error: false, text: "output" });
  assert.deepEqual(results[1], { tool_use_id: "t2", is_error: true, text: "boom" });
});

test("toolInvocations reads the real tool_use shape and ignores opaque fields", () => {
  const calls = toolInvocations({
    content: [
      {
        type: "tool_use",
        id: "toolu_01",
        name: "Read",
        input: { file_path: "/Users/dev/Source/acme-api/src/retry.rs" },
        caller: { type: "direct" },
      },
      { type: "text", text: "not a call" },
    ],
  });
  assert.deepEqual(calls, [
    {
      tool_use_id: "toolu_01",
      name: "Read",
      input: '{"file_path":"/Users/dev/Source/acme-api/src/retry.rs"}',
    },
  ]);
});

test("lastUserPrompt takes the newest real prompt, not a tool result", () => {
  const file = transcript([
    entry({ uuid: "u1", message: { role: "user", content: "the first ask" } }),
    entry({ uuid: "a1", type: "assistant", message: { role: "assistant", content: "working" } }),
    entry({ uuid: "u2", message: { role: "user", content: "the second ask" } }),
    entry({
      uuid: "u3",
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "ls output" }] },
    }),
  ]);
  assert.equal(lastUserPrompt(readTranscript(file)), "the second ask");
});

test("lastUserPrompt is undefined when the session has no prompt yet", () => {
  const file = transcript([
    entry({ uuid: "a1", type: "assistant", message: { role: "assistant", content: "hello" } }),
  ]);
  assert.equal(lastUserPrompt(readTranscript(file)), undefined);
});

test("a prompt is capped at inject's own task limit", () => {
  const file = transcript([entry({ uuid: "u1", message: { role: "user", content: "x".repeat(5000) } })]);
  assert.equal(lastUserPrompt(readTranscript(file))?.length, 4096);
});

test("truncation never splits a surrogate pair", () => {
  // Four astral code points: eight UTF-16 units, four characters as the
  // gateway counts them.
  const emoji = "🙂🙃🙂🙃";
  const cut = truncateChars(emoji, 2);
  assert.equal(Array.from(cut).length, 2);
  assert.equal(cut, "🙂🙃");
  // A round trip through JSON must not produce a lone surrogate, which
  // Rust could not parse into a String.
  assert.equal(JSON.parse(JSON.stringify(cut)), cut);
});

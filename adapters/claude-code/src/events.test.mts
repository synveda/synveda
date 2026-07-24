/**
 * The observe mapping (ADR-0027 decision 8) against MEM-1's published
 * contract: 256 events per batch, 64 KiB per payload, the entry uuid as
 * the client-minted idempotency key.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { chunk, MAX_EVENT_PAYLOAD_BYTES, MAX_EVENTS_PER_BATCH, toObserveEvents } from "./events.mjs";
import type { TranscriptEntry } from "./transcript.mjs";

function entry(overrides: Partial<TranscriptEntry> = {}): TranscriptEntry {
  return {
    uuid: "u1",
    type: "user",
    timestamp: "2026-07-24T10:00:00.000Z",
    message: { role: "user", content: "ship the release train" },
    ...overrides,
  };
}

function payloadOf(value: unknown): Record<string, unknown> {
  assert.ok(value !== null && typeof value === "object");
  return value as Record<string, unknown>;
}

test("a plain turn is a transcript_delta keyed by the entry uuid", () => {
  const [event] = toObserveEvents([entry()], "claude-opus-5");
  assert.ok(event);
  assert.equal(event.kind, "transcript_delta");
  assert.equal(event.idempotency_key, "u1");
  assert.equal(event.occurred_at, "2026-07-24T10:00:00.000Z");
  const payload = payloadOf(event.payload);
  assert.equal(payload.role, "user");
  assert.equal(payload.text, "ship the release train");
});

test("an entry carrying tool output is a tool_result", () => {
  const [event] = toObserveEvents(
    [
      entry({
        message: {
          role: "user",
          content: [{ type: "tool_result", tool_use_id: "t1", content: "3 tests passed" }],
        },
      }),
    ],
    undefined,
  );
  assert.ok(event);
  assert.equal(event.kind, "tool_result");
  const tools = payloadOf(event.payload).tools;
  assert.ok(Array.isArray(tools));
  assert.equal(tools.length, 1);
});

test("entries with nothing to say are skipped", () => {
  const events = toObserveEvents(
    [entry({ message: { role: "user", content: [] } }), entry({ uuid: "u2", message: undefined })],
    undefined,
  );
  assert.deepEqual(events, []);
});

test("context carries the project name, never the full path", () => {
  const [event] = toObserveEvents(
    [entry({ cwd: "/Users/someone/Source/synveda", gitBranch: "feat/ADPT-1" })],
    "claude-opus-5",
  );
  assert.ok(event);
  assert.deepEqual(payloadOf(event.payload).context, {
    project: "synveda",
    git_branch: "feat/ADPT-1",
    model: "claude-opus-5",
  });
});

test("context is omitted entirely when there is nothing to say", () => {
  const [event] = toObserveEvents([entry()], undefined);
  assert.ok(event);
  assert.equal(payloadOf(event.payload).context, undefined);
});

test("an unparseable timestamp falls back to now instead of rejecting the batch", () => {
  const [event] = toObserveEvents([entry({ timestamp: "not a date" })], undefined);
  assert.ok(event);
  assert.ok(!Number.isNaN(Date.parse(event.occurred_at)));
});

test("an oversized payload is truncated, marked, and still sent", () => {
  const [event] = toObserveEvents(
    [
      entry({
        message: {
          role: "user",
          content: [{ type: "tool_result", tool_use_id: "t1", content: "x".repeat(400_000) }],
        },
      }),
    ],
    undefined,
  );
  assert.ok(event);
  const payload = payloadOf(event.payload);
  assert.equal(payload.truncated, true);
  const bytes = new TextEncoder().encode(JSON.stringify(payload)).length;
  assert.ok(
    bytes <= MAX_EVENT_PAYLOAD_BYTES,
    `payload is ${String(bytes)} bytes, over the ${String(MAX_EVENT_PAYLOAD_BYTES)} cap`,
  );
  // The gist survives: truncation is not deletion.
  assert.ok(String(payload.text).length + JSON.stringify(payload.tools).length > 1000);
});

test("an oversized message text is truncated the same way", () => {
  const [event] = toObserveEvents(
    [entry({ message: { role: "assistant", content: "y".repeat(300_000) }, type: "assistant" })],
    undefined,
  );
  assert.ok(event);
  const payload = payloadOf(event.payload);
  assert.equal(payload.truncated, true);
  const bytes = new TextEncoder().encode(JSON.stringify(payload)).length;
  assert.ok(bytes <= MAX_EVENT_PAYLOAD_BYTES);
});

test("batches never exceed the gateway's per-batch cap", () => {
  const entries = Array.from({ length: 600 }, (_, index) => entry({ uuid: `u${String(index)}` }));
  const batches = chunk(toObserveEvents(entries, undefined), MAX_EVENTS_PER_BATCH);
  assert.deepEqual(
    batches.map((batch) => batch.length),
    [256, 256, 88],
  );
});

test("chunking an empty list yields no batches", () => {
  assert.deepEqual(chunk([], MAX_EVENTS_PER_BATCH), []);
});

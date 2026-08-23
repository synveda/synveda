/**
 * The transcript → session-event mapping (CPR-12, ADR-0078) against the append
 * route's published contract: 200 events per batch, 64 KiB per payload, the
 * entry uuid as the client-minted event id.
 *
 * The interesting change from the observe mapping is that **one turn can be
 * two events**. `ObserveKind` had one value for "a turn that also called
 * tools"; the session vocabulary separates what was said from what a tool
 * returned, so the ids have to stay unique without becoming unstable.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  chunk,
  MAX_EVENT_PAYLOAD_BYTES,
  MAX_EVENTS_PER_BATCH,
  toSessionEvents,
} from "./events.mjs";
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

test("a user turn is a message.user keyed by the entry uuid", () => {
  const [event] = toSessionEvents([entry()], "claude-opus-5");
  assert.ok(event);
  assert.equal(event.event_type, "message.user");
  assert.equal(event.client_event_id, "u1");
  assert.equal(event.occurred_at, "2026-07-24T10:00:00.000Z");
  assert.equal(payloadOf(event.payload).text, "ship the release train");
});

test("an assistant turn is a message.assistant", () => {
  const [event] = toSessionEvents(
    [entry({ type: "assistant", message: { role: "assistant", content: "done" } })],
    undefined,
  );
  assert.ok(event);
  assert.equal(event.event_type, "message.assistant");
});

/**
 * The case the old mapping could not express: a turn that both said something
 * and returned tool output is two events, not one event with both in it.
 */
test("a turn with text and tool output is two events with stable distinct ids", () => {
  const events = toSessionEvents(
    [
      entry({
        type: "assistant",
        message: {
          role: "assistant",
          content: [
            { type: "text", text: "running it" },
            { type: "tool_result", tool_use_id: "t1", content: "exit 0" },
          ],
        },
      }),
    ],
    undefined,
  );
  assert.equal(events.length, 2);
  assert.deepEqual(
    events.map((event) => event.event_type),
    ["message.assistant", "tool.result"],
  );
  assert.deepEqual(
    events.map((event) => event.client_event_id),
    ["u1:msg", "u1:tool"],
  );
  // Re-reading the same transcript must produce the same two ids, or the
  // append's idempotency gate cannot recognise them.
  const again = toSessionEvents(
    [
      entry({
        type: "assistant",
        message: {
          role: "assistant",
          content: [
            { type: "text", text: "running it" },
            { type: "tool_result", tool_use_id: "t1", content: "exit 0" },
          ],
        },
      }),
    ],
    undefined,
  );
  assert.deepEqual(
    again.map((event) => event.client_event_id),
    ["u1:msg", "u1:tool"],
  );
});

/** A turn that only returned tool output keeps the bare uuid. */
test("a tool-only turn keeps the unsuffixed id", () => {
  const events = toSessionEvents(
    [
      entry({
        type: "assistant",
        message: {
          role: "assistant",
          content: [{ type: "tool_result", tool_use_id: "t1", content: "exit 0" }],
        },
      }),
    ],
    undefined,
  );
  assert.equal(events.length, 1);
  assert.equal(events[0]?.event_type, "tool.result");
  assert.equal(events[0]?.client_event_id, "u1");
});

test("an entry with neither text nor tools yields nothing", () => {
  assert.deepEqual(toSessionEvents([entry({ message: { role: "user", content: "" } })], undefined), []);
});

test("the model and project ride in the payload context", () => {
  const [event] = toSessionEvents(
    [entry({ cwd: "/Users/someone/work/synveda", gitBranch: "main", version: "2.1.0" })],
    "claude-opus-5",
  );
  assert.ok(event);
  const context = payloadOf(payloadOf(event.payload).context);
  // The basename, never the path: a home directory is user data that would
  // otherwise ride into every record.
  assert.equal(context.project, "synveda");
  assert.equal(context.git_branch, "main");
  assert.equal(context.model, "claude-opus-5");
  assert.equal(context.harness_version, "2.1.0");
});

test("an unparseable timestamp falls back to now rather than rejecting the batch", () => {
  const [event] = toSessionEvents([entry({ timestamp: "not a date" })], undefined);
  assert.ok(event);
  assert.ok(!Number.isNaN(Date.parse(event.occurred_at)));
});

test("an oversized payload is truncated and says so", () => {
  const [event] = toSessionEvents(
    [entry({ message: { role: "user", content: "x".repeat(MAX_EVENT_PAYLOAD_BYTES * 2) } })],
    undefined,
  );
  assert.ok(event);
  const encoded = new TextEncoder().encode(JSON.stringify(event.payload)).length;
  assert.ok(encoded <= MAX_EVENT_PAYLOAD_BYTES, `payload was ${String(encoded)} bytes`);
  assert.equal(payloadOf(event.payload).truncated, true);
});

test("batching respects the route's cap", () => {
  const batches = chunk(Array.from({ length: MAX_EVENTS_PER_BATCH * 2 + 3 }, (_, i) => i), MAX_EVENTS_PER_BATCH);
  assert.equal(batches.length, 3);
  assert.equal(batches[0]?.length, MAX_EVENTS_PER_BATCH);
  assert.equal(batches[2]?.length, 3);
});

/**
 * The batch cap is the gateway's `MAX_EVENT_BATCH`, restated. It moved from
 * 256 to 200 with the cutover, and a stale copy here would produce a 400 on
 * every full batch.
 */
test("the batch cap matches the append route", () => {
  assert.equal(MAX_EVENTS_PER_BATCH, 200);
});

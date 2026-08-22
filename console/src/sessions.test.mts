/**
 * What the Sessions page decides (CPR-10, ADR-0076), asserted without a
 * browser — the split `people.test.tsx` established, and for its reason:
 * there is still no browser test runner, so what a page decides must be
 * decidable outside it.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_SENTENCE,
  countSummary,
  durationOf,
  isContextRun,
  runDescription,
  runTitle,
  statusLabel,
  statusTone,
} from "./sessions.mjs";
import type { SessionView } from "./generated/api.js";

function session(overrides: Partial<SessionView> = {}): SessionView {
  return {
    id: "00000000-0000-0000-0000-000000000001",
    workspace_id: "00000000-0000-0000-0000-000000000002",
    scope_id: "00000000-0000-0000-0000-000000000003",
    principal_id: "alice@example.com",
    client_name: "claude-code",
    status: "active",
    started_at: "2026-08-23T10:00:00Z",
    metadata: {},
    created_at: "2026-08-23T10:00:00Z",
    updated_at: "2026-08-23T10:00:00Z",
    ...overrides,
  } as SessionView;
}

test("every state has a label and a tone, and the two that read alike stay apart", () => {
  const states = ["active", "ending", "ended", "abandoned", "failed"] as const;
  const labels = states.map((status) => statusLabel(status));
  assert.equal(new Set(labels).size, states.length, "five states, five distinct sentences");
  for (const label of labels) {
    assert.ok(label.length > 0);
  }
  // The distinction this page exists not to collapse: a run nobody closed is
  // not a run that broke.
  assert.notEqual(statusLabel("abandoned"), statusLabel("failed"));
  assert.equal(statusTone("active"), "live");
  assert.equal(statusTone("ending"), "live");
  assert.equal(statusTone("ended"), "done");
  assert.equal(statusTone("abandoned"), "warn");
  assert.equal(statusTone("failed"), "warn");
});

test("a description names what the client reported and nothing it did not", () => {
  assert.equal(runDescription(session()), "claude-code");
  assert.equal(
    runDescription(
      session({
        client_version: "2.1.0",
        model_name: "claude-opus-5",
        agent_name: "reviewer",
        branch: "main",
      }),
    ),
    "claude-code · v2.1.0 · claude-opus-5 · reviewer · on main",
  );
  // An absent field is absent, not an em-dash: its absence tells a reader the
  // client did not report it, where a placeholder tells them nothing.
  assert.ok(!runDescription(session()).includes("—"));
  assert.ok(!runDescription(session()).includes("undefined"));
});

test("a run's title is what the client said, or a recognisable fallback", () => {
  assert.equal(runTitle(session({ task_summary: "Refactor the ledger" })), "Refactor the ledger");
  const fallback = runTitle(session());
  assert.equal(fallback, "claude-code run");
  // Never the id: it is the one thing a reader cannot recognise.
  assert.ok(!fallback.includes("00000000"));
});

test("a duration is a measurement or it is absent", () => {
  const now = Date.parse("2026-08-23T10:05:30Z");
  assert.equal(durationOf(session(), now), "5m");
  assert.equal(
    durationOf(session({ ended_at: "2026-08-23T10:00:42Z" }), now),
    "42s",
    "a closed run measures to its own end, not to now",
  );
  assert.equal(durationOf(session({ ended_at: "2026-08-23T13:20:00Z" }), now), "3h 20m");
  // Unreadable or impossible timestamps produce nothing rather than `0s`,
  // which would be a measurement nobody made.
  assert.equal(durationOf(session({ started_at: "not a date" }), now), null);
  assert.equal(durationOf(session({ started_at: "2026-08-23T11:00:00Z" }), now), null);
});

test("the count summary leads with the busiest family", () => {
  assert.deepEqual(
    countSummary({ "adapter.warning": 1, "message.user": 10, "tool.invoked": 4 }),
    [
      { type: "message.user", count: 10 },
      { type: "tool.invoked", count: 4 },
      { type: "adapter.warning", count: 1 },
    ],
  );
  // Ties break by name, so the order is stable across reads.
  assert.deepEqual(countSummary({ b: 2, a: 2 }), [
    { type: "a", count: 2 },
    { type: "b", count: 2 },
  ]);
  assert.deepEqual(countSummary({}), []);
});

test("a context run is told from an event by kind, not by shape", () => {
  assert.equal(
    isContextRun({ kind: "context_run", id: "1", at: "2026-08-23T10:00:00Z", summary: "x" }),
    true,
  );
  assert.equal(
    isContextRun({ kind: "event", id: "1", at: "2026-08-23T10:00:00Z", summary: "x" }),
    false,
  );
});

test("the empty sentence refuses to claim nothing happened", () => {
  // The assertion is about what it must NOT say: a caller sees the runs their
  // grants reach, so "nothing has happened here" would be a claim the console
  // cannot make.
  assert.ok(EMPTY_SENTENCE.includes("read"));
  assert.ok(
    /nobody has run an agent/.test(EMPTY_SENTENCE),
    "it names the other possibility rather than picking one",
  );
  assert.ok(!/^No runs\.$/.test(EMPTY_SENTENCE));
});

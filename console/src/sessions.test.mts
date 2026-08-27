/**
 * What the Sessions page decides (CPR-10, ADR-0076), asserted without a
 * browser — the split `people.test.tsx` established, and for its reason:
 * there is still no browser test runner, so what a page decides must be
 * decidable outside it.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_FILTERED_SENTENCE,
  EMPTY_SENTENCE,
  NO_FILTERS,
  appendPage,
  countSummary,
  dayAfter,
  dayStart,
  deliveryNote,
  durationOf,
  endLine,
  gapOf,
  isContextRun,
  isFiltered,
  isIncomplete,
  isLate,
  isWarning,
  listQuery,
  repositoryLine,
  runDescription,
  runTitle,
  statusLabel,
  statusTone,
  warningCount,
} from "./sessions.mjs";
import type { RepositoryView, SessionView, TimelineEntry } from "./generated/api.js";

function entry(overrides: Partial<TimelineEntry> = {}): TimelineEntry {
  return {
    kind: "event",
    id: "e-1",
    at: "2026-08-23T10:00:00Z",
    delayed: false,
    summary: "message.user",
    ...overrides,
  };
}

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
    isContextRun(entry({ kind: "context_run" })),
    true,
  );
  assert.equal(isContextRun(entry()), false);
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

// ── CPR-11: what the product surface decides ────────────────────────────────

test("an incomplete run is told from a running one and from a finished one", () => {
  // The distinction the page turns on: `active` is not incomplete — it is
  // running, which reads differently and calls for nothing.
  assert.equal(isIncomplete(session({ status: "active" })), false);
  assert.equal(isIncomplete(session({ status: "ended" })), false);
  assert.equal(isIncomplete(session({ status: "ending" })), true);
  assert.equal(isIncomplete(session({ status: "abandoned" })), true);
  assert.equal(isIncomplete(session({ status: "failed" })), true);
});

test("the end line names the reason when there is one, and says so when there is not", () => {
  assert.equal(endLine(session({ status: "active" })), null);
  assert.equal(endLine(session({ status: "ended" })), null);

  const withReason = endLine(
    session({ status: "failed", ended_at: "2026-08-23T10:05:00Z", end_reason: "hook timed out" }),
  );
  assert.ok(withReason?.includes("hook timed out"), withReason ?? "no line");

  // Without one it says nothing was recorded rather than inventing a cause:
  // that is a fact about the client, and a reader who sees it knows to go and
  // look at the adapter rather than at the run.
  const without = endLine(session({ status: "failed", ended_at: "2026-08-23T10:05:00Z" }));
  assert.ok(without?.includes("No reason was recorded"), without ?? "no line");

  // `abandoned` and `failed` do not share a sentence, for `statusLabel`'s
  // reason one layer up.
  assert.notEqual(
    endLine(session({ status: "abandoned", ended_at: "2026-08-23T10:05:00Z" })),
    without,
  );
});

test("what a run was working on names the repository, never its id", () => {
  const repository: RepositoryView = {
    id: "r-1",
    project_id: "p-1",
    canonical_uri: "https://github.com/acme/ledger",
    provider: "github",
    repository_name: "acme/ledger",
    metadata: {},
    created_at: "2026-08-20T09:00:00Z",
    updated_at: "2026-08-20T09:00:00Z",
  };

  assert.equal(repositoryLine(session(), []), null);
  assert.equal(repositoryLine(session({ branch: "main" }), []), "branch main");
  assert.equal(
    repositoryLine(session({ repository_id: "r-1", branch: "main" }), [repository]),
    "https://github.com/acme/ledger · branch main",
  );
  // A repository the project no longer has is described rather than shown as
  // a uuid: the run really did happen against something.
  const detached = repositoryLine(session({ repository_id: "r-9" }), [repository]);
  assert.ok(detached?.includes("detached"), detached ?? "no line");
  assert.ok(!detached?.includes("r-9"));
});

test("a warning is an event family, and it is counted from the timeline's own counts", () => {
  assert.equal(isWarning(entry({ event_type: "adapter.warning" })), true);
  assert.equal(isWarning(entry({ event_type: "message.user" })), false);
  assert.equal(isWarning(entry({ kind: "context_run" })), false);
  assert.equal(warningCount({ "adapter.warning": 3, "message.user": 9 }), 3);
  assert.equal(warningCount({ "message.user": 9 }), 0);
});

test("lateness is the server's answer, and the note reports the gap rather than the cause", () => {
  // The console never recomputes the threshold: one definition of "late", in
  // the API, so every reader of a timeline agrees.
  assert.equal(isLate(entry({ delayed: false })), false);
  assert.equal(isLate(entry({ delayed: true })), true);

  assert.equal(deliveryNote(entry()), null, "a live entry says nothing at all");
  assert.equal(
    deliveryNote(entry({ delayed: true })),
    null,
    "and neither does one with no received_at — a context run",
  );

  const note = deliveryNote(
    entry({ delayed: true, at: "2026-08-23T10:00:00Z", received_at: "2026-08-23T11:30:00Z" }),
  );
  assert.ok(note?.includes("1h 30m"), note ?? "no note");
  // It reports the gap and refuses to name a cause: a local spool replay and
  // a machine with a wrong clock produce the same two instants, and the
  // server cannot tell them apart.
  assert.ok(note?.includes("recovered or delayed"), note ?? "no note");
});

test("a gap reads in the largest unit that is still true", () => {
  assert.equal(gapOf(4_000), "4s");
  assert.equal(gapOf(90_000), "1m");
  assert.equal(gapOf(5_400_000), "1h 30m");
  assert.equal(gapOf(100_000_000), "1d 3h");
  assert.equal(gapOf(-5), "0s");
});

test("a filter nobody set is absent from the query, not empty in it", () => {
  // The distinction `client.mts` documents: an undefined parameter is dropped
  // entirely, and a filter set to nothing is a different request.
  const bare = listQuery(NO_FILTERS, "scope-1", null);
  assert.equal(bare.scope_id, "scope-1");
  assert.equal(bare.status, undefined);
  assert.equal(bare.client_name, undefined);
  assert.equal(bare.principal_id, undefined);
  assert.equal(bare.started_after, undefined);
  assert.equal(bare.started_before, undefined);
  assert.equal(bare.cursor, undefined);

  // Whitespace is nothing, not a client called "   ".
  assert.equal(listQuery({ ...NO_FILTERS, clientName: "   " }, null, null).client_name, undefined);
  assert.equal(
    listQuery({ ...NO_FILTERS, clientName: " claude-code " }, null, null).client_name,
    "claude-code",
  );
});

test("a date range is the reader's calendar days, half-open at the API", () => {
  assert.equal(dayStart("2026-08-23"), "2026-08-23T00:00:00.000Z");
  // The upper bound is the *next* midnight, so "to the 23rd" includes the
  // 23rd. The off-by-one belongs here rather than to the reader.
  assert.equal(dayAfter("2026-08-23"), "2026-08-24T00:00:00.000Z");
  assert.equal(dayStart(""), undefined);
  assert.equal(dayStart("23/08/2026"), undefined);
  assert.equal(dayAfter("nonsense"), undefined);

  const ranged = listQuery({ ...NO_FILTERS, from: "2026-08-23", to: "2026-08-23" }, null, null);
  assert.equal(ranged.started_after, "2026-08-23T00:00:00.000Z");
  assert.equal(ranged.started_before, "2026-08-24T00:00:00.000Z");
});

test("the page knows whether the reader narrowed it, because the empty sentence differs", () => {
  assert.equal(isFiltered(NO_FILTERS), false);
  assert.equal(isFiltered({ ...NO_FILTERS, status: "failed" }), true);
  assert.equal(isFiltered({ ...NO_FILTERS, projectId: "p-1" }), true);
  assert.equal(isFiltered({ ...NO_FILTERS, clientName: "  " }), false, "whitespace is not a filter");
  assert.equal(isFiltered({ ...NO_FILTERS, from: "2026-08-23" }), true);
  assert.notEqual(EMPTY_SENTENCE, EMPTY_FILTERED_SENTENCE);
});

test("a page appended twice adds nothing twice", () => {
  // The failure this exists for: a reader who clicks Load more twice before
  // the first answer lands sends one cursor twice and is served one page
  // twice.
  const first = [session({ id: "s-1" }), session({ id: "s-2" })];
  const second = [session({ id: "s-2" }), session({ id: "s-3" })];
  assert.deepEqual(
    appendPage(first, second).map((row) => row.id),
    ["s-1", "s-2", "s-3"],
  );
  assert.deepEqual(appendPage([], []).length, 0);
  assert.deepEqual(
    appendPage(first, first).map((row) => row.id),
    ["s-1", "s-2"],
  );
});

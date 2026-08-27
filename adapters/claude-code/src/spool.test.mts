/**
 * The durable spool (CPR-12, ADR-0078 decision 6).
 *
 * The properties worth pinning are the ones that make it *durable* rather than
 * a file that happens to hold events: the write is atomic, the hash matches
 * what the Rust side computes, and any non-current shape is refused.
 */

import assert from "node:assert/strict";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";

import {
  acknowledge,
  allSpools,
  bindGateway,
  loadSpool,
  loadOrCreateSpool,
  newSpool,
  payloadHash,
  pending,
  readSpool,
  record,
  recordAttempt,
  retireIfComplete,
  saveSpool,
  spoolFile,
  SPOOL_VERSION,
} from "./spool.mjs";
import { spoolDir } from "./paths.mjs";

let home: string;
let previous: string | undefined;

before(() => {
  home = mkdtempSync(join(tmpdir(), "synveda-spool-"));
  previous = process.env.XDG_STATE_HOME;
  process.env.XDG_STATE_HOME = home;
});

after(() => {
  if (previous === undefined) delete process.env.XDG_STATE_HOME;
  else process.env.XDG_STATE_HOME = previous;
  rmSync(home, { recursive: true, force: true });
});

function fresh(id: string) {
  return newSpool(id, "claude-code", "install-1");
}

describe("the spool", () => {
  test("records an event and reads it back", () => {
    const spool = fresh("harness-a");
    const added = record(spool, [
      {
        event_type: "message.user",
        client_event_id: "u1",
        occurred_at: "2026-08-25T10:00:00.000Z",
        payload: { text: "hello" },
      },
    ]);
    assert.equal(added, 1);
    assert.ok(saveSpool(spool));

    const reloaded = loadSpool("harness-a");
    assert.ok(reloaded);
    assert.equal(reloaded.entries.length, 1);
    assert.equal(reloaded.entries[0]?.sequence, 1);
    assert.equal(reloaded.entries[0]?.acknowledged, false);
    assert.equal(reloaded.spool_version, SPOOL_VERSION);
  });

  /**
   * The skip is what makes a hook that fires twice for one turn — a retry, an
   * overlapping Stop and PreCompact — record each entry once.
   */
  test("recording the same event id twice adds it once", () => {
    const spool = fresh("harness-b");
    const event = {
      event_type: "message.user" as const,
      client_event_id: "u1",
      occurred_at: "2026-08-25T10:00:00.000Z",
      payload: { text: "hello" },
    };
    assert.equal(record(spool, [event]), 1);
    assert.equal(record(spool, [event]), 0);
    assert.equal(spool.entries.length, 1);
  });

  test("sequence is per spool and monotonic", () => {
    const spool = fresh("harness-c");
    record(spool, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
      { event_type: "message.assistant", client_event_id: "b", occurred_at: "2026-08-25T10:00:01Z", payload: {} },
    ]);
    record(spool, [
      { event_type: "tool.result", client_event_id: "c", occurred_at: "2026-08-25T10:00:02Z", payload: {} },
    ]);
    assert.deepEqual(spool.entries.map((entry) => entry.sequence), [1, 2, 3]);
  });

  /**
   * The one property that makes the two halves of this format interoperate:
   * key order must not change the digest, because the Rust side sorts and a
   * JSON encoder that preserves insertion order would otherwise disagree.
   */
  test("the payload hash does not depend on key order", () => {
    assert.equal(
      payloadHash({ a: 1, b: [2, 3] }),
      payloadHash({ b: [2, 3], a: 1 }),
    );
    assert.equal(payloadHash({ a: 1 }).length, 64);
    // Order *within* an array is content, not encoding.
    assert.notEqual(payloadHash({ b: [2, 3] }), payloadHash({ b: [3, 2] }));
  });

  test("a spool is pinned to its first authenticated gateway", () => {
    const spool = fresh("gateway-pin");
    assert.equal(bindGateway(spool, "https://one.example"), true);
    assert.equal(bindGateway(spool, "https://one.example"), true);
    assert.equal(bindGateway(spool, "https://two.example"), false);
    assert.equal(spool.gateway_url, "https://one.example");
  });

  /**
   * Acknowledgement is keyed by the client's own id, so a batch whose answers
   * come back in a different order still marks the right rows.
   */
  test("acknowledgement is keyed by event id and not by position", () => {
    const spool = fresh("harness-d");
    record(spool, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
      { event_type: "message.user", client_event_id: "b", occurred_at: "2026-08-25T10:00:01Z", payload: {} },
      { event_type: "message.user", client_event_id: "c", occurred_at: "2026-08-25T10:00:02Z", payload: {} },
    ]);
    const marked = acknowledge(
      spool,
      new Map([
        ["c", "appended"],
        ["a", "duplicate"],
      ]),
    );
    assert.equal(marked, 2);
    assert.deepEqual(pending(spool).map((entry) => entry.client_event_id), ["b"]);
    assert.equal(spool.entries[0]?.outcome, "duplicate");
  });

  /**
   * Every terminal answer acknowledges, including the two that store nothing
   * useful: re-sending a denied event produces the same denial forever, and a
   * spool that retried it would never drain.
   */
  test("a denied event is acknowledged like any other", () => {
    const spool = fresh("harness-e");
    record(spool, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    acknowledge(spool, new Map([["a", "denied"]]));
    assert.equal(pending(spool).length, 0);
  });

  test("an attempt counts only against what is still pending", () => {
    const spool = fresh("harness-f");
    record(spool, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
      { event_type: "message.user", client_event_id: "b", occurred_at: "2026-08-25T10:00:01Z", payload: {} },
    ]);
    acknowledge(spool, new Map([["a", "appended"]]));
    recordAttempt(spool);
    recordAttempt(spool);
    assert.equal(spool.entries[0]?.delivery_attempts, 0);
    assert.equal(spool.entries[1]?.delivery_attempts, 2);
  });

  /** A rename that did not happen leaves a `.tmp` behind. */
  test("the write is atomic and leaves no temporary", () => {
    const spool = fresh("harness-g");
    record(spool, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    mkdirSync(spoolDir(), { recursive: true });
    const stale = `${spoolFile("harness-g")}.${String(process.pid)}.tmp`;
    writeFileSync(stale, "abandoned", { mode: 0o666 });
    if (process.platform !== "win32") chmodSync(stale, 0o666);
    saveSpool(spool);
    const leftovers = readdirSync(spoolDir()).filter((name) => name.endsWith(".tmp"));
    assert.deepEqual(leftovers, []);
    // And the file that landed is complete JSON, not a partial write.
    const raw = readFileSync(spoolFile("harness-g"), "utf8");
    assert.doesNotThrow(() => JSON.parse(raw));
    if (process.platform !== "win32") {
      assert.equal(statSync(spoolDir()).mode & 0o777, 0o700);
      assert.equal(statSync(spoolFile("harness-g")).mode & 0o777, 0o600);
    }
  });

  /**
   * ADR-0078 decision 6: nothing reads the previous format. It held a cursor
   * and no events, so parsing one optimistically would produce an empty spool
   * that silently claims there is nothing to deliver.
   */
  test("a non-current cursor shape is not read", () => {
    const path = spoolFile("harness-pre-cut");
    writeFileSync(
      path,
      JSON.stringify({
        session_id: "claude-code:abc",
        transcript_path: "/tmp/t.jsonl",
        cursor: "uuid-1",
        updated_at: "2026-01-01T00:00:00Z",
      }),
    );
    assert.equal(readSpool(path), undefined);
    rmSync(path, { force: true });
  });

  test("an unknown spool version is refused rather than guessed at", () => {
    const path = spoolFile("harness-future");
    const future = JSON.stringify({ ...fresh("harness-future"), spool_version: 99 });
    writeFileSync(path, future);
    assert.equal(readSpool(path), undefined);
    assert.equal(
      loadOrCreateSpool("harness-future", "claude-code", "install-1"),
      undefined,
      "a hook must hold unknown state instead of treating it as a missing spool",
    );
    assert.equal(readFileSync(path, "utf8"), future, "the refused bytes remain untouched");
    rmSync(path, { force: true });
  });

  test("a tampered payload is held and never replaced by a fresh spool", () => {
    const spool = fresh("harness-tampered");
    record(spool, [
      {
        event_type: "message.user",
        client_event_id: "u-tampered",
        occurred_at: "2026-08-25T10:00:00Z",
        payload: { text: "approved bytes" },
      },
    ]);
    assert.equal(saveSpool(spool), true);
    const path = spoolFile("harness-tampered");
    const changed = JSON.parse(readFileSync(path, "utf8")) as {
      entries: { payload: unknown }[];
    };
    if (changed.entries[0] === undefined) throw new Error("fixture has no event");
    changed.entries[0].payload = { text: "forged bytes" };
    const tampered = JSON.stringify(changed);
    writeFileSync(path, tampered);

    assert.equal(readSpool(path), undefined, "the payload hash must be checked on automatic reads");
    assert.equal(
      loadOrCreateSpool("harness-tampered", "claude-code", "install-1"),
      undefined,
    );
    assert.equal(readFileSync(path, "utf8"), tampered, "recovery evidence must stay in place");
    rmSync(path, { force: true });
  });

  /**
   * The backlog retry reads every spool on the machine, because a conversation
   * that ended while the gateway was down has no hook of its own left to fire.
   */
  test("every spool on the machine is visible to the backlog retry", () => {
    const one = fresh("backlog-1");
    record(one, [
      { event_type: "message.user", client_event_id: "x", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    saveSpool(one);
    const two = fresh("backlog-2");
    record(two, [
      { event_type: "message.user", client_event_id: "y", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    saveSpool(two);
    const ids = allSpools().map(({ spool }) => spool.external_session_id);
    assert.ok(ids.includes("backlog-1"));
    assert.ok(ids.includes("backlog-2"));
  });

  test("a finished spool is retired and one still owing a close is not", () => {
    const done = fresh("retire-1");
    record(done, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    acknowledge(done, new Map([["a", "appended"]]));
    saveSpool(done);
    assert.equal(retireIfComplete(done), true);
    assert.equal(loadSpool("retire-1"), undefined);

    const owing = fresh("retire-2");
    record(owing, [
      { event_type: "message.user", client_event_id: "a", occurred_at: "2026-08-25T10:00:00Z", payload: {} },
    ]);
    acknowledge(owing, new Map([["a", "appended"]]));
    owing.close_requested = true;
    saveSpool(owing);
    assert.equal(retireIfComplete(owing), false);
    assert.ok(loadSpool("retire-2"));
  });

});

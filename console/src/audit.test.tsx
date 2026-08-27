import assert from "node:assert/strict";
import test from "node:test";

import { assembleAuditExport } from "./Audit.js";
import type { AuditExportPage } from "./generated/api.js";

function page(after: number): AuditExportPage {
  const common = {
    format: "synveda.audit-chain.v1",
    hash_algorithm: "blake3:synveda-audit-event-v1",
    canonicalization: "synveda.audit-event.v1",
    tenant_id: "00000000-0000-0000-0000-000000000001",
    genesis_hash: "00".repeat(32),
    snapshot_seq: 2,
    snapshot_hash: "22".repeat(32),
  };
  return after === 0
    ? {
        ...common,
        events: [event(1)],
        first_seq: 1,
        last_seq: 1,
        truncated: true,
        next_cursor: 1,
      }
    : {
        ...common,
        events: [event(2)],
        first_seq: 2,
        last_seq: 2,
        truncated: false,
      };
}

function event(seq: number) {
  return {
    seq,
    occurred_at: "2026-08-25T12:00:00Z",
    actor_kind: "subject",
    actor_subject: "auditor",
    action: "authz.decision",
    resource: "tenant 00000000-0000-0000-0000-000000000001",
    outcome: "allow",
    payload: { count: seq },
    prev_hash: seq === 1 ? "00".repeat(32) : "11".repeat(32),
    hash: seq === 1 ? "11".repeat(32) : "22".repeat(32),
  };
}

test("the console walks one frozen audit snapshot", async () => {
  const calls: Array<[number, number | undefined]> = [];
  const result = await assembleAuditExport(async (after, through) => {
    calls.push([after, through]);
    return page(after);
  });
  assert.deepEqual(calls, [
    [0, undefined],
    [1, 2],
  ]);
  assert.deepEqual(
    result.events.map((entry) => entry.seq),
    [1, 2],
  );
  assert.equal(result.snapshot_seq, 2);
  assert.ok(!("next_cursor" in result), "the downloaded document is not a cursor page");
});

test("the console refuses pages from different frozen heads", async () => {
  await assert.rejects(
    assembleAuditExport(async (after) => {
      const value = page(after);
      return after === 0 ? value : { ...value, snapshot_hash: "ff".repeat(32) };
    }),
    /changed snapshot_hash/,
  );
});

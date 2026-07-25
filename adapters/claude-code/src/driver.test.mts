/**
 * The recorded-payload driver, in the unit suite (ADR-0027 decision 14).
 *
 * The driver is the same code the demo runs against a live gateway; here
 * it runs against its mock, so every push gets the whole matrix — dead
 * gateway, 401, degraded header, oversized payload, replayed batch, cursor
 * resume, damaged transcript — without needing Postgres, an IdP, or a
 * running gateway.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { runDriver } from "./driver.mjs";

test("every recorded hook payload exits 0 against a mock gateway", async () => {
  const lines: string[] = [];
  const report = await runDriver({ report: (line) => lines.push(line) });

  assert.equal(report.failed, 0, `driver failures:\n${lines.join("\n")}`);
  // A driver that silently stopped running cases would report zero
  // failures too. The count is what makes the pass mean something.
  assert.ok(report.passed >= 14, `only ${String(report.passed)} cases ran:\n${lines.join("\n")}`);
  assert.equal(report.skipped, 0, "no case is mock-only in the mock run");
});

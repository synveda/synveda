import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const parallel = `     Running tests/authz_hierarchy.rs (/target/debug/deps/authz_hierarchy-0a1b2c3d)

running 2 tests
test committed_policy_governs_the_next_request_and_repair_is_audited ... ok
test a_failed_refresh_keeps_the_valid_compile_in_the_process_cache ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.02s

     Running tests/rls.rs (/target/debug/deps/rls-99aa)

running 2 tests
test rls_tamper_is_refused ... ignored, serial administrator tamper acceptance
test tenant_rows_are_isolated ... ok

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.20s

     Running tests/claude_lifecycle.rs (/target/debug/deps/claude_lifecycle-77)

running 1 test
test live_claude_session ... ignored, requires an authenticated Claude Code executable

test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests synveda_store

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
`;
const serial = `     Running tests/rls.rs (/target/debug/deps/rls-99aa)

running 1 test
test rls_tamper_is_refused ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.40s
`;

function summarize(log, { full = 1, required = "authz_hierarchy rls" } = {}) {
  const directory = mkdtempSync(join(tmpdir(), "synveda-db-summary-"));
  try {
    const path = join(directory, "workspace-tests.log");
    writeFileSync(path, log);
    return spawnSync(
      "awk",
      ["-v", `full=${full}`, "-v", `required=${required}`, "-f", "scripts/db-test-summary.awk", path],
      { encoding: "utf8" },
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test("the full gate reports binaries and serial executions of ignored cases", () => {
  const result = summarize(parallel + serial);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /authz_hierarchy tests\/authz_hierarchy\.rs 2\/0\/0/);
  assert.match(result.stdout, /rls tests\/rls\.rs 2\/0\/1/);
  assert.match(result.stdout, /4 binaries, 4 passed, 0 failed, 2 ignored/);
  assert.match(result.stdout, /ignored, then executed serially: rls rls_tamper_is_refused/);
  assert.match(
    result.stdout,
    /ignored and not executed here: claude_lifecycle live_claude_session \(requires an authenticated/,
  );
});

test("a serial case that never ran, a failure or a silent required suite fails", () => {
  const unexecuted = summarize(parallel);
  assert.equal(unexecuted.status, 1);
  assert.match(unexecuted.stderr, /serial case was never executed: rls rls_tamper_is_refused/);

  const failed = summarize(
    parallel.replace(
      "2 passed; 0 failed; 0 ignored",
      "1 passed; 1 failed; 0 ignored",
    ) + serial,
  );
  assert.equal(failed.status, 1);
  assert.match(failed.stderr, /records failed cases/);

  const silent = summarize(parallel + serial, { required: "authz_hierarchy access_api" });
  assert.equal(silent.status, 1);
  assert.match(silent.stderr, /required database suite executed no case: access_api/);

  const empty = summarize("db-test: no cargo output\n", { required: "" });
  assert.equal(empty.status, 1);
  assert.match(empty.stderr, /no executed test binary/);
});

test("an ignored case passing in the earlier optimized phase counts as executed", () => {
  const debug = `     Running tests/session_ingest_load.rs (/target/debug/deps/session_ingest_load-0f1e)

running 2 tests
test append_ack_sustains_1k_events_per_second ... ignored, serial MEM-1 throughput AC; the database gate runs it alone under the optimized ingest-load profile
test observe_is_idempotent ... ok

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.30s
`;
  const optimized = `     Running tests/session_ingest_load.rs (/target/ingest-load/deps/session_ingest_load-a9b8)

running 1 test
test append_ack_sustains_1k_events_per_second ... ok

successes:

---- append_ack_sustains_1k_events_per_second stdout ----
session append load: 10000 events over 10.02s (998 events/s sustained)


successes:
    append_ack_sustains_1k_events_per_second

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 12.00s
`;
  const passed = summarize(optimized + parallel + debug + serial);
  assert.equal(passed.status, 0, passed.stderr);
  assert.match(passed.stdout, /session_ingest_load tests\/session_ingest_load\.rs 2\/0\/1/);
  assert.match(
    passed.stdout,
    /ignored, then executed serially: session_ingest_load append_ack_sustains_1k_events_per_second/,
  );

  const failed = summarize(
    optimized
      .replace("_per_second ... ok", "_per_second ... FAILED")
      .replace("test result: ok. 1 passed; 0 failed", "test result: FAILED. 0 passed; 1 failed") +
      parallel + debug + serial,
  );
  assert.equal(failed.status, 1);
  assert.match(
    failed.stderr,
    /ignored, then executed in another phase and FAILED: session_ingest_load append_ack_sustains_1k_events_per_second/,
  );
  assert.doesNotMatch(failed.stderr, /never executed/);

  const skipped = summarize(parallel + debug + serial);
  assert.equal(skipped.status, 1);
  assert.match(
    skipped.stderr,
    /serial case was never executed: session_ingest_load append_ack_sustains_1k_events_per_second/,
  );
});

test("a focused invocation reports without the full-gate floor", () => {
  const result = summarize(parallel, { full: 0, required: "" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /ignored and not executed here: rls rls_tamper_is_refused/);
});

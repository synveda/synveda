import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("../demos/fixtures/ops-2/failover.sh", import.meta.url));

function fixture(t, responses, clockStep = 1) {
  const work = mkdtempSync(join(tmpdir(), "synveda-failover-"));
  t.after(() => rmSync(work, { recursive: true, force: true }));
  const calls = join(work, "calls.jsonl");
  writeFileSync(join(work, "session-id"), "pre-existing-session\n");
  writeFileSync(join(work, "responses.json"), JSON.stringify(responses));
  writeFileSync(join(work, "synveda"), `#!/bin/sh
printf '%s\\n' "$*" >> "$WORK_DIR/auth-calls"
printf '%s\\n' 'private-fixture-bearer'
`, { mode: 0o755 });
  writeFileSync(join(work, "sleep"), "#!/bin/sh\nexit 0\n", { mode: 0o755 });
  writeFileSync(join(work, "date"), `#!/bin/sh
value=1000
if [ -f "$WORK_DIR/clock" ]; then value=$(cat "$WORK_DIR/clock"); fi
printf '%s\\n' "$value"
printf '%s\\n' "$((value + ${clockStep}))" > "$WORK_DIR/clock"
`, { mode: 0o755 });
  writeFileSync(join(work, "curl"), `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
const work = process.env.WORK_DIR, calls = path.join(work, "calls.jsonl");
const count = fs.existsSync(calls) ? fs.readFileSync(calls, "utf8").trim().split("\\n").length : 0;
const args = process.argv.slice(2);
fs.appendFileSync(calls, JSON.stringify(args) + "\\n");
const responses = JSON.parse(fs.readFileSync(path.join(work, "responses.json"), "utf8"));
const response = responses[Math.min(count, responses.length - 1)];
fs.writeFileSync(args[args.indexOf("-o") + 1], response.body ?? "");
process.stdout.write(response.code);
process.exit(response.exit ?? 0);
`, { mode: 0o755 });
  return {
    run() {
      const result = spawnSync("sh", [script], {
        env: { ...process.env, PATH: `${work}${delimiter}${process.env.PATH}`, WORK_DIR: work, SYNVEDA_GATEWAY: "http://gateway.invalid:8120" },
        encoding: "utf8", timeout: 10_000,
      });
      assert.equal(result.error, undefined);
      assert.doesNotMatch(result.stdout + result.stderr, /private-fixture-bearer/);
      return { ...result, calls: readFileSync(calls, "utf8").trim().split("\n").map((line) => JSON.parse(line)), auth: readFileSync(join(work, "auth-calls"), "utf8") };
    },
  };
}

test("failover retries transport errors and incomplete responses for the same session and operation", (t) => {
  const result = fixture(t, [
    { code: "000", exit: 7 },
    { code: "200", exit: 28, body: '{"id":"partial"}' },
    { code: "503", body: '{"id":"unavailable"}' },
    { code: "200", body: "{}" },
    { code: "201", body: '{"id":"recovered-context"}' },
  ]).run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /succeeded.*attempt 5/);
  assert.equal(result.auth, "auth token\n");
  assert.equal(result.calls.length, 5);
  for (const args of result.calls) {
    assert.ok(args.includes("http://gateway.invalid:8120/v1/sessions/pre-existing-session/context-runs"));
    assert.ok(args.includes("Idempotency-Key: ops2-failover-pre-existing-session"));
    assert.equal(args[args.indexOf("--connect-timeout") + 1], "5");
    assert.equal(args[args.indexOf("--max-time") + 1], "10");
  }
});

test("failover recovery expires even when the gateway stays unreachable", (t) => {
  for (const [clockStep, maxTime] of [[120, "10"], [239, "1"]]) {
    const result = fixture(t, [{ code: "000", exit: 7 }], clockStep).run();
    assert.equal(result.status, 1);
    assert.equal(result.calls.length, 1);
    assert.equal(result.calls[0][result.calls[0].indexOf("--max-time") + 1], maxTime);
    assert.match(result.stderr, /never recovered within 240 seconds/);
    assert.doesNotMatch(result.stdout, /succeeded/);
  }
});

test("failover does not hide authentication or policy refusal behind retries", (t) => {
  for (const code of ["401", "403"]) {
    const result = fixture(t, [{ code }, { code: "201", body: '{"id":"must-not-retry"}' }]).run();
    assert.equal(result.status, 1);
    assert.equal(result.calls.length, 1);
    assert.match(result.stderr, new RegExp(`refused.*HTTP ${code}`));
  }
});

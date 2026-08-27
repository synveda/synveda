import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { validateSuite } from "./product-evaluation.mjs";

const root = resolve(import.meta.dirname, "..");
const original = JSON.parse(readFileSync(resolve(root, "evals/product/suite.json"), "utf8"));
const baseline = JSON.parse(readFileSync(resolve(root, "evals/product/baseline.json"), "utf8"));
const copy = () => structuredClone(original);

test("the committed product suite is complete", () => {
  assert.deepEqual(validateSuite(copy(), structuredClone(baseline), root), []);
});

test("dropping an outcome signal is refused", () => {
  const suite = copy();
  suite.required_measurements.pop();
  assert.match(validateSuite(suite, baseline, root).join("\n"), /eight distinct outcome signals/);
});

test("a non-zero trust tolerance is refused", () => {
  const suite = copy();
  suite.hard_gates.cross_tenant_leakage.maximum = 1;
  assert.match(validateSuite(suite, baseline, root).join("\n"), /maximum must be zero/);
});

test("a stale exact test name is refused", () => {
  const suite = copy();
  suite.scenarios[0].command[6] = "missing_test";
  assert.match(validateSuite(suite, baseline, root).join("\n"), /exact Rust test missing_test is absent/);
});

test("only LongMemEval widens the disposable service-token ceiling", () => {
  const evaluate = (actors, ttl) =>
    spawnSync(
      "sh",
      [
        "-c",
        ". evals/lib.sh; eval_service_token_ttl_for_run",
      ],
      {
        cwd: root,
        encoding: "utf8",
        env: {
          ...process.env,
          EVAL_LONGMEMEVAL_ACTORS: actors,
          ...(ttl === undefined ? {} : { EVAL_LONGMEMEVAL_TOKEN_TTL_SECS: ttl }),
        },
      },
    );

  const ordinary = evaluate("0");
  assert.equal(ordinary.status, 0);
  assert.equal(ordinary.stdout.trim(), "3600");

  const longRun = evaluate("10");
  assert.equal(longRun.status, 0);
  assert.equal(longRun.stdout.trim(), "7200");

  const explicit = evaluate("10", "5400");
  assert.equal(explicit.status, 0);
  assert.equal(explicit.stdout.trim(), "5400");

  const invalid = evaluate("10", "forever");
  assert.notEqual(invalid.status, 0);
  assert.match(invalid.stderr, /positive integer/);
});

test("the executable product gate uses the fresh-database lifecycle", () => {
  const makefile = readFileSync(resolve(root, "Makefile"), "utf8");
  const target = makefile.match(/\neval-product:\n(?<recipe>(?:\t.*\n)+)/)?.groups?.recipe ?? "";
  assert.match(target, /SYNVEDA_DB_TEST_TASK=product-evaluation/);
  assert.match(target, /bash scripts\/db-test\.sh/);
  assert.doesNotMatch(target, /node scripts\/product-evaluation\.mjs/);
});

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";

import { prepare } from "./prepare-context-model-probe.mjs";

const rubricPath = resolve("evals/product/context-model-probe.json");
const rubricBytes = readFileSync(rubricPath);
const rubric = JSON.parse(rubricBytes);
const runner = resolve("scripts/run-context-model-probe.mjs");

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

test("the bounded CLI path needs explicit approval and completes a paired mock run", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-context-model-probe-test-"));
  try {
    const inputs = ["context_optimisation", "checkpoint_restart"].map((family) => {
      const variants = family === "context_optimisation"
        ? ["off", "conservative"] : ["without_assist", "with_assist"];
      return {
        schema_version: 1,
        family,
        synthetic: true,
        encoding: "o200k_base",
        budget_tokens: 1400,
        tasks: rubric.tasks.filter((task) => task.family === family).map((task) => ({
          id: task.id,
          query: task.id,
          [variants[0]]: { rendered: `baseline ${task.id}`, tokens: 8 },
          [variants[1]]: { rendered: `candidate ${task.id}`, tokens: 8 },
        })),
      };
    });
    const manifest = {
      ...prepare(rubric, inputs),
      code_revision: "test-revision",
      code_dirty: false,
      rubric_sha256: sha256(rubricBytes),
    };
    const prompts = join(scratch, "prompts.json");
    writeFileSync(prompts, JSON.stringify(manifest));
    const answers = {};
    for (const prompt of manifest.prompts) {
      const task = rubric.tasks.find((row) => row.family === prompt.family && row.id === prompt.task_id);
      const answer = { ...task.expected };
      if (prompt.family === "checkpoint_restart" && prompt.variant === "without_assist") {
        answer[Object.keys(answer)[0]] = null;
      }
      answers[prompt.prompt_sha256] = answer;
    }
    const answerPath = join(scratch, "mock-answers.json");
    writeFileSync(answerPath, JSON.stringify(answers));

    const fakeGit = join(scratch, "git");
    writeFileSync(fakeGit, "#!/bin/sh\ncase \"$1\" in rev-parse) echo test-revision ;; status) : ;; *) exit 1 ;; esac\n");
    chmodSync(fakeGit, 0o755);
    const fakeClaude = join(scratch, "claude");
    writeFileSync(fakeClaude, `#!/usr/bin/env node
const fs = require("fs");
const crypto = require("crypto");
const args = process.argv.slice(2);
if (args[0] === "--version") { process.stdout.write("2.1.241 mock\\n"); process.exit(0); }
if (args[0] === "auth") { process.stdout.write('{"loggedIn":true}'); process.exit(0); }
const prompt = args[args.indexOf("-p") + 1];
if (!args.includes("--safe-mode") || !args.includes("--no-session-persistence")
    || args[args.indexOf("--tools") + 1] !== ""
    || args[args.indexOf("--max-budget-usd") + 1] !== "0.31") process.exit(9);
const digest = crypto.createHash("sha256").update(prompt).digest("hex");
const answer = JSON.parse(fs.readFileSync(process.env.SYNVEDA_FAKE_ANSWERS, "utf8"))[digest];
if (!answer) process.exit(10);
fs.appendFileSync(process.env.SYNVEDA_FAKE_CALL_LOG, "call\\n");
process.stdout.write(JSON.stringify({
  type: "result", subtype: "success", is_error: false,
  modelUsage: { "mock-sonnet-exact": { inputTokens: 100, outputTokens: 30 } },
  usage: { input_tokens: 100, output_tokens: 30 },
  total_cost_usd: 0.02, duration_ms: 10, result: JSON.stringify(answer),
}));
`);
    chmodSync(fakeClaude, 0o755);

    const output = join(scratch, "output");
    const callLog = join(scratch, "calls.log");
    const env = {
      ...process.env,
      PATH: `${scratch}:${process.env.PATH}`,
      SYNVEDA_CLAUDE_BIN: fakeClaude,
      SYNVEDA_FAKE_ANSWERS: answerPath,
      SYNVEDA_FAKE_CALL_LOG: callLog,
    };
    const argv = [runner, prompts, "sonnet", "5.00", output];
    const refused = spawnSync(process.execPath, argv, { env, encoding: "utf8" });
    assert.notEqual(refused.status, 0);
    assert.equal(existsSync(callLog), false);

    const approved = spawnSync(process.execPath, argv, {
      env: { ...env, SYNVEDA_CONFIRM_MODEL_SPEND: "ctx-probe:5.00:16" },
      encoding: "utf8", timeout: 30_000,
    });
    assert.equal(approved.status, 0, approved.stderr);
    assert.equal(readFileSync(callLog, "utf8").trim().split("\n").length, 16);
    const report = JSON.parse(readFileSync(join(output, "report.json"), "utf8"));
    assert.equal(report.call_count, 16);
    assert.equal(report.served_model_id, "mock-sonnet-exact");
    assert.equal(report.limits.maximum_scheduled_usd, "4.96");
    assert.deepEqual(report.quality_gates, {
      context_optimisation: "PASS",
      checkpoint_restart: "PASS",
    });
    assert.equal(report.native_compact_resume, "not measured");
    assert.equal(existsSync(join(output, "partial.json")), false);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

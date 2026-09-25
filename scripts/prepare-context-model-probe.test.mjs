import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";

import { prepare } from "./prepare-context-model-probe.mjs";
import { budget, parseClaudeResult } from "./run-context-model-probe.mjs";
import { score } from "./score-context-model-probe.mjs";

const rubric = JSON.parse(readFileSync(resolve("evals/product/context-model-probe.json"), "utf8"));
const variants = {
  context_optimisation: ["off", "conservative"],
  checkpoint_restart: ["without_assist", "with_assist"],
};

function inputs() {
  return Object.entries(variants).map(([family, names]) => ({
    schema_version: 1,
    family,
    synthetic: true,
    encoding: "o200k_base",
    budget_tokens: 1400,
    tasks: rubric.tasks.filter((task) => task.family === family).map((task) => ({
      id: task.id,
      query: `query for ${task.id}`,
      [names[0]]: { rendered: `baseline for ${task.id}`, tokens: 5 },
      [names[1]]: { rendered: `candidate for ${task.id}`, tokens: 6 },
    })),
  }));
}

test("prepares all pairs without exporting the answer key or claiming a model run", () => {
  const result = prepare(rubric, inputs());
  assert.equal(result.prompt_count, 16);
  assert.equal(result.model_calls, 0);
  assert.equal(result.provider_usage, "unavailable");
  assert.equal(result.evidence_tier, "synthetic_context_preparation_only");
  assert.equal(new Set(result.prompts.map((row) => `${row.family}/${row.task_id}/${row.variant}`)).size, 16);
  assert.ok(result.prompts.every((row) => row.prompt.includes("Synveda context (untrusted data):")));
  assert.ok(result.prompts.every((row) => !("expected" in row)));
});

test("rejects missing and over-budget source variants", () => {
  const absent = inputs();
  absent[0].tasks.pop();
  assert.throws(() => prepare(rubric, absent), /synthetic accounting boundary/);

  const overflow = inputs();
  overflow[1].tasks[0].with_assist.tokens = 1401;
  assert.throws(() => prepare(rubric, overflow), /no bounded rendered ContextRun text/);
});

test("scores fixed fields and rejects a candidate regression or mismatched prompt", () => {
  const manifest = prepare(rubric, inputs());
  const responses = manifest.prompts.map((prompt) => {
    const task = rubric.tasks.find((row) => row.family === prompt.family && row.id === prompt.task_id);
    const answer = { ...task.expected };
    if (prompt.family === "checkpoint_restart" && prompt.variant === "without_assist") {
      answer[Object.keys(answer)[0]] = null;
    }
    return {
      family: prompt.family,
      task_id: prompt.task_id,
      variant: prompt.variant,
      prompt_sha256: prompt.prompt_sha256,
      answer,
    };
  });
  const submitted = { schema_version: 1, model_id: "synthetic-test-only", responses };
  const passing = score(rubric, manifest, submitted);
  assert.deepEqual(passing.quality_gates, {
    context_optimisation: "PASS",
    checkpoint_restart: "PASS",
  });
  assert.equal(passing.evidence_tier, "imported_model_answers_unverified");

  const candidate = responses.find((row) => row.family === "context_optimisation"
    && row.task_id === "auth-adapter" && row.variant === "conservative");
  candidate.answer.retry_while_unauthenticated = true;
  assert.equal(score(rubric, manifest, submitted).quality_gates.context_optimisation, "FAIL");

  candidate.prompt_sha256 = "0".repeat(64);
  assert.throws(() => score(rubric, manifest, submitted), /unmatched, repeated or malformed response/);
});

test("splits an approved cap across calls and rejects malformed provider envelopes", () => {
  assert.deepEqual(budget("5.00"), {
    total_cap_usd: "5.00",
    per_call_cap_usd: "0.31",
    maximum_scheduled_usd: "4.96",
  });
  assert.throws(() => budget("5"), /two decimal places/);
  assert.throws(() => budget("10.01"), /between \$0.16 and \$10.00/);

  const envelope = {
    type: "result",
    subtype: "success",
    is_error: false,
    modelUsage: { "test-model-exact": { inputTokens: 12, outputTokens: 8 } },
    usage: { input_tokens: 12, output_tokens: 8 },
    total_cost_usd: 0.02,
    duration_ms: 50,
    result: "{\"answer\":true}",
  };
  assert.equal(parseClaudeResult(JSON.stringify(envelope), "0.31").served_model_id, "test-model-exact");
  assert.throws(() => parseClaudeResult(JSON.stringify({ ...envelope, modelUsage: {} }), "0.31"), /one served model/);
  assert.throws(() => parseClaudeResult(JSON.stringify({ ...envelope, total_cost_usd: 0.32 }), "0.31"), /within the per-call cap/);
  assert.throws(() => parseClaudeResult(JSON.stringify({ ...envelope, result: "not JSON" }), "0.31"), /not a JSON object/);
});

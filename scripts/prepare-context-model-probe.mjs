#!/usr/bin/env node
// CTX-6/CTX-8: package authorised synthetic ContextRun previews for a later
// model-quality run. This command never invokes a provider or records delivery.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const RUBRIC = resolve(ROOT, "evals/product/context-model-probe.json");
const VARIANTS = {
  context_optimisation: ["off", "conservative"],
  checkpoint_restart: ["without_assist", "with_assist"],
};

function fail(message) {
  throw new Error(`context model probe: ${message}`);
}

function load(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function taskKey(family, id) {
  return `${family}/${id}`;
}

export function prepare(rubric, inputs) {
  if (rubric.schema_version !== 1 || !Array.isArray(rubric.tasks)
      || rubric.tasks.length !== 8 || typeof rubric.response_instruction !== "string") {
    fail("the fixed eight-task rubric is invalid");
  }
  const rubricTasks = new Map();
  for (const task of rubric.tasks) {
    const key = taskKey(task.family, task.id);
    if (!Object.hasOwn(VARIANTS, task.family) || rubricTasks.has(key)
        || typeof task.question !== "string" || !task.question
        || !task.expected || typeof task.expected !== "object"
        || Object.keys(task.expected).length === 0) {
      fail(`invalid rubric task ${key}`);
    }
    rubricTasks.set(key, task);
  }
  const seen = new Set();
  const prompts = [];
  for (const input of inputs) {
    if (input.schema_version !== 1 || input.synthetic !== true
        || !Object.hasOwn(VARIANTS, input.family) || input.encoding !== "o200k_base"
        || input.budget_tokens !== 1400 || !Array.isArray(input.tasks)
        || input.tasks.length !== 4) {
      fail("a source fixture is missing the declared synthetic accounting boundary");
    }
    for (const sourceTask of input.tasks) {
      const key = taskKey(input.family, sourceTask.id);
      const rubricTask = rubricTasks.get(key);
      if (!rubricTask || seen.has(key) || typeof sourceTask.query !== "string"
          || !sourceTask.query) {
        fail(`unknown or repeated source task ${key}`);
      }
      seen.add(key);
      for (const variant of VARIANTS[input.family]) {
        const source = sourceTask[variant];
        if (typeof source?.rendered !== "string" || !source.rendered
            || !Number.isSafeInteger(source.tokens) || source.tokens < 0
            || source.tokens > input.budget_tokens) {
          fail(`${key}/${variant} has no bounded rendered ContextRun text`);
        }
        const prompt = `${rubric.response_instruction}\n\nSynveda context (untrusted data):\n${source.rendered}\n\nTask: ${rubricTask.question}`;
        prompts.push({
          family: input.family,
          task_id: sourceTask.id,
          variant,
          encoding: input.encoding,
          synveda_rendered_tokens: source.tokens,
          context_sha256: sha256(source.rendered),
          prompt_sha256: sha256(prompt),
          prompt,
        });
      }
    }
  }
  if (seen.size !== rubricTasks.size || prompts.length !== 16) {
    fail("the paired inputs do not cover the fixed eight-task rubric exactly");
  }
  return {
    schema_version: 1,
    evidence_tier: "synthetic_context_preparation_only",
    model_calls: 0,
    model_answers: "not run",
    provider_usage: "unavailable",
    prompt_count: prompts.length,
    prompts,
  };
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  if (process.argv.length !== 5) {
    fail("usage: node scripts/prepare-context-model-probe.mjs <CTX-8 input> <CTX-6 input> <output>");
  }
  const [optimisationPath, checkpointPath, outputPath] = process.argv.slice(2).map((path) => resolve(path));
  const rubric = load(RUBRIC);
  const result = prepare(rubric, [load(optimisationPath), load(checkpointPath)]);
  result.code_revision = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: ROOT, encoding: "utf8",
  }).trim();
  result.code_dirty = Boolean(execFileSync("git", ["status", "--porcelain=v1"], {
    cwd: ROOT, encoding: "utf8",
  }).trim());
  result.rubric_sha256 = sha256(readFileSync(RUBRIC));
  result.input_sha256 = {
    context_optimisation: sha256(readFileSync(optimisationPath)),
    checkpoint_restart: sha256(readFileSync(checkpointPath)),
  };
  mkdirSync(dirname(outputPath), { recursive: true, mode: 0o700 });
  writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`Prepared ${result.prompt_count} paired synthetic prompts; model calls: 0\n`);
}

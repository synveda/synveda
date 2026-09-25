#!/usr/bin/env node
// CTX-6/CTX-8: exact-field scoring for externally collected model answers.
// Imported answers remain unverified until a bounded provider run records
// their actual model, settings, usage and prompt receipts.

import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const RUBRIC = resolve(ROOT, "evals/product/context-model-probe.json");
const PAIRS = {
  context_optimisation: ["off", "conservative"],
  checkpoint_restart: ["without_assist", "with_assist"],
};

function fail(message) {
  throw new Error(`context model score: ${message}`);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function key(family, id, variant) {
  return `${family}/${id}/${variant}`;
}

export function score(rubric, manifest, answers) {
  if (rubric.schema_version !== 1 || manifest.schema_version !== 1
      || manifest.evidence_tier !== "synthetic_context_preparation_only"
      || manifest.model_calls !== 0 || manifest.prompt_count !== 16
      || !Array.isArray(manifest.prompts) || manifest.prompts.length !== 16
      || answers.schema_version !== 1 || !Array.isArray(answers.responses)
      || answers.responses.length !== 16 || typeof answers.model_id !== "string"
      || !answers.model_id.trim()) {
    fail("expected one fixed synthetic prompt set and 16 named model responses");
  }
  const prompts = new Map();
  for (const prompt of manifest.prompts) {
    const id = key(prompt.family, prompt.task_id, prompt.variant);
    if (prompts.has(id) || !Object.hasOwn(PAIRS, prompt.family)
        || !PAIRS[prompt.family].includes(prompt.variant)
        || sha256(prompt.prompt) !== prompt.prompt_sha256) {
      fail(`invalid or repeated prompt ${id}`);
    }
    prompts.set(id, prompt);
  }
  const responses = new Map();
  for (const response of answers.responses) {
    const id = key(response.family, response.task_id, response.variant);
    const prompt = prompts.get(id);
    if (!prompt || responses.has(id) || response.prompt_sha256 !== prompt.prompt_sha256
        || !response.answer || typeof response.answer !== "object"
        || Array.isArray(response.answer)) {
      fail(`unmatched, repeated or malformed response ${id}`);
    }
    responses.set(id, response.answer);
  }
  const tasks = [];
  const rubricKeys = new Set();
  for (const task of rubric.tasks) {
    const rubricKey = `${task.family}/${task.id}`;
    if (!Object.hasOwn(PAIRS, task.family) || rubricKeys.has(rubricKey)) {
      fail(`unknown or repeated rubric task ${rubricKey}`);
    }
    rubricKeys.add(rubricKey);
    const pair = PAIRS[task.family];
    const expectedKeys = Object.keys(task.expected).sort();
    const counts = [];
    for (const variant of pair) {
      const id = key(task.family, task.id, variant);
      const answer = responses.get(id);
      if (!answer || !prompts.has(id)
          || JSON.stringify(Object.keys(answer).sort()) !== JSON.stringify(expectedKeys)) {
        fail(`${id} must answer exactly the fixed rubric fields`);
      }
      counts.push(expectedKeys.filter((field) => Object.is(answer[field], task.expected[field])).length);
    }
    tasks.push({
      family: task.family,
      task_id: task.id,
      baseline_correct_fields: counts[0],
      candidate_correct_fields: counts[1],
      expected_fields: expectedKeys.length,
      regressed: counts[1] < counts[0],
      improved: counts[1] > counts[0],
      candidate_all_correct: counts[1] === expectedKeys.length,
    });
  }
  if (tasks.length !== 8 || responses.size !== prompts.size
      || [...prompts.values()].some((prompt) => !rubricKeys.has(`${prompt.family}/${prompt.task_id}`))) {
    fail("rubric, prompts and responses do not cover the same eight paired tasks");
  }
  const optimisation = tasks.filter((task) => task.family === "context_optimisation");
  const checkpoint = tasks.filter((task) => task.family === "checkpoint_restart");
  const optimisationPassed = optimisation.length === 4
    && optimisation.every((task) => !task.regressed && task.candidate_all_correct);
  const checkpointPassed = checkpoint.length === 4
    && checkpoint.every((task) => !task.regressed && task.candidate_all_correct)
    && checkpoint.some((task) => task.improved);
  return {
    schema_version: 1,
    evidence_tier: "imported_model_answers_unverified",
    claimed_model_id: answers.model_id,
    provider_usage: "unverified",
    task_success: "not measured",
    tasks,
    quality_gates: {
      context_optimisation: optimisationPassed ? "PASS" : "FAIL",
      checkpoint_restart: checkpointPassed ? "PASS" : "FAIL",
    },
  };
}

const invoked = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invoked) {
  if (process.argv.length !== 5) {
    fail("usage: node scripts/score-context-model-probe.mjs <prompts> <answers> <output>");
  }
  const [promptsPath, answersPath, outputPath] = process.argv.slice(2).map((path) => resolve(path));
  const rubricBytes = readFileSync(RUBRIC);
  const promptsBytes = readFileSync(promptsPath);
  const answersBytes = readFileSync(answersPath);
  const manifest = JSON.parse(promptsBytes);
  if (manifest.rubric_sha256 !== sha256(rubricBytes)) {
    fail("the answer rubric differs from the one used to prepare prompts");
  }
  const result = score(JSON.parse(rubricBytes), manifest, JSON.parse(answersBytes));
  result.rubric_sha256 = manifest.rubric_sha256;
  result.prompts_sha256 = sha256(promptsBytes);
  result.answers_sha256 = sha256(answersBytes);
  mkdirSync(dirname(outputPath), { recursive: true, mode: 0o700 });
  writeFileSync(outputPath, `${JSON.stringify(result, null, 2)}\n`, { mode: 0o600 });
  process.stdout.write(`Scored 8 paired tasks; imported answers remain unverified\n`);
}

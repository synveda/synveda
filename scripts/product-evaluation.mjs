#!/usr/bin/env node
// CPR-40: deterministic product/trust evaluation over public application
// behavior. The suite points at exact acceptance tests; the PulseBoard path
// additionally emits persisted funnel measurements from the database.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { performance } from "node:perf_hooks";
import { createHash } from "node:crypto";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const SUITE = resolve(ROOT, "evals/product/suite.json");
const BASELINE = resolve(ROOT, "evals/product/baseline.json");
const OUTPUT = resolve(ROOT, "target/product-evaluation");
const REQUIRED_SCENARIOS = [
  "capture_precision",
  "duplicate_identification",
  "cross_session_reuse",
  "cross_user_project_sharing",
  "principal_private_isolation",
  "project_isolation",
  "tenant_isolation",
  "conflict_detection",
  "supersession",
  "as_of_retrieval",
  "source_trace_completeness",
  "token_budget_selection",
  "graph_expansion",
  "versioned_activation",
  "mcp_schema_quarantine",
  "secret_safety",
  "okf_round_trip",
  "adapter_outage_recovery",
];
const REQUIRED_MEASUREMENTS = [
  "retrieved",
  "selected",
  "injected",
  "referenced_by_agent",
  "accepted_by_user",
  "helpful",
  "unhelpful",
  "caused_correction",
];
const REQUIRED_GATES = [
  "cross_tenant_leakage",
  "private_scope_leakage",
  "superseded_current_injection",
  "selected_without_provenance",
  "unversioned_skill_or_tool_activation",
  "plaintext_secret_leakage",
];

function load(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function sameMembers(actual, required) {
  return actual.length === required.length && required.every((value) => actual.includes(value));
}

export function validateSuite(suite, baseline, root = ROOT) {
  const failures = [];
  const fail = (message) => failures.push(message);
  if (suite.schema_version !== 1) fail("suite schema_version must be 1");
  if (suite.feature !== "CPR-40") fail("suite feature must be CPR-40");
  if (!sameMembers(suite.required_measurements ?? [], REQUIRED_MEASUREMENTS)) {
    fail("required_measurements must contain the eight distinct outcome signals exactly");
  }
  if (!Array.isArray(suite.scenarios)) fail("scenarios must be an array");
  const byId = new Map();
  for (const scenario of suite.scenarios ?? []) {
    if (!/^[a-z][a-z0-9_]*$/.test(scenario.id ?? "")) fail(`${scenario.id ?? "<missing>"}: invalid id`);
    if (byId.has(scenario.id)) fail(`${scenario.id}: duplicate scenario id`);
    byId.set(scenario.id, scenario);
    if (!scenario.title) fail(`${scenario.id}: title is required`);
    if (!existsSync(resolve(root, scenario.evidence ?? ""))) fail(`${scenario.id}: evidence path is missing`);
    if ((scenario.command ? 1 : 0) + (scenario.command_ref ? 1 : 0) !== 1) {
      fail(`${scenario.id}: set exactly one of command or command_ref`);
    }
    if (scenario.command && (!Array.isArray(scenario.command) || scenario.command.length < 2)) {
      fail(`${scenario.id}: command must be a non-empty argv array`);
    }
    if (!Array.isArray(scenario.measures) || scenario.measures.length === 0) {
      fail(`${scenario.id}: measures must be non-empty`);
    }
    const joined = (scenario.command ?? []).join(" ");
    if (/\/v1\/(observe|inject|recall)(?:\s|$)/.test(joined)) {
      fail(`${scenario.id}: command names a retired global runtime route`);
    }
  }
  if (!sameMembers([...byId.keys()], REQUIRED_SCENARIOS)) {
    fail("scenario inventory does not cover the required CPR-40 product cases exactly");
  }
  for (const scenario of suite.scenarios ?? []) {
    if (scenario.command_ref && !byId.has(scenario.command_ref)) {
      fail(`${scenario.id}: unknown command_ref ${scenario.command_ref}`);
    }
    if (scenario.command_ref === scenario.id) fail(`${scenario.id}: command_ref is recursive`);
    if (scenario.command?.[0] === "cargo" && scenario.command.includes("--test")) {
      const testName = scenario.command.at(-3);
      const source = readFileSync(resolve(root, scenario.evidence), "utf8");
      if (!source.includes(`fn ${testName}(`)) fail(`${scenario.id}: exact Rust test ${testName} is absent`);
    }
  }
  if (!sameMembers(Object.keys(suite.hard_gates ?? {}), REQUIRED_GATES)) {
    fail("hard_gates must contain the six zero-tolerance trust gates exactly");
  }
  for (const [name, gate] of Object.entries(suite.hard_gates ?? {})) {
    if (gate.maximum !== 0) fail(`${name}: hard-gate maximum must be zero`);
    if (!byId.has(gate.evidence_scenario)) fail(`${name}: evidence scenario is absent`);
  }
  if (baseline.schema_version !== 1) fail("baseline schema_version must be 1");
  for (const measurement of REQUIRED_MEASUREMENTS) {
    if (!(measurement in (baseline.minimum ?? {}))) fail(`baseline minimum is missing ${measurement}`);
  }
  for (const gate of REQUIRED_GATES) {
    if (baseline.maximum?.[gate] !== 0) fail(`baseline maximum for ${gate} must be zero`);
  }
  return failures;
}

function commandFor(scenario, byId) {
  let current = scenario;
  const seen = new Set();
  while (current.command_ref) {
    if (seen.has(current.id)) throw new Error(`recursive command_ref at ${current.id}`);
    seen.add(current.id);
    current = byId.get(current.command_ref);
  }
  return current.command;
}

function git(args) {
  // A baseline squash legitimately makes a pre-commit patch much larger than
  // Node's spawnSync default buffer. Dirty-tree provenance is a supported
  // evaluator mode, so hash the complete patch instead of failing at the
  // package boundary that most needs it.
  const result = spawnSync("git", args, {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.status !== 0) {
    const detail = result.error?.message ?? result.stderr?.trim();
    throw new Error(`git ${args.join(" ")} failed${detail ? `: ${detail}` : ""}`);
  }
  return result.stdout;
}

function codeProvenance() {
  const revision = git(["rev-parse", "HEAD"]).trim();
  const status = git(["status", "--porcelain=v1"]);
  if (!status.trim()) return { revision, dirty: false, worktree_patch_sha256: null };

  const digest = createHash("sha256");
  digest.update(git(["diff", "--binary", "HEAD"]));
  const untracked = git(["ls-files", "--others", "--exclude-standard", "-z"])
    .split("\0")
    .filter(Boolean)
    .sort();
  for (const relative of untracked) {
    digest.update(`\0${relative}\0`);
    digest.update(readFileSync(resolve(ROOT, relative)));
  }
  return { revision, dirty: true, worktree_patch_sha256: digest.digest("hex") };
}

function runCommand(command, env) {
  const started = performance.now();
  const result = spawnSync(command[0], command.slice(1), {
    cwd: ROOT,
    env,
    stdio: "inherit",
  });
  return { passed: result.status === 0, exit_code: result.status, wall_ms: Math.round(performance.now() - started) };
}

function finite(value, name) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error(`${name} is not finite`);
  return value;
}

function renderHuman(report) {
  const scenarioRows = report.scenarios
    .map((item) => `| ${item.id} | ${item.passed ? "PASS" : "FAIL"} | ${item.wall_ms} |`)
    .join("\n");
  const measurements = Object.entries(report.measurements)
    .map(([name, value]) => `| ${name} | ${Array.isArray(value) ? value.map((v) => v.toFixed(3)).join(", ") : value} |`)
    .join("\n");
  const gates = Object.entries(report.hard_gates)
    .map(([name, value]) => `| ${name} | ${value.measured ?? "not measured"} | ${value.passed ? "PASS" : "FAIL"} | ${value.evidence_scenario} |`)
    .join("\n");
  const codeState = report.code_dirty
    ? `dirty; patch SHA-256 \`${report.worktree_patch_sha256}\``
    : "clean";
  return `# Context-platform product evaluation\n\nRevision: \`${report.code_revision}\` (${codeState})  \nStarted: ${report.started_at}  \nResult: **${report.passed ? "PASS" : "FAIL"}**\n\nRuntime versions: model/extractor \`${report.runtime.model}\`, retrieval \`${report.runtime.retrieval_version}\`, index \`${report.runtime.index_version}\`, embedding \`${report.runtime.embedding_model ?? "none"}\`.\n\n## Scenarios\n\n| Scenario | Result | Wall ms |\n| --- | --- | ---: |\n${scenarioRows}\n\n## Separate outcome measurements\n\n| Measurement | Value |\n| --- | ---: |\n${measurements}\n\nThe five feedback rows are deliberately independent observations against one exact ContextRun selection. They do not infer “helpful” from retrieval or injection.\n\n## Zero-tolerance trust gates\n\n| Gate | Measured | Result | Evidence scenario |\n| --- | ---: | --- | --- |\n${gates}\n\n## Reproducibility\n\nThe JSON sibling is the machine-readable authority. A dirty pre-commit run records the exact worktree patch digest; checkpoint evidence is rerun from a clean feature commit. Scenario wall time includes test-process overhead and is reported, not gated. Context latency values are measured around the two public in-process ContextRun requests. The deterministic embedder is lexical-only and is not described as semantic. Model-backed extraction and BGE-M3 retrieval remain separate opt-in runs.\n`;
}

export function runEvaluation({ suitePath = SUITE, baselinePath = BASELINE, outputDir = OUTPUT } = {}) {
  const startedAt = new Date().toISOString();
  const suite = load(suitePath);
  const baseline = load(baselinePath);
  const failures = validateSuite(suite, baseline);
  if (failures.length) throw new Error(failures.join("\n"));
  if (!process.env.DATABASE_URL) throw new Error("DATABASE_URL is required; skipped database tests are not product evidence");

  mkdirSync(outputDir, { recursive: true });
  const evidencePath = resolve(outputDir, "pulseboard-evidence.json");
  const provenance = codeProvenance();
  const revision = provenance.revision;
  const byId = new Map(suite.scenarios.map((scenario) => [scenario.id, scenario]));
  const commands = new Map();
  for (const scenario of suite.scenarios) {
    const command = commandFor(scenario, byId);
    commands.set(JSON.stringify(command), command);
  }
  const commandResults = new Map();
  for (const [key, command] of commands) {
    const pulseboard = command.includes("pulseboard_cross_session_team_knowledge_loop_is_governed_end_to_end");
    const env = {
      ...process.env,
      SQLX_OFFLINE: "true",
      SYNVEDA_PRODUCT_EVAL_CODE_REVISION: revision,
      ...(pulseboard ? { SYNVEDA_PRODUCT_EVAL_EVIDENCE: evidencePath } : {}),
    };
    commandResults.set(key, runCommand(command, env));
  }
  if (!existsSync(evidencePath)) {
    throw new Error("PulseBoard emitted no evidence; a database-backed test probably skipped or failed");
  }
  const evidence = load(evidencePath);
  if (evidence.code_revision !== revision) throw new Error("PulseBoard evidence names a different code revision");
  const scenarios = suite.scenarios.map((scenario) => {
    const result = commandResults.get(JSON.stringify(commandFor(scenario, byId)));
    return { id: scenario.id, title: scenario.title, ...result };
  });
  const passedById = new Map(scenarios.map((scenario) => [scenario.id, scenario.passed]));
  const measurements = { ...evidence.measurements };
  const denominator = finite(measurements.accepted_candidates, "accepted_candidates")
    + finite(measurements.dismissed_candidates, "dismissed_candidates");
  measurements.capture_candidate_precision = denominator === 0 ? 0 : measurements.accepted_candidates / denominator;
  measurements.adapter_delivery = passedById.get("adapter_outage_recovery") ? 1 : 0;

  const observations = { ...evidence.hard_gate_observations };
  observations.cross_tenant_leakage = passedById.get("tenant_isolation") ? 0 : null;
  observations.unversioned_skill_or_tool_activation = passedById.get("versioned_activation") && passedById.get("mcp_schema_quarantine") ? 0 : null;
  observations.plaintext_secret_leakage = passedById.get("secret_safety") && observations.plaintext_sensitive_audit_leakage === 0 ? 0 : null;
  const hardGates = {};
  for (const [name, gate] of Object.entries(suite.hard_gates)) {
    const measured = observations[name] ?? null;
    hardGates[name] = {
      measured,
      maximum: gate.maximum,
      evidence_scenario: gate.evidence_scenario,
      passed: passedById.get(gate.evidence_scenario) === true && measured !== null && measured <= gate.maximum,
    };
  }
  const baselineChecks = [];
  for (const [name, minimum] of Object.entries(baseline.minimum)) {
    const measured = finite(measurements[name], name);
    baselineChecks.push({ metric: name, bound: `>= ${minimum}`, measured, passed: measured >= minimum });
  }
  for (const [name, maximum] of Object.entries(baseline.maximum)) {
    const measured = hardGates[name]?.measured;
    baselineChecks.push({ metric: name, bound: `<= ${maximum}`, measured, passed: measured !== null && measured <= maximum });
  }
  const report = {
    schema_version: 1,
    feature: suite.feature,
    started_at: startedAt,
    code_revision: revision,
    code_dirty: provenance.dirty,
    worktree_patch_sha256: provenance.worktree_patch_sha256,
    synthetic_product: suite.synthetic_product,
    runtime: {
      extractor: "deterministic-rules",
      model: evidence.model_version,
      retrieval_version: evidence.retrieval_version,
      embedding_model: evidence.embedding_model,
      index_version: evidence.index_version,
    },
    scenarios,
    measurements,
    hard_gates: hardGates,
    baseline_checks: baselineChecks,
    passed: scenarios.every((scenario) => scenario.passed)
      && Object.values(hardGates).every((gate) => gate.passed)
      && baselineChecks.every((check) => check.passed),
  };
  writeFileSync(resolve(outputDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(resolve(outputDir, "report.md"), renderHuman(report));
  return report;
}

function main() {
  const suite = load(SUITE);
  const baseline = load(BASELINE);
  const failures = validateSuite(suite, baseline);
  if (failures.length) {
    console.error("product evaluation definition failed:");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exitCode = 1;
    return;
  }
  if (process.argv.includes("--check")) {
    console.log(`product evaluation: ${suite.scenarios.length} scenarios, ${REQUIRED_MEASUREMENTS.length} outcome signals and ${REQUIRED_GATES.length} zero-tolerance gates are complete`);
    return;
  }
  try {
    const report = runEvaluation();
    console.log(`product evaluation: ${report.passed ? "PASS" : "FAIL"}; reports at ${OUTPUT}`);
    if (!report.passed) process.exitCode = 1;
  } catch (error) {
    console.error(`product evaluation failed: ${error.message}`);
    process.exitCode = 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();

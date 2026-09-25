#!/usr/bin/env node
// CPR-40: deterministic product/trust evaluation over public application
// behavior. The suite points at exact acceptance tests; the PulseBoard path
// additionally emits persisted funnel measurements from the database.

import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
  "conservative_context_compression",
  "conservative_context_paired_tasks",
  "conservative_authority_dedup",
  "checkpoint_restart_held_out_tasks",
  "checkpoint_conservative_shared_budget",
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
  const optimisationGates = suite.context_optimisation_gates ?? {};
  for (const name of ["critical_fact_loss_maximum", "private_source_leak_maximum", "obsolete_revision_served_maximum", "preview_delivery_maximum"]) {
    if (optimisationGates[name] !== 0) fail(`${name}: CTX-8 safety tolerance must be zero`);
  }
  if (optimisationGates.paired_tasks_minimum !== 4) fail("CTX-8 must compare four paired task fixtures");
  if (optimisationGates.compression_case_reduction_minimum_tokens !== 1) {
    fail("CTX-8 fixture must save at least one rendered-text token");
  }
  const checkpointGates = suite.checkpoint_restart_gates ?? {};
  if (checkpointGates.task_count_minimum !== 4
      || checkpointGates.critical_fact_loss_maximum !== 0
      || checkpointGates.provenance_failure_maximum !== 0
      || checkpointGates.assisted_p95_latency_maximum_ms !== 500) {
    fail("CTX-6 requires four tasks, zero fact/provenance loss and a predeclared 500 ms local p95 ceiling");
  }
  const checkpointCorpus = load(resolve(root, "evals/product/checkpoint-tasks.json"));
  if (checkpointCorpus.schema_version !== 1 || checkpointCorpus.encoding !== "o200k_base"
      || checkpointCorpus.budget_tokens !== 1400 || !Array.isArray(checkpointCorpus.tasks)
      || !sameMembers(checkpointCorpus.tasks.map((task) => task.id),
        ["auth-adapter", "configuration", "rust-typescript", "long-running-resume"])) {
    fail("CTX-6 held-out task corpus has changed identity or accounting boundary");
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
  const optimisation = report.context_optimisation;
  const taskRows = optimisation.paired_tasks.map((task) => `| ${task.task} | ${task.off_tokens} | ${task.conservative_tokens} | ${task.required_body_exact ? "PASS" : "FAIL"} |`).join("\n");
  const optimisationChecks = optimisation.gates.map((gate) => `| ${gate.name} | ${gate.measured} | ${gate.bound} | ${gate.passed ? "PASS" : "FAIL"} |`).join("\n");
  const checkpoint = report.checkpoint_restart;
  const checkpointRows = checkpoint.tasks.map((task) => `| ${task.task} | ${task.without_assist_tokens} | ${task.with_assist_tokens} | ${task.baseline_ms.toFixed(1)} | ${task.assisted_ms.toFixed(1)} | ${task.critical_facts_retained}/${task.critical_facts_expected} |`).join("\n");
  const checkpointChecks = checkpoint.gates.map((gate) => `| ${gate.name} | ${gate.measured} | ${gate.bound} | ${gate.passed ? "PASS" : "FAIL"} |`).join("\n");
  return `# Context-platform product evaluation\n\nRevision: \`${report.code_revision}\` (${codeState})  \nStarted: ${report.started_at}  \nResult: **${report.passed ? "PASS" : "FAIL"}**\n\nRuntime versions: model/extractor \`${report.runtime.model}\`, retrieval \`${report.runtime.retrieval_version}\`, index \`${report.runtime.index_version}\`, embedding \`${report.runtime.embedding_model ?? "none"}\`.\n\n## Scenarios\n\n| Scenario | Result | Wall ms |\n| --- | --- | ---: |\n${scenarioRows}\n\n## Separate outcome measurements\n\n| Measurement | Value |\n| --- | ---: |\n${measurements}\n\nThe five feedback rows are deliberately independent observations against one exact ContextRun selection. They do not infer “helpful” from retrieval or injection.\n\n## Zero-tolerance trust gates\n\n| Gate | Measured | Result | Evidence scenario |\n| --- | ---: | --- | --- |\n${gates}\n\n## CTX-8 paired context evidence\n\nThe compression fixture used ${optimisation.compression.off_tokens} off and ${optimisation.compression.conservative_tokens} conservative local \`${optimisation.encoding}\` rendered-text tokens. The four required-body tasks below use the same policy-visible source snapshot per pair. They establish exact fact retention, not task success. Provider usage, cache effects and monetary cost are unavailable.\n\n| Task | Off tokens | Conservative tokens | Required body exact |\n| --- | ---: | ---: | --- |\n${taskRows}\n\n| Gate | Measured | Bound | Result |\n| --- | ---: | --- | --- |\n${optimisationChecks}\n\n## CTX-6 synthetic restart probes\n\nEach pair uses the same authorised snapshot, exact \`${checkpoint.encoding}\` Synveda-rendered-text budget and two checkpoint boundaries. The unassisted preview omits Session facts; the assisted preview must retain and attribute all three independent facts plus exact required Knowledge. Both paths receive one warmup request before timing. The 500 ms local in-process p95 ceiling is a regression guard over four single samples, not a production latency objective. Task success, provider usage and monetary cost were not measured.\n\n| Task | No restart tokens | Restart tokens | No restart ms | Restart ms | Session facts retained |\n| --- | ---: | ---: | ---: | ---: | ---: |\n${checkpointRows}\n\n| Gate | Measured | Bound | Result |\n| --- | ---: | --- | --- |\n${checkpointChecks}\n\n## Reproducibility\n\nThe JSON sibling is the machine-readable authority. A dirty pre-commit run records the exact worktree patch digest; checkpoint evidence is rerun from a clean feature commit. Scenario wall time includes test-process overhead and is reported, not gated. Context latency values are measured around the public in-process preview requests. The deterministic embedder is lexical-only and is not described as semantic. Model-backed extraction and BGE-M3 retrieval remain separate opt-in runs.\n`;
}

function checkpointRestartEvidence(evidence, policy) {
  if (evidence.schema_version !== 1
      || evidence.scope !== "Synthetic held-out restart facts; no model task outcome"
      || evidence.encoding !== "o200k_base" || evidence.budget_tokens !== 1400
      || evidence.warmup_requests_per_task !== 2
      || evidence.measured_requests_per_mode_per_task !== 1
      || evidence.provider_usage !== "unavailable" || evidence.task_outcome !== "not measured"
      || evidence.cost !== "unavailable" || !Array.isArray(evidence.tasks)) {
    throw new Error("CTX-6 checkpoint evidence has an incomplete accounting boundary");
  }
  const tasks = evidence.tasks;
  if (!sameMembers(tasks.map((task) => task.task),
    ["auth-adapter", "configuration", "rust-typescript", "long-running-resume"])) {
    throw new Error("CTX-6 checkpoint evidence does not cover the four fixed tasks");
  }
  for (const task of tasks) {
    for (const field of ["without_assist_tokens", "with_assist_tokens", "baseline_ms", "assisted_ms", "critical_facts_retained", "critical_facts_expected"]) {
      if (finite(task[field], `${task.task} ${field}`) < 0) throw new Error(`${task.task} ${field} is negative`);
    }
    if (task.critical_facts_expected !== 3 || task.critical_facts_retained > 3
        || task.provider_usage !== "unavailable") {
      throw new Error(`${task.task}: CTX-6 task evidence has changed its probe boundary`);
    }
  }
  const lost = tasks.reduce((sum, task) => sum + task.critical_facts_expected - task.critical_facts_retained, 0);
  const provenanceFailures = tasks.filter((task) => !task.facts_attributed || !task.knowledge_exact
    || !task.within_budget || task.coverage !== "observed_window").length;
  const latencies = tasks.map((task) => task.assisted_ms).sort((a, b) => a - b);
  const p95 = latencies[Math.ceil(0.95 * latencies.length) - 1];
  const gates = [
    { name: "task_count", measured: tasks.length, bound: `>= ${policy.task_count_minimum}`, passed: tasks.length >= policy.task_count_minimum },
    { name: "critical_fact_loss", measured: lost, bound: `<= ${policy.critical_fact_loss_maximum}`, passed: lost <= policy.critical_fact_loss_maximum },
    { name: "provenance_or_knowledge_failure", measured: provenanceFailures, bound: `<= ${policy.provenance_failure_maximum}`, passed: provenanceFailures <= policy.provenance_failure_maximum },
    { name: "assisted_p95_latency_ms", measured: p95, bound: `<= ${policy.assisted_p95_latency_maximum_ms}`, passed: p95 <= policy.assisted_p95_latency_maximum_ms },
  ];
  return { ...evidence, assisted_p95_latency_ms: p95, gates, passed: gates.every((gate) => gate.passed) };
}

function contextOptimisationEvidence(compression, paired, policy) {
  if (compression.schema_version !== 1 || paired.schema_version !== 1
      || compression.encoding !== "o200k_base" || paired.encoding !== "o200k_base"
      || compression.scope !== "Synveda-rendered text only"
      || paired.scope !== compression.scope
      || compression.provider_usage !== "unavailable" || paired.provider_usage !== "unavailable"
      || compression.cost !== "unavailable" || paired.cost !== "unavailable"
      || !Array.isArray(paired.tasks)) {
    throw new Error("CTX-8 evidence has an incomplete accounting boundary");
  }
  const tasks = paired.tasks;
  const taskIds = new Set(tasks.map((task) => task.task));
  if (tasks.length !== 4 || !sameMembers([...taskIds], ["auth-adapter", "configuration", "rust-typescript", "multilingual"])) {
    throw new Error("CTX-8 paired tasks do not cover the required fixture identities");
  }
  for (const task of tasks) {
    finite(task.off_tokens, `${task.task} off_tokens`);
    finite(task.conservative_tokens, `${task.task} conservative_tokens`);
  }
  const saved = finite(compression.off_tokens, "CTX-8 off_tokens")
    - finite(compression.conservative_tokens, "CTX-8 conservative_tokens");
  const lost = finite(compression.required_fact_loss_count, "CTX-8 required_fact_loss_count")
    + finite(paired.critical_fact_loss_count, "CTX-8 critical_fact_loss_count")
    + tasks.filter((task) => task.required_body_exact !== true).length;
  const gates = [
    { name: "compression_fixture_reduction_tokens", measured: saved, bound: `>= ${policy.compression_case_reduction_minimum_tokens}`, passed: saved >= policy.compression_case_reduction_minimum_tokens },
    { name: "critical_fact_loss_count", measured: lost, bound: `<= ${policy.critical_fact_loss_maximum}`, passed: lost <= policy.critical_fact_loss_maximum },
    { name: "private_source_leak_count", measured: finite(compression.private_source_leak_count, "CTX-8 private_source_leak_count"), bound: `<= ${policy.private_source_leak_maximum}`, passed: compression.private_source_leak_count <= policy.private_source_leak_maximum },
    { name: "obsolete_required_revision_served_count", measured: finite(compression.obsolete_required_revision_served_count, "CTX-8 obsolete_required_revision_served_count"), bound: `<= ${policy.obsolete_revision_served_maximum}`, passed: compression.obsolete_required_revision_served_count <= policy.obsolete_revision_served_maximum },
    { name: "preview_delivery_count", measured: finite(compression.preview_delivery_count, "CTX-8 preview_delivery_count"), bound: `<= ${policy.preview_delivery_maximum}`, passed: compression.preview_delivery_count <= policy.preview_delivery_maximum },
    { name: "paired_task_count", measured: tasks.length, bound: `>= ${policy.paired_tasks_minimum}`, passed: tasks.length >= policy.paired_tasks_minimum },
    { name: "paired_token_nonincrease", measured: tasks.filter((task) => task.conservative_tokens > task.off_tokens).length, bound: "= 0", passed: tasks.every((task) => task.conservative_tokens <= task.off_tokens) },
  ];
  return {
    boundary: compression.scope,
    encoding: compression.encoding,
    provider_usage: "unavailable",
    cost: "unavailable",
    compression: { off_tokens: compression.off_tokens, conservative_tokens: compression.conservative_tokens, excerpt_selected: compression.excerpt_selected },
    paired_tasks: tasks,
    gates,
    passed: gates.every((gate) => gate.passed) && compression.excerpt_selected === true,
  };
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
  const compressionPath = resolve(outputDir, "context-compression-evidence.json");
  const pairedPath = resolve(outputDir, "context-paired-evidence.json");
  const checkpointPath = resolve(outputDir, "checkpoint-task-evidence.json");
  for (const path of [evidencePath, compressionPath, pairedPath, checkpointPath]) {
    rmSync(path, { force: true });
  }
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
    const compression = command.includes("conservative_preview_is_not_delivery_and_required_revisions_fail_closed");
    const paired = command.includes("conservative_required_fact_matrix_preserves_exact_task_evidence");
    const checkpoint = command.includes("held_out_checkpoint_restart_tasks_preserve_critical_facts_and_provenance");
    const env = {
      ...process.env,
      SQLX_OFFLINE: "true",
      SYNVEDA_PRODUCT_EVAL_CODE_REVISION: revision,
      ...(pulseboard ? { SYNVEDA_PRODUCT_EVAL_EVIDENCE: evidencePath } : {}),
      ...(compression ? { SYNVEDA_CONTEXT_OPT_COMPRESSION_EVIDENCE: compressionPath } : {}),
      ...(paired ? { SYNVEDA_CONTEXT_OPT_TASK_EVIDENCE: pairedPath } : {}),
      ...(checkpoint ? { SYNVEDA_CHECKPOINT_TASK_EVIDENCE: checkpointPath } : {}),
    };
    commandResults.set(key, runCommand(command, env));
  }
  if (!existsSync(evidencePath)) {
    throw new Error("PulseBoard emitted no evidence; a database-backed test probably skipped or failed");
  }
  if (!existsSync(compressionPath) || !existsSync(pairedPath)) {
    throw new Error("CTX-8 emitted no paired evidence; a database-backed test probably skipped or failed");
  }
  if (!existsSync(checkpointPath)) {
    throw new Error("CTX-6 emitted no checkpoint task evidence; a database-backed test probably skipped or failed");
  }
  const evidence = load(evidencePath);
  const contextOptimisation = contextOptimisationEvidence(load(compressionPath), load(pairedPath), suite.context_optimisation_gates);
  const checkpointRestart = checkpointRestartEvidence(load(checkpointPath), suite.checkpoint_restart_gates);
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
    context_optimisation: contextOptimisation,
    checkpoint_restart: checkpointRestart,
    baseline_checks: baselineChecks,
    passed: scenarios.every((scenario) => scenario.passed)
      && Object.values(hardGates).every((gate) => gate.passed)
      && baselineChecks.every((check) => check.passed)
      && contextOptimisation.passed
      && checkpointRestart.passed,
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

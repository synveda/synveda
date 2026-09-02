import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import {
  CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256,
  planControlledProviderRoot,
  probeControlledProcessGroup,
} from "../deploy/compose/scripts/clean-engine-provider-actor.mjs";
import {
  appendProviderCleanupReceiptForExecutor,
  appendReceiptForExecutor,
  executeControlledProviderCreateForExecutor,
  executeProviderCreateForExecutor,
  finalizeEnvironmentForExecutor,
  providerRecoveryConfirmationForExecutor,
  recoverControlledProviderCreateForExecutor,
  recoverProviderCreateForExecutor,
} from "../deploy/compose/scripts/clean-engine-state.mjs";
import {
  canonicalBytes,
  receiptSuccessPath,
} from "../deploy/compose/scripts/clean-engine-receipts.mjs";
import { cleanEngineReceiptResult } from "./fixtures/clean-engine-receipt-fixture.mjs";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");
const executeFixture = resolve("scripts/fixtures/execute-clean-engine-controlled-provider.mjs");
const recoverFixture = resolve("scripts/fixtures/recover-clean-engine-controlled-provider.mjs");

function command(executable, args, options = {}) {
  return spawnSync(executable, args, {
    encoding: "utf8",
    env: { LANG: "C", LC_ALL: "C", PATH: process.env.PATH },
    ...options,
  });
}

function git(repo, args) {
  const result = command("git", ["-C", repo, ...args]);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

function fixture() {
  const systemTemporary = realpathSync(process.platform === "darwin" ? "/private/tmp" : "/tmp");
  const root = realpathSync(mkdtempSync(join(systemTemporary, "s-")));
  chmodSync(root, 0o700);
  const repo = join(root, "repo");
  const state = join(root, "state");
  const providerBase = join(root, "p");
  for (const path of [repo, state, providerBase]) mkdirSync(path, { mode: 0o700 });
  const files = {
    ".dockerignore": `
.git
.git/**
.agents
.agents/**
.codex
.codex/**
.claude
.claude/**
target
target/**
**/target
**/target/**
node_modules
node_modules/**
**/node_modules
**/node_modules/**
.pnpm-store
.pnpm-store/**
**/dist
**/dist/**
**/dist-test
**/dist-test/**
data
data/**
evals/fixtures/longmemeval/longmemeval_*.json
evals/fixtures/longmemeval/LICENSE
evals/fixtures/longmemeval/LICENSE.*
.env
.env.*
**/.env
**/.env.*
!**/.env.example
deploy/compose/secrets
deploy/compose/secrets/**
deploy/compose/runtime
deploy/compose/runtime/**
deploy/compose/backups
deploy/compose/backups/**
.DS_Store
**/.DS_Store
Thumbs.db
**/Thumbs.db
*.log
**/*.log
*.tmp
**/*.tmp
`.trimStart(),
    ".gitignore": ".claude/\n.codex/\ntarget/\nnode_modules/\n",
    ".env.example": "NON_SECRET_EXAMPLE=true\n",
    "Makefile": "compose-config:\n\t@true\n",
    "deploy/compose/compose.yaml": "name: fixture\nservices: {}\n",
    "docs/DEPLOYMENT_CONTRACT.md": "# Fixture deployment contract\n",
    "docs/SECURITY.md": "# Fixture security contract\n",
    "source.txt": "fixture source\n",
  };
  for (const [path, contents] of Object.entries(files)) {
    mkdirSync(dirname(join(repo, path)), { recursive: true, mode: 0o700 });
    writeFileSync(join(repo, path), contents, { mode: 0o600 });
  }
  git(repo, ["init", "-q"]);
  git(repo, ["add", "."]);
  git(repo, [
    "-c",
    "user.name=Synveda Test",
    "-c",
    "user.email=synveda-test@example.invalid",
    "commit",
    "-q",
    "-m",
    "fixture",
  ]);
  const planned = run(stateTool, [
    "plan",
    "--repo-root",
    repo,
    "--state-base",
    state,
    "--ipv4-pool",
    "10.239.18.0/24",
    "--provider",
    "colima",
  ]);
  assert.equal(planned.status, 0, planned.stderr);
  return { providerBase, repo, root, state };
}

function run(executable, args) {
  return command(process.execPath, [executable, ...args]);
}

function stateRun(state, action) {
  return run(stateTool, [
    action,
    "--repo-root",
    state.repo,
    "--state-base",
    state.state,
  ]);
}

function activeRun(state) {
  const active = JSON.parse(readFileSync(join(state.state, "active"), "utf8"));
  return join(state.state, `.run-${active.fixture_id}`);
}

function parse(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function adapter({ scenario = "pass", deadline = 5_000, gateDelivery = "correct" } = {}) {
  return {
    after_decision_hold_milliseconds: 0,
    after_intent_hold_milliseconds: 0,
    after_outcome_publish_hold_milliseconds: 0,
    after_root_plan_hold_milliseconds: 0,
    after_settlement_hold_milliseconds: 0,
    before_decision_hold_milliseconds: 0,
    before_root_creation_hold_milliseconds: 0,
    before_root_mirror_hold_milliseconds: 0,
    before_witness_hold_milliseconds: 0,
    child_scenario: scenario,
    close_prelink_hold_milliseconds: 0,
    deadline_milliseconds: deadline,
    gate_delivery: gateDelivery,
    kill_grace_milliseconds: 1_000,
    kind: "controlled-fake-provider-v1",
    term_grace_milliseconds: 100,
  };
}

function legacyAdapter() {
  return {
    close_prelink_hold_milliseconds: 0,
    execute_outcome: "failed",
    execute_result: {},
    hold_milliseconds: 0,
    kind: "deterministic-fake-provider-v1",
    prelink_hold_milliseconds: 0,
    publication_hold_milliseconds: 0,
    reconcile_hold_milliseconds: 0,
    reconcile_outcome: "failed",
    reconcile_result: {},
  };
}

function launch(state, mode) {
  const child = spawn(
    process.execPath,
    [executeFixture, state.repo, state.state, state.providerBase, mode],
    {
      env: { LANG: "C", LC_ALL: "C", PATH: process.env.PATH },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  const closed = new Promise((resolvePromise) => {
    child.once("close", (status, signal) => resolvePromise({ signal, status, stderr, stdout }));
  });
  return { child, closed };
}

async function waitFor(path, timeout = 8_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (existsSync(path)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  assert.fail(`timed out waiting for ${path}`);
}

function pause(milliseconds) {
  return new Promise((resolvePromise) => setTimeout(resolvePromise, milliseconds));
}

async function waitForGroupAbsent(pgid, timeout = 8_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (probeControlledProcessGroup(pgid) === "absent") return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  assert.equal(probeControlledProcessGroup(pgid), "absent");
}

function rootPlanFor(state) {
  const active = parse(join(state.state, "active"));
  return planControlledProviderRoot({
    fixtureId: active.fixture_id,
    providerBase: state.providerBase,
    repoRoot: state.repo,
    stateBase: state.state,
  });
}

test("controlled provider roots require one canonical private non-overlapping base", () => {
  const state = fixture();
  try {
    const active = parse(join(state.state, "active"));
    assert.equal(rootPlanFor(state).fixture_id, active.fixture_id);
    assert.throws(
      () => planControlledProviderRoot({
        fixtureId: active.fixture_id,
        providerBase: "relative",
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /arguments were refused/,
    );
    chmodSync(state.providerBase, 0o755);
    assert.throws(() => rootPlanFor(state), /identity was refused/);
    chmodSync(state.providerBase, 0o700);
    assert.throws(
      () => planControlledProviderRoot({
        fixtureId: active.fixture_id,
        providerBase: state.state,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /overlapped protected state/,
    );
    const alias = join(state.root, "provider-alias");
    symlinkSync(state.providerBase, alias);
    assert.throws(
      () => planControlledProviderRoot({
        fixtureId: active.fixture_id,
        providerBase: alias,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /canonical absolute path/,
    );
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("the controlled fake publishes root actor outcome settlement and close evidence", async () => {
  const state = fixture();
  try {
    const receipt = await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const provider = join(active, "provider");
    const witness = parse(join(provider, "actor-witness.json"));
    const decision = parse(join(provider, "actor-decision.json"));
    const outcomePath = join(provider, "actor-outcome.json");
    const outcome = existsSync(outcomePath) ? parse(outcomePath) : undefined;
    const settlementBytes = readFileSync(join(provider, "actor-settlement.json"));
    const settlement = JSON.parse(settlementBytes);
    assert.equal(
      receipt.phase,
      "provider-create-passed",
      JSON.stringify({ outcome, settlement }),
    );
    const close = parse(join(active, ".mutation-close-00"));
    const intent = parse(join(active, "01-provider-create-intent.json"));
    assert.equal(intent.result.provider_contract_sha256, CONTROLLED_FAKE_PROVIDER_CONTRACT_SHA256);
    assert.equal(decision.decision, "start");
    assert.equal(outcome?.outcome, "passed");
    assert.equal(settlement.disposition, "completed");
    assert.equal(settlement.group_probe, "esrch");
    assert.equal(probeControlledProcessGroup(Number(witness.actor_pgid)), "absent");
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v2");
    assert.equal(close.operation_evidence_sha256, sha256(settlementBytes));
    const rootPlan = parse(join(provider, "root-plan.json"));
    assert.deepEqual(
      readFileSync(join(provider, "root-owner.json")),
      readFileSync(join(rootPlan.root_path, ".synveda-clean-engine-owner.json")),
    );
    const effect = parse(join(rootPlan.root_path, "t", "fake-effect.json"));
    const expectedEnvironmentKeys = [
      "COLIMA_CACHE_HOME",
      "COLIMA_HOME",
      "DOCKER_CONFIG",
      "LANG",
      "LC_ALL",
      "LIMA_HOME",
      "TMPDIR",
    ];
    if (process.platform === "darwin") expectedEnvironmentKeys.push("__CF_USER_TEXT_ENCODING");
    expectedEnvironmentKeys.sort();
    assert.deepEqual(effect.environment_keys, expectedEnvironmentKeys);
    assert.equal(effect.environment_keys.includes("HOME"), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a pre-intent root collision is terminal and never adopted", async () => {
  const state = fixture();
  try {
    const plan = rootPlanFor(state);
    mkdirSync(plan.root_path, { mode: 0o700 });
    writeFileSync(join(plan.root_path, "foreign"), "preserve\n", { mode: 0o600 });
    const receipt = await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(receipt.phase, "preflight-refused");
    assert.equal(readFileSync(join(plan.root_path, "foreign"), "utf8"), "preserve\n");
    assert.equal(existsSync(join(plan.root_path, ".synveda-clean-engine-owner.json")), false);
    assert.equal(existsSync(join(activeRun(state), "01-provider-create-intent.json")), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a root plan cannot be rebound to a different intended provider receipt", async () => {
  const state = fixture();
  try {
    const plan = rootPlanFor(state);
    mkdirSync(plan.root_path, { mode: 0o700 });
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const rootPlanPath = join(activeRun(state), "provider", "root-plan.json");
    const rootPlan = parse(rootPlanPath);
    rootPlan.ownership_nonce = "f".repeat(64);
    writeFileSync(rootPlanPath, canonicalBytes(rootPlan), { mode: 0o600 });
    const refused = stateRun(state, "status");
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /outside its intended receipt/);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a root collision after intent stays foreign and receives no actor", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "race-root");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "root-reservation.json"));
    const rootPlan = parse(join(provider, "root-plan.json"));
    mkdirSync(rootPlan.root_path, { mode: 0o700 });
    writeFileSync(join(rootPlan.root_path, "foreign"), "preserve\n", { mode: 0o600 });
    const closed = await execution.closed;
    assert.equal(closed.signal, null);
    assert.equal(closed.status, 0, closed.stderr);
    assert.equal(parse(join(activeRun(state), "02-provider-create-failed.json")).result.resource_disposition,
      "foreign-preserved");
    assert.equal(existsSync(join(provider, "actor-witness.json")), false);
    assert.equal(readFileSync(join(rootPlan.root_path, "foreign"), "utf8"), "preserve\n");
    assert.equal(stateRun(state, "status").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("failed timeout and orphan scenarios settle the entire actor group", async () => {
  for (const [scenario, expectedDisposition] of [
    ["fail", "completed"],
    ["hang", "terminated"],
    ["orphan", "terminated"],
  ]) {
    const state = fixture();
    try {
      const receipt = await executeControlledProviderCreateForExecutor({
        adapter: adapter({ deadline: scenario === "hang" ? 300 : 5_000, scenario }),
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      assert.equal(receipt.phase, "provider-create-failed");
      const provider = join(activeRun(state), "provider");
      const witness = parse(join(provider, "actor-witness.json"));
      const settlement = parse(join(provider, "actor-settlement.json"));
      assert.equal(settlement.disposition, expectedDisposition, scenario);
      assert.equal(settlement.group_absent, true);
      assert.equal(probeControlledProcessGroup(Number(witness.actor_pgid)), "absent");
      assert.equal(stateRun(state, "verify").status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  }
});

test("wrong and repeated gate tokens produce at most one fixed fake effect", async () => {
  for (const [gateDelivery, expectedPhase, expectedEffects] of [
    ["wrong", "provider-create-failed", 0],
    ["duplicate", "provider-create-passed", 1],
  ]) {
    const state = fixture();
    try {
      const receipt = await executeControlledProviderCreateForExecutor({
        adapter: adapter({ gateDelivery }),
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      assert.equal(receipt.phase, expectedPhase, gateDelivery);
      assert.equal(
        readdirCount(rootPlanFor(state).root_path, "fake-effect.json"),
        expectedEffects,
        gateDelivery,
      );
      const witness = parse(join(activeRun(state), "provider", "actor-witness.json"));
      assert.equal(probeControlledProcessGroup(Number(witness.actor_pgid)), "absent");
      assert.equal(stateRun(state, "verify").status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  }
});

test("supervisor death before the decision publishes a recovered abort without an effect", async () => {
  for (const signal of ["SIGKILL", "SIGTERM"]) {
    const state = fixture();
    try {
      const execution = launch(state, "hold-before-decision");
      const provider = join(activeRun(state), "provider");
      await waitFor(join(provider, "actor-witness.json"));
      const witness = parse(join(provider, "actor-witness.json"));
      assert.equal(existsSync(join(provider, "actor-decision.json")), false);
      assert.equal(existsSync(join(rootPlanFor(state).root_path, "t", "fake-effect.json")), false);
      process.kill(execution.child.pid, signal);
      const killed = await execution.closed;
      assert.equal(killed.signal, signal);
      await waitForGroupAbsent(Number(witness.actor_pgid));
      const confirmation = providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      });
      const recovered = await recoverControlledProviderCreateForExecutor({
        confirmation,
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      assert.equal(recovered.phase, "provider-create-failed");
      assert.equal(parse(join(provider, "actor-decision.json")).decision, "abort");
      assert.equal(parse(join(provider, "actor-settlement.json")).disposition, "aborted");
      assert.equal(existsSync(join(rootPlanFor(state).root_path, "t", "fake-effect.json")), false);
      assert.equal(stateRun(state, "verify").status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  }
});

test("a marker-to-mirror crash converges only the exact reserved owner", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-before-root-mirror");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "root-reservation.json"));
    const rootPlan = parse(join(provider, "root-plan.json"));
    const marker = join(rootPlan.root_path, ".synveda-clean-engine-owner.json");
    await waitFor(marker);
    assert.equal(existsSync(join(provider, "root-owner.json")), false);
    assert.equal(existsSync(join(provider, "actor-launch.json")), false);
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovery = run(recoverFixture, [
      state.repo,
      state.state,
      state.providerBase,
      confirmation,
    ]);
    assert.equal(recovery.status, 0, recovery.stderr);
    const recovered = parse(join(activeRun(state), "02-provider-create-failed.json"));
    assert.equal(recovered.phase, "provider-create-failed");
    assert.deepEqual(
      readFileSync(join(provider, "root-owner.json")),
      readFileSync(marker),
    );
    assert.equal(existsSync(join(provider, "actor-launch.json")), false);
    assert.equal(existsSync(join(rootPlan.root_path, "t", "fake-effect.json")), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery closes an intent published before its root reservation", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-after-intent");
    const active = activeRun(state);
    const provider = join(active, "provider");
    await waitFor(join(active, "01-provider-create-intent.json"));
    assert.equal(existsSync(join(provider, "root-reservation.json")), false);
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.equal(recovered.result.safe_code, "evidence-refused");
    assert.equal(existsSync(join(provider, "root-reservation.json")), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a terminal foreign collision recovers without adopting a later root", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-root-collision-close");
    const active = activeRun(state);
    const provider = join(active, "provider");
    await waitFor(join(provider, "root-reservation.json"));
    const rootPlan = parse(join(provider, "root-plan.json"));
    mkdirSync(rootPlan.root_path, { mode: 0o700 });
    writeFileSync(join(rootPlan.root_path, "foreign"), "preserve\n", { mode: 0o600 });
    await waitFor(join(active, "02-provider-create-failed.json"));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    rmSync(rootPlan.root_path, { force: false, recursive: true });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.equal(recovered.result.safe_code, "resource-collision");
    assert.equal(existsSync(join(provider, "root-owner.json")), false);
    assert.equal(existsSync(rootPlan.root_path), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("controlled evidence cannot be recovered or reused by the legacy provider path", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-after-root-plan");
    const active = activeRun(state);
    await waitFor(join(active, "provider", "root-plan.json"));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: legacyAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /dedicated executor/,
    );
    assert.equal(
      readdirSync(active).some((name) => name.startsWith(".mutation-recovery-")),
      false,
    );
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "plan");
    assert.throws(
      () => executeProviderCreateForExecutor({
        adapter: legacyAdapter(),
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider creation state was refused/,
    );
    assert.equal(existsSync(join(active, ".mutation-slot-01")), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a durable launch without a witness remains an explicit recovery blocker", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-before-witness");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-launch.json"));
    assert.equal(existsSync(join(provider, "actor-witness.json")), false);
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    await pause(200);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    await assert.rejects(
      recoverControlledProviderCreateForExecutor({
        confirmation,
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /launch state remained uncertain/,
    );
    assert.equal(existsSync(join(provider, "actor-settlement.json")), false);
    assert.equal(existsSync(join(rootPlanFor(state).root_path, "t", "fake-effect.json")), false);
    assert.equal(stateRun(state, "status").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a durable start is never replayed and recovery can use its exact outcome", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-after-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-outcome.json"));
    const witness = parse(join(provider, "actor-witness.json"));
    assert.equal(parse(join(provider, "actor-decision.json")).decision, "start");
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    await waitForGroupAbsent(Number(witness.actor_pgid));
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-passed");
    assert.equal(parse(join(provider, "actor-settlement.json")).termination_reason, "recovered");
    assert.equal(readdirCount(rootPlanFor(state).root_path, "fake-effect.json"), 1);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery never converts an orphan outcome into provider success", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-orphan-after-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-outcome.json"));
    const witness = parse(join(provider, "actor-witness.json"));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    await waitForGroupAbsent(Number(witness.actor_pgid));
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.equal(parse(join(provider, "actor-settlement.json")).disposition, "terminated");
    assert.equal(readdirCount(rootPlanFor(state).root_path, "fake-effect.json"), 1);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

function readdirCount(rootPath, name) {
  return existsSync(join(rootPath, "t", name)) ? 1 : 0;
}

test("a crash after settlement recovers result publication without rerunning the child", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-after-settlement");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-settlement.json"));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-passed");
    assert.equal(readdirCount(rootPlanFor(state).root_path, "fake-effect.json"), 1);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a live started group blocks recovery until its disconnect watchdog settles it", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "kill-hang-after-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(rootPlanFor(state).root_path, "t", "fake-effect.json"));
    const witness = parse(join(provider, "actor-witness.json"));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    await assert.rejects(
      recoverControlledProviderCreateForExecutor({
        confirmation,
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /group remained present or unknown/,
    );
    await waitForGroupAbsent(Number(witness.actor_pgid));
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("actor SIGKILL closes its IPC-owned hanging child before recovery", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "kill-hang-after-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(rootPlanFor(state).root_path, "t", "fake-effect.json"));
    const witness = parse(join(provider, "actor-witness.json"));
    process.kill(Number(witness.actor_pid), "SIGKILL");
    await waitForGroupAbsent(Number(witness.actor_pgid));
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    const settlement = parse(join(provider, "actor-settlement.json"));
    assert.equal(settlement.termination_reason, "recovered");
    assert.notEqual(settlement.actor_effect_sha256, "0".repeat(64));
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a durable outcome wins a delayed acknowledgement and deadline race", async () => {
  const state = fixture();
  try {
    const delayed = adapter({ deadline: 300 });
    delayed.after_outcome_publish_hold_milliseconds = 1_000;
    const receipt = await executeControlledProviderCreateForExecutor({
      adapter: delayed,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(receipt.phase, "provider-create-passed");
    const settlement = parse(join(activeRun(state), "provider", "actor-settlement.json"));
    assert.equal(settlement.disposition, "completed");
    assert.equal(settlement.termination_reason, "none");
    assert.notEqual(settlement.actor_effect_sha256, "0".repeat(64));
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("source drift before the gate aborts without starting the child", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "race-before-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-witness.json"));
    writeFileSync(join(state.repo, "source.txt"), "drifted\n", { mode: 0o600 });
    const closed = await execution.closed;
    assert.equal(closed.status, 0, closed.stderr);
    assert.equal(parse(join(provider, "actor-decision.json")).decision, "abort");
    assert.equal(existsSync(join(rootPlanFor(state).root_path, "t", "fake-effect.json")), false);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n", { mode: 0o600 });
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("source drift after the start decision records a failed owned effect", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "race-after-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-decision.json"));
    assert.equal(parse(join(provider, "actor-decision.json")).decision, "start");
    writeFileSync(join(state.repo, "source.txt"), "drifted\n", { mode: 0o600 });
    const closed = await execution.closed;
    assert.equal(closed.status, 0, closed.stderr);
    assert.equal(parse(join(activeRun(state), "02-provider-create-failed.json")).phase,
      "provider-create-failed");
    assert.equal(readdirCount(rootPlanFor(state).root_path, "fake-effect.json"), 1);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n", { mode: 0o600 });
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery converts a durable controlled pass plus source drift to execution failure", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "hold-passed-close");
    const active = activeRun(state);
    await waitFor(join(active, "02-provider-create-passed.json"));
    writeFileSync(join(state.repo, "source.txt"), "drifted\n", { mode: 0o600 });
    process.kill(execution.child.pid, "SIGKILL");
    assert.equal((await execution.closed).signal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "execution-failed");
    assert.equal(recovered.sequence, 3);
    assert.equal(recovered.result.safe_code, "evidence-refused");
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n", { mode: 0o600 });
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("root identity drift at the start gate produces no provider effect", async () => {
  const state = fixture();
  try {
    const execution = launch(state, "race-before-decision");
    const provider = join(activeRun(state), "provider");
    await waitFor(join(provider, "actor-witness.json"));
    const witness = parse(join(provider, "actor-witness.json"));
    const root = rootPlanFor(state).root_path;
    chmodSync(join(root, "c"), 0o755);
    const closed = await execution.closed;
    assert.equal(closed.status, 78, closed.stderr);
    assert.equal(existsSync(join(root, "t", "fake-effect.json")), false);
    chmodSync(join(root, "c"), 0o700);
    await waitForGroupAbsent(Number(witness.actor_pgid));
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = await recoverControlledProviderCreateForExecutor({
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.equal(parse(join(provider, "actor-settlement.json")).disposition, "aborted");
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("controlled roots cannot use synthetic cleanup or finalization evidence", async () => {
  const state = fixture();
  try {
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const candidate = parse(join(activeRun(state), "candidate.json"));
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "provider-cleanup-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("provider-cleanup-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /dedicated mutation executor/,
    );
    for (const phase of receiptSuccessPath.slice(3, 13)) {
      appendReceiptForExecutor({
        phase,
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(phase, candidate.run_id),
        stateBase: state.state,
      });
    }
    assert.throws(
      () => appendProviderCleanupReceiptForExecutor({
        phase: "provider-cleanup-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("provider-cleanup-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /dedicated ownership evidence/,
    );
    assert.throws(
      () => finalizeEnvironmentForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /cleanup must complete before finalization/,
    );
    assert.equal(existsSync(join(activeRun(state), "13-provider-cleanup-intent.json")), false);
    assert.equal(existsSync(join(activeRun(state), "environment.json")), false);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("more than one fake-effect publication stage is refused as replay evidence", async () => {
  const state = fixture();
  try {
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const temporary = join(rootPlanFor(state).root_path, "t");
    const first = join(temporary, `.fake-effect-stage-${"a".repeat(32)}`);
    writeFileSync(first, "{}\n", {
      mode: 0o600,
    });
    assert.equal(stateRun(state, "status").status, 78);
    writeFileSync(join(temporary, `.fake-effect-stage-${"b".repeat(32)}`), "{}\n", {
      mode: 0o600,
    });
    assert.equal(stateRun(state, "status").status, 78);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("terminal success is bound to the exact completed settlement", async () => {
  for (const mutation of ["close-evidence", "engine-digest", "settlement-matrix"]) {
    const state = fixture();
    try {
      await executeControlledProviderCreateForExecutor({
        adapter: adapter(),
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      const active = activeRun(state);
      const provider = join(active, "provider");
      const settlementPath = join(provider, "actor-settlement.json");
      const receiptPath = join(active, "02-provider-create-passed.json");
      const closePath = join(active, ".mutation-close-00");
      const settlement = parse(settlementPath);
      const receipt = parse(receiptPath);
      const close = parse(closePath);
      if (mutation === "close-evidence") {
        close.operation_evidence_sha256 = "0".repeat(64);
      } else if (mutation === "settlement-matrix") {
        settlement.disposition = "terminated";
        const settlementBytes = canonicalBytes(settlement);
        writeFileSync(settlementPath, settlementBytes, { mode: 0o600 });
        const settlementSha256 = sha256(settlementBytes);
        receipt.result.engine_identity_sha256 = settlementSha256;
        close.operation_evidence_sha256 = settlementSha256;
      } else {
        receipt.result.engine_identity_sha256 = "f".repeat(64);
      }
      const receiptBytes = canonicalBytes(receipt);
      writeFileSync(receiptPath, receiptBytes, { mode: 0o600 });
      close.result_head_sha256 = sha256(receiptBytes);
      writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      const refused = stateRun(state, "status");
      assert.equal(refused.status, 78, mutation);
      assert.match(
        refused.stderr,
        mutation === "settlement-matrix"
          ? /actor settlement was refused/
          : mutation === "close-evidence"
            ? /close evidence was refused|operation evidence was refused/
            : /passing terminal evidence was refused/,
      );
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  }
});

test("provider artifact crash stages are inert or exact hard-link aliases", async () => {
  const state = fixture();
  try {
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const provider = join(activeRun(state), "provider");
    const stage = join(provider, `.artifact-stage-${"c".repeat(32)}`);
    writeFileSync(stage, "{", { mode: 0o600 });
    assert.equal(stateRun(state, "status").status, 0);
    unlinkSync(stage);
    linkSync(join(provider, "actor-settlement.json"), stage);
    assert.equal(stateRun(state, "status").status, 0);
    unlinkSync(stage);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a failed receipt cannot relabel a completed passing settlement", async () => {
  const state = fixture();
  try {
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const passedPath = join(active, "02-provider-create-passed.json");
    const failedPath = join(active, "02-provider-create-failed.json");
    const receipt = parse(passedPath);
    receipt.outcome = "failed";
    receipt.phase = "provider-create-failed";
    receipt.result = {
      cleanup_required: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "child-failed",
    };
    const receiptBytes = canonicalBytes(receipt);
    writeFileSync(passedPath, receiptBytes, { mode: 0o600 });
    renameSync(passedPath, failedPath);
    const closePath = join(active, ".mutation-close-00");
    const close = parse(closePath);
    close.result_head_sha256 = sha256(receiptBytes);
    writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
    const refused = stateRun(state, "status");
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /failing terminal evidence was refused/);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("root marker links and environment-root mode drift fail closed", async () => {
  const state = fixture();
  try {
    await executeControlledProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const plan = rootPlanFor(state);
    const marker = join(plan.root_path, ".synveda-clean-engine-owner.json");
    const alias = join(state.providerBase, "foreign-marker-link");
    linkSync(marker, alias);
    assert.equal(stateRun(state, "status").status, 78);
    unlinkSync(alias);
    assert.equal(stateRun(state, "status").status, 0);
    chmodSync(join(plan.root_path, "c"), 0o755);
    assert.equal(stateRun(state, "status").status, 78);
    chmodSync(join(plan.root_path, "c"), 0o700);
    assert.equal(stateRun(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

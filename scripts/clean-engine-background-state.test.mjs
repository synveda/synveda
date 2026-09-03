#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import {
  chmodSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  renameSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import test from "node:test";
import {
  appendReceiptForExecutor,
  backgroundProviderCleanupRecoveryConfirmationForExecutor,
  executeBackgroundProviderCreateForExecutor,
  executeBackgroundProviderCleanupForExecutor,
  executeProviderCreateForExecutor,
  finalizeEnvironmentForExecutor,
  providerRecoveryConfirmationForExecutor,
  recoverBackgroundProviderCreateForExecutor,
  recoverBackgroundProviderCleanupForExecutor,
  recoverProviderCreateForExecutor,
} from "../deploy/compose/scripts/clean-engine-state.mjs";
import { canonicalBytes } from "../deploy/compose/scripts/clean-engine-receipts.mjs";
import {
  CONTROLLED_BACKGROUND_PROVIDER_CONTRACT,
  controlledBackgroundOperationEvidence,
  inspectControlledBackgroundProvider,
  inspectControlledBackgroundProviderPrefix,
  inspectControlledBackgroundRetirementPrefix,
  launchControlledBackgroundProvider,
  launchControlledBackgroundProviderWithAuthorityGate,
  planControlledBackgroundProviderCreateWithAuthorityGate,
  planControlledBackgroundRetirement,
  planControlledBackgroundRetirementWithAuthorityGate,
  retireControlledBackgroundProvider,
} from "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");
const executeFixture = resolve("scripts/fixtures/execute-clean-engine-background-provider.mjs");
const cleanupFixture = resolve(
  "scripts/fixtures/execute-clean-engine-background-cleanup.mjs",
);
const cleanupRecoveryFixture = resolve(
  "scripts/fixtures/recover-clean-engine-background-cleanup.mjs",
);

function command(executable, args, options = {}) {
  return spawnSync(executable, args, {
    encoding: "utf8",
    env: { HOME: process.env.HOME, LANG: "C", LC_ALL: "C", PATH: process.env.PATH },
    ...options,
  });
}

function git(repo, args) {
  const result = command("git", ["-C", repo, ...args]);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

function fixture() {
  const temporary = realpathSync(process.platform === "darwin" ? "/private/tmp" : "/tmp");
  const root = realpathSync(mkdtempSync(join(temporary, "s-bg-")));
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
    Makefile: "compose-config:\n\t@true\n",
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
  const planned = command(process.execPath, [
    stateTool,
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

function parse(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function activeRun(state) {
  const active = parse(join(state.state, "active"));
  return join(state.state, `.run-${active.fixture_id}`);
}

function adapter(overrides = {}) {
  return {
    after_authority_hold_milliseconds: 0,
    after_evidence_hold_milliseconds: 0,
    after_intent_hold_milliseconds: 0,
    after_result_hold_milliseconds: 0,
    after_settlement_hold_milliseconds: 0,
    before_detach_hold_milliseconds: 0,
    before_identity_probe_hold_milliseconds: 0,
    before_start_decision_hold_milliseconds: 0,
    before_start_hold_milliseconds: 0,
    kind: "controlled-background-provider-v1",
    maximum_lifetime_milliseconds: 5_000,
    ...overrides,
  };
}

function cleanupAdapter(overrides = {}) {
  return {
    after_claim_hold_milliseconds: 0,
    after_intent_hold_milliseconds: 0,
    after_plan_hold_milliseconds: 0,
    after_result_hold_milliseconds: 0,
    after_retirement_hold_milliseconds: 0,
    after_settlement_hold_milliseconds: 0,
    after_slot_hold_milliseconds: 0,
    before_close_hold_milliseconds: 0,
    close_prelink_hold_milliseconds: 0,
    crash_after_delete_sequence: null,
    crash_after_delete_syscall_sequence: null,
    crash_after_hostagent_settlement: false,
    kind: "controlled-background-provider-cleanup-v1",
    stop_after_sequence: null,
    ...overrides,
  };
}

function cleanupProviderAdapter() {
  return adapter({ maximum_lifetime_milliseconds: 30_000 });
}

function continuationResult(phase, fixtureId) {
  const values = {
    "builder-canary-intent": {
      builder: `synveda-cpr45-canary-${fixtureId.slice(0, 16)}`,
      canonical_builder: "default",
      endpoint: "loopback-inert-tcp",
      expected_connections: 0,
    },
    "builder-canary-passed": {
      canonical_builder_driver: "docker",
      canonical_builder_endpoint: "default",
      connections: 0,
      private_buildx_removed: true,
    },
    "compose-browser-intent": {
      capture: "disabled",
      profiles: ["browser-acceptance", "demo"],
      project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
    },
    "compose-browser-passed": {
      admin_admitted: true,
      browser_exit: 0,
      captured_artifacts: 0,
      container_proxy_empty_variables: 10,
      logout: true,
      pkce_s256: true,
    },
    "project-cleanup-intent": {
      project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
      resolver: "managed-test-block",
      scope: "exact-receipt-owned-only",
    },
    "project-cleanup-passed": {
      builder_canary_absent: true,
      project_absent: true,
      registry_absent: true,
      resolver_absent: true,
      runtime_secrets_absent: true,
    },
    "proxy-intent": {
      config: "synthetic-nonsecret-v1",
      expected_injected_variables: 10,
      expected_runtime_empty_variables: 10,
    },
    "proxy-passed": {
      auth_preserved: true,
      injected_variables: 10,
      runtime_empty_variables: 10,
    },
    "registry-intent": {
      authentication: "basic-bcrypt-cost-12",
      container: `synveda-cpr45-registry-${fixtureId.slice(0, 16)}`,
      image:
        "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33",
      port: 54_321,
      transport: "tls-loopback",
    },
    "registry-passed": {
      authenticated_pull: true,
      authenticated_push: true,
      basic_challenge: true,
      canary_image_sha256: "4".repeat(64),
      certificate_sha256: "5".repeat(64),
      negative_status: 401,
      unauthenticated_pull_rejected: true,
      wrong_password_rejected: true,
    },
  };
  return values[phase];
}

function continueThroughProjectCleanup(state) {
  const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
  for (const phase of [
    "registry-intent",
    "registry-passed",
    "proxy-intent",
    "proxy-passed",
    "builder-canary-intent",
    "builder-canary-passed",
    "compose-browser-intent",
    "compose-browser-passed",
    "project-cleanup-intent",
    "project-cleanup-passed",
  ]) {
    appendReceiptForExecutor({
      phase,
      repoRoot: state.repo,
      result: continuationResult(phase, fixtureId),
      stateBase: state.state,
    });
  }
}

function deterministicAdapter() {
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

function stateCreateBindings(slotBytes, slot) {
  return {
    create_intent_sha256: slot.intent_receipt_sha256,
    create_slot_sequence: slot.journal_sequence,
    create_slot_sha256: sha256(slotBytes),
    ownership_nonce: slot.operation_plan.ownership_nonce,
    source_head_sha256: slot.source_head_sha256,
    source_sequence: slot.source_sequence,
    state_integration: "mutation-journal-v2",
  };
}

function verify(state) {
  return command(process.execPath, [
    stateTool,
    "verify",
    "--repo-root",
    state.repo,
    "--state-base",
    state.state,
  ]);
}

function launchOwner(state, mode) {
  const child = spawn(
    process.execPath,
    [executeFixture, state.repo, state.state, state.providerBase, mode],
    {
      env: {
        HOME: process.env.HOME,
        LANG: "C",
        LC_ALL: "C",
        PATH: process.env.PATH,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  return { child, closed: once(child, "close"), stderr: () => stderr };
}

function launchCleanupOwner(state, mode) {
  const child = spawn(
    process.execPath,
    [cleanupFixture, state.repo, state.state, mode],
    {
      env: {
        HOME: process.env.HOME,
        LANG: "C",
        LC_ALL: "C",
        PATH: process.env.PATH,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  return { child, closed: once(child, "close"), stderr: () => stderr };
}

function launchCleanupRecovery(state, confirmation, mode) {
  const child = spawn(
    process.execPath,
    [cleanupRecoveryFixture, state.repo, state.state, confirmation, mode],
    {
      env: {
        HOME: process.env.HOME,
        LANG: "C",
        LC_ALL: "C",
        PATH: process.env.PATH,
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  return { child, closed: once(child, "close"), stderr: () => stderr };
}

async function waitFor(path, timeoutMilliseconds = 12_000, owner) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (!existsSync(path) || lstatSync(path, { bigint: true }).nlink !== 1n) {
    if (owner !== undefined && owner.child.exitCode !== null) {
      assert.fail(`background owner exited before ${path}: ${owner.stderr()}`);
    }
    assert.ok(Date.now() < deadline, `timed out waiting for ${path}`);
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
}

async function waitForMutationStage(run, owner, timeoutMilliseconds = 12_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    const name = readdirSync(run).find((candidate) =>
      candidate.startsWith(".mutation-stage-"));
    if (
      name !== undefined &&
      lstatSync(join(run, name), { bigint: true }).nlink === 1n
    ) {
      return join(run, name);
    }
    if (owner.child.exitCode !== null) {
      assert.fail(`background owner exited before mutation staging: ${owner.stderr()}`);
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  assert.fail("timed out waiting for mutation staging");
}

async function killOwner(owner) {
  process.kill(owner.child.pid, "SIGKILL");
  const [status, signal] = await owner.closed;
  assert.equal(status, null, owner.stderr());
  assert.equal(signal, "SIGKILL", owner.stderr());
}

async function expectOwnerSuccess(owner) {
  const [status, signal] = await owner.closed;
  assert.equal(status, 0, owner.stderr());
  assert.equal(signal, null, owner.stderr());
}

function evidenceSnapshot(state) {
  const directory = join(activeRun(state), "provider");
  return readdirSync(directory).sort().map((name) => ({
    name,
    sha256: sha256(readFileSync(join(directory, name))),
  }));
}

function replaceStaticFileWithSameBytes(state, active) {
  const identity = parse(join(active, "provider", "provider-identity.json"));
  const entry = identity.provider_root_inventory.find(
    (candidate) =>
      candidate.kind === "file" && candidate.relative_path.endsWith("basedisk"),
  );
  assert.notEqual(entry, undefined);
  const path = join(identity.provider_root.path, entry.relative_path);
  const bytes = readFileSync(path);
  const before = lstatSync(path, { bigint: true });
  const displaced = join(state.root, `displaced-${sha256(bytes)}.json`);
  const replacement = `${path}.replacement`;
  writeFileSync(replacement, bytes, { mode: 0o600 });
  linkSync(path, displaced);
  renameSync(replacement, path);
  const after = lstatSync(path, { bigint: true });
  assert.equal(after.dev, before.dev);
  assert.notEqual(after.ino, before.ino);
  assert.equal(after.mode & 0o7777n, before.mode & 0o7777n);
  assert.deepEqual(readFileSync(path), bytes);
  return path;
}

async function waitForHostagentExit(slot) {
  const pidPath = join(
    slot.operation_plan.provider_root_path,
    "l",
    `colima-${slot.operation_plan.provider_profile}`,
    "ha.pid",
  );
  if (!existsSync(pidPath)) return;
  const pid = parse(pidPath).pid;
  const deadline = Date.now() + 8_000;
  while (Date.now() < deadline) {
    try {
      process.kill(pid, 0);
    } catch (error) {
      if (error?.code === "ESRCH") return;
      throw error;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 25));
  }
  assert.fail(`background hostagent ${pid} did not exit`);
}

function recoveryArguments(state) {
  return {
    adapter: adapter(),
    confirmation: providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    }),
    providerBase: state.providerBase,
    repoRoot: state.repo,
    stateBase: state.state,
  };
}

function cleanupRecoveryArguments(state) {
  return {
    adapter: cleanupAdapter(),
    confirmation: backgroundProviderCleanupRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    }),
    repoRoot: state.repo,
    stateBase: state.state,
  };
}

test("the background owner binds one operation through settlement receipt and close", async () => {
  const state = fixture();
  try {
    const receipt = await executeBackgroundProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const slotBytes = readFileSync(join(active, ".mutation-slot-00"));
    const slot = JSON.parse(slotBytes);
    const intent = parse(join(active, "01-provider-create-intent.json"));
    const settlementBytes = readFileSync(join(active, ".mutation-operation-00"));
    const settlement = JSON.parse(settlementBytes);
    const close = parse(join(active, ".mutation-close-00"));
    const authority = parse(join(active, "provider", "background-create-authority.json"));
    assert.equal(receipt.phase, "provider-create-passed");
    assert.equal(slot.schema, "synveda.clean-engine.mutation-slot.v2");
    assert.equal(intent.schema, "synveda.clean-engine.receipt.v4");
    assert.equal(settlement.authority, "owner");
    assert.equal(settlement.authority_sha256, sha256(slotBytes));
    assert.equal(settlement.disposition, "complete-identity");
    assert.equal(receipt.result.operation_evidence_sha256, sha256(settlementBytes));
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v3");
    assert.equal(close.operation_evidence_sha256, sha256(settlementBytes));
    assert.equal(
      intent.result.operation_plan_sha256,
      sha256(canonicalBytes(slot.operation_plan)),
    );
    assert.equal(slot.intent_receipt_sha256, sha256(canonicalBytes(intent)));
    assert.equal(authority.create_intent_sha256, slot.intent_receipt_sha256);
    assert.equal(authority.create_slot_sequence, slot.journal_sequence);
    assert.equal(authority.create_slot_sha256, sha256(slotBytes));
    assert.equal(authority.ownership_nonce, slot.operation_plan.ownership_nonce);
    for (const value of [intent.result, receipt.result, settlement, close]) {
      assert.equal(value.operation_kind, slot.operation_kind);
      assert.equal(value.operation_plan_sha256, intent.result.operation_plan_sha256);
      assert.equal(value.provider_contract_sha256 ?? value.operation_contract_sha256,
        slot.operation_contract_sha256);
    }
    const verifiedCreate = verify(state);
    assert.equal(verifiedCreate.status, 0, verifiedCreate.stderr);
    await waitForHostagentExit(slot);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup receipt and close refuse inner or unrelated operation evidence", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    await executeBackgroundProviderCleanupForExecutor({
      adapter: cleanupAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const cleanupSlotName = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-slot-"))
      .sort()
      .at(-1);
    const sequence = cleanupSlotName.slice(-2);
    const slotPath = join(active, cleanupSlotName);
    const outerPath = join(active, `.mutation-operation-${sequence}`);
    const receiptPath = join(active, "14-provider-cleanup-passed.json");
    const closePath = join(active, `.mutation-close-${sequence}`);
    const innerPath = join(
      active,
      "provider",
      "provider-retirement-settlement.json",
    );
    const identityPath = join(active, "provider", "provider-identity.json");
    const originals = new Map([
      [slotPath, readFileSync(slotPath)],
      [outerPath, readFileSync(outerPath)],
      [receiptPath, readFileSync(receiptPath)],
      [closePath, readFileSync(closePath)],
    ]);
    const restore = () => {
      for (const [path, bytes] of originals) {
        writeFileSync(path, bytes, { mode: 0o600 });
      }
    };
    const innerSha256 = sha256(readFileSync(innerPath));
    const identitySha256 = sha256(readFileSync(identityPath));
    const mutations = [
      () => {
        const slot = parse(slotPath);
        slot.operation_plan.create_close_sha256 = "f".repeat(64);
        writeFileSync(slotPath, canonicalBytes(slot), { mode: 0o600 });
      },
      () => {
        const receipt = parse(receiptPath);
        receipt.result.operation_evidence_sha256 = innerSha256;
        const receiptBytes = canonicalBytes(receipt);
        writeFileSync(receiptPath, receiptBytes, { mode: 0o600 });
        const close = parse(closePath);
        close.result_head_sha256 = sha256(receiptBytes);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      () => {
        const close = parse(closePath);
        close.operation_evidence_sha256 = innerSha256;
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      () => {
        const close = parse(closePath);
        close.authority = "recovery";
        close.authority_sha256 = "f".repeat(64);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      () => {
        const outer = parse(outerPath);
        outer.provider_retirement_settlement_sha256 = identitySha256;
        const outerBytes = canonicalBytes(outer);
        writeFileSync(outerPath, outerBytes, { mode: 0o600 });
        const receipt = parse(receiptPath);
        receipt.result.operation_evidence_sha256 = sha256(outerBytes);
        const receiptBytes = canonicalBytes(receipt);
        writeFileSync(receiptPath, receiptBytes, { mode: 0o600 });
        const close = parse(closePath);
        close.operation_evidence_sha256 = sha256(outerBytes);
        close.result_head_sha256 = sha256(receiptBytes);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
    ];
    for (const mutate of mutations) {
      restore();
      mutate();
      const refused = verify(state);
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /^clean-engine: /);
      restore();
      assert.equal(verify(state).status, 0);
    }
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("state-owned cleanup retires the controlled provider through one outer settlement", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const createSlotBytes = readFileSync(join(active, ".mutation-slot-00"));
    const createSlot = JSON.parse(createSlotBytes);
    const createSettlementBytes = readFileSync(
      join(active, ".mutation-operation-00"),
    );
    const createCloseBytes = readFileSync(join(active, ".mutation-close-00"));
    const providerIdentityBytes = readFileSync(
      join(active, "provider", "provider-identity.json"),
    );
    const immutableCreation = new Map(
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.artifact_order.map((name) => [
        name,
        readFileSync(join(active, "provider", name)),
      ]),
    );

    continueThroughProjectCleanup(state);
    const receipt = await executeBackgroundProviderCleanupForExecutor({
      adapter: cleanupAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });

    const cleanupSlotName = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-slot-"))
      .sort()
      .at(-1);
    const cleanupSlotBytes = readFileSync(join(active, cleanupSlotName));
    const cleanupSlot = JSON.parse(cleanupSlotBytes);
    const sequence = String(cleanupSlot.journal_sequence).padStart(2, "0");
    const outerBytes = readFileSync(
      join(active, `.mutation-operation-${sequence}`),
    );
    const outer = JSON.parse(outerBytes);
    const close = parse(join(active, `.mutation-close-${sequence}`));
    const innerBytes = readFileSync(
      join(active, "provider", "provider-retirement-settlement.json"),
    );
    const intent = parse(join(active, "13-provider-cleanup-intent.json"));
    const passed = parse(join(active, "14-provider-cleanup-passed.json"));
    const bindings = {
      cleanup_intent_sha256: cleanupSlot.intent_receipt_sha256,
      cleanup_operation_plan_sha256: sha256(
        canonicalBytes(cleanupSlot.operation_plan),
      ),
      cleanup_slot_sequence: cleanupSlot.journal_sequence,
      cleanup_slot_sha256: sha256(cleanupSlotBytes),
      create_close_sha256: sha256(createCloseBytes),
      create_settlement_sha256: sha256(createSettlementBytes),
      create_slot_sha256: sha256(createSlotBytes),
      source_head_sha256: cleanupSlot.source_head_sha256,
      source_sequence: cleanupSlot.source_sequence,
    };
    const inspected = inspectControlledBackgroundRetirementPrefix(
      join(active, "provider"),
      createSlot.operation_plan.fixture_id,
      { expectedBindings: bindings, providerBase: state.providerBase },
    );

    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(cleanupSlot.action, "provider-cleanup");
    assert.equal(intent.result.operation_plan_sha256,
      sha256(canonicalBytes(cleanupSlot.operation_plan)));
    assert.equal(cleanupSlot.intent_receipt_sha256, sha256(canonicalBytes(intent)));
    assert.equal(inspected.cleanupStage, "settled");
    assert.equal(inspected.rootDisposition, "retired");
    assert.equal(existsSync(createSlot.operation_plan.provider_root_path), false);
    assert.equal(outer.schema,
      "synveda.clean-engine.background-cleanup-settlement.v1");
    assert.equal(outer.provider_retirement_settlement_sha256, sha256(innerBytes));
    assert.equal(outer.result_receipt_authorized, true);
    assert.equal(JSON.parse(innerBytes).result_receipt_authorized, false);
    assert.equal(passed.result.operation_evidence_sha256, sha256(outerBytes));
    assert.equal(close.operation_evidence_sha256, sha256(outerBytes));
    for (const alternate of [
      sha256(innerBytes),
      sha256(providerIdentityBytes),
      sha256(createSettlementBytes),
    ]) {
      assert.notEqual(passed.result.operation_evidence_sha256, alternate);
      assert.notEqual(close.operation_evidence_sha256, alternate);
    }
    inspectControlledBackgroundProvider(
      join(active, "provider"),
      createSlot.operation_plan.fixture_id,
      { expectedCreateBindings: stateCreateBindings(createSlotBytes, createSlot) },
    );
    for (const [name, bytes] of immutableCreation) {
      assert.deepEqual(readFileSync(join(active, "provider", name)), bytes);
    }
    assert.equal(verify(state).status, 0);
    assert.throws(
      () => finalizeEnvironmentForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /controlled background evidence is not eligible for environment finalization/,
    );
    const slotsBeforeRetry = readdirSync(active).filter((name) =>
      name.startsWith(".mutation-slot-"));
    await assert.rejects(
      () => executeBackgroundProviderCleanupForExecutor({
        adapter: cleanupAdapter(),
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /background cleanup state was refused/,
    );
    assert.equal(
      readdirSync(active).filter((name) => name.startsWith(".mutation-slot-"))
        .length,
      slotsBeforeRetry.length,
    );
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup state authority binds every retirement checkpoint field", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const fields = [
      "checkpoint",
      "cleanup_intent_sha256",
      "cleanup_operation_plan_sha256",
      "cleanup_plan_sha256",
      "cleanup_slot_sequence",
      "cleanup_slot_sha256",
      "completed_steps",
      "create_close_sha256",
      "create_settlement_sha256",
      "create_slot_sha256",
      "next_action",
      "next_resources",
      "operation_kind",
      "provider_identity_sha256",
      "publication_disposition",
      "publication_expected_sha256",
      "publication_phase",
      "publication_stage_declared_sha256",
      "publication_stage_identity_sha256",
      "publication_stage_sha256",
      "publication_target_name",
      "resource_identity_sha256",
      "retirement_contract_sha256",
      "source_head_sha256",
      "source_sequence",
    ];
    const probed = new Set();
    const checkpoints = new Set();
    const phases = new Set();
    await executeBackgroundProviderCleanupForExecutor({
      adapter: cleanupAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
      testAuthorityCheckpointObserver(checkpoint, authorize) {
        checkpoints.add(checkpoint.checkpoint);
        phases.add(checkpoint.publication_phase);
        if (probed.size !== 0) return;
        for (const field of fields) {
          const mutated = JSON.parse(JSON.stringify(checkpoint));
          const value = mutated[field];
          if (field === "checkpoint") {
            mutated[field] = "before-retirement-progress-publication";
          } else if (Array.isArray(value)) {
            mutated[field] = [...value, "unexpected-resource"];
          } else if (typeof value === "number") {
            mutated[field] = value + 1;
          } else if (/^[0-9a-f]{64}$/.test(value)) {
            mutated[field] = value === "f".repeat(64)
              ? "e".repeat(64)
              : "f".repeat(64);
          } else {
            mutated[field] = `${value}-mutated`;
          }
          assert.throws(
            () => authorize(mutated),
            (error) => {
              assert.equal(error.exitStatus, 73);
              assert.match(error.message, /background cleanup process authority/);
              return true;
            },
            field,
          );
          probed.add(field);
        }
      },
    });
    assert.deepEqual([...probed].sort(), [...fields].sort());
    assert.deepEqual(
      [...checkpoints].sort(),
      [
        "before-hostagent-shutdown-delivery",
        "before-resource-rmdir",
        "before-resource-unlink",
        "before-retirement-plan-publication",
        "before-retirement-progress-publication",
        "before-retirement-settlement-publication",
      ],
    );
    assert.deepEqual(
      [...phases].sort(),
      [
        "before-final-consumption",
        "before-final-link",
        "before-stage-removal",
        "before-stage-write",
        "not-applicable",
      ],
    );
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup recovery resumes a durable plan under a cleanup-only claim", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "hold-after-plan");
    const active = activeRun(state);
    await waitFor(
      join(active, "provider", "provider-retirement-plan.json"),
      12_000,
      owner,
    );
    await killOwner(owner);
    const cleanupSlotName = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-slot-"))
      .sort()
      .at(-1);
    const cleanupSlot = parse(join(active, cleanupSlotName));
    const sequence = String(cleanupSlot.journal_sequence).padStart(2, "0");
    const argumentsValue = cleanupRecoveryArguments(state);
    assert.throws(
      () => recoverBackgroundProviderCreateForExecutor({
        adapter: adapter(),
        confirmation: argumentsValue.confirmation,
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /matching dedicated executor/,
    );
    assert.equal(
      existsSync(join(active, `.mutation-recovery-${sequence}-00`)),
      false,
    );
    const receipt = await recoverBackgroundProviderCleanupForExecutor(
      argumentsValue,
    );
    const claimNames = readdirSync(active)
      .filter((name) => name.startsWith(`.mutation-recovery-${sequence}-`))
      .sort();
    assert.equal(claimNames.length, 2);
    const initialClaimBytes = readFileSync(join(active, claimNames[0]));
    const initialClaim = JSON.parse(initialClaimBytes);
    const authorityClaimBytes = readFileSync(join(active, claimNames.at(-1)));
    const authorityClaim = JSON.parse(authorityClaimBytes);
    const outerBytes = readFileSync(
      join(active, `.mutation-operation-${sequence}`),
    );
    const outer = JSON.parse(outerBytes);
    const close = parse(join(active, `.mutation-close-${sequence}`));
    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(initialClaim.action, "provider-cleanup");
    assert.equal(initialClaim.operation_kind, cleanupSlot.operation_kind);
    assert.equal(authorityClaim.observed_evidence_stage, "settled");
    assert.equal(authorityClaim.observed_settlement_sha256, "0".repeat(64));
    assert.equal(
      authorityClaim.observed_evidence_prefix_sha256,
      outer.evidence_prefix_sha256,
    );
    assert.equal(
      authorityClaim.observed_residual_sha256,
      outer.residual_sha256,
    );
    assert.equal(outer.authority, "recovery");
    assert.equal(outer.authority_sha256, sha256(authorityClaimBytes));
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(authorityClaimBytes));
    assert.equal(close.operation_evidence_sha256, sha256(outerBytes));
    assert.equal(verify(state).status, 0);

    const outerPath = join(active, `.mutation-operation-${sequence}`);
    const receiptPath = join(active, "14-provider-cleanup-passed.json");
    const closePath = join(active, `.mutation-close-${sequence}`);
    const originalReceiptBytes = readFileSync(receiptPath);
    const originalCloseBytes = readFileSync(closePath);
    const rewrittenOuter = { ...outer, authority_sha256: sha256(initialClaimBytes) };
    const rewrittenOuterBytes = canonicalBytes(rewrittenOuter);
    const rewrittenReceipt = parse(receiptPath);
    rewrittenReceipt.result.operation_evidence_sha256 =
      sha256(rewrittenOuterBytes);
    const rewrittenReceiptBytes = canonicalBytes(rewrittenReceipt);
    const rewrittenClose = parse(closePath);
    rewrittenClose.operation_evidence_sha256 = sha256(rewrittenOuterBytes);
    rewrittenClose.result_head_sha256 = sha256(rewrittenReceiptBytes);
    writeFileSync(outerPath, rewrittenOuterBytes, { mode: 0o600 });
    writeFileSync(receiptPath, rewrittenReceiptBytes, { mode: 0o600 });
    writeFileSync(closePath, canonicalBytes(rewrittenClose), { mode: 0o600 });
    const refused = verify(state);
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /background cleanup settlement authority/);
    writeFileSync(outerPath, outerBytes, { mode: 0o600 });
    writeFileSync(receiptPath, originalReceiptBytes, { mode: 0o600 });
    writeFileSync(closePath, originalCloseBytes, { mode: 0o600 });
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup recovery reuses durable inner outer and receipt frontiers", async (t) => {
  for (const scenario of [
    {
      marker: (active) =>
        join(active, "provider", "provider-retirement-settlement.json"),
      mode: "hold-after-retirement",
      outerBefore: false,
      receiptBefore: false,
    },
    {
      marker: (active) => join(active, ".mutation-operation-11"),
      mode: "hold-after-settlement",
      outerBefore: true,
      receiptBefore: false,
    },
    {
      marker: (active) => join(active, "14-provider-cleanup-passed.json"),
      mode: "hold-after-result",
      outerBefore: true,
      receiptBefore: true,
    },
  ]) {
    await t.test(scenario.mode, async () => {
      const state = fixture();
      try {
        await executeBackgroundProviderCreateForExecutor({
          adapter: cleanupProviderAdapter(),
          providerBase: state.providerBase,
          repoRoot: state.repo,
          stateBase: state.state,
        });
        continueThroughProjectCleanup(state);
        const owner = launchCleanupOwner(state, scenario.mode);
        const active = activeRun(state);
        await waitFor(scenario.marker(active), 20_000, owner);
        const outerPath = join(active, ".mutation-operation-11");
        const receiptPath = join(active, "14-provider-cleanup-passed.json");
        const outerBytes = scenario.outerBefore
          ? readFileSync(outerPath)
          : undefined;
        const receiptBytes = scenario.receiptBefore
          ? readFileSync(receiptPath)
          : undefined;
        await killOwner(owner);
        const argumentsValue = cleanupRecoveryArguments(state);
        const receipt = await recoverBackgroundProviderCleanupForExecutor(
          argumentsValue,
        );
        const claim = parse(join(active, ".mutation-recovery-11-00"));
        const durableOuterBytes = readFileSync(outerPath);
        const durableOuter = JSON.parse(durableOuterBytes);
        const close = parse(join(active, ".mutation-close-11"));
        assert.equal(receipt.phase, "provider-cleanup-passed");
        assert.equal(
          claim.observed_settlement_sha256,
          scenario.outerBefore ? sha256(outerBytes) : "0".repeat(64),
        );
        if (scenario.outerBefore) {
          assert.deepEqual(durableOuterBytes, outerBytes);
          assert.equal(durableOuter.authority, "owner");
        } else {
          assert.equal(durableOuter.authority, "recovery");
        }
        if (scenario.receiptBefore) {
          assert.deepEqual(readFileSync(receiptPath), receiptBytes);
        }
        assert.equal(close.authority, "recovery");
        assert.equal(
          close.operation_evidence_sha256,
          sha256(durableOuterBytes),
        );
        assert.equal(verify(state).status, 0);
      } finally {
        rmSync(state.root, { force: true, recursive: true });
      }
    });
  }
});

test("cleanup recovery converges deletion completed before progress publication", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "crash-after-delete");
    const [status, signal] = await owner.closed;
    assert.equal(status, 75, owner.stderr());
    assert.equal(signal, null, owner.stderr());
    assert.match(owner.stderr(), /simulated controlled background state retirement crash/);
    const active = activeRun(state);
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
    const receipt = await recoverBackgroundProviderCleanupForExecutor(
      cleanupRecoveryArguments(state),
    );
    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(existsSync(join(active, ".mutation-operation-11")), true);
    assert.equal(existsSync(join(active, ".mutation-close-11")), true);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("competing cleanup owners publish one operation slot and one close", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owners = [
      launchCleanupOwner(state, "pass"),
      launchCleanupOwner(state, "pass"),
    ];
    const outcomes = await Promise.all(
      owners.map(async (owner) => {
        const [status, signal] = await owner.closed;
        return { signal, status, stderr: owner.stderr() };
      }),
    );
    assert.deepEqual(
      outcomes.map(({ status }) => status).sort((left, right) => left - right),
      [0, 73],
    );
    assert.equal(outcomes.find(({ status }) => status === 0).signal, null);
    assert.match(
      outcomes.find(({ status }) => status === 73).stderr,
      /another clean-engine mutation|background cleanup state was refused/,
    );
    const active = activeRun(state);
    const cleanupSlots = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-slot-"))
      .map((name) => parse(join(active, name)))
      .filter((slot) => slot.action === "provider-cleanup");
    assert.equal(cleanupSlots.length, 1);
    const sequence = String(cleanupSlots[0].journal_sequence).padStart(2, "0");
    assert.equal(existsSync(join(active, `.mutation-operation-${sequence}`)), true);
    assert.equal(existsSync(join(active, `.mutation-close-${sequence}`)), true);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a live cleanup recoverer fences contenders and the next claim completes", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "hold-after-plan");
    const active = activeRun(state);
    await waitFor(
      join(active, "provider", "provider-retirement-plan.json"),
      12_000,
      owner,
    );
    await killOwner(owner);
    const confirmation =
      backgroundProviderCleanupRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      });
    const first = launchCleanupRecovery(state, confirmation, "hold-after-plan");
    await waitFor(join(active, ".mutation-recovery-11-00"), 12_000, first);
    await assert.rejects(
      () => recoverBackgroundProviderCleanupForExecutor({
        adapter: cleanupAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /another provider recovery is active or could not be identified/,
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-11-01")), false);
    await killOwner(first);
    const receipt = await recoverBackgroundProviderCleanupForExecutor({
      adapter: cleanupAdapter(),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const finalClaimName = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-recovery-11-"))
      .sort()
      .at(-1);
    const finalClaimBytes = readFileSync(join(active, finalClaimName));
    const close = parse(join(active, ".mutation-close-11"));
    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(finalClaimBytes));
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup recovery reserves its final-observation claim before effects", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    await waitForHostagentExit(parse(join(active, ".mutation-slot-00")));
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "hold-after-intent");
    await waitFor(
      join(active, "13-provider-cleanup-intent.json"),
      12_000,
      owner,
    );
    await killOwner(owner);
    const first = launchCleanupRecovery(
      state,
      backgroundProviderCleanupRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      "hold-after-plan",
    );
    await waitFor(
      join(active, "provider", "provider-retirement-plan.json"),
      12_000,
      first,
    );
    await killOwner(first);
    let previous = parse(join(active, ".mutation-recovery-11-00"));
    for (let sequence = 1; sequence < 7; sequence += 1) {
      const claim = {
        ...previous,
        nonce: sequence.toString(16).repeat(32),
        parent_sha256: sha256(canonicalBytes(previous)),
        sequence,
      };
      writeFileSync(
        join(
          active,
          `.mutation-recovery-11-${String(sequence).padStart(2, "0")}`,
        ),
        canonicalBytes(claim),
        { mode: 0o600 },
      );
      previous = claim;
    }
    assert.equal(verify(state).status, 0);
    const providerIdentity = parse(
      join(active, "provider", "provider-identity.json"),
    );
    await assert.rejects(
      () => recoverBackgroundProviderCleanupForExecutor(
        cleanupRecoveryArguments(state),
      ),
      /provider mutation recovery capacity was exhausted/,
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-11-07")), false);
    assert.equal(existsSync(providerIdentity.provider_root.path), true);
    assert.equal(
      readdirSync(join(active, "provider")).some((name) =>
        name.startsWith("retirement-step-")),
      false,
    );
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup recovery history refuses regression from a settled prefix", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const active = activeRun(state);
    const owner = launchCleanupOwner(state, "hold-after-retirement");
    await waitFor(
      join(active, "provider", "provider-retirement-settlement.json"),
      20_000,
      owner,
    );
    await killOwner(owner);
    const recovery = launchCleanupRecovery(
      state,
      backgroundProviderCleanupRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      "pause-after-claim",
    );
    const claimPath = join(active, ".mutation-recovery-11-00");
    await waitFor(claimPath, 12_000, recovery);
    await killOwner(recovery);
    const settled = parse(claimPath);
    assert.equal(settled.observed_evidence_stage, "settled");
    const regressed = {
      ...settled,
      nonce: "d".repeat(32),
      observed_effect_disposition: "pending",
      observed_evidence_stage: "retiring",
      parent_sha256: sha256(canonicalBytes(settled)),
      sequence: 1,
    };
    writeFileSync(
      join(active, ".mutation-recovery-11-01"),
      canonicalBytes(regressed),
      { mode: 0o600 },
    );
    const refused = verify(state);
    assert.equal(refused.status, 78);
    assert.match(
      refused.stderr,
      /background cleanup recovery settlement history was refused/,
    );
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("source drift before retirement leaves cleanup open and effect-free", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "source-drift-before-plan");
    const active = activeRun(state);
    await waitFor(join(active, "13-provider-cleanup-intent.json"), 12_000, owner);
    writeFileSync(join(state.repo, "source.txt"), "cleanup source drift\n");
    const [status, signal] = await owner.closed;
    assert.equal(status, 73, owner.stderr());
    assert.equal(signal, null, owner.stderr());
    assert.match(
      owner.stderr(),
      /source closure|owner authority|background cleanup state authority/,
    );
    assert.equal(
      existsSync(join(active, "provider", "provider-retirement-plan.json")),
      false,
    );
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
    const receipt = await recoverBackgroundProviderCleanupForExecutor(
      cleanupRecoveryArguments(state),
    );
    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("cleanup recovery refuses replaced bound parent directories before planning", async (t) => {
  for (const target of ["provider-base", "evidence-directory"]) {
    await t.test(target, async () => {
      const state = fixture();
      try {
        await executeBackgroundProviderCreateForExecutor({
          adapter: adapter(),
          providerBase: state.providerBase,
          repoRoot: state.repo,
          stateBase: state.state,
        });
        const active = activeRun(state);
        await waitForHostagentExit(parse(join(active, ".mutation-slot-00")));
        continueThroughProjectCleanup(state);
        const owner = launchCleanupOwner(state, "hold-after-intent");
        await waitFor(
          join(active, "13-provider-cleanup-intent.json"),
          12_000,
          owner,
        );
        await killOwner(owner);
        const slot = parse(join(active, ".mutation-slot-11"));
        const recovery = launchCleanupRecovery(
          state,
          backgroundProviderCleanupRecoveryConfirmationForExecutor({
            repoRoot: state.repo,
            stateBase: state.state,
          }),
          "pause-after-claim",
        );
        await waitFor(
          join(active, ".mutation-recovery-11-00"),
          12_000,
          recovery,
        );
        if (target === "provider-base") {
          const displaced = `${state.providerBase}-displaced`;
          const providerRoot = parse(
            join(active, "provider", "provider-identity.json"),
          ).provider_root.path;
          renameSync(state.providerBase, displaced);
          mkdirSync(state.providerBase, { mode: 0o700 });
          renameSync(
            join(displaced, basename(providerRoot)),
            join(state.providerBase, basename(providerRoot)),
          );
        } else {
          const evidence = slot.operation_plan.evidence_directory.path;
          const displaced = join(state.root, "displaced-evidence");
          renameSync(evidence, displaced);
          mkdirSync(evidence, { mode: 0o700 });
          for (const name of readdirSync(displaced)) {
            renameSync(join(displaced, name), join(evidence, name));
          }
        }
        const [status, signal] = await recovery.closed;
        assert.equal(status, 78, recovery.stderr());
        assert.equal(signal, null, recovery.stderr());
        assert.match(
          recovery.stderr(),
          /controlled background operation plan identity changed|background cleanup state authority/,
        );
        assert.equal(
          existsSync(
            join(active, "provider", "provider-retirement-plan.json"),
          ),
          false,
        );
        assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
        assert.equal(existsSync(join(active, ".mutation-close-11")), false);
      } finally {
        rmSync(state.root, { force: true, recursive: true });
      }
    });
  }
});

test("cleanup close reasserts completion at the owner and recovery link boundary", async (t) => {
  await t.test("owner-source-drift", async () => {
    const state = fixture();
    try {
      await executeBackgroundProviderCreateForExecutor({
        adapter: cleanupProviderAdapter(),
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      continueThroughProjectCleanup(state);
      const owner = launchCleanupOwner(state, "hold-before-close-link");
      const active = activeRun(state);
      await waitFor(
        join(active, "14-provider-cleanup-passed.json"),
        20_000,
        owner,
      );
      await waitForMutationStage(active, owner, 20_000);
      writeFileSync(join(state.repo, "source.txt"), "cleanup close drift\n");
      const [status, signal] = await owner.closed;
      assert.equal(status, 73, owner.stderr());
      assert.equal(signal, null, owner.stderr());
      assert.match(owner.stderr(), /source closure/);
      assert.equal(existsSync(join(active, ".mutation-close-11")), false);
      writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
      const receipt = await recoverBackgroundProviderCleanupForExecutor(
        cleanupRecoveryArguments(state),
      );
      assert.equal(receipt.phase, "provider-cleanup-passed");
      assert.equal(verify(state).status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  });

  await t.test("recovery-inert-staging", async () => {
    const state = fixture();
    try {
      await executeBackgroundProviderCreateForExecutor({
        adapter: cleanupProviderAdapter(),
        providerBase: state.providerBase,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      continueThroughProjectCleanup(state);
      const owner = launchCleanupOwner(state, "hold-after-settlement");
      const active = activeRun(state);
      await waitFor(join(active, ".mutation-operation-11"), 20_000, owner);
      await killOwner(owner);
      const confirmation =
        backgroundProviderCleanupRecoveryConfirmationForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        });
      const recovery = launchCleanupRecovery(
        state,
        confirmation,
        "hold-before-close-link",
      );
      await waitFor(
        join(active, "14-provider-cleanup-passed.json"),
        20_000,
        recovery,
      );
      await waitForMutationStage(active, recovery, 20_000);
      const inert = join(state.state, `.pending-${"e".repeat(32)}`);
      mkdirSync(inert, { mode: 0o700 });
      const [status, signal] = await recovery.closed;
      assert.equal(status, 73, recovery.stderr());
      assert.equal(signal, null, recovery.stderr());
      assert.match(recovery.stderr(), /inert staging/);
      assert.equal(existsSync(join(active, ".mutation-close-11")), false);
      rmSync(inert, { force: false, recursive: true });
      const receipt = await recoverBackgroundProviderCleanupForExecutor({
        adapter: cleanupAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      assert.equal(receipt.phase, "provider-cleanup-passed");
      assert.equal(verify(state).status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  });
});

test("pre-intent cleanup recovery aborts without effect and permits a fresh slot", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const active = activeRun(state);
    const providerBefore = evidenceSnapshot(state);
    const createSlot = parse(join(active, ".mutation-slot-00"));
    const owner = launchCleanupOwner(state, "hold-after-slot");
    await waitFor(join(active, ".mutation-slot-11"), 12_000, owner);
    assert.equal(existsSync(join(active, "13-provider-cleanup-intent.json")), false);
    assert.equal(
      existsSync(join(active, "provider", "provider-retirement-plan.json")),
      false,
    );
    await killOwner(owner);
    writeFileSync(join(state.repo, "source.txt"), "pre-intent source drift\n");
    const source = await recoverBackgroundProviderCleanupForExecutor(
      cleanupRecoveryArguments(state),
    );
    const aborted = parse(join(active, ".mutation-close-11"));
    assert.equal(source.phase, "project-cleanup-passed");
    assert.equal(aborted.disposition, "aborted-before-effect");
    assert.equal(aborted.operation_evidence_sha256, "0".repeat(64));
    assert.equal(existsSync(createSlot.operation_plan.provider_root_path), true);
    assert.deepEqual(evidenceSnapshot(state), providerBefore);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
    const verifiedAborted = verify(state);
    assert.equal(verifiedAborted.status, 0, verifiedAborted.stderr);

    const receipt = await executeBackgroundProviderCleanupForExecutor({
      adapter: cleanupAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(receipt.phase, "provider-cleanup-passed");
    const cleanupSlots = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-slot-"))
      .map((name) => parse(join(active, name)))
      .filter((slot) => slot.action === "provider-cleanup");
    assert.equal(cleanupSlots.length, 2);
    assert.equal(cleanupSlots[0].journal_sequence, 11);
    assert.equal(cleanupSlots[1].journal_sequence, 12);
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-operation-12")), true);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("retirement evidence published before the cleanup intent is refused", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    await waitForHostagentExit(parse(join(active, ".mutation-slot-00")));
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "hold-after-slot");
    await waitFor(join(active, ".mutation-slot-11"), 12_000, owner);
    await killOwner(owner);
    const cleanupSlotBytes = readFileSync(join(active, ".mutation-slot-11"));
    const cleanupSlot = JSON.parse(cleanupSlotBytes);
    await planControlledBackgroundRetirementWithAuthorityGate(
      {
        bindings: {
          cleanup_intent_sha256: cleanupSlot.intent_receipt_sha256,
          cleanup_operation_plan_sha256: sha256(
            canonicalBytes(cleanupSlot.operation_plan),
          ),
          cleanup_slot_sequence: cleanupSlot.journal_sequence,
          cleanup_slot_sha256: sha256(cleanupSlotBytes),
          create_close_sha256:
            cleanupSlot.operation_plan.create_close_sha256,
          create_settlement_sha256:
            cleanupSlot.operation_plan.create_settlement_sha256,
          create_slot_sha256: cleanupSlot.operation_plan.create_slot_sha256,
          source_head_sha256: cleanupSlot.source_head_sha256,
          source_sequence: cleanupSlot.source_sequence,
        },
        evidenceDirectory: cleanupSlot.operation_plan.evidence_directory.path,
        fixtureId: cleanupSlot.fixture_id,
        providerBase: cleanupSlot.operation_plan.provider_base.path,
      },
      () => undefined,
    );
    const refused = verify(state);
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /background cleanup evidence preceded its intent/);
    assert.equal(existsSync(join(active, "13-provider-cleanup-intent.json")), false);
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("inert state staging blocks the outer cleanup receipt until removed", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: cleanupProviderAdapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    continueThroughProjectCleanup(state);
    const owner = launchCleanupOwner(state, "inert-before-outer");
    const active = activeRun(state);
    await waitFor(
      join(active, "provider", "provider-retirement-settlement.json"),
      20_000,
      owner,
    );
    const inert = join(state.state, `.pending-${"f".repeat(32)}`);
    mkdirSync(inert, { mode: 0o700 });
    const [status, signal] = await owner.closed;
    assert.equal(status, 73, owner.stderr());
    assert.equal(signal, null, owner.stderr());
    assert.match(owner.stderr(), /inert staging/);
    assert.equal(existsSync(join(active, ".mutation-operation-11")), false);
    assert.equal(existsSync(join(active, "14-provider-cleanup-passed.json")), false);
    assert.equal(existsSync(join(active, ".mutation-close-11")), false);
    rmSync(inert, { force: false, recursive: true });
    const receipt = await recoverBackgroundProviderCleanupForExecutor(
      cleanupRecoveryArguments(state),
    );
    assert.equal(receipt.phase, "provider-cleanup-passed");
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("outer settlement receipt and close refuse independent evidence tamper", async () => {
  const state = fixture();
  try {
    await executeBackgroundProviderCreateForExecutor({
      adapter: adapter(),
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const settlementPath = join(active, ".mutation-operation-00");
    const receiptPath = join(active, "02-provider-create-passed.json");
    const closePath = join(active, ".mutation-close-00");
    const originals = new Map([
      [settlementPath, readFileSync(settlementPath)],
      [receiptPath, readFileSync(receiptPath)],
      [closePath, readFileSync(closePath)],
    ]);
    const restore = () => {
      for (const [path, bytes] of originals) writeFileSync(path, bytes, { mode: 0o600 });
    };
    const mutations = [
      () => {
        const settlement = parse(settlementPath);
        settlement.schema = "synveda.clean-engine.background-create-settlement.v0";
        writeFileSync(settlementPath, canonicalBytes(settlement), { mode: 0o600 });
      },
      () => {
        const settlement = parse(settlementPath);
        settlement.static_root_identity_sha256 = "f".repeat(64);
        writeFileSync(settlementPath, canonicalBytes(settlement), { mode: 0o600 });
      },
      () => {
        const settlement = parse(settlementPath);
        settlement.authority_sha256 = "f".repeat(64);
        writeFileSync(settlementPath, canonicalBytes(settlement), { mode: 0o600 });
      },
      () => {
        const receipt = parse(receiptPath);
        receipt.result.operation_evidence_sha256 = "f".repeat(64);
        const receiptBytes = canonicalBytes(receipt);
        writeFileSync(receiptPath, receiptBytes, { mode: 0o600 });
        const close = parse(closePath);
        close.result_head_sha256 = sha256(receiptBytes);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      () => {
        const close = parse(closePath);
        close.operation_evidence_sha256 = "f".repeat(64);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
    ];
    for (const mutate of mutations) {
      restore();
      mutate();
      const refused = verify(state);
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /^clean-engine: /);
      restore();
      assert.equal(verify(state).status, 0);
    }
    const slot = parse(join(active, ".mutation-slot-00"));
    await waitForHostagentExit(slot);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("same-byte static inode replacement blocks settlement after provider identity", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-evidence");
    const active = activeRun(state);
    await waitFor(join(active, "provider", "provider-identity.json"), 12_000, owner);
    const slot = parse(join(active, ".mutation-slot-00"));
    await killOwner(owner);
    replaceStaticFileWithSameBytes(state, active);
    assert.throws(
      () => providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /static root identity changed/,
    );
    assert.equal(existsSync(join(active, ".mutation-operation-00")), false);
    assert.equal(existsSync(join(active, ".mutation-close-00")), false);
    await waitForHostagentExit(slot);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("same-byte static inode replacement blocks close after settlement", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-settlement");
    const active = activeRun(state);
    await waitFor(join(active, ".mutation-operation-00"), 12_000, owner);
    const slot = parse(join(active, ".mutation-slot-00"));
    await killOwner(owner);
    replaceStaticFileWithSameBytes(state, active);
    assert.throws(
      () => providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /static root identity changed/,
    );
    assert.equal(existsSync(join(active, ".mutation-operation-00")), true);
    assert.equal(existsSync(join(active, ".mutation-close-00")), false);
    await waitForHostagentExit(slot);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("same-byte static inode replacement is fenced at the final close gate", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-result");
    const active = activeRun(state);
    const receiptPath = join(active, "02-provider-create-passed.json");
    const settlementPath = join(active, ".mutation-operation-00");
    await waitFor(receiptPath, 12_000, owner);
    const receiptBytes = readFileSync(receiptPath);
    const settlementBytes = readFileSync(settlementPath);
    const slot = parse(join(active, ".mutation-slot-00"));
    replaceStaticFileWithSameBytes(state, active);
    await killOwner(owner);
    assert.throws(
      () => providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /static root identity changed/,
    );
    assert.deepEqual(readFileSync(receiptPath), receiptBytes);
    assert.deepEqual(readFileSync(settlementPath), settlementBytes);
    assert.equal(existsSync(join(active, ".mutation-close-00")), false);
    await waitForHostagentExit(slot);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery settles a linked-complete identity without mutating either link", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-evidence");
    const active = activeRun(state);
    const identityPath = join(active, "provider", "provider-identity.json");
    await waitFor(identityPath, 12_000, owner);
    const slot = parse(join(active, ".mutation-slot-00"));
    const identityBytes = readFileSync(identityPath);
    const identitySha256 = sha256(identityBytes);
    const stagePath = join(
      active,
      "provider",
      `.provider-process-stage-provider-identity-${identitySha256}-${"a".repeat(32)}`,
    );
    linkSync(identityPath, stagePath);
    const beforeIdentity = lstatSync(identityPath, { bigint: true });
    const beforeStage = lstatSync(stagePath, { bigint: true });
    assert.equal(beforeIdentity.dev, beforeStage.dev);
    assert.equal(beforeIdentity.ino, beforeStage.ino);
    assert.equal(beforeIdentity.nlink, 2n);
    const before = evidenceSnapshot(state);
    await killOwner(owner);

    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const settlementBytes = readFileSync(join(active, ".mutation-operation-00"));
    const settlement = JSON.parse(settlementBytes);
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(receipt.phase, "provider-create-passed");
    assert.equal(settlement.disposition, "complete-identity");
    assert.equal(settlement.pending_evidence_publication.target_name,
      "provider-identity.json");
    assert.equal(settlement.pending_evidence_publication.disposition,
      "linked-complete");
    assert.equal(settlement.pending_evidence_publication.links, 2);
    assert.equal(receipt.result.operation_evidence_sha256, sha256(settlementBytes));
    assert.equal(close.operation_evidence_sha256, sha256(settlementBytes));

    inspectControlledBackgroundProvider(
      join(active, "provider"),
      slot.operation_plan.fixture_id,
    );
    const beforeRefusals = evidenceSnapshot(state);
    assert.throws(
      () =>
        controlledBackgroundOperationEvidence({
          action: "provider-create",
          evidenceDirectory: join(active, "provider"),
          fixtureId: slot.operation_plan.fixture_id,
        }),
      /operation evidence integration was refused/,
    );
    await assert.rejects(
      () =>
        launchControlledBackgroundProvider({
          evidenceDirectory: join(active, "provider"),
          fixtureId: slot.operation_plan.fixture_id,
          maximumLifetimeMilliseconds: 5_000,
          providerBase: state.providerBase,
        }),
      /launch integration was refused/,
    );
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: join(active, "provider"),
            fixtureId: slot.operation_plan.fixture_id,
            maximumLifetimeMilliseconds: 5_000,
            providerBase: state.providerBase,
          },
          () => undefined,
        ),
      /state authority publication was incomplete/,
    );
    await assert.rejects(
      () =>
        planControlledBackgroundRetirement({
          bindings: {
            cleanup_intent_sha256: "a".repeat(64),
            cleanup_slot_sequence: 1,
            cleanup_slot_sha256: "b".repeat(64),
            create_close_sha256: sha256(readFileSync(join(active, ".mutation-close-00"))),
            create_slot_sha256: sha256(readFileSync(join(active, ".mutation-slot-00"))),
            source_head_sha256: "c".repeat(64),
            source_sequence: 2,
          },
          evidenceDirectory: join(active, "provider"),
          fixtureId: slot.operation_plan.fixture_id,
          providerBase: state.providerBase,
        }),
      /retirement integration was refused/,
    );
    await assert.rejects(
      () =>
        retireControlledBackgroundProvider({
          evidenceDirectory: join(active, "provider"),
          fixtureId: slot.operation_plan.fixture_id,
          providerBase: state.providerBase,
        }),
      /retirement integration was refused/,
    );
    assert.deepEqual(evidenceSnapshot(state), beforeRefusals);
    const afterIdentity = lstatSync(identityPath, { bigint: true });
    const afterStage = lstatSync(stagePath, { bigint: true });
    assert.equal(afterIdentity.dev, beforeIdentity.dev);
    assert.equal(afterIdentity.ino, beforeIdentity.ino);
    assert.equal(afterIdentity.nlink, 2n);
    assert.equal(afterStage.dev, beforeStage.dev);
    assert.equal(afterStage.ino, beforeStage.ino);
    assert.equal(afterStage.nlink, 2n);
    assert.deepEqual(evidenceSnapshot(state), before);
    await waitForHostagentExit(slot);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery reuses an owner settlement without replaying inner evidence", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-settlement");
    const active = activeRun(state);
    await waitFor(join(active, ".mutation-operation-00"), 12_000, owner);
    assert.equal(existsSync(join(active, "02-provider-create-passed.json")), false);
    const settlementBytes = readFileSync(join(active, ".mutation-operation-00"));
    const before = evidenceSnapshot(state);
    await killOwner(owner);
    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const claimBytes = readFileSync(join(active, ".mutation-recovery-00-00"));
    const claim = JSON.parse(claimBytes);
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(receipt.phase, "provider-create-passed");
    assert.deepEqual(readFileSync(join(active, ".mutation-operation-00")), settlementBytes);
    assert.equal(claim.observed_settlement_sha256, sha256(settlementBytes));
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(claimBytes));
    assert.deepEqual(evidenceSnapshot(state), before);
    const slot = parse(join(active, ".mutation-slot-00"));
    await waitForHostagentExit(slot);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery closes a durable pass without replacing its receipt or settlement", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-result");
    const active = activeRun(state);
    await waitFor(join(active, "02-provider-create-passed.json"), 12_000, owner);
    assert.equal(existsSync(join(active, ".mutation-close-00")), false);
    const receiptBytes = readFileSync(join(active, "02-provider-create-passed.json"));
    const settlementBytes = readFileSync(join(active, ".mutation-operation-00"));
    await killOwner(owner);
    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    assert.equal(receipt.phase, "provider-create-passed");
    assert.deepEqual(readFileSync(join(active, "02-provider-create-passed.json")), receiptBytes);
    assert.deepEqual(readFileSync(join(active, ".mutation-operation-00")), settlementBytes);
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(close.result_head_sha256, sha256(receiptBytes));
    assert.equal(close.operation_evidence_sha256, sha256(settlementBytes));
    const slot = parse(join(active, ".mutation-slot-00"));
    await waitForHostagentExit(slot);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("authority-only recovery records an exact residual without creating a provider root", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-authority");
    const active = activeRun(state);
    await waitFor(
      join(active, "provider", "background-create-authority.json"),
      12_000,
      owner,
    );
    const slot = parse(join(active, ".mutation-slot-00"));
    assert.equal(existsSync(slot.operation_plan.provider_root_path), false);
    const before = evidenceSnapshot(state);
    await killOwner(owner);
    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const settlement = parse(join(active, ".mutation-operation-00"));
    assert.equal(receipt.phase, "provider-create-failed");
    assert.equal(settlement.disposition, "exact-residual");
    assert.equal(settlement.safe_code, "evidence-refused");
    assert.equal(existsSync(slot.operation_plan.provider_root_path), false);
    assert.deepEqual(evidenceSnapshot(state), before);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("state recovery preserves permanently unattested process windows without replay", async () => {
  const expectations = [
    {
      checkpoint: "before-controller-spawn",
      controllerPresence: "unattested",
      evidenceStage: "controller-launch-decision",
      hostagentPresence: "not-started",
    },
    {
      checkpoint: "before-hostagent-start-delivery",
      controllerPresence: "proved-absent",
      evidenceStage: "provider-start-decision",
      hostagentPresence: "unattested",
    },
  ];
  for (const expectation of expectations) {
    const state = fixture();
    try {
      const owner = launchOwner(state, "hold-after-intent");
      const active = activeRun(state);
      await waitFor(join(active, "01-provider-create-intent.json"), 12_000, owner);
      await killOwner(owner);
      const slotBytes = readFileSync(join(active, ".mutation-slot-00"));
      const slot = JSON.parse(slotBytes);
      const bindings = stateCreateBindings(slotBytes, slot);
      planControlledBackgroundProviderCreateWithAuthorityGate(
        {
          bindings,
          evidenceDirectory: slot.operation_plan.evidence_directory.path,
          fixtureId: slot.operation_plan.fixture_id,
          operationPlan: slot.operation_plan,
          providerBase: slot.operation_plan.provider_base.path,
        },
        () => undefined,
      );
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: slot.operation_plan.evidence_directory.path,
              fixtureId: slot.operation_plan.fixture_id,
              maximumLifetimeMilliseconds: 5_000,
              providerBase: slot.operation_plan.provider_base.path,
            },
            ({ checkpoint }) => {
              if (checkpoint === expectation.checkpoint) {
                throw new Error(`stop at ${expectation.checkpoint}`);
              }
            },
          ),
        new RegExp(`stop at ${expectation.checkpoint}`),
      );
      const before = inspectControlledBackgroundProviderPrefix(
        slot.operation_plan.evidence_directory.path,
        slot.operation_plan.fixture_id,
        { expectedCreateBindings: bindings, providerBase: state.providerBase },
      );
      assert.equal(before.evidenceStage, expectation.evidenceStage);
      assert.equal(before.residual.controller_presence, expectation.controllerPresence);
      assert.equal(before.residual.hostagent_presence, expectation.hostagentPresence);
      assert.deepEqual(before.effectFrontier, {
        disposition: "complete",
        effect: expectation.evidenceStage,
      });
      const providerBefore = evidenceSnapshot(state);
      const confirmation = providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      });
      assert.throws(
        () =>
          recoverBackgroundProviderCreateForExecutor({
            adapter: adapter(),
            confirmation,
            providerBase: state.providerBase,
            repoRoot: state.repo,
            stateBase: state.state,
          }),
        /background provider effect remained uncertain/,
      );
      const claim = parse(join(active, ".mutation-recovery-00-00"));
      assert.equal(claim.observed_effect_disposition, "complete");
      assert.equal(claim.observed_effect_name, expectation.evidenceStage);
      assert.equal(existsSync(join(active, ".mutation-operation-00")), false);
      assert.equal(existsSync(join(active, ".mutation-close-00")), false);
      assert.equal(existsSync(join(active, "02-provider-create-passed.json")), false);
      assert.equal(existsSync(join(active, "02-provider-create-failed.json")), false);
      assert.deepEqual(evidenceSnapshot(state), providerBefore);
      const after = inspectControlledBackgroundProviderPrefix(
        slot.operation_plan.evidence_directory.path,
        slot.operation_plan.fixture_id,
        { expectedCreateBindings: bindings, providerBase: state.providerBase },
      );
      assert.equal(after.evidencePrefixSha256, before.evidencePrefixSha256);
      assert.equal(after.residualSha256, before.residualSha256);
      assert.equal(verify(state).status, 0);
    } finally {
      rmSync(state.root, { force: true, recursive: true });
    }
  }
});

test("source drift is fenced before root start pass and close boundaries", async (t) => {
  const cases = [
    {
      marker: (active) => join(active, "01-provider-create-intent.json"),
      mode: "source-drift-before-root",
      passReceipt: false,
      phase: "provider-create-failed",
      root: "absent",
      sequence: 2,
    },
    {
      marker: (active) => join(active, "provider", "controller-witness.json"),
      mode: "source-drift-before-start",
      passReceipt: false,
      phase: "provider-create-failed",
      root: "owned",
      sequence: 2,
    },
    {
      marker: (active) => join(active, ".mutation-operation-00"),
      mode: "source-drift-before-pass",
      passReceipt: false,
      phase: "execution-failed",
      root: "owned",
      sequence: 2,
    },
    {
      marker: (active) => join(active, "02-provider-create-passed.json"),
      mode: "source-drift-before-close",
      passReceipt: true,
      phase: "execution-failed",
      root: "owned",
      sequence: 3,
    },
  ];
  for (const scenario of cases) {
    await t.test(scenario.mode, async () => {
      const state = fixture();
      try {
        const owner = launchOwner(state, scenario.mode);
        const active = activeRun(state);
        await waitFor(scenario.marker(active), 12_000, owner);
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
        writeFileSync(join(state.repo, "source.txt"), "source drift\n");
        await expectOwnerSuccess(owner);
        writeFileSync(join(state.repo, "source.txt"), "fixture source\n");

        const close = parse(join(active, ".mutation-close-00"));
        const settlement = parse(join(active, ".mutation-operation-00"));
        const terminal = parse(
          join(
            active,
            `${String(scenario.sequence).padStart(2, "0")}-${scenario.phase}.json`,
          ),
        );
        const slot = parse(join(active, ".mutation-slot-00"));
        assert.equal(terminal.phase, scenario.phase);
        assert.equal(close.result_sequence, scenario.sequence);
        assert.equal(close.result_head_sha256,
          sha256(canonicalBytes(terminal)));
        assert.equal(
          existsSync(join(active, "02-provider-create-passed.json")),
          scenario.passReceipt,
        );
        assert.equal(settlement.root_disposition, scenario.root);
        assert.equal(
          existsSync(slot.operation_plan.provider_root_path),
          scenario.root === "owned",
        );
        await waitForHostagentExit(slot);
        assert.equal(verify(state).status, 0);
      } finally {
        rmSync(state.root, { force: true, recursive: true });
      }
    });
  }
});

test("recovery retires a staged intent and preserves a pre-intent foreign root", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-intent");
    const active = activeRun(state);
    const intentPath = join(active, "01-provider-create-intent.json");
    await waitFor(intentPath, 12_000, owner);
    await killOwner(owner);
    const slot = parse(join(active, ".mutation-slot-00"));
    const receiptStage = join(active, ".receipt-publish");
    linkSync(intentPath, receiptStage);
    unlinkSync(intentPath);
    assert.equal(existsSync(intentPath), false);
    assert.equal(lstatSync(receiptStage, { bigint: true }).nlink, 1n);
    assert.equal(sha256(readFileSync(receiptStage)), slot.intent_receipt_sha256);
    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    const foreign = join(slot.operation_plan.provider_root_path, "foreign");
    writeFileSync(foreign, "preserve\n", { mode: 0o600 });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: deterministicAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /matching dedicated executor/,
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-00-00")), false);
    assert.throws(
      () => executeProviderCreateForExecutor({
        adapter: deterministicAdapter(),
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider creation state was refused/,
    );
    const receipt = recoverBackgroundProviderCreateForExecutor({
      adapter: adapter(),
      confirmation,
      providerBase: state.providerBase,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const settlement = parse(join(active, ".mutation-operation-00"));
    const receiptBytes = readFileSync(join(active, "01-preflight-refused.json"));
    const closeBytes = readFileSync(join(active, ".mutation-close-00"));
    const close = JSON.parse(closeBytes);
    const claimBytes = readFileSync(join(active, ".mutation-recovery-00-00"));
    assert.equal(receipt.phase, "preflight-refused");
    assert.deepEqual(receipt.result, {
      cleanup_required: false,
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    });
    assert.equal(existsSync(receiptStage), false);
    assert.equal(existsSync(intentPath), false);
    assert.equal(settlement.authority, "recovery");
    assert.equal(settlement.authority_sha256, sha256(claimBytes));
    assert.equal(settlement.disposition, "exact-residual");
    assert.equal(settlement.effect_name, "provider-root-collision");
    assert.equal(settlement.safe_code, "resource-collision");
    assert.equal(settlement.source_head_sha256, slot.source_head_sha256);
    assert.equal(settlement.root_disposition, "ownership-pending");
    assert.equal(settlement.controller_presence, "not-started");
    assert.equal(settlement.hostagent_presence, "not-started");
    assert.equal(settlement.sockets, "uninspected");
    assert.equal(settlement.pending_evidence_publication, null);
    assert.deepEqual(settlement.pending_private_publications, []);
    assert.equal(close.result_sequence, 1);
    assert.equal(close.result_head_sha256, sha256(receiptBytes));
    assert.equal(
      close.operation_evidence_sha256,
      sha256(readFileSync(join(active, ".mutation-operation-00"))),
    );
    assert.equal(readFileSync(foreign, "utf8"), "preserve\n");
    assert.deepEqual(readdirSync(join(active, "provider")), []);
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a durable collision settlement does not depend on the foreign root afterward", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "foreign-collision-hold-after-settlement");
    const active = activeRun(state);
    await waitFor(join(active, "01-provider-create-intent.json"), 12_000, owner);
    const slot = parse(join(active, ".mutation-slot-00"));
    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    writeFileSync(
      join(slot.operation_plan.provider_root_path, "foreign-before"),
      "foreign before\n",
      { mode: 0o600 },
    );
    await waitFor(join(active, ".mutation-operation-00"), 12_000, owner);
    const settlementBytes = readFileSync(join(active, ".mutation-operation-00"));
    await killOwner(owner);
    rmSync(slot.operation_plan.provider_root_path, { recursive: true });

    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(receipt.phase, "provider-create-failed");
    assert.equal(close.disposition, "completed");
    assert.deepEqual(
      readFileSync(join(active, ".mutation-operation-00")),
      settlementBytes,
    );

    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    const replacement = join(slot.operation_plan.provider_root_path, "foreign-after");
    writeFileSync(replacement, "foreign after\n", { mode: 0o600 });
    assert.equal(verify(state).status, 0);
    assert.equal(readFileSync(replacement, "utf8"), "foreign after\n");
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a collision after create authority retains only Synveda evidence dependencies", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "foreign-collision-after-authority");
    const active = activeRun(state);
    const authorityPath = join(
      active,
      "provider",
      "background-create-authority.json",
    );
    await waitFor(authorityPath, 12_000, owner);
    const authorityBytes = readFileSync(authorityPath);
    const slot = parse(join(active, ".mutation-slot-00"));
    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    writeFileSync(
      join(slot.operation_plan.provider_root_path, "foreign-before"),
      "foreign before authority resume\n",
      { mode: 0o600 },
    );
    await waitFor(join(active, ".mutation-operation-00"), 12_000, owner);
    await killOwner(owner);
    rmSync(slot.operation_plan.provider_root_path, { recursive: true });

    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const settlement = parse(join(active, ".mutation-operation-00"));
    assert.equal(receipt.phase, "provider-create-failed");
    assert.equal(settlement.evidence_stage, "create-authority");
    assert.equal(settlement.safe_code, "resource-collision");
    assert.deepEqual(readFileSync(authorityPath), authorityBytes);
    assert.equal(existsSync(join(active, ".mutation-close-00")), true);

    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    const replacement = join(slot.operation_plan.provider_root_path, "foreign-after");
    writeFileSync(replacement, "foreign after authority\n", { mode: 0o600 });
    assert.equal(verify(state).status, 0);
    assert.equal(readFileSync(replacement, "utf8"), "foreign after authority\n");
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a staged create authority collision remains historical after close", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-authority");
    const active = activeRun(state);
    const providerDirectory = join(active, "provider");
    const authorityPath = join(providerDirectory, "background-create-authority.json");
    await waitFor(authorityPath, 12_000, owner);
    await killOwner(owner);
    const authorityBytes = readFileSync(authorityPath);
    const authoritySha256 = sha256(authorityBytes);
    const stagePath = join(
      providerDirectory,
      `.provider-process-stage-background-create-authority-${authoritySha256}-${"b".repeat(32)}`,
    );
    linkSync(authorityPath, stagePath);
    unlinkSync(authorityPath);
    const slot = parse(join(active, ".mutation-slot-00"));
    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    const foreign = join(slot.operation_plan.provider_root_path, "foreign");
    writeFileSync(foreign, "foreign staged authority\n", { mode: 0o600 });

    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const settlement = parse(join(active, ".mutation-operation-00"));
    assert.equal(receipt.phase, "provider-create-failed");
    assert.equal(settlement.evidence_stage, "empty");
    assert.equal(settlement.pending_evidence_publication.target_name,
      "background-create-authority.json");
    assert.equal(settlement.pending_evidence_publication.disposition,
      "staged-complete");
    assert.equal(existsSync(authorityPath), false);
    assert.equal(lstatSync(stagePath, { bigint: true }).nlink, 1n);
    assert.equal(existsSync(join(active, ".mutation-close-00")), true);

    rmSync(slot.operation_plan.provider_root_path, { recursive: true });
    assert.equal(verify(state).status, 0);
    mkdirSync(slot.operation_plan.provider_root_path, { mode: 0o700 });
    writeFileSync(foreign, "foreign replacement\n", { mode: 0o600 });
    assert.equal(verify(state).status, 0);
    assert.equal(readFileSync(foreign, "utf8"), "foreign replacement\n");
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("recovery retires a source-vetoed staged intent and aborts before effect", async () => {
  const state = fixture();
  try {
    const owner = launchOwner(state, "hold-after-intent");
    const active = activeRun(state);
    const intentPath = join(active, "01-provider-create-intent.json");
    await waitFor(intentPath, 12_000, owner);
    await killOwner(owner);
    const receiptStage = join(active, ".receipt-publish");
    linkSync(intentPath, receiptStage);
    unlinkSync(intentPath);
    writeFileSync(join(state.repo, "source.txt"), "source drift\n");

    const receipt = recoverBackgroundProviderCreateForExecutor(recoveryArguments(state));
    const close = parse(join(active, ".mutation-close-00"));
    const slotBytes = readFileSync(join(active, ".mutation-slot-00"));
    const slot = JSON.parse(slotBytes);
    assert.equal(receipt.phase, "plan");
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(existsSync(receiptStage), false);
    assert.equal(existsSync(intentPath), false);
    assert.equal(existsSync(join(active, ".mutation-operation-00")), false);
    assert.deepEqual(readdirSync(join(active, "provider")), []);
    assert.throws(
      () =>
        planControlledBackgroundProviderCreateWithAuthorityGate(
          {
            bindings: stateCreateBindings(slotBytes, slot),
            evidenceDirectory: slot.operation_plan.evidence_directory.path,
            fixtureId: slot.operation_plan.fixture_id,
            operationPlan: slot.operation_plan,
            providerBase: slot.operation_plan.provider_base.path,
          },
          () => undefined,
        ),
      /state operation was already settling/,
    );
    assert.deepEqual(readdirSync(join(active, "provider")), []);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
    assert.equal(verify(state).status, 0);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

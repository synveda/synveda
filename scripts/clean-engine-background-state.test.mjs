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
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import {
  executeBackgroundProviderCreateForExecutor,
  executeProviderCreateForExecutor,
  providerRecoveryConfirmationForExecutor,
  recoverBackgroundProviderCreateForExecutor,
  recoverProviderCreateForExecutor,
} from "../deploy/compose/scripts/clean-engine-state.mjs";
import { canonicalBytes } from "../deploy/compose/scripts/clean-engine-receipts.mjs";
import {
  controlledBackgroundOperationEvidence,
  inspectControlledBackgroundProvider,
  inspectControlledBackgroundProviderPrefix,
  launchControlledBackgroundProvider,
  launchControlledBackgroundProviderWithAuthorityGate,
  planControlledBackgroundProviderCreateWithAuthorityGate,
  planControlledBackgroundRetirement,
  retireControlledBackgroundProvider,
} from "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");
const executeFixture = resolve("scripts/fixtures/execute-clean-engine-background-provider.mjs");

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
    assert.equal(verify(state).status, 0);
    await waitForHostagentExit(slot);
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

#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  readdirSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import {
  appendProviderCleanupReceiptForExecutor,
  appendReceiptForExecutor,
  executeProviderCreateForExecutor,
  finalizeEnvironmentForExecutor,
  providerRecoveryConfirmationForExecutor,
  recoverProviderCreateForExecutor,
} from "../deploy/compose/scripts/clean-engine-state.mjs";
import {
  canonicalBytes,
  createFinalization,
  createNextReceipt,
  receiptFileName,
  receiptSuccessPath,
  sha256,
} from "../deploy/compose/scripts/clean-engine-receipts.mjs";
import { cleanEngineReceiptResult } from "./fixtures/clean-engine-receipt-fixture.mjs";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");

function command(binary, args, options = {}) {
  return spawnSync(binary, args, {
    encoding: "utf8",
    env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
    ...options,
  });
}

function git(repo, args) {
  const result = command("git", ["-C", repo, ...args]);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

function fixture() {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "synveda-clean-engine-state-")));
  chmodSync(root, 0o700);
  const repo = join(root, "repo");
  const state = join(root, "state");
  mkdirSync(repo, { mode: 0o700 });
  mkdirSync(state, { mode: 0o700 });
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
    ".gitignore":
      ".claude/\n.codex/\ntarget/\nnode_modules/\n" +
      "evals/fixtures/longmemeval/longmemeval_*.json\n" +
      "evals/fixtures/longmemeval/LICENSE\n" +
      "evals/fixtures/longmemeval/LICENSE.*\n",
    ".claude/RESUME.md": "local harness recovery state\n",
    ".env.example": "NON_SECRET_EXAMPLE=true\n",
    ".env.secret.example": "excluded example-shaped residue\n",
    Makefile: "compose-config:\n\t@true\n",
    "deploy/compose/compose.yaml": "name: fixture\nservices: {}\n",
    "docs/DEPLOYMENT_CONTRACT.md": "# Fixture deployment contract\n",
    "docs/SECURITY.md": "# Fixture security contract\n",
    "evals/fixtures/longmemeval/LICENSE": "upstream fixture licence\n",
    "evals/fixtures/longmemeval/LICENSE.txt": "upstream fixture licence variant\n",
    "evals/fixtures/longmemeval/NOTICE.md": "# Tracked corpus notice\n",
    "evals/fixtures/longmemeval/longmemeval_s_cleaned.json": "[]\n",
    "source.txt": "fixture source\n",
  };
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(repo, path)), { recursive: true, mode: 0o700 });
    writeFileSync(join(repo, path), content, { mode: 0o600 });
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
  return { root, repo, state };
}

function run(state, action, extra = []) {
  return command(process.execPath, toolArgs(state, action, extra));
}

function toolArgs(state, action, extra = []) {
  const args = [
    stateTool,
    action,
    "--repo-root",
    state.repo,
    "--state-base",
    state.state,
  ];
  if (action === "plan") {
    args.push(
      "--ipv4-pool",
      "10.239.17.0/24",
      "--provider",
      "colima",
    );
  }
  args.push(...extra);
  return args;
}

function parse(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function assertPrivate(path, mode, directory = false, links = 1) {
  const metadata = lstatSync(path);
  assert.equal(metadata.isSymbolicLink(), false);
  assert.equal(directory ? metadata.isDirectory() : metadata.isFile(), true);
  if (!directory) assert.equal(metadata.nlink, links);
  assert.equal(metadata.mode & 0o777, mode);
  assert.equal(metadata.uid, process.getuid());
}

function activeRun(state) {
  const receipt = parse(join(state.state, "active"));
  return join(state.state, `.run-${receipt.fixture_id}`);
}

function mutationLease(state, {
  action = "append-receipt",
  intentReceiptSha256 = "0".repeat(64),
  journalSequence = 0,
  ownerPid = 2_147_483_647,
  previousCloseSha256 = "0".repeat(64),
} = {}) {
  const active = activeRun(state);
  const candidate = parse(join(active, "candidate.json"));
  const planReceipt = parse(join(active, "00-plan.json"));
  return {
    action,
    fixture_id: candidate.run_id,
    intent_receipt_sha256: intentReceiptSha256,
    journal_sequence: journalSequence,
    nonce: "f".repeat(32),
    owner_boot_sha256: "a".repeat(64),
    owner_instance_sha256: "b".repeat(64),
    owner_pid: ownerPid,
    owner_probe: "opaque-process-instance-v1",
    previous_close_sha256: previousCloseSha256,
    schema: "synveda.clean-engine.mutation-slot.v1",
    source_environment_sha256: "0".repeat(64),
    source_head_sha256: sha256(canonicalBytes(planReceipt)),
    source_sequence: 0,
  };
}

function fakeProviderAdapter({
  closePrelinkHoldMilliseconds = 0,
  executeOutcome = "passed",
  executeResult = cleanEngineReceiptResult("provider-create-passed"),
  holdMilliseconds = 0,
  prelinkHoldMilliseconds = 0,
  publicationHoldMilliseconds = 0,
  reconcileHoldMilliseconds = 0,
  reconcileOutcome = "passed",
  reconcileResult = cleanEngineReceiptResult("provider-create-passed"),
} = {}) {
  return {
    close_prelink_hold_milliseconds: closePrelinkHoldMilliseconds,
    execute_outcome: executeOutcome,
    execute_result: executeResult,
    hold_milliseconds: holdMilliseconds,
    kind: "deterministic-fake-provider-v1",
    prelink_hold_milliseconds: prelinkHoldMilliseconds,
    publication_hold_milliseconds: publicationHoldMilliseconds,
    reconcile_hold_milliseconds: reconcileHoldMilliseconds,
    reconcile_outcome: reconcileOutcome,
    reconcile_result: reconcileResult,
  };
}

function stageAbandonedProviderLease(state, {
  ownerPid = 2_147_483_647,
  providerContractSha256 =
    "644704a6fccc5867c9987d6a971a980086d7fe77712ca4f892ae4aef839fd799",
  publishIntent = true,
} = {}) {
  const active = activeRun(state);
  const candidate = parse(join(active, "candidate.json"));
  const planReceipt = parse(join(active, "00-plan.json"));
  const intentResult = {
    ...cleanEngineReceiptResult("provider-create-intent", candidate.run_id),
    provider_contract_sha256: providerContractSha256,
  };
  const intentReceipt = createNextReceipt(
    [planReceipt],
    candidate.run_id,
    "provider-create-intent",
    intentResult,
  );
  if (publishIntent) {
    writeFileSync(join(active, receiptFileName(intentReceipt)), canonicalBytes(intentReceipt), {
      mode: 0o600,
    });
  }
  const lease = mutationLease(state, {
    action: "provider-create",
    intentReceiptSha256: sha256(canonicalBytes(intentReceipt)),
    ownerPid,
  });
  const leaseBytes = canonicalBytes(lease);
  writeFileSync(join(active, ".mutation-slot-00"), leaseBytes, { mode: 0o600 });
  return { active, candidate, intentReceipt, lease, leaseBytes, planReceipt };
}

function stageAbandonedProviderMutation(state, options) {
  const staged = stageAbandonedProviderLease(state, options);
  const { active, candidate, intentReceipt, leaseBytes } = staged;
  const recovery = {
    action: "provider-create",
    chain_root_sha256: sha256(canonicalBytes({
      action: "provider-create",
      fixture_id: candidate.run_id,
      lease_sha256: sha256(leaseBytes),
      schema: "synveda.clean-engine.mutation-recovery-root.v1",
    })),
    fixture_id: candidate.run_id,
    lease_sha256: sha256(leaseBytes),
    nonce: "e".repeat(32),
    owner_boot_sha256: "c".repeat(64),
    owner_instance_sha256: "d".repeat(64),
    owner_pid: 2_147_483_646,
    owner_probe: "opaque-process-instance-v1",
    parent_sha256: "0".repeat(64),
    schema: "synveda.clean-engine.mutation-recovery.v1",
    sequence: 0,
    slot_sequence: 0,
    source_head_sha256: sha256(canonicalBytes(intentReceipt)),
  };
  writeFileSync(join(active, ".mutation-recovery-00-00"), canonicalBytes(recovery), { mode: 0o600 });
  return { ...staged, recovery };
}

function recoveryClaimForSlot(state, {
  ownerPid = 2_147_483_645,
  previous,
  sequence = 0,
  slotSequence = 0,
  sourceHeadSha256,
} = {}) {
  const active = activeRun(state);
  const candidate = parse(join(active, "candidate.json"));
  const slotBytes = readFileSync(
    join(active, `.mutation-slot-${String(slotSequence).padStart(2, "0")}`),
  );
  const receiptNames = readdirSync(active)
    .filter((name) => /^[0-9]{2}-[a-z][a-z0-9-]*\.json$/.test(name))
    .sort();
  const head = parse(join(active, receiptNames.at(-1)));
  return {
    action: "provider-create",
    chain_root_sha256: sha256(canonicalBytes({
      action: "provider-create",
      fixture_id: candidate.run_id,
      lease_sha256: sha256(slotBytes),
      schema: "synveda.clean-engine.mutation-recovery-root.v1",
    })),
    fixture_id: candidate.run_id,
    lease_sha256: sha256(slotBytes),
    nonce: ((sequence + 1) % 16).toString(16).repeat(32),
    owner_boot_sha256: "8".repeat(64),
    owner_instance_sha256: "9".repeat(64),
    owner_pid: ownerPid,
    owner_probe: "opaque-process-instance-v1",
    parent_sha256: previous === undefined ? "0".repeat(64) : sha256(canonicalBytes(previous)),
    schema: "synveda.clean-engine.mutation-recovery.v1",
    sequence,
    slot_sequence: slotSequence,
    source_head_sha256: sourceHeadSha256 ?? sha256(canonicalBytes(head)),
  };
}

async function waitForProviderIntent(state, timeoutMilliseconds = 8_000) {
  const path = join(activeRun(state), "01-provider-create-intent.json");
  const deadline = Date.now() + timeoutMilliseconds;
  while (!existsSync(path)) {
    assert.ok(Date.now() < deadline, "timed out waiting for fake provider intent");
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
}

async function waitForMutationPublicationStage(state, timeoutMilliseconds = 8_000) {
  const active = activeRun(state);
  const lease = join(active, ".mutation-slot-00");
  const deadline = Date.now() + timeoutMilliseconds;
  while (
    !existsSync(lease) ||
    !readdirSync(active).some((name) => name.startsWith(".mutation-stage-"))
  ) {
    assert.ok(Date.now() < deadline, "timed out waiting for linked mutation publication");
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
}

async function waitForUnlinkedCloseStage(
  state,
  terminalReceipt = "02-provider-create-passed.json",
  timeoutMilliseconds = 8_000,
) {
  const active = activeRun(state);
  const terminal = join(active, terminalReceipt);
  const close = join(active, ".mutation-close-00");
  const deadline = Date.now() + timeoutMilliseconds;
  while (true) {
    const stage = readdirSync(active)
      .filter((name) => name.startsWith(".mutation-stage-"))
      .map((name) => join(active, name))
      .find((path) => {
        try {
          return lstatSync(path).nlink === 1;
        } catch (error) {
          if (error?.code === "ENOENT") return false;
          throw error;
        }
      });
    if (existsSync(terminal) && !existsSync(close) && stage !== undefined) return stage;
    assert.ok(Date.now() < deadline, "timed out waiting for unlinked close publication");
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
}

async function waitForRecoveryClaim(state, slotSequence, sequence, timeoutMilliseconds = 8_000) {
  const active = activeRun(state);
  const path = join(
    active,
    `.mutation-recovery-${String(slotSequence).padStart(2, "0")}-${String(sequence).padStart(2, "0")}`,
  );
  const deadline = Date.now() + timeoutMilliseconds;
  while (
    !existsSync(path) ||
    lstatSync(path).nlink !== 1 ||
    existsSync(join(active, ".receipt-publish")) ||
    readdirSync(active).some((name) => name.startsWith(".mutation-stage-"))
  ) {
    assert.ok(Date.now() < deadline, "timed out waiting for provider recovery claim");
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
}

function stageSuccessfulReceipts(state) {
  const active = activeRun(state);
  const candidate = parse(join(active, "candidate.json"));
  executeProviderCreateForExecutor({
    adapter: fakeProviderAdapter(),
    repoRoot: state.repo,
    stateBase: state.state,
  });
  const receipts = [
    parse(join(active, "00-plan.json")),
    parse(join(active, "01-provider-create-intent.json")),
    parse(join(active, "02-provider-create-passed.json")),
  ];
  for (const phase of receiptSuccessPath.slice(3, -1)) {
    const append = phase.startsWith("provider-cleanup-")
      ? appendProviderCleanupReceiptForExecutor
      : appendReceiptForExecutor;
    const receipt = append({
      phase,
      repoRoot: state.repo,
      result: cleanEngineReceiptResult(phase, candidate.run_id),
      stateBase: state.state,
    });
    receipts.push(receipt);
  }
  return { active, candidate, receipts };
}

test("plan publishes one canonical content-free candidate, receipt and proxy template", () => {
  const state = fixture();
  try {
    mkdirSync(join(state.repo, ".codex"), { mode: 0o700 });
    writeFileSync(join(state.repo, ".codex", "journal.md"), "ignored journal\n", { mode: 0o600 });
    const result = run(state, "plan");
    assert.equal(result.status, 0, result.stderr);
    assert.match(
      result.stdout,
      /^clean-engine: plan [0-9a-f]{32} prepared for synveda-development-acceptance-[0-9a-f]{24}\n$/,
    );
    assert.equal(result.stderr, "");

    const active = activeRun(state);
    const candidatePath = join(active, "candidate.json");
    const receiptPath = join(active, "00-plan.json");
    const proxyPath = join(active, "client", "proxy-template.json");
    assertPrivate(active, 0o700, true);
    assertPrivate(candidatePath, 0o600);
    assertPrivate(receiptPath, 0o600, false, 2);
    assertPrivate(join(state.state, "active"), 0o600, false, 2);
    assert.equal(lstatSync(receiptPath).ino, lstatSync(join(state.state, "active")).ino);
    const exactRun = lstatSync(active, { bigint: true });
    assertPrivate(proxyPath, 0o600);

    const candidateRaw = readFileSync(candidatePath, "utf8");
    const receiptRaw = readFileSync(receiptPath, "utf8");
    const proxyRaw = readFileSync(proxyPath, "utf8");
    for (const raw of [candidateRaw, receiptRaw]) {
      assert.equal(raw.endsWith("\n"), true);
      assert.doesNotMatch(
        raw,
        /password|private[_ -]?key|authorization|bearer|cookie|token|database_url|Users\/|home\//i,
      );
    }
    const candidate = JSON.parse(candidateRaw);
    const receipt = JSON.parse(receiptRaw);
    assert.equal(candidate.run_id, receipt.fixture_id);
    assert.equal(candidate.selection.project_suffix, `acceptance-${candidate.run_id.slice(0, 24)}`);
    assert.equal(candidate.selection.ipv4_pool, "10.239.17.0/24");
    assert.equal(candidate.fixtures.registry_transport, "loopback-tls-ephemeral");
    assert.equal(candidate.source.worktree_clean, true);
    assert.match(candidate.source.build_context_manifest_sha256, /^[0-9a-f]{64}$/);
    assert.match(candidate.source.tracked_index_manifest_sha256, /^[0-9a-f]{64}$/);
    assert.equal(receipt.result.state_device, String(exactRun.dev));
    assert.equal(receipt.result.state_inode, String(exactRun.ino));
    assert.deepEqual(JSON.parse(proxyRaw), {
      auths: {},
      proxies: {
        default: {
          allProxy: "socks5://all-proxy-canary.invalid:65535",
          ftpProxy: "http://ftp-proxy-canary.invalid:65535",
          httpProxy: "http://http-proxy-canary.invalid:65535",
          httpsProxy: "http://https-proxy-canary.invalid:65535",
          noProxy: "proxy-bypass-canary.invalid",
        },
      },
    });

    const status = run(state, "status");
    assert.equal(status.status, 0, status.stderr);
    assert.match(status.stdout, / is prepared\n$/);
    const verify = run(state, "verify");
    assert.equal(verify.status, 0, verify.stderr);
    assert.match(verify.stdout, / is source-verified\n$/);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("one active plan is a durable lease and cannot be replaced", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const original = readFileSync(join(activeRun(state), "candidate.json"));
    const second = run(state, "plan");
    assert.equal(second.status, 73);
    assert.equal(second.stdout, "");
    assert.equal(second.stderr, "clean-engine: an active clean-engine plan already exists\n");
    assert.deepEqual(readFileSync(join(activeRun(state), "candidate.json")), original);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the state loader appends one exclusive generic receipt and rejects receipt drift", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const result = cleanEngineReceiptResult("registry-intent", candidate.run_id);
    const receipt = appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result,
      stateBase: state.state,
    });
    const receiptPath = join(active, "03-registry-intent.json");
    assert.equal(receipt.sequence, 3);
    assertPrivate(receiptPath, 0o600);
    assert.equal(run(state, "status").status, 0);
    const retried = appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result,
      stateBase: state.state,
    });
    assert.deepEqual(retried, receipt);
    assert.throws(() => appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result: { ...result, safe_code: "different-safe-code" },
      stateBase: state.state,
    }), /completed receipt result did not match retry/);

    const mutated = parse(receiptPath);
    mutated.previous_sha256 = "f".repeat(64);
    const ordered = Object.fromEntries(
      Object.entries(mutated).sort(([left], [right]) => left.localeCompare(right)),
    );
    writeFileSync(receiptPath, `${JSON.stringify(ordered)}\n`, { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: receipt chain was refused\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("receipt publication outside an open mutation slot is retained and refused", () => {
  for (const crashPoint of ["partial-staging", "complete-staging", "published-link"]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const active = activeRun(state);
      const candidate = parse(join(active, "candidate.json"));
      const planReceipt = parse(join(active, "00-plan.json"));
      const result = cleanEngineReceiptResult("preflight-refused", candidate.run_id);
      const receipt = createNextReceipt(
        [planReceipt],
        candidate.run_id,
        "preflight-refused",
        result,
      );
      const staging = join(active, ".receipt-publish");
      const destination = join(active, "01-preflight-refused.json");
      if (crashPoint === "partial-staging") {
        writeFileSync(staging, "{", { mode: 0o600 });
      } else {
        writeFileSync(staging, canonicalBytes(receipt), { mode: 0o600 });
        if (crashPoint === "published-link") linkSync(staging, destination);
      }
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /outside (?:an open mutation slot|the mutation journal)/);
      if (crashPoint !== "published-link") assert.equal(existsSync(destination), false);
      assertPrivate(staging, 0o600, false, crashPoint === "published-link" ? 2 : 1);
      if (crashPoint === "published-link") assertPrivate(destination, 0o600, false, 2);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("canonical mismatched publication stages are retained and refused", () => {
  const receiptState = fixture();
  try {
    assert.equal(run(receiptState, "plan").status, 0);
    const active = activeRun(receiptState);
    const candidate = parse(join(active, "candidate.json"));
    const planReceipt = parse(join(active, "00-plan.json"));
    const result = cleanEngineReceiptResult("preflight-refused", candidate.run_id);
    const staged = createNextReceipt(
      [planReceipt],
      candidate.run_id,
      "preflight-refused",
      result,
    );
    staged.previous_sha256 = "f".repeat(64);
    const staging = join(active, ".receipt-publish");
    writeFileSync(staging, canonicalBytes(staged), { mode: 0o600 });
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: receiptState.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: receiptState.state,
      }),
      /pending receipt publication was outside an open mutation slot/,
    );
    assertPrivate(staging, 0o600);
    assert.equal(existsSync(join(active, "01-preflight-refused.json")), false);
  } finally {
    rmSync(receiptState.root, { recursive: true, force: true });
  }

  const environmentState = fixture();
  try {
    assert.equal(run(environmentState, "plan").status, 0);
    const { active, candidate, receipts } = stageSuccessfulReceipts(environmentState);
    const manifest = createFinalization(
      candidate,
      canonicalBytes(candidate),
      receipts,
    ).manifest;
    const staging = join(active, ".environment-publish");
    writeFileSync(join(active, "environment.json"), canonicalBytes(manifest), { mode: 0o600 });
    writeFileSync(staging, canonicalBytes({ ...manifest, unreviewed: true }), { mode: 0o600 });
    assert.throws(
      () => finalizeEnvironmentForExecutor({
        repoRoot: environmentState.repo,
        stateBase: environmentState.state,
      }),
      /pending environment publication was outside a finalization slot/,
    );
    assertPrivate(staging, 0o600);
    assertPrivate(join(active, "environment.json"), 0o600);
  } finally {
    rmSync(environmentState.root, { recursive: true, force: true });
  }
});

test("source drift after a positive receipt durably enters cleanup-only failure", () => {
  const state = fixture();
  const originalPath = process.env.PATH;
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const result = cleanEngineReceiptResult("registry-intent", candidate.run_id);
    const resolvedGit = command("sh", ["-c", "command -v git"]).stdout.trim();
    assert.match(resolvedGit, /^\//);
    const bin = join(state.root, "fake-bin");
    const marker = join(state.root, "drift-triggered");
    const drift = join(state.repo, "post-publication-drift.txt");
    mkdirSync(bin, { mode: 0o700 });
    const wrapper = join(bin, "git");
    writeFileSync(
      wrapper,
      "#!/bin/sh\n" +
        `if [ -f ${JSON.stringify(join(active, "03-registry-intent.json"))} ] && ` +
        `[ ! -f ${JSON.stringify(marker)} ]; then\n` +
        `  printf '%s\\n' drift > ${JSON.stringify(drift)}\n` +
        `  : > ${JSON.stringify(marker)}\n` +
        "fi\n" +
        `exec ${JSON.stringify(resolvedGit)} \"$@\"\n`,
      { mode: 0o700 },
    );
    process.env.PATH = `${bin}:${originalPath}`;
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result,
        stateBase: state.state,
      }),
      /source worktree is not clean/,
    );
    process.env.PATH = originalPath;
    assertPrivate(join(active, "03-registry-intent.json"), 0o600);
    assertPrivate(join(active, "04-execution-failed.json"), 0o600);
    assert.equal(existsSync(join(active, "environment.json")), false);
    rmSync(drift);
    const cleanup = appendReceiptForExecutor({
      phase: "failure-cleanup-intent",
      repoRoot: state.repo,
      result: {
        authorized_resources: ["provider", "registry", "runtime-secrets"],
        scope: "exact-receipt-owned-only",
      },
      stateBase: state.state,
    });
    assert.equal(cleanup.phase, "failure-cleanup-intent");
    assert.equal(run(state, "verify").status, 0);
  } finally {
    process.env.PATH = originalPath;
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("one atomic mutation slot serializes concurrent provider executors", async () => {
  const concurrent = fixture();
  try {
    assert.equal(run(concurrent, "plan").status, 0);
    const helper = resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs");
    const launch = () => {
      const child = spawn(
        process.execPath,
        [helper, concurrent.repo, concurrent.state, "hold"],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk) => { stdout += chunk; });
      child.stderr.on("data", (chunk) => { stderr += chunk; });
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ signal, status, stderr, stdout }));
      });
    };
    const results = await Promise.all([launch(), launch()]);
    assert.deepEqual(results.map((value) => value.status).sort((a, b) => a - b), [0, 73]);
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(
      readdirSync(activeRun(concurrent)).filter((name) => /^01-.*\.json$/.test(name)).length,
      1,
    );
    assertPrivate(join(activeRun(concurrent), ".mutation-slot-00"), 0o600);
    assertPrivate(join(activeRun(concurrent), ".mutation-close-00"), 0o600);
    assert.equal(run(concurrent, "verify").status, 0);
  } finally {
    rmSync(concurrent.root, { recursive: true, force: true });
  }
});

test("an abandoned mutation slot is retained for explicit recovery", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    writeFileSync(
      join(active, ".mutation-slot-00"),
      canonicalBytes(mutationLease(state)),
      { mode: 0o600 },
    );
    assert.equal(run(state, "status").status, 0);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /abandoned clean-engine mutation requires explicit recovery/,
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assert.equal(existsSync(join(active, "01-provider-create-intent.json")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a live mutation slot is content-free state and refuses a competing writer", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    writeFileSync(
      join(active, ".mutation-slot-00"),
      canonicalBytes(mutationLease(state, {
        action: "finalize-environment",
        ownerPid: process.ppid,
      })),
      { mode: 0o600 },
    );
    assert.equal(run(state, "status").status, 0);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /another clean-engine mutation is active or could not be identified/,
    );
    assert.equal(existsSync(join(active, "01-provider-create-intent.json")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("mutation lease v1 is a hard-cut refusal", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    writeFileSync(
      join(active, ".mutation-lease"),
      canonicalBytes({
        action: "append-receipt",
        fixture_id: candidate.run_id,
        nonce: "f".repeat(32),
        pid: 2_147_483_647,
        schema: "synveda.clean-engine.mutation-lease.v1",
      }),
      { mode: 0o600 },
    );
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(
      refused.stderr,
      "clean-engine: legacy mutation lease was refused; prepare a fresh clean-engine plan\n",
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("mutation close v1 and non-provider operation evidence are hard-cut refusals", () => {
  const legacy = fixture();
  try {
    assert.equal(run(legacy, "plan").status, 0);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: legacy.repo,
      stateBase: legacy.state,
    });
    const closePath = join(activeRun(legacy), ".mutation-close-00");
    const close = parse(closePath);
    close.schema = "synveda.clean-engine.mutation-close.v1";
    writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
    const refused = run(legacy, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: mutation close was refused\n");
  } finally {
    rmSync(legacy.root, { recursive: true, force: true });
  }

  const evidence = fixture();
  try {
    assert.equal(run(evidence, "plan").status, 0);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: evidence.repo,
      stateBase: evidence.state,
    });
    const active = activeRun(evidence);
    const candidate = parse(join(active, "candidate.json"));
    appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: evidence.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: evidence.state,
    });
    const closePath = join(active, ".mutation-close-01");
    const close = parse(closePath);
    close.operation_evidence_sha256 = "f".repeat(64);
    writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
    const refused = run(evidence, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: mutation close receipt binding was refused\n");
  } finally {
    rmSync(evidence.root, { recursive: true, force: true });
  }
});

test("mutation slots and recovery claims refuse links modes gaps and field drift", () => {
  const mutations = [
    ({ active }) => chmodSync(join(active, ".mutation-slot-00"), 0o644),
    ({ active }) => linkSync(
      join(active, ".mutation-slot-00"),
      join(dirname(dirname(active)), "lease-hardlink"),
    ),
    ({ active }) => chmodSync(join(active, ".mutation-recovery-00-00"), 0o644),
    ({ active }) => linkSync(
      join(active, ".mutation-recovery-00-00"),
      join(dirname(dirname(active)), "recovery-hardlink"),
    ),
    ({ active, recovery }) => {
      writeFileSync(join(active, ".mutation-recovery-00-02"), canonicalBytes({
        ...recovery,
        parent_sha256: sha256(canonicalBytes(recovery)),
        sequence: 2,
      }), { mode: 0o600 });
    },
    ({ active, recovery }) => writeFileSync(
      join(active, ".mutation-recovery-00-00"),
      canonicalBytes({ ...recovery, unreviewed: true }),
      { mode: 0o600 },
    ),
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const staged = stageAbandonedProviderMutation(state);
      assert.equal(run(state, "status").status, 0);
      mutate(staged);
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /^clean-engine: /);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("slot predecessor and close authority corruption are fail-closed", () => {
  const mutations = [
    (state, active) => {
      const closePath = join(active, ".mutation-close-00");
      const close = parse(closePath);
      close.authority_sha256 = "f".repeat(64);
      writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
    },
    (state, active) => {
      const candidate = parse(join(active, "candidate.json"));
      appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      });
      const slotPath = join(active, ".mutation-slot-01");
      const slot = parse(slotPath);
      slot.previous_close_sha256 = "f".repeat(64);
      writeFileSync(slotPath, canonicalBytes(slot), { mode: 0o600 });
    },
    (_state, active) => {
      const slot = parse(join(active, ".mutation-slot-00"));
      slot.journal_sequence = 2;
      writeFileSync(join(active, ".mutation-slot-02"), canonicalBytes(slot), { mode: 0o600 });
    },
    (_state, active) => {
      const close = parse(join(active, ".mutation-close-00"));
      close.slot_sequence = 1;
      writeFileSync(join(active, ".mutation-close-01"), canonicalBytes(close), { mode: 0o600 });
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      executeProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        repoRoot: state.repo,
        stateBase: state.state,
      });
      const active = activeRun(state);
      mutate(state, active);
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /^clean-engine: /);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("the one-dimensional recovery basename is a hard-cut refusal", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    writeFileSync(join(activeRun(state), ".mutation-recovery-00"), "{}\n", { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: plan run inventory was refused\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("inert mutation publication stages retire before mutation", () => {
  for (const [suffix, bytes] of [
    ["1".repeat(32), Buffer.from("{", "utf8")],
    ["2".repeat(32), canonicalBytes({ inert: true })],
  ]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const active = activeRun(state);
      const candidate = parse(join(active, "candidate.json"));
      executeProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        repoRoot: state.repo,
        stateBase: state.state,
      });
      const stage = join(active, `.mutation-stage-${suffix}`);
      writeFileSync(stage, bytes, { mode: 0o600 });
      assert.equal(run(state, "status").status, 0);
      const receipt = appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      });
      assert.equal(receipt.phase, "registry-intent");
      assert.equal(existsSync(stage), false);
      assertPrivate(join(active, ".mutation-slot-01"), 0o600);
      assertPrivate(join(active, ".mutation-close-01"), 0o600);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("mutation publication staging is bounded and excess evidence is retained", () => {
  const accepted = fixture();
  try {
    assert.equal(run(accepted, "plan").status, 0);
    const active = activeRun(accepted);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: accepted.repo,
      stateBase: accepted.state,
    });
    for (let index = 0; index < 16; index += 1) {
      writeFileSync(
        join(active, `.mutation-stage-${index.toString(16).padStart(32, "0")}`),
        "{",
        { mode: 0o600 },
      );
    }
    assert.equal(run(accepted, "status").status, 0);
    const candidate = parse(join(active, "candidate.json"));
    assert.equal(appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: accepted.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: accepted.state,
    }).phase, "registry-intent");
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-stage-")), false);
  } finally {
    rmSync(accepted.root, { recursive: true, force: true });
  }

  const excess = fixture();
  try {
    assert.equal(run(excess, "plan").status, 0);
    const active = activeRun(excess);
    for (let index = 0; index < 17; index += 1) {
      writeFileSync(
        join(active, `.mutation-stage-${index.toString(16).padStart(32, "0")}`),
        "{",
        { mode: 0o600 },
      );
    }
    const refused = run(excess, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: plan run inventory was refused\n");
    assert.equal(
      readdirSync(active).filter((name) => name.startsWith(".mutation-stage-")).length,
      17,
    );
  } finally {
    rmSync(excess.root, { recursive: true, force: true });
  }
});

test("a linked slot stage retires without removing its published blocker", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const stage = join(active, `.mutation-stage-${"3".repeat(32)}`);
    const lease = join(active, ".mutation-slot-00");
    writeFileSync(stage, canonicalBytes(mutationLease(state)), { mode: 0o600 });
    linkSync(stage, lease);
    assert.equal(run(state, "status").status, 0);
    assertPrivate(stage, 0o600, false, 2);
    assertPrivate(lease, 0o600, false, 2);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(active, "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /abandoned clean-engine mutation requires explicit recovery/,
    );
    assert.equal(existsSync(stage), false);
    assertPrivate(lease, 0o600);
    assert.equal(run(state, "status").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a linked recovery stage retires while the exact claim remains authoritative", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderMutation(state);
    const claim = join(active, ".mutation-recovery-00-00");
    const claimBytes = readFileSync(claim);
    unlinkSync(claim);
    const stage = join(active, `.mutation-stage-${"4".repeat(32)}`);
    writeFileSync(stage, claimBytes, { mode: 0o600 });
    linkSync(stage, claim);
    assert.equal(run(state, "status").status, 0);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(existsSync(stage), false);
    assertPrivate(claim, 0o600);
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-stage-")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a linked close stage retires without reopening its permanent generation", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const close = join(active, ".mutation-close-00");
    const stage = join(active, `.mutation-stage-${"6".repeat(32)}`);
    linkSync(close, stage);
    assert.equal(run(state, "status").status, 0);
    assertPrivate(close, 0o600, false, 2);
    appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: state.state,
    });
    assert.equal(existsSync(stage), false);
    assertPrivate(close, 0o600);
    assertPrivate(join(active, ".mutation-slot-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-01"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a publisher accepts its exact final blocker after concurrent stage reconciliation", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const child = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "publish-race",
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    await waitForMutationPublicationStage(state);
    const active = activeRun(state);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600, false, 2);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(active, "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /another clean-engine mutation is active or could not be identified/,
    );
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-stage-")), false);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    const [status, signal] = await once(child, "close");
    assert.equal(signal, null);
    assert.equal(status, 0, stderr);
    assert.equal(parse(join(active, "02-provider-create-passed.json")).phase,
      "provider-create-passed");
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a close publisher retries after a competing writer reconciles its live stage", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const child = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "close-race",
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    const firstStage = await waitForUnlinkedCloseStage(state);
    const active = activeRun(state);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(active, "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /another clean-engine mutation is active or could not be identified/,
    );
    assert.equal(existsSync(firstStage), false);
    const [status, signal] = await once(child, "close");
    assert.equal(signal, null);
    assert.equal(status, 0, stderr);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-stage-")), false);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a losing pre-link slot contender cannot block the winner's close", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const child = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "hold",
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    await waitForProviderIntent(state);
    const active = activeRun(state);
    const losingStage = join(active, `.mutation-stage-${"7".repeat(32)}`);
    writeFileSync(losingStage, canonicalBytes(mutationLease(state)), { mode: 0o600 });
    const [status, signal] = await once(child, "close");
    assert.equal(signal, null);
    assert.equal(status, 0, stderr);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assertPrivate(losingStage, 0o600);
    assert.throws(() => linkSync(losingStage, join(active, ".mutation-slot-00")), {
      code: "EEXIST",
    });
    const candidate = parse(join(active, "candidate.json"));
    appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: state.state,
    });
    assert.equal(existsSync(losingStage), false);
    assertPrivate(join(active, ".mutation-slot-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-01"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a recovery close retries after a competing writer reconciles its live stage", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    stageAbandonedProviderLease(state);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const child = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/recover-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        confirmation,
        "close-race",
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    const firstStage = await waitForUnlinkedCloseStage(
      state,
      "02-provider-create-failed.json",
    );
    const active = activeRun(state);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(active, "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /mutation recovery is active or abandoned/,
    );
    assert.equal(existsSync(firstStage), false);
    const [status, signal] = await once(child, "close");
    assert.equal(signal, null);
    assert.equal(status, 0, stderr);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-stage-")), false);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("mutation stages with any foreign hard link are retained and refused", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const stage = join(activeRun(state), `.mutation-stage-${"5".repeat(32)}`);
    writeFileSync(stage, "{", { mode: 0o600 });
    linkSync(stage, join(state.root, "foreign-mutation-stage-link"));
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: pending mutation publication link was refused\n");
    assertPrivate(stage, 0o600, false, 2);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the fake provider adapter holds one slot across intent effect and result", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const candidate = parse(join(activeRun(state), "candidate.json"));
    const receipt = executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(receipt.phase, "provider-create-passed");
    const intent = parse(join(activeRun(state), "01-provider-create-intent.json"));
    assert.equal(
      intent.result.provider_contract_sha256,
      "644704a6fccc5867c9987d6a971a980086d7fe77712ca4f892ae4aef839fd799",
    );
    assert.equal(intent.result.provider_resource, `synveda-cpr45-${candidate.run_id}`);
    assertPrivate(join(activeRun(state), ".mutation-slot-00"), 0o600);
    assertPrivate(join(activeRun(state), ".mutation-close-00"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("generic receipt append cannot own provider or finalization evidence", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const candidate = parse(join(activeRun(state), "candidate.json"));
    for (const phase of [
      "preflight-refused",
      "provider-create-intent",
      "provider-create-passed",
      "provider-cleanup-intent",
      "provider-cleanup-passed",
      "finalize-passed",
    ]) {
      assert.throws(
        () => appendReceiptForExecutor({
          phase,
          repoRoot: state.repo,
          result: cleanEngineReceiptResult(
            new Set(["finalize-passed", "provider-cleanup-passed"]).has(phase)
              ? "provider-create-passed"
              : phase,
            candidate.run_id,
          ),
          stateBase: state.state,
        }),
        /receipt phase requires its dedicated mutation executor/,
      );
    }
    assert.equal(
      readdirSync(activeRun(state)).some((name) => /^\.mutation-(?:slot|close)-/.test(name)),
      false,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("closed slots form one immutable predecessor chain without name reuse", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const staleSlot = join(state.root, "stale-slot-00");
    writeFileSync(staleSlot, canonicalBytes(mutationLease(state)), { mode: 0o600 });
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(() => linkSync(staleSlot, join(active, ".mutation-slot-00")), { code: "EEXIST" });
    const candidate = parse(join(active, "candidate.json"));
    appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: state.state,
    });
    const close0 = readFileSync(join(active, ".mutation-close-00"));
    const slot1 = parse(join(active, ".mutation-slot-01"));
    const close0Value = JSON.parse(close0);
    assert.equal(slot1.journal_sequence, 1);
    assert.equal(slot1.previous_close_sha256, sha256(close0));
    assert.equal(slot1.source_head_sha256, close0Value.result_head_sha256);
    assert.equal(slot1.source_environment_sha256, close0Value.result_environment_sha256);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assertPrivate(join(active, ".mutation-slot-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-01"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("mutation slot capacity refuses wraparound and retains all closed generations", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const result = cleanEngineReceiptResult("registry-intent", candidate.run_id);
    for (let sequence = 1; sequence < 64; sequence += 1) {
      const receipt = appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result,
        stateBase: state.state,
      });
      assert.equal(receipt.phase, "registry-intent");
    }
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result,
        stateBase: state.state,
      }),
      /mutation slot journal capacity was exhausted/,
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assertPrivate(join(active, ".mutation-slot-63"), 0o600);
    assertPrivate(join(active, ".mutation-close-63"), 0o600);
    assert.equal(existsSync(join(active, ".mutation-slot-64")), false);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a competing writer cannot enter while the fake provider effect lease is held", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const candidate = parse(join(activeRun(state), "candidate.json"));
    const helper = resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs");
    const child = spawn(
      process.execPath,
      [helper, state.repo, state.state, "kill"],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let stderr = "";
    child.stdout.resume();
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    await waitForProviderIntent(state);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /another clean-engine mutation is active or could not be identified/,
    );
    process.kill(child.pid, "SIGKILL");
    const [, signal] = await once(child, "close");
    assert.equal(signal, "SIGKILL", stderr);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assertPrivate(join(activeRun(state), ".mutation-slot-00"), 0o600);
    assertPrivate(join(activeRun(state), ".mutation-close-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a killed fake provider owner is recovered with the exact slot confirmation", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const helper = resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs");
    const child = spawn(
      process.execPath,
      [helper, state.repo, state.state, "kill"],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "ignore", "ignore"],
      },
    );
    await waitForProviderIntent(state);
    process.kill(child.pid, "SIGKILL");
    const [, signal] = await once(child, "close");
    assert.equal(signal, "SIGKILL");
    const active = activeRun(state);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assert.equal(parse(join(active, ".mutation-slot-00")).action, "provider-create");
    assert.equal(parse(join(active, "01-provider-create-intent.json")).phase,
      "provider-create-intent");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.match(confirmation, /^recover:[0-9a-f]{32}:[0-9]{2}:[0-9a-f]{64}$/);
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation: `${confirmation}0`,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider recovery confirmation was refused/,
    );
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-passed");
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("pre-intent recovery closes an absent-owner provider slot without creating a receipt", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderLease(state, {
      publishIntent: false,
    });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "plan");
    assert.deepEqual(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/.test(name)),
      ["00-plan.json"],
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a recovery claim without its permanent slot is refused", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, leaseBytes } = stageAbandonedProviderLease(state, {
      publishIntent: false,
    });
    const candidate = parse(join(active, "candidate.json"));
    const planReceipt = parse(join(active, "00-plan.json"));
    const leaseSha256 = sha256(leaseBytes);
    const recovery = {
      action: "provider-create",
      chain_root_sha256: sha256(canonicalBytes({
        action: "provider-create",
        fixture_id: candidate.run_id,
        lease_sha256: leaseSha256,
        schema: "synveda.clean-engine.mutation-recovery-root.v1",
      })),
      fixture_id: candidate.run_id,
      lease_sha256: leaseSha256,
      nonce: "d".repeat(32),
      owner_boot_sha256: "c".repeat(64),
      owner_instance_sha256: "d".repeat(64),
      owner_pid: 2_147_483_646,
      owner_probe: "opaque-process-instance-v1",
      parent_sha256: "0".repeat(64),
      schema: "synveda.clean-engine.mutation-recovery.v1",
      sequence: 0,
      slot_sequence: 0,
      source_head_sha256: sha256(canonicalBytes(planReceipt)),
    };
    writeFileSync(join(active, ".mutation-recovery-00-00"), canonicalBytes(recovery), {
      mode: 0o600,
    });
    unlinkSync(join(active, ".mutation-slot-00"));
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: mutation recovery slot was refused\n");
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("provider recovery rebinds the fixed fake contract recorded by the intent", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderMutation(state, {
      providerContractSha256: "3".repeat(64),
    });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider recovery intent binding was refused/,
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-01"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("provider slots bind both their source head and intended receipt", () => {
  for (const field of ["source_head_sha256", "intent_receipt_sha256"]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const { active } = stageAbandonedProviderLease(state);
      const leasePath = join(active, ".mutation-slot-00");
      const lease = parse(leasePath);
      lease[field] = "f".repeat(64);
      writeFileSync(leasePath, canonicalBytes(lease), { mode: 0o600 });
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.equal(refused.stderr, "clean-engine: mutation slot receipt binding was refused\n");
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("a same-PID mismatched owner challenge is unidentifiable and cannot be recovered", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderLease(state, {
      ownerPid: process.pid,
      publishIntent: false,
    });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider mutation owner is active or could not be identified/,
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-recovery-")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("recovery refuses the exact live provider owner without publishing a claim", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    assert.throws(
      () => executeProviderCreateForExecutor({
        adapter: { ...fakeProviderAdapter(), contract_sha256: "2".repeat(64) },
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /fake provider adapter fields were refused/,
    );
    assert.equal(existsSync(join(activeRun(state), ".mutation-slot-00")), false);
    assert.throws(
      () => executeProviderCreateForExecutor({
        adapter: fakeProviderAdapter({
          executeResult: {},
        }),
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider result fields were refused/,
    );
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider mutation owner is active or could not be identified/,
    );
    const active = activeRun(state);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assert.equal(readdirSync(active).some((name) => name.startsWith(".mutation-recovery-")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("provider recovery converts a passed fake effect plus source drift into cleanup-only failure", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    stageAbandonedProviderMutation(state);
    writeFileSync(join(state.repo, "source.txt"), "fake provider source drift\n", { mode: 0o600 });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assert.deepEqual(recovered.result, {
      cleanup_required: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "evidence-refused",
    });
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n", { mode: 0o600 });
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("recovery rechecks source after a durable provider-passed crash boundary", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, candidate, intentReceipt, planReceipt } =
      stageAbandonedProviderLease(state);
    const passed = createNextReceipt(
      [planReceipt, intentReceipt],
      candidate.run_id,
      "provider-create-passed",
      cleanEngineReceiptResult("provider-create-passed", candidate.run_id),
    );
    writeFileSync(join(active, receiptFileName(passed)), canonicalBytes(passed), { mode: 0o600 });
    writeFileSync(join(state.repo, "source.txt"), "drift after provider passed\n", { mode: 0o600 });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "execution-failed");
    assert.equal(recovered.sequence, 3);
    assert.equal(recovered.result.safe_code, "evidence-refused");
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n", { mode: 0o600 });
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("uncertain provider recovery retains a claim and a later recovery supersedes it", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const executor = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "kill",
      ],
      { env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" }, stdio: "ignore" },
    );
    await waitForProviderIntent(state);
    process.kill(executor.pid, "SIGKILL");
    const [, executorSignal] = await once(executor, "close");
    assert.equal(executorSignal, "SIGKILL");
    const active = activeRun(state);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recoverer = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/recover-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        confirmation,
        "unknown",
      ],
      { env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" }, stdio: ["ignore", "ignore", "pipe"] },
    );
    let recoveryError = "";
    recoverer.stderr.setEncoding("utf8");
    recoverer.stderr.on("data", (chunk) => { recoveryError += chunk; });
    const [recoveryStatus, recoverySignal] = await once(recoverer, "close");
    assert.equal(recoverySignal, null);
    assert.equal(recoveryStatus, 73);
    assert.equal(recoveryError, "provider effect remained uncertain\n");
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(active, "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /recovery is active or abandoned/,
    );
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("provider recovery capacity is bounded without replacing an existing claim", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, recovery } = stageAbandonedProviderMutation(state);
    let previous = recovery;
    for (let sequence = 1; sequence < 8; sequence += 1) {
      const claim = {
        ...previous,
        nonce: sequence.toString(16).repeat(32),
        parent_sha256: sha256(canonicalBytes(previous)),
        sequence,
      };
      writeFileSync(
        join(active, `.mutation-recovery-00-${String(sequence).padStart(2, "0")}`),
        canonicalBytes(claim),
        { mode: 0o600 },
      );
      previous = claim;
    }
    assert.equal(run(state, "status").status, 0);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider mutation recovery capacity was exhausted/,
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-00-08")), false);
    assertPrivate(join(active, ".mutation-recovery-00-07"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a provider close binds the newest permanent recovery claim", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderMutation(state);
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    const newestClaim = readFileSync(join(active, ".mutation-recovery-00-01"));
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(newestClaim));
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a recovery claim published after an owner close remains inert history", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const claim = recoveryClaimForSlot(state);
    writeFileSync(join(active, ".mutation-recovery-00-00"), canonicalBytes(claim), {
      mode: 0o600,
    });
    assert.equal(run(state, "status").status, 0);
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(close.authority, "owner");
    const candidate = parse(join(active, "candidate.json"));
    appendReceiptForExecutor({
      phase: "registry-intent",
      repoRoot: state.repo,
      result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
      stateBase: state.state,
    });
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-slot-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-01"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("late recovery contenders cannot replace a closed recovery generation", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, recovery } = stageAbandonedProviderMutation(state);
    const staleClaim = recoveryClaimForSlot(state, { previous: recovery, sequence: 1 });
    const staleStage = join(state.root, "stale-recovery-00-01");
    writeFileSync(staleStage, canonicalBytes(staleClaim), { mode: 0o600 });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.throws(
      () => linkSync(staleStage, join(active, ".mutation-recovery-00-01")),
      { code: "EEXIST" },
    );
    const claim1 = parse(join(active, ".mutation-recovery-00-01"));
    const claim2 = recoveryClaimForSlot(state, { previous: claim1, sequence: 2 });
    writeFileSync(join(active, ".mutation-recovery-00-02"), canonicalBytes(claim2), {
      mode: 0o600,
    });
    assert.equal(run(state, "status").status, 0);
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(
      close.authority_sha256,
      sha256(readFileSync(join(active, ".mutation-recovery-00-01"))),
    );
    appendReceiptForExecutor({
      phase: "failure-cleanup-intent",
      repoRoot: state.repo,
      result: {
        authorized_resources: ["provider"],
        scope: "exact-receipt-owned-only",
      },
      stateBase: state.state,
    });
    assertPrivate(join(active, ".mutation-recovery-00-02"), 0o600);
    assertPrivate(join(active, ".mutation-slot-01"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a live recovery claim fences recoverers writers and finalization", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const executor = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "kill",
      ],
      { env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" }, stdio: "ignore" },
    );
    await waitForProviderIntent(state);
    process.kill(executor.pid, "SIGKILL");
    const [, executorSignal] = await once(executor, "close");
    assert.equal(executorSignal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recoverer = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/recover-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        confirmation,
        "hold-failed",
      ],
      { env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" }, stdio: "ignore" },
    );
    await waitForRecoveryClaim(state, 0, 0);
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /another provider recovery is active or could not be identified/,
    );
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(
          "registry-intent",
          parse(join(activeRun(state), "candidate.json")).run_id,
        ),
        stateBase: state.state,
      }),
      /mutation recovery is active or abandoned/,
    );
    assert.throws(
      () => finalizeEnvironmentForExecutor({ repoRoot: state.repo, stateBase: state.state }),
      /mutation recovery is active or abandoned/,
    );
    assert.equal(existsSync(join(activeRun(state), ".mutation-recovery-00-01")), false);
    process.kill(recoverer.pid, "SIGKILL");
    const [, recovererSignal] = await once(recoverer, "close");
    assert.equal(recovererSignal, "SIGKILL");
    const recovered = recoverProviderCreateForExecutor({
      adapter: fakeProviderAdapter({
        reconcileOutcome: "failed",
        reconcileResult: {
          cleanup_required: true,
          collision_resource: "none",
          resource_disposition: "receipt-owned-or-absent",
          safe_code: "child-failed",
        },
      }),
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "provider-create-failed");
    const active = activeRun(state);
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-01"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("two simultaneous provider recoverers publish one terminal branch", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const executor = spawn(
      process.execPath,
      [
        resolve("scripts/fixtures/execute-clean-engine-fake-provider.mjs"),
        state.repo,
        state.state,
        "kill",
      ],
      { env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" }, stdio: "ignore" },
    );
    await waitForProviderIntent(state);
    process.kill(executor.pid, "SIGKILL");
    const [, executorSignal] = await once(executor, "close");
    assert.equal(executorSignal, "SIGKILL");
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const helper = resolve("scripts/fixtures/recover-clean-engine-fake-provider.mjs");
    const launch = () => {
      const child = spawn(
        process.execPath,
        [helper, state.repo, state.state, confirmation, "failed"],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["ignore", "ignore", "pipe"],
        },
      );
      let stderr = "";
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => { stderr += chunk; });
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ signal, status, stderr }));
      });
    };
    const results = await Promise.all([launch(), launch()]);
    assert.deepEqual(results.map(({ status }) => status).sort((a, b) => a - b), [0, 73]);
    assert.equal(results.every(({ signal }) => signal === null), true);
    const refused = results.find(({ status }) => status === 73);
    assert.match(
      refused.stderr,
      /^(?:another provider recovery (?:is active or could not be identified|won the mutation claim)|no abandoned provider mutation was available)\n$/,
    );
    const active = activeRun(state);
    assert.equal(parse(join(active, "02-provider-create-failed.json")).phase,
      "provider-create-failed");
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assertPrivate(join(active, ".mutation-close-00"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the state API appends the complete synthetic success path before finalization", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const candidate = parse(join(active, "candidate.json"));
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    for (const phase of receiptSuccessPath.slice(3, -1)) {
      const append = phase.startsWith("provider-cleanup-")
        ? appendProviderCleanupReceiptForExecutor
        : appendReceiptForExecutor;
      const receipt = append({
        phase,
        repoRoot: state.repo,
        result: cleanEngineReceiptResult(phase, candidate.run_id),
        stateBase: state.state,
      });
      assert.equal(receipt.phase, phase);
    }
    const finalized = finalizeEnvironmentForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(finalized.receipt.phase, "finalize-passed");
    assert.equal(run(state, "verify").status, 0);
    assert.equal(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/.test(name)).length,
      receiptSuccessPath.length,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("competing finalization and failure append leave one valid branch", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageSuccessfulReceipts(state);
    const helper = resolve("scripts/fixtures/race-clean-engine-finalization.mjs");
    const launch = (action) => {
      const child = spawn(process.execPath, [helper, action, state.repo, state.state], {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      });
      child.stdout.resume();
      child.stderr.resume();
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ signal, status }));
      });
    };
    const results = await Promise.all([launch("finalize"), launch("fail")]);
    assert.deepEqual(results.map((value) => value.status).sort((a, b) => a - b), [0, 73]);
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(run(state, "status").status, 0);
    const slots = readdirSync(active).filter((name) => /^\.mutation-slot-[0-9]{2}$/.test(name));
    const closes = readdirSync(active).filter((name) => /^\.mutation-close-[0-9]{2}$/.test(name));
    assert.equal(slots.length, closes.length);
    assert.ok(slots.length >= 14);
    const finalized = existsSync(join(active, "15-finalize-passed.json"));
    const failed = existsSync(join(active, "15-execution-failed.json"));
    assert.notEqual(finalized, failed);
    assert.equal(existsSync(join(active, "environment.json")), finalized);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("state-owned finalization publishes and verifies the exact eligible environment", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, candidate } = stageSuccessfulReceipts(state);
    assert.match(run(state, "status").stdout, /prepared at provider-cleanup-passed\n$/);
    const finalized = finalizeEnvironmentForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(finalized.receipt.phase, "finalize-passed");
    assert.equal(finalized.manifest.schema, "synveda.clean-engine.environment.v1");
    assertPrivate(join(active, "environment.json"), 0o600);
    assertPrivate(join(active, "15-finalize-passed.json"), 0o600);
    assert.equal(run(state, "verify").status, 0);
    assert.deepEqual(
      finalizeEnvironmentForExecutor({ repoRoot: state.repo, stateBase: state.state }),
      finalized,
    );
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /environment finalization is already in progress/,
    );

    const environmentPath = join(active, "environment.json");
    const environment = parse(environmentPath);
    environment.unreviewed = true;
    const ordered = Object.fromEntries(
      Object.entries(environment).sort(([left], [right]) => left.localeCompare(right)),
    );
    writeFileSync(environmentPath, `${JSON.stringify(ordered)}\n`, { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: environment manifest content was refused\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("environment publication outside a finalization slot is retained and refused", () => {
  for (const crashPoint of ["partial-staging", "complete-staging", "published-link"]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const { active, candidate, receipts } = stageSuccessfulReceipts(state);
      const manifest = createFinalization(
        candidate,
        canonicalBytes(candidate),
        receipts,
      ).manifestBytes;
      const staging = join(active, ".environment-publish");
      const destination = join(active, "environment.json");
      if (crashPoint === "partial-staging") {
        writeFileSync(staging, "{", { mode: 0o600 });
      } else {
        writeFileSync(staging, manifest, { mode: 0o600 });
        if (crashPoint === "published-link") linkSync(staging, destination);
      }
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /outside a finalization slot|did not cover the environment/);
      assertPrivate(staging, 0o600, false, crashPoint === "published-link" ? 2 : 1);
      if (crashPoint === "published-link") assertPrivate(destination, 0o600, false, 2);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("final receipt publication outside a finalization slot is retained and refused", () => {
  for (const crashPoint of ["partial-staging", "complete-staging", "published-link"]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const { active, candidate, receipts } = stageSuccessfulReceipts(state);
      const finalization = createFinalization(candidate, canonicalBytes(candidate), receipts);
      writeFileSync(join(active, "environment.json"), finalization.manifestBytes, { mode: 0o600 });
      const staging = join(active, ".receipt-publish");
      const destination = join(active, "15-finalize-passed.json");
      if (crashPoint === "partial-staging") {
        writeFileSync(staging, "{", { mode: 0o600 });
      } else {
        writeFileSync(staging, canonicalBytes(finalization.receipt), { mode: 0o600 });
        if (crashPoint === "published-link") linkSync(staging, destination);
      }
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(
        refused.stderr,
        /outside (?:an open mutation slot|the mutation journal)|closed mutation journal did not cover (?:the receipt head|the environment)/,
      );
      assertPrivate(staging, 0o600, false, crashPoint === "published-link" ? 2 : 1);
      if (crashPoint === "published-link") assertPrivate(destination, 0o600, false, 2);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("a complete final receipt stage without an open slot is refused", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active, candidate, receipts } = stageSuccessfulReceipts(state);
    const finalization = createFinalization(candidate, canonicalBytes(candidate), receipts);
    const staging = join(active, ".receipt-publish");
    writeFileSync(staging, canonicalBytes(finalization.receipt), { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(
      refused.stderr,
      "clean-engine: pending receipt publication was outside an open mutation slot\n",
    );
    assertPrivate(staging, 0o600);
    assert.equal(existsSync(join(active, "15-finalize-passed.json")), false);
    assert.equal(existsSync(join(active, "environment.json")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("finalization refuses inert staging and leaves no manifest on source drift", () => {
  const inertState = fixture();
  try {
    assert.equal(run(inertState, "plan").status, 0);
    stageSuccessfulReceipts(inertState);
    mkdirSync(join(inertState.state, `.pending-${"f".repeat(32)}`), { mode: 0o700 });
    assert.equal(run(inertState, "status").status, 0);
    assert.throws(
      () => finalizeEnvironmentForExecutor({
        repoRoot: inertState.repo,
        stateBase: inertState.state,
      }),
      /environment finalization requires absent inert staging/,
    );
    assert.equal(existsSync(join(activeRun(inertState), "environment.json")), false);
  } finally {
    rmSync(inertState.root, { recursive: true, force: true });
  }

  const driftState = fixture();
  try {
    assert.equal(run(driftState, "plan").status, 0);
    const { active } = stageSuccessfulReceipts(driftState);
    writeFileSync(join(driftState.repo, "source.txt"), "source drift\n");
    assert.throws(
      () => finalizeEnvironmentForExecutor({
        repoRoot: driftState.repo,
        stateBase: driftState.state,
      }),
      /source worktree is not clean/,
    );
    assert.equal(existsSync(join(active, "environment.json")), false);
    assert.equal(existsSync(join(active, "15-finalize-passed.json")), false);
    assert.equal(run(driftState, "status").status, 0);

    writeFileSync(join(driftState.repo, "source.txt"), "fixture source\n");
    const recovered = finalizeEnvironmentForExecutor({
      repoRoot: driftState.repo,
      stateBase: driftState.state,
    });
    assert.equal(recovered.receipt.phase, "finalize-passed");
  } finally {
    rmSync(driftState.root, { recursive: true, force: true });
  }
});

test("a finalized environment is mandatory, private and not replaceable", () => {
  for (const mutation of ["missing", "mode", "symlink"]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const { active } = stageSuccessfulReceipts(state);
      finalizeEnvironmentForExecutor({ repoRoot: state.repo, stateBase: state.state });
      const environmentPath = join(active, "environment.json");
      if (mutation === "missing") {
        rmSync(environmentPath);
      } else if (mutation === "mode") {
        chmodSync(environmentPath, 0o644);
      } else {
        const substitute = join(state.root, "environment-substitute.json");
        writeFileSync(substitute, "{}\n", { mode: 0o600 });
        rmSync(environmentPath);
        symlinkSync(substitute, environmentPath);
      }
      const refused = run(state, "status");
      assert.notEqual(refused.status, 0);
      assert.match(refused.stderr, /^clean-engine: /);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("receipt schema v1 is an explicit pre-provider hard-cut refusal", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const path = join(activeRun(state), "00-plan.json");
    const legacy = parse(path);
    legacy.schema = "synveda.clean-engine.receipt.v1";
    const ordered = Object.fromEntries(
      Object.entries(legacy).sort(([left], [right]) => left.localeCompare(right)),
    );
    writeFileSync(path, `${JSON.stringify(ordered)}\n`, { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: plan receipt contract was refused\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("dirty tracked, untracked and ignored build-context inputs leave no active receipt", () => {
  const mutations = [
    (state) => writeFileSync(join(state.repo, "source.txt"), "dirty tracked\n"),
    (state) => writeFileSync(join(state.repo, "untracked.txt"), "dirty untracked\n"),
    (state) => {
      git(state.repo, ["update-index", "--assume-unchanged", "source.txt"]);
      writeFileSync(join(state.repo, "source.txt"), "hidden tracked drift\n");
    },
    (state) => {
      git(state.repo, ["update-index", "--skip-worktree", "source.txt"]);
      writeFileSync(join(state.repo, "source.txt"), "hidden skip-worktree drift\n");
    },
    (state) => {
      writeFileSync(join(state.repo, ".gitignore"), ".codex/\ntarget/\nnode_modules/\n.idea/\n");
      git(state.repo, ["add", ".gitignore"]);
      git(state.repo, [
        "-c",
        "user.name=Synveda Test",
        "-c",
        "user.email=synveda-test@example.invalid",
        "commit",
        "-q",
        "-m",
        "ignore mutation",
      ]);
      mkdirSync(join(state.repo, ".idea"), { mode: 0o700 });
      writeFileSync(join(state.repo, ".idea", "leak"), "ignored context leak\n");
    },
    (state) => {
      const ignore = join(state.repo, ".gitignore");
      writeFileSync(ignore, `${readFileSync(ignore, "utf8")}dist/\n`);
      git(state.repo, ["add", ".gitignore"]);
      git(state.repo, [
        "-c",
        "user.name=Synveda Test",
        "-c",
        "user.email=synveda-test@example.invalid",
        "commit",
        "-q",
        "-m",
        "ignore generated output",
      ]);
      const generated = join(state.repo, "sdks", "typescript", "dist");
      mkdirSync(generated, { mode: 0o700, recursive: true });
      writeFileSync(join(generated, "index.js"), "generated but stale\n");
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      mutate(state);
      const result = run(state, "plan");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(
        result.stderr,
        /^clean-engine: (?:source worktree|source index|ignored source input)/,
      );
      assert.equal(lstatSync(state.state).isDirectory(), true);
      assert.throws(() => lstatSync(join(state.state, "active")), { code: "ENOENT" });
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("verification detects source drift while status remains content-free and resumable", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    writeFileSync(join(state.repo, "source.txt"), "source drift\n");
    const verify = run(state, "verify");
    assert.equal(verify.status, 78);
    assert.equal(verify.stdout, "");
    assert.equal(verify.stderr, "clean-engine: source worktree is not clean\n");
    const status = run(state, "status");
    assert.equal(status.status, 0, status.stderr);
    assert.match(status.stdout, / is prepared\n$/);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("canonical schema, receipt chain, private modes and link identity are fail-closed", () => {
  const mutations = [
    (active) => {
      const path = join(active, "candidate.json");
      const parsed = parse(path);
      parsed.unreviewed = true;
      writeFileSync(path, `${JSON.stringify(parsed)}\n`, { mode: 0o600 });
    },
    (active) => {
      const path = join(active, "candidate.json");
      writeFileSync(path, ` ${readFileSync(path, "utf8")}`, { mode: 0o600 });
    },
    (active) => chmodSync(join(active, "00-plan.json"), 0o644),
    (active) => linkSync(join(active, "candidate.json"), join(active, "candidate-hardlink.json")),
    (active) => {
      const path = join(active, "00-plan.json");
      const parsed = parse(path);
      parsed.previous_sha256 = "1".repeat(64);
      const ordered = Object.fromEntries(Object.entries(parsed).sort(([left], [right]) => left.localeCompare(right)));
      writeFileSync(path, `${JSON.stringify(ordered)}\n`, { mode: 0o600 });
    },
    (active) => writeFileSync(join(active, "unreviewed"), "unexpected\n", { mode: 0o600 }),
    (active) => chmodSync(join(active, "provider"), 0o711),
    (active) => writeFileSync(join(active, "runtime", "unexpected"), "state\n", { mode: 0o600 }),
    (active) => {
      const client = join(active, "client");
      const external = join(dirname(dirname(active)), "client-substitute");
      rmSync(client, { recursive: true, force: false });
      mkdirSync(external, { mode: 0o700 });
      writeFileSync(join(external, "proxy-template.json"), "{}\n", { mode: 0o600 });
      symlinkSync(external, client);
    },
    (active) => linkSync(join(active, "00-plan.json"), join(active, "third-plan-link.json")),
    (active) => {
      const lease = join(dirname(active), "active");
      const replacement = join(dirname(active), "replacement-active");
      copyFileSync(lease, replacement);
      chmodSync(replacement, 0o600);
      rmSync(lease);
      copyFileSync(replacement, lease);
      chmodSync(lease, 0o600);
      rmSync(replacement);
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      mutate(activeRun(state));
      const result = run(state, "status");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^clean-engine: /);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("an interrupted pre-provider staging run is inert and does not gain authority", async () => {
  const state = fixture();
  const bin = join(state.root, "bin");
  const entered = join(state.root, "git-entered");
  const realGit = command("/bin/sh", ["-c", "command -v git"]).stdout.trim();
  mkdirSync(bin, { mode: 0o700 });
  const fakeGit = join(bin, "git");
  writeFileSync(
    fakeGit,
    `#!/bin/sh\nset -eu\ncase " $* " in *" status "*) : > ${JSON.stringify(entered)}; while :; do /bin/sleep 1; done ;; esac\nexec ${JSON.stringify(realGit)} "$@"\n`,
    { mode: 0o700 },
  );
  chmodSync(fakeGit, 0o700);
  try {
    const child = spawn(process.execPath, toolArgs(state, "plan"), {
      detached: true,
      env: { PATH: `${bin}:${process.env.PATH}`, LANG: "C", LC_ALL: "C" },
      stdio: ["ignore", "pipe", "pipe"],
    });
    child.stdout.resume();
    child.stderr.resume();
    const deadline = Date.now() + 8_000;
    while (!existsSync(entered)) {
      assert.ok(Date.now() < deadline, "timed out waiting for staged source inventory");
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    process.kill(-child.pid, "SIGKILL");
    const [, signal] = await once(child, "close");
    assert.equal(signal, "SIGKILL");
    const entries = readdirSync(state.state);
    assert.equal(entries.length, 1);
    assert.match(entries[0], /^\.pending-[0-9a-f]{32}$/);
    assert.equal(existsSync(join(state.state, "active")), false);

    const resumed = run(state, "plan");
    assert.equal(resumed.status, 0, resumed.stderr);
    assert.equal(run(state, "verify").status, 0);
    const finalEntries = readdirSync(state.state).sort();
    assert.equal(finalEntries.includes("active"), true);
    assert.equal(finalEntries.filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length, 1);
    assert.equal(finalEntries.filter((entry) => /^\.pending-[0-9a-f]{32}$/.test(entry)).length, 1);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a completed run interrupted before active-link publication remains inert", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const orphan = activeRun(state);
    rmSync(join(state.state, "active"));
    assert.equal(lstatSync(join(orphan, "00-plan.json")).nlink, 1);

    const resumed = run(state, "plan");
    assert.equal(resumed.status, 0, resumed.stderr);
    assert.equal(run(state, "verify").status, 0);
    assert.equal(
      readdirSync(state.state).filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length,
      2,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("planning refuses inert staging with foreign leaves or external-mutation state", () => {
  const mutations = [
    (pending) => writeFileSync(join(pending, "foreign"), "foreign\n", { mode: 0o600 }),
    (pending) => {
      mkdirSync(join(pending, "provider"), { mode: 0o700 });
      writeFileSync(join(pending, "provider", "resource"), "resource\n", { mode: 0o600 });
    },
  ];
  for (const mutate of mutations) {
    const state = fixture();
    const pending = join(state.state, `.pending-${"1".repeat(32)}`);
    try {
      mkdirSync(pending, { mode: 0o700 });
      mutate(pending);
      const result = run(state, "plan");
      assert.equal(result.status, 78);
      assert.equal(result.stdout, "");
      assert.match(result.stderr, /^clean-engine: pending /);
      assert.equal(existsSync(pending), true);
    } finally {
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("retained inert staging is bounded", () => {
  const state = fixture();
  try {
    for (let index = 0; index < 9; index += 1) {
      mkdirSync(join(state.state, `.pending-${index.toString(16).padStart(32, "0")}`), {
        mode: 0o700,
      });
    }
    const result = run(state, "plan");
    assert.equal(result.status, 73);
    assert.equal(result.stdout, "");
    assert.equal(result.stderr, "clean-engine: inert staging limit was exceeded\n");
    assert.equal(existsSync(join(state.state, "active")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("two simultaneous planners publish one no-replace active receipt", async () => {
  const state = fixture();
  try {
    const launch = () => {
      const child = spawn(process.execPath, toolArgs(state, "plan"), {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      });
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8");
      child.stderr.setEncoding("utf8");
      child.stdout.on("data", (chunk) => { stdout += chunk; });
      child.stderr.on("data", (chunk) => { stderr += chunk; });
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ status, signal, stdout, stderr }));
      });
    };
    const results = await Promise.all([launch(), launch()]);
    assert.deepEqual(results.map((result) => result.status).sort((a, b) => a - b), [0, 73]);
    assert.equal(results.every((result) => result.signal === null), true);
    assert.equal(results.filter((result) => result.status === 0)[0].stderr, "");
    assert.match(
      results.filter((result) => result.status === 73)[0].stderr,
      /active clean-engine plan already exists/,
    );
    assert.equal(run(state, "verify").status, 0);
    const entries = readdirSync(state.state).sort();
    assert.equal(entries.length, 2);
    assert.equal(entries.includes("active"), true);
    assert.equal(entries.filter((entry) => /^\.run-[0-9a-f]{32}$/.test(entry)).length, 1);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("actual Docker-context modes and empty directories are part of source closure", () => {
  const modeState = fixture();
  try {
    assert.equal(run(modeState, "plan").status, 0);
    chmodSync(join(modeState.repo, "source.txt"), 0o644);
    assert.equal(git(modeState.repo, ["status", "--porcelain"]), "");
    const result = run(modeState, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source closure changed\n");
  } finally {
    rmSync(modeState.root, { recursive: true, force: true });
  }

  const emptyState = fixture();
  try {
    assert.equal(run(emptyState, "plan").status, 0);
    mkdirSync(join(emptyState.repo, "included-empty"), { mode: 0o700 });
    assert.equal(git(emptyState.repo, ["status", "--porcelain"]), "");
    const result = run(emptyState, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source worktree/context is not clean\n");
  } finally {
    rmSync(emptyState.root, { recursive: true, force: true });
  }

  const ignoredState = fixture();
  try {
    assert.equal(run(ignoredState, "plan").status, 0);
    mkdirSync(join(ignoredState.repo, ".codex", "empty"), {
      mode: 0o700,
      recursive: true,
    });
    mkdirSync(join(ignoredState.repo, "target", "empty"), {
      mode: 0o700,
      recursive: true,
    });
    writeFileSync(join(ignoredState.repo, ".claude", "RESUME.md"), "changed local state\n");
    writeFileSync(
      join(ignoredState.repo, "evals", "fixtures", "longmemeval", "longmemeval_oracle.json"),
      "[]\n",
    );
    assert.equal(run(ignoredState, "verify").status, 0);
  } finally {
    rmSync(ignoredState.root, { recursive: true, force: true });
  }
});

test("only the exact .env.example basename is re-included in the Docker context", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    chmodSync(join(state.repo, ".env.secret.example"), 0o644);
    assert.equal(git(state.repo, ["status", "--porcelain"]), "");
    assert.equal(run(state, "verify").status, 0);
    chmodSync(join(state.repo, ".env.example"), 0o644);
    const result = run(state, "verify");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: source closure changed\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("tracked symlinks are accepted and Dockerignore order is exact", () => {
  const symlinkState = fixture();
  try {
    symlinkSync("source.txt", join(symlinkState.repo, "source-link"));
    git(symlinkState.repo, ["add", "source-link"]);
    git(symlinkState.repo, [
      "-c",
      "user.name=Synveda Test",
      "-c",
      "user.email=synveda-test@example.invalid",
      "commit",
      "-q",
      "-m",
      "symlink fixture",
    ]);
    assert.equal(run(symlinkState, "plan").status, 0);
    assert.equal(run(symlinkState, "verify").status, 0);
  } finally {
    rmSync(symlinkState.root, { recursive: true, force: true });
  }

  const orderState = fixture();
  try {
    const ignorePath = join(orderState.repo, ".dockerignore");
    const reordered = readFileSync(ignorePath, "utf8").replace(
      "**/.env.*\n!**/.env.example",
      "!**/.env.example\n**/.env.*",
    );
    writeFileSync(ignorePath, reordered, { mode: 0o600 });
    git(orderState.repo, ["add", ".dockerignore"]);
    git(orderState.repo, [
      "-c",
      "user.name=Synveda Test",
      "-c",
      "user.email=synveda-test@example.invalid",
      "commit",
      "-q",
      "-m",
      "reordered ignore",
    ]);
    const result = run(orderState, "plan");
    assert.equal(result.status, 78);
    assert.equal(result.stderr, "clean-engine: Docker ignore contract was refused\n");
  } finally {
    rmSync(orderState.root, { recursive: true, force: true });
  }
});

test("read-only actions never create a missing state root", () => {
  const state = fixture();
  try {
    rmSync(state.state, { recursive: true, force: false });
    const result = run(state, "status");
    assert.equal(result.status, 69);
    assert.equal(result.stderr, "clean-engine: state base was unavailable\n");
    assert.equal(existsSync(state.state), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("unsafe pools, providers, repository-local state and unknown arguments are refused", () => {
  const state = fixture();
  try {
    const base = [
      stateTool,
      "plan",
      "--repo-root",
      state.repo,
      "--state-base",
      state.state,
      "--ipv4-pool",
    ];
    for (const pool of ["172.15.1.0/24", "10.1.2.1/24", "10.01.2.0/24", "public.invalid/24"]) {
      const result = command(process.execPath, [...base, pool, "--provider", "colima"]);
      assert.equal(result.status, 64);
      assert.equal(result.stderr, "clean-engine: IPv4 pool must be a canonical private /24\n");
    }
    const provider = command(process.execPath, [...base, "10.1.2.0/24", "--provider", "desktop"]);
    assert.equal(provider.status, 64);
    assert.equal(provider.stderr, "clean-engine: provider must be colima\n");
    const local = command(process.execPath, [
      stateTool,
      "plan",
      "--repo-root",
      state.repo,
      "--state-base",
      join(state.repo, "state"),
      "--ipv4-pool",
      "10.1.2.0/24",
      "--provider",
      "colima",
    ]);
    assert.equal(local.status, 78);
    assert.equal(local.stderr, "clean-engine: state base must be outside the repository\n");
    const unknown = run(state, "status", ["--unreviewed", "value"]);
    assert.equal(unknown.status, 64);
    assert.equal(unknown.stderr, "clean-engine: invalid arguments\n");
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

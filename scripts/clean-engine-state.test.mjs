#!/usr/bin/env node
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  createHmac,
  createPrivateKey,
  createPublicKey,
  sign,
  verify,
} from "node:crypto";
import { once } from "node:events";
import {
  chmodSync,
  closeSync,
  constants,
  copyFileSync,
  existsSync,
  fsyncSync,
  lstatSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readlinkSync,
  realpathSync,
  readdirSync,
  renameSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import test from "node:test";
import * as cleanEngineStateModule from "../deploy/compose/scripts/clean-engine-state.mjs";
import {
  appendProviderCleanupReceiptForExecutor,
  appendReceiptForExecutor,
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
  COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
  executeProviderCreateForExecutor,
  finalizeEnvironmentForExecutor,
  liveProviderFixtureEffectRecoveryConfirmationForTest,
  liveProviderFixtureReservationRecoveryConfirmationForTest,
  liveProviderFixtureStartDecisionRecoveryConfirmationForTest,
  liveProviderIntentRecoveryConfirmationForExecutor,
  liveProviderPlanRecoveryConfirmationForExecutor,
  liveProviderStartDecisionRecoveryConfirmationForExecutor,
  observeColimaLivePreEffectAdmissionForExecutor,
  observeColimaLivePreEffectAdmissionForTest,
  observeColimaLiveProviderStartFreshAdmissionForExecutor,
  observeColimaLiveProviderStartFreshAdmissionForTest,
  observeColimaLiveProviderStartEffectFreshAdmissionForExecutor,
  observeColimaLiveProviderStartEffectFreshAdmissionForTest,
  projectLiveProviderPlanCompletionForExecutor,
  providerRecoveryConfirmationForExecutor,
  publishColimaLiveProviderIntentForExecutor,
  publishColimaLiveProviderIntentForTest,
  publishColimaLiveProviderEffectAttemptFenceForTest,
  publishColimaLiveProviderEffectConclusiveNotCreatedForTest,
  publishColimaLiveProviderEffectPreAttemptForTest,
  publishColimaLiveProviderReservationForTest,
  publishColimaLiveProviderStartDecisionForExecutor,
  publishColimaLiveProviderStartDecisionForTest,
  recordLiveProviderOperationPlanForTest,
  recoverLiveProviderIntentForExecutor,
  recoverLiveProviderPlanForExecutor,
  recoverLiveProviderStartDecisionForExecutor,
  recoverLiveProviderStartDecisionForTest,
  recoverColimaLiveProviderReservationForTest,
  recoverColimaLiveProviderEffectForTest,
  recoverProviderCreateForExecutor,
} from "../deploy/compose/scripts/clean-engine-state.mjs";
import {
  buildColimaLiveObservationForTest,
  colimaLiveBytes,
  colimaLiveDigest,
} from "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";
import {
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
  buildColimaLiveEffectIntentCandidateStructure,
  buildColimaLiveProviderIntentPublicationPlan,
} from "../deploy/compose/scripts/clean-engine-live-provider-intent.mjs";
import {
  COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_DECISION_CANDIDATE_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_FRESH_ADMISSION_SCHEMA,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
  liveProviderStartDecisionBytes,
} from "../deploy/compose/scripts/clean-engine-live-provider-start-decision.mjs";
import {
  COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  liveProviderProcessStartBytes,
} from "../deploy/compose/scripts/clean-engine-live-provider-process-start.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS,
  liveProviderEffectBytes,
  liveProviderEffectDigest,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect.mjs";
import {
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS,
  buildColimaLiveProviderEffectFixtureBlueprintStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
} from "../deploy/compose/scripts/clean-engine-live-provider-reservation.mjs";
import {
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
} from "../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import {
  canonicalBytes,
  createFinalization,
  createNextReceipt,
  receiptFileName,
  receiptSuccessPath,
  sha256,
} from "../deploy/compose/scripts/clean-engine-receipts.mjs";
import { cleanEngineReceiptResult } from "./fixtures/clean-engine-receipt-fixture.mjs";
import { cleanEngineLiveProviderOperationPlan } from "./fixtures/clean-engine-live-provider-plan-fixture.mjs";
import {
  cloneColimaLiveObservationInput,
  createCleanEngineColimaLiveObservationFixture,
  writePrivateColimaLiveFixtureFile,
} from "./fixtures/clean-engine-colima-live-observation-fixture.mjs";

const stateTool = resolve("deploy/compose/scripts/clean-engine-state.mjs");
const FIXTURE_EFFECT_ADAPTER_CONTRACT_SHA256 =
  "f97fef2c614db9e656a0cb9ed7ad79a5ce4eae314103670c57c988f8c707c6f2";
const FIXTURE_EFFECT_ADAPTER_RESULT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-result.v1";

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

function fsyncDirectoryForTest(path) {
  const descriptor = openSync(
    path,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function snapshotTree(root) {
  const snapshot = [];
  function visit(path, relativePath) {
    const metadata = lstatSync(path, { bigint: true });
    const entry = {
      ctimeNs: String(metadata.ctimeNs),
      dev: String(metadata.dev),
      gid: String(metadata.gid),
      ino: String(metadata.ino),
      mode: String(metadata.mode),
      mtimeNs: String(metadata.mtimeNs),
      nlink: String(metadata.nlink),
      path: relativePath,
      size: String(metadata.size),
      uid: String(metadata.uid),
    };
    if (metadata.isDirectory()) {
      entry.kind = "directory";
      snapshot.push(entry);
      for (const name of readdirSync(path).sort()) {
        visit(join(path, name), join(relativePath, name));
      }
    } else if (metadata.isFile()) {
      entry.kind = "file";
      entry.sha256 = sha256(readFileSync(path));
      snapshot.push(entry);
    } else if (metadata.isSymbolicLink()) {
      entry.kind = "symbolic-link";
      entry.target = readlinkSync(path);
      snapshot.push(entry);
    } else {
      assert.fail(`unsupported fixture entry at ${relativePath}`);
    }
  }
  visit(root, ".");
  return snapshot;
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function activeRun(state) {
  const receipt = parse(join(state.state, "active"));
  return join(state.state, `.run-${receipt.fixture_id}`);
}

function liveProviderOperationPlan(state, observationSha256 = "e".repeat(64)) {
  const active = activeRun(state);
  const candidate = parse(join(active, "candidate.json"));
  const planReceipt = parse(join(active, "00-plan.json"));
  return cleanEngineLiveProviderOperationPlan({
    candidateSha256: sha256(canonicalBytes(candidate)),
    fixtureId: candidate.run_id,
    observationSha256,
    sourceHeadSha256: sha256(canonicalBytes(planReceipt)),
  });
}

function prepareLiveProviderIntentFixture(state) {
  const planned = run(state, "plan");
  assert.equal(planned.status, 0, planned.stderr);
  const active = activeRun(state);
  const fixtureId = parse(join(active, "candidate.json")).run_id;
  const preparation = createCleanEngineColimaLiveObservationFixture({
    fixtureId,
  });
  const operationPlan = liveProviderOperationPlan(
    state,
    colimaLiveDigest(colimaLiveBytes(preparation.observation)),
  );
  recordLiveProviderOperationPlanForTest({
    operationPlan,
    repoRoot: state.repo,
    stateBase: state.state,
  });
  return { active, fixtureId, operationPlan, preparation };
}

function fixtureIntentArguments(state, preparation, testCheckpoint = () => {}) {
  return {
    observation: preparation.observation,
    observationInput: preparation.input,
    repoRoot: state.repo,
    requirements: preparation.requirements,
    stateBase: state.state,
    testCheckpoint,
  };
}

function prepareCompletedLiveProviderIntentFixture(state) {
  const prepared = prepareLiveProviderIntentFixture(state);
  const intentCompletion = publishColimaLiveProviderIntentForTest(
    fixtureIntentArguments(state, prepared.preparation),
  );
  return { ...prepared, intentCompletion };
}

function prepareCompletedLiveProviderStartDecisionFixture(state) {
  const prepared = prepareCompletedLiveProviderIntentFixture(state);
  const startDecisionCompletion =
    publishColimaLiveProviderStartDecisionForTest(
      fixtureStartAdmissionArguments(state, prepared.preparation),
    );
  return { ...prepared, startDecisionCompletion };
}

function fixtureStartAdmissionArguments(
  state,
  preparation,
  testCheckpoint = () => {},
) {
  return {
    observation: preparation.observation,
    observationInput: preparation.input,
    repoRoot: state.repo,
    requirements: preparation.requirements,
    stateBase: state.state,
    testCheckpoint,
  };
}

function fixtureEffectPrivateHmac(bindingKey, fixtureId, purpose, value) {
  return createHmac("sha256", bindingKey)
    .update(
      "synveda.clean-engine.colima-live-fixture-provider-effect-private.v1\0",
      "utf8",
    )
    .update(fixtureId, "ascii")
    .update("\0", "ascii")
    .update(purpose, "ascii")
    .update("\0", "ascii")
    .update(liveProviderEffectBytes(value))
    .digest("hex");
}

function fixtureEffectAdapterObservation(input, result) {
  return sha256(
    canonicalBytes({
      adapter_contract_sha256: input.adapter_contract_sha256,
      domain: FIXTURE_EFFECT_ADAPTER_RESULT_SCHEMA,
      fixture_id: input.fixture_id,
      operation_contract_sha256: input.operation_contract_sha256,
      operation_kind: input.operation_kind,
      planned_start_attempt_sha256: input.planned_start_attempt_sha256,
      result,
      schema: FIXTURE_EFFECT_ADAPTER_RESULT_SCHEMA,
    }),
  );
}

function fixtureEffectRoleKey(bindingKey, fixtureId, role) {
  const seed = Buffer.from(
    fixtureEffectPrivateHmac(
      bindingKey,
      fixtureId,
      `${role}-role-key-seed`,
      { role },
    ),
    "hex",
  );
  try {
    const privateKey = createPrivateKey({
      format: "der",
      key: Buffer.concat([
        Buffer.from("302e020100300506032b657004220420", "hex"),
        seed,
      ]),
      type: "pkcs8",
    });
    return {
      privateKey,
      publicKey: createPublicKey(privateKey),
    };
  } finally {
    seed.fill(0);
  }
}

function assertNoStartDecisionArtifacts(active) {
  assert.equal(
    readdirSync(active).some((name) => /start-decision/u.test(name)),
    false,
  );
  assert.equal(existsSync(join(active, "environment.json")), false);
  assert.deepEqual(
    readdirSync(active)
      .filter((name) => /^\.mutation-slot-/u.test(name))
      .sort(),
    [".mutation-slot-00", ".mutation-slot-01"],
  );
  assert.deepEqual(
    readdirSync(active)
      .filter((name) => /^\.mutation-close-/u.test(name))
      .sort(),
    [".mutation-close-00", ".mutation-close-01"],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^\.mutation-operation-/u.test(name)),
    [],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^\.mutation-stage-/u.test(name)),
    [],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^\.pending/u.test(name)),
    [],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
    ["00-plan.json"],
  );
  for (const directory of ["evidence", "provider", "registry", "runtime"]) {
    assert.deepEqual(readdirSync(join(active, directory)), []);
  }
}

function assertInertStartDecisionJournal(active, terminalSequence) {
  const expected = Array.from(
    { length: terminalSequence + 1 },
    (_, sequence) => String(sequence).padStart(2, "0"),
  );
  assert.deepEqual(
    readdirSync(active)
      .filter((name) => /^\.mutation-slot-/u.test(name))
      .sort(),
    expected.map((sequence) => `.mutation-slot-${sequence}`),
  );
  assert.deepEqual(
    readdirSync(active)
      .filter((name) => /^\.mutation-close-/u.test(name))
      .sort(),
    expected.map((sequence) => `.mutation-close-${sequence}`),
  );
  assert.equal(existsSync(join(active, "environment.json")), false);
  assert.deepEqual(
    readdirSync(active).filter((name) => /^\.mutation-operation-/u.test(name)),
    [],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^\.mutation-stage-/u.test(name)),
    [],
  );
  assert.deepEqual(
    readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
    ["00-plan.json"],
  );
  for (const directory of ["evidence", "provider", "registry", "runtime"]) {
    assert.deepEqual(readdirSync(join(active, directory)), []);
  }
}

function productionIntentPublicationPlanForFixture(
  state,
  preparation,
  operationPlan,
) {
  const fixtureAdmission = observeColimaLivePreEffectAdmissionForTest({
    observation: preparation.observation,
    observationInput: preparation.input,
    repoRoot: state.repo,
    requirements: preparation.requirements,
    stateBase: state.state,
    testCheckpoint() {},
  });
  const admission = JSON.parse(JSON.stringify(fixtureAdmission));
  admission.authority = "point-in-time-not-effect-authority";
  admission.schema = COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA;
  admission.root_observation.evidence_class = "production-pinned";
  admission.root_observation.requirements_sha256 =
    operationPlan.requirements_sha256;
  admission.root_observation.schema =
    COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA;
  return buildColimaLiveProviderIntentPublicationPlan({
    admission,
    fixtureOnly: false,
    operationPlan,
  });
}

function stageProductionLiveProviderIntent(
  state,
  publicationPlan,
  disposition,
) {
  const active = activeRun(state);
  const previousCloseBytes = readFileSync(join(active, ".mutation-close-00"));
  const slot = {
    ...mutationLease(state, {
      action: "provider-intent",
      journalSequence: 1,
      previousCloseSha256: sha256(previousCloseBytes),
    }),
    operation_contract_sha256:
      COLIMA_LIVE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
    operation_kind: COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
    operation_plan: publicationPlan,
  };
  const slotBytes = canonicalBytes(slot);
  writeFileSync(join(active, ".mutation-slot-01"), slotBytes, { mode: 0o600 });
  const close = {
    authority: "owner",
    authority_sha256: sha256(slotBytes),
    disposition,
    fixture_id: slot.fixture_id,
    operation_evidence_sha256: "0".repeat(64),
    operation_contract_sha256: slot.operation_contract_sha256,
    operation_kind: slot.operation_kind,
    operation_plan_sha256: sha256(canonicalBytes(publicationPlan)),
    result_environment_sha256: slot.source_environment_sha256,
    result_head_sha256: slot.source_head_sha256,
    result_sequence: slot.source_sequence,
    schema: "synveda.clean-engine.mutation-close.v8",
    slot_sequence: slot.journal_sequence,
    slot_sha256: sha256(slotBytes),
  };
  writeFileSync(join(active, ".mutation-close-01"), canonicalBytes(close), {
    mode: 0o600,
  });
}

function stageAbortedProductionLiveProviderIntent(state, publicationPlan) {
  stageProductionLiveProviderIntent(
    state,
    publicationPlan,
    "aborted-before-effect",
  );
}

function stageCompletedProductionLiveProviderIntent(state, publicationPlan) {
  stageProductionLiveProviderIntent(state, publicationPlan, "completed");
}

function replaceCompletedFixtureIntentGeneration(state) {
  const active = activeRun(state);
  const firstSlot = parse(join(active, ".mutation-slot-01"));
  const firstClosePath = join(active, ".mutation-close-01");
  const firstCloseBytes = readFileSync(firstClosePath);
  const firstClose = JSON.parse(firstCloseBytes.toString("utf8"));
  firstClose.disposition = "aborted-before-effect";
  const abortedCloseBytes = canonicalBytes(firstClose);
  writeFileSync(firstClosePath, abortedCloseBytes, { mode: 0o600 });

  const secondSlot = {
    ...mutationLease(state, {
      action: "provider-intent",
      journalSequence: 2,
      previousCloseSha256: sha256(abortedCloseBytes),
    }),
    operation_contract_sha256:
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
    operation_kind: COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
    operation_plan: firstSlot.operation_plan,
  };
  const secondSlotBytes = canonicalBytes(secondSlot);
  const secondSlotPath = join(active, ".mutation-slot-02");
  writeFileSync(secondSlotPath, secondSlotBytes, { mode: 0o600 });
  const secondClose = {
    authority: "owner",
    authority_sha256: sha256(secondSlotBytes),
    disposition: "completed",
    fixture_id: secondSlot.fixture_id,
    operation_contract_sha256: secondSlot.operation_contract_sha256,
    operation_evidence_sha256: "0".repeat(64),
    operation_kind: secondSlot.operation_kind,
    operation_plan_sha256: sha256(canonicalBytes(secondSlot.operation_plan)),
    result_environment_sha256: secondSlot.source_environment_sha256,
    result_head_sha256: secondSlot.source_head_sha256,
    result_sequence: secondSlot.source_sequence,
    schema: "synveda.clean-engine.mutation-close.v8",
    slot_sequence: secondSlot.journal_sequence,
    slot_sha256: sha256(secondSlotBytes),
  };
  const secondClosePath = join(active, ".mutation-close-02");
  writeFileSync(secondClosePath, canonicalBytes(secondClose), { mode: 0o600 });
  return () => {
    unlinkSync(secondClosePath);
    unlinkSync(secondSlotPath);
    writeFileSync(firstClosePath, firstCloseBytes, { mode: 0o600 });
  };
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
  const providerCreate = action === "provider-create";
  return {
    action,
    fixture_id: candidate.run_id,
    intent_receipt_sha256: intentReceiptSha256,
    journal_sequence: journalSequence,
    nonce: "f".repeat(32),
    operation_contract_sha256: providerCreate
      ? "81159eaee9bc06e651a529008b9531b873b700a1fbf88852d58f018cd8c8d39e"
      : "0".repeat(64),
    operation_kind: providerCreate ? "deterministic-fake-provider-create-v1" : "none",
    operation_plan: null,
    owner_boot_sha256: "a".repeat(64),
    owner_instance_sha256: "b".repeat(64),
    owner_pid: ownerPid,
    owner_probe: "opaque-process-instance-v1",
    previous_close_sha256: previousCloseSha256,
    schema: "synveda.clean-engine.mutation-slot.v7",
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
    "81159eaee9bc06e651a529008b9531b873b700a1fbf88852d58f018cd8c8d39e",
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
  const { active, candidate, intentReceipt, lease, leaseBytes } = staged;
  const recovery = {
    action: "provider-create",
    chain_root_sha256: sha256(canonicalBytes({
      action: "provider-create",
      fixture_id: candidate.run_id,
      lease_sha256: sha256(leaseBytes),
      operation_contract_sha256: lease.operation_contract_sha256,
      operation_kind: lease.operation_kind,
      operation_plan_sha256: "0".repeat(64),
      schema: "synveda.clean-engine.mutation-recovery-root.v6",
    })),
    fixture_id: candidate.run_id,
    lease_sha256: sha256(leaseBytes),
    nonce: "e".repeat(32),
    observed_effect_disposition: "not-reached",
    observed_effect_name: "none",
    observed_evidence_head_sha256: "0".repeat(64),
    observed_evidence_prefix_sha256: "0".repeat(64),
    observed_evidence_stage: "none",
    observed_residual_sha256: "0".repeat(64),
    observed_settlement_sha256: "0".repeat(64),
    operation_contract_sha256: lease.operation_contract_sha256,
    operation_kind: lease.operation_kind,
    operation_plan_sha256: "0".repeat(64),
    owner_boot_sha256: "c".repeat(64),
    owner_instance_sha256: "d".repeat(64),
    owner_pid: 2_147_483_646,
    owner_probe: "opaque-process-instance-v1",
    parent_sha256: "0".repeat(64),
    schema: "synveda.clean-engine.mutation-recovery.v6",
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
  const slot = JSON.parse(slotBytes.toString("utf8"));
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
      operation_contract_sha256: slot.operation_contract_sha256,
      operation_kind: slot.operation_kind,
      operation_plan_sha256: slot.operation_plan === null
        ? "0".repeat(64)
        : sha256(canonicalBytes(slot.operation_plan)),
      schema: "synveda.clean-engine.mutation-recovery-root.v6",
    })),
    fixture_id: candidate.run_id,
    lease_sha256: sha256(slotBytes),
    nonce: ((sequence + 1) % 16).toString(16).repeat(32),
    observed_effect_disposition: "not-reached",
    observed_effect_name: "none",
    observed_evidence_head_sha256: "0".repeat(64),
    observed_evidence_prefix_sha256: "0".repeat(64),
    observed_evidence_stage: "none",
    observed_residual_sha256: "0".repeat(64),
    observed_settlement_sha256: "0".repeat(64),
    operation_contract_sha256: slot.operation_contract_sha256,
    operation_kind: slot.operation_kind,
    operation_plan_sha256: slot.operation_plan === null
      ? "0".repeat(64)
      : sha256(canonicalBytes(slot.operation_plan)),
    owner_boot_sha256: "8".repeat(64),
    owner_instance_sha256: "9".repeat(64),
    owner_pid: ownerPid,
    owner_probe: "opaque-process-instance-v1",
    parent_sha256: previous === undefined ? "0".repeat(64) : sha256(canonicalBytes(previous)),
    schema: "synveda.clean-engine.mutation-recovery.v6",
    sequence,
    slot_sequence: slotSequence,
    source_head_sha256: sourceHeadSha256 ?? sha256(canonicalBytes(head)),
  };
}

async function waitForProviderIntent(state, timeoutMilliseconds = 8_000) {
  const path = join(activeRun(state), "01-provider-create-intent.json");
  const deadline = Date.now() + timeoutMilliseconds;
  while (
    !existsSync(path) ||
    lstatSync(path, { bigint: true }).nlink !== 1n
  ) {
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

async function waitForPublishedMutationSlot(
  state,
  expectedAction,
  timeoutMilliseconds = 8_000,
) {
  const active = activeRun(state);
  const path = join(active, ".mutation-slot-00");
  const deadline = Date.now() + timeoutMilliseconds;
  while (true) {
    try {
      const metadata = lstatSync(path, { bigint: true });
      if (
        metadata.isFile() &&
        metadata.nlink === 1n &&
        metadata.size >= 2n &&
        !readdirSync(active).some((name) => name.startsWith(".mutation-stage-"))
      ) {
        const value = parse(path);
        assert.equal(value.schema, "synveda.clean-engine.mutation-slot.v7");
        assert.equal(value.action, expectedAction);
        return;
      }
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    assert.ok(Date.now() < deadline, "timed out waiting for published mutation slot");
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

async function crashLiveProviderReservationAt(
  state,
  { action, checkpoint, input, releaseName },
) {
  const helper = resolve(
    "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
  );
  const releasePath = join(state.root, releaseName);
  const child = spawn(
    process.execPath,
    [
      helper,
      action,
      state.repo,
      state.state,
      checkpoint,
      releasePath,
    ],
    {
      env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  let inputWriteError;
  child.stdin.on("error", (error) => {
    inputWriteError = error;
  });
  child.stdin.end(JSON.stringify(input));
  child.stdout.resume();
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  try {
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
  }
}

async function crashLiveProviderEffectAt(
  state,
  {
    action = "publish",
    checkpoint,
    input,
    releaseName,
    signal: terminationSignal = "SIGKILL",
  },
) {
  const helper = resolve(
    "scripts/fixtures/clean-engine-live-provider-effect-process.mjs",
  );
  const releasePath = join(state.root, releaseName);
  const child = spawn(
    process.execPath,
    [
      helper,
      action,
      state.repo,
      state.state,
      checkpoint,
      releasePath,
    ],
    {
      env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  let inputWriteError;
  child.stdin.on("error", (error) => {
    inputWriteError = error;
  });
  child.stdin.end(JSON.stringify(input));
  child.stdout.resume();
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  try {
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 90_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill(terminationSignal);
    const [status, observedSignal] = await once(child, "close");
    assert.equal(status, null);
    assert.equal(observedSignal, terminationSignal);
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill(terminationSignal);
    }
  }
}

async function releaseLiveProviderEffectCheckpoint(
  state,
  { action = "conclusive", checkpoint, input, mutate, releaseName },
) {
  const helper = resolve(
    "scripts/fixtures/clean-engine-live-provider-effect-process.mjs",
  );
  const releasePath = join(state.root, releaseName);
  const child = spawn(
    process.execPath,
    [helper, action, state.repo, state.state, checkpoint, releasePath],
    {
      env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  let inputWriteError;
  child.stdin.on("error", (error) => {
    inputWriteError = error;
  });
  child.stdin.end(JSON.stringify(input));
  child.stdout.resume();
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  try {
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 90_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    mutate();
    writeFileSync(releasePath, "release\n", { flag: "wx", mode: 0o600 });
    const [status, signal] = await once(child, "close");
    return { signal, status, stderr };
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
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

test("a live provider operation plan is journaled once and grants no effect authority", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
    const recorded = recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.deepEqual(recorded.operationPlan, operationPlan);

    const active = activeRun(state);
    const slotPath = join(active, ".mutation-slot-00");
    const closePath = join(active, ".mutation-close-00");
    const slot = parse(slotPath);
    const close = parse(closePath);
    assertPrivate(slotPath, 0o600);
    assertPrivate(closePath, 0o600);
    assert.equal(slot.schema, "synveda.clean-engine.mutation-slot.v7");
    assert.equal(slot.action, "provider-plan");
    assert.deepEqual(slot.operation_plan, operationPlan);
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v8");
    assert.equal(close.disposition, "completed");
    assert.equal(close.operation_plan_sha256, sha256(canonicalBytes(operationPlan)));
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.deepEqual(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
      ["00-plan.json"],
    );
    for (const directory of ["evidence", "provider", "registry", "runtime"]) {
      assert.deepEqual(readdirSync(join(active, directory)), []);
    }
    assert.equal(existsSync(join(active, "environment.json")), false);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);

    const beforeProjection = snapshotTree(state.root);
    const slotBytes = readFileSync(slotPath);
    const closeBytes = readFileSync(closePath);
    const projected = projectLiveProviderPlanCompletionForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(projected.plan_slot_sha256, sha256(slotBytes));
    assert.equal(projected.plan_close_sha256, sha256(closeBytes));
    assert.equal(
      projected.operation_plan_sha256,
      sha256(canonicalBytes(operationPlan)),
    );
    assert.equal(
      projected.preparation_observation_sha256,
      operationPlan.preparation_observation_sha256,
    );
    const structuralIntent = buildColimaLiveEffectIntentCandidateStructure({
      completionProjection: projected,
      operationPlan,
    });
    assert.equal(structuralIntent.effect_authorization, "requested-not-authorized");
    assert.deepEqual(snapshotTree(state.root), beforeProjection);

    const unrelatedStage = join(active, `.mutation-stage-${"9".repeat(32)}`);
    writeFileSync(unrelatedStage, "{", { mode: 0o600 });
    const beforeRefusal = snapshotTree(state.root);
    assert.throws(
      () =>
        projectLiveProviderPlanCompletionForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /completed live provider plan was unavailable/u,
    );
    assert.deepEqual(snapshotTree(state.root), beforeRefusal);
    unlinkSync(unrelatedStage);

    const idempotent = recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.deepEqual(idempotent.operationPlan, operationPlan);
    assert.equal(readdirSync(active).filter((name) => /^\.mutation-slot-/u.test(name)).length, 1);

    for (const operation of [
      () =>
        executeProviderCreateForExecutor({
          adapter: fakeProviderAdapter(),
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      () =>
        appendReceiptForExecutor({
          phase: "registry-intent",
          repoRoot: state.repo,
          result: cleanEngineReceiptResult("registry-intent", fixtureId),
          stateBase: state.state,
        }),
      () =>
        appendProviderCleanupReceiptForExecutor({
          phase: "provider-cleanup-intent",
          repoRoot: state.repo,
          result: cleanEngineReceiptResult("provider-cleanup-intent", fixtureId),
          stateBase: state.state,
        }),
      () =>
        finalizeEnvironmentForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
    ]) {
      assert.throws(operation, /live provider execution remains disabled after state planning/u);
    }
    assert.equal(existsSync(join(active, "01-provider-create-intent.json")), false);
    assert.equal(existsSync(join(active, "environment.json")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("live pre-effect admission rejects caller provenance and publishes nothing", () => {
  assert.equal(
    COLIMA_LIVE_PRE_EFFECT_ADMISSION_SCHEMA,
    "synveda.clean-engine.colima-live-pre-effect-admission.v1",
  );
  const sensitive = "/private/never-inspect-caller-provenance";
  const base = {
    observation: {},
    observationInput: {},
    repoRoot: sensitive,
    stateBase: `${sensitive}-state`,
  };
  for (const value of [
    null,
    {},
    { ...base, completionProjection: {} },
    { ...base, intent: {} },
    { ...base, operationPlan: {} },
    { ...base, preEffectPrefix: {} },
    { ...base, providerRoot: sensitive },
    { ...base, requirements: {} },
    { ...base, testHoldMilliseconds: 1 },
    { ...base, repoRoot: null },
    { ...base, repoRoot: 1 },
    { ...base, repoRoot: {} },
    { ...base, stateBase: [] },
    { ...base, stateBase: "" },
    { ...base, observation: null },
    { ...base, observation: [] },
    { ...base, observationInput: undefined },
    { ...base, observationInput: "not-an-object" },
  ]) {
    for (const operation of [
      observeColimaLivePreEffectAdmissionForExecutor,
      publishColimaLiveProviderIntentForExecutor,
    ]) {
      assert.throws(
        () => operation(value),
        (error) => {
          assert.equal(error.exitStatus, 64);
          assert.equal(error.message.includes(sensitive), false);
          return true;
        },
      );
    }
  }

  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const before = snapshotTree(state.root);
    assert.throws(
      () =>
        observeColimaLivePreEffectAdmissionForExecutor({
          observation: {},
          observationInput: {},
          repoRoot: state.repo,
          stateBase: state.state,
      }),
      (error) => {
        assert.equal(
          error.message,
          "live provider pre-effect observation binding was refused",
        );
        assert.equal(error.exitStatus, 73);
        return true;
      },
    );
    assert.deepEqual(snapshotTree(state.root), before);
    assert.equal(
      readdirSync(activeRun(state)).some((name) =>
        /(?:admission|effect-intent)/u.test(name),
      ),
      false,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture admission composes state provenance with pristine namespaces", () => {
  const state = fixture();
  let preparation;
  try {
    assert.equal(run(state, "plan").status, 0);
    const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
    preparation = createCleanEngineColimaLiveObservationFixture({ fixtureId });
    const observationSha256 = colimaLiveDigest(
      colimaLiveBytes(preparation.observation),
    );
    const operationPlan = liveProviderOperationPlan(state, observationSha256);
    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const stateBefore = snapshotTree(state.root);
    const preparationBefore = snapshotTree(preparation.root);
    const result = observeColimaLivePreEffectAdmissionForTest({
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
      testCheckpoint() {},
    });

    assert.deepEqual(snapshotTree(state.root), stateBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
    assert.deepEqual(Object.keys(result).sort(), [
      "authority",
      "completion_projection",
      "intent_candidate",
      "pre_effect_prefix",
      "root_observation",
      "schema",
      "supervisor_process_group_label",
    ]);
    assert.equal(
      result.schema,
      COLIMA_LIVE_FIXTURE_PRE_EFFECT_ADMISSION_SCHEMA,
    );
    assert.equal(
      result.authority,
      "fixture-only-point-in-time-not-effect-authority",
    );
    assert.equal(result.root_observation.evidence_class, "fixture-only");
    assert.equal(
      result.root_observation.schema,
      COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
    );
    assert.equal(result.root_observation.root_set_disposition, "observed-pristine");
    assert.equal(
      result.completion_projection.operation_plan_sha256,
      sha256(canonicalBytes(operationPlan)),
    );
    assert.equal(
      result.completion_projection.preparation_observation_sha256,
      observationSha256,
    );
    assert.equal(result.intent_candidate.effect_name, "provider-create");
    assert.equal(
      result.intent_candidate.effect_authorization,
      "requested-not-authorized",
    );
    assert.equal(result.pre_effect_prefix.entry_count, 0);
    const expectedLabel =
      `sv-c45-colima-pg-${fixtureId}-` +
      result.completion_projection.plan_slot_sha256.slice(0, 12);
    assert.equal(result.supervisor_process_group_label, expectedLabel);
    assert.match(
      result.supervisor_process_group_label,
      /^sv-c45-colima-pg-[0-9a-f]{32}-[0-9a-f]{12}$/u,
    );
    assert.equal(
      Buffer.byteLength(result.supervisor_process_group_label, "ascii"),
      62,
    );
    assertRecursivelyFrozen(result);

    const serialized = canonicalBytes(result).toString("utf8");
    for (const forbidden of [
      preparation.root,
      preparation.home,
      preparation.providerRoot,
      "binding_key",
      "command",
      "HOME",
      "PID",
      "PGID",
      "socket",
      "docker-context",
      "guest-engine",
      "hostagent",
    ]) {
      assert.equal(serialized.includes(forbidden), false, forbidden);
    }
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture intent publication commits one inert state-only successor", () => {
  const state = fixture();
  let preparation;
  try {
    assert.equal(run(state, "plan").status, 0);
    const active = activeRun(state);
    const fixtureId = parse(join(active, "candidate.json")).run_id;
    preparation = createCleanEngineColimaLiveObservationFixture({ fixtureId });
    const operationPlan = liveProviderOperationPlan(
      state,
      colimaLiveDigest(colimaLiveBytes(preparation.observation)),
    );
    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const repoBefore = snapshotTree(state.repo);
    const preparationBefore = snapshotTree(preparation.root);
    const checkpoints = [];
    const argumentsValue = {
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
      testCheckpoint(checkpoint) {
        checkpoints.push(checkpoint);
      },
    };
    const completed = publishColimaLiveProviderIntentForTest(argumentsValue);
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
    );
    assert.equal(
      completed.authority,
      "fixture-only-durable-inert-intent-not-effect-authority",
    );
    assertRecursivelyFrozen(completed);
    assert.deepEqual(
      checkpoints,
      [
        "initial:after-first-admission-observation",
        "after-initial-admission",
        "before-slot-stage",
        "before-slot-link:after-first-admission-observation",
        "after-slot-authority-reassertion",
        "before-slot-link:after-first-admission-observation",
        "after-slot-link",
        "after-slot-acquisition:after-first-admission-observation",
        "after-slot-acquisition",
        "before-close-link:after-first-admission-observation",
        "after-close-link",
      ],
    );

    const slotPath = join(active, ".mutation-slot-01");
    const closePath = join(active, ".mutation-close-01");
    const slot = parse(slotPath);
    const close = parse(closePath);
    assertPrivate(slotPath, 0o600);
    assertPrivate(closePath, 0o600);
    assert.equal(slot.schema, "synveda.clean-engine.mutation-slot.v7");
    assert.equal(slot.action, "provider-intent");
    assert.equal(
      slot.operation_kind,
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_KIND,
    );
    assert.equal(
      slot.operation_contract_sha256,
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_OPERATION_CONTRACT_SHA256,
    );
    assert.equal(slot.intent_receipt_sha256, "0".repeat(64));
    assert.equal(
      slot.operation_plan.schema,
      "synveda.clean-engine.colima-live-fixture-provider-intent-publication-plan.v1",
    );
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v8");
    assert.equal(close.disposition, "completed");
    assert.equal(close.authority, "owner");
    assert.equal(close.result_sequence, 0);
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(existsSync(join(active, ".mutation-operation-01")), false);
    assert.deepEqual(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
      ["00-plan.json"],
    );
    for (const directory of ["evidence", "provider", "registry", "runtime"]) {
      assert.deepEqual(readdirSync(join(active, directory)), []);
    }
    assert.equal(existsSync(join(active, "environment.json")), false);
    assert.deepEqual(snapshotTree(state.repo), repoBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);

    checkpoints.length = 0;
    const repeated = publishColimaLiveProviderIntentForTest(argumentsValue);
    assert.deepEqual(repeated, completed);
    assert.deepEqual(checkpoints, []);
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\.mutation-slot-/u.test(name)),
      [".mutation-slot-00", ".mutation-slot-01"],
    );
    const beforeCrossClassRetry = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderIntentForExecutor({
          observation: preparation.observation,
          observationInput: preparation.input,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /evidence class already differs/u,
    );
    assert.deepEqual(snapshotTree(state.state), beforeCrossClassRetry);

    for (const operation of [
      () =>
        executeProviderCreateForExecutor({
          adapter: fakeProviderAdapter(),
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      () =>
        appendReceiptForExecutor({
          phase: "registry-intent",
          repoRoot: state.repo,
          result: cleanEngineReceiptResult("registry-intent", fixtureId),
          stateBase: state.state,
        }),
      () =>
        finalizeEnvironmentForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
    ]) {
      assert.throws(
        operation,
        /live provider execution remains disabled after state planning/u,
      );
    }
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("completed fixture intent derives a fresh inert process-start admission", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const stateBefore = snapshotTree(state.root);
    const preparationBefore = snapshotTree(preparation.root);
    const checkpoints = [];
    const result = observeColimaLiveProviderStartFreshAdmissionForTest(
      fixtureStartAdmissionArguments(state, preparation, (checkpoint) => {
        checkpoints.push(checkpoint);
      }),
    );

    assert.deepEqual(Object.keys(result).sort(), [
      "authority",
      "completed_intent_projection",
      "process_start_decision_candidate",
      "root_observation",
      "schema",
      "supervisor_process_group_label",
    ]);
    assert.equal(
      result.schema,
      COLIMA_LIVE_FIXTURE_PROCESS_START_FRESH_ADMISSION_SCHEMA,
    );
    assert.equal(
      result.authority,
      "fixture-only-point-in-time-not-process-start-authority",
    );
    assert.equal(
      result.completed_intent_projection.schema,
      COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_INTENT_PROJECTION_SCHEMA,
    );
    assert.equal(
      result.completed_intent_projection.provider_intent_slot_sha256,
      prepared.intentCompletion.intent_slot_sha256,
    );
    assert.equal(
      result.completed_intent_projection.provider_intent_close_sha256,
      prepared.intentCompletion.intent_close_sha256,
    );
    assert.equal(
      result.completed_intent_projection.provider_intent_publication_plan_sha256,
      prepared.intentCompletion.intent_publication_plan_sha256,
    );
    assert.equal(
      result.completed_intent_projection.completed_plan_projection_sha256,
      prepared.intentCompletion.completed_plan_projection_sha256,
    );
    assert.equal(result.root_observation.root_set_disposition, "observed-pristine");
    const candidate = result.process_start_decision_candidate;
    assert.notEqual(candidate, null);
    assert.equal(
      candidate.schema,
      COLIMA_LIVE_FIXTURE_PROCESS_START_DECISION_CANDIDATE_SCHEMA,
    );
    assert.equal(candidate.decision, "requested-not-executed-not-authorized");
    assert.equal(candidate.effect_name, "provider-process-start");
    assert.equal(candidate.process_state, "not-reached");
    assert.equal(candidate.process_start_authorized, false);
    assert.equal(candidate.cross_namespace_atomic_reservation, false);
    assert.equal(candidate.future_effect_fresh_admission_required, true);
    assert.equal(
      candidate.completed_intent_projection_sha256,
      sha256(liveProviderStartDecisionBytes(result.completed_intent_projection)),
    );
    assert.equal(
      candidate.root_observation_sha256,
      sha256(liveProviderStartDecisionBytes(result.root_observation)),
    );
    assert.deepEqual(checkpoints, [
      "after-first-process-start-admission-observation",
    ]);
    assertRecursivelyFrozen(result);
    assert.deepEqual(snapshotTree(state.root), stateBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
    assertNoStartDecisionArtifacts(prepared.active);

    const serialized = liveProviderStartDecisionBytes(result).toString("utf8");
    for (const forbidden of [
      preparation.root,
      preparation.home,
      preparation.providerRoot,
      preparation.input.binding_key.toString("hex"),
      preparation.input.binding_key.toString("base64"),
      "binding_key",
      "command",
      "credential",
      "docker-context",
      "environment.json",
      "guest-engine",
      "host-agent",
      "hostagent",
      "HOME",
      "PGID",
      "PID",
      "provider-start-decision.json",
      "socket",
    ]) {
      assert.equal(serialized.includes(forbidden), false, forbidden);
    }

    checkpoints.length = 0;
    const repeatedStateBefore = snapshotTree(state.root);
    const repeatedPreparationBefore = snapshotTree(preparation.root);
    const repeated = observeColimaLiveProviderStartFreshAdmissionForTest(
      fixtureStartAdmissionArguments(state, preparation, (checkpoint) => {
        checkpoints.push(checkpoint);
      }),
    );
    assert.deepEqual(repeated, result);
    assert.deepEqual(checkpoints, [
      "after-first-process-start-admission-observation",
    ]);
    assert.deepEqual(snapshotTree(state.root), repeatedStateBefore);
    assert.deepEqual(snapshotTree(preparation.root), repeatedPreparationBefore);

    const collisionPath = join(
      preparation.input.environment.COLIMA_HOME,
      preparation.input.provider_profile,
    );
    writePrivateColimaLiveFixtureFile(
      collisionPath,
      Buffer.from("foreign collision\n", "utf8"),
      0o600,
    );
    const collisionStateBefore = snapshotTree(state.root);
    const collisionPreparationBefore = snapshotTree(preparation.root);
    const collision = observeColimaLiveProviderStartFreshAdmissionForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.equal(collision.root_observation.root_set_disposition, "foreign-collision");
    assert.equal(collision.process_start_decision_candidate, null);
    assertRecursivelyFrozen(collision);
    assert.deepEqual(snapshotTree(state.root), collisionStateBefore);
    assert.deepEqual(snapshotTree(preparation.root), collisionPreparationBefore);
    unlinkSync(collisionPath);

    const restoredStateBefore = snapshotTree(state.root);
    const restoredPreparationBefore = snapshotTree(preparation.root);
    const restored = observeColimaLiveProviderStartFreshAdmissionForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.notEqual(restored.process_start_decision_candidate, null);
    assert.deepEqual(snapshotTree(state.root), restoredStateBefore);
    assert.deepEqual(snapshotTree(preparation.root), restoredPreparationBefore);
    assertNoStartDecisionArtifacts(prepared.active);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("process-start admission rejects caller provenance and the wrong intent class", () => {
  const state = fixture();
  const incomplete = fixture();
  const production = fixture();
  let foreignPreparation;
  let preparation;
  let incompletePreparation;
  let productionPreparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const incompletePrepared = prepareLiveProviderIntentFixture(incomplete);
    incompletePreparation = incompletePrepared.preparation;
    assert.throws(
      () =>
        observeColimaLiveProviderStartFreshAdmissionForTest(
          fixtureStartAdmissionArguments(incomplete, incompletePreparation),
        ),
      /completed live provider intent was unavailable/u,
    );

    const productionPrepared = prepareLiveProviderIntentFixture(production);
    productionPreparation = productionPrepared.preparation;
    stageCompletedProductionLiveProviderIntent(
      production,
      productionIntentPublicationPlanForFixture(
        production,
        productionPreparation,
        productionPrepared.operationPlan,
      ),
    );
    assert.throws(
      () =>
        observeColimaLiveProviderStartFreshAdmissionForTest(
          fixtureStartAdmissionArguments(production, productionPreparation),
        ),
      (error) => {
        assert.equal(
          error.message,
          "live provider intent evidence class already differs",
        );
        assert.equal(error.exitStatus, 73);
        return true;
      },
    );

    const stateBefore = snapshotTree(state.state);
    chmodSync(preparation.providerRoot, 0o000);
    try {
      const base = fixtureStartAdmissionArguments(state, preparation);
      for (const field of [
        "admission",
        "authority",
        "closeAuthority",
        "completedIntentProjection",
        "completed_intent_projection",
        "fixtureOnly",
        "intentCompletion",
        "intentPublicationPlan",
        "operationPlan",
        "processStartDecisionCandidate",
        "rootObservation",
        "source",
        "stateSnapshot",
      ]) {
        assert.throws(
          () =>
            observeColimaLiveProviderStartFreshAdmissionForTest({
              ...base,
              [field]: null,
            }),
          (error) => {
            assert.equal(error.exitStatus, 64, field);
            return true;
          },
        );
      }
      assert.throws(
        () =>
          observeColimaLiveProviderStartFreshAdmissionForExecutor({
            observation: preparation.observation,
            observationInput: preparation.input,
            repoRoot: state.repo,
            stateBase: state.state,
          }),
        (error) => {
          assert.equal(error.message, "live provider intent evidence class already differs");
          assert.equal(error.exitStatus, 73);
          return true;
        },
      );

      const executorBase = {
        observation: preparation.observation,
        observationInput: preparation.input,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      for (const malformed of [
        null,
        {},
        { ...executorBase, observation: null },
        { ...executorBase, observationInput: [] },
        { ...executorBase, repoRoot: "" },
        { ...executorBase, stateBase: 1 },
      ]) {
        assert.throws(
          () =>
            observeColimaLiveProviderStartFreshAdmissionForExecutor(malformed),
          (error) => {
            assert.equal(error.exitStatus, 64);
            return true;
          },
        );
      }
      for (const malformed of [
        { ...base, requirements: null },
        { ...base, requirements: [] },
        { ...base, testCheckpoint: null },
      ]) {
        assert.throws(
          () => observeColimaLiveProviderStartFreshAdmissionForTest(malformed),
          (error) => {
            assert.equal(error.exitStatus, 64);
            return true;
          },
        );
      }
    } finally {
      chmodSync(preparation.providerRoot, 0o700);
    }

    foreignPreparation = createCleanEngineColimaLiveObservationFixture({
      fixtureId: "f".repeat(32),
    });
    chmodSync(foreignPreparation.providerRoot, 0o000);
    assert.throws(
      () =>
        observeColimaLiveProviderStartFreshAdmissionForTest({
          observation: foreignPreparation.observation,
          observationInput: foreignPreparation.input,
          repoRoot: state.repo,
          requirements: foreignPreparation.requirements,
          stateBase: state.state,
          testCheckpoint() {},
        }),
      (error) => {
        assert.equal(
          error.message,
          "live provider pre-effect observation binding was refused",
        );
        assert.equal(error.exitStatus, 73);
        return true;
      },
    );
    chmodSync(foreignPreparation.providerRoot, 0o700);
    assert.deepEqual(snapshotTree(state.state), stateBefore);
    assertNoStartDecisionArtifacts(prepared.active);
    assertNoStartDecisionArtifacts(productionPrepared.active);
  } finally {
    if (foreignPreparation !== undefined) {
      chmodSync(foreignPreparation.providerRoot, 0o700);
      rmSync(foreignPreparation.root, { recursive: true, force: true });
    }
    if (incompletePreparation !== undefined) {
      rmSync(incompletePreparation.root, { recursive: true, force: true });
    }
    if (productionPreparation !== undefined) {
      rmSync(productionPreparation.root, { recursive: true, force: true });
    }
    if (preparation !== undefined) {
      chmodSync(preparation.providerRoot, 0o700);
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(incomplete.root, { recursive: true, force: true });
    rmSync(production.root, { recursive: true, force: true });
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("process-start admission refuses state and namespace drift between fresh samples", () => {
  const cases = [
    {
      errorMessage: "live provider start fresh observation changed",
      exitStatus: 73,
      name: "target-appeared",
      mutate({ preparation }) {
        writePrivateColimaLiveFixtureFile(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
          Buffer.from("appeared\n", "utf8"),
          0o600,
        );
      },
      restore({ preparation }) {
        unlinkSync(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
        );
      },
    },
    {
      errorMessage: "live provider start fresh observation changed",
      exitStatus: 73,
      name: "collision-disappeared",
      prepare({ preparation }) {
        writePrivateColimaLiveFixtureFile(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
          Buffer.from("collision\n", "utf8"),
          0o600,
        );
      },
      mutate({ preparation }) {
        unlinkSync(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
        );
      },
      restore({ preparation }) {
        writePrivateColimaLiveFixtureFile(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
          Buffer.from("collision\n", "utf8"),
          0o600,
        );
      },
    },
    {
      errorMessage: "source worktree is not clean",
      exitStatus: 78,
      name: "source-drifted",
      mutate({ state }) {
        writeFileSync(join(state.repo, "source.txt"), "drifted source\n");
      },
      restore({ state }) {
        writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
      },
    },
    {
      errorMessage: "mutation close was refused",
      exitStatus: 78,
      name: "intent-slot-drifted",
      mutate({ active }) {
        const path = join(active, ".mutation-slot-01");
        const slot = parse(path);
        slot.intent_receipt_sha256 = "f".repeat(64);
        writeFileSync(path, canonicalBytes(slot), { mode: 0o600 });
      },
      restore({ active, slotBytes }) {
        writeFileSync(join(active, ".mutation-slot-01"), slotBytes, {
          mode: 0o600,
        });
      },
    },
    {
      errorMessage: "mutation close receipt binding was refused",
      exitStatus: 78,
      name: "intent-close-drifted",
      mutate({ active }) {
        const path = join(active, ".mutation-close-01");
        const close = parse(path);
        close.result_head_sha256 = "f".repeat(64);
        writeFileSync(path, canonicalBytes(close), { mode: 0o600 });
      },
      restore({ active, closeBytes }) {
        writeFileSync(join(active, ".mutation-close-01"), closeBytes, {
          mode: 0o600,
        });
      },
    },
    {
      errorMessage: "completed live provider intent state changed",
      exitStatus: 73,
      name: "valid-completed-intent-generation-changed",
      mutate(context) {
        context.restoreGeneration =
          replaceCompletedFixtureIntentGeneration(context.state);
      },
      restore({ restoreGeneration }) {
        restoreGeneration();
      },
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      const context = {
        active: prepared.active,
        closeBytes: readFileSync(join(prepared.active, ".mutation-close-01")),
        preparation,
        slotBytes: readFileSync(join(prepared.active, ".mutation-slot-01")),
        state,
      };
      value.prepare?.(context);
      const stateBefore = snapshotTree(state.state);
      let checkpoints = 0;
      assert.throws(
        () =>
          observeColimaLiveProviderStartFreshAdmissionForTest(
            fixtureStartAdmissionArguments(state, preparation, (checkpoint) => {
              assert.equal(
                checkpoint,
                "after-first-process-start-admission-observation",
              );
              checkpoints += 1;
              value.mutate(context);
            }),
        ),
        (error) => {
          assert.equal(error.message, value.errorMessage, value.name);
          assert.equal(error.exitStatus, value.exitStatus, value.name);
          return true;
        },
      );
      assert.equal(checkpoints, 1, value.name);
      value.restore(context);
      if (
        !value.name.includes("intent-") &&
        !value.name.includes("generation-")
      ) {
        assert.deepEqual(snapshotTree(state.state), stateBefore, value.name);
      }
      assertNoStartDecisionArtifacts(prepared.active);
      assert.equal(run(state, "verify").status, 0, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("state-owned process-start admission has no persistence or effect surface", () => {
  const stateSource = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  const start = stateSource.indexOf(
    "function completedLiveProviderIntentSnapshot",
  );
  const end = stateSource.indexOf(
    "function buildLiveProviderStartDecisionPublicationPlan",
    start,
  );
  assert.ok(start >= 0 && end > start);
  const observerSource = stateSource.slice(start, end);
  assert.doesNotMatch(
    observerSource,
    /\b(?:acquire|append|close|execute|finalize|launch|link|publish|reconcile|recover|rename|spawn|unlink|write)[A-Za-z0-9_]*\s*\(/u,
  );
  assert.doesNotMatch(
    observerSource,
    /buildColimaLiveProviderStartDecision(?:PublicationPlan|Completion)/u,
  );
  assert.doesNotMatch(
    observerSource,
    /(?:providerAdapter|mutationOperation|receiptFileName)/u,
  );

  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /process-start-fresh-admission/u);

  const registry = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(registry, /provider-start-decision-publication/u);
});

test("fixture start decision publication commits one inert completed successor", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const active = prepared.active;
    const repoBefore = snapshotTree(state.repo);
    const preparationBefore = snapshotTree(preparation.root);
    const checkpoints = [];
    const argumentsValue = fixtureStartAdmissionArguments(
      state,
      preparation,
      (checkpoint) => {
        checkpoints.push(checkpoint);
      },
    );
    const completed = publishColimaLiveProviderStartDecisionForTest(
      argumentsValue,
    );
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
    );
    assert.equal(
      completed.authority,
      "fixture-only-durable-inert-process-start-decision-not-effect-authority",
    );
    assertRecursivelyFrozen(completed);
    assert.deepEqual(checkpoints, [
      "initial:after-first-process-start-admission-observation",
      "after-initial-admission",
      "before-slot-stage",
      "before-slot-link:after-first-process-start-admission-observation",
      "after-slot-authority-reassertion",
      "before-slot-link:after-first-process-start-admission-observation",
      "after-slot-link",
      "after-slot-acquisition:after-first-process-start-admission-observation",
      "after-slot-acquisition",
      "before-close-link:after-first-process-start-admission-observation",
      "after-close-link",
    ]);

    const slotPath = join(active, ".mutation-slot-02");
    const closePath = join(active, ".mutation-close-02");
    const slot = parse(slotPath);
    const close = parse(closePath);
    assertPrivate(slotPath, 0o600);
    assertPrivate(closePath, 0o600);
    assert.equal(slot.schema, "synveda.clean-engine.mutation-slot.v7");
    assert.equal(slot.action, "provider-start-decision");
    assert.equal(
      slot.operation_kind,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_KIND,
    );
    assert.equal(
      slot.operation_contract_sha256,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
    );
    assert.equal(
      slot.operation_plan.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_PUBLICATION_PLAN_SCHEMA,
    );
    assert.equal(
      slot.operation_plan.state_integration,
      "mutation-journal-v7-inert-start-decision-only",
    );
    assert.equal(slot.intent_receipt_sha256, "0".repeat(64));
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v8");
    assert.equal(close.disposition, "completed");
    assert.equal(close.authority, "owner");
    assert.equal(close.result_sequence, 0);
    assert.equal(close.result_head_sha256, slot.source_head_sha256);
    assert.equal(close.result_environment_sha256, "0".repeat(64));
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(close.operation_plan_sha256, sha256(canonicalBytes(slot.operation_plan)));
    assertInertStartDecisionJournal(active, 2);
    assert.deepEqual(snapshotTree(state.repo), repoBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);

    checkpoints.length = 0;
    const repeated = publishColimaLiveProviderStartDecisionForTest(
      argumentsValue,
    );
    assert.deepEqual(repeated, completed);
    assert.deepEqual(checkpoints, []);
    assertInertStartDecisionJournal(active, 2);

    rmSync(preparation.root, { recursive: true, force: true });
    preparation = undefined;
    checkpoints.length = 0;
    const repeatedWithoutObservableRoots =
      publishColimaLiveProviderStartDecisionForTest(argumentsValue);
    assert.deepEqual(repeatedWithoutObservableRoots, completed);
    assert.deepEqual(checkpoints, []);
    assertInertStartDecisionJournal(active, 2);

    const beforeHistoricalRefusals = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForTest({
          ...argumentsValue,
          observation: {
            ...argumentsValue.observation,
            historical_retry_probe: "changed",
          },
        }),
      /completed live provider start decision retry differed/u,
    );
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForTest({
          ...argumentsValue,
          requirements: {
            ...argumentsValue.requirements,
            historical_retry_probe: "changed",
          },
        }),
      /completed live provider start decision retry differed/u,
    );
    assert.deepEqual(snapshotTree(state.state), beforeHistoricalRefusals);
    assert.deepEqual(checkpoints, []);

    const beforeCrossClass = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForExecutor({
          observation: argumentsValue.observation,
          observationInput: argumentsValue.observationInput,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /evidence class already differs/u,
    );
    assert.deepEqual(snapshotTree(state.state), beforeCrossClass);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("start decision refuses a pre-publication evidence-class crossing", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const before = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForExecutor({
          observation: preparation.observation,
          observationInput: preparation.input,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /live provider intent evidence class already differs/u,
    );
    assert.deepEqual(snapshotTree(state.state), before);
    assertNoStartDecisionArtifacts(prepared.active);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("durable start decision state rejects rehashed authority and completion drift", () => {
  const cases = [
    {
      expectedStatus: 69,
      name: "authorizing decision candidate",
      mutate({ close, slot, writeBound }) {
        slot.operation_plan.admission.process_start_decision_candidate
          .process_start_authorized = true;
        slot.operation_plan.decision_candidate_sha256 = sha256(
          liveProviderStartDecisionBytes(
            slot.operation_plan.admission.process_start_decision_candidate,
          ),
        );
        slot.operation_plan.admission_sha256 = sha256(
          liveProviderStartDecisionBytes(slot.operation_plan.admission),
        );
        writeBound(slot, close);
      },
    },
    {
      expectedStatus: 69,
      name: "completed intent projection",
      mutate({ close, slot, writeBound }) {
        const projection =
          slot.operation_plan.admission.completed_intent_projection;
        projection.provider_intent_slot_sha256 = "f".repeat(64);
        slot.operation_plan.admission.process_start_decision_candidate
          .completed_intent_projection_sha256 = sha256(
            liveProviderStartDecisionBytes(projection),
          );
        slot.operation_plan.completed_intent_projection_sha256 = sha256(
          liveProviderStartDecisionBytes(projection),
        );
        slot.operation_plan.decision_candidate_sha256 = sha256(
          liveProviderStartDecisionBytes(
            slot.operation_plan.admission.process_start_decision_candidate,
          ),
        );
        slot.operation_plan.admission_sha256 = sha256(
          liveProviderStartDecisionBytes(slot.operation_plan.admission),
        );
        writeBound(slot, close);
      },
    },
    {
      expectedStatus: 78,
      name: "nonzero operation evidence",
      mutate({ close, closePath }) {
        close.operation_evidence_sha256 = "f".repeat(64);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
    },
    {
      expectedStatus: 78,
      name: "second start decision after completed decision",
      mutate({ active, close, slot }) {
        const successor = {
          ...slot,
          journal_sequence: 3,
          nonce: "e".repeat(32),
          previous_close_sha256: sha256(canonicalBytes(close)),
        };
        writeFileSync(
          join(active, ".mutation-slot-03"),
          canonicalBytes(successor),
          { mode: 0o600 },
        );
      },
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      publishColimaLiveProviderStartDecisionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      const active = prepared.active;
      const slotPath = join(active, ".mutation-slot-02");
      const closePath = join(active, ".mutation-close-02");
      const slotBytes = readFileSync(slotPath);
      const closeBytes = readFileSync(closePath);
      const slot = JSON.parse(slotBytes.toString("utf8"));
      const close = JSON.parse(closeBytes.toString("utf8"));
      const writeBound = (changedSlot, changedClose) => {
        const changedSlotBytes = canonicalBytes(changedSlot);
        writeFileSync(slotPath, changedSlotBytes, { mode: 0o600 });
        writeFileSync(
          closePath,
          canonicalBytes({
            ...changedClose,
            authority_sha256: sha256(changedSlotBytes),
            operation_plan_sha256: sha256(
              canonicalBytes(changedSlot.operation_plan),
            ),
            slot_sha256: sha256(changedSlotBytes),
          }),
          { mode: 0o600 },
        );
      };
      value.mutate({ active, close, closePath, slot, writeBound });
      const refused = run(state, "status");
      assert.equal(refused.status, value.expectedStatus, value.name);
      if (existsSync(join(active, ".mutation-slot-03"))) {
        unlinkSync(join(active, ".mutation-slot-03"));
      }
      writeFileSync(slotPath, slotBytes, { mode: 0o600 });
      writeFileSync(closePath, closeBytes, { mode: 0o600 });
      assert.equal(run(state, "verify").status, 0, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("decision journal refuses a direct plan successor and class switching", () => {
  {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      publishColimaLiveProviderStartDecisionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      const active = prepared.active;
      const decisionSlot = parse(join(active, ".mutation-slot-02"));
      const decisionClose = parse(join(active, ".mutation-close-02"));
      unlinkSync(join(active, ".mutation-close-02"));
      unlinkSync(join(active, ".mutation-slot-02"));
      unlinkSync(join(active, ".mutation-close-01"));
      unlinkSync(join(active, ".mutation-slot-01"));
      const directSlot = {
        ...decisionSlot,
        journal_sequence: 1,
        previous_close_sha256: sha256(
          readFileSync(join(active, ".mutation-close-00")),
        ),
      };
      const directSlotBytes = canonicalBytes(directSlot);
      const directClose = {
        ...decisionClose,
        authority_sha256: sha256(directSlotBytes),
        slot_sequence: 1,
        slot_sha256: sha256(directSlotBytes),
      };
      writeFileSync(join(active, ".mutation-slot-01"), directSlotBytes, {
        mode: 0o600,
      });
      writeFileSync(
        join(active, ".mutation-close-01"),
        canonicalBytes(directClose),
        { mode: 0o600 },
      );
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(
        refused.stderr,
        /live provider start decision lacked a completed intent/u,
      );
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }

  {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      publishColimaLiveProviderStartDecisionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      const active = prepared.active;
      const decisionSlot = parse(join(active, ".mutation-slot-02"));
      const decisionClosePath = join(active, ".mutation-close-02");
      const decisionClose = parse(decisionClosePath);
      decisionClose.disposition = "aborted-before-effect";
      const decisionCloseBytes = canonicalBytes(decisionClose);
      writeFileSync(decisionClosePath, decisionCloseBytes, { mode: 0o600 });
      const productionPlan = structuredClone(decisionSlot.operation_plan);
      productionPlan.operation_contract_sha256 =
        COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256;
      productionPlan.operation_kind =
        COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND;
      const crossedSlot = {
        ...decisionSlot,
        journal_sequence: 3,
        nonce: "e".repeat(32),
        operation_contract_sha256:
          COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_CONTRACT_SHA256,
        operation_kind: COLIMA_LIVE_PROVIDER_START_DECISION_OPERATION_KIND,
        operation_plan: productionPlan,
        previous_close_sha256: sha256(decisionCloseBytes),
      };
      writeFileSync(
        join(active, ".mutation-slot-03"),
        canonicalBytes(crossedSlot),
        { mode: 0o600 },
      );
      const refused = run(state, "status");
      assert.equal(refused.status, 78);
      assert.match(
        refused.stderr,
        /live provider start decision evidence classes were mixed/u,
      );
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("start decision publication refuses a current namespace collision without a slot", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const collisionPath = join(
      preparation.input.environment.COLIMA_HOME,
      preparation.input.provider_profile,
    );
    writePrivateColimaLiveFixtureFile(
      collisionPath,
      Buffer.from("foreign collision\n", "utf8"),
      0o600,
    );
    const before = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForTest(
          fixtureStartAdmissionArguments(state, preparation),
        ),
      /start decision namespaces were not pristine/u,
    );
    assert.deepEqual(snapshotTree(state.state), before);
    assertNoStartDecisionArtifacts(prepared.active);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("namespace drift after start decision acquisition aborts and a fresh retry wins", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const collisionPath = join(
      preparation.input.environment.COLIMA_HOME,
      preparation.input.provider_profile,
    );
    let changed = false;
    assert.throws(
      () =>
        publishColimaLiveProviderStartDecisionForTest(
          fixtureStartAdmissionArguments(state, preparation, (checkpoint) => {
            if (checkpoint !== "after-slot-acquisition" || changed) return;
            changed = true;
            writePrivateColimaLiveFixtureFile(
              collisionPath,
              Buffer.from("appeared after acquisition\n", "utf8"),
              0o600,
            );
          }),
        ),
      /start decision namespaces were not pristine|start fresh observation changed/u,
    );
    assert.equal(changed, true);
    assert.equal(
      parse(join(prepared.active, ".mutation-close-02")).disposition,
      "aborted-before-effect",
    );
    assert.equal(existsSync(join(prepared.active, ".mutation-operation-02")), false);
    rmSync(collisionPath);
    const completed = publishColimaLiveProviderStartDecisionForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
    );
    assert.equal(
      parse(join(prepared.active, ".mutation-close-03")).disposition,
      "completed",
    );
    assertInertStartDecisionJournal(prepared.active, 3);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("an abandoned start decision is recovered only by an all-zero abort", async () => {
  const state = fixture();
  let preparation;
  let child;
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const active = prepared.active;
    const releasePath = join(state.root, "release-start-decision-writer");
    const helper = resolve(
      "scripts/fixtures/race-clean-engine-live-provider-start-decision.mjs",
    );
    child = spawn(
      process.execPath,
      [
        helper,
        state.repo,
        state.state,
        JSON.stringify({
          observation: preparation.observation,
          observationInput: preparation.input,
          requirements: preparation.requirements,
        }),
        "after-slot-acquisition",
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let childStderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      childStderr += chunk;
    });
    child.stdout.resume();
    const slotPath = join(active, ".mutation-slot-02");
    const deadline = Date.now() + 8_000;
    while (true) {
      try {
        const metadata = lstatSync(slotPath, { bigint: true });
        if (
          metadata.isFile() &&
          metadata.nlink === 1n &&
          parse(slotPath).action === "provider-start-decision" &&
          !readdirSync(active).some((name) =>
            name.startsWith(".mutation-stage-"),
          )
        ) {
          break;
        }
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
      assert.ok(
        Date.now() < deadline,
        `timed out waiting for start decision slot: ${childStderr}`,
      );
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");

    const beforeConfirmation = snapshotTree(state.state);
    const confirmation =
      liveProviderFixtureStartDecisionRecoveryConfirmationForTest({
        repoRoot: state.repo,
        stateBase: state.state,
      });
    assert.deepEqual(snapshotTree(state.state), beforeConfirmation);
    assert.throws(
      () =>
        liveProviderStartDecisionRecoveryConfirmationForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /start decision recovery action was refused/u,
    );
    assert.deepEqual(snapshotTree(state.state), beforeConfirmation);
    assert.throws(
      () =>
        recoverLiveProviderStartDecisionForTest({
          confirmation: `${confirmation}-wrong`,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      (error) => {
        assert.equal(error.exitStatus, 64);
        return true;
      },
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-02-00")), false);
    assert.throws(
      () =>
        recoverLiveProviderStartDecisionForExecutor({
          confirmation,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /effect-free decision slot/u,
    );
    assert.throws(
      () =>
        recoverProviderCreateForExecutor({
          adapter: fakeProviderAdapter(),
          confirmation,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /matching dedicated executor/u,
    );

    const recovered = recoverLiveProviderStartDecisionForTest({
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "plan");
    const recoveryPath = join(active, ".mutation-recovery-02-00");
    const closePath = join(active, ".mutation-close-02");
    const recovery = parse(recoveryPath);
    const close = parse(closePath);
    assert.equal(recovery.schema, "synveda.clean-engine.mutation-recovery.v6");
    assert.equal(recovery.action, "provider-start-decision");
    assert.deepEqual(
      {
        disposition: recovery.observed_effect_disposition,
        effect: recovery.observed_effect_name,
        evidenceHead: recovery.observed_evidence_head_sha256,
        evidencePrefix: recovery.observed_evidence_prefix_sha256,
        evidenceStage: recovery.observed_evidence_stage,
        residual: recovery.observed_residual_sha256,
        settlement: recovery.observed_settlement_sha256,
      },
      {
        disposition: "not-reached",
        effect: "none",
        evidenceHead: "0".repeat(64),
        evidencePrefix: "0".repeat(64),
        evidenceStage: "none",
        residual: "0".repeat(64),
        settlement: "0".repeat(64),
      },
    );
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(readFileSync(recoveryPath)));
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));

    const recoveryBytes = readFileSync(recoveryPath);
    const closeBytes = readFileSync(closePath);
    for (const [field, replacement] of [
      ["observed_effect_disposition", "pending"],
      ["observed_effect_name", "provider-process-start"],
      ["observed_evidence_head_sha256", "f".repeat(64)],
      ["observed_evidence_prefix_sha256", "f".repeat(64)],
      ["observed_evidence_stage", "process-started"],
      ["observed_residual_sha256", "f".repeat(64)],
      ["observed_settlement_sha256", "f".repeat(64)],
    ]) {
      const changedRecovery = { ...recovery, [field]: replacement };
      const changedRecoveryBytes = canonicalBytes(changedRecovery);
      writeFileSync(recoveryPath, changedRecoveryBytes, { mode: 0o600 });
      writeFileSync(
        closePath,
        canonicalBytes({
          ...close,
          authority_sha256: sha256(changedRecoveryBytes),
        }),
        { mode: 0o600 },
      );
      const refused = run(state, "status");
      assert.equal(refused.status, 78, field);
      assert.equal(
        refused.stderr,
        "clean-engine: deterministic provider recovery observation was refused\n",
        field,
      );
      writeFileSync(recoveryPath, recoveryBytes, { mode: 0o600 });
      writeFileSync(closePath, closeBytes, { mode: 0o600 });
    }

    const completed = publishColimaLiveProviderStartDecisionForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
    );
    assert.equal(
      parse(join(active, ".mutation-close-03")).disposition,
      "completed",
    );
    assertInertStartDecisionJournal(active, 3);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("start decision publication refuses collisions at every journal boundary and retries", () => {
  const cases = [
    {
      checkpoint: undefined,
      name: "initial",
      prepare(collisionPath) {
        writePrivateColimaLiveFixtureFile(
          collisionPath,
          Buffer.from("initial decision collision\n", "utf8"),
          0o600,
        );
      },
      permanentSlot: false,
    },
    {
      checkpoint:
        "before-slot-link:after-first-process-start-admission-observation",
      name: "slot-prelink",
      prepare() {},
      permanentSlot: false,
    },
    {
      checkpoint:
        "after-slot-acquisition:after-first-process-start-admission-observation",
      name: "post-acquisition-observation",
      prepare() {},
      permanentSlot: true,
    },
    {
      checkpoint: "after-slot-acquisition",
      name: "before-close-observation",
      prepare() {},
      permanentSlot: true,
    },
    {
      checkpoint:
        "before-close-link:after-first-process-start-admission-observation",
      name: "close-prelink",
      prepare() {},
      permanentSlot: true,
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      const active = prepared.active;
      const collisionPath = join(
        preparation.input.environment.COLIMA_HOME,
        preparation.input.provider_profile,
      );
      value.prepare(collisionPath);
      let injected = false;
      const beforeState = snapshotTree(state.state);
      const beforeEntries = readdirSync(active).sort();
      assert.throws(
        () =>
          publishColimaLiveProviderStartDecisionForTest(
            fixtureStartAdmissionArguments(
              state,
              preparation,
              (checkpoint) => {
                if (checkpoint === value.checkpoint && !injected) {
                  injected = true;
                  writePrivateColimaLiveFixtureFile(
                    collisionPath,
                    Buffer.from(`${value.name} decision collision\n`, "utf8"),
                    0o600,
                  );
                }
              },
            ),
          ),
        (error) => {
          assert.equal(error.exitStatus, 73, value.name);
          return true;
        },
      );
      assert.equal(
        readdirSync(active).some((name) => name.startsWith(".mutation-stage-")),
        false,
        value.name,
      );
      assert.equal(
        existsSync(join(active, ".mutation-slot-02")),
        value.permanentSlot,
        value.name,
      );
      assert.equal(
        existsSync(join(active, ".mutation-close-02")),
        value.permanentSlot,
        value.name,
      );
      if (value.permanentSlot) {
        assert.equal(
          parse(join(active, ".mutation-close-02")).disposition,
          "aborted-before-effect",
          value.name,
        );
        assertInertStartDecisionJournal(active, 2);
      } else {
        assert.deepEqual(readdirSync(active).sort(), beforeEntries, value.name);
        if (value.name === "initial") {
          assert.deepEqual(snapshotTree(state.state), beforeState, value.name);
        }
      }
      assert.equal(existsSync(join(active, ".mutation-operation-02")), false);
      if (existsSync(collisionPath)) unlinkSync(collisionPath);
      const completed = publishColimaLiveProviderStartDecisionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      assert.equal(
        completed.schema,
        COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
        value.name,
      );
      const completedSequence = value.permanentSlot ? "03" : "02";
      assert.equal(
        parse(join(active, `.mutation-close-${completedSequence}`)).disposition,
        "completed",
        value.name,
      );
      assertInertStartDecisionJournal(
        active,
        value.permanentSlot ? 3 : 2,
      );
      assert.equal(run(state, "verify").status, 0, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("two start decision writers leave one inert completed CAS winner", async () => {
  const state = fixture();
  let preparation;
  const children = [];
  try {
    const prepared = prepareCompletedLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const active = prepared.active;
    const helper = resolve(
      "scripts/fixtures/race-clean-engine-live-provider-start-decision.mjs",
    );
    const releasePath = join(state.root, "release-start-decision-writers");
    const preStageReleasePath = `${releasePath}-pre-stage`;
    const serialized = JSON.stringify({
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    });
    const launch = () => {
      const child = spawn(
        process.execPath,
        [
          helper,
          state.repo,
          state.state,
          serialized,
          "after-slot-authority-reassertion",
          releasePath,
          preStageReleasePath,
        ],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      const output = { child, stderr: "" };
      child.stdout.resume();
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        output.stderr += chunk;
      });
      children.push(output);
      return output;
    };
    const first = launch();
    const second = launch();
    const waitForBoth = async (path, label) => {
      const deadline = Date.now() + 8_000;
      while (
        readdirSync(state.root).filter((name) =>
          name.startsWith(`${basename(path)}.ready-`),
        ).length !== 2
      ) {
        assert.ok(
          Date.now() < deadline,
          `timed out waiting for ${label}: ${first.stderr}${second.stderr}`,
        );
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
      }
    };
    await waitForBoth(preStageReleasePath, "pre-stage decision writers");
    writeFileSync(preStageReleasePath, "release\n", { mode: 0o600 });
    await waitForBoth(releasePath, "decision stages");
    assert.equal(
      readdirSync(active).filter((name) => name.startsWith(".mutation-stage-"))
        .length,
      2,
    );
    writeFileSync(releasePath, "release\n", { mode: 0o600 });
    const results = await Promise.all(
      children.map((output) =>
        new Promise((resolvePromise) => {
          output.child.on("close", (status, signal) =>
            resolvePromise({ signal, status, stderr: output.stderr }),
          );
        }),
      ),
    );
    children.length = 0;
    assert.deepEqual(
      results.map((value) => value.status).sort((left, right) => left - right),
      [0, 73],
      results.map((value) => value.stderr).join("\n"),
    );
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(
      results.find((value) => value.status === 73)?.stderr,
      "another clean-engine mutation is active\n",
    );
    assertInertStartDecisionJournal(active, 2);
    assert.equal(
      parse(join(active, ".mutation-close-02")).disposition,
      "completed",
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    for (const { child } of children) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("start decision crash boundaries reconcile without inventing effects", async () => {
  const cases = [
    {
      checkpoint:
        "before-slot-link:after-first-process-start-admission-observation",
      disposition: "stage-only",
      ready(active) {
        return (
          !existsSync(join(active, ".mutation-slot-02")) &&
          readdirSync(active).filter((name) =>
            name.startsWith(".mutation-stage-"),
          ).length === 1
        );
      },
    },
    {
      checkpoint: "after-slot-link",
      disposition: "open",
      ready(active) {
        const slot = join(active, ".mutation-slot-02");
        const stageName = readdirSync(active).find((name) =>
          name.startsWith(".mutation-stage-"),
        );
        if (!existsSync(slot) || stageName === undefined) return false;
        const stage = join(active, stageName);
        return (
          lstatSync(slot).nlink === 2 &&
          lstatSync(stage).nlink === 2 &&
          lstatSync(slot).ino === lstatSync(stage).ino
        );
      },
    },
    {
      checkpoint:
        "before-close-link:after-first-process-start-admission-observation",
      disposition: "open",
      ready(active) {
        return (
          existsSync(join(active, ".mutation-slot-02")) &&
          !existsSync(join(active, ".mutation-close-02")) &&
          readdirSync(active).some((name) => {
            if (!name.startsWith(".mutation-stage-")) return false;
            return lstatSync(join(active, name)).nlink === 1;
          })
        );
      },
    },
    {
      checkpoint: "after-close-link",
      disposition: "completed",
      ready(active) {
        const close = join(active, ".mutation-close-02");
        const stageName = readdirSync(active).find((name) =>
          name.startsWith(".mutation-stage-"),
        );
        if (!existsSync(close) || stageName === undefined) return false;
        const stage = join(active, stageName);
        return (
          lstatSync(close).nlink === 2 &&
          lstatSync(stage).nlink === 2 &&
          lstatSync(close).ino === lstatSync(stage).ino
        );
      },
    },
  ];

  for (const terminationSignal of ["SIGTERM", "SIGKILL"]) {
    for (const [index, value] of cases.entries()) {
      const state = fixture();
      let preparation;
      let child;
      try {
        const prepared = prepareCompletedLiveProviderIntentFixture(state);
        preparation = prepared.preparation;
        const active = prepared.active;
        const helper = resolve(
          "scripts/fixtures/race-clean-engine-live-provider-start-decision.mjs",
        );
        const releasePath = join(
          state.root,
          `release-decision-${terminationSignal}-${index}`,
        );
        child = spawn(
          process.execPath,
          [
            helper,
            state.repo,
            state.state,
            JSON.stringify({
              observation: preparation.observation,
              observationInput: preparation.input,
              requirements: preparation.requirements,
            }),
            value.checkpoint,
            releasePath,
          ],
          {
            env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
            stdio: ["ignore", "pipe", "pipe"],
          },
        );
        let stderr = "";
        child.stdout.resume();
        child.stderr.setEncoding("utf8");
        child.stderr.on("data", (chunk) => {
          stderr += chunk;
        });
        const readyPath = `${releasePath}.ready-${child.pid}`;
        const deadline = Date.now() + 8_000;
        while (!existsSync(readyPath) || !value.ready(active)) {
          assert.equal(child.exitCode, null, stderr);
          assert.ok(
            Date.now() < deadline,
            `timed out at ${value.checkpoint}: ${stderr}`,
          );
          await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
        }
        child.kill(terminationSignal);
        const [status, signal] = await once(child, "close");
        child = undefined;
        assert.equal(status, null, value.checkpoint);
        assert.equal(signal, terminationSignal, value.checkpoint);

        if (value.disposition === "stage-only") {
          assert.equal(existsSync(join(active, ".mutation-slot-02")), false);
        } else if (value.disposition === "open") {
          const confirmation =
            liveProviderFixtureStartDecisionRecoveryConfirmationForTest({
              repoRoot: state.repo,
              stateBase: state.state,
            });
          recoverLiveProviderStartDecisionForTest({
            confirmation,
            repoRoot: state.repo,
            stateBase: state.state,
          });
          assert.equal(
            parse(join(active, ".mutation-close-02")).disposition,
            "aborted-before-effect",
            value.checkpoint,
          );
        }
        const completed = publishColimaLiveProviderStartDecisionForTest(
          fixtureStartAdmissionArguments(state, preparation),
        );
        assert.equal(
          completed.schema,
          COLIMA_LIVE_FIXTURE_PROVIDER_START_DECISION_COMPLETION_SCHEMA,
          value.checkpoint,
        );
        assertInertStartDecisionJournal(
          active,
          value.disposition === "open" ? 3 : 2,
        );
        assert.equal(run(state, "verify").status, 0, value.checkpoint);
      } finally {
        if (child !== undefined) child.kill("SIGKILL");
        if (preparation !== undefined) {
          rmSync(preparation.root, { recursive: true, force: true });
        }
        rmSync(state.root, { recursive: true, force: true });
      }
    }
  }
});

test("completed start decision admits only a current deny-only effect candidate", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const active = prepared.active;
    const stateBefore = snapshotTree(state.root);
    const preparationBefore = snapshotTree(preparation.root);
    const checkpoints = [];
    const admission =
      observeColimaLiveProviderStartEffectFreshAdmissionForTest(
        fixtureStartAdmissionArguments(
          state,
          preparation,
          (checkpoint) => checkpoints.push(checkpoint),
        ),
      );
    assert.deepEqual(Object.keys(admission).sort(), [
      "authority",
      "completed_start_decision_projection",
      "process_start_effect_candidate",
      "root_observation",
      "schema",
      "supervisor_process_group_label",
    ]);
    assert.equal(
      admission.schema,
      COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
    );
    assert.equal(
      admission.authority,
      "fixture-only-point-in-time-deny-only-not-effect-authority",
    );
    const projection = admission.completed_start_decision_projection;
    assert.equal(
      projection.schema,
      COLIMA_LIVE_FIXTURE_COMPLETED_PROVIDER_START_DECISION_PROJECTION_SCHEMA,
    );
    assert.equal(
      projection.provider_start_decision_slot_sha256,
      prepared.startDecisionCompletion.decision_slot_sha256,
    );
    assert.equal(
      projection.provider_start_decision_close_sha256,
      prepared.startDecisionCompletion.decision_close_sha256,
    );
    assert.equal(
      projection.provider_start_decision_publication_plan_sha256,
      prepared.startDecisionCompletion.decision_publication_plan_sha256,
    );
    assert.equal(
      projection.decision_candidate_sha256,
      prepared.startDecisionCompletion.decision_candidate_sha256,
    );
    assert.equal(
      projection.completed_intent_projection_sha256,
      prepared.startDecisionCompletion.completed_intent_projection_sha256,
    );
    const candidate = admission.process_start_effect_candidate;
    assert.notEqual(candidate, null);
    assert.equal(
      candidate.schema,
      COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_CANDIDATE_SCHEMA,
    );
    assert.equal(candidate.effect_name, "provider-process-start");
    assert.equal(
      candidate.effect_authorization,
      "denied-no-durable-effect-contract",
    );
    assert.equal(candidate.process_state, "not-reached");
    assert.equal(candidate.durable_effect_slot_required, true);
    assert.equal(candidate.future_effect_fresh_admission_required, true);
    assert.equal(candidate.process_start_authorized, false);
    assert.equal(candidate.process_spawn_authorized, false);
    assert.equal(candidate.process_signal_authorized, false);
    assert.equal(candidate.process_group_ownership_authorized, false);
    assert.equal(candidate.provider_root_mutation_authorized, false);
    assert.equal(candidate.provider_adapter_execution_authorized, false);
    assert.equal(candidate.effect_execution_authorized, false);
    assert.equal(candidate.evidence_publication_authorized, false);
    assert.equal(candidate.receipt_publication_authorized, false);
    assert.equal(candidate.provider_recovery_authorized, false);
    assert.equal(candidate.lifecycle_exposed, false);
    assert.equal(candidate.finalization_authorized, false);
    assert.equal(candidate.cross_namespace_atomic_reservation, false);
    assert.equal(
      candidate.completed_start_decision_projection_sha256,
      sha256(liveProviderProcessStartBytes(projection)),
    );
    assert.deepEqual(checkpoints, [
      "after-first-process-start-effect-admission-observation",
    ]);
    assertRecursivelyFrozen(admission);
    assert.deepEqual(snapshotTree(state.root), stateBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
    assertInertStartDecisionJournal(active, 2);

    checkpoints.length = 0;
    const repeated =
      observeColimaLiveProviderStartEffectFreshAdmissionForTest(
        fixtureStartAdmissionArguments(
          state,
          preparation,
          (checkpoint) => checkpoints.push(checkpoint),
        ),
      );
    assert.deepEqual(repeated, admission);
    assert.deepEqual(checkpoints, [
      "after-first-process-start-effect-admission-observation",
    ]);
    assertInertStartDecisionJournal(active, 2);

    const collisionPath = join(
      preparation.input.environment.COLIMA_HOME,
      preparation.input.provider_profile,
    );
    writePrivateColimaLiveFixtureFile(
      collisionPath,
      Buffer.from("foreign collision\n", "utf8"),
      0o600,
    );
    const collisionStateBefore = snapshotTree(state.root);
    const collisionPreparationBefore = snapshotTree(preparation.root);
    const collision =
      observeColimaLiveProviderStartEffectFreshAdmissionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
    assert.equal(
      collision.root_observation.root_set_disposition,
      "foreign-collision",
    );
    assert.equal(collision.process_start_effect_candidate, null);
    assertRecursivelyFrozen(collision);
    assert.deepEqual(snapshotTree(state.root), collisionStateBefore);
    assert.deepEqual(snapshotTree(preparation.root), collisionPreparationBefore);
    assertInertStartDecisionJournal(active, 2);
    unlinkSync(collisionPath);

    const restored =
      observeColimaLiveProviderStartEffectFreshAdmissionForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
    assert.notEqual(restored.process_start_effect_candidate, null);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the state owner retires the fixture provider effect before attempt with no receipt", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const checkpoints = [];
    const argumentsValue = fixtureStartAdmissionArguments(
      state,
      preparation,
      (checkpoint) => {
        checkpoints.push(checkpoint);
      },
    );
    const completion =
      publishColimaLiveProviderEffectPreAttemptForTest(argumentsValue);
    assert.equal(
      completion.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS.event.completion,
    );
    assert.equal(completion.event_kind, "completion");
    assert.equal(completion.event_sequence, 1);
    assert.equal(
      completion.operation_kind,
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
    );
    assert.equal(
      completion.operation_contract_sha256,
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
    );
    assert.equal(
      completion.evidence.variant,
      "pre-attempt-retired-zero-receipt",
    );
    for (const field of [
      "cleanup_settlement_sha256",
      "create_settlement_sha256",
      "receipt_event_sha256",
      "receipt_sha256",
      "start_attempt_sha256",
    ]) {
      assert.equal(completion.evidence[field], "0".repeat(64));
    }
    assert.equal(
      existsSync(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
      ),
      false,
    );
    assertPrivate(
      join(prepared.active, ".provider-effect-witness-03"),
      0o600,
      false,
      1,
    );
    assertPrivate(join(prepared.active, ".provider-effect-event-03-000"), 0o600);
    assertPrivate(join(prepared.active, ".provider-effect-event-03-001"), 0o600);
    assertPrivate(join(prepared.active, ".mutation-close-03"), 0o600);
    assert.equal(
      parse(join(prepared.active, ".mutation-slot-03")).schema,
      "synveda.clean-engine.mutation-slot.v7",
    );
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.schema, "synveda.clean-engine.mutation-close.v8");
    assert.equal(close.disposition, "completed");
    assert.equal(
      close.operation_evidence_sha256,
      sha256(canonicalBytes(completion)),
    );
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        /^\d{2}-.*\.json$/u.test(name),
      ),
      ["00-plan.json"],
    );
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        name.startsWith(".provider-effect-stage-"),
      ),
      false,
    );
    assert.equal(run(state, "verify").status, 0);
    assert.deepEqual(
      publishColimaLiveProviderEffectPreAttemptForTest(argumentsValue),
      completion,
    );
    const completedState = snapshotTree(state.root);
    assert.throws(
      () => publishColimaLiveProviderEffectAttemptFenceForTest(argumentsValue),
      /fixture provider effect terminal differed/u,
    );
    assert.deepEqual(snapshotTree(state.root), completedState);
    assert.equal(checkpoints.includes("after-start-authority"), true);
    assert.equal(checkpoints.includes("after-completion"), true);
    assert.equal(checkpoints.includes("after-effect-close"), true);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the fixture provider effect attempt fence is durable and process-free", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fenced = publishColimaLiveProviderEffectAttemptFenceForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    const active = prepared.active;
    const eventPath = join(active, ".provider-effect-event-03-001");
    const eventBytes = readFileSync(eventPath);
    assert.deepEqual(fenced, {
      disposition: "attempt-fenced",
      event_sha256: sha256(eventBytes),
      process_invoked: false,
      receipt_published: false,
    });
    assert.match(fenced.event_sha256, /^[0-9a-f]{64}$/u);
    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const witness = join(active, ".provider-effect-witness-03");
    assertPrivate(marker, 0o600, false, 2);
    assertPrivate(witness, 0o600, false, 2);
    const markerIdentity = lstatSync(marker, { bigint: true });
    const witnessIdentity = lstatSync(witness, { bigint: true });
    assert.equal(markerIdentity.nlink, 2n);
    assert.equal(witnessIdentity.nlink, 2n);
    assert.equal(markerIdentity.dev, witnessIdentity.dev);
    assert.equal(markerIdentity.ino, witnessIdentity.ino);
    assert.deepEqual(readFileSync(marker), readFileSync(witness));
    const slotPath = join(active, ".mutation-slot-03");
    const slot = parse(slotPath);
    const publicationPlan = slot.operation_plan;
    const witnessValue = parse(witness);
    const blueprintPath = resolve(
      "deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs",
    );
    const componentPaths =
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS.map(
        (component) =>
          component.module_path === null
            ? realpathSync(process.execPath)
            : resolve(dirname(blueprintPath), component.module_path),
      );
    const componentManifest =
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS.map(
        (component, index) => ({
          kind: component.kind,
          sha256: sha256(readFileSync(componentPaths[index])),
        }),
      );
    const closedEnvironment =
      process.platform === "darwin"
        ? { __CF_USER_TEXT_ENCODING: process.env.__CF_USER_TEXT_ENCODING }
        : {};
    assert.match(
      closedEnvironment.__CF_USER_TEXT_ENCODING ?? "linux",
      /^(?:linux|0x[0-9A-F]+:[0-9]+:[0-9]+)$/u,
    );
    assert.equal(
      publicationPlan.invocation_binding.environment_sha256,
      liveProviderEffectDigest(liveProviderEffectBytes(closedEnvironment)),
    );
    assert.equal(
      publicationPlan.invocation_binding.executable_sha256,
      componentManifest[0].sha256,
    );
    const rolePath = componentPaths[
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS.findIndex(
        (component) => component.kind === "role",
      )
    ];
    const adapterComponent = componentManifest.find(
      (component) => component.kind === "conclusive-adapter",
    );
    assert.notEqual(rolePath, undefined);
    assert.notEqual(adapterComponent, undefined);
    for (const [sequence, roleContract] of
      publicationPlan.planned_role_contracts.entries()) {
      const expectedArgv = [
        componentPaths[0],
        rolePath,
        roleContract.role,
        join(
          preparation.input.environment.TMPDIR,
          `.synveda-cpr45-effect-edge-${String(sequence).padStart(2, "0")}.json`,
        ),
      ];
      assert.equal(
        roleContract.argv_sha256,
        liveProviderEffectDigest(liveProviderEffectBytes(expectedArgv)),
      );
      assert.equal(
        roleContract.role_challenge_sha256,
        fixtureEffectPrivateHmac(
          preparation.input.binding_key,
          publicationPlan.fixture_id,
          `${roleContract.role}-role-challenge`,
          { role: roleContract.role },
        ),
      );
      const roleKey = fixtureEffectRoleKey(
        preparation.input.binding_key,
        publicationPlan.fixture_id,
        roleContract.role,
      );
      const publicDer = roleKey.publicKey.export({
        format: "der",
        type: "spki",
      });
      assert.equal(
        roleContract.public_key_spki_sha256,
        liveProviderEffectDigest(publicDer),
      );
      const proof = Buffer.from(`fixture-role-proof:${roleContract.role}`, "utf8");
      const signature = sign(null, proof, roleKey.privateKey);
      assert.equal(verify(null, proof, roleKey.publicKey, signature), true);
      assert.doesNotMatch(
        readFileSync(slotPath, "utf8"),
        new RegExp(
          roleKey.privateKey
            .export({ format: "der", type: "pkcs8" })
            .toString("hex"),
          "u",
        ),
      );
    }
    const endpointPaths = {
      "engine-api": join(
        preparation.input.environment.COLIMA_HOME,
        "engine.sock",
      ),
      "hostagent-control": join(
        preparation.input.environment.LIMA_HOME,
        "hostagent.sock",
      ),
      "ssh-control": join(
        preparation.input.environment.TMPDIR,
        "ssh-control.sock",
      ),
      "usernet-control": join(
        preparation.input.environment.LIMA_HOME,
        "usernet.sock",
      ),
    };
    const dockerContextIdentity = fixtureEffectPrivateHmac(
      preparation.input.binding_key,
      publicationPlan.fixture_id,
      "docker-context-identity",
      { fixture_id: publicationPlan.fixture_id },
    );
    const dockerContextPath = join(
      preparation.input.environment.DOCKER_CONFIG,
      "contexts",
      "meta",
      dockerContextIdentity,
      "meta.json",
    );
    for (const endpoint of publicationPlan.planned_endpoint_contracts) {
      assert.equal(
        endpoint.path_identity_hmac_sha256,
        fixtureEffectPrivateHmac(
          preparation.input.binding_key,
          publicationPlan.fixture_id,
          `${endpoint.endpoint_kind}-path`,
          { path: endpointPaths[endpoint.endpoint_kind] },
        ),
      );
      assert.equal(
        endpoint.initial_challenge_commitment_sha256,
        fixtureEffectPrivateHmac(
          preparation.input.binding_key,
          publicationPlan.fixture_id,
          `${endpoint.endpoint_kind}-initial-challenge`,
          { endpoint_kind: endpoint.endpoint_kind },
        ),
      );
      assert.equal(
        endpoint.final_challenge_commitment_sha256,
        fixtureEffectPrivateHmac(
          preparation.input.binding_key,
          publicationPlan.fixture_id,
          `${endpoint.endpoint_kind}-final-challenge`,
          { endpoint_kind: endpoint.endpoint_kind },
        ),
      );
      if (endpoint.endpoint_kind === "engine-api") {
        assert.equal(
          endpoint.docker_context_path_identity_hmac_sha256,
          fixtureEffectPrivateHmac(
            preparation.input.binding_key,
            publicationPlan.fixture_id,
            "engine-api-docker-context-path",
            { path: dockerContextPath },
          ),
        );
      }
    }
    assert.equal(
      publicationPlan.planned_start_attempt_sha256,
      fixtureEffectPrivateHmac(
        preparation.input.binding_key,
        publicationPlan.fixture_id,
        "start-attempt",
        {
          adapter_component_sha256: adapterComponent.sha256,
          adapter_contract_sha256:
            FIXTURE_EFFECT_ADAPTER_CONTRACT_SHA256,
          fixture_id: publicationPlan.fixture_id,
        },
      ),
    );
    assert.equal(
      publicationPlan.planned_quiescence_fence_sha256,
      fixtureEffectPrivateHmac(
        preparation.input.binding_key,
        publicationPlan.fixture_id,
        "quiescence-fence",
        { fixture_id: publicationPlan.fixture_id },
      ),
    );
    const blueprint =
      buildColimaLiveProviderEffectFixtureBlueprintStructure({
        adapter_contract_sha256:
          FIXTURE_EFFECT_ADAPTER_CONTRACT_SHA256,
        architecture: process.arch,
        component_manifest: componentManifest,
        endpoint_bindings: publicationPlan.planned_endpoint_contracts.map(
          ({ role: _role, ...binding }) => binding,
        ),
        environment_sha256:
          publicationPlan.invocation_binding.environment_sha256,
        fixture_id: publicationPlan.fixture_id,
        operation_contract_sha256:
          publicationPlan.operation_contract_sha256,
        operation_kind: publicationPlan.operation_kind,
        planned_quiescence_fence_sha256:
          publicationPlan.planned_quiescence_fence_sha256,
        planned_start_attempt_sha256:
          publicationPlan.planned_start_attempt_sha256,
        platform: process.platform,
        role_bindings: publicationPlan.planned_role_contracts.map(
          ({
            argv_sha256,
            cwd_identity_sha256,
            public_key_spki_sha256,
            role,
            role_challenge_sha256,
            uid,
          }) => ({
            argv_sha256,
            cwd_identity_sha256,
            public_key_spki_sha256,
            role,
            role_challenge_sha256,
            uid,
          }),
        ),
      });
    assert.deepEqual(
      publicationPlan.invocation_binding,
      blueprint.invocation_binding,
    );
    assert.deepEqual(
      publicationPlan.planned_role_contracts,
      blueprint.role_contracts,
    );
    assert.deepEqual(
      publicationPlan.planned_endpoint_contracts,
      blueprint.endpoint_contracts,
    );
    assert.equal(
      publicationPlan.driver_contract_sha256,
      blueprint.driver_contract_sha256,
    );
    assert.equal(
      witnessValue.invocation_binding_sha256,
      liveProviderEffectDigest(
        liveProviderEffectBytes(publicationPlan.invocation_binding),
      ),
    );
    assert.equal(
      witnessValue.planned_role_contracts_sha256,
      liveProviderEffectDigest(
        liveProviderEffectBytes(publicationPlan.planned_role_contracts),
      ),
    );
    assert.equal(
      witnessValue.planned_endpoint_contracts_sha256,
      liveProviderEffectDigest(
        liveProviderEffectBytes(publicationPlan.planned_endpoint_contracts),
      ),
    );
    const persisted = Buffer.concat([
      readFileSync(slotPath),
      readFileSync(witness),
      eventBytes,
    ]).toString("utf8");
    for (const secret of [
      preparation.input.binding_key.toString("hex"),
      preparation.providerRoot,
      ...Object.values(preparation.input.environment).filter(
        (value) => typeof value === "string" && value.startsWith("/"),
      ),
      ...componentPaths,
    ]) {
      assert.equal(persisted.includes(secret), false, secret);
    }
    const stateSource = readFileSync(stateTool, "utf8");
    assert.doesNotMatch(stateSource, /effectCommitment\(|fixed-fixture-/u);
    assert.doesNotMatch(stateSource, /FixtureComponentCache/u);
    assert.match(stateSource, /constants\.O_NOFOLLOW/u);
    assert.match(stateSource, /exactFstat\(descriptor\)/u);
    assert.equal(existsSync(join(active, ".mutation-close-03")), false);
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\d{2}-.*\.json$/u.test(name)),
      ["00-plan.json"],
    );
    assert.equal(
      JSON.parse(eventBytes.toString("utf8")).event_kind,
      "start-attempt",
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the state owner retires one conclusive fixture attempt through an exact receipt", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const checkpoints = [];
    const argumentsValue = fixtureStartAdmissionArguments(
      state,
      preparation,
      (checkpoint) => {
        checkpoints.push(checkpoint);
      },
    );
    const completion =
      publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
        argumentsValue,
      );
    assert.equal(completion.event_kind, "completion");
    assert.equal(completion.event_sequence, 10);
    assert.equal(completion.evidence.variant, "attempted-effect-retired");
    assert.equal(completion.evidence.marker_present, false);
    assert.equal(completion.evidence.witness_link_count, 1);

    const eventNames = readdirSync(prepared.active)
      .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
      .sort();
    assert.equal(eventNames.length, 11);
    const events = eventNames.map((name) =>
      parse(join(prepared.active, name)),
    );
    assert.deepEqual(
      events.map((event) => event.event_kind),
      [
        "start-authority",
        "start-attempt",
        "launch-edge",
        "delivery-result",
        "create-settlement",
        "cleanup-plan-page",
        "cleanup-plan",
        "cleanup-progress",
        "cleanup-settlement",
        "terminal-receipt",
        "completion",
      ],
    );
    const slot = parse(join(prepared.active, ".mutation-slot-03"));
    const witnessValue = parse(
      join(prepared.active, ".provider-effect-witness-03"),
    );
    for (const [index, event] of events.entries()) {
      assert.equal(event.event_sequence, index);
      assert.equal(
        event.parent_sha256,
        index === 0
          ? sha256(canonicalBytes(witnessValue))
          : sha256(canonicalBytes(events[index - 1])),
      );
      assert.equal(
        event.publisher_authority_sha256,
        sha256(canonicalBytes(slot)),
      );
    }
    const delivery = events[3];
    assert.deepEqual(Object.keys(delivery.evidence).sort(), [
      "attempt_event_sha256",
      "child_handle_identity_sha256",
      "delivery_disposition",
      "driver_contract_sha256",
      "effect_possible",
      "observation_sha256",
      "outer_launch_edge_sha256",
      "safe_error_code",
    ]);
    assert.equal(delivery.evidence.delivery_disposition, "conclusive-not-created");
    assert.equal(delivery.evidence.child_handle_identity_sha256, "0".repeat(64));
    assert.equal(delivery.evidence.effect_possible, false);
    assert.equal(delivery.evidence.safe_error_code, "not-created");

    const createSettlement = events[4].evidence;
    assert.equal(createSettlement.variant, "exact-attempted-residual");
    assert.equal(createSettlement.residual_basis, "conclusive-not-created");
    assert.equal(createSettlement.causal_edge_count, 1);
    assert.equal(createSettlement.causal_node_count, 0);
    assert.equal(
      createSettlement.endpoint_set_sha256,
      "37517e5f3dc66819f61f5a7bb8ace1921282415f10551d2defa5c3eb0985b570",
    );
    assert.equal(
      createSettlement.role_identity_set_sha256,
      "37517e5f3dc66819f61f5a7bb8ace1921282415f10551d2defa5c3eb0985b570",
    );
    assert.equal(createSettlement.inventory_set_sha256, "0".repeat(64));
    assert.equal(createSettlement.quiescence_fence_sha256, "0".repeat(64));

    const pageActions = events[5].evidence.actions;
    assert.equal(pageActions.length, 7);
    assert.deepEqual(
      pageActions.map((action) => action.action_kind),
      [
        "prove-process-subtree-absent",
        ...Array(6).fill("prove-namespace-baseline"),
      ],
    );
    assert.deepEqual(
      pageActions.map((action) => action.namespace_role),
      [
        "none",
        "temporary-namespace",
        "private-home-namespace",
        "lima-home-namespace",
        "docker-config-namespace",
        "colima-home-namespace",
        "colima-cache-namespace",
      ],
    );
    assert.equal(events[6].evidence.action_count, 7);
    assert.equal(events[7].evidence.outcomes.length, 7);
    assert.deepEqual(
      events[7].evidence.outcomes.map((outcome) => outcome.disposition),
      Array(7).fill("completed-exact"),
    );

    const terminal = events[9];
    const receipt = terminal.evidence.receipt;
    assert.equal(receipt.phase, "provider-effect-retired");
    assert.equal(terminal.evidence.receipt_sha256, sha256(canonicalBytes(receipt)));
    const receiptPath = join(prepared.active, receiptFileName(receipt));
    assertPrivate(receiptPath, 0o600);
    assert.deepEqual(readFileSync(receiptPath), canonicalBytes(receipt));
    assert.equal(existsSync(join(prepared.active, ".receipt-publish")), false);

    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const witness = join(prepared.active, ".provider-effect-witness-03");
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.disposition, "completed");
    assert.equal(close.authority, "owner");
    assert.equal(close.result_sequence, slot.source_sequence + 1);
    assert.equal(close.result_head_sha256, sha256(canonicalBytes(receipt)));
    assert.equal(close.operation_evidence_sha256, sha256(canonicalBytes(completion)));
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);

    const persisted = Buffer.concat([
      ...eventNames.map((name) => readFileSync(join(prepared.active, name))),
      readFileSync(receiptPath),
    ]).toString("utf8");
    for (const forbidden of [
      preparation.input.binding_key.toString("hex"),
      preparation.providerRoot,
      ...Object.values(preparation.input.environment).filter(
        (value) => typeof value === "string" && value.startsWith("/"),
      ),
      "conclusive-adapter-result.v1",
    ]) {
      assert.equal(persisted.includes(forbidden), false, forbidden);
    }
    assert.ok(
      checkpoints.indexOf(
        "after-terminal-receipt-publication-stage-directory-sync",
      ) <
        checkpoints.indexOf("after-marker-unlink"),
    );
    assert.ok(
      checkpoints.indexOf("after-marker-unlink") <
        checkpoints.indexOf("after-completion"),
    );
    const completedState = snapshotTree(state.root);
    assert.deepEqual(
      publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
        argumentsValue,
      ),
      completion,
    );
    assert.deepEqual(snapshotTree(state.root), completedState);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("recovery never replays a conclusive canary without a durable delivery event", async () => {
  for (const [checkpoint, eventCount] of [
    ["after-start-attempt", 2],
    ["after-launch-edge", 3],
    ["before-conclusive-fixture-adapter", 3],
    ["after-conclusive-fixture-adapter", 3],
    ["after-delivery-result-stage", 3],
  ]) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      await crashLiveProviderEffectAt(state, {
        action: "conclusive",
        checkpoint,
        input: {
          observation: preparation.observation,
          observationInput: preparation.input,
          requirements: preparation.requirements,
        },
        releaseName: `conclusive-blocked-${checkpoint}`,
      });
      const events = readdirSync(prepared.active).filter((name) =>
        /^\.provider-effect-event-03-\d{3}$/u.test(name),
      );
      assert.equal(events.length, eventCount, checkpoint);
      const before = snapshotTree(state.root);
      const recoveryArguments = {
        observation: preparation.observation,
        observationInput: preparation.input,
        repoRoot: state.repo,
        requirements: preparation.requirements,
        stateBase: state.state,
      };
      assert.throws(
        () =>
          liveProviderFixtureEffectRecoveryConfirmationForTest(
            recoveryArguments,
          ),
        /attempt fence requires operator resolution/u,
      );
      assert.deepEqual(snapshotTree(state.root), before);
      if (checkpoint === "after-delivery-result-stage") {
        const stageNames = readdirSync(prepared.active).filter((name) =>
          name.startsWith(".mutation-stage-"),
        );
        assert.equal(stageNames.length, 1);
        assertPrivate(join(prepared.active, stageNames[0]), 0o600);
        const durableEvents = readdirSync(prepared.active)
          .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
          .sort()
          .map((name) => {
            const path = join(prepared.active, name);
            const metadata = lstatSync(path, { bigint: true });
            return {
              bytes: readFileSync(path),
              device: metadata.dev,
              inode: metadata.ino,
              name,
            };
          });
        const marker = join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        );
        const markerBytes = readFileSync(marker);
        const markerIdentity = lstatSync(marker, { bigint: true });
        assert.throws(
          () =>
            publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
              fixtureStartAdmissionArguments(state, preparation),
            ),
          /fixture provider effect start decision was unavailable/u,
        );
        assert.equal(existsSync(join(prepared.active, stageNames[0])), false);
        assert.equal(durableEvents.length, 3);
        for (const event of durableEvents) {
          const path = join(prepared.active, event.name);
          const metadata = lstatSync(path, { bigint: true });
          assert.equal(metadata.dev, event.device);
          assert.equal(metadata.ino, event.inode);
          assert.deepEqual(readFileSync(path), event.bytes);
        }
        const markerAfter = lstatSync(marker, { bigint: true });
        assert.equal(markerAfter.dev, markerIdentity.dev);
        assert.equal(markerAfter.ino, markerIdentity.ino);
        assert.equal(markerAfter.nlink, 2n);
        assert.deepEqual(readFileSync(marker), markerBytes);
        assert.throws(
          () =>
            liveProviderFixtureEffectRecoveryConfirmationForTest(
              recoveryArguments,
            ),
          /attempt fence requires operator resolution/u,
        );
      } else {
        const beforeRetry = snapshotTree(state.root);
        assert.throws(
          () =>
            publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
              fixtureStartAdmissionArguments(state, preparation),
            ),
          /start decision was unavailable|explicit recovery/u,
        );
        assert.deepEqual(snapshotTree(state.root), beforeRetry);
      }
      assert.equal(existsSync(join(prepared.active, ".mutation-close-03")), false);
      assert.equal(
        readdirSync(prepared.active).some((name) =>
          name.includes("provider-effect-retired"),
        ),
        false,
      );
      assert.equal(run(state, "verify").status, 0);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("recovery completes the exact suffix after a durable conclusive delivery", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    await crashLiveProviderEffectAt(state, {
      action: "conclusive",
      checkpoint: "after-delivery-result-link",
      input: {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      },
      releaseName: "conclusive-delivery-durable",
    });
    const recoveryArguments = {
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
    };
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    const completion = recoverColimaLiveProviderEffectForTest({
      ...recoveryArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(completion.event_kind, "completion");
    assert.equal(completion.event_sequence, 10);
    assert.equal(completion.evidence.variant, "attempted-effect-retired");
    const eventNames = readdirSync(prepared.active)
      .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
      .sort();
    assert.equal(eventNames.length, 11);
    const events = eventNames.map((name) => parse(join(prepared.active, name)));
    assert.deepEqual(
      events.map((event) => event.event_kind),
      [
        "start-authority",
        "start-attempt",
        "launch-edge",
        "delivery-result",
        "create-settlement",
        "cleanup-plan-page",
        "cleanup-plan",
        "cleanup-progress",
        "cleanup-settlement",
        "terminal-receipt",
        "completion",
      ],
    );
    const terminal = events[9];
    assert.deepEqual(
      readFileSync(join(prepared.active, receiptFileName(terminal.evidence.receipt))),
      canonicalBytes(terminal.evidence.receipt),
    );
    assert.equal(parse(join(prepared.active, ".mutation-close-03")).authority, "recovery");
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("only the newest same-frontier conclusive recoverer publishes the suffix", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "conclusive",
      checkpoint: "after-delivery-result",
      input: serialized,
      releaseName: "conclusive-same-frontier-owner",
    });
    const recoveryArguments = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    for (let sequence = 0; sequence < 2; sequence += 1) {
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      await crashLiveProviderEffectAt(state, {
        action: "recover",
        checkpoint: "after-recovery-claim",
        input: { ...serialized, confirmation },
        releaseName: `conclusive-same-frontier-recoverer-${sequence}`,
        signal: sequence === 0 ? "SIGTERM" : "SIGKILL",
      });
    }
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    const completion = recoverColimaLiveProviderEffectForTest({
      ...recoveryArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(completion.evidence.variant, "attempted-effect-retired");
    const claimNames = readdirSync(prepared.active)
      .filter((name) => /^\.mutation-recovery-03-\d{2}$/u.test(name))
      .sort();
    assert.deepEqual(claimNames, [
      ".mutation-recovery-03-00",
      ".mutation-recovery-03-01",
      ".mutation-recovery-03-02",
    ]);
    const claims = claimNames.map((name) => ({
      bytes: readFileSync(join(prepared.active, name)),
      value: parse(join(prepared.active, name)),
    }));
    for (const [index, claim] of claims.entries()) {
      assert.equal(claim.value.observed_evidence_stage, "attempt-fenced");
      assert.equal(
        claim.value.parent_sha256,
        index === 0 ? "0".repeat(64) : sha256(claims[index - 1].bytes),
      );
      assert.equal(
        claim.value.observed_evidence_head_sha256,
        claims[0].value.observed_evidence_head_sha256,
      );
      assert.equal(
        claim.value.observed_evidence_prefix_sha256,
        claims[0].value.observed_evidence_prefix_sha256,
      );
      assert.equal(
        claim.value.observed_residual_sha256,
        claims[0].value.observed_residual_sha256,
      );
      assert.equal(
        claim.value.source_head_sha256,
        claims[0].value.source_head_sha256,
      );
    }
    const slotAuthority = sha256(
      readFileSync(join(prepared.active, ".mutation-slot-03")),
    );
    const newestAuthority = sha256(claims.at(-1).bytes);
    const events = readdirSync(prepared.active)
      .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
      .sort()
      .map((name) => parse(join(prepared.active, name)));
    assert.equal(events.length, 11);
    assert.deepEqual(
      events.map((event) => event.publisher_authority_sha256),
      [
        ...Array(4).fill(slotAuthority),
        ...Array(7).fill(newestAuthority),
      ],
    );
    assert.equal(
      events[9].evidence.receipt.result.publisher_authority_sha256,
      newestAuthority,
    );
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, newestAuthority);
    assert.equal(
      close.operation_evidence_sha256,
      sha256(canonicalBytes(events[10])),
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a linked conclusive recovery close reconciles without new authority", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "conclusive",
      checkpoint: "after-completion",
      input: serialized,
      releaseName: "conclusive-linked-recovery-close-owner",
    });
    const recoveryArguments = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-recovery-close-link",
      input: { ...serialized, confirmation },
      releaseName: "conclusive-linked-recovery-close-recoverer",
      signal: "SIGTERM",
    });
    const stageNames = readdirSync(prepared.active).filter((name) =>
      /^\.mutation-stage-[0-9a-f]{32}$/u.test(name),
    );
    assert.equal(stageNames.length, 1);
    const stagePath = join(prepared.active, stageNames[0]);
    const closePath = join(prepared.active, ".mutation-close-03");
    const completionPath = join(
      prepared.active,
      ".provider-effect-event-03-010",
    );
    const terminal = parse(
      join(prepared.active, ".provider-effect-event-03-009"),
    );
    const receiptPath = join(
      prepared.active,
      receiptFileName(terminal.evidence.receipt),
    );
    const claimPath = join(
      prepared.active,
      ".mutation-recovery-03-00",
    );
    const artifacts = [closePath, completionPath, receiptPath, claimPath].map(
      (path) => {
        const metadata = lstatSync(path, { bigint: true });
        return {
          bytes: readFileSync(path),
          device: metadata.dev,
          inode: metadata.ino,
          path,
        };
      },
    );
    const stageIdentity = lstatSync(stagePath, { bigint: true });
    const closeIdentity = lstatSync(closePath, { bigint: true });
    assert.equal(stageIdentity.nlink, 2n);
    assert.equal(closeIdentity.nlink, 2n);
    assert.equal(stageIdentity.dev, closeIdentity.dev);
    assert.equal(stageIdentity.ino, closeIdentity.ino);
    assert.deepEqual(readFileSync(stagePath), readFileSync(closePath));
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        ),
      /no abandoned provider mutation was available/u,
    );
    const claimNamesBefore = readdirSync(prepared.active)
      .filter((name) => name.startsWith(".mutation-recovery-03-"))
      .sort();
    const completion =
      publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
    assert.equal(completion.evidence.variant, "attempted-effect-retired");
    assert.equal(existsSync(stagePath), false);
    for (const artifact of artifacts) {
      const metadata = lstatSync(artifact.path, { bigint: true });
      assert.equal(metadata.dev, artifact.device);
      assert.equal(metadata.ino, artifact.inode);
      assert.deepEqual(readFileSync(artifact.path), artifact.bytes);
    }
    assert.equal(lstatSync(closePath, { bigint: true }).nlink, 1n);
    assert.deepEqual(
      readdirSync(prepared.active)
        .filter((name) => name.startsWith(".mutation-recovery-03-"))
        .sort(),
      claimNamesBefore,
    );
    const close = parse(closePath);
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(readFileSync(claimPath)));
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("conclusive recovery preserves every semantic tail publication frontier", async () => {
  const expectedKinds = [
    "start-authority",
    "start-attempt",
    "launch-edge",
    "delivery-result",
    "create-settlement",
    "cleanup-plan-page",
    "cleanup-plan",
    "cleanup-progress",
    "cleanup-settlement",
    "terminal-receipt",
    "completion",
  ];
  const checkpoints = [
    "after-create-settlement",
    "after-cleanup-plan-page",
    "after-cleanup-plan-stage",
    "after-cleanup-plan-link",
    "after-cleanup-progress",
    "after-cleanup-settlement",
    "after-terminal-receipt",
    "after-terminal-receipt-publication-stage",
    "after-terminal-receipt-publication-link",
    "after-terminal-receipt-publication-directory-sync",
    "after-marker-unlink",
    "after-completion-link",
    "after-effect-close-link",
  ];
  for (const [index, checkpoint] of checkpoints.entries()) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        action: "conclusive",
        checkpoint,
        input: serialized,
        releaseName: `conclusive-tail-${checkpoint}`,
        signal: index % 2 === 0 ? "SIGKILL" : "SIGTERM",
      });
      const durablePrefix = readdirSync(prepared.active)
        .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
        .sort()
        .map((name) => {
          const path = join(prepared.active, name);
          const metadata = lstatSync(path, { bigint: true });
          return {
            bytes: readFileSync(path),
            device: metadata.dev,
            inode: metadata.ino,
            name,
          };
        });
      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      let completion;
      if (checkpoint === "after-effect-close-link") {
        completion =
          publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
            fixtureStartAdmissionArguments(state, preparation),
          );
      } else {
        const confirmation =
          liveProviderFixtureEffectRecoveryConfirmationForTest(
            recoveryArguments,
          );
        completion = recoverColimaLiveProviderEffectForTest({
          ...recoveryArguments,
          confirmation,
          testCheckpoint() {},
        });
      }
      assert.equal(completion.event_kind, "completion", checkpoint);
      assert.equal(completion.event_sequence, 10, checkpoint);
      assert.equal(
        completion.evidence.variant,
        "attempted-effect-retired",
        checkpoint,
      );
      for (const prefix of durablePrefix) {
        const path = join(prepared.active, prefix.name);
        const metadata = lstatSync(path, { bigint: true });
        assert.equal(metadata.dev, prefix.device, checkpoint);
        assert.equal(metadata.ino, prefix.inode, checkpoint);
        assert.deepEqual(readFileSync(path), prefix.bytes, checkpoint);
      }
      const eventNames = readdirSync(prepared.active)
        .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
        .sort();
      const events = eventNames.map((name) =>
        parse(join(prepared.active, name)),
      );
      assert.deepEqual(
        events.map((event) => event.event_kind),
        expectedKinds,
        checkpoint,
      );
      const slot = parse(join(prepared.active, ".mutation-slot-03"));
      const ownerAuthority = sha256(canonicalBytes(slot));
      const claimNames = readdirSync(prepared.active)
        .filter((name) => /^\.mutation-recovery-03-\d{2}$/u.test(name))
        .sort();
      if (checkpoint === "after-effect-close-link") {
        assert.deepEqual(claimNames, [], checkpoint);
        assert.deepEqual(
          events.map((event) => event.publisher_authority_sha256),
          Array(events.length).fill(ownerAuthority),
          checkpoint,
        );
      } else {
        assert.deepEqual(claimNames, [".mutation-recovery-03-00"], checkpoint);
        const recoveryAuthority = sha256(
          readFileSync(join(prepared.active, claimNames[0])),
        );
        const firstRecovered = events.findIndex(
          (event) => event.publisher_authority_sha256 === recoveryAuthority,
        );
        assert.equal(
          events.every(
            (event, eventIndex) =>
              event.publisher_authority_sha256 ===
              (firstRecovered === -1 || eventIndex < firstRecovered
                ? ownerAuthority
                : recoveryAuthority),
          ),
          true,
          checkpoint,
        );
        assert.equal(
          events.every((event) =>
            new Set([ownerAuthority, recoveryAuthority]).has(
              event.publisher_authority_sha256,
            ),
          ),
          true,
          checkpoint,
        );
      }
      const terminal = events[9];
      const receipt = terminal.evidence.receipt;
      assert.equal(
        receipt.result.publisher_authority_sha256,
        terminal.publisher_authority_sha256,
        checkpoint,
      );
      assert.deepEqual(
        readFileSync(join(prepared.active, receiptFileName(receipt))),
        canonicalBytes(receipt),
        checkpoint,
      );
      assert.equal(existsSync(join(prepared.active, ".receipt-publish")), false);
      assert.equal(
        readdirSync(prepared.active).some((name) =>
          name.startsWith(".mutation-stage-"),
        ),
        false,
        checkpoint,
      );
      assert.equal(
        existsSync(
          join(
            preparation.providerRoot,
            ".synveda-clean-engine-provider-reservation",
          ),
        ),
        false,
        checkpoint,
      );
      assertPrivate(
        join(prepared.active, ".provider-effect-witness-03"),
        0o600,
        false,
        1,
      );
      const close = parse(join(prepared.active, ".mutation-close-03"));
      assert.equal(
        close.authority,
        checkpoint === "after-effect-close-link" ? "owner" : "recovery",
        checkpoint,
      );
      assert.equal(
        close.operation_evidence_sha256,
        sha256(canonicalBytes(events[10])),
        checkpoint,
      );
      assert.equal(close.result_sequence, slot.source_sequence + 1, checkpoint);
      assert.equal(
        close.result_head_sha256,
        sha256(canonicalBytes(receipt)),
        checkpoint,
      );
      assert.equal(run(state, "verify").status, 0, checkpoint);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("receipt-first, marker-first, and mismatched conclusive retirement are refused", async () => {
  for (const testCase of [
    {
      checkpoint: "after-terminal-receipt-stage",
      name: "receipt-first",
    },
    {
      checkpoint: "after-terminal-receipt",
      name: "marker-first",
    },
    {
      checkpoint:
        "after-terminal-receipt-publication-stage-directory-sync",
      name: "receipt-mismatch",
    },
  ]) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        action: "conclusive",
        checkpoint: testCase.checkpoint,
        input: serialized,
        releaseName: `conclusive-order-${testCase.name}`,
      });
      const marker = join(
        preparation.providerRoot,
        ".synveda-clean-engine-provider-reservation",
      );
      let expectedError;
      let restore = () => {};
      if (testCase.name === "receipt-first") {
        const stageNames = readdirSync(prepared.active).filter((name) =>
          name.startsWith(".mutation-stage-"),
        );
        assert.equal(stageNames.length, 1);
        const stagedEvent = parse(join(prepared.active, stageNames[0]));
        assert.equal(stagedEvent.event_kind, "terminal-receipt");
        assert.equal(
          existsSync(join(prepared.active, ".provider-effect-event-03-009")),
          false,
        );
        const receipt = stagedEvent.evidence.receipt;
        const receiptPath = join(prepared.active, receiptFileName(receipt));
        writeFileSync(receiptPath, canonicalBytes(receipt), {
          flag: "wx",
          mode: 0o600,
        });
        fsyncDirectoryForTest(prepared.active);
        expectedError =
          "clean-engine: open mutation slot did not cover the receipt head\n";
      } else if (testCase.name === "marker-first") {
        const terminal = parse(
          join(prepared.active, ".provider-effect-event-03-009"),
        );
        assert.equal(
          existsSync(
            join(prepared.active, receiptFileName(terminal.evidence.receipt)),
          ),
          false,
        );
        unlinkSync(marker);
        fsyncDirectoryForTest(preparation.providerRoot);
        expectedError =
          "clean-engine: live provider effect retired receipt was refused\n";
      } else {
        const terminal = parse(
          join(prepared.active, ".provider-effect-event-03-009"),
        );
        const receiptPath = join(
          prepared.active,
          receiptFileName(terminal.evidence.receipt),
        );
        const original = readFileSync(receiptPath);
        const changed = JSON.parse(original.toString("utf8"));
        changed.result.publisher_authority_sha256 = "f".repeat(64);
        assert.notDeepEqual(canonicalBytes(changed), original);
        writeFileSync(receiptPath, canonicalBytes(changed), { mode: 0o600 });
        restore = () => writeFileSync(receiptPath, original, { mode: 0o600 });
        expectedError =
          "clean-engine: open mutation slot did not cover the receipt head\n";
      }
      const invalid = snapshotTree(state.root);
      for (const action of ["status", "verify"]) {
        const refused = run(state, action);
        assert.equal(refused.status, 78, testCase.name);
        assert.equal(refused.stderr, expectedError, testCase.name);
        assert.deepEqual(snapshotTree(state.root), invalid, testCase.name);
      }
      assert.equal(
        existsSync(join(prepared.active, ".mutation-close-03")),
        false,
        testCase.name,
      );
      if (testCase.name === "receipt-mismatch") {
        restore();
        restore = () => {};
        assert.equal(run(state, "status").status, 0);
        const recoveryArguments = {
          ...serialized,
          repoRoot: state.repo,
          stateBase: state.state,
        };
        const confirmation =
          liveProviderFixtureEffectRecoveryConfirmationForTest(
            recoveryArguments,
          );
        const completion = recoverColimaLiveProviderEffectForTest({
          ...recoveryArguments,
          confirmation,
          testCheckpoint() {},
        });
        assert.equal(completion.evidence.variant, "attempted-effect-retired");
        assert.equal(run(state, "verify").status, 0);
      } else {
        assert.equal(existsSync(marker), testCase.name === "receipt-first");
      }
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("conclusive recovery preserves pending receipt inode and embedded bytes", async () => {
  for (const [index, checkpoint] of [
    "after-terminal-receipt-publication-stage",
    "after-terminal-receipt-publication-directory-sync",
  ].entries()) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        action: "conclusive",
        checkpoint,
        input: serialized,
        releaseName: `conclusive-pending-receipt-${index}`,
        signal: index === 0 ? "SIGKILL" : "SIGTERM",
      });
      const terminalPath = join(
        prepared.active,
        ".provider-effect-event-03-009",
      );
      const terminal = parse(terminalPath);
      const expectedBytes = canonicalBytes(terminal.evidence.receipt);
      const stagePath = join(prepared.active, ".receipt-publish");
      const receiptPath = join(
        prepared.active,
        receiptFileName(terminal.evidence.receipt),
      );
      const stageIdentity = lstatSync(stagePath, { bigint: true });
      assert.equal(stageIdentity.nlink, BigInt(index + 1), checkpoint);
      assert.deepEqual(readFileSync(stagePath), expectedBytes, checkpoint);
      if (index === 0) {
        assert.equal(existsSync(receiptPath), false, checkpoint);
      } else {
        const linked = lstatSync(receiptPath, { bigint: true });
        assert.equal(linked.dev, stageIdentity.dev, checkpoint);
        assert.equal(linked.ino, stageIdentity.ino, checkpoint);
        assert.deepEqual(readFileSync(receiptPath), expectedBytes, checkpoint);
      }
      const prefix = readdirSync(prepared.active)
        .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
        .sort()
        .map((name) => {
          const path = join(prepared.active, name);
          const metadata = lstatSync(path, { bigint: true });
          return {
            bytes: readFileSync(path),
            device: metadata.dev,
            inode: metadata.ino,
            name,
          };
        });
      const beforeObservation = snapshotTree(state.root);
      assert.equal(run(state, "status").status, 0, checkpoint);
      assert.equal(run(state, "verify").status, 0, checkpoint);
      assert.deepEqual(snapshotTree(state.root), beforeObservation, checkpoint);
      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      const completion = recoverColimaLiveProviderEffectForTest({
        ...recoveryArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.equal(completion.evidence.variant, "attempted-effect-retired");
      assert.equal(existsSync(stagePath), false, checkpoint);
      const receiptIdentity = lstatSync(receiptPath, { bigint: true });
      assert.equal(receiptIdentity.dev, stageIdentity.dev, checkpoint);
      assert.equal(receiptIdentity.ino, stageIdentity.ino, checkpoint);
      assert.equal(receiptIdentity.nlink, 1n, checkpoint);
      assert.deepEqual(readFileSync(receiptPath), expectedBytes, checkpoint);
      for (const event of prefix) {
        const path = join(prepared.active, event.name);
        const metadata = lstatSync(path, { bigint: true });
        assert.equal(metadata.dev, event.device, checkpoint);
        assert.equal(metadata.ino, event.inode, checkpoint);
        assert.deepEqual(readFileSync(path), event.bytes, checkpoint);
      }
      assert.equal(
        parse(join(prepared.active, ".mutation-close-03")).authority,
        "recovery",
        checkpoint,
      );
      assert.equal(run(state, "verify").status, 0, checkpoint);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("conclusive recovery refuses a same-byte pending receipt inode replacement", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "conclusive",
      checkpoint: "after-terminal-receipt-publication-stage",
      input: serialized,
      releaseName: "conclusive-pending-receipt-replacement-owner",
    });
    const terminal = parse(
      join(prepared.active, ".provider-effect-event-03-009"),
    );
    const stagePath = join(prepared.active, ".receipt-publish");
    const receiptPath = join(
      prepared.active,
      receiptFileName(terminal.evidence.receipt),
    );
    const expectedBytes = readFileSync(stagePath);
    const originalIdentity = lstatSync(stagePath, { bigint: true });
    const recoveryArguments = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    const displacedPath = join(
      state.root,
      "original-conclusive-pending-receipt",
    );
    const result = await releaseLiveProviderEffectCheckpoint(state, {
      action: "recover",
      checkpoint: "after-terminal-receipt-reconciliation-observation",
      input: { ...serialized, confirmation },
      mutate() {
        renameSync(stagePath, displacedPath);
        writeFileSync(stagePath, expectedBytes, {
          flag: "wx",
          mode: 0o600,
        });
        fsyncDirectoryForTest(prepared.active);
      },
      releaseName: "conclusive-pending-receipt-replacement-recovery",
    });
    assert.equal(result.signal, null);
    assert.equal(result.status, 73);
    assert.match(
      result.stderr,
      /pending receipt publication identity changed/u,
    );
    const replacementIdentity = lstatSync(stagePath, { bigint: true });
    assert.notEqual(replacementIdentity.ino, originalIdentity.ino);
    assert.deepEqual(readFileSync(stagePath), expectedBytes);
    assert.equal(existsSync(receiptPath), false);
    assert.equal(
      existsSync(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
      ),
      true,
    );
    assert.equal(
      existsSync(join(prepared.active, ".mutation-close-03")),
      false,
    );
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("recovery binds conclusive delivery to the exact canary and private outer identity", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    await crashLiveProviderEffectAt(state, {
      action: "conclusive",
      checkpoint: "after-delivery-result",
      input: {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      },
      releaseName: "conclusive-delivery-binding",
    });
    const edgePath = join(
      prepared.active,
      ".provider-effect-event-03-002",
    );
    const deliveryPath = join(
      prepared.active,
      ".provider-effect-event-03-003",
    );
    const originalEdgeBytes = readFileSync(edgePath);
    const originalDeliveryBytes = readFileSync(deliveryPath);
    const originalDelivery = JSON.parse(originalDeliveryBytes);
    writeFileSync(
      deliveryPath,
      canonicalBytes({
        ...originalDelivery,
        evidence: {
          ...originalDelivery.evidence,
          observation_sha256: "f".repeat(64),
        },
      }),
      { mode: 0o600 },
    );
    const invalidCanary = run(state, "status");
    assert.equal(invalidCanary.status, 70);
    assert.match(invalidCanary.stderr, /conclusive canary evidence was refused/u);
    writeFileSync(deliveryPath, originalDeliveryBytes, { mode: 0o600 });
    assert.equal(run(state, "status").status, 0);

    const changedEdge = {
      ...JSON.parse(originalEdgeBytes),
      evidence: {
        ...JSON.parse(originalEdgeBytes).evidence,
        child_node_identity_sha256: "e".repeat(64),
      },
    };
    const changedEdgeSha256 = sha256(canonicalBytes(changedEdge));
    const changedDelivery = {
      ...originalDelivery,
      parent_sha256: changedEdgeSha256,
      evidence: {
        ...originalDelivery.evidence,
        outer_launch_edge_sha256: changedEdgeSha256,
      },
    };
    const { observation_sha256: _observationSha256, ...fixedResult } =
      changedDelivery.evidence;
    changedDelivery.evidence.observation_sha256 =
      fixtureEffectAdapterObservation(
        {
          adapter_contract_sha256:
            FIXTURE_EFFECT_ADAPTER_CONTRACT_SHA256,
          attempt_event_sha256:
            changedDelivery.evidence.attempt_event_sha256,
          driver_contract_sha256:
            changedDelivery.evidence.driver_contract_sha256,
          fixture_id: changedDelivery.fixture_id,
          operation_contract_sha256:
            changedDelivery.operation_contract_sha256,
          operation_kind: changedDelivery.operation_kind,
          outer_launch_edge_sha256: changedEdgeSha256,
          planned_start_attempt_sha256: parse(
            join(prepared.active, ".mutation-slot-03"),
          ).operation_plan.planned_start_attempt_sha256,
        },
        {
          ...fixedResult,
          outer_launch_edge_sha256: changedEdgeSha256,
        },
      );
    writeFileSync(edgePath, canonicalBytes(changedEdge), { mode: 0o600 });
    writeFileSync(deliveryPath, canonicalBytes(changedDelivery), {
      mode: 0o600,
    });
    assert.equal(run(state, "status").status, 0);
    const recoveryArguments = {
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
    };
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        ),
      /outer identity was refused/u,
    );
    writeFileSync(edgePath, originalEdgeBytes, { mode: 0o600 });
    writeFileSync(deliveryPath, originalDeliveryBytes, { mode: 0o600 });
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    const completion = recoverColimaLiveProviderEffectForTest({
      ...recoveryArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(completion.evidence.variant, "attempted-effect-retired");
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("physical drift cannot cross event, marker-retirement, or close publication", async () => {
  for (const [checkpoint, driftKind, markerPresent] of [
    ["after-cleanup-settlement-stage", "source", true],
    ["after-cleanup-progress-stage", "observation-component", true],
    [
      "before-cleanup-temporary-namespace-parent-fsync",
      "namespace-swap",
      true,
    ],
    [
      "after-terminal-receipt-publication-stage-directory-sync",
      "namespace",
      true,
    ],
    ["after-terminal-receipt-publication-stage", "source", true],
    [
      "before-marker-retirement-provider-root-fsync",
      "provider-root-swap",
      false,
    ],
    ["after-completion", "source", false],
  ]) {
    const state = fixture();
    let preparation;
    let restore = () => {};
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const result = await releaseLiveProviderEffectCheckpoint(state, {
        checkpoint,
        input: {
          observation: preparation.observation,
          observationInput: preparation.input,
          requirements: preparation.requirements,
        },
        mutate() {
          if (driftKind === "source") {
            const sourcePath = join(state.repo, "source.txt");
            writeFileSync(sourcePath, "source drift during publication\n", {
              mode: 0o600,
            });
            restore = () => {
              writeFileSync(sourcePath, "fixture source\n", { mode: 0o600 });
            };
          } else if (driftKind === "observation-component") {
            const componentPath =
              preparation.input.component_paths["docker-cli-binary"];
            const original = readFileSync(componentPath);
            const changed = Buffer.from(original);
            changed[0] ^= 0xff;
            chmodSync(componentPath, 0o700);
            writeFileSync(componentPath, changed);
            chmodSync(componentPath, 0o500);
            restore = () => {
              chmodSync(componentPath, 0o700);
              writeFileSync(componentPath, original);
              chmodSync(componentPath, 0o500);
            };
          } else if (driftKind === "namespace") {
            const driftPath = join(
              preparation.input.environment.COLIMA_HOME,
              "late-publication-drift",
            );
            writePrivateColimaLiveFixtureFile(
              driftPath,
              Buffer.from("late namespace drift\n", "utf8"),
              0o600,
            );
            restore = () => unlinkSync(driftPath);
          } else {
            const originalPath =
              driftKind === "namespace-swap"
                ? preparation.input.environment.TMPDIR
                : preparation.providerRoot;
            const displacedPath = `${originalPath}-displaced`;
            renameSync(originalPath, displacedPath);
            symlinkSync(displacedPath, originalPath);
            restore = () => {
              unlinkSync(originalPath);
              renameSync(displacedPath, originalPath);
            };
          }
        },
        releaseName: `conclusive-drift-${driftKind}-${checkpoint}`,
      });
      assert.equal(result.signal, null);
      assert.equal(
        result.status,
        73,
      );
      assert.match(
        result.stderr,
        /source changed|tail authority changed|root observation changed|root observation was refused|blueprint changed|identity changed/u,
      );
      const marker = join(
        preparation.providerRoot,
        ".synveda-clean-engine-provider-reservation",
      );
      assert.equal(existsSync(marker), markerPresent, checkpoint);
      if (checkpoint === "after-terminal-receipt-publication-stage") {
        const terminal = parse(
          join(prepared.active, ".provider-effect-event-03-009"),
        );
        assert.equal(
          existsSync(
            join(prepared.active, receiptFileName(terminal.evidence.receipt)),
          ),
          false,
          checkpoint,
        );
        assert.equal(
          existsSync(join(prepared.active, ".receipt-publish")),
          true,
          checkpoint,
        );
      }
      assert.equal(
        existsSync(join(prepared.active, ".mutation-close-03")),
        false,
      );
      restore();
      restore = () => {};
      const recoveryArguments = {
        observation: preparation.observation,
        observationInput: preparation.input,
        repoRoot: state.repo,
        requirements: preparation.requirements,
        stateBase: state.state,
      };
      if (driftKind === "namespace" || driftKind === "namespace-swap") {
        assert.throws(
          () =>
            liveProviderFixtureEffectRecoveryConfirmationForTest(
              recoveryArguments,
            ),
          /root observation changed/u,
        );
        assert.equal(existsSync(marker), true, checkpoint);
        assert.equal(
          existsSync(join(prepared.active, ".mutation-close-03")),
          false,
        );
        assert.equal(run(state, "verify").status, 0, checkpoint);
        continue;
      }
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      const completion = recoverColimaLiveProviderEffectForTest({
        ...recoveryArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.equal(completion.evidence.variant, "attempted-effect-retired");
      assert.equal(run(state, "verify").status, 0);
    } finally {
      restore();
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("conclusive recovery revalidates every admitted namespace", async () => {
  for (const environmentKey of [
    "TMPDIR",
    "HOME",
    "LIMA_HOME",
    "DOCKER_CONFIG",
    "COLIMA_HOME",
    "COLIMA_CACHE_HOME",
  ]) {
    const state = fixture();
    let preparation;
    let driftPath;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        action: "conclusive",
        checkpoint: "after-delivery-result",
        input: serialized,
        releaseName: `conclusive-namespace-${environmentKey.toLowerCase()}`,
      });
      driftPath = join(
        preparation.input.environment[environmentKey],
        `recovery-drift-${environmentKey.toLowerCase()}`,
      );
      writePrivateColimaLiveFixtureFile(
        driftPath,
        Buffer.from("namespace recovery drift\n", "utf8"),
        0o600,
      );
      const stateBefore = snapshotTree(state.root);
      assert.throws(
        () =>
          liveProviderFixtureEffectRecoveryConfirmationForTest({
            ...serialized,
            repoRoot: state.repo,
            stateBase: state.state,
          }),
        /root observation changed/u,
        environmentKey,
      );
      assert.deepEqual(snapshotTree(state.root), stateBefore, environmentKey);
      assert.equal(
        readdirSync(prepared.active).filter((name) =>
          /^\.provider-effect-event-03-\d{3}$/u.test(name),
        ).length,
        4,
        environmentKey,
      );
      assert.equal(
        readdirSync(prepared.active).some((name) =>
          name.startsWith(".mutation-recovery-03-"),
        ),
        false,
        environmentKey,
      );
      assert.equal(
        existsSync(join(prepared.active, ".mutation-close-03")),
        false,
        environmentKey,
      );
      assertPrivate(
        join(prepared.active, ".provider-effect-witness-03"),
        0o600,
        false,
        2,
      );
      assertPrivate(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
        0o600,
        false,
        2,
      );
      unlinkSync(driftPath);
      driftPath = undefined;
      assert.equal(run(state, "verify").status, 0, environmentKey);
    } finally {
      if (driftPath !== undefined) unlinkSync(driftPath);
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("fixture provider effect recovery closes a dead pre-attempt owner", async () => {
  for (const [index, checkpoint] of [
    "after-effect-slot",
    "after-witness-stage-create",
  ].entries()) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        checkpoint,
        input: serialized,
        releaseName: `effect-owner-${checkpoint}`,
        signal: index % 2 === 0 ? "SIGKILL" : "SIGTERM",
      });
      const confirmationArguments = {
        observation: preparation.observation,
        observationInput: preparation.input,
        repoRoot: state.repo,
        requirements: preparation.requirements,
        stateBase: state.state,
      };
      assert.throws(
        () => providerRecoveryConfirmationForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
        /fixture provider effect requires its physical recovery confirmation/u,
      );
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          confirmationArguments,
        );
      const recovered = recoverColimaLiveProviderEffectForTest({
        ...confirmationArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.equal(recovered.phase, "plan");
      const close = parse(join(prepared.active, ".mutation-close-03"));
      assert.equal(close.disposition, "aborted-before-effect");
      assert.equal(close.operation_evidence_sha256, "0".repeat(64));
      assert.equal(
        readdirSync(prepared.active).some((name) =>
          name.startsWith(".provider-effect-stage-"),
        ),
        false,
      );
      const completion = publishColimaLiveProviderEffectPreAttemptForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      assert.equal(completion.slot_sequence, 4);
      assert.equal(run(state, "verify").status, 0);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("fixture provider effect recovery classifies and retires a partial stage", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      checkpoint: "after-witness-stage-create",
      input: serialized,
      releaseName: "effect-partial-stage-owner",
    });
    const stageName = readdirSync(prepared.active).find((name) =>
      name.startsWith(".provider-effect-stage-03-"),
    );
    assert.notEqual(stageName, undefined);
    writeFileSync(join(prepared.active, stageName), '{"schema":', {
      mode: 0o600,
    });
    const base = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const beforeConfirmation = snapshotTree(state.root);
    let confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    assert.deepEqual(snapshotTree(state.root), beforeConfirmation);
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-recovery-claim",
      input: { ...serialized, confirmation },
      releaseName: "effect-partial-stage-recovery",
      signal: "SIGTERM",
    });
    const firstClaim = parse(
      join(prepared.active, ".mutation-recovery-03-00"),
    );
    assert.equal(firstClaim.observed_evidence_stage, "stage-initializing");
    assert.equal(firstClaim.observed_effect_disposition, "not-reached");
    assert.match(firstClaim.observed_evidence_head_sha256, /^[0-9a-f]{64}$/u);
    assert.match(
      firstClaim.observed_evidence_prefix_sha256,
      /^[0-9a-f]{64}$/u,
    );
    assert.notEqual(firstClaim.observed_evidence_head_sha256, "0".repeat(64));
    assert.notEqual(
      firstClaim.observed_evidence_prefix_sha256,
      "0".repeat(64),
    );
    confirmation = liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    const recovered = recoverColimaLiveProviderEffectForTest({
      ...base,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(recovered.phase, "plan");
    assert.equal(existsSync(join(prepared.active, stageName)), false);
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture provider effect recovery claims bind the surviving physical stage", async () => {
  const frontiers = [
    {
      checkpoint: "after-witness-stage-create",
      localLinks: 1,
      markerPresent: false,
      recoveryStage: "stage-initializing",
    },
    {
      checkpoint: "after-witness-stage",
      localLinks: 1,
      markerPresent: false,
      recoveryStage: "stage-only",
    },
    {
      checkpoint: "after-marker-link",
      localLinks: 2,
      markerPresent: true,
      recoveryStage: "marker-linked",
    },
  ];
  for (const frontier of frontiers) {
    for (const tamperedHeadSha256 of ["a".repeat(64), "0".repeat(64)]) {
      const state = fixture();
      let preparation;
      try {
        const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
        preparation = prepared.preparation;
        const serialized = {
          observation: preparation.observation,
          observationInput: preparation.input,
          requirements: preparation.requirements,
        };
        const suffix =
          tamperedHeadSha256 === "0".repeat(64) ? "zero" : "foreign";
        await crashLiveProviderEffectAt(state, {
          checkpoint: frontier.checkpoint,
          input: serialized,
          releaseName: `effect-stage-binding-owner-${frontier.recoveryStage}-${suffix}`,
        });
        const stageName = readdirSync(prepared.active).find((name) =>
          name.startsWith(".provider-effect-stage-03-"),
        );
        assert.notEqual(stageName, undefined);
        const stagePath = join(prepared.active, stageName);
        const stageBytes = readFileSync(stagePath);
        const stageMetadata = lstatSync(stagePath, { bigint: true });
        assert.equal(stageMetadata.nlink, BigInt(frontier.localLinks));
        const markerPath = join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        );
        assert.equal(existsSync(markerPath), frontier.markerPresent);
        if (frontier.markerPresent) {
          const markerBytes = readFileSync(markerPath);
          const markerMetadata = lstatSync(markerPath, { bigint: true });
          assert.equal(markerMetadata.nlink, 2n);
          assert.equal(markerMetadata.dev, stageMetadata.dev);
          assert.equal(markerMetadata.ino, stageMetadata.ino);
          assert.deepEqual(markerBytes, stageBytes);
        }
        const expectedPhysicalSha256 =
          frontier.recoveryStage === "stage-initializing"
            ? sha256(
                canonicalBytes({
                  content_sha256: sha256(stageBytes),
                  device: String(stageMetadata.dev),
                  inode: String(stageMetadata.ino),
                  mode: (stageMetadata.mode & 0o7777n)
                    .toString(8)
                    .padStart(4, "0"),
                  name: stageName,
                  schema:
                    "synveda.clean-engine.provider-effect-initializing-stage.v1",
                  size: String(stageMetadata.size),
                  slot_sequence: 3,
                  uid: String(stageMetadata.uid),
                }),
              )
            : sha256(stageBytes);
        const recoveryArguments = {
          ...serialized,
          repoRoot: state.repo,
          stateBase: state.state,
        };
        const confirmation =
          liveProviderFixtureEffectRecoveryConfirmationForTest(
            recoveryArguments,
          );
        await crashLiveProviderEffectAt(state, {
          action: "recover",
          checkpoint: "after-recovery-claim",
          input: { ...serialized, confirmation },
          releaseName: `effect-stage-binding-recovery-${frontier.recoveryStage}-${suffix}`,
          signal: "SIGTERM",
        });
        const claimPath = join(
          prepared.active,
          ".mutation-recovery-03-00",
        );
        const claim = parse(claimPath);
        assert.equal(
          claim.observed_evidence_stage,
          frontier.recoveryStage,
        );
        assert.equal(
          claim.observed_evidence_head_sha256,
          expectedPhysicalSha256,
        );
        const expectedTopologySha256 = sha256(
          canonicalBytes({
            event_count: 0,
            event_head_sha256: "0".repeat(64),
            evidence_stage: frontier.recoveryStage,
            local_links: frontier.localLinks,
            marker_present: frontier.markerPresent,
            slot_sequence: 3,
            witness_sha256: expectedPhysicalSha256,
          }),
        );
        assert.equal(
          claim.observed_evidence_prefix_sha256,
          expectedTopologySha256,
        );
        assert.equal(claim.observed_residual_sha256, expectedTopologySha256);
        const untamperedState = snapshotTree(state.root);
        const untamperedPreparation = snapshotTree(preparation.root);
        assert.equal(run(state, "status").status, 0);
        assert.equal(run(state, "verify").status, 0);
        assert.deepEqual(snapshotTree(state.root), untamperedState);
        assert.deepEqual(
          snapshotTree(preparation.root),
          untamperedPreparation,
        );
        claim.observed_evidence_head_sha256 = tamperedHeadSha256;
        const topologySha256 = sha256(
          canonicalBytes({
            event_count: 0,
            event_head_sha256: "0".repeat(64),
            evidence_stage: frontier.recoveryStage,
            local_links: frontier.localLinks,
            marker_present: frontier.markerPresent,
            slot_sequence: 3,
            witness_sha256: tamperedHeadSha256,
          }),
        );
        claim.observed_evidence_prefix_sha256 = topologySha256;
        claim.observed_residual_sha256 = topologySha256;
        writeFileSync(claimPath, canonicalBytes(claim), { mode: 0o600 });
        const stateBefore = snapshotTree(state.root);
        const preparationBefore = snapshotTree(preparation.root);
        for (const action of ["status", "verify"]) {
          const refused = run(state, action);
          assert.equal(refused.status, 78, `${frontier.recoveryStage}:${suffix}`);
          assert.equal(
            refused.stderr,
            tamperedHeadSha256 === "0".repeat(64)
              ? "clean-engine: live provider effect recovery history was refused\n"
              : "clean-engine: live provider effect recovery event frontier was refused\n",
          );
        }
        assert.deepEqual(snapshotTree(state.root), stateBefore);
        assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
      } finally {
        if (preparation !== undefined) {
          rmSync(preparation.root, { recursive: true, force: true });
        }
        rmSync(state.root, { recursive: true, force: true });
      }
    }
  }
});

test("fixture provider effect recovery accepts a skipped transitive frontier", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      checkpoint: "after-marker-link",
      input: serialized,
      releaseName: "effect-marker-linked-owner",
      signal: "SIGTERM",
    });
    const base = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    let confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-witness-stage-unlink",
      input: { ...serialized, confirmation },
      releaseName: "effect-transitive-recovery",
    });
    const firstClaim = parse(
      join(prepared.active, ".mutation-recovery-03-00"),
    );
    assert.equal(firstClaim.observed_evidence_stage, "marker-linked");
    confirmation = liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    const completion = recoverColimaLiveProviderEffectForTest({
      ...base,
      confirmation,
      testCheckpoint() {},
    });
    const secondClaim = parse(
      join(prepared.active, ".mutation-recovery-03-01"),
    );
    assert.equal(secondClaim.observed_evidence_stage, "marker-held");
    assert.equal(
      secondClaim.parent_sha256,
      sha256(canonicalBytes(firstClaim)),
    );
    assert.equal(completion.event_kind, "completion");
    assert.equal(
      completion.publisher_authority_sha256,
      sha256(canonicalBytes(secondClaim)),
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture provider effect recovery persists the derived retired-stage frontier", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      checkpoint: "after-witness-stage-create",
      input: serialized,
      releaseName: "effect-derived-stage-owner",
      signal: "SIGTERM",
    });
    const stageName = readdirSync(prepared.active).find((name) =>
      name.startsWith(".provider-effect-stage-03-"),
    );
    assert.notEqual(stageName, undefined);
    const stagePath = join(prepared.active, stageName);
    const stageBytes = readFileSync(stagePath);
    const stageIdentity = lstatSync(stagePath, { bigint: true });
    const runIdentity = lstatSync(prepared.active, { bigint: true });
    assert.equal(stageIdentity.isFile(), true);
    assert.equal(stageBytes.length, 0);
    assert.equal(stageIdentity.size, 0n);
    assert.equal(stageIdentity.nlink, 1n);
    assert.equal(stageIdentity.dev, runIdentity.dev);
    assert.equal(stageIdentity.uid, runIdentity.uid);
    assert.equal(stageIdentity.mode & 0o7777n, 0o600n);
    const physicalStageSha256 = sha256(
      canonicalBytes({
        content_sha256: sha256(stageBytes),
        device: String(stageIdentity.dev),
        inode: String(stageIdentity.ino),
        mode: (stageIdentity.mode & 0o7777n)
          .toString(8)
          .padStart(4, "0"),
        name: stageName,
        schema: "synveda.clean-engine.provider-effect-initializing-stage.v1",
        size: String(stageIdentity.size),
        slot_sequence: 3,
        uid: String(stageIdentity.uid),
      }),
    );
    const recoveryArguments = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    let confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-witness-stage-unlink",
      input: { ...serialized, confirmation },
      releaseName: "effect-derived-stage-first-recoverer",
      signal: "SIGKILL",
    });
    assert.equal(existsSync(stagePath), false);
    const firstClaimPath = join(
      prepared.active,
      ".mutation-recovery-03-00",
    );
    const firstClaimBytes = readFileSync(firstClaimPath);
    const firstClaim = JSON.parse(firstClaimBytes.toString("utf8"));
    const firstTopologySha256 = sha256(
      canonicalBytes({
        event_count: 0,
        event_head_sha256: "0".repeat(64),
        evidence_stage: "stage-initializing",
        local_links: 1,
        marker_present: false,
        slot_sequence: 3,
        witness_sha256: physicalStageSha256,
      }),
    );
    assert.equal(firstClaim.observed_evidence_stage, "stage-initializing");
    assert.equal(
      firstClaim.observed_evidence_head_sha256,
      physicalStageSha256,
    );
    assert.equal(
      firstClaim.observed_evidence_prefix_sha256,
      firstTopologySha256,
    );
    assert.equal(firstClaim.observed_residual_sha256, firstTopologySha256);

    confirmation = liveProviderFixtureEffectRecoveryConfirmationForTest(
      recoveryArguments,
    );
    const recovered = recoverColimaLiveProviderEffectForTest({
      ...recoveryArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.deepEqual(recovered, parse(join(prepared.active, "00-plan.json")));
    const secondClaimPath = join(
      prepared.active,
      ".mutation-recovery-03-01",
    );
    const secondClaimBytes = readFileSync(secondClaimPath);
    const secondClaim = JSON.parse(secondClaimBytes.toString("utf8"));
    const secondTopologySha256 = sha256(
      canonicalBytes({
        event_count: 0,
        event_head_sha256: "0".repeat(64),
        evidence_stage: "stage-retired-before-effect",
        local_links: 0,
        marker_present: false,
        slot_sequence: 3,
        witness_sha256: physicalStageSha256,
      }),
    );
    assert.equal(secondClaim.parent_sha256, sha256(firstClaimBytes));
    assert.equal(
      secondClaim.observed_effect_disposition,
      "not-reached",
    );
    assert.equal(
      secondClaim.observed_evidence_stage,
      "stage-retired-before-effect",
    );
    assert.equal(
      secondClaim.observed_evidence_head_sha256,
      physicalStageSha256,
    );
    assert.equal(
      secondClaim.observed_evidence_prefix_sha256,
      secondTopologySha256,
    );
    assert.equal(secondClaim.observed_residual_sha256, secondTopologySha256);
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, sha256(secondClaimBytes));
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture provider effect recovery resumes every physical pre-attempt frontier", async () => {
  const checkpoints = [
    "after-witness-stage",
    "after-marker-link",
    "after-state-witness-link",
    "after-witness-stage-unlink",
    "after-start-authority",
    "after-marker-unlink",
    "after-completion",
  ];
  for (const [index, checkpoint] of checkpoints.entries()) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        checkpoint,
        input: serialized,
        releaseName: `effect-frontier-${checkpoint}`,
        signal: index % 2 === 0 ? "SIGTERM" : "SIGKILL",
      });
      let witnessLinkedBytes;
      if (checkpoint === "after-state-witness-link") {
        const stageName = readdirSync(prepared.active).find((name) =>
          name.startsWith(".provider-effect-stage-03-"),
        );
        assert.notEqual(stageName, undefined);
        const stagePath = join(prepared.active, stageName);
        const witnessPath = join(
          prepared.active,
          ".provider-effect-witness-03",
        );
        const markerPath = join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        );
        const stageIdentity = lstatSync(stagePath, { bigint: true });
        const witnessIdentity = lstatSync(witnessPath, { bigint: true });
        const markerIdentity = lstatSync(markerPath, { bigint: true });
        for (const identity of [
          stageIdentity,
          witnessIdentity,
          markerIdentity,
        ]) {
          assert.equal(identity.nlink, 3n);
          assert.equal(identity.dev, stageIdentity.dev);
          assert.equal(identity.ino, stageIdentity.ino);
        }
        witnessLinkedBytes = readFileSync(witnessPath);
        assert.deepEqual(readFileSync(stagePath), witnessLinkedBytes);
        assert.deepEqual(readFileSync(markerPath), witnessLinkedBytes);
      }
      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      const recovered = recoverColimaLiveProviderEffectForTest({
        ...recoveryArguments,
        confirmation,
        testCheckpoint() {},
      });
      if (witnessLinkedBytes !== undefined) {
        const claim = parse(
          join(prepared.active, ".mutation-recovery-03-00"),
        );
        const witnessSha256 = sha256(witnessLinkedBytes);
        const topologySha256 = sha256(
          canonicalBytes({
            event_count: 0,
            event_head_sha256: "0".repeat(64),
            evidence_stage: "witness-linked",
            local_links: 3,
            marker_present: true,
            slot_sequence: 3,
            witness_sha256: witnessSha256,
          }),
        );
        assert.equal(claim.observed_effect_disposition, "pending");
        assert.equal(claim.observed_evidence_stage, "witness-linked");
        assert.equal(claim.observed_evidence_head_sha256, witnessSha256);
        assert.equal(claim.observed_evidence_prefix_sha256, topologySha256);
        assert.equal(claim.observed_residual_sha256, topologySha256);
      }
      if (checkpoint === "after-witness-stage") {
        assert.equal(recovered.phase, "plan", checkpoint);
        const close = parse(join(prepared.active, ".mutation-close-03"));
        assert.equal(close.disposition, "aborted-before-effect", checkpoint);
        const completion = publishColimaLiveProviderEffectPreAttemptForTest(
          fixtureStartAdmissionArguments(state, preparation),
        );
        assert.equal(completion.slot_sequence, 4, checkpoint);
        assert.equal(run(state, "verify").status, 0, checkpoint);
        continue;
      }
      assert.equal(recovered.event_kind, "completion", checkpoint);
      assert.equal(
        recovered.evidence.variant,
        "pre-attempt-retired-zero-receipt",
        checkpoint,
      );
      assert.equal(recovered.slot_sequence, 3, checkpoint);
      const close = parse(join(prepared.active, ".mutation-close-03"));
      assert.equal(close.disposition, "completed", checkpoint);
      assert.equal(
        close.operation_evidence_sha256,
        sha256(canonicalBytes(recovered)),
        checkpoint,
      );
      assertPrivate(
        join(prepared.active, ".provider-effect-witness-03"),
        0o600,
        false,
        1,
      );
      assert.equal(
        existsSync(
          join(
            preparation.providerRoot,
            ".synveda-clean-engine-provider-reservation",
          ),
        ),
        false,
        checkpoint,
      );
      assert.deepEqual(
        publishColimaLiveProviderEffectPreAttemptForTest(
          fixtureStartAdmissionArguments(state, preparation),
        ),
        recovered,
        checkpoint,
      );
      assert.equal(run(state, "verify").status, 0, checkpoint);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("fixture provider effect reconciles every linked generic publication stage", async () => {
  const cases = [
    {
      checkpoint: "after-effect-slot-link",
      destination: ".mutation-slot-03",
      disposition: "aborted-before-effect",
      effectDisposition: "not-reached",
      eventCount: 0,
      recoveryStage: "not-started",
    },
    {
      checkpoint: "after-start-authority-link",
      destination: ".provider-effect-event-03-000",
      disposition: "completed",
      effectDisposition: "pending",
      eventCount: 1,
      recoveryStage: "authority-held",
    },
    {
      checkpoint: "after-completion-link",
      destination: ".provider-effect-event-03-001",
      disposition: "completed",
      effectDisposition: "complete",
      eventCount: 2,
      recoveryStage: "completion-retired",
    },
  ];
  for (const [index, testCase] of cases.entries()) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        checkpoint: testCase.checkpoint,
        input: serialized,
        releaseName: `effect-linked-publication-${testCase.checkpoint}`,
        signal: index % 2 === 0 ? "SIGKILL" : "SIGTERM",
      });
      const stageNames = readdirSync(prepared.active).filter((name) =>
        name.startsWith(".mutation-stage-"),
      );
      assert.equal(stageNames.length, 1, testCase.checkpoint);
      assert.match(stageNames[0], /^\.mutation-stage-[0-9a-f]{32}$/u);
      const stagePath = join(prepared.active, stageNames[0]);
      const destinationPath = join(prepared.active, testCase.destination);
      assertPrivate(stagePath, 0o600, false, 2);
      assertPrivate(destinationPath, 0o600, false, 2);
      const stage = lstatSync(stagePath, { bigint: true });
      const destination = lstatSync(destinationPath, { bigint: true });
      assert.equal(stage.nlink, 2n, testCase.checkpoint);
      assert.equal(destination.nlink, 2n, testCase.checkpoint);
      assert.equal(stage.dev, destination.dev, testCase.checkpoint);
      assert.equal(stage.ino, destination.ino, testCase.checkpoint);
      const destinationBytes = readFileSync(destinationPath);
      assert.deepEqual(readFileSync(stagePath), destinationBytes);
      const eventNamesBefore = readdirSync(prepared.active)
        .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
        .sort();
      assert.equal(
        eventNamesBefore.length,
        testCase.eventCount,
        testCase.checkpoint,
      );
      const preservedEvents = eventNamesBefore.map((name) => {
        const path = join(prepared.active, name);
        return {
          bytes: readFileSync(path),
          identity: lstatSync(path, { bigint: true }),
          name,
          value: parse(path),
        };
      });
      const slotPath = join(prepared.active, ".mutation-slot-03");
      const slotBefore = {
        bytes: readFileSync(slotPath),
        identity: lstatSync(slotPath, { bigint: true }),
      };
      const witnessPath = join(
        prepared.active,
        ".provider-effect-witness-03",
      );
      const witnessBefore = existsSync(witnessPath)
        ? {
            bytes: readFileSync(witnessPath),
            identity: lstatSync(witnessPath, { bigint: true }),
          }
        : undefined;
      assert.equal(
        witnessBefore === undefined,
        testCase.recoveryStage === "not-started",
        testCase.checkpoint,
      );
      const stateBeforeObservation = snapshotTree(state.root);
      const preparationBeforeObservation = snapshotTree(preparation.root);
      assert.equal(run(state, "status").status, 0, testCase.checkpoint);
      assert.equal(run(state, "verify").status, 0, testCase.checkpoint);

      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      assert.deepEqual(
        snapshotTree(state.root),
        stateBeforeObservation,
        testCase.checkpoint,
      );
      assert.deepEqual(
        snapshotTree(preparation.root),
        preparationBeforeObservation,
        testCase.checkpoint,
      );
      const recovered = recoverColimaLiveProviderEffectForTest({
        ...recoveryArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.equal(
        existsSync(stagePath),
        false,
        testCase.checkpoint,
      );
      const destinationAfter = lstatSync(destinationPath, { bigint: true });
      assert.equal(destinationAfter.nlink, 1n, testCase.checkpoint);
      assert.equal(destinationAfter.dev, destination.dev, testCase.checkpoint);
      assert.equal(destinationAfter.ino, destination.ino, testCase.checkpoint);
      assert.deepEqual(
        readFileSync(destinationPath),
        destinationBytes,
        testCase.checkpoint,
      );
      for (const preserved of preservedEvents) {
        const path = join(prepared.active, preserved.name);
        const identity = lstatSync(path, { bigint: true });
        assert.equal(identity.dev, preserved.identity.dev, testCase.checkpoint);
        assert.equal(identity.ino, preserved.identity.ino, testCase.checkpoint);
        assert.deepEqual(
          readFileSync(path),
          preserved.bytes,
          testCase.checkpoint,
        );
      }
      const slotAfter = lstatSync(slotPath, { bigint: true });
      assert.equal(slotAfter.dev, slotBefore.identity.dev, testCase.checkpoint);
      assert.equal(slotAfter.ino, slotBefore.identity.ino, testCase.checkpoint);
      assert.deepEqual(
        readFileSync(slotPath),
        slotBefore.bytes,
        testCase.checkpoint,
      );
      if (witnessBefore !== undefined) {
        const witnessAfter = lstatSync(witnessPath, { bigint: true });
        assert.equal(
          witnessAfter.dev,
          witnessBefore.identity.dev,
          testCase.checkpoint,
        );
        assert.equal(
          witnessAfter.ino,
          witnessBefore.identity.ino,
          testCase.checkpoint,
        );
        assert.deepEqual(
          readFileSync(witnessPath),
          witnessBefore.bytes,
          testCase.checkpoint,
        );
      }
      const slot = parse(slotPath);
      const claimPath = join(prepared.active, ".mutation-recovery-03-00");
      const claim = parse(claimPath);
      const zero = "0".repeat(64);
      assert.equal(claim.schema, "synveda.clean-engine.mutation-recovery.v6");
      assert.equal(claim.action, "provider-effect");
      assert.equal(claim.sequence, 0);
      assert.equal(claim.slot_sequence, 3);
      assert.equal(claim.lease_sha256, sha256(slotBefore.bytes));
      assert.equal(claim.parent_sha256, zero);
      assert.equal(
        claim.operation_kind,
        COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
      );
      assert.equal(
        claim.operation_contract_sha256,
        COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
      );
      assert.equal(claim.observed_effect_name, "provider-effect");
      assert.equal(
        claim.observed_effect_disposition,
        testCase.effectDisposition,
      );
      assert.equal(claim.observed_evidence_stage, testCase.recoveryStage);
      assert.equal(claim.observed_settlement_sha256, zero);
      const eventHeadSha256 =
        preservedEvents.length === 0
          ? zero
          : sha256(preservedEvents.at(-1).bytes);
      const witnessSha256 =
        witnessBefore === undefined ? zero : sha256(witnessBefore.bytes);
      const topologySha256 =
        testCase.recoveryStage === "not-started"
          ? zero
          : sha256(
              canonicalBytes({
                event_count: testCase.eventCount,
                event_head_sha256: eventHeadSha256,
                evidence_stage: testCase.recoveryStage,
                local_links:
                  testCase.recoveryStage === "completion-retired" ? 1 : 2,
                marker_present:
                  testCase.recoveryStage !== "completion-retired",
                slot_sequence: 3,
                witness_sha256: witnessSha256,
              }),
            );
      assert.equal(claim.observed_evidence_head_sha256, eventHeadSha256);
      assert.equal(claim.observed_evidence_prefix_sha256, topologySha256);
      assert.equal(claim.observed_residual_sha256, topologySha256);
      const close = parse(join(prepared.active, ".mutation-close-03"));
      assert.equal(
        close.disposition,
        testCase.disposition,
        testCase.checkpoint,
      );
      assert.equal(close.schema, "synveda.clean-engine.mutation-close.v8");
      assert.equal(close.authority, "recovery", testCase.checkpoint);
      assert.equal(
        close.authority_sha256,
        sha256(canonicalBytes(claim)),
        testCase.checkpoint,
      );
      if (testCase.disposition === "completed") {
        assert.equal(recovered.event_kind, "completion", testCase.checkpoint);
        const eventsAfter = readdirSync(prepared.active)
          .filter((name) => /^\.provider-effect-event-03-\d{3}$/u.test(name))
          .sort();
        assert.deepEqual(
          eventsAfter,
          [
            ".provider-effect-event-03-000",
            ".provider-effect-event-03-001",
          ],
          testCase.checkpoint,
        );
        const completionPath = join(prepared.active, eventsAfter.at(-1));
        const completion = parse(completionPath);
        assert.deepEqual(recovered, completion, testCase.checkpoint);
        assert.equal(
          close.operation_evidence_sha256,
          sha256(canonicalBytes(completion)),
          testCase.checkpoint,
        );
        const expectedPublisher =
          testCase.recoveryStage === "authority-held"
            ? sha256(canonicalBytes(claim))
            : sha256(canonicalBytes(slot));
        assert.equal(
          completion.publisher_authority_sha256,
          expectedPublisher,
          testCase.checkpoint,
        );
        if (testCase.recoveryStage === "authority-held") {
          assert.equal(
            completion.evidence.start_authority_sha256,
            sha256(preservedEvents[0].bytes),
          );
        }
        const beforeRetry = snapshotTree(state.root);
        const beforePreparationRetry = snapshotTree(preparation.root);
        assert.deepEqual(
          publishColimaLiveProviderEffectPreAttemptForTest(
            fixtureStartAdmissionArguments(state, preparation),
          ),
          completion,
          testCase.checkpoint,
        );
        assert.deepEqual(snapshotTree(state.root), beforeRetry);
        assert.deepEqual(snapshotTree(preparation.root), beforePreparationRetry);
      } else {
        assert.deepEqual(
          recovered,
          parse(join(prepared.active, "00-plan.json")),
          testCase.checkpoint,
        );
        assert.equal(close.operation_evidence_sha256, zero);
        assert.equal(
          readdirSync(prepared.active).some((name) =>
            name.startsWith(".provider-effect-"),
          ),
          false,
        );
        const oldArtifacts = [
          ".mutation-slot-03",
          ".mutation-recovery-03-00",
          ".mutation-close-03",
        ].map((name) => ({
          bytes: readFileSync(join(prepared.active, name)),
          name,
        }));
        const next = publishColimaLiveProviderEffectPreAttemptForTest(
          fixtureStartAdmissionArguments(state, preparation),
        );
        assert.equal(next.slot_sequence, 4);
        for (const artifact of oldArtifacts) {
          assert.deepEqual(
            readFileSync(join(prepared.active, artifact.name)),
            artifact.bytes,
            artifact.name,
          );
        }
      }
      assert.deepEqual(
        readdirSync(prepared.active).filter((name) =>
          /^\d{2}-.*\.json$/u.test(name),
        ),
        ["00-plan.json"],
      );
      assert.equal(run(state, "verify").status, 0, testCase.checkpoint);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }

  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      checkpoint: "after-effect-close-link",
      input: serialized,
      releaseName: "effect-linked-publication-after-effect-close-link",
      signal: "SIGTERM",
    });
    const stageNames = readdirSync(prepared.active).filter((name) =>
      name.startsWith(".mutation-stage-"),
    );
    assert.equal(stageNames.length, 1);
    assert.match(stageNames[0], /^\.mutation-stage-[0-9a-f]{32}$/u);
    const stagePath = join(prepared.active, stageNames[0]);
    const closePath = join(prepared.active, ".mutation-close-03");
    assertPrivate(stagePath, 0o600, false, 2);
    assertPrivate(closePath, 0o600, false, 2);
    const stage = lstatSync(stagePath, { bigint: true });
    const close = lstatSync(closePath, { bigint: true });
    const closeBytes = readFileSync(closePath);
    assert.equal(stage.nlink, 2n);
    assert.equal(close.nlink, 2n);
    assert.equal(stage.dev, close.dev);
    assert.equal(stage.ino, close.ino);
    assert.deepEqual(readFileSync(stagePath), closeBytes);
    const slot = parse(join(prepared.active, ".mutation-slot-03"));
    const completionPath = join(
      prepared.active,
      ".provider-effect-event-03-001",
    );
    const completionBefore = parse(completionPath);
    const completionBytes = readFileSync(completionPath);
    const completionIdentity = lstatSync(completionPath, { bigint: true });
    const closeValue = parse(closePath);
    assert.equal(closeValue.schema, "synveda.clean-engine.mutation-close.v8");
    assert.equal(closeValue.disposition, "completed");
    assert.equal(closeValue.authority, "owner");
    assert.equal(closeValue.authority_sha256, sha256(canonicalBytes(slot)));
    assert.equal(
      closeValue.operation_evidence_sha256,
      sha256(completionBytes),
    );
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        /^\d{2}-.*\.json$/u.test(name),
      ),
      ["00-plan.json"],
    );
    assert.equal(
      existsSync(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
      ),
      false,
    );
    assertPrivate(
      join(prepared.active, ".provider-effect-witness-03"),
      0o600,
      false,
      1,
    );
    const stateBeforeObservation = snapshotTree(state.root);
    const preparationBeforeObservation = snapshotTree(preparation.root);
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);
    const beforeWrongBinding = snapshotTree(state.root);
    assert.throws(
      () =>
        publishColimaLiveProviderEffectPreAttemptForTest({
          ...fixtureStartAdmissionArguments(state, preparation),
          requirements: {
            ...preparation.requirements,
            historical_retry_probe: "changed",
          },
        }),
      /live provider reservation preparation binding was refused/u,
    );
    assert.deepEqual(snapshotTree(state.root), beforeWrongBinding);
    assert.deepEqual(
      snapshotTree(preparation.root),
      preparationBeforeObservation,
    );
    assert.equal(lstatSync(stagePath, { bigint: true }).nlink, 2n);
    assert.equal(lstatSync(closePath, { bigint: true }).nlink, 2n);
    const beforeWrongTerminal = snapshotTree(state.root);
    assert.throws(
      () =>
        publishColimaLiveProviderEffectAttemptFenceForTest(
          fixtureStartAdmissionArguments(state, preparation),
        ),
      /fixture provider effect terminal differed/u,
    );
    assert.deepEqual(snapshotTree(state.root), beforeWrongTerminal);
    assert.deepEqual(
      snapshotTree(preparation.root),
      preparationBeforeObservation,
    );
    assert.equal(
      lstatSync(join(prepared.active, stageNames[0]), { bigint: true }).nlink,
      2n,
    );
    assert.equal(
      lstatSync(join(prepared.active, ".mutation-close-03"), {
        bigint: true,
      }).nlink,
      2n,
    );
    const beforeRecoveryConfirmation = snapshotTree(state.root);
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest({
          ...serialized,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /no abandoned provider mutation was available/u,
    );
    assert.deepEqual(snapshotTree(state.root), beforeRecoveryConfirmation);
    assert.deepEqual(
      snapshotTree(preparation.root),
      preparationBeforeObservation,
    );
    assert.deepEqual(stateBeforeObservation, beforeWrongTerminal);
    const completion = publishColimaLiveProviderEffectPreAttemptForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.deepEqual(completion, completionBefore);
    assert.equal(existsSync(stagePath), false);
    const closeAfter = lstatSync(closePath, { bigint: true });
    assert.equal(closeAfter.nlink, 1n);
    assert.equal(closeAfter.dev, close.dev);
    assert.equal(closeAfter.ino, close.ino);
    assert.deepEqual(readFileSync(closePath), closeBytes);
    const completionAfter = lstatSync(completionPath, { bigint: true });
    assert.equal(completionAfter.dev, completionIdentity.dev);
    assert.equal(completionAfter.ino, completionIdentity.ino);
    assert.deepEqual(readFileSync(completionPath), completionBytes);
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        /^\.mutation-recovery-03-/u.test(name) ||
        name === ".mutation-slot-04" ||
        name === ".provider-effect-event-03-002",
      ),
      false,
    );
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        /^\d{2}-.*\.json$/u.test(name),
      ),
      ["00-plan.json"],
    );
    assert.deepEqual(
      snapshotTree(preparation.root),
      preparationBeforeObservation,
    );
    const beforeSecondRetry = snapshotTree(state.root);
    const beforeSecondPreparationRetry = snapshotTree(preparation.root);
    assert.deepEqual(
      publishColimaLiveProviderEffectPreAttemptForTest(
        fixtureStartAdmissionArguments(state, preparation),
      ),
      completionBefore,
    );
    assert.deepEqual(snapshotTree(state.root), beforeSecondRetry);
    assert.deepEqual(
      snapshotTree(preparation.root),
      beforeSecondPreparationRetry,
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture provider effect recovery reconciles linked claim and close stages", async () => {
  {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        checkpoint: "after-effect-slot",
        input: serialized,
        releaseName: "effect-linked-recovery-claim-owner",
      });
      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      let confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      await crashLiveProviderEffectAt(state, {
        action: "recover",
        checkpoint: "after-recovery-claim",
        input: { ...serialized, confirmation },
        releaseName: "effect-linked-recovery-claim-first-recoverer",
        signal: "SIGTERM",
      });
      const claimPath = join(
        prepared.active,
        ".mutation-recovery-03-00",
      );
      const stagePath = join(
        prepared.active,
        `.mutation-stage-${"b".repeat(32)}`,
      );
      const claimBytes = readFileSync(claimPath);
      const claimIdentity = lstatSync(claimPath, { bigint: true });
      linkSync(claimPath, stagePath);
      assertPrivate(claimPath, 0o600, false, 2);
      assertPrivate(stagePath, 0o600, false, 2);
      const linkedClaimIdentity = lstatSync(claimPath, { bigint: true });
      const linkedStageIdentity = lstatSync(stagePath, { bigint: true });
      assert.equal(linkedClaimIdentity.nlink, 2n);
      assert.equal(linkedStageIdentity.nlink, 2n);
      assert.equal(linkedStageIdentity.dev, linkedClaimIdentity.dev);
      assert.equal(linkedStageIdentity.ino, linkedClaimIdentity.ino);
      assert.deepEqual(readFileSync(stagePath), claimBytes);
      const stateBeforeObservation = snapshotTree(state.root);
      const preparationBeforeObservation = snapshotTree(preparation.root);
      assert.equal(run(state, "status").status, 0);
      assert.equal(run(state, "verify").status, 0);
      confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      assert.deepEqual(snapshotTree(state.root), stateBeforeObservation);
      assert.deepEqual(
        snapshotTree(preparation.root),
        preparationBeforeObservation,
      );
      const recovered = recoverColimaLiveProviderEffectForTest({
        ...recoveryArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.deepEqual(
        recovered,
        parse(join(prepared.active, "00-plan.json")),
      );
      assert.equal(existsSync(stagePath), false);
      const claimAfter = lstatSync(claimPath, { bigint: true });
      assert.equal(claimAfter.nlink, 1n);
      assert.equal(claimAfter.dev, claimIdentity.dev);
      assert.equal(claimAfter.ino, claimIdentity.ino);
      assert.deepEqual(readFileSync(claimPath), claimBytes);
      const nextClaim = parse(
        join(prepared.active, ".mutation-recovery-03-01"),
      );
      assert.equal(nextClaim.sequence, 1);
      assert.equal(nextClaim.parent_sha256, sha256(claimBytes));
      const close = parse(join(prepared.active, ".mutation-close-03"));
      assert.equal(close.disposition, "aborted-before-effect");
      assert.equal(close.authority, "recovery");
      assert.equal(close.authority_sha256, sha256(canonicalBytes(nextClaim)));
      assert.equal(close.operation_evidence_sha256, "0".repeat(64));
      assert.equal(run(state, "verify").status, 0);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }

  {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const serialized = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderEffectAt(state, {
        checkpoint: "after-completion",
        input: serialized,
        releaseName: "effect-linked-recovery-close-owner",
        signal: "SIGTERM",
      });
      const recoveryArguments = {
        ...serialized,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const confirmation =
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        );
      await crashLiveProviderEffectAt(state, {
        action: "recover",
        checkpoint: "after-recovery-close-link",
        input: { ...serialized, confirmation },
        releaseName: "effect-linked-recovery-close-recoverer",
      });
      const stageNames = readdirSync(prepared.active).filter((name) =>
        /^\.mutation-stage-[0-9a-f]{32}$/u.test(name),
      );
      assert.equal(stageNames.length, 1);
      const stagePath = join(prepared.active, stageNames[0]);
      const closePath = join(prepared.active, ".mutation-close-03");
      const claimPath = join(
        prepared.active,
        ".mutation-recovery-03-00",
      );
      const completionPath = join(
        prepared.active,
        ".provider-effect-event-03-001",
      );
      assertPrivate(stagePath, 0o600, false, 2);
      assertPrivate(closePath, 0o600, false, 2);
      const stageIdentity = lstatSync(stagePath, { bigint: true });
      const closeIdentity = lstatSync(closePath, { bigint: true });
      assert.equal(stageIdentity.nlink, 2n);
      assert.equal(closeIdentity.nlink, 2n);
      assert.equal(stageIdentity.dev, closeIdentity.dev);
      assert.equal(stageIdentity.ino, closeIdentity.ino);
      const closeBytes = readFileSync(closePath);
      assert.deepEqual(readFileSync(stagePath), closeBytes);
      const claimBytes = readFileSync(claimPath);
      const completionBytes = readFileSync(completionPath);
      const close = parse(closePath);
      assert.equal(close.disposition, "completed");
      assert.equal(close.authority, "recovery");
      assert.equal(close.authority_sha256, sha256(claimBytes));
      assert.equal(close.operation_evidence_sha256, sha256(completionBytes));
      const stateBeforeObservation = snapshotTree(state.root);
      const preparationBeforeObservation = snapshotTree(preparation.root);
      assert.equal(run(state, "status").status, 0);
      assert.equal(run(state, "verify").status, 0);
      assert.throws(
        () =>
          liveProviderFixtureEffectRecoveryConfirmationForTest(
            recoveryArguments,
          ),
        /no abandoned provider mutation was available/u,
      );
      assert.deepEqual(snapshotTree(state.root), stateBeforeObservation);
      assert.deepEqual(
        snapshotTree(preparation.root),
        preparationBeforeObservation,
      );
      const completion = publishColimaLiveProviderEffectPreAttemptForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      assert.deepEqual(completion, parse(completionPath));
      assert.equal(existsSync(stagePath), false);
      const closeAfter = lstatSync(closePath, { bigint: true });
      assert.equal(closeAfter.nlink, 1n);
      assert.equal(closeAfter.dev, closeIdentity.dev);
      assert.equal(closeAfter.ino, closeIdentity.ino);
      assert.deepEqual(readFileSync(closePath), closeBytes);
      assert.deepEqual(readFileSync(claimPath), claimBytes);
      assert.deepEqual(readFileSync(completionPath), completionBytes);
      assert.equal(
        existsSync(join(prepared.active, ".mutation-recovery-03-01")),
        false,
      );
      assert.equal(run(state, "verify").status, 0);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("fixture provider effect recovery newest same-frontier claim alone completes", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      checkpoint: "after-marker-unlink",
      input: serialized,
      releaseName: "effect-owner-marker-unlinked",
    });
    const base = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    let confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-recovery-claim",
      input: { ...serialized, confirmation },
      releaseName: "effect-recovery-first-claim",
      signal: "SIGTERM",
    });
    confirmation = liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    await crashLiveProviderEffectAt(state, {
      action: "recover",
      checkpoint: "after-recovery-marker-retirement-directory-sync",
      input: { ...serialized, confirmation },
      releaseName: "effect-recovery-second-claim",
    });
    confirmation = liveProviderFixtureEffectRecoveryConfirmationForTest(base);
    const completion = recoverColimaLiveProviderEffectForTest({
      ...base,
      confirmation,
      testCheckpoint() {},
    });
    const claimNames = readdirSync(prepared.active)
      .filter((name) => /^\.mutation-recovery-03-\d{2}$/u.test(name))
      .sort();
    assert.deepEqual(claimNames, [
      ".mutation-recovery-03-00",
      ".mutation-recovery-03-01",
      ".mutation-recovery-03-02",
    ]);
    const claims = claimNames.map((name) =>
      parse(join(prepared.active, name)),
    );
    assert.equal(
      new Set(claims.map((claim) => claim.observed_evidence_stage)).size,
      1,
    );
    assert.equal(claims[0].observed_evidence_stage, "witness-unlinked");
    for (let index = 1; index < claims.length; index += 1) {
      assert.equal(
        claims[index].parent_sha256,
        sha256(canonicalBytes(claims[index - 1])),
      );
    }
    assert.equal(
      completion.publisher_authority_sha256,
      sha256(canonicalBytes(claims.at(-1))),
    );
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.authority, "recovery");
    assert.equal(close.authority_sha256, completion.publisher_authority_sha256);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a dead fixture provider effect attempt fence refuses recovery without mutation", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "attempt",
      checkpoint: "after-start-attempt",
      input: serialized,
      releaseName: "effect-attempt-fence",
    });
    const before = snapshotTree(state.root);
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest({
          ...serialized,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /attempt fence requires operator resolution/u,
    );
    assert.deepEqual(snapshotTree(state.root), before);
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        name.startsWith(".mutation-recovery-03-"),
      ),
      false,
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a linked attempt-fence publication stage remains inert and unrecoverable", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "attempt",
      checkpoint: "after-start-attempt-link",
      input: serialized,
      releaseName: "effect-attempt-fence-linked-stage",
      signal: "SIGTERM",
    });
    const stageNames = readdirSync(prepared.active).filter((name) =>
      name.startsWith(".mutation-stage-"),
    );
    assert.equal(stageNames.length, 1);
    const stagePath = join(prepared.active, stageNames[0]);
    const eventPath = join(
      prepared.active,
      ".provider-effect-event-03-001",
    );
    const stageIdentity = lstatSync(stagePath, { bigint: true });
    const eventIdentity = lstatSync(eventPath, { bigint: true });
    assert.equal(stageIdentity.nlink, 2n);
    assert.equal(eventIdentity.nlink, 2n);
    assert.equal(stageIdentity.dev, eventIdentity.dev);
    assert.equal(stageIdentity.ino, eventIdentity.ino);
    assert.deepEqual(readFileSync(stagePath), readFileSync(eventPath));
    const before = snapshotTree(state.root);
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest({
          ...serialized,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /attempt fence requires operator resolution/u,
    );
    assert.deepEqual(snapshotTree(state.root), before);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture provider effect marker collision preserves foreign evidence and aborts", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const foreign = Buffer.from("foreign provider effect marker\n", "utf8");
    const argumentsValue = fixtureStartAdmissionArguments(
      state,
      preparation,
      (checkpoint) => {
        if (checkpoint === "before-marker-link") {
          writePrivateColimaLiveFixtureFile(marker, foreign, 0o600);
        }
      },
    );
    assert.throws(
      () => publishColimaLiveProviderEffectPreAttemptForTest(argumentsValue),
      /fixture provider effect marker was already held/u,
    );
    assert.deepEqual(readFileSync(marker), foreign);
    const active = activeRun(state);
    const close = parse(join(active, ".mutation-close-03"));
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(
      readdirSync(active).some((name) =>
        name.startsWith(".provider-effect-stage-") ||
        name.startsWith(".provider-effect-witness-") ||
        name.startsWith(".provider-effect-event-"),
      ),
      false,
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("crashed fixture provider effect collision never retires the foreign marker", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const serialized = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderEffectAt(state, {
      action: "collision",
      checkpoint: "after-witness-stage-unlink",
      input: serialized,
      releaseName: "effect-collision-after-stage-unlink",
    });
    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const markerBytes = readFileSync(marker);
    const markerIdentity = lstatSync(marker, { bigint: true });
    assert.equal(markerIdentity.nlink, 1n);
    assert.equal(
      markerBytes.toString("utf8"),
      "foreign provider effect marker\n",
    );
    const recoveryArguments = {
      ...serialized,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const before = snapshotTree(state.root);
    assert.throws(
      () =>
        liveProviderFixtureEffectRecoveryConfirmationForTest(
          recoveryArguments,
        ),
      /fixture provider effect marker was not absent/u,
    );
    assert.deepEqual(snapshotTree(state.root), before);
    const markerAfterRefusal = lstatSync(marker, { bigint: true });
    assert.equal(markerAfterRefusal.nlink, 1n);
    assert.equal(markerAfterRefusal.dev, markerIdentity.dev);
    assert.equal(markerAfterRefusal.ino, markerIdentity.ino);
    assert.deepEqual(readFileSync(marker), markerBytes);

    unlinkSync(marker);
    const confirmation =
      liveProviderFixtureEffectRecoveryConfirmationForTest(
        recoveryArguments,
      );
    const recovered = recoverColimaLiveProviderEffectForTest({
      ...recoveryArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(recovered.phase, "plan");
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(close.disposition, "aborted-before-effect");
    assert.equal(close.operation_evidence_sha256, "0".repeat(64));
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the state owner publishes and retires one no-process provider reservation", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const checkpoints = [];
    const argumentsValue = fixtureStartAdmissionArguments(
      state,
      preparation,
      (checkpoint) => {
        checkpoints.push(checkpoint);
      },
    );
    const completion =
      publishColimaLiveProviderReservationForTest(argumentsValue);
    assert.equal(
      completion.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
    );
    assert.equal(completion.slot_sequence, 3);
    assert.equal(completion.state_integration, "mutation-journal-v7-no-spawn-reservation-only");
    assert.equal(
      existsSync(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
      ),
      false,
    );
    const witness = join(
      prepared.active,
      ".provider-reservation-witness-03",
    );
    assertPrivate(witness, 0o600, false, 1);
    assertPrivate(join(prepared.active, ".mutation-operation-03"), 0o600);
    assertPrivate(join(prepared.active, ".mutation-close-03"), 0o600);
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        name.startsWith(".provider-reservation-stage-"),
      ),
      [],
    );
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        /^\.mutation-slot-/u.test(name),
      ),
      [
        ".mutation-slot-00",
        ".mutation-slot-01",
        ".mutation-slot-02",
        ".mutation-slot-03",
      ],
    );
    assert.deepEqual(
      readdirSync(prepared.active).filter((name) =>
        /^[0-9]{2}-.*\.json$/u.test(name),
      ),
      ["00-plan.json"],
    );
    assert.equal(existsSync(join(prepared.active, "environment.json")), false);
    for (const directory of ["evidence", "provider", "registry", "runtime"]) {
      assert.deepEqual(readdirSync(join(prepared.active, directory)), []);
    }
    assert.equal(run(state, "verify").status, 0);
    const repeated =
      publishColimaLiveProviderReservationForTest(argumentsValue);
    assert.deepEqual(repeated, completion);
    assert.equal(
      checkpoints.includes("after-retirement-authorization-publication"),
      true,
    );
    assert.equal(checkpoints.includes("after-marker-unlink"), true);
    assert.equal(checkpoints.includes("after-reservation-close"), true);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation state refuses malformed orphan and non-effect settlements", () => {
  for (const mutation of [
    "malformed-root-observation",
    "orphan-operation",
    "recovery-evidence-head",
    "stage-only-recovery-authority",
  ]) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      publishColimaLiveProviderReservationForTest(
        fixtureStartAdmissionArguments(state, preparation),
      );
      const slotPath = join(prepared.active, ".mutation-slot-03");
      const operationPath = join(prepared.active, ".mutation-operation-03");
      const closePath = join(prepared.active, ".mutation-close-03");
      const slotBytes = readFileSync(slotPath);
      const slot = JSON.parse(slotBytes.toString("utf8"));
      const operation = parse(operationPath);
      const close = parse(closePath);
      let expectedError;

      if (mutation === "malformed-root-observation") {
        operation.pre_retirement_root_observation_sha256 = "f".repeat(64);
        close.operation_evidence_sha256 = sha256(canonicalBytes(operation));
        writeFileSync(operationPath, canonicalBytes(operation), { mode: 0o600 });
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
        expectedError = /live provider reservation settlement was refused/u;
      } else if (mutation === "orphan-operation") {
        const orphanSlotBytes = readFileSync(
          join(prepared.active, ".mutation-slot-02"),
        );
        const orphan = {
          ...operation,
          slot_sequence: 2,
          slot_sha256: sha256(orphanSlotBytes),
        };
        writeFileSync(
          join(prepared.active, ".mutation-operation-02"),
          canonicalBytes(orphan),
          { mode: 0o600 },
        );
        expectedError = /mutation operation settlement action was refused/u;
      } else {
        const witnessBytes = readFileSync(
          join(prepared.active, ".provider-reservation-witness-03"),
        );
        const zeroSha256 = "0".repeat(64);
        const evidenceSha256 =
          mutation === "recovery-evidence-head"
            ? "d".repeat(64)
            : sha256(witnessBytes);
        const evidenceStage =
          mutation === "recovery-evidence-head"
            ? "retirement-authorized-retired"
            : "stage-only";
        const observedSettlementSha256 =
          mutation === "recovery-evidence-head"
            ? sha256(canonicalBytes(operation))
            : zeroSha256;
        const topologySha256 = sha256(canonicalBytes({
          evidence_sha256: evidenceSha256,
          evidence_stage: evidenceStage,
          local_links: 1,
          settlement_sha256: observedSettlementSha256,
          slot_sequence: 3,
        }));
        const leaseSha256 = sha256(slotBytes);
        const operationPlanSha256 = sha256(
          canonicalBytes(slot.operation_plan),
        );
        const recovery = {
          action: slot.action,
          chain_root_sha256: sha256(canonicalBytes({
            action: slot.action,
            fixture_id: slot.fixture_id,
            lease_sha256: leaseSha256,
            operation_contract_sha256: slot.operation_contract_sha256,
            operation_kind: slot.operation_kind,
            operation_plan_sha256: operationPlanSha256,
            schema: "synveda.clean-engine.mutation-recovery-root.v6",
          })),
          fixture_id: slot.fixture_id,
          lease_sha256: leaseSha256,
          nonce: "a".repeat(32),
          observed_effect_disposition:
            evidenceStage === "stage-only" ? "not-reached" : "complete",
          observed_effect_name: "provider-reservation",
          observed_evidence_head_sha256: evidenceSha256,
          observed_evidence_prefix_sha256: topologySha256,
          observed_evidence_stage: evidenceStage,
          observed_residual_sha256: topologySha256,
          observed_settlement_sha256: observedSettlementSha256,
          operation_contract_sha256: slot.operation_contract_sha256,
          operation_kind: slot.operation_kind,
          operation_plan_sha256: operationPlanSha256,
          owner_boot_sha256: "b".repeat(64),
          owner_instance_sha256: "c".repeat(64),
          owner_pid: 2_147_483_645,
          owner_probe: "opaque-process-instance-v1",
          parent_sha256: zeroSha256,
          schema: "synveda.clean-engine.mutation-recovery.v6",
          sequence: 0,
          slot_sequence: 3,
          source_head_sha256: slot.source_head_sha256,
        };
        const recoveryBytes = canonicalBytes(recovery);
        const recoverySha256 = sha256(recoveryBytes);
        writeFileSync(
          join(prepared.active, ".mutation-recovery-03-00"),
          recoveryBytes,
          { mode: 0o600 },
        );
        if (mutation === "stage-only-recovery-authority") {
          operation.authority = "recovery";
          operation.authority_sha256 = recoverySha256;
        }
        const operationBytes = canonicalBytes(operation);
        close.authority = "recovery";
        close.authority_sha256 = recoverySha256;
        close.operation_evidence_sha256 = sha256(operationBytes);
        writeFileSync(operationPath, operationBytes, { mode: 0o600 });
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
        expectedError = mutation === "recovery-evidence-head"
          ? /live provider reservation recovery witness was refused/u
          : /live provider reservation recovery topology regressed/u;
      }

      const refused = run(state, "verify");
      assert.notEqual(refused.status, 0, `${mutation}: ${refused.stderr}`);
      assert.match(refused.stderr, expectedError, mutation);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("reservation recovery retires an inert stage and a fresh generation completes", async () => {
  const state = fixture();
  let preparation;
  let child;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(state.root, "release-reservation-stage");
    const serialized = JSON.stringify({
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    });
    child = spawn(
      process.execPath,
      [
        helper,
        "publish",
        state.repo,
        state.state,
        "after-witness-stage",
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let inputWriteError;
    child.stdin.on("error", (error) => {
      inputWriteError = error;
    });
    child.stdin.end(serialized);
    child.stdout.resume();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    const stageName = readdirSync(prepared.active).find((name) =>
      name.startsWith(".provider-reservation-stage-"),
    );
    assert.notEqual(stageName, undefined);
    assertPrivate(join(prepared.active, stageName), 0o600, false, 1);
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");

    const confirmationArguments = {
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
    };
    assert.throws(
      () => providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /live provider reservation requires its physical recovery confirmation/u,
    );
    const confirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider recovery requires the matching dedicated executor/u,
    );
    const recovered = recoverColimaLiveProviderReservationForTest({
      ...confirmationArguments,
      confirmation,
      testCheckpoint() {},
    });
    assert.equal(recovered.phase, "plan");
    assert.equal(
      parse(join(prepared.active, ".mutation-close-03")).disposition,
      "aborted-before-effect",
    );
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        name.startsWith(".provider-reservation-stage-"),
      ),
      false,
    );
    const completion = publishColimaLiveProviderReservationForTest(
      fixtureStartAdmissionArguments(state, preparation),
    );
    assert.equal(completion.slot_sequence, 4);
    assertPrivate(
      join(prepared.active, ".provider-reservation-witness-04"),
      0o600,
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation recovery survives a crash after retiring an inert stage", async () => {
  const state = fixture();
  let preparation;
  let child;
  const stopAt = async (action, serialized, checkpoint, releaseName) => {
    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(state.root, releaseName);
    child = spawn(
      process.execPath,
      [
        helper,
        action,
        state.repo,
        state.state,
        checkpoint,
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let inputWriteError;
    child.stdin.on("error", (error) => {
      inputWriteError = error;
    });
    child.stdin.end(JSON.stringify(serialized));
    child.stdout.resume();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");
  };
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fixtureInput = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await stopAt(
      "publish",
      fixtureInput,
      "after-witness-stage",
      "release-owner-stage",
    );
    const confirmationArguments = {
      ...fixtureInput,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const firstConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await stopAt(
      "recover",
      { ...fixtureInput, confirmation: firstConfirmation },
      "after-witness-stage-unlink",
      "release-recovery-stage-retirement",
    );
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        name.startsWith(".provider-reservation-stage-"),
      ),
      false,
    );
    const secondConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    const recovered = recoverColimaLiveProviderReservationForTest({
      ...confirmationArguments,
      confirmation: secondConfirmation,
      testCheckpoint() {},
    });
    assert.equal(recovered.phase, "plan");
    assert.equal(
      parse(join(prepared.active, ".mutation-close-03")).disposition,
      "aborted-before-effect",
    );
    assert.deepEqual(
      readdirSync(prepared.active)
        .filter((name) => name.startsWith(".mutation-recovery-03-"))
        .sort(),
      [".mutation-recovery-03-00", ".mutation-recovery-03-01"],
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation recovery accepts reachable stages across repeated crashes", async () => {
  const state = fixture();
  let preparation;
  let child;
  const stopAt = async (action, serialized, checkpoint, releaseName) => {
    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(state.root, releaseName);
    child = spawn(
      process.execPath,
      [
        helper,
        action,
        state.repo,
        state.state,
        checkpoint,
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let inputWriteError;
    child.stdin.on("error", (error) => {
      inputWriteError = error;
    });
    child.stdin.end(JSON.stringify(serialized));
    child.stdout.resume();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");
  };
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fixtureInput = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await stopAt(
      "publish",
      fixtureInput,
      "after-marker-directory-sync",
      "release-owner-marker-linked",
    );
    const confirmationArguments = {
      ...fixtureInput,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const firstConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await stopAt(
      "recover",
      { ...fixtureInput, confirmation: firstConfirmation },
      "after-witness-stage-directory-sync",
      "release-recovery-marker-held",
    );

    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const witness = join(
      prepared.active,
      ".provider-reservation-witness-03",
    );
    assertPrivate(marker, 0o600, false, 2);
    assertPrivate(witness, 0o600, false, 2);
    assert.equal(lstatSync(marker).ino, lstatSync(witness).ino);
    assert.equal(
      readdirSync(prepared.active).some((name) =>
        name.startsWith(".provider-reservation-stage-"),
      ),
      false,
    );

    const secondConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await crashLiveProviderReservationAt(state, {
      action: "recover",
      checkpoint: "after-marker-retirement-directory-sync",
      input: { ...fixtureInput, confirmation: secondConfirmation },
      releaseName: "release-second-recovery-after-marker-retirement",
    });
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    const thirdConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    const completion = recoverColimaLiveProviderReservationForTest({
      ...confirmationArguments,
      confirmation: thirdConfirmation,
      testCheckpoint() {},
    });
    assert.equal(
      completion.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
    );
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    assert.deepEqual(
      readdirSync(prepared.active)
        .filter((name) => name.startsWith(".mutation-recovery-03-"))
        .sort(),
      [
        ".mutation-recovery-03-00",
        ".mutation-recovery-03-01",
        ".mutation-recovery-03-02",
      ],
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation recovery preserves settlement and newest-close causality", async () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fixtureInput = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await crashLiveProviderReservationAt(state, {
      action: "publish",
      checkpoint: "before-retirement-observation",
      input: fixtureInput,
      releaseName: "release-owner-before-recovery-settlement",
    });
    const confirmationArguments = {
      ...fixtureInput,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const firstConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await crashLiveProviderReservationAt(state, {
      action: "recover",
      checkpoint: "after-retirement-authorization-publication",
      input: { ...fixtureInput, confirmation: firstConfirmation },
      releaseName: "release-first-recovery-after-settlement",
    });

    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const witness = join(
      prepared.active,
      ".provider-reservation-witness-03",
    );
    assertPrivate(marker, 0o600, false, 2);
    assertPrivate(witness, 0o600, false, 2);
    const operationPath = join(
      prepared.active,
      ".mutation-operation-03",
    );
    assertPrivate(operationPath, 0o600);

    const secondConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await crashLiveProviderReservationAt(state, {
      action: "recover",
      checkpoint: "after-marker-retirement-directory-sync",
      input: { ...fixtureInput, confirmation: secondConfirmation },
      releaseName: "release-causal-recovery-after-marker-retirement",
    });
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    const thirdConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    const completion = recoverColimaLiveProviderReservationForTest({
      ...confirmationArguments,
      confirmation: thirdConfirmation,
      testCheckpoint() {},
    });
    assert.equal(
      completion.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
    );
    const firstClaimPath = join(
      prepared.active,
      ".mutation-recovery-03-00",
    );
    const secondClaimPath = join(
      prepared.active,
      ".mutation-recovery-03-01",
    );
    const thirdClaimPath = join(
      prepared.active,
      ".mutation-recovery-03-02",
    );
    const firstClaim = parse(firstClaimPath);
    const secondClaim = parse(secondClaimPath);
    const thirdClaim = parse(thirdClaimPath);
    const operationBytes = readFileSync(operationPath);
    const operation = JSON.parse(operationBytes.toString("utf8"));
    const close = parse(join(prepared.active, ".mutation-close-03"));
    assert.equal(firstClaim.observed_settlement_sha256, "0".repeat(64));
    assert.equal(
      secondClaim.observed_settlement_sha256,
      sha256(operationBytes),
    );
    assert.equal(
      thirdClaim.observed_settlement_sha256,
      sha256(operationBytes),
    );
    assert.equal(operation.authority, "recovery");
    assert.equal(
      operation.authority_sha256,
      sha256(readFileSync(firstClaimPath)),
    );
    assert.equal(close.authority, "recovery");
    assert.equal(
      close.authority_sha256,
      sha256(readFileSync(thirdClaimPath)),
    );
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation state refuses a recovery claim followed by local topology regression", async () => {
  const state = fixture();
  let preparation;
  let child;
  const stopAt = async (action, serialized, checkpoint, releaseName) => {
    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(state.root, releaseName);
    child = spawn(
      process.execPath,
      [
        helper,
        action,
        state.repo,
        state.state,
        checkpoint,
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let inputWriteError;
    child.stdin.on("error", (error) => {
      inputWriteError = error;
    });
    child.stdin.end(JSON.stringify(serialized));
    child.stdout.resume();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath)) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");
  };
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fixtureInput = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    await stopAt(
      "publish",
      fixtureInput,
      "after-marker-directory-sync",
      "release-owner-marker-before-regression",
    );
    const confirmationArguments = {
      ...fixtureInput,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const confirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await stopAt(
      "recover",
      { ...fixtureInput, confirmation },
      "after-recovery-claim",
      "release-recovery-claim-before-regression",
    );
    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    unlinkSync(marker);
    assertPrivate(
      readdirSync(prepared.active)
        .filter((name) => name.startsWith(".provider-reservation-stage-"))
        .map((name) => join(prepared.active, name))[0],
      0o600,
      false,
      1,
    );
    const refused = run(state, "verify");
    assert.notEqual(refused.status, 0, refused.stderr);
    assert.match(
      refused.stderr,
      /live provider reservation recovery topology regressed/u,
    );
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation state refuses derived recovery stages as the first claim", async () => {
  for (const derivedStage of [
    "marker-reacquired",
    "stage-retired-before-effect",
  ]) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const fixtureInput = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      await crashLiveProviderReservationAt(state, {
        action: "publish",
        checkpoint:
          derivedStage === "marker-reacquired"
            ? "before-retirement-observation"
            : "after-witness-stage",
        input: fixtureInput,
        releaseName: `release-owner-before-${derivedStage}`,
      });
      const confirmationArguments = {
        ...fixtureInput,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const confirmation =
        liveProviderFixtureReservationRecoveryConfirmationForTest(
          confirmationArguments,
        );
      await crashLiveProviderReservationAt(state, {
        action: "recover",
        checkpoint: "after-recovery-claim",
        input: { ...fixtureInput, confirmation },
        releaseName: `release-first-claim-before-${derivedStage}`,
      });

      const recoveryPath = join(
        prepared.active,
        ".mutation-recovery-03-00",
      );
      const recovery = parse(recoveryPath);
      if (derivedStage === "stage-retired-before-effect") {
        const stageName = readdirSync(prepared.active).find((name) =>
          name.startsWith(".provider-reservation-stage-"),
        );
        assert.notEqual(stageName, undefined);
        unlinkSync(join(prepared.active, stageName));
      }
      recovery.observed_effect_disposition =
        derivedStage === "stage-retired-before-effect"
          ? "not-reached"
          : "pending";
      recovery.observed_evidence_stage = derivedStage;
      const topologySha256 = sha256(canonicalBytes({
        evidence_sha256: recovery.observed_evidence_head_sha256,
        evidence_stage: derivedStage,
        local_links:
          derivedStage === "stage-retired-before-effect" ? 0 : 2,
        settlement_sha256: recovery.observed_settlement_sha256,
        slot_sequence: recovery.slot_sequence,
      }));
      recovery.observed_evidence_prefix_sha256 = topologySha256;
      recovery.observed_residual_sha256 = topologySha256;
      writeFileSync(recoveryPath, canonicalBytes(recovery), { mode: 0o600 });

      const refused = run(state, "verify");
      assert.notEqual(refused.status, 0, `${derivedStage}: ${refused.stderr}`);
      assert.match(
        refused.stderr,
        /live provider reservation recovery history was refused/u,
        derivedStage,
      );
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("reservation recovery durably reacquires a prematurely unlinked marker", async () => {
  const state = fixture();
  let preparation;
  let child;
  const stopAt = async (action, serialized, checkpoint, releaseName, ready) => {
    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(state.root, releaseName);
    child = spawn(
      process.execPath,
      [
        helper,
        action,
        state.repo,
        state.state,
        checkpoint,
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let inputWriteError;
    child.stdin.on("error", (error) => {
      inputWriteError = error;
    });
    child.stdin.end(JSON.stringify(serialized));
    child.stdout.resume();
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    const readyPath = `${releasePath}.ready-${child.pid}`;
    const deadline = Date.now() + 10_000;
    while (!existsSync(readyPath) || !ready()) {
      assert.equal(inputWriteError, undefined);
      assert.equal(child.exitCode, null, stderr);
      assert.ok(Date.now() < deadline, stderr);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    return child;
  };
  const killStopped = async () => {
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");
  };
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const fixtureInput = {
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    };
    const marker = join(
      preparation.providerRoot,
      ".synveda-clean-engine-provider-reservation",
    );
    const witness = join(
      prepared.active,
      ".provider-reservation-witness-03",
    );
    await stopAt(
      "publish",
      fixtureInput,
      "before-retirement-observation",
      "release-owner-held-marker",
      () => existsSync(marker) && existsSync(witness),
    );
    assertPrivate(marker, 0o600, false, 2);
    assertPrivate(witness, 0o600, false, 2);
    assert.equal(lstatSync(marker).ino, lstatSync(witness).ino);
    unlinkSync(marker);
    assertPrivate(witness, 0o600, false, 1);
    await killStopped();

    const confirmationArguments = {
      ...fixtureInput,
      repoRoot: state.repo,
      stateBase: state.state,
    };
    const firstConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    await stopAt(
      "recover",
      { ...fixtureInput, confirmation: firstConfirmation },
      "after-recovery-marker-link",
      "release-recovery-marker-link",
      () => existsSync(marker) && lstatSync(witness).nlink === 2,
    );
    assert.equal(lstatSync(marker).ino, lstatSync(witness).ino);
    await killStopped();

    const secondConfirmation =
      liveProviderFixtureReservationRecoveryConfirmationForTest(
        confirmationArguments,
      );
    const completion = recoverColimaLiveProviderReservationForTest({
      ...confirmationArguments,
      confirmation: secondConfirmation,
      testCheckpoint() {},
    });
    assert.equal(
      completion.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
    );
    assert.equal(existsSync(marker), false);
    assertPrivate(witness, 0o600, false, 1);
    assert.deepEqual(
      readdirSync(prepared.active)
        .filter((name) => name.startsWith(".mutation-recovery-03-"))
        .sort(),
      [".mutation-recovery-03-00", ".mutation-recovery-03-01"],
    );
    const settlement = parse(
      join(prepared.active, ".mutation-operation-03"),
    );
    assert.equal(settlement.authority, "recovery");
    assert.equal(
      settlement.authority_sha256,
      sha256(
        readFileSync(
          join(prepared.active, ".mutation-recovery-03-01"),
        ),
      ),
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("reservation crash boundaries converge without process or receipt authority", async () => {
  const cases = [
    "after-reservation-plan",
    "after-reservation-slot-link",
    "after-reservation-slot",
    "after-witness-stage",
    "after-marker-link",
    "after-marker-directory-sync",
    "after-state-witness-link",
    "after-state-witness-directory-sync",
    "after-witness-stage-unlink",
    "after-witness-stage-directory-sync",
    "before-retirement-observation",
    "after-retirement-observation",
    "after-retirement-authorization-link",
    "after-retirement-authorization-publication",
    "after-marker-unlink",
    "after-marker-retirement-directory-sync",
    "after-reservation-close-link",
    "after-reservation-close",
  ];
  const helper = resolve(
    "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
  );
  for (const [index, checkpoint] of cases.entries()) {
    const state = fixture();
    let preparation;
    let child;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const fixtureInput = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      const releasePath = join(
        state.root,
        `release-reservation-boundary-${index}`,
      );
      child = spawn(
        process.execPath,
        [
          helper,
          "publish",
          state.repo,
          state.state,
          checkpoint,
          releasePath,
        ],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["pipe", "pipe", "pipe"],
        },
      );
      let inputWriteError;
      child.stdin.on("error", (error) => {
        inputWriteError = error;
      });
      child.stdin.end(JSON.stringify(fixtureInput));
      child.stdout.resume();
      let stderr = "";
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        stderr += chunk;
      });
      const readyPath = `${releasePath}.ready-${child.pid}`;
      const deadline = Date.now() + 15_000;
      while (!existsSync(readyPath)) {
        assert.equal(inputWriteError, undefined, checkpoint);
        assert.equal(child.exitCode, null, `${checkpoint}: ${stderr}`);
        assert.ok(Date.now() < deadline, `${checkpoint}: ${stderr}`);
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
      }
      const terminationSignal = index % 2 === 0 ? "SIGTERM" : "SIGKILL";
      child.kill(terminationSignal);
      const [status, signal] = await once(child, "close");
      child = undefined;
      assert.equal(status, null, checkpoint);
      assert.equal(signal, terminationSignal, checkpoint);

      const confirmationArguments = {
        ...fixtureInput,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      const activeEntries = readdirSync(prepared.active);
      const hasOpenReservation =
        activeEntries.includes(".mutation-slot-03") &&
        !activeEntries.includes(".mutation-close-03");
      if (hasOpenReservation) {
        const confirmation =
          liveProviderFixtureReservationRecoveryConfirmationForTest(
            confirmationArguments,
          );
        const recovered = recoverColimaLiveProviderReservationForTest({
          ...confirmationArguments,
          confirmation,
          testCheckpoint() {},
        });
        if (recovered.phase === "plan") {
          publishColimaLiveProviderReservationForTest(
            fixtureStartAdmissionArguments(state, preparation),
          );
        }
      } else {
        publishColimaLiveProviderReservationForTest(
          fixtureStartAdmissionArguments(state, preparation),
        );
      }

      const finalEntries = readdirSync(prepared.active);
      const completedSequence = finalEntries.includes(".mutation-close-04")
        ? "04"
        : "03";
      assert.equal(
        parse(
          join(prepared.active, `.mutation-close-${completedSequence}`),
        ).disposition,
        "completed",
        checkpoint,
      );
      assertPrivate(
        join(
          prepared.active,
          `.provider-reservation-witness-${completedSequence}`,
        ),
        0o600,
        false,
        1,
      );
      assert.equal(
        existsSync(
          join(
            preparation.providerRoot,
            ".synveda-clean-engine-provider-reservation",
          ),
        ),
        false,
        checkpoint,
      );
      assert.deepEqual(
        finalEntries.filter((name) =>
          name.startsWith(".provider-reservation-stage-"),
        ),
        [],
        checkpoint,
      );
      assert.deepEqual(
        finalEntries.filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
        ["00-plan.json"],
        checkpoint,
      );
      assert.equal(existsSync(join(prepared.active, "environment.json")), false);
      for (const directory of ["evidence", "provider", "registry", "runtime"]) {
        assert.deepEqual(readdirSync(join(prepared.active, directory)), []);
      }
      assert.equal(run(state, "verify").status, 0, checkpoint);
    } finally {
      if (child !== undefined) child.kill("SIGKILL");
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("two state journals sharing one provider root admit one reservation CAS winner", async () => {
  const firstState = fixture();
  const secondState = fixture();
  let preparation;
  const children = [];
  try {
    assert.equal(run(firstState, "plan").status, 0);
    assert.equal(run(secondState, "plan").status, 0);
    const firstActive = activeRun(firstState);
    const secondActive = activeRun(secondState);
    const firstFixtureId = parse(join(firstActive, "candidate.json")).run_id;
    const secondFixtureId = parse(join(secondActive, "candidate.json")).run_id;
    preparation = createCleanEngineColimaLiveObservationFixture({
      fixtureId: firstFixtureId,
    });
    const secondInput = cloneColimaLiveObservationInput(preparation.input);
    secondInput.fixture_id = secondFixtureId;
    secondInput.provider_profile = `synveda-cpr45-${secondFixtureId}`;
    const secondPreparation = {
      input: secondInput,
      observation: buildColimaLiveObservationForTest(
        preparation.requirements,
        secondInput,
      ),
      providerRoot: preparation.providerRoot,
      requirements: preparation.requirements,
    };
    for (const [state, prepared] of [
      [firstState, preparation],
      [secondState, secondPreparation],
    ]) {
      const operationPlan = liveProviderOperationPlan(
        state,
        colimaLiveDigest(colimaLiveBytes(prepared.observation)),
      );
      recordLiveProviderOperationPlanForTest({
        operationPlan,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      publishColimaLiveProviderIntentForTest(
        fixtureIntentArguments(state, prepared),
      );
      publishColimaLiveProviderStartDecisionForTest(
        fixtureStartAdmissionArguments(state, prepared),
      );
    }

    const helper = resolve(
      "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
    );
    const releasePath = join(firstState.root, "release-reservation-writers");
    const launch = (state, prepared) => {
      const privateInput = {
        observation: prepared.observation,
        observationInput: prepared.input,
        requirements: prepared.requirements,
      };
      const child = spawn(
        process.execPath,
        [
          helper,
          "publish",
          state.repo,
          state.state,
          "after-witness-stage",
          releasePath,
        ],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["pipe", "pipe", "pipe"],
        },
      );
      child.stdout.resume();
      const output = { child, inputWriteError: undefined, stderr: "" };
      child.stdin.on("error", (error) => {
        output.inputWriteError = error;
      });
      child.stdin.end(JSON.stringify(privateInput));
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        output.stderr += chunk;
      });
      children.push(output);
      return output;
    };
    const first = launch(firstState, preparation);
    const second = launch(secondState, secondPreparation);
    const deadline = Date.now() + 15_000;
    while (
      readdirSync(firstState.root).filter((name) =>
        name.startsWith(`${basename(releasePath)}.ready-`),
      ).length !== 2
    ) {
      assert.equal(first.inputWriteError, undefined);
      assert.equal(second.inputWriteError, undefined);
      assert.equal(first.child.exitCode, null, first.stderr);
      assert.equal(second.child.exitCode, null, second.stderr);
      assert.ok(Date.now() < deadline, `${first.stderr}${second.stderr}`);
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    writeFileSync(releasePath, "release\n", { mode: 0o600 });
    const results = await Promise.all(
      children.map(
        (output) =>
          new Promise((resolvePromise) => {
            output.child.on("close", (status, signal) =>
              resolvePromise({ signal, status, stderr: output.stderr }),
            );
          }),
      ),
    );
    children.length = 0;
    assert.deepEqual(
      results.map((value) => value.status).sort((left, right) => left - right),
      [0, 73],
      results.map((value) => value.stderr).join("\n"),
    );
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(
      results.find((value) => value.status === 73)?.stderr,
      "live provider reservation marker was already held\n",
    );
    const closes = [firstActive, secondActive].map((active) =>
      parse(join(active, ".mutation-close-03")),
    );
    assert.deepEqual(
      closes.map((close) => close.disposition).sort(),
      ["aborted-before-effect", "completed"],
    );
    assert.equal(
      [firstActive, secondActive].filter((active) =>
        existsSync(join(active, ".provider-reservation-witness-03")),
      ).length,
      1,
    );
    assert.equal(
      existsSync(
        join(
          preparation.providerRoot,
          ".synveda-clean-engine-provider-reservation",
        ),
      ),
      false,
    );
    assert.equal(run(firstState, "verify").status, 0);
    assert.equal(run(secondState, "verify").status, 0);
  } finally {
    for (const { child } of children) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(firstState.root, { recursive: true, force: true });
    rmSync(secondState.root, { recursive: true, force: true });
  }
});

test("reservation authority preserves replacement extra-link and namespace drift", async () => {
  for (const mutation of [
    "marker-replacement",
    "marker-symlink",
    "marker-mode",
    "extra-hard-link",
    "namespace-drift",
  ]) {
    const state = fixture();
    let preparation;
    let child;
    let cleanupMutation = () => {};
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const fixtureInput = {
        observation: preparation.observation,
        observationInput: preparation.input,
        requirements: preparation.requirements,
      };
      const helper = resolve(
        "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
      );
      const releasePath = join(
        state.root,
        `release-reservation-drift-${mutation}`,
      );
      child = spawn(
        process.execPath,
        [
          helper,
          "publish",
          state.repo,
          state.state,
          "before-retirement-observation",
          releasePath,
        ],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["pipe", "pipe", "pipe"],
        },
      );
      let inputWriteError;
      child.stdin.on("error", (error) => {
        inputWriteError = error;
      });
      child.stdin.end(JSON.stringify(fixtureInput));
      child.stdout.resume();
      let stderr = "";
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        stderr += chunk;
      });
      const readyPath = `${releasePath}.ready-${child.pid}`;
      const marker = join(
        preparation.providerRoot,
        ".synveda-clean-engine-provider-reservation",
      );
      const witness = join(
        prepared.active,
        ".provider-reservation-witness-03",
      );
      const deadline = Date.now() + 15_000;
      while (!existsSync(readyPath) || !existsSync(marker) || !existsSync(witness)) {
        assert.equal(inputWriteError, undefined, mutation);
        assert.equal(child.exitCode, null, `${mutation}: ${stderr}`);
        assert.ok(Date.now() < deadline, `${mutation}: ${stderr}`);
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
      }
      if (mutation === "marker-replacement") {
        unlinkSync(marker);
        writeFileSync(marker, "foreign reservation marker\n", { mode: 0o600 });
        cleanupMutation = () => unlinkSync(marker);
      } else if (mutation === "marker-symlink") {
        unlinkSync(marker);
        symlinkSync(witness, marker);
        cleanupMutation = () => unlinkSync(marker);
      } else if (mutation === "marker-mode") {
        unlinkSync(marker);
        writeFileSync(marker, "foreign reservation marker\n", { mode: 0o644 });
        cleanupMutation = () => unlinkSync(marker);
      } else if (mutation === "extra-hard-link") {
        const extra = join(state.root, "foreign-reservation-link");
        linkSync(witness, extra);
        cleanupMutation = () => unlinkSync(extra);
      } else {
        const drift = join(
          preparation.input.environment.COLIMA_HOME,
          "foreign-reservation-drift",
        );
        writeFileSync(drift, "foreign namespace entry\n", { mode: 0o600 });
        cleanupMutation = () => unlinkSync(drift);
      }
      writeFileSync(releasePath, "release\n", { mode: 0o600 });
      const [status, signal] = await once(child, "close");
      child = undefined;
      assert.notEqual(status, 0, mutation);
      assert.equal(signal, null, mutation);
      assert.equal(existsSync(join(prepared.active, ".mutation-close-03")), false);
      assert.equal(existsSync(join(prepared.active, ".mutation-operation-03")), false);

      const confirmationArguments = {
        ...fixtureInput,
        repoRoot: state.repo,
        stateBase: state.state,
      };
      if (mutation !== "namespace-drift") {
        assert.throws(
          () =>
            liveProviderFixtureReservationRecoveryConfirmationForTest(
              confirmationArguments,
            ),
          /(?:reservation|plan run)/u,
        );
      }
      cleanupMutation();
      cleanupMutation = () => {};
      const confirmation =
        liveProviderFixtureReservationRecoveryConfirmationForTest(
          confirmationArguments,
        );
      if (mutation === "namespace-drift") {
        assert.throws(
          () =>
            recoverColimaLiveProviderReservationForTest({
              ...confirmationArguments,
              confirmation,
              testCheckpoint() {},
            }),
          /root observation changed/u,
        );
        assert.equal(existsSync(marker), true);
        assertPrivate(marker, 0o600, false, 2);
        assertPrivate(witness, 0o600, false, 2);
        assert.equal(existsSync(join(prepared.active, ".mutation-close-03")), false);
        assert.equal(run(state, "verify").status, 0, mutation);
        continue;
      }
      const completion = recoverColimaLiveProviderReservationForTest({
        ...confirmationArguments,
        confirmation,
        testCheckpoint() {},
      });
      assert.equal(
        completion.schema,
        COLIMA_LIVE_FIXTURE_PROVIDER_RESERVATION_COMPLETION_SCHEMA,
        mutation,
      );
      assert.equal(existsSync(marker), false, mutation);
      assertPrivate(witness, 0o600, false, 1);
      assert.equal(run(state, "verify").status, 0, mutation);
    } finally {
      if (child !== undefined) child.kill("SIGKILL");
      cleanupMutation();
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("post-decision effect admission refuses root and state drift between samples", () => {
  for (const drift of ["root-appearance", "decision-close-authority"]) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
      preparation = prepared.preparation;
      const closePath = join(prepared.active, ".mutation-close-02");
      let changed = false;
      assert.throws(
        () =>
          observeColimaLiveProviderStartEffectFreshAdmissionForTest(
            fixtureStartAdmissionArguments(
              state,
              preparation,
              (checkpoint) => {
                if (
                  checkpoint !==
                    "after-first-process-start-effect-admission-observation" ||
                  changed
                ) {
                  return;
                }
                changed = true;
                if (drift === "root-appearance") {
                  writePrivateColimaLiveFixtureFile(
                    join(
                      preparation.input.environment.COLIMA_HOME,
                      preparation.input.provider_profile,
                    ),
                    Buffer.from("late foreign collision\n", "utf8"),
                    0o600,
                  );
                } else {
                  const close = parse(closePath);
                  close.authority = "recovery";
                  writeFileSync(closePath, canonicalBytes(close), {
                    mode: 0o600,
                  });
                }
              },
            ),
          ),
        drift === "root-appearance"
          ? /root observation changed/u
          : /mutation close|recovery authority|start decision/u,
      );
      assert.equal(changed, true, drift);
      assert.deepEqual(
        readdirSync(prepared.active).filter((name) =>
          /^\.mutation-slot-03|^\.mutation-close-03|^\.mutation-operation-/u.test(
            name,
          ),
        ),
        [],
        drift,
      );
      for (const directory of ["evidence", "provider", "registry", "runtime"]) {
        assert.deepEqual(readdirSync(join(prepared.active, directory)), [], drift);
      }
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("post-decision effect admission requires current completed state and closed arguments", () => {
  for (const phase of ["plan", "intent"]) {
    const state = fixture();
    let preparation;
    try {
      const prepared =
        phase === "plan"
          ? prepareLiveProviderIntentFixture(state)
          : prepareCompletedLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      assert.throws(
        () =>
          observeColimaLiveProviderStartEffectFreshAdmissionForTest(
            fixtureStartAdmissionArguments(state, preparation),
          ),
        /completed live provider start decision was unavailable/u,
        phase,
      );
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }

  const state = fixture();
  let preparation;
  try {
    const prepared = prepareCompletedLiveProviderStartDecisionFixture(state);
    preparation = prepared.preparation;
    const argumentsValue = fixtureStartAdmissionArguments(state, preparation);
    const before = snapshotTree(state.state);
    assert.throws(
      () =>
        observeColimaLiveProviderStartEffectFreshAdmissionForExecutor({
          observation: preparation.observation,
          observationInput: preparation.input,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /evidence class already differs|completed live provider start decision/u,
    );
    assert.throws(
      () =>
        observeColimaLiveProviderStartEffectFreshAdmissionForTest({
          ...argumentsValue,
          callerSource: {},
        }),
      /pre-effect admission arguments were refused/u,
    );
    assert.deepEqual(snapshotTree(state.state), before);

    rmSync(preparation.root, { recursive: true, force: true });
    preparation = undefined;
    assert.throws(
      () =>
        observeColimaLiveProviderStartEffectFreshAdmissionForTest(
          argumentsValue,
        ),
      /observation|unavailable/u,
    );
    assert.deepEqual(snapshotTree(state.state), before);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the post-decision effect observer has no provider-effect control surface", () => {
  const stateSource = readFileSync(stateTool, "utf8");
  const start = stateSource.indexOf(
    "function completedLiveProviderStartDecisionCoreSnapshot",
  );
  const end = stateSource.indexOf(
    "function callLiveProviderEffectCheckpoint",
    start,
  );
  assert.ok(start >= 0 && end > start);
  const observerSource = stateSource.slice(start, end);
  assert.doesNotMatch(
    observerSource,
    /\b(?:acquire|append|chmod|chown|close|copyFile|execute|finalize|fork|kill|launch|link|mkdir|publish|reconcile|recover|rename|retire|rm|rmdir|settle|signal|spawn|symlink|truncate|unlink|write)[A-Za-z0-9_]*\s*\(/u,
  );
  assert.doesNotMatch(
    observerSource,
    /\b(?:inspectControlledBackgroundProvider|inspectControlledBackgroundProviderPrefix|launchControlledBackgroundProviderWithAuthorityGate|mutationOperationFileName|planControlledBackgroundProviderCreateWithAuthorityGate|planControlledBackgroundProviderOperation|providerAdapterReceipt|providerProcessBytes|providerProcessDigest|receiptFileName|retireControlledBackgroundProviderWithAuthorityGate)\b/u,
  );
  assert.doesNotMatch(observerSource, /authorizeColimaLiveProviderProcessStartEffect/u);
  const expectedEffectExports = [
    "liveProviderFixtureEffectRecoveryConfirmationForTest",
    "observeColimaLivePreEffectAdmissionForExecutor",
    "observeColimaLivePreEffectAdmissionForTest",
    "observeColimaLiveProviderStartEffectFreshAdmissionForExecutor",
    "observeColimaLiveProviderStartEffectFreshAdmissionForTest",
    "publishColimaLiveProviderEffectAttemptFenceForTest",
    "publishColimaLiveProviderEffectConclusiveNotCreatedForTest",
    "publishColimaLiveProviderEffectPreAttemptForTest",
    "recoverColimaLiveProviderEffectForTest",
  ];
  const effectExports = Object.keys(cleanEngineStateModule)
    .filter((name) => name.includes("Effect"))
    .sort();
  assert.deepEqual(effectExports, expectedEffectExports);
  for (const name of expectedEffectExports) {
    assert.match(stateSource, new RegExp(`export function ${name}\\b`, "u"));
  }
  const effectMutators = effectExports.filter((name) =>
    /^(?:publish|recover)/u.test(name),
  );
  assert.deepEqual(effectMutators, [
    "publishColimaLiveProviderEffectAttemptFenceForTest",
    "publishColimaLiveProviderEffectConclusiveNotCreatedForTest",
    "publishColimaLiveProviderEffectPreAttemptForTest",
    "recoverColimaLiveProviderEffectForTest",
  ]);

  const trackedSources = command("git", [
    "ls-files",
    "-z",
    "--cached",
    "--others",
    "--exclude-standard",
    "--",
    "adapters",
    "console",
    "crates",
    "demos",
    "deploy",
    "policies",
    "scripts",
  ]);
  assert.equal(trackedSources.status, 0, trackedSources.stderr);
  const effectMutatorConsumers = trackedSources.stdout
    .split("\0")
    .filter((path) => path.length > 0)
    .filter((path) => {
      const bytes = readFileSync(path);
      if (bytes.includes(0)) return false;
      let source;
      try {
        source = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      } catch {
        return false;
      }
      return effectMutators.some((name) => source.includes(name));
    })
    .sort();
  assert.deepEqual(effectMutatorConsumers, [
    "deploy/compose/scripts/clean-engine-state.mjs",
    "scripts/clean-engine-state.test.mjs",
    "scripts/fixtures/clean-engine-live-provider-effect-process.mjs",
  ]);

  const malformedInputSentinel = "must-not-escape-private-fixture-input";
  for (const path of [
    "scripts/fixtures/clean-engine-live-provider-effect-process.mjs",
    "scripts/fixtures/clean-engine-live-provider-reservation-process.mjs",
  ]) {
    const helper = readFileSync(path, "utf8");
    assert.match(
      helper,
      /const \[action, repoRoot, stateBase, checkpoint, releasePath\] = values;/u,
    );
    assert.match(helper, /values\.length !== 5/u);
    assert.match(helper, /readFileSync\(0, "utf8"\)/u);
    assert.doesNotMatch(
      helper,
      /const \[[^\]]*\bserialized\b[^\]]*\] = values;/u,
    );
    const malformed = command(
      process.execPath,
      [
        resolve(path),
        "publish",
        "unused-repository-root",
        "unused-state-root",
        "unused-checkpoint",
        "unused-release-path",
      ],
      {
        input: `{"observationInput":{"binding_key":"${malformedInputSentinel}`,
      },
    );
    assert.equal(malformed.status, 64);
    assert.equal(malformed.stdout, "");
    assert.equal(malformed.stderr, "fixture input was malformed\n");
    assert.doesNotMatch(malformed.stderr, new RegExp(malformedInputSentinel, "u"));
  }

  const effectPublicationStart = stateSource.indexOf(
    "function callLiveProviderEffectCheckpoint",
  );
  const effectPublicationEnd = stateSource.indexOf(
    "function callLiveProviderReservationCheckpoint",
    effectPublicationStart,
  );
  const effectRecoveryStart = stateSource.indexOf(
    "function observeRetiredLiveProviderEffectMarker",
  );
  const effectRecoveryEnd = stateSource.indexOf(
    "function recoverColimaLiveProviderReservation(",
    effectRecoveryStart,
  );
  assert.ok(
    effectPublicationStart >= 0 && effectPublicationEnd > effectPublicationStart,
  );
  assert.ok(effectRecoveryStart >= 0 && effectRecoveryEnd > effectRecoveryStart);
  const effectPublicationSource = stateSource.slice(
    effectPublicationStart,
    effectPublicationEnd,
  );
  const effectRecoverySource = stateSource.slice(
    effectRecoveryStart,
    effectRecoveryEnd,
  );
  assert.equal(
    [
      ...effectPublicationSource.matchAll(
        /executeColimaLiveProviderEffectFixtureConclusiveAdapter\s*\(/gu,
      ),
    ].length,
    1,
  );
  assert.doesNotMatch(
    effectRecoverySource,
    /executeColimaLiveProviderEffectFixtureConclusiveAdapter\s*\(/u,
  );
  const reproofSlices = [
    ["function publishLiveProviderEffectEvent", "function retireLiveProviderEffectMarker"],
    ["function retireLiveProviderEffectMarker", "function liveProviderEffectPreAttemptCompletionEvidence"],
    ["const closed = closeMutationLease(", "export function publishColimaLiveProviderEffectPreAttemptForTest"],
    ["function validateLiveProviderEffectRecoveryPhysicalState", "export function liveProviderFixtureEffectRecoveryConfirmationForTest"],
  ].map(([startMarker, endMarker]) => {
    const start = stateSource.indexOf(startMarker, effectPublicationStart);
    const end = stateSource.indexOf(endMarker, start);
    assert.ok(start >= 0 && end > start, startMarker);
    return stateSource.slice(start, end);
  });
  for (const source of reproofSlices) {
    assert.match(source, /reproveLiveProviderEffectFixtureBlueprint\s*\(/u);
  }
  const boundSyncStart = stateSource.indexOf(
    "function syncLiveProviderEffectBoundDirectory",
  );
  const boundSyncEnd = stateSource.indexOf(
    "function liveProviderEffectNamespaceTarget",
    boundSyncStart,
  );
  assert.ok(boundSyncStart >= 0 && boundSyncEnd > boundSyncStart);
  const boundSyncSource = stateSource.slice(boundSyncStart, boundSyncEnd);
  assert.match(boundSyncSource, /constants\.O_NOFOLLOW/u);
  assert.match(boundSyncSource, /exactFstat\(descriptor\)/u);
  assert.match(boundSyncSource, /sameMetadata\(openedBeforeSync, openedAfterSync\)/u);
  assert.match(boundSyncSource, /sameMetadata\(openedAfterSync, namedAfterSync\)/u);
  for (const source of [
    effectPublicationSource,
    effectRecoverySource,
  ]) {
    assert.doesNotMatch(
      source,
      /\b(?:authorizeColimaLiveProviderProcessStartEffect|executeProviderCreateForExecutor|launchControlledBackgroundProviderWithAuthorityGate|retireControlledBackgroundProviderWithAuthorityGate)\b/u,
    );
    assert.doesNotMatch(
      source,
      /\b(?:execFile|execFileSync|execSync|fork|spawn|spawnSync)\s*\(/u,
    );
  }
  const processHelper = readFileSync(
    resolve("scripts/fixtures/clean-engine-live-provider-effect-process.mjs"),
    "utf8",
  );
  assert.match(
    processHelper,
    /new Set\(\["attempt", "collision", "conclusive", "publish", "recover"\]\)/u,
  );
  assert.match(
    processHelper,
    /action === "conclusive"[\s\S]*publishColimaLiveProviderEffectConclusiveNotCreatedForTest/u,
  );

  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /process-start-effect/u);

  for (const path of [
    "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
    "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
    "../deploy/compose/scripts/clean-engine-receipts.mjs",
  ]) {
    const consumer = readFileSync(new URL(path, import.meta.url), "utf8");
    assert.doesNotMatch(
      consumer,
      /clean-engine-live-provider-process-start/u,
    );
  }
});

test("intent publication reasserts its evidence class after a competing abort", () => {
  const state = fixture();
  let preparation;
  try {
    const prepared = prepareLiveProviderIntentFixture(state);
    preparation = prepared.preparation;
    const productionPlan = productionIntentPublicationPlanForFixture(
      state,
      preparation,
      prepared.operationPlan,
    );
    let injected = false;
    assert.throws(
      () =>
        publishColimaLiveProviderIntentForTest(
          fixtureIntentArguments(state, preparation, (checkpoint) => {
            if (checkpoint !== "after-initial-admission" || injected) return;
            stageAbortedProductionLiveProviderIntent(state, productionPlan);
            injected = true;
          }),
        ),
      /live provider intent evidence classes were mixed/u,
    );
    assert.equal(injected, true);

    const active = prepared.active;
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\.mutation-slot-/u.test(name)),
      [".mutation-slot-00", ".mutation-slot-01"],
    );
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\.mutation-close-/u.test(name)),
      [".mutation-close-00", ".mutation-close-01"],
    );
    assert.equal(
      readdirSync(active).some((name) => name.startsWith(".mutation-stage-")),
      false,
    );
    assert.equal(
      parse(join(active, ".mutation-slot-01")).operation_kind,
      COLIMA_LIVE_PROVIDER_INTENT_OPERATION_KIND,
    );
    assert.equal(
      parse(join(active, ".mutation-close-01")).disposition,
      "aborted-before-effect",
    );
    assert.equal(run(state, "status").status, 0);
    assert.equal(run(state, "verify").status, 0);

    const beforeRetry = snapshotTree(state.state);
    assert.throws(
      () =>
        publishColimaLiveProviderIntentForTest(
          fixtureIntentArguments(state, preparation),
        ),
      /live provider intent evidence classes were mixed/u,
    );
    assert.deepEqual(snapshotTree(state.state), beforeRetry);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("intent journal refuses rehashed semantic drift and invalid successors", () => {
  const cases = [
    {
      name: "rehashed completed-plan projection drift",
      publishIntent: true,
      mutate(state, active) {
        const slotPath = join(active, ".mutation-slot-01");
        const closePath = join(active, ".mutation-close-01");
        const slot = parse(slotPath);
        const close = parse(closePath);
        const publicationPlan = slot.operation_plan;
        const projection = publicationPlan.admission.completion_projection;
        projection.plan_close_sha256 = "f".repeat(64);
        publicationPlan.admission.intent_candidate
          .completed_plan_projection_sha256 = sha256(canonicalBytes(projection));
        publicationPlan.admission.pre_effect_prefix.intent_candidate_sha256 =
          sha256(canonicalBytes(publicationPlan.admission.intent_candidate));
        publicationPlan.admission_sha256 = sha256(
          canonicalBytes(publicationPlan.admission),
        );
        const slotBytes = canonicalBytes(slot);
        writeFileSync(slotPath, slotBytes, { mode: 0o600 });
        close.authority_sha256 = sha256(slotBytes);
        close.operation_plan_sha256 = sha256(canonicalBytes(publicationPlan));
        close.slot_sha256 = sha256(slotBytes);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      stderr: "clean-engine: live provider intent journal was refused\n",
    },
    {
      name: "rehashed embedded provider-plan drift",
      publishIntent: true,
      mutate(state, active) {
        const slotPath = join(active, ".mutation-slot-01");
        const closePath = join(active, ".mutation-close-01");
        const slot = parse(slotPath);
        const close = parse(closePath);
        const publicationPlan = slot.operation_plan;
        publicationPlan.provider_operation_plan.source_candidate_sha256 =
          "f".repeat(64);
        publicationPlan.provider_operation_plan_sha256 = sha256(
          canonicalBytes(publicationPlan.provider_operation_plan),
        );
        const projection = publicationPlan.admission.completion_projection;
        projection.operation_plan_sha256 =
          publicationPlan.provider_operation_plan_sha256;
        publicationPlan.admission.intent_candidate
          .completed_plan_projection_sha256 = sha256(canonicalBytes(projection));
        publicationPlan.admission.pre_effect_prefix.intent_candidate_sha256 =
          sha256(canonicalBytes(publicationPlan.admission.intent_candidate));
        publicationPlan.admission_sha256 = sha256(
          canonicalBytes(publicationPlan.admission),
        );
        const slotBytes = canonicalBytes(slot);
        writeFileSync(slotPath, slotBytes, { mode: 0o600 });
        close.authority_sha256 = sha256(slotBytes);
        close.operation_plan_sha256 = sha256(canonicalBytes(publicationPlan));
        close.slot_sha256 = sha256(slotBytes);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      stderr: "clean-engine: live provider intent journal was refused\n",
    },
    {
      name: "second completed intent",
      publishIntent: true,
      mutate(state, active) {
        const priorCloseBytes = readFileSync(join(active, ".mutation-close-01"));
        const slot = parse(join(active, ".mutation-slot-01"));
        slot.journal_sequence = 2;
        slot.nonce = "e".repeat(32);
        slot.previous_close_sha256 = sha256(priorCloseBytes);
        const slotBytes = canonicalBytes(slot);
        writeFileSync(join(active, ".mutation-slot-02"), slotBytes, {
          mode: 0o600,
        });
        const close = parse(join(active, ".mutation-close-01"));
        close.authority_sha256 = sha256(slotBytes);
        close.slot_sequence = 2;
        close.slot_sha256 = sha256(slotBytes);
        writeFileSync(join(active, ".mutation-close-02"), canonicalBytes(close), {
          mode: 0o600,
        });
      },
      stderr: "clean-engine: live provider intent journal was refused\n",
    },
    {
      name: "non-intent successor",
      publishIntent: false,
      mutate(state, active) {
        const successor = mutationLease(state, {
          action: "provider-create",
          intentReceiptSha256: "f".repeat(64),
          journalSequence: 1,
          previousCloseSha256: sha256(
            readFileSync(join(active, ".mutation-close-00")),
          ),
        });
        writeFileSync(
          join(active, ".mutation-slot-01"),
          canonicalBytes(successor),
          { mode: 0o600 },
        );
      },
      stderr: "clean-engine: live provider plan successor was refused\n",
    },
    {
      name: "intent operation evidence",
      publishIntent: true,
      mutate(state, active) {
        const closePath = join(active, ".mutation-close-01");
        const close = parse(closePath);
        close.operation_evidence_sha256 = "f".repeat(64);
        writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      },
      stderr: "clean-engine: mutation close receipt binding was refused\n",
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      const prepared = prepareLiveProviderIntentFixture(state);
      preparation = prepared.preparation;
      if (value.publishIntent) {
        publishColimaLiveProviderIntentForTest(
          fixtureIntentArguments(state, preparation),
        );
      }
      value.mutate(state, prepared.active);
      const refused = run(state, "status");
      assert.equal(refused.status, 78, value.name);
      assert.equal(refused.stderr, value.stderr, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("intent publication refuses collisions at every journal boundary and retries", () => {
  const cases = [
    {
      checkpoint: undefined,
      name: "initial",
      prepare(collisionPath) {
        writePrivateColimaLiveFixtureFile(
          collisionPath,
          Buffer.from("initial collision\n", "utf8"),
          0o600,
        );
      },
      permanentSlot: false,
    },
    {
      checkpoint:
        "before-slot-link:after-first-admission-observation",
      name: "slot-prelink",
      prepare() {},
      permanentSlot: false,
    },
    {
      checkpoint:
        "after-slot-acquisition:after-first-admission-observation",
      name: "post-acquisition-observation",
      prepare() {},
      permanentSlot: true,
    },
    {
      checkpoint: "after-slot-acquisition",
      name: "before-close-observation",
      prepare() {},
      permanentSlot: true,
    },
    {
      checkpoint:
        "before-close-link:after-first-admission-observation",
      name: "close-prelink",
      prepare() {},
      permanentSlot: true,
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      ({ preparation } = prepareLiveProviderIntentFixture(state));
      const active = activeRun(state);
      const collisionPath = join(
        preparation.input.environment.COLIMA_HOME,
        preparation.input.provider_profile,
      );
      value.prepare(collisionPath);
      let injected = false;
      const beforeState = snapshotTree(state.state);
      const beforeEntries = readdirSync(active).sort();
      assert.throws(
        () =>
          publishColimaLiveProviderIntentForTest(
            fixtureIntentArguments(state, preparation, (checkpoint) => {
              if (checkpoint === value.checkpoint && !injected) {
                injected = true;
                writePrivateColimaLiveFixtureFile(
                  collisionPath,
                  Buffer.from(`${value.name} collision\n`, "utf8"),
                  0o600,
                );
              }
            }),
          ),
        (error) => {
          assert.equal(error.exitStatus, 73, value.name);
          return true;
        },
      );
      assert.equal(
        readdirSync(active).some((name) => name.startsWith(".mutation-stage-")),
        false,
        value.name,
      );
      assert.equal(
        existsSync(join(active, ".mutation-slot-01")),
        value.permanentSlot,
        value.name,
      );
      assert.equal(
        existsSync(join(active, ".mutation-close-01")),
        value.permanentSlot,
        value.name,
      );
      if (value.permanentSlot) {
        assert.equal(
          parse(join(active, ".mutation-close-01")).disposition,
          "aborted-before-effect",
          value.name,
        );
      } else {
        assert.deepEqual(readdirSync(active).sort(), beforeEntries, value.name);
        if (value.name === "initial") {
          assert.deepEqual(snapshotTree(state.state), beforeState, value.name);
        }
      }
      assert.deepEqual(
        readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
        ["00-plan.json"],
        value.name,
      );
      assert.equal(existsSync(join(active, ".mutation-operation-01")), false);
      if (existsSync(collisionPath)) unlinkSync(collisionPath);
      const completed = publishColimaLiveProviderIntentForTest(
        fixtureIntentArguments(state, preparation),
      );
      assert.equal(
        completed.schema,
        COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
        value.name,
      );
      const completedSequence = value.permanentSlot ? "02" : "01";
      assert.equal(
        parse(join(active, `.mutation-close-${completedSequence}`)).disposition,
        "completed",
        value.name,
      );
      assert.equal(run(state, "verify").status, 0, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("catchable post-link fixture failures leave a recoverable intent journal", () => {
  for (const value of [
    { checkpoint: "after-slot-link", disposition: "aborted-before-effect" },
    { checkpoint: "after-close-link", disposition: "completed" },
  ]) {
    const state = fixture();
    let preparation;
    try {
      ({ preparation } = prepareLiveProviderIntentFixture(state));
      const active = activeRun(state);
      assert.throws(
        () =>
          publishColimaLiveProviderIntentForTest(
            fixtureIntentArguments(state, preparation, (checkpoint) => {
              if (checkpoint === value.checkpoint) {
                throw new Error(`fixture checkpoint failed at ${checkpoint}`);
              }
            }),
          ),
        new RegExp(`fixture checkpoint failed at ${value.checkpoint}`, "u"),
      );
      assert.equal(
        readdirSync(active).some((name) => name.startsWith(".mutation-stage-")),
        false,
        value.checkpoint,
      );
      assert.equal(
        parse(join(active, ".mutation-close-01")).disposition,
        value.disposition,
        value.checkpoint,
      );
      const completed = publishColimaLiveProviderIntentForTest(
        fixtureIntentArguments(state, preparation),
      );
      assert.equal(
        completed.schema,
        COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
      );
      assert.equal(
        parse(
          join(
            active,
            value.disposition === "completed"
              ? ".mutation-close-01"
              : ".mutation-close-02",
          ),
        ).disposition,
        "completed",
      );
      assert.equal(run(state, "verify").status, 0, value.checkpoint);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("an abandoned inert intent is recovered only by an all-zero abort", async () => {
  const state = fixture();
  let preparation;
  let child;
  try {
    ({ preparation } = prepareLiveProviderIntentFixture(state));
    const active = activeRun(state);
    const releasePath = join(state.root, "release-intent-writer");
    const helper = resolve(
      "scripts/fixtures/race-clean-engine-live-provider-intent.mjs",
    );
    child = spawn(
      process.execPath,
      [
        helper,
        state.repo,
        state.state,
        JSON.stringify({
          observation: preparation.observation,
          observationInput: preparation.input,
          requirements: preparation.requirements,
        }),
        "after-slot-acquisition",
        releasePath,
      ],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let childStderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      childStderr += chunk;
    });
    child.stdout.resume();
    const slotPath = join(active, ".mutation-slot-01");
    const deadline = Date.now() + 8_000;
    while (true) {
      try {
        const metadata = lstatSync(slotPath, { bigint: true });
        if (
          metadata.isFile() &&
          metadata.nlink === 1n &&
          parse(slotPath).action === "provider-intent" &&
          !readdirSync(active).some((name) =>
            name.startsWith(".mutation-stage-"),
          )
        ) {
          break;
        }
      } catch (error) {
        if (error?.code !== "ENOENT") throw error;
      }
      assert.ok(
        Date.now() < deadline,
        `timed out waiting for intent slot: ${childStderr}`,
      );
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    child = undefined;
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");

    const beforeConfirmation = snapshotTree(state.state);
    const confirmation = liveProviderIntentRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.deepEqual(snapshotTree(state.state), beforeConfirmation);
    assert.throws(
      () =>
        recoverLiveProviderIntentForExecutor({
          confirmation: `${confirmation}-wrong`,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      (error) => {
        assert.equal(error.exitStatus, 64);
        return true;
      },
    );
    assert.equal(existsSync(join(active, ".mutation-recovery-01-00")), false);
    assert.throws(
      () =>
        recoverProviderCreateForExecutor({
          adapter: fakeProviderAdapter(),
          confirmation,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /matching dedicated executor/u,
    );

    const recovered = recoverLiveProviderIntentForExecutor({
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "plan");
    const recovery = parse(join(active, ".mutation-recovery-01-00"));
    assert.equal(recovery.schema, "synveda.clean-engine.mutation-recovery.v6");
    assert.equal(recovery.action, "provider-intent");
    assert.deepEqual(
      {
        disposition: recovery.observed_effect_disposition,
        effect: recovery.observed_effect_name,
        evidenceHead: recovery.observed_evidence_head_sha256,
        evidencePrefix: recovery.observed_evidence_prefix_sha256,
        evidenceStage: recovery.observed_evidence_stage,
        residual: recovery.observed_residual_sha256,
        settlement: recovery.observed_settlement_sha256,
      },
      {
        disposition: "not-reached",
        effect: "none",
        evidenceHead: "0".repeat(64),
        evidencePrefix: "0".repeat(64),
        evidenceStage: "none",
        residual: "0".repeat(64),
        settlement: "0".repeat(64),
      },
    );
    const recoveryPath = join(active, ".mutation-recovery-01-00");
    const closePath = join(active, ".mutation-close-01");
    const recoveryBytes = readFileSync(recoveryPath);
    const closeBytes = readFileSync(closePath);
    for (const [field, replacement] of [
      ["observed_effect_disposition", "pending"],
      ["observed_effect_name", "provider-create"],
      ["observed_evidence_head_sha256", "f".repeat(64)],
      ["observed_evidence_prefix_sha256", "f".repeat(64)],
      ["observed_evidence_stage", "root-created"],
      ["observed_residual_sha256", "f".repeat(64)],
      ["observed_settlement_sha256", "f".repeat(64)],
    ]) {
      const changedRecovery = { ...recovery, [field]: replacement };
      const changedRecoveryBytes = canonicalBytes(changedRecovery);
      const changedClose = {
        ...JSON.parse(closeBytes.toString("utf8")),
        authority_sha256: sha256(changedRecoveryBytes),
      };
      writeFileSync(recoveryPath, changedRecoveryBytes, { mode: 0o600 });
      writeFileSync(closePath, canonicalBytes(changedClose), { mode: 0o600 });
      const refused = run(state, "status");
      assert.equal(refused.status, 78, field);
      assert.equal(
        refused.stderr,
        "clean-engine: deterministic provider recovery observation was refused\n",
        field,
      );
      writeFileSync(recoveryPath, recoveryBytes, { mode: 0o600 });
      writeFileSync(closePath, closeBytes, { mode: 0o600 });
    }
    assert.equal(
      parse(join(active, ".mutation-close-01")).disposition,
      "aborted-before-effect",
    );
    assert.equal(existsSync(join(active, ".mutation-operation-01")), false);
    const completed = publishColimaLiveProviderIntentForTest(
      fixtureIntentArguments(state, preparation),
    );
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
    );
    assert.equal(
      parse(join(active, ".mutation-close-02")).disposition,
      "completed",
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (child !== undefined) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("two intent writers tolerate competing stages and one journal CAS wins", async () => {
  const state = fixture();
  let preparation;
  const children = [];
  try {
    ({ preparation } = prepareLiveProviderIntentFixture(state));
    const active = activeRun(state);
    const helper = resolve(
      "scripts/fixtures/race-clean-engine-live-provider-intent.mjs",
    );
    const releasePath = join(state.root, "release-intent-writers");
    const preStageReleasePath = `${releasePath}-pre-stage`;
    const serialized = JSON.stringify({
      observation: preparation.observation,
      observationInput: preparation.input,
      requirements: preparation.requirements,
    });
    const launch = () => {
      const child = spawn(
        process.execPath,
        [
          helper,
          state.repo,
          state.state,
          serialized,
          "after-slot-authority-reassertion",
          releasePath,
          preStageReleasePath,
        ],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      const output = { child, stderr: "" };
      child.stdout.resume();
      child.stderr.setEncoding("utf8");
      child.stderr.on("data", (chunk) => {
        output.stderr += chunk;
      });
      children.push(output);
      return output;
    };
    const first = launch();
    const second = launch();
    const waitForBoth = async (path, label) => {
      const deadline = Date.now() + 8_000;
      while (
        readdirSync(state.root).filter((name) =>
          name.startsWith(`${basename(path)}.ready-`),
        ).length !== 2
      ) {
        assert.ok(
          Date.now() < deadline,
          `timed out waiting for ${label}: ${first.stderr}${second.stderr}`,
        );
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
      }
    };
    await waitForBoth(preStageReleasePath, "pre-stage intent writers");
    writeFileSync(preStageReleasePath, "release\n", { mode: 0o600 });
    await waitForBoth(releasePath, "competing intent stages");
    assert.equal(
      readdirSync(active).filter((name) => name.startsWith(".mutation-stage-"))
        .length,
      2,
    );
    writeFileSync(releasePath, "release\n", { mode: 0o600 });
    const results = await Promise.all(
      children.map((output) =>
        new Promise((resolvePromise) => {
          output.child.on("close", (status, signal) =>
            resolvePromise({ signal, status, stderr: output.stderr }),
          );
        }),
      ),
    );
    children.length = 0;
    assert.deepEqual(
      results.map((value) => value.status).sort((left, right) => left - right),
      [0, 73],
      results.map((value) => value.stderr).join("\n"),
    );
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(
      results.find((value) => value.status === 73)?.stderr,
      "another clean-engine mutation is active\n",
    );
    assert.equal(
      readdirSync(active).some((name) => name.startsWith(".mutation-stage-")),
      false,
    );
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\.mutation-slot-/u.test(name)),
      [".mutation-slot-00", ".mutation-slot-01"],
    );
    assert.deepEqual(
      readdirSync(active).filter((name) => /^\.mutation-close-/u.test(name)),
      [".mutation-close-00", ".mutation-close-01"],
    );
    assert.equal(
      parse(join(active, ".mutation-close-01")).disposition,
      "completed",
    );
    assert.equal(existsSync(join(active, ".mutation-operation-01")), false);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    for (const { child } of children) child.kill("SIGKILL");
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("source drift after intent acquisition aborts without effect and can retry", () => {
  const state = fixture();
  let preparation;
  try {
    ({ preparation } = prepareLiveProviderIntentFixture(state));
    const active = activeRun(state);
    let changed = false;
    assert.throws(
      () =>
        publishColimaLiveProviderIntentForTest(
          fixtureIntentArguments(state, preparation, (checkpoint) => {
            if (checkpoint === "after-slot-acquisition" && !changed) {
              changed = true;
              writeFileSync(
                join(state.repo, "source.txt"),
                "source drift during intent publication\n",
              );
            }
          }),
        ),
      /source (?:closure changed|worktree is not clean)/u,
    );
    assert.equal(changed, true);
    assert.equal(
      parse(join(active, ".mutation-close-01")).disposition,
      "aborted-before-effect",
    );
    assert.equal(existsSync(join(active, ".mutation-operation-01")), false);
    assert.deepEqual(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
      ["00-plan.json"],
    );
    for (const directory of ["evidence", "provider", "registry", "runtime"]) {
      assert.deepEqual(readdirSync(join(active, directory)), []);
    }
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
    const completed = publishColimaLiveProviderIntentForTest(
      fixtureIntentArguments(state, preparation),
    );
    assert.equal(
      completed.schema,
      COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
    );
    assert.equal(
      parse(join(active, ".mutation-close-02")).disposition,
      "completed",
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("intent crash boundaries reconcile without inventing provider effects", async () => {
  const cases = [
    {
      checkpoint: "before-slot-link:after-first-admission-observation",
      disposition: "stage-only",
      ready(active) {
        return (
          !existsSync(join(active, ".mutation-slot-01")) &&
          readdirSync(active).filter((name) =>
            name.startsWith(".mutation-stage-"),
          ).length === 1
        );
      },
    },
    {
      checkpoint: "after-slot-link",
      disposition: "open",
      ready(active) {
        const slot = join(active, ".mutation-slot-01");
        const stageName = readdirSync(active).find((name) =>
          name.startsWith(".mutation-stage-"),
        );
        if (!existsSync(slot) || stageName === undefined) return false;
        const stage = join(active, stageName);
        return (
          lstatSync(slot).nlink === 2 &&
          lstatSync(stage).nlink === 2 &&
          lstatSync(slot).ino === lstatSync(stage).ino
        );
      },
    },
    {
      checkpoint: "before-close-link:after-first-admission-observation",
      disposition: "open",
      ready(active) {
        return (
          existsSync(join(active, ".mutation-slot-01")) &&
          !existsSync(join(active, ".mutation-close-01")) &&
          readdirSync(active).some((name) => {
            if (!name.startsWith(".mutation-stage-")) return false;
            return lstatSync(join(active, name)).nlink === 1;
          })
        );
      },
    },
    {
      checkpoint: "after-close-link",
      disposition: "completed",
      ready(active) {
        const close = join(active, ".mutation-close-01");
        const stageName = readdirSync(active).find((name) =>
          name.startsWith(".mutation-stage-"),
        );
        if (!existsSync(close) || stageName === undefined) return false;
        const stage = join(active, stageName);
        return (
          lstatSync(close).nlink === 2 &&
          lstatSync(stage).nlink === 2 &&
          lstatSync(close).ino === lstatSync(stage).ino
        );
      },
    },
  ];

  for (const terminationSignal of ["SIGTERM", "SIGKILL"]) {
    for (const value of cases) {
      const state = fixture();
      let preparation;
      let child;
      try {
        ({ preparation } = prepareLiveProviderIntentFixture(state));
        const active = activeRun(state);
        const helper = resolve(
          "scripts/fixtures/race-clean-engine-live-provider-intent.mjs",
        );
        const releasePath = join(
          state.root,
          `release-${terminationSignal}-${value.disposition}`,
        );
        child = spawn(
          process.execPath,
          [
            helper,
            state.repo,
            state.state,
            JSON.stringify({
              observation: preparation.observation,
              observationInput: preparation.input,
              requirements: preparation.requirements,
            }),
            value.checkpoint,
            releasePath,
          ],
          {
            env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
            stdio: ["ignore", "pipe", "pipe"],
          },
        );
        let stderr = "";
        child.stdout.resume();
        child.stderr.setEncoding("utf8");
        child.stderr.on("data", (chunk) => {
          stderr += chunk;
        });
        const readyPath = `${releasePath}.ready-${child.pid}`;
        const deadline = Date.now() + 8_000;
        while (!existsSync(readyPath) || !value.ready(active)) {
          assert.equal(child.exitCode, null, stderr);
          assert.ok(
            Date.now() < deadline,
            `timed out at ${value.checkpoint}: ${stderr}`,
          );
          await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
        }
        child.kill(terminationSignal);
        const [status, signal] = await once(child, "close");
        child = undefined;
        assert.equal(status, null, value.checkpoint);
        assert.equal(signal, terminationSignal, value.checkpoint);

        if (value.disposition === "stage-only") {
          assert.equal(existsSync(join(active, ".mutation-slot-01")), false);
        } else if (value.disposition === "open") {
          const confirmation =
            liveProviderIntentRecoveryConfirmationForExecutor({
              repoRoot: state.repo,
              stateBase: state.state,
            });
          recoverLiveProviderIntentForExecutor({
            confirmation,
            repoRoot: state.repo,
            stateBase: state.state,
          });
          assert.equal(
            parse(join(active, ".mutation-close-01")).disposition,
            "aborted-before-effect",
            value.checkpoint,
          );
        }
        const completed = publishColimaLiveProviderIntentForTest(
          fixtureIntentArguments(state, preparation),
        );
        assert.equal(
          completed.schema,
          COLIMA_LIVE_FIXTURE_PROVIDER_INTENT_COMPLETION_SCHEMA,
          value.checkpoint,
        );
        assert.equal(
          readdirSync(active).some((name) =>
            name.startsWith(".mutation-stage-"),
          ),
          false,
          value.checkpoint,
        );
        assert.deepEqual(
          readdirSync(active).filter((name) =>
            /^[0-9]{2}-.*\.json$/u.test(name),
          ),
          ["00-plan.json"],
          value.checkpoint,
        );
        assert.equal(
          readdirSync(active).some((name) =>
            name.startsWith(".mutation-operation-"),
          ),
          false,
          value.checkpoint,
        );
        for (const directory of [
          "evidence",
          "provider",
          "registry",
          "runtime",
        ]) {
          assert.deepEqual(
            readdirSync(join(active, directory)),
            [],
            value.checkpoint,
          );
        }
        assert.equal(run(state, "verify").status, 0, value.checkpoint);
      } finally {
        if (child !== undefined) child.kill("SIGKILL");
        if (preparation !== undefined) {
          rmSync(preparation.root, { recursive: true, force: true });
        }
        rmSync(state.root, { recursive: true, force: true });
      }
    }
  }
});

test("fixture admission classifies a collision without constructing intent", () => {
  const state = fixture();
  let preparation;
  try {
    assert.equal(run(state, "plan").status, 0);
    const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
    preparation = createCleanEngineColimaLiveObservationFixture({ fixtureId });
    const operationPlan = liveProviderOperationPlan(
      state,
      colimaLiveDigest(colimaLiveBytes(preparation.observation)),
    );
    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const collisionPath = join(
      preparation.input.environment.COLIMA_HOME,
      preparation.input.provider_profile,
    );
    writePrivateColimaLiveFixtureFile(
      collisionPath,
      Buffer.from("foreign collision\n", "utf8"),
      0o600,
    );
    const stateBefore = snapshotTree(state.root);
    const preparationBefore = snapshotTree(preparation.root);
    const result = observeColimaLivePreEffectAdmissionForTest({
      observation: preparation.observation,
      observationInput: preparation.input,
      repoRoot: state.repo,
      requirements: preparation.requirements,
      stateBase: state.state,
      testCheckpoint() {},
    });
    assert.equal(result.root_observation.root_set_disposition, "foreign-collision");
    assert.equal(result.intent_candidate, null);
    assert.equal(result.pre_effect_prefix, null);
    assertRecursivelyFrozen(result);
    assert.deepEqual(snapshotTree(state.root), stateBefore);
    assert.deepEqual(snapshotTree(preparation.root), preparationBefore);
  } finally {
    if (preparation !== undefined) {
      rmSync(preparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture admission refuses crossed observations before touching their roots", () => {
  const state = fixture();
  let ownPreparation;
  let foreignPreparation;
  try {
    assert.equal(run(state, "plan").status, 0);
    const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
    ownPreparation = createCleanEngineColimaLiveObservationFixture({ fixtureId });
    foreignPreparation = createCleanEngineColimaLiveObservationFixture({
      fixtureId: "f".repeat(32),
    });
    const operationPlan = liveProviderOperationPlan(
      state,
      colimaLiveDigest(colimaLiveBytes(ownPreparation.observation)),
    );
    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    chmodSync(foreignPreparation.providerRoot, 0o000);
    const stateBefore = snapshotTree(state.root);
    assert.throws(
      () =>
        observeColimaLivePreEffectAdmissionForTest({
          observation: foreignPreparation.observation,
          observationInput: foreignPreparation.input,
          repoRoot: state.repo,
          requirements: foreignPreparation.requirements,
          stateBase: state.state,
          testCheckpoint() {},
        }),
      (error) => {
        assert.equal(
          error.message,
          "live provider pre-effect observation binding was refused",
        );
        assert.equal(error.exitStatus, 73);
        return true;
      },
    );
    assert.deepEqual(snapshotTree(state.root), stateBefore);
    chmodSync(foreignPreparation.providerRoot, 0o700);

    assert.throws(
      () =>
        observeColimaLivePreEffectAdmissionForExecutor({
          observation: ownPreparation.observation,
          observationInput: ownPreparation.input,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /live provider pre-effect observation was refused/u,
    );
  } finally {
    if (foreignPreparation !== undefined) {
      chmodSync(foreignPreparation.providerRoot, 0o700);
      rmSync(foreignPreparation.root, { recursive: true, force: true });
    }
    if (ownPreparation !== undefined) {
      rmSync(ownPreparation.root, { recursive: true, force: true });
    }
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("fixture admission refuses drift between its state-namespace observations", () => {
  const cases = [
    {
      name: "target-appeared",
      prepare() {},
      mutate({ preparation }) {
        writePrivateColimaLiveFixtureFile(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
          Buffer.from("appeared\n", "utf8"),
          0o600,
        );
      },
      restore() {},
    },
    {
      name: "collision-disappeared",
      prepare({ preparation }) {
        writePrivateColimaLiveFixtureFile(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
          Buffer.from("collision\n", "utf8"),
          0o600,
        );
      },
      mutate({ preparation }) {
        rmSync(
          join(
            preparation.input.environment.COLIMA_HOME,
            preparation.input.provider_profile,
          ),
        );
      },
      restore() {},
    },
    {
      name: "static-component-replaced",
      prepare() {},
      mutate({ preparation }) {
        const path = preparation.input.component_paths["docker-cli-binary"];
        renameSync(path, `${path}-displaced`);
        writePrivateColimaLiveFixtureFile(
          path,
          preparation.componentBytes.get("docker-cli-binary"),
          0o500,
        );
      },
      restore() {},
    },
    {
      name: "source-drifted",
      prepare() {},
      mutate({ state }) {
        writeFileSync(join(state.repo, "source.txt"), "drifted source\n");
      },
      restore({ state }) {
        writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
      },
    },
  ];

  for (const value of cases) {
    const state = fixture();
    let preparation;
    try {
      assert.equal(run(state, "plan").status, 0);
      const fixtureId = parse(join(activeRun(state), "candidate.json")).run_id;
      preparation = createCleanEngineColimaLiveObservationFixture({ fixtureId });
      const operationPlan = liveProviderOperationPlan(
        state,
        colimaLiveDigest(colimaLiveBytes(preparation.observation)),
      );
      recordLiveProviderOperationPlanForTest({
        operationPlan,
        repoRoot: state.repo,
        stateBase: state.state,
      });
      const context = { preparation, state };
      value.prepare(context);
      let checkpoints = 0;
      assert.throws(
        () =>
          observeColimaLivePreEffectAdmissionForTest({
            observation: preparation.observation,
            observationInput: preparation.input,
            repoRoot: state.repo,
            requirements: preparation.requirements,
            stateBase: state.state,
            testCheckpoint(checkpoint) {
              assert.equal(checkpoint, "after-first-admission-observation");
              checkpoints += 1;
              value.mutate(context);
            },
          }),
        (error) => {
          assert.equal(
            error.exitStatus >= 70,
            true,
            `${value.name}: ${error.stack}`,
          );
          return true;
        },
      );
      assert.equal(checkpoints, 1, value.name);
      value.restore(context);
      assert.equal(
        readdirSync(activeRun(state)).some((name) =>
          /(?:admission|effect-intent)/u.test(name),
        ),
        false,
      );
      assert.equal(run(state, "verify").status, 0, value.name);
    } finally {
      if (preparation !== undefined) {
        rmSync(preparation.root, { recursive: true, force: true });
      }
      rmSync(state.root, { recursive: true, force: true });
    }
  }
});

test("live pre-effect admission has no supported mutation or lifecycle surface", () => {
  const stateSource = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  const start = stateSource.indexOf(
    "function admissionArguments",
  );
  const end = stateSource.indexOf(
    "function buildLiveProviderStartDecisionPublicationPlan",
    start,
  );
  assert.ok(start >= 0 && end > start);
  const admissionSource = stateSource.slice(start, end);
  assert.doesNotMatch(
    admissionSource,
    /\b(?:acquire|append|close|execute|finalize|launch|publish|reconcile|recover|spawn|unlink|write)[A-Za-z0-9_]*\s*\(/u,
  );

  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /pre-effect-admission|live-admission/u);

  const receipts = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-receipts.mjs", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(receipts, /colima-live-pre-effect-admission/u);
});

test("a live provider plan is run-bound, immutable and fails closed on drift", () => {
  const state = fixture();
  const other = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    assert.equal(run(other, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    for (const [field, value] of [
      ["source_candidate_sha256", "d".repeat(64)],
      ["source_head_sha256", "d".repeat(64)],
    ]) {
      const stateMismatched = structuredClone(operationPlan);
      stateMismatched[field] = value;
      assert.throws(
        () =>
          recordLiveProviderOperationPlanForTest({
            operationPlan: stateMismatched,
            repoRoot: state.repo,
            stateBase: state.state,
          }),
        /live provider planning operation binding was refused/u,
      );
      assert.equal(
        readdirSync(activeRun(state)).some((name) => /^\.mutation-slot-/u.test(name)),
        false,
      );
    }
    assert.throws(
      () =>
        recordLiveProviderOperationPlanForTest({
          operationPlan,
          repoRoot: other.repo,
          stateBase: other.state,
        }),
      /live provider planning operation binding was refused/u,
    );
    assert.equal(
      readdirSync(activeRun(other)).some((name) => /^\.mutation-slot-/u.test(name)),
      false,
    );

    recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const slotPath = join(active, ".mutation-slot-00");
    const original = readFileSync(slotPath);
    const changed = structuredClone(operationPlan);
    changed.preparation_observation_sha256 = "d".repeat(64);
    assert.throws(
      () =>
        recordLiveProviderOperationPlanForTest({
          operationPlan: changed,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /already differs/u,
    );
    assert.deepEqual(readFileSync(slotPath), original);

    const corrupted = parse(slotPath);
    corrupted.operation_plan.capabilities.execution_authorized = true;
    writeFileSync(slotPath, canonicalBytes(corrupted), { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 69);
    assert.equal(refused.stdout, "");
    assert.match(refused.stderr, /live provider operation plan was refused/u);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
    rmSync(other.root, { recursive: true, force: true });
  }
});

test("live provider planning and the fake executor share one atomic mutation slot", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    const helper = resolve("scripts/fixtures/race-clean-engine-live-provider-plan.mjs");
    const launch = (action) => {
      const child = spawn(
        process.execPath,
        [helper, action, state.repo, state.state, JSON.stringify(operationPlan)],
        {
          env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      child.stdout.resume();
      child.stderr.resume();
      return new Promise((resolvePromise) => {
        child.on("close", (status, signal) => resolvePromise({ signal, status }));
      });
    };
    const results = await Promise.all([launch("plan"), launch("fake")]);
    assert.deepEqual(
      results.map((value) => value.status).sort((left, right) => left - right),
      [0, 73],
    );
    assert.equal(results.every((value) => value.signal === null), true);
    assert.equal(run(state, "status").status, 0);
    const active = activeRun(state);
    const slots = readdirSync(active).filter((name) => /^\.mutation-slot-/u.test(name));
    const closes = readdirSync(active).filter((name) => /^\.mutation-close-/u.test(name));
    assert.deepEqual(slots, [".mutation-slot-00"]);
    assert.deepEqual(closes, [".mutation-close-00"]);
    const action = parse(join(active, slots[0])).action;
    assert.ok(new Set(["provider-create", "provider-plan"]).has(action));
    assert.equal(
      action === "provider-plan",
      existsSync(join(active, "01-provider-create-intent.json")) === false,
    );
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("an abandoned effect-free live plan slot is explicitly aborted before retry", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    const helper = resolve("scripts/fixtures/race-clean-engine-live-provider-plan.mjs");
    const child = spawn(
      process.execPath,
      [helper, "plan", state.repo, state.state, JSON.stringify(operationPlan), "30000"],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout.resume();
    child.stderr.resume();
    await waitForPublishedMutationSlot(state, "provider-plan");
    assert.throws(
      () =>
        projectLiveProviderPlanCompletionForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /completed live provider plan was unavailable/u,
    );
    child.kill("SIGKILL");
    const [status, signal] = await once(child, "close");
    assert.equal(status, null);
    assert.equal(signal, "SIGKILL");

    assert.throws(
      () =>
        recordLiveProviderOperationPlanForTest({
          operationPlan,
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /explicit recovery/u,
    );
    const confirmation = liveProviderPlanRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const recovered = recoverLiveProviderPlanForExecutor({
      confirmation,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.equal(recovered.phase, "plan");
    assert.equal(parse(join(activeRun(state), ".mutation-close-00")).disposition, "aborted-before-effect");
    assert.throws(
      () =>
        projectLiveProviderPlanCompletionForExecutor({
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /completed live provider plan was unavailable/u,
    );
    const retried = recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.deepEqual(retried.operationPlan, operationPlan);
    assert.equal(parse(join(activeRun(state), ".mutation-slot-01")).action, "provider-plan");
    assert.equal(parse(join(activeRun(state), ".mutation-close-01")).disposition, "completed");
    assert.equal(
      projectLiveProviderPlanCompletionForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }).operation_plan_sha256,
      sha256(canonicalBytes(operationPlan)),
    );
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("source drift after live plan acquisition leaves only an aborted or abortable slot", async () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const operationPlan = liveProviderOperationPlan(state);
    const helper = resolve("scripts/fixtures/race-clean-engine-live-provider-plan.mjs");
    const child = spawn(
      process.execPath,
      [helper, "plan", state.repo, state.state, JSON.stringify(operationPlan), "750"],
      {
        env: { PATH: process.env.PATH, LANG: "C", LC_ALL: "C" },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    child.stdout.resume();
    child.stderr.resume();
    const active = activeRun(state);
    const slotPath = join(active, ".mutation-slot-00");
    const deadline = Date.now() + 8_000;
    while (!existsSync(slotPath)) {
      assert.ok(Date.now() < deadline, "timed out waiting for live plan slot");
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    }
    writeFileSync(join(state.repo, "source.txt"), "source changed during planning\n");
    const [status, signal] = await once(child, "close");
    assert.equal(status, 78);
    assert.equal(signal, null);
    assert.deepEqual(
      readdirSync(active).filter((name) => /^[0-9]{2}-.*\.json$/u.test(name)),
      ["00-plan.json"],
    );

    if (!existsSync(join(active, ".mutation-close-00"))) {
      const confirmation = liveProviderPlanRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      });
      recoverLiveProviderPlanForExecutor({
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      });
    }
    assert.equal(
      parse(join(active, ".mutation-close-00")).disposition,
      "aborted-before-effect",
    );
    writeFileSync(join(state.repo, "source.txt"), "fixture source\n");
    const retried = recordLiveProviderOperationPlanForTest({
      operationPlan,
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assert.deepEqual(retried.operationPlan, operationPlan);
    assert.equal(run(state, "verify").status, 0);
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

test("superseded mutation schemas and non-provider operation evidence are refused", () => {
  const legacySlot = fixture();
  try {
    assert.equal(run(legacySlot, "plan").status, 0);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: legacySlot.repo,
      stateBase: legacySlot.state,
    });
    const slotPath = join(activeRun(legacySlot), ".mutation-slot-00");
    const slot = parse(slotPath);
    for (const schema of [
      "synveda.clean-engine.mutation-slot.v1",
      "synveda.clean-engine.mutation-slot.v2",
      "synveda.clean-engine.mutation-slot.v3",
      "synveda.clean-engine.mutation-slot.v4",
      "synveda.clean-engine.mutation-slot.v5",
      "synveda.clean-engine.mutation-slot.v6",
    ]) {
      slot.schema = schema;
      writeFileSync(slotPath, canonicalBytes(slot), { mode: 0o600 });
      const refused = run(legacySlot, "status");
      assert.equal(refused.status, 78);
      assert.equal(refused.stderr, "clean-engine: mutation slot was refused\n");
    }
  } finally {
    rmSync(legacySlot.root, { recursive: true, force: true });
  }

  for (const schema of [
    "synveda.clean-engine.mutation-close.v1",
    "synveda.clean-engine.mutation-close.v2",
    "synveda.clean-engine.mutation-close.v3",
    "synveda.clean-engine.mutation-close.v4",
    "synveda.clean-engine.mutation-close.v5",
    "synveda.clean-engine.mutation-close.v6",
    "synveda.clean-engine.mutation-close.v7",
  ]) {
    const legacyClose = fixture();
    try {
      assert.equal(run(legacyClose, "plan").status, 0);
      executeProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        repoRoot: legacyClose.repo,
        stateBase: legacyClose.state,
      });
      const closePath = join(activeRun(legacyClose), ".mutation-close-00");
      const close = parse(closePath);
      close.schema = schema;
      writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
      const refused = run(legacyClose, "status");
      assert.equal(refused.status, 78);
      assert.equal(refused.stderr, "clean-engine: mutation close was refused\n");
    } finally {
      rmSync(legacyClose.root, { recursive: true, force: true });
    }
  }

  for (const legacyRoot of [false, true]) {
    for (const version of [1, 2, 3, 4, 5]) {
      const legacyRecovery = fixture();
      try {
        assert.equal(run(legacyRecovery, "plan").status, 0);
        const staged = stageAbandonedProviderMutation(legacyRecovery);
        const recoveryPath = join(staged.active, ".mutation-recovery-00-00");
        const recovery = parse(recoveryPath);
        if (legacyRoot) {
          recovery.chain_root_sha256 = sha256(canonicalBytes({
            action: "provider-create",
            fixture_id: staged.candidate.run_id,
            lease_sha256: sha256(staged.leaseBytes),
            operation_contract_sha256: staged.lease.operation_contract_sha256,
            operation_kind: staged.lease.operation_kind,
            operation_plan_sha256: "0".repeat(64),
            schema: `synveda.clean-engine.mutation-recovery-root.v${version}`,
          }));
        } else {
          recovery.schema = `synveda.clean-engine.mutation-recovery.v${version}`;
        }
        writeFileSync(recoveryPath, canonicalBytes(recovery), { mode: 0o600 });
        const refused = run(legacyRecovery, "status");
        assert.equal(refused.status, 78);
        assert.equal(refused.stderr, "clean-engine: mutation recovery claim was refused\n");
      } finally {
        rmSync(legacyRecovery.root, { recursive: true, force: true });
      }
    }
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

test("recovery confirmation is read-only and acquisition retires a linked claim stage", () => {
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
    assertPrivate(stage, 0o600, false, 2);
    assertPrivate(claim, 0o600, false, 2);
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
    assert.equal(existsSync(stage), false);
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
      "81159eaee9bc06e651a529008b9531b873b700a1fbf88852d58f018cd8c8d39e",
    );
    assert.equal(intent.result.provider_resource, `synveda-cpr45-${candidate.run_id}`);
    assertPrivate(join(activeRun(state), ".mutation-slot-00"), 0o600);
    assertPrivate(join(activeRun(state), ".mutation-close-00"), 0o600);
    assert.equal(run(state, "verify").status, 0);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("the synchronous fake cannot publish controlled-provider evidence", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    assert.throws(
      () =>
        executeProviderCreateForExecutor({
          adapter: fakeProviderAdapter({
            executeResult: {
              evidence_class: "controlled-fake",
              platform: "deterministic-posix",
              provider_contract_sha256:
                "81159eaee9bc06e651a529008b9531b873b700a1fbf88852d58f018cd8c8d39e",
              provider_evidence_sha256: "d".repeat(64),
              provider_name: "controlled-fake",
              runtime_name: "none",
            },
          }),
          repoRoot: state.repo,
          stateBase: state.state,
        }),
      /fake provider result was refused/,
    );
    assert.equal(existsSync(join(activeRun(state), "02-provider-create-passed.json")), false);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("persisted synchronous state cannot be relabelled as controlled evidence", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    executeProviderCreateForExecutor({
      adapter: fakeProviderAdapter(),
      repoRoot: state.repo,
      stateBase: state.state,
    });
    const active = activeRun(state);
    const receiptPath = join(active, "02-provider-create-passed.json");
    const closePath = join(active, ".mutation-close-00");
    const receipt = parse(receiptPath);
    receipt.result = {
      evidence_class: "controlled-fake",
      platform: "deterministic-posix",
      provider_contract_sha256: receipt.result.provider_contract_sha256,
      provider_evidence_sha256: "d".repeat(64),
      provider_name: "controlled-fake",
      runtime_name: "none",
    };
    const receiptBytes = canonicalBytes(receipt);
    writeFileSync(receiptPath, receiptBytes, { mode: 0o600 });
    const close = parse(closePath);
    close.result_head_sha256 = sha256(receiptBytes);
    writeFileSync(closePath, canonicalBytes(close), { mode: 0o600 });
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.equal(refused.stderr, "clean-engine: provider result was refused\n");
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
      "provider-effect-retired",
      "finalize-passed",
    ]) {
      assert.throws(
        () => appendReceiptForExecutor({
          phase,
          repoRoot: state.repo,
          result: cleanEngineReceiptResult(
            new Set([
              "finalize-passed",
              "provider-cleanup-passed",
              "provider-effect-retired",
            ]).has(phase)
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

test("provider recovery rejects a mismatched fixed fake contract before claiming", () => {
  const state = fixture();
  try {
    assert.equal(run(state, "plan").status, 0);
    const { active } = stageAbandonedProviderMutation(state, {
      providerContractSha256: "3".repeat(64),
    });
    assert.throws(
      () => providerRecoveryConfirmationForExecutor({
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /deterministic provider intent binding was refused/,
    );
    assertPrivate(join(active, ".mutation-slot-00"), 0o600);
    assert.deepEqual(
      readdirSync(active).filter((name) => name.startsWith(".mutation-recovery-")),
      [".mutation-recovery-00-00"],
    );
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
    const stage = join(active, `.mutation-stage-${"a".repeat(32)}`);
    const stageBytes = canonicalBytes({ schema: "synveda.test.live-owner-stage" });
    writeFileSync(stage, stageBytes, { mode: 0o600 });
    const stageIdentity = lstatSync(stage, { bigint: true });
    const confirmation = providerRecoveryConfirmationForExecutor({
      repoRoot: state.repo,
      stateBase: state.state,
    });
    assertPrivate(stage, 0o600);
    assert.throws(
      () => recoverProviderCreateForExecutor({
        adapter: fakeProviderAdapter(),
        confirmation,
        repoRoot: state.repo,
        stateBase: state.state,
      }),
      /provider mutation owner is active or could not be identified/,
    );
    const currentStageIdentity = lstatSync(stage, { bigint: true });
    assert.equal(currentStageIdentity.dev, stageIdentity.dev);
    assert.equal(currentStageIdentity.ino, stageIdentity.ino);
    assert.deepEqual(readFileSync(stage), stageBytes);
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
      /fake provider result was refused/,
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

test("a recovery claim published after an owner close invalidates the journal", () => {
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
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.match(
      refused.stderr,
      /mutation close (?:authority|receipt binding) was refused/,
    );
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(close.authority, "owner");
    const candidate = parse(join(active, "candidate.json"));
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "registry-intent",
        repoRoot: state.repo,
        result: cleanEngineReceiptResult("registry-intent", candidate.run_id),
        stateBase: state.state,
      }),
      /mutation close (?:authority|receipt binding) was refused/,
    );
    assertPrivate(join(active, ".mutation-recovery-00-00"), 0o600);
    assert.equal(existsSync(join(active, ".mutation-slot-01")), false);
    assert.equal(run(state, "verify").status, 78);
  } finally {
    rmSync(state.root, { recursive: true, force: true });
  }
});

test("a trailing recovery claim invalidates a closed recovery generation", () => {
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
    const refused = run(state, "status");
    assert.equal(refused.status, 78);
    assert.match(
      refused.stderr,
      /mutation close (?:authority|receipt binding) was refused/,
    );
    const close = parse(join(active, ".mutation-close-00"));
    assert.equal(
      close.authority_sha256,
      sha256(readFileSync(join(active, ".mutation-recovery-00-01"))),
    );
    assert.throws(
      () => appendReceiptForExecutor({
        phase: "failure-cleanup-intent",
        repoRoot: state.repo,
        result: {
          authorized_resources: ["provider"],
          scope: "exact-receipt-owned-only",
        },
        stateBase: state.state,
      }),
      /mutation close (?:authority|receipt binding) was refused/,
    );
    assertPrivate(join(active, ".mutation-recovery-00-02"), 0o600);
    assert.equal(existsSync(join(active, ".mutation-slot-01")), false);
    assert.equal(run(state, "verify").status, 78);
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
      /^(?:another provider recovery (?:is active or could not be identified|won the mutation claim)|no abandoned provider mutation was available|provider recovery observation changed before claim publication)\n$/,
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
    assert.equal(finalized.manifest.schema, "synveda.clean-engine.synthetic-environment.v1");
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

test("receipt schemas v1 through v5 are explicit pre-provider hard-cut refusals", () => {
  for (const schema of [
    "synveda.clean-engine.receipt.v1",
    "synveda.clean-engine.receipt.v2",
    "synveda.clean-engine.receipt.v3",
    "synveda.clean-engine.receipt.v4",
    "synveda.clean-engine.receipt.v5",
  ]) {
    const state = fixture();
    try {
      assert.equal(run(state, "plan").status, 0);
      const path = join(activeRun(state), "00-plan.json");
      const legacy = parse(path);
      legacy.schema = schema;
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

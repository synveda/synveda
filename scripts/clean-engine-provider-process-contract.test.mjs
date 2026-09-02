import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  realpathSync,
  readdirSync,
  renameSync,
  rmSync,
  rmdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative } from "node:path";
import { test } from "node:test";
import {
  COLIMA_LIVE_PREPARATION_CONTRACT,
  COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
  CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS,
  CONTROLLED_BACKGROUND_PROVIDER_CONTRACT,
  ProviderProcessContractFailure,
  authorizeColimaLiveStart,
  controlledBackgroundEngineArchitecture,
  controlledBackgroundEnvironmentNames,
  controlledBackgroundOperationEvidence,
  inspectControlledBackgroundProvider,
  inspectControlledBackgroundProviderPrefix,
  launchControlledBackgroundProvider,
  launchControlledBackgroundProviderWithAuthorityGate,
  planControlledBackgroundProviderCreate,
  planControlledBackgroundRetirement,
  providerProcessBytes,
  providerProcessDigest,
  retireControlledBackgroundProvider,
  validateColimaLiveHostEligibility,
  validateColimaLivePreparationContract,
} from "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs";

function fixture() {
  const temporary = realpathSync(process.platform === "darwin" ? "/private/tmp" : "/tmp");
  const root = realpathSync(mkdtempSync(join(temporary, "s-bg-")));
  chmodSync(root, 0o700);
  const providerBase = join(root, "p");
  const evidenceDirectory = join(root, "e");
  for (const path of [providerBase, evidenceDirectory]) {
    mkdirSync(path, { mode: 0o700 });
    chmodSync(path, 0o700);
  }
  return {
    evidenceDirectory,
    fixtureId: randomBytes(16).toString("hex"),
    providerBase,
    root,
  };
}

function bindings(seed = "1") {
  const digit = Number.parseInt(seed, 16);
  const value = (offset) => ((digit + offset) % 16).toString(16).repeat(64);
  return {
    cleanup_intent_sha256: value(0),
    cleanup_slot_sequence: 1,
    cleanup_slot_sha256: value(1),
    create_close_sha256: value(2),
    create_slot_sha256: createBindings().create_slot_sha256,
    source_head_sha256: value(4),
    source_sequence: 2,
  };
}

function createBindings(seed = "a", stateIntegration = "fixture-only") {
  const digit = Number.parseInt(seed, 16);
  const value = (offset) => ((digit + offset) % 16).toString(16).repeat(64);
  return {
    create_intent_sha256: value(0),
    create_slot_sequence: 0,
    create_slot_sha256: value(1),
    ownership_nonce: value(3),
    source_head_sha256: value(2),
    source_sequence: 0,
    state_integration: stateIntegration,
  };
}

async function launch(state, maximumLifetimeMilliseconds = 5_000) {
  planControlledBackgroundProviderCreate({
    bindings: createBindings(),
    evidenceDirectory: state.evidenceDirectory,
    fixtureId: state.fixtureId,
    providerBase: state.providerBase,
  });
  return launchPlanned(state, maximumLifetimeMilliseconds);
}

async function launchPlanned(
  state,
  maximumLifetimeMilliseconds = 5_000,
  options = {},
) {
  return launchControlledBackgroundProvider({
    ...options,
    evidenceDirectory: state.evidenceDirectory,
    fixtureId: state.fixtureId,
    maximumLifetimeMilliseconds,
    providerBase: state.providerBase,
  });
}

function providerRootPath(state) {
  return join(state.providerBase, `svb-${state.fixtureId.slice(0, 12)}`);
}

function hostagentRecordPath(state) {
  const profile = `svb-${state.fixtureId.slice(0, 12)}`;
  return join(state.providerBase, profile, "l", `colima-${profile}`, "ha.pid");
}

function providerSocketPaths(state) {
  const profile = `svb-${state.fixtureId.slice(0, 12)}`;
  const root = join(state.providerBase, profile);
  return {
    engine: join(root, "c", profile, "docker.sock"),
    hostagent: join(root, "l", `colima-${profile}`, "ha.sock"),
  };
}

function processPresent(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    throw error;
  }
}

function knownAbsentPid() {
  for (let candidate = 999_999; candidate >= 999_000; candidate -= 1) {
    try {
      process.kill(candidate, 0);
    } catch (error) {
      if (error?.code === "ESRCH") return candidate;
      throw error;
    }
  }
  throw new Error("a test-owned absent PID was unavailable");
}

async function waitForProcessAbsent(pid, timeoutMilliseconds = 7_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (!processPresent(pid)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  throw new Error("test-owned background hostagent remained present");
}

async function waitForPath(path, timeoutMilliseconds = 7_000) {
  const deadline = Date.now() + timeoutMilliseconds;
  while (Date.now() < deadline) {
    if (existsSync(path)) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
  }
  throw new Error(`test-owned path did not appear: ${path}`);
}

async function cleanupFixture(state) {
  let pid;
  try {
    pid = JSON.parse(readFileSync(hostagentRecordPath(state), "utf8")).pid;
  } catch {
    pid = undefined;
  }
  if (Number.isSafeInteger(pid) && processPresent(pid)) {
    try {
      if (!existsSync(join(state.evidenceDirectory, "provider-retirement-plan.json"))) {
        await plan(state);
      }
      await retire(state);
    } catch {
      await waitForProcessAbsent(pid);
    }
  }
  if (Number.isSafeInteger(pid) && processPresent(pid)) await waitForProcessAbsent(pid);
  rmSync(state.root, { force: true, recursive: true });
}

function artifactStageName(targetName, bytes, nonce = "a".repeat(32)) {
  const sha256 = providerProcessDigest(bytes);
  return `.provider-process-stage-${targetName.slice(0, -5)}-${sha256}-${nonce}`;
}

function privateStageName(targetPath, expectedBytes, nonce = "b".repeat(32)) {
  return `.background-private-stage-${providerProcessDigest(Buffer.from(basename(targetPath), "utf8"))}-${providerProcessDigest(expectedBytes)}-${nonce}`;
}

function writeDurable(path, bytes) {
  writeFileSync(path, bytes, { mode: 0o600 });
  for (const target of [path, dirname(path)]) {
    const descriptor = openSync(
      target,
      target === path ? constants.O_RDONLY : constants.O_RDONLY | constants.O_DIRECTORY,
    );
    try {
      fsyncSync(descriptor);
    } finally {
      closeSync(descriptor);
    }
  }
}

function unlinkDurable(path) {
  unlinkSync(path);
  const descriptor = openSync(dirname(path), constants.O_RDONLY | constants.O_DIRECTORY);
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

function rewriteCanonicalArtifact(path, value) {
  if (existsSync(path)) unlinkDurable(path);
  const bytes = providerProcessBytes(value);
  writeDurable(path, bytes);
  return providerProcessDigest(bytes);
}

function moveToPrivateStage(path) {
  const artifactBytes = readFileSync(path);
  unlinkDurable(path);
  const stage = join(dirname(path), privateStageName(path, artifactBytes));
  writeDurable(stage, artifactBytes);
  return stage;
}

function fileIdentity(path, role) {
  const canonicalPath = realpathSync(path);
  const metadata = lstatSync(canonicalPath, { bigint: true });
  const bytes = readFileSync(canonicalPath);
  return {
    device: String(metadata.dev),
    inode: String(metadata.ino),
    links: String(metadata.nlink),
    mode: (metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
    path: canonicalPath,
    role,
    sha256: providerProcessDigest(bytes),
    size: String(metadata.size),
    uid: String(metadata.uid),
  };
}

function substituteControllerToolchainPath(evidenceDirectory, replacementPath) {
  const read = (name) =>
    JSON.parse(readFileSync(join(evidenceDirectory, name), "utf8"));
  const write = (name, value) =>
    rewriteCanonicalArtifact(join(evidenceDirectory, name), value);

  const toolchain = read("background-toolchain.json");
  const component = toolchain.components.find((entry) => entry.role === "controller-script");
  Object.assign(component, fileIdentity(replacementPath, "controller-script"));
  const toolchainSha256 = write("background-toolchain.json", toolchain);

  const controllerLaunch = read("controller-launch-decision.json");
  controllerLaunch.toolchain_sha256 = toolchainSha256;
  const controllerLaunchSha256 = write(
    "controller-launch-decision.json",
    controllerLaunch,
  );

  const controller = read("controller-witness.json");
  controller.controller_launch_decision_sha256 = controllerLaunchSha256;
  controller.toolchain_sha256 = toolchainSha256;
  const controllerSha256 = write("controller-witness.json", controller);

  const startDecision = read("provider-start-decision.json");
  startDecision.controller_witness_sha256 = controllerSha256;
  const startDecisionSha256 = write("provider-start-decision.json", startDecision);

  const hostagent = read("hostagent-witness.json");
  hostagent.controller_witness_sha256 = controllerSha256;
  hostagent.start_decision_sha256 = startDecisionSha256;
  const hostagentSha256 = write("hostagent-witness.json", hostagent);

  const engine = read("engine-witness.json");
  engine.hostagent_witness_sha256 = hostagentSha256;
  const engineSha256 = write("engine-witness.json", engine);

  const controllerSettlement = read("controller-settlement.json");
  controllerSettlement.controller_witness_sha256 = controllerSha256;
  controllerSettlement.provider_start_decision_sha256 = startDecisionSha256;
  const controllerSettlementSha256 = write(
    "controller-settlement.json",
    controllerSettlement,
  );

  const identity = read("provider-identity.json");
  identity.controller_launch_decision_sha256 = controllerLaunchSha256;
  identity.controller_settlement_sha256 = controllerSettlementSha256;
  identity.controller_witness_sha256 = controllerSha256;
  identity.engine_witness_sha256 = engineSha256;
  identity.hostagent_witness_sha256 = hostagentSha256;
  identity.start_decision_sha256 = startDecisionSha256;
  identity.toolchain_sha256 = toolchainSha256;
  write("provider-identity.json", identity);
}

async function plan(state, binding = bindings()) {
  return planControlledBackgroundRetirement({
    bindings: binding,
    evidenceDirectory: state.evidenceDirectory,
    fixtureId: state.fixtureId,
    providerBase: state.providerBase,
  });
}

async function retire(state, options = {}) {
  return retireControlledBackgroundProvider({
    evidenceDirectory: state.evidenceDirectory,
    fixtureId: state.fixtureId,
    providerBase: state.providerBase,
    ...options,
  });
}

test("the tagged Colima contract is closed and cannot authorize a live start", () => {
  assert.equal(validateColimaLivePreparationContract(COLIMA_LIVE_PREPARATION_CONTRACT),
    COLIMA_LIVE_PREPARATION_CONTRACT);
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.application.version, "0.10.3");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.application.source_revision,
    "00f6c297e92a82c04a4ab507db0a61435650d7e8");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.lima.version, "2.2.0");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.lima.source_revision,
    "de0816ea4bdc5267b428ab21025889b8dd785526");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.controller_semantics,
    "waits-after-background-lima-start");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.lima_start_semantics,
    "background-hostagent");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.home_policy,
    "ambient-inherited-unchanged");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.helper_closure, "unresolved-blocking");
  assert.equal(COLIMA_LIVE_PREPARATION_CONTRACT.start_authorized, false);
  assert.deepEqual(COLIMA_LIVE_PREPARATION_CONTRACT.target_host, {
    architecture: "arm64",
    os_version_gate: "unresolved-blocking",
    platform: "darwin",
  });
  const targetHost = { architecture: "arm64", platform: "darwin" };
  assert.equal(
    validateColimaLiveHostEligibility(COLIMA_LIVE_PREPARATION_CONTRACT, targetHost),
    targetHost,
  );
  assert.throws(
    () => authorizeColimaLiveStart(COLIMA_LIVE_PREPARATION_CONTRACT, targetHost),
    /unresolved toolchain closure/,
  );
  assert.throws(
    () => validateColimaLiveHostEligibility(COLIMA_LIVE_PREPARATION_CONTRACT, {
      architecture: "x64",
      platform: "linux",
    }),
    /host shape was refused/,
  );
  const relabelled = structuredClone(COLIMA_LIVE_PREPARATION_CONTRACT);
  relabelled.start_authorized = true;
  assert.throws(
    () => validateColimaLivePreparationContract(relabelled),
    /preparation contract was refused/,
  );
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.provider_kind,
    "controlled-background-fake");
  assert.equal(
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.live_preparation_contract_sha256,
    COLIMA_LIVE_PREPARATION_CONTRACT_SHA256,
  );
  assert.deepEqual(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.artifact_order, [
    "background-create-authority.json",
    "background-toolchain.json",
    "controller-launch-decision.json",
    "controller-witness.json",
    "provider-start-decision.json",
    "hostagent-witness.json",
    "engine-witness.json",
    "context-witness.json",
    "controller-settlement.json",
    "provider-identity.json",
  ]);
  assert.equal(
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.launch_protocol,
    "durable-evidence-state-veto-gate-v2",
  );
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.kind,
    "controlled-background-provider-v4");
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.schema,
    "synveda.clean-engine.controlled-background-provider-contract.v4");
  assert.equal(
    CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.child_process_identity_proof,
    "full-causal-record-hmac-sha256-v1",
  );
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.private_file_publication,
    "fsync-stage-link-no-replace-v1");
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.root_publication,
    "authority-before-mkdir-owner-atomic-v1");
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.socket_publication,
    "umask-0177-listen-chmod-fsync-v1");
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.state_authority_gate,
    "synchronous-veto-only-five-checkpoint-v1");
  assert.deepEqual(CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS, [
    "before-root-publication",
    "before-controller-spawn",
    "before-start-decision-publication",
    "before-hostagent-start-delivery",
    "before-provider-identity-publication",
  ]);
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.fixture_launch_authorized, true);
  assert.equal(CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.lifecycle_exposure_authorized, false);
  assert.deepEqual(
    controlledBackgroundEnvironmentNames({
      hasCfUserTextEncoding: false,
      platform: "darwin",
    }),
    COLIMA_LIVE_PREPARATION_CONTRACT.environment_names,
  );
  assert.deepEqual(
    controlledBackgroundEnvironmentNames({
      hasCfUserTextEncoding: true,
      platform: "darwin",
    }),
    [
      ...COLIMA_LIVE_PREPARATION_CONTRACT.environment_names,
      "__CF_USER_TEXT_ENCODING",
    ].sort(),
  );
  assert.deepEqual(
    controlledBackgroundEnvironmentNames({
      hasCfUserTextEncoding: true,
      platform: "linux",
    }),
    COLIMA_LIVE_PREPARATION_CONTRACT.environment_names,
  );
  assert.equal(controlledBackgroundEngineArchitecture("arm64"), "aarch64");
  assert.equal(controlledBackgroundEngineArchitecture("x64"), "x86_64");
  assert.throws(
    () => controlledBackgroundEngineArchitecture("ppc64"),
    /host architecture was refused/,
  );
});

test("the read-only inspector identifies empty, authorised and complete create prefixes", async () => {
  const state = fixture();
  try {
    const empty = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { providerBase: state.providerBase },
    );
    assert.equal(empty.evidenceStage, "empty");
    assert.equal(empty.residual.root_disposition, "absent");
    assert.equal(empty.residual.controller_presence, "not-started");
    assert.equal(empty.residual.hostagent_presence, "not-started");
    assert.equal(empty.replaySafe, false);
    const boundEmpty = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    const differentlyBoundEmpty = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings("b"), providerBase: state.providerBase },
    );
    const secondEvidenceDirectory = join(state.root, "e2");
    mkdirSync(secondEvidenceDirectory, { mode: 0o700 });
    chmodSync(secondEvidenceDirectory, 0o700);
    const otherDirectoryEmpty = inspectControlledBackgroundProviderPrefix(
      secondEvidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.notEqual(boundEmpty.residualSha256, differentlyBoundEmpty.residualSha256);
    assert.notEqual(boundEmpty.residualSha256, otherDirectoryEmpty.residualSha256);
    assert.throws(
      () => inspectControlledBackgroundProviderPrefix(
        state.evidenceDirectory,
        state.fixtureId,
      ),
      /prefix arguments were refused/,
    );
    assert.throws(
      () => inspectControlledBackgroundProviderPrefix(
        state.evidenceDirectory,
        state.fixtureId,
        { expectedCreateBindings: {}, providerBase: state.providerBase },
      ),
      /create bindings fields were refused/,
    );
    assert.throws(
      () => inspectControlledBackgroundProviderPrefix(
        state.evidenceDirectory,
        state.fixtureId,
        { providerBase: state.providerBase, revalidateCurrentToolchain: "yes" },
      ),
      /prefix arguments were refused/,
    );

    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const authorised = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      {
        expectedCreateBindings: createBindings(),
        providerBase: state.providerBase,
      },
    );
    assert.equal(authorised.evidenceStage, "create-authority");
    assert.equal(authorised.residual.root_disposition, "absent");
    assert.equal(authorised.replaySafe, true);

    const checkpoints = [];
    await launchControlledBackgroundProviderWithAuthorityGate(
      {
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        maximumLifetimeMilliseconds: 5_000,
        providerBase: state.providerBase,
      },
      ({ checkpoint }) => {
        checkpoints.push(checkpoint);
      },
    );
    assert.deepEqual(checkpoints, CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS);
    const complete = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      {
        expectedCreateBindings: createBindings(),
        providerBase: state.providerBase,
      },
    );
    assert.equal(complete.evidenceStage, "provider-identity");
    assert.equal(complete.residual.root_disposition, "owned");
    assert.equal(complete.replaySafe, false);
    assert.equal(complete.pendingPublication, undefined);
    assert.equal(complete.evidenceHeadSha256,
      complete.artifacts["provider-identity.json"].sha256);
  } finally {
    await cleanupFixture(state);
  }
});

test("every state-authority checkpoint is a veto before its next effect", async (t) => {
  const expectations = [
    {
      checkpoint: "before-root-publication",
      controller: "not-started",
      head: "background-create-authority.json",
      hostagent: "not-started",
      next: "background-toolchain.json",
      replaySafe: true,
      root: "absent",
      sockets: "absent",
      stage: "create-authority",
    },
    {
      checkpoint: "before-controller-spawn",
      controller: "unattested",
      head: "controller-launch-decision.json",
      hostagent: "not-started",
      next: "controller-witness.json",
      replaySafe: false,
      root: "owned",
      sockets: "absent",
      stage: "controller-launch-decision",
    },
    {
      checkpoint: "before-start-decision-publication",
      controller: "proved-absent",
      head: "controller-witness.json",
      hostagent: "not-started",
      next: "provider-start-decision.json",
      replaySafe: false,
      root: "owned",
      sockets: "absent",
      stage: "controller-witness",
    },
    {
      checkpoint: "before-hostagent-start-delivery",
      controller: "proved-absent",
      head: "provider-start-decision.json",
      hostagent: "unattested",
      next: "hostagent-witness.json",
      replaySafe: false,
      root: "owned",
      sockets: "absent",
      stage: "provider-start-decision",
    },
    {
      checkpoint: "before-provider-identity-publication",
      controller: "proved-absent",
      head: "controller-settlement.json",
      hostagent: "observed-present",
      next: "provider-identity.json",
      replaySafe: false,
      root: "owned",
      sockets: "present",
      stage: "controller-settlement",
    },
  ];
  for (const [index, expectation] of expectations.entries()) {
    await t.test(expectation.checkpoint, async () => {
      const state = fixture();
      try {
        planControlledBackgroundProviderCreate({
          bindings: createBindings(),
          evidenceDirectory: state.evidenceDirectory,
          fixtureId: state.fixtureId,
          providerBase: state.providerBase,
        });
        const observed = [];
        await assert.rejects(
          () =>
            launchControlledBackgroundProviderWithAuthorityGate(
              {
                evidenceDirectory: state.evidenceDirectory,
                fixtureId: state.fixtureId,
                maximumLifetimeMilliseconds: 2_000,
                providerBase: state.providerBase,
              },
              ({ checkpoint, evidence_head_sha256: evidenceHeadSha256 }) => {
                assert.match(evidenceHeadSha256, /^[0-9a-f]{64}$/);
                observed.push({ checkpoint, evidenceHeadSha256 });
                if (checkpoint === expectation.checkpoint) {
                  throw new Error("state authority veto");
                }
              },
            ),
          /state authority veto/,
        );
        assert.deepEqual(
          observed.map(({ checkpoint }) => checkpoint),
          CONTROLLED_BACKGROUND_AUTHORITY_CHECKPOINTS.slice(0, index + 1),
        );
        const prefix = inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        );
        assert.equal(prefix.evidenceStage, expectation.stage);
        assert.equal(prefix.evidenceHeadSha256, prefix.artifacts[expectation.head].sha256);
        assert.equal(prefix.evidenceHeadSha256, observed.at(-1).evidenceHeadSha256);
        assert.equal(prefix.artifacts[expectation.next], undefined);
        assert.equal(prefix.artifacts["provider-identity.json"], undefined);
        assert.equal(prefix.replaySafe, expectation.replaySafe);
        assert.equal(prefix.residual.root_disposition, expectation.root);
        assert.equal(prefix.residual.controller_presence, expectation.controller);
        assert.equal(prefix.residual.hostagent_presence, expectation.hostagent);
        assert.equal(prefix.residual.sockets, expectation.sockets);
        assert.deepEqual(prefix.effectFrontier, {
          disposition: "complete",
          effect: expectation.stage,
        });
        if (index < 4) {
          assert.equal(existsSync(hostagentRecordPath(state)), false);
        }
        if (index > 0) {
          const before = {
            evidencePrefixSha256: prefix.evidencePrefixSha256,
            residualSha256: prefix.residualSha256,
            rootInventory: prefix.residual.root_inventory,
          };
          await assert.rejects(
            () => launchPlanned(state, 2_000),
            /pre-launch evidence inventory was refused/,
          );
          const after = inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          );
          assert.equal(after.evidencePrefixSha256, before.evidencePrefixSha256);
          assert.equal(after.residualSha256, before.residualSha256);
          assert.deepEqual(after.residual.root_inventory, before.rootInventory);
        }
      } finally {
        await cleanupFixture(state);
      }
    });
  }
});

test("durable child process identities remain probeable before parent witnesses", async (t) => {
  const evidenceSuffix = [
    "controller-witness.json",
    "provider-start-decision.json",
    "hostagent-witness.json",
    "engine-witness.json",
    "context-witness.json",
    "controller-settlement.json",
    "provider-identity.json",
  ];

  await t.test(
    "controller readiness proves an absent group without a controller witness",
    async () => {
      const state = fixture();
      try {
        const started = await launch(state, 1_000);
        await waitForProcessAbsent(started.hostagentPid);
        for (const name of evidenceSuffix) {
          unlinkDurable(join(state.evidenceDirectory, name));
        }
        unlinkDurable(hostagentRecordPath(state));

        const prefix = inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        );
        assert.equal(prefix.evidenceStage, "controller-launch-decision");
        assert.deepEqual(prefix.effectFrontier, {
          disposition: "complete",
          effect: "controller-ready",
        });
        assert.equal(prefix.residual.controller_presence, "proved-absent");
        assert.equal(prefix.residual.hostagent_presence, "not-started");
        assert.equal(prefix.residual.sockets, "absent");
      } finally {
        await cleanupFixture(state);
      }
    },
  );

  await t.test("host-agent PID proves absence without a host-agent witness", async () => {
    const state = fixture();
    try {
      const started = await launch(state, 1_000);
      await waitForProcessAbsent(started.hostagentPid);
      for (const name of evidenceSuffix.slice(2)) {
        unlinkDurable(join(state.evidenceDirectory, name));
      }

      const prefix = inspectControlledBackgroundProviderPrefix(
        state.evidenceDirectory,
        state.fixtureId,
        { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
      );
      assert.equal(prefix.evidenceStage, "provider-start-decision");
      assert.deepEqual(prefix.effectFrontier, {
        disposition: "complete",
        effect: "hostagent-pid",
      });
      assert.equal(prefix.residual.controller_presence, "proved-absent");
      assert.equal(prefix.residual.hostagent_presence, "proved-absent");
      assert.equal(prefix.residual.sockets, "absent");
    } finally {
      await cleanupFixture(state);
    }
  });
});

test("pre-witness child records authenticate live PIDs and complete private stages", async (t) => {
  await t.test("controller readiness refuses a live process-group relabel", async () => {
    const state = fixture();
    let inspected = false;
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint !== "before-start-decision-publication") return;
              unlinkDurable(join(state.evidenceDirectory, "controller-witness.json"));
              const current = inspectControlledBackgroundProviderPrefix(
                state.evidenceDirectory,
                state.fixtureId,
                { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
              );
              assert.equal(current.residual.controller_presence, "observed-present");
              assert.deepEqual(current.effectFrontier, {
                disposition: "complete",
                effect: "controller-ready",
              });
              const readyPath = join(providerRootPath(state), "t", "controller-ready.json");
              const ready = JSON.parse(readFileSync(readyPath, "utf8"));
              ready.controller_pid = knownAbsentPid();
              ready.controller_pgid = ready.controller_pid;
              rewriteCanonicalArtifact(readyPath, ready);
              assert.throws(
                () =>
                  inspectControlledBackgroundProviderPrefix(
                    state.evidenceDirectory,
                    state.fixtureId,
                    { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
                  ),
                /controller readiness was refused/,
              );
              inspected = true;
            },
          ),
        /controller readiness was refused/,
      );
      assert.equal(inspected, true);
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("controller readiness stage retains live process classification", async () => {
    const state = fixture();
    let inspected = false;
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint !== "before-start-decision-publication") return;
              unlinkDurable(join(state.evidenceDirectory, "controller-witness.json"));
              moveToPrivateStage(
                join(providerRootPath(state), "t", "controller-ready.json"),
              );
              const current = inspectControlledBackgroundProviderPrefix(
                state.evidenceDirectory,
                state.fixtureId,
                { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
              );
              assert.equal(current.residual.controller_presence, "observed-present");
              assert.deepEqual(current.effectFrontier, {
                disposition: "pending",
                effect: "controller-ready",
              });
              inspected = true;
            },
          ),
        /authority checkpoint prefix was refused/,
      );
      assert.equal(inspected, true);
      assert.equal(
        existsSync(join(state.evidenceDirectory, "provider-start-decision.json")),
        false,
      );
      assert.equal(existsSync(hostagentRecordPath(state)), false);
      const sockets = providerSocketPaths(state);
      assert.equal(existsSync(sockets.hostagent), false);
      assert.equal(existsSync(sockets.engine), false);
    } finally {
      await cleanupFixture(state);
    }
  });

  for (const staged of [false, true]) {
    await t.test(
      staged
        ? "host-agent PID stage retains live process classification"
        : "host-agent PID record refuses a live process relabel",
      async () => {
        const state = fixture();
        let inspected = false;
        let liveHostagentPid;
        try {
          planControlledBackgroundProviderCreate({
            bindings: createBindings(),
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            providerBase: state.providerBase,
          });
          await assert.rejects(
            () =>
              launchControlledBackgroundProviderWithAuthorityGate(
                {
                  evidenceDirectory: state.evidenceDirectory,
                  fixtureId: state.fixtureId,
                  maximumLifetimeMilliseconds: 2_000,
                  providerBase: state.providerBase,
                },
                ({ checkpoint }) => {
                  if (checkpoint !== "before-provider-identity-publication") return;
                  for (const name of [
                    "hostagent-witness.json",
                    "engine-witness.json",
                    "context-witness.json",
                    "controller-settlement.json",
                  ]) {
                    unlinkDurable(join(state.evidenceDirectory, name));
                  }
                  const pidPath = hostagentRecordPath(state);
                  const pidRecord = JSON.parse(readFileSync(pidPath, "utf8"));
                  liveHostagentPid = pidRecord.pid;
                  if (staged) moveToPrivateStage(pidPath);
                  const current = inspectControlledBackgroundProviderPrefix(
                    state.evidenceDirectory,
                    state.fixtureId,
                    { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
                  );
                  assert.equal(current.residual.controller_presence, "proved-absent");
                  assert.equal(current.residual.hostagent_presence, "observed-present");
                  assert.deepEqual(current.effectFrontier, {
                    disposition: staged ? "pending" : "complete",
                    effect: "hostagent-pid",
                  });
                  if (!staged) {
                    pidRecord.pid = knownAbsentPid();
                    rewriteCanonicalArtifact(pidPath, pidRecord);
                    assert.throws(
                      () =>
                        inspectControlledBackgroundProviderPrefix(
                          state.evidenceDirectory,
                          state.fixtureId,
                          {
                            expectedCreateBindings: createBindings(),
                            providerBase: state.providerBase,
                          },
                        ),
                      /hostagent PID record was refused/,
                    );
                  }
                  inspected = true;
                },
              ),
            staged
              ? /authority checkpoint prefix was refused/
              : /hostagent PID record was refused/,
          );
          assert.equal(inspected, true);
          assert.equal(
            existsSync(join(state.evidenceDirectory, "provider-identity.json")),
            false,
          );
        } finally {
          if (Number.isSafeInteger(liveHostagentPid)) {
            await waitForProcessAbsent(liveHostagentPid);
          }
          await cleanupFixture(state);
        }
      },
    );
  }
});

test("authority gates reject causal-prefix mutation before the named effect", async (t) => {
  await t.test("prepared root mutation blocks controller spawn", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      const profile = `svb-${state.fixtureId.slice(0, 12)}`;
      const diskPath = join(providerRootPath(state), "l", `colima-${profile}`, "basedisk");
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint === "before-controller-spawn") unlinkDurable(diskPath);
            },
          ),
        /causal gap|checkpoint prefix changed/,
      );
      assert.equal(
        existsSync(join(providerRootPath(state), "t", "controller-ready.json")),
        false,
      );
      assert.equal(
        existsSync(join(state.evidenceDirectory, "controller-witness.json")),
        false,
      );
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("non-head evidence rewrite blocks provider identity", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint !== "before-provider-identity-publication") return;
              const path = join(state.evidenceDirectory, "hostagent-witness.json");
              rewriteCanonicalArtifact(path, JSON.parse(readFileSync(path, "utf8")));
            },
          ),
        /checkpoint artifact changed/,
      );
      assert.equal(
        existsSync(join(state.evidenceDirectory, "provider-identity.json")),
        false,
      );
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("same-byte context replacement blocks provider identity", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint !== "before-provider-identity-publication") return;
              const contextPath = JSON.parse(
                readFileSync(
                  join(state.evidenceDirectory, "background-create-authority.json"),
                  "utf8",
                ),
              ).provider_root_path;
              const contextKey = providerProcessDigest(
                Buffer.from(`context\0${state.fixtureId}`, "utf8"),
              ).slice(0, 32);
              const path = join(contextPath, "d", "contexts", "meta", contextKey, "meta.json");
              const bytes = readFileSync(path);
              unlinkDurable(path);
              writeDurable(path, bytes);
            },
          ),
        /context file identity changed|checkpoint root changed/,
      );
      assert.equal(
        existsSync(join(state.evidenceDirectory, "provider-identity.json")),
        false,
      );
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("authority change during terminal probes blocks provider identity", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      let authorityCurrent = true;
      const invalidateAuthority = (async () => {
        await waitForPath(
          join(state.evidenceDirectory, "controller-settlement.json"),
        );
        authorityCurrent = false;
      })();
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              beforeIdentityProbeHoldMilliseconds: 100,
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 5_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (
                checkpoint === "before-provider-identity-publication" &&
                !authorityCurrent
              ) {
                throw new Error("state authority changed during terminal probes");
              }
            },
          ),
        /state authority changed during terminal probes/,
      );
      await invalidateAuthority;
      assert.equal(
        existsSync(join(state.evidenceDirectory, "provider-identity.json")),
        false,
      );
    } finally {
      await cleanupFixture(state);
    }
  });
});

test("prefix inspection preserves private-file stages and rejects foreign targets", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          ({ checkpoint }) => {
            if (checkpoint === "before-controller-spawn") {
              throw new Error("stop before controller");
            }
          },
        ),
      /stop before controller/,
    );
    const root = providerRootPath(state);
    const readyPath = join(root, "t", "controller-ready.json");
    const completeBytes = providerProcessBytes({ schema: "simulated-ready" });
    const partialStage = join(
      dirname(readyPath),
      privateStageName(readyPath, completeBytes),
    );
    writeDurable(partialStage, completeBytes.subarray(0, completeBytes.length - 1));
    const partial = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(partial.evidenceStage, "controller-launch-decision");
    assert.equal(existsSync(partialStage), true);
    unlinkDurable(partialStage);

    unlinkDurable(join(state.evidenceDirectory, "controller-launch-decision.json"));

    const configPath = join(root, "t", "controller-config.json");
    const configBytes = readFileSync(configPath);
    const linkedStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "c".repeat(32)),
    );
    linkSync(configPath, linkedStage);
    const linked = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(linked.residual.root_disposition, "owned");
    assert.equal(linked.residual.private_publications.length, 1);
    assert.deepEqual(
      {
        actual_sha256: linked.residual.private_publications[0].actual_sha256,
        declared_sha256: linked.residual.private_publications[0].declared_sha256,
        disposition: linked.residual.private_publications[0].disposition,
        links: linked.residual.private_publications[0].links,
        target_path: linked.residual.private_publications[0].target_path,
      },
      {
        actual_sha256: providerProcessDigest(configBytes),
        declared_sha256: providerProcessDigest(configBytes),
        disposition: "linked-complete",
        links: 2,
        target_path: relative(root, configPath),
      },
    );
    assert.equal(existsSync(linkedStage), true);
    assert.equal(lstatSync(configPath, { bigint: true }).nlink, 2n);
    unlinkDurable(linkedStage);

    const duplicateFinalStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "2".repeat(32)),
    );
    writeDurable(duplicateFinalStage, configBytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /stage link was refused/,
    );
    assert.equal(existsSync(duplicateFinalStage), true);
    unlinkDurable(duplicateFinalStage);

    const foreignAliasSource = join(state.root, "foreign-config-source.json");
    writeDurable(foreignAliasSource, configBytes);
    const foreignLinkedStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "5".repeat(32)),
    );
    linkSync(foreignAliasSource, foreignLinkedStage);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /stage link was refused/,
    );
    assert.equal(existsSync(foreignLinkedStage), true);
    assert.equal(existsSync(foreignAliasSource), true);
    unlinkDurable(foreignLinkedStage);
    unlinkDurable(foreignAliasSource);

    unlinkDurable(configPath);
    const configSha256 = providerProcessDigest(configBytes);
    const completeStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "6".repeat(32)),
    );
    writeDurable(completeStage, configBytes);
    const stagedComplete = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(stagedComplete.residual.private_publications.length, 1);
    assert.equal(
      stagedComplete.residual.private_publications[0].disposition,
      "staged-complete",
    );
    assert.equal(
      stagedComplete.effectFrontier.effect,
      "controller-config",
    );
    assert.equal(stagedComplete.effectFrontier.disposition, "pending");
    assert.equal(existsSync(completeStage), true);
    unlinkDurable(completeStage);

    const wrongDigestStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "d".repeat(32)).replace(
        configSha256,
        "1".repeat(64),
      ),
    );
    writeDurable(wrongDigestStage, configBytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /progressive stage digest was refused/,
    );
    assert.equal(existsSync(wrongDigestStage), true);
    unlinkDurable(wrongDigestStage);

    const invalidConfigBytes = providerProcessBytes({ schema: "invalid-controller-config" });
    const invalidConfigStage = join(
      dirname(configPath),
      privateStageName(configPath, invalidConfigBytes, "e".repeat(32)),
    );
    writeDurable(invalidConfigStage, invalidConfigBytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /controller config fields were refused/,
    );
    assert.equal(existsSync(invalidConfigStage), true);
    unlinkDurable(invalidConfigStage);

    const toolchainPath = join(state.evidenceDirectory, "background-toolchain.json");
    const toolchainBytes = readFileSync(toolchainPath);
    const linkedToolchainStage = join(
      state.evidenceDirectory,
      artifactStageName("background-toolchain.json", toolchainBytes, "3".repeat(32)),
    );
    linkSync(toolchainPath, linkedToolchainStage);
    const concurrentPrivateStage = join(
      dirname(configPath),
      privateStageName(configPath, configBytes, "4".repeat(32)),
    );
    writeDurable(
      concurrentPrivateStage,
      configBytes.subarray(0, configBytes.length - 1),
    );
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /pending publications were ambiguous/,
    );
    assert.equal(existsSync(linkedToolchainStage), true);
    assert.equal(existsSync(concurrentPrivateStage), true);
    unlinkDurable(linkedToolchainStage);
    unlinkDurable(concurrentPrivateStage);
    writeDurable(configPath, configBytes);

    const hostagentConfigPath = join(root, "t", "hostagent-config.json");
    const staleLinkedStage = join(
      dirname(hostagentConfigPath),
      privateStageName(
        hostagentConfigPath,
        readFileSync(hostagentConfigPath),
        "f".repeat(32),
      ),
    );
    linkSync(hostagentConfigPath, staleLinkedStage);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /causal gap/,
    );
    assert.equal(existsSync(staleLinkedStage), true);
    unlinkDurable(staleLinkedStage);

    const pidPath = hostagentRecordPath(state);
    const secondPartialStage = join(
      dirname(pidPath),
      privateStageName(pidPath, completeBytes, "1".repeat(32)),
    );
    writeDurable(partialStage, completeBytes.subarray(0, completeBytes.length - 1));
    writeDurable(secondPartialStage, completeBytes.subarray(0, completeBytes.length - 1));
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /progressive stages were ambiguous/,
    );
    assert.equal(existsSync(partialStage), true);
    assert.equal(existsSync(secondPartialStage), true);
    unlinkDurable(partialStage);
    unlinkDurable(secondPartialStage);

    const foreignStage = join(
      dirname(configPath),
      `.background-private-stage-${"d".repeat(64)}-${providerProcessDigest(configBytes)}-${"e".repeat(32)}`,
    );
    writeDurable(foreignStage, configBytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /stage target was refused/,
    );
    assert.equal(existsSync(foreignStage), true);
  } finally {
    await cleanupFixture(state);
  }
});

test("dynamic private stages require their durable process decisions", async (t) => {
  await t.test("controller readiness requires launch authority", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint === "before-controller-spawn") throw new Error("veto launch");
            },
          ),
        /veto launch/,
      );
      unlinkDurable(join(state.evidenceDirectory, "controller-launch-decision.json"));
      const readyPath = join(providerRootPath(state), "t", "controller-ready.json");
      const readyBytes = providerProcessBytes({ schema: "partial-ready" });
      const stage = join(dirname(readyPath), privateStageName(readyPath, readyBytes));
      writeDurable(stage, readyBytes.subarray(0, readyBytes.length - 1));
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /controller-ready had a causal gap/,
      );
      assert.equal(existsSync(stage), true);
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("hostagent PID requires start authority", async () => {
    const state = fixture();
    try {
      planControlledBackgroundProviderCreate({
        bindings: createBindings(),
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
        providerBase: state.providerBase,
      });
      await assert.rejects(
        () =>
          launchControlledBackgroundProviderWithAuthorityGate(
            {
              evidenceDirectory: state.evidenceDirectory,
              fixtureId: state.fixtureId,
              maximumLifetimeMilliseconds: 2_000,
              providerBase: state.providerBase,
            },
            ({ checkpoint }) => {
              if (checkpoint === "before-hostagent-start-delivery") {
                throw new Error("veto start");
              }
            },
          ),
        /veto start/,
      );
      unlinkDurable(join(state.evidenceDirectory, "provider-start-decision.json"));
      const pidPath = hostagentRecordPath(state);
      const pidBytes = providerProcessBytes({ schema: "partial-pid" });
      const stage = join(dirname(pidPath), privateStageName(pidPath, pidBytes));
      writeDurable(stage, pidBytes.subarray(0, pidBytes.length - 1));
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /hostagent-pid had a causal gap/,
      );
      assert.equal(existsSync(stage), true);
    } finally {
      await cleanupFixture(state);
    }
  });
});

test("the global create prefix binds root preparation, toolchain and configs", async (t) => {
  const prepareThroughLaunchDecision = async (state) => {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          ({ checkpoint }) => {
            if (checkpoint === "before-controller-spawn") throw new Error("veto launch");
          },
        ),
      /veto launch/,
    );
  };

  await t.test("toolchain cannot survive an incomplete prepared root", async () => {
    const state = fixture();
    try {
      await prepareThroughLaunchDecision(state);
      const contextPath = join(
        providerRootPath(state),
        "d",
        "contexts",
        "meta",
        providerProcessDigest(Buffer.from(`context\0${state.fixtureId}`, "utf8")).slice(0, 32),
        "meta.json",
      );
      unlinkDurable(contextPath);
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /causal gap/,
      );
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("configs cannot survive without their toolchain", async () => {
    const state = fixture();
    try {
      await prepareThroughLaunchDecision(state);
      unlinkDurable(join(state.evidenceDirectory, "controller-launch-decision.json"));
      unlinkDurable(join(state.evidenceDirectory, "background-toolchain.json"));
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /config lacked its toolchain|causal gap/,
      );
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("an earlier-final linked evidence stage is refused and preserved", async () => {
    const state = fixture();
    try {
      await prepareThroughLaunchDecision(state);
      unlinkDurable(join(state.evidenceDirectory, "controller-launch-decision.json"));
      const authorityPath = join(
        state.evidenceDirectory,
        "background-create-authority.json",
      );
      const authorityBytes = readFileSync(authorityPath);
      const stagePath = join(
        state.evidenceDirectory,
        artifactStageName(
          "background-create-authority.json",
          authorityBytes,
          "7".repeat(32),
        ),
      );
      linkSync(authorityPath, stagePath);
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /create-prefix stage link was foreign/,
      );
      assert.equal(existsSync(stagePath), true);
      assert.equal(lstatSync(authorityPath, { bigint: true }).nlink, 2n);
    } finally {
      await cleanupFixture(state);
    }
  });

  await t.test("a prepared root cannot disappear behind a later prefix", async () => {
    const state = fixture();
    try {
      await prepareThroughLaunchDecision(state);
      const displacedRoot = join(state.root, "displaced-provider-root");
      renameSync(providerRootPath(state), displacedRoot);
      assert.throws(
        () =>
          inspectControlledBackgroundProviderPrefix(
            state.evidenceDirectory,
            state.fixtureId,
            { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
          ),
        /provider root disappeared after preparation|causal gap/,
      );
      assert.equal(existsSync(displacedRoot), true);
    } finally {
      await cleanupFixture(state);
    }
  });
});

test("the controller refuses a wrong configuration digest before child effects", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          ({ checkpoint }) => {
            if (checkpoint === "before-controller-spawn") throw new Error("veto launch");
          },
        ),
      /veto launch/,
    );
    const controllerSource = readFileSync(
      new URL("./fixtures/clean-engine-background-controller.mjs", import.meta.url),
      "utf8",
    );
    const result = spawnSync(
      process.execPath,
      [
        "--input-type=module",
        "--eval",
        controllerSource,
        join(providerRootPath(state), "t", "controller-config.json"),
        "0".repeat(64),
      ],
      {
        cwd: providerRootPath(state),
        encoding: "utf8",
        timeout: 2_000,
      },
    );
    assert.equal(result.signal, null);
    assert.equal(result.status, 70);
    assert.match(result.stderr, /config identity changed/);
    assert.equal(
      existsSync(join(providerRootPath(state), "t", "controller-ready.json")),
      false,
    );
    assert.equal(
      existsSync(join(state.evidenceDirectory, "controller-witness.json")),
      false,
    );
    assert.equal(
      existsSync(join(state.evidenceDirectory, "provider-start-decision.json")),
      false,
    );
  } finally {
    await cleanupFixture(state);
  }
});

test("evidence-prefix inspection is non-mutating and validates complete next stages", () => {
  const state = fixture();
  try {
    const planned = planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const authorityPath = join(
      state.evidenceDirectory,
      "background-create-authority.json",
    );
    const linkedStage = join(
      state.evidenceDirectory,
      artifactStageName(
        "background-create-authority.json",
        planned.authority.bytes,
        "f".repeat(32),
      ),
    );
    linkSync(authorityPath, linkedStage);
    const linked = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(linked.pendingPublication.disposition, "linked-complete");
    assert.equal(linked.replaySafe, false);
    assert.equal(existsSync(linkedStage), true);
    assert.equal(lstatSync(authorityPath, { bigint: true }).nlink, 2n);
    unlinkDurable(linkedStage);

    const actualSha256 = providerProcessDigest(planned.authority.bytes);
    const wrongDigestStage = join(
      state.evidenceDirectory,
      artifactStageName(
        "background-toolchain.json",
        planned.authority.bytes,
        "0".repeat(32),
      ).replace(actualSha256, "2".repeat(64)),
    );
    writeDurable(wrongDigestStage, planned.authority.bytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /stage digest was refused/,
    );
    assert.equal(existsSync(wrongDigestStage), true);
    unlinkDurable(wrongDigestStage);

    const invalidToolchainStage = join(
      state.evidenceDirectory,
      artifactStageName(
        "background-toolchain.json",
        planned.authority.bytes,
        "1".repeat(32),
      ),
    );
    writeDurable(invalidToolchainStage, planned.authority.bytes);
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /toolchain (?:fields )?were refused/,
    );
    assert.equal(existsSync(invalidToolchainStage), true);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("an interrupted root mkdir is an exact residual and unknown leaves remain blocking", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const root = providerRootPath(state);
    mkdirSync(root, { mode: 0o700 });
    chmodSync(root, 0o700);
    const interrupted = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(interrupted.residual.root_disposition, "ownership-pending");
    assert.deepEqual(interrupted.residual.root_inventory, []);
    assert.match(interrupted.residualSha256, /^[0-9a-f]{64}$/);
    assert.equal(interrupted.replaySafe, false);

    let gateCalls = 0;
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          () => {
            gateCalls += 1;
          },
        ),
      /checkpoint (?:effect )?frontier was refused/,
    );
    assert.equal(gateCalls, 0);

    const unknown = join(root, "foreign-leaf");
    writeDurable(unknown, Buffer.from("foreign\n", "utf8"));
    assert.throws(
      () =>
        inspectControlledBackgroundProviderPrefix(
          state.evidenceDirectory,
          state.fixtureId,
          { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
        ),
      /root inventory was refused/,
    );
    assert.equal(readFileSync(unknown, "utf8"), "foreign\n");
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("launch requires one exact fixture-only create authority before root mutation", async () => {
  const state = fixture();
  try {
    await assert.rejects(() => launchPlanned(state), /create-authority.*unavailable/);
    assert.equal(existsSync(providerRootPath(state)), false);
    assert.deepEqual(readdirSync(state.evidenceDirectory), []);

    assert.throws(
      () =>
        planControlledBackgroundProviderCreate({
          bindings: createBindings("a", "mutation-journal-v2"),
          evidenceDirectory: state.evidenceDirectory,
          fixtureId: state.fixtureId,
          providerBase: state.providerBase,
        }),
      /create bindings were refused/,
    );
    assert.equal(existsSync(providerRootPath(state)), false);
    assert.deepEqual(readdirSync(state.evidenceDirectory), []);
  } finally {
    await cleanupFixture(state);
  }
});

test("the state gate can only veto and cannot return launch authority", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          () => ({ authority: "forged" }),
        ),
      /gate returned authority/,
    );
    assert.equal(existsSync(providerRootPath(state)), false);
    await assert.rejects(
      () =>
        launchControlledBackgroundProviderWithAuthorityGate(
          {
            evidenceDirectory: state.evidenceDirectory,
            fixtureId: state.fixtureId,
            maximumLifetimeMilliseconds: 2_000,
            providerBase: state.providerBase,
          },
          async () => undefined,
        ),
      /gate returned authority/,
    );
    assert.equal(existsSync(providerRootPath(state)), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("a dangling provider-root collision blocks authority publication", () => {
  const state = fixture();
  try {
    symlinkSync(join(state.root, "missing-provider-root"), providerRootPath(state));
    assert.throws(
      () =>
        planControlledBackgroundProviderCreate({
          bindings: createBindings(),
          evidenceDirectory: state.evidenceDirectory,
          fixtureId: state.fixtureId,
          providerBase: state.providerBase,
        }),
      /provider root collided/,
    );
    assert.deepEqual(readdirSync(state.evidenceDirectory), []);
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("create authority publication recovers exact complete and partial stages", async () => {
  const state = fixture();
  try {
    const first = planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const authorityPath = join(
      state.evidenceDirectory,
      "background-create-authority.json",
    );
    const bytes = first.authority.bytes;

    unlinkDurable(authorityPath);
    const completeStage = join(
      state.evidenceDirectory,
      artifactStageName("background-create-authority.json", bytes, "1".repeat(32)),
    );
    writeDurable(completeStage, bytes);
    const recovered = planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    assert.equal(recovered.authority.sha256, first.authority.sha256);
    assert.equal(existsSync(completeStage), false);

    unlinkDurable(authorityPath);
    const partialStage = join(
      state.evidenceDirectory,
      artifactStageName("background-create-authority.json", bytes, "2".repeat(32)),
    );
    writeDurable(partialStage, bytes.subarray(0, bytes.length - 1));
    const repaired = planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    assert.equal(repaired.authority.sha256, first.authority.sha256);
    assert.equal(existsSync(partialStage), false);

    const linkedStage = join(
      state.evidenceDirectory,
      artifactStageName("background-create-authority.json", bytes, "3".repeat(32)),
    );
    linkSync(authorityPath, linkedStage);
    assert.equal(lstatSync(authorityPath, { bigint: true }).nlink, 2n);
    const relinked = planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    assert.equal(relinked.authority.sha256, first.authority.sha256);
    assert.equal(existsSync(linkedStage), false);
    assert.equal(lstatSync(authorityPath, { bigint: true }).nlink, 1n);
  } finally {
    await cleanupFixture(state);
  }
});

test("provider-base replacement after authority is refused before root mutation", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    renameSync(state.providerBase, join(state.root, "provider-base-before-replacement"));
    mkdirSync(state.providerBase, { mode: 0o700 });
    chmodSync(state.providerBase, 0o700);

    await assert.rejects(() => launchPlanned(state), /create authority was refused/);
    assert.equal(existsSync(providerRootPath(state)), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("a future evidence artifact blocks launch before root mutation", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    writeDurable(
      join(state.evidenceDirectory, "controller-witness.json"),
      providerProcessBytes({ schema: "foreign" }),
    );

    await assert.rejects(() => launchPlanned(state), /pre-launch evidence inventory/);
    assert.equal(existsSync(providerRootPath(state)), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("controller reproof rejects a rewritten durable start decision with no hostagent", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const startDecisionPath = join(
      state.evidenceDirectory,
      "provider-start-decision.json",
    );
    const execution = launchPlanned(state, 5_000, {
      beforeStartHoldMilliseconds: 500,
    });
    await waitForPath(startDecisionPath);
    const rewritten = JSON.parse(readFileSync(startDecisionPath, "utf8"));
    rewritten.decision = "abort";
    rewriteCanonicalArtifact(startDecisionPath, rewritten);

    await assert.rejects(
      () => execution,
      /provider start decision was refused/,
    );
    assert.equal(existsSync(hostagentRecordPath(state)), false);
    assert.equal(
      existsSync(
        join(
          providerRootPath(state),
          "c",
          `svb-${state.fixtureId.slice(0, 12)}`,
          "docker.sock",
        ),
      ),
      false,
    );
  } finally {
    await cleanupFixture(state);
  }
});

test("controller reproof rejects changed start-gate inputs with no hostagent", async (t) => {
  const cases = [
    {
      error: /create authority binding changed/,
      label: "create authority",
      path: (state) => join(state.evidenceDirectory, "background-create-authority.json"),
      rewrite: (value) => ({ ...value, source_sequence: value.source_sequence + 1 }),
    },
    {
      error: /controller launch decision was refused/,
      label: "controller launch decision",
      path: (state) =>
        join(state.evidenceDirectory, "controller-launch-decision.json"),
      rewrite: (value) => ({ ...value, decision: "abort" }),
    },
    {
      error: /controller witness was refused/,
      label: "controller witness",
      path: (state) => join(state.evidenceDirectory, "controller-witness.json"),
      rewrite: (value) => ({ ...value, execution_protocol: "changed" }),
    },
    {
      error: /root owner was refused/,
      label: "root owner",
      path: (state) =>
        join(providerRootPath(state), ".synveda-background-provider-owner.json"),
      rewrite: (value) => ({ ...value, provider_profile: `${value.provider_profile}-changed` }),
    },
    {
      error: /controller launch inputs were refused/,
      label: "hostagent config",
      path: (state) => join(providerRootPath(state), "t", "hostagent-config.json"),
      rewrite: (value) => ({ ...value, profile: `${value.profile}-changed` }),
    },
  ];
  for (const entry of cases) {
    await t.test(entry.label, async () => {
      const state = fixture();
      try {
        planControlledBackgroundProviderCreate({
          bindings: createBindings(),
          evidenceDirectory: state.evidenceDirectory,
          fixtureId: state.fixtureId,
          providerBase: state.providerBase,
        });
        const startDecisionPath = join(
          state.evidenceDirectory,
          "provider-start-decision.json",
        );
        const execution = launchPlanned(state, 5_000, {
          beforeStartHoldMilliseconds: 500,
        });
        await waitForPath(startDecisionPath);
        const target = entry.path(state);
        const value = JSON.parse(readFileSync(target, "utf8"));
        rewriteCanonicalArtifact(target, entry.rewrite(value));

        await assert.rejects(() => execution, entry.error);
        assert.equal(existsSync(hostagentRecordPath(state)), false);
        const sockets = providerSocketPaths(state);
        assert.equal(existsSync(sockets.hostagent), false);
        assert.equal(existsSync(sockets.engine), false);
      } finally {
        await cleanupFixture(state);
      }
    });
  }
});

test("controller death before the durable start request creates no hostagent", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const witnessPath = join(state.evidenceDirectory, "controller-witness.json");
    const execution = launchPlanned(state, 5_000, {
      beforeStartHoldMilliseconds: 500,
    });
    await waitForPath(witnessPath);
    const witness = JSON.parse(readFileSync(witnessPath, "utf8"));
    process.kill(-witness.controller_pgid, "SIGKILL");

    await assert.rejects(
      () => execution,
      /authority checkpoint frontier was refused/,
    );
    await waitForProcessAbsent(witness.controller_pid);
    assert.equal(existsSync(hostagentRecordPath(state)), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("shutdown received during start waits for authenticated hostagent detach", async () => {
  const state = fixture();
  try {
    planControlledBackgroundProviderCreate({
      bindings: createBindings(),
      evidenceDirectory: state.evidenceDirectory,
      fixtureId: state.fixtureId,
      providerBase: state.providerBase,
    });
    const started = await launchPlanned(state, 5_000, {
      requireShutdownDuringStart: true,
    });
    assert.equal(started.controllerSettlement.value.controller_group_absent, true);
    assert.equal(started.controllerSettlement.value.hostagent_disposition,
      "authenticated-running");
    assert.equal(started.controllerSettlement.value.shutdown_during_start, true);
    await plan(state);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("controller settlement retains authenticated process and context evidence", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const evidence = inspectControlledBackgroundProvider(
      state.evidenceDirectory,
      state.fixtureId,
    );
    assert.equal(started.controllerSettlement.value.controller_group_absent, true);
    assert.equal(started.controllerSettlement.value.controller_group_probe, "esrch");
    assert.equal(started.controllerSettlement.value.hostagent_disposition,
      "authenticated-running");
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    assert.notEqual(
      evidence.hostagentWitness.value.socket.inode,
      evidence.engineWitness.value.socket.inode,
    );
    assert.notEqual(
      evidence.hostagentWitness.value.socket.relative_path,
      evidence.engineWitness.value.socket.relative_path,
    );
    assert.equal(
      evidence.contextWitness.value.endpoint,
      `unix://${started.paths.engineSocket}`,
    );
    assert.equal(
      evidence.providerIdentity.value.provider_kind,
      "controlled-background-fake",
    );
    assert.equal(
      evidence.controllerLaunchDecision.value.create_authority_sha256,
      evidence.createAuthority.sha256,
    );
    assert.equal(
      evidence.controllerWitness.value.controller_launch_decision_sha256,
      evidence.controllerLaunchDecision.sha256,
    );
    assert.equal(
      evidence.providerStartDecision.value.controller_witness_sha256,
      evidence.controllerWitness.sha256,
    );
    assert.equal(
      evidence.hostagentWitness.value.start_decision_sha256,
      evidence.providerStartDecision.sha256,
    );
    assert.equal(
      evidence.controllerSettlement.value.provider_start_decision_sha256,
      evidence.providerStartDecision.sha256,
    );
    assert.equal(
      evidence.providerIdentity.value.create_authority_sha256,
      evidence.createAuthority.sha256,
    );
    assert.doesNotThrow(() =>
      inspectControlledBackgroundProvider(state.evidenceDirectory, state.fixtureId, {
        expectedCreateBindings: createBindings(),
      }),
    );
    assert.deepEqual(
      evidence.toolchain.value.components.map((component) => component.role),
      ["controller-script", "hostagent-script", "node-runtime"],
    );
    await plan(state);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("current-source reproof rejects a coherent toolchain role-path substitution", async () => {
  const state = fixture();
  const artifactNames = [
    "background-toolchain.json",
    "controller-launch-decision.json",
    "controller-witness.json",
    "provider-start-decision.json",
    "hostagent-witness.json",
    "engine-witness.json",
    "controller-settlement.json",
    "provider-identity.json",
  ];
  let originals;
  try {
    const started = await launch(state);
    originals = new Map(
      artifactNames.map((name) => [name, readFileSync(join(state.evidenceDirectory, name))]),
    );
    const replacementPath = join(state.root, "controller-source-copy.mjs");
    writeDurable(
      replacementPath,
      readFileSync(
        started.toolchain.value.components.find(
          (component) => component.role === "controller-script",
        ).path,
      ),
    );
    substituteControllerToolchainPath(state.evidenceDirectory, replacementPath);

    assert.doesNotThrow(() =>
      inspectControlledBackgroundProvider(state.evidenceDirectory, state.fixtureId),
    );
    await assert.rejects(() => plan(state), /toolchain component changed/);

    for (const [name, bytes] of originals) {
      const path = join(state.evidenceDirectory, name);
      unlinkDurable(path);
      writeDurable(path, bytes);
    }
    unlinkDurable(replacementPath);
    await plan(state);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("terminal provider identity cannot contradict its creation witness chain", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const identityPath = join(state.evidenceDirectory, "provider-identity.json");
    const original = readFileSync(identityPath);
    const identity = JSON.parse(original);
    const contextEntry = identity.provider_root_inventory.find(
      (entry) => entry.relative_path.endsWith("meta.json"),
    );
    contextEntry.inode = String(BigInt(contextEntry.inode) + 1n);
    rewriteCanonicalArtifact(identityPath, identity);

    assert.throws(
      () => inspectControlledBackgroundProvider(state.evidenceDirectory, state.fixtureId),
      /provider identity was refused/,
    );

    unlinkDurable(identityPath);
    writeDurable(identityPath, original);
    await plan(state);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("context evidence cannot substitute a foreign endpoint with the same suffix", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const contextPath = join(state.evidenceDirectory, "context-witness.json");
    const identityPath = join(state.evidenceDirectory, "provider-identity.json");
    const originalContext = readFileSync(contextPath);
    const originalIdentity = readFileSync(identityPath);
    const context = JSON.parse(originalContext);
    context.endpoint = [
      "unix:///foreign",
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.root_layout.COLIMA_HOME,
      started.paths.profile,
      "docker.sock",
    ].join("/");
    const contextSha256 = rewriteCanonicalArtifact(contextPath, context);
    const identity = JSON.parse(originalIdentity);
    identity.context_witness_sha256 = contextSha256;
    rewriteCanonicalArtifact(identityPath, identity);

    assert.throws(
      () => inspectControlledBackgroundProvider(state.evidenceDirectory, state.fixtureId),
      /context witness was refused/,
    );

    unlinkDurable(contextPath);
    writeDurable(contextPath, originalContext);
    unlinkDurable(identityPath);
    writeDurable(identityPath, originalIdentity);
    await plan(state);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement keeps separate immutable create and cleanup evidence heads", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const planned = await plan(state);
    assert.equal(
      planned.plan.value.create_operation_evidence_sha256,
      started.providerIdentity.sha256,
    );
    assert.equal(planned.plan.value.cleanup_slot_sha256, bindings().cleanup_slot_sha256);
    assert.equal(planned.plan.value.state_integration, "not-authorized");
    const retired = await retire(state);
    assert.equal(
      retired.create_operation_evidence_sha256,
      started.providerIdentity.sha256,
    );
    assert.equal(
      controlledBackgroundOperationEvidence({
        action: "provider-create",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      retired.create_operation_evidence_sha256,
    );
    assert.equal(
      controlledBackgroundOperationEvidence({
        action: "provider-cleanup",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      retired.cleanup_operation_evidence_sha256,
    );
    assert.equal(
      controlledBackgroundOperationEvidence({
        action: "provider-cleanup",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      retired.cleanup_operation_evidence_sha256,
    );
    assert.notEqual(
      retired.create_operation_evidence_sha256,
      retired.cleanup_operation_evidence_sha256,
    );
    assert.equal(retired.settlement.value.result_receipt_authorized, false);
    assert.equal(retired.settlement.value.source_closure, "state-integration-required");
    assert.equal(retired.settlement.value.resources.hostagent_socket, "retired");
    assert.equal(retired.settlement.value.resources.engine_socket, "retired");
    const repeated = await retire(state);
    assert.equal(
      repeated.cleanup_operation_evidence_sha256,
      retired.cleanup_operation_evidence_sha256,
    );
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement refuses a create slot not bound by the launch authority", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const mismatched = {
      ...bindings(),
      create_slot_sha256: "f".repeat(64),
    };
    assert.notEqual(
      mismatched.create_slot_sha256,
      createBindings().create_slot_sha256,
    );
    await assert.rejects(() => plan(state, mismatched), /create slot binding was refused/);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    await plan(state);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement resumes after hostagent settlement without replaying provider start", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    const partial = await retire(state, { stopAfterSequence: 0 });
    assert.equal(partial.complete, false);
    assert.equal(partial.completed_steps, 1);
    assert.equal(existsSync(started.paths.haSocket), false);
    assert.equal(existsSync(started.paths.engineSocket), false);
    assert.equal(existsSync(started.paths.root), true);
    assert.deepEqual(
      readdirSync(state.evidenceDirectory).filter((name) => name.startsWith("retirement-step-")),
      ["retirement-step-00.json"],
    );
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    assert.equal(existsSync(started.paths.root), false);
    assert.equal(
      readdirSync(state.evidenceDirectory).filter((name) => name === "background-toolchain.json")
        .length,
      1,
    );
  } finally {
    await cleanupFixture(state);
  }
});

test("publication resumes complete pre-link stages and discards its partial stage", async () => {
  const state = fixture();
  try {
    await launch(state);
    await plan(state);
    const name = "provider-retirement-plan.json";
    const finalPath = join(state.evidenceDirectory, name);
    const bytes = readFileSync(finalPath);
    unlinkDurable(finalPath);
    const completeStage = join(
      state.evidenceDirectory,
      artifactStageName(name, bytes, "1".repeat(32)),
    );
    const partialStage = join(
      state.evidenceDirectory,
      artifactStageName(name, bytes, "2".repeat(32)),
    );
    writeDurable(completeStage, bytes);
    writeDurable(partialStage, bytes.subarray(0, Math.max(1, bytes.length - 1)));
    await plan(state);
    assert.deepEqual(readFileSync(finalPath), bytes);
    assert.equal(existsSync(completeStage), false);
    assert.equal(existsSync(partialStage), false);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("artifact publication retires the exact link left after final publication", async () => {
  const state = fixture();
  try {
    await launch(state);
    await plan(state);
    const name = "provider-retirement-plan.json";
    const finalPath = join(state.evidenceDirectory, name);
    const bytes = readFileSync(finalPath);
    const stagePath = join(state.evidenceDirectory, artifactStageName(name, bytes));
    linkSync(finalPath, stagePath);
    assert.equal(lstatSync(finalPath, { bigint: true }).nlink, 2n);
    await retire(state);
    assert.equal(existsSync(stagePath), false);
    assert.equal(lstatSync(finalPath, { bigint: true }).nlink, 1n);
  } finally {
    await cleanupFixture(state);
  }
});

test("foreign publication stages are preserved and block provider mutation", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    const name = "provider-retirement-plan.json";
    const finalPath = join(state.evidenceDirectory, name);
    const bytes = readFileSync(finalPath);
    const wrongDigestStage = join(
      state.evidenceDirectory,
      `.provider-process-stage-provider-retirement-plan-${"f".repeat(64)}-${"3".repeat(32)}`,
    );
    writeDurable(wrongDigestStage, bytes);
    await assert.rejects(() => retire(state), /stage digest was refused/);
    assert.equal(existsSync(wrongDigestStage), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    unlinkDurable(wrongDigestStage);

    const foreign = join(state.root, "foreign-stage-source");
    writeDurable(foreign, bytes);
    const linkedStage = join(
      state.evidenceDirectory,
      artifactStageName(name, bytes, "4".repeat(32)),
    );
    linkSync(foreign, linkedStage);
    await assert.rejects(() => retire(state), /stage link was foreign/);
    assert.equal(existsSync(foreign), true);
    assert.equal(existsSync(linkedStage), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    unlinkDurable(linkedStage);
    await retire(state);
    assert.deepEqual(readFileSync(foreign), bytes);
  } finally {
    await cleanupFixture(state);
  }
});

test("evidence inventory capacity is bounded before artifact parsing", () => {
  const state = fixture();
  try {
    const bytes = providerProcessBytes({ schema: "inert-stage-fixture" });
    for (let index = 0; index < 97; index += 1) {
      writeFileSync(
        join(
          state.evidenceDirectory,
          artifactStageName(
            "provider-retirement-plan.json",
            bytes,
            index.toString(16).padStart(32, "0"),
          ),
        ),
        bytes,
        { mode: 0o600 },
      );
    }
    assert.throws(
      () => inspectControlledBackgroundProvider(state.evidenceDirectory, state.fixtureId),
      /evidence capacity was exceeded/,
    );
  } finally {
    rmSync(state.root, { force: true, recursive: true });
  }
});

test("a settlement collision is preserved before hostagent or provider mutation", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    const settlement = join(
      state.evidenceDirectory,
      "provider-retirement-settlement.json",
    );
    writeDurable(settlement, providerProcessBytes({ schema: "foreign-settlement" }));
    await assert.rejects(() => retire(state), /settlement preceded completion/);
    assert.equal(existsSync(settlement), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    unlinkDurable(settlement);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("progress-stage collisions block their exact effect before mutation", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const planned = await plan(state);
    const wrongDigest = "e".repeat(64);
    const futureStage = join(
      state.evidenceDirectory,
      `.provider-process-stage-retirement-step-01-${wrongDigest}-${"4".repeat(32)}`,
    );
    writeDurable(futureStage, providerProcessBytes({ schema: "foreign-progress" }));
    await assert.rejects(() => retire(state), /progress stage was not the next slot/);
    assert.equal(existsSync(futureStage), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    unlinkDurable(futureStage);

    const stepZeroStage = join(
      state.evidenceDirectory,
      `.provider-process-stage-retirement-step-00-${wrongDigest}-${"5".repeat(32)}`,
    );
    writeDurable(stepZeroStage, providerProcessBytes({ schema: "foreign-progress" }));
    await assert.rejects(() => retire(state), /progress stage preceded mutation/);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    unlinkDurable(stepZeroStage);

    await retire(state, { stopAfterSequence: 0 });
    const step = planned.plan.value.retirement_steps[1];
    const resource = join(started.paths.root, step.resources[0]);
    const stepOneStage = join(
      state.evidenceDirectory,
      `.provider-process-stage-retirement-step-01-${wrongDigest}-${"6".repeat(32)}`,
    );
    writeDurable(stepOneStage, providerProcessBytes({ schema: "foreign-progress" }));
    await assert.rejects(() => retire(state), /progress stage preceded mutation/);
    assert.equal(existsSync(resource), true);
    unlinkDurable(stepOneStage);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("a progress stage outside the immutable plan blocks cleanup evidence", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    const stage = join(
      state.evidenceDirectory,
      `.provider-process-stage-retirement-step-99-${"d".repeat(64)}-${"7".repeat(32)}`,
    );
    writeDurable(stage, providerProcessBytes({ schema: "foreign-progress" }));
    await assert.rejects(() => retire(state), /progress stage exceeded its plan/);
    assert.equal(existsSync(stage), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.throws(
      () => controlledBackgroundOperationEvidence({
        action: "provider-cleanup",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      /provider-retirement-settlement.json was unavailable/,
    );
    unlinkDurable(stage);
    await retire(state);
    writeDurable(stage, providerProcessBytes({ schema: "foreign-progress" }));
    assert.throws(
      () => controlledBackgroundOperationEvidence({
        action: "provider-cleanup",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      /progress stage exceeded its plan/,
    );
    unlinkDurable(stage);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement recovers a durable hostagent stop before its progress append", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    await assert.rejects(
      () => retire(state, { crashAfterHostagentSettlement: true }),
      (error) =>
        error instanceof ProviderProcessContractFailure &&
        error.exitStatus === 75 &&
        /hostagent-settlement/.test(error.message),
    );
    assert.equal(existsSync(started.paths.haSocket), false);
    assert.equal(existsSync(started.paths.engineSocket), false);
    assert.equal(
      existsSync(join(state.evidenceDirectory, "retirement-step-00.json")),
      false,
    );
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-00.json"), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement planning recovers an exact pre-plan abrupt hostagent death", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    process.kill(started.hostagentPid, "SIGKILL");
    await waitForProcessAbsent(started.hostagentPid);

    const planned = await plan(state);
    assert.equal(planned.plan.value.root_inventory.length > 0, true);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-00.json"), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement planning recovers exact pre-plan graceful expiry", async () => {
  const state = fixture();
  try {
    const started = await launch(state, 3_000);
    await waitForProcessAbsent(started.hostagentPid);
    assert.equal(existsSync(started.paths.haSocket), false);
    assert.equal(existsSync(started.paths.engineSocket), false);
    const expired = inspectControlledBackgroundProviderPrefix(
      state.evidenceDirectory,
      state.fixtureId,
      { expectedCreateBindings: createBindings(), providerBase: state.providerBase },
    );
    assert.equal(expired.evidenceStage, "provider-identity");
    assert.equal(expired.residual.hostagent_presence, "proved-absent");
    assert.equal(expired.residual.sockets, "absent");

    await plan(state);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-00.json"), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement removes only exact stale sockets after abrupt hostagent death", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    process.kill(started.hostagentPid, "SIGKILL");
    await waitForProcessAbsent(started.hostagentPid);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-00.json"), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement converges one missing and one exact stale socket", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    process.kill(started.hostagentPid, "SIGKILL");
    await waitForProcessAbsent(started.hostagentPid);
    unlinkDurable(started.paths.haSocket);
    assert.equal(existsSync(started.paths.engineSocket), true);
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("retirement repairs an exact delete-before-progress crash and no broader state", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const planned = await plan(state);
    const crashSequence = 1;
    const resource = planned.plan.value.retirement_steps[crashSequence].resources[0];
    await assert.rejects(
      () => retire(state, { crashAfterDeleteSequence: crashSequence }),
      (error) =>
        error instanceof ProviderProcessContractFailure &&
        error.exitStatus === 75 &&
        /simulated/.test(error.message),
    );
    assert.equal(existsSync(join(started.paths.root, resource)), false);
    assert.equal(
      existsSync(join(state.evidenceDirectory, "retirement-step-01.json")),
      false,
    );
    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const repaired = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-01.json"), "utf8"),
    );
    assert.equal(repaired.recovered_absence, true);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("recovered file absence is fsynced before progress publication", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const planned = await plan(state);
    const crashSequence = 1;
    const resource = planned.plan.value.retirement_steps[crashSequence].resources[0];
    await assert.rejects(
      () => retire(state, { crashAfterDeleteSyscallSequence: crashSequence }),
      (error) =>
        error instanceof ProviderProcessContractFailure &&
        error.exitStatus === 75 &&
        /pre-fsync/.test(error.message),
    );
    assert.equal(existsSync(join(started.paths.root, resource)), false);
    assert.equal(
      existsSync(join(state.evidenceDirectory, "retirement-step-01.json")),
      false,
    );

    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, "retirement-step-01.json"), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
  } finally {
    await cleanupFixture(state);
  }
});

test("recovered root absence is fsynced before terminal progress", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const planned = await plan(state);
    const crashSequence = planned.plan.value.retirement_steps.length - 1;
    await assert.rejects(
      () => retire(state, { crashAfterDeleteSyscallSequence: crashSequence }),
      (error) =>
        error instanceof ProviderProcessContractFailure &&
        error.exitStatus === 75 &&
        /pre-fsync/.test(error.message),
    );
    assert.equal(existsSync(started.paths.root), false);
    const progressName = `retirement-step-${String(crashSequence).padStart(2, "0")}.json`;
    assert.equal(existsSync(join(state.evidenceDirectory, progressName)), false);

    const retired = await retire(state);
    assert.equal(retired.complete, true);
    const recovered = JSON.parse(
      readFileSync(join(state.evidenceDirectory, progressName), "utf8"),
    );
    assert.equal(recovered.recovered_absence, true);
  } finally {
    await cleanupFixture(state);
  }
});

test("an unknown retirement leaf is preserved and blocks the first deletion", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const foreign = join(started.paths.root, "foreign");
    writeFileSync(foreign, "preserve\n", { mode: 0o600 });
    await assert.rejects(
      () => plan(state),
      /provider creation identity changed|root inventory was refused/,
    );
    assert.equal(readFileSync(foreign, "utf8"), "preserve\n");
    assert.equal(existsSync(started.paths.haSocket), true);
    rmSync(foreign);
    await plan(state);
    await retire(state);
    assert.equal(existsSync(started.paths.root), false);
  } finally {
    await cleanupFixture(state);
  }
});

test("a pre-plan file replacement is preserved and grants no deletion authority", async () => {
  const state = fixture();
  let started;
  try {
    started = await launch(state);
    const original = readFileSync(started.paths.contextFile);
    unlinkDurable(started.paths.contextFile);
    writeDurable(started.paths.contextFile, original);

    await assert.rejects(() => plan(state), /provider creation identity changed/);
    assert.equal(existsSync(started.paths.contextFile), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
  } finally {
    if (started !== undefined && processPresent(started.hostagentPid)) {
      process.kill(started.hostagentPid, "SIGKILL");
      await waitForProcessAbsent(started.hostagentPid);
    }
    await cleanupFixture(state);
  }
});

test("a pre-plan directory replacement is preserved and grants no deletion authority", async () => {
  const state = fixture();
  let started;
  try {
    started = await launch(state);
    const cacheRoot = join(
      started.paths.root,
      CONTROLLED_BACKGROUND_PROVIDER_CONTRACT.root_layout.COLIMA_CACHE_HOME,
    );
    rmdirSync(cacheRoot);
    mkdirSync(cacheRoot, { mode: 0o700 });
    chmodSync(cacheRoot, 0o700);

    await assert.rejects(() => plan(state), /provider creation identity changed/);
    assert.equal(existsSync(cacheRoot), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
  } finally {
    if (started !== undefined && processPresent(started.hostagentPid)) {
      process.kill(started.hostagentPid, "SIGKILL");
      await waitForProcessAbsent(started.hostagentPid);
    }
    await cleanupFixture(state);
  }
});

test("a post-plan hard-link alias is preserved and blocks hostagent shutdown", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    const alias = join(started.paths.root, "foreign-context-alias");
    linkSync(started.paths.contextFile, alias);
    await assert.rejects(
      () => retire(state),
      /inventory identity was refused|retirement subset was refused/,
    );
    assert.equal(existsSync(alias), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
    rmSync(alias);
    await retire(state);
  } finally {
    await cleanupFixture(state);
  }
});

test("an unplanned symlink is preserved and cannot enter a retirement inventory", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    const target = join(state.root, "foreign-target");
    writeFileSync(target, "preserve\n", { mode: 0o600 });
    const alias = join(started.paths.root, "foreign-link");
    symlinkSync(target, alias);
    await assert.rejects(() => plan(state), /inventory identity was refused/);
    assert.equal(readFileSync(target, "utf8"), "preserve\n");
    assert.equal(existsSync(alias), true);
    rmSync(alias);
    await plan(state);
    await retire(state);
    assert.equal(readFileSync(target, "utf8"), "preserve\n");
  } finally {
    await cleanupFixture(state);
  }
});

test("a post-plan context replacement is preserved and blocks retirement", async () => {
  const state = fixture();
  try {
    const started = await launch(state, 2_000);
    await plan(state);
    const original = readFileSync(started.paths.contextFile);
    rmSync(started.paths.contextFile);
    writeFileSync(started.paths.contextFile, original, { mode: 0o600 });
    await assert.rejects(() => retire(state), /retirement identity changed/);
    assert.deepEqual(readFileSync(started.paths.contextFile), original);
    assert.equal(existsSync(started.paths.haSocket), true);
    await waitForProcessAbsent(started.hostagentPid);
  } finally {
    await cleanupFixture(state);
  }
});

test("a rewritten plan cannot adopt a post-creation leaf replacement", async () => {
  const state = fixture();
  let started;
  try {
    started = await launch(state);
    await plan(state);
    const contextBytes = readFileSync(started.paths.contextFile);
    unlinkDurable(started.paths.contextFile);
    writeDurable(started.paths.contextFile, contextBytes);

    const planPath = join(state.evidenceDirectory, "provider-retirement-plan.json");
    const planValue = JSON.parse(readFileSync(planPath, "utf8"));
    const contextRelativePath = relative(started.paths.root, started.paths.contextFile);
    const contextEntry = planValue.root_inventory.find(
      (entry) => entry.relative_path === contextRelativePath,
    );
    contextEntry.inode = String(lstatSync(started.paths.contextFile, { bigint: true }).ino);
    rewriteCanonicalArtifact(planPath, planValue);

    await assert.rejects(() => retire(state), /retirement plan was refused/);
    assert.equal(existsSync(started.paths.contextFile), true);
    assert.equal(existsSync(started.paths.haSocket), true);
    assert.equal(existsSync(started.paths.engineSocket), true);
  } finally {
    if (started !== undefined && processPresent(started.hostagentPid)) {
      process.kill(started.hostagentPid, "SIGKILL");
      await waitForProcessAbsent(started.hostagentPid);
    }
    await cleanupFixture(state);
  }
});

test("retirement evidence cannot be transplanted across provider identities", async () => {
  const first = fixture();
  const second = fixture();
  try {
    await launch(first);
    await launch(second);
    await plan(first, bindings("3"));
    const firstPlan = readFileSync(
      join(first.evidenceDirectory, "provider-retirement-plan.json"),
    );
    const transplanted = join(second.evidenceDirectory, "provider-retirement-plan.json");
    writeFileSync(transplanted, firstPlan, { mode: 0o600 });
    await assert.rejects(() => retire(second), /retirement plan was refused/);
    const secondRoot = join(second.providerBase, `svb-${second.fixtureId.slice(0, 12)}`);
    assert.equal(existsSync(secondRoot), true);
    rmSync(transplanted);
    await plan(second, bindings("5"));
    await retire(second);
    await retire(first);
  } finally {
    await cleanupFixture(first);
    await cleanupFixture(second);
  }
});

test("provider-root recreation invalidates cleanup evidence", async () => {
  const state = fixture();
  try {
    const started = await launch(state);
    await plan(state);
    await retire(state);
    mkdirSync(started.paths.root, { mode: 0o700 });
    writeFileSync(join(started.paths.root, "foreign"), "preserve\n", { mode: 0o600 });
    assert.throws(
      () => controlledBackgroundOperationEvidence({
        action: "provider-cleanup",
        evidenceDirectory: state.evidenceDirectory,
        fixtureId: state.fixtureId,
      }),
      /provider root reappeared/,
    );
    assert.equal(readFileSync(join(started.paths.root, "foreign"), "utf8"), "preserve\n");
  } finally {
    await cleanupFixture(state);
  }
});

test("the retirement implementation contains no recursive removal primitive", () => {
  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(source, /\brmSync\b|\brm\s+-rf\b|\bremoveAll\b/);
  assert.match(source, /unlinkSync\(/);
  assert.match(source, /rmdirSync\(/);
});

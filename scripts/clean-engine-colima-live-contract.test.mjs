import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  readlinkSync,
  renameSync,
  rmSync,
  rmdirSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, relative, sep } from "node:path";
import { test } from "node:test";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
  COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS,
  COLIMA_LIVE_REQUIREMENTS_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS_SHA256,
  ColimaLiveContractFailure,
  authorizeColimaLiveObservationForTest,
  buildColimaLiveObservationForTest,
  colimaLiveBytes,
  colimaLiveDigest,
  colimaLivePublicProjectionForTest,
  observeColimaLivePreEffectRootsForTest,
  observeColimaLiveProviderReservationRootsForTest,
  revalidateColimaLiveObservationForTest,
  validateColimaLiveObservationForTest,
  validateColimaLiveRequirements,
} from "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";
import {
  LiveProviderPlanFailure,
  buildColimaLiveProviderOperationPlan,
} from "../deploy/compose/scripts/clean-engine-live-provider-plan.mjs";
import {
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CREATE_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_CLASS,
} from "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs";
import {
  cloneColimaLiveObservationInput,
  createCleanEngineColimaLiveObservationFixture,
  writePrivateColimaLiveFixtureFile,
} from "./fixtures/clean-engine-colima-live-observation-fixture.mjs";

function digest(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

function clone(value) {
  return structuredClone(value);
}

function cloneInput(input) {
  return cloneColimaLiveObservationInput(input);
}

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof ColimaLiveContractFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

function writePrivate(path, bytes, mode) {
  writePrivateColimaLiveFixtureFile(path, bytes, mode);
}

function fixture(t) {
  const state = createCleanEngineColimaLiveObservationFixture();
  t.after(() => rmSync(state.root, { force: true, recursive: true }));
  return state;
}

function build(state) {
  return buildColimaLiveObservationForTest(state.requirements, state.input);
}

function snapshotTree(root, opaqueDirectories = new Set()) {
  const entries = [];
  function visit(path) {
    const metadata = lstatSync(path, { bigint: true });
    const kind = metadata.isDirectory()
      ? "directory"
      : metadata.isFile()
        ? "file"
        : metadata.isSymbolicLink()
          ? "symlink"
          : "other";
    entries.push({
      ctime_nanoseconds: String(metadata.ctimeNs),
      device: String(metadata.dev),
      gid: String(metadata.gid),
      inode: String(metadata.ino),
      kind,
      links: String(metadata.nlink),
      mode: (metadata.mode & 0o7777n).toString(8).padStart(4, "0"),
      mtime_nanoseconds: String(metadata.mtimeNs),
      path: relative(root, path) || ".",
      sha256: metadata.isFile()
        ? digest("sha256", readFileSync(path))
        : undefined,
      size: String(metadata.size),
      symlink_target: metadata.isSymbolicLink() ? readlinkSync(path) : undefined,
      uid: String(metadata.uid),
    });
    if (metadata.isDirectory() && !opaqueDirectories.has(path)) {
      for (const name of readdirSync(path).sort()) visit(join(path, name));
    }
  }
  visit(root);
  return entries;
}

function mutationNamespaces(state) {
  return [
    ["colima-cache-namespace", state.input.environment.COLIMA_CACHE_HOME],
    ["colima-home-namespace", state.input.environment.COLIMA_HOME],
    ["docker-config-namespace", state.input.environment.DOCKER_CONFIG],
    ["lima-home-namespace", state.input.environment.LIMA_HOME],
    ["private-home-namespace", state.input.environment.HOME],
    ["temporary-namespace", state.input.environment.TMPDIR],
  ].map(([role, path]) => ({ path, role }));
}

test("production live requirements are exact, pinned and execution-disabled", () => {
  assert.equal(COLIMA_LIVE_REQUIREMENTS.schema, COLIMA_LIVE_REQUIREMENTS_SCHEMA);
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS_SCHEMA,
    "synveda.clean-engine.colima-live-requirements.v4",
  );
  assert.equal(
    COLIMA_LIVE_OBSERVATION_SCHEMA,
    "synveda.clean-engine.colima-live-observation.v4",
  );
  assert.equal(
    COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA,
    "synveda.clean-engine.colima-live-public-projection.v4",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-pre-effect-root-observation.v4",
  );
  assert.equal(
    COLIMA_LIVE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
    "synveda.clean-engine.colima-live-pre-effect-root-observation.v4",
  );
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.execution_authorized, false);
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.lifecycle_exposure_authorized, false);
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.finalization_eligible, false);
  assert.ok(COLIMA_LIVE_REQUIREMENTS.command_template.includes("--activate=false"));
  assert.ok(COLIMA_LIVE_REQUIREMENTS.command_template.includes("grpc"));
  assert.ok(COLIMA_LIVE_REQUIREMENTS.command_template.includes("192.168.5.2"));
  assert.deepEqual(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.namespaces.map(
      (entry) => entry.role,
    ),
    COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  );
  assert.deepEqual(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.namespaces.map((entry) => [
      entry.role,
      entry.effect_policy,
    ]),
    [
      ["colima-cache-namespace", "expected-no-write-v1"],
      ["colima-home-namespace", "expected-owned-mutation-v1"],
      ["docker-config-namespace", "expected-owned-mutation-v1"],
      ["lima-home-namespace", "expected-owned-mutation-v1"],
      ["private-home-namespace", "expected-no-write-v1"],
      ["temporary-namespace", "expected-owned-mutation-v1"],
    ],
  );
  assert.deepEqual(COLIMA_LIVE_REQUIREMENTS.host.system_executables, [
    {
      path: "/bin/sh",
      purpose: "lima-wrapper-interpreter",
      trust_policy: "exact-os-build-trusted-boundary-v1",
    },
    {
      path: "/usr/sbin/ioreg",
      purpose: "lima-machine-id-probe",
      trust_policy: "exact-os-build-trusted-boundary-v1",
    },
  ]);
  assert.deepEqual(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.derived_child_environment,
    { LIMA_SSH_PORT_FORWARDER: "false" },
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.maximum_entry_name_bytes,
    255,
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.maximum_top_level_entries,
    64,
  );
  assert.deepEqual(
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.provider_root_path,
    {
      encoding: "utf8",
      maximum_canonical_bytes: 21,
      policy: "lexical-and-realpath-byte-bound-before-admission-v1",
      provider_root_to_lima_home_bytes: 2,
      upstream_longest_socket_from_lima_home_bytes: 80,
      upstream_refusal_threshold_bytes: 104,
    },
  );
  assert.equal(21 + 2 + 80, 103);
  assert.ok(
    21 +
      COLIMA_LIVE_REQUIREMENTS.mutation_surface.provider_root_path
        .provider_root_to_lima_home_bytes +
      COLIMA_LIVE_REQUIREMENTS.mutation_surface.provider_root_path
        .upstream_longest_socket_from_lima_home_bytes <
      COLIMA_LIVE_REQUIREMENTS.mutation_surface.provider_root_path
        .upstream_refusal_threshold_bytes,
  );
  const guestAgent = COLIMA_LIVE_REQUIREMENTS.components.find(
    (entry) => entry.role === "lima-guestagent",
  );
  const defaultTemplate = COLIMA_LIVE_REQUIREMENTS.components.find(
    (entry) => entry.role === "lima-default-template",
  );
  const networkConfig = COLIMA_LIVE_REQUIREMENTS.components.find(
    (entry) => entry.role === "lima-network-config",
  );
  assert.equal(
    guestAgent.stage_relative_path,
    "share/lima/lima-guestagent.Linux-aarch64.gz",
  );
  assert.deepEqual(
    {
      expected_sha256: defaultTemplate.expected_sha256,
      expected_size: defaultTemplate.expected_size,
      stage_relative_path: defaultTemplate.stage_relative_path,
    },
    {
      expected_sha256:
        "9b36702315b0716108a64631faf62a664d94f8f4a57acf3174f80022b308f5a3",
      expected_size: "34256",
      stage_relative_path: "share/lima/templates/default.yaml",
    },
  );
  assert.equal(networkConfig.mode_policy, "private-mutable-data-0600");
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS.release_artifacts.colima.sha256,
    "980ad8bf61a4ca370243f4cb41401a61276dcd2c2502bee7b9b86f9250169f34",
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS.release_artifacts.lima.sha256,
    "bbdef91774885a0d05f7b048c4eb89ae2bcf3a0c252ae7ca7934e63df76d93c3",
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS.release_artifacts.disk_image.sha256,
    "1fc0354f4f99734ce3886628cc7af8b0437c1a1d391b126bd09cba0df35ee53f",
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS_SHA256,
    colimaLiveDigest(colimaLiveBytes(COLIMA_LIVE_REQUIREMENTS)),
  );
  assert.equal(
    COLIMA_LIVE_REQUIREMENTS_SHA256,
    "f08813ed481d42a6ac5f20ff19dffb812efc53705b3d5f73206ee0cccf118aa4",
  );
  assert.equal(validateColimaLiveRequirements(COLIMA_LIVE_REQUIREMENTS), COLIMA_LIVE_REQUIREMENTS);
  assert.ok(Object.isFrozen(COLIMA_LIVE_REQUIREMENTS));
  assert.ok(Object.isFrozen(COLIMA_LIVE_REQUIREMENTS.components));
});

test("pinned requirements refuse field, provenance, release and authorization drift", () => {
  const mutations = [
    (value) => delete value.host,
    (value) => {
      value.unexpected = true;
    },
    (value) => {
      value.components.reverse();
    },
    (value) => {
      value.components[1].role = value.components[0].role;
    },
    (value) => {
      value.components[0].expected_sha256 = "f".repeat(64);
    },
    (value) => {
      value.release_artifacts.colima.source_revision = "f".repeat(40);
    },
    (value) => {
      value.release_artifacts.lima.sha256 = "f".repeat(64);
    },
    (value) => {
      value.release_artifacts.disk_image.sha512 = "f".repeat(128);
    },
    (value) => {
      value.authorizations.execution_authorized = true;
    },
    (value) => {
      value.environment.home_policy = "external-home";
    },
    (value) => {
      value.host.system_executables[0].path = "/bin/false";
    },
    (value) => {
      value.mutation_surface.derived_child_environment.LIMA_SSH_PORT_FORWARDER =
        "true";
    },
    (value) => {
      value.mutation_surface.maximum_entry_name_bytes = 256;
    },
    (value) => {
      value.mutation_surface.maximum_top_level_entries = 65;
    },
    (value) => {
      value.mutation_surface.provider_root_path.maximum_canonical_bytes = 22;
    },
    (value) => {
      value.mutation_surface.provider_root_path.encoding = "characters";
    },
    (value) => {
      value.mutation_surface.provider_root_path.upstream_refusal_threshold_bytes =
        105;
    },
    (value) => {
      value.mutation_surface.namespaces[0].effect_policy =
        "expected-owned-mutation-v1";
    },
    (value) => {
      value.legacy_preparation_contract_sha256 = "f".repeat(64);
    },
  ];
  for (const mutate of mutations) {
    const requirements = clone(COLIMA_LIVE_REQUIREMENTS);
    mutate(requirements);
    expectRefusal(() => validateColimaLiveRequirements(requirements));
  }
});

test("superseded v1 through v3 preparation generations are refused", (t) => {
  for (const version of ["v1", "v2", "v3"]) {
    const requirements = clone(COLIMA_LIVE_REQUIREMENTS);
    requirements.schema = `synveda.clean-engine.colima-live-requirements.${version}`;
    expectRefusal(() => validateColimaLiveRequirements(requirements));
  }
  const state = fixture(t);
  for (const version of ["v1", "v2", "v3"]) {
    const observation = clone(build(state));
    observation.schema = `synveda.clean-engine.colima-live-observation.${version}`;
    expectRefusal(() =>
      validateColimaLiveObservationForTest(state.requirements, observation),
    );
  }
});

test("fixture requirements refuse missing, duplicate and misplaced component roles", (t) => {
  const state = fixture(t);
  const mutations = [
    (value) => value.components.pop(),
    (value) => {
      value.components[1].role = value.components[0].role;
    },
    (value) => {
      value.components.find(
        (entry) => entry.role === "ssh-keygen",
      ).stage_relative_path = "b/ssh";
    },
    (value) => {
      value.components[0].stage_relative_path = "a/colima";
    },
    (value) => {
      value.components[0].unexpected = true;
    },
  ];
  for (const mutate of mutations) {
    const requirements = clone(state.requirements);
    mutate(requirements);
    expectRefusal(() => buildColimaLiveObservationForTest(requirements, state.input));
  }
});

test("a closed fixture builds, validates and deterministically revalidates", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const maximumRootBytes =
    state.requirements.mutation_surface.provider_root_path
      .maximum_canonical_bytes;
  assert.equal(Buffer.byteLength(state.providerRoot, "utf8"), maximumRootBytes);
  assert.equal(dirname(state.providerRoot), state.root);
  assert.equal(dirname(state.root), realpathSync("/tmp"));
  for (const path of [state.root, state.providerRoot]) {
    const metadata = lstatSync(path, { bigint: true });
    assert.equal(metadata.uid, BigInt(process.getuid()));
    assert.equal(metadata.mode & 0o7777n, 0o700n);
  }
  assert.equal(observation.schema, COLIMA_LIVE_OBSERVATION_SCHEMA);
  assert.equal(observation.requirements_sha256, colimaLiveDigest(colimaLiveBytes(state.requirements)));
  assert.equal(observation.components.length, 13);
  assert.equal(observation.directories.length, 13);
  assert.equal(validateColimaLiveObservationForTest(state.requirements, observation), observation);
  assert.equal(
    revalidateColimaLiveObservationForTest(state.requirements, observation, state.input),
    observation,
  );
});

test("reservation root observation binds the marker disposition and state run", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const stateRun = join(state.root, "state-run");
  mkdirSync(stateRun, { mode: 0o700 });
  const checkpoints = [];
  const absent = observeColimaLiveProviderReservationRootsForTest(
    state.requirements,
    observation,
    state.input,
    stateRun,
    "absent",
    (checkpoint) => {
      checkpoints.push(checkpoint);
    },
  );
  assert.equal(absent.marker_disposition, "absent");
  assert.equal(absent.root_observation.root_set_disposition, "observed-pristine");
  assert.equal(absent.namespace_bindings.length, 6);
  assert.notEqual(
    absent.provider_root_identity.inode,
    absent.state_run_identity.inode,
  );
  assert.deepEqual(checkpoints, [
    "after-first-root-sample",
    "after-reservation-root-observation",
  ]);

  const marker = join(
    state.providerRoot,
    COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
  );
  writeFileSync(marker, "opaque reservation\n", { mode: 0o600 });
  const present = observeColimaLiveProviderReservationRootsForTest(
    state.requirements,
    observation,
    state.input,
    stateRun,
    "present",
  );
  assert.equal(present.marker_disposition, "present");
  assert.deepEqual(present.root_observation, absent.root_observation);
  assert.deepEqual(present.namespace_bindings, absent.namespace_bindings);
  assert.deepEqual(present.provider_root_identity, absent.provider_root_identity);
  assert.deepEqual(present.state_run_identity, absent.state_run_identity);

  const extra = join(state.providerRoot, "unexpected-root-entry");
  writeFileSync(extra, "collision\n", { mode: 0o600 });
  expectRefusal(
    () =>
      observeColimaLiveProviderReservationRootsForTest(
        state.requirements,
        observation,
        state.input,
        stateRun,
        "present",
      ),
    78,
  );
  unlinkSync(extra);
  unlinkSync(marker);

  chmodSync(stateRun, 0o755);
  expectRefusal(
    () =>
      observeColimaLiveProviderReservationRootsForTest(
        state.requirements,
        observation,
        state.input,
        stateRun,
        "absent",
      ),
  );
  chmodSync(stateRun, 0o700);
  const stateRunAlias = join(state.root, "state-run-alias");
  symlinkSync(stateRun, stateRunAlias);
  expectRefusal(
    () =>
      observeColimaLiveProviderReservationRootsForTest(
        state.requirements,
        observation,
        state.input,
        stateRunAlias,
        "absent",
      ),
    78,
  );
});

test("provider-root admission enforces canonical UTF-8 byte length", (t) => {
  const state = fixture(t);
  const expectBoundRefusal = (input, forbiddenValues, exitStatus = 64) => {
    assert.throws(
      () => buildColimaLiveObservationForTest(state.requirements, input),
      (error) => {
        assert.ok(error instanceof ColimaLiveContractFailure);
        assert.equal(error.exitStatus, exitStatus);
        assert.equal(
          error.message,
          "Colima live provider root path bound was refused",
        );
        for (const value of forbiddenValues) {
          assert.equal(error.message.includes(value), false);
        }
        return true;
      },
    );
  };

  const asciiOverflow = cloneInput(state.input);
  asciiOverflow.provider_root = `${state.providerRoot}x`;
  assert.equal(Buffer.byteLength(asciiOverflow.provider_root, "utf8"), 22);
  expectBoundRefusal(asciiOverflow, [asciiOverflow.provider_root, state.root]);

  const unicodeOverflow = cloneInput(state.input);
  unicodeOverflow.provider_root = `/${"é".repeat(10)}x`;
  assert.ok(unicodeOverflow.provider_root.length < 22);
  assert.equal(Buffer.byteLength(unicodeOverflow.provider_root, "utf8"), 22);
  expectBoundRefusal(unicodeOverflow, [unicodeOverflow.provider_root, state.root]);

  const unicodeState = fixture(t);
  const maximumRootBytes =
    unicodeState.requirements.mutation_surface.provider_root_path
      .maximum_canonical_bytes;
  const temporaryRoot = dirname(unicodeState.root);
  const unicodeNameBytes =
    maximumRootBytes - Buffer.byteLength(temporaryRoot, "utf8") - 1;
  const uniqueAscii = basename(unicodeState.root).slice(0, unicodeNameBytes - 2);
  const unicodeRoot = join(temporaryRoot, `${uniqueAscii}¢`);
  assert.equal(Buffer.byteLength(unicodeRoot, "utf8"), maximumRootBytes);
  renameSync(unicodeState.providerRoot, unicodeRoot);
  t.after(() => rmSync(unicodeRoot, { force: true, recursive: true }));
  const rebaseProviderPath = (path) =>
    path === unicodeState.providerRoot ||
    path.startsWith(`${unicodeState.providerRoot}${sep}`)
      ? `${unicodeRoot}${path.slice(unicodeState.providerRoot.length)}`
      : path;
  const unicodeInput = cloneInput(unicodeState.input);
  unicodeInput.provider_root = unicodeRoot;
  unicodeInput.receipt_owned_disk_image_path = rebaseProviderPath(
    unicodeInput.receipt_owned_disk_image_path,
  );
  for (const [role, path] of Object.entries(unicodeInput.component_paths)) {
    unicodeInput.component_paths[role] = rebaseProviderPath(path);
  }
  for (const [name, value] of Object.entries(unicodeInput.environment)) {
    unicodeInput.environment[name] = rebaseProviderPath(value);
  }
  const unicodeObservation = buildColimaLiveObservationForTest(
    unicodeState.requirements,
    unicodeInput,
  );
  assert.equal(
    validateColimaLiveObservationForTest(
      unicodeState.requirements,
      unicodeObservation,
    ),
    unicodeObservation,
  );

  const aliasState = fixture(t);
  const alias = join(aliasState.root, "q");
  symlinkSync(aliasState.providerRoot, alias);
  const aliasInput = cloneInput(aliasState.input);
  aliasInput.provider_root = alias;
  aliasInput.receipt_owned_disk_image_path = join(
    alias,
    aliasState.requirements.source_disk_image.copy_relative_path,
  );
  expectRefusal(
    () =>
      buildColimaLiveObservationForTest(aliasState.requirements, aliasInput),
    78,
  );

  const serialized = clone(build(aliasState));
  const serializedRoot = `/${"s".repeat(maximumRootBytes)}`;
  const providerDirectory = serialized.directories.find(
    (entry) => entry.role === "provider-root",
  );
  providerDirectory.path = serializedRoot;
  serialized.provider_root_identity_sha256 = colimaLiveDigest(
    colimaLiveBytes(providerDirectory),
  );
  assert.throws(
    () =>
      validateColimaLiveObservationForTest(aliasState.requirements, serialized),
    (error) => {
      assert.ok(error instanceof ColimaLiveContractFailure);
      assert.equal(error.exitStatus, 78);
      assert.equal(error.message, "Colima live provider root path bound was refused");
      assert.equal(error.message.includes(serializedRoot), false);
      return true;
    },
  );

  const longPhysicalRoot = join(state.root, "long-provider-root");
  renameSync(state.providerRoot, longPhysicalRoot);
  symlinkSync(longPhysicalRoot, state.providerRoot);
  assert.ok(Buffer.byteLength(state.providerRoot, "utf8") <= 21);
  assert.ok(Buffer.byteLength(realpathSync(state.providerRoot), "utf8") > 21);
  expectBoundRefusal(state.input, [longPhysicalRoot, state.root], 69);
});

test("pre-effect observation binds all six complete mutation namespaces", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const before = snapshotTree(state.root);
  const result = observeColimaLivePreEffectRootsForTest(
    state.requirements,
    observation,
    state.input,
  );
  assert.deepEqual(snapshotTree(state.root), before);
  assert.deepEqual(Object.keys(result).sort(), [
    "evidence_class",
    "planned_names",
    "preparation_observation_sha256",
    "requirements_sha256",
    "root_observations",
    "root_set_disposition",
    "schema",
  ]);
  assert.equal(
    result.schema,
    COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
  );
  assert.equal(result.evidence_class, "fixture-only");
  assert.equal(result.root_set_disposition, "observed-pristine");
  assert.deepEqual(result.planned_names, {
    lima_instance: `colima-${state.input.provider_profile}`,
    provider_profile: state.input.provider_profile,
  });
  assert.equal(
    result.preparation_observation_sha256,
    colimaLiveDigest(colimaLiveBytes(observation)),
  );
  assert.equal(
    result.requirements_sha256,
    colimaLiveDigest(colimaLiveBytes(state.requirements)),
  );
  assert.deepEqual(
    result.root_observations.map((entry) => [entry.role, entry.disposition]),
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.map((role) => [
      role,
      "observed-pristine",
    ]),
  );
  for (const entry of result.root_observations) {
    assert.deepEqual(Object.keys(entry).sort(), [
      "disposition",
      "namespace_identity_hmac_sha256",
      "observed_entry_set_hmac_sha256",
      "role",
    ]);
    assert.match(entry.namespace_identity_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.match(entry.observed_entry_set_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.notEqual(entry.namespace_identity_hmac_sha256, "0".repeat(64));
    assert.notEqual(entry.observed_entry_set_hmac_sha256, "0".repeat(64));
    assert.ok(Object.isFrozen(entry));
  }
  assert.ok(Object.isFrozen(result));
  assert.ok(Object.isFrozen(result.planned_names));
  assert.ok(Object.isFrozen(result.root_observations));
  const serialized = colimaLiveBytes(result).toString("utf8");
  for (const forbidden of [
    state.root,
    state.home,
    state.providerRoot,
    "binding_key",
    "command",
    "DOCKER_CONFIG",
    "HOME",
    "PID",
    "PGID",
    "socket",
  ]) {
    assert.equal(serialized.includes(forbidden), false, forbidden);
  }
});

test("file, link and directory entries are opaque foreign collisions", (t) => {
  const cases = ["file", "hardlink", "directory", "symlink", "dangling-symlink"];
  for (const [index, kind] of cases.entries()) {
    const state = fixture(t);
    const observation = build(state);
    const namespace = mutationNamespaces(state)[index];
    const target = join(namespace.path, `foreign-${kind}`);
    const outside = join(state.root, `outside-${kind}`);
    let protectedDirectory;
    let protectedSentinel;
    if (kind === "file") {
      writePrivate(target, Buffer.from("foreign file\n", "utf8"), 0o600);
    } else if (kind === "hardlink") {
      writePrivate(outside, Buffer.from("foreign hardlink\n", "utf8"), 0o600);
      linkSync(outside, target);
    } else if (kind === "directory") {
      mkdirSync(target, { mode: 0o700 });
      protectedDirectory = target;
      protectedSentinel = join(target, "must-not-be-read");
      writePrivate(protectedSentinel, Buffer.from("opaque sentinel\n", "utf8"), 0o600);
      chmodSync(target, 0o000);
    } else if (kind === "symlink") {
      mkdirSync(outside, { mode: 0o700 });
      protectedSentinel = join(outside, "must-not-be-read");
      writePrivate(protectedSentinel, Buffer.from("outside sentinel\n", "utf8"), 0o600);
      symlinkSync(outside, target);
    } else {
      symlinkSync(join(state.root, "missing-outside-target"), target);
    }

    const opaqueDirectories = new Set(
      protectedDirectory === undefined ? [] : [protectedDirectory],
    );
    const fixtureBefore = snapshotTree(state.root, opaqueDirectories);
    const outsideBefore = existsSync(outside)
      ? snapshotTree(outside)
      : undefined;
    const result = observeColimaLivePreEffectRootsForTest(
      state.requirements,
      observation,
      state.input,
    );
    assert.equal(result.root_set_disposition, "foreign-collision", kind);
    const collision = result.root_observations.find(
      (entry) => entry.disposition === "foreign-collision",
    );
    assert.notEqual(collision, undefined, kind);
    assert.equal(collision.role, namespace.role);
    assert.match(collision.observed_entry_set_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.notEqual(collision.observed_entry_set_hmac_sha256, "0".repeat(64));
    assert.equal(lstatSync(target).isSymbolicLink(), kind.includes("symlink"));
    assert.deepEqual(
      snapshotTree(state.root, opaqueDirectories),
      fixtureBefore,
    );
    if (outsideBefore !== undefined) {
      assert.deepEqual(snapshotTree(outside), outsideBefore);
    }
    if (protectedDirectory !== undefined) {
      chmodSync(protectedDirectory, 0o700);
      assert.equal(
        readFileSync(protectedSentinel, "utf8"),
        "opaque sentinel\n",
      );
    }
  }
});

test("namespace collisions accept exact bounds and refuse excess", (t) => {
  const all = fixture(t);
  const allObservation = build(all);
  for (const [index, namespace] of mutationNamespaces(all).entries()) {
    writePrivate(
      join(namespace.path, `foreign-${index}`),
      Buffer.from(`foreign ${index}\n`, "utf8"),
      0o600,
    );
  }
  const collision = observeColimaLivePreEffectRootsForTest(
    all.requirements,
    allObservation,
    all.input,
  );
  assert.deepEqual(
    collision.root_observations.map((entry) => entry.disposition),
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.map(() => "foreign-collision"),
  );

  const atLimit = fixture(t);
  const atLimitObservation = build(atLimit);
  const atLimitNamespace = mutationNamespaces(atLimit)[0];
  const boundaryNames = [
    "x".repeat(255),
    ...Array.from(
      { length: 63 },
      (_, index) => `foreign-${String(index).padStart(2, "0")}`,
    ),
  ];
  for (const [index, name] of boundaryNames.entries()) {
    writePrivate(
      join(atLimitNamespace.path, name),
      Buffer.from(`${index}\n`, "utf8"),
      0o600,
    );
  }
  const boundary = observeColimaLivePreEffectRootsForTest(
    atLimit.requirements,
    atLimitObservation,
    atLimit.input,
  );
  assert.equal(boundary.root_set_disposition, "foreign-collision");
  assert.equal(
    boundary.root_observations.find(
      (entry) => entry.role === atLimitNamespace.role,
    )?.disposition,
    "foreign-collision",
  );

  const excessive = fixture(t);
  const excessiveObservation = build(excessive);
  const targetNamespace = mutationNamespaces(excessive)[0];
  for (let index = 0; index < 65; index += 1) {
    writePrivate(
      join(targetNamespace.path, `foreign-${String(index).padStart(2, "0")}`),
      Buffer.from(`${index}\n`, "utf8"),
      0o600,
    );
  }
  expectRefusal(
    () =>
      observeColimaLivePreEffectRootsForTest(
        excessive.requirements,
        excessiveObservation,
        excessive.input,
      ),
    69,
  );
});

test("collision-aware revalidation does not weaken the preparation contract", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const target = join(
    mutationNamespaces(state)[1].path,
    state.input.provider_profile,
  );
  mkdirSync(target, { mode: 0o700 });
  assert.equal(
    observeColimaLivePreEffectRootsForTest(
      state.requirements,
      observation,
      state.input,
    ).root_set_disposition,
    "foreign-collision",
  );
  expectRefusal(() =>
    revalidateColimaLiveObservationForTest(
      state.requirements,
      observation,
      state.input,
    ),
  );

  const changedInput = cloneInput(state.input);
  changedInput.binding_key = randomBytes(32);
  expectRefusal(() =>
    observeColimaLivePreEffectRootsForTest(
      state.requirements,
      observation,
      changedInput,
    ),
  );
});

test("fixture checkpoints refuse target and parent transitions", (t) => {
  const cases = [
    {
      name: "absent-to-present",
      prepare() {},
      mutate(state, target) {
        writePrivate(target, Buffer.from("appeared\n", "utf8"), 0o600);
      },
    },
    {
      name: "collision-to-absent",
      prepare(state, target) {
        writePrivate(target, Buffer.from("departing\n", "utf8"), 0o600);
      },
      mutate(state, target) {
        rmSync(target);
      },
    },
    {
      name: "collision-identity-replacement",
      prepare(state, target) {
        writePrivate(target, Buffer.from("first\n", "utf8"), 0o600);
      },
      mutate(state, target) {
        rmSync(target);
        writePrivate(target, Buffer.from("second\n", "utf8"), 0o600);
      },
    },
    {
      name: "parent-replacement",
      prepare() {},
      mutate(state) {
        const parent = state.input.environment.COLIMA_HOME;
        renameSync(parent, `${parent}-displaced`);
        mkdirSync(parent, { mode: 0o700 });
        chmodSync(parent, 0o700);
      },
    },
    {
      name: "parent-symlink-replacement",
      prepare() {},
      mutate(state) {
        const parent = state.input.environment.COLIMA_HOME;
        const displaced = `${parent}-displaced`;
        renameSync(parent, displaced);
        symlinkSync(displaced, parent);
      },
    },
  ];
  for (const value of cases) {
    const state = fixture(t);
    const target = join(
      mutationNamespaces(state)[1].path,
      state.input.provider_profile,
    );
    value.prepare(state, target);
    let checkpoints = 0;
    expectRefusal(() =>
      observeColimaLivePreEffectRootsForTest(
        state.requirements,
        state.observation,
        state.input,
        (checkpoint) => {
          assert.equal(checkpoint, "after-first-root-sample");
          checkpoints += 1;
          value.mutate(state, target);
        },
      ),
    );
    assert.equal(checkpoints, 1, value.name);
  }

  const state = fixture(t);
  expectRefusal(
    () =>
      observeColimaLivePreEffectRootsForTest(
        state.requirements,
        state.observation,
        state.input,
        "not-a-checkpoint",
      ),
    64,
  );
});

test("public projection omits private paths, HOME, fixture and component identities", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const projection = colimaLivePublicProjectionForTest(state.requirements, observation);
  const serialized = JSON.stringify(projection);
  assert.equal(projection.schema, COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA);
  assert.equal(projection.authorizations.execution_authorized, false);
  assert.equal(projection.authorizations.finalization_eligible, false);
  for (const privateValue of [
    state.root,
    state.home,
    state.providerRoot,
    state.input.fixture_id,
    state.input.provider_profile,
    ...Object.values(state.input.component_paths),
  ]) {
    assert.equal(serialized.includes(privateValue), false);
  }
  assert.deepEqual(Object.keys(projection).sort(), [
    "authorizations",
    "host",
    "observation_sha256",
    "provider_class",
    "provider_kind",
    "requirements_sha256",
    "schema",
  ]);
});

test("the preparation observer imports no process execution surface", () => {
  const source = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-colima-live-contract.mjs", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(source, /node:(?:child_process|net)/u);
  assert.equal(/\b(?:execFile|execSync|fork|spawn)(?:Sync)?\s*\(/u.test(source), false);
  assert.doesNotMatch(
    source,
    /\b(?:chmod|link|mkdir|rename|rm|rmdir|symlink|unlink|writeFile)Sync\s*\(/u,
  );
  assert.doesNotMatch(source, /\breaddirSync\s*\(/u);
  assert.match(source, /\bopendirSync\s*\(/u);
  const namespaceStart = source.indexOf("function captureAdmissionNamespace");
  const namespaceEnd = source.indexOf(
    "function captureAdmissionRoots",
    namespaceStart,
  );
  assert.ok(namespaceStart >= 0 && namespaceEnd > namespaceStart);
  const namespaceSource = source.slice(namespaceStart, namespaceEnd);
  assert.equal(
    namespaceSource.match(/boundedDirectoryEntries\(\s*parent\.path,/gu)
      ?.length,
    2,
  );
  assert.match(
    namespaceSource,
    /lstatSync\(join\(parent\.path, name\), \{ bigint: true \}\)/gu,
  );
  assert.doesNotMatch(
    namespaceSource,
    /\b(?:readFileSync|readlinkSync|realpathSync|statSync)\s*\(/u,
  );
});

test("input paths, profile and closed environment refuse ambient drift", (t) => {
  const state = fixture(t);
  const mutations = [
    (input) => {
      input.environment.PATH = "/usr/bin";
    },
    (input) => {
      input.environment.HTTP_PROXY = "http://127.0.0.1:9";
    },
    (input) => {
      delete input.environment.SSH;
    },
    (input) => {
      input.environment = null;
    },
    (input) => {
      input.component_paths["ssh-client"] = "ssh";
    },
    (input) => {
      input.provider_profile = "default";
    },
    (input) => {
      input.receipt_owned_disk_image_path = input.source_disk_image_path;
    },
    (input) => {
      input.binding_key = Buffer.alloc(31);
    },
  ];
  for (const mutate of mutations) {
    const input = cloneInput(state.input);
    mutate(input);
    expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, input));
  }
});

test("HOME must be the receipt-private empty namespace", (t) => {
  const state = fixture(t);
  const relative = cloneInput(state.input);
  relative.environment.HOME = "relative-home";
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, relative), 69);

  const external = cloneInput(state.input);
  external.environment.HOME = state.external;
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, external));

  rmdirSync(state.home);
  symlinkSync(state.external, state.home);
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, state.input));
  rmSync(state.home);
  mkdirSync(state.home, { mode: 0o700 });
  chmodSync(state.home, 0o700);

  const observation = build(state);
  assert.equal(JSON.stringify(observation.environment.home).includes(state.home), false);
  const firstBinding = observation.environment.home.path_hmac_sha256;
  const rebound = cloneInput(state.input);
  rebound.binding_key = randomBytes(32);
  assert.notEqual(buildColimaLiveObservationForTest(state.requirements, rebound).environment.home.path_hmac_sha256, firstBinding);
});

test("component symlinks, hard links, modes, contents and paths refuse", (t) => {
  const state = fixture(t);
  const original = state.input.component_paths["docker-cli-binary"];
  chmodSync(original, 0o700);
  expectRefusal(() => build(state));
  chmodSync(original, 0o500);

  const hardLink = join(state.providerRoot, "b", "docker-link");
  linkSync(original, hardLink);
  expectRefusal(() => build(state));
  rmSync(hardLink);

  const renamed = join(state.providerRoot, "b", "docker-real");
  renameSync(original, renamed);
  symlinkSync(renamed, original);
  expectRefusal(() => build(state));
  rmSync(original);
  renameSync(renamed, original);

  const pinned = state.input.component_paths["colima-binary"];
  chmodSync(pinned, 0o600);
  writeFileSync(pinned, "changed colima fixture\n");
  chmodSync(pinned, 0o500);
  expectRefusal(() => build(state));
});

test("disk source and receipt copy require exact distinct regular private bytes", (t) => {
  const state = fixture(t);
  chmodSync(state.sourceDisk, 0o600);
  expectRefusal(() => build(state));
  chmodSync(state.sourceDisk, 0o400);

  chmodSync(state.copiedDisk, 0o600);
  writeFileSync(state.copiedDisk, "changed disk\n");
  chmodSync(state.copiedDisk, 0o400);
  expectRefusal(() => build(state));
});

test("host version, architecture, build, kernel and boot identity are closed", (t) => {
  const state = fixture(t);
  const mutations = [
    (host) => {
      host.product_version = "12.9.9";
    },
    (host) => {
      host.architecture = "x86_64";
    },
    (host) => {
      host.build_version = "unknown";
    },
    (host) => {
      host.kernel_release = "22.1";
    },
    (host) => {
      host.boot_session_sha256 = "A".repeat(64);
    },
  ];
  for (const mutate of mutations) {
    const input = cloneInput(state.input);
    mutate(input.host);
    expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, input), 69);
  }
});

test("serialized observations refuse rebinding and command, file, directory or environment drift", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const mutations = [
    (value) => {
      value.requirements_sha256 = "f".repeat(64);
    },
    (value) => {
      value.authorizations.execution_authorized = true;
    },
    (value) => {
      value.command[1] = "delete";
    },
    (value) => {
      value.components[0].sha256 = "f".repeat(64);
    },
    (value) => {
      value.components[0].mode = "0700";
    },
    (value) => {
      value.components[0].path = state.input.component_paths["docker-cli-binary"];
    },
    (value) => {
      value.directories[6].path = join(state.root, "other-provider");
    },
    (value) => {
      value.environment.variables.find((entry) => entry.name === "PATH").value = "/usr/bin";
    },
    (value) => {
      value.receipt_owned_disk_image.inode = value.source_disk_image.inode;
      value.receipt_owned_disk_image.device = value.source_disk_image.device;
    },
    (value) => {
      value.host.build_version = "lowercase";
    },
  ];
  for (const mutate of mutations) {
    const changed = clone(observation);
    mutate(changed);
    expectRefusal(() => validateColimaLiveObservationForTest(state.requirements, changed));
  }
});

test("revalidation detects component and HOME physical replacement", (t) => {
  const componentState = fixture(t);
  const componentObservation = build(componentState);
  const componentPath = componentState.input.component_paths["docker-cli-binary"];
  const replacement = join(componentState.root, "replacement");
  writePrivate(replacement, componentState.componentBytes.get("docker-cli-binary"), 0o500);
  rmSync(componentPath);
  renameSync(replacement, componentPath);
  expectRefusal(() =>
    revalidateColimaLiveObservationForTest(
      componentState.requirements,
      componentObservation,
      componentState.input,
    ),
  );

  const homeState = fixture(t);
  const homeObservation = build(homeState);
  rmdirSync(homeState.home);
  mkdirSync(homeState.home, { mode: 0o700 });
  chmodSync(homeState.home, 0o700);
  expectRefusal(
    () =>
      revalidateColimaLiveObservationForTest(
        homeState.requirements,
        homeObservation,
        homeState.input,
      ),
    73,
  );
});

test("complete preparation evidence still cannot authorize execution", (t) => {
  const state = fixture(t);
  const observation = build(state);
  expectRefusal(
    () => authorizeColimaLiveObservationForTest(state.requirements, observation),
    69,
  );
});

test("fixture preparation cannot be stamped as a production operation plan", (t) => {
  const state = fixture(t);
  const observation = build(state);
  assert.notEqual(observation.requirements_sha256, COLIMA_LIVE_REQUIREMENTS_SHA256);
  assert.throws(
    () =>
      buildColimaLiveProviderOperationPlan({
        observation,
        observationInput: state.input,
        stateBinding: {
          candidate_sha256: "b".repeat(64),
          fixture_id: state.input.fixture_id,
          source_head_sha256: "c".repeat(64),
          source_sequence: 0,
        },
        tuple: {
          action: "provider-create",
          operation_contract_sha256:
            COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
          operation_kind: COLIMA_LIVE_CREATE_OPERATION_KIND,
          provider_class: COLIMA_LIVE_PROVIDER_CLASS,
        },
      }),
    (error) => {
      assert.ok(error instanceof LiveProviderPlanFailure);
      assert.match(error.message, /preparation observation was refused/u);
      return true;
    },
  );
});

test("supported lifecycle remains preparation-only", () => {
  const lifecycle = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-acceptance.sh", import.meta.url),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /(?:execute|recover|run|start)\)/u);
});

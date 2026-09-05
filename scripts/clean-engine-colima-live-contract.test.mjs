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
  readlinkSync,
  renameSync,
  rmSync,
  rmdirSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { join, relative } from "node:path";
import { test } from "node:test";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  COLIMA_LIVE_FIXTURE_PRE_EFFECT_ROOT_OBSERVATION_SCHEMA,
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

function preEffectPaths(state) {
  return {
    colima: join(state.input.environment.COLIMA_HOME, state.input.provider_profile),
    lima: join(
      state.input.environment.LIMA_HOME,
      `colima-${state.input.provider_profile}`,
    ),
  };
}

test("production live requirements are exact, pinned and execution-disabled", () => {
  assert.equal(COLIMA_LIVE_REQUIREMENTS.schema, COLIMA_LIVE_REQUIREMENTS_SCHEMA);
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.execution_authorized, false);
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.lifecycle_exposure_authorized, false);
  assert.equal(COLIMA_LIVE_REQUIREMENTS.authorizations.finalization_eligible, false);
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
  ];
  for (const mutate of mutations) {
    const requirements = clone(COLIMA_LIVE_REQUIREMENTS);
    mutate(requirements);
    expectRefusal(() => validateColimaLiveRequirements(requirements));
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
      value.components[7].stage_relative_path = "b/ssh";
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
  assert.equal(observation.schema, COLIMA_LIVE_OBSERVATION_SCHEMA);
  assert.equal(observation.requirements_sha256, colimaLiveDigest(colimaLiveBytes(state.requirements)));
  assert.equal(observation.components.length, 12);
  assert.equal(observation.directories.length, 9);
  assert.equal(validateColimaLiveObservationForTest(state.requirements, observation), observation);
  assert.equal(
    revalidateColimaLiveObservationForTest(state.requirements, observation, state.input),
    observation,
  );
});

test("pre-effect observation reports only the two exact point-in-time absences", (t) => {
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
  assert.equal(result.root_set_disposition, "observed-absent");
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
    [
      ["colima-profile-root", "observed-absent"],
      ["lima-instance-root", "observed-absent"],
    ],
  );
  for (const entry of result.root_observations) {
    assert.deepEqual(Object.keys(entry).sort(), [
      "disposition",
      "parent_identity_hmac_sha256",
      "role",
      "target_entry_identity_hmac_sha256",
      "target_path_hmac_sha256",
    ]);
    assert.match(entry.parent_identity_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.match(entry.target_path_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.equal(entry.target_entry_identity_hmac_sha256, "0".repeat(64));
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

test("each existing target kind is an opaque foreign collision", (t) => {
  const cases = ["file", "hardlink", "directory", "symlink", "dangling-symlink"];
  for (const [index, kind] of cases.entries()) {
    const state = fixture(t);
    const observation = build(state);
    const paths = preEffectPaths(state);
    const target = index % 2 === 0 ? paths.colima : paths.lima;
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
    assert.match(collision.target_entry_identity_hmac_sha256, /^[0-9a-f]{64}$/u);
    assert.notEqual(collision.target_entry_identity_hmac_sha256, "0".repeat(64));
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

test("both exact collisions are bounded while unrelated siblings remain drift", (t) => {
  const both = fixture(t);
  const bothObservation = build(both);
  const bothPaths = preEffectPaths(both);
  mkdirSync(bothPaths.colima, { mode: 0o700 });
  writePrivate(bothPaths.lima, Buffer.from("foreign\n", "utf8"), 0o600);
  const collision = observeColimaLivePreEffectRootsForTest(
    both.requirements,
    bothObservation,
    both.input,
  );
  assert.deepEqual(
    collision.root_observations.map((entry) => entry.disposition),
    ["foreign-collision", "foreign-collision"],
  );

  for (const parent of ["COLIMA_HOME", "LIMA_HOME"]) {
    const state = fixture(t);
    const observation = build(state);
    writePrivate(
      join(state.input.environment[parent], "unrelated-entry"),
      Buffer.from("unrelated\n", "utf8"),
      0o600,
    );
    expectRefusal(() =>
      observeColimaLivePreEffectRootsForTest(
        state.requirements,
        observation,
        state.input,
      ),
    );
  }
});

test("collision-aware revalidation does not weaken the preparation contract", (t) => {
  const state = fixture(t);
  const observation = build(state);
  const target = preEffectPaths(state).colima;
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
    const target = preEffectPaths(state).colima;
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
  const targetStart = source.indexOf("function captureAdmissionTarget");
  const targetEnd = source.indexOf("function captureAdmissionRoots", targetStart);
  assert.ok(targetStart >= 0 && targetEnd > targetStart);
  const targetSource = source.slice(targetStart, targetEnd);
  assert.match(targetSource, /optionalNoFollowMetadata\(\s*targetPath/gu);
  assert.match(targetSource, /readdirSync\(parent\.path\)/u);
  assert.doesNotMatch(
    targetSource,
    /\b(?:openSync|readFileSync|readdirSync|readlinkSync|realpathSync|statSync)\(\s*targetPath/u,
  );
  const noFollowStart = source.indexOf("function optionalNoFollowMetadata");
  const noFollowEnd = source.indexOf("function recordedDirectoryMatches", noFollowStart);
  const noFollowSource = source.slice(noFollowStart, noFollowEnd);
  assert.match(noFollowSource, /lstatSync\(path, \{ bigint: true \}\)/u);
  assert.doesNotMatch(
    noFollowSource,
    /\b(?:openSync|readFileSync|readdirSync|readlinkSync|realpathSync|statSync)\s*\(/u,
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

test("HOME must be absolute, non-symlinked, disjoint and privately bound", (t) => {
  const state = fixture(t);
  const relative = cloneInput(state.input);
  relative.environment.HOME = "relative-home";
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, relative), 69);

  const overlap = cloneInput(state.input);
  overlap.environment.HOME = state.providerRoot;
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, overlap));

  const linkedHome = join(state.root, "linked-home");
  symlinkSync(state.home, linkedHome);
  const linked = cloneInput(state.input);
  linked.environment.HOME = linkedHome;
  expectRefusal(() => buildColimaLiveObservationForTest(state.requirements, linked));

  const observation = build(state);
  assert.equal(JSON.stringify(observation).includes(state.home), false);
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

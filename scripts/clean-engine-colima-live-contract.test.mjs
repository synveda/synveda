import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  linkSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  rmdirSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  COLIMA_LIVE_PUBLIC_PROJECTION_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS,
  COLIMA_LIVE_REQUIREMENTS_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS_SHA256,
  ColimaLiveContractFailure,
  authorizeColimaLiveObservationForTest,
  buildColimaLiveObservationForTest,
  colimaLiveBytes,
  colimaLiveDigest,
  colimaLiveLimaNetworkConfigBytesForTest,
  colimaLivePublicProjectionForTest,
  revalidateColimaLiveObservationForTest,
  validateColimaLiveObservationForTest,
  validateColimaLiveRequirements,
} from "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";

const COMPONENT_LAYOUT = Object.freeze({
  "colima-binary": ["b/colima", 0o500],
  "docker-cli-binary": ["b/docker", 0o500],
  "lima-guestagent": ["a/lima-guestagent.Linux-aarch64.gz", 0o400],
  "lima-network-config": ["l/_config/networks.yaml", 0o400],
  "lima-wrapper": ["b/lima", 0o500],
  "limactl-binary": ["b/limactl", 0o500],
  "ssh-client": ["b/ssh", 0o500],
  "ssh-keygen": ["b/ssh-keygen", 0o500],
  "state-owner-node": ["x/node", 0o500],
  "state-owner-script": ["x/state.mjs", 0o400],
  "sw-vers": ["b/sw_vers", 0o500],
  "system-profiler": ["b/system_profiler", 0o500],
});

function digest(algorithm, bytes) {
  return createHash(algorithm).update(bytes).digest("hex");
}

function clone(value) {
  return structuredClone(value);
}

function cloneInput(input) {
  return { ...clone(input), binding_key: Buffer.from(input.binding_key) };
}

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof ColimaLiveContractFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

function writePrivate(path, bytes, mode) {
  writeFileSync(path, bytes, { flag: "wx", mode });
  chmodSync(path, mode);
}

function fixture(t) {
  const temporaryRoot = realpathSync(process.platform === "darwin" ? "/private/tmp" : "/tmp");
  const root = realpathSync(mkdtempSync(join(temporaryRoot, "s-colima-live-")));
  chmodSync(root, 0o700);
  t.after(() => rmSync(root, { force: true, recursive: true }));

  const providerRoot = join(root, "p");
  const home = join(root, "h");
  const external = join(root, "x");
  for (const path of [
    providerRoot,
    home,
    external,
    ...["a", "b", "c", "d", "k", "l", "l/_config", "t"].map((name) =>
      join(providerRoot, name),
    ),
  ]) {
    mkdirSync(path, { mode: 0o700 });
    chmodSync(path, 0o700);
  }

  const componentPaths = {};
  const componentBytes = new Map();
  for (const [role, [relativePath, mode]] of Object.entries(COMPONENT_LAYOUT)) {
    const path = relativePath.startsWith("x/")
      ? join(root, relativePath)
      : join(providerRoot, relativePath);
    const bytes =
      role === "lima-network-config"
        ? colimaLiveLimaNetworkConfigBytesForTest()
        : Buffer.from(`synveda deterministic ${role}\n`, "utf8");
    writePrivate(path, bytes, mode);
    componentPaths[role] = path;
    componentBytes.set(role, bytes);
  }

  const diskBytes = Buffer.from("synveda deterministic colima disk image\n", "utf8");
  const sourceDisk = join(root, "source.raw.gz");
  const copiedDisk = join(providerRoot, "a", "colima-disk-image.raw.gz");
  writePrivate(sourceDisk, diskBytes, 0o400);
  writePrivate(copiedDisk, diskBytes, 0o400);

  const requirements = clone(COLIMA_LIVE_REQUIREMENTS);
  for (const component of requirements.components) {
    if (component.expected_sha256 === "0".repeat(64)) continue;
    const bytes = componentBytes.get(component.role);
    component.expected_sha256 = digest("sha256", bytes);
    component.expected_size = String(bytes.length);
  }
  const colima = requirements.components.find((entry) => entry.role === "colima-binary");
  requirements.release_artifacts.colima.sha256 = colima.expected_sha256;
  requirements.release_artifacts.colima.size = colima.expected_size;
  requirements.release_artifacts.disk_image.sha256 = digest("sha256", diskBytes);
  requirements.release_artifacts.disk_image.sha512 = digest("sha512", diskBytes);
  requirements.release_artifacts.disk_image.size = String(diskBytes.length);

  const fixtureId = randomBytes(16).toString("hex");
  const input = {
    binding_key: randomBytes(32),
    component_paths: componentPaths,
    environment: {
      COLIMA_CACHE_HOME: join(providerRoot, "k"),
      COLIMA_DOWNLOADER: "native",
      COLIMA_HOME: join(providerRoot, "c"),
      DOCKER_CONFIG: join(providerRoot, "d"),
      HOME: home,
      LANG: "C",
      LC_ALL: "C",
      LIMA_HOME: join(providerRoot, "l"),
      PATH: join(providerRoot, "b"),
      SSH: join(providerRoot, "b", "ssh"),
      TMPDIR: join(providerRoot, "t"),
      XPC_SERVICE_NAME: "0",
    },
    fixture_id: fixtureId,
    host: {
      architecture: "arm64",
      boot_session_sha256: "a".repeat(64),
      build_version: "22A400",
      kernel_release: "22.1.0",
      platform: "darwin",
      product_version: "13.0.0",
    },
    provider_profile: `synveda-cpr45-${fixtureId}`,
    provider_root: providerRoot,
    receipt_owned_disk_image_path: copiedDisk,
    source_disk_image_path: sourceDisk,
  };
  return { componentBytes, copiedDisk, diskBytes, home, input, providerRoot, requirements, root, sourceDisk };
}

function build(state) {
  return buildColimaLiveObservationForTest(state.requirements, state.input);
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
  assert.equal(source.includes("node:child_process"), false);
  assert.equal(/\b(?:execFile|execSync|fork|spawn)(?:Sync)?\s*\(/u.test(source), false);
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

test("supported lifecycle remains preparation-only", () => {
  const lifecycle = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-acceptance.sh", import.meta.url),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /(?:execute|recover|run|start)\)/u);
});

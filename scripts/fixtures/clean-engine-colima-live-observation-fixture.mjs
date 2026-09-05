import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import {
  COLIMA_LIVE_REQUIREMENTS,
  buildColimaLiveObservationForTest,
  colimaLiveLimaNetworkConfigBytesForTest,
} from "../../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";

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

export function cloneColimaLiveObservationInput(input) {
  return { ...clone(input), binding_key: Buffer.from(input.binding_key) };
}

export function writePrivateColimaLiveFixtureFile(path, bytes, mode) {
  writeFileSync(path, bytes, { flag: "wx", mode });
  chmodSync(path, mode);
}

export function createCleanEngineColimaLiveObservationFixture({
  fixtureId = randomBytes(16).toString("hex"),
} = {}) {
  if (!/^[0-9a-f]{32}$/u.test(fixtureId)) {
    throw new Error("Colima live observation fixture identity was refused");
  }
  const temporaryRoot = realpathSync(
    process.platform === "darwin" ? "/private/tmp" : "/tmp",
  );
  const root = realpathSync(mkdtempSync(join(temporaryRoot, "s-colima-live-")));
  chmodSync(root, 0o700);

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
    writePrivateColimaLiveFixtureFile(path, bytes, mode);
    componentPaths[role] = path;
    componentBytes.set(role, bytes);
  }

  const diskBytes = Buffer.from(
    "synveda deterministic colima disk image\n",
    "utf8",
  );
  const sourceDisk = join(root, "source.raw.gz");
  const copiedDisk = join(providerRoot, "a", "colima-disk-image.raw.gz");
  writePrivateColimaLiveFixtureFile(sourceDisk, diskBytes, 0o400);
  writePrivateColimaLiveFixtureFile(copiedDisk, diskBytes, 0o400);

  const requirements = clone(COLIMA_LIVE_REQUIREMENTS);
  for (const component of requirements.components) {
    if (component.expected_sha256 === "0".repeat(64)) continue;
    const bytes = componentBytes.get(component.role);
    component.expected_sha256 = digest("sha256", bytes);
    component.expected_size = String(bytes.length);
  }
  const colima = requirements.components.find(
    (entry) => entry.role === "colima-binary",
  );
  requirements.release_artifacts.colima.sha256 = colima.expected_sha256;
  requirements.release_artifacts.colima.size = colima.expected_size;
  requirements.release_artifacts.disk_image.sha256 = digest("sha256", diskBytes);
  requirements.release_artifacts.disk_image.sha512 = digest("sha512", diskBytes);
  requirements.release_artifacts.disk_image.size = String(diskBytes.length);

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
  const observation = buildColimaLiveObservationForTest(requirements, input);
  return {
    componentBytes,
    copiedDisk,
    diskBytes,
    external,
    home,
    input,
    observation,
    providerRoot,
    requirements,
    root,
    sourceDisk,
  };
}

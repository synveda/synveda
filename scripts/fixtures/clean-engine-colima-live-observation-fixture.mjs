import { createHash, randomBytes } from "node:crypto";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, sep } from "node:path";
import {
  COLIMA_LIVE_REQUIREMENTS,
  buildColimaLiveObservationForTest,
  colimaLiveLimaNetworkConfigBytesForTest,
} from "../../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";

const COMPONENT_LAYOUT = Object.freeze({
  "colima-binary": ["b/colima", 0o500],
  "docker-cli-binary": ["b/docker", 0o500],
  "lima-default-template": ["share/lima/templates/default.yaml", 0o400],
  "lima-guestagent": [
    "share/lima/lima-guestagent.Linux-aarch64.gz",
    0o400,
  ],
  "lima-network-config": ["l/_config/networks.yaml", 0o600],
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

function allocateShortFixtureRoot(maximumProviderRootBytes) {
  const temporaryRoot = realpathSync("/tmp");
  const fixedBytes =
    Buffer.byteLength(temporaryRoot, "utf8") +
    Buffer.byteLength(sep, "utf8") +
    6 +
    Buffer.byteLength(`${sep}p`, "utf8");
  const paddingBytes = maximumProviderRootBytes - fixedBytes;
  if (paddingBytes < 0) {
    throw new Error("Colima live fixture root budget was unavailable");
  }
  const prefix = `${temporaryRoot}${sep}${"s".repeat(paddingBytes)}`;
  const root = realpathSync(mkdtempSync(prefix));
  chmodSync(root, 0o700);
  const providerRoot = join(root, "p");
  if (
    dirname(root) !== temporaryRoot ||
    Buffer.byteLength(providerRoot, "utf8") !== maximumProviderRootBytes
  ) {
    rmSync(root, { force: true, recursive: true });
    throw new Error("Colima live fixture root shape was refused");
  }
  return { providerRoot, root, temporaryRoot };
}

export function createCleanEngineColimaLiveObservationFixture({
  fixtureId = randomBytes(16).toString("hex"),
} = {}) {
  if (!/^[0-9a-f]{32}$/u.test(fixtureId)) {
    throw new Error("Colima live observation fixture identity was refused");
  }
  const maximumProviderRootBytes =
    COLIMA_LIVE_REQUIREMENTS.mutation_surface.provider_root_path
      .maximum_canonical_bytes;
  const { providerRoot, root, temporaryRoot } =
    allocateShortFixtureRoot(maximumProviderRootBytes);
  const home = join(providerRoot, "h");
  const external = join(root, "x");
  for (const path of [
    providerRoot,
    external,
    ...[
      "a",
      "b",
      "c",
      "d",
      "h",
      "k",
      "l",
      "l/_config",
      "share",
      "share/lima",
      "share/lima/templates",
      "t",
    ].map((name) => join(providerRoot, name)),
  ]) {
    mkdirSync(path, { mode: 0o700 });
    chmodSync(path, 0o700);
  }
  const rootMetadata = lstatSync(root, { bigint: true });
  const providerRootMetadata = lstatSync(providerRoot, { bigint: true });
  if (
    realpathSync(providerRoot) !== providerRoot ||
    rootMetadata.uid !== BigInt(process.getuid()) ||
    providerRootMetadata.uid !== BigInt(process.getuid()) ||
    (rootMetadata.mode & 0o7777n) !== 0o700n ||
    (providerRootMetadata.mode & 0o7777n) !== 0o700n
  ) {
    rmSync(root, { force: true, recursive: true });
    throw new Error("Colima live fixture root identity was refused");
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

#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

function fail(message, status = 78) {
  process.stderr.write(`compose-assets: ${message}\n`);
  process.exit(status);
}

function parseArgs(argv) {
  const result = {};
  for (let index = 2; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || value === "") fail("invalid arguments", 64);
    result[name.slice(2)] = value;
  }
  const allowed = new Set(["config-file", "project", "docker-bin", "state"]);
  if (
    ![3, 4].includes(Object.keys(result).length) ||
    Object.keys(result).some((name) => !allowed.has(name)) ||
    result["config-file"] === undefined ||
    result.project === undefined ||
    result["docker-bin"] === undefined
  ) {
    fail("expected --config-file, --project and --docker-bin", 64);
  }
  result.state ??= "existing";
  if (!new Set(["existing", "stopped"]).has(result.state)) fail("state was refused", 64);
  return result;
}

function docker(binary, args, allowMissing = false) {
  const result = spawnSync(binary, args, {
    encoding: "utf8",
    maxBuffer: 2 * 1024 * 1024,
    timeout: 30_000,
    killSignal: "SIGKILL",
  });
  if (result.error?.code === "ETIMEDOUT") fail("Docker inventory timed out", 69);
  if (result.error !== undefined) fail("Docker inventory could not start", 69);
  if (result.status !== 0) {
    if (allowMissing && result.status === 1) return "";
    fail("Docker inventory was unavailable", 69);
  }
  if (Buffer.byteLength(result.stdout) > 2 * 1024 * 1024) fail("Docker inventory exceeded its bound", 69);
  return result.stdout;
}

function lines(output, grammar, label) {
  if (output === "") return [];
  const values = output.trimEnd().split("\n");
  if (values.some((value) => !grammar.test(value)) || new Set(values).size !== values.length) {
    fail(`${label} inventory was malformed`);
  }
  return values;
}

function object(value, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") fail(`${label} was malformed`);
  return value;
}

function inventoryContainsIdentity(inventory, identity) {
  const matches = [...inventory].filter((candidate) => identity.startsWith(candidate));
  return matches.length === 1;
}

function exactLabel(labels, name, value, label) {
  if (object(labels, `${label} labels`)[name] !== value) fail(`${label} label ${name} was refused`);
}

function emptyObjectOrNull(value) {
  return value === undefined || value === null ||
    (typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0);
}

const args = parseArgs(process.argv);
if (!/^synveda-(development|reference)(-acceptance-[a-z0-9][a-z0-9-]{0,23})?$/.test(args.project)) {
  fail("project name was refused", 64);
}
let rawConfig;
try {
  rawConfig = readFileSync(args["config-file"]);
} catch {
  fail("rendered configuration could not be read", 69);
}
if (rawConfig.length < 2 || rawConfig.length > 2 * 1024 * 1024) fail("rendered configuration size was refused");
let config;
try {
  config = JSON.parse(rawConfig.toString("utf8"));
} catch {
  fail("rendered configuration was not JSON");
}
object(config, "rendered configuration");
if (config.name !== args.project) fail("rendered project identity was refused");
const services = object(config.services, "rendered services");
const networks = object(config.networks, "rendered networks");
const volumes = object(config.volumes ?? {}, "rendered volumes");
const expectedContainers = new Map();
for (const [service, definition] of Object.entries(services)) {
  if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(service)) fail("rendered service name was refused");
  exactLabel(object(definition, `service ${service}`).labels, "com.synveda.contract", "cpr-45", `service ${service}`);
  expectedContainers.set(`${args.project}-${service}-1`, service);
}
const expectedNetworks = new Map();
for (const [logical, definitionValue] of Object.entries(networks)) {
  if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(logical)) fail("rendered network name was refused");
  const definition = object(definitionValue, `network ${logical}`);
  if (typeof definition.name !== "string" || definition.name !== `${args.project}_${logical}`) {
    fail(`network ${logical} physical name was refused`);
  }
  exactLabel(definition.labels, "com.synveda.contract", "cpr-45", `network ${logical}`);
  exactLabel(definition.labels, "com.synveda.network", logical, `network ${logical}`);
  if (
    definition.driver !== undefined && definition.driver !== "bridge" ||
    !emptyObjectOrNull(definition.driver_opts) ||
    definition.attachable === true || definition.external === true ||
    definition.enable_ipv6 === true
  ) fail(`network ${logical} rendered runtime contract was refused`);
  const ipam = object(definition.ipam, `network ${logical} IPAM`);
  if (
    ipam.driver !== undefined && ipam.driver !== "default" ||
    !emptyObjectOrNull(ipam.options) ||
    !Array.isArray(ipam.config) || ipam.config.length !== 1
  ) fail(`network ${logical} rendered IPAM contract was refused`);
  const ipamConfig = object(ipam.config[0], `network ${logical} IPAM configuration`);
  if (
    typeof ipamConfig.subnet !== "string" || ipamConfig.subnet === "" ||
    typeof ipamConfig.gateway !== "string" || ipamConfig.gateway === "" ||
    (ipamConfig.ip_range !== undefined &&
      (typeof ipamConfig.ip_range !== "string" || ipamConfig.ip_range === ""))
  ) fail(`network ${logical} rendered IPAM configuration was refused`);
  expectedNetworks.set(definition.name, {
    logical,
    internal: definition.internal === true,
    subnet: ipamConfig.subnet,
    gateway: ipamConfig.gateway,
    ipRange: ipamConfig.ip_range,
  });
}
const expectedVolumes = new Map();
for (const [logical, definitionValue] of Object.entries(volumes)) {
  if (!/^[a-z0-9][a-z0-9-]{0,62}$/.test(logical)) fail("rendered volume name was refused");
  const definition = object(definitionValue, `volume ${logical}`);
  if (typeof definition.name !== "string" || definition.name !== `${args.project}_${logical}`) {
    fail(`volume ${logical} physical name was refused`);
  }
  exactLabel(definition.labels, "com.synveda.contract", "cpr-45", `volume ${logical}`);
  exactLabel(definition.labels, "com.synveda.volume", logical, `volume ${logical}`);
  expectedVolumes.set(definition.name, logical);
}

const containerIds = new Set(
  lines(
    docker(args["docker-bin"], [
      "container", "ls", "--all", "--quiet", "--filter",
      `label=com.docker.compose.project=${args.project}`,
    ]),
    /^[0-9a-f]{12,64}$/,
    "project container",
  ),
);
for (const name of expectedContainers.keys()) {
  for (const id of lines(
    docker(args["docker-bin"], [
      "container", "ls", "--all", "--quiet", "--filter", `name=^/${name}$`,
    ]),
    /^[0-9a-f]{12,64}$/,
    "named container",
  )) containerIds.add(id);
}
if (containerIds.size > Object.keys(services).length) fail("project container inventory exceeded the service contract");
if (args.state === "stopped" && containerIds.size !== 0) fail("project containers remain after shutdown");
const seenServices = new Set();
if (containerIds.size > 0) {
  let inspected;
  try {
    inspected = JSON.parse(docker(args["docker-bin"], ["container", "inspect", ...containerIds]));
  } catch {
    fail("project container inspection was malformed");
  }
  if (!Array.isArray(inspected) || inspected.length !== containerIds.size) fail("project container inspection was incomplete");
  for (const container of inspected) {
    object(container, "project container");
    if (!/^[0-9a-f]{64}$/.test(container.Id) || !inventoryContainsIdentity(containerIds, container.Id)) {
      fail("project container identity was refused");
    }
    const labels = object(container.Config, "project container configuration").Labels;
    exactLabel(labels, "com.docker.compose.project", args.project, "project container");
    exactLabel(labels, "com.synveda.contract", "cpr-45", "project container");
    exactLabel(labels, "com.docker.compose.oneoff", "False", "project container");
    exactLabel(labels, "com.docker.compose.container-number", "1", "project container");
    const service = object(labels, "project container labels")["com.docker.compose.service"];
    if (typeof service !== "string" || expectedContainers.get(container.Name?.slice(1)) !== service) {
      fail("project container name or service was refused");
    }
    if (seenServices.has(service)) fail("duplicate project service container was refused");
    seenServices.add(service);
  }
}

const networkIds = new Set(
  lines(
    docker(args["docker-bin"], [
      "network", "ls", "--quiet", "--filter",
      `label=com.docker.compose.project=${args.project}`,
    ]),
    /^[0-9a-f]{12,64}$/,
    "project network",
  ),
);
for (const name of expectedNetworks.keys()) {
  for (const id of lines(
    docker(args["docker-bin"], ["network", "ls", "--quiet", "--filter", `name=^${name}$`]),
    /^[0-9a-f]{12,64}$/,
    "named network",
  )) networkIds.add(id);
}
if (networkIds.size > Object.keys(networks).length) fail("project network inventory exceeded the network contract");
if (args.state === "stopped" && networkIds.size !== 0) fail("project networks remain after shutdown");
const seenNetworks = new Set();
if (networkIds.size > 0) {
  let inspected;
  try {
    inspected = JSON.parse(docker(args["docker-bin"], ["network", "inspect", ...networkIds]));
  } catch {
    fail("project network inspection was malformed");
  }
  if (!Array.isArray(inspected) || inspected.length !== networkIds.size) fail("project network inspection was incomplete");
  for (const network of inspected) {
    object(network, "project network");
    if (!/^[0-9a-f]{64}$/.test(network.Id) || !inventoryContainsIdentity(networkIds, network.Id)) {
      fail("project network identity was refused");
    }
    const expected = expectedNetworks.get(network.Name);
    if (expected === undefined || seenNetworks.has(network.Name)) fail("project network name was refused");
    seenNetworks.add(network.Name);
    if (
      network.Driver !== "bridge" || network.Scope !== "local" ||
      network.Attachable !== false || network.Ingress !== false ||
      network.Internal !== expected.internal || network.EnableIPv6 !== false ||
      (network.EnableIPv4 !== undefined && network.EnableIPv4 !== true) ||
      network.ConfigOnly !== false || !emptyObjectOrNull(network.Options) ||
      !(network.ConfigFrom === undefined || network.ConfigFrom?.Network === "")
    ) fail("project network runtime contract was refused");
    const runtimeIpam = object(network.IPAM, "project network IPAM");
    const runtimeIpamConfig = Array.isArray(runtimeIpam.Config) &&
        runtimeIpam.Config.length === 1
      ? object(runtimeIpam.Config[0], "project network IPAM configuration")
      : undefined;
    if (
      runtimeIpam.Driver !== "default" || !emptyObjectOrNull(runtimeIpam.Options) ||
      runtimeIpamConfig === undefined ||
      runtimeIpamConfig.Subnet !== expected.subnet ||
      runtimeIpamConfig.Gateway !== expected.gateway ||
      (expected.ipRange === undefined
        ? !(runtimeIpamConfig.IPRange === undefined || runtimeIpamConfig.IPRange === "")
        : runtimeIpamConfig.IPRange !== expected.ipRange)
    ) fail("project network runtime IPAM contract was refused");
    exactLabel(network.Labels, "com.docker.compose.project", args.project, "project network");
    exactLabel(network.Labels, "com.docker.compose.network", expected.logical, "project network");
    exactLabel(network.Labels, "com.synveda.contract", "cpr-45", "project network");
    exactLabel(network.Labels, "com.synveda.network", expected.logical, "project network");
  }
}

const volumeNames = new Set(
  lines(
    docker(args["docker-bin"], [
      "volume", "ls", "--quiet", "--filter",
      `label=com.docker.compose.project=${args.project}`,
    ]),
    /^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,254}$/,
    "project volume",
  ),
);
for (const name of expectedVolumes.keys()) {
  for (const candidate of lines(
    docker(args["docker-bin"], ["volume", "ls", "--quiet", "--filter", `name=^${name}$`]),
    /^[a-zA-Z0-9][a-zA-Z0-9_.-]{0,254}$/,
    "named volume",
  )) volumeNames.add(candidate);
}
if (volumeNames.size > Object.keys(volumes).length) fail("project volume inventory exceeded the volume contract");
const seenVolumes = new Set();
if (volumeNames.size > 0) {
  let inspected;
  try {
    inspected = JSON.parse(docker(args["docker-bin"], ["volume", "inspect", ...volumeNames]));
  } catch {
    fail("project volume inspection was malformed");
  }
  if (!Array.isArray(inspected) || inspected.length !== volumeNames.size) fail("project volume inspection was incomplete");
  for (const volume of inspected) {
    object(volume, "project volume");
    const logical = expectedVolumes.get(volume.Name);
    if (logical === undefined || seenVolumes.has(volume.Name)) fail("project volume name was refused");
    seenVolumes.add(volume.Name);
    if (
      volume.Driver !== "local" || volume.Scope !== "local" ||
      !(
        volume.Options === null ||
        (typeof volume.Options === "object" && !Array.isArray(volume.Options) && Object.keys(volume.Options).length === 0)
      )
    ) fail("project volume runtime contract was refused");
    exactLabel(volume.Labels, "com.docker.compose.project", args.project, "project volume");
    exactLabel(volume.Labels, "com.docker.compose.volume", logical, "project volume");
    exactLabel(volume.Labels, "com.synveda.contract", "cpr-45", "project volume");
    exactLabel(volume.Labels, "com.synveda.volume", logical, "project volume");
  }
}

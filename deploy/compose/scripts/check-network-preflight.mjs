#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import process from "node:process";
import { fileURLToPath } from "node:url";

const COMMAND_TIMEOUT_MS = 10_000;

function ipv4Integer(value) {
  const parts = value.split(".");
  if (parts.length !== 4) return undefined;
  let result = 0;
  for (const part of parts) {
    if (!/^(?:0|[1-9]\d{0,2})$/.test(part)) return undefined;
    const octet = Number(part);
    if (octet > 255) return undefined;
    result = result * 256 + octet;
  }
  return result;
}

export function ipv4Interval(cidr) {
  const match = /^([^/]+)\/(\d|[12]\d|3[0-2])$/.exec(cidr);
  if (match === null) return undefined;
  const address = ipv4Integer(match[1]);
  if (address === undefined) return undefined;
  const prefix = Number(match[2]);
  const size = 2 ** (32 - prefix);
  const start = Math.floor(address / size) * size;
  if (start !== address) return undefined;
  return { start, end: start + size - 1 };
}

function overlaps(left, right) {
  return left.start <= right.end && right.start <= left.end;
}

function expectedNetworks(project, pool) {
  const network = pool.slice(0, -".0/24".length);
  const slots = [
    ["identity-backend", 0, 1, true, `${network}.8/29`],
    ["public-edge", 16, 17, false],
    ["app-backend", 32, 33, true],
    ["synveda-data", 48, 49, true],
    ["keycloak-data", 64, 65, true],
    ["keycloak-management", 80, 81, true],
    ["telemetry", 96, 97, true],
    ["application-egress", 112, 113, false],
    ["identity-egress", 128, 129, false],
    ["telemetry-egress", 144, 145, false],
  ];
  return new Map(
    slots.map(([logical, offset, gateway, internal, ipRange]) => [
      `${project}_${logical}`,
      {
        logical,
        subnet: `${network}.${offset}/28`,
        gateway: `${network}.${gateway}`,
        internal,
        ...(ipRange ? { ipRange } : {}),
      },
    ]),
  );
}

function ipamConfigs(network) {
  return network?.IPAM?.Config ?? [];
}

function emptyObjectOrNull(value) {
  return value === null ||
    (typeof value === "object" && !Array.isArray(value) && Object.keys(value).length === 0);
}

export function networkPreflightFindings(networks, project, pool) {
  const findings = [];
  const selected = ipv4Interval(pool);
  if (selected === undefined || !pool.endsWith(".0/24")) return ["selected pool was refused"];
  const expected = expectedNetworks(project, pool);
  const seenIds = new Set();
  const seenNames = new Set();

  for (const network of networks) {
    if (typeof network?.Id !== "string" || !network.Id || seenIds.has(network.Id)) {
      findings.push("network inventory was malformed");
      continue;
    }
    seenIds.add(network.Id);
    if (typeof network?.Name !== "string" || !network.Name || seenNames.has(network.Name)) {
      findings.push("network inventory was malformed");
      continue;
    }
    seenNames.add(network.Name);
    const labels = network.Labels ?? {};
    const belongsToProject = labels["com.docker.compose.project"] === project;
    const expectedNetwork = expected.get(network.Name);
    const configs = ipamConfigs(network);
    const intervals = [];
    for (const config of configs) {
      if (typeof config?.Subnet !== "string") continue;
      const interval = ipv4Interval(config.Subnet);
      if (interval === undefined) {
        if (config.Subnet.includes(".")) findings.push("network inventory was malformed");
        continue;
      }
      intervals.push({ interval, config });
    }
    const selectedOverlap = intervals.some(({ interval }) => overlaps(interval, selected));

    if (belongsToProject && expectedNetwork === undefined) {
      findings.push("stale project network was refused");
      continue;
    }
    if (expectedNetwork !== undefined && !belongsToProject) {
      findings.push("project network contract drifted");
      continue;
    }
    if (!selectedOverlap && !belongsToProject) continue;
    if (!belongsToProject || expectedNetwork === undefined) {
      findings.push("selected pool overlaps a foreign network");
      continue;
    }
    const exactLabels =
      labels["com.docker.compose.network"] === expectedNetwork.logical &&
      labels["com.synveda.contract"] === "cpr-45" &&
      labels["com.synveda.network"] === expectedNetwork.logical;
    const config = configs.length === 1 ? configs[0] : undefined;
    const exactIpam =
      network?.IPAM?.Driver === "default" &&
      emptyObjectOrNull(network?.IPAM?.Options) &&
      config?.Subnet === expectedNetwork.subnet &&
      config?.Gateway === expectedNetwork.gateway &&
      (expectedNetwork.ipRange === undefined
        ? config?.IPRange === undefined || config?.IPRange === ""
        : config?.IPRange === expectedNetwork.ipRange);
    const exactIsolation =
      network.Driver === "bridge" &&
      network.Scope === "local" &&
      network.Internal === expectedNetwork.internal &&
      network.Attachable === false &&
      network.Ingress === false &&
      network.EnableIPv6 === false &&
      (network.EnableIPv4 === undefined || network.EnableIPv4 === true) &&
      network.ConfigOnly === false &&
      emptyObjectOrNull(network.Options) &&
      (network.ConfigFrom === undefined || network.ConfigFrom?.Network === "");
    if (!exactLabels || !exactIpam || !exactIsolation) {
      findings.push("project network contract drifted");
    }
  }
  return [...new Set(findings)];
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined || values.has(key)) return undefined;
    values.set(key, value);
  }
  const allowed = new Set(["--project", "--pool", "--docker-bin"]);
  if ([...values.keys()].some((key) => !allowed.has(key))) return undefined;
  const project = values.get("--project");
  const pool = values.get("--pool");
  const dockerBin = values.get("--docker-bin") ?? "docker";
  if (!/^synveda-(?:development|reference)(?:-acceptance-[a-z0-9][a-z0-9-]{0,23})?$/.test(project ?? "")) {
    return undefined;
  }
  if (ipv4Interval(pool ?? "") === undefined || !pool.endsWith(".0/24")) return undefined;
  return { project, pool, dockerBin };
}

function run(binary, args, maxBuffer) {
  const result = spawnSync(binary, args, {
    encoding: "utf8",
    env: process.env,
    timeout: COMMAND_TIMEOUT_MS,
    maxBuffer,
  });
  return result.status === 0 && result.error === undefined ? result.stdout : undefined;
}

export function main(argv = process.argv.slice(2)) {
  const selection = parseArguments(argv);
  if (selection === undefined) {
    console.error("compose-network: configuration was refused");
    process.exitCode = 64;
    return;
  }
  const listed = run(selection.dockerBin, ["network", "ls", "--format", "{{.ID}}"], 1024 * 1024);
  if (listed === undefined) {
    console.error("compose-network: Docker network inventory was unavailable");
    process.exitCode = 69;
    return;
  }
  const ids = listed.split("\n").filter(Boolean);
  let networks = [];
  if (ids.length > 0) {
    const inspected = run(selection.dockerBin, ["network", "inspect", ...ids], 8 * 1024 * 1024);
    if (inspected === undefined) {
      console.error("compose-network: Docker network inventory was unavailable");
      process.exitCode = 69;
      return;
    }
    try {
      networks = JSON.parse(inspected);
    } catch {
      console.error("compose-network: Docker network inventory was malformed");
      process.exitCode = 78;
      return;
    }
  }
  if (!Array.isArray(networks)) {
    console.error("compose-network: Docker network inventory was malformed");
    process.exitCode = 78;
    return;
  }
  const findings = networkPreflightFindings(networks, selection.project, selection.pool);
  if (findings.length > 0) {
    console.error(`compose-network: ${findings[0]}`);
    process.exitCode = 78;
    return;
  }
  console.log("Docker network interval preflight validated");
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) main();

#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { lookup } from "node:dns/promises";
import { realpathSync } from "node:fs";
import process from "node:process";
import { fileURLToPath } from "node:url";

const LOOKUP_TIMEOUT_MS = 5_000;

function refuse(message, status) {
  console.error(`compose-resolver: ${message}`);
  process.exitCode = status;
}

export function normalizeAddresses(addresses) {
  return [...new Map(addresses.map(({ address, family }) => [`${family}:${address}`, { address, family }])).values()]
    .sort((left, right) => `${left.family}:${left.address}`.localeCompare(`${right.family}:${right.address}`));
}

export function developmentResolutionIsExact(addresses) {
  const normalized = normalizeAddresses(addresses);
  return (
    normalized.length === 1 &&
    normalized[0].family === 4 &&
    normalized[0].address === "127.0.0.1"
  );
}

export function parseVersion(value) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/.exec(value.trim());
  if (match === null) return undefined;
  return match.slice(1).map(Number);
}

export function versionAtLeast(actual, minimum) {
  for (let index = 0; index < minimum.length; index += 1) {
    if (actual[index] > minimum[index]) return true;
    if (actual[index] < minimum[index]) return false;
  }
  return true;
}

export function localDockerEndpoint(endpoint) {
  return /^unix:\/\/\/(?:[^\s]+)$/.test(endpoint.trim());
}

function dockerOutput(binary, args, environment = process.env) {
  const result = spawnSync(binary, args, {
    encoding: "utf8",
    env: environment,
    timeout: LOOKUP_TIMEOUT_MS,
    maxBuffer: 16 * 1024,
  });
  if (result.status !== 0 || result.error !== undefined) return undefined;
  return result.stdout.trim();
}

export function effectiveDockerEndpoint(binary, environment = process.env) {
  if (environment.DOCKER_CONTEXT) {
    return dockerOutput(binary, [
      "context",
      "inspect",
      environment.DOCKER_CONTEXT,
      "--format",
      "{{.Endpoints.docker.Host}}",
    ], environment);
  }
  if (environment.DOCKER_HOST) return environment.DOCKER_HOST;
  const context = dockerOutput(binary, ["context", "show"], environment);
  if (!context) return undefined;
  return dockerOutput(binary, [
    "context",
    "inspect",
    context,
    "--format",
    "{{.Endpoints.docker.Host}}",
  ], environment);
}

async function boundedLookup(host) {
  let timer;
  try {
    return await Promise.race([
      lookup(host, { all: true, order: "verbatim" }),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error("lookup-timeout")), LOOKUP_TIMEOUT_MS);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!name?.startsWith("--") || value === undefined || values.has(name)) return undefined;
    values.set(name, value);
  }
  const runtime = values.get("--runtime");
  const oidc = values.get("--oidc");
  const appHost = values.get("--app-host");
  const authHost = values.get("--auth-host");
  const dockerBin = values.get("--docker-bin") ?? "docker";
  const dockerOnly = values.get("--docker-only") ?? "false";
  const printDockerEndpoint = values.get("--print-docker-endpoint") ?? "false";
  const allowed = new Set([
    "--runtime",
    "--oidc",
    "--app-host",
    "--auth-host",
    "--docker-bin",
    "--docker-only",
    "--print-docker-endpoint",
  ]);
  if ([...values.keys()].some((key) => !allowed.has(key))) return undefined;
  if (dockerOnly === "true") {
    if (!new Set(["true", "false"]).has(printDockerEndpoint)) return undefined;
    if ([runtime, oidc, appHost, authHost].some((value) => value !== undefined)) return undefined;
    return { dockerBin, dockerOnly: true, printDockerEndpoint: printDockerEndpoint === "true" };
  }
  if (dockerOnly !== "false" || printDockerEndpoint !== "false") return undefined;
  if (!new Set(["development", "reference"]).has(runtime)) return undefined;
  if (!new Set(["bundled", "external"]).has(oidc) || !appHost) return undefined;
  if ((oidc === "bundled") !== Boolean(authHost)) return undefined;
  return { runtime, oidc, appHost, authHost, dockerBin, dockerOnly: false };
}

export async function main(argv = process.argv.slice(2)) {
  const selection = parseArguments(argv);
  if (selection === undefined) {
    refuse("configuration was refused", 64);
    return;
  }
  if (Number(process.versions.node.split(".")[0]) < 22 || !["darwin", "linux"].includes(process.platform)) {
    refuse("Node 22 or newer on macOS or Linux is required", 69);
    return;
  }
  const endpoint = effectiveDockerEndpoint(selection.dockerBin);
  if (!endpoint || !localDockerEndpoint(endpoint)) {
    refuse("local Docker endpoint is required", 69);
    return;
  }
  const pinnedDockerEnvironment = {
    ...process.env,
    DOCKER_HOST: endpoint,
  };
  delete pinnedDockerEnvironment.DOCKER_CONTEXT;
  const serverVersion = dockerOutput(selection.dockerBin, [
    "version",
    "--format",
    "{{.Server.Version}}",
  ], pinnedDockerEnvironment);
  const parsedVersion = serverVersion && parseVersion(serverVersion);
  if (!parsedVersion || !versionAtLeast(parsedVersion, [28, 0, 0])) {
    refuse("Docker Engine 28.0.0 or newer is required", 69);
    return;
  }

  if (selection.dockerOnly) {
    console.log(selection.printDockerEndpoint ? endpoint : "local Docker prerequisite validated");
    return;
  }

  const roles = [["application-host", selection.appHost]];
  if (selection.oidc === "bundled") roles.push(["identity-host", selection.authHost]);
  for (const [role, host] of roles) {
    let answers;
    try {
      answers = await boundedLookup(host);
    } catch (error) {
      const status = error?.message === "lookup-timeout" ? 75 : 78;
      refuse(`${role} ${status === 75 ? "lookup timed out" : "mapping was refused"}`, status);
      return;
    }
    if (selection.runtime === "development" && !developmentResolutionIsExact(answers)) {
      refuse(`${role} mapping was refused`, 78);
      return;
    }
    if (selection.runtime === "reference" && normalizeAddresses(answers).length === 0) {
      refuse(`${role} mapping was refused`, 78);
      return;
    }
  }
  console.log("host resolver and local Docker prerequisites validated");
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  await main();
}

#!/usr/bin/env node
import { realpathSync } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import process from "node:process";
import { fileURLToPath } from "node:url";

const MAX_STATUS_BYTES = 1024 * 1024;
const REQUEST_TIMEOUT_MS = 5_000;
const HOST_TRUST_ENVIRONMENT = Object.freeze([
  "NODE_OPTIONS",
  "NODE_EXTRA_CA_CERTS",
  "NODE_TLS_REJECT_UNAUTHORIZED",
  "NODE_USE_SYSTEM_CA",
  "NODE_USE_ENV_PROXY",
  "SSL_CERT_FILE",
  "SSL_CERT_DIR",
  "OPENSSL_CONF",
  "OPENSSL_CONF_INCLUDE",
  "OPENSSL_MODULES",
  "OPENSSL_ENGINES",
]);

export function parseComposePs(source) {
  const trimmed = source.trim();
  if (!trimmed) return [];
  try {
    const parsed = JSON.parse(trimmed);
    return Array.isArray(parsed) ? parsed : [parsed];
  } catch {
    return trimmed.split("\n").map((line) => JSON.parse(line));
  }
}

export function runtimeStateFindings(rows, { postgres, oidc }) {
  const oneShots = new Set([
    "database-preflight",
    "issuer-diagnostic",
    "migrate",
    "tenant-convergence",
  ]);
  const longRunning = new Set(["gateway", "otel-collector", "proxy", "worker"]);
  if (postgres === "bundled") {
    oneShots.add("database-bootstrap");
    longRunning.add("postgres");
  }
  if (oidc === "bundled") {
    oneShots.add("keycloak-database-bootstrap");
    longRunning.add("keycloak");
    longRunning.add("keycloak-realm-convergence");
  }
  const expected = new Set([...oneShots, ...longRunning]);
  const findings = [];
  const observed = new Map();
  for (const row of rows) {
    const service = row?.Service;
    if (typeof service !== "string" || !service || observed.has(service)) {
      findings.push("Compose service status inventory was malformed");
      continue;
    }
    observed.set(service, row);
  }
  if (
    JSON.stringify([...observed.keys()].sort()) !== JSON.stringify([...expected].sort())
  ) {
    findings.push("Compose service status set differs from the selected topology");
  }
  for (const service of oneShots) {
    const row = observed.get(service);
    if (row?.State !== "exited" || Number(row?.ExitCode) !== 0) {
      findings.push(`${service} convergence did not complete successfully`);
    }
  }
  for (const service of longRunning) {
    const row = observed.get(service);
    if (row?.State !== "running" || row?.Health !== "healthy") {
      findings.push(`${service} is not running and healthy`);
    }
  }
  return [...new Set(findings)];
}

export function referenceHostTrustFindings({ runtime, environment, execArgv }) {
  if (runtime !== "reference") return [];
  if (
    environment === null ||
    typeof environment !== "object" ||
    !Array.isArray(execArgv)
  ) {
    return ["reference host trust process was malformed"];
  }
  if (HOST_TRUST_ENVIRONMENT.some((name) => Object.hasOwn(environment, name))) {
    return ["reference host trust environment was refused"];
  }
  const normalized = execArgv.map((argument) =>
    typeof argument === "string" ? argument.replaceAll("_", "-") : "",
  );
  if (
    !normalized.includes("--use-bundled-ca") ||
    normalized.some((argument) =>
      new Set(["--use-openssl-ca", "--use-system-ca", "--use-env-proxy"]).has(
        argument,
      ),
    )
  ) {
    return ["reference host trust mode was refused"];
  }
  return [];
}

export function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith("--") || value === undefined || values.has(key)) return undefined;
    values.set(key, value);
  }
  const allowed = new Set([
    "--status-file",
    "--runtime",
    "--postgres",
    "--oidc",
    "--app-url",
    "--issuer",
  ]);
  if ([...values.keys()].some((key) => !allowed.has(key))) return undefined;
  const selection = Object.fromEntries([...values].map(([key, value]) => [key.slice(2), value]));
  if (!selection["status-file"]?.startsWith("/")) return undefined;
  if (!["development", "reference"].includes(selection.runtime)) return undefined;
  if (!["bundled", "external"].includes(selection.postgres)) return undefined;
  if (!["bundled", "external"].includes(selection.oidc)) return undefined;
  try {
    const app = new URL(selection["app-url"]);
    const issuer = new URL(selection.issuer);
    if (!new Set(["http:", "https:"]).has(app.protocol) || !new Set(["http:", "https:"]).has(issuer.protocol)) {
      return undefined;
    }
    if (selection.runtime === "reference" &&
        (app.protocol !== "https:" || issuer.protocol !== "https:")) {
      return undefined;
    }
    if (selection.runtime === "development" && app.protocol !== "http:") {
      return undefined;
    }
  } catch {
    return undefined;
  }
  return selection;
}

async function probe(url, expectedStatus, stage) {
  let response;
  try {
    response = await fetch(url, {
      redirect: "manual",
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      headers: { "user-agent": "synveda-compose-smoke/1" },
    });
  } catch {
    throw new Error(stage);
  }
  if (response.status !== expectedStatus) throw new Error(stage);
  return response;
}

export async function boundedResponseBody(response, maximumBytes) {
  const declaredLength = response.headers.get("content-length");
  if (declaredLength !== null &&
      (!/^(?:0|[1-9]\d*)$/.test(declaredLength) || Number(declaredLength) > maximumBytes)) {
    throw new Error("bounded response was refused");
  }
  if (response.body === null) return Buffer.alloc(0);
  const reader = response.body.getReader();
  const chunks = [];
  let bytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      bytes += value.byteLength;
      if (bytes > maximumBytes) {
        await reader.cancel();
        throw new Error("bounded response was refused");
      }
      chunks.push(Buffer.from(value));
    }
  } finally {
    reader.releaseLock();
  }
  return Buffer.concat(chunks, bytes);
}

export async function main(argv = process.argv.slice(2)) {
  const selection = parseArguments(argv);
  if (selection === undefined) {
    console.error("compose-smoke: configuration was refused");
    process.exitCode = 64;
    return;
  }
  const hostTrustFindings = referenceHostTrustFindings({
    runtime: selection.runtime,
    environment: process.env,
    execArgv: process.execArgv,
  });
  if (hostTrustFindings.length > 0) {
    console.error(`compose-smoke: ${hostTrustFindings[0]}`);
    process.exitCode = 78;
    return;
  }
  let metadata;
  try {
    metadata = await lstat(selection["status-file"]);
  } catch {
    console.error("compose-smoke: service status inventory was unavailable");
    process.exitCode = 69;
    return;
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > MAX_STATUS_BYTES) {
    console.error("compose-smoke: service status inventory was refused");
    process.exitCode = 78;
    return;
  }
  let rows;
  try {
    rows = parseComposePs(await readFile(selection["status-file"], "utf8"));
  } catch {
    console.error("compose-smoke: service status inventory was malformed");
    process.exitCode = 78;
    return;
  }
  const findings = runtimeStateFindings(rows, selection);
  if (findings.length > 0) {
    console.error(`compose-smoke: ${findings[0]}`);
    process.exitCode = 78;
    return;
  }

  const appUrl = selection["app-url"].replace(/\/$/, "");
  try {
    await probe(`${appUrl}/healthz`, 200, "application liveness probe failed");
    await probe(`${appUrl}/readyz`, 200, "application readiness probe failed");
    await probe(`${appUrl}/console/`, 200, "console probe failed");
    await probe(`${appUrl}/metrics`, 404, "public metrics refusal probe failed");
    if (selection.oidc === "bundled") {
      const discovery = await probe(
        `${selection.issuer}/.well-known/openid-configuration`,
        200,
        "OIDC discovery probe failed",
      );
      let document;
      try {
        const body = await boundedResponseBody(discovery, MAX_STATUS_BYTES);
        document = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
      } catch {
        throw new Error("OIDC discovery contract failed");
      }
      if (document?.issuer !== selection.issuer) throw new Error("OIDC discovery contract failed");
      const identityOrigin = new URL(selection.issuer).origin;
      await probe(`${identityOrigin}/health/ready`, 404, "identity management refusal probe failed");
      await probe(`${identityOrigin}/metrics`, 404, "identity metrics refusal probe failed");
      await probe(`${identityOrigin}/admin/`, 404, "identity administration refusal probe failed");
      await probe(
        `${identityOrigin}/realms/master/.well-known/openid-configuration`,
        404,
        "identity master realm refusal probe failed",
      );
    }
  } catch (error) {
    const stage = error instanceof Error ? error.message : "public endpoint probe failed";
    console.error(`compose-smoke: ${stage}`);
    process.exitCode = 78;
    return;
  }
  console.log("service state and public endpoint smoke validated");
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) await main();

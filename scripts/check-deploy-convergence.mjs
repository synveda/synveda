#!/usr/bin/env node
// CPR-36 / ADR-0095: one application runtime across host, Compose and Helm.
// This check is intentionally database- and daemon-free. It renders the two
// Compose shapes and Helm, inspects the generated public contract, and builds
// the release profile twice so an upgrade-shaped stale file cannot survive.

import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));

export const RETIRED_RUNTIME_MARKERS = [
  "/v1/observe",
  "/v1/inject",
  "/v1/recall",
  "record_embeddings",
  "hierarchy_nodes",
  "role_bindings",
  "policy_lapses",
  "demo/seed.sh",
  "organisation.txt",
];

export function serviceBlock(source, service) {
  const start = source.indexOf(`\n  ${service}:\n`);
  if (start < 0) return "";
  const bodyStart = start + `\n  ${service}:\n`.length;
  const rest = source.slice(bodyStart);
  const next = rest.search(/^  [a-zA-Z0-9_-]+:\s*$/m);
  return next < 0 ? rest : rest.slice(0, next);
}

export function retiredFindings(source) {
  const active = source
    .split("\n")
    .filter((line) => !/^\s*#/.test(line))
    .join("\n");
  return RETIRED_RUNTIME_MARKERS.filter((marker) => active.includes(marker));
}

export function hasRetiredDemoField(source) {
  return /^\s*demo:\s*bool,/m.test(source);
}

function fail(message) {
  throw new Error(`deployment convergence: ${message}`);
}

function read(relative) {
  return readFileSync(join(ROOT, relative), "utf8");
}

function run(command, args, options = {}) {
  try {
    return execFileSync(command, args, {
      cwd: ROOT,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      ...options,
    });
  } catch (error) {
    const stderr = error?.stderr?.toString().trim();
    fail(`${command} ${args.join(" ")} failed${stderr ? `: ${stderr}` : ""}`);
  }
}

function checkCompose(relative, release) {
  const source = read(relative);
  const gateway = serviceBlock(`\n${source}`, "gateway");
  if (!gateway) fail(`${relative} has no gateway service`);
  if (!gateway.includes("synveda_gateway")) {
    fail(`${relative} does not connect the gateway as synveda_gateway`);
  }
  if (/postgres:\/\/synveda:/.test(gateway)) {
    fail(`${relative} hands the database-owner DSN to the gateway`);
  }
  if (!gateway.includes("SYNVEDA_GATEWAY_DATABASE_URL")) {
    fail(`${relative} cannot receive a separately provisioned runtime DSN`);
  }
  if (release && /^\s*build:/m.test(source)) {
    fail(`${relative} is a release manifest with a source build`);
  }
  const retired = retiredFindings(source);
  if (retired.length) fail(`${relative} retains ${retired.join(", ")}`);

  // Compose parsing and interpolation are different failure modes. Rendering
  // with interpolation disabled checks the manifest without exposing the
  // restricted .env values that `synveda init` may have written beside it.
  run("docker", ["compose", "-f", relative, "config", "--no-interpolate"]);
}

function checkHelm() {
  const rendered = run("helm", [
    "template",
    "synveda",
    "deploy/helm/synveda",
    "-f",
    "deploy/helm/synveda/ci/full-values.yaml",
  ]);
  if (!rendered.includes("replicas: 1")) fail("Helm no longer pins one gateway replica");
  if (!rendered.includes("synveda-pg-app")) fail("Helm does not use CloudNativePG's app Secret");
  if (!rendered.includes("synveda_app")) fail("Helm does not grant the runtime capability role");
  if (!rendered.includes("/readyz")) fail("Helm lost the schema-epoch readiness check");
  for (const secret of ["synveda-dev", "Synveda-Demo-Passw0rd", "API-Key synveda-dev"]) {
    if (rendered.includes(secret)) fail(`Helm rendered a plaintext credential marker: ${secret}`);
  }
  const retired = retiredFindings(rendered);
  if (retired.length) fail(`rendered Helm retains ${retired.join(", ")}`);
}

function checkPublicContract() {
  const openapi = JSON.parse(read("docs/api/openapi.json"));
  for (const path of [
    "/v1/sessions",
    "/v1/sessions/{session_id}/events",
    "/v1/knowledge",
    "/v1/capture-candidates",
    "/v1/context-runs/{id}",
    "/v1/configurations/effective",
  ]) {
    if (!openapi.paths[path]) fail(`generated OpenAPI is missing ${path}`);
  }
  for (const path of ["/v1/observe", "/v1/inject", "/v1/recall"]) {
    if (openapi.paths[path]) fail(`generated OpenAPI resurrected ${path}`);
  }

  const cli = `${read("crates/synveda-cli/src/main.rs")}\n${read("crates/synveda-cli/src/init.rs")}`;
  if (hasRetiredDemoField(cli)) {
    fail("the removed init demo switch remains in the CLI model");
  }
}

function checkReleaseUpgradeShape() {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-deploy-check-"));
  try {
    const version = "0.2.0";
    run("bash", ["scripts/package-release.sh", version, scratch]);
    const stage = join(scratch, `synveda-profile-${version}`);
    const stale = join(stage, "retired-demo-sentinel");
    writeFileSync(stale, "must be removed by replacement\n");
    run("bash", ["scripts/package-release.sh", version, scratch]);
    if (existsSync(stale)) fail("a repeated release package retained a stale profile file");

    const archive = join(scratch, `synveda-profile-${version}.tar.gz`);
    const entries = run("tar", ["-tzf", archive]).split("\n").filter(Boolean);
    const expected = new Set([
      `synveda-profile-${version}/`,
      `synveda-profile-${version}/docker-compose.yml`,
      `synveda-profile-${version}/rauthy/`,
      `synveda-profile-${version}/rauthy/config.toml`,
      `synveda-profile-${version}/version`,
    ]);
    for (const entry of entries) {
      if (!expected.has(entry)) fail(`release profile contains unexpected entry ${entry}`);
    }
    if (entries.some((entry) => entry.includes("/demo/"))) {
      fail("release profile still packages the retired demo seeder");
    }
    const packaged = readFileSync(join(stage, "docker-compose.yml"), "utf8");
    if (!serviceBlock(`\n${packaged}`, "gateway").includes("synveda_gateway")) {
      fail("packaged release drifted from the least-privilege gateway DSN");
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

export function main() {
  checkCompose("deploy/compose/docker-compose.yml", false);
  checkCompose("deploy/release/docker-compose.yml", true);
  checkHelm();
  checkPublicContract();
  checkReleaseUpgradeShape();
  console.log(
    "deployment convergence holds: 2 Compose renders, Helm render, current OpenAPI, " +
      "least-privilege runtime DSNs and repeatable release replacement",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}

#!/usr/bin/env node
// CPR-36 / ADR-0095: one application runtime across host, Compose and Helm.
// This check is intentionally database- and daemon-free. It renders the two
// Compose shapes and Helm, inspects the generated public contract, and builds
// the release profile twice so an upgrade-shaped stale file cannot survive.

import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
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

export function releaseNoteFindings(source) {
  const block = source.match(/cat > notes\.md <<NOTES\r?\n([\s\S]*?)^[ \t]*NOTES[ \t]*$/mu);
  if (!block) return ["release-note block is missing"];
  const notes = block[1];
  const findings = [];
  if (notes.includes("synveda init --demo")) {
    findings.push("retired synveda init --demo command");
  }
  const commands = [
    "synveda init --slug pulseboard --name PulseBoard --embedder tei",
    "synveda login",
    "synveda demo start --profile personal",
  ];
  let previous = -1;
  for (const command of commands) {
    const index = notes.indexOf(command);
    if (index < 0) findings.push(`${command} is missing`);
    else if (index <= previous) findings.push(`${command} is out of order`);
    previous = Math.max(previous, index);
  }
  return findings;
}

export function localDockerCopySources(source) {
  const sources = [];
  for (const line of source.split("\n")) {
    const instruction = line.trim();
    if (!instruction.startsWith("COPY ") || instruction.startsWith("COPY --from=")) continue;
    const fields = instruction.slice("COPY ".length).trim().split(/\s+/);
    if (fields.length < 2 || fields.some((field) => field.includes("$"))) continue;
    sources.push(...fields.slice(0, -1).filter((field) => !field.startsWith("--")));
  }
  return sources;
}

export function missingLocalDockerCopySources(source, pathExists) {
  return localDockerCopySources(source).filter((path) => !pathExists(path));
}

export function missingWorkspaceManifestCopies(source, manifests) {
  const copied = new Set(localDockerCopySources(source));
  return manifests.filter((manifest) => !copied.has(manifest));
}

export function suppressesCargoBuildFailure(source) {
  return /cargo build[^\n]*\|\|\s*true/.test(source);
}

export function productImageFindings(source) {
  const findings = [];
  const stages = [...source.matchAll(/^FROM\s+.*$/gim)];
  const finalStage = stages.length > 0 ? source.slice(stages.at(-1).index) : "";
  const finalActive = finalStage.replace(/^[ \t]*#.*(?:\r?\n|$)/gm, "");
  if (!/^FROM\s+\S+\s+AS\s+runtime\s*$/im.test(finalActive)) {
    findings.push("final stage is not the named runtime stage");
  }
  const users = [...finalActive.matchAll(/^\s*USER\s+([^\s#]+)/gim)].map(
    (match) => match[1],
  );
  const runtimeUser = users.at(-1);
  if (!runtimeUser || !/^[1-9][0-9]*:[1-9][0-9]*$/.test(runtimeUser)) {
    findings.push("final runtime user is not an explicit non-zero UID:GID");
  }
  for (const [binary, instruction] of [
    [
      "synveda-gateway",
      "COPY --from=build /src/target/release/synveda-gateway /usr/local/bin/synveda-gateway",
    ],
    ["synveda", "COPY --from=build /src/target/release/synveda /usr/local/bin/synveda"],
    [
      "synveda-container",
      "COPY --chmod=0755 deploy/compose/gateway/synveda-container /usr/local/bin/synveda-container",
    ],
  ]) {
    if (!finalActive.includes(instruction)) {
      findings.push(`final runtime stage omits ${binary}`);
    }
  }
  if (!finalActive.includes('ENTRYPOINT ["/usr/local/bin/synveda-container"]')) {
    findings.push("role-neutral entrypoint is missing");
  }
  if (!finalActive.includes('CMD ["gateway"]')) {
    findings.push("default gateway role is missing");
  }
  if (/^\s*HEALTHCHECK\b/m.test(finalActive)) {
    findings.push("image hard-codes a role-specific healthcheck");
  }
  if (!/^\s*STOPSIGNAL\s+SIGTERM\s*$/m.test(finalActive)) {
    findings.push("SIGTERM stop signal is missing");
  }
  return findings;
}

export function dockerignoreFindings(source) {
  const rules = new Set(
    source
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#")),
  );
  const findings = [];
  for (const required of [
    ".git",
    ".git/**",
    ".agents",
    ".agents/**",
    ".codex",
    ".codex/**",
    "target",
    "target/**",
    "node_modules",
    "node_modules/**",
    ".env",
    ".env.*",
    "**/.env",
    "**/.env.*",
    "deploy/compose/secrets",
    "deploy/compose/secrets/**",
    "deploy/compose/backups",
    "deploy/compose/backups/**",
  ]) {
    if (!rules.has(required)) findings.push(`build context includes ${required}`);
  }
  const allowedNegations = new Set(["!**/.env.example"]);
  for (const rule of rules) {
    if (rule.startsWith("!") && !allowedNegations.has(rule)) {
      findings.push(`unreviewed build-context re-inclusion ${rule}`);
    }
  }
  return findings;
}

export function productLauncherFindings(source) {
  const findings = [];
  const active = source.replace(/^[ \t]*#.*(?:\r?\n|$)/gm, "");
  const caseLabels = active
    .split("\n")
    .map((line) => line.trimStart())
    .filter((line) => !/^[a-zA-Z_][a-zA-Z0-9_]*\(\)[ \t]*\{/.test(line))
    .map((line) => line.match(/^([^)]*)\)/)?.[1]?.trim())
    .filter((label) => label !== undefined);
  const expectedCaseLabels = ["gateway", "migrate", "probe", "live", "ready", "*", "*"];
  if (JSON.stringify(caseLabels) !== JSON.stringify(expectedCaseLabels)) {
    findings.push("launcher case vocabulary is not closed and ordered");
  }
  const roleMatches = [...active.matchAll(/^ {4}(gateway|migrate|probe|\*)\)[ \t]*$/gm)];
  const labels = roleMatches.map(
    (match) => match[1],
  );
  if (JSON.stringify(labels) !== JSON.stringify(["gateway", "migrate", "probe", "*"])) {
    findings.push("launcher role vocabulary is not closed and ordered");
  }

  const roleBlock = (role) => {
    const position = roleMatches.findIndex((match) => match[1] === role);
    if (position < 0) return "";
    const current = roleMatches[position];
    const next = roleMatches[position + 1];
    const start = current.index + current[0].length;
    return active.slice(start, next?.index);
  };
  const gateway = roleBlock("gateway");
  const migrate = roleBlock("migrate");
  const probe = roleBlock("probe").replace(/\\\r?\n\s*/g, " ");
  const unknown = roleBlock("*").replace(/\s+/g, " ").trim();

  if (!gateway.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("gateway role does not enforce exact arity");
  }
  if (!gateway.includes("exec /usr/local/bin/synveda-gateway")) {
    findings.push("gateway role does not exec the gateway binary");
  }
  if (!migrate.includes('[ "$#" -eq 1 ] || usage')) {
    findings.push("migrate role does not enforce exact arity");
  }
  if (!migrate.includes("exec /usr/local/bin/synveda db migrate")) {
    findings.push("migrate role does not exec the migration command");
  }
  if (!probe.includes('[ "$#" -eq 3 ] || usage')) {
    findings.push("probe role does not enforce exact arity");
  }
  if (!probe.includes('[ "$2" = "gateway" ] || usage')) {
    findings.push("probe role accepts an undeclared target");
  }
  if (!/live\)\s+path=healthz\s+;;/.test(probe)) {
    findings.push("live probe does not select /healthz");
  }
  if (!/ready\)\s+path=readyz\s+;;/.test(probe)) {
    findings.push("ready probe does not select /readyz");
  }
  if (!probe.includes('"http://127.0.0.1:8120/${path}"')) {
    findings.push("probe does not use the fixed loopback gateway endpoint");
  }
  if (!/exec \/usr\/bin\/curl\s+--disable(?:\s|$)/.test(probe)) {
    findings.push("probe permits curl configuration loading");
  }
  if (!/--noproxy\s+['"]\*['"]/.test(probe)) {
    findings.push("probe permits inherited proxy routing");
  }
  for (const option of ["--connect-timeout 1", "--max-time 2", "--fail"]) {
    if (!probe.includes(option)) findings.push(`probe is missing ${option}`);
  }
  if (unknown !== "usage ;; esac") {
    findings.push("unknown role does not fail through usage");
  }
  if (!/^#!\/bin\/sh\s*$/m.test(source) || !/^set -eu\s*$/m.test(active)) {
    findings.push("launcher shell or fail-closed options are missing");
  }
  if (/\beval\b/.test(active) || /\$\(|`/.test(active)) {
    findings.push("launcher evaluates input");
  }
  if (/\b(?:compose|saas|enterprise)\b/.test(active)) {
    findings.push("launcher branches on deployment shape");
  }
  return findings;
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
  if (
    !gateway.includes(
      '"/usr/local/bin/synveda-container", "probe", "gateway", "live"',
    )
  ) {
    fail(`${relative} does not attach the gateway role-specific liveness probe`);
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

function checkReleaseNotes() {
  const findings = releaseNoteFindings(read(".github/workflows/release.yml"));
  if (findings.length > 0) {
    fail(`release notes contain ${findings.join(", ")}`);
  }
}

function checkProductImageInputs() {
  const relative = "deploy/compose/gateway/Dockerfile";
  const source = read(relative);
  const missing = missingLocalDockerCopySources(source, (path) =>
    existsSync(join(ROOT, path)),
  );
  if (missing.length > 0) {
    fail(`${relative} copies missing build inputs: ${missing.join(", ")}`);
  }
  const manifests = readdirSync(join(ROOT, "crates"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => `crates/${entry.name}/Cargo.toml`)
    .filter((path) => existsSync(join(ROOT, path)))
    .sort();
  const omitted = missingWorkspaceManifestCopies(source, manifests);
  if (omitted.length > 0) {
    fail(`${relative} omits workspace manifests from its cache stage: ${omitted.join(", ")}`);
  }
  if (!localDockerCopySources(source).includes("adapters/registry.json")) {
    fail(`${relative} omits the CLI's embedded adapters/registry.json`);
  }
  if (suppressesCargoBuildFailure(source)) {
    fail(`${relative} suppresses a dependency-cache cargo build failure`);
  }
  const imageFindings = productImageFindings(source);
  if (imageFindings.length > 0) {
    fail(`${relative} violates the product image contract: ${imageFindings.join(", ")}`);
  }

  const launcherRelative = "deploy/compose/gateway/synveda-container";
  const launcher = read(launcherRelative);
  const launcherFindings = productLauncherFindings(launcher);
  if (launcherFindings.length > 0) {
    fail(`${launcherRelative} violates the launcher contract: ${launcherFindings.join(", ")}`);
  }
  run("sh", ["-n", launcherRelative]);

  const ignoreFindings = dockerignoreFindings(read(".dockerignore"));
  if (ignoreFindings.length > 0) {
    fail(`.dockerignore violates the build-context contract: ${ignoreFindings.join(", ")}`);
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
  checkProductImageInputs();
  checkPublicContract();
  checkReleaseNotes();
  checkReleaseUpgradeShape();
  console.log(
    "deployment convergence holds: 2 Compose renders, Helm render, product image inputs, " +
      "current OpenAPI, least-privilege runtime DSNs and repeatable release replacement",
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}

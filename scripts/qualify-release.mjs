#!/usr/bin/env node
// CPR-45: qualify the extracted bytes; no builds, registry login or source CLI.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, readdirSync, renameSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { evaluationFailureChecks } from "./evaluation-failure-checks.mjs";
import { qualifyConsumer } from "./qualify-consumer.mjs";

if (process.argv[2] === "--consumer-candidate") {
  qualifyConsumer(...process.argv.slice(3));
  process.exit(0);
}

const [input, output] = process.argv.slice(2);
assert.ok(input && output, "usage: qualify-release.mjs <extracted-bundle> <report.json>");
const bundle = resolve(input);
const manifest = JSON.parse(readFileSync(join(bundle, "environment.json"), "utf8"));
const home = mkdtempSync(join(tmpdir(), "synveda-release-qualification-"));
const env = { ...process.env, SYNVEDA_HOME: home };
delete env.SYNVEDA_COMPOSE_RUNTIME;
const report = { version: manifest.release_version, source: manifest.source_sha, sourceDirty: manifest.source_dirty ?? false, images: manifest.images, at: new Date().toISOString(), checks: {}, imageDownloadMs: null };
function run(command, args, extra = {}) {
  const result = spawnSync(command, args, { env, cwd: bundle, encoding: "utf8", timeout: 900_000, maxBuffer: 4 * 1024 * 1024, ...extra });
  if (result.status !== 0) throw new Error(`${command} ${args[0]} failed (${result.status ?? "timeout"}); ${result.stderr?.slice(-1800) ?? ""}`);
  return result.stdout;
}
const launcher = (...args) => {
  console.log(`qualification: ${args[0]}`);
  return run("sh", [join(bundle, "synveda-compose"), ...args]);
};
const compose = () => ["compose", "--project-name", "synveda-evaluation", "--env-file", join(home, "state/evaluation.env"), ...readFileSync(join(home, "state/evaluation-files"), "utf8").trim().split("\n").flatMap((file) => ["-f", join(bundle, "deploy/compose", file)])];
const browser = () => run("docker", [...compose(), "-f", join(bundle, "deploy/compose/compose.browser-acceptance.yaml"), "-f", join(bundle, "deploy/compose/compose.evaluation-sample.yaml"), "run", "--rm", "--no-deps", "--entrypoint", "node", "browser-acceptance", "evaluation.mjs"]);
function secretHashes() {
  const root = join(home, "state/synveda-evaluation/secrets");
  return readdirSync(root, { withFileTypes: true }).filter((file) => file.isFile()).map((file) => [file.name, createHash("sha256").update(readFileSync(join(root, file.name))).digest("hex")]);
}
function reportFailedRealmConvergence() {
  // Read diagnostics before the exact-project cleanup removes the container.
  // The supervisor's ordinary logs are bounded and private file values are
  // redacted before anything reaches the Actions log.
  try {
    const ids = spawnSync("docker", ["ps", "-aq", "--filter", "label=com.docker.compose.project=synveda-evaluation",
      "--filter", "label=com.docker.compose.service=keycloak-realm-convergence"],
    { env, encoding: "utf8", timeout: 15_000 });
    const id = ids.status === 0 ? ids.stdout.trim().split("\n")[0] : "";
    if (!id) return;
    const inspected = spawnSync("docker", ["inspect", "--format", "{{json .State}}", id],
      { env, encoding: "utf8", timeout: 15_000 });
    const state = inspected.status === 0 ? JSON.parse(inspected.stdout) : {};
    const logs = spawnSync("docker", ["logs", "--tail", "50", id],
      { env, encoding: "utf8", timeout: 15_000, maxBuffer: 1024 * 1024 });
    let health = JSON.stringify({ status: state.Status, exitCode: state.ExitCode,
      health: state.Health?.Status, recentHealth: state.Health?.Log?.slice(-3).map((entry) => entry.Output?.slice(-400)) });
    let recentLogs = (logs.stdout ?? "") + (logs.stderr ?? "");
    const secrets = join(home, "state/synveda-evaluation/secrets");
    for (const file of readdirSync(secrets, { withFileTypes: true }).filter((entry) => entry.isFile())) {
      const value = readFileSync(join(secrets, file.name), "utf8").trim();
      if (value.length >= 8) {
        health = health.replaceAll(value, "[REDACTED]");
        recentLogs = recentLogs.replaceAll(value, "[REDACTED]");
      }
    }
    console.error(`qualification realm-convergence state: ${health.slice(-800)}`);
    console.error(`qualification realm-convergence recent logs: ${recentLogs.slice(-2200)}`);
  } catch {
    console.error("qualification realm-convergence diagnostics unavailable");
  }
}
// Never reuse an operator's deployment or quietly clean up its data.
assert.equal(run("docker", ["ps", "-aq", "--filter", "label=com.docker.compose.project=synveda-evaluation"]).trim(), "", "evaluation project already exists");
assert.equal(run("docker", ["volume", "ls", "-q", "--filter", "name=^synveda-evaluation_"]).trim(), "", "evaluation volumes already exist");
try {
  const preparing = Date.now(); launcher("prepare"); report.preparationMs = Date.now() - preparing;
  const original = secretHashes();
  const concurrent = await Promise.all([0, 1].map(() => new Promise((resolve, reject) => {
    const child = spawn("sh", [join(bundle, "synveda-compose"), "prepare"], { env, cwd: bundle, stdio: "ignore" });
    child.on("error", reject); child.on("close", resolve);
  })));
  assert.ok(concurrent.includes(0) && concurrent.every((code) => code === 0 || code === 75));
  assert.deepEqual(secretHashes(), original);
  report.checks.concurrentPreparationPreservesSecrets = true;
  // Losing one credential must refuse, never silently replace a live key set.
  const key = join(home, "state/synveda-evaluation/secrets/synveda_kms_key");
  renameSync(key, join(home, "saved-key"));
  try {
    // Desktop file sharing can lag a host rename. Observe the injected fault
    // inside the same bind mount before testing the application's refusal.
    run("docker", ["run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL",
      "--user", `${process.getuid()}:${process.getgid()}`, "--mount", `type=bind,source=${join(home, "state")},target=/state,readonly`,
      "--entrypoint", "sh", manifest.images.product, "-c",
      "for attempt in 1 2 3 4 5 6 7 8 9 10; do test ! -e /state/synveda-evaluation/secrets/synveda_kms_key && exit 0; sleep 1; done; exit 1"]);
    const refused = spawnSync("sh", [join(bundle, "synveda-compose"), "prepare"], { env, cwd: bundle, encoding: "utf8", timeout: 120_000 });
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /incomplete or unsafe/);
  } finally { renameSync(join(home, "saved-key"), key); }
  launcher("prepare"); assert.deepEqual(secretHashes(), original);
  report.checks.missingCredentialRefusedAndRetryRecovered = true;
  const started = Date.now();
  launcher("up");
  report.coldStartupMs = report.preparationMs + Date.now() - started;
  browser(); report.checks.emptyConsoleLoginLogout = true;
  launcher("sample"); launcher("sample");
  report.checks.optInSampleAndRepeat = true;
  launcher("down");
  const warm = Date.now(); launcher("up"); report.warmRecreationMs = Date.now() - warm;
  assert.deepEqual(secretHashes(), original); browser(); launcher("sample");
  report.checks.recreationPreservesKeysIdentityAndReceipt = true;
  launcher("backup", "qualification");
  console.log("qualification: reset");
  run("sh", [join(bundle, "synveda-compose"), "reset"], { env: { ...env, SYNVEDA_CONFIRM_RESET: "delete:synveda-evaluation:postgres-data" } });
  console.log("qualification: restore");
  run("sh", [join(bundle, "synveda-compose"), "restore", "qualification"], { env: { ...env, SYNVEDA_CONFIRM_RESTORE: "restore:synveda-evaluation:qualification" } });
  browser(); launcher("sample"); assert.deepEqual(secretHashes(), original);
  report.checks.pairedDatabaseRestoreAndUsableAccess = true;
  const logs = run("docker", [...compose(), "logs", "--no-color"]);
  const secretRoot = join(home, "state/synveda-evaluation/secrets");
  for (const name of readdirSync(secretRoot).filter((name) => name.endsWith("_password") || name === "synveda_kms_key")) {
    const value = readFileSync(join(secretRoot, name), "utf8").trim();
    assert.ok(value.length >= 16 && !logs.includes(value), "secret value appeared in ordinary service logs");
  }
  report.checks.noPasswordsOrEncryptionKeyInServiceLogs = true;
  report.resources = run("docker", ["stats", "--no-stream", "--format", "{{.Name}} {{.MemUsage}} {{.CPUPerc}}", ...run("docker", ["ps", "-q", "--filter", "label=com.docker.compose.project=synveda-evaluation"]).trim().split("\n")]).trim().split("\n");
  report.tools = { docker: run("docker", ["version", "--format", "{{.Server.Version}}"]).trim(), compose: run("docker", ["compose", "version", "--short"]).trim(), architecture: process.arch, platform: process.platform };
  report.manualActions = { definition: "after download/extraction: up, credential retrieval, browser sign-in; optional sample is a fourth command/action", basic: 3, withSample: 4 };
  report.failures = evaluationFailureChecks(bundle, home);
  writeFileSync(resolve(output), `${JSON.stringify(report, null, 2)}\n`);
  console.log(`PASS candidate Docker installation and recovery: ${resolve(output)}`);
} catch (error) {
  reportFailedRealmConvergence();
  throw error;
} finally {
  // Only the exact project proved absent above was created by this fixture.
  // Keep private backups for diagnosis; no global prune or unrelated cleanup.
  const result = spawnSync("sh", [join(bundle, "synveda-compose"), "down"], { env, cwd: bundle, stdio: "ignore", timeout: 240_000 });
  if (result.status !== 0) console.error(`qualification cleanup incomplete; private state remains at ${home}`);
}

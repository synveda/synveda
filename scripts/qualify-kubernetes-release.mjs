#!/usr/bin/env node
// OPS-11: retag pulled bytes for the existing fixture, never rebuild them.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
const [input, chartArchive, output] = process.argv.slice(2);
assert.ok(input && chartArchive && output, "usage: qualify-kubernetes-release.mjs <bundle> <chart.tgz> <report.json>");
const manifest = JSON.parse(readFileSync(join(resolve(input), "environment.json"), "utf8"));
const scratch = mkdtempSync(join(tmpdir(), "synveda-qualified-chart-"));
function run(command, args, env = process.env) {
  const result = spawnSync(command, args, { env, stdio: "inherit", timeout: 3600_000 });
  assert.equal(result.status, 0, `${command} ${args[0]} failed`);
}
run("tar", ["-xzf", resolve(chartArchive), "-C", scratch]);
const env = { ...process.env, BUNDLED_MATRIX: "1", PORTABILITY: "1", OPERATIONS: "1", STARTER_MATRIX: "1", SKIP_BUILD: "1", NATIVE_IMPORT: "1", RELEASE_CHART: join(scratch, "synveda"), CLUSTER: "synveda-ops11-starter-release" };
for (const key of ["product", "postgres", "keycloak", "proxy"]) {
  const digest = manifest.images[key];
  assert.match(digest, /^[\w./:-]+@sha256:[a-f0-9]{64}$/);
  const tag = `synveda/release-${key}:${manifest.release_version}`;
  run("docker", ["tag", digest, tag]);
  env[`${key.toUpperCase()}_IMAGE`] = tag;
}
run("bash", ["demos/ops-2-helm-install.sh"], env);
const evidence = JSON.parse(readFileSync("demos/evidence/ops11-bundled.json", "utf8"));
run("bash", ["demos/ops-2-helm-install.sh"], { ...env, LOCAL_EVALUATION: "1", ...(process.platform === "linux" ? {
  BROWSER_IMAGE: manifest.images.browser_acceptance,
  BROWSER_SECCOMP: join(resolve(input), "deploy/compose/browser/seccomp_profile.json"),
} : {}) });
const localPortForward = JSON.parse(readFileSync("demos/evidence/ops11-local-evaluation.json", "utf8"));
writeFileSync(resolve(output), `${JSON.stringify({ source: manifest.source_sha, sourceDirty: manifest.source_dirty ?? false, version: manifest.release_version, images: manifest.images, chartArchive: resolve(chartArchive), evidence, localPortForward }, null, 2)}\n`);

// OPS-12: the plain-Compose subset of the existing extracted-release gate.
// Deliberately not release qualification until paired recovery also passes.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";

export function qualifyConsumer(input, output) {
  assert.ok(input && output, "usage: qualify-release.mjs --consumer-candidate EXTRACTED_BUNDLE REPORT.json");
  const bundle = resolve(input), reportPath = resolve(output);
  const manifest = JSON.parse(readFileSync(join(bundle, "environment.json"), "utf8"));
  const project = `synveda-local-acceptance-${randomUUID().slice(0, 8)}`;
  const env = { ...process.env, COMPOSE_PROJECT_NAME: project };
  for (const name of ["COMPOSE_FILE", "COMPOSE_PROFILES", "COMPOSE_ENV_FILES"]) delete env[name];
  const report = {
    qualification: "consumer-candidate-startup-and-recreation", version: manifest.release_version,
    source: manifest.source_sha, sourceDirty: manifest.source_dirty ?? false, images: manifest.images,
    at: new Date().toISOString(), project, checks: {}, timingsMs: {},
    imageDownloadBytes: null, imageDownloadMs: null, firstAuthenticatedHarnessCallMs: null,
    outstanding: ["paired named-volume backup/restore", "anonymous Docker Hub publication", "Docker Desktop and native Windows qualification"],
  };
  const result = (args, extra = {}) => spawnSync("docker", args, {
    env, cwd: bundle, encoding: "utf8", timeout: 660_000, maxBuffer: 8 * 1024 * 1024, ...extra,
  });
  const run = (args, extra) => {
    const value = result(args, extra);
    if (value.status !== 0) throw new Error(`Docker candidate ${args[0]} failed (${value.status ?? "timeout"}); ${value.stderr?.slice(-1800) ?? ""}`);
    return value.stdout;
  };
  const compose = (...args) => run(["compose", ...args]);
  const utility = (code) => compose("run", "--rm", "--no-deps", "--entrypoint", "node", "initialize", "-e", code);
  const hashes = () => utility("const fs=require('node:fs'),c=require('node:crypto');const p='/state/synveda-evaluation/secrets';console.log(JSON.stringify(fs.readdirSync(p).filter(n=>fs.lstatSync(p+'/'+n).isFile()).sort().map(n=>[n,c.createHash('sha256').update(fs.readFileSync(p+'/'+n)).digest('hex')])));");
  const browser = () => compose("run", "--rm", "--no-deps", "--entrypoint", "node", "browser-acceptance", "evaluation.mjs");
  const sample = () => compose("run", "--rm", "--no-deps", "--entrypoint", "node", "browser-acceptance", "product-demo.mjs", "sample");
  const missingInstallation = `${project}_missing-installation`;
  let missingVolumeCreated = false;
  assert.equal(run(["ps", "-aq", "--filter", `label=com.docker.compose.project=${project}`]).trim(), "", "candidate project already exists");
  assert.equal(run(["volume", "ls", "-q", "--filter", `name=^${project}_`]).trim(), "", "candidate volumes already exist");
  let failure;
  try {
    const imageRefs = [...new Set([...Object.entries(manifest.images).filter(([name]) => name !== "helm_postgres").map(([, reference]) => reference), manifest.external_images.otel_collector])];
    report.allImagesCachedBeforeStart = imageRefs.every((reference) => result(["image", "inspect", reference], { timeout: 30_000 }).status === 0);
    report.tools = {
      host: `${process.platform}/${process.arch}`,
      engine: run(["version", "--format", "{{.Server.Os}}/{{.Server.Arch}} {{.Server.Version}}"], { timeout: 30_000 }).trim(),
      engineOS: run(["info", "--format", "{{.OperatingSystem}}"], { timeout: 30_000 }).trim(),
      compose: run(["compose", "version", "--short"], { timeout: 30_000 }).trim(),
    };
    console.log("consumer qualification: fresh ordinary Compose startup");
    const starting = Date.now();
    compose("up", "-d", "--wait", "--wait-timeout", "600");
    report.timingsMs.freshStateStartup = Date.now() - starting;
    report.checks.freshOrdinaryComposeStartup = true;
    const original = hashes();
    compose("run", "--rm", "--no-deps", "initialize");
    assert.equal(hashes(), original, "repeated initialization changed secret files");
    report.checks.repeatedInitializationPreservesSecrets = true;

    console.log("consumer qualification: retained-volume refusals");
    utility("require('node:fs').renameSync('/state/synveda-evaluation/secrets/synveda_kms_key','/state/qualification-key');");
    try {
      const refused = result(["compose", "run", "--rm", "--no-deps", "initialize"]);
      assert.equal(refused.status, 78);
      assert.match(refused.stderr, /secret set is missing or changed/);
    } finally {
      utility("require('node:fs').renameSync('/state/qualification-key','/state/synveda-evaluation/secrets/synveda_kms_key');");
    }
    assert.equal(hashes(), original);
    report.checks.missingKeyRefusedAndOriginalRestored = true;
    run(["volume", "create", "--label", "com.synveda.test=ops12-consumer", missingInstallation]);
    missingVolumeCreated = true;
    const refused = result(["compose", "run", "--rm", "--no-deps", "--volume", `${missingInstallation}:/state`, "initialize"]);
    assert.equal(refused.status, 78);
    assert.match(refused.stderr, /retained PostgreSQL requires its matching installation volume/);
    assert.equal(hashes(), original);
    report.checks.lostInstallationVolumeRefusedBesideRetainedPostgres = true;

    console.log("consumer qualification: real browser and opt-in fictional sample");
    const signingIn = Date.now(); browser();
    report.timingsMs.browserLoginOperationLogout = Date.now() - signingIn;
    report.checks.browserIssuerPkceLoginApiLogout = true;
    sample(); sample();
    report.checks.optInSampleAndIdempotentRepeat = true;
    const credential = compose("run", "--rm", "--no-deps", "credentials");
    assert.match(credential, /^Username: synveda-demo-admin\nPassword: [A-Za-z0-9._~-]{32,}\n$/);
    const logs = compose("logs", "--no-color");
    // Private bytes stay inside this process; reports contain booleans only.
    const privateValues = JSON.parse(utility("const fs=require('node:fs');const p='/state/synveda-evaluation/secrets';console.log(JSON.stringify(fs.readdirSync(p).filter(n=>n.endsWith('_password')||n==='synveda_kms_key').map(n=>fs.readFileSync(p+'/'+n,'utf8').trim())));") );
    for (const value of privateValues) assert.ok(value.length >= 16 && !logs.includes(value), "private value appeared in ordinary service logs");
    report.checks.deliberateCredentialRetrievalAndNoSecretsInLogs = true;

    console.log("consumer qualification: ordinary down/up with retained data and keys");
    compose("down");
    const recreating = Date.now(); compose("up", "-d", "--wait", "--wait-timeout", "600");
    report.timingsMs.retainedStateStartup = Date.now() - recreating;
    assert.equal(hashes(), original);
    browser(); sample();
    report.checks.recreationPreservesKeysIdentityAndSample = true;
  } catch (error) {
    failure = error;
    report.failure = error.message;
  } finally {
    const stopped = result(["compose", "down"], { timeout: 300_000 });
    report.checks.exactProjectStoppedWithVolumesRetained = stopped.status === 0;
    if (stopped.status !== 0) failure ??= new Error(`candidate cleanup incomplete for ${project}; volumes retained`);
    if (missingVolumeCreated) {
      const label = result(["volume", "inspect", "--format", '{{index .Labels "com.synveda.test"}}', missingInstallation], { timeout: 30_000 });
      if (label.status === 0 && label.stdout.trim() === "ops12-consumer") run(["volume", "rm", missingInstallation], { timeout: 30_000 });
    }
    writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  }
  if (failure) throw failure;
  console.log(`PASS consumer candidate startup/recreation subset: ${reportPath}. Paired recovery remains unqualified.`);
}

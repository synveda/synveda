// CPR-45: deliberate faults only against state created by qualify-release.mjs.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function evaluationFailureChecks(bundle, home) {
  assert.ok(basename(home).startsWith("synveda-release-qualification-"), "fault drills require qualification-owned state");
  const manifest = JSON.parse(readFileSync(join(bundle, "environment.json"), "utf8"));
  const options = JSON.parse(readFileSync(join(bundle, "evaluation.json"), "utf8"));
  const env = { ...process.env, SYNVEDA_HOME: home };
  delete env.SYNVEDA_COMPOSE_RUNTIME;
  const run = (command, args) => spawnSync(command, args, { env, cwd: bundle, encoding: "utf8", timeout: 900_000, maxBuffer: 4 * 1024 * 1024 });
  const success = (command, args) => {
    const result = run(command, args);
    assert.equal(result.status, 0, `${command} ${args[0]} failed: ${result.stderr?.slice(-1000)}`);
    return result.stdout;
  };
  const compose = ["compose", "--project-name", "synveda-evaluation", "--env-file", join(home, "state/evaluation.env"),
    ...readFileSync(join(home, "state/evaluation-files"), "utf8").trim().split("\n").flatMap((file) => ["-f", join(bundle, "deploy/compose", file)])];
  const launcher = [join(bundle, "synveda-compose"), "up"];
  const reachable = () => success("curl", ["--fail", "--silent", "--show-error", "--connect-timeout", "5", "--max-time", "10", "--output", "/dev/null", `http://127.0.0.1:${options.port ?? 8080}/console/`]);
  const diagnostic = () => run("docker", [...compose, "run", "--rm", "--no-deps", "issuer-diagnostic"]);
  const issuerFile = join(home, "state/synveda-evaluation/issuers.json");
  const original = readFileSync(issuerFile);
  assert.equal(diagnostic().status, 0, "fault drill requires a healthy starting issuer");
  reachable();
  for (const fault of ["wrong-issuer", "unreachable-backchannel"]) {
    console.log(`qualification fault: ${fault}`);
    const configuration = JSON.parse(original);
    if (fault === "wrong-issuer") configuration[0].issuer += "-wrong";
    else configuration[0].discovery_url = "http://keycloak:1/realms/synveda/.well-known/openid-configuration";
    try {
      writeFileSync(issuerFile, JSON.stringify(configuration), { mode: 0o600 });
      const result = diagnostic();
      assert.ok(result.status !== null && result.status !== 0, `${fault} was not refused within the diagnostic deadline`);
      assert.match(result.stdout + result.stderr, /issuer|discovery|oidc/i);
    } finally { writeFileSync(issuerFile, original, { mode: 0o600 }); }
  }
  assert.equal(diagnostic().status, 0, "restoring the exact configuration must recover diagnostics");

  // Exhaust only a tiny, disposable container tmpfs; never fill the host/PVC.
  console.log("qualification fault: preparation-storage-exhaustion");
  const storage = run("docker", ["run", "--rm", "--network", "none", "--read-only", "--cap-drop", "ALL",
    "--security-opt", "no-new-privileges:true", "--user", `${process.getuid()}:${process.getgid()}`,
    "--tmpfs", "/tmp:rw,nosuid,nodev,size=32m",
    "--tmpfs", `/state:rw,nosuid,nodev,size=4k,mode=700,uid=${process.getuid()},gid=${process.getgid()}`,
    "--mount", `type=bind,source=${bundle},target=/bundle,readonly`,
    "-e", "SYNVEDA_HOST_STATE=/state", "-e", "SYNVEDA_HOST_BUNDLE=/bundle",
    "--entrypoint", "sh", manifest.images.product, "-c",
    'node /bundle/deploy/compose/scripts/prepare-evaluation.mjs; result=$?; df -Pk /state; exit "$result"']);
  assert.equal(storage.status, 78);
  // POSIX shell printf can report ENOSPC as a generic I/O error. Prove the
  // injected filesystem is full as well as checking the preparation refusal.
  assert.match(storage.stdout, /\s+0\s+100%\s+\/state(?:\r?\n|$)/);
  assert.match(storage.stderr, /no space|ENOSPC|I\/O error/i);

  const collision = `synveda-qualification-port-${process.pid}`;
  console.log("qualification fault: occupied-port");
  success("docker", [...compose, "stop", "proxy"]);
  success("docker", [...compose, "rm", "--force", "proxy"]);
  let ownsCollision = false;
  try {
    success("docker", ["run", "--detach", "--name", collision, "--read-only", "--cap-drop", "ALL",
      "--publish", `127.0.0.1:${options.port ?? 8080}:8080`, "--entrypoint", "sleep", manifest.images.product, "600"]);
    ownsCollision = true;
    const refused = run("sh", launcher);
    assert.ok(refused.status !== null && refused.status !== 0, "occupied port must refuse startup");
    assert.match(refused.stdout + refused.stderr, /port.*allocated|address already in use|bind.*failed/i);
  } finally {
    if (ownsCollision) success("docker", ["rm", "--force", collision]);
  }
  // A daemon may retain an unbound endpoint after a failed port allocation.
  // Exercise the documented recreation recovery, then check the host edge;
  // a healthy in-container probe alone cannot prove a published port works.
  success("sh", [join(bundle, "synveda-compose"), "down"]);
  success("sh", launcher);
  reachable();
  return { wrongIssuerRefused: true, unreachableBackchannelRefused: true, preparationStorageExhaustionRefused: true, occupiedDockerPortRefused: true, recoveredAfterFaults: true };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [input, state, reportPath] = process.argv.slice(2);
  assert.ok(input && state && reportPath, "usage: evaluation-failure-checks.mjs <bundle> <qualification-home> <report.json>");
  const checks = evaluationFailureChecks(resolve(input), resolve(state));
  writeFileSync(resolve(reportPath), `${JSON.stringify(checks, null, 2)}\n`);
  console.log("PASS bounded evaluation fault refusal and recovery");
}

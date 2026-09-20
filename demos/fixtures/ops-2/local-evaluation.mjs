// OPS-11: exercise the shipped preparation and loopback port-forward recipe.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const cluster = "synveda-ops11-local-evaluation";
const scratch = mkdtempSync(join(tmpdir(), "synveda-local-chart-"));
const env = { ...process.env, KUBECONFIG: join(scratch, "kubeconfig") };
const product = process.env.PRODUCT_IMAGE ?? "synveda/installation-candidate:local";
const postgres = process.env.POSTGRES_IMAGE ?? "synveda/postgres:ops11";
const keycloak = process.env.KEYCLOAK_IMAGE ?? "synveda/keycloak:ops11";
const namespace = "synveda-evaluation";
const chart = process.env.RELEASE_CHART ?? "deploy/helm/synveda";
let owned = false;
const forwards = [];
function run(command, args, extra = {}) {
  const result = spawnSync(command, args, { env, encoding: "utf8", timeout: 120_000, maxBuffer: 8 * 1024 * 1024, ...extra });
  assert.equal(result.status, 0, `${command} ${args[0]} failed: ${result.stderr}`);
  return result.stdout;
}
const k = (args) => run("kubectl", ["-n", namespace, ...args]);
function forward(service, ports) {
  const child = spawn("kubectl", ["-n", namespace, "port-forward", "--address", "127.0.0.1", `service/${service}`, ports], { env, stdio: ["ignore", "pipe", "pipe"] });
  forwards.push(child);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("port-forward readiness timeout")), 30_000);
    child.stdout.on("data", (data) => { if (data.toString().includes("Forwarding from")) { clearTimeout(timer); resolve(); } });
    child.on("exit", (code) => { clearTimeout(timer); reject(new Error(`port-forward exited ${code}`)); });
  });
}
try {
  assert.ok(!run("kind", ["get", "clusters"]).trim().split(/\s+/).includes(cluster), "fixture refuses an existing cluster");
  run("kind", ["create", "cluster", "--name", cluster, "--kubeconfig", env.KUBECONFIG, "--config", "demos/fixtures/ops-2/kind-cluster.yaml", "--wait", "120s"], { timeout: 240_000 });
  owned = true;
  const architecture = run("docker", ["info", "--format", "{{.Architecture}}"]).trim();
  const platform = { aarch64: "linux/arm64", arm64: "linux/arm64", x86_64: "linux/amd64", amd64: "linux/amd64" }[architecture];
  assert.ok(platform, "unsupported native image platform");
  for (const image of [product, postgres, keycloak]) {
    const archive = join(scratch, "native-image.tar");
    run("docker", ["image", "save", "--platform", platform, "-o", archive, image], { timeout: 600_000 });
    run("kind", ["load", "image-archive", "--name", cluster, archive], { timeout: 600_000 });
    rmSync(archive);
  }
  run("sh", [`${chart}/examples/prepare-local.sh`, scratch], { env: { ...env, SYNVEDA_PRODUCT_IMAGE: product } });
  const values = JSON.parse(readFileSync(join(scratch, "values.json"), "utf8"));
  const imageValue = (image) => ({ repository: image.slice(0, image.lastIndexOf(":")), tag: image.slice(image.lastIndexOf(":") + 1), pullPolicy: "Never" });
  values.image = imageValue(product);
  values.postgres.bundled.image = postgres;
  values.keycloak.image = imageValue(keycloak);
  // Prove filesystem behaviour for assigned identities on all bundled
  // processes, including PostgreSQL. Kubernetes admission is not OpenShift SCC.
  const assigned = { runAsNonRoot: true, runAsUser: 1000900000, runAsGroup: 0, fsGroup: 1000900000, seccompProfile: { type: "RuntimeDefault" } };
  values.podSecurityContext = assigned;
  values.postgres.bundled.podSecurityContext = assigned;
  values.keycloak.podSecurityContext = assigned;
  values.keycloak.securityContext = { runAsNonRoot: true, runAsUser: 1000900000, allowPrivilegeEscalation: false, readOnlyRootFilesystem: true, capabilities: { drop: ["ALL"] } };
  writeFileSync(join(scratch, "candidate.json"), JSON.stringify(values), { mode: 0o600 });
  k(["create", "namespace", namespace]);
  k(["label", "namespace", namespace, "pod-security.kubernetes.io/enforce=restricted"]);
  k(["apply", "-f", join(scratch, "secrets.json")]);
  const start = Date.now();
  run("helm", ["upgrade", "--install", "synveda", chart, "-n", namespace, "-f", join(scratch, "candidate.json"), "--wait", "--wait-for-jobs", "--timeout", "15m"], { timeout: 920_000 });
  const startupMs = Date.now() - start;
  // A migration run under the ordinary gateway role must fail; a subsequent
  // correctly provisioned chart reapply must retain usable data and identity.
  const installation = JSON.parse(k(["get", "jobs", "-o", "json"])).items.find((job) => job.metadata.name.startsWith("synveda-install-"));
  const refusedMigration = structuredClone(installation.spec.template.spec);
  refusedMigration.containers = [refusedMigration.initContainers.find((container) => container.name === "migrate")];
  delete refusedMigration.initContainers;
  refusedMigration.volumes.find((volume) => volume.name === "migrator-database").secret = {
    secretName: values.gateway.databaseExistingSecret, items: [{ key: values.gateway.databaseUrlSecretKey, path: "database_url" }],
  };
  run("kubectl", ["-n", namespace, "apply", "-f", "-"], { input: JSON.stringify({ apiVersion: "batch/v1", kind: "Job", metadata: { name: "migration-role-refusal" }, spec: { backoffLimit: 0, activeDeadlineSeconds: 90, template: { spec: refusedMigration } } }) });
  k(["wait", "job/migration-role-refusal", "--for=condition=Failed", "--timeout=120s"]);
  assert.match(k(["logs", "job/migration-role-refusal", "-c", "migrate"]), /role|owner|migrat/i);
  run("helm", ["upgrade", "--install", "synveda", chart, "-n", namespace, "-f", join(scratch, "candidate.json"), "--wait", "--wait-for-jobs", "--timeout", "15m"], { timeout: 920_000 });
  await forward("synveda", "8120:8120");
  await forward("synveda-keycloak-http", "8081:80");
  const browserEnvironment = {
    SYNVEDA_BROWSER_APP_URL: "http://localhost:8120", SYNVEDA_BROWSER_ISSUER: "http://localhost:8081/realms/synveda",
    SYNVEDA_BOOTSTRAP_TENANT_ID: "019b53c0-7c00-7000-8000-000000000045",
  };
  if (process.env.BROWSER_IMAGE) {
    assert.equal(process.platform, "linux", "container host networking is qualified only on Linux");
    assert.ok(process.env.BROWSER_SECCOMP, "exact release browser seccomp required");
    console.log(run("docker", ["run", "--rm", "--network", "host", "--read-only", "--cap-drop", "ALL",
      "--security-opt", "no-new-privileges:true", "--security-opt", `seccomp=${process.env.BROWSER_SECCOMP}`,
      "--user", `${process.getuid()}:${process.getgid()}`, "--pids-limit", "256", "--memory", "1g", "--shm-size", "256m",
      "--tmpfs", "/tmp:rw,nosuid,nodev,size=256m", "--tmpfs", `/var/lib/synveda-browser:rw,nosuid,nodev,size=64m,uid=${process.getuid()},gid=${process.getgid()},mode=700`,
      "--mount", `type=bind,source=${join(scratch, "secrets.json")},target=/input/secrets.json,readonly`,
      ...Object.entries(browserEnvironment).flatMap(([key, value]) => ["-e", `${key}=${value}`]),
      "-e", "SYNVEDA_TEST_KUBERNETES_SECRETS=/input/secrets.json", "--entrypoint", "sh", process.env.BROWSER_IMAGE,
      "-c", "mkdir -p \"$HOME\" \"$XDG_CONFIG_HOME\" \"$XDG_STATE_HOME\"; exec node evaluation.mjs"]));
  } else console.log(run("node", ["deploy/compose/browser/evaluation.mjs"], { env: { ...env, ...browserEnvironment, SYNVEDA_TEST_KUBERNETES_SECRETS: join(scratch, "secrets.json") } }));
  const versions = JSON.parse(k(["version", "-o", "json"]));
  const report = { at: new Date().toISOString(), startupMs, versions: { helm: run("helm", ["version", "--short"]).trim(), kubernetes: versions.serverVersion.gitVersion, kubectl: versions.clientVersion.gitVersion }, restrictedAdmission: true, assignedUid: 1000900000, assignedUidProcesses: ["gateway", "worker", "install", "keycloak", "postgres"], runtimeRoleMigrationRefused: true, reapplyAfterRefusal: true, openShiftScc: "NOT RUN: no OpenShift cluster", portForward: "127.0.0.1 only", browserLoginLogout: true, images: { product, postgres, keycloak }, clusterWideInstall: false };
  writeFileSync("demos/evidence/ops11-local-evaluation.json", `${JSON.stringify(report, null, 2)}\n`);
  console.log("PASS packaged Kubernetes evaluation: loopback login/logout, no operator, restricted admission");
} finally {
  for (const child of forwards) child.kill("SIGTERM");
  if (owned) run("kind", ["delete", "cluster", "--name", cluster], { timeout: 180_000 });
  rmSync(scratch, { recursive: true, force: true });
}

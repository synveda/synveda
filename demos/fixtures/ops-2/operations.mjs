// OPS-11: bounded drills in namespaces created by starter.mjs. No shared-cluster entry point.
import assert from "node:assert/strict";
import { spawnSync, spawn } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

export async function operations(c) {
  const { ns, providers, identityNs, database, identity, values, k, run, env, scratch, chart, parseYaml,
    apply, secret, providerDatabase, ca, passwords, roles, helm, wait, team, quiesce } = c;
  const result = {};
  const sql = (namespace, target, db, query) => k(["exec", "-n", namespace, target, "--", "psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", db, "-c", query]).trim();
  const primary = () => database === "cnpg"
    ? k(["get", "cluster/synveda-pg", "-n", ns, "-o", "jsonpath={.status.currentPrimary}"]).trim()
    : database === "bundled" ? "synveda-pg-0" : "deployment/postgres";
  const dbNs = database === "external" ? providers : ns;

  // A provider that intentionally holds a request proves the claim is live at
  // SIGTERM. It never returns an untrusted payload or contacts an external model.
  const interrupted = structuredClone(values);
  interrupted.extractor = { kind: "vllm", model: "ops11-interruption", baseUrl: "http://team-test:8088" };
  helm(ns, interrupted);
  team(ns, "job-start");
  const stats = () => JSON.parse(k(["exec", "-n", ns, "team-test", "-c", "node", "--", "node", "-e", "fetch('http://127.0.0.1:8088/stats').then(r=>r.text()).then(console.log)"]));
  for (let attempt = 0; attempt < 30 && stats().calls === 0; attempt++) await sleep(500);
  assert.equal(stats().calls, 1, "worker must be inside the first extractor call");
  assert.equal(stats().active, 1, "the provider request must still be blocked");
  const worker = JSON.parse(k(["get", "pods", "-n", ns, "-l", "app.kubernetes.io/component=worker", "-o", "json"])).items[0];
  const workerPod = worker.metadata.name, restartsBefore = worker.status.containerStatuses[0].restartCount;
  const stopAt = Date.now();
  // Signal PID 1 through the normal container namespace; kubelet restarts the
  // same pod, preserving terminated exit status for inspection.
  k(["exec", "-n", ns, workerPod, "--", "sh", "-ec", "kill -TERM 1"]);
  let status;
  for (let attempt = 0; attempt < 90; attempt++) {
    status = JSON.parse(k(["get", "pod", "-n", ns, workerPod, "-o", "json"])).status.containerStatuses[0];
    if (status.restartCount > restartsBefore && status.ready) break;
    await sleep(1000);
  }
  assert.ok(status.restartCount > restartsBefore && status.ready, "worker did not restart ready within 90 seconds");
  assert.equal(status.lastState.terminated.exitCode, 0, "SIGTERM did not exit cooperatively");
  const logs = k(["logs", "-n", ns, workerPod, "--previous"]);
  assert.ok(logs.includes("worker readiness withdrawn"), "worker did not report readiness withdrawal");
  // The outer authority gate can cancel the inner Capture future before its
  // own shutdown log runs. Observe the provider socket and final database
  // effects instead of depending on which supervised receiver wins that race.
  assert.ok(logs.includes("core worker stopped cleanly"), "worker did not report cooperative shutdown");
  assert.equal(stats().cancelled, 1, "shutdown must cancel the blocked provider request");
  k(["exec", "-n", ns, "team-test", "-c", "node", "--", "node", "-e", "fetch('http://127.0.0.1:8088/release',{method:'POST'}).then(r=>{if(!r.ok)process.exit(1)})"]);
  result.interruptedJob = { ...team(ns, "job-verify"), recoveryMs: Date.now() - stopAt, providerCalls: stats().calls, cancelledProviderRequests: stats().cancelled, exitCode: status.lastState.terminated.exitCode, readinessWithdrawn: true };
  helm(ns, values);
  const gateway = JSON.parse(k(["get", "pods", "-n", ns, "-l", "app.kubernetes.io/component=gateway", "-o", "json"])).items[0];
  const gatewayStopAt = Date.now();
  k(["exec", "-n", ns, gateway.metadata.name, "--", "sh", "-ec", "kill -TERM 1"]);
  for (let attempt = 0; attempt < 60; attempt++) {
    status = JSON.parse(k(["get", "pod", "-n", ns, gateway.metadata.name, "-o", "json"])).status.containerStatuses[0];
    if (status.restartCount > gateway.status.containerStatuses[0].restartCount && status.ready) break;
    await sleep(1000);
  }
  assert.ok(status.restartCount > gateway.status.containerStatuses[0].restartCount && status.ready, "gateway did not restart ready within 60 seconds");
  assert.equal(status.lastState.terminated.exitCode, 0, "gateway SIGTERM did not exit cooperatively");
  result.gatewayShutdown = { exitCode: 0, restartedReadyMs: Date.now() - gatewayStopAt, inFlightHttpDrainTested: false };
  const repeatedAt = Date.now();
  result.repeatedWorkload = { rounds: [] };
  for (let round = 0; round < 5; round++) {
    team(ns, "verify");
    result.repeatedWorkload.rounds.push(team(ns, "load"));
  }
  result.repeatedWorkload.durationMs = Date.now() - repeatedAt;

  // Two actual migration commands contend for SQLx's advisory lock. Observe the
  // wait in pg_locks while holding that exact lock in a bounded admin session.
  const lock = run("python3", ["-c", "import zlib; print(0x3d32ad9e * zlib.crc32(b'synveda'))"]).trim();
  const locker = spawn("kubectl", ["exec", "-n", dbNs, primary(), "--", "psql", "-X", "-qAt", "-U", "postgres", "-d", "synveda", "-c", `SELECT pg_advisory_lock(${lock}); SELECT pg_sleep(25);`], { env, stdio: "ignore" });
  const lockDone = new Promise((resolve, reject) => { locker.on("error", reject); locker.on("close", (code) => code === 0 ? resolve() : reject(new Error("migration lock holder failed"))); });
  const install = JSON.parse(k(["get", "jobs", "-n", ns, "-o", "json"])).items.filter((j) => j.metadata.name.startsWith("synveda-install-")).sort((a, b) => Number(b.metadata.name.split("-").at(-1)) - Number(a.metadata.name.split("-").at(-1)))[0];
  const migrationSpec = structuredClone(install.spec.template.spec);
  migrationSpec.containers = [migrationSpec.initContainers.find((p) => p.name === "migrate")];
  delete migrationSpec.initContainers;
  for (const name of ["migration-a", "migration-b"]) apply(ns, { apiVersion: "batch/v1", kind: "Job", metadata: { name }, spec: { activeDeadlineSeconds: 120, backoffLimit: 0, template: { spec: migrationSpec } } });
  let waiting = 0;
  for (let attempt = 0; attempt < 20; attempt++) {
    waiting = Number(sql(dbNs, primary(), "synveda", "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a USING(pid) WHERE l.locktype='advisory' AND NOT l.granted AND a.usename='synveda_migrator'"));
    if (waiting === 2) break;
    await sleep(500);
  }
  assert.equal(waiting, 2, "both migrators must wait on the existing SQLx lock");
  await lockDone;
  for (const name of ["migration-a", "migration-b"]) k(["wait", "-n", ns, `job/${name}`, "--for=condition=Complete", "--timeout=120s"], { timeout: 130000 });
  assert.equal(sql(dbNs, primary(), "synveda", "SELECT count(*) FROM _sqlx_migrations WHERE success"), "1");
  result.migration = { concurrentWaiters: waiting, completedReruns: 2, baselineRows: 1 };

  // Reuse the workload after reconnection; no application data is inspected
  // with the backup administrator. Mandatory dependency loss closes readiness.
  const queryHealth = (path) => k(["exec", "-n", ns, "team-test", "-c", "node", "--", "node", "-e", `fetch('http://synveda:8120/${path}',{signal:AbortSignal.timeout(5000)}).then(r=>console.log(r.status)).catch(()=>console.log(0))`]).trim();
  if (database === "external") {
    k(["scale", "-n", providers, "deployment/postgres", "--replicas=0"]);
    k(["wait", "-n", providers, "pod", "-l", "app=postgres", "--for=delete", "--timeout=180s"], { timeout: 190000 });
    for (let attempt = 0; attempt < 40 && queryHealth("readyz") !== "503"; attempt++) await sleep(1000);
    assert.equal(queryHealth("readyz"), "503"); assert.equal(queryHealth("healthz"), "200");
    const resumed = Date.now();
    k(["scale", "-n", providers, "deployment/postgres", "--replicas=1"]); wait(providers, "deployment", "postgres");
    // Deployment Ready can still reflect a probe taken before the outage.
    // Prove the current authority gates reopened, within a wall-clock bound.
    const deadline = Date.now() + 90000;
    let workerReady = "0";
    while (Date.now() < deadline) {
      workerReady = k(["exec", "-n", ns, "deployment/synveda-worker", "--", "curl", "--silent", "--max-time", "5", "--output", "/dev/null", "--write-out", "%{http_code}", "http://127.0.0.1:8121/readyz"]).trim();
      if (queryHealth("readyz") === "200" && workerReady === "200") break;
      await sleep(1000);
    }
    assert.equal(queryHealth("readyz"), "200", "gateway did not recover within 90 seconds");
    assert.equal(workerReady, "200", "worker did not recover within 90 seconds");
    wait(ns, "deployment", "synveda"); wait(ns, "deployment", "synveda-worker");
    wait(identityNs, "statefulset", "keycloak");
    const gateRecoveredMs = Date.now() - resumed;
    const publicFlow = team(ns, "reconnect");
    result.reconnect = { unavailableReadiness: 503, liveDuringOutage: 200, gateRecoveredMs, ...publicFlow, recoveredMs: Date.now() - resumed };
  }

  // Ordinary PostgreSQL tools write outside every working PVC. Scratch is a
  // private host directory; production custody is a separate encrypted copy.
  result.recoveryPoint = team(ns, "recovery-point");
  const backupAt = Date.now();
  await quiesce(ns, identityNs);
  const backup = join(scratch, `${ns}-backup`); mkdirSync(backup, { mode: 0o700 });
  const archived = [];
  for (const db of ["synveda", "keycloak"]) {
    const namespace = db === "synveda" || (database !== "external" && identity === "packaged") ? dbNs : providers;
    const target = namespace === dbNs ? primary() : "deployment/postgres";
    // PostgreSQL can retain a backend briefly after Kubernetes removes its
    // owning Pod. Fail closed if any client remains past the bounded drain.
    const drainDeadline = Date.now() + 30_000;
    let connections;
    do {
      connections = sql(namespace, target, "postgres", `SELECT count(*) FROM pg_stat_activity WHERE datname='${db}'`);
      if (connections === "0" || Date.now() >= drainDeadline) break;
      await sleep(1000);
    } while (true);
    assert.equal(connections, "0", `${db}: all clients must disconnect before the pair is dumped`);
    const dump = spawnSync("kubectl", ["exec", "-n", namespace, target, "--", "pg_dump", "-U", "postgres", "-d", db, "--format=custom", "--create", "--lock-wait-timeout=10s"], { env, timeout: 120000, maxBuffer: 64 * 1024 * 1024 });
    assert.equal(dump.status, 0, `${db} pg_dump failed`);
    writeFileSync(join(backup, `${db}.dump`), dump.stdout, { mode: 0o600 });
    archived.push({ database: db, bytes: dump.stdout.length, sha256: createHash("sha256").update(dump.stdout).digest("hex") });
  }
  result.backup = { quiesced: true, durationMs: Date.now() - backupAt, archives: archived, offPvc: true, encryptedOffHostCopy: false };
  const restoreAt = Date.now(), restored = `${ns}-restore`;
  assert.equal(k(["get", "ns", restored, "--ignore-not-found", "-o", "name"]).trim(), "");
  k(["create", "namespace", restored]);
  const restorePasswords = { ...passwords };
  if (database === "cnpg") {
    const saved = JSON.parse(k(["get", "secret/synveda-pg-app", "-n", ns, "-o", "json"], { quiet: true }));
    restorePasswords.synveda_migrator_password = Buffer.from(saved.data.password, "base64").toString();
  }
  const restoreHost = providerDatabase(restored, restorePasswords, roles, ca);
  secret(restored, "provider-ca", { "ca.crt": ca });
  for (const name of ["synveda-kms", "synveda-oidc", "synveda-keycloak-db", "synveda-keycloak-admin"]) {
    const sourceNs = name.startsWith("synveda-keycloak") ? identityNs : ns;
    const saved = JSON.parse(k(["get", `secret/${name}`, "-n", sourceNs, "-o", "json"], { quiet: true }));
    apply(restored, { apiVersion: "v1", kind: "Secret", metadata: { name }, type: saved.type, data: saved.data });
  }
  for (const role of ["migrator", "gateway", "worker"]) secret(restored, `synveda-${role}-db`, { DATABASE_URL: `postgresql://synveda_${role}:${restorePasswords[`synveda_${role}_password`]}@${restoreHost}:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt` });
  for (const db of ["synveda", "keycloak"]) {
    assert.equal(sql(restored, "deployment/postgres", db, "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema')"), "0", "refuse nonempty restore target");
    const load = spawnSync("kubectl", ["exec", "-i", "-n", restored, "deployment/postgres", "--", "pg_restore", "-U", "postgres", "-d", "postgres", "--clean", "--if-exists", "--create", "--exit-on-error"], { env, input: readFileSync(join(backup, `${db}.dump`)), timeout: 120000, maxBuffer: 1024 * 1024 });
    assert.equal(load.status, 0, `${db} pg_restore failed`);
    sql(restored, "deployment/postgres", db, "ANALYZE");
  }
  const restoredValues = structuredClone(values);
  restoredValues.postgres = parseYaml(readFileSync(`${chart}/ci/external-values.yaml`, "utf8")).postgres;
  Object.assign(restoredValues.postgres.external, { host: restoreHost, caExistingSecret: "provider-ca", roles });
  Object.assign(restoredValues.keycloak, { enabled: true, databaseCaExistingSecret: "provider-ca" });
  restoredValues.keycloak.database.hostname = restoreHost;
  const savedProxy = JSON.parse(k(["get", "configmap/proxy", "-n", ns, "-o", "json"]));
  // Apply declarative fields only; replaying the old resourceVersion after
  // the route switch would conflict with Kubernetes' current object version.
  const sourceProxy = { apiVersion: "v1", kind: "ConfigMap", metadata: { name: "proxy" }, data: savedProxy.data };
  const proxy = structuredClone(sourceProxy);
  proxy.data.Caddyfile = proxy.data.Caddyfile.replace("reverse_proxy synveda:8120", `reverse_proxy synveda.${restored}.svc.cluster.local:8120`).replace(/reverse_proxy keycloak-http\.[^\s]+/, `reverse_proxy keycloak-http.${restored}.svc.cluster.local:80`);
  apply(ns, proxy);
  k(["rollout", "restart", "-n", ns, "deployment/proxy"]); wait(ns, "deployment", "proxy");
  helm(restored, restoredValues);
  const verification = k(["exec", "-n", ns, "team-test", "-c", "node", "--", "env", `DIRECT_APP=http://synveda.${restored}.svc.cluster.local:8120`, "node", "/scripts/starter-team.mjs", "restore-verify"], { timeout: 180000 });
  result.restore = { ...JSON.parse(verification.trim().split("\n").at(-1)), durationMs: Date.now() - restoreAt, target: "fresh namespace; independent PostgreSQL; packaged Keycloak", issuerPreserved: true, sourceDataRetained: true };
  console.log(`PASS joint restore: ${result.restore.durationMs} ms; native logical archives plus original KMS and issuer`);
  await quiesce(restored, restored);
  apply(ns, sourceProxy); k(["rollout", "restart", "-n", ns, "deployment/proxy"]); wait(ns, "deployment", "proxy");
  k(["scale", "-n", identityNs, "statefulset/keycloak", "--replicas=1"]); wait(identityNs, "statefulset", "keycloak");
  k(["scale", "-n", ns, "deployment/synveda", "deployment/synveda-worker", "--replicas=1"]);
  wait(ns, "deployment", "synveda"); wait(ns, "deployment", "synveda-worker");
  team(ns, "verify");
  k(["delete", "namespace", restored, "--wait=true", "--timeout=180s"], { timeout: 200000 });
  return result;
}

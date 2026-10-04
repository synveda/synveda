#!/usr/bin/env node
// OPS-11: bounded, allowlisted operator checks. No implicit cluster mutation.
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash, X509Certificate } from "node:crypto";
import { derive, json, parseJson, privatePath } from "./prepare.mjs";

const chart = fileURLToPath(new URL("..", import.meta.url));
export function command(binary, args, input) {
  const r = spawnSync(binary, args, { encoding: "utf8", input, timeout: 20000, maxBuffer: 4 * 1024 * 1024 });
  // Caller-generated messages deliberately exclude kubectl stderr and raw URLs.
  return { ok: r.status === 0, text: r.stdout ?? "" };
}
export function quantity(value, memory = false) {
  const m = String(value).match(/^([0-9]+(?:\.[0-9]+)?|\.[0-9]+)(m|Ki|Mi|Gi|Ti|K|M|G|T)?$/);
  if (!m) throw new Error("unsupported resource quantity; inspect rendered requests manually");
  return Number(m[1]) * (memory ? ({ Ki: 1 / 1024, Mi: 1, Gi: 1024, Ti: 1048576, K: 1000 / 1048576, M: 1000000 / 1048576, G: 1000000000 / 1048576, T: 1000000000000 / 1048576 }[m[2]] ?? 1 / 1048576) : m[2] === "m" ? 1 : 1000);
}
export function requests(v) {
  const resource = (r, cpu, memory) => ({ cpu: quantity(r?.requests?.cpu ?? cpu), memory: quantity(r?.requests?.memory ?? memory, true) });
  const workloads = [resource(v.gateway.resources, "500m", "512Mi"), resource(v.worker.resources, "250m", "256Mi")];
  for (let i = 0; i < (v.worker.captureOnlyReplicas ?? 0); i++) workloads.push(resource(v.worker.resources, "250m", "256Mi"));
  if (v.postgres.mode === "bundled") workloads.push(resource(v.postgres.bundled.resources, "250m", "512Mi"));
  if (v.postgres.mode === "cnpg") for (let i = 0; i < (v.postgres.instances ?? 1); i++) workloads.push(resource(v.postgres.resources, "1", "2Gi"));
  if (v.keycloak?.enabled) workloads.push(resource(v.keycloak.resources, "500m", "1280Mi"));
  if (v.tei?.enabled) workloads.push(resource(v.tei.resources, "2", "4Gi"));
  const steady = workloads.reduce((a, x) => ({ cpu: a.cpu + x.cpu, memory: a.memory + x.memory }), { cpu: 0, memory: 0 });
  const job = resource(v.install?.resources, "100m", "128Mi");
  return { steady, installation: { cpu: steady.cpu + job.cpu, memory: steady.memory + job.memory }, pods: workloads.length + 1 };
}
export function containers(v) {
  const row = (name, r, cpu, memory, limit) => ({ name, requests: { cpu: quantity(r?.requests?.cpu ?? cpu), memory: quantity(r?.requests?.memory ?? memory, true) }, limits: { cpu: r?.limits?.cpu ? quantity(r.limits.cpu) : null, memory: r?.limits?.memory ?? limit ? quantity(r?.limits?.memory ?? limit, true) : null } });
  const rows = [row("gateway", v.gateway.resources, "500m", "512Mi", "2Gi"), row("worker", v.worker.resources, "250m", "256Mi", "1Gi"), row("install (sequential stages)", v.install?.resources, "100m", "128Mi", "1Gi")];
  for (let i = 0; i < (v.worker.captureOnlyReplicas ?? 0); i++) rows.push(row("capture worker", v.worker.resources, "250m", "256Mi", "1Gi"));
  if (v.postgres.mode === "bundled") rows.push(row("postgres", v.postgres.bundled.resources, "250m", "512Mi", "2Gi"));
  if (v.postgres.mode === "cnpg") for (let i = 0; i < (v.postgres.instances ?? 1); i++) rows.push(row("cnpg", v.postgres.resources, "1", "2Gi", null));
  if (v.keycloak?.enabled) rows.push(row("keycloak", v.keycloak.resources, "500m", "1280Mi", "2Gi"));
  if (v.tei?.enabled) rows.push(row("tei", v.tei.resources, "2", "4Gi", null));
  return rows;
}
export function storageRequests(v) {
  const claims = [];
  const add = (name, size, storageClass = "") => claims.push({ name, memory: quantity(size, true), storageClass });
  if (v.postgres.mode === "bundled" && !v.postgres.bundled.existingClaim) add("bundled database", v.postgres.bundled.size ?? "10Gi", v.postgres.bundled.storageClass);
  if (v.postgres.mode === "cnpg") for (let i = 0; i < (v.postgres.instances ?? 1); i++) add("CNPG database", v.postgres.storage.size ?? "100Gi", v.postgres.storage.storageClass);
  if (v.tei?.enabled && !v.tei.cache?.existingClaim) add("TEI cache", v.tei.cache?.size ?? "20Gi", v.tei.cache?.storageClass);
  return claims;
}
export function limitFailures(rows, ranges, claims = []) {
  const failures = [];
  for (const range of ranges) for (const rule of range.spec?.limits ?? []) {
    if (rule.type === "PersistentVolumeClaim") {
      for (const claim of claims) if ((rule.min?.storage && claim.memory < quantity(rule.min.storage, true)) || (rule.max?.storage && claim.memory > quantity(rule.max.storage, true))) failures.push(`${range.metadata.name}: ${claim.name} storage violates PVC bounds`);
      continue;
    }
    if (!["Container", "Pod"].includes(rule.type)) continue;
    for (const row of rows) for (const key of ["cpu", "memory"]) {
      const request = row.requests[key];
      const limit = row.limits[key] ?? (rule.default?.[key] ? quantity(rule.default[key], key === "memory") : null);
      const min = rule.min?.[key] ? quantity(rule.min[key], key === "memory") : null;
      const max = rule.max?.[key] ? quantity(rule.max[key], key === "memory") : null;
      if ((min !== null && request < min) || (limit !== null && limit < request) || (max !== null && (request > max || limit === null || limit > max)) || (rule.maxLimitRequestRatio?.[key] && limit !== null && limit / request > Number(rule.maxLimitRequestRatio[key]))) failures.push(`${range.metadata.name}: ${row.name} ${key} request/limit violates ${rule.type} bounds`);
    }
  }
  return failures;
}
export function secretKeys(v) {
  const refs = new Map();
  const add = (name, keys) => { if (name) refs.set(name, [...new Set([...(refs.get(name) ?? []), ...keys])]); };
  const db = v.postgres;
  for (const role of ["gateway", "worker"]) add(v[role].databaseExistingSecret, [v[role].databaseUrlSecretKey ?? "DATABASE_URL", ...(db.mode === "external" ? [] : [v[role].databasePasswordSecretKey ?? "password"])]);
  add(v.oidc.existingSecret, [v.oidc.secretKey ?? "SYNVEDA_OIDC_ISSUERS"]);
  add(v.oidc.caExistingSecret, [v.oidc.caSecretKey ?? "ca.crt"]);
  add(v.kms.existingSecret, [v.kms.secretKey ?? "SYNVEDA_KMS_KEY", v.kms.keyRefSecretKey ?? "SYNVEDA_KMS_KEY_REF"]);
  if (db.mode === "external") {
    add(db.external.migratorExistingSecret, [db.external.migratorUrlSecretKey ?? "DATABASE_URL"]);
    add(db.external.caExistingSecret, [db.external.caSecretKey ?? "ca.crt"]);
    add(db.external.clientExistingSecret, [db.external.clientCertSecretKey ?? "tls.crt", db.external.clientKeySecretKey ?? "tls.key"]);
  } else if (db.mode === "bundled") {
    add(db.bundled.administratorExistingSecret, [db.bundled.administratorPasswordSecretKey ?? "password"]);
    add(db.bundled.migratorExistingSecret, [db.bundled.migratorPasswordSecretKey ?? "password", db.bundled.migratorUrlSecretKey ?? "uri"]);
    add(db.bundled.tlsExistingSecret, [db.bundled.tlsCertSecretKey ?? "tls.crt", db.bundled.tlsKeySecretKey ?? "tls.key", db.bundled.caSecretKey ?? "ca.crt"]);
  }
  if (v.keycloak?.enabled) {
    add(v.keycloak.adminExistingSecret, ["username", "password"]);
    add(v.keycloak.database.existingSecret, [v.keycloak.database.existingSecretKey ?? "password"]);
    // CNPG creates its CA and owner Secrets after installation starts.
    if (db.mode !== "cnpg" || v.keycloak.databaseCaExistingSecret !== `${v.fullnameOverride}-pg-ca`) add(v.keycloak.databaseCaExistingSecret, [v.keycloak.databaseCaSecretKey ?? "ca.crt"]);
    add(v.keycloak.evaluationAccountsExistingSecret, ["admin", "member", "approver", "viewer"]);
    for (const ref of v.keycloak.imagePullSecrets ?? []) add(ref.name ?? ref, [".dockerconfigjson"]);
  }
  for (const tls of v.ingress?.tls ?? []) add(tls.secretName, ["tls.crt", "tls.key"]);
  for (const tls of v.keycloak?.ingress?.tls ?? []) add(tls.secretName, ["tls.crt", "tls.key"]);
  for (const ref of v.imagePullSecrets ?? []) add(ref.name, [".dockerconfigjson"]);
  add(v.extractor?.existingSecret, [v.extractor?.secretKey ?? "ANTHROPIC_API_KEY"]);
  add(v.outbound?.proxyExistingSecret, ["HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"]);
  add(v.outbound?.caBundleExistingSecret, [v.outbound?.caBundleSecretKey ?? "ca-bundle.crt"]);
  return refs;
}
export function storageResult(claim, storageClass) {
  if (claim?.status?.phase === "Bound") return { level: "PASS", message: "existing claim is Bound; matching database/key recovery still needs operator verification" };
  if (claim?.status?.phase === "Pending" && storageClass?.volumeBindingMode === "WaitForFirstConsumer") return { level: "PASS", message: "claim is waiting for its first consumer; scheduling the PostgreSQL pod triggers binding" };
  return { level: "FAIL", message: "claim is not Bound; inspect claim events and provisioner before retrying" };
}
export function diagnostic(pods = [], jobs = [], pvcs = []) {
  const reasons = new Set(["", "Completed", "Error", "OOMKilled", "CrashLoopBackOff", "ImagePullBackOff", "ErrImagePull", "CreateContainerConfigError", "CreateContainerError", "ContainerCreating", "PodInitializing", "Unschedulable", "DeadlineExceeded", "BackoffLimitExceeded", "JobAlreadyActive", "TooManyActivePods", "FailedCreate"]);
  const reason = (value) => reasons.has(value ?? "") ? value ?? "" : "unclassified";
  const phase = (value) => ["Pending", "Running", "Succeeded", "Failed", "Unknown", "Bound", "Lost"].includes(value) ? value : "unknown";
  const statuses = (p) => [...(p.status?.initContainerStatuses ?? []), ...(p.status?.containerStatuses ?? [])].map((s) => ({ container: s.name, ready: s.ready === true, restarts: s.restartCount ?? 0, state: s.state?.waiting ? "waiting" : s.state?.terminated ? "terminated" : s.state?.running ? "running" : "unknown", reason: reason(s.state?.waiting?.reason ?? s.state?.terminated?.reason), exitCode: s.state?.terminated?.exitCode ?? null }));
  // No annotations, env, command, events/messages, logs, URLs or content.
  return {
    pods: pods.map((p) => ({ name: p.metadata.name, phase: phase(p.status?.phase), containers: statuses(p) })),
    jobs: jobs.map((j) => ({ name: j.metadata.name, active: j.status?.active ?? 0, succeeded: j.status?.succeeded ?? 0, failed: j.status?.failed ?? 0, conditions: (j.status?.conditions ?? []).map((c) => ({ type: ["Complete", "Failed", "FailureTarget", "SuccessCriteriaMet", "Suspended"].includes(c.type) ? c.type : "unclassified", status: ["True", "False", "Unknown"].includes(c.status) ? c.status : "Unknown", reason: reason(c.reason) })) })),
    pvcs: pvcs.map((p) => ({ name: p.metadata.name, phase: phase(p.status?.phase), storageClass: p.spec?.storageClassName ?? "", requested: p.spec?.resources?.requests?.storage ?? "" })),
  };
}
export function check({ action, directory, architecture, confirmedStorageClass }, run = command, report = console.log) {
  privatePath(directory, true);
  const path = (name) => resolve(directory, name);
  for (const name of ["state.json", "values.json", "release-images.yaml"]) privatePath(path(name));
  const state = parseJson(readFileSync(path("state.json"), "utf8"), "private preparation state");
  const v = parseJson(readFileSync(path("values.json"), "utf8"), "saved values file"); const p = state.inputs;
  if (state.version !== 1 || state.materialHash !== createHash("sha256").update(json({ tenant: state.tenant, credentials: state.credentials, tls: state.tls, accounts: state.accounts })).digest("hex")) throw new Error("private preparation incomplete or changed; restore the original matching state.json and retry");
  if (v.fullnameOverride !== p.release || v.install?.tenant?.id !== state.tenant || v.gateway.publicUrl !== p.appUrl || v.postgres.mode !== p.database) throw new Error("values conflict with prepared release, tenant, origin or database ownership; preserve files and recover matching inputs before retrying");
  if (createHash("sha256").update(readFileSync(path("release-images.yaml"))).digest("hex") !== state.evidenceHash) throw new Error("image overlay changed; restore the saved verified overlay before retrying");
  let failures = 0;
  const result = (level, message, next = "") => { if (level === "FAIL") failures++; report(`${level}: ${message}${next && level !== "PASS" ? ` → ${next}` : ""}`); };
  const retry = `node "$CHART/examples/operator.mjs" ${action} --prepared "$PREPARED" --architecture ${architecture ?? "amd64|arm64"}${confirmedStorageClass ? ' --confirmed-storage-class "$STORAGE_CLASS"' : ""}`;
  const kube = (...args) => run("kubectl", ["--context", p.context, "--namespace", p.namespace, "--request-timeout=15s", ...args]);
  const get = (kind, name, extra = []) => { const r = kube("get", kind, ...(name ? [name] : []), ...extra, "-o", "json"); try { return r.ok ? JSON.parse(r.text) : null; } catch { return null; } };
  if (action !== "diagnose") report(`Context: ${p.context}; namespace: ${p.namespace}; release: ${p.release}; exposure: ${v.gateway.publicUrl}; PostgreSQL: ${v.postgres.mode}; identity: ${v.keycloak?.enabled ? "bundled" : "external"}.`);
  if (action === "secrets") {
    privatePath(path("secrets.json"));
    const saved = readFileSync(path("secrets.json"), "utf8");
    if (saved !== json(derive(state).secrets)) throw new Error("prepared Secrets conflict with original recovery material; rerun preparation with its original inputs before retrying secrets");
    const items = parseJson(saved, "protected Secret list").items;
    const missing = [];
    for (const item of items) {
      if (item.kind !== "Secret" || item.metadata.namespace !== p.namespace) throw new Error("prepared Secret namespace/kind mismatch; preserve files and retry preparation");
      const read = kube("get", "secret", item.metadata.name, "--ignore-not-found=true", "-o", "json");
      if (!read.ok) throw new Error(`cannot inspect Secret ${item.metadata.name}; obtain get/create Secrets in the selected namespace, then retry secrets`);
      if (!read.text.trim()) { missing.push(item); continue; }
      const existing = parseJson(read.text, "Secret API response");
      if (Object.entries(item.stringData).some(([key, value]) => existing.data?.[key] !== Buffer.from(value).toString("base64"))) throw new Error(`Secret ${item.metadata.name} conflicts with saved recovery material; stop and recover the original matching preparation/Secret. No Secrets were written`);
    }
    if (missing.length) {
      const created = run("kubectl", ["--context", p.context, "--namespace", p.namespace, "--request-timeout=15s", "create", "-f", "-"], JSON.stringify({ apiVersion: "v1", kind: "List", items: missing }));
      if (!created.ok) throw new Error("Secret creation incomplete or refused; credentials preserved. Retry secrets with the same prepared directory (existing matching Secrets are reused)");
    }
    report(`PASS: ${items.length} prepared Secrets present with matching original bytes; no existing credential replaced.`);
    return 0;
  }
  if (action === "verify") {
    const jobs = get("jobs", null, ["-l", `app.kubernetes.io/instance=${p.release},app.kubernetes.io/component=install`]);
    const latest = jobs?.items.sort((a, b) => Number(a.metadata.name.split("-").at(-1)) - Number(b.metadata.name.split("-").at(-1))).at(-1);
    result(latest?.status?.conditions?.some((c) => c.type === "Complete" && c.status === "True") ? "PASS" : "FAIL", `installation Job ${latest?.metadata.name ?? "missing"}`, "use diagnose to identify database-bootstrap, database-preflight, migrate or tenant; correct that stage and reapply the same release");
    const ready = kube("exec", `deployment/${p.release}`, "-c", "gateway", "--", "node", "-e", "fetch('http://127.0.0.1:8120/readyz',{signal:AbortSignal.timeout(5000)}).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))");
    result(ready.ok ? "PASS" : "FAIL", "private gateway readiness", "obtain pods/exec permission and use diagnose on failure");
    report("Readiness does not prove login or client delivery. Sign in, run client context/recall, and inspect its Session/Context sources separately.");
    report(`Retry: ${retry}`);
    return failures;
  }
  if (action === "diagnose") {
    const pods = get("pods", null, ["-l", `app.kubernetes.io/instance=${p.release}`]);
    const jobs = get("jobs", null, ["-l", `app.kubernetes.io/instance=${p.release}`]);
    const claims = get("pvc");
    if (!pods || !jobs || !claims) throw new Error("diagnostics need namespaced list pods/jobs/pvc; ask the namespace administrator for these permissions and retry diagnose");
    report(JSON.stringify({ installation: { context: p.context, namespace: p.namespace, release: p.release, origin: p.appUrl, database: p.database, identity: p.identity }, ...diagnostic(pods.items, jobs.items, claims.items.filter((x) => x.metadata.name === `${p.release}-pg-data` || x.metadata.name === v.postgres.bundled?.existingClaim || x.metadata.labels?.["app.kubernetes.io/instance"] === p.release)) }, null, 2));
    return 0;
  }
  const helmVersion = run("helm", ["version", "--short"]);
  result(helmVersion.ok ? helmVersion.text.startsWith("v4.2.3") ? "PASS" : "WARN" : "FAIL", helmVersion.ok ? `Helm ${helmVersion.text.trim()}; qualified recipe uses 4.2.3` : "Helm unavailable", helmVersion.ok ? "" : "install Helm 4.2.3, then retry");
  const renderArgs = ["template", p.release, chart, "--namespace", p.namespace, "-f", path("values.json"), "-f", path("release-images.yaml")];
  if (v.postgres.mode === "cnpg") renderArgs.push("--api-versions", "postgresql.cnpg.io/v1");
  if (v.postgres.backup?.enabled) renderArgs.push("--api-versions", "barmancloud.cnpg.io/v1");
  if (v.route?.enabled) renderArgs.push("--api-versions", "route.openshift.io/v1");
  const render = run("helm", renderArgs);
  result(render.ok ? "PASS" : "FAIL", render.ok ? "offline chart render and schema validation" : "offline chart render refused", render.ok ? "" : 'run helm template with the saved values and overlay to see the configuration error; correct values, then retry');
  if (!render.ok) return failures;
  const budget = requests(v);
  report(`Reservations: steady ${budget.steady.cpu}m CPU / ${budget.steady.memory}Mi; during install ${budget.installation.cpu}m / ${budget.installation.memory}Mi and ${budget.pods} pods. Cluster overhead, surges and external services excluded; no measured minimum.`);
  result(["amd64", "arm64"].includes(architecture) ? "PASS" : "FAIL", "image architecture input", architecture ? "" : "obtain node architecture from cluster administrator; supply --architecture amd64 or arm64; verified images cover both");
  if (action === "offline") return failures;
  if (action === "database-probe") {
    if (failures) return failures;
    if (v.postgres.mode !== "external") throw new Error("database-probe is for already provisioned external PostgreSQL; bundled/CNPG bootstrap and preflight run in the normal ordered install Job");
    const rendered = run("helm", [...renderArgs, "--show-only", "templates/install-job.yaml", "--show-only", "templates/database-roles.yaml"]);
    if (!rendered.ok) throw new Error("database probe render refused; run offline checks and retry database-probe");
    // kubectl's native parser preserves the chart contract; no second YAML runtime.
    const parsed = run("kubectl", ["--context", p.context, "--namespace", p.namespace, "--request-timeout=15s", "create", "--dry-run=client", "--validate=false", "-f", "-", "-o", "json"], rendered.text);
    if (!parsed.ok) throw new Error("database probe conversion refused; obtain namespaced API discovery/Job permissions and retry database-probe");
    const documents = JSON.parse(parsed.text).items;
    const job = documents?.find((x) => x.kind === "Job"); const roles = documents?.find((x) => x.kind === "ConfigMap");
    if (!job || !roles) throw new Error("database probe lacks the chart's install Job or role contract; stop and use matching chart bytes");
    const container = job.spec.template.spec.initContainers.find((x) => x.name === "database-preflight");
    if (!container) throw new Error("database preflight container absent from chart");
    const name = `${p.release}-db-probe-${Date.now().toString(36)}`;
    roles.metadata = { name: `${name}-roles`, namespace: p.namespace };
    container.command = ["/usr/local/bin/synveda-container", "database-preflight"]; delete container.args;
    const pod = job.spec.template.spec;
    delete pod.initContainers; pod.containers = [container]; pod.serviceAccountName = "default";
    const mounts = new Set(container.volumeMounts.map((x) => x.name));
    pod.volumes = pod.volumes.filter((x) => mounts.has(x.name));
    pod.volumes.find((x) => x.name === "database-roles").configMap.name = roles.metadata.name;
    job.metadata = { name, namespace: p.namespace, labels: { "app.kubernetes.io/instance": p.release, "app.kubernetes.io/component": "database-probe" } };
    job.spec.activeDeadlineSeconds = 120; job.spec.backoffLimit = 0; job.spec.ttlSecondsAfterFinished = 3600;
    const created = run("kubectl", ["--context", p.context, "--namespace", p.namespace, "--request-timeout=15s", "create", "-f", "-"], JSON.stringify({ apiVersion: "v1", kind: "List", items: [roles, job] }));
    if (!created.ok) throw new Error(`database probe create incomplete/refused; inspect Job ${name} and ConfigMap ${name}-roles in the selected namespace, correct RBAC/quota/Secrets and retry database-probe`);
    report(`CREATED temporary Job ${name} (100m/128Mi default request). Kubernetes mutation; only the existing ordinary-role database verifier runs, no migration or administrator credential.`);
    report(`Require success before install: kubectl --context "${p.context}" -n "${p.namespace}" wait job/${name} --for=condition=Complete --timeout=130s`);
    report(`If refused: inspect private logs with kubectl --context "${p.context}" -n "${p.namespace}" logs job/${name} -c database-preflight --tail=30; give the DBA the content-free failure. Delete only this temporary Job/ConfigMap after inspection, then retry database-probe.`);
    return 0;
  }
  const client = run("kubectl", ["version", "--client", "-o", "json"]);
  const clientVersion = client.ok ? JSON.parse(client.text).clientVersion?.gitVersion ?? "unreported" : "unavailable";
  result(client.ok ? "PASS" : "FAIL", `kubectl ${clientVersion}; qualification used 1.36.1`, client.ok ? "" : "install kubectl compatible with the cluster");
  if (!client.ok) return failures;
  const versions = kube("version", "-o", "json");
  if (!versions.ok) {
    result("FAIL", "selected Kubernetes API is unavailable", "correct context credentials, API reachability or access with the cluster administrator; retry preflight before applying resources");
    report(`Retry: ${retry}`);
    return failures;
  }
  const serverVersion = versions.ok ? JSON.parse(versions.text).serverVersion?.gitVersion ?? "unreported" : "unavailable";
  const minor = (version) => /^v1\.([0-9]+)\./.exec(version)?.[1];
  result(minor(clientVersion) && minor(serverVersion) ? Math.abs(Number(minor(clientVersion)) - Number(minor(serverVersion))) <= 1 ? "PASS" : "FAIL" : "WARN", `Kubernetes ${serverVersion}; client/server version skew`, "use a kubectl minor within one of the server; distribution/ingress/storage/CNI still need named qualification");
  const current = run("kubectl", ["config", "current-context"]);
  result(current.ok && current.text.trim() === p.context ? "PASS" : "FAIL", "selected context matches prepared context", "select the reviewed context explicitly; commands also pin --context");
  const ns = get("namespace", p.namespace);
  result(ns ? "PASS" : "WARN", "namespace inspection", ns ? "" : `cluster-wide namespace get unavailable; administrator must confirm namespace ${p.namespace} exists; namespaced RBAC is checked below`);
  const resources = ["deployments.apps", "jobs.batch", "services", "configmaps", "secrets", "serviceaccounts", ...(v.postgres.mode === "bundled" || v.keycloak?.enabled ? ["statefulsets.apps"] : []), ...(v.postgres.mode !== "external" || v.tei?.enabled ? ["persistentvolumeclaims"] : []), ...(v.ingress?.enabled ? ["ingresses.networking.k8s.io"] : []), ...(v.networkPolicy?.enabled ? ["networkpolicies.networking.k8s.io"] : []), ...(v.postgres.mode === "cnpg" ? ["clusters.postgresql.cnpg.io"] : []), ...(v.postgres.backup?.enabled ? ["scheduledbackups.postgresql.cnpg.io"] : []), ...(v.route?.enabled ? ["routes.route.openshift.io", "roles.rbac.authorization.k8s.io", "rolebindings.rbac.authorization.k8s.io"] : [])];
  for (const resource of resources) for (const verb of ["get", "list", "watch", "create", "patch", "update", "delete"]) {
    const allowed = kube("auth", "can-i", verb, resource);
    if (!allowed.ok || allowed.text.trim() !== "yes") result("FAIL", `RBAC ${verb} ${resource}`, `ask namespace administrator for this Helm permission in ${p.namespace}; retry preflight`);
  }
  for (const [verb, resource, subresource] of [["get", "pods"], ["list", "pods"], ["watch", "pods"], ["create", "pods"], ["delete", "pods"], ["get", "pods", "log"], ["create", "pods", "exec"], ...(p.route === "local" ? [["create", "pods", "portforward"]] : [])]) {
    const allowed = kube("auth", "can-i", verb, resource, ...(subresource ? [`--subresource=${subresource}`] : []));
    result(allowed.ok && allowed.text.trim() === "yes" ? "PASS" : "FAIL", `RBAC ${verb} ${resource}${subresource ? `/${subresource}` : ""}`, allowed.ok && allowed.text.trim() === "yes" ? "" : `administrator must grant this namespaced verification/forward permission; retry preflight`);
  }
  const quota = get("resourcequotas"); const limits = get("limitranges");
  if (!quota || !limits) result("FAIL", "quota/LimitRange inspection unavailable", "ask namespace administrator for get/list resourcequotas and limitranges and installation headroom; retry preflight");
  const rows = containers(v);
  const claims = storageRequests(v);
  for (const failure of limitFailures(rows, limits?.items ?? [], claims)) result("FAIL", failure, "review ordinary resources/storage in saved Helm values with namespace administrator; retry preflight");
  for (const q of quota?.items ?? []) {
    const hard = q.status?.hard ?? {}; const used = q.status?.used ?? {};
    for (const [key, need, memory] of [["requests.cpu", budget.installation.cpu, false], ["requests.memory", budget.installation.memory, true], ["pods", budget.pods, null], ["count/pods", budget.pods, null], ["count/jobs.batch", 1, null]]) {
      if (hard[key] !== undefined && ((memory === null ? Number(hard[key]) : quantity(hard[key], memory)) - (memory === null ? Number(used[key] ?? 0) : quantity(used[key] ?? 0, memory))) < need) result("FAIL", `quota ${q.metadata.name} lacks ${key} install/replacement headroom`, "administrator must reserve the printed requests, replacement pods and retained claims; rerun preflight (existing release usage is conservatively counted)");
    }
    report(`Quota ${q.metadata.name}: ${JSON.stringify({ hard, used })}; also reserve limits and storage.`);
    for (const key of ["cpu", "memory"]) if (hard[`limits.${key}`] !== undefined) {
      const needed = rows.reduce((total, row) => {
        const value = row.limits[key] ?? (limits?.items.flatMap((l) => l.spec?.limits ?? []).find((r) => r.type === "Container" && r.default?.[key])?.default?.[key]);
        return value === null || value === undefined ? NaN : total + (typeof value === "number" ? value : quantity(value, key === "memory"));
      }, 0);
      result(Number.isFinite(needed) && quantity(hard[`limits.${key}`], key === "memory") - quantity(used[`limits.${key}`] ?? "0", key === "memory") >= needed ? "PASS" : "FAIL", `quota ${q.metadata.name} limits.${key} admission/headroom`, "supply required per-container limits and reserve install/replacement headroom; retry preflight");
    }
    for (const [key, value] of Object.entries(hard)) {
      if (!["persistentvolumeclaims", "count/persistentvolumeclaims", "requests.storage"].includes(key) && !/\.storageclass\.storage\.k8s\.io\/(requests.storage|persistentvolumeclaims)$/.test(key)) continue;
      const className = key.includes(".storageclass.") ? key.split(".storageclass.")[0] : null;
      const applicable = className ? claims.filter((x) => x.storageClass === className) : claims;
      if (className && claims.some((x) => !x.storageClass)) result("WARN", `quota ${q.metadata.name} class-specific storage selection unknown`, "administrator must confirm the default class and its quota headroom; explicitly select a named class in saved values and retry preflight");
      const storage = key.endsWith("requests.storage");
      const need = storage ? applicable.reduce((total, x) => total + x.memory, 0) : applicable.length;
      const available = storage ? quantity(value, true) - quantity(used[key] ?? "0", true) : Number(value) - Number(used[key] ?? 0);
      result(available >= need ? "PASS" : "FAIL", `quota ${q.metadata.name} ${key} storage headroom`, available >= need ? "" : "storage owner must reserve the selected claims/capacity; preserve retained claims and retry preflight");
    }
  }
  for (const l of limits?.items ?? []) report(`LimitRange ${l.metadata.name}: ${JSON.stringify(l.spec?.limits ?? [])}; compare per-container bounds/default CPU limits with rendered pods.`);
  const originalSecrets = new Map(derive(state).secrets.items.map((x) => [x.metadata.name, x]));
  for (const [name, keys] of secretKeys(v)) {
    const secret = get("secret", name);
    const original = originalSecrets.get(name);
    if (original && secret) result(Object.entries(original.stringData).every(([key, value]) => secret.data?.[key] === Buffer.from(value).toString("base64")) ? "PASS" : "FAIL", `original recovery material for Secret ${name}`, "restore the matching original Secret/preparation files; do not replace keys to make install pass; retry preflight");
    for (const key of keys) result(secret?.data?.[key] ? "PASS" : "FAIL", `Secret ${name} key ${key}`, secret?.data?.[key] ? "" : `provide the original matching nonempty Secret/key through protected files or secret manager in ${p.namespace}; retry preflight`);
    const databaseRole = ["gateway", "worker", "migrator"].find((role) => name === (role === "migrator" ? v.postgres.external?.migratorExistingSecret ?? v.postgres.bundled?.migratorExistingSecret : v[role].databaseExistingSecret));
    if (databaseRole && v.postgres.mode === "external") {
      try {
        const key = databaseRole === "migrator" ? v.postgres.external.migratorUrlSecretKey ?? "DATABASE_URL" : v[databaseRole].databaseUrlSecretKey ?? "DATABASE_URL";
        const uri = new URL(Buffer.from(secret.data[key], "base64").toString("utf8").trimEnd());
        const ok = ["postgres:", "postgresql:"].includes(uri.protocol) && decodeURIComponent(uri.username) === v.postgres.external.roles[databaseRole] && uri.hostname === v.postgres.external.host && Number(uri.port || 5432) === v.postgres.external.port && decodeURIComponent(uri.pathname.slice(1)) === v.postgres.external.database && uri.searchParams.get("sslmode") === "verify-full" && uri.searchParams.get("sslrootcert") === "/run/secrets/synveda-postgres/ca.crt";
        result(ok ? "PASS" : "FAIL", `${databaseRole} credential endpoint/role/TLS declaration`, "DBA must correct the complete protected URL to its declared role/host/database/verify-full CA path; retry preflight");
      } catch { result("FAIL", `${databaseRole} protected URL format`, "supply its valid complete SQLx URL from the DBA through protected files; retry preflight"); }
    }
    const caKeys = [...new Set([name === v.postgres.external?.caExistingSecret ? v.postgres.external.caSecretKey ?? "ca.crt" : null, name === v.postgres.bundled?.tlsExistingSecret ? "ca.crt" : null, name === v.keycloak?.databaseCaExistingSecret ? v.keycloak.databaseCaSecretKey ?? "ca.crt" : null, name === v.oidc.caExistingSecret ? v.oidc.caSecretKey ?? "ca.crt" : null].filter(Boolean))];
    for (const key of caKeys) if (secret?.data?.[key]) {
      try { const ca = new X509Certificate(Buffer.from(secret.data[key], "base64")); result(ca.ca && Date.parse(ca.validTo) > Date.now() ? "PASS" : "FAIL", `CA format/expiry ${name}`, "supply the provider's valid CA PEM, then retry; its trust in pod connections is established by the install preflight"); } catch { result("FAIL", `CA PEM ${name}`, "supply the provider's issuing root certificate, then retry"); }
    }
    if (name === v.postgres.bundled?.tlsExistingSecret && secret?.data?.["tls.crt"] && secret?.data?.["ca.crt"]) {
      try { const cert = new X509Certificate(Buffer.from(secret.data["tls.crt"], "base64")); const ca = new X509Certificate(Buffer.from(secret.data["ca.crt"], "base64")); result(cert.checkHost(`${p.release}-pg-rw`) && cert.verify(ca.publicKey) ? "PASS" : "FAIL", "bundled database SAN/CA signature", "restore the matching database certificate and CA from recovery; retry preflight"); } catch { result("FAIL", "bundled database certificate", "restore original TLS files and retry preflight"); }
    }
    if (name === v.oidc.existingSecret && secret?.data?.[v.oidc.secretKey ?? "SYNVEDA_OIDC_ISSUERS"]) {
      try {
        const issuers = JSON.parse(Buffer.from(secret.data[v.oidc.secretKey ?? "SYNVEDA_OIDC_ISSUERS"], "base64"));
        const expected = JSON.parse(derive(state).secrets.items.find((x) => x.metadata.name === v.oidc.existingSecret).stringData.SYNVEDA_OIDC_ISSUERS);
        const ok = JSON.stringify(issuers) === JSON.stringify(expected) && issuers[0].tenant?.static?.tenant_id === v.install.tenant.id;
        result(ok ? "PASS" : "FAIL", "issuer/static tenant binding", ok ? "" : "restore the original prepared issuer Secret; do not change issuer for retained identities; retry preflight");
      } catch { result("FAIL", "issuer JSON malformed", "restore the matching issuer file and retry preflight"); }
    }
    const hosts = [...(v.ingress?.tls ?? []), ...(v.keycloak?.ingress?.tls ?? [])].filter((x) => x.secretName === name).flatMap((x) => x.hosts);
    if (hosts.length && secret?.data?.["tls.crt"]) {
      try {
        const cert = new X509Certificate(Buffer.from(secret.data["tls.crt"], "base64"));
        result(hosts.every((h) => cert.checkHost(h)) && Date.parse(cert.validTo) > Date.now() ? "PASS" : "FAIL", `certificate names/expiry for ${name}`, "certificate owner must supply a valid chain for the configured hosts; retry preflight");
      } catch { result("FAIL", `invalid certificate ${name}`, "replace with the certificate owner's correct PEM chain; retry preflight"); }
    }
  }
  for (const [label, storage] of [...(v.postgres.mode !== "external" ? [["database", v.postgres.mode === "bundled" ? v.postgres.bundled : v.postgres.storage]] : []), ...(v.tei?.enabled ? [["TEI cache", v.tei.cache ?? {}]] : [])]) {
    const claim = storage.existingClaim ? get("pvc", storage.existingClaim) : null;
    const classes = get("storageclasses");
    const selected = storage.storageClass ? classes?.items.find((x) => x.metadata.name === storage.storageClass) : classes?.items.find((x) => x.metadata.annotations?.["storageclass.kubernetes.io/is-default-class"] === "true");
    if (!classes) {
      const confirmed = confirmedStorageClass && storage.storageClass === confirmedStorageClass;
      result(claim?.status?.phase === "Bound" || confirmed ? "WARN" : "FAIL", "StorageClass listing unavailable under namespaced RBAC", `${label}: ${claim?.status?.phase === "Bound" ? "existing claim is Bound; administrator still supplies provisioner/UID and recovery ownership" : confirmed ? `administrator confirmed class ${confirmedStorageClass}; the install still needs to establish provisioner/fsync/UID behavior` : `administrator must confirm class ${storage.storageClass || "(select an explicit class)"}, provisioner, fsync/UID support, volumeBindingMode and quota; pass --confirmed-storage-class only for that named class and rerun`}`);
    }
    else if (!selected && !storage.existingClaim) result("FAIL", `no selected/default StorageClass for ${label}`, "administrator must supply a usable class; deliberately adjust saved Helm storage before install (new preparation selects --storage-class); preserve original credentials and retry preflight");
    else if (selected) report(`PASS: storage ${selected.metadata.name}; binding=${selected.volumeBindingMode ?? "Immediate"}; provisioner=${selected.provisioner}.`);
    if (storage.existingClaim) {
      const c = classes?.items.find((x) => x.metadata.name === claim?.spec?.storageClassName);
      const verdict = storageResult(claim, c); result(verdict.level, verdict.message, "inspect kubectl describe pvc; retry preflight after correcting storage");
    }
  }
  for (const group of [ ...(v.postgres.mode === "cnpg" ? ["postgresql.cnpg.io/v1"] : []), ...(v.postgres.backup?.enabled ? ["barmancloud.cnpg.io/v1"] : []), ...(v.route?.enabled ? ["route.openshift.io/v1"] : []) ]) {
    const apis = kube("api-versions"); result(apis.ok && apis.text.split(/\s+/).includes(group) ? "PASS" : "FAIL", `selected API ${group}`, "administrator must provide its existing controller/API and namespaced access; retry preflight");
  }
  report("Not established by preflight: registry pull/pod connectivity, enforcing CNI, DNS/HTTPS from pods, CA acceptance by SQLx/OIDC, login or Capture. The install Job runs the mandatory database authority verifier; watch its named stage.");
  report(`Retry: ${retry}`);
  return failures;
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [action, ...args] = process.argv.slice(2); const o = {};
    if (!["offline", "secrets", "preflight", "database-probe", "verify", "diagnose"].includes(action)) throw new Error("usage: operator.mjs offline|secrets|preflight|database-probe|verify|diagnose --prepared /absolute/directory --architecture amd64|arm64 [--confirmed-storage-class NAME]");
    for (let i = 0; i < args.length; i += 2) {
      if (!["--prepared", "--architecture", "--confirmed-storage-class"].includes(args[i]) || !args[i + 1]) throw new Error("unknown/incomplete operator option");
      o[args[i].slice(2)] = args[i + 1];
    }
    process.exitCode = check({ action, directory: resolve(o.prepared), architecture: o.architecture, confirmedStorageClass: o["confirmed-storage-class"] }) ? 1 : 0;
  } catch (error) { console.error(error.message); process.exitCode = 78; }
}

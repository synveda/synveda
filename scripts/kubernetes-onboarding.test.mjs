import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { prepare, imagePins, json, options } from "../deploy/helm/synveda/examples/prepare.mjs";
import { check, containers, diagnostic, limitFailures, quantity, requests, secretKeys, storageRequests, storageResult } from "../deploy/helm/synveda/examples/operator.mjs";

const chart = resolve("deploy/helm/synveda");
function fixture(t) {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), "synveda-human-"))); chmodSync(directory, 0o700);
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const images = join(directory, "images.yaml");
  writeFileSync(images, `image:\n  repository: ghcr.io/synveda/product\n  tag: ""\n  digest: sha256:${"a".repeat(64)}\npostgres:\n  bundled:\n    image: ghcr.io/synveda/postgres@sha256:${"b".repeat(64)}\nkeycloak:\n  image:\n    repository: ghcr.io/synveda/keycloak\n    digest: sha256:${"c".repeat(64)}\n`, { mode: 0o600 });
  return { directory, images, o: { output: join(directory, "private"), images, context: "disposable-example", release: "northstar", namespace: "human-install", "storage-class": "selected-fast", "storage-size": "37Gi", "app-port": "9130", "identity-port": "9081" } };
}
const read = (f, name) => readFileSync(join(f.o.output, name), "utf8");
function objects(yaml) {
  const parsed = spawnSync("ruby", ["-rjson", "-ryaml", "-e", "puts JSON.generate(YAML.load_stream(STDIN.read).compact)"], { input: yaml, encoding: "utf8" });
  assert.equal(parsed.status, 0, parsed.stderr);
  return JSON.parse(parsed.stdout);
}
function kubeFixture(f, overrides = {}) {
  const secrets = JSON.parse(read(f, "secrets.json")).items;
  return (binary, args, input) => {
    if (binary === "helm") { const r = spawnSync(binary, args, { encoding: "utf8" }); return { ok: r.status === 0, text: r.stdout }; }
    if (args.includes("version")) return { ok: true, text: JSON.stringify({ clientVersion: { gitVersion: "v1.36.1" }, serverVersion: { gitVersion: "v1.36.1" } }) };
    if (args.includes("current-context")) return { ok: true, text: f.o.context };
    if (args.includes("can-i")) return { ok: true, text: "yes" };
    if (args.includes("--dry-run=client")) return { ok: true, text: JSON.stringify({ items: objects(input) }) };
    const kind = args[args.indexOf("get") + 1];
    if (kind in overrides) return overrides[kind](args, input);
    if (kind === "secret") {
      const item = secrets.find((s) => s.metadata.name === args[args.indexOf("secret") + 1]);
      return { ok: Boolean(item), text: item ? JSON.stringify({ data: Object.fromEntries(Object.entries(item.stringData).map(([k, v]) => [k, Buffer.from(v).toString("base64")])) }) : "" };
    }
    if (kind === "storageclasses") return { ok: false, text: "" };
    if (kind === "namespace") return { ok: false, text: "" };
    return { ok: true, text: '{"items":[]}' };
  };
}
function render(f, extra = []) {
  return spawnSync("helm", ["template", f.o.release, chart, "-n", f.o.namespace, "-f", join(f.o.output, "values.json"), "-f", f.images, ...extra], { encoding: "utf8" });
}

test("native preparation renders custom names, ports, exact PVC and SANs without Docker", (t) => {
  const f = fixture(t); const state = prepare(f.o); const r = render(f);
  assert.equal(r.status, 0, r.stderr);
  assert.match(state.tenant, /^[a-f0-9-]{14}7/);
  assert.match(r.stdout, /name: northstar-pg-data/);
  const pvc = r.stdout.split(/^---$/m).find((x) => x.includes("kind: PersistentVolumeClaim"));
  assert.match(pvc, /storageClassName: "selected-fast"/); assert.match(pvc, /storage: 37Gi/);
  assert.match(r.stdout, /http:\/\/localhost:9130\/auth\/callback/);
  const secrets = JSON.parse(read(f, "secrets.json")).items;
  assert.ok(secrets.every((x) => x.metadata.namespace === "human-install"));
  const issuer = JSON.parse(secrets.find((x) => x.metadata.name === "northstar-oidc").stringData.SYNVEDA_OIDC_ISSUERS)[0];
  assert.equal(issuer.issuer, "http://localhost:9081/realms/synveda");
  assert.match(issuer.discovery_url, /northstar-keycloak-http/);
  assert.ok(!r.stdout.includes(state.credentials.kms));
  const notes = spawnSync("helm", ["install", "northstar", chart, "--dry-run=client", "-n", f.o.namespace, "-f", join(f.o.output, "values.json"), "-f", f.images], { encoding: "utf8" });
  assert.equal(notes.status, 0, notes.stderr);
  assert.match(notes.stdout, /service\/northstar 9130:8120/);
  assert.match(notes.stdout, /service\/northstar-keycloak-http 9081:80/);
});

test("retry and interrupted output preserve all private bytes and recovery identity", (t) => {
  const f = fixture(t); prepare(f.o);
  const saved = Object.fromEntries(["state.json", "secrets.json", "values.json", "release-images.yaml"].map((n) => [n, read(f, n)]));
  prepare(f.o);
  for (const [n, bytes] of Object.entries(saved)) assert.equal(read(f, n), bytes);
  rmSync(join(f.o.output, "secrets.json")); rmSync(join(f.o.output, "values.json"));
  prepare(f.o);
  for (const [n, bytes] of Object.entries(saved)) assert.equal(read(f, n), bytes);
  for (const [key, value] of Object.entries({ release: "other", namespace: "other", "identity-port": "9091", context: "other" })) assert.throws(() => prepare({ ...f.o, [key]: value }), /conflicts/);
  assert.equal(read(f, "state.json"), saved["state.json"]);
});

test("partial and conflicting private preparation stops without rotating keys", (t) => {
  const f = fixture(t); prepare(f.o); const state = read(f, "state.json"); const secret = read(f, "secrets.json");
  writeFileSync(join(f.o.output, "secrets.json"), "changed", { mode: 0o600 });
  assert.throws(() => prepare(f.o), /conflicts/); assert.equal(read(f, "state.json"), state);
  writeFileSync(join(f.o.output, "secrets.json"), secret);
  rmSync(join(f.o.output, "state.json")); assert.throws(() => prepare(f.o), /outputs exist without state/);
  writeFileSync(join(f.o.output, "state.json"), state, { mode: 0o600 });
  chmodSync(join(f.o.output, "state.json"), 0o644); assert.throws(() => prepare(f.o), /permissions/);
  chmodSync(join(f.o.output, "state.json"), 0o600);
  writeFileSync(f.images, 'image:\n  tag: latest\n'); assert.throws(() => prepare(f.o), /digest overlay/);
});

test("bundled silently ignored CNPG storage settings now refuse", () => {
  for (const setting of ["postgres.storage.size=37Gi", "postgres.storage.storageClass=selected-fast"]) {
    const r = spawnSync("helm", ["template", "human", chart, "-f", `${chart}/examples/bundled-database.json`, "--set", setting], { encoding: "utf8" });
    assert.notEqual(r.status, 0); assert.match(r.stderr, /apply only/);
  }
});

test("all four customer ownership combinations render without CNPG APIs", () => {
  for (const database of ["bundled", "external"]) for (const identity of ["external", "bundled"]) {
    const args = ["template", "synveda", chart, "-f", `${chart}/examples/${database}-database.json`];
    if (identity === "bundled") args.push("-f", `${chart}/examples/bundled-identity.json`);
    const r = spawnSync("helm", args, { encoding: "utf8" }); assert.equal(r.status, 0, r.stderr);
    assert.ok(!r.stdout.includes("kind: Cluster\n")); assert.equal(r.stdout.includes("name: keycloak\n"), identity === "bundled");
  }
});

test("native preparation covers all six independent database and identity ownership recipes", (t) => {
  for (const database of ["bundled", "external", "cnpg"]) for (const identity of ["bundled", "external"]) {
    const f = fixture(t);
    if (database === "cnpg") writeFileSync(f.images, readFileSync(f.images, "utf8").replace("\npostgres:\n", `\npostgres:\n  image: ghcr.io/synveda/cnpg-postgres@sha256:${"d".repeat(64)}\n`));
    const valuesFile = join(f.directory, "services.json");
    const values = database === "external" ? JSON.parse(readFileSync(`${chart}/examples/external-database.json`, "utf8")) : {};
    if (database === "external") {
      values.postgres.external.host = "db.team.test";
      if (identity === "bundled") values.keycloak = { database: { hostname: "identity-db.team.test", existingSecret: "identity-db" }, databaseCaExistingSecret: "identity-db-ca" };
    }
    writeFileSync(valuesFile, json(values));
    const issuerFile = join(f.directory, "issuers.json");
    const tenant = "019a0c10-0010-7111-8111-111111111111";
    writeFileSync(issuerFile, json([{ issuer: "https://login.team.test/realms/team", client_id: "synveda", audience: "synveda-api", groups_claim: "groups", algorithms: ["RS256"], tenant: { static: { tenant_id: tenant } } }]), { mode: 0o600 });
    const o = { output: f.o.output, images: f.images, context: f.o.context, release: f.o.release, namespace: f.o.namespace, route: database === "cnpg" ? "cnpg" : "shared", database, identity, "app-url": "https://memory.team.test", "ingress-class": "team-ingress", "app-tls-secret": "application-tls", ...(database === "external" ? { values: valuesFile } : { "storage-class": "selected-fast", "storage-size": "37Gi" }), ...(identity === "bundled" ? { "identity-url": "https://login.team.test", "identity-tls-secret": "identity-tls", "proxy-cidrs": "192.0.2.0/24", "admin-user": "chosen-private-admin" } : { "issuer-file": issuerFile }) };
    const state = prepare(o); const rendered = render(f, database === "cnpg" ? ["--api-versions", "postgresql.cnpg.io/v1"] : []); assert.equal(rendered.status, 0, `${database}/${identity}: ${rendered.stderr}`);
    const secrets = JSON.parse(read(f, "secrets.json")).items;
    const keycloakAdmin = secrets.find((x) => x.metadata.name === "northstar-keycloak-admin");
    assert.equal(Boolean(keycloakAdmin), identity === "bundled");
    assert.equal(state.accounts, null); assert.ok(!secrets.some((x) => x.metadata.name.endsWith("evaluation-accounts")));
    assert.equal(Boolean(state.credentials.postgres), database === "bundled");
    if (database === "external") { assert.equal(Boolean(state.credentials.gateway), false); assert.ok(!secrets.some((x) => x.metadata.name.endsWith("gateway-db"))); }
    if (identity === "external") assert.equal(state.tenant, tenant);
    if (database === "cnpg") { assert.ok(!secrets.some((x) => x.metadata.name.endsWith("pg-ca"))); assert.match(rendered.stdout, /storageClass: selected-fast/); assert.match(rendered.stdout, /size: 37Gi/); }
    const saved = read(f, "state.json"); prepare(o); assert.equal(read(f, "state.json"), saved);
  }
});

test("storage classification handles no default class and WaitForFirstConsumer", (t) => {
  assert.equal(storageResult({ status: { phase: "Pending" } }, { volumeBindingMode: "WaitForFirstConsumer" }).level, "PASS");
  assert.equal(storageResult({ status: { phase: "Pending" } }, { volumeBindingMode: "Immediate" }).level, "FAIL");
  const f = fixture(t); prepare(f.o);
  const messages = [];
  const run = (binary, args) => {
    if (binary === "helm") return { ok: true, text: "v4.2.3" };
    if (args[0] === "version") return { ok: true, text: "{}" };
    if (args[0] === "config") return { ok: true, text: f.o.context };
    if (args.includes("can-i")) return { ok: true, text: "yes" };
    if (args.includes("storageclasses")) return { ok: true, text: '{"items":[]}' };
    if (args.includes("secret")) return { ok: false, text: "" };
    return { ok: true, text: '{"items":[]}' };
  };
  const n = check({ action: "preflight", directory: f.o.output, architecture: "arm64" }, run, (x) => messages.push(x));
  assert.ok(n > 0); assert.match(messages.join("\n"), /no selected\/default StorageClass/); assert.match(messages.join("\n"), /Secret northstar-gateway-db key DATABASE_URL/);
  assert.ok(!messages.join("\n").includes(JSON.parse(read(f, "state.json")).credentials.kms));
});

test("source resource totals count installation Job once and provider ownership", () => {
  for (const [mode, bundledIdentity, cpu, memory] of [["bundled", true, 1600, 2688], ["bundled", false, 1100, 1408], ["external", true, 1350, 2176], ["external", false, 850, 896]]) {
    const v = JSON.parse(readFileSync(`${chart}/examples/${mode}-database.json`, "utf8")); v.keycloak = { enabled: bundledIdentity };
    assert.deepEqual(requests(v).installation, { cpu, memory });
    const r = spawnSync("helm", ["template", "synveda", chart, "-f", `${chart}/examples/${mode}-database.json`, ...(bundledIdentity ? ["-f", `${chart}/examples/bundled-identity.json`] : [])], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    const sum = objects(r.stdout).filter((x) => ["Deployment", "StatefulSet", "Job"].includes(x.kind)).reduce((total, x) => {
      const pod = x.spec.template.spec;
      const cpuOf = (c) => Number(String(c.resources?.requests?.cpu ?? "0").replace("m", "")) * (String(c.resources?.requests?.cpu ?? "0").endsWith("m") ? 1 : 1000);
      const memOf = (c) => Number(String(c.resources?.requests?.memory ?? "0Mi").replace(/Gi|Mi/, "")) * (String(c.resources?.requests?.memory ?? "0Mi").endsWith("Gi") ? 1024 : 1);
      const cpus = Math.max(pod.containers.reduce((s, c) => s + cpuOf(c), 0), ...(pod.initContainers ?? []).map(cpuOf));
      const memoryMi = Math.max(pod.containers.reduce((s, c) => s + memOf(c), 0), ...(pod.initContainers ?? []).map(memOf));
      return { cpu: total.cpu + cpus, memory: total.memory + memoryMi };
    }, { cpu: 0, memory: 0 });
    assert.deepEqual(sum, requests(v).installation, "reservation documentation must follow actual rendered Pod requests");
  }
});

test("namespaced installer can supply an explicit administrator-confirmed StorageClass", (t) => {
  const f = fixture(t); prepare(f.o); const messages = [];
  assert.equal(check({ action: "preflight", directory: f.o.output, architecture: "arm64", confirmedStorageClass: "selected-fast" }, kubeFixture(f), (s) => messages.push(s)), 0, messages.join("\n"));
  assert.match(messages.join("\n"), /StorageClass listing unavailable under namespaced RBAC/);
  assert.match(messages.join("\n"), /namespace inspection/);
  assert.match(messages.join("\n"), /Not established by preflight/);
});

test("wrong CA, issuer/audience and missing key refuse without disclosing credentials", (t) => {
  const f = fixture(t); prepare(f.o); const other = fixture(t); prepare(other.o);
  const secrets = JSON.parse(read(f, "secrets.json")).items;
  const tls = secrets.find((x) => x.metadata.name === "northstar-pg-ca");
  tls.stringData["ca.crt"] = JSON.parse(read(other, "state.json")).tls["ca.crt"];
  const issuer = secrets.find((x) => x.metadata.name === "northstar-oidc");
  const trust = JSON.parse(issuer.stringData.SYNVEDA_OIDC_ISSUERS); trust[0].audience = "wrong-api"; issuer.stringData.SYNVEDA_OIDC_ISSUERS = JSON.stringify(trust);
  delete secrets.find((x) => x.metadata.name === "northstar-worker-db").stringData.DATABASE_URL;
  const messages = [];
  const run = kubeFixture(f, { secret: (args) => {
    const item = secrets.find((x) => x.metadata.name === args[args.indexOf("secret") + 1]);
    return { ok: true, text: JSON.stringify({ data: Object.fromEntries(Object.entries(item?.stringData ?? {}).map(([k, v]) => [k, Buffer.from(v).toString("base64")])) }) };
  } });
  assert.ok(check({ action: "preflight", directory: f.o.output, architecture: "arm64", confirmedStorageClass: "selected-fast" }, run, (s) => messages.push(s)) >= 3);
  for (const reason of ["FAIL: bundled database SAN/CA signature", "FAIL: issuer/static tenant binding", "FAIL: Secret northstar-worker-db key DATABASE_URL"]) assert.ok(messages.some((s) => s.startsWith(reason)), reason);
  assert.ok(!messages.join("\n").includes(JSON.parse(read(f, "state.json")).credentials.worker));
});

test("Secret conflict is checked before any mutation; retry reuses original matching values", (t) => {
  const f = fixture(t); prepare(f.o); let created = 0;
  const secrets = JSON.parse(read(f, "secrets.json")).items;
  const run = (binary, args) => {
    if (args.includes("create")) { created++; return { ok: true, text: "" }; }
    const name = args[args.indexOf("secret") + 1];
    if (name !== secrets.at(-1).metadata.name) return { ok: true, text: "" };
    return { ok: true, text: '{"data":{"admin":"changed"}}' };
  };
  assert.throws(() => check({ action: "secrets", directory: f.o.output }, run, () => {}), /conflicts/);
  assert.equal(created, 0);
  assert.equal(check({ action: "secrets", directory: f.o.output }, kubeFixture(f), () => {}), 0);
});

test("Container limits and quota prerequisites retain installation headroom", () => {
  const v = JSON.parse(readFileSync(`${chart}/examples/bundled-database.json`, "utf8"));
  const rows = containers(v); assert.equal(rows.find((x) => x.name.startsWith("install")).limits.memory, 1024);
  assert.ok(limitFailures(rows, [{ metadata: { name: "bounded" }, spec: { limits: [{ type: "Container", min: { cpu: "200m" } }] } }]).some((s) => s.includes("install")));
});

test("external database probe reuses the rendered authority container without administrator or DDL", (t) => {
  const f = fixture(t);
  const values = JSON.parse(readFileSync(`${chart}/examples/external-database.json`, "utf8")); values.postgres.external.host = "db.team.test";
  const file = join(f.directory, "external.json"); writeFileSync(file, json(values));
  prepare({ output: f.o.output, images: f.images, context: f.o.context, release: f.o.release, namespace: f.o.namespace, route: "shared", database: "external", identity: "bundled", "app-url": "https://app.team.test", "identity-url": "https://identity.team.test", "ingress-class": "private-edge", "app-tls-secret": "app-tls", "identity-tls-secret": "id-tls", "proxy-cidrs": "192.0.2.0/24", "admin-user": "operator", values: (() => { values.keycloak = { database: { hostname: "db.identity.test", existingSecret: "identity-db" }, databaseCaExistingSecret: "identity-ca" }; writeFileSync(file, json(values)); return file; })() });
  const original = kubeFixture(f); let created;
  const run = (binary, args, input) => { if (binary === "kubectl" && args.includes("create") && !args.includes("--dry-run=client")) { created = JSON.parse(input); return { ok: true, text: "" }; } return original(binary, args, input); };
  assert.equal(check({ action: "database-probe", directory: f.o.output, architecture: "arm64" }, run, () => {}), 0);
  const pod = created.items.find((x) => x.kind === "Job").spec.template.spec;
  assert.equal(pod.initContainers, undefined); assert.equal(pod.containers.length, 1);
  assert.deepEqual(pod.containers[0].command, ["/usr/local/bin/synveda-container", "database-preflight"]);
  assert.equal(pod.automountServiceAccountToken, false);
  assert.ok(!JSON.stringify(pod).includes("keycloak-admin")); assert.ok(!JSON.stringify(pod).includes("tenant-kms"));
});

test("support diagnostic ignores secret env, logs, annotations and product content", () => {
  const output = JSON.stringify(diagnostic([{ metadata: { name: "northstar", annotations: { token: "SENSITIVE" } }, spec: { containers: [{ env: [{ value: "SENSITIVE" }] }] }, status: { phase: "Pending", message: "SENSITIVE", containerStatuses: [{ name: "gateway", state: { waiting: { reason: "ImagePullBackOff", message: "SENSITIVE" } } }] } }], [{ metadata: { name: "install" }, status: { conditions: [{ type: "SENSITIVE", status: "SENSITIVE", reason: "SENSITIVE" }] } }], []));
  assert.match(output, /ImagePullBackOff/); assert.ok(!output.includes("SENSITIVE"));
});

test("preflight derives Secret keys without making CNPG generated CA a preinstall prerequisite", () => {
  const v = JSON.parse(readFileSync(`${chart}/examples/existing-cnpg.json`, "utf8"));
  v.fullnameOverride = "synveda"; v.keycloak = JSON.parse(readFileSync(`${chart}/examples/bundled-identity.json`, "utf8")).keycloak;
  assert.ok(!secretKeys(v).has("synveda-pg-ca")); assert.ok(secretKeys(v).has("synveda-keycloak-db"));
  assert.throws(() => options(["--output", "/tmp/x", "--output", "/tmp/y"]), /duplicate/);
});

test("the shipped manual preparation blocks preserve credentials and stop on partial recovery", (t) => {
  const f = fixture(t);
  const blocks = [...readFileSync(`${chart}/MANUAL.md`, "utf8").matchAll(/```sh\n([\s\S]*?)```/g)].map((x) => x[1]);
  const env = { ...process.env, CHART: chart, IMAGES: f.images, MANUAL: f.o.output, CONTEXT: f.o.context, NAMESPACE: f.o.namespace, RELEASE: f.o.release, STORAGE_CLASS: "selected-fast", APP_PORT: "9130", IDENTITY_PORT: "9081" };
  const run = () => spawnSync("sh", ["-c", blocks.slice(0, 2).join("\n")], { env, encoding: "utf8" });
  const first = run(); assert.equal(first.status, 0, first.stderr);
  const names = ["tenant-id", "kms-key", "postgres-password", "gateway-password", "worker-password", "migrator-password", "keycloak-password", "admin-password", "member-password", "approver-password", "viewer-password", "ca.key", "ca.crt", "server.key", "server.crt", "secrets.json", "values.json", "release-images.yaml"];
  const saved = Object.fromEntries(names.map((n) => [n, read(f, n)]));
  const again = run(); assert.equal(again.status, 0, again.stderr);
  for (const [n, bytes] of Object.entries(saved)) { assert.equal(read(f, n), bytes); assert.equal(statSync(join(f.o.output, n)).mode & 0o077, 0); }
  const rendered = render(f); assert.equal(rendered.status, 0, rendered.stderr);
  assert.match(rendered.stdout, /storageClassName: "selected-fast"/);
  rmSync(join(f.o.output, "worker-password"));
  const partial = run(); assert.notEqual(partial.status, 0); assert.match(partial.stderr, /incomplete private material/);
  assert.equal(read(f, "kms-key"), saved["kms-key"]); assert.equal(read(f, "tenant-id"), saved["tenant-id"]);
});

test("preparation requires the selected dependency digests and rejects duplicate image mappings", () => {
  const product = `image:\n  digest: sha256:${"a".repeat(64)}\n`;
  assert.throws(() => imagePins(product, { database: "bundled", identity: "external" }), /pinned selected/);
  assert.throws(() => imagePins(product, { database: "external", identity: "bundled" }), /pinned selected/);
  assert.throws(() => imagePins(product, { database: "cnpg", identity: "external" }), /pinned selected/);
  assert.throws(() => imagePins(`${product}${product}`, { database: "external", identity: "external" }), /duplicate/);
});

test("malformed private JSON and unsafe context input do not echo credential fragments", (t) => {
  const f = fixture(t); prepare(f.o);
  writeFileSync(join(f.o.output, "state.json"), '{"password":"SENSITIVE-PRIVATE-FRAGMENT" broken', { mode: 0o600 });
  for (const [script, args] of [["prepare.mjs", Object.entries(f.o).flatMap(([k, v]) => [`--${k}`, v])], ["operator.mjs", ["offline", "--prepared", f.o.output, "--architecture", "arm64"]]]) {
    const r = spawnSync(process.execPath, [`${chart}/examples/${script}`, ...args], { encoding: "utf8" });
    assert.notEqual(r.status, 0); assert.match(r.stderr, /malformed JSON/); assert.ok(!`${r.stderr}${r.stdout}`.includes("SENSITIVE-PRIVATE-FRAGMENT"));
  }
  assert.throws(() => prepare({ ...f.o, context: '$(unsafe)' }), /context is required/);
});

test("retained reinstall uses the original PVC and Secret bytes without provisioning a replacement", (t) => {
  const f = fixture(t); prepare(f.o);
  const saved = read(f, "secrets.json"); const state = read(f, "state.json");
  const values = JSON.parse(read(f, "values.json"));
  values.postgres.bundled.existingClaim = `${f.o.release}-pg-data`; values.postgres.bundled.storageClass = "";
  writeFileSync(join(f.o.output, "values.json"), json(values));
  const r = render(f); assert.equal(r.status, 0, r.stderr);
  const docs = objects(r.stdout);
  assert.ok(!docs.some((x) => x.kind === "PersistentVolumeClaim"));
  const postgres = docs.find((x) => x.kind === "StatefulSet" && x.metadata.name === "northstar-pg");
  assert.ok(postgres.spec.template.spec.volumes.some((x) => x.persistentVolumeClaim?.claimName === "northstar-pg-data"));
  const messages = [];
  const run = kubeFixture(f, { pvc: () => ({ ok: true, text: '{"status":{"phase":"Bound"},"spec":{"storageClassName":"selected-fast"}}' }) });
  assert.equal(check({ action: "preflight", directory: f.o.output, architecture: "arm64" }, run, (s) => messages.push(s)), 0, messages.join("\n"));
  assert.equal(read(f, "secrets.json"), saved); assert.equal(read(f, "state.json"), state);
  assert.match(messages.join("\n"), /matching database\/key recovery still needs operator verification/);
  assert.match(messages.join("\n"), /existing claim is Bound; administrator still supplies/);
});

test("storage quota and PVC limits fail before Helm while retained claims need no new allocation", (t) => {
  const f = fixture(t); prepare(f.o);
  const messages = [];
  const run = kubeFixture(f, {
    resourcequotas: () => ({ ok: true, text: JSON.stringify({ items: [{ metadata: { name: "namespace-storage" }, status: { hard: { "count/jobs.batch": "0", "requests.storage": "30Gi", "selected-fast.storageclass.storage.k8s.io/requests.storage": "35Gi" }, used: { "requests.storage": "0", "selected-fast.storageclass.storage.k8s.io/requests.storage": "0" } } }] }) }),
    limitranges: () => ({ ok: true, text: JSON.stringify({ items: [{ metadata: { name: "bounded" }, spec: { limits: [{ type: "PersistentVolumeClaim", max: { storage: "32Gi" } }, { type: "Pod", min: { cpu: "200m" } }] } }] }) }),
  });
  assert.ok(check({ action: "preflight", directory: f.o.output, architecture: "arm64", confirmedStorageClass: "selected-fast" }, run, (s) => messages.push(s)) >= 4);
  assert.match(messages.join("\n"), /storage violates PVC bounds/);
  assert.match(messages.join("\n"), /install .* violates Pod bounds/);
  assert.match(messages.join("\n"), /FAIL: quota namespace-storage requests.storage storage headroom/);
  assert.match(messages.join("\n"), /FAIL: quota namespace-storage selected-fast.storageclass.storage.k8s.io\/requests.storage storage headroom/);
  assert.match(messages.join("\n"), /FAIL: quota namespace-storage lacks count\/jobs.batch install\/replacement headroom/);
  const v = JSON.parse(read(f, "values.json")); v.postgres.bundled.existingClaim = "northstar-pg-data";
  assert.deepEqual(storageRequests(v), []);
  assert.throws(() => quantity("1..2Gi", true), /unsupported resource quantity/);
});

test("operator verification separates a failed install stage and registry pull failure from readiness", (t) => {
  const f = fixture(t); prepare(f.o); const messages = [];
  const original = kubeFixture(f, { jobs: () => ({ ok: true, text: JSON.stringify({ items: [{ metadata: { name: "northstar-install-1" }, status: { conditions: [{ type: "Failed", status: "True", reason: "BackoffLimitExceeded" }] } }] }) }) });
  const run = (binary, args, input) => args.includes("exec") ? { ok: false, text: "" } : original(binary, args, input);
  assert.equal(check({ action: "verify", directory: f.o.output }, run, (s) => messages.push(s)), 2);
  assert.match(messages.join("\n"), /database-bootstrap, database-preflight, migrate or tenant/);
  assert.match(messages.join("\n"), /Readiness does not prove login or client delivery/);
  const report = diagnostic([{ metadata: { name: "northstar" }, status: { containerStatuses: [{ name: "gateway", state: { waiting: { reason: "ImagePullBackOff", message: "private registry token" } } }] } }], [], []);
  assert.equal(report.pods[0].containers[0].reason, "ImagePullBackOff"); assert.ok(!JSON.stringify(report).includes("token"));
});

test("API outage stops preflight before a long series of permission checks", (t) => {
  const f = fixture(t); prepare(f.o); const calls = []; const messages = [];
  const original = kubeFixture(f);
  const run = (binary, args, input) => {
    calls.push([binary, args]);
    if (binary === "kubectl" && args.includes("--context") && args.includes("version")) return { ok: false, text: "" };
    return original(binary, args, input);
  };
  assert.equal(check({ action: "preflight", directory: f.o.output, architecture: "arm64" }, run, (s) => messages.push(s)), 1);
  assert.match(messages.join("\n"), /selected Kubernetes API is unavailable/);
  assert.ok(!calls.some(([, args]) => args.includes("can-i") || args.includes("get")));
});

test("the support entry point emits one parseable allowlisted JSON object", (t) => {
  const f = fixture(t); prepare(f.o); const messages = [];
  assert.equal(check({ action: "diagnose", directory: f.o.output }, kubeFixture(f), (s) => messages.push(s)), 0);
  assert.equal(messages.length, 1);
  const report = JSON.parse(messages[0]);
  assert.equal(report.installation.release, "northstar"); assert.deepEqual(report.pods, []);
  assert.ok(!messages[0].includes(JSON.parse(read(f, "state.json")).credentials.kms));
});

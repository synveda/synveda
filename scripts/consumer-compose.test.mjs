import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, symlinkSync, chmodSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { consumerGraph } from "./package-consumer-compose.mjs";
import { retainedState, projectSecrets } from "../deploy/compose/scripts/initialize-consumer.mjs";

const image = `docker.io/test-owner/product@sha256:${"a".repeat(64)}`;
const source = {
  name: "synveda-evaluation",
  services: {
    gateway: { image, environment: {}, secrets: [{ source: "gateway", target: "database_url" }], volumes: [
      { type: "bind", source: "/state/synveda-evaluation/issuers.json", target: "/etc/synveda/oidc/issuers.json", read_only: true },
      { type: "bind", source: "/bundle/deploy/compose/configs/database/roles.reference.json", target: "/etc/synveda/database/roles.json", read_only: true },
    ] },
    "database-bootstrap": { image, environment: {}, secrets: [{ source: "owner", target: "postgres_bootstrap_password" }], volumes: [
      { type: "bind", source: "/bundle/deploy/compose/configs/database/roles.reference.json", target: "/run/secrets/database_roles.json", read_only: true },
    ] },
    worker: { image, environment: {}, secrets: [{ source: "worker", target: "database_url" }], volumes: [
      { type: "bind", source: "/state/synveda-evaluation/secrets/oidc-directory", target: "/run/secrets/oidc_directory", read_only: true },
    ] },
  },
  secrets: { gateway: { file: "/state/secrets/synveda_gateway_database_url" }, owner: { file: "/state/secrets/postgres_owner_password" }, worker: { file: "/state/secrets/synveda_worker_database_url" } },
  volumes: { "postgres-data": { name: "synveda-evaluation_postgres-data" } },
  networks: { data: { name: "synveda-evaluation_data", internal: true } },
};

test("consumer projection keeps the canonical service authority and removes host secret prerequisites", () => {
  const { graph, projections } = consumerGraph(source, "/bundle", { images: { product: image } });
  assert.equal(graph.name, "synveda-local");
  assert.equal(graph.secrets, undefined);
  assert.equal(graph.volumes["postgres-data"].name, undefined);
  assert.equal(graph.networks.data.name, undefined);
  assert.equal(graph.networks.data.internal, true);
  assert.equal(graph.services.initialize.network_mode, "none");
  assert.equal(graph.services.initialize.volumes.find((v) => v.target === "/retained-postgres").read_only, true);
  assert.deepEqual(projections.gateway, { database_url: "synveda_gateway_database_url" });
  assert.deepEqual(projections["database-bootstrap"], { postgres_bootstrap_password: "postgres_owner_password", "database_roles.json": "config:database-roles" });
  for (const name of Object.keys(source.services)) {
    assert.equal(graph.services[name].image, image);
    assert.equal(graph.services[name].depends_on.initialize.condition, "service_completed_successfully");
    assert.equal(graph.services[name].secrets, undefined);
    assert.equal(graph.services[name].volumes.find((v) => v.target === "/run/secrets").read_only, true);
    assert.equal(graph.services[name].volumes.some((v) => v.source === "/state" || v.target === "/var/run/docker.sock"), false);
  }
  assert.equal(graph.services.gateway.volumes.find((v) => v.target === "/etc/synveda/oidc").volume.subpath, "oidc");
  assert.equal(graph.services["database-bootstrap"].volumes.some((v) => v.type === "bind" && v.target.startsWith("/run/secrets/")), false);
  assert.equal(graph.services.worker.volumes.some((v) => v.target === "/run/secrets/oidc_directory"), false);
  assert.equal(source.secrets.gateway.file, "/state/secrets/synveda_gateway_database_url", "canonical input remains unchanged");
});

test("consumer rendering refuses builds, mutable images, foreign binds and unsafe secret names", () => {
  for (const mutate of [
    (graph) => { graph.services.gateway.build = "."; },
    (graph) => { graph.services.gateway.image = "product:latest"; },
    (graph) => { graph.services.gateway.volumes[1].source = "/foreign/roles.json"; },
    (graph) => { graph.services.gateway.secrets[0].target = "../escape"; },
  ]) {
    const graph = structuredClone(source);
    mutate(graph);
    assert.throws(() => consumerGraph(graph, "/bundle", { images: { product: image } }));
  }
});

function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), "synveda consumer λ "));
  chmodSync(root, 0o700);
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}
const uid = process.getuid();
const identity = { project: "synveda-local", release: "test", issuer: "http://localhost:8080" };

test("retained database and changed project/release/issuer refuse before secret generation", (t) => {
  const root = fixture(t);
  assert.throws(() => retainedState(root, true, identity, uid), /retained PostgreSQL/);
  const state = retainedState(root, false, identity, uid);
  assert.equal(state.binding, path.join(root, "consumer.json"));
  writeFileSync(state.binding, `${JSON.stringify(identity)}\n`, { mode: 0o600 });
  assert.doesNotThrow(() => retainedState(root, false, identity, uid));
  assert.throws(() => retainedState(root, false, { ...identity, issuer: "http://localhost:9999" }, uid), /configuration changed/);
  assert.throws(() => retainedState(root, true, identity, uid), /retained PostgreSQL/);
  writeFileSync(state.seal, "{}\n", { mode: 0o600 });
  assert.throws(() => retainedState(root, true, identity, uid), /secret set is missing/);
  rmSync(state.binding);
  assert.throws(() => retainedState(root, false, identity, uid), /not empty/);
});

test("secret projections are private, exact, repeatable and refuse tampering or symlinks", (t) => {
  const root = fixture(t);
  const state = path.join(root, "synveda-evaluation");
  mkdirSync(path.join(state, "secrets"), { recursive: true, mode: 0o700 });
  writeFileSync(path.join(state, "issuers.json"), "[]\n", { mode: 0o600 });
  writeFileSync(path.join(state, "secrets/gateway_url"), "private-fixture\n", { mode: 0o600 });
  writeFileSync(path.join(state, "secrets/owner_password"), "owner-fixture\n", { mode: 0o600 });
  const projections = { gateway: { database_url: "gateway_url" }, worker: { database_url: "gateway_url" } };
  projectSecrets(root, projections, uid);
  projectSecrets(root, projections, uid);
  assert.equal(readFileSync(path.join(root, "projections/gateway/database_url"), "utf8"), "private-fixture\n");
  assert.throws(() => readFileSync(path.join(root, "projections/gateway/owner_password")), /ENOENT/);
  assert.equal(readFileSync(path.join(root, "oidc/issuers.json"), "utf8"), "[]\n");
  const file = path.join(root, "projections/gateway/database_url");
  writeFileSync(file, "changed\n");
  assert.throws(() => projectSecrets(root, projections, uid), /retained configuration differs/);
  rmSync(file);
  symlinkSync(path.join(state, "secrets/gateway_url"), file);
  assert.throws(() => projectSecrets(root, projections, uid), /private file/);
});

test("the opt-in packaged candidate resolves fresh without any generated host secret files", (t) => {
  const root = fixture(t);
  const version = "0.0.0-consumer-test";
  const sha = "b".repeat(40);
  execFileSync("bash", ["scripts/package-release.sh", version, root, sha, ...Array(6).fill(`sha256:${"a".repeat(64)}`)], {
    env: { ...process.env, SYNVEDA_PACKAGE_CONSUMER_CANDIDATE: "1" }, stdio: "pipe", timeout: 30_000,
  });
  execFileSync("tar", ["-xzf", path.join(root, `synveda-reference-${version}.tar.gz`), "-C", root]);
  const bundle = path.join(root, `synveda-reference-${version}`);
  const packaged = JSON.parse(readFileSync(path.join(bundle, "deploy/compose/consumer-runtime.yaml"), "utf8").replace(/^#.*\n/, ""));
  for (const service of Object.values(packaged.services)) {
    for (const volume of service.volumes ?? []) {
      if (volume.type === "bind") assert.equal(volume.bind.create_host_path, false);
    }
  }
  const project = "synveda-local-acceptance-render";
  const rendered = JSON.parse(execFileSync("docker", ["compose", "--profile", "*", "config", "--format", "json"], {
    cwd: bundle, env: { PATH: process.env.PATH, HOME: process.env.HOME, COMPOSE_PROJECT_NAME: project }, encoding: "utf8", timeout: 15_000, stdio: "pipe",
  }));
  assert.equal(rendered.name, project);
  assert.equal(rendered.services.initialize.environment.SYNVEDA_CONSUMER_PROJECT, project);
  assert.equal(rendered.volumes.installation.name, `${project}_installation`);
  assert.equal(rendered.secrets, undefined);
  assert.equal(rendered.services.proxy.ports[0].host_ip, "127.0.0.1");
  assert.equal(rendered.services.proxy.ports[0].published, "8080");
  assert.equal(Object.values(rendered.services).filter((service) => service.ports?.length).length, 1);
  for (const [name, service] of Object.entries(rendered.services)) {
    assert.equal(service.build, undefined);
    assert.match(service.image, /@sha256:[0-9a-f]{64}$/);
    for (const volume of service.volumes ?? []) {
      if (volume.type === "bind") {
        const mounted = realpathSync(volume.source), expected = realpathSync(bundle);
        assert.ok(mounted === expected || mounted.startsWith(`${expected}/deploy/compose/`), `${name} has a non-bundle host path`);
        assert.equal(volume.read_only, true);
        // Compose 2.x omits false from normalized JSON. The artifact above
        // must still carry it explicitly for consumers with other defaults.
        assert.equal(volume.bind?.create_host_path ?? false, false);
      }
    }
  }
  assert.match(readFileSync(path.join(bundle, "CONSUMER.md"), "utf8"), /unpublished OPS-12/);
  assert.match(readFileSync(path.join(bundle, "synveda-compose"), "utf8"), /evaluation.sh/, "existing operator/recovery launcher is preserved");
  const projections = JSON.parse(readFileSync(path.join(bundle, "deploy/compose/consumer-projections.json")));
  assert.equal(projections["browser-acceptance"].keycloak_demo_admin_password, "keycloak_demo_admin_password");
});

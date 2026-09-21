#!/usr/bin/env node
// OPS-12: derive the consumer artifact from Compose's canonical merged graph.
// This build-time renderer requires the Compose CLI, never a running daemon.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, renameSync, rmSync } from "node:fs";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { evaluationEnvironment } from "../deploy/compose/scripts/prepare-evaluation.mjs";

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
function publish(file, bytes) {
  const temporary = `${file}.${randomUUID()}.preparing`;
  try {
    writeFileSync(temporary, bytes, { flag: "wx" });
    renameSync(temporary, file);
  } finally { rmSync(temporary, { force: true }); }
}
const mount = (subpath, target, read_only = true) => ({ type: "volume", source: "installation", target, read_only, volume: { nocopy: true, subpath } });
const bind = (source, target) => ({ type: "bind", source, target, read_only: true, bind: { create_host_path: false } });
const completed = { condition: "service_completed_successfully", required: true };

export function consumerGraph(graph, bundle, manifest) {
  graph = structuredClone(graph);
  const projections = {};
  for (const [name, service] of Object.entries(graph.services)) {
    if (service.build || !/^[\w./:-]+@sha256:[a-f0-9]{64}$/.test(service.image ?? "")) throw new Error(`consumer ${name} must use an immutable prebuilt image`);
    service.depends_on = { ...service.depends_on, initialize: completed };
    const projected = Object.fromEntries((service.secrets ?? []).map(({ source, target }) => {
      const file = path.basename(graph.secrets[source]?.file ?? "");
      if (!/^[a-z][a-z0-9_]+$/.test(file) || !/^[a-z][a-z0-9_]+$/.test(target)) throw new Error("unsupported secret projection");
      return [target, file];
    }));
    delete service.secrets;
    service.volumes = (service.volumes ?? []).flatMap((volume) => {
      if (volume.type !== "bind") return [volume];
      if (volume.target === "/etc/synveda/oidc/issuers.json") return [mount("synveda-evaluation", "/etc/synveda/oidc")];
      if (volume.target === "/run/synveda/database-authority") return [mount("synveda-evaluation/database-authority", volume.target, volume.read_only === true)];
      if (volume.target === "/run/synveda/keycloak-public-gate") return [mount("synveda-evaluation/keycloak-public-gate", volume.target, volume.read_only === true)];
      if (volume.target === "/run/secrets/oidc_directory") return [];
      const relative = path.relative(bundle, volume.source);
      if (!relative.startsWith("deploy/compose/") || relative.includes("..")) throw new Error(`unexpected consumer bind for ${name}`);
      if (volume.target === "/run/secrets/database_roles.json") {
        if (relative !== "deploy/compose/configs/database/roles.reference.json") throw new Error("unexpected database authority contract");
        projected["database_roles.json"] = "config:database-roles";
        return [];
      }
      return [bind(`./${relative}`, volume.target)];
    });
    if (Object.keys(projected).length) {
      projections[name] = projected;
      service.volumes.unshift(mount(`projections/${name}`, "/run/secrets"));
    }
    if (service.security_opt) service.security_opt = service.security_opt.map((option) => option.startsWith("seccomp=") ? "seccomp=./deploy/compose/browser/seccomp_profile.json" : option);
  }
  // The issuer directory must contain only public configuration, never the
  // parent directory that also carries private encryption/database material.
  for (const service of Object.values(graph.services)) for (const volume of service.volumes ?? []) {
    if (volume.target === "/etc/synveda/oidc") volume.volume.subpath = "oidc";
  }
  delete graph.secrets;
  graph.name = "synveda-local";
  for (const kind of ["networks", "volumes"]) for (const value of Object.values(graph[kind] ?? {})) delete value.name;
  graph.volumes.installation = { labels: { "com.synveda.contract": "cpr-45", "com.synveda.volume": "consumer-installation" } };
  const closed = Object.fromEntries(["HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy", "FTP_PROXY", "ftp_proxy", "ALL_PROXY", "all_proxy"].map((key) => [key, ""]));
  const utility = {
    image: manifest.images.product, read_only: true, init: true, cap_drop: ["ALL"],
    security_opt: ["no-new-privileges:true"], pids_limit: 64, mem_limit: "268435456", cpus: 0.5,
    restart: "no", network_mode: "none", environment: closed,
    tmpfs: ["/tmp:rw,noexec,nosuid,nodev,mode=1777,size=32m"],
  };
  graph.services.initialize = {
    ...utility, user: "0:0", cap_add: ["CHOWN", "DAC_OVERRIDE", "FOWNER", "SETGID", "SETUID"],
    entrypoint: ["/usr/bin/timeout", "--signal=TERM", "--kill-after=5s", "90s", "/usr/bin/flock", "-w", "30", "/state/.consumer.lock", "/usr/local/bin/node", "/bundle/deploy/compose/scripts/initialize-consumer.mjs"],
    environment: { ...closed, SYNVEDA_CONSUMER_PROJECT: "${COMPOSE_PROJECT_NAME}" },
    volumes: [
      bind(".", "/bundle"),
      { type: "volume", source: "installation", target: "/state", volume: { nocopy: true } },
      { type: "volume", source: "postgres-data", target: "/retained-postgres", read_only: true, volume: { nocopy: true } },
    ],
  };
  projections.credentials = { keycloak_demo_admin_password: "keycloak_demo_admin_password" };
  graph.services.credentials = {
    ...utility, user: "65532:65532", profiles: ["tools"], depends_on: { initialize: completed },
    entrypoint: ["/usr/local/bin/node", "-e", "const fs=require('node:fs');process.stdout.write('Username: synveda-demo-admin\\nPassword: '+fs.readFileSync('/run/secrets/keycloak_demo_admin_password','utf8'));"],
    volumes: [mount("projections/credentials", "/run/secrets")],
  };
  if (graph.services["browser-acceptance"]) {
    const browser = graph.services["browser-acceptance"];
    browser.network_mode = "service:proxy";
    delete browser.networks;
  }
  return { graph, projections };
}

export function packageConsumer(bundle, run = execFileSync) {
  bundle = path.resolve(bundle);
  const manifest = readJson(path.join(bundle, "environment.json"));
  const options = readJson(path.join(bundle, "evaluation.json"));
  if (!options.demoAccounts) throw new Error("consumer first login requires evaluation accounts");
  const compose = path.join(bundle, "deploy/compose");
  const env = {
    PATH: process.env.PATH, HOME: process.env.HOME, DOCKER_CONFIG: process.env.DOCKER_CONFIG,
    ...Object.fromEntries(Object.entries(evaluationEnvironment(manifest, options, "/state", 65532, 65532, compose)).map(([key, value]) => [key, String(value)])),
  };
  const files = ["compose.yaml", "compose.postgres.yaml", "compose.keycloak.yaml", "compose.keycloak-postgres.yaml", "compose.evaluation.yaml", "compose.demo.yaml", "compose.browser-acceptance.yaml"];
  const merged = JSON.parse(run("docker", ["compose", "--env-file", "/dev/null", "--profile", "*", ...files.flatMap((file) => ["-f", path.join(compose, file)]), "config", "--format", "json"], { env, cwd: bundle, encoding: "utf8", timeout: 30_000, stdio: ["pipe", "pipe", "pipe"] }));
  const { graph, projections } = consumerGraph(merged, bundle, manifest);
  // JSON is valid YAML. Preserve explicit false values and unresolved project
  // names: older Compose serializers omit create_host_path=false, changing
  // the bind contract when the resulting artifact is read by newer versions.
  publish(path.join(compose, "consumer-runtime.yaml"), `# Generated from the canonical CPR-45 fragments; do not edit.\n${JSON.stringify(graph, null, 2)}\n`);
  publish(path.join(compose, "consumer-projections.json"), `${JSON.stringify(projections, null, 2)}\n`);
  publish(path.join(bundle, "compose.yaml"), "# OPS-12 consumer candidate. See CONSUMER.md for qualification status.\nname: synveda-local\ninclude:\n  - path: ./deploy/compose/consumer-runtime.yaml\n    project_directory: .\n");
  return graph;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw new Error("usage: package-consumer-compose.mjs EXTRACTED_REFERENCE_BUNDLE");
    packageConsumer(process.argv[2]);
  } catch (error) {
    console.error(`consumer-compose: ${error.message}`);
    process.exitCode = 1;
  }
}

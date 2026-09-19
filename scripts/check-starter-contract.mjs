#!/usr/bin/env node
// OPS-11: prove independent provider selection and the credential/exposure boundary.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const chart = "deploy/helm/synveda";
const read = (path) => readFileSync(path, "utf8");
function render(database, identity, extra = []) {
  const args = ["template", "synveda", chart, "--api-versions", "postgresql.cnpg.io/v1",
    "-f", `${chart}/ci/${database}-values.yaml`];
  if (identity === "packaged") args.push("-f", `${chart}/ci/packaged-keycloak-values.yaml`);
  return spawnSync("helm", [...args, ...extra], { encoding: "utf8" });
}
for (const database of ["external", "cnpg"]) {
  for (const identity of ["external", "packaged"]) {
    const result = render(database, identity);
    assert.equal(result.status, 0, result.stderr);
    const out = result.stdout;
    assert.equal(/^kind: Cluster$/m.test(out), database === "cnpg");
    assert.equal(/^kind: StatefulSet$/m.test(out), identity === "packaged");
    assert.equal(out.includes("synveda-database-bootstrap keycloak"), database === "cnpg" && identity === "packaged");
    assert.ok(!/hostPath:|start-dev|automountServiceAccountToken: true/.test(out));
    if (identity === "packaged") {
      assert.match(out, /name: KC_DB_URL_DATABASE\n\s+value: keycloak/);
      assert.match(out, /name: KC_DB_USERNAME\n\s+value: keycloak/);
      assert.match(out, /name: KC_DB_PASSWORD\n\s+valueFrom:\n\s+secretKeyRef:\n\s+name: synveda-keycloak-db/);
      assert.match(out, /sslmode=verify-full/);
      assert.match(out, /type: OnDelete/);
      assert.match(out, /"registrationAllowed": false/);
      assert.ok(!/"users"\s*:|"credentials"\s*:/.test(out), "no seeded team identities");
      if (database === "cnpg") {
        assert.match(out, /"forbidden_databases":\["keycloak","postgres","template1"\]/);
        assert.match(out, /"isolated_peer_roles":\["keycloak"\]/);
        assert.match(out, /SYNVEDA_DATABASE_PEER_WITNESS_FILE/);
      }
    }
    console.log(`ok: ${database} PostgreSQL / ${identity} OIDC`);
  }
}
const negative = [
  ["postgres.primaryUpdateStrategy=supervised", "requires unsupervised"],
  ["keycloak.database.existingSecret=synveda-gateway-db", "must be distinct"],
  ["keycloak.database.database=synveda", "keycloak database/user"],
  ["keycloak.database.username=synveda_migrator", "keycloak database/user"],
  ["keycloak.database.hostname=unrelated", "share the existing CNPG server"],
  ["keycloak.database.password=not-a-real-password", "inline database passwords"],
  ["keycloak.replicas=2", "one replica"],
  ["keycloak.updateStrategy=RollingUpdate", "explicit maintenance"],
  ["keycloak.image.tag=", "appVersion fallback"],
  ["keycloak.image.tag=latest", "fixed image"],
  ["keycloak.http.relativePath=/auth", "existing optimized image"],
  ["keycloak.publicUrl=http://auth.example.com", "HTTPS"],
  ["keycloak.proxyTrustedAddresses=0.0.0.0/0", "trusted proxy network"],
  ["keycloak.ingress.console.enabled=true", "private ClusterIP"],
  ["keycloak.serviceAccount.automountServiceAccountToken=true", "Kubernetes API credentials"],
  ["keycloak.ingress.enabled=true", "TLS Secret"],
];
for (const [setting, refusal] of negative) {
  const result = render("cnpg", "packaged", ["--set", setting]);
  assert.notEqual(result.status, 0, setting);
  assert.ok(result.stderr.includes(refusal), `${setting}: ${result.stderr}`);
}
const preset = spawnSync("helm", ["template", "synveda", chart, "--api-versions", "postgresql.cnpg.io/v1", "-f", `${chart}/starter-values.yaml`], { encoding: "utf8" });
assert.equal(preset.status, 0, preset.stderr);
assert.match(preset.stdout, /helm.sh\/resource-policy: keep/);
assert.match(preset.stdout, /primaryUpdateStrategy: unsupervised/);
const ingresses = preset.stdout.split(/^---\s*$/m).filter((doc) => /^kind: Ingress$/m.test(doc));
assert.equal(ingresses.length, 2);
assert.ok(ingresses.every((doc) => !/path:.*(?:admin|metrics|health|master)/.test(doc)));
for (const mapper of ["audience", "groups"]) {
  const name = `synveda-${mapper}-mapper.json`;
  assert.equal(read(`${chart}/files/${name}`), read(`deploy/compose/keycloak/${name}`));
}
const lock = read(`${chart}/Chart.lock`);
const privateRegistry = render("cnpg", "packaged", ["--set", "imagePullSecrets[0].name=product-registry", "--set", "keycloak.imagePullSecrets[0].name=identity-registry"]);
assert.equal(privateRegistry.status, 0, privateRegistry.stderr);
for (const [kind, name] of [["Cluster", "product-registry"], ["StatefulSet", "identity-registry"]]) {
  const resource = privateRegistry.stdout.split(/^---\s*$/m).find((doc) => new RegExp(`^kind: ${kind}$`, "m").test(doc));
  assert.match(resource, new RegExp(`imagePullSecrets:\\n\\s+- name: ${name}`));
}
assert.match(lock, /version: 7.3.2/);
// This is the upstream repository index's package digest, not Helm's dependency-list digest.
assert.equal(createHash("sha256").update(readFileSync(`${chart}/charts/keycloakx-7.3.2.tgz`)).digest("hex"),
  "4ebb630c1f842245f7953be4f80ed080d1979ded25b31d48052f43f012e3c1d7");
assert.match(read(`${chart}/third-party/keycloakx-LICENSE`), /Apache License/);
console.log("ok: retained starter, private administration, isolated credentials, upstream lock and unchanged OIDC mappers");

#!/usr/bin/env node
// OPS-11: inspect actual rendered objects, including the locked dependency.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const chart = "deploy/helm/synveda";
const scratch = mkdtempSync(join(tmpdir(), "synveda-portability-"));
const env = { ...process.env, HELM_CACHE_HOME: scratch, HELM_PLUGINS: resolve(chart, "post-renderers") };
function render(extra = [], success = true) {
  const result = spawnSync("helm", ["template", "synveda", chart, "-n", "team",
    "-f", `${chart}/ci/external-values.yaml`, ...extra], { env, encoding: "utf8" });
  if (!success) return result;
  assert.equal(result.status, 0, result.stderr);
  const parsed = spawnSync("ruby", ["-rjson", "-ryaml", "-e", "puts JSON.generate(YAML.load_stream(STDIN.read).compact)"], { input: result.stdout, encoding: "utf8" });
  assert.equal(parsed.status, 0, parsed.stderr);
  return JSON.parse(parsed.stdout);
}
const openshift = ["-f", `${chart}/examples/openshift.yaml`, "--api-versions", "route.openshift.io/v1"];
const packaged = ["-f", `${chart}/ci/packaged-keycloak-values.yaml`];
const policy = ["-f", `${chart}/examples/network-policy.yaml`];
const tei = ["--set", "embedder=tei,tei.enabled=true", "--set-string", "tei.model=/data/preloaded", "--set-string", "tei.cache.existingClaim=preloaded-model"];
function pods(objects) {
  return objects.filter((o) => ["Deployment", "StatefulSet", "Job", "Pod"].includes(o.kind))
    .map((o) => ({ name: o.metadata.name, pod: o.kind === "Pod" ? o : o.spec.template }));
}
function checkSecurity(objects, assigned, userNamespaces) {
  const accounts = new Map(objects.filter((o) => o.kind === "ServiceAccount").map((o) => [o.metadata.name, o]));
  for (const { name, pod } of pods(objects)) {
    const spec = pod.spec;
    assert.ok(spec.automountServiceAccountToken === false || accounts.get(spec.serviceAccountName)?.automountServiceAccountToken === false, `${name}: token automount`);
    assert.equal(spec.securityContext.runAsNonRoot, true, name);
    if (assigned) for (const key of ["runAsUser", "runAsGroup", "fsGroup"]) assert.equal(spec.securityContext[key], undefined, `${name}: ${key}`);
    if (userNamespaces) {
      assert.equal(spec.hostUsers, false, name);
      assert.equal(pod.metadata.annotations["openshift.io/required-scc"], "restricted-v3", name);
    }
    for (const container of [...(spec.initContainers ?? []), ...spec.containers]) {
      const security = container.securityContext;
      assert.equal(security.allowPrivilegeEscalation, false, `${name}/${container.name}`);
      assert.equal(security.readOnlyRootFilesystem, true, name);
      assert.deepEqual(security.capabilities.drop, ["ALL"], name);
      assert.equal((security.seccompProfile ?? spec.securityContext.seccompProfile)?.type, "RuntimeDefault", name);
      if (assigned) for (const key of ["runAsUser", "runAsGroup"]) assert.equal(security[key], undefined, name);
      for (const port of container.ports ?? []) assert.ok(port.containerPort >= 1024, name);
    }
  }
}
try {
  for (const version of ["1.32.0", "1.33.0", "1.36.1"]) {
    for (const identity of [[], packaged]) {
      const objects = render([...identity, ...openshift, ...policy, ...tei, "--kube-version", version]);
      checkSecurity(objects, true, false);
      assert.equal(objects.some((o) => o.kind === "Ingress"), false);
      assert.equal(objects.some((o) => o.kind === "PersistentVolumeClaim"), false, "existing model claim must be reused");
      const routes = objects.filter((o) => o.kind === "Route");
      assert.equal(routes.length, identity.length ? 6 : 4);
      assert.deepEqual(routes.map((o) => o.spec.path).sort(), ["/auth", "/v1", "/console", "/scim/v2", ...(identity.length ? ["/realms/synveda", "/resources"] : [])].sort());
      for (const route of routes) {
        assert.equal(route.spec.tls.termination, "edge");
        assert.equal(route.spec.tls.insecureEdgeTerminationPolicy, "Redirect");
        assert.equal(route.metadata.annotations["haproxy.router.openshift.io/set-forwarded-headers"], "replace");
      }
      const policies = objects.filter((o) => o.kind === "NetworkPolicy");
      assert.equal(policies.length, identity.length ? 5 : 4);
      for (const p of policies) assert.deepEqual(p.spec.policyTypes, ["Ingress", "Egress"]);
      for (const component of ["worker", "install"]) assert.deepEqual(policies.find((p) => p.metadata.name === `synveda-${component}`).spec.ingress, []);
      const install = policies.find((p) => p.metadata.name === "synveda-install");
      assert.deepEqual(install.spec.egress.flatMap((r) => r.ports.map((p) => p.port)), [53, 53, 5432]);
      console.log(`ok: Kubernetes ${version} APIs; ${identity.length ? "packaged" : "external"} identity; assigned IDs, Routes, policies, existing TEI cache`);
    }
  }
  const helmMajor = spawnSync("helm", ["version", "--short"], { encoding: "utf8" }).stdout;
  const renderer = helmMajor.startsWith("v4.") ? "synveda-openshift-v3" : resolve(chart, "post-renderers/openshift-v3/render.sh");
  const v3 = render([...packaged, ...openshift, ...tei, "--kube-version", "1.33.0", "--post-renderer", renderer]);
  checkSecurity(v3, true, true);
  const certs = render([...packaged, ...openshift, "--set", "route.existingCertificateSecret=app-tls,route.keycloakCertificateSecret=id-tls"]);
  const role = certs.find((o) => o.kind === "Role");
  assert.deepEqual(role.rules[0].resourceNames, ["app-tls", "id-tls"]);
  assert.deepEqual(certs.find((o) => o.kind === "RoleBinding").subjects, [{ kind: "ServiceAccount", name: "router", namespace: "openshift-ingress" }]);
  const native = render(["--set", "outbound.caBundleExistingSecret=ca,outbound.proxyExistingSecret=proxy"]);
  for (const { pod } of pods(native).filter((p) => !p.name.includes("install"))) {
    const env = pod.spec.containers[0].env;
    assert.equal(env.find((e) => e.name === "SSL_CERT_FILE").value, "/run/synveda-ca/ca-bundle.crt");
    assert.equal(env.find((e) => e.name === "NO_PROXY").valueFrom.secretKeyRef.name, "proxy");
  }
  checkSecurity(render(), false, false);
  const managed = render(["-f", `${chart}/examples/managed-kubernetes.yaml`]);
  assert.equal(managed.filter((o) => o.kind === "Ingress").length, 1);
  assert.equal(managed.some((o) => o.kind === "Route"), false);
  const cnpg = render(["-f", `${chart}/ci/cnpg-values.yaml`, "--set-json", "postgres.external={}", "--api-versions", "postgresql.cnpg.io/v1", ...packaged, ...policy,
    "--set-json", 'networkPolicy.database=[{"podSelector":{"matchLabels":{"cnpg.io/cluster":"synveda-pg"}}}]',
    "--set-json", 'networkPolicy.postgresIngress=[{"from":[{"namespaceSelector":{"matchLabels":{"kubernetes.io/metadata.name":"cnpg-system"}}}],"ports":[{"protocol":"TCP","port":8000}]}]',
    "--set-json", 'networkPolicy.postgresEgress=[{"to":[{"ipBlock":{"cidr":"192.0.2.30/32"}}],"ports":[{"protocol":"TCP","port":6443}]}]']);
  checkSecurity(cnpg, false, false);
  const databasePolicy = cnpg.find((o) => o.kind === "NetworkPolicy" && o.metadata.name === "synveda-postgres");
  assert.deepEqual(databasePolicy.spec.podSelector.matchLabels, { "cnpg.io/cluster": "synveda-pg" });
  assert.deepEqual(databasePolicy.spec.ingress.flatMap((r) => r.ports.map((p) => p.port)), [5432, 8000]);
  assert.deepEqual(databasePolicy.spec.egress.flatMap((r) => r.ports.map((p) => p.port)), [53, 53, 5432, 6443]);
  for (const [args, message] of [
    [["-f", `${chart}/examples/openshift.yaml`], "requires the existing OpenShift"],
    [[...openshift, "-f", `${chart}/examples/managed-kubernetes.yaml`], "exactly one exposure"],
    [[...openshift, "--set", "route.termination=reencrypt"], "termination"],
    [[...openshift, "--set-string", "gateway.publicUrl=https://app.example.test:8443"], "without a port or path"],
    [[...packaged, "--set", "keycloak.test.enabled=true"], "upstream Selenium"],
    [[...packaged, "--set", "keycloak.serviceAccount.create=false"], "token-free ServiceAccount"],
    [[...packaged, "--set", "keycloak.securityContext.privileged=true"], "cannot request privilege"],
    [[...packaged, ...openshift, "--set", "keycloak.securityContext.runAsUser=1000"], "omit numeric"],
    [["--set", "networkPolicy.enabled=true"], "explicit networkPolicy.ingress"],
    [[...policy, "--set-json", 'networkPolicy.dns=[{}]'], "minProperties"],
    [[...policy, "--set-json", 'networkPolicy.oidc=[{}]'], "ports"],
    [[...policy, "--set", "extractor.kind=claude,extractor.existingSecret=anthropic"], "extraEgress.worker"],
    [[...policy, "--set", "embedder=tei,tei.enabled=true"], "TEI model download"],
    [["--set-string", "otel.endpoint=https://collector.example.test:4317"], "private HTTP Collector"],
    [[...tei, "--set", "tei.cache.storageClass=another"], "existingClaim"],
  ]) {
    const result = render(args, false);
    assert.notEqual(result.status, 0, `expected refusal: ${message}`);
    assert.ok(result.stderr.includes(message), result.stderr);
  }
  console.log("ok: restricted-v3 post-renderer, certificate RBAC, native client inputs, and portability refusals");
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

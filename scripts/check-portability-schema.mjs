#!/usr/bin/env node
// OPS-11: opt-in structural schema validation; never an SCC admission test.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const chart = "deploy/helm/synveda";
const scratch = mkdtempSync(join(tmpdir(), "synveda-platform-schema-"));
const env = { ...process.env, HELM_CACHE_HOME: scratch, HELM_PLUGINS: resolve(chart, "post-renderers") };
function run(command, args) {
  const result = spawnSync(command, args, { env, encoding: "utf8", timeout: 120000, maxBuffer: 16 * 1024 * 1024 });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  return result.stdout;
}
function strict(schema) {
  if (!schema || typeof schema !== "object") return;
  if (schema.type === "object" && schema.properties && schema.additionalProperties === undefined) schema.additionalProperties = false;
  for (const child of Object.values(schema)) strict(child);
}
try {
  const validator = process.env.KUBECONFORM || "kubeconform";
  console.log(run(validator, ["-v"]).trim());
  for (const [version, kube, commit, digest] of [
    ["4.19", "1.32.0", "1ebdd7bba8e6c503a1f61c6d467ed877783826e1", "071b7866022c113bf401d308b7e1e15a7602a435e3939439b33433bab18027d6"],
    ["4.20", "1.33.0", "05da40ea98557f8351483a39db3d0e7d9076bc8f", "6add57dc2e0a7573fc296962e28c6862672b623d53d4b28f74fa132953a03931"],
  ]) {
    const path = join(scratch, version);
    mkdirSync(path);
    const upstream = run("curl", ["--fail", "--silent", "--show-error", "--location", "--max-time", "60", `https://raw.githubusercontent.com/openshift/api/${commit}/openapi/openapi.json`]);
    assert.equal(createHash("sha256").update(upstream).digest("hex"), digest);
    const { definitions } = JSON.parse(upstream);
    // Translate the one Swagger union needed by Route targetPort to JSON Schema.
    definitions["io.k8s.apimachinery.pkg.util.intstr.IntOrString"] = { anyOf: [{ type: "string" }, { type: "integer" }] };
    const schema = { $schema: "http://json-schema.org/draft-04/schema#", ...definitions["com.github.openshift.api.route.v1.Route"], definitions };
    strict(schema);
    writeFileSync(join(path, "Route.json"), JSON.stringify(schema));
    const args = ["template", "synveda", chart, "--kube-version", kube, "--api-versions", "route.openshift.io/v1",
      "-f", `${chart}/ci/external-values.yaml`, "-f", `${chart}/ci/packaged-keycloak-values.yaml`,
      "-f", `${chart}/examples/openshift.yaml`, "-f", `${chart}/examples/network-policy.yaml`,
      "--set", "embedder=tei,tei.enabled=true,tei.model=/data/preloaded,route.existingCertificateSecret=app-tls,route.keycloakCertificateSecret=id-tls"];
    if (version === "4.20") {
      const renderer = run("helm", ["version", "--short"]).startsWith("v4.") ? "synveda-openshift-v3" : resolve(chart, "post-renderers/openshift-v3/render.sh");
      args.push("--post-renderer", renderer);
    }
    const manifest = join(path, "rendered.yaml");
    writeFileSync(manifest, run("helm", args));
    console.log(`OpenShift ${version} Route schema ${commit}; Kubernetes ${kube}:`);
    console.log(run(validator, ["-strict", "-summary", "-kubernetes-version", kube, "-schema-location", "default", "-schema-location", `${path}/{{.ResourceKind}}.json`, manifest]).trim());
  }
  const manifest = join(scratch, "kubernetes.yaml");
  writeFileSync(manifest, run("helm", ["template", "synveda", chart, "-f", `${chart}/ci/external-values.yaml`, "-f", `${chart}/examples/managed-kubernetes.yaml`]));
  console.log("Standard Kubernetes 1.36.1:");
  console.log(run(validator, ["-strict", "-summary", "-kubernetes-version", "1.36.1", manifest]).trim());
} finally {
  rmSync(scratch, { recursive: true, force: true });
}

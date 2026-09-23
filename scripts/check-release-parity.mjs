#!/usr/bin/env node
// CPR-45 / PR-01: deterministic release-to-Helm artifact parity. This gate
// packages and renders the chart locally, but never contacts a Docker daemon,
// registry, Kubernetes or GitHub. Pullability and provenance remain live
// release evidence, not conclusions of this check.

import { execFileSync } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const read = (path) => readFileSync(join(ROOT, path), "utf8");

function stepBlock(source, name) {
  const marker = `      - name: ${name}\n`;
  const start = source.indexOf(marker);
  if (start < 0) return "";
  const next = source.indexOf("\n      - ", start + marker.length);
  return source.slice(start, next < 0 ? source.length : next);
}

function topLevelYamlBlock(source, key) {
  const match = new RegExp(`^${key}:\\s*$`, "m").exec(source);
  if (!match) return "";
  const start = match.index;
  const rest = source.slice(start + match[0].length);
  const next = /^\S[^\n]*:\s*$/m.exec(rest);
  return source.slice(start, next ? start + match[0].length + next.index : source.length);
}

export function workspaceVersion(source) {
  return source.match(
    /^\[workspace\.package\]\s*$[\s\S]*?^version\s*=\s*"([^"]+)"\s*$/m,
  )?.[1];
}

export function chartParityFindings(cargo, chart, values, helpers, cluster, install) {
  const findings = [];
  const workspace = workspaceVersion(cargo);
  const chartVersion = chart.match(/^version:\s*"?([^"\s]+)"?\s*$/m)?.[1];
  const appVersion = chart.match(/^appVersion:\s*"?([^"\s]+)"?\s*$/m)?.[1];
  if (!workspace || chartVersion !== workspace || appVersion !== workspace) {
    findings.push("workspace, chart version and appVersion are not identical");
  }
  const image = topLevelYamlBlock(values, "image");
  if (!image.includes("  repository: ghcr.io/synveda/product\n")) {
    findings.push("chart product image does not use the GHCR publication repository");
  }
  if (!image.includes('  tag: ""\n')) {
    findings.push("chart product image tag does not default to appVersion");
  }
  const postgres = topLevelYamlBlock(values, "postgres");
  if (!postgres.includes('  image: ""\n')) {
    findings.push("chart PostgreSQL override is not empty by default");
  }
  if (!/  bundled:\n(?:[^\n]*\n)*?    image: ""\n/.test(postgres)) {
    findings.push("bundled PostgreSQL image override must default to the release coordinate");
  }
  const postgresHelper =
    '{{- default (printf "ghcr.io/synveda/cnpg-postgres:17.11-synveda-%s" .Chart.AppVersion) .Values.postgres.image -}}';
  if (!helpers.includes(postgresHelper)) {
    findings.push("chart PostgreSQL image does not default to the CNPG-compatible release coordinate");
  }
  const chartLabel =
    'helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimAll "-._" }}';
  if (!helpers.includes(chartLabel)) {
    findings.push("chart version label does not remain valid after truncation");
  }
  for (const [name, source, marker] of [
    ["CNPG Cluster", cluster, 'imageName: {{ include "synveda.postgresImage" . }}'],
    ["database bootstrap", install, 'image: {{ include "synveda.postgresImage" . }}'],
  ]) {
    if (!source.includes(marker)) {
      findings.push(`${name} does not consume the shared PostgreSQL image helper`);
    }
    if (source.includes(".Values.postgres.image")) {
      findings.push(`${name} bypasses the shared PostgreSQL image helper`);
    }
  }
  return findings;
}

export { releaseWorkflowFindings } from "./check-workflows.mjs";
import { releaseWorkflowFindings } from "./check-workflows.mjs";

function versionPattern(source) {
  return source.match(/grep -Eq \\\n\s+'([^']+)'/)?.[1];
}

export function versionBoundaryFindings(shared, installer, packagers) {
  const findings = [];
  const sharedPattern = versionPattern(shared);
  if (!sharedPattern || versionPattern(installer) !== sharedPattern) {
    findings.push("installer and package tooling do not share the release-version grammar");
  }
  if (!shared.includes('[ "${#version}" -le 63 ]')) {
    findings.push("shared release version has no Kubernetes-label-safe length bound");
  }
  if (!installer.includes('[ "${#candidate}" -le 63 ]')) {
    findings.push("installer release version has no matching length bound");
  }
  for (const source of [shared, installer]) {
    if (!source.includes("*[!0-9A-Za-z.-]* | *[!0-9A-Za-z]")) {
      findings.push("release version may end outside the Kubernetes label vocabulary");
    }
  }
  const installerSelection = installer.indexOf('select_release_version "$version"');
  const platformProbe = installer.indexOf('os="$(uname -s)"');
  if (!(installerSelection >= 0 && installerSelection < platformProbe)) {
    findings.push("explicit installer version is not validated before platform activity");
  }
  for (const [name, source, firstMutation] of packagers) {
    const validation = source.indexOf('sh scripts/release-version.sh "$version"');
    const mutation = source.indexOf(firstMutation);
    if (!(validation >= 0 && mutation > validation)) {
      findings.push(`${name} does not validate version before derived filesystem state`);
    }
  }
  return findings;
}

export function helmAcceptanceFindings(demo, clientPod, keycloakFixture) {
  const findings = [];
  for (const [name, marker] of [
    ["product coordinate", 'PRODUCT_IMAGE="ghcr.io/synveda/product:$IMAGE_TAG"'],
    ["Keycloak coordinate", 'KEYCLOAK_IMAGE="ghcr.io/synveda/keycloak:$IMAGE_TAG"'],
    [
      "CloudNativePG coordinate",
      'CNPG_IMAGE="ghcr.io/synveda/cnpg-postgres:17.11-synveda-$IMAGE_TAG"',
    ],
    [
      "product build",
      'bash scripts/build-kind-image.sh "$PRODUCT_IMAGE" deploy/compose/product/Dockerfile product',
    ],
    [
      "CloudNativePG build",
      'bash scripts/build-kind-image.sh "$CNPG_IMAGE" deploy/helm/postgres/Dockerfile cnpg-postgres',
    ],
    [
      "Keycloak build",
      'bash scripts/build-kind-image.sh "$KEYCLOAK_IMAGE" deploy/compose/keycloak/Dockerfile keycloak',
    ],
    [
      "kind image load",
      'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE" "$CNPG_IMAGE" "$KEYCLOAK_IMAGE"',
    ],
  ]) {
    if (!demo.includes(marker)) findings.push(`Helm acceptance ${name} has drifted`);
  }
  if (!clientPod.includes("image: ghcr.io/synveda/product:__IMAGE_TAG__")) {
    findings.push("Helm acceptance client does not use the chart product coordinate");
  }
  if (
    !clientPod.includes(
      '- name: SYNVEDA_INSECURE_DEVELOPMENT_HTTP\n          value: "true"',
    )
  ) {
    findings.push("Helm acceptance client does not explicitly admit its disposable HTTP origin");
  }
  if (!clientPod.includes("secretName: ops2-keycloak")) {
    findings.push("Helm acceptance client does not mount the Keycloak demo credential Secret");
  }
  for (const [name, marker] of [
    ["optimized Keycloak image", "image: ghcr.io/synveda/keycloak:__IMAGE_TAG__"],
    ["production-mode Keycloak command", 'args: ["start", "--optimized"]'],
    ["isolated Keycloak database", "name: keycloak-postgres"],
    ["file-only PostgreSQL owner credential", "POSTGRES_PASSWORD_FILE"],
    ["file-only Keycloak database credential", "KC_DB_PASSWORD_FILE"],
    ["realm supervisor", 'args: ["synveda-realm-supervise"]'],
    ["generation-fence reachability", "publishNotReadyAddresses: true"],
    [
      "exact issuer host",
      'KC_HOSTNAME, value: "http://keycloak.synveda-test.svc.cluster.local:8080"',
    ],
  ]) {
    if (!keycloakFixture.includes(marker)) {
      findings.push(`Helm acceptance Keycloak ${name} has drifted`);
    }
  }
  if ((keycloakFixture.match(/image: ghcr\.io\/synveda\/keycloak:__IMAGE_TAG__/g) ?? []).length !== 3) {
    findings.push("Helm acceptance does not use one Keycloak image for preparation, server and convergence");
  }
  const runtimeSecretVolume = keycloakFixture.match(
    /        - name: source-secrets\n          secret:\n            secretName: ops2-keycloak\n            defaultMode: 288\n            items:\n(?:              - \{[^\n]+\}\n)+(?=        - name: server-secrets)/,
  )?.[0] ?? "";
  for (const key of [
    "keycloak_database_password",
    "keycloak_admin_username",
    "keycloak_admin_password",
    "keycloak_convergence_admin_password",
    "keycloak_demo_admin_password",
    "keycloak_demo_approver_password",
    "keycloak_demo_member_password",
    "keycloak_demo_viewer_password",
  ]) {
    if (!runtimeSecretVolume.includes(`- { key: ${key}, path: ${key} }`)) {
      findings.push(`Helm acceptance Keycloak init cannot read ${key}`);
    }
  }
  if (runtimeSecretVolume.includes("postgres_owner_password")) {
    findings.push("Helm acceptance Keycloak init can read the PostgreSQL owner credential");
  }
  return findings;
}

function packageAndRenderChart(version) {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-release-parity-"));
  try {
    const first = join(scratch, "first");
    const second = join(scratch, "second");
    mkdirSync(first);
    mkdirSync(second);
    for (const output of [first, second]) {
      execFileSync("sh", ["scripts/package-chart.sh", version, output], {
        cwd: ROOT,
        stdio: "pipe",
      });
    }
    const name = `synveda-${version}.tgz`;
    const digests = "123456".split("").map((c) => `sha256:${c.repeat(64)}`);
    execFileSync("bash", ["scripts/package-release.sh", version, first, "0".repeat(40), ...digests], { cwd: ROOT, stdio: "pipe" });
    for (const mode of ["external", "cnpg"]) for (const identity of ["external", "packaged"]) {
      const args = ["template", "synveda", join(first, name), "-f", `deploy/helm/synveda/ci/${mode}-values.yaml`];
      if (identity === "packaged") args.push("-f", "deploy/helm/synveda/ci/packaged-keycloak-values.yaml");
      args.push("-f", join(first, `synveda-images-${version}.yaml`));
      if (mode === "cnpg") args.push("--api-versions", "postgresql.cnpg.io/v1", "-f", join(first, `synveda-cnpg-image-${version}.yaml`));
      const locked = execFileSync("helm", args, { cwd: ROOT, encoding: "utf8" });
      if (!locked.includes(`ghcr.io/synveda/product@${digests[0]}`)) throw new Error("release overlay lost immutable product image");
      if (identity === "packaged" && !locked.includes(`ghcr.io/synveda/keycloak@${digests[2]}`)) throw new Error("release overlay lost immutable Keycloak image");
      if (mode === "cnpg" && !locked.includes(`17.11-synveda-${version}@${digests[5]}`)) throw new Error("release overlay lost version-bearing CNPG digest");
    }
    const firstBytes = readFileSync(join(first, name));
    const secondBytes = readFileSync(join(second, name));
    if (!firstBytes.equals(secondBytes)) {
      throw new Error("two chart packages from the same source are not byte-identical");
    }
    const metadata = execFileSync("helm", ["show", "chart", join(first, name)], {
      cwd: ROOT,
      encoding: "utf8",
    });
    if (!metadata.includes(`version: ${version}`) || !metadata.includes(`appVersion: ${version}`)) {
      throw new Error("packaged chart metadata does not match the workspace version");
    }
    const rendered = execFileSync(
      "helm",
      [
        "template",
        "release-parity",
        join(first, name),
        "-f",
        "deploy/helm/synveda/ci/full-values.yaml",
      ],
      { cwd: ROOT, encoding: "utf8" },
    );
    if (!rendered.includes(`ghcr.io/synveda/product:${version}`)) {
      throw new Error("packaged external chart does not render the versioned product image");
    }
    if (rendered.includes("kind: Cluster\n") || rendered.includes("synveda-pg-superuser")) {
      throw new Error("packaged external chart unexpectedly owns a database provider");
    }
    const acceptance = execFileSync(
      "helm",
      [
        "template",
        "release-parity",
        join(first, name),
        "-f",
        "demos/fixtures/ops-2/values.yaml",
        "--api-versions",
        "postgresql.cnpg.io/v1",
      ],
      { cwd: ROOT, encoding: "utf8" },
    );
    for (const image of [
      `ghcr.io/synveda/product:${version}`,
      `ghcr.io/synveda/cnpg-postgres:17.11-synveda-${version}`,
    ]) {
      if (!acceptance.includes(image)) {
        throw new Error(`Helm acceptance values do not render ${image}`);
      }
    }

    // The accepted release vocabulary can place punctuation exactly at the
    // chart label's truncation boundary. Render that adversarial version so
    // the helper must remove an invalid terminal dot/hyphen after truncation.
    const edgeVersion = `1.2.3-${"a".repeat(48)}.b`;
    const edge = join(scratch, "label-edge");
    mkdirSync(edge);
    execFileSync("sh", ["scripts/package-chart.sh", edgeVersion, edge], {
      cwd: ROOT,
      stdio: "pipe",
    });
    const edgeRender = execFileSync(
      "helm",
      [
        "template",
        "release-parity",
        join(edge, `synveda-${edgeVersion}.tgz`),
        "-f",
        "deploy/helm/synveda/ci/full-values.yaml",
      ],
      { cwd: ROOT, encoding: "utf8" },
    );
    const chartLabels = [...edgeRender.matchAll(/^\s*helm\.sh\/chart:\s*(\S+)\s*$/gm)].map(
      ([, label]) => label,
    );
    if (
      chartLabels.length === 0 ||
      chartLabels.some(
        (label) =>
          label.length > 63 ||
          !/^[A-Za-z0-9](?:[-._A-Za-z0-9]*[A-Za-z0-9])?$/.test(label),
      )
    ) {
      throw new Error("packaged chart renders an invalid version label after truncation");
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

export function main() {
  const cargo = read("Cargo.toml");
  const version = workspaceVersion(cargo);
  const findings = [
    ...chartParityFindings(
      cargo,
      read("deploy/helm/synveda/Chart.yaml"),
      read("deploy/helm/synveda/values.yaml"),
      read("deploy/helm/synveda/templates/_helpers.tpl"),
      read("deploy/helm/synveda/templates/postgres-cluster.yaml"),
      read("deploy/helm/synveda/templates/install-job.yaml"),
    ),
    ...releaseWorkflowFindings(read(".github/workflows/release.yml")),
    ...versionBoundaryFindings(read("scripts/release-version.sh"), read("scripts/install.sh"), [
      ["Docker reference packager", read("scripts/package-release.sh"), 'stage="$outdir/synveda-reference-$version"'],
      ["plugin packager", read("scripts/package-plugin.sh"), 'stage="$outdir/plugin"'],
      ["chart packager", read("scripts/package-chart.sh"), 'mkdir -p "$outdir"'],
    ]),
    ...helmAcceptanceFindings(
      read("demos/ops-2-helm-install.sh"),
      read("demos/fixtures/ops-2/client-pod.yaml"),
      read("demos/fixtures/ops-2/keycloak.yaml"),
    ),
  ];
  if (!version) findings.push("workspace release version is missing");
  if (findings.length > 0) {
    for (const finding of findings) console.error(`FAIL ${finding}`);
    process.exitCode = 1;
    return;
  }
  packageAndRenderChart(version);
  console.log(
    `ok: release version ${version}, five deployment images plus one acceptance fixture, the reference bundle, chart package and GHCR product/CNPG defaults form one deterministic release plan`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}

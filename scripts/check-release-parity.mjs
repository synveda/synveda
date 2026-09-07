#!/usr/bin/env node
// CPR-45 / PR-01: deterministic release-to-Helm artifact parity. This gate
// packages and renders the chart locally, but never contacts Docker, a
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
  if (!image.includes("  repository: ghcr.io/synveda/gateway\n")) {
    findings.push("chart product image does not use the public GHCR repository");
  }
  if (!image.includes('  tag: ""\n')) {
    findings.push("chart product image tag does not default to appVersion");
  }
  const postgres = topLevelYamlBlock(values, "postgres");
  if (!postgres.includes('  image: ""\n')) {
    findings.push("chart PostgreSQL override is not empty by default");
  }
  const postgresHelper =
    '{{- default (printf "ghcr.io/synveda/enterprise-postgres:%s" .Chart.AppVersion) .Values.postgres.image -}}';
  if (!helpers.includes(postgresHelper)) {
    findings.push("chart PostgreSQL image does not default to the public appVersion coordinate");
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

export function releaseWorkflowFindings(source) {
  const findings = [];
  if (!source.includes("permissions:\n  contents: read\n\njobs:")) {
    findings.push("workflow default permissions are not read-only");
  }
  const imagesJob = source.slice(source.indexOf("  images:\n"), source.indexOf("\n  publish:\n"));
  const publishJob = source.slice(source.indexOf("  publish:\n"));
  if (!imagesJob.includes("    permissions:\n      contents: read\n      packages: write\n")) {
    findings.push("image job lacks scoped package-write permission");
  }
  if (!publishJob.includes("    permissions:\n      contents: write\n      packages: write\n")) {
    findings.push("publish job lacks scoped release/package-write permission");
  }
  const untrustedInput = "${{ inputs.version }}";
  if (
    source.split(untrustedInput).length - 1 !== 1 ||
    !source.includes(`INPUT_VERSION: ${untrustedInput}`)
  ) {
    findings.push("workflow-dispatch version is interpolated outside its environment boundary");
  }
  const versionBlock = stepBlock(source, "Resolve the version, and refuse a tag the crates disagree with");
  const inputRead = versionBlock.indexOf('version="$INPUT_VERSION"');
  const validation = versionBlock.indexOf('sh scripts/release-version.sh "$version"');
  const output = versionBlock.indexOf('echo "version=$version" >> "$GITHUB_OUTPUT"');
  if (!(inputRead >= 0 && validation > inputRead && output > validation)) {
    findings.push("workflow version is not validated before publication");
  }
  if (!versionBlock.includes('test "$(git rev-parse HEAD)" = "$TRIGGER_SHA"')) {
    findings.push("workflow does not bind checkout HEAD to the trigger SHA");
  }
  if (!source.includes('run: sh scripts/package-chart.sh "${{ needs.version.outputs.version }}" .')) {
    findings.push("workflow does not package the versioned Helm chart");
  }
  if (!source.includes("            synveda-*.tgz\n")) {
    findings.push("chart archive is absent from uploaded bundle artifacts");
  }
  const cnpg = stepBlock(source, "CloudNativePG PostgreSQL");
  for (const marker of [
    "          context: .\n",
    "          file: deploy/helm/postgres/Dockerfile\n",
    "          platforms: ${{ matrix.platform }}\n",
    "          push: ${{ needs.version.outputs.publish == 'true' }}\n",
    "          tags: ghcr.io/synveda/enterprise-postgres:${{ needs.version.outputs.version }}-${{ matrix.arch }}\n",
  ]) {
    if (!cnpg.includes(marker)) {
      findings.push(`CNPG release build is missing ${marker.trim()}`);
    }
  }
  if (!source.includes("for image in gateway postgres enterprise-postgres; do")) {
    findings.push("multi-architecture manifest join omits a release image");
  }
  if (!source.includes("sha256sum synveda-*.tar.gz synveda-*.tgz > SHA256SUMS")) {
    findings.push("release checksums omit the Helm chart");
  }
  if (!source.includes('"synveda-$version.tgz"; do') || !source.includes("all six assets present")) {
    findings.push("release asset inventory omits the Helm chart");
  }
  return findings;
}

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

export function helmAcceptanceFindings(demo, clientPod) {
  const findings = [];
  for (const [name, marker] of [
    ["product coordinate", 'PRODUCT_IMAGE="ghcr.io/synveda/gateway:$IMAGE_TAG"'],
    [
      "CloudNativePG coordinate",
      'CNPG_IMAGE="ghcr.io/synveda/enterprise-postgres:$IMAGE_TAG"',
    ],
    [
      "product build",
      'docker build -t "$PRODUCT_IMAGE" -f deploy/compose/gateway/Dockerfile .',
    ],
    [
      "CloudNativePG build",
      'docker build -t "$CNPG_IMAGE" -f deploy/helm/postgres/Dockerfile .',
    ],
    [
      "kind image load",
      'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE" "$CNPG_IMAGE"',
    ],
  ]) {
    if (!demo.includes(marker)) findings.push(`Helm acceptance ${name} has drifted`);
  }
  if (!clientPod.includes("image: ghcr.io/synveda/gateway:__IMAGE_TAG__")) {
    findings.push("Helm acceptance client does not use the chart product coordinate");
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
    for (const image of [
      `ghcr.io/synveda/gateway:${version}`,
      `ghcr.io/synveda/enterprise-postgres:${version}`,
    ]) {
      if (!rendered.includes(image)) throw new Error(`packaged chart does not render ${image}`);
    }
    const acceptance = execFileSync(
      "helm",
      [
        "template",
        "release-parity",
        join(first, name),
        "-f",
        "demos/fixtures/ops-2/values.yaml",
      ],
      { cwd: ROOT, encoding: "utf8" },
    );
    for (const image of [
      `ghcr.io/synveda/gateway:${version}`,
      `ghcr.io/synveda/enterprise-postgres:${version}`,
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
      ["release profile packager", read("scripts/package-release.sh"), 'stage="$outdir/synveda-profile-$version"'],
      ["plugin packager", read("scripts/package-plugin.sh"), 'stage="$outdir/plugin"'],
      ["chart packager", read("scripts/package-chart.sh"), 'mkdir -p "$outdir"'],
    ]),
    ...helmAcceptanceFindings(
      read("demos/ops-2-helm-install.sh"),
      read("demos/fixtures/ops-2/client-pod.yaml"),
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
    `ok: release version ${version}, chart package and GHCR product/CNPG defaults form one deterministic release plan`,
  );
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}

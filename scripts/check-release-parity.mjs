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

export function releaseWorkflowFindings(source) {
  const findings = [];
  const pins = JSON.parse(read(".github/action-pins.json"));
  for (const match of source.matchAll(/uses: ([^\s]+)(?: # ([^\n]+))?/g)) {
    const [repository, sha] = match[1].split("@");
    const ref = `${repository}@${match[2]}`;
    if (!/^[a-f0-9]{40}$/.test(sha ?? "") || pins[ref] !== sha && pins[`${repository}@${sha}`] !== sha) findings.push(`unreviewed action reference: ${repository}`);
  }
  for (const [ref, sha] of Object.entries(pins)) {
    source = source.replaceAll(`${ref.split("@")[0]}@${sha} # ${ref.split("@")[1]}`, ref);
  }
  if (!source.includes("permissions:\n  contents: read\n\njobs:")) {
    findings.push("workflow default permissions are not read-only");
  }
  const job = (name) => {
    const start = source.indexOf(`  ${name}:\n`);
    if (start < 0) return "";
    const rest = source.slice(start + 3 + name.length);
    const next = /^  [a-z][a-z-]*:\n/m.exec(rest);
    return source.slice(start, next ? start + 3 + name.length + next.index : source.length);
  };
  const imagesJob = job("images");
  const assemblyJob = job("assemble");
  const verificationJob = job("verify-images");
  const publishJob = job("publish");
  if (!imagesJob.startsWith("  images:\n    needs: version\n")) {
    findings.push("release image plan does not depend exactly on resolved version");
  }
  if (!assemblyJob.startsWith("  assemble:\n    needs: [version, binaries, bundles, images]\n")) {
    findings.push("release assembly does not await the exact artifact producers");
  }
  if (!publishJob.startsWith("  publish:\n    needs: [version, assemble, verify-images]\n")) {
    findings.push("release announcement does not await native anonymous verification");
  }
  for (const block of [imagesJob, assemblyJob, verificationJob, publishJob]) {
    if (/^    (continue-on-error|if):/m.test(block)) {
      findings.push("release job may mask a failed prerequisite");
    }
  }
  for (const block of [imagesJob, assemblyJob]) {
    if (!block.includes("    permissions:\n      contents: read\n      packages: write\n")) {
      findings.push("image/assembly job lacks scoped package-write permission");
    }
  }
  if (!publishJob.includes("    permissions:\n      contents: write\n") || publishJob.includes("packages: write")) {
    findings.push("publish job must have only scoped release-write permission");
  }
  if (!verificationJob.includes("    permissions:\n      contents: read\n") ||
      verificationJob.includes("packages: write") || verificationJob.includes("docker/login-action") ||
      verificationJob.includes("secrets.") ||
      !verificationJob.includes("    needs: [version, assemble]\n") ||
      !verificationJob.includes('mktemp -d "$RUNNER_TEMP/synveda-anonymous-docker.XXXXXX"') ||
      !verificationJob.includes('echo "DOCKER_CONFIG=$directory" >> "$GITHUB_ENV"')) {
    findings.push("verification must use fresh anonymous Docker credentials and assembled artifacts");
  }
  const verify = stepBlock(verificationJob, "Pull and execute the released images anonymously");
  if (!verify.includes("        if: needs.version.outputs.publish == 'true'\n") ||
      !verify.includes("node scripts/verify-release-images.mjs") ||
      !verify.includes('"$RUNNER_TEMP/synveda-reference-$VERSION" "$PLATFORM"') ||
      !verify.includes('"$VERSION" "$SOURCE_SHA" "release-images-${{ matrix.arch }}.json"') ||
      verify.includes("continue-on-error") ||
      !verificationJob.includes("run: sha256sum --check SHA256SUMS") ||
      !verificationJob.includes("name: release-verification-${{ matrix.arch }}")) {
    findings.push("native image verification must bind checksums, version/source and its report");
  }
  if (!assemblyJob.includes("          name: release-assets\n") ||
      !verificationJob.includes("          name: release-assets\n") ||
      !publishJob.includes("          name: release-assets\n") ||
      !publishJob.includes("          pattern: release-verification-*\n") ||
      !publishJob.includes("test -s release-images-amd64.json") ||
      !publishJob.includes("test -s release-images-arm64.json") ||
      !publishJob.includes("sha256sum release-images-*.json release-docker-*.json release-kubernetes-*.json >> SHA256SUMS")) {
    findings.push("announcement must carry the assembled assets and both checksummed reports");
  }
  if (!assemblyJob.includes("          path: |\n            assets/SHA256SUMS\n            assets/synveda-*.tar.gz\n            assets/synveda-*.tgz\n            assets/synveda-*.yaml\n") ||
      assemblyJob.includes("path: assets/*") || publishJob.includes("assets/*") ||
      !publishJob.includes('test -f "$asset" && test ! -L "$asset"')) {
    findings.push("release uploads must select only the regular packaged asset inventory");
  }
  const untrustedInput = "${{ inputs.version }}";
  const qualification = stepBlock(verificationJob, "Qualify the exact Docker and chart artifacts");
  if (!qualification.includes("if: needs.version.outputs.publish == 'true'") ||
      qualification.includes("continue-on-error") ||
      !qualification.includes("node scripts/qualify-release.mjs") ||
      !qualification.includes("node scripts/qualify-kubernetes-release.mjs") ||
      !qualification.includes('cmp "assets/synveda-$VERSION.tgz" "anonymous-chart/synveda-$VERSION.tgz"') ||
      !publishJob.includes('test -s "release-docker-$arch.json"') ||
      !publishJob.includes('test -s "release-kubernetes-$arch.json"')) {
    findings.push("release must require exact Docker/chart qualification and anonymous OCI retrieval");
  }
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
  const nativeArchitectures = [
    "          - arch: amd64\n            platform: linux/amd64\n            runs-on: ubuntu-latest\n",
    "          - arch: arm64\n            platform: linux/arm64\n            runs-on: ubuntu-24.04-arm\n",
  ];
  if ((imagesJob.match(/^          - arch:/gm) ?? []).length !== nativeArchitectures.length) {
    findings.push("release image matrix is not the exact two-architecture contract");
  }
  for (const architecture of nativeArchitectures) {
    if (!imagesJob.includes(architecture) || !verificationJob.includes(architecture)) {
      findings.push(`release image matrix is missing ${architecture.trim().split("\n")[0]}`);
    }
  }
  if (
    !imagesJob.includes("    runs-on: ${{ matrix.runs-on }}\n") ||
    imagesJob.includes("docker/setup-qemu-action") ||
    !verificationJob.includes("    runs-on: ${{ matrix.runs-on }}\n") ||
    verificationJob.includes("docker/setup-qemu-action") ||
    (verificationJob.match(/^          - arch:/gm) ?? []).length !== 2
  ) {
    findings.push("release image matrix is not bound to native runners");
  }
  const releaseImages = [
    ["The product image", "deploy/compose/product/Dockerfile", "product", null, ""],
    ["Postgres", "deploy/compose/postgres/Dockerfile", "postgres", "reference", ""],
    [
      "CloudNativePG PostgreSQL",
      "deploy/helm/postgres/Dockerfile",
      "cnpg-postgres",
      null,
      "17.11-synveda-",
    ],
    ["Bundled Keycloak", "deploy/compose/keycloak/Dockerfile", "keycloak", null, ""],
    ["Reference proxy", "deploy/compose/proxy/Dockerfile", "proxy", null, ""],
    [
      "Browser acceptance fixture",
      "deploy/compose/product/Dockerfile",
      "browser-acceptance",
      "browser-acceptance",
      "",
    ],
  ];
  if (source.split("uses: docker/build-push-action@v6").length - 1 !== releaseImages.length) {
    findings.push("release image build set is not five deployment images plus one fixture");
  }
  for (const [name, dockerfile, repository, target, tagPrefix] of releaseImages) {
    if (imagesJob.split(`      - name: ${name}\n`).length - 1 !== 1) {
      findings.push(`${name} release build is not declared exactly once`);
    }
    const block = stepBlock(imagesJob, name);
    const stepKeys = [
      ...block.matchAll(/^        ([a-z][a-z0-9-]*):[ \t]*(.*?)[ \t]*$/gm),
    ];
    if (
      stepKeys.length !== 2 ||
      stepKeys[0][1] !== "uses" ||
      stepKeys[0][2] !== "docker/build-push-action@v6" ||
      stepKeys[1][1] !== "with" ||
      stepKeys[1][2] !== ""
    ) {
      findings.push(`${name} release build does not use the exact build action boundary`);
    }
    const semanticInputs = new Map([
      ["context", "."],
      ["file", dockerfile],
      ...(target === null ? [] : [["target", target]]),
      ["platforms", "${{ matrix.platform }}"],
      ["push", "${{ needs.version.outputs.publish == 'true' }}"],
      ["provenance", "mode=max"],
      ["sbom", "true"],
      ["labels", "|"],
      [
        "tags",
        `ghcr.io/synveda/${repository}:${tagPrefix}\${{ needs.version.outputs.version }}-\${{ matrix.arch }}`,
      ],
    ]);
    for (const label of [
      "org.opencontainers.image.source=https://github.com/${{ github.repository }}",
      "org.opencontainers.image.revision=${{ github.sha }}",
      "org.opencontainers.image.version=${{ needs.version.outputs.version }}",
    ]) {
      if (!block.includes(`            ${label}\n`)) findings.push(`${name}: missing release label`);
    }
    const allowedInputs = new Set([
      ...semanticInputs.keys(),
      "cache-from",
      "cache-to",
    ]);
    const actualInputs = [
      ...block.matchAll(/^          ([a-z][a-z0-9-]*):[ \t]*(.*?)[ \t]*$/gm),
    ];
    const actualKeys = actualInputs.map(([, key]) => key);
    if (
      actualKeys.some((key) => !allowedInputs.has(key)) ||
      new Set(actualKeys).size !== actualKeys.length
    ) {
      findings.push(`${name} release build has an unexpected or duplicate input`);
    }
    for (const [key, value] of semanticInputs) {
      const matches = actualInputs.filter(([, actualKey]) => actualKey === key);
      if (matches.length !== 1 || matches[0][2] !== value) {
        findings.push(`${name} release build has invalid ${key}`);
      }
    }
  }
  const join = stepBlock(source, "Join the per-architecture image tags");
  const expectedJoin = [
    "      - name: Join the per-architecture image tags",
    "        if: needs.version.outputs.publish == 'true'",
    "        run: |",
    "          set -euo pipefail",
    '          version="${{ needs.version.outputs.version }}"',
    "          for image in product postgres keycloak proxy browser-acceptance; do",
    "            docker buildx imagetools create \\",
    '              --tag "ghcr.io/synveda/$image:$version" \\',
    '              "ghcr.io/synveda/$image:$version-amd64" \\',
    '              "ghcr.io/synveda/$image:$version-arm64"',
    '            docker buildx imagetools inspect "ghcr.io/synveda/$image:$version"',
    "          done",
    '          cnpg_tag="17.11-synveda-$version"',
    "          docker buildx imagetools create \\",
    '            --tag "ghcr.io/synveda/cnpg-postgres:$cnpg_tag" \\',
    '            "ghcr.io/synveda/cnpg-postgres:$cnpg_tag-amd64" \\',
    '            "ghcr.io/synveda/cnpg-postgres:$cnpg_tag-arm64"',
    '          docker buildx imagetools inspect "ghcr.io/synveda/cnpg-postgres:$cnpg_tag"',
  ].join("\n");
  if (
    assemblyJob.split("      - name: Join the per-architecture image tags\n").length - 1 !== 1 ||
    join !== expectedJoin
  ) {
    findings.push("multi-architecture manifest join is not the exact publish-bound plan");
  }
  const packageReference = stepBlock(source, "Package the digest-bound Docker reference");
  const packagePosition = assemblyJob.indexOf(
    "      - name: Package the digest-bound Docker reference\n",
  );
  const joinPosition = assemblyJob.indexOf(
    "      - name: Join the per-architecture image tags\n",
  );
  const inventoryPosition = assemblyJob.indexOf("      - name: Every release asset is present\n");
  const checksumPosition = assemblyJob.indexOf("      - name: Checksums\n");
  if (
    assemblyJob.split("      - name: Package the digest-bound Docker reference\n").length - 1 !==
      1 ||
    !(joinPosition >= 0 && packagePosition > joinPosition && inventoryPosition > packagePosition) ||
    checksumPosition <= inventoryPosition ||
    !packageReference.includes('SOURCE_SHA: ${{ github.sha }}') ||
    !packageReference.includes(
      'scripts/package-release.sh "$version" assets "$SOURCE_SHA"',
    ) ||
    !packageReference.includes(
      'if ! docker buildx imagetools inspect --raw "$image" > "$output"; then',
    ) ||
    !packageReference.includes('if [ ! -s "$output" ]; then') ||
    !packageReference.includes('digest=$(sha256sum "$output" | awk')
  ) {
    findings.push("reference package is not digest-bound after image join and before inventory");
  }
  if (!source.includes("sha256sum synveda-*.tar.gz synveda-*.tgz synveda-*.yaml > SHA256SUMS")) {
    findings.push("release checksums omit the Helm chart or immutable image overlays");
  }
  const ociChart = stepBlock(source, "Publish and pull the OCI chart");
  if (!ociChart.includes("        if: needs.version.outputs.publish == 'true'") ||
      !ociChart.includes("helm registry login ghcr.io") ||
      !ociChart.includes("--password-stdin") ||
      !ociChart.includes('helm push "assets/synveda-$version.tgz" oci://ghcr.io/synveda/charts') ||
      !ociChart.includes('helm pull oci://ghcr.io/synveda/charts/synveda --version "$version"') ||
      !ociChart.includes('cmp "assets/synveda-$version.tgz" "pulled-chart/synveda-$version.tgz"')) {
    findings.push("OCI chart publication must be tag-only and pull-verified");
  }
  if (
    !source.includes('"synveda-reference-$version.tar.gz"') ||
    !source.includes('"synveda-$version.tgz"; do') ||
    !source.includes("all six release archives are present")
  ) {
    findings.push("release asset inventory omits the reference bundle or Helm chart");
  }
  const release = stepBlock(source, "Publish");
  const releaseKeys = [
    ...release.matchAll(/^        ([a-z][a-z0-9-]*):[ \t]*(.*?)[ \t]*$/gm),
  ];
  const continuation = String.fromCharCode(92);
  const releaseCommand = [
    '          gh release create "${GITHUB_REF_NAME}" ' + continuation,
    '            --title "Synveda ${GITHUB_REF_NAME}" ' + continuation,
    "            --notes-file notes.md " + continuation,
    '            "${release_assets[@]}"',
  ].join("\n");
  if (
    publishJob.split("      - name: Publish\n").length - 1 !== 1 ||
    releaseKeys.length !== 3 ||
    releaseKeys[0][1] !== "if" ||
    releaseKeys[0][2] !== "needs.version.outputs.publish == 'true'" ||
    releaseKeys[1][1] !== "env" ||
    releaseKeys[1][2] !== "" ||
    releaseKeys[2][1] !== "run" ||
    releaseKeys[2][2] !== "|" ||
    !release.includes("          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n") ||
    !release.includes("          GH_REPO: ${{ github.repository }}\n") ||
    !release.includes("installed \\`synveda-compose\\` launcher") ||
    release.includes("installed `synveda-compose` launcher") ||
    !release.includes("SYNVEDA_VERSION=${GITHUB_REF_NAME} sh synveda-install.sh") ||
    !release.trimEnd().endsWith(releaseCommand)
  ) {
    findings.push("GitHub Release publication is not the exact failure-propagating boundary");
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

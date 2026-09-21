import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  chartParityFindings,
  helmAcceptanceFindings,
  releaseWorkflowFindings,
  versionBoundaryFindings,
  workspaceVersion,
} from "./check-release-parity.mjs";

const ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const read = (path) => readFileSync(join(ROOT, path), "utf8");

const currentChartInputs = () => [
  read("Cargo.toml"),
  read("deploy/helm/synveda/Chart.yaml"),
  read("deploy/helm/synveda/values.yaml"),
  read("deploy/helm/synveda/templates/_helpers.tpl"),
  read("deploy/helm/synveda/templates/postgres-cluster.yaml"),
  read("deploy/helm/synveda/templates/install-job.yaml"),
];

const currentVersionInputs = () => [
  read("scripts/release-version.sh"),
  read("scripts/install.sh"),
  [
    [
      "Docker reference packager",
      read("scripts/package-release.sh"),
      'stage="$outdir/synveda-reference-$version"',
    ],
    ["plugin packager", read("scripts/package-plugin.sh"), 'stage="$outdir/plugin"'],
    ["chart packager", read("scripts/package-chart.sh"), 'mkdir -p "$outdir"'],
  ],
];

function validateVersion(version) {
  return spawnSync("sh", ["scripts/release-version.sh", version], {
    cwd: ROOT,
    encoding: "utf8",
  });
}

test("release versions form one bounded label- and OCI-safe vocabulary", () => {
  for (const version of [
    "0.2.0",
    "0.0.0-dev",
    "1.2.3-rc.1",
    "10.20.30-alpha-1.0",
    "9999999999999999999.0.0",
    `1.2.3-${"a".repeat(57)}`,
  ]) {
    const result = validateVersion(version);
    assert.equal(result.status, 0, `${version}: ${result.stderr}`);
    assert.equal(result.stdout, "");
  }

  for (const version of [
    "",
    "v0.2.0",
    "01.2.3",
    "1.02.3",
    "1.2.03",
    "1.2",
    "1.2.3-",
    "1.2.3-01",
    "1.2.3-rc-",
    "1.2.3+build",
    "18446744073709551616.0.0",
    "../0.2.0",
    "x/../../victim",
    "1.2.3&touch-owned",
    "1.2.3\\replacement",
    "1.2.3\n0.0.0",
    `1.2.3-${"a".repeat(58)}`,
    `1.2.3-${"a".repeat(121)}`,
  ]) {
    const result = validateVersion(version);
    assert.equal(result.status, 64, version);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /^release-version: expected /);
    assert.doesNotMatch(result.stderr, /victim|touch-owned|replacement/);
  }
});

test("invalid versions are refused before packager or installer mutation", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-release-version-test-"));
  try {
    const output = join(scratch, "out");
    const victim = join(scratch, "victim");
    mkdirSync(join(output, "synveda-reference-x"), { recursive: true });
    mkdirSync(join(output, "plugin"), { recursive: true });
    mkdirSync(victim);
    writeFileSync(join(victim, "sentinel"), "preserve\n");
    writeFileSync(join(output, "plugin", "sentinel"), "preserve\n");

    const digest = `sha256:${"1".repeat(64)}`;
    for (const [interpreter, script, extra] of [
      ["bash", "scripts/package-release.sh", ["0".repeat(40), ...Array(6).fill(digest)]],
      ["bash", "scripts/package-plugin.sh", []],
      ["sh", "scripts/package-chart.sh", []],
    ]) {
      const result = spawnSync(interpreter, [script, "x/../../victim", output, ...extra], {
        cwd: ROOT,
        encoding: "utf8",
      });
      assert.equal(result.status, 64, `${script}: ${result.stderr}`);
      assert.equal(readFileSync(join(victim, "sentinel"), "utf8"), "preserve\n");
      assert.equal(
        readFileSync(join(output, "plugin", "sentinel"), "utf8"),
        "preserve\n",
      );
    }

    const home = join(scratch, "home");
    const bin = join(scratch, "bin");
    const install = spawnSync("sh", ["scripts/install.sh"], {
      cwd: ROOT,
      encoding: "utf8",
      env: {
        ...process.env,
        SYNVEDA_BASE_URL: `file://${scratch}/assets`,
        SYNVEDA_BIN: bin,
        SYNVEDA_HOME: home,
        SYNVEDA_VERSION: "x/../../victim",
      },
    });
    assert.equal(install.status, 1, install.stderr);
    assert.match(install.stderr, /^install: invalid release version;/);
    assert.equal(existsSync(home), false);
    assert.equal(existsSync(bin), false);
    assert.equal(readFileSync(join(victim, "sentinel"), "utf8"), "preserve\n");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("release workflow binds the chart and digest-addressed reference images", () => {
  const current = read(".github/workflows/release.yml");
  assert.deepEqual(releaseWorkflowFindings(current), []);
  for (const [index, mutant] of [
    current.replace("needs: [version, assemble, verify-images]", "needs: [version, assemble]"),
    current.replace("node scripts/verify-release-images.mjs", "echo skipped"),
    current.replace("node scripts/qualify-release.mjs", "echo skipped"),
    current.replace("node scripts/qualify-kubernetes-release.mjs", "echo skipped"),
    current.replace('test -s "release-docker-$arch.json"', "true"),
    current.replace('test -s "release-kubernetes-$arch.json"', "true"),
    current.replace('cmp "assets/synveda-$VERSION.tgz" "anonymous-chart/synveda-$VERSION.tgz"', "true"),
    current.replace("- name: Qualify the exact Docker and chart artifacts", "- name: Qualify the exact Docker and chart artifacts\n        continue-on-error: true"),
    current.replace("org.opencontainers.image.revision=${{ github.sha }}", "org.opencontainers.image.revision=old"),
    current.replace('mktemp -d "$RUNNER_TEMP/synveda-anonymous-docker.XXXXXX"', 'echo /home/runner/.docker'),
    current.replace("test -s release-images-arm64.json", "true"),
    current.replace("GH_REPO: ${{ github.repository }}", "GH_REPO: another/repository"),
    current.replace('"${release_assets[@]}"\n', 'assets/*\n'),
    current.replace('test -f "$asset" && test ! -L "$asset"', "true"),
    current.replace("          path: |\n            assets/SHA256SUMS\n            assets/synveda-*.tar.gz\n            assets/synveda-*.tgz\n            assets/synveda-*.yaml\n", "          path: assets/*\n"),
    current.replace('version="$INPUT_VERSION"', 'version="${{ inputs.version }}"'),
    current.replace('sh scripts/release-version.sh "$version"', "true"),
    current.replace("permissions:\n  contents: read\n\nconcurrency:", "permissions:\n  contents: write\n\nconcurrency:"),
    current.replace("file: deploy/helm/postgres/Dockerfile", "file: deploy/compose/postgres/Dockerfile"),
    current.replace(
      "file: deploy/compose/keycloak/Dockerfile",
      "file: deploy/compose/product/Dockerfile",
    ),
    current.replace(
      "ghcr.io/synveda/proxy:${{ needs.version.outputs.version }}-${{ matrix.arch }}",
      "ghcr.io/synveda/keycloak:${{ needs.version.outputs.version }}-${{ matrix.arch }}",
    ),
    current.replace("- name: Bundled Keycloak", "- name: Omitted Keycloak"),
    current.replace('node scripts/release-registries.mjs preflight', 'echo preflight-skipped'),
    current.replace(
      "          - arch: arm64\n            platform: linux/arm64\n            runs-on: ubuntu-24.04-arm\n",
      "",
    ),
    current.replace("platform: linux/arm64", "platform: linux/amd64"),
    current.replace(
      "          file: deploy/compose/keycloak/Dockerfile\n          platforms:",
      "          file: deploy/compose/keycloak/Dockerfile\n          target: builder\n          platforms:",
    ),
    current.replace(
      "          file: deploy/compose/product/Dockerfile\n          platforms:",
      "          file: deploy/compose/product/Dockerfile\n          target: build\n          platforms:",
    ),
    current.replace(
      "          file: deploy/helm/postgres/Dockerfile\n          platforms:",
      "          file: deploy/helm/postgres/Dockerfile\n          build-args: CNPG_BASE=evil.example/postgres:latest\n          platforms:",
    ),
    current.replace(
      "      - name: Bundled Keycloak\n",
      "      - name: Bundled Keycloak\n        if: false\n",
    ),
    current.replace("  images:\n    needs: version\n", "  images:\n"),
    current.replace(
      "  images:\n    needs: version\n",
      "  images:\n    needs: version\n    continue-on-error: true\n",
    ),
    current.replace(
      "  images:\n    needs: version\n",
      "  images:\n    needs: version\n    if: false\n",
    ),
    current.replace(
      "    needs: [version, binaries, bundles, images]",
      "    needs: [version, binaries, bundles]",
    ),
    current.replace(
      "  assemble:\n    needs: [version, binaries, bundles, images]\n",
      "  assemble:\n    needs: [version, binaries, bundles, images]\n    if: always()\n",
    ),
    current.replace('node scripts/release-registries.mjs assemble', 'echo assembly-skipped'),
    current.replace('"$VERSION" "$SOURCE_SHA" "$PUBLISH"', '"$VERSION" "$SOURCE_SHA" "true"'),
    current.replace('node scripts/release-registries.mjs package', 'echo packaging-skipped'),
    current.replace('docker.io/${{ needs.version.outputs.dockerhub_namespace }}/product:', 'docker.io/assumed-owner/product:'),
    current.replace('"assets/synveda-registry-images-$VERSION.json" assets', '"unverified.json" assets'),
    current.replace('subject-path: assets/SHA256SUMS', 'subject-path: assets/unrelated'),
    current.replace('--source-ref "$GITHUB_REF" --source-digest "$SOURCE_SHA"', '--source-ref refs/heads/main'),
    current.replace('password: ${{ secrets.DOCKERHUB_TOKEN }}', 'password: hardcoded'),
    current.replace("environment: ${{ needs.version.outputs.publish == 'true' && 'release' || 'release-dry-run' }}", 'environment: unprotected'),
    current.replace(
      "      - name: Package the digest-bound Docker reference\n",
      "      - name: Package the Docker reference too early\n",
    ),
    current.replace("sha256sum synveda-*.tar.gz synveda-*.tgz", "sha256sum synveda-*.tar.gz"),
    current.replace("installed \\`synveda-compose\\` launcher", "installed `synveda-compose` launcher"),
    current.replace(
      "SYNVEDA_VERSION=${GITHUB_REF_NAME} sh synveda-install.sh",
      "sh synveda-install.sh",
    ),
    current.replace('"synveda-$version.tgz"; do', '"synveda-plugin-$version.tar.gz"; do'),
    current.replace("          provenance: mode=max", "          provenance: false"),
    current.replace("          sbom: true", "          sbom: false"),
    current.replace(
      "      - name: Publish and pull the OCI chart\n        if: needs.version.outputs.publish == 'true'\n",
      "      - name: Publish and pull the OCI chart\n",
    ),
    current.replace('cmp "assets/synveda-$version.tgz" "pulled-chart/synveda-$version.tgz"', "true"),
    current.replace(
      "      - name: Publish\n        if: needs.version.outputs.publish == 'true'\n",
      "      - name: Publish\n        if: false\n",
    ),
    current.replace(
      '          gh release create "${GITHUB_REF_NAME}" ',
      "          true ",
    ),
  ].entries()) {
    assert.notEqual(mutant, current, `mutation ${index} did not apply`);
    assert.ok(releaseWorkflowFindings(mutant).length > 0, `mutant ${index}`);
  }
});

test("publication uploads exactly the release files despite checkout asset directories", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-release-publication-"));
  const version = "0.4.0";
  const names = [
    "SHA256SUMS", "SHA256SUMS.sigstore.json", `synveda-registry-images-${version}.json`, `synveda-${version}-darwin-arm64.tar.gz`,
    `synveda-${version}-linux-x86_64.tar.gz`, `synveda-console-${version}.tar.gz`,
    `synveda-reference-${version}.tar.gz`, `synveda-plugin-${version}.tar.gz`,
    `synveda-${version}.tgz`, `synveda-cnpg-image-${version}.yaml`,
    `synveda-images-${version}.yaml`,
    ...["amd64", "arm64"].flatMap((arch) => ["images", "docker", "kubernetes"].map((report) => `release-${report}-${arch}.json`)),
  ];
  try {
    mkdirSync(join(scratch, "bin"));
    mkdirSync(join(scratch, "assets", "brand"), { recursive: true });
    mkdirSync(join(scratch, "assets", "product"));
    writeFileSync(join(scratch, "assets", "brand", "logo.svg"), "brand");
    writeFileSync(join(scratch, "assets", "synveda-9.9.9.tgz"), "unrelated version");
    for (const name of names) writeFileSync(join(scratch, "assets", name), "fixture");
    const capture = join(scratch, "arguments");
    writeFileSync(join(scratch, "bin", "gh"), '#!/bin/sh\nprintf "%s\\n" "$@" > "$CAPTURE"\nexit "${GH_EXIT_CODE:-0}"\n', { mode: 0o755 });
    const workflow = read(".github/workflows/release.yml");
    const start = workflow.indexOf("          release_assets=(\n");
    assert.ok(start > 0);
    const command = `version=${version}\n${workflow.slice(start).replace(/^ {10}/gm, "")}`;
    const run = (extra = {}) => spawnSync("bash", ["-euo", "pipefail", "-c", command], {
      cwd: scratch, encoding: "utf8",
      env: { ...process.env, PATH: `${join(scratch, "bin")}:${process.env.PATH}`, GITHUB_REF_NAME: `v${version}`, CAPTURE: capture, ...extra },
    });
    assert.equal(run().status, 0);
    assert.deepEqual(readFileSync(capture, "utf8").trim().split("\n"), [
      "release", "create", `v${version}`, "--title", `Synveda v${version}`,
      "--notes-file", "notes.md", ...names.map((name) => `assets/${name}`),
    ]);
    assert.equal(run({ GH_EXIT_CODE: "55" }).status, 55, "upload failures must propagate");
    rmSync(capture);
    const required = join(scratch, "assets", names[1]);
    rmSync(required);
    assert.equal(run().status, 1, "missing archive must fail before publication");
    assert.equal(existsSync(capture), false);
    mkdirSync(required);
    assert.equal(run().status, 1, "directory must fail before publication");
    assert.equal(existsSync(capture), false);
    rmSync(required, { recursive: true });
    symlinkSync(join(scratch, "assets", "SHA256SUMS"), required);
    assert.equal(run().status, 1, "symlink must fail before publication");
    assert.equal(existsSync(capture), false);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("workspace and chart default to one versioned GHCR image pair", () => {
  const current = currentChartInputs();
  const version = workspaceVersion(current[0]);
  assert.ok(version);
  assert.equal(validateVersion(version).status, 0);
  assert.deepEqual(chartParityFindings(...current), []);

  const mutants = [
    current.with(
      1,
      current[1].replace(
        `appVersion: "${version}"`,
        `appVersion: "${version}-mismatch"`,
      ),
    ),
    current.with(
      2,
      current[2].replace("repository: ghcr.io/synveda/product", "repository: synveda/product"),
    ),
    current.with(2, current[2].replace('  image: ""', "  image: latest")),
    current.with(
      3,
      current[3].replace(
        "ghcr.io/synveda/cnpg-postgres:17.11-synveda-%s",
        "synveda/postgres:17-%s",
      ),
    ),
    current.with(3, current[3].replace('trimAll "-._"', 'trimSuffix "-"')),
    current.with(
      4,
      current[4].replace(
        'include "synveda.postgresImage" .',
        ".Values.postgres.image",
      ),
    ),
  ];
  for (const mutant of mutants) assert.ok(chartParityFindings(...mutant).length > 0);
});

test("all local release consumers validate before deriving filesystem state", () => {
  const current = currentVersionInputs();
  assert.deepEqual(versionBoundaryFindings(...current), []);

  const installer = current[1].replace("[ \"${#candidate}\" -le 63 ]", "true");
  assert.ok(versionBoundaryFindings(current[0], installer, current[2]).length > 0);
  const terminal = current[1].replace(
    "*[!0-9A-Za-z.-]* | *[!0-9A-Za-z]",
    "*[!0-9A-Za-z.-]*",
  );
  assert.ok(versionBoundaryFindings(current[0], terminal, current[2]).length > 0);

  const packagers = current[2].map((entry, index) =>
    index === 0
      ? [entry[0], entry[1].replace('sh scripts/release-version.sh "$version"', "true"), entry[2]]
      : entry,
  );
  assert.ok(versionBoundaryFindings(current[0], current[1], packagers).length > 0);
});

test("kind acceptance builds and loads the chart's exact image coordinates", () => {
  const demo = read("demos/ops-2-helm-install.sh");
  const client = read("demos/fixtures/ops-2/client-pod.yaml");
  const keycloak = read("demos/fixtures/ops-2/keycloak.yaml");
  assert.deepEqual(helmAcceptanceFindings(demo, client, keycloak), []);

  for (const [demoMutant, clientMutant, keycloakMutant] of [
    [
      demo.replace("ghcr.io/synveda/product:$IMAGE_TAG", "synveda/product:$IMAGE_TAG"),
      client,
      keycloak,
    ],
    [
      demo.replace(
        "ghcr.io/synveda/cnpg-postgres:17.11-synveda-$IMAGE_TAG",
        "synveda/cnpg-postgres:17",
      ),
      client,
      keycloak,
    ],
    [
      demo.replace(
        'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE" "$CNPG_IMAGE" "$KEYCLOAK_IMAGE"',
        'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE"',
      ),
      client,
      keycloak,
    ],
    [
      demo.replace("ghcr.io/synveda/keycloak:$IMAGE_TAG", "synveda/keycloak:$IMAGE_TAG"),
      client,
      keycloak,
    ],
    [demo, client.replace("ghcr.io/synveda/product", "synveda/product"), keycloak],
    [
      demo,
      client.replace(
        "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
        "SYNVEDA_INSECURE_DEVELOPMENT_HTTP_REMOVED",
      ),
      keycloak,
    ],
    [
      demo,
      client.replace("secretName: ops2-keycloak", "secretName: missing-keycloak"),
      keycloak,
    ],
    [
      demo,
      client,
      keycloak.replace('args: ["synveda-realm-supervise"]', 'args: ["start-dev"]'),
    ],
    [
      demo,
      client,
      keycloak.replace('args: ["start", "--optimized"]', 'args: ["start-dev"]'),
    ],
    [
      demo,
      client,
      keycloak.replace("KC_DB_PASSWORD_FILE", "KC_DB_PASSWORD"),
    ],
    [
      demo,
      client,
      keycloak.replace(
        "              - { key: keycloak_demo_viewer_password, path: keycloak_demo_viewer_password }\n        - name: server-secrets",
        "              - { key: postgres_owner_password, path: postgres_owner_password }\n        - name: server-secrets",
      ),
    ],
    [
      demo,
      client,
      keycloak.replace("publishNotReadyAddresses: true", "publishNotReadyAddresses: false"),
    ],
  ]) {
    assert.ok(helmAcceptanceFindings(demoMutant, clientMutant, keycloakMutant).length > 0);
  }
});

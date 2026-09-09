import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
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
      "release profile packager",
      read("scripts/package-release.sh"),
      'stage="$outdir/synveda-profile-$version"',
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
    mkdirSync(join(output, "synveda-profile-x"), { recursive: true });
    mkdirSync(join(output, "plugin"), { recursive: true });
    mkdirSync(victim);
    writeFileSync(join(victim, "sentinel"), "preserve\n");
    writeFileSync(join(output, "plugin", "sentinel"), "preserve\n");

    for (const [interpreter, script] of [
      ["bash", "scripts/package-release.sh"],
      ["bash", "scripts/package-plugin.sh"],
      ["sh", "scripts/package-chart.sh"],
    ]) {
      const result = spawnSync(interpreter, [script, "x/../../victim", output], {
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

test("release workflow scopes authority and binds the chart plus five images", () => {
  const current = read(".github/workflows/release.yml");
  assert.deepEqual(releaseWorkflowFindings(current), []);
  for (const [index, mutant] of [
    current.replace('version="$INPUT_VERSION"', 'version="${{ inputs.version }}"'),
    current.replace('sh scripts/release-version.sh "$version"', "true"),
    current.replace("permissions:\n  contents: read\n\njobs:", "permissions:\n  contents: write\n\njobs:"),
    current.replace("file: deploy/helm/postgres/Dockerfile", "file: deploy/compose/postgres/Dockerfile"),
    current.replace(
      "file: deploy/compose/keycloak/Dockerfile",
      "file: deploy/compose/gateway/Dockerfile",
    ),
    current.replace(
      "tags: ghcr.io/synveda/proxy:${{ needs.version.outputs.version }}-${{ matrix.arch }}",
      "tags: ghcr.io/synveda/keycloak:${{ needs.version.outputs.version }}-${{ matrix.arch }}",
    ),
    current.replace("- name: Bundled Keycloak", "- name: Omitted Keycloak"),
    current.replace(
      "for image in gateway postgres keycloak proxy; do",
      "for image in gateway postgres keycloak; do",
    ),
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
      "          file: deploy/compose/gateway/Dockerfile\n          platforms:",
      "          file: deploy/compose/gateway/Dockerfile\n          target: build\n          platforms:",
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
      "  publish:\n    needs: [version, binaries, bundles, images]\n",
      "  publish:\n    needs: [version, binaries, bundles, images]\n    if: always()\n",
    ),
    current.replace(
      "      - name: Join the per-architecture image tags\n        if: needs.version.outputs.publish == 'true'\n",
      "      - name: Join the per-architecture image tags\n",
    ),
    current.replace(
      '          version="${{ needs.version.outputs.version }}"\n          for image in gateway postgres keycloak proxy; do',
      "          version=latest\n          for image in gateway postgres keycloak proxy; do",
    ),
    current.replace(
      '          version="${{ needs.version.outputs.version }}"\n          for image in gateway postgres keycloak proxy; do',
      '          version="${{ needs.version.outputs.version }}"\n          version=latest\n          for image in gateway postgres keycloak proxy; do',
    ),
    current.replace(
      '--tag "ghcr.io/synveda/$image:$version"',
      '--tag "ghcr.io/synveda/$image:latest"',
    ),
    current.replace(
      '"ghcr.io/synveda/$image:$version-arm64"',
      '"ghcr.io/synveda/$image:$version-amd64"',
    ),
    current.replace('cnpg_tag="17.11-synveda-$version"', 'cnpg_tag="$version"'),
    current.replace("sha256sum synveda-*.tar.gz synveda-*.tgz", "sha256sum synveda-*.tar.gz"),
    current.replace('"synveda-$version.tgz"; do', '"synveda-plugin-$version.tar.gz"; do'),
    current.replace(
      "      - name: Publish\n        if: needs.version.outputs.publish == 'true'\n",
      "      - name: Publish\n        if: false\n",
    ),
    current.replace(
      '          gh release create "${GITHUB_REF_NAME}" ',
      "          true ",
    ),
  ].entries()) {
    assert.ok(releaseWorkflowFindings(mutant).length > 0, `mutant ${index}`);
  }
});

test("workspace and chart default to one versioned GHCR image pair", () => {
  const current = currentChartInputs();
  assert.equal(workspaceVersion(current[0]), "0.2.0");
  assert.deepEqual(chartParityFindings(...current), []);

  const mutants = [
    current.with(1, current[1].replace('appVersion: "0.2.0"', 'appVersion: "0.2.1"')),
    current.with(
      2,
      current[2].replace("repository: ghcr.io/synveda/gateway", "repository: synveda/gateway"),
    ),
    current.with(2, current[2].replace('  image: ""', "  image: latest")),
    current.with(
      3,
      current[3].replace(
        "ghcr.io/synveda/enterprise-postgres:17.11-synveda-%s",
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
  assert.deepEqual(helmAcceptanceFindings(demo, client), []);

  for (const [demoMutant, clientMutant] of [
    [demo.replace("ghcr.io/synveda/gateway:$IMAGE_TAG", "synveda/gateway:$IMAGE_TAG"), client],
    [
      demo.replace(
        "ghcr.io/synveda/enterprise-postgres:17.11-synveda-$IMAGE_TAG",
        "synveda/enterprise-postgres:17",
      ),
      client,
    ],
    [
      demo.replace(
        'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE" "$CNPG_IMAGE"',
        'kind load docker-image --name "$CLUSTER" "$PRODUCT_IMAGE"',
      ),
      client,
    ],
    [demo, client.replace("ghcr.io/synveda/gateway", "synveda/gateway")],
    [
      demo,
      client.replace(
        "SYNVEDA_INSECURE_DEVELOPMENT_HTTP",
        "SYNVEDA_INSECURE_DEVELOPMENT_HTTP_REMOVED",
      ),
    ],
  ]) {
    assert.ok(helmAcceptanceFindings(demoMutant, clientMutant).length > 0);
  }
});

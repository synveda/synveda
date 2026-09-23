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

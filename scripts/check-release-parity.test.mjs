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
import { setTimeout as delay } from "node:timers/promises";
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

test("one-command source chart packaging includes its dependency and native tools without downloads", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-chart-command-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const output = join(scratch, "chart output");
  const version = workspaceVersion(read("Cargo.toml"));
  const source = spawnSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).stdout.trim();
  const before = read("deploy/helm/synveda/Chart.lock");
  const beforeGuide = read("deploy/helm/synveda/README.md");
  const archive = join(output, `synveda-${version}.tgz`);
  let first;
  for (let attempt = 0; attempt < 2; attempt++) {
    const result = spawnSync("make", ["chart-package"], {
      cwd: ROOT, encoding: "utf8", timeout: 30_000,
      env: { ...process.env, SYNVEDA_CHART_OUTPUT: output, SYNVEDA_BUILD_SOURCE_SHA: "" },
    });
    assert.equal(result.status, 0, result.stderr);
    assert.ok(result.stdout.includes(`Source chart: ${archive}`));
    const bytes = readFileSync(archive);
    if (first) assert.deepEqual(bytes, first, "retry must preserve archive bytes");
    first = bytes;
  }
  assert.equal(read("deploy/helm/synveda/Chart.lock"), before);
  assert.equal(read("deploy/helm/synveda/README.md"), beforeGuide);
  for (const member of ["examples/prepare.mjs", "examples/operator.mjs", "examples/bundled-database.json", "examples/bundled-identity.json"]) {
    const extracted = spawnSync("tar", ["-xOzf", archive, `synveda/${member}`], { encoding: "utf8" });
    assert.equal(extracted.status, 0, extracted.stderr);
    assert.equal(extracted.stdout, read(`deploy/helm/synveda/${member}`));
  }
  const guide = (member) => {
    const result = spawnSync("tar", ["-xOzf", archive, `synveda/${member}`], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout;
  };
  const main = guide("README.md"), examples = guide("examples/README.md"), base = `https://github.com/synveda/synveda/blob/${source}/`;
  for (const packaged of [main, examples]) {
    assert.ok(packaged.includes(`**Chart archive ${version}.** This guide belongs to source \`${source}\`.`));
    assert.ok(packaged.includes("publisher attestation, checksums"));
    assert.doesNotMatch(packaged, /installation-version:|source candidates, absent|source candidates for the/);
  }
  assert.ok(main.includes(`  RELEASE_VERSION=${version}\n  SOURCE_SHA=${source}\n`));
  assert.ok(main.includes("independently reviewed release record"));
  assert.ok(main.includes(`](${base}docs/PRODUCTION_READINESS.md)`));
  assert.ok(main.includes("](BUILD.md)"), "chart-local links must survive extraction");
  assert.ok(examples.includes("](../README.md)"));
  assert.ok(guide("BUILD.md").includes(`](${base}docs/RELEASING.md)`));
  assert.ok(guide("CONFIGURATION.md").includes(`](${base}deploy/compose/postgres/synveda-database-bootstrap)`));
  const dependency = spawnSync("helm", ["show", "chart", archive], { encoding: "utf8" });
  assert.equal(dependency.status, 0, dependency.stderr);
  assert.match(dependency.stdout, /name: keycloakx/);
  assert.ok(dependency.stdout.includes(`version: ${version}`));
});

test("unknown chart source refuses before creating an archive output directory", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-chart-source-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  for (const source of ["main", "a".repeat(39), "A".repeat(40), "../source", "a".repeat(40) + "\n"]) {
    const output = join(scratch, "output");
    const result = spawnSync("sh", ["scripts/package-chart.sh"], {
      cwd: ROOT, encoding: "utf8", env: { ...process.env, SYNVEDA_BUILD_SOURCE_SHA: source, SYNVEDA_CHART_OUTPUT: output },
    });
    assert.equal(result.status, 64, result.stderr);
    assert.match(result.stderr, /^package-chart: select an exact reviewed source commit/);
    assert.equal(existsSync(output), false);
  }
});

test("changed chart guide identity boundaries refuse instead of leaving stale release pins", () => {
  const main = read("deploy/helm/synveda/README.md");
  const examples = read("deploy/helm/synveda/examples/README.md");
  for (const [mode, original, markers] of [
    ["main", main, ["<!-- installation-version:", "<!-- chart-package-source-status:end -->", "<!-- chart-package-source-identity:start -->", "<!-- chart-package-source-identity:end -->", "  RELEASE_VERSION=", "  SOURCE_SHA="]],
    ["examples", examples, ["<!-- chart-package-source-status:start -->", "<!-- chart-package-source-status:end -->"]],
  ]) {
    for (const marker of markers) {
      const result = spawnSync("awk", ["-v", `${mode}=1`, "-v", "version=0.4.4", "-v", `source=${"a".repeat(40)}`, "-f", "scripts/package-chart-guide.awk"], {
        cwd: ROOT, encoding: "utf8", input: original.replace(marker, "removed-boundary"),
      });
      assert.equal(result.status, 66, `${mode}: ${marker}`);
      assert.match(result.stderr, /guide identity boundaries changed/);
    }
  }
});

test("an incomplete chart checkout refuses packaging before output or dependency fetch", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-chart-missing-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  mkdirSync(join(scratch, "scripts"));
  for (const name of ["package-chart.sh", "release-version.sh"]) writeFileSync(join(scratch, "scripts", name), read(`scripts/${name}`));
  writeFileSync(join(scratch, "Cargo.toml"), read("Cargo.toml"));
  const output = join(scratch, "output");
  const result = spawnSync("sh", [join(scratch, "scripts/package-chart.sh")], {
    cwd: scratch, encoding: "utf8", env: { ...process.env, SYNVEDA_CHART_OUTPUT: output },
  });
  assert.equal(result.status, 66);
  assert.ok(result.stderr.includes("restore the matching source checkout or verified chart archive"));
  assert.equal(existsSync(output), false);
});

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

test("chart packages preserve complete notices and reproduce across clock seconds", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-chart-reproducibility-"));
  const version = workspaceVersion(read("Cargo.toml"));
  try {
    const packageChart = (directory) => {
      const result = spawnSync("sh", ["scripts/package-chart.sh", version, directory], {
        cwd: ROOT, encoding: "utf8", timeout: 60_000, maxBuffer: 1024 * 1024,
      });
      assert.equal(result.status, 0, result.stderr);
      return join(directory, `synveda-${version}.tgz`);
    };
    const first = packageChart(join(scratch, "first"));
    await delay(1100);
    const second = packageChart(join(scratch, "second"));
    assert.deepEqual(readFileSync(first), readFileSync(second), "same-source chart bytes depend on packaging time");
    for (const name of ["LICENSE", "NOTICE"]) {
      const result = spawnSync("tar", ["-xOzf", first, `synveda/${name}`], {
        timeout: 30_000, maxBuffer: 1024 * 1024,
      });
      assert.equal(result.status, 0, result.stderr.toString());
      assert.deepEqual(result.stdout, readFileSync(join(ROOT, name)), "chart notice bytes differ from source");
    }
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

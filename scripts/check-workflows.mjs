#!/usr/bin/env node
// FND-1 / OPS-8: protect the stage and publication boundaries; actionlint owns YAML.
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { requiredJobs } from "./ci-result.mjs";

const read = (path) =>
  readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
export const workflowSources = () =>
  Object.fromEntries(
    ["ci", "cli", "docker", "release"].map((name) => [
      name,
      read(`.github/workflows/${name}.yml`),
    ]),
  );
export function jobBlock(source, name) {
  const start = new RegExp(`^  ${name}:\\n`, "m").exec(source)?.index ?? -1;
  if (start < 0) return "";
  const rest = source.slice(start + name.length + 4);
  const next = /^  [a-z][a-z-]*:\n/m.exec(rest);
  return source.slice(
    start,
    next ? start + name.length + 4 + next.index : source.length,
  );
}
function stepBlock(source, name) {
  const marker = `      - name: ${name}\n`;
  const start = source.indexOf(marker);
  if (start < 0) return "";
  const next = source.indexOf("\n      - ", start + marker.length);
  return source.slice(start, next < 0 ? source.length : next);
}
export function actionFindings(source) {
  const pins = new Set(
    Object.entries(JSON.parse(read(".github/action-pins.json"))).map(
      ([ref, sha]) => `${ref.split("@")[0]}@${sha}`,
    ),
  );
  return [...source.matchAll(/uses: ([^\s]+)/g)].flatMap(([, ref]) =>
    pins.has(ref) ||
    ["./.github/workflows/cli.yml", "./.github/workflows/docker.yml"].includes(
      ref,
    )
      ? []
      : [`unreviewed action reference: ${ref}`],
  );
}
export function releaseWorkflowFindings(source, shared = workflowSources()) {
  const findings = [];
  const require = (ok, message) => {
    if (!ok) findings.push(message);
  };
  for (const text of [source, shared.cli, shared.docker]) {
    findings.push(...actionFindings(text));
    require(!/continue-on-error:|pull_request_target:|workflow_run:/.test(
      text,
    ), "release paths cannot mask failures or ingest privileged PR events");
  }
  require(source.includes(
    "permissions:\n  contents: read\n",
  ), "release default token must be read-only");
  require(source.includes(
    "cancel-in-progress: false",
  ), "releases must not cancel one another");
  const jobs = Object.fromEntries(
    [
      "version",
      "cli",
      "docker",
      "bundles",
      "images",
      "assemble",
      "verify-images",
      "publish",
    ].map((name) => [name, jobBlock(source, name)]),
  );
  for (const [name, needs] of Object.entries({
    cli: "version",
    docker: "version",
    bundles: "version",
    images: "[version, cli, bundles, docker]",
    assemble: "[version, cli, bundles, images]",
    "verify-images": "[version, assemble]",
    publish: "[version, assemble, verify-images]",
  })) {
    require(jobs[name].includes(
      `    needs: ${needs}\n`,
    ), `${name}: missing required producers`);
    require(!/^    if:/m.test(jobs[name]) ||
      (name === "publish" &&
        jobs[name].includes(
          "    if: needs.version.outputs.publish == 'true'\n",
        )), `${name}: unexpected job skip`);
  }
  require(jobs.cli.includes("uses: ./.github/workflows/cli.yml") &&
    jobs.cli.includes(
      "servers: true",
    ), "release must package all native clients and historical servers");
  require(jobs.docker.includes(
    "uses: ./.github/workflows/docker.yml",
  ), "release must test shared Docker candidates");
  const version = jobs.version;
  for (const marker of [
    'test "$(git rev-parse HEAD)" = "$TRIGGER_SHA"',
    "INPUT_VERSION: ${{ inputs.version }}",
    'sh scripts/release-version.sh "$version"',
    '[ "$version" != "$crate" ]',
    "node scripts/require-release-ci.mjs",
    "if: steps.pick.outputs.publish == 'true'",
    "fetch-depth: 0",
    "actions: read",
  ])
    require(version.includes(
      marker,
    ), `release source/version boundary missing: ${marker}`);
  require(source.split("${{ inputs.version }}").length ===
    2, "dispatch version must enter only through an environment variable");
  for (const name of ["images", "assemble", "publish"])
    require(jobs[name].includes(
      "environment: ${{ needs.version.outputs.publish == 'true' && 'release' || 'release-dry-run' }}",
    ), `${name}: protected publication environment missing`);
  for (const name of ["images", "assemble"]) {
    require(jobs[name].includes(
      "      packages: write",
    ), `${name}: scoped registry permission missing`);
    const hub = stepBlock(jobs[name], "Log in to Docker Hub");
    for (const marker of [
      "if: needs.version.outputs.publish == 'true'",
      "registry: docker.io",
      "username: ${{ vars.DOCKERHUB_USERNAME }}",
      "password: ${{ secrets.DOCKERHUB_TOKEN }}",
    ])
      require(hub.includes(
        marker,
      ), `${name}: Docker Hub login boundary missing`);
  }
  require(jobs.images.includes("node scripts/publish-images.mjs candidate") &&
    jobs.images.includes("name: docker-candidate-${{ matrix.arch }}") &&
    !jobs.images.includes(
      "docker/build-push-action",
    ), "publisher must copy this run's tested OCI artifacts without a rebuild");
  for (const text of [source, shared.cli, shared.docker])
    require(!/run-id:|github-token:|repository:\s*[^$\n]/.test(
      text,
    ), "release cannot download artifacts from another run or repository");
  const assembly = jobs.assemble;
  for (const marker of [
    'node scripts/release-registries.mjs assemble "$VERSION" "$SOURCE_SHA" "$PUBLISH"',
    "node scripts/release-registries.mjs package",
    'SYNVEDA_PACKAGE_CONSUMER_CANDIDATE: "1"',
    'node scripts/check-release-assets.mjs assets "$VERSION" "$GITHUB_SHA" "$PUBLISH" assembled',
    'pattern: "{binaries-*,bundles}"',
    "name: release-assets",
    "assets/synveda-client-*.zip",
    "assets/synveda-client-report-*.json",
  ])
    require(assembly.includes(marker), `assembly boundary missing: ${marker}`);
  const oci = stepBlock(assembly, "Publish and pull the OCI chart");
  for (const marker of [
    "if: needs.version.outputs.publish == 'true'",
    "helm registry login ghcr.io",
    "--password-stdin",
    'helm push "assets/synveda-$version.tgz"',
    "helm pull oci://ghcr.io/synveda/charts/synveda",
    'cmp "assets/synveda-$version.tgz" "pulled-chart/synveda-$version.tgz"',
  ])
    require(oci.includes(
      marker,
    ), "tag-only OCI chart publication/pull comparison missing");
  const verify = jobs["verify-images"];
  require(!/secrets\.|docker\/login-action|packages: write/.test(
    verify,
  ), "public verification must be anonymous and read-only");
  for (const marker of [
    'mktemp -d "$RUNNER_TEMP/synveda-anonymous-docker.XXXXXX"',
    'echo "DOCKER_CONFIG=$directory" >> "$GITHUB_ENV"',
    "sha256sum --check SHA256SUMS",
    "node scripts/verify-release-images.mjs",
    '"assets/synveda-registry-images-$VERSION.json"',
    "node scripts/qualify-release.mjs \\",
    "node scripts/qualify-release.mjs --consumer-candidate",
    "node scripts/qualify-kubernetes-release.mjs",
    'cmp "assets/synveda-$VERSION.tgz" "anonymous-chart/synveda-$VERSION.tgz"',
    "release-consumer-${{ matrix.arch }}.json",
    "name: release-verification-${{ matrix.arch }}",
  ])
    require(verify.includes(
      marker,
    ), `public artifact qualification missing: ${marker}`);
  for (const arch of ["amd64", "arm64"])
    for (const text of [verify, shared.docker])
      require(text.includes(
        `arch: ${arch}\n            platform: linux/${arch}`,
      ), `missing native ${arch} Docker runner`);
  const publish = jobs.publish;
  require(publish.includes("      contents: write") &&
    publish.includes("      id-token: write\n      attestations: write") &&
    !publish.includes("packages: write"), "final publisher permissions differ");
  for (const marker of [
    "pattern: release-verification-*",
    'node scripts/check-release-assets.mjs assets "$VERSION" "$GITHUB_SHA" true qualified',
    "subject-path: assets/SHA256SUMS",
    "uses: actions/attest@",
    '--repo "$GH_REPO" --signer-workflow "$GH_REPO/.github/workflows/release.yml"',
    '--source-ref "$GITHUB_REF" --source-digest "$SOURCE_SHA" --deny-self-hosted-runners',
    'node scripts/publish-release.mjs assets "$VERSION"',
  ])
    require(publish.includes(
      marker,
    ), `final publication boundary missing: ${marker}`);
  for (const text of [shared.cli, shared.docker])
    require(!/secrets\.|packages: write|contents: write|docker\/login-action|secrets: inherit/.test(
      text,
    ), "candidate validation must not receive publishing privileges");
  for (const target of [
    "darwin-arm64",
    "darwin-x86_64",
    "linux-arm64",
    "linux-x86_64",
    "windows-arm64",
    "windows-x86_64",
  ])
    require(shared.cli.includes(
      `target: ${target}\n`,
    ), `missing native client target: ${target}`);
  for (const marker of [
    "macos-14",
    "macos-15-intel",
    "ubuntu-latest",
    "ubuntu-24.04-arm",
    "windows-2025",
    "windows-11-arm",
    "node scripts/package-client.mjs",
    "node scripts/check-client-package.mjs",
    "node scripts/windows-client-candidate.mjs",
    "--test credential_refresh",
    "node scripts/check-windows-state.mjs",
    "peer_witness_refuses_before_opening",
    "credentials::windows",
    "private_state::tests",
    "local_state::windows_tests",
    "spool::tests",
    "--test client_platform",
  ])
    require(shared.cli.includes(
      marker,
    ), `native client coverage missing: ${marker}`);
  const plan = [
    ["The product image", "deploy/compose/product/Dockerfile", "product", null],
    ["Postgres", "deploy/compose/postgres/Dockerfile", "postgres", "reference"],
    [
      "CloudNativePG PostgreSQL",
      "deploy/helm/postgres/Dockerfile",
      "cnpg-postgres",
      null,
    ],
    [
      "Bundled Keycloak",
      "deploy/compose/keycloak/Dockerfile",
      "keycloak",
      null,
    ],
    ["Reference proxy", "deploy/compose/proxy/Dockerfile", "proxy", null],
    [
      "Browser acceptance fixture",
      "deploy/compose/product/Dockerfile",
      "browser-acceptance",
      "browser-acceptance",
    ],
  ];
  require((shared.docker.match(/uses: docker\/build-push-action@/g) ?? [])
    .length === 6, "candidate must build exactly six images");
  for (const [name, file, archive, target] of plan) {
    const block = stepBlock(shared.docker, name);
    const inputs = Object.fromEntries(
      [...block.matchAll(/^          ([\w-]+): (.+)$/gm)].map(
        ([, key, value]) => [key, value],
      ),
    );
    for (const [key, value] of Object.entries({
      context: ".",
      file,
      outputs: `type=oci,dest=\${{ runner.temp }}/images/${archive}.tar`,
      platforms: "${{ matrix.platform }}",
      provenance: "mode=max",
      sbom: "true",
      labels: "|",
    }))
      require(inputs[key] === value, `${name}: invalid ${key}`);
    require((inputs.target ?? null) === target, `${name}: wrong build target`);
    require(Object.keys(inputs).every((key) =>
      [
        "context",
        "file",
        "outputs",
        "platforms",
        "provenance",
        "sbom",
        "labels",
        "target",
        "cache-from",
        "cache-to",
      ].includes(key),
    ), `${name}: unexpected build override`);
    require(!/^        if:/m.test(
      block,
    ), `${name}: native build may be skipped`);
    for (const marker of [
      "org.opencontainers.image.source=https://github.com/${{ github.repository }}",
      "org.opencontainers.image.revision=${{ github.sha }}",
      "org.opencontainers.image.version=${{ inputs.version }}",
    ])
      require(block.includes(marker), `${name}: source identity label missing`);
  }
  for (const marker of [
    "127.0.0.1:5000:5000",
    "node scripts/docker-candidate.mjs",
    "node scripts/qualify-release.mjs",
    "node scripts/qualify-release.mjs --consumer-candidate",
    "node scripts/qualify-kubernetes-release.mjs",
    "name: docker-candidate-${{ matrix.arch }}",
  ])
    require(shared.docker.includes(
      marker,
    ), `local Docker qualification missing: ${marker}`);
  return findings;
}
export function ciWorkflowFindings(source) {
  const errors = actionFindings(source);
  const result = jobBlock(source, "result");
  const expected = [
    "changes",
    "check",
    "licences",
    ...Object.keys(requiredJobs),
  ].sort();
  const actual = result
    .match(/^    needs: \[([^\]]+)\]/m)?.[1]
    .split(", ")
    .sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected))
    errors.push("CI Result must depend on every required job");
  if (
    !result.includes("name: CI Result") ||
    !result.includes("if: always()") ||
    !result.includes("run: node scripts/ci-result.mjs") ||
    !result.includes("CI_NEEDS: ${{ toJSON(needs) }}")
  )
    errors.push(
      "CI Result must run even after failure and inspect actual results",
    );
  for (const [name, stage] of Object.entries(requiredJobs))
    if (
      !jobBlock(source, name).includes(
        `if: needs.changes.outputs.${stage} == 'true'`,
      )
    )
      errors.push(`${name}: wrong change-selection dependency`);
  if (
    !source.includes("  merge_group:") ||
    !source.includes("    branches: [main]") ||
    !source.includes(
      "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
    )
  )
    errors.push("CI trigger/cancellation contract changed");
  return errors;
}
if (process.argv[1] && resolve(process.argv[1]) === import.meta.filename) {
  const sources = workflowSources();
  const errors = [
    ...releaseWorkflowFindings(sources.release, sources),
    ...ciWorkflowFindings(sources.ci),
  ];
  for (const file of readdirSync(
    new URL("../.github/workflows/", import.meta.url),
  ))
    if (file.endsWith(".yml"))
      errors.push(...actionFindings(read(`.github/workflows/${file}`)));
  if (errors.length) {
    console.error(errors.join("\n"));
    process.exitCode = 1;
  } else
    console.log(
      "Workflow coverage, action pins and publication boundaries agree.",
    );
}

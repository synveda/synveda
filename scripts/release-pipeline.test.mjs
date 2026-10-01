import { requiredRustPackages } from "./rust-sbom.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  workflowSources,
  releaseWorkflowFindings,
  ciWorkflowFindings,
} from "./check-workflows.mjs";
import { validatedRun, requireResult } from "./require-release-ci.mjs";
import { candidateIdentity, fileHash } from "./docker-candidate.mjs";
import { copyPlan } from "./publish-images.mjs";
import { releaseImages } from "./release-registries.mjs";
import {
  releaseAssets,
  regularAssets,
  checkArchiveNotices,
  checkQualification,
  checkCandidateImageReports,
} from "./check-release-assets.mjs";
import { noticeHashes } from "./verify-release-images.mjs";
import { publishRelease, uploadedAssets } from "./publish-release.mjs";
import { rustImageRoots, sbomGenerator } from "./rust-image-sbom.mjs";

import { checkConsolePackage } from "./check-console-package.mjs";
import { consolePackageFixture } from "./fixtures/console-package.mjs";
import { productNodeReport } from "./fixtures/node-image.mjs";

const rustSbomFixture = (name, version, imageManifest) => ({
  schema_version: 1, scanner: "syft-v1.51.0", spdx_version: "SPDX-2.3",
  image_manifest: imageManifest, attestation_manifest: `sha256:${"f".repeat(64)}`,
  statement_sha256: "9".repeat(64), package_count: 100, cargo_package_count: 50, cargo_identity_count: 50,
  required_packages: requiredRustPackages(name, version),
});


test("workflow refactor retains release, native platform and security boundaries", () => {
  const current = workflowSources();
  assert.deepEqual(releaseWorkflowFindings(current.release, current), []);
  assert.deepEqual(ciWorkflowFindings(current.ci), []);
  const mutants = [
    [
      "release",
      "needs: [version, assemble, verify-images]",
      "needs: [version, assemble]",
    ],
    ["release", "needs: [version, cli, bundles, docker]", "needs: version"],
    [
      "release",
      "needs: [version, cli, bundles, images]",
      "needs: [version, bundles]",
    ],
    ["release", "node scripts/require-release-ci.mjs", "echo skipped"],
    ["release", "full_compose: true", "full_compose: false"],
    ["release", "node scripts/publish-images.mjs", "echo rebuilt"],
    ["release", "node scripts/verify-release-images.mjs", "echo skipped"],
    ["release", "pattern: docker-checks-*", "pattern: wrong-reports-*"],
    ["release", "assets/release-candidate-*.json", "assets/unrelated-*.json"],
    [
      "release",
      'SYNVEDA_PACKAGE_CONSUMER_CANDIDATE: "1"',
      'SYNVEDA_PACKAGE_CONSUMER_CANDIDATE: "0"',
    ],
    ["release", "node scripts/check-release-assets.mjs", "echo incomplete"],
    ["release", "node scripts/check-console-package.mjs", "echo skipped console inventory"],
    ["release", "SYNVEDA_BUILD_SOURCE_SHA: ${{ github.sha }}", "SYNVEDA_BUILD_SOURCE_SHA: obsolete"],
    ["docker", "build-args: SYNVEDA_BUILD_SOURCE_SHA=${{ github.sha }}", "build-args: SYNVEDA_BUILD_SOURCE_SHA=obsolete"],
    ["release", "subject-path: assets/SHA256SUMS", "subject-path: unrelated"],
    [
      "release",
      '--source-ref "$GITHUB_REF" --source-digest "$SOURCE_SHA"',
      "--source-ref refs/heads/main",
    ],
    [
      "release",
      "password: ${{ secrets.DOCKERHUB_TOKEN }}",
      "password: hardcoded",
    ],
    [
      "release",
      "environment: ${{ needs.version.outputs.publish == 'true' && 'release' || 'release-dry-run' }}",
      "environment: unprotected",
    ],
    ["release", 'sh scripts/release-version.sh "$version"', "true"],
    [
      "release",
      "INPUT_VERSION: ${{ inputs.version }}",
      "INPUT_VERSION: unused",
    ],
    ["release", "cancel-in-progress: false", "cancel-in-progress: true"],
    [
      "release",
      'cmp "assets/synveda-$VERSION.tgz" "anonymous-chart/synveda-$VERSION.tgz"',
      "true",
    ],
    [
      "release",
      'cmp "assets/synveda-$version.tgz" "pulled-chart/synveda-$version.tgz"',
      "true",
    ],
    [
      "release",
      'mktemp -d "$RUNNER_TEMP/synveda-anonymous-docker.XXXXXX"',
      "echo ~/.docker",
    ],
    [
      "release",
      "          name: docker-candidate-${{ matrix.arch }}",
      "          name: docker-candidate-${{ matrix.arch }}\n          run-id: 123",
    ],
    ["release", "node scripts/publish-release.mjs", "echo skipped"],
    ["cli", "node scripts/package-client.mjs", "echo skipped"],
    ["cli", "node scripts/check-client-package.mjs", "echo skipped"],
    ["cli", "node scripts/windows-client-candidate.mjs", "echo skipped"],
    ["cli", "node scripts/rust-archive-sbom.mjs client", "echo skipped"],
    ["cli", "node scripts/rust-archive-sbom.mjs server", "echo skipped"],
    ["cli", "node scripts/download-syft.mjs", "echo unpinned scanner"],
    ["cli", "cargo auditable build --release --locked", "cargo build --release --locked"],
    ["cli", "cargo-auditable --version 0.7.6", "cargo-auditable --version 0.7.5"],
    ["cli", '--root "$RUNNER_TEMP/synveda-auditable"', '--root "$HOME"'],
    ["cli", 'strip "$stage/synveda" "$stage/synveda-gateway" "$stage/synveda-worker"', 'strip "$stage/synveda" "$stage/synveda-gateway" "$stage/synveda-worker" || true'],
    ["release", "assets/synveda-*.spdx.json", "assets/unrelated-*.spdx.json"],
    ["release", "assets/synveda-*.rust-sbom.json", "assets/unrelated-*.rust-sbom.json"],
    ["cli", "runner: windows-11-arm", "runner: windows-2025"],
    ["cli", "target: linux-arm64", "target: linux-unsupported"],
    ["cli", "  binaries:\n", "  binaries:\n    continue-on-error: true\n"],
    ["cli", "contents: read", "contents: write"],
    [
      "docker",
      "org.opencontainers.image.revision=${{ github.sha }}",
      "org.opencontainers.image.revision=old",
    ],
    [
      "docker",
      "file: deploy/helm/postgres/Dockerfile",
      "file: deploy/compose/postgres/Dockerfile",
    ],
    [
      "docker",
      "file: deploy/compose/keycloak/Dockerfile",
      "file: deploy/compose/product/Dockerfile",
    ],
    [
      "docker",
      "      - name: Bundled Keycloak\n",
      "      - name: Bundled Keycloak\n        if: false\n",
    ],
    ["docker", "          provenance: mode=max", "          provenance: false"],
    ["docker", "          sbom: true", "          sbom: false"],
    ["docker", `sbom: generator=${sbomGenerator}`, "sbom: true"],
    ["docker", "tags: localhost:5000/synveda/product:${{ inputs.version }}-${{ matrix.arch }}", "tags: untrusted.example/product:latest"],
    [
      "docker",
      "          file: deploy/helm/postgres/Dockerfile\n",
      "          file: deploy/helm/postgres/Dockerfile\n          build-args: CNPG_BASE=evil.example/postgres:latest\n",
    ],
    [
      "docker",
      "          file: deploy/compose/product/Dockerfile\n",
      "          file: deploy/compose/product/Dockerfile\n          target: build\n",
    ],
    ["docker", "platform: linux/arm64", "platform: linux/amd64"],
    ["docker", "127.0.0.1:5000:5000", "5000:5000"],
    [
      "docker",
      "node scripts/qualify-release.mjs --consumer-candidate",
      "echo skipped",
    ],
    ["docker", "if: inputs.full_compose", "if: false"],
    ["docker", "node scripts/qualify-kubernetes-release.mjs", "echo skipped"],
    ["docker", "${{ runner.temp }}/images/candidate.json", "missing-candidate.json"],
  ];
  for (const [file, from, to] of mutants) {
    const shared = { ...current, [file]: current[file].replace(from, to) };
    assert.notEqual(
      shared[file],
      current[file],
      `mutation did not apply: ${from}`,
    );
    assert.ok(
      releaseWorkflowFindings(shared.release, shared).length,
      `${file}: ${from}`,
    );
  }
  for (const [from, to] of [
    ["name: CI Result\n    if: always()", "name: CI Result\n    if: success()"],
    ["helm-operations, cli]", "helm-operations]"],
    ["CI_NEEDS: ${{ toJSON(needs) }}", "CI_NEEDS: '{}'"],
    ["if: needs.changes.outputs.cli == 'true'", "if: false"],
    ["node scripts/verify-starter-image-reuse.mjs", "echo unchecked images"],
    ["PRODUCT_IMAGE: synveda/product:ops11", "PRODUCT_IMAGE: unrelated/image:old"],
    ["full_compose: false", "full_compose: true"],
  ])
    assert.ok(ciWorkflowFindings(current.ci.replace(from, to)).length);
});

test("release requires the latest exact-source successful full main CI and its gate", () => {
  const source = "a".repeat(40),
    repository = "synveda/synveda";
  const run = {
    id: 1,
    path: ".github/workflows/ci.yml",
    head_sha: source,
    head_branch: "main",
    event: "push",
    head_repository: { full_name: repository },
    status: "completed",
    conclusion: "success",
  };
  assert.equal(validatedRun([run], repository, source).id, 1);
  for (const change of [
    { event: "pull_request" },
    { event: "workflow_dispatch" },
    { head_sha: "b".repeat(40) },
    { head_branch: "feature" },
    { head_repository: { full_name: "fork/synveda" } },
    { conclusion: "failure" },
    { status: "in_progress" },
    { path: ".github/workflows/release.yml" },
  ])
    assert.throws(() =>
      validatedRun([{ ...run, ...change }], repository, source),
    );
  assert.throws(() =>
    validatedRun(
      [run, { ...run, id: 2, conclusion: "cancelled" }],
      repository,
      source,
    ),
  );
  requireResult([
    { name: "CI Result", conclusion: "success" },
    { name: "CLI / windows-arm64", conclusion: "success" },
  ]);
  for (const conclusion of [
    "failure",
    "skipped",
    "cancelled",
    "neutral",
    undefined,
  ])
    assert.throws(() =>
      requireResult([
        { name: "CI Result", conclusion: "success" },
        { name: "required", conclusion },
      ]),
    );
  assert.throws(() => requireResult([{ name: "rust", conclusion: "success" }]));
});

test("two registry destinations copy one complete immutable OCI candidate set", async () => {
  const candidate = {
    version: "0.4.0",
    source: "a".repeat(40),
    source_dirty: false,
    arch: "arm64",
    images: Object.fromEntries(
      Object.entries(releaseImages).map(([name, repository]) => [
        name,
        {
          archive: `${repository}.tar`,
          digest: `sha256:${"b".repeat(64)}`,
          sha256: "c".repeat(64),
          ...(Object.hasOwn(rustImageRoots, name) ? { rust_sbom: rustSbomFixture(name, "0.4.0", `sha256:${"d".repeat(64)}`) } : {}),
        },
      ]),
    ),
  };
  candidateIdentity(
    candidate,
    candidate.version,
    candidate.source,
    candidate.arch,
  );
  const plan = copyPlan(candidate, "test-owner");
  assert.equal(plan.length, 12);
  assert.deepEqual(
    plan.slice(0, 6).map((entry) => entry.archive),
    plan.slice(6).map((entry) => entry.archive),
  );
  assert.ok(plan.every((entry) => entry.reference.endsWith("0.4.0-arm64")));
  assert.ok(
    plan.some(
      (entry) =>
        entry.reference ===
        "ghcr.io/synveda/cnpg-postgres:17.11-synveda-0.4.0-arm64",
    ),
  );
  for (const change of [
    { version: "0.5.0" },
    { source: "b".repeat(40) },
    { source_dirty: true },
    { source_dirty: undefined },
    { arch: "amd64" },
    { images: { product: candidate.images.product } },
  ])
    assert.throws(() =>
      candidateIdentity(
        { ...candidate, ...change },
        candidate.version,
        candidate.source,
        candidate.arch,
      ),
    );
  const scratch = mkdtempSync(join(tmpdir(), "synveda-oci-digest-"));
  try {
    const file = join(scratch, "oci.tar");
    writeFileSync(file, "tested bytes");
    const hash = await fileHash(file);
    writeFileSync(file, "changed bytes");
    assert.notEqual(await fileHash(file), hash);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("all non-client release archives retain exact regular licence and notice files", (t) => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-release-notices-"));
  t.after(() => rmSync(scratch, { recursive: true, force: true }));
  const version = "0.4.0";
  const archives = [
    [`synveda-${version}-darwin-arm64.tar.gz`, ""],
    [`synveda-${version}-linux-x86_64.tar.gz`, ""],
    [`synveda-console-${version}.tar.gz`, "console"],
    [`synveda-reference-${version}.tar.gz`, `synveda-reference-${version}`],
    [`synveda-plugin-${version}.tar.gz`, "plugin"],
    [`synveda-${version}.tgz`, "synveda"],
  ];
  const stages = archives.map(([archive, prefix], index) => {
    const stage = join(scratch, `stage-${index}`);
    const directory = join(stage, prefix);
    mkdirSync(directory, { recursive: true });
    for (const name of ["LICENSE", "NOTICE"])
      copyFileSync(new URL(`../${name}`, import.meta.url), join(directory, name));
    const pack = (duplicate = false) => execFileSync("tar", ["-czf", join(scratch, archive), "-C", stage,
      ...(prefix ? [prefix] : readdirSync(stage)), ...(duplicate ? [prefix ? `${prefix}/NOTICE` : "NOTICE"] : [])]);
    pack();
    return { directory, pack };
  });
  checkArchiveNotices(scratch, version);
  for (const { directory, pack } of stages) {
    const notice = join(directory, "NOTICE");
    const original = readFileSync(notice);
    for (const damage of [
      () => rmSync(notice),
      () => writeFileSync(notice, "changed notice\n"),
      () => { rmSync(notice); symlinkSync("LICENSE", notice); },
    ]) {
      damage();
      pack();
      assert.throws(() => checkArchiveNotices(scratch, version));
      rmSync(notice, { force: true });
      writeFileSync(notice, original);
    }
    pack(true);
    assert.throws(() => checkArchiveNotices(scratch, version), /one regular file/);
    pack();
  }
  checkArchiveNotices(scratch, version);
});

test("qualification rejects incomplete, failed or transplanted native reports", t => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-qualified-reports-"));
  try {
    const version = "0.4.0",
      source = "a".repeat(40);
    const inventory = {
      registries: Object.fromEntries(
        ["dockerhub", "ghcr"].map((registry) => [
          registry,
          {
            images: Object.fromEntries(
              Object.entries(releaseImages).map(([name, repo]) => [
                name,
                {
                  reference: `${registry}/${repo}@sha256:${"b".repeat(64)}`,
                  platforms: {
                    "linux/amd64": `sha256:${"d".repeat(64)}`,
                    "linux/arm64": `sha256:${"e".repeat(64)}`,
                  },
                },
              ]),
            ),
          },
        ]),
      ),
    };
    const console = consolePackageFixture(t, { version, source });
    console.pack();
    const consoleReport = checkConsolePackage(console.archive, version, source, console.root);
    const consoleEvidence = { ...consoleReport };
    delete consoleEvidence.archive_sha256;
    // The retained operator fixture has day-two evidence for the two complete
    // ownership modes and install/upgrade evidence for all four combinations.
    const evidence = JSON.parse(
      readFileSync("demos/evidence/ops11-bundled.json", "utf8"),
    );
    // Model the new candidate field without rewriting historical live evidence.
    for (const item of evidence.cases)
      item.reinstallTransport = { directGateway: 200, applicationEdge: 200, issuerEdge: 200, recoveredMs: 1, failedProbes: 0 };
    const reports = {};
    for (const arch of ["amd64", "arm64"]) {
      const platform = `linux/${arch}`;
      const candidate = {
        version,
        source,
        source_dirty: false,
        arch,
        images: Object.fromEntries(Object.entries(releaseImages).map(
          ([name, repo]) => [name, {
            archive: `${repo}.tar`,
            sha256: "c".repeat(64),
            digest: `sha256:${"a".repeat(64)}`,
            ...(Object.hasOwn(rustImageRoots, name) ? { rust_sbom: rustSbomFixture(name, version, inventory.registries.dockerhub.images[name].platforms[platform]) } : {}),
          }],
        )),
      };
      const images = Object.fromEntries(Object.entries(releaseImages).map(
        ([name, repo]) => [name,
          `localhost:5000/synveda/${repo}@${candidate.images[name].digest}`],
      ));
      reports[`release-candidate-${arch}.json`] = candidate;
      reports[`release-local-images-${arch}.json`] = {
        release_version: version,
        source_sha: source,
        platform,
        anonymous_pull: false,
        local_candidate: true,
        images: Object.entries(images).map(([name, image]) => ({
          name,
          image,
          platforms: { [platform]: inventory.registries.dockerhub.images[name].platforms[platform] },
          executable_checks: 1,
          notice_sha256: noticeHashes,
          ...(name === "product" ? { console_inventory: consoleEvidence } : {}),
          ...(name === "product" ? { node_inventory: productNodeReport(platform) } : {}),
          ...(Object.hasOwn(rustImageRoots, name) ? { rust_sbom: candidate.images[name].rust_sbom } : {}),
        })),
      };
      reports[`release-images-${arch}.json`] = {
        release_version: version,
        source_sha: source,
        platform,
        anonymous_pull: true,
        registries: Object.fromEntries(
          Object.entries(inventory.registries).map(([registry, result]) => [
            registry,
            {
              platform,
              anonymous_pull: true,
              images: Object.entries(result.images).map(([name, entry]) => ({
                name,
                image: entry.reference,
                platforms: { [platform]: entry.platforms[platform] },
                executable_checks: 1,
                notice_sha256: noticeHashes,
                ...(name === "product" ? { console_inventory: consoleEvidence } : {}),
                ...(name === "product" ? { node_inventory: productNodeReport(platform) } : {}),
              })),
            },
          ]),
        ),
      };
      for (const kind of ["docker", "consumer", "kubernetes"])
        reports[`release-${kind}-${arch}.json`] = {
          version,
          source,
          sourceDirty: false,
          images,
          ...(kind === "kubernetes"
            ? { evidence, localPortForward: { browserLoginLogout: true } }
            : {
                checks: Object.fromEntries(
                  Array.from(
                    { length: kind === "consumer" ? 20 : 7 },
                    (_, i) => [`check-${i}`, true],
                  ),
                ),
              }),
        };
    }
    const check = (change = () => {}) => {
      const changed = structuredClone(reports);
      change(changed);
      for (const [file, report] of Object.entries(changed))
        writeFileSync(join(scratch, file), JSON.stringify(report));
      checkCandidateImageReports(scratch, version, source, consoleReport);
      checkQualification(scratch, version, source, inventory, consoleReport);
    };
    check();
    for (const change of [
      r => { delete r["release-local-images-arm64.json"].images[0].node_inventory; },
      r => { r["release-local-images-amd64.json"].images[0].node_inventory.binary_sha256 = "0".repeat(64); },
      r => { r["release-local-images-arm64.json"].images[0].node_inventory.dependencies.openssl = "0.0.0"; },
      r => { r["release-images-arm64.json"].registries.ghcr.images[0].node_inventory.notices["nbytes-LICENSE"].sha256 = "0".repeat(64); },
      r => { delete r["release-images-amd64.json"].registries.dockerhub.images[0].node_inventory; },
      r => { delete r["release-local-images-arm64.json"].images[0].console_inventory; },
      r => { r["release-local-images-amd64.json"].images[0].console_inventory.inventory_sha256 = "0".repeat(64); },
      r => { r["release-local-images-arm64.json"].images[0].console_inventory.source_sha = "b".repeat(40); },
      r => { r["release-images-amd64.json"].registries.ghcr.images[0].console_inventory.notices_sha256 = "0".repeat(64); },
      r => { delete r["release-images-arm64.json"].registries.dockerhub.images[0].console_inventory; },
      (r) => { delete r["release-candidate-arm64.json"].images.product.rust_sbom; },
      (r) => { delete r["release-local-images-amd64.json"].images[0].rust_sbom; },
      (r) => { r["release-candidate-arm64.json"].images.product.rust_sbom.required_packages.sqlx = "0.0.0"; },
      (r) => {
        r["release-docker-arm64.json"].source = "c".repeat(40);
      },
      (r) => {
        r["release-candidate-arm64.json"].images.product.digest = `sha256:${"f".repeat(64)}`;
      },
      (r) => {
        r["release-local-images-amd64.json"].images[0].platforms["linux/amd64"] = `sha256:${"f".repeat(64)}`;
      },
      (r) => {
        r["release-consumer-amd64.json"].sourceDirty = true;
      },
      (r) => {
        r["release-consumer-arm64.json"].checks["check-0"] = false;
      },
      (r) => {
        delete r["release-consumer-arm64.json"].checks["check-0"];
      },
      (r) => {
        r["release-images-arm64.json"].platform = "linux/amd64";
      },
      (r) => {
        r["release-images-amd64.json"].registries.ghcr.anonymous_pull = false;
      },
      (r) => {
        r["release-images-arm64.json"].registries.dockerhub.images.pop();
      },
      (r) => {
        delete r["release-local-images-arm64.json"].images[0].notice_sha256;
      },
      (r) => {
        r["release-images-amd64.json"].registries.ghcr.images[0].notice_sha256.NOTICE = "0".repeat(64);
      },
      (r) => {
        r["release-kubernetes-arm64.json"].evidence.cases.pop();
      },
      (r) => {
        r["release-kubernetes-amd64.json"].evidence.cases[0].persistentContent =
          false;
      },
      (r) => {
        delete r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport;
      },
      (r) => {
        r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport.applicationEdge = 502;
      },
      (r) => {
        r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport.recoveredMs = 60_001;
      },
      (r) => {
        r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport.recoveredMs = -1;
      },
      (r) => {
        delete r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport.issuerEdge;
      },
      (r) => {
        r["release-kubernetes-arm64.json"].evidence.cases[0].reinstallTransport.failedProbes = -1;
      },
      (r) => {
        delete r["release-kubernetes-arm64.json"].evidence.cases[0].dayTwo
          .restore;
      },
      (r) => {
        r["release-kubernetes-arm64.json"].localPortForward.browserLoginLogout =
          false;
      },
    ])
      assert.throws(() => check(change));
    check();
    rmSync(join(scratch, "release-consumer-arm64.json"));
    assert.throws(() =>
      checkQualification(scratch, version, source, inventory, consoleReport),
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("stable publication waits for every expected upload and never includes checkout directories", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-release-assets-"));
  try {
    const version = "0.4.0",
      source = "a".repeat(40);
    assert.equal(releaseAssets(version).length, 51);
    const names = [
      ...releaseAssets(version, true),
      "SHA256SUMS",
      "SHA256SUMS.sigstore.json",
    ];
    assert.equal(names.length, 55);
    for (const target of [
      "darwin-arm64",
      "darwin-x86_64",
      "linux-arm64",
      "linux-x86_64",
      "windows-arm64",
      "windows-x86_64",
    ])
      assert.ok(names.includes(`synveda-client-report-${target}.json`));
    for (const name of names)
      writeFileSync(join(scratch, name), "qualified bytes");
    mkdirSync(join(scratch, "brand"));
    mkdirSync(join(scratch, "product"));
    writeFileSync(join(scratch, "synveda-9.9.9.tgz"), "unrelated bytes");
    const expected = regularAssets(scratch, names);
    const calls = [];
    const invoke = (fault) =>
      publishRelease(
        scratch,
        version,
        source,
        "synveda/synveda",
        "owner",
        (command, args) => {
          calls.push([command, args]);
          if (
            command === "gh" &&
            args[0] === "release" &&
            args[1] === "create" &&
            fault === "upload"
          )
            throw new Error("upload failed");
          if (command === "gh" && args[0] === "api") {
            if (args[1] === "--method" && args[2] === "PATCH")
              return JSON.stringify({
                id: 42,
                tag_name: `v${version}`,
                target_commitish: source,
                draft: false,
              });
            if (args[1].includes("/commits/"))
              return JSON.stringify({
                sha: fault === "tag" ? "b".repeat(40) : source,
              });
            if (args[1].includes("/releases/tags/"))
              throw new Error("draft release lookup by tag returns 404");
            if (args[1].includes("/releases?")) {
              const draft = {
                id: 42,
                tag_name: `v${version}`,
                target_commitish: fault === "source" ? "b".repeat(40) : source,
                name: `Synveda v${version}`,
                draft: fault !== "draft",
              };
              return JSON.stringify([
                { ...draft, id: 41, tag_name: "v0.3.0" },
                draft,
                ...(fault === "ambiguous" ? [{ ...draft, id: 43 }] : []),
              ]);
            }
            if (args[1].includes("/assets?"))
              return JSON.stringify(
                expected
                  .filter((entry) => fault !== "sbom" || !entry.name.endsWith(".spdx.json"))
                  .slice(fault === "missing" ? 1 : 0)
                  .map((entry) => ({ ...entry, state: "uploaded" })),
              );
            return "";
          }
          return "";
        },
      );
    invoke();
    const notes = readFileSync(join(scratch, "release-notes.md"), "utf8");
    assert.ok(notes.includes(`SYNVEDA_SOURCE_SHA=${source}`));
    assert.ok(notes.includes(`-SourceSha ${source}`));
    assert.ok(notes.includes(`https://raw.githubusercontent.com/synveda/synveda/${source}/scripts/install.sh`));
    assert.ok(!notes.includes("{{source}}"));
    const create = calls.find(
      ([command, args]) => command === "gh" && args[1] === "create",
    )[1];
    assert.ok(
      create.includes("--draft") &&
        create.includes("--verify-tag") &&
        create.includes(source),
    );
    assert.deepEqual(
      create.slice(create.indexOf("--notes-file") + 2),
      names.map((name) => join(scratch, name)),
    );
    assert.deepEqual(calls.at(-1), [
      "gh",
      [
        "api",
        "--method",
        "PATCH",
        "repos/synveda/synveda/releases/42",
        "-F",
        "draft=false",
      ],
    ]);
    for (const fault of [
      "upload",
      "missing",
      "sbom",
      "tag",
      "source",
      "draft",
      "ambiguous",
    ]) {
      calls.length = 0;
      assert.throws(() => invoke(fault));
      assert.ok(
        !calls.some(
          ([command, args]) =>
            command === "gh" &&
            args[0] === "api" &&
            args[1] === "--method" &&
            args[2] === "PATCH",
        ),
      );
      if (fault === "tag")
        assert.ok(
          !calls.some(
            ([command, args]) => command === "gh" && args[1] === "create",
          ),
        );
    }
    for (const target of [
      "darwin-arm64",
      "darwin-x86_64",
      "linux-arm64",
      "linux-x86_64",
      "windows-arm64",
      "windows-x86_64",
    ]) {
      const file = join(scratch, `synveda-client-report-${target}.json`);
      rmSync(file);
      calls.length = 0;
      assert.throws(() => invoke());
      assert.ok(!calls.some(([command]) => command === "gh"));
      writeFileSync(file, "qualified bytes");
    }
    const file = join(scratch, names[0]);
    rmSync(file);
    symlinkSync(join(scratch, names[1]), file);
    assert.throws(() => regularAssets(scratch, names));
    for (const state of ["new", "starter", undefined])
      assert.throws(() =>
        uploadedAssets({ draft: true, assets: [{ ...expected[0], state }] }, [
          expected[0],
        ]),
      );
    assert.throws(() => uploadedAssets({ draft: false, assets: [] }, []));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

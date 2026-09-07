import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_SCHEMA,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ENDPOINT_TOPOLOGY,
  COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY,
  LiveProviderEffectFixtureBlueprintFailure,
  buildColimaLiveProviderEffectFixtureBlueprintStructure,
  liveProviderEffectFixtureBlueprintBytes,
  liveProviderEffectFixtureBlueprintDigest,
  validateColimaLiveProviderEffectFixtureBlueprintStructure,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
  COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
  COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
  liveProviderEffectBytes,
  liveProviderEffectDigest,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect.mjs";
import {
  MAX_FRAME_BYTES,
  MAX_REQUEST_LIFETIME_MILLISECONDS,
  ROLE_EDGES,
  ROLES,
} from "./fixtures/cpr-45-four-role/protocol.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const ZERO_SHA256 = "0".repeat(64);
const ADAPTER_CONTRACT_SHA256 =
  "b476c4f4c9258943fff3745abfc622e822684f95fa0b72d1a311a9cc86bed681";
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
const BLUEPRINT_MODULE_NAME =
  "clean-engine-live-provider-effect-fixture-blueprint.mjs";
const BLUEPRINT_STATIC_IMPORTERS = new Set([
  join(ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
  join(
    ROOT,
    "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
  ),
  join(ROOT, "scripts/clean-engine-state.test.mjs"),
]);
const BLUEPRINT_REFERENCE_ONLY = new Set([
  join(ROOT, "scripts/check-cpr-45-four-role-boundary.test.mjs"),
  join(
    ROOT,
    "scripts/clean-engine-live-provider-effect-fixture-adapter.test.mjs",
  ),
]);
const BLUEPRINT_FORBIDDEN_AUTHORITY_SURFACES = [
  "deploy/compose/scripts/clean-engine-acceptance.sh",
  "deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
  "deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
  "deploy/compose/scripts/clean-engine-receipts.mjs",
  "deploy/compose/scripts/compose.sh",
  "scripts/compose-lifecycle.test.mjs",
].map((path) => join(ROOT, path));

function digest(label) {
  return liveProviderEffectDigest(Buffer.from(`blueprint-test:${label}`, "utf8"));
}

function valueDigest(value) {
  return liveProviderEffectDigest(liveProviderEffectBytes(value));
}

function inputFixture() {
  return {
    adapter_contract_sha256: ADAPTER_CONTRACT_SHA256,
    architecture: "arm64",
    component_manifest:
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS.map(
        (kind) => ({ kind, sha256: digest(`component:${kind}`) }),
      ),
    endpoint_bindings:
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ENDPOINT_TOPOLOGY.map((entry) => ({
        docker_context_namespace_role:
          entry.docker_context_namespace_role,
        docker_context_path_identity_hmac_sha256:
          entry.endpoint_kind === "engine-api"
            ? digest("docker-context-path")
            : ZERO_SHA256,
        docker_context_sha256:
          entry.endpoint_kind === "engine-api"
            ? digest("docker-context")
            : ZERO_SHA256,
        endpoint_kind: entry.endpoint_kind,
        final_challenge_commitment_sha256: digest(
          `endpoint-final:${entry.endpoint_kind}`,
        ),
        initial_challenge_commitment_sha256: digest(
          `endpoint-initial:${entry.endpoint_kind}`,
        ),
        namespace_role: entry.namespace_role,
        path_identity_hmac_sha256: digest(`endpoint:${entry.endpoint_kind}`),
      })),
    environment_sha256: digest("environment"),
    fixture_id: "a".repeat(32),
    operation_contract_sha256:
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
    operation_kind: COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
    platform: "darwin",
    role_bindings: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY.map(
      (entry) => ({
        argv_sha256: digest(`argv:${entry.role}`),
        cwd_identity_sha256: digest("cwd"),
        public_key_spki_sha256: digest(`public-key:${entry.role}`),
        role: entry.role,
        role_challenge_sha256: digest(`role-challenge:${entry.role}`),
        uid: "501",
      }),
    ),
    planned_quiescence_fence_sha256: digest("quiescence-fence"),
    planned_start_attempt_sha256: digest("start-attempt"),
  };
}

function clone(value) {
  return structuredClone(value);
}

function setPath(value, path, next) {
  let target = value;
  for (const part of path.slice(0, -1)) target = target[part];
  target[path.at(-1)] = next;
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
      } else if (entry.isFile()) paths.push(child);
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

function expectRefusal(callback, pattern = /live provider effect fixture/u) {
  assert.throws(callback, (error) => {
    assert.ok(error instanceof LiveProviderEffectFixtureBlueprintFailure);
    assert.match(error.message, pattern);
    assert.ok(new Set([69, 70]).has(error.exitStatus));
    return true;
  });
}

test("the fixture launch blueprint is one closed process-free public projection", () => {
  const input = inputFixture();
  const blueprint =
    buildColimaLiveProviderEffectFixtureBlueprintStructure(input);
  assert.equal(
    validateColimaLiveProviderEffectFixtureBlueprintStructure(
      blueprint,
      input,
    ),
    blueprint,
  );
  assertRecursivelyFrozen(blueprint);
  assert.equal(
    liveProviderEffectFixtureBlueprintBytes(blueprint).at(-1),
    0x0a,
  );
  assert.match(
    liveProviderEffectFixtureBlueprintDigest(blueprint),
    /^[0-9a-f]{64}$/u,
  );
  assert.equal(
    blueprint.schema,
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_SCHEMA,
  );
  assert.equal(
    blueprint.schema,
    "synveda.clean-engine.colima-live-fixture-provider-effect-blueprint.v3",
  );
  assert.equal(
    liveProviderEffectFixtureBlueprintDigest(blueprint),
    "f17a1387395a5b402e7f81331cef1c4fde10792fb55a27e1dc4a9bbe78c4128c",
  );
  assert.equal(blueprint.adapter_contract_sha256, ADAPTER_CONTRACT_SHA256);
  assert.equal(blueprint.authority, "none-process-free-public-projection-only");
  assert.equal(blueprint.evidence_class, "fixture-only");
  assert.equal(
    blueprint.state_integration,
    COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
  );
  assert.deepEqual(
    blueprint.bounds,
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS,
  );
  assert.equal(blueprint.bounds.frame_bytes, MAX_FRAME_BYTES);
  assert.equal(
    blueprint.bounds.request_lifetime_milliseconds,
    MAX_REQUEST_LIFETIME_MILLISECONDS,
  );
  assert.deepEqual(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY.map(
      (entry) => entry.role,
    ),
    ROLES,
  );
  assert.deepEqual(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY.filter(
      (entry) => entry.parent_role !== "state-owner",
    ).map((entry) => [entry.parent_role, entry.role]),
    ROLE_EDGES,
  );
  assert.deepEqual(
    blueprint.role_contracts.map((entry) => entry.role),
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
  );
  assert.deepEqual(
    blueprint.endpoint_contracts.map((entry) => entry.endpoint_kind),
    COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
  );
  assert.deepEqual(
    blueprint.launch_edges.map((entry) => [
      entry.parent_role,
      entry.child_role,
      entry.depth,
    ]),
    [
      ["state-owner", "outer", 0],
      ["outer", "hostagent", 1],
      ["hostagent", "usernet", 2],
      ["hostagent", "ssh-controlmaster", 2],
    ],
  );
  assert.equal(
    blueprint.driver_contract_sha256,
    valueDigest({
      invocation_binding: blueprint.invocation_binding,
      operation_kind: blueprint.operation_kind,
    }),
  );
  assert.deepEqual(
    COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS,
    [
      { kind: "node-runtime", module_path: null },
      {
        kind: "protocol",
        module_path: "../../../scripts/fixtures/cpr-45-four-role/protocol.mjs",
      },
      {
        kind: "role",
        module_path: "../../../scripts/fixtures/cpr-45-four-role/role.mjs",
      },
      {
        kind: "conclusive-adapter",
        module_path: "./clean-engine-live-provider-effect-fixture-adapter.mjs",
      },
    ],
  );
  assert.equal(
    blueprint.invocation_binding.toolchain_sha256,
    blueprint.role_contracts[0].toolchain_sha256,
  );
  assert.equal(
    blueprint.invocation_binding.toolchain_sha256,
    "b9ea3c56b6e830021994d930234da8621e843afae40be9a4d8cdc98463bc7b9b",
  );
  assert.equal(
    blueprint.driver_contract_sha256,
    "300f13403fbbce4a047c511fa2e8929ee7988f95f7a2d6608353ffce666970a7",
  );
  assert.equal(
    new Set(
      blueprint.role_contracts.map(
        (entry) => entry.public_key_spki_sha256,
      ),
    ).size,
    4,
  );
  const serialized = liveProviderEffectFixtureBlueprintBytes(blueprint).toString(
    "utf8",
  );
  assert.deepEqual(
    liveProviderEffectFixtureBlueprintBytes(blueprint),
    liveProviderEffectBytes(blueprint),
  );
  for (const forbidden of [
    "/tmp/",
    "private_key",
    "binding_key",
    "launch_nonce",
    "process_spawn_authorized",
    "provider_adapter_execution_authorized",
  ]) {
    assert.doesNotMatch(serialized, new RegExp(forbidden, "u"));
  }
});

test("every public driver commitment is bound and class confusion is refused", () => {
  const input = inputFixture();
  const blueprint =
    buildColimaLiveProviderEffectFixtureBlueprintStructure(input);
  const inputMutations = [
    [["architecture"], "x64"],
    [["platform"], "linux"],
    [["fixture_id"], "b".repeat(32)],
    [["adapter_contract_sha256"], digest("other-adapter-contract")],
    [["planned_start_attempt_sha256"], digest("other-attempt")],
    [
      ["planned_quiescence_fence_sha256"],
      digest("other-quiescence-fence"),
    ],
    [["environment_sha256"], digest("other-environment")],
    [["component_manifest", 1, "sha256"], digest("other-blueprint")],
    [["component_manifest", 3, "sha256"], digest("other-adapter")],
    [["role_bindings", 2, "argv_sha256"], digest("other-argv")],
    [["role_bindings", 3, "public_key_spki_sha256"], digest("other-key")],
    [
      ["endpoint_bindings", 1, "path_identity_hmac_sha256"],
      digest("other-endpoint"),
    ],
  ];
  for (const [path, next] of inputMutations) {
    const changedInput = clone(input);
    setPath(changedInput, path, next);
    expectRefusal(() =>
      validateColimaLiveProviderEffectFixtureBlueprintStructure(
        blueprint,
        changedInput,
      ),
    );
  }
  for (const [path, next] of [
    [["operation_kind"], "colima-live-provider-effect-v1"],
    [["operation_contract_sha256"], digest("production-contract")],
    [["component_manifest", 0, "kind"], "provider-binary"],
    [["component_manifest", 3, "kind"], "provider-adapter"],
    [["role_bindings", 0, "role"], "hostagent"],
    [["role_bindings", 0, "uid"], "-1"],
    [["role_bindings", 1, "uid"], "502"],
    [["endpoint_bindings", 0, "endpoint_kind"], "engine-api"],
    [["endpoint_bindings", 1, "docker_context_sha256"], digest("forbidden")],
  ]) {
    const changedInput = clone(input);
    setPath(changedInput, path, next);
    expectRefusal(() =>
      buildColimaLiveProviderEffectFixtureBlueprintStructure(changedInput),
    );
  }
  const changedBlueprint = clone(blueprint);
  changedBlueprint.launch_edges[0].parent_role = "fixture-supervisor";
  expectRefusal(() =>
    validateColimaLiveProviderEffectFixtureBlueprintStructure(
      changedBlueprint,
      input,
    ),
  );
});

test("hostile closed-data shapes fail before blueprint property evaluation", () => {
  const input = inputFixture();
  let traps = 0;
  const proxied = new Proxy(input, {
    ownKeys() {
      traps += 1;
      return [];
    },
  });
  expectRefusal(
    () => buildColimaLiveProviderEffectFixtureBlueprintStructure(proxied),
    /proxy/u,
  );
  assert.equal(traps, 0);

  const cyclic = inputFixture();
  cyclic.role_bindings.push(cyclic);
  expectRefusal(
    () => buildColimaLiveProviderEffectFixtureBlueprintStructure(cyclic),
    /cycle/u,
  );

  const accessor = inputFixture();
  Object.defineProperty(accessor, "platform", {
    enumerable: true,
    get() {
      throw new Error("accessor ran");
    },
  });
  expectRefusal(
    () => buildColimaLiveProviderEffectFixtureBlueprintStructure(accessor),
    /accessor/u,
  );
});

test("the blueprint module has no observation, executor or authority imports", () => {
  const path = join(
    ROOT,
    "deploy/compose/scripts/clean-engine-live-provider-effect-fixture-blueprint.mjs",
  );
  const source = readFileSync(path, "utf8");
  for (const forbidden of [
    "node:child_process",
    "node:fs",
    "node:net",
    "node:http",
    "node:https",
    "clean-engine-state.mjs",
    "clean-engine-provider-adapter-registry.mjs",
    "clean-engine-provider-process-contract.mjs",
    "clean-engine-live-provider-effect.mjs",
  ]) {
    assert.doesNotMatch(source, new RegExp(forbidden.replaceAll(".", "\\."), "u"));
  }
  assert.doesNotMatch(source, /\b(?:spawn|exec|fork)\s*\(/u);
  assert.doesNotMatch(source, /process\.(?:env|argv|execPath)/u);
  assert.doesNotMatch(source, /\bimport\s*\(/u);
  assert.doesNotMatch(source, /\bimport\s*["']/u);
  assert.doesNotMatch(source, /\brequire\s*\(/u);
  assert.deepEqual(
    [...source.matchAll(/from\s+["']([^"']+)["']/gu)].map(
      (match) => match[1],
    ),
    ["node:crypto", "node:util/types"],
  );
  const authorityPaths = [
    ...AUTHORITY_ROOTS.flatMap(firstPartyFiles),
    ...ROOT_AUTHORITY_FILES,
  ];
  assert.ok(authorityPaths.length > 100, "authority scan was unexpectedly small");
  const references = authorityPaths.filter((candidate) =>
    textSource(candidate)?.includes(BLUEPRINT_MODULE_NAME),
  );
  assert.deepEqual(references.map((candidate) => relative(ROOT, candidate)).sort(), [
    "deploy/compose/scripts/clean-engine-state.mjs",
    "scripts/check-cpr-45-four-role-boundary.test.mjs",
    "scripts/clean-engine-live-provider-effect-fixture-adapter.test.mjs",
    "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
    "scripts/clean-engine-state.test.mjs",
  ]);
  const quotedBlueprint =
    /["'][^"'\n]*clean-engine-live-provider-effect-fixture-blueprint\.mjs["']/u;
  const staticBlueprintImport =
    /\bfrom\s+["'][^"'\n]*clean-engine-live-provider-effect-fixture-blueprint\.mjs["']/u;
  const nonStaticBlueprintImports = [
    /\bimport\s*["'][^"'\n]*clean-engine-live-provider-effect-fixture-blueprint\.mjs["']/u,
    /\bimport\s*\(\s*["'][^"'\n]*clean-engine-live-provider-effect-fixture-blueprint\.mjs["']/u,
    /\brequire\s*\(\s*["'][^"'\n]*clean-engine-live-provider-effect-fixture-blueprint\.mjs["']/u,
  ];
  for (const importer of BLUEPRINT_STATIC_IMPORTERS) {
    const importerSource = readFileSync(importer, "utf8");
    assert.match(importerSource, staticBlueprintImport, importer);
    for (const pattern of nonStaticBlueprintImports) {
      assert.doesNotMatch(importerSource, pattern, importer);
    }
  }
  for (const referenceOnly of BLUEPRINT_REFERENCE_ONLY) {
    const referenceOnlySource = readFileSync(referenceOnly, "utf8");
    assert.match(referenceOnlySource, quotedBlueprint);
    assert.doesNotMatch(referenceOnlySource, staticBlueprintImport);
    for (const pattern of nonStaticBlueprintImports) {
      assert.doesNotMatch(referenceOnlySource, pattern);
    }
  }
  for (const boundary of BLUEPRINT_FORBIDDEN_AUTHORITY_SURFACES) {
    assert.doesNotMatch(
      readFileSync(boundary, "utf8"),
      quotedBlueprint,
      boundary,
    );
  }
});

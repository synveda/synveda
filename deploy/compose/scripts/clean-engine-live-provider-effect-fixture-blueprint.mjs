#!/usr/bin/env node
import { createHash } from "node:crypto";
import { isProxy } from "node:util/types";

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_SCHEMA =
  "synveda.clean-engine.colima-live-fixture-provider-effect-blueprint.v1";

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS =
  Object.freeze([
    "node-runtime",
    "protocol",
    "role",
  ]);

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENT_LOCATORS =
  Object.freeze([
    Object.freeze({ kind: "node-runtime", module_path: null }),
    Object.freeze({
      kind: "protocol",
      module_path: "../../../scripts/fixtures/cpr-45-four-role/protocol.mjs",
    }),
    Object.freeze({
      kind: "role",
      module_path: "../../../scripts/fixtures/cpr-45-four-role/role.mjs",
    }),
  ]);

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS =
  Object.freeze({
    endpoint_count: 4,
    frame_bytes: 4_096,
    launch_edge_count: 4,
    maximum_depth: 2,
    process_lifetime_milliseconds: 45_000,
    request_lifetime_milliseconds: 18_000,
    role_count: 4,
  });

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY =
  Object.freeze([
    Object.freeze({
      children: Object.freeze(["hostagent"]),
      depth: 0,
      parent_role: "state-owner",
      role: "outer",
      sequence: 0,
    }),
    Object.freeze({
      children: Object.freeze(["usernet", "ssh-controlmaster"]),
      depth: 1,
      parent_role: "outer",
      role: "hostagent",
      sequence: 1,
    }),
    Object.freeze({
      children: Object.freeze([]),
      depth: 2,
      parent_role: "hostagent",
      role: "usernet",
      sequence: 2,
    }),
    Object.freeze({
      children: Object.freeze([]),
      depth: 2,
      parent_role: "hostagent",
      role: "ssh-controlmaster",
      sequence: 3,
    }),
  ]);

export const COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ENDPOINT_TOPOLOGY =
  Object.freeze([
    Object.freeze({
      docker_context_namespace_role: "none",
      endpoint_kind: "hostagent-control",
      namespace_role: "lima-home-namespace",
      role: "hostagent",
    }),
    Object.freeze({
      docker_context_namespace_role: "none",
      endpoint_kind: "usernet-control",
      namespace_role: "lima-home-namespace",
      role: "usernet",
    }),
    Object.freeze({
      docker_context_namespace_role: "none",
      endpoint_kind: "ssh-control",
      namespace_role: "temporary-namespace",
      role: "ssh-controlmaster",
    }),
    Object.freeze({
      docker_context_namespace_role: "docker-config-namespace",
      endpoint_kind: "engine-api",
      namespace_role: "colima-home-namespace",
      role: "hostagent",
    }),
  ]);

const FIXTURE_OPERATION_KIND = "colima-live-fixture-provider-effect-v1";
const FIXTURE_OPERATION_CONTRACT_SHA256 =
  "2926b334f9f63a665fc3648e32e3438627bff04a9dd90d1fed9b71e3d23aeae8";
const EFFECT_STATE_INTEGRATION = "mutation-journal-v7-sibling-effect-only";
const ROLES = Object.freeze([
  "outer",
  "hostagent",
  "usernet",
  "ssh-controlmaster",
]);
const ENDPOINTS = Object.freeze([
  "hostagent-control",
  "usernet-control",
  "ssh-control",
  "engine-api",
]);
const INPUT_FIELDS = Object.freeze([
  "architecture",
  "component_manifest",
  "endpoint_bindings",
  "environment_sha256",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "planned_quiescence_fence_sha256",
  "planned_start_attempt_sha256",
  "platform",
  "role_bindings",
]);
const BLUEPRINT_FIELDS = Object.freeze([
  "architecture",
  "authority",
  "bounds",
  "branch",
  "component_manifest",
  "driver_contract_sha256",
  "endpoint_contracts",
  "evidence_class",
  "fixture_id",
  "invocation_binding",
  "launch_edges",
  "operation_contract_sha256",
  "operation_kind",
  "planned_quiescence_fence_sha256",
  "planned_start_attempt_sha256",
  "platform",
  "role_contracts",
  "schema",
  "state_integration",
]);
const COMPONENT_FIELDS = Object.freeze(["kind", "sha256"]);
const ROLE_BINDING_FIELDS = Object.freeze([
  "argv_sha256",
  "cwd_identity_sha256",
  "public_key_spki_sha256",
  "role",
  "role_challenge_sha256",
  "uid",
]);
const ENDPOINT_BINDING_FIELDS = Object.freeze([
  "docker_context_namespace_role",
  "docker_context_path_identity_hmac_sha256",
  "docker_context_sha256",
  "endpoint_kind",
  "final_challenge_commitment_sha256",
  "initial_challenge_commitment_sha256",
  "namespace_role",
  "path_identity_hmac_sha256",
]);
const INVOCATION_FIELDS = Object.freeze([
  "argv_sha256",
  "cwd_identity_sha256",
  "environment_sha256",
  "executable_sha256",
  "toolchain_sha256",
]);
const ROLE_CONTRACT_FIELDS = Object.freeze([
  ...INVOCATION_FIELDS,
  "depth",
  "parent_role",
  "public_key_spki_sha256",
  "role",
  "role_challenge_sha256",
  "uid",
]);
const ENDPOINT_CONTRACT_FIELDS = Object.freeze([
  ...ENDPOINT_BINDING_FIELDS,
  "role",
]);
const LAUNCH_EDGE_FIELDS = Object.freeze([
  "child_public_key_spki_sha256",
  "child_role",
  "depth",
  "parent_role",
  "role_contract_sha256",
  "sequence",
]);
const BOUNDS_FIELDS = Object.freeze(
  Object.keys(COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS),
);
const ZERO_SHA256 = "0".repeat(64);
const U32_MAX = 4_294_967_295n;
const MAX_CANONICAL_DEPTH = 16;
const MAX_CANONICAL_NODES = 1_024;
const MAX_CANONICAL_CONTAINER_ENTRIES = 64;
const MAX_CANONICAL_BYTES = 64 * 1_024;

export class LiveProviderEffectFixtureBlueprintFailure extends Error {
  constructor(message, exitStatus = 69) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 69) {
  throw new LiveProviderEffectFixtureBlueprintFailure(message, exitStatus);
}

function closedDataEntries(value, label) {
  if (isProxy(value)) fail(`${label} contained a proxy`, 70);
  const array = Array.isArray(value);
  const keys = Reflect.ownKeys(value);
  if (keys.length > MAX_CANONICAL_CONTAINER_ENTRIES + (array ? 1 : 0)) {
    fail("fixture blueprint canonical container budget was exceeded", 70);
  }
  if (keys.some((key) => typeof key !== "string")) {
    fail(`${label} contained a symbol property`, 70);
  }
  if (array) {
    const length = Object.getOwnPropertyDescriptor(value, "length");
    if (
      Object.getPrototypeOf(value) !== Array.prototype ||
      length === undefined ||
      !("value" in length) ||
      !Number.isSafeInteger(length.value) ||
      length.value < 0 ||
      keys.length !== length.value + 1
    ) {
      fail(`${label} was not a dense data array`, 70);
    }
    const entries = [];
    for (const key of keys) {
      if (key === "length") continue;
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      const index = Number(key);
      if (
        !/^(?:0|[1-9][0-9]*)$/u.test(key) ||
        !Number.isSafeInteger(index) ||
        index >= length.value ||
        descriptor === undefined ||
        !("value" in descriptor) ||
        descriptor.enumerable !== true
      ) {
        fail(`${label} was not a dense data array`, 70);
      }
      entries.push([index, descriptor.value]);
    }
    entries.sort((left, right) => left[0] - right[0]);
    if (entries.some(([index], position) => index !== position)) {
      fail(`${label} was not a dense data array`, 70);
    }
    return Object.freeze({ entries, kind: "array" });
  }
  if (value === null || Object.getPrototypeOf(value) !== Object.prototype) {
    fail(`${label} did not have the plain data-object prototype`, 70);
  }
  const entries = [];
  for (const key of keys) {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (
      descriptor === undefined ||
      !("value" in descriptor) ||
      descriptor.enumerable !== true
    ) {
      fail(`${label} contained a hidden or accessor property`, 70);
    }
    entries.push([key, descriptor.value]);
  }
  return Object.freeze({ entries, kind: "record" });
}

function debitBytes(context, count) {
  if (count > MAX_CANONICAL_BYTES - context.bytes) {
    fail("fixture blueprint canonical byte budget was exceeded", 70);
  }
  context.bytes += count;
}

function debitString(context, value) {
  const remaining = MAX_CANONICAL_BYTES - context.bytes;
  if (
    remaining < 2 ||
    Buffer.byteLength(value, "utf8") > remaining - 2
  ) {
    fail("fixture blueprint canonical byte budget was exceeded", 70);
  }
  debitBytes(context, Buffer.byteLength(JSON.stringify(value), "utf8"));
}

function preflightClosedDataGraph(value) {
  const active = new WeakSet();
  const context = { bytes: 1, nodes: 0 };
  const pending = [{ depth: 0, kind: "value", value }];
  while (pending.length > 0) {
    const frame = pending.pop();
    if (frame.kind === "leave") {
      active.delete(frame.value);
      continue;
    }
    if (frame.depth > MAX_CANONICAL_DEPTH) {
      fail("fixture blueprint canonical depth was exceeded", 70);
    }
    context.nodes += 1;
    if (context.nodes > MAX_CANONICAL_NODES) {
      fail("fixture blueprint canonical node budget was exceeded", 70);
    }
    const current = frame.value;
    if (current === null) {
      debitBytes(context, 4);
      continue;
    }
    if (typeof current === "string") {
      debitString(context, current);
      continue;
    }
    if (typeof current === "boolean") {
      debitBytes(context, current ? 4 : 5);
      continue;
    }
    if (
      typeof current === "number" &&
      Number.isSafeInteger(current) &&
      !Object.is(current, -0)
    ) {
      debitBytes(context, Buffer.byteLength(String(current), "utf8"));
      continue;
    }
    if (typeof current !== "object") {
      fail("fixture blueprint canonical value was refused", 70);
    }
    if (active.has(current)) {
      fail("fixture blueprint canonical cycle was refused", 70);
    }
    const inspected = closedDataEntries(
      current,
      "fixture blueprint canonical value",
    );
    debitBytes(context, 2 + Math.max(0, inspected.entries.length - 1));
    if (inspected.kind === "record") {
      for (const [key] of inspected.entries) {
        debitString(context, key);
        debitBytes(context, 1);
      }
    }
    active.add(current);
    pending.push({ kind: "leave", value: current });
    for (let index = inspected.entries.length - 1; index >= 0; index -= 1) {
      pending.push({
        depth: frame.depth + 1,
        kind: "value",
        value: inspected.entries[index][1],
      });
    }
  }
}

function canonicalUnchecked(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (
    typeof value === "number" &&
    Number.isSafeInteger(value) &&
    !Object.is(value, -0)
  ) {
    return String(value);
  }
  const inspected = closedDataEntries(
    value,
    "fixture blueprint canonical value",
  );
  if (inspected.kind === "array") {
    return `[${inspected.entries
      .map(([, child]) => canonicalUnchecked(child))
      .join(",")}]`;
  }
  return `{${inspected.entries
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    .map(([key, child]) => `${JSON.stringify(key)}:${canonicalUnchecked(child)}`)
    .join(",")}}`;
}

export function liveProviderEffectFixtureBlueprintBytes(value) {
  preflightClosedDataGraph(value);
  return Buffer.from(`${canonicalUnchecked(value)}\n`, "utf8");
}

export function liveProviderEffectFixtureBlueprintDigest(value) {
  return createHash("sha256")
    .update(liveProviderEffectFixtureBlueprintBytes(value))
    .digest("hex");
}

function exactKeys(value, keys, label) {
  if (
    value === null ||
    Array.isArray(value) ||
    typeof value !== "object" ||
    JSON.stringify(Object.keys(value).sort()) !==
      JSON.stringify([...keys].sort())
  ) {
    fail(`${label} fields were refused`, 70);
  }
}

function nonzeroSha256(value) {
  return (
    typeof value === "string" &&
    /^[0-9a-f]{64}$/u.test(value) &&
    value !== ZERO_SHA256
  );
}

function decimalUid(value) {
  if (
    typeof value !== "string" ||
    !/^(?:0|[1-9][0-9]*)$/u.test(value) ||
    value.length > 10
  ) {
    return false;
  }
  return BigInt(value) <= U32_MAX;
}

function allDistinct(values) {
  return new Set(values).size === values.length;
}

function deepFreeze(value) {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

function topologyByRole(role) {
  return COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ROLE_TOPOLOGY.find(
    (entry) => entry.role === role,
  );
}

function topologyByEndpoint(endpointKind) {
  return COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_ENDPOINT_TOPOLOGY.find(
    (entry) => entry.endpoint_kind === endpointKind,
  );
}

function valueDigest(value) {
  return liveProviderEffectFixtureBlueprintDigest(value);
}

function validateInput(input) {
  liveProviderEffectFixtureBlueprintBytes(input);
  exactKeys(input, INPUT_FIELDS, "live provider effect fixture blueprint input");
  if (
    !new Set(["arm64", "x64"]).has(input.architecture) ||
    !new Set(["darwin", "linux"]).has(input.platform) ||
    !/^[0-9a-f]{32}$/u.test(input.fixture_id) ||
    input.operation_kind !== FIXTURE_OPERATION_KIND ||
    input.operation_contract_sha256 !== FIXTURE_OPERATION_CONTRACT_SHA256 ||
    !nonzeroSha256(input.environment_sha256) ||
    !nonzeroSha256(input.planned_quiescence_fence_sha256) ||
    !nonzeroSha256(input.planned_start_attempt_sha256) ||
    input.planned_quiescence_fence_sha256 ===
      input.planned_start_attempt_sha256
  ) {
    fail("live provider effect fixture blueprint identity was refused");
  }
  if (
    !Array.isArray(input.component_manifest) ||
    input.component_manifest.length !==
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS.length
  ) {
    fail("live provider effect fixture component manifest was refused");
  }
  input.component_manifest.forEach((component, index) => {
    exactKeys(
      component,
      COMPONENT_FIELDS,
      "live provider effect fixture component",
    );
    if (
      component.kind !==
        COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS[index] ||
      !nonzeroSha256(component.sha256)
    ) {
      fail("live provider effect fixture component was refused");
    }
  });
  if (!allDistinct(input.component_manifest.map((entry) => entry.sha256))) {
    fail("live provider effect fixture components were not distinct");
  }
  if (
    !Array.isArray(input.role_bindings) ||
    input.role_bindings.length !== ROLES.length
  ) {
    fail("live provider effect fixture role bindings were refused");
  }
  input.role_bindings.forEach((binding, index) => {
    exactKeys(
      binding,
      ROLE_BINDING_FIELDS,
      "live provider effect fixture role binding",
    );
    if (
      binding.role !== ROLES[index] ||
      !nonzeroSha256(binding.argv_sha256) ||
      !nonzeroSha256(binding.cwd_identity_sha256) ||
      !nonzeroSha256(binding.public_key_spki_sha256) ||
      !nonzeroSha256(binding.role_challenge_sha256) ||
      !decimalUid(binding.uid)
    ) {
      fail("live provider effect fixture role binding was refused");
    }
  });
  if (
    !allDistinct(input.role_bindings.map((entry) => entry.argv_sha256)) ||
    !allDistinct(
      input.role_bindings.map((entry) => entry.public_key_spki_sha256),
    ) ||
    !allDistinct(
      input.role_bindings.map((entry) => entry.role_challenge_sha256),
    ) ||
    new Set(input.role_bindings.map((entry) => entry.cwd_identity_sha256))
      .size !== 1 ||
    new Set(input.role_bindings.map((entry) => entry.uid)).size !== 1
  ) {
    fail("live provider effect fixture role identities were refused");
  }
  if (
    !Array.isArray(input.endpoint_bindings) ||
    input.endpoint_bindings.length !== ENDPOINTS.length
  ) {
    fail("live provider effect fixture endpoint bindings were refused");
  }
  input.endpoint_bindings.forEach((binding, index) => {
    exactKeys(
      binding,
      ENDPOINT_BINDING_FIELDS,
      "live provider effect fixture endpoint binding",
    );
    const expected = topologyByEndpoint(ENDPOINTS[index]);
    const engine = expected.endpoint_kind === "engine-api";
    if (
      binding.endpoint_kind !== expected.endpoint_kind ||
      binding.namespace_role !== expected.namespace_role ||
      binding.docker_context_namespace_role !==
        expected.docker_context_namespace_role ||
      !nonzeroSha256(binding.path_identity_hmac_sha256) ||
      !nonzeroSha256(binding.initial_challenge_commitment_sha256) ||
      !nonzeroSha256(binding.final_challenge_commitment_sha256) ||
      binding.initial_challenge_commitment_sha256 ===
        binding.final_challenge_commitment_sha256 ||
      (engine
        ? !nonzeroSha256(
            binding.docker_context_path_identity_hmac_sha256,
          ) || !nonzeroSha256(binding.docker_context_sha256)
        : binding.docker_context_path_identity_hmac_sha256 !== ZERO_SHA256 ||
          binding.docker_context_sha256 !== ZERO_SHA256)
    ) {
      fail("live provider effect fixture endpoint binding was refused");
    }
  });
  const endpointCommitments = input.endpoint_bindings.flatMap((entry) => [
    entry.path_identity_hmac_sha256,
    entry.docker_context_path_identity_hmac_sha256,
    entry.initial_challenge_commitment_sha256,
    entry.final_challenge_commitment_sha256,
  ]).filter((value) => value !== ZERO_SHA256);
  if (!allDistinct(endpointCommitments)) {
    fail("live provider effect fixture endpoint identities were reused");
  }
  return input;
}

function blueprintFor(input) {
  const componentManifest = input.component_manifest.map((entry) => ({
    ...entry,
  }));
  const toolchainSha256 = valueDigest({
    architecture: input.architecture,
    bounds: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS,
    component_manifest: componentManifest,
    platform: input.platform,
    schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_SCHEMA,
  });
  const nodeSha256 = componentManifest[0].sha256;
  const roleContracts = input.role_bindings.map((binding) => {
    const topology = topologyByRole(binding.role);
    return {
      argv_sha256: binding.argv_sha256,
      cwd_identity_sha256: binding.cwd_identity_sha256,
      depth: topology.depth,
      environment_sha256: input.environment_sha256,
      executable_sha256: nodeSha256,
      parent_role: topology.parent_role,
      public_key_spki_sha256: binding.public_key_spki_sha256,
      role: binding.role,
      role_challenge_sha256: binding.role_challenge_sha256,
      toolchain_sha256: toolchainSha256,
      uid: binding.uid,
    };
  });
  const invocationBinding = Object.fromEntries(
    INVOCATION_FIELDS.map((field) => [field, roleContracts[0][field]]),
  );
  const driverContractSha256 = valueDigest({
    invocation_binding: invocationBinding,
    operation_kind: input.operation_kind,
  });
  const endpointContracts = input.endpoint_bindings.map((binding) => ({
    ...binding,
    role: topologyByEndpoint(binding.endpoint_kind).role,
  }));
  const launchEdges = roleContracts.map((contract, sequence) => ({
    child_public_key_spki_sha256: contract.public_key_spki_sha256,
    child_role: contract.role,
    depth: contract.depth,
    parent_role: contract.parent_role,
    role_contract_sha256: valueDigest(contract),
    sequence,
  }));
  return {
    architecture: input.architecture,
    authority: "none-process-free-public-projection-only",
    bounds: { ...COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_BOUNDS },
    branch: "fixture-effect-attempt-preparation-only",
    component_manifest: componentManifest,
    driver_contract_sha256: driverContractSha256,
    endpoint_contracts: endpointContracts,
    evidence_class: "fixture-only",
    fixture_id: input.fixture_id,
    invocation_binding: invocationBinding,
    launch_edges: launchEdges,
    operation_contract_sha256: input.operation_contract_sha256,
    operation_kind: input.operation_kind,
    planned_quiescence_fence_sha256:
      input.planned_quiescence_fence_sha256,
    planned_start_attempt_sha256: input.planned_start_attempt_sha256,
    platform: input.platform,
    role_contracts: roleContracts,
    schema: COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_SCHEMA,
    state_integration: EFFECT_STATE_INTEGRATION,
  };
}

function validateBlueprintShape(value) {
  liveProviderEffectFixtureBlueprintBytes(value);
  exactKeys(value, BLUEPRINT_FIELDS, "live provider effect fixture blueprint");
  exactKeys(
    value.bounds,
    BOUNDS_FIELDS,
    "live provider effect fixture blueprint bounds",
  );
  if (
    !Array.isArray(value.component_manifest) ||
    value.component_manifest.length !==
      COLIMA_LIVE_PROVIDER_EFFECT_FIXTURE_BLUEPRINT_COMPONENTS.length ||
    !Array.isArray(value.role_contracts) ||
    value.role_contracts.length !== ROLES.length ||
    !Array.isArray(value.launch_edges) ||
    value.launch_edges.length !== ROLES.length ||
    !Array.isArray(value.endpoint_contracts) ||
    value.endpoint_contracts.length !== ENDPOINTS.length
  ) {
    fail("live provider effect fixture blueprint collection was refused", 70);
  }
  value.component_manifest.forEach((entry) =>
    exactKeys(
      entry,
      COMPONENT_FIELDS,
      "live provider effect fixture component",
    ),
  );
  exactKeys(
    value.invocation_binding,
    INVOCATION_FIELDS,
    "live provider effect fixture invocation binding",
  );
  value.role_contracts.forEach((entry) =>
    exactKeys(
      entry,
      ROLE_CONTRACT_FIELDS,
      "live provider effect fixture role contract",
    ),
  );
  value.launch_edges.forEach((entry) =>
    exactKeys(
      entry,
      LAUNCH_EDGE_FIELDS,
      "live provider effect fixture launch edge",
    ),
  );
  value.endpoint_contracts.forEach((entry) =>
    exactKeys(
      entry,
      ENDPOINT_CONTRACT_FIELDS,
      "live provider effect fixture endpoint contract",
    ),
  );
}

export function validateColimaLiveProviderEffectFixtureBlueprintStructure(
  value,
  input,
) {
  validateBlueprintShape(value);
  validateInput(input);
  const expected = blueprintFor(input);
  if (
    !liveProviderEffectFixtureBlueprintBytes(value).equals(
      liveProviderEffectFixtureBlueprintBytes(expected),
    )
  ) {
    fail("live provider effect fixture blueprint changed");
  }
  return value;
}

export function buildColimaLiveProviderEffectFixtureBlueprintStructure(input) {
  validateInput(input);
  const value = blueprintFor(input);
  validateColimaLiveProviderEffectFixtureBlueprintStructure(value, input);
  return deepFreeze(value);
}

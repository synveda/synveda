import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA,
  COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT,
  COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SCHEMA,
  COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CLEANUP_OPERATION_KIND,
  COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA,
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT,
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SCHEMA,
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_CREATE_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_CLASS,
  PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES,
  PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
  PROVIDER_ADAPTER_REGISTRY,
  PROVIDER_ADAPTER_REGISTRY_SCHEMA,
  PROVIDER_ADAPTER_REGISTRY_SHA256,
  PROVIDER_ADAPTER_RESOLUTION_SCHEMA,
  ProviderAdapterRegistryFailure,
  authorizeProviderAdapter,
  authorizeProviderAdapterPlanning,
  providerAdapterRegistryKey,
  resolveProviderAdapter,
  validateColimaLiveCleanupOperationContract,
  validateColimaLiveCreateOperationContract,
  validateProviderAdapterRegistry,
} from "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs";
import {
  COLIMA_LIVE_OBSERVATION_SCHEMA,
  COLIMA_LIVE_REQUIREMENTS_SHA256,
} from "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs";

const CREATE_CONTRACT_SHA256 =
  "34af3ffb3993172112c9218e737db57acb9ad78b8dc0e89240bbcb2d8ab269b5";
const CLEANUP_CONTRACT_SHA256 =
  "47a6ca5a6b76542a05b5631c24eaed5cefaa9ea6c33bcfe1c4aa997e7db393b3";
const REGISTRY_SHA256 =
  "50ebdacaafa2b98de3a9acdd82fc563f3f99045df53a96280515a8e5627ed901";

function clone(value) {
  return structuredClone(value);
}

function tuple(contract, contractSha256) {
  return {
    action: contract.action,
    operation_contract_sha256: contractSha256,
    operation_kind: contract.operation_kind,
    provider_class: contract.provider_class,
  };
}

const CREATE_TUPLE = tuple(
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT,
  COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
);
const CLEANUP_TUPLE = tuple(
  COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT,
  COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256,
);

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof ProviderAdapterRegistryFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

function assertDeepFrozen(value) {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertDeepFrozen(child);
}

test("live operation identities are fresh, exact and content addressed", () => {
  assert.equal(
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SCHEMA,
    "synveda.clean-engine.colima-live-create-operation-contract.v1",
  );
  assert.equal(
    COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SCHEMA,
    "synveda.clean-engine.colima-live-cleanup-operation-contract.v1",
  );
  assert.equal(
    COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA,
    "synveda.clean-engine.colima-live-create-evidence.v1",
  );
  assert.equal(
    COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA,
    "synveda.clean-engine.colima-live-cleanup-evidence.v1",
  );
  assert.equal(COLIMA_LIVE_CREATE_OPERATION_KIND, "colima-vz-docker-live-create-v1");
  assert.equal(COLIMA_LIVE_CLEANUP_OPERATION_KIND, "colima-vz-docker-live-cleanup-v1");
  assert.equal(COLIMA_LIVE_PROVIDER_CLASS, "colima-vz-docker-live");
  assert.equal(COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256, CREATE_CONTRACT_SHA256);
  assert.equal(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT_SHA256, CLEANUP_CONTRACT_SHA256);
  assert.equal(PROVIDER_ADAPTER_REGISTRY_SHA256, REGISTRY_SHA256);
  assert.doesNotMatch(
    `${COLIMA_LIVE_CREATE_OPERATION_KIND}\n${COLIMA_LIVE_CLEANUP_OPERATION_KIND}`,
    /controlled|deterministic|fake/u,
  );
});

test("create grants only state planning while cleanup remains deny only", () => {
  assert.equal(
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT.preparation_observation_schema,
    COLIMA_LIVE_OBSERVATION_SCHEMA,
  );
  assert.equal(
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT.requirements_sha256,
    COLIMA_LIVE_REQUIREMENTS_SHA256,
  );
  assert.equal(
    COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT.requirements_sha256,
    COLIMA_LIVE_REQUIREMENTS_SHA256,
  );
  assert.equal(
    COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT.create_operation_contract_sha256,
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT_SHA256,
  );
  assert.equal(
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT.state_integration,
    "mutation-journal-v3-plan-only",
  );
  assert.equal(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT.state_integration, "not-authorized");
  assert.deepEqual(PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES, {
    execution_authorized: false,
    finalization_eligible: false,
    lifecycle_exposure_authorized: false,
    recovery_authorized: false,
    state_planning_authorized: false,
  });
  assert.deepEqual(PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES, {
    execution_authorized: false,
    finalization_eligible: false,
    lifecycle_exposure_authorized: false,
    recovery_authorized: false,
    state_planning_authorized: true,
  });
  assert.equal(
    COLIMA_LIVE_CREATE_OPERATION_CONTRACT.capabilities,
    PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
  );
  assert.equal(
    COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT.capabilities,
    PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES,
  );
  validateColimaLiveCreateOperationContract(COLIMA_LIVE_CREATE_OPERATION_CONTRACT);
  validateColimaLiveCleanupOperationContract(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT);
  assertDeepFrozen(COLIMA_LIVE_CREATE_OPERATION_CONTRACT);
  assertDeepFrozen(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT);
});

test("the registry contains exactly two unique exact tuple keys", () => {
  assert.equal(PROVIDER_ADAPTER_REGISTRY.schema, PROVIDER_ADAPTER_REGISTRY_SCHEMA);
  assert.equal(
    PROVIDER_ADAPTER_REGISTRY.selection_policy,
    "exact-action-kind-contract-class-tuple-v1",
  );
  assert.equal(PROVIDER_ADAPTER_REGISTRY.entries.length, 2);
  assert.deepEqual(
    PROVIDER_ADAPTER_REGISTRY.entries.map((entry) => entry.action),
    ["provider-create", "provider-cleanup"],
  );
  assert.equal(PROVIDER_ADAPTER_REGISTRY.entries[0].key_sha256, providerAdapterRegistryKey(CREATE_TUPLE));
  assert.equal(PROVIDER_ADAPTER_REGISTRY.entries[1].key_sha256, providerAdapterRegistryKey(CLEANUP_TUPLE));
  assert.equal(
    new Set(PROVIDER_ADAPTER_REGISTRY.entries.map((entry) => entry.key_sha256)).size,
    2,
  );
  validateProviderAdapterRegistry(PROVIDER_ADAPTER_REGISTRY);
  assertDeepFrozen(PROVIDER_ADAPTER_REGISTRY);
});

test("exact create and cleanup tuples resolve only their closed capabilities", () => {
  for (const [request, evidenceSchema, capabilities, stateIntegration] of [
    [
      CREATE_TUPLE,
      COLIMA_LIVE_CREATE_EVIDENCE_SCHEMA,
      PROVIDER_ADAPTER_PLAN_ONLY_CAPABILITIES,
      "mutation-journal-v3-plan-only",
    ],
    [
      CLEANUP_TUPLE,
      COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA,
      PROVIDER_ADAPTER_DENY_ONLY_CAPABILITIES,
      "not-authorized",
    ],
  ]) {
    const resolution = resolveProviderAdapter(request);
    assert.equal(resolution.schema, PROVIDER_ADAPTER_RESOLUTION_SCHEMA);
    assert.equal(resolution.action, request.action);
    assert.equal(resolution.operation_kind, request.operation_kind);
    assert.equal(
      resolution.operation_contract_sha256,
      request.operation_contract_sha256,
    );
    assert.equal(resolution.provider_class, request.provider_class);
    assert.equal(resolution.evidence_schema, evidenceSchema);
    assert.equal(resolution.requirements_sha256, COLIMA_LIVE_REQUIREMENTS_SHA256);
    assert.equal(resolution.registry_sha256, PROVIDER_ADAPTER_REGISTRY_SHA256);
    assert.equal(resolution.state_integration, stateIntegration);
    assert.deepEqual(resolution.capabilities, capabilities);
    assertDeepFrozen(resolution);
  }
});

test("every tuple field participates in the content-addressed key", () => {
  const original = providerAdapterRegistryKey(CREATE_TUPLE);
  const mutations = [
    { ...CREATE_TUPLE, action: "provider-cleanup" },
    { ...CREATE_TUPLE, operation_kind: `${CREATE_TUPLE.operation_kind}-other` },
    { ...CREATE_TUPLE, operation_contract_sha256: "f".repeat(64) },
    { ...CREATE_TUPLE, provider_class: `${CREATE_TUPLE.provider_class}-other` },
  ];
  for (const changed of mutations) {
    assert.notEqual(providerAdapterRegistryKey(changed), original);
  }
});

test("partial, extra, malformed and unknown tuples fail closed", () => {
  const mutations = [
    (value) => {
      delete value.action;
    },
    (value) => {
      value.extra = "field";
    },
    (value) => {
      value.action = "start";
    },
    (value) => {
      value.operation_kind = null;
    },
    (value) => {
      value.operation_contract_sha256 = "A".repeat(64);
    },
    (value) => {
      value.provider_class = "controlled-background-fake";
    },
  ];
  for (const mutate of mutations) {
    const changed = clone(CREATE_TUPLE);
    mutate(changed);
    expectRefusal(() => resolveProviderAdapter(changed));
  }
  expectRefusal(() => resolveProviderAdapter(null));
  expectRefusal(() => resolveProviderAdapter([]));
});

test("crossed create and cleanup tuple fields never select an adapter", () => {
  for (const createField of ["operation_kind", "operation_contract_sha256"]) {
    const changed = clone(CREATE_TUPLE);
    changed[createField] = CLEANUP_TUPLE[createField];
    expectRefusal(() => resolveProviderAdapter(changed), 69);
  }
  for (const cleanupField of ["operation_kind", "operation_contract_sha256"]) {
    const changed = clone(CLEANUP_TUPLE);
    changed[cleanupField] = CREATE_TUPLE[cleanupField];
    expectRefusal(() => resolveProviderAdapter(changed), 69);
  }
  expectRefusal(
    () => resolveProviderAdapter({ ...CREATE_TUPLE, action: CLEANUP_TUPLE.action }),
    69,
  );
});

test("create contract drift and capability escalation are refused", () => {
  const mutations = [
    (value) => {
      value.action = "provider-cleanup";
    },
    (value) => {
      value.capabilities.execution_authorized = true;
    },
    (value) => {
      value.evidence_schema = COLIMA_LIVE_CLEANUP_EVIDENCE_SCHEMA;
    },
    (value) => {
      value.operation_kind = COLIMA_LIVE_CLEANUP_OPERATION_KIND;
    },
    (value) => {
      value.preparation_observation_schema = "other";
    },
    (value) => {
      value.requirements_sha256 = "f".repeat(64);
    },
    (value) => {
      value.state_integration = "mutation-journal-v2";
    },
    (value) => {
      value.extra = false;
    },
  ];
  for (const mutate of mutations) {
    const changed = clone(COLIMA_LIVE_CREATE_OPERATION_CONTRACT);
    mutate(changed);
    expectRefusal(() => validateColimaLiveCreateOperationContract(changed));
  }
});

test("cleanup contract cannot detach from its create contract or gain capability", () => {
  const mutations = [
    (value) => {
      value.create_operation_contract_sha256 = "f".repeat(64);
    },
    (value) => {
      value.capabilities.recovery_authorized = true;
    },
    (value) => {
      value.capabilities.state_planning_authorized = true;
    },
    (value) => {
      value.operation_kind = COLIMA_LIVE_CREATE_OPERATION_KIND;
    },
    (value) => {
      value.provider_class = "controlled-background-fake";
    },
    (value) => {
      delete value.evidence_schema;
    },
  ];
  for (const mutate of mutations) {
    const changed = clone(COLIMA_LIVE_CLEANUP_OPERATION_CONTRACT);
    mutate(changed);
    expectRefusal(() => validateColimaLiveCleanupOperationContract(changed));
  }
});

test("registry mutation, reordering and duplicate selection are refused", () => {
  const mutations = [
    (value) => {
      value.entries.reverse();
    },
    (value) => {
      value.entries[0] = clone(value.entries[1]);
    },
    (value) => {
      value.entries[0].adapter_id = "other";
    },
    (value) => {
      value.entries[0].key_sha256 = "f".repeat(64);
    },
    (value) => {
      value.entries[0].capabilities.lifecycle_exposure_authorized = true;
    },
    (value) => {
      value.entries[1].requirements_sha256 = "f".repeat(64);
    },
    (value) => {
      value.entries[1].state_integration = "mutation-journal-v2";
    },
    (value) => {
      value.selection_policy = "first-match";
    },
    (value) => {
      value.extra = [];
    },
  ];
  for (const mutate of mutations) {
    const changed = clone(PROVIDER_ADAPTER_REGISTRY);
    mutate(changed);
    expectRefusal(() => validateProviderAdapterRegistry(changed));
  }
});

test("planning authority is create-only and execution remains denied", () => {
  const planning = authorizeProviderAdapterPlanning(CREATE_TUPLE);
  assert.equal(planning.capabilities.state_planning_authorized, true);
  assert.equal(planning.capabilities.execution_authorized, false);
  expectRefusal(() => authorizeProviderAdapterPlanning(CLEANUP_TUPLE), 69);
  expectRefusal(() => authorizeProviderAdapter(CREATE_TUPLE), 69);
  expectRefusal(() => authorizeProviderAdapter(CLEANUP_TUPLE), 69);
});

test("refusals do not reproduce hostile tuple values", () => {
  const sensitive = "sensitive-provider-value-never-render";
  assert.throws(
    () =>
      resolveProviderAdapter({
        ...CREATE_TUPLE,
        operation_kind: sensitive,
      }),
    (error) => {
      assert.ok(error instanceof ProviderAdapterRegistryFailure);
      assert.equal(error.message.includes(sensitive), false);
      return true;
    },
  );
});

test("the registry has no execution, state implementation, receipt or fake-provider seam", () => {
  const registrySource = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-provider-adapter-registry.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(registrySource, /from "node:crypto"/u);
  assert.match(registrySource, /from "\.\/clean-engine-colima-live-contract\.mjs"/u);
  assert.doesNotMatch(registrySource, /node:child_process|node:fs|node:net|node:http/u);
  assert.doesNotMatch(
    registrySource,
    /clean-engine-state|clean-engine-receipts|clean-engine-provider-process-contract/u,
  );
  assert.doesNotMatch(registrySource, /\b(?:spawn|spawnSync|exec|execFile|fork)\s*\(/u);
  assert.doesNotMatch(registrySource, /process\.(?:argv|env)/u);

  const stateSource = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  assert.match(stateSource, /clean-engine-live-provider-plan/u);
  assert.doesNotMatch(stateSource, /clean-engine-provider-adapter-registry/u);
  assert.doesNotMatch(
    stateSource,
    /colima-vz-docker-live-(?:create|cleanup)-v1/u,
  );

  for (const relativePath of [
    "../deploy/compose/scripts/clean-engine-receipts.mjs",
    "../deploy/compose/scripts/clean-engine-provider-process-contract.mjs",
  ]) {
    const source = readFileSync(new URL(relativePath, import.meta.url), "utf8");
    assert.doesNotMatch(source, /clean-engine-provider-adapter-registry/u);
    assert.doesNotMatch(source, /colima-vz-docker-live-(?:create|cleanup)-v1/u);
  }
});

test("the supported lifecycle remains preparation only", () => {
  const lifecycle = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-acceptance.sh", import.meta.url),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(lifecycle, /provider-adapter-registry|colima-vz-docker-live/u);
  assert.doesNotMatch(lifecycle, /(?:execute|recover|run|start)\)/u);
});

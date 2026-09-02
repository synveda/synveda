#!/usr/bin/env node
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import {
  ReceiptFailure,
  buildEnvironmentManifest,
  canonicalBytes,
  createFinalization,
  createNextReceipt,
  receiptFileName,
  receiptSuccessPath,
  sha256,
  validateReceiptChain,
} from "../deploy/compose/scripts/clean-engine-receipts.mjs";

const fixtureId = "a".repeat(32);
const hash = (value) => createHash("sha256").update(value).digest("hex");

function plan(candidateSha256 = sha256(canonicalBytes(candidate()))) {
  return {
    fixture_id: fixtureId,
    outcome: "passed",
    phase: "plan",
    previous_sha256: "0".repeat(64),
    result: {
      candidate_sha256: candidateSha256,
      project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
      provider: "colima",
      provider_resource: `synveda-cpr45-${fixtureId}`,
      state_device: "42",
      state_inode: "73",
    },
    schema: "synveda.clean-engine.receipt.v3",
    sequence: 0,
  };
}

function result(phase) {
  switch (phase) {
    case "provider-create-intent":
      return {
        cleanup_command: "colima-delete-data-force",
        preexisting_resource: "absent",
        provider_contract_sha256: "2".repeat(64),
        provider_resource: `synveda-cpr45-${fixtureId}`,
        provider_root_key: `sv-c45-${fixtureId.slice(0, 16)}`,
      };
    case "provider-create-passed":
      return {
        evidence_class: "deterministic-fixture",
        engine_identity_sha256: "3".repeat(64),
        initial_containers: 0,
        initial_images: 0,
        initial_networks: ["bridge", "host", "none"],
        initial_volumes: 0,
        platform: "darwin-arm64-colima-vz",
        provider_contract_sha256: "2".repeat(64),
        provider_name: "colima",
        provider_version: "0.10.3",
        runtime_client_version: "29.4.0",
        runtime_name: "docker",
        runtime_server_version: "29.4.0",
        socket_contract: "receipt-owned-unix",
      };
    case "registry-intent":
      return {
        authentication: "basic-bcrypt-cost-12",
        container: `synveda-cpr45-registry-${fixtureId.slice(0, 16)}`,
        image:
          "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33",
        port: 54_321,
        transport: "tls-loopback",
      };
    case "registry-passed":
      return {
        authenticated_pull: true,
        authenticated_push: true,
        basic_challenge: true,
        canary_image_sha256: "4".repeat(64),
        certificate_sha256: "5".repeat(64),
        negative_status: 401,
        unauthenticated_pull_rejected: true,
        wrong_password_rejected: true,
      };
    case "proxy-intent":
      return {
        config: "synthetic-nonsecret-v1",
        expected_injected_variables: 10,
        expected_runtime_empty_variables: 10,
      };
    case "proxy-passed":
      return { auth_preserved: true, injected_variables: 10, runtime_empty_variables: 10 };
    case "builder-canary-intent":
      return {
        builder: `synveda-cpr45-canary-${fixtureId.slice(0, 16)}`,
        canonical_builder: "default",
        endpoint: "loopback-inert-tcp",
        expected_connections: 0,
      };
    case "builder-canary-passed":
      return {
        canonical_builder_driver: "docker",
        canonical_builder_endpoint: "default",
        connections: 0,
        private_buildx_removed: true,
      };
    case "compose-browser-intent":
      return {
        capture: "disabled",
        profiles: ["browser-acceptance", "demo"],
        project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
      };
    case "compose-browser-passed":
      return {
        admin_admitted: true,
        browser_exit: 0,
        captured_artifacts: 0,
        container_proxy_empty_variables: 10,
        logout: true,
        pkce_s256: true,
      };
    case "project-cleanup-intent":
      return {
        project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
        resolver: "managed-test-block",
        scope: "exact-receipt-owned-only",
      };
    case "project-cleanup-passed":
      return {
        builder_canary_absent: true,
        project_absent: true,
        registry_absent: true,
        resolver_absent: true,
        runtime_secrets_absent: true,
      };
    case "provider-cleanup-intent":
      return {
        command: "colima-delete-data-force",
        provider_resource: `synveda-cpr45-${fixtureId}`,
        scope: "exact-receipt-owned-only",
      };
    case "provider-cleanup-passed":
      return {
        context_absent: true,
        inert_staging_absent: true,
        provider_absent: true,
        runtime_root_absent: true,
        socket_absent: true,
        source_closure_unchanged: true,
      };
    case "finalize-passed":
      throw new Error("finalize result needs the manifest hash");
    default:
      throw new Error(`missing fixture result for ${phase}`);
  }
}

function successBeforeFinalize(initialPlan = plan()) {
  const receipts = [initialPlan];
  for (const phase of receiptSuccessPath.slice(1, -1)) {
    receipts.push(createNextReceipt(receipts, fixtureId, phase, result(phase)));
  }
  return receipts;
}

function candidate() {
  return {
    created_at: "2026-09-01T00:00:00.000Z",
    excluded_claims: [
      "disaster-recovery",
      "docker-desktop-parity",
      "enterprise-certification",
      "high-availability",
      "host-loss-tolerance",
      "native-linux-parity",
      "production-saas-readiness",
      "reference-https",
      "signed-provenance",
      "zero-downtime-upgrades",
    ],
    feature: "CPR-45",
    fixtures: {
      builder_canary: "ambient-remote-inert-zero-read-v1",
      docker_proxy: "synthetic-nonsecret-v1",
      registry_authentication: "one-run-basic-bcrypt",
      registry_image:
        "registry:3.1.1@sha256:1be55279f18a2fe1a74edf2664cac61c1bea305b7b4642dab412e7affdcb3e33",
      registry_transport: "loopback-tls-ephemeral",
    },
    kind: "synveda-cpr45-clean-engine-candidate",
    requested_assertions: [
      "browser-pkce-admin-logout-no-capture",
      "builder-canary-zero-connections",
      "canonical-proxy-values-empty",
      "clean-engine-initial-state",
      "disposable-engine-destroyed",
      "docker-client-proxy-active",
      "exact-local-embedded-builder",
      "exact-project-cleanup",
      "registry-auth-negative-positive",
      "source-closure-unchanged",
    ],
    run_id: fixtureId,
    schema_version: 1,
    selection: {
      app_host: "app.synveda.test",
      auth_host: "auth.synveda.test",
      ipv4_pool: "10.239.17.0/24",
      oidc: "bundled",
      port: 8080,
      postgres: "bundled",
      profiles: ["browser-acceptance", "demo"],
      project: `synveda-development-acceptance-${fixtureId.slice(0, 24)}`,
      project_suffix: `acceptance-${fixtureId.slice(0, 24)}`,
      runtime: "development",
      scheme: "http",
    },
    source: {
      build_context_manifest_sha256: "6".repeat(64),
      commit_sha: "7".repeat(40),
      deployment_contract_sha256: "8".repeat(64),
      deployment_input_manifest_sha256: "9".repeat(64),
      tracked_index_manifest_sha256: "a".repeat(64),
      tree_sha: "b".repeat(40),
      worktree_clean: true,
    },
  };
}

test("the full success chain alone makes a content-free environment manifest eligible", () => {
  const receipts = successBeforeFinalize();
  const state = validateReceiptChain(receipts, fixtureId);
  assert.equal(state.manifest_eligible, true);
  assert.equal(state.terminal, false);
  assert.equal(receipts.length, 15);
  assert.deepEqual(receipts.map((receipt) => receipt.phase), receiptSuccessPath.slice(0, -1));
  assert.deepEqual(receipts.map(receiptFileName), [
    "00-plan.json",
    "01-provider-create-intent.json",
    "02-provider-create-passed.json",
    "03-registry-intent.json",
    "04-registry-passed.json",
    "05-proxy-intent.json",
    "06-proxy-passed.json",
    "07-builder-canary-intent.json",
    "08-builder-canary-passed.json",
    "09-compose-browser-intent.json",
    "10-compose-browser-passed.json",
    "11-project-cleanup-intent.json",
    "12-project-cleanup-passed.json",
    "13-provider-cleanup-intent.json",
    "14-provider-cleanup-passed.json",
  ]);

  const selectedCandidate = candidate();
  const candidateBytes = canonicalBytes(selectedCandidate);
  const manifest = buildEnvironmentManifest(selectedCandidate, candidateBytes, receipts);
  const raw = canonicalBytes(manifest);
  assert.equal(manifest.receipt.count, 15);
  assert.equal(Object.keys(manifest.assertions).length, 10);
  assert.equal(Object.values(manifest.cleanup).every(Boolean), true);
  assert.doesNotMatch(
    raw.toString("utf8"),
    /password|authorization|bearer|cookie|private[_ -]?key|\$2[aby]\$|Users\/|home\//i,
  );

  const finalization = createFinalization(selectedCandidate, candidateBytes, receipts);
  assert.deepEqual(finalization.manifest, manifest);
  assert.deepEqual(finalization.manifestBytes, raw);
  receipts.push(finalization.receipt);
  assert.equal(validateReceiptChain(receipts, fixtureId).terminal, true);
});

test("unknown, skipped, reordered and mutated receipts are refused", () => {
  const mutations = [
    (receipts) => {
      receipts[1].phase = "registry-intent";
    },
    (receipts) => {
      receipts[1].sequence = 2;
    },
    (receipts) => {
      receipts[1].previous_sha256 = "f".repeat(64);
    },
    (receipts) => {
      receipts[1].outcome = "passed";
    },
    (receipts) => {
      receipts[1].result.unreviewed = true;
    },
    (receipts) => {
      receipts[0].result.project = "foreign";
    },
  ];
  for (const mutate of mutations) {
    const receipts = [plan()];
    receipts.push(createNextReceipt(receipts, fixtureId, "provider-create-intent", result("provider-create-intent")));
    mutate(receipts);
    assert.throws(() => validateReceiptChain(receipts, fixtureId), ReceiptFailure);
  }
  assert.throws(
    () => createNextReceipt([plan()], fixtureId, "registry-intent", result("registry-intent")),
    /next receipt phase was refused/,
  );
});

test("provider success binds its explicit evidence class and intent contract", () => {
  const receipts = [plan()];
  receipts.push(
    createNextReceipt(
      receipts,
      fixtureId,
      "provider-create-intent",
      result("provider-create-intent"),
    ),
  );

  const mismatchedContract = result("provider-create-passed");
  mismatchedContract.provider_contract_sha256 = "f".repeat(64);
  assert.throws(
    () =>
      createNextReceipt(
        receipts,
        fixtureId,
        "provider-create-passed",
        mismatchedContract,
      ),
    /provider result contract binding was refused/,
  );

  const relabelledFixture = result("provider-create-passed");
  relabelledFixture.evidence_class = "controlled-fake";
  assert.throws(
    () =>
      createNextReceipt(
        receipts,
        fixtureId,
        "provider-create-passed",
        relabelledFixture,
      ),
    /provider result(?: was| fields were) refused/,
  );

  const controlledReceipts = [plan()];
  const controlledContract = "c".repeat(64);
  controlledReceipts.push(
    createNextReceipt(controlledReceipts, fixtureId, "provider-create-intent", {
      ...result("provider-create-intent"),
      provider_contract_sha256: controlledContract,
    }),
  );
  controlledReceipts.push(
    createNextReceipt(controlledReceipts, fixtureId, "provider-create-passed", {
      evidence_class: "controlled-fake",
      platform: "deterministic-posix",
      provider_contract_sha256: controlledContract,
      provider_evidence_sha256: "d".repeat(64),
      provider_name: "controlled-fake",
      runtime_name: "none",
    }),
  );
  for (const phase of receiptSuccessPath.slice(3, -1)) {
    controlledReceipts.push(
      createNextReceipt(controlledReceipts, fixtureId, phase, result(phase)),
    );
  }
  assert.equal(validateReceiptChain(controlledReceipts, fixtureId).manifest_eligible, false);
  assert.throws(
    () => buildEnvironmentManifest(candidate(), canonicalBytes(candidate()), controlledReceipts),
    /environment manifest is not eligible/,
  );
});

test("a failed phase can only enter exact cleanup and can never publish a manifest", () => {
  const receipts = [plan()];
  receipts.push(createNextReceipt(receipts, fixtureId, "provider-create-intent", result("provider-create-intent")));
  receipts.push(
    createNextReceipt(receipts, fixtureId, "provider-create-failed", {
      cleanup_required: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "child-timeout",
    }),
  );
  assert.throws(
    () => createNextReceipt(receipts, fixtureId, "registry-intent", result("registry-intent")),
    /next receipt phase was refused/,
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider"],
      scope: "exact-receipt-owned-only",
    }),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-failed", {
      cleanup_incomplete: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "cleanup-incomplete",
    }),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider"],
      scope: "exact-receipt-owned-only",
    }),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-passed", {
      foreign_collision_preserved: true,
      manifest_published: false,
      receipt_owned_resources_absent: true,
    }),
  );
  const state = validateReceiptChain(receipts, fixtureId);
  assert.equal(state.manifest_eligible, false);
  assert.equal(state.terminal, true);
  assert.throws(
    () => buildEnvironmentManifest(candidate(), canonicalBytes(candidate()), receipts),
    /environment manifest is not eligible/,
  );
  assert.throws(
    () => createFinalization(candidate(), canonicalBytes(candidate()), receipts),
    /environment manifest is not eligible/,
  );
});

test("receipt hashes cover canonical bytes and never claim signatures", () => {
  const receipts = successBeforeFinalize();
  for (let index = 1; index < receipts.length; index += 1) {
    assert.equal(receipts[index].previous_sha256, hash(canonicalBytes(receipts[index - 1])));
  }
  const raw = canonicalBytes(receipts.at(-1)).toString("utf8");
  assert.equal(raw.endsWith("\n"), true);
  assert.doesNotMatch(raw, /signature|provenance|credential|secret/i);
});

test("manifest construction requires the exact planned closed candidate", () => {
  const selected = candidate();
  const receipts = successBeforeFinalize();
  const changed = structuredClone(selected);
  changed.selection.port = 8081;
  assert.throws(
    () => buildEnvironmentManifest(changed, canonicalBytes(changed), receipts),
    /candidate selection was refused|candidate digest was refused/,
  );

  const injected = { ...selected, secret_value: "sentinel-do-not-persist" };
  const rebound = successBeforeFinalize(plan(sha256(canonicalBytes(injected))));
  assert.throws(
    () => buildEnvironmentManifest(injected, canonicalBytes(injected), rebound),
    /candidate fields were refused/,
  );

  const nested = structuredClone(selected);
  nested.source.unreviewed = "sentinel-do-not-persist";
  const nestedReceipts = successBeforeFinalize(plan(sha256(canonicalBytes(nested))));
  assert.throws(
    () => buildEnvironmentManifest(nested, canonicalBytes(nested), nestedReceipts),
    /candidate source fields were refused/,
  );

  const mismatched = successBeforeFinalize(plan("f".repeat(64)));
  assert.throws(
    () => buildEnvironmentManifest(selected, canonicalBytes(selected), mismatched),
    /candidate digest was refused/,
  );
});

test("a registry collision remains foreign across failed cleanup retries", () => {
  const receipts = [plan()];
  for (const phase of [
    "provider-create-intent",
    "provider-create-passed",
    "registry-intent",
  ]) {
    receipts.push(createNextReceipt(receipts, fixtureId, phase, result(phase)));
  }
  receipts.push(
    createNextReceipt(receipts, fixtureId, "registry-failed", {
      cleanup_required: true,
      collision_resource: "registry",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
  );
  const cleanupIntent = {
    authorized_resources: ["provider", "runtime-secrets"],
    scope: "exact-receipt-owned-only",
  };
  assert.throws(
    () => createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider", "registry", "runtime-secrets"],
      scope: "exact-receipt-owned-only",
    }),
    /failure cleanup intent result was refused/,
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", cleanupIntent),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-failed", {
      cleanup_incomplete: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "cleanup-incomplete",
    }),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", cleanupIntent),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-passed", {
      foreign_collision_preserved: true,
      manifest_published: false,
      receipt_owned_resources_absent: true,
    }),
  );
  assert.equal(validateReceiptChain(receipts, fixtureId).terminal, true);
});

test("collision authority is phase-bound and cleanup-time collisions stay foreign", () => {
  const registryFailure = [plan()];
  for (const phase of [
    "provider-create-intent",
    "provider-create-passed",
    "registry-intent",
  ]) {
    registryFailure.push(createNextReceipt(registryFailure, fixtureId, phase, result(phase)));
  }
  assert.throws(
    () => createNextReceipt(registryFailure, fixtureId, "registry-failed", {
      cleanup_required: true,
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
    /failure result collision was refused/,
  );

  const providerFailure = [plan()];
  providerFailure.push(
    createNextReceipt(
      providerFailure,
      fixtureId,
      "provider-create-intent",
      result("provider-create-intent"),
    ),
  );
  assert.throws(
    () => createNextReceipt(providerFailure, fixtureId, "provider-create-failed", {
      cleanup_required: true,
      collision_resource: "registry",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
    /failure result collision was refused/,
  );
  assert.throws(
    () => createNextReceipt(providerFailure, fixtureId, "execution-failed", {
      cleanup_required: true,
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
    /failure result collision was refused/,
  );
  providerFailure.push(
    createNextReceipt(providerFailure, fixtureId, "provider-create-failed", {
      cleanup_required: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "child-failed",
    }),
  );
  providerFailure.push(
    createNextReceipt(providerFailure, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider"],
      scope: "exact-receipt-owned-only",
    }),
  );
  providerFailure.push(
    createNextReceipt(providerFailure, fixtureId, "failure-cleanup-failed", {
      cleanup_incomplete: true,
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
  );
  assert.throws(
    () => createNextReceipt(providerFailure, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider"],
      scope: "exact-receipt-owned-only",
    }),
    /failure cleanup intent result was refused/,
  );
  providerFailure.push(
    createNextReceipt(providerFailure, fixtureId, "failure-cleanup-intent", {
      authorized_resources: [],
      scope: "exact-receipt-owned-only",
    }),
  );
  providerFailure.push(
    createNextReceipt(providerFailure, fixtureId, "failure-cleanup-passed", {
      foreign_collision_preserved: true,
      manifest_published: false,
      receipt_owned_resources_absent: true,
    }),
  );
  assert.equal(validateReceiptChain(providerFailure, fixtureId).terminal, true);
});

test("passed cleanup receipts permanently retire destructive authority", () => {
  const receipts = successBeforeFinalize();
  receipts.push(
    createNextReceipt(receipts, fixtureId, "execution-failed", {
      cleanup_required: true,
      collision_resource: "none",
      resource_disposition: "receipt-owned-or-absent",
      safe_code: "evidence-refused",
    }),
  );
  assert.throws(
    () => createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: ["provider", "registry"],
      scope: "exact-receipt-owned-only",
    }),
    /failure cleanup intent result was refused/,
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: [],
      scope: "exact-receipt-owned-only",
    }),
  );
  receipts.push(
    createNextReceipt(receipts, fixtureId, "failure-cleanup-passed", {
      foreign_collision_preserved: true,
      manifest_published: false,
      receipt_owned_resources_absent: true,
    }),
  );
  assert.equal(validateReceiptChain(receipts, fixtureId).terminal, true);
});

test("a preflight collision publishes no cleanup authority and is terminal", () => {
  const receipts = [plan()];
  receipts.push(
    createNextReceipt(receipts, fixtureId, "preflight-refused", {
      cleanup_required: false,
      collision_resource: "provider",
      resource_disposition: "foreign-preserved",
      safe_code: "resource-collision",
    }),
  );
  const state = validateReceiptChain(receipts, fixtureId);
  assert.equal(state.terminal, true);
  assert.equal(state.manifest_eligible, false);
  assert.throws(
    () => createNextReceipt(receipts, fixtureId, "failure-cleanup-intent", {
      authorized_resources: [],
      scope: "exact-receipt-owned-only",
    }),
    /next receipt phase was refused/,
  );
});

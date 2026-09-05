import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  LiveProviderPlanFailure,
  buildColimaLiveProviderOperationPlan,
  liveProviderPlanBytes,
  liveProviderPlanDigest,
  validateColimaLiveProviderOperationPlan,
} from "../deploy/compose/scripts/clean-engine-live-provider-plan.mjs";
import { cleanEngineLiveProviderOperationPlan } from "./fixtures/clean-engine-live-provider-plan-fixture.mjs";

const FIXTURE_ID = "a".repeat(32);
const CANDIDATE_SHA256 = "b".repeat(64);
const SOURCE_HEAD_SHA256 = "c".repeat(64);

function plan() {
  return cleanEngineLiveProviderOperationPlan({
    candidateSha256: CANDIDATE_SHA256,
    fixtureId: FIXTURE_ID,
    sourceHeadSha256: SOURCE_HEAD_SHA256,
  });
}

function expectRefusal(operation, exitStatus) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderPlanFailure);
    if (exitStatus !== undefined) assert.equal(error.exitStatus, exitStatus);
    return true;
  });
}

test("the production-shaped fixture plan is exact, content addressed and plan only", () => {
  const value = plan();
  assert.equal(validateColimaLiveProviderOperationPlan(value), value);
  assert.equal(value.action, "provider-create");
  assert.equal(value.state_integration, "mutation-journal-v3-plan-only");
  assert.deepEqual(value.capabilities, {
    execution_authorized: false,
    finalization_eligible: false,
    lifecycle_exposure_authorized: false,
    recovery_authorized: false,
    state_planning_authorized: true,
  });
  assert.equal(value.source_candidate_sha256, CANDIDATE_SHA256);
  assert.equal(value.source_head_sha256, SOURCE_HEAD_SHA256);
  assert.equal(value.source_sequence, 0);
  assert.match(liveProviderPlanDigest(liveProviderPlanBytes(value)), /^[0-9a-f]{64}$/u);
});

test("the operation plan contains no private preparation inputs", () => {
  const serialized = liveProviderPlanBytes(plan()).toString("utf8");
  for (const forbidden of [
    "/Users/private",
    "/home/private",
    "HOME",
    "binding_key",
    "command",
    "environment",
    "password",
    "secret",
    "token",
  ]) {
    assert.equal(serialized.includes(forbidden), false);
  }
});

test("every registry, preparation and source binding fails closed on drift", () => {
  const mutations = [
    (value) => {
      value.action = "provider-cleanup";
    },
    (value) => {
      value.adapter_key_sha256 = "f".repeat(64);
    },
    (value) => {
      value.capabilities.execution_authorized = true;
    },
    (value) => {
      value.capabilities.state_planning_authorized = false;
    },
    (value) => {
      value.evidence_schema = "other";
    },
    (value) => {
      value.operation_contract_sha256 = "f".repeat(64);
    },
    (value) => {
      value.operation_kind = "other";
    },
    (value) => {
      value.preparation_observation_schema = "other";
    },
    (value) => {
      value.preparation_observation_sha256 = "0".repeat(64);
    },
    (value) => {
      value.provider_class = "controlled-background-fake";
    },
    (value) => {
      value.provider_profile = "default";
    },
    (value) => {
      value.provider_resource = "other";
    },
    (value) => {
      value.registry_sha256 = "f".repeat(64);
    },
    (value) => {
      value.requirements_sha256 = "f".repeat(64);
    },
    (value) => {
      value.source_candidate_sha256 = "0".repeat(64);
    },
    (value) => {
      value.source_head_sha256 = "0".repeat(64);
    },
    (value) => {
      value.source_sequence = 1;
    },
    (value) => {
      value.state_integration = "mutation-journal-v2";
    },
  ];
  for (const mutate of mutations) {
    const changed = plan();
    mutate(changed);
    expectRefusal(() => validateColimaLiveProviderOperationPlan(changed));
  }
});

test("missing, extra and malformed plan shapes are refused", () => {
  const missing = plan();
  delete missing.registry_sha256;
  expectRefusal(() => validateColimaLiveProviderOperationPlan(missing));

  const extra = plan();
  extra.unreviewed = true;
  expectRefusal(() => validateColimaLiveProviderOperationPlan(extra));
  expectRefusal(() => validateColimaLiveProviderOperationPlan(null));
  expectRefusal(() => validateColimaLiveProviderOperationPlan([]));
});

test("the production builder refuses anything short of production observation evidence", () => {
  const sensitive = "/Users/private/secret-live-provider-root";
  assert.throws(
    () =>
      buildColimaLiveProviderOperationPlan({
        observation: { fixture_id: FIXTURE_ID, path: sensitive },
        observationInput: { path: sensitive },
        stateBinding: {
          candidate_sha256: CANDIDATE_SHA256,
          fixture_id: FIXTURE_ID,
          source_head_sha256: SOURCE_HEAD_SHA256,
          source_sequence: 0,
        },
        tuple: {
          action: "provider-create",
          operation_contract_sha256: "f".repeat(64),
          operation_kind: "colima-vz-docker-live-create-v1",
          provider_class: "colima-vz-docker-live",
        },
      }),
    (error) => {
      assert.ok(error instanceof LiveProviderPlanFailure);
      assert.equal(error.message.includes(sensitive), false);
      return true;
    },
  );
});

test("the plan boundary owns no process, network or test seam", () => {
  const source = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-live-provider-plan.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.doesNotMatch(source, /node:(?:child_process|fs|http|https|net|tls)/u);
  assert.doesNotMatch(source, /\b(?:exec|execFile|fork|spawn)(?:Sync)?\s*\(/u);
  assert.doesNotMatch(source, /ForTest|scripts\/fixtures/u);
  assert.match(source, /revalidateColimaLiveObservation/u);

  const observationSource = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-colima-live-contract.mjs",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(observationSource, /node:fs/u);
  assert.doesNotMatch(observationSource, /node:child_process/u);

  const stateSource = readFileSync(
    new URL("../deploy/compose/scripts/clean-engine-state.mjs", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(stateSource, /scripts\/fixtures/u);

  const lifecycle = readFileSync(
    new URL(
      "../deploy/compose/scripts/clean-engine-acceptance.sh",
      import.meta.url,
    ),
    "utf8",
  );
  assert.match(lifecycle, /plan\|status\|verify/u);
  assert.doesNotMatch(
    lifecycle,
    /provider-plan|recordLiveProvider|live-provider-operation-plan/u,
  );
});

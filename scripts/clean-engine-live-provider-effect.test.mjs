#!/usr/bin/env node
import assert from "node:assert/strict";
import { createPublicKey, verify } from "node:crypto";
import fs, { readFileSync, readdirSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { dirname, join, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import * as effect from "../deploy/compose/scripts/clean-engine-live-provider-effect.mjs";
import {
  minimumGenerationForestEntries,
} from "../deploy/compose/scripts/clean-engine-live-provider-effect-capacity.mjs";
import {
  COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
} from "../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import * as effectFixture from "./fixtures/clean-engine-live-provider-effect-fixture.mjs";
import {
  cleanEngineLiveProviderProcessStartFreshAdmissionFixture,
} from "./fixtures/clean-engine-live-provider-process-start-fixture.mjs";

const {
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS,
  COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
  COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS,
  COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS,
  COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
  COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS,
  COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT,
  COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_KIND,
  COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
  COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS,
  COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
  LiveProviderEffectFailure,
  buildColimaLiveProviderEffectEvent,
  buildColimaLiveProviderEffectPublicationPlan,
  buildColimaLiveProviderEffectWitness,
  colimaLiveProviderEffectProofBytes,
  deriveColimaLiveFixtureProviderEffectCleanupActions,
  liveProviderEffectBytes,
  liveProviderEffectDigest,
  validateColimaLiveProviderEffectEvent,
  validateColimaLiveProviderEffectEventHistory,
  validateColimaLiveProviderEffectPublicationPlan,
  validateColimaLiveProviderEffectWitness,
} = effect;
const { cleanEngineLiveProviderEffectFixture } = effectFixture;

const ZERO_SHA256 = "0".repeat(64);
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "..");
const MODULE_PATH = resolve(
  REPO_ROOT,
  "deploy/compose/scripts/clean-engine-live-provider-effect.mjs",
);
const CAPACITY_PATH = resolve(
  REPO_ROOT,
  "deploy/compose/scripts/clean-engine-live-provider-effect-capacity.mjs",
);
const FIXTURE_PATH = resolve(
  REPO_ROOT,
  "scripts/fixtures/clean-engine-live-provider-effect-fixture.mjs",
);
const TEST_PATH = fileURLToPath(import.meta.url);
const fixtureCache = new Map();
let productionCache;

function clone(value) {
  return structuredClone(value);
}

function digest(value) {
  return liveProviderEffectDigest(liveProviderEffectBytes(value));
}

function inventoryOwnershipBinding(fixture, binding, identity, ownership) {
  return digest({
    baseline_set_binding_sha256: binding.baseline_set_binding_sha256,
    namespace_identity_hmac_sha256:
      binding.namespace_identity_hmac_sha256,
    ownership,
    relative_identity_hmac_sha256: identity,
    witness_sha256:
      ownership === "generation-owned"
        ? digest(fixture.witness)
        : ZERO_SHA256,
  });
}

function prepared(outcome) {
  if (!fixtureCache.has(outcome)) {
    fixtureCache.set(
      outcome,
      outcome === "retired"
        ? cleanEngineLiveProviderEffectFixture()
        : cleanEngineLiveProviderEffectFixture({ outcome }),
    );
  }
  return fixtureCache.get(outcome);
}

function validationOptions(fixture) {
  return {
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: fixture.publisherAuthorityChain,
    source: fixture.source,
    witness: fixture.witness,
  };
}

function eventValidationOptions(fixture, history) {
  return { history, ...validationOptions(fixture) };
}

function expectRefusal(operation, pattern = /provider effect/u) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof LiveProviderEffectFailure);
    assert.match(error.message, pattern);
    assert.ok(error.exitStatus >= 69);
    return true;
  });
}

function assertRecursivelyFrozen(value, visited = new WeakSet()) {
  if (value === null || typeof value !== "object" || visited.has(value)) return;
  visited.add(value);
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) {
    assertRecursivelyFrozen(child, visited);
  }
}

function changedValue(value) {
  if (typeof value === "boolean") return !value;
  if (typeof value === "number") return value + 1;
  if (typeof value === "string") {
    if (/^[0-9a-f]{64}$/u.test(value)) {
      return `${value[0] === "a" ? "b" : "a"}${value.slice(1)}`;
    }
    return `${value}-changed`;
  }
  if (Array.isArray(value)) return [...value, "unexpected"];
  if (value !== null && typeof value === "object") {
    return { ...value, unexpected: true };
  }
  return "unexpected";
}

function nestedArray(depth) {
  let value = "leaf";
  for (let index = 0; index < depth; index += 1) value = [value];
  return value;
}

function binaryTree(depth) {
  if (depth === 0) return "leaf";
  return [binaryTree(depth - 1), binaryTree(depth - 1)];
}

function buildEvent(fixture, history, original, evidence = original.evidence) {
  return buildColimaLiveProviderEffectEvent({
    eventKind: original.event_kind,
    evidence,
    history,
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: fixture.publisherAuthorityChain,
    publisherAuthoritySha256: original.publisher_authority_sha256,
    source: fixture.source,
    witness: fixture.witness,
  });
}

function eventIndex(fixture, kind, predicate = () => true) {
  const index = fixture.events.findIndex(
    (event) => event.event_kind === kind && predicate(event.evidence),
  );
  assert.notEqual(index, -1, `missing ${kind} exemplar`);
  return index;
}

function validateAt(fixture, index, event = fixture.events[index]) {
  return validateColimaLiveProviderEffectEvent(
    event,
    eventValidationOptions(fixture, fixture.events.slice(0, index)),
  );
}

function setAtPath(value, path, transform = changedValue) {
  const copy = clone(value);
  let parent = copy;
  for (const component of path.slice(0, -1)) parent = parent[component];
  const leaf = path.at(-1);
  parent[leaf] = transform(parent[leaf]);
  return copy;
}

function inventoryPageContentDigest(page) {
  return digest({
    entries: page.evidence.entries,
    entry_start: page.evidence.entry_start,
    namespace_role: page.evidence.namespace_role,
    page_sequence: page.evidence.page_sequence,
  });
}

function inventorySampleDigest(pages) {
  return digest({
    page_count: pages.length,
    page_sha256: pages.map(inventoryPageContentDigest),
  });
}

function productionPrepared() {
  if (productionCache !== undefined) return productionCache;
  const blueprint = prepared("pre-attempt-retired");
  const upstream =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(false);
  const namespaceBindings =
    upstream.admission.root_observation.root_observations.map(
      (observation, index) => {
        const prior = blueprint.publicationPlan.namespace_bindings[index];
        return {
          device: prior.device,
          inode: prior.inode,
          mode: prior.mode,
          role: observation.role,
          uid: prior.uid,
        };
      },
    );
  const publicationPlan = buildColimaLiveProviderEffectPublicationPlan({
    admission: upstream.admission,
    invocationBinding: blueprint.publicationPlan.invocation_binding,
    namespaceBindings,
    plannedEndpointContracts:
      blueprint.publicationPlan.planned_endpoint_contracts,
    plannedQuiescenceFenceSha256:
      blueprint.publicationPlan.planned_quiescence_fence_sha256,
    plannedRoleContracts: blueprint.publicationPlan.planned_role_contracts,
    plannedStartAttemptSha256:
      blueprint.publicationPlan.planned_start_attempt_sha256,
    providerRootIdentity: blueprint.publicationPlan.provider_root_identity,
    source: upstream.source,
    stateRunIdentity: blueprint.publicationPlan.state_run_identity,
  });
  const witness = buildColimaLiveProviderEffectWitness({
    markerWitnessIdentitySha256:
      blueprint.witness.marker_witness_identity_sha256,
    publicationPlan,
    slotSequence: blueprint.witness.slot_sequence,
    slotSha256: blueprint.witness.slot_sha256,
    source: upstream.source,
  });
  productionCache = Object.freeze({
    publicationPlan,
    publisherAuthorityChain: Object.freeze([
      Object.freeze({
        authority_sha256: witness.slot_sha256,
        first_event_sequence: 0,
        kind: "owner-slot",
        prior_authority_sha256: ZERO_SHA256,
        recovery_claim_sha256: ZERO_SHA256,
      }),
    ]),
    source: upstream.source,
    witness,
  });
  return productionCache;
}

function rebuildInventorySet(fixture, transform) {
  const firstPageIndex = eventIndex(fixture, "recursive-inventory-page");
  const originalSet = fixture.events.find(
    (event) => event.event_kind === "recursive-inventory-set",
  );
  const originalPages = fixture.events.filter(
    (event) => event.event_kind === "recursive-inventory-page",
  );
  const history = fixture.events.slice(0, firstPageIndex);
  const pagesByGroup = new Map();
  for (const sampleSequence of [0, 1]) {
    for (const namespaceRole of COLIMA_LIVE_MUTATION_SURFACE_ROLES) {
      const entries = originalPages
        .filter(
          (page) =>
            page.evidence.sample_sequence === sampleSequence &&
            page.evidence.namespace_role === namespaceRole,
        )
        .flatMap((page) => clone(page.evidence.entries));
      const transformed = transform(entries, {
        namespaceRole,
        sampleSequence,
      });
      for (const entry of transformed) {
        if (entry.type === "directory") {
          entry.directory_entry_count = transformed.filter(
            (candidate) =>
              candidate.parent_identity_hmac_sha256 ===
              entry.relative_identity_hmac_sha256,
          ).length;
        }
      }
      transformed.forEach((entry, index) => {
        entry.entry_sequence = index;
      });
      const rebuiltPages = [];
      const pageSize =
        COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries_per_page;
      for (let entryStart = 0; entryStart < transformed.length;
        entryStart += pageSize) {
        const original = originalPages[0];
        const evidence = {
          entries: transformed.slice(entryStart, entryStart + pageSize),
          entry_start: entryStart,
          namespace_role: namespaceRole,
          page_sequence: rebuiltPages.length,
          parent_page_sha256:
            rebuiltPages.length === 0
              ? ZERO_SHA256
              : digest(rebuiltPages.at(-1)),
          sample_sequence: sampleSequence,
        };
        const page = buildEvent(fixture, history, original, evidence);
        history.push(page);
        rebuiltPages.push(page);
      }
      pagesByGroup.set(`${sampleSequence}:${namespaceRole}`, rebuiltPages);
    }
  }
  const samples = COLIMA_LIVE_MUTATION_SURFACE_ROLES.map((namespaceRole) => {
    const first = pagesByGroup.get(`0:${namespaceRole}`);
    const second = pagesByGroup.get(`1:${namespaceRole}`);
    return {
      entry_count: first.reduce(
        (total, page) => total + page.evidence.entries.length,
        0,
      ),
      first_sample_sha256: inventorySampleDigest(first),
      namespace_role: namespaceRole,
      page_count: first.length,
      second_sample_sha256: inventorySampleDigest(second),
    };
  });
  return buildEvent(fixture, history, originalSet, {
    inventory_sha256: digest(samples),
    quiescence_fence_sha256:
      originalSet.evidence.quiescence_fence_sha256,
    sample_count: 2,
    samples,
  });
}

function rebuildFirstInventorySampleBoundary(fixture, transform) {
  const firstPageIndex = eventIndex(fixture, "recursive-inventory-page");
  const originalPages = fixture.events.filter(
    (event) => event.event_kind === "recursive-inventory-page",
  );
  const history = fixture.events.slice(0, firstPageIndex);
  for (const namespaceRole of COLIMA_LIVE_MUTATION_SURFACE_ROLES) {
    const entries = originalPages
      .filter(
        (page) =>
          page.evidence.sample_sequence === 0 &&
          page.evidence.namespace_role === namespaceRole,
      )
      .flatMap((page) => clone(page.evidence.entries));
    const transformed = transform(entries, { namespaceRole }) ?? entries;
    for (const entry of transformed) {
      if (entry.type === "directory") {
        entry.directory_entry_count = transformed.filter(
          (candidate) =>
            candidate.parent_identity_hmac_sha256 ===
            entry.relative_identity_hmac_sha256,
        ).length;
      }
    }
    transformed.forEach((entry, index) => {
      entry.entry_sequence = index;
    });
    const pages = [];
    const pageSize =
      COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries_per_page;
    for (let entryStart = 0; entryStart < transformed.length;
      entryStart += pageSize) {
      const page = buildEvent(fixture, history, originalPages[0], {
        entries: transformed.slice(entryStart, entryStart + pageSize),
        entry_start: entryStart,
        namespace_role: namespaceRole,
        page_sequence: pages.length,
        parent_page_sha256:
          pages.length === 0 ? ZERO_SHA256 : digest(pages.at(-1)),
        sample_sequence: 0,
      });
      history.push(page);
      pages.push(page);
    }
  }
  const firstSecondSamplePage = originalPages.find(
    (page) => page.evidence.sample_sequence === 1,
  );
  return buildEvent(
    fixture,
    history,
    firstSecondSamplePage,
    firstSecondSamplePage.evidence,
  );
}

function cleanupActions(fixture) {
  return fixture.events
    .filter((event) => event.event_kind === "cleanup-plan-page")
    .flatMap((event) => event.evidence.actions);
}

function cleanupObservationBinding(fixture, progressEvent, outcome) {
  const plan = fixture.events.find(
    (event) => event.event_kind === "cleanup-plan",
  );
  const action = cleanupActions(fixture)[outcome.action_sequence];
  return digest({
    action_sha256: outcome.action_sha256,
    authority_sha256: progressEvent.evidence.authority_sha256,
    cleanup_plan_sha256: progressEvent.evidence.cleanup_plan_sha256,
    expected_postcondition_sha256: action.expected_postcondition_sha256,
    expected_precondition_sha256: action.expected_precondition_sha256,
    marker_link_count: 2,
    marker_witness_observation_sha256:
      outcome.marker_witness_observation_sha256,
    observation_challenge_sha256: outcome.observation_challenge_sha256,
    operation_plan_sha256: digest(fixture.publicationPlan),
    outcome_observation_sha256: outcome.outcome_observation_sha256,
    parent_fsync_observation_sha256:
      outcome.parent_fsync_observation_sha256,
    parent_progress_sha256: progressEvent.evidence.parent_progress_sha256,
    precondition_observation_sha256:
      outcome.precondition_observation_sha256,
    recursive_prefix_observation_sha256:
      outcome.recursive_prefix_observation_sha256,
    source_reproof_observation_sha256:
      outcome.source_reproof_observation_sha256,
    source_sha256: digest(fixture.source),
    witness_sha256: digest(fixture.witness),
  });
}

function markerRetirementBinding(fixture, completionEvidence) {
  return digest({
    cleanup_settlement_sha256:
      completionEvidence.cleanup_settlement_sha256,
    marker_retirement_observation_sha256:
      completionEvidence.marker_retirement_observation_sha256,
    provider_root_fsync_observation_sha256:
      completionEvidence.provider_root_fsync_observation_sha256,
    publisher_authority_sha256: fixture.publisherAuthoritySha256,
    receipt_sha256: completionEvidence.receipt_sha256,
    witness_identity_sha256: completionEvidence.witness_identity_sha256,
    witness_link_count: completionEvidence.witness_link_count,
  });
}

function firstPartyFiles(root) {
  const files = [];
  const pending = [root];
  const skipped = new Set([
    ".git",
    ".next",
    "build",
    "coverage",
    "dist",
    "node_modules",
    "target",
  ]);
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      if (skipped.has(entry.name)) continue;
      const path = join(current, entry.name);
      if (entry.isDirectory()) pending.push(path);
      else if (entry.isFile()) files.push(path);
    }
  }
  return files;
}

function rebuildCleanupPlan(fixture, transform) {
  const firstPageIndex = eventIndex(fixture, "cleanup-plan-page");
  const originalPage = fixture.events[firstPageIndex];
  const originalPlan = fixture.events.find(
    (event) => event.event_kind === "cleanup-plan",
  );
  const history = fixture.events.slice(0, firstPageIndex);
  const createSettlement = history.find(
    (event) => event.event_kind === "create-settlement",
  );
  const actions = clone(
    deriveColimaLiveFixtureProviderEffectCleanupActions({
      history,
      ...validationOptions(fixture),
    }),
  );
  transform(actions);
  for (const [index, action] of actions.entries()) {
    action.action_sequence = index;
    action.prerequisite_sha256 =
      index === 0 ? digest(createSettlement) : digest(actions[index - 1]);
    const common = {
      action_kind: action.action_kind,
      action_sequence: action.action_sequence,
      depth: action.depth,
      namespace_role: action.namespace_role,
      parent_fsync_required: action.parent_fsync_required,
      prerequisite_sha256: action.prerequisite_sha256,
      role: action.role,
      target_class: action.target_class,
      target_identity_sha256: action.target_identity_sha256,
    };
    action.expected_postcondition_sha256 = digest({
      ...common,
      expected_state: "retired-or-baseline-proved",
    });
    action.expected_precondition_sha256 = digest({
      ...common,
      expected_state: "current-exact-target",
    });
  }
  const pages = [];
  const pageSize =
    COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.canonical_container_entries;
  for (let actionStart = 0; actionStart < actions.length;
    actionStart += pageSize) {
    const page = buildEvent(fixture, history, originalPage, {
      action_start: actionStart,
      actions: actions.slice(actionStart, actionStart + pageSize),
      create_settlement_sha256: digest(createSettlement),
      page_sequence: pages.length,
      parent_page_sha256:
        pages.length === 0 ? ZERO_SHA256 : digest(pages.at(-1)),
    });
    history.push(page);
    pages.push(page);
  }
  return buildEvent(fixture, history, originalPlan, {
    action_count: actions.length,
    create_settlement_sha256: digest(createSettlement),
    marker_link_count: 2,
    page_count: pages.length,
    page_set_sha256: digest({
      page_count: pages.length,
      terminal_page_sha256: digest(pages.at(-1)),
    }),
    reverse_causal_order: true,
  });
}

test("the public effect boundary is exact, class-separated and recursively frozen", () => {
  assert.deepEqual(Object.keys(effect), [
    "COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT",
    "COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256",
    "COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND",
    "COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS",
    "COLIMA_LIVE_PROVIDER_EFFECT_ACTION",
    "COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS",
    "COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS",
    "COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS",
    "COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS",
    "COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT",
    "COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256",
    "COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_KIND",
    "COLIMA_LIVE_PROVIDER_EFFECT_ROLES",
    "COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS",
    "COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION",
    "LiveProviderEffectFailure",
    "buildColimaLiveProviderEffectEvent",
    "buildColimaLiveProviderEffectPublicationPlan",
    "buildColimaLiveProviderEffectWitness",
    "colimaLiveProviderEffectProofBytes",
    "deriveColimaLiveFixtureProviderEffectCleanupActions",
    "liveProviderEffectBytes",
    "liveProviderEffectDigest",
    "validateColimaLiveProviderEffectEvent",
    "validateColimaLiveProviderEffectEventHistory",
    "validateColimaLiveProviderEffectPublicationPlan",
    "validateColimaLiveProviderEffectWitness",
  ]);
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_ACTION, "provider-effect");
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
    "mutation-journal-v7-sibling-effect-only",
  );
  assert.notEqual(
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_KIND,
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
  );
  assert.notEqual(
    COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS.contract,
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS.contract,
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
    "e57ab31606d0cf6e33a0fd45cc86335a6ca1288d9beb28839aeb45225f24df63",
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
    "2926b334f9f63a665fc3648e32e3438627bff04a9dd90d1fed9b71e3d23aeae8",
  );
  assert.equal(
    digest(COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT),
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  );
  assert.equal(
    digest(COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT),
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256,
  );
  assert.deepEqual(COLIMA_LIVE_PROVIDER_EFFECT_ROLES, [
    "outer",
    "hostagent",
    "usernet",
    "ssh-controlmaster",
  ]);
  assert.deepEqual(COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS, [
    "hostagent-control",
    "usernet-control",
    "ssh-control",
    "engine-api",
  ]);
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.causal_nodes, 4);
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.causal_edges, 4);
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.causal_depth, 2);
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.receipt_binding_schema,
    "synveda.clean-engine.provider-effect-receipt-binding.v1",
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.terminal_receipt_schema,
    "synveda.clean-engine.receipt.v6",
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS.receipt_binding,
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.receipt_binding_schema,
  );
  assert.equal(
    COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS.terminal_receipt,
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.terminal_receipt_schema,
  );
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS.length, 18);
  assert.equal(COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS.length, 11);
  assert.deepEqual(
    Object.values(COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities),
    Object.values(
      COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities,
    ).map(() => false),
  );
  assert.deepEqual(
    Object.entries(
      COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities,
    )
      .filter(([, authorized]) => authorized)
      .map(([name]) => name),
    [
      "cleanup_authorized",
      "effect_witness_publication_authorized",
      "fixture_effect_execution_authorized",
      "fixture_effect_recovery_authorized",
      "marker_link_authorized",
      "marker_retirement_authorized",
      "process_group_ownership_authorized",
      "process_signal_authorized",
      "process_spawn_authorized",
      "process_start_authorized",
      "receipt_publication_authorized",
      "start_attempt_publication_authorized",
      "start_authority_publication_authorized",
      "state_effect_slot_publication_authorized",
      "state_evidence_publication_authorized",
    ],
  );
  assert.equal(
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities
      .fixture_effect_recovery_authorized,
    true,
  );
  for (const value of [
    COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS,
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
    COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
    COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS,
    COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS,
    COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS,
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS,
    COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT,
    COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT,
  ]) {
    assertRecursivelyFrozen(value);
  }
});

test("canonical bytes reject ambiguous, executable and unbounded values", () => {
  assert.equal(
    liveProviderEffectBytes({ b: 1, a: 2 }).toString("utf8"),
    '{"a":2,"b":1}\n',
  );
  const shared = { value: "same" };
  assert.deepEqual(
    liveProviderEffectBytes({ left: shared, right: shared }),
    liveProviderEffectBytes({
      left: { value: "same" },
      right: { value: "same" },
    }),
  );
  for (const value of [
    -0,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    1.5,
    Number.MAX_SAFE_INTEGER + 1,
    1n,
    undefined,
    () => undefined,
    Symbol("value"),
    new Date(0),
    new Map(),
    Object.create(null),
    nestedArray(17),
    binaryTree(10),
    Array.from({ length: 65 }, () => "value"),
    "x".repeat(65_534),
  ]) {
    expectRefusal(() => liveProviderEffectBytes(value));
  }
  assert.doesNotThrow(() => liveProviderEffectBytes(nestedArray(16)));
  assert.doesNotThrow(() => liveProviderEffectBytes([binaryTree(9)]));
  expectRefusal(() => liveProviderEffectBytes([binaryTree(9), "leaf"]));
  assert.doesNotThrow(() =>
    liveProviderEffectBytes(Array.from({ length: 64 }, () => "value")),
  );
  assert.doesNotThrow(() => liveProviderEffectBytes("x".repeat(65_533)));

  const sparse = [];
  sparse.length = 2;
  sparse[1] = "value";
  expectRefusal(() => liveProviderEffectBytes(sparse), /dense/u);
  const arrayAccessor = ["value"];
  let arrayGetterCalled = false;
  Object.defineProperty(arrayAccessor, "0", {
    enumerable: true,
    get() {
      arrayGetterCalled = true;
      return "changed";
    },
  });
  expectRefusal(() => liveProviderEffectBytes(arrayAccessor), /dense/u);
  assert.equal(arrayGetterCalled, false);
  const namedArray = ["value"];
  namedArray.extra = true;
  expectRefusal(() => liveProviderEffectBytes(namedArray), /dense/u);
  const symbolArray = ["value"];
  symbolArray[Symbol("extra")] = true;
  expectRefusal(() => liveProviderEffectBytes(symbolArray), /symbol/u);
  const customArray = ["value"];
  Object.setPrototypeOf(customArray, { custom: true });
  expectRefusal(() => liveProviderEffectBytes(customArray), /dense/u);
  const record64 = Object.fromEntries(
    Array.from({ length: 64 }, (_, index) => [`field-${index}`, true]),
  );
  assert.doesNotThrow(() => liveProviderEffectBytes(record64));
  expectRefusal(() =>
    liveProviderEffectBytes({ ...record64, "field-64": true }),
  );
  const hidden = { visible: true };
  Object.defineProperty(hidden, "hidden", { value: true });
  expectRefusal(() => liveProviderEffectBytes(hidden), /hidden/u);
  const symbol = { visible: true };
  symbol[Symbol("hidden")] = true;
  expectRefusal(() => liveProviderEffectBytes(symbol), /symbol/u);
  const inherited = Object.create({ inherited: true });
  inherited.visible = true;
  expectRefusal(() => liveProviderEffectBytes(inherited), /prototype/u);
  let getterCalled = false;
  const accessor = {};
  Object.defineProperty(accessor, "value", {
    enumerable: true,
    get() {
      getterCalled = true;
      return "value";
    },
  });
  expectRefusal(() => liveProviderEffectBytes(accessor), /accessor/u);
  assert.equal(getterCalled, false);
  const cyclic = {};
  cyclic.self = cyclic;
  expectRefusal(() => liveProviderEffectBytes(cyclic), /cycle/u);
  expectRefusal(
    () => liveProviderEffectBytes(new Proxy({ value: true }, {})),
    /proxy/u,
  );
  const revocable = Proxy.revocable({ value: true }, {});
  revocable.revoke();
  expectRefusal(() => liveProviderEffectBytes(revocable), /proxy/u);
});

test("the fixture API is closed, content-free and returns no signing authority", () => {
  const invalidInputs = [
    null,
    [],
    "retired",
    { outcome: undefined },
    { outcome: "unknown" },
    { extra: true },
    Object.create(null),
    new Proxy({ outcome: "retired" }, {}),
  ];
  for (const input of invalidInputs) {
    assert.throws(
      () => cleanEngineLiveProviderEffectFixture(input),
      /fixture input|fixture outcome/u,
    );
  }
  const accessor = {};
  let getterCalled = false;
  Object.defineProperty(accessor, "outcome", {
    enumerable: true,
    get() {
      getterCalled = true;
      return "retired";
    },
  });
  assert.throws(
    () => cleanEngineLiveProviderEffectFixture(accessor),
    /fixture input/u,
  );
  assert.equal(getterCalled, false);
  const hidden = {};
  Object.defineProperty(hidden, "outcome", {
    enumerable: false,
    value: "retired",
  });
  assert.throws(
    () => cleanEngineLiveProviderEffectFixture(hidden),
    /fixture input/u,
  );
  const symbol = { outcome: "retired" };
  symbol[Symbol("extra")] = true;
  assert.throws(
    () => cleanEngineLiveProviderEffectFixture(symbol),
    /fixture input/u,
  );

  const fixture = prepared("retired");
  assert.deepEqual(Object.keys(fixture), [
    "events",
    "fixtureOnly",
    "publicationPlan",
    "publisherAuthorityChain",
    "publisherAuthoritySha256",
    "source",
    "upstream",
    "witness",
  ]);
  assert.equal(fixture.fixtureOnly, true);
  assert.equal(Object.hasOwn(fixture, "roleKeys"), false);
  assertRecursivelyFrozen(fixture);
  const serialized = JSON.stringify(fixture);
  assert.doesNotMatch(serialized, /PRIVATE KEY|privateKey/u);
  assert.doesNotMatch(
    serialized,
    /"(?:argv|cwd|environment|executable|path|pid|credential|secret)":/u,
  );
});

test("all closed histories and every prefix validate with stable terminal bindings", () => {
  const variants = [
    {
      digest: "3c5cf35361e874e7ff3538192783089cf502b22cde4f9b617b0a12f61dd3367d",
      events: 1,
      outcome: "pre-attempt-retired",
      terminal: "completion",
    },
    {
      digest: "bcae354d262b3d49f2e9bc3fe4b8d712ab0714a001b540e008485c680f1c0c2f",
      events: 2,
      outcome: "authority-retired",
      terminal: "completion",
    },
    {
      digest: "f554e81fbb8aa52776c639f4b3876eb4656c385db2f7f1d825a027b701470caf",
      events: 11,
      outcome: "residual",
      terminal: "completion",
    },
    {
      digest: "a3fad401cf7a84a86c1c78ba2394c33b19c1e71c9ede9864e4b9cba52cf0e12a",
      events: 12,
      outcome: "uncertain",
      terminal: "uncertain-start",
    },
    {
      digest: "b85815f48d8e5577cd3f0c5465a1ab13e092b15a088647648bea9c8f01365aae",
      events: 44,
      outcome: "retired",
      terminal: "completion",
    },
    {
      digest: "dd6a8d8fb3a0d3c0140853661c58a0f86cd368db043fdccf0dfae7c1fbe79f60",
      events: 54,
      outcome: "rich-retired",
      terminal: "completion",
    },
  ];
  const seenKinds = new Set();
  for (const variant of variants) {
    const fixture = prepared(variant.outcome);
    assert.equal(fixture.events.length, variant.events);
    assert.equal(fixture.events.at(-1).event_kind, variant.terminal);
    assert.equal(digest(fixture.events.at(-1)), variant.digest);
    assert.equal(fixture.publisherAuthorityChain.length, 1);
    assert.deepEqual(fixture.publisherAuthorityChain[0], {
      authority_sha256: fixture.witness.slot_sha256,
      first_event_sequence: 0,
      kind: "owner-slot",
      prior_authority_sha256: ZERO_SHA256,
      recovery_claim_sha256: ZERO_SHA256,
    });
    assert.equal(
      validateColimaLiveProviderEffectEventHistory(
        fixture.events,
        validationOptions(fixture),
      ),
      fixture.events,
    );
    for (const [index, event] of fixture.events.entries()) {
      seenKinds.add(event.event_kind);
      assert.equal(event.event_sequence, index);
      assert.equal(
        event.parent_sha256,
        index === 0
          ? digest(fixture.witness)
          : digest(fixture.events[index - 1]),
      );
      assert.equal(event.publisher_authority_sha256, fixture.witness.slot_sha256);
      assert.equal(validateAt(fixture, index), event);
    }
  }
  assert.deepEqual([...seenKinds].sort(), [...COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS].sort());
});

test("publication plans, witnesses and nested contracts reject every mutation", () => {
  const fixture = prepared("retired");
  const bindingFixture = prepared("pre-attempt-retired");
  assert.equal(
    validateColimaLiveProviderEffectPublicationPlan(
      fixture.publicationPlan,
      fixture.source,
    ),
    fixture.publicationPlan,
  );
  assert.equal(
    validateColimaLiveProviderEffectWitness(
      fixture.witness,
      {
        publicationPlan: fixture.publicationPlan,
        source: fixture.source,
      },
    ),
    fixture.witness,
  );
  for (const field of Object.keys(fixture.publicationPlan)) {
    const changed = clone(fixture.publicationPlan);
    changed[field] = changedValue(changed[field]);
    expectRefusal(() =>
      validateColimaLiveProviderEffectEventHistory(
        bindingFixture.events,
        {
          ...validationOptions(bindingFixture),
          publicationPlan: changed,
        },
      ),
    );
  }
  for (const field of Object.keys(fixture.witness)) {
    const changed = clone(fixture.witness);
    changed[field] = changedValue(changed[field]);
    expectRefusal(() =>
      validateColimaLiveProviderEffectEventHistory(
        bindingFixture.events,
        {
          ...validationOptions(bindingFixture),
          witness: changed,
        },
      ),
    );
  }
  const nestedPaths = [
    ...Object.keys(fixture.publicationPlan.invocation_binding).map((field) => [
      "invocation_binding",
      field,
    ]),
    ...Object.keys(fixture.publicationPlan.provider_root_identity).map(
      (field) => ["provider_root_identity", field],
    ),
    ...Object.keys(fixture.publicationPlan.state_run_identity).map((field) => [
      "state_run_identity",
      field,
    ]),
    ...fixture.publicationPlan.namespace_bindings.flatMap((entry, index) =>
      Object.keys(entry).map((field) => ["namespace_bindings", index, field]),
    ),
    ...fixture.publicationPlan.namespace_bindings.flatMap((entry, index) =>
      entry.baseline_descriptors.flatMap((descriptor, descriptorIndex) =>
        Object.keys(descriptor).map((field) => [
          "namespace_bindings",
          index,
          "baseline_descriptors",
          descriptorIndex,
          field,
        ]),
      ),
    ),
    ...fixture.publicationPlan.planned_role_contracts.flatMap((entry, index) =>
      Object.keys(entry).map((field) => [
        "planned_role_contracts",
        index,
        field,
      ]),
    ),
    ...fixture.publicationPlan.planned_endpoint_contracts.flatMap(
      (entry, index) =>
        Object.keys(entry).map((field) => [
          "planned_endpoint_contracts",
          index,
          field,
        ]),
    ),
  ];
  for (const path of nestedPaths) {
    const changed = setAtPath(fixture.publicationPlan, path);
    expectRefusal(() =>
      validateColimaLiveProviderEffectEventHistory(
        bindingFixture.events,
        {
          ...validationOptions(bindingFixture),
          publicationPlan: changed,
        },
      ),
    );
  }
  const forgedBaseline = clone(fixture.publicationPlan);
  const forgedBinding = forgedBaseline.namespace_bindings.find(
    (binding) => binding.role === "lima-home-namespace",
  );
  forgedBinding.baseline_relative_identity_hmac_sha256 = [
    digest("caller-selected-foreign-baseline"),
  ];
  forgedBinding.baseline_descriptors = [
    {
      descriptor_sha256: digest("caller-selected-foreign-descriptor"),
      relative_identity_hmac_sha256:
        forgedBinding.baseline_relative_identity_hmac_sha256[0],
    },
  ];
  forgedBinding.baseline_set_binding_sha256 = digest({
    baseline_descriptor_set_hmac_sha256:
      forgedBinding.baseline_descriptor_set_hmac_sha256,
    baseline_descriptors: forgedBinding.baseline_descriptors,
    baseline_relative_identity_hmac_sha256:
      forgedBinding.baseline_relative_identity_hmac_sha256,
    observed_entry_set_hmac_sha256:
      forgedBinding.observed_entry_set_hmac_sha256,
    role: forgedBinding.role,
  });
  expectRefusal(
    () =>
      validateColimaLiveProviderEffectPublicationPlan(
        forgedBaseline,
        fixture.source,
      ),
    /namespace baseline/u,
  );
  const rootIdentity =
    fixture.publicationPlan.namespace_bindings[0]
      .namespace_identity_hmac_sha256;
  const endpointCollision = clone(fixture.publicationPlan);
  endpointCollision.planned_endpoint_contracts[0]
    .path_identity_hmac_sha256 = rootIdentity;
  expectRefusal(
    () =>
      validateColimaLiveProviderEffectPublicationPlan(
        endpointCollision,
        fixture.source,
      ),
    /publication plan was refused/u,
  );
  const baselineIdentity = fixture.publicationPlan.namespace_bindings
    .find((binding) => binding.role === "lima-home-namespace")
    .baseline_relative_identity_hmac_sha256[0];
  const contextCollision = clone(fixture.publicationPlan);
  const engineEndpoint = contextCollision.planned_endpoint_contracts.find(
    (contract) => contract.endpoint_kind === "engine-api",
  );
  engineEndpoint.docker_context_path_identity_hmac_sha256 = baselineIdentity;
  expectRefusal(
    () =>
      validateColimaLiveProviderEffectPublicationPlan(
        contextCollision,
        fixture.source,
      ),
    /publication plan was refused/u,
  );
  for (const field of Object.keys(fixture.source)) {
    const source = clone(fixture.source);
    source[field] = changedValue(source[field]);
    expectRefusal(() =>
      validateColimaLiveProviderEffectPublicationPlan(
        fixture.publicationPlan,
        source,
      ),
    );
  }
  for (const value of [fixture.publicationPlan, fixture.witness]) {
    const missing = clone(value);
    delete missing[Object.keys(missing)[0]];
    const extra = { ...clone(value), unexpected: true };
    const operation = value === fixture.publicationPlan
      ? (candidate) =>
          validateColimaLiveProviderEffectPublicationPlan(
            candidate,
            fixture.source,
          )
      : (candidate) =>
          validateColimaLiveProviderEffectWitness(candidate, {
            publicationPlan: fixture.publicationPlan,
            source: fixture.source,
          });
    expectRefusal(() => operation(missing));
    expectRefusal(() => operation(extra));
  }
});

test("publisher authority is owner-first and permits only chained fixture recovery", () => {
  const fixture = prepared("pre-attempt-retired");
  const validate = (publisherAuthorityChain, history = fixture.events) =>
    validateColimaLiveProviderEffectEventHistory(history, {
      ...validationOptions(fixture),
      publisherAuthorityChain,
    });
  assert.doesNotThrow(() => validate(fixture.publisherAuthorityChain));
  for (const field of Object.keys(fixture.publisherAuthorityChain[0])) {
    const changed = clone(fixture.publisherAuthorityChain);
    changed[0][field] = changedValue(changed[0][field]);
    expectRefusal(() => validate(changed), /publisher authority/u);
  }
  for (const invalid of [
    [],
    [{}],
    [{ ...clone(fixture.publisherAuthorityChain[0]), unexpected: true }],
    new Proxy(clone(fixture.publisherAuthorityChain), {}),
  ]) {
    expectRefusal(() => validate(invalid), /publisher authority/u);
  }
  const claimSha256 = "a".repeat(64);
  const structuralRecovery = [
    ...clone(fixture.publisherAuthorityChain),
    {
      authority_sha256: claimSha256,
      first_event_sequence: 0,
      kind: "state-recovery-claim",
      prior_authority_sha256: fixture.witness.slot_sha256,
      recovery_claim_sha256: claimSha256,
    },
  ];
  const recoveredEvidence = clone(fixture.events[0].evidence);
  recoveredEvidence.marker_retirement_binding_sha256 = digest({
    cleanup_settlement_sha256:
      recoveredEvidence.cleanup_settlement_sha256,
    marker_retirement_observation_sha256:
      recoveredEvidence.marker_retirement_observation_sha256,
    provider_root_fsync_observation_sha256:
      recoveredEvidence.provider_root_fsync_observation_sha256,
    publisher_authority_sha256: claimSha256,
    receipt_sha256: recoveredEvidence.receipt_sha256,
    witness_identity_sha256: recoveredEvidence.witness_identity_sha256,
    witness_link_count: recoveredEvidence.witness_link_count,
  });
  const recoveredCompletion = buildColimaLiveProviderEffectEvent({
    eventKind: "completion",
    evidence: recoveredEvidence,
    history: [],
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: structuralRecovery,
    publisherAuthoritySha256: claimSha256,
    source: fixture.source,
    witness: fixture.witness,
  });
  assert.doesNotThrow(() =>
    validate(structuralRecovery, [recoveredCompletion]),
  );
  const supersedingClaimSha256 = "c".repeat(64);
  const repeatedRecovery = [
    ...structuralRecovery,
    {
      authority_sha256: supersedingClaimSha256,
      first_event_sequence: 0,
      kind: "state-recovery-claim",
      prior_authority_sha256: claimSha256,
      recovery_claim_sha256: supersedingClaimSha256,
    },
  ];
  const repeatedRecoveryEvidence = clone(recoveredEvidence);
  repeatedRecoveryEvidence.marker_retirement_binding_sha256 = digest({
    cleanup_settlement_sha256:
      repeatedRecoveryEvidence.cleanup_settlement_sha256,
    marker_retirement_observation_sha256:
      repeatedRecoveryEvidence.marker_retirement_observation_sha256,
    provider_root_fsync_observation_sha256:
      repeatedRecoveryEvidence.provider_root_fsync_observation_sha256,
    publisher_authority_sha256: supersedingClaimSha256,
    receipt_sha256: repeatedRecoveryEvidence.receipt_sha256,
    witness_identity_sha256:
      repeatedRecoveryEvidence.witness_identity_sha256,
    witness_link_count: repeatedRecoveryEvidence.witness_link_count,
  });
  const repeatedRecoveryCompletion = buildColimaLiveProviderEffectEvent({
    eventKind: "completion",
    evidence: repeatedRecoveryEvidence,
    history: [],
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: repeatedRecovery,
    publisherAuthoritySha256: supersedingClaimSha256,
    source: fixture.source,
    witness: fixture.witness,
  });
  assert.doesNotThrow(() =>
    validate(repeatedRecovery, [repeatedRecoveryCompletion]),
  );

  const authorityFixture = prepared("authority-retired");
  const authorityClaimSha256 = "b".repeat(64);
  const authorityRecovery = [
    ...clone(authorityFixture.publisherAuthorityChain),
    {
      authority_sha256: authorityClaimSha256,
      first_event_sequence: 1,
      kind: "state-recovery-claim",
      prior_authority_sha256: authorityFixture.witness.slot_sha256,
      recovery_claim_sha256: authorityClaimSha256,
    },
  ];
  const authorityRecoveryEvidence = clone(
    authorityFixture.events[1].evidence,
  );
  authorityRecoveryEvidence.marker_retirement_binding_sha256 = digest({
    cleanup_settlement_sha256:
      authorityRecoveryEvidence.cleanup_settlement_sha256,
    marker_retirement_observation_sha256:
      authorityRecoveryEvidence.marker_retirement_observation_sha256,
    provider_root_fsync_observation_sha256:
      authorityRecoveryEvidence.provider_root_fsync_observation_sha256,
    publisher_authority_sha256: authorityClaimSha256,
    receipt_sha256: authorityRecoveryEvidence.receipt_sha256,
    witness_identity_sha256:
      authorityRecoveryEvidence.witness_identity_sha256,
    witness_link_count: authorityRecoveryEvidence.witness_link_count,
  });
  const authorityRecoveryCompletion =
    buildColimaLiveProviderEffectEvent({
      eventKind: "completion",
      evidence: authorityRecoveryEvidence,
      history: [authorityFixture.events[0]],
      publicationPlan: authorityFixture.publicationPlan,
      publisherAuthorityChain: authorityRecovery,
      publisherAuthoritySha256: authorityClaimSha256,
      source: authorityFixture.source,
      witness: authorityFixture.witness,
    });
  assert.doesNotThrow(() =>
    validateColimaLiveProviderEffectEventHistory(
      [authorityFixture.events[0], authorityRecoveryCompletion],
      {
        ...validationOptions(authorityFixture),
        publisherAuthorityChain: authorityRecovery,
      },
    ),
  );
  expectRefusal(
    () =>
      buildColimaLiveProviderEffectEvent({
        eventKind: "start-authority",
        evidence: authorityFixture.events[0].evidence,
        history: [],
        publicationPlan: authorityFixture.publicationPlan,
        publisherAuthorityChain: [
          authorityFixture.publisherAuthorityChain[0],
          {
            authority_sha256: authorityClaimSha256,
            first_event_sequence: 0,
            kind: "state-recovery-claim",
            prior_authority_sha256: authorityFixture.witness.slot_sha256,
            recovery_claim_sha256: authorityClaimSha256,
          },
        ],
        publisherAuthoritySha256: authorityClaimSha256,
        source: authorityFixture.source,
        witness: authorityFixture.witness,
      }),
    /event envelope/u,
  );
  const malformedRecoveries = [
    setAtPath(structuralRecovery, [1, "authority_sha256"]),
    setAtPath(structuralRecovery, [1, "first_event_sequence"], () => 2),
    setAtPath(structuralRecovery, [1, "kind"]),
    setAtPath(structuralRecovery, [1, "prior_authority_sha256"]),
    setAtPath(structuralRecovery, [1, "recovery_claim_sha256"]),
  ];
  for (const invalid of malformedRecoveries) {
    expectRefusal(() => validate(invalid), /publisher authority/u);
  }
  const tooMany = [clone(fixture.publisherAuthorityChain[0])];
  for (let index = 1; index <= 64; index += 1) {
    const authoritySha256 = digest(`recovery-${index}`);
    tooMany.push({
      authority_sha256: authoritySha256,
      first_event_sequence: index,
      kind: "state-recovery-claim",
      prior_authority_sha256: tooMany.at(-1).authority_sha256,
      recovery_claim_sha256: authoritySha256,
    });
  }
  expectRefusal(() => validate(tooMany), /publisher authority chain/u);

  const accessor = [];
  Object.defineProperty(accessor, "0", {
    enumerable: true,
    get() {
      throw new Error("authority getter must not run");
    },
  });
  accessor.length = 1;
  expectRefusal(() => validate(accessor));
});

test("event envelopes and every evidence field are hash-chain tamper evident", () => {
  const retired = prepared("retired");
  const uncertain = prepared("uncertain");
  const fixturesByKind = new Map(
    COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS.map((kind) => [
      kind,
      kind === "uncertain-start" ? uncertain : retired,
    ]),
  );
  for (const kind of COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS) {
    const fixture = fixturesByKind.get(kind);
    const index = eventIndex(fixture, kind);
    const original = fixture.events[index];
    for (const field of Object.keys(original.evidence)) {
      const history = clone(fixture.events);
      history[index].evidence[field] = changedValue(
        history[index].evidence[field],
      );
      expectRefusal(() =>
        validateColimaLiveProviderEffectEventHistory(
          history,
          validationOptions(fixture),
        ),
      );
    }
    const missing = clone(original);
    delete missing.evidence[Object.keys(missing.evidence)[0]];
    expectRefusal(() => validateAt(fixture, index, missing));
    const extra = clone(original);
    extra.evidence.unexpected = true;
    expectRefusal(() => validateAt(fixture, index, extra));
  }

  const completionIndex = eventIndex(retired, "completion");
  const completion = retired.events[completionIndex];
  for (const field of Object.keys(completion).filter(
    (name) => name !== "evidence",
  )) {
    const history = clone(retired.events);
    history[completionIndex][field] = changedValue(
      history[completionIndex][field],
    );
    expectRefusal(() =>
      validateColimaLiveProviderEffectEventHistory(
        history,
        validationOptions(retired),
      ),
    );
  }
  const reordered = clone(retired.events);
  [reordered[0], reordered[1]] = [reordered[1], reordered[0]];
  expectRefusal(() =>
    validateColimaLiveProviderEffectEventHistory(
      reordered,
      validationOptions(retired),
    ),
  );
  expectRefusal(() =>
    validateColimaLiveProviderEffectEventHistory(
      [...retired.events, retired.events.at(-1)],
      validationOptions(retired),
    ),
  );
  expectRefusal(() =>
    buildEvent(retired, retired.events, retired.events.at(-1)),
  );
  expectRefusal(() =>
    buildEvent(uncertain, uncertain.events, uncertain.events.at(-1)),
  );
});

test("a fully valid production plan and witness remain publication-deny-only", () => {
  const production = productionPrepared();
  assert.equal(production.publicationPlan.evidence_class, "production-pinned");
  assert.equal(production.witness.evidence_class, "production-pinned");
  assert.equal(
    validateColimaLiveProviderEffectPublicationPlan(
      production.publicationPlan,
      production.source,
    ),
    production.publicationPlan,
  );
  assert.equal(
    validateColimaLiveProviderEffectWitness(production.witness, {
      publicationPlan: production.publicationPlan,
      source: production.source,
    }),
    production.witness,
  );
  expectRefusal(
    () =>
      buildColimaLiveProviderEffectEvent({
        eventKind: "completion",
        evidence: {},
        history: [],
        publicationPlan: production.publicationPlan,
        publisherAuthorityChain: production.publisherAuthorityChain,
        publisherAuthoritySha256: production.witness.slot_sha256,
        source: production.source,
        witness: production.witness,
      }),
    /production live provider effect publication was refused/u,
  );
  const fixture = prepared("pre-attempt-retired");
  expectRefusal(() =>
    validateColimaLiveProviderEffectPublicationPlan(
      fixture.publicationPlan,
      production.source,
    ),
  );
  expectRefusal(() =>
    validateColimaLiveProviderEffectPublicationPlan(
      production.publicationPlan,
      fixture.source,
    ),
  );
});

test("signed roles, endpoints and quiescence acknowledgements bind their exact parents", () => {
  const fixture = prepared("retired");
  for (const kind of ["role-identity", "endpoint", "quiescence-ack"]) {
    const index = eventIndex(fixture, kind);
    const changedProof = clone(fixture.events[index]);
    const proof = changedProof.evidence.proof_base64;
    changedProof.evidence.proof_base64 = `${
      proof[0] === "A" ? "B" : "A"
    }${proof.slice(1)}`;
    expectRefusal(() => validateAt(fixture, index, changedProof), /proof/u);
  }
  const identities = fixture.events.filter(
    (event) => event.event_kind === "role-identity",
  );
  const identityIndex = eventIndex(fixture, "role-identity");
  const wrongKey = clone(fixture.events[identityIndex]);
  wrongKey.evidence.public_key_spki_der_base64 =
    identities[1].evidence.public_key_spki_der_base64;
  expectRefusal(() => validateAt(fixture, identityIndex, wrongKey));
  const wrongParent = clone(fixture.events[identityIndex]);
  wrongParent.parent_sha256 = digest("wrong-parent");
  expectRefusal(() => validateAt(fixture, identityIndex, wrongParent));

  const fence = fixture.events.find(
    (event) => event.event_kind === "quiescence-fence",
  );
  assert.equal(
    fence.evidence.process_reconciliation.observed_process_count,
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length,
  );
  assert.equal(
    fixture.events.filter((event) => event.event_kind === "launch-edge").length,
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length,
  );
  assert.equal(identities.length, COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length);
  assert.equal(
    Math.max(
      ...fixture.events
        .filter((event) => event.event_kind === "launch-edge")
        .map((event) => event.evidence.depth),
    ),
    COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.causal_depth,
  );
  assert.equal(
    fixture.events.filter((event) => event.event_kind === "quiescence-ack")
      .length,
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length,
  );
  const endpoints = fixture.events.filter(
    (event) => event.event_kind === "endpoint",
  );
  assert.deepEqual(
    endpoints.map((event) => event.evidence.phase),
    [
      ...COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.map(() => "initial"),
      ...COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.map(
        () => "post-detachment-final",
      ),
    ],
  );
  for (const kind of COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS) {
    const pair = endpoints.filter(
      (event) => event.evidence.endpoint_kind === kind,
    );
    assert.equal(pair.length, 2);
    assert.equal(
      pair[1].evidence.prior_endpoint_event_sha256,
      digest(pair[0]),
    );
    assert.notEqual(
      pair[0].evidence.challenge_sha256,
      pair[1].evidence.challenge_sha256,
    );
  }

  const firstEdgeIndex = eventIndex(fixture, "launch-edge");
  const unknownRole = clone(fixture.events[firstEdgeIndex]);
  unknownRole.evidence.child_role = "fifth-role";
  expectRefusal(
    () => validateAt(fixture, firstEdgeIndex, unknownRole),
    /launch edge/u,
  );
  const reparentedIndex = eventIndex(
    fixture,
    "launch-edge",
    (evidence) => evidence.child_role === "usernet",
  );
  const reparented = clone(fixture.events[reparentedIndex]);
  reparented.evidence.parent_node_identity_sha256 = identities.find(
    (event) => event.evidence.role === "outer",
  ).evidence.node_identity_sha256;
  expectRefusal(
    () => validateAt(fixture, reparentedIndex, reparented),
    /launch parent/u,
  );
});

test("predeclared role challenges are reserved before role publication", () => {
  const fixture = prepared("retired");
  const deliveryIndex = eventIndex(fixture, "delivery-result");
  const delivery = fixture.events[deliveryIndex];
  const changed = clone(delivery.evidence);
  changed.observation_sha256 =
    fixture.publicationPlan.planned_role_contracts[0].role_challenge_sha256;
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, deliveryIndex),
        delivery,
        changed,
      ),
    /fresh evidence was reused/u,
  );
});

test("the rich inventory proves paging, baseline closure and physical identity", () => {
  const fixture = prepared("rich-retired");
  const inventorySet = fixture.events.find(
    (event) => event.event_kind === "recursive-inventory-set",
  );
  const pages = fixture.events.filter(
    (event) => event.event_kind === "recursive-inventory-page",
  );
  assert.equal(pages.length, 18);
  assert.deepEqual(
    inventorySet.evidence.samples.map((sample) => [
      sample.namespace_role,
      sample.entry_count,
      sample.page_count,
    ]),
    [
      ["colima-cache-namespace", 1, 1],
      ["colima-home-namespace", 73, 4],
      ["docker-config-namespace", 2, 1],
      ["lima-home-namespace", 5, 1],
      ["private-home-namespace", 1, 1],
      ["temporary-namespace", 2, 1],
    ],
  );
  for (const sampleSequence of [0, 1]) {
    assert.deepEqual(
      pages
        .filter(
          (page) =>
            page.evidence.sample_sequence === sampleSequence &&
            page.evidence.namespace_role === "colima-home-namespace",
        )
        .map((page) => page.evidence.entries.length),
      [24, 24, 24, 1],
    );
  }
  const firstSampleEntries = pages
    .filter((page) => page.evidence.sample_sequence === 0)
    .flatMap((page) => page.evidence.entries);
  const typeCounts = Object.fromEntries(
    ["directory", "regular", "socket", "symlink"].map((type) => [
      type,
      firstSampleEntries.filter((entry) => entry.type === type).length,
    ]),
  );
  assert.deepEqual(typeCounts, {
    directory: 9,
    regular: 70,
    socket: 4,
    symlink: 1,
  });
  const hardLinks = firstSampleEntries.filter(
    (entry) => entry.hard_link_group_sha256 !== ZERO_SHA256,
  );
  assert.equal(hardLinks.length, 2);
  assert.equal(new Set(hardLinks.map((entry) => entry.inode)).size, 1);
  assert.equal(new Set(hardLinks.map((entry) => entry.links)).size, 1);
  assert.equal(hardLinks[0].links, "2");
  assert.equal(
    firstSampleEntries.filter(
      (entry) => entry.resource_kind === "closed-disk-image",
    ).length,
    1,
  );
  assert.equal(
    firstSampleEntries.filter(
      (entry) => entry.resource_kind === "docker-context",
    ).length,
    1,
  );
  assert.equal(
    firstSampleEntries.filter((entry) => entry.ownership === "admitted-baseline")
      .length,
    2,
  );
  assert.doesNotThrow(() => rebuildInventorySet(fixture, (entries) => entries));

  expectRefusal(
    () =>
      rebuildInventorySet(fixture, (entries, { namespaceRole }) =>
        namespaceRole === "lima-home-namespace"
          ? entries.filter((entry) => entry.ownership !== "admitted-baseline")
          : entries,
      ),
    /inventory ownership was unreachable/u,
  );
  expectRefusal(
    () =>
      rebuildInventorySet(fixture, (entries) =>
        entries.filter((entry) => entry.resource_kind !== "docker-context"),
      ),
    /inventory ownership was unreachable/u,
  );
  expectRefusal(
    () =>
      rebuildInventorySet(
        fixture,
        (entries, { namespaceRole, sampleSequence }) => {
          if (
            namespaceRole !== "colima-home-namespace" ||
            sampleSequence !== 1
          ) {
            return entries;
          }
          const target = entries.find(
            (entry) =>
              entry.type === "regular" &&
              entry.links === "1" &&
              entry.resource_kind === "none" &&
              entry.ownership === "generation-owned",
          );
          target.content_sha256 = digest("changed-second-sample-content");
          return entries;
        },
      ),
    /samples differed/u,
  );
  expectRefusal(
    () =>
      rebuildInventorySet(fixture, (entries, { namespaceRole }) => {
        if (namespaceRole !== "colima-home-namespace") return entries;
        const aliases = entries.filter(
          (entry) => entry.hard_link_group_sha256 !== ZERO_SHA256,
        );
        aliases[1].ctime_nanoseconds = "1000000501";
        return entries;
      }),
    /physical alias/u,
  );
});

test("sample zero global closure precedes the first sample-one page", () => {
  const fixture = prepared("rich-retired");
  assert.doesNotThrow(() =>
    rebuildFirstInventorySampleBoundary(fixture, () => undefined),
  );
  expectRefusal(
    () =>
      rebuildFirstInventorySampleBoundary(
        fixture,
        (entries, { namespaceRole }) => {
          if (namespaceRole !== "colima-home-namespace") return entries;
          const aliases = entries.filter(
            (entry) => entry.hard_link_group_sha256 !== ZERO_SHA256,
          );
          aliases[1].mtime_nanoseconds = "1000000501";
          return entries;
        },
      ),
    /physical alias/u,
  );
  expectRefusal(
    () =>
      rebuildFirstInventorySampleBoundary(
        fixture,
        (entries, { namespaceRole }) =>
          namespaceRole === "colima-home-namespace"
            ? entries.filter(
                (entry) =>
                  entry.hard_link_group_sha256 === ZERO_SHA256 ||
                  entry.relative_identity_hmac_sha256 ===
                    entries.find(
                      (candidate) =>
                        candidate.hard_link_group_sha256 !== ZERO_SHA256,
                    ).relative_identity_hmac_sha256,
              )
            : entries,
      ),
    /hard-link/u,
  );
  let socketRemoved = false;
  expectRefusal(
    () =>
      rebuildFirstInventorySampleBoundary(fixture, (entries) =>
        entries.filter((entry) => {
          if (!socketRemoved && entry.type === "socket") {
            socketRemoved = true;
            return false;
          }
          return true;
        }),
      ),
    /inventory ownership was unreachable/u,
  );
  expectRefusal(
    () =>
      rebuildFirstInventorySampleBoundary(fixture, (entries) => {
        const dockerContext = entries.find(
          (entry) => entry.resource_kind === "docker-context",
        );
        if (dockerContext !== undefined) {
          dockerContext.content_sha256 = digest("wrong-docker-context");
        }
        return entries;
      }),
    /reserved inventory identity was refused/u,
  );
});

test("baseline entries cannot survive beneath a generation-owned ancestor", () => {
  const fixture = prepared("rich-retired");
  const namespaceBinding = fixture.publicationPlan.namespace_bindings.find(
    (binding) => binding.role === "lima-home-namespace",
  );
  const generationIdentity = digest("adversarial-generation-parent");
  expectRefusal(
    () =>
      rebuildInventorySet(fixture, (entries, { namespaceRole }) => {
        if (namespaceRole !== "lima-home-namespace") return entries;
        const root = entries[0];
        const baselineLeaf = entries.find(
          (entry) =>
            entry.ownership === "admitted-baseline" &&
            entry.type === "regular",
        );
        baselineLeaf.parent_identity_hmac_sha256 = generationIdentity;
        const generationParent = {
          ...clone(root),
          content_disposition: "metadata-only",
          ctime_nanoseconds: "1000000600",
          depth: 1,
          directory_entry_count: 1,
          entry_sequence: 0,
          inode: "7777",
          mtime_nanoseconds: "1000000600",
          name_bytes: 12,
          ownership: "generation-owned",
          ownership_binding_sha256: digest({
            baseline_set_binding_sha256:
              namespaceBinding.baseline_set_binding_sha256,
            namespace_identity_hmac_sha256:
              namespaceBinding.namespace_identity_hmac_sha256,
            ownership: "generation-owned",
            relative_identity_hmac_sha256: generationIdentity,
            witness_sha256: digest(fixture.witness),
          }),
          parent_identity_hmac_sha256:
            namespaceBinding.namespace_identity_hmac_sha256,
          relative_identity_hmac_sha256: generationIdentity,
          size: "0",
        };
        const leafIndex = entries.indexOf(baselineLeaf);
        entries.splice(leafIndex, 0, generationParent);
        return entries;
      }),
    /baseline/u,
  );
});

test("admitted baseline identities cannot swap or drift from their descriptors", () => {
  const fixture = prepared("rich-retired");
  const pageIndex = eventIndex(
    fixture,
    "recursive-inventory-page",
    (evidence) =>
      evidence.namespace_role === "lima-home-namespace" &&
      evidence.sample_sequence === 0,
  );
  const original = fixture.events[pageIndex];
  const binding = fixture.publicationPlan.namespace_bindings.find(
    (candidate) => candidate.role === "lima-home-namespace",
  );
  const honestBaseline = original.evidence.entries.filter(
    (entry) => entry.ownership === "admitted-baseline",
  );
  assert.equal(honestBaseline.length, 2);
  for (const entry of honestBaseline) {
    const descriptor = binding.baseline_descriptors.find(
      (candidate) =>
        candidate.relative_identity_hmac_sha256 ===
        entry.relative_identity_hmac_sha256,
    );
    assert.equal(
      digest(
        Object.fromEntries(
          COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS.map((field) => [
            field,
            entry[field],
          ]),
        ),
      ),
      descriptor.descriptor_sha256,
    );
  }

  const swapped = clone(original.evidence);
  const directory = swapped.entries.find(
    (entry) =>
      entry.ownership === "admitted-baseline" && entry.type === "directory",
  );
  const regular = swapped.entries.find(
    (entry) =>
      entry.ownership === "admitted-baseline" && entry.type === "regular",
  );
  const directoryIdentity = directory.relative_identity_hmac_sha256;
  directory.relative_identity_hmac_sha256 =
    regular.relative_identity_hmac_sha256;
  regular.relative_identity_hmac_sha256 = directoryIdentity;
  regular.parent_identity_hmac_sha256 =
    directory.relative_identity_hmac_sha256;
  for (const entry of [directory, regular]) {
    entry.ownership_binding_sha256 = inventoryOwnershipBinding(
      fixture,
      binding,
      entry.relative_identity_hmac_sha256,
      "admitted-baseline",
    );
  }
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, pageIndex),
        original,
        swapped,
      ),
    /admitted baseline descriptor/u,
  );

  for (const field of COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS) {
    const changed = clone(original.evidence);
    const target = changed.entries.find(
      (entry) =>
        entry.ownership === "admitted-baseline" &&
        (field === "directory_entry_count"
          ? entry.type === "directory"
          : entry.type === "regular"),
    );
    if (field === "mode") {
      target[field] = "0640";
    } else if (field === "type") {
      target[field] = "symlink";
    } else if (typeof target[field] === "number") {
      target[field] += 1;
    } else if (/^[0-9]+$/u.test(target[field])) {
      target[field] = String(BigInt(target[field]) + 1n);
    } else if (/^[0-9a-f]{64}$/u.test(target[field])) {
      target[field] = digest(`changed-baseline-${field}`);
    } else {
      target[field] = `${target[field]}-changed`;
    }
    if (field === "relative_identity_hmac_sha256") {
      target.ownership_binding_sha256 = inventoryOwnershipBinding(
        fixture,
        binding,
        target.relative_identity_hmac_sha256,
        "admitted-baseline",
      );
    }
    expectRefusal(
      () =>
        buildEvent(
          fixture,
          fixture.events.slice(0, pageIndex),
          original,
          changed,
        ),
      undefined,
    );
  }

  const resourceRebound = clone(original.evidence);
  const reboundRegular = resourceRebound.entries.find(
    (entry) =>
      entry.ownership === "admitted-baseline" && entry.type === "regular",
  );
  reboundRegular.resource_kind = "docker-context";
  reboundRegular.endpoint_sha256 = digest("baseline-docker-context-endpoint");
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, pageIndex),
        original,
        resourceRebound,
      ),
    /admitted baseline descriptor/u,
  );
});

test("sample-zero prefixes refuse physical and reserved identity poisons", () => {
  const fixture = prepared("rich-retired");
  const pageIndex = eventIndex(
    fixture,
    "recursive-inventory-page",
    (evidence) =>
      evidence.namespace_role === "colima-home-namespace" &&
      evidence.sample_sequence === 0,
  );
  const original = fixture.events[pageIndex];
  const binding = fixture.publicationPlan.namespace_bindings.find(
    (candidate) => candidate.role === "colima-home-namespace",
  );
  const buildChangedPage = (evidence) =>
    buildEvent(
      fixture,
      fixture.events.slice(0, pageIndex),
      original,
      evidence,
    );

  const conflictingAlias = clone(original.evidence);
  const aliases = conflictingAlias.entries.filter(
    (entry) => entry.hard_link_group_sha256 !== ZERO_SHA256,
  );
  assert.equal(aliases.length, 2);
  aliases[1].mtime_nanoseconds = "1000000501";
  expectRefusal(
    () => buildChangedPage(conflictingAlias),
    /physical alias inventory/u,
  );

  const engineEndpoint = fixture.events.find(
    (event) =>
      event.event_kind === "endpoint" &&
      event.evidence.endpoint_kind === "engine-api" &&
      event.evidence.phase === "post-detachment-final",
  );
  const stolenReservation = clone(original.evidence);
  const ordinary = stolenReservation.entries.find(
    (entry) =>
      entry.type === "regular" &&
      entry.links === "1" &&
      entry.resource_kind === "none" &&
      entry.ownership === "generation-owned",
  );
  ordinary.relative_identity_hmac_sha256 =
    engineEndpoint.evidence.path_identity_hmac_sha256;
  ordinary.ownership_binding_sha256 = inventoryOwnershipBinding(
    fixture,
    binding,
    ordinary.relative_identity_hmac_sha256,
    "generation-owned",
  );
  expectRefusal(
    () => buildChangedPage(stolenReservation),
    /reserved inventory identity/u,
  );

  for (const [links, pattern] of [
    ["4097", /regular inventory/u],
    ["4096", /completion was impossible/u],
  ]) {
    const excessiveAliases = clone(original.evidence);
    const target = excessiveAliases.entries.find(
      (entry) =>
        entry.type === "regular" &&
        entry.links === "1" &&
        entry.resource_kind === "none" &&
        entry.ownership === "generation-owned",
    );
    target.links = links;
    target.hard_link_group_sha256 = digest(`hard-link-group-${links}`);
    expectRefusal(() => buildChangedPage(excessiveAliases), pattern);
  }

  assert.doesNotThrow(() =>
    rebuildInventorySet(
      fixture,
      (entries, { namespaceRole }) => {
        if (namespaceRole !== "colima-home-namespace") return entries;
        const generatedDirectory = entries.find(
          (entry) =>
            entry.type === "directory" &&
            entry.ownership === "generation-owned" &&
            entry.depth === 1,
        );
        const socket = entries.find(
          (entry) => entry.type === "socket",
        );
        socket.parent_identity_hmac_sha256 =
          generatedDirectory.relative_identity_hmac_sha256;
        socket.depth = generatedDirectory.depth + 1;
        return entries;
      },
    ),
  );

  const limaPageIndex = eventIndex(
    fixture,
    "recursive-inventory-page",
    (evidence) =>
      evidence.namespace_role === "lima-home-namespace" &&
      evidence.sample_sequence === 0,
  );
  const limaPage = fixture.events[limaPageIndex];
  const limaBinding = fixture.publicationPlan.namespace_bindings.find(
    (candidate) => candidate.role === "lima-home-namespace",
  );
  const generationTemplate = original.evidence.entries.find(
    (entry) =>
      entry.type === "regular" &&
      entry.links === "1" &&
      entry.resource_kind === "none" &&
      entry.ownership === "generation-owned",
  );
  const generationRegular = ({
    depth = 1,
    index,
    label,
    namespaceBinding,
    parentIdentity = namespaceBinding.namespace_identity_hmac_sha256,
  }) => {
    const identity = digest(label);
    return {
      ...clone(generationTemplate),
      depth,
      device: namespaceBinding.device,
      entry_sequence: index,
      inode: String(8_000 + index),
      name_bytes: Buffer.byteLength(label, "utf8"),
      ownership_binding_sha256: inventoryOwnershipBinding(
        fixture,
        namespaceBinding,
        identity,
        "generation-owned",
      ),
      parent_identity_hmac_sha256: parentIdentity,
      relative_identity_hmac_sha256: identity,
      uid: namespaceBinding.uid,
    };
  };
  const partitionedEntries = [clone(limaPage.evidence.entries[0])];
  partitionedEntries[0].directory_entry_count = 24;
  for (let index = 0; index < 23; index += 1) {
    partitionedEntries.push(
      generationRegular({
        index: index + 1,
        label: `lima-partition-generation-${index}`,
        namespaceBinding: limaBinding,
      }),
    );
  }
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, limaPageIndex),
        limaPage,
        { ...clone(limaPage.evidence), entries: partitionedEntries },
      ),
    /inventory ownership was unreachable/u,
  );

  const temporaryPageIndex = eventIndex(
    fixture,
    "recursive-inventory-page",
    (evidence) =>
      evidence.namespace_role === "temporary-namespace" &&
      evidence.sample_sequence === 0,
  );
  const temporaryPage = fixture.events[temporaryPageIndex];
  const temporaryBinding = fixture.publicationPlan.namespace_bindings.find(
    (candidate) => candidate.role === "temporary-namespace",
  );
  const capacityEntries = [clone(temporaryPage.evidence.entries[0])];
  capacityEntries[0].directory_entry_count = 24;
  const capacityAlias = generationRegular({
    index: 1,
    label: "temporary-capacity-hard-link",
    namespaceBinding: temporaryBinding,
  });
  capacityAlias.hard_link_group_sha256 = digest(
    "temporary-capacity-hard-link-group",
  );
  capacityEntries.push(capacityAlias);
  capacityEntries.push(clone(temporaryPage.evidence.entries[1]));
  for (let index = 0; index < 21; index += 1) {
    capacityEntries.push(
      generationRegular({
        index: capacityEntries.length,
        label: `temporary-capacity-filler-${index}`,
        namespaceBinding: temporaryBinding,
      }),
    );
  }
  capacityEntries.forEach((entry, index) => {
    entry.entry_sequence = index;
  });
  assert.equal(capacityEntries.length, 24);
  const capacityPage = (links) => {
    const entries = clone(capacityEntries);
    entries[1].links = links;
    return buildEvent(
      fixture,
      fixture.events.slice(0, temporaryPageIndex),
      temporaryPage,
      { ...clone(temporaryPage.evidence), entries },
    );
  };
  assert.doesNotThrow(() => capacityPage("1510"));
  expectRefusal(
    () => capacityPage("1511"),
    /inventory completion was impossible/u,
  );

  const depthEntries = [clone(temporaryPage.evidence.entries[0])];
  depthEntries[0].directory_entry_count = 17;
  let parentIdentity = temporaryBinding.namespace_identity_hmac_sha256;
  for (let depth = 1; depth <= 31; depth += 1) {
    const directory = generationRegular({
      depth,
      index: depthEntries.length,
      label: `temporary-closed-chain-${depth}`,
      namespaceBinding: temporaryBinding,
      parentIdentity,
    });
    directory.content_disposition = "metadata-only";
    directory.content_sha256 = ZERO_SHA256;
    directory.directory_entry_count = 1;
    directory.mode = "0700";
    directory.size = "0";
    directory.type = "directory";
    depthEntries.push(directory);
    parentIdentity = directory.relative_identity_hmac_sha256;
  }
  depthEntries.push(clone(temporaryPage.evidence.entries[1]));
  const depthAlias = generationRegular({
    index: depthEntries.length,
    label: "temporary-trapped-hard-link",
    namespaceBinding: temporaryBinding,
  });
  depthAlias.hard_link_group_sha256 = digest(
    "temporary-trapped-hard-link-group",
  );
  depthEntries.push(depthAlias);
  for (let index = 0; index < 14; index += 1) {
    depthEntries.push(
      generationRegular({
        index: depthEntries.length,
        label: `temporary-closed-filler-${index}`,
        namespaceBinding: temporaryBinding,
      }),
    );
  }
  depthEntries.forEach((entry, index) => {
    entry.entry_sequence = index;
  });
  assert.equal(depthEntries.length, 48);
  const firstDepthPage = buildEvent(
    fixture,
    fixture.events.slice(0, temporaryPageIndex),
    temporaryPage,
    {
      ...clone(temporaryPage.evidence),
      entries: depthEntries.slice(0, 24),
    },
  );
  const depthPage = (links) => {
    const entries = clone(depthEntries.slice(24));
    entries.find(
      (entry) =>
        entry.hard_link_group_sha256 ===
        depthAlias.hard_link_group_sha256,
    ).links = links;
    return buildEvent(
      fixture,
      [...fixture.events.slice(0, temporaryPageIndex), firstDepthPage],
      temporaryPage,
      {
        ...clone(temporaryPage.evidence),
        entries,
        entry_start: 24,
        page_sequence: 1,
        parent_page_sha256: digest(firstDepthPage),
      },
    );
  };
  assert.doesNotThrow(() => depthPage("2"));
  expectRefusal(
    () => depthPage("3"),
    /hard-link inventory was unreachable/u,
  );

  const dockerBinding = fixture.publicationPlan.namespace_bindings.find(
    (candidate) => candidate.role === "docker-config-namespace",
  );
  assert.equal(binding.device, dockerBinding.device);
  assert.equal(binding.uid, dockerBinding.uid);
  expectRefusal(
    () =>
      rebuildInventorySet(
        fixture,
        (entries, { namespaceRole }) =>
          namespaceRole === "colima-home-namespace"
            ? entries.filter(
                (entry, index) =>
                  entry.hard_link_group_sha256 === ZERO_SHA256 ||
                  index === entries.findIndex(
                    (candidate) =>
                      candidate.hard_link_group_sha256 !== ZERO_SHA256,
                  ),
              )
            : entries,
      ),
    /hard-link inventory was unreachable/u,
  );
});

test("proof bytes bind domain, sequence, parent, publisher, plan and witness", () => {
  const fixture = prepared("retired");
  const identityIndex = eventIndex(fixture, "role-identity");
  const identity = fixture.events[identityIndex];
  const input = {
    domain: "role-identity",
    eventSequence: identity.event_sequence,
    evidence: identity.evidence,
    parentSha256: identity.parent_sha256,
    publicationPlan: fixture.publicationPlan,
    publisherAuthoritySha256: identity.publisher_authority_sha256,
    source: fixture.source,
    witness: fixture.witness,
  };
  const bytes = colimaLiveProviderEffectProofBytes(input);
  const key = createPublicKey({
    format: "der",
    key: Buffer.from(identity.evidence.public_key_spki_der_base64, "base64"),
    type: "spki",
  });
  const signature = Buffer.from(identity.evidence.proof_base64, "base64");
  assert.equal(verify(null, bytes, key, signature), true);
  const changedProofOnly = {
    ...clone(identity.evidence),
    proof_base64: "changed-but-excluded",
  };
  assert.deepEqual(
    colimaLiveProviderEffectProofBytes({
      ...input,
      evidence: changedProofOnly,
    }),
    bytes,
  );
  for (const changed of [
    { domain: "endpoint" },
    { eventSequence: identity.event_sequence + 1 },
    { parentSha256: digest("different-parent") },
    { publisherAuthoritySha256: digest("different-publisher") },
  ]) {
    assert.equal(
      verify(
        null,
        colimaLiveProviderEffectProofBytes({ ...input, ...changed }),
        key,
        signature,
      ),
      false,
    );
  }
  for (const domain of ["endpoint", "quiescence-ack", "role-identity"]) {
    assert.doesNotThrow(() =>
      colimaLiveProviderEffectProofBytes({ ...input, domain }),
    );
  }
  for (const eventSequence of [0, 767]) {
    assert.doesNotThrow(() =>
      colimaLiveProviderEffectProofBytes({ ...input, eventSequence }),
    );
  }
  for (const changed of [
    { domain: "unknown" },
    { eventSequence: -1 },
    { eventSequence: 768 },
    { parentSha256: ZERO_SHA256 },
    { publisherAuthoritySha256: ZERO_SHA256 },
  ]) {
    expectRefusal(() =>
      colimaLiveProviderEffectProofBytes({ ...input, ...changed }),
    );
  }
  const extra = { ...input, unexpected: true };
  const missing = { ...input };
  delete missing.domain;
  expectRefusal(() => colimaLiveProviderEffectProofBytes(extra));
  expectRefusal(() => colimaLiveProviderEffectProofBytes(missing));
});

test("uncertainty reasons are derived from the exact published frontier", () => {
  const fixture = prepared("uncertain");
  const attempt = fixture.events.find(
    (event) => event.event_kind === "start-attempt",
  );
  const acceptedDeliveryIndex = eventIndex(fixture, "delivery-result");
  const acceptedDelivery = fixture.events[acceptedDeliveryIndex];
  const baseUncertainEvidence = fixture.events.at(-1).evidence;

  const identityPrefix = fixture.events.slice(0, acceptedDeliveryIndex + 1);
  const identityIncomplete = buildColimaLiveProviderEffectEvent({
    eventKind: "uncertain-start",
    evidence: {
      ...clone(baseUncertainEvidence),
      delivery_result_sha256: digest(acceptedDelivery),
      reason: "identity-incomplete",
      start_attempt_sha256: digest(attempt),
    },
    history: identityPrefix,
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: fixture.publisherAuthorityChain,
    publisherAuthoritySha256: fixture.publisherAuthoritySha256,
    source: fixture.source,
    witness: fixture.witness,
  });
  assert.equal(identityIncomplete.evidence.reason, "identity-incomplete");

  const deliveryPrefix = fixture.events.slice(0, acceptedDeliveryIndex);
  const indeterminateDelivery = buildColimaLiveProviderEffectEvent({
    eventKind: "delivery-result",
    evidence: {
      ...clone(acceptedDelivery.evidence),
      child_handle_identity_sha256: ZERO_SHA256,
      delivery_disposition: "indeterminate-effect-possible",
      observation_sha256: digest("indeterminate-delivery-observation"),
      safe_error_code: "indeterminate",
    },
    history: deliveryPrefix,
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: fixture.publisherAuthorityChain,
    publisherAuthoritySha256: fixture.publisherAuthoritySha256,
    source: fixture.source,
    witness: fixture.witness,
  });
  const deliveryIndeterminate = buildColimaLiveProviderEffectEvent({
    eventKind: "uncertain-start",
    evidence: {
      ...clone(baseUncertainEvidence),
      delivery_result_sha256: digest(indeterminateDelivery),
      reason: "delivery-indeterminate",
      start_attempt_sha256: digest(attempt),
    },
    history: [...deliveryPrefix, indeterminateDelivery],
    publicationPlan: fixture.publicationPlan,
    publisherAuthorityChain: fixture.publisherAuthorityChain,
    publisherAuthoritySha256: fixture.publisherAuthoritySha256,
    source: fixture.source,
    witness: fixture.witness,
  });
  assert.equal(
    deliveryIndeterminate.evidence.reason,
    "delivery-indeterminate",
  );

  assert.equal(baseUncertainEvidence.reason, "endpoint-incomplete");
  const retired = prepared("retired");
  const fenceIndex = eventIndex(retired, "quiescence-fence");
  const topologyPrefix = retired.events.slice(0, fenceIndex);
  const topologyAttempt = topologyPrefix.find(
    (event) => event.event_kind === "start-attempt",
  );
  const topologyDelivery = topologyPrefix.find(
    (event) => event.event_kind === "delivery-result",
  );
  const topologyIncomplete = buildColimaLiveProviderEffectEvent({
    eventKind: "uncertain-start",
    evidence: {
      ...clone(baseUncertainEvidence),
      delivery_result_sha256: digest(topologyDelivery),
      reason: "topology-incomplete",
      start_attempt_sha256: digest(topologyAttempt),
    },
    history: topologyPrefix,
    publicationPlan: retired.publicationPlan,
    publisherAuthorityChain: retired.publisherAuthorityChain,
    publisherAuthoritySha256: retired.publisherAuthoritySha256,
    source: retired.source,
    witness: retired.witness,
  });
  assert.equal(topologyIncomplete.evidence.reason, "topology-incomplete");

  for (const [event, history, wrongReason] of [
    [identityIncomplete, identityPrefix, "endpoint-incomplete"],
    [deliveryIndeterminate, [...deliveryPrefix, indeterminateDelivery], "identity-incomplete"],
    [fixture.events.at(-1), fixture.events.slice(0, -1), "topology-incomplete"],
    [topologyIncomplete, topologyPrefix, "endpoint-incomplete"],
  ]) {
    const changed = clone(event);
    changed.evidence.reason = wrongReason;
    expectRefusal(() =>
      validateColimaLiveProviderEffectEvent(
        changed,
        eventValidationOptions(
          event === topologyIncomplete ? retired : fixture,
          history,
        ),
      ),
    );
  }
});

test("nested causal and terminal binding fields fail at their own event", () => {
  const fixture = prepared("rich-retired");
  const cases = [
    ["start-authority", ["first_reproof", "admission_sha256"]],
    ["start-authority", ["first_reproof", "marker_witness_identity_sha256"]],
    ["start-attempt", ["second_reproof", "observation_challenge_sha256"]],
    ["quiescence-fence", ["process_reconciliation", "observed_node_set_sha256"]],
    ["recursive-inventory-set", ["samples", 0, "first_sample_sha256"]],
    ["cleanup-progress", ["outcomes", 0, "action_sha256"]],
    ["cleanup-progress", ["outcomes", 0, "observation_binding_sha256"]],
    ["terminal-receipt", ["receipt", "result", "operation_plan_sha256"]],
    ["terminal-receipt", ["receipt", "result", "publisher_authority_sha256"]],
  ];
  for (const [kind, path] of cases) {
    const index = eventIndex(fixture, kind);
    const changed = clone(fixture.events[index]);
    let parent = changed.evidence;
    for (const component of path.slice(0, -1)) parent = parent[component];
    const leaf = path.at(-1);
    parent[leaf] = changedValue(parent[leaf]);
    expectRefusal(() => validateAt(fixture, index, changed));
  }
  const liveSettlement = fixture.events.find(
    (event) => event.event_kind === "create-settlement",
  );
  assert.equal(liveSettlement.evidence.variant, "authenticated-live-identity");
  const residualSettlement = prepared("residual").events.find(
    (event) => event.event_kind === "create-settlement",
  );
  assert.equal(residualSettlement.evidence.variant, "exact-attempted-residual");
});

test("terminal receipt fields remain source-derived after digest recomputation", () => {
  const fixture = prepared("rich-retired");
  const index = eventIndex(fixture, "terminal-receipt");
  const original = fixture.events[index];
  const receiptPaths = [
    ...Object.keys(original.evidence.receipt)
      .filter((field) => field !== "result")
      .map((field) => [field]),
    ...Object.keys(original.evidence.receipt.result).map((field) => [
      "result",
      field,
    ]),
  ];
  for (const path of receiptPaths) {
    const changed = clone(original.evidence);
    let parent = changed.receipt;
    for (const component of path.slice(0, -1)) parent = parent[component];
    const leaf = path.at(-1);
    parent[leaf] = changedValue(parent[leaf]);
    changed.receipt_sha256 = digest(changed.receipt);
    expectRefusal(
      () =>
        buildEvent(
          fixture,
          fixture.events.slice(0, index),
          original,
          changed,
        ),
      /terminal receipt/u,
    );
  }
});

test("paged event prefixes refuse short nonterminal pages", () => {
  const fixture = prepared("rich-retired");

  const inventoryPages = fixture.events.filter(
    (event) =>
      event.event_kind === "recursive-inventory-page" &&
      event.evidence.sample_sequence === 0 &&
      event.evidence.namespace_role === "colima-home-namespace",
  );
  assert.ok(inventoryPages.length > 1);
  const inventoryIndex = fixture.events.indexOf(inventoryPages[0]);
  const inventoryHistory = fixture.events.slice(0, inventoryIndex);
  const duplicateRootPage = clone(inventoryPages[1].evidence);
  duplicateRootPage.entries[0] = clone(
    inventoryPages[0].evidence.entries[0],
  );
  duplicateRootPage.entries[0].entry_sequence =
    duplicateRootPage.entry_start;
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, fixture.events.indexOf(inventoryPages[1])),
        inventoryPages[1],
        duplicateRootPage,
      ),
    /inventory root was refused/u,
  );
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        inventoryHistory,
        inventoryPages[0],
        {
          ...clone(inventoryPages[0].evidence),
          entries: clone(inventoryPages[0].evidence.entries.slice(0, 1)),
        },
      ),
    /directory inventory was incomplete/u,
  );
  const firstInventoryPage = fixture.events.find(
    (event) => event.event_kind === "recursive-inventory-page",
  );
  const firstInventoryPageIndex = fixture.events.indexOf(firstInventoryPage);
  const invalidContinuation = clone(firstInventoryPage.evidence);
  invalidContinuation.entry_start = firstInventoryPage.evidence.entries.length;
  invalidContinuation.page_sequence = 1;
  invalidContinuation.parent_page_sha256 = digest(firstInventoryPage);
  invalidContinuation.entries[0].entry_sequence =
    invalidContinuation.entry_start;
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, firstInventoryPageIndex + 1),
        firstInventoryPage,
        invalidContinuation,
      ),
    /inventory page was refused/u,
  );
  const incompleteTerminalPage = clone(firstInventoryPage.evidence);
  incompleteTerminalPage.entries[0].directory_entry_count += 1;
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, firstInventoryPageIndex),
        firstInventoryPage,
        incompleteTerminalPage,
      ),
    /directory inventory was incomplete/u,
  );

  const impossibleFullPage = clone(inventoryPages[0].evidence);
  const impossibleDirectories = impossibleFullPage.entries
    .filter(
      (entry) =>
        entry.depth > 0 &&
        entry.ownership === "generation-owned" &&
        entry.type === "regular" &&
        entry.links === "1",
    )
    .slice(0, 3);
  assert.equal(impossibleDirectories.length, 3);
  for (const entry of impossibleDirectories) {
    entry.content_disposition = "metadata-only";
    entry.content_sha256 = ZERO_SHA256;
    entry.directory_entry_count = 512;
    entry.endpoint_sha256 = ZERO_SHA256;
    entry.hard_link_group_sha256 = ZERO_SHA256;
    entry.links = "1";
    entry.resource_kind = "none";
    entry.size = "0";
    entry.symlink_target_sha256 = ZERO_SHA256;
    entry.type = "directory";
  }
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        inventoryHistory,
        inventoryPages[0],
        impossibleFullPage,
      ),
    /inventory completion was impossible/u,
  );

  const firstRepeatedPage = fixture.events.find(
    (event) =>
      event.event_kind === "recursive-inventory-page" &&
      event.evidence.sample_sequence === 1 &&
      event.evidence.namespace_role === "colima-home-namespace",
  );
  const firstRepeatedPageIndex = fixture.events.indexOf(firstRepeatedPage);
  const changedRepeatedPage = clone(firstRepeatedPage.evidence);
  const changedRepeatedEntry = changedRepeatedPage.entries.find(
    (entry) =>
      entry.content_disposition === "hashed" &&
      entry.resource_kind === "none" &&
      entry.links === "1",
  );
  assert.notEqual(changedRepeatedEntry, undefined);
  changedRepeatedEntry.content_sha256 = digest(
    "changed-full-second-sample-prefix",
  );
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        fixture.events.slice(0, firstRepeatedPageIndex),
        firstRepeatedPage,
        changedRepeatedPage,
      ),
    /inventory samples differed/u,
  );

  const planPages = fixture.events.filter(
    (event) => event.event_kind === "cleanup-plan-page",
  );
  assert.ok(planPages.length > 1);
  const planIndex = fixture.events.indexOf(planPages[0]);
  const planHistory = fixture.events.slice(0, planIndex);
  const alteredPlanPage = clone(planPages[0].evidence);
  alteredPlanPage.actions[0].target_identity_sha256 = "a".repeat(64);
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        planHistory,
        planPages[0],
        alteredPlanPage,
      ),
    /cleanup plan page was refused/u,
  );
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        planHistory,
        planPages[0],
        {
          ...clone(planPages[0].evidence),
          actions: clone(planPages[0].evidence.actions.slice(0, 1)),
        },
      ),
    /cleanup plan page was refused/u,
  );

  const progress = fixture.events.filter(
    (event) => event.event_kind === "cleanup-progress",
  );
  assert.ok(progress.length > 1);
  const progressIndex = fixture.events.indexOf(progress[0]);
  const progressHistory = fixture.events.slice(0, progressIndex);
  expectRefusal(
    () =>
      buildEvent(
        fixture,
        progressHistory,
        progress[0],
        {
          ...clone(progress[0].evidence),
          outcomes: clone(progress[0].evidence.outcomes.slice(0, 1)),
        },
      ),
    /cleanup progress was refused/u,
  );
});

test("generation capacity charges directory scaffolding", () => {
  const minimum = (slotGroups, requiredLeaves) =>
    minimumGenerationForestEntries({
      branchingFactor: 512,
      maximumDirectoryNodes: 4_096,
      requiredLeaves,
      slotGroups,
    });
  assert.equal(minimum([{ count: 1, remaining_depth: 32 }], 76), 77);
  assert.equal(3_996 + 24 + 76, 4_096);
  assert.equal(3_996 + 24 + minimum([
    { count: 1, remaining_depth: 32 },
  ], 76), 4_097);
  assert.equal(minimum([{ count: 512, remaining_depth: 32 }], 1_533), 1_535);
  assert.equal(minimum([{ count: 512, remaining_depth: 32 }], 1_534), 1_536);
  assert.equal(
    minimum([{ count: 1, remaining_depth: 1 }], 2),
    Number.POSITIVE_INFINITY,
  );
  assert.throws(
    () => minimumGenerationForestEntries({
      branchingFactor: 1,
      maximumDirectoryNodes: 4_096,
      requiredLeaves: 1,
      slotGroups: [{ count: 1, remaining_depth: 1 }],
    }),
    /capacity arguments were refused/u,
  );
});

test("cleanup is derived, paged, globally fresh and receipt-bound", () => {
  const fixture = prepared("rich-retired");
  const actions = cleanupActions(fixture);
  const planPages = fixture.events.filter(
    (event) => event.event_kind === "cleanup-plan-page",
  );
  const progress = fixture.events.filter(
    (event) => event.event_kind === "cleanup-progress",
  );
  assert.equal(actions.length, 92);
  assert.deepEqual(
    planPages.map((page) => page.evidence.actions.length),
    [64, 28],
  );
  assert.deepEqual(
    progress.map((event) => event.evidence.outcomes.length),
    [24, 24, 24, 20],
  );
  assert.deepEqual(
    actions.slice(0, 10).map((action) => action.action_kind),
    [
      "withdraw-engine-context",
      "request-engine-shutdown",
      "request-ssh-controlmaster-shutdown",
      "request-usernet-shutdown",
      "request-hostagent-shutdown",
      "request-outer-shutdown",
      "prove-process-subtree-absent",
      "prove-process-subtree-absent",
      "prove-process-subtree-absent",
      "prove-process-subtree-absent",
    ],
  );
  assert.equal(
    actions.filter((action) => action.action_kind === "retire-socket-evidence")
      .length,
    COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length,
  );
  assert.equal(
    actions.filter((action) => action.action_kind === "prove-namespace-baseline")
      .length,
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.length,
  );
  for (const [index, action] of actions.entries()) {
    assert.equal(action.action_sequence, index);
    assert.equal(
      action.prerequisite_sha256,
      index === 0
        ? digest(
            fixture.events.find(
              (event) => event.event_kind === "create-settlement",
            ),
          )
        : digest(actions[index - 1]),
    );
  }
  const inventoryEntries = fixture.events
    .filter(
      (event) =>
        event.event_kind === "recursive-inventory-page" &&
        event.evidence.sample_sequence === 0,
    )
    .flatMap((event) => event.evidence.entries);
  const generationTargets = inventoryEntries
    .filter((entry) => entry.ownership === "generation-owned")
    .map((entry) => entry.relative_identity_hmac_sha256)
    .sort();
  const filesystemTargets = actions
    .filter((action) => action.target_class === "filesystem")
    .map((action) => action.target_identity_sha256)
    .sort();
  assert.deepEqual(filesystemTargets, generationTargets);
  const baselineTargets = new Set(
    inventoryEntries
      .filter((entry) => entry.ownership === "admitted-baseline")
      .map((entry) => entry.relative_identity_hmac_sha256),
  );
  assert.equal(
    filesystemTargets.some((target) => baselineTargets.has(target)),
    false,
  );
  const filesystemActions = actions.filter(
    (action) => action.target_class === "filesystem",
  );
  for (let index = 1; index < filesystemActions.length; index += 1) {
    const prior = filesystemActions[index - 1];
    const current = filesystemActions[index];
    assert.ok(prior.depth >= current.depth);
    if (prior.depth === current.depth && prior.action_kind === "retire-directory") {
      assert.equal(current.action_kind, "retire-directory");
    }
  }
  assert.doesNotThrow(() => rebuildCleanupPlan(fixture, () => undefined));
  expectRefusal(
    () =>
      rebuildCleanupPlan(fixture, (candidateActions) => {
        candidateActions[0].target_identity_sha256 = digest(
          "altered-cleanup-target",
        );
      }),
    /cleanup plan(?: page)? was refused/u,
  );

  const observationValues = [];
  for (const event of progress) {
    for (const outcome of event.evidence.outcomes) {
      observationValues.push(
        outcome.marker_witness_observation_sha256,
        outcome.observation_challenge_sha256,
        outcome.outcome_observation_sha256,
        outcome.precondition_observation_sha256,
        outcome.recursive_prefix_observation_sha256,
        outcome.source_reproof_observation_sha256,
      );
      if (outcome.parent_fsync_observation_sha256 !== ZERO_SHA256) {
        observationValues.push(outcome.parent_fsync_observation_sha256);
      }
    }
  }
  const settlement = fixture.events.find(
    (event) => event.event_kind === "cleanup-settlement",
  );
  observationValues.push(
    settlement.evidence.endpoint_absence_observation_sha256,
    settlement.evidence.marker_witness_topology_observation_sha256,
    settlement.evidence.role_absence_observation_sha256,
    settlement.evidence.source_reproof_observation_sha256,
  );
  assert.equal(new Set(observationValues).size, observationValues.length);

  const firstProgressIndex = eventIndex(fixture, "cleanup-progress");
  const samePageReplay = clone(fixture.events[firstProgressIndex]);
  samePageReplay.evidence.outcomes[1].observation_challenge_sha256 =
    samePageReplay.evidence.outcomes[0].outcome_observation_sha256;
  samePageReplay.evidence.outcomes[1].observation_binding_sha256 =
    cleanupObservationBinding(
      fixture,
      samePageReplay,
      samePageReplay.evidence.outcomes[1],
    );
  expectRefusal(
    () => validateAt(fixture, firstProgressIndex, samePageReplay),
    /cleanup outcome/u,
  );
  const fsyncReplay = clone(fixture.events[firstProgressIndex]);
  const fsyncOutcome = fsyncReplay.evidence.outcomes.find(
    (outcome) => outcome.parent_fsync_observation_sha256 !== ZERO_SHA256,
  );
  fsyncOutcome.parent_fsync_observation_sha256 =
    fsyncOutcome.observation_challenge_sha256;
  fsyncOutcome.observation_binding_sha256 = cleanupObservationBinding(
    fixture,
    fsyncReplay,
    fsyncOutcome,
  );
  expectRefusal(
    () => validateAt(fixture, firstProgressIndex, fsyncReplay),
    /cleanup outcome/u,
  );
  const crossPhaseReplay = clone(fixture.events[firstProgressIndex]);
  crossPhaseReplay.evidence.outcomes[0].observation_challenge_sha256 =
    fixture.events.find((event) => event.event_kind === "endpoint").evidence
      .challenge_sha256;
  crossPhaseReplay.evidence.outcomes[0].observation_binding_sha256 =
    cleanupObservationBinding(
      fixture,
      crossPhaseReplay,
      crossPhaseReplay.evidence.outcomes[0],
    );
  expectRefusal(
    () => validateAt(fixture, firstProgressIndex, crossPhaseReplay),
    /fresh evidence was reused/u,
  );
  const roleChallengeReplay = clone(fixture.events[firstProgressIndex]);
  roleChallengeReplay.evidence.outcomes[0].observation_challenge_sha256 =
    fixture.events.find((event) => event.event_kind === "role-identity")
      .evidence.challenge_sha256;
  roleChallengeReplay.evidence.outcomes[0].observation_binding_sha256 =
    cleanupObservationBinding(
      fixture,
      roleChallengeReplay,
      roleChallengeReplay.evidence.outcomes[0],
    );
  expectRefusal(
    () => validateAt(fixture, firstProgressIndex, roleChallengeReplay),
    /fresh evidence was reused/u,
  );
  const secondProgressIndex = fixture.events.findIndex(
    (event, index) =>
      event.event_kind === "cleanup-progress" && index > firstProgressIndex,
  );
  const crossPageReplay = clone(fixture.events[secondProgressIndex]);
  crossPageReplay.evidence.outcomes[0].observation_challenge_sha256 =
    progress[0].evidence.outcomes[0].observation_challenge_sha256;
  crossPageReplay.evidence.outcomes[0].observation_binding_sha256 =
    cleanupObservationBinding(
      fixture,
      crossPageReplay,
      crossPageReplay.evidence.outcomes[0],
    );
  expectRefusal(
    () => validateAt(fixture, secondProgressIndex, crossPageReplay),
    /cleanup outcome/u,
  );

  const completionIndex = eventIndex(fixture, "completion");
  const markerReplay = clone(fixture.events[completionIndex]);
  markerReplay.evidence.marker_retirement_observation_sha256 =
    settlement.evidence.source_reproof_observation_sha256;
  markerReplay.evidence.marker_retirement_binding_sha256 =
    markerRetirementBinding(fixture, markerReplay.evidence);
  expectRefusal(
    () => validateAt(fixture, completionIndex, markerReplay),
    /fresh evidence was reused/u,
  );
  const fsyncCompletionReplay = clone(fixture.events[completionIndex]);
  fsyncCompletionReplay.evidence.provider_root_fsync_observation_sha256 =
    settlement.evidence.endpoint_absence_observation_sha256;
  fsyncCompletionReplay.evidence.marker_retirement_binding_sha256 =
    markerRetirementBinding(fixture, fsyncCompletionReplay.evidence);
  expectRefusal(
    () => validateAt(fixture, completionIndex, fsyncCompletionReplay),
    /fresh evidence was reused/u,
  );
  const settlementIndex = eventIndex(fixture, "cleanup-settlement");
  const forgedFinalInventory = clone(fixture.events[settlementIndex]);
  forgedFinalInventory.evidence.namespace_baselines_sha256 = digest(
    "caller-selected-final-baseline",
  );
  forgedFinalInventory.evidence.final_inventory_sha256 =
    forgedFinalInventory.evidence.namespace_baselines_sha256;
  expectRefusal(
    () => validateAt(fixture, settlementIndex, forgedFinalInventory),
    /cleanup settlement/u,
  );
  const settlementReplay = clone(fixture.events[settlementIndex]);
  settlementReplay.evidence.endpoint_absence_observation_sha256 =
    fixture.events[0].evidence.first_reproof.observation_challenge_sha256;
  settlementReplay.evidence.endpoint_absence_sha256 = digest({
    disposition: "absent",
    endpoints: fixture.events
      .filter(
        (event) =>
          event.event_kind === "endpoint" &&
          event.evidence.phase === "post-detachment-final",
      )
      .map((event) => event.evidence.socket_identity_hmac_sha256),
    observation_sha256:
      settlementReplay.evidence.endpoint_absence_observation_sha256,
  });
  expectRefusal(
    () => validateAt(fixture, settlementIndex, settlementReplay),
    /fresh evidence was reused/u,
  );
  const receiptEvent = fixture.events.find(
    (event) => event.event_kind === "terminal-receipt",
  );
  assert.equal(
    fixture.events.at(-1).evidence.receipt_sha256,
    digest(receiptEvent.evidence.receipt),
  );
  assert.notEqual(
    fixture.events.at(-1).evidence.receipt_sha256,
    digest(receiptEvent.evidence.receipt.result),
  );
  assert.equal(
    receiptEvent.evidence.receipt.previous_sha256,
    fixture.publicationPlan.receipt_previous_sha256,
  );
  assert.equal(receiptEvent.evidence.marker_link_count, 2);
  assert.equal(receiptEvent.evidence.witness_link_count, 2);
  assert.equal(
    fixture.events.at(-1).evidence.receipt_event_sha256,
    digest(receiptEvent),
  );
  assert.equal(fixture.events.at(-1).evidence.marker_present, false);
  assert.equal(fixture.events.at(-1).evidence.witness_link_count, 1);
  const zeroReceipt = prepared("pre-attempt-retired").events.at(-1).evidence;
  assert.equal(zeroReceipt.receipt_event_sha256, ZERO_SHA256);
  assert.equal(zeroReceipt.receipt_sha256, ZERO_SHA256);
});

test("pure construction and validation perform no filesystem observation", () => {
  const observedMethods = [
    "closeSync",
    "fstatSync",
    "lstatSync",
    "openSync",
    "opendirSync",
    "readFileSync",
    "readlinkSync",
    "readSync",
    "realpathSync",
    "writeFileSync",
  ];
  const originals = new Map(observedMethods.map((name) => [name, fs[name]]));
  let attemptedMethod;
  try {
    for (const name of observedMethods) {
      fs[name] = () => {
        attemptedMethod = name;
        throw new Error(`unexpected filesystem observation through ${name}`);
      };
    }
    syncBuiltinESMExports();
    const fixture = cleanEngineLiveProviderEffectFixture({
      outcome: "pre-attempt-retired",
    });
    validateColimaLiveProviderEffectEventHistory(
      fixture.events,
      validationOptions(fixture),
    );
  } finally {
    for (const [name, implementation] of originals) {
      fs[name] = implementation;
    }
    syncBuiltinESMExports();
  }
  assert.equal(attemptedMethod, undefined);
});

test("the pure boundary has no executor imports and its joint maxima fit the event budget", () => {
  const moduleSource = readFileSync(MODULE_PATH, "utf8");
  const capacitySource = readFileSync(CAPACITY_PATH, "utf8");
  const fixtureSource = readFileSync(FIXTURE_PATH, "utf8");
  const moduleImports = [...moduleSource.matchAll(/from "([^"]+)";/gu)].map(
    (match) => match[1],
  );
  assert.deepEqual(moduleImports, [
    "node:crypto",
    "node:util/types",
    "./clean-engine-colima-live-schemas.mjs",
    "./clean-engine-live-provider-process-start.mjs",
    "./clean-engine-live-provider-effect-capacity.mjs",
  ]);
  const capacityImports = [
    ...capacitySource.matchAll(/from "([^"]+)";/gu),
  ].map((match) => match[1]);
  assert.deepEqual(capacityImports, []);
  const fixtureImports = [
    ...fixtureSource.matchAll(/from "([^"]+)";/gu),
  ].map((match) => match[1]);
  assert.deepEqual(fixtureImports, [
    "node:crypto",
    "node:util/types",
    "../../deploy/compose/scripts/clean-engine-live-provider-effect.mjs",
    "../../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs",
    "./clean-engine-live-provider-process-start-fixture.mjs",
  ]);
  for (const forbidden of [
    "node:child_process",
    "node:dgram",
    "node:fs",
    "node:http",
    "node:https",
    "node:net",
    "node:tls",
    "clean-engine-receipts",
    "clean-engine-state",
    "clean-engine-live-provider-effect-generation",
    // Assemble the sentinel so the repository-wide spike-isolation scanner can
    // inspect this guard without mistaking the guard itself for a dependency.
    ["cpr", "45", "four", "role"].join("-"),
  ]) {
    assert.equal(
      moduleImports.some((specifier) => specifier.includes(forbidden)),
      false,
      forbidden,
    );
    assert.equal(
      fixtureImports.some((specifier) => specifier.includes(forbidden)),
      false,
      forbidden,
    );
    assert.equal(
      capacityImports.some((specifier) => specifier.includes(forbidden)),
      false,
      forbidden,
    );
  }
  assert.doesNotMatch(
    `${moduleSource}\n${capacitySource}\n${fixtureSource}`,
    /node:(?:child_process|dgram|fs|http|https|net|tls|worker_threads)/u,
  );
  assert.deepEqual(Object.keys(effectFixture), [
    "cleanEngineLiveProviderEffectFixture",
  ]);
  assert.doesNotMatch(
    `${moduleSource}\n${capacitySource}\n${fixtureSource}`,
    /\b(?:exec|fork|kill|link|lstat|mkdir|open|readFile|rename|rmdir|spawn|unlink|writeFile)(?:Sync)?\s*\(/u,
  );

  const excluded = new Set([
    CAPACITY_PATH,
    MODULE_PATH,
    FIXTURE_PATH,
    TEST_PATH,
  ]);
  const authorityRoots = [
    "adapters",
    "console",
    "crates",
    "demos",
    "deploy",
    "policies",
    "scripts",
  ];
  const authorityPaths = authorityRoots
    .flatMap((root) => firstPartyFiles(resolve(REPO_ROOT, root)))
    .filter((path) => !excluded.has(path));
  const permittedConsumers = new Set([
    resolve(
      REPO_ROOT,
      "deploy/compose/scripts/clean-engine-receipts.mjs",
    ),
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
    resolve(REPO_ROOT, "scripts/clean-engine-receipts.test.mjs"),
    resolve(
      REPO_ROOT,
      "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
    ),
    resolve(REPO_ROOT, "scripts/clean-engine-state.test.mjs"),
  ]);
  const observedConsumers = [];
  assert.ok(authorityPaths.length > 100, "authority scan was unexpectedly small");
  for (const path of authorityPaths) {
    const bytes = readFileSync(path);
    if (bytes.includes(0)) continue;
    let consumer;
    try {
      consumer = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      continue;
    }
    if (/clean-engine-live-provider-effect\.mjs/u.test(consumer)) {
      assert.equal(
        permittedConsumers.has(path),
        true,
        relative(REPO_ROOT, path),
      );
      observedConsumers.push(relative(REPO_ROOT, path));
    }
  }
  assert.deepEqual(observedConsumers.sort(), [
    "deploy/compose/scripts/clean-engine-receipts.mjs",
    "deploy/compose/scripts/clean-engine-state.mjs",
    "scripts/clean-engine-live-provider-effect-fixture-blueprint.test.mjs",
    "scripts/clean-engine-receipts.test.mjs",
    "scripts/clean-engine-state.test.mjs",
  ]);

  const stateSource = readFileSync(
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-state.mjs"),
    "utf8",
  );
  const lifecycleSource = readFileSync(
    resolve(REPO_ROOT, "deploy/compose/scripts/clean-engine-acceptance.sh"),
    "utf8",
  );
  assert.doesNotMatch(
    stateSource,
    /COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_(?:CONTRACT|CONTRACT_SHA256|KIND)/u,
  );
  assert.doesNotMatch(
    stateSource,
    /export function (?:publish|recover)[A-Za-z0-9]*ProviderEffect[A-Za-z0-9]*ForExecutor/u,
  );
  assert.doesNotMatch(lifecycleSource, /provider-effect/u);

  const inventoryPagesPerSample = Math.floor(
    (COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries +
      (COLIMA_LIVE_MUTATION_SURFACE_ROLES.length *
        (COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries_per_page - 1))) /
      COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries_per_page,
  );
  const maximumEvents =
    24 +
    (2 * inventoryPagesPerSample) +
    1 +
    1 +
    Math.ceil(
      COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.cleanup_actions /
        COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.canonical_container_entries,
    ) +
    1 +
    Math.ceil(
      COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.cleanup_actions /
        COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.cleanup_outcomes_per_page,
    ) +
    1 +
    1 +
    1;
  assert.equal(inventoryPagesPerSample, 176);
  assert.equal(maximumEvents, 632);
  assert.ok(maximumEvents < COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.effect_events);

  const fixture = prepared("rich-retired");
  const started = performance.now();
  validateColimaLiveProviderEffectEventHistory(
    fixture.events,
    validationOptions(fixture),
  );
  const elapsed = performance.now() - started;
  assert.ok(elapsed < 2_000, `rich validation took ${elapsed} ms`);
  const sizes = fixture.events.map(
    (event) => liveProviderEffectBytes(event).byteLength,
  );
  assert.ok(
    Math.max(...sizes) < COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.canonical_bytes,
  );
});

#!/usr/bin/env node
import { createHash, createPublicKey, verify } from "node:crypto";
import { isProxy } from "node:util/types";
import {
  COLIMA_LIVE_BASELINE_DESCRIPTOR_BINDING_FIELDS,
  COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS,
  COLIMA_LIVE_MAX_BASELINE_DESCENDANTS,
  COLIMA_LIVE_MUTATION_SURFACE_ROLES,
  COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
} from "./clean-engine-colima-live-schemas.mjs";
import {
  COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA,
  LiveProviderProcessStartFailure,
  liveProviderProcessStartBytes,
  liveProviderProcessStartDigest,
  validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure,
} from "./clean-engine-live-provider-process-start.mjs";
import {
  minimumGenerationForestEntries,
} from "./clean-engine-live-provider-effect-capacity.mjs";

export const COLIMA_LIVE_PROVIDER_EFFECT_ACTION = "provider-effect";
export const COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_KIND =
  "colima-live-provider-effect-v1";
export const COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND =
  "colima-live-fixture-provider-effect-v1";
export const COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION =
  "mutation-journal-v7-sibling-effect-only";

const ZERO_SHA256 = "0".repeat(64);
const RECEIPT_BINDING_SCHEMA =
  "synveda.clean-engine.provider-effect-receipt-binding.v1";
const TERMINAL_RECEIPT_SCHEMA = "synveda.clean-engine.receipt.v6";
const MAX_CANONICAL_DEPTH = 16;
const MAX_CANONICAL_NODES = 1_024;
const MAX_CANONICAL_CONTAINER_ENTRIES = 64;
const MAX_CANONICAL_BYTES = 64 * 1024;
// This bound admits two full inventory samples plus paged cleanup plan and
// progress evidence. Keeping the lower 256 bound would make the declared
// inventory and cleanup limits impossible to represent.
const MAX_EFFECT_EVENTS = 768;
const MAX_INVENTORY_ENTRIES_PER_PAGE = 24;
const MAX_INVENTORY_ENTRIES = 4_096;
const MAX_INVENTORY_PAGES_PER_NAMESPACE = 64;
const MAX_RECURSIVE_DEPTH = 32;
const MAX_DIRECTORY_ENTRIES = 512;
const MAX_ENTRY_NAME_BYTES = 255;
// Fixture v1 closes exactly the four authenticated Colima spine roles. Any
// additional or reparented process is blocking evidence, not an unmodelled
// descendant that can be silently admitted.
const MAX_CAUSAL_NODES = 4;
const MAX_CAUSAL_EDGES = 4;
const MAX_CAUSAL_DEPTH = 2;
const MAX_ENDPOINT_CHALLENGES = 32;
const MAX_ENDPOINT_FRAME_BYTES = 4 * 1024;
const MAX_SYMLINK_TARGET_BYTES = 4 * 1024;
const MAX_HASHED_FILE_BYTES = 16 * 1024 * 1024;
const MAX_TOTAL_HASHED_BYTES = 256 * 1024 * 1024;
const MAX_PROVIDER_RESOURCE_BYTES = 64 * 1024 * 1024 * 1024;
const MAX_EFFECT_MILLISECONDS = 900_000;
const MAX_CLEANUP_ACTIONS = 4_352;
const MAX_CLEANUP_OUTCOMES_PER_PAGE = 24;
const MAX_PUBLISHER_AUTHORITIES = 64;

export const COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS = Object.freeze({
  canonical_bytes: MAX_CANONICAL_BYTES,
  canonical_container_entries: MAX_CANONICAL_CONTAINER_ENTRIES,
  canonical_depth: MAX_CANONICAL_DEPTH,
  canonical_value_occurrences: MAX_CANONICAL_NODES,
  causal_depth: MAX_CAUSAL_DEPTH,
  causal_edges: MAX_CAUSAL_EDGES,
  causal_nodes: MAX_CAUSAL_NODES,
  cleanup_actions: MAX_CLEANUP_ACTIONS,
  cleanup_outcomes_per_page: MAX_CLEANUP_OUTCOMES_PER_PAGE,
  directory_entries: MAX_DIRECTORY_ENTRIES,
  effect_events: MAX_EFFECT_EVENTS,
  effect_milliseconds: MAX_EFFECT_MILLISECONDS,
  endpoint_challenges: MAX_ENDPOINT_CHALLENGES,
  endpoint_frame_bytes: MAX_ENDPOINT_FRAME_BYTES,
  entry_name_bytes: MAX_ENTRY_NAME_BYTES,
  hashed_file_bytes: MAX_HASHED_FILE_BYTES,
  inventory_entries: MAX_INVENTORY_ENTRIES,
  inventory_entries_per_page: MAX_INVENTORY_ENTRIES_PER_PAGE,
  inventory_pages_per_namespace: MAX_INVENTORY_PAGES_PER_NAMESPACE,
  provider_resource_bytes: MAX_PROVIDER_RESOURCE_BYTES,
  publisher_authorities: MAX_PUBLISHER_AUTHORITIES,
  recursive_depth: MAX_RECURSIVE_DEPTH,
  symlink_target_bytes: MAX_SYMLINK_TARGET_BYTES,
  total_hashed_bytes: MAX_TOTAL_HASHED_BYTES,
});

export const COLIMA_LIVE_PROVIDER_EFFECT_ROLES = Object.freeze([
  "outer",
  "hostagent",
  "usernet",
  "ssh-controlmaster",
]);

export const COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS = Object.freeze([
  "hostagent-control",
  "usernet-control",
  "ssh-control",
  "engine-api",
]);

export const COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS = Object.freeze([
  "withdraw-engine-context",
  "request-engine-shutdown",
  "request-ssh-controlmaster-shutdown",
  "request-usernet-shutdown",
  "request-hostagent-shutdown",
  "request-outer-shutdown",
  "prove-process-subtree-absent",
  "retire-socket-evidence",
  "retire-file",
  "retire-directory",
  "prove-namespace-baseline",
]);

export const COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS = Object.freeze([
  "start-authority",
  "start-attempt",
  "delivery-result",
  "launch-edge",
  "role-identity",
  "endpoint",
  "quiescence-fence",
  "quiescence-ack",
  "recursive-inventory-page",
  "recursive-inventory-set",
  "create-settlement",
  "uncertain-start",
  "cleanup-plan-page",
  "cleanup-plan",
  "cleanup-progress",
  "cleanup-settlement",
  "terminal-receipt",
  "completion",
]);

function schemaMap(prefix) {
  return Object.freeze({
    contract: `${prefix}-operation-contract.v1`,
    event: Object.freeze(
      Object.fromEntries(
        COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS.map((kind) => [
          kind,
          `${prefix}-${kind}.v1`,
        ]),
      ),
    ),
    plan: `${prefix}-publication-plan.v1`,
    receipt_binding: RECEIPT_BINDING_SCHEMA,
    terminal_receipt: TERMINAL_RECEIPT_SCHEMA,
    witness: `${prefix}-witness.v1`,
  });
}

export const COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS = schemaMap(
  "synveda.clean-engine.colima-live-provider-effect",
);
export const COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS = schemaMap(
  "synveda.clean-engine.colima-live-fixture-provider-effect",
);

const IDENTITY_FIELDS = Object.freeze(["device", "inode", "mode", "uid"]);
const PHYSICAL_NAMESPACE_BINDING_FIELDS = Object.freeze([
  ...IDENTITY_FIELDS,
  "role",
]);
const NAMESPACE_BINDING_FIELDS = Object.freeze([
  ...IDENTITY_FIELDS,
  "baseline_descriptor_set_hmac_sha256",
  "baseline_descriptors",
  "baseline_relative_identity_hmac_sha256",
  "baseline_set_binding_sha256",
  "namespace_identity_hmac_sha256",
  "observed_entry_set_hmac_sha256",
  "role",
]);
const INVOCATION_BINDING_FIELDS = Object.freeze([
  "argv_sha256",
  "cwd_identity_sha256",
  "environment_sha256",
  "executable_sha256",
  "toolchain_sha256",
]);
const ROLE_CONTRACT_FIELDS = Object.freeze([
  "argv_sha256",
  "cwd_identity_sha256",
  "depth",
  "environment_sha256",
  "executable_sha256",
  "parent_role",
  "public_key_spki_sha256",
  "role",
  "role_challenge_sha256",
  "toolchain_sha256",
  "uid",
]);
const ENDPOINT_CONTRACT_FIELDS = Object.freeze([
  "docker_context_namespace_role",
  "docker_context_path_identity_hmac_sha256",
  "docker_context_sha256",
  "endpoint_kind",
  "final_challenge_commitment_sha256",
  "initial_challenge_commitment_sha256",
  "namespace_role",
  "path_identity_hmac_sha256",
  "role",
]);
const CAPABILITY_FIELDS = Object.freeze([
  "cleanup_authorized",
  "effect_witness_publication_authorized",
  "environment_publication_authorized",
  "evidence_publication_authorized",
  "finalization_authorized",
  "fixture_effect_execution_authorized",
  "fixture_effect_recovery_authorized",
  "lifecycle_exposed",
  "marker_link_authorized",
  "marker_retirement_authorized",
  "process_group_ownership_authorized",
  "process_signal_authorized",
  "process_spawn_authorized",
  "process_start_authorized",
  "provider_adapter_execution_authorized",
  "provider_artifact_publication_authorized",
  "provider_recovery_authorized",
  "provider_root_mutation_authorized",
  "receipt_publication_authorized",
  "runtime_publication_authorized",
  "start_attempt_publication_authorized",
  "start_authority_publication_authorized",
  "state_effect_slot_publication_authorized",
  "state_evidence_publication_authorized",
]);
const CONTRACT_FIELDS = Object.freeze([
  "action",
  "authority_model",
  "bounds",
  "capabilities",
  "endpoints",
  "evidence_class",
  "fixed_marker_name",
  "operation_kind",
  "receipt_binding_schema",
  "roles",
  "schema",
  "state_integration",
  "terminal_receipt_schema",
  "threat_boundary",
]);
const PLAN_FIELDS = Object.freeze([
  "admission",
  "admission_sha256",
  "branch",
  "completed_start_decision_projection_sha256",
  "driver_contract_sha256",
  "evidence_class",
  "fixed_marker_name",
  "fixture_id",
  "invocation_binding",
  "namespace_bindings",
  "operation_contract_sha256",
  "operation_kind",
  "planned_endpoint_contracts",
  "planned_quiescence_fence_sha256",
  "planned_role_contracts",
  "planned_start_attempt_sha256",
  "preparation_observation_sha256",
  "provider_root_identity",
  "receipt_previous_sha256",
  "receipt_sequence",
  "schema",
  "state_integration",
  "state_run_identity",
]);
const SLOT_BINDING_FIELDS = Object.freeze([
  "evidence_class",
  "fixture_id",
  "operation_contract_sha256",
  "operation_kind",
  "operation_plan_sha256",
  "slot_sequence",
  "slot_sha256",
  "state_integration",
]);
const WITNESS_FIELDS = Object.freeze([
  ...SLOT_BINDING_FIELDS,
  "fixed_marker_name",
  "invocation_binding_sha256",
  "marker_link_count",
  "marker_witness_identity_sha256",
  "namespace_bindings_sha256",
  "planned_endpoint_contracts_sha256",
  "planned_quiescence_fence_sha256",
  "planned_role_contracts_sha256",
  "planned_start_attempt_sha256",
  "process_attempt",
  "reservation_disposition",
  "schema",
]);
const EVENT_FIELDS = Object.freeze([
  ...SLOT_BINDING_FIELDS,
  "event_kind",
  "event_sequence",
  "evidence",
  "parent_sha256",
  "publisher_authority_sha256",
  "schema",
]);
const PUBLISHER_AUTHORITY_FIELDS = Object.freeze([
  "authority_sha256",
  "first_event_sequence",
  "kind",
  "prior_authority_sha256",
  "recovery_claim_sha256",
]);
const START_AUTHORITY_FIELDS = Object.freeze([
  "authority_scope",
  "current_owner_boot_sha256",
  "current_owner_instance_sha256",
  "first_reproof",
  "first_reproof_sha256",
  "recovery_invocation_authorized",
  "serialized_invocation_authorized",
  "witness_sha256",
]);
const START_REPROOF_FIELDS = Object.freeze([
  "admission_sha256",
  "invocation_sha256",
  "marker_link_count",
  "marker_present",
  "marker_witness_identity_sha256",
  "marker_witness_same_inode",
  "namespace_sha256",
  "observation_challenge_sha256",
  "observation_sequence",
  "provider_root_identity_sha256",
  "source_sha256",
  "state_run_identity_sha256",
  "topology_sha256",
  "witness_link_count",
]);
const START_ATTEMPT_FIELDS = Object.freeze([
  "authority_event_sha256",
  "attempt_sha256",
  "invocation_binding_sha256",
  "invocation_limit",
  "recovery_invocation_authorized",
  "replay_authorized",
  "second_reproof",
  "second_reproof_sha256",
  "variant",
]);
const DELIVERY_RESULT_FIELDS = Object.freeze([
  "attempt_event_sha256",
  "child_handle_identity_sha256",
  "delivery_disposition",
  "driver_contract_sha256",
  "effect_possible",
  "observation_sha256",
  "outer_launch_edge_sha256",
  "safe_error_code",
]);
const LAUNCH_EDGE_FIELDS = Object.freeze([
  "child_node_identity_sha256",
  "child_public_key_spki_der_base64",
  "child_role",
  "depth",
  "edge_sequence",
  "parent_node_identity_sha256",
  "start_attempt_sha256",
]);
const ROLE_IDENTITY_FIELDS = Object.freeze([
  "argv_sha256",
  "boot_identity_sha256",
  "challenge_sha256",
  "cwd_identity_sha256",
  "detachment",
  "detachment_observation_sha256",
  "environment_sha256",
  "executable_sha256",
  "launch_edge_sha256",
  "node_identity_sha256",
  "parent_node_identity_sha256",
  "parent_after_detachment_sha256",
  "parent_before_detachment_sha256",
  "pgid_identity_sha256",
  "pid_identity_sha256",
  "proof_base64",
  "public_key_spki_der_base64",
  "role",
  "session_identity_sha256",
  "start_identity_sha256",
  "toolchain_sha256",
  "uid",
]);
const ENDPOINT_FIELDS = Object.freeze([
  "activation_authorized",
  "ambient_context_authorized",
  "challenge_sha256",
  "connection_observation_sha256",
  "docker_context_binding_sha256",
  "docker_context_namespace_role",
  "docker_context_path_identity_hmac_sha256",
  "docker_context_sha256",
  "endpoint_kind",
  "hostagent_detachment_observation_sha256",
  "initial_endpoint_set_sha256",
  "namespace_role",
  "path_identity_hmac_sha256",
  "phase",
  "planned_quiescence_fence_sha256",
  "private_engine_socket_identity_hmac_sha256",
  "prior_endpoint_event_sha256",
  "proof_base64",
  "role_identity_sha256",
  "socket_identity_hmac_sha256",
]);
const QUIESCENCE_FENCE_FIELDS = Object.freeze([
  "authority_event_sha256",
  "deadline_milliseconds",
  "fence_sha256",
  "graph_frontier_sha256",
  "process_reconciliation",
  "process_reconciliation_sha256",
  "start_attempt_event_sha256",
]);
const PROCESS_RECONCILIATION_FIELDS = Object.freeze([
  "observation_challenge_sha256",
  "observed_edge_set_sha256",
  "observed_node_set_sha256",
  "observed_process_count",
  "process_observation_sha256",
]);
const QUIESCENCE_ACK_FIELDS = Object.freeze([
  "acknowledgement_sha256",
  "disposition",
  "endpoint_frontier_sha256",
  "fence_event_sha256",
  "outgoing_edge_set_sha256",
  "proof_base64",
  "resource_frontier_observation_sha256",
  "role",
  "role_identity_sha256",
]);
const INVENTORY_ENTRY_FIELDS = Object.freeze([
  "content_disposition",
  "content_sha256",
  "ctime_nanoseconds",
  "depth",
  "device",
  "directory_entry_count",
  "endpoint_sha256",
  "entry_sequence",
  "hard_link_group_sha256",
  "inode",
  "links",
  "mode",
  "mtime_nanoseconds",
  "name_bytes",
  "parent_identity_hmac_sha256",
  "relative_identity_hmac_sha256",
  "resource_kind",
  "size",
  "symlink_target_sha256",
  "type",
  "uid",
  "ownership",
  "ownership_binding_sha256",
]);
const INVENTORY_PAGE_FIELDS = Object.freeze([
  "entries",
  "entry_start",
  "namespace_role",
  "page_sequence",
  "parent_page_sha256",
  "sample_sequence",
]);
const INVENTORY_SAMPLE_FIELDS = Object.freeze([
  "entry_count",
  "first_sample_sha256",
  "namespace_role",
  "page_count",
  "second_sample_sha256",
]);
const INVENTORY_SET_FIELDS = Object.freeze([
  "inventory_sha256",
  "quiescence_fence_sha256",
  "sample_count",
  "samples",
]);
const CREATE_SETTLEMENT_FIELDS = Object.freeze([
  "absence_observation_sha256",
  "causal_edge_count",
  "causal_graph_sha256",
  "causal_node_count",
  "delivery_result_sha256",
  "endpoint_set_sha256",
  "inventory_set_sha256",
  "marker_link_count",
  "quiescence_fence_sha256",
  "residual_basis",
  "role_identity_set_sha256",
  "start_attempt_sha256",
  "variant",
]);
const UNCERTAIN_START_FIELDS = Object.freeze([
  "delivery_result_sha256",
  "marker_link_count",
  "reason",
  "recovery_invocation_authorized",
  "replay_authorized",
  "start_attempt_sha256",
]);
const CLEANUP_ACTION_FIELDS = Object.freeze([
  "action_kind",
  "action_sequence",
  "depth",
  "expected_postcondition_sha256",
  "expected_precondition_sha256",
  "namespace_role",
  "parent_fsync_required",
  "prerequisite_sha256",
  "role",
  "target_class",
  "target_identity_sha256",
]);
const CLEANUP_PLAN_PAGE_FIELDS = Object.freeze([
  "action_start",
  "actions",
  "create_settlement_sha256",
  "page_sequence",
  "parent_page_sha256",
]);
const CLEANUP_PLAN_FIELDS = Object.freeze([
  "action_count",
  "create_settlement_sha256",
  "marker_link_count",
  "page_count",
  "page_set_sha256",
  "reverse_causal_order",
]);
const CLEANUP_PROGRESS_FIELDS = Object.freeze([
  "action_start",
  "authority_sha256",
  "cleanup_plan_sha256",
  "outcomes",
  "parent_progress_sha256",
]);
const CLEANUP_OUTCOME_FIELDS = Object.freeze([
  "action_sequence",
  "action_sha256",
  "disposition",
  "marker_witness_observation_sha256",
  "observation_binding_sha256",
  "observation_challenge_sha256",
  "outcome_observation_sha256",
  "parent_fsync_observation_sha256",
  "precondition_observation_sha256",
  "recursive_prefix_observation_sha256",
  "source_reproof_observation_sha256",
]);
const CLEANUP_SETTLEMENT_FIELDS = Object.freeze([
  "cleanup_plan_sha256",
  "create_settlement_sha256",
  "endpoint_absence_sha256",
  "endpoint_absence_observation_sha256",
  "final_inventory_sha256",
  "marker_link_count",
  "marker_witness_topology_sha256",
  "marker_witness_topology_observation_sha256",
  "namespace_baselines_sha256",
  "progress_count",
  "progress_head_sha256",
  "role_absence_sha256",
  "role_absence_observation_sha256",
  "source_reproof_sha256",
  "source_reproof_observation_sha256",
  "terminal_receipt_publication_authorized",
]);
const RECEIPT_BINDING_FIELDS = Object.freeze([
  "cleanup_settlement_sha256",
  "create_settlement_sha256",
  "evidence_class",
  "operation_contract_sha256",
  "operation_kind",
  "operation_plan_sha256",
  "phase",
  "publisher_authority_sha256",
  "schema",
  "slot_sequence",
  "slot_sha256",
  "start_attempt_sha256",
  "witness_sha256",
]);
const TERMINAL_RECEIPT_FIELDS = Object.freeze([
  "fixture_id",
  "outcome",
  "phase",
  "previous_sha256",
  "result",
  "schema",
  "sequence",
]);
const TERMINAL_RECEIPT_EVENT_FIELDS = Object.freeze([
  "cleanup_settlement_sha256",
  "marker_link_count",
  "receipt",
  "receipt_publication_observation_sha256",
  "receipt_sha256",
  "witness_link_count",
]);
const COMPLETION_FIELDS = Object.freeze([
  "cleanup_settlement_sha256",
  "create_settlement_sha256",
  "marker_present",
  "marker_retirement_binding_sha256",
  "marker_retirement_observation_sha256",
  "provider_root_fsync_observation_sha256",
  "receipt_event_sha256",
  "receipt_sha256",
  "start_authority_sha256",
  "start_attempt_sha256",
  "variant",
  "witness_identity_sha256",
  "witness_link_count",
]);

const DELIVERY_DISPOSITIONS = new Set([
  "conclusive-not-created",
  "delivery-accepted-effect-possible",
  "indeterminate-effect-possible",
]);
const CREATE_SETTLEMENT_VARIANTS = new Set([
  "authenticated-live-identity",
  "exact-attempted-residual",
]);
const UNCERTAIN_START_REASONS = new Set([
  "delivery-indeterminate",
  "identity-incomplete",
  "endpoint-incomplete",
  "topology-incomplete",
]);
const INVENTORY_TYPES = new Set([
  "directory",
  "regular",
  "socket",
  "symlink",
]);

export class LiveProviderEffectFailure extends Error {
  constructor(message, exitStatus = 78) {
    super(message);
    this.exitStatus = exitStatus;
  }
}

function fail(message, exitStatus = 78) {
  throw new LiveProviderEffectFailure(message, exitStatus);
}

function closedDataEntries(value, label) {
  if (isProxy(value)) fail(`${label} contained a proxy`, 70);
  const array = Array.isArray(value);
  const keys = Reflect.ownKeys(value);
  if (keys.length > MAX_CANONICAL_CONTAINER_ENTRIES + (array ? 1 : 0)) {
    fail("live provider effect canonical container entry budget was exceeded", 70);
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
    fail("live provider effect canonical byte budget was exceeded", 70);
  }
  context.bytes += count;
}

function debitString(context, value) {
  const remaining = MAX_CANONICAL_BYTES - context.bytes;
  if (
    remaining < 2 ||
    Buffer.byteLength(value, "utf8") > remaining - 2
  ) {
    fail("live provider effect canonical byte budget was exceeded", 70);
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
      fail("live provider effect canonical depth was exceeded", 70);
    }
    context.nodes += 1;
    if (context.nodes > MAX_CANONICAL_NODES) {
      fail("live provider effect canonical node budget was exceeded", 70);
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
      fail("live provider effect canonical value was refused", 70);
    }
    if (active.has(current)) {
      fail("live provider effect canonical cycle was refused", 70);
    }
    const inspected = closedDataEntries(
      current,
      "live provider effect canonical value",
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
    "live provider effect canonical value",
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

function canonical(value) {
  preflightClosedDataGraph(value);
  return canonicalUnchecked(value);
}

export function liveProviderEffectBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function liveProviderEffectDigest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function valueDigest(value) {
  return liveProviderEffectDigest(liveProviderEffectBytes(value));
}

function processValueDigest(value) {
  return liveProviderProcessStartDigest(liveProviderProcessStartBytes(value));
}

function deepFreezeUnchecked(value) {
  if (value !== null && typeof value === "object") {
    const children = closedDataEntries(
      value,
      "live provider effect freeze value",
    ).entries;
    for (const [, child] of children) deepFreezeUnchecked(child);
    if (!Object.isFrozen(value)) Object.freeze(value);
  }
  return value;
}

function deepFreeze(value) {
  preflightClosedDataGraph(value);
  return deepFreezeUnchecked(value);
}

function exactKeys(value, keys, label) {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    fail(`${label} was malformed`, 70);
  }
  const inspected = closedDataEntries(value, label);
  if (
    inspected.kind !== "record" ||
    canonical(inspected.entries.map(([key]) => key).sort()) !==
      canonical([...keys].sort())
  ) {
    fail(`${label} fields were refused`, 70);
  }
}

function lowerHex(value, length) {
  return (
    typeof value === "string" &&
    value.length === length &&
    /^[0-9a-f]+$/u.test(value)
  );
}

function nonzeroSha256(value) {
  return lowerHex(value, 64) && value !== ZERO_SHA256;
}

function sha256OrZero(value) {
  return lowerHex(value, 64);
}

const U32_MAX = 4_294_967_295n;
const U64_MAX = 18_446_744_073_709_551_615n;

function decimal(
  value,
  { maximum = U64_MAX, positive = false } = {},
) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 20 ||
    !/^(?:0|[1-9][0-9]*)$/u.test(value)
  ) {
    return false;
  }
  const parsed = BigInt(value);
  return parsed <= maximum && (!positive || parsed > 0n);
}

function variant(fixtureOnly) {
  if (typeof fixtureOnly !== "boolean") {
    fail("live provider effect variant was refused", 70);
  }
  return fixtureOnly
    ? Object.freeze({
        evidenceClass: "fixture-only",
        operationKind: COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_KIND,
        schemas: COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_SCHEMAS,
      })
    : Object.freeze({
        evidenceClass: "production-pinned",
        operationKind: COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_KIND,
        schemas: COLIMA_LIVE_PROVIDER_EFFECT_SCHEMAS,
      });
}

function sourceContext(admission, source) {
  canonical({ admission, source });
  exactKeys(
    source,
    [
      "closeAuthority",
      "fixtureOnly",
      "startDecisionCompletion",
      "startDecisionPublicationPlan",
      "startDecisionSource",
    ],
    "live provider effect source",
  );
  const selected = variant(source.fixtureOnly);
  let fixtureId;
  let operationPlan;
  try {
    operationPlan =
      source.startDecisionSource.intentPublicationPlan.provider_operation_plan;
    fixtureId = operationPlan.fixture_id;
  } catch {
    fail("live provider effect source was refused", 69);
  }
  if (
    source.closeAuthority !== "owner" ||
    source.startDecisionSource?.fixtureOnly !== source.fixtureOnly ||
    !lowerHex(fixtureId, 32) ||
    admission?.schema !==
      (source.fixtureOnly
        ? COLIMA_LIVE_FIXTURE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA
        : COLIMA_LIVE_PROCESS_START_EFFECT_FRESH_ADMISSION_SCHEMA)
  ) {
    fail("live provider effect source was refused", 69);
  }
  try {
    validateColimaLiveProviderProcessStartEffectFreshAdmissionStructure(
      admission,
      source,
    );
  } catch (error) {
    if (error instanceof LiveProviderProcessStartFailure) {
      fail("live provider effect admission was refused", error.exitStatus);
    }
    throw error;
  }
  if (
    admission.root_observation.root_set_disposition !== "observed-pristine" ||
    admission.process_start_effect_candidate === null ||
    admission.process_start_effect_candidate.process_start_authorized !== false ||
    admission.process_start_effect_candidate.effect_execution_authorized !== false
  ) {
    fail("live provider effect admission was not pristine and deny-only", 73);
  }
  return Object.freeze({ fixtureId, operationPlan, selected });
}

function validateIdentity(value, label) {
  exactKeys(value, IDENTITY_FIELDS, label);
  if (
    !decimal(value.device) ||
    !decimal(value.inode, { positive: true }) ||
    value.mode !== "0700" ||
    !decimal(value.uid, { maximum: U32_MAX })
  ) {
    fail(`${label} was refused`, 69);
  }
  return value;
}

function validateBaselineDescriptorBindings(binding, label) {
  if (
    !Array.isArray(binding.baseline_relative_identity_hmac_sha256) ||
    !Array.isArray(binding.baseline_descriptors) ||
    binding.baseline_descriptors.length !==
      binding.baseline_relative_identity_hmac_sha256.length ||
    binding.baseline_descriptors.length > MAX_CANONICAL_CONTAINER_ENTRIES
  ) {
    fail(`${label} baseline descriptors were refused`, 69);
  }
  for (const descriptor of binding.baseline_descriptors) {
    exactKeys(
      descriptor,
      COLIMA_LIVE_BASELINE_DESCRIPTOR_BINDING_FIELDS,
      `${label} baseline descriptor`,
    );
    if (
      !nonzeroSha256(descriptor.descriptor_sha256) ||
      !nonzeroSha256(descriptor.relative_identity_hmac_sha256)
    ) {
      fail(`${label} baseline descriptor was refused`, 69);
    }
  }
  if (
    new Set(
      binding.baseline_descriptors.map((entry) => entry.descriptor_sha256),
    ).size !== binding.baseline_descriptors.length ||
    !liveProviderEffectBytes(
      binding.baseline_descriptors.map(
        (entry) => entry.relative_identity_hmac_sha256,
      ),
    ).equals(
      liveProviderEffectBytes(
        binding.baseline_relative_identity_hmac_sha256,
      ),
    )
  ) {
    fail(`${label} baseline descriptors were refused`, 69);
  }
}

function validateNamespaceBindings(value, admission, commonDevice, commonUid) {
  if (
    !Array.isArray(value) ||
    value.length !== COLIMA_LIVE_MUTATION_SURFACE_ROLES.length
  ) {
    fail("live provider effect namespace bindings were refused", 69);
  }
  for (const [index, binding] of value.entries()) {
    exactKeys(
      binding,
      NAMESPACE_BINDING_FIELDS,
      "live provider effect namespace binding",
    );
    const observed = admission.root_observation.root_observations[index];
    validateBaselineDescriptorBindings(
      binding,
      "live provider effect namespace binding",
    );
    if (
      !Array.isArray(binding.baseline_relative_identity_hmac_sha256) ||
      binding.baseline_relative_identity_hmac_sha256.length >
        MAX_CANONICAL_CONTAINER_ENTRIES ||
      binding.baseline_relative_identity_hmac_sha256.some(
        (identity) => !nonzeroSha256(identity),
      ) ||
      new Set(binding.baseline_relative_identity_hmac_sha256).size !==
        binding.baseline_relative_identity_hmac_sha256.length ||
      binding.baseline_relative_identity_hmac_sha256.includes(
        binding.namespace_identity_hmac_sha256,
      ) ||
      binding.baseline_descriptor_set_hmac_sha256 !==
        observed.baseline_descriptor_set_hmac_sha256 ||
      !nonzeroSha256(binding.baseline_descriptor_set_hmac_sha256) ||
      !liveProviderEffectBytes(
        binding.baseline_relative_identity_hmac_sha256,
      ).equals(
        liveProviderEffectBytes(
          observed.baseline_relative_identity_hmac_sha256,
        ),
      ) ||
      !liveProviderEffectBytes(binding.baseline_descriptors).equals(
        liveProviderEffectBytes(observed.baseline_descriptors),
      ) ||
      binding.baseline_set_binding_sha256 !==
        valueDigest({
          baseline_descriptor_set_hmac_sha256:
            binding.baseline_descriptor_set_hmac_sha256,
          baseline_descriptors: binding.baseline_descriptors,
          baseline_relative_identity_hmac_sha256:
            binding.baseline_relative_identity_hmac_sha256,
          observed_entry_set_hmac_sha256:
            binding.observed_entry_set_hmac_sha256,
          role: binding.role,
        })
    ) {
      fail("live provider effect namespace baseline was refused", 69);
    }
    if (
      binding.role !== COLIMA_LIVE_MUTATION_SURFACE_ROLES[index] ||
      binding.role !== observed.role ||
      binding.device !== commonDevice ||
      binding.uid !== commonUid ||
      binding.mode !== "0700" ||
      !decimal(binding.inode, { positive: true }) ||
      !nonzeroSha256(binding.namespace_identity_hmac_sha256) ||
      !nonzeroSha256(binding.observed_entry_set_hmac_sha256) ||
      binding.namespace_identity_hmac_sha256 !==
        observed.namespace_identity_hmac_sha256 ||
      binding.observed_entry_set_hmac_sha256 !==
        observed.observed_entry_set_hmac_sha256
    ) {
      fail("live provider effect namespace binding was refused", 69);
    }
  }
  if (
    new Set(value.map((binding) => binding.namespace_identity_hmac_sha256))
      .size !== value.length ||
    new Set(value.map((binding) => binding.observed_entry_set_hmac_sha256))
      .size !== value.length
  ) {
    fail("live provider effect namespace identities were not distinct", 69);
  }
  const baselineIdentities = value.flatMap(
    (binding) => binding.baseline_relative_identity_hmac_sha256,
  );
  const namespaceIdentities = new Set(
    value.map((binding) => binding.namespace_identity_hmac_sha256),
  );
  if (
    baselineIdentities.length > COLIMA_LIVE_MAX_BASELINE_DESCENDANTS ||
    new Set(baselineIdentities).size !== baselineIdentities.length ||
    baselineIdentities.some((identity) => namespaceIdentities.has(identity))
  ) {
    fail("live provider effect baseline identities were not distinct", 69);
  }
  return value;
}

function deriveNamespaceBindings(value, admission) {
  if (
    !Array.isArray(value) ||
    value.length !== COLIMA_LIVE_MUTATION_SURFACE_ROLES.length
  ) {
    fail("live provider effect physical namespace bindings were refused", 69);
  }
  return value.map((binding, index) => {
    exactKeys(
      binding,
      PHYSICAL_NAMESPACE_BINDING_FIELDS,
      "live provider effect physical namespace binding",
    );
    const observed = admission.root_observation.root_observations[index];
    if (
      binding.role !== COLIMA_LIVE_MUTATION_SURFACE_ROLES[index] ||
      binding.role !== observed.role
    ) {
      fail("live provider effect physical namespace binding was refused", 69);
    }
    const derived = {
      baseline_descriptor_set_hmac_sha256:
        observed.baseline_descriptor_set_hmac_sha256,
      baseline_descriptors: observed.baseline_descriptors,
      baseline_relative_identity_hmac_sha256:
        observed.baseline_relative_identity_hmac_sha256,
      baseline_set_binding_sha256: valueDigest({
        baseline_descriptor_set_hmac_sha256:
          observed.baseline_descriptor_set_hmac_sha256,
        baseline_descriptors: observed.baseline_descriptors,
        baseline_relative_identity_hmac_sha256:
          observed.baseline_relative_identity_hmac_sha256,
        observed_entry_set_hmac_sha256:
          observed.observed_entry_set_hmac_sha256,
        role: observed.role,
      }),
      device: binding.device,
      inode: binding.inode,
      mode: binding.mode,
      namespace_identity_hmac_sha256:
        observed.namespace_identity_hmac_sha256,
      observed_entry_set_hmac_sha256:
        observed.observed_entry_set_hmac_sha256,
      role: observed.role,
      uid: binding.uid,
    };
    return derived;
  });
}

function baselineDescriptorProjection(entry) {
  return Object.fromEntries(
    COLIMA_LIVE_BASELINE_DESCRIPTOR_FIELDS.map((field) => [field, entry[field]]),
  );
}

function validateInvocationBinding(value) {
  exactKeys(
    value,
    INVOCATION_BINDING_FIELDS,
    "live provider effect invocation binding",
  );
  if (Object.values(value).some((entry) => !nonzeroSha256(entry))) {
    fail("live provider effect invocation binding was refused", 69);
  }
  return value;
}

const ROLE_PARENTS = Object.freeze({
  hostagent: "outer",
  outer: "state-owner",
  "ssh-controlmaster": "hostagent",
  usernet: "hostagent",
});
const ROLE_DEPTHS = Object.freeze({
  hostagent: 1,
  outer: 0,
  "ssh-controlmaster": 2,
  usernet: 2,
});
const ENDPOINT_ROLES = Object.freeze({
  "engine-api": "hostagent",
  "hostagent-control": "hostagent",
  "ssh-control": "ssh-controlmaster",
  "usernet-control": "usernet",
});
const ENDPOINT_NAMESPACES = Object.freeze({
  "engine-api": "colima-home-namespace",
  "hostagent-control": "lima-home-namespace",
  "ssh-control": "temporary-namespace",
  "usernet-control": "lima-home-namespace",
});
const DOCKER_CONTEXT_NAMESPACE = "docker-config-namespace";

function validateRoleContracts(value) {
  if (!Array.isArray(value) || value.length !== COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length) {
    fail("live provider effect role contracts were refused", 69);
  }
  for (const [index, contract] of value.entries()) {
    exactKeys(contract, ROLE_CONTRACT_FIELDS, "live provider effect role contract");
    const role = COLIMA_LIVE_PROVIDER_EFFECT_ROLES[index];
    if (
      contract.role !== role ||
      contract.parent_role !== ROLE_PARENTS[role] ||
      contract.depth !== ROLE_DEPTHS[role] ||
      !nonzeroSha256(contract.public_key_spki_sha256) ||
      !nonzeroSha256(contract.role_challenge_sha256) ||
      !nonzeroSha256(contract.argv_sha256) ||
      !nonzeroSha256(contract.cwd_identity_sha256) ||
      !nonzeroSha256(contract.environment_sha256) ||
      !nonzeroSha256(contract.executable_sha256) ||
      !nonzeroSha256(contract.toolchain_sha256) ||
      !decimal(contract.uid, { maximum: U32_MAX })
    ) {
      fail("live provider effect role contract was refused", 69);
    }
  }
  if (
    new Set(value.map((entry) => entry.public_key_spki_sha256)).size !==
      value.length ||
    new Set(value.map((entry) => entry.role_challenge_sha256)).size !==
      value.length
  ) {
    fail("live provider effect role identities were not distinct", 69);
  }
  return value;
}

function validateEndpointContracts(value) {
  if (!Array.isArray(value) || value.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length) {
    fail("live provider effect endpoint contracts were refused", 69);
  }
  for (const [index, contract] of value.entries()) {
    exactKeys(
      contract,
      ENDPOINT_CONTRACT_FIELDS,
      "live provider effect endpoint contract",
    );
    const kind = COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS[index];
    if (
      contract.endpoint_kind !== kind ||
      contract.role !== ENDPOINT_ROLES[kind] ||
      contract.namespace_role !== ENDPOINT_NAMESPACES[kind] ||
      !nonzeroSha256(contract.initial_challenge_commitment_sha256) ||
      !nonzeroSha256(contract.final_challenge_commitment_sha256) ||
      contract.initial_challenge_commitment_sha256 ===
        contract.final_challenge_commitment_sha256 ||
      !nonzeroSha256(contract.path_identity_hmac_sha256) ||
      (kind === "engine-api"
        ? !nonzeroSha256(contract.docker_context_sha256) ||
          contract.docker_context_namespace_role !==
            DOCKER_CONTEXT_NAMESPACE ||
          !nonzeroSha256(
            contract.docker_context_path_identity_hmac_sha256,
          )
        : contract.docker_context_sha256 !== ZERO_SHA256 ||
          contract.docker_context_namespace_role !== "none" ||
          contract.docker_context_path_identity_hmac_sha256 !== ZERO_SHA256)
    ) {
      fail("live provider effect endpoint contract was refused", 69);
    }
  }
  if (
    new Set(
      value.flatMap((entry) => [
        entry.initial_challenge_commitment_sha256,
        entry.final_challenge_commitment_sha256,
        entry.path_identity_hmac_sha256,
      ]),
    ).size !== value.length * 3
  ) {
    fail("live provider effect endpoint challenges were not distinct", 69);
  }
  return value;
}

function operationContract(fixtureOnly) {
  const selected = variant(fixtureOnly);
  const fixtureAuthority = fixtureOnly;
  return deepFreeze({
    action: COLIMA_LIVE_PROVIDER_EFFECT_ACTION,
    authority_model: fixtureOnly
      ? "state-owned-single-use-fixed-fixture-invoker"
      : "production-deny-only-no-invoker",
    bounds: { ...COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS },
    capabilities: {
      cleanup_authorized: fixtureAuthority,
      effect_witness_publication_authorized: fixtureAuthority,
      environment_publication_authorized: false,
      evidence_publication_authorized: false,
      finalization_authorized: false,
      fixture_effect_execution_authorized: fixtureAuthority,
      fixture_effect_recovery_authorized: fixtureAuthority,
      lifecycle_exposed: false,
      marker_link_authorized: fixtureAuthority,
      marker_retirement_authorized: fixtureAuthority,
      process_group_ownership_authorized: fixtureAuthority,
      process_signal_authorized: fixtureAuthority,
      process_spawn_authorized: fixtureAuthority,
      process_start_authorized: fixtureAuthority,
      provider_adapter_execution_authorized: false,
      provider_artifact_publication_authorized: false,
      provider_recovery_authorized: false,
      provider_root_mutation_authorized: false,
      receipt_publication_authorized: fixtureAuthority,
      runtime_publication_authorized: false,
      start_attempt_publication_authorized: fixtureAuthority,
      start_authority_publication_authorized: fixtureAuthority,
      state_effect_slot_publication_authorized: fixtureAuthority,
      state_evidence_publication_authorized: fixtureAuthority,
    },
    endpoints: [...COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS],
    evidence_class: selected.evidenceClass,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    operation_kind: selected.operationKind,
    receipt_binding_schema: selected.schemas.receipt_binding,
    roles: [...COLIMA_LIVE_PROVIDER_EFFECT_ROLES],
    schema: selected.schemas.contract,
    state_integration: COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
    terminal_receipt_schema: selected.schemas.terminal_receipt,
    threat_boundary: "cooperative-same-uid-single-host",
  });
}

function validateOperationContract(value, fixtureOnly) {
  const expected = operationContract(fixtureOnly);
  exactKeys(value, CONTRACT_FIELDS, "live provider effect operation contract");
  exactKeys(value.bounds, Object.keys(COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS), "live provider effect bounds");
  exactKeys(value.capabilities, CAPABILITY_FIELDS, "live provider effect capabilities");
  if (!liveProviderEffectBytes(value).equals(liveProviderEffectBytes(expected))) {
    fail("live provider effect operation contract was refused", 69);
  }
  return value;
}

export const COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT =
  operationContract(false);
export const COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT =
  operationContract(true);
export const COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT);
export const COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256 =
  valueDigest(COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT);

function expectedContractSha256(fixtureOnly) {
  return fixtureOnly
    ? COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256
    : COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT_SHA256;
}

export function validateColimaLiveProviderEffectPublicationPlan(value, source) {
  exactKeys(value, PLAN_FIELDS, "live provider effect publication plan");
  preflightClosedDataGraph(value);
  const { fixtureId, operationPlan, selected } = sourceContext(
    value.admission,
    source,
  );
  validateIdentity(value.provider_root_identity, "live provider effect provider root identity");
  validateIdentity(value.state_run_identity, "live provider effect state run identity");
  validateNamespaceBindings(
    value.namespace_bindings,
    value.admission,
    value.provider_root_identity.device,
    value.provider_root_identity.uid,
  );
  validateInvocationBinding(value.invocation_binding);
  validateRoleContracts(value.planned_role_contracts);
  validateEndpointContracts(value.planned_endpoint_contracts);
  const engineEndpointContract = value.planned_endpoint_contracts.find(
    (contract) => contract.endpoint_kind === "engine-api",
  );
  const reservedInventoryIdentities = [
    ...value.planned_endpoint_contracts.map(
      (contract) => contract.path_identity_hmac_sha256,
    ),
    engineEndpointContract?.docker_context_path_identity_hmac_sha256,
  ];
  const admissionInventoryIdentities = new Set(
    value.namespace_bindings.flatMap((binding) => [
      binding.namespace_identity_hmac_sha256,
      ...binding.baseline_relative_identity_hmac_sha256,
    ]),
  );
  const outerRole = value.planned_role_contracts.find(
    (contract) => contract.role === "outer",
  );
  const outerInvocationMatches = [
    ["argv_sha256", "argv_sha256"],
    ["cwd_identity_sha256", "cwd_identity_sha256"],
    ["environment_sha256", "environment_sha256"],
    ["executable_sha256", "executable_sha256"],
    ["toolchain_sha256", "toolchain_sha256"],
  ].every(
    ([roleField, invocationField]) =>
      outerRole?.[roleField] === value.invocation_binding[invocationField],
  );
  const identities = [
    value.provider_root_identity,
    value.state_run_identity,
    ...value.namespace_bindings,
  ].map((identity) => `${identity.device}:${identity.inode}`);
  if (
    value.schema !== selected.schemas.plan ||
    value.evidence_class !== selected.evidenceClass ||
    value.fixture_id !== fixtureId ||
    value.operation_kind !== selected.operationKind ||
    value.operation_contract_sha256 !== expectedContractSha256(source.fixtureOnly) ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION ||
    value.branch !== "sibling-after-completed-start-decision" ||
    value.fixed_marker_name !== COLIMA_LIVE_PROVIDER_RESERVATION_NAME ||
    value.admission_sha256 !== processValueDigest(value.admission) ||
    value.completed_start_decision_projection_sha256 !==
      processValueDigest(value.admission.completed_start_decision_projection) ||
    value.driver_contract_sha256 !==
      valueDigest({
        invocation_binding: value.invocation_binding,
        operation_kind: value.operation_kind,
      }) ||
    value.preparation_observation_sha256 !==
      operationPlan.preparation_observation_sha256 ||
    value.receipt_previous_sha256 !== operationPlan.source_head_sha256 ||
    value.receipt_sequence !== operationPlan.source_sequence + 1 ||
    !nonzeroSha256(value.planned_start_attempt_sha256) ||
    !nonzeroSha256(value.planned_quiescence_fence_sha256) ||
    value.planned_start_attempt_sha256 ===
      value.planned_quiescence_fence_sha256 ||
    value.provider_root_identity.device !== value.state_run_identity.device ||
    value.provider_root_identity.uid !== value.state_run_identity.uid ||
    value.planned_role_contracts.some(
      (contract) => contract.uid !== value.provider_root_identity.uid,
    ) ||
    reservedInventoryIdentities.some((identity) => !nonzeroSha256(identity)) ||
    new Set(reservedInventoryIdentities).size !==
      reservedInventoryIdentities.length ||
    reservedInventoryIdentities.some((identity) =>
      admissionInventoryIdentities.has(identity),
    ) ||
    !outerInvocationMatches ||
    new Set(identities).size !== identities.length
  ) {
    fail("live provider effect publication plan was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderEffectPublicationPlan(input) {
  exactKeys(
    input,
    [
      "admission",
      "invocationBinding",
      "namespaceBindings",
      "plannedEndpointContracts",
      "plannedQuiescenceFenceSha256",
      "plannedRoleContracts",
      "plannedStartAttemptSha256",
      "providerRootIdentity",
      "source",
      "stateRunIdentity",
    ],
    "live provider effect publication plan input",
  );
  const {
    admission,
    invocationBinding,
    namespaceBindings,
    plannedEndpointContracts,
    plannedQuiescenceFenceSha256,
    plannedRoleContracts,
    plannedStartAttemptSha256,
    providerRootIdentity,
    source,
    stateRunIdentity,
  } = input;
  const { fixtureId, operationPlan, selected } = sourceContext(admission, source);
  const value = {
    admission,
    admission_sha256: processValueDigest(admission),
    branch: "sibling-after-completed-start-decision",
    completed_start_decision_projection_sha256: processValueDigest(
      admission.completed_start_decision_projection,
    ),
    driver_contract_sha256: valueDigest({
      invocation_binding: invocationBinding,
      operation_kind: selected.operationKind,
    }),
    evidence_class: selected.evidenceClass,
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    fixture_id: fixtureId,
    invocation_binding: invocationBinding,
    namespace_bindings: deriveNamespaceBindings(namespaceBindings, admission),
    operation_contract_sha256: expectedContractSha256(source.fixtureOnly),
    operation_kind: selected.operationKind,
    planned_endpoint_contracts: plannedEndpointContracts,
    planned_quiescence_fence_sha256: plannedQuiescenceFenceSha256,
    planned_role_contracts: plannedRoleContracts,
    planned_start_attempt_sha256: plannedStartAttemptSha256,
    preparation_observation_sha256:
      operationPlan.preparation_observation_sha256,
    provider_root_identity: providerRootIdentity,
    receipt_previous_sha256: operationPlan.source_head_sha256,
    receipt_sequence: operationPlan.source_sequence + 1,
    schema: selected.schemas.plan,
    state_integration: COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
    state_run_identity: stateRunIdentity,
  };
  validateColimaLiveProviderEffectPublicationPlan(value, source);
  return deepFreeze(value);
}

function slotBinding(publicationPlan, slotSequence, slotSha256) {
  if (
    !Number.isSafeInteger(slotSequence) ||
    slotSequence < 0 ||
    slotSequence > 63 ||
    !nonzeroSha256(slotSha256)
  ) {
    fail("live provider effect slot binding was refused", 69);
  }
  return {
    evidence_class: publicationPlan.evidence_class,
    fixture_id: publicationPlan.fixture_id,
    operation_contract_sha256: publicationPlan.operation_contract_sha256,
    operation_kind: publicationPlan.operation_kind,
    operation_plan_sha256: valueDigest(publicationPlan),
    slot_sequence: slotSequence,
    slot_sha256: slotSha256,
    state_integration: COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION,
  };
}

function validateSlotBinding(value, publicationPlan) {
  if (
    value.evidence_class !== publicationPlan.evidence_class ||
    value.fixture_id !== publicationPlan.fixture_id ||
    value.operation_contract_sha256 !== publicationPlan.operation_contract_sha256 ||
    value.operation_kind !== publicationPlan.operation_kind ||
    value.operation_plan_sha256 !== valueDigest(publicationPlan) ||
    !Number.isSafeInteger(value.slot_sequence) ||
    value.slot_sequence < 0 ||
    value.slot_sequence > 63 ||
    !nonzeroSha256(value.slot_sha256) ||
    value.state_integration !== COLIMA_LIVE_PROVIDER_EFFECT_STATE_INTEGRATION
  ) {
    fail("live provider effect slot binding was refused", 69);
  }
}

export function validateColimaLiveProviderEffectWitness(
  value,
  options,
) {
  exactKeys(
    options,
    ["publicationPlan", "source"],
    "live provider effect witness validation input",
  );
  const { publicationPlan, source } = options;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  const { selected } = sourceContext(publicationPlan.admission, source);
  exactKeys(value, WITNESS_FIELDS, "live provider effect witness");
  preflightClosedDataGraph(value);
  validateSlotBinding(value, publicationPlan);
  if (
    value.schema !== selected.schemas.witness ||
    value.fixed_marker_name !== COLIMA_LIVE_PROVIDER_RESERVATION_NAME ||
    value.marker_link_count !== 2 ||
    !nonzeroSha256(value.marker_witness_identity_sha256) ||
    value.process_attempt !== "not-attempted" ||
    value.reservation_disposition !== "held-pre-attempt" ||
    value.invocation_binding_sha256 !== valueDigest(publicationPlan.invocation_binding) ||
    value.namespace_bindings_sha256 !== valueDigest(publicationPlan.namespace_bindings) ||
    value.planned_role_contracts_sha256 !== valueDigest(publicationPlan.planned_role_contracts) ||
    value.planned_endpoint_contracts_sha256 !==
      valueDigest(publicationPlan.planned_endpoint_contracts) ||
    value.planned_quiescence_fence_sha256 !==
      publicationPlan.planned_quiescence_fence_sha256 ||
    value.planned_start_attempt_sha256 !==
      publicationPlan.planned_start_attempt_sha256
  ) {
    fail("live provider effect witness was refused", 69);
  }
  return value;
}

export function buildColimaLiveProviderEffectWitness(input) {
  exactKeys(
    input,
    [
      "markerWitnessIdentitySha256",
      "publicationPlan",
      "slotSequence",
      "slotSha256",
      "source",
    ],
    "live provider effect witness input",
  );
  const {
    markerWitnessIdentitySha256,
    publicationPlan,
    slotSequence,
    slotSha256,
    source,
  } = input;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  const { selected } = sourceContext(publicationPlan.admission, source);
  const value = {
    ...slotBinding(publicationPlan, slotSequence, slotSha256),
    fixed_marker_name: COLIMA_LIVE_PROVIDER_RESERVATION_NAME,
    invocation_binding_sha256: valueDigest(publicationPlan.invocation_binding),
    marker_link_count: 2,
    marker_witness_identity_sha256: markerWitnessIdentitySha256,
    namespace_bindings_sha256: valueDigest(publicationPlan.namespace_bindings),
    planned_endpoint_contracts_sha256: valueDigest(
      publicationPlan.planned_endpoint_contracts,
    ),
    planned_quiescence_fence_sha256:
      publicationPlan.planned_quiescence_fence_sha256,
    planned_role_contracts_sha256: valueDigest(
      publicationPlan.planned_role_contracts,
    ),
    planned_start_attempt_sha256: publicationPlan.planned_start_attempt_sha256,
    process_attempt: "not-attempted",
    reservation_disposition: "held-pre-attempt",
    schema: selected.schemas.witness,
  };
  validateColimaLiveProviderEffectWitness(value, { publicationPlan, source });
  return deepFreeze(value);
}

function validatePublisherAuthorityChain(value, witness) {
  if (
    isProxy(value) ||
    !Array.isArray(value) ||
    Object.getPrototypeOf(value) !== Array.prototype ||
    value.length === 0 ||
    value.length > MAX_PUBLISHER_AUTHORITIES
  ) {
    fail("live provider effect publisher authority chain was refused", 70);
  }
  preflightClosedDataGraph(value);
  for (const [index, authority] of value.entries()) {
    exactKeys(
      authority,
      PUBLISHER_AUTHORITY_FIELDS,
      "live provider effect publisher authority",
    );
    const prior = value[index - 1];
    if (
      !Number.isSafeInteger(authority.first_event_sequence) ||
      authority.first_event_sequence < 0 ||
      authority.first_event_sequence >= MAX_EFFECT_EVENTS ||
      !nonzeroSha256(authority.authority_sha256) ||
      (index === 0
        ? authority.kind !== "owner-slot" ||
          authority.authority_sha256 !== witness.slot_sha256 ||
          authority.first_event_sequence !== 0 ||
          authority.prior_authority_sha256 !== ZERO_SHA256 ||
          authority.recovery_claim_sha256 !== ZERO_SHA256
        : authority.kind !== "state-recovery-claim" ||
          authority.authority_sha256 !== authority.recovery_claim_sha256 ||
          authority.prior_authority_sha256 !== prior.authority_sha256 ||
          authority.first_event_sequence < prior.first_event_sequence)
    ) {
      fail("live provider effect publisher authority was refused", 69);
    }
  }
  if (
    new Set(value.map((authority) => authority.authority_sha256)).size !==
      value.length
  ) {
    fail("live provider effect publisher authority was reused", 69);
  }
  return value;
}

function publisherAuthorityAt(eventSequence, publisherAuthorityChain, witness) {
  validatePublisherAuthorityChain(publisherAuthorityChain, witness);
  const authority = publisherAuthorityChain.findLast(
    (candidate) => candidate.first_event_sequence <= eventSequence,
  );
  if (authority === undefined) {
    fail("live provider effect publisher authority frontier was refused", 69);
  }
  return authority.authority_sha256;
}

function strictBase64(value, label, maximumBytes = MAX_ENDPOINT_FRAME_BYTES) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} was refused`, 69);
  }
  let decoded;
  try {
    decoded = Buffer.from(value, "base64");
  } catch {
    fail(`${label} was refused`, 69);
  }
  if (
    decoded.length === 0 ||
    decoded.length > maximumBytes ||
    decoded.toString("base64") !== value
  ) {
    fail(`${label} was refused`, 69);
  }
  return decoded;
}

function publicKey(value, label) {
  const der = strictBase64(value, label, 44);
  try {
    const key = createPublicKey({ format: "der", key: der, type: "spki" });
    if (
      der.length !== 44 ||
      key.asymmetricKeyType !== "ed25519" ||
      !key.export({ format: "der", type: "spki" }).equals(der)
    ) {
      fail(`${label} was refused`, 69);
    }
    return Object.freeze({ der, key });
  } catch (error) {
    if (error instanceof LiveProviderEffectFailure) throw error;
    fail(`${label} was refused`, 69);
  }
}

function withoutProof(evidence) {
  preflightClosedDataGraph(evidence);
  return Object.fromEntries(
    Object.entries(evidence).filter(([field]) => field !== "proof_base64"),
  );
}

const PROOF_DOMAINS = new Set([
  "endpoint",
  "quiescence-ack",
  "role-identity",
]);

export function colimaLiveProviderEffectProofBytes(input) {
  exactKeys(
    input,
    [
      "domain",
      "eventSequence",
      "evidence",
      "parentSha256",
      "publicationPlan",
      "publisherAuthoritySha256",
      "source",
      "witness",
    ],
    "live provider effect proof input",
  );
  const {
    domain,
    eventSequence,
    evidence,
    parentSha256,
    publicationPlan,
    publisherAuthoritySha256,
    source,
    witness,
  } = input;
  if (
    !PROOF_DOMAINS.has(domain) ||
    !Number.isSafeInteger(eventSequence) ||
    eventSequence < 0 ||
    eventSequence >= MAX_EFFECT_EVENTS ||
    !nonzeroSha256(parentSha256) ||
    !nonzeroSha256(publisherAuthoritySha256)
  ) {
    fail("live provider effect proof context was refused", 69);
  }
  validateColimaLiveProviderEffectWitness(witness, {
    publicationPlan,
    source,
  });
  return liveProviderEffectBytes({
    domain: `synveda.clean-engine.provider-effect.${domain}.v1`,
    event_sequence: eventSequence,
    evidence: withoutProof(evidence),
    operation_plan_sha256: valueDigest(publicationPlan),
    parent_sha256: parentSha256,
    publisher_authority_sha256: publisherAuthoritySha256,
    slot_sha256: witness.slot_sha256,
    witness_sha256: valueDigest(witness),
  });
}

function proofBytes({
  domain,
  eventSequence,
  evidence,
  parentSha256,
  publicationPlan,
  publisherAuthoritySha256,
  witness,
}) {
  return liveProviderEffectBytes({
    domain: `synveda.clean-engine.provider-effect.${domain}.v1`,
    event_sequence: eventSequence,
    evidence: withoutProof(evidence),
    operation_plan_sha256: valueDigest(publicationPlan),
    parent_sha256: parentSha256,
    publisher_authority_sha256: publisherAuthoritySha256,
    slot_sha256: witness.slot_sha256,
    witness_sha256: valueDigest(witness),
  });
}

function validateProof({
  domain,
  eventSequence,
  evidence,
  parentSha256,
  publicationPlan,
  publicKeyDerBase64,
  publisherAuthoritySha256,
  witness,
}) {
  const { key } = publicKey(
    publicKeyDerBase64,
    "live provider effect proof public key",
  );
  const signature = strictBase64(
    evidence.proof_base64,
    "live provider effect proof",
    128,
  );
  let valid = false;
  try {
    valid = verify(
      null,
      proofBytes({
        domain,
        eventSequence,
        evidence,
        parentSha256,
        publicationPlan,
        publisherAuthoritySha256,
        witness,
      }),
      key,
      signature,
    );
  } catch {
    valid = false;
  }
  if (!valid) fail("live provider effect proof was refused", 69);
}

function findEvent(history, sha256, kind) {
  if (!nonzeroSha256(sha256)) return undefined;
  return history.find(
    (event) =>
      event.event_kind === kind && valueDigest(event) === sha256,
  );
}

function eventsOfKind(history, kind) {
  return history.filter((event) => event.event_kind === kind);
}

function uniqueEvent(history, kind) {
  const matches = eventsOfKind(history, kind);
  if (matches.length !== 1) {
    fail(`live provider effect ${kind} history was refused`, 69);
  }
  return matches[0];
}

function validateStartReproof(
  value,
  { observationSequence, publicationPlan, source, witness },
) {
  exactKeys(value, START_REPROOF_FIELDS, "live provider effect start reproof");
  const expectedTopologySha256 = valueDigest({
    endpoints: publicationPlan.planned_endpoint_contracts,
    roles: publicationPlan.planned_role_contracts,
  });
  if (
    value.observation_sequence !== observationSequence ||
    value.admission_sha256 !== processValueDigest(publicationPlan.admission) ||
    value.invocation_sha256 !== valueDigest(publicationPlan.invocation_binding) ||
    value.namespace_sha256 !== valueDigest(publicationPlan.namespace_bindings) ||
    value.provider_root_identity_sha256 !==
      valueDigest(publicationPlan.provider_root_identity) ||
    value.state_run_identity_sha256 !==
      valueDigest(publicationPlan.state_run_identity) ||
    value.source_sha256 !== valueDigest(source) ||
    value.topology_sha256 !== expectedTopologySha256 ||
    !nonzeroSha256(value.observation_challenge_sha256) ||
    !nonzeroSha256(value.marker_witness_identity_sha256) ||
    value.marker_present !== true ||
    value.marker_witness_same_inode !== true ||
    value.marker_witness_identity_sha256 !==
      witness.marker_witness_identity_sha256 ||
    value.marker_link_count !== 2 ||
    value.witness_link_count !== 2
  ) {
    fail("live provider effect start reproof was refused", 69);
  }
}

function validateStartAuthority(
  evidence,
  { eventSequence, history, publicationPlan, source, witness },
) {
  exactKeys(
    evidence,
    START_AUTHORITY_FIELDS,
    "live provider effect start authority",
  );
  const hashes = [
    evidence.current_owner_boot_sha256,
    evidence.current_owner_instance_sha256,
    evidence.first_reproof_sha256,
    evidence.witness_sha256,
  ];
  validateStartReproof(evidence.first_reproof, {
    observationSequence: 0,
    publicationPlan,
    source,
    witness,
  });
  if (
    eventSequence !== 0 ||
    history.length !== 0 ||
    evidence.authority_scope !==
      "current-in-memory-owner-attempt-publication-only" ||
    evidence.serialized_invocation_authorized !== false ||
    evidence.recovery_invocation_authorized !== false ||
    hashes.some((value) => !nonzeroSha256(value)) ||
    new Set(hashes).size !== hashes.length ||
    evidence.witness_sha256 !== valueDigest(witness) ||
    evidence.first_reproof_sha256 !== valueDigest(evidence.first_reproof)
  ) {
    fail("live provider effect start authority was refused", 69);
  }
}

function validateStartAttempt(
  evidence,
  { history, publicationPlan, source, witness },
) {
  exactKeys(
    evidence,
    START_ATTEMPT_FIELDS,
    "live provider effect start attempt",
  );
  const authority = uniqueEvent(history, "start-authority");
  validateStartReproof(evidence.second_reproof, {
    observationSequence: 1,
    publicationPlan,
    source,
    witness,
  });
  if (
    evidence.variant !== "attempt-fence" ||
    evidence.authority_event_sha256 !== valueDigest(authority) ||
    evidence.attempt_sha256 !== publicationPlan.planned_start_attempt_sha256 ||
    evidence.invocation_binding_sha256 !==
      valueDigest(publicationPlan.invocation_binding) ||
    evidence.invocation_limit !== 1 ||
    evidence.recovery_invocation_authorized !== false ||
    evidence.replay_authorized !== false ||
    evidence.second_reproof_sha256 !== valueDigest(evidence.second_reproof) ||
    evidence.second_reproof.observation_challenge_sha256 ===
      authority.evidence.first_reproof.observation_challenge_sha256 ||
    eventsOfKind(history, "start-attempt").length !== 0
  ) {
    fail("live provider effect start attempt was refused", 69);
  }
}

function validateDeliveryResult(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    DELIVERY_RESULT_FIELDS,
    "live provider effect delivery result",
  );
  const attempt = uniqueEvent(history, "start-attempt");
  const outerEdges = eventsOfKind(history, "launch-edge").filter(
    (event) => event.evidence.child_role === "outer",
  );
  const expectedSafeErrorCode = {
    "conclusive-not-created": "not-created",
    "delivery-accepted-effect-possible": "accepted",
    "indeterminate-effect-possible": "indeterminate",
  }[evidence.delivery_disposition];
  if (
    !DELIVERY_DISPOSITIONS.has(evidence.delivery_disposition) ||
    evidence.attempt_event_sha256 !== valueDigest(attempt) ||
    outerEdges.length !== 1 ||
    evidence.outer_launch_edge_sha256 !== valueDigest(outerEdges[0]) ||
    evidence.driver_contract_sha256 !==
      publicationPlan.driver_contract_sha256 ||
    evidence.safe_error_code !== expectedSafeErrorCode ||
    evidence.effect_possible !==
      (evidence.delivery_disposition !== "conclusive-not-created") ||
    (evidence.delivery_disposition === "conclusive-not-created"
      ? evidence.child_handle_identity_sha256 !== ZERO_SHA256
      : evidence.delivery_disposition === "delivery-accepted-effect-possible"
        ? !nonzeroSha256(evidence.child_handle_identity_sha256)
        : !sha256OrZero(evidence.child_handle_identity_sha256)) ||
    !nonzeroSha256(evidence.observation_sha256) ||
    eventsOfKind(history, "delivery-result").length !== 0
  ) {
    fail("live provider effect delivery result was refused", 69);
  }
}

function validateLaunchEdge(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    LAUNCH_EDGE_FIELDS,
    "live provider effect launch edge",
  );
  const attempt = uniqueEvent(history, "start-attempt");
  const namedRole = COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(
    evidence.child_role,
  );
  const selectedKey = publicKey(
    evidence.child_public_key_spki_der_base64,
    "live provider effect launch key",
  );
  const expectedRole = namedRole
    ? publicationPlan.planned_role_contracts.find(
        (entry) => entry.role === evidence.child_role,
      )
    : undefined;
  if (
    !namedRole ||
    !Number.isSafeInteger(evidence.depth) ||
    evidence.depth < 0 ||
    evidence.depth > MAX_CAUSAL_DEPTH ||
    !Number.isSafeInteger(evidence.edge_sequence) ||
    evidence.edge_sequence !== eventsOfKind(history, "launch-edge").length ||
    !nonzeroSha256(evidence.child_node_identity_sha256) ||
    !nonzeroSha256(evidence.parent_node_identity_sha256) ||
    evidence.child_node_identity_sha256 ===
      evidence.parent_node_identity_sha256 ||
    evidence.start_attempt_sha256 !== valueDigest(attempt) ||
    (namedRole && evidence.depth !== ROLE_DEPTHS[evidence.child_role]) ||
    (namedRole &&
      liveProviderEffectDigest(selectedKey.der) !==
        expectedRole?.public_key_spki_sha256) ||
    eventsOfKind(history, "launch-edge").some(
      (event) =>
        event.evidence.child_node_identity_sha256 ===
          evidence.child_node_identity_sha256 ||
        event.evidence.child_public_key_spki_der_base64 ===
          evidence.child_public_key_spki_der_base64 ||
        (namedRole && event.evidence.child_role === evidence.child_role),
    ) ||
    eventsOfKind(history, "launch-edge").length >= MAX_CAUSAL_EDGES
  ) {
    fail("live provider effect launch edge was refused", 69);
  }
  if (evidence.child_role === "outer") {
    if (
      evidence.parent_node_identity_sha256 !==
      uniqueEvent(history, "start-authority").evidence.current_owner_instance_sha256
    ) {
      fail("live provider effect outer launch edge was refused", 69);
    }
  } else {
    const parent = eventsOfKind(history, "role-identity").find(
      (event) =>
        event.evidence.node_identity_sha256 ===
        evidence.parent_node_identity_sha256,
    );
    if (
      parent === undefined ||
      evidence.depth !==
        eventsOfKind(history, "launch-edge").find(
          (event) =>
            event.evidence.child_node_identity_sha256 ===
            parent.evidence.node_identity_sha256,
        )?.evidence.depth +
          1 ||
      (namedRole && parent.evidence.role !== ROLE_PARENTS[evidence.child_role])
    ) {
      fail("live provider effect launch parent was refused", 69);
    }
  }
}

function validateRoleIdentity(
  evidence,
  {
    eventSequence,
    history,
    parentSha256,
    publicationPlan,
    publisherAuthoritySha256,
    witness,
  },
) {
  exactKeys(
    evidence,
    ROLE_IDENTITY_FIELDS,
    "live provider effect role identity",
  );
  const edge = findEvent(history, evidence.launch_edge_sha256, "launch-edge");
  const delivery = uniqueEvent(history, "delivery-result");
  if (edge === undefined) {
    fail("live provider effect role launch edge was refused", 69);
  }
  const namedRole = COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(evidence.role);
  const roleContract = namedRole
    ? publicationPlan.planned_role_contracts.find(
        (entry) => entry.role === evidence.role,
      )
    : undefined;
  const { der } = publicKey(
    evidence.public_key_spki_der_base64,
    "live provider effect role public key",
  );
  const identityHashes = [
    evidence.argv_sha256,
    evidence.boot_identity_sha256,
    evidence.challenge_sha256,
    evidence.cwd_identity_sha256,
    evidence.environment_sha256,
    evidence.executable_sha256,
    evidence.node_identity_sha256,
    evidence.parent_node_identity_sha256,
    evidence.pgid_identity_sha256,
    evidence.pid_identity_sha256,
    evidence.session_identity_sha256,
    evidence.start_identity_sha256,
    evidence.toolchain_sha256,
  ];
  if (
    !namedRole ||
    delivery.evidence.effect_possible !== true ||
    edge.evidence.child_role !== evidence.role ||
    edge.evidence.child_node_identity_sha256 !==
      evidence.node_identity_sha256 ||
    edge.evidence.parent_node_identity_sha256 !==
      evidence.parent_node_identity_sha256 ||
    edge.evidence.child_public_key_spki_der_base64 !==
      evidence.public_key_spki_der_base64 ||
    identityHashes.some((value) => !nonzeroSha256(value)) ||
    new Set([
      evidence.node_identity_sha256,
      evidence.parent_node_identity_sha256,
      evidence.pid_identity_sha256,
      evidence.start_identity_sha256,
    ]).size !== 4 ||
    !decimal(evidence.uid) ||
    (evidence.role === "outer" &&
      (evidence.detachment !== "foreground-state-owner-child" ||
        evidence.parent_before_detachment_sha256 !==
          evidence.parent_node_identity_sha256 ||
        evidence.parent_after_detachment_sha256 !==
          evidence.parent_node_identity_sha256 ||
        evidence.detachment_observation_sha256 !== ZERO_SHA256)) ||
    (evidence.role === "hostagent" &&
      (evidence.detachment !== "detached-reparented" ||
        evidence.parent_before_detachment_sha256 !==
          evidence.parent_node_identity_sha256 ||
        !nonzeroSha256(evidence.parent_after_detachment_sha256) ||
        evidence.parent_after_detachment_sha256 ===
          evidence.parent_before_detachment_sha256 ||
        !nonzeroSha256(evidence.detachment_observation_sha256))) ||
    (!new Set(["outer", "hostagent"]).has(evidence.role) &&
      (evidence.detachment !== "parent-bound" ||
        evidence.parent_before_detachment_sha256 !==
          evidence.parent_node_identity_sha256 ||
        evidence.parent_after_detachment_sha256 !==
          evidence.parent_node_identity_sha256 ||
        evidence.detachment_observation_sha256 !== ZERO_SHA256)) ||
    (namedRole &&
      evidence.challenge_sha256 !== roleContract?.role_challenge_sha256) ||
    (namedRole && evidence.argv_sha256 !== roleContract?.argv_sha256) ||
    (namedRole &&
      evidence.cwd_identity_sha256 !== roleContract?.cwd_identity_sha256) ||
    (namedRole &&
      evidence.environment_sha256 !== roleContract?.environment_sha256) ||
    (namedRole &&
      evidence.executable_sha256 !== roleContract?.executable_sha256) ||
    (namedRole && evidence.toolchain_sha256 !== roleContract?.toolchain_sha256) ||
    (namedRole && evidence.uid !== roleContract?.uid) ||
    evidence.uid !== publicationPlan.provider_root_identity.uid ||
    (namedRole &&
      liveProviderEffectDigest(der) !== roleContract?.public_key_spki_sha256) ||
    eventsOfKind(history, "role-identity").some(
      (event) =>
        event.evidence.node_identity_sha256 ===
          evidence.node_identity_sha256 ||
        event.evidence.challenge_sha256 === evidence.challenge_sha256 ||
        event.evidence.pid_identity_sha256 === evidence.pid_identity_sha256 ||
        event.evidence.start_identity_sha256 ===
          evidence.start_identity_sha256 ||
        (namedRole && event.evidence.role === evidence.role),
    ) ||
    eventsOfKind(history, "role-identity").length >= MAX_CAUSAL_NODES
  ) {
    fail("live provider effect role identity was refused", 69);
  }
  validateProof({
    domain: "role-identity",
    eventSequence,
    evidence,
    parentSha256,
    publicationPlan,
    publicKeyDerBase64: evidence.public_key_spki_der_base64,
    publisherAuthoritySha256,
    witness,
  });
}

function validateEventPhase(eventKind, history) {
  const kinds = new Set(history.map((event) => event.event_kind));
  if (
    (kinds.has("quiescence-fence") &&
      new Set([
        "delivery-result",
        "endpoint",
        "launch-edge",
        "role-identity",
        "start-attempt",
        "start-authority",
      ]).has(eventKind)) ||
    (kinds.has("recursive-inventory-set") &&
      new Set([
        "quiescence-ack",
        "quiescence-fence",
        "recursive-inventory-page",
      ]).has(eventKind)) ||
    (kinds.has("create-settlement") &&
      !new Set([
        "cleanup-plan-page",
        "cleanup-plan",
        "cleanup-progress",
        "cleanup-settlement",
        "terminal-receipt",
        "completion",
      ]).has(eventKind)) ||
    (kinds.has("cleanup-plan") && eventKind === "cleanup-plan-page") ||
    (kinds.has("cleanup-settlement") &&
      !kinds.has("terminal-receipt") &&
      eventKind !== "terminal-receipt") ||
    (kinds.has("terminal-receipt") && eventKind !== "completion")
  ) {
    fail("live provider effect event phase was refused", 69);
  }
  const delivery = history.find(
    (event) => event.event_kind === "delivery-result",
  );
  if (
    delivery?.evidence.delivery_disposition === "conclusive-not-created" &&
    new Set([
      "endpoint",
      "launch-edge",
      "quiescence-ack",
      "quiescence-fence",
      "recursive-inventory-page",
      "recursive-inventory-set",
      "role-identity",
    ]).has(eventKind)
  ) {
    fail("live provider effect conclusive delivery phase was refused", 69);
  }
}

function validateEndpoint(
  evidence,
  {
    eventSequence,
    history,
    parentSha256,
    publicationPlan,
    publisherAuthoritySha256,
    witness,
  },
) {
  exactKeys(evidence, ENDPOINT_FIELDS, "live provider effect endpoint");
  const roleEvent = findEvent(
    history,
    evidence.role_identity_sha256,
    "role-identity",
  );
  const planned = publicationPlan.planned_endpoint_contracts.find(
    (entry) => entry.endpoint_kind === evidence.endpoint_kind,
  );
  const engine = evidence.endpoint_kind === "engine-api";
  const hostagent = eventsOfKind(history, "role-identity").find(
    (event) => event.evidence.role === "hostagent",
  );
  const sameKind = eventsOfKind(history, "endpoint").filter(
    (event) => event.evidence.endpoint_kind === evidence.endpoint_kind,
  );
  const endpointHistory = eventsOfKind(history, "endpoint");
  const endpointIndex = COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.indexOf(
    evidence.endpoint_kind,
  );
  const expectedPhase =
    endpointHistory.length < COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length
      ? "initial"
      : "post-detachment-final";
  const expectedEndpointKind =
    COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS[
      endpointHistory.length % COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length
    ];
  const initialEndpoints = endpointHistory.filter(
    (event) => event.evidence.phase === "initial",
  );
  const initialEndpointSetSha256 =
    evidence.phase === "post-detachment-final"
      ? valueDigest(initialEndpoints.map((event) => valueDigest(event)))
      : ZERO_SHA256;
  const expectedChallengeCommitment =
    evidence.phase === "initial"
      ? planned?.initial_challenge_commitment_sha256
      : evidence.phase === "post-detachment-final"
        ? planned?.final_challenge_commitment_sha256
        : undefined;
  const expectedContextBindingSha256 = engine
    ? valueDigest({
        docker_context_namespace_role:
          evidence.docker_context_namespace_role,
        docker_context_path_identity_hmac_sha256:
          evidence.docker_context_path_identity_hmac_sha256,
        docker_context_sha256: evidence.docker_context_sha256,
        path_identity_hmac_sha256: evidence.path_identity_hmac_sha256,
        socket_identity_hmac_sha256: evidence.socket_identity_hmac_sha256,
      })
    : ZERO_SHA256;
  if (
    !COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.includes(evidence.endpoint_kind) ||
    endpointIndex < 0 ||
    evidence.endpoint_kind !== expectedEndpointKind ||
    evidence.phase !== expectedPhase ||
    roleEvent === undefined ||
    roleEvent.evidence.role !== ENDPOINT_ROLES[evidence.endpoint_kind] ||
    planned?.role !== roleEvent.evidence.role ||
    evidence.namespace_role !== planned?.namespace_role ||
    evidence.namespace_role !== ENDPOINT_NAMESPACES[evidence.endpoint_kind] ||
    evidence.initial_endpoint_set_sha256 !== initialEndpointSetSha256 ||
    (evidence.phase === "post-detachment-final" &&
      initialEndpoints.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length) ||
    evidence.path_identity_hmac_sha256 !==
      planned?.path_identity_hmac_sha256 ||
    !nonzeroSha256(evidence.challenge_sha256) ||
    expectedChallengeCommitment !==
      valueDigest({
        challenge_sha256: evidence.challenge_sha256,
        endpoint_kind: evidence.endpoint_kind,
        phase: evidence.phase,
      }) ||
    !nonzeroSha256(evidence.connection_observation_sha256) ||
    eventsOfKind(history, "endpoint").some(
      (event) =>
        event.evidence.challenge_sha256 === evidence.challenge_sha256 ||
        event.evidence.connection_observation_sha256 ===
          evidence.connection_observation_sha256,
    ) ||
    hostagent === undefined ||
    evidence.hostagent_detachment_observation_sha256 !==
      hostagent.evidence.detachment_observation_sha256 ||
    !nonzeroSha256(evidence.socket_identity_hmac_sha256) ||
    eventsOfKind(history, "endpoint").some(
      (event) =>
        event.evidence.endpoint_kind !== evidence.endpoint_kind &&
        event.evidence.socket_identity_hmac_sha256 ===
          evidence.socket_identity_hmac_sha256,
    ) ||
    evidence.planned_quiescence_fence_sha256 !==
      publicationPlan.planned_quiescence_fence_sha256 ||
    evidence.ambient_context_authorized !== false ||
    evidence.activation_authorized !== false ||
    evidence.docker_context_namespace_role !==
      planned?.docker_context_namespace_role ||
    evidence.docker_context_path_identity_hmac_sha256 !==
      planned?.docker_context_path_identity_hmac_sha256 ||
    evidence.docker_context_sha256 !== planned?.docker_context_sha256 ||
    evidence.docker_context_binding_sha256 !== expectedContextBindingSha256 ||
    (engine &&
      evidence.private_engine_socket_identity_hmac_sha256 !==
        evidence.socket_identity_hmac_sha256) ||
    (!engine && evidence.docker_context_sha256 !== ZERO_SHA256) ||
    (!engine && evidence.docker_context_namespace_role !== "none") ||
    (!engine &&
      evidence.docker_context_path_identity_hmac_sha256 !== ZERO_SHA256) ||
    (!engine &&
      evidence.private_engine_socket_identity_hmac_sha256 !== ZERO_SHA256) ||
    (evidence.phase === "initial" &&
      (sameKind.length !== 0 ||
        evidence.prior_endpoint_event_sha256 !== ZERO_SHA256)) ||
    (evidence.phase === "post-detachment-final" &&
      (sameKind.length !== 1 ||
        sameKind[0].evidence.phase !== "initial" ||
        evidence.prior_endpoint_event_sha256 !== valueDigest(sameKind[0]) ||
        evidence.socket_identity_hmac_sha256 !==
          sameKind[0].evidence.socket_identity_hmac_sha256 ||
        evidence.path_identity_hmac_sha256 !==
          sameKind[0].evidence.path_identity_hmac_sha256 ||
        evidence.challenge_sha256 === sameKind[0].evidence.challenge_sha256 ||
        evidence.connection_observation_sha256 ===
          sameKind[0].evidence.connection_observation_sha256)) ||
    !new Set(["initial", "post-detachment-final"]).has(evidence.phase) ||
    eventsOfKind(history, "endpoint").length >=
      COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length * 2
  ) {
    fail("live provider effect endpoint was refused", 69);
  }
  validateProof({
    domain: "endpoint",
    eventSequence,
    evidence,
    parentSha256,
    publicationPlan,
    publicKeyDerBase64: roleEvent.evidence.public_key_spki_der_base64,
    publisherAuthoritySha256,
    witness,
  });
}

function validateQuiescenceFence(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    QUIESCENCE_FENCE_FIELDS,
    "live provider effect quiescence fence",
  );
  const authority = uniqueEvent(history, "start-authority");
  const attempt = uniqueEvent(history, "start-attempt");
  const namedRoles = eventsOfKind(history, "role-identity").filter((event) =>
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(event.evidence.role),
  );
  const launchEdges = eventsOfKind(history, "launch-edge");
  const endpointHistory = eventsOfKind(history, "endpoint");
  const endpoints = endpointHistory.filter(
    (event) => event.evidence.phase === "post-detachment-final",
  );
  const delivery = uniqueEvent(history, "delivery-result");
  const graphFrontierSha256 = valueDigest({
    endpoints: endpointHistory.map((event) => valueDigest(event)),
    launch_edges: eventsOfKind(history, "launch-edge").map((event) =>
      valueDigest(event),
    ),
    role_identities: eventsOfKind(history, "role-identity").map((event) =>
      valueDigest(event),
    ),
  });
  exactKeys(
    evidence.process_reconciliation,
    PROCESS_RECONCILIATION_FIELDS,
    "live provider effect process reconciliation",
  );
  const expectedObservedNodesSha256 = valueDigest(
    launchEdges.map((event) => event.evidence.child_node_identity_sha256),
  );
  const expectedObservedEdgesSha256 = valueDigest(
    launchEdges.map((event) => ({
      child_node_identity_sha256: event.evidence.child_node_identity_sha256,
      depth: event.evidence.depth,
      parent_node_identity_sha256: event.evidence.parent_node_identity_sha256,
    })),
  );
  if (
    evidence.authority_event_sha256 !== valueDigest(authority) ||
    evidence.start_attempt_event_sha256 !== valueDigest(attempt) ||
    evidence.fence_sha256 !==
      publicationPlan.planned_quiescence_fence_sha256 ||
    evidence.graph_frontier_sha256 !== graphFrontierSha256 ||
    evidence.process_reconciliation_sha256 !==
      valueDigest(evidence.process_reconciliation) ||
    !nonzeroSha256(
      evidence.process_reconciliation.observation_challenge_sha256,
    ) ||
    !nonzeroSha256(evidence.process_reconciliation.process_observation_sha256) ||
    evidence.process_reconciliation.observation_challenge_sha256 ===
      evidence.process_reconciliation.process_observation_sha256 ||
    evidence.process_reconciliation.observed_process_count !==
      launchEdges.length ||
    evidence.process_reconciliation.observed_node_set_sha256 !==
      expectedObservedNodesSha256 ||
    evidence.process_reconciliation.observed_edge_set_sha256 !==
      expectedObservedEdgesSha256 ||
    delivery.evidence.effect_possible !== true ||
    !Number.isSafeInteger(evidence.deadline_milliseconds) ||
    evidence.deadline_milliseconds <= 0 ||
    evidence.deadline_milliseconds > MAX_EFFECT_MILLISECONDS ||
    namedRoles.length !== COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length ||
    launchEdges.length !== COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length ||
    namedRoles.some(
      (identity) =>
        findEvent(
          history,
          identity.evidence.launch_edge_sha256,
          "launch-edge",
        ) === undefined,
    ) ||
    new Set(namedRoles.map((event) => event.evidence.role)).size !==
      COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length ||
    endpoints.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length ||
    endpointHistory.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length * 2 ||
    new Set(endpoints.map((event) => event.evidence.endpoint_kind)).size !==
      COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length ||
    eventsOfKind(history, "quiescence-fence").length !== 0
  ) {
    fail("live provider effect quiescence fence was refused", 69);
  }
}

function validateQuiescenceAcknowledgement(
  evidence,
  {
    eventSequence,
    history,
    parentSha256,
    publicationPlan,
    publisherAuthoritySha256,
    witness,
  },
) {
  exactKeys(
    evidence,
    QUIESCENCE_ACK_FIELDS,
    "live provider effect quiescence acknowledgement",
  );
  const fence = uniqueEvent(history, "quiescence-fence");
  const roleEvent = findEvent(
    history,
    evidence.role_identity_sha256,
    "role-identity",
  );
  const acknowledgements = eventsOfKind(history, "quiescence-ack");
  const expectedRole =
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES[acknowledgements.length];
  const outgoingEdgeSetSha256 = valueDigest(
    eventsOfKind(history, "launch-edge")
      .filter(
        (event) =>
          event.evidence.parent_node_identity_sha256 ===
          roleEvent?.evidence.node_identity_sha256,
      )
      .map((event) => valueDigest(event)),
  );
  const endpointFrontierSha256 = valueDigest(
    eventsOfKind(history, "endpoint")
      .filter(
        (event) =>
          event.evidence.role_identity_sha256 === valueDigest(roleEvent),
      )
      .map((event) => valueDigest(event)),
  );
  if (
    !COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(evidence.role) ||
    evidence.role !== expectedRole ||
    roleEvent === undefined ||
    roleEvent.evidence.role !== evidence.role ||
    evidence.fence_event_sha256 !== valueDigest(fence) ||
    evidence.disposition !==
      "no-new-descendants-endpoints-or-resources" ||
    evidence.outgoing_edge_set_sha256 !== outgoingEdgeSetSha256 ||
    evidence.endpoint_frontier_sha256 !== endpointFrontierSha256 ||
    !nonzeroSha256(evidence.resource_frontier_observation_sha256) ||
    acknowledgements.some(
      (event) =>
        event.evidence.resource_frontier_observation_sha256 ===
        evidence.resource_frontier_observation_sha256,
    ) ||
    evidence.acknowledgement_sha256 !==
      valueDigest({
        disposition: evidence.disposition,
        endpoint_frontier_sha256: evidence.endpoint_frontier_sha256,
        fence_event_sha256: evidence.fence_event_sha256,
        outgoing_edge_set_sha256: evidence.outgoing_edge_set_sha256,
        resource_frontier_observation_sha256:
          evidence.resource_frontier_observation_sha256,
        role_identity_sha256: evidence.role_identity_sha256,
      }) ||
    acknowledgements.some(
      (event) =>
        event.evidence.role_identity_sha256 ===
        evidence.role_identity_sha256,
    ) ||
    acknowledgements.length >=
      MAX_CAUSAL_NODES
  ) {
    fail("live provider effect quiescence acknowledgement was refused", 69);
  }
  validateProof({
    domain: "quiescence-ack",
    eventSequence,
    evidence,
    parentSha256,
    publicationPlan,
    publicKeyDerBase64: roleEvent.evidence.public_key_spki_der_base64,
    publisherAuthoritySha256,
    witness,
  });
}

function validateInventoryEntry(entry, namespaceBinding, witness) {
  exactKeys(
    entry,
    INVENTORY_ENTRY_FIELDS,
    "live provider effect inventory entry",
  );
  const size = decimal(entry.size, { maximum: BigInt(MAX_PROVIDER_RESOURCE_BYTES) })
    ? BigInt(entry.size)
    : -1n;
  const links = decimal(entry.links, {
    maximum: BigInt(MAX_INVENTORY_ENTRIES + 2),
    positive: true,
  })
    ? BigInt(entry.links)
    : -1n;
  const expectedOwnership =
    entry.depth === 0
      ? "namespace-root"
      : namespaceBinding.baseline_relative_identity_hmac_sha256.includes(
            entry.relative_identity_hmac_sha256,
          )
        ? "admitted-baseline"
        : "generation-owned";
  const expectedOwnershipBindingSha256 = valueDigest({
    baseline_set_binding_sha256: namespaceBinding.baseline_set_binding_sha256,
    namespace_identity_hmac_sha256:
      namespaceBinding.namespace_identity_hmac_sha256,
    ownership: expectedOwnership,
    relative_identity_hmac_sha256: entry.relative_identity_hmac_sha256,
    witness_sha256:
      expectedOwnership === "generation-owned"
        ? valueDigest(witness)
        : ZERO_SHA256,
  });
  if (
    !INVENTORY_TYPES.has(entry.type) ||
    !Number.isSafeInteger(entry.depth) ||
    entry.depth < 0 ||
    entry.depth > MAX_RECURSIVE_DEPTH ||
    entry.device !== namespaceBinding.device ||
    !decimal(entry.inode, { positive: true }) ||
    !decimal(entry.uid, { maximum: U32_MAX }) ||
    entry.uid !== namespaceBinding.uid ||
    !decimal(entry.ctime_nanoseconds, { positive: true }) ||
    !decimal(entry.mtime_nanoseconds, { positive: true }) ||
    typeof entry.mode !== "string" ||
    !/^0[0-7]{3}$/u.test(entry.mode) ||
    size < 0n ||
    links < 1n ||
    !Number.isSafeInteger(entry.name_bytes) ||
    entry.name_bytes <= 0 ||
    entry.name_bytes > MAX_ENTRY_NAME_BYTES ||
    !Number.isSafeInteger(entry.directory_entry_count) ||
    entry.directory_entry_count < 0 ||
    entry.directory_entry_count > MAX_DIRECTORY_ENTRIES ||
    (entry.type !== "directory" && entry.directory_entry_count !== 0) ||
    !nonzeroSha256(entry.relative_identity_hmac_sha256) ||
    (entry.depth === 0
      ? entry.parent_identity_hmac_sha256 !== ZERO_SHA256
      : !nonzeroSha256(entry.parent_identity_hmac_sha256)) ||
    (entry.type === "regular" &&
      links === 1n &&
      entry.hard_link_group_sha256 !== ZERO_SHA256) ||
    (entry.type === "regular" &&
      links > 1n &&
      !nonzeroSha256(entry.hard_link_group_sha256)) ||
    (entry.type !== "regular" &&
      entry.hard_link_group_sha256 !== ZERO_SHA256) ||
    (new Set(["socket", "symlink"]).has(entry.type) && links !== 1n)
    || entry.ownership !== expectedOwnership ||
    entry.ownership_binding_sha256 !== expectedOwnershipBindingSha256
  ) {
    fail("live provider effect inventory entry was refused", 69);
  }
  switch (entry.type) {
    case "directory":
      if (
        entry.content_disposition !== "metadata-only" ||
        entry.resource_kind !== "none" ||
        entry.directory_entry_count > MAX_DIRECTORY_ENTRIES ||
        entry.content_sha256 !== ZERO_SHA256 ||
        entry.symlink_target_sha256 !== ZERO_SHA256 ||
        entry.endpoint_sha256 !== ZERO_SHA256
      ) {
        fail("live provider effect directory inventory was refused", 69);
      }
      break;
    case "regular":
      if (
        links > BigInt(MAX_INVENTORY_ENTRIES) ||
        entry.symlink_target_sha256 !== ZERO_SHA256 ||
        !(
          (entry.content_disposition === "hashed" &&
            size <= BigInt(MAX_HASHED_FILE_BYTES) &&
            nonzeroSha256(entry.content_sha256) &&
            ((entry.resource_kind === "none" &&
              entry.endpoint_sha256 === ZERO_SHA256) ||
              (entry.resource_kind === "docker-context" &&
                nonzeroSha256(entry.endpoint_sha256)))) ||
          (entry.content_disposition ===
            "metadata-only-closed-provider-resource" &&
            size <= BigInt(MAX_PROVIDER_RESOURCE_BYTES) &&
            entry.content_sha256 === ZERO_SHA256 &&
            entry.resource_kind === "closed-disk-image" &&
            entry.endpoint_sha256 === ZERO_SHA256)
        )
      ) {
        fail("live provider effect regular inventory was refused", 69);
      }
      break;
    case "symlink":
      if (
        entry.content_disposition !== "target-hashed-no-follow" ||
        entry.resource_kind !== "none" ||
        entry.content_sha256 !== ZERO_SHA256 ||
        !nonzeroSha256(entry.symlink_target_sha256) ||
        entry.endpoint_sha256 !== ZERO_SHA256 ||
        size > BigInt(MAX_SYMLINK_TARGET_BYTES)
      ) {
        fail("live provider effect symlink inventory was refused", 69);
      }
      break;
    case "socket":
      if (
        entry.content_disposition !== "endpoint-bound" ||
        entry.resource_kind !== "none" ||
        entry.content_sha256 !== ZERO_SHA256 ||
        entry.symlink_target_sha256 !== ZERO_SHA256 ||
        !nonzeroSha256(entry.endpoint_sha256)
      ) {
        fail("live provider effect socket inventory was refused", 69);
      }
      break;
    default:
      fail("live provider effect inventory type was refused", 69);
  }
  if (expectedOwnership === "admitted-baseline") {
    const descriptor = namespaceBinding.baseline_descriptors.find(
      (candidate) =>
        candidate.relative_identity_hmac_sha256 ===
        entry.relative_identity_hmac_sha256,
    );
    if (
      descriptor === undefined ||
      descriptor.descriptor_sha256 !==
        valueDigest(baselineDescriptorProjection(entry)) ||
      entry.endpoint_sha256 !== ZERO_SHA256 ||
      entry.hard_link_group_sha256 !== ZERO_SHA256 ||
      entry.resource_kind !== "none"
    ) {
      fail("live provider effect admitted baseline descriptor was refused", 73);
    }
  }
}

function inventoryPages(history, namespaceRole, sampleSequence) {
  return eventsOfKind(history, "recursive-inventory-page").filter(
    (event) =>
      event.evidence.namespace_role === namespaceRole &&
      event.evidence.sample_sequence === sampleSequence,
  );
}

function inventoryPageContentSha256(page) {
  return valueDigest({
    entries: page.evidence.entries,
    entry_start: page.evidence.entry_start,
    namespace_role: page.evidence.namespace_role,
    page_sequence: page.evidence.page_sequence,
  });
}

function inventorySampleSha256(pages) {
  return valueDigest({
    page_count: pages.length,
    page_sha256: pages.map(inventoryPageContentSha256),
  });
}

function validateInventoryPage(evidence, { history, publicationPlan, witness }) {
  exactKeys(
    evidence,
    INVENTORY_PAGE_FIELDS,
    "live provider effect inventory page",
  );
  const namespaceIndex = COLIMA_LIVE_MUTATION_SURFACE_ROLES.indexOf(
    evidence.namespace_role,
  );
  const namespaceBinding = publicationPlan.namespace_bindings[namespaceIndex];
  const pages = inventoryPages(
    history,
    evidence.namespace_role,
    evidence.sample_sequence,
  );
  const allPages = eventsOfKind(history, "recursive-inventory-page");
  const priorEntries = pages.flatMap((page) => page.evidence.entries);
  const priorSampleEntries = allPages
    .filter(
      (page) =>
        page.evidence.sample_sequence === evidence.sample_sequence,
    )
    .flatMap((page) => page.evidence.entries);
  const groupSequence =
    evidence.sample_sequence * COLIMA_LIVE_MUTATION_SURFACE_ROLES.length +
    namespaceIndex;
  const priorPage = allPages.at(-1);
  const priorNamespaceIndex = priorPage
    ? COLIMA_LIVE_MUTATION_SURFACE_ROLES.indexOf(
        priorPage.evidence.namespace_role,
      )
    : -1;
  const priorGroupSequence = priorPage
    ? priorPage.evidence.sample_sequence *
        COLIMA_LIVE_MUTATION_SURFACE_ROLES.length +
      priorNamespaceIndex
    : -1;
  if (
    namespaceIndex < 0 ||
    evidence.sample_sequence !== 0 && evidence.sample_sequence !== 1 ||
    (priorPage === undefined
      ? groupSequence !== 0
      : groupSequence !== priorGroupSequence &&
        groupSequence !== priorGroupSequence + 1) ||
    (priorPage !== undefined &&
      groupSequence === priorGroupSequence &&
      priorPage.evidence.entries.length !==
        MAX_INVENTORY_ENTRIES_PER_PAGE) ||
    !Number.isSafeInteger(evidence.page_sequence) ||
    evidence.page_sequence !== pages.length ||
    evidence.page_sequence >= MAX_INVENTORY_PAGES_PER_NAMESPACE ||
    !Number.isSafeInteger(evidence.entry_start) ||
    evidence.entry_start < 0 ||
    !Array.isArray(evidence.entries) ||
    evidence.entries.length === 0 ||
    evidence.entries.length > MAX_INVENTORY_ENTRIES_PER_PAGE ||
    priorSampleEntries.length + evidence.entries.length >
      MAX_INVENTORY_ENTRIES ||
    eventsOfKind(history, "quiescence-ack").length !==
      eventsOfKind(history, "role-identity").length ||
    !eventsOfKind(history, "role-identity").every((identity) =>
      eventsOfKind(history, "quiescence-ack").some(
        (acknowledgement) =>
          acknowledgement.evidence.role_identity_sha256 ===
          valueDigest(identity),
      ),
    )
  ) {
    fail("live provider effect inventory page was refused", 69);
  }
  if (
    priorPage !== undefined &&
    groupSequence === priorGroupSequence + 1
  ) {
    const completedNamespaceIndex =
      COLIMA_LIVE_MUTATION_SURFACE_ROLES.indexOf(
        priorPage.evidence.namespace_role,
      );
    validateCompletedInventoryGroup(
      sampleEntries(
        history,
        priorPage.evidence.namespace_role,
        priorPage.evidence.sample_sequence,
      ),
      publicationPlan.namespace_bindings[completedNamespaceIndex],
      priorPage.evidence.sample_sequence,
      history,
    );
    if (groupSequence === COLIMA_LIVE_MUTATION_SURFACE_ROLES.length) {
      validateCompletedInventorySample(history, publicationPlan, 0);
    }
  }
  if (
    evidence.entry_start !== priorEntries.length ||
    evidence.parent_page_sha256 !==
      (pages.length === 0 ? ZERO_SHA256 : valueDigest(pages.at(-1)))
  ) {
    fail("live provider effect inventory page chain was refused", 69);
  }
  for (const [index, entry] of evidence.entries.entries()) {
    validateInventoryEntry(entry, namespaceBinding, witness);
    if (entry.entry_sequence !== evidence.entry_start + index) {
      fail("live provider effect inventory ordering was refused", 69);
    }
    if (
      entry.type === "socket" &&
      findEvent(history, entry.endpoint_sha256, "endpoint")?.evidence.phase !==
        "post-detachment-final"
    ) {
      fail("live provider effect inventory endpoint was refused", 69);
    }
  }
  const entries = [...priorEntries, ...evidence.entries];
  const prefix = validateInventorySamplePrefix(entries, namespaceBinding);
  const maximumNamespaceEntries =
    MAX_INVENTORY_PAGES_PER_NAMESPACE * MAX_INVENTORY_ENTRIES_PER_PAGE;
  let sampleZeroPrefix;
  let unseenCurrentReservations = 0;
  let sampleZeroRequiredCurrentEntries = 0;
  if (evidence.sample_sequence === 0) {
    sampleZeroPrefix = validateInventorySampleZeroPrefix(
      [
        ...allPages
          .filter((page) => page.evidence.sample_sequence === 0)
          .flatMap((page) =>
            page.evidence.entries.map((entry) => ({
              entry,
              namespaceRole: page.evidence.namespace_role,
            })),
          ),
        ...evidence.entries.map((entry) => ({
          entry,
          namespaceRole: evidence.namespace_role,
        })),
      ],
      history,
      publicationPlan,
    );
    unseenCurrentReservations =
      sampleZeroPrefix.unseenByNamespace.get(evidence.namespace_role);
    const missingCurrentHardLinkAliases =
      sampleZeroPrefix.missingHardLinkAliasesByNamespace.find(
        ({ namespaceRole }) => namespaceRole === evidence.namespace_role,
      )?.count ?? 0;
    if (
      missingCurrentHardLinkAliases !==
      sampleZeroPrefix.missingHardLinkAliases
    ) {
      fail("live provider effect hard-link inventory was incomplete", 69);
    }
    const baselineResidual = Math.max(
      0,
      prefix.missingBaseline - prefix.reachableBaselineOnlyEntries,
    );
    const requiredGenerationLeaves =
      unseenCurrentReservations + missingCurrentHardLinkAliases;
    const generationResidual = Math.max(
      0,
      requiredGenerationLeaves - prefix.reachableGenerationOnlyEntries,
    );
    const requiredBaselineRootSlots =
      baselineResidual === 0
        ? 0
        : Math.ceil(baselineResidual / prefix.rootSubtreeCapacityPerSlot);
    const requiredGenerationRootSlots =
      generationResidual === 0
        ? 0
        : Math.ceil(generationResidual / prefix.rootSubtreeCapacityPerSlot);
    if (
      prefix.baselineOnlyOutstandingChildren > prefix.missingBaseline ||
      requiredBaselineRootSlots + requiredGenerationRootSlots >
        prefix.rootOutstandingChildren
    ) {
      fail(
        missingCurrentHardLinkAliases > 0
          ? "live provider effect hard-link inventory was unreachable"
          : "live provider effect inventory ownership was unreachable",
        69,
      );
    }
    const remainingRootSlots =
      prefix.rootOutstandingChildren - requiredBaselineRootSlots;
    const generationSlotGroups = [...prefix.generationOnlySlotGroups];
    if (remainingRootSlots > 0) {
      generationSlotGroups.push(
        Object.freeze({
          count: remainingRootSlots,
          remaining_depth: MAX_RECURSIVE_DEPTH,
        }),
      );
    }
    const requiredGenerationEntries = minimumGenerationForestEntries({
      branchingFactor: MAX_DIRECTORY_ENTRIES,
      maximumDirectoryNodes: MAX_INVENTORY_ENTRIES,
      requiredLeaves: requiredGenerationLeaves,
      slotGroups: generationSlotGroups,
    });
    if (!Number.isFinite(requiredGenerationEntries)) {
      fail(
        missingCurrentHardLinkAliases > 0
          ? "live provider effect hard-link inventory was unreachable"
          : "live provider effect inventory ownership was unreachable",
        69,
      );
    }
    sampleZeroRequiredCurrentEntries =
      prefix.missingBaseline + requiredGenerationEntries;
  }
  let requiredCurrentEntries = sampleZeroRequiredCurrentEntries;
  if (prefix.missingBaseline > prefix.reachableBaselineEntries) {
    fail("live provider effect inventory baseline was unreachable", 69);
  }
  if (evidence.sample_sequence === 1) {
    const firstSample = sampleEntries(history, evidence.namespace_role, 0);
    if (
      entries.length > firstSample.length ||
      evidence.entries.some(
        (entry, index) =>
          !liveProviderEffectBytes(entry).equals(
            liveProviderEffectBytes(
              firstSample[evidence.entry_start + index],
            ),
          ),
      )
    ) {
      fail("live provider effect inventory samples differed", 73);
    }
    requiredCurrentEntries = firstSample.length - entries.length;
  }
  const futureNamespaceEntries = COLIMA_LIVE_MUTATION_SURFACE_ROLES.slice(
    namespaceIndex + 1,
  ).reduce((total, role, offset) => {
    if (evidence.sample_sequence === 1) {
      return total + sampleEntries(history, role, 0).length;
    }
    const binding = publicationPlan.namespace_bindings[
      namespaceIndex + offset + 1
    ];
    return (
      total +
      1 +
      binding.baseline_relative_identity_hmac_sha256.length +
      sampleZeroPrefix.unseenByNamespace.get(role)
    );
  }, 0);
  if (
    entries.length + requiredCurrentEntries > maximumNamespaceEntries ||
    priorSampleEntries.length +
        evidence.entries.length +
        requiredCurrentEntries +
        futureNamespaceEntries >
      MAX_INVENTORY_ENTRIES
  ) {
    fail("live provider effect inventory completion was impossible", 69);
  }
  const groupIsComplete =
    prefix.outstandingChildren === 0 &&
    prefix.missingBaseline === 0 &&
    unseenCurrentReservations === 0;
  if (
    evidence.entries.length < MAX_INVENTORY_ENTRIES_PER_PAGE ||
    evidence.page_sequence === MAX_INVENTORY_PAGES_PER_NAMESPACE - 1 ||
    priorSampleEntries.length + evidence.entries.length ===
      MAX_INVENTORY_ENTRIES ||
    groupIsComplete
  ) {
    validateCompletedInventoryGroup(
      entries,
      namespaceBinding,
      evidence.sample_sequence,
      history,
    );
    if (
      evidence.sample_sequence === 0 &&
      namespaceIndex === COLIMA_LIVE_MUTATION_SURFACE_ROLES.length - 1
    ) {
      validateCompletedInventorySample(history, publicationPlan, 0, {
        entries,
        namespaceRole: evidence.namespace_role,
      });
    }
  }
  const hashedBytes = [...priorSampleEntries, ...evidence.entries]
    .filter((entry) => entry.content_disposition === "hashed")
    .reduce((total, entry) => total + BigInt(entry.size), 0n);
  if (hashedBytes > BigInt(MAX_TOTAL_HASHED_BYTES)) {
    fail("live provider effect inventory hash budget was exceeded", 69);
  }
}

function validateInventorySamplePrefix(entries, namespaceBinding) {
  if (
    entries.length === 0 ||
    entries[0].type !== "directory" ||
    entries[0].depth !== 0 ||
    entries[0].entry_sequence !== 0 ||
    entries[0].relative_identity_hmac_sha256 !==
      namespaceBinding.namespace_identity_hmac_sha256 ||
    entries[0].parent_identity_hmac_sha256 !== ZERO_SHA256 ||
    entries[0].device !== namespaceBinding.device ||
    entries[0].inode !== namespaceBinding.inode ||
    entries[0].mode !== namespaceBinding.mode ||
    entries[0].uid !== namespaceBinding.uid ||
    entries.filter((entry) => entry.depth === 0).length !== 1 ||
    new Set(entries.map((entry) => entry.relative_identity_hmac_sha256)).size !==
      entries.length
  ) {
    fail("live provider effect inventory root was refused", 69);
  }
  const byIdentity = new Map();
  const directChildren = new Map();
  for (const [index, entry] of entries.entries()) {
    if (entry.entry_sequence !== index) {
      fail("live provider effect inventory sequence was refused", 69);
    }
    if (index > 0) {
      const parent = byIdentity.get(entry.parent_identity_hmac_sha256);
      if (
        parent === undefined ||
        parent.type !== "directory" ||
        entry.depth !== parent.depth + 1
      ) {
        fail("live provider effect inventory parent was refused", 69);
      }
      directChildren.set(
        parent.relative_identity_hmac_sha256,
        (directChildren.get(parent.relative_identity_hmac_sha256) ?? 0) + 1,
      );
    }
    byIdentity.set(entry.relative_identity_hmac_sha256, entry);
  }
  let outstandingChildren = 0;
  let baselineOnlyOutstandingChildren = 0;
  let generationOnlyOutstandingChildren = 0;
  const generationOnlySlotGroups = [];
  let rootOutstandingChildren = 0;
  let rootSubtreeCapacityPerSlot = 0;
  let reachableBaselineEntries = 0;
  let reachableBaselineOnlyEntries = 0;
  let reachableGenerationOnlyEntries = 0;
  const missingBaseline = new Set(
    namespaceBinding.baseline_relative_identity_hmac_sha256,
  );
  for (const entry of entries) {
    if (entry.ownership === "admitted-baseline") {
      missingBaseline.delete(entry.relative_identity_hmac_sha256);
    }
    const observedChildren =
      directChildren.get(entry.relative_identity_hmac_sha256) ?? 0;
    if (
      entry.type === "directory" &&
      entry.directory_entry_count < observedChildren
    ) {
      fail("live provider effect directory inventory was exceeded", 69);
    }
    if (entry.type === "directory") {
      const unobservedChildren =
        entry.directory_entry_count - observedChildren;
      if (unobservedChildren > 0 && entry.depth === MAX_RECURSIVE_DEPTH) {
        fail("live provider effect inventory depth was impossible", 69);
      }
      outstandingChildren += unobservedChildren;
      if (unobservedChildren > 0) {
        let subtreeEntries = 1;
        for (
          let depth = MAX_RECURSIVE_DEPTH - 1;
          depth >= entry.depth + 1 &&
          subtreeEntries < MAX_INVENTORY_ENTRIES;
          depth -= 1
        ) {
          subtreeEntries = Math.min(
            MAX_INVENTORY_ENTRIES,
            1 + MAX_DIRECTORY_ENTRIES * subtreeEntries,
          );
        }
        if (
          entry.ownership === "namespace-root" ||
          entry.ownership === "admitted-baseline"
        ) {
          reachableBaselineEntries = Math.min(
            MAX_INVENTORY_ENTRIES,
            reachableBaselineEntries + unobservedChildren * subtreeEntries,
          );
        }
        if (entry.ownership === "admitted-baseline") {
          baselineOnlyOutstandingChildren += unobservedChildren;
          reachableBaselineOnlyEntries = Math.min(
            MAX_INVENTORY_ENTRIES,
            reachableBaselineOnlyEntries +
              unobservedChildren * subtreeEntries,
          );
        }
        if (entry.ownership === "namespace-root") {
          rootOutstandingChildren += unobservedChildren;
          rootSubtreeCapacityPerSlot = subtreeEntries;
        }
        if (entry.ownership === "generation-owned") {
          reachableGenerationOnlyEntries = Math.min(
            MAX_INVENTORY_ENTRIES,
            reachableGenerationOnlyEntries +
              unobservedChildren * subtreeEntries,
          );
          generationOnlyOutstandingChildren += unobservedChildren;
          generationOnlySlotGroups.push(
            Object.freeze({
              count: unobservedChildren,
              remaining_depth: MAX_RECURSIVE_DEPTH - entry.depth,
            }),
          );
        }
      }
    }
    if (entry.ownership === "admitted-baseline") {
      let parent = byIdentity.get(entry.parent_identity_hmac_sha256);
      while (parent !== undefined) {
        if (
          parent.ownership !== "admitted-baseline" &&
          parent.ownership !== "namespace-root"
        ) {
          fail("live provider effect baseline ancestry was refused", 69);
        }
        if (parent.ownership === "namespace-root") break;
        parent = byIdentity.get(parent.parent_identity_hmac_sha256);
      }
    }
    if (entry.ownership === "generation-owned") {
      let parent = byIdentity.get(entry.parent_identity_hmac_sha256);
      while (parent !== undefined) {
        if (parent.ownership === "admitted-baseline") {
          fail("live provider effect generation ancestry was refused", 69);
        }
        if (parent.ownership === "namespace-root") break;
        parent = byIdentity.get(parent.parent_identity_hmac_sha256);
      }
    }
  }
  return Object.freeze({
    baselineOnlyOutstandingChildren,
    directChildren,
    generationOnlyOutstandingChildren,
    generationOnlySlotGroups: Object.freeze(generationOnlySlotGroups),
    missingBaseline: missingBaseline.size,
    outstandingChildren,
    reachableBaselineEntries,
    reachableBaselineOnlyEntries,
    reachableGenerationOnlyEntries,
    rootOutstandingChildren,
    rootSubtreeCapacityPerSlot,
  });
}

function validateCompleteInventorySample(entries, namespaceBinding) {
  const { directChildren } = validateInventorySamplePrefix(
    entries,
    namespaceBinding,
  );
  for (const entry of entries) {
    if (
      entry.type === "directory" &&
      entry.directory_entry_count !==
        (directChildren.get(entry.relative_identity_hmac_sha256) ?? 0)
    ) {
      fail("live provider effect directory inventory was incomplete", 69);
    }
  }
}

function validateCompletedInventoryGroup(
  entries,
  namespaceBinding,
  sampleSequence,
  history,
) {
  validateCompleteInventorySample(entries, namespaceBinding);
  if (
    validateInventoryPhysicalPrefix(
      entries.map((entry) => ({
        entry,
        namespaceRole: namespaceBinding.role,
      })),
    ).missingHardLinkAliases !== 0
  ) {
    fail("live provider effect hard-link inventory was incomplete", 69);
  }
  const expectedBaseline = [
    ...namespaceBinding.baseline_relative_identity_hmac_sha256,
  ].sort();
  const actualBaseline = entries
    .filter((entry) => entry.ownership === "admitted-baseline")
    .map((entry) => entry.relative_identity_hmac_sha256)
    .sort();
  if (
    !liveProviderEffectBytes(actualBaseline).equals(
      liveProviderEffectBytes(expectedBaseline),
    )
  ) {
    fail("live provider effect inventory baseline was incomplete", 73);
  }
  if (sampleSequence === 0) {
    const identities = new Set(
      entries.map((entry) => entry.relative_identity_hmac_sha256),
    );
    if (
      inventoryReservations(history)
        .filter(
          (reservation) =>
            reservation.namespace_role === namespaceBinding.role,
        )
        .some((reservation) => !identities.has(reservation.identity_sha256))
    ) {
      fail("live provider effect reserved inventory was incomplete", 69);
    }
  }
  if (sampleSequence === 1) {
    const firstSample = sampleEntries(
      history,
      namespaceBinding.role,
      0,
    );
    if (
      entries.length !== firstSample.length ||
      entries.some(
        (entry, index) =>
          !liveProviderEffectBytes(entry).equals(
            liveProviderEffectBytes(firstSample[index]),
          ),
      )
    ) {
      fail("live provider effect inventory samples differed", 73);
    }
  }
}

function sampleEntries(history, namespaceRole, sampleSequence) {
  const pages = inventoryPages(history, namespaceRole, sampleSequence);
  if (
    pages.length === 0 ||
    pages.some(
      (page, index) =>
        page.evidence.page_sequence !== index ||
        page.evidence.entry_start !==
          pages
            .slice(0, index)
            .reduce((total, prior) => total + prior.evidence.entries.length, 0),
    )
  ) {
    fail("live provider effect inventory sample was incomplete", 69);
  }
  return pages.flatMap((page) => page.evidence.entries);
}

function validateInventoryPhysicalPrefix(entriesByNamespace) {
  const physicalAliases = new Map();
  const hardLinkGroups = new Map();
  for (const item of entriesByNamespace) {
    const { entry } = item;
    const physicalIdentity = `${entry.device}:${entry.inode}`;
    const aliases = physicalAliases.get(physicalIdentity) ?? [];
    aliases.push(item);
    physicalAliases.set(physicalIdentity, aliases);
    if (entry.hard_link_group_sha256 !== ZERO_SHA256) {
      const group = hardLinkGroups.get(entry.hard_link_group_sha256) ?? [];
      group.push(item);
      hardLinkGroups.set(entry.hard_link_group_sha256, group);
    }
  }
  let missingHardLinkAliases = 0;
  const missingHardLinkAliasesByNamespace = new Map();
  for (const aliases of physicalAliases.values()) {
    const exemplar = aliases[0].entry;
    const exemplarNamespace = aliases[0].namespaceRole;
    if (exemplar.type !== "regular") {
      if (aliases.length !== 1) {
        fail("live provider effect physical alias inventory was refused", 69);
      }
      continue;
    }
    const expectedLinks = BigInt(exemplar.links);
    if (
      expectedLinks > BigInt(MAX_INVENTORY_ENTRIES) ||
      BigInt(aliases.length) > expectedLinks ||
      aliases.some(
        ({ entry, namespaceRole }) =>
          namespaceRole !== exemplarNamespace ||
          entry.type !== "regular" ||
          entry.device !== exemplar.device ||
          entry.inode !== exemplar.inode ||
          entry.links !== exemplar.links ||
          entry.mode !== exemplar.mode ||
          entry.ctime_nanoseconds !== exemplar.ctime_nanoseconds ||
          entry.mtime_nanoseconds !== exemplar.mtime_nanoseconds ||
          entry.uid !== exemplar.uid ||
          entry.size !== exemplar.size ||
          entry.hard_link_group_sha256 !==
            exemplar.hard_link_group_sha256 ||
          entry.content_disposition !== exemplar.content_disposition ||
          entry.content_sha256 !== exemplar.content_sha256 ||
          entry.endpoint_sha256 !== exemplar.endpoint_sha256 ||
          entry.ownership !== exemplar.ownership ||
          entry.resource_kind !== exemplar.resource_kind,
      ) ||
      (expectedLinks === 1n
        ? exemplar.hard_link_group_sha256 !== ZERO_SHA256
        : !nonzeroSha256(exemplar.hard_link_group_sha256))
    ) {
      fail("live provider effect physical alias inventory was refused", 69);
    }
    const missingAliases = Number(expectedLinks) - aliases.length;
    missingHardLinkAliases += missingAliases;
    if (missingAliases > 0) {
      missingHardLinkAliasesByNamespace.set(
        exemplarNamespace,
        (missingHardLinkAliasesByNamespace.get(exemplarNamespace) ?? 0) +
          missingAliases,
      );
    }
  }
  for (const group of hardLinkGroups.values()) {
    const exemplar = group[0];
    if (
      group.some(
        ({ entry, namespaceRole }) =>
          namespaceRole !== exemplar.namespaceRole ||
          entry.device !== exemplar.entry.device ||
          entry.inode !== exemplar.entry.inode,
      ) || BigInt(group.length) > BigInt(exemplar.entry.links)
    ) {
      fail("live provider effect hard-link inventory was refused", 69);
    }
  }
  return Object.freeze({
    missingHardLinkAliases,
    missingHardLinkAliasesByNamespace: Object.freeze(
      [...missingHardLinkAliasesByNamespace.entries()]
        .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
        .map(([namespaceRole, count]) =>
          Object.freeze({ count, namespaceRole }),
        ),
    ),
  });
}

function inventoryReservations(history) {
  const finalEndpoints = eventsOfKind(history, "endpoint").filter(
    (event) => event.evidence.phase === "post-detachment-final",
  );
  if (
    finalEndpoints.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length ||
    new Set(finalEndpoints.map((event) => event.evidence.endpoint_kind)).size !==
      COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length
  ) {
    fail("live provider effect inventory reservations were incomplete", 69);
  }
  const engineEndpoint = finalEndpoints.find(
    (endpoint) => endpoint.evidence.endpoint_kind === "engine-api",
  );
  if (engineEndpoint === undefined) {
    fail("live provider effect inventory reservations were incomplete", 69);
  }
  return Object.freeze([
    ...finalEndpoints.map((endpoint) =>
      Object.freeze({
        content_sha256: ZERO_SHA256,
        endpoint_sha256: valueDigest(endpoint),
        identity_sha256: endpoint.evidence.path_identity_hmac_sha256,
        kind: "socket",
        namespace_role: endpoint.evidence.namespace_role,
      }),
    ),
    Object.freeze({
      content_sha256: engineEndpoint.evidence.docker_context_sha256,
      endpoint_sha256: valueDigest(engineEndpoint),
      identity_sha256:
        engineEndpoint.evidence.docker_context_path_identity_hmac_sha256,
      kind: "docker-context",
      namespace_role:
        engineEndpoint.evidence.docker_context_namespace_role,
    }),
  ]);
}

function validateInventoryReservedPrefix(
  entriesByNamespace,
  history,
  publicationPlan,
) {
  const reservations = inventoryReservations(history);
  const reservationByIdentity = new Map(
    reservations.map((reservation) => [
      reservation.identity_sha256,
      reservation,
    ]),
  );
  if (reservationByIdentity.size !== reservations.length) {
    fail("live provider effect inventory reservations were not distinct", 69);
  }
  const seen = new Set();
  for (const { entry, namespaceRole } of entriesByNamespace) {
    const reservation = reservationByIdentity.get(
      entry.relative_identity_hmac_sha256,
    );
    if (reservation === undefined) {
      if (entry.type === "socket" || entry.resource_kind === "docker-context") {
        fail("live provider effect reserved inventory identity was refused", 69);
      }
      continue;
    }
    const commonMismatch =
      namespaceRole !== reservation.namespace_role ||
      entry.ownership !== "generation-owned" ||
      entry.links !== "1" ||
      entry.hard_link_group_sha256 !== ZERO_SHA256 ||
      entry.endpoint_sha256 !== reservation.endpoint_sha256;
    const kindMismatch =
      reservation.kind === "socket"
        ? entry.type !== "socket" ||
          entry.content_disposition !== "endpoint-bound" ||
          entry.resource_kind !== "none" ||
          entry.content_sha256 !== ZERO_SHA256
        : entry.type !== "regular" ||
          entry.content_disposition !== "hashed" ||
          entry.resource_kind !== "docker-context" ||
          entry.content_sha256 !== reservation.content_sha256;
    if (commonMismatch || kindMismatch || seen.has(reservation.identity_sha256)) {
      fail("live provider effect reserved inventory identity was refused", 69);
    }
    seen.add(reservation.identity_sha256);
  }
  const unseenByNamespace = new Map(
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.map((role) => [role, 0]),
  );
  for (const reservation of reservations) {
    if (!seen.has(reservation.identity_sha256)) {
      unseenByNamespace.set(
        reservation.namespace_role,
        unseenByNamespace.get(reservation.namespace_role) + 1,
      );
    }
  }
  return Object.freeze({
    totalUnseen: reservations.length - seen.size,
    unseenByNamespace,
  });
}

function validateInventorySampleZeroPrefix(
  entriesByNamespace,
  history,
  publicationPlan,
) {
  return Object.freeze({
    ...validateInventoryPhysicalPrefix(entriesByNamespace),
    ...validateInventoryReservedPrefix(
      entriesByNamespace,
      history,
      publicationPlan,
    ),
  });
}

function validateInventoryPhysicalClosure(
  entriesByNamespace,
  history,
  publicationPlan,
) {
  const prefix = validateInventorySampleZeroPrefix(
    entriesByNamespace,
    history,
    publicationPlan,
  );
  if (prefix.missingHardLinkAliases !== 0) {
    fail("live provider effect hard-link inventory was incomplete", 69);
  }
  if (prefix.totalUnseen !== 0) {
    fail("live provider effect reserved inventory was incomplete", 69);
  }
  const finalEndpoints = eventsOfKind(history, "endpoint").filter(
    (event) => event.evidence.phase === "post-detachment-final",
  );
  const socketEntries = entriesByNamespace.filter(
    ({ entry }) => entry.type === "socket",
  );
  if (
    socketEntries.length !== finalEndpoints.length ||
    finalEndpoints.some(
      (endpoint) =>
        socketEntries.filter(
          ({ entry }) => entry.endpoint_sha256 === valueDigest(endpoint),
        ).length !== 1,
    ) ||
    socketEntries.some(({ entry, namespaceRole }) => {
      const endpoint = findEvent(history, entry.endpoint_sha256, "endpoint");
      return (
        endpoint?.evidence.phase !== "post-detachment-final" ||
        namespaceRole !== endpoint.evidence.namespace_role ||
        entry.relative_identity_hmac_sha256 !==
          endpoint.evidence.path_identity_hmac_sha256
      );
    })
  ) {
    fail("live provider effect endpoint inventory closure was refused", 69);
  }
  const engineEndpoint = finalEndpoints.find(
    (endpoint) => endpoint.evidence.endpoint_kind === "engine-api",
  );
  const dockerContextEntries = entriesByNamespace.filter(
    ({ entry }) =>
      entry.type === "regular" && entry.resource_kind === "docker-context",
  );
  if (
    engineEndpoint === undefined ||
    dockerContextEntries.length !== 1 ||
    dockerContextEntries[0].namespaceRole !==
      engineEndpoint.evidence.docker_context_namespace_role ||
    dockerContextEntries[0].entry.relative_identity_hmac_sha256 !==
      engineEndpoint.evidence.docker_context_path_identity_hmac_sha256 ||
    dockerContextEntries[0].entry.content_sha256 !==
      engineEndpoint.evidence.docker_context_sha256 ||
    dockerContextEntries[0].entry.endpoint_sha256 !== valueDigest(engineEndpoint) ||
    dockerContextEntries[0].entry.ownership !== "generation-owned"
  ) {
    fail("live provider effect Docker context inventory was refused", 69);
  }
}

function validateCompletedInventorySample(
  history,
  publicationPlan,
  sampleSequence,
  pendingGroup = undefined,
) {
  const entriesByNamespace = [];
  let totalHashedBytes = 0n;
  for (const [index, namespaceRole] of
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.entries()) {
    const entries =
      pendingGroup?.namespaceRole === namespaceRole
        ? pendingGroup.entries
        : sampleEntries(history, namespaceRole, sampleSequence);
    validateCompletedInventoryGroup(
      entries,
      publicationPlan.namespace_bindings[index],
      sampleSequence,
      history,
    );
    entriesByNamespace.push(
      ...entries.map((entry) => ({ entry, namespaceRole })),
    );
    totalHashedBytes += entries
      .filter((entry) => entry.content_disposition === "hashed")
      .reduce((total, entry) => total + BigInt(entry.size), 0n);
  }
  if (
    entriesByNamespace.length > MAX_INVENTORY_ENTRIES ||
    totalHashedBytes > BigInt(MAX_TOTAL_HASHED_BYTES)
  ) {
    fail("live provider effect inventory sample bounds were refused", 69);
  }
  if (sampleSequence === 0) {
    validateInventoryPhysicalClosure(
      entriesByNamespace,
      history,
      publicationPlan,
    );
  }
  return entriesByNamespace;
}

function validateInventorySet(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    INVENTORY_SET_FIELDS,
    "live provider effect inventory set",
  );
  const fence = uniqueEvent(history, "quiescence-fence");
  if (
    evidence.sample_count !== 2 ||
    evidence.quiescence_fence_sha256 !== valueDigest(fence) ||
    !Array.isArray(evidence.samples) ||
    evidence.samples.length !== COLIMA_LIVE_MUTATION_SURFACE_ROLES.length ||
    eventsOfKind(history, "recursive-inventory-set").length !== 0
  ) {
    fail("live provider effect inventory set was refused", 69);
  }
  validateCompletedInventorySample(history, publicationPlan, 0);
  validateCompletedInventorySample(history, publicationPlan, 1);
  for (const [index, sample] of evidence.samples.entries()) {
    exactKeys(
      sample,
      INVENTORY_SAMPLE_FIELDS,
      "live provider effect inventory sample",
    );
    const namespaceRole = COLIMA_LIVE_MUTATION_SURFACE_ROLES[index];
    const first = sampleEntries(history, namespaceRole, 0);
    const second = sampleEntries(history, namespaceRole, 1);
    const firstPageCount = inventoryPages(history, namespaceRole, 0).length;
    const secondPageCount = inventoryPages(history, namespaceRole, 1).length;
    const firstPages = inventoryPages(history, namespaceRole, 0);
    const secondPages = inventoryPages(history, namespaceRole, 1);
    const firstSha256 = inventorySampleSha256(firstPages);
    const secondSha256 = inventorySampleSha256(secondPages);
    const declaredBaseline = [
      ...publicationPlan.namespace_bindings[index]
        .baseline_relative_identity_hmac_sha256,
    ].sort();
    const firstBaseline = first
      .filter((entry) => entry.ownership === "admitted-baseline")
      .map((entry) => entry.relative_identity_hmac_sha256)
      .sort();
    const secondBaseline = second
      .filter((entry) => entry.ownership === "admitted-baseline")
      .map((entry) => entry.relative_identity_hmac_sha256)
      .sort();
    if (
      sample.namespace_role !== namespaceRole ||
      sample.first_sample_sha256 !== firstSha256 ||
      sample.second_sample_sha256 !== secondSha256 ||
      firstSha256 !== secondSha256 ||
      sample.entry_count !== first.length ||
      sample.page_count !== firstPageCount ||
      second.length !== first.length ||
      secondPageCount !== firstPageCount ||
      [...firstPages.slice(0, -1), ...secondPages.slice(0, -1)].some(
        (page) =>
          page.evidence.entries.length !== MAX_INVENTORY_ENTRIES_PER_PAGE,
      )
    ) {
      fail("live provider effect inventory samples differed", 73);
    }
    if (
      !liveProviderEffectBytes(firstBaseline).equals(
        liveProviderEffectBytes(declaredBaseline),
      ) ||
      !liveProviderEffectBytes(secondBaseline).equals(
        liveProviderEffectBytes(declaredBaseline),
      )
    ) {
      fail("live provider effect inventory baseline was incomplete", 73);
    }
    validateCompleteInventorySample(
      first,
      publicationPlan.namespace_bindings[index],
    );
    validateCompleteInventorySample(
      second,
      publicationPlan.namespace_bindings[index],
    );
  }
  if (evidence.inventory_sha256 !== valueDigest(evidence.samples)) {
    fail("live provider effect inventory set bounds were refused", 69);
  }
}

function eventDigestSet(history, kind) {
  return eventsOfKind(history, kind).map((event) => valueDigest(event));
}

function validateCreateSettlement(evidence, { history }) {
  exactKeys(
    evidence,
    CREATE_SETTLEMENT_FIELDS,
    "live provider effect create settlement",
  );
  const attempt = uniqueEvent(history, "start-attempt");
  const delivery = uniqueEvent(history, "delivery-result");
  const launchEdges = eventsOfKind(history, "launch-edge");
  const roleIdentities = eventsOfKind(history, "role-identity");
  const endpointHistory = eventsOfKind(history, "endpoint");
  const endpoints = endpointHistory.filter(
    (event) => event.evidence.phase === "post-detachment-final",
  );
  const completeEdgeIdentityBijection =
    launchEdges.every(
      (edge) =>
        roleIdentities.filter(
          (identity) =>
            identity.evidence.launch_edge_sha256 === valueDigest(edge),
        ).length === 1,
    ) &&
    roleIdentities.every(
      (identity) =>
        launchEdges.filter(
          (edge) =>
            valueDigest(edge) === identity.evidence.launch_edge_sha256,
        ).length === 1,
    );
  if (
    !CREATE_SETTLEMENT_VARIANTS.has(evidence.variant) ||
    evidence.start_attempt_sha256 !== valueDigest(attempt) ||
    evidence.delivery_result_sha256 !== valueDigest(delivery) ||
    evidence.marker_link_count !== 2 ||
    eventsOfKind(history, "create-settlement").length !== 0 ||
    eventsOfKind(history, "uncertain-start").length !== 0
  ) {
    fail("live provider effect create settlement was refused", 69);
  }
  if (evidence.variant === "authenticated-live-identity") {
    const inventory = uniqueEvent(history, "recursive-inventory-set");
    const fence = uniqueEvent(history, "quiescence-fence");
    const namedRoles = roleIdentities.filter((event) =>
      COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(event.evidence.role),
    );
    if (
      delivery.evidence.effect_possible !== true ||
      !completeEdgeIdentityBijection ||
      namedRoles.length !== COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length ||
      new Set(namedRoles.map((event) => event.evidence.role)).size !==
        COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length ||
      endpoints.length !== COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length ||
      endpointHistory.length !==
        COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length * 2 ||
      evidence.causal_node_count !== roleIdentities.length ||
      evidence.causal_edge_count !== launchEdges.length ||
      evidence.causal_node_count > MAX_CAUSAL_NODES ||
      evidence.causal_edge_count > MAX_CAUSAL_EDGES ||
      evidence.causal_graph_sha256 !==
        valueDigest({
          launch_edges: eventDigestSet(history, "launch-edge"),
          role_identities: eventDigestSet(history, "role-identity"),
        }) ||
      evidence.role_identity_set_sha256 !==
        valueDigest(eventDigestSet(history, "role-identity")) ||
      evidence.endpoint_set_sha256 !==
        valueDigest(endpoints.map((event) => valueDigest(event))) ||
      evidence.quiescence_fence_sha256 !== valueDigest(fence) ||
      evidence.inventory_set_sha256 !== valueDigest(inventory) ||
      evidence.absence_observation_sha256 !== ZERO_SHA256 ||
      evidence.residual_basis !== "none"
    ) {
      fail("live provider effect live settlement was refused", 69);
    }
    return;
  }
  const commonResidualValid =
    evidence.causal_node_count === roleIdentities.length &&
    evidence.causal_edge_count === launchEdges.length &&
    evidence.causal_graph_sha256 ===
      valueDigest({
        launch_edges: eventDigestSet(history, "launch-edge"),
        role_identities: eventDigestSet(history, "role-identity"),
      }) &&
    evidence.role_identity_set_sha256 ===
      valueDigest(eventDigestSet(history, "role-identity")) &&
    evidence.endpoint_set_sha256 ===
      valueDigest(endpoints.map((event) => valueDigest(event))) &&
    evidence.quiescence_fence_sha256 === ZERO_SHA256 &&
    evidence.inventory_set_sha256 === ZERO_SHA256 &&
    nonzeroSha256(evidence.absence_observation_sha256);
  const conclusiveResidual =
    evidence.residual_basis === "conclusive-not-created" &&
    delivery.evidence.delivery_disposition === "conclusive-not-created" &&
    delivery.evidence.effect_possible === false &&
    launchEdges.length === 1 &&
    launchEdges[0].evidence.child_role === "outer" &&
    roleIdentities.length === 0 &&
    endpointHistory.length === 0;
  if (!commonResidualValid || !conclusiveResidual) {
    fail("live provider effect residual settlement was refused", 69);
  }
}

function validateUncertainStart(evidence, { history }) {
  exactKeys(
    evidence,
    UNCERTAIN_START_FIELDS,
    "live provider effect uncertain start",
  );
  const attempt = uniqueEvent(history, "start-attempt");
  const delivery = uniqueEvent(history, "delivery-result");
  const namedRoles = eventsOfKind(history, "role-identity").filter((event) =>
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.includes(event.evidence.role),
  );
  const endpointHistory = eventsOfKind(history, "endpoint");
  const expectedReason =
    delivery.evidence.delivery_disposition === "indeterminate-effect-possible"
      ? "delivery-indeterminate"
      : namedRoles.length !== COLIMA_LIVE_PROVIDER_EFFECT_ROLES.length
        ? "identity-incomplete"
        : endpointHistory.length !==
            COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.length * 2
          ? "endpoint-incomplete"
          : "topology-incomplete";
  if (
    delivery.evidence.effect_possible !== true ||
    evidence.start_attempt_sha256 !== valueDigest(attempt) ||
    evidence.delivery_result_sha256 !== valueDigest(delivery) ||
    evidence.marker_link_count !== 2 ||
    !UNCERTAIN_START_REASONS.has(evidence.reason) ||
    evidence.reason !== expectedReason ||
    evidence.recovery_invocation_authorized !== false ||
    evidence.replay_authorized !== false ||
    eventsOfKind(history, "uncertain-start").length !== 0 ||
    eventsOfKind(history, "create-settlement").length !== 0
  ) {
    fail("live provider effect uncertain start was refused", 69);
  }
}

function deriveCleanupActions(history, createSettlement, publicationPlan) {
  const descriptors = [];
  const add = ({
    actionKind,
    depth,
    namespaceRole = "none",
    parentFsyncRequired,
    role = "none",
    targetClass,
    targetIdentitySha256,
  }) => {
    descriptors.push({
      actionKind,
      depth,
      namespaceRole,
      parentFsyncRequired,
      role,
      targetClass,
      targetIdentitySha256,
    });
  };
  const finalEndpoints = eventsOfKind(history, "endpoint").filter(
    (event) => event.evidence.phase === "post-detachment-final",
  );
  const endpoint = (kind) =>
    finalEndpoints.find((event) => event.evidence.endpoint_kind === kind);
  const roles = eventsOfKind(history, "role-identity");
  const role = (name) =>
    roles.find((event) => event.evidence.role === name);
  const engine = endpoint("engine-api");
  if (engine !== undefined) {
    add({
      actionKind: "withdraw-engine-context",
      depth: 0,
      parentFsyncRequired: false,
      targetClass: "engine-context",
      targetIdentitySha256: engine.evidence.docker_context_binding_sha256,
    });
    add({
      actionKind: "request-engine-shutdown",
      depth: 0,
      parentFsyncRequired: false,
      role: "hostagent",
      targetClass: "endpoint",
      targetIdentitySha256: valueDigest(engine),
    });
  }
  const shutdowns = [
    ["ssh-controlmaster", "request-ssh-controlmaster-shutdown"],
    ["usernet", "request-usernet-shutdown"],
    ["hostagent", "request-hostagent-shutdown"],
    ["outer", "request-outer-shutdown"],
  ];
  for (const [roleName, actionKind] of shutdowns) {
    const identity = role(roleName);
    if (identity !== undefined) {
      add({
        actionKind,
        depth: eventsOfKind(history, "launch-edge").find(
          (event) =>
            valueDigest(event) === identity.evidence.launch_edge_sha256,
        ).evidence.depth,
        parentFsyncRequired: false,
        role: roleName,
        targetClass: "process",
        targetIdentitySha256: identity.evidence.node_identity_sha256,
      });
    }
  }
  for (const identity of [...roles].reverse()) {
    const edge = findEvent(
      history,
      identity.evidence.launch_edge_sha256,
      "launch-edge",
    );
    add({
      actionKind: "prove-process-subtree-absent",
      depth: edge.evidence.depth,
      parentFsyncRequired: false,
      role: identity.evidence.role,
      targetClass: "process",
      targetIdentitySha256: identity.evidence.node_identity_sha256,
    });
  }
  if (roles.length === 0) {
    for (const edge of [...eventsOfKind(history, "launch-edge")].reverse()) {
      add({
        actionKind: "prove-process-subtree-absent",
        depth: edge.evidence.depth,
        parentFsyncRequired: false,
        role: edge.evidence.child_role,
        targetClass: "process",
        targetIdentitySha256: edge.evidence.child_node_identity_sha256,
      });
    }
  }
  const inventoryEntries =
    eventsOfKind(history, "recursive-inventory-set").length === 0
      ? []
      : COLIMA_LIVE_MUTATION_SURFACE_ROLES.flatMap((namespaceRole) =>
          sampleEntries(history, namespaceRole, 0).map((entry) => ({
            entry,
            namespaceRole,
          })),
        );
  for (const { entry, namespaceRole } of [...inventoryEntries].sort(
    (left, right) =>
      right.entry.depth - left.entry.depth ||
      Number(left.entry.type === "directory") -
        Number(right.entry.type === "directory") ||
      (left.namespaceRole < right.namespaceRole
        ? -1
        : left.namespaceRole > right.namespaceRole
          ? 1
          : 0) ||
      (left.entry.relative_identity_hmac_sha256 <
      right.entry.relative_identity_hmac_sha256
        ? -1
        : left.entry.relative_identity_hmac_sha256 >
            right.entry.relative_identity_hmac_sha256
          ? 1
          : 0),
  )) {
    if (entry.ownership !== "generation-owned") continue;
    add({
      actionKind:
        entry.type === "directory"
          ? "retire-directory"
          : entry.type === "socket"
            ? "retire-socket-evidence"
            : "retire-file",
      depth: entry.depth,
      namespaceRole,
      parentFsyncRequired: true,
      targetClass: "filesystem",
      targetIdentitySha256: entry.relative_identity_hmac_sha256,
    });
  }
  for (const namespaceRole of [...COLIMA_LIVE_MUTATION_SURFACE_ROLES].reverse()) {
    const namespaceBinding = publicationPlan.namespace_bindings.find(
      (binding) => binding.role === namespaceRole,
    );
    add({
      actionKind: "prove-namespace-baseline",
      depth: 0,
      namespaceRole,
      parentFsyncRequired: true,
      targetClass: "namespace",
      targetIdentitySha256: namespaceBinding.namespace_identity_hmac_sha256,
    });
  }
  if (descriptors.length === 0 || descriptors.length > MAX_CLEANUP_ACTIONS) {
    fail("live provider effect cleanup action bound was refused", 69);
  }
  const actions = [];
  for (const [actionSequence, descriptor] of descriptors.entries()) {
    const prerequisiteSha256 =
      actionSequence === 0
        ? valueDigest(createSettlement)
        : valueDigest(actions.at(-1));
    const common = {
      action_kind: descriptor.actionKind,
      action_sequence: actionSequence,
      depth: descriptor.depth,
      namespace_role: descriptor.namespaceRole,
      parent_fsync_required: descriptor.parentFsyncRequired,
      prerequisite_sha256: prerequisiteSha256,
      role: descriptor.role,
      target_class: descriptor.targetClass,
      target_identity_sha256: descriptor.targetIdentitySha256,
    };
    actions.push({
      ...common,
      expected_postcondition_sha256: valueDigest({
        ...common,
        expected_state: "retired-or-baseline-proved",
      }),
      expected_precondition_sha256: valueDigest({
        ...common,
        expected_state: "current-exact-target",
      }),
    });
  }
  return actions;
}

function validateCleanupAction(action, expectedSequence) {
  exactKeys(
    action,
    CLEANUP_ACTION_FIELDS,
    "live provider effect cleanup action",
  );
  if (
    !COLIMA_LIVE_PROVIDER_EFFECT_CLEANUP_ACTIONS.includes(action.action_kind) ||
    action.action_sequence !== expectedSequence ||
    !Number.isSafeInteger(action.depth) ||
    action.depth < 0 ||
    action.depth > Math.max(MAX_CAUSAL_DEPTH, MAX_RECURSIVE_DEPTH) ||
    !new Set(["none", ...COLIMA_LIVE_MUTATION_SURFACE_ROLES]).has(
      action.namespace_role,
    ) ||
    !new Set(["none", ...COLIMA_LIVE_PROVIDER_EFFECT_ROLES]).has(
      action.role,
    ) ||
    !new Set([
      "endpoint",
      "engine-context",
      "filesystem",
      "namespace",
      "process",
    ]).has(action.target_class) ||
    typeof action.parent_fsync_required !== "boolean" ||
    !nonzeroSha256(action.prerequisite_sha256) ||
    !nonzeroSha256(action.expected_precondition_sha256) ||
    !nonzeroSha256(action.expected_postcondition_sha256) ||
    !nonzeroSha256(action.target_identity_sha256)
  ) {
    fail("live provider effect cleanup action was refused", 69);
  }
}

function cleanupPlanPages(history) {
  return eventsOfKind(history, "cleanup-plan-page");
}

function cleanupPlanPageSetSha256(pages) {
  return valueDigest({
    page_count: pages.length,
    terminal_page_sha256:
      pages.length === 0 ? ZERO_SHA256 : valueDigest(pages.at(-1)),
  });
}

function validateCleanupPlanPage(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    CLEANUP_PLAN_PAGE_FIELDS,
    "live provider effect cleanup plan page",
  );
  const settlement = uniqueEvent(history, "create-settlement");
  const pages = cleanupPlanPages(history);
  const priorActions = pages.reduce(
    (total, page) => total + page.evidence.actions.length,
    0,
  );
  const expectedActions = deriveCleanupActions(
    history,
    settlement,
    publicationPlan,
  );
  const expectedPageActions = expectedActions.slice(
    priorActions,
    priorActions + MAX_CANONICAL_CONTAINER_ENTRIES,
  );
  if (
    evidence.create_settlement_sha256 !== valueDigest(settlement) ||
    !Number.isSafeInteger(evidence.page_sequence) ||
    evidence.page_sequence !== pages.length ||
    evidence.action_start !== priorActions ||
    !Array.isArray(evidence.actions) ||
    evidence.actions.length !== expectedPageActions.length ||
    evidence.actions.length === 0 ||
    (pages.length > 0 &&
      pages.at(-1).evidence.actions.length !==
        MAX_CANONICAL_CONTAINER_ENTRIES) ||
    priorActions + evidence.actions.length > MAX_CLEANUP_ACTIONS ||
    evidence.parent_page_sha256 !==
      (pages.length === 0 ? ZERO_SHA256 : valueDigest(pages.at(-1))) ||
    evidence.actions.some(
      (action, index) =>
        !liveProviderEffectBytes(action).equals(
          liveProviderEffectBytes(expectedPageActions[index]),
        ),
    ) ||
    eventsOfKind(history, "cleanup-plan").length !== 0
  ) {
    fail("live provider effect cleanup plan page was refused", 69);
  }
  for (const [index, action] of evidence.actions.entries()) {
    validateCleanupAction(action, priorActions + index);
  }
}

function validateCleanupPlan(evidence, { history, publicationPlan }) {
  exactKeys(
    evidence,
    CLEANUP_PLAN_FIELDS,
    "live provider effect cleanup plan",
  );
  const settlement = uniqueEvent(history, "create-settlement");
  const pages = cleanupPlanPages(history);
  const actionCount = pages.reduce(
    (total, page) => total + page.evidence.actions.length,
    0,
  );
  const actions = pages.flatMap((page) => page.evidence.actions);
  const expectedActions = deriveCleanupActions(
    history,
    settlement,
    publicationPlan,
  );
  if (
    pages.length === 0 ||
    evidence.create_settlement_sha256 !== valueDigest(settlement) ||
    evidence.page_count !== pages.length ||
    evidence.action_count !== actionCount ||
    actionCount <= 0 ||
    actionCount > MAX_CLEANUP_ACTIONS ||
    evidence.page_set_sha256 !== cleanupPlanPageSetSha256(pages) ||
    evidence.reverse_causal_order !== true ||
    pages.slice(0, -1).some(
      (page) =>
        page.evidence.actions.length !== MAX_CANONICAL_CONTAINER_ENTRIES,
    ) ||
    actions.length !== expectedActions.length ||
    actions.some(
      (action, index) =>
        !liveProviderEffectBytes(action).equals(
          liveProviderEffectBytes(expectedActions[index]),
        ),
    ) ||
    evidence.marker_link_count !== 2 ||
    eventsOfKind(history, "cleanup-plan").length !== 0
  ) {
    fail("live provider effect cleanup plan was refused", 69);
  }
}

function cleanupActionsForPlan(history, plan) {
  const settlementSha256 = plan.evidence.create_settlement_sha256;
  return cleanupPlanPages(history)
    .filter(
      (page) =>
        page.evidence.create_settlement_sha256 === settlementSha256,
    )
    .flatMap((page) => page.evidence.actions);
}

function validateCleanupProgress(
  evidence,
  {
    history,
    publicationPlan,
    publisherAuthoritySha256,
    source,
    witness,
  },
) {
  exactKeys(
    evidence,
    CLEANUP_PROGRESS_FIELDS,
    "live provider effect cleanup progress",
  );
  const plan = uniqueEvent(history, "cleanup-plan");
  const actions = cleanupActionsForPlan(history, plan);
  const progress = eventsOfKind(history, "cleanup-progress");
  const priorCount = progress.reduce(
    (total, event) => total + event.evidence.outcomes.length,
    0,
  );
  const priorObservationValues = new Set(
    progress.flatMap((event) =>
      event.evidence.outcomes.flatMap((outcome) => [
        outcome.marker_witness_observation_sha256,
        outcome.observation_challenge_sha256,
        outcome.outcome_observation_sha256,
        outcome.parent_fsync_observation_sha256,
        outcome.precondition_observation_sha256,
        outcome.recursive_prefix_observation_sha256,
        outcome.source_reproof_observation_sha256,
      ]),
    ).filter((value) => value !== ZERO_SHA256),
  );
  const currentObservationValues = new Set();
  if (
    evidence.cleanup_plan_sha256 !== valueDigest(plan) ||
    evidence.action_start !== priorCount ||
    !Array.isArray(evidence.outcomes) ||
    evidence.outcomes.length !==
      Math.min(MAX_CLEANUP_OUTCOMES_PER_PAGE, actions.length - priorCount) ||
    evidence.outcomes.length === 0 ||
    (progress.length > 0 &&
      progress.at(-1).evidence.outcomes.length !==
        MAX_CLEANUP_OUTCOMES_PER_PAGE) ||
    priorCount + evidence.outcomes.length > actions.length ||
    evidence.authority_sha256 !== publisherAuthoritySha256 ||
    evidence.parent_progress_sha256 !==
      (progress.length === 0 ? ZERO_SHA256 : valueDigest(progress.at(-1)))
  ) {
    fail("live provider effect cleanup progress was refused", 69);
  }
  for (const [index, outcome] of evidence.outcomes.entries()) {
    exactKeys(
      outcome,
      CLEANUP_OUTCOME_FIELDS,
      "live provider effect cleanup outcome",
    );
    const action = actions[priorCount + index];
    const requiredObservationValues = [
      outcome.marker_witness_observation_sha256,
      outcome.observation_challenge_sha256,
      outcome.outcome_observation_sha256,
      outcome.precondition_observation_sha256,
      outcome.recursive_prefix_observation_sha256,
      outcome.source_reproof_observation_sha256,
    ];
    const observationValues = [
      ...requiredObservationValues,
      ...(outcome.parent_fsync_observation_sha256 === ZERO_SHA256
        ? []
        : [outcome.parent_fsync_observation_sha256]),
    ];
    const expectedObservationBindingSha256 = valueDigest({
      action_sha256: outcome.action_sha256,
      authority_sha256: evidence.authority_sha256,
      cleanup_plan_sha256: evidence.cleanup_plan_sha256,
      expected_postcondition_sha256: action.expected_postcondition_sha256,
      expected_precondition_sha256: action.expected_precondition_sha256,
      marker_link_count: 2,
      marker_witness_observation_sha256:
        outcome.marker_witness_observation_sha256,
      observation_challenge_sha256: outcome.observation_challenge_sha256,
      outcome_observation_sha256: outcome.outcome_observation_sha256,
      parent_fsync_observation_sha256:
        outcome.parent_fsync_observation_sha256,
      parent_progress_sha256: evidence.parent_progress_sha256,
      precondition_observation_sha256:
        outcome.precondition_observation_sha256,
      recursive_prefix_observation_sha256:
        outcome.recursive_prefix_observation_sha256,
      source_reproof_observation_sha256:
        outcome.source_reproof_observation_sha256,
      source_sha256: valueDigest(source),
      operation_plan_sha256: valueDigest(publicationPlan),
      witness_sha256: valueDigest(witness),
    });
    if (
      outcome.action_sequence !== priorCount + index ||
      outcome.action_sha256 !== valueDigest(action) ||
      outcome.disposition !== "completed-exact" ||
      requiredObservationValues.some((value) => !nonzeroSha256(value)) ||
      new Set(observationValues).size !== observationValues.length ||
      observationValues.some(
        (value) =>
          priorObservationValues.has(value) ||
          currentObservationValues.has(value),
      ) ||
      outcome.observation_binding_sha256 !==
        expectedObservationBindingSha256 ||
      (action.parent_fsync_required
        ? !nonzeroSha256(outcome.parent_fsync_observation_sha256)
        : outcome.parent_fsync_observation_sha256 !== ZERO_SHA256)
    ) {
      fail("live provider effect cleanup outcome was refused", 69);
    }
    for (const value of observationValues) currentObservationValues.add(value);
  }
}

function validateCleanupSettlement(
  evidence,
  { history, publicationPlan, source, witness },
) {
  exactKeys(
    evidence,
    CLEANUP_SETTLEMENT_FIELDS,
    "live provider effect cleanup settlement",
  );
  const plan = uniqueEvent(history, "cleanup-plan");
  const progress = eventsOfKind(history, "cleanup-progress");
  const progressCount = progress.reduce(
    (total, event) => total + event.evidence.outcomes.length,
    0,
  );
  const createSettlement = uniqueEvent(history, "create-settlement");
  const expectedBaselineSha256 = valueDigest(
    publicationPlan.namespace_bindings.map((binding) => ({
      baseline_descriptor_set_hmac_sha256:
        binding.baseline_descriptor_set_hmac_sha256,
      baseline_descriptors: binding.baseline_descriptors,
      baseline_relative_identity_hmac_sha256:
        binding.baseline_relative_identity_hmac_sha256,
      baseline_set_binding_sha256: binding.baseline_set_binding_sha256,
      namespace_identity_hmac_sha256:
        binding.namespace_identity_hmac_sha256,
      observed_entry_set_hmac_sha256:
        binding.observed_entry_set_hmac_sha256,
      role: binding.role,
    })),
  );
  const expectedRoleAbsenceSha256 = valueDigest({
    disposition: "absent",
    node_identities: eventsOfKind(history, "launch-edge").map(
      (event) => event.evidence.child_node_identity_sha256,
    ),
    observation_sha256: evidence.role_absence_observation_sha256,
  });
  const expectedEndpointAbsenceSha256 = valueDigest({
    disposition: "absent",
    endpoints: eventsOfKind(history, "endpoint")
      .filter((event) => event.evidence.phase === "post-detachment-final")
      .map((event) => event.evidence.socket_identity_hmac_sha256),
    observation_sha256: evidence.endpoint_absence_observation_sha256,
  });
  const settlementObservationValues = [
    evidence.endpoint_absence_observation_sha256,
    evidence.marker_witness_topology_observation_sha256,
    evidence.role_absence_observation_sha256,
    evidence.source_reproof_observation_sha256,
  ];
  const priorObservationValues = new Set(
    progress.flatMap((event) =>
      event.evidence.outcomes.flatMap((outcome) => [
        outcome.marker_witness_observation_sha256,
        outcome.observation_challenge_sha256,
        outcome.outcome_observation_sha256,
        outcome.parent_fsync_observation_sha256,
        outcome.precondition_observation_sha256,
        outcome.recursive_prefix_observation_sha256,
        outcome.source_reproof_observation_sha256,
      ]),
    ).filter((value) => value !== ZERO_SHA256),
  );
  if (
    evidence.cleanup_plan_sha256 !== valueDigest(plan) ||
    evidence.create_settlement_sha256 !== valueDigest(createSettlement) ||
    progressCount !== plan.evidence.action_count ||
    progress.slice(0, -1).some(
      (event) =>
        event.evidence.outcomes.length !== MAX_CLEANUP_OUTCOMES_PER_PAGE,
    ) ||
    evidence.progress_count !== progressCount ||
    evidence.progress_head_sha256 !==
      (progress.length === 0 ? ZERO_SHA256 : valueDigest(progress.at(-1))) ||
    evidence.namespace_baselines_sha256 !== expectedBaselineSha256 ||
    settlementObservationValues.some((value) => !nonzeroSha256(value)) ||
    new Set(settlementObservationValues).size !==
      settlementObservationValues.length ||
    settlementObservationValues.some((value) =>
      priorObservationValues.has(value),
    ) ||
    evidence.final_inventory_sha256 !== expectedBaselineSha256 ||
    evidence.role_absence_sha256 !== expectedRoleAbsenceSha256 ||
    evidence.endpoint_absence_sha256 !== expectedEndpointAbsenceSha256 ||
    evidence.marker_witness_topology_sha256 !==
      valueDigest({
        marker_link_count: 2,
        marker_present: true,
        observation_sha256:
          evidence.marker_witness_topology_observation_sha256,
        witness_link_count: 2,
        witness_sha256: valueDigest(witness),
      }) ||
    evidence.source_reproof_sha256 !==
      valueDigest({
        cleanup_plan_sha256: valueDigest(plan),
        observation_sha256: evidence.source_reproof_observation_sha256,
        source_sha256: valueDigest(source),
        witness_sha256: valueDigest(witness),
      }) ||
    evidence.marker_link_count !== 2 ||
    evidence.terminal_receipt_publication_authorized !== true ||
    eventsOfKind(history, "cleanup-settlement").length !== 0
  ) {
    fail("live provider effect cleanup settlement was refused", 69);
  }
}

function terminalReceiptResult(
  history,
  publicationPlan,
  publisherAuthoritySha256,
  witness,
) {
  const attempt = uniqueEvent(history, "start-attempt");
  const createSettlement = uniqueEvent(history, "create-settlement");
  const cleanupSettlement = uniqueEvent(history, "cleanup-settlement");
  return {
    cleanup_settlement_sha256: valueDigest(cleanupSettlement),
    create_settlement_sha256: valueDigest(createSettlement),
    evidence_class: publicationPlan.evidence_class,
    operation_contract_sha256: publicationPlan.operation_contract_sha256,
    operation_kind: publicationPlan.operation_kind,
    operation_plan_sha256: valueDigest(publicationPlan),
    phase: "provider-effect-retired",
    publisher_authority_sha256: publisherAuthoritySha256,
    schema: RECEIPT_BINDING_SCHEMA,
    slot_sequence: witness.slot_sequence,
    slot_sha256: witness.slot_sha256,
    start_attempt_sha256: valueDigest(attempt),
    witness_sha256: valueDigest(witness),
  };
}

export function buildColimaLiveProviderEffectTerminalReceipt(input) {
  exactKeys(
    input,
    [
      "history",
      "publicationPlan",
      "publisherAuthorityChain",
      "publisherAuthoritySha256",
      "source",
      "witness",
    ],
    "live provider effect terminal receipt input",
  );
  const {
    history,
    publicationPlan,
    publisherAuthorityChain,
    publisherAuthoritySha256,
    source,
    witness,
  } = input;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  validateColimaLiveProviderEffectWitness(witness, {
    publicationPlan,
    source,
  });
  const validatedHistory = validateHistoryInternal(
    history,
    {
      publicationPlan,
      publisherAuthorityChain,
      source,
      witness,
    },
    { allowNextAuthority: true },
  );
  if (
    validatedHistory.at(-1)?.event_kind !== "cleanup-settlement" ||
    publisherAuthoritySha256 !==
      publisherAuthorityAt(
        validatedHistory.length,
        publisherAuthorityChain,
        witness,
      )
  ) {
    fail("live provider effect terminal receipt frontier was refused", 69);
  }
  return deepFreeze({
    fixture_id: publicationPlan.fixture_id,
    outcome: "passed",
    phase: "provider-effect-retired",
    previous_sha256: publicationPlan.receipt_previous_sha256,
    result: terminalReceiptResult(
      validatedHistory,
      publicationPlan,
      publisherAuthoritySha256,
      witness,
    ),
    schema: TERMINAL_RECEIPT_SCHEMA,
    sequence: publicationPlan.receipt_sequence,
  });
}

function validateTerminalReceipt(
  evidence,
  { history, publicationPlan, publisherAuthoritySha256, witness },
) {
  exactKeys(
    evidence,
    TERMINAL_RECEIPT_EVENT_FIELDS,
    "live provider effect terminal receipt event",
  );
  exactKeys(
    evidence.receipt,
    TERMINAL_RECEIPT_FIELDS,
    "live provider effect terminal receipt",
  );
  exactKeys(
    evidence.receipt.result,
    RECEIPT_BINDING_FIELDS,
    "live provider effect terminal receipt result",
  );
  const cleanupSettlement = uniqueEvent(history, "cleanup-settlement");
  const expectedReceipt = {
    fixture_id: publicationPlan.fixture_id,
    outcome: "passed",
    phase: "provider-effect-retired",
    previous_sha256: publicationPlan.receipt_previous_sha256,
    result: terminalReceiptResult(
      history,
      publicationPlan,
      publisherAuthoritySha256,
      witness,
    ),
    schema: TERMINAL_RECEIPT_SCHEMA,
    sequence: publicationPlan.receipt_sequence,
  };
  if (
    evidence.cleanup_settlement_sha256 !== valueDigest(cleanupSettlement) ||
    evidence.marker_link_count !== 2 ||
    evidence.witness_link_count !== 2 ||
    !nonzeroSha256(evidence.receipt_publication_observation_sha256) ||
    evidence.receipt_sha256 !== valueDigest(expectedReceipt) ||
    !liveProviderEffectBytes(evidence.receipt).equals(
      liveProviderEffectBytes(expectedReceipt),
    ) ||
    eventsOfKind(history, "terminal-receipt").length !== 0
  ) {
    fail("live provider effect terminal receipt was refused", 69);
  }
}

function validateCompletion(
  evidence,
  {
    eventSequence,
    history,
    publisherAuthoritySha256,
    witness,
  },
) {
  exactKeys(
    evidence,
    COMPLETION_FIELDS,
    "live provider effect completion",
  );
  if (
    evidence.marker_present !== false ||
    evidence.witness_link_count !== 1 ||
    !nonzeroSha256(evidence.marker_retirement_observation_sha256) ||
    !nonzeroSha256(evidence.provider_root_fsync_observation_sha256) ||
    !nonzeroSha256(evidence.witness_identity_sha256) ||
    evidence.witness_identity_sha256 !==
      witness.marker_witness_identity_sha256 ||
    evidence.marker_retirement_observation_sha256 ===
      evidence.provider_root_fsync_observation_sha256 ||
    evidence.marker_retirement_binding_sha256 !==
      valueDigest({
        cleanup_settlement_sha256: evidence.cleanup_settlement_sha256,
        marker_retirement_observation_sha256:
          evidence.marker_retirement_observation_sha256,
        provider_root_fsync_observation_sha256:
          evidence.provider_root_fsync_observation_sha256,
        publisher_authority_sha256: publisherAuthoritySha256,
        receipt_sha256: evidence.receipt_sha256,
        witness_identity_sha256: evidence.witness_identity_sha256,
        witness_link_count: evidence.witness_link_count,
      }) ||
    eventsOfKind(history, "completion").length !== 0
  ) {
    fail("live provider effect completion topology was refused", 69);
  }
  if (evidence.variant === "pre-attempt-retired-zero-receipt") {
    const legalPrefix =
      history.length === 0 ||
      (history.length === 1 && history[0].event_kind === "start-authority");
    if (
      !legalPrefix ||
      eventSequence !== history.length ||
      evidence.start_authority_sha256 !==
        (history.length === 0 ? ZERO_SHA256 : valueDigest(history[0])) ||
      (history.length === 1 &&
        evidence.witness_identity_sha256 !==
          history[0].evidence.first_reproof.marker_witness_identity_sha256) ||
      evidence.start_attempt_sha256 !== ZERO_SHA256 ||
      evidence.create_settlement_sha256 !== ZERO_SHA256 ||
      evidence.cleanup_settlement_sha256 !== ZERO_SHA256 ||
      evidence.receipt_event_sha256 !== ZERO_SHA256 ||
      evidence.receipt_sha256 !== ZERO_SHA256
    ) {
      fail("live provider effect pre-attempt completion was refused", 69);
    }
    return;
  }
  if (evidence.variant !== "attempted-effect-retired") {
    fail("live provider effect completion variant was refused", 69);
  }
  const attempt = uniqueEvent(history, "start-attempt");
  const authority = uniqueEvent(history, "start-authority");
  const createSettlement = uniqueEvent(history, "create-settlement");
  const cleanupSettlement = uniqueEvent(history, "cleanup-settlement");
  const receiptEvent = uniqueEvent(history, "terminal-receipt");
  if (
    history.at(-1) !== receiptEvent ||
    evidence.start_authority_sha256 !== valueDigest(authority) ||
    evidence.start_attempt_sha256 !== valueDigest(attempt) ||
    evidence.create_settlement_sha256 !== valueDigest(createSettlement) ||
    evidence.cleanup_settlement_sha256 !== valueDigest(cleanupSettlement) ||
    evidence.receipt_event_sha256 !== valueDigest(receiptEvent) ||
    evidence.receipt_sha256 !== receiptEvent.evidence.receipt_sha256 ||
    evidence.witness_identity_sha256 !==
      authority.evidence.first_reproof.marker_witness_identity_sha256
  ) {
    fail("live provider effect attempted completion was refused", 69);
  }
}

function freshEvidenceValues(event) {
  const evidence = event.evidence;
  let values;
  switch (event.event_kind) {
    case "start-authority":
      values = [evidence.first_reproof.observation_challenge_sha256];
      break;
    case "start-attempt":
      values = [evidence.second_reproof.observation_challenge_sha256];
      break;
    case "delivery-result":
      values = [evidence.observation_sha256];
      break;
    case "role-identity":
      values = [
        evidence.challenge_sha256,
        evidence.detachment_observation_sha256,
      ];
      break;
    case "endpoint":
      values = [
        evidence.challenge_sha256,
        evidence.connection_observation_sha256,
      ];
      break;
    case "quiescence-fence":
      values = [
        evidence.process_reconciliation.observation_challenge_sha256,
        evidence.process_reconciliation.process_observation_sha256,
      ];
      break;
    case "quiescence-ack":
      values = [evidence.resource_frontier_observation_sha256];
      break;
    case "create-settlement":
      values = [evidence.absence_observation_sha256];
      break;
    case "cleanup-progress":
      values = evidence.outcomes.flatMap((outcome) => [
        outcome.marker_witness_observation_sha256,
        outcome.observation_challenge_sha256,
        outcome.outcome_observation_sha256,
        outcome.parent_fsync_observation_sha256,
        outcome.precondition_observation_sha256,
        outcome.recursive_prefix_observation_sha256,
        outcome.source_reproof_observation_sha256,
      ]);
      break;
    case "cleanup-settlement":
      values = [
        evidence.endpoint_absence_observation_sha256,
        evidence.marker_witness_topology_observation_sha256,
        evidence.role_absence_observation_sha256,
        evidence.source_reproof_observation_sha256,
      ];
      break;
    case "terminal-receipt":
      values = [evidence.receipt_publication_observation_sha256];
      break;
    case "completion":
      values = [
        evidence.marker_retirement_observation_sha256,
        evidence.provider_root_fsync_observation_sha256,
      ];
      break;
    default:
      values = [];
  }
  return values.filter((value) => value !== ZERO_SHA256);
}

function validateFreshEvidence(event, history, publicationPlan) {
  const current = freshEvidenceValues(event);
  const prior = new Set(history.flatMap(freshEvidenceValues));
  const reservedRoleChallenges = new Set(
    publicationPlan.planned_role_contracts.map(
      (contract) => contract.role_challenge_sha256,
    ),
  );
  const permittedReservedValue =
    event.event_kind === "role-identity"
      ? event.evidence.challenge_sha256
      : undefined;
  if (
    current.some((value) => !nonzeroSha256(value)) ||
    new Set(current).size !== current.length ||
    current.some((value) => prior.has(value)) ||
    current.some(
      (value) =>
        reservedRoleChallenges.has(value) &&
        value !== permittedReservedValue,
    )
  ) {
    fail("live provider effect fresh evidence was reused", 69);
  }
}

function validateEventSequenceInput(history) {
  if (
    isProxy(history) ||
    !Array.isArray(history) ||
    Object.getPrototypeOf(history) !== Array.prototype ||
    history.length > MAX_EFFECT_EVENTS
  ) {
    fail("live provider effect history was refused", 70);
  }
  const keys = Reflect.ownKeys(history);
  if (
    keys.some((key) => typeof key !== "string") ||
    keys.length !== history.length + 1
  ) {
    fail("live provider effect history was not a dense data array", 70);
  }
  for (let index = 0; index < history.length; index += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(history, String(index));
    if (
      descriptor === undefined ||
      !("value" in descriptor) ||
      descriptor.enumerable !== true
    ) {
      fail("live provider effect history was not a dense data array", 70);
    }
  }
}

function validateEventEvidence(value, context) {
  switch (value.event_kind) {
    case "start-authority":
      return validateStartAuthority(value.evidence, context);
    case "start-attempt":
      return validateStartAttempt(value.evidence, context);
    case "delivery-result":
      return validateDeliveryResult(value.evidence, context);
    case "launch-edge":
      return validateLaunchEdge(value.evidence, context);
    case "role-identity":
      return validateRoleIdentity(value.evidence, context);
    case "endpoint":
      return validateEndpoint(value.evidence, context);
    case "quiescence-fence":
      return validateQuiescenceFence(value.evidence, context);
    case "quiescence-ack":
      return validateQuiescenceAcknowledgement(value.evidence, context);
    case "recursive-inventory-page":
      return validateInventoryPage(value.evidence, context);
    case "recursive-inventory-set":
      return validateInventorySet(value.evidence, context);
    case "create-settlement":
      return validateCreateSettlement(value.evidence, context);
    case "uncertain-start":
      return validateUncertainStart(value.evidence, context);
    case "cleanup-plan-page":
      return validateCleanupPlanPage(value.evidence, context);
    case "cleanup-plan":
      return validateCleanupPlan(value.evidence, context);
    case "cleanup-progress":
      return validateCleanupProgress(value.evidence, context);
    case "cleanup-settlement":
      return validateCleanupSettlement(value.evidence, context);
    case "terminal-receipt":
      return validateTerminalReceipt(value.evidence, context);
    case "completion":
      return validateCompletion(value.evidence, context);
    default:
      fail("live provider effect event kind was refused", 69);
  }
}

function validateEventInternal(
  value,
  { history, publicationPlan, publisherAuthorityChain, source, witness },
) {
  const selected = variant(source.fixtureOnly);
  if (source.fixtureOnly !== true) {
    fail("production live provider effect publication was refused", 69);
  }
  exactKeys(value, EVENT_FIELDS, "live provider effect event");
  preflightClosedDataGraph(value);
  validateSlotBinding(value, publicationPlan);
  if (
    !COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS.includes(value.event_kind) ||
    value.schema !== selected.schemas.event[value.event_kind] ||
    value.slot_sequence !== witness.slot_sequence ||
    value.slot_sha256 !== witness.slot_sha256 ||
    !Number.isSafeInteger(value.event_sequence) ||
    value.event_sequence !== history.length ||
    value.event_sequence >= MAX_EFFECT_EVENTS ||
    value.parent_sha256 !==
      (history.length === 0
        ? valueDigest(witness)
        : valueDigest(history.at(-1))) ||
    value.publisher_authority_sha256 !==
      publisherAuthorityAt(
        value.event_sequence,
        publisherAuthorityChain,
        witness,
      ) ||
    (new Set(["start-attempt", "start-authority"]).has(value.event_kind) &&
      value.publisher_authority_sha256 !== witness.slot_sha256) ||
    history.some(
      (event) =>
        event.event_kind === "uncertain-start" ||
        event.event_kind === "completion",
    )
  ) {
    fail("live provider effect event envelope was refused", 69);
  }
  validateEventPhase(value.event_kind, history);
  if (
    eventsOfKind(history, "cleanup-settlement").length > 0 &&
    eventsOfKind(history, "terminal-receipt").length === 0 &&
    value.event_kind !== "terminal-receipt"
  ) {
    fail("live provider effect terminal cleanup history was refused", 69);
  }
  validateEventEvidence(value, {
    eventSequence: value.event_sequence,
    history,
    parentSha256: value.parent_sha256,
    publicationPlan,
    publisherAuthoritySha256: value.publisher_authority_sha256,
    source,
    witness,
  });
  validateFreshEvidence(value, history, publicationPlan);
  return value;
}

function validateHistoryInternal(
  history,
  { publicationPlan, publisherAuthorityChain, source, witness },
  { allowNextAuthority = false } = {},
) {
  validateEventSequenceInput(history);
  validatePublisherAuthorityChain(publisherAuthorityChain, witness);
  const recoveryAuthorized = source.fixtureOnly
    ? COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities
        .fixture_effect_recovery_authorized
    : COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT.capabilities
        .provider_recovery_authorized;
  if (
    publisherAuthorityChain.length > 1 &&
    !recoveryAuthorized
  ) {
    fail("live provider effect recovery publisher was not authorized", 69);
  }
  const validated = [];
  for (const event of history) {
    validateEventInternal(event, {
      history: validated,
      publicationPlan,
      publisherAuthorityChain,
      source,
      witness,
    });
    validated.push(event);
  }
  const pendingAuthorities = publisherAuthorityChain
    .slice(1)
    .filter(
      (authority) => authority.first_event_sequence >= validated.length,
    );
  if (
    pendingAuthorities.some(
      (authority) => authority.first_event_sequence !== validated.length,
    ) ||
    (!allowNextAuthority && pendingAuthorities.length > 0)
  ) {
    fail("live provider effect publisher authority frontier was refused", 69);
  }
  return validated;
}

export function validateColimaLiveProviderEffectEventHistory(
  history,
  options,
) {
  exactKeys(
    options,
    ["publicationPlan", "publisherAuthorityChain", "source", "witness"],
    "live provider effect history validation input",
  );
  const { publicationPlan, publisherAuthorityChain, source, witness } = options;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  validateColimaLiveProviderEffectWitness(witness, {
    publicationPlan,
    source,
  });
  validateHistoryInternal(history, {
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  });
  return history;
}

export function validateColimaLiveProviderEffectEvent(
  value,
  options,
) {
  exactKeys(
    options,
    [
      "history",
      "publicationPlan",
      "publisherAuthorityChain",
      "source",
      "witness",
    ],
    "live provider effect event validation input",
  );
  const {
    history,
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  } = options;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  validateColimaLiveProviderEffectWitness(witness, {
    publicationPlan,
    source,
  });
  const validatedHistory = validateHistoryInternal(history, {
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  }, { allowNextAuthority: true });
  return validateEventInternal(value, {
    history: validatedHistory,
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  });
}

export function deriveColimaLiveFixtureProviderEffectCleanupActions(input) {
  exactKeys(
    input,
    [
      "history",
      "publicationPlan",
      "publisherAuthorityChain",
      "source",
      "witness",
    ],
    "live provider effect cleanup derivation input",
  );
  const {
    history,
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  } = input;
  validateColimaLiveProviderEffectEventHistory(history, {
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  });
  const settlement = uniqueEvent(history, "create-settlement");
  if (
    eventsOfKind(history, "cleanup-plan-page").length !== 0 ||
    eventsOfKind(history, "cleanup-plan").length !== 0
  ) {
    fail("live provider effect cleanup derivation frontier was refused", 69);
  }
  return Object.freeze(
    deriveCleanupActions(history, settlement, publicationPlan).map((action) =>
      deepFreeze(action),
    ),
  );
}

export function buildColimaLiveProviderEffectEvent(input) {
  exactKeys(
    input,
    [
      "eventKind",
      "evidence",
      "history",
      "publicationPlan",
      "publisherAuthorityChain",
      "publisherAuthoritySha256",
      "source",
      "witness",
    ],
    "live provider effect event input",
  );
  const {
    eventKind,
    evidence,
    history,
    publicationPlan,
    publisherAuthorityChain,
    publisherAuthoritySha256,
    source,
    witness,
  } = input;
  validateColimaLiveProviderEffectPublicationPlan(publicationPlan, source);
  const { selected } = sourceContext(publicationPlan.admission, source);
  validateColimaLiveProviderEffectWitness(witness, {
    publicationPlan,
    source,
  });
  const validatedHistory = validateHistoryInternal(history, {
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  }, { allowNextAuthority: true });
  if (!COLIMA_LIVE_PROVIDER_EFFECT_EVENT_KINDS.includes(eventKind)) {
    fail("live provider effect event kind was refused", 69);
  }
  const value = {
    ...slotBinding(publicationPlan, witness.slot_sequence, witness.slot_sha256),
    event_kind: eventKind,
    event_sequence: validatedHistory.length,
    evidence,
    parent_sha256:
      validatedHistory.length === 0
        ? valueDigest(witness)
        : valueDigest(validatedHistory.at(-1)),
    publisher_authority_sha256: publisherAuthoritySha256,
    schema: selected.schemas.event[eventKind],
  };
  validateEventInternal(value, {
    history: validatedHistory,
    publicationPlan,
    publisherAuthorityChain,
    source,
    witness,
  });
  return deepFreeze(value);
}

validateOperationContract(COLIMA_LIVE_PROVIDER_EFFECT_OPERATION_CONTRACT, false);
validateOperationContract(
  COLIMA_LIVE_FIXTURE_PROVIDER_EFFECT_OPERATION_CONTRACT,
  true,
);

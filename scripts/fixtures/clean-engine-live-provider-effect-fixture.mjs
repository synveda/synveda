import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign,
} from "node:crypto";
import { isProxy } from "node:util/types";
import {
  COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS,
  COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS,
  COLIMA_LIVE_PROVIDER_EFFECT_ROLES,
  buildColimaLiveProviderEffectEvent,
  buildColimaLiveProviderEffectPublicationPlan,
  buildColimaLiveProviderEffectWitness,
  colimaLiveProviderEffectProofBytes,
  deriveColimaLiveFixtureProviderEffectCleanupActions,
  liveProviderEffectBytes,
  liveProviderEffectDigest,
} from "../../deploy/compose/scripts/clean-engine-live-provider-effect.mjs";
import { COLIMA_LIVE_MUTATION_SURFACE_ROLES } from "../../deploy/compose/scripts/clean-engine-colima-live-schemas.mjs";
import {
  cleanEngineLiveBaselineNetworkConfigBytesFixture,
  cleanEngineLiveProviderProcessStartFreshAdmissionFixture,
} from "./clean-engine-live-provider-process-start-fixture.mjs";

const ZERO_SHA256 = "0".repeat(64);

function sha256(label) {
  return createHash("sha256").update(label).digest("hex");
}

function valueDigest(value) {
  return liveProviderEffectDigest(liveProviderEffectBytes(value));
}

function keyMaterial(role) {
  const seed = createHash("sha256")
    .update(`synveda-cpr-45-fixture-ed25519:${role}`, "utf8")
    .digest();
  const privateKey = createPrivateKey({
    format: "der",
    key: Buffer.concat([
      Buffer.from("302e020100300506032b657004220420", "hex"),
      seed,
    ]),
    type: "pkcs8",
  });
  const publicKey = createPublicKey(privateKey);
  const der = publicKey.export({ format: "der", type: "spki" });
  return {
    privateKey,
    publicKeyDerBase64: der.toString("base64"),
    publicKeySha256: liveProviderEffectDigest(der),
  };
}

function append(context, eventKind, evidence) {
  const event = buildColimaLiveProviderEffectEvent({
    eventKind,
    evidence,
    history: context.events,
    publicationPlan: context.publicationPlan,
    publisherAuthorityChain: context.publisherAuthorityChain,
    publisherAuthoritySha256: context.publisherAuthoritySha256,
    source: context.source,
    witness: context.witness,
  });
  context.events.push(event);
  return event;
}

function signedEvidence(context, domain, evidence, privateKey) {
  const parentSha256 =
    context.events.length === 0
      ? valueDigest(context.witness)
      : valueDigest(context.events.at(-1));
  const proof = sign(
    null,
    colimaLiveProviderEffectProofBytes({
      domain,
      eventSequence: context.events.length,
      evidence,
      parentSha256,
      publicationPlan: context.publicationPlan,
      publisherAuthoritySha256: context.publisherAuthoritySha256,
      source: context.source,
      witness: context.witness,
    }),
    privateKey,
  ).toString("base64");
  return { ...evidence, proof_base64: proof };
}

function prepare(fixtureOnly) {
  const upstream =
    cleanEngineLiveProviderProcessStartFreshAdmissionFixture(fixtureOnly);
  const invocationBinding = {
    argv_sha256: sha256("argv"),
    cwd_identity_sha256: sha256("cwd"),
    environment_sha256: sha256("environment"),
    executable_sha256: sha256("executable"),
    toolchain_sha256: sha256("toolchain"),
  };
  const roleKeys = Object.fromEntries(
    COLIMA_LIVE_PROVIDER_EFFECT_ROLES.map((role) => [role, keyMaterial(role)]),
  );
  const plannedRoleContracts = COLIMA_LIVE_PROVIDER_EFFECT_ROLES.map(
    (role, index) => ({
      argv_sha256:
        role === "outer" ? invocationBinding.argv_sha256 : sha256(`${role}-argv`),
      cwd_identity_sha256:
        role === "outer"
          ? invocationBinding.cwd_identity_sha256
          : sha256(`${role}-cwd`),
      depth: role === "outer" ? 0 : role === "hostagent" ? 1 : 2,
      environment_sha256:
        role === "outer"
          ? invocationBinding.environment_sha256
          : sha256(`${role}-environment`),
      executable_sha256:
        role === "outer"
          ? invocationBinding.executable_sha256
          : sha256(`${role}-executable`),
      parent_role:
        role === "outer"
          ? "state-owner"
          : role === "hostagent"
            ? "outer"
            : "hostagent",
      public_key_spki_sha256: roleKeys[role].publicKeySha256,
      role,
      role_challenge_sha256: sha256(`role-challenge-${index}`),
      toolchain_sha256:
        role === "outer"
          ? invocationBinding.toolchain_sha256
          : sha256(`${role}-toolchain`),
      uid: "501",
    }),
  );
  const endpointRoles = {
    "engine-api": "hostagent",
    "hostagent-control": "hostagent",
    "ssh-control": "ssh-controlmaster",
    "usernet-control": "usernet",
  };
  const endpointNamespaces = {
    "engine-api": "colima-home-namespace",
    "hostagent-control": "lima-home-namespace",
    "ssh-control": "temporary-namespace",
    "usernet-control": "lima-home-namespace",
  };
  const plannedEndpointContracts = COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.map(
    (endpointKind) => ({
      docker_context_namespace_role:
        endpointKind === "engine-api" ? "docker-config-namespace" : "none",
      docker_context_path_identity_hmac_sha256:
        endpointKind === "engine-api"
          ? sha256("docker-context-path-identity")
          : ZERO_SHA256,
      docker_context_sha256:
        endpointKind === "engine-api"
          ? sha256("exact-private-docker-context")
          : ZERO_SHA256,
      endpoint_kind: endpointKind,
      final_challenge_commitment_sha256: valueDigest({
        challenge_sha256: sha256(`${endpointKind}-final-challenge`),
        endpoint_kind: endpointKind,
        phase: "post-detachment-final",
      }),
      initial_challenge_commitment_sha256: valueDigest({
        challenge_sha256: sha256(`${endpointKind}-initial-challenge`),
        endpoint_kind: endpointKind,
        phase: "initial",
      }),
      namespace_role: endpointNamespaces[endpointKind],
      path_identity_hmac_sha256: sha256(`${endpointKind}-path-identity`),
      role: endpointRoles[endpointKind],
    }),
  );
  const namespaceBindings = upstream.admission.root_observation.root_observations.map(
    (observation, index) => {
      return {
        device: "1",
        inode: String(102 + index),
        mode: "0700",
        role: observation.role,
        uid: "501",
      };
    },
  );
  const publicationPlan = buildColimaLiveProviderEffectPublicationPlan({
    admission: upstream.admission,
    invocationBinding,
    namespaceBindings,
    plannedEndpointContracts,
    plannedQuiescenceFenceSha256: sha256("quiescence-fence"),
    plannedRoleContracts,
    plannedStartAttemptSha256: sha256("start-attempt"),
    providerRootIdentity: {
      device: "1",
      inode: "100",
      mode: "0700",
      uid: "501",
    },
    source: upstream.source,
    stateRunIdentity: {
      device: "1",
      inode: "101",
      mode: "0700",
      uid: "501",
    },
  });
  const witness = buildColimaLiveProviderEffectWitness({
    markerWitnessIdentitySha256: sha256("marker-witness-identity"),
    publicationPlan,
    slotSequence: 3,
    slotSha256: sha256("slot"),
    source: upstream.source,
  });
  const publisherAuthorityChain = Object.freeze([
    Object.freeze({
      authority_sha256: witness.slot_sha256,
      first_event_sequence: 0,
      kind: "owner-slot",
      prior_authority_sha256: ZERO_SHA256,
      recovery_claim_sha256: ZERO_SHA256,
    }),
  ]);
  return {
    events: [],
    fixtureOnly,
    publicationPlan,
    publisherAuthorityChain,
    publisherAuthoritySha256: witness.slot_sha256,
    roleKeys,
    source: upstream.source,
    upstream,
    witness,
  };
}

function freezeFixtureValue(value) {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) freezeFixtureValue(child);
    Object.freeze(value);
  }
  return value;
}

function publicFixture(context) {
  return freezeFixtureValue({
    events: context.events,
    fixtureOnly: context.fixtureOnly,
    publicationPlan: context.publicationPlan,
    publisherAuthorityChain: context.publisherAuthorityChain,
    publisherAuthoritySha256: context.publisherAuthoritySha256,
    source: context.source,
    upstream: context.upstream,
    witness: context.witness,
  });
}

function publishStart(context, { includeAttempt = true } = {}) {
  const reproof = (observationSequence) => ({
    admission_sha256: valueDigest(context.publicationPlan.admission),
    invocation_sha256: valueDigest(
      context.publicationPlan.invocation_binding,
    ),
    marker_link_count: 2,
    marker_present: true,
    marker_witness_identity_sha256: sha256("marker-witness-identity"),
    marker_witness_same_inode: true,
    namespace_sha256: valueDigest(context.publicationPlan.namespace_bindings),
    observation_challenge_sha256: sha256(
      `fresh-observation-challenge-${observationSequence}`,
    ),
    observation_sequence: observationSequence,
    provider_root_identity_sha256: valueDigest(
      context.publicationPlan.provider_root_identity,
    ),
    source_sha256: valueDigest(context.source),
    state_run_identity_sha256: valueDigest(
      context.publicationPlan.state_run_identity,
    ),
    topology_sha256: valueDigest({
      endpoints: context.publicationPlan.planned_endpoint_contracts,
      roles: context.publicationPlan.planned_role_contracts,
    }),
    witness_link_count: 2,
  });
  const firstReproof = reproof(0);
  const authority = append(context, "start-authority", {
    authority_scope: "current-in-memory-owner-attempt-publication-only",
    current_owner_boot_sha256: sha256("owner-boot"),
    current_owner_instance_sha256: sha256("owner-instance"),
    first_reproof: firstReproof,
    first_reproof_sha256: valueDigest(firstReproof),
    recovery_invocation_authorized: false,
    serialized_invocation_authorized: false,
    witness_sha256: valueDigest(context.witness),
  });
  if (!includeAttempt) return { authority };
  const secondReproof = reproof(1);
  const attempt = append(context, "start-attempt", {
    authority_event_sha256: valueDigest(authority),
    attempt_sha256: context.publicationPlan.planned_start_attempt_sha256,
    invocation_binding_sha256: valueDigest(
      context.publicationPlan.invocation_binding,
    ),
    invocation_limit: 1,
    recovery_invocation_authorized: false,
    replay_authorized: false,
    second_reproof: secondReproof,
    second_reproof_sha256: valueDigest(secondReproof),
    variant: "attempt-fence",
  });
  return { attempt, authority };
}

function roleIdentityEvidence(context, role, edge) {
  const key = context.roleKeys[role];
  const roleContract = context.publicationPlan.planned_role_contracts.find(
    (entry) => entry.role === role,
  );
  const parentNodeIdentitySha256 = edge.evidence.parent_node_identity_sha256;
  const detached = role === "hostagent";
  const base = {
    argv_sha256: roleContract.argv_sha256,
    boot_identity_sha256: sha256(`${role}-boot`),
    challenge_sha256: roleContract.role_challenge_sha256,
    cwd_identity_sha256: roleContract.cwd_identity_sha256,
    detachment:
      role === "outer"
        ? "foreground-state-owner-child"
        : detached
          ? "detached-reparented"
          : "parent-bound",
    detachment_observation_sha256: detached
      ? sha256(`${role}-detachment`)
      : ZERO_SHA256,
    environment_sha256: roleContract.environment_sha256,
    executable_sha256: roleContract.executable_sha256,
    launch_edge_sha256: valueDigest(edge),
    node_identity_sha256: edge.evidence.child_node_identity_sha256,
    parent_after_detachment_sha256: detached
      ? sha256(`${role}-reaper`)
      : parentNodeIdentitySha256,
    parent_before_detachment_sha256: parentNodeIdentitySha256,
    parent_node_identity_sha256: parentNodeIdentitySha256,
    pgid_identity_sha256: sha256(`${role}-pgid`),
    pid_identity_sha256: sha256(`${role}-pid`),
    proof_base64: "pending",
    public_key_spki_der_base64: key.publicKeyDerBase64,
    role,
    session_identity_sha256: sha256(`${role}-session`),
    start_identity_sha256: sha256(`${role}-start`),
    toolchain_sha256: roleContract.toolchain_sha256,
    uid: roleContract.uid,
  };
  return signedEvidence(context, "role-identity", base, key.privateKey);
}

function publishLaunchEdge(
  context,
  role,
  parentNodeIdentitySha256,
  attempt,
) {
  const key = context.roleKeys[role];
  return append(context, "launch-edge", {
    child_node_identity_sha256: sha256(`${role}-node`),
    child_public_key_spki_der_base64: key.publicKeyDerBase64,
    child_role: role,
    depth: role === "outer" ? 0 : role === "hostagent" ? 1 : 2,
    edge_sequence: context.events.filter(
      (event) => event.event_kind === "launch-edge",
    ).length,
    parent_node_identity_sha256: parentNodeIdentitySha256,
    start_attempt_sha256: valueDigest(attempt),
  });
}

function publishRoleIdentity(context, role, edge) {
  return append(
    context,
    "role-identity",
    roleIdentityEvidence(context, role, edge),
  );
}

function publishRole(context, role, parentNodeIdentitySha256, attempt) {
  const edge = publishLaunchEdge(
    context,
    role,
    parentNodeIdentitySha256,
    attempt,
  );
  const identity = publishRoleIdentity(context, role, edge);
  return { edge, identity };
}

function publishRolesAndDelivery(context, attempt) {
  const owner = context.events.find(
    (event) => event.event_kind === "start-authority",
  );
  const outerEdge = publishLaunchEdge(
    context,
    "outer",
    owner.evidence.current_owner_instance_sha256,
    attempt,
  );
  const delivery = append(context, "delivery-result", {
    attempt_event_sha256: valueDigest(attempt),
    child_handle_identity_sha256: sha256("outer-child-handle"),
    delivery_disposition: "delivery-accepted-effect-possible",
    driver_contract_sha256: context.publicationPlan.driver_contract_sha256,
    effect_possible: true,
    observation_sha256: sha256("delivery-accepted"),
    outer_launch_edge_sha256: valueDigest(outerEdge),
    safe_error_code: "accepted",
  });
  const outer = {
    edge: outerEdge,
    identity: publishRoleIdentity(context, "outer", outerEdge),
  };
  const hostagent = publishRole(
    context,
    "hostagent",
    outer.identity.evidence.node_identity_sha256,
    attempt,
  );
  const usernet = publishRole(
    context,
    "usernet",
    hostagent.identity.evidence.node_identity_sha256,
    attempt,
  );
  const ssh = publishRole(
    context,
    "ssh-controlmaster",
    hostagent.identity.evidence.node_identity_sha256,
    attempt,
  );
  return { delivery, hostagent, outer, ssh, usernet };
}

function publishEndpoints(context, roles) {
  const byRole = {
    hostagent: roles.hostagent.identity,
    "ssh-controlmaster": roles.ssh.identity,
    usernet: roles.usernet.identity,
  };
  const endpointRoles = {
    "engine-api": "hostagent",
    "hostagent-control": "hostagent",
    "ssh-control": "ssh-controlmaster",
    "usernet-control": "usernet",
  };
  const priorEndpoints = new Map();
  for (const phase of ["initial", "post-detachment-final"]) {
    const initialEndpointSetSha256 =
      phase === "initial"
        ? ZERO_SHA256
        : valueDigest(
            context.events
              .filter(
                (event) =>
                  event.event_kind === "endpoint" &&
                  event.evidence.phase === "initial",
              )
              .map(valueDigest),
          );
    for (const endpointKind of COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS) {
      const role = endpointRoles[endpointKind];
      const roleIdentity = byRole[role];
      const engine = endpointKind === "engine-api";
      const socketIdentitySha256 = sha256(
        `${endpointKind}-socket-identity`,
      );
      const planned = context.publicationPlan.planned_endpoint_contracts.find(
        (entry) => entry.endpoint_kind === endpointKind,
      );
      const priorEndpoint = priorEndpoints.get(endpointKind) ?? null;
      const dockerContextBindingSha256 = engine
        ? valueDigest({
            docker_context_namespace_role:
              planned.docker_context_namespace_role,
            docker_context_path_identity_hmac_sha256:
              planned.docker_context_path_identity_hmac_sha256,
            docker_context_sha256: planned.docker_context_sha256,
            path_identity_hmac_sha256: planned.path_identity_hmac_sha256,
            socket_identity_hmac_sha256: socketIdentitySha256,
          })
        : ZERO_SHA256;
      const base = {
        activation_authorized: false,
        ambient_context_authorized: false,
        challenge_sha256: sha256(`${endpointKind}-${phase === "initial" ? "initial" : "final"}-challenge`),
        connection_observation_sha256: sha256(
          `${endpointKind}-${phase}-connection-observation`,
        ),
        docker_context_binding_sha256: dockerContextBindingSha256,
        docker_context_namespace_role:
          planned.docker_context_namespace_role,
        docker_context_path_identity_hmac_sha256:
          planned.docker_context_path_identity_hmac_sha256,
        docker_context_sha256: planned.docker_context_sha256,
        endpoint_kind: endpointKind,
        hostagent_detachment_observation_sha256:
          roles.hostagent.identity.evidence.detachment_observation_sha256,
        initial_endpoint_set_sha256: initialEndpointSetSha256,
        namespace_role: planned.namespace_role,
        path_identity_hmac_sha256: planned.path_identity_hmac_sha256,
        phase,
        planned_quiescence_fence_sha256:
          context.publicationPlan.planned_quiescence_fence_sha256,
        private_engine_socket_identity_hmac_sha256: engine
          ? socketIdentitySha256
          : ZERO_SHA256,
        prior_endpoint_event_sha256:
          priorEndpoint === null ? ZERO_SHA256 : valueDigest(priorEndpoint),
        proof_base64: "pending",
        role_identity_sha256: valueDigest(roleIdentity),
        socket_identity_hmac_sha256: socketIdentitySha256,
      };
      const endpoint = append(
        context,
        "endpoint",
        signedEvidence(
          context,
          "endpoint",
          base,
          context.roleKeys[role].privateKey,
        ),
      );
      priorEndpoints.set(endpointKind, endpoint);
    }
  }
}

function publishQuiescence(context, attempt) {
  const authority = context.events.find(
    (event) => event.event_kind === "start-authority",
  );
  const launchEdges = context.events.filter(
    (event) => event.event_kind === "launch-edge",
  );
  const processReconciliation = {
    observation_challenge_sha256: sha256(
      "process-reconciliation-challenge",
    ),
    observed_edge_set_sha256: valueDigest(
      launchEdges.map((event) => ({
        child_node_identity_sha256:
          event.evidence.child_node_identity_sha256,
        depth: event.evidence.depth,
        parent_node_identity_sha256:
          event.evidence.parent_node_identity_sha256,
      })),
    ),
    observed_node_set_sha256: valueDigest(
      launchEdges.map((event) => event.evidence.child_node_identity_sha256),
    ),
    observed_process_count: launchEdges.length,
    process_observation_sha256: sha256("process-reconciliation-observation"),
  };
  const fence = append(context, "quiescence-fence", {
    authority_event_sha256: valueDigest(authority),
    deadline_milliseconds: 900_000,
    fence_sha256: context.publicationPlan.planned_quiescence_fence_sha256,
    graph_frontier_sha256: valueDigest({
      endpoints: context.events
        .filter((event) => event.event_kind === "endpoint")
        .map(valueDigest),
      launch_edges: context.events
        .filter((event) => event.event_kind === "launch-edge")
        .map(valueDigest),
      role_identities: context.events
        .filter((event) => event.event_kind === "role-identity")
        .map(valueDigest),
    }),
    process_reconciliation: processReconciliation,
    process_reconciliation_sha256: valueDigest(processReconciliation),
    start_attempt_event_sha256: valueDigest(attempt),
  });
  for (const role of COLIMA_LIVE_PROVIDER_EFFECT_ROLES) {
    const roleIdentity = context.events.find(
      (event) =>
        event.event_kind === "role-identity" && event.evidence.role === role,
    );
    const disposition = "no-new-descendants-endpoints-or-resources";
    const outgoingEdgeSetSha256 = valueDigest(
      launchEdges
        .filter(
          (event) =>
            event.evidence.parent_node_identity_sha256 ===
            roleIdentity.evidence.node_identity_sha256,
        )
        .map(valueDigest),
    );
    const endpointFrontierSha256 = valueDigest(
      context.events
        .filter(
          (event) =>
            event.event_kind === "endpoint" &&
            event.evidence.role_identity_sha256 === valueDigest(roleIdentity),
        )
        .map(valueDigest),
    );
    const resourceFrontierObservationSha256 = sha256(
      `${role}-resource-frontier-observation`,
    );
    const base = {
      acknowledgement_sha256: valueDigest({
        disposition,
        endpoint_frontier_sha256: endpointFrontierSha256,
        fence_event_sha256: valueDigest(fence),
        outgoing_edge_set_sha256: outgoingEdgeSetSha256,
        resource_frontier_observation_sha256:
          resourceFrontierObservationSha256,
        role_identity_sha256: valueDigest(roleIdentity),
      }),
      disposition,
      endpoint_frontier_sha256: endpointFrontierSha256,
      fence_event_sha256: valueDigest(fence),
      outgoing_edge_set_sha256: outgoingEdgeSetSha256,
      proof_base64: "pending",
      resource_frontier_observation_sha256:
        resourceFrontierObservationSha256,
      role,
      role_identity_sha256: valueDigest(roleIdentity),
    };
    append(
      context,
      "quiescence-ack",
      signedEvidence(
        context,
        "quiescence-ack",
        base,
        context.roleKeys[role].privateKey,
      ),
    );
  }
  return fence;
}

function ownershipBinding(
  context,
  namespaceBinding,
  relativeIdentityHmacSha256,
  ownership,
) {
  return valueDigest({
    baseline_set_binding_sha256: namespaceBinding.baseline_set_binding_sha256,
    namespace_identity_hmac_sha256:
      namespaceBinding.namespace_identity_hmac_sha256,
    ownership,
    relative_identity_hmac_sha256: relativeIdentityHmacSha256,
    witness_sha256:
      ownership === "generation-owned"
        ? valueDigest(context.witness)
        : ZERO_SHA256,
  });
}

function inventoryRoot(context, namespaceRole, namespaceBinding, childCount) {
  const relativeIdentityHmacSha256 =
    namespaceBinding.namespace_identity_hmac_sha256;
  return {
    content_disposition: "metadata-only",
    content_sha256: ZERO_SHA256,
    ctime_nanoseconds: "1000000000",
    depth: 0,
    device: namespaceBinding.device,
    directory_entry_count: childCount,
    endpoint_sha256: ZERO_SHA256,
    entry_sequence: 0,
    hard_link_group_sha256: ZERO_SHA256,
    inode: namespaceBinding.inode,
    links: "1",
    mode: "0700",
    mtime_nanoseconds: "1000000000",
    name_bytes: Buffer.byteLength(namespaceRole, "utf8"),
    ownership: "namespace-root",
    ownership_binding_sha256: ownershipBinding(
      context,
      namespaceBinding,
      relativeIdentityHmacSha256,
      "namespace-root",
    ),
    parent_identity_hmac_sha256: ZERO_SHA256,
    relative_identity_hmac_sha256: relativeIdentityHmacSha256,
    resource_kind: "none",
    size: "0",
    symlink_target_sha256: ZERO_SHA256,
    type: "directory",
    uid: "501",
  };
}

function inventorySocket(
  context,
  endpoint,
  endpointIndex,
  entrySequence,
  namespaceBinding,
) {
  return {
    content_disposition: "endpoint-bound",
    content_sha256: ZERO_SHA256,
    ctime_nanoseconds: "1000000100",
    depth: 1,
    device: namespaceBinding.device,
    directory_entry_count: 0,
    endpoint_sha256: valueDigest(endpoint),
    entry_sequence: entrySequence,
    hard_link_group_sha256: ZERO_SHA256,
    inode: String(1_000 + endpointIndex),
    links: "1",
    mode: "0600",
    mtime_nanoseconds: "1000000100",
    name_bytes: Buffer.byteLength(endpoint.evidence.endpoint_kind, "utf8"),
    ownership: "generation-owned",
    ownership_binding_sha256: ownershipBinding(
      context,
      namespaceBinding,
      endpoint.evidence.path_identity_hmac_sha256,
      "generation-owned",
    ),
    parent_identity_hmac_sha256:
      namespaceBinding.namespace_identity_hmac_sha256,
    relative_identity_hmac_sha256:
      endpoint.evidence.path_identity_hmac_sha256,
    resource_kind: "none",
    size: "0",
    symlink_target_sha256: ZERO_SHA256,
    type: "socket",
    uid: namespaceBinding.uid,
  };
}

function inventoryBaselineEntries(context, namespaceBinding) {
  if (namespaceBinding.role !== "lima-home-namespace") return [];
  const networkConfigBytes = cleanEngineLiveBaselineNetworkConfigBytesFixture();
  const [configurationIdentity, networkIdentity] =
    namespaceBinding.baseline_relative_identity_hmac_sha256;
  return [
    {
      content_disposition: "metadata-only",
      content_sha256: ZERO_SHA256,
      ctime_nanoseconds: "1000000200",
      depth: 1,
      device: namespaceBinding.device,
      directory_entry_count: 1,
      endpoint_sha256: ZERO_SHA256,
      entry_sequence: 0,
      hard_link_group_sha256: ZERO_SHA256,
      inode: "900",
      links: "1",
      mode: "0700",
      mtime_nanoseconds: "1000000200",
      name_bytes: Buffer.byteLength("_config", "utf8"),
      ownership: "admitted-baseline",
      ownership_binding_sha256: ownershipBinding(
        context,
        namespaceBinding,
        configurationIdentity,
        "admitted-baseline",
      ),
      parent_identity_hmac_sha256:
        namespaceBinding.namespace_identity_hmac_sha256,
      relative_identity_hmac_sha256: configurationIdentity,
      resource_kind: "none",
      size: "0",
      symlink_target_sha256: ZERO_SHA256,
      type: "directory",
      uid: namespaceBinding.uid,
    },
    {
      content_disposition: "hashed",
      content_sha256: sha256(networkConfigBytes),
      ctime_nanoseconds: "1000000300",
      depth: 2,
      device: namespaceBinding.device,
      directory_entry_count: 0,
      endpoint_sha256: ZERO_SHA256,
      entry_sequence: 0,
      hard_link_group_sha256: ZERO_SHA256,
      inode: "901",
      links: "1",
      mode: "0600",
      mtime_nanoseconds: "1000000300",
      name_bytes: Buffer.byteLength("networks.yaml", "utf8"),
      ownership: "admitted-baseline",
      ownership_binding_sha256: ownershipBinding(
        context,
        namespaceBinding,
        networkIdentity,
        "admitted-baseline",
      ),
      parent_identity_hmac_sha256: configurationIdentity,
      relative_identity_hmac_sha256: networkIdentity,
      resource_kind: "none",
      size: String(networkConfigBytes.length),
      symlink_target_sha256: ZERO_SHA256,
      type: "regular",
      uid: namespaceBinding.uid,
    },
  ];
}

function inventoryDockerContext(context, endpoint, namespaceBinding) {
  return {
    content_disposition: "hashed",
    content_sha256: endpoint.evidence.docker_context_sha256,
    ctime_nanoseconds: "1000000400",
    depth: 1,
    device: namespaceBinding.device,
    directory_entry_count: 0,
    endpoint_sha256: valueDigest(endpoint),
    entry_sequence: 0,
    hard_link_group_sha256: ZERO_SHA256,
    inode: "950",
    links: "1",
    mode: "0600",
    mtime_nanoseconds: "1000000400",
    name_bytes: Buffer.byteLength("meta.json", "utf8"),
    ownership: "generation-owned",
    ownership_binding_sha256: ownershipBinding(
      context,
      namespaceBinding,
      endpoint.evidence.docker_context_path_identity_hmac_sha256,
      "generation-owned",
    ),
    parent_identity_hmac_sha256:
      namespaceBinding.namespace_identity_hmac_sha256,
    relative_identity_hmac_sha256:
      endpoint.evidence.docker_context_path_identity_hmac_sha256,
    resource_kind: "docker-context",
    size: "256",
    symlink_target_sha256: ZERO_SHA256,
    type: "regular",
    uid: namespaceBinding.uid,
  };
}

function richInventoryEntry(
  context,
  namespaceBinding,
  {
    contentDisposition = "hashed",
    contentSha256,
    depth,
    directoryEntryCount = 0,
    hardLinkGroupSha256 = ZERO_SHA256,
    inode,
    label,
    links = "1",
    parentIdentityHmacSha256,
    resourceKind = "none",
    size = "16",
    symlinkTargetSha256 = ZERO_SHA256,
    type = "regular",
  },
) {
  const relativeIdentityHmacSha256 = sha256(`rich-inventory:${label}`);
  return {
    content_disposition: contentDisposition,
    content_sha256:
      contentSha256 ??
      (contentDisposition === "hashed"
        ? sha256(`rich-inventory-content:${label}`)
        : ZERO_SHA256),
    ctime_nanoseconds: "1000000500",
    depth,
    device: namespaceBinding.device,
    directory_entry_count: directoryEntryCount,
    endpoint_sha256: ZERO_SHA256,
    entry_sequence: 0,
    hard_link_group_sha256: hardLinkGroupSha256,
    inode,
    links,
    mode: type === "directory" ? "0700" : "0600",
    mtime_nanoseconds: "1000000500",
    name_bytes: Buffer.byteLength(label, "utf8"),
    ownership: "generation-owned",
    ownership_binding_sha256: ownershipBinding(
      context,
      namespaceBinding,
      relativeIdentityHmacSha256,
      "generation-owned",
    ),
    parent_identity_hmac_sha256: parentIdentityHmacSha256,
    relative_identity_hmac_sha256: relativeIdentityHmacSha256,
    resource_kind: resourceKind,
    size,
    symlink_target_sha256: symlinkTargetSha256,
    type,
    uid: namespaceBinding.uid,
  };
}

function richInventoryEntries(context, namespaceBinding) {
  if (namespaceBinding.role !== "colima-home-namespace") return [];
  const rootIdentity = namespaceBinding.namespace_identity_hmac_sha256;
  const directoryIdentity = sha256("rich-inventory:generated");
  const nestedIdentity = sha256("rich-inventory:generated-nested");
  const hardLinkGroupSha256 = sha256("rich-inventory-hard-link-group");
  const hardLinkContentSha256 = sha256("rich-inventory-hard-link-content");
  const entries = [
    richInventoryEntry(context, namespaceBinding, {
      contentDisposition: "metadata-only",
      depth: 1,
      directoryEntryCount: 1,
      inode: "2000",
      label: "generated",
      parentIdentityHmacSha256: rootIdentity,
      size: "0",
      type: "directory",
    }),
    richInventoryEntry(context, namespaceBinding, {
      contentDisposition: "metadata-only",
      depth: 2,
      directoryEntryCount: 69,
      inode: "2001",
      label: "generated-nested",
      parentIdentityHmacSha256: directoryIdentity,
      size: "0",
      type: "directory",
    }),
    richInventoryEntry(context, namespaceBinding, {
      depth: 3,
      inode: "2002",
      label: "regular",
      parentIdentityHmacSha256: nestedIdentity,
    }),
    richInventoryEntry(context, namespaceBinding, {
      contentDisposition: "target-hashed-no-follow",
      depth: 3,
      inode: "2003",
      label: "symlink",
      parentIdentityHmacSha256: nestedIdentity,
      size: "18",
      symlinkTargetSha256: sha256("rich-inventory-symlink-target"),
      type: "symlink",
    }),
    richInventoryEntry(context, namespaceBinding, {
      contentSha256: hardLinkContentSha256,
      depth: 3,
      hardLinkGroupSha256,
      inode: "2004",
      label: "hard-link-a",
      links: "2",
      parentIdentityHmacSha256: nestedIdentity,
    }),
    richInventoryEntry(context, namespaceBinding, {
      contentSha256: hardLinkContentSha256,
      depth: 3,
      hardLinkGroupSha256,
      inode: "2004",
      label: "hard-link-b",
      links: "2",
      parentIdentityHmacSha256: nestedIdentity,
    }),
    richInventoryEntry(context, namespaceBinding, {
      contentDisposition: "metadata-only-closed-provider-resource",
      depth: 3,
      inode: "2005",
      label: "disk.img",
      parentIdentityHmacSha256: nestedIdentity,
      resourceKind: "closed-disk-image",
      size: "1048576",
    }),
  ];
  for (let index = 0; index < 64; index += 1) {
    entries.push(
      richInventoryEntry(context, namespaceBinding, {
        depth: 3,
        inode: String(2_100 + index),
        label: `leaf-${String(index).padStart(2, "0")}`,
        parentIdentityHmacSha256: nestedIdentity,
      }),
    );
  }
  return entries;
}

function inventorySampleSha256(pages) {
  return valueDigest({
    page_count: pages.length,
    page_sha256: pages.map((page) =>
      valueDigest({
        entries: page.evidence.entries,
        entry_start: page.evidence.entry_start,
        namespace_role: page.evidence.namespace_role,
        page_sequence: page.evidence.page_sequence,
      }),
    ),
  });
}

function publishInventory(context, fence, { rich = false } = {}) {
  const entries = new Map();
  const pages = new Map();
  const finalEndpoints = context.events.filter(
    (event) =>
      event.event_kind === "endpoint" &&
      event.evidence.phase === "post-detachment-final",
  );
  for (const [namespaceIndex, namespaceRole] of
    COLIMA_LIVE_MUTATION_SURFACE_ROLES.entries()) {
    const namespaceBinding =
      context.publicationPlan.namespace_bindings[namespaceIndex];
    const namespaceEndpoints = finalEndpoints.filter(
      (endpoint) =>
        endpoint.evidence.namespace_role === namespaceRole,
    );
    const engineEndpoint = finalEndpoints.find(
      (endpoint) => endpoint.evidence.endpoint_kind === "engine-api",
    );
    const children = [
      ...inventoryBaselineEntries(context, namespaceBinding),
      ...(rich ? richInventoryEntries(context, namespaceBinding) : []),
      ...(namespaceRole === "docker-config-namespace"
        ? [inventoryDockerContext(context, engineEndpoint, namespaceBinding)]
        : []),
      ...namespaceEndpoints.map((endpoint) =>
        inventorySocket(
          context,
          endpoint,
          COLIMA_LIVE_PROVIDER_EFFECT_ENDPOINTS.indexOf(
            endpoint.evidence.endpoint_kind,
          ),
          0,
          namespaceBinding,
        ),
      ),
    ].map((entry, index) => ({ ...entry, entry_sequence: index + 1 }));
    entries.set(namespaceRole, [
      inventoryRoot(
        context,
        namespaceRole,
        namespaceBinding,
        children.filter((entry) => entry.depth === 1).length,
      ),
      ...children,
    ]);
  }
  for (const sampleSequence of [0, 1]) {
    for (const namespaceRole of COLIMA_LIVE_MUTATION_SURFACE_ROLES) {
      const namespacePages = [];
      const namespaceEntries = entries.get(namespaceRole);
      const pageSize =
        COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.inventory_entries_per_page;
      for (let entryStart = 0; entryStart < namespaceEntries.length;
        entryStart += pageSize) {
        const page = append(context, "recursive-inventory-page", {
          entries: namespaceEntries.slice(entryStart, entryStart + pageSize),
          entry_start: entryStart,
          namespace_role: namespaceRole,
          page_sequence: namespacePages.length,
          parent_page_sha256:
            namespacePages.length === 0
              ? ZERO_SHA256
              : valueDigest(namespacePages.at(-1)),
          sample_sequence: sampleSequence,
        });
        namespacePages.push(page);
      }
      pages.set(`${sampleSequence}:${namespaceRole}`, namespacePages);
    }
  }
  const samples = COLIMA_LIVE_MUTATION_SURFACE_ROLES.map((namespaceRole) => ({
    entry_count: entries.get(namespaceRole).length,
    first_sample_sha256: inventorySampleSha256(
      pages.get(`0:${namespaceRole}`),
    ),
    namespace_role: namespaceRole,
    page_count: pages.get(`0:${namespaceRole}`).length,
    second_sample_sha256: inventorySampleSha256(
      pages.get(`1:${namespaceRole}`),
    ),
  }));
  return append(context, "recursive-inventory-set", {
    inventory_sha256: valueDigest(samples),
    quiescence_fence_sha256: valueDigest(fence),
    sample_count: 2,
    samples,
  });
}

function publishLiveSettlement(context, attempt, delivery, fence, inventory) {
  const launchEdges = context.events
    .filter((event) => event.event_kind === "launch-edge")
    .map(valueDigest);
  const roleIdentities = context.events
    .filter((event) => event.event_kind === "role-identity")
    .map(valueDigest);
  const endpoints = context.events
    .filter(
      (event) =>
        event.event_kind === "endpoint" &&
        event.evidence.phase === "post-detachment-final",
    )
    .map(valueDigest);
  return append(context, "create-settlement", {
    absence_observation_sha256: ZERO_SHA256,
    causal_edge_count: launchEdges.length,
    causal_graph_sha256: valueDigest({
      launch_edges: launchEdges,
      role_identities: roleIdentities,
    }),
    causal_node_count: roleIdentities.length,
    delivery_result_sha256: valueDigest(delivery),
    endpoint_set_sha256: valueDigest(endpoints),
    inventory_set_sha256: valueDigest(inventory),
    marker_link_count: 2,
    quiescence_fence_sha256: valueDigest(fence),
    residual_basis: "none",
    role_identity_set_sha256: valueDigest(roleIdentities),
    start_attempt_sha256: valueDigest(attempt),
    variant: "authenticated-live-identity",
  });
}

function publishCleanup(context, createSettlement) {
  const actions = deriveColimaLiveFixtureProviderEffectCleanupActions({
    history: context.events,
    publicationPlan: context.publicationPlan,
    publisherAuthorityChain: context.publisherAuthorityChain,
    source: context.source,
    witness: context.witness,
  });
  const pages = [];
  const planPageSize =
    COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.canonical_container_entries;
  for (let actionStart = 0; actionStart < actions.length;
    actionStart += planPageSize) {
    const page = append(context, "cleanup-plan-page", {
      action_start: actionStart,
      actions: actions.slice(actionStart, actionStart + planPageSize),
      create_settlement_sha256: valueDigest(createSettlement),
      page_sequence: pages.length,
      parent_page_sha256:
        pages.length === 0 ? ZERO_SHA256 : valueDigest(pages.at(-1)),
    });
    pages.push(page);
  }
  const plan = append(context, "cleanup-plan", {
    action_count: actions.length,
    create_settlement_sha256: valueDigest(createSettlement),
    marker_link_count: 2,
    page_count: pages.length,
    page_set_sha256: valueDigest({
      page_count: pages.length,
      terminal_page_sha256: valueDigest(pages.at(-1)),
    }),
    reverse_causal_order: true,
  });
  const progressEvents = [];
  const outcomePageSize =
    COLIMA_LIVE_PROVIDER_EFFECT_BOUNDS.cleanup_outcomes_per_page;
  for (let actionStart = 0; actionStart < actions.length;
    actionStart += outcomePageSize) {
    const parentProgressSha256 =
      progressEvents.length === 0
        ? ZERO_SHA256
        : valueDigest(progressEvents.at(-1));
    const outcomes = actions
      .slice(actionStart, actionStart + outcomePageSize)
      .map((action) => {
      const actionSha256 = valueDigest(action);
      const markerWitnessObservationSha256 = sha256(
        `cleanup-marker-witness-${action.action_sequence}`,
      );
      const observationChallengeSha256 = sha256(
        `cleanup-challenge-${action.action_sequence}`,
      );
      const outcomeObservationSha256 = sha256(
        `cleanup-outcome-${action.action_sequence}`,
      );
      const parentFsyncObservationSha256 = action.parent_fsync_required
        ? sha256(`cleanup-parent-fsync-${action.action_sequence}`)
        : ZERO_SHA256;
      const preconditionObservationSha256 = sha256(
        `cleanup-precondition-${action.action_sequence}`,
      );
      const recursivePrefixObservationSha256 = sha256(
        `cleanup-recursive-prefix-${action.action_sequence}`,
      );
      const sourceReproofObservationSha256 = sha256(
        `cleanup-source-reproof-${action.action_sequence}`,
      );
      return {
        action_sequence: action.action_sequence,
        action_sha256: actionSha256,
        disposition: "completed-exact",
        marker_witness_observation_sha256:
          markerWitnessObservationSha256,
        observation_binding_sha256: valueDigest({
          action_sha256: actionSha256,
          authority_sha256: context.publisherAuthoritySha256,
          cleanup_plan_sha256: valueDigest(plan),
          expected_postcondition_sha256:
            action.expected_postcondition_sha256,
          expected_precondition_sha256:
            action.expected_precondition_sha256,
          marker_link_count: 2,
          marker_witness_observation_sha256:
            markerWitnessObservationSha256,
          observation_challenge_sha256: observationChallengeSha256,
          operation_plan_sha256: valueDigest(context.publicationPlan),
          outcome_observation_sha256: outcomeObservationSha256,
          parent_fsync_observation_sha256:
            parentFsyncObservationSha256,
          parent_progress_sha256: parentProgressSha256,
          precondition_observation_sha256:
            preconditionObservationSha256,
          recursive_prefix_observation_sha256:
            recursivePrefixObservationSha256,
          source_reproof_observation_sha256:
            sourceReproofObservationSha256,
          source_sha256: valueDigest(context.source),
          witness_sha256: valueDigest(context.witness),
        }),
        observation_challenge_sha256: observationChallengeSha256,
        outcome_observation_sha256: outcomeObservationSha256,
        parent_fsync_observation_sha256: parentFsyncObservationSha256,
        precondition_observation_sha256: preconditionObservationSha256,
        recursive_prefix_observation_sha256:
          recursivePrefixObservationSha256,
        source_reproof_observation_sha256:
          sourceReproofObservationSha256,
      };
      });
    progressEvents.push(
      append(context, "cleanup-progress", {
        action_start: actionStart,
        authority_sha256: context.publisherAuthoritySha256,
        cleanup_plan_sha256: valueDigest(plan),
        outcomes,
        parent_progress_sha256: parentProgressSha256,
      }),
    );
  }
  const expectedBaselineSha256 = valueDigest(
    context.publicationPlan.namespace_bindings.map((binding) => ({
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
  const endpointAbsenceObservationSha256 = sha256(
    "cleanup-endpoint-absence-observation",
  );
  const markerWitnessTopologyObservationSha256 = sha256(
    "cleanup-marker-witness-topology-observation",
  );
  const roleAbsenceObservationSha256 = sha256(
    "cleanup-role-absence-observation",
  );
  const sourceReproofObservationSha256 = sha256(
    "cleanup-source-reproof-observation",
  );
  return append(context, "cleanup-settlement", {
    cleanup_plan_sha256: valueDigest(plan),
    create_settlement_sha256: valueDigest(createSettlement),
    endpoint_absence_sha256: valueDigest({
      disposition: "absent",
      endpoints: context.events
        .filter(
          (event) =>
            event.event_kind === "endpoint" &&
            event.evidence.phase === "post-detachment-final",
        )
        .map((event) => event.evidence.socket_identity_hmac_sha256),
      observation_sha256: endpointAbsenceObservationSha256,
    }),
    endpoint_absence_observation_sha256: endpointAbsenceObservationSha256,
    final_inventory_sha256: expectedBaselineSha256,
    marker_link_count: 2,
    marker_witness_topology_sha256: valueDigest({
      marker_link_count: 2,
      marker_present: true,
      observation_sha256: markerWitnessTopologyObservationSha256,
      witness_link_count: 2,
      witness_sha256: valueDigest(context.witness),
    }),
    marker_witness_topology_observation_sha256:
      markerWitnessTopologyObservationSha256,
    namespace_baselines_sha256: expectedBaselineSha256,
    progress_count: actions.length,
    progress_head_sha256: valueDigest(progressEvents.at(-1)),
    role_absence_sha256: valueDigest({
      disposition: "absent",
      node_identities: context.events
        .filter((event) => event.event_kind === "launch-edge")
        .map((event) => event.evidence.child_node_identity_sha256),
      observation_sha256: roleAbsenceObservationSha256,
    }),
    role_absence_observation_sha256: roleAbsenceObservationSha256,
    source_reproof_sha256: valueDigest({
      cleanup_plan_sha256: valueDigest(plan),
      observation_sha256: sourceReproofObservationSha256,
      source_sha256: valueDigest(context.source),
      witness_sha256: valueDigest(context.witness),
    }),
    source_reproof_observation_sha256: sourceReproofObservationSha256,
    terminal_receipt_publication_authorized: true,
  });
}

function terminalReceiptBinding(
  context,
  startAttempt,
  createSettlement,
  cleanupSettlement,
) {
  return {
    cleanup_settlement_sha256: valueDigest(cleanupSettlement),
    create_settlement_sha256: valueDigest(createSettlement),
    evidence_class: context.publicationPlan.evidence_class,
    operation_contract_sha256:
      context.publicationPlan.operation_contract_sha256,
    operation_kind: context.publicationPlan.operation_kind,
    operation_plan_sha256: valueDigest(context.publicationPlan),
    phase: "provider-effect-retired",
    publisher_authority_sha256: context.publisherAuthoritySha256,
    schema: "synveda.clean-engine.provider-effect-receipt-binding.v1",
    slot_sequence: context.witness.slot_sequence,
    slot_sha256: context.witness.slot_sha256,
    start_attempt_sha256: valueDigest(startAttempt),
    witness_sha256: valueDigest(context.witness),
  };
}

function terminalReceipt(context, result) {
  return {
    fixture_id: context.publicationPlan.fixture_id,
    outcome: "passed",
    phase: "provider-effect-retired",
    previous_sha256: context.publicationPlan.receipt_previous_sha256,
    result,
    schema: "synveda.clean-engine.receipt.v6",
    sequence: context.publicationPlan.receipt_sequence,
  };
}

function publishTerminalReceipt(
  context,
  startAttempt,
  createSettlement,
  cleanupSettlement,
) {
  const result = terminalReceiptBinding(
    context,
    startAttempt,
    createSettlement,
    cleanupSettlement,
  );
  const receipt = terminalReceipt(context, result);
  const event = append(context, "terminal-receipt", {
    cleanup_settlement_sha256: valueDigest(cleanupSettlement),
    marker_link_count: 2,
    receipt,
    receipt_publication_observation_sha256: sha256(
      `receipt-publication-${context.events.length}`,
    ),
    receipt_sha256: valueDigest(receipt),
    witness_link_count: 2,
  });
  return { event, receipt };
}

function markerRetirementBinding(
  context,
  cleanupSettlementSha256,
  markerRetirementObservationSha256,
  providerRootFsyncObservationSha256,
  receiptSha256,
) {
  return valueDigest({
    cleanup_settlement_sha256: cleanupSettlementSha256,
    marker_retirement_observation_sha256:
      markerRetirementObservationSha256,
    provider_root_fsync_observation_sha256:
      providerRootFsyncObservationSha256,
    publisher_authority_sha256: context.publisherAuthoritySha256,
    receipt_sha256: receiptSha256,
    witness_identity_sha256: context.witness.marker_witness_identity_sha256,
    witness_link_count: 1,
  });
}

export function cleanEngineLiveProviderEffectFixture(input = {}) {
  if (
    isProxy(input) ||
    input === null ||
    Array.isArray(input) ||
    typeof input !== "object" ||
    Object.getPrototypeOf(input) !== Object.prototype ||
    Reflect.ownKeys(input).some(
      (key) => typeof key !== "string" || key !== "outcome",
    )
  ) {
    throw new TypeError("live provider effect fixture input was refused");
  }
  const descriptor = Object.getOwnPropertyDescriptor(input, "outcome");
  if (
    descriptor !== undefined &&
    (!("value" in descriptor) || descriptor.enumerable !== true)
  ) {
    throw new TypeError("live provider effect fixture input was refused");
  }
  const outcome = descriptor === undefined ? "retired" : descriptor.value;
  if (
    !new Set([
      "authority-retired",
      "pre-attempt-retired",
      "residual",
      "retired",
      "rich-retired",
      "uncertain",
    ]).has(outcome)
  ) {
    throw new TypeError("live provider effect fixture outcome was refused");
  }
  const context = prepare(true);
  if (outcome === "pre-attempt-retired") {
    const markerRetirementObservationSha256 = sha256(
      "pre-attempt-marker-retirement",
    );
    const providerRootFsyncObservationSha256 = sha256(
      "pre-attempt-provider-root-fsync",
    );
    append(context, "completion", {
      cleanup_settlement_sha256: ZERO_SHA256,
      create_settlement_sha256: ZERO_SHA256,
      marker_present: false,
      marker_retirement_binding_sha256: markerRetirementBinding(
        context,
        ZERO_SHA256,
        markerRetirementObservationSha256,
        providerRootFsyncObservationSha256,
        ZERO_SHA256,
      ),
      marker_retirement_observation_sha256:
        markerRetirementObservationSha256,
      provider_root_fsync_observation_sha256:
        providerRootFsyncObservationSha256,
      receipt_event_sha256: ZERO_SHA256,
      receipt_sha256: ZERO_SHA256,
      start_authority_sha256: ZERO_SHA256,
      start_attempt_sha256: ZERO_SHA256,
      variant: "pre-attempt-retired-zero-receipt",
      witness_identity_sha256: sha256("marker-witness-identity"),
      witness_link_count: 1,
    });
    return publicFixture(context);
  }
  if (outcome === "authority-retired") {
    const { authority } = publishStart(context, { includeAttempt: false });
    const markerRetirementObservationSha256 = sha256(
      "authority-marker-retirement",
    );
    const providerRootFsyncObservationSha256 = sha256(
      "authority-provider-root-fsync",
    );
    append(context, "completion", {
      cleanup_settlement_sha256: ZERO_SHA256,
      create_settlement_sha256: ZERO_SHA256,
      marker_present: false,
      marker_retirement_binding_sha256: markerRetirementBinding(
        context,
        ZERO_SHA256,
        markerRetirementObservationSha256,
        providerRootFsyncObservationSha256,
        ZERO_SHA256,
      ),
      marker_retirement_observation_sha256:
        markerRetirementObservationSha256,
      provider_root_fsync_observation_sha256:
        providerRootFsyncObservationSha256,
      receipt_event_sha256: ZERO_SHA256,
      receipt_sha256: ZERO_SHA256,
      start_authority_sha256: valueDigest(authority),
      start_attempt_sha256: ZERO_SHA256,
      variant: "pre-attempt-retired-zero-receipt",
      witness_identity_sha256:
        authority.evidence.first_reproof.marker_witness_identity_sha256,
      witness_link_count: 1,
    });
    return publicFixture(context);
  }
  const { attempt } = publishStart(context);
  if (outcome === "residual") {
    const owner = context.events.find(
      (event) => event.event_kind === "start-authority",
    );
    const outerEdge = append(context, "launch-edge", {
      child_node_identity_sha256: sha256("outer-node"),
      child_public_key_spki_der_base64:
        context.roleKeys.outer.publicKeyDerBase64,
      child_role: "outer",
      depth: 0,
      edge_sequence: 0,
      parent_node_identity_sha256:
        owner.evidence.current_owner_instance_sha256,
      start_attempt_sha256: valueDigest(attempt),
    });
    const delivery = append(context, "delivery-result", {
      attempt_event_sha256: valueDigest(attempt),
      child_handle_identity_sha256: ZERO_SHA256,
      delivery_disposition: "conclusive-not-created",
      driver_contract_sha256: context.publicationPlan.driver_contract_sha256,
      effect_possible: false,
      observation_sha256: sha256("delivery-refused"),
      outer_launch_edge_sha256: valueDigest(outerEdge),
      safe_error_code: "not-created",
    });
    const createSettlement = append(context, "create-settlement", {
      absence_observation_sha256: sha256("exact-residual-absence"),
      causal_edge_count: 1,
      causal_graph_sha256: valueDigest({
        launch_edges: context.events
          .filter((event) => event.event_kind === "launch-edge")
          .map(valueDigest),
        role_identities: [],
      }),
      causal_node_count: 0,
      delivery_result_sha256: valueDigest(delivery),
      endpoint_set_sha256: valueDigest([]),
      inventory_set_sha256: ZERO_SHA256,
      marker_link_count: 2,
      quiescence_fence_sha256: ZERO_SHA256,
      residual_basis: "conclusive-not-created",
      role_identity_set_sha256: valueDigest([]),
      start_attempt_sha256: valueDigest(attempt),
      variant: "exact-attempted-residual",
    });
    const cleanupSettlement = publishCleanup(context, createSettlement);
    const authority = context.events.find(
      (event) => event.event_kind === "start-authority",
    );
    const publishedReceipt = publishTerminalReceipt(
      context,
      attempt,
      createSettlement,
      cleanupSettlement,
    );
    const markerRetirementObservationSha256 = sha256("marker-retirement");
    const providerRootFsyncObservationSha256 = sha256("provider-root-fsync");
    const receiptSha256 = valueDigest(publishedReceipt.receipt);
    append(context, "completion", {
      cleanup_settlement_sha256: valueDigest(cleanupSettlement),
      create_settlement_sha256: valueDigest(createSettlement),
      marker_present: false,
      marker_retirement_binding_sha256: markerRetirementBinding(
        context,
        valueDigest(cleanupSettlement),
        markerRetirementObservationSha256,
        providerRootFsyncObservationSha256,
        receiptSha256,
      ),
      marker_retirement_observation_sha256:
        markerRetirementObservationSha256,
      provider_root_fsync_observation_sha256:
        providerRootFsyncObservationSha256,
      receipt_event_sha256: valueDigest(publishedReceipt.event),
      receipt_sha256: receiptSha256,
      start_authority_sha256: valueDigest(authority),
      start_attempt_sha256: valueDigest(attempt),
      variant: "attempted-effect-retired",
      witness_identity_sha256: sha256("marker-witness-identity"),
      witness_link_count: 1,
    });
    return publicFixture(context);
  }
  const roles = publishRolesAndDelivery(context, attempt);
  if (outcome === "uncertain") {
    append(context, "uncertain-start", {
      delivery_result_sha256: valueDigest(roles.delivery),
      marker_link_count: 2,
      reason: "endpoint-incomplete",
      recovery_invocation_authorized: false,
      replay_authorized: false,
      start_attempt_sha256: valueDigest(attempt),
      variant: "durable-delivery-incomplete-evidence",
    });
    return publicFixture(context);
  }
  publishEndpoints(context, roles);
  const fence = publishQuiescence(context, attempt);
  const inventory = publishInventory(context, fence, {
    rich: outcome === "rich-retired",
  });
  const createSettlement = publishLiveSettlement(
    context,
    attempt,
    roles.delivery,
    fence,
    inventory,
  );
  const cleanupSettlement = publishCleanup(context, createSettlement);
  const authority = context.events.find(
    (event) => event.event_kind === "start-authority",
  );
  const publishedReceipt = publishTerminalReceipt(
    context,
    attempt,
    createSettlement,
    cleanupSettlement,
  );
  const markerRetirementObservationSha256 = sha256("marker-retirement");
  const providerRootFsyncObservationSha256 = sha256("provider-root-fsync");
  const receiptSha256 = valueDigest(publishedReceipt.receipt);
  append(context, "completion", {
    cleanup_settlement_sha256: valueDigest(cleanupSettlement),
    create_settlement_sha256: valueDigest(createSettlement),
    marker_present: false,
    marker_retirement_binding_sha256: markerRetirementBinding(
      context,
      valueDigest(cleanupSettlement),
      markerRetirementObservationSha256,
      providerRootFsyncObservationSha256,
      receiptSha256,
    ),
    marker_retirement_observation_sha256:
      markerRetirementObservationSha256,
    provider_root_fsync_observation_sha256:
      providerRootFsyncObservationSha256,
    receipt_event_sha256: valueDigest(publishedReceipt.event),
    receipt_sha256: receiptSha256,
    start_authority_sha256: valueDigest(authority),
    start_attempt_sha256: valueDigest(attempt),
    variant: "attempted-effect-retired",
    witness_identity_sha256: sha256("marker-witness-identity"),
    witness_link_count: 1,
  });
  return publicFixture(context);
}

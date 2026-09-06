#!/usr/bin/env node
import { spawn } from "node:child_process";
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  rmdirSync,
  unlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  EVIDENCE_CLASS,
  MAX_GRAPH_BYTES,
  MAX_REQUEST_LIFETIME_MILLISECONDS,
  PHASES,
  ROLE_EDGES,
  ROLES,
  SCHEMAS,
  assertPrivateRoot,
  bootstrapCommitment,
  canonical,
  canonicalBytes,
  closedFixtureEnvironment,
  exactKeys,
  isSha256,
  privateDirectoryIdentity,
  publishExclusive,
  randomSha256,
  readExact,
  readPublished,
  requestFrame,
  remainingDeadlineMilliseconds,
  sameSocketIdentity,
  sha256Bytes,
  sha256Value,
  signedValue,
  socketIdentity,
  syncDirectory,
  verifySignedValue,
} from "./protocol.mjs";

const ROLE_FILE = fileURLToPath(new URL("./role.mjs", import.meta.url));
const PROTOCOL_FILE = fileURLToPath(new URL("./protocol.mjs", import.meta.url));
const PHASE_TIMEOUT_MILLISECONDS = MAX_REQUEST_LIFETIME_MILLISECONDS;
const ROLE_LIFETIME_MILLISECONDS = 45_000;
const INJECTED_PARTIAL_FAILURE = "hostagent-after-first-child-ready";
const INJECTED_PARTIAL_EXIT_CODE = 72;

const PARENT = Object.freeze({
  outer: "fixture-supervisor",
  hostagent: "outer",
  usernet: "hostagent",
  "ssh-controlmaster": "hostagent",
});
const DEPTH = Object.freeze({
  outer: 0,
  hostagent: 1,
  usernet: 2,
  "ssh-controlmaster": 2,
});
const SEQUENCE = Object.freeze({
  outer: 0,
  hostagent: 1,
  usernet: 2,
  "ssh-controlmaster": 3,
});
const CHILDREN = Object.freeze({
  outer: Object.freeze(["hostagent"]),
  hostagent: Object.freeze(["usernet", "ssh-controlmaster"]),
  usernet: Object.freeze([]),
  "ssh-controlmaster": Object.freeze([]),
});
const FILE_STEMS = Object.freeze({
  outer: "o",
  hostagent: "h",
  usernet: "u",
  "ssh-controlmaster": "s",
});

let cachedToolchain;

function toolchain() {
  cachedToolchain ??= {
    node_sha256: sha256Bytes(readFileSync(process.execPath)),
    protocol_sha256: sha256Bytes(readFileSync(PROTOCOL_FILE)),
    role_sha256: sha256Bytes(readFileSync(ROLE_FILE)),
  };
  return cachedToolchain;
}

function makePublicConfig(root, operationId, role, startupDelayMilliseconds) {
  const stem = FILE_STEMS[role];
  return {
    child_roles: CHILDREN[role],
    depth: DEPTH[role],
    detach_ack_path: role === "outer" ? join(root, "d.json") : null,
    edge_path: join(root, "edges", `${SEQUENCE[role]}-${stem}.json`),
    evidence_class: EVIDENCE_CLASS,
    exit_ack_path: join(root, "exit", `${stem}.json`),
    failure_path: join(root, "exit", `${stem}-failure.json`),
    identity_path: join(root, "identity", `${stem}.json`),
    launch_nonce: randomSha256(),
    maximum_lifetime_milliseconds: ROLE_LIFETIME_MILLISECONDS,
    operation_id: operationId,
    parent_role: PARENT[role],
    quiescence_ack_path: join(root, "quiescence", `${stem}.json`),
    quiescence_fence_path: join(root, "f.json"),
    role,
    schema: SCHEMAS.config,
    sequence: SEQUENCE[role],
    socket_path: join(root, `${stem}.sock`),
    startup_delay_milliseconds: startupDelayMilliseconds,
    toolchain: toolchain(),
    workspace: root,
  };
}

function makeDescriptor(publicConfig, fenceKey, children = [], fault = "none") {
  return {
    children,
    edge_key: randomSha256(),
    fault,
    fence_key: fenceKey,
    public: publicConfig,
    role_key: randomSha256(),
  };
}

function createFixtureTree(
  root,
  { failureAt = null, startupDelayMilliseconds = 0 } = {},
) {
  const operationId = randomSha256();
  const fenceKey = randomSha256();
  const usernet = makeDescriptor(
    makePublicConfig(root, operationId, "usernet", startupDelayMilliseconds),
    fenceKey,
  );
  const ssh = makeDescriptor(
    makePublicConfig(
      root,
      operationId,
      "ssh-controlmaster",
      startupDelayMilliseconds,
    ),
    fenceKey,
  );
  const hostagent = makeDescriptor(
    makePublicConfig(root, operationId, "hostagent", startupDelayMilliseconds),
    fenceKey,
    [usernet, ssh],
    failureAt === "hostagent-after-first-child-ready"
      ? "after-first-child-ready"
      : "none",
  );
  const outer = makeDescriptor(
    makePublicConfig(root, operationId, "outer", startupDelayMilliseconds),
    fenceKey,
    [hostagent],
  );
  return {
    descriptors: { hostagent, outer, "ssh-controlmaster": ssh, usernet },
    fenceKey,
    operationId,
    outer,
  };
}

function makePrivateRoot() {
  const root = realpathSync.native(
    resolve(mkdtempSync(join(tmpdir(), "sv-f4-"))),
  );
  chmodSync(root, 0o700);
  for (const name of ["edges", "identity", "quiescence", "exit"]) {
    mkdirSync(join(root, name), { mode: 0o700 });
  }
  assertPrivateRoot(root);
  return root;
}

function expectedRoleArgv(publicConfig) {
  return [
    process.execPath,
    ROLE_FILE,
    publicConfig.role,
    publicConfig.edge_path,
  ];
}

function publishAdmission(descriptor, supervisorIdentitySha256) {
  const publicConfig = descriptor.public;
  const body = {
    argv_sha256: sha256Value(expectedRoleArgv(publicConfig)),
    child_bootstrap_sha256: sha256Value(bootstrapCommitment(descriptor)),
    child_launch_nonce_sha256: sha256Bytes(
      Buffer.from(publicConfig.launch_nonce, "ascii"),
    ),
    child_role: "outer",
    child_role_key_sha256: sha256Bytes(
      Buffer.from(descriptor.role_key, "ascii"),
    ),
    cwd_identity_sha256: sha256Value(assertPrivateRoot(publicConfig.workspace)),
    depth: 0,
    edge_key_sha256: sha256Bytes(Buffer.from(descriptor.edge_key, "ascii")),
    environment_sha256: sha256Value(closedFixtureEnvironment()),
    evidence_class: EVIDENCE_CLASS,
    fence_key_sha256: sha256Bytes(Buffer.from(descriptor.fence_key, "ascii")),
    operation_id: publicConfig.operation_id,
    parent_identity_sha256: supervisorIdentitySha256,
    parent_role: "fixture-supervisor",
    prior_edge_sha256: null,
    schema: SCHEMAS.admission,
    sequence: 0,
    toolchain: publicConfig.toolchain,
  };
  return publishExclusive(
    publicConfig.edge_path,
    signedValue(descriptor.edge_key, "launch-edge", body),
  );
}

function flattenDescriptors(rootDescriptor) {
  const values = [];
  const visit = (descriptor) => {
    values.push(descriptor);
    descriptor.children.forEach(visit);
  };
  visit(rootDescriptor);
  return values;
}

function descriptorMap(rootDescriptor) {
  return new Map(
    flattenDescriptors(rootDescriptor).map((descriptor) => [
      descriptor.public.role,
      descriptor,
    ]),
  );
}

function waitForReady(child, descriptor, admissionSha256) {
  return new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    let stderr = "";
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.off("message", onMessage);
      child.off("exit", onExit);
      if (error === undefined) resolvePromise({ ...value, stderr });
      else rejectPromise(error);
    };
    const onExit = (code, signal) => {
      const error = new Error(
          stderr === ""
            ? "fixture outer exited before readiness"
            : `fixture outer exited before readiness: ${stderr.trim()}`,
      );
      if (
        code === INJECTED_PARTIAL_EXIT_CODE &&
        signal === null &&
        descriptor.children.some(
          (child) => child.fault === "after-first-child-ready",
        )
      ) {
        try {
          const hostagent = descriptor.children.find(
            (child) => child.fault === "after-first-child-ready",
          );
          const failure = readInjectedFailure(hostagent);
          error.fixtureFailure = failure.body.failure_point;
          error.fixtureFailureSha256 = failure.sha256;
        } catch (failureError) {
          finish(failureError);
          return;
        }
      }
      finish(error);
    };
    const onMessage = (message) => {
      try {
        const body = verifySignedValue(
          message,
          descriptor.role_key,
          "ready",
          "fixture outer readiness",
        );
        exactKeys(
          body,
          [
            "causal_head_sha256",
            "edge_sha256",
            "evidence_class",
            "identities",
            "identity_sha256",
            "operation_id",
            "phase",
            "pid",
            "role",
            "schema",
          ],
          "fixture outer readiness body",
        );
        if (
          body.schema !== SCHEMAS.ready ||
          body.evidence_class !== EVIDENCE_CLASS ||
          body.operation_id !== descriptor.public.operation_id ||
          body.role !== "outer" ||
          body.edge_sha256 !== admissionSha256 ||
          body.phase !== "ready" ||
          body.pid !== child.pid ||
          !isSha256(body.identity_sha256) ||
          !isSha256(body.causal_head_sha256) ||
          !Array.isArray(body.identities) ||
          body.identities.length !== 4
        ) {
          throw new Error("fixture outer readiness was refused");
        }
        finish(undefined, { body });
      } catch (error) {
        finish(error);
      }
    };
    const timer = setTimeout(
      () => finish(new Error("fixture outer readiness timed out")),
      PHASE_TIMEOUT_MILLISECONDS,
    );
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      if (stderr.length + chunk.length <= 4_096) stderr += chunk;
    });
    child.once("message", onMessage);
    child.once("exit", onExit);
    child.send({
      descriptor,
      edge_sha256: admissionSha256,
      schema: SCHEMAS.bootstrap,
    });
  });
}

function readIdentity(descriptor, recordSha256) {
  const record = readExact(
    descriptor.public.identity_path,
    recordSha256,
    "fixture identity",
  );
  const body = verifySignedValue(
    record,
    descriptor.role_key,
    "identity",
    "fixture identity",
  );
  exactKeys(
    body,
    [
      "argv_sha256",
      "cwd_identity_sha256",
      "depth",
      "endpoint_identity",
      "environment_sha256",
      "evidence_class",
      "identity_scope",
      "identity_sha256",
      "launch_edge_sha256",
      "launch_nonce_sha256",
      "operation_id",
      "pid",
      "ppid_at_start",
      "process_nonce_sha256",
      "role",
      "schema",
      "toolchain",
      "uid",
    ],
    "fixture identity body",
  );
  const { identity_sha256: identitySha256, ...identityBody } = body;
  if (
    body.schema !== SCHEMAS.identity ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.role !== descriptor.public.role ||
    body.depth !== descriptor.public.depth ||
    body.identity_scope !== "trusted-fixture-nonce-not-os-start-identity" ||
    identitySha256 !== sha256Value(identityBody) ||
    body.argv_sha256 !== sha256Value(expectedRoleArgv(descriptor.public)) ||
    body.environment_sha256 !== sha256Value(closedFixtureEnvironment()) ||
    body.cwd_identity_sha256 !==
      sha256Value(assertPrivateRoot(descriptor.public.workspace)) ||
    body.launch_nonce_sha256 !==
      sha256Bytes(Buffer.from(descriptor.public.launch_nonce, "ascii")) ||
    canonical(body.toolchain) !== canonical(toolchain()) ||
    body.uid !== process.getuid() ||
    !Number.isSafeInteger(body.pid) ||
    body.pid < 2 ||
    !Number.isSafeInteger(body.ppid_at_start) ||
    body.ppid_at_start < 1 ||
    !sameSocketIdentity(
      body.endpoint_identity,
      socketIdentity(descriptor.public.socket_path),
    )
  ) {
    throw new Error("fixture identity did not bind its role process");
  }
  return body;
}

function readEdge(descriptor, edgeSha256) {
  const record = readExact(
    descriptor.public.edge_path,
    edgeSha256,
    "fixture launch edge",
  );
  const body = verifySignedValue(
    record,
    descriptor.edge_key,
    "launch-edge",
    "fixture launch edge",
  );
  return { body, sha256: edgeSha256 };
}

function readInjectedFailure(descriptor) {
  const published = readPublished(
    descriptor.public.failure_path,
    "fixture injected failure",
  );
  const body = verifySignedValue(
    published.value,
    descriptor.role_key,
    "injected-failure",
    "fixture injected failure",
  );
  exactKeys(
    body,
    [
      "causal_head_sha256",
      "child_edge_sha256",
      "child_identity_record_sha256",
      "child_identity_sha256",
      "child_role",
      "evidence_class",
      "failure_point",
      "hostagent_edge_sha256",
      "hostagent_identity_record_sha256",
      "hostagent_identity_sha256",
      "operation_id",
      "schema",
    ],
    "fixture injected failure body",
  );
  const child = descriptor.children[0];
  if (
    descriptor.public.role !== "hostagent" ||
    descriptor.fault !== "after-first-child-ready" ||
    child?.public.role !== "usernet" ||
    body.schema !== SCHEMAS.failure ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.failure_point !== INJECTED_PARTIAL_FAILURE ||
    body.child_role !== child.public.role ||
    body.causal_head_sha256 !== body.child_edge_sha256 ||
    ![
      body.child_edge_sha256,
      body.child_identity_record_sha256,
      body.child_identity_sha256,
      body.hostagent_edge_sha256,
      body.hostagent_identity_record_sha256,
      body.hostagent_identity_sha256,
      published.sha256,
    ].every(isSha256)
  ) {
    throw new Error("fixture injected failure was refused");
  }
  const hostEdge = readEdge(descriptor, body.hostagent_edge_sha256);
  const childEdge = readEdge(child, body.child_edge_sha256);
  if (
    hostEdge.body.child_role !== "hostagent" ||
    childEdge.body.child_role !== "usernet" ||
    childEdge.body.parent_role !== "hostagent" ||
    childEdge.body.parent_identity_sha256 !== body.hostagent_identity_sha256 ||
    childEdge.body.prior_edge_sha256 !== body.hostagent_edge_sha256
  ) {
    throw new Error("fixture injected failure frontier was refused");
  }
  return { body, sha256: published.sha256 };
}

export function validateFixtureGraph({ admission, edges, identities }) {
  if (
    canonicalBytes({ admission, edges, identities }).length > MAX_GRAPH_BYTES ||
    !Array.isArray(edges) ||
    edges.length !== 3 ||
    !Array.isArray(identities) ||
    identities.length !== 4
  ) {
    throw new Error("fixture graph exceeded its closed bounds");
  }
  const edgeBodyKeys = [
    "argv_sha256",
    "child_bootstrap_sha256",
    "child_launch_nonce_sha256",
    "child_role",
    "child_role_key_sha256",
    "cwd_identity_sha256",
    "depth",
    "edge_key_sha256",
    "environment_sha256",
    "evidence_class",
    "fence_key_sha256",
    "operation_id",
    "parent_identity_sha256",
    "parent_role",
    "prior_edge_sha256",
    "schema",
    "sequence",
    "toolchain",
  ];
  for (const [index, edge] of [admission, ...edges].entries()) {
    exactKeys(edge, ["body", "sha256"], "fixture graph edge");
    exactKeys(edge.body, edgeBodyKeys, "fixture graph edge body");
    exactKeys(
      edge.body.toolchain,
      ["node_sha256", "protocol_sha256", "role_sha256"],
      "fixture graph edge toolchain",
    );
    if (
      !isSha256(edge.sha256) ||
      edge.body.evidence_class !== EVIDENCE_CLASS ||
      !isSha256(edge.body.operation_id) ||
      edge.body.schema !== (index === 0 ? SCHEMAS.admission : SCHEMAS.edge) ||
      !Object.values(edge.body.toolchain).every(isSha256) ||
      ![
        edge.body.argv_sha256,
        edge.body.child_bootstrap_sha256,
        edge.body.child_launch_nonce_sha256,
        edge.body.child_role_key_sha256,
        edge.body.cwd_identity_sha256,
        edge.body.edge_key_sha256,
        edge.body.environment_sha256,
        edge.body.fence_key_sha256,
        edge.body.parent_identity_sha256,
      ].every(isSha256) ||
      (edge.body.prior_edge_sha256 !== null &&
        !isSha256(edge.body.prior_edge_sha256))
    ) {
      throw new Error("fixture graph edge was refused");
    }
  }
  const identitiesByRole = new Map();
  const pids = new Set();
  for (const identity of identities) {
    exactKeys(
      identity,
      [
        "argv_sha256",
        "cwd_identity_sha256",
        "depth",
        "endpoint_identity",
        "environment_sha256",
        "evidence_class",
        "identity_scope",
        "identity_sha256",
        "launch_edge_sha256",
        "launch_nonce_sha256",
        "operation_id",
        "pid",
        "ppid_at_start",
        "process_nonce_sha256",
        "role",
        "schema",
        "toolchain",
        "uid",
      ],
      "fixture graph identity",
    );
    exactKeys(
      identity.endpoint_identity,
      ["dev", "ino", "mode", "nlink", "uid"],
      "fixture graph endpoint identity",
    );
    exactKeys(
      identity.toolchain,
      ["node_sha256", "protocol_sha256", "role_sha256"],
      "fixture graph identity toolchain",
    );
    if (
      !ROLES.includes(identity.role) ||
      identitiesByRole.has(identity.role) ||
      pids.has(identity.pid) ||
      identity.depth !== DEPTH[identity.role] ||
      identity.schema !== SCHEMAS.identity ||
      identity.evidence_class !== EVIDENCE_CLASS ||
      identity.identity_scope !== "trusted-fixture-nonce-not-os-start-identity" ||
      ![
        identity.argv_sha256,
        identity.cwd_identity_sha256,
        identity.environment_sha256,
        identity.identity_sha256,
        identity.launch_edge_sha256,
        identity.launch_nonce_sha256,
        identity.operation_id,
        identity.process_nonce_sha256,
      ].every(isSha256) ||
      !Object.values(identity.toolchain).every(isSha256) ||
      !Number.isSafeInteger(identity.pid) ||
      identity.pid < 2 ||
      !Number.isSafeInteger(identity.ppid_at_start) ||
      identity.ppid_at_start < 1 ||
      !Number.isSafeInteger(identity.uid) ||
      identity.endpoint_identity.mode !== 0o600 ||
      identity.endpoint_identity.nlink !== 1 ||
      !Number.isSafeInteger(identity.endpoint_identity.uid) ||
      !/^[0-9]+$/u.test(identity.endpoint_identity.dev) ||
      !/^[0-9]+$/u.test(identity.endpoint_identity.ino)
    ) {
      throw new Error("fixture graph identity was refused");
    }
    identitiesByRole.set(identity.role, identity);
    pids.add(identity.pid);
  }
  if (ROLES.some((role) => !identitiesByRole.has(role))) {
    throw new Error("fixture graph omitted a role");
  }
  if (
    admission.body.schema !== SCHEMAS.admission ||
    admission.body.child_role !== "outer" ||
    admission.body.parent_role !== "fixture-supervisor" ||
    admission.body.sequence !== 0 ||
    admission.body.depth !== 0 ||
    admission.body.prior_edge_sha256 !== null ||
    identities.some(
      (identity) => identity.operation_id !== admission.body.operation_id,
    ) ||
    identitiesByRole.get("outer").launch_edge_sha256 !== admission.sha256
  ) {
    throw new Error("fixture admission edge was refused");
  }
  const orderedEdges = edges;
  for (const [index, [parentRole, childRole]] of ROLE_EDGES.entries()) {
    const edge = orderedEdges[index];
    if (
      edge.body.schema !== SCHEMAS.edge ||
      edge.body.sequence !== index + 1 ||
      edge.body.parent_role !== parentRole ||
      edge.body.child_role !== childRole ||
      edge.body.parent_identity_sha256 !==
        identitiesByRole.get(parentRole).identity_sha256 ||
      edge.body.depth !== DEPTH[childRole] ||
      identitiesByRole.get(childRole).launch_edge_sha256 !== edge.sha256 ||
      edge.body.prior_edge_sha256 !==
        (index === 0 ? admission.sha256 : orderedEdges[index - 1].sha256) ||
      edge.body.operation_id !== admission.body.operation_id ||
      identitiesByRole.get(childRole).operation_id !== admission.body.operation_id ||
      canonical(edge.body.toolchain) !==
        canonical(identitiesByRole.get(childRole).toolchain)
    ) {
      throw new Error("fixture provider edge was refused");
    }
  }
  const causalGraphSha256 = sha256Value({
    admission_sha256: admission.sha256,
    edge_sha256s: orderedEdges.map((edge) => edge.sha256),
    identities: ROLES.map((role) => ({
      identity_sha256: identitiesByRole.get(role).identity_sha256,
      role,
    })),
  });
  return {
    causal_graph_sha256: causalGraphSha256,
    edge_count: 3,
    launch_edge_head_sha256: orderedEdges[2].sha256,
    maximum_depth: 2,
    node_count: 4,
  };
}

function requestFor(
  descriptor,
  identitySha256,
  action,
  fenceSha256,
  challenge,
  deadlineEpochMilliseconds = Date.now() + PHASE_TIMEOUT_MILLISECONDS,
) {
  const body = {
    action,
    challenge,
    deadline_epoch_milliseconds: deadlineEpochMilliseconds,
    expected_identity_sha256: identitySha256,
    fence_sha256: fenceSha256,
    operation_id: descriptor.public.operation_id,
    role: descriptor.public.role,
    schema: SCHEMAS.request,
  };
  return signedValue(descriptor.role_key, "request", body);
}

function validateEndpointResponseDetail(body, descriptor) {
  if (body.action === "probe") {
    exactKeys(
      body.detail,
      ["child_identities", "identity_scope"],
      "fixture probe response detail",
    );
    if (
      body.detail.identity_scope !==
        "trusted-fixture-nonce-not-os-start-identity" ||
      !Array.isArray(body.detail.child_identities) ||
      body.detail.child_identities.length !== descriptor.children.length
    ) {
      throw new Error("fixture probe response detail was refused");
    }
    for (const [index, child] of body.detail.child_identities.entries()) {
      exactKeys(
        child,
        ["identity_sha256", "role"],
        "fixture probe child identity",
      );
      if (
        child.role !== descriptor.children[index].public.role ||
        !isSha256(child.identity_sha256)
      ) {
        throw new Error("fixture probe child identity was refused");
      }
    }
    return;
  }
  if (body.action === "quiesce") {
    exactKeys(body.detail, ["ack_sha256"], "fixture quiescence response detail");
    if (!isSha256(body.detail.ack_sha256)) {
      throw new Error("fixture quiescence response detail was refused");
    }
    return;
  }
  if (body.action === "detach") {
    exactKeys(
      body.detail,
      ["detach_ack_sha256", "exit_ack_sha256"],
      "fixture detach response detail",
    );
    if (
      !isSha256(body.detail.detach_ack_sha256) ||
      !isSha256(body.detail.exit_ack_sha256)
    ) {
      throw new Error("fixture detach response detail was refused");
    }
    return;
  }
  if (body.action === "shutdown") {
    exactKeys(body.detail, ["exit_ack_sha256"], "fixture shutdown response detail");
    if (!isSha256(body.detail.exit_ack_sha256)) {
      throw new Error("fixture shutdown response detail was refused");
    }
    return;
  }
  throw new Error("fixture endpoint response action was refused");
}

function verifyResponse(
  response,
  descriptor,
  identity,
  action,
  challenge,
  fenceSha256,
  deadlineEpochMilliseconds,
) {
  const body = verifySignedValue(
    response,
    descriptor.role_key,
    "response",
    "fixture endpoint response",
  );
  exactKeys(
    body,
    [
      "action",
      "causal_head_sha256",
      "challenge_sha256",
      "current_ppid",
      "deadline_epoch_milliseconds",
      "detail",
      "edge_sha256",
      "endpoint_identity",
      "evidence_class",
      "fence_sha256",
      "identity_sha256",
      "operation_id",
      "phase",
      "role",
      "schema",
    ],
    "fixture endpoint response body",
  );
  if (
    body.schema !== SCHEMAS.response ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.role !== descriptor.public.role ||
    body.action !== action ||
    body.challenge_sha256 !== sha256Bytes(Buffer.from(challenge, "ascii")) ||
    body.deadline_epoch_milliseconds !== deadlineEpochMilliseconds ||
    body.identity_sha256 !== identity.identity_sha256 ||
    body.edge_sha256 !== identity.launch_edge_sha256 ||
    body.fence_sha256 !== fenceSha256 ||
    !isSha256(body.causal_head_sha256) ||
    !Number.isSafeInteger(body.current_ppid) ||
    body.current_ppid < 1 ||
    !PHASES.includes(body.phase) ||
    !sameSocketIdentity(body.endpoint_identity, identity.endpoint_identity)
  ) {
    throw new Error("fixture endpoint response did not bind the role");
  }
  const expectedPhase = {
    detach: "completing",
    quiesce: "quiesced",
    shutdown: "stopping",
  }[action];
  if (
    (action === "probe" &&
      !new Set(["ready", "quiesced", "detached-quiesced"]).has(body.phase)) ||
    (action !== "probe" && body.phase !== expectedPhase)
  ) {
    throw new Error("fixture endpoint response phase was refused");
  }
  validateEndpointResponseDetail(body, descriptor);
  return body;
}

async function roleRequest(
  descriptor,
  identity,
  action,
  fenceSha256 = null,
  challenge = randomSha256(),
) {
  const deadlineEpochMilliseconds =
    Date.now() + PHASE_TIMEOUT_MILLISECONDS;
  const before = socketIdentity(descriptor.public.socket_path);
  if (!sameSocketIdentity(before, identity.endpoint_identity)) {
    throw new Error("fixture endpoint changed before request");
  }
  const request = requestFor(
    descriptor,
    identity.identity_sha256,
    action,
    fenceSha256,
    challenge,
    deadlineEpochMilliseconds,
  );
  const response = await requestFrame(
    descriptor.public.socket_path,
    request,
    remainingDeadlineMilliseconds(deadlineEpochMilliseconds),
  );
  const body = verifyResponse(
    response,
    descriptor,
    identity,
    action,
    challenge,
    fenceSha256,
    deadlineEpochMilliseconds,
  );
  const endpointMayClose = new Set(["detach", "shutdown"]).has(action);
  try {
    const after = socketIdentity(descriptor.public.socket_path);
    if (!sameSocketIdentity(before, after)) {
      throw new Error("fixture endpoint changed across request");
    }
  } catch (error) {
    if (!(endpointMayClose && error?.code === "ENOENT")) throw error;
  }
  return body;
}

async function expectEndpointRefusal(path, request) {
  try {
    await requestFrame(path, request);
  } catch {
    return;
  }
  throw new Error("fixture endpoint accepted a refused request");
}

async function stableProbeSample(descriptors, identities, roles, fenceSha256) {
  const sample = [];
  for (const role of roles) {
    const response = await roleRequest(
      descriptors.get(role),
      identities.get(role),
      "probe",
      fenceSha256,
    );
    sample.push({
      causal_head_sha256: response.causal_head_sha256,
      current_ppid: response.current_ppid,
      edge_sha256: response.edge_sha256,
      endpoint_identity: response.endpoint_identity,
      fence_sha256: response.fence_sha256,
      identity_sha256: response.identity_sha256,
      phase: response.phase,
      role: response.role,
    });
  }
  return sample;
}

function publishFence(tree, graph, identities) {
  const body = {
    causal_head_sha256: graph.causal_graph_sha256,
    endpoint_identities: ROLES.map((role) => ({
      endpoint_identity: identities.get(role).endpoint_identity,
      role,
    })),
    evidence_class: EVIDENCE_CLASS,
    nonce_sha256: sha256Bytes(Buffer.from(randomSha256(), "ascii")),
    operation_id: tree.operationId,
    role_identities: ROLES.map((role) => ({
      identity_sha256: identities.get(role).identity_sha256,
      role,
    })),
    schema: SCHEMAS.fence,
  };
  return publishExclusive(
    tree.outer.public.quiescence_fence_path,
    signedValue(tree.fenceKey, "quiescence-fence", body),
  );
}

function collectQuiescenceAcks(
  descriptor,
  ackSha256,
  fenceSha256,
  causalHeadSha256,
  identities,
  values,
) {
  if (values.has(descriptor.public.role)) {
    throw new Error("fixture quiescence acknowledgement repeated a role");
  }
  const record = readExact(
    descriptor.public.quiescence_ack_path,
    ackSha256,
    "fixture quiescence acknowledgement",
  );
  const body = verifySignedValue(
    record,
    descriptor.role_key,
    "quiescence-ack",
    "fixture quiescence acknowledgement",
  );
  exactKeys(
    body,
    [
      "causal_head_sha256",
      "child_ack_sha256s",
      "evidence_class",
      "fence_sha256",
      "identity_sha256",
      "operation_id",
      "phase",
      "role",
      "schema",
    ],
    "fixture quiescence acknowledgement body",
  );
  if (
    body.schema !== SCHEMAS.ack ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.role !== descriptor.public.role ||
    body.identity_sha256 !==
      identities.get(descriptor.public.role).identity_sha256 ||
    body.fence_sha256 !== fenceSha256 ||
    body.causal_head_sha256 !== causalHeadSha256 ||
    body.phase !== "quiesced" ||
    !Array.isArray(body.child_ack_sha256s) ||
    body.child_ack_sha256s.length !== descriptor.children.length
  ) {
    throw new Error("fixture quiescence acknowledgement was refused");
  }
  values.set(descriptor.public.role, { body, sha256: ackSha256 });
  const childRoles = new Set();
  for (const childAck of body.child_ack_sha256s) {
    exactKeys(
      childAck,
      ["role", "sha256"],
      "fixture child quiescence acknowledgement",
    );
    const child = descriptor.children.find(
      (candidate) => candidate.public.role === childAck?.role,
    );
    if (
      child === undefined ||
      childRoles.has(childAck.role) ||
      !isSha256(childAck.sha256)
    ) {
      throw new Error("fixture child quiescence acknowledgement was refused");
    }
    childRoles.add(childAck.role);
    collectQuiescenceAcks(
      child,
      childAck.sha256,
      fenceSha256,
      causalHeadSha256,
      identities,
      values,
    );
  }
}

function collectExitAcks(
  descriptor,
  exitAckSha256,
  fenceSha256,
  identities,
  values,
) {
  if (values.has(descriptor.public.role)) {
    throw new Error("fixture exit acknowledgement repeated a role");
  }
  const record = readExact(
    descriptor.public.exit_ack_path,
    exitAckSha256,
    "fixture exit acknowledgement",
  );
  const body = verifySignedValue(
    record,
    descriptor.role_key,
    "exit-ack",
    "fixture exit acknowledgement",
  );
  exactKeys(
    body,
    [
      "action",
      "child_exit_ack_sha256s",
      "detach_ack_sha256",
      "evidence_class",
      "fence_sha256",
      "identity_sha256",
      "operation_id",
      "role",
      "schema",
    ],
    "fixture exit acknowledgement body",
  );
  if (
    body.schema !== SCHEMAS.exit ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.role !== descriptor.public.role ||
    body.identity_sha256 !==
      identities.get(descriptor.public.role).identity_sha256 ||
    body.fence_sha256 !== fenceSha256 ||
    body.action !== (descriptor.public.role === "outer" ? "detach-complete" : "shutdown") ||
    !Array.isArray(body.child_exit_ack_sha256s) ||
    body.child_exit_ack_sha256s.length !==
      (descriptor.public.role === "outer" ? 0 : descriptor.children.length) ||
    (descriptor.public.role === "outer") !== isSha256(body.detach_ack_sha256)
  ) {
    throw new Error("fixture exit acknowledgement was refused");
  }
  values.set(descriptor.public.role, { body, sha256: exitAckSha256 });
  const childRoles = new Set();
  for (const childAck of body.child_exit_ack_sha256s) {
    exactKeys(
      childAck,
      ["role", "sha256"],
      "fixture child exit acknowledgement",
    );
    const child = descriptor.children.find(
      (candidate) => candidate.public.role === childAck?.role,
    );
    if (
      child === undefined ||
      childRoles.has(childAck.role) ||
      !isSha256(childAck.sha256)
    ) {
      throw new Error("fixture child exit acknowledgement was refused");
    }
    childRoles.add(childAck.role);
    collectExitAcks(
      child,
      childAck.sha256,
      fenceSha256,
      identities,
      values,
    );
  }
}

async function waitForOuterExit(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    if (child.exitCode !== 0 || child.signalCode !== null) {
      throw new Error("fixture outer did not exit cleanly");
    }
    return;
  }
  await new Promise((resolvePromise, rejectPromise) => {
    const timer = setTimeout(
      () => rejectPromise(new Error("fixture outer exit timed out")),
      PHASE_TIMEOUT_MILLISECONDS,
    );
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      if (code === 0 && signal === null) resolvePromise();
      else rejectPromise(new Error("fixture outer did not exit cleanly"));
    });
  });
}

function pidExists(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    throw new Error("fixture PID observation was indeterminate");
  }
}

async function waitForKnownPidsAbsent(pids) {
  const deadline = Date.now() + PHASE_TIMEOUT_MILLISECONDS;
  while (Date.now() < deadline) {
    if (pids.every((pid) => !pidExists(pid))) return;
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  throw new Error("fixture known role PIDs remained after cooperative exit");
}

async function waitForReparent(descriptor, identity, outerPid, fenceSha256) {
  const deadline = Date.now() + PHASE_TIMEOUT_MILLISECONDS;
  while (Date.now() < deadline) {
    const response = await roleRequest(
      descriptor,
      identity,
      "probe",
      fenceSha256,
    );
    if (
      response.current_ppid !== outerPid &&
      response.current_ppid !== identity.pid
    ) {
      return response;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  throw new Error("fixture hostagent was not observed after reparenting");
}

function allSecretValues(rootDescriptor, fenceKey) {
  return [
    fenceKey,
    ...flattenDescriptors(rootDescriptor).flatMap((descriptor) => [
      descriptor.edge_key,
      descriptor.public.launch_nonce,
      descriptor.role_key,
    ]),
  ];
}

function regularFiles(root) {
  const values = [];
  const visit = (path) => {
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      const childPath = join(path, entry.name);
      if (entry.isDirectory()) visit(childPath);
      else if (entry.isFile()) values.push(childPath);
    }
  };
  visit(root);
  return values;
}

function assertSecretsAbsent(root, secrets, stderr, spawnArgs) {
  const haystacks = [
    stderr,
    canonical(spawnArgs),
    ...regularFiles(root).map((path) => readFileSync(path, "utf8")),
  ];
  for (const secret of secrets) {
    if (haystacks.some((value) => value.includes(secret))) {
      throw new Error("fixture secret escaped its inherited IPC channel");
    }
  }
}

async function waitForAnyOuterExit(outer, timeoutMilliseconds) {
  if (outer.exitCode !== null || outer.signalCode !== null) return true;
  return await new Promise((resolvePromise) => {
    const timer = setTimeout(() => {
      outer.off("exit", onExit);
      resolvePromise(false);
    }, timeoutMilliseconds);
    const onExit = () => {
      clearTimeout(timer);
      resolvePromise(true);
    };
    outer.once("exit", onExit);
  });
}

async function emergencyStop(pids, outer, descriptors) {
  if (
    outer !== undefined &&
    outer.exitCode === null &&
    outer.signalCode === null
  ) {
    // Only the exact direct ChildProcess handle is signalled. Each role owns
    // and verifies the exit of its directly spawned handles recursively.
    outer.kill("SIGTERM");
    if (!(await waitForAnyOuterExit(outer, 8_000))) {
      outer.kill("SIGKILL");
      if (!(await waitForAnyOuterExit(outer, 2_000))) {
        throw new Error("fixture direct outer handle could not be stopped");
      }
    }
  }
  if (
    outer !== undefined &&
    (outer.signalCode !== null ||
      !new Set([0, 70, INJECTED_PARTIAL_EXIT_CODE]).has(outer.exitCode))
  ) {
    throw new Error("fixture outer cleanup status was indeterminate");
  }
  const uniquePids = [...new Set(pids)].filter(
    (pid) => Number.isSafeInteger(pid) && pid > 1,
  );
  const deadline = Date.now() + ROLE_LIFETIME_MILLISECONDS + 2_000;
  let stableAbsenceSamples = 0;
  while (Date.now() < deadline) {
    const endpointsAbsent = [...descriptors.values()].every(
      (descriptor) => !existsSync(descriptor.public.socket_path),
    );
    const pidsAbsent = uniquePids.every((pid) => !pidExists(pid));
    if (endpointsAbsent && pidsAbsent) {
      stableAbsenceSamples += 1;
      if (stableAbsenceSamples === 2) return;
    } else {
      stableAbsenceSamples = 0;
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
  }
  throw new Error("fixture emergency cleanup remained indeterminate");
}

function unlinkTrustedFixtureFile(path) {
  if (!existsSync(path)) return;
  const parent = dirname(path);
  const parentIdentity = privateDirectoryIdentity(parent);
  const identity = lstatSync(path, { bigint: true });
  if (
    !identity.isFile() ||
    identity.isSymbolicLink() ||
    identity.uid !== BigInt(process.getuid()) ||
    identity.nlink !== 1n ||
    (identity.mode & 0o7777n) !== 0o600n
  ) {
    throw new Error("fixture cleanup file identity was refused");
  }
  unlinkSync(path);
  syncDirectory(parent);
  if (
    canonical(privateDirectoryIdentity(parent)) !== canonical(parentIdentity)
  ) {
    throw new Error("fixture cleanup parent changed");
  }
}

function removeFixtureRoot(root, rootIdentity, tree) {
  const descriptors = flattenDescriptors(tree.outer);
  if (
    descriptors.some((descriptor) =>
      existsSync(descriptor.public.socket_path),
    )
  ) {
    throw new Error("fixture endpoint remained before root cleanup");
  }
  const files = new Set([
    tree.outer.public.detach_ack_path,
    tree.outer.public.quiescence_fence_path,
  ]);
  for (const descriptor of descriptors) {
    files.add(descriptor.public.edge_path);
    files.add(descriptor.public.exit_ack_path);
    files.add(descriptor.public.failure_path);
    files.add(descriptor.public.identity_path);
    files.add(descriptor.public.quiescence_ack_path);
  }
  for (const path of files) unlinkTrustedFixtureFile(path);
  for (const name of ["exit", "quiescence", "identity", "edges"]) {
    const path = join(root, name);
    privateDirectoryIdentity(path);
    if (readdirSync(path).length !== 0) {
      throw new Error("fixture cleanup found an unknown artifact");
    }
    rmdirSync(path);
    syncDirectory(root);
  }
  if (
    readdirSync(root).length !== 0 ||
    canonical(assertPrivateRoot(root)) !== canonical(rootIdentity)
  ) {
    throw new Error("fixture root changed before cleanup");
  }
  rmdirSync(root);
  syncDirectory(dirname(root));
}

export async function runFourRoleFixture({
  exerciseRefusals = true,
  failureAt = null,
  startupDelayMilliseconds = 0,
} = {}) {
  if (!new Set(["darwin", "linux"]).has(process.platform)) {
    throw new Error("four-role fixture supports only Darwin and Linux");
  }
  if (
    !new Set([null, "hostagent-after-first-child-ready"]).has(failureAt) ||
    !Number.isSafeInteger(startupDelayMilliseconds) ||
    startupDelayMilliseconds < 0 ||
    startupDelayMilliseconds > 250
  ) {
    throw new Error("four-role fixture scenario was refused");
  }
  const root = makePrivateRoot();
  const rootIdentity = assertPrivateRoot(root);
  const tree = createFixtureTree(root, {
    failureAt,
    startupDelayMilliseconds,
  });
  const descriptors = descriptorMap(tree.outer);
  const supervisorIdentitySha256 = randomSha256();
  const knownPids = [];
  let identities;
  let fenceSha256;
  let outer;
  let completed = false;
  let refusalChecks = 0;
  try {
    const admissionSha256 = publishAdmission(
      tree.outer,
      supervisorIdentitySha256,
    );
    const admission = readEdge(tree.outer, admissionSha256);
    outer = spawn(
      process.execPath,
      [ROLE_FILE, "outer", tree.outer.public.edge_path],
      {
        cwd: root,
        detached: false,
        env: closedFixtureEnvironment(),
        shell: false,
        stdio: ["ignore", "ignore", "pipe", "ipc"],
      },
    );
    const ready = await waitForReady(outer, tree.outer, admissionSha256);
    if (ready.stderr !== "") {
      throw new Error("fixture role wrote an unexpected diagnostic");
    }
    const readyEntries = new Map();
    for (const entry of ready.body.identities) {
      exactKeys(
        entry,
        ["identity_sha256", "record_sha256", "role"],
        "fixture readiness identity",
      );
      if (
        !ROLES.includes(entry.role) ||
        readyEntries.has(entry.role) ||
        !isSha256(entry.identity_sha256) ||
        !isSha256(entry.record_sha256)
      ) {
        throw new Error("fixture readiness identity was refused");
      }
      readyEntries.set(entry.role, entry);
    }
    identities = new Map();
    const edges = [];
    for (const role of ROLES) {
      const descriptor = descriptors.get(role);
      const readyEntry = readyEntries.get(role);
      if (descriptor === undefined || readyEntry === undefined) {
        throw new Error("fixture readiness omitted a role");
      }
      const identity = readIdentity(descriptor, readyEntry.record_sha256);
      if (identity.identity_sha256 !== readyEntry.identity_sha256) {
        throw new Error("fixture readiness changed an identity");
      }
      identities.set(role, identity);
      knownPids.push(identity.pid);
      if (role !== "outer") {
        edges.push(readEdge(descriptor, identity.launch_edge_sha256));
      }
    }
    if (
      identities.get("outer").pid !== outer.pid ||
      identities.get("hostagent").ppid_at_start !== outer.pid ||
      identities.get("usernet").ppid_at_start !== identities.get("hostagent").pid ||
      identities.get("ssh-controlmaster").ppid_at_start !==
        identities.get("hostagent").pid
    ) {
      throw new Error("fixture causal process ancestry was refused");
    }
    const graph = validateFixtureGraph({ admission, edges, identities: [...identities.values()] });
    if (ready.body.causal_head_sha256 !== graph.launch_edge_head_sha256) {
      throw new Error("fixture readiness did not bind the causal head");
    }

    const preFenceSample = await stableProbeSample(
      descriptors,
      identities,
      ROLES,
      null,
    );
    if (exerciseRefusals) {
      const hostDescriptor = descriptors.get("hostagent");
      const hostIdentity = identities.get("hostagent");
      const wrongKeyRequest = signedValue(randomSha256(), "request", {
        action: "probe",
        challenge: randomSha256(),
        deadline_epoch_milliseconds:
          Date.now() + PHASE_TIMEOUT_MILLISECONDS,
        expected_identity_sha256: hostIdentity.identity_sha256,
        fence_sha256: null,
        operation_id: tree.operationId,
        role: "hostagent",
        schema: SCHEMAS.request,
      });
      await expectEndpointRefusal(
        hostDescriptor.public.socket_path,
        wrongKeyRequest,
      );
      refusalChecks += 1;
      const replayChallenge = randomSha256();
      const replayRequest = requestFor(
        hostDescriptor,
        hostIdentity.identity_sha256,
        "probe",
        null,
        replayChallenge,
      );
      const replayResponse = await requestFrame(
        hostDescriptor.public.socket_path,
        replayRequest,
      );
      verifyResponse(
        replayResponse,
        hostDescriptor,
        hostIdentity,
        "probe",
        replayChallenge,
        null,
        replayRequest.deadline_epoch_milliseconds,
      );
      await expectEndpointRefusal(
        hostDescriptor.public.socket_path,
        replayRequest,
      );
      refusalChecks += 1;
      await expectEndpointRefusal(
        hostDescriptor.public.socket_path,
        signedValue(hostDescriptor.role_key, "request", {
          action: "probe",
          challenge: randomSha256(),
          deadline_epoch_milliseconds:
            Date.now() + PHASE_TIMEOUT_MILLISECONDS,
          expected_identity_sha256: hostIdentity.identity_sha256,
          fence_sha256: null,
          operation_id: tree.operationId,
          role: "usernet",
          schema: SCHEMAS.request,
        }),
      );
      refusalChecks += 1;
      await expectEndpointRefusal(
        hostDescriptor.public.socket_path,
        signedValue(hostDescriptor.role_key, "request", {
          action: "probe",
          challenge: randomSha256(),
          deadline_epoch_milliseconds:
            Date.now() + PHASE_TIMEOUT_MILLISECONDS,
          expected_identity_sha256: randomSha256(),
          fence_sha256: null,
          operation_id: tree.operationId,
          role: "hostagent",
          schema: SCHEMAS.request,
        }),
      );
      refusalChecks += 1;
    }

    fenceSha256 = publishFence(tree, graph, identities);
    if (exerciseRefusals) {
      const outerDescriptor = descriptors.get("outer");
      const outerIdentity = identities.get("outer");
      const wrongFenceChallenge = randomSha256();
      await expectEndpointRefusal(
        outerDescriptor.public.socket_path,
        requestFor(
          outerDescriptor,
          outerIdentity.identity_sha256,
          "quiesce",
          randomSha256(),
          wrongFenceChallenge,
        ),
      );
      refusalChecks += 1;
      const stillReady = await roleRequest(
        outerDescriptor,
        outerIdentity,
        "probe",
      );
      if (stillReady.phase !== "ready") {
        throw new Error("fixture refusal changed the outer phase");
      }
    }
    const quiescence = await roleRequest(
      descriptors.get("outer"),
      identities.get("outer"),
      "quiesce",
      fenceSha256,
    );
    if (quiescence.phase !== "quiesced" || !isSha256(quiescence.detail?.ack_sha256)) {
      throw new Error("fixture quiescence was not acknowledged");
    }
    const quiescenceAcks = new Map();
    collectQuiescenceAcks(
      tree.outer,
      quiescence.detail.ack_sha256,
      fenceSha256,
      graph.causal_graph_sha256,
      identities,
      quiescenceAcks,
    );
    if (quiescenceAcks.size !== 4) {
      throw new Error("fixture quiescence omitted a role");
    }
    const quiescedSampleOne = await stableProbeSample(
      descriptors,
      identities,
      ROLES,
      fenceSha256,
    );
    const quiescedSampleTwo = await stableProbeSample(
      descriptors,
      identities,
      ROLES,
      fenceSha256,
    );
    if (canonical(quiescedSampleOne) !== canonical(quiescedSampleTwo)) {
      throw new Error("fixture quiesced role sample changed");
    }
    if (exerciseRefusals) {
      const outerDescriptor = descriptors.get("outer");
      const outerIdentity = identities.get("outer");
      await expectEndpointRefusal(
        outerDescriptor.public.socket_path,
        requestFor(
          outerDescriptor,
          outerIdentity.identity_sha256,
          "spawn",
          fenceSha256,
          randomSha256(),
        ),
      );
      refusalChecks += 1;
      const stillQuiesced = await roleRequest(
        outerDescriptor,
        outerIdentity,
        "probe",
        fenceSha256,
      );
      if (stillQuiesced.phase !== "quiesced") {
        throw new Error("fixture post-fence spawn refusal changed phase");
      }
    }

    const detach = await roleRequest(
      descriptors.get("outer"),
      identities.get("outer"),
      "detach",
      fenceSha256,
    );
    if (
      detach.phase !== "completing" ||
      !isSha256(detach.detail?.detach_ack_sha256) ||
      !isSha256(detach.detail?.exit_ack_sha256)
    ) {
      throw new Error("fixture outer detach was not acknowledged");
    }
    await waitForOuterExit(outer);
    if (existsSync(descriptors.get("outer").public.socket_path)) {
      throw new Error("fixture outer endpoint remained after completion");
    }
    const reparent = await waitForReparent(
      descriptors.get("hostagent"),
      identities.get("hostagent"),
      identities.get("outer").pid,
      fenceSha256,
    );
    if (
      reparent.identity_sha256 !== identities.get("hostagent").identity_sha256 ||
      reparent.current_ppid === identities.get("hostagent").ppid_at_start
    ) {
      throw new Error("fixture reparent observation changed process identity");
    }
    const survivingRoles = ["hostagent", "usernet", "ssh-controlmaster"];
    const detachedSampleOne = await stableProbeSample(
      descriptors,
      identities,
      survivingRoles,
      fenceSha256,
    );
    const detachedSampleTwo = await stableProbeSample(
      descriptors,
      identities,
      survivingRoles,
      fenceSha256,
    );
    if (canonical(detachedSampleOne) !== canonical(detachedSampleTwo)) {
      throw new Error("fixture detached role sample changed");
    }

    const shutdown = await roleRequest(
      descriptors.get("hostagent"),
      identities.get("hostagent"),
      "shutdown",
      fenceSha256,
    );
    if (shutdown.phase !== "stopping" || !isSha256(shutdown.detail?.exit_ack_sha256)) {
      throw new Error("fixture shutdown was not acknowledged");
    }
    await waitForKnownPidsAbsent(knownPids);
    for (const descriptor of descriptors.values()) {
      if (existsSync(descriptor.public.socket_path)) {
        throw new Error("fixture endpoint remained after cooperative shutdown");
      }
    }
    const exitAcks = new Map();
    collectExitAcks(
      tree.outer,
      detach.detail.exit_ack_sha256,
      fenceSha256,
      identities,
      exitAcks,
    );
    collectExitAcks(
      descriptors.get("hostagent"),
      shutdown.detail.exit_ack_sha256,
      fenceSha256,
      identities,
      exitAcks,
    );
    if (exitAcks.size !== 4) {
      throw new Error("fixture cooperative exit omitted a role");
    }
    const detachRecord = readExact(
      tree.outer.public.detach_ack_path,
      detach.detail.detach_ack_sha256,
      "fixture detach acknowledgement",
    );
    const detachBody = verifySignedValue(
      detachRecord,
      tree.outer.role_key,
      "detach-ack",
      "fixture detach acknowledgement",
    );
    exactKeys(
      detachBody,
      [
        "evidence_class",
        "fence_sha256",
        "hostagent_identity_sha256",
        "hostagent_ppid_before_outer_exit",
        "operation_id",
        "outer_identity_sha256",
        "schema",
      ],
      "fixture detach acknowledgement body",
    );
    if (
      detachBody.schema !== SCHEMAS.detach ||
      detachBody.evidence_class !== EVIDENCE_CLASS ||
      detachBody.operation_id !== tree.operationId ||
      detachBody.fence_sha256 !== fenceSha256 ||
      detachBody.outer_identity_sha256 !== identities.get("outer").identity_sha256 ||
      detachBody.hostagent_identity_sha256 !==
        identities.get("hostagent").identity_sha256 ||
      detachBody.hostagent_ppid_before_outer_exit !== identities.get("outer").pid
    ) {
      throw new Error("fixture detach acknowledgement was refused");
    }
    assertSecretsAbsent(
      root,
      allSecretValues(tree.outer, tree.fenceKey),
      ready.stderr,
      outer.spawnargs,
    );
    completed = true;
    return {
      causal_graph: graph,
      cooperative_shutdown: true,
      detached_hostagent_reparent_observed: true,
      evidence_class: EVIDENCE_CLASS,
      lifecycle_authority: false,
      live_provider_evidence: false,
      operation_id: tree.operationId,
      provider_effect_evidence: false,
      refusal_checks: refusalChecks,
      stable_samples: {
        before_fence_roles: preFenceSample.length,
        detached_roles: detachedSampleOne.length,
        quiesced_roles: quiescedSampleOne.length,
      },
      state_integration: false,
    };
  } finally {
    if (!completed) {
      await emergencyStop(knownPids, outer, descriptors);
    }
    removeFixtureRoot(root, rootIdentity, tree);
  }
}

export async function runPartialStartFailureFixture() {
  try {
    await runFourRoleFixture({
      exerciseRefusals: false,
      failureAt: "hostagent-after-first-child-ready",
    });
  } catch (error) {
    if (
      error instanceof Error &&
      error.fixtureFailure === INJECTED_PARTIAL_FAILURE &&
      isSha256(error.fixtureFailureSha256)
    ) {
      // The fresh trusted fixture tree is removed only after recursive
      // direct-handle cleanup and two stable endpoint/PID absence samples.
      return {
        cleanup_verified: true,
        evidence_class: EVIDENCE_CLASS,
        failure_point: "hostagent-after-first-child-ready",
        frontier_authenticated: true,
        success_evidence: false,
      };
    }
    throw error;
  }
  throw new Error("fixture partial-start failure was not injected");
}

if (
  process.argv[1] !== undefined &&
  pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  runFourRoleFixture()
    .then((result) => process.stdout.write(`${canonical(result)}\n`))
    .catch(() => {
      process.stderr.write("four-role fixture failed\n");
      process.exit(1);
    });
}

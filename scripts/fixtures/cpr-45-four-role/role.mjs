#!/usr/bin/env node
import { spawn } from "node:child_process";
import {
  chmodSync,
  existsSync,
  readFileSync,
  unlinkSync,
} from "node:fs";
import { createServer } from "node:net";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  EVIDENCE_CLASS,
  MAX_ACCEPTED_CHALLENGES,
  MAX_FRAME_BYTES,
  MAX_REQUEST_LIFETIME_MILLISECONDS,
  PHASES,
  ROLES,
  SCHEMAS,
  assertPathInRoot,
  assertPrivateRoot,
  canonical,
  canonicalBytes,
  bootstrapCommitment,
  closedFixtureEnvironment,
  exactKeys,
  isSha256,
  publishExclusive,
  randomSha256,
  readExact,
  readPublished,
  remainingDeadlineMilliseconds,
  sameSocketIdentity,
  sha256Bytes,
  sha256Value,
  signedValue,
  socketIdentity,
  syncDirectory,
  verifySignedValue,
} from "./protocol.mjs";

const ROLE_FILE = fileURLToPath(import.meta.url);
const PROTOCOL_FILE = fileURLToPath(new URL("./protocol.mjs", import.meta.url));
const ROLE = process.argv[2];
const EDGE_PATH = process.argv[3];
const BOOTSTRAP_TIMEOUT_MILLISECONDS = 3_000;
const LEAF_PHASE_TIMEOUT_MILLISECONDS = 3_000;
const SUBTREE_PHASE_TIMEOUT_MILLISECONDS = 15_000;
const ENDPOINT_TIMEOUT_MILLISECONDS =
  MAX_REQUEST_LIFETIME_MILLISECONDS + 1_000;
const ROLE_LIFETIME_MILLISECONDS = 45_000;
const MAX_ACTIVE_SOCKETS = 8;
const INJECTED_PARTIAL_FAILURE = "hostagent-after-first-child-ready";
const INJECTED_PARTIAL_EXIT_CODE = 72;

const EXPECTED_CHILDREN = Object.freeze({
  outer: Object.freeze(["hostagent"]),
  hostagent: Object.freeze(["usernet", "ssh-controlmaster"]),
  usernet: Object.freeze([]),
  "ssh-controlmaster": Object.freeze([]),
});
const EXPECTED_PARENT = Object.freeze({
  outer: "fixture-supervisor",
  hostagent: "outer",
  usernet: "hostagent",
  "ssh-controlmaster": "hostagent",
});
const EXPECTED_DEPTH = Object.freeze({
  outer: 0,
  hostagent: 1,
  usernet: 2,
  "ssh-controlmaster": 2,
});
const EXPECTED_SEQUENCE = Object.freeze({
  outer: 0,
  hostagent: 1,
  usernet: 2,
  "ssh-controlmaster": 3,
});

let cachedToolchain;
let activeRuntime;

function childPhaseTimeout(role) {
  return role === "hostagent"
    ? SUBTREE_PHASE_TIMEOUT_MILLISECONDS
    : LEAF_PHASE_TIMEOUT_MILLISECONDS;
}

function remainingPhaseMilliseconds(deadlineEpochMilliseconds, maximum) {
  return Math.min(
    remainingDeadlineMilliseconds(deadlineEpochMilliseconds),
    maximum,
  );
}

function actualToolchain() {
  cachedToolchain ??= {
    node_sha256: sha256Bytes(readFileSync(process.execPath)),
    protocol_sha256: sha256Bytes(readFileSync(PROTOCOL_FILE)),
    role_sha256: sha256Bytes(readFileSync(ROLE_FILE)),
  };
  return cachedToolchain;
}

function environmentValue() {
  return Object.fromEntries(
    Object.keys(process.env)
      .sort()
      .map((key) => [key, process.env[key]]),
  );
}

function expectedArgv(role, edgePath) {
  return [process.execPath, ROLE_FILE, role, edgePath];
}

function validatePublicConfig(value) {
  exactKeys(
    value,
    [
      "child_roles",
      "depth",
      "detach_ack_path",
      "edge_path",
      "evidence_class",
      "exit_ack_path",
      "failure_path",
      "identity_path",
      "launch_nonce",
      "maximum_lifetime_milliseconds",
      "operation_id",
      "parent_role",
      "quiescence_ack_path",
      "quiescence_fence_path",
      "role",
      "schema",
      "sequence",
      "socket_path",
      "startup_delay_milliseconds",
      "toolchain",
      "workspace",
    ],
    "fixture public config",
  );
  if (
    value.schema !== SCHEMAS.config ||
    value.evidence_class !== EVIDENCE_CLASS ||
    !isSha256(value.operation_id) ||
    !ROLES.includes(value.role) ||
    value.parent_role !== EXPECTED_PARENT[value.role] ||
    value.depth !== EXPECTED_DEPTH[value.role] ||
    value.sequence !== EXPECTED_SEQUENCE[value.role] ||
    !isSha256(value.launch_nonce) ||
    canonical(value.child_roles) !== canonical(EXPECTED_CHILDREN[value.role]) ||
    !Number.isSafeInteger(value.startup_delay_milliseconds) ||
    value.startup_delay_milliseconds < 0 ||
    value.startup_delay_milliseconds > 250 ||
    value.maximum_lifetime_milliseconds !== ROLE_LIFETIME_MILLISECONDS
  ) {
    throw new Error("fixture public config was refused");
  }
  exactKeys(
    value.toolchain,
    ["node_sha256", "protocol_sha256", "role_sha256"],
    "fixture toolchain",
  );
  if (
    !Object.values(value.toolchain).every(isSha256) ||
    canonical(value.toolchain) !== canonical(actualToolchain())
  ) {
    throw new Error("fixture toolchain was refused");
  }
  assertPrivateRoot(value.workspace);
  for (const path of [
    value.edge_path,
    value.identity_path,
    value.quiescence_ack_path,
    value.exit_ack_path,
    value.failure_path,
    value.quiescence_fence_path,
  ]) {
    assertPathInRoot(path, value.workspace);
  }
  assertPathInRoot(value.socket_path, value.workspace, true);
  if (value.role === "outer") {
    assertPathInRoot(value.detach_ack_path, value.workspace);
  } else if (value.detach_ack_path !== null) {
    throw new Error("fixture detach path was refused");
  }
  return value;
}

function validateDescriptor(value, expectedRole = ROLE) {
  exactKeys(
    value,
    ["children", "edge_key", "fault", "fence_key", "public", "role_key"],
    "fixture bootstrap descriptor",
  );
  const publicConfig = validatePublicConfig(value.public);
  if (
    publicConfig.role !== expectedRole ||
    !isSha256(value.edge_key) ||
    !isSha256(value.fence_key) ||
    !isSha256(value.role_key) ||
    !new Set(["none", "after-first-child-ready"]).has(value.fault) ||
    (value.fault !== "none" && expectedRole !== "hostagent") ||
    !Array.isArray(value.children) ||
    value.children.length !== EXPECTED_CHILDREN[expectedRole].length
  ) {
    throw new Error("fixture bootstrap descriptor was refused");
  }
  value.children.forEach((child, index) => {
    if (child?.public?.role !== EXPECTED_CHILDREN[expectedRole][index]) {
      throw new Error("fixture child order was refused");
    }
  });
  return value;
}

async function waitForBootstrap() {
  if (typeof process.send !== "function") {
    throw new Error("fixture bootstrap channel was unavailable");
  }
  return await new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const timer = setTimeout(
      () => finish(new Error("fixture bootstrap timed out")),
      BOOTSTRAP_TIMEOUT_MILLISECONDS,
    );
    process.once("message", (message) => {
      try {
        exactKeys(
          message,
          ["descriptor", "edge_sha256", "schema"],
          "fixture bootstrap",
        );
        if (message.schema !== SCHEMAS.bootstrap || !isSha256(message.edge_sha256)) {
          throw new Error("fixture bootstrap schema was refused");
        }
        finish(undefined, {
          descriptor: validateDescriptor(message.descriptor),
          edgeSha256: message.edge_sha256,
        });
      } catch (error) {
        finish(error);
      }
    });
  });
}

function validateLaunchEdge(descriptor, expectedDigest) {
  const publicConfig = descriptor.public;
  const expectedSchema = ROLE === "outer" ? SCHEMAS.admission : SCHEMAS.edge;
  const record = readExact(EDGE_PATH, expectedDigest, "fixture launch edge");
  const body = verifySignedValue(
    record,
    descriptor.edge_key,
    "launch-edge",
    "fixture launch edge",
  );
  exactKeys(
    body,
    [
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
    ],
    "fixture launch-edge body",
  );
  const rootIdentitySha256 = sha256Value(assertPrivateRoot(publicConfig.workspace));
  if (
    body.schema !== expectedSchema ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== publicConfig.operation_id ||
    body.sequence !== publicConfig.sequence ||
    body.parent_role !== publicConfig.parent_role ||
    !isSha256(body.parent_identity_sha256) ||
    body.child_role !== ROLE ||
    body.child_bootstrap_sha256 !==
      sha256Value(bootstrapCommitment(descriptor)) ||
    body.child_role_key_sha256 !==
      sha256Bytes(Buffer.from(descriptor.role_key, "ascii")) ||
    body.child_launch_nonce_sha256 !==
      sha256Bytes(Buffer.from(publicConfig.launch_nonce, "ascii")) ||
    body.edge_key_sha256 !==
      sha256Bytes(Buffer.from(descriptor.edge_key, "ascii")) ||
    body.fence_key_sha256 !==
      sha256Bytes(Buffer.from(descriptor.fence_key, "ascii")) ||
    body.depth !== publicConfig.depth ||
    canonical(body.toolchain) !== canonical(publicConfig.toolchain) ||
    body.argv_sha256 !== sha256Value(expectedArgv(ROLE, EDGE_PATH)) ||
    body.environment_sha256 !== sha256Value(closedFixtureEnvironment()) ||
    body.cwd_identity_sha256 !== rootIdentitySha256 ||
    (body.prior_edge_sha256 !== null && !isSha256(body.prior_edge_sha256))
  ) {
    throw new Error("fixture launch edge did not bind the process");
  }
  return { body, sha256: expectedDigest };
}

function requestBody(
  runtime,
  action,
  fenceSha256,
  deadlineEpochMilliseconds,
  challenge = randomSha256(),
) {
  return {
    action,
    challenge,
    deadline_epoch_milliseconds: deadlineEpochMilliseconds,
    expected_identity_sha256: runtime.identitySha256,
    fence_sha256: fenceSha256,
    operation_id: runtime.publicConfig.operation_id,
    role: runtime.publicConfig.role,
    schema: SCHEMAS.ipcRequest,
  };
}

function signedRequest(
  runtime,
  action,
  fenceSha256,
  deadlineEpochMilliseconds,
  challenge,
) {
  const body = requestBody(
    runtime,
    action,
    fenceSha256,
    deadlineEpochMilliseconds,
    challenge,
  );
  return signedValue(runtime.roleKey, "request", body);
}

function validateChildResponseDetail(body) {
  if (body.action === "quiesce") {
    exactKeys(body.detail, ["ack_sha256"], "fixture quiescence response detail");
    if (!isSha256(body.detail.ack_sha256)) {
      throw new Error("fixture quiescence response detail was refused");
    }
    return;
  }
  if (body.action === "detach") {
    exactKeys(body.detail, ["detached"], "fixture detach response detail");
    if (body.detail.detached !== true) {
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
  throw new Error("fixture child response action was refused");
}

function readInjectedFailure(descriptor, expectedEdgeSha256) {
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
  if (
    descriptor.public.role !== "hostagent" ||
    descriptor.fault !== "after-first-child-ready" ||
    body.schema !== SCHEMAS.failure ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== descriptor.public.operation_id ||
    body.failure_point !== INJECTED_PARTIAL_FAILURE ||
    body.hostagent_edge_sha256 !== expectedEdgeSha256 ||
    body.child_role !== descriptor.children[0]?.public.role ||
    ![
      body.causal_head_sha256,
      body.child_edge_sha256,
      body.child_identity_record_sha256,
      body.child_identity_sha256,
      body.hostagent_identity_record_sha256,
      body.hostagent_identity_sha256,
      published.sha256,
    ].every(isSha256)
  ) {
    throw new Error("fixture injected failure was refused");
  }
  return { body, sha256: published.sha256 };
}

function validateResponse(
  response,
  child,
  action,
  challenge,
  fenceSha256,
  deadlineEpochMilliseconds,
) {
  const expectedPhase = {
    detach: "detached-quiesced",
    quiesce: "quiesced",
    shutdown: "stopping",
  }[action];
  const body = verifySignedValue(
    response,
    child.descriptor.role_key,
    "response",
    "fixture child response",
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
    "fixture child response body",
  );
  if (
    body.schema !== SCHEMAS.response ||
    body.evidence_class !== EVIDENCE_CLASS ||
    body.operation_id !== child.descriptor.public.operation_id ||
    body.role !== child.descriptor.public.role ||
    body.action !== action ||
    body.challenge_sha256 !== sha256Bytes(Buffer.from(challenge, "ascii")) ||
    body.deadline_epoch_milliseconds !== deadlineEpochMilliseconds ||
    body.identity_sha256 !== child.identitySha256 ||
    body.edge_sha256 !== child.edgeSha256 ||
    body.fence_sha256 !== fenceSha256 ||
    !isSha256(body.causal_head_sha256) ||
    !Number.isSafeInteger(body.current_ppid) ||
    body.current_ppid < 1 ||
    !PHASES.includes(body.phase) ||
    body.phase !== expectedPhase
  ) {
    throw new Error("fixture child response was refused");
  }
  validateChildResponseDetail(body);
  return body;
}

async function requestChild(
  child,
  action,
  fenceSha256,
  deadlineEpochMilliseconds,
) {
  const challenge = randomSha256();
  const request = signedRequest(
    {
      identitySha256: child.identitySha256,
      publicConfig: child.descriptor.public,
      roleKey: child.descriptor.role_key,
    },
    action,
    fenceSha256,
    deadlineEpochMilliseconds,
    challenge,
  );
  return await new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.process.off("message", onMessage);
      child.process.off("exit", onExit);
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const onMessage = (message) => {
      try {
        finish(
          undefined,
          validateResponse(
            message,
            child,
            action,
            challenge,
            fenceSha256,
            deadlineEpochMilliseconds,
          ),
        );
      } catch (error) {
        finish(error);
      }
    };
    const onExit = () => finish(new Error("fixture child exited before replying"));
    const timer = setTimeout(
      () => finish(new Error("fixture child request timed out")),
      remainingPhaseMilliseconds(
        deadlineEpochMilliseconds,
        childPhaseTimeout(child.descriptor.public.role),
      ),
    );
    child.process.once("message", onMessage);
    child.process.once("exit", onExit);
    child.process.send(request, (error) => {
      if (error !== null && error !== undefined) finish(error);
    });
  });
}

async function waitForChildExit(child, deadlineEpochMilliseconds) {
  const remaining = remainingPhaseMilliseconds(
    deadlineEpochMilliseconds,
    LEAF_PHASE_TIMEOUT_MILLISECONDS,
  );
  if (child.process.exitCode !== null || child.process.signalCode !== null) {
    if (child.process.exitCode !== 0) {
      throw new Error("fixture child did not exit cleanly");
    }
    return;
  }
  await new Promise((resolvePromise, rejectPromise) => {
    const timer = setTimeout(
      () => rejectPromise(new Error("fixture child exit timed out")),
      remaining,
    );
    child.process.once("exit", (code, signal) => {
      clearTimeout(timer);
      if (code === 0 && signal === null) resolvePromise();
      else rejectPromise(new Error("fixture child did not exit cleanly"));
    });
  });
}

async function waitForEmergencyChildExit(child, timeoutMilliseconds) {
  if (child.process.exitCode !== null || child.process.signalCode !== null) {
    return true;
  }
  return await new Promise((resolvePromise) => {
    const timer = setTimeout(() => {
      child.process.off("exit", onExit);
      resolvePromise(false);
    }, timeoutMilliseconds);
    const onExit = () => {
      clearTimeout(timer);
      resolvePromise(true);
    };
    child.process.once("exit", onExit);
  });
}

function assertTrackedChildExit(child) {
  if (
    child.process.signalCode !== null ||
    !new Set([0, 70, INJECTED_PARTIAL_EXIT_CODE]).has(child.process.exitCode)
  ) {
    throw new Error("fixture tracked child cleanup was indeterminate");
  }
}

async function stopTrackedChild(child) {
  if (child.process.exitCode !== null || child.process.signalCode !== null) {
    assertTrackedChildExit(child);
    return;
  }
  try {
    child.process.kill("SIGTERM");
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
  if (await waitForEmergencyChildExit(child, 4_000)) {
    assertTrackedChildExit(child);
    return;
  }
  try {
    child.process.kill("SIGKILL");
  } catch (error) {
    if (error?.code !== "ESRCH") throw error;
  }
  if (!(await waitForEmergencyChildExit(child, 2_000))) {
    throw new Error("fixture tracked child could not be stopped");
  }
  assertTrackedChildExit(child);
}

class RoleRuntime {
  constructor(descriptor, launchEdge) {
    this.descriptor = descriptor;
    this.publicConfig = descriptor.public;
    this.roleKey = descriptor.role_key;
    this.fenceKey = descriptor.fence_key;
    this.launchEdge = launchEdge;
    this.rootIdentity = assertPrivateRoot(this.publicConfig.workspace);
    this.edgeSha256 = launchEdge.sha256;
    this.causalHeadSha256 = launchEdge.sha256;
    this.phase = "starting";
    this.children = [];
    this.acceptedChallenges = new Set();
    this.sockets = new Set();
    this.endpoint = undefined;
    this.endpointIdentity = undefined;
    this.fenceSha256 = null;
    this.identitySha256 = undefined;
    this.subtreeIdentities = [];
    this.exiting = false;
  }

  async listen() {
    if (
      canonical(assertPrivateRoot(this.publicConfig.workspace)) !==
      canonical(this.rootIdentity)
    ) {
      throw new Error("fixture root changed before endpoint creation");
    }
    if (existsSync(this.publicConfig.socket_path)) {
      throw new Error("fixture endpoint collision was refused");
    }
    this.endpoint = createServer({ allowHalfOpen: true }, (socket) => {
      if (this.sockets.size >= MAX_ACTIVE_SOCKETS) {
        socket.destroy();
        return;
      }
      this.sockets.add(socket);
      socket.setTimeout(ENDPOINT_TIMEOUT_MILLISECONDS, () => socket.destroy());
      socket.once("close", () => this.sockets.delete(socket));
      const chunks = [];
      let size = 0;
      let overflow = false;
      socket.on("data", (chunk) => {
        if (overflow) return;
        if (size + chunk.length > MAX_FRAME_BYTES) {
          overflow = true;
          socket.destroy();
          return;
        }
        size += chunk.length;
        chunks.push(chunk);
      });
      socket.once("end", async () => {
        if (overflow) return;
        try {
          const requestBytes = Buffer.concat(chunks, size);
          if (
            requestBytes.length < 2 ||
            requestBytes[requestBytes.length - 1] !== 0x0a
          ) {
            throw new Error("fixture request framing was refused");
          }
          const request = JSON.parse(
            requestBytes.subarray(0, requestBytes.length - 1).toString("utf8"),
          );
          if (!canonicalBytes(request).equals(requestBytes)) {
            throw new Error("fixture request was not canonical");
          }
          const outcome = await this.handleRequest(request, "endpoint");
          const responseBytes = canonicalBytes(outcome.response);
          if (responseBytes.length > MAX_FRAME_BYTES) {
            throw new Error("fixture response exceeded its byte bound");
          }
          socket.end(responseBytes, () => {
            if (outcome.afterReply !== undefined) outcome.afterReply();
          });
        } catch {
          socket.destroy();
        }
      });
    });
    await new Promise((resolvePromise, rejectPromise) => {
      this.endpoint.once("error", rejectPromise);
      this.endpoint.listen(this.publicConfig.socket_path, () => {
        this.endpoint.off("error", rejectPromise);
        resolvePromise();
      });
    });
    chmodSync(this.publicConfig.socket_path, 0o600);
    syncDirectory(dirname(this.publicConfig.socket_path));
    this.endpointIdentity = socketIdentity(this.publicConfig.socket_path);
  }

  publishIdentity() {
    const identityBody = {
      argv_sha256: sha256Value(process.argv),
      cwd_identity_sha256: sha256Value(assertPrivateRoot(process.cwd())),
      depth: this.publicConfig.depth,
      endpoint_identity: this.endpointIdentity,
      environment_sha256: sha256Value(environmentValue()),
      evidence_class: EVIDENCE_CLASS,
      identity_scope: "trusted-fixture-nonce-not-os-start-identity",
      launch_edge_sha256: this.edgeSha256,
      launch_nonce_sha256: sha256Bytes(
        Buffer.from(this.publicConfig.launch_nonce, "ascii"),
      ),
      operation_id: this.publicConfig.operation_id,
      pid: process.pid,
      ppid_at_start: process.ppid,
      process_nonce_sha256: sha256Bytes(Buffer.from(randomSha256(), "ascii")),
      role: ROLE,
      schema: SCHEMAS.identity,
      toolchain: this.publicConfig.toolchain,
      uid: process.getuid(),
    };
    this.identitySha256 = sha256Value(identityBody);
    const record = signedValue(this.roleKey, "identity", {
      ...identityBody,
      identity_sha256: this.identitySha256,
    });
    this.identityRecordSha256 = publishExclusive(
      this.publicConfig.identity_path,
      record,
    );
    this.subtreeIdentities = [
      {
        identity_sha256: this.identitySha256,
        record_sha256: this.identityRecordSha256,
        role: ROLE,
      },
    ];
  }

  createChildEdge(childDescriptor, priorEdgeSha256) {
    if (this.phase !== "starting") {
      throw new Error("fixture spawn gate was closed");
    }
    const childPublic = validatePublicConfig(childDescriptor.public);
    const edgeBody = {
      argv_sha256: sha256Value(
        expectedArgv(childPublic.role, childPublic.edge_path),
      ),
      child_bootstrap_sha256: sha256Value(
        bootstrapCommitment(childDescriptor),
      ),
      child_launch_nonce_sha256: sha256Bytes(
        Buffer.from(childPublic.launch_nonce, "ascii"),
      ),
      child_role: childPublic.role,
      child_role_key_sha256: sha256Bytes(
        Buffer.from(childDescriptor.role_key, "ascii"),
      ),
      cwd_identity_sha256: sha256Value(
        assertPrivateRoot(childPublic.workspace),
      ),
      depth: childPublic.depth,
      edge_key_sha256: sha256Bytes(
        Buffer.from(childDescriptor.edge_key, "ascii"),
      ),
      environment_sha256: sha256Value(closedFixtureEnvironment()),
      evidence_class: EVIDENCE_CLASS,
      fence_key_sha256: sha256Bytes(
        Buffer.from(childDescriptor.fence_key, "ascii"),
      ),
      operation_id: childPublic.operation_id,
      parent_identity_sha256: this.identitySha256,
      parent_role: ROLE,
      prior_edge_sha256: priorEdgeSha256,
      schema: SCHEMAS.edge,
      sequence: childPublic.sequence,
      toolchain: childPublic.toolchain,
    };
    const edge = signedValue(
      childDescriptor.edge_key,
      "launch-edge",
      edgeBody,
    );
    return publishExclusive(childPublic.edge_path, edge);
  }

  async launchChild(childDescriptor) {
    const edgeSha256 = this.createChildEdge(
      childDescriptor,
      this.causalHeadSha256,
    );
    const child = spawn(
      process.execPath,
      [ROLE_FILE, childDescriptor.public.role, childDescriptor.public.edge_path],
      {
        cwd: childDescriptor.public.workspace,
        detached: ROLE === "outer" && childDescriptor.public.role === "hostagent",
        env: closedFixtureEnvironment(),
        shell: false,
        stdio: ["ignore", "ignore", "ignore", "ipc"],
      },
    );
    const childState = {
      descriptor: childDescriptor,
      edgeSha256,
      identitySha256: undefined,
      process: child,
      ready: undefined,
    };
    // The handle is retained before bootstrap or readiness so every partial
    // start remains recursively owned by its exact spawning process.
    this.children.push(childState);
    const readyPromise = new Promise((resolvePromise, rejectPromise) => {
      const timer = setTimeout(
        () => rejectPromise(new Error("fixture child readiness timed out")),
        childPhaseTimeout(childDescriptor.public.role),
      );
      const onExit = (code, signal) => {
        clearTimeout(timer);
        const error = new Error("fixture child exited before readiness");
        if (
          code === INJECTED_PARTIAL_EXIT_CODE &&
          signal === null &&
          childDescriptor.fault === "after-first-child-ready"
        ) {
          try {
            const failure = readInjectedFailure(childDescriptor, edgeSha256);
            error.fixtureFailure = failure.body.failure_point;
            error.fixtureFailureSha256 = failure.sha256;
          } catch (failureError) {
            rejectPromise(failureError);
            return;
          }
        }
        rejectPromise(error);
      };
      const onError = () => {
        clearTimeout(timer);
        rejectPromise(new Error("fixture child spawn failed"));
      };
      child.once("exit", onExit);
      child.once("error", onError);
      child.once("message", (message) => {
        clearTimeout(timer);
        child.off("exit", onExit);
        child.off("error", onError);
        try {
          const body = verifySignedValue(
            message,
            childDescriptor.role_key,
            "ready",
            "fixture child readiness",
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
            "fixture readiness body",
          );
          if (
            body.schema !== SCHEMAS.ready ||
            body.evidence_class !== EVIDENCE_CLASS ||
            body.operation_id !== childDescriptor.public.operation_id ||
            body.role !== childDescriptor.public.role ||
            body.edge_sha256 !== edgeSha256 ||
            body.phase !== "ready" ||
            !isSha256(body.identity_sha256) ||
            !isSha256(body.causal_head_sha256) ||
            !Number.isSafeInteger(body.pid) ||
            body.pid !== child.pid ||
            !Array.isArray(body.identities)
          ) {
            throw new Error("fixture child readiness was refused");
          }
          resolvePromise(body);
        } catch (error) {
          rejectPromise(error);
        }
      });
    });
    child.send({
      descriptor: childDescriptor,
      edge_sha256: edgeSha256,
      schema: SCHEMAS.bootstrap,
    });
    const ready = await readyPromise;
    childState.identitySha256 = ready.identity_sha256;
    childState.ready = ready;
    this.causalHeadSha256 = ready.causal_head_sha256;
    this.subtreeIdentities.push(...ready.identities);
  }

  async launchChildren() {
    for (const [index, childDescriptor] of this.descriptor.children.entries()) {
      validateDescriptor(childDescriptor, childDescriptor.public.role);
      if (childDescriptor.fence_key !== this.fenceKey) {
        throw new Error("fixture child fence key was refused");
      }
      await this.launchChild(childDescriptor);
      if (
        this.descriptor.fault === "after-first-child-ready" &&
        index === 0
      ) {
        const child = this.children[index];
        const childIdentity = child.ready.identities.find(
          (entry) => entry.role === child.descriptor.public.role,
        );
        if (
          childIdentity === undefined ||
          childIdentity.identity_sha256 !== child.identitySha256 ||
          !isSha256(childIdentity.record_sha256)
        ) {
          throw new Error("fixture injected failure frontier was refused");
        }
        const failureRecord = signedValue(this.roleKey, "injected-failure", {
          causal_head_sha256: this.causalHeadSha256,
          child_edge_sha256: child.edgeSha256,
          child_identity_record_sha256: childIdentity.record_sha256,
          child_identity_sha256: child.identitySha256,
          child_role: child.descriptor.public.role,
          evidence_class: EVIDENCE_CLASS,
          failure_point: INJECTED_PARTIAL_FAILURE,
          hostagent_edge_sha256: this.edgeSha256,
          hostagent_identity_record_sha256: this.identityRecordSha256,
          hostagent_identity_sha256: this.identitySha256,
          operation_id: this.publicConfig.operation_id,
          schema: SCHEMAS.failure,
        });
        const failureSha256 = publishExclusive(
          this.publicConfig.failure_path,
          failureRecord,
        );
        const error = new Error("fixture injected partial-start failure");
        error.fixtureFailure = INJECTED_PARTIAL_FAILURE;
        error.fixtureFailureSha256 = failureSha256;
        throw error;
      }
    }
  }

  readyRecord() {
    return signedValue(this.roleKey, "ready", {
      causal_head_sha256: this.causalHeadSha256,
      edge_sha256: this.edgeSha256,
      evidence_class: EVIDENCE_CLASS,
      identities: [...this.subtreeIdentities].sort((left, right) =>
        left.role.localeCompare(right.role),
      ),
      identity_sha256: this.identitySha256,
      operation_id: this.publicConfig.operation_id,
      phase: "ready",
      pid: process.pid,
      role: ROLE,
      schema: SCHEMAS.ready,
    });
  }

  validateEnvelope(request, source) {
    const expectedSchema =
      source === "endpoint" ? SCHEMAS.request : SCHEMAS.ipcRequest;
    const body = verifySignedValue(
      request,
      this.roleKey,
      "request",
      "fixture request",
    );
    exactKeys(
      body,
      [
        "action",
        "challenge",
        "deadline_epoch_milliseconds",
        "expected_identity_sha256",
        "fence_sha256",
        "operation_id",
        "role",
        "schema",
      ],
      "fixture request body",
    );
    if (
      body.schema !== expectedSchema ||
      body.operation_id !== this.publicConfig.operation_id ||
      body.role !== ROLE ||
      body.expected_identity_sha256 !== this.identitySha256 ||
      !isSha256(body.challenge) ||
      (body.fence_sha256 !== null && !isSha256(body.fence_sha256))
    ) {
      throw new Error("fixture request was refused");
    }
    remainingDeadlineMilliseconds(body.deadline_epoch_milliseconds);
    if (
      (body.action === "probe" && body.fence_sha256 !== this.fenceSha256) ||
      (this.fenceSha256 !== null &&
        body.action !== "probe" &&
        body.fence_sha256 !== this.fenceSha256)
    ) {
      throw new Error("fixture request fence was refused");
    }
    const challengeSha256 = sha256Bytes(Buffer.from(body.challenge, "ascii"));
    if (
      this.acceptedChallenges.has(challengeSha256) ||
      this.acceptedChallenges.size >= MAX_ACCEPTED_CHALLENGES
    ) {
      throw new Error("fixture request challenge was refused");
    }
    this.acceptedChallenges.add(challengeSha256);
    return body;
  }

  response(
    action,
    challenge,
    fenceSha256,
    deadlineEpochMilliseconds,
    detail,
  ) {
    const body = {
      action,
      causal_head_sha256: this.causalHeadSha256,
      challenge_sha256: sha256Bytes(Buffer.from(challenge, "ascii")),
      current_ppid: process.ppid,
      deadline_epoch_milliseconds: deadlineEpochMilliseconds,
      detail,
      edge_sha256: this.edgeSha256,
      endpoint_identity: this.endpointIdentity,
      evidence_class: EVIDENCE_CLASS,
      fence_sha256: fenceSha256,
      identity_sha256: this.identitySha256,
      operation_id: this.publicConfig.operation_id,
      phase: this.phase,
      role: ROLE,
      schema: SCHEMAS.response,
    };
    return signedValue(this.roleKey, "response", body);
  }

  validateFence(fenceSha256) {
    if (!isSha256(fenceSha256)) {
      throw new Error("fixture quiescence fence was required");
    }
    const record = readExact(
      this.publicConfig.quiescence_fence_path,
      fenceSha256,
      "fixture quiescence fence",
    );
    const body = verifySignedValue(
      record,
      this.fenceKey,
      "quiescence-fence",
      "fixture quiescence fence",
    );
    exactKeys(
      body,
      [
        "causal_head_sha256",
        "endpoint_identities",
        "evidence_class",
        "nonce_sha256",
        "operation_id",
        "role_identities",
        "schema",
      ],
      "fixture quiescence-fence body",
    );
    if (
      body.schema !== SCHEMAS.fence ||
      body.evidence_class !== EVIDENCE_CLASS ||
      body.operation_id !== this.publicConfig.operation_id ||
      !isSha256(body.causal_head_sha256) ||
      !isSha256(body.nonce_sha256) ||
      !Array.isArray(body.role_identities) ||
      body.role_identities.length !== 4 ||
      !Array.isArray(body.endpoint_identities) ||
      body.endpoint_identities.length !== 4
    ) {
      throw new Error("fixture quiescence fence was refused");
    }
    const roleIdentities = new Map();
    const endpointIdentities = new Map();
    for (const entry of body.role_identities) {
      exactKeys(
        entry,
        ["identity_sha256", "role"],
        "fixture fence role identity",
      );
      if (
        !ROLES.includes(entry.role) ||
        roleIdentities.has(entry.role) ||
        !isSha256(entry.identity_sha256)
      ) {
        throw new Error("fixture fence role identity was refused");
      }
      roleIdentities.set(entry.role, entry.identity_sha256);
    }
    for (const entry of body.endpoint_identities) {
      exactKeys(
        entry,
        ["endpoint_identity", "role"],
        "fixture fence endpoint identity",
      );
      if (!ROLES.includes(entry.role) || endpointIdentities.has(entry.role)) {
        throw new Error("fixture fence endpoint identity was refused");
      }
      exactKeys(
        entry.endpoint_identity,
        ["dev", "ino", "mode", "nlink", "uid"],
        "fixture fence endpoint object",
      );
      endpointIdentities.set(entry.role, entry.endpoint_identity);
    }
    if (
      ROLES.some(
        (role) =>
          !roleIdentities.has(role) || !endpointIdentities.has(role),
      ) ||
      roleIdentities.get(ROLE) !== this.identitySha256 ||
      canonical(endpointIdentities.get(ROLE)) !== canonical(this.endpointIdentity)
    ) {
      throw new Error("fixture quiescence fence identity was refused");
    }
    return body;
  }

  async quiesce(fenceSha256, deadlineEpochMilliseconds) {
    if (this.phase !== "ready") {
      throw new Error("fixture quiescence phase was refused");
    }
    const fence = this.validateFence(fenceSha256);
    // Closing this phase gate precedes every recursive request. The sole spawn
    // path checks for `starting`, so no fixture child or endpoint can be added
    // after the fence begins.
    this.phase = "quiescing";
    this.fenceSha256 = fenceSha256;
    this.causalHeadSha256 = fence.causal_head_sha256;
    const childAckSha256s = [];
    for (const child of [...this.children].reverse()) {
      const response = await requestChild(
        child,
        "quiesce",
        fenceSha256,
        deadlineEpochMilliseconds,
      );
      const ackSha256 = response.detail?.ack_sha256;
      if (!isSha256(ackSha256)) {
        throw new Error("fixture child quiescence acknowledgement was refused");
      }
      const ack = readExact(
        child.descriptor.public.quiescence_ack_path,
        ackSha256,
        "fixture child quiescence acknowledgement",
      );
      verifySignedValue(
        ack,
        child.descriptor.role_key,
        "quiescence-ack",
        "fixture child quiescence acknowledgement",
      );
      childAckSha256s.push({ role: child.descriptor.public.role, sha256: ackSha256 });
    }
    const ack = signedValue(this.roleKey, "quiescence-ack", {
      causal_head_sha256: this.causalHeadSha256,
      child_ack_sha256s: childAckSha256s,
      evidence_class: EVIDENCE_CLASS,
      fence_sha256: fenceSha256,
      identity_sha256: this.identitySha256,
      operation_id: this.publicConfig.operation_id,
      phase: "quiesced",
      role: ROLE,
      schema: SCHEMAS.ack,
    });
    const ackSha256 = publishExclusive(
      this.publicConfig.quiescence_ack_path,
      ack,
    );
    this.phase = "quiesced";
    return ackSha256;
  }

  async detachHostagent(fenceSha256, deadlineEpochMilliseconds) {
    if (ROLE !== "outer" || this.phase !== "quiesced") {
      throw new Error("fixture detach phase was refused");
    }
    const hostagent = this.children[0];
    const response = await requestChild(
      hostagent,
      "detach",
      fenceSha256,
      deadlineEpochMilliseconds,
    );
    if (
      response.phase !== "detached-quiesced" ||
      response.detail?.detached !== true ||
      response.current_ppid !== process.pid
    ) {
      throw new Error("fixture hostagent detach was refused");
    }
    hostagent.process.unref();
    const detachAck = signedValue(this.roleKey, "detach-ack", {
      evidence_class: EVIDENCE_CLASS,
      fence_sha256: fenceSha256,
      hostagent_identity_sha256: hostagent.identitySha256,
      hostagent_ppid_before_outer_exit: response.current_ppid,
      operation_id: this.publicConfig.operation_id,
      outer_identity_sha256: this.identitySha256,
      schema: SCHEMAS.detach,
    });
    const detachAckSha256 = publishExclusive(
      this.publicConfig.detach_ack_path,
      detachAck,
    );
    const exitAck = signedValue(this.roleKey, "exit-ack", {
      action: "detach-complete",
      child_exit_ack_sha256s: [],
      detach_ack_sha256: detachAckSha256,
      evidence_class: EVIDENCE_CLASS,
      fence_sha256: fenceSha256,
      identity_sha256: this.identitySha256,
      operation_id: this.publicConfig.operation_id,
      role: ROLE,
      schema: SCHEMAS.exit,
    });
    const exitAckSha256 = publishExclusive(
      this.publicConfig.exit_ack_path,
      exitAck,
    );
    this.phase = "completing";
    return { detachAckSha256, exitAckSha256 };
  }

  async detachFromOuter(fenceSha256) {
    if (ROLE !== "hostagent" || this.phase !== "quiesced") {
      throw new Error("fixture hostagent detach phase was refused");
    }
    this.validateFence(fenceSha256);
    this.phase = "detached-quiesced";
  }

  async shutdown(fenceSha256, deadlineEpochMilliseconds) {
    if (
      !(
        (ROLE === "hostagent" && this.phase === "detached-quiesced") ||
        (EXPECTED_CHILDREN[ROLE].length === 0 && this.phase === "quiesced")
      )
    ) {
      throw new Error("fixture shutdown phase was refused");
    }
    this.validateFence(fenceSha256);
    this.phase = "stopping";
    const childExitAckSha256s = [];
    for (const child of [...this.children].reverse()) {
      const response = await requestChild(
        child,
        "shutdown",
        fenceSha256,
        deadlineEpochMilliseconds,
      );
      const exitAckSha256 = response.detail?.exit_ack_sha256;
      if (!isSha256(exitAckSha256)) {
        throw new Error("fixture child exit acknowledgement was refused");
      }
      const exitAck = readExact(
        child.descriptor.public.exit_ack_path,
        exitAckSha256,
        "fixture child exit acknowledgement",
      );
      verifySignedValue(
        exitAck,
        child.descriptor.role_key,
        "exit-ack",
        "fixture child exit acknowledgement",
      );
      await waitForChildExit(child, deadlineEpochMilliseconds);
      if (existsSync(child.descriptor.public.socket_path)) {
        throw new Error("fixture child endpoint remained after exit");
      }
      childExitAckSha256s.push({
        role: child.descriptor.public.role,
        sha256: exitAckSha256,
      });
    }
    const exitAck = signedValue(this.roleKey, "exit-ack", {
      action: "shutdown",
      child_exit_ack_sha256s: childExitAckSha256s,
      detach_ack_sha256: null,
      evidence_class: EVIDENCE_CLASS,
      fence_sha256: fenceSha256,
      identity_sha256: this.identitySha256,
      operation_id: this.publicConfig.operation_id,
      role: ROLE,
      schema: SCHEMAS.exit,
    });
    return publishExclusive(this.publicConfig.exit_ack_path, exitAck);
  }

  async handleRequest(request, source) {
    const body = this.validateEnvelope(request, source);
    if (body.action === "probe") {
      return {
        response: this.response(
          "probe",
          body.challenge,
          body.fence_sha256,
          body.deadline_epoch_milliseconds,
          {
            child_identities: this.children.map((child) => ({
              identity_sha256: child.identitySha256,
              role: child.descriptor.public.role,
            })),
            identity_scope: "trusted-fixture-nonce-not-os-start-identity",
          },
        ),
      };
    }
    if (
      body.action === "quiesce" &&
      ((ROLE === "outer" && source === "endpoint") ||
        (ROLE !== "outer" && source === "ipc"))
    ) {
      const ackSha256 = await this.quiesce(
        body.fence_sha256,
        body.deadline_epoch_milliseconds,
      );
      return {
        response: this.response(
          "quiesce",
          body.challenge,
          body.fence_sha256,
          body.deadline_epoch_milliseconds,
          { ack_sha256: ackSha256 },
        ),
      };
    }
    if (body.action === "detach") {
      if (ROLE === "outer" && source === "endpoint") {
        const detail = await this.detachHostagent(
          body.fence_sha256,
          body.deadline_epoch_milliseconds,
        );
        return {
          afterReply: () => void this.finishExit(),
          response: this.response(
            "detach",
            body.challenge,
            body.fence_sha256,
            body.deadline_epoch_milliseconds,
            {
              detach_ack_sha256: detail.detachAckSha256,
              exit_ack_sha256: detail.exitAckSha256,
            },
          ),
        };
      }
      if (ROLE === "hostagent" && source === "ipc") {
        await this.detachFromOuter(body.fence_sha256);
        return {
          afterReply: () => {
            if (process.connected) process.disconnect();
          },
          response: this.response(
            "detach",
            body.challenge,
            body.fence_sha256,
            body.deadline_epoch_milliseconds,
            { detached: true },
          ),
        };
      }
    }
    if (
      body.action === "shutdown" &&
      ((ROLE === "hostagent" && source === "endpoint") ||
        (EXPECTED_CHILDREN[ROLE].length === 0 && source === "ipc"))
    ) {
      const exitAckSha256 = await this.shutdown(
        body.fence_sha256,
        body.deadline_epoch_milliseconds,
      );
      return {
        afterReply: () => void this.finishExit(),
        response: this.response(
          "shutdown",
          body.challenge,
          body.fence_sha256,
          body.deadline_epoch_milliseconds,
          { exit_ack_sha256: exitAckSha256 },
        ),
      };
    }
    throw new Error("fixture action was refused");
  }

  installIpcHandler() {
    process.on("message", async (message) => {
      try {
        const outcome = await this.handleRequest(message, "ipc");
        process.send(outcome.response, (error) => {
          if (error !== null && error !== undefined) {
            void this.emergencyExit();
            return;
          }
          if (outcome.afterReply !== undefined) outcome.afterReply();
        });
      } catch {
        void this.emergencyExit();
      }
    });
    process.on("disconnect", () => {
      if (!(ROLE === "hostagent" && this.phase === "detached-quiesced")) {
        void this.emergencyExit();
      }
    });
  }

  async closeEndpoint() {
    if (this.endpoint === undefined) return;
    for (const socket of this.sockets) socket.destroy();
    await new Promise((resolvePromise) => this.endpoint.close(resolvePromise));
    if (existsSync(this.publicConfig.socket_path)) {
      if (
        canonical(assertPrivateRoot(this.publicConfig.workspace)) !==
        canonical(this.rootIdentity)
      ) {
        throw new Error("fixture root changed before endpoint unlink");
      }
      const current = socketIdentity(this.publicConfig.socket_path);
      if (!sameSocketIdentity(current, this.endpointIdentity)) {
        throw new Error("fixture endpoint changed before unlink");
      }
      unlinkSync(this.publicConfig.socket_path);
      syncDirectory(dirname(this.publicConfig.socket_path));
    }
  }

  async finishExit() {
    if (this.exiting) return;
    this.exiting = true;
    clearTimeout(this.watchdog);
    await this.closeEndpoint();
    process.exit(0);
  }

  async emergencyExit(exitCode = 70) {
    if (this.exiting) return;
    if (!new Set([70, INJECTED_PARTIAL_EXIT_CODE]).has(exitCode)) {
      process.exit(71);
    }
    this.exiting = true;
    clearTimeout(this.watchdog);
    let cleanupFailed = false;
    try {
      await Promise.all(this.children.map((child) => stopTrackedChild(child)));
    } catch {
      cleanupFailed = true;
    }
    try {
      await this.closeEndpoint();
    } catch {
      cleanupFailed = true;
    }
    // Emergency teardown is never evidence. A distinct code records that even
    // exact direct-handle cleanup could not be completely observed.
    process.exit(cleanupFailed ? 71 : exitCode);
  }

  async start() {
    this.watchdog = setTimeout(
      () => void this.emergencyExit(),
      this.publicConfig.maximum_lifetime_milliseconds,
    );
    try {
      await this.listen();
    } catch (error) {
      const failure = new Error("fixture endpoint start failed");
      failure.code = error?.code;
      throw failure;
    }
    try {
      this.publishIdentity();
    } catch (error) {
      const failure = new Error("fixture identity publication failed");
      failure.code = error?.code;
      throw failure;
    }
    try {
      await this.launchChildren();
    } catch (error) {
      const failure = new Error("fixture child launch failed");
      failure.code = error?.code;
      failure.fixtureFailure = error?.fixtureFailure;
      failure.fixtureFailureSha256 = error?.fixtureFailureSha256;
      throw failure;
    }
    if (this.publicConfig.startup_delay_milliseconds > 0) {
      await new Promise((resolvePromise) =>
        setTimeout(
          resolvePromise,
          this.publicConfig.startup_delay_milliseconds,
        ),
      );
    }
    this.phase = "ready";
    this.installIpcHandler();
    process.send(this.readyRecord());
  }
}

async function main() {
  if (
    !ROLES.includes(ROLE) ||
    typeof EDGE_PATH !== "string" ||
    process.argv.length !== 4 ||
    process.cwd() === dirname(ROLE_FILE) ||
    canonical(environmentValue()) !== canonical(closedFixtureEnvironment())
  ) {
    throw new Error("fixture role invocation was refused");
  }
  process.umask(0o177);
  const { descriptor, edgeSha256 } = await waitForBootstrap();
  if (
    descriptor.public.role !== ROLE ||
    descriptor.public.edge_path !== EDGE_PATH ||
    descriptor.public.workspace !== process.cwd() ||
    canonical(process.argv) !== canonical(expectedArgv(ROLE, EDGE_PATH))
  ) {
    throw new Error("fixture role bootstrap did not bind invocation");
  }
  const launchEdge = validateLaunchEdge(descriptor, edgeSha256);
  const runtime = new RoleRuntime(descriptor, launchEdge);
  activeRuntime = runtime;
  process.on("SIGINT", () => void runtime.emergencyExit());
  process.on("SIGTERM", () => void runtime.emergencyExit());
  await runtime.start();
}

main().catch(async (error) => {
  const role = ROLES.includes(ROLE) ? ROLE : "unknown";
  process.stderr.write(`four-role fixture ${role} failed\n`);
  if (activeRuntime !== undefined) {
    await activeRuntime.emergencyExit(
      error?.fixtureFailure === INJECTED_PARTIAL_FAILURE
        ? INJECTED_PARTIAL_EXIT_CODE
        : 70,
    );
  }
  process.exit(70);
});

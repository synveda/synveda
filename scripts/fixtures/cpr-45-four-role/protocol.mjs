import {
  createHash,
  createHmac,
  randomBytes,
  timingSafeEqual,
} from "node:crypto";
import {
  closeSync,
  constants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  openSync,
  readFileSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { createConnection } from "node:net";
import { basename, dirname, isAbsolute, relative, resolve } from "node:path";

export const EVIDENCE_CLASS =
  "deterministic-four-role-protocol-fixture-only";
export const ROLES = Object.freeze([
  "outer",
  "hostagent",
  "usernet",
  "ssh-controlmaster",
]);
export const PHASES = Object.freeze([
  "starting",
  "ready",
  "quiescing",
  "quiesced",
  "detached-quiesced",
  "completing",
  "stopping",
]);
export const ROLE_EDGES = Object.freeze([
  Object.freeze(["outer", "hostagent"]),
  Object.freeze(["hostagent", "usernet"]),
  Object.freeze(["hostagent", "ssh-controlmaster"]),
]);
export const MAX_FRAME_BYTES = 4 * 1024;
export const MAX_GRAPH_BYTES = 32 * 1024;
export const MAX_ACCEPTED_CHALLENGES = 32;
export const MAX_SOCKET_PATH_BYTES = 103;
export const MAX_ARTIFACT_BYTES = 32 * 1024;
export const MAX_REQUEST_LIFETIME_MILLISECONDS = 18_000;

export const SCHEMAS = Object.freeze({
  ack: "synveda.fixture.cpr45-four-role-quiescence-ack.v1",
  admission: "synveda.fixture.cpr45-four-role-admission.v1",
  bootstrap: "synveda.fixture.cpr45-four-role-bootstrap.v1",
  config: "synveda.fixture.cpr45-four-role-config.v1",
  detach: "synveda.fixture.cpr45-four-role-detach-ack.v1",
  edge: "synveda.fixture.cpr45-four-role-launch-edge.v1",
  exit: "synveda.fixture.cpr45-four-role-exit-ack.v1",
  failure: "synveda.fixture.cpr45-four-role-failure.v1",
  fence: "synveda.fixture.cpr45-four-role-quiescence-fence.v1",
  identity: "synveda.fixture.cpr45-four-role-identity.v1",
  ipcRequest: "synveda.fixture.cpr45-four-role-ipc-request.v1",
  ready: "synveda.fixture.cpr45-four-role-ready.v1",
  request: "synveda.fixture.cpr45-four-role-request.v1",
  response: "synveda.fixture.cpr45-four-role-response.v1",
});

export function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) {
    return String(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((entry) => canonical(entry)).join(",")}]`;
  }
  if (typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`;
  }
  throw new Error("unsupported canonical value");
}

export function canonicalBytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

export function sha256Bytes(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function sha256Value(value) {
  return sha256Bytes(canonicalBytes(value));
}

export function isSha256(value) {
  return typeof value === "string" && /^[0-9a-f]{64}$/u.test(value);
}

export function remainingDeadlineMilliseconds(deadlineEpochMilliseconds) {
  const remaining = deadlineEpochMilliseconds - Date.now();
  if (
    !Number.isSafeInteger(deadlineEpochMilliseconds) ||
    remaining < 1 ||
    remaining > MAX_REQUEST_LIFETIME_MILLISECONDS
  ) {
    throw new Error("fixture request deadline was refused");
  }
  return remaining;
}

export function randomSha256() {
  return randomBytes(32).toString("hex");
}

export function closedFixtureEnvironment() {
  if (process.platform !== "darwin") return {};
  const value = process.env.__CF_USER_TEXT_ENCODING;
  if (typeof value !== "string" || !/^0x[0-9A-F]+:[0-9]+:[0-9]+$/u.test(value)) {
    throw new Error("fixture Darwin text environment was refused");
  }
  return { __CF_USER_TEXT_ENCODING: value };
}

export function exactKeys(value, keys, label = "value") {
  if (
    value === null ||
    Array.isArray(value) ||
    typeof value !== "object" ||
    canonical(Object.keys(value).sort()) !== canonical([...keys].sort())
  ) {
    throw new Error(`${label} fields were refused`);
  }
}

export function bootstrapCommitment(descriptor) {
  const count = { value: 0 };
  const visit = (value, depth) => {
    count.value += 1;
    if (depth > 2 || count.value > 4) {
      throw new Error("fixture bootstrap tree exceeded its bound");
    }
    exactKeys(
      value,
      ["children", "edge_key", "fault", "fence_key", "public", "role_key"],
      "fixture bootstrap commitment",
    );
    if (
      !isSha256(value.edge_key) ||
      !isSha256(value.fence_key) ||
      !isSha256(value.role_key) ||
      !Array.isArray(value.children) ||
      !new Set(["none", "after-first-child-ready"]).has(value.fault)
    ) {
      throw new Error("fixture bootstrap commitment was refused");
    }
    return {
      children: value.children.map((child) => visit(child, depth + 1)),
      edge_key_sha256: sha256Bytes(Buffer.from(value.edge_key, "ascii")),
      fence_key_sha256: sha256Bytes(Buffer.from(value.fence_key, "ascii")),
      fault: value.fault,
      public_sha256: sha256Value(value.public),
      role_key_sha256: sha256Bytes(Buffer.from(value.role_key, "ascii")),
    };
  };
  return visit(descriptor, 0);
}

export function signValue(key, domain, value) {
  if (!isSha256(key) || !/^[a-z0-9-]{1,64}$/u.test(domain)) {
    throw new Error("fixture signing input was refused");
  }
  return createHmac("sha256", Buffer.from(key, "hex"))
    .update(`synveda.fixture.cpr45-four-role.v1\0${domain}\0`, "utf8")
    .update(canonicalBytes(value))
    .digest("hex");
}

export function proofEquals(left, right) {
  return (
    isSha256(left) &&
    isSha256(right) &&
    timingSafeEqual(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"))
  );
}

export function signedValue(key, domain, body) {
  return {
    ...body,
    proof_sha256: signValue(key, domain, body),
  };
}

export function verifySignedValue(value, key, domain, label = "record") {
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new Error(`${label} was refused`);
  }
  const { proof_sha256: proof, ...body } = value;
  if (!proofEquals(proof, signValue(key, domain, body))) {
    throw new Error(`${label} proof was refused`);
  }
  return body;
}

function sameMetadata(left, right) {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.nlink === right.nlink &&
    left.size === right.size &&
    left.uid === right.uid
  );
}

export function syncDirectory(path) {
  const descriptor = openSync(
    path,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

export function publishExclusive(path, value) {
  if (!isAbsolute(path) || resolve(path) !== path) {
    throw new Error("fixture artifact path was refused");
  }
  const parentIdentity = privateDirectoryIdentity(dirname(path));
  const valueBytes = canonicalBytes(value);
  if (valueBytes.length > MAX_ARTIFACT_BYTES) {
    throw new Error("fixture artifact exceeded its byte bound");
  }
  const stagePath = resolve(
    dirname(path),
    `.fixture-stage-${sha256Bytes(Buffer.from(basename(path), "utf8"))}-${randomBytes(16).toString("hex")}`,
  );
  const descriptor = openSync(
    stagePath,
    constants.O_WRONLY |
      constants.O_CREAT |
      constants.O_EXCL |
      constants.O_NOFOLLOW,
    0o600,
  );
  try {
    fchmodSync(descriptor, 0o600);
    let offset = 0;
    while (offset < valueBytes.length) {
      const written = writeSync(
        descriptor,
        valueBytes,
        offset,
        valueBytes.length - offset,
        offset,
      );
      if (written < 1) throw new Error("fixture artifact write failed");
      offset += written;
    }
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
  try {
    if (
      canonical(privateDirectoryIdentity(dirname(path))) !==
      canonical(parentIdentity)
    ) {
      throw new Error("fixture artifact parent changed before publication");
    }
    linkSync(stagePath, path);
    syncDirectory(dirname(path));
  } finally {
    unlinkSync(stagePath);
    syncDirectory(dirname(path));
  }
  if (
    canonical(privateDirectoryIdentity(dirname(path))) !== canonical(parentIdentity)
  ) {
    throw new Error("fixture artifact parent changed after publication");
  }
  return sha256Bytes(valueBytes);
}

export function readPublished(path, label = "artifact") {
  if (!isAbsolute(path) || resolve(path) !== path) {
    throw new Error(`${label} path was refused`);
  }
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  let valueBytes;
  try {
    const before = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(path, { bigint: true });
    if (
      !before.isFile() ||
      !sameMetadata(before, named) ||
      before.uid !== BigInt(process.getuid()) ||
      before.nlink !== 1n ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size < 1n ||
      before.size > BigInt(MAX_ARTIFACT_BYTES)
    ) {
      throw new Error(`${label} identity was refused`);
    }
    valueBytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (
      !sameMetadata(before, after)
    ) {
      throw new Error(`${label} identity changed`);
    }
  } finally {
    closeSync(descriptor);
  }
  const value = JSON.parse(valueBytes.toString("utf8"));
  if (!canonicalBytes(value).equals(valueBytes)) {
    throw new Error(`${label} was not canonical`);
  }
  return { sha256: sha256Bytes(valueBytes), value };
}

export function readExact(path, expectedSha256, label = "artifact") {
  if (!isSha256(expectedSha256)) {
    throw new Error(`${label} digest was refused`);
  }
  const published = readPublished(path, label);
  if (published.sha256 !== expectedSha256) {
    throw new Error(`${label} digest changed`);
  }
  return published.value;
}

export function privateDirectoryIdentity(path) {
  if (!isAbsolute(path) || resolve(path) !== path) {
    throw new Error("fixture private directory was refused");
  }
  const identity = lstatSync(path, { bigint: true });
  if (
    !identity.isDirectory() ||
    identity.isSymbolicLink() ||
    identity.uid !== BigInt(process.getuid()) ||
    (identity.mode & 0o7777n) !== 0o700n
  ) {
    throw new Error("fixture private directory identity was refused");
  }
  return {
    dev: identity.dev.toString(),
    ino: identity.ino.toString(),
    mode: Number(identity.mode & 0o7777n),
    uid: Number(identity.uid),
  };
}

export function assertPrivateRoot(path) {
  return privateDirectoryIdentity(path);
}

export function assertPathInRoot(path, root, socket = false) {
  if (!isAbsolute(path) || resolve(path) !== path) {
    throw new Error("fixture path was refused");
  }
  const pathFromRoot = relative(root, path);
  if (
    pathFromRoot.length < 1 ||
    pathFromRoot === ".." ||
    pathFromRoot.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) ||
    isAbsolute(pathFromRoot)
  ) {
    throw new Error("fixture path escaped its private root");
  }
  if (socket && Buffer.byteLength(path, "utf8") > MAX_SOCKET_PATH_BYTES) {
    throw new Error("fixture socket path exceeded its byte bound");
  }
}

export function socketIdentity(path) {
  const identity = lstatSync(path, { bigint: true });
  if (
    !identity.isSocket() ||
    identity.isSymbolicLink() ||
    identity.uid !== BigInt(process.getuid()) ||
    identity.nlink !== 1n ||
    (identity.mode & 0o7777n) !== 0o600n
  ) {
    throw new Error("fixture endpoint identity was refused");
  }
  return {
    dev: identity.dev.toString(),
    ino: identity.ino.toString(),
    mode: Number(identity.mode & 0o7777n),
    nlink: Number(identity.nlink),
    uid: Number(identity.uid),
  };
}

export function sameSocketIdentity(left, right) {
  return canonical(left) === canonical(right);
}

export async function requestFrame(path, request, timeoutMilliseconds = 12_000) {
  const requestBytes = canonicalBytes(request);
  if (requestBytes.length > MAX_FRAME_BYTES) {
    throw new Error("fixture request exceeded its byte bound");
  }
  return await new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    let size = 0;
    const chunks = [];
    const socket = createConnection({ path });
    const finish = (error, value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const timer = setTimeout(
      () => finish(new Error("fixture endpoint request timed out")),
      timeoutMilliseconds,
    );
    socket.once("connect", () => socket.end(requestBytes));
    socket.on("data", (chunk) => {
      if (size + chunk.length > MAX_FRAME_BYTES) {
        finish(new Error("fixture response exceeded its byte bound"));
        return;
      }
      size += chunk.length;
      chunks.push(chunk);
    });
    socket.once("error", () =>
      finish(new Error("fixture endpoint refused the request")),
    );
    socket.once("end", () => {
      try {
        const responseBytes = Buffer.concat(chunks, size);
        if (
          responseBytes.length < 2 ||
          responseBytes[responseBytes.length - 1] !== 0x0a
        ) {
          throw new Error("fixture endpoint returned no complete frame");
        }
        const response = JSON.parse(
          responseBytes.subarray(0, responseBytes.length - 1).toString("utf8"),
        );
        if (!canonicalBytes(response).equals(responseBytes)) {
          throw new Error("fixture endpoint response was not canonical");
        }
        finish(undefined, response);
      } catch {
        finish(new Error("fixture endpoint response was refused"));
      }
    });
  });
}

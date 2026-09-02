#!/usr/bin/env node
import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import {
  chmodSync,
  closeSync,
  constants,
  existsSync,
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
import { createServer } from "node:net";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";

const MAX_REQUEST_BYTES = 4096;

function canonical(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "number" && Number.isSafeInteger(value)) return String(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`;
  }
  throw new Error("unsupported canonical value");
}

function bytes(value) {
  return Buffer.from(`${canonical(value)}\n`, "utf8");
}

function digest(value) {
  return createHash("sha256").update(value).digest("hex");
}

function proof(secret, action, challenge, processIdentity) {
  return createHmac("sha256", secret)
    .update(
      `synveda.clean-engine.background-provider.v1\0${action}\0${challenge}\0${processIdentity}`,
    )
    .digest("hex");
}

function artifactProof(secret, action, value) {
  return createHmac("sha256", secret)
    .update(`synveda.clean-engine.background-provider-artifact.v1\0${action}\0`)
    .update(bytes(value))
    .digest("hex");
}

function proofEquals(left, right) {
  return (
    typeof left === "string" &&
    typeof right === "string" &&
    /^[0-9a-f]{64}$/.test(left) &&
    /^[0-9a-f]{64}$/.test(right) &&
    timingSafeEqual(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"))
  );
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

function evaluatedSource() {
  const index = process.execArgv.indexOf("--eval");
  if (index < 0 || typeof process.execArgv[index + 1] !== "string") {
    throw new Error("evaluated hostagent source was unavailable");
  }
  return process.execArgv[index + 1];
}

function engineArchitecture(nodeArchitecture) {
  if (nodeArchitecture === "arm64") return "aarch64";
  if (nodeArchitecture === "x64") return "x86_64";
  throw new Error("host architecture was refused");
}

function exactKeys(value, keys) {
  if (
    value === null ||
    Array.isArray(value) ||
    typeof value !== "object" ||
    JSON.stringify(Object.keys(value).sort()) !== JSON.stringify([...keys].sort())
  ) {
    throw new Error("fields were refused");
  }
}

function syncDirectory(path) {
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

function publish(path, value) {
  const valueBytes = bytes(value);
  const stagePath = join(
    dirname(path),
    `.background-private-stage-${digest(Buffer.from(basename(path), "utf8"))}-${digest(valueBytes)}-${randomBytes(16).toString("hex")}`,
  );
  const descriptor = openSync(
    stagePath,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
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
      if (written < 1) throw new Error("artifact write failed");
      offset += written;
    }
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
  linkSync(stagePath, path);
  syncDirectory(dirname(path));
  unlinkSync(stagePath);
  syncDirectory(dirname(path));
}

function readConfig(path, expectedSha256) {
  if (
    !isAbsolute(path) ||
    resolve(path) !== path ||
    typeof expectedSha256 !== "string" ||
    !/^[0-9a-f]{64}$/.test(expectedSha256)
  ) {
    throw new Error("config path or digest was refused");
  }
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  let configBytes;
  try {
    const before = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(path, { bigint: true });
    if (
      !before.isFile() ||
      before.isSymbolicLink() ||
      !sameMetadata(before, named) ||
      before.uid !== BigInt(process.getuid()) ||
      before.nlink !== 1n ||
      (before.mode & 0o7777n) !== 0o600n ||
      before.size < 1n ||
      before.size > 128n * 1024n
    ) {
      throw new Error("config identity was refused");
    }
    configBytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameMetadata(before, after) || digest(configBytes) !== expectedSha256) {
      throw new Error("config identity changed");
    }
  } finally {
    closeSync(descriptor);
  }
  const value = JSON.parse(configBytes.toString("utf8"));
  if (!bytes(value).equals(configBytes)) throw new Error("config was not canonical");
  exactKeys(value, [
    "engine_architecture",
    "engine_socket",
    "fixture_id",
    "ha_socket",
    "hostagent_script_sha256",
    "instance_nonce",
    "maximum_lifetime_milliseconds",
    "node_sha256",
    "pid_record",
    "profile",
    "schema",
    "working_directory",
  ]);
  if (
    value.schema !== "synveda.clean-engine.background-hostagent-config.v1" ||
    value.engine_architecture !== engineArchitecture(process.arch) ||
    !/^[0-9a-f]{32}$/.test(value.fixture_id) ||
    !/^[0-9a-f]{64}$/.test(value.instance_nonce) ||
    !/^[0-9a-f]{64}$/.test(value.hostagent_script_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.node_sha256) ||
    !Number.isSafeInteger(value.maximum_lifetime_milliseconds) ||
    value.maximum_lifetime_milliseconds < 1_000 ||
    value.maximum_lifetime_milliseconds > 30_000 ||
    ![value.engine_socket, value.ha_socket, value.pid_record, value.working_directory].every(
      (candidate) => typeof candidate === "string" && isAbsolute(candidate),
    ) ||
    process.cwd() !== value.working_directory ||
    digest(Buffer.from(evaluatedSource(), "utf8")) !== value.hostagent_script_sha256 ||
    digest(readFileSync(process.execPath)) !== value.node_sha256
  ) {
    throw new Error("config was refused");
  }
  return { configSha256: digest(configBytes), value };
}

function serverResponse(config, processIdentity, kind, request) {
  exactKeys(request, ["action", "challenge"]);
  if (!/^[0-9a-f]{64}$/.test(request.challenge)) throw new Error("challenge was refused");
  if (kind === "hostagent" && request.action === "probe") {
    return {
      challenge_sha256: digest(Buffer.from(request.challenge, "ascii")),
      fixture_id: config.fixture_id,
      pid: process.pid,
      process_instance_sha256: processIdentity,
      profile: config.profile,
      proof_sha256: proof(
        config.instance_nonce,
        "hostagent-probe",
        request.challenge,
        processIdentity,
      ),
      schema: "synveda.clean-engine.background-hostagent-probe.v1",
    };
  }
  if (kind === "engine" && request.action === "version") {
    return {
      api_version: "1.52",
      architecture: config.engine_architecture,
      challenge_sha256: digest(Buffer.from(request.challenge, "ascii")),
      fixture_id: config.fixture_id,
      name: `synveda-cpr45-${config.fixture_id}`,
      operating_system: "linux",
      process_instance_sha256: processIdentity,
      proof_sha256: proof(
        config.instance_nonce,
        "engine-version",
        request.challenge,
        processIdentity,
      ),
      schema: "synveda.clean-engine.background-engine-probe.v1",
      server_id: digest(Buffer.from(`engine\0${config.fixture_id}\0${processIdentity}`, "utf8")),
      version: "29.4.0-fake",
    };
  }
  throw new Error("action was refused");
}

function createProtocolServer(config, processIdentity, kind, onShutdown) {
  const sockets = new Set();
  const server = createServer({ allowHalfOpen: true }, (socket) => {
    sockets.add(socket);
    socket.setTimeout(1_000, () => socket.destroy());
    socket.once("close", () => sockets.delete(socket));
    let requestBytes = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      requestBytes = Buffer.concat([requestBytes, chunk]);
      if (requestBytes.length > MAX_REQUEST_BYTES) socket.destroy();
    });
    socket.once("end", () => {
      if (requestBytes.length > MAX_REQUEST_BYTES) return;
      const newline = requestBytes.indexOf(0x0a);
      if (newline < 1 || newline !== requestBytes.length - 1) {
        socket.destroy();
        return;
      }
      const frame = requestBytes.subarray(0, newline);
      try {
        const request = JSON.parse(frame.toString("utf8"));
        if (!bytes(request).equals(requestBytes)) throw new Error("request was not canonical");
        if (kind === "hostagent" && request?.action === "shutdown") {
          exactKeys(request, ["action", "challenge", "proof_sha256"]);
          if (
            !/^[0-9a-f]{64}$/.test(request.challenge) ||
            !proofEquals(
              request.proof_sha256,
              proof(
                config.instance_nonce,
                "hostagent-shutdown",
                request.challenge,
                processIdentity,
              ),
            )
          ) {
            throw new Error("shutdown proof was refused");
          }
          socket.end(
            `${canonical({
              challenge_sha256: digest(Buffer.from(request.challenge, "ascii")),
              fixture_id: config.fixture_id,
              process_instance_sha256: processIdentity,
              proof_sha256: proof(
                config.instance_nonce,
                "hostagent-shutdown-accepted",
                request.challenge,
                processIdentity,
              ),
              schema: "synveda.clean-engine.background-hostagent-shutdown.v1",
            })}\n`,
          );
          socket.once("close", onShutdown);
          return;
        }
        socket.end(`${canonical(serverResponse(config, processIdentity, kind, request))}\n`);
      } catch {
        socket.destroy();
      }
    });
  });
  return { server, sockets };
}

async function listen(endpoint, path) {
  if (existsSync(path)) throw new Error("socket collision");
  await new Promise((resolvePromise, rejectPromise) => {
    endpoint.server.once("error", rejectPromise);
    endpoint.server.listen(path, () => {
      endpoint.server.off("error", rejectPromise);
      resolvePromise();
    });
  });
}

async function closeServer(endpoint) {
  for (const socket of endpoint.sockets) socket.destroy();
  await new Promise((resolvePromise) => endpoint.server.close(resolvePromise));
}

async function main() {
  const { configSha256, value: config } = readConfig(process.argv[1], process.argv[2]);
  const providerStartDecisionSha256 = process.argv[3];
  const controllerWitnessSha256 = process.argv[4];
  if (
    !/^[0-9a-f]{64}$/.test(providerStartDecisionSha256 ?? "") ||
    !/^[0-9a-f]{64}$/.test(controllerWitnessSha256 ?? "")
  ) {
    throw new Error("provider start decision digest was refused");
  }
  process.umask(0o177);
  const processIdentity = digest(
    Buffer.from(
      [
        "synveda.clean-engine.background-process.v1",
        config.instance_nonce,
        process.pid,
        process.ppid,
        process.hrtime.bigint(),
        randomBytes(16).toString("hex"),
      ].join("\0"),
      "utf8",
    ),
  );
  let shuttingDown = false;
  let detached = false;
  let lifetime;
  const shutdown = async () => {
    if (shuttingDown) return;
    shuttingDown = true;
    clearTimeout(lifetime);
    await Promise.all([closeServer(hostagent), closeServer(engine)]);
    process.exit(0);
  };
  const hostagent = createProtocolServer(config, processIdentity, "hostagent", shutdown);
  const engine = createProtocolServer(config, processIdentity, "engine", () => {});
  const disconnectBeforeDetach = () => {
    if (!detached) void shutdown();
  };
  process.on("disconnect", disconnectBeforeDetach);
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
  await listen(hostagent, config.ha_socket);
  await listen(engine, config.engine_socket);
  chmodSync(config.ha_socket, 0o600);
  chmodSync(config.engine_socket, 0o600);
  syncDirectory(dirname(config.ha_socket));
  syncDirectory(dirname(config.engine_socket));
  const pidIdentity = {
    controller_witness_sha256: controllerWitnessSha256,
    environment_keys: Object.keys(process.env).sort(),
    fixture_id: config.fixture_id,
    hostagent_config_sha256: configSha256,
    hostagent_script_sha256: config.hostagent_script_sha256,
    instance_nonce_sha256: digest(Buffer.from(config.instance_nonce, "ascii")),
    node_sha256: config.node_sha256,
    pid: process.pid,
    ppid: process.ppid,
    process_instance_sha256: processIdentity,
    profile: config.profile,
    provider_start_decision_sha256: providerStartDecisionSha256,
    schema: "synveda.clean-engine.background-hostagent-pid.v2",
    working_directory: process.cwd(),
  };
  const pidRecord = {
    ...pidIdentity,
    proof_sha256: artifactProof(config.instance_nonce, "hostagent-pid", pidIdentity),
  };
  publish(config.pid_record, pidRecord);
  if (typeof process.send !== "function") throw new Error("controller channel was unavailable");
  process.send({
    fixture_id: config.fixture_id,
    pid: process.pid,
    process_instance_sha256: processIdentity,
    schema: "synveda.clean-engine.background-hostagent-ready.v1",
  });
  await new Promise((resolvePromise, rejectPromise) => {
    let settled = false;
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (error === undefined) resolvePromise();
      else rejectPromise(error);
    };
    const timeout = setTimeout(
      () => finish(new Error("hostagent detach timed out")),
      8_000,
    );
    process.once("message", (message) => {
      try {
        exactKeys(message, ["action", "challenge", "proof_sha256"]);
        if (
          message.action !== "detach" ||
          !/^[0-9a-f]{64}$/.test(message.challenge) ||
          !proofEquals(
            message.proof_sha256,
            proof(
              config.instance_nonce,
              "hostagent-detach",
              message.challenge,
              processIdentity,
            ),
          )
        ) {
          throw new Error("hostagent detach was refused");
        }
        process.send(
          {
            challenge_sha256: digest(Buffer.from(message.challenge, "ascii")),
            fixture_id: config.fixture_id,
            process_instance_sha256: processIdentity,
            proof_sha256: proof(
              config.instance_nonce,
              "hostagent-detached",
              message.challenge,
              processIdentity,
            ),
            schema: "synveda.clean-engine.background-hostagent-detached.v1",
          },
          (error) => {
            if (error !== null && error !== undefined) {
              finish(error);
              return;
            }
            detached = true;
            process.off("disconnect", disconnectBeforeDetach);
            finish();
          },
        );
      } catch (error) {
        finish(error);
      }
    });
  });
  if (process.connected) process.disconnect();
  lifetime = setTimeout(shutdown, config.maximum_lifetime_milliseconds);
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : "closed failure";
  process.stderr.write(
    `clean-engine-background-hostagent: ${message}\n`,
  );
  process.exit(70);
});

#!/usr/bin/env node
import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import {
  closeSync,
  constants,
  fchmodSync,
  fsyncSync,
  openSync,
  readFileSync,
  writeSync,
} from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { spawn } from "node:child_process";

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

function proofEquals(left, right) {
  return (
    typeof left === "string" &&
    typeof right === "string" &&
    /^[0-9a-f]{64}$/.test(left) &&
    /^[0-9a-f]{64}$/.test(right) &&
    timingSafeEqual(Buffer.from(left, "ascii"), Buffer.from(right, "ascii"))
  );
}

function evaluatedSource() {
  const index = process.execArgv.indexOf("--eval");
  if (index < 0 || typeof process.execArgv[index + 1] !== "string") {
    throw new Error("evaluated controller source was unavailable");
  }
  return process.execArgv[index + 1];
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
  const descriptor = openSync(
    path,
    constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
    0o600,
  );
  try {
    fchmodSync(descriptor, 0o600);
    const valueBytes = bytes(value);
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
  syncDirectory(dirname(path));
}

function readConfig(path) {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error("config path was refused");
  const configBytes = readFileSync(path);
  const value = JSON.parse(configBytes.toString("utf8"));
  if (!bytes(value).equals(configBytes)) throw new Error("config was not canonical");
  exactKeys(value, [
    "controller_nonce",
    "controller_ready",
    "controller_script_sha256",
    "fixture_id",
    "hostagent_config",
    "hostagent_source_base64",
    "hostagent_source_sha256",
    "maximum_lifetime_milliseconds",
    "node_sha256",
    "schema",
    "working_directory",
  ]);
  if (
    value.schema !== "synveda.clean-engine.background-controller-config.v1" ||
    !/^[0-9a-f]{64}$/.test(value.controller_nonce) ||
    !/^[0-9a-f]{64}$/.test(value.controller_script_sha256) ||
    !/^[0-9a-f]{32}$/.test(value.fixture_id) ||
    !/^[0-9a-f]{64}$/.test(value.hostagent_source_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.node_sha256) ||
    !Number.isSafeInteger(value.maximum_lifetime_milliseconds) ||
    value.maximum_lifetime_milliseconds < 1_000 ||
    value.maximum_lifetime_milliseconds > 30_000 ||
    ![value.controller_ready, value.hostagent_config, value.working_directory].every(
      (candidate) => typeof candidate === "string" && isAbsolute(candidate),
    ) ||
    typeof value.hostagent_source_base64 !== "string"
  ) {
    throw new Error("config was refused");
  }
  const hostagentSource = Buffer.from(value.hostagent_source_base64, "base64");
  if (
    hostagentSource.toString("base64") !== value.hostagent_source_base64 ||
    digest(hostagentSource) !== value.hostagent_source_sha256
  ) {
    throw new Error("hostagent execution source was refused");
  }
  if (
    process.cwd() !== value.working_directory ||
    digest(Buffer.from(evaluatedSource(), "utf8")) !== value.controller_script_sha256 ||
    digest(readFileSync(process.execPath)) !== value.node_sha256
  ) {
    throw new Error("controller execution identity was refused");
  }
  return { hostagentSource: hostagentSource.toString("utf8"), value };
}

async function main() {
  const { hostagentSource, value: config } = readConfig(process.argv[1]);
  const controllerProcessIdentity = digest(
    Buffer.from(
      [
        "synveda.clean-engine.background-controller.v1",
        config.controller_nonce,
        process.pid,
        process.ppid,
        process.hrtime.bigint(),
        randomBytes(16).toString("hex"),
      ].join("\0"),
      "utf8",
    ),
  );
  const child = spawn(
    process.execPath,
    ["--input-type=module", "--eval", hostagentSource, config.hostagent_config],
    {
      cwd: config.working_directory,
      detached: true,
      env: process.env,
      stdio: ["ignore", "ignore", "ignore", "ipc"],
    },
  );
  const ready = await new Promise((resolvePromise, rejectPromise) => {
    const timeout = setTimeout(
      () => rejectPromise(new Error("hostagent readiness timed out")),
      8_000,
    );
    child.once("error", rejectPromise);
    child.once("exit", (status, signal) => {
      rejectPromise(new Error(`hostagent exited before readiness: ${status ?? signal}`));
    });
    child.once("message", (message) => {
      clearTimeout(timeout);
      resolvePromise(message);
    });
  });
  exactKeys(ready, ["fixture_id", "pid", "process_instance_sha256", "schema"]);
  if (
    ready.schema !== "synveda.clean-engine.background-hostagent-ready.v1" ||
    ready.fixture_id !== config.fixture_id ||
    !Number.isSafeInteger(ready.pid) ||
    ready.pid < 2 ||
    !/^[0-9a-f]{64}$/.test(ready.process_instance_sha256)
  ) {
    throw new Error("hostagent readiness was refused");
  }
  child.disconnect();
  child.unref();
  publish(config.controller_ready, {
    controller_environment_keys: Object.keys(process.env).sort(),
    controller_pid: process.pid,
    controller_process_instance_sha256: controllerProcessIdentity,
    controller_script_sha256: digest(Buffer.from(evaluatedSource(), "utf8")),
    fixture_id: config.fixture_id,
    hostagent_pid: ready.pid,
    hostagent_process_instance_sha256: ready.process_instance_sha256,
    node_sha256: digest(readFileSync(process.execPath)),
    proof_sha256: proof(
      config.controller_nonce,
      "controller-ready",
      config.fixture_id,
      controllerProcessIdentity,
    ),
    schema: "synveda.clean-engine.background-controller-ready.v1",
    working_directory: process.cwd(),
  });
  const lifetime = setTimeout(
    () => process.exit(70),
    config.maximum_lifetime_milliseconds,
  );
  process.on("message", (message) => {
    try {
      exactKeys(message, ["action", "challenge", "proof_sha256"]);
      if (
        message.action !== "shutdown" ||
        !/^[0-9a-f]{64}$/.test(message.challenge) ||
        !proofEquals(
          message.proof_sha256,
          proof(
            config.controller_nonce,
            "controller-shutdown",
            message.challenge,
            controllerProcessIdentity,
          ),
        )
      ) {
        throw new Error("controller shutdown was refused");
      }
      clearTimeout(lifetime);
      process.send(
        {
          challenge_sha256: digest(Buffer.from(message.challenge, "ascii")),
          fixture_id: config.fixture_id,
          process_instance_sha256: controllerProcessIdentity,
          proof_sha256: proof(
            config.controller_nonce,
            "controller-shutdown-accepted",
            message.challenge,
            controllerProcessIdentity,
          ),
          schema: "synveda.clean-engine.background-controller-shutdown.v1",
        },
        () => process.exit(0),
      );
    } catch {
      process.exit(70);
    }
  });
  process.on("disconnect", () => process.exit(70));
  process.on("SIGINT", () => process.exit(70));
  process.on("SIGTERM", () => process.exit(70));
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : "closed failure";
  process.stderr.write(
    `clean-engine-background-controller: ${message}\n`,
  );
  process.exit(70);
});

#!/usr/bin/env node
import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
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
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
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

function controllerStartProofIdentity(processIdentity, startDecisionSha256) {
  return `${processIdentity}\0${startDecisionSha256}`;
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
    "before_detach_hold_milliseconds",
    "controller_launch_decision",
    "controller_nonce",
    "controller_ready",
    "controller_script_sha256",
    "controller_witness",
    "create_authority",
    "create_authority_sha256",
    "create_intent_sha256",
    "create_slot_sha256",
    "fixture_id",
    "hostagent_config",
    "hostagent_config_sha256",
    "hostagent_source_base64",
    "hostagent_source_sha256",
    "instance_nonce",
    "maximum_lifetime_milliseconds",
    "node_sha256",
    "provider_contract_sha256",
    "provider_start_decision",
    "require_shutdown_during_start",
    "root_owner",
    "root_owner_sha256",
    "schema",
    "working_directory",
  ]);
  if (
    value.schema !== "synveda.clean-engine.background-controller-config.v2" ||
    !Number.isSafeInteger(value.before_detach_hold_milliseconds) ||
    value.before_detach_hold_milliseconds < 0 ||
    value.before_detach_hold_milliseconds > 5_000 ||
    !/^[0-9a-f]{64}$/.test(value.controller_nonce) ||
    !/^[0-9a-f]{64}$/.test(value.controller_script_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.create_authority_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.create_intent_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.create_slot_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.hostagent_config_sha256) ||
    !/^[0-9a-f]{32}$/.test(value.fixture_id) ||
    !/^[0-9a-f]{64}$/.test(value.hostagent_source_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.instance_nonce) ||
    !/^[0-9a-f]{64}$/.test(value.node_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.provider_contract_sha256) ||
    !/^[0-9a-f]{64}$/.test(value.root_owner_sha256) ||
    typeof value.require_shutdown_during_start !== "boolean" ||
    !Number.isSafeInteger(value.maximum_lifetime_milliseconds) ||
    value.maximum_lifetime_milliseconds < 1_000 ||
    value.maximum_lifetime_milliseconds > 30_000 ||
    ![
      value.controller_launch_decision,
      value.controller_ready,
      value.controller_witness,
      value.create_authority,
      value.hostagent_config,
      value.provider_start_decision,
      value.root_owner,
      value.working_directory,
    ].every(
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
  return {
    configSha256: digest(configBytes),
    hostagentSource: hostagentSource.toString("utf8"),
    value,
  };
}

function readPrivateArtifact(path, expectedSha256, label) {
  let descriptor;
  try {
    descriptor = openSync(
      path,
      constants.O_RDONLY | constants.O_NOFOLLOW,
    );
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
      throw new Error(`${label} identity was refused`);
    }
    const artifactBytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameMetadata(before, after) || digest(artifactBytes) !== expectedSha256) {
      throw new Error(`${label} changed`);
    }
    const value = JSON.parse(artifactBytes.toString("utf8"));
    if (!bytes(value).equals(artifactBytes)) {
      throw new Error(`${label} was not canonical`);
    }
    return value;
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function validateStartGate(config, configSha256, processIdentity, expectedSha256) {
  const decision = readPrivateArtifact(
    config.provider_start_decision,
    expectedSha256,
    "provider start decision",
  );
  exactKeys(decision, [
    "controller_witness_sha256",
    "create_authority_sha256",
    "create_intent_sha256",
    "create_slot_sha256",
    "decision",
    "fixture_id",
    "schema",
  ]);
  const witness = readPrivateArtifact(
    config.controller_witness,
    decision.controller_witness_sha256,
    "controller witness",
  );
  exactKeys(witness, [
    "argv_contract",
    "controller_config_sha256",
    "controller_launch_decision_sha256",
    "controller_nonce_sha256",
    "controller_pgid",
    "controller_pid",
    "controller_process_instance_sha256",
    "create_authority_sha256",
    "execution_protocol",
    "fixture_id",
    "hostagent_config_sha256",
    "root_owner_sha256",
    "schema",
    "toolchain_sha256",
  ]);
  const launch = readPrivateArtifact(
    config.controller_launch_decision,
    witness.controller_launch_decision_sha256,
    "controller launch decision",
  );
  exactKeys(launch, [
    "controller_config_sha256",
    "controller_nonce_sha256",
    "create_authority_sha256",
    "decision",
    "fixture_id",
    "hostagent_config_sha256",
    "root_owner_sha256",
    "schema",
    "toolchain_sha256",
  ]);
  const authority = readPrivateArtifact(
    config.create_authority,
    config.create_authority_sha256,
    "create authority",
  );
  exactKeys(authority, [
    "base",
    "create_intent_sha256",
    "create_slot_sequence",
    "create_slot_sha256",
    "evidence_directory",
    "fixture_id",
    "ownership_nonce",
    "provider_contract_sha256",
    "provider_profile",
    "provider_root_path",
    "root_preexisting",
    "schema",
    "source_head_sha256",
    "source_sequence",
    "state_integration",
  ]);
  const owner = readPrivateArtifact(
    config.root_owner,
    config.root_owner_sha256,
    "provider root owner",
  );
  exactKeys(owner, [
    "create_authority_sha256",
    "fixture_id",
    "ownership_nonce",
    "provider_contract_sha256",
    "provider_kind",
    "provider_profile",
    "root_path",
    "schema",
  ]);
  const hostagentConfig = readPrivateArtifact(
    config.hostagent_config,
    config.hostagent_config_sha256,
    "hostagent config",
  );
  exactKeys(hostagentConfig, [
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
    decision.schema !== "synveda.clean-engine.background-provider-start-decision.v1" ||
    decision.fixture_id !== config.fixture_id ||
    decision.decision !== "start" ||
    decision.create_authority_sha256 !== config.create_authority_sha256 ||
    decision.create_intent_sha256 !== config.create_intent_sha256 ||
    decision.create_slot_sha256 !== config.create_slot_sha256 ||
    witness.schema !== "synveda.clean-engine.background-controller-witness.v2" ||
    witness.fixture_id !== config.fixture_id ||
    witness.controller_pid !== process.pid ||
    witness.controller_pgid !== process.pid ||
    witness.controller_process_instance_sha256 !== processIdentity ||
    witness.controller_config_sha256 !== configSha256 ||
    witness.controller_nonce_sha256 !==
      digest(Buffer.from(config.controller_nonce, "ascii")) ||
    witness.create_authority_sha256 !== config.create_authority_sha256 ||
    witness.hostagent_config_sha256 !== config.hostagent_config_sha256 ||
    witness.root_owner_sha256 !== config.root_owner_sha256 ||
    witness.execution_protocol !== "authenticated-ipc-start-shutdown-v2" ||
    launch.schema !==
      "synveda.clean-engine.background-controller-launch-decision.v1" ||
    launch.fixture_id !== config.fixture_id ||
    launch.decision !== "launch-waiting" ||
    launch.controller_config_sha256 !== configSha256 ||
    launch.controller_nonce_sha256 !== witness.controller_nonce_sha256 ||
    launch.create_authority_sha256 !== config.create_authority_sha256 ||
    launch.hostagent_config_sha256 !== config.hostagent_config_sha256 ||
    launch.root_owner_sha256 !== config.root_owner_sha256 ||
    launch.toolchain_sha256 !== witness.toolchain_sha256 ||
    authority.schema !== "synveda.clean-engine.background-create-authority.v1" ||
    authority.fixture_id !== config.fixture_id ||
    authority.create_intent_sha256 !== config.create_intent_sha256 ||
    authority.create_slot_sha256 !== config.create_slot_sha256 ||
    authority.ownership_nonce !== config.instance_nonce ||
    authority.provider_contract_sha256 !== config.provider_contract_sha256 ||
    authority.provider_root_path !== config.working_directory ||
    authority.root_preexisting !== "absent" ||
    authority.state_integration !== "fixture-only" ||
    owner.schema !== "synveda.clean-engine.background-provider-root-owner.v2" ||
    owner.fixture_id !== config.fixture_id ||
    owner.create_authority_sha256 !== config.create_authority_sha256 ||
    owner.ownership_nonce !== config.instance_nonce ||
    owner.provider_contract_sha256 !== config.provider_contract_sha256 ||
    owner.provider_kind !== "controlled-background-fake" ||
    owner.provider_profile !== authority.provider_profile ||
    owner.root_path !== config.working_directory ||
    hostagentConfig.schema !== "synveda.clean-engine.background-hostagent-config.v1" ||
    hostagentConfig.fixture_id !== config.fixture_id ||
    hostagentConfig.instance_nonce !== config.instance_nonce ||
    hostagentConfig.hostagent_script_sha256 !== config.hostagent_source_sha256 ||
    hostagentConfig.node_sha256 !== config.node_sha256 ||
    hostagentConfig.working_directory !== config.working_directory
  ) {
    throw new Error("provider start gate was refused");
  }
  return decision;
}

function waitForHostagentReady(child, config) {
  return new Promise((resolvePromise, rejectPromise) => {
    const finish = (error, value) => {
      clearTimeout(timeout);
      child.off("error", onError);
      child.off("exit", onExit);
      child.off("message", onMessage);
      if (error === undefined) resolvePromise(value);
      else rejectPromise(error);
    };
    const timeout = setTimeout(
      () => finish(new Error("hostagent readiness timed out")),
      8_000,
    );
    const onError = (error) => finish(error);
    const onExit = (status, signal) =>
      finish(new Error(`hostagent exited before readiness: ${status ?? signal}`));
    const onMessage = (message) => {
      try {
        exactKeys(message, ["fixture_id", "pid", "process_instance_sha256", "schema"]);
        if (
          message.schema !== "synveda.clean-engine.background-hostagent-ready.v1" ||
          message.fixture_id !== config.fixture_id ||
          !Number.isSafeInteger(message.pid) ||
          message.pid < 2 ||
          !/^[0-9a-f]{64}$/.test(message.process_instance_sha256)
        ) {
          throw new Error("hostagent readiness was refused");
        }
        finish(undefined, message);
      } catch (error) {
        finish(error);
      }
    };
    child.once("error", onError);
    child.once("exit", onExit);
    child.once("message", onMessage);
  });
}

function detachHostagent(child, config, ready) {
  return new Promise((resolvePromise, rejectPromise) => {
    const challenge = randomBytes(32).toString("hex");
    let acknowledged = false;
    let settled = false;
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      child.off("error", onError);
      child.off("exit", onExit);
      child.off("disconnect", onDisconnect);
      child.off("message", onMessage);
      if (error === undefined) resolvePromise();
      else rejectPromise(error);
    };
    const timeout = setTimeout(
      () => finish(new Error("hostagent detach timed out")),
      8_000,
    );
    const onError = (error) => finish(error);
    const onExit = (status, signal) =>
      finish(new Error(`hostagent exited before detach: ${status ?? signal}`));
    const onDisconnect = () => {
      if (acknowledged) finish();
      else finish(new Error("hostagent disconnected before detach acknowledgement"));
    };
    const onMessage = (message) => {
      try {
        exactKeys(message, [
          "challenge_sha256",
          "fixture_id",
          "process_instance_sha256",
          "proof_sha256",
          "schema",
        ]);
        if (
          message.schema !== "synveda.clean-engine.background-hostagent-detached.v1" ||
          message.fixture_id !== config.fixture_id ||
          message.process_instance_sha256 !== ready.process_instance_sha256 ||
          message.challenge_sha256 !== digest(Buffer.from(challenge, "ascii")) ||
          !proofEquals(
            message.proof_sha256,
            proof(
              config.instance_nonce,
              "hostagent-detached",
              challenge,
              ready.process_instance_sha256,
            ),
          )
        ) {
          throw new Error("hostagent detach acknowledgement was refused");
        }
        acknowledged = true;
      } catch (error) {
        finish(error);
      }
    };
    child.once("error", onError);
    child.once("exit", onExit);
    child.once("disconnect", onDisconnect);
    child.once("message", onMessage);
    child.send({
      action: "detach",
      challenge,
      proof_sha256: proof(
        config.instance_nonce,
        "hostagent-detach",
        challenge,
        ready.process_instance_sha256,
      ),
    });
  });
}

function acknowledgeControllerStart(config, controllerProcessIdentity, message, ready) {
  return new Promise((resolvePromise, rejectPromise) => {
    process.send(
      {
        challenge_sha256: digest(Buffer.from(message.challenge, "ascii")),
        fixture_id: config.fixture_id,
        hostagent_pid: ready.pid,
        hostagent_process_instance_sha256: ready.process_instance_sha256,
        process_instance_sha256: controllerProcessIdentity,
        proof_sha256: proof(
          config.controller_nonce,
          "controller-start-accepted",
          message.challenge,
          controllerStartProofIdentity(
            controllerProcessIdentity,
            message.start_decision_sha256,
          ),
        ),
        schema: "synveda.clean-engine.background-controller-start.v1",
        start_decision_sha256: message.start_decision_sha256,
      },
      (error) => {
        if (error === null || error === undefined) resolvePromise();
        else rejectPromise(error);
      },
    );
  });
}

async function main() {
  const { configSha256, hostagentSource, value: config } = readConfig(
    process.argv[1],
    process.argv[2],
  );
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
  publish(config.controller_ready, {
    controller_environment_keys: Object.keys(process.env).sort(),
    controller_pid: process.pid,
    controller_process_instance_sha256: controllerProcessIdentity,
    controller_script_sha256: digest(Buffer.from(evaluatedSource(), "utf8")),
    fixture_id: config.fixture_id,
    node_sha256: digest(readFileSync(process.execPath)),
    proof_sha256: proof(
      config.controller_nonce,
      "controller-ready",
      config.fixture_id,
      controllerProcessIdentity,
    ),
    schema: "synveda.clean-engine.background-controller-ready.v2",
    working_directory: process.cwd(),
  });
  let child;
  let state = "waiting";
  let startCompletion;
  let releaseShutdownGate;
  let shutdownDuringStartRequested = false;
  const lifetime = setTimeout(
    () => process.exit(70),
    config.maximum_lifetime_milliseconds,
  );
  process.on("message", async (message) => {
    try {
      if (
        message === null ||
        Array.isArray(message) ||
        typeof message !== "object" ||
        !/^[0-9a-f]{64}$/.test(message.challenge)
      ) {
        throw new Error("controller action was refused");
      }
      if (message.action === "start" && state === "waiting") {
        exactKeys(message, [
          "action",
          "challenge",
          "proof_sha256",
          "start_decision_sha256",
        ]);
        if (!/^[0-9a-f]{64}$/.test(message.start_decision_sha256)) {
          throw new Error("controller start decision was refused");
        }
        if (!proofEquals(
          message.proof_sha256,
          proof(
            config.controller_nonce,
            "controller-start",
            message.challenge,
            controllerStartProofIdentity(
              controllerProcessIdentity,
              message.start_decision_sha256,
            ),
          ),
        )) {
          throw new Error("controller start was refused");
        }
        validateStartGate(
          config,
          configSha256,
          controllerProcessIdentity,
          message.start_decision_sha256,
        );
        state = "starting";
        startCompletion = (async () => {
          child = spawn(
            process.execPath,
            [
              "--input-type=module",
              "--eval",
              hostagentSource,
              config.hostagent_config,
              config.hostagent_config_sha256,
            ],
            {
              cwd: config.working_directory,
              detached: true,
              env: process.env,
              stdio: ["ignore", "ignore", "ignore", "ipc"],
            },
          );
          const ready = await waitForHostagentReady(child, config);
          await acknowledgeControllerStart(
            config,
            controllerProcessIdentity,
            message,
            ready,
          );
          if (config.before_detach_hold_milliseconds > 0) {
            await new Promise((resolvePromise) =>
              setTimeout(resolvePromise, config.before_detach_hold_milliseconds),
            );
          }
          if (
            config.require_shutdown_during_start &&
            !shutdownDuringStartRequested
          ) {
            await new Promise((resolvePromise) => {
              releaseShutdownGate = resolvePromise;
            });
          }
          await detachHostagent(child, config, ready);
          child.unref();
          state = "started";
        })();
        await startCompletion;
        return;
      }
      if (
        message.action !== "shutdown" ||
        !new Set(["waiting", "starting", "started"]).has(state)
      ) {
        throw new Error("controller action was refused");
      }
      exactKeys(message, ["action", "challenge", "proof_sha256"]);
      if (!proofEquals(
        message.proof_sha256,
        proof(
          config.controller_nonce,
          "controller-shutdown",
          message.challenge,
          controllerProcessIdentity,
        ),
      )) {
        throw new Error("controller shutdown was refused");
      }
      const startWasPending = state === "starting";
      if (startWasPending) {
        shutdownDuringStartRequested = true;
        if (releaseShutdownGate !== undefined) {
          const release = releaseShutdownGate;
          releaseShutdownGate = undefined;
          release();
        }
        await startCompletion;
      }
      if (!new Set(["waiting", "started"]).has(state)) {
        throw new Error("controller shutdown state was refused");
      }
      state = "stopping";
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
          schema: "synveda.clean-engine.background-controller-shutdown.v2",
          start_was_pending: startWasPending,
        },
        () => process.exit(0),
      );
    } catch {
      process.exit(70);
    }
  });
  const stop = () => {
    if (child?.connected) child.disconnect();
    process.exit(70);
  };
  process.on("disconnect", stop);
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : "closed failure";
  process.stderr.write(
    `clean-engine-background-controller: ${message}\n`,
  );
  process.exit(70);
});

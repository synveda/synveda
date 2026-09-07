import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  publishColimaLiveProviderEffectAttemptFenceForTest,
  publishColimaLiveProviderEffectConclusiveNotCreatedForTest,
  publishColimaLiveProviderEffectPreAttemptForTest,
  recoverColimaLiveProviderEffectForTest,
  resolveColimaLiveProviderEffectMissingDeliveryForTest,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";

function malformedFixtureInput() {
  const error = new Error("fixture input was malformed");
  error.exitStatus = 64;
  return error;
}

function readFixtureInput() {
  let input;
  try {
    const serialized = readFileSync(0, "utf8");
    if (serialized.length === 0) throw malformedFixtureInput();
    input = JSON.parse(serialized);
  } catch {
    throw malformedFixtureInput();
  }
  try {
    input.observationInput.binding_key = Buffer.from(
      input.observationInput.binding_key.data,
    );
  } catch {
    throw malformedFixtureInput();
  }
  return input;
}

const values = process.argv.slice(2);
const [action, repoRoot, stateBase, checkpoint, releasePath] = values;
if (
  values.length !== 5 ||
  !new Set([
    "attempt",
    "collision",
    "conclusive",
    "publish",
    "recover",
    "resolve",
  ]).has(action) ||
  [repoRoot, stateBase, checkpoint, releasePath].some(
    (value) => typeof value !== "string" || value.length === 0,
  )
) {
  process.exitCode = 64;
} else {
  try {
    const input = readFixtureInput();
    const waitAtCheckpoint = (name) => {
      if (action === "collision" && name === "before-marker-link") {
        writeFileSync(
          join(
            input.observationInput.provider_root,
            ".synveda-clean-engine-provider-reservation",
          ),
          "foreign provider effect marker\n",
          { flag: "wx", mode: 0o600 },
        );
      }
      if (name !== checkpoint) return;
      writeFileSync(`${releasePath}.ready-${process.pid}`, "ready\n", {
        flag: "wx",
        mode: 0o600,
      });
      const cell = new Int32Array(new SharedArrayBuffer(4));
      while (!existsSync(releasePath)) Atomics.wait(cell, 0, 0, 10);
    };
    const argumentsValue = {
      ...input,
      repoRoot,
      stateBase,
      testCheckpoint: waitAtCheckpoint,
    };
    if (action === "publish" || action === "collision") {
      publishColimaLiveProviderEffectPreAttemptForTest(argumentsValue);
    } else if (action === "attempt") {
      publishColimaLiveProviderEffectAttemptFenceForTest(argumentsValue);
    } else if (action === "conclusive") {
      publishColimaLiveProviderEffectConclusiveNotCreatedForTest(
        argumentsValue,
      );
    } else if (action === "recover") {
      recoverColimaLiveProviderEffectForTest(argumentsValue);
    } else {
      resolveColimaLiveProviderEffectMissingDeliveryForTest(argumentsValue);
    }
  } catch (error) {
    process.stderr.write(
      `${error?.message ?? "live provider effect fixture failed"}\n`,
    );
    process.exitCode = Number.isSafeInteger(error?.exitStatus)
      ? error.exitStatus
      : 70;
  }
}

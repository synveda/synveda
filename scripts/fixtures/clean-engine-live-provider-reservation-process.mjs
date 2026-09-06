import { existsSync, writeFileSync } from "node:fs";
import {
  publishColimaLiveProviderReservationForTest,
  recoverColimaLiveProviderReservationForTest,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";

const values = process.argv.slice(2);
const [action, repoRoot, stateBase, serialized, checkpoint, releasePath] = values;
if (
  values.length !== 6 ||
  !new Set(["publish", "recover"]).has(action) ||
  [repoRoot, stateBase, serialized, checkpoint, releasePath].some(
    (value) => typeof value !== "string" || value.length === 0,
  )
) {
  process.exitCode = 64;
} else {
  try {
    const input = JSON.parse(serialized);
    input.observationInput.binding_key = Buffer.from(
      input.observationInput.binding_key.data,
    );
    const waitAtCheckpoint = (name) => {
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
    if (action === "publish") {
      publishColimaLiveProviderReservationForTest(argumentsValue);
    } else {
      recoverColimaLiveProviderReservationForTest(argumentsValue);
    }
  } catch (error) {
    process.stderr.write(
      `${error?.message ?? "live provider reservation fixture failed"}\n`,
    );
    process.exitCode = Number.isSafeInteger(error?.exitStatus)
      ? error.exitStatus
      : 70;
  }
}

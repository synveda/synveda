import { existsSync, writeFileSync } from "node:fs";
import {
  publishColimaLiveProviderStartDecisionForTest,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";

const argumentsValue = process.argv.slice(2);
const [
  repoRoot,
  stateBase,
  serialized,
  checkpoint,
  releasePath,
  preStageReleasePath,
] = argumentsValue;
if (
  !new Set([5, 6]).has(argumentsValue.length) ||
  [repoRoot, stateBase, serialized, checkpoint, releasePath].some(
    (value) => typeof value !== "string" || value.length === 0,
  ) ||
  (preStageReleasePath !== undefined &&
    (typeof preStageReleasePath !== "string" ||
      preStageReleasePath.length === 0))
) {
  process.exitCode = 64;
} else {
  try {
    const input = JSON.parse(serialized);
    input.observationInput.binding_key = Buffer.from(
      input.observationInput.binding_key.data,
    );
    const waitAt = (path) => {
      writeFileSync(`${path}.ready-${process.pid}`, "ready\n", {
        flag: "wx",
        mode: 0o600,
      });
      const cell = new Int32Array(new SharedArrayBuffer(4));
      while (!existsSync(path)) Atomics.wait(cell, 0, 0, 10);
    };
    publishColimaLiveProviderStartDecisionForTest({
      ...input,
      repoRoot,
      stateBase,
      testCheckpoint(name) {
        if (
          preStageReleasePath !== undefined &&
          name === "before-slot-stage"
        ) {
          waitAt(preStageReleasePath);
        }
        if (name !== checkpoint) return;
        waitAt(releasePath);
      },
    });
  } catch (error) {
    process.stderr.write(
      `${error?.message ?? "start decision fixture failed"}\n`,
    );
    process.exitCode = Number.isSafeInteger(error?.exitStatus)
      ? error.exitStatus
      : 70;
  }
}

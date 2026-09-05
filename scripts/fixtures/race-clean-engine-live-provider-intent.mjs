import { existsSync, writeFileSync } from "node:fs";
import {
  publishColimaLiveProviderIntentForTest,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, serialized, checkpoint, releasePath] =
  process.argv.slice(2);
if (
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
    publishColimaLiveProviderIntentForTest({
      ...input,
      repoRoot,
      stateBase,
      testCheckpoint(name) {
        if (name !== checkpoint) return;
        writeFileSync(`${releasePath}.ready-${process.pid}`, "ready\n", {
          flag: "wx",
          mode: 0o600,
        });
        const cell = new Int32Array(new SharedArrayBuffer(4));
        while (!existsSync(releasePath)) Atomics.wait(cell, 0, 0, 10);
      },
    });
  } catch (error) {
    process.stderr.write(`${error?.message ?? "intent fixture failed"}\n`);
    process.exitCode = Number.isSafeInteger(error?.exitStatus)
      ? error.exitStatus
      : 70;
  }
}

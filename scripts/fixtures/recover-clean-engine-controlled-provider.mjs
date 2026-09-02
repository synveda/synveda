#!/usr/bin/env node
import { resolve } from "node:path";
import { recoverControlledProviderCreateForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, providerBase, confirmation] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !providerBase?.startsWith("/") ||
  !/^recover:[0-9a-f]{32}:[0-9]{2}:[0-9a-f]{64}$/.test(confirmation ?? "")
) {
  process.exit(64);
}

try {
  await recoverControlledProviderCreateForExecutor({
    confirmation,
    providerBase: resolve(providerBase),
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

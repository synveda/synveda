#!/usr/bin/env node
import { resolve } from "node:path";
import { executeProviderCreateForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";
import { cleanEngineReceiptResult } from "./clean-engine-receipt-fixture.mjs";

const [repoRoot, stateBase, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !new Set(["close-race", "hold", "kill", "publish-race"]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  close_prelink_hold_milliseconds: mode === "close-race" ? 1_000 : 0,
  execute_outcome: "passed",
  execute_result: cleanEngineReceiptResult("provider-create-passed"),
  hold_milliseconds: mode === "kill" ? 30_000 : mode === "hold" ? 1_000 : 0,
  kind: "deterministic-fake-provider-v1",
  prelink_hold_milliseconds: 0,
  publication_hold_milliseconds: mode === "publish-race" ? 10_000 : 0,
  reconcile_hold_milliseconds: 0,
  reconcile_outcome: "passed",
  reconcile_result: cleanEngineReceiptResult("provider-create-passed"),
};

try {
  executeProviderCreateForExecutor({ adapter, repoRoot: resolve(repoRoot), stateBase: resolve(stateBase) });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

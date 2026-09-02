#!/usr/bin/env node
import { resolve } from "node:path";
import { recoverProviderCreateForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";
import { cleanEngineReceiptResult } from "./clean-engine-receipt-fixture.mjs";

const [repoRoot, stateBase, confirmation, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !/^recover:[0-9a-f]{32}:[0-9]{2}:[0-9a-f]{64}$/.test(confirmation ?? "") ||
  !new Set(["close-race", "failed", "hold-failed", "unknown"]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  close_prelink_hold_milliseconds: mode === "close-race" ? 1_000 : 0,
  execute_outcome: "failed",
  execute_result: {},
  hold_milliseconds: 0,
  kind: "deterministic-fake-provider-v1",
  prelink_hold_milliseconds: 0,
  publication_hold_milliseconds: 0,
  reconcile_hold_milliseconds: mode === "hold-failed" ? 30_000 : 0,
  reconcile_outcome: mode === "unknown" ? "unknown" : "failed",
  reconcile_result: mode === "unknown" ? {} : {
    cleanup_required: true,
    collision_resource: "none",
    resource_disposition: "receipt-owned-or-absent",
    safe_code: "child-failed",
  },
};

try {
  recoverProviderCreateForExecutor({
    adapter,
    confirmation,
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

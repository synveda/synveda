#!/usr/bin/env node
import { resolve } from "node:path";
import { executeBackgroundProviderCleanupForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !new Set([
    "crash-after-delete",
    "hold-after-intent",
    "hold-after-plan",
    "hold-after-result",
    "hold-after-retirement",
    "hold-after-settlement",
    "hold-after-slot",
    "hold-before-close-link",
    "inert-before-outer",
    "pass",
    "source-drift-before-plan",
    "stop-after-first",
  ]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  after_claim_hold_milliseconds: 0,
  after_intent_hold_milliseconds:
    mode === "hold-after-intent"
      ? 30_000
      : mode === "source-drift-before-plan"
        ? 1_500
        : 0,
  after_plan_hold_milliseconds: mode === "hold-after-plan" ? 30_000 : 0,
  after_result_hold_milliseconds: mode === "hold-after-result" ? 30_000 : 0,
  after_retirement_hold_milliseconds:
    mode === "hold-after-retirement"
      ? 30_000
      : mode === "inert-before-outer"
        ? 1_500
        : 0,
  after_settlement_hold_milliseconds:
    mode === "hold-after-settlement" ? 30_000 : 0,
  after_slot_hold_milliseconds: mode === "hold-after-slot" ? 30_000 : 0,
  before_close_hold_milliseconds: 0,
  close_prelink_hold_milliseconds:
    mode === "hold-before-close-link" ? 1_500 : 0,
  crash_after_delete_sequence: mode === "crash-after-delete" ? 1 : null,
  crash_after_delete_syscall_sequence: null,
  crash_after_hostagent_settlement: false,
  kind: "controlled-background-provider-cleanup-v1",
  stop_after_sequence: mode === "stop-after-first" ? 0 : null,
};

try {
  await executeBackgroundProviderCleanupForExecutor({
    adapter,
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

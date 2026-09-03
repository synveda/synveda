#!/usr/bin/env node
import { resolve } from "node:path";
import { recoverBackgroundProviderCleanupForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, confirmation, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  typeof confirmation !== "string" ||
  !new Set([
    "hold-after-plan",
    "hold-before-close-link",
    "pass",
    "pause-after-claim",
    "pause-before-retirement",
  ]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  after_claim_hold_milliseconds: mode === "pause-after-claim" ? 1_500 : 0,
  after_intent_hold_milliseconds: 0,
  after_plan_hold_milliseconds:
    mode === "hold-after-plan"
      ? 30_000
      : mode === "pause-before-retirement"
        ? 1_500
        : 0,
  after_result_hold_milliseconds: 0,
  after_retirement_hold_milliseconds: 0,
  after_settlement_hold_milliseconds: 0,
  after_slot_hold_milliseconds: 0,
  before_close_hold_milliseconds: 0,
  close_prelink_hold_milliseconds:
    mode === "hold-before-close-link" ? 1_500 : 0,
  crash_after_delete_sequence: null,
  crash_after_delete_syscall_sequence: null,
  crash_after_hostagent_settlement: false,
  kind: "controlled-background-provider-cleanup-v1",
  stop_after_sequence: null,
};

try {
  await recoverBackgroundProviderCleanupForExecutor({
    adapter,
    confirmation,
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

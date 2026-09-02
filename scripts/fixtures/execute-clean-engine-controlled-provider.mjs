#!/usr/bin/env node
import { resolve } from "node:path";
import { executeControlledProviderCreateForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, providerBase, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !providerBase?.startsWith("/") ||
  !new Set([
    "fail",
    "hang",
    "hold-after-decision",
    "hold-after-effect-mirror",
    "hold-after-intent",
    "hold-after-outcome-publish",
    "hold-after-provider-identity",
    "hold-after-root-plan",
    "hold-orphan-after-decision",
    "hold-passed-close",
    "hold-root-collision-close",
    "hold-after-settlement",
    "hold-before-decision",
    "hold-before-root",
    "hold-before-root-mirror",
    "hold-before-witness",
    "orphan",
    "pass",
    "race-failed-close",
    "race-passed-close",
    "race-before-decision",
    "race-after-decision",
    "race-root",
    "kill-hang-after-decision",
  ]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  after_decision_hold_milliseconds:
    new Set([
      "hold-after-decision",
      "hold-orphan-after-decision",
      "kill-hang-after-decision",
    ]).has(mode) ? 30_000 : mode === "race-after-decision" ? 500 : 0,
  after_effect_mirror_hold_milliseconds:
    mode === "hold-after-effect-mirror" ? 30_000 : 0,
  after_intent_hold_milliseconds: mode === "hold-after-intent" ? 30_000 : 0,
  after_outcome_publish_hold_milliseconds:
    mode === "hold-after-outcome-publish" ? 1_000 : 0,
  after_provider_identity_hold_milliseconds:
    mode === "hold-after-provider-identity" ? 30_000 : 0,
  after_root_plan_hold_milliseconds: mode === "hold-after-root-plan" ? 30_000 : 0,
  after_settlement_hold_milliseconds: mode === "hold-after-settlement" ? 30_000 : 0,
  before_decision_hold_milliseconds:
    mode === "hold-before-decision" ? 30_000 : mode === "race-before-decision" ? 500 : 0,
  before_root_creation_hold_milliseconds:
    mode === "hold-before-root"
      ? 30_000
      : new Set(["race-root", "hold-root-collision-close"]).has(mode)
        ? 500
        : 0,
  before_root_mirror_hold_milliseconds: mode === "hold-before-root-mirror" ? 30_000 : 0,
  before_witness_hold_milliseconds: mode === "hold-before-witness" ? 30_000 : 0,
  child_scenario:
    mode === "kill-hang-after-decision"
      ? "hang"
      : mode === "hold-orphan-after-decision"
        ? "orphan"
      : mode === "race-failed-close"
        ? "fail"
        : new Set(["fail", "hang", "orphan"]).has(mode)
          ? mode
          : "pass",
  close_prelink_hold_milliseconds:
    new Set(["hold-passed-close", "hold-root-collision-close"]).has(mode)
      ? 30_000
      : new Set(["race-failed-close", "race-passed-close"]).has(mode)
        ? 1_000
        : 0,
  deadline_milliseconds:
    new Set(["hang", "hold-after-outcome-publish"]).has(mode) ? 300 : 5_000,
  gate_delivery: "correct",
  kill_grace_milliseconds: 1_000,
  kind: "controlled-fake-provider-v1",
  term_grace_milliseconds: mode === "kill-hang-after-decision" ? 1_000 : 100,
};

try {
  await executeControlledProviderCreateForExecutor({
    adapter,
    providerBase: resolve(providerBase),
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

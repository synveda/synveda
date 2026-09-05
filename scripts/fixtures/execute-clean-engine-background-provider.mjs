#!/usr/bin/env node
import { resolve } from "node:path";
import { executeBackgroundProviderCreateForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [repoRoot, stateBase, providerBase, mode] = process.argv.slice(2);
if (
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !providerBase?.startsWith("/") ||
  !new Set([
    "hold-after-authority",
    "hold-after-evidence",
    "hold-after-intent",
    "hold-after-result",
    "hold-after-settlement",
    "foreign-collision-hold-after-settlement",
    "foreign-collision-after-authority",
    "pass",
    "source-drift-before-close",
    "source-drift-before-pass",
    "source-drift-before-root",
    "source-drift-before-start",
  ]).has(mode)
) {
  process.exit(64);
}

const adapter = {
  after_authority_hold_milliseconds:
    mode === "hold-after-authority"
      ? 30_000
      : mode === "foreign-collision-after-authority"
        ? 1_500
        : mode === "source-drift-before-root"
          ? 1_500
          : 0,
  after_evidence_hold_milliseconds: mode === "hold-after-evidence" ? 30_000 : 0,
  after_intent_hold_milliseconds:
    mode === "hold-after-intent"
      ? 30_000
      : mode === "foreign-collision-hold-after-settlement"
        ? 1_500
        : 0,
  after_result_hold_milliseconds:
    mode === "hold-after-result"
      ? 30_000
      : mode === "source-drift-before-close"
        ? 1_500
        : 0,
  after_settlement_hold_milliseconds:
    new Set([
      "foreign-collision-after-authority",
      "foreign-collision-hold-after-settlement",
      "hold-after-settlement",
    ]).has(mode)
      ? 30_000
      : mode === "source-drift-before-pass"
        ? 1_500
        : 0,
  before_detach_hold_milliseconds: 0,
  before_identity_probe_hold_milliseconds: 0,
  before_start_decision_hold_milliseconds:
    mode === "source-drift-before-start" ? 1_500 : 0,
  before_start_hold_milliseconds: 0,
  kind: "controlled-background-provider-v1",
  maximum_lifetime_milliseconds: 5_000,
};

try {
  await executeBackgroundProviderCreateForExecutor({
    adapter,
    providerBase: resolve(providerBase),
    repoRoot: resolve(repoRoot),
    stateBase: resolve(stateBase),
  });
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exit(error.exitStatus ?? 70);
}

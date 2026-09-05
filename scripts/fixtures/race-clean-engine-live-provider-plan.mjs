#!/usr/bin/env node
import {
  executeProviderCreateForExecutor,
  recordLiveProviderOperationPlanForTest,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";
import { cleanEngineReceiptResult } from "./clean-engine-receipt-fixture.mjs";

const [action, repoRoot, stateBase, serializedPlan, holdValue = "0"] =
  process.argv.slice(2);
const operationPlan = JSON.parse(serializedPlan);

try {
  if (action === "plan") {
    recordLiveProviderOperationPlanForTest({
      holdMilliseconds: Number(holdValue),
      operationPlan,
      repoRoot,
      stateBase,
    });
  } else if (action === "fake") {
    executeProviderCreateForExecutor({
      adapter: {
        close_prelink_hold_milliseconds: 0,
        execute_outcome: "passed",
        execute_result: cleanEngineReceiptResult("provider-create-passed"),
        hold_milliseconds: 0,
        kind: "deterministic-fake-provider-v1",
        prelink_hold_milliseconds: 0,
        publication_hold_milliseconds: 0,
        reconcile_hold_milliseconds: 0,
        reconcile_outcome: "passed",
        reconcile_result: cleanEngineReceiptResult("provider-create-passed"),
      },
      repoRoot,
      stateBase,
    });
  } else {
    process.exitCode = 64;
  }
} catch (error) {
  process.exitCode = Number.isSafeInteger(error?.exitStatus)
    ? error.exitStatus
    : 70;
}

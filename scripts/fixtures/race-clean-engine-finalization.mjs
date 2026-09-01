#!/usr/bin/env node
import {
  appendReceiptForExecutor,
  finalizeEnvironmentForExecutor,
} from "../../deploy/compose/scripts/clean-engine-state.mjs";

const [action, repoRoot, stateBase] = process.argv.slice(2);
if (
  process.argv.length !== 5 ||
  !new Set(["fail", "finalize"]).has(action) ||
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/")
) {
  process.stderr.write("finalization-race-fixture: invalid arguments\n");
  process.exit(64);
}

try {
  if (action === "finalize") {
    finalizeEnvironmentForExecutor({ repoRoot, stateBase });
  } else {
    appendReceiptForExecutor({
      phase: "execution-failed",
      repoRoot,
      result: {
        cleanup_required: true,
        collision_resource: "none",
        resource_disposition: "receipt-owned-or-absent",
        safe_code: "evidence-refused",
      },
      stateBase,
    });
  }
  process.stdout.write(`finalization-race-fixture: ${action} durable\n`);
} catch {
  process.stderr.write(`finalization-race-fixture: ${action} was not published\n`);
  process.exit(73);
}

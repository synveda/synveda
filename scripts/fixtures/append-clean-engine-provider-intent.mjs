#!/usr/bin/env node
import { appendReceiptForExecutor } from "../../deploy/compose/scripts/clean-engine-state.mjs";
import { cleanEngineReceiptResult } from "./clean-engine-receipt-fixture.mjs";

const [repoRoot, stateBase, fixtureId, digestDigit] = process.argv.slice(2);
if (
  process.argv.length !== 6 ||
  !repoRoot?.startsWith("/") ||
  !stateBase?.startsWith("/") ||
  !/^[0-9a-f]{32}$/.test(fixtureId ?? "") ||
  !/^[2-9a-f]$/.test(digestDigit ?? "")
) {
  process.stderr.write("append-fixture: invalid arguments\n");
  process.exit(64);
}

try {
  const result = cleanEngineReceiptResult("provider-create-intent", fixtureId);
  result.provider_contract_sha256 = digestDigit.repeat(64);
  appendReceiptForExecutor({
    phase: "provider-create-intent",
    repoRoot,
    result,
    stateBase,
  });
  process.stdout.write("append-fixture: provider intent durable\n");
} catch {
  process.stderr.write("append-fixture: provider intent was not published\n");
  process.exit(73);
}

#!/usr/bin/env node
import { chromium } from "playwright-core";

import { BrowserContractError } from "./console-login-contract.mjs";
import { runProductAcceptance } from "./product-demo-runner.mjs";

try {
  if (
    process.argv.length !== 3 ||
    !["seed", "verify"].includes(process.argv[2])
  ) throw new BrowserContractError("configuration");
  const phase = process.argv[2];
  await runProductAcceptance({ chromium, phase });
  process.stdout.write(`compose-product: public-API ${phase} passed\n`);
} catch (error) {
  const stage =
    error instanceof BrowserContractError ? error.stage : "unexpected";
  const cause = error?.cause?.message;
  const detail =
    stage === "retry-review-rerun" &&
    typeof cause === "string" &&
    /^(workspace-(stable-fields|description|revision|updated-at)|resource-[a-z_]+)$/.test(
      cause,
    )
      ? `/${cause}`
      : "";
  process.stderr.write(`compose-product: ${stage}${detail} failed\n`);
  process.exitCode = 78;
}

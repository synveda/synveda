#!/usr/bin/env node
import { chromium } from "playwright-core";

import { BrowserContractError } from "./console-login-contract.mjs";
import { runBrowserAcceptance } from "./console-login-runner.mjs";

try {
  await runBrowserAcceptance({ chromium });
  process.stdout.write("compose-browser: PKCE login, administrator admission and logout passed\n");
} catch (error) {
  const stage = error instanceof BrowserContractError ? error.stage : "unexpected";
  process.stderr.write(`compose-browser: ${stage} failed\n`);
  process.exitCode = 78;
}

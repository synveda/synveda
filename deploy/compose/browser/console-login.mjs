#!/usr/bin/env node
import {
  BrowserContractError,
  validateSettings,
  validateTenantId,
} from "./console-login-contract.mjs";
import { runBrowserAcceptance } from "./console-login-runner.mjs";

try {
  const environment = Object.freeze({
    SYNVEDA_BROWSER_APP_URL: process.env.SYNVEDA_BROWSER_APP_URL,
    SYNVEDA_BROWSER_ISSUER: process.env.SYNVEDA_BROWSER_ISSUER,
    SYNVEDA_BOOTSTRAP_TENANT_ID: process.env.SYNVEDA_BOOTSTRAP_TENANT_ID,
  });
  validateSettings(
    environment.SYNVEDA_BROWSER_APP_URL,
    environment.SYNVEDA_BROWSER_ISSUER,
  );
  validateTenantId(environment.SYNVEDA_BOOTSTRAP_TENANT_ID);
  const { chromium } = await import("playwright-core");
  await runBrowserAcceptance({ chromium, environment });
  process.stdout.write("compose-browser: PKCE login, administrator admission and logout passed\n");
} catch (error) {
  const stage = error instanceof BrowserContractError ? error.stage : "unexpected";
  process.stderr.write(`compose-browser: ${stage} failed\n`);
  process.exitCode = 78;
}

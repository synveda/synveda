#!/usr/bin/env node
// CPR-45 / OPS-11: real browser login, optional product screenshot, and logout.
import { chromium } from "playwright-core";
import { readFileSync } from "node:fs";
import { runBrowserAcceptance } from "./console-login-runner.mjs";
import { readDemoPassword } from "./console-login-contract.mjs";

try {
  const executablePath = process.env.SYNVEDA_TEST_BROWSER_EXECUTABLE;
  await runBrowserAcceptance({
    chromium: { launch: (options) => chromium.launch({ ...options, ...(executablePath ? { executablePath } : {}) }) },
    passwordFile: process.env.SYNVEDA_TEST_PASSWORD_FILE,
    readPassword: process.env.SYNVEDA_TEST_KUBERNETES_SECRETS
      ? () => Buffer.from(JSON.parse(readFileSync(process.env.SYNVEDA_TEST_KUBERNETES_SECRETS, "utf8")).items.find((s) => s.metadata.name === "synveda-evaluation-accounts").stringData.admin, "ascii")
      : readDemoPassword,
    afterLogin: async ({ page, settings }) => {
      page.setDefaultTimeout(30_000);
      if (process.env.SYNVEDA_TEST_SAMPLE === "1") {
        await page.goto(`${settings.appOrigin}/console/learnings`);
        await page.getByRole("heading", { name: "New Learnings", exact: true }).waitFor();
        await page.locator(".learning-batches").waitFor({ state: "visible" });
      }
      if (process.env.SYNVEDA_TEST_SCREENSHOT) {
        await page.screenshot({ path: process.env.SYNVEDA_TEST_SCREENSHOT, fullPage: true });
      }
    },
  });
  console.log("PASS real browser: exact issuer, PKCE, console cookie, tenant admission, API operation, console logout and post-logout 401");
} catch (error) {
  console.error(`evaluation browser failed: ${error.stage ?? "unexpected"}`);
  process.exitCode = 1;
}

import {
  BrowserContractError,
  allowedRequest,
  readDemoPassword,
  validateAuthorizationUrl,
  validateCallbackUrl,
  validateSettings,
} from "./console-login-contract.mjs";

const PASSWORD_FILE = "/run/secrets/keycloak_demo_admin_password";
const USERNAME = "synveda-demo-admin";
const TIMEOUT = 60_000;
const FETCH_TIMEOUT = 5_000;
const CLEANUP_TIMEOUT = 5_000;

async function atStage(stage, operation) {
  try {
    return await operation();
  } catch (error) {
    if (error instanceof BrowserContractError) throw error;
    throw new BrowserContractError(stage);
  }
}

async function boundedEvaluation(page, operation, argument, timeout) {
  let deadline;
  try {
    return await Promise.race([
      page.evaluate(operation, argument),
      new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error("evaluation deadline")), timeout);
      }),
    ]);
  } finally {
    clearTimeout(deadline);
  }
}

async function boundedCleanup(operation) {
  let deadline;
  try {
    await Promise.race([
      Promise.resolve().then(operation),
      new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error("cleanup deadline")), CLEANUP_TIMEOUT);
      }),
    ]);
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(deadline);
  }
}

export async function runBrowserAcceptance({
  chromium,
  environment = process.env,
  passwordFile = PASSWORD_FILE,
  readPassword = readDemoPassword,
  timeout = TIMEOUT,
} = {}) {
  let browser;
  let context;
  let page;
  let password;
  let authorizationState;
  let callbackSeen = false;
  let completed = false;
  let primaryError;
  let routeError;
  const pendingRoutes = new Set();
  const requireCleanRoutes = () => {
    if (routeError !== undefined) throw routeError;
  };

  try {
    const settings = validateSettings(
      environment.SYNVEDA_BROWSER_APP_URL,
      environment.SYNVEDA_BROWSER_ISSUER,
    );
    if (typeof chromium?.launch !== "function" || typeof readPassword !== "function") {
      throw new BrowserContractError("configuration");
    }
    password = readPassword(passwordFile);
    if (!Buffer.isBuffer(password)) throw new BrowserContractError("password-file");

    browser = await atStage("browser-launch", () =>
      chromium.launch({
        headless: true,
        chromiumSandbox: true,
        args: [
          "--disable-background-networking",
          "--disable-breakpad",
          "--disable-component-update",
          "--disable-default-apps",
          "--disable-domain-reliability",
          "--disable-sync",
          "--metrics-recording-only",
          "--no-first-run",
        ],
        timeout,
      }),
    );
    context = await atStage("browser-launch", () =>
      browser.newContext({
        acceptDownloads: false,
        locale: "en-GB",
        serviceWorkers: "block",
        timezoneId: "UTC",
        viewport: { width: 1280, height: 800 },
      }),
    );
    page = await atStage("browser-launch", () => context.newPage());
    await atStage("browser-launch", () =>
      page.route("**/*", (route) => {
        const operation = (async () => {
          const raw = route.request().url();
          try {
            if (!allowedRequest(raw, settings)) {
              throw new BrowserContractError("network-boundary");
            }
            const parsed = new URL(raw);
            if (
              parsed.origin === settings.issuerOrigin &&
              parsed.pathname === settings.authorizationPath
            ) {
              if (authorizationState !== undefined) {
                throw new BrowserContractError("authorization-request");
              }
              authorizationState = validateAuthorizationUrl(raw, settings);
            }
            if (
              parsed.origin === settings.appOrigin &&
              parsed.pathname === "/auth/callback"
            ) {
              if (callbackSeen) throw new BrowserContractError("callback");
              validateCallbackUrl(raw, settings, authorizationState);
              callbackSeen = true;
            }
            await route.continue();
          } catch (error) {
            const failure = error instanceof BrowserContractError
              ? error
              : new BrowserContractError("network-boundary");
            routeError ??= failure;
            try {
              await route.abort("blockedbyclient");
            } catch {}
            throw failure;
          }
        })();
        pendingRoutes.add(operation);
        operation.then(
          () => pendingRoutes.delete(operation),
          () => pendingRoutes.delete(operation),
        );
        return operation;
      }),
    );

    await atStage("signed-out-console", async () => {
      await page.goto(`${settings.appOrigin}/console/`, {
        timeout,
        waitUntil: "domcontentloaded",
      });
      await page.getByRole("heading", { name: "Sign in", exact: true }).waitFor({
        state: "visible",
        timeout,
      });
    });
    requireCleanRoutes();

    await atStage("authorization-request", async () => {
      await page.getByRole("link", { name: "Sign in", exact: true }).click({ timeout });
      await page.locator("#username").waitFor({ state: "visible", timeout });
      if (authorizationState === undefined) throw new BrowserContractError("authorization-request");
    });
    requireCleanRoutes();

    await atStage("credential-submit", async () => {
      await page.locator("#username").fill(USERNAME);
      await page.locator("#password").fill(password.toString("ascii"));
      password.fill(0);
      await page.locator("#kc-login").click({ timeout });
    });
    requireCleanRoutes();

    await atStage("callback", async () => {
      await page.waitForURL(`${settings.appOrigin}/console/`, {
        timeout,
        waitUntil: "domcontentloaded",
      });
      if (!callbackSeen) throw new BrowserContractError("callback");
      await page.getByRole("button", { name: "Sign out", exact: true }).waitFor({
        state: "visible",
        timeout,
      });
    });
    requireCleanRoutes();

    const admission = await atStage("administrator-admission", () =>
      boundedEvaluation(page, async (fetchTimeout) => {
        const controller = new AbortController();
        const deadline = setTimeout(() => controller.abort(), fetchTimeout);
        try {
          const response = await fetch("/v1/whoami?capabilities=true", {
            credentials: "same-origin",
            signal: controller.signal,
          });
          if (response.status !== 200) return { authenticated: false, administrator: false };
          const value = await response.json();
          return {
            authenticated: true,
            administrator:
              Array.isArray(value?.capabilities?.role_keys) &&
              value.capabilities.role_keys.includes("administrator"),
          };
        } finally {
          clearTimeout(deadline);
        }
      }, FETCH_TIMEOUT, Math.min(timeout, FETCH_TIMEOUT + 1_000)),
    );
    if (admission.authenticated !== true || admission.administrator !== true) {
      throw new BrowserContractError("administrator-admission");
    }
    requireCleanRoutes();

    await atStage("session-cleanup", async () => {
      await page.getByRole("button", { name: "Sign out", exact: true }).click({ timeout });
      await page.getByRole("heading", { name: "Sign in", exact: true }).waitFor({
        state: "visible",
        timeout,
      });
      const signedOut = await boundedEvaluation(page, async (fetchTimeout) => {
        const controller = new AbortController();
        const deadline = setTimeout(() => controller.abort(), fetchTimeout);
        try {
          const response = await fetch("/v1/whoami", {
            credentials: "same-origin",
            signal: controller.signal,
          });
          return response.status === 401;
        } finally {
          clearTimeout(deadline);
        }
      }, FETCH_TIMEOUT, Math.min(timeout, FETCH_TIMEOUT + 1_000));
      if (signedOut !== true) throw new BrowserContractError("session-cleanup");
    });
    requireCleanRoutes();

    completed = true;
  } catch (error) {
    primaryError = error;
  } finally {
    if (password !== undefined) password.fill(0);
    let cleanupFailed = false;
    if (typeof page?.unrouteAll === "function") {
      cleanupFailed = !(await boundedCleanup(
        () => page.unrouteAll({ behavior: "ignoreErrors" }),
      )) || cleanupFailed;
    }
    if (page !== undefined) {
      cleanupFailed = !(await boundedCleanup(
        () => page.close({ runBeforeUnload: false }),
      )) || cleanupFailed;
    }
    if (context !== undefined) {
      cleanupFailed = !(await boundedCleanup(() => context.close())) || cleanupFailed;
    }
    if (browser !== undefined) {
      cleanupFailed = !(await boundedCleanup(() => browser.close())) || cleanupFailed;
    }
    if (pendingRoutes.size > 0) {
      cleanupFailed = !(await boundedCleanup(
        () => Promise.allSettled([...pendingRoutes]),
      )) || cleanupFailed;
    }
    if (primaryError === undefined && routeError !== undefined) primaryError = routeError;
    if (primaryError === undefined && cleanupFailed) {
      primaryError = new BrowserContractError("browser-cleanup");
    }
  }
  if (primaryError !== undefined) throw primaryError;
  return completed;
}

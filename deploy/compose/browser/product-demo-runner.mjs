import { spawn } from "node:child_process";

import {
  BrowserContractError,
  readDemoPassword,
  validateAuthorizationUrl,
  validateCallbackUrl,
  validateSettings,
} from "./console-login-contract.mjs";
import {
  allowedCliRequest,
  validateCliHandoffUrl,
  validateCliLoginUrl,
  validateDemoReceipt,
  validateDemoStatus,
} from "./product-demo-contract.mjs";

const CLI = "/usr/local/bin/synveda";
const ADMIN_PASSWORD_FILE = "/run/secrets/keycloak_demo_admin_password";
const MEMBER_PASSWORD_FILE = "/run/secrets/keycloak_demo_member_password";
const MAX_LOGIN_OUTPUT = 16 * 1024;
const MAX_DEMO_OUTPUT = 2 * 1024 * 1024;
const LOGIN_TIMEOUT = 90_000;
const DEMO_TIMEOUT = 10 * 60_000;
const CLEANUP_TIMEOUT = 5_000;
const CHILD_STOP_TIMEOUT = 1_000;

function failure(stage) {
  return new BrowserContractError(stage);
}

async function boundedCleanup(operation, timeout = CLEANUP_TIMEOUT) {
  let deadline;
  try {
    await Promise.race([
      Promise.resolve().then(operation),
      new Promise((_, reject) => {
        deadline = setTimeout(
          () => reject(new Error("cleanup deadline")),
          timeout,
        );
      }),
    ]);
    return true;
  } catch {
    return false;
  } finally {
    clearTimeout(deadline);
  }
}

async function stopChild(process) {
  process.child.kill("SIGTERM");
  if (await boundedCleanup(() => process.result, CHILD_STOP_TIMEOUT)) return;
  process.child.kill("SIGKILL");
  await boundedCleanup(() => process.result, CHILD_STOP_TIMEOUT);
}

function childEnvironment(environment) {
  return {
    HOME: environment.HOME ?? "/tmp/browser-home",
    LANG: environment.LANG ?? "C.UTF-8",
    LC_ALL: environment.LC_ALL ?? "C.UTF-8",
    PATH: "/usr/local/bin:/usr/bin:/bin",
    SYNVEDA_INSECURE_DEVELOPMENT_HTTP:
      environment.SYNVEDA_INSECURE_DEVELOPMENT_HTTP ?? "false",
    TZ: "UTC",
    XDG_CONFIG_HOME:
      environment.XDG_CONFIG_HOME ?? "/tmp/browser-home/config",
    XDG_STATE_HOME: environment.XDG_STATE_HOME ?? "/tmp/browser-home/state",
  };
}

function launchChild(args, environment, outputLimit, stage, spawnProcess = spawn) {
  let child;
  try {
    child = spawnProcess(CLI, args, {
      env: childEnvironment(environment),
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch {
    throw failure(stage);
  }
  if (
    child === null ||
    typeof child !== "object" ||
    typeof child.once !== "function" ||
    typeof child.kill !== "function" ||
    typeof child.stdout?.on !== "function" ||
    typeof child.stderr?.on !== "function"
  ) throw failure(stage);

  let stdout = Buffer.alloc(0);
  let stderr = Buffer.alloc(0);
  let overLimit = false;
  let signalOutputLimit;
  const outputLimitReached = new Promise((resolve) => {
    signalOutputLimit = resolve;
  });
  const append = (current, chunk) => {
    const value = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    if (current.length + value.length > outputLimit) {
      overLimit = true;
      signalOutputLimit();
      child.kill("SIGTERM");
      return current;
    }
    return Buffer.concat([current, value]);
  };
  child.stdout.on("data", (chunk) => {
    stdout = append(stdout, chunk);
  });
  child.stderr.on("data", (chunk) => {
    stderr = append(stderr, chunk);
  });

  const result = new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    child.once("error", () => finish({ error: true }));
    child.once("close", (code, signal) =>
      finish({ code, signal, stdout, stderr }),
    );
  });
  return {
    child,
    get overLimit() {
      return overLimit;
    },
    get stderr() {
      return stderr;
    },
    outputLimitReached,
    result,
  };
}

async function within(promise, timeout, stage, onTimeout = () => {}) {
  let deadline;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        deadline = setTimeout(() => {
          onTimeout();
          reject(failure(stage));
        }, timeout);
      }),
    ]);
  } finally {
    clearTimeout(deadline);
  }
}

async function waitForLoginUrl(process, settings, timeout) {
  const started = Date.now();
  while (Date.now() - started < timeout) {
    if (process.overLimit) throw failure("cli-login");
    const lines = process.stderr.toString("utf8").split(/\r?\n/);
    for (const line of lines) {
      const candidate = line.trim();
      if (candidate.startsWith(`${settings.appOrigin}/auth/login?`)) {
        return validateCliLoginUrl(candidate, settings);
      }
    }
    const completed = await Promise.race([
      process.result.then(() => true),
      new Promise((resolve) => setTimeout(() => resolve(false), 20)),
    ]);
    if (completed) throw failure("cli-login");
  }
  throw failure("cli-login");
}

async function driveCliBrowser({
  chromium,
  settings,
  login,
  username,
  password,
  timeout,
}) {
  let browser;
  let context;
  let page;
  let authorizationState;
  let callbackSeen = false;
  let handoffSeen = false;
  let routeError;
  let primaryError;
  let requestListener;
  const pendingRoutes = new Set();
  const inspectedRequests = new WeakMap();
  try {
    browser = await chromium.launch({
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
    });
    context = await browser.newContext({
      acceptDownloads: false,
      locale: "en-GB",
      serviceWorkers: "block",
      timezoneId: "UTC",
      viewport: { width: 1280, height: 800 },
    });
    page = await context.newPage();
    const inspectRequest = (request) => {
      const prior = inspectedRequests.get(request);
      if (prior === true) return;
      if (prior instanceof BrowserContractError) throw prior;
      try {
        const raw = request.url();
        if (!allowedCliRequest(raw, settings, login)) {
          throw failure("network-boundary");
        }
        const parsed = new URL(raw);
        if (
          parsed.origin === settings.issuerOrigin &&
          parsed.pathname === settings.authorizationPath
        ) {
          if (authorizationState !== undefined) {
            throw failure("authorization-request");
          }
          authorizationState = validateAuthorizationUrl(raw, settings);
        }
        if (
          parsed.origin === settings.appOrigin &&
          parsed.pathname === "/auth/callback"
        ) {
          if (callbackSeen) throw failure("callback");
          validateCallbackUrl(raw, settings, authorizationState);
          callbackSeen = true;
        }
        if (
          parsed.origin === login.redirectOrigin &&
          `${parsed.origin}${parsed.pathname}` === login.redirect
        ) {
          if (handoffSeen) throw failure("cli-handoff");
          validateCliHandoffUrl(raw, login);
          handoffSeen = true;
        }
        inspectedRequests.set(request, true);
      } catch (error) {
        const closed =
          error instanceof BrowserContractError
            ? error
            : failure("network-boundary");
        inspectedRequests.set(request, closed);
        routeError ??= closed;
        throw closed;
      }
    };
    // Redirect targets bypass Playwright route handlers. Observe every request
    // to validate those hops, while routing aborts any routable request outside
    // the closed app, issuer and exact loopback-handoff contract.
    requestListener = (request) => {
      try {
        inspectRequest(request);
      } catch {}
    };
    page.on("request", requestListener);
    await page.route("**/*", (route) => {
      const operation = (async () => {
        try {
          inspectRequest(route.request());
          await route.continue();
        } catch (error) {
          const closed =
            error instanceof BrowserContractError
              ? error
              : failure("network-boundary");
          routeError ??= closed;
          try {
            await route.abort("blockedbyclient");
          } catch {}
          throw closed;
        }
      })();
      pendingRoutes.add(operation);
      operation.then(
        () => pendingRoutes.delete(operation),
        () => pendingRoutes.delete(operation),
      );
      return operation;
    });

    await page.goto(login.startUrl, {
      timeout,
      waitUntil: "domcontentloaded",
    });
    await page.locator("#username").waitFor({ state: "visible", timeout });
    await page.locator("#username").fill(username);
    await page.locator("#password").fill(password.toString("ascii"));
    password.fill(0);
    await page.locator("#kc-login").click({ timeout });
    await page.waitForURL(
      (url) =>
        url.origin === login.redirectOrigin &&
        `${url.origin}${url.pathname}` === login.redirect,
      { timeout, waitUntil: "domcontentloaded" },
    );
    if (authorizationState === undefined) throw failure("authorization-request");
    if (!callbackSeen) throw failure("callback");
    if (!handoffSeen) throw failure("cli-handoff");
    if (routeError !== undefined) throw routeError;
  } catch (error) {
    primaryError =
      error instanceof BrowserContractError ? error : failure("cli-browser");
  } finally {
    password.fill(0);
    let cleanupFailed = false;
    if (requestListener !== undefined && typeof page?.off === "function") {
      try {
        page.off("request", requestListener);
      } catch {
        cleanupFailed = true;
      }
    }
    if (typeof page?.unrouteAll === "function") {
      cleanupFailed =
        !(await boundedCleanup(() =>
          page.unrouteAll({ behavior: "ignoreErrors" }),
        )) || cleanupFailed;
    }
    if (page !== undefined) {
      cleanupFailed =
        !(await boundedCleanup(() =>
          page.close({ runBeforeUnload: false }),
        )) || cleanupFailed;
    }
    if (context !== undefined) {
      cleanupFailed =
        !(await boundedCleanup(() => context.close())) || cleanupFailed;
    }
    if (browser !== undefined) {
      cleanupFailed =
        !(await boundedCleanup(() => browser.close())) || cleanupFailed;
    }
    if (pendingRoutes.size > 0) {
      cleanupFailed =
        !(await boundedCleanup(() => Promise.allSettled([...pendingRoutes]))) ||
        cleanupFailed;
    }
    if (routeError !== undefined) primaryError = routeError;
    if (primaryError === undefined && cleanupFailed) {
      primaryError = failure("browser-cleanup");
    }
  }
  if (primaryError !== undefined) throw primaryError;
}

async function loginIdentity({
  chromium,
  environment,
  settings,
  profile,
  username,
  password,
  timeout,
  spawnProcess,
}) {
  const process = launchChild(
    [
      "login",
      "--gateway",
      settings.appOrigin,
      "--issuer",
      settings.issuer,
      "--profile",
      profile,
      "--no-browser",
    ],
    environment,
    MAX_LOGIN_OUTPUT,
    "cli-login",
    spawnProcess,
  );
  let completed = false;
  try {
    const login = await waitForLoginUrl(
      process,
      settings,
      Math.min(timeout, 10_000),
    );
    await driveCliBrowser({
      chromium,
      settings,
      login,
      username,
      password,
      timeout,
    });
    const result = await within(
      process.result,
      timeout,
      "cli-login",
      () => process.child.kill("SIGTERM"),
    );
    if (
      process.overLimit ||
      result.error === true ||
      result.code !== 0 ||
      result.signal !== null
    ) throw failure("cli-login");
    completed = true;
  } finally {
    password.fill(0);
    if (!completed) {
      await stopChild(process);
    }
  }
}

async function runCli(args, environment, timeout, spawnProcess) {
  const process = launchChild(
    args,
    environment,
    MAX_DEMO_OUTPUT,
    "product-demo",
    spawnProcess,
  );
  let completed = false;
  try {
    const result = await within(
      Promise.race([
        process.result,
        process.outputLimitReached.then(() => {
          throw failure("product-demo");
        }),
      ]),
      timeout,
      "product-demo",
    );
    if (
      process.overLimit ||
      result.error === true ||
      result.code !== 0 ||
      result.signal !== null
    ) throw failure("product-demo");
    completed = true;
    try {
      return JSON.parse(result.stdout.toString("utf8"));
    } catch {
      throw failure("product-demo");
    }
  } finally {
    if (!completed) await stopChild(process);
  }
}

export async function runProductAcceptance({
  chromium,
  phase = "seed",
  environment = process.env,
  readPassword = readDemoPassword,
  login = loginIdentity,
  command = runCli,
  spawnProcess = spawn,
  loginTimeout = LOGIN_TIMEOUT,
  demoTimeout = DEMO_TIMEOUT,
} = {}) {
  const settings = validateSettings(
    environment.SYNVEDA_BROWSER_APP_URL,
    environment.SYNVEDA_BROWSER_ISSUER,
  );
  if (
    !["seed", "verify"].includes(phase) ||
    typeof chromium?.launch !== "function" ||
    typeof readPassword !== "function"
  ) throw failure("configuration");
  let adminPassword;
  let memberPassword;
  try {
    adminPassword = readPassword(ADMIN_PASSWORD_FILE);
    if (phase === "seed") {
      memberPassword = readPassword(MEMBER_PASSWORD_FILE);
    }
    if (
      !Buffer.isBuffer(adminPassword) ||
      (phase === "seed" && !Buffer.isBuffer(memberPassword))
    ) {
      throw failure("password-file");
    }
    await login({
      chromium,
      environment,
      settings,
      profile: "alice",
      username: "synveda-demo-admin",
      password: adminPassword,
      timeout: loginTimeout,
      spawnProcess,
    });
    if (phase === "seed") {
      await login({
        chromium,
        environment,
        settings,
        profile: "bob",
        username: "synveda-demo-member",
        password: memberPassword,
        timeout: loginTimeout,
        spawnProcess,
      });
    }
  } finally {
    adminPassword?.fill?.(0);
    memberPassword?.fill?.(0);
  }
  if (phase === "seed") {
    const args = [
      "demo",
      "start",
      "--profile",
      "team",
      "--credentials",
      "alice",
      "--bob-credentials",
      "bob",
      "--json",
    ];
    validateDemoReceipt(
      await command(args, environment, demoTimeout, spawnProcess),
    );
    validateDemoReceipt(
      await command(args, environment, demoTimeout, spawnProcess),
    );
  }
  validateDemoStatus(
    await command(
      ["demo", "status", "--credentials", "alice", "--json"],
      environment,
      demoTimeout,
      spawnProcess,
    ),
  );
  return true;
}

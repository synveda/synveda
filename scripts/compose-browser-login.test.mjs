import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmodSync,
  closeSync,
  fstatSync,
  linkSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  openSync,
  readSync,
  realpathSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

import {
  BrowserContractError,
  allowedRequest,
  readDemoPassword,
  validateAuthorizationUrl,
  validateCallbackUrl,
  validateSettings,
} from "../deploy/compose/browser/console-login-contract.mjs";
import { runBrowserAcceptance } from "../deploy/compose/browser/console-login-runner.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DRIVER = join(ROOT, "deploy/compose/browser/console-login.mjs");
const RUNNER = join(ROOT, "deploy/compose/browser/console-login-runner.mjs");
const DOCKERFILE = join(ROOT, "deploy/compose/browser/Dockerfile");
const MAKEFILE = join(ROOT, "Makefile");
const SECCOMP = join(ROOT, "deploy/compose/browser/seccomp_profile.json");
const SECCOMP_NOTICE = join(ROOT, "deploy/compose/browser/seccomp_profile.NOTICE");
const PLAYWRIGHT_LICENSE = join(ROOT, "deploy/compose/browser/PLAYWRIGHT-LICENSE");
const PLAYWRIGHT_NOTICE = join(ROOT, "deploy/compose/browser/PLAYWRIGHT-NOTICE");
const SECCOMP_CHECKER = join(ROOT, "deploy/compose/scripts/check-browser-seccomp.mjs");
const STATE = "s".repeat(43);
const NONCE = "n".repeat(43);
const CHALLENGE = "c".repeat(43);
const SESSION_STATE = "t".repeat(24);
const SETTINGS = validateSettings(
  "http://app.synveda.test:8080",
  "http://auth.synveda.test:8080/realms/synveda",
);

function authorizationUrl(overrides = {}, extra = []) {
  const url = new URL(
    "http://auth.synveda.test:8080/realms/synveda/protocol/openid-connect/auth",
  );
  const values = {
    response_type: "code",
    client_id: "synveda",
    redirect_uri: "http://app.synveda.test:8080/auth/callback",
    scope: "openid profile email",
    state: STATE,
    nonce: NONCE,
    code_challenge: CHALLENGE,
    code_challenge_method: "S256",
    ...overrides,
  };
  for (const [name, value] of Object.entries(values)) {
    if (value !== undefined) url.searchParams.append(name, value);
  }
  for (const [name, value] of extra) url.searchParams.append(name, value);
  return url.href;
}

function callbackUrl(overrides = {}, extra = []) {
  const url = new URL("http://app.synveda.test:8080/auth/callback");
  const values = {
    code: "opaque-code",
    iss: SETTINGS.issuer,
    session_state: SESSION_STATE,
    state: STATE,
    ...overrides,
  };
  for (const [name, value] of Object.entries(values)) {
    if (value !== undefined) url.searchParams.append(name, value);
  }
  for (const [name, value] of extra) url.searchParams.append(name, value);
  return url.href;
}

function contractFailure(operation, stage) {
  assert.throws(operation, (error) => {
    assert.ok(error instanceof BrowserContractError);
    assert.equal(error.stage, stage);
    return true;
  });
}

test("the browser contract accepts only the exact PKCE S256 request", () => {
  assert.equal(validateAuthorizationUrl(authorizationUrl(), SETTINGS), STATE);
  for (const [name, value] of [
    ["response_type", "token"],
    ["client_id", "other"],
    ["redirect_uri", "http://app.synveda.test:8080/other"],
    ["scope", "openid email"],
    ["state", "short"],
    ["nonce", "short"],
    ["code_challenge", "short"],
    ["code_challenge_method", "plain"],
  ]) {
    contractFailure(
      () => validateAuthorizationUrl(authorizationUrl({ [name]: value }), SETTINGS),
      "authorization-request",
    );
  }
  for (const name of [
    "response_type",
    "client_id",
    "redirect_uri",
    "scope",
    "state",
    "nonce",
    "code_challenge",
    "code_challenge_method",
  ]) {
    contractFailure(
      () => validateAuthorizationUrl(authorizationUrl({ [name]: undefined }), SETTINGS),
      "authorization-request",
    );
  }
  for (const extra of [
    ["state", STATE],
    ["code_verifier", "private-verifier"],
    ["client_secret", "private-secret"],
    ["access_token", "private-token"],
    ["unexpected", "value"],
  ]) {
    contractFailure(
      () => validateAuthorizationUrl(authorizationUrl({}, [extra]), SETTINGS),
      "authorization-request",
    );
  }
  const wrongOrigin = authorizationUrl().replace(
    "auth.synveda.test",
    "other.synveda.test",
  );
  contractFailure(
    () => validateAuthorizationUrl(wrongOrigin, SETTINGS),
    "authorization-request",
  );
});

test("callback, settings and request-origin contracts are exact", () => {
  const callback = callbackUrl();
  assert.equal(validateCallbackUrl(callback, SETTINGS, STATE), true);
  for (const mutant of [
    callback.replace("/auth/callback", "/other"),
    callback.replace("app.synveda.test", "other.synveda.test"),
    callbackUrl({ state: "short" }),
    callbackUrl({ code: undefined }),
    callbackUrl({ state: undefined }),
    callbackUrl({ iss: "http://other.synveda.test:8080/realms/synveda" }),
    callbackUrl({ iss: `${SETTINGS.issuer}?query=1` }),
    callbackUrl({ session_state: "short" }),
    callbackUrl({ session_state: "u".repeat(23) }),
    callbackUrl({ session_state: "u".repeat(25) }),
    callbackUrl({ session_state: `${"u".repeat(23)}=` }),
    callbackUrl({ iss: undefined }),
    callbackUrl({ session_state: undefined }),
    callbackUrl({ code: "x".repeat(4097) }),
    `${callback}&state=${STATE}`,
    `${callback}&iss=${encodeURIComponent(SETTINGS.issuer)}`,
    `${callback}&session_state=${SESSION_STATE}`,
    `${callback}&error=denied`,
    callbackUrl({ state: "q".repeat(43) }),
    callback.replace("http://", "http://user@"),
  ]) contractFailure(() => validateCallbackUrl(mutant, SETTINGS, STATE), "callback");
  contractFailure(() => validateCallbackUrl(callback, SETTINGS, undefined), "callback");

  assert.equal(allowedRequest("http://app.synveda.test:8080/console/", SETTINGS), true);
  assert.equal(
    allowedRequest("http://auth.synveda.test:8080/resources/style.css", SETTINGS),
    true,
  );
  assert.equal(
    allowedRequest(
      "http://auth.synveda.test:8080/realms/synveda/login-actions/authenticate?session_code=opaque",
      SETTINGS,
    ),
    true,
  );
  for (const refused of [
    "https://app.synveda.test:8080/console/",
    "http://other.synveda.test:8080/",
    "http://user@app.synveda.test:8080/console/",
    "http://user:password@auth.synveda.test:8080/realms/synveda",
    "http://app.synveda.test:8080/console/#private-fragment",
    "http://auth.synveda.test:8080/realms/synveda/protocol/openid-connect/userinfo",
    "http://auth.synveda.test:8080/realms/synveda/account/",
    "http://auth.synveda.test:8080/realms/other/login-actions/authenticate",
    "data:text/plain,private",
    "not-a-url",
  ]) assert.equal(allowedRequest(refused, SETTINGS), false);

  for (const [app, issuer] of [
    ["http://app.synveda.test", "http://auth.synveda.test/realms/synveda"],
    ["http://app.synveda.test:8080/path", "http://auth.synveda.test:8080/realms/synveda"],
    ["http://app.synveda.test:8080", "https://auth.synveda.test:8080/realms/synveda"],
    ["http://same.synveda.test:8080", "http://same.synveda.test:8080/realms/synveda"],
    ["https://app.example:443", "https://auth.example:443/other"],
    ["https://user@app.example", "https://auth.example/realms/synveda"],
  ]) contractFailure(() => validateSettings(app, issuer), "configuration");
});

test("the demo password reader is bounded, no-follow and ownership strict", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-browser-secret-"));
  try {
    const valid = join(scratch, "valid");
    writeFileSync(valid, `${"a".repeat(64)}\n`, { mode: 0o600 });
    chmodSync(valid, 0o600);
    const password = readDemoPassword(valid);
    assert.equal(password.toString("ascii"), "a".repeat(64));
    password.fill(0);

    const badMode = join(scratch, "bad-mode");
    writeFileSync(badMode, `${"b".repeat(64)}\n`, { mode: 0o644 });
    chmodSync(badMode, 0o644);
    contractFailure(() => readDemoPassword(badMode), "password-file");

    const specialModeFilesystem = Object.freeze({
      closeSync,
      fstatSync,
      lstatSync: (path) => {
        const actual = lstatSync(path);
        return statRecord(actual, { mode: actual.mode | 0o4000 });
      },
      openSync,
      readSync,
    });
    contractFailure(
      () => readDemoPassword(valid, specialModeFilesystem),
      "password-file",
    );

    const malformed = join(scratch, "malformed");
    writeFileSync(malformed, `${"z".repeat(64)}\n`, { mode: 0o600 });
    contractFailure(() => readDemoPassword(malformed), "password-file");

    const oversized = join(scratch, "oversized");
    writeFileSync(oversized, `${"c".repeat(65)}\n`, { mode: 0o600 });
    contractFailure(() => readDemoPassword(oversized), "password-file");

    const linked = join(scratch, "linked");
    linkSync(valid, linked);
    contractFailure(() => readDemoPassword(valid), "password-file");

    const symlink = join(scratch, "symlink");
    symlinkSync(malformed, symlink);
    contractFailure(() => readDemoPassword(symlink), "password-file");

    const directory = join(scratch, "directory");
    mkdirSync(directory, { mode: 0o600 });
    contractFailure(() => readDemoPassword(directory), "password-file");
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

function statRecord(stat, overrides = {}) {
  return {
    dev: stat.dev,
    ino: stat.ino,
    mode: stat.mode,
    uid: stat.uid,
    nlink: stat.nlink,
    size: stat.size,
    mtimeMs: stat.mtimeMs,
    ctimeMs: stat.ctimeMs,
    isFile: () => stat.isFile(),
    isSymbolicLink: () => stat.isSymbolicLink(),
    ...overrides,
  };
}

test("the demo password reader revalidates and zeroes its opened descriptor", () => {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-browser-fd-secret-"));
  try {
    const path = join(scratch, "password");
    writeFileSync(path, `${"d".repeat(64)}\n`, { mode: 0o600 });
    chmodSync(path, 0o600);

    for (const mutation of ["opened-mode", "after-link", "after-time"]) {
      let calls = 0;
      let raw;
      let closed = 0;
      const filesystem = Object.freeze({
        lstatSync,
        openSync,
        fstatSync: (descriptor) => {
          calls += 1;
          const actual = fstatSync(descriptor);
          if (mutation === "opened-mode" && calls === 1) {
            return statRecord(actual, { mode: (actual.mode & ~0o7777) | 0o644 });
          }
          if (mutation === "after-link" && calls === 2) {
            return statRecord(actual, { nlink: 2 });
          }
          if (mutation === "after-time" && calls === 2) {
            return statRecord(actual, { ctimeMs: actual.ctimeMs + 1 });
          }
          return actual;
        },
        readSync: (descriptor, buffer, offset, length, position) => {
          raw = buffer;
          return readSync(descriptor, buffer, offset, length, position);
        },
        closeSync: (descriptor) => {
          closed += 1;
          closeSync(descriptor);
        },
      });
      contractFailure(() => readDemoPassword(path, filesystem), "password-file");
      assert.equal(closed, 1);
      if (raw !== undefined) assert.ok(raw.every((value) => value === 0));
    }

    let growthBuffer;
    const growthFilesystem = Object.freeze({
      closeSync,
      fstatSync,
      lstatSync,
      openSync,
      readSync: (_descriptor, buffer) => {
        growthBuffer = buffer;
        buffer.fill(0x61);
        return buffer.length;
      },
    });
    contractFailure(() => readDemoPassword(path, growthFilesystem), "password-file");
    assert.equal(growthBuffer.length, 66);
    assert.ok(growthBuffer.every((value) => value === 0));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the one-shot image and driver forbid capture and TLS bypass surfaces", () => {
  const dockerfile = readFileSync(DOCKERFILE, "utf8");
  const driver = `${readFileSync(DRIVER, "utf8")}\n${readFileSync(RUNNER, "utf8")}`;
  const makefile = readFileSync(MAKEFILE, "utf8");
  assert.match(
    dockerfile,
    /^FROM mcr\.microsoft\.com\/playwright:v1\.62\.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e$/m,
  );
  assert.match(dockerfile, /^USER 65532:65532$/m);
  assert.match(dockerfile, /^RUN \/usr\/local\/bin\/assert-build-proxy-closed$/m);
  assert.match(dockerfile, /console-login-runner\.mjs/);
  assert.match(
    dockerfile,
    /COPY --chmod=0444 deploy\/compose\/browser\/PLAYWRIGHT-LICENSE deploy\/compose\/browser\/PLAYWRIGHT-NOTICE deploy\/compose\/browser\/seccomp_profile\.NOTICE \/usr\/share\/licenses\/synveda-browser-acceptance\//,
  );
  for (const forbidden of [
    /\.screenshot\s*\(/,
    /\.content\s*\(/,
    /\.tracing\b/,
    /recordHar/i,
    /recordVideo/i,
    /storageState\s*[:(]/,
    /ignoreHTTPSErrors/,
    /--no-sandbox/,
    /page\.on\s*\(\s*["']console/,
    /docker\.sock/,
  ]) assert.doesNotMatch(driver, forbidden);
  assert.match(driver, /chromiumSandbox: true/);
  assert.match(driver, /acceptDownloads: false/);
  assert.match(driver, /serviceWorkers: "block"/);
  assert.match(
    makefile,
    /^compose-browser-acceptance:\n\tSYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance deploy\/compose\/scripts\/compose\.sh up --initial-assets absent$/m,
  );
});

test("the vendored Playwright sandbox profile is exact, default-deny and licensed", () => {
  const bytes = readFileSync(SECCOMP);
  assert.equal(
    createHash("sha256").update(bytes).digest("hex"),
    "cc3e61cabda6bbc1e53e54d27ba4d55a9d3be829b6dd1a596f4a7b31b1cc7849",
  );
  const profile = JSON.parse(bytes.toString("utf8"));
  assert.equal(profile.defaultAction, "SCMP_ACT_ERRNO");
  assert.ok(profile.archMap.some(({ architecture }) => architecture === "SCMP_ARCH_X86_64"));
  assert.ok(profile.archMap.some(({ architecture }) => architecture === "SCMP_ARCH_AARCH64"));
  assert.deepEqual(profile.syscalls[0], {
    comment: "Allow create user namespaces",
    names: ["clone", "setns", "unshare"],
    action: "SCMP_ACT_ALLOW",
    args: [],
    includes: {},
    excludes: {},
  });
  const notice = readFileSync(SECCOMP_NOTICE, "utf8");
  assert.match(notice, /v1\.62\.1/);
  assert.match(notice, /26a9e470a7b3c7822084b09fb7f13902c5f37b51/);
  assert.match(notice, /Modifications: none/);
  const license = readFileSync(PLAYWRIGHT_LICENSE);
  assert.equal(license.length, 11399);
  assert.equal(
    createHash("sha256").update(license).digest("hex"),
    "7fab1461b41970ff376f1c9303a637076bfaaeb71cd12dd3a1c44aaf59a1a2b9",
  );
  assert.match(license.toString("utf8"), /Apache License\s+Version 2\.0/);
  assert.equal(
    readFileSync(PLAYWRIGHT_NOTICE, "utf8"),
    [
      "Playwright",
      "Copyright (c) Microsoft Corporation",
      "",
      "This software contains code derived from the Puppeteer project (https://github.com/puppeteer/puppeteer),",
      "available under the Apache 2.0 license (https://github.com/puppeteer/puppeteer/blob/master/LICENSE).",
      "",
    ].join("\n"),
  );

  const accepted = spawnSync(process.execPath, [SECCOMP_CHECKER, "--profile", SECCOMP], {
    cwd: ROOT,
    encoding: "utf8",
  });
  assert.equal(accepted.status, 0, accepted.stderr);
  const scratch = mkdtempSync(join(tmpdir(), "synveda-browser-seccomp-"));
  try {
    const mutant = join(scratch, "seccomp_profile.json");
    writeFileSync(
      mutant,
      bytes.toString("utf8").replace('"unshare"', '"unsharz"'),
      { mode: 0o600 },
    );
    const refused = spawnSync(
      process.execPath,
      [SECCOMP_CHECKER, "--profile", realpathSync(mutant)],
      { cwd: ROOT, encoding: "utf8" },
    );
    assert.equal(refused.status, 78, refused.stderr);
    assert.match(refused.stderr, /profile digest was refused/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

function fakeBrowserFlow({
  authorizationCount = 1,
  callbackCount = 1,
  foreign = false,
  invalidCallback = false,
  backgroundForeign = false,
  evaluationHang = false,
  cleanupFailure = false,
} = {}) {
  let routeHandler;
  let evaluation = 0;
  const evaluatedSources = [];
  const closed = { browser: 0, context: 0, page: 0 };
  const cleanup = { unrouted: 0 };
  const routes = { aborted: 0, continued: 0 };

  async function dispatch(raw) {
    const route = {
      request: () => ({ url: () => raw }),
      abort: async () => {
        routes.aborted += 1;
      },
      continue: async () => {
        routes.continued += 1;
      },
    };
    await routeHandler(route);
  }

  const page = {
    route: async (_pattern, handler) => {
      routeHandler = handler;
    },
    goto: async () => {},
    getByRole: (role, options) => ({
      waitFor: async () => {},
      click: async () => {
        if (role === "link" && options.name === "Sign in") {
          for (let index = 0; index < authorizationCount; index += 1) {
            await dispatch(foreign ? "http://foreign.invalid/" : authorizationUrl());
          }
          if (backgroundForeign) {
            await dispatch("http://foreign.invalid/background").catch(() => {});
          }
        }
      },
    }),
    locator: (selector) => ({
      waitFor: async () => {},
      fill: async () => {},
      click: async () => {
        if (selector === "#kc-login") {
          for (let index = 0; index < callbackCount; index += 1) {
            await dispatch(
              invalidCallback
                ? callbackUrl({ iss: "http://foreign.invalid/realms/synveda" })
                : callbackUrl(),
            );
          }
        }
      },
    }),
    waitForURL: async () => {},
    unrouteAll: async () => {
      cleanup.unrouted += 1;
    },
    evaluate: async (operation) => {
      evaluatedSources.push(operation.toString());
      if (evaluationHang) return new Promise(() => {});
      evaluation += 1;
      return evaluation === 1
        ? { authenticated: true, administrator: true }
        : true;
    },
    close: async () => {
      closed.page += 1;
      if (cleanupFailure) throw new Error("private cleanup detail");
    },
  };
  const context = {
    newPage: async () => page,
    close: async () => {
      closed.context += 1;
    },
  };
  const browser = {
    newContext: async () => context,
    close: async () => {
      closed.browser += 1;
    },
  };
  return {
    chromium: { launch: async () => browser },
    closed,
    cleanup,
    routes,
    evaluatedSources,
  };
}

function runInjected(flow, password, timeout = 60_000) {
  return runBrowserAcceptance({
    chromium: flow.chromium,
    environment: {
      SYNVEDA_BROWSER_APP_URL: "http://app.synveda.test:8080",
      SYNVEDA_BROWSER_ISSUER: "http://auth.synveda.test:8080/realms/synveda",
    },
    readPassword: () => password,
    timeout,
  });
}

test("the injected browser flow correlates login and exports only bounded aggregates", async () => {
  const flow = fakeBrowserFlow();
  const password = Buffer.from("a".repeat(64));
  assert.equal(await runInjected(flow, password), true);
  assert.deepEqual(flow.closed, { browser: 1, context: 1, page: 1 });
  assert.deepEqual(flow.cleanup, { unrouted: 1 });
  assert.deepEqual(flow.routes, { aborted: 0, continued: 2 });
  assert.ok(password.every((value) => value === 0));
  assert.equal(flow.evaluatedSources.length, 2);
  for (const source of flow.evaluatedSources) {
    assert.match(source, /new AbortController\(\)/);
    assert.match(source, /setTimeout\(\(\) => controller\.abort\(\), fetchTimeout\)/);
    assert.match(source, /signal: controller\.signal/);
    assert.match(source, /clearTimeout\(deadline\)/);
  }
  assert.doesNotMatch(flow.evaluatedSources[0], /return\s+(?:response|value)(?:\.|;)/);
  assert.match(flow.evaluatedSources[1], /return response\.status === 401/);
});

test("the injected browser flow refuses missing, duplicate and foreign redirects", async () => {
  for (const [options, stage] of [
    [{ authorizationCount: 0 }, "authorization-request"],
    [{ authorizationCount: 2 }, "authorization-request"],
    [{ callbackCount: 0 }, "callback"],
    [{ callbackCount: 2 }, "callback"],
    [{ foreign: true }, "network-boundary"],
    [{ backgroundForeign: true }, "network-boundary"],
    [{ invalidCallback: true }, "callback"],
  ]) {
    const flow = fakeBrowserFlow(options);
    const password = Buffer.from("b".repeat(64));
    await assert.rejects(runInjected(flow, password), (error) => {
      assert.ok(error instanceof BrowserContractError);
      assert.equal(error.stage, stage);
      return true;
    });
    assert.deepEqual(flow.closed, { browser: 1, context: 1, page: 1 });
    assert.deepEqual(flow.cleanup, { unrouted: 1 });
    assert.ok(password.every((value) => value === 0));
    if (
      options.foreign === true ||
      options.backgroundForeign === true ||
      options.invalidCallback === true
    ) {
      assert.equal(flow.routes.aborted, 1);
      assert.equal(
        flow.routes.continued,
        options.invalidCallback === true || options.backgroundForeign === true ? 1 : 0,
      );
    }
  }
});

test("browser evaluation and cleanup failures are bounded and content-free", async () => {
  for (const [options, timeout, stage] of [
    [{ evaluationHang: true }, 10, "administrator-admission"],
    [{ cleanupFailure: true }, 60_000, "browser-cleanup"],
  ]) {
    const flow = fakeBrowserFlow(options);
    const password = Buffer.from("e".repeat(64));
    await assert.rejects(runInjected(flow, password, timeout), (error) => {
      assert.ok(error instanceof BrowserContractError);
      assert.equal(error.stage, stage);
      assert.ok(!error.message.includes("private cleanup detail"));
      return true;
    });
    assert.deepEqual(flow.closed, { browser: 1, context: 1, page: 1 });
    assert.deepEqual(flow.cleanup, { unrouted: 1 });
    assert.ok(password.every((value) => value === 0));
  }
});

test("entrypoint configuration failures disclose no supplied value", () => {
  const sentinel = "private-browser-configuration-sentinel";
  const result = spawnSync(process.execPath, [DRIVER], {
    cwd: ROOT,
    encoding: "utf8",
    env: {
      PATH: process.env.PATH ?? "/usr/bin:/bin",
      SYNVEDA_BROWSER_APP_URL: `http://${sentinel}`,
      SYNVEDA_BROWSER_ISSUER: `http://${sentinel}/other`,
    },
  });
  assert.equal(result.status, 78, result.stderr);
  assert.equal(result.stdout, "");
  assert.equal(result.stderr, "compose-browser: configuration failed\n");
  assert.ok(!`${result.stdout}${result.stderr}`.includes(sentinel));
});

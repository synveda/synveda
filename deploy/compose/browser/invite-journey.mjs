#!/usr/bin/env node
// CPR-5/CPR-8 two-person invitation journey through the real proxy, console
// and OIDC provider. The administrator mints a link in People; a recipient
// with no grant in that workspace opens exactly that link signed out, signs in
// in a separate browser context, accepts explicitly and then reads what was
// granted. Output is content-free: one stage name on failure, never a token,
// password, identifier or page text.
import { pathToFileURL } from "node:url";

import {
  BrowserContractError,
  allowedRequest,
  readDemoPassword,
  validateSettings,
  validateTenantId,
} from "./console-login-contract.mjs";

const ADMIN = Object.freeze({
  username: "synveda-demo-admin",
  passwordFile: "/run/secrets/keycloak_demo_admin_password",
});
const RECIPIENT = Object.freeze({
  username: "synveda-demo-viewer",
  passwordFile: "/run/secrets/keycloak_demo_viewer_password",
});
const TIMEOUT = 60_000;
const TOKEN_SHAPE = /synveda_invite_v1\.[0-9a-f-]{36}\.[A-Za-z0-9_-]+/;

async function stage(name, operation) {
  try {
    return await operation();
  } catch (error) {
    if (error instanceof BrowserContractError) throw error;
    throw new BrowserContractError(name);
  }
}

function requireTrue(value, name) {
  if (value !== true) throw new BrowserContractError(name);
}

/**
 * One isolated browser context: its own cookies, so the identity provider's
 * single-sign-on session of one person can never sign in the other.
 */
async function openContext(browser, settings, observed) {
  const context = await browser.newContext({
    acceptDownloads: false,
    locale: "en-GB",
    serviceWorkers: "block",
    timezoneId: "UTC",
    viewport: { width: 1280, height: 800 },
  });
  // A navigation can report its fragment, but no request carries one: the
  // boundary is checked on what actually goes over the wire. Request events
  // also see redirect hops, which routing does not. Both are context-wide, so
  // a link opened in a new tab is covered too.
  const wire = (raw) => {
    const url = new URL(raw);
    url.hash = "";
    return url.href;
  };
  context.on("request", (request) => {
    observed.push({ url: wire(request.url()), method: request.method() });
  });
  await context.route("**/*", async (route) => {
    const raw = wire(route.request().url());
    if (!allowedRequest(raw, settings)) {
      await route.abort("blockedbyclient");
      return;
    }
    await route.continue();
  });
  const page = await context.newPage();
  return { context, page };
}

async function signIn(page, person, timeout, signInName) {
  const password = readDemoPassword(person.passwordFile);
  try {
    await page.getByRole("link", { name: signInName, exact: true }).click({ timeout });
    await page.locator("#username").waitFor({ state: "visible", timeout });
    await page.locator("#username").fill(person.username);
    await page.locator("#password").fill(password.toString("ascii"));
    await page.locator("#kc-login").click({ timeout });
  } finally {
    password.fill(0);
  }
}

async function json(page, path, init = {}) {
  return page.evaluate(
    async ({ path, init }) => {
      const response = await fetch(path, {
        credentials: "same-origin",
        headers: { accept: "application/json", ...(init.headers ?? {}) },
        ...init,
      });
      const text = await response.text();
      return { status: response.status, body: text.length > 0 ? JSON.parse(text) : null };
    },
    { path, init },
  );
}

async function create(page, path, body, key) {
  return json(page, path, {
    method: "POST",
    headers: { "content-type": "application/json", "idempotency-key": key },
    body: JSON.stringify(body),
  });
}

export async function runInviteJourney({ chromium, environment = process.env, timeout = TIMEOUT }) {
  const settings = validateSettings(
    environment.SYNVEDA_BROWSER_APP_URL,
    environment.SYNVEDA_BROWSER_ISSUER,
  );
  validateTenantId(environment.SYNVEDA_BOOTSTRAP_TENANT_ID);
  const observed = [];
  const browser = await stage("browser-launch", () =>
    chromium.launch({ headless: true, chromiumSandbox: true, timeout }),
  );
  try {
    // ── The administrator: a workspace, a project, and a link from People ──
    const admin = await stage("browser-launch", () => openContext(browser, settings, observed));
    await stage("admin-sign-in", async () => {
      await admin.page.goto(`${settings.appOrigin}/console/`, { timeout });
      await signIn(admin.page, ADMIN, timeout, "Sign in");
      await admin.page.waitForURL(`${settings.appOrigin}/console/**`, { timeout });
    });
    const suffix = Date.now().toString(36);
    const workspace = await stage("admin-workspace", async () => {
      const created = await create(
        admin.page,
        "/v1/workspaces",
        { slug: `invite-${suffix}`, display_name: `Invite journey ${suffix}` },
        `invite-journey-ws-${suffix}`,
      );
      requireTrue(created.status === 201, "admin-workspace");
      const project = await create(
        admin.page,
        `/v1/workspaces/${created.body.id}/projects`,
        { slug: `granted-${suffix}`, display_name: `Granted ${suffix}` },
        `invite-journey-project-${suffix}`,
      );
      requireTrue(project.status === 201, "admin-workspace");
      return { ...created.body, project: project.body };
    });
    const sharedLink = await stage("people-invite", async () => {
      await admin.page.goto(`${settings.appOrigin}/console/people`, { timeout });
      await admin.page.getByRole("combobox").first().selectOption(workspace.id);
      await admin.page.getByRole("button", { name: "Create pending invitation" }).click({ timeout });
      const link = admin.page.locator(".banner .mono.breakable").first();
      await link.waitFor({ state: "visible", timeout });
      return (await link.textContent())?.trim() ?? "";
    });
    await stage("shared-link-shape", async () => {
      const url = new URL(sharedLink);
      requireTrue(url.origin === settings.appOrigin, "shared-link-shape");
      requireTrue(url.pathname === "/console/invite" && url.search === "", "shared-link-shape");
      requireTrue(TOKEN_SHAPE.test(new URLSearchParams(url.hash.slice(1)).get("token") ?? ""), "shared-link-shape");
    });
    const token = new URLSearchParams(new URL(sharedLink).hash.slice(1)).get("token");
    // An outstanding link in the redemption operation's shape, minted the way
    // earlier versions did and kept for the opened-but-pending check below.
    const legacy = await stage("legacy-link", async () => {
      const created = await create(
        admin.page,
        `/v1/workspaces/${workspace.id}/invites`,
        { role: "viewer" },
        `invite-journey-legacy-${suffix}`,
      );
      requireTrue(created.status === 201, "legacy-link");
      return { url: created.body.accept_url, id: created.body.invite.id };
    });
    const inventoryBefore = await stage("admin-inventory", () => json(admin.page, "/v1/workspaces"));

    // ── The recipient: open exactly the shared link, signed out ──
    const recipient = await stage("browser-launch", () => openContext(browser, settings, observed));
    await stage("recipient-legacy-landing", async () => {
      await recipient.page.goto(legacy.url, { timeout });
      await recipient.page.waitForURL(`${settings.appOrigin}/console/invite`, { timeout });
      await recipient.page
        .getByRole("heading", { name: "You have been invited to Synveda", exact: true })
        .waitFor({ state: "visible", timeout });
      requireTrue(!recipient.page.url().includes("synveda_invite"), "recipient-legacy-landing");
      await recipient.page.getByRole("button", { name: "Cancel invitation" }).click({ timeout });
    });
    // The shared link, opened the way a recipient clicks it: a fresh tab.
    const tab = await stage("recipient-landing", () => recipient.context.newPage());
    await stage("recipient-landing", async () => {
      await tab.goto(sharedLink, { timeout });
      await tab
        .getByRole("heading", { name: "You have been invited to Synveda", exact: true })
        .waitFor({ state: "visible", timeout });
      // The token left the address bar before anything else ran.
      requireTrue(tab.url() === `${settings.appOrigin}/console/invite`, "recipient-landing");
      await tab.reload({ timeout });
      await tab
        .getByRole("heading", { name: "You have been invited to Synveda", exact: true })
        .waitFor({ state: "visible", timeout });
    });
    await stage("pending-after-open", async () => {
      const listed = await json(admin.page, `/v1/workspaces/${workspace.id}/invites`);
      requireTrue(listed.status === 200, "pending-after-open");
      requireTrue(listed.body.invites.every((invite) => invite.status === "pending"), "pending-after-open");
    });
    await stage("recipient-sign-in", async () => {
      await signIn(tab, RECIPIENT, timeout, "Sign in to continue");
      await tab.waitForURL(`${settings.appOrigin}/console/invite`, { timeout });
      await tab.getByText("You will accept as").waitFor({ state: "visible", timeout });
    });
    const before = await stage("recipient-before", () => json(tab, "/v1/me"));
    requireTrue(
      before.body.workspaces.every((candidate) => candidate.id !== workspace.id),
      "recipient-before",
    );
    await stage("recipient-accept", async () => {
      await tab.getByRole("button", { name: "Accept invitation" }).click({ timeout });
      await tab.waitForURL(`${settings.appOrigin}/console/`, { timeout });
      await tab.getByRole("button", { name: "Sign out", exact: true }).waitFor({
        state: "visible",
        timeout,
      });
    });
    await stage("recipient-access", async () => {
      const me = await json(tab, "/v1/me");
      requireTrue(me.status === 200, "recipient-access");
      requireTrue(me.body.workspaces.some((candidate) => candidate.id === workspace.id), "recipient-access");
      requireTrue(me.body.projects.some((candidate) => candidate.id === workspace.project.id), "recipient-access");
      requireTrue(me.body.capabilities.role_keys.length === 0, "recipient-access");
      const project = await json(tab, `/v1/projects/${workspace.project.id}`);
      requireTrue(project.status === 200, "recipient-access");
      // Selection landed on the accepted workspace.
      const selected = await tab.getByRole("combobox").first().inputValue();
      requireTrue(selected === workspace.id, "recipient-access");
    });
    await stage("no-unrelated-creation", async () => {
      const after = await json(admin.page, "/v1/workspaces");
      requireTrue(after.body.workspaces.length === inventoryBefore.body.workspaces.length, "no-unrelated-creation");
      const listed = await json(admin.page, `/v1/workspaces/${workspace.id}/invites`);
      const legacyRow = listed.body.invites.find((invite) => invite.id === legacy.id);
      requireTrue(legacyRow?.status === "pending", "no-unrelated-creation");
    });
    await stage("token-boundary", async () => {
      for (const request of observed) {
        const url = new URL(request.url);
        const carries = request.url.includes(token);
        if (url.origin !== settings.appOrigin) requireTrue(!carries, "token-boundary");
        const acceptPost = request.method === "POST" && url.pathname === `/v1/invites/${token}/accept`;
        if (carries) requireTrue(acceptPost, "token-boundary");
      }
    });
    await admin.context.close();
    await recipient.context.close();
  } finally {
    await browser.close();
  }
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  try {
    const { chromium } = await import("playwright-core");
    await runInviteJourney({ chromium });
    process.stdout.write("invite-journey: shared link, sign-in continuation and explicit acceptance passed\n");
  } catch (error) {
    const name = error instanceof BrowserContractError ? error.stage : "unexpected";
    process.stderr.write(`invite-journey: ${name} failed\n`);
    process.exitCode = 78;
  }
}

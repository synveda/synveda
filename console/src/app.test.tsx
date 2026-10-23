/**
 * The entry point's routing priorities: an invitation waiting
 * in this tab is reached before the shell, any selection and the onboarding
 * redirect; it is displayed, never accepted, by rendering; and a signed-in
 * person without shared access is told how to get it rather than sent into a
 * creation wizard.
 */

import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { App, ME_KEY } from "./App.js";
import { cache } from "./cache.mjs";
import { clearPending, writePending } from "./invite.mjs";
import type { MeView } from "./generated/api.js";

const TOKEN =
  "synveda_invite_v1.0199c000-0000-7000-8000-000000000000.AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE";

function me(state: MeView["onboarding"]["state"]): MeView {
  return {
    principal: { subject: "robin@example.test", display_name: "Robin", quarantined: false },
    tenant: { id: "tenant-1", slug: "acme", name: "ACME", status: "active" },
    onboarding: {
      state,
      access_reason: state === "access_required" ? "no_shared_access" : undefined,
      workspace_count: 0,
      project_count: 0,
      workspace_create: state === "needs_workspace",
      project_create_workspace_ids: [],
    },
    capabilities: { actions: {}, role_keys: [] },
    anchors: [],
    workspaces: [],
    projects: [],
  };
}

async function render(outcome: Outcome): Promise<{ html: string; requests: string[] }> {
  cache.clear();
  await cache.ensure(ME_KEY, async () => outcome);
  const requests: string[] = [];
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    requests.push(String(input));
    return new Response("{}", { status: 500 });
  }) as typeof fetch;
  try {
    return { html: renderToStaticMarkup(<App />), requests };
  } finally {
    globalThis.fetch = original;
  }
}

test("a waiting invitation comes before onboarding, and rendering it accepts nothing", async () => {
  writePending(null, { token: TOKEN, capturedAt: Date.now(), account: null });
  const { html, requests } = await render({ kind: "ok", body: me("needs_workspace") });
  assert.match(html, /Join a workspace in ACME/);
  assert.match(html, /You will accept as <strong>Robin<\/strong>/);
  assert.match(html, /robin@example\.test/);
  assert.match(html, /Accept invitation/);
  assert.doesNotMatch(html, /Getting started|Create workspace|Product navigation/);
  assert.ok(!html.includes(TOKEN), "the token is never rendered");
  assert.deepEqual(requests, [], "displaying the invitation sends nothing");
  clearPending(null);
});

test("a signed-out recipient is asked to sign in, with a way to cancel", async () => {
  writePending(null, { token: TOKEN, capturedAt: Date.now(), account: null });
  const { html, requests } = await render({ kind: "unauthenticated" });
  assert.match(html, /You have been invited to Synveda/);
  assert.match(html, /href="\/auth\/login\?console=true"/);
  assert.match(html, /Cancel invitation/);
  assert.doesNotMatch(html, /Accept invitation/);
  assert.ok(!html.includes(TOKEN));
  assert.deepEqual(requests, []);
  clearPending(null);
});

test("an expired continuation is not offered", async () => {
  writePending(null, { token: TOKEN, capturedAt: Date.now() - 60 * 60 * 1000, account: null });
  const { html } = await render({ kind: "unauthenticated" });
  assert.match(html, /Welcome to Synveda/);
  assert.doesNotMatch(html, /invited/);
});

test("a signed-in person without shared access gets guidance, not a wizard", async () => {
  clearPending(null);
  const { html } = await render({ kind: "ok", body: me("access_required") });
  assert.match(html, /You need access to a workspace or project/);
  assert.match(html, /Refresh access/);
  assert.match(html, /Signed in as <strong>Robin<\/strong> \(robin@example\.test\) in ACME/);
  assert.doesNotMatch(html, /Create workspace|Create your first|Your first project/);
});

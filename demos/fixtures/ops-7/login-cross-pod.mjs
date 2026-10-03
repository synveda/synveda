#!/usr/bin/env node
// OPS-7: force OIDC and CLI handoff legs through three distinct gateway pods.
import { randomBytes } from "node:crypto";
import { readFileSync } from "node:fs";

const app = "http://synveda.synveda-test.svc.cluster.local:8120";
const issuer = "http://keycloak.synveda-test.svc.cluster.local:8080";
const pods = JSON.parse(process.env.POD_URLS ?? "null");
if (!Array.isArray(pods) || pods.length !== 3 || new Set(pods).size !== 3 ||
    !pods.every((pod) => /^http:\/\/\d{1,3}(?:\.\d{1,3}){3}:8120$/.test(pod))) {
  throw new Error("three distinct gateway pod URLs are required");
}
const password = readFileSync("/run/secrets/keycloak_demo_admin_password", "utf8").trimEnd();
const cookies = new Map();
const check = (value, message) => { if (!value) throw new Error(message); };
const request = async (url, init = {}) => fetch(url, {
  ...init, redirect: "manual", signal: AbortSignal.timeout(15_000),
});
const podRequest = (index, path, init) => request(`${pods[index]}${path}`, init);
const pathOf = (url) => {
  const parsed = new URL(url);
  check(parsed.origin === app, "callback origin changed");
  return `${parsed.pathname}${parsed.search}`;
};

async function identityRequest(url, init = {}) {
  check(url.startsWith(`${issuer}/realms/synveda/`), "identity request left the configured issuer");
  const headers = new Headers(init.headers);
  if (cookies.size) headers.set("Cookie", [...cookies].map(([key, value]) => `${key}=${value}`).join("; "));
  const response = await request(url, { ...init, headers });
  for (const header of response.headers.getSetCookie()) {
    const pair = header.split(";", 1)[0];
    const at = pair.indexOf("=");
    if (at > 0) cookies.set(pair.slice(0, at), pair.slice(at + 1));
  }
  return response;
}

async function finishIdentityLogin(authorize, username, secret) {
  const url = new URL(authorize);
  check(url.origin === issuer && url.pathname === "/realms/synveda/protocol/openid-connect/auth", "unexpected authorization endpoint");
  check(url.searchParams.get("code_challenge_method") === "S256", "PKCE S256 missing");
  check(url.searchParams.get("redirect_uri") === `${app}/auth/callback`, "redirect URI changed");
  for (const key of ["state", "nonce", "code_challenge"]) check(url.searchParams.has(key), `missing ${key}`);

  let pageUrl = authorize;
  let page;
  for (let hops = 0; hops < 5; hops++) {
    page = await identityRequest(pageUrl);
    const next = page.headers.get("location");
    if (!next) break;
    pageUrl = new URL(next, pageUrl).toString();
  }
  check(page?.status === 200, "identity login page unavailable");
  const html = await page.text();
  const tag = [...html.matchAll(/<form\b[^>]*>/gi)].find((match) => /\bid=["']kc-form-login["']/i.test(match[0]))?.[0];
  const encodedAction = tag?.match(/\baction=["']([^"']+)["']/i)?.[1];
  check(encodedAction, "bounded identity login form missing");
  const action = new URL(encodedAction.replaceAll("&amp;", "&").replaceAll("&#x3D;", "=").replaceAll("&#61;", "=").replaceAll("&quot;", '"'), pageUrl).toString();
  check(action.startsWith(`${issuer}/realms/synveda/login-actions/authenticate?`), "identity form action changed");
  const submitted = await identityRequest(action, {
    method: "POST",
    body: new URLSearchParams({ username, password: secret, credentialId: "" }),
  });
  const callback = submitted.headers.get("location");
  check(callback?.startsWith(`${app}/auth/callback?`), "identity callback missing");
  check(!/access_token|refresh_token|Bearer/i.test(callback), "credential appeared in callback URL");
  return callback;
}

async function beginLogin(path, username = "synveda-demo-admin", secret = password) {
  cookies.clear();
  const begun = await podRequest(0, path);
  check(begun.status === 307, `pod A refused login start (${begun.status})`);
  const authorize = begun.headers.get("location");
  check(authorize, "pod A did not redirect to identity provider");
  return finishIdentityLogin(authorize, username, secret);
}

const callback = await beginLogin("/auth/login");
const completed = await podRequest(1, pathOf(callback));
check(completed.status === 200, `pod B refused JSON callback (${completed.status})`);
const session = await completed.json();
check(typeof session.access_token === "string" && typeof session.subject === "string", "JSON login omitted session identity");
const replay = await podRequest(2, pathOf(callback));
check(replay.status >= 400, "pod C accepted a consumed login state");
for (let index = 0; index < pods.length; index++) {
  const me = await podRequest(index, "/v1/me", { headers: { Authorization: `Bearer ${session.access_token}` } });
  check(me.status === 200, `pod ${index + 1} refused the authenticated identity (${me.status})`);
  const identity = await me.json();
  check(identity.principal?.subject === session.subject, `pod ${index + 1} returned another identity`);
}

const cliState = randomBytes(32).toString("base64url");
const cliRedirect = "http://127.0.0.1:48943/callback";
const cliQuery = new URLSearchParams({ cli_redirect_uri: cliRedirect, cli_state: cliState });
const cliCallback = await beginLogin(`/auth/login?${cliQuery}`);
const handedOff = await podRequest(1, pathOf(cliCallback));
check(handedOff.status === 307, `pod B refused CLI callback (${handedOff.status})`);
const handoffUrl = handedOff.headers.get("location");
check(handoffUrl?.startsWith(`${cliRedirect}?`), "CLI handoff target changed");
check(!/access_token|refresh_token|Bearer/i.test(handoffUrl), "credential appeared in handoff URL");
const handoff = new URL(handoffUrl);
const code = handoff.searchParams.get("code");
check(code && handoff.searchParams.get("state") === cliState, "CLI handoff code/state missing");
const exchange = async () => podRequest(2, "/auth/cli/exchange", {
  method: "POST", headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ code, state: cliState }),
});
const redeemed = await exchange();
check(redeemed.status === 200, `pod C refused CLI redemption (${redeemed.status})`);
const cliSession = await redeemed.json();
check(cliSession.subject === session.subject, "CLI handoff returned another identity");
const duplicate = await exchange();
check(duplicate.status >= 400, "pod C redeemed a handoff twice");
console.log("OPS-7: three distinct pods completed JSON login A→B, CLI handoff A→B→C, replay refusal and per-pod identity reads");

// A second probe in this disposable cluster uses the same authenticated
// principal; never print or persist its short-lived bearer token.
export const accessToken = session.access_token;

export async function loginApprover() {
  const secret = readFileSync("/run/secrets/keycloak_demo_approver_password", "utf8").trimEnd();
  const callback = await beginLogin("/auth/login", "synveda-demo-approver", secret);
  const completed = await podRequest(1, pathOf(callback));
  check(completed.status === 200, `pod B refused approver login (${completed.status})`);
  const approver = await completed.json();
  check(typeof approver.access_token === "string" && typeof approver.subject === "string",
    "approver login omitted session identity");
  return { token: approver.access_token, subject: approver.subject };
}

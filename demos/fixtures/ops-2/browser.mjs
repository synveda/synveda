#!/usr/bin/env node
// The browser half of `synveda login`, driven headlessly — inside the
// cluster, in the same pod as the CLI that is waiting for it.
//
// This is a port of the block in demos/adpt-1-claude-code.sh, and it is a
// port rather than a new client on purpose: it drives the *product's* login
// flow end to end (AUTH-1's authorization code + PKCE, the gateway's own
// state and nonce, AUTH-2's provisioning at the callback, ADPT-1's one-time
// handoff code) and invents no second path to a bearer.
//
// Two containers, one pod, so `127.0.0.1` here is the CLI's loopback
// listener: containers in a pod share a network namespace, which is the
// only reason this can play the part a browser plays on a laptop.
//
// What it does NOT prove is stated where the demo reports it: no browser
// is involved, so this is the protocol path, not a person's.

import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";

const WORK = process.env.WORK_DIR ?? "/work";
const IDP = process.env.IDP_URL; // http://idp.synveda-test.svc.cluster.local:8080
const EMAIL = process.env.OPERATOR_EMAIL;
const PASSWORD = process.env.OPERATOR_PASSWORD;

if (!IDP || !EMAIL || !PASSWORD) {
  console.error("browser: IDP_URL, OPERATOR_EMAIL and OPERATOR_PASSWORD are required");
  process.exit(2);
}

const die = (message, detail) => {
  console.error(`browser FAILED: ${message}`);
  if (detail !== undefined) console.error(detail);
  writeFileSync(`${WORK}/browser.failed`, `${message}\n`);
  process.exit(1);
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// `fetch` follows redirects by default and this flow is made of them; every
// call here reads the Location itself.
const hop = (url, init = {}) => fetch(url, { ...init, redirect: "manual" });

const locationOf = (response) => response.headers.get("location");

// ── 1. what the CLI printed ──────────────────────────────────────────────
// `synveda login --no-browser` writes "if it does not open, visit: <url>"
// to stderr, which the client container redirects into this file.
let loginUrl = "";
for (let attempt = 0; attempt < 300 && !loginUrl; attempt++) {
  try {
    const log = readFileSync(`${WORK}/login.log`, "utf8");
    loginUrl = log.match(/https?:\/\/\S+\/auth\/login\?\S+/)?.[0] ?? "";
  } catch {
    // not written yet
  }
  if (!loginUrl) await sleep(200);
}
if (!loginUrl) die("the CLI never printed a login URL", tryRead(`${WORK}/login.log`));
console.log(`browser: the CLI wants a browser at ${loginUrl.slice(0, 72)}...`);

// ── 2. the gateway hands us to the IdP ───────────────────────────────────
const authorizeUrl = locationOf(await hop(loginUrl));
if (!authorizeUrl?.startsWith(`${IDP}/auth/v1/oidc/authorize`)) {
  die("/auth/login did not redirect to the test issuer", authorizeUrl);
}
// Every parameter comes from the gateway's own redirect — the state and
// nonce it parked, the PKCE challenge it will verify, and the redirect_uri
// it derived from SYNVEDA_PUBLIC_URL. Reconstructing any of them here would
// be testing this script instead of the product.
const params = new URL(authorizeUrl).searchParams;
for (const required of ["state", "nonce", "code_challenge", "redirect_uri", "client_id"]) {
  if (!params.get(required)) die(`the authorize URL carries no ${required}`, authorizeUrl);
}

// ── 3. a session and a CSRF token ────────────────────────────────────────
const page = await hop(authorizeUrl);
const cookies = page.headers
  .getSetCookie()
  .map((cookie) => cookie.split(";")[0])
  .join("; ");
const html = await page.text();
const csrf = html.match(/<template id="tpl_csrf_token">([^<]*)/)?.[1];
if (!cookies || !csrf) die("no session cookie or CSRF token from the IdP's login page", html.slice(0, 400));

// ── 4. the proof of work ─────────────────────────────────────────────────
// Rauthy requires one on the login endpoint. Cheap for one login and
// expensive for a script that guesses passwords, which is the point.
const powChallenge = await (await fetch(`${IDP}/auth/v1/pow`, { method: "POST" })).text();
const difficulty = Number.parseInt(powChallenge.split(":")[1], 10);
if (!Number.isFinite(difficulty)) die("could not read the PoW difficulty", powChallenge);
const leadingZeroBits = (digest) => {
  let bits = 0;
  for (const byte of digest) {
    if (byte === 0) {
      bits += 8;
      continue;
    }
    bits += Math.clz32(byte) - 24;
    break;
  }
  return bits;
};
let pow = "";
for (let counter = 0; ; counter++) {
  const digest = createHash("sha256").update(powChallenge).update(String(counter)).digest();
  if (leadingZeroBits(digest) >= difficulty) {
    pow = powChallenge + counter;
    break;
  }
}

// ── 5. the credentials ───────────────────────────────────────────────────
const login = await hop(`${IDP}/auth/v1/oidc/authorize`, {
  method: "POST",
  headers: { "Content-Type": "application/json", Cookie: cookies, "x-csrf-token": csrf },
  body: JSON.stringify({
    email: EMAIL,
    password: PASSWORD,
    client_id: params.get("client_id"),
    redirect_uri: params.get("redirect_uri"),
    state: params.get("state"),
    nonce: params.get("nonce"),
    code_challenge: params.get("code_challenge"),
    code_challenge_method: "S256",
    scopes: ["openid", "profile", "email", "groups"],
    pow,
  }),
});
const callbackUrl = locationOf(login);
if (!callbackUrl) {
  // The body is where Rauthy says what it objected to; a bare status hid
  // "Invalid user credentials" behind a 401 long enough to cost the ADPT-1
  // demo an afternoon.
  die(`the IdP returned no callback location (status ${login.status})`, await login.text());
}

// ── 6. the gateway's callback, and the handoff ───────────────────────────
// This is where AUTH-2 provisions: the org root is created from the
// tenant's own slug, the operator lands under it because their only group
// is the admin one, and both facts are chained under the operator's subject
// rather than an installer's.
const handoff = locationOf(await hop(callbackUrl));
if (!handoff?.startsWith("http://127.0.0.1:")) {
  die("the CLI callback was not a loopback handoff", handoff ?? "(no location)");
}
if (/access_token|refresh_token|Bearer/.test(handoff)) {
  die("a token travelled in the redirect URL", handoff);
}
const redeemed = await fetch(handoff);
if (!redeemed.ok) die(`the handoff redemption returned ${redeemed.status}`, await redeemed.text());

console.log("browser: login complete — the CLI holds a bearer it never saw in a URL");
writeFileSync(`${WORK}/browser.done`, "ok\n");

function tryRead(path) {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "(no log)";
  }
}

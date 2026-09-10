#!/usr/bin/env node
// Drives the server-rendered Keycloak login form for the CLI's real
// authorization-code + PKCE flow. This deliberately remains a protocol
// fixture; canonical Compose owns the Playwright browser claim. After the
// handoff it inspects, but never prints or exports, the CLI's private bearer so
// the issuer/audience/subject/group contract is proved by the installed image.

import { readFileSync, writeFileSync } from "node:fs";

const WORK = process.env.WORK_DIR ?? "/work";
const IDP = process.env.IDP_URL;
const APP = process.env.APP_URL;
const USERNAME = process.env.OPERATOR_USERNAME;
const PASSWORD_FILE = process.env.OPERATOR_PASSWORD_FILE;
const CREDENTIALS_FILE = process.env.CREDENTIALS_FILE;

if (!IDP || !APP || !USERNAME || !PASSWORD_FILE || !CREDENTIALS_FILE) {
  console.error(
    "browser: IDP_URL, APP_URL, OPERATOR_USERNAME, OPERATOR_PASSWORD_FILE and CREDENTIALS_FILE are required",
  );
  process.exit(2);
}

const die = (message, detail) => {
  console.error(`browser FAILED: ${message}`);
  if (detail !== undefined) console.error(detail);
  writeFileSync(`${WORK}/browser.failed`, `${message}\n`);
  process.exit(1);
};

const readSecret = (path) => {
  const raw = readFileSync(path, "utf8");
  if (raw.includes("\0") || !/^[^\r\n]+\r?\n?$/.test(raw)) {
    die("the operator password file was malformed");
  }
  return raw.replace(/\r?\n$/, "");
};

const PASSWORD = readSecret(PASSWORD_FILE);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const cookies = new Map();

const rememberCookies = (response) => {
  const values = response.headers.getSetCookie?.() ?? [];
  for (const value of values) {
    const pair = value.split(";", 1)[0];
    const separator = pair.indexOf("=");
    if (separator > 0) cookies.set(pair.slice(0, separator), pair.slice(separator + 1));
  }
};

const cookieHeader = () =>
  [...cookies.entries()].map(([name, value]) => `${name}=${value}`).join("; ");

const hop = async (url, init = {}) => {
  const headers = new Headers(init.headers ?? {});
  const cookie = cookieHeader();
  if (cookie) headers.set("Cookie", cookie);
  const response = await fetch(url, { ...init, headers, redirect: "manual" });
  rememberCookies(response);
  return response;
};

const locationOf = (response, base) => {
  const location = response.headers.get("location");
  return location ? new URL(location, base).toString() : null;
};

const decodeAttribute = (value) =>
  value
    .replaceAll("&amp;", "&")
    .replaceAll("&#x3D;", "=")
    .replaceAll("&#61;", "=")
    .replaceAll("&quot;", '"');

let loginUrl = "";
for (let attempt = 0; attempt < 300 && !loginUrl; attempt++) {
  try {
    const log = readFileSync(`${WORK}/login.log`, "utf8");
    loginUrl = log.match(/https?:\/\/\S+\/auth\/login\?\S+/)?.[0] ?? "";
  } catch {
    // The client has not written its log yet.
  }
  if (!loginUrl) await sleep(200);
}
if (!loginUrl) die("the CLI never printed a login URL", tryRead(`${WORK}/login.log`));
console.log(`browser: the CLI wants a browser at ${loginUrl.slice(0, 72)}...`);

const gatewayLogin = await hop(loginUrl);
const authorizeUrl = locationOf(gatewayLogin, loginUrl);
const expectedAuthorizePrefix = `${IDP}/realms/synveda/protocol/openid-connect/auth`;
if (!authorizeUrl?.startsWith(expectedAuthorizePrefix)) {
  die("/auth/login did not redirect to the bundled Keycloak issuer", authorizeUrl);
}

const params = new URL(authorizeUrl).searchParams;
for (const required of ["state", "nonce", "code_challenge", "redirect_uri", "client_id"]) {
  if (!params.get(required)) die(`the authorize URL carries no ${required}`, authorizeUrl);
}
if (params.get("code_challenge_method") !== "S256") {
  die("the authorize URL did not require PKCE S256");
}
if (params.get("client_id") !== "synveda") {
  die("the authorize URL selected an unexpected client");
}
if (!params.get("redirect_uri")?.startsWith(`${APP}/auth/callback`)) {
  die("the authorize URL selected an unexpected callback");
}

let pageUrl = authorizeUrl;
let page;
for (let redirects = 0; redirects < 5; redirects++) {
  page = await hop(pageUrl);
  const next = locationOf(page, pageUrl);
  if (!next) break;
  if (!next.startsWith(`${IDP}/`)) die("Keycloak redirected its login page off issuer", next);
  pageUrl = next;
}
if (!page || page.status !== 200) {
  die(`Keycloak did not return a login page (status ${page?.status ?? "unknown"})`);
}
const html = await page.text();
const formTag = [...html.matchAll(/<form\b[^>]*>/gi)].find((match) =>
  /\bid=["']kc-form-login["']/i.test(match[0]),
)?.[0];
const actionMatch = formTag?.match(/\baction=["']([^"']+)["']/i);
if (!actionMatch) die("Keycloak returned no bounded login form");
const action = new URL(decodeAttribute(actionMatch[1]), pageUrl).toString();
if (!action.startsWith(`${IDP}/realms/synveda/login-actions/authenticate?`)) {
  die("Keycloak returned an unexpected login action");
}

const form = new URLSearchParams({
  username: USERNAME,
  password: PASSWORD,
  credentialId: "",
});
const login = await hop(action, {
  method: "POST",
  headers: { "Content-Type": "application/x-www-form-urlencoded" },
  body: form,
});
const callbackUrl = locationOf(login, action);
if (!callbackUrl?.startsWith(`${APP}/auth/callback?`)) {
  die(`Keycloak returned no application callback (status ${login.status})`);
}

const callback = await hop(callbackUrl);
const handoff = locationOf(callback, callbackUrl);
if (!handoff?.startsWith("http://127.0.0.1:")) {
  die("the application callback was not a loopback handoff", handoff ?? "(no location)");
}
if (/access_token|refresh_token|Bearer/i.test(`${callbackUrl}\n${handoff}`)) {
  die("a token travelled in a redirect URL");
}
const redeemed = await fetch(handoff);
if (!redeemed.ok) die(`the handoff redemption returned ${redeemed.status}`);

let profile;
for (let attempt = 0; attempt < 300 && !profile; attempt++) {
  try {
    profile = JSON.parse(readFileSync(CREDENTIALS_FILE, "utf8"))?.profiles?.default;
  } catch {
    // The CLI writes by atomic rename after exchanging the one-time handoff.
  }
  if (!profile) await sleep(200);
}
if (!profile) die("the CLI never stored its exchanged credentials");

let header;
let claims;
try {
  const segments = profile.access_token.split(".");
  if (segments.length !== 3) throw new Error();
  header = JSON.parse(Buffer.from(segments[0], "base64url").toString("utf8"));
  claims = JSON.parse(Buffer.from(segments[1], "base64url").toString("utf8"));
} catch {
  die("the stored access token was not a compact JWT");
}
if (!header || typeof header !== "object" || Array.isArray(header) ||
    !claims || typeof claims !== "object" || Array.isArray(claims)) {
  die("the stored access token carried malformed JSON objects");
}
const audiences = typeof claims.aud === "string" ? [claims.aud] : claims.aud;
if (header.alg !== "RS256") die("the stored access token did not use RS256");
if (claims.iss !== `${IDP}/realms/synveda` || profile.issuer !== claims.iss) {
  die("the stored access token did not use the exact configured issuer");
}
if (claims.azp !== "synveda") die("the stored access token named the wrong client");
if (!Array.isArray(audiences) || audiences.length !== 1 || audiences[0] !== "synveda-api") {
  die("the stored access token named an unexpected audience");
}
if (typeof claims.sub !== "string" || claims.sub.length === 0 || profile.subject !== claims.sub) {
  die("the stored access token carried no stable subject");
}
if (!Array.isArray(claims.groups) || claims.groups.length !== 1 || claims.groups[0] !== "synveda-admins") {
  die("the stored access token carried an unexpected group projection");
}
if (claims.auth_time !== undefined && (!Number.isSafeInteger(claims.auth_time) || claims.auth_time <= 0)) {
  die("the stored access token carried malformed authentication time");
}

console.log("browser: login complete — exact issuer, RS256, subject, audience and group verified");
writeFileSync(`${WORK}/browser.done`, "ok\n");

function tryRead(path) {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return "(no log)";
  }
}

#!/usr/bin/env node
// Live probe of the bundled Keycloak revocation profile (ADR-0130) against an
// isolated, disposable Keycloak. It imports the realm exactly as the Helm
// chart ships it, creates one synthetic user and one synthetic service client,
// and drives the real `synveda` public client through authorization code +
// PKCE as a browser would. With PROBE_GATEWAY_URLS it also drives one or two
// Synveda gateways that share a database and enforce the profile.
//
// It never prints or stores a token, password, cookie or identifier value:
// the evidence file holds booleans, counts, lifetimes and timings only.
//
// Usage:
//   PROBE_KEYCLOAK_URL=http://127.0.0.1:18180 \
//   PROBE_ADMIN_PASSWORD_FILE=/path/to/bootstrap-admin-password \
//   [PROBE_GATEWAY_URLS=http://127.0.0.1:18120,http://127.0.0.1:18121] \
//   [PROBE_CLI_BIN=target/debug/synveda] \
//   node demos/auth-6-keycloak-revocation-probe.mjs <evidence.json>
//
// The Keycloak must be disposable: the probe creates (or replaces) the
// `synveda` realm in it. Never point it at a retained deployment.

import { spawnSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const REALM = "synveda";
const CLIENT = "synveda";
const SERVICE_CLIENT = "synveda-probe-agent";
const SERVICE_AUDIENCE = "synveda-agents";
const API_AUDIENCE = "synveda-api";
const DENIAL_BOUND_MS = 30_000;
const POLL_MS = 100;

const keycloak = (process.env.PROBE_KEYCLOAK_URL ?? "http://127.0.0.1:18180").replace(/\/$/u, "");
const gateways = (process.env.PROBE_GATEWAY_URLS ?? "")
  .split(",")
  .map((url) => url.trim().replace(/\/$/u, ""))
  .filter(Boolean);
const publicUrl = gateways[0] ?? (process.env.PROBE_PUBLIC_URL ?? "http://127.0.0.1:18120");
const issuer = `${keycloak}/realms/${REALM}`;
const evidencePath = process.argv[2];
if (!evidencePath) fail("usage: node demos/auth-6-keycloak-revocation-probe.mjs <evidence.json>");
if (gateways.length > 2) fail("PROBE_GATEWAY_URLS names at most two gateways");

function fail(message) {
  console.error(`probe: ${message}`);
  process.exit(2);
}

function secret() {
  return randomBytes(32).toString("base64url");
}

// ── Minimal cookie jar: one browser, host-scoped like a browser's cookies ──

class Browser {
  constructor() {
    this.cookies = new Map();
  }

  header(url) {
    const jar = this.cookies.get(new URL(url).hostname);
    return jar ? [...jar].map(([name, value]) => `${name}=${value}`).join("; ") : "";
  }

  remember(url, response) {
    const host = new URL(url).hostname;
    const jar = this.cookies.get(host) ?? new Map();
    for (const line of response.headers.getSetCookie()) {
      const [pair, ...attributes] = line.split(";");
      const index = pair.indexOf("=");
      const name = pair.slice(0, index).trim();
      const value = pair.slice(index + 1).trim();
      const expired = attributes.some((attribute) => /^\s*max-age=0\s*$/iu.test(attribute));
      if (expired || value === "") jar.delete(name);
      else jar.set(name, value);
    }
    this.cookies.set(host, jar);
  }

  cookie(url, name) {
    return this.cookies.get(new URL(url).hostname)?.get(name);
  }

  async fetch(url, options = {}) {
    const headers = new Headers(options.headers ?? {});
    const cookie = this.header(url);
    if (cookie) headers.set("cookie", cookie);
    const response = await fetch(url, { ...options, headers, redirect: "manual" });
    this.remember(url, response);
    return response;
  }
}

function decode(jwt) {
  const [, payload] = jwt.split(".");
  return JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
}

function bounded(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 256;
}

async function form(url, fields, headers = {}) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded", ...headers },
    body: new URLSearchParams(fields),
  });
  const body = await response.json().catch(() => ({}));
  return { status: response.status, body };
}

// ── Isolated fixture: the shipped realm, one user, one service client ──────

function shippedRealm() {
  const template = readFileSync(
    join(ROOT, "deploy/helm/synveda/templates/keycloak-realm.yaml"),
    "utf8",
  ).replace(/\r\n/gu, "\n");
  const start = template.indexOf("synveda-realm.json: |\n");
  if (start < 0) fail("the Helm realm template changed shape");
  let json = template
    .slice(start + "synveda-realm.json: |\n".length)
    .replace(/\n\{\{- end \}\}\s*$/u, "\n");
  // Evaluation accounts are replaced by one synthetic probe user below.
  json = json.replace(
    /\{\{- if \.Values\.keycloak\.evaluationAccountsExistingSecret \}\}[\s\S]*?\{\{- end \}\}\n(\s*"clients")/u,
    "$1",
  );
  const mapper = (name) =>
    JSON.stringify(JSON.parse(readFileSync(join(ROOT, "deploy/helm/synveda/files", name), "utf8")));
  json = json
    .replace(/\{\{ ternary "none" "external" \.Values\.keycloak\.localEvaluation \| toJson \}\}/u, '"none"')
    .replace(/\{\{ printf "%s\/console\/" \.Values\.gateway\.publicUrl \| toJson \}\}/u, JSON.stringify(`${publicUrl}/console/`))
    .replace(/\{\{ printf "%s\/auth\/callback" \.Values\.gateway\.publicUrl \| toJson \}\}/u, JSON.stringify(`${publicUrl}/auth/callback`))
    .replaceAll("{{ .Values.gateway.publicUrl | toJson }}", JSON.stringify(publicUrl))
    .replace(/\{\{ \.Files\.Get "files\/(synveda-[a-z]+-mapper\.json)" \| fromJson \| toJson \}\}/gu, (_, name) => mapper(name));
  if (json.includes("{{")) fail("an unrendered Helm expression remains in the realm");
  return JSON.parse(json);
}

async function adminToken() {
  const passwordFile = process.env.PROBE_ADMIN_PASSWORD_FILE;
  if (!passwordFile) fail("PROBE_ADMIN_PASSWORD_FILE is required");
  const { status, body } = await form(`${keycloak}/realms/master/protocol/openid-connect/token`, {
    grant_type: "password",
    client_id: "admin-cli",
    username: process.env.PROBE_ADMIN_USER ?? "probe-admin",
    password: readFileSync(passwordFile, "utf8").trim(),
  });
  if (status !== 200) fail(`isolated Keycloak administrator login failed with HTTP ${status}`);
  return body.access_token;
}

async function admin(token, method, path, body) {
  const response = await fetch(`${keycloak}/admin/realms${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status >= 400 && !(method === "DELETE" && response.status === 404)) {
    fail(`administrator ${method} ${path.replace(/[0-9a-f-]{36}/gu, "<id>")} returned HTTP ${response.status}`);
  }
  return response;
}

async function removeNamed(token, collection, query) {
  const response = await admin(token, "GET", `/${REALM}/${collection}?${query}&exact=true`);
  for (const entry of await response.json()) {
    await admin(token, "DELETE", `/${REALM}/${collection}/${entry.id}`);
  }
}

// The realm is imported once per Keycloak so its signing keys stay stable for
// gateways that already cached them; the probe user and client are renewed.
async function prepareFixture(realm) {
  const token = await adminToken();
  const existing = await fetch(`${keycloak}/admin/realms/${REALM}`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (existing.status === 404) await admin(token, "POST", "", realm);
  await removeNamed(token, "users", "username=probe-person");
  await removeNamed(token, "clients", `clientId=${SERVICE_CLIENT}`);
  const user = { username: "probe-person", password: secret() };
  await admin(token, "POST", `/${REALM}/users`, {
    username: user.username,
    enabled: true,
    email: "probe-person@synveda.invalid",
    emailVerified: true,
    firstName: "Probe",
    lastName: "Person",
    credentials: [{ type: "password", temporary: false, value: user.password }],
  });
  const service = { clientId: SERVICE_CLIENT, secret: secret() };
  await admin(token, "POST", `/${REALM}/clients`, {
    clientId: service.clientId,
    enabled: true,
    protocol: "openid-connect",
    publicClient: false,
    secret: service.secret,
    standardFlowEnabled: false,
    directAccessGrantsEnabled: false,
    serviceAccountsEnabled: true,
    fullScopeAllowed: false,
    protocolMappers: [
      {
        name: "synveda-agent-audience",
        protocol: "openid-connect",
        protocolMapper: "oidc-audience-mapper",
        config: {
          "included.custom.audience": SERVICE_AUDIENCE,
          "access.token.claim": "true",
          "id.token.claim": "false",
        },
      },
    ],
  });
  return { user, service };
}

// ── The real `synveda` public client, as a browser ─────────────────────────

async function signIn(browser, user, authorizeUrl, finalPrefix) {
  let response = await browser.fetch(authorizeUrl);
  let page = 0;
  while (response.status === 200 && page < 3) {
    const html = await response.text();
    const action = /<form[^>]*id="kc-form-login"[^>]*action="([^"]+)"/u.exec(html)?.[1];
    if (!action) fail("the Keycloak login page had no login form");
    const target = action.replaceAll("&amp;", "&");
    response = await browser.fetch(target, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ username: user.username, password: user.password, credentialId: "" }),
    });
    page += 1;
  }
  let location = response.headers.get("location");
  for (let hop = 0; location && !location.startsWith(finalPrefix) && hop < 6; hop += 1) {
    response = await browser.fetch(new URL(location, keycloak).toString());
    location = response.headers.get("location");
  }
  if (!location?.startsWith(finalPrefix)) fail(`the login did not reach ${finalPrefix.replace(/\?.*$/u, "")}`);
  return new URL(location);
}

async function directLogin(browser, user, scope = "openid profile email") {
  const verifier = secret();
  const state = secret();
  const query = new URLSearchParams({
    response_type: "code",
    client_id: CLIENT,
    redirect_uri: `${publicUrl}/auth/callback`,
    scope,
    state,
    nonce: secret(),
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    code_challenge_method: "S256",
  });
  const landed = await signIn(
    browser,
    user,
    `${issuer}/protocol/openid-connect/auth?${query}`,
    `${publicUrl}/auth/callback?`,
  );
  if (landed.searchParams.get("error")) return { refused: landed.searchParams.get("error") };
  const exchanged = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "authorization_code",
    client_id: CLIENT,
    code: landed.searchParams.get("code"),
    redirect_uri: `${publicUrl}/auth/callback`,
    code_verifier: verifier,
  });
  if (exchanged.status !== 200) fail(`code exchange returned HTTP ${exchanged.status}`);
  return exchanged.body;
}

function accessShape(token) {
  const claims = decode(token);
  const audiences = [claims.aud].flat();
  return {
    jti: bounded(claims.jti),
    sid: bounded(claims.sid),
    numeric_times: Number.isInteger(claims.iat) && Number.isInteger(claims.exp),
    lifetime_seconds: claims.exp - claims.iat,
    api_audience: audiences.includes(API_AUDIENCE),
    azp_is_client: claims.azp === CLIENT,
  };
}

async function claimProbe(fixture) {
  const browser = new Browser();
  const first = await directLogin(browser, fixture.user);
  const access = decode(first.access_token);
  const identity = decode(first.id_token);
  const refreshClaims = decode(first.refresh_token);
  const evidence = {
    login_access_token: accessShape(first.access_token),
    id_token_sid_matches_access_sid: identity.sid === access.sid,
    refresh_token_type: refreshClaims.typ,
  };

  const refreshed = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "refresh_token",
    client_id: CLIENT,
    refresh_token: first.refresh_token,
  });
  const next = decode(refreshed.body.access_token);
  evidence.refresh = {
    status: refreshed.status,
    access_token: accessShape(refreshed.body.access_token),
    sid_stable: next.sid === access.sid,
    jti_changed: next.jti !== access.jti,
    refresh_token_rotated: refreshed.body.refresh_token !== first.refresh_token,
  };
  const replay = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "refresh_token",
    client_id: CLIENT,
    refresh_token: first.refresh_token,
  });
  evidence.rotated_refresh_reuse = { status: replay.status, error: replay.body.error ?? null };
  const after = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "refresh_token",
    client_id: CLIENT,
    refresh_token: refreshed.body.refresh_token,
  });
  evidence.latest_refresh_after_reuse_attempt = { status: after.status, error: after.body.error ?? null };
  const latest = after.status === 200 ? after.body.refresh_token : refreshed.body.refresh_token;

  const again = await directLogin(browser, fixture.user);
  evidence.second_login_same_browser_shares_sid = decode(again.access_token).sid === access.sid;
  const offline = await directLogin(browser, fixture.user, "openid offline_access");
  evidence.offline_access_requested = offline.refused
    ? { refused: offline.refused }
    : { refresh_token_type: decode(offline.refresh_token).typ };

  const logout = await form(`${issuer}/protocol/openid-connect/logout`, {
    client_id: CLIENT,
    refresh_token: again.refresh_token ?? latest,
  });
  evidence.issuer_logout_status = logout.status;
  const dead = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "refresh_token",
    client_id: CLIENT,
    refresh_token: latest,
  });
  evidence.refresh_after_issuer_logout = { status: dead.status, error: dead.body.error ?? null };
  const fresh = await directLogin(new Browser(), fixture.user);
  evidence.fresh_login_has_new_sid = decode(fresh.access_token).sid !== access.sid;

  const service = await form(`${issuer}/protocol/openid-connect/token`, {
    grant_type: "client_credentials",
    client_id: fixture.service.clientId,
    client_secret: fixture.service.secret,
  });
  const serviceClaims = service.status === 200 ? decode(service.body.access_token) : {};
  evidence.service_token = {
    status: service.status,
    jti: bounded(serviceClaims.jti),
    sid_present: serviceClaims.sid !== undefined,
    lifetime_seconds: serviceClaims.exp - serviceClaims.iat,
    service_audience: [serviceClaims.aud].flat().includes(SERVICE_AUDIENCE),
    refresh_token_issued: service.body.refresh_token !== undefined,
  };
  return evidence;
}

// ── Synveda gateways enforcing the profile ─────────────────────────────────

const CONSOLE_COOKIE = "synveda_console_dev";

async function gatewayConsoleLogin(browser, user, gateway) {
  const start = await browser.fetch(`${gateway}/auth/login?console=true`);
  const landed = await signIn(browser, user, start.headers.get("location"), `${gateway}/auth/callback?`);
  const callback = await browser.fetch(landed.toString());
  const location = callback.headers.get("location") ?? "";
  if (location.includes("error=")) return { error: new URL(location, gateway).searchParams.get("error") };
  return { cookie: browser.cookie(gateway, CONSOLE_COOKIE) };
}

async function gatewayCliLogin(browser, user, gateway) {
  const state = secret();
  const start = await browser.fetch(
    `${gateway}/auth/login?cli_redirect_uri=${encodeURIComponent("http://127.0.0.1:9/callback")}&cli_state=${state}`,
  );
  const landed = await signIn(browser, user, start.headers.get("location"), `${gateway}/auth/callback?`);
  const handed = await browser.fetch(landed.toString());
  const code = new URL(handed.headers.get("location")).searchParams.get("code");
  const response = await fetch(`${gateway}/auth/cli/exchange`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ code, state }),
  });
  if (response.status !== 200) fail(`CLI exchange returned HTTP ${response.status}`);
  return response.json();
}

async function api(gateway, path, { bearer, cookie, method = "GET", body } = {}) {
  const headers = {};
  if (bearer) headers.authorization = `Bearer ${bearer}`;
  if (cookie) {
    headers.cookie = `${CONSOLE_COOKIE}=${cookie}`;
    headers.origin = gateways[0];
  }
  if (body !== undefined) headers["content-type"] = "application/json";
  const response = await fetch(`${gateway}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { status: response.status, body: await response.json().catch(() => null) };
}

// The native CLI as a third public-API client. The bearer travels only in the
// child's environment; output is discarded and only the exit status kept.
function cliWhoami(gateway, bearer) {
  const binary = process.env.PROBE_CLI_BIN;
  if (!binary) return null;
  const result = spawnSync(binary, ["whoami", "--json"], {
    env: { ...process.env, SYNVEDA_TOKEN: bearer, SYNVEDA_GATEWAY: gateway },
    stdio: "ignore",
    timeout: 30_000,
  });
  return result.status;
}

async function refreshThroughGateway(gateway, refreshToken) {
  return api(gateway, "/auth/refresh", { method: "POST", body: { refresh_token: refreshToken } });
}

async function deniedWithin(check) {
  const started = performance.now();
  while (performance.now() - started < DENIAL_BOUND_MS) {
    if ((await check()) === 401) return Math.round(performance.now() - started);
    await new Promise((resolve) => setTimeout(resolve, POLL_MS));
  }
  return null;
}

async function gatewayProbe(fixture) {
  const [first, second = gateways[0]] = gateways;
  const browser = new Browser();
  const consoleLogin = await gatewayConsoleLogin(browser, fixture.user, first);
  if (!consoleLogin.cookie) fail(`console login was refused (${consoleLogin.error})`);
  const cli = await gatewayCliLogin(browser, fixture.user, first);
  const evidence = {
    gateways: gateways.length,
    before: {
      console_whoami: (await api(first, "/v1/whoami", { cookie: consoleLogin.cookie })).status,
      cli_whoami: (await api(second, "/v1/whoami", { bearer: cli.access_token })).status,
      native_cli_exit: cliWhoami(second, cli.access_token),
    },
  };
  const listed = await api(first, "/v1/me/console-sessions", { cookie: consoleLogin.cookie });
  evidence.inventory = {
    status: listed.status,
    sessions: listed.body?.sessions?.length ?? 0,
    fields: Object.keys(listed.body?.sessions?.[0] ?? {}).sort(),
  };

  // Single bearer: a refreshed sibling of the same family survives.
  const sibling = await refreshThroughGateway(first, cli.refresh_token);
  const tokenRevoked = await api(second, "/v1/me/credential/revoke", {
    bearer: cli.access_token,
    method: "POST",
    body: { identifier: "token" },
  });
  evidence.single_bearer = {
    revoke_status: tokenRevoked.status,
    enforced: tokenRevoked.body?.enforced ?? null,
    revoked_bearer_denied_ms: await deniedWithin(
      async () => (await api(first, "/v1/whoami", { bearer: cli.access_token })).status,
    ),
    sibling_still_admitted: (await api(first, "/v1/whoami", { bearer: sibling.body?.access_token })).status === 200,
  };

  // Family: revoke the console session; every bearer of its issuer session ends.
  const id = listed.body.sessions[0].id;
  const revoked = await api(first, `/v1/me/console-sessions/${id}`, {
    cookie: consoleLogin.cookie,
    method: "DELETE",
  });
  evidence.family = {
    revoke_status: revoked.status,
    console_denied_ms: await deniedWithin(
      async () => (await api(first, "/v1/whoami", { cookie: consoleLogin.cookie })).status,
    ),
    cli_bearer_denied_on_other_gateway_ms: await deniedWithin(
      async () => (await api(second, "/v1/whoami", { bearer: sibling.body?.access_token })).status,
    ),
    native_cli_exit_after: cliWhoami(second, sibling.body?.access_token),
    refresh_through_gateway_status: (await refreshThroughGateway(second, sibling.body?.refresh_token)).status,
    relogin_same_issuer_session: (await gatewayConsoleLogin(browser, fixture.user, first)).error ?? "admitted",
  };
  const fresh = await gatewayConsoleLogin(new Browser(), fixture.user, first);
  evidence.family.fresh_issuer_session = fresh.cookie
    ? (await api(first, "/v1/whoami", { cookie: fresh.cookie })).status
    : fresh.error;
  return evidence;
}

const realm = shippedRealm();
const fixture = await prepareFixture(realm);
const startedAt = new Date().toISOString();
const record = {
  kind: "isolated bundled-Keycloak revocation-profile probe",
  started_at: startedAt,
  keycloak_server: (await (await fetch(`${keycloak}/realms/master/.well-known/openid-configuration`)).json()).issuer
    ? "reachable"
    : "unknown",
  realm_source: "deploy/helm/synveda/templates/keycloak-realm.yaml",
  realm_settings: {
    accessTokenLifespan: realm.accessTokenLifespan,
    ssoSessionMaxLifespan: realm.ssoSessionMaxLifespan,
    revokeRefreshToken: realm.revokeRefreshToken,
    refreshTokenMaxReuse: realm.refreshTokenMaxReuse,
  },
  claims: await claimProbe(fixture),
};
if (gateways.length > 0) record.gateway = await gatewayProbe(fixture);
record.finished_at = new Date().toISOString();
writeFileSync(evidencePath, `${JSON.stringify(record, null, 2)}\n`);
console.log(JSON.stringify(record, null, 2));

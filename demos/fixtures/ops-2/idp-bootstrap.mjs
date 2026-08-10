#!/usr/bin/env node
// Provisions the test IdP: the `synveda` OIDC client, the
// `synveda-admins` group, and one operator in it. Then prints the issuer
// **as the discovery document states it**, so the gateway's trust entry is
// built from what the IdP actually publishes rather than from a string
// somebody assembled — ADR-0010 compares the two byte-for-byte, and a
// trailing slash is the kind of difference that costs an afternoon.
//
// Runs on the host, against a `kubectl port-forward` to the IdP Service.
// That is administration, not login: what has to happen inside the cluster
// is the login flow, because the issuer URL the gateway resolves and the
// one the client resolves must be the same URL (ADR-0062 decision 8).
//
// Usage: node idp-bootstrap.mjs <admin-base-url> <gateway-public-url>
//   e.g. node idp-bootstrap.mjs http://127.0.0.1:18080 http://synveda.synveda-test.svc.cluster.local:8120

const [adminBase, gatewayPublicUrl] = process.argv.slice(2);
if (!adminBase || !gatewayPublicUrl) {
  console.error("usage: idp-bootstrap.mjs <admin-base-url> <gateway-public-url>");
  process.exit(2);
}

// Matches deploy/compose/rauthy/config.toml and demos/fixtures/ops-2/idp.yaml.
const API_KEY = "synveda-dev";
const API_SECRET = "6xxmjZD7Wqe9zWN1fWzOW1jA4uxAkFQ9rYlVFpxBzVgJ0xEj2KWSLiaRTZzKV1oz";
const AUTH = `API-Key ${API_KEY}$${API_SECRET}`;

export const OPERATOR_EMAIL = "operator@synveda.test";
export const OPERATOR_PASSWORD = "install-test-Operator-1";
const ADMIN_GROUP = "synveda-admins";
const CLIENT_ID = "synveda";

const api = async (method, path, body) => {
  const response = await fetch(`${adminBase}/auth/v1${path}`, {
    method,
    headers: { Authorization: AUTH, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  return { status: response.status, text, json: safeJson(text) };
};

const safeJson = (text) => {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
};

const die = (message, detail) => {
  console.error(`idp-bootstrap: ${message}`);
  if (detail !== undefined) console.error(detail);
  process.exit(1);
};

// Every step converges rather than creating: this script is re-runnable
// against a cluster somebody kept with KEEP=1, and Rauthy refuses a
// password it has seen in its last three, which is exactly what a re-run
// looks like to it (ADR-0055 decision 7, learned by the ADPT-1 bootstrap).

// ── the admin group ──────────────────────────────────────────────────────
{
  const existing = await api("GET", "/groups");
  if (existing.status !== 200) die("could not list groups", existing.text);
  if (!existing.json.some((group) => group.name === ADMIN_GROUP)) {
    const created = await api("POST", "/groups", { group: ADMIN_GROUP });
    if (created.status >= 300) die("could not create the admin group", created.text);
  }
}

// ── the client ───────────────────────────────────────────────────────────
// Public client with PKCE: the gateway is the OAuth client and holds no
// secret for this one, which is what AUTH-1 configured in the dev IdP too.
//
// Create-then-always-update, which is the shape demos/auth-1-oidc-login.sh
// uses and the reason it works. Rauthy's create takes the identity of a
// client and defaults everything else — including **EdDSA tokens**, where
// ADR-0010's trust entry allows RS256 — so a bootstrap that only creates
// leaves a client whose tokens the gateway will refuse with "token
// algorithm not allowed for issuer". The first run of this test failed
// exactly there: the login completed at the IdP, the callback and the
// handoff both succeeded, and the CLI's own bearer was then rejected.
{
  const redirectUris = [`${gatewayPublicUrl}/auth/callback`];
  const existing = await api("GET", `/clients/${CLIENT_ID}`);
  if (existing.status !== 200) {
    const created = await api("POST", "/clients", {
      id: CLIENT_ID,
      name: "Synveda Gateway",
      confidential: false,
      redirect_uris: redirectUris,
      post_logout_redirect_uris: [],
    });
    if (created.status >= 300) die("could not create the synveda client", created.text);
  }
  const desired = await api("PUT", `/clients/${CLIENT_ID}`, {
    id: CLIENT_ID,
    name: "Synveda Gateway",
    enabled: true,
    confidential: false,
    redirect_uris: redirectUris,
    flows_enabled: ["authorization_code"],
    access_token_alg: "RS256",
    id_token_alg: "RS256",
    auth_code_lifetime: 60,
    access_token_lifetime: 1800,
    scopes: ["openid", "email", "profile", "groups"],
    default_scopes: ["openid"],
    challenges: ["S256"],
    force_mfa: false,
  });
  if (desired.status >= 300) die("could not configure the synveda client", desired.text);
}

// ── the operator ─────────────────────────────────────────────────────────
// In the admin group and nowhere else. ADR-0015 decision 6 places a subject
// like this under the org root rather than in quarantine, and AUTH-2's
// `ensure_root` manufactures that root on the way — which is the whole
// reason this test needs no seeding step and the installer creates no
// hierarchy.
{
  const users = await api("GET", "/users");
  if (users.status !== 200) die("could not list users", users.text);
  const existing = users.json.find((user) => user.email === OPERATOR_EMAIL);
  const attributes = {
    email: OPERATOR_EMAIL,
    given_name: "Install",
    family_name: "Operator",
    groups: [ADMIN_GROUP],
    roles: [],
    enabled: true,
    email_verified: true,
    language: "en",
  };
  const id = existing?.id;
  if (!id) {
    const created = await api("POST", "/users", attributes);
    if (created.status >= 300) die("could not create the operator", created.text);
  } else {
    const updated = await api("PUT", `/users/${id}`, { ...attributes, user_expires: null });
    if (updated.status >= 300) die("could not update the operator", updated.text);
  }

  const settled = await api("GET", "/users");
  const operator = settled.json.find((user) => user.email === OPERATOR_EMAIL);
  if (!operator) die("the operator is not there after provisioning it");

  // The password. Rauthy's history rule can refuse the constant on a
  // re-run; that means it is already current, which is the state we want.
  const reset = await api("PUT", `/users/${operator.id}`, {
    ...attributes,
    password: OPERATOR_PASSWORD,
  });
  if (reset.status >= 300 && !/password/i.test(reset.text)) {
    die("could not set the operator's password", reset.text);
  }
}

// ── the issuer, as published ─────────────────────────────────────────────
{
  const discovery = await fetch(`${adminBase}/auth/v1/.well-known/openid-configuration`);
  if (!discovery.ok) die(`discovery returned ${discovery.status}`);
  const document = await discovery.json();
  if (!document.issuer) die("the discovery document names no issuer");
  // Printed alone on stdout: the caller puts this in the trust entry.
  process.stdout.write(`${document.issuer}\n`);
}

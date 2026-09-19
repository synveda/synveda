#!/usr/bin/env node
// OPS-11: opt-in, disposable identities and data; every product operation uses the public API.
import { readFileSync, writeFileSync } from "node:fs";
import { randomBytes, randomUUID } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

const { APP, AUTH, ADMIN_URL } = process.env;
const phase = process.argv[2];
let stage = "initialization";
const check = (condition, message) => { if (!condition) throw new Error(message); };
const save = (name, value) => writeFileSync(`/work/${name}`, value, { mode: 0o600 });
const password = () => randomBytes(32).toString("hex");
const marker = "The starter release train leaves on Thursday at 14:00 UTC.";
const query = "starter release train Thursday";
let state = phase === "seed" ? { users: {} } : JSON.parse(readFileSync("/work/state.json", "utf8"));

async function request(url, init = {}, statuses = [200, 201, 204]) {
  const response = await fetch(url, { ...init, redirect: "manual", signal: AbortSignal.timeout(30000) });
  // Never report a body or full URL: either can contain an invitation or credential.
  const route = new URL(url).pathname.replace(/\/invites\/[^/]+\/accept/, "/invites/:token/accept");
  check(statuses.includes(response.status), `unexpected HTTP ${response.status} on ${init.method ?? "GET"} ${route}`);
  const raw = await response.text();
  return { status: response.status, body: raw ? JSON.parse(raw) : null, headers: response.headers };
}
async function api(token, path, method = "GET", body, statuses) {
  const headers = { "Idempotency-Key": randomUUID() };
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers["Content-Type"] = "application/json";
  return request(`${APP}${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) }, statuses);
}
const json = async (...args) => (await api(...args)).body;
const denied = (...args) => api(...args, [401, 403, 404]);
const jwt = (token) => JSON.parse(Buffer.from(token.split(".")[1], "base64url").toString());

async function login(user, consoleFlow = false) {
  const jars = new Map();
  async function hop(url, init = {}) {
    const origin = new URL(url).origin;
    const jar = jars.get(origin) ?? new Map();
    const headers = new Headers(init.headers);
    if (jar.size) headers.set("Cookie", [...jar].map(([k, v]) => `${k}=${v}`).join("; "));
    const response = await fetch(url, { ...init, headers, redirect: "manual", signal: AbortSignal.timeout(30000) });
    for (const cookie of response.headers.getSetCookie()) {
      const pair = cookie.split(";", 1)[0], at = pair.indexOf("=");
      jar.set(pair.slice(0, at), pair.slice(at + 1));
    }
    jars.set(origin, jar);
    return response;
  }
  const begin = await hop(`${APP}/auth/login${consoleFlow ? "?console=true" : ""}`);
  const authorize = begin.headers.get("location");
  check(authorize?.startsWith(`${AUTH}/realms/synveda/protocol/openid-connect/auth?`), "PKCE issuer redirect missing");
  const params = new URL(authorize).searchParams;
  for (const key of ["state", "nonce", "code_challenge"]) check(params.get(key), `PKCE ${key} missing`);
  check(params.get("code_challenge_method") === "S256" && params.get("redirect_uri") === `${APP}/auth/callback`, "PKCE binding incorrect");
  const form = await hop(authorize);
  check(form.status === 200, "identity login page unavailable");
  const html = await form.text();
  const tag = [...html.matchAll(/<form\b[^>]*>/gi)].find((m) => /\bid=["']kc-form-login["']/i.test(m[0]))?.[0];
  const action = tag?.match(/\baction=["']([^"']+)["']/i)?.[1]?.replaceAll("&amp;", "&").replaceAll("&#x3D;", "=").replaceAll("&#61;", "=").replaceAll("&quot;", '"');
  check(action?.startsWith(`${AUTH}/realms/synveda/login-actions/authenticate?`), "bounded identity login form missing");
  const submitted = await hop(action, { method: "POST", body: new URLSearchParams({ username: user.username, password: user.password, credentialId: "" }) });
  const callback = submitted.headers.get("location");
  check(callback?.startsWith(`${APP}/auth/callback?`) && !/access_token|refresh_token/.test(callback), "application callback missing");
  const completed = await hop(callback);
  if (consoleFlow) {
    check(completed.status === 307 && completed.headers.get("location") === "/console/", "console cookie handoff failed");
    const me = await hop(`${APP}/v1/me`);
    check(me.status === 200 && (await me.json()).principal.subject === user.subject, "console API identity mismatch");
    return;
  }
  check(completed.status === 200, `login callback HTTP ${completed.status}`);
  const session = await completed.json();
  const claims = jwt(session.access_token);
  check(claims.iss === `${AUTH}/realms/synveda` && claims.sub === session.subject && session.subject === user.id, "stable issuer/subject mismatch");
  check(claims.exp - claims.iat <= 300, "human access-token lifetime exceeded five minutes");
  if (user.subject) check(session.subject === user.subject && session.identity.id === user.identity, "identity changed after restart");
  Object.assign(user, { subject: session.subject, identity: session.identity.id, token: session.access_token });
}

async function adminSession() {
  return (await request(`${ADMIN_URL}/realms/master/protocol/openid-connect/token`, {
    method: "POST", body: new URLSearchParams({ grant_type: "password", client_id: "admin-cli", username: readFileSync("/admin/username", "utf8"), password: readFileSync("/admin/password", "utf8") }),
  })).body.access_token;
}
async function admin(token, path, method = "GET", body) {
  return request(`${ADMIN_URL}/admin/realms/synveda${path}`, { method, headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
}
async function agentToken() {
  const result = (await request(`${AUTH}/realms/synveda/protocol/openid-connect/token`, { method: "POST", body: new URLSearchParams({ grant_type: "client_credentials", client_id: "starter-agent", client_secret: state.agent.secret }) })).body;
  const claims = jwt(result.access_token);
  check(claims.iss === `${AUTH}/realms/synveda` && claims.sub === state.agent.subject && claims.aud === "synveda-agents", "service token binding incorrect");
  check(claims.exp - claims.iat <= 300 && !result.refresh_token, "service token expiry/refresh contract incorrect");
  state.agent.token = result.access_token;
  save("agent-token", result.access_token);
}
function knowledge(project) {
  return { scope_id: project.scope_id, project_id: project.id, knowledge_type: "convention", content: { title: "Starter release train", body_markdown: marker, summary: marker, tags: ["starter-acceptance"], sensitivity: "internal", confidence_permille: 940 } };
}
const openSession = (token, workspace = state.workspace, project = state.project) => json(token, "/v1/sessions", "POST", { workspace_id: workspace.id, project_id: project.id, client_name: "mcp", external_session_id: randomUUID(), task_summary: "Isolated starter acceptance" });
const containsMarker = (value) => JSON.stringify(value).includes(marker);
async function noWorkspaceAccess(token, workspace, project, item) {
  await denied(token, `/v1/workspaces/${workspace.id}`, "GET", undefined);
  await denied(token, `/v1/projects/${project.id}`, "GET", undefined);
  await denied(token, `/v1/skills/available?scope_id=${project.scope_id}`, "GET", undefined);
  await denied(token, "/v1/sessions", "POST", { workspace_id: workspace.id, project_id: project.id, client_name: "mcp" });
  if (item) await denied(token, `/v1/knowledge/${item}`, "GET", undefined);
  const search = await api(token, `/v1/knowledge?query=Thursday&workspace_id=${workspace.id}`, "GET", undefined, [200, 403, 404]);
  check(!containsMarker(search.body), "search disclosed forbidden workspace content");
}
async function usableTeam() {
  for (const name of ["owner", "member", "viewer", "stranger"]) await login(state.users[name]);
  await login(state.users.owner, true);
  for (const name of ["owner", "member", "viewer"]) {
    const token = state.users[name].token;
    check(containsMarker(await json(token, `/v1/knowledge/${state.knowledge}`)), `${name} cannot read persisted content`);
    check(containsMarker(await json(token, `/v1/knowledge?query=Thursday&workspace_id=${state.workspace.id}`)), `${name} cannot search persisted content`);
  }
  await noWorkspaceAccess(state.users.stranger.token, state.workspace, state.project, state.knowledge);
  await noWorkspaceAccess(state.users.member.token, state.foreignWorkspace, state.foreignProject, state.foreignKnowledge);
  await denied(state.users.viewer.token, "/v1/knowledge", "POST", knowledge(state.project));
  await agentToken();
  check(containsMarker(await json(state.agent.token, `/v1/sessions/${state.agent.session}/knowledge-query`, "POST", { query })), "service recall omitted content");
  check(containsMarker(await json(state.users.member.token, `/v1/sessions/${state.memberSession}/context-runs`, "POST", { query })), "member context omitted content");
  await noWorkspaceAccess(state.agent.token, state.foreignWorkspace, state.foreignProject, state.foreignKnowledge);
  const audit = await json(state.users.owner.token, "/v1/audit/verify");
  check(audit.valid && audit.events > 0, "audit chain verification failed");
}

try {
  check(APP?.startsWith("https://") && AUTH?.startsWith("https://") && ADMIN_URL?.startsWith("http://keycloak-http."), "fixture endpoints invalid");
  if (phase === "seed") {
    stage = "public administration and management refusal";
    for (const url of [`${APP}/metrics`, `${APP}/healthz`, `${AUTH}/admin/`, `${AUTH}/realms/master`, `${AUTH}/metrics`, `${AUTH}/health`]) {
      const response = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(10000) });
      check(response.status === 404, "public route exposed a private endpoint");
    }
    stage = "private administration and empty realm";
    const administrator = await adminSession();
    const users = (await admin(administrator, "/users?max=20")).body;
    check(users.length === 0, "packaged realm shipped users");
    const bootstrap = (await admin(administrator, "/groups?search=synveda-admins&exact=true")).body.find((g) => g.name === "synveda-admins");
    check(bootstrap, "explicit bootstrap group missing");
    for (const name of ["owner", "member", "viewer", "stranger"]) {
      const user = { username: `acceptance-${name}`, password: password() };
      const created = await admin(administrator, "/users", "POST", { username: user.username, enabled: true, firstName: "Acceptance", lastName: name, email: `${name}@fixture.invalid`, emailVerified: true, credentials: [{ type: "password", value: user.password, temporary: false }] });
      user.id = created.headers.get("location").split("/").at(-1);
      if (name === "owner") await admin(administrator, `/users/${user.id}/groups/${bootstrap.id}`, "PUT");
      state.users[name] = user;
    }
    stage = "explicit owner bootstrap";
    await api(null, "/v1/workspaces", "POST", { slug: "forbidden", display_name: "Forbidden" }, [401]);
    await login(state.users.stranger);
    check(!(await json(state.users.stranger.token, "/v1/me")).capabilities.role_keys.includes("administrator"), "first authenticated stranger became administrator");
    await login(state.users.owner);
    const owner = state.users.owner.token;
    const ownerMe = await json(owner, "/v1/me");
    check(ownerMe.capabilities.role_keys.includes("administrator"), "designated owner did not bootstrap");
    state.tenantScope = ownerMe.onboarding.tenant_scope_id;
    await admin(administrator, `/users/${state.users.owner.id}/groups/${bootstrap.id}`, "DELETE");
    await admin(administrator, `/users/${state.users.stranger.id}/groups/${bootstrap.id}`, "PUT");
    await login(state.users.stranger);
    check(!(await json(state.users.stranger.token, "/v1/me")).capabilities.role_keys.includes("administrator"), "later bootstrap group recreated administrator authority");
    await admin(administrator, `/users/${state.users.stranger.id}/groups/${bootstrap.id}`, "DELETE");
    stage = "workspace and governed team configuration";
    state.workspace = await json(owner, "/v1/workspaces", "POST", { slug: "starter-team", display_name: "Starter acceptance team" });
    state.project = await json(owner, `/v1/workspaces/${state.workspace.id}/projects`, "POST", { slug: "release", display_name: "Release" });
    const teamTemplate = (await json(owner, "/v1/configuration-templates")).templates.find((t) => t.name === "team");
    const config = await json(owner, "/v1/configurations", "POST", { governing_scope_id: state.tenantScope, name: "starter-team", source_template: "team", document: teamTemplate.document });
    check(config.outcome === "applied", "initial configuration requires a governed review");
    const binding = await json(owner, "/v1/configuration-bindings", "POST", { scope_id: state.tenantScope, artifact_id: config.artifact_id, enabled: true });
    check(binding.outcome === "applied", "initial configuration binding requires a governed review");
    stage = "member admission and viewer";
    await login(state.users.member); await login(state.users.viewer);
    await noWorkspaceAccess(state.users.member.token, state.workspace, state.project);
    const invite = await json(owner, `/v1/workspaces/${state.workspace.id}/invites`, "POST", { role: "member", expires_in_secs: 600 });
    const accepted = await json(state.users.member.token, `/v1/invites/${invite.token}/accept`, "POST");
    state.memberGrant = accepted.grant.id;
    await json(owner, "/v1/admin/grants", "POST", { principal_id: state.users.viewer.subject, role: "viewer", scope_id: state.workspace.scope_id });
    stage = "governed content and foreign workspace";
    const published = await json(owner, "/v1/knowledge", "POST", knowledge(state.project));
    check(published.outcome === "applied", "owner publication requires a governed review");
    state.knowledge = published.knowledge_item_id;
    state.foreignWorkspace = await json(owner, "/v1/workspaces", "POST", { slug: "other-team", display_name: "Isolated second team" });
    state.foreignProject = await json(owner, `/v1/workspaces/${state.foreignWorkspace.id}/projects`, "POST", { slug: "private", display_name: "Private" });
    const foreign = await json(owner, "/v1/knowledge", "POST", knowledge(state.foreignProject));
    check(foreign.outcome === "applied", "foreign publication requires a governed review");
    state.foreignKnowledge = foreign.knowledge_item_id;
    state.memberSession = (await openSession(state.users.member.token)).id;
    stage = "scoped service credentials";
    const createdClient = await admin(administrator, "/clients", "POST", { clientId: "starter-agent", enabled: true, protocol: "openid-connect", publicClient: false, serviceAccountsEnabled: true, standardFlowEnabled: false, directAccessGrantsEnabled: false, fullScopeAllowed: false, defaultClientScopes: ["basic"], optionalClientScopes: [], attributes: { "access.token.lifespan": "300" }, protocolMappers: [{ name: "synveda-agents", protocol: "openid-connect", protocolMapper: "oidc-audience-mapper", config: { "included.custom.audience": "synveda-agents", "access.token.claim": "true", "id.token.claim": "false" } }] });
    const client = createdClient.headers.get("location").split("/").at(-1);
    state.agent = { client, secret: (await admin(administrator, `/clients/${client}/client-secret`)).body.value, subject: (await admin(administrator, `/clients/${client}/service-account-user`)).body.id };
    await agentToken();
    await api(state.agent.token, "/v1/me", "GET", undefined, [401]);
    const service = await json(owner, "/v1/service-identities", "POST", { subject: state.agent.subject, scope_id: state.workspace.scope_id, display_name: "Starter acceptance agent" });
    state.agent.identity = service.id;
    await json(owner, "/v1/admin/grants", "POST", { principal_id: state.agent.subject, role: "member", scope_id: state.project.scope_id });
    state.agent.session = (await openSession(state.agent.token)).id;
    save("agent-session", state.agent.session);
    stage = "all read and denial surfaces";
    await usableTeam();
    console.log(JSON.stringify({ privateEndpointsDenied: true, ownerBootstrap: true, strangerNotAdministrator: true, laterBootstrapGroupDenied: true, consoleCookie: true, memberInvitation: true, viewer: true, governedPublication: true, foreignWorkspaceDenied: true, serviceTokenSeconds: 300, unregisteredServiceDenied: true }));
  } else if (phase === "verify") {
    stage = "persisted identities and content";
    await usableTeam();
    console.log(JSON.stringify({ stableSubjects: true, freshLogin: true, persistentContent: true, authorizedRecall: true }));
  } else if (phase === "load") {
    stage = "four concurrent agents and ingestion";
    await agentToken();
    const started = Date.now();
    const sessions = await Promise.all(Array.from({ length: 4 }, async (_, agent) => {
      const session = await openSession(state.agent.token);
      for (let batch = 0; batch < 20; batch++) {
        await json(state.agent.token, `/v1/sessions/${session.id}/events`, "POST", { events: Array.from({ length: 5 }, (_, event) => ({ event_type: "message.user", client_event_id: `${agent}-${batch}-${event}`, occurred_at: new Date().toISOString(), payload: { text: `Remember: starter agent ${agent} validates release batch ${batch}, event ${event}.` } })) });
        await json(state.agent.token, `/v1/sessions/${session.id}/context-runs`, "POST", { query });
        await sleep(500);
      }
      await json(state.agent.token, `/v1/sessions/${session.id}/end`, "POST", { status: "ended" });
      return session.id;
    }));
    let captured = 0, capturedEvents = 0, candidates = 0;
    for (let attempt = 0; attempt < 90 && captured < sessions.length; attempt++) {
      const batches = await json(state.users.owner.token, `/v1/capture-batches?project_id=${state.project.id}&limit=100`);
      const completed = batches.batches.filter((b) => sessions.includes(b.session_id) && b.state === "completed");
      captured = sessions.filter((id) => completed.some((b) => b.session_id === id && b.candidate_count > 0)).length;
      capturedEvents = completed.reduce((total, b) => total + b.event_count, 0);
      candidates = completed.reduce((total, b) => total + b.candidate_count, 0);
      if (captured < sessions.length) await sleep(1000);
    }
    check(captured === 4 && capturedEvents >= 400, "worker did not complete Capture for four ended sessions");
    console.log(JSON.stringify({ concurrentAgents: 4, sessions: 4, events: 400, contextRuns: 80, capturedSessions: captured, capturedEvents, candidates, durationMs: Date.now() - started }));
  } else if (phase === "revoke") {
    stage = "membership role change";
    const owner = state.users.owner.token, member = state.users.member.token;
    await api(owner, `/v1/admin/grants/${state.memberGrant}`, "DELETE");
    const viewer = await json(owner, "/v1/admin/grants", "POST", { principal_id: state.users.member.subject, role: "viewer", scope_id: state.workspace.scope_id });
    check(containsMarker(await json(member, `/v1/knowledge/${state.knowledge}`)), "viewer role change lost reads");
    await denied(member, "/v1/knowledge", "POST", knowledge(state.project));
    stage = "membership revocation with unchanged bearer";
    const started = Date.now();
    await api(owner, `/v1/admin/grants/${viewer.id}`, "DELETE");
    await noWorkspaceAccess(member, state.workspace, state.project, state.knowledge);
    await denied(member, `/v1/sessions/${state.memberSession}/context-runs`, "POST", { query });
    await denied(member, `/v1/sessions/${state.memberSession}/knowledge-query`, "POST", { query });
    await login(state.users.member);
    await noWorkspaceAccess(state.users.member.token, state.workspace, state.project, state.knowledge);
    stage = "service revocation with unchanged bearer";
    await api(owner, `/v1/service-identities/${state.agent.identity}`, "DELETE");
    await api(state.agent.token, `/v1/sessions/${state.agent.session}/knowledge-query`, "POST", { query }, [401]);
    const audit = await json(owner, "/v1/audit/verify");
    check(audit.valid && audit.events > 0, "post-revocation audit chain verification failed");
    console.log(JSON.stringify({ memberToViewer: true, unchangedBearerDenied: true, memberReloginDenied: true, serviceBearerDenied: true, auditValid: true, auditEvents: audit.events, observedChecksMs: Date.now() - started }));
  } else throw new Error("unknown fixture phase");
  save("state.json", JSON.stringify(state));
} catch (error) {
  save("state.json", JSON.stringify(state));
  console.error(`Starter ${phase} failed at ${stage}: ${error.message}`);
  process.exitCode = 1;
}

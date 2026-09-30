#!/usr/bin/env node
// OPS-7: observe authorization, not just compilation, on three gateway pods.
import { randomBytes, randomUUID } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as sleep } from "node:timers/promises";
import { accessToken, loginApprover } from "./login-cross-pod.mjs";

const run = promisify(execFile);
const tenant = "019b53c0-7c00-7000-8000-000000000002";
const suffix = randomBytes(5).toString("hex");
const name = `ops7-decision-${suffix}`;
const pods = JSON.parse(process.env.POD_URLS ?? "null");
if (!Array.isArray(pods) || pods.length !== 3 || new Set(pods).size !== 3 ||
    !pods.every((pod) => /^http:\/\/\d{1,3}(?:\.\d{1,3}){3}:8120$/.test(pod))) {
  throw new Error("three distinct gateway pod URLs are required");
}
const check = (value, message) => { if (!value) throw new Error(message); };

async function api(index, path, method = "GET", body, token = accessToken) {
  const response = await fetch(`${pods[index]}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      "Idempotency-Key": randomUUID(),
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(5_000),
  });
  const raw = await response.text();
  return { status: response.status, body: raw ? JSON.parse(raw) : null };
}

async function applied(index, path, method = "GET", body, token = accessToken) {
  const response = await api(index, path, method, body, token);
  check([200, 201].includes(response.status), `${method} ${path} returned HTTP ${response.status}`);
  return response.body;
}

async function governed(path, method, body, approverToken) {
  const opened = await applied(0, path, method, body);
  check(opened.change_id, `governed ${method} ${path} returned no change`);
  if (opened.outcome === "applied") return opened;
  check(opened.outcome === "pending_review", `governed ${method} ${path} was rejected`);
  const detail = await applied(0, `/v1/proposals/${opened.change_id}`, "GET", undefined, approverToken);
  check(detail.commit, "Configuration review has no commit");
  const approval = await applied(0, `/v1/proposals/${opened.change_id}/approve`, "POST", {
    expected_commit: detail.commit,
  }, approverToken);
  check(approval.state === "approved", `Configuration review still needs ${approval.outstanding}`);
  const result = await applied(0, `/v1/proposals/${opened.change_id}/apply`, "POST");
  check(result.outcome === "applied", "approved Configuration change was not applied");
  return result;
}

async function installedCount(pod) {
  const response = await fetch(`${pod}/metrics`, { signal: AbortSignal.timeout(3_000) });
  check(response.ok, "one gateway metrics endpoint was unavailable");
  const line = (await response.text()).split("\n").find((value) =>
    /^synveda_policy_pack_reloads_total\{outcome="installed"\} /.test(value));
  return line ? Number(line.split(" ").at(-1)) : 0;
}

async function policyCommand(args) {
  try {
    return await run("/usr/local/bin/synveda", ["policy", ...args], {
      timeout: 15_000, maxBuffer: 8_192,
    });
  } catch {
    throw new Error("audited policy test command failed");
  }
}

async function applyPack(file, version) {
  const baseline = await Promise.all(pods.map(installedCount));
  const { stdout } = await policyCommand([
    "apply", "--tenant", tenant, "--name", name, `/scripts/${file}`,
  ]);
  let stored;
  try { stored = JSON.parse(stdout); } catch { throw new Error("policy command returned no revision"); }
  check(stored.name === name && stored.version === version, "policy revision did not advance exactly");
  const committedAt = Date.parse(stored.updated_at);
  check(Number.isFinite(committedAt), "policy revision carried no database timestamp");
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const counts = await Promise.all(pods.map(installedCount));
    if (counts.every((count, index) => count > baseline[index])) return committedAt;
    await sleep(250);
  }
  throw new Error("one gateway did not compile the policy revision within 30 seconds");
}

let binding;
let approverGrant;
try {
  await applyPack("decision-allow.cedar", 1);

  const me = await applied(0, "/v1/me");
  const tenantScope = me.onboarding?.tenant_scope_id;
  check(tenantScope, "demo administrator has no tenant scope");
  const templates = await applied(0, "/v1/configuration-templates");
  const teamDocument = templates.templates.find((template) => template.name === "team")?.document;
  check(teamDocument, "team Configuration template was absent");
  const rootEffective = await applied(0, `/v1/configurations/effective?scope_id=${tenantScope}`);
  if (rootEffective.fail_safe) {
    // A fresh tenant starts with regulated-strict. Exercise its documented
    // one-time canonical team adoption before this small-team review probe.
    const first = await applied(0, "/v1/configurations", "POST", {
      governing_scope_id: tenantScope, name: `ops7-team-${suffix}`,
      source_template: "team", document: teamDocument,
    });
    check(first.outcome === "applied" && first.artifact_id, "canonical first Configuration was not applied");
    const firstBinding = await applied(0, "/v1/configuration-bindings", "POST", {
      scope_id: tenantScope, artifact_id: first.artifact_id, enabled: true,
    });
    check(firstBinding.outcome === "applied", "canonical first binding was not applied");
  } else {
    check(rootEffective.document?.policy_pack === "standard", "probe requires a standard team tenant");
  }
  const workspace = await applied(0, "/v1/workspaces", "POST", {
    slug: `ops7-${suffix}`, display_name: "OPS-7 decision probe",
  });
  const project = await applied(0, `/v1/workspaces/${workspace.id}/projects`, "POST", {
    slug: "decision", display_name: "OPS-7 decision probe",
  });
  const approver = await loginApprover();
  await applied(0, "/v1/me", "GET", undefined, approver.token);
  approverGrant = await applied(0, "/v1/admin/grants", "POST", {
    principal_id: approver.subject, role: "administrator", scope_id: project.scope_id,
  });
  check(approverGrant.id, "reviewer project grant was not created");
  const document = structuredClone(teamDocument);
  document.policy_pack = name;
  const config = await governed("/v1/configurations", "POST", {
    governing_scope_id: project.scope_id, name, document,
  }, approver.token);
  check(config.outcome === "applied" && config.artifact_id, "configuration creation was not applied");
  binding = await governed("/v1/configuration-bindings", "POST", {
    scope_id: project.scope_id, artifact_id: config.artifact_id, enabled: true,
  }, approver.token);
  check(binding.outcome === "applied" && binding.binding_id && binding.binding_revision,
    "configuration binding was not applied");

  for (let index = 0; index < pods.length; index++) {
    const effective = await applied(index, `/v1/configurations/effective?scope_id=${project.scope_id}`);
    check(effective.document?.policy_pack === name, `pod ${index + 1} did not read the binding`);
    const permitted = await api(index, `/v1/admin/scopes/${project.scope_id}`, "PATCH", {
      display_name: `OPS-7 permitted ${index + 1}`,
    });
    check(permitted.status === 200, `pod ${index + 1} refused a permitted update (${permitted.status})`);
  }

  // The next revision removes ScopeUpdate. Probe decisions directly during
  // convergence; every pre-denial 200 is an audited ordinary API mutation.
  const baseline = await Promise.all(pods.map(installedCount));
  const { stdout } = await policyCommand([
    "apply", "--tenant", tenant, "--name", name, "/scripts/decision-deny.cedar",
  ]);
  let stored;
  try { stored = JSON.parse(stdout); } catch { throw new Error("policy command returned no revision"); }
  check(stored.name === name && stored.version === 2, "restrictive revision did not advance exactly");
  const committedAt = Date.parse(stored.updated_at);
  check(Number.isFinite(committedAt), "restrictive revision carried no database timestamp");
  const observed = Array(3).fill(null);
  const deadline = Date.now() + 30_000;
  while (observed.some((value) => value === null) && Date.now() < deadline) {
    for (let index = 0; index < pods.length; index++) {
      if (observed[index] !== null) continue;
      const result = await api(index, `/v1/admin/scopes/${project.scope_id}`, "PATCH", {
        display_name: `OPS-7 pending ${index + 1}`,
      });
      if (result.status === 403) {
        check(result.body?.reason?.includes(`${name}@2`), `pod ${index + 1} denied for another reason`);
        observed[index] = Date.now() - committedAt;
      } else {
        check(result.status === 200, `pod ${index + 1} returned HTTP ${result.status} during convergence`);
      }
    }
    if (observed.some((value) => value === null)) await sleep(1_000);
  }
  check(observed.every((value) => value !== null && value >= 0),
    "a gateway kept permitting ScopeUpdate 30 seconds after the stored revision");
  const counts = await Promise.all(pods.map(installedCount));
  check(counts.every((count, index) => count > baseline[index]),
    "a restrictive decision arrived without the expected local compile metric");
  for (let index = 0; index < pods.length; index++) {
    const result = await api(index, `/v1/admin/scopes/${project.scope_id}`, "PATCH", {
      display_name: `OPS-7 refused again ${index + 1}`,
    });
    check(result.status === 403 && result.body?.reason?.includes(`${name}@2`),
      `pod ${index + 1} permitted after it observed denial`);
  }
  console.log(`OPS-7: three pods changed ScopeUpdate from 200 to 403; maximum observed DB-timestamp-to-denial lag ${Math.max(...observed)} ms`);
} finally {
  let bindingDisabled = !binding;
  if (binding) {
    try {
      const disabled = await applied(0, `/v1/configuration-bindings/${binding.binding_id}`, "PATCH", {
        expected_revision: binding.binding_revision,
        artifact_id: binding.artifact_id,
        enabled: false,
        reason: "OPS-7 disposable probe complete",
      });
      check(disabled.outcome === "applied", "test binding disable was not applied");
      bindingDisabled = true;
    } catch {
      console.error("OPS-7: disposable Configuration binding could not be disabled");
    }
  }
  if (approverGrant && bindingDisabled) {
    const revoked = await api(0, `/v1/admin/grants/${approverGrant.id}`, "DELETE");
    check(revoked.status === 204, "disposable project review grant was not revoked");
  }
  // The immutable Configuration version remains a rollback target, so the
  // product deliberately refuses to clear its referenced policy pack.
  check(bindingDisabled, "disposable Configuration binding remains active");
}

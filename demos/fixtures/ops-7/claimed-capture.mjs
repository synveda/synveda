#!/usr/bin/env node
// OPS-7: establish and verify a fenced Capture claim through the public API.
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { accessToken } from "./login-cross-pod.mjs";

const pods = JSON.parse(process.env.POD_URLS ?? "null");
const phase = process.argv[2];
if (!Array.isArray(pods) || pods.length !== 3 ||
    !["start", "pre-release", "verify", "start-three", "pre-release-three", "verify-three",
      "start-provider-failure", "pre-release-provider-failure", "verify-provider-failure"].includes(phase)) {
  throw new Error("three gateway URLs and a supported phase are required");
}
const providerFailure = phase.endsWith("-provider-failure");
const batchCount = phase.endsWith("-three") ? 3 : 1;
const action = phase.replace(/-(three|provider-failure)$/, "");
const claimFile = providerFailure ? "/work/provider-failure.json" : "/work/claim.json";
const check = (condition, message) => { if (!condition) throw new Error(message); };
const provider = "http://127.0.0.1:8088";

async function request(url, init = {}) {
  return fetch(url, { ...init, signal: AbortSignal.timeout(5_000) });
}

async function api(path, method = "GET", body) {
  const headers = { Authorization: `Bearer ${accessToken}` };
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    headers["Idempotency-Key"] = randomUUID();
  }
  const response = await request(`${pods[0]}${path}`, {
    method, headers, body: body === undefined ? undefined : JSON.stringify(body),
  });
  check([200, 201, 204].includes(response.status),
    `${method} ${path.split("?")[0]} returned HTTP ${response.status}`);
  return response.status === 204 ? null : response.json();
}

async function stats() {
  const response = await request(`${provider}/stats`);
  check(response.status === 200, "blocked extractor stats unavailable");
  return response.json();
}

if (action === "start") {
  if (providerFailure) {
    const before = await stats();
    check(before.calls === 4 && before.active === 0 && before.cancelled === 1,
      "three-worker pod-loss drill did not finish before provider-failure drill");
    for (const endpoint of ["block", "fail-next"]) {
      const response = await request(`${provider}/${endpoint}`, { method: "POST" });
      check(response.status === 200, `provider ${endpoint} fault could not be armed`);
    }
  }
  const slug = `ops7-claim-${randomUUID().slice(0, 12)}`;
  const workspace = await api("/v1/workspaces", "POST", { slug, display_name: "OPS-7 claimed work" });
  const project = await api(`/v1/workspaces/${workspace.id}/projects`, "POST",
    { slug: "capture", display_name: "Capture" });
  const sessionIds = [];
  for (let index = 0; index < batchCount; index++) {
    const session = await api("/v1/sessions", "POST", {
      workspace_id: workspace.id, project_id: project.id,
      client_name: "ops7-worker-recovery", external_session_id: randomUUID(),
    });
    await api(`/v1/sessions/${session.id}/events`, "POST", { events: [{
      event_type: "message.user", client_event_id: randomUUID(),
      occurred_at: new Date().toISOString(),
      payload: { text: `Remember: fenced Capture claim ${index} must recover once.` },
    }] });
    await api(`/v1/sessions/${session.id}/end`, "POST", { status: "ended" });
    sessionIds.push(session.id);
  }

  let batches = [];
  for (let attempt = 0; attempt < 60; attempt++) {
    const rows = await api(`/v1/capture-batches?project_id=${project.id}&limit=100`);
    batches = sessionIds.map(id => rows.batches.find(entry => entry.session_id === id));
    const providerState = await stats();
    if (batches.every(batch => batch?.state === "running" &&
          batch.attempts === (providerFailure ? 2 : 1)) &&
        providerState.calls === (providerFailure ? 6 : batchCount) &&
        providerState.active === batchCount) break;
    await sleep(500);
  }
  const providerState = await stats();
  check(batches.every(batch => batch?.state === "running" &&
        batch.attempts === (providerFailure ? 2 : 1)) &&
        providerState.calls === (providerFailure ? 6 : batchCount) &&
        providerState.active === batchCount &&
        (!providerFailure || providerState.failed === 1),
    "Capture batches were not separately claimed and blocked");
  if (batchCount === 3) {
    check(new Set(providerState.peers).size === 3, "three workers did not make distinct provider calls");
  }
  writeFileSync(claimFile, JSON.stringify({ batchIds: batches.map(batch => batch.id) }),
    { mode: 0o600, flag: "wx" });
  console.log(providerFailure ?
    "OPS-7: provider returned 503 once; a different Capture attempt reached the blocked retry" :
    batchCount === 3 ? "OPS-7: three Capture claims blocked on three distinct worker calls" :
      "OPS-7: one Capture claim blocked on one worker call");
} else if (action === "pre-release") {
  const { batchIds } = JSON.parse(readFileSync(claimFile, "utf8"));
  check(batchIds.length === batchCount, "Capture claim count changed");
  const batches = await Promise.all(batchIds.map(id => api(`/v1/capture-batches/${id}`)));
  const attempts = batches.map(batch => batch.attempts).sort().join(",");
  if (batchCount === 3) {
    check(batches.filter(batch => batch.state === "completed" && batch.attempts === 1 &&
          batch.candidate_count === 1).length === 2 &&
          batches.filter(batch => batch.state === "running" && batch.attempts === 2 &&
            batch.candidate_count === 0).length === 1 && attempts === "1,1,2",
      "surviving calls did not complete while the fenced recovery stayed uncommitted");
  } else {
    check(batches.every(batch => batch.state === "running" && batch.candidate_count === 0) &&
          attempts === "2", "Capture committed candidates before the fenced retry completed");
  }
  console.log("OPS-7: only the expected candidates exist before recovery release");
} else {
  const { batchIds } = JSON.parse(readFileSync(claimFile, "utf8"));
  check(batchIds.length === batchCount, "Capture claim count changed");
  let batches = [];
  for (let attempt = 0; attempt < 120; attempt++) {
    batches = await Promise.all(batchIds.map(id => api(`/v1/capture-batches/${id}`)));
    if (batches.every(batch => batch.state === "completed")) break;
    await sleep(500);
  }
  const attempts = batches.map(batch => batch.attempts).sort().join(",");
  check(batches.every(batch => batch.state === "completed" && batch.candidate_count === 1) &&
        attempts === (batchCount === 3 ? "1,1,2" : "2"),
    "Capture batches did not complete once after fenced recovery");
  for (const batchId of batchIds) {
    const candidates = await api(`/v1/capture-candidates?batch_id=${batchId}&limit=100`);
    check(candidates.candidates.length === 1, "Capture persisted duplicate candidates");
  }
  const providerState = await stats();
  check(providerState.calls === (providerFailure ? 6 : batchCount + 1) &&
        providerState.cancelled === 1 && providerState.active === 0 &&
        (!providerFailure || providerState.failed === 1),
    "extractor did not see exactly one cancelled call and one completed retry");
  console.log(`OPS-7: ${batchCount} Capture batch${batchCount === 1 ? "" : "es"} completed with one candidate each; attempts=${attempts}, provider_calls=${providerState.calls}, cancelled=1`);
}

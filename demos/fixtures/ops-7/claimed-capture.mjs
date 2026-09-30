#!/usr/bin/env node
// OPS-7: establish and verify a fenced Capture claim through the public API.
import { randomUUID } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { accessToken } from "./login-cross-pod.mjs";

const pods = JSON.parse(process.env.POD_URLS ?? "null");
const phase = process.argv[2];
if (!Array.isArray(pods) || pods.length !== 3 || !["start", "pre-release", "verify"].includes(phase)) {
  throw new Error("three gateway URLs and a supported phase are required");
}
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

if (phase === "start") {
  const slug = `ops7-claim-${randomUUID().slice(0, 12)}`;
  const workspace = await api("/v1/workspaces", "POST", { slug, display_name: "OPS-7 claimed work" });
  const project = await api(`/v1/workspaces/${workspace.id}/projects`, "POST",
    { slug: "capture", display_name: "Capture" });
  const session = await api("/v1/sessions", "POST", {
    workspace_id: workspace.id, project_id: project.id,
    client_name: "ops7-worker-expiry", external_session_id: randomUUID(),
  });
  await api(`/v1/sessions/${session.id}/events`, "POST", { events: [{
    event_type: "message.user", client_event_id: randomUUID(),
    occurred_at: new Date().toISOString(),
    payload: { text: "Remember: an expired policy lease cancels claimed Capture work." },
  }] });
  await api(`/v1/sessions/${session.id}/end`, "POST", { status: "ended" });

  let batch;
  for (let attempt = 0; attempt < 60; attempt++) {
    const rows = await api(`/v1/capture-batches?project_id=${project.id}&limit=100`);
    batch = rows.batches.find((entry) => entry.session_id === session.id);
    const providerState = await stats();
    if (batch?.state === "running" && batch.attempts === 1 &&
        providerState.calls === 1 && providerState.active === 1) break;
    await sleep(500);
  }
  check(batch?.state === "running" && batch.attempts === 1, "Capture was not claimed once");
  check((await stats()).active === 1, "first extractor request was not blocked");
  writeFileSync("/work/claim.json", JSON.stringify({ batchId: batch.id }), { mode: 0o600, flag: "wx" });
  console.log("OPS-7: claimed Capture and blocked extractor ready");
} else if (phase === "pre-release") {
  const { batchId } = JSON.parse(readFileSync("/work/claim.json", "utf8"));
  const batch = await api(`/v1/capture-batches/${batchId}`);
  check(batch.state === "running" && batch.attempts === 2 && batch.candidate_count === 0,
    "Capture committed candidates before the fenced retry completed");
  console.log("OPS-7: retry is claimed and no candidate was committed before provider release");
} else {
  const { batchId } = JSON.parse(readFileSync("/work/claim.json", "utf8"));
  let batch;
  for (let attempt = 0; attempt < 120; attempt++) {
    batch = await api(`/v1/capture-batches/${batchId}`);
    if (batch.state === "completed") break;
    await sleep(500);
  }
  check(batch?.state === "completed" && batch.attempts === 2 &&
        batch.candidate_count === 1, "Capture did not complete once on its fenced retry");
  const candidates = await api(`/v1/capture-candidates?batch_id=${batchId}&limit=100`);
  check(candidates.candidates.length === 1, "Capture persisted duplicate candidates");
  const providerState = await stats();
  check(providerState.calls === 2 && providerState.cancelled === 1 && providerState.active === 0,
    "extractor did not see exactly one cancelled call and one completed retry");
  console.log("OPS-7: claimed Capture recovered once; attempts=2, candidates=1, provider_calls=2, cancelled=1");
}

#!/usr/bin/env node
// OPS-7: measure three local compiles after an audited test-pack revision.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { setTimeout as sleep } from "node:timers/promises";

const run = promisify(execFile);
const tenant = "019b53c0-7c00-7000-8000-000000000002";
const name = "ops7-lag";
const pods = JSON.parse(process.env.POD_URLS ?? "null");
if (!Array.isArray(pods) || pods.length !== 3 || new Set(pods).size !== 3 ||
    !pods.every((pod) => /^http:\/\/\d{1,3}(?:\.\d{1,3}){3}:8120$/.test(pod))) {
  throw new Error("three distinct gateway pod URLs are required");
}
const check = (value, message) => { if (!value) throw new Error(message); };

async function installedCount(pod) {
  const response = await fetch(`${pod}/metrics`, { signal: AbortSignal.timeout(3_000) });
  check(response.ok, "one gateway metrics endpoint was unavailable");
  const metrics = await response.text();
  const line = metrics.split("\n").find((value) =>
    /^synveda_policy_pack_reloads_total\{outcome="installed"\} /.test(value));
  return line ? Number(line.split(" ").at(-1)) : 0;
}

async function ready(pod) {
  const response = await fetch(`${pod}/readyz`, { signal: AbortSignal.timeout(3_000) });
  check(response.status === 200, "one gateway was not ready during policy convergence");
}

async function policyCommand(args) {
  try {
    return await run("/usr/local/bin/synveda", ["policy", ...args], {
      timeout: 15_000,
      maxBuffer: 8_192,
    });
  } catch {
    throw new Error("audited policy test command failed");
  }
}

let applied = false;
try {
  await policyCommand(["clear", "--tenant", tenant, "--name", name]);
  for (let revision = 1; revision <= 2; revision++) {
    const baseline = await Promise.all(pods.map(installedCount));
    applied = true;
    const { stdout } = await policyCommand([
      "apply", "--tenant", tenant, "--name", name, "/scripts/lag-pack.cedar",
    ]);
    let stored;
    try { stored = JSON.parse(stdout); } catch { throw new Error("policy command returned no revision"); }
    check(stored.name === name && stored.version === revision, "policy revision did not advance exactly");
    const committedAt = Date.parse(stored.updated_at);
    check(Number.isFinite(committedAt), "policy revision carried no database timestamp");

    const observed = Array(3).fill(null);
    const deadline = Date.now() + 30_000;
    while (observed.some((value) => value === null) && Date.now() < deadline) {
      const counts = await Promise.all(pods.map(installedCount));
      for (let index = 0; index < pods.length; index++) {
        if (observed[index] === null && counts[index] > baseline[index]) {
          observed[index] = Date.now() - committedAt;
        }
      }
      if (observed.some((value) => value === null)) await sleep(250);
    }
    check(observed.every((value) => value !== null && value >= 0), "a gateway did not compile the stored revision within 30 seconds of the database timestamp");
    await Promise.all(pods.map(ready));
    console.log(`OPS-7: revision ${revision} compiled on three pods; maximum observed DB-timestamp-to-metric lag ${Math.max(...observed)} ms`);
  }
} finally {
  if (applied) await policyCommand(["clear", "--tenant", tenant, "--name", name]);
}

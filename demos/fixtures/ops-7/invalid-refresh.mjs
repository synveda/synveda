#!/usr/bin/env node
// OPS-7: isolate failed Cedar compilation from the database-authority gate.
import { setTimeout as sleep } from "node:timers/promises";
import { accessToken } from "./login-cross-pod.mjs";

const pods = JSON.parse(process.env.POD_URLS ?? "null");
if (!Array.isArray(pods) || pods.length !== 3 || new Set(pods).size !== 3 ||
    !pods.every((pod) => /^http:\/\/\d{1,3}(?:\.\d{1,3}){3}:8120$/.test(pod))) {
  throw new Error("three distinct gateway pod URLs are required");
}
const check = (value, message) => { if (!value) throw new Error(message); };

async function metrics(pod) {
  const response = await fetch(`${pod}/metrics`, { signal: AbortSignal.timeout(3_000) });
  check(response.status === 200, "gateway metrics endpoint was unavailable");
  const lines = (await response.text()).split("\n");
  const number = (name) => {
    const line = lines.find((value) => value.startsWith(`${name} `));
    return line ? Number(line.split(" ").at(-1)) : 0;
  };
  return {
    authorityReady: number("synveda_gateway_authority_ready"),
    authorityUnavailable: number('synveda_gateway_authority_checks_total{outcome="unavailable"}') +
      number('synveda_gateway_authority_checks_total{outcome="timeout"}'),
    refreshError: number('synveda_policy_pack_refresh_sweeps_total{outcome="error"}'),
    reloadError: number('synveda_policy_pack_reloads_total{outcome="error"}'),
  };
}

async function sample(pod) {
  const [ready, app] = await Promise.all([
    fetch(`${pod}/readyz`, { signal: AbortSignal.timeout(3_000) }),
    fetch(`${pod}/v1/me`, {
      headers: { Authorization: `Bearer ${accessToken}` },
      signal: AbortSignal.timeout(3_000),
    }),
  ]);
  check([200, 503].includes(ready.status), `readiness returned HTTP ${ready.status}`);
  check([200, 503].includes(app.status), `authenticated read returned HTTP ${app.status}`);
  return { ready: ready.status, app: app.status };
}

const baseline = await Promise.all(pods.map(sample));
check(baseline.every((value) => value.ready === 200 && value.app === 200),
  "three gateway pods were not serving before the invalid test pack");
const metricBaseline = await Promise.all(pods.map(metrics));
check(metricBaseline.every((value) => value.authorityReady === 1),
  "database authority was not ready before the failed refresh");
const started = Date.now();
console.log("OPS-7: baseline ready");

const closedAt = Array(3).fill(null);
const recoveredAt = Array(3).fill(null);
let samples = 0;
while (Date.now() - started < 65_000 && recoveredAt.some((value) => value === null)) {
  const states = await Promise.all(pods.map(sample));
  samples += states.length;
  for (let index = 0; index < pods.length; index++) {
    const { ready, app } = states[index];
    if (closedAt[index] === null && ready === 503 && app === 503) {
      const current = await metrics(pods[index]);
      check(current.authorityReady === 1,
        `pod ${index + 1} lost database authority during the policy-only fault`);
      closedAt[index] = Date.now() - started;
    }
    if (closedAt[index] !== null && recoveredAt[index] === null) {
      if (ready === 200 && app === 200) {
        check(Date.now() - started >= 42_000,
          `pod ${index + 1} reopened before the invalid test pack was cleared`);
        recoveredAt[index] = Date.now() - started;
      } else if (ready === 503 && app === 200) {
        check(Date.now() - started >= 42_000,
          `pod ${index + 1} served authenticated traffic with stale policy`);
      } else {
        check(ready === 503 && app === 503 || ready === 200 && app === 503,
          `pod ${index + 1} returned inconsistent admission after expiry`);
      }
    }
  }
  if (recoveredAt.some((value) => value === null)) await sleep(250);
}
check(closedAt.every((value) => value !== null && value <= 36_000),
  "a gateway did not close within the provisional 30-second policy lease plus test start margin");
check(recoveredAt.every((value) => value !== null),
  "a gateway did not reopen after the invalid test pack was cleared");
const metricAfter = await Promise.all(pods.map(metrics));
check(metricAfter.every((value, index) =>
  value.authorityReady === 1 &&
  value.authorityUnavailable === metricBaseline[index].authorityUnavailable &&
  value.refreshError > metricBaseline[index].refreshError &&
  value.reloadError > metricBaseline[index].reloadError),
"all pods must retain database authority and record failed policy sweeps");
console.log(`OPS-7: ${samples} paired authenticated/readiness samples across three pods; policy-only failure closed all by ${Math.max(...closedAt)} ms and recovered all by ${Math.max(...recoveredAt)} ms from baseline`);

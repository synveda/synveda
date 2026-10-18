/**
 * Selected-target forecasts (CPR-6): a missing `/v1/me` anchor is a question
 * for the probe, never a denial, and the probe's own answer — including a
 * restrictive child policy, a dropped scope or a failure — stays distinct.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import type { Entry } from "./cache.mjs";
import { forecastAt, forecastKey, summary, unansweredScopes } from "./forecast.mjs";
import type { AnchorCapabilities, MeView } from "./generated/api.js";

function me(subject = "robin", tenant = "tenant-1"): MeView {
  const workspaceAnchor: AnchorCapabilities = {
    scope_id: "scope-workspace",
    kind: "workspace",
    source: "grant",
    direct: true,
    roles: ["member"],
    actions: { "knowledge.write": true, "tool.write": false },
  };
  return {
    principal: { subject, quarantined: false },
    tenant: { id: tenant, slug: "t", name: "T", status: "active" },
    anchors: [workspaceAnchor],
  } as unknown as MeView;
}

const loading: Entry = { status: "loading" };
const failed: Entry = {
  status: "ready",
  outcome: { kind: "unavailable", message: "down" },
  loadedAt: 1,
};
function probe(body: unknown): Entry {
  return { status: "ready", outcome: { kind: "ok", body }, loadedAt: 1 };
}

test("only scopes without an answering anchor are probed, once each and in order", () => {
  const view = me();
  assert.deepEqual(
    unansweredScopes(view, ["scope-project", "scope-workspace", null, "scope-project", undefined], [
      "knowledge.write",
    ]),
    ["scope-project"],
  );
  // An anchor carries no tiered read, so asking one sends that scope too.
  assert.deepEqual(unansweredScopes(view, ["scope-workspace"], ["knowledge.read"]), [
    "scope-workspace",
  ]);
});

test("an anchor answers at its own scope and says nothing about its descendants", () => {
  const view = me();
  assert.equal(forecastAt(view, loading, "scope-workspace", "knowledge.write"), "allowed");
  assert.equal(forecastAt(view, loading, "scope-workspace", "tool.write"), "denied");
  // The inherited project has no anchor: it is waiting for the probe.
  assert.equal(forecastAt(view, loading, "scope-project", "knowledge.write"), "loading");
});

test("the probe's own verdict wins at the child, including a stricter child policy", () => {
  const view = me();
  const answered = probe({
    capabilities: [
      {
        scope_id: "scope-project",
        actions: { "knowledge.write": false, "skill.write": true },
        read_tiers: { "knowledge.read": ["public", "internal"], "skill.read": [] },
      },
    ],
    max_scopes: 128,
  });
  assert.equal(forecastAt(view, answered, "scope-project", "knowledge.write"), "denied");
  assert.equal(forecastAt(view, answered, "scope-project", "skill.write"), "allowed");
  assert.equal(forecastAt(view, answered, "scope-project", "knowledge.read"), "allowed");
  assert.equal(forecastAt(view, answered, "scope-project", "skill.read"), "denied");
  assert.equal(forecastAt(view, answered, "scope-project", "no.such.action"), "unanswered");
});

test("loading, failure and a dropped scope are not denials", () => {
  const view = me();
  assert.equal(forecastAt(view, failed, "scope-project", "knowledge.write"), "failed");
  const bounded = probe({ capabilities: [], not_answered: ["scope-project"], max_scopes: 128 });
  assert.equal(forecastAt(view, bounded, "scope-project", "knowledge.write"), "unanswered");
  assert.equal(
    forecastAt(view, probe({ capabilities: [] }), "scope-project", "knowledge.write"),
    "unanswered",
  );
});

test("a probe is cached per account and tenant, never shared across them", () => {
  const scopes = ["scope-project"];
  assert.notEqual(forecastKey(me("robin"), scopes), forecastKey(me("sam"), scopes));
  assert.notEqual(forecastKey(me("robin", "t-1"), scopes), forecastKey(me("robin", "t-2"), scopes));
  assert.ok(forecastKey(me(), scopes).startsWith("capabilities/"), "Refresh access clears it");
});

test("a status line reports what is unsettled before what is settled", () => {
  assert.equal(summary(["allowed", "failed"]), "failed");
  assert.equal(summary(["denied", "unanswered"]), "unanswered");
  assert.equal(summary(["denied", "loading"]), "loading");
  assert.equal(summary(["denied", "allowed"]), "allowed");
  assert.equal(summary(["denied"]), "denied");
  assert.equal(summary([]), "denied");
});

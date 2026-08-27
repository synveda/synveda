/**
 * The route table and its gating (CPR-8, ADR-0075 decision 1).
 *
 * Two things are asserted here and neither is decoration. The **shape of
 * the product** — which items are primary, which are advanced, and in what
 * order — is what this feature delivers, so a reordering or a quiet
 * demotion should fail a test rather than ship. And the **capability names**
 * are strings shared with `synveda_policy::Action::as_str()`: a drift there
 * silently empties the advanced menu for everybody, which is the worst
 * possible failure mode because it looks exactly like "you are not an
 * administrator".
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  BASE,
  ROUTES,
  advancedNav,
  hrefOf,
  matchRoute,
  offersRoute,
  primaryNav,
  routeOf,
} from "./routes.mjs";

test("the primary navigation is the product, in this order, for everybody", () => {
  assert.deepEqual(
    primaryNav().map((route) => route.label),
    [
      "Home",
      "Sessions",
      "Knowledge",
      "New Learnings",
      "Import / Export",
      "Skills",
      "Tools",
      "People",
      "Settings",
    ],
  );
  // No primary route is gated. This is the rule that keeps the shape of the
  // application the same for every reader (see the module note in routes.mts).
  for (const route of primaryNav()) {
    assert.equal(route.capability, undefined, `${route.id} gates the primary menu`);
    assert.ok(offersRoute(route, {}), `${route.id} is hidden from a caller with no capabilities`);
  }
});

test("the advanced navigation is governance, and every item is gated", () => {
  const advanced = ROUTES.filter((route) => route.group === "advanced");
  assert.deepEqual(
    advanced.map((route) => [route.label, route.capability]),
    [
      ["Reviews", "proposal.read"],
      ["Scopes", "scope.read"],
      ["Configuration", "configuration.read"],
      ["Audit", "audit.read"],
      ["Service identities", "service_identity.read"],
    ],
  );
});

test("a caller with no capabilities is offered no advanced page at all", () => {
  assert.deepEqual(advancedNav({}), []);
  // And an explicit denial is the same as an absent one: `false` is the
  // PDP's answer, not a missing key, and both mean no.
  assert.deepEqual(advancedNav({ "proposal.read": false, "audit.read": false }), []);
});

test("the advanced menu carries exactly the planes the forecast offers", () => {
  const labels = advancedNav({ "proposal.read": true, "audit.read": true }).map(
    (route) => route.label,
  );
  assert.deepEqual(labels, ["Reviews", "Audit"]);
});

test("welcome is reachable but never in a menu", () => {
  // Onboarding is somewhere you are sent, not somewhere you choose — and a
  // menu item for it would still be sitting there after it was finished.
  const welcome = routeOf("welcome");
  assert.equal(welcome.group, "none");
  assert.deepEqual(matchRoute("/console/welcome"), { id: "welcome", params: {} });
  assert.ok(!primaryNav().includes(welcome));
  assert.ok(!advancedNav({}).includes(welcome));
});

test("paths round-trip, with and without their trailing slash", () => {
  for (const route of ROUTES) {
    // A parameterised route needs a value for every placeholder; anything
    // will do, because what is asserted is that the two functions agree.
    const params = Object.fromEntries(
      route.segment
        .split("/")
        .filter((part) => part.startsWith(":"))
        .map((part) => [part.slice(1), "abc-123"]),
    );
    const href = hrefOf(route.id, params);
    assert.deepEqual(matchRoute(href), { id: route.id, params }, href);
    assert.deepEqual(matchRoute(`${href}/`), { id: route.id, params }, `${href}/`);
  }
  assert.deepEqual(matchRoute(BASE), { id: "home", params: {} });
  assert.deepEqual(matchRoute(`${BASE}/`), { id: "home", params: {} });
});

test("a run has its own address, and the id in it is the id it renders", () => {
  // The whole reason routing grew a parameter (CPR-11): a run somebody is
  // investigating has to survive a refresh and paste into a ticket.
  const href = hrefOf("session", { session_id: "018f-abc" });
  assert.equal(href, "/console/sessions/018f-abc");
  assert.deepEqual(matchRoute(href), { id: "session", params: { session_id: "018f-abc" } });

  // A value with a slash in it is encoded on the way out and decoded on the
  // way back, so a round trip is lossless rather than a second route.
  const odd = hrefOf("session", { session_id: "a/b" });
  assert.equal(odd, "/console/sessions/a%2Fb");
  assert.deepEqual(matchRoute(odd), { id: "session", params: { session_id: "a/b" } });

  // The literal listing still wins over the pattern, and the pattern does not
  // swallow a deeper path.
  assert.deepEqual(matchRoute("/console/sessions"), { id: "sessions", params: {} });
  assert.equal(matchRoute("/console/sessions/a/b"), null);

  // A link nothing filled is a loud failure rather than a `:session_id` in
  // the DOM that 404s on click.
  assert.throws(() => hrefOf("session"), /session_id/);
});

test("a context run has one linkable inspector address", () => {
  const href = hrefOf("context-run", { context_run_id: "018f-context" });
  assert.equal(href, "/console/context-runs/018f-context");
  assert.deepEqual(matchRoute(href), {
    id: "context-run",
    params: { context_run_id: "018f-context" },
  });
  assert.equal(routeOf("context-run").group, "none");
  assert.throws(() => hrefOf("context-run"), /context_run_id/);
});

test("a Knowledge item has a stable address", () => {
  const href = hrefOf("knowledge-item", { knowledge_id: "018f-knowledge" });
  assert.equal(href, "/console/knowledge/018f-knowledge");
  assert.deepEqual(matchRoute(href), {
    id: "knowledge-item",
    params: { knowledge_id: "018f-knowledge" },
  });
  assert.throws(() => hrefOf("knowledge-item"), /knowledge_id/);
});

test("a Skill aggregate has a stable library address", () => {
  const href = hrefOf("skill-item", { skill_id: "018f-skill" });
  assert.equal(href, "/console/skills/018f-skill");
  assert.deepEqual(matchRoute(href), {
    id: "skill-item",
    params: { skill_id: "018f-skill" },
  });
  assert.equal(routeOf("skill-item").group, "none");
  assert.throws(() => hrefOf("skill-item"), /skill_id/);
});

test("an MCP server has a stable catalogue address", () => {
  const href = hrefOf("tool-server", { server_id: "018f-server" });
  assert.equal(href, "/console/tools/018f-server");
  assert.deepEqual(matchRoute(href), {
    id: "tool-server",
    params: { server_id: "018f-server" },
  });
  assert.equal(routeOf("tool-server").group, "none");
  assert.throws(() => hrefOf("tool-server"), /server_id/);
});

test("a path this console does not have matches nothing", () => {
  // `null` is rendered as a not-found page rather than redirected: the
  // gateway answers every path under the prefix with the bundle, so a typo
  // arrives here, and landing silently on Home would tell somebody their
  // link worked.
  assert.equal(matchRoute("/console/nope"), null);
  assert.equal(matchRoute("/console/advanced"), null);
  assert.equal(matchRoute("/console/advanced/nope"), null);
  // Nothing outside the prefix is ours, including a lookalike.
  assert.equal(matchRoute("/v1/me"), null);
  assert.equal(matchRoute("/consoleish/people"), null);
});

test("a malformed encoded route parameter is not a router crash", () => {
  for (const pathname of [
    "/console/sessions/%",
    "/console/context-runs/%E0%A4%A",
    "/console/knowledge/%C0%AF",
  ]) {
    assert.doesNotThrow(() => matchRoute(pathname), pathname);
    assert.equal(matchRoute(pathname), null, pathname);
  }
});

test("the advanced routes live under one prefix, and the primary ones do not", () => {
  // Not cosmetic: the segment is what a bookmark and a shared link carry,
  // and "advanced/" is what makes a governance URL legible as one.
  for (const route of ROUTES) {
    if (route.group === "advanced") {
      assert.ok(route.segment.startsWith("advanced/"), route.id);
    } else {
      assert.ok(!route.segment.startsWith("advanced/"), route.id);
    }
  }
});

test("every route has a blurb, and no two share a path", () => {
  const segments = new Set<string>();
  for (const route of ROUTES) {
    assert.ok(route.blurb.length > 0, `${route.id} has no blurb`);
    assert.ok(!segments.has(route.segment), `${route.segment} is claimed twice`);
    segments.add(route.segment);
  }
});

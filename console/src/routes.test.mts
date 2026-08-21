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
      ["Policies", "policy.read"],
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
  assert.equal(matchRoute("/console/welcome"), "welcome");
  assert.ok(!primaryNav().includes(welcome));
  assert.ok(!advancedNav({}).includes(welcome));
});

test("paths round-trip, with and without their trailing slash", () => {
  for (const route of ROUTES) {
    const href = hrefOf(route.id);
    assert.equal(matchRoute(href), route.id, href);
    assert.equal(matchRoute(`${href}/`), route.id, `${href}/`);
  }
  assert.equal(matchRoute(BASE), "home");
  assert.equal(matchRoute(`${BASE}/`), "home");
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

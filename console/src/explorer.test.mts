/**
 * The explorer's judgements (CNSL-2, ADR-0058).
 *
 * The pure half, tested here; the rendered half is asserted against the
 * parity corpus beside the CLI's, which is what keeps the two surfaces
 * naming the same facts about the same payload (decision 10).
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  deniedCount,
  describeEnd,
  lapsesTouching,
  mayDo,
  mayRead,
  offers,
  type Capabilities,
  type Lapse,
} from "./explorer.mjs";

const HERE = "0199aa11-1111-7111-8111-111111111111";
const ABOVE = "0199aa11-2222-7222-8222-222222222222";

function capabilities(overrides: Partial<Capabilities> = {}): Capabilities {
  return {
    scope_id: HERE,
    scope_path: "acme/eng/platform",
    pack: { name: "regulated-strict", version: 3, origin: { kind: "assigned", scope_id: ABOVE } },
    roles: ["viewer"],
    actions: { "proposal.read": true, "proposal.review": false, "policy.assign": false },
    read_tiers: { "memory.read": ["public", "internal"], "skill.read": [] },
    ...overrides,
  };
}

test("the capability summaries split allowed from denied", () => {
  const caps = capabilities();
  assert.deepEqual(mayDo(caps), ["proposal.read"]);
  assert.equal(deniedCount(caps), 2);
});

test("a tiered read with no permitted tier is not listed as readable", () => {
  // An empty tier list is a real answer — "nothing here, at any tier" — and
  // rendering the action name with an empty value would read as a partial
  // permission rather than as none.
  assert.deepEqual(mayRead(capabilities()), [["memory.read", ["public", "internal"]]]);
});

test("offers is false for a probe that never arrived", () => {
  // Fail closed. An unreachable PDP shows a reviewer no buttons rather than
  // buttons that will not work — and this is the only function in the
  // bundle that reads a capability answer, so it is the only place the
  // forecast rule can be got wrong.
  assert.equal(offers(null, "proposal.review"), false);
  assert.equal(offers(capabilities(), "proposal.review"), false);
  assert.equal(
    offers(capabilities({ actions: { "proposal.review": true } }), "proposal.review"),
    true,
  );
});

test("an action the probe never asked about is not offered", () => {
  // A vocabulary skew — a console older than its gateway — must not read an
  // absent key as permission.
  assert.equal(offers(capabilities(), "something.new"), false);
});

function lapse(overrides: Partial<Lapse> = {}): Lapse {
  return {
    id: "0199bb11-1111-7111-8111-111111111111",
    grantee_scope_id: HERE,
    target_scope_id: ABOVE,
    action: "memory.read",
    reason: "joint incident review",
    granted_at: "2026-08-01T09:00:00Z",
    expires_at: "2026-09-01T09:00:00Z",
    granted_by: "0199bb11-1111-7111-8111-111111111112",
    proposal_id: "0199bb11-1111-7111-8111-111111111113",
    outcome: "active",
    ...overrides,
  };
}

test("a scope sees the grants at both of its ends", () => {
  // ADR-0058 decision 7 in the renderer: a grant is as much a fact about
  // the team that received it as about the team that disclosed, and a view
  // showing only the target end tells a granted team nothing is happening.
  const receiving = lapse();
  const disclosing = lapse({ id: "b", grantee_scope_id: ABOVE, target_scope_id: HERE });
  const elsewhere = lapse({ id: "c", grantee_scope_id: ABOVE, target_scope_id: ABOVE });
  const touching = lapsesTouching([receiving, disclosing, elsewhere], HERE);
  assert.deepEqual(
    touching.map((entry) => entry.id),
    [receiving.id, "b"],
  );
});

test("an end the reader may not read shows an abbreviated id and no path", () => {
  // The gateway omits the path for an end this caller cannot read, so a
  // grant visible from one end never discloses where the other end sits.
  assert.equal(describeEnd("acme/eng/platform", HERE), "acme/eng/platform");
  const hidden = describeEnd(undefined, HERE);
  assert.match(hidden, /^«/);
  assert.equal(hidden.includes("/"), false, `no path leaked: ${hidden}`);
});

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
  mayDo,
  mayRead,
  offers,
  relaxationsAt,
  type Capabilities,
  type Relaxation,
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

function relaxation(overrides: Partial<Relaxation> = {}): Relaxation {
  return {
    id: "0199bb11-1111-7111-8111-111111111111",
    governing_scope_id: HERE,
    current_version_id: "0199bb11-1111-7111-8111-111111111112",
    revision: 1,
    status: "active",
    current: {
      id: "0199bb11-1111-7111-8111-111111111112",
      relaxation_id: "0199bb11-1111-7111-8111-111111111111",
      ordinal: 1,
      change_id: "0199bb11-1111-7111-8111-111111111113",
      subject_identity_id: "0199bb11-1111-7111-8111-111111111114",
      subject: "alice",
      target_scope_id: HERE,
      action: "knowledge.read",
      max_sensitivity: "internal",
      requested_start_at: "2026-08-01T09:00:00Z",
      requested_end_at: "2026-09-01T09:00:00Z",
      effective_start_at: "2026-08-01T09:00:00Z",
      hard_expires_at: "2026-09-01T09:00:00Z",
      reason: "joint incident review",
      configuration_hash: "a".repeat(64),
      content_hash: "b".repeat(64),
      creator_id: "0199bb11-1111-7111-8111-111111111114",
      approver_ids: [],
      auto_applied: true,
      created_at: "2026-08-01T09:00:00Z",
    },
    created_at: "2026-08-01T09:00:00Z",
    created_by: "0199bb11-1111-7111-8111-111111111114",
    updated_at: "2026-08-01T09:00:00Z",
    updated_by: "0199bb11-1111-7111-8111-111111111114",
    ...overrides,
  };
}

test("a scope shows only relaxations governed at that exact target", () => {
  const here = relaxation();
  const elsewhere = relaxation({ id: "c", governing_scope_id: ABOVE });
  const touching = relaxationsAt([here, elsewhere], HERE);
  assert.deepEqual(
    touching.map((entry) => entry.id),
    [here.id],
  );
});

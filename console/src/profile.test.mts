/**
 * The organisation's profile choice (ADR-0162).
 *
 * The copy is part of the decision: the screen must say who approves a later
 * change under each profile, that one administrator cannot change
 * Configuration under Team, and, before Enterprise is confirmed, that leaving
 * it needs four people. The adoption itself is two ordinary governed calls at
 * the tenant root, reported exactly as the server decided them.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import type { TenantProfileView } from "./generated/api.js";
import {
  PROFILE_CHOICES,
  adoptProfile,
  profileArtifactName,
  profileChoosable,
  profileStatus,
} from "./profile.mjs";

const OPEN: TenantProfileView = { state: "open", policy_pack: "regulated-strict" };

test("the three canonical templates are offered with their packs and nothing that brands", () => {
  assert.deepEqual(
    PROFILE_CHOICES.map((choice) => [choice.template, choice.pack]),
    [
      ["personal", "open-collaboration"],
      ["team", "standard"],
      ["enterprise", "regulated-strict"],
    ],
  );
  for (const choice of PROFILE_CHOICES) {
    assert.match(choice.approvals, /approv/, `${choice.template} says who approves`);
    for (const forbidden of ["kind", "edition", "tier", "plan", "workspaceKind"]) {
      assert.ok(!(forbidden in choice), `a profile must not carry ${forbidden}`);
    }
  }
});

test("the screen says what each profile costs before it is chosen", () => {
  const [personal, team, enterprise] = PROFILE_CHOICES;
  assert.match(personal?.approvals ?? "", /their own Configuration change/);
  assert.match(team?.approvals ?? "", /one administrator other than its author/);
  assert.match(team?.approvals ?? "", /until you add a second/);
  assert.match(enterprise?.approvals ?? "", /two administrators other than its author/);
  assert.match(enterprise?.warning ?? "", /Leaving Enterprise later needs that four-person review/);
  assert.equal(personal?.warning, undefined);
  assert.equal(team?.warning, undefined);
});

test("only the server's open and prepared states offer the choice", () => {
  assert.equal(profileChoosable(OPEN), true);
  assert.equal(profileChoosable({ ...OPEN, state: "prepared", template: "team", artifact_id: "a" }), true);
  assert.equal(profileChoosable({ ...OPEN, state: "unavailable" }), false);
  assert.equal(profileChoosable({ state: "chosen", template: "team", policy_pack: "standard" }), false);
  assert.equal(profileChoosable(null), false);
  assert.equal(profileChoosable(undefined), false);
});

test("a reader who cannot choose is told what the organisation runs", () => {
  assert.match(
    profileStatus({ state: "chosen", template: "team", policy_pack: "standard" }) ?? "",
    /uses the Team profile \(standard\)\. Changing it goes through review under standard/,
  );
  assert.match(
    profileStatus({ state: "chosen", policy_pack: "standard" }) ?? "",
    /runs standard/,
  );
  assert.match(
    profileStatus({ state: "unavailable", policy_pack: "regulated-strict" }) ?? "",
    /has not chosen a profile, so it runs regulated-strict, the default/,
  );
  assert.equal(profileStatus(OPEN), null);
  assert.equal(profileStatus(null), null);
});

type Seen = { method: string; url: string; body: Record<string, unknown> | null };

function gateway(answers: Record<string, () => Response>): { fetch: typeof fetch; seen: Seen[] } {
  const seen: Seen[] = [];
  const fake: typeof fetch = async (input, init) => {
    const method = init?.method ?? "GET";
    const url = String(input);
    seen.push({
      method,
      url,
      body: typeof init?.body === "string" ? (JSON.parse(init.body) as Record<string, unknown>) : null,
    });
    const answer = answers[`${method} ${url}`];
    if (!answer) throw new Error(`unexpected ${method} ${url}`);
    return answer();
  };
  return { fetch: fake, seen };
}

const json = (status: number, body: unknown) => () =>
  new Response(JSON.stringify(body), { status });

const TEMPLATES = json(200, {
  templates: [
    { name: "personal", document: { policy_pack: "open-collaboration" } },
    { name: "team", document: { policy_pack: "standard" } },
    { name: "enterprise", document: { policy_pack: "regulated-strict" } },
  ],
});

test("adopting creates the canonical template at the tenant root and binds it there", async () => {
  const { fetch, seen } = gateway({
    "GET /v1/configuration-templates": TEMPLATES,
    "POST /v1/configurations": json(201, { change_id: "c1", outcome: "applied", artifact_id: "a1", version_id: "v1" }),
    "POST /v1/configuration-bindings": json(201, { change_id: "c2", outcome: "applied", binding_id: "b1" }),
  });
  const outcome = await adoptProfile(OPEN, "root-scope", "team", fetch);
  assert.deepEqual(outcome, { kind: "applied", what: "The Team profile for your organisation" });
  assert.deepEqual(seen[1]?.body, {
    governing_scope_id: "root-scope",
    name: profileArtifactName("team"),
    source_template: "team",
    document: { policy_pack: "standard" },
  });
  assert.deepEqual(seen[2]?.body, { scope_id: "root-scope", artifact_id: "a1", enabled: true });
});

test("a create sent to review is reported, and nothing is bound behind it", async () => {
  const { fetch, seen } = gateway({
    "GET /v1/configuration-templates": TEMPLATES,
    "POST /v1/configurations": json(201, { change_id: "review-1", outcome: "pending_review", artifact_id: "a1" }),
  });
  const outcome = await adoptProfile(OPEN, "root-scope", "personal", fetch);
  assert.deepEqual(outcome, {
    kind: "pending",
    what: "The Personal profile for your organisation",
    changeId: "review-1",
  });
  assert.equal(seen.length, 2, "no binding is attempted for an unapplied artifact");
});

test("a prepared adoption is finished by binding the artifact the server named", async () => {
  const { fetch, seen } = gateway({
    "POST /v1/configuration-bindings": json(201, { change_id: "c2", outcome: "applied", binding_id: "b1" }),
  });
  const prepared: TenantProfileView = {
    state: "prepared",
    template: "enterprise",
    artifact_id: "earlier",
    policy_pack: "regulated-strict",
  };
  const outcome = await adoptProfile(prepared, "root-scope", "enterprise", fetch);
  assert.equal(outcome.kind, "applied");
  assert.deepEqual(seen.map((call) => `${call.method} ${call.url}`), ["POST /v1/configuration-bindings"]);
  assert.deepEqual(seen[0]?.body, { scope_id: "root-scope", artifact_id: "earlier", enabled: true });
});

test("a refusal names why and never claims the profile", async () => {
  const { fetch } = gateway({
    "GET /v1/configuration-templates": TEMPLATES,
    "POST /v1/configurations": json(403, { message: "the pack in force refuses this" }),
  });
  const outcome = await adoptProfile(OPEN, "root-scope", "team", fetch);
  assert.deepEqual(outcome, {
    kind: "refused",
    what: "The Team profile for your organisation",
    why: "the pack in force refuses this",
  });
});

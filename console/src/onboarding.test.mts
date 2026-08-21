/**
 * First-run onboarding's model (CPR-8, ADR-0075 decision 6).
 *
 * Three things are asserted, and the first is the important one: that the
 * personal/team choice **seeds** and does not brand. ADR-0068 decision 1 is
 * locked — one domain model, one runtime, no edition conditionals — and the
 * friendliest possible door for that branch to arrive through is a wizard
 * asking "is this just you?". So the plan it produces is a pack name and a
 * boolean about invitations, and there is nowhere for a `kind` to be
 * written.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  CHECK_CANNOT,
  CHECK_COVERS,
  CLIENTS,
  STEPS,
  STEP_COUNT,
  checkVerdict,
  clientOf,
  connectionSteps,
  nextStep,
  seedPlan,
  seedSentence,
  slugFrom,
  stepNumber,
} from "./onboarding.mjs";

test("the shape is a seeding plan and carries no edition", () => {
  const personal = seedPlan("personal");
  const team = seedPlan("team");
  assert.deepEqual(Object.keys(personal).sort(), ["invitesMembers", "pack", "summary"]);
  // Two fields that seed, and nothing that brands: no `kind`, no `edition`,
  // no `tier`. A field like that is the branch ADR-0068 decision 1 forbids,
  // arriving through the friendliest possible door.
  for (const plan of [personal, team]) {
    for (const forbidden of ["kind", "edition", "tier", "plan", "workspaceKind"]) {
      assert.ok(!(forbidden in plan), `a seeding plan must not carry ${forbidden}`);
    }
  }
  assert.notEqual(personal.pack, team.pack, "the two choices seed different policy");
  assert.equal(personal.invitesMembers, false);
  assert.equal(team.invitesMembers, true);
});

test("both shapes seed a pack this build actually ships", () => {
  // The names are `synveda_policy::EMBEDDED_PACKS`'. A drift here is a
  // wizard that assigns a pack the gateway refuses, on the very first act
  // somebody takes.
  const shipped = ["standard", "regulated-strict", "open-collaboration"];
  assert.ok(shipped.includes(seedPlan("personal").pack));
  assert.ok(shipped.includes(seedPlan("team").pack));
});

test("a refused seeding step names where to finish the job", () => {
  // A refusal with no next step is a dead end, and the next step genuinely
  // exists — the workspace works meanwhile under whatever it inherits.
  const sentence = seedSentence({
    kind: "refused",
    what: "Assigning the standard policy pack",
    why: "you hold no role that permits policy.assign here",
  });
  assert.match(sentence, /was refused/);
  assert.match(sentence, /Advanced ▸ Policies/);
  assert.match(sentence, /Your workspace works/);
});

test("an applied seeding step says so plainly", () => {
  assert.match(seedSentence({ kind: "applied", what: "The standard policy pack" }), /done/);
  assert.match(seedSentence({ kind: "skipped", what: "A group" }), /not needed/);
});

test("the six steps run in order and end at done", () => {
  assert.deepEqual(
    [...STEPS],
    ["workspace", "project", "repository", "client", "instructions", "check", "done"],
  );
  assert.equal(STEP_COUNT, 6);
  assert.equal(nextStep("workspace"), "project");
  assert.equal(nextStep("check"), "done");
  assert.equal(nextStep("done"), "done", "done is terminal");
  assert.equal(stepNumber("workspace"), 1);
  assert.equal(stepNumber("check"), 6);
});

test("the plugin client gets the plugin commands and an MCP client gets its own", () => {
  const claudeCode = clientOf("claude-code");
  assert.deepEqual(connectionSteps(claudeCode, "https://synveda.example"), [
    "synveda login --gateway https://synveda.example",
    "synveda plugin install",
  ]);
  const cursor = clientOf("cursor");
  assert.deepEqual(connectionSteps(cursor, "http://127.0.0.1:8120"), [
    "synveda login --gateway http://127.0.0.1:8120",
    "synveda mcp install --client cursor",
  ]);
});

test("an unknown client falls through to the extensible one rather than to a guess", () => {
  // Seed §2 principle 6: the harness is a guest, and supporting a new one
  // must never require touching the core — so the last entry is "any other
  // MCP client" and it points at the CLI's own config file.
  const unknown = clientOf("some-editor-nobody-has-shipped-yet");
  assert.equal(unknown.id, "other");
  assert.match(connectionSteps(unknown, "http://x")[1] ?? "", /--client <your-client>/);
});

test("every listed client has an id the CLI would accept", () => {
  // The ids are pasted into `synveda mcp install --client <id>`, which
  // reads `crates/synveda-cli/src/mcp/clients.jsonc`.
  const known = ["claude-code", "cursor", "claude-desktop", "zed", "other"];
  assert.deepEqual(
    CLIENTS.map((client) => client.id),
    known,
  );
});

test("the check passes without a repository and fails without a readable project", () => {
  // Attaching a repository is a step of the wizard because a project
  // usually is about one — but a project that is not is a legitimate
  // project, and an agent is about to name the *project*.
  const noRepo = checkVerdict({ projectReadable: true, repositoryCount: 0 });
  assert.equal(noRepo.kind, "pass");
  assert.ok(noRepo.lines.some((line) => line.includes("no repository attached")));

  const unreadable = checkVerdict({
    projectReadable: false,
    projectWhy: "not this tenant's project",
    repositoryCount: 2,
  });
  assert.equal(unreadable.kind, "fail");
  assert.equal(unreadable.kind === "fail" ? unreadable.why : "", "not this tenant's project");
});

test("the check says what it cannot prove", () => {
  // The browser is confined to this origin by the console's own CSP, so it
  // cannot reach a process on the reader's machine — and a green tick that
  // implied otherwise would be the worst kind.
  assert.ok(CHECK_COVERS.length >= 3);
  assert.equal(CHECK_CANNOT.length, 1);
  assert.match(CHECK_CANNOT[0] ?? "", /agent client is installed/);
});

test("a slug is proposed in the grammar the gateway enforces", () => {
  const grammar = /^[a-z0-9][a-z0-9-]{0,62}$/;
  for (const [name, expected] of [
    ["Payments team", "payments-team"],
    ["ACME  Ltd.", "acme-ltd"],
    ["  leading", "leading"],
    ["Ünicode Name", "unicode-name"],
  ] as const) {
    const slug = slugFrom(name);
    assert.equal(slug, expected, name);
    assert.match(slug, grammar, name);
  }
  // A name with nothing usable in it produces nothing, and the form's
  // submit stays disabled rather than sending a slug the server refuses.
  assert.equal(slugFrom("!!!"), "");
});

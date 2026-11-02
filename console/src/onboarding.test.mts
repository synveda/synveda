/**
 * First-run onboarding's model (ADR-0075 decision 6, amended by ADR-0162).
 *
 * The organisation's profile comes first and only when the server offers it;
 * the profile's own copy and adoption are asserted in `profile.test.mts`.
 * What is asserted here is the walk: the steps in order, where it starts,
 * the sentences a seeding outcome shows, and the client instructions.
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
  clientEvidence,
  readProgress,
  clientOf,
  afterProfile,
  connectionSteps,
  initialStep,
  nextStep,
  seedSentence,
  slugFrom,
  stepNumber,
} from "./onboarding.mjs";

test("a refused seeding step names where to finish the job", () => {
  // A refusal with no next step is a dead end, and the next step genuinely
  // exists — the workspace works meanwhile under whatever it inherits.
  const sentence = seedSentence({
    kind: "refused",
    what: "The Team profile for your organisation",
    why: "you hold no role that permits configuration.write here",
  });
  assert.match(sentence, /was refused/);
  assert.match(sentence, /Advanced ▸ Configuration/);
  assert.match(sentence, /keeps its current profile/);
});

test("an applied seeding step says so plainly", () => {
  assert.match(seedSentence({ kind: "applied", what: "The team Configuration" }), /done/);
  assert.match(
    seedSentence({ kind: "pending", what: "The team Configuration", changeId: "change-1" }),
    /Advanced Reviews/,
  );
  assert.match(
    seedSentence({ kind: "skipped", what: "Your organisation's profile" }),
    /not chosen yet\. Your organisation runs the regulated-strict default/,
  );
});

test("the seven steps run in order, profile first, and end at done", () => {
  assert.deepEqual(
    [...STEPS],
    ["profile", "workspace", "project", "repository", "client", "instructions", "check", "done"],
  );
  assert.equal(STEP_COUNT, 7);
  assert.equal(nextStep("profile"), "workspace");
  assert.equal(nextStep("workspace"), "project");
  assert.equal(nextStep("check"), "done");
  assert.equal(nextStep("done"), "done", "done is terminal");
  assert.equal(stepNumber("profile"), 1);
  assert.equal(stepNumber("check"), 7);
});

test("the profile step opens first-run setup only when the server offers it", () => {
  const firstRun = {
    state: "needs_workspace" as const,
    workspace_count: 0,
    project_count: 0,
    workspace_create: true,
    project_create_workspace_ids: [],
  };
  const nothing = { workspaceId: null, projectId: null };
  const open = { state: "open" as const, policy_pack: "regulated-strict" };
  assert.equal(initialStep({ ...firstRun, profile: open }, nothing), "profile");
  assert.equal(
    initialStep({ ...firstRun, profile: { ...open, state: "prepared", template: "team", artifact_id: "a" } }, nothing),
    "profile",
  );
  for (const state of ["unavailable", "chosen"] as const) {
    assert.equal(initialStep({ ...firstRun, profile: { ...open, state } }, nothing), "workspace");
  }
  assert.equal(initialStep(firstRun, nothing), "workspace", "no disclosure, no offer");
  assert.equal(afterProfile({ ...firstRun, profile: open }, nothing), "workspace");

  // Reconnecting an agent to a working project is never interrupted.
  const ready = {
    ...firstRun,
    state: "ready" as const,
    workspace_create: false,
    project_count: 1,
    project_create_workspace_ids: ["w"],
    profile: open,
  };
  assert.equal(initialStep(ready, { workspaceId: "w", projectId: "p" }), "client");
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
  // projects configuration from `adapters/registry.json`.
  const known = [
    "claude-code",
    "copilot-cli",
    "cursor",
    "vscode",
    "codex",
    "claude-desktop",
    "zed",
    "windsurf",
    "continue",
    "other",
  ];
  assert.deepEqual(
    CLIENTS.map((client) => client.id),
    known,
  );
});

test("connection instructions carry evidence levels without promoting recipes", () => {
  assert.equal(clientOf("claude-code").supportLevel, "verified");
  assert.equal(clientOf("cursor").supportLevel, "experimental");
  assert.equal(clientOf("vscode").supportLevel, "configured");
  assert.match(clientOf("cursor").limits, /complete native lifecycle is not qualified/);
});

test("hook clients bind the selected project through the managed hook installer", () => {
  for (const id of ["codex", "copilot-cli"]) {
    const client = clientOf(id);
    assert.equal(client.registration, "automatic");
    assert.equal(client.hooks, true);
    const commands = connectionSteps(client, "https://actual.example", "project-123");
    assert.equal(commands[0], "synveda login --gateway https://actual.example");
    assert.ok(commands.includes("synveda setup --project project-123 --observation off"));
    assert.equal(commands.at(-1), `synveda adapter install --client ${id}`);
    assert.ok(!commands.some((c) => c.includes("--task")), "a shared server must stay unbound across tasks");
  }
});

test("project instructions use the managed MCP writer with the selected setup receipt", () => {
  for (const client of CLIENTS.filter((c) => c.via === "mcp" && c.registration === "automatic" && !c.hooks)) {
    const commands = connectionSteps(client, "https://actual.example", "project-123");
    assert.ok(commands.includes("synveda setup --project project-123 --observation off"));
    assert.equal(commands.at(-1), `synveda adapter install --client ${client.id} --scope user`);
    assert.ok(!commands.some((c) => c.includes("--task")), "a user-wide writer must stay unbound across tasks");
  }
});

test("project access alone never becomes evidence of a client operation", () => {
  const empty = clientEvidence([], [], "project", "reader", "codex");
  assert.equal(empty.contextRecorded, false); assert.equal(empty.observationsRecorded, false);
  assert.match(empty.lines[0] ?? "", /No completed/);
  const session = { id: "session", project_id: "project", principal_id: "reader", client_name: "mcp", last_observed_at: null };
  const run = { session_id: "session", project_id: "project", completion_status: "completed" };
  const evidence = clientEvidence([session as never], [run as never], "project", "reader", "codex");
  assert.equal(evidence.contextRecorded, true); assert.equal(evidence.observationsRecorded, false);
  assert.match(evidence.lines[0] ?? "", /authenticated context operation/);
  assert.match(evidence.lines[1] ?? "", /observation remains unverified/);
  assert.match(evidence.lines[2] ?? "", /self-reported/);
  assert.equal(clientEvidence([{ ...session, principal_id: "other" } as never], [run as never], "project", "reader", "codex").contextRecorded, false);
  assert.equal(clientEvidence([session as never], [{ ...run, completion_status: "failed" } as never], "project", "reader", "codex").contextRecorded, false);
  assert.equal(clientEvidence([{ ...session, last_observed_at: "2026-10-03T12:00:00Z" } as never], [], "project", "reader", "codex").observationsRecorded, true);
});

test("progress is scoped to the current project and validates saved seeding outcomes", () => {
  const progress = { projectId: "p", step: "instructions", clientId: "codex", setupCompleted: true, seed: { kind: "pending", what: "Configuration", changeId: "review" } };
  const storage = { getItem: () => JSON.stringify(progress) };
  assert.deepEqual(readProgress(storage, "key", "p"), progress);
  assert.equal(readProgress(storage, "key", "other"), null);
  assert.equal(readProgress({ getItem: () => JSON.stringify({ ...progress, seed: { kind: "pending" } }) }, "key", "p"), null);
  assert.equal(readProgress({ getItem: () => { throw new Error("storage unavailable"); } }, "key", "p"), null);
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

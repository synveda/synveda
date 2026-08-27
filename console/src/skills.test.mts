/** Pure product rules for the CPR-24 Skills Library. */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  activationEvidence,
  evidenceLabel,
  formatBundleFiles,
  manifestSummary,
  parseBundleFiles,
  scanSummary,
  skillMutationMessage,
  skillScopes,
  sourceLabel,
} from "./skills.mjs";
import type {
  MeView,
  ProjectView,
  SkillUsageEventView,
  SkillVersionView,
} from "./generated/api.js";

test("personal and project placements come only from real anchors and carry forecasts", () => {
  const project = { id: "project", scope_id: "scope-project", display_name: "PulseBoard" } as ProjectView;
  const me = {
    anchors: [
      {
        scope_id: "scope-personal",
        kind: "principal",
        source: "principal_scope",
        direct: true,
        roles: ["owner"],
        actions: { "skill.write": true },
      },
      {
        scope_id: "scope-project",
        kind: "project",
        source: "selected_project",
        direct: true,
        roles: ["viewer"],
        actions: { "skill.write": false },
      },
      {
        scope_id: "scope-workspace",
        kind: "workspace",
        source: "selected_workspace",
        direct: true,
        roles: ["member"],
        actions: { "skill.write": true },
      },
    ],
  } as unknown as MeView;
  assert.deepEqual(skillScopes(me, project), [
    { id: "scope-personal", kind: "principal", label: "Private to me", canWrite: true },
    {
      id: "scope-project",
      kind: "project",
      label: "Project · PulseBoard",
      canWrite: false,
    },
  ]);
});

test("manifest parsing keeps extension metadata and treats declared tools as data", () => {
  const summary = manifestSummary({
    name: "release-check",
    description: "Check releases",
    compatibility: "Claude Code >= 2.1",
    license: "MIT",
    "allowed-tools": "Read Bash(git status:*)",
    metadata: { owner: "release" },
    "x-synveda-fixture": "pulseboard",
  });
  assert.equal(summary.description, "Check releases");
  assert.equal(summary.compatibility, "Claude Code >= 2.1");
  assert.deepEqual(summary.declaredTools, ["Read", "Bash(git", "status:*)"]);
  assert.deepEqual(summary.extensions, {
    metadata: { owner: "release" },
    "x-synveda-fixture": "pulseboard",
  });
});

test("extensible scan evidence is rendered without recomputing a verdict", () => {
  assert.deepEqual(
    scanSummary({
      worst: "high",
      blocks_at: "critical",
      findings: [
        { path: "scripts/release.sh", rule: "shell-network", severity: "high", line: 8, count: 2 },
        { extension_only: true },
      ],
    }),
    {
      worst: "high",
      blocksAt: "critical",
      findings: [
        { path: "scripts/release.sh", rule: "shell-network", severity: "high", line: 8, count: 2 },
        { path: "bundle", rule: "unknown-rule", severity: "unknown", line: null, count: 1 },
      ],
    },
  );
});

test("complete replacement bundles require unique text files", () => {
  const files = [
    { path: "SKILL.md", content: "instructions" },
    { path: "references/check.md", content: "fixture" },
  ];
  assert.deepEqual(parseBundleFiles(formatBundleFiles(files)), files);
  assert.throws(() => parseBundleFiles("[]"), /non-empty/);
  assert.throws(
    () => parseBundleFiles('[{"path":"SKILL.md","content":"a"},{"path":"SKILL.md","content":"b"}]'),
    /more than once/,
  );
  assert.throws(() => parseBundleFiles('[{"path":"SKILL.md"}]'), /path and content/);
});

test("governed outcomes say whether active state moved", () => {
  assert.match(
    skillMutationMessage({ change_id: "c", outcome: "pending_review" }),
    /waiting for review.*unchanged/,
  );
  assert.match(skillMutationMessage({ change_id: "c", outcome: "applied" }), /applied/);
  assert.match(skillMutationMessage({ change_id: "c", outcome: "rejected" }), /rejected.*unchanged/);
});

test("activation evidence never collapses host observation into model self-report", () => {
  const base = {
    binding_id: "binding",
    client_event_id: "client",
    id: "event",
    metadata: {},
    occurred_at: "2026-08-24T10:00:00Z",
    principal_id: "alice",
    received_at: "2026-08-24T10:00:01Z",
    stage: "activated",
    version_id: "version",
  } satisfies Omit<SkillUsageEventView, "evidence">;
  const events: SkillUsageEventView[] = [
    { ...base, id: "host", evidence: "host_observed" },
    { ...base, id: "model", evidence: "model_reported", stage: "outcome_reported" },
  ];
  assert.deepEqual(activationEvidence(events), { hostObserved: 1, modelReported: 1, activated: 1 });
  assert.equal(evidenceLabel("host_observed"), "Host-observed");
  assert.equal(evidenceLabel("model_reported"), "Model-reported");
});

test("source labels retain the non-secret provenance reference and revision", () => {
  const version = {
    source_kind: "git",
    provenance: { reference: "https://example.test/release-skill", revision: "abc123" },
  } as unknown as SkillVersionView;
  assert.equal(sourceLabel(version), "git · https://example.test/release-skill · at abc123");
});

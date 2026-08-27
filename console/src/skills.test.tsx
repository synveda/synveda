/** Reader-visible acceptance for the generated-API Skills Library (CPR-24). */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { AppProvider, type AppContextValue } from "./Shell.js";
import { SkillItem, Skills } from "./Skills.js";
import { applyMutationOutcome, type MutationNotice } from "./skills/ui.js";
import { toText } from "./text.mjs";
import type {
  AnchorCapabilities,
  AvailableSkillListView,
  MeView,
  ProjectView,
  SkillBindingView,
  SkillTestRunListView,
  SkillUsageListView,
  SkillVersionFileListView,
  SkillVersionListView,
  SkillVersionView,
  SkillView,
  WorkspaceView,
} from "./generated/api.js";

const SKILL_ID = "skill-1";
const CURRENT_ID = "version-2";
const OLD_ID = "version-1";
const PERSONAL_SCOPE = "scope-personal";
const PROJECT_SCOPE = "scope-project";

function anchor(scopeId: string, kind: string, source: string, canWrite: boolean): AnchorCapabilities {
  return {
    scope_id: scopeId,
    kind,
    source,
    direct: true,
    roles: canWrite ? ["owner"] : ["viewer"],
    actions: { "skill.read": true, "skill.write": canWrite },
  };
}

function version(ordinal: number, overrides: Partial<SkillVersionView> = {}): SkillVersionView {
  return {
    id: ordinal === 2 ? CURRENT_ID : OLD_ID,
    skill_id: SKILL_ID,
    ordinal,
    bundle_digest: ordinal === 2 ? "b".repeat(64) : "a".repeat(64),
    sensitivity: "internal",
    manifest: {
      name: "release-check",
      description: "Check PulseBoard releases before publishing.",
      license: "MIT",
      compatibility: "Claude Code >= 2.1",
      "allowed-tools": ["Read", "Bash(git status:*)"],
      metadata: { owner: "release-engineering" },
      "x-fixture": "pulseboard-release",
    },
    source_kind: "git",
    provenance: {
      reference: "https://github.com/example/pulseboard-skills",
      revision: ordinal === 2 ? "abc123" : "def000",
    },
    scan_ruleset_version: 4,
    scan: {
      worst: "medium",
      blocks_at: "critical",
      findings: [
        {
          path: "scripts/release.sh",
          rule: "shell-process",
          severity: "medium",
          line: 7,
          count: 1,
        },
      ],
    },
    rubric_version: 3,
    quality_score: 91,
    declared_tools_are_authorization: false,
    created_at: `2026-08-${ordinal === 2 ? "24" : "20"}T10:00:00Z`,
    created_by: "alice@example.test",
    ...overrides,
  };
}

function skill(): SkillView {
  return {
    id: SKILL_ID,
    governing_scope_id: PROJECT_SCOPE,
    name: "release-check",
    current_version_id: CURRENT_ID,
    current_version: version(2),
    created_at: "2026-08-20T10:00:00Z",
    created_by: "alice@example.test",
    updated_at: "2026-08-24T10:00:00Z",
    updated_by: "alice@example.test",
  };
}

function binding(
  id: string,
  scopeId: string,
  enabled: boolean,
  pinned: string | null,
): SkillBindingView {
  return {
    id,
    scope_id: scopeId,
    skill_id: SKILL_ID,
    pinned_version_id: pinned,
    enabled,
    revision: enabled ? 4 : 2,
    created_at: "2026-08-24T10:10:00Z",
    created_by: "alice@example.test",
    updated_at: "2026-08-24T10:20:00Z",
    updated_by: "alice@example.test",
  };
}

function context(canWrite = true): AppContextValue {
  const workspace = {
    id: "workspace-1",
    scope_id: "scope-workspace",
    display_name: "PulseBoard",
  } as WorkspaceView;
  const project = {
    id: "project-1",
    workspace_id: workspace.id,
    scope_id: PROJECT_SCOPE,
    display_name: "PulseBoard API",
  } as ProjectView;
  const me = {
    anchors: [
      anchor(PERSONAL_SCOPE, "principal", "principal_scope", canWrite),
      anchor(PROJECT_SCOPE, "project", "selected_project", canWrite),
    ],
    principal: { subject: "alice@example.test", quarantined: false },
    workspaces: [workspace],
    projects: [project],
  } as MeView;
  return {
    me,
    selection: { workspaceId: workspace.id, projectId: project.id },
    workspace,
    project,
    chooseWorkspace: () => {},
    chooseProject: () => {},
    reload: () => {},
  };
}

async function seed(key: string, body: unknown): Promise<void> {
  await cache.ensure(key, async (): Promise<Outcome> => ({ kind: "ok", body }));
}

async function seedDetail({ empty = false }: { empty?: boolean } = {}): Promise<void> {
  const current = skill();
  const versions: SkillVersionListView = { versions: [version(2), version(1)] };
  const personal = binding("binding-personal", PERSONAL_SCOPE, true, OLD_ID);
  const project = binding("binding-project", PROJECT_SCOPE, false, null);
  const personalAvailable: AvailableSkillListView = {
    scope_id: PERSONAL_SCOPE,
    skills: [
      {
        name: current.name,
        binding: personal,
        version: version(1),
        manifest_object_hash: "manifest-old",
      },
    ],
  };
  const projectAvailable: AvailableSkillListView = {
    scope_id: PROJECT_SCOPE,
    skills: [
      {
        name: current.name,
        binding: personal,
        version: version(1),
        manifest_object_hash: "manifest-old",
      },
    ],
  };
  const files: SkillVersionFileListView = {
    files: empty ? [] : [
      {
        path: "SKILL.md",
        object_hash: "object-manifest",
        chars: 181,
        created_at: "2026-08-24T10:00:00Z",
      },
      {
        path: "scripts/release.sh",
        object_hash: "object-script",
        chars: 48,
        created_at: "2026-08-24T10:00:00Z",
      },
    ],
  };
  const tests: SkillTestRunListView = {
    runs: empty ? [] : [
      {
        id: "test-validation",
        version_id: CURRENT_ID,
        harness: "validation_sandbox",
        harness_version: "validation-sandbox-v1",
        outcome: "passed",
        scan_ruleset_version: 4,
        rubric_version: 3,
        evidence: { executes_bundle_code: false, files: 2 },
        created_at: "2026-08-24T11:00:00Z",
        created_by: "alice@example.test",
      },
      {
        id: "test-client",
        version_id: CURRENT_ID,
        harness: "controlled_client",
        harness_version: "claude-code-2.1.241",
        outcome: "passed",
        scan_ruleset_version: 4,
        rubric_version: 3,
        evidence: { fixture: "authentic-release-frame" },
        created_at: "2026-08-24T11:05:00Z",
        created_by: "adapter:claude-code",
      },
    ],
  };
  const usage: SkillUsageListView = {
    events: empty ? [] : [
      {
        id: "usage-host",
        binding_id: personal.id,
        version_id: CURRENT_ID,
        principal_id: "alice@example.test",
        session_id: "session-1",
        client_event_id: "host-1",
        stage: "activated",
        evidence: "host_observed",
        metadata: {},
        occurred_at: "2026-08-24T11:10:00Z",
        received_at: "2026-08-24T11:10:01Z",
      },
      {
        id: "usage-model",
        binding_id: personal.id,
        version_id: CURRENT_ID,
        principal_id: "alice@example.test",
        client_event_id: "model-1",
        stage: "outcome_reported",
        evidence: "model_reported",
        metadata: {},
        occurred_at: "2026-08-24T11:11:00Z",
        received_at: "2026-08-24T11:11:01Z",
      },
    ],
  };

  await Promise.all([
    seed(`skills/item/${SKILL_ID}`, current),
    seed(`skills/item/${SKILL_ID}/versions`, versions),
    seed(`skills/bindings/${PERSONAL_SCOPE}`, { bindings: empty ? [] : [personal] }),
    seed(`skills/bindings/${PROJECT_SCOPE}`, { bindings: empty ? [] : [project] }),
    seed(
      `skills/available/${PERSONAL_SCOPE}`,
      empty ? { scope_id: PERSONAL_SCOPE, skills: [] } : personalAvailable,
    ),
    seed(
      `skills/available/${PROJECT_SCOPE}`,
      empty ? { scope_id: PROJECT_SCOPE, skills: [] } : projectAvailable,
    ),
    seed(`skills/item/${SKILL_ID}/versions/${CURRENT_ID}/files`, files),
    ...(empty
      ? []
      : [
          seed(`skills/item/${SKILL_ID}/versions/${CURRENT_ID}/files/SKILL.md`, {
            version_id: CURRENT_ID,
            path: "SKILL.md",
            object_hash: "object-manifest",
            content: "# Release check\n\nRun the fixture in the controlled release harness.",
          }),
        ]),
    seed(`skills/item/${SKILL_ID}/versions/${CURRENT_ID}/tests`, tests),
    seed(`skills/item/${SKILL_ID}/versions/${CURRENT_ID}/usage`, usage),
  ]);
}

beforeEach(() => cache.clear());

test("the catalogue shows installed immutable heads and exact session availability", async () => {
  const current = skill();
  const personal = binding("binding-personal", PERSONAL_SCOPE, true, OLD_ID);
  await Promise.all([
    seed("skills/catalogue/first", { skills: [current] }),
    seed(`skills/available/${PERSONAL_SCOPE}`, {
      scope_id: PERSONAL_SCOPE,
      skills: [
        {
          name: current.name,
          binding: personal,
          version: version(1),
          manifest_object_hash: "manifest-old",
        },
      ],
    }),
  ]);
  const text = toText(
    renderToStaticMarkup(
      <AppProvider value={{ ...context(), project: null }}>
        <Skills />
      </AppProvider>,
    ),
  );
  for (const expected of [
    "Installed Skills",
    "release-check",
    "Current v2",
    "quality 91/100",
    "Available to a session",
    "Private to me",
    "release-check v1",
    "pinned",
    "Install Skill",
  ]) {
    assert.match(text, new RegExp(expected, "i"), expected);
  }
});

test("one Skill exposes versions, files, provenance, bindings, tests and distinct usage evidence", async () => {
  await seedDetail();
  const markup = renderToStaticMarkup(
    <AppProvider value={context()}>
      <SkillItem skillId={SKILL_ID} />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "current v2",
    "Private to me",
    "Project · PulseBoard API",
    "follows current",
    "pinned",
    "Disable",
    "Enable",
    "Pin selected",
    "Follow current",
    "Roll back binding",
    "Create immutable version",
    "Claude Code >= 2.1",
    "https://github.com/example/pulseboard-skills",
    "abc123",
    "91/100",
    "release-engineering",
    "pulseboard-release",
    "Tool declarations are metadata only",
    "grant no access",
    "Bash(git status:*)",
    "shell-process",
    "scripts/release.sh:7",
    "SKILL.md",
    "object-manifest",
    "Run the fixture in the controlled release harness",
    "Fixture testing",
    "validation sandbox",
    "executes no Skill scripts",
    "controlled client",
    "claude-code-2.1.241",
    "Recent activation evidence",
    "Host-observed",
    "Model-reported",
    "1 activations",
  ]) {
    assert.match(text, new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "i"), expected);
  }
  assert.match(markup, /href="\/console\/skills"/);
  assert.doesNotMatch(text, /declared tools.*authori[sz]ed/i);
});

test("write forecasts hide every mutation while leaving governed evidence readable", async () => {
  await seedDetail();
  const markup = renderToStaticMarkup(
    <AppProvider value={context(false)}>
      <SkillItem skillId={SKILL_ID} />
    </AppProvider>,
  );
  const text = toText(markup);
  assert.match(text, /does not forecast skill\.write/i);
  assert.match(text, /Host-observed/);
  assert.match(text, /validation sandbox/);
  for (const action of [
    "Disable",
    "Enable",
    "Pin selected",
    "Follow current",
    "Roll back binding",
    "Create immutable version",
    "Run validation sandbox",
  ]) {
    assert.doesNotMatch(
      markup,
      new RegExp(`>${action.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}</button>`, "i"),
      action,
    );
  }
});

test("capability and evidence boundaries preserve honest empty states", async () => {
  await seedDetail({ empty: true });
  const markup = renderToStaticMarkup(
    <AppProvider value={context(false)}>
      <SkillItem skillId={SKILL_ID} />
    </AppProvider>,
  );
  const text = toText(markup);
  assert.equal((text.match(/policy does not offer creation here/gi) ?? []).length, 2);
  assert.match(text, /This version contains no visible files/i);
  assert.match(text, /No controlled test evidence has been recorded/i);
  assert.match(text, /No usage evidence has been recorded/i);
  assert.doesNotMatch(markup, />Run validation sandbox<\/button>/i);
});

test("only an applied governed outcome invalidates state or navigates", () => {
  const invalidations: string[][] = [];
  const navigations: string[] = [];
  const sideEffects = {
    invalidate: (...prefixes: string[]) => invalidations.push(prefixes),
    navigateToSkill: (skillId: string) => navigations.push(skillId),
  };
  const notice = (outcome: "applied" | "pending_review" | "rejected"): MutationNotice => ({
    kind: "result",
    result: { change_id: `change-${outcome}`, outcome, skill_id: SKILL_ID },
  });

  applyMutationOutcome(notice("pending_review"), ["skills"], sideEffects);
  applyMutationOutcome(notice("rejected"), ["skills"], sideEffects);
  assert.deepEqual(invalidations, []);
  assert.deepEqual(navigations, []);

  applyMutationOutcome(notice("applied"), ["skills"], sideEffects);
  assert.deepEqual(invalidations, [["skills"]]);
  assert.deepEqual(navigations, [SKILL_ID]);
});

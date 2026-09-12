/** Reader-visible acceptance for the bounded local Configuration editor (CPR-45). */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { cache } from "./cache.mjs";
import { Configuration, NoticeView } from "./Configuration.js";
import { reconcile } from "./selection.mjs";
import { AppProvider, appContext } from "./Shell.js";
import { toText } from "./text.mjs";
import type {
  ConfigurationDocumentBody,
  MeView,
  ProjectView,
  WorkspaceView,
} from "./generated/api.js";

const WORKSPACE_SCOPE = "scope-workspace-configuration";
const PROJECT_SCOPE = "scope-project-configuration";
const ARTIFACT_ID = "configuration-runtime";
const VERSION_ID = "configuration-version-3";
const BINDING_ID = "configuration-binding-project";

const document: ConfigurationDocumentBody = {
  policy_pack: "strict-review",
  capture: {
    enabled: true,
    on_session_end: true,
    explicit_request: true,
    minimum_confidence_permille: 600,
    maximum_candidates_per_batch: 48,
  },
  context: {
    token_budget: 1500,
    channels: ["current_knowledge"],
    trace_retention: "redacted",
    graph: {
      enabled: true,
      max_hops: 2,
      fan_out_per_node: 6,
      max_expanded_candidates: 24,
      time_budget_ms: 350,
      token_budget: 384,
    },
  },
  freshness: {
    fact_days: 30,
    decision_days: 0,
    preference_days: 0,
    procedure_days: 90,
    entity_days: 60,
    episode_days: 0,
    convention_days: 30,
    warning_days: 14,
    reference_days: 30,
  },
  advertisement: { skills: true, tools: true },
  relaxations: { enabled: false, maximum_duration_secs: 3600, allowed_actions: [] },
  allowed_external_providers: [],
};

function context(): { me: MeView; workspace: WorkspaceView; project: ProjectView } {
  const workspace = {
    id: "workspace-configuration",
    scope_id: WORKSPACE_SCOPE,
    slug: "demo",
    display_name: "Demo workspace",
    status: "active",
  } as WorkspaceView;
  const project = {
    id: "project-configuration",
    workspace_id: workspace.id,
    scope_id: PROJECT_SCOPE,
    slug: "retry-review",
    display_name: "Retry review",
    status: "active",
  } as ProjectView;
  const me = {
    principal: { subject: "subject-avery", display_name: "Avery Author", quarantined: false },
    tenant: { id: "tenant-demo", slug: "demo", name: "Demo tenant", status: "active" },
    onboarding: { state: "ready", workspace_count: 1, project_count: 1 },
    capabilities: { actions: { "configuration.read": true }, role_keys: ["administrator"] },
    anchors: [],
    workspaces: [workspace],
    projects: [project],
  } as MeView;
  return { me, workspace, project };
}

const ok = (body: unknown) => ({ kind: "ok" as const, body });

beforeEach(() => cache.clear());

test("the normal editor is typed, scoped, and distinct from the effective version", async () => {
  const { me, workspace, project } = context();
  await Promise.all([
    cache.ensure("configuration/templates", async () => ok({ templates: [] })),
    cache.ensure("configuration/artifacts", async () =>
      ok({
        artifacts: [
          {
            id: ARTIFACT_ID,
            name: "demo-runtime",
            governing_scope_id: PROJECT_SCOPE,
            current_version_id: VERSION_ID,
            created_by: "subject-avery",
            updated_by: "subject-avery",
            created_at: "2026-09-10T09:00:00Z",
            updated_at: "2026-09-10T09:05:00Z",
          },
        ],
      }),
    ),
    cache.ensure(`configuration/effective/${PROJECT_SCOPE}`, async () =>
      ok({
        artifact_id: ARTIFACT_ID,
        version_id: VERSION_ID,
        binding_id: BINDING_ID,
        binding_scope_id: PROJECT_SCOPE,
        content_hash: "effective-configuration-hash",
        document,
        fail_safe: false,
        scope_id: PROJECT_SCOPE,
      }),
    ),
    cache.ensure(`configuration/versions/${ARTIFACT_ID}`, async () =>
      ok({
        versions: [
          {
            id: VERSION_ID,
            artifact_id: ARTIFACT_ID,
            ordinal: 3,
            content_hash: "effective-configuration-hash",
            document,
            change_id: "change-applied",
            created_by: "subject-avery",
            created_at: "2026-09-10T09:05:00Z",
          },
        ],
      }),
    ),
    cache.ensure(`configuration/bindings/${PROJECT_SCOPE}`, async () =>
      ok({
        bindings: [
          {
            id: BINDING_ID,
            artifact_id: ARTIFACT_ID,
            scope_id: PROJECT_SCOPE,
            enabled: true,
            revision: 2,
            created_by: "subject-avery",
            updated_by: "subject-avery",
            created_at: "2026-09-10T09:05:00Z",
            updated_at: "2026-09-10T09:05:00Z",
          },
        ],
      }),
    ),
    cache.ensure("policy/packs", async () => ok({ packs: [] })),
    cache.ensure("relaxations", async () => ok({ relaxations: [] })),
  ]);

  const selection = reconcile({ workspaceId: workspace.id, projectId: project.id }, me);
  const markup = renderToStaticMarkup(
    <AppProvider value={appContext(me, selection, () => {})}>
      <Configuration />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "Effective now at project retry-review",
    "A governed version is effective",
    "proposed version or binding does not replace this evidence",
    "not proof this artifact is effective",
    "Propose Capture and Context settings",
    "Minimum confidence (0–1000)",
    "Maximum candidates per batch (1–256)",
    "Maximum token budget (1–100000)",
    "Include visibly unreviewed candidates",
    "Complete immutable document to propose",
    "pending is not effective",
  ]) {
    assert.ok(text.toLowerCase().includes(expected.toLowerCase()), expected);
  }
  assert.doesNotMatch(markup, /<textarea/);
  assert.match(markup, /type="number" min="0" max="1000"/);
  assert.match(markup, /type="number" min="1" max="256"/);
  assert.match(markup, /type="number" min="1" max="100000"/);
});

test("a pending Configuration mutation is warning state linked to its exact review", () => {
  const markup = renderToStaticMarkup(
    <NoticeView notice={{ result: { change_id: "change-configuration-pending", outcome: "pending_review" } }} />,
  );
  assert.match(markup, /banner warning/);
  assert.match(markup, /href="\/console\/advanced\/reviews\/change-configuration-pending"/);
  assert.match(toText(markup), /runtime selection is unchanged/i);
});

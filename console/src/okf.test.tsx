/** Reader-visible acceptance for OKF import/export workflows (CPR-28). */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { ExportSummary, OkfExchange, PlanEvidence } from "./Okf.js";
import { AppProvider, type AppContextValue } from "./Shell.js";
import { toText } from "./text.mjs";
import type {
  AnchorCapabilities,
  KnowledgeListView,
  MeView,
  OkfExportView,
  OkfImportJobView,
  OkfImportPlanView,
  OkfMappingView,
  OkfMaterializationView,
  ProjectView,
  WorkspaceView,
} from "./generated/api.js";

const PROJECT_ID = "project-pulseboard";
const PROJECT_SCOPE = "scope-project";

function context(canWrite = true, canRead = true): AppContextValue {
  const workspace = {
    id: "workspace-pulseboard",
    scope_id: "scope-workspace",
    display_name: "PulseBoard",
  } as WorkspaceView;
  const project = {
    id: PROJECT_ID,
    workspace_id: workspace.id,
    scope_id: PROJECT_SCOPE,
    display_name: "PulseBoard API",
  } as ProjectView;
  const anchor = {
    scope_id: PROJECT_SCOPE,
    kind: "project",
    source: "selected_project",
    direct: true,
    roles: canWrite ? ["editor"] : ["viewer"],
    actions: { "knowledge.read": canRead, "knowledge.write": canWrite },
  } as AnchorCapabilities;
  const me = {
    anchors: [anchor],
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

function job(state = "planned"): OkfImportJobView {
  return {
    id: "import-pulseboard",
    project_id: PROJECT_ID,
    format: "okf",
    format_version: "0.2",
    specification_commit: "ad30107c31c06aec8a7d5636e0d1058118604e6f",
    source_kind: "git",
    source_locator: "pulseboard-knowledge",
    source_revision: "release-42",
    bundle_digest: "b".repeat(64),
    state,
    artifact_count: 5,
    mapping_count: 4,
    candidate_count: state === "materialized" ? 3 : 0,
    capture_batch_id: state === "materialized" ? "batch-okf" : null,
    notices: ["one internal link targets a concept outside this bundle"],
    created_at: "2026-08-25T12:00:00Z",
    completed_at: state === "materialized" ? "2026-08-25T12:01:00Z" : null,
  };
}

function mapping(
  ordinal: number,
  classification: string,
  type = "pulseboard-runbook",
): OkfMappingView {
  return {
    id: `mapping-${ordinal}`,
    artifact_id: `artifact-${ordinal}`,
    ordinal,
    okf_type: type,
    knowledge_type: ordinal === 2 ? "warning" : "procedure",
    content: {
      title: `PulseBoard concept ${ordinal}`,
      body_markdown: `Body ${ordinal}`,
      summary: `Summary ${ordinal}`,
      tags: ["pulseboard"],
      sensitivity: "internal",
      confidence_permille: 900,
      metadata: {
        okf: {
          logical_path: `concept-${ordinal}.md`,
          extensions: { "x-retention-class": "operational" },
        },
      },
    },
    content_hash: `${ordinal}`.repeat(64),
    classification,
    matched_item_id: classification === "addition" ? null : `knowledge-${ordinal}`,
    matched_revision_id: classification === "addition" ? null : `revision-${ordinal}`,
    proposed_relations: { links: [{ target: "decision.md", relation: "related_to" }] },
    materializable: classification !== "duplicate",
    content_erased: false,
  };
}

function plan(): OkfImportPlanView {
  return {
    job: job(),
    artifacts: [1, 2, 3, 4, 5].map((ordinal) => ({
      id: `artifact-${ordinal}`,
      ordinal,
      logical_path: ordinal === 5 ? "index.md" : `concept-${ordinal}.md`,
      kind: ordinal === 5 ? "index" : "concept",
      content_hash: `${ordinal}`.repeat(64),
      frontmatter:
        ordinal === 1
          ? { type: "pulseboard-runbook", "x-retention-class": "operational" }
          : { type: "reference" },
      body_markdown: `Body ${ordinal}`,
    })),
    mappings: [
      mapping(1, "addition"),
      mapping(2, "update"),
      mapping(3, "duplicate"),
      mapping(4, "conflict"),
    ],
  };
}

function materialized(): OkfMaterializationView {
  return {
    job: job("materialized"),
    batch: { id: "batch-okf" },
    candidates: [
      {
        id: "candidate-1",
        state: "pending",
        content: { title: "PulseBoard concept 1" },
      },
      {
        id: "candidate-2",
        state: "pending",
        content: { title: "PulseBoard concept 2" },
      },
      {
        id: "candidate-4",
        state: "pending",
        content: { title: "PulseBoard concept 4" },
      },
    ],
  } as OkfMaterializationView;
}

async function seed(key: string, body: unknown): Promise<void> {
  await cache.ensure(key, async (): Promise<Outcome> => ({ kind: "ok", body }));
}

beforeEach(() => cache.clear());

test("a dry-run exposes every classification unknown metadata and resulting candidates", () => {
  const markup = renderToStaticMarkup(
    <PlanEvidence plan={plan()} materialized={materialized()} canMaterialize={false} />,
  );
  const text = toText(markup);
  for (const expected of [
    "pulseboard-knowledge",
    "release-42",
    "Validation passed",
    "1 Additions",
    "1 Updates",
    "1 Duplicates",
    "1 Conflicts",
    "3 Candidates",
    "pulseboard-runbook",
    "x-retention-class",
    "operational",
    "New Learnings",
    "candidate-1",
    "Unreviewed candidates are not active Knowledge",
  ]) {
    assert.match(text, new RegExp(expected), text);
  }
});

test("an erased mapping names the state without reconstructing deleted content", () => {
  const erased = mapping(4, "conflict");
  erased.content = {
    ...erased.content,
    title: "",
    body_markdown: "",
    summary: "",
    tags: [],
    metadata: {},
  };
  erased.matched_item_id = null;
  erased.matched_revision_id = null;
  erased.proposed_relations = {};
  erased.materializable = false;
  erased.content_erased = true;
  const evidence = plan();
  evidence.mappings = [erased];

  const text = toText(renderToStaticMarkup(<PlanEvidence plan={evidence} />));
  for (const expected of [
    "Derived mapping content erased",
    "conflict",
    "Derived plaintext and live Knowledge addresses erased",
    erased.content_hash,
    erased.artifact_id,
  ]) {
    assert.match(text, new RegExp(expected), text);
  }
  assert.doesNotMatch(text, /retained only|compared with|Preserved metadata and proposed relations/);
});

test("the project page offers source planning history selection and export status", async () => {
  const knowledge = {
    items: [
      {
        id: "knowledge-1",
        knowledge_type: "decision",
        current_revision: { title: "Use traceparent", revision_number: 2 },
      },
    ],
    retrieval_mode: "listing",
  } as KnowledgeListView;
  await Promise.all([
    seed(`okf/imports/${PROJECT_ID}`, { jobs: [job()] }),
    seed(`okf/export-knowledge/${PROJECT_ID}`, knowledge),
  ]);
  const markup = renderToStaticMarkup(
    <AppProvider value={context()}>
      <OkfExchange />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "Import / Export",
    "Import source",
    "Validate and plan dry-run",
    "Import history",
    "Dry-run complete · no candidates created",
    "Inspect plan",
    "Export current Knowledge",
    "Use traceparent",
    "Export job status: idle",
    "does not run scripts",
  ]) {
    assert.match(text, new RegExp(expected), text);
  }
});

test("scope forecasts hide import and export controls without hiding the product", async () => {
  await Promise.all([
    seed(`okf/imports/${PROJECT_ID}`, { jobs: [] }),
    seed(`okf/export-knowledge/${PROJECT_ID}`, { items: [], retrieval_mode: "listing" }),
  ]);
  const text = toText(
    renderToStaticMarkup(
      <AppProvider value={context(false, false)}>
        <OkfExchange />
      </AppProvider>,
    ),
  );
  assert.match(text, /does not forecast knowledge.write/);
  assert.match(text, /does not forecast knowledge.read/);
  assert.doesNotMatch(text, /Validate and plan dry-run/);
  assert.doesNotMatch(text, /Export OKF v0.2/);
});

test("the completed export keeps exact paths hashes extension metadata and file downloads", () => {
  const result: OkfExportView = {
    format_version: "0.2",
    specification_commit: "ad30107c31c06aec8a7d5636e0d1058118604e6f",
    bundle_digest: "d".repeat(64),
    files: [
      {
        logical_path: "index.md",
        content_hash: "i".repeat(64),
        content: "# Knowledge\n",
      },
      {
        logical_path: "runbooks/webhooks.md",
        content_hash: "w".repeat(64),
        content:
          "---\ntype: pulseboard-runbook\nx-retention-class: operational\n---\nDeduplicate webhooks.\n",
      },
    ],
  };
  const markup = renderToStaticMarkup(<ExportSummary result={result} />);
  const text = toText(markup);
  for (const expected of [
    "Exported bundle summary",
    "2 stable file\\(s\\)",
    "runbooks/webhooks.md",
    "pulseboard-runbook",
    "x-retention-class",
    "Download file",
    "complete stable directory shape atomically",
  ]) {
    assert.match(text, new RegExp(expected), text);
  }
  assert.match(markup, /download="webhooks.md"/);
});

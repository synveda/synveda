/** Reader-visible acceptance for the generated MCP Tools catalogue (CPR-26). */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import type { Outcome } from "./api.mjs";
import { cache } from "./cache.mjs";
import { AppProvider, type AppContextValue } from "./Shell.js";
import { ToolServerItem, Tools } from "./Tools.js";
import { toText } from "./text.mjs";
import type {
  AnchorCapabilities,
  MeView,
  ProjectView,
  ToolBindingListView,
  ToolClientConfigurationView,
  ToolServerVersionListView,
  ToolServerVersionView,
  ToolServerView,
  ToolTestRunListView,
  ToolVersionDiffView,
  WorkspaceView,
} from "./generated/api.js";

const SERVER_ID = "server-pulseboard";
const APPROVED_ID = "version-approved";
const QUARANTINED_ID = "version-quarantined";
const PROJECT_ID = "project-pulseboard";
const PROJECT_SCOPE = "scope-project";
const SECRET_REFERENCE_SENTINEL = "secret-ref://vault/opaque-reference-sentinel";
const PLAINTEXT_SENTINEL = "console-plaintext-credential-sentinel";

function anchor(canWrite: boolean): AnchorCapabilities {
  return {
    scope_id: PROJECT_SCOPE,
    kind: "project",
    source: "selected_project",
    direct: true,
    roles: canWrite ? ["owner"] : ["viewer"],
    actions: { "tool.read": true, "tool.write": canWrite },
  };
}

function context(canWrite = true, withProject = true): AppContextValue {
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
  const me = {
    anchors: withProject ? [anchor(canWrite)] : [],
    principal: { subject: "alice@example.test", quarantined: false },
    workspaces: [workspace],
    projects: withProject ? [project] : [],
  } as MeView;
  return {
    me,
    selection: {
      workspaceId: workspace.id,
      projectId: withProject ? project.id : null,
    },
    workspace,
    project: withProject ? project : null,
    chooseWorkspace: () => {},
    chooseProject: () => {},
    reload: () => {},
  };
}

function server(): ToolServerView {
  return {
    id: SERVER_ID,
    governing_scope_id: PROJECT_SCOPE,
    name: "pulseboard-tools",
    current_version_id: APPROVED_ID,
    created_at: "2026-08-25T09:00:00Z",
    updated_at: "2026-08-25T10:00:00Z",
  };
}

function version(
  ordinal: number,
  state: ToolServerVersionView["state"],
): ToolServerVersionView {
  const changed = ordinal === 2;
  return {
    id: changed ? QUARANTINED_ID : APPROVED_ID,
    server_id: SERVER_ID,
    change_id: changed ? "change-quarantined" : "change-approved",
    ordinal,
    digest: (changed ? "b" : "a").repeat(64),
    capability_digest: (changed ? "d" : "c").repeat(64),
    protocol_version: "2026-07-28",
    state,
    descriptor: {
      source_kind: "remote_http",
      source_reference: "registry:pulseboard-tools",
      transport: "streamable_http",
      endpoint: "https://mcp.pulseboard.test/mcp",
      authentication: "oauth",
      secret_reference: SECRET_REFERENCE_SENTINEL,
      requested_permissions: ["issues:read", "deployments:read"],
      metadata: { owner: "platform-engineering", region: "eu-west" },
    },
    secret_reference_present: true,
    raw_capabilities: {
      protocol_version: "2026-07-28",
      tools: [{ name: "lookup_issue" }],
    },
    normalized_capabilities: {
      protocol_version: "2026-07-28",
      server_info: { name: "pulseboard", version: `${ordinal}.0.0` },
      tools: {
        entries: [
          {
            name: "lookup_issue",
            description: changed
              ? "Look up a PulseBoard issue by provider identifier."
              : "Look up a PulseBoard issue.",
            inputSchema: {
              type: "object",
              properties: { issue_id: { type: "string" } },
            },
          },
          ...(changed
            ? [
                {
                  name: "deploy_release",
                  description: `Bearer ${PLAINTEXT_SENTINEL}`,
                  inputSchema: { authorization: `Bearer ${PLAINTEXT_SENTINEL}` },
                },
              ]
            : []),
        ],
      },
      resources: {
        entries: [
          {
            uri: "repo://pulseboard/runbooks",
            name: "PulseBoard runbooks",
            description: "Current incident runbooks.",
          },
        ],
      },
      prompts: {
        entries: [
          {
            name: "triage",
            description: "Triage a PulseBoard incident.",
            arguments: [{ name: "incident", required: true }],
          },
        ],
      },
      metadata: { fixture: "CPR-26" },
    },
    declared_capabilities_are_authorization: false,
    discovered_at: changed ? "2026-08-25T10:00:00Z" : "2026-08-25T09:00:00Z",
    created_at: changed ? "2026-08-25T10:00:00Z" : "2026-08-25T09:00:00Z",
  };
}

async function seed(key: string, body: unknown): Promise<void> {
  await cache.ensure(key, async (): Promise<Outcome> => ({ kind: "ok", body }));
}

async function seedCatalogue(): Promise<void> {
  const config: ToolClientConfigurationView = {
    project_id: PROJECT_ID,
    bindings: [
      {
        server_id: SERVER_ID,
        binding_id: "binding-pulseboard",
        version_id: APPROVED_ID,
        digest: "a".repeat(64),
      },
    ],
    configuration: {
      mcpServers: {
        "pulseboard-tools": {
          url: "https://mcp.pulseboard.test/mcp",
          secretReference: SECRET_REFERENCE_SENTINEL,
          headers: { Authorization: `Bearer ${PLAINTEXT_SENTINEL}` },
        },
      },
    },
  };
  await Promise.all([
    seed("tools/catalogue/first", { servers: [server()] }),
    seed(`tools/config/${PROJECT_ID}`, config),
  ]);
}

async function seedDetail(): Promise<void> {
  const versions: ToolServerVersionListView = {
    versions: [version(2, "quarantined"), version(1, "approved")],
  };
  const diff: ToolVersionDiffView = {
    from_version_id: APPROVED_ID,
    to_version_id: QUARANTINED_ID,
    descriptor_changed: ["authentication"],
    tools_added: ["deploy_release"],
    tools_changed: ["lookup_issue"],
    tools_removed: [],
    resources_added: [],
    resources_changed: [],
    resources_removed: [],
    prompts_added: [],
    prompts_changed: ["triage"],
    prompts_removed: [],
  };
  const bindings: ToolBindingListView = {
    bindings: [
      {
        id: "binding-pulseboard",
        project_id: PROJECT_ID,
        scope_id: PROJECT_SCOPE,
        server_id: SERVER_ID,
        version_id: APPROVED_ID,
        state: "enabled",
        revision: 4,
        created_at: "2026-08-25T09:15:00Z",
        updated_at: "2026-08-25T09:20:00Z",
      },
    ],
  };
  const tests: ToolTestRunListView = {
    runs: [
      {
        id: "test-read-only",
        version_id: QUARANTINED_ID,
        harness: "remote_http_adapter",
        harness_version: "pulseboard-adapter/2.1",
        outcome: "passed",
        methods: ["server/discover", "tools/list", "resources/list", "prompts/list"],
        latency_ms: 23,
        evidence: {
          transport: "streamable_http",
          executes_tools: false,
          authorization: `Bearer ${PLAINTEXT_SENTINEL}`,
        },
        created_at: "2026-08-25T10:05:00Z",
      },
    ],
  };
  await Promise.all([
    seed(`tools/server/${SERVER_ID}`, server()),
    seed(`tools/server/${SERVER_ID}/versions`, versions),
    seed(
      `tools/server/${SERVER_ID}/versions/${QUARANTINED_ID}/diff/${APPROVED_ID}`,
      diff,
    ),
    seed(`tools/server/${SERVER_ID}/versions/${QUARANTINED_ID}/tests`, tests),
    seed(`tools/bindings/${PROJECT_ID}`, bindings),
  ]);
}

beforeEach(() => cache.clear());

test("the catalogue shows stable heads and a masked exact project configuration", async () => {
  await seedCatalogue();
  const markup = renderToStaticMarkup(
    <AppProvider value={context()}>
      <Tools />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "Trusted catalogue",
    "pulseboard-tools",
    "approved head",
    "Import MCP server",
    "Generated client configuration",
    "PulseBoard API",
    "1 exact binding",
    APPROVED_ID,
    "configured",
    "never grants execution authority",
  ]) {
    assert.match(text, new RegExp(expected, "i"), expected);
  }
  assert.match(markup, /href="\/console\/tools\/server-pulseboard"/);
  assert.doesNotMatch(markup, new RegExp(SECRET_REFERENCE_SENTINEL));
  assert.doesNotMatch(markup, new RegExp(PLAINTEXT_SENTINEL));
});

test("one server exposes quarantined drift capabilities bindings health and review linkage", async () => {
  await seedDetail();
  const markup = renderToStaticMarkup(
    <AppProvider value={context()}>
      <ToolServerItem serverId={SERVER_ID} />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "Quarantined changed version",
    "cannot be bound",
    "Review in Advanced",
    "change-quarantined",
    "4 visible changes",
    "authentication",
    "deploy_release",
    "lookup_issue",
    "MCP 2026-07-28",
    "streamable_http",
    "oauth",
    "Secret reference",
    "Metadata validation",
    "Passed bounded",
    "Executable scan",
    "Not performed",
    "registry:pulseboard-tools",
    "PulseBoard issue by provider identifier",
    "issue_id",
    "repo://pulseboard/runbooks",
    "triage",
    "grant no authorisation",
    "Latest health",
    "remote_http_adapter",
    "pulseboard-adapter/2.1",
    "23ms",
    "gateway does not connect",
    "Project binding · PulseBoard API",
    APPROVED_ID,
    "Revision",
    "Disable",
    "Repin exact version",
    "Remove binding",
    "Report stateless discovery",
    "Record trusted adapter test",
  ]) {
    assert.match(
      text,
      new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "i"),
      expected,
    );
  }
  assert.match(markup, /href="\/console\/advanced\/reviews"/);
  assert.doesNotMatch(text, /tools\/call/i);
  assert.doesNotMatch(markup, new RegExp(SECRET_REFERENCE_SENTINEL));
  assert.doesNotMatch(markup, new RegExp(PLAINTEXT_SENTINEL));
});

test("write forecasts hide every mutation while immutable evidence remains readable", async () => {
  await seedDetail();
  const markup = renderToStaticMarkup(
    <AppProvider value={context(false)}>
      <ToolServerItem serverId={SERVER_ID} />
    </AppProvider>,
  );
  const text = toText(markup);
  assert.match(text, /does not forecast tool\.write/i);
  assert.match(text, /Quarantined changed version/i);
  assert.match(text, /Latest health/i);
  assert.match(text, /Policy does not offer binding changes/i);
  for (const action of [
    "Report stateless discovery",
    "Record trusted adapter test",
    "Disable",
    "Repin exact version",
    "Remove binding",
  ]) {
    assert.doesNotMatch(
      markup,
      new RegExp(`>${action.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}</button>`, "i"),
      action,
    );
  }
});

test("the catalogue is honest when no project is selected", async () => {
  await seed("tools/catalogue/first", { servers: [server()] });
  const text = toText(
    renderToStaticMarkup(
      <AppProvider value={context(false, false)}>
        <Tools />
      </AppProvider>,
    ),
  );
  assert.match(text, /Select a project before importing or binding/i);
  assert.match(text, /pulseboard-tools/i);
  assert.doesNotMatch(text, /Import MCP server/i);
  assert.doesNotMatch(text, /Generated client configuration/i);
});

test("a server with no visible version stops before trust binding and connectivity work", async () => {
  await Promise.all([
    seed(`tools/server/${SERVER_ID}`, server()),
    seed(`tools/server/${SERVER_ID}/versions`, { versions: [] }),
  ]);
  const text = toText(
    renderToStaticMarkup(
      <AppProvider value={context()}>
        <ToolServerItem serverId={SERVER_ID} />
      </AppProvider>,
    ),
  );
  assert.match(text, /No policy-visible immutable version exists/i);
  for (const unavailableCapability of [
    "Report stateless discovery",
    "Read-only connectivity evidence",
    "Project binding",
  ]) {
    assert.doesNotMatch(text, new RegExp(unavailableCapability, "i"));
  }
});

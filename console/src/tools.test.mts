/** Pure product rules for the CPR-26 MCP Tools catalogue. */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  capabilityEntries,
  descriptorForDisplay,
  diffCount,
  diffSections,
  displayJson,
  mayWriteToolsAt,
  MCP_PROTOCOL_VERSION,
  parseJsonObject,
  READ_ONLY_METHODS,
  toolMutationMessage,
  versionStateLabel,
} from "./tools.mjs";
import type {
  AnchorCapabilities,
  ToolServerDescriptorBody,
  ToolServerVersionView,
  ToolVersionDiffView,
} from "./generated/api.js";

function version(): ToolServerVersionView {
  return {
    id: "version-2",
    server_id: "server-1",
    change_id: "change-2",
    ordinal: 2,
    digest: "b".repeat(64),
    capability_digest: "c".repeat(64),
    protocol_version: MCP_PROTOCOL_VERSION,
    state: "quarantined",
    descriptor: {
      source_kind: "remote_http",
      source_reference: "registry:pulseboard",
      transport: "streamable_http",
      endpoint: "https://mcp.example.test/mcp",
      authentication: "oauth",
      secret_reference: "secret-ref://vault/opaque-reference",
      requested_permissions: ["issues:read"],
      metadata: { region: "eu-west" },
    },
    secret_reference_present: true,
    raw_capabilities: {},
    normalized_capabilities: {
      protocol_version: MCP_PROTOCOL_VERSION,
      tools: {
        entries: [
          {
            name: "lookup_issue",
            description: "Look up one issue",
            inputSchema: { type: "object", properties: { issue: { type: "string" } } },
            "x-owner": "platform",
          },
          {
            name: "unsafe_description",
            description: "Bearer console-plaintext-secret",
            inputSchema: { authorization: "Bearer another-secret" },
          },
        ],
      },
      resources: { entries: [{ uri: "repo://pulseboard/runbooks", name: "Runbooks" }] },
      prompts: { entries: [{ name: "triage", arguments: [{ name: "incident" }] }] },
    },
    declared_capabilities_are_authorization: false,
    discovered_at: "2026-08-25T10:00:00Z",
    created_at: "2026-08-25T10:00:00Z",
  };
}

test("tool.write forecasts are scope-specific offers, not tenant shortcuts", () => {
  const anchors = [
    {
      scope_id: "scope-project",
      kind: "project",
      source: "selected_project",
      direct: true,
      roles: ["member"],
      actions: { "tool.read": true, "tool.write": true },
    },
    {
      scope_id: "scope-other",
      kind: "project",
      source: "grant",
      direct: true,
      roles: ["viewer"],
      actions: { "tool.read": true, "tool.write": false },
    },
  ] as AnchorCapabilities[];
  assert.equal(mayWriteToolsAt(anchors, "scope-project"), true);
  assert.equal(mayWriteToolsAt(anchors, "scope-other"), false);
  assert.equal(mayWriteToolsAt(anchors, "scope-absent"), false);
});

test("normalised tools resources and prompts retain schemas and extension metadata", () => {
  const item = version();
  const tools = capabilityEntries(item, "tools");
  assert.equal(tools[0]?.identity, "lookup_issue");
  assert.equal(tools[0]?.description, "Look up one issue");
  assert.deepEqual(tools[0]?.schema, {
    type: "object",
    properties: { issue: { type: "string" } },
  });
  assert.equal(tools[0]?.details["x-owner"], "platform");
  assert.equal(capabilityEntries(item, "resources")[0]?.identity, "repo://pulseboard/runbooks");
  assert.equal(capabilityEntries(item, "prompts")[0]?.identity, "triage");
  assert.equal(tools[1]?.description, "[redacted credential]");
  assert.doesNotMatch(displayJson(tools[1]), /console-plaintext-secret|another-secret/);
});

test("version comparisons retain additions changes removals and descriptor drift", () => {
  const diff: ToolVersionDiffView = {
    from_version_id: "version-1",
    to_version_id: "version-2",
    descriptor_changed: ["authentication"],
    tools_added: ["deploy_release"],
    tools_changed: ["lookup_issue"],
    tools_removed: [],
    resources_added: [],
    resources_changed: [],
    resources_removed: ["repo://old"],
    prompts_added: [],
    prompts_changed: ["triage"],
    prompts_removed: [],
  };
  assert.equal(diffCount(diff), 5);
  assert.deepEqual(diffSections(diff), [
    {
      label: "Tools",
      added: ["deploy_release"],
      changed: ["lookup_issue"],
      removed: [],
    },
    { label: "Resources", added: [], changed: [], removed: ["repo://old"] },
    { label: "Prompts", added: [], changed: ["triage"], removed: [] },
  ]);
});

test("ordinary display exposes secret-reference status but never its opaque value or credentials", () => {
  const descriptor = version().descriptor as ToolServerDescriptorBody;
  const rendered = displayJson(descriptorForDisplay(descriptor));
  assert.match(rendered, /reference_status/);
  assert.match(rendered, /configured/);
  assert.doesNotMatch(rendered, /opaque-reference/);

  const configuration = displayJson({
    mcpServers: {
      pulseboard: {
        secretReference: "secret-ref://vault/reference-sentinel",
        headers: { Authorization: "Bearer credential-sentinel" },
        token: "sk-console-secret-token",
      },
    },
  });
  assert.match(configuration, /\[configured\]/);
  assert.match(configuration, /\[redacted\]/);
  assert.doesNotMatch(configuration, /reference-sentinel|credential-sentinel|console-secret-token/);
});

test("open metadata inputs require complete JSON objects", () => {
  assert.deepEqual(parseJsonObject('{"protocol_version":"2026-07-28"}', "Snapshot"), {
    protocol_version: "2026-07-28",
  });
  assert.throws(() => parseJsonObject("not-json", "Snapshot"), /valid JSON/);
  assert.throws(() => parseJsonObject("[]", "Snapshot"), /JSON object/);
  assert.throws(() => parseJsonObject('"value"', "Snapshot"), /JSON object/);
});

test("governed outcomes and read-only method vocabulary cannot imply execution", () => {
  assert.match(
    toolMutationMessage({ change_id: "c", outcome: "pending_review" }),
    /waiting for review.*unchanged/,
  );
  assert.match(toolMutationMessage({ change_id: "c", outcome: "applied" }), /applied/);
  assert.match(toolMutationMessage({ change_id: "c", outcome: "rejected" }), /rejected.*unchanged/);
  assert.equal(versionStateLabel("quarantined"), "Quarantined — review required");
  assert.deepEqual(READ_ONLY_METHODS, [
    "server/discover",
    "tools/list",
    "resources/list",
    "prompts/list",
  ]);
  assert.ok(!([...READ_ONLY_METHODS] as string[]).includes("tools/call"));
});

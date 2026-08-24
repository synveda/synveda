/** Pure product readings for the CPR-26 MCP Tools catalogue. */

import type {
  AnchorCapabilities,
  ToolMutationView,
  ToolServerDescriptorBody,
  ToolServerVersionView,
  ToolVersionDiffView,
} from "./generated/api.js";

export const MCP_PROTOCOL_VERSION = "2026-07-28";

export const READ_ONLY_METHODS = [
  "server/discover",
  "tools/list",
  "resources/list",
  "prompts/list",
] as const;

export type ToolCapabilityFamily = "tools" | "resources" | "prompts";

export interface ToolCapabilityReading {
  identity: string;
  description: string | null;
  schema: unknown | null;
  details: Record<string, unknown>;
}

export interface ToolDiffSection {
  label: string;
  added: string[];
  changed: string[];
  removed: string[];
}

/** A forecast improves the UI; the gateway still decides every mutation. */
export function mayWriteToolsAt(anchors: AnchorCapabilities[], scopeId: string): boolean {
  return anchors.some(
    (anchor) => anchor.scope_id === scopeId && anchor.actions["tool.write"] === true,
  );
}

/** Read one canonical capability family while retaining extension metadata. */
export function capabilityEntries(
  version: ToolServerVersionView,
  family: ToolCapabilityFamily,
): ToolCapabilityReading[] {
  const root = objectOf(version.normalized_capabilities);
  const collection = objectOf(root?.[family]);
  const entries = Array.isArray(collection?.entries) ? collection.entries : [];
  const identityKey = family === "resources" ? "uri" : "name";
  return entries.flatMap((entry): ToolCapabilityReading[] => {
    const object = objectOf(entry);
    if (!object) return [];
    const identity = object[identityKey];
    if (typeof identity !== "string" || identity.length === 0) return [];
    const safeDescription = sanitiseForDisplay(object.description);
    const schema =
      object.inputSchema ??
      object.input_schema ??
      object.outputSchema ??
      object.output_schema ??
      object.arguments ??
      null;
    return [
      {
        identity,
        description: typeof safeDescription === "string" ? safeDescription : null,
        schema: sanitiseForDisplay(schema),
        details: sanitiseForDisplay(object) as Record<string, unknown>,
      },
    ];
  });
}

/** Turn the wire diff into the three reader-facing capability families. */
export function diffSections(diff: ToolVersionDiffView): ToolDiffSection[] {
  return [
    {
      label: "Tools",
      added: diff.tools_added,
      changed: diff.tools_changed,
      removed: diff.tools_removed,
    },
    {
      label: "Resources",
      added: diff.resources_added,
      changed: diff.resources_changed,
      removed: diff.resources_removed,
    },
    {
      label: "Prompts",
      added: diff.prompts_added,
      changed: diff.prompts_changed,
      removed: diff.prompts_removed,
    },
  ];
}

export function diffCount(diff: ToolVersionDiffView): number {
  return (
    diff.descriptor_changed.length +
    diffSections(diff).reduce(
      (total, section) =>
        total + section.added.length + section.changed.length + section.removed.length,
      0,
    )
  );
}

/** The descriptor's secret-reference value is never ordinary console output. */
export function descriptorForDisplay(
  descriptor: ToolServerDescriptorBody,
): Record<string, unknown> {
  const { secret_reference: _secretReference, ...visible } = descriptor;
  return sanitiseForDisplay({
    ...visible,
    reference_status: _secretReference ? "configured" : "not configured",
  }) as Record<string, unknown>;
}

/**
 * Defensive output boundary for extensible JSON.
 *
 * CPR-25 rejects credential-shaped fields before persistence. The console
 * repeats the boundary because forward-compatible metadata and generated
 * client configuration are open objects: a later producer must not turn an
 * opaque reference or an accidentally embedded credential into DOM text.
 */
export function sanitiseForDisplay(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sanitiseForDisplay);
  if (typeof value === "string") return credentialShaped(value) ? "[redacted credential]" : value;
  const object = objectOf(value);
  if (!object) return value;
  return Object.fromEntries(
    Object.entries(object).map(([key, child]) => {
      const normal = key.toLowerCase().replace(/[^a-z0-9]/g, "");
      if (normal.includes("secretreference")) {
        return [key, child === null || child === undefined ? null : "[configured]"];
      }
      if (
        normal.includes("password") ||
        normal.includes("authorization") ||
        normal.includes("credential") ||
        normal.includes("privatekey") ||
        normal === "token" ||
        normal.endsWith("token") ||
        normal === "apikey" ||
        normal.endsWith("apikey")
      ) {
        return [key, "[redacted]"];
      }
      return [key, sanitiseForDisplay(child)];
    }),
  );
}

export function displayJson(value: unknown): string {
  return JSON.stringify(sanitiseForDisplay(value), null, 2);
}

/** Parse one complete JSON object; arrays and scalar shorthand are refused. */
export function parseJsonObject(input: string, label: string): Record<string, unknown> {
  let value: unknown;
  try {
    value = JSON.parse(input);
  } catch {
    throw new Error(`${label} must be valid JSON.`);
  }
  const object = objectOf(value);
  if (!object) throw new Error(`${label} must be a JSON object.`);
  return object;
}

export function toolMutationMessage(result: ToolMutationView): string {
  switch (result.outcome) {
    case "applied":
      return "The governed Tool change applied.";
    case "pending_review":
      return "The governed Tool change is waiting for review; approved versions and bindings are unchanged.";
    case "rejected":
      return "Policy rejected the governed Tool change; approved versions and bindings are unchanged.";
  }
}

export function versionStateLabel(state: ToolServerVersionView["state"]): string {
  switch (state) {
    case "approved":
      return "Approved";
    case "quarantined":
      return "Quarantined — review required";
    case "rejected":
      return "Rejected";
  }
}

function credentialShaped(value: string): boolean {
  return (
    /\bbearer\s+[a-z0-9._~+/=-]{6,}/i.test(value) ||
    /\b(?:sk|ghp|gho|github_pat|glpat|xox[baprs])[-_][a-z0-9_-]{8,}/i.test(value)
  );
}

function objectOf(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

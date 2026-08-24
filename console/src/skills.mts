/** Pure readings for the CPR-24 Skills Library. */

import type {
  AnchorCapabilities,
  MeView,
  ProjectView,
  SkillFileBody,
  SkillMutationView,
  SkillUsageEventView,
  SkillVersionView,
} from "./generated/api.js";

export interface SkillScopeOption {
  id: string;
  kind: "principal" | "project";
  label: string;
  canWrite: boolean;
}

/**
 * The only two product placements CPR-23 admits. A capability is an offer,
 * never authority; every mutation still meets the gateway PDP.
 */
export function skillScopes(me: MeView, project: ProjectView | null): SkillScopeOption[] {
  const options: SkillScopeOption[] = [];
  const personal = me.anchors.find(
    (anchor) => anchor.kind === "principal" && anchor.source === "principal_scope",
  );
  if (personal) {
    options.push({
      id: personal.scope_id,
      kind: "principal",
      label: "Private to me",
      canWrite: personal.actions["skill.write"] === true,
    });
  }
  if (project) {
    const anchor = me.anchors.find((candidate) => candidate.scope_id === project.scope_id);
    options.push({
      id: project.scope_id,
      kind: "project",
      label: `Project · ${project.display_name}`,
      canWrite: anchor?.actions["skill.write"] === true,
    });
  }
  return options;
}

export function mayWriteAt(anchors: AnchorCapabilities[], scopeId: string): boolean {
  return anchors.some(
    (anchor) => anchor.scope_id === scopeId && anchor.actions["skill.write"] === true,
  );
}

export interface SkillManifestSummary {
  description: string;
  license: string | null;
  compatibility: string | null;
  declaredTools: string[];
  extensions: Record<string, unknown>;
}

/** Reads the forward-compatible Agent Skills manifest without narrowing it. */
export function manifestSummary(manifest: Record<string, unknown>): SkillManifestSummary {
  const known = new Set(["name", "description", "license", "compatibility", "allowed-tools"]);
  const tools = manifest["allowed-tools"];
  const declaredTools = Array.isArray(tools)
    ? tools.filter((tool): tool is string => typeof tool === "string")
    : typeof tools === "string"
      ? tools.split(/\s+/).filter(Boolean)
      : [];
  return {
    description:
      typeof manifest.description === "string" && manifest.description.trim().length > 0
        ? manifest.description
        : "No description supplied.",
    license: typeof manifest.license === "string" ? manifest.license : null,
    compatibility:
      typeof manifest.compatibility === "string" ? manifest.compatibility : null,
    declaredTools,
    extensions: Object.fromEntries(
      Object.entries(manifest).filter(([key]) => !known.has(key)),
    ),
  };
}

export interface ScanFinding {
  path: string;
  rule: string;
  severity: string;
  line: number | null;
  count: number;
}

export interface ScanSummary {
  worst: string | null;
  blocksAt: string | null;
  findings: ScanFinding[];
}

/** Scanner evidence is extensible JSON; keep unknown evidence visible. */
export function scanSummary(scan: Record<string, unknown>): ScanSummary {
  const findings = Array.isArray(scan.findings)
    ? scan.findings.flatMap((value): ScanFinding[] => {
        if (!isObject(value)) return [];
        return [
          {
            path: stringOf(value.path, "bundle"),
            rule: stringOf(value.rule, "unknown-rule"),
            severity: stringOf(value.severity, "unknown"),
            line: typeof value.line === "number" ? value.line : null,
            count: typeof value.count === "number" ? value.count : 1,
          },
        ];
      })
    : [];
  return {
    worst: typeof scan.worst === "string" ? scan.worst : null,
    blocksAt: typeof scan.blocks_at === "string" ? scan.blocks_at : null,
    findings,
  };
}

export function sourceLabel(version: SkillVersionView): string {
  const reference = version.provenance.reference;
  const revision = version.provenance.revision;
  return [
    version.source_kind,
    typeof reference === "string" && reference.length > 0 ? reference : null,
    typeof revision === "string" && revision.length > 0 ? `at ${revision}` : null,
  ]
    .filter((part): part is string => part !== null)
    .join(" · ");
}

export function skillMutationMessage(result: SkillMutationView): string {
  switch (result.outcome) {
    case "applied":
      return "The governed change applied.";
    case "pending_review":
      return "The governed change is waiting for review; active bindings and versions are unchanged.";
    case "rejected":
      return "Policy rejected the governed change; active bindings and versions are unchanged.";
  }
}

export function evidenceLabel(evidence: SkillUsageEventView["evidence"]): string {
  return evidence === "host_observed" ? "Host-observed" : "Model-reported";
}

export function activationEvidence(events: SkillUsageEventView[]): {
  hostObserved: number;
  modelReported: number;
  activated: number;
} {
  return {
    hostObserved: events.filter((event) => event.evidence === "host_observed").length,
    modelReported: events.filter((event) => event.evidence === "model_reported").length,
    activated: events.filter((event) => event.stage === "activated").length,
  };
}

/** Parse a complete replacement bundle without inventing a partial update. */
export function parseBundleFiles(input: string): SkillFileBody[] {
  const value: unknown = JSON.parse(input);
  if (!Array.isArray(value) || value.length === 0) {
    throw new Error("Bundle files must be a non-empty JSON array.");
  }
  const seen = new Set<string>();
  return value.map((file, index) => {
    if (!isObject(file) || typeof file.path !== "string" || typeof file.content !== "string") {
      throw new Error(`Bundle file ${index + 1} must contain string path and content fields.`);
    }
    const path = file.path.trim();
    if (path.length === 0) throw new Error(`Bundle file ${index + 1} has an empty path.`);
    if (seen.has(path)) throw new Error(`Bundle path ${path} appears more than once.`);
    seen.add(path);
    return { path, content: file.content };
  });
}

export function formatBundleFiles(files: SkillFileBody[]): string {
  return JSON.stringify(files, null, 2);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function stringOf(value: unknown, fallback: string): string {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

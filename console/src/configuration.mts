/** Pure presentation helpers for governed runtime Configuration (CPR-30). */

import type {
  ConfigurationDocumentBody,
  ConfigurationMutationView,
  MeView,
  ProjectView,
  WorkspaceView,
} from "./generated/api.js";

export interface ConfigurationTarget {
  id: string;
  label: string;
}

/** Nearest product scope selected by the reader, then their own scope. */
export function configurationTarget(
  me: MeView,
  workspace: WorkspaceView | null,
  project: ProjectView | null,
): ConfigurationTarget | null {
  if (project) return { id: project.scope_id, label: `project ${project.slug}` };
  if (workspace) return { id: workspace.scope_id, label: `workspace ${workspace.slug}` };
  const principal = me.anchors.find((anchor) => anchor.source === "principal_scope");
  return principal ? { id: principal.scope_id, label: "your private scope" } : null;
}

/** Stable, readable JSON is also the exact shape the publish form accepts. */
export function renderConfiguration(document: ConfigurationDocumentBody): string {
  return JSON.stringify(document, null, 2);
}

/** Parse an edited complete document without pretending the browser is its validator. */
export function parseConfiguration(text: string): ConfigurationDocumentBody {
  const parsed: unknown = JSON.parse(text);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("configuration must be one complete JSON object");
  }
  return parsed as ConfigurationDocumentBody;
}

export function configurationSummary(document: ConfigurationDocumentBody): string {
  return `${document.policy_pack} · ${document.context.token_budget} tokens · ${document.context.trace_retention} traces · Skills ${document.advertisement.skills ? "on" : "off"} · Tools ${document.advertisement.tools ? "on" : "off"}`;
}

export function mutationMessage(result: ConfigurationMutationView): string {
  switch (result.outcome) {
    case "applied":
      return `Applied through VedaFlow change ${result.change_id}.`;
    case "pending_review":
      return `Waiting in Advanced Reviews as change ${result.change_id}; runtime selection is unchanged.`;
    case "rejected":
      return `Rejected as change ${result.change_id}; runtime selection is unchanged.`;
  }
}

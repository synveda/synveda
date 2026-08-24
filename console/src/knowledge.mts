/** Pure Knowledge-browser vocabulary and query shaping (CPR-17, ADR-0082). */

import type { KnowledgeItemView, KnowledgeMutationView } from "./generated/api.js";

export const KNOWLEDGE_TYPES = [
  "fact",
  "decision",
  "preference",
  "procedure",
  "entity",
  "episode",
  "convention",
  "warning",
  "reference",
] as const;

export const ORIGINS = ["observed", "asserted", "authored", "imported"] as const;
export const LIFECYCLES = [
  "active",
  "stale",
  "superseded",
  "archived",
  "erasure_pending",
  "erased",
] as const;
export const SOURCE_TYPES = [
  "session_event",
  "manual",
  "document",
  "repository",
  "url",
  "okf",
  "system_derived",
] as const;
export const SENSITIVITIES = ["public", "internal", "confidential", "restricted"] as const;

export interface KnowledgeFilters {
  query: string;
  workspaceId: string;
  projectId: string;
  scopeId: string;
  owner: string;
  knowledgeType: string;
  origin: string;
  lifecycle: string;
  tag: string;
  source: string;
  updatedFrom: string;
  updatedBefore: string;
  stale: "" | "true" | "false";
}

export const EMPTY_KNOWLEDGE_FILTERS: KnowledgeFilters = {
  query: "",
  workspaceId: "",
  projectId: "",
  scopeId: "",
  owner: "",
  knowledgeType: "",
  origin: "",
  lifecycle: "",
  tag: "",
  source: "",
  updatedFrom: "",
  updatedBefore: "",
  stale: "",
};

/** Converts browser controls into the generated client's string-only query. */
export function knowledgeQuery(
  filters: KnowledgeFilters,
  cursor: string | null,
): Record<string, string | undefined> {
  const value = (raw: string): string | undefined => raw.trim() || undefined;
  const instant = (raw: string): string | undefined => {
    const present = value(raw);
    return present === undefined ? undefined : new Date(present).toISOString();
  };
  return {
    query: value(filters.query),
    workspace_id: value(filters.workspaceId),
    project_id: value(filters.projectId),
    scope_id: value(filters.scopeId),
    owner: value(filters.owner),
    knowledge_type: value(filters.knowledgeType),
    origin: value(filters.origin),
    lifecycle_state: value(filters.lifecycle),
    tag: value(filters.tag),
    source: value(filters.source),
    updated_from: instant(filters.updatedFrom),
    updated_before: instant(filters.updatedBefore),
    stale: value(filters.stale),
    cursor: cursor ?? undefined,
    limit: "50",
  };
}

export function knowledgeIsFiltered(filters: KnowledgeFilters): boolean {
  return Object.values(filters).some((value) => value.length > 0);
}

/** Scope wording is a reading of the aggregate, not an authorisation guess. */
export function visibilityLabel(item: KnowledgeItemView): string {
  if (item.owner_principal_id) return `Private to ${item.owner_principal_id}`;
  if (item.project_id) return "Shared with project";
  return "Shared at governed scope";
}

export function mutationMessage(result: KnowledgeMutationView): string {
  switch (result.outcome) {
    case "applied":
      return `Applied through VedaFlow change ${result.change_id}.`;
    case "pending_review":
      return `Waiting for review as VedaFlow change ${result.change_id}.`;
    case "rejected":
      return `Rejected as VedaFlow change ${result.change_id}; no Knowledge state changed.`;
  }
}

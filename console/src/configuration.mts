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

export const TRACE_RETENTION_OPTIONS = ["full", "redacted", "hashes_only", "disabled"] as const;
export type TraceRetention = (typeof TRACE_RETENTION_OPTIONS)[number];

/** The deliberately small normal editor for the local product walkthrough. */
export interface DemoConfigurationDraft {
  captureEnabled: boolean;
  captureOnSessionEnd: boolean;
  captureExplicitRequest: boolean;
  captureMinimumConfidencePermille: string;
  captureMaximumCandidatesPerBatch: string;
  contextTokenBudget: string;
  contextTraceRetention: TraceRetention;
  contextIncludeUnreviewedCandidates: boolean;
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

/** Populate the bounded form from one exact immutable version. */
export function demoConfigurationDraft(
  document: ConfigurationDocumentBody,
): DemoConfigurationDraft {
  return {
    captureEnabled: document.capture.enabled,
    captureOnSessionEnd: document.capture.on_session_end,
    captureExplicitRequest: document.capture.explicit_request,
    captureMinimumConfidencePermille: String(document.capture.minimum_confidence_permille),
    captureMaximumCandidatesPerBatch: String(document.capture.maximum_candidates_per_batch),
    contextTokenBudget: String(document.context.token_budget),
    contextTraceRetention: document.context.trace_retention,
    contextIncludeUnreviewedCandidates: document.context.channels.includes(
      "unreviewed_candidates",
    ),
  };
}

/**
 * Apply only the supported demo fields while preserving the rest of the full
 * immutable document. These are the same bounds and cross-field rule enforced
 * by `synveda_types::configuration`; the gateway remains the canonical
 * validator and rechecks the complete body.
 */
export function applyDemoConfigurationDraft(
  document: ConfigurationDocumentBody,
  draft: DemoConfigurationDraft,
): ConfigurationDocumentBody {
  const minimumConfidence = boundedInteger(
    "Capture minimum confidence",
    draft.captureMinimumConfidencePermille,
    0,
    1_000,
  );
  const maximumCandidates = boundedInteger(
    "Capture maximum candidates per batch",
    draft.captureMaximumCandidatesPerBatch,
    1,
    256,
  );
  const contextBudget = boundedInteger(
    "Context token budget",
    draft.contextTokenBudget,
    1,
    100_000,
  );
  if (
    !draft.captureEnabled &&
    (draft.captureOnSessionEnd || draft.captureExplicitRequest)
  ) {
    throw new Error("Disabled Capture cannot keep session-end or explicit extraction enabled.");
  }
  if (!TRACE_RETENTION_OPTIONS.includes(draft.contextTraceRetention)) {
    throw new Error("Context trace retention is not a supported contract value.");
  }
  const channels = document.context.channels.filter(
    (channel) => channel !== "unreviewed_candidates",
  );
  if (draft.contextIncludeUnreviewedCandidates) {
    channels.push("unreviewed_candidates");
  }

  return {
    ...document,
    capture: {
      ...document.capture,
      enabled: draft.captureEnabled,
      on_session_end: draft.captureOnSessionEnd,
      explicit_request: draft.captureExplicitRequest,
      minimum_confidence_permille: minimumConfidence,
      maximum_candidates_per_batch: maximumCandidates,
    },
    context: {
      ...document.context,
      token_budget: contextBudget,
      trace_retention: draft.contextTraceRetention,
      channels,
    },
  };
}

function boundedInteger(label: string, value: string, minimum: number, maximum: number): number {
  const parsed = Number(value);
  if (value.trim().length === 0 || !Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${label} must be a whole number from ${minimum} through ${maximum}.`);
  }
  return parsed;
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

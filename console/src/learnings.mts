/**
 * Pure decisions for New Learnings (CPR-19).
 *
 * Capture candidates are review input, not a second proposal model. This
 * module shapes filters and the generated capture-command bodies, and derives
 * the small set of publication destinations the caller may be offered from
 * `/v1/me`. The forecast only controls what the console offers; the gateway
 * repeats the real PDP decision for every mutation.
 */

import type {
  AcceptCandidateBody,
  CaptureBatchView,
  CaptureCandidateView,
  CaptureMatchView,
  DismissCandidateBody,
  MeView,
  MergeCandidateBody,
  ReplaceCandidateBody,
} from "./generated/api.js";

export const CANDIDATE_STATES = [
  "pending",
  "accepted",
  "edited_and_accepted",
  "merged",
  "replaced",
  "dismissed",
  "failed",
] as const;

export type CandidateState = (typeof CANDIDATE_STATES)[number];

export interface LearningsFilters {
  projectId: string;
  sessionId: string;
  state: "" | CandidateState;
}

export const EMPTY_LEARNINGS_FILTERS: LearningsFilters = {
  projectId: "",
  sessionId: "",
  state: "",
};

/** The two collection queries deliberately share their placement filters. */
export function candidateQuery(
  filters: LearningsFilters,
  cursor: string | null,
): Record<string, string | undefined> {
  return {
    project_id: present(filters.projectId),
    session_id: present(filters.sessionId),
    // State is a display filter. Reading all states is what lets each batch
    // show truthful progress and resulting Knowledge links in one view.
    cursor: cursor ?? undefined,
    limit: "200",
  };
}

export function batchQuery(
  filters: LearningsFilters,
  cursor: string | null,
): Record<string, string | undefined> {
  return {
    project_id: present(filters.projectId),
    session_id: present(filters.sessionId),
    cursor: cursor ?? undefined,
    limit: "100",
  };
}

function present(value: string): string | undefined {
  return value.trim() || undefined;
}

/** One user-facing publication placement. Nulls are meaningful overrides. */
export interface PublishScope {
  id: string;
  label: string;
  visibility: "private" | "project" | "workspace";
  projectId: string | null;
  ownerPrincipalId: string | null;
}

/**
 * The publication choices relevant to one run.
 *
 * Personal, project and workspace are one domain model. They differ only in
 * placement and policy. A scope without a positive `knowledge.write`
 * forecast never becomes an `<option>`; a stale positive forecast still has
 * to pass the gateway's exact decision.
 */
export function publishableScopes(
  me: MeView,
  context: { projectId?: string | null; sourceScopeId?: string | null } = {},
): PublishScope[] {
  const candidates: PublishScope[] = [];

  for (const anchor of me.anchors) {
    if (anchor.source === "principal_scope") {
      candidates.push({
        id: anchor.scope_id,
        label: "Private to me",
        visibility: "private",
        projectId: null,
        ownerPrincipalId: me.principal.subject,
      });
    }
  }

  const project = context.projectId
    ? me.projects.find((entry) => entry.id === context.projectId)
    : me.projects.find((entry) => entry.scope_id === context.sourceScopeId);
  if (project) {
    candidates.push({
      id: project.scope_id,
      label: `Shared with project · ${project.display_name}`,
      visibility: "project",
      projectId: project.id,
      ownerPrincipalId: null,
    });
    const workspace = me.workspaces.find((entry) => entry.id === project.workspace_id);
    if (workspace) {
      candidates.push({
        id: workspace.scope_id,
        label: `Shared at workspace · ${workspace.display_name}`,
        visibility: "workspace",
        projectId: null,
        ownerPrincipalId: null,
      });
    }
  } else {
    const workspace = me.workspaces.find((entry) => entry.scope_id === context.sourceScopeId);
    if (workspace) {
      candidates.push({
        id: workspace.scope_id,
        label: `Shared at workspace · ${workspace.display_name}`,
        visibility: "workspace",
        projectId: null,
        ownerPrincipalId: null,
      });
    }
  }

  return candidates.filter((candidate, index) => {
    if (candidates.findIndex((entry) => entry.id === candidate.id) !== index) return false;
    const anchor = me.anchors.find((entry) => entry.scope_id === candidate.id);
    return anchor?.actions["knowledge.write"] === true;
  });
}

/** The currently proposed placement, only when it is still publishable. */
export function proposedPublishScope(
  candidate: CaptureCandidateView,
  scopes: PublishScope[],
): PublishScope | null {
  return (
    scopes.find(
      (scope) =>
        scope.id === candidate.proposed_scope_id &&
        scope.projectId === (candidate.proposed_project_id ?? null) &&
        scope.ownerPrincipalId === (candidate.proposed_owner_principal_id ?? null),
    ) ?? null
  );
}

/**
 * Placement overrides for a candidate command.
 *
 * Unchanged fields are omitted so an ordinary Accept remains `accepted`, not
 * `edited_and_accepted`. Moving from project/private placement requires
 * explicit JSON nulls, hence the comparisons against normalised null values.
 */
export function placementEdits(
  candidate: CaptureCandidateView,
  target: PublishScope,
): AcceptCandidateBody {
  const body: AcceptCandidateBody = {};
  if (target.id !== candidate.proposed_scope_id) body.scope_id = target.id;
  if (target.projectId !== (candidate.proposed_project_id ?? null)) {
    body.project_id = target.projectId;
  }
  if (target.ownerPrincipalId !== (candidate.proposed_owner_principal_id ?? null)) {
    body.owner_principal_id = target.ownerPrincipalId;
  }
  return body;
}

export function editAndAcceptBody(
  candidate: CaptureCandidateView,
  target: PublishScope,
  edit: {
    knowledgeType: string;
    content: CaptureCandidateView["content"];
  },
): AcceptCandidateBody {
  const body = placementEdits(candidate, target);
  body.content = edit.content;
  if (edit.knowledgeType !== candidate.knowledge_type) {
    body.knowledge_type = edit.knowledgeType;
  }
  return body;
}

export function mergeBody(
  candidate: CaptureCandidateView,
  target: PublishScope,
  matched: CaptureMatchView,
): MergeCandidateBody {
  const result = placementEdits(candidate, target);
  return {
    inputs: [
      {
        item_id: matched.knowledge_item_id,
        revision_id: matched.knowledge_revision_id,
      },
    ],
    ...(Object.keys(result).length === 0 ? {} : { result }),
  };
}

export function replaceBody(
  candidate: CaptureCandidateView,
  target: PublishScope,
  matched: CaptureMatchView,
): ReplaceCandidateBody {
  const replacement = placementEdits(candidate, target);
  return {
    item_id: matched.knowledge_item_id,
    expected_revision_id: matched.knowledge_revision_id,
    ...(Object.keys(replacement).length === 0 ? {} : { replacement }),
  };
}

export function dismissBody(reason: string): DismissCandidateBody {
  const trimmed = reason.trim();
  return trimmed.length === 0 ? {} : { reason: trimmed };
}

/** Human wording for the candidate's proposed/resulting placement. */
export function candidateVisibility(candidate: CaptureCandidateView): string {
  if (candidate.proposed_owner_principal_id) return "Private to the principal";
  if (candidate.proposed_project_id) return "Shared with the project";
  return "Shared at workspace scope";
}

export function stateLabel(state: string): string {
  return state.replaceAll("_", " ");
}

export function matchLabel(kind: string): string {
  switch (kind) {
    case "duplicate":
      return "Near duplicate";
    case "conflict":
      return "Likely conflict";
    case "possible_supersession":
      return "Possible replacement";
    default:
      return stateLabel(kind);
  }
}

/** One batch's loaded review progress, never pretending a partial page is all. */
export function batchProgress(
  batch: CaptureBatchView,
  candidates: CaptureCandidateView[],
): string {
  const reviewed = candidates.filter((candidate) => candidate.state !== "pending").length;
  const pending = candidates.filter((candidate) => candidate.state === "pending").length;
  const loaded = candidates.length;
  const total = batch.candidate_count;
  const prefix = `${reviewed} reviewed · ${pending} pending`;
  return loaded < total ? `${prefix} · ${loaded} of ${total} loaded` : `${prefix} · ${total} total`;
}

export interface CandidateGroup {
  batch: CaptureBatchView | null;
  batchId: string;
  candidates: CaptureCandidateView[];
}

/** Batches stay visible as groups; candidate rows remain stable by ordinal. */
export function groupCandidates(
  batches: CaptureBatchView[],
  candidates: CaptureCandidateView[],
  state: LearningsFilters["state"],
): CandidateGroup[] {
  const grouped = new Map<string, CandidateGroup>();
  for (const batch of batches) {
    grouped.set(batch.id, { batch, batchId: batch.id, candidates: [] });
  }
  for (const candidate of candidates) {
    const group = grouped.get(candidate.batch_id) ?? {
      batch: null,
      batchId: candidate.batch_id,
      candidates: [],
    };
    group.candidates.push(candidate);
    grouped.set(candidate.batch_id, group);
  }
  return [...grouped.values()]
    .map((group) => ({
      ...group,
      candidates: group.candidates
        .filter((candidate) => state === "" || candidate.state === state)
        .sort((left, right) => left.ordinal - right.ordinal),
    }))
    .filter((group) => state === "" || group.candidates.length > 0)
    .sort((left, right) => {
      const leftAt = left.batch?.created_at ?? left.candidates[0]?.created_at ?? "";
      const rightAt = right.batch?.created_at ?? right.candidates[0]?.created_at ?? "";
      return rightAt.localeCompare(leftAt) || right.batchId.localeCompare(left.batchId);
    });
}

/** The lightweight workflow's result sentence. */
export function decisionMessage(candidate: CaptureCandidateView): string | null {
  if (candidate.state === "pending") return null;
  if (candidate.state === "dismissed") {
    return "Dismissed. No Knowledge was published.";
  }
  if (candidate.state === "failed") {
    return "This decision failed. The candidate remains outside active Knowledge.";
  }
  const change = candidate.resulting_change_id
    ? `VedaFlow change ${candidate.resulting_change_id}`
    : "the governed VedaFlow change";
  switch (candidate.resulting_outcome) {
    case "applied":
      return `${stateLabel(candidate.state)} and published through ${change}.`;
    case "pending_review":
      return `${stateLabel(candidate.state)} through ${change}; waiting in Advanced Reviews and not active Knowledge yet.`;
    case "rejected":
      return `${change} was rejected. No Knowledge state changed.`;
    default:
      return `${stateLabel(candidate.state)}; the governed result is still being resolved.`;
  }
}

export function appendBatches(
  seen: CaptureBatchView[],
  next: CaptureBatchView[],
): CaptureBatchView[] {
  const rows = new Map(seen.map((batch) => [batch.id, batch]));
  for (const batch of next) rows.set(batch.id, batch);
  return [...rows.values()];
}

export function appendCandidates(
  seen: CaptureCandidateView[],
  next: CaptureCandidateView[],
): CaptureCandidateView[] {
  const rows = new Map(seen.map((candidate) => [candidate.id, candidate]));
  for (const candidate of next) rows.set(candidate.id, candidate);
  return [...rows.values()];
}

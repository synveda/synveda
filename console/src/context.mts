/**
 * Pure presentation rules for the Context Inspector (CPR-21).
 *
 * The generated contract deliberately makes retained revision and score
 * detail nullable: redacted, hashes-only and disabled traces are different
 * disclosure products, not partially broken full traces. These helpers keep
 * that distinction out of JSX conditionals and make the feedback address
 * testable without a browser event harness.
 */

import type {
  ContextCandidateView,
  ContextFeedbackBody,
  ContextScoreView,
  ContextSelectionView,
  KnowledgeRevisionView,
} from "./generated/api.js";

export const FEEDBACK_TYPES = [
  "referenced_by_agent",
  "accepted_by_user",
  "helpful",
  "unhelpful",
  "caused_correction",
] as const;

export type FeedbackType = (typeof FEEDBACK_TYPES)[number];

const REASON_LABELS: Record<string, string> = {
  semantic_match: "semantic match",
  keyword_match: "keyword match",
  project_convention: "project convention",
  personal_preference: "personal preference",
  freshness_boost: "freshness boost",
  explicit_pin: "explicit pin",
  superseded: "superseded",
  stale: "stale",
  outside_task_scope: "outside task scope",
  token_budget: "token budget",
  duplicate: "duplicate",
};

const FEEDBACK_LABELS: Record<FeedbackType, string> = {
  referenced_by_agent: "Referenced by agent",
  accepted_by_user: "Accepted by user",
  helpful: "Helpful",
  unhelpful: "Unhelpful",
  caused_correction: "Caused correction",
};

/** Human copy that never turns a reduced-retention trace into an empty claim. */
export function retentionDescription(mode: string): string {
  switch (mode) {
    case "full":
      return "Full trace retained: the task, rendered context, visible Knowledge content, source evidence and score components are available.";
    case "redacted":
      return "Redacted trace: the task, rendered context, Knowledge content, sources and scores were not retained; visible revision addresses and reason codes remain.";
    case "hashes_only":
      return "Hashes-only trace: content hashes, ranks, token costs and reason codes remain; Knowledge addresses, content, evidence and feedback targets were not retained.";
    case "disabled":
      return "Trace retention was disabled: only the immutable delivery envelope, budgets, versions and degradation state remain.";
    default:
      return `Trace retention mode ${mode} is not understood by this console.`;
  }
}

export function reasonLabel(reason: string): string {
  return REASON_LABELS[reason] ?? reason.replaceAll("_", " ");
}

export function feedbackLabel(feedback: string): string {
  return FEEDBACK_TYPES.includes(feedback as FeedbackType)
    ? FEEDBACK_LABELS[feedback as FeedbackType]
    : feedback.replaceAll("_", " ");
}

/** The candidate row that explains a selected revision. */
export function candidateForSelection(
  candidates: readonly ContextCandidateView[],
  selection: ContextSelectionView,
): ContextCandidateView | null {
  const captureCandidateId = selection.capture_candidate_id;
  if (captureCandidateId) {
    const exact = candidates.find(
      (candidate) => candidate.capture_candidate_id === captureCandidateId,
    );
    if (exact) return exact;
  }
  const revisionId = selection.knowledge_revision_id;
  if (revisionId) {
    const exact = candidates.find((candidate) => candidate.knowledge_revision_id === revisionId);
    if (exact) return exact;
  }
  return candidates.find((candidate) => candidate.content_hash === selection.content_hash) ?? null;
}

/** Nullable generated content narrowed only after its required shape is present. */
export function revisionOf(value: unknown): KnowledgeRevisionView | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<KnowledgeRevisionView>;
  return typeof candidate.id === "string" &&
    typeof candidate.title === "string" &&
    typeof candidate.body_markdown === "string"
    ? (candidate as KnowledgeRevisionView)
    : null;
}

/** Nullable generated score detail narrowed without trusting a cast at render time. */
export function scoresOf(value: unknown): ContextScoreView | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<ContextScoreView>;
  return typeof candidate.final_micros === "number" &&
    typeof candidate.keyword_micros === "number" &&
    typeof candidate.semantic_micros === "number" &&
    typeof candidate.freshness_micros === "number" &&
    typeof candidate.pin_micros === "number" &&
    typeof candidate.current_state_micros === "number"
    ? (candidate as ContextScoreView)
    : null;
}

export function selectionState(
  candidate: ContextCandidateView | null,
  revision: KnowledgeRevisionView | null,
): string {
  if (candidate?.channel === "unreviewed_candidates") return "unreviewed at planning time";
  if (candidate?.lifecycle_state === "superseded") return "superseded at planning time";
  if (candidate?.lifecycle_state === "stale" || revision?.stale) return "stale at planning time";
  if (candidate?.lifecycle_state === "active") return "current at planning time";
  if (candidate?.lifecycle_state) return candidate.lifecycle_state.replaceAll("_", " ");
  return "state not retained";
}

/** Per-million integer contribution rendered without floating-point ambiguity. */
export function scorePercent(micros: number): string {
  const value = (micros / 10_000).toFixed(2).replace(/\.00$/, "").replace(/(\.\d)0$/, "$1");
  return `${value}%`;
}

/** A feedback request always judges this run's exact selection and revision. */
export function feedbackBody(
  selection: ContextSelectionView,
  feedbackType: FeedbackType,
): ContextFeedbackBody | null {
  if (!selection.knowledge_revision_id) return null;
  return {
    context_selection_id: selection.id,
    knowledge_revision_id: selection.knowledge_revision_id,
    feedback_type: feedbackType,
  };
}

export function canGiveFeedback(selection: ContextSelectionView): boolean {
  return selection.channel === "current_knowledge" &&
    Boolean(selection.knowledge_item_id && selection.knowledge_revision_id);
}

export function excludedCandidates(
  candidates: readonly ContextCandidateView[],
): ContextCandidateView[] {
  return candidates.filter((candidate) => Boolean(candidate.exclusion_reason));
}

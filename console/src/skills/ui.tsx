import type { Answer } from "../client.mjs";
import { Link } from "../Router.js";
import { hrefOf } from "../routes.mjs";
import { skillMutationMessage } from "../skills.mjs";
import type { SkillMutationView, SkillVersionView } from "../generated/api.js";

export type Sensitivity = SkillVersionView["sensitivity"];

export type MutationNotice =
  | { kind: "result"; result: SkillMutationView }
  | { kind: "error"; message: string };

interface MutationSideEffects {
  invalidate: (...prefixes: string[]) => void;
  navigateToSkill?: (skillId: string) => void;
}

export function applyMutationOutcome(
  notice: MutationNotice,
  invalidations: readonly string[],
  sideEffects: MutationSideEffects,
): void {
  if (notice.kind !== "result" || notice.result.outcome !== "applied") return;
  sideEffects.invalidate(...invalidations);
  if (notice.result.skill_id && sideEffects.navigateToSkill) {
    sideEffects.navigateToSkill(notice.result.skill_id);
  }
}

export function SensitivitySelect({
  value,
  onChange,
}: {
  value: Sensitivity;
  onChange: (value: Sensitivity) => void;
}) {
  return (
    <label>
      Sensitivity
      <select value={value} onChange={(event) => onChange(event.target.value as Sensitivity)}>
        {(["public", "internal", "confidential", "restricted"] as const).map((level) => (
          <option key={level}>{level}</option>
        ))}
      </select>
    </label>
  );
}

export function MutationNoticeView({ notice }: { notice: MutationNotice | null }) {
  if (!notice) return null;
  if (notice.kind === "error") {
    return (
      <div className="banner error" role="alert">
        {notice.message}
      </div>
    );
  }
  return (
    <div
      className={`banner ${notice.result.outcome === "rejected" ? "error" : "success"}`}
      role="status"
    >
      {skillMutationMessage(notice.result)} Change {notice.result.change_id}.{" "}
      {notice.result.outcome === "pending_review" ? (
        <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
      ) : null}
    </div>
  );
}

export function JsonValue({ value, empty }: { value: Record<string, unknown>; empty: string }) {
  if (Object.keys(value).length === 0) return <p className="muted">{empty}</p>;
  return <pre className="json-value">{JSON.stringify(value, null, 2)}</pre>;
}

export function noticeOf(answer: Answer<SkillMutationView>): MutationNotice {
  return answer.kind === "ok"
    ? { kind: "result", result: answer.body }
    : { kind: "error", message: failedAnswerMessage(answer) };
}

export function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "The bundle could not be read.";
}

export function failedAnswerMessage(answer: Exclude<Answer<unknown>, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

import type { Answer } from "../client.mjs";
import { Link } from "../Router.js";
import { hrefOf } from "../routes.mjs";
import { MCP_PROTOCOL_VERSION, toolMutationMessage } from "../tools.mjs";
import type { ToolMutationView, ToolTestRunView } from "../generated/api.js";

export type ToolMutationNoticeValue =
  | { kind: "result"; value: ToolMutationView }
  | { kind: "error"; message: string };
export type ToolTestNoticeValue =
  | { kind: "result"; value: ToolTestRunView }
  | { kind: "error"; message: string };

export const INITIAL_CAPABILITIES = JSON.stringify(
  {
    protocol_version: MCP_PROTOCOL_VERSION,
    server_info: { name: "pulseboard-tools", version: "1.0.0" },
    tools: [],
    resources: [],
    prompts: [],
  },
  null,
  2,
);

export function ToolMutationNotice({ notice }: { notice: ToolMutationNoticeValue | null }) {
  if (!notice) return null;
  if (notice.kind === "error") {
    return (
      <div className="banner error" role="alert">
        {notice.message}
      </div>
    );
  }
  const tone =
    notice.value.outcome === "rejected"
      ? "error"
      : notice.value.outcome === "pending_review"
        ? "warning"
        : "success";
  return (
    <div className={`banner ${tone}`} role="status">
      {toolMutationMessage(notice.value)} Change {notice.value.change_id}.{" "}
      {notice.value.outcome === "pending_review" ? (
        <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
      ) : null}
    </div>
  );
}

export function ToolTestNotice({ notice }: { notice: ToolTestNoticeValue | null }) {
  if (!notice) return null;
  if (notice.kind === "error") {
    return (
      <div className="banner error" role="alert">
        {notice.message}
      </div>
    );
  }
  return (
    <div className="banner success" role="status">
      Recorded immutable {notice.value.outcome} report {notice.value.id} for exact version{" "}
      {notice.value.version_id}.
    </div>
  );
}

export function mutationNotice(answer: Answer<ToolMutationView>): ToolMutationNoticeValue {
  return answer.kind === "ok"
    ? { kind: "result", value: answer.body }
    : { kind: "error", message: failedAnswerMessage(answer) };
}

export function failedAnswerMessage(answer: Exclude<Answer<unknown>, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

export function inputErrorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "The JSON input could not be read.";
}

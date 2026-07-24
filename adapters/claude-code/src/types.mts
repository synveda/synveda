/**
 * Wire and hook types (ADPT-1, ADR-0027).
 *
 * Everything the adapter reads from the harness is optional and checked
 * at the edge: a hook payload and a session transcript are internal
 * formats of another program (ADR-0027 decision 9), so the adapter
 * declares only the fields it uses and treats the rest as opaque.
 */

/** The subset of a Claude Code hook payload the adapter reads. */
export interface HookInput {
  /** `SessionStart` | `Stop` | `PreCompact` | `SessionEnd`. */
  hook_event_name?: string;
  session_id?: string;
  /** Absent on `PreCompact` — which is why the spool holds it. */
  transcript_path?: string;
  cwd?: string;
  /** `SessionStart` only: startup | resume | clear | compact | fork. */
  source?: string;
  model?: string;
}

/**
 * What a hook prints on stdout. Of the four events this adapter
 * registers for, only `SessionStart` can contribute context; the
 * observe hooks print nothing at all (ADR-0027 decision 2).
 */
export interface HookOutput {
  systemMessage?: string;
  hookSpecificOutput?: {
    hookEventName: string;
    additionalContext?: string;
  };
}

/** `POST /v1/inject` request (CTX-3, ADR-0026 decision 1). */
export interface InjectRequest {
  task?: string;
  session_id?: string;
  budget_tokens?: number;
}

/** `POST /v1/inject` response. */
export interface InjectResponse {
  text: string;
  block_hash: string;
  record_ids: string[];
  tokens: number;
  budget_tokens: number;
  as_of: string;
  degraded: string[];
}

/** The observe vocabulary (`ObserveKind` in synveda-types). */
export type ObserveKind = "transcript_delta" | "tool_result" | "decision";

/** One buffered event (MEM-1, ADR-0020). */
export interface ObserveEvent {
  idempotency_key: string;
  kind: ObserveKind;
  payload: unknown;
  occurred_at: string;
}

/** `POST /v1/observe` request. */
export interface ObserveRequest {
  session_id: string;
  events: ObserveEvent[];
}

/** `POST /v1/observe` response — the counts the adapter logs. */
export interface ObserveResponse {
  session_id: string;
  accepted: number;
  duplicates: number;
  quarantined: number;
  denied: number;
}

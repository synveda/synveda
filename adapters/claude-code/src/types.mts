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
  /** Present on all four events today; the spool holds one regardless. */
  transcript_path?: string;
  cwd?: string;
  /** `SessionStart` only: startup | resume | clear | compact | fork. */
  source?: string;
  /** `SessionStart` only, which is why the spool carries it to the flush. */
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

/**
 * `POST /v1/recall` request (CTX-4 ADR-0041; CTX-5 ADR-0042 decision 1).
 *
 * `ids` xor `query` — the gateway rejects both together, and the MCP tool
 * checks it first so an agent gets a usable message rather than a 400.
 */
export interface RecallRequest {
  ids?: string[];
  query?: string;
  as_of?: string;
  valid_at?: string;
  limit?: number;
  session_id?: string;
}

/** One record a recall served, with the labels that let an agent weigh it. */
export interface RecallEntry {
  record_id: string;
  scope_id: string;
  channel: string;
  kind: string;
  class: string;
  sensitivity: string;
  content: string;
  valid_from: string;
  valid_to: string | null;
  object_hash: string;
  staleness_permille: number;
}

/** `POST /v1/recall` response. */
export interface RecallResponse {
  entries: RecallEntry[];
  mode: string;
  requested: number;
  as_of: string;
  valid_at: string;
  scopes_considered: number;
  scopes_decided: number;
  truncated: boolean;
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

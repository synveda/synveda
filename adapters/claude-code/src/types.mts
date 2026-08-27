/**
 * Wire and hook types (ADPT-1, ADR-0027; re-cut onto the session API by
 * CPR-12, ADR-0078).
 *
 * Everything the adapter reads from the harness is optional and checked at the
 * edge: a hook payload and a session transcript are internal formats of
 * another program (ADR-0027 decision 9), so the adapter declares only the
 * fields it uses and treats the rest as opaque.
 *
 * Every runtime request names the server-owned run it belongs to; the adapter
 * carries no parallel application model.
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
  /** `SessionStart` only, which is why the spool carries it forward. */
  model?: string;
  /** `SessionEnd` only: why the harness stopped, when it says. */
  reason?: string;
}

/**
 * What a hook prints on stdout. Of the four events this adapter registers
 * for, only `SessionStart` can contribute context; the delivery hooks print
 * nothing at all (ADR-0027 decision 2).
 */
export interface HookOutput {
  systemMessage?: string;
  hookSpecificOutput?: {
    hookEventName: string;
    additionalContext?: string;
  };
}

/** The session event vocabulary (`SessionEventType` in synveda-types). */
export type SessionEventType =
  | "session.started"
  | "session.ended"
  | "message.user"
  | "message.assistant"
  | "tool.invoked"
  | "tool.result"
  | "file.read"
  | "file.changed"
  | "command.executed"
  | "skill.loaded"
  | "context.requested"
  | "adapter.warning"
  | "memory.asserted";

/** `POST /v1/sessions` — open a run. */
export interface OpenSessionRequest {
  workspace_id: string;
  project_id?: string;
  client_name: string;
  client_version?: string;
  client_installation_id?: string;
  /** The harness's own id: what makes opening idempotent across hooks. */
  external_session_id?: string;
  agent_name?: string;
  model_name?: string;
  branch?: string;
  task_summary?: string;
}

/** `POST /v1/sessions` — the run, as much of it as the adapter reads. */
export interface SessionResponse {
  id: string;
  workspace_id: string;
  project_id?: string;
  status: string;
}

/** One event of a `POST /v1/sessions/{id}/events` batch. */
export interface NewEvent {
  event_type: SessionEventType;
  /** The client's own id — the idempotency unit. */
  client_event_id: string;
  occurred_at: string;
  payload: unknown;
}

/** `POST /v1/sessions/{id}/events`. */
export interface AppendEventsRequest {
  events: NewEvent[];
}

/**
 * One event's outcome. Four values, and all four are terminal: `appended` and
 * `duplicate` mean it is held, `quarantined` means it is held and withheld
 * from the pipeline pending review, and `denied` means the redaction policy
 * refused it and nothing persisted. Re-sending any of them produces the same
 * answer forever, so the spool acknowledges all four.
 */
export interface AppendedEvent {
  outcome: "appended" | "duplicate" | "quarantined" | "denied";
  client_event_id: string;
  redactions?: unknown;
}

/** `POST /v1/sessions/{id}/events` — the response. */
export interface AppendEventsResponse {
  events: AppendedEvent[];
  appended: number;
  duplicates: number;
  quarantined: number;
  denied: number;
}

/** `POST /v1/sessions/{id}/context-runs`. */
export interface ContextRunRequest {
  query?: string;
  budget_tokens?: number;
}

/** `POST /v1/sessions/{id}/context-runs` — the composed block. */
export interface ContextRunResponse {
  id: string;
  rendered: string;
  block_hash: string;
  tokens: number;
  budget_tokens: number;
  entry_count: number;
  degraded: string[];
  created_at: string;
}

/** `POST /v1/sessions/{id}/end`. */
export interface EndSessionRequest {
  /** `ending` to announce the close while events are still arriving, or one
   *  of `ended`, `abandoned`, `failed` to close it. */
  status: "ending" | "ended" | "abandoned" | "failed";
  end_reason?: string;
}

/** As much of `GET /v1/me` as choosing a workspace needs. */
export interface MeResponse {
  workspaces?: { id?: unknown; name?: unknown }[];
}

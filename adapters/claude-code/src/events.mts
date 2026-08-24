/**
 * Mapping transcript entries to session events (CPR-12, ADR-0078), inside the
 * contract the append route publishes: 200 events per batch, 64 KiB per
 * payload, one client-minted event id per event.
 *
 * The entry's `uuid` is that id. It is per-session unique and stable across
 * retries, which is exactly what the append's idempotency gate asks for.
 *
 * # Four names where there used to be two
 *
 * `ObserveKind` had `transcript_delta` and `tool_result` to describe a whole
 * turn, so a turn that called three tools and said something arrived as one
 * `tool_result` with the text attached. The session vocabulary has
 * `message.user`, `message.assistant`, `tool.invoked` and `tool.result`, and
 * they are separate events — which is what makes a timeline read as a
 * transcript rather than as a list of turns, and what lets
 * `SessionEventType::capture_eligible` keep bookkeeping out of extraction.
 *
 * **`tool.invoked` was named here and never emitted** until CPR-14 replayed a
 * real tool-using transcript: an assistant entry whose content is a single
 * `tool_use` block has no text and no tool *result*, so it fell through both
 * branches and the entry was skipped whole. A session that read six files and
 * ran four commands therefore reached the gateway as the sentence it wrote at
 * the end. That is now three branches, and the acceptance harness asserts the
 * call and its result arrive as separate ordered events.
 */

import { basename } from "node:path";

import {
  messageText,
  toolInvocations,
  toolResults,
  truncateChars,
  type ToolInvocation,
  type ToolResult,
  type TranscriptEntry,
} from "./transcript.mjs";
import type { SessionEventType } from "./types.mjs";

/** `MAX_EVENT_BATCH` in the gateway. */
export const MAX_EVENTS_PER_BATCH = 200;

/** `MAX_EVENT_PAYLOAD_BYTES` in the gateway, over the serialised JSON. */
export const MAX_EVENT_PAYLOAD_BYTES = 64 * 1024;

/**
 * What the adapter fits a payload into. The gateway re-serialises the parsed
 * payload before measuring it, and two JSON encoders need not agree to the
 * byte; a kilobyte of headroom costs nothing and keeps a borderline event from
 * being rejected.
 */
const PAYLOAD_BUDGET_BYTES = MAX_EVENT_PAYLOAD_BYTES - 1024;

/** `MAX_CLIENT_EVENT_ID_CHARS` in the gateway. */
const MAX_ID_CHARS = 200;

const TRUNCATION_MARKER = "\n…[truncated by the Synveda adapter]";

/** One event ready for the spool. */
export interface RecordedEvent {
  event_type: SessionEventType;
  client_event_id: string;
  occurred_at: string;
  payload: unknown;
}

interface Payload {
  text?: string;
  tools?: ToolResult[];
  calls?: ToolInvocation[];
  context?: PayloadContext;
  truncated?: true;
}

interface PayloadContext {
  project?: string;
  git_branch?: string;
  model?: string;
  /** The harness version, as the transcript entry itself records it. */
  harness_version?: string;
}

/**
 * Turns transcript entries into session events.
 *
 * A turn can yield three events — what was said, what it called, and what its
 * tools returned — so the ids are suffixed to stay unique. The suffix is
 * stable across retries because it is derived from the entry, never from a
 * counter or a clock, which is what makes a redelivery answer `duplicate`
 * rather than append a second copy.
 *
 * Existing message/result ids keep CPR-12's spelling. That matters across an
 * adapter upgrade: learning to see a tool call must not rename a message the
 * previous build may already have appended. A new call takes `:call` whenever
 * it shares its entry with anything else.
 */
export function toSessionEvents(
  entries: TranscriptEntry[],
  model: string | undefined,
): RecordedEvent[] {
  const events: RecordedEvent[] = [];
  for (const entry of entries) {
    const tools = toolResults(entry.message);
    const calls = toolInvocations(entry.message);
    const text = messageText(entry.message).trim();
    // Nothing to say: an entry with no text, no tool call and no tool output
    // is structure, and structure is not memory.
    if (text.length === 0 && tools.length === 0 && calls.length === 0) continue;

    const context = contextOf(entry, model);
    const occurred_at = occurredAt(entry.timestamp);
    if (text.length > 0) {
      const payload: Payload = { text };
      if (context !== undefined) payload.context = context;
      events.push({
        event_type: entry.type === "user" ? "message.user" : "message.assistant",
        client_event_id: eventId(entry.uuid, tools.length > 0 ? "msg" : undefined),
        occurred_at,
        payload: fit(payload),
      });
    }
    if (calls.length > 0) {
      const payload: Payload = { calls };
      if (context !== undefined) payload.context = context;
      events.push({
        event_type: "tool.invoked",
        client_event_id: eventId(
          entry.uuid,
          text.length > 0 || tools.length > 0 ? "call" : undefined,
        ),
        occurred_at,
        payload: fit(payload),
      });
    }
    if (tools.length > 0) {
      const payload: Payload = { tools };
      if (context !== undefined) payload.context = context;
      events.push({
        event_type: "tool.result",
        client_event_id: eventId(entry.uuid, text.length > 0 ? "tool" : undefined),
        occurred_at,
        payload: fit(payload),
      });
    }
  }
  return events;
}

/**
 * The event id for one entry, with an optional discriminator.
 *
 * Bare when the entry produced one event, suffixed when it produced two. The
 * bare form is the common case and keeps an id readable in a timeline; the
 * suffix is deterministic, so a re-read of the same transcript produces the
 * same two ids and the append answers `duplicate` for both.
 */
function eventId(uuid: string, suffix?: string): string {
  const id = suffix === undefined ? uuid : `${uuid}:${suffix}`;
  return truncateChars(id, MAX_ID_CHARS);
}

export function chunk<T>(items: T[], size: number): T[][] {
  const batches: T[][] = [];
  for (let index = 0; index < items.length; index += size) {
    batches.push(items.slice(index, index + size));
  }
  return batches;
}

/**
 * Only what earns its place. The project is the basename of the working
 * directory, not the path: extraction benefits from knowing which project a
 * memory came from, while a full home-directory path is user data that would
 * otherwise ride into every record. The harness is not named here — the run
 * already says which client opened it — but its version is, because nothing
 * else does.
 */
function contextOf(
  entry: TranscriptEntry,
  model: string | undefined,
): PayloadContext | undefined {
  const project = entry.cwd !== undefined ? basename(entry.cwd) : undefined;
  const context: PayloadContext = {};
  if (project !== undefined) context.project = project;
  if (entry.gitBranch !== undefined) context.git_branch = entry.gitBranch;
  if (model !== undefined) context.model = model;
  if (entry.version !== undefined) context.harness_version = entry.version;
  return Object.keys(context).length > 0 ? context : undefined;
}

/**
 * A timestamp the gateway will parse. One malformed value in a transcript must
 * not reject a batch of two hundred, so an unparseable stamp falls back to now
 * and the event still lands.
 */
function occurredAt(timestamp: string): string {
  const parsed = Date.parse(timestamp);
  return Number.isNaN(parsed) ? new Date().toISOString() : new Date(parsed).toISOString();
}

/**
 * Bring a payload inside the per-event cap. Tool output is the usual cause of
 * an oversized payload, so the longest text is halved until the whole thing
 * fits and the payload says `truncated` — extraction wants the gist, and
 * dropping the event silently would be a lie (ADR-0027 decision 8).
 */
function fit(payload: Payload): Payload {
  if (byteLength(payload) <= PAYLOAD_BUDGET_BYTES) return payload;
  const candidate: Payload = { ...payload, truncated: true };
  if (candidate.tools !== undefined) {
    candidate.tools = candidate.tools.map((tool) => ({ ...tool }));
  }
  if (candidate.calls !== undefined) {
    candidate.calls = candidate.calls.map((call) => ({ ...call }));
  }
  // Each pass removes half of the largest contributor, so even a pathological
  // payload converges in a few dozen passes.
  for (let pass = 0; pass < 64; pass += 1) {
    if (byteLength(candidate) <= PAYLOAD_BUDGET_BYTES) return candidate;
    if (!shorten(candidate)) break;
  }
  // Nothing left to shorten and still too large: keep the envelope and drop
  // the content rather than send a batch the gateway will reject.
  const stripped: Payload = { ...candidate, text: TRUNCATION_MARKER.trim() };
  delete stripped.tools;
  delete stripped.calls;
  return stripped;
}

function shorten(payload: Payload): boolean {
  let longest = payload.text?.length ?? 0;
  let target: { list: "tools" | "calls"; index: number } | undefined;
  payload.tools?.forEach((tool, index) => {
    if (tool.text.length > longest) {
      longest = tool.text.length;
      target = { list: "tools", index };
    }
  });
  payload.calls?.forEach((call, index) => {
    if (call.input.length > longest) {
      longest = call.input.length;
      target = { list: "calls", index };
    }
  });
  if (longest === 0) return false;
  if (target === undefined) {
    if (payload.text === undefined) return false;
    payload.text = half(payload.text);
    return true;
  }
  if (target.list === "tools") {
    const tool = payload.tools?.[target.index];
    if (tool === undefined) return false;
    tool.text = half(tool.text);
    return true;
  }
  const call = payload.calls?.[target.index];
  if (call === undefined) return false;
  call.input = half(call.input);
  return true;
}

function half(text: string): string {
  const body = text.endsWith(TRUNCATION_MARKER)
    ? text.slice(0, text.length - TRUNCATION_MARKER.length)
    : text;
  const points = Array.from(body);
  if (points.length === 0) return "";
  return points.slice(0, Math.floor(points.length / 2)).join("") + TRUNCATION_MARKER;
}

function byteLength(payload: Payload): number {
  return new TextEncoder().encode(JSON.stringify(payload)).length;
}

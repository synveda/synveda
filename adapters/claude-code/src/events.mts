/**
 * Mapping transcript entries to observe events (ADR-0027 decision 8),
 * inside the contract MEM-1 publishes: 256 events per batch, 64 KiB per
 * payload, one client-minted idempotency key per event.
 *
 * The entry's `uuid` is that key. It is per-session unique and stable
 * across retries, which is precisely what ADR-0020 decision 2 asks for
 * and what makes the cursor design of decision 7 safe.
 */

import { basename } from "node:path";

import {
  messageText,
  toolResults,
  truncateChars,
  type ToolResult,
  type TranscriptEntry,
} from "./transcript.mjs";
import type { ObserveEvent, ObserveKind } from "./types.mjs";

/** `MAX_EVENTS_PER_BATCH` in the gateway. */
export const MAX_EVENTS_PER_BATCH = 256;

/** `MAX_EVENT_PAYLOAD_BYTES` in the gateway, over the serialised JSON. */
export const MAX_EVENT_PAYLOAD_BYTES = 64 * 1024;

/**
 * What the adapter fits a payload into. The gateway re-serialises the
 * parsed payload before measuring it, and two JSON encoders need not
 * agree to the byte; a kilobyte of headroom costs nothing and keeps a
 * borderline event from being rejected.
 */
const PAYLOAD_BUDGET_BYTES = MAX_EVENT_PAYLOAD_BYTES - 1024;

/** `MAX_TEXT_FIELD_CHARS` in the gateway's staging table. */
const MAX_KEY_CHARS = 200;

const TRUNCATION_MARKER = "\n…[truncated by the Synveda adapter]";

interface Payload {
  role: "user" | "assistant";
  text: string;
  tools?: ToolResult[];
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

export function toObserveEvents(
  entries: TranscriptEntry[],
  model: string | undefined,
): ObserveEvent[] {
  const events: ObserveEvent[] = [];
  for (const entry of entries) {
    const tools = toolResults(entry.message);
    const text = messageText(entry.message).trim();
    // Nothing to say: an entry with neither text nor tool output is
    // structure, and structure is not memory.
    if (text.length === 0 && tools.length === 0) continue;

    const kind: ObserveKind = tools.length > 0 ? "tool_result" : "transcript_delta";
    const payload: Payload = { role: entry.type, text };
    if (tools.length > 0) payload.tools = tools;
    const context = contextOf(entry, model);
    if (context !== undefined) payload.context = context;

    events.push({
      idempotency_key: truncateChars(entry.uuid, MAX_KEY_CHARS),
      kind,
      payload: fit(payload),
      occurred_at: occurredAt(entry.timestamp),
    });
  }
  return events;
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
 * directory, not the path: extraction benefits from knowing which
 * project a memory came from, while a full home-directory path is user
 * data that would otherwise ride into every record. The harness is not
 * named here — the session id already says it (decision 10) — but its
 * version is, because nothing else does.
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
 * A timestamp the gateway will parse. One malformed value in a
 * transcript must not reject a batch of 256, so an unparseable stamp
 * falls back to now and the event still lands.
 */
function occurredAt(timestamp: string): string {
  const parsed = Date.parse(timestamp);
  return Number.isNaN(parsed) ? new Date().toISOString() : new Date(parsed).toISOString();
}

/**
 * Bring a payload inside the per-event cap. Tool output is the usual
 * cause of an oversized payload, so the longest text is halved until the
 * whole thing fits and the payload says `truncated` — MEM-3 wants the
 * gist, and dropping the event silently would be a lie (decision 8).
 */
function fit(payload: Payload): Payload {
  if (byteLength(payload) <= PAYLOAD_BUDGET_BYTES) return payload;
  const candidate: Payload = { ...payload, truncated: true };
  if (candidate.tools !== undefined) {
    candidate.tools = candidate.tools.map((tool) => ({ ...tool }));
  }
  // Each pass removes half of the largest contributor, so even a
  // pathological payload converges in a few dozen passes.
  for (let pass = 0; pass < 64; pass += 1) {
    if (byteLength(candidate) <= PAYLOAD_BUDGET_BYTES) return candidate;
    if (!shorten(candidate)) break;
  }
  // Nothing left to shorten and still too large: keep the envelope and
  // drop the content rather than send a batch the gateway will reject.
  const stripped: Payload = { ...candidate, text: TRUNCATION_MARKER.trim() };
  delete stripped.tools;
  return stripped;
}

function shorten(payload: Payload): boolean {
  let longest = payload.text.length;
  let target = -1;
  payload.tools?.forEach((tool, index) => {
    if (tool.text.length > longest) {
      longest = tool.text.length;
      target = index;
    }
  });
  if (longest === 0) return false;
  if (target < 0) {
    payload.text = half(payload.text);
    return true;
  }
  const tools = payload.tools;
  if (tools === undefined) return false;
  const tool = tools[target];
  if (tool === undefined) return false;
  tool.text = half(tool.text);
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

/**
 * Reading the session transcript (ADR-0027 decision 9).
 *
 * The transcript is an internal JSONL format of another program. The
 * parser therefore reads exactly the fields it needs, treats every other
 * field as opaque, and skips any line it cannot parse rather than
 * failing the flush: the adapter's job is to keep working across harness
 * releases, not to validate them.
 */

import { readFileSync } from "node:fs";

/** Inject's own cap on a task (`MAX_TASK_CHARS` in the gateway). */
export const MAX_TASK_CHARS = 4096;

export interface TranscriptEntry {
  uuid: string;
  type: "user" | "assistant";
  timestamp: string;
  message: unknown;
  cwd?: string;
  gitBranch?: string;
  /** The harness version that wrote the entry (ADR-0027 decision 8). */
  version?: string;
}

/** One tool result carried by a transcript entry. */
export interface ToolResult {
  tool_use_id?: string;
  is_error: boolean;
  text: string;
}

/** Every session-content entry in the transcript, in document order. */
export function readTranscript(path: string): TranscriptEntry[] {
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    return [];
  }
  const entries: TranscriptEntry[] = [];
  for (const line of raw.split("\n")) {
    const entry = parseEntry(line);
    if (entry !== undefined) entries.push(entry);
  }
  return entries;
}

function parseEntry(line: string): TranscriptEntry | undefined {
  if (line.trim().length === 0) return undefined;
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (value === null || typeof value !== "object" || Array.isArray(value)) return undefined;
  const record = value as Record<string, unknown>;

  const type = record.type;
  if (type !== "user" && type !== "assistant") return undefined;
  // Meta entries are harness bookkeeping, not session content. Sidechain
  // entries are subagent transcripts, and which scope a subagent's work
  // belongs to is its own question (ADR-0027 decision 8).
  if (record.isMeta === true || record.isSidechain === true) return undefined;

  const uuid = record.uuid;
  const timestamp = record.timestamp;
  if (typeof uuid !== "string" || typeof timestamp !== "string") return undefined;

  return {
    uuid,
    type,
    timestamp,
    message: record.message,
    cwd: typeof record.cwd === "string" ? record.cwd : undefined,
    gitBranch: typeof record.gitBranch === "string" ? record.gitBranch : undefined,
    version: typeof record.version === "string" ? record.version : undefined,
  };
}

export interface Delta {
  entries: TranscriptEntry[];
  /** True when a cursor was set but no longer appears in the transcript. */
  resynced: boolean;
}

/**
 * Everything after the cursor. A cursor that no longer appears — the
 * transcript was rewritten, most often by compaction — resynchronises
 * from the beginning: redelivery is free, because the buffer reports
 * duplicates and re-enqueues nothing (ADR-0020 decision 2), whereas
 * silence loses the session.
 */
export function entriesAfter(
  entries: TranscriptEntry[],
  cursor: string | undefined,
): Delta {
  if (cursor === undefined) return { entries, resynced: false };
  const index = entries.findIndex((entry) => entry.uuid === cursor);
  if (index < 0) return { entries, resynced: true };
  return { entries: entries.slice(index + 1), resynced: false };
}

/** The text blocks of a message, concatenated in document order. */
export function messageText(message: unknown): string {
  const content = contentOf(message);
  if (typeof content === "string") return content;
  return textOfBlocks(content);
}

/** The `tool_result` blocks of a message, if any. */
export function toolResults(message: unknown): ToolResult[] {
  const content = contentOf(message);
  if (!Array.isArray(content)) return [];
  const results: ToolResult[] = [];
  for (const block of content) {
    if (block === null || typeof block !== "object") continue;
    const record = block as Record<string, unknown>;
    if (record.type !== "tool_result") continue;
    results.push({
      tool_use_id: typeof record.tool_use_id === "string" ? record.tool_use_id : undefined,
      is_error: record.is_error === true,
      text:
        typeof record.content === "string" ? record.content : textOfBlocks(record.content),
    });
  }
  return results;
}

/**
 * The task for a resumed, forked, or compacted session (ADR-0027
 * decision 11): the most recent thing the user actually asked for. An
 * entry carrying tool results is the harness replying to itself, not a
 * prompt.
 */
export function lastUserPrompt(entries: TranscriptEntry[]): string | undefined {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (entry === undefined || entry.type !== "user") continue;
    if (toolResults(entry.message).length > 0) continue;
    const text = messageText(entry.message).trim();
    if (text.length > 0) return truncateChars(text, MAX_TASK_CHARS);
  }
  return undefined;
}

/**
 * Truncate by Unicode code point, never by UTF-16 unit: half a surrogate
 * pair is not a character the gateway can parse, and the gateway counts
 * `chars()` anyway.
 */
export function truncateChars(text: string, limit: number): string {
  const points = Array.from(text);
  return points.length <= limit ? text : points.slice(0, limit).join("");
}

function contentOf(message: unknown): unknown {
  if (message === null || typeof message !== "object") return undefined;
  return (message as Record<string, unknown>).content;
}

function textOfBlocks(content: unknown): string {
  if (!Array.isArray(content)) return "";
  const parts: string[] = [];
  for (const block of content) {
    if (block === null || typeof block !== "object") continue;
    const record = block as Record<string, unknown>;
    if (record.type === "text" && typeof record.text === "string") parts.push(record.text);
  }
  return parts.join("\n");
}

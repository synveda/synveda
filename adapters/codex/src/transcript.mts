/** Codex 0.152.0's captured JSONL → the existing Session event mapper.
 * This is an internal vendor format; unsupported shapes grant no capabilities.
 */
import { closeSync, constants, fstatSync, openSync, readSync } from "node:fs";
import type { TranscriptEntry } from "@synveda/claude-code-adapter/session-runtime";

export const MAX_TRANSCRIPT_BYTES = 8 * 1024 * 1024;
const MAX_RECORDS = 20_000;
type ObjectValue = Record<string, unknown>;
type ToolResult = { kind: "command" | "mcp"; failed: boolean };

type InputReason = "input_limit" | "record_limit" | "invalid_json" | "session_mismatch" |
  "missing_identity" | "unreadable" | "size_or_type" | "changed_during_read" |
  "invalid_tool_arguments" | "tool_result_status_unknown" | "tool_result_shape_unknown";

/** A closed diagnostic vocabulary; never retain rejected content in logs. */
export class CodexInputError extends Error {
  constructor(readonly reason: InputReason) { super(reason); }
}

export function readCodexTranscript(path: string, sessionId: string): TranscriptEntry[] {
  const raw = boundedRead(path);
  if (raw === undefined || raw.length === 0) return [];
  const lines = raw.split("\n");
  if (lines.length > MAX_RECORDS) throw new CodexInputError("record_limit");
  const entries: TranscriptEntry[] = [];
  const toolResults = new Map<string, ToolResult>();
  let identified = false;
  for (const line of lines) {
    if (line.trim().length === 0) continue;
    // A partially written tail is retried next hook, without advancing past it.
    let record: ObjectValue;
    try { record = object(JSON.parse(line)); }
    catch { throw new CodexInputError("invalid_json"); }
    const payload = object(record.payload);
    if (record.type === "session_meta") {
      if (identified || payload.id !== sessionId) throw new CodexInputError("session_mismatch");
      identified = true;
    } else if (!identified) {
      throw new CodexInputError("missing_identity");
    } else if (record.type === "event_msg") {
      const item = object(payload.item);
      if (item.type === "CommandExecution" && typeof item.id === "string" && Number.isSafeInteger(item.exit_code)) {
        toolResults.set(item.id, { kind: "command", failed: item.exit_code !== 0 });
      } else if (item.type === "McpToolCall" && typeof item.id === "string") {
        const result = object(item.result);
        if (typeof result.isError === "boolean" && item.status === (result.isError ? "failed" : "completed")) {
          toolResults.set(item.id, { kind: "mcp", failed: result.isError });
        }
      }
    } else if (record.type === "response_item") {
      const entry = translate(record, payload, toolResults);
      if (entry !== undefined) entries.push(entry);
    }
  }
  return entries;
}

function boundedRead(path: string): string | undefined {
  let fd: number;
  try { fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK); }
  catch (error) {
    if (object(error).code === "ENOENT") return undefined;
    throw new CodexInputError("unreadable");
  }
  try {
    const stat = fstatSync(fd);
    if (!stat.isFile() || stat.size > MAX_TRANSCRIPT_BYTES) {
      throw new CodexInputError("size_or_type");
    }
    // Read at most the validated size plus one byte to detect concurrent growth.
    const bytes = Buffer.alloc(stat.size + 1);
    let size = 0;
    while (size < bytes.length) {
      const count = readSync(fd, bytes, size, bytes.length - size, null);
      if (count === 0) break;
      size += count;
    }
    if (size > stat.size) throw new CodexInputError("changed_during_read");
    return bytes.subarray(0, size).toString("utf8");
  } finally { closeSync(fd); }
}

function translate(
  record: ObjectValue,
  payload: ObjectValue,
  toolResults: ReadonlyMap<string, ToolResult>,
): TranscriptEntry | undefined {
  const id = payload.id;
  const timestamp = record.timestamp;
  if (
    typeof id !== "string" || id.length === 0 || id.length > 180 ||
    typeof timestamp !== "string" || !Number.isFinite(Date.parse(timestamp))
  ) return undefined;
  const base = { uuid: id, timestamp };
  if (payload.type === "message" && (payload.role === "user" || payload.role === "assistant")) {
    if (!Array.isArray(payload.content)) return undefined;
    const kinds = object(payload.internal_chat_message_metadata_passthrough).content_item_kinds;
    const content = payload.content.flatMap((value: unknown, index: number) => {
      const block = object(value);
      // Codex also puts environment/AGENTS instructions in user-role records.
      // Only the captured user.text tag establishes a user-authored text block.
      if (payload.role === "user" && (!Array.isArray(kinds) || kinds[index] !== "user.text")) return [];
      const type = payload.role === "user" ? "input_text" : "output_text";
      return block.type === type && typeof block.text === "string"
        ? [{ type: "text", text: block.text }] : [];
    });
    return content.length === 0 ? undefined : { ...base, type: payload.role, message: { content } };
  }
  if (payload.type === "function_call" && typeof payload.name === "string" && typeof payload.call_id === "string") {
    if (typeof payload.arguments !== "string") return undefined;
    let input: unknown;
    try { input = JSON.parse(payload.arguments); }
    catch { throw new CodexInputError("invalid_tool_arguments"); }
    const name = typeof payload.namespace === "string" ? `${payload.namespace}__${payload.name}` : payload.name;
    return { ...base, type: "assistant", message: { content: [
      { type: "tool_use", id: payload.call_id, name, input },
    ] } };
  }
  if (payload.type === "function_call_output" && typeof payload.call_id === "string") {
    // Only native completion metadata establishes success. Unknown output
    // shapes are held, never silently skipped while the cursor advances.
    const result = toolResults.get(payload.call_id);
    if (result === undefined) throw new CodexInputError("tool_result_status_unknown");
    const content = toolOutput(payload.output, result.kind);
    return { ...base, type: "user", message: { content: [
      { type: "tool_result", tool_use_id: payload.call_id, content, is_error: result.failed },
    ] } };
  }
  return undefined;
}

function toolOutput(output: unknown, kind: ToolResult["kind"]): string {
  if (kind === "command" && typeof output === "string") return output;
  if (kind === "mcp" && Array.isArray(output) && output.length > 0) {
    const text: string[] = [];
    for (const value of output) {
      const block = object(value);
      if (block.type !== "input_text" || typeof block.text !== "string") {
        throw new CodexInputError("tool_result_shape_unknown");
      }
      text.push(block.text);
    }
    return text.join("\n");
  }
  throw new CodexInputError("tool_result_shape_unknown");
}

function object(value: unknown): ObjectValue {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as ObjectValue : {};
}

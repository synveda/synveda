/** ADPT-9: captured Copilot CLI 1.0.83 JSONL into the existing Session mapper.
 * Native execution records establish tool evidence; model intentions do not.
 */
import { readBoundedTranscript, type TranscriptEntry } from "@synveda/claude-code-adapter/session-runtime";

const MAX_RECORDS = 20_000;
type ObjectValue = Record<string, unknown>;
type InputReason = "record_limit" | "invalid_json" | "missing_identity" | "session_mismatch" |
  "record_shape_unknown" | "session_schema_unknown" | "event_identity" | "duplicate_event" | "message_shape_unknown" |
  "tool_call_shape_unknown" | "tool_result_shape_unknown" | "tool_pair_unknown";

/** Closed reasons keep rejected transcript bytes out of diagnostics. */
export class CopilotInputError extends Error {
  constructor(readonly reason: InputReason) { super(reason); }
}

export function readCopilotTranscript(path: string, sessionId: string): TranscriptEntry[] {
  const raw = readBoundedTranscript(path);
  if (raw === undefined || raw.length === 0) return [];
  const lines = raw.split("\n");
  if (lines.length > MAX_RECORDS) throw new CopilotInputError("record_limit");
  const entries: TranscriptEntry[] = [];
  const ids = new Set<string>();
  const calls = new Set<string>();
  let version: string | undefined;
  for (const line of lines) {
    if (line.trim().length === 0) continue;
    let record: ObjectValue;
    try { record = object(JSON.parse(line)); }
    catch { throw new CopilotInputError("invalid_json"); }
    if (typeof record.type !== "string") throw new CopilotInputError("record_shape_unknown");
    const data = object(record.data);
    if (record.type === "session.start") {
      if (version !== undefined || data.sessionId !== sessionId) throw new CopilotInputError("session_mismatch");
      if (data.version !== 1 || !text(data.copilotVersion, 100)) throw new CopilotInputError("session_schema_unknown");
      version = data.copilotVersion;
      continue;
    }
    if (version === undefined) throw new CopilotInputError("missing_identity");
    if (record.type !== "user.message" && record.type !== "assistant.message" &&
        record.type !== "tool.execution_start" && record.type !== "tool.execution_complete") continue;
    if (!text(record.id, 180) || !text(record.timestamp, 64) || !Number.isFinite(Date.parse(record.timestamp))) {
      throw new CopilotInputError("event_identity");
    }
    if (ids.has(record.id)) throw new CopilotInputError("duplicate_event");
    ids.add(record.id);
    const base = { uuid: record.id, timestamp: record.timestamp, version };
    const entry = translate(record.type, data, base, calls);
    if (entry !== undefined) entries.push(entry);
  }
  return entries;
}

function translate(
  type: string,
  data: ObjectValue,
  base: Pick<TranscriptEntry, "uuid" | "timestamp" | "version">,
  calls: Set<string>,
): TranscriptEntry | undefined {
  if (type === "user.message" || type === "assistant.message") {
    if (typeof data.content !== "string" || (data.phase !== undefined && data.phase !== "final_answer")) {
      throw new CopilotInputError("message_shape_unknown");
    }
    // Ignore assistant.toolRequests: the native execution_start is the one
    // observation source. Never copy reasoning, injected context or Skill bodies.
    if (data.content.trim().length === 0) return undefined;
    return { ...base, type: type === "user.message" ? "user" : "assistant",
      message: { content: data.content } };
  }
  if (!text(data.toolCallId, 180)) throw new CopilotInputError("tool_pair_unknown");
  if (type === "tool.execution_start") {
    if (!text(data.toolName, 200) || data.arguments === null || typeof data.arguments !== "object" || Array.isArray(data.arguments)) {
      throw new CopilotInputError("tool_call_shape_unknown");
    }
    if (calls.has(data.toolCallId)) throw new CopilotInputError("tool_pair_unknown");
    calls.add(data.toolCallId);
    return { ...base, type: "assistant", message: { content: [
      { type: "tool_use", id: data.toolCallId, name: data.toolName, input: data.arguments },
    ] } };
  }
  const result = object(data.result);
  if (typeof data.success !== "boolean" || typeof result.content !== "string") {
    throw new CopilotInputError("tool_result_shape_unknown");
  }
  if (!calls.delete(data.toolCallId)) throw new CopilotInputError("tool_pair_unknown");
  return { ...base, type: "user", message: { content: [
    { type: "tool_result", tool_use_id: data.toolCallId, content: result.content, is_error: !data.success },
  ] } };
}

function text(value: unknown, limit: number): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= limit;
}

function object(value: unknown): ObjectValue {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as ObjectValue : {};
}

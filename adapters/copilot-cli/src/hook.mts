#!/usr/bin/env node
/** ADPT-9: captured start/resume, local Stop and bounded runtime-exit seams.
 * Public-API context and durable observations share one application task.
 */
import { isAbsolute } from "node:path";
import {
  diagnostic, loadConfig, log, sessionStart, turn, TranscriptReadError, type HookInput,
} from "@synveda/claude-code-adapter/session-runtime";
import { CopilotInputError, readCopilotTranscript } from "./transcript.mjs";

const MAX_INPUT_BYTES = 64 * 1024;
const watchdog = setTimeout(() => {
  log("copilot.hook_failed", { reason: "deadline" });
  process.exit(0);
}, 10_000);
watchdog.unref();
try { await main(); }
catch (error) { log("copilot.hook_failed", {
  error: error instanceof CopilotInputError || error instanceof TranscriptReadError ? error.reason : diagnostic(error),
}); }
finally { clearTimeout(watchdog); }

async function main(): Promise<void> {
  // Native camelCase input has no event name. The registration supplies it;
  // Other hooks must never be mistaken for a captured lifecycle boundary.
  const event = process.argv[2];
  if (event !== "sessionStart" && event !== "agentStop" && event !== "sessionEnd") return;
  const input = await readInput(event);
  if (input === undefined) return;
  const configured = loadConfig(input.cwd);
  if (configured.disabled) return;
  const config = { ...configured, clientName: "copilot-cli" as const };
  const nativeId = input.session_id as string;
  const bound = { ...input, session_id: `copilot-cli:${nativeId}` };
  const reader = (path: string) => config.observe ? readCopilotTranscript(path, nativeId) : [];
  const output = event === "sessionStart"
    ? await sessionStart(bound, config, reader) : await turn(bound, config, reader);
  if (event !== "sessionStart") return;
  const context = [output.hookSpecificOutput?.additionalContext, output.systemMessage]
    .filter((value): value is string => typeof value === "string" && value.length > 0);
  if (context.length > 0) process.stdout.write(JSON.stringify({ additionalContext: context.join("\n\n") }));
}

async function readInput(event: "sessionStart" | "agentStop" | "sessionEnd"): Promise<HookInput | undefined> {
  const chunks: Buffer[] = [];
  let bytes = 0;
  for await (const piece of process.stdin) {
    const chunk = Buffer.isBuffer(piece) ? piece : Buffer.from(piece);
    bytes += chunk.length;
    if (bytes > MAX_INPUT_BYTES) return invalid("payload_size");
    chunks.push(chunk);
  }
  let value: unknown;
  try { value = JSON.parse(Buffer.concat(chunks).toString("utf8")); }
  catch { return invalid("json"); }
  if (value === null || typeof value !== "object" || Array.isArray(value)) return invalid("shape");
  const input = value as Record<string, unknown>;
  if (typeof input.sessionId !== "string" ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(input.sessionId)) return invalid("identity");
  if (!absolutePath(input.cwd)) return invalid("cwd");
  if (typeof input.timestamp !== "number" || !Number.isSafeInteger(input.timestamp) ||
      input.timestamp < 0) return invalid("timestamp");
  // Construct, do not spread. Only agentStop supplies the captured transcript
  // path; start/end reuse its saved path, always checked against native identity.
  const base = { session_id: input.sessionId, cwd: input.cwd };
  if (event === "sessionStart") {
    if (input.source !== "startup" && input.source !== "resume" && input.source !== "new") return invalid("source");
    return { ...base, hook_event_name: "SessionStart", source: input.source };
  }
  if (event === "agentStop") {
    if (!absolutePath(input.transcriptPath)) return invalid("transcript_path");
    return { ...base, hook_event_name: "Stop", transcript_path: input.transcriptPath };
  }
  return { ...base, hook_event_name: "SessionEnd" };
}

function absolutePath(value: unknown): value is string {
  return typeof value === "string" && value.length <= 4096 && !value.includes("\0") && isAbsolute(value);
}

function invalid(reason: "payload_size" | "json" | "shape" | "identity" | "cwd" | "source" | "timestamp" | "transcript_path"): undefined {
  log("copilot.input_held", { reason });
  return undefined;
}

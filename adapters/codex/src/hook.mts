#!/usr/bin/env node
/** CPR-39: captured start/compaction/Stop/runtime-exit boundaries.
 * The watchdog never blocks the host. Saved observations survive API outages.
 */
import { isAbsolute } from "node:path";
import {
  diagnostic, loadConfig, log, sessionStart, turn, type HookInput,
} from "@synveda/claude-code-adapter/session-runtime";
import { CodexInputError, readCodexTranscript } from "./transcript.mjs";

const MAX_INPUT_BYTES = 64 * 1024;
const watchdog = setTimeout(() => process.exit(0), 10_000);
watchdog.unref();
try { await main(); }
catch (error) {
  log("codex.hook_failed", { error: error instanceof CodexInputError ? error.reason : diagnostic(error) });
}
finally { clearTimeout(watchdog); }

async function main(): Promise<void> {
  const input = await readInput();
  if (input === undefined) return;
  const configured = loadConfig(input.cwd);
  if (configured.disabled) return;
  const config = { ...configured, clientName: "codex" as const };
  const sessionId = input.session_id as string;
  // The spool is shared with the CLI; namespace native IDs before any lookup.
  const bound = { ...input, session_id: `codex:${sessionId}` };
  const reader = (path: string) => config.observe ? readCodexTranscript(path, sessionId) : [];
  const result = input.hook_event_name === "SessionStart"
    ? await sessionStart(bound, config, reader) : await turn(bound, config, reader);
  if (Object.keys(result).length > 0) process.stdout.write(JSON.stringify(result));
}

async function readInput(): Promise<HookInput | undefined> {
  if (process.stdin.isTTY === true) return undefined;
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const chunk of process.stdin) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk));
    size += bytes.length;
    if (size > MAX_INPUT_BYTES) throw new CodexInputError("input_limit");
    chunks.push(bytes);
  }
  const value: unknown = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  if (value === null || typeof value !== "object" || Array.isArray(value)) return undefined;
  const input = value as HookInput & { trigger?: unknown };
  if (
    typeof input.session_id !== "string" ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(input.session_id) ||
    typeof input.cwd !== "string" || !isAbsolute(input.cwd) ||
    typeof input.transcript_path !== "string" || !isAbsolute(input.transcript_path) ||
    (input.model !== undefined && (typeof input.model !== "string" || input.model.length > 200))
  ) return undefined;
  if (input.hook_event_name === "SessionStart") {
    return input.source === "startup" || input.source === "resume" || input.source === "compact" ? input : undefined;
  }
  if (input.hook_event_name === "PreCompact") {
    return input.trigger === "manual" || input.trigger === "auto" ? input : undefined;
  }
  return input.hook_event_name === "Stop" || input.hook_event_name === "SessionEnd" ? input : undefined;
}

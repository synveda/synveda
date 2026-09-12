#!/usr/bin/env node
/** ADPT-9: the documented camelCase start/resume seam (ADR-0107).
 * Only public-API context and task binding; transcript capture is unqualified.
 */
import { isAbsolute } from "node:path";
import {
  diagnostic, loadConfig, log, sessionStart, type HookInput,
} from "@synveda/claude-code-adapter/session-runtime";

const MAX_INPUT_BYTES = 64 * 1024;
const watchdog = setTimeout(() => {
  log("copilot.hook_failed", { reason: "deadline" });
  process.exit(0);
}, 10_000);
watchdog.unref();
try { await main(); }
catch (error) { log("copilot.hook_failed", { error: diagnostic(error) }); }
finally { clearTimeout(watchdog); }

async function main(): Promise<void> {
  // Native camelCase input has no event name. The registration supplies it;
  // other hooks must never be mistaken for an application start or end.
  if (process.argv[2] !== "sessionStart") return;
  const input = await readInput();
  if (input === undefined) return;
  const configured = loadConfig(input.cwd);
  if (configured.disabled) return;
  const config = { ...configured, clientName: "copilot-cli" as const, observe: false };
  const output = await sessionStart(input, config, () => []);
  const context = [output.hookSpecificOutput?.additionalContext, output.systemMessage]
    .filter((value): value is string => typeof value === "string" && value.length > 0);
  if (context.length > 0) process.stdout.write(JSON.stringify({ additionalContext: context.join("\n\n") }));
}

async function readInput(): Promise<HookInput | undefined> {
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
  if (typeof input.cwd !== "string" || input.cwd.length > 4096 ||
      input.cwd.includes("\0") || !isAbsolute(input.cwd)) return invalid("cwd");
  if (input.source !== "startup" && input.source !== "resume" && input.source !== "new") return invalid("source");
  if (typeof input.timestamp !== "number" || !Number.isSafeInteger(input.timestamp) ||
      input.timestamp < 0) return invalid("timestamp");
  // Construct, do not spread: unqualified transcript/model fields cannot enter
  // the shared reader, and the native ID is never a Synveda Session UUID.
  return { hook_event_name: "SessionStart", session_id: `copilot-cli:${input.sessionId}`,
    cwd: input.cwd, source: input.source };
}

function invalid(reason: "payload_size" | "json" | "shape" | "identity" | "cwd" | "source" | "timestamp"): undefined {
  log("copilot.input_held", { reason });
  return undefined;
}

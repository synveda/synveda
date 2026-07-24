/**
 * Per-session state (ADR-0027 decision 7): the cursor that makes observe
 * at-least-once, and the transcript path — which `PreCompact` needs and
 * does not carry in its payload.
 *
 * The cursor is the uuid of the last transcript entry a gateway 2xx has
 * accepted, and nothing else advances it. A failed flush therefore
 * resends on the next hook, where the buffer reports the overlap as
 * duplicates and re-enqueues nothing (ADR-0020 decision 2). That is the
 * whole delivery design: no daemon, no local queue.
 */

import { createHash } from "node:crypto";
import {
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";

import { log } from "./log.mjs";
import { sessionDir, stateDir } from "./paths.mjs";

/** Session state is worthless once the session is long gone. */
const PRUNE_AFTER_MS = 30 * 24 * 60 * 60 * 1000;

export interface SessionState {
  session_id: string;
  transcript_path?: string;
  cursor?: string;
  updated_at: string;
}

export function loadSession(sessionId: string): SessionState | undefined {
  try {
    const parsed: unknown = JSON.parse(readFileSync(sessionFile(sessionId), "utf8"));
    if (parsed !== null && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as SessionState;
    }
  } catch {
    // No state yet, or unreadable. A missing cursor is always safe:
    // redelivery is free, and a wrong cursor is not.
  }
  return undefined;
}

export function saveSession(
  sessionId: string,
  transcriptPath: string | undefined,
  cursor: string | undefined,
): void {
  const state: SessionState = { session_id: sessionId, updated_at: new Date().toISOString() };
  if (transcriptPath !== undefined) state.transcript_path = transcriptPath;
  if (cursor !== undefined) state.cursor = cursor;
  try {
    mkdirSync(sessionDir(), { recursive: true });
    const file = sessionFile(sessionId);
    // Write-then-rename: a hook killed mid-write must never leave a
    // half-written cursor behind.
    const temporary = `${file}.${process.pid}.tmp`;
    writeFileSync(temporary, JSON.stringify(state));
    renameSync(temporary, file);
  } catch (error) {
    log("spool.write_failed", { session: sessionId, error: String(error) });
  }
}

/** Drop state for sessions no one will resume. Best-effort by design. */
export function prune(): void {
  let names: string[];
  try {
    names = readdirSync(sessionDir());
  } catch {
    return;
  }
  const now = Date.now();
  for (const name of names) {
    const file = join(sessionDir(), name);
    try {
      if (now - statSync(file).mtimeMs > PRUNE_AFTER_MS) rmSync(file, { force: true });
    } catch {
      // Raced with another hook, or vanished. Either way: nothing to do.
    }
  }
}

/**
 * One disclosure per project (ADR-0027 decision 13). Login is consent,
 * but silent capture is not something this product gets to do. The `wx`
 * flag makes the claim atomic, so concurrent sessions disclose once.
 */
export function claimDisclosure(cwd: string | undefined): boolean {
  if (cwd === undefined || cwd.length === 0) return false;
  try {
    const dir = join(stateDir(), "disclosed");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, digest(cwd, 16)), cwd, { flag: "wx" });
    return true;
  } catch {
    return false;
  }
}

/**
 * A filename that is readable and collision-free: the sanitised id for
 * a human reading the spool, a digest so two ids that sanitise alike
 * cannot share a cursor.
 */
function sessionFile(sessionId: string): string {
  const readable = sessionId.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 96);
  return join(sessionDir(), `${readable}-${digest(sessionId, 8)}.json`);
}

function digest(value: string, length: number): string {
  return createHash("sha256").update(value).digest("hex").slice(0, length);
}

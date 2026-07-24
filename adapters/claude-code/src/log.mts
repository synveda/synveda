/**
 * The adapter's diagnostics go to a file and NEVER to stdout: for
 * `SessionStart`, stdout is context the model reads, so a stray debug
 * line would become context — and a stray token would become context
 * (ADR-0027 compliance notes).
 *
 * Logging is best-effort by construction. A failure to log must never
 * fail a hook, so every path here swallows its errors.
 */

import { appendFileSync, mkdirSync, renameSync, statSync } from "node:fs";
import { join } from "node:path";

import { stateDir } from "./paths.mjs";

/** Rotate at 4 MiB; one generation is kept. */
const MAX_LOG_BYTES = 4 * 1024 * 1024;

export function log(event: string, fields: Record<string, unknown> = {}): void {
  try {
    const dir = stateDir();
    mkdirSync(dir, { recursive: true });
    const file = join(dir, "adapter.log");
    rotate(file);
    // The fixed keys are written last on purpose: a caller's field must
    // never be able to rename the event it is logging under.
    appendFileSync(
      file,
      `${JSON.stringify({ ...fields, at: new Date().toISOString(), event })}\n`,
    );
  } catch {
    // Diagnostics are never worth a failed hook.
  }
}

function rotate(file: string): void {
  try {
    if (statSync(file).size > MAX_LOG_BYTES) {
      renameSync(file, `${file}.1`);
    }
  } catch {
    // No log yet, or an unwritable directory: nothing to rotate.
  }
}

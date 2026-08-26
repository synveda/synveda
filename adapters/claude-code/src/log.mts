/**
 * The adapter's diagnostics go to a file and NEVER to stdout: for
 * `SessionStart`, stdout is context the model reads, so a stray debug
 * line would become context — and a stray token would become context
 * (ADR-0027 compliance notes).
 *
 * Logging is best-effort by construction. A failure to log must never
 * fail a hook, so every path here swallows its errors.
 */

import { appendFileSync, renameSync, statSync } from "node:fs";
import { join } from "node:path";

import { ensureDir, stateDir } from "./paths.mjs";

/** Rotate at 4 MiB; one generation is kept. */
const MAX_LOG_BYTES = 4 * 1024 * 1024;

export function log(event: string, fields: Record<string, unknown> = {}): void {
  try {
    const dir = stateDir();
    ensureDir(dir);
    const file = join(dir, "adapter.log");
    rotate(file);
    // The fixed keys are written last on purpose: a caller's field must
    // never be able to rename the event it is logging under.
    appendFileSync(
      file,
      `${JSON.stringify({ ...safeFields(fields), at: new Date().toISOString(), event })}\n`,
    );
  } catch {
    // Diagnostics are never worth a failed hook.
  }
}

const SECRET_FIELDS = new Set([
  "access_token",
  "authorization",
  "bearer",
  "body",
  "content",
  "password",
  "payload",
  "raw",
  "refresh_token",
  "secret",
  "secret_value",
  "token",
  "transcript",
  "transcript_path",
]);

/**
 * A stable diagnostic classification with no exception message.
 *
 * Parser, process and network messages may quote the bytes they rejected.
 * Those bytes can be a bearer, transcript fragment or malformed secret
 * configuration, so ordinary adapter logs retain the error class/code and
 * never the message.
 */
export function diagnostic(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const code = (error as { code?: unknown }).code;
    if (typeof code === "string" && SAFE_ERROR_CODES.has(code)) return code;
    if (error instanceof Error && SAFE_ERROR_NAMES.has(error.name)) {
      return error.name;
    }
    return "object";
  }
  return typeof error;
}

function safeFields(fields: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(fields).map(([key, value]) => [key, safeValue(key, value, 0)]),
  );
}

function safeValue(key: string, value: unknown, depth: number): unknown {
  if (SECRET_FIELDS.has(key.toLowerCase())) return "[redacted]";
  if (value === null || typeof value !== "object") return value;
  if (depth >= 4) return "[truncated]";
  if (Array.isArray(value)) return value.map((item) => safeValue("", item, depth + 1));
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>).map(([nested, item]) => [
      nested,
      safeValue(nested, item, depth + 1),
    ]),
  );
}

const SAFE_ERROR_CODES = new Set([
  "ABORT_ERR",
  "EACCES",
  "EAI_AGAIN",
  "ECONNREFUSED",
  "ECONNRESET",
  "EISDIR",
  "EMFILE",
  "ENFILE",
  "ENOENT",
  "ENOSPC",
  "ENOTDIR",
  "ENOTFOUND",
  "EPERM",
  "EPIPE",
  "ETIMEDOUT",
]);

const SAFE_ERROR_NAMES = new Set([
  "AbortError",
  "Error",
  "RangeError",
  "SyntaxError",
  "TimeoutError",
  "TypeError",
]);

function rotate(file: string): void {
  try {
    if (statSync(file).size > MAX_LOG_BYTES) {
      renameSync(file, `${file}.1`);
    }
  } catch {
    // No log yet, or an unwritable directory: nothing to rotate.
  }
}

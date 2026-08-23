/**
 * The durable observation spool (CPR-12, ADR-0078 decision 6).
 *
 * One file per harness session under `$XDG_STATE_HOME/synveda/spool/`, holding
 * every event this client has recorded and whether the gateway has it yet.
 *
 * # What this replaced, and why the replacement is not a refinement
 *
 * ADR-0027 decision 7 made this a **cursor**: the uuid of the last transcript
 * entry a gateway 2xx had accepted, and nothing else. Everything after it was
 * re-derived on the next hook by re-reading the harness's own transcript file.
 *
 * That is at-least-once only while that file still exists and still contains
 * those entries. A compaction rewrites it. A `/clear` truncates it. A deleted
 * project takes it. And a `Stop` hook that fired while the gateway was down
 * left no local record at all that anything had happened — the events existed
 * only as a byte range of somebody else's file that this adapter had chosen
 * not to copy.
 *
 * So the unit here is the **event**, copied out of the transcript as soon as a
 * hook is handed it and kept until the gateway answers for it.
 *
 * # Two programs read these bytes
 *
 * This writes them; `synveda session flush|spool status|spool purge` reads
 * them. A hook runs for milliseconds inside somebody else's process and cannot
 * own a retry schedule, so the thing that retries has to be able to pick up
 * where a hook left off. Any field added here must be added to
 * `crates/synveda-cli/src/spool.rs` as well — that side rewrites the whole
 * file, so a field it does not know is a field it drops.
 *
 * # Nothing reads the previous format
 *
 * A file from before this cut is not migrated, not parsed and not consulted:
 * it held a cursor and no events, so there is nothing in one to recover.
 * `~/.local/state/synveda/sessions/` is removed on sight.
 */

import { createHash } from "node:crypto";
import {
  closeSync,
  fchmodSync,
  fsyncSync,
  openSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  unlinkSync,
  writeSync,
} from "node:fs";
import { join } from "node:path";

import { log } from "./log.mjs";
import { ensureDir, legacySessionDir, spoolDir } from "./paths.mjs";
import type { SessionEventType } from "./types.mjs";

/** The format version this build writes and reads. */
export const SPOOL_VERSION = 1;

/** One recorded event. */
export interface SpoolEntry {
  /** The client's own id — the idempotency unit the API keys on. */
  client_event_id: string;
  /**
   * The client's local order. Not the server's: the gateway assigns its own
   * `sequence` on append and that one orders the timeline. This one makes a
   * bounded flush deterministic and lets `spool status` name a range.
   */
  sequence: number;
  event_type: SessionEventType;
  occurred_at: string;
  payload: unknown;
  /** SHA-256 over the canonical encoding of `payload`, hex. */
  payload_hash: string;
  delivery_attempts: number;
  last_attempt_at?: string;
  /**
   * Whether the gateway has resolved this event — true for every terminal
   * answer, including the two that store nothing useful.
   */
  acknowledged: boolean;
  outcome?: string;
}

/** One harness session's spool file. */
export interface Spool {
  spool_version: number;
  client_installation_id: string;
  client_name: string;
  /** The Synveda run, once one has been opened. */
  session_id?: string;
  /** The harness's own id for this run. */
  external_session_id: string;
  workspace_id?: string;
  project_id?: string;
  gateway_url?: string;
  /** The last transcript entry turned into an entry here. */
  recorded_through?: string;
  /** Whether a close is owed once the backlog drains. */
  close_requested: boolean;
  end_reason?: string;
  /** Carried across hooks: only `SessionStart` payloads name them. */
  transcript_path?: string;
  model?: string;
  created_at: string;
  updated_at: string;
  entries: SpoolEntry[];
}

/**
 * SHA-256 over a payload's canonical encoding, hex.
 *
 * Canonical — object keys sorted, recursively — because the Rust side computes
 * the same digest with a sorted encoder, and two encoders that disagree about
 * key order would make every entry read as corrupt on the other side.
 *
 * SHA-256 and not BLAKE3 is the one place this format diverges from the rest
 * of the product: `node:crypto` has no BLAKE3 and this package takes no
 * dependencies. Its job is detecting local corruption between the hook that
 * wrote the file and the flush that reads it. The authoritative digest of an
 * event is the server's BLAKE3, computed on append.
 */
export function payloadHash(payload: unknown): string {
  return createHash("sha256").update(canonical(payload)).digest("hex");
}

/** A payload's canonical JSON encoding: object keys sorted, recursively. */
function canonical(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, item]) => item !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`);
  return `{${entries.join(",")}}`;
}

/** The file one harness session's spool lives in. */
export function spoolFile(externalSessionId: string): string {
  const readable = externalSessionId.replace(/[^A-Za-z0-9._-]/g, "_").slice(0, 96);
  const digest = createHash("sha256").update(externalSessionId).digest("hex").slice(0, 8);
  return join(spoolDir(), `${readable}-${digest}.json`);
}

/**
 * Reads one spool, or `undefined` when there is none this build can read.
 *
 * A file carrying an unknown `spool_version` reads as absent rather than as an
 * error: the caller is a hook, and the only thing it could do with an error is
 * swallow it. It is left on disk for `synveda session spool status` to name.
 */
export function loadSpool(externalSessionId: string): Spool | undefined {
  return readSpool(spoolFile(externalSessionId));
}

/** Reads a spool by path. */
export function readSpool(path: string): Spool | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    // No spool yet, or unreadable. Either way a hook has nothing to do about
    // it: a fresh spool loses no events, because the events it would have
    // held were never delivered either.
    return undefined;
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return undefined;
  const spool = parsed as Spool;
  if (spool.spool_version !== SPOOL_VERSION) {
    log("spool.unknown_version", { path, version: spool.spool_version });
    return undefined;
  }
  if (!Array.isArray(spool.entries)) spool.entries = [];
  return spool;
}

/** A fresh spool for a harness session. */
export function newSpool(
  externalSessionId: string,
  clientName: string,
  installationId: string,
): Spool {
  const now = new Date().toISOString();
  return {
    spool_version: SPOOL_VERSION,
    client_installation_id: installationId,
    client_name: clientName,
    external_session_id: externalSessionId,
    close_requested: false,
    created_at: now,
    updated_at: now,
    entries: [],
  };
}

/**
 * Writes a spool atomically: a temporary in the same directory, `fsync`ed,
 * then renamed over the target.
 *
 * Same directory so the rename is within one filesystem and therefore atomic.
 * `fsync` before the rename because a rename that lands before the data does
 * leaves a file whose name says it is complete and whose bytes are not — which
 * is exactly the failure this format exists to survive.
 *
 * Returns whether it landed. A hook never throws over its own bookkeeping, but
 * a caller that just recorded a turn does want to know it is durable before it
 * reports success.
 */
export function saveSpool(spool: Spool, path?: string): boolean {
  const target = path ?? spoolFile(spool.external_session_id);
  spool.updated_at = new Date().toISOString();
  const temporary = `${target}.${process.pid}.tmp`;
  try {
    ensureDir(spoolDir());
    // A transcript payload is sensitive even before the gateway classifies
    // it. The explicit mode survives a permissive process umask and is kept by
    // the atomic rename.
    const handle = openSync(temporary, "w", 0o600);
    try {
      // `mode` applies only when this call creates the file. Tighten an
      // abandoned temporary from a killed process too: pids are reusable, so
      // such a name can exist before this writer opens it.
      if (process.platform !== "win32") fchmodSync(handle, 0o600);
      writeSync(handle, JSON.stringify(spool));
      fsyncSync(handle);
    } finally {
      closeSync(handle);
    }
    renameSync(temporary, target);
    return true;
  } catch (error) {
    log("spool.write_failed", { session: spool.external_session_id, error: String(error) });
    try {
      unlinkSync(temporary);
    } catch {
      // The temporary may never have been created. Nothing to clean up.
    }
    return false;
  }
}

/** Every spool on this machine, for the backlog retry. */
export function allSpools(): { path: string; spool: Spool }[] {
  let names: string[];
  try {
    names = readdirSync(spoolDir());
  } catch {
    return [];
  }
  const found: { path: string; spool: Spool }[] = [];
  for (const name of names.sort()) {
    if (!name.endsWith(".json")) continue;
    const path = join(spoolDir(), name);
    const spool = readSpool(path);
    if (spool !== undefined) found.push({ path, spool });
  }
  return found;
}

/** The entries the gateway has not resolved, in client order. */
export function pending(spool: Spool): SpoolEntry[] {
  return spool.entries.filter((entry) => !entry.acknowledged);
}

/**
 * Appends events, skipping any id already recorded.
 *
 * The skip is what makes a hook that fires twice for one turn — a retry, an
 * overlapping `Stop` and `PreCompact` — record each entry once. Returns how
 * many were new.
 */
export function record(
  spool: Spool,
  events: { event_type: SessionEventType; client_event_id: string; occurred_at: string; payload: unknown }[],
): number {
  const known = new Set(spool.entries.map((entry) => entry.client_event_id));
  let next = spool.entries.reduce((high, entry) => Math.max(high, entry.sequence), 0);
  let added = 0;
  for (const event of events) {
    if (known.has(event.client_event_id)) continue;
    known.add(event.client_event_id);
    next += 1;
    added += 1;
    spool.entries.push({
      client_event_id: event.client_event_id,
      sequence: next,
      event_type: event.event_type,
      occurred_at: event.occurred_at,
      payload: event.payload,
      payload_hash: payloadHash(event.payload),
      delivery_attempts: 0,
      acknowledged: false,
    });
  }
  return added;
}

/**
 * Marks what the gateway resolved, keyed by the client's own event id.
 *
 * By id and not by position, because the answer is per event: a batch that
 * overlapped a previous one by three of ten comes back with three `duplicate`
 * and seven `appended`, and a denied event shortens nothing — the gateway
 * answers for every event it was sent.
 */
export function acknowledge(spool: Spool, outcomes: Map<string, string>): number {
  const at = new Date().toISOString();
  let marked = 0;
  for (const entry of spool.entries) {
    if (entry.acknowledged) continue;
    const outcome = outcomes.get(entry.client_event_id);
    if (outcome === undefined) continue;
    entry.acknowledged = true;
    entry.outcome = outcome;
    entry.last_attempt_at = at;
    marked += 1;
  }
  return marked;
}

/** Counts an attempt against everything still pending. */
export function recordAttempt(spool: Spool): void {
  const at = new Date().toISOString();
  for (const entry of spool.entries) {
    if (entry.acknowledged) continue;
    entry.delivery_attempts += 1;
    entry.last_attempt_at = at;
  }
}

/**
 * Removes a spool whose work is finished: every event acknowledged and no
 * close owed.
 *
 * Deleting acknowledged events is otherwise `synveda session spool purge
 * --acknowledged`'s job. This is the narrow case where the whole file is
 * finished, and leaving it would mean a directory that only ever grows.
 */
export function retireIfComplete(spool: Spool, path?: string): boolean {
  if (spool.close_requested) return false;
  if (spool.entries.some((entry) => !entry.acknowledged)) return false;
  try {
    rmSync(path ?? spoolFile(spool.external_session_id), { force: true });
    return true;
  } catch {
    return false;
  }
}

/**
 * Removes the pre-cut state directory, once.
 *
 * Not a migration: the old format held a cursor and no events, so there is
 * nothing in one to carry forward (ADR-0078 decision 6). It is deleted rather
 * than left because a directory of stale cursors nothing reads is a thing
 * somebody will eventually try to interpret.
 */
export function removeLegacyState(): void {
  try {
    rmSync(legacySessionDir(), { recursive: true, force: true });
  } catch {
    // Best-effort. A hook never fails over housekeeping.
  }
}

/**
 * One disclosure per project (ADR-0027 decision 13). Login is consent, but
 * silent capture is not something this product gets to do. The `wx` flag makes
 * the claim atomic, so concurrent sessions disclose once.
 */
export function claimDisclosure(cwd: string | undefined): boolean {
  if (cwd === undefined || cwd.length === 0) return false;
  try {
    const dir = join(spoolDir(), "..", "disclosed");
    ensureDir(dir);
    const name = createHash("sha256").update(cwd).digest("hex").slice(0, 16);
    const handle = openSync(join(dir, name), "wx");
    try {
      writeSync(handle, cwd);
    } finally {
      closeSync(handle);
    }
    return true;
  } catch {
    return false;
  }
}

/**
 * Recording and delivery (CPR-12, ADR-0078 decisions 6 and 7).
 *
 * The order here is the whole design and it is worth reading once:
 *
 *   1. read the transcript delta,
 *   2. **record it into the spool and fsync**,
 *   3. only then try to deliver it.
 *
 * Step 2 before step 3 is what makes an unreachable gateway, a killed hook, a
 * compaction and a reboot all cost nothing. The previous design did 1 and 3
 * and skipped 2, which is why everything after the last acknowledged entry
 * lived only in somebody else's transcript file.
 *
 * # The loss boundary this cannot close
 *
 * If the host client terminates without running **any** lifecycle hook —
 * `kill -9`, a harness panic, a machine losing power mid-turn — the events of
 * the turn in flight are lost. They were never handed to a hook, so no code
 * here ever saw them. That is bounded at "the turn in flight when the client
 * died" rather than "everything since the gateway went down", and it is stated
 * in the README and INSTALL.md rather than left for somebody to discover.
 */

import { appendEvents, endSession } from "./client.mjs";
import type { AdapterConfig } from "./config.mjs";
import { chunk, MAX_EVENTS_PER_BATCH, toSessionEvents } from "./events.mjs";
import { log } from "./log.mjs";
import {
  acknowledge,
  allSpools,
  pending,
  record,
  recordAttempt,
  retireIfComplete,
  saveSpool,
  type Spool,
} from "./spool.mjs";
import { entriesAfter, readTranscript } from "./transcript.mjs";

/** What one delivery attempt did. */
export interface Delivered {
  /** Events the gateway resolved this time. */
  acknowledged: number;
  /** Events still held after it. */
  pending: number;
  /** Whether every batch reached the gateway. */
  complete: boolean;
}

/**
 * Reads the transcript delta and records it into the spool, durably.
 *
 * Returns how many new events were recorded. The read happens before anything
 * else on purpose: `PreCompact` runs while compaction proceeds, so the content
 * must be in memory before the transcript can be rewritten underneath us.
 */
export function recordDelta(spool: Spool, transcriptPath: string | undefined): number {
  const path = transcriptPath ?? spool.transcript_path;
  if (path === undefined) {
    log("record.no_transcript", { session: spool.external_session_id });
    return 0;
  }
  spool.transcript_path = path;
  const delta = entriesAfter(readTranscript(path), spool.recorded_through);
  if (delta.resynced) {
    // The watermark named an entry the transcript no longer holds — a
    // compaction, a `/clear`, a fork. Everything it still holds is re-read;
    // the ids are the entries' own, so anything already recorded is skipped
    // by `record` and anything already delivered comes back `duplicate`.
    log("record.resynced", {
      session: spool.external_session_id,
      entries: delta.entries.length,
    });
  }
  const events = toSessionEvents(delta.entries, spool.model);
  const added = record(spool, events);
  const last = delta.entries[delta.entries.length - 1];
  if (last !== undefined) spool.recorded_through = last.uuid;
  return added;
}

/**
 * Delivers one spool's pending events.
 *
 * `deadlineAt` bounds the whole attempt: `SessionEnd` gives it a fixed budget
 * because a hook that blocks a client's exit indefinitely is worse than one
 * that leaves a backlog. Reaching the deadline is not a failure — it is the
 * designed outcome, and what is left is what the next `SessionStart` retries.
 */
export async function deliver(
  spool: Spool,
  config: AdapterConfig,
  bearer: string,
  deadlineAt?: number,
): Promise<Delivered> {
  const sessionId = spool.session_id;
  if (sessionId === undefined) {
    // Recorded but nowhere to go yet: the run has not been opened. Not a
    // failure — the next SessionStart opens it and delivers this.
    return { acknowledged: 0, pending: pending(spool).length, complete: false };
  }
  let acknowledged = 0;
  let complete = true;
  for (const batch of chunk(pending(spool), MAX_EVENTS_PER_BATCH)) {
    if (deadlineAt !== undefined && Date.now() >= deadlineAt) {
      complete = false;
      log("deliver.deadline", {
        session: spool.external_session_id,
        unsent: pending(spool).length,
      });
      break;
    }
    recordAttempt(spool);
    const appendStarted = Date.now();
    const result = await appendEvents(config, bearer, sessionId, {
      events: batch.map((entry) => ({
        event_type: entry.event_type,
        client_event_id: entry.client_event_id,
        occurred_at: entry.occurred_at,
        payload: entry.payload,
      })),
    });
    log("deliver.batch", {
      session: spool.external_session_id,
      events: batch.length,
      ok: result.ok,
      elapsed_ms: Date.now() - appendStarted,
    });
    if (!result.ok) {
      complete = false;
      log("deliver.failed", {
        session: spool.external_session_id,
        status: result.status,
        reason: result.reason,
        unsent: pending(spool).length,
      });
      break;
    }
    const outcomes = new Map(
      result.value.events.map((event) => [event.client_event_id, event.outcome]),
    );
    const marked = acknowledge(spool, outcomes);
    acknowledged += marked;
    if (result.value.denied > 0 || result.value.quarantined > 0) {
      log("deliver.withheld", {
        session: spool.external_session_id,
        quarantined: result.value.quarantined,
        denied: result.value.denied,
      });
    }
    // The gateway answered without resolving anything this batch sent.
    // Continuing would loop forever on the same events.
    if (marked === 0) {
      complete = false;
      log("deliver.unacknowledged", { session: spool.external_session_id, batch: batch.length });
      break;
    }
  }
  return { acknowledged, pending: pending(spool).length, complete };
}

/**
 * Closes a run whose events have all landed, or leaves the close owed.
 *
 * The two-phase close (ADR-0076): a client that still has a backlog says
 * `ending` — no new work, still flushing — and whoever drains the spool
 * finishes the job. `synveda session flush` does that, and so does the next
 * `SessionStart`'s backlog retry.
 */
export async function closeRun(
  spool: Spool,
  config: AdapterConfig,
  bearer: string,
  reason: string | undefined,
): Promise<void> {
  const sessionId = spool.session_id;
  if (sessionId === undefined) return;
  const drained = pending(spool).length === 0;
  const result = await endSession(config, bearer, sessionId, {
    status: drained ? "ended" : "ending",
    ...(reason === undefined ? {} : { end_reason: reason }),
  });
  if (!result.ok) {
    log("close.failed", {
      session: spool.external_session_id,
      status: result.status,
      reason: result.reason,
    });
  }
  if (drained) {
    spool.close_requested = false;
  } else {
    spool.close_requested = true;
    spool.end_reason = reason;
  }
}

/**
 * Retries every spool on this machine.
 *
 * Runs at `SessionStart`, which is the one hook that is allowed to take a
 * moment and the one that fires when somebody is present again. It carries
 * other conversations' backlogs as well as this one's — a session that ended
 * while the gateway was down has no hook of its own left to fire, and without
 * this its events would sit on disk until somebody ran the CLI.
 */
export async function retryBacklog(
  config: AdapterConfig,
  bearer: string,
  exceptExternalId: string,
  deadlineAt: number,
): Promise<number> {
  let delivered = 0;
  for (const { path, spool } of allSpools()) {
    if (spool.external_session_id === exceptExternalId) continue;
    if (pending(spool).length === 0 && !spool.close_requested) {
      retireIfComplete(spool, path);
      continue;
    }
    if (Date.now() >= deadlineAt) {
      log("backlog.deadline", { remaining: spool.external_session_id });
      break;
    }
    const result = await deliver(spool, config, bearer, deadlineAt);
    delivered += result.acknowledged;
    if (spool.close_requested && result.pending === 0) {
      await closeRun(spool, config, bearer, spool.end_reason);
    }
    saveSpool(spool, path);
    retireIfComplete(spool, path);
  }
  if (delivered > 0) log("backlog.delivered", { events: delivered });
  return delivered;
}

/**
 * `Stop`, `PreCompact`, and `SessionEnd` → `POST /v1/observe`
 * (ADR-0027 decisions 2 and 7).
 *
 * `Stop` is the real write seam: it carries the transcript path and
 * fires at the end of every turn. `PreCompact` and `SessionEnd` are the
 * same code doing a retry — they inject nothing, because they cannot,
 * and `PreCompact` does not even carry a transcript path, which is why
 * the spool holds one.
 *
 * The cursor advances only on a gateway 2xx. Everything else — a failed
 * batch, a killed hook, a crashed machine — leaves it where the last
 * accepted batch put it, and the next hook resends. The buffer reports
 * the overlap as duplicates and re-enqueues nothing (ADR-0020
 * decision 2), so at-least-once delivery costs nothing and loses
 * nothing.
 */

import { observe } from "./client.mjs";
import type { AdapterConfig } from "./config.mjs";
import { resolveBearer } from "./credentials.mjs";
import { chunk, MAX_EVENTS_PER_BATCH, toObserveEvents } from "./events.mjs";
import { log } from "./log.mjs";
import { qualifiedSessionId } from "./session-start.mjs";
import { loadSession, saveSession } from "./spool.mjs";
import { entriesAfter, readTranscript } from "./transcript.mjs";
import type { HookInput, HookOutput } from "./types.mjs";

export async function flush(input: HookInput, config: AdapterConfig): Promise<HookOutput> {
  if (!config.observe) return {};
  const sessionId = qualifiedSessionId(input.session_id);
  const state = loadSession(sessionId);

  // `PreCompact` carries no transcript path; the spool does.
  const transcriptPath = input.transcript_path ?? state?.transcript_path;
  if (transcriptPath === undefined) {
    log("observe.no_transcript", { session: sessionId, hook: input.hook_event_name });
    return {};
  }

  const bearer = await resolveBearer();
  // Silent: the session-start hook already told the user to log in, and
  // saying it again on every turn would be noise, not help.
  if (bearer === undefined) return {};

  // Read before anything else. `PreCompact` runs in the background while
  // compaction proceeds, so the content must be in memory before the
  // transcript can be rewritten underneath us.
  const delta = entriesAfter(readTranscript(transcriptPath), state?.cursor);
  if (delta.resynced) {
    log("observe.resynced", { session: sessionId, entries: delta.entries.length });
  }

  const events = toObserveEvents(delta.entries, input.model);
  if (events.length === 0) {
    saveSession(sessionId, transcriptPath, state?.cursor);
    return {};
  }

  let cursor = state?.cursor;
  let accepted = 0;
  let duplicates = 0;
  for (const batch of chunk(events, MAX_EVENTS_PER_BATCH)) {
    const result = await observe(config, bearer, { session_id: sessionId, events: batch });
    if (!result.ok) {
      log("observe.failed", {
        session: sessionId,
        status: result.status,
        reason: result.reason,
        // The cursor stays where the last accepted batch left it.
        unsent: events.length - accepted - duplicates,
      });
      break;
    }
    accepted += result.value.accepted;
    duplicates += result.value.duplicates;
    const last = batch[batch.length - 1];
    if (last !== undefined) {
      cursor = last.idempotency_key;
      saveSession(sessionId, transcriptPath, cursor);
    }
    if (result.value.denied > 0 || result.value.quarantined > 0) {
      log("observe.withheld", {
        session: sessionId,
        quarantined: result.value.quarantined,
        denied: result.value.denied,
      });
    }
  }

  log("observe.done", {
    session: sessionId,
    hook: input.hook_event_name,
    events: events.length,
    accepted,
    duplicates,
  });
  return {};
}

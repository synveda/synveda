/**
 * `Stop`, `PreCompact` and `SessionEnd` (CPR-12, ADR-0078 decision 7).
 *
 * All three record; they differ in what they do after.
 *
 * | Hook | Records | Then |
 * |---|---|---|
 * | `Stop` | the turn | returns once the local spool is durable |
 * | `PreCompact` | everything the transcript still holds | returns once the local spool is durable |
 * | `SessionEnd` | the last turn | a **bounded** synchronous flush; `clear` closes |
 *
 * `Stop` is the real write seam: it fires at the end of every turn. Its hook is
 * synchronous because Claude Code kills async hooks when `-p` tears down, but
 * only the local durable write sits in the turn. Network delivery remains the
 * next SessionEnd, SessionStart or explicit flush command's work. PreCompact
 * uses the same local-only boundary before Claude rewrites the transcript.
 *
 * `PreCompact` matters for one reason: it runs while compaction proceeds, so
 * the content must be in memory before the transcript is rewritten underneath
 * us. Recording there is what stops a compaction eating a turn.
 *
 * `SessionEnd` is bounded on purpose. A hook that blocks a client's exit until
 * a down gateway comes back is worse than one that leaves a backlog, so it
 * gets a fixed budget and whatever does not fit is the next `SessionStart`'s
 * to retry — or `synveda session flush`'s.
 */

import { resolveGateway, type AdapterConfig } from "./config.mjs";
import { resolveBearer } from "./credentials.mjs";
import { closeRun, deliver, recordDelta } from "./deliver.mjs";
import { CLIENT_NAME } from "./client.mjs";
import { installationId } from "./install-id.mjs";
import { observeGit } from "./git.mjs";
import { log } from "./log.mjs";
import {
  bindGateway,
  loadOrCreateSpool,
  pending,
  record,
  retireIfComplete,
  saveSpool,
} from "./spool.mjs";
import { harnessSessionId, pinPlacement } from "./session-start.mjs";
import type { HookInput, HookOutput } from "./types.mjs";
import { readTranscript } from "./transcript.mjs";

/**
 * How long `SessionEnd`'s flush gets.
 *
 * Under the hook's own timeout in `hooks.json`, so the deadline that fires is
 * this one and the client is never the thing that kills it — a hook killed by
 * its host leaves the spool unwritten, which is the one outcome this whole
 * design exists to avoid.
 */
const END_FLUSH_BUDGET_MS = 3000;
// Reusable Codex/Copilot tasks flush within two seconds, including credentials.
// This also fits Codex 0.152.0's three-second exit-hook ceiling.
const TASK_EXIT_BUDGET_MS = 2000;

export async function turn(
  input: HookInput,
  configured: AdapterConfig,
  readEntries: typeof readTranscript = readTranscript,
): Promise<HookOutput> {
  const hookStarted = Date.now();
  if (!configured.observe) return {};
  if (
    input.hook_event_name !== "Stop" &&
    input.hook_event_name !== "PreCompact" &&
    input.hook_event_name !== "SessionEnd"
  ) {
    log("turn.unrecognised", { hook: input.hook_event_name });
    return {};
  }
  const externalId = harnessSessionId(input.session_id);
  if (externalId === undefined) {
    log("session.identity_unavailable", { hook: input.hook_event_name });
    return {};
  }
  const currentInstallation = installationId();
  if (currentInstallation === undefined) {
    log("installation.identity_unavailable", { hook: input.hook_event_name });
    return {};
  }
  const spool = loadOrCreateSpool(externalId, configured.clientName ?? CLIENT_NAME, currentInstallation);
  if (spool === undefined) return {};
  if (!pinPlacement(spool, configured)) {
    log("session.binding_mismatch", { session: externalId });
    return {};
  }
  const currentCheckout = observeGit(input.cwd, spool.client_installation_id);
  if (spool.checkout === undefined) spool.checkout = currentCheckout;
  if (input.transcript_path !== undefined) spool.transcript_path = input.transcript_path;
  // Terminal intent is part of the durable observation, even when a login is
  // unavailable at this hook. A later start can drain and close the run.
  const closesTask = input.hook_event_name === "SessionEnd" &&
    (configured.clientName ?? CLIENT_NAME) === CLIENT_NAME && input.reason === "clear";
  if (closesTask) {
    spool.close_requested = true;
    spool.end_reason = endReason(input, false);
  }

  // Record first, always, and persist before anything touches the network.
  // This is the step the previous design did not have.
  let recorded = recordDelta(spool, input.transcript_path, readEntries, spool.checkout);
  if (input.hook_event_name === "PreCompact") {
    let previousBoundary = -1;
    for (let index = spool.entries.length - 1; index >= 0; index -= 1) {
      if (spool.entries[index]?.event_type === "session.compaction_boundary") {
        previousBoundary = index;
        break;
      }
    }
    const sourceEntries = spool.entries.slice(previousBoundary + 1);
    const localHigh = spool.entries.reduce((high, entry) => Math.max(high, entry.sequence), 0) + 1;
    recorded += record(spool, [{
      event_type: "session.compaction_boundary",
      client_event_id: `precompact:${spool.recorded_through ?? "none"}:${localHigh}`,
      occurred_at: new Date().toISOString(),
      payload: {
        schema_version: 1,
        local_high_sequence: localHigh,
        expected_client_event_ids: sourceEntries.slice(-64).map((entry) => entry.client_event_id),
        expected_ids_truncated: sourceEntries.length > 64,
      },
    }]);
  }
  const durable = saveSpool(spool);
  if (!durable) {
    // The spool did not land. Delivering anyway would risk sending events
    // that nothing on disk remembers, so a failure here would be invisible.
    log("turn.not_durable", { session: externalId, recorded });
    return {};
  }

  // Claude Code 2.1.241 cancels async hooks during successful `-p` teardown.
  // Registering Stop and PreCompact synchronously crosses the durability
  // boundary, while returning here keeps an interactive turn and compaction
  // independent of gateway latency.
  if (input.hook_event_name === "Stop" || input.hook_event_name === "PreCompact") {
    const held = pending(spool).length;
    log("turn.done", {
      session: externalId,
      hook: input.hook_event_name,
      recorded,
      acknowledged: 0,
      pending: held,
      complete: held === 0,
      elapsed_ms: Date.now() - hookStarted,
    });
    return {};
  }

  // Claude can emit SessionEnd on ordinary exit or a switch to another
  // conversation, then later resume the same native ID. Only /clear retires
  // that identity. The Codex/Copilot wrappers have their own task semantics.
  const taskDeadline = closesTask ? undefined : hookStarted + TASK_EXIT_BUDGET_MS;
  const bearer = await resolveBearer(taskDeadline);
  // Silent: the session-start hook already told the user to log in, and saying
  // it again on every turn would be noise rather than help. The events are
  // recorded regardless and go out when a credential exists.
  if (bearer === undefined) return {};
  const config = resolveGateway(configured, bearer);
  if (!bindGateway(spool, config.gatewayUrl)) {
    log("spool.held", { session: externalId, reason: "gateway_mismatch", corrupt: 0 });
    return {};
  }

  const result = await deliver(
    spool,
    config,
    bearer.token,
    taskDeadline ?? Date.now() + END_FLUSH_BUDGET_MS,
  );

  if (closesTask) {
    await closeRun(spool, config, bearer.token, endReason(input, result.complete));
  }
  const saved = saveSpool(spool);
  if (saved && closesTask) retireIfComplete(spool);

  log("turn.done", {
    session: externalId,
    hook: input.hook_event_name,
    recorded,
    acknowledged: result.acknowledged,
    pending: result.pending,
    complete: result.complete,
    elapsed_ms: Date.now() - hookStarted,
  });
  return {};
}

/**
 * Why the run stopped, in the client's words.
 *
 * The harness's own reason when it gives one, and otherwise a statement about
 * what this adapter knows: that the flush did not finish is a fact worth
 * carrying, because it explains a run whose last events arrive minutes later.
 */
function endReason(input: HookInput, complete: boolean): string | undefined {
  const reason = typeof input.reason === "string" ? input.reason.trim() : "";
  if (reason.length > 0) {
    return complete ? reason : `${reason}; delivery incomplete at exit`;
  }
  return complete ? undefined : "delivery incomplete at exit";
}

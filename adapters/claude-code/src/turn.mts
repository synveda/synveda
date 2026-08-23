/**
 * `Stop`, `PreCompact` and `SessionEnd` (CPR-12, ADR-0078 decision 7).
 *
 * All three record; they differ in what they do after.
 *
 * | Hook | Records | Then |
 * |---|---|---|
 * | `Stop` | the turn | starts delivery |
 * | `PreCompact` | everything the transcript still holds | starts delivery |
 * | `SessionEnd` | the last turn | a **bounded** synchronous flush, then closes |
 *
 * `Stop` is the real write seam: it fires at the end of every turn. The
 * recording is durable before any delivery is attempted, which is the property
 * that makes an unreachable gateway cost nothing.
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
import { log } from "./log.mjs";
import { loadSpool, newSpool, retireIfComplete, saveSpool } from "./spool.mjs";
import { harnessSessionId } from "./session-start.mjs";
import type { HookInput, HookOutput } from "./types.mjs";

/**
 * How long `SessionEnd`'s flush gets.
 *
 * Under the hook's own timeout in `hooks.json`, so the deadline that fires is
 * this one and the client is never the thing that kills it — a hook killed by
 * its host leaves the spool unwritten, which is the one outcome this whole
 * design exists to avoid.
 */
const END_FLUSH_BUDGET_MS = 3000;

/** How long `Stop`'s opportunistic delivery gets. */
const TURN_DELIVERY_BUDGET_MS = 2000;

export async function turn(input: HookInput, configured: AdapterConfig): Promise<HookOutput> {
  const hookStarted = Date.now();
  if (!configured.observe) return {};
  const externalId = harnessSessionId(input.session_id);
  const spool = loadSpool(externalId) ?? newSpool(externalId, CLIENT_NAME, installationId());
  if (input.transcript_path !== undefined) spool.transcript_path = input.transcript_path;

  // Record first, always, and persist before anything touches the network.
  // This is the step the previous design did not have.
  const recorded = recordDelta(spool, input.transcript_path);
  const durable = saveSpool(spool);
  if (!durable) {
    // The spool did not land. Delivering anyway would risk sending events
    // that nothing on disk remembers, so a failure here would be invisible.
    log("turn.not_durable", { session: externalId, recorded });
    return {};
  }

  const bearer = await resolveBearer();
  // Silent: the session-start hook already told the user to log in, and saying
  // it again on every turn would be noise rather than help. The events are
  // recorded regardless and go out when a credential exists.
  if (bearer === undefined) return {};
  const config = resolveGateway(configured, bearer);
  spool.gateway_url = config.gatewayUrl;

  const ending = input.hook_event_name === "SessionEnd";
  const budget = ending ? END_FLUSH_BUDGET_MS : TURN_DELIVERY_BUDGET_MS;
  const result = await deliver(spool, config, bearer.token, Date.now() + budget);

  if (ending) {
    await closeRun(spool, config, bearer.token, endReason(input, result.complete));
  }
  saveSpool(spool);
  if (ending) retireIfComplete(spool);

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

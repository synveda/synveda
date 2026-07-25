/**
 * `SessionStart` → `POST /v1/inject` (ADR-0027 decision 2).
 *
 * This is the only hook whose output becomes context, and therefore the
 * only injection seam this harness offers: `PreCompact` has no
 * context-injection output at all, so post-compaction re-injection is
 * this same hook firing again with `source: "compact"`.
 *
 * Nothing here fails a session. A missing credential, an unreachable
 * gateway, or an expired deadline yields a hook that contributes no
 * context and returns success (decision 3).
 */

import { inject } from "./client.mjs";
import { resolveGateway, type AdapterConfig } from "./config.mjs";
import { resolveBearer, SIGN_IN_MESSAGE } from "./credentials.mjs";
import { log } from "./log.mjs";
import { claimDisclosure, loadSession, saveSession } from "./spool.mjs";
import { lastUserPrompt, readTranscript } from "./transcript.mjs";
import type { HookInput, HookOutput, InjectRequest } from "./types.mjs";

/**
 * The audit correlation (ADR-0027 decision 10): one opaque, content-free
 * string that joins this session's `context.injected` event to its
 * `memory.observed` events in the AUD-1 chain.
 */
export function qualifiedSessionId(sessionId: string | undefined): string {
  const id = sessionId !== undefined && sessionId.length > 0 ? sessionId : "unknown";
  return `claude-code:${id}`.slice(0, 200);
}

export async function sessionStart(
  input: HookInput,
  configured: AdapterConfig,
): Promise<HookOutput> {
  const sessionId = qualifiedSessionId(input.session_id);
  // Remember the transcript path and the model even when injection is
  // off: this is the only hook whose payload carries either, and the
  // observe hooks want both.
  remember(sessionId, input);
  if (!configured.inject) return {};

  const bearer = await resolveBearer();
  if (bearer === undefined) return { systemMessage: SIGN_IN_MESSAGE };
  const config = resolveGateway(configured, bearer);

  const request: InjectRequest = { session_id: sessionId };
  const task = deriveTask(input.source, input.transcript_path);
  if (task !== undefined) request.task = task;
  const budget = budgetFor(input.source, config);
  if (budget !== undefined) request.budget_tokens = budget;

  const started = Date.now();
  const result = await inject(config, bearer.token, request);
  const elapsedMs = Date.now() - started;

  if (!result.ok) {
    log("inject.failed", {
      session: sessionId,
      status: result.status,
      reason: result.reason,
      elapsed_ms: elapsedMs,
    });
    // An expired or rejected credential is the one failure the user can
    // act on; everything else stays quiet and simply has no memory.
    return result.status === 401 ? { systemMessage: SIGN_IN_MESSAGE } : {};
  }

  log("inject.ok", {
    session: sessionId,
    source: input.source,
    task: task !== undefined,
    block_hash: result.value.block_hash,
    records: result.value.record_ids.length,
    tokens: result.value.tokens,
    // A degraded inject still delivers context and stays silent to the
    // user: it is already recorded in the audit event and the metrics.
    degraded: result.degraded,
    elapsed_ms: elapsedMs,
  });

  const output: HookOutput = {};
  const text = result.value.text.trim();
  if (text.length > 0) {
    output.hookSpecificOutput = {
      hookEventName: "SessionStart",
      additionalContext: text,
    };
  }
  const disclosure = disclose(input.cwd, config);
  if (disclosure !== undefined) output.systemMessage = disclosure;
  return output;
}

/**
 * A cold start has no task — `SessionStart` fires before any prompt, so
 * the block is the recency-ordered branch by construction (ADR-0025
 * decision 5). A resumed, forked, or compacted session does have one,
 * and post-compaction is exactly where relevance is worth the embed
 * round-trip (ADR-0027 decision 11).
 */
function deriveTask(
  source: string | undefined,
  transcriptPath: string | undefined,
): string | undefined {
  if (source !== "resume" && source !== "compact" && source !== "fork") return undefined;
  if (transcriptPath === undefined) return undefined;
  return lastUserPrompt(readTranscript(transcriptPath));
}

/**
 * A request budget narrows, never widens (ADR-0026 decision 7). The
 * harness does not tell a hook how much room is left, so the adapter
 * sends only what the project configured.
 */
function budgetFor(source: string | undefined, config: AdapterConfig): number | undefined {
  if (source === "compact") return config.compactBudgetTokens ?? config.budgetTokens;
  return config.budgetTokens;
}

/**
 * Carries forward what only this hook's payload knows — the transcript
 * path and the model — without disturbing the cursor, which belongs to
 * the flush path alone.
 */
function remember(sessionId: string, input: HookInput): void {
  if (input.transcript_path === undefined && input.model === undefined) return;
  const existing = loadSession(sessionId);
  saveSession(sessionId, {
    transcript_path: input.transcript_path ?? existing?.transcript_path,
    cursor: existing?.cursor,
    model: input.model ?? existing?.model,
  });
}

function disclose(cwd: string | undefined, config: AdapterConfig): string | undefined {
  if (!config.observe) return undefined;
  if (!claimDisclosure(cwd)) return undefined;
  return (
    `Synveda is active in this project: session transcripts are sent to ${config.gatewayUrl} ` +
    "for governed memory, and context is composed back at session start. " +
    "Set SYNVEDA_DISABLED=1, or `.synveda/config.json` with {\"disabled\": true}, to turn it off."
  );
}

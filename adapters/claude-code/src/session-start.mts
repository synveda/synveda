/**
 * `SessionStart` → open or resume the run, retry the backlog, compose context
 * (CPR-12, ADR-0078 decision 7).
 *
 * This is the only hook whose output becomes context, and therefore the only
 * injection seam this harness offers: `PreCompact` has no context-injection
 * output at all, so post-compaction re-injection is this same hook firing
 * again with `source: "compact"`.
 *
 * It does three things, in this order and for this reason:
 *
 *   1. **Open or resume** the Synveda run, so everything after it has somewhere
 *      to go.
 *   2. **Retry the backlog** — this conversation's and every other one's.
 *      A session that ended while the gateway was down has no hook of its own
 *      left to fire, so this is where its events finally land.
 *   3. **Compose** the context block and return it.
 *
 * Nothing here fails a session. A missing credential, an unreachable gateway
 * or an expired deadline yields a hook that contributes no context and returns
 * success (ADR-0027 decision 3).
 */

import { randomUUID } from "node:crypto";

import { contextRun, CLIENT_NAME, CLIENT_VERSION, me, openSession } from "./client.mjs";
import { resolveGateway, type AdapterConfig } from "./config.mjs";
import { resolveBearer, SIGN_IN_MESSAGE } from "./credentials.mjs";
import { recordDelta, retryBacklog, deliver } from "./deliver.mjs";
import { installationId } from "./install-id.mjs";
import { log } from "./log.mjs";
import {
  claimDisclosure,
  loadSpool,
  newSpool,
  removeLegacyState,
  saveSpool,
  type Spool,
} from "./spool.mjs";
import { lastUserPrompt, readTranscript } from "./transcript.mjs";
import type { HookInput, HookOutput } from "./types.mjs";

/**
 * How long the backlog retry gets before the session start gives up on it.
 *
 * Bounded because this hook is on the path to the model's first token: a
 * machine holding a week of undelivered events must not make somebody wait for
 * it. What does not fit is retried at the next start, or by `synveda session
 * flush`.
 */
const BACKLOG_BUDGET_MS = 2000;

export async function sessionStart(
  input: HookInput,
  configured: AdapterConfig,
): Promise<HookOutput> {
  // The old per-session cursor directory, removed once. Not a migration: it
  // held a cursor and no events (ADR-0078 decision 6).
  removeLegacyState();

  const externalId = harnessSessionId(input.session_id);
  const spool =
    loadSpool(externalId) ?? newSpool(externalId, CLIENT_NAME, installationId());
  if (input.transcript_path !== undefined) spool.transcript_path = input.transcript_path;
  if (input.model !== undefined) spool.model = input.model;

  const bearer = await resolveBearer();
  if (bearer === undefined) {
    // Record anyway. A conversation that starts before anybody has logged in
    // still happened, and the events are worth keeping for the session that
    // follows the login.
    recordDelta(spool, input.transcript_path);
    saveSpool(spool);
    return { systemMessage: SIGN_IN_MESSAGE };
  }
  const config = resolveGateway(configured, bearer);
  spool.gateway_url = config.gatewayUrl;

  // 1. The run.
  const opened = await resolveRun(spool, config, bearer.token, input);
  saveSpool(spool);
  if (!opened) {
    // No run and therefore nowhere to compose from. Events keep accumulating
    // locally and the next start tries again.
    recordDelta(spool, input.transcript_path);
    saveSpool(spool);
    return {};
  }

  // 2. The backlog — this conversation's, then everything else's.
  recordDelta(spool, input.transcript_path);
  await deliver(spool, config, bearer.token, Date.now() + BACKLOG_BUDGET_MS);
  saveSpool(spool);
  await retryBacklog(config, bearer.token, externalId, Date.now() + BACKLOG_BUDGET_MS);

  // 3. The context block.
  if (!configured.inject) return disclosureOnly(input, config);

  const request: { query?: string; budget_tokens?: number } = {};
  const task = deriveTask(input.source, spool.transcript_path);
  if (task !== undefined) request.query = task;
  const budget = budgetFor(input.source, config);
  if (budget !== undefined) request.budget_tokens = budget;

  const started = Date.now();
  const result = await contextRun(
    config,
    bearer.token,
    spool.session_id as string,
    request,
    // A fresh key per start: a resumed conversation composing again is a new
    // composition over a corpus that may have moved, not a retry.
    `cc-ctx-${randomUUID()}`,
  );
  const elapsedMs = Date.now() - started;

  if (!result.ok) {
    log("context.failed", {
      session: externalId,
      status: result.status,
      reason: result.reason,
      elapsed_ms: elapsedMs,
    });
    // An expired or rejected credential is the one failure the user can act
    // on; everything else stays quiet and simply has no memory.
    return result.status === 401 ? { systemMessage: SIGN_IN_MESSAGE } : {};
  }

  log("context.ok", {
    session: externalId,
    source: input.source,
    task: task !== undefined,
    block_hash: result.value.block_hash,
    entries: result.value.entry_count,
    tokens: result.value.tokens,
    degraded: result.degraded,
    elapsed_ms: elapsedMs,
  });

  const output: HookOutput = {};
  const text = result.value.rendered.trim();
  if (text.length > 0) {
    output.hookSpecificOutput = { hookEventName: "SessionStart", additionalContext: text };
  }
  const disclosure = disclose(input.cwd, config);
  if (disclosure !== undefined) output.systemMessage = disclosure;
  return output;
}

/**
 * The harness's id for this conversation, bounded and never empty.
 *
 * Sent as `external_session_id`, which is what makes opening idempotent: a
 * hook holding only this can find the run it already opened instead of minting
 * a second one.
 */
export function harnessSessionId(sessionId: string | undefined): string {
  const id = sessionId !== undefined && sessionId.length > 0 ? sessionId : "unknown";
  return id.slice(0, 200);
}

/**
 * Ensures `spool.session_id`, opening the run when there is not one yet.
 *
 * Returns whether there is a run to work with.
 */
async function resolveRun(
  spool: Spool,
  config: AdapterConfig,
  bearer: string,
  input: HookInput,
): Promise<boolean> {
  if (spool.session_id !== undefined) return true;

  const workspace = await resolveWorkspace(spool, config, bearer);
  if (workspace === undefined) return false;

  const result = await openSession(
    config,
    bearer,
    {
      workspace_id: workspace,
      ...(spool.project_id === undefined ? {} : { project_id: spool.project_id }),
      client_name: CLIENT_NAME,
      client_version: CLIENT_VERSION,
      client_installation_id: spool.client_installation_id,
      external_session_id: spool.external_session_id,
      agent_name: "claude-code",
      ...(input.model === undefined ? {} : { model_name: input.model }),
    },
    // Derived from the harness id rather than random: a SessionStart that
    // times out and fires again must land on the same run.
    `cc-open-${spool.external_session_id}`,
  );
  if (!result.ok) {
    log("session.open_failed", {
      session: spool.external_session_id,
      status: result.status,
      reason: result.reason,
    });
    return false;
  }
  spool.session_id = result.value.id;
  spool.workspace_id = result.value.workspace_id;
  if (result.value.project_id !== undefined) spool.project_id = result.value.project_id;
  log("session.opened", { session: spool.external_session_id, run: result.value.id });
  return true;
}

/**
 * The workspace this run belongs to.
 *
 * Configured wins. Otherwise `/v1/me` decides, and only when it names exactly
 * one: writing a project's transcript into whichever workspace sorted first
 * would put one team's material in another team's scope, silently.
 */
async function resolveWorkspace(
  spool: Spool,
  config: AdapterConfig,
  bearer: string,
): Promise<string | undefined> {
  if (config.workspaceId !== undefined) return config.workspaceId;
  if (spool.workspace_id !== undefined) return spool.workspace_id;
  const result = await me(config, bearer);
  if (!result.ok) {
    log("workspace.unresolved", { status: result.status, reason: result.reason });
    return undefined;
  }
  const workspaces = (result.value.workspaces ?? []).filter(
    (workspace): workspace is { id: string; name?: unknown } => typeof workspace.id === "string",
  );
  if (workspaces.length === 1) return workspaces[0]?.id;
  log("workspace.ambiguous", { count: workspaces.length });
  return undefined;
}

/** The disclosure, when injection is off but observation is not. */
function disclosureOnly(input: HookInput, config: AdapterConfig): HookOutput {
  const disclosure = disclose(input.cwd, config);
  return disclosure === undefined ? {} : { systemMessage: disclosure };
}

/**
 * A cold start has no task — `SessionStart` fires before any prompt, so the
 * block is the recency-ordered branch by construction (ADR-0025 decision 5). A
 * resumed, forked or compacted session does have one, and post-compaction is
 * exactly where relevance is worth the embed round-trip (ADR-0027 decision 11).
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
 * A request budget narrows, never widens (ADR-0026 decision 7). The harness
 * does not tell a hook how much room is left, so the adapter sends only what
 * the project configured.
 */
function budgetFor(source: string | undefined, config: AdapterConfig): number | undefined {
  if (source === "compact") return config.compactBudgetTokens ?? config.budgetTokens;
  return config.budgetTokens;
}

function disclose(cwd: string | undefined, config: AdapterConfig): string | undefined {
  if (!config.observe) return undefined;
  if (!claimDisclosure(cwd)) return undefined;
  return (
    `Synveda is active in this project: session transcripts are recorded locally and sent to ` +
    `${config.gatewayUrl} for governed memory, and context is composed back at session start. ` +
    'Set SYNVEDA_DISABLED=1, or `.synveda/config.json` with {"disabled": true}, to turn it off.'
  );
}

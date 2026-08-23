/**
 * The HTTP client for the session plane (CPR-12, ADR-0078).
 *
 * Every call is deadline-bounded and returns a result rather than throwing:
 * the caller's contract is to degrade (ADR-0027 decision 3), and an exception
 * is a poor way to express "no context this time".
 *
 * Four calls, and each one names the run it is about. The three global
 * primitives this file used to speak — `/v1/observe`, `/v1/inject`,
 * `/v1/recall` — are deleted, and with them the opaque correlation string that
 * was the only thing joining a session's writes to its reads.
 */

import { randomBytes } from "node:crypto";

import type { AdapterConfig } from "./config.mjs";
import type {
  AppendEventsRequest,
  AppendEventsResponse,
  ContextRunRequest,
  ContextRunResponse,
  EndSessionRequest,
  MeResponse,
  OpenSessionRequest,
  SessionResponse,
} from "./types.mjs";

export const CLIENT_NAME = "claude-code";
export const CLIENT_VERSION = "0.2.0";

export type CallResult<T> =
  | { ok: true; value: T; degraded: string[] }
  | { ok: false; status?: number; reason: string };

/** What this caller can see — how the adapter finds a workspace to run in. */
export async function me(
  config: AdapterConfig,
  bearer: string,
): Promise<CallResult<MeResponse>> {
  return call<MeResponse>(config, bearer, "GET", "/v1/me", undefined);
}

/**
 * Opens a run, or finds the one this harness session already opened.
 *
 * The `Idempotency-Key` is derived from the harness's own session id, so a
 * `SessionStart` that times out and fires again lands on the same run rather
 * than minting a second one for the same conversation.
 */
export async function openSession(
  config: AdapterConfig,
  bearer: string,
  request: OpenSessionRequest,
  idempotencyKey: string,
): Promise<CallResult<SessionResponse>> {
  return call<SessionResponse>(config, bearer, "POST", "/v1/sessions", request, idempotencyKey);
}

/**
 * Appends a batch of events.
 *
 * Takes no `Idempotency-Key`, and that is the design rather than an omission:
 * the unit of idempotency here is the **event**, keyed by the client's own
 * `client_event_id`, because a redelivered batch overlapping a previous one by
 * three of ten must append seven and answer `duplicate` for three — at their
 * original positions — and a request-level key cannot express that.
 */
export async function appendEvents(
  config: AdapterConfig,
  bearer: string,
  sessionId: string,
  request: AppendEventsRequest,
): Promise<CallResult<AppendEventsResponse>> {
  return call<AppendEventsResponse>(
    config,
    bearer,
    "POST",
    `/v1/sessions/${encodeURIComponent(sessionId)}/events`,
    request,
  );
}

/** Composes context for a run. */
export async function contextRun(
  config: AdapterConfig,
  bearer: string,
  sessionId: string,
  request: ContextRunRequest,
  idempotencyKey: string,
): Promise<CallResult<ContextRunResponse>> {
  return call<ContextRunResponse>(
    config,
    bearer,
    "POST",
    `/v1/sessions/${encodeURIComponent(sessionId)}/context-runs`,
    request,
    idempotencyKey,
  );
}

/** Moves a run's lifecycle forward. */
export async function endSession(
  config: AdapterConfig,
  bearer: string,
  sessionId: string,
  request: EndSessionRequest,
): Promise<CallResult<SessionResponse>> {
  return call<SessionResponse>(
    config,
    bearer,
    "POST",
    `/v1/sessions/${encodeURIComponent(sessionId)}/end`,
    request,
  );
}

async function call<T>(
  config: AdapterConfig,
  bearer: string,
  method: "GET" | "POST",
  path: string,
  body: unknown,
  idempotencyKey?: string,
  timeoutMs?: number,
): Promise<CallResult<T>> {
  let status: number | undefined;
  const headers: Record<string, string> = {
    authorization: `Bearer ${bearer}`,
    "x-synveda-client": `${CLIENT_NAME}/${CLIENT_VERSION}`,
    traceparent: traceparent(),
  };
  if (body !== undefined) headers["content-type"] = "application/json";
  if (idempotencyKey !== undefined) headers["idempotency-key"] = idempotencyKey;
  try {
    const response = await fetch(`${config.gatewayUrl}${path}`, {
      method,
      signal: AbortSignal.timeout(timeoutMs ?? config.timeoutMs),
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    status = response.status;
    if (!response.ok) {
      return { ok: false, status, reason: `gateway returned ${String(status)}` };
    }
    // Read the header before the body: the degradation ladder is part of a
    // successful response, not an error (ADR-0026 decision 4).
    const header = response.headers.get("x-synveda-degraded");
    const value = (await response.json()) as T;
    return {
      ok: true,
      value,
      degraded: header === null ? [] : header.split(",").filter((part) => part.length > 0),
    };
  } catch (error) {
    return { ok: false, status, reason: reasonFor(error) };
  }
}

/**
 * A W3C trace parent, so a slow session start is one trace from the hook
 * through plan, embed, search and compose (ADR-0027 compliance notes).
 */
function traceparent(): string {
  return `00-${randomBytes(16).toString("hex")}-${randomBytes(8).toString("hex")}-01`;
}

function reasonFor(error: unknown): string {
  if (error instanceof Error && error.name === "TimeoutError") return "deadline expired";
  return String(error);
}

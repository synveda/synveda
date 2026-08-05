/**
 * The HTTP client for the two primitives the hooks own.
 *
 * Every call is deadline-bounded and returns a result rather than
 * throwing: the caller's contract is to degrade (ADR-0027 decision 3),
 * and an exception is a poor way to express "no context this time".
 *
 * Recall is deliberately absent. It was here for the hand-written MCP loop
 * and for nothing else, and since ADR-0057 decision 4 that loop is
 * `synveda mcp` — so this file is back to exactly the two primitives with
 * a hook behind them, which is also the only two that must degrade in
 * silence.
 */

import { randomBytes } from "node:crypto";

import type { AdapterConfig } from "./config.mjs";
import type {
  InjectRequest,
  InjectResponse,
  ObserveRequest,
  ObserveResponse,
} from "./types.mjs";

export const CLIENT_NAME = "claude-code";
export const CLIENT_VERSION = "0.1.0";

export type CallResult<T> =
  | { ok: true; value: T; degraded: string[] }
  | { ok: false; status?: number; reason: string };

export async function inject(
  config: AdapterConfig,
  bearer: string,
  request: InjectRequest,
): Promise<CallResult<InjectResponse>> {
  return call<InjectResponse>(config, bearer, "/v1/inject", request);
}

export async function observe(
  config: AdapterConfig,
  bearer: string,
  request: ObserveRequest,
): Promise<CallResult<ObserveResponse>> {
  return call<ObserveResponse>(config, bearer, "/v1/observe", request);
}

async function call<T>(
  config: AdapterConfig,
  bearer: string,
  path: string,
  body: unknown,
): Promise<CallResult<T>> {
  let status: number | undefined;
  try {
    const response = await fetch(`${config.gatewayUrl}${path}`, {
      method: "POST",
      signal: AbortSignal.timeout(config.timeoutMs),
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${bearer}`,
        "x-synveda-client": `${CLIENT_NAME}/${CLIENT_VERSION}`,
        traceparent: traceparent(),
      },
      body: JSON.stringify(body),
    });
    status = response.status;
    if (!response.ok) {
      return { ok: false, status, reason: `gateway returned ${String(status)}` };
    }
    // Read the header before the body: the degradation ladder is part of
    // a successful response, not an error (ADR-0026 decision 4).
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
 * through plan, embed, search, and compose (ADR-0027 compliance notes).
 */
function traceparent(): string {
  return `00-${randomBytes(16).toString("hex")}-${randomBytes(8).toString("hex")}-01`;
}

function reasonFor(error: unknown): string {
  if (error instanceof Error && error.name === "TimeoutError") return "deadline expired";
  return String(error);
}

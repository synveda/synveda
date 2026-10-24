/**
 * The signed-in sessions panel's requests, as plain functions.
 *
 * Kept apart from the component so the behaviour that matters (which
 * operation is called, how revoke-all is bounded, how a full revocation
 * ledger is told apart from an outage) is asserted without a browser.
 */

import type { Outcome } from "./api.mjs";
import { request } from "./client.mjs";
import type { ConsoleSessionPage, RevokeAllProgress } from "./generated/api.js";

/** The most revoke-all calls made before the panel stops and says so. */
export const REVOKE_ALL_CALL_LIMIT = 64;

/** Page size asked of the list. */
export const PAGE_LIMIT = 25;

export const LEDGER_FULL =
  "The revocation ledger is full, so nothing was revoked. Try again later or ask an administrator.";

/** Wraps a fetch so the HTTP status of the last answer is observable. */
function observing(fetchImpl: typeof fetch): { fetchImpl: typeof fetch; status: () => number } {
  let last = 0;
  const wrapped = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const response = await fetchImpl(input, init);
    last = response.status;
    return response;
  }) as typeof fetch;
  return { fetchImpl: wrapped, status: () => last };
}

export type Failed = Exclude<Outcome, { kind: "ok" }>;

export type RevokeResult =
  | { kind: "ok" }
  | { kind: "ledger-full" }
  | { kind: "failed"; outcome: Failed };

export async function listPage(
  after?: string,
  fetchImpl: typeof fetch = fetch,
): Promise<{ kind: "ok"; page: ConsoleSessionPage } | { kind: "failed"; outcome: Failed }> {
  const answer = await request(
    "list_my_console_sessions",
    { query: { limit: String(PAGE_LIMIT), after } },
    fetchImpl,
  );
  return answer.kind === "ok"
    ? { kind: "ok", page: answer.body }
    : { kind: "failed", outcome: answer };
}

export async function revokeOne(
  id: string,
  fetchImpl: typeof fetch = fetch,
): Promise<RevokeResult> {
  const seen = observing(fetchImpl);
  const answer = await request("revoke_my_console_session", { path: { id } }, seen.fetchImpl);
  if (answer.kind === "ok") return { kind: "ok" };
  if (seen.status() === 429) return { kind: "ledger-full" };
  return { kind: "failed", outcome: answer };
}

export type RevokeAllResult =
  | { kind: "done"; revoked: number }
  | { kind: "incomplete"; revoked: number }
  | { kind: "ledger-full"; revoked: number }
  | { kind: "failed"; revoked: number; outcome: Failed };

/** Repeats revoke-all until the gateway says it is complete, at most {@link REVOKE_ALL_CALL_LIMIT} times. */
export async function revokeAll(fetchImpl: typeof fetch = fetch): Promise<RevokeAllResult> {
  let revoked = 0;
  for (let call = 0; call < REVOKE_ALL_CALL_LIMIT; call += 1) {
    const seen = observing(fetchImpl);
    const answer = await request("revoke_all_my_console_sessions", {}, seen.fetchImpl);
    if (answer.kind !== "ok") {
      return seen.status() === 429
        ? { kind: "ledger-full", revoked }
        : { kind: "failed", revoked, outcome: answer };
    }
    const progress: RevokeAllProgress = answer.body;
    revoked += progress.revoked;
    if (progress.complete) return { kind: "done", revoked };
  }
  return { kind: "incomplete", revoked };
}

/**
 * The selected-target forecast hook and its one status line (`forecast.mts`).
 *
 * One probe per rendered set of unanswered scopes, through the generated
 * client and the shared cache: two components asking about the same set share
 * the request, and Refresh access re-asks it with `/v1/me`.
 */

import { request } from "./client.mjs";
import { forecastAt, forecastKey, unansweredScopes, type Forecast } from "./forecast.mjs";
import { useQuery, useRefresh } from "./Query.js";
import { useApp } from "./Shell.js";

export interface Forecasts {
  /** What is known about `action` at `scopeId`; no scope is unanswered. */
  at(scopeId: string | null | undefined, action: string): Forecast;
  /** Re-asks the probe — the bounded retry a failed forecast offers. */
  retry(): void;
}

/** Forecasts of `actions` at the scopes a page renders. */
export function useForecasts(
  scopeIds: readonly (string | null | undefined)[],
  actions: readonly string[],
): Forecasts {
  const { me } = useApp();
  const missing = unansweredScopes(me, scopeIds, actions);
  const key = forecastKey(me, missing);
  const probe = useQuery(key, () =>
    missing.length === 0
      ? // Every rendered scope is an anchor `/v1/me` already answered.
        Promise.resolve({ kind: "ok" as const, body: { capabilities: [] } })
      : request("get_capabilities", { query: { scopes: missing.join(",") } }),
  );
  const retry = useRefresh(key);
  return {
    at: (scopeId, action) => (scopeId ? forecastAt(me, probe, scopeId, action) : "unanswered"),
    retry,
  };
}

/**
 * Says why a control is not offered when the reason is not a denial: still
 * checking, a probe that failed, or a scope the gateway did not answer for.
 * A settled answer renders nothing here; the page words its own denial.
 */
export function ForecastNote({
  state,
  what,
  onRetry,
}: {
  state: Forecast;
  what: string;
  onRetry: () => void;
}) {
  switch (state) {
    case "allowed":
    case "denied":
      return null;
    case "loading":
      return <p className="muted">Checking whether you can {what} here…</p>;
    case "failed":
    case "unanswered":
      return (
        <div className="banner warning" role="status">
          {state === "failed"
            ? `Your access could not be checked, so ${what} is not offered yet.`
            : `Synveda did not answer whether you can ${what} here, so it is not offered.`}{" "}
          The gateway still decides every request.{" "}
          <button type="button" onClick={onRetry}>
            Check again
          </button>
        </div>
      );
  }
}

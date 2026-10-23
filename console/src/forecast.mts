/**
 * Selected-target capability forecasts (ADR-0058, ADR-0073).
 *
 * `/v1/me` anchors are the scopes where this caller's grants are written,
 * their own scope and the tenant root. They are not an inventory of every
 * descendant: a workspace member's project has no anchor of its own, and the
 * response may say it dropped anchors past its bound. So a scope missing
 * from the anchors is **unanswered**, never a denial. The console asks
 * `GET /v1/capabilities` about exactly the unanswered scopes it renders,
 * which decides each one under that scope's own effective policy — a
 * restrictive child policy answers no even where its parent says yes.
 *
 * Both are forecasts, never grants: they choose what to offer, and every act
 * still meets the gateway's decision at its own seam.
 */

import type { Entry } from "./cache.mjs";
import type { MeView } from "./generated/api.js";

/** One action at one scope, as far as the console knows. */
export type Forecast = "loading" | "allowed" | "denied" | "unanswered" | "failed";

/** The batch probe's answer (`capabilities::BatchResponse`), as read here. */
interface ProbeBody {
  capabilities?: {
    scope_id?: unknown;
    actions?: Record<string, unknown>;
    read_tiers?: Record<string, unknown>;
  }[];
  not_answered?: unknown[];
}

/**
 * The rendered scopes `/v1/me` did not answer every asked action for,
 * deduplicated and ordered. An anchor carries the operand-free scope actions
 * only; a tiered read such as `knowledge.read` is always probed.
 */
export function unansweredScopes(
  me: MeView,
  scopeIds: readonly (string | null | undefined)[],
  actions: readonly string[],
): string[] {
  const wanted = new Set<string>();
  for (const id of scopeIds) {
    if (!id) continue;
    const anchor = me.anchors.find((candidate) => candidate.scope_id === id);
    if (!anchor || actions.some((action) => !(action in anchor.actions))) wanted.add(id);
  }
  return [...wanted].sort();
}

/**
 * The cache key for one probe. It names the identity and tenant it was asked
 * for, so an answer about one account can never be read back for another.
 */
export function forecastKey(me: MeView, scopeIds: readonly string[]): string {
  return `capabilities/${me.tenant.id}/${me.principal.subject}/${scopeIds.join(",")}`;
}

/** What is known about `action` at `scopeId`. */
export function forecastAt(
  me: MeView,
  probe: Entry,
  scopeId: string,
  action: string,
): Forecast {
  const anchor = me.anchors.find((candidate) => candidate.scope_id === scopeId);
  if (anchor && action in anchor.actions) return verdict(anchor.actions[action]);
  if (probe.status === "loading") return "loading";
  if (probe.outcome.kind !== "ok") return "failed";
  const body = probe.outcome.body as ProbeBody | null;
  if (body?.not_answered?.includes(scopeId)) return "unanswered";
  const answer = body?.capabilities?.find((candidate) => candidate.scope_id === scopeId);
  if (!answer) return "unanswered";
  if (answer.actions && action in answer.actions) return verdict(answer.actions[action]);
  // A tiered read answers with the tiers it permits; any tier is an offer.
  const tiers = answer.read_tiers?.[action];
  return Array.isArray(tiers) ? (tiers.length > 0 ? "allowed" : "denied") : "unanswered";
}

function verdict(value: unknown): Forecast {
  if (value === true) return "allowed";
  if (value === false) return "denied";
  return "unanswered";
}

/**
 * Several forecasts as one status line: any failure, then anything
 * unanswered or still loading, wins over a settled answer; settled answers
 * read as allowed when any one of them is.
 */
export function summary(forecasts: readonly Forecast[]): Forecast {
  for (const state of ["failed", "unanswered", "loading", "allowed"] as const) {
    if (forecasts.includes(state)) return state;
  }
  return "denied";
}

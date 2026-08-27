/**
 * The explorer's types and the judgements it makes (CNSL-2, ADR-0058).
 *
 * `review.mts`'s shape: the wire types plus the pure functions worth
 * testing, kept out of the components so a test can reach them without
 * rendering anything.
 *
 * # Why the origin sentence is here rather than at the gateway
 *
 * ADR-0056 decision 5 moved *verdicts* to the gateway — "does this block?"
 * is a rank comparison against a rule table and a pack, and two clients
 * computing it produce two answers that agree only on the day they are
 * written. An origin is not that. The gateway serves `{kind, scope_id}`,
 * which is the whole fact; whether it reads "here" is a comparison against
 * the node the reader asked about, which the client already knows and which
 * has no rule table in it. And the two surfaces genuinely want different
 * words: a terminal prints an id, a browser can show the ancestor's own
 * name.
 *
 * The safety net is ADR-0058 decision 10 rather than a shared
 * implementation — the parity corpus asserts both surfaces name the same
 * origin for the same payload, so a divergence fails a test rather than
 * waiting to be noticed.
 */

import type {
  BatchResponse,
  ListResponse,
  NodeCapabilities,
  RelaxationListView,
  RelaxationView,
  ScopeView,
} from "./generated/api.js";

/** A governed scope, as `/v1/admin/scopes` serves it (CPR-7). */
export type Node = ScopeView;

/** One level of the tree: the parent it hangs from, and its children. */
export type ScopeLevel = ListResponse;

export type Capabilities = NodeCapabilities;

/** The batch probe's envelope. */
export type CapabilityBatch = BatchResponse;

export type Relaxation = RelaxationView;

export type RelaxationListing = RelaxationListView;

/**
 * The actions a capability answer says yes to, sorted.
 *
 * Named `mayDo` rather than `allowed` deliberately: this is what the reader
 * may *attempt*, and the gateway decides again when they do.
 */
export function mayDo(capabilities: Capabilities): string[] {
  return Object.entries(capabilities.actions)
    .filter(([, permitted]) => permitted)
    .map(([action]) => action)
    .sort();
}

/** How many probed actions came back denied — the other half of the pair count. */
export function deniedCount(capabilities: Capabilities): number {
  return Object.values(capabilities.actions).filter((permitted) => !permitted).length;
}

/** The tiered reads that permit anything here, as `action → tiers`. */
export function mayRead(capabilities: Capabilities): [string, string[]][] {
  return Object.entries(capabilities.read_tiers)
    .filter(([, tiers]) => tiers.length > 0)
    .sort(([a], [b]) => a.localeCompare(b));
}

/** The roles this reader may bind here, sorted. */
/**
 * Whether a capability answer offers an action.
 *
 * The one function the inbox uses, and the one place to look when asking
 * whether this bundle ever gates on a probe. It does not: the caller uses
 * this to choose what to *render*, and every act it renders is refused by
 * the gateway if the forecast was wrong (ADR-0058 decision 2).
 */
export function offers(capabilities: Capabilities | null, action: string): boolean {
  return capabilities?.actions[action] === true;
}

/**
 * Relaxations governed at this exact scope.
 *
 * A CPR-31 relaxation freezes one provisioned identity as its subject and
 * one non-principal scope as its target. There is no inherited grantee end
 * and therefore no client-side reconstruction of one.
 */
export function relaxationsAt(relaxations: Relaxation[], scopeId: string): Relaxation[] {
  return relaxations.filter((relaxation) => relaxation.governing_scope_id === scopeId);
}

/**
 * What the Sessions page renders, as pure functions (CPR-10, ADR-0076).
 *
 * The rendering lives in `Sessions.tsx`; everything that has a *right
 * answer* lives here, so it can be asserted without a browser. That is the
 * same split `people.mts` makes, and for the same reason: there is still no
 * browser test runner, so what a page decides must be decidable outside it.
 */

import type { SessionView, TimelineEntry } from "./generated/api.js";

/**
 * How a run's state should read to somebody who did not open it.
 *
 * Five states, five sentences, and the two that are easy to conflate are
 * kept apart deliberately: `abandoned` is a run **nobody closed** — a killed
 * client, a closed laptop, a headless run that exited — and `failed` is a run
 * that **broke**. A console that showed both as "did not finish" would be
 * hiding the difference between an infrastructure problem and a lost laptop.
 */
export function statusLabel(status: SessionView["status"]): string {
  switch (status) {
    case "active":
      return "running";
    case "ending":
      return "finishing up";
    case "ended":
      return "finished";
    case "abandoned":
      return "never closed";
    case "failed":
      return "failed";
  }
}

/** Which of the three visual tones a state gets. */
export function statusTone(status: SessionView["status"]): "live" | "done" | "warn" {
  switch (status) {
    case "active":
    case "ending":
      return "live";
    case "ended":
      return "done";
    case "abandoned":
    case "failed":
      return "warn";
  }
}

/**
 * The one line under a run's title: who ran it, with what, and where.
 *
 * The client is always named. The model and the agent are named only when
 * the client said them — an em-dash placeholder would be three characters
 * telling a reader nothing, where their absence tells them the client did
 * not report it.
 */
export function runDescription(session: SessionView): string {
  const parts = [session.client_name];
  if (session.client_version) parts.push(`v${session.client_version}`);
  if (session.model_name) parts.push(session.model_name);
  if (session.agent_name) parts.push(session.agent_name);
  if (session.branch) parts.push(`on ${session.branch}`);
  return parts.join(" · ");
}

/**
 * A run's title: what the client said it was about, or an honest fallback.
 *
 * The fallback names the client and the time rather than the id, because an
 * id is the one thing a reader cannot recognise and the pair is the one
 * thing they can.
 */
export function runTitle(session: SessionView): string {
  return session.task_summary ?? `${session.client_name} run`;
}

/**
 * How long a run took, or how long it has been going.
 *
 * `null` when the timestamps cannot be read: a duration nobody can compute
 * is not rendered as `0m`, which would be a measurement.
 */
export function durationOf(session: SessionView, now: number): string | null {
  const started = Date.parse(session.started_at);
  const finished = session.ended_at ? Date.parse(session.ended_at) : now;
  if (Number.isNaN(started) || Number.isNaN(finished) || finished < started) {
    return null;
  }
  const seconds = Math.round((finished - started) / 1000);
  if (seconds < 60) return `${seconds}s`;
  // Floored, not rounded: a run that has been going five and a half minutes
  // reads as `5m`, because overstating how long an agent has been running is
  // the direction that makes somebody go looking for a problem.
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

/**
 * The event-count summary a timeline shows above its entries: the busiest
 * families first, then alphabetically.
 *
 * Sorted by count because the shape of a run is what an auditor reads first,
 * and a fixed vocabulary order would put `adapter.warning` above
 * `message.user` in every run that had one of each.
 */
export function countSummary(counts: Record<string, number>): { type: string; count: number }[] {
  return Object.entries(counts)
    .map(([type, count]) => ({ type, count }))
    .sort((a, b) => b.count - a.count || a.type.localeCompare(b.type));
}

/**
 * Whether a timeline entry is a context run rather than an event.
 *
 * A named predicate rather than an inline comparison, because the two kinds
 * render differently in three places and a typo in one of them would show a
 * composed block as a message.
 */
export function isContextRun(entry: TimelineEntry): boolean {
  return entry.kind === "context_run";
}

/**
 * The sentence an empty listing shows.
 *
 * It must not read as "nothing has happened". A caller sees the runs their
 * grants reach, so an empty list at a project they hold nothing at is a
 * different fact from a project nobody has run an agent in — and the two are
 * indistinguishable from the client's side, which is exactly why the page
 * says both rather than picking one.
 */
export const EMPTY_SENTENCE =
  "No runs you can read here yet. A run you are not granted at is omitted rather than " +
  "refused, so this is not the same as “nobody has run an agent”.";

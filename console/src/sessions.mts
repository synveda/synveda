/**
 * What the Sessions pages decide, as pure functions (CPR-10, ADR-0076;
 * CPR-11, ADR-0077).
 *
 * The rendering lives in `Sessions.tsx` and `Session.tsx`; everything that
 * has a *right answer* lives here, so it can be asserted without a browser.
 * That is the same split `people.mts` makes, and for the same reason: there
 * is still no browser test runner, so what a page decides must be decidable
 * outside it.
 *
 * # The two clocks
 *
 * A timeline entry carries `at` — what the client says happened when — and,
 * for an event, `received_at`, which is when this deployment was told. They
 * are different facts and the console shows both, because the only reader who
 * can act on a delayed transcript is one who can see that it was delayed.
 * The server decides *whether* the gap is a delay ({@link isLate}); this file
 * decides how to say so.
 */

import type { RepositoryView, SessionView, TimelineEntry } from "./generated/api.js";

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
 * Whether a run stopped without a clean finish, or has not finished at all.
 *
 * The requirement this exists for is that a reader can tell an **incomplete**
 * run from a complete one at a glance. `ending` is incomplete because the
 * adapter said it was flushing and never said it had; `abandoned` and
 * `failed` are the two ways a run stops without finishing. `active` is not
 * incomplete — it is running, which is a different thing and reads as one.
 */
export function isIncomplete(session: SessionView): boolean {
  return (
    session.status === "ending" ||
    session.status === "abandoned" ||
    session.status === "failed"
  );
}

/**
 * The sentence under an incomplete run's heading, or `null` for one that
 * finished or is still going.
 *
 * It names the **end reason** when the client gave one, because that is the
 * whole point of the field: `failed` says a run broke and `end_reason` says
 * what broke. When there is none the sentence says so rather than inventing a
 * cause — "no reason recorded" is a fact about this deployment's clients, and
 * a reader who sees it knows to go and look at the adapter rather than at the
 * run.
 */
export function endLine(session: SessionView): string | null {
  if (!isIncomplete(session)) return null;
  const reason = session.end_reason?.trim();
  switch (session.status) {
    case "ending":
      return reason
        ? `Still finishing: ${reason}`
        : "Still finishing. The client said it was flushing buffered events and never said it had.";
    case "abandoned":
      return reason
        ? `Nobody closed this run: ${reason}`
        : "Nobody closed this run. No reason was recorded — a killed client, a closed laptop, or a headless run that exited.";
    case "failed":
      return reason ? `This run failed: ${reason}` : "This run failed. No reason was recorded.";
    default:
      return null;
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
 * What a run was working on: the repository's canonical URI and the branch.
 *
 * The repository comes from the project's own attachment list rather than
 * from the session, because a session carries the attachment's **id** and an
 * id is not something a reader recognises. A run whose repository is no
 * longer attached says so instead of showing a uuid: the run really did
 * happen against something, and "detached" is the true account of it.
 */
export function repositoryLine(
  session: SessionView,
  repositories: RepositoryView[],
): string | null {
  if (!session.repository_id) {
    return session.branch ? `branch ${session.branch}` : null;
  }
  const found = repositories.find((repository) => repository.id === session.repository_id);
  const name = found ? found.canonical_uri : "a repository since detached from this project";
  return session.branch ? `${name} · branch ${session.branch}` : name;
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
 * Whether an entry is the adapter telling on itself.
 *
 * `adapter.warning` is how a client reports that it could not do something —
 * a delivery that failed, a hook that timed out, a spool it had to write. It
 * is the one event family a reader must not have to go looking for, so it is
 * counted, banner-ed and marked in place rather than being one grey line
 * among two hundred.
 */
export function isWarning(entry: TimelineEntry): boolean {
  return entry.event_type === "adapter.warning";
}

/** How many warnings a run recorded, from the timeline's own counts. */
export function warningCount(counts: Record<string, number>): number {
  return counts["adapter.warning"] ?? 0;
}

/**
 * Whether an entry did not arrive live.
 *
 * The **server** decides this — `TimelineEntry.delayed` is computed from the
 * two instants against one threshold, so the console, the CLI and anything
 * else that reads a timeline agree about what "late" means. This is a named
 * reader for it, so that the pages never grow their own arithmetic over the
 * two timestamps and quietly disagree with the API.
 */
export function isLate(entry: TimelineEntry): boolean {
  return entry.delayed;
}

/**
 * How an entry's delivery should read: nothing at all, or how far behind it
 * was.
 *
 * Three cases and only one of them prints. A live entry says nothing, because
 * a badge on every one of two hundred rows is a badge nobody reads. A delayed
 * one says how long the gap was, because "recovered from a local spool" and
 * "a machine whose clock is an hour out" produce the same two instants and
 * this console will not pretend to tell them apart — it reports the gap and
 * lets the reader judge.
 */
export function deliveryNote(entry: TimelineEntry): string | null {
  if (!entry.delayed || !entry.received_at) return null;
  const happened = Date.parse(entry.at);
  const arrived = Date.parse(entry.received_at);
  if (Number.isNaN(happened) || Number.isNaN(arrived)) {
    return "recovered or delayed";
  }
  return `recovered or delayed — reached this deployment ${gapOf(arrived - happened)} later`;
}

/** A rounded, human gap. Floored for the same reason `durationOf` floors. */
export function gapOf(millis: number): string {
  const seconds = Math.max(0, Math.round(millis / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** The states the listing offers as a filter, in lifecycle order. */
export const STATUS_FILTERS = ["active", "ending", "ended", "abandoned", "failed"] as const;

export type StatusFilter = (typeof STATUS_FILTERS)[number];

/** What the listing is currently narrowed to. Every field is optional. */
export interface Filters {
  status: StatusFilter | null;
  projectId: string | null;
  clientName: string;
  principalId: string;
  /** `YYYY-MM-DD`, as an `<input type="date">` gives it. */
  from: string;
  to: string;
}

/** Nothing selected. The state the page opens in. */
export const NO_FILTERS: Filters = {
  status: null,
  projectId: null,
  clientName: "",
  principalId: "",
  from: "",
  to: "",
};

/**
 * The query the listing sends for these filters, one page at a time.
 *
 * `undefined` rather than `""` for anything nobody set: the typed client
 * drops an undefined parameter entirely, and a filter set to nothing is a
 * different request from a filter nobody set (`client.mts`).
 *
 * The two dates become instants here rather than at the gateway, because a
 * date picker means *a day in the reader's calendar* and the API takes an
 * instant. `to` becomes the start of the following day, so the range a reader
 * described as "the 3rd to the 4th" includes everything that happened on the
 * 4th — the half-open bound the API documents, applied where the off-by-one
 * would otherwise be the reader's to discover.
 */
export function listQuery(
  filters: Filters,
  scopeId: string | null,
  cursor: string | null,
): Record<string, string | undefined> {
  return {
    scope_id: scopeId ?? undefined,
    project_id: filters.projectId ?? undefined,
    status: filters.status ?? undefined,
    client_name: filters.clientName.trim() || undefined,
    principal_id: filters.principalId.trim() || undefined,
    started_after: dayStart(filters.from),
    started_before: dayAfter(filters.to),
    cursor: cursor ?? undefined,
  };
}

/** Midnight UTC on a `YYYY-MM-DD`, or `undefined` for a blank or bad one. */
export function dayStart(day: string): string | undefined {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) return undefined;
  const parsed = Date.parse(`${day}T00:00:00Z`);
  return Number.isNaN(parsed) ? undefined : new Date(parsed).toISOString();
}

/** Midnight UTC on the day *after* one, which is the half-open upper bound. */
export function dayAfter(day: string): string | undefined {
  const start = dayStart(day);
  if (start === undefined) return undefined;
  return new Date(Date.parse(start) + 24 * 60 * 60 * 1000).toISOString();
}

/**
 * Whether the reader has narrowed anything.
 *
 * What it is for: an empty list under a filter is a different fact from an
 * empty list without one, and the page says which (`EMPTY_SENTENCE` against
 * `EMPTY_FILTERED_SENTENCE`).
 */
export function isFiltered(filters: Filters): boolean {
  return (
    filters.status !== null ||
    filters.projectId !== null ||
    filters.clientName.trim().length > 0 ||
    filters.principalId.trim().length > 0 ||
    filters.from.length > 0 ||
    filters.to.length > 0
  );
}

/**
 * Appends a page to what is already on screen, without repeating a row.
 *
 * The de-duplication is not defensive tidiness. Rows are keyed by
 * `(started_at, id)` and a run opened between two requests shifts nothing —
 * but a reader who clicks "Load more" twice before the first answer lands
 * sends the same cursor twice, and the second answer is the same page. One
 * `Map` keyed by id, insertion-ordered, is the whole fix.
 */
export function appendPage(seen: SessionView[], page: SessionView[]): SessionView[] {
  const byId = new Map(seen.map((session) => [session.id, session]));
  for (const session of page) {
    if (!byId.has(session.id)) byId.set(session.id, session);
  }
  return [...byId.values()];
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

/** The same, when the reader narrowed it themselves. */
export const EMPTY_FILTERED_SENTENCE =
  "No runs match these filters. Clear them to see everything you can read here.";

/**
 * One run (CPR-11, ADR-0077): what it was, how it ended, and everything that
 * happened in it.
 *
 * Its own route rather than an expander inside the listing, because this is
 * the page somebody opens when a run went wrong: it has to survive a refresh,
 * be pasteable into a ticket, and still be there after a Back. The id comes
 * from the URL and from nowhere else.
 *
 * # Three reads, three cache keys
 *
 * The run, its timeline and — when the run names a project — that project's
 * repositories. Separate keys because they change on different clocks and are
 * different sizes: a transcript is the largest thing on this plane, and the
 * repository list exists only to turn an attachment id into something a
 * reader recognises.
 *
 * # What this page will not show you by default
 *
 * Event **payloads**. A timeline says a message was sent and summarises it;
 * the payload is what the person and the agent actually said, byte for byte.
 * Those are different disclosures, so the API prices them differently
 * (`session.diagnostics` against `session.read`) and this page offers the
 * expansion only where the caller's forecast says the plane is theirs — and
 * even then the gateway decides again on the click. A forecast is what to
 * offer; it is never what to allow (ADR-0058 decision 2).
 */

import { useState } from "react";

import { request } from "./client.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import {
  countSummary,
  deliveryNote,
  durationOf,
  endLine,
  isContextRun,
  isWarning,
  repositoryLine,
  runDescription,
  runTitle,
  statusLabel,
  statusTone,
  warningCount,
} from "./sessions.mjs";
import type {
  MeView,
  RepositoryList,
  SessionEventView,
  SessionView,
  TimelineEntry,
  TimelineView,
} from "./generated/api.js";

/** The action a payload expansion is priced at, server-side. */
const DIAGNOSTICS = "session.diagnostics";

export function Session({ sessionId }: { sessionId: string }) {
  const { me } = useApp();
  const cacheKey = `sessions/one/${sessionId}`;
  const entry = useQuery(cacheKey, () =>
    request("get_session", { path: { session_id: sessionId } }),
  );
  const retry = useRefresh(cacheKey);

  return (
    <>
      <PageHeading route="session" />
      <p>
        <Link href={hrefOf("sessions")}>← All sessions</Link>
      </p>
      <Loaded<SessionView> entry={entry} what="this run" onRetry={retry}>
        {(session) => (
          <>
            <Summary session={session} />
            <Timeline session={session} diagnostics={offersDiagnostics(me, session)} />
          </>
        )}
      </Loaded>
    </>
  );
}

/**
 * Whether to offer the payload expansion, from the caller's own forecast at
 * **this run's scope** when `/v1/me` reported one, and from the tenant-wide
 * forecast otherwise.
 *
 * The per-anchor answer is preferred because it is the one about this run: a
 * caller may hold the plane in one project and not in another, and the
 * tenant-wide figure would offer a control that 403s in half the places it
 * appears.
 */
export function offersDiagnostics(me: MeView, session: SessionView): boolean {
  const anchor = me.anchors.find((candidate) => candidate.scope_id === session.scope_id);
  const actions = anchor ? anchor.actions : me.capabilities.actions;
  return actions[DIAGNOSTICS] === true;
}

/** The header block: what this run was, who ran it, and how it ended. */
function Summary({ session }: { session: SessionView }) {
  const duration = durationOf(session, Date.now());
  const ended = endLine(session);
  return (
    <section className="run-summary">
      <h2>
        {runTitle(session)}{" "}
        <span className={`tag ${statusTone(session.status)}`}>{statusLabel(session.status)}</span>
      </h2>
      {ended ? (
        <div className="banner warn" role="status">
          {ended}
        </div>
      ) : null}
      <dl className="facts">
        <dt>Client</dt>
        <dd>{runDescription(session)}</dd>
        <dt>Opened by</dt>
        <dd>{session.principal_id}</dd>
        <dt>Started</dt>
        <dd>
          {whenOf(session.started_at)}
          {duration === null ? null : ` · ran for ${duration}`}
        </dd>
        <dt>Ended</dt>
        <dd>{session.ended_at ? whenOf(session.ended_at) : "not yet"}</dd>
        {session.last_observed_at ? (
          <>
            <dt>Last activity</dt>
            <dd>{whenOf(session.last_observed_at)}</dd>
          </>
        ) : null}
        {session.project_id ? <Repository session={session} /> : null}
      </dl>
    </section>
  );
}

/**
 * The repository row, resolved through the project's own attachment list.
 *
 * Its own component so the extra read only happens for a run that names a
 * project, and so a repository list this caller may not read degrades to the
 * branch alone rather than failing the whole page.
 */
function Repository({ session }: { session: SessionView }) {
  const projectId = session.project_id as string;
  const cacheKey = `projects/${projectId}/repositories`;
  const entry = useQuery(cacheKey, () =>
    request("list_repositories", { path: { project_id: projectId } }),
  );
  // A repository list this caller may not read, or one that has not landed
  // yet, degrades to the branch alone rather than failing the page: what a
  // run was working on is context, and context is not worth a blank screen.
  const repositories =
    entry.status === "ready" && entry.outcome.kind === "ok"
      ? (entry.outcome.body as RepositoryList).repositories
      : [];
  const line = repositoryLine(session, repositories);
  if (!line) return null;
  return (
    <>
      <dt>Working on</dt>
      <dd>{line}</dd>
    </>
  );
}

/**
 * One run's timeline — the projection over its events and its context runs.
 *
 * Its own cache key, so opening a run is one request for that run rather than
 * a listing that pre-fetched every transcript in the project.
 */
function Timeline({ session, diagnostics }: { session: SessionView; diagnostics: boolean }) {
  const cacheKey = `sessions/timeline/${session.id}`;
  const entry = useQuery(cacheKey, () =>
    request("get_session_timeline", { path: { session_id: session.id } }),
  );
  const retry = useRefresh(cacheKey);

  return (
    <Loaded<TimelineView> entry={entry} what="the timeline" onRetry={retry}>
      {(body) => {
        const warnings = warningCount(body.event_counts);
        return (
          <section className="timeline">
            <h2>Timeline</h2>
            {warnings > 0 ? (
              <div className="banner warn" role="status">
                {warnings === 1
                  ? "This run recorded 1 adapter warning — the client telling you it could not do something."
                  : `This run recorded ${warnings} adapter warnings — the client telling you it could not do something.`}
              </div>
            ) : null}
            <p className="muted">
              {countSummary(body.event_counts)
                .map(({ type, count }) => `${count} ${type}`)
                .join(" · ") || "no events recorded"}
            </p>
            {body.truncated ? (
              <p className="muted">Showing the first entries of a longer run.</p>
            ) : null}
            {diagnostics ? null : (
              // Said once, above the entries, rather than on each of two
              // hundred rows. A reader who cannot expand a payload needs to
              // know which role it takes; they do not need to be told it per
              // line.
              <p className="muted diagnostic-closed">
                Raw event payloads are not shown. Expanding one needs {DIAGNOSTICS} at this
                run&rsquo;s scope.
              </p>
            )}
            <ol className="entries">
              {body.entries.map((item) => (
                <Entry
                  key={`${item.kind}-${item.id}`}
                  entry={item}
                  sessionId={session.id}
                  diagnostics={diagnostics}
                />
              ))}
            </ol>
          </section>
        );
      }}
    </Loaded>
  );
}

/**
 * One timeline entry.
 *
 * Both instants are shown and they are labelled, because the reader this page
 * is for is one asking "did that actually happen when it says?" — and a
 * delayed entry is marked in place rather than only counted at the top, since
 * a transcript with one recovered hour in the middle of it reads perfectly
 * plausibly until somebody notices.
 */
function Entry({
  entry,
  sessionId,
  diagnostics,
}: {
  entry: TimelineEntry;
  sessionId: string;
  diagnostics: boolean;
}) {
  const classes = ["entry"];
  if (isContextRun(entry)) classes.push("context");
  if (isWarning(entry)) classes.push("warning");
  if (entry.delayed) classes.push("late");
  const note = deliveryNote(entry);

  return (
    <li className={classes.join(" ")}>
      <div>
        <span className="muted">occurred {whenOf(entry.at)}</span>{" "}
        {entry.received_at ? (
          <span className="muted">· received {whenOf(entry.received_at)}</span>
        ) : null}{" "}
        {isWarning(entry) ? <span className="tag warn">warning</span> : null}
        {isContextRun(entry) ? <span className="tag done">context</span> : null}
      </div>
      <div>
        {isContextRun(entry) ? (
          <Link href={hrefOf("context-run", { context_run_id: entry.id })}>{entry.summary}</Link>
        ) : (
          entry.summary
        )}
      </div>
      {note ? <div className="late-note">{note}</div> : null}
      {diagnostics && !isContextRun(entry) ? (
        <Payload eventId={entry.id} sessionId={sessionId} />
      ) : null}
    </li>
  );
}

/**
 * The diagnostic expansion for one event.
 *
 * Closed until asked for, and the request only happens on the first open —
 * an eagerly-fetched payload would be exactly the disclosure this action
 * exists to gate, taken on everybody's behalf without anybody asking.
 *
 * Rendered only where the forecast offers the plane; the sentence explaining
 * what it takes is said once by {@link Timeline}, not per row.
 */
function Payload({ eventId, sessionId }: { eventId: string; sessionId: string }) {
  const [open, setOpen] = useState(false);
  if (!open) {
    return (
      <p>
        <button type="button" className="link" onClick={() => setOpen(true)}>
          Show raw payload
        </button>
      </p>
    );
  }
  return <PayloadBody eventId={eventId} sessionId={sessionId} />;
}

/** The read itself, mounted only once somebody has asked for it. */
function PayloadBody({ eventId, sessionId }: { eventId: string; sessionId: string }) {
  const cacheKey = `sessions/event/${sessionId}/${eventId}`;
  const entry = useQuery(cacheKey, () =>
    request("get_session_event", { path: { session_id: sessionId, event_id: eventId } }),
  );
  const retry = useRefresh(cacheKey);
  return (
    <Loaded<SessionEventView> entry={entry} what="the payload" onRetry={retry}>
      {(event) => (
        <div className="diagnostic">
          <p className="muted">
            {event.client_event_id} · sequence {event.sequence} · schema v
            {event.event_schema_version} · blake3 {event.payload_hash.slice(0, 16)}…
          </p>
          <pre>{JSON.stringify(event.payload, null, 2)}</pre>
        </div>
      )}
    </Loaded>
  );
}

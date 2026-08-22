/**
 * Sessions (CPR-10, ADR-0076): every run of an agent, and what one did.
 *
 * The first of CPR-8's four planned pages to get a plane behind it. It reads
 * `GET /v1/sessions` at the selected project's scope — or the workspace's
 * when no project is chosen — and, when a run is opened, projects its
 * timeline through `GET /v1/sessions/{id}/timeline`.
 *
 * Both calls go through the **generated client**: sessions are on the
 * OpenAPI contract from the day the routes exist, so nothing here is a
 * hand-written path. That is the first plane in this programme for which
 * that is true from the start rather than from Prompt 19.
 *
 * # An empty list is not "nothing happened"
 *
 * ADR-0075 decision 7's rule, one plane later and now with real data behind
 * it: a caller sees the runs their grants reach, so an empty listing at a
 * project they hold nothing at is a different fact from a project nobody has
 * run an agent in — and from the client's side the two are indistinguishable.
 * The page says both rather than picking one (`sessions.mts`'s
 * `EMPTY_SENTENCE`).
 *
 * # It reads and does not write
 *
 * There is deliberately no "start a session" button. A session is opened by
 * an agent, from a harness, with a `client_name` only that harness knows —
 * a browser opening one would create a run that never ran.
 */

import { useState } from "react";

import { request } from "./client.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import {
  EMPTY_SENTENCE,
  countSummary,
  durationOf,
  isContextRun,
  runDescription,
  runTitle,
  statusLabel,
  statusTone,
} from "./sessions.mjs";
import type { SessionList, TimelineView } from "./generated/api.js";

export function Sessions() {
  const { project, workspace } = useApp();
  // The project's scope when there is one, else the workspace's — the
  // nearest scope the reader has selected is the one they mean, and the
  // gateway lists the subtree, so a workspace shows its projects' runs.
  const scopeId = project?.scope_id ?? workspace?.scope_id ?? null;
  const cacheKey = `sessions/${scopeId ?? "all"}`;
  const entry = useQuery(cacheKey, () =>
    request("list_sessions", { query: { scope_id: scopeId ?? undefined } }),
  );
  const retry = useRefresh(cacheKey);
  const [opened, setOpened] = useState<string | null>(null);

  return (
    <>
      <PageHeading route="sessions" />
      <Loaded<SessionList> entry={entry} what="the sessions" onRetry={retry}>
        {(body) => (
          <>
            {body.sessions.length === 0 ? (
              <p className="muted">{EMPTY_SENTENCE}</p>
            ) : (
              <>
                {body.truncated ? (
                  <p className="muted">
                    Showing the most recent runs. There are more than this answer carries.
                  </p>
                ) : null}
                <ul className="sessions">
                  {body.sessions.map((session) => (
                    <li key={session.id}>
                      <button
                        type="button"
                        className="row"
                        aria-expanded={opened === session.id}
                        onClick={() => setOpened(opened === session.id ? null : session.id)}
                      >
                        <strong>{runTitle(session)}</strong>{" "}
                        <span className={`tag ${statusTone(session.status)}`}>
                          {statusLabel(session.status)}
                        </span>
                        <div className="muted">
                          {runDescription(session)} · started {whenOf(session.started_at)}
                          {durationOf(session, Date.now()) === null
                            ? null
                            : ` · ${durationOf(session, Date.now())}`}
                        </div>
                      </button>
                      {opened === session.id ? <Timeline sessionId={session.id} /> : null}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </>
        )}
      </Loaded>
    </>
  );
}

/**
 * One run's timeline — the projection over its events and its context runs.
 *
 * Its own component with its own cache key, so opening a run is one request
 * for that run rather than a listing that pre-fetched every transcript in the
 * project. A transcript is the largest thing on this plane and the one a
 * reader asks for least often.
 */
function Timeline({ sessionId }: { sessionId: string }) {
  const cacheKey = `sessions/timeline/${sessionId}`;
  const entry = useQuery(cacheKey, () =>
    request("get_session_timeline", { path: { session_id: sessionId } }),
  );
  const retry = useRefresh(cacheKey);

  return (
    <Loaded<TimelineView> entry={entry} what="the timeline" onRetry={retry}>
      {(body) => (
        <div className="timeline">
          <p className="muted">
            {countSummary(body.event_counts)
              .map(({ type, count }) => `${count} ${type}`)
              .join(" · ") || "no events recorded"}
          </p>
          {body.truncated ? (
            <p className="muted">Showing the first entries of a longer run.</p>
          ) : null}
          <ol>
            {body.entries.map((item) => (
              <li key={`${item.kind}-${item.id}`} className={isContextRun(item) ? "context" : ""}>
                <span className="muted">{whenOf(item.at)}</span> {item.summary}
              </li>
            ))}
          </ol>
        </div>
      )}
    </Loaded>
  );
}

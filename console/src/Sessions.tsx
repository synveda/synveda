/**
 * Sessions (CPR-10, ADR-0076; CPR-11, ADR-0077): every run of an agent, and
 * a way to find the one you are looking for.
 *
 * CPR-10 gave this page a plane and a single call: the newest fifty runs at
 * the selected scope, expandable in place. This is the product surface over
 * it — filters, pages, and a route per run — and the difference is what a
 * reader can *do*. A run from last Tuesday was unreachable before: the
 * listing said `truncated` and had no way to say where to continue.
 *
 * Every call goes through the **generated client**: sessions are on the
 * OpenAPI contract from the day the routes exist, so nothing here is a
 * hand-written path or a hand-written DTO.
 *
 * # A page is a cache key
 *
 * The filters and the cursor are both part of the identity of a read
 * (`cache.mts` rule 1), so the key carries them. Changing a filter is
 * therefore a different question with a different answer rather than the same
 * key holding two, and pressing Back over a filter change re-reads nothing.
 *
 * # An empty list is not "nothing happened"
 *
 * ADR-0075 decision 7's rule with real data behind it: a caller sees the runs
 * their grants reach, so an empty listing at a project they hold nothing at is
 * a different fact from a project nobody has run an agent in — and from the
 * client's side the two are indistinguishable. The page says both rather than
 * picking one, and says something different again when the reader narrowed it
 * themselves.
 *
 * # It reads and does not write
 *
 * There is deliberately no "start a session" button. A session is opened by
 * an agent, from a harness, with a `client_name` only that harness knows —
 * a browser opening one would create a run that never ran.
 */

import { useCallback, useState } from "react";

import { request } from "./client.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import { projectsOf } from "./selection.mjs";
import {
  EMPTY_FILTERED_SENTENCE,
  EMPTY_SENTENCE,
  NO_FILTERS,
  STATUS_FILTERS,
  appendPage,
  durationOf,
  isFiltered,
  isIncomplete,
  listQuery,
  runDescription,
  runTitle,
  statusLabel,
  statusTone,
  type Filters,
} from "./sessions.mjs";
import type { SessionList, SessionView } from "./generated/api.js";

export function Sessions() {
  const { me, project, selection, workspace } = useApp();
  // The project's scope when there is one, else the workspace's — the
  // nearest scope the reader has selected is the one they mean, and the
  // gateway lists the subtree, so a workspace shows its projects' runs.
  const scopeId = project?.scope_id ?? workspace?.scope_id ?? null;
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  // Pages already read, oldest page first. Reset whenever the question
  // changes, because a page of one question is not a page of another.
  const [seen, setSeen] = useState<SessionView[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);

  const narrow = useCallback((next: Partial<Filters>) => {
    setFilters((current) => ({ ...current, ...next }));
    setSeen([]);
    setCursor(null);
  }, []);

  const projects = projectsOf(me, selection.workspaceId);
  const query = listQuery(filters, scopeId, cursor);
  // Every part of the question is in the key. `cursor` included: a second
  // page is a second read, and holding both under one key would mean the
  // first answer overwriting the second on any refresh.
  const cacheKey = `sessions/${scopeId ?? "all"}/${JSON.stringify(query)}`;
  const entry = useQuery(cacheKey, () => request("list_sessions", { query }));
  const retry = useRefresh(cacheKey);

  return (
    <>
      <PageHeading route="sessions" />
      <FilterBar
        filters={filters}
        projects={projects.map((item) => ({ id: item.id, label: item.display_name }))}
        onChange={narrow}
        onClear={() => narrow(NO_FILTERS)}
      />
      <Loaded<SessionList> entry={entry} what="the sessions" onRetry={retry}>
        {(body) => {
          // The accumulated view: pages already read, plus this one. Computed
          // in render rather than pushed into state from an effect — an
          // effect that appends runs twice under StrictMode and would show
          // every row twice.
          const rows = appendPage(seen, body.sessions);
          return (
            <>
              {rows.length === 0 ? (
                <p className="muted">
                  {isFiltered(filters) ? EMPTY_FILTERED_SENTENCE : EMPTY_SENTENCE}
                </p>
              ) : (
                <ul className="sessions">
                  {rows.map((session) => (
                    <Row key={session.id} session={session} />
                  ))}
                </ul>
              )}
              {body.next_cursor ? (
                <p>
                  <button
                    type="button"
                    onClick={() => {
                      setSeen(rows);
                      setCursor(body.next_cursor ?? null);
                    }}
                  >
                    Load more
                  </button>{" "}
                  <span className="muted">
                    {rows.length} shown. A page can be empty and still have more below it — runs
                    you may not read are skipped rather than counted.
                  </span>
                </p>
              ) : rows.length > 0 ? (
                <p className="muted">That is every run you can read here.</p>
              ) : null}
            </>
          );
        }}
      </Loaded>
    </>
  );
}

/**
 * One run in the listing.
 *
 * A link, not an expander. CPR-10 opened the timeline in place; a run now has
 * its own address, so it can be bookmarked, pasted into a ticket and reopened
 * after a refresh — which is what somebody investigating a failed run
 * actually needs to do with it.
 */
function Row({ session }: { session: SessionView }) {
  const duration = durationOf(session, Date.now());
  return (
    <li className={isIncomplete(session) ? "incomplete" : undefined}>
      <Link href={hrefOf("session", { session_id: session.id })} className="row">
        <strong>{runTitle(session)}</strong>{" "}
        <span className={`tag ${statusTone(session.status)}`}>{statusLabel(session.status)}</span>
        <div className="muted">
          {runDescription(session)} · {session.principal_id} · started {whenOf(session.started_at)}
          {duration === null ? null : ` · ${duration}`}
        </div>
      </Link>
    </li>
  );
}

/**
 * The filters, as one row of controls.
 *
 * Every one of them narrows and none of them widens, which is why they can be
 * combined without the page having to explain what a combination means. The
 * dates are a reader's calendar days; `listQuery` turns them into the
 * half-open instant range the API documents, so "the 3rd to the 4th" includes
 * the 4th.
 */
function FilterBar({
  filters,
  projects,
  onChange,
  onClear,
}: {
  filters: Filters;
  projects: { id: string; label: string }[];
  onChange: (next: Partial<Filters>) => void;
  onClear: () => void;
}) {
  return (
    <form className="filters" onSubmit={(event) => event.preventDefault()}>
      <label>
        <span className="switcher-label">State</span>
        <select
          value={filters.status ?? ""}
          onChange={(event) =>
            onChange({ status: (event.target.value || null) as Filters["status"] })
          }
        >
          <option value="">Any state</option>
          {STATUS_FILTERS.map((status) => (
            <option key={status} value={status}>
              {statusLabel(status)}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span className="switcher-label">Project</span>
        <select
          value={filters.projectId ?? ""}
          onChange={(event) => onChange({ projectId: event.target.value || null })}
        >
          <option value="">Any project</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span className="switcher-label">Client</span>
        <input
          type="text"
          value={filters.clientName}
          placeholder="claude-code"
          onChange={(event) => onChange({ clientName: event.target.value })}
        />
      </label>
      <label>
        <span className="switcher-label">Who</span>
        <input
          type="text"
          value={filters.principalId}
          placeholder="somebody@example.com"
          onChange={(event) => onChange({ principalId: event.target.value })}
        />
      </label>
      <label>
        <span className="switcher-label">From</span>
        <input
          type="date"
          value={filters.from}
          onChange={(event) => onChange({ from: event.target.value })}
        />
      </label>
      <label>
        <span className="switcher-label">To</span>
        <input
          type="date"
          value={filters.to}
          onChange={(event) => onChange({ to: event.target.value })}
        />
      </label>
      {isFiltered(filters) ? (
        <button type="button" onClick={onClear}>
          Clear filters
        </button>
      ) : null}
    </form>
  );
}

/**
 * Customer-safe operational visibility for one selected project (CPR-45).
 *
 * This page composes three existing generated public APIs. Each API applies
 * its own PDP and forced-RLS decision; the page does not infer tenant totals,
 * call the infrastructure health plane or turn an empty authorised page into
 * a claim that no hidden rows exist.
 */

import { request } from "./client.mjs";
import { Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import type {
  CaptureBatchListView,
  CaptureBatchView,
  ContextRunListView,
  ContextRunView,
  SessionList,
  SessionView,
} from "./generated/api.js";

const RECENT_LIMIT = "8";

/** The exact three bounded public reads this page makes. */
export function operationsReadPlan(projectId: string, scopeId: string) {
  const sessions = { scope_id: scopeId, project_id: projectId, limit: RECENT_LIMIT };
  const contextRuns = { project_id: projectId, limit: RECENT_LIMIT };
  const captureBatches = { project_id: projectId, limit: RECENT_LIMIT };
  return {
    sessions: {
      key: `operations/${projectId}/sessions/${JSON.stringify(sessions)}`,
      query: sessions,
    },
    contextRuns: {
      key: `operations/${projectId}/context-runs/${JSON.stringify(contextRuns)}`,
      query: contextRuns,
    },
    captureBatches: {
      key: `operations/${projectId}/capture-batches/${JSON.stringify(captureBatches)}`,
      query: captureBatches,
    },
  };
}

export function Operations() {
  const { project } = useApp();
  return (
    <>
      <PageHeading route="operations" />
      {project ? (
        <ProjectOperations projectId={project.id} scopeId={project.scope_id} />
      ) : (
        <section>
          <h2>Select a project</h2>
          <p className="muted">
            Operations reads one selected project. Choose one above; no tenant-wide fallback is
            inferred.
          </p>
        </section>
      )}
    </>
  );
}

function ProjectOperations({ projectId, scopeId }: { projectId: string; scopeId: string }) {
  const plan = operationsReadPlan(projectId, scopeId);
  const sessions = useQuery(plan.sessions.key, () =>
    request("list_sessions", { query: plan.sessions.query }),
  );
  const contextRuns = useQuery(plan.contextRuns.key, () =>
    request("list_context_runs", { query: plan.contextRuns.query }),
  );
  const captureBatches = useQuery(plan.captureBatches.key, () =>
    request("list_capture_batches", { query: plan.captureBatches.query }),
  );
  const refreshSessions = useRefresh(plan.sessions.key);
  const refreshContextRuns = useRefresh(plan.contextRuns.key);
  const refreshCaptureBatches = useRefresh(plan.captureBatches.key);
  const loadedTimes = [sessions, contextRuns, captureBatches].flatMap((entry) =>
    entry.status === "ready" ? [entry.loadedAt] : [],
  );
  const latestLoadedAt = loadedTimes.length > 0 ? Math.max(...loadedTimes) : null;

  return (
    <>
      <div className="banner" role="status">
        {latestLoadedAt === null
          ? "Each section is a separate bounded, authorised read."
          : `Latest response loaded ${whenOf(new Date(latestLoadedAt).toISOString())}.`}{" "}
        Sections may have different fetch times and can already be stale.
        <p>
          <button
            type="button"
            onClick={() => {
              refreshSessions();
              refreshContextRuns();
              refreshCaptureBatches();
            }}
          >
            Refresh all
          </button>
        </p>
      </div>

      <section>
        <h2>Recent sessions</h2>
        <Loaded<SessionList>
          entry={sessions}
          what="recent sessions"
          onRetry={refreshSessions}
        >
          {(body) => <SessionRows rows={body.sessions} />}
        </Loaded>
      </section>

      <section>
        <h2>Recent context runs</h2>
        <Loaded<ContextRunListView>
          entry={contextRuns}
          what="recent context runs"
          onRetry={refreshContextRuns}
        >
          {(body) => <ContextRows rows={body.runs} />}
        </Loaded>
      </section>

      <section>
        <h2>Recent Capture work</h2>
        <Loaded<CaptureBatchListView>
          entry={captureBatches}
          what="recent Capture work"
          onRetry={refreshCaptureBatches}
        >
          {(body) => <CaptureRows rows={body.batches} />}
        </Loaded>
      </section>

      <UnavailableSignals />
    </>
  );
}

function SessionRows({ rows }: { rows: SessionView[] }) {
  if (rows.length === 0) {
    return (
      <p className="muted">No policy-visible recent sessions were returned for this project.</p>
    );
  }
  return (
    <ul className="sessions">
      {rows.map((session) => (
        <li key={session.id}>
          <Link href={hrefOf("session", { session_id: session.id })} className="row">
            <strong>{session.client_name}</strong>{" "}
            <span className={`tag ${statusTone(session.status)}`}>{session.status}</span>
            <div className="muted">
              Started {whenOf(session.started_at)}
              {session.last_observed_at
                ? ` · last activity ${whenOf(session.last_observed_at)}`
                : ""}
              {session.ended_at ? ` · ended ${whenOf(session.ended_at)}` : ""}
            </div>
          </Link>
        </li>
      ))}
    </ul>
  );
}

function ContextRows({ rows }: { rows: ContextRunView[] }) {
  if (rows.length === 0) {
    return (
      <p className="muted">No policy-visible recent context runs were returned for this project.</p>
    );
  }
  return (
    <ul className="sessions">
      {rows.map((run) => (
        <ContextRow key={run.id} run={run} />
      ))}
    </ul>
  );
}

function ContextRow({ run }: { run: ContextRunView }) {
  const affected = run.degraded.flatMap((leg) =>
    leg === "embedder" || leg === "retrieval" ? [leg] : [],
  );
  if (affected.length < run.degraded.length) affected.push("another recorded leg");
  return (
    <li>
      <Link href={hrefOf("context-run", { context_run_id: run.id })} className="row">
        <strong>Context composition</strong>{" "}
        <span className={`tag ${statusTone(run.completion_status)}`}>
          {run.completion_status}
        </span>
        {run.degraded.length > 0 ? <span className="tag warn">degraded</span> : null}
        <div className="muted">
          Composed {whenOf(run.created_at)}
          {affected.length > 0 ? ` · affected legs: ${affected.join(", ")}` : ""}
        </div>
      </Link>
    </li>
  );
}

function CaptureRows({ rows }: { rows: CaptureBatchView[] }) {
  if (rows.length === 0) {
    return (
      <p className="muted">No policy-visible recent Capture work was returned for this project.</p>
    );
  }
  return (
    <ul className="sessions">
      {rows.map((batch) => (
        <li key={batch.id}>
          <div className="row">
            <strong>Capture</strong>{" "}
            <span className={`tag ${statusTone(batch.state)}`}>{batch.state}</span>
            <div className="muted">
              {batch.event_count} frozen event{batch.event_count === 1 ? "" : "s"} ·{" "}
              {batch.attempts} attempt{batch.attempts === 1 ? "" : "s"} · created{" "}
              {whenOf(batch.created_at)}
              {batch.started_at ? ` · started ${whenOf(batch.started_at)}` : ""}
              {batch.completed_at ? ` · completed ${whenOf(batch.completed_at)}` : ""}
            </div>
            {batch.error_code ? (
              <div className="banner warning" role="status">
                Safe failure code: {batch.error_code}
              </div>
            ) : null}
          </div>
        </li>
      ))}
    </ul>
  );
}

function statusTone(status: string): string {
  switch (status) {
    case "completed":
    case "ended":
      return "done";
    case "active":
    case "running":
      return "active";
    case "failed":
    case "abandoned":
      return "warn";
    default:
      return "";
  }
}

function UnavailableSignals() {
  return (
    <section>
      <h2>Not available through the public API yet</h2>
      <p className="muted">
        These signals are not inferred from infrastructure probes, missing rows or policy-filtered
        pages:
      </p>
      <ul>
        <li>dependency health and degraded external providers;</li>
        <li>worker last-seen and durable operation retry or dead-letter state;</li>
        <li>context latency and token aggregates;</li>
        <li>Knowledge freshness/index health and unhealthy Skill or MCP tests; and</li>
        <li>latest backup and isolated-restore result.</li>
      </ul>
    </section>
  );
}

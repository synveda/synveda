/**
 * One explainable context plan (CPR-21, ADR-0084).
 *
 * This page renders only the generated detail response. It never follows a
 * Knowledge or source address behind the gateway, and it never derives a
 * denied count from missing rows: exact candidates, selections and evidence
 * have already been freshly decided by the context API. A policy gap is the
 * one aggregate sentence that API permits.
 */

import { useEffect, useState } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import {
  FEEDBACK_TYPES,
  canGiveFeedback,
  candidateForSelection,
  excludedCandidates,
  feedbackBody,
  feedbackLabel,
  reasonLabel,
  retentionDescription,
  revisionOf,
  scorePercent,
  scoresOf,
  selectionState,
  type FeedbackType,
} from "./context.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import { runDescription, runTitle, statusLabel } from "./sessions.mjs";
import type {
  ContextCandidateView,
  ContextFeedbackView,
  ContextGraphStepView,
  ContextRunDetailView,
  ContextRunListView,
  ContextRunView,
  ContextSelectionView,
  KnowledgeSourceView,
  MeView,
  SessionList,
  SessionView,
} from "./generated/api.js";

/** Core Context journey: choose a visible Session, request, then inspect. */
export function Context() {
  const { me, project } = useApp();
  return (
    <>
      <PageHeading route="context" />
      {project ? (
        <ContextWorkbench me={me} projectId={project.id} />
      ) : (
        <section>
          <h2>Select a project</h2>
          <p className="muted">
            Context is requested for a Session in one selected project. Choose a project above;
            no tenant-wide fallback is inferred.
          </p>
        </section>
      )}
    </>
  );
}

function ContextWorkbench({ me, projectId }: { me: MeView; projectId: string }) {
  const sessionsKey = `context/sessions/${projectId}`;
  const runsKey = `context/runs/${projectId}`;
  const sessions = useQuery(sessionsKey, () =>
    request("list_sessions", { query: { project_id: projectId, limit: "50" } }),
  );
  const runs = useQuery(runsKey, () =>
    request("list_context_runs", { query: { project_id: projectId, limit: "12" } }),
  );
  const refreshSessions = useRefresh(sessionsKey);
  const refreshRuns = useRefresh(runsKey);
  const [created, setCreated] = useState<ContextRunView | null>(null);

  return (
    <>
      <section className="workbench-section">
        <h2>Request context</h2>
        <p className="muted">
          Synveda composes policy-visible Knowledge for an existing Session. The governed
          configuration may narrow the requested budget or sensitivity.
        </p>
        <Loaded<SessionList>
          entry={sessions}
          what="sessions available for context"
          onRetry={refreshSessions}
        >
          {(body) => (
            <ContextRequestForm
              key={projectId}
              me={me}
              sessions={body.sessions}
              onCreated={(run) => {
                setCreated(run);
                invalidate(runsKey);
              }}
            />
          )}
        </Loaded>
      </section>

      {created ? (
        <section className="workbench-result" aria-live="polite">
          <div className="result-heading">
            <div>
              <span className="eyebrow">Returned context</span>
              <h2>{created.query ?? "Session context"}</h2>
            </div>
            <span className={`tag ${created.completion_status === "completed" ? "done" : "warn"}`}>
              {created.completion_status}
            </span>
          </div>
          <p className="muted">
            {created.tokens} tokens · {created.selection_count} selected revisions · trace {created.trace_retention_mode}
          </p>
          <ContextRunDetail contextRunId={created.id} embedded />
        </section>
      ) : null}

      <section className="workbench-section">
        <h2>Recent context</h2>
        <Loaded<ContextRunListView> entry={runs} what="recent context" onRetry={refreshRuns}>
          {(body) => <ContextRunRows rows={body.runs} />}
        </Loaded>
      </section>
    </>
  );
}

function ContextRequestForm({
  me,
  sessions,
  onCreated,
}: {
  me: MeView;
  sessions: SessionView[];
  onCreated: (run: ContextRunView) => void;
}) {
  const [sessionId, setSessionId] = useState(sessions[0]?.id ?? "");
  const [query, setQuery] = useState("");
  const [budget, setBudget] = useState("");
  const [sensitivity, setSensitivity] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selected = sessions.find((session) => session.id === sessionId) ?? null;
  const mayRequest = selected ? offersSessionWrite(me, selected) : false;

  useEffect(() => {
    if (!sessions.some((session) => session.id === sessionId)) {
      setSessionId(sessions[0]?.id ?? "");
    }
  }, [sessionId, sessions]);

  if (sessions.length === 0) {
    return (
      <p className="muted">
        No policy-visible Session exists in this project yet. An agent opens Sessions; the
        console does not create a run that never ran.
      </p>
    );
  }

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!selected || !mayRequest) return;
    setBusy(true);
    setError(null);
    const answer = await request("create_context_run", {
      path: { session_id: selected.id },
      body: {
        ...(query.trim() ? { query: query.trim() } : {}),
        ...(budget ? { budget_tokens: Number(budget) } : {}),
        ...(sensitivity ? { max_sensitivity: sensitivity } : {}),
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    if (answer.kind === "ok") {
      onCreated(answer.body);
    } else {
      setError(
        answer.kind === "unauthenticated"
          ? "Your session expired before context was requested."
          : answer.message,
      );
    }
  };

  return (
    <form className="context-request" onSubmit={(event) => void submit(event)}>
      <label>
        <span>Session</span>
        <select value={sessionId} onChange={(event) => setSessionId(event.target.value)}>
          {sessions.map((session) => (
            <option key={session.id} value={session.id}>
              {runTitle(session)} · {statusLabel(session.status)} · {runDescription(session)}
            </option>
          ))}
        </select>
      </label>
      <label className="wide-field">
        <span>Task or query</span>
        <textarea
          rows={3}
          value={query}
          placeholder="What should the agent know for this task?"
          onChange={(event) => setQuery(event.target.value)}
        />
      </label>
      <label>
        <span>Token budget <span className="muted">(optional)</span></span>
        <input
          type="number"
          min={1}
          max={4294967295}
          step={1}
          inputMode="numeric"
          value={budget}
          placeholder="Governed default"
          onChange={(event) => setBudget(event.target.value)}
        />
      </label>
      <label>
        <span>Maximum sensitivity <span className="muted">(optional)</span></span>
        <select value={sensitivity} onChange={(event) => setSensitivity(event.target.value)}>
          <option value="">Governed default</option>
          <option value="public">Public</option>
          <option value="internal">Internal</option>
          <option value="confidential">Confidential</option>
          <option value="restricted">Restricted</option>
        </select>
      </label>
      <div className="form-actions wide-field">
        <button type="submit" disabled={busy || !mayRequest}>
          {busy ? "Requesting…" : "Request context"}
        </button>
        {mayRequest ? null : (
          <span className="muted">
            Your current capability forecast does not offer session.write at this Session&rsquo;s
            scope. The gateway remains authoritative.
          </span>
        )}
      </div>
      {error ? <div className="banner error wide-field" role="alert">{error}</div> : null}
    </form>
  );
}

function offersSessionWrite(me: MeView, session: SessionView): boolean {
  const anchor = me.anchors.find((candidate) => candidate.scope_id === session.scope_id);
  return (anchor?.actions ?? me.capabilities.actions)["session.write"] === true;
}

function ContextRunRows({ rows }: { rows: ContextRunView[] }) {
  if (rows.length === 0) {
    return <p className="muted">No policy-visible context has been requested for this project.</p>;
  }
  return (
    <ul className="sessions compact-list">
      {rows.map((run) => (
        <li key={run.id}>
          <Link href={hrefOf("context-run", { context_run_id: run.id })} className="row">
            <strong>{run.query ?? "Session context"}</strong>{" "}
            <span className={`tag ${run.completion_status === "completed" ? "done" : "warn"}`}>
              {run.completion_status}
            </span>
            <span className="muted">
              {whenOf(run.created_at)} · {run.tokens} tokens · {run.selection_count} selections
            </span>
          </Link>
        </li>
      ))}
    </ul>
  );
}

export function ContextInspector({ contextRunId }: { contextRunId: string }) {
  return (
    <>
      <PageHeading route="context-run" />
      <ContextRunDetail contextRunId={contextRunId} />
    </>
  );
}

function ContextRunDetail({
  contextRunId,
  embedded = false,
}: {
  contextRunId: string;
  embedded?: boolean;
}) {
  const cacheKey = `context-runs/${contextRunId}`;
  const entry = useQuery(cacheKey, () =>
    request("get_context_run", { path: { id: contextRunId } }),
  );
  const retry = useRefresh(cacheKey);
  return (
    <Loaded<ContextRunDetailView> entry={entry} what="this context run" onRetry={retry}>
      {(detail) => <Inspector detail={detail} cacheKey={cacheKey} embedded={embedded} />}
    </Loaded>
  );
}

function Inspector({
  detail,
  cacheKey,
  embedded,
}: {
  detail: ContextRunDetailView;
  cacheKey: string;
  embedded: boolean;
}) {
  const { run } = detail;
  const exclusions = excludedCandidates(detail.candidates);
  const policyMessage = detail.policy_exclusion_message ?? run.policy_exclusion_message;
  return (
    <article className="context-inspector">
      <p>
        <Link href={hrefOf("session", { session_id: run.session_id })}>← Session timeline</Link>
      </p>
      {embedded ? null : (
        <header className="context-run-heading">
          <div>
            <span className="eyebrow">Context run {run.id}</span>
            <h2>{run.query ?? "Task text not retained"}</h2>
          </div>
          <span className="tag done">{run.completion_status}</span>
        </header>
      )}

      <div className="banner" role="status">
        {retentionDescription(run.trace_retention_mode)}
      </div>
      {policyMessage ? (
        <div className="banner warning" role="status">
          {policyMessage} No hidden candidate address, title, reason or count is shown.
        </div>
      ) : null}

      <TaskAndRendered run={run} />
      <Selections
        detail={detail}
        cacheKey={cacheKey}
      />
      <RunFacts run={run} />
      <Exclusions run={run} candidates={exclusions} />
      <FeedbackHistory feedback={detail.feedback} />
    </article>
  );
}

function RunFacts({ run }: { run: ContextRunView }) {
  const requested = run.requested_budget_tokens;
  return (
    <section>
      <h3>Plan facts</h3>
      <dl className="facts">
        <dt>Composed</dt>
        <dd>{whenOf(run.created_at)}</dd>
        <dt>Knowledge as of</dt>
        <dd>{whenOf(run.as_of)}</dd>
        <dt>Token budget</dt>
        <dd>
          {requested === undefined || requested === null
            ? `${run.budget_tokens} governed tokens`
            : `${requested} requested · ${run.budget_tokens} governed`}
          {` · ${run.tokens} used`}
        </dd>
        <dt>Visible trace</dt>
        <dd>
          {run.selection_count} selected · {run.candidate_count} candidates · {run.entry_count} rendered entries
        </dd>
        <dt>Trace retention</dt>
        <dd>{run.trace_retention_mode.replaceAll("_", " ")}</dd>
      </dl>
      <details className="technical-details">
        <summary>Planner versions and integrity evidence</summary>
        <dl className="facts">
          <dt>Retrieval</dt><dd>{run.retrieval_version}</dd>
          <dt>Index</dt><dd>{run.index_version}</dd>
          <dt>Embedding</dt><dd>{run.embedding_model ?? "not run"}</dd>
          <dt>Graph</dt><dd>{run.graph_version ?? "not run"}</dd>
          <dt>Rendered context hash</dt><dd className="mono breakable">{run.block_hash}</dd>
          {run.query_hash ? <><dt>Task hash</dt><dd className="mono breakable">{run.query_hash}</dd></> : null}
        </dl>
      </details>
      {run.degraded.length > 0 ? (
        <div className="banner warning" role="status">
          Degraded retrieval: {run.degraded.join(" · ")}. The planner recorded this fallback; it did not silently claim the missing leg ran.
        </div>
      ) : null}
    </section>
  );
}

function TaskAndRendered({ run }: { run: ContextRunView }) {
  return (
    <section>
      <h3>Delivered context</h3>
      {run.query === undefined || run.query === null ? (
        <p className="muted">The original task is unavailable under this trace-retention mode.</p>
      ) : (
        <p><strong>Original task or query:</strong> {run.query}</p>
      )}
      {run.rendered === undefined || run.rendered === null ? (
        <p className="muted">The rendered context body is unavailable under this trace-retention mode.</p>
      ) : run.rendered.length === 0 ? (
        <p>No context content was delivered.</p>
      ) : (
        <pre className="context-rendered">{run.rendered}</pre>
      )}
    </section>
  );
}

function Selections({ detail, cacheKey }: { detail: ContextRunDetailView; cacheKey: string }) {
  const mode = detail.run.trace_retention_mode;
  return (
    <section>
      <h3>Selected context</h3>
      {mode === "disabled" ? (
        <p className="muted">Selection detail was not retained. This is not a claim that the delivery selected nothing.</p>
      ) : detail.selections.length === 0 ? (
        <p>No policy-visible context selection is available in this trace.</p>
      ) : (
        <ol className="context-selections">
          {detail.selections.map((selection) => (
            <Selection
              key={selection.id}
              selection={selection}
              candidate={candidateForSelection(detail.candidates, selection)}
              feedback={detail.feedback.filter((entry) => entry.context_selection_id === selection.id)}
              runId={detail.run.id}
              mode={mode}
              cacheKey={cacheKey}
            />
          ))}
        </ol>
      )}
    </section>
  );
}

function Selection({
  selection,
  candidate,
  feedback,
  runId,
  mode,
  cacheKey,
}: {
  selection: ContextSelectionView;
  candidate: ContextCandidateView | null;
  feedback: ContextFeedbackView[];
  runId: string;
  mode: string;
  cacheKey: string;
}) {
  const revision = revisionOf(selection.revision);
  const proposal = selection.unreviewed_candidate;
  const scores = scoresOf(candidate?.scores);
  const title = revision?.title ?? proposal?.content.title ?? `Content ${selection.content_hash.slice(0, 16)}…`;
  const state = selectionState(candidate, revision);
  return (
    <li className="context-selection">
      <header>
        <div>
          <span className="eyebrow">Rank {selection.rank} · {selection.token_count} tokens</span>
          <h4>{title}</h4>
        </div>
        <span className={`tag ${state.startsWith("current") ? "done" : "warn"}`}>
          {state}
        </span>
      </header>
      <p className="context-reasons">
        {selection.reason_codes.map((reason) => (
          <span className="tag" key={reason}>{reasonLabel(reason)}</span>
        ))}
      </p>
      {proposal ? (
        <>
          <div className="banner warning" role="status">
            Unreviewed capture candidate. This was explicitly admitted by the effective governed configuration and was not published Knowledge at planning time.
          </div>
          <p>{proposal.content.summary}</p>
          <pre className="context-content">{proposal.content.body_markdown}</pre>
          <p className="muted">
            Candidate {proposal.id} · {proposal.knowledge_type} · {proposal.content.sensitivity} · confidence {proposal.content.confidence_permille} / 1000 · state now {proposal.state}
          </p>
          <UnreviewedEvidence candidate={proposal} mode={mode} />
        </>
      ) : revision ? (
        <>
          <p>{revision.summary}</p>
          <pre className="context-content">{revision.body_markdown}</pre>
          <p className="muted">
            Revision {revision.revision_number} · {revision.sensitivity} · confidence {revision.confidence_permille} / 1000 · transaction {whenOf(revision.transaction_time)}
          </p>
        </>
      ) : (
        <p className="muted">Context content was not retained in this {mode} trace.</p>
      )}
      {selection.knowledge_item_id ? (
        <p>
          <Link
            href={`${hrefOf("knowledge-item", { knowledge_id: selection.knowledge_item_id })}${
              revision ? `#revision-${revision.id}` : ""
            }`}
          >
            {revision ? `Open selected Knowledge revision ${revision.revision_number}` : "Open current Knowledge item"}
          </Link>
        </p>
      ) : null}
      <p className="mono muted">Content hash {selection.content_hash}</p>
      <ScoreBreakdown scores={scores} />
      <GraphPath steps={selection.graph_path ?? []} mode={mode} />
      {proposal ? null : <Sources sources={selection.sources ?? []} mode={mode} />}
      <SelectionFeedback
        selection={selection}
        feedback={feedback}
        runId={runId}
        cacheKey={cacheKey}
      />
    </li>
  );
}

function UnreviewedEvidence({
  candidate,
  mode,
}: {
  candidate: NonNullable<ContextSelectionView["unreviewed_candidate"]>;
  mode: string;
}) {
  const evidence = [
    ...candidate.source_event_ids.map((id) => `session event ${id}`),
    ...candidate.source_artifact_ids.map((id) => `import artifact ${id}`),
  ];
  return (
    <section className="context-sources">
      <h5>Source evidence</h5>
      {evidence.length === 0 ? (
        <p className="muted">Source evidence is unavailable in this {mode} trace.</p>
      ) : (
        <ul>{evidence.map((address) => <li key={address}>{address}</li>)}</ul>
      )}
    </section>
  );
}

function ScoreBreakdown({ scores }: { scores: ReturnType<typeof scoresOf> }) {
  if (!scores) {
    return <p className="muted">Score components were not retained for this trace.</p>;
  }
  const rows = [
    ["Keyword", scores.keyword_micros],
    ["Embedding", scores.semantic_micros],
    ["Anchor retrieval", scores.anchor_micros],
    ["Relationship edges", scores.edge_weight_micros],
    ["Hop penalty", -scores.hop_penalty_micros],
    ["Freshness", scores.freshness_micros],
    ["Explicit pin", scores.pin_micros],
    ["Current state", scores.current_state_micros],
    ["Final score", scores.final_micros],
  ] as const;
  return (
    <dl className="context-scores">
      {rows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{scorePercent(value)}</dd>
        </div>
      ))}
    </dl>
  );
}

function GraphPath({ steps, mode }: { steps: ContextGraphStepView[]; mode: string }) {
  return (
    <section className="context-graph-path">
      <h5>Anchor and relationship path</h5>
      {steps.length === 0 ? (
        <p className="muted">Direct lexical, semantic, pinned or recency anchor; no relationship edge contributed.</p>
      ) : (
        <ol>
          {steps.map((step) => {
            const from = step.from_item_id
              ? `Knowledge ${step.from_item_id} @ ${step.from_revision_id}`
              : `content ${step.from_content_hash}`;
            const to = step.to_item_id
              ? `Knowledge ${step.to_item_id} @ ${step.to_revision_id}`
              : `content ${step.to_content_hash}`;
            return (
              <li key={`${step.ordinal}:${step.relation_hash}`}>
                <strong>Hop {step.hop}: {step.relation_type.replaceAll("_", " ")}</strong>
                {` · ${step.direction} · ${step.supporting ? scorePercent(step.edge_weight_micros) : "warning only"}`}
                <br />
                <span className="mono">{from} → {to}</span>
                <br />
                <span className="mono">
                  {step.relation_id ? `relation ${step.relation_id}` : `${mode} relation hash ${step.relation_hash}`}
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

function Sources({ sources, mode }: { sources: KnowledgeSourceView[]; mode: string }) {
  return (
    <section className="context-sources">
      <h5>Source evidence</h5>
      {sources.length === 0 ? (
        <p className="muted">Source evidence is unavailable in this {mode} trace.</p>
      ) : (
        <ul>
          {sources.map((source) => (
            <li key={source.id}>
              <strong>{source.source_type.replaceAll("_", " ")}</strong>
              {source.locator ? ` · ${source.locator}` : ""}
              {source.source_revision ? ` · revision ${source.source_revision}` : ""}
              {source.session_event_id ? ` · session event ${source.session_event_id}` : ""}
              {source.content_hash ? <span className="mono"> · {source.content_hash}</span> : null}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

type FeedbackAnswer = Answer<ContextFeedbackView>;

function SelectionFeedback({
  selection,
  feedback,
  runId,
  cacheKey,
}: {
  selection: ContextSelectionView;
  feedback: ContextFeedbackView[];
  runId: string;
  cacheKey: string;
}) {
  const [busy, setBusy] = useState<FeedbackType | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (!canGiveFeedback(selection)) {
    return (
      <p className="muted">
        {selection.channel === "unreviewed_candidates"
          ? "Outcome feedback is unavailable until this candidate becomes an immutable published Knowledge revision."
          : "Feedback is unavailable because this trace retained no exact revision address."}
      </p>
    );
  }
  const record = async (feedbackType: FeedbackType): Promise<void> => {
    const body = feedbackBody(selection, feedbackType);
    if (!body) return;
    setBusy(feedbackType);
    setError(null);
    const answer: FeedbackAnswer = await request("create_context_feedback", {
      path: { id: runId },
      body,
      idempotencyKey: idempotencyKey(),
    });
    setBusy(null);
    if (answer.kind === "ok") {
      invalidate(cacheKey);
    } else {
      setError(
        answer.kind === "unauthenticated"
          ? "Your session expired before feedback was recorded."
          : answer.message,
      );
    }
  };

  return (
    <section className="context-feedback">
      <h5>Outcome feedback</h5>
      <p className="muted">Selection alone records no positive outcome. Add only what was actually observed.</p>
      <div className="context-feedback-actions">
        {FEEDBACK_TYPES.map((feedbackType) => (
          <button
            type="button"
            key={feedbackType}
            disabled={busy !== null}
            onClick={() => void record(feedbackType)}
          >
            {busy === feedbackType ? "Recording…" : feedbackLabel(feedbackType)}
          </button>
        ))}
      </div>
      {feedback.length > 0 ? (
        <ul className="context-feedback-history">
          {feedback.map((entry) => (
            <li key={entry.id}>{feedbackLabel(entry.feedback_type)} · {entry.principal_id} · {whenOf(entry.created_at)}</li>
          ))}
        </ul>
      ) : null}
      {error ? <p className="form-error" role="alert">{error}</p> : null}
    </section>
  );
}

function Exclusions({ run, candidates }: { run: ContextRunView; candidates: ContextCandidateView[] }) {
  return (
    <section>
      <h3>Visible exclusions</h3>
      {run.trace_retention_mode === "disabled" ? (
        <p className="muted">Candidate exclusions were not retained.</p>
      ) : candidates.length === 0 ? (
        <p>No visible candidate exclusion is retained.</p>
      ) : (
        <ol className="context-exclusions">
          {candidates.map((candidate) => {
            const revision = revisionOf(candidate.revision);
            const scores = scoresOf(candidate.scores);
            return (
              <li key={candidate.id}>
                <header>
                  <div>
                    <span className="eyebrow">Candidate {candidate.ordinal + 1}</span>
                    <h4>{revision?.title ?? candidate.unreviewed_candidate?.content.title ?? `Content ${candidate.content_hash.slice(0, 16)}…`}</h4>
                  </div>
                  <span className="tag warn">{reasonLabel(candidate.exclusion_reason as string)}</span>
                </header>
                <p>{selectionState(candidate, revision)}</p>
                <p>Excluded because <strong>{reasonLabel(candidate.exclusion_reason as string)}</strong>.</p>
                {candidate.reason_codes.length > 0 ? <p className="muted">Evidence: {candidate.reason_codes.map(reasonLabel).join(" · ")}</p> : null}
                <ScoreBreakdown scores={scores} />
                <GraphPath steps={candidate.graph_path ?? []} mode={run.trace_retention_mode} />
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

function FeedbackHistory({ feedback }: { feedback: ContextFeedbackView[] }) {
  return (
    <section>
      <h3>Recorded outcomes</h3>
      {feedback.length === 0 ? (
        <p>No feedback has been asserted. Retrieval and selection do not count as helpfulness.</p>
      ) : (
        <ul>
          {feedback.map((entry) => (
            <li key={entry.id}>
              {feedbackLabel(entry.feedback_type)} · revision <span className="mono">{entry.knowledge_revision_id}</span> · selection <span className="mono">{entry.context_selection_id}</span> · {entry.principal_id} · {whenOf(entry.created_at)}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

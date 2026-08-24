/**
 * One explainable context plan (CPR-21, ADR-0084).
 *
 * This page renders only the generated detail response. It never follows a
 * Knowledge or source address behind the gateway, and it never derives a
 * denied count from missing rows: exact candidates, selections and evidence
 * have already been freshly decided by the context API. A policy gap is the
 * one aggregate sentence that API permits.
 */

import { useState } from "react";

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
import { PageHeading } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import type {
  ContextCandidateView,
  ContextFeedbackView,
  ContextRunDetailView,
  ContextRunView,
  ContextSelectionView,
  KnowledgeSourceView,
} from "./generated/api.js";

export function ContextInspector({ contextRunId }: { contextRunId: string }) {
  const cacheKey = `context-runs/${contextRunId}`;
  const entry = useQuery(cacheKey, () =>
    request("get_context_run", { path: { id: contextRunId } }),
  );
  const retry = useRefresh(cacheKey);

  return (
    <>
      <PageHeading route="context-run" />
      <Loaded<ContextRunDetailView> entry={entry} what="this context run" onRetry={retry}>
        {(detail) => <Inspector detail={detail} cacheKey={cacheKey} />}
      </Loaded>
    </>
  );
}

function Inspector({ detail, cacheKey }: { detail: ContextRunDetailView; cacheKey: string }) {
  const { run } = detail;
  const exclusions = excludedCandidates(detail.candidates);
  const policyMessage = detail.policy_exclusion_message ?? run.policy_exclusion_message;
  return (
    <article className="context-inspector">
      <p>
        <Link href={hrefOf("session", { session_id: run.session_id })}>← Session timeline</Link>
      </p>
      <header className="context-run-heading">
        <div>
          <span className="eyebrow">Context run {run.id}</span>
          <h2>{run.query ?? "Task text not retained"}</h2>
        </div>
        <span className="tag done">{run.completion_status}</span>
      </header>

      <div className="banner" role="status">
        {retentionDescription(run.trace_retention_mode)}
      </div>
      {policyMessage ? (
        <div className="banner warning" role="status">
          {policyMessage} No hidden candidate address, title, reason or count is shown.
        </div>
      ) : null}

      <RunFacts run={run} />
      <TaskAndRendered run={run} />
      <Selections
        detail={detail}
        cacheKey={cacheKey}
      />
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
        <dt>Retrieval</dt>
        <dd>{run.retrieval_version}</dd>
        <dt>Index</dt>
        <dd>{run.index_version}</dd>
        <dt>Embedding</dt>
        <dd>{run.embedding_model ?? "not run"}</dd>
        <dt>Graph</dt>
        <dd>{run.graph_version ?? "not run"}</dd>
        <dt>Rendered context hash</dt>
        <dd className="mono">{run.block_hash}</dd>
        {run.query_hash ? (
          <>
            <dt>Task hash</dt>
            <dd className="mono">{run.query_hash}</dd>
          </>
        ) : null}
      </dl>
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
      <h3>Selected Knowledge</h3>
      {mode === "disabled" ? (
        <p className="muted">Selection detail was not retained. This is not a claim that the delivery selected nothing.</p>
      ) : detail.selections.length === 0 ? (
        <p>No policy-visible Knowledge revision is available in this trace.</p>
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
  const scores = scoresOf(candidate?.scores);
  const title = revision?.title ?? `Content ${selection.content_hash.slice(0, 16)}…`;
  return (
    <li className="context-selection">
      <header>
        <div>
          <span className="eyebrow">Rank {selection.rank} · {selection.token_count} tokens</span>
          <h4>{title}</h4>
        </div>
        <span className={`tag ${selectionState(candidate, revision).startsWith("current") ? "done" : "warn"}`}>
          {selectionState(candidate, revision)}
        </span>
      </header>
      <p className="context-reasons">
        {selection.reason_codes.map((reason) => (
          <span className="tag" key={reason}>{reasonLabel(reason)}</span>
        ))}
      </p>
      {revision ? (
        <>
          <p>{revision.summary}</p>
          <pre className="context-content">{revision.body_markdown}</pre>
          <p className="muted">
            Revision {revision.revision_number} · {revision.sensitivity} · confidence {revision.confidence_permille} / 1000 · transaction {whenOf(revision.transaction_time)}
          </p>
        </>
      ) : (
        <p className="muted">Knowledge content was not retained in this {mode} trace.</p>
      )}
      {selection.knowledge_item_id ? (
        <p><Link href={hrefOf("knowledge-item", { knowledge_id: selection.knowledge_item_id })}>Open current Knowledge item</Link></p>
      ) : null}
      <p className="mono muted">Content hash {selection.content_hash}</p>
      <ScoreBreakdown scores={scores} />
      <Sources sources={selection.sources ?? []} mode={mode} />
      <SelectionFeedback
        selection={selection}
        feedback={feedback}
        runId={runId}
        cacheKey={cacheKey}
      />
    </li>
  );
}

function ScoreBreakdown({ scores }: { scores: ReturnType<typeof scoresOf> }) {
  if (!scores) {
    return <p className="muted">Score components were not retained for this trace.</p>;
  }
  const rows = [
    ["Keyword", scores.keyword_micros],
    ["Embedding", scores.semantic_micros],
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
    return <p className="muted">Feedback is unavailable because this trace retained no exact revision address.</p>;
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
                    <h4>{revision?.title ?? `Content ${candidate.content_hash.slice(0, 16)}…`}</h4>
                  </div>
                  <span className="tag warn">{reasonLabel(candidate.exclusion_reason as string)}</span>
                </header>
                <p>{selectionState(candidate, revision)}</p>
                <p>Excluded because <strong>{reasonLabel(candidate.exclusion_reason as string)}</strong>.</p>
                {candidate.reason_codes.length > 0 ? <p className="muted">Evidence: {candidate.reason_codes.map(reasonLabel).join(" · ")}</p> : null}
                <ScoreBreakdown scores={scores} />
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

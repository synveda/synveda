/**
 * New Learnings (CPR-19).
 *
 * This is the lightweight capture-review experience. A candidate stays
 * visibly separate from published Knowledge; accepting, editing, merging or
 * replacing it calls the generated capture contract, which in turn enters
 * the one VedaFlow Knowledge command layer. Advanced Reviews remains the
 * place for a stricter profile's pending change, not a competing candidate
 * inbox.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import { KNOWLEDGE_TYPES, SENSITIVITIES } from "./knowledge.mjs";
import {
  CANDIDATE_STATES,
  EMPTY_LEARNINGS_FILTERS,
  appendBatches,
  appendCandidates,
  batchProgress,
  batchQuery,
  candidateQuery,
  candidateVisibility,
  decisionMessage,
  dismissBody,
  editAndAcceptBody,
  groupCandidates,
  matchLabel,
  mergeBody,
  placementEdits,
  proposedPublishScope,
  publishableScopes,
  replaceBody,
  stateLabel,
  type LearningsFilters,
  type PublishScope,
} from "./learnings.mjs";
import type {
  CaptureBatchListView,
  CaptureBatchView,
  CaptureCandidateListView,
  CaptureCandidateView,
  CaptureDecisionView,
  CaptureMatchView,
  KnowledgeContentBody,
  KnowledgeItemView,
  MeView,
  SessionEventView,
  TimelineEntry,
  TimelineView,
} from "./generated/api.js";

export function Learnings() {
  const { me, project } = useApp();
  const initial = useMemo<LearningsFilters>(
    () => ({ ...EMPTY_LEARNINGS_FILTERS, projectId: project?.id ?? "" }),
    [project?.id],
  );
  const [draft, setDraft] = useState(initial);
  const [filters, setFilters] = useState(initial);
  const [seenBatches, setSeenBatches] = useState<CaptureBatchView[]>([]);
  const [seenCandidates, setSeenCandidates] = useState<CaptureCandidateView[]>([]);
  const [batchCursor, setBatchCursor] = useState<string | null>(null);
  const [candidateCursor, setCandidateCursor] = useState<string | null>(null);

  useEffect(() => {
    setDraft(initial);
    setFilters(initial);
    setSeenBatches([]);
    setSeenCandidates([]);
    setBatchCursor(null);
    setCandidateCursor(null);
  }, [initial]);

  const batchParams = batchQuery(filters, batchCursor);
  const candidateParams = candidateQuery(filters, candidateCursor);
  const batchKey = `learnings/batches/${JSON.stringify(batchParams)}`;
  const candidateKey = `learnings/candidates/${JSON.stringify(candidateParams)}`;
  const batches = useQuery(batchKey, () => request("list_capture_batches", { query: batchParams }));
  const candidates = useQuery(candidateKey, () =>
    request("list_capture_candidates", { query: candidateParams }),
  );

  const apply = useCallback(() => {
    setSeenBatches([]);
    setSeenCandidates([]);
    setBatchCursor(null);
    setCandidateCursor(null);
    setFilters({ ...draft });
  }, [draft]);

  return (
    <>
      <PageHeading route="learnings" />
      <p className="muted learnings-intro">
        These are suggestions extracted from sessions, not active Knowledge. Accepting one always
        creates a VedaFlow change; a permissive profile may apply it immediately, while a stricter
        profile sends it to <Link href={hrefOf("reviews")}>Advanced Reviews</Link>.
      </p>
      <LearningFilters
        me={me}
        value={draft}
        onChange={(next) => setDraft((current) => ({ ...current, ...next }))}
        onApply={apply}
        onClear={() => {
          setDraft(EMPTY_LEARNINGS_FILTERS);
          setFilters(EMPTY_LEARNINGS_FILTERS);
          setSeenBatches([]);
          setSeenCandidates([]);
          setBatchCursor(null);
          setCandidateCursor(null);
        }}
      />
      <Loaded<CaptureBatchListView>
        entry={batches}
        what="capture batches"
        onRetry={useRefresh(batchKey)}
      >
        {(batchPage) => (
          <Loaded<CaptureCandidateListView>
            entry={candidates}
            what="new learnings"
            onRetry={useRefresh(candidateKey)}
          >
            {(candidatePage) => {
              const allBatches = appendBatches(seenBatches, batchPage.batches);
              const allCandidates = appendCandidates(seenCandidates, candidatePage.candidates);
              const groups = groupCandidates(allBatches, allCandidates, filters.state);
              return (
                <>
                  {groups.length === 0 ? (
                    <p className="muted">
                      {filters.state
                        ? `No visible ${stateLabel(filters.state)} candidates match these filters.`
                        : "No reviewable session learnings are visible here yet."}
                    </p>
                  ) : (
                    <div className="learning-batches">
                      {groups.map((group) => (
                        <BatchGroup
                          key={group.batchId}
                          batch={group.batch}
                          candidates={group.candidates}
                          allCandidates={allCandidates.filter(
                            (candidate) => candidate.batch_id === group.batchId,
                          )}
                          me={me}
                        />
                      ))}
                    </div>
                  )}
                  <div className="learning-pagination">
                    {candidatePage.next_cursor ? (
                      <button
                        type="button"
                        onClick={() => {
                          setSeenCandidates(allCandidates);
                          setCandidateCursor(candidatePage.next_cursor ?? null);
                        }}
                      >
                        Load more candidates
                      </button>
                    ) : null}
                    {batchPage.next_cursor ? (
                      <button
                        type="button"
                        onClick={() => {
                          setSeenBatches(allBatches);
                          setBatchCursor(batchPage.next_cursor ?? null);
                        }}
                      >
                        Load more batches
                      </button>
                    ) : null}
                  </div>
                </>
              );
            }}
          </Loaded>
        )}
      </Loaded>
    </>
  );
}

function LearningFilters({
  me,
  value,
  onChange,
  onApply,
  onClear,
}: {
  me: MeView;
  value: LearningsFilters;
  onChange: (next: Partial<LearningsFilters>) => void;
  onApply: () => void;
  onClear: () => void;
}) {
  return (
    <form
      className="filters learning-filters"
      onSubmit={(event) => {
        event.preventDefault();
        onApply();
      }}
    >
      <Field label="Project">
        <select value={value.projectId} onChange={(event) => onChange({ projectId: event.target.value })}>
          <option value="">All visible projects</option>
          {me.projects.map((project) => (
            <option value={project.id} key={project.id}>
              {project.display_name}
            </option>
          ))}
        </select>
      </Field>
      <Field label="Session ID">
        <input
          value={value.sessionId}
          placeholder="Exact session"
          onChange={(event) => onChange({ sessionId: event.target.value })}
        />
      </Field>
      <Field label="Decision state">
        <select
          value={value.state}
          onChange={(event) => onChange({ state: event.target.value as LearningsFilters["state"] })}
        >
          <option value="">Any state</option>
          {CANDIDATE_STATES.map((state) => (
            <option value={state} key={state}>
              {stateLabel(state)}
            </option>
          ))}
        </select>
      </Field>
      <button type="submit">Apply filters</button>
      {value.projectId || value.sessionId || value.state ? (
        <button type="button" onClick={onClear}>
          Clear
        </button>
      ) : null}
    </form>
  );
}

function BatchGroup({
  batch,
  candidates,
  allCandidates,
  me,
}: {
  batch: CaptureBatchView | null;
  candidates: CaptureCandidateView[];
  allCandidates: CaptureCandidateView[];
  me: MeView;
}) {
  const sessionId = batch?.session_id ?? candidates[0]?.session_id ?? "";
  const sourceKind = batch?.source_kind ?? candidates[0]?.source_kind ?? "session";
  const importJobId = batch?.import_job_id ?? candidates[0]?.import_job_id ?? null;
  const project = batch?.project_id
    ? me.projects.find((entry) => entry.id === batch.project_id)
    : null;
  const anchor = batch ? me.anchors.find((entry) => entry.scope_id === batch.scope_id) : null;
  const requiredAction = sourceKind === "okf_import" ? "knowledge.write" : "session.write";
  const mayDecide = anchor?.actions[requiredAction] === true;
  const diagnostics =
    sourceKind === "session" && anchor?.actions["session.diagnostics"] === true;
  return (
    <section className="learning-batch">
      <header>
        <div>
          <h2>{project ? project.display_name : sourceKind === "okf_import" ? "OKF import" : "Session capture"}</h2>
          <p className="muted">
            {sessionId ? (
              <Link href={hrefOf("session", { session_id: sessionId })}>
                Session {shortId(sessionId)}
              </Link>
            ) : (
              <>OKF import {shortId(importJobId ?? "unknown")}</>
            )}
            {batch ? ` · ${stateLabel(batch.state)} · ${whenOf(batch.created_at)}` : ""}
          </p>
        </div>
        {batch ? (
          <div className="batch-progress" aria-label="Batch progress">
            <strong>{batchProgress(batch, allCandidates)}</strong>
            <span className="muted">
              {batch.extractor_method ?? "extractor pending"}
              {batch.model_version ? ` · ${batch.model_version}` : ""}
            </span>
          </div>
        ) : null}
      </header>
      {!mayDecide && candidates.some((candidate) => candidate.state === "pending") ? (
        <div className="banner warning" role="status">
          You may read these candidates, but this source scope does not currently offer{" "}
          {requiredAction}, so no decision control is shown.
        </div>
      ) : null}
      {candidates.length === 0 ? (
        <p className="muted">This batch produced no candidates in the selected state.</p>
      ) : (
        <div className="learning-candidates">
          {candidates.map((candidate) => (
            <CandidateCard
              key={candidate.id}
              candidate={candidate}
              batch={batch}
              me={me}
              mayDecide={mayDecide}
              diagnostics={diagnostics}
            />
          ))}
        </div>
      )}
    </section>
  );
}

type DecisionAnswer = Answer<CaptureDecisionView>;

function CandidateCard({
  candidate,
  batch,
  me,
  mayDecide,
  diagnostics,
}: {
  candidate: CaptureCandidateView;
  batch: CaptureBatchView | null;
  me: MeView;
  mayDecide: boolean;
  diagnostics: boolean;
}) {
  const [current, setCurrent] = useState(candidate);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setCurrent(candidate), [candidate]);

  const scopes = useMemo(
    () =>
      publishableScopes(me, {
        projectId: batch?.project_id ?? current.proposed_project_id,
        sourceScopeId: batch?.scope_id ?? current.proposed_scope_id,
      }),
    [batch?.project_id, batch?.scope_id, current.proposed_project_id, current.proposed_scope_id, me],
  );
  const proposed = proposedPublishScope(current, scopes);
  const [scopeId, setScopeId] = useState(proposed?.id ?? scopes[0]?.id ?? "");
  useEffect(() => {
    if (!scopes.some((scope) => scope.id === scopeId)) {
      setScopeId(proposed?.id ?? scopes[0]?.id ?? "");
    }
  }, [proposed?.id, scopeId, scopes]);
  const target = scopes.find((scope) => scope.id === scopeId) ?? null;

  const perform = useCallback(async (name: string, promise: Promise<DecisionAnswer>) => {
    setBusy(name);
    setError(null);
    const answer = await promise;
    setBusy(null);
    if (answer.kind !== "ok") {
      setError(answer.kind === "unauthenticated" ? "Your session has expired." : answer.message);
      return;
    }
    setCurrent(answer.body.candidate);
    invalidate("learnings");
    if (answer.body.candidate.resulting_outcome === "applied") invalidate("knowledge");
  }, []);

  const pending = current.state === "pending";
  const message = decisionMessage(current);
  return (
    <article className="learning-card">
      <header>
        <div>
          <span className="eyebrow">{current.knowledge_type}</span>
          <h3>{current.content_erased ? "Erased candidate" : current.content.title}</h3>
        </div>
        <span className={`tag ${pending ? "warn" : "done"}`}>{stateLabel(current.state)}</span>
      </header>
      <p>{current.content.summary}</p>
      <div className="knowledge-body">{current.content.body_markdown}</div>
      <div className="learning-facts">
        <span>{candidateVisibility(current)}</span>
        <span>{current.content.sensitivity}</span>
        <span>{current.content.confidence_permille} / 1000 confidence</span>
        <span>{current.origin}</span>
      </div>
      <MatchBadges matches={current.matches} />
      <SourcePreview candidate={current} diagnostics={diagnostics} />
      <ExistingComparisons matches={current.matches} />

      {message ? (
        <div
          className={`banner ${current.resulting_outcome === "rejected" || current.state === "failed" ? "error" : "success"}`}
          role="status"
        >
          {message}{" "}
          {current.resulting_outcome === "pending_review" ? (
            <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
          ) : null}
          {current.resulting_outcome === "applied" && current.resulting_knowledge_item_id ? (
            <Link
              href={hrefOf("knowledge-item", {
                knowledge_id: current.resulting_knowledge_item_id,
              })}
            >
              Open resulting Knowledge
            </Link>
          ) : null}
        </div>
      ) : null}
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}

      {pending && mayDecide ? (
        <section className="learning-actions">
          <h4>Decide this learning</h4>
          {scopes.length === 0 ? (
            <div className="banner error">
              None of the private, project or workspace destinations for this run currently offers
              knowledge.write. Ask for a publishing role or dismiss the candidate.
            </div>
          ) : (
            <>
              {!proposed ? (
                <div className="banner warning" role="status">
                  The proposed scope is readable but not publishable by you. Choose one of the
                  destinations below; the unavailable scope is not offered.
                </div>
              ) : null}
              <Field label="Change scope / publish at">
                <select value={scopeId} onChange={(event) => setScopeId(event.target.value)}>
                  {scopes.map((scope) => (
                    <option key={scope.id} value={scope.id}>
                      {scope.label}
                    </option>
                  ))}
                </select>
              </Field>
              <p className="scope-explanation">{target ? scopeExplanation(target) : null}</p>
              <button
                type="button"
                disabled={!target || busy !== null}
                onClick={() => {
                  if (!target) return;
                  void perform(
                    "accept",
                    request("accept_capture_candidate", {
                      path: { id: current.id },
                      body: placementEdits(current, target),
                      idempotencyKey: idempotencyKey(),
                    }),
                  );
                }}
              >
                {busy === "accept" ? "Accepting…" : target && proposed?.id !== target.id ? "Change scope and accept" : "Accept"}
              </button>
              {target ? (
                <EditAndAccept
                  candidate={current}
                  target={target}
                  busy={busy !== null}
                  onSubmit={(body) =>
                    perform(
                      "edit",
                      request("accept_capture_candidate", {
                        path: { id: current.id },
                        body,
                        idempotencyKey: idempotencyKey(),
                      }),
                    )
                  }
                />
              ) : null}
              {target ? (
                <MergeAction
                  candidate={current}
                  target={target}
                  busy={busy !== null}
                  onSubmit={(body) =>
                    perform(
                      "merge",
                      request("merge_capture_candidate", {
                        path: { id: current.id },
                        body,
                        idempotencyKey: idempotencyKey(),
                      }),
                    )
                  }
                />
              ) : null}
              {target ? (
                <ReplaceAction
                  candidate={current}
                  target={target}
                  busy={busy !== null}
                  onSubmit={(body) =>
                    perform(
                      "replace",
                      request("replace_capture_candidate", {
                        path: { id: current.id },
                        body,
                        idempotencyKey: idempotencyKey(),
                      }),
                    )
                  }
                />
              ) : null}
            </>
          )}
          <DismissAction
            busy={busy !== null}
            onSubmit={(reason) =>
              perform(
                "dismiss",
                request("dismiss_capture_candidate", {
                  path: { id: current.id },
                  body: dismissBody(reason),
                  idempotencyKey: idempotencyKey(),
                }),
              )
            }
          />
        </section>
      ) : null}
    </article>
  );
}

function MatchBadges({ matches }: { matches: CaptureMatchView[] }) {
  if (matches.length === 0) {
    return <p className="muted">No policy-visible duplicate or conflict was found.</p>;
  }
  return (
    <div className="match-badges" aria-label="Candidate comparisons">
      {matches.map((match) => (
        <span className={`tag match-${match.kind}`} key={`${match.kind}-${match.knowledge_item_id}`}>
          {matchLabel(match.kind)} · {match.similarity_permille} / 1000
        </span>
      ))}
    </div>
  );
}

function SourcePreview({
  candidate,
  diagnostics,
}: {
  candidate: CaptureCandidateView;
  diagnostics: boolean;
}) {
  if (candidate.source_kind === "okf_import") {
    return (
      <section className="source-preview">
        <h4>Imported OKF provenance</h4>
        <p>
          {candidate.source_artifact_ids.length} immutable source artifact(s) from OKF import{" "}
          {shortId(candidate.import_job_id ?? "unknown")} support this candidate.
        </p>
        <p className="muted">
          Imported content remains review input until this candidate is accepted through the
          governed Knowledge lifecycle.
        </p>
      </section>
    );
  }
  if (!candidate.session_id) {
    return (
      <section className="source-preview">
        <h4>Source evidence</h4>
        <p className="muted">This candidate has no visible source reference.</p>
      </section>
    );
  }
  return (
    <SessionSourcePreview
      candidate={candidate}
      diagnostics={diagnostics}
      sessionId={candidate.session_id}
    />
  );
}

function SessionSourcePreview({
  candidate,
  diagnostics,
  sessionId,
}: {
  candidate: CaptureCandidateView;
  diagnostics: boolean;
  sessionId: string;
}) {
  const key = `sessions/timeline/${sessionId}`;
  const timeline = useQuery(key, () =>
    request("get_session_timeline", { path: { session_id: sessionId } }),
  );
  return (
    <section className="source-preview">
      <h4>Source conversation preview</h4>
      <Loaded<TimelineView> entry={timeline} what="source conversation" onRetry={useRefresh(key)}>
        {(body) => {
          const wanted = new Set(candidate.source_event_ids);
          const entries = body.entries.filter(
            (entry) => entry.kind === "event" && wanted.has(entry.id),
          );
          return (
            <>
              {entries.length === 0 ? (
                <p className="muted">
                  The exact event IDs are retained, but this bounded timeline page does not contain
                  their summaries. Open the session to inspect the evidence.
                </p>
              ) : (
                <ol className="source-events">
                  {entries.map((entry) => (
                    <SourceEntry
                      key={entry.id}
                      entry={entry}
                      sessionId={sessionId}
                      diagnostics={diagnostics}
                    />
                  ))}
                </ol>
              )}
              {entries.length < candidate.source_event_ids.length ? (
                <p className="muted">
                  {candidate.source_event_ids.length - entries.length} retained source event(s) are
                  outside this timeline projection.
                </p>
              ) : null}
              <p>
                <Link href={hrefOf("session", { session_id: sessionId })}>
                  Open the complete session timeline
                </Link>
              </p>
            </>
          );
        }}
      </Loaded>
    </section>
  );
}

function SourceEntry({
  entry,
  sessionId,
  diagnostics,
}: {
  entry: TimelineEntry;
  sessionId: string;
  diagnostics: boolean;
}) {
  const [open, setOpen] = useState(false);
  return (
    <li>
      <strong>{entry.event_type ?? "session event"}</strong> · {whenOf(entry.at)}
      <p>{entry.summary}</p>
      {diagnostics ? (
        open ? (
          <SourcePayload sessionId={sessionId} eventId={entry.id} />
        ) : (
          <button type="button" className="link" onClick={() => setOpen(true)}>
            Show authorised source payload
          </button>
        )
      ) : (
        <p className="muted">Raw source text requires session.diagnostics at this run.</p>
      )}
    </li>
  );
}

function SourcePayload({ sessionId, eventId }: { sessionId: string; eventId: string }) {
  const key = `sessions/event/${sessionId}/${eventId}`;
  const event = useQuery(key, () =>
    request("get_session_event", { path: { session_id: sessionId, event_id: eventId } }),
  );
  return (
    <Loaded<SessionEventView> entry={event} what="source payload" onRetry={useRefresh(key)}>
      {(body) => (
        <div className="diagnostic">
          <p className="muted">
            Exact event {body.id} · blake3 {body.payload_hash.slice(0, 16)}…
          </p>
          <pre>{JSON.stringify(body.payload, null, 2)}</pre>
        </div>
      )}
    </Loaded>
  );
}

function ExistingComparisons({ matches }: { matches: CaptureMatchView[] }) {
  if (matches.length === 0) return null;
  return (
    <section className="existing-comparisons">
      <h4>Compare with existing Knowledge</h4>
      <div className="comparison-grid">
        {matches.map((match) => (
          <ExistingComparison key={`${match.kind}-${match.knowledge_item_id}`} match={match} />
        ))}
      </div>
    </section>
  );
}

function ExistingComparison({ match }: { match: CaptureMatchView }) {
  const key = `knowledge/item/${match.knowledge_item_id}`;
  const item = useQuery(key, () =>
    request("get_knowledge", { path: { id: match.knowledge_item_id } }),
  );
  return (
    <Loaded<KnowledgeItemView> entry={item} what="existing Knowledge" onRetry={useRefresh(key)}>
      {(body) => (
        <article className="comparison-card">
          <span className={`tag match-${match.kind}`}>{matchLabel(match.kind)}</span>
          <h5>
            <Link href={hrefOf("knowledge-item", { knowledge_id: body.id })}>
              {body.current_revision.title}
            </Link>
          </h5>
          <p>{body.current_revision.summary}</p>
          <p className="muted">
            Revision {body.current_revision.revision_number} · {match.similarity_permille} / 1000
            · {match.reason_code}
          </p>
        </article>
      )}
    </Loaded>
  );
}

function EditAndAccept({
  candidate,
  target,
  busy,
  onSubmit,
}: {
  candidate: CaptureCandidateView;
  target: PublishScope;
  busy: boolean;
  onSubmit: (body: ReturnType<typeof editAndAcceptBody>) => Promise<void>;
}) {
  const [knowledgeType, setKnowledgeType] = useState(candidate.knowledge_type);
  const [title, setTitle] = useState(candidate.content.title);
  const [summary, setSummary] = useState(candidate.content.summary);
  const [body, setBody] = useState(candidate.content.body_markdown);
  const [tags, setTags] = useState((candidate.content.tags ?? []).join(", "));
  const [sensitivity, setSensitivity] = useState(candidate.content.sensitivity);
  return (
    <details className="candidate-action">
      <summary>Edit and accept</summary>
      <form
        className="stacked-form"
        onSubmit={(event) => {
          event.preventDefault();
          const content: KnowledgeContentBody = {
            ...candidate.content,
            title,
            summary,
            body_markdown: body,
            tags: splitTags(tags),
            sensitivity,
          };
          void onSubmit(editAndAcceptBody(candidate, target, { knowledgeType, content }));
        }}
      >
        <Field label="Type">
          <select value={knowledgeType} onChange={(event) => setKnowledgeType(event.target.value)}>
            {KNOWLEDGE_TYPES.map((kind) => (
              <option value={kind} key={kind}>
                {kind}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Title">
          <input required value={title} onChange={(event) => setTitle(event.target.value)} />
        </Field>
        <Field label="Summary">
          <input required value={summary} onChange={(event) => setSummary(event.target.value)} />
        </Field>
        <Field label="Markdown body">
          <textarea required rows={6} value={body} onChange={(event) => setBody(event.target.value)} />
        </Field>
        <Field label="Tags">
          <input value={tags} onChange={(event) => setTags(event.target.value)} />
        </Field>
        <Field label="Sensitivity">
          <select
            value={sensitivity}
            onChange={(event) =>
              setSensitivity(event.target.value as KnowledgeContentBody["sensitivity"])
            }
          >
            {SENSITIVITIES.map((value) => (
              <option value={value} key={value}>
                {value}
              </option>
            ))}
          </select>
        </Field>
        <button type="submit" disabled={busy}>
          Edit and accept
        </button>
      </form>
    </details>
  );
}

function MergeAction({
  candidate,
  target,
  busy,
  onSubmit,
}: {
  candidate: CaptureCandidateView;
  target: PublishScope;
  busy: boolean;
  onSubmit: (body: ReturnType<typeof mergeBody>) => Promise<void>;
}) {
  const [matchId, setMatchId] = useState(candidate.matches[0]?.knowledge_item_id ?? "");
  const matched = candidate.matches.find((match) => match.knowledge_item_id === matchId) ?? null;
  return (
    <details className="candidate-action">
      <summary>Merge with existing</summary>
      {candidate.matches.length === 0 ? (
        <p className="muted">No policy-visible current Knowledge input was suggested.</p>
      ) : (
        <form
          className="stacked-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (matched) void onSubmit(mergeBody(candidate, target, matched));
          }}
        >
          <MatchSelect matches={candidate.matches} value={matchId} onChange={setMatchId} />
          <p className="muted">
            The candidate's session-event provenance and the selected item's provenance are both
            retained. Its exact revision is a stale-write precondition.
          </p>
          <button type="submit" disabled={busy || !matched}>
            Merge with existing
          </button>
        </form>
      )}
    </details>
  );
}

function ReplaceAction({
  candidate,
  target,
  busy,
  onSubmit,
}: {
  candidate: CaptureCandidateView;
  target: PublishScope;
  busy: boolean;
  onSubmit: (body: ReturnType<typeof replaceBody>) => Promise<void>;
}) {
  const preferred =
    candidate.matches.find((match) => match.kind === "supersession") ??
    candidate.matches.find((match) => match.kind === "transition") ??
    candidate.matches.find((match) => match.kind === "contradiction") ??
    candidate.matches[0];
  const [matchId, setMatchId] = useState(preferred?.knowledge_item_id ?? "");
  const matched = candidate.matches.find((match) => match.knowledge_item_id === matchId) ?? null;
  return (
    <details className="candidate-action">
      <summary>Replace existing</summary>
      {candidate.matches.length === 0 ? (
        <p className="muted">No policy-visible current Knowledge item was suggested.</p>
      ) : (
        <form
          className="stacked-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (matched) void onSubmit(replaceBody(candidate, target, matched));
          }}
        >
          <MatchSelect matches={candidate.matches} value={matchId} onChange={setMatchId} />
          <p className="muted">
            Replace creates an explicit governed supersession. The existing item and every revision
            remain in history; nothing is deleted.
          </p>
          <button type="submit" disabled={busy || !matched}>
            Replace existing
          </button>
        </form>
      )}
    </details>
  );
}

function DismissAction({
  busy,
  onSubmit,
}: {
  busy: boolean;
  onSubmit: (reason: string) => Promise<void>;
}) {
  const [reason, setReason] = useState("");
  return (
    <details className="candidate-action">
      <summary>Dismiss</summary>
      <form
        className="stacked-form"
        onSubmit={(event) => {
          event.preventDefault();
          void onSubmit(reason);
        }}
      >
        <Field label="Reason (optional)">
          <input value={reason} maxLength={1000} onChange={(event) => setReason(event.target.value)} />
        </Field>
        <p className="muted">Dismissal records the decision and publishes no Knowledge.</p>
        <button type="submit" className="danger" disabled={busy}>
          Dismiss
        </button>
      </form>
    </details>
  );
}

function MatchSelect({
  matches,
  value,
  onChange,
}: {
  matches: CaptureMatchView[];
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <Field label="Existing Knowledge">
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {matches.map((match) => (
          <option value={match.knowledge_item_id} key={match.knowledge_item_id}>
            {matchLabel(match.kind)} · {shortId(match.knowledge_item_id)} · {match.similarity_permille} / 1000
          </option>
        ))}
      </select>
    </Field>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label>
      <span className="switcher-label">{label}</span>
      {children}
    </label>
  );
}

function scopeExplanation(scope: PublishScope): string {
  switch (scope.visibility) {
    case "private":
      return "Private to your principal scope. Teammates cannot read it through the project.";
    case "project":
      return "Shared with this project and policy-visible to teammates who can read it.";
    case "workspace":
      return "Shared at workspace scope, outside any single project.";
  }
}

function splitTags(raw: string): string[] {
  return [...new Set(raw.split(",").map((tag) => tag.trim().toLowerCase()).filter(Boolean))].sort();
}

function shortId(id: string): string {
  return id.length <= 12 ? id : `${id.slice(0, 8)}…`;
}

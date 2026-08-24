/**
 * Knowledge Browser (CPR-17, ADR-0082).
 *
 * The stable item and its immutable revisions are the only product model on
 * this page. Reads use the generated contract, every write returns the
 * VedaFlow change that governed it, and source/history/usage disclosures are
 * separate calls because the PDP prices them separately at the gateway.
 */

import { useCallback, useEffect, useMemo, useState } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link, navigate } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import {
  EMPTY_KNOWLEDGE_FILTERS,
  KNOWLEDGE_TYPES,
  LIFECYCLES,
  ORIGINS,
  SENSITIVITIES,
  SOURCE_TYPES,
  knowledgeIsFiltered,
  knowledgeQuery,
  mutationMessage,
  visibilityLabel,
  type KnowledgeFilters,
} from "./knowledge.mjs";
import type {
  CreateKnowledgeBody,
  KnowledgeContentBody,
  KnowledgeHistoryView,
  KnowledgeItemView,
  KnowledgeListView,
  KnowledgeMutationView,
  KnowledgeSourcesView,
  KnowledgeUsageListView,
  MergeKnowledgeBody,
  SupersedeKnowledgeBody,
} from "./generated/api.js";

type ScopeOption = {
  id: string;
  label: string;
  owner?: string;
  projectId?: string;
};

export function Knowledge() {
  const { me, project, workspace } = useApp();
  const initial = useMemo(
    () => ({
      ...EMPTY_KNOWLEDGE_FILTERS,
      workspaceId: project ? "" : (workspace?.id ?? ""),
      projectId: project?.id ?? "",
    }),
    [project?.id, workspace?.id],
  );
  const [draft, setDraft] = useState<KnowledgeFilters>(initial);
  const [filters, setFilters] = useState<KnowledgeFilters>(initial);
  const [seen, setSeen] = useState<KnowledgeItemView[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    setDraft(initial);
    setFilters(initial);
    setSeen([]);
    setCursor(null);
  }, [initial]);

  const query = knowledgeQuery(filters, cursor);
  const key = `knowledge/list/${JSON.stringify(query)}`;
  const entry = useQuery(key, () => request("list_knowledge", { query }));
  const retry = useRefresh(key);
  const scopes = writableScopes(me, workspace, project);

  const applyFilters = useCallback(() => {
    setSeen([]);
    setCursor(null);
    setFilters({ ...draft });
  }, [draft]);

  return (
    <>
      <PageHeading route="knowledge" />
      <div className="knowledge-toolbar">
        <button type="button" onClick={() => setCreating((value) => !value)}>
          {creating ? "Close new item" : "Add Knowledge"}
        </button>
      </div>
      {creating ? (
        <CreateForm
          scopes={scopes}
          onCreated={(result) => {
            invalidate("knowledge");
            if (result.outcome === "applied" && result.knowledge_item_id) {
              navigate(hrefOf("knowledge-item", { knowledge_id: result.knowledge_item_id }));
            }
          }}
        />
      ) : null}
      <KnowledgeFilterBar
        filters={draft}
        onChange={(next) => setDraft((current) => ({ ...current, ...next }))}
        onApply={applyFilters}
        onClear={() => {
          setDraft(EMPTY_KNOWLEDGE_FILTERS);
          setFilters(EMPTY_KNOWLEDGE_FILTERS);
          setSeen([]);
          setCursor(null);
        }}
      />
      <Loaded<KnowledgeListView> entry={entry} what="Knowledge" onRetry={retry}>
        {(body) => {
          const rows = appendItems(seen, body.items);
          return (
            <>
              <SearchMode body={body} />
              {rows.length === 0 ? (
                <p className="muted">
                  {knowledgeIsFiltered(filters)
                    ? "No visible current Knowledge matches these filters."
                    : "No visible current Knowledge has been published yet."}
                </p>
              ) : (
                <ul className="knowledge-list">
                  {rows.map((item) => (
                    <KnowledgeRow key={item.id} item={item} />
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
                    The cursor advances over candidates the PDP considered, including rows it did
                    not disclose.
                  </span>
                </p>
              ) : rows.length > 0 ? (
                <p className="muted">That is every matching item this policy lets you read.</p>
              ) : null}
            </>
          );
        }}
      </Loaded>
    </>
  );
}

export function KnowledgeItem({ knowledgeId }: { knowledgeId: string }) {
  const detailKey = `knowledge/item/${knowledgeId}`;
  const entry = useQuery(detailKey, () =>
    request("get_knowledge", { path: { id: knowledgeId } }),
  );
  const retry = useRefresh(detailKey);
  return (
    <>
      <PageHeading route="knowledge-item" />
      <p>
        <Link href={hrefOf("knowledge")}>← Knowledge Browser</Link>
      </p>
      <Loaded<KnowledgeItemView> entry={entry} what="this Knowledge item" onRetry={retry}>
        {(item) => <KnowledgeDetail key={item.current_revision.id} item={item} />}
      </Loaded>
    </>
  );
}

function SearchMode({ body }: { body: KnowledgeListView }) {
  if (body.retrieval_mode === "listing" && !body.degradation) return null;
  return (
    <p className="muted search-mode" role="status">
      Retrieval: {body.retrieval_mode}
      {body.degradation ? ` · ${body.degradation.replaceAll("_", " ")}` : ""}
    </p>
  );
}

export function KnowledgeRow({ item }: { item: KnowledgeItemView }) {
  const revision = item.current_revision;
  return (
    <li>
      <Link href={hrefOf("knowledge-item", { knowledge_id: item.id })} className="row">
        <strong>{revision.title}</strong>{" "}
        <span className={`tag ${item.lifecycle_state === "active" ? "done" : "warn"}`}>
          {item.lifecycle_state}
        </span>
        <p>{revision.summary}</p>
        <div className="muted">
          {item.knowledge_type} · {visibilityLabel(item)} · revision {revision.revision_number} ·{" "}
          {whenOf(item.updated_at)}
          {item.match_score == null ? "" : ` · score ${item.match_score.toFixed(4)}`}
        </div>
        <TagList tags={revision.tags} />
      </Link>
    </li>
  );
}

function KnowledgeDetail({ item }: { item: KnowledgeItemView }) {
  const revision = item.current_revision;
  const historyKey = `knowledge/item/${item.id}/history`;
  const sourcesKey = `knowledge/item/${item.id}/sources`;
  const usageKey = `knowledge/item/${item.id}/usage`;
  const history = useQuery(historyKey, () =>
    request("get_knowledge_history", { path: { id: item.id }, query: { limit: "200" } }),
  );
  const sources = useQuery(sourcesKey, () =>
    request("get_knowledge_sources", { path: { id: item.id } }),
  );
  const usage = useQuery(usageKey, () =>
    request("get_knowledge_usage", { path: { id: item.id }, query: { limit: "200" } }),
  );
  const [result, setResult] = useState<KnowledgeMutationView | null>(null);

  const settled = (next: KnowledgeMutationView) => {
    setResult(next);
    if (next.outcome === "applied") invalidate("knowledge");
  };

  return (
    <article className="knowledge-detail">
      <header>
        <h2>{revision.title}</h2>
        <p className="muted">
          {item.knowledge_type} · {item.origin} · {item.lifecycle_state} · {visibilityLabel(item)}
        </p>
        <TagList tags={revision.tags} />
      </header>
      {result ? (
        <div className={`banner ${result.outcome === "rejected" ? "error" : "success"}`} role="status">
          {mutationMessage(result)}{" "}
          {result.outcome === "pending_review" ? (
            <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
          ) : null}
        </div>
      ) : null}
      <section>
        <h3>Current content</h3>
        <div className="knowledge-body">{revision.body_markdown}</div>
        <dl className="facts">
          <dt>Summary</dt>
          <dd>{revision.summary}</dd>
          <dt>Revision</dt>
          <dd className="mono breakable">{revision.id}</dd>
          <dt>Content hash</dt>
          <dd className="mono breakable">{revision.content_hash}</dd>
          <dt>Sensitivity</dt>
          <dd>{revision.sensitivity}</dd>
          <dt>Confidence</dt>
          <dd>{revision.confidence_permille} / 1000</dd>
          <dt>Valid time</dt>
          <dd>
            {whenOf(revision.valid_from)} → {revision.valid_to ? whenOf(revision.valid_to) : "open"}
          </dd>
          <dt>Verification due</dt>
          <dd>{revision.stale_after ? whenOf(revision.stale_after) : "not scheduled"}</dd>
          <dt>Transaction time</dt>
          <dd>{whenOf(revision.transaction_time)}</dd>
        </dl>
      </section>
      <section className="knowledge-columns">
        <Panel title="Revision history">
          <Loaded<KnowledgeHistoryView> entry={history} what="revision history">
            {(body) =>
              body.revisions.length === 0 ? (
                <p className="muted">No visible revisions.</p>
              ) : (
                <ol className="revision-history">
                  {body.revisions.map((entry) => (
                    <li key={entry.id}>
                      <strong>Revision {entry.revision_number}</strong> · {whenOf(entry.transaction_time)}
                      <p>{entry.summary}</p>
                      <span className="mono breakable">{entry.content_hash}</span>
                    </li>
                  ))}
                </ol>
              )
            }
          </Loaded>
        </Panel>
        <Panel title="Provenance">
          <Loaded<KnowledgeSourcesView> entry={sources} what="provenance">
            {(body) =>
              body.sources.length === 0 ? (
                <p className="muted">No source descriptor is visible under this policy.</p>
              ) : (
                <ul>
                  {body.sources.map((source) => (
                    <li key={source.id}>
                      <strong>{source.source_type}</strong> · scope {source.scope_id}
                      {source.locator ? <div className="breakable">{source.locator}</div> : null}
                      {source.source_revision ? <div>Revision {source.source_revision}</div> : null}
                      {source.session_event_id ? (
                        <div className="mono breakable">Event {source.session_event_id}</div>
                      ) : null}
                    </li>
                  ))}
                </ul>
              )
            }
          </Loaded>
        </Panel>
        <Panel title="Usage">
          <Loaded<KnowledgeUsageListView> entry={usage} what="usage history">
            {(body) =>
              body.usages.length === 0 ? (
                <p className="muted">
                  No context selection has used this revision. Context-run selections begin with
                  the explainable planner.
                </p>
              ) : (
                <ul>
                  {body.usages.map((entry) => (
                    <li key={`${entry.context_run_id}-${entry.revision_id}`}>
                      {whenOf(entry.selected_at)} · {entry.reason_codes.join(", ")}
                      <div className="mono breakable">{entry.context_run_id}</div>
                    </li>
                  ))}
                </ul>
              )
            }
          </Loaded>
        </Panel>
        <Panel title="Relationships">
          {(item.relationships ?? []).length === 0 ? (
            <p className="muted">No visible relationships.</p>
          ) : (
            <ul>
              {(item.relationships ?? []).map((relation) => {
                const other =
                  relation.source_item_id === item.id
                    ? relation.target_item_id
                    : relation.source_item_id;
                return (
                  <li key={relation.id}>
                    {relation.relation_type.replaceAll("_", " ")} ·{" "}
                    <Link href={hrefOf("knowledge-item", { knowledge_id: other })}>{other}</Link>
                  </li>
                );
              })}
            </ul>
          )}
        </Panel>
      </section>
      <section>
        <h3>Governed actions</h3>
        <p className="muted">
          Every action below creates a VedaFlow change. A permissive profile may apply it
          immediately; a stricter one leaves it in Advanced Reviews.
        </p>
        <EditForm item={item} onSettled={settled} />
        <VerifyForm item={item} onSettled={settled} />
        <SupersedeForm item={item} onSettled={settled} />
        <MergeForm item={item} onSettled={settled} />
        <LifecycleForms item={item} onSettled={settled} />
      </section>
    </article>
  );
}

function KnowledgeFilterBar({
  filters,
  onChange,
  onApply,
  onClear,
}: {
  filters: KnowledgeFilters;
  onChange: (next: Partial<KnowledgeFilters>) => void;
  onApply: () => void;
  onClear: () => void;
}) {
  return (
    <form
      className="filters knowledge-filters"
      onSubmit={(event) => {
        event.preventDefault();
        onApply();
      }}
    >
      <Field label="Search">
        <input
          aria-label="Search"
          value={filters.query}
          placeholder="webhook retry"
          onChange={(event) => onChange({ query: event.target.value })}
        />
      </Field>
      <Select label="Type" value={filters.knowledgeType} values={KNOWLEDGE_TYPES} onChange={(value) => onChange({ knowledgeType: value })} />
      <Select label="Origin" value={filters.origin} values={ORIGINS} onChange={(value) => onChange({ origin: value })} />
      <Select label="State" value={filters.lifecycle} values={LIFECYCLES} onChange={(value) => onChange({ lifecycle: value })} />
      <Select label="Source" value={filters.source} values={SOURCE_TYPES} onChange={(value) => onChange({ source: value })} />
      <Field label="Tag">
        <input value={filters.tag} onChange={(event) => onChange({ tag: event.target.value })} />
      </Field>
      <Field label="Owner">
        <input value={filters.owner} onChange={(event) => onChange({ owner: event.target.value })} />
      </Field>
      <Field label="Scope ID">
        <input value={filters.scopeId} onChange={(event) => onChange({ scopeId: event.target.value })} />
      </Field>
      <Field label="Workspace ID">
        <input value={filters.workspaceId} onChange={(event) => onChange({ workspaceId: event.target.value })} />
      </Field>
      <Field label="Project ID">
        <input value={filters.projectId} onChange={(event) => onChange({ projectId: event.target.value })} />
      </Field>
      <Field label="Updated from">
        <input type="datetime-local" value={filters.updatedFrom} onChange={(event) => onChange({ updatedFrom: event.target.value })} />
      </Field>
      <Field label="Updated before">
        <input type="datetime-local" value={filters.updatedBefore} onChange={(event) => onChange({ updatedBefore: event.target.value })} />
      </Field>
      <Select label="Staleness" value={filters.stale} values={["true", "false"] as const} onChange={(value) => onChange({ stale: value as KnowledgeFilters["stale"] })} />
      <button type="submit">Search</button>
      {knowledgeIsFiltered(filters) ? (
        <button type="button" onClick={onClear}>Clear</button>
      ) : null}
    </form>
  );
}

function CreateForm({ scopes, onCreated }: { scopes: ScopeOption[]; onCreated: (result: KnowledgeMutationView) => void }) {
  const [scopeId, setScopeId] = useState(scopes[0]?.id ?? "");
  const [kind, setKind] = useState<(typeof KNOWLEDGE_TYPES)[number]>("fact");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [sensitivity, setSensitivity] = useState<(typeof SENSITIVITIES)[number]>("internal");
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const selected = scopes.find((scope) => scope.id === scopeId);

  return (
    <form className="stacked-form knowledge-create" onSubmit={async (event) => {
      event.preventDefault();
      if (!selected) return;
      setBusy(true);
      setStatus(null);
      const content = contentBody(title, body, sensitivity);
      const payload: CreateKnowledgeBody = {
        scope_id: selected.id,
        project_id: selected.projectId,
        owner_principal_id: selected.owner,
        knowledge_type: kind,
        origin: "authored",
        content,
      };
      const answer = await request("create_knowledge", { body: payload, idempotencyKey: idempotencyKey() });
      setBusy(false);
      if (answer.kind === "ok") {
        setStatus(mutationMessage(answer.body));
        onCreated(answer.body);
      } else setStatus(failureMessage(answer));
    }}>
      <h2>Add Knowledge</h2>
      {scopes.length === 0 ? <div className="banner error">No scope currently offers knowledge.write.</div> : null}
      <Field label="Publish at">
        <select value={scopeId} onChange={(event) => setScopeId(event.target.value)} required>
          {scopes.map((scope) => <option key={scope.id} value={scope.id}>{scope.label}</option>)}
        </select>
      </Field>
      <Select label="Type" value={kind} values={KNOWLEDGE_TYPES} allowEmpty={false} onChange={(value) => setKind(value as typeof kind)} />
      <Field label="Title"><input required value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
      <Field label="Markdown body"><textarea required rows={7} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
      <Select label="Sensitivity" value={sensitivity} values={SENSITIVITIES} allowEmpty={false} onChange={(value) => setSensitivity(value as typeof sensitivity)} />
      <button type="submit" disabled={busy || !selected}>{busy ? "Submitting…" : "Create governed change"}</button>
      {status ? <p role="status">{status}</p> : null}
    </form>
  );
}

function EditForm({ item, onSettled }: ActionProps) {
  const revision = item.current_revision;
  const [title, setTitle] = useState(revision.title);
  const [body, setBody] = useState(revision.body_markdown);
  const [summary, setSummary] = useState(revision.summary);
  const [tags, setTags] = useState(revision.tags.join(", "));
  return <Action title="Edit" submit="Create revision" run={() => request("edit_knowledge", {
    path: { id: item.id }, idempotencyKey: idempotencyKey(), body: {
      expected_revision_id: revision.id,
      content: { ...copyContent(revision), title, body_markdown: body, summary, tags: splitTags(tags) },
    },
  })} onSettled={onSettled}>
    <Field label="Title"><input value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
    <Field label="Summary"><input value={summary} onChange={(event) => setSummary(event.target.value)} /></Field>
    <Field label="Markdown body"><textarea rows={7} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
    <Field label="Tags"><input value={tags} onChange={(event) => setTags(event.target.value)} /></Field>
  </Action>;
}

function VerifyForm({ item, onSettled }: ActionProps) {
  const [note, setNote] = useState("");
  return <Action title="Verify" submit="Record verification" run={() => request("verify_knowledge", {
    path: { id: item.id }, idempotencyKey: idempotencyKey(), body: {
      expected_revision_id: item.current_revision.id,
      verification_metadata: { method: "console-review", note, verified_at: new Date().toISOString() },
    },
  })} onSettled={onSettled}>
    <Field label="Verification note"><input required value={note} onChange={(event) => setNote(event.target.value)} /></Field>
  </Action>;
}

function SupersedeForm({ item, onSettled }: ActionProps) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  return <Action title="Supersede" submit="Propose replacement" run={() => {
    const payload: SupersedeKnowledgeBody = {
      expected_revision_id: item.current_revision.id,
      scope_id: item.scope_id,
      project_id: item.project_id,
      owner_principal_id: item.owner_principal_id,
      knowledge_type: item.knowledge_type,
      origin: "authored",
      content: contentBody(title, body, item.current_revision.sensitivity),
    };
    return request("supersede_knowledge", { path: { id: item.id }, body: payload, idempotencyKey: idempotencyKey() });
  }} onSettled={onSettled}>
    <p className="muted">Creates a replacement item and an explicit supersedes relationship; this history remains.</p>
    <Field label="Replacement title"><input required value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
    <Field label="Replacement body"><textarea required rows={5} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
  </Action>;
}

function MergeForm({ item, onSettled }: ActionProps) {
  const [otherItem, setOtherItem] = useState("");
  const [otherRevision, setOtherRevision] = useState("");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  return <Action title="Merge" submit="Propose merge" run={() => {
    const payload: MergeKnowledgeBody = {
      inputs: [
        { item_id: item.id, revision_id: item.current_revision.id },
        { item_id: otherItem, revision_id: otherRevision },
      ],
      scope_id: item.scope_id,
      project_id: item.project_id,
      owner_principal_id: item.owner_principal_id,
      knowledge_type: item.knowledge_type,
      origin: "authored",
      content: contentBody(title, body, item.current_revision.sensitivity),
    };
    return request("merge_knowledge", { body: payload, idempotencyKey: idempotencyKey() });
  }} onSettled={onSettled}>
    <p className="muted">Both current revision IDs are preconditions; all input provenance is retained.</p>
    <Field label="Other item ID"><input required value={otherItem} onChange={(event) => setOtherItem(event.target.value)} /></Field>
    <Field label="Other current revision ID"><input required value={otherRevision} onChange={(event) => setOtherRevision(event.target.value)} /></Field>
    <Field label="Merged title"><input required value={title} onChange={(event) => setTitle(event.target.value)} /></Field>
    <Field label="Merged body"><textarea required rows={5} value={body} onChange={(event) => setBody(event.target.value)} /></Field>
  </Action>;
}

function LifecycleForms({ item, onSettled }: ActionProps) {
  const [reason, setReason] = useState("");
  const [confirmForget, setConfirmForget] = useState(false);
  const lifecycle = item.lifecycle_state === "archived" ? "restore" : "archive";
  return <details className="knowledge-action">
    <summary>Archive, restore or forget</summary>
    <div className="stacked-form">
      <Field label="Reason"><input required value={reason} onChange={(event) => setReason(event.target.value)} /></Field>
      <MutationButton label={lifecycle === "archive" ? "Archive" : "Restore"} run={() => request(
        lifecycle === "archive" ? "archive_knowledge" : "restore_knowledge",
        { path: { id: item.id }, idempotencyKey: idempotencyKey(), body: { expected_revision_id: item.current_revision.id, reason } },
      )} onSettled={onSettled} disabled={!reason.trim()} />
      <label className="choice"><input type="checkbox" checked={confirmForget} onChange={(event) => setConfirmForget(event.target.checked)} /> Permanently erase plaintext content and embeddings when policy allows. Audit retains only a content-free tombstone.</label>
      <MutationButton label="Forget" danger run={() => request("delete_knowledge", {
        path: { id: item.id }, idempotencyKey: idempotencyKey(), body: {
          mode: "forget", expected_revision_id: item.current_revision.id, reason,
        },
      })} onSettled={onSettled} disabled={!confirmForget || !reason.trim()} />
    </div>
  </details>;
}

type MutationAnswer = Answer<KnowledgeMutationView>;
type ActionProps = { item: KnowledgeItemView; onSettled: (result: KnowledgeMutationView) => void };

function Action({ title, submit, run, onSettled, children }: {
  title: string;
  submit: string;
  run: () => Promise<MutationAnswer>;
  onSettled: (result: KnowledgeMutationView) => void;
  children: React.ReactNode;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return <details className="knowledge-action">
    <summary>{title}</summary>
    <form className="stacked-form" onSubmit={async (event) => {
      event.preventDefault(); setBusy(true); setError(null);
      const answer = await run(); setBusy(false);
      if (answer.kind === "ok") onSettled(answer.body); else setError(failureMessage(answer));
    }}>
      {children}
      <button type="submit" disabled={busy}>{busy ? "Submitting…" : submit}</button>
      {error ? <p className="form-error" role="alert">{error}</p> : null}
    </form>
  </details>;
}

function MutationButton({ label, run, onSettled, disabled = false, danger = false }: {
  label: string; run: () => Promise<MutationAnswer>; onSettled: (result: KnowledgeMutationView) => void;
  disabled?: boolean; danger?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return <div>
    <button type="button" className={danger ? "danger" : undefined} disabled={disabled || busy} onClick={async () => {
      setBusy(true); setError(null); const answer = await run(); setBusy(false);
      if (answer.kind === "ok") onSettled(answer.body); else setError(failureMessage(answer));
    }}>{busy ? "Submitting…" : label}</button>
    {error ? <p className="form-error" role="alert">{error}</p> : null}
  </div>;
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <label><span className="switcher-label">{label}</span>{children}</label>;
}

function Select<T extends readonly string[]>({ label, value, values, onChange, allowEmpty = true }: {
  label: string; value: string; values: T; onChange: (value: T[number] | "") => void;
  allowEmpty?: boolean;
}) {
  return <Field label={label}><select value={value} onChange={(event) => onChange(event.target.value as T[number] | "")}>
    {allowEmpty ? <option value="">Any</option> : null}
    {values.map((entry) => <option key={entry} value={entry}>{entry.replaceAll("_", " ")}</option>)}
  </select></Field>;
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return <section><h3>{title}</h3>{children}</section>;
}

function TagList({ tags }: { tags: string[] }) {
  if (tags.length === 0) return null;
  return <span className="tag-list">{tags.map((tag) => <span className="tag" key={tag}>{tag}</span>)}</span>;
}

function contentBody(title: string, body: string, sensitivity: KnowledgeContentBody["sensitivity"]): KnowledgeContentBody {
  return { title, body_markdown: body, summary: body, sensitivity, confidence_permille: 900, tags: [] };
}

function copyContent(revision: KnowledgeItemView["current_revision"]): KnowledgeContentBody {
  return {
    title: revision.title,
    body_markdown: revision.body_markdown,
    summary: revision.summary,
    tags: revision.tags,
    sensitivity: revision.sensitivity,
    confidence_permille: revision.confidence_permille,
    valid_from: revision.valid_from,
    valid_to: revision.valid_to,
    stale_after: revision.stale_after,
    verification_metadata: revision.verification_metadata,
    metadata: revision.metadata,
  };
}

function splitTags(raw: string): string[] {
  return raw.split(",").map((tag) => tag.trim()).filter(Boolean);
}

function failureMessage(answer: Exclude<MutationAnswer, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

function appendItems(seen: KnowledgeItemView[], next: KnowledgeItemView[]): KnowledgeItemView[] {
  const rows = new Map(seen.map((item) => [item.id, item]));
  for (const item of next) rows.set(item.id, item);
  return [...rows.values()];
}

function writableScopes(
  me: ReturnType<typeof useApp>["me"],
  workspace: ReturnType<typeof useApp>["workspace"],
  project: ReturnType<typeof useApp>["project"],
): ScopeOption[] {
  const candidates: ScopeOption[] = [
    ...me.anchors
      .filter((anchor) => anchor.source === "principal_scope")
      .map((anchor) => ({
        id: anchor.scope_id,
        label: "Private to me",
        owner: me.principal.subject,
      })),
    ...(workspace ? [{ id: workspace.scope_id, label: `Workspace · ${workspace.display_name}` }] : []),
    ...(project ? [{ id: project.scope_id, label: `Project · ${project.display_name}`, projectId: project.id }] : []),
  ];
  return candidates.filter((candidate, index) => {
    if (candidates.findIndex((value) => value.id === candidate.id) !== index) return false;
    const anchor = me.anchors.find((value) => value.scope_id === candidate.id);
    return anchor?.actions["knowledge.write"] === true;
  });
}

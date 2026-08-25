/** Project OKF v0.2 import/export workflow (CPR-28, ADR-0087). */

import { useState, type FormEvent } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { hrefOf } from "./routes.mjs";
import { whenOf } from "./people.mjs";
import {
  classificationCounts,
  importBody,
  importProgress,
  OKF_SPEC_COMMIT,
  OKF_VERSION,
} from "./okf.mjs";
import type {
  KnowledgeListView,
  OkfExportView,
  OkfImportJobListView,
  OkfImportPlanView,
  OkfMaterializationView,
  ProjectView,
} from "./generated/api.js";

type Notice = { error?: string; message?: string };

export function OkfExchange() {
  const { me, project } = useApp();
  const [selectedJob, setSelectedJob] = useState<string | null>(null);

  return (
    <>
      <PageHeading route="okf" />
      <p>
        Validate and exchange portable Knowledge through the pinned Open Knowledge Format v
        {OKF_VERSION}. Imports stop as reviewable New Learnings; they never publish Knowledge
        directly.
      </p>
      <div className="banner warning">
        Bundle files are inert data. Synveda does not run scripts, fetch source URLs, follow
        symlinks or synchronise a Git repository. The specification is pinned to commit{" "}
        <code>{OKF_SPEC_COMMIT}</code>.
      </div>
      {project === null ? (
        <p className="muted">Select a project before importing or exporting OKF.</p>
      ) : (
        <ProjectExchange
          project={project}
          canWrite={mayAt(me.anchors, project.scope_id, "knowledge.write")}
          canRead={mayAt(me.anchors, project.scope_id, "knowledge.read")}
          selectedJob={selectedJob}
          selectJob={setSelectedJob}
        />
      )}
    </>
  );
}

function ProjectExchange({
  project,
  canWrite,
  canRead,
  selectedJob,
  selectJob,
}: {
  project: ProjectView;
  canWrite: boolean;
  canRead: boolean;
  selectedJob: string | null;
  selectJob: (id: string | null) => void;
}) {
  return (
    <>
      <section>
        <h2>Import source</h2>
        {canWrite ? (
          <ImportPanel project={project} onPlanned={(id) => selectJob(id)} />
        ) : (
          <div className="banner error">
            This project does not forecast <code>knowledge.write</code>. Import and candidate
            materialisation controls are not offered; the gateway remains authoritative.
          </div>
        )}
      </section>
      <ImportHistory project={project} selected={selectedJob} select={selectJob} />
      {selectedJob ? (
        <SelectedPlan jobId={selectedJob} canWrite={canWrite} projectId={project.id} />
      ) : null}
      <ExportPanel project={project} canRead={canRead} />
    </>
  );
}

function ImportPanel({ project, onPlanned }: { project: ProjectView; onPlanned: (id: string) => void }) {
  const [files, setFiles] = useState<File[]>([]);
  const [source, setSource] = useState("pulseboard-okf");
  const [revision, setRevision] = useState("");
  const [plan, setPlan] = useState<OkfImportPlanView | null>(null);
  const [materialized, setMaterialized] = useState<OkfMaterializationView | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [stage, setStage] = useState<"idle" | "packaging" | "planning" | "materializing">(
    "idle",
  );

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setNotice(null);
    setMaterialized(null);
    setStage("packaging");
    try {
      const body = await importBody(files, source, revision);
      setStage("planning");
      const answer = await request("plan_okf_import", {
        path: { project_id: project.id },
        body,
        idempotencyKey: idempotencyKey(),
      });
      if (answer.kind !== "ok") {
        setNotice({ error: failedAnswerMessage(answer) });
        return;
      }
      setPlan(answer.body);
      onPlanned(answer.body.job.id);
      setNotice({
        message: "Validation and immutable dry-run completed. No candidate or Knowledge was created.",
      });
      invalidate("okf/imports");
    } catch (cause) {
      setNotice({ error: errorMessage(cause) });
    } finally {
      setStage("idle");
    }
  };

  const materialize = async () => {
    if (!plan) return;
    setNotice(null);
    setStage("materializing");
    const answer = await request("materialize_okf_import", {
      path: { id: plan.job.id },
      idempotencyKey: `okf-materialize-${plan.job.id}`,
    });
    if (answer.kind === "ok") {
      setMaterialized(answer.body);
      setNotice({
        message: `${answer.body.candidates.length} reviewable candidate(s) are ready in New Learnings. No Knowledge was published.`,
      });
      invalidate("okf/imports", "capture", "learnings");
    } else {
      setNotice({ error: failedAnswerMessage(answer) });
    }
    setStage("idle");
  };

  return (
    <>
      <form className="knowledge-filters" onSubmit={(event) => void submit(event)}>
        <label>
          Source name
          <input value={source} onChange={(event) => setSource(event.target.value)} required />
        </label>
        <label>
          Explicit Git revision (optional)
          <input
            value={revision}
            onChange={(event) => setRevision(event.target.value)}
            placeholder="commit or tag supplied by the client"
          />
        </label>
        <label>
          Checked-out directory
          <input
            type="file"
            multiple
            {...({ directory: "", webkitdirectory: "" } as Record<string, string>)}
            onChange={(event) => setFiles(Array.from(event.target.files ?? []))}
          />
        </label>
        <label>
          Or one .zip/.tar/.tar.gz archive
          <input
            type="file"
            accept=".zip,.tar,.gz,.tgz,application/zip,application/gzip"
            onChange={(event) => setFiles(Array.from(event.target.files ?? []))}
          />
        </label>
        <div className="form-actions">
          <button type="submit" disabled={stage !== "idle" || files.length === 0}>
            Validate and plan dry-run
          </button>
          <span className="muted">
            {stage === "packaging"
              ? "Reading local files…"
              : stage === "planning"
                ? "Validating through the pinned server adapter…"
                : `${files.length} file(s) selected`}
          </span>
        </div>
      </form>
      <NoticeView notice={notice} />
      {plan ? (
        <PlanEvidence
          plan={plan}
          materialized={materialized}
          canMaterialize={stage === "idle" && plan.job.state === "planned"}
          onMaterialize={() => void materialize()}
        />
      ) : null}
    </>
  );
}

function ImportHistory({
  project,
  selected,
  select,
}: {
  project: ProjectView;
  selected: string | null;
  select: (id: string | null) => void;
}) {
  const key = `okf/imports/${project.id}`;
  const entry = useQuery(key, () =>
    request("list_okf_imports", { query: { project_id: project.id, limit: "100" } }),
  );
  const retry = useRefresh(key);
  return (
    <section>
      <h2>Import history</h2>
      <Loaded<OkfImportJobListView> entry={entry} what="OKF import history" onRetry={retry}>
        {(body) =>
          body.jobs.length === 0 ? (
            <p className="muted">No visible OKF import has been planned for this project.</p>
          ) : (
            <ul className="session-list">
              {body.jobs.map((job) => (
                <li key={job.id} className={selected === job.id ? "selected" : undefined}>
                  <div className="row">
                    <strong>{job.source_locator}</strong>{" "}
                    <span className={`tag ${job.state === "materialized" ? "done" : "warn"}`}>
                      {job.state}
                    </span>
                    <p className="muted">
                      {job.source_kind}
                      {job.source_revision ? ` @ ${job.source_revision}` : ""} · OKF{" "}
                      {job.format_version} · {whenOf(job.created_at)}
                    </p>
                    <p>{importProgress(job)}</p>
                    <button type="button" onClick={() => select(selected === job.id ? null : job.id)}>
                      {selected === job.id ? "Close plan" : "Inspect plan"}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )
        }
      </Loaded>
    </section>
  );
}

function SelectedPlan({ jobId, canWrite, projectId }: { jobId: string; canWrite: boolean; projectId: string }) {
  const key = `okf/import/${jobId}`;
  const entry = useQuery(key, () => request("get_okf_import", { path: { id: jobId } }));
  const retry = useRefresh(key);
  const [materialized, setMaterialized] = useState<OkfMaterializationView | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);

  const materialize = async () => {
    setBusy(true);
    const answer = await request("materialize_okf_import", {
      path: { id: jobId },
      idempotencyKey: `okf-materialize-${jobId}`,
    });
    if (answer.kind === "ok") {
      setMaterialized(answer.body);
      setNotice({ message: `${answer.body.candidates.length} candidate(s) are ready for review.` });
      invalidate("okf/imports", `okf/import/${jobId}`, "capture", "learnings");
    } else {
      setNotice({ error: failedAnswerMessage(answer) });
    }
    setBusy(false);
  };

  return (
    <section>
      <h2>Dry-run detail</h2>
      <NoticeView notice={notice} />
      <Loaded<OkfImportPlanView> entry={entry} what="the immutable OKF plan" onRetry={retry}>
        {(plan) => (
          <PlanEvidence
            plan={plan}
            materialized={materialized}
            canMaterialize={canWrite && !busy && plan.job.state === "planned" && plan.job.project_id === projectId}
            onMaterialize={() => void materialize()}
          />
        )}
      </Loaded>
    </section>
  );
}

export function PlanEvidence({
  plan,
  materialized = null,
  canMaterialize = false,
  onMaterialize = () => {},
}: {
  plan: OkfImportPlanView;
  materialized?: OkfMaterializationView | null;
  canMaterialize?: boolean;
  onMaterialize?: () => void;
}) {
  const counts = classificationCounts(plan.mappings);
  return (
    <article className="knowledge-detail">
      <header>
        <h3>{plan.job.source_locator}</h3>
        <p className="muted">
          {plan.job.source_kind}
          {plan.job.source_revision ? ` @ ${plan.job.source_revision}` : ""} · OKF{" "}
          {plan.job.format_version} · spec {plan.job.specification_commit}
        </p>
        <p>
          Validation passed · immutable bundle <code>{plan.job.bundle_digest}</code>
        </p>
      </header>
      <div className="metric-strip">
        <Metric label="Additions" value={counts.addition} />
        <Metric label="Updates" value={counts.update} />
        <Metric label="Duplicates" value={counts.duplicate} />
        <Metric label="Conflicts" value={counts.conflict} />
        <Metric label="Candidates" value={materialized?.candidates.length ?? plan.job.candidate_count} />
      </div>
      <p>{importProgress(materialized?.job ?? plan.job)}</p>
      {plan.job.notices.map((notice) => (
        <div key={notice} className="banner warning">
          {notice}
        </div>
      ))}
      <h4>Proposed mappings</h4>
      <ul className="session-list">
        {plan.mappings.map((mapping) => (
          <li key={mapping.id}>
            <strong>{mapping.content.title}</strong>{" "}
            <span className={`tag ${mapping.classification === "duplicate" ? "done" : "warn"}`}>
              {mapping.classification}
            </span>
            <p>
              Producer type <code>{mapping.okf_type}</code> → proposed Knowledge{" "}
              <code>{mapping.knowledge_type}</code>
            </p>
            <p className="muted">
              {mapping.content.summary} · {mapping.materializable ? "reviewable" : "retained only"}
              {mapping.matched_item_id ? ` · compared with ${mapping.matched_item_id}` : ""}
            </p>
            <details>
              <summary>Preserved metadata and proposed relations</summary>
              <pre>{displayJson({ metadata: mapping.content.metadata, relations: mapping.proposed_relations })}</pre>
            </details>
          </li>
        ))}
      </ul>
      <h4>Validated artifacts</h4>
      <ul className="session-list">
        {plan.artifacts.map((artifact) => (
          <li key={artifact.id}>
            <strong>{artifact.logical_path}</strong> · {artifact.kind} ·{" "}
            <code>{artifact.content_hash}</code>
            <details>
              <summary>Exact frontmatter, including unknown extensions</summary>
              <pre>{displayJson(artifact.frontmatter)}</pre>
            </details>
          </li>
        ))}
      </ul>
      {materialized ? (
        <div className="banner success" role="status">
          Candidate materialisation completed. Review {materialized.candidates.length} result(s) in{" "}
          <Link href={hrefOf("learnings")}>New Learnings</Link>. Unreviewed candidates are not active
          Knowledge.
          <ul>
            {materialized.candidates.map((candidate) => (
              <li key={candidate.id}>
                {candidate.content.title} · {candidate.state} · {candidate.id}
              </li>
            ))}
          </ul>
        </div>
      ) : canMaterialize ? (
        <button type="button" onClick={onMaterialize}>
          Create review candidates
        </button>
      ) : plan.job.state === "planned" ? (
        <p className="muted">
          Candidate materialisation requires <code>knowledge.write</code> at this project.
        </p>
      ) : (
        <p>
          <Link href={hrefOf("learnings")}>Open resulting New Learnings</Link>
        </p>
      )}
    </article>
  );
}

function ExportPanel({ project, canRead }: { project: ProjectView; canRead: boolean }) {
  const key = `okf/export-knowledge/${project.id}`;
  const entry = useQuery(key, () =>
    request("list_knowledge", {
      query: { project_id: project.id, lifecycle_state: "active", limit: "200" },
    }),
  );
  const retry = useRefresh(key);
  const [selected, setSelected] = useState<string[]>([]);
  const [status, setStatus] = useState<"idle" | "running" | "completed" | "failed">("idle");
  const [result, setResult] = useState<OkfExportView | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);

  const run = async () => {
    setStatus("running");
    setNotice(null);
    const answer = await request("export_okf", {
      path: { project_id: project.id },
      body: { item_ids: selected },
    });
    if (answer.kind === "ok") {
      setResult(answer.body);
      setStatus("completed");
      setNotice({ message: "Deterministic export completed." });
    } else {
      setResult(null);
      setStatus("failed");
      setNotice({ error: failedAnswerMessage(answer) });
    }
  };

  return (
    <section>
      <h2>Export current Knowledge</h2>
      <p className="muted">
        Select exact current items, or select none to export every current policy-visible item in{" "}
        {project.display_name}. Export is a freshly authorised projection, not a synchronisation job.
      </p>
      {!canRead ? (
        <div className="banner error">
          This project does not forecast <code>knowledge.read</code>; export is not offered.
        </div>
      ) : (
        <Loaded<KnowledgeListView> entry={entry} what="current project Knowledge" onRetry={retry}>
          {(body) => (
            <>
              {body.items.length === 0 ? (
                <p className="muted">No current active Knowledge is visible for selection.</p>
              ) : (
                <ul className="session-list">
                  {body.items.map((item) => (
                    <li key={item.id}>
                      <label>
                        <input
                          type="checkbox"
                          checked={selected.includes(item.id)}
                          onChange={() =>
                            setSelected((before) =>
                              before.includes(item.id)
                                ? before.filter((id) => id !== item.id)
                                : [...before, item.id],
                            )
                          }
                        />{" "}
                        <strong>{item.current_revision.title}</strong> · {item.knowledge_type} · revision{" "}
                        {item.current_revision.revision_number}
                      </label>
                    </li>
                  ))}
                </ul>
              )}
              <button type="button" disabled={status === "running"} onClick={() => void run()}>
                {status === "running" ? "Exporting…" : "Export OKF v0.2"}
              </button>
              <p className="muted">Export job status: {status}</p>
            </>
          )}
        </Loaded>
      )}
      <NoticeView notice={notice} />
      {result ? <ExportSummary result={result} /> : null}
    </section>
  );
}

export function ExportSummary({ result }: { result: OkfExportView }) {
  return (
    <article className="knowledge-detail">
      <h3>Exported bundle summary</h3>
      <p>
        OKF {result.format_version} · spec <code>{result.specification_commit}</code> ·{" "}
        {result.files.length} stable file(s)
      </p>
      <p>
        Bundle digest <code>{result.bundle_digest}</code>
      </p>
      <ul className="session-list">
        {result.files.map((file) => (
          <li key={file.logical_path}>
            <strong>{file.logical_path}</strong> · <code>{file.content_hash}</code>{" "}
            <a
              download={file.logical_path.split("/").at(-1)}
              href={`data:text/markdown;charset=utf-8,${encodeURIComponent(file.content)}`}
            >
              Download file
            </a>
            <details>
              <summary>Preview exact Markdown</summary>
              <pre>{file.content}</pre>
            </details>
          </li>
        ))}
      </ul>
      <p className="muted">
        The CLI preserves the complete stable directory shape atomically; browser downloads expose
        each exact file and its logical path.
      </p>
    </article>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <span>
      <strong>{value}</strong> {label}
    </span>
  );
}

function NoticeView({ notice }: { notice: Notice | null }) {
  if (!notice) return null;
  return (
    <div className={`banner ${notice.error ? "error" : "success"}`} role="status">
      {notice.error ?? notice.message}
    </div>
  );
}

function failedAnswerMessage(answer: Exclude<Answer<unknown>, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "The selected OKF input could not be read.";
}

function displayJson(value: unknown): string {
  return JSON.stringify(value ?? {}, null, 2);
}

function mayAt(
  anchors: Array<{ scope_id: string; actions: Record<string, boolean> }>,
  scopeId: string,
  action: string,
): boolean {
  return anchors.find((anchor) => anchor.scope_id === scopeId)?.actions[action] === true;
}

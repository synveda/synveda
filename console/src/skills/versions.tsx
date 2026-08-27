import { useState, type FormEvent } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import { whenOf } from "../people.mjs";
import {
  formatBundleFiles,
  manifestSummary,
  parseBundleFiles,
  scanSummary,
  sourceLabel,
} from "../skills.mjs";
import { FixtureTests, UsageEvidence } from "./evidence.js";
import {
  applyMutationOutcome,
  errorMessage,
  failedAnswerMessage,
  JsonValue,
  MutationNoticeView,
  noticeOf,
  SensitivitySelect,
  type MutationNotice,
  type Sensitivity,
} from "./ui.js";
import type {
  SkillFileBody,
  SkillVersionFileListView,
  SkillVersionView,
  SkillView,
} from "../generated/api.js";

export function NewVersionForm({ skill }: { skill: SkillView }) {
  const [open, setOpen] = useState(false);
  const [sensitivity, setSensitivity] = useState<Sensitivity>(skill.current_version.sensitivity);
  const [sourceKind, setSourceKind] = useState<SkillVersionView["source_kind"]>(
    skill.current_version.source_kind,
  );
  const [reference, setReference] = useState(
    typeof skill.current_version.provenance.reference === "string"
      ? skill.current_version.provenance.reference
      : "",
  );
  const [revision, setRevision] = useState("");
  const [files, setFiles] = useState("");
  const [notice, setNotice] = useState<MutationNotice | null>(null);
  const [busy, setBusy] = useState(false);

  const loadCurrent = async () => {
    setBusy(true);
    setNotice(null);
    const listing = await request("list_skill_version_files", {
      path: { id: skill.id, version_id: skill.current_version_id },
    });
    if (listing.kind !== "ok") {
      setBusy(false);
      setNotice({ kind: "error", message: failedAnswerMessage(listing) });
      return;
    }
    const loaded: SkillFileBody[] = [];
    for (const descriptor of listing.body.files) {
      const answer = await request("get_skill_version_file", {
        path: {
          id: skill.id,
          version_id: skill.current_version_id,
          path: descriptor.path,
        },
      });
      if (answer.kind !== "ok") {
        setBusy(false);
        setNotice({ kind: "error", message: failedAnswerMessage(answer) });
        return;
      }
      loaded.push({ path: answer.body.path, content: answer.body.content });
    }
    setFiles(formatBundleFiles(loaded));
    setBusy(false);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let bundle: SkillFileBody[];
    try {
      bundle = parseBundleFiles(files);
    } catch (cause) {
      setNotice({ kind: "error", message: errorMessage(cause) });
      return;
    }
    setBusy(true);
    setNotice(null);
    const answer = await request("update_skill", {
      path: { id: skill.id },
      body: {
        expected_current_version_id: skill.current_version_id,
        sensitivity,
        files: bundle,
        provenance: {
          kind: sourceKind,
          reference: reference.trim() || undefined,
          revision: revision.trim() || undefined,
        },
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const notice = noticeOf(answer);
    setNotice(notice);
    applyMutationOutcome(notice, ["skills"], { invalidate });
  };

  return (
    <section className="skill-form">
      <h3>Update</h3>
      <button type="button" onClick={() => setOpen((value) => !value)}>
        {open ? "Close version editor" : "Create immutable version"}
      </button>
      {open ? (
        <form onSubmit={(event) => void submit(event)}>
          <p className="muted">
            An update is a complete replacement bundle with a stale-head precondition. Load the
            current bytes first so no resource disappears accidentally.
          </p>
          <p>
            <button type="button" disabled={busy} onClick={() => void loadCurrent()}>
              Load current bundle
            </button>
          </p>
          <div className="form-grid">
            <SensitivitySelect value={sensitivity} onChange={setSensitivity} />
            <label>
              Source
              <select
                value={sourceKind}
                onChange={(event) =>
                  setSourceKind(event.target.value as SkillVersionView["source_kind"])
                }
              >
                {(["authored", "directory", "archive", "git", "registry"] as const).map(
                  (kind) => (
                    <option key={kind}>{kind}</option>
                  ),
                )}
              </select>
            </label>
            <label>
              Source reference
              <input value={reference} onChange={(event) => setReference(event.target.value)} />
            </label>
            <label>
              Source revision
              <input value={revision} onChange={(event) => setRevision(event.target.value)} />
            </label>
          </div>
          <label className="full-field">
            Complete bundle files (JSON)
            <textarea rows={16} value={files} onChange={(event) => setFiles(event.target.value)} />
          </label>
          <p>
            <button type="submit" disabled={busy || files.length === 0}>
              {busy ? "Submitting…" : "Propose new version"}
            </button>
          </p>
        </form>
      ) : null}
      <MutationNoticeView notice={notice} />
    </section>
  );
}

export function VersionExplorer({
  skill,
  versions,
  canTest,
}: {
  skill: SkillView;
  versions: SkillVersionView[];
  canTest: boolean;
}) {
  const [versionId, setVersionId] = useState(skill.current_version_id);
  const version = versions.find((candidate) => candidate.id === versionId) ?? versions[0];
  if (!version) return null;
  return (
    <>
      <label>
        Inspect version{" "}
        <select value={version.id} onChange={(event) => setVersionId(event.target.value)}>
          {versions.map((candidate) => (
            <option key={candidate.id} value={candidate.id}>
              v{candidate.ordinal}
              {candidate.id === skill.current_version_id ? " · current" : ""} ·{" "}
              {whenOf(candidate.created_at)}
            </option>
          ))}
        </select>
      </label>
      <VersionDetail skill={skill} version={version} canTest={canTest} />
    </>
  );
}

function VersionDetail({
  skill,
  version,
  canTest,
}: {
  skill: SkillView;
  version: SkillVersionView;
  canTest: boolean;
}) {
  const manifest = manifestSummary(version.manifest);
  const scan = scanSummary(version.scan);
  const filesKey = `skills/item/${skill.id}/versions/${version.id}/files`;
  const files = useQuery(filesKey, () =>
    request("list_skill_version_files", {
      path: { id: skill.id, version_id: version.id },
    }),
  );
  return (
    <div className="skill-version">
      <section>
        <h4>Manifest and provenance</h4>
        <dl className="facts">
          <dt>Version</dt>
          <dd>
            v{version.ordinal} · <span className="mono breakable">{version.id}</span>
          </dd>
          <dt>Digest</dt>
          <dd className="mono breakable">{version.bundle_digest}</dd>
          <dt>Source</dt>
          <dd>{sourceLabel(version)}</dd>
          <dt>Compatibility</dt>
          <dd>{manifest.compatibility ?? "No client constraint declared"}</dd>
          <dt>License</dt>
          <dd>{manifest.license ?? "Not declared"}</dd>
          <dt>Quality</dt>
          <dd>
            {version.quality_score}/100 · rubric v{version.rubric_version}
          </dd>
          <dt>Created</dt>
          <dd>
            {whenOf(version.created_at)} by {version.created_by}
          </dd>
        </dl>
        <h5>Declared tools</h5>
        <div className="banner warning">
          Tool declarations are metadata only. They grant no access and are never treated as
          authorisation.
        </div>
        {manifest.declaredTools.length === 0 ? (
          <p className="muted">No tools declared.</p>
        ) : (
          <ul className="inline-list">
            {manifest.declaredTools.map((tool) => (
              <li key={tool}>
                <code>{tool}</code>
              </li>
            ))}
          </ul>
        )}
        <h5>Manifest metadata and extensions</h5>
        <JsonValue value={manifest.extensions} empty="No extension metadata." />
      </section>
      <section>
        <h4>Security scan</h4>
        <p className="muted">
          Ruleset v{version.scan_ruleset_version}
          {scan.worst ? ` · worst ${scan.worst}` : " · no findings"}
          {scan.blocksAt ? ` · refuses at ${scan.blocksAt}` : ""}
        </p>
        {scan.findings.length === 0 ? (
          <p>No scanner findings.</p>
        ) : (
          <ul className="scan-findings">
            {scan.findings.map((finding, index) => (
              <li key={`${finding.path}-${finding.rule}-${index}`}>
                <span className={`tag ${finding.severity}`}>{finding.severity}</span>{" "}
                <code>{finding.rule}</code> · {finding.path}
                {finding.line === null ? "" : `:${finding.line}`}
                {finding.count === 1 ? "" : ` ×${finding.count}`}
              </li>
            ))}
          </ul>
        )}
      </section>
      <section>
        <h4>File browser</h4>
        <Loaded<SkillVersionFileListView> entry={files} what="Skill files">
          {(body) =>
            body.files.length === 0 ? (
              <p className="muted">This version contains no visible files.</p>
            ) : (
              <FileBrowser skillId={skill.id} versionId={version.id} files={body} />
            )
          }
        </Loaded>
      </section>
      <FixtureTests skill={skill} version={version} canRun={canTest} />
      <UsageEvidence skill={skill} version={version} />
    </div>
  );
}

function FileBrowser({
  skillId,
  versionId,
  files,
}: {
  skillId: string;
  versionId: string;
  files: SkillVersionFileListView;
}) {
  const initial = files.files.find((file) => file.path === "SKILL.md")?.path ?? files.files[0]?.path;
  const [path, setPath] = useState(initial ?? "");
  const key = `skills/item/${skillId}/versions/${versionId}/files/${path}`;
  const content = useQuery(key, () =>
    request("get_skill_version_file", {
      path: { id: skillId, version_id: versionId, path },
    }),
  );
  return (
    <div className="skill-files">
      <ul>
        {files.files.map((file) => (
          <li key={file.path}>
            <button
              type="button"
              className={file.path === path ? "selected" : undefined}
              onClick={() => setPath(file.path)}
            >
              {file.path}
            </button>{" "}
            <span className="muted">{file.chars} characters</span>
          </li>
        ))}
      </ul>
      <Loaded<{ content: string; object_hash: string; path: string }>
        entry={content}
        what={path}
      >
        {(body) => (
          <>
            <p className="mono breakable">object {body.object_hash}</p>
            <pre className="file-content">{body.content}</pre>
          </>
        )}
      </Loaded>
    </div>
  );
}

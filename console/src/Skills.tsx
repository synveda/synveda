/**
 * Skills Library (CPR-24, ADR-0085).
 *
 * This is a view over CPR-23's stable Skill aggregates, immutable versions
 * and revisioned bindings. It calls only generated public operations. Every
 * offered write still becomes a VedaFlow change at the gateway; capability
 * forecasts here improve the product but grant no authority.
 */

import { useEffect, useMemo, useState, type FormEvent } from "react";

import { idempotencyKey, request, type Answer } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link, navigate } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import {
  activationEvidence,
  evidenceLabel,
  formatBundleFiles,
  manifestSummary,
  mayWriteAt,
  parseBundleFiles,
  scanSummary,
  skillMutationMessage,
  skillScopes,
  sourceLabel,
  type SkillScopeOption,
} from "./skills.mjs";
import type {
  AvailableSkillListView,
  SkillBindingListView,
  SkillBindingView,
  SkillFileBody,
  SkillListView,
  SkillMutationView,
  SkillTestRunListView,
  SkillUsageListView,
  SkillVersionFileListView,
  SkillVersionListView,
  SkillVersionView,
  SkillView,
} from "./generated/api.js";

type Sensitivity = SkillVersionView["sensitivity"];
type Notice = { result?: SkillMutationView; error?: string };

const INITIAL_BUNDLE = formatBundleFiles([
  {
    path: "SKILL.md",
    content:
      "---\nname: release-check\ndescription: Check a release before publishing.\n---\n\n# Release check\n\nDescribe the controlled release procedure.\n",
  },
]);

export function Skills() {
  const { me, project } = useApp();
  const scopes = useMemo(() => skillScopes(me, project), [me, project]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [seen, setSeen] = useState<SkillView[]>([]);
  const [installing, setInstalling] = useState(false);
  const query = { cursor: cursor ?? undefined, limit: "100" };
  const key = `skills/catalogue/${cursor ?? "first"}`;
  const entry = useQuery(key, () => request("list_skills", { query }));
  const retry = useRefresh(key);

  return (
    <>
      <PageHeading route="skills" />
      <p className="skills-intro">
        Install once, retain every immutable version, then bind an exact version—or follow the
        current one—privately or to this project. A session sees a Skill only when both its
        binding and the live policy decision permit it.
      </p>
      <div className="knowledge-toolbar">
        <button type="button" onClick={() => setInstalling((value) => !value)}>
          {installing ? "Close installer" : "Install Skill"}
        </button>
      </div>
      {installing ? <InstallSkill scopes={scopes} /> : null}
      <ScopeAvailability scopes={scopes} />
      <Loaded<SkillListView> entry={entry} what="the Skill catalogue" onRetry={retry}>
        {(body) => {
          const skills = appendSkills(seen, body.skills);
          return (
            <>
              <h2>Installed Skills</h2>
              {skills.length === 0 ? (
                <p className="muted">
                  No installed Skill is visible under this policy. A denied aggregate is omitted,
                  so this does not disclose whether one exists elsewhere.
                </p>
              ) : (
                <ul className="skill-library-list">
                  {skills.map((skill) => (
                    <SkillRow key={skill.id} skill={skill} />
                  ))}
                </ul>
              )}
              {body.next_cursor ? (
                <p>
                  <button
                    type="button"
                    onClick={() => {
                      setSeen(skills);
                      setCursor(body.next_cursor ?? null);
                    }}
                  >
                    Load more
                  </button>{" "}
                  <span className="muted">
                    The cursor advances over catalogue rows considered by the PDP, including rows
                    it did not disclose.
                  </span>
                </p>
              ) : null}
            </>
          );
        }}
      </Loaded>
    </>
  );
}

function SkillRow({ skill }: { skill: SkillView }) {
  const manifest = manifestSummary(skill.current_version.manifest);
  return (
    <li>
      <Link href={hrefOf("skill-item", { skill_id: skill.id })} className="row">
        <strong>{skill.name}</strong>{" "}
        <span className={`tag ${skill.current_version.sensitivity}`}>
          {skill.current_version.sensitivity}
        </span>
        <p>{manifest.description}</p>
        <div className="muted">
          Current v{skill.current_version.ordinal} · quality {skill.current_version.quality_score}
          /100 · {sourceLabel(skill.current_version)} · updated {whenOf(skill.updated_at)}
        </div>
        <div className="mono breakable">{skill.current_version.bundle_digest}</div>
      </Link>
    </li>
  );
}

function ScopeAvailability({ scopes }: { scopes: SkillScopeOption[] }) {
  const [scopeId, setScopeId] = useState(scopes[0]?.id ?? "");
  useEffect(() => {
    if (!scopes.some((scope) => scope.id === scopeId)) setScopeId(scopes[0]?.id ?? "");
  }, [scopeId, scopes]);
  if (scopes.length === 0) {
    return (
      <div className="banner warning">
        No principal or selected-project scope is visible, so session availability cannot be
        resolved here.
      </div>
    );
  }
  return (
    <section className="skill-availability">
      <h2>Available to a session</h2>
      <label>
        Placement{" "}
        <select value={scopeId} onChange={(event) => setScopeId(event.target.value)}>
          {scopes.map((scope) => (
            <option key={scope.id} value={scope.id}>
              {scope.label}
            </option>
          ))}
        </select>
      </label>
      {scopeId ? <AvailableSkills scopeId={scopeId} /> : null}
    </section>
  );
}

function AvailableSkills({ scopeId }: { scopeId: string }) {
  const key = `skills/available/${scopeId}`;
  const entry = useQuery(key, () =>
    request("list_available_skills", { query: { scope_id: scopeId } }),
  );
  return (
    <Loaded<AvailableSkillListView> entry={entry} what="session Skill availability">
      {(body) =>
        body.skills.length === 0 ? (
          <p className="muted">No enabled, policy-visible binding resolves at this placement.</p>
        ) : (
          <ul className="inline-list">
            {body.skills.map((available) => (
              <li key={available.binding.id}>
                <strong>{available.name}</strong> v{available.version.ordinal}{" "}
                <span className="muted">
                  ({available.binding.pinned_version_id ? "pinned" : "follows current"})
                </span>
              </li>
            ))}
          </ul>
        )
      }
    </Loaded>
  );
}

function InstallSkill({ scopes }: { scopes: SkillScopeOption[] }) {
  const writable = scopes.filter((scope) => scope.canWrite);
  const [scopeId, setScopeId] = useState(writable[0]?.id ?? "");
  const [name, setName] = useState("release-check");
  const [sensitivity, setSensitivity] = useState<Sensitivity>("internal");
  const [files, setFiles] = useState(INITIAL_BUNDLE);
  const [sourceKind, setSourceKind] = useState<SkillVersionView["source_kind"]>("authored");
  const [reference, setReference] = useState("");
  const [revision, setRevision] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);

  if (writable.length === 0) {
    return (
      <section className="skill-form">
        <h2>Install Skill</h2>
        <p className="muted">
          No personal or selected-project scope forecasts <code>skill.write</code>, so the
          installer is not offered. The gateway remains the authority if that forecast changes.
        </p>
      </section>
    );
  }

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    let bundle: SkillFileBody[];
    try {
      bundle = parseBundleFiles(files);
    } catch (cause) {
      setNotice({ error: errorMessage(cause) });
      return;
    }
    setBusy(true);
    setNotice(null);
    const answer = await request("install_skill", {
      body: {
        governing_scope_id: scopeId,
        name,
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
    const result = noticeOf(answer);
    setNotice(result);
    if (result.result?.outcome === "applied") {
      invalidate("skills");
      if (result.result.skill_id) {
        navigate(hrefOf("skill-item", { skill_id: result.result.skill_id }));
      }
    }
  };

  return (
    <section className="skill-form">
      <h2>Install Skill</h2>
      <p className="muted">
        The complete bundle is proposed through VedaFlow. Agent Skills frontmatter and unknown
        extension metadata are retained byte-for-byte.
      </p>
      <form onSubmit={(event) => void submit(event)}>
        <div className="form-grid">
          <label>
            Governed scope
            <select value={scopeId} onChange={(event) => setScopeId(event.target.value)}>
              {writable.map((scope) => (
                <option key={scope.id} value={scope.id}>
                  {scope.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            Bundle name
            <input required value={name} onChange={(event) => setName(event.target.value)} />
          </label>
          <SensitivitySelect value={sensitivity} onChange={setSensitivity} />
          <label>
            Source
            <select
              value={sourceKind}
              onChange={(event) =>
                setSourceKind(event.target.value as SkillVersionView["source_kind"])
              }
            >
              {(["authored", "directory", "archive", "git", "registry"] as const).map((kind) => (
                <option key={kind}>{kind}</option>
              ))}
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
          <textarea rows={14} value={files} onChange={(event) => setFiles(event.target.value)} />
        </label>
        <p>
          <button type="submit" disabled={busy}>
            {busy ? "Submitting…" : "Propose installation"}
          </button>
        </p>
      </form>
      <MutationNotice notice={notice} />
    </section>
  );
}

export function SkillItem({ skillId }: { skillId: string }) {
  const key = `skills/item/${skillId}`;
  const entry = useQuery(key, () => request("get_skill", { path: { id: skillId } }));
  const retry = useRefresh(key);
  return (
    <>
      <PageHeading route="skill-item" />
      <p>
        <Link href={hrefOf("skills")}>← Skills Library</Link>
      </p>
      <Loaded<SkillView> entry={entry} what="this Skill" onRetry={retry}>
        {(skill) => <SkillDetail key={skill.current_version_id} skill={skill} />}
      </Loaded>
    </>
  );
}

function SkillDetail({ skill }: { skill: SkillView }) {
  const { me, project } = useApp();
  const scopes = useMemo(() => skillScopes(me, project), [me, project]);
  const versionKey = `skills/item/${skill.id}/versions`;
  const versions = useQuery(versionKey, () =>
    request("list_skill_versions", {
      path: { id: skill.id },
      query: { limit: "200" },
    }),
  );
  const canUpdate = mayWriteAt(me.anchors, skill.governing_scope_id);
  return (
    <article className="skill-detail">
      <header>
        <h2>{skill.name}</h2>
        <p>
          <span className="tag done">current v{skill.current_version.ordinal}</span>{" "}
          <span className={`tag ${skill.current_version.sensitivity}`}>
            {skill.current_version.sensitivity}
          </span>
        </p>
        <p>{manifestSummary(skill.current_version.manifest).description}</p>
        <p className="muted">
          Stable Skill {skill.id} · governing scope {skill.governing_scope_id} · updated{" "}
          {whenOf(skill.updated_at)}
        </p>
      </header>
      <section>
        <h3>Bindings and session availability</h3>
        <p className="muted">
          Personal and project bindings are independent revisioned records. Disable, pin and
          rollback change a binding; they never rewrite version history.
        </p>
        <Loaded<SkillVersionListView> entry={versions} what="immutable Skill versions">
          {(body) => (
            <div className="skill-binding-grid">
              {scopes.map((scope) => (
                <BindingPanel key={scope.id} skill={skill} scope={scope} versions={body.versions} />
              ))}
            </div>
          )}
        </Loaded>
      </section>
      {canUpdate ? <NewVersionForm skill={skill} /> : null}
      {!canUpdate ? (
        <p className="muted">
          This scope does not forecast <code>skill.write</code>, so update and fixture-test actions
          are not offered. Reads still meet their own gateway decision.
        </p>
      ) : null}
      <section>
        <h3>Immutable versions</h3>
        <Loaded<SkillVersionListView> entry={versions} what="immutable Skill versions">
          {(body) =>
            body.versions.length === 0 ? (
              <p className="muted">No visible version exists.</p>
            ) : (
              <VersionExplorer skill={skill} versions={body.versions} canTest={canUpdate} />
            )
          }
        </Loaded>
      </section>
    </article>
  );
}

function BindingPanel({
  skill,
  scope,
  versions,
}: {
  skill: SkillView;
  scope: SkillScopeOption;
  versions: SkillVersionView[];
}) {
  const key = `skills/bindings/${scope.id}`;
  const entry = useQuery(key, () =>
    request("list_skill_bindings", { query: { scope_id: scope.id, limit: "200" } }),
  );
  const availableKey = `skills/available/${scope.id}`;
  const available = useQuery(availableKey, () =>
    request("list_available_skills", { query: { scope_id: scope.id } }),
  );
  return (
    <section className="skill-binding-card">
      <h4>{scope.label}</h4>
      <Loaded<SkillBindingListView> entry={entry} what={`${scope.label} bindings`}>
        {(body) => {
          const binding = body.bindings.find((candidate) => candidate.skill_id === skill.id);
          return binding ? (
            <BindingControls binding={binding} scope={scope} versions={versions} />
          ) : (
            <CreateBinding skill={skill} scope={scope} />
          );
        }}
      </Loaded>
      <Loaded<AvailableSkillListView> entry={available} what={`${scope.label} availability`}>
        {(body) => {
          const resolved = body.skills.find((candidate) => candidate.binding.skill_id === skill.id);
          return resolved ? (
            <p className="availability-state">
              <span className="tag done">available</span> exact v{resolved.version.ordinal} ·{" "}
              <span className="mono breakable">{resolved.version.bundle_digest}</span>
            </p>
          ) : (
            <p className="muted">Not advertised here: no enabled, policy-visible resolution.</p>
          );
        }}
      </Loaded>
    </section>
  );
}

function CreateBinding({ skill, scope }: { skill: SkillView; scope: SkillScopeOption }) {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  if (!scope.canWrite) {
    return <p className="muted">No binding exists, and policy does not offer creation here.</p>;
  }
  const create = async () => {
    setBusy(true);
    setNotice(null);
    const answer = await request("create_skill_binding", {
      body: { skill_id: skill.id, scope_id: scope.id, enabled: true },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const notice = noticeOf(answer);
    setNotice(notice);
    if (notice.result?.outcome === "applied") invalidate("skills/bindings", "skills/available");
  };
  return (
    <>
      <p className="muted">No binding exists.</p>
      <button type="button" disabled={busy} onClick={() => void create()}>
        {busy ? "Submitting…" : "Bind and follow current"}
      </button>
      <MutationNotice notice={notice} />
    </>
  );
}

function BindingControls({
  binding,
  scope,
  versions,
}: {
  binding: SkillBindingView;
  scope: SkillScopeOption;
  versions: SkillVersionView[];
}) {
  const oldest = versions.at(-1)?.id ?? "";
  const [selected, setSelected] = useState(binding.pinned_version_id ?? oldest);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);
  const selectedVersion = versions.find((version) => version.id === selected);
  const effectiveVersion = binding.pinned_version_id
    ? versions.find((version) => version.id === binding.pinned_version_id)
    : versions[0];
  const canRollback =
    selectedVersion !== undefined &&
    effectiveVersion !== undefined &&
    selectedVersion.ordinal < effectiveVersion.ordinal;

  const change = async (kind: "toggle" | "pin" | "follow" | "rollback") => {
    setBusy(true);
    setNotice(null);
    const answer =
      kind === "rollback"
        ? await request("rollback_skill_binding", {
            path: { id: binding.id },
            body: { expected_revision: binding.revision, version_id: selected },
            idempotencyKey: idempotencyKey(),
          })
        : await request("update_skill_binding", {
            path: { id: binding.id },
            body: {
              expected_revision: binding.revision,
              enabled: kind === "toggle" ? !binding.enabled : binding.enabled,
              pinned_version_id:
                kind === "follow" ? null : kind === "pin" ? selected : binding.pinned_version_id,
              reason:
                kind === "toggle" ? (binding.enabled ? "disable" : "enable") : kind,
            },
            idempotencyKey: idempotencyKey(),
          });
    setBusy(false);
    const notice = noticeOf(answer);
    setNotice(notice);
    if (notice.result?.outcome === "applied") invalidate("skills/bindings", "skills/available");
  };

  return (
    <>
      <dl className="facts compact">
        <dt>Status</dt>
        <dd>{binding.enabled ? "enabled" : "disabled"}</dd>
        <dt>Version</dt>
        <dd>{binding.pinned_version_id ? "pinned" : "follows current"}</dd>
        <dt>Revision</dt>
        <dd>{binding.revision}</dd>
      </dl>
      {scope.canWrite ? (
        <div className="binding-actions">
          <button type="button" disabled={busy} onClick={() => void change("toggle")}>
            {binding.enabled ? "Disable" : "Enable"}
          </button>
          <label>
            Exact version
            <select value={selected} onChange={(event) => setSelected(event.target.value)}>
              {versions.map((version) => (
                <option key={version.id} value={version.id}>
                  v{version.ordinal} · {version.bundle_digest.slice(0, 12)}
                </option>
              ))}
            </select>
          </label>
          <button type="button" disabled={busy || !selected} onClick={() => void change("pin")}>
            Pin selected
          </button>
          <button type="button" disabled={busy} onClick={() => void change("follow")}>
            Follow current
          </button>
          <button
            type="button"
            disabled={busy || !canRollback}
            onClick={() => void change("rollback")}
          >
            Roll back binding
          </button>
        </div>
      ) : (
        <p className="muted">Policy does not offer binding changes at this scope.</p>
      )}
      <MutationNotice notice={notice} />
    </>
  );
}

function NewVersionForm({ skill }: { skill: SkillView }) {
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
  const [notice, setNotice] = useState<Notice | null>(null);
  const [busy, setBusy] = useState(false);

  const loadCurrent = async () => {
    setBusy(true);
    setNotice(null);
    const listing = await request("list_skill_version_files", {
      path: { id: skill.id, version_id: skill.current_version_id },
    });
    if (listing.kind !== "ok") {
      setBusy(false);
      setNotice({ error: failedAnswerMessage(listing) });
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
        setNotice({ error: failedAnswerMessage(answer) });
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
      setNotice({ error: errorMessage(cause) });
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
    if (notice.result?.outcome === "applied") invalidate("skills");
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
      <MutationNotice notice={notice} />
    </section>
  );
}

function VersionExplorer({
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

function FixtureTests({
  skill,
  version,
  canRun,
}: {
  skill: SkillView;
  version: SkillVersionView;
  canRun: boolean;
}) {
  const key = `skills/item/${skill.id}/versions/${version.id}/tests`;
  const entry = useQuery(key, () =>
    request("list_skill_tests", {
      path: { id: skill.id, version_id: version.id },
      query: { limit: "50" },
    }),
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    setNotice(null);
    const answer = await request("run_skill_test", {
      path: { id: skill.id, version_id: version.id },
      body: { harness: "validation_sandbox" },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    if (answer.kind === "ok") {
      setNotice("Validation sandbox passed and recorded immutable evidence.");
      invalidate(key);
    } else {
      setNotice(failedAnswerMessage(answer));
    }
  };
  return (
    <section>
      <h4>Fixture testing</h4>
      <div className="banner warning">
        The built-in <strong>validation sandbox</strong> parses, scans and scores this exact bundle.
        It executes no Skill scripts. Controlled-client results are labelled separately by the
        adapter harness that observed them.
      </div>
      {canRun ? (
        <p>
          <button type="button" disabled={busy} onClick={() => void run()}>
            {busy ? "Running…" : "Run validation sandbox"}
          </button>
        </p>
      ) : null}
      {notice ? <div className="banner" role="status">{notice}</div> : null}
      <Loaded<SkillTestRunListView> entry={entry} what="Skill test history">
        {(body) =>
          body.runs.length === 0 ? (
            <p className="muted">No controlled test evidence has been recorded.</p>
          ) : (
            <ul className="evidence-list">
              {body.runs.map((run) => (
                <li key={run.id}>
                  <strong>{run.outcome}</strong> · {run.harness.replaceAll("_", " ")} ·{" "}
                  {run.harness_version} · {whenOf(run.created_at)}
                  <JsonValue value={run.evidence} empty="No evidence fields." />
                </li>
              ))}
            </ul>
          )
        }
      </Loaded>
    </section>
  );
}

function UsageEvidence({ skill, version }: { skill: SkillView; version: SkillVersionView }) {
  const key = `skills/item/${skill.id}/versions/${version.id}/usage`;
  const entry = useQuery(key, () =>
    request("list_skill_usage", {
      path: { id: skill.id, version_id: version.id },
      query: { limit: "100" },
    }),
  );
  return (
    <section>
      <h4>Recent activation evidence</h4>
      <Loaded<SkillUsageListView> entry={entry} what="Skill usage evidence">
        {(body) => {
          const counts = activationEvidence(body.events);
          return body.events.length === 0 ? (
            <p className="muted">
              No usage evidence has been recorded. Absence of evidence is not proof that a model
              did not mention the Skill.
            </p>
          ) : (
            <>
              <p className="muted">
                {counts.activated} activations · {counts.hostObserved} host-observed events ·{" "}
                {counts.modelReported} model-reported events
              </p>
              <ul className="evidence-list">
                {body.events.map((event) => (
                  <li key={event.id}>
                    <strong>{event.stage.replaceAll("_", " ")}</strong>{" "}
                    <span className={`evidence ${event.evidence}`}>{evidenceLabel(event.evidence)}</span>
                    {event.resource_path ? ` · ${event.resource_path}` : ""} ·{" "}
                    {whenOf(event.occurred_at)}
                    {event.session_id ? (
                      <div className="muted">session {event.session_id}</div>
                    ) : null}
                  </li>
                ))}
              </ul>
            </>
          );
        }}
      </Loaded>
    </section>
  );
}

function SensitivitySelect({
  value,
  onChange,
}: {
  value: Sensitivity;
  onChange: (value: Sensitivity) => void;
}) {
  return (
    <label>
      Sensitivity
      <select value={value} onChange={(event) => onChange(event.target.value as Sensitivity)}>
        {(["public", "internal", "confidential", "restricted"] as const).map((level) => (
          <option key={level}>{level}</option>
        ))}
      </select>
    </label>
  );
}

function MutationNotice({ notice }: { notice: Notice | null }) {
  if (!notice) return null;
  if (notice.error) {
    return (
      <div className="banner error" role="alert">
        {notice.error}
      </div>
    );
  }
  if (!notice.result) return null;
  return (
    <div
      className={`banner ${notice.result.outcome === "rejected" ? "error" : "success"}`}
      role="status"
    >
      {skillMutationMessage(notice.result)} Change {notice.result.change_id}.{" "}
      {notice.result.outcome === "pending_review" ? (
        <Link href={hrefOf("reviews")}>Open Advanced Reviews</Link>
      ) : null}
    </div>
  );
}

function JsonValue({ value, empty }: { value: Record<string, unknown>; empty: string }) {
  if (Object.keys(value).length === 0) return <p className="muted">{empty}</p>;
  return <pre className="json-value">{JSON.stringify(value, null, 2)}</pre>;
}

function noticeOf(answer: Answer<SkillMutationView>): Notice {
  return answer.kind === "ok" ? { result: answer.body } : { error: failedAnswerMessage(answer) };
}

function appendSkills(before: SkillView[], next: SkillView[]): SkillView[] {
  const byId = new Map(before.map((skill) => [skill.id, skill]));
  for (const skill of next) byId.set(skill.id, skill);
  return [...byId.values()];
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : "The bundle could not be read.";
}

function failedAnswerMessage(answer: Exclude<Answer<unknown>, { kind: "ok" }>): string {
  return answer.kind === "unauthenticated" ? "Your session has expired." : answer.message;
}

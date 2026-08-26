/**
 * Skills Library (CPR-24, ADR-0085).
 *
 * This is a view over CPR-23's stable Skill aggregates, immutable versions
 * and revisioned bindings. It calls only generated public operations. Every
 * offered write still becomes a VedaFlow change at the gateway; capability
 * forecasts here improve the product but grant no authority.
 */

import { useEffect, useMemo, useState, type FormEvent } from "react";

import { idempotencyKey, request } from "./client.mjs";
import { invalidate, Loaded, useQuery, useRefresh } from "./Query.js";
import { Link, navigate } from "./Router.js";
import { PageHeading, useApp } from "./Shell.js";
import { whenOf } from "./people.mjs";
import { hrefOf } from "./routes.mjs";
import {
  formatBundleFiles,
  manifestSummary,
  mayWriteAt,
  parseBundleFiles,
  skillScopes,
  sourceLabel,
  type SkillScopeOption,
} from "./skills.mjs";
import { BindingPanel } from "./skills/bindings.js";
import { NewVersionForm, VersionExplorer } from "./skills/versions.js";
import {
  applyMutationOutcome,
  errorMessage,
  MutationNoticeView,
  noticeOf,
  SensitivitySelect,
  type MutationNotice,
  type Sensitivity,
} from "./skills/ui.js";
import type {
  AvailableSkillListView,
  SkillFileBody,
  SkillListView,
  SkillVersionListView,
  SkillVersionView,
  SkillView,
} from "./generated/api.js";

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
  const [notice, setNotice] = useState<MutationNotice | null>(null);
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
      setNotice({ kind: "error", message: errorMessage(cause) });
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
    applyMutationOutcome(result, ["skills"], {
      invalidate,
      navigateToSkill: (skillId) => navigate(hrefOf("skill-item", { skill_id: skillId })),
    });
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
      <MutationNoticeView notice={notice} />
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

function appendSkills(before: SkillView[], next: SkillView[]): SkillView[] {
  const byId = new Map(before.map((skill) => [skill.id, skill]));
  for (const skill of next) byId.set(skill.id, skill);
  return [...byId.values()];
}

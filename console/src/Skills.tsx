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
  AvailableSkillView,
  SkillFileBody,
  SkillBindingListView,
  SkillBindingView,
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
      <details className="technical-details">
        <summary>What the Skill states mean</summary>
        <ul>
          <li><strong>Installed/current</strong> is the aggregate's applied immutable head.</li>
          <li><strong>Bound</strong> is configured placement; disabled bindings stay configured.</li>
          <li><strong>Available</strong> is an enabled binding resolved through a live policy read.</li>
          <li><strong>Tested</strong> requires an immutable validation run; usage is separate host evidence.</li>
          <li>A pending VedaFlow change is not installed. Skill versions expose no separate quarantine state.</li>
        </ul>
      </details>
      <div className="knowledge-toolbar">
        {scopes.some((scope) => scope.canWrite) ? (
          <button type="button" onClick={() => setInstalling((value) => !value)}>
            {installing ? "Close installer" : "Install Skill"}
          </button>
        ) : (
          <span className="muted">Skill installation is read-only at the visible placements.</span>
        )}
      </div>
      {installing ? <InstallSkill scopes={scopes} /> : null}
      <Loaded<SkillListView> entry={entry} what="the Skill catalogue" onRetry={retry}>
        {(body) => {
          const skills = appendSkills(seen, body.skills);
          return (
            <>
              {skills.length === 0 ? (
                <>
                  <h2>Installed Skills</h2>
                  <p className="muted">
                    No installed Skill is visible under this policy. A denied aggregate is omitted,
                    so this does not disclose whether one exists elsewhere.
                  </p>
                </>
              ) : scopes.length > 0 ? (
                <SkillCatalogue
                  key={project?.id ?? "personal"}
                  skills={skills}
                  scopes={scopes}
                />
              ) : (
                <>
                  <div className="banner warning">
                    No principal or selected-project scope is visible, so binding status cannot be
                    resolved here.
                  </div>
                  <h2>Installed Skills</h2>
                  <SkillRows skills={skills} status={() => null} />
                </>
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

type PlacementStatus = { label: string; tone: "done" | "warn" | "neutral" };

function SkillCatalogue({ skills, scopes }: { skills: SkillView[]; scopes: SkillScopeOption[] }) {
  const preferred = scopes.find((scope) => scope.kind === "project") ?? scopes[0] as SkillScopeOption;
  const [scopeId, setScopeId] = useState(preferred.id);
  useEffect(() => {
    if (!scopes.some((scope) => scope.id === scopeId)) {
      setScopeId((scopes.find((scope) => scope.kind === "project") ?? scopes[0] as SkillScopeOption).id);
    }
  }, [scopeId, scopes]);
  const scope = scopes.find((candidate) => candidate.id === scopeId) ?? preferred;
  const bindingsKey = `skills/bindings/${scope.id}`;
  const availableKey = `skills/available/${scope.id}`;
  const bindings = useQuery(bindingsKey, () =>
    request("list_skill_bindings", { query: { scope_id: scope.id, limit: "200" } }),
  );
  const available = useQuery(availableKey, () =>
    request("list_available_skills", { query: { scope_id: scope.id } }),
  );
  const refreshBindings = useRefresh(bindingsKey);
  const refreshAvailable = useRefresh(availableKey);
  const bindingRows =
    bindings.status === "ready" && bindings.outcome.kind === "ok"
      ? (bindings.outcome.body as SkillBindingListView).bindings
      : null;
  const availableRows =
    available.status === "ready" && available.outcome.kind === "ok"
      ? (available.outcome.body as AvailableSkillListView).skills
      : null;

  return (
    <section className="skill-catalogue">
      <div className="section-heading">
        <div>
          <h2>Installed Skills</h2>
          <p className="muted">
            Each row shows its current immutable version and the exact version a Session can
            discover at the selected placement.
          </p>
        </div>
        <label>
          <span className="switcher-label">Binding status at</span>
          <select value={scope.id} onChange={(event) => setScopeId(event.target.value)}>
            {scopes.map((option) => (
              <option key={option.id} value={option.id}>{option.label}</option>
            ))}
          </select>
        </label>
      </div>
      <div className="availability-read">
        <h3>Available to a session</h3>
        <Loaded<SkillBindingListView>
          entry={bindings}
          what="Skill bindings at this placement"
          onRetry={refreshBindings}
        >
          {() => (
            <Loaded<AvailableSkillListView>
              entry={available}
              what="session Skill availability"
              onRetry={refreshAvailable}
            >
              {(body) => (
                <p className="muted">
                  {body.skills.length === 0
                    ? `No enabled, policy-visible binding resolves at ${scope.label}.`
                    : `${body.skills.length} ${body.skills.length === 1 ? "Skill resolves" : "Skills resolve"} to an exact version at ${scope.label}.`}
                </p>
              )}
            </Loaded>
          )}
        </Loaded>
      </div>
      <SkillRows
        skills={skills}
        status={(skill) => placementStatus(skill, scope, scopes, bindingRows, availableRows)}
      />
    </section>
  );
}

function SkillRows({
  skills,
  status,
}: {
  skills: SkillView[];
  status: (skill: SkillView) => PlacementStatus | null;
}) {
  return (
    <ul className="skill-library-list">
      {skills.map((skill) => (
        <SkillRow key={skill.id} skill={skill} placement={status(skill)} />
      ))}
    </ul>
  );
}

function SkillRow({ skill, placement }: { skill: SkillView; placement: PlacementStatus | null }) {
  const manifest = manifestSummary(skill.current_version.manifest);
  return (
    <li className="skill-library-row">
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
        <div className="skill-binding-status">
          <span className={`tag ${placement?.tone === "neutral" ? "" : placement?.tone ?? ""}`}>
            {placement?.label ?? "Binding status unavailable"}
          </span>
        </div>
      </Link>
      <details className="technical-details row-details">
        <summary>Version integrity evidence</summary>
        <div className="mono breakable">{skill.current_version.bundle_digest}</div>
      </details>
    </li>
  );
}

function placementStatus(
  skill: SkillView,
  scope: SkillScopeOption,
  scopes: SkillScopeOption[],
  bindings: SkillBindingView[] | null,
  available: AvailableSkillView[] | null,
): PlacementStatus | null {
  const resolved = available?.find((candidate) => candidate.binding.skill_id === skill.id);
  if (resolved) {
    const source = scopes.find((candidate) => candidate.id === resolved.binding.scope_id)?.label ??
      "another visible placement";
    return {
      label: `${skill.name} v${resolved.version.ordinal} available · ${resolved.binding.pinned_version_id ? "pinned" : "follows current"} · ${source}`,
      tone: "done",
    };
  }
  const binding = bindings?.find((candidate) => candidate.skill_id === skill.id);
  if (binding && !binding.enabled) {
    return { label: `Disabled · ${scope.label}`, tone: "warn" };
  }
  if (binding) {
    return { label: `Enabled binding has no policy-visible resolution · ${scope.label}`, tone: "warn" };
  }
  if (bindings && available) {
    return { label: `Not bound · ${scope.label}`, tone: "neutral" };
  }
  return null;
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

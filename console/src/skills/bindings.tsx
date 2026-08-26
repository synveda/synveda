import { useState } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import type { SkillScopeOption } from "../skills.mjs";
import {
  applyMutationOutcome,
  MutationNoticeView,
  noticeOf,
  type MutationNotice,
} from "./ui.js";
import type {
  AvailableSkillListView,
  SkillBindingListView,
  SkillBindingView,
  SkillVersionView,
  SkillView,
} from "../generated/api.js";

export function BindingPanel({
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
  const [notice, setNotice] = useState<MutationNotice | null>(null);
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
    applyMutationOutcome(notice, ["skills/bindings", "skills/available"], { invalidate });
  };
  return (
    <>
      <p className="muted">No binding exists.</p>
      <button type="button" disabled={busy} onClick={() => void create()}>
        {busy ? "Submitting…" : "Bind and follow current"}
      </button>
      <MutationNoticeView notice={notice} />
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
  const [notice, setNotice] = useState<MutationNotice | null>(null);
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
    applyMutationOutcome(notice, ["skills/bindings", "skills/available"], { invalidate });
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
      <MutationNoticeView notice={notice} />
    </>
  );
}

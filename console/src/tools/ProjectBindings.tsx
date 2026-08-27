import { useState } from "react";

import { idempotencyKey, request } from "../client.mjs";
import { invalidate, Loaded, useQuery } from "../Query.js";
import { whenOf } from "../people.mjs";
import { displayJson } from "../tools.mjs";
import type {
  ProjectView,
  ToolBindingListView,
  ToolBindingView,
  ToolClientConfigurationView,
  ToolServerVersionView,
  ToolServerView,
} from "../generated/api.js";
import {
  mutationNotice,
  ToolMutationNotice,
  type ToolMutationNoticeValue,
} from "./forms.js";

export function ProjectBindingPanel({
  server,
  versions,
  project,
  canWrite,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  project: ProjectView | null;
  canWrite: boolean;
}) {
  if (!project) {
    return (
      <section>
        <h3>Project binding</h3>
        <p className="muted">Select a project to inspect or change its exact binding.</p>
      </section>
    );
  }
  return (
    <ProjectBindingQuery
      server={server}
      versions={versions}
      project={project}
      canWrite={canWrite}
    />
  );
}

function ProjectBindingQuery({
  server,
  versions,
  project,
  canWrite,
}: {
  server: ToolServerView;
  versions: ToolServerVersionView[];
  project: ProjectView;
  canWrite: boolean;
}) {
  const key = `tools/bindings/${project.id}`;
  const entry = useQuery(key, () =>
    request("list_tool_bindings", {
      query: { project_id: project.id, include_removed: "true", limit: "200" },
    }),
  );
  const approved = versions.filter((version) => version.state === "approved");
  return (
    <section>
      <h3>Project binding · {project.display_name}</h3>
      <p className="muted">
        A binding names one exact approved digest. Approving a newer version never changes what
        this project advertises.
      </p>
      <Loaded<ToolBindingListView> entry={entry} what="this project's MCP bindings">
        {(body) => {
          const binding = body.bindings.find((candidate) => candidate.server_id === server.id);
          return binding ? (
            <ExistingBinding binding={binding} approved={approved} canWrite={canWrite} />
          ) : (
            <CreateBinding
              server={server}
              project={project}
              approved={approved}
              canWrite={canWrite}
            />
          );
        }}
      </Loaded>
      {versions.some((version) => version.state === "quarantined") ? (
        <p className="muted">
          Quarantined versions are deliberately absent from the binding picker. Review their
          VedaFlow change first.
        </p>
      ) : null}
    </section>
  );
}

function CreateBinding({
  server,
  project,
  approved,
  canWrite,
}: {
  server: ToolServerView;
  project: ProjectView;
  approved: ToolServerVersionView[];
  canWrite: boolean;
}) {
  const [selected, setSelected] = useState(
    approved.find((version) => version.id === server.current_version_id)?.id ??
      approved[0]?.id ??
      "",
  );
  const [notice, setNotice] = useState<ToolMutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  if (approved.length === 0) {
    return <p className="muted">No approved version exists, so this server cannot be bound.</p>;
  }
  if (!canWrite) {
    return <p className="muted">No binding exists, and policy does not offer creation here.</p>;
  }

  const create = async () => {
    setBusy(true);
    setNotice(null);
    const answer = await request("create_tool_binding", {
      body: {
        project_id: project.id,
        server_id: server.id,
        version_id: selected,
        state: "enabled",
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.kind === "result" && result.value.outcome === "applied") {
      invalidate("tools/bindings", `tools/config/${project.id}`);
    }
  };

  return (
    <div className="binding-actions">
      <label>
        Exact approved version
        <select value={selected} onChange={(event) => setSelected(event.target.value)}>
          {approved.map((version) => (
            <option key={version.id} value={version.id}>
              v{version.ordinal} · {version.digest.slice(0, 12)}
            </option>
          ))}
        </select>
      </label>
      <button type="button" disabled={busy || !selected} onClick={() => void create()}>
        {busy ? "Submitting…" : "Bind exact version"}
      </button>
      <ToolMutationNotice notice={notice} />
    </div>
  );
}

function ExistingBinding({
  binding,
  approved,
  canWrite,
}: {
  binding: ToolBindingView;
  approved: ToolServerVersionView[];
  canWrite: boolean;
}) {
  const [selected, setSelected] = useState(binding.version_id);
  const [notice, setNotice] = useState<ToolMutationNoticeValue | null>(null);
  const [busy, setBusy] = useState(false);
  const change = async (
    state: ToolBindingView["state"],
    reason: "disable" | "enable" | "repin" | "remove",
    versionId = binding.version_id,
  ) => {
    setBusy(true);
    setNotice(null);
    const answer = await request("update_tool_binding", {
      path: { id: binding.id },
      body: {
        expected_revision: binding.revision,
        version_id: versionId,
        state,
        reason,
      },
      idempotencyKey: idempotencyKey(),
    });
    setBusy(false);
    const result = mutationNotice(answer);
    setNotice(result);
    if (result.kind === "result" && result.value.outcome === "applied") {
      invalidate("tools/bindings", `tools/config/${binding.project_id}`);
    }
  };

  return (
    <div className="tool-binding-card">
      <dl className="facts compact">
        <dt>Status</dt>
        <dd>{binding.state}</dd>
        <dt>Exact version</dt>
        <dd className="mono breakable">{binding.version_id}</dd>
        <dt>Revision</dt>
        <dd>{binding.revision}</dd>
        <dt>Updated</dt>
        <dd>{whenOf(binding.updated_at)}</dd>
      </dl>
      {canWrite ? (
        <div className="binding-actions">
          {binding.state === "enabled" ? (
            <button type="button" disabled={busy} onClick={() => void change("disabled", "disable")}>
              Disable
            </button>
          ) : (
            <button type="button" disabled={busy} onClick={() => void change("enabled", "enable")}>
              {binding.state === "removed" ? "Restore binding" : "Enable"}
            </button>
          )}
          <label>
            Exact approved version
            <select value={selected} onChange={(event) => setSelected(event.target.value)}>
              {approved.map((version) => (
                <option key={version.id} value={version.id}>
                  v{version.ordinal} · {version.digest.slice(0, 12)}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            disabled={busy || selected === binding.version_id}
            onClick={() => void change(binding.state, "repin", selected)}
          >
            Repin exact version
          </button>
          {binding.state !== "removed" ? (
            <button type="button" disabled={busy} onClick={() => void change("removed", "remove")}>
              Remove binding
            </button>
          ) : null}
        </div>
      ) : (
        <p className="muted">Policy does not offer binding changes at this project scope.</p>
      )}
      <ToolMutationNotice notice={notice} />
    </div>
  );
}

export function ProjectConfiguration({ project }: { project: ProjectView }) {
  const key = `tools/config/${project.id}`;
  const entry = useQuery(key, () =>
    request("generate_tool_client_config", { path: { project_id: project.id } }),
  );
  return (
    <section className="tool-configuration">
      <h2>Generated client configuration · {project.display_name}</h2>
      <p className="muted">
        Only enabled exact approved bindings appear. The browser masks opaque secret-reference
        identifiers; a trusted adapter resolves them outside the gateway.
      </p>
      <Loaded<ToolClientConfigurationView> entry={entry} what="secret-safe client configuration">
        {(body) => (
          <>
            <p>
              {body.bindings.length} exact binding{body.bindings.length === 1 ? "" : "s"}.
            </p>
            {body.bindings.length > 0 ? (
              <ul className="inline-list">
                {body.bindings.map((binding) => (
                  <li key={binding.binding_id}>
                    <span className="mono">{binding.digest.slice(0, 12)}</span> · exact{" "}
                    {binding.version_id}
                  </li>
                ))}
              </ul>
            ) : null}
            <pre className="json-value">{displayJson(body.configuration)}</pre>
          </>
        )}
      </Loaded>
    </section>
  );
}

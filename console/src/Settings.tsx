/**
 * Settings (CPR-8): this workspace, this project, and what the project is
 * about.
 *
 * Three mutations, all of them on the contract plane, all of them under the
 * two rules CPR-4 put on it (ADR-0071 decisions 5 and 6):
 *
 * - **Every update names the revision it saw.** `expected_revision` is
 *   required by the API, and it is required here for the reason it is
 *   required there: an update without a precondition is a last-writer-wins
 *   update, and two people editing one workspace description is not an
 *   exotic case. A 409 is rendered as what it is — somebody else changed
 *   this — with the current values still on screen.
 * - **Every creation carries an idempotency key**, minted when the form is
 *   submitted rather than when the request is sent, so a retry of one
 *   attempt replays instead of creating a second project.
 *
 * A repository is identified by its **canonical remote**, never by a path
 * (ADR-0071 decision 4), and the form says so rather than letting somebody
 * paste `/Users/me/src/thing` and receive a validation error they have to
 * decode.
 */

import { useCallback, useState } from "react";

import { idempotencyKey, request } from "./client.mjs";
import { Loaded, invalidate, useQuery, useRefresh } from "./Query.js";
import { PageHeading, useApp } from "./Shell.js";
import { ME_KEY } from "./App.js";
import { slugFrom } from "./onboarding.mjs";
import { whenOf } from "./people.mjs";
import type { RepositoryList } from "./generated/api.js";

export function Settings() {
  const { workspace, project } = useApp();
  return (
    <>
      <PageHeading route="settings" />
      {workspace ? (
        <WorkspaceSettings />
      ) : (
        <p className="muted">No workspace selected.</p>
      )}
      {workspace ? <ProjectSection /> : null}
      {project ? <Repositories projectId={project.id} /> : null}
    </>
  );
}

function WorkspaceSettings() {
  const { workspace, reload } = useApp();
  const [displayName, setDisplayName] = useState(workspace?.display_name ?? "");
  const [description, setDescription] = useState(workspace?.description ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);

  const save = useCallback(async () => {
    if (!workspace) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    const outcome = await request("update_workspace", {
      path: { workspace_id: workspace.id },
      body: {
        expected_revision: workspace.revision,
        display_name: displayName.trim(),
        // Three cases on the wire and the form says them apart: an empty
        // box clears the description (`null`), text replaces it.
        description: description.trim().length > 0 ? description.trim() : null,
      },
    });
    setBusy(false);
    if (outcome.kind !== "ok") {
      setError(
        outcome.kind === "conflict"
          ? `${outcome.message} — somebody else changed this workspace. Reload to see theirs.`
          : outcome.kind === "unauthenticated"
            ? "Your session has expired."
            : outcome.message,
      );
      return;
    }
    setSaved(true);
    reload();
  }, [workspace, displayName, description, reload]);

  if (!workspace) return null;
  return (
    <section>
      <h2>Workspace</h2>
      <p className="muted">
        {workspace.slug} · revision {workspace.revision} · scope{" "}
        {workspace.scope_id.slice(0, 8)} · created {whenOf(workspace.created_at)}
      </p>
      <form
        className="stacked-form"
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <label>
          <span className="switcher-label">Display name</span>
          <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} />
        </label>
        <label>
          <span className="switcher-label">Description</span>
          <textarea
            value={description}
            rows={3}
            onChange={(event) => setDescription(event.target.value)}
          />
        </label>
        <p className="muted">
          The handle <code>{workspace.slug}</code> is immutable — it is the scope's slug too, and
          renaming it would rename what every grant, version and audit event points at.
        </p>
        <div>
          <button type="submit" disabled={busy}>
            Save
          </button>{" "}
          {saved ? <span className="muted">saved</span> : null}
          {error ? <span className="form-error">{error}</span> : null}
        </div>
      </form>
    </section>
  );
}

function ProjectSection() {
  const { workspace, project, reload, chooseProject } = useApp();
  const [displayName, setDisplayName] = useState("");
  const [slug, setSlug] = useState("");
  const [touchedSlug, setTouchedSlug] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const create = useCallback(async () => {
    if (!workspace) return;
    const name = displayName.trim();
    const handle = (touchedSlug ? slug : slugFrom(name)).trim();
    if (name.length === 0 || handle.length === 0) return;
    setBusy(true);
    setError(null);
    const outcome = await request("create_project", {
      path: { workspace_id: workspace.id },
      idempotencyKey: idempotencyKey(),
      body: { display_name: name, slug: handle },
    });
    setBusy(false);
    if (outcome.kind !== "ok") {
      setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
      return;
    }
    setDisplayName("");
    setSlug("");
    setTouchedSlug(false);
    chooseProject(outcome.body.id);
    invalidate(ME_KEY);
    reload();
  }, [workspace, displayName, slug, touchedSlug, chooseProject, reload]);

  return (
    <section>
      <h2>Project</h2>
      {project ? (
        <p className="muted">
          {project.display_name} ({project.slug}) · revision {project.revision} · scope{" "}
          {project.scope_id.slice(0, 8)}
        </p>
      ) : (
        <p className="muted">No project selected.</p>
      )}
      <h3>New project</h3>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <label>
          <span className="switcher-label">Name</span>
          <input
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Payments"
          />
        </label>
        <label>
          <span className="switcher-label">Handle</span>
          <input
            value={touchedSlug ? slug : slugFrom(displayName)}
            onChange={(event) => {
              setTouchedSlug(true);
              setSlug(event.target.value);
            }}
            placeholder="payments"
          />
        </label>
        <button type="submit" disabled={busy || displayName.trim().length === 0}>
          Create
        </button>
        {error ? <span className="form-error">{error}</span> : null}
      </form>
    </section>
  );
}

function Repositories({ projectId }: { projectId: string }) {
  const cacheKey = `projects/${projectId}/repositories`;
  const entry = useQuery(cacheKey, () =>
    request("list_repositories", { path: { project_id: projectId } }),
  );
  const retry = useRefresh(cacheKey);
  const [remote, setRemote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const attach = useCallback(async () => {
    const uri = remote.trim();
    if (uri.length === 0) return;
    setBusy(true);
    setError(null);
    const outcome = await request("attach_repository", {
      path: { project_id: projectId },
      idempotencyKey: idempotencyKey(),
      body: { remote_uri: uri },
    });
    setBusy(false);
    if (outcome.kind !== "ok") {
      setError(outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message);
      return;
    }
    setRemote("");
    invalidate(cacheKey);
  }, [remote, projectId, cacheKey]);

  const detach = useCallback(
    async (repositoryId: string) => {
      setBusy(true);
      setError(null);
      const outcome = await request("detach_repository", {
        path: { project_id: projectId, repository_id: repositoryId },
      });
      setBusy(false);
      if (outcome.kind !== "ok") {
        setError(
          outcome.kind === "unauthenticated" ? "Your session has expired." : outcome.message,
        );
        return;
      }
      invalidate(cacheKey);
    },
    [projectId, cacheKey],
  );

  return (
    <section>
      <h2>Repositories</h2>
      <p className="muted">
        What this project is about, by <strong>canonical remote</strong>. Transports,
        credentials, ports and a <code>.git</code> suffix all collapse to one identity — and a
        filesystem path is never one, because it differs per machine and changes when somebody
        moves a directory.
      </p>
      {error ? (
        <div className="banner error" role="alert">
          {error}
        </div>
      ) : null}
      <Loaded<RepositoryList> entry={entry} what="the repositories" onRetry={retry}>
        {(body) =>
          body.repositories.length === 0 ? (
            <p className="muted">None attached.</p>
          ) : (
            <ul className="repositories">
              {body.repositories.map((repository) => (
                <li key={repository.id}>
                  <span className="tag">{repository.provider}</span>{" "}
                  <span className="mono breakable">{repository.canonical_uri}</span>
                  <div className="muted">
                    {repository.repository_owner
                      ? `${repository.repository_owner}/${repository.repository_name}`
                      : repository.repository_name}
                    {repository.default_branch ? ` · ${repository.default_branch}` : ""} · attached{" "}
                    {whenOf(repository.created_at)}
                  </div>
                  <button type="button" disabled={busy} onClick={() => void detach(repository.id)}>
                    Detach
                  </button>
                </li>
              ))}
            </ul>
          )
        }
      </Loaded>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault();
          void attach();
        }}
      >
        <label>
          <span className="switcher-label">Remote</span>
          <input
            value={remote}
            onChange={(event) => setRemote(event.target.value)}
            placeholder="https://github.com/acme/payments — or git@github.com:acme/payments.git"
          />
        </label>
        <button type="submit" disabled={busy || remote.trim().length === 0}>
          Attach
        </button>
      </form>
    </section>
  );
}

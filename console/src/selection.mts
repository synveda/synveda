/**
 * The current workspace and project (CPR-8, ADR-0075 decision 2).
 *
 * Two ids, remembered across reloads, reconciled against what the caller
 * may actually read. Everything interesting is in the reconciliation, and
 * it is pure so that a test can drive it.
 *
 * # Why the selection is not in the URL
 *
 * Because it is a property of the *reader*, not of the page. Somebody who
 * opens People and then Settings expects the same project on both; a
 * selection in the query string makes every internal link carry it, and
 * every link somebody copies then pins a project the recipient may not be
 * able to read. Persisted per browser, reconciled on every load, is the
 * behaviour a person actually wants from a switcher.
 *
 * # Why it is reconciled rather than trusted
 *
 * A stored id can name a workspace that was archived, that this person lost
 * access to, or that belongs to a tenant they are no longer signed in to.
 * Rendering a page against an id `/v1/me` did not return would produce a
 * screen of 404s that looks like a broken product. So the stored value is a
 * *preference*, and what is in `MeView` is the truth: if the preference is
 * still there it wins, and otherwise the first thing that is.
 */

import type { MeView, ProjectView, WorkspaceView } from "./generated/api.js";

export interface Selection {
  workspaceId: string | null;
  projectId: string | null;
}

/** Nothing chosen — the state a first-run deployment is in. */
export const NOTHING: Selection = { workspaceId: null, projectId: null };

/** Where the preference is kept. Namespaced, because the origin is shared. */
const STORAGE_KEY = "synveda.console.selection";

/**
 * The subset of `Storage` this uses, so a test can pass a map and a private
 * browsing window that throws on write can be handled rather than crash.
 */
export interface SelectionStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Reads the stored preference, treating anything unparseable as none. */
export function readStored(store: SelectionStore | null): Selection {
  if (!store) return NOTHING;
  try {
    const raw = store.getItem(STORAGE_KEY);
    if (!raw) return NOTHING;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return NOTHING;
    const record = parsed as Record<string, unknown>;
    return {
      workspaceId: typeof record.workspaceId === "string" ? record.workspaceId : null,
      projectId: typeof record.projectId === "string" ? record.projectId : null,
    };
  } catch {
    // A quota error, a disabled store, a value somebody else wrote. None of
    // them is a reason to fail to render: the selection falls back to the
    // first readable workspace, which is where a new reader starts anyway.
    return NOTHING;
  }
}

/** Writes the preference. Failure is silent by design — see {@link readStored}. */
export function writeStored(store: SelectionStore | null, selection: Selection): void {
  if (!store) return;
  try {
    store.setItem(STORAGE_KEY, JSON.stringify(selection));
  } catch {
    /* see readStored */
  }
}

/** Active first, then by slug — the order a switcher offers them in. */
export function orderedWorkspaces(me: MeView): WorkspaceView[] {
  return [...me.workspaces].sort((a, b) => {
    if (a.status !== b.status) return a.status === "active" ? -1 : 1;
    return a.slug.localeCompare(b.slug);
  });
}

/** The projects of one workspace, in the same order. */
export function projectsOf(me: MeView, workspaceId: string | null): ProjectView[] {
  if (!workspaceId) return [];
  return me.projects
    .filter((project) => project.workspace_id === workspaceId)
    .sort((a, b) => {
      if (a.status !== b.status) return a.status === "active" ? -1 : 1;
      return a.slug.localeCompare(b.slug);
    });
}

/**
 * The selection to actually use, given a preference and what exists.
 *
 * The rules, in order:
 *
 * 1. Keep the preferred workspace if the caller can still read it.
 * 2. Otherwise take the first one they can — active before archived.
 * 3. Keep the preferred project if it is in the chosen workspace.
 * 4. Otherwise take that workspace's first project, or none.
 *
 * Rule 3 is the one worth stating: a project id is only meaningful inside
 * its workspace, so switching workspace must drop a project that belonged
 * to the old one rather than carry an id that now names somewhere else.
 */
export function reconcile(preference: Selection, me: MeView): Selection {
  const workspaces = orderedWorkspaces(me);
  const preferred = workspaces.find((workspace) => workspace.id === preference.workspaceId);
  const workspace = preferred ?? workspaces[0] ?? null;
  if (!workspace) {
    return NOTHING;
  }
  const projects = projectsOf(me, workspace.id);
  const keptProject = projects.find((project) => project.id === preference.projectId);
  return {
    workspaceId: workspace.id,
    projectId: (keptProject ?? projects[0])?.id ?? null,
  };
}

/** The chosen workspace, when there is one. */
export function selectedWorkspace(me: MeView, selection: Selection): WorkspaceView | null {
  return me.workspaces.find((workspace) => workspace.id === selection.workspaceId) ?? null;
}

/** The chosen project, when there is one. */
export function selectedProject(me: MeView, selection: Selection): ProjectView | null {
  return me.projects.find((project) => project.id === selection.projectId) ?? null;
}

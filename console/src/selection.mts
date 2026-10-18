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
 * The preference key for one account in one tenant, so a browser shared by
 * two people, or one person in two tenants, never applies one account's
 * remembered selection to another.
 */
export function selectionKey(me: Pick<MeView, "tenant" | "principal">): string {
  return `${STORAGE_KEY}:${me.tenant.id}:${me.principal.subject}`;
}

/**
 * The subset of `Storage` this uses, so a test can pass a map and a private
 * browsing window that throws on write can be handled rather than crash.
 */
export interface SelectionStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** `localStorage`, or nothing at all where it is unavailable. */
export function browserSelectionStore(): SelectionStore | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    // Some browsers throw on the *accessor* when site data is blocked, not
    // just on the read. The console works without a remembered selection;
    // it does not work if the first thing it does is throw.
    return null;
  }
}

/** Reads the stored preference, treating anything unparseable as none. */
export function readStored(store: SelectionStore | null, key: string = STORAGE_KEY): Selection {
  if (!store) return NOTHING;
  try {
    const raw = store.getItem(key);
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
export function writeStored(
  store: SelectionStore | null,
  selection: Selection,
  key: string = STORAGE_KEY,
): void {
  if (!store) return;
  try {
    store.setItem(key, JSON.stringify(selection));
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

/** Active first, then by slug. */
function byStatusThenSlug(a: ProjectView, b: ProjectView): number {
  if (a.status !== b.status) return a.status === "active" ? -1 : 1;
  return a.slug.localeCompare(b.slug);
}

/**
 * Projects readable on their own: their workspace is not one this caller may
 * read. Only the `workspace_id` the project row already discloses is used; no
 * workspace object is made up for it and nothing more about it is fetched.
 */
export function sharedProjects(me: MeView): ProjectView[] {
  const readable = new Set(me.workspaces.map((workspace) => workspace.id));
  return me.projects.filter((project) => !readable.has(project.workspace_id)).sort(byStatusThenSlug);
}

/**
 * The projects offered beside a selection: the selected workspace's, or —
 * when the selection is a project whose workspace is not readable — every
 * project shared that way.
 */
export function projectsOf(me: MeView, workspaceId: string | null): ProjectView[] {
  if (!workspaceId) return [];
  if (!me.workspaces.some((workspace) => workspace.id === workspaceId)) {
    const shared = sharedProjects(me);
    return shared.some((project) => project.workspace_id === workspaceId) ? shared : [];
  }
  return me.projects.filter((project) => project.workspace_id === workspaceId).sort(byStatusThenSlug);
}

/**
 * The selection to actually use, given a preference and what exists.
 *
 * The rules, in order:
 *
 * 1. Keep the preferred workspace if the caller can still read it, with the
 *    preferred project if it is in that workspace, else its first project.
 * 2. Otherwise keep the preferred project if it is readable on its own — a
 *    project shared without its workspace ({@link sharedProjects}).
 * 3. Otherwise take the first readable workspace that has a project, then the
 *    first shared project, then the first readable workspace, or nothing.
 *
 * A project id is only meaningful inside its workspace, so switching
 * workspace drops a project that belonged to the old one rather than carry an
 * id that now names somewhere else. A selection `/v1/me` no longer returns —
 * a revoked grant — is never kept.
 */
export function reconcile(preference: Selection, me: MeView): Selection {
  const workspaces = orderedWorkspaces(me);
  const preferred = workspaces.find((workspace) => workspace.id === preference.workspaceId);
  if (preferred) return inWorkspace(me, preferred.id, preference.projectId);
  const shared = sharedProjects(me);
  const keptShared = shared.find((project) => project.id === preference.projectId);
  if (keptShared) return { workspaceId: keptShared.workspace_id, projectId: keptShared.id };
  const withProject = workspaces.find((workspace) => projectsOf(me, workspace.id).length > 0);
  if (withProject) return inWorkspace(me, withProject.id, null);
  const firstShared = shared[0];
  if (firstShared) return { workspaceId: firstShared.workspace_id, projectId: firstShared.id };
  const first = workspaces[0];
  return first ? { workspaceId: first.id, projectId: null } : NOTHING;
}

function inWorkspace(me: MeView, workspaceId: string, projectId: string | null): Selection {
  const projects = projectsOf(me, workspaceId);
  const kept = projects.find((project) => project.id === projectId);
  return { workspaceId, projectId: (kept ?? projects[0])?.id ?? null };
}

/**
 * The selection that lands on a newly granted scope, when `/v1/me` now shows
 * it: its workspace, or its project. `null` when the grant is not readable
 * yet — the caller explains rather than pretending it is.
 */
export function selectionAt(me: MeView, scopeId: string): Selection | null {
  const workspace = me.workspaces.find((candidate) => candidate.scope_id === scopeId);
  if (workspace) return inWorkspace(me, workspace.id, null);
  const project = me.projects.find((candidate) => candidate.scope_id === scopeId);
  return project ? { workspaceId: project.workspace_id, projectId: project.id } : null;
}

/** The chosen workspace, when there is one. */
export function selectedWorkspace(me: MeView, selection: Selection): WorkspaceView | null {
  return me.workspaces.find((workspace) => workspace.id === selection.workspaceId) ?? null;
}

/** The chosen project, when there is one. */
export function selectedProject(me: MeView, selection: Selection): ProjectView | null {
  return me.projects.find((project) => project.id === selection.projectId) ?? null;
}

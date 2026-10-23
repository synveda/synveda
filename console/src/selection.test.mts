/**
 * The persistent selection (ADR-0075 decision 2).
 *
 * The reconciliation is the whole of it, and every one of these cases is a
 * screen full of 404s if it goes the other way: a stored workspace this
 * person lost access to, a project that belongs to a workspace they just
 * switched away from, a browser that refuses to store anything at all.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  NOTHING,
  orderedWorkspaces,
  projectsOf,
  readStored,
  reconcile,
  selectedProject,
  selectedWorkspace,
  selectionAt,
  selectionKey,
  sharedProjects,
  writeStored,
  type SelectionStore,
} from "./selection.mjs";
import type { MeView, ProjectView, WorkspaceView } from "./generated/api.js";

function workspace(id: string, slug: string, status: "active" | "archived" = "active"): WorkspaceView {
  return {
    id,
    slug,
    display_name: slug,
    status,
    revision: 1,
    scope_id: `scope-${id}`,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

function project(
  id: string,
  workspaceId: string,
  slug: string,
  status: "active" | "archived" = "active",
): ProjectView {
  return {
    id,
    workspace_id: workspaceId,
    slug,
    display_name: slug,
    status,
    revision: 1,
    scope_id: `scope-${id}`,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

function me(workspaces: WorkspaceView[], projects: ProjectView[]): MeView {
  return {
    principal: { subject: "robin@example.test", quarantined: false },
    tenant: { id: "t", slug: "acme", name: "ACME", status: "active" },
    onboarding: {
      state: "ready",
      workspace_count: workspaces.length,
      project_count: projects.length,
      workspace_create: false,
      project_create_workspace_ids: [],
    },
    workspaces,
    projects,
    capabilities: { actions: {}, role_keys: [] },
    anchors: [],
  };
}

/** A `Storage` that is just a map — and one that refuses, for the other case. */
function memoryStore(): SelectionStore & { values: Map<string, string> } {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

test("a preference the caller can still read is kept", () => {
  const state = me([workspace("w-1", "alpha"), workspace("w-2", "beta")], [project("p-1", "w-2", "pay")]);
  const chosen = reconcile({ workspaceId: "w-2", projectId: "p-1" }, state);
  assert.deepEqual(chosen, { workspaceId: "w-2", projectId: "p-1" });
});

test("a preference naming a workspace this caller lost falls back to the first", () => {
  // The failure this prevents: rendering every page against an id `/v1/me`
  // did not return, which looks like a broken product rather than like
  // access that changed.
  const state = me([workspace("w-1", "alpha")], [project("p-1", "w-1", "pay")]);
  const chosen = reconcile({ workspaceId: "gone", projectId: "also-gone" }, state);
  assert.deepEqual(chosen, { workspaceId: "w-1", projectId: "p-1" });
});

test("a project from another workspace is dropped rather than carried", () => {
  // Rule 3. A project id is only meaningful inside its own workspace, so
  // switching must not keep an id that now names somewhere else.
  const state = me(
    [workspace("w-1", "alpha"), workspace("w-2", "beta")],
    [project("p-1", "w-1", "one"), project("p-2", "w-2", "two")],
  );
  const chosen = reconcile({ workspaceId: "w-2", projectId: "p-1" }, state);
  assert.deepEqual(chosen, { workspaceId: "w-2", projectId: "p-2" });
});

test("a workspace with no project selects no project", () => {
  const state = me([workspace("w-1", "alpha")], []);
  assert.deepEqual(reconcile(NOTHING, state), { workspaceId: "w-1", projectId: null });
});

test("a deployment with nothing in it selects nothing", () => {
  assert.deepEqual(reconcile({ workspaceId: "w-1", projectId: "p-1" }, me([], [])), NOTHING);
});

test("active comes before archived, then slug order", () => {
  // An archived workspace is listed — omitting it would make it
  // indistinguishable from one that never existed — but it is never what a
  // fallback lands on while an active one exists.
  const state = me(
    [workspace("w-1", "zulu"), workspace("w-2", "alpha", "archived"), workspace("w-3", "mike")],
    [],
  );
  assert.deepEqual(
    orderedWorkspaces(state).map((w) => w.slug),
    ["mike", "zulu", "alpha"],
  );
  assert.equal(reconcile(NOTHING, state).workspaceId, "w-3");
});

test("projects are listed for their own workspace only", () => {
  const state = me(
    [workspace("w-1", "alpha"), workspace("w-2", "beta")],
    [project("p-1", "w-1", "one"), project("p-2", "w-2", "two"), project("p-3", "w-1", "three")],
  );
  assert.deepEqual(
    projectsOf(state, "w-1").map((p) => p.slug),
    ["one", "three"],
  );
  assert.deepEqual(projectsOf(state, null), []);
});

test("the selection round-trips through storage", () => {
  const store = memoryStore();
  writeStored(store, { workspaceId: "w-1", projectId: "p-1" });
  assert.deepEqual(readStored(store), { workspaceId: "w-1", projectId: "p-1" });
});

test("a browser that stores nothing is not a broken console", () => {
  // Private windows, blocked site data, a quota error. None of them is a
  // reason to fail to render: the selection falls back to the first
  // readable workspace, which is where a new reader starts anyway.
  assert.deepEqual(readStored(null), NOTHING);
  const hostile: SelectionStore = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  assert.deepEqual(readStored(hostile), NOTHING);
  writeStored(hostile, { workspaceId: "w", projectId: null });
  writeStored(null, { workspaceId: "w", projectId: null });
});

test("a stored value somebody else wrote is ignored rather than trusted", () => {
  const store = memoryStore();
  store.values.set("synveda.console.selection", "not json");
  assert.deepEqual(readStored(store), NOTHING);
  store.values.set("synveda.console.selection", JSON.stringify({ workspaceId: 7 }));
  assert.deepEqual(readStored(store), NOTHING);
});

test("the selected rows are looked up rather than assumed", () => {
  const state = me([workspace("w-1", "alpha")], [project("p-1", "w-1", "pay")]);
  assert.equal(selectedWorkspace(state, { workspaceId: "w-1", projectId: null })?.slug, "alpha");
  assert.equal(selectedWorkspace(state, NOTHING), null);
  assert.equal(selectedProject(state, { workspaceId: "w-1", projectId: "p-1" })?.slug, "pay");
  assert.equal(selectedProject(state, { workspaceId: "w-1", projectId: "gone" }), null);
});

test("a project shared without its workspace is selectable on its own", () => {
  // Project-only membership: `/v1/me` returns the project, not its workspace.
  const state = me([], [project("p-1", "w-hidden", "pay")]);
  const chosen = reconcile(NOTHING, state);
  assert.deepEqual(chosen, { workspaceId: "w-hidden", projectId: "p-1" });
  assert.equal(selectedWorkspace(state, chosen), null, "no workspace object is made up");
  assert.equal(selectedProject(state, chosen)?.slug, "pay");
  assert.deepEqual(
    projectsOf(state, "w-hidden").map((p) => p.id),
    ["p-1"],
  );
  assert.deepEqual(
    sharedProjects(state).map((p) => p.id),
    ["p-1"],
  );
});

test("a remembered shared project is kept, and a readable workspace with work comes first", () => {
  const state = me(
    [workspace("w-empty", "alpha"), workspace("w-busy", "beta")],
    [project("p-busy", "w-busy", "one"), project("p-shared", "w-hidden", "two")],
  );
  assert.deepEqual(reconcile({ workspaceId: "w-hidden", projectId: "p-shared" }, state), {
    workspaceId: "w-hidden",
    projectId: "p-shared",
  });
  // With no preference, an authorized project rather than an empty workspace
  // that happens to sort first.
  assert.deepEqual(reconcile(NOTHING, state), { workspaceId: "w-busy", projectId: "p-busy" });
  // A preferred readable workspace still wins, empty or not.
  assert.deepEqual(reconcile({ workspaceId: "w-empty", projectId: null }, state), {
    workspaceId: "w-empty",
    projectId: null,
  });
});

test("revoked access drops the remembered selection on the next reconcile", () => {
  const before = me([workspace("w-1", "alpha")], [project("p-1", "w-1", "pay")]);
  const kept = reconcile({ workspaceId: "w-1", projectId: "p-1" }, before);
  assert.deepEqual(kept, { workspaceId: "w-1", projectId: "p-1" });
  assert.deepEqual(reconcile(kept, me([], [])), NOTHING);
  const projectOnly = me([], [project("p-2", "w-1", "other")]);
  assert.deepEqual(reconcile(kept, projectOnly), { workspaceId: "w-1", projectId: "p-2" });
});

test("a newly granted scope is selected only once /v1/me shows it", () => {
  const state = me(
    [workspace("w-1", "alpha")],
    [project("p-1", "w-1", "pay"), project("p-2", "w-hidden", "shared")],
  );
  assert.deepEqual(selectionAt(state, "scope-w-1"), { workspaceId: "w-1", projectId: "p-1" });
  assert.deepEqual(selectionAt(state, "scope-p-2"), { workspaceId: "w-hidden", projectId: "p-2" });
  assert.equal(selectionAt(state, "scope-not-visible"), null);
});

test("each account keeps its own remembered selection", () => {
  const store = memoryStore();
  const view = me([], []);
  const robin = selectionKey({ ...view, principal: { subject: "robin", quarantined: false } });
  const sam = selectionKey({ ...view, principal: { subject: "sam", quarantined: false } });
  const otherTenant = selectionKey({
    principal: { subject: "robin", quarantined: false },
    tenant: { ...view.tenant, id: "t-2" },
  });
  assert.equal(new Set([robin, sam, otherTenant]).size, 3);
  writeStored(store, { workspaceId: "w-1", projectId: "p-1" }, robin);
  assert.deepEqual(readStored(store, sam), NOTHING);
  assert.deepEqual(readStored(store, robin), { workspaceId: "w-1", projectId: "p-1" });
});

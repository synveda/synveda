import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { Onboarding } from "./Onboarding.js";
import { initialStep, projectTarget } from "./onboarding.mjs";
import { AppProvider, appContext } from "./Shell.js";
import type { MeView } from "./generated/api.js";

function render(
  state: MeView["onboarding"]["state"],
  selectedProject = "project-2",
) {
  const view: MeView = {
    principal: {
      subject: "test-user",
      display_name: "Taylor",
      quarantined: false,
    },
    tenant: { id: "tenant-1", slug: "test", name: "Test", status: "active" },
    onboarding: {
      state,
      workspace_count: state === "needs_workspace" ? 0 : 1,
      project_count: state === "ready" ? 2 : 0,
      workspace_create: state === "needs_workspace",
      project_create_workspace_ids:
        state === "needs_project" || state === "ready" ? ["workspace-1"] : [],
    },
    capabilities: { actions: {}, role_keys: [] },
    anchors: [],
    workspaces:
      state === "needs_workspace" || state === "blocked" || state === "access_required"
        ? []
        : [
            {
              id: "workspace-1",
              display_name: "Test workspace",
              slug: "test",
              status: "active",
              revision: 1,
              scope_id: "workspace-scope",
              created_at: "2026-09-20T00:00:00Z",
              updated_at: "2026-09-20T00:00:00Z",
            },
          ],
    projects:
      state === "ready"
        ? ["project-1", "project-2"].map((id) => ({
            id,
            display_name:
              id === "project-2" ? "Selected project" : "Different project",
            slug: id,
            workspace_id: "workspace-1",
            scope_id: `${id}-scope`,
            status: "active",
            revision: 1,
            created_at: "2026-09-20T00:00:00Z",
            updated_at: "2026-09-20T00:00:00Z",
          }))
        : [],
  };
  const context = appContext(
    view,
    {
      workspaceId: view.workspaces[0]?.id ?? null,
      projectId: state === "ready" ? selectedProject || null : null,
    },
    () => {},
  );
  return renderToStaticMarkup(
    <AppProvider value={context}>
      <Onboarding />
    </AppProvider>,
  );
}

test("connecting an existing project starts with the client and keeps the selected project", () => {
  const html = render("ready");
  assert.match(html, /Connect an agent/);
  assert.match(html, /Which agent client do you use/);
  assert.match(html, /Step 1 of 3/);
  assert.match(html, /Selected project/);
  assert.doesNotMatch(
    html,
    /Create workspace|Create project|Different project/,
  );
});

test("first-run onboarding still follows the server state", () => {
  assert.match(render("needs_workspace"), /Create workspace/);
  const html = render("needs_project");
  assert.match(html, /Create project/);
  assert.doesNotMatch(html, /Create workspace/);
  assert.match(render("ready", ""), /Create project/);
});

test("a blocked caller gets an access explanation without a creation form", () => {
  const html = render("blocked");
  assert.match(html, /Your account cannot use Synveda yet/);
  assert.doesNotMatch(html, /<form|Create workspace|Which agent client|Refresh access/);
  // No refresh control is offered, so none is mentioned.
  assert.doesNotMatch(html, /refresh it here/);
});

test("a caller without shared access is told how to get it, not sent into creation", () => {
  // Signed in through the identity provider, but holding no shared grant and
  // no creation authority: the next step is an invitation or an administrator.
  const html = render("access_required");
  assert.match(html, /You need access to a workspace or project/);
  assert.match(html, /invitation link/);
  assert.match(html, /Refresh access/);
  assert.match(html, /refresh it here/);
  assert.match(html, /Signed in as <strong>Taylor<\/strong> \(test-user\)/);
  assert.doesNotMatch(html, /<form|Create workspace|Create project|Which agent client/);
});

test("project creation targets the eligible workspace, not the selected one", () => {
  const onboarding = {
    state: "needs_project" as const,
    workspace_count: 2,
    project_count: 0,
    workspace_create: false,
    project_create_workspace_ids: ["eligible"],
  };
  assert.equal(projectTarget(onboarding, { workspaceId: "viewed", projectId: null }), "eligible");
  assert.equal(projectTarget(onboarding, { workspaceId: "eligible", projectId: null }), "eligible");
  assert.equal(initialStep(onboarding, { workspaceId: "viewed", projectId: null }), "project");
  const none = { ...onboarding, state: "access_required" as const, project_create_workspace_ids: [] };
  assert.equal(projectTarget(none, { workspaceId: "viewed", projectId: null }), null);
  assert.notEqual(initialStep(none, { workspaceId: "viewed", projectId: null }), "project");
  assert.notEqual(initialStep(none, { workspaceId: "viewed", projectId: null }), "workspace");
});

test("onboarding offers manual native clients and separates access from delivery", () => {
  const html = render("ready");
  assert.match(html, /Codex CLI/);
  assert.match(html, /GitHub Copilot CLI/);
  assert.match(html, /Manual configuration available/);
  assert.match(html, /Explore the console; connect a client later/);
  assert.match(html, /Client setup completed: not confirmed/);
  assert.match(html, /Client operation observed: not checked/);
  assert.match(html, /selected; check below to verify current access/);
  assert.doesNotMatch(html, /agent connected|client connected/i);
});

import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { Onboarding } from "./Onboarding.js";
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
    },
    capabilities: { actions: {}, role_keys: [] },
    anchors: [],
    workspaces:
      state === "needs_workspace" || state === "blocked"
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
  assert.match(html, /Ask your administrator for workspace access/);
  assert.doesNotMatch(html, /<form|Create workspace|Which agent client/);
});

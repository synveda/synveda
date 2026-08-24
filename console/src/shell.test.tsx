/**
 * What the shell actually renders (CPR-8, ADR-0075 decision 1).
 *
 * `renderToStaticMarkup` plus `toText`, the convention CNSL-1 established
 * (ADR-0056 decision 7): a browser is where markup is laid out, but *which
 * facts appear in it* is decided by this code and by nothing the DOM
 * contributes.
 *
 * The assertions are about the two claims this feature makes to a reader:
 * that the product's shape is the same for everybody, and that the
 * governance surface appears only for somebody who may read it.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { NotFound, NotOffered, PageHeading, Shell, appContext } from "./Shell.js";
import { Planned } from "./Planned.js";
import { reconcile } from "./selection.mjs";
import { toText } from "./text.mjs";
import type { MeView, ProjectView, WorkspaceView } from "./generated/api.js";

function workspace(id: string, slug: string): WorkspaceView {
  return {
    id,
    slug,
    display_name: slug,
    status: "active",
    revision: 1,
    scope_id: `scope-${id}`,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

function project(id: string, workspaceId: string, slug: string): ProjectView {
  return {
    id,
    workspace_id: workspaceId,
    slug,
    display_name: slug,
    status: "active",
    revision: 1,
    scope_id: `scope-${id}`,
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
  };
}

function me(actions: Record<string, boolean>): MeView {
  return {
    principal: { subject: "robin@example.test", display_name: "Robin", quarantined: false },
    tenant: { id: "t", slug: "acme", name: "ACME", status: "active" },
    onboarding: { state: "ready", workspace_count: 1, project_count: 1 },
    workspaces: [workspace("w-1", "payments")],
    projects: [project("p-1", "w-1", "ledger")],
    capabilities: { actions, role_keys: ["member"] },
    anchors: [],
  };
}

function shell(actions: Record<string, boolean>): string {
  const view = me(actions);
  const selection = reconcile({ workspaceId: null, projectId: null }, view);
  const context = appContext(view, selection, () => {});
  return toText(
    renderToStaticMarkup(
      <Shell route="home" context={context}>
        <p>page body</p>
      </Shell>,
    ),
  );
}

test("everybody gets the same product navigation, whatever they hold", () => {
  // The rule the whole nav model turns on: a menu that appeared and
  // disappeared with a role would teach every reader a different shape for
  // the same application.
  const cases: Record<string, boolean>[] = [
    {},
    { "audit.read": true, "scope.read": true, "proposal.read": true },
  ];
  for (const actions of cases) {
    const rendered = shell(actions);
    for (const label of [
      "Home",
      "Sessions",
      "Knowledge",
      "New Learnings",
      "Skills",
      "Tools",
      "People",
      "Settings",
    ]) {
      assert.ok(rendered.includes(label), `${label} is missing:\n\n${rendered}`);
    }
  }
});

test("a caller with no governance capability is shown no Advanced section at all", () => {
  const rendered = shell({});
  assert.ok(!rendered.includes("Advanced"), `an empty Advanced heading was rendered:\n\n${rendered}`);
  for (const label of ["Reviews", "Scopes", "Policies", "Audit", "Service identities"]) {
    assert.ok(!rendered.includes(label), `${label} was offered to a caller with nothing`);
  }
});

test("the Advanced section carries exactly the planes the forecast offers", () => {
  const rendered = shell({ "proposal.read": true, "policy.read": true });
  assert.ok(rendered.includes("Advanced"));
  assert.ok(rendered.includes("Reviews"));
  assert.ok(rendered.includes("Policies"));
  assert.ok(!rendered.includes("Audit"), `Audit was offered without audit.read:\n\n${rendered}`);
  assert.ok(!rendered.includes("Service identities"));
});

test("the shell names the tenant, the caller, and both switchers", () => {
  const rendered = shell({});
  assert.ok(rendered.includes("ACME"));
  assert.ok(rendered.includes("Robin"));
  assert.ok(rendered.includes("Workspace"));
  assert.ok(rendered.includes("Project"));
  assert.ok(rendered.includes("payments"));
  assert.ok(rendered.includes("ledger"));
  assert.ok(rendered.includes("Sign out"));
  assert.ok(rendered.includes("page body"), "the outlet renders its child");
});

test("a workspace with no project says so rather than hiding the switcher", () => {
  const view = me({});
  view.projects = [];
  const selection = reconcile({ workspaceId: null, projectId: null }, view);
  const rendered = toText(
    renderToStaticMarkup(
      <Shell route="home" context={appContext(view, selection, () => {})}>
        <p />
      </Shell>,
    ),
  );
  // A switcher that vanished would read as a broken header; "no project
  // yet" is a fact somebody needs.
  assert.ok(rendered.includes("no project yet"), rendered);
});

test("a guarded page reached anyway explains the role rather than redirecting", () => {
  // A redirect to Home tells somebody their link was wrong when in fact
  // their role was.
  const rendered = toText(renderToStaticMarkup(<NotOffered route="audit" />));
  assert.ok(rendered.includes("audit.read"), rendered);
  assert.ok(rendered.includes("policy decision point"), rendered);
  assert.ok(rendered.includes("signing in again will not change the answer".toLowerCase()) ||
    rendered.includes("signing in again will not change the answer"), rendered);
});

test("an address this console does not have is a page, not a silent Home", () => {
  const rendered = toText(renderToStaticMarkup(<NotFound />));
  assert.ok(rendered.includes("No such page"), rendered);
});

test("a page heading comes from the route table", () => {
  const rendered = toText(renderToStaticMarkup(<PageHeading route="people" />));
  assert.ok(rendered.includes("People"));
  assert.ok(rendered.includes("Who may act here"));
});

test("the remaining plane that is not built says so, and shows no empty list", () => {
  // The failure this prevents: an empty list is indistinguishable from a
  // plane that works and has nothing in it, which is precisely the wrong
  // thing to tell somebody whose agent has been running all week.
  const rendered = toText(renderToStaticMarkup(<Planned route="tools" />));
  assert.ok(rendered.includes("not built yet"), rendered);
  assert.ok(rendered.includes("waiting on"), "Tools does not say what it is waiting on");
  assert.ok(!/\b0 (sessions|items|rows)\b/.test(rendered), `Tools fabricated an empty count:\n\n${rendered}`);
});

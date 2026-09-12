/**
 * The People page's judgements, and what a member row says (CPR-8,
 * ADR-0075 decision 5).
 *
 * Two halves. The derivations — who is project-only, where access came
 * from, whether a control is offered — are pure and tested directly. The
 * rendering is asserted through `renderToStaticMarkup` + `toText`, because
 * the claim this page makes is that a reader can answer "why can Robin see
 * this?" **from the row**, and that is a claim about the text.
 */

import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import { describe } from "./client.mjs";
import { cache } from "./cache.mjs";
import {
  ROLE_KEYS,
  accessLabels,
  accessSource,
  directMembers,
  hasLapsed,
  inheritedMembers,
  mayManageAccessAt,
  mayRemove,
  memberKey,
  pendingInvites,
  settledInvites,
  viaGroup,
  whenOf,
} from "./people.mjs";
import { People } from "./People.js";
import { reconcile } from "./selection.mjs";
import { AppProvider, appContext } from "./Shell.js";
import { toText } from "./text.mjs";
import type {
  GroupView,
  InviteView,
  MeView,
  MemberView,
  ProjectView,
  ScopeView,
  WorkspaceView,
} from "./generated/api.js";

function member(overrides: Partial<MemberView> = {}): MemberView {
  return {
    principal_id: "robin@example.test",
    role: "member",
    scope_id: "scope-project",
    source: "direct",
    inherited: false,
    directory_managed: false,
    grant_id: "g-1",
    granted_at: "2026-08-21T09:30:00Z",
    ...overrides,
  };
}

function invite(overrides: Partial<InviteView> = {}): InviteView {
  return {
    id: "i-1",
    role: "member",
    scope_id: "scope-workspace",
    status: "pending",
    created_at: "2026-08-20T09:00:00Z",
    expires_at: "2026-08-27T09:00:00Z",
    ...overrides,
  };
}

beforeEach(() => cache.clear());

test("project-only is derived from the row's own inherited flag", () => {
  // Not by diffing the workspace list against the project list: the API
  // answers it per row, and a diff would disagree with it the first time
  // somebody holds two roles.
  const rows = [
    member({ grant_id: "g-1", inherited: false }),
    member({ grant_id: "g-2", inherited: true, scope_id: "scope-workspace" }),
    member({ grant_id: "g-3", inherited: false, role: "viewer" }),
  ];
  assert.deepEqual(directMembers(rows).map((row) => row.grant_id), ["g-1", "g-3"]);
  assert.deepEqual(inheritedMembers(rows).map((row) => row.grant_id), ["g-2"]);
});

test("a row key distinguishes roles and principals resolved through one group grant", () => {
  const two = [member({ grant_id: "g-1", role: "member" }), member({ grant_id: "g-2", role: "curator" })];
  assert.notEqual(memberKey(two[0] as MemberView), memberKey(two[1] as MemberView));
  assert.notEqual(
    memberKey(member({ grant_id: "group-grant", principal_id: "subject-riley" })),
    memberKey(member({ grant_id: "group-grant", principal_id: "subject-vera" })),
  );
});

test("access source names the mechanism, because that is what you have to change", () => {
  assert.equal(accessSource(member({ source: "owner" })), "granted here, as its creator");
  assert.equal(
    accessSource(member({ source: "invite", inherited: true })),
    "inherited from a scope above, by redeeming an invitation",
  );
  assert.equal(
    accessSource(member({ via_group: { id: "grp-1", slug: "engineering" } })),
    "granted here, through the engineering group",
  );
  assert.equal(
    accessSource(member({ directory_managed: true, source: "directory" })),
    "granted here, managed by your directory",
  );
  // Both clauses, when both are true. "Managed by your directory" says you
  // cannot change it here; the group says what to change instead, and
  // dropping the second because the first is true drops the actionable half.
  assert.equal(
    accessSource(
      member({ directory_managed: true, source: "directory", via_group: { id: "g", slug: "eng" } }),
    ),
    "granted here, through the eng group, managed by your directory",
  );
});

test("an absent generated group reference is normalised", () => {
  assert.equal(viaGroup(member()), null);
  assert.equal(viaGroup(member({ via_group: null })), null);
  assert.deepEqual(viaGroup(member({ via_group: { id: "g", slug: "eng" } })), {
    id: "g",
    slug: "eng",
  });
});

test("remove is offered only where the API would accept it", () => {
  // Absent rather than disabled (ADR-0056): a disabled button is a promise
  // that trying harder would enable it, and it would not.
  assert.equal(mayRemove(member()), true);
  assert.equal(mayRemove(member({ inherited: true })), false, "an inherited grant is written above");
  assert.equal(
    mayRemove(member({ directory_managed: true })),
    false,
    "a directory would put it straight back",
  );
  assert.equal(
    mayRemove(member({ via_group: { id: "g", slug: "engineering" } })),
    false,
    "a member row must not make one group grant look like one-person access",
  );
});

test("governed scope and group reads add names while exact ids remain the authority", () => {
  const me = {
    principal: { subject: "subject-avery", display_name: "Avery Author" },
    workspaces: [
      { scope_id: "scope-workspace", display_name: "Demo workspace", slug: "demo" },
    ],
    projects: [{ scope_id: "scope-project", display_name: "Retry review", slug: "retry" }],
    anchors: [
      {
        scope_id: "scope-project",
        actions: { "membership.grant": true },
      },
    ],
  } as unknown as MeView;
  const rileyScope = {
    id: "scope-riley",
    kind: "principal",
    principal_id: "subject-riley",
    display_name: "Riley Reviewer",
  } as ScopeView;
  const group = {
    id: "group-reviewers",
    display_name: "Release reviewers",
  } as GroupView;
  const labels = accessLabels(me, { scopes: [rileyScope] }, [group]);

  assert.equal(labels.principals.get("subject-avery"), "Avery Author");
  assert.equal(labels.principals.get("subject-riley"), "Riley Reviewer");
  assert.equal(labels.scopes.get("scope-workspace"), "Workspace · Demo workspace");
  assert.equal(labels.scopes.get("scope-project"), "Project · Retry review");
  assert.equal(labels.groups.get("group-reviewers"), "Release reviewers");
  assert.equal(mayManageAccessAt(me, "scope-project"), true);
  assert.equal(mayManageAccessAt(me, "scope-workspace"), false);
});

test("the access UI revokes one exact grant through the generated application API", () => {
  const call = describe("revoke_grant", { path: { grant_id: "grant-reviewer" } });
  assert.equal(call.path, "/admin/grants/grant-reviewer");
  assert.equal(call.init.method, "DELETE");
  assert.equal(call.init.body, undefined);
});

test("invitations split into what is actionable and what is history", () => {
  const all = [
    invite({ id: "i-1", status: "pending" }),
    invite({ id: "i-2", status: "accepted", accepted_at: "2026-08-21T10:00:00Z" }),
    invite({ id: "i-3", status: "revoked" }),
    invite({ id: "i-4", status: "expired" }),
  ];
  assert.deepEqual(pendingInvites(all).map((i) => i.id), ["i-1"]);
  assert.deepEqual(settledInvites(all).map((i) => i.id), ["i-2", "i-3", "i-4"]);
});

test("the console notices a lapsed clock without overruling the server's status", () => {
  const past = invite({ expires_at: "2026-08-01T00:00:00Z" });
  const future = invite({ expires_at: "2099-01-01T00:00:00Z" });
  const now = Date.parse("2026-08-21T00:00:00Z");
  assert.equal(hasLapsed(past, now), true);
  assert.equal(hasLapsed(future, now), false);
  // Still `pending`: the server owns the status, and the console only
  // points out that the clock disagrees.
  assert.equal(past.status, "pending");
});

test("timestamps render the way every other surface here renders them", () => {
  assert.equal(whenOf("2026-08-21T09:30:00Z"), "2026-08-21 09:30 UTC");
  assert.equal(whenOf("not a date"), "not a date", "an unparseable value is shown, not hidden");
});

test("the six role keys are the vocabulary, in picker order", () => {
  assert.deepEqual(
    [...ROLE_KEYS],
    ["owner", "administrator", "curator", "reviewer", "member", "viewer"],
  );
});

test("a rendered row answers why, without an audit log", () => {
  // The feature, asserted on the text. A member list that shows a name and
  // a role makes somebody go and read the chain; this one does not.
  const rows = [
    member({
      grant_id: "g-1",
      principal_id: "robin@example.test",
      role: "curator",
      via_group: { id: "grp", slug: "engineering" },
      inherited: true,
      directory_managed: true,
    }),
  ];
  const rendered = toText(
    renderToStaticMarkup(
      <ul>
        {rows.map((row) => (
          <li key={memberKey(row)}>
            {row.principal_id} {row.role} {accessSource(row)}{" "}
            {row.directory_managed ? "directory" : "direct"} since {whenOf(row.granted_at)}
          </li>
        ))}
      </ul>,
    ),
  );
  assert.ok(rendered.includes("robin@example.test"));
  assert.ok(rendered.includes("curator"));
  assert.ok(rendered.includes("inherited from a scope above"));
  assert.ok(rendered.includes("engineering group"));
  assert.ok(rendered.includes("directory"));
  assert.ok(rendered.includes("2026-08-21 09:30 UTC"));
});

test("the People surface joins disclosed names to exact scoped access and real invite state", async () => {
  const workspace = {
    id: "workspace-demo",
    scope_id: "scope-workspace-full",
    slug: "demo",
    display_name: "Demo workspace",
    status: "active",
  } as WorkspaceView;
  const project = {
    id: "project-retry",
    workspace_id: workspace.id,
    scope_id: "scope-project-full",
    slug: "retry-review",
    display_name: "Retry review",
    status: "active",
  } as ProjectView;
  const me = {
    principal: { subject: "subject-avery", display_name: "Avery Author", quarantined: false },
    tenant: { id: "tenant-demo", slug: "demo", name: "Demo tenant", status: "active" },
    onboarding: { state: "ready", workspace_count: 1, project_count: 1 },
    capabilities: { actions: {}, role_keys: ["administrator"] },
    anchors: [
      {
        scope_id: workspace.scope_id,
        kind: "workspace",
        source: "grant",
        direct: true,
        roles: ["owner"],
        actions: { "membership.grant": true },
      },
      {
        scope_id: project.scope_id,
        kind: "project",
        source: "selected_project",
        direct: false,
        roles: ["owner"],
        actions: { "membership.grant": true },
      },
    ],
    workspaces: [workspace],
    projects: [project],
  } as MeView;
  const riley = member({
    principal_id: "subject-riley",
    scope_id: workspace.scope_id,
    role: "reviewer",
    grant_id: "grant-riley-reviewer",
  });
  const group = {
    id: "group-release-reviewers",
    slug: "release-reviewers",
    display_name: "Release reviewers",
    source: "direct",
    status: "active",
    revision: 2,
    members: [{ identity_id: "identity-riley", principal_id: "subject-riley" }],
    created_at: "2026-08-21T09:00:00Z",
    updated_at: "2026-08-21T09:00:00Z",
  } as GroupView;
  const ok = (body: unknown) => ({ kind: "ok" as const, body });
  await Promise.all([
    cache.ensure("access/reference/scopes", async () =>
      ok({
        scopes: [
          {
            id: "scope-riley",
            kind: "principal",
            principal_id: "subject-riley",
            display_name: "Riley Reviewer",
            slug: "riley",
          },
        ],
      }),
    ),
    cache.ensure("access/reference/groups", async () => ok({ groups: [group] })),
    cache.ensure(`workspaces/${workspace.id}/members`, async () => ok({ members: [riley] })),
    cache.ensure(`projects/${project.id}/members`, async () =>
      ok({ members: [{ ...riley, inherited: true }] }),
    ),
    cache.ensure(`access/grants/${workspace.scope_id}`, async () =>
      ok({
        grants: [
          {
            id: riley.grant_id,
            scope_id: workspace.scope_id,
            subject_kind: "principal",
            principal_id: riley.principal_id,
            role: riley.role,
            source: "direct",
            directory_managed: false,
            created_at: riley.granted_at,
          },
        ],
      }),
    ),
    cache.ensure(`access/grants/${project.scope_id}`, async () => ok({ grants: [] })),
    cache.ensure(`workspaces/${workspace.id}/invites`, async () =>
      ok({
        invites: [
          invite({ id: "invite-pending", scope_id: workspace.scope_id, email: "vera@example.test" }),
          invite({
            id: "invite-accepted",
            scope_id: workspace.scope_id,
            status: "accepted",
            accepted_at: "2026-08-22T09:00:00Z",
          }),
        ],
      }),
    ),
  ]);

  const selection = reconcile({ workspaceId: workspace.id, projectId: project.id }, me);
  const markup = renderToStaticMarkup(
    <AppProvider value={appContext(me, selection, () => {})}>
      <People />
    </AppProvider>,
  );
  const text = toText(markup);
  for (const expected of [
    "Riley Reviewer",
    "subject-riley",
    "scope-workspace-full",
    "Exact access grants",
    "grant-riley-reviewer",
    "Release reviewers",
    "pending member vera@example.test",
    "accepted",
    "Create pending invitation",
    "Keycloak owns sign-in profiles",
  ]) {
    assert.match(text, new RegExp(expected, "i"), expected);
  }
  assert.doesNotMatch(text, /Groups and tenant-wide grants live under Advanced/);
});

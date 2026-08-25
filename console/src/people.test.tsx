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
import { test } from "node:test";

import { renderToStaticMarkup } from "react-dom/server";

import {
  ROLE_KEYS,
  accessSource,
  directMembers,
  hasLapsed,
  inheritedMembers,
  mayRemove,
  memberKey,
  pendingInvites,
  settledInvites,
  viaGroup,
  whenOf,
} from "./people.mjs";
import { toText } from "./text.mjs";
import type { InviteView, MemberView } from "./generated/api.js";

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

test("project-only is derived from the row's own inherited flag", () => {
  // Not by diffing the workspace list against the project list: the API
  // answers it per row, and a diff would disagree with it the first time
  // somebody holds two roles.
  const rows = [
    member({ grant_id: "g-1", inherited: false }),
    member({ grant_id: "g-2", inherited: true, scope_id: "scope-workspace" }),
    member({ grant_id: "g-3", inherited: false, role: "viewer" }),
  ];
  assert.deepEqual(directMembers(rows).map(memberKey), ["g-1", "g-3"]);
  assert.deepEqual(inheritedMembers(rows).map(memberKey), ["g-2"]);
});

test("a row's key is its grant, not its principal", () => {
  // One entry per (principal, role): somebody holding two roles appears
  // twice, because the two came from different grants and are revoked
  // separately.
  const two = [member({ grant_id: "g-1", role: "member" }), member({ grant_id: "g-2", role: "curator" })];
  assert.notEqual(memberKey(two[0] as MemberView), memberKey(two[1] as MemberView));
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

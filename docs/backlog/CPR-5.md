---
title: "CPR-5: Membership, groups, grants & invitations"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-5: Membership, groups, grants & invitations

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

> **Successor note (CPR-34, ADR-0093):** the original `directory_ref` and
> token-subject membership shape below records what CPR-5 shipped. Directory
> groups now carry source + stable resource id + optional protocol
> `externalId`, and `group_members` keys stable identities so membership may
> arrive before first login. Direct groups use that identity-keyed shape too.

## Description

Prompt 5 of the 33-prompt context-platform programme. CPR-4 gave the platform
workspaces and projects and left one thing conspicuously absent: **nobody is in
them**. A workspace created through `POST /v1/workspaces` had a name, a
description, a governed scope and no members, and the only way anybody could act
on it was a `role_bindings` row bound at a node of the *old* hierarchy — a
different tree that Prompt 6 deletes whole.

Four pieces:

- **`groups`** — a named set of principals: `slug`, `display_name`,
  `description`, `source` (`direct` | `directory`), `directory_ref`, `status`,
  `revision`. Grants nothing on its own.
- **`group_members`** — a principal's membership of one, with its own source.
- **`scope_grants`** — one subject's one **role key** at one scope: the subject
  is a principal or a group, the source is `owner` | `direct` | `invite` |
  `directory` | `automation`, and the scope's subtree inherits it.
- **`pending_invites`** — an expiring, one-time, revocable token that mints a
  grant when somebody redeems it. The token is stored only as a SHA-256.

Fourteen routes, four Cedar actions, seven audit action types, and the
`owner` grant that creating a workspace or a project now mints for its creator.

Decisions in ADR-0072.

## Why this exists

The product's target user is one person, or four people sharing agent context.
For that person, "who else can see this" is not an enterprise feature — it is
the second thing they do, right after making the workspace. And the only
mechanism the tree had was a role binding on an organisational hierarchy that
this programme is deleting.

The harder half is what the feature refuses to build. Every product with roles
eventually grows a `role_permissions` table: inspectable, editable, and a
**second decision point** that disagrees with the policy engine the first time
somebody edits one. Seed §2.2 permits exactly one decision point, and this
product sells the property that the one is auditable. So the schema stores a
role *key* and nothing about what it permits.

## Design

### A grant is a subject, a role key and a scope

```
scope_grants
  scope_id      → the scope. Its subtree inherits.
  subject_kind  → principal | group   (exactly one column populated)
  role_key      → owner | member | viewer | reviewer | curator | administrator
  source        → owner | direct | invite | directory | automation
  invite_id     → set exactly when source = 'invite'
```

Additive and inherited. There is **no deny row** and there must not be one: a
denial that lives in a membership table is a second policy engine.

### A role key is a key

Six words, and nothing in the schema or in `synveda-types` says what any of them
may do. The Cedar packs decide that. `RoleKey::ALL` is the vocabulary a CHECK
constraint mirrors, and a unit test asserts the six are exactly the six —
because a pack written against a vocabulary that grew silently would stop being
exhaustive.

The cost is stated in ADR-0072 decision 3 and repeated below: **until the PDP is
re-cut over generic scopes, a role key decides nothing.**

### Inheritance is the scope tree, not a fan-out

`members_of` walks `scope_closure` upward. A grant at a workspace's scope is in
force at every project inside it, resolved at read time, with **no per-project
row**. Nothing has to be repaired when a scope moves, and there is no window in
which a copy and its source disagree.

Three rules, all in one query so no caller can apply two of them:

| rule | where |
|---|---|
| inheritance up the ancestry | the `scope_closure` join |
| **a `principal` scope inherits nothing** | the `chain` CTE's kind test, mirroring `access::inherits_into` |
| a group resolves to its members; an archived one to nobody | the `groups`/`group_members` joins |

The second is the one worth reading twice. A `principal`-shaped scope is
somebody's own: no ancestor reaches into it, not the tenant root and not a
workspace owner. A grant written *at* it still applies — isolation is about
inheritance, not about the scope being unreachable.

### A principal is a token subject

`principal_id` is `text`, not a foreign key into `identities`. ADR-0015
decision 2's reasoning (a grant may precede first login) plus one sharper: an
`identities` row in this tree still requires a `hierarchy_nodes` node, so a
membership model keyed on it would need the model it replaces. That would be a
synchronisation between the two models, which this programme forbids outright.

What it costs: the member list carries subjects rather than names. Display names
belong to the identity plane, which a later prompt re-cuts.

### Where each rule lives

ADR-0070 decision 2's doctrine again:

| rule | enforced by |
|---|---|
| a grant has exactly one subject | `scope_grants_principal_shape_check` + `..._group_shape_check` |
| one row per (scope, subject, role) | `scope_grants_unique` (`nulls not distinct`) |
| an `invite` grant names its invitation, and no other may | `scope_grants_invite_shape_check` |
| **a grant is never edited** | `synveda_grants_are_immutable` trigger + no `UPDATE` grant |
| an invitation is one-time | `synveda_invites_are_terminal` trigger |
| an invitation always expires | `pending_invites_expiry_check` |
| a directory group carries its reference, a direct one does not | `groups_directory_ref_check` |
| a group's slug, source and provenance are immutable, and its revision steps forward by one | `synveda_groups_immutable_columns` trigger |

The one rule that is **not** a database fact is "a directory-managed row cannot
be edited here": the directory adapter and a person hold the same database role,
so no constraint can tell them apart. It lives in `synveda_store::access` with a
message naming the directory, and a test.

### Invitations

`synveda_invite_v1.<tenant-uuid>.<43-char base64url secret>` — the provisioning
credential's shape (ADR-0059 decision 13), for its reasons: 256 bits of entropy,
the **whole string** hashed so a secret lifted behind another tenant's prefix
hashes to nothing, and a versioned greppable prefix so one leaked into a chat
window can be found.

The plaintext exists once, in the creation response, beside a copyable
`accept_url`. Email delivery is deliberately not a requirement.

Three consequences:

- **The token is in a URL path**, so `make_request_span` records the matched
  route *pattern* rather than the URI for that one route, from an explicit list.
- **A replayed creation is a 409**, not a 200: the original response carried a
  token that no longer exists, and a 200 with it missing would look successful
  and be unusable.
- **Redeeming takes no `Idempotency-Key`** — the token is one-time, so it *is*
  the key. A retry by the principal who redeemed it replays; anybody else is a
  409.

### The four actions

`MembershipRead`, `MembershipGrant`, `GroupManage`, `InviteAccept`.

Read is one authority over the whole plane (`DirectoryManage`'s argument), and
is **not** `WorkspaceRead`: a workspace's name discloses nothing, its membership
discloses who works on what. The packs grade it — `regulated-strict` keeps it to
the admin roles, `standard` adds `curator`, `open-collaboration` adds every
content role.

`InviteAccept` is permitted to any principal the base layer has not already
forbidden, under every pack. The token is the authority; a person this product
invited must not be turned away for want of a role. It is an *action* rather
than an exemption so the invariant floor still runs over it, and so a deployment
that wants the mechanism off says so in a pack.

## What this prompt deliberately does not do

- **It does not make grants a PDP input.** The resolution is implemented and
  served; the role keys do not reach Cedar. Feeding a grant written on a generic
  scope into a decision taken at the *tenant* would be a translation between the
  two models and would widen every tenant-wide decision by every project-level
  grant. **This is the largest thing the feature defers** (ADR-0072 decision 3).
- **It does not re-anchor the PDP**, inheriting ADR-0071 decision 3's tenant
  anchoring for its reason.
- **It does not mint `principal` scopes.** The isolation rule is implemented and
  tested against scopes a test creates; minting one per identity is the identity
  plane's re-cut.
- **It ships no directory adapter.** `source = 'directory'` and its refusals
  exist so a person's group and a directory's are the same row shape when
  Prompt 29 lands; until then that path is exercised only by tests.
- **No console screen and no CLI command.** Prompt 20 and Prompt 24.
- **`GET /v1/admin/grants` does not page.** It filters and returns an envelope
  with room for a cursor.

## Acceptance criteria

- Creating a workspace or a project mints an `owner` grant for its creator in
  the creating transaction, with the source no route hands out, and chains it.
- A grant at a workspace is in force at every project inside it and **writes no
  row there**; a project-only grant reaches neither its workspace nor a sibling;
  the resolution orders the nearest scope first and every entry names the scope
  its grant is actually at.
- **A `principal`-shaped scope inherits nothing**, asserted against a grant at
  the tenant root — the widest thing the model can say — while a grant written
  at that scope still applies.
- A grant to a group resolves to its members and follows them as the group
  changes, with no grant written; an archived group and an empty one resolve to
  nobody; a membership replacement is the whole list and a duplicate is one
  membership.
- The structural rules hold against direct SQL: a grant has exactly one subject,
  is never edited, and only an `invite`-sourced one names an invitation; a
  group's slug, source and provenance are immutable and its revision cannot be
  rewound or skipped; a terminal invitation cannot be reopened and its terms
  cannot be re-pointed.
- A stale `expected_revision` is a 409 that writes nothing, membership included;
  an empty update is refused.
- An invitation is one-time — a retry by the same principal replays with 200, a
  second person is a 409 — **expires without anything running**, is refused after
  either terminal state, and cannot outlive the product ceiling.
- The token is stored only as a 32-byte hash, appears in exactly one response,
  and is **absent from the audit chain** — swept for rather than argued; a
  replayed creation is a 409 saying it cannot be re-served; a string that is not
  one of this product's tokens is refused by shape without the refusal echoing
  it; and an unknown token is indistinguishable from another tenant's.
- Redeeming needs the token rather than a role under every pack, while the
  invariant floor still refuses a quarantined principal and every service
  identity.
- Removing a member touches only what was written at that scope, and refuses
  inherited, group-derived and directory-managed authority with the place to go.
- Every route denies without its action, refuses without a credential and chains
  its event; a replay still takes the PDP decision; another tenant's group,
  grant and invitation are 404s rather than 403s.
- All four tables join the adversarial RLS suite's completeness inventory, with
  a wrong-GUC read seeing nothing, a cross-tenant grant rejected, the lifecycle
  working as `synveda_app`, and no DELETE on `groups` or UPDATE on
  `scope_grants`.
- The OpenAPI document grows to twenty-six operations across seventeen paths,
  every one mounted, with `console/src/generated/api.ts` regenerated from it and
  both checks in `make ci`.
- Demonstrated by `crates/synveda-store/tests/access.rs`,
  `crates/synveda-gateway/tests/access_api.rs`,
  `crates/synveda-policy/tests/access.rs`, the CPR-5 block of
  `crates/synveda-store/tests/rls.rs`, `crates/synveda-gateway/tests/openapi.rs`,
  and `demos/cpr-5-access.sh`.

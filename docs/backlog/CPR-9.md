---
title: "CPR-9: The foundation audit — hardening the scope and access cutover"
labels:
  - epic:CPR
  - phase:5
size: M
---

# CPR-9: The foundation audit — hardening the scope and access cutover

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** M

## Description

Prompt 9 of the 33-prompt context-platform programme, and the first one asked
to **audit** its predecessors rather than build on them.

Prompts 1–8 each shipped green. Each suite proved that its own plane worked:
the epoch guard refuses a pre-cut database, the scope substrate holds its
constraints, workspaces and projects create transactionally, grants inherit
down a subtree, the PDP decides at anchors, the console renders a shell. What
no suite had asked was the question that spans all of them — **what does a
caller learn, or fail to learn, that their grants do not say?** — and it is the
question a cutover is most exposed to, because a hard cut moves a boundary and
leaves every caller of the old one behind.

## What it adds

`crates/synveda-gateway/tests/foundation_audit.rs`, an adversarial suite with
three adversaries and three leak channels.

**The adversaries.**

1. **Another tenant, with valid identifiers.** A second admitted tenant, whose
   own administrator mints a real workspace, project, scope, group, grant and
   invitation. Every per-object route on the context-platform plane is then
   probed with those ids and the first tenant's bearer, and the assertion is
   not "not 200" but **indistinguishable from an id nobody ever minted** —
   same status, same error kind. A caller who can tell a foreign id from a
   fictional one can enumerate another tenant's inventory one uuid at a time
   without ever reading a row.
2. **Another workspace, inside one tenant.** The case tenancy cannot help
   with: both rows are the same tenant's, so RLS is silent and every refusal is
   the PDP's and the anchor resolver's.
3. **Somebody else's own scope**, probed by the tenant administrator — the one
   caller who reaches everything else, so a test that used an outsider would
   pass under any rule that merely required a role.

**The channels.** Counts (listing lengths, the onboarding tallies the console
routes people by), errors (a message that names what the caller may not see)
and **navigation capabilities** (`/v1/me`'s anchors and the capability probe,
which the console builds its menu from).

## What it found

Three defects, all cutover residue rather than design faults.

### 1. A grant at a workspace did not reach the listings

`GET /v1/workspaces` and `GET /v1/me` took **one** decision, at the tenant
root, and applied its verdict to every row. For an administrator — who holds a
grant at the root — that is the right answer by accident. For a member it was
wrong in the direction that matters: a caller granted `member` at one workspace
holds nothing at the root, so the single decision denied and the listing came
back **empty**, with `workspace_count: 0` and `onboarding.state:
needs_workspace`.

The same response's `anchors` block said `workspace.read: true` at that
workspace. Two answers to one question, from one payload — and the console
renders both, so an invited member was sent to the first-run wizard to create
the workspace they had just been added to.

The fix: listings decide **per row, against the row**, which is the decision
`GET /v1/workspaces/{id}` already took. One gather, one materialised entity
batch, one Cedar evaluation per row under that row's own chain and pack
assignments — `crate::workspaces::decide_each`, shaped after
`capabilities::at_anchors` and for the same reason (the effective pack is a
property of the resource, ADR-0014 decision 3). There is deliberately **no
fast path** for a caller permitted at the root: "permitted above ⇒ permitted
below" is not a property Cedar has, because a forbid overrides a permit at any
depth and a stored pack may write one.

The route still refuses a caller who holds nothing at the root *and* nothing
below it, so the contract an outsider has always had is unchanged.

### 2. Two client/server contracts had drifted apart

Both on routes Prompt 19 has not yet put on the OpenAPI contract, so both
sides are hand-written and nothing checked they agreed.

- **`synveda login` could not parse a successful login.** CPR-7 deleted
  `identity.quarantined` from the gateway's session response — placement is
  identity now, so the field could only ever be `false` — and the CLI kept
  requiring it. serde has no default for a missing field, so every login
  failed at the last step: after the browser round trip, after the code
  exchange, with the credential already minted.
- **`synveda whoami --capabilities` could not parse any response.** CPR-7
  renamed `roles` to `role_keys` and deleted `role_assign` with the
  role-binding vocabulary; the CLI read the old shape. Plain `synveda whoami`
  shares the route and never asks for the block, which is why nothing noticed.

Both are fixed, and both are now **pinned from each side**: the CLI parses a
literal of what the gateway serves, and the gateway asserts the exact key set
its view serialises, each test naming the other's file. The server cannot drop
a field the CLI needs without one of them going red.

### 3. The no-data-migrator guard checked one file of forty-one

`no_old_to_new_data_migrator_exists` scanned the epoch migration — the file a
translator written *today* would live in — and left the rest of the chain
unchecked, which is where translations written *before* the cut already were.

It now scans the whole chain, skipping dollar-quoted function bodies so an
audit trigger's `insert into ..._history` is not mistaken for a translation,
and pins the three inherited pre-epoch upgrade statements by name.

They are **not deleted**, and the reason is written into the test. They are
unreachable: a pre-cut database never reaches the migrator (`epoch::preflight`
refuses it first), so the only databases that run migrations 8 and 38 are fresh
ones, where the tables those statements touch are empty at that point in the
chain. Deleting them changes a checksum, which would trade the guard's reset
instruction for `migration 8 was previously applied but has been modified` on
every existing epoch-2 database — an epoch bump and a reset for every
deployment, in exchange for removing statements that cannot run. Prompt 33's
squash removes them. The value of the pinned list is that a **fourth** DML
statement anywhere in the chain fails the build.

## Also verified, and found sound

- **Forced RLS is complete.** 52 tenant-bound tables, every one `ENABLE` +
  `FORCE` with at least one policy, checked against the live schema
  independently of the suite that asserts it. The four exempt tables are the
  documented structural ones. `hierarchy_nodes`, `hierarchy_closure`,
  `role_bindings` and `group_mappings` are absent.
- **The scope closure holds** over the ~23,000 scopes the suite produces: a
  distance-0 self row per scope, no cross-tenant pair, no cycle, a distance-1
  edge per parent pointer, one parentless `tenant`-shaped root per tenant, and
  `principal_id` present exactly on `principal`-shaped scopes.
- **The privacy forbid holds.** A tenant administrator is offered nothing at
  somebody else's `principal` scope.
- **The capability probe discloses nothing extra.** A scope the caller holds
  nothing at answers every verdict false, with no `scope_path`, no `pack` and
  no roles.
- **An invitation is tenant-bound and survives a cross-tenant attempt** —
  refused, and still spendable by its rightful recipient, so a leaked link is
  not a denial-of-service against the person it was for.
- **Console capability gating fails closed.** `offersRoute` requires
  `actions[capability] === true`, so a missing key hides the route.

## Residue classified

Every match of the audit's search terms was classified. `RoleBinding`,
`serde(alias)`, `hierarchy rank` and `dual write` match no code at all — only
prose, including three ADRs that *refuse* a dual write by name. The three live
defects above were the only unjustified matches in production code. Stale prose naming
deleted concepts was corrected where a reader would act on it — `synveda init`
told operators to run `synveda hierarchy list` and claimed the first login
binds `org-admin`; `synveda whoami` pointed at `synveda hierarchy capabilities`;
`ScopeId`'s own doc comment still defined a scope as a rung of the
`org`/`division`/`department`/`team`/`user` ladder. The remaining matches are
prose *narrating* the deletion (justified, and the record the programme keeps),
negative tests asserting the 404s (the point), and test-local variable names
for org units.

The 43 Phase-3 demos that still seed through `role bind`, `hierarchy_closure`
or `/v1/hierarchy` are unchanged and were **counted again rather than
trusted**: the number CPR-7 recorded is exactly right. They stay for the
prompts that re-anchor their subsystems.

## Acceptance criteria

- A valid workspace, project, scope, group, grant or invitation id from
  another tenant is a 404 with the same error kind as a fictional one on every
  per-object route, and no listing, `/v1/me` field or onboarding tally names it.
- An invitation minted in one tenant cannot be redeemed from another and
  survives the attempt, still redeemable by its rightful recipient.
- A member of one workspace sees exactly that workspace and its project in
  `/v1/me` and in `GET /v1/workspaces`, the two agree, and both the other
  workspace and its project are refused.
- The capability probe answers a scope the caller holds nothing at with every
  verdict false and no node detail, and answers somebody else's `principal`
  scope the same way for a tenant administrator.
- A caller who holds nothing is answered with nothing rather than an error.
- `synveda login` and `synveda whoami --capabilities` parse what the gateway
  serves, pinned from both sides.
- Every tenant-bound table is enabled + forced with a policy; the four
  hierarchy tables are absent.
- The scope closure carries a self row per scope, no cross-tenant pair, no
  cycle, and a distance-1 edge per parent pointer.
- No migration in the chain runs DML outside a function body but the three
  pinned inherited statements, and a fourth fails the build.

## Definition of done

1. Acceptance criteria met and demonstrated — `foundation_audit.rs` (6),
   the widened `epoch.rs` guard, the two wire-shape pins.
2. Tests written — 6 new integration tests, 1 widened guard, 4 new unit tests.
3. Tracing spans + metrics on new paths — the filtered listings run inside the
   spans and counters their routes already carry (`workspace.list`,
   `project.list`, `me`); no new server path was added.
4. Audit events emitted — no new action type. The listings chain the same
   `authz.decision` they always did, now reporting the decision that actually
   admitted the rows and the `not_answered` count beside it.
5. docs/backlog/STATUS.md updated.

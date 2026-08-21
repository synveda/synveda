---
title: "CPR-8: The console product shell & first-run onboarding"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-8: The console product shell & first-run onboarding

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Prompt 8 of the 33-prompt context-platform programme. Six prompts built a
context platform reachable only from the CLI; this one makes it reachable from
a browser, and replaces the console's governance-first entry point with a
route-based product shell.

Deleted: `App.tsx`'s direct mounting of the proposals inbox and the scope
explorer, the `whoami` session call, and the no-navigation shell that put a
review queue in front of everybody who signed in.

What replaces them:

- **A route table with two menus.** Primary — Home, Sessions, Knowledge,
  New Learnings, Skills, Tools, People, Settings — shown to everybody.
  Advanced — Reviews, Scopes, Policies, Audit, Service identities — shown only
  where the caller's capability forecast offers the plane. Client-side
  routing written in-repo over the History API; no dependency added.
- **A workspace switcher and a project switcher**, over a selection persisted
  per browser and reconciled on every load against what `/v1/me` says the
  caller may read.
- **A typed client generated from the OpenAPI contract**, including the
  runtime path/method table and a compile-time `Idempotency-Key` obligation on
  the eight operations whose document requires one.
- **One query/cache layer**, and therefore one loading state and one error
  state for every route.
- **A People page** — workspace members, project-only members, pending and
  settled invitations, each row carrying its role, its access source, the
  scope its grant is written at, the group it came through and whether a
  directory owns it — with invite, revoke and remove offered exactly where the
  API would accept them.
- **First-run onboarding**: workspace, project, repository, agent client,
  connection instructions, connection check.
- The proposals inbox re-homed to **Advanced ▸ Reviews** and the scope
  explorer to **Advanced ▸ Scopes**, both unchanged in substance.

Decisions in ADR-0075.

## Why this exists

CPR-3 through CPR-7 built scopes, workspaces, projects, membership,
invitations and a re-cut decision point. A person who installs this product
and opens the console sees none of it: they see a proposals inbox with nothing
in it, a scope tree containing one scope, and no way to create a workspace —
which is the first thing the product now asks anybody to do.

The audience is the reason. Everything above Phase 5 was built for an
organisation whose console user is a reviewer or an administrator. Phase 5's
audience is one person, or four sharing agent context, and their first
question is not "what needs approving" — it is "how do I connect my agent to
this".

## Design

See ADR-0075 for the seven decisions: a route-based shell with an
unconditional primary menu and a capability-gated advanced one; the selection
as persisted preference plus reconciliation; contract-covered calls through
the generated client with the rest marked as Prompt 19's; one cache and
therefore one pair of route states; the People page's access-source
derivations; onboarding that seeds without branding; and an honest page for a
plane that has no API yet.

## Acceptance criteria

- The primary navigation carries all eight items **for every caller**,
  including one whose capability map is empty, and no primary route is gated.
- The advanced navigation appears only where the forecast offers a plane, is
  absent entirely (heading included) for a caller with none, and carries
  exactly the planes offered — asserted per item against the tenant-plane
  action names `proposal.read`, `scope.read`, `policy.read`, `audit.read`,
  `service_identity.read`.
- A guarded route reached directly renders an explanation naming the missing
  action rather than redirecting, and a path the console does not have renders
  a not-found page rather than silently landing on Home.
- The selection survives a reload, falls back when it names a workspace the
  caller can no longer read, drops a project that belongs to another
  workspace, and degrades to no-preference in a browser that refuses to store
  anything.
- Every call to a contract-covered route goes through the generated client;
  an operation the document marks idempotent cannot be sent without an
  `Idempotency-Key`, and one it does not mark cannot carry one.
- The People page distinguishes workspace members from project-only members
  by each row's own `inherited` flag, names the access source of every row
  including the group and the directory together, and offers remove only where
  the grant is direct and not directory-managed.
- Onboarding walks workspace → project → repository → client → instructions →
  check; the personal/team choice produces a seeding plan carrying **no**
  edition field of any kind; a refused seeding step is reported with the plane
  that can finish it and does not block the wizard; the connection check
  passes without a repository, fails on an unreadable project, and states what
  it cannot verify.
- A plane with no API says it is not built and what it is waiting on, and
  fabricates no empty count.
- The OpenAPI document and `console/src/generated/api.ts` agree
  (`make check-api-types`), and no npm dependency was added.

## Standing after this feature

**Four primary pages have no plane behind them.** Sessions, Knowledge, New
Learnings and Tools render prose. They are waiting on the session aggregate,
candidates, knowledge versions and the tool registry — Prompts 9–15 of this
programme — and the pages name what they are waiting on.

**Seven surfaces still call hand-written paths.** Proposals, capabilities,
policy packs, lapses, audit, skills and service identities are not on the
OpenAPI contract, so the console reaches them through `api.mts` rather than
the generated client. They are grouped and labelled there; Prompt 19 puts the
rest of `/v1` on the contract and deletes them.

**The console has no group management screen.** `POST /v1/admin/groups` and
`PATCH /v1/admin/groups/{id}` are on the contract and reachable from the CLI;
People manages grants and invitations, and directory-managed groups are shown
as the source of a row rather than edited. A group screen belongs beside the
directory adapter's own surface (Prompt 29).

**No browser test runner.** Rendering is asserted with `react-dom/server` and
the `toText` reduction, the convention CNSL-1 established: what is covered is
*which facts appear*, and what is not covered is anything needing events and
effects — the switchers' `onChange`, the wizard's step transitions, the
mutation round trips. Those are asserted at the seam below them (the route
table, the selection reconciliation, the cache, the client's wire shape),
which is where the logic actually lives.

**No demo script.** The 72-test console suite is this feature's
demonstration, and `make ci` runs it (`ts-test`) where no CI target runs a
demo. A console demo needs a browser flow against a live Rauthy and a live
stack — `demos/cnsl-1-proposals-inbox.sh` is 542 lines of exactly that — and
one written without being run would be an unverifiable script rather than a
demonstration. Two clauses of the definition of done are vacuous here and
said rather than skipped: no server path is added, so there is no span or
metric to add; no new action type is added, so there is no audit event to
emit.

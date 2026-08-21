# ADR-0075: The console as a product shell — routes, selection, generated client, onboarding

- **Status**: Accepted
- **Date**: 2026-08-21
- **Feature(s)**: CPR-8
- **Deciders**: Prompt 8 of the CPR programme

## Context

Six prompts of this programme built a context platform: governed scopes
(CPR-3), workspaces and projects (CPR-4), membership and invitations (CPR-5),
anchors and the re-cut PDP (CPR-6), and one scope tree (CPR-7). Every one of
them is reachable from the CLI. **None of them is reachable from the
console.**

What the console is, at this commit, is what CNSL-1 and CNSL-2 built for a
different audience. `App.tsx` resolves a session with `GET /v1/whoami` and
then mounts two components one after the other — the proposals inbox and the
scope explorer — with no navigation between them and no way to reach anything
else. That is a governance console: the right first screen for the person who
reviews other people's publications, and the wrong one for everybody else.
For a product whose smallest unit is now one person (ADR-0068), it is the
wrong first screen for the person who just installed it, who has no
proposals, no scopes they did not create, and nothing at all until they make
a workspace — which the console cannot do.

Three further forces:

- **`GET /v1/me` exists and nothing uses it.** CPR-4 built it precisely as
  "the one call a client makes first" — who, which tenant, what exists, what
  is missing, what this caller may do — and its `onboarding` field is the
  server's own answer to "is this person set up yet" (ADR-0071 decision 2).
  The console still asks `whoami`, which answers the first two of five.
- **There is a generated client and nothing uses it either.** CPR-4 made
  `docs/api/openapi.json` authoritative and generates
  `console/src/generated/api.ts` from it. Every console call is still
  hand-written against a hand-written path string.
- **The bundle's dependency list is governed.** `scripts/check-npm-licences.mjs`
  holds shipped dependencies to `deny.toml`'s allowlist with no exception
  mechanism, and `console.rs` serves the bundle under `default-src 'none'`
  with `connect-src 'self'`. Anything installed here is a supply chain
  somebody owns and an air-gap claim somebody has to keep true.

## Decision

We take seven decisions.

**1. The console becomes a route-based product shell.** A flat route table
(`routes.mts`) with two menus: a **primary** menu that is the product — Home,
Sessions, Knowledge, New Learnings, Skills, Tools, People, Settings — shown to
everybody unconditionally, and an **advanced** menu that is governance —
Reviews, Scopes, Policies, Audit, Service identities — shown only where the
caller's capability forecast offers the plane behind it. Routing is written
here (History API, `popstate`, a `Link` that is a real `<a href>`) rather than
installed, for the dependency reason above and because the table is flat by
design; the gateway already answers every path under `/console/` with the
bundle for exactly this.

The primary menu is unconditional on purpose. A navigation that grew and
shrank with a role would teach every reader a different shape for one
application, and the first question a new user asks — *what is this?* —
should have the same answer for all of them.

**2. The current workspace and project are a persisted, reconciled
selection.** Two ids in `localStorage`, reconciled on every load against what
`/v1/me` says the caller can read: a preference that still exists wins,
otherwise the first thing that does, and a project is dropped when it does not
belong to the chosen workspace. Not in the URL, because the selection is a
property of the reader rather than of the page — a selection in the query
string makes every copied link pin a project the recipient may not be able to
read.

**3. Contract-covered routes go through a generated client; the rest are
marked.** `client.mts` is typed by the generated `Operations` table: an
operation id, its path parameters, its query and its body, all checked against
the document. The generator now also emits the **runtime** half of that table
(`OPERATIONS`) and an `idempotent: true` flag for every operation whose
document requires an `Idempotency-Key`, so the client can require the key at
compile time and there is no hand-written second copy of a path anywhere.

The rest of `/v1` — proposals, capabilities, policy packs, lapses, audit,
skills, service identities — stays hand-written in `api.mts`, grouped and
labelled as "not on the contract yet". A hand-written call is a **marker**
rather than a style: it names work Prompt 19 does. Hiding those behind the
same facade would make them look generated, which is worse than a hand-written
call you can see.

**4. One query/cache layer, and therefore one loading and one error state.**
`cache.mts` is a map, a listener set and four rules — a key is the identity of
a read, an entry is immutable, invalidation refetches what is watched, and a
refusal is a cached answer. `Query.tsx` binds it to React and owns
`Loading`/`Failure`/`Loaded`, so no page writes its own. Written rather than
installed, same reason as decision 1.

**5. The People page answers "why", not just "who".** Workspace members,
**project-only** members (derived from each row's own `inherited` flag rather
than by diffing two lists), pending and settled invitations, each row carrying
its role, its access source, the scope the grant is actually written at, the
group it came through and whether a directory owns it. Invite, revoke and
remove are offered exactly where the API would accept them — absent, not
disabled, where it would not.

**6. First-run onboarding seeds; it does not brand.** Six steps: workspace,
project, repository, agent client, connection instructions, connection check.
The personal/team question **seeds** two things — the policy pack assigned at
the new workspace's scope, and whether a group and invitations are set up
beside it — and then it is over. Nothing records the choice, no column holds
it, and nothing downstream branches on it: ADR-0068 decision 1 forbids an
edition, and a wizard asking "is this just you?" is the friendliest possible
door for that branch to arrive through.

The seeding is **best-effort and reported**. A first caller holds an `owner`
grant on what they just created and may hold nothing that permits
`policy.assign`; a refusal is rendered as a sentence pointing at Advanced ▸
Policies and never blocks the wizard. Silently skipping it is the one
unacceptable option — somebody would leave believing their workspace was
governed the way they chose.

The connection check claims only what a browser can prove. The console's own
CSP confines it to this origin, so it verifies that the gateway answers this
reader, that the chosen project is readable and what is attached to it — and
it states, on the page, that it cannot see whether the agent client on the
reader's machine is installed or holds a credential.

**7. A plane with no API gets an honest page, not a hidden menu item or an
empty list.** Sessions, Knowledge, New Learnings and Tools are in the primary
menu because they are what the product is, and none of them has an API at this
commit. Each renders what will be there and which piece of work delivers it.
An empty list would be indistinguishable from a plane that works and has
nothing in it, which is precisely the wrong thing to tell somebody whose agent
has been running all week.

## Options considered

1. **Install a router and a query library** (react-router, TanStack Query) —
   fewer lines here, and everything they add over a flat table and four cache
   rules (nested layouts, data loaders, retries, suspense, devtools) is
   machinery this console has no use for. Against: two shipped dependencies in
   a bundle whose licence gate has no exception mechanism, in a product sold
   into air-gapped deployments, to avoid roughly two hundred lines that are
   entirely tested. Rejected on that trade, not on principle: if the route
   table ever stops being flat, this is the decision to revisit.
2. **Gate the primary menu on capabilities too.** Tidier — nobody sees a page
   they cannot use. Rejected: the shape of the product would become a function
   of who is looking at it, and Sessions and Knowledge are what the product
   *is* rather than a privilege.
3. **Hide the planes that have no API.** Would avoid four pages that fetch
   nothing. Also means the navigation grows as the backend catches up, so no
   two readers a month apart see the same product — and it hides the shape
   this feature exists to establish.
4. **Record the personal/team choice on the workspace.** Would let later
   features "know" which kind it is. This is exactly the edition conditional
   ADR-0068 decision 1 forbids, and the moment it exists something branches on
   it. Rejected outright.
5. **Put the selection in the URL.** Deep-linkable. Also makes every shared
   link carry a project id, which is a disclosure nobody decided and a 404 for
   the recipient who cannot read it.
6. **Do nothing** — the console keeps showing a review queue to a person who
   has never published anything, and six prompts of platform stay
   CLI-only.

## Consequences

- **Positive.** The context platform is reachable from a browser. A person can
  go from a fresh install to a workspace, a project, a repository and a
  connected agent client without reading `docs/INSTALL.md`, and the six
  governance surfaces are still one click away for whoever holds the roles.
- **Positive.** The contract got stricter in the process. `list_scopes`
  declared its `parent_id` as a **path** parameter in a route that has no such
  placeholder — a defect invisible while every caller was hand-written, and
  the first thing a generated client trips over. Fixed at the source
  (`#[into_params(parameter_in = Query)]`), document and types regenerated.
- **Positive.** Idempotency is now a compile-time obligation on the eight
  operations whose document requires it, rather than a convention.
- **Negative / accepted.** Four primary pages render prose instead of data
  until Prompts 9–15 build their planes. This is deliberate (decision 7) and
  it is the visible cost of establishing the shape first.
- **Negative / accepted.** Seven surfaces still call hand-written paths, and
  the console therefore has two ways of reaching the gateway until Prompt 19
  puts the rest of `/v1` on the contract. They are grouped, labelled, and
  their disappearance is a named piece of work rather than a hope.
- **Negative / accepted.** A router and a cache are now this repository's to
  maintain. Both are small and both are tested; the reversal trigger is
  above.
- **Reversal trigger.** If the route table needs nesting, per-route data
  loaders, or route-level code splitting, decision 1's build-it-here trade has
  failed and a router is the right answer. If a page ever needs to read a
  capability forecast to decide whether an **act** is permitted rather than
  whether to offer it, the guard has become enforcement and ADR-0058 decision
  2 has been broken.

## Compliance notes

- **Policy enforcement.** Nothing here decides anything. The capability
  forecast in `/v1/me` chooses what to render; every act still takes its own
  decision at its own seam, under the pack effective then, and a reader who
  gets past a stale forecast meets the gateway's refusal rendered in the
  page's own error state. ADR-0058 decision 2 is restated at the top of
  `routes.mts` where somebody editing the table will meet it.
- **No console-only route.** ADR-0056 decision 9 is standing and unbroken:
  every call this feature makes is a route the CLI also has. Onboarding in
  particular is an ordinary client of `POST /v1/workspaces`,
  `POST /v1/workspaces/{id}/projects`, `POST /v1/projects/{id}/repositories`
  and `PUT /v1/admin/scopes/{id}/policy` — an installer or a wizard runs once,
  before anybody is watching, which makes it the worst place in a product to
  keep a shortcut (seed §2.2).
- **Secrets.** The one credential this feature can produce is an invitation
  token, which the API returns exactly once at creation and on no other route
  (ADR-0072). It is held in component state so the inviter can copy it, it is
  never cached (the query cache holds reads, not mutation responses), and the
  page says in the banner that it exists nowhere else.
- **Tenancy and audit.** Unchanged. Every read this console makes is a `/v1`
  call behind tenant resolution, and the events those routes chain are the
  ones they already chained.
- **Air gap.** No dependency was added: the shipped set is still React and
  React-DOM. `scripts/check-npm-licences.mjs` reports 3 shipped packages
  before and after, and the Content-Security-Policy is untouched.

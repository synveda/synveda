# ADR-0076: Sessions as the root of agent runtime activity

- **Status**: Accepted
- **Date**: 2026-08-23
- **Feature(s)**: CPR-10
- **Deciders**: Prompt 10 of the CPR programme

## Context

ADR-0068 decision 5 is one paragraph and it is the whole of what this ADR
implements:

> **Sessions are the root of agent runtime activity.** A session is a
> first-class, tenant-bound aggregate with a stable id. Observed events,
> extracted candidates, recalls, injections and their audit events all hang
> off it. `session_id: String` as a correlation hint is deleted.

What exists at this commit is the correlation hint. `observe_events.session_id`
is `text`, documented as an "opaque harness session identifier", with a CHECK
that it is between 1 and 200 characters and nothing else — no row anywhere
says a session exists, when it started, who ran it, which agent client it was,
whether it is still open, or which governed scope its work belongs to.
`/v1/inject` and `/v1/recall` take the same string as an optional field and
copy it into an audit payload. `synveda mcp` mints `mcp-<random>` once per
server process and sends it along. Nothing reads any of them back.

Four consequences follow from that, and each of them is a thing this product
claims to do and cannot:

1. **A session an agent only *read* in does not exist.** The string appears in
   `observe_events`, so a run that injected context and never observed
   anything leaves no trace at all. ADPT-8 measured exactly this against a
   headless Claude Code run: three sessions, three `inject.ok`, **zero**
   `observe.done`, exit 0. The product's own adapter produced three runs the
   product cannot name.
2. **A session cannot be governed.** There is no resource for the PDP to
   decide about, so "who may see what my agents have been doing here" is not a
   question this deployment can be asked. The nearest available answer is
   "whoever may read the observe events", which is a different question with a
   different blast radius.
3. **A session cannot be retained, ended or audited.** No lifecycle, no
   `session.opened`, no `session.ended`, nothing for TEN-5's disposal to walk.
4. **The console cannot show one.** CPR-8 put Sessions first in the primary
   menu because it is what the product *is*, and had to render an honest
   placeholder page because there is no plane behind it.

This ADR decides the aggregate, its lifecycle and its governance. It
deliberately decides **nothing** about what hangs off it: session events are
the next prompt, candidates the one after, and the composition path later
still. What it must not do is make those prompts harder, which is why the
shape below is the narrowest one that is still a real aggregate.

## Decision

**1. A session is a row, attached to a governed scope.**

`sessions` is a tenant-bound table with a stable `SessionId`. It carries a
`scope_id` — the governed scope the run's work belongs to — and that scope is
what the PDP decides against, what policy is assigned to, and what everything
the session produces will attach to. A session is **not** a subtype of a scope
the way a workspace and a project are (ADR-0071): it does not own one, it
names one. A workspace is a place; a session is something that happened at a
place.

Any scope shape may host a session and none is required. The ordinary case is
a project's scope. A person's own agent, working on nothing shared, opens one
at their `principal` scope — where the base layer's privacy forbid (ADR-0073
decision 6) makes it nobody else's, with no flag, no column and no second code
path. That is decision 1 of ADR-0068 discharged for this noun: the person
working alone and the bank differ in which scope they name.

**2. Two states, and ending is a verb.**

`status` is `active` or `ended`. There is no `abandoned` status and no sweeper
that would set one, because nothing in this product sweeps sessions yet and a
state nothing produces is a lie in a vocabulary. What a client *can* say is
how the run finished, so ending takes an `outcome` — `completed`, `failed` or
`abandoned` — and the database holds `(status = 'ended') = (outcome is not
null)` and the same equivalence for `ended_at`.

Ending is `POST /v1/sessions/{id}/end`, not `PATCH /v1/sessions/{id}`. A PATCH
whose only legal body is `{"status": "ended"}` documents an update surface
that does not exist: every other column on this table is immutable, so the
patch verb would be an invitation to add mutable ones.

**3. No revision, and therefore no `expected_revision`.**

Every other row on the context-platform plane carries a monotonic `revision`
and every update names it as a precondition (ADR-0071 decision 5). A session
carries neither, and the reason is what a precondition is *for*: it stops a
lost update — two writers, two different target states, the second silently
overwriting the first. Ending has exactly one target state. Two concurrent
ends are not a lost update; they are one transition and one refusal, and the
refusal is already exact because the transition is `where status = 'active'`
and the 409 names when the session ended and who ended it. Adding a number for
the client to echo back would buy nothing and would have to be kept monotonic
by a trigger that exists for no other reason.

This is stated rather than skipped because the plane's rule is otherwise
uniform, and a reader who finds one table without a revision should find the
argument beside it.

**4. The harness's own identifier is an attribute, not the identity.**

`client_session_ref` holds whatever the agent harness calls this run — Claude
Code's session uuid, an MCP server's process token. It is nullable, it is
never an identity in this product, and nothing joins on it. What it is for is
the stateless hook: a `Stop` hook runs in a fresh process holding only the
harness's id, and without this column it would have to either keep state on
disk or mint a second session.

It is unique per `(tenant, principal, client)` when present, so "find the
session I opened for this run" is one indexed read and two clients cannot
squat each other's references. This is the honest replacement for
`session_id: text`: the correlation string becomes a **named, bounded,
tenant-bound attribute of an aggregate that exists**, rather than a field on
somebody else's table that means whatever the last writer meant.

**5. No metadata bag.**

Every other product-level table on this plane that could carry one does
(`scopes.attributes`, `project_repositories.metadata`). This one does not, and
the reason is specific rather than stylistic: the open bag on a *session* is
where an agent harness would put its environment — the working directory, the
model, the arguments, the variables — and an agent's environment is precisely
where credentials live. The seed's rule is that secrets never appear in
ordinary API responses, logs or audit payloads; a bag that would need a secret
scanner in front of it before it could be served, logged or audited is a bag
this table should not have until something needs it enough to bring the
scanner. Named, bounded fields only: `client`, `client_version`, `title`.

**6. Two actions: `SessionRead` and `SessionWrite`.**

Opening and ending share one authority. The separability rule this schema uses
elsewhere (`ChannelPublish` versus `ChannelRollback`) asks whether a
deployment would ever want to price two acts differently; an agent that may
open a session must be able to close it, and an authority to open that could
not close would produce sessions nobody can end. Ending *somebody else's*
session at a shared scope is permitted by the same key, and that is correct: a
project's owner should be able to close a runaway agent's session in their own
project. Nobody can reach a session at another person's `principal` scope at
all, because the base layer forbids it and neither action is on that forbid's
governance carve-out.

`Session` is a Cedar entity parented to the scope it names, with `tenant` and
`scope` attributes — ADR-0073 decision 3's rule, so a decision and its audit
event name the session rather than the scope it happens to sit in. It
deliberately carries no `principal` attribute: the ownership distinction a
pack might want to write over one is already expressed by *which scope the
session was opened at*, and adding a second way to say it would be a second
answer to one question.

**7. Every listing decides per row, against the row.**

CPR-9's rule, applied from the start rather than retrofitted. A session
listing takes one gather and one Cedar evaluation per row under that row's own
chain and pack assignments, with no fast path for a caller permitted at the
root. The chain and assignment **reads** are memoised per scope inside one
request — that is a read cache, not a decision cache, and the decision is
still taken once per row.

The listing is bounded and says so: at most `SCAN_LIMIT` rows are considered,
ordered newest first, and the envelope carries `truncated`. This is a
different thing from the cap CPR-9 refused. That cap would have dropped rows
from a *complete* inventory of workspaces with no way for a client to notice;
this is a recency-ordered feed of an unbounded event-like table, where "the
most recent N" is a well-defined answer, the ordering is documented, and the
flag says when there are more.

**8. A session is never deleted.**

No `DELETE` grant, no delete route. A session is what events, candidates,
knowledge provenance and audit events will name. Disposal belongs to the
retention plane, which owns the whole tenant's material and its schedule.

## Options considered

**1. Keep `session_id: text` and add a `sessions` view over `observe_events`.**
Cheapest, and refused by ADR-0068 by name (its option 4). A session that has
to be aggregated out of rows written for another purpose cannot be retained,
governed or asked about — and the session an agent only read in does not
exist, which is exactly the session ADPT-8 measured three of.

**2. Make the session a subtype of a scope, like a workspace and a project.**
Symmetric with ADR-0071 and wrong. A scope per session means the scope tree
grows by one node per agent run, `scope_closure` grows quadratically in the
depth of that tree, and every policy walk, capability probe and anchor
resolution pays for it. A scope is a place authority applies at; there is no
authority anybody would write at a single run.

**3. Require a project.** `sessions.project_id` instead of `scope_id`. Reads
well — "every run of an agent against this project" is the console's own
sentence — and forecloses the case this programme exists for: the person whose
agent is working on their own material, who would have to invent a project to
hold it, or the deployment that wants a session at a workspace before any
project exists. A project's scope is still the ordinary answer; it is simply
not the only one the column can hold.

**4. A three-state lifecycle with `abandoned` set by a sweeper.** Correct
eventually and dishonest now: nothing sweeps, so the state would be
unreachable, and an unreachable state is one that quietly means "this never
happens" until somebody builds the sweeper and discovers what it should have
meant. `outcome = 'abandoned'` on an explicit end says the same thing a client
actually knows, and a sweeper can be added later without changing what
`status` means.

**5. Carry `expected_revision` on the end transition for uniformity.** See
decision 3. Uniformity that costs a column, a trigger and a required argument
in exchange for a guard against a failure that cannot occur is not uniformity;
it is a rule applied where its reason does not reach.

## Consequences

- **Positive.** The thing an agent does becomes a thing the product can name,
  decide about, audit and show. Every later prompt of Stage B — session
  events, candidates, promotion, redaction — has a parent aggregate to hang
  off, and the composition and recall paths have one to attribute to.
- **Positive.** `GET /v1/sessions` is the first plane in this programme whose
  volume is unbounded by anything a human does, which is why it is also the
  first with a documented listing bound. The shape is available to the
  candidate and event listings that follow.
- **Positive.** A private session is private by construction, through the same
  base-layer forbid that protects a private note. No `visibility` column
  exists, and ADR-0068 decision 1 keeps it from arriving.
- **Negative / accepted.** `observe_events.session_id` still exists and still
  means nothing. This prompt does not touch the observe path — that is the
  next one — so for exactly one commit the product has both a session
  aggregate and a correlation string. The string is deleted with the observe
  re-cut, and nothing synchronises them meanwhile: no code reads one to
  populate the other, in either direction.
- **Negative / accepted.** A session carries no free-form metadata, so a
  harness with something to record beyond `client`, `client_version` and
  `title` has nowhere to put it until a later prompt decides how such a field
  is scanned. That is the trade decision 5 takes deliberately.
- **Reversal trigger.** If a pack ever needs to decide about a session by a
  property that is not its scope — the client that ran it, the person who
  opened it, whether it is still open — then decision 6's minimal entity is
  wrong and the attribute belongs on it, materialised beside `tenant` and
  `scope`. Equally: if the listing's `truncated` flag is ever *true* in
  ordinary use rather than under load, the bound is a paging surface pretending
  to be a limit, and the plane needs a cursor.

## Compliance notes

- **Tenancy.** `sessions` is tenant-bound with forced RLS, a tenant-isolation
  policy and least-privilege grants (`select, insert, update`; no `delete`) in
  the migration that creates it — ADR-0009's structural rule, and the
  completeness guard in `crates/synveda-store/tests/rls.rs` enforces it.
- **PDP.** Every route on the plane decides through `authorize()`; there is no
  path that reads or writes a session without one, in production or in tests.
  Ownership is checked before the decision on every per-object route, so a
  foreign id is a 404 rather than a denial oracle (ADR-0012 decision 7).
- **Audit.** `session.opened` and `session.ended` are new action types on the
  hash chain; reads chain the allowed-read decision event (ADR-0019
  decision 4) like every other governed read.
- **Secrets.** No column on this table can hold one: `client`,
  `client_version`, `title` and `client_session_ref` are bounded scalars, and
  decision 5 refuses the bag that would be the obvious place for one to
  arrive.
- **VedaFlow.** A session is runtime activity, not governed configuration or
  published knowledge, so it does not pass through a proposal. What a session
  *produces* does, and that is the boundary the candidate/knowledge split
  (ADR-0068 decision 6) draws two prompts from here.

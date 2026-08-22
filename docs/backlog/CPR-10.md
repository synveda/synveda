---
title: "CPR-10: The session ledger and runtime API"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-10: The session ledger and runtime API

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Prompt 10 of the 33-prompt context-platform programme, and the first of
Stage B: the plane that makes **what an agent does** a thing this product can
name, decide about, audit and show.

ADR-0068 decision 5 is one paragraph and this feature is all of it:

> **Sessions are the root of agent runtime activity.** A session is a
> first-class, tenant-bound aggregate with a stable id. Observed events,
> extracted candidates, recalls, injections and their audit events all hang
> off it. `session_id: String` as a correlation hint is deleted.

## What existed before it

A string. `observe_events.session_id` is `text`, documented as an "opaque
harness session identifier", with a CHECK that it is 1–200 characters and
nothing else. `/v1/inject` and `/v1/recall` take the same string as an
optional field and copy it into an audit payload. `synveda mcp` mints
`mcp-<random>` once per server process. Nothing reads any of them back.

Four things followed from that, and each is something this product claims to
do and could not:

1. **A run an agent only *read* in did not exist.** The string appears in
   `observe_events`, so a run that injected context and observed nothing left
   no trace at all. ADPT-8 measured exactly this against a headless Claude
   Code run: three sessions, three `inject.ok`, **zero** `observe.done`,
   exit 0.
2. **A run could not be governed.** No resource for the PDP to decide about,
   so "who may see what my agents have been doing here" was not a question a
   deployment could be asked.
3. **A run could not be retained, ended or audited.** No lifecycle, no
   `session.opened`, no `session.ended`.
4. **The console could not show one.** CPR-8 put Sessions first in the primary
   menu because it is what the product *is*, and had to render a placeholder.

## What it adds

**Three tables** (migration `0044`), one aggregate:

- `sessions` — one run: the workspace and (optionally) the project it happened
  in, the derived governed scope it is decided at, who opened it, the client
  and its version and installation, the harness's own id, the agent, the
  model, the repository and branch, a task summary, a five-state lifecycle,
  the start/end/last-observed instants, and a bounded metadata bag.
- `session_events` — immutable, append-only, ordered, idempotent: the event
  type from a closed twelve-name vocabulary, the client's declared schema
  version, its own `client_event_id`, a server-assigned `sequence`, both
  `occurred_at` and `received_at`, a bounded payload and the server's BLAKE3
  digest of it.
- `session_context_runs` — one act of composing context for a run: the query,
  the rendered block, its hash, the tokens against the budget, the entry
  count, and which retrieval legs degraded.

**Seven routes**, all on the OpenAPI contract from the day they exist:

| | |
|---|---|
| `POST /v1/sessions` | open a run (`Idempotency-Key`) |
| `GET /v1/sessions` | the runs this caller may read, filtered and bounded |
| `GET /v1/sessions/{id}` | one run |
| `POST /v1/sessions/{id}/events` | append a batch, idempotent per event |
| `POST /v1/sessions/{id}/end` | move it through its close |
| `GET /v1/sessions/{id}/timeline` | the projection |
| `POST /v1/sessions/{id}/context-runs` | compose context for it |

**Two Cedar actions** — `SessionRead` and `SessionWrite` — a `Session` entity
parented to the scope it runs at, and the permits in all three shipped packs
(`@18 → @19`).

**Four audit action types**: `session.opened`, `session.ended`,
`session.events.appended`, `session.context.composed`.

**A console page**: Sessions is the first of CPR-8's four planned pages to get
a plane behind it, driven entirely through the generated client.

## Decisions worth reading (all in ADR-0076)

**The governed scope is derived, never submitted** (decision 1). A session
names a workspace and optionally a project; the scope it is decided at is the
project's when there is a project and the workspace's when there is not — and
that is a row-local fact, not a service's discipline. Three columns carry it,
each pinned by a composite foreign key, with a CHECK holding
`scope_id = coalesce(project_scope_id, workspace_scope_id)`. A client that
could name the scope could name one its workspace is not in.

**Five states, because the close is two-phase** (decision 2). An adapter
learns a run is over at a hook that must return quickly and usually still has
events buffered. `ending` is what it says then — *no new work, I am flushing*
— and `ended` is what it says when the flush lands. Collapsing them means
either an adapter that blocks its host while it drains, or a run that reads as
finished while its last five events are still arriving. `abandoned` (nobody
closed it) and `failed` (it broke) are separate because they call for
different things.

**No revision, and therefore no `expected_revision`** (decision 3). Every
other row on this plane carries one; a precondition stops a *lost update*, and
this aggregate has no update to lose. Ending has one target state, so two
concurrent ends are one transition and one refusal — and the refusal is
already exact, because the transition is conditional on the state and the 409
names the state the run is in.

**The harness's own id is an attribute, not the identity** (decision 4).
`external_session_id` is nullable, joined on by nothing, and unique per
`(tenant, principal, client_name)`. It exists so a stateless hook holding only
the harness's id can find the run it already opened rather than minting a
second.

**Two idempotency mechanisms, and they are not redundant** (decision 8).
Opening a run and composing a context run each take a required
`Idempotency-Key`. Appending events does not: its unit is the *event*, keyed
by `client_event_id`, because a redelivered batch overlapping a previous one
by three of ten must append seven and answer `duplicate` for three — which a
request-level key cannot express.

**Nothing on the wire carries a tenant or an acting principal** (decision 8).
Every body is `deny_unknown_fields`, so a client sending `tenant_id`,
`principal_id` or `scope_id` is **refused rather than ignored** — because a
server that silently dropped the field would behave correctly and teach every
client author that it works.

**The timeline is a projection** (decision 9). Events and context runs merged
and ordered at read time. There is no timeline table and there must not be
one: a materialised transcript would be a second copy of `session_events`, and
the two would disagree the first time one was written and the other was not.

**A listing decides per row against the row.** CPR-9's rule, applied from the
start rather than retrofitted, and bounded and honest about it: at most 500
candidates are considered, newest first, and the envelope carries `truncated`.
That is a different thing from the cap CPR-9 refused — a complete inventory of
workspaces silently losing rows — because this is a recency-ordered feed of an
unbounded event-like table, where "the most recent N" is a well-defined answer
and the flag says when there are more.

## Two defects its own tests found, and only one was its own

**`SessionEventType` answered with one spelling and refused the other.** The
enum derived `serde(rename_all = "snake_case")` beside an `as_str()` of
`message.user`, so a request body naming `message.user` was a 400 quoting
twelve names nobody would send. Four integration tests failed at once, which is
the good version of that mistake. Per-variant renames, with a unit test that
walks every variant asserting serde and `as_str` agree.

**`payload_hash` was not canonical.** It hashed `Value::to_string()` on the
belief that `serde_json::Map` is a `BTreeMap`, so an event re-sent with its
keys in a different order would have got a different digest — and idempotency
is what the digest is for. It is an `IndexMap` in this workspace, because
**`cedar-policy-core` enables `serde_json/preserve_order`** and Cargo unifies
features across a build.

Chasing it found the more interesting half: CPR-4 had already written the same
recursion inside the gateway's idempotency seam, with a comment saying it was a
no-op "today" and was kept only against the day somebody turned the flag on.
The flag was already on when that comment was written. The mechanism was right
and its stated reason was wrong — the failure mode a comment is worst at
catching. The canonicaliser now lives once, in `synveda_types::json`; both
callers use it, the gateway's private copy is deleted and its comment
corrected. The behaviour also changes with the build's *scope* — `cargo test -p
synveda-types` has no Cedar in its graph and the two encodings match there,
`cargo test --workspace` unifies the feature in and they do not — which is why
the canonicalisation is unconditional and why nothing asserts the raw strings
differ.

## Acceptance criteria

- An agent opens a run through `POST /v1/sessions`, and the run's governed
  scope is the project's — derived, with no request naming it.
- A body naming `tenant_id`, `principal_id` or `scope_id` is a 400, not a
  silent no-op; the acting principal on the stored row is the token's.
- Opening is idempotent by the header: the same key replays with 200, a
  different body with the same key is 409, and no key at all is a 400 naming
  the header.
- Appending is idempotent by the event: a redelivered batch appends only what
  is new, reports `duplicate` for the rest, serves the **stored** rows at
  their original positions, and a batch repeating an id inside itself is
  refused by name.
- A run in `ending` still accepts buffered events; a closed run accepts none,
  never reopens, and never changes how it closed — at the API and at the row,
  against direct SQL.
- `POST …/context-runs` composes through the existing retrieval engine,
  persists the identity and the rendered block, and chains the watermark.
- The timeline merges events and context runs in one order, reports the run's
  event counts, and no timeline table exists.
- Every route refuses a caller who holds nothing with a 403 naming the action;
  a caller granted `member` at one project sees that project's runs and no
  others, with the listing and the per-object route agreeing.
- A session id from another tenant is a 404 with the same error kind as one
  nobody ever minted, on every per-object route.
- A tenant with no governed scopes is **answered** rather than errored.
- The session's `metadata` never reaches the audit chain — its size does.
- An append chains **one** audit event however many events it carried,
  carrying counts, the sequence range and the per-type breakdown, never the
  events.
- An administrator is offered neither `session.read` nor `session.write` at
  somebody else's `principal` scope.
- A payload's digest is over its content and not over the order a client wrote
  its keys in.
- All three tables are tenant-bound with forced RLS; the application role
  holds no UPDATE or DELETE on `session_events` or `session_context_runs`, and
  no DELETE on `sessions`.

## Definition of done

1. Acceptance criteria met and demonstrated — `sessions_api.rs` (14), the
   store-level rules in `rls.rs` (5), and `demos/cpr-10-sessions.sh`.
2. Tests written — 14 integration, 5 RLS/store, 17 unit.
3. Tracing spans + metrics on new paths — `synveda_session_operations_total`
   on the gateway plane, `synveda_session_mutations_total` in the store, and a
   `#[tracing::instrument]` span on every store service.
4. Audit events emitted — four new action types.
5. docs/backlog/STATUS.md updated.

# ADR-0077: The session product surface — paging, delivery lateness, diagnostics and end reasons

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-11
- **Deciders**: Prompt 11 of the CPR programme

## Context

CPR-10 (ADR-0076) made a run a governed record: three tables, seven routes,
two Cedar actions, an audit chain, and a console page that listed the newest
runs and expanded one in place. What it did **not** do is make that record
usable by somebody who has a question about a particular run.

Four things stood in the way, and each of them is a hole rather than a
polish item.

1. **A run older than one answer was unreachable.** `GET /v1/sessions`
   served up to `limit` rows, newest first, and set `truncated: true` when
   there were more. `truncated` says *that* an answer was cut short; it
   cannot say **where to continue**. A deployment whose agents open a few
   hundred runs a week therefore had no API path to last Tuesday's run at
   all — not a slow one, none. And the listing's filters were `scope_id`,
   `workspace_id`, `project_id` and `status`: nothing narrowed by who ran a
   thing, which client, or when.

2. **A timeline reported one clock.** Every entry carried `at` — for an
   event, the client's own `occurred_at`. `session_events` has stored
   `received_at` since migration `0044` and the API served it nowhere. That
   matters because the adapters this product ships **spool to disk when the
   gateway is unreachable** and flush later: an hour of a transcript can
   arrive at once, an hour late, and a reader with one clock sees a
   perfectly plausible transcript with no sign that any of it was recovered.

3. **There was no way to see what was actually said, and no way to stop
   people seeing it.** A timeline entry carries a one-line `summary` derived
   from a payload's conventional keys; the payload itself is served by no
   read route at all — only echoed back to the client that wrote it. So the
   forensic question ("what exactly did the agent send?") had no answer, and
   the moment one is added it becomes the single largest disclosure on this
   plane, priced identically to reading a project's session list.

4. **A run said how it stopped and never why.** `abandoned` and `failed` are
   two of five states and CPR-10 was right to keep them apart. But a
   `failed` run with nothing else on it tells a reader that something broke
   and gives them nowhere to go next.

## Decision

### 1. The listing is keyset-paginated, and `truncated` is deleted

`GET /v1/sessions` takes an opaque `cursor` and answers with `next_cursor`,
absent on the last page. `truncated` is **gone** rather than kept beside it:
two ways to say "there is more" is two contracts, and only one of them can be
followed.

The cursor is a keyset over the listing's own order, `(started_at desc, id
desc)` — not an offset. An offset re-counts rows on every page and skips or
repeats whenever a run is opened between two requests, which on a table a
fleet of agents writes to all night is every request. It is
`base64url("<rfc3339>|<uuid>")`: opaque so no client comes to depend on its
shape, not secret — it carries a row the client was just served. A cursor
this listing did not issue is a **400**, not a silent restart from the newest
row, because a client sending one is looping and answering page one forever
is an infinite scroll nobody notices.

**The cursor follows the last candidate the page considered, not the last row
it served**, and this is the part that is easy to get wrong. Rows are decided
one at a time against the row (CPR-9), *after* they are scanned. If the
cursor were the last row served, a page whose candidates were all denied
would serve nothing, carry no cursor, and end the listing while readable rows
sat below it. So a page may be **empty and still carry a cursor**, and the
response schema says so. The alternative — a server that keeps scanning until
it fills a page — is unbounded work driven by rows the caller cannot read.

Four filters join it: `client_name` and `principal_id` (exact, never a
prefix — `zed` and `zed-nightly` are two clients) and `started_after` /
`started_before`, half-open so `[after, before)` composes and two adjacent
days cover every run exactly once. A window that ends before it starts is a
400 rather than an empty list, because an empty list reads as "no runs".

### 2. A timeline reports both clocks, and the **server** decides what is late

Every event entry carries `received_at` beside `at`, and a boolean `delayed`
computed against one threshold — 60 seconds, well above a live hook's
round trip and well below a spool replay. A context run carries neither: a
composition happens here, so its two instants would be one number written
twice.

`delayed` is computed on the server rather than left to each client so that
"this did not arrive live" means one thing across the console, the CLI and
anything else that reads a timeline. It is deliberately **one flag and not
three**: a locally spooled batch, a replay after a crash, and a machine whose
clock is an hour out produce the same two instants, and the server cannot
tell them apart. It reports the gap; it does not name a cause. A console that
labelled the third case "recovered from a local spool" would be inventing a
distinction the data does not contain.

Skew in the other direction — `received_at` earlier than `occurred_at` — is
**not** late. It is something else, and reporting it as late would be a
second wrong answer on top of the clock's.

### 3. A raw payload is its own authority: `SessionDiagnostics`

`GET /v1/sessions/{session_id}/events/{event_id}` serves one event with its
payload, decided under a **new** Cedar action `SessionDiagnostics`
(`session.diagnostics`), not under `SessionRead`.

The split is the decision. A timeline says a message was sent, a tool ran, a
file changed, and summarises each in a line. A payload is what the person and
the agent actually said, byte for byte: the prompt, the tool arguments, the
diff. A pack must be able to let a project's members follow what their agents
have been doing without handing every one of them a transcript of everybody's
prompts — the same argument `SessionRead` itself makes against `ProjectRead`
(ADR-0076 decision 6), one level further in.

In the three shipped packs it is **strictly narrower than that pack's own
`SessionRead`**, and packs move `@19 → @20`:

| Pack | `SessionRead` | `SessionDiagnostics` |
|---|---|---|
| `regulated-strict` | own chain, or a content key tenant-wide | own chain, or `reviewer`/`owner`/`administrator` |
| `standard` | own chain, `principal.ambit`, or a content key | own chain, or `reviewer`/`owner`/`administrator` — **no `ambit`** |
| `open-collaboration` | anything in the tenant, role-free | own chain, or **any** grant in the tenant |

`standard` deliberately does not extend it by `principal.ambit`: sharing one
step outward is a decision about a *reading surface*, and a neighbouring
project's raw prompts are not a default under any pack. Under
`open-collaboration` the narrowing is still real — somebody holding nothing
anywhere can see that a run happened and cannot read what was said in it.

It is not added to `base.cedar`'s governance carve-out, so the personal-scope
privacy forbid reaches it exactly as it reaches `SessionRead` and
`SessionWrite`: a run at somebody's own `principal` scope is not the tenant
administrator's to expand.

**One event per request**, deliberately. A bulk "every payload of this run"
route is the same disclosure with better ergonomics, and the shape this has
is the shape a reader uses: they read a timeline, one entry looks wrong, they
expand that entry. The chain records **which** event was expanded — id, type,
sequence, payload digest — and never the payload, because an audit log that
copied every prompt somebody read would be a second, unbounded transcript
store with weaker access rules than the first.

### 4. A close carries a reason

`sessions.end_reason` (migration `0045`), set through `POST …/end`, nullable,
at most 500 characters, and forbidden on an `active` row by a CHECK.

It is not `task_summary`. That field is what the run was *about* and is set
at open; overloading it would make the two indistinguishable the first time a
client set both. Free text rather than a vocabulary, for `client_name`'s
reason (seed §2 principle 6): `hook timed out`, `context window exhausted`,
`user cancelled` belong to the harness, and a closed list here would be a
core change per harness. It **is** carried into the `session.ended` audit
payload, because "why did that run fail" is precisely what an auditor asks
and the status alone cannot answer.

### 5. The console gets a route per run, and routing gets one parameter

`/console/sessions/{id}` is a real, linkable, refreshable URL. CPR-8 wrote a
flat route table with literal segments; this adds `:name` parameters to it —
one level of pattern, still written in-repo, because a run somebody is
investigating has to survive a refresh and paste into a ticket, and that is
the whole of what a routing library would be installed for here.
`matchRoute` returns `{ id, params }` so a detail page reads the id it is
showing out of the address bar and from nowhere else — the alternative is two
sources for one fact that disagree after a Back.

### 6. The console offers the expansion from the **per-anchor** forecast

`/v1/me` reports capabilities at each anchor as well as tenant-wide. The
payload control is offered from the anchor matching the run's own scope when
there is one, and from the tenant-wide figure otherwise: a caller may hold
the plane in one project and not in another, and the tenant-wide figure would
render a control that 403s in half the places it appears. It remains a
**forecast, never a grant** (ADR-0058 decision 2) — the gateway decides again
on the click, and nothing is fetched until somebody asks.

## Consequences

- `SessionList.truncated` is gone. Any client reading it breaks loudly rather
  than silently paginating nothing. This is a pre-1.0 hard cut and CPR-10's
  console was the only reader.
- The packs move to `@20`, so every deployment's decisions are evaluated
  under a new embedded version. No existing permit changed.
- The listing's per-request cost is unchanged: the scan bound is still
  `synveda_store::sessions::SCAN_LIMIT` and the decision is still one Cedar
  evaluation per scanned candidate. What changed is that the scan starts from
  a cursor rather than always from the newest row.
- A page that is empty but carries a cursor is a shape clients must handle.
  It is documented on the schema and the console renders it honestly ("a page
  can be empty and still have more below it").
- **A later page can refuse where an earlier one served.** CPR-9's rule
  refuses a listing only when the caller is denied at the anchor *and* no row
  is readable; with paging, a caller denied at the anchor can meet a page
  whose candidates all belong to somebody else. The alternative — answer an
  empty page and a cursor — would hand a caller who holds nothing both the
  fact that rows exist and one row's key, which is the class of leak CPR-9's
  audit was about. The refusal is honest and the disclosure is not. The
  console never meets it, because it lists at the selected scope, where the
  gate is a scope the reader holds.
- `session.diagnostics` joins `Action::PROBED_AT_SCOPE`, so it appears in
  every capability forecast and in CNSL-2's explorer parity corpus.

## Alternatives considered

**Keep `truncated` and add `cursor` beside it.** Rejected: two ways to say
"there is more", one of which cannot be acted on. A field that exists only to
avoid breaking one in-repo consumer is a compatibility shim, which this
programme does not build.

**Offset pagination (`?page=3`).** Simpler to explain and wrong on this
table: runs are inserted continuously, so page 3 of a request made a minute
apart is a different set of rows, with repeats and gaps at every boundary.

**Fold payload reading into `SessionRead`.** Rejected: it makes the largest
disclosure on this plane free with the smallest, and no pack could then
express "follow what the agents did without reading everybody's prompts" —
which is the ordinary posture for a team.

**Classify lateness into `spooled` / `replayed` / `clock-skew`.** Rejected:
the server cannot distinguish them from two timestamps, and a label that is
right two thirds of the time is worse than a measured gap. If an adapter ever
*declares* how an event was delivered, that is a client-supplied field with
its own name, not an inference dressed up as one.

**A `?include=payloads` parameter on the timeline.** Rejected: it would make
the disclosure a property of a query string on a route decided under
`SessionRead`, so the authority would depend on a parameter rather than on
the action. One route, one action, one decision.

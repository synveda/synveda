# ADR-0084: Context planning selects immutable Knowledge revisions and exposes only re-authorised traces

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-20
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

The session plane is now the only adapter runtime and Knowledge is the only
published learned-content aggregate, but the path between them still reads the
retired `records` model. `POST /v1/sessions/{id}/context-runs` performs an
opaque record composition, stores one rendered block and exposes no durable
answer to which immutable revision was considered, selected or excluded and
why. Nothing writes new records after CPR-18, so retaining that reader would
make accepted Knowledge invisible to a clean session and would preserve the
last production dependency on the replaced model.

The same endpoint must remain the budgeted context-delivery seam. Evaluation
also needs an exact, non-budgeted way to query or enumerate the visible
Knowledge corpus, but restoring the deleted tenant-global `/v1/recall` route
would lose the session/project authority and make an enumeration sweep look
like ordinary agent context. Trace data itself is governed content: persisting
a denied candidate id, title, edge, count or exclusion reason would create the
side channel the PDP is meant to prevent.

## Decision

### 1. A context run is an immutable plan record, not only a rendered block

`session_context_runs` remains the stable `ContextRun` aggregate and gains the
session's project, as-of time, requested and actual budgets, retrieval,
embedding, index and graph versions, completion state, degradation mode,
rendered/query hashes and one governed trace-retention mode.

`context_candidates` records the visible, bounded retrieval pool and integer
score components. `context_selections` records the exact immutable Knowledge
revision chosen, its reason codes, rank and token cost. `context_feedback`
appends one typed outcome to one selection and revision; feedback never rewrites
the historical plan. All four tables are tenant-bound, forced-RLS and
append-only to the application role.

Retrieval, selection, context delivery and later outcome feedback are distinct
traced, metered and hash-chained transitions. The existing
`session.context.composed` event remains the delivery event so historical
consumers do not have to infer that a planner row was actually served.

### 2. Learned context comes only from current Knowledge revisions

The planner retrieves `knowledge_current`, hydrates the immutable current
revision and never queries `records`, `record_embeddings`, derived record
channels or a record-to-Knowledge bridge. Current active Knowledge valid at the
run's `as_of` instant is the selectable universe. Stale and superseded material
may appear only as an independently visible exclusion in a retained trace; it
is never silently selected as current.

Context packs and skill advertisements remain different authored assets and
continue through their existing `ContextPackRead` and `SkillRead` decisions.
Their rendering is integrated into the same budget rather than treating them
as learned Knowledge or dropping them during the cutover.

The core run row does not retain a second authored-object trace. Consequently,
a later diagnostic read masks the aggregate rendered block, its hash, token
count and advertised-skill list whenever a context pack or skill contributed.
The original delivery and its idempotent retry remain the session write; the
Knowledge candidate/selection trace stays independently inspectable. This is a
deliberately conservative disclosure boundary until authored artifacts have
their own immutable version/binding model rather than an invitation to infer
authority from a historical block.

Unreviewed capture candidates are not ordinary context. They may enter only
after a later governed configuration explicitly permits an unreviewed channel;
until then that channel is disabled and no implicit profile enables it.

### 3. Candidate generation is not authorisation

Lexical and configured semantic search generate a bounded candidate pool at
the session project, its governed ancestor chain and the caller's own principal
scope. Queryless composition uses bounded recency plus current preferences and
conventions. Every exact candidate then receives a fresh `KnowledgeRead`
decision before its id, revision, score, relation or reason is persisted or
rendered. Source evidence receives its own source-scope decision.

The persisted score vocabulary is integer-valued and separates keyword,
semantic, freshness, explicit pin, scope/current-state and final contributions.
The initial reason vocabulary is `semantic_match`, `keyword_match`,
`project_convention`, `personal_preference`, `freshness_boost`, `explicit_pin`,
`superseded`, `stale`, `outside_task_scope`, `token_budget` and `duplicate`.
The graph version is absent and graph contribution is zero until bounded graph
expansion is implemented; that absence is explicit rather than a graph claim.

A denied candidate contributes no persisted row, id, title, relationship,
reason or count. A run may retain one boolean/message saying that policy
filtering occurred. Detail reads re-authorise retained Knowledge references;
revocation removes the whole detail and still exposes no count.

### 4. Retention controls diagnostic trace detail, never the delivery record

The idempotent `POST` resource is also the delivery record: it retains the
query and exact rendered response needed to replay the same run after a lost
acknowledgement. That core row is not a diagnostic trace and is retained under
every mode. Subsequent list/detail reads never expose it in bulk and apply the
mode plus fresh session and exact-revision decisions before disclosure.

The effective composition configuration carries one of four diagnostic modes:

- `full` retains visible candidate/selection references and score detail, and
  an authorised detail read may disclose the core query and rendered block;
- `redacted` retains visible candidate/selection references and reason detail,
  but a trace read never discloses the core query, rendered block or hydrated
  revision content;
- `hashes_only` retains candidate/selection hashes and reason data without
  Knowledge ids and never discloses the core plaintext through trace reads;
- `disabled` creates no candidate or selection trace rows and exposes only the
  minimal immutable completion, budget, version, degradation and masked
  delivery envelope through trace reads.

The mode is selected by governed policy/configuration and cannot be widened by
a request. Immutable Knowledge content is not copied into trace rows; a full or
redacted trace hydrates it from its revision only after a fresh decision.

### 5. Runtime query and evaluation are session-scoped application reads

The public surface retains
`POST /v1/sessions/{session_id}/context-runs` and adds cursor-paginated
`GET /v1/context-runs`, `GET /v1/context-runs/{id}` and idempotent
`POST /v1/context-runs/{id}/feedback`.

`POST /v1/sessions/{session_id}/knowledge-query` is the ordinary deep-query
surface. It derives tenant, project, scope and actor from the authenticated
session, requires `SessionRead`, and returns bounded current Knowledge only
after exact `KnowledgeRead` decisions. It is not constrained by a context token
budget and therefore does not masquerade as a context run.

`POST /v1/sessions/{session_id}/knowledge-evaluation` is the separately
authorised enumeration/query/id lens. It requires the existing, stricter
`SessionDiagnostics` action and still performs exact `KnowledgeRead` per item.
It uses an opaque candidate cursor that advances over denied rows, so a caller
cannot infer their number. No tenant-global recall route or direct-store client
exists. Ownership is resolved before every session or context decision, so an
unknown or cross-tenant address is 404 rather than an authorisation oracle.

### 6. Feedback names exactly what it judges

Feedback values are `referenced_by_agent`, `accepted_by_user`, `helpful`,
`unhelpful` and `caused_correction`. A request names an exact selection and
immutable Knowledge revision under one context run. The database constrains
that relationship, the gateway re-authorises the session and revision, and an
idempotency key makes retries converge. Merely retrieving or selecting a
revision creates no positive feedback.

## Options considered

1. **Knowledge-backed immutable plans plus scoped query lenses (chosen).** It
   gives runtime composition, diagnostics and evaluation distinct semantics
   while sharing one current Knowledge/PDP retrieval seam.
2. **Translate Knowledge into records and keep the composer.** This is a dual
   write and preserves two current models. Rejected.
3. **Use context runs for evaluation sweeps.** A budgeted selection cannot
   measure whether a corpus contains every expected item. Rejected.
4. **Restore `/v1/recall` with mode flags.** A tenant-global route cannot
   derive the governed run/project and would make ordinary recall and privileged
   enumeration one ambiguous authority. Rejected.
5. **Persist all candidates and filter traces on read.** The database would
   retain a denied candidate graph that a worker, export or bug could disclose.
   Rejected.

## Consequences

- Accepted Knowledge becomes visible to the next clean session without any
  record translation; new Knowledge and new context can no longer drift apart.
- Every idempotent delivery can be replayed exactly from its core run record.
  Redacted, hashes-only and disabled modes prevent later diagnostic trace
  disclosure and progressively reduce auxiliary candidate/selection storage;
  they are not a promise that the delivered session payload never existed on
  the server.
- Evaluation clients must hold session diagnostics authority and an actual
  session address. They can enumerate visible Knowledge without changing the
  product's ordinary runtime authority.
- Existing record-oriented composition tests and recall request tombstones are
  deleted or re-cut. Context-pack and skill tests remain and prove those
  authored assets survived the learned-content cutover.
- Bounded graph expansion, richer conflict resolution and governed
  configuration artifacts remain later packages. Their version/configuration
  seams are recorded now without pretending they already exist.

## Compliance notes

- **Tenancy/RLS:** every new trace/feedback table is tenant-bound, enabled and
  forced RLS; tenant-qualified foreign keys bind runs, sessions, items and
  revisions.
- **PDP:** session ownership precedes `SessionRead`, `SessionWrite` or
  `SessionDiagnostics`; every retained/exposed Knowledge reference and source
  is independently decided. Denied candidates leave no object-shaped trace.
- **VedaFlow:** this is a read and feedback package. It creates no Knowledge
  mutation and cannot publish a capture candidate.
- **Audit:** retrieval, selection, delivery and feedback carry ids, hashes,
  counts, versions and decisions only, never query, rendered context, title,
  body, source locator or event payload.

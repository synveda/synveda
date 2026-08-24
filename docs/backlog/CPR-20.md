---
title: "CPR-20: Explainable Knowledge context planning and scoped query"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-20: Explainable Knowledge context planning and scoped query

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Replace the last record-backed runtime reader with an explainable planner over
current immutable Knowledge revisions. A context run retains the visible
candidate pool, exact selections, reason and score components, versions,
budgets, degradation, rendering hash and governed trace-retention mode. It
keeps context packs and skill advertisements as separately authorised authored
inputs and never translates them or Knowledge through records.

Expose re-authorised run inspection and feedback, a session-scoped deep query
for ordinary application recall, and a separately authorised evaluation lens
that can query, enumerate or fetch exact visible Knowledge without abusing a
budgeted context run or restoring `/v1/recall`.

## Acceptance criteria

1. `ContextRun`, `ContextCandidate`, `ContextSelection` and `ContextFeedback`
   are tenant-bound, forced-RLS and immutable where they describe history. A
   run records session, project, principal, task/query hash, as-of time,
   requested/actual budget, retrieval/embedding/index/graph versions,
   degradation, rendered hash, retention mode and completion state.
2. The planner selects only current active Knowledge revisions valid at the
   run's as-of instant. Accepted Knowledge reaches a clean session; stale and
   superseded revisions are not selected as current; no production context
   query reads `records` or `record_embeddings`.
3. Candidate rows retain integer lexical, semantic, freshness, pin,
   current-state and final contributions plus the eleven initial reason codes.
   Token-budget and visible lifecycle exclusions are explainable. Graph absence
   is explicit until bounded graph expansion exists.
4. Every candidate and source is independently decided before persistence and
   disclosure. Denied Knowledge leaks no id, title, edge, reason or count; at
   most a single aggregate policy-exclusion message is exposed. Detail reads
   re-authorise references after grants or policy change.
5. `full`, `redacted`, `hashes_only` and `disabled` retention modes have tested
   storage and response semantics. No mode copies immutable Knowledge content
   into trace rows or weakens the enforcement/audit envelope.
6. The existing context-run POST remains stable. Generated public operations
   add cursor-paginated run list/detail, idempotent revision-specific feedback,
   ordinary session-scoped Knowledge query, and a `SessionDiagnostics`-gated
   evaluation query/enumeration/id lens. No route accepts tenant, principal,
   project or scope from the body and no global recall route returns.
7. Feedback distinguishes `referenced_by_agent`, `accepted_by_user`, `helpful`,
   `unhelpful` and `caused_correction` and is bound by database constraints to
   one exact run selection and immutable revision. Selection alone creates no
   positive feedback.
8. Retrieval, selection, delivery and feedback are distinct spans, metrics and
   content-free hash-chain events. Knowledge usage is populated from visible,
   re-authorised selections and enforces both session and Knowledge access.
9. Context packs and skill advertisements still compose under their existing
   separate PDP actions and the same total token budget. Unreviewed capture
   candidates do not enter context because no governed profile yet permits
   that channel.
10. Temporary `RecallSweepRequest`/`RecallIdsRequest` refusal tombstones and
    the runtime record composer are deleted only after the new query lenses
    replace their purpose. Focused gateway/store/policy/OpenAPI/RLS tests, a
    runnable demo, `make ci` and `make db-test` pass.

## Decision

[ADR-0084](../adr/adr-0084-explainable-knowledge-context-planning.md) fixes the
Knowledge-only learned-context boundary, trace-retention semantics and the
separate ordinary/evaluation authority before implementation.

## Completion evidence

Delivered 2026-08-24 from
`e90dac9c9f36e747c380b377f524dd383b7603ce` under accepted ADR-0084.

Migration `0051_context_planning` extends the stable session context-run row
and adds tenant-bound, forced-RLS candidate, selection and feedback history.
Application roles cannot rewrite trace history. The migration contains no data
translation: pre-cut opaque rows have no planner marker and every application
query excludes them; native rows satisfy an all-or-nothing planner-shape
constraint. The chain remains schema epoch 2 with 49 migrations.

The planner reads current immutable Knowledge revisions only. Lexical and
configured semantic retrieval create a bounded pool, each exact item and source
is independently decided before persistence or rendering, and selection records
integer score components, reason codes, rank and token cost. Stale and
superseded revisions remain visible exclusions only when authorised. Denied
material creates no address, title, relation, reason or count; a later detail
read repeats exact decisions. Full, redacted, hashes-only and disabled trace
modes are pinned. Because the old aggregate run row cannot name exact authored
pack/skill versions, a later trace read conservatively masks the whole rendered
block whenever either authored input contributed.

The generated contract grows from 62 to 67 operations: the existing context
delivery POST now returns the explainable plan; list/detail and idempotent exact-
revision feedback are added; ordinary deep query is session-scoped; and exact
query/enumeration/id evaluation is a separate `SessionDiagnostics` lens. CLI,
the generic MCP server and the evaluation client use those public APIs. The
runtime `records` composer, `index_tier` gateway suite and temporary
`RecallSweepRequest`/`RecallIdsRequest` refusal tombstones are deleted. Audit
query now resolves immutable Knowledge selections rather than record entries.

Focused evidence: public context acceptance **3/3**, audit query **13/13**,
context-pack re-authorisation **10/10**, sessions **22/22**, skills **24/24**,
OpenAPI **5/5**, console **165/165**, CLI recall **2/2**, CLI MCP **14/14**,
RLS trace immutability/completeness **2/2**, and the broader type, retrieval,
audit and evaluation suites pass. `make ci` and the full `make db-test` pass.
`demos/cpr-20-context-planning.sh` passes on an isolated fresh database and
reports **55 Knowledge items, 47 plans, 75 immutable selections, 2 feedback
rows and zero old records**. No external-client or live-model claim is part of
this package; graph expansion and benchmark remeasurement remain their filed
later objectives.

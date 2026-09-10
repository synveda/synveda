# ADR-0083: Capture freezes session evidence and publishes only through Knowledge changes

- **Status**: Accepted
- **Date**: 2026-08-24
- **Feature(s)**: CPR-18, CPR-45
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

The session ledger is the only adapter write plane and Knowledge is the only
public content aggregate. The remaining extractor still embodies the model
between them that both replace: each eligible event emits a PGMQ signal; a
worker classifies one event, embeds it, writes an active `record`, mutates
dedup/supersession state and appends it to a derived memory channel. CPR-16
stopped that worker, so current sessions truthfully produce no durable
learning. Restarting it would reintroduce the bypass the redesign exists to
remove.

Extraction is not publication. Model-selected material needs a durable,
reviewable address, exact evidence and a decision outcome before it can become
Knowledge. It must also survive request loss and gateway restart without
calling a model twice for the same event snapshot or creating two VedaFlow
changes when an acceptance response is lost.

## Decision

### 1. A capture batch freezes an eligible session-event snapshot

`capture_batches` is a durable extraction job keyed by the session and a
canonical BLAKE3 digest of the exact eligible event ids, payload hashes and
event types it covers. `capture_batch_events` freezes that ordered input. An
explicit request and a terminal session transition both ask for a batch; the
same snapshot replays the existing row, while later events or a newly released
quarantined event produce a different digest and therefore a new batch.

The batch has `pending`, `running`, `completed` and `failed` states, bounded
attempts and a lease. A database-polling worker claims rows per tenant under
forced RLS. The batch itself is the durable job address; it is not put into
`durable_operations`, whose current shape is deliberately bound to a
VedaFlow proposal and work already authorised for application. Extraction has
not proposed a Knowledge mutation yet.

CPR-45 tightens that lease before extraction moves into a separately
restartable process. The claim identity is the exact tuple of tenant, batch,
process-unique owner and incremented `attempts` value; the attempt counter is
the fencing token. Claim, renewal, completion and failure compare PostgreSQL
statement time, not transaction-start time. The worker renews independently
while an external extractor is in flight and discards the result as soon as it
cannot prove renewal. Because the claim clock starts before configuration and
PDP preflight finishes, the worker first commits a renewal after preflight and
before disclosing any event to a provider. Renewal shutdown is
cancellation-aware and bounded. Completion performs the fenced terminal
transition before inserting any candidate or evidence row, so even a caller
that catches the conflict and commits cannot retain stale output. A crashed,
expired final attempt is terminalised with the stable `lease_expired` code and
content-free audit evidence before the tenant receives more work. These rules
reuse the existing counter and require no schema-era or data compatibility
path.

### 2. The extractor returns proposed Knowledge, never storage instructions

The existing deterministic, Claude and OpenAI-compatible extractor boundary is
retained, but its output vocabulary becomes the nine `KnowledgeType` values
and complete bounded candidate content: title, Markdown body, summary,
confidence, sensitivity proposal, tags and entity metadata. Output is
rescanned for secrets and validated with the same Knowledge content validators
the command layer uses.

The worker writes only `capture_candidates`, their exact source-event links
and visible match rows. It does not write Knowledge, records, vectors, graph
edges or VedaFlow channels. The `session_events` PGMQ queue and the old
record/embed/dedup/link/derived-channel commit worker are deleted rather than
left callable but unstarted.

### 3. Capture follows session authority; publication is separately decided

A batch is about one exact session. Creating or processing it takes
`SessionWrite` on that `Session` entity; reading a batch or candidate takes
`SessionRead` on the same entity. This deliberately adds no parallel
`CaptureRead` tree: a candidate discloses a derived part of a transcript, so a
caller who cannot read that transcript cannot learn that its candidate, match
or count exists.

The worker re-decides current session authority as the principal that opened
the run before persisting output. Candidate acceptance then takes the ordinary
Knowledge command decisions at every proposed/input scope. Session authority
never implies publication authority, and a successful extraction is not a
cached authorisation for a later acceptance.

### 4. Matches are persisted only after exact Knowledge decisions

For each validated proposal the worker retrieves a bounded lexical candidate
set from current active Knowledge and hydrates immutable current revisions.
It independently decides `KnowledgeRead` for every exact item and sensitivity
as the session principal before comparison or persistence. A denied item
contributes no id, match row, count or reason.

The initial deterministic comparison records `duplicate`, `conflict` or
`possible_supersession` with integer similarity and a stable reason code.
These are review hints, not current-state mutations. Ambiguous candidates stay
pending; no contradiction closes a Knowledge window and no duplicate is
silently merged during extraction. The later conflict/freshness package may
replace the classifier behind this stored vocabulary without changing the
publication boundary.

Candidate plaintext has a second read decision at its proposed Knowledge
destination in addition to `SessionRead`. This prevents a personal preference
derived inside a team session from becoming a team-readable draft. Preferences
default to the session principal's own principal scope; other types default to
the session's project/workspace scope. A caller may explicitly change
placement only through the decision request, and an explicit JSON `null` can
remove a project or owner association rather than inheriting it accidentally.

A match is also re-authorised at disclosure time. Persist-time visibility is
not cached authority: grants, packs and lifecycle can change between extraction
and review. A now-denied match disappears whole, including its item id,
revision id, relation, reason and contribution to visible counts.

### 5. A candidate decision is durable before a Knowledge command runs

`capture_candidate_decisions` is an append-only intent/result row, unique per
candidate. It binds action, canonical request hash, caller idempotency key and
actor before invoking the Knowledge command service. A crash before the
command leaves a resumable intent; a crash after the command replays that
command through the existing idempotency ledger and then finalises the same
candidate. A second key or changed payload conflicts rather than opening a
second change.

Accept and edit-and-accept create Knowledge, merge calls Knowledge merge, and
replace calls Knowledge supersede. All therefore create CPR-16's typed
VedaFlow proposal even when policy auto-applies it. Dismiss is a governed
candidate-state decision and opens no content change. Whole-batch acceptance
uses deterministic per-candidate child keys and can resume without duplicating
an already opened change. It records its parent key only after every child
finishes, so a mid-batch failure resumes child-by-child; the parent key then
prevents reuse against another batch.

Request validation happens before the one durable decision slot is occupied.
Once valid intent exists, concurrent/restarted callers converge on its one
Knowledge idempotency record. Only the transaction that wins the
`running -> succeeded` transition emits the candidate-decision audit event.

The candidate's terminal state records `accepted`, `edited_and_accepted`,
`merged`, `replaced`, `dismissed` or `failed`, plus the VedaFlow outcome and
result ids where applicable. `pending_review` describes the VedaFlow outcome;
it never misrepresents the candidate as active Knowledge.

### 6. Evidence is immutable and content stays out of audit

Candidate source links must name events frozen into the same batch and session;
composite foreign keys make cross-session and cross-tenant provenance
unrepresentable. Candidate content remains in the candidate aggregate for the
review experience, under forced RLS and per-row PDP reads.

The chain records batch creation/completion/failure and candidate decisions
with session, batch, candidate, event, match and change ids, counts, hashes,
method/model and policy context. It never copies event payloads, candidate
titles/bodies/summaries or Knowledge content. Traces and metrics follow the
same content-free rule.

Governed Knowledge erasure also scrubs candidate title/body/summary, extension
metadata and the decision request payload for the resulting aggregate. Stable
candidate, decision, change and content hashes remain as a content-free
tombstone; source payload ownership follows CPR-16's erasure operation.

## Options considered

1. **Frozen batches and reviewable candidates (chosen).** Gives retries,
   evidence and review one durable address without confusing extraction with
   publication.
2. **Restart the record worker and translate records later.** Reintroduces an
   active-content bypass and creates the forbidden bridge between aggregates.
   Rejected.
3. **Write Knowledge directly, then let review archive it.** Unreviewed model
   output would be current during the interval and would never have passed
   VedaFlow. Rejected.
4. **Use a context run as the extraction job.** Context composition is a
   budgeted read with different inputs, authority and evidence. Rejected.
5. **Persist every lexical neighbour then hide denied ones at read time.** The
   match table itself would become a policy side channel and a later worker
   could consume it without the hiding layer. Rejected.

## Consequences

- Appending an event no longer queues extraction. Session end or the explicit
  capture route freezes a batch, and the worker may finish after the request.
- Releasing a quarantined event from a closed session may create a new batch
  because the eligible event digest changed; the immutable event is never
  rewritten.
- Candidate extraction can succeed while every acceptance remains pending
  review. That is an ordinary policy outcome, not a failed batch.
- The deterministic comparator is intentionally conservative. It proposes
  likely relations and mutates no current state; Prompt 27 owns deeper
  conflict and freshness resolution.
- The record-backed context composer remains a separately bounded old read
  seam until external Prompt 18 (expected repository feature CPR-20) re-cuts
  retrieval to Knowledge. CPR-18 removes the direct record producer, not that
  later read package.

## Compliance notes

- **Tenancy/RLS:** every capture table is tenant-bound, enabled and forced RLS
  and enters both explicit and dynamic completeness gates.
- **PDP:** session ownership resolves before the exact Session decision; match
  details are decided per Knowledge item before persistence or disclosure;
  acceptance re-runs Knowledge decisions through CPR-16.
- **VedaFlow:** extraction writes no Knowledge. Every accepted create, merge
  or replacement is a typed Knowledge/apply change in the one review engine.
- **Audit:** semantic transitions are hash-chained with ids, hashes and counts,
  never transcript or candidate content.

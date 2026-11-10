# ADR-0163: Progressive context disclosure — a manifest is a ContextRun, an expansion is a re-authorised delivery, and every refusal looks the same

- **Status**: Accepted. Implementation in progress
- **Date**: 2026-11-09
- **Feature(s)**: Progressive context disclosure
- **Deciders**: sujitn

## Context

A session-start ContextRun renders Knowledge bodies until its budget runs out.
`knowledge-query` returns full bodies with no budget, and fetching an exact id
is available only through the `SessionDiagnostics` evaluation lens. Nothing
records which assets an agent pulled after being offered them.

ADR-0041 built an index tier and ids-based recall. ADR-0078 deleted that route,
and ADR-0042's superseded note says `ids` and `as_of` have no successor and that
the ADR "is the specification of what has to come back". This decision is that
return, re-cut over ContextRun and immutable Knowledge.

The feature request was written against vocabulary the product has since
replaced (`inject`, `recall` routes, `observe`, Knowledge channels) and asks for
some outcomes that conflict with accepted decisions:

- Per-item `denied` and `not_found_at_as_of` statuses would distinguish outcomes
  that ADR-0041 decision 6, ADR-0084 decision 5 and ADR-0141 keep uniform. They
  also conflict with the request's own requirement that an unauthorised
  principal sees no trace of an asset, including in error messages.
- Resolving an id "against the manifest instant" would find a retracted asset,
  because it existed at that instant. The request also requires that it not be
  returned.
- Generated descriptions and summaries would add a model call to a read path
  that makes none, or an unreviewed derivative beside an immutable revision.
- Knowledge is not a channel member (technical plan section 2.2), and an
  unreviewed channel opens only through governed Configuration (ADR-0084
  decision 2).

## Decision

1. **A manifest is a ContextRun.** `POST /v1/sessions/{id}/context-runs` accepts
   `disclosure` of `full` or `manifest`. `ContextConfiguration` gains
   `disclosure_mode`, defaulting to `full` in every built-in template. A request
   may narrow `full` to `manifest` and never the reverse. The manifest id is the
   run id and its instant is the run's `as_of`. The existing governed
   `token_budget` and requested `budget_tokens` are the manifest budget; there
   is no second budget (ADR-0041 option 4). Candidate generation, per-candidate
   `KnowledgeRead` and first-fit selection are unchanged, and selections record
   `EntryTier::Index`.
2. **An entry is stored revision fields rendered shallow.** It carries the item
   and revision ids, Knowledge type, title and summary cut to fixed character bounds at a
   character boundary, channel label, sensitivity marker, and the body's cost
   under the run's counter, labelled as an estimate unless the encoding is exact.
   There is no model call and no clock read. A denied candidate leaves no entry,
   count or marker. `truncated` and the pointer to recall are computed only from
   authorised candidates, and the manifest exposes no policy-filtering flag.
   Caller-required revisions and restart evidence remain verbatim because the
   caller or the compaction contract asked for them. Authored pack bodies stay in
   the shared allowance and are not expandable; bringing them into the manifest
   needs its own decision. `manifest` ignores `optimization_mode` because an entry
   has no body to extract from, so `conservative` still applies to `full` runs,
   and an entry's cost is always the whole body's count.
3. **Expansion is a delivery.** `POST /v1/sessions/{session_id}/knowledge-expand`
   takes 1–32 unique `ids`, an optional `manifest_id` and `resolve`. It requires
   `SessionWrite` because it appends a Session event (decision 9), then a fresh
   `KnowledgeRead` per item and independently authorised sources. A full-depth
   recall appends nothing and so stays under `SessionRead`. Items return in
   request order. The response
   is capped at 512 KiB, filled first-fit in request order. The evaluation lens
   stays diagnostic. Knowledge has no attached files, so linked resources mean
   provenance, which is always returned, and there is no `include_resources`.
4. **There is one refusal.** Each item is `served`, `not_found_at_as_of` or
   `over_budget`. A missing, foreign-tenant, denied, above-tier, withdrawn,
   archived, erased, superseded or changed id gets the identical
   `not_found_at_as_of`, with no reason field. `over_budget` is emitted only
   after a fresh allow, so it discloses nothing new. An unknown manifest, or one
   belonging to another tenant or Session, is the same 404 as an unknown Session.
5. **A pin never resurrects content.** `resolve` is `pinned`, the default when a
   manifest is named, or `current`, the only mode without one. Pinned serves an
   item only when the revision the existing as-known read returns for the run's
   recorded creation instant is still the current head and
   that head is active and valid now. This is ADR-0141's saved-delivery rule. No
   historical bytes are
   served, so withdrawal, Forget and supersession always win. `current` serves
   the head after the same checks. Tool guidance tells agents to pass
   `manifest_id` only for ids taken from that manifest.
6. **Recall gains depth and stays current.** `knowledge-query` accepts `depth` of
   `summary` (default) or `full`. `summary` returns stored title, summary, type,
   sensitivity, revision address and authorised provenance. `full` is today's
   shape under the 512 KiB cap with an explicit `truncated`. There is no
   compatibility path for the changed default. Recall answers against current
   heads with its own `as_of`; it does not rewind to the manifest instant.
7. **Redaction happens once, at admission.** Every tier renders admitted revision
   text, so no read-time redactor or `redacted` status exists. Knowledge admission
   previously refused only `Secret` findings and stored PII. Every Knowledge
   command now applies the effective policy pack's redaction ladder (ADR-0021,
   ADR-0050), as packs and Session events do, across title, summary, body, tags,
   verification and revision metadata, and source locators, revisions and
   metadata, because provenance is returned at L1 and L2: `redact` scrubs before
   validation and hashing, and `quarantine` or `deny` refuses with rule
   identifiers only. Knowledge has always refused credentials, and the `standard`
   and `open-collaboration` packs set secrets to `redact`, so for Knowledge a
   pack's `redact` is raised to `quarantine` for `Secret` findings: a credential
   is refused on every pack and never scrubbed into a stored revision. Strings in metadata and source references that are wholly a
   UUID or RFC 3339 timestamp are not scanned, because the scanner misreads their
   digit groups as payment cards or phone numbers (the rule the Procedure and task
   state scanners already apply). A title and summary belong to one revision and
   one VedaFlow change, so they inherit its review and need no separate promotion
   or regeneration.
8. **Channels keep their existing meaning.** The universe is the existing
   `ConfigurationContextChannel` selection, and each entry carries that label. A
   request cannot widen it. Unreviewed candidates keep their unreviewed rendering
   and, having no revision to pin, are not expandable.
9. **Expansion evidence is a server-derived Session event.** Each expansion with
   a served item appends one immutable `context.knowledge_expanded` event naming
   the manifest, exact served items, revisions, hashes and tokens, never content.
   Its identity derives from the run and sorted revisions, so retries converge.
   Timeline reads apply the trace-retention mask. Compaction needs no new hook or
   payload, because the Session ledger already holds what was expanded. A
   `restart` run reads recorded expansions newest first, bounded, re-reads at
   most 32 distinct expanded items, applies decisions
   3–5 under fresh Cedar and renders survivors verbatim inside the shared
   allowance.
   Feedback is not used: ADR-0084 decision 6 says retrieval creates no positive
   feedback, and `hashes_only` runs have no addressable selection. A full-depth
   recall is a query, so it appends no event and is outside the offered-versus-
   expanded ratio; its audit event records the depth, and the evaluation harness
   reports the full-depth recalls it issued beside that ratio.
10. **Audit is content-free and per act.** `session.context.composed` carries
    entry count, tier and addresses subject to retention. Expansion and recall
    record principal, tenant, depth or resolve mode, served addresses per
    retention, status counts and tokens returned. Refused ids are never listed
    (ADR-0041 decision 8).
11. **One new MCP tool.** `synveda mcp` advertises `expand` beside `recall` in
    both `--writes` modes, and `recall` gains `depth`. Expansion is a scoped
    Knowledge read by id, so it stays within ADR-0057's primitives. The block's
    legend describes the expansion marker without containing it (ADR-0041
    decision 12).
12. **Ranking is unchanged.** Expansion-derived inputs are deferred. Per-principal
    expansion frequency would be a cross-session behavioural profile; if adopted
    later it requires an explicit tenant Configuration opt-in.

## Options considered

1. **Extend ContextRun and add one expansion route (chosen).** It reuses the
   planner, Cedar seam, delivery record, event ledger and retention contract.
2. **A new manifest aggregate and session-start route.** It creates a second plan
   store and a second delivery surface to authorise and retain.
3. **Distinct `denied` and retraction statuses, as requested.** Agent ergonomics
   improve, but the status becomes an existence and access oracle.
4. **A distinct `changed` status after a fresh allow.** It leaks nothing but adds
   wire surface to save one retry in a rare case. Revisit if pinned misses matter.
5. **Serve the bytes known at the manifest instant.** This is literal snapshot
   consistency, and it would serve withdrawn or forgotten content.
6. **Model-written descriptions.** Rejected on the read path, and at write time
   they would be unreviewed text beside a reviewed revision.
7. **A read-time redactor on every tier.** It adds hot-path CPU and a second
   scrubbing outcome for one immutable revision.
8. **`referenced_by_agent` feedback or adapter-tracked expansions.** Feedback
   contradicts ADR-0084, and hooks cannot observe `synveda mcp` tool calls.

## Consequences

- Positive: session start can name more than it could carry, each tier is
  independently authorised, and the product gains a real offered-versus-pulled
  signal without new client state.
- Negative / accepted: `recall` defaults to summaries, which changes existing
  callers and evaluations. Manual Knowledge containing PII will be scrubbed or
  refused according to the effective policy pack. A pinned expansion misses after any edit
  until retried with `current`. An agent cannot tell why an id was refused.
  Response uniformity makes no timing claim. Revisions admitted before this
  change are not rescanned. Pack bodies remain in the shared allowance until
  authored assets become expandable.
- Reversal triggers: measured task quality falls against `full` on the same
  fixtures; pinned misses are a material share of expansions; authored packs
  dominate manifest budgets.

## Compliance notes

Cedar decides every tier afresh and no tier trusts an earlier one; ids are names,
never capabilities (ADR-0041 decision 5). All reads run in forced-RLS tenant
transactions. The new Session event type needs one additive migration and pinned
head; the feature adds no table. Knowledge admission is a protected-path change
inside the existing typed VedaFlow command. Configuration can narrow disclosure
and cannot bypass the PDP, RLS or audit. Audit and telemetry carry identifiers,
hashes, counts and closed vocabularies only.

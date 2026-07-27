# ADR-0041: Tiered injection — the index tier is the permitted set rendered shallow, a demotion happens only when it saves, and a handle is a name rather than a capability

- **Status**: Accepted
- **Date**: 2026-07-27
- **Feature(s)**: CTX-4
- **Deciders**: sujitn

## Context

CTX-4's text is "inject carries a compact index of available deeper
assets (names+descriptions, skills-style ~80 tokens each); bodies
fetched via recall", and its acceptance criterion is "token cost of index
tier measured; agent can navigate index→body in a live Claude Code
session".

Three accepted ADRs park obligations here. ADR-0025's reversal triggers
name "CTX-4's index tier or watermark overhead measured material → short-id
watermark scheme". ADR-0026 option 7 refused a response cache and said
"CTX-4/CTX-6 own read-path shaping if EVAL-6 ever shows compose itself
binding". ADR-0027 rejected injecting at `UserPromptSubmit` because "the
designed answer to per-prompt relevance is CTX-4's tiered index/body
split, not more full injects", and recorded "CTX-4's tiering lands →
revisit option 6" as the trigger for reconsidering it.

Forces at play:

- **Today's block drops what does not fit and never says so.** ADR-0025
  decision 4's first-fit assembly skips an entry that exceeds the
  remaining budget and continues; the count reaches the audit event as
  `skipped_over_budget` and reaches the caller as nothing at all. The
  defect progressive disclosure actually fixes is not token efficiency —
  it is silence. An agent that does not know a runbook exists cannot ask
  for it, and a thin block is indistinguishable from an empty corpus.
- **The "~80 tokens each" figure comes from skills, and skills do not
  exist yet.** A skill's index entry is an authored name and description
  standing in for a body of thousands of tokens; that ratio is what makes
  tiering pay. A memory record is *summarised at write time* (seed §4.2,
  MEM-3), so the median record's body is already well under 80 tokens.
  Applied naively to today's corpus, an index tier costs more than the
  material it replaces. `AssetKind` has five variants and exactly one is
  populated — `Prompt` and `ContextPack` are PRMT-1/2, `Skill` is SKIL-1
  — so the mechanism has to be general over the asset kinds that will
  arrive without making the product worse for the one that is here.
- **An index entry is a disclosure.** A name and a description are
  content. AUTHZ-5's leak suite asks a corpus back forty-odd ways to
  prove that `restricted` material never reaches a reader who did not
  earn the tier; a line naming that record, its class and its subject
  defeats that suite as thoroughly as its body would. Seed §2.2: there is
  no code path from harness to storage that bypasses the PDP.
- **A record id in a block is not authority.** Every freshness promise
  this codebase has made — a pack flip (ADR-0014), a role revocation
  (ADR-0015), a hierarchy move (ADR-0016), a lapse's window closing in
  the query that asks (ADR-0037), a retention schedule governing the very
  next inject (ADR-0040) — says the *next* request is decided by current
  state. A handle exchangeable for a body without a fresh decision would
  be the first thing in the product that outlives the decision that
  minted it.
- **The read path makes no model calls and reads no clock.** ADR-0024
  decision 7 made the first structural for the crate; CTX-2's acceptance
  criterion is byte-identical re-composition at the same instant.
  Whatever an index line says must be computed from stored bytes under
  the caller's explicit instant.
- **`recall` is a primitive with a first-class audit obligation.** Seed
  §3 has listed it beside `inject` and `observe` since day one, and seed
  §2.2 principle 5 names "recall" among the acts the tamper-evident log
  records. CTX-5 owns its full shape — hybrid retrieval plus graph
  traversal plus as-of, results labelled with channel, provenance and
  validity, exposed as one MCP tool. CTX-4 needs only the floor its
  handles point at.
- **A second budget would be a second config plane.** ADR-0025 decision 3
  rejected a per-node settings table because pack assignment already has
  versioning, audit events, hot reload, display routes and break-glass.
  Anything CTX-4 needs to configure rides the pack beside
  `budget_tokens` or it does not exist.
- **Composition drops things on purpose.** ADR-0025 decision 6 drops the
  loser of a conflict group entirely; MEM-5 closes a superseded fact's
  window; MEM-6 cuts expired material at the read path and exempts
  pinned. An index tier that named what composition deliberately removed
  would re-introduce, at lower fidelity, exactly what those features
  exist to remove.

## Decision

`compose` gains a second rendering per candidate. Assembly is otherwise
unchanged — gradient order, pinned-first, conflict-resolved, first-fit
under one budget — except that a candidate whose body does not fit the
remaining budget is offered its **index line** rather than being dropped,
and takes it only when that line is genuinely cheaper. The gateway gains
`POST /v1/recall`, which re-runs the composition plan and serves bodies
for exactly the ids the *current* plan admits.

Decisions, specifically:

1. **The index tier is the same permitted set rendered shallow. There is
   no new Cedar action.** Index candidates are the candidates `compose`
   already fetched under the plan: the same per-scope `MemoryRead`
   decisions, the same per-scope tiers (ADR-0038 decision 3), the same
   channel rules including bank mode (ADR-0031), the same retention cut
   and pinned exemption (ADR-0040 decisions 2 and 8), the same lapse
   sections (ADR-0037 decision 12). There is no "may list but not read"
   verdict, because a name and a description are content and a second,
   weaker decision would be a second leak surface across every pack ×
   role × scope × tier cell the AUTHZ-5 suite covers. The index makes a
   block say more about material it already holds the right to show; it
   never widens what that is.
2. **A demotion happens only when it saves.** Both renderings are
   estimated with the ADR-0025 decision 4 estimator. The index line is
   tried only when the body does not fit the remaining budget, and taken
   only when its estimate is strictly smaller than the body's. A short
   record — the median memory record today — is therefore never demoted,
   because demoting it would spend budget to say less. This one rule is
   what lets a mechanism built for assets that do not exist yet ship
   against a corpus made entirely of assets that do, and it is why the
   AC's measurement is a measurement rather than a formality.
3. **The index line is the body truncated at a character boundary, and
   the handle is the record id.**
   `- [class] <content truncated to index_entry_chars>… [markers] (recall <id>)`
   — the same shape a body line has, through the same renderer, so a body
   and its index form can never disagree about a trust marker:
   `[unreviewed]` for derived material,
   `[confidential]`/`[restricted]` above the working tier. No new column,
   no model call, no clock. `index_entry_chars` defaults to 320, which is
   the feature's "~80 tokens each" through the chars/4 estimator. The
   trailing ellipsis and the handle *are* the marker: one piece of text
   saying both "this is not the whole thing" and "here is how to get the
   whole thing", spending no budget on a separate label. Decision 2 makes
   this consistent — an entry short enough not to truncate is never
   demoted, so every index line in a block is genuinely elided.
4. **A truncated body is a poor description, and that is recorded rather
   than papered over.** Memory records have no name and no description
   (seed §4.2 lists neither), and inventing one on the read path would
   mean a model call the crate structurally cannot make. When PRMT-1,
   PRMT-2 and SKIL-1 land, their assets carry an authored name and
   description, and the index slot renders those instead — which is the
   whole reason this is a per-`AssetKind` rendering seam rather than a
   memory-record special case.
5. **A handle is a name, not a capability.** `POST /v1/recall` takes ids
   and re-runs `composition_plan` exactly as inject does: identity from
   the token, chain from the HIER-2 cache, one PDP `MemoryRead` walk, the
   per-scope tiers, the channel rules, the retention cut. An id the
   current plan does not admit is not served — including an id that was
   in a block composed five minutes ago under a role the caller has since
   lost, a scope a lapse has since stopped reaching, or a class a
   retention schedule has since cut. Nothing is carried from the block,
   and there is no token, cursor or signed handle to carry.
6. **Recall's refusals are uniform and silent.** An id that does not
   exist, an id in another tenant, an id at a denied scope, an id above
   the caller's tier, an id past its horizon and an id on a channel bank
   mode closed all produce the same outcome: the entry is absent from the
   response. No per-id error, no distinguishing status — the uniform
   not-found discipline of ADR-0032 and ADR-0015, applied so that recall
   cannot become an oracle for "does this record exist". A request naming
   ten ids of which three are inadmissible serves seven; it is never a
   whole-request refusal, because policy outcomes on a read are results
   rather than errors (ADR-0026 decision 1).
7. **Recall is bounded by an id cap and serves bodies in full.** At most
   32 ids per request — comfortably above any block's plausible index
   tier, far below a corpus — and each served entry carries its full
   content. There is no token budget: the caller named specific bodies,
   which is what makes this the deep surface rather than a second inject.
8. **One chained audit event: `context.recalled`.** `AuditAction::ContextRecalled`
   — the feature's only new action vocabulary, and the line that
   discharges seed §2.2 principle 5's naming of recall. Payload: the
   requested id count, the served entries with their VedaFlow object
   addresses and channels (the ADR-0025 decision 7 watermark shape, as
   ADR-0031 decision 11 upgraded it, so a recall is exactly as
   recomputable as an inject), the aggregated per-scope decisions with
   their permitted tiers (ADR-0019 decision 4, ADR-0038 decision 13), the
   instant, and `session_id` when given. Never the content, never the
   requested ids that were refused as a distinguishable list — the
   ADR-0021 discipline.
9. **Every composed entry carries its tier, in the response and on the
   chain.** `body` | `index` on `ComposedEntry`, in the inject response
   per entry, and in the `context.injected` payload beside the object
   address. "Was that agent given the payments runbook, or only told it
   exists" is a question an auditor asks, and it should be answered by
   reading the chain rather than by re-deriving rendered widths from a
   corpus that has since moved.
10. **Index entries are watermarked and audited like bodies.** They
    composed; the block disclosed that the record exists, its class and
    its subject; so their ids join the watermark line's id list and their
    object addresses join the audit event. The id therefore appears twice
    for an index entry — once in-line as the handle, once in the
    watermark — which is honest overhead named in the consequences, with
    ADR-0025's short-id scheme recorded as the escape if the measurement
    says it binds.
11. **The tiering knobs ride the pack, and `off` restores the previous
    product exactly.** `CompositionConfig` gains `index_tier`
    (`off` | `demote`) and `index_entry_chars`, configured in the same
    `composition` JSONB column and the same `synveda policy apply` flags
    as the budget and the channel rule (ADR-0025 decision 3), resolved
    per candidate scope through the same effective-pack walk. The product
    default is `demote` in all three embedded packs: unlike MEM-6's
    horizons, nothing is destroyed or hidden by this default — a
    demotion only ever converts a silent omission into a named one. Under
    `off`, composition is byte-identical to its pre-CTX-4 output, which
    is the MEM-5/MEM-6 discipline and a test rather than a claim.
12. **The legend is charged to the first demotion.** A block containing
    an index entry needs one line saying what `(recall <id>)` means,
    without which the tier is a cost with no navigation. It is counted as
    part of the first demoted entry's cost — so a block with no index
    entry never pays for it and stays byte-identical to today's, and a
    first demotion that cannot afford both the legend and its own line
    does not happen. The line is placed after the preamble when the block
    is joined; the accounting is unchanged by the placement. It must also
    not *contain* the parenthesised handle form it describes — an agent
    scanning the block for `(recall …)` would otherwise find the legend
    first and go looking for a record called `<id>`, which the first draft
    of this feature did and its own acceptance test caught. A legend has
    to describe the marker without being one.
13. **The index names what this composition considered and could not
    carry — never a broader sweep.** Conflict losers (ADR-0025 decision
    6), superseded facts (MEM-5), expired records (MEM-6), material above
    the caller's tier and material on a channel the scope's pack closed
    do not become index entries, because none of them were candidates.
    The set the index draws from is bounded by the same
    `per_scope_kind_candidates` cap and the same relevance ranking that
    bound the body tier. Naming the corpus beyond that is what recall is
    for, and a query-shaped recall is CTX-5's.
14. **The measurement is the acceptance criterion, so it is a metric and
    a test rather than a paragraph.** `synveda_tokens_per_inject` gains a
    companion recorded per tier, and the AC test composes a seeded corpus
    at a real budget with the tier `off` and `demote` and reports three
    numbers: the tokens the index tier spent, the assets named per block
    that the previous product dropped in silence, and the bodies
    displaced to pay for them. That number is what discharges ADR-0025's
    "index tier or watermark overhead measured material" trigger, and it
    is what any later short-id scheme has to beat.
15. **ADR-0027's option 6 is reconsidered and re-rejected, on the
    record.** That ADR made "CTX-4's tiering lands" the trigger for
    revisiting an inject at every `UserPromptSubmit`. It has landed, and
    the answer is no: the index tier *reduces* the need for per-prompt
    injection — the session-start block now names what it could not carry
    and the agent fetches what the prompt actually turns out to need —
    where a per-prompt inject would multiply latency, token cost and
    audit volume to guess at the same thing. Recall is the per-prompt
    surface, and it is one call for one body rather than a whole
    recomposition.

## Options considered

1. **A separate `MemoryList` / `MemoryIndex` Cedar action** — the honest
   reading of "you may know it exists without reading it", and the right
   model for a discovery UI where a user browses names they cannot open.
   Rejected: a second verdict per scope and tier doubles the matrix the
   AUTHZ-5 leak suite covers for a read-path feature whose value is token
   efficiency, and a description is content. If the product ever wants
   it, CNSL-4's memory browser is where the case gets made.
2. **An LLM-written name and description per asset** — what "names +
   descriptions" really wants, and what makes a skills-style index
   readable. Rejected: the read path makes no model calls (structural
   since ADR-0024 decision 7), and a description written at *write* time
   is the extraction pipeline's job. Recorded as MEM-3's to take when an
   asset without an authored description needs one.
3. **A `title`/`summary` column backfilled onto every record** — better
   index lines than truncation, computed once. Rejected for CTX-4: a
   migration, a backfill and an extractor change to improve a line whose
   entire purpose is to be cheap, when MEM-3 already summarises content
   at write time — the truncation is a summary of a summary.
4. **A separate index budget** — clean separation, and the index could
   never displace a body. Rejected: it is the second config plane
   ADR-0025 decision 3 refused, and the displacement it prevents is the
   seed §4.4 gradient doing exactly its job — spending room on a nearer
   scope's named material rather than a farther scope's full text.
5. **A trailing "also available" section rather than inline entries** —
   easier for an agent to scan, and closer to how a skills index looks.
   Rejected: it needs a second ordering, and it breaks the gradient by
   putting a nearer scope's index entry below a farther scope's body.
   Marked inline keeps one order and one section structure.
6. **Signed, expiring handles** — makes recall a cheap lookup with no
   plan walk, and is the obvious performance answer. Rejected outright:
   it is the first construct in the product that would outlive the
   decision that minted it, and every freshness promise since ADR-0014
   dies at it. The plan walk is ~100µs (ADR-0012's benchmark); the
   promise is worth more.
7. **Recall taking a query rather than ids** — CTX-5's actual shape,
   and arguably where the value is. Deferred deliberately: hybrid plus
   graph traversal plus as-of with channel/provenance/validity labels is
   a feature in its own right, and CTX-4 needs only the floor its handles
   point at. The route is shaped so CTX-5 adds a query alternative to the
   same surface under the same audit action rather than replacing them.
8. **Distinguishing recall's refusals — 403 for denied, 404 for
   missing** — better developer experience, and what an API reviewer
   would ask for. Rejected: it makes recall an existence oracle across
   tenants, scopes and tiers, which is precisely what the uniform
   not-found posture exists to prevent.
9. **Demote every candidate over a size threshold, whether or not it
   fits** — the literal skills-style reading, and the right answer once a
   context pack's body is thousands of tokens. Rejected *for now* as a
   guess: nothing in today's corpus has that shape, and decision 2's
   cheaper-than rule reaches the same behaviour automatically once such
   assets exist. Recorded as a trigger rather than built blind.
10. **Do nothing until PRMT-2 and SKIL-1 give the tier something worth
    indexing** — defensible, and the honest observation behind it is that
    tiering pays most for assets that do not exist yet. Rejected: silent
    truncation is a defect *today*, the AC asks for the measurement
    today, and building the slot now is what lets those features arrive
    as a rendering rather than as a redesign of the read path.
11. **Short ids in the index line** — ADR-0025's own recorded successor,
    saving roughly seven tokens per index entry. Rejected pending
    decision 14's number: resolving a short id needs either a
    block-scoped table (state on the read path, which no other read-path
    feature has) or a prefix scan (an oracle). Revisit when the
    measurement says the overhead binds.
12. **Serving bodies from the inject response on a second call keyed by
    block hash** — a cache of what was composed, so recall is a lookup
    rather than a decision. Rejected with ADR-0026 option 7 for the same
    reason: a cache with any lifetime breaks the next-request freshness
    promises, and this one would additionally serve material the caller
    may no longer read.

## Consequences

- Positive: an agent is told what it did not get, so a thin block stops
  being indistinguishable from an empty corpus, and the audit chain
  records the difference per entry; the mechanism is general over
  `AssetKind`, so PRMT-1/2 and SKIL-1 arrive as a rendering rather than a
  read-path redesign; `recall` exists as an audited primitive, which seed
  §3 has promised since day one and seed §2.2 principle 5 requires; no
  new Cedar vocabulary, no new scope producer, no cache, so every
  freshness promise holds unchanged; the index tier's cost is measured
  rather than assumed.
- Negative / accepted trade-offs: a demotion can displace a smaller,
  lower-priority body — the gradient's own consequence, but a real change
  in what a fixed budget buys; the record id appears twice for an index
  entry (in-line and in the watermark), roughly nine tokens of honest
  overhead per entry; a truncated body is a poor description until
  authored assets exist; recall composes a plan per call, so it pays
  inject's decision cost without inject's retrieval leg, and it is
  deliberately uncapped in body size; the id cap is a blunt instrument
  against corpus exfiltration where a rate limit would be the sharper
  one, and AUTH-6 owns that.

### The measurement (decision 14), as taken

`crates/synveda-gateway/tests/tiered.rs::the_index_tiers_token_cost_is_measured`
composes one corpus twice over `POST /v1/inject` — a long unsummarised
runbook at a team, a short note at the reader's own scope, a
240-token budget — first with `index_tier: off` (the product exactly as
it behaved before CTX-4) and then with it on:

| | records named | block tokens | index tier cost |
|---|---|---|---|
| `off` | 1 | 80 | 0 |
| `demote` | 2 | 217 | 122 (56% of the block) |

The 122 breaks down as ~90 for the index line itself (320 characters of
elided body), ~23 for the legend, and ~9 for the record id the watermark
grew by. Read honestly: at this deliberately tight budget the tier is
**expensive** — more than half the block to name one record — and at the
seed §4.4 default of 1,500 tokens the same entry is under 8%. The cost is
a fixed ~90 tokens per named record against whatever the body would have
been, so the tier pays in proportion to how much bigger the body is:
nothing at all for the 15-token records MEM-3's write-time summarisation
produces (decision 2 declines to demote them), roughly 4× for this
360-token runbook, and one to two orders of magnitude for the context
packs and skills PRMT-2 and SKIL-1 will bring.

That is the number ADR-0025's "index tier or watermark overhead measured
material" trigger asked for. It does **not** fire the short-id scheme: the
id is 9 of the 122 tokens, so the scheme option 11 describes would recover
about 7% of the tier's cost in exchange for read-path state or a prefix
oracle. The line to watch is the 90, not the 9 — if it binds, the lever is
`index_entry_chars`, which is already a pack field.

- Reversal triggers: decision 14's measured overhead exceeds what the
  budget absorbs → the short-id scheme (ADR-0025's trigger, discharged
  here with a number rather than closed: it did not fire, and the
  measurement says the id is not where the cost is); PRMT-2/SKIL-1 land with bodies
  far exceeding their index lines → option 9's always-index threshold,
  behind the pack knob decision 11 already added; EVAL-4 shows the index
  tier displacing bodies that mattered → option 4's separate index
  budget; ~~CTX-5 lands → the query alternative joins this route and the
  MCP tool joins the ADPT-1 plugin manifest per ADR-0027's own trigger~~
  **(ADR-0042: both, as recorded — `ids` xor `query` on one route under
  one audit action (decision 1), and one MCP tool in the ADPT-1 manifest
  (decision 15). One thing this ADR did not anticipate: the two forms now
  share the *widened* universe (ADR-0042 decisions 2–3), so a handle
  resolves against the same answer a query would get, rather than against
  a narrower chain-only one)**;
  recall volume or latency binds → the buffered read-path appender
  ADR-0019 option 2 records for inject applies unchanged.

## Compliance notes

- The PDP remains unbypassable: recall's only scope producer is
  `composition_plan`, the same walk inject uses, and an empty plan serves
  nothing; index candidates are plan candidates by construction. Tests
  use packs, embedded and stored, never a bypass (seed §2.2).
- The tier choice never changes *which* records are admissible, only how
  much of an admitted record is shown. A `confidential` or `restricted`
  record that reaches the index carries its tier marker exactly as a body
  entry does (ADR-0038): the harness cannot know what it is holding
  unless the block says so, and that holds for a name as much as for a
  body.
- Audit: one new action type, `context.recalled`, one chained event per
  recall with aggregated per-scope decisions and no content (ADR-0019
  decision 4; DoD #4). `context.injected` gains a per-entry tier so the
  chain distinguishes "was given" from "was told exists".
- Tenant isolation: recall runs inside a tenant transaction
  (`rls::begin_tenant_tx`, ADR-0009) and reads `records`, whose policy the
  adversarial suite already covers — this feature adds no table, so it
  adds nothing for that suite to guard. What is new is the *shape* of the
  attack it invites, a caller naming an id it was never given, and that is
  asserted at the route: `refusals_are_uniform_and_silent` seeds a second
  tenant's record and names it directly, and it comes back
  indistinguishable from an id that never existed.
- Determinism: the tier choice is a function of estimated widths and
  remaining budget — no clock read, no map iteration order — so CTX-2's
  byte-identical re-composition AC holds with the tier on, and is
  asserted with it on.
- Observability: `gateway.recall` span over plan → fetch → append;
  `synveda_context_recalls_total{outcome}` and the per-tier token
  companion of decision 14 (DoD #3).

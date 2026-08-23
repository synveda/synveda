# ADR-0042: recall becomes a query — the universe is the scopes that can contribute, as-of rewinds the corpus but never the authority, and one MCP tool over the route that already exists

- **Status**: Accepted, **superseded in part 2026-08-23 by ADR-0078** (CPR-12): the route is deleted and two of its three shapes have no successor
- **Date**: 2026-07-27
- **Feature(s)**: CTX-5
- **Deciders**: sujitn

## Superseded in part (2026-08-23, CPR-12): `POST /v1/recall` is deleted

ADR-0078 decision 5 deletes the route. Its three shapes did not fare alike,
and saying which is which is the point of this note.

- **`query`** — served by a context run, and **narrower**. This ADR's whole
  subject was the widening: recall asked about every scope that could
  contribute, where inject asked only about the caller's chain. A context
  run is inject's descendant, so *the widening is gone with the route.*
  `synveda recall --query` and the `recall` MCP tool both still work and
  both now compose a run; neither reaches the sibling-team material this
  ADR was written to reach.
- **`as_of`** — no successor. A context run reads the live corpus. The
  bitemporal read this ADR built, and the finding that as-of rewinds the
  corpus and never the authority, are unexercised until Prompt 18.
- **`ids`** — no successor. A context run takes no handles, so the
  re-decide-on-the-way-in property of ADR-0041 has no caller.

The **sweep**, which is `as_of` with no query, is what three eval suites
enumerated a corpus with; they fail by name rather than measure something
else (ADR-0078 decision 5). `demos/ctx-5-recall.sh` is deleted: both of its
claims were properties of the route.

Prompt 18 re-cuts recall over the governed scope model; Prompt 32
re-measures. Nothing here is a decision reversed — it is a surface removed
ahead of being rebuilt, and this ADR is the specification of what has to
come back.

## Context

CTX-5's text is "explicit deep query: hybrid + graph traversal + as-of;
results labelled with channel, provenance, validity. Exposed as ONE MCP
tool", and its acceptance criterion is "MCP client E2E; as-of returns
historically accurate context (`--as-of` demo)". Seed §3 has listed
`recall` beside `inject` and `observe` since day one — "richer and
slower" — and tech plan §3 spells it: "same authZ → richer retrieval
incl. graph traversal + as-of queries → results carry provenance +
channel labels so the agent can weigh derived vs published".

Seven accepted ADRs park obligations here, and this is the feature that
discharges them:

- **ADR-0024 decision 1** fixed CTX-1's candidate universe at the
  caller's placement chain and said the scopes packs permit *beyond* it
  "are recall's deep-query surface, and CTX-5 owns enumerating a broader
  candidate set". Its option 2 deferred full enumeration here — "which
  needs it and can afford batch evaluation work" — and its reversal
  triggers name "CTX-5's broader universe needs batch PDP evaluation →
  revisit decision 1's chain-only universe".
- **ADR-0025 option 6** deferred "compose reading `records_versions` for
  transaction-time as-of … to CTX-5 with the refs half", and its
  decision 5 calls the `at` input "the valid-time half of the as-of seam
  CTX-5 extends to transaction time".
- **ADR-0026 decision 6** says "the as-of *parameter* — transaction time,
  refs, 'what did the agent know on March 3rd' — is CTX-5's surface", and
  its reversal triggers name "CTX-5 lands → the as-of parameter extends
  decision 6".
- **ADR-0027 decision 1**: the ADPT-1 plugin manifest "reserves
  `mcpServers` for CTX-5/ADPT-2 … so both arrive as configuration rather
  than restructuring", with the trigger "CTX-5/ADPT-2 land → the MCP
  recall tool joins this plugin's manifest as `mcpServers`, no
  restructuring".
- **ADR-0029** derived the budget this feature is measured against.
  Seed §10 budgets `inject` and `observe` and says nothing about `recall`
  beyond "richer and slower", so the graph gate derived one — **300ms
  p95** — and decomposed it over "the stages recall already has", of
  which the first is **"PDP plan (permitted scopes): 15ms"**. Its
  consequences record that "the recall budget gets a written decomposition
  that CTX-5 and EVAL-6 inherit". That 15ms is exactly the allowance the
  widened universe spends, which makes this ADR's central decision a
  pre-registered number rather than an open question.
- **ADR-0033**'s trigger (d): "CTX-5 landing → the swept action set gains
  explicit recall", because "CTX-5's explicit `recall` is the stronger
  signal" than an inject for auto-promotion, and
  `promotion.rs` already asserts the evidence names which signal it
  counted "because the set grows (CTX-5)".
- **ADR-0041 option 7** deferred "recall taking a query rather than ids"
  and shaped the route for this: "CTX-5 adds a query alternative to the
  same surface under the same audit action rather than replacing them".
  ADR-0037 decision 12 likewise sends cross-scope reads here: "are
  recall's deep-query surface; CTX-5 owns".

Forces at play:

- **Most of this feature already shipped, on purpose.** CTX-4 built
  `POST /v1/recall`, `AuditAction::ContextRecalled`, and a `RecallEntry`
  that already carries scope, channel, kind, class, sensitivity,
  provenance, the valid window, the object address and staleness — the
  "results labelled with channel, provenance, validity" clause is
  *done*. What does not exist is a way to ask a question rather than name
  an answer, a universe wider than the chain, and a second time axis.
  CTX-5 is a second way in to one surface, not a second surface.
- **The chain-only universe was a cost decision, and it is now a
  functional defect.** The packs grant beyond the chain and say so in
  Cedar: `standard` permits `resource in principal.department &&
  resource.kind != "user"`, `open-collaboration` permits `resource in
  principal.tenant && resource.kind != "user"`, and every pack permits
  the bound subtree of a `viewer`/`contributor`/`curator` binding. Under
  CTX-1's universe none of those reads can be *performed* by the product:
  `standard`'s whole sharing default and a curator's entire bound
  subtree are grants nothing can exercise. That is not a policy
  limitation, it is a missing surface, and it has been recorded as this
  feature's since ADR-0024.
- **A tenant-wide decision sweep is O(scopes × tiers), and the cost is
  materialisation.** AUTHZ-5 made the ask per `(scope, tier)` — four PDP
  calls per scope, no short-circuit (ADR-0038 decision 3) — and AUTHZ-1
  measured the facade at ~109µs median *including entity
  materialisation*. At 1 000 scopes that is ~436ms of decisions, which is
  why ADR-0024 refused it for inject. But `Pdp::entities` rebuilds
  Cedar's entity store per call from HIER-3 fragments; a sweep that
  materialises once and evaluates many is a different cost curve, and
  that is precisely the "batch evaluation work" ADR-0024 said recall
  could afford.
- **A scope that cannot contribute a record is a decision with no
  effect.** `admit` reaches records two ways and only two: a derived
  sweep with a `records.scope_id in (…)` predicate, and a published-member
  read of each planned scope's `memory/published` ref (by id, with no
  scope predicate, because since FLOW-5 a published tree may name a
  record living below it — ADR-0034 decision 6). A scope holding no
  records and publishing nothing produces the same empty contribution
  whether the PDP allowed it or denied it.
- **The bitemporal pair is populated with meaning, deliberately.** MEM-5
  closes a superseded fact's valid window rather than deleting it, "and
  CTX-5's as-of surface inherits a corpus where as-of means something"
  (ADR-0039). MEM-6 chose *expire* as a temporal delete explicitly "so
  `records_history` keeps answering 'what did the agent know on date X'",
  and reasoned about the consequence in the same breath: "a product that
  only expires keeps every payload forever behind an as-of query, which
  is not retention" — which is why there is a second, destroy horizon
  (ADR-0040 decision 5). The store already has `records::as_of` and
  `as_of_bitemporal` over the `records_versions` view.
- **Classification is a judgment about content, not a fact about the
  past.** AUTHZ-5's leak suite asks a corpus back forty ways to prove
  `restricted` material never reaches a reader who did not earn the tier.
  A record raised from `internal` to `restricted` in April has the same
  bytes it had in March; serving its March version at its March tier
  would defeat that suite with a query parameter.
- **Where material *lived* is a fact about the past.** The symmetric
  argument does not hold for scope: a record that sat at a team the
  caller could read, and has since moved, discloses nothing by being
  reported where it was. The asymmetry is real and has to be stated
  rather than resolved by uniformity.
- **Authority must not rewind.** Every freshness promise since ADR-0014
  — a pack flip, a role revocation, a hierarchy move, a lapse's window
  closing in the query that asks, a retention schedule governing the very
  next inject — says the request in front of us is decided by current
  state. ADR-0041 decision 5 refused a signed handle for exactly this
  reason. An as-of that reconstructed March's *permissions* would let a
  leaver read March, and would be the same defect wearing a timestamp.
  Reconstructing historical authority is a real question, and it is
  AUD-2's: "who could see X on date D" is answered from the audit chain,
  which recorded the decisions as they were taken.
- **The search indexes hold current truth by construction.** The Tantivy
  sidecar tails the bitemporal pair and re-reads each changed id's
  *current* version (ADR-0024 decision 4); the HNSW index is over
  `record_embeddings` for live records. Neither can rank a version that
  no longer exists. A query leg and a time machine do not compose for
  free.
- **The harness adapter is prebuilt and dependency-free.** ADR-0027
  decision 1 fixed "Node ≥22 stdlib only — global `fetch`, no runtime
  dependencies, no install step when the plugin is enabled (the AC's two
  minutes cannot include an `npm install`)", built with `tsc` and no
  bundler. An MCP SDK is a dependency tree and a bundler.
- **Graph traversal has no schema to traverse.** GRPH-1 and GRPH-2 are
  unbuilt, and ADR-0029's spike already moved the burden of proof off
  AGE and onto relational adjacency for GRPH-1's own design ADR. The
  backlog puts graph-in-recall in GRPH-3 (Phase 3), specified as "1–2 hop
  expansion in recall ranking; degradable (retrieval works with graph
  off)" and feature-flagged — and tech plan §6 lists "graph features are
  additive … degrade gracefully" as the standing mitigation.
- **Recall's caller can see an error; inject's cannot.** ADR-0026
  decision 4 built a degradation ladder where every rung is a 200,
  because a session-start hook must never break a session. An agent that
  called a tool and got a partial answer with a header it cannot read is
  worse off than one told the truth.

## Decision

`POST /v1/recall` gains a **query** alternative beside CTX-4's ids, and
an **as-of** pair beside its instant. Both forms are decided over the same
widened universe — the scopes that can contribute to *this* request,
every one of them individually PDP-decided — and both chain the same
`context.recalled` event. The tool half is one MCP tool, `recall`, served
over stdio from the ADPT-1 plugin package and registered through the
`mcpServers` slot ADR-0027 reserved for it.

Decisions, specifically:

1. **One route, three request shapes, exclusive.** `POST /v1/recall` takes
   `{ ids }` **xor** `{ query }` **xor** neither — plus optional `as_of`,
   `valid_at`, `limit` and `session_id`. `ids` and `query` together is an
   `Invalid`: a query that also names ids is two questions, and answering
   the intersection would be a third nobody asked. *Neither* is valid only
   when `as_of` is given, and that is the swept form decision 14 needs —
   "everything I may read, as it stood then" is a complete question, and
   the one a query cannot answer. A bare recall with nothing at all stays
   an `Invalid`: it has not said what it wants. The response shape, `RecallEntry`, is
   unchanged: the labels CTX-5 owes are the ones CTX-4 already renders.
   `limit` is bounded by the same 32 the ids form is capped at (ADR-0041
   decision 7) and defaults to it, so both shapes return bodies in full
   under one entry ceiling and neither becomes a bulk export — the id cap
   was never about ids, it was about how much corpus one call may carry.
   Policy outcomes remain results rather than errors (ADR-0026
   decision 1): a query matching nothing the caller may read returns an
   empty entry list with a 200.
2. **The universe is the scopes that can contribute to this request, and
   every one of them is decided.** This is the ADR-0024 deferral, and the
   answer is neither "the chain" nor "every scope in the tenant":
   - the **ids** form decides the scopes that *hold or publish those ids*
     — one indexed read over `records` for the named ids' scopes, one over
     the `memory/published` members that name them;
   - the **query** form decides the tenant's **occupied** scopes — those
     holding at least one record, plus those whose `memory/published` ref
     is non-empty.

   This is a cost narrowing with no semantic content: `admit` reaches
   records only through a scope-predicated derived sweep and a published-member
   read, so a scope that holds nothing and publishes nothing contributes
   the empty set whether the PDP allowed it or denied it. Deciding it
   would change no result, only the clock. Every scope that *can*
   contribute is asked, per tier, through the same facade — there is no
   shortcut that infers a verdict from a pack's shape, because that would
   be a second source of truth about policy, which is what ADR-0041
   decision 1 refused for the index tier and what this refuses here.
3. **The broad plan is the lapse mechanism generalised, not a new
   one.** AUTHZ-4 already built "a scope outside the caller's chain,
   decided under its own chain and its own assignments" — `LapsedScope
   { lapse, chain, assignments }` and `composition_plan`'s second loop
   (ADR-0037 decisions 10–12). CTX-5 widens that struct's `lapse` to an
   `Option` and feeds it candidates from decision 2 instead of from
   `lapsed_scopes`. `composition_plan` gains one entry point that takes a
   candidate list; the per-scope body — four `MemoryRead` asks, the
   target's own effective pack for its channel rule, its own retention
   horizons, its own index-tier config — is byte-for-byte the loop that
   is there. Composition, inject and both recall forms therefore keep
   sharing exactly one answer to "may this caller see this record".

   **A widened universe needs a widened binding set.** `gather` reads the
   caller's role bindings *on their own chain*, which is right for inject
   — a binding anywhere else could not bear on a decision it takes. Recall
   decides scopes off that chain, and a binding at one of those scopes is
   exactly the grant an administrator issues to widen someone's reach: one
   of the two ADR-0024 left unreachable. So recall reads every binding the
   subject holds in the tenant. This widens what is *read*, never what a
   decision considers — `effective_roles_at` still admits a binding only
   at a resource whose own chain contains its scope. Missing this makes
   the feature silently not work, which is how it was found: the AC test
   for it failed.
4. **The caller's chain is always decided in full; beyond it, only
   contributors are.** The chain half of the plan is unchanged, denials
   included, so the audit event keeps carrying "you were denied at your
   own department" the way `context.injected` does (ADR-0019 decision 4,
   ADR-0038 decision 13). Scopes beyond the chain appear in the decision
   list only when they were candidates, because a decision the product
   did not need to take is not a decision it should claim to have taken.
5. **The universe is capped by ADR-0029's plan allowance, and a
   truncation is reported rather than silent.** `SYNVEDA_RECALL_MAX_SCOPES`
   bounds the query form's candidate set, and its default is not a round
   number picked for comfort: it is the count decision 17 measures as
   fitting the **15ms** the graph gate allotted the plan stage of a 300ms
   recall, which is why that measurement is an assert rather than a
   report. It is **32** — a small number, arrived at honestly, and the
   consequences section says what it costs. Candidates are ordered
   nearest-first — the caller's chain, then lapse targets, then remaining
   occupied scopes by hierarchy distance — so a cap drops the farthest
   material rather than an arbitrary slice. When it binds, the response
   carries `truncated: true` with the counts and the audit payload records
   them. A bounded answer presented as a complete one is the failure mode
   this product cannot afford.
6. **One entity materialisation per request, evaluated many times.** The
   sweep resolves each candidate's chain through the HIER-2 cache and
   reads the tenant's policy assignments once, then materialises the
   Cedar entity set once and evaluates every `(scope, tier)` against it,
   instead of paying `Entities::from_entities` per call. This is the only
   performance mechanism the feature adds, it changes no verdict — the
   same policies over the same entities and the same context — and it is
   exactly the "batch evaluation work" ADR-0024 option 2 said recall
   could afford. Its contribution is measured with it and without it
   rather than assumed (decision 17).
7. **As-of is two explicit instants, both defaulting to the request
   instant.** `as_of` is **transaction time** ("as the database knew it
   then") and `valid_at` is **valid time** ("about the world as of
   then") — the two axes FND-4 built and ADR-0006 recorded, no third
   concept. Given neither, recall behaves exactly as it does today, which
   makes CTX-4's surface a special case of this one rather than a
   sibling. `synveda recall --as-of <t>` and the MCP tool's `as_of` set
   both, because "what did the agent know on March 3rd" is the
   diagonal query and it is the one the AC asks for; `--valid-at` is
   available separately for "what does today's corpus say about March".
8. **As-of rewinds the corpus; it never rewinds the authority.** The PDP
   decides with the caller's *current* identity, roles, packs, lapses and
   hierarchy placement, at the request instant, whatever `as_of` says. A
   leaver, a demoted user, a caller whose lapse expired last night reads
   nothing historical, because there is no historical permission to
   inherit. This is the ADR-0041 decision 5 promise ("a handle is a name,
   not a capability") stated for time instead of for ids, and it draws
   the line between this feature and AUD-2: CTX-5 answers *what was
   there*, AUD-2 answers *who could see it*, from the chain that recorded
   the decision.
9. **Classification is retroactive: a version is admitted at the
   strictest tier its record has carried at or after the as-of instant.**
   A record raised to `restricted` in April is `restricted` for its March
   version too, so AUTHZ-5's leak suite cannot be defeated by a
   timestamp. The rule is a maximum over the versions in the window, so
   at `as_of = now` it degenerates to the current tier and today's
   behaviour is unchanged — and a *declassification* does not retroactively
   expose the history that was written while the record was classified,
   which is the direction a compliance function actually cares about.
10. **The axis is fact versus judgment, and it decides all three: scope
    rewinds, classification and publication do not.** Where material
    lived is a *fact* about the past, so the version's own `scope_id`
    attributes it and the plan decides that scope. What material *is*
    (decision 9) and whether the organisation *stands behind* it are
    *judgments*, revisable, and a revision governs every read including
    historical ones — so the `memory/published` tree is read at its
    current state, never as of the instant, and bank mode's
    published-only rule is likewise the pack's answer now.

    Rewinding the refs is the tempting symmetry and it would undo
    FLOW-7. That feature's whole claim is a bad instruction reaching not
    one further agent 0.2 seconds after a curator rolls the channel back;
    if `as_of` could re-publish what a rollback withdrew, the withdrawal
    is defeated by a query parameter — the identical failure shape
    decision 9 closes for classification. A record published in March and
    withdrawn since is still *served* as-of March if the caller may read
    its scope; it is served as derived material, marked unreviewed, which
    is the honest statement that nobody stands behind it any more.
11. **As-of reaches expired material and never destroyed material —
    MEM-6's decision, inherited rather than retaken.** ADR-0040
    decision 5 chose the two horizons knowing that "a product that only
    expires keeps every payload forever behind an as-of query, which is
    not retention", and answered with destruction. So the read horizon
    governs the *live* corpus (inject, and recall at the request
    instant), and a transaction-time as-of read reaches what the database
    held then, bounded absolutely by the destroy horizon, which is the
    thing that makes retention real. The lever if a deployment needs
    history to close sooner is the destroy horizon, already a pack field.
12. **The query leg is CTX-3's, with the degradation posture inverted.**
    The task text is embedded through the MEM-4 `Embedder` seam under the
    same read-path deadline (ADR-0026 decision 3), fused by the CTX-1
    engine over the widened `SearchFilter`, and the fused ids become
    `ComposeRequest.only` — so the ranked set is *narrowed* by admission
    and never widened by it. Embedder failure degrades to sparse-only
    with `X-Synveda-Degraded: embedder`, exactly as inject does, because
    that is still a genuine ranked answer over the same corpus. A
    retrieval failure is an honest 5xx rather than an unranked sweep:
    inject degrades because its caller cannot see the error, and recall
    reports because its caller asked the question and can.
13. **The ids form returns gradient order; the query form returns
    relevance order.** CTX-4 sorted by gradient so a recall reads like the
    block its handles came from, and that stays. A query is not a block:
    the best match belongs first, or the `limit` truncates the wrong end.
    Two orders on one route, each following from what was asked.
14. **A transaction-time as-of *without* a query is a bitemporal sweep;
    *with* a query it ranks over material that still exists.** The
    indexes hold current truth by construction (ADR-0024 decision 4), so
    a since-expired record cannot be ranked — it can only be swept or
    named. Rather than pretend otherwise, the two shapes are honest and
    different: `{ as_of }` alone reads `records_versions` directly through
    the same admission, gradient- and recency-ordered, and is the complete
    answer to "what did the agent know on March 3rd" — the `--as-of` demo's
    shape; `{ query, as_of }` ranks the survivors and serves their
    as-of bodies, and says so in the response. Indexing historical
    versions is the recorded upgrade, not a thing to build on a guess.
15. **One MCP tool, `recall`, stdio JSON-RPC, in the ADPT-1 package.**
    Exactly one tool for the whole surface — `{ query?, ids?, as_of?,
    valid_at?, limit? }` — never one per shape, because "exposed as ONE
    MCP tool" is the feature text and because a second tool would be a
    second place for the id/query exclusivity to be got wrong. It ships
    as a third entry point in `adapters/claude-code` beside the hook and
    the driver, registered via `mcpServers` in `.claude-plugin/plugin.json`
    (ADR-0027's trigger, discharged as configuration), Node ≥22 stdlib
    only, with the JSON-RPC framing written directly rather than taken as
    a dependency (option 8 records why, and what reverses it). Its bearer comes from the same
    `synveda` CLI seam the hooks already shell to (ADR-0027 decision 4) —
    no second credential path, and nothing the caller's own bearer could
    not fetch. Results are returned as text rendered by the block's own
    line renderer plus a watermark line, so an agent that has read an
    inject block does not have to learn a second format.
16. **No new Cedar vocabulary, no new audit action, and the promotion
    sweep's signal set grows.** Authorization is the plan, per `(scope,
    tier)` `MemoryRead`, so agents reach the widened surface through the
    AUTH-3 floor with zero new grants. `context.recalled` (ADR-0041
    decision 8) carries the query form too: `mode` (`ids` | `query`), the
    two instants, the universe size and truncation, the requested/served
    counts, the per-entry watermark, and `query_hash` (BLAKE3) when a
    query was given — never the query text, the ADR-0021 discipline that
    already governs `task_hash`. ADR-0033's trigger (d) is discharged
    here: the promotion sweep's action set gains `context.recalled`, and
    its evidence names which signal it counted, which
    `promotion.rs` was written to assert.
17. **The plan stage is measured against ADR-0029's 15ms, and that
    number sets the cap.** The widened universe's decision cost is what
    this ADR is most exposed on, and it is the one stage of recall with a
    pre-registered allowance nobody can tune to the result. An `--ignored`
    test seeds a tenant with scopes spread across the hierarchy and
    reports candidate scopes decided, PDP decisions taken, and plan-stage
    wall time — with and without decision 6's single materialisation, so
    the mechanism's contribution is visible rather than assumed — and
    **asserts the median inside 15ms** at the shipped cap, reporting
    p95/p99 (the HIER-1/MEM-1/CTX-1 discipline: virtualised dev IO owns
    the tails, EVAL-6 owns percentile enforcement). The same run reports
    the whole recall split against the 300ms decomposition, so the slice
    GRPH-3 will claim for graph expansion is measured as still being
    there. If the assert cannot be met, the cap comes down and the
    truncation of decision 5 becomes visible — which is the honest
    failure, and the reason truncation is reported at all.

## Options considered

1. **Sweep every scope in the tenant, occupied or not** — the honest
   literal reading of "the full permitted set", and one fewer query.
   Rejected as pure cost: an empty scope's verdict changes no byte of the
   answer (decision 2), so the sweep would be buying decisions nobody can
   observe, and it makes the feature's cost scale with the org chart
   rather than with the corpus.
2. **Infer the universe from the pack's shape** — read `standard` as
   "department subtree", `open-collaboration` as "tenant", and skip the
   PDP for scopes no pack could reach. Tempting, and much cheaper.
   Rejected outright: it is a second model of what the policies mean,
   living beside the policies, and it silently *under*-returns the moment
   a custom pack grants something the inference does not know about. The
   packs are data (ADR-0002); reading them twice, once as Cedar and once
   as a Rust heuristic, is how a policy engine stops being the decision
   point.
3. **Declare the universe as a pack field** (`recall_universe: chain |
   department | tenant`) — one source of truth, reviewable, rides the
   config plane ADR-0025 decision 3 established. Genuinely close, and
   rejected on a narrow point: it still has to *agree* with the pack's
   own permits, so it is the same two-statements-must-match problem as
   option 2 with better ergonomics, and decision 2's occupied-scope
   bound already delivers the cost it was meant to buy. Recorded as the
   lever if the measurement shows the sweep binding at scale.
4. **Cache the permitted-scope set per identity** — the obvious answer to
   the sweep's cost, and it would make repeat recalls nearly free.
   Rejected with ADR-0026 option 7 and ADR-0041 option 12: a cache with
   any lifetime breaks the next-request freshness promises, and this one
   would cache *authorization*, which is the worst thing in the product
   to serve stale.
5. **Rewind authority with the corpus** — reconstruct March's packs,
   roles and placements so "what did the agent know" is answered exactly
   as the agent would have been answered. Superficially the most honest
   reading of the AC, and rejected as a security hole with good manners:
   it lets a revoked reader read, it makes every historical
   misconfiguration permanently exploitable, and the question it answers
   is AUD-2's, from the audit chain, which recorded the real decisions
   rather than a replay of them.
6. **Serve historical versions at their historical sensitivity** —
   internally consistent, and arguably what "historically accurate"
   means. Rejected: it defeats AUTHZ-5's leak suite with a query
   parameter (decision 9). A classification is a statement about the
   content, and the content is what is being served.
7. **Refuse to serve records that no longer exist** — the strictest rule,
   and it would make every guarantee mechanical because admission would
   always run over a live row. Rejected: MEM-6 chose expire-as-temporal-delete
   specifically so history stays answerable, and this option would make
   that choice pointless and the seed's lead demo unbuildable.
8. **Take `@modelcontextprotocol/sdk` for the MCP server** — protocol
   correctness for free, version negotiation maintained upstream, less
   code, and the licence is fine. Rejected on ADR-0027 decision 1's
   standing constraint: the plugin is `tsc`-built with no bundler and no
   runtime dependencies precisely so enabling it needs no install step,
   and vendoring an SDK plus its transitive tree means adopting a bundler
   to protect a two-minute AC. The surface actually needed is
   `initialize`, `tools/list`, `tools/call` and one notification over
   newline-delimited JSON-RPC. Reversal trigger recorded: protocol
   revisions churning, or a second transport, and the SDK plus a bundler
   is the answer.
9. **Ship the MCP server as `synveda mcp` in the Rust CLI** — the CLI is
   already the credential holder, already a shipped binary, and a Rust
   server would serve ADPT-2's standalone case too. Rejected for CTX-5:
   seed §7 fixes TypeScript for the harness adapter, ADR-0027 decision 1
   reserved this package's manifest slot for exactly this server, and the
   credential seam is already a CLI shell-out from TS. ADPT-2's generic
   server has a different deployment story (standalone, likely HTTP) and
   should not be pre-built here; if it wants Rust, the TS entry point
   becomes a thin alias.
10. **A separate `POST /v1/search` for the query form** — cleaner
    request types, no xor validation, and REST reviewers like it.
    Rejected: seed §3 has three primitives and this is one of them; two
    routes would be two audit emission points, two places for the
    universe to drift, and the "ONE MCP tool" clause would immediately
    have to bridge them back together.
11. **Let the caller supply the query vector** — ADR-0026 option 5
    parked this here ("CTX-5's recall surface may revisit for explicit
    deep queries"). Revisited and rejected: it couples the caller to the
    corpus's model identity, and the text/vector mismatch it opens is
    unverifiable server-side — on the *deep* surface, where a mismatched
    vector silently returns plausible nonsense, that is worse than on
    inject. The gateway keeps owning the embed call.
12. **A graph traversal leg now, over a minimal edge table** — CTX-5's
    feature text names it, and MEM-5's supersession edges are already a
    relational adjacency that ADR-0039 declined to mirror into AGE.
    Rejected: GRPH-1 owns the schema decision and ADR-0029 pre-registered
    the criteria for it; building a private edge shape here would either
    duplicate that or pre-empt it. GRPH-3 is where graph joins recall,
    specified as degradable and feature-flagged, and decision 12's fused
    id list is the seam it plugs into without touching admission.
13. **Do nothing until GRPH-1/2 land, so CTX-5 can ship the whole
    sentence** — defensible, and the feature text does say "hybrid +
    graph traversal". Rejected: the AC says "MCP client E2E" and the
    `--as-of` demo, neither of which needs a graph; the broader universe
    is a defect being carried since CTX-1; and the sequencing already
    puts graph-augmented recall in GRPH-3 for exactly this reason.
14. **Rewind the channel refs with the corpus** — the symmetric reading
    of as-of, genuinely more faithful to "what did the agent know", and
    what ADR-0025 option 6 had in mind by "the refs half". FLOW-1/2 keep
    the history to do it: commits are a log and `synveda channel history`
    already renders it. Rejected in decision 10: it would let `as_of`
    re-publish material a FLOW-7 rollback withdrew, which is the one
    thing that feature exists to make impossible. The history is not lost
    — the withdrawn record still composes as-of, as *derived* and marked
    unreviewed, which says something true that a rewound ref would not:
    it was there, and nobody stands behind it now.

## Consequences

### The measurement (decision 17), as taken

`crates/synveda-gateway/tests/recall.rs::the_plan_stage_fits_the_budget_adr_0029_derived`
seeds 512 occupied teams under one department and drives
`POST /v1/recall` with a query, reading the stage split from the gateway's
own Prometheus exposition. The number moved three times before it fit, and
each move is worth recording because each was a different lesson:

| | plan stage | whole request |
|---|---|---|
| 512 scopes, entities re-materialised per decision | 378ms | 411ms |
| 512 scopes, one materialisation per walk (decision 6) | 120ms | 141ms |
| 512 scopes, pack + roles resolved once per scope | 120ms | 141ms |
| 48 scopes, assignments read once rather than per candidate | 15.2ms | — |
| **32 scopes (the shipped cap)** | **13.2ms** | **17.1ms** |

Read honestly, three things:

1. **Decision 6 was the whole ballgame** — 3.1× — and it was written into
   this ADR before the code existed, which is the only reason the feature
   was ever plausible. Cedar's per-decision cost is dominated by building
   the entity store, not by evaluating policy against it.
2. **The next two optimisations barely moved it.** Resolving the pack once
   per scope instead of once per tier is obviously right and bought
   nothing measurable; what remains is `Request::new` with schema
   validation, four times per scope, and that is Cedar's floor rather than
   ours. The lesson is that the cap, not the code, was the lever left.
3. **Most of the remaining budget is not the sweep at all.** At the
   shipped cap the fixed cost — identity read, chain, occupancy reads,
   assignment and binding reads — is roughly 7.7ms of the 13.2ms, on
   Docker Desktop's virtualised fsync. So 32 is a *dev-hardware* number
   and deliberately conservative; `SYNVEDA_RECALL_MAX_SCOPES` exists so an
   operator who has measured their own plan histogram can raise it, and
   EVAL-6 owns re-deriving it on production-shaped IO.

The uncomfortable part, stated plainly: **32 scopes is a small universe
for an enterprise product**, and a tenant with more occupied scopes than
that gets a genuinely incomplete answer to a query. It is reported —
`truncated`, with both counts, in the response and on the audit chain —
and the ordering means what is dropped is the farthest material rather
than an arbitrary slice. But it is a real limitation on the day this
ships, not a theoretical one, and the reversal triggers below name what
would lift it.

- Positive: the grants the packs have always made are finally
  exercisable — `standard`'s department default and a curator's bound
  subtree stop being unreachable text; the third primitive becomes the
  deep query seed §3 promised, over the surface, audit action and entry
  labels CTX-4 already built, so nothing about the read path is
  restructured; the bitemporal machinery FND-4 built, and that MEM-5 and
  MEM-6 went out of their way to keep meaningful, becomes a surface a
  customer can call — the regulated demo the seed leads with, two phases
  after FND-4 laid the tables for it; the id and query forms share one universe and
  one admission function, so the surface cannot answer two ways about the
  same record; no new Cedar vocabulary, no new audit action, no cache, so
  every freshness promise holds unchanged; the MCP tool arrives as a
  manifest entry, as ADR-0027 designed for.
- Negative / accepted trade-offs: the query form's plan walk costs one
  decision per occupied scope per tier, so recall's floor now scales with
  how widely a tenant spreads its material rather than with how much of it
  there is — bounded by a cap derived from ADR-0029's 15ms plan allowance,
  asserted rather than reported, and with option 3's declared universe
  recorded as the lever if the shape of real tenants breaks it; a tenant
  wide enough to trip the cap gets a genuinely incomplete answer, which is
  reported honestly but is still incomplete; CTX-4's id form
  inherits the widened plan and therefore a slightly costlier walk than
  the chain-only one it shipped with, bounded by decision 2 to the scopes
  the named ids actually touch; a query cannot find a since-expired
  record, so the complete historical answer is the swept form rather than
  the ranked one (decision 14); recall as-of reaches material past its
  read horizon until the destroy horizon closes it, which is MEM-6's
  accepted position and not a new one; the hand-written JSON-RPC loop is
  protocol code the project now maintains; and the id cap plus the scope
  cap remain blunt instruments against corpus exfiltration where a rate
  limit is the sharp one — still AUTH-6's.
- Reversal triggers: the 32-scope cap is reported as `truncated` often
  enough to matter in the field → in order, (a) EVAL-6 re-derives it on
  production-shaped IO, where the ~7.7ms fixed cost that dominates it
  today largely disappears, (b) option 3's pack-declared universe behind a
  field on the existing composition config, (c) a cheaper decision —
  Cedar-side batch APIs, or the OpenFGA adapter path (AUTHZ-6, the
  ADR-0002 escape) — which is the lever that raises the ceiling rather
  than moving the ration; EVAL-6 or GRPH-3 measures the 300ms
  decomposition breached end to end → the stage allowances are re-cut
  where the evidence says, not where this ADR guessed;
  as-of queries needing to rank since-expired material → index historical
  versions in the sidecar, extending ADR-0024 decision 4's change feed
  rather than replacing it; GRPH-1/2 land → GRPH-3 adds a third leg to
  decision 12's fusion, feature-flagged and degradable; MCP protocol
  revisions churn or a second transport is wanted → option 8's SDK plus a
  bundler; recall volume or latency binds → the buffered read-path
  appender ADR-0019 option 2 records, applying here exactly as it does to
  inject; ADPT-2 needs a standalone server → option 9's Rust binary, with
  the TS entry point as an alias.

## Compliance notes

- The PDP remains unbypassable, and the widened universe is the point
  where that has to be said carefully: the only scope producer is still
  `composition_plan`, every candidate scope is an individual per-`(scope,
  tier)` `MemoryRead` decision under that scope's own effective pack, and
  decision 2's narrowing removes only scopes that could not have
  contributed a record under any verdict. An empty plan serves nothing.
  Tests use packs, embedded and stored, never a bypass (seed §2.2).
- Refusals stay uniform and silent (ADR-0041 decision 6). The query form
  adds an existence-oracle surface the id form did not have — a caller
  can probe with a query rather than an id — and the answer is the same:
  inadmissible material is absent, never distinguished, and the counts
  the response carries are of what was asked and served, never of what
  was refused and why.
- Audit: no new action type. One chained `context.recalled` event per
  recall, aggregated per-scope decisions with their permitted tiers
  (ADR-0019 decision 4, ADR-0038 decision 13), the universe size and any
  truncation, and no user content — the query rides as a BLAKE3 hash, as
  the task does on inject (ADR-0021). DoD #4.
- Tenant isolation: both forms run inside `rls::begin_tenant_tx`
  (ADR-0009); the occupied-scope and candidate reads are tenant-predicated
  in SQL as well as RLS-backstopped; `records_versions` is the FND-4 view
  over `records` and `records_history`, both already in the adversarial
  RLS suite. The new attack shape is a caller asking as-of for a record
  they may not read *now*, and it is asserted at the route: the historical
  version is refused indistinguishably from one that never existed.
- Classification: decision 9 makes the tier check a maximum over the
  as-of window, so no historical version can be served below the
  strictest tier its record has carried since. The AUTHZ-5 leak suite
  gains the time axis — every case it asks now gets asked again with an
  `as_of` that predates the classification.
- Determinism: the two instants are explicit inputs and echoed in the
  response, and nothing below the seam reads a clock, so CTX-2's
  byte-identical re-composition holds for recall — the same request at
  the same instants over the same database state returns the same
  entries, which is what makes the audit event a recomputable claim.
- Observability: the `gateway.recall` span gains universe → embed →
  search → admit → append stages; `synveda_context_recalls_total` gains a
  `mode` label, and the candidate/decision counts of decision 17 are
  recorded per request (DoD #3).

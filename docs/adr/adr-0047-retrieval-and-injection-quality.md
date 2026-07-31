# ADR-0047: Retrieval & injection quality — the block as the lens, a corpus that had to be promoted to span scopes, and two paths because only one of them can rank

- **Status**: Accepted
- **Date**: 2026-07-31
- **Feature(s)**: EVAL-4
- **Deciders**: sujitn

## Context

EVAL-4 is "fixture Q&A per scope; probe-based compression eval (CTX-6);
tokens-per-inject trend. AC: composition changes show measurable quality effect
before merge." Like EVAL-1 and EVAL-2 it arrived without usable acceptance
criteria — "measurable quality effect" names no axis and no artefact, and one of
its three clauses names a feature that does not exist yet — so this ADR writes
them (recorded in SYNVEDA_FEATURES.md and docs/backlog/EVAL-4.md) as well as
deciding the shape. Third time, same precedent (ADR-0028, ADR-0046), same
reason.

Forces at play:

- **Six features have parked something here, and each is a decision waiting on a
  number.** CTX-1 (ADR-0024 decision 8) measures fused quality against
  constructed geometry in CI and sends real-model quality targets and their
  regression gates to EVAL-4 by name. CTX-2 (ADR-0025 decision 4, option 2, and
  a reversal trigger) ships `ceil(chars/4)` as the token estimator and says
  "EVAL-4 measures the bias". CTX-3 (ADR-0026's reversal triggers) sends
  "EVAL-4/6 evidence" as the condition for revisiting the client-supplied query
  vector and the response-block cache. CTX-4 (ADR-0041's reversal triggers)
  makes "EVAL-4 shows the index tier displacing bodies that mattered" the
  trigger for option 4's separate index budget. MEM-6 (ADR-0040 option 7,
  option 9, reversal trigger (b)) records the staleness score as "an unvalidated
  heuristic until EVAL-4 measures it". And EVAL-1 (ADR-0028 option 5) names
  "EVAL-4 lands with composition-quality scenarios" as the trigger to move the
  gate onto the pull-request path. None of those unblock without a measurement
  someone can act on.
- **The nightly's embedder cannot rank, and this is the feature that is named
  for ranking.** ADR-0028 decision 6 fixes the deterministic extractor and
  embedder as the suite's configuration; ADR-0023 decision 6 makes that
  embedder's geometry meaningless by construction. The dense leg therefore ranks
  by nothing on the path the gate runs on — `evals/scenarios/02` already says so
  in its own `description`, having been rewritten once for exactly this reason.
  The sparse leg is unaffected: Tantivy BM25 over content is real on any stack.
  So half of retrieval is measurable where the gate lives and half is not, and
  which half is which has to be decided deliberately rather than discovered by a
  scenario that fails for a reason no code change can fix.
- **"Per scope" is not a free parameter.** Records land at the caller's home
  scope (ADR-0020), and a service identity is placed as a `ScopeKind::User` leaf
  under its anchor (ADR-0018 decision 2). No actor can write to a team, a
  department or an org node, so a corpus that spans scopes cannot be arranged by
  registering actors higher up the tree — it exists only through FLOW-5's
  promotion or FLOW-2's publication. This is the same mechanism ADR-0046
  decision 2 relies on for the opposite purpose, where one actor per fixture
  group *isolates* a corpus.
- **A Q&A corpus is seeded once and asked many times, and the scenario format
  cannot say that.** EVAL-2's second finding is the reason it matters:
  `wait_for_seed` waits only for the material a scenario is graded on, so two
  scenarios sharing a tenant compose over different corpora — two byte-identical
  runs measured `tokens_mean` 129.8 and then 157 with no product change. Twenty
  questions expressed as twenty scenarios would seed twenty times and measure
  twenty different corpora.
- **"Before merge" contradicts the standing trade.** ADR-0028 decision 8 keeps
  the gate nightly so `ci.yml` stays database-free and fast, and states the trade
  explicitly: a regression is caught within a day, not within a merge. EVAL-4's
  AC asks for the other side of it.
- **One of the three clauses has no product to measure.** "Probe-based
  compression eval (CTX-6)" names Session compression assist, which is Phase 3
  and unbuilt. EVAL-1's own rule — a bounded metric that stopped being measured
  is a breach, not a pass — means an axis for it would fail every run from the
  day it was written.

## Decision

EVAL-4 measures **what a fixed budget buys** over `POST /v1/inject`, against a
Q&A corpus whose material sits at four scope tiers because the suite **promoted
it there through the governed path**. Questions declare what they need from the
embedder, which splits the measurement across two committed baselines: the
composition and lexical-retrieval axes gate on the deterministic path and move
onto the **pull-request** job, and the dense-leg axes gate on a nightly run with
live TEI. Grading joins seed to block by **record identity**, not by string
containment.

Decisions, specifically:

1. **The lens is the inject block, which is the inverse of EVAL-2's choice for
   the same reason.** ADR-0046 option 1 rejected the block because it is
   budget-bounded, relevance-ranked, and elides what CTX-4 demotes. Those three
   properties are precisely what this feature measures: absence *is* the
   signal here. The response already carries every field the measurement needs —
   `record_ids` and `tiers` in block order, `index_entries`, `index_tokens`,
   `tokens`, `budget_tokens`, `staleness_permille`, `block_hash`, `channels`,
   and `degraded`. EVAL-4 adds no route, no action type and no PDP surface; the
   harness's `InjectResponse` stops ignoring fields the gateway is already
   sending it, and the empty dependency set (ADR-0028 decision 1) is untouched.

2. **Grading is by record identity, not by marker containment.** The join
   already exists in the client and is used end to end: observe's per-event
   `event_id` (`ObserveEventOutcome`) → the recall sweep's `provenance.event_id`
   (`RecallEntry::source_event_id`) → `record_id` → its index in the block's
   `record_ids` → `tiers[i]`. Containment cannot do this job at all: an index
   entry carries the body truncated at `index_entry_chars` (ADR-0041
   decision 3), so a marker may or may not survive into it, and "demoted" and
   "absent" are then the same measurement. It is also the predicate EVAL-2's
   first live run broke (ADR-0046 decision 5, amended) — worth not inheriting
   where an exact key is available.

3. **The corpus spans scopes through promotion, because nothing else can put
   material above a leaf.** Each Q&A corpus declares seed batches as usual and
   then `promotions`: the owning actor opens `POST /v1/proposals` naming the
   target scope, its own home as `source_scope_id`, and the record ids the sweep
   returned; the approvers the target's pack requires approve; the material then
   sits on the target's published channel and composes for every reader whose
   chain walks through it. A per-scope answer rate is therefore an assertion
   about FLOW-5 as much as about CTX-2, which is correct — it is the same claim
   the product makes. `evals/lib.sh` binds the approver roles at `eng` and
   `acme` and registers the readers; the runner stays a client that calls `/v1`.

4. **A third suite, for the reason EVAL-2 discovered rather than for tidiness.**
   `evals/fixtures/qa/*.json`: one `corpus` (seed batches plus promotions) and
   many `questions`, each with its own probe and expectations. The runner seeds,
   waits for **all** of it, promotes, and only then asks — so every question in a
   file measures the same corpus, which is exactly what `wait_for_seed` cannot
   guarantee across scenarios. Same reduction, same metrics map, same baseline
   vocabulary; own file kind, because a scenario's fields and a corpus's fields
   are half inert in each other (ADR-0046 decision 10, applied a second time).
   `RESERVED_PREFIX` becomes a list and gains `qa_`, and `tokens_per_answer`
   joins `RESERVED_METRICS`, so a scenario category cannot collide with an axis
   this suite produces — the guard EVAL-2 needed for the same reason
   (`scenario.rs`'s `is_reserved`).

5. **Every question declares what it needs from the embedder, and that is what
   splits the two paths.** `needs: "lexical"` for a question whose wording
   shares terms with the material — reachable through the sparse leg on any
   stack — and `needs: "semantic"` for a paraphrase that shares none, which only
   a real embedding model can reach. It is CTX-1's own fixture construction
   (ADR-0024 decision 8: paraphrase docs share no query terms, so the sparse leg
   cannot see them) lifted from the engine to the product. On the deterministic
   run the semantic questions are **skipped and counted in the report**, never
   scored zero: a question the configured path structurally cannot answer is not
   a regression, and scoring it as one would train the next person to delete it.

   **Amended 2026-07-31, first green run.** The declaration is necessary and
   not sufficient, and the first version of both `semantic` questions proved
   it by *passing*. Written without a caller-side budget they composed
   against the pack's own 1,500, the block carried all twelve records, and
   their answers arrived whether the dense leg had ranked them or not — a
   reachability check wearing a retrieval question's name. A question that
   measures ranking must also be **bound**: something has to have made a
   choice. Bound at 120 tokens one of the two now fails against real BGE-M3,
   and that failure sits in `evals/baseline-retrieval.json` rather than being
   tuned away. The same run showed *where* a bound question may ask: scopes
   are placed nearest-first and totally ordered (ADR-0025 decision 5), so a
   bound block spends itself on the near end and never reaches the far one,
   and a narrow-budget question about department or org material measures
   distance rather than relevance. Every ranking question in the corpus
   therefore asks about the reader's own leaf, which the gradient forces
   rather than the corpus choosing.

   **Also amended: the index-readiness wait, which took three attempts.**
   Waiting through an inject probe became unsatisfiable the moment the demo
   narrowed the budget below what the far end of the chain needs — the wait
   burned its whole timeout and reported an indexing failure for what was a
   composition change. Moving it to `POST /v1/recall`'s query form then
   failed for every *promoted* record, which is the useful finding: a
   promotion publishes a channel that names a record at its current address
   (ADR-0034 decision 3) and the record never leaves its author's leaf, so a
   reader composes it through the published channel while a query-shaped
   recall — which searches the scopes the caller may read — does not reach
   it. The check now runs before any climb and asks each record's own
   author. The sparse index is one per tenant (ADR-0024 decision 3), so
   readiness established there is readiness full stop.

6. **Two baselines, split by what a number depends on.** The composition axes
   ride `evals/baseline.json` beside EVAL-1's and EVAL-2's. The dense-leg axes
   ride `evals/baseline-retrieval.json` on a run that brings TEI up — EVAL-2's
   two-file shape (ADR-0046 decision 12) at the other model seam, and for the
   same reason: one file holding two incomparable sets of numbers invites the
   comparison. **Unlike the live-extraction half, the retrieval half does go on
   the nightly.** ADR-0028 decision 6's objection was that a nightly failure
   should mean someone changed the code rather than that a model drifted, and
   that does not hold here: BGE-M3 is served locally from an image tag and a
   model id written in `deploy/compose/docker-compose.yml`, so it changes when
   someone edits that file — which is someone changing the code. A hosted alias
   is the thing that drifts underneath you; a pinned local embedding model is
   not.

7. **The gate moves onto the pull-request path — ADR-0028's own reversal trigger,
   fired by name.** `ci.yml` gains an `eval` job that starts compose Postgres,
   runs the whole deterministic suite — EVAL-1's scenarios, EVAL-2's extraction
   corpus, EVAL-4's Q&A corpus — against `evals/baseline.json`, and fails the
   pull request on a breach. The other four jobs stay database-free and fast, so
   the toll is one parallel job rather than a change to how anything else runs.
   One baseline still covers the deterministic path, which is why this does not
   need a third file. The retrieval half stays nightly: TEI is a ~2.3 GB model
   download, and that cost is exactly what makes a per-PR job the wrong place
   for it.

8. **`tokens_per_answer` is the axis the AC is actually asking for.**
   "Tokens-per-inject trend" is `tokens_mean`, which EVAL-1 already bounds, and
   a trend is not a gate. What a composition change makes worse is the exchange
   rate: block tokens spent per expected record actually served at the body
   tier. A narrowed budget, a closed channel rule, a moved demotion threshold, a
   drifting estimator — every one of them moves it, and none of them moves
   `tokens_mean` in a way a reader can attribute. It is a ceiling, and
   deliberately a tighter one than `tokens_mean`'s: a fixed corpus through a
   deterministic composition has no jitter to absorb, so 320 against 258
   measured catches a quarter's worth of regression where EVAL-1's "roughly
   twice what this suite measures" would not.

   **Amended 2026-07-31, first green run.** `retrieval_precision` reads only
   the questions whose block was **bound** — it carried fewer records than the
   reader is served — and that predicate is measured rather than declared. Two
   earlier attempts were worse. Over every task-carrying question it read
   0.097, which is not precision at all but corpus size: a block that carries
   everything made no ranking decision, and the number would move whenever a
   fixture was added. Over questions that *declared* a caller-side budget it
   was better but still a corpus author's judgement call, opt-out-able by
   mistake. The reader's own post-climb sweep gives the exact denominator
   instead, and it costs one call. Note what `bound` does and does not
   distinguish: a block can carry less because the budget ran out or because
   retrieval offered fewer candidates, and this axis treats those the same,
   because either way a choice was made and precision is about the choice.

9. **`qa_body_rate` beside `qa_answer_rate` is ADR-0041's parked number.**
   Answer rate counts an expected record that reached the block at any tier;
   body rate counts the ones that arrived whole. Body ≤ answer always, because
   the index tier names what it could not carry (ADR-0041 decision 13). The gap
   between the two is "the index tier displacing bodies that mattered" — the
   exact trigger ADR-0041 recorded against option 4's separate index budget,
   expressed as two gated numbers rather than as a judgement someone has to
   make. Both reduce per scope tier as well (`qa_scope_user`, `qa_scope_team`,
   `qa_scope_department`, `qa_scope_org`), so a gradient regression fails naming
   the tier it happened at instead of being averaged across the corpus — the
   rule ADR-0039 decision 14 established for categories.

10. **The estimator bias and the staleness distribution are measured and
    reported, and gated by nothing on the first run.** `estimator_bias_p95` —
    the block's `ceil(chars/4)` estimate over a real tokenizer's count of the
    same text, for one named model — and `staleness_p50_permille` over served
    records both land as report fields with no bound. ADR-0046 decision 13's
    rule: a target invented before any measurement is a wish, and this repo's
    own precedent (GRPH-2 asserted precision and reported recall) is to report
    first. The bias number is declared model-specific in the report, which keeps
    ADR-0025 option 2's objection rather than arguing with it. This ADR is
    amended with both measurements when the first green run produces them.

    **Amended 2026-07-31, first green run.** `estimator_bias_p95` reads
    **0.69–0.73** across runs: `ceil(chars/4)` is roughly seven tenths of what
    `o200k_base` counts for the same block. ADR-0025 parked "EVAL-4 measures
    the bias" and that is the number, with the consequence stated plainly — a
    block sized to fit a real harness's budget can overrun it by something
    like forty per cent, and that is the direction that matters, because the
    estimator is what CTX-2 uses to decide an entry fits. It stays ungated for
    the reason above; what it now supports is a decision about which harness
    the product owes token accuracy to, and ADR-0025's reversal trigger is
    live rather than hypothetical. `staleness_p50_permille` reads **1000** and
    says nothing yet: every record in this corpus is fresh, so the axis is
    measuring an absence of decay rather than the heuristic. Validating MEM-6's
    score needs a corpus with age in it, which is the real precondition
    ADR-0040 reversal trigger (b) was waiting on and did not say.

11. **CTX-6's compression clause is scoped out with a trigger, not dropped.**
    An axis for an unbuilt feature is either permanently absent — which EVAL-1's
    coverage-loss guard makes a breach on every run — or permanently zero, which
    reads as coverage. It is recorded in the acceptance criteria as deferred and
    as a reversal trigger below: CTX-6 lands → the Q&A corpus grows a
    compression phase and `compression_fidelity` (the same questions, answered
    from a compressed session) joins the baseline.

12. **The demo is a real composition change and the gate fails naming it.**
    EVAL-2's precedent, with the knob this feature is about: `budget_tokens`
    narrowed at a department through the governed pack path, on a fresh tenant,
    with every other input identical. The seed §4.4 gradient (ADR-0025
    decision 5) places department material furthest from the reader, so it is
    what a smaller budget stops buying: `qa_scope_department` falls first,
    `tokens_per_answer` rises, and the gate names the axis, the baseline, the
    measurement and the delta. A tenant per phase, for ADR-0028 decision 7's
    reason as EVAL-2 rediscovered it — two phases on one tenant would compare
    two corpora and blame the budget.

## Options considered

1. **Reuse CTX-1's `crates/synveda-retrieval/tests/fixtures/quality/` as the
   corpus** — one corpus, three readers, and EVAL-2's discipline extended. It
   already ships docs, queries and relevance judgments, and the live variant
   already reads it while ignoring the `topic_mix` vectors. Rejected: it is
   engine-shaped (documents inserted directly, vectors supplied by the fixture)
   and EVAL-4's is product-shaped (transcripts through observe, records
   promoted, blocks composed under a budget), so half the fields would be inert
   in each — ADR-0046 decision 10's argument. And the two existing readers are
   deliberately *partial*, so EVAL-2's both-readers-refuse-unknown-fields guard
   could not be reproduced; a corpus with three readers and no guard is the
   failure that guard exists for. The relationship is recorded instead: the
   engine's recall@6 over synthetic geometry and the product's answer rate over
   a governed corpus answer different questions and are not comparable numbers.
2. **Grade by marker containment, as EVAL-1 does** — no join, no sweep, less
   code. Rejected per decision 2: it cannot distinguish a demotion from an
   absence, which is the measurement CTX-4 parked here.
3. **Arrange the per-scope corpus by registering actors at the department and
   org nodes** — one line in `evals/lib.sh` and no promotion machinery.
   Rejected per decision 3: their writes land at a `ScopeKind::User` leaf under
   the anchor, which no sibling's chain contains and the privacy floor excludes
   anyway. It looks like it works right up until a reader is asked to find the
   material, and then it measures the wrong thing quietly.
4. **Put the whole suite, live TEI included, on the pull-request path** — the
   strongest reading of "before merge", and it would gate the dense leg too.
   Rejected: a 2.3 GB model download per pull request, for axes that a
   *retrieval* change moves — rarer than a composition change, and caught within
   a day by the nightly. Decision 7 takes the half that is cheap and the half
   the AC is about.
5. **One baseline file holding both paths** — fewer files, one place to look.
   Rejected for ADR-0046 decision 12's reason, restated: two incomparable sets
   of numbers in one file invite exactly the comparison the split exists to
   prevent.
6. **Gate `estimator_bias_p95` immediately against a chosen tokenizer** — the
   obligation ADR-0025 parked, discharged with a gate rather than a number.
   Rejected: the bias is model-specific by construction (ADR-0025 option 2's own
   rejection reason), so gating it would bind the product to one harness's
   vocabulary. Reported first; a floor arrives with a decision about which
   harness the product owes token accuracy to, which is an ADPT question.
7. **A time-series store for the tokens trend** — "trend" read literally, with a
   database and a chart. Rejected: the nightly already uploads `report.json` per
   run, so the history exists; a web UI is CNSL-4's (Phase 3), which is the
   reading ADR-0046 decision 11 made for the extraction dashboard and there is
   no reason to make a different one here.
8. **Score a semantic question zero on the deterministic path rather than
   skipping it** — simpler, and no `needs` field. Rejected: it makes a gate fail
   for a reason no code change can fix, and the first person to fix it deletes
   the question. Skipped-and-counted keeps the corpus honest in both directions.
9. **Add a `GET /v1/blocks/{hash}` or similar so the harness can re-read a
   composed block** — would make grading a pure function of the watermark.
   Rejected on ADR-0046 option 2's grounds: a governed route built because an
   eval wanted it. The response already carries what is needed.

## Consequences

- Positive: six parked obligations across five ADRs get the measurement each was
  waiting on — CTX-1's real-model quality target, CTX-2's estimator bias,
  CTX-4's displacement number, MEM-6's staleness heuristic, CTX-3's evidence
  condition, and EVAL-1's pull-request trigger. A composition change's quality
  effect becomes consequential rather than merely observable, which is what the
  AC asks for and what "evaluation is a functional requirement" (ADR-0028) has
  to mean once there is something to gate. Per-scope answer rates make FLOW-5's
  promotion a measured product property rather than one test's. Grading by
  record identity is a stronger predicate than either existing suite's, and it
  is the one that makes a tier measurable at all.
- Negative / accepted trade-offs: every pull request now needs Postgres and a
  gateway build, which is the toll ADR-0028 declined for EVAL-1's thin suite and
  the AC asks for here; the privileged half of the harness grows approvers and
  readers; a promotion inside the seed phase means a *promotion* failure reads
  as a quality failure on the axes, which is correct and still something a
  reader has to diagnose from the report rather than from the number; semantic
  questions are unmeasured on the pull-request path by construction, so that
  gate covers composition and the sparse leg and not the dense one; TEI on the
  nightly makes it slower and adds a failure mode — a model download — that is
  not a product regression; and floating-point differences across runner
  architectures are absorbed by declared `slack` rather than eliminated, which
  is a tolerance someone has to keep honest.
- Reversal triggers: **(a)** CTX-6 lands → the compression phase and
  `compression_fidelity` join the corpus and the baseline (decision 11);
  **(b)** `estimator_bias_p95` exceeds what budget headroom absorbs →
  ADR-0025's per-adapter tokenizer behind the estimator seam, which is that
  ADR's own trigger discharged with a number; **(c)** `qa_body_rate` falls while
  `qa_answer_rate` holds → ADR-0041 option 4's separate index budget;
  **(d)** the staleness distribution shows readers served material they should
  not have acted on → ADR-0040 option 7's `[stale]` marker with its budget cost
  accepted, and option 9's retrieval-time penalty reconsidered; **(e)** the
  pull-request job's wall clock gets long enough that people wait on it → split
  it, composition on the PR path and extraction back to the nightly, which needs
  the third baseline file decision 7 declined; **(f)** EVAL-3's benchmark
  corpora land → they arrive in this Q&A format or the reason they cannot is
  recorded, because a third corpus format would be the point at which the
  vocabulary stopped being shared; **(g)** the promoted-record asymmetry
  found by decision 5's amendment — a reader composes climbed material
  through the published channel but a query-shaped `POST /v1/recall` does
  not reach it, because the record never left its author's leaf — becomes a
  question someone actually asks (a reader shown material in a block wanting
  more like it) → CTX-5 decides whether the query universe should follow
  published channels, with CNSL-4 as the surface that would want it.
  Recorded rather than fixed here: changing a shipped read surface to suit
  an eval is the shape of change ADR-0046 option 7 already refused.

## Compliance notes

- **PDP**: unchanged and unbypassable. The harness keeps the empty dependency
  set (`check-crate-deps.mjs`) and reaches the stack only through `/v1/observe`,
  `/v1/recall`, `/v1/proposals`, `/v1/proposals/{id}/approve` and `/v1/inject`,
  each with an actor's own bearer. The promotions in decision 3 are real
  proposals approved by real approvers under the target scope's own approval
  matrix (FLOW-3, ADR-0032) — a test policy pack may set the requirement, and
  nothing skips the review. Every number reported is one the governed path
  produced (seed §2.2). No new PDP action, no new permit, no pack version bump.
- **Tenancy**: one scratch database and a fresh tenant per run (ADR-0028
  decision 7), and a fresh tenant per demo phase (decision 12). Readers,
  writers and approvers are dev-mode identities (ADR-0008) placed by
  `evals/lib.sh`; the runner never names a tenant, because the token does.
- **Audit**: no new action types. The suite drives `memory.observed`,
  `context.recalled`, `context.injected` and FLOW-3/FLOW-5's proposal and
  publication events, all of which already chain — an eval run stays
  indistinguishable from the sessions and reviews it imitates, which is correct,
  because it is those sessions and those reviews.
- **Secrets**: Q&A fixtures are documentation-only under the MEM-2/MEM-3
  discipline — no credentials, real or synthetic-but-live-format.
- **Observability** (DoD #3): the runner is a client and emits its timings into
  the report, as EVAL-1 and EVAL-2 do. `synveda_tokens_per_inject` and the
  per-tier companion ADR-0041 decision 14 added remain the operational view and
  are unchanged; this suite measures the same quantity from the outside, which
  is the point.

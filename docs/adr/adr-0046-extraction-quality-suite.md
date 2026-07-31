# ADR-0046: The extraction quality suite — the recall sweep as the lens, one corpus with two readers, and a gate with declared slack

- **Status**: Proposed
- **Date**: 2026-07-30
- **Feature(s)**: EVAL-2
- **Deciders**: sujitn

## Context

EVAL-2 is "labelled transcript fixtures → precision/recall per memory class;
hallucinated-memory rate (HaluMem-style). AC: dashboard; gate on regression
>2pts." Like EVAL-1 it arrived without usable acceptance criteria — "dashboard"
names no artefact, "2pts" names no axis — so this ADR writes them
(recorded in SYNVEDA_FEATURES.md and docs/backlog/EVAL-2.md) as well as
deciding the shape. That is the EVAL-1 precedent (ADR-0028), applied for the
same reason: a gate whose threshold nobody wrote down is a dashboard.

Forces at play:

- **Five features have parked something here, and each is a decision waiting
  on a number.** MEM-3 (ADR-0022 decision 8) asserts a *provisional* ≥0.8
  macro precision and says "EVAL-2 owns the real target, dashboard, and
  calibration"; MEM-2 (ADR-0021) sends redaction ruleset precision/recall
  here as the recorded trigger for an ML pass behind the `Ruleset` seam;
  MEM-5 (ADR-0039 decision 6) makes "EVAL-2's measurement is its trigger"
  the condition for the model-backed dedup judge; GRPH-2 (ADR-0044) gates
  fuzzy entity resolution on "EVAL-2 producing a corpus bigger than a fixture
  file". None of those unblock without a measurement someone can act on.
- **The harness may not link a Synveda crate.** ADR-0028 decision 1 holds
  `synveda-eval` to an empty dependency set in `check-crate-deps.mjs`,
  because "an eval that can link the store can seed and read around the PDP
  and would then report quality the product cannot deliver" (seed §2.2). Any
  lens EVAL-2 picks has to be an HTTP surface the product already exposes.
- **Extraction quality is a property of a record *set*, not of a block.**
  EVAL-1 grades by string containment over the composed inject block. That
  cannot express "the pipeline produced four records, three of which were the
  right class": a block is budget-bounded and relevance-ranked, so a record's
  absence means "did not fit or did not rank", never "was not extracted".
- **The deterministic extractor cannot hallucinate, and the nightly runs
  deterministic.** ADR-0028 decision 6 fixes the deterministic extractor and
  embedder as the nightly's configuration so "a nightly failure should mean
  someone changed the code, not that a model drifted". A rule-based extractor
  copies spans; a hallucination axis measured against it is either identically
  zero or a genuine code regression. Which of those it is has to be decided
  deliberately rather than discovered.
- **Extraction quality is noisy in a way EVAL-1's axes are not.**
  `Baseline::updated` gives cost ceilings 1.5× headroom and quality floors
  none, on the stated grounds that EVAL-1's suite is "small enough that any
  scenario failing is a regression, not noise". A per-class precision figure
  over a labelled corpus — especially against a live model — does not have
  that property, and the feature's own AC asks for a 2-point tolerance.

## Decision

EVAL-2 measures extraction over the **real product path** by seeding
transcripts through `/v1/observe` and enumerating what the pipeline produced
through **`POST /v1/recall`'s sweep form** — the bare-`as_of` shape that
returns every record the caller may read, each carrying its `class`, full
`content`, and `provenance`. The corpus is one set of files read by both the
harness and MEM-3's unit test. The gate is EVAL-1's committed baseline,
extended with a declared per-metric slack so the AC's "regression >2pts" is a
number in a reviewable file rather than a rule in code.

Decisions, specifically:

1. **The lens is `POST /v1/recall`'s sweep, not the inject block.** A recall
   with neither `ids` nor `query` but an `as_of` instant is
   `ComposeRequest::sweeping` (ADR-0042 decision 14) — "everything I may read,
   as it stood then". Each `RecallEntry` carries `record_id`, `scope_id`,
   `channel`, `kind`, `class`, `sensitivity`, the **untruncated** `content`
   (ADR-0041 decision 7), `provenance`, the valid window, `object_hash`, and
   staleness. That is precisely the per-record structure per-class precision
   and recall require, and it already exists: EVAL-2 adds no route, no action
   type, and no PDP surface. The harness grows one client method beside
   `observe` and `inject`, and the empty dependency set is untouched.

2. **Fixtures are partitioned one actor per group, and the corpus is sized to
   the product's own cap.** A sweep is bounded by `limit`, defaulting to and
   capped at `MAX_RECALL_IDS` (32). Records land at the caller's home scope
   (ADR-0020) and a service identity is placed as a `ScopeKind::User` leaf
   under its anchor (ADR-0018 decision 2), so one actor per fixture group
   isolates each group's corpus. Note the mechanism, because CTX-5 widened the
   universe past the chain: a sibling actor's home *is* an occupied scope the
   plan considers, and what excludes it is the PDP's privacy floor — no pack
   opens another principal's personal scope (ADR-0037/ADR-0038) — rather than
   the candidate set, which is how inject would have excluded it (ADR-0024).
   Isolation is therefore a policy property this suite depends on, which makes
   it worth asserting rather than assuming: a group whose sweep returns
   another group's records is a leak, and the suite fails on it by name.
   Growing the corpus means registering more actors in `evals/lib.sh`, never
   raising a product cap for an eval's convenience.

3. **A sweep that returns exactly `limit` records is refused as a
   measurement.** `RecallResponse.truncated` reports the *scope* cap
   (`universe.truncated`), not the record cap — a sweep that returned 32 of 40
   records reports `truncated: false`. The consumer cannot distinguish a full
   answer from a truncated one, so the harness treats `entries.len() == limit`
   as unmeasurable and fails the group by name. This is AUD-2's rule ("a
   bounded answer presented as a complete one is the one failure this surface
   cannot afford") applied from the consumer's side, because the surface does
   not apply it here. That gap is recorded as a forward obligation below
   rather than fixed under this feature.

4. **Two lenses, two questions, and the difference between them is a
   reported number.** The sweep answers *what a reader is served*; a
   tenant-wide `auditor` reading `GET /v1/audit/events?action=memory.extracted`
   answers *what the pipeline committed* — the payload carries `records`,
   `classes`, and `merged` per event. They are not the same question: `admit`
   applies tiers, channels, retention horizons and the valid-window predicate,
   so a record MEM-5 superseded at write time is committed and never served.
   The gated axes are computed from the **sweep**, because that is the product
   claim; the audit counts ride the report as an attribution column, so a
   recall shortfall caused by a dedup merge or a horizon is visible as itself
   rather than absorbed into "the extractor missed it". The auditor is a
   dev-mode subject with a tenant-wide `auditor` binding, never a service
   identity — AUTH-3's confinement forbid denies the tenant plane to those
   however bound (ADR-0045). This makes EVAL-2 the first consumer of AUD-2's
   query surface outside AUD-2's own tests, which is the useful kind of
   validation.

5. **Matching is a declared one-to-one assignment over the existing
   predicate.** A produced record matches an expected entry iff
   `produced.class == expected.class` and `produced.content` contains
   `expected.content_contains` — MEM-3's predicate, unchanged. Assignment is
   greedy in file order and one-to-one in both directions: without that, one
   produced record can satisfy three expectations and inflate recall to
   nonsense. Per class C: precision = matched(C) / produced(C), recall =
   matched(C) / expected(C); macro is the unweighted mean over the classes the
   corpus actually exercises, so a class with no fixtures is absent from the
   report rather than counted as 1.0 or 0.0.

   **Amended 2026-07-31, first live run.** This predicate is sound for the
   deterministic path and **unsound for the live one**, and the first live
   measurement is what showed it. `content_contains` works against a
   span-copying extractor because the source text survives into the record
   verbatim; a model paraphrases, so the same labels measure lexical
   agreement rather than semantic correctness. Against `claude-opus-4-8`
   the corpus reported macro precision 0.820 and recall 0.783, and reading
   all fifteen unmatched records one by one, **not one is a fabrication**:
   `epsilon-fact-beyond-truncation` produced both facts *including* the one
   past the 300-character truncation the fixture exists to predict a model
   would reach, and scored a double miss for writing "acts as the backstop
   for tenant isolation" where the expectation said `store.rls.denied`;
   `beta-procedure-and-fact-windows` reached its second claim and scored
   zero over "a lock on **its** binary" against "lock on **the** binary";
   six more are class disagreements on genuinely ambiguous ground truth
   (episode vs fact for a tool result, entity vs fact for a definition);
   and five are additional true records the corpus does not label, one of
   them a split into a decision plus the fact enforcing it that is
   arguably the better extraction. The live axes are therefore left
   unbounded (decision 12's "the first live run writes them" is answered
   with "not these numbers"), and the corpus needs a predicate that
   accepts paraphrase — several accepted phrasings per expectation, a
   normalised-key match, or option 6's judge — before they mean anything
   on that path. `hallucination_rate` is the exception and is bounded at
   zero, because bait is an *absence* predicate: a model rephrasing what
   the transcript does say cannot trip it. The deterministic gate is
   unaffected, because there the predicate is exactly right.

6. **The hallucination axis is fixture-declared bait, gated at zero, and
   honest about what bait cannot catch.** Each fixture may declare
   `must_not_extract` — phrases a hallucinating extractor would plausibly
   produce from that transcript and which the transcript does not support.
   `hallucination_rate` is bait hits over fixtures carrying bait, with a
   `max: 0.0` bound. Against the deterministic extractor this axis is
   currently zero **by construction** — a span-copying extractor cannot invent
   — and that is exactly why it is worth gating: the assertion is "the
   deterministic path cannot fabricate", and a future templating or
   summarisation step that breaks it fails the nightly. What bait cannot catch
   is invention the fixture author did not anticipate; for that, the report
   lists every produced record that matched no expected entry, as a review
   queue rather than a score. A lexical grounding ratio was considered and
   rejected (option 5).

7. **One corpus, two readers, and a format change breaks both.** The labelled
   set moves to `evals/fixtures/extraction/*.json`, one file per fixture group.
   The harness reads the directory at runtime; MEM-3's
   `crates/synveda-ingest/tests/extraction_precision.rs` reads the same
   directory relative to `CARGO_MANIFEST_DIR`. Both deserialize the **full**
   format with `deny_unknown_fields`, so a field added for one reader cannot be
   silently ignored by the other — EVAL-1 decision 2's rule, applied across the
   seam. This is a data dependency, not a crate dependency: the empty set
   holds.

8. **The targets are not shared, because they have different jobs.** The
   ingest test keeps an asserted floor as a fast, hermetic, no-stack tripwire
   on the extractor function; `evals/baseline.json` holds the gate on the
   product path. Two numbers, two purposes, and the ADR says which is which so
   a later reader does not "unify" them.

9. **`Bound` gains an optional `slack`, used only by `--update-baseline`.**
   The AC's "gate on regression >2pts" is expressed as a floor written at
   `measured − slack`; the gate itself is unchanged and still compares against
   `min` exactly as committed. Metrics that declare no `slack` keep EVAL-1's
   zero-tolerance behaviour byte-for-byte, so every existing baseline entry and
   every existing test is unaffected. The tolerance lives in the committed
   number where a reviewer sees it, not in a comparison nobody reads.

10. **Extraction fixtures are a second suite, not a stretched scenario.**
    `evals/scenarios/*.json` declares actors, a seed, a probe and block
    expectations; an extraction fixture declares a transcript and an expected
    record set. Forcing both into one struct leaves half the fields inert in
    each mode, which is the failure `deny_unknown_fields` exists to prevent, in
    spirit. `synveda-eval run` gains `--fixtures evals/fixtures/extraction`,
    reduces both suites into the **same** metrics map, and gates them against
    the **same** baseline — the gate vocabulary stays unified, which is the
    part that matters, while each format says only what it means.

11. **The dashboard is the report, and the report is per class.** `Report`
    gains an extraction section: per class, produced / expected / matched /
    precision / recall; the macro figures; the bait outcomes; the unmatched
    record list; and the audit-attributed committed counts from decision 4.
    The stderr summary prints the per-class table. The nightly workflow already
    uploads `report.json` as an artefact and needs no change. A web UI is
    CNSL's (CNSL-4, Phase 3) and this feature does not start one — recorded
    here so the reading is not re-litigated later.

12. **The nightly gates the deterministic path; the live model is measured on
    demand against its own baseline.** `SYNVEDA_EXTRACTOR=claude` runs the
    identical corpus through `make eval-extraction-live`, reporting into the
    same metric names but gating against `evals/baseline-live.json` — a
    separate file, because deterministic and live numbers are not comparable
    and one file holding both invites exactly that comparison. It is not on the
    nightly: it costs money per run, it needs a key CI does not hold, and a
    gate that pages on model drift is the one ADR-0028 decision 6 already
    refused. Reproducibility comes from the chain rather than from the request:
    `provenance.model_version` records the model the API **served**
    (`ClaudeExtractor` sets it from the response, not from the requested
    alias), so the report states which model produced the numbers and a
    baseline can be keyed to it.

13. **This ADR pre-registers one target and refuses to invent the other.**
    Deterministic macro precision ships at **≥0.90**, raising MEM-3's
    provisional 0.8 on the strength of the 0.958 already measured — that
    discharges "EVAL-2 owns the real target" with a number rather than a
    promise. Macro **recall has never been measured anywhere in this product**,
    so no floor is pre-registered: the first green run writes it via
    `--update-baseline` and the ADR is amended with the measurement. A floor
    invented before any measurement is a wish, and the repo's own discipline
    (GRPH-2 asserted precision and *reported* recall for exactly this reason)
    says to report it first.

    **Amended 2026-07-30, corpus step.** The corpus exists —
    `evals/fixtures/extraction/`, 5 groups, 50 fixtures, 54 expectations,
    7–13 per class — and the deterministic extractor measures **macro
    precision 0.983, macro recall 0.914** over it, per class:

    | class | precision | recall |
    |---|---|---|
    | decision | 9/9 = 1.000 | 9/10 = 0.900 |
    | entity | 7/7 = 1.000 | 7/7 = 1.000 |
    | episode | 8/8 = 1.000 | 8/8 = 1.000 |
    | fact | 9/10 = 0.900 | 9/13 = 0.692 |
    | preference | 8/8 = 1.000 | 8/9 = 0.889 |
    | procedure | 7/7 = 1.000 | 7/7 = 1.000 |

    The ≥0.90 precision floor stands with headroom. Recall is still
    **reported and not asserted** here, because this is the extractor-level
    number and the gate belongs on the product path — but it is now a
    measurement rather than an absence, and the shape of it is worth
    recording: `fact` recall is the outlier at 0.692 for two named reasons,
    both structural rather than accidental. The multi-claim fixtures state
    two true things in one utterance and the deterministic ruleset emits one
    record per event, and `epsilon-fact-beyond-truncation` puts a true claim
    past the 300-character truncation the ruleset calls a summary. Neither is
    a bug to fix in the rules; both are exactly what the model-backed
    extractor is the path to reaching, which is what makes them worth keeping
    in the corpus rather than trimming out of it. The single unmatched
    record — `beta-preference-tabs-implicit`, an implicit preference carrying
    none of the ruleset's marker phrases — is the one honest classification
    miss, and it reads correctly from both sides at once: a point off `fact`
    precision and a point off `preference` recall.

## Options considered

1. **Parse the inject block's rendered text** — the block already renders
   `- [{class}] {content}`, so a class-labelled record set is sitting there and
   the harness needs no new call. Rejected: the block is bounded by the pack
   budget and ordered by relevance, and CTX-4 elides demoted entries to an
   index line. Absence would mean "did not fit, did not rank, or was
   summarised away", never "was not extracted" — the measurement would be of
   composition, which is EVAL-4's.
2. **A new `GET /v1/records` enumeration route** — the honest surface for
   "list what exists", with a clean shape and no caps to work around.
   Rejected: it is a governed route with a PDP action and an audit event,
   built because an eval wanted it, in a product where the memory browser
   (CNSL-4) is the feature that owns that question. Decision 1 gets the same
   answer from a surface two features already needed.
3. **Leave the measurement in `crates/synveda-ingest`'s test and gate it
   there** — fastest, hermetic, no stack, and the fixtures are already
   there. Rejected as the *whole* answer, kept as half of one (decision 8):
   it measures the extractor function rather than the pipeline (no redaction
   re-scan, no dedup, no policy re-decision at commit), it cannot be pointed
   at a deployment, and `cargo test` has no vocabulary for "2 points worse
   than last time". It stays as the fast tripwire.
4. **Give `synveda-eval` a store dependency, for measurement only** — direct
   reads of `records` would end every ambiguity in decisions 2, 3 and 4 at a
   stroke. Rejected on ADR-0028 decision 1 and seed §2.2. The ambiguities are
   the price of measuring only what the product will actually serve, and that
   is the price worth paying.
5. **A lexical grounding ratio as the hallucination metric** — score each
   produced record by the fraction of its content words present in the source
   transcript, threshold it, and gate. Rejected: a legitimate summary reads as
   ungrounded and a plausible recombination of source words reads as grounded,
   so the number would be noisy in both directions and would then be argued
   with instead of acted on. Bait (decision 6) is unambiguous where it fires
   and silent where it does not, and the ADR says which.
6. **A model-backed judge for grounding, HaluMem-style** — what the benchmark
   actually does, and better than bait at the cases bait misses. Deferred for
   MEM-5's reason (ADR-0039 decision 6): the AC must pass hermetically, and a
   judge whose own precision nobody has measured should not be the thing that
   decides whether the product regressed. The seam is decision 6's unmatched
   list — that is the labelled set a judge would be evaluated against.
7. **Raise `MAX_RECALL_IDS`, or add a record-level `truncated` flag, so the
   sweep can carry a bigger corpus** — a small honest change, and the flag is
   arguably a real gap in CTX-5 (option 2's own words: "a bounded answer
   presented as a complete one is the one failure this surface cannot
   afford"). Rejected *for this feature*: changing a shipped read surface to
   suit an eval is the shape of change this repo refuses, and decision 3's
   consumer-side rule gets a correct measurement without it. Recorded as a
   forward obligation with the recommendation, not folded in silently.
8. **Extend the scenario format with an optional extraction block** — one
   suite, one file kind, and EVAL-1's "adding coverage is adding a file to
   `evals/scenarios`" stays literally true. Rejected per decision 10.
9. **Gate the live-model run nightly** — model drift is the thing that most
   plausibly makes extraction quality worse in production, and a gate that
   does not watch it is watching the easy half. Rejected per decision 12, with
   the reversal trigger recorded below.

## Consequences

- Positive: the four parked decisions (MEM-2's ML pass, MEM-5's model judge,
  GRPH-2's fuzzy resolution, MEM-3's real target) get the measurement each was
  waiting for, and MEM-3's provisional target is replaced by a committed number
  rather than closed by assertion. Per-class precision and recall are measured
  over the path a customer actually gets — observe → redact → extract → embed →
  dedup → commit → admit → serve — so a regression anywhere in it fails one
  gate. The harness stays architecturally incapable of measuring a path the
  product does not expose. AUD-2's query surface gains its first outside
  consumer. The 2-point tolerance becomes a reviewable number, and every
  existing baseline entry keeps zero tolerance untouched.
- Negative / accepted trade-offs: the sweep's 32-record cap makes corpus size
  an actor-count question, so growing the corpus edits `evals/lib.sh` as well
  as adding files; decision 3 means a group that outgrows the cap fails loudly
  rather than measuring less, which is correct and still a failure someone has
  to go fix. The gated numbers are what a *reader is served*, so they are a
  product measurement rather than an extractor measurement, and the two only
  separate through the audit column. Bait catches only anticipated invention.
  `--update-baseline` with slack can ratchet a floor downward across repeated
  deliberate updates — the same shape ADR-0028 already accepted for ceilings,
  and visible in the diff each time. The live path is unmeasured until someone
  runs it, by choice.
- Reversal triggers: **(a)** a live run shows deterministic and live numbers
  diverging enough that the nightly stops predicting production quality → move
  the live run onto a schedule with its own budget (option 9); **(b)** the
  unmatched-record list from decision 6 grows into a corpus a judge could be
  evaluated on → build the model-backed grounding judge (option 6), which is
  also the trigger MEM-5 and MEM-2 are waiting on; **(c)** a second consumer of
  the recall sweep hits decision 3's ambiguity → add the record-level
  truncation flag to `RecallResponse` as a CTX-5 correction (option 7), at
  which point decision 3's consumer-side rule becomes belt-and-braces;
  **(d)** extraction fixtures outgrow what a human reads in JSON → a fixture
  format, not a bigger JSON (ADR-0028's own trigger, restated).

## Compliance notes

- **PDP**: unchanged and unbypassable. The harness keeps the empty dependency
  set (`check-crate-deps.mjs`), holds no privileged path, and reaches the stack
  only through `/v1/observe`, `/v1/recall` and `/v1/audit/events`, each with an
  actor's own bearer. Every number it reports is a number the governed path
  produced (seed §2.2). No new PDP action, no new permit, no pack version bump.
- **Tenancy**: one scratch database and one fresh tenant per run
  (ADR-0028 decision 7), unchanged. The extraction actors are service
  identities at their own home leaves; the auditor is a dev-mode subject whose
  `auditor` binding is tenant-wide because `AuditRead` declares
  `resource: [Tenant]` and admits nothing narrower (ADR-0045 decision 2).
- **Audit**: no new action types. The suite drives `memory.observed`,
  `context.recalled` and `audit.read` on the chain — an eval run is
  indistinguishable from the sessions it imitates, which is correct, because it
  is those sessions. The audit read is a *read* of the chain the run is
  writing; the harness filters by `action=memory.extracted` and by the
  fixture's session, so its own reads are not folded into its own numbers.
- **Secrets**: fixtures stay documentation-only under the MEM-2/MEM-3
  discipline — no credentials, real or synthetic-but-live-format; `[REDACTED:*]`
  placeholders are corpus content and must survive extraction untouched
  (ADR-0021), which the corpus asserts as a fixture rather than as a rule.
  Bait phrases are subject to the same rule: bait is plausible-but-absent
  *content*, never a plausible-looking secret.
- **Observability** (DoD #3): the runner is a client and emits its timings into
  the report, as EVAL-1 does; the gateway spans for its calls are the ordinary
  observe/recall/audit spans. The pipeline's own
  `synveda_extraction_lag_seconds` and
  `synveda_extraction_rescan_findings_total` are unchanged and remain the
  operational view.

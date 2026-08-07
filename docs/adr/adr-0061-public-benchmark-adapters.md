# ADR-0061: LongMemEval through the governed path — a judge measured before it measures, and a score that names everything it depends on

- **Status**: Proposed
- **Date**: 2026-08-07
- **Feature(s)**: EVAL-3
- **Deciders**: sujitn

## Context

The feature text is one line (SYNVEDA_FEATURES.md:635): LoCoMo + LongMemEval
run end-to-end through Synveda (observe→inject/recall→judge). The AC is
"reproducible scores published in repo; tracked per release", with a
parenthetical that is load-bearing rather than decorative — "Marketing
artefact too — every credible 2026 memory system publishes these."

Unlike EVAL-1, 2, 4 and 5, this feature arrives with most of its design
already written *by other ADRs*. Five deferrals name its judge, and they do
not merely mention it — each states a property the judge must have:

- **ADR-0046 option 6** deferred the model-backed grounding judge and gave a
  reason that is not cost: "the AC must pass hermetically, and a judge whose
  own precision nobody has measured should not be the thing that decides
  whether the product regressed." It then named the seam — decision 6's
  **unmatched-record list** is "the labelled set a judge would be evaluated
  against" — and reversal trigger (b) makes that list growing into a corpus
  the trigger to build the judge.
- **ADR-0047 reversal trigger (f)**: the benchmark corpora "arrive in this
  Q&A format or the reason they cannot is recorded, because a third corpus
  format would be the point at which the vocabulary stopped being shared."
- **ADR-0048 decision 16 and trigger (c)**: the behavioural half of the
  injection suite rides this judge and joins it "against its own
  **model-keyed** baseline".
- **ADR-0039 decision 14**: the `knowledge_update` category exists already,
  written in LongMemEval's *shape* over four hand-written scenarios, and that
  ADR set the honesty bar for this one — "pretending otherwise here would be
  claiming a published benchmark score for a suite of four scenarios."
- **ADR-0053 option 9 / trigger (b)** wants less than it appears to: not the
  `SkillJudge` implementation, but "EVAL-3's harness giving a way to measure
  whether a judge actually predicts anything the lexical rubric does not."

Forces at play:

- **The corpus licences differ, and one of them forecloses the feature text.**
  LongMemEval is MIT (Copyright (c) 2024 Di Wu). LoCoMo's `LICENSE.txt` is
  Creative Commons **Attribution-NonCommercial 4.0 International**, which
  grants rights "for NonCommercial purposes only" and defines NonCommercial
  as not "primarily intended for or directed towards commercial advantage or
  monetary compensation". This feature's own AC says the scores are a
  marketing artefact. Publishing LoCoMo numbers to sell an enterprise product
  is the paradigm case of the use that licence withholds. CLAUDE.md's licence
  rule names MIT/Apache-2.0/PostgreSQL for the core path, and `cargo-deny`
  enforces it — but a corpus is data, not a crate, so nothing in the build
  would have caught this. That is precisely why it is a decision here rather
  than something discovered after publication.
- **The harness cannot cheat, and that is what makes it slow.** ADR-0028
  decision 1 gave `crates/synveda-eval` an empty dependency set, enforced by
  `check-crate-deps.mjs`: it reaches the stack only through `/v1`, with each
  actor's own bearer, through the PDP. Seed §2.2 admits no path around it.
  A benchmark of 500 instances × ~40 sessions cannot be seeded with direct
  inserts, and ADR-0028 option 4 already refused that trade for a far smaller
  corpus.
- **The 32-record sweep cap is a rule, not a limit to raise.** ADR-0046
  decision 3 and ADR-0048 trigger (f) both say the same thing: when a corpus
  outgrows `MAX_RECALL_IDS`, "split it across more actors, never raise the
  limit." EVAL-4's whole corpus is 12 records against that cap. A single
  LongMemEval instance exceeds it.
- **A memory benchmark score is never a measurement of the memory system
  alone.** LongMemEval grades whether a free-text answer matches a reference.
  Synveda does not answer questions; it serves a governed block. Producing a
  number therefore requires a *reader* model that answers from the block and
  a *judge* model that grades the answer — so the published figure is a joint
  property of three things, only one of which is this product.
- **The gate must not page on model drift.** ADR-0028 decision 6: "a nightly
  failure should mean someone changed the code, not that a model drifted."
  ADR-0046 decision 12 built the escape already — `make eval-extraction-live`
  against `evals/baseline-live.json`, off the nightly, with reproducibility
  taken from `provenance.model_version` recording the model the API *served*
  rather than the alias requested.

## Decision

EVAL-3 runs **LongMemEval** end-to-end through `/v1`, grades it in **two
tiers** — a deterministic retrieval gate that blocks a regression and a
model-judged QA score that is published and gates nothing — and **measures
its own judge before that judge is allowed to measure the product**. LoCoMo
is dropped, with its licence recorded as the reason and a follow-on feature
filed for a second corpus.

Decisions, specifically:

1. **LongMemEval only, and the reason LoCoMo is absent is a licence rather
   than an effort estimate.** CC BY-NC 4.0 cannot cover a score published to
   sell a commercial product, and a benchmark run whose result we may not
   quote is a benchmark run with no acceptance criterion. This discharges
   ADR-0047 trigger (f)'s "or the reason they cannot is recorded" clause in a
   register that ADR did not anticipate — the corpus does not arrive in *any*
   format, and the obstacle is legal rather than structural. **EVAL-7** is
   filed for the second corpus with two named paths: written permission from
   Snap Research for commercial benchmark use, or a permissively-licensed
   substitute in LoCoMo's slot. SYNVEDA_FEATURES.md, the Sequencing line and
   the Phase 3 demo goal in CLAUDE.md are amended to match, because a goal
   naming a corpus we may not publish is a goal that cannot be met.

2. **A third corpus format, sharing the reporting vocabulary and not the seed
   or question shape — and here is what trigger (f) was actually protecting.**
   ADR-0047's worry was "the point at which the vocabulary stopped being
   shared", not the file layout. The layout cannot be reused for three
   reasons, each of which is a property EVAL-4 chose on purpose:

   **The predicate differs at the root.** EVAL-4 grades by *record identity*
   and says why: "containment could not tell a demotion from an absence, and
   those are the two answers this suite turns on." LongMemEval grades whether
   a free-text answer is correct against a reference. `expect_records` cannot
   express "is this answer right", and no amount of extending it would.

   **The `needs` guards are rules for a corpus we wrote.** Rules 4 and 5 —
   a `semantic` question may share no content word with its answer, a
   `lexical` one must share at least one — are authoring discipline for a
   hand-built corpus. An external corpus cannot satisfy them, and editing an
   external corpus until it does is the one thing that would invalidate the
   score.

   **Scale breaks the container.** The QA format is one file per corpus with
   one reader; LongMemEval is 500 independent instances with no shared
   universe. One file per corpus would be one file of 115k-token haystacks
   ×500.

   What *is* reused, verbatim: the axis/baseline/`slack` mechanism, the
   floor-and-ceiling gate, skip-and-count for questions the configured stack
   structurally cannot answer, per-category reduction, and `synveda-eval
   check` parsing the corpus with no database. The vocabulary is shared; the
   schema is not.

3. **The judge is a seam with a deterministic default, exactly as `Extractor`
   is.** A `Judge` trait in `crates/synveda-eval`, with the deterministic
   implementation as the default and a Claude-backed implementation selected
   the way `SYNVEDA_EXTRACTOR=claude` already selects one
   (`crates/synveda-ingest/src/extraction/claude.rs` is the shape to copy,
   including its `model_version`-from-the-response honesty). This is also
   what ADR-0053 option 9 asked for — its `SkillJudge` becomes an
   implementation of a seam that exists, rather than a second judge
   abstraction.

4. **The judge is measured before it measures, and its agreement is a
   published number rather than an assumption.** This is the decision the
   other four deferrals are actually waiting on, and ADR-0046 option 6 named
   its labelled set: EVAL-2's **unmatched-record list**. The judge is scored
   against two sets — that list, and LongMemEval's own 500 reference answers
   — and the report carries the judge's agreement rate as a first-class axis
   beside the product's score. A benchmark number produced by an unmeasured
   judge is not a measurement; it is a second opinion with a decimal point.
   No claim EVAL-3 publishes may be tighter than its judge's own agreement,
   and the published artefact states both figures together.

5. **Two tiers, and the split is LongMemEval's own rather than one we
   invented.** The benchmark publishes a retrieval evaluation and a QA
   accuracy, and they have different reproducibility properties:

   - **Deterministic tier — retrieval, and it gates.** Did the block bind the
     evidence sessions the instance names in `answer_session_ids`? That is
     record identity, the predicate EVAL-4 already grades and the one that is
     reproducible from bytes. It runs on the nightly against
     `evals/baseline.json`'s discipline and fails naming the axis, the
     baseline, the measurement and the delta. LongMemEval excludes its 30
     abstention instances from retrieval scoring; so do we, and the exclusion
     is logged rather than silent (decision 7).
   - **Model-judged tier — end-to-end QA accuracy, and it gates nothing.** It
     is the published figure and the marketing artefact, run on demand
     against its own baseline file. It is off the merge path *and* off the
     nightly for ADR-0028 decision 6's reason, restated: a gate that fails
     when a model changes rather than when the code changes is an alarm
     nobody keeps.

   The gate therefore watches the half of the benchmark this product is
   actually responsible for, and the published number states the whole thing.

6. **The baseline is keyed to every model the number depends on, and there
   are two of them.** The score is a joint property of Synveda's block, the
   **reader** model that answers from it, and the **judge** model that grades
   the answer. Both are recorded from what the API *served* rather than from
   the alias requested — ADR-0046 decision 12's mechanism, applied twice —
   and both are keyed into `evals/baseline-longmemeval.json`. The published
   artefact names both alongside the score. A memory benchmark figure quoted
   without its reader model is not reproducible by anyone, including us, and
   the industry convention of quoting one anyway is not a reason to adopt it.

7. **A declared slice on the routine path, the full 500 behind its own
   target, and nothing dropped silently.** This is EVAL-5's shape (ADR-0048:
   400 variants of 11,680 on the pull-request path, the full run nightly),
   and its rule travels with it — a suite that bounds coverage says what it
   bounded. `make eval-longmemeval` runs the deterministic slice;
   `make eval-longmemeval-full` runs all 500; `make eval-longmemeval-judged`
   is the model-judged published run. Every run's report states the instance
   count, the slice interval, the abstention exclusion and the skip count. A
   silent cap reads as "we covered it" when we did not.

8. **One actor per instance; the 32-record cap is not touched.** EVAL-2 set
   the rule and EVAL-4 restated it — a corpus grows by adding actors, never
   by adding records past the arithmetic. LongMemEval's instances are
   independent by construction, which makes this the natural mapping rather
   than a workaround: instance ↔ actor ↔ tenant-scoped leaf.

9. **Everything lands at the `user` tier, and the ADR says so rather than
   inventing governance the corpus does not have.** LongMemEval is one user's
   own chat history; it has no teams, no promotion, no review. Synthesising a
   hierarchy to make it look like EVAL-4's corpus would measure a fiction.
   This is the honest complement to EVAL-4: that corpus measures the
   *governance* axis (four tiers, real proposals, per-tier answer rates) and
   this one measures the *memory* axis (extraction, multi-session reasoning,
   temporal reasoning, knowledge update, abstention) at a single scope. The
   two suites are not redundant and neither subsumes the other.

10. **The abstention instances land on an axis that already exists.**
    EVAL-1 decision 4 made abstention first-class — "of the scenarios that
    must compose nothing, the fraction that did" — because "a memory system
    that invents context is worse than one that stays quiet". LongMemEval's
    30 abstention questions are that axis with an external corpus behind it,
    and they are deterministically gradeable at the retrieval tier: the
    correct block binds nothing.

11. **Published scores live in the repo and are stamped per release.**
    `evals/scores/longmemeval-<version>.json` plus a rendered table in
    `docs/BENCHMARKS.md`, each row carrying the score, the judge's agreement
    rate, both model versions as served, the instance count and the commit.
    "Tracked per release" is a file that accumulates rows, not a number
    somebody edits.

12. **Definition of done items.** Tracing spans on the judged path;
    `synveda_eval_judge_seconds` and `synveda_eval_judge_calls_total`
    labelled by outcome. **No new audit action type**: the harness reaches
    the stack only through existing `/v1` surfaces with actors' own bearers,
    so every event it produces is one an ordinary caller produces. The judge
    calls an external API and touches no Synveda action.

## Options considered

1. **Deterministic grading only, defer the model judge a sixth time.**
   Cheapest, and it would produce reproducible published numbers. Rejected:
   LongMemEval's published figures are model-judged, so deterministic string
   matching yields a number not comparable with anyone else's — which defeats
   the AC's stated purpose — and it re-defers five ADRs that have each waited
   through a phase. Decision 5 keeps the honest half of this option as the
   gate rather than throwing it away.
2. **Build the judge, measure it later.** Faster to a headline number.
   Rejected on ADR-0046 option 6's own words: a judge whose precision nobody
   has measured should not decide whether the product regressed. Decision 4
   is that objection discharged rather than deferred again, and decision 5
   further ensures an unmeasured judge could not have gated anything anyway.
3. **Run LoCoMo internally and publish only LongMemEval.** Considered
   seriously. Rejected: using a non-commercial corpus to improve a commercial
   product is arguably the commercial use the licence withholds, publication
   or not, and the ADR would be recording an accepted legal risk rather than
   avoiding one — for a signal we could never quote.
4. **Extend the EVAL-4 Q&A format to carry LongMemEval.** One format, and
   trigger (f) satisfied literally. Rejected per decision 2: the grading
   predicate differs at the root, and stretching `expect_records` to express
   answer correctness would degrade the surface EVAL-4 deliberately built to
   distinguish a demotion from an absence.
5. **Raise `MAX_RECALL_IDS` so an instance fits one reader.** Rejected twice
   before (ADR-0046 option 7, ADR-0048 trigger (f)) and rejected again for
   the same reason: changing a shipped read surface to suit an eval is the
   shape of change this repo refuses.
6. **Gate the judged run on the nightly.** Model drift is the thing most
   likely to move a published score. Rejected per decision 5 and ADR-0028
   decision 6, with the reversal trigger recorded below — the same
   disposition ADR-0046 option 9 reached for live extraction.
7. **Use one model as both reader and judge.** Simpler, one model version to
   record. Rejected: a model grading answers produced from its own reading is
   a measurement with a known bias and no way to bound it. The reader and the
   judge stay separate even when they are the same model family, and
   decision 6 records both.
8. **Do nothing — leave the `knowledge_update` category as the memory
   evidence.** Fails ADR-0039 decision 14 by name: four hand-written
   scenarios in LongMemEval's shape are not a published benchmark score, and
   that ADR already refused to let them be described as one.

## Consequences

- **Positive**: five deferrals discharge on one capability, which is what
  ADR-0048 decision 16 predicted when it said "one capability, three features
  waiting on it, named in one place". The judge arrives measured, so MEM-5,
  MEM-2, ADR-0048's behavioural half and ADR-0053's `SkillJudge` each get the
  seam *and* the evidence their triggers asked for. The product gains its
  first externally-comparable quality number. The gate watches the half this
  product controls, so a retrieval regression fails a build while a model
  upgrade does not. And the licence finding lands before publication rather
  than after.
- **Negative / accepted trade-offs**: the phase demo goal loses a named
  benchmark and gains a follow-on, which is a smaller claim honestly made
  rather than a larger one we could not defend. The published score depends
  on two external models and can move when neither the code nor the corpus
  did — decision 6 makes that legible rather than preventing it. A third
  corpus format is a third thing to learn, mitigated but not erased by the
  shared reporting vocabulary. The judged run costs money per invocation and
  needs a key CI does not hold, so it will be run deliberately and rarely,
  and the published number will sometimes be older than `main`. Seeding 500
  instances through the governed path is slow by construction; the slice
  exists because of it, and `make eval-longmemeval-full` will be a target
  someone schedules rather than one they wait on.
- **Reversal triggers**: **(a)** the judge's measured agreement is low enough
  that the published score's error bar exceeds the differences we want to
  claim → the judge is the finding, and the corpus of disagreements becomes
  the next feature's input rather than a footnote; **(b)** a published score
  moves between releases with no code change and no corpus change → the
  reader or judge model drifted, and the judged run earns a schedule with its
  own budget (option 6), which is ADR-0046 trigger (a) restated for this
  suite; **(c)** EVAL-7 lands a permissively-licensed second corpus → it
  arrives in *this* feature's format, or the reason it cannot is recorded,
  which is trigger (f) inherited rather than escaped; **(d)** the
  deterministic retrieval tier holds at 1.0 while the judged score stays low
  → the block is binding the right material and the loss is downstream of
  retrieval, which makes it CTX-2/CTX-4's composition problem rather than
  MEM's; **(e)** Snap Research grants written permission for commercial use →
  LoCoMo re-enters through EVAL-7 with the grant recorded beside the corpus;
  **(f)** `make eval-longmemeval-full` grows past the point where anyone runs
  it → bounded concurrency with an ordered result set, ADR-0048 option 8's
  deferral inherited.

## Compliance notes

- **PDP**: unchanged and unbypassable. The harness keeps its empty dependency
  set (`check-crate-deps.mjs`) and reaches the stack only through `/v1`
  surfaces with each actor's own bearer. No new action type, no new permit,
  no pack version bump. Decision 8's one-actor-per-instance mapping is
  ordinary registration through the same path `evals/lib.sh` already uses.
- **Multi-tenancy**: a fresh tenant per run, per ADR-0028 decision 7 — "a
  fresh tenant per run is what makes two runs comparable", which EVAL-2's
  notes found to be load-bearing rather than tidy.
- **Audit**: no new action type. Every event the run produces is one an
  ordinary caller produces on the same surface.
- **Licences**: LongMemEval is MIT and is vendored with its licence file
  intact and attribution preserved. No non-commercially-licensed corpus
  enters the repository. `make check-corpus-licences` asserts that every
  directory under `evals/fixtures/` carrying third-party material has a
  licence file naming a permitted licence — the gap that let a CC BY-NC
  corpus get as far as a feature specification, closed where the build can
  see it.

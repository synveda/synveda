# The Q&A corpus (EVAL-4, ADR-0047)

A corpus planted through `/v1/observe`, climbed to a team, a department and
the org through **real proposals and real approvals**, and then questioned
through the reader's own `POST /v1/inject` block. One reader, one corpus,
many questions.

The lens is the block, which is the exact surface EVAL-2 rejected: ADR-0046
threw it out because it is budget-bounded, relevance-ranked and elides what
CTX-4 demotes, and those three properties are what this suite measures. Here
absence *is* the signal.

## Why the material has to be promoted

Nothing can write to a team, a department or an org node. Observe lands
records at the caller's home scope (ADR-0020), and a service identity's home
is a `principal`-shaped scope **under** its anchor (ADR-0018 decision 2) — so
registering an author "at Engineering" puts its writes on a leaf under
Engineering, which no sibling's chain contains and the privacy floor excludes
anyway. A corpus that spans scope tiers is therefore a corpus that climbed
through review, and a per-scope answer rate is an assertion about FLOW-5 as
much as about CTX-2.

That also fixes where the authors sit. AUTH-3's confinement forbid denies a
service identity every resource outside its anchor subtree, and a climb names
its target scope — so the author of team material is anchored at the team,
the author of department material at the department, and the reviewers at the
org, from which roles inherit downward to every level they review.
`evals/lib.sh` owns all of that.

## Format

One file per **corpus**.

```json
{
  "corpus": "acme-engineering",
  "note": "why this corpus exists, for whoever adds to it next",
  "reader": "qa-reader",
  "seed": [
    {
      "actor": "qa-reader",
      "session_id": "qa:acme:own",
      "tier": "user",
      "events": [{ "key": "own-testing", "kind": "transcript_delta", "text": "…" }]
    },
    {
      "actor": "qa-team",
      "session_id": "qa:acme:team",
      "tier": "team",
      "promote_to": "payments",
      "events": [{ "key": "team-retries", "kind": "decision", "text": "…" }]
    }
  ],
  "questions": [
    {
      "name": "team-retry-policy",
      "note": "optional — why this question is interesting, or what it is expected to miss and why",
      "task": "what does payments cap card retries at",
      "needs": "lexical",
      "expect_records": ["team-retries"],
      "must_not_contain": ["…"],
      "budget_tokens": 120
    }
  ]
}
```

- `tier` is one of `user`, `team`, `department`, `org`, and it is the axis the
  batch reports into (`qa_scope_team` and friends).
- `promote_to` names a hierarchy node as `evals/lib.sh` names it. It is
  required for every tier except `user` and forbidden for `user`, because
  `user` is the author's own leaf and the only tier that needs no review.
- `task` absent is the taskless session start (ADR-0025 decision 5's
  else-branch): no retrieval leg at all, so the question measures the gradient
  and the budget alone.
- `needs` is `lexical` or `semantic` — see below.
- `expect_records` names seed keys. Grading is by **record identity**, never
  by string containment: observe's `event_id` → the sweep's
  `provenance.event_id` → `record_id` → its position in the block's
  `record_ids` and `tiers`. Containment could not tell a demotion from an
  absence, and those are the two answers this suite turns on.

## The rules the guards enforce

All of these run in `synveda-eval check`, which needs no database and no
gateway — `make ci` runs it on every pull request.

1. **Unknown fields are refused, not ignored.** A typo'd expectation that
   reads as "no expectation" is an eval that passes for the wrong reason.
2. **A tier and a promotion must agree.** `user` promotes nowhere; every other
   tier names a `promote_to`, because nothing but a climb can put material
   there.
3. **`expect_records` must name something the corpus seeds.**
4. **A `semantic` question may share no content word with its own answer.**
   If it does, the sparse leg can reach it and the question would pass without
   the dense leg ever working.
5. **A `lexical` question must share at least one.** If it does not, the
   sparse leg cannot reach it and it would fail on the deterministic path for
   a corpus reason rather than a product one.
6. **A taskless question cannot be `semantic`.** It takes no retrieval leg at
   all, so it reaches its answer on any stack.
7. **A question that expects no records measures nothing.**
8. Closed vocabularies for `tier`, `needs` and `kind`; no duplicate corpus
   names, session ids, seed keys or question names.

## Writing a question that measures ranking

Two things are needed, and the first version of this corpus had neither.

**It must be bound.** A question whose block carried everything the reader is
served made no ranking decision — its answer arrived because there was room,
not because anything ranked it. `retrieval_precision` reads only bound blocks,
and the predicate is measured (`block_records < served_records`) rather than
declared, so a corpus cannot opt a question in or out by mistake.

**It must ask about the reader's own leaf.** Scopes are placed nearest-first
and totally ordered (seed §4.4, ADR-0025 decision 5), so a bound block spends
itself on the near end and never reaches the far one. A narrow-budget question
about department or org material measures *distance*, not relevance. Every
ranking question here — both narrow-budget ones and both paraphrases —
therefore asks about the reader's own material, which the gradient forces
rather than the corpus choosing.

## The two paths

The nightly's deterministic hash embedder has no meaningful geometry by
construction (ADR-0023 decision 6), so the dense leg ranks by nothing there;
the sparse leg (Tantivy BM25) is real on any stack. `semantic` questions are
therefore **skipped and counted** on that path rather than scored zero — a
question the configured embedder structurally cannot answer is not a
regression.

- `make eval` — deterministic, gated by `evals/baseline.json`, and since
  EVAL-4 this is what `ci.yml` runs on every pull request.
- `make eval-retrieval` — real BGE-M3 through TEI, gated by
  `evals/baseline-retrieval.json`, on the nightly. Its floors are measurements
  and one of them is below 1.0 on purpose: `semantic-personal-tooling` misses,
  and the corpus keeps it. A suite that only asks what already passes measures
  nothing.

## Growing it

Add corpora, not questions past the arithmetic. A recall sweep is capped at 32
records (`MAX_RECALL_IDS`), and the runner refuses a full page rather than
mis-scoring it — the same rule EVAL-2's corpus lives under. Each corpus needs
its own reader and its own authors, and `evals/lib.sh` is where they are
registered and placed.

Fixtures are documentation-only content under the MEM-2/MEM-3 discipline: no
credentials, real or synthetic-but-live-format.

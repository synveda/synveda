# Benchmarks

Published scores for Synveda, tracked per release (EVAL-3, ADR-0061
decision 11).

Every number below was produced by `make eval-longmemeval-judged` against
a live stack, seeded through `/v1` with each actor's own bearer and through
the PDP — the harness holds no Synveda crate dependency and has no path
around the policy decision point (ADR-0028 decision 1, enforced by
`scripts/check-crate-deps.mjs`). What is measured is what a caller gets.

## Read this before the table

**A memory benchmark score is never a measurement of the memory system
alone.** LongMemEval grades whether a free-text answer matches a reference.
Synveda does not answer questions; it serves a governed context block.
Producing a number therefore takes a **reader** model that answers from the
block and a **judge** model that grades the answer, so every figure here is
a joint property of three things, only one of which is this product. Both
models are named in every row, recorded from what the API *served* rather
than from the alias requested. A memory benchmark figure quoted without its
reader model is not reproducible by anyone, including us, and the industry
convention of quoting one anyway is not a reason to adopt it.

**No claim here is tighter than the judge's own agreement.** ADR-0046
option 6 refused to let an unmeasured judge decide whether this product had
regressed, and that objection is discharged rather than deferred: the
judge is scored against a labelled set *inside the same run that uses it*,
and its agreement rate is a column beside the score rather than a footnote.
A benchmark number produced by an unmeasured judge is not a measurement; it
is a second opinion with a decimal point.

**Every row states its slice.** A suite that bounds its coverage says what
it bounded. The Instances column is the declared slice out of the corpus's
500, chosen deterministically and stratified across LongMemEval's six
question types so no category is silently at zero. Abstention instances are
excluded from the retrieval figure by upstream's own convention, and the
count excluded is in each score file.

**The corpus is identified by hash, not by presence.** LongMemEval's
haystacks are hundreds of megabytes, so the corpus is fetched rather than
committed (`evals/fixtures/longmemeval/NOTICE.md`). Each row therefore
carries the BLAKE3 of the exact file it was computed from. A published
score whose corpus cannot be identified is one nobody can reproduce.

## The two tiers

| | Deterministic — retrieval | Model-judged — QA |
| --- | --- | --- |
| Question | did the block bind the evidence sessions the instance names? | is the answer right? |
| Reproducible from bytes | yes | no — two external models |
| Gates a build | **yes**, `make eval-longmemeval` | **no**, deliberately |
| Costs money | no | yes, per instance |

The gate watches the half this product is actually responsible for. The
judged tier is off the merge path *and* off the nightly, because a gate
that fails when a model changes rather than when the code changes is an
alarm nobody keeps (ADR-0028 decision 6). It is run deliberately and
rarely, so the published number will sometimes be older than `main` — the
Commit column says which code produced it.

Both figures appear in every row on purpose. Retrieval holding while the
answer stays wrong means the block bound the right material and the loss is
downstream of retrieval, which is a composition problem rather than a
memory one; the two columns apart cannot say that.

## Results

<!-- benchmarks:begin -->

_No score has been published yet._ The corpus is fetched rather than committed
(`evals/fixtures/longmemeval/NOTICE.md`), and `scripts/publish-benchmark.mjs` refuses
any run that did not measure a LongMemEval release — so this table stays empty until
somebody runs the real thing, rather than filling with numbers from a corpus that
merely has the right shape.

<!-- benchmarks:end -->

This table is generated from `evals/scores/*.json` and `make ci` asserts
the two agree. It is not a place to type a number: a published score is a
measurement, and `scripts/publish-benchmark.mjs --check` exists so that a
hand-edited cell fails a build rather than becoming a claim.

## Reproducing a row

```sh
# The corpus is not in this repository — see evals/fixtures/longmemeval/NOTICE.md
export ANTHROPIC_API_KEY=...
make dev-up
EVAL_LONGMEMEVAL_INSTANCES=500 EVAL_REPORT=/tmp/longmemeval.json make eval-longmemeval-judged
node scripts/publish-benchmark.mjs /tmp/longmemeval.json
```

`publish-benchmark.mjs` refuses a report that is not the judged tier, that
names no reader or judge model, that could not grade every instance it
read, that was computed against a corpus which is not a LongMemEval
release, or that was produced from a dirty working tree. Each refusal is
one way a row could have claimed more than the run established.

Check out the row's commit to reproduce it exactly. The score also depends
on two models this repository does not control; a figure that moves between
releases with no code change and no corpus change is one of them having
drifted, which the score files make visible rather than prevent.

## The corpus, and the one that is missing

LongMemEval is MIT (Copyright © 2024 Di Wu) —
[xiaowu0162/LongMemEval](https://github.com/xiaowu0162/LongMemEval), *"LongMemEval:
Benchmarking Chat Assistants on Long-Term Interactive Memory"* (Wu et al., ICLR 2025).
It is used here under that licence, unmodified and unconverted: the harness reads
upstream's own format and never rewrites it, because a corpus edited until it fits
a harness is a corpus whose score means nothing.

**LoCoMo is absent, and the reason is a licence rather than an effort
estimate.** Its corpus is Creative Commons Attribution-NonCommercial 4.0,
which grants rights "for NonCommercial purposes only" — and this page is,
by its own feature's acceptance criterion, a commercial artefact.
Publishing LoCoMo numbers to sell an enterprise product is the paradigm
case of the use that licence withholds, so this repository does not carry
that corpus and does not quote its numbers. `make check-corpus-licences`
enforces that where the build can see it. **EVAL-7** carries the follow-on
with two named paths: written permission from Snap Research for commercial
benchmark use, or a permissively-licensed substitute.

A smaller claim honestly made, rather than a larger one we could not
defend.

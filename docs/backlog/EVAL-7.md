# EVAL-7: A second public benchmark

## Problem and evidence

LongMemEval is the only public memory benchmark currently implemented. Its deterministic retrieval tier gates and its model-judged tier is published but non-gating under ADR-0061 and ADR-0099. LoCoMo was the intended second corpus, but its CC BY-NC 4.0 terms do not establish permission for Synveda's commercial benchmark use; `evals/fixtures/longmemeval/NOTICE.md` and `scripts/check-corpus-licences.mjs` preserve that finding. A second result cannot be claimed until the corpus use is demonstrably permitted.

## Scope

- Select either a permissively licensed, externally comparable second benchmark or LoCoMo backed by written commercial-use permission.
- Record upstream version, source URL, licence or grant, immutable corpus digest, acquisition instructions, and any transformation before adapter work begins.
- Adapt the corpus through current session events, capture candidates, governed Knowledge acceptance, Knowledge query, and context-run APIs; do not create a benchmark-only storage path.
- Reuse the two-tier evaluation contract: deterministic, reproducible measures gate; reader/judge measures are published with served-model metadata and do not gate.
- Produce machine-readable and rendered reports that identify corpus variant, code commit, embedder, reader, judge, effort, sample coverage, and exclusions.

## Non-goals

- Using a non-commercial corpus on an assumption that internal or marketing use is permitted.
- Quietly editing benchmark questions, evidence, denominators, or answer keys to improve a score.
- Comparing a full-haystack result with an oracle/evidence-only variant as if they were the same benchmark.
- Making a model-judged result a merge gate or inventing a score before a complete run.

## Architecture seam

Add a corpus adapter and runner in `synveda-eval`, using the same public API and report/baseline machinery as LongMemEval. Third-party material lives only under a declared `evals/fixtures/<benchmark>/` inventory that `make check-corpus-licences` can validate. Benchmark vocabulary stays at the evaluation boundary; production domain types remain Sessions, capture candidates, Knowledge, and context runs.

## Acceptance criteria

- Repository evidence proves the selected corpus may be used and its result published for this product.
- A pinned full benchmark run completes through the epoch-3 public path without direct database writes or policy bypasses.
- The deterministic report is reproducible from the pinned corpus and configuration and fails on missing coverage or a reviewed bound regression.
- The judged report names the served reader and judge versions and effort, publishes judge agreement and ungraded counts, and gates nothing.
- The published result states corpus variant, sample count, exclusions, limitations, and a digest that another operator can verify.

## Required tests

- Corpus licence/inventory, digest, schema, timestamp, duplicate, and malformed-row tests.
- Deterministic adapter fixtures proving session/capture/Knowledge provenance and stable report output.
- Public-path end-to-end slice with ordinary tenant transactions and the test policy pack.
- Baseline tests that reject missing metrics, corpus/model drift, silent denominator changes, and judged-tier gating.
- Publication test that refuses an unapproved corpus variant or incomplete run.

## Rollout and rollback

Land licence evidence and a small non-publishable adapter fixture first, then a full observation run, baseline review, and publication. If permission, corpus integrity, or reproducibility is withdrawn, disable publication and retain the affected report as withdrawn evidence; LongMemEval remains the independent benchmark.

## Dependencies

The owner must choose the benchmark and publication claim. LoCoMo specifically remains blocked on written permission and legal acceptance; a permissively licensed substitute removes that external dependency. Full judged runs require pinned external reader/judge credentials and capacity, but deterministic evidence must remain runnable without them.

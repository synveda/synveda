---
title: "EVAL-4: Retrieval & injection quality"
labels:
  - epic:EVAL
  - phase:2
size: M
---

# EVAL-4: Retrieval & injection quality

**Epic:** EVAL — Evaluation (functional requirement) · **Phase:** 2 · **Size:** M

## Description

Fixture Q&A per scope; probe-based compression eval (CTX-6); tokens-per-inject trend.

## Acceptance criteria

One Q&A corpus under `evals/fixtures/qa/` whose material sits at four scope tiers because the suite **promoted it there through the governed path** — seeded at an actor's home through `/v1/observe`, then climbed to a team, a department and the org through `POST /v1/proposals` and real approvals, because records land at the caller's home scope and a service identity is a leaf under its anchor, so no other arrangement can put material above a leaf; one corpus seeded once and asked many times, so every question in a file measures the same corpus rather than the corpus as it stood when that question's own seed landed; grading joins seed to block by **record identity** — observe's `event_id` → the recall sweep's `provenance.event_id` → `record_id` → its position in the block's `record_ids` and `tiers` — never by string containment, because an index entry carries a truncated head and "demoted" and "absent" are otherwise the same measurement; `make eval` reports `qa_answer_rate` and `qa_body_rate` per scope tier and over the corpus, and `tokens_per_answer` as the exchange rate a composition change actually moves; the gap between answer rate and body rate is the index tier's displacement, reported as its own number; every question declares `needs: lexical | semantic`, and semantic questions are **skipped and counted in the report** on a run whose embedder cannot rank rather than scored zero; `retrieval_precision` reads only the blocks something bound — a block that carried everything the reader is served made no ranking decision to measure — and it is gated on both paths at different values, because on the deterministic one it is the sparse leg alone and the two numbers are not comparable; the dense leg is measured against live TEI on the nightly and gated against its own `evals/baseline-retrieval.json`, whose floors are measurements of today rather than round numbers, one of them below 1.0 because a paraphrase the corpus labels as ground truth is one the current embedder does not reach; `estimator_bias_p95` (CTX-2's `ceil(chars/4)` against a real tokenizer, declared model-specific) and `staleness_p50_permille` (MEM-6's unvalidated heuristic) are measured and reported, gated by nothing on the first run; **the deterministic gate runs on the pull-request path** — a Postgres-backed `eval` job in `ci.yml` that fails the merge on a breach, with the other jobs left database-free — which is EVAL-1's own recorded trigger fired by name and what "before merge" has to mean; a real composition change that degrades quality (a department's `budget_tokens` narrowed through the governed pack path, on a fresh tenant per phase) fails the gate naming the axis, the baseline, the measurement and the delta, and the scope tier that fell says which end of the gradient paid for it; nightly workflow; demo script. **Deferred with a recorded trigger:** the probe-based compression eval, because CTX-6 is Phase 3 and unbuilt — an axis for it would be permanently absent, which the harness treats as a coverage breach on every run, or permanently zero, which reads as coverage; it lands as `compression_fidelity` when CTX-6 does. Written 2026-07-31 (EVAL-4, ADR-0047): the feature text named three clauses, one of which has no product to measure, and an AC with no axis in it. The load-bearing parts are the lens — the block, which EVAL-2 rejected for exactly the properties that make it right here — and the discovery that a per-scope corpus has to be promoted rather than placed.

## Design

[ADR-0047](../adr/adr-0047-retrieval-and-injection-quality.md) — the block as the lens, a corpus that had to be promoted to span scopes, and two paths because only one of them can rank.

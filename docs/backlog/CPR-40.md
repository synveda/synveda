---
title: "CPR-40: Context-platform product and trust evaluation"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-40: Context-platform product and trust evaluation

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Replace the Record-era evaluation assumptions with deterministic, versioned
product and trust evidence over sessions, capture candidates, immutable
Knowledge, ContextRun, Skills, Tools, OKF and public adapters.

## Acceptance criteria

- A declarative suite covers capture precision, duplicate detection,
  cross-session/cross-user reuse, private/project/tenant isolation, conflicts,
  supersession, bitemporal query, provenance, token selection, bounded graph,
  versioned Skill use, MCP quarantine, OKF round-trip and adapter recovery.
- Retrieved, selected, injected, referenced, accepted, helpful, unhelpful and
  correction-causing outcomes remain separate exact measurements. Reports name
  code, extractor/model, retrieval, embedding and index versions plus latency,
  token and candidate quality.
- Six hard trust counts remain zero: tenant leakage, private leakage,
  superseded current injection, selection without provenance, unversioned
  Skill/Tool activation and plaintext-secret leakage.
- The original deterministic scenario, extraction, QA and security questions
  run through public session/capture/Knowledge/query/context APIs and real
  product scopes. No ContextRun substitutes for enumeration and no direct
  domain-table seed creates candidates or Knowledge.
- Supplied Markdown cannot forge context structure or attribution. Focused
  tests pin the evidence-bearing JSON envelope and current Knowledge address.
- `make eval-product`, `make eval`, `make eval-check`, focused tests, a demo,
  `make ci` and `make db-test` pass. Specialised BGE-M3, model extraction,
  10k security and Stage-H results remain separately labelled when their
  prerequisites are absent.

## Evidence

Delivered 2026-08-26 from `0958d69` under ADR-0099. The product suite defines
eighteen exact cases, eight distinct outcome signals and six zero-count trust
gates and emits revision/runtime-labelled JSON and Markdown reports. The
inherited evaluator now opens explicit workspace/project sessions, creates and
governs capture candidates, queries current Knowledge through the public
session lenses and uses principal/project/workspace/tenant scope tiers. Its
safe JSON renderer and current-address parser close structural-line and old
watermark attribution forgery. The deterministic extractor advances to
`builtin@4`, restoring measured macro precision/recall to 0.983/0.914.

The final default run passes six scenarios, 50 labelled extraction fixtures
(49 governed outputs), every QA tier and 1,276 security probes over 400
variants with every leakage, attribution and watermark-gap count zero. The
fresh-database PulseBoard report passes all eighteen cases, records the eight
outcomes separately and holds all six hard trust counts at zero. The
10,000-variant security tier passes 10,876 probes with all nine controls and
every leakage count zero at 31.335/79.427 ms p50/p95. The local
BAAI/bge-m3 tier runs under an immutable VedaFlow-reviewed provider
configuration, answers all ten QA questions and measures 0.800 explicitly
bounded retrieval precision at 78.170/174.550 ms median/p95. The authentic
264.5 MiB Stage-H slice (`blake3 cd766d50fe98`) delivers all 4,927 turns,
measures all ten instances and holds its correctness gate: zero empty blocks,
zero unattributed records and every composition bound. Retrieval recall is
0.643, per-type score 0.577, complete-instance rate 0.375, p95 549.277 ms and
mean context 1,433.8 tokens; evidence sessions that remained unrankable after
the fixed 1,800-second readiness window are reported per instance rather than
hidden. Two-hour tokens are enforced only for the disposable `lme-*` actors;
ordinary evaluation and production retain the one-hour default. The exact
`make ci`, fresh-scratch `make db-test`, `make eval`, `make eval-security`,
`make eval-product`, `make eval-check` and focused suites pass. Model-backed
extraction remains unmeasured because no Anthropic credential or local vLLM
endpoint is available; no deterministic result is represented as that live
tier. The resulting commit hash is recorded by CPR-41.

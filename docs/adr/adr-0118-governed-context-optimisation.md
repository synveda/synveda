# ADR-0118: Governed context optimisation stays inside ContextRun delivery

- **Status**: Accepted
- **Date**: 2026-09-25
- **Feature(s)**: CTX-8
- **Deciders**: Synveda implementation review

## Context

ADR-0084 already authorises immutable Knowledge and authored inputs before context delivery; ADR-0089 selects one immutable Configuration document; ADR-0099 separates delivery from usefulness. The current character heuristic, authored whitespace folding and title-only "summary" can overstate accuracy or change meaning. A fixed authored quota can also leave shared space unused. CTX-6 owns Session checkpoints. A conservative improvement must retain Cedar, forced RLS, VedaFlow Configuration changes, source provenance and content-free audit without adding a model service to normal startup.

## Decision

1. Add `off` and `conservative` to the existing governed context settings. Existing documents and built-in templates resolve to off; conservative is an explicit versioned Configuration choice. Balanced is withheld until a backend passes CTX-8's held-out quality and security gates.
2. Count the final rendered Synveda text after serialisation through one local counter. An explicitly selected, compatible encoding may be enforced exactly; an unknown receiving model yields a labelled estimate and independent byte/item ceilings. Synveda does not claim to count host history, other tools or output reservation.
3. Select authorised current revisions before extracting complete semantic units. Preserve their exact byte spans, source addresses and source hashes; hash the delivered representation separately. A title-only fallback is an index. Exact same-revision duplicates may be suppressed within one request; equal text across distinct authority/provenance remains independently represented.
4. Use one allowance for Knowledge, authored packs, Skills, wrappers and injected provenance. Required material is designated only by trusted policy/caller metadata and causes an explicit insufficient-budget result when it cannot fit. Optional omissions carry bounded reason codes. Deterministic tie-breaking and no-task fallback need no model call.
5. A context preview uses the same read/PDP planner but persists no ContextRun delivery or usage event. Real delivery retains the existing idempotent ContextRun row and content-free audit. Diagnostic reads repeat exact source decisions and mask data if later access is lost.
6. Keep progressive disclosure on the current exact-revision read and Skill activation paths. No cross-turn suppression or host-owned prompt rewrite is inferred from MCP access. Learned compression begins as an evaluation experiment and gets no public provider contract until measured promotion.

## Options considered

1. **Extend the existing planner and Configuration (chosen).** It shares authority, budget and trace semantics, and preserves off-mode comparison.
2. **Add a separate compressor API/service.** It cannot account for the final rendered block or inherit exact PDP decisions without duplicating the read plane.
3. **Use a character heuristic as an exact bound.** It is cheap but cannot prove a token cap for code or multilingual text.

## Consequences

- Positive: a source-built conservative path can run without extra credentials or model downloads, and the user can inspect honest accounting and provenance.
- Negative / accepted: unknown models have estimates only; MCP-only adapters cannot report full request usage or guarantee a host context window.
- Reversal trigger: measured representative workloads show a materially better optional backend with no deterministic protected-fact or security regression; record a new accepted decision before promoting it.

## Compliance notes

Candidate and detail content must be authorised before optimisation, with exact revisions under forced tenant RLS. Configuration changes remain typed VedaFlow effects; neither mode grants permission. Delivery/preview telemetry carries counts, hashes, versions and reason vocabularies only, never source text or denied-resource existence. CTX-6 derivatives remain Session working state, not published Knowledge.

# ADR-0025: Composition engine — chain gradient, pack-carried composition config, pre-VedaFlow watermarks

- **Status**: Accepted
- **Date**: 2026-07-23
- **Feature(s)**: CTX-2
- **Deciders**: sujitn

## Context

CTX-2 is the read path's assembly half: "scope-gradient assembly
(user>team>dept>org), pinned-first, conflict rules, token budget
(default 1.5k, per-scope configurable); channel rules (published +
policy-permitted derived). AC: deterministic given same inputs; every
block watermarked with commit hashes + record IDs; tokens_per_inject
metric emitted." Seed §4.4 is the contract: specificity gradient,
pinned priority within each level, conflicts resolved by (1) pinned
beats derived, (2) more specific scope, (3) newer valid time, all
under a configurable budget defaulting to 1,500 tokens. CTX-3 wires
the inject API onto it; CTX-5 inherits its as-of seam; PRMT-2's
context packs later arrive as pinned material.

Forces at play:

- **VedaFlow does not exist yet.** FLOW-1 (BLAKE3 commits) and FLOW-2
  (derived/staged/published channels) are Phase 2. There are no commit
  hashes to watermark with and no channel refs to read — yet the AC
  demands commit-hash watermarks and the feature names channel rules.
  What exists today is `RecordKind` (FND-3): `pinned` — authored,
  canonical, cannot be shadowed — and `derived` — pipeline output,
  "clearly watermarked as unreviewed" (seed §2.2 of the tech plan's
  channel table describes exactly this split).
- **MEM-5 (dedup & conflict detection) is Phase 2.** Semantic conflict
  *detection* — near-dup merge, supersession edges — is explicitly
  MEM-5's feature. CTX-2's "conflict rules" are seed §4.4's
  *resolution* ordering; they need a Phase-1 predicate to apply to.
- **The scope predicate already exists.** CTX-1's
  `permitted_chain_scopes` produces the caller's PDP-allowed chain
  scopes in gradient order (nearest-first), one `MemoryRead` decision
  per scope (ADR-0024 decision 1); ADR-0016 directs CTX-2 to consume
  `ScopeChainCache` rather than re-reading closure rows. Composition
  must not invent a second authorization path.
- **Packs already carry non-Cedar config.** MEM-2 put `RedactionConfig`
  on the loaded pack (ADR-0021 decision 3): embedded packs compile it
  in, stored packs configure it in a JSONB column, it hot-reloads with
  the pack, and it resolves through the same effective-pack walk as
  every decision. "Per-scope configurable" already has a mechanism —
  assign a pack at a node — with audit events, break-glass CLI, and
  next-request freshness, none of which a new per-node settings table
  would have for free.
- **Determinism is the AC.** Same inputs must yield the same block.
  Anything sampled inside compose — clocks, map iteration order,
  tokenizer versions — breaks it.
- **No LLM calls, no network, on the read path** (ADR-0024 decision 7
  made this structural for the crate). Token counting must be local
  and deterministic; the real tokenizer of whatever model the harness
  runs is unknowable here anyway.
- **The metric contract predates the feature.** FND-5 registered
  `synveda_tokens_per_inject` with budget-shaped buckets "so the
  contract exists from day one; the composition engine (CTX-2) records
  into it".
- **Audit shape is already decided.** Inject chains ONE event carrying
  commit-hash watermarks with aggregated per-candidate decisions
  (ADR-0019 decision 4) — CTX-3's emission point, not CTX-2's. CTX-2
  must produce a watermark that event can carry.

## Decision

`synveda-retrieval` gains `compose`: deterministic chain-gradient
assembly over the CTX-1 predicate, budgeted by a pack-carried
`CompositionConfig`, channel-ruled by `RecordKind` as the pre-FLOW-2
stand-in, watermarked with BLAKE3 version hashes as the pre-FLOW-1
content address, emitting `synveda_tokens_per_inject`.

Decisions, specifically:

1. **Composition consumes the CTX-1 predicate; the plan is one PDP
   walk.** `composition_plan` (beside `permitted_chain_scopes` in
   `retrieval::authz`) walks the caller's resolved chain once and
   produces, per PDP-allowed scope, the scope plus its channel rule,
   and the budget in force. The candidate universe is the chain — the
   seed §4.4 gradient — exactly as ADR-0024 decision 1 bounded it;
   broader pack-permitted scopes remain CTX-5's recall surface.
   `compose` itself takes the plan and a connection: given the same
   plan, instant, and database state, its output is byte-identical.
2. **`RecordKind` is the Phase-1 channel; the rule is pack config.**
   `pinned` stands in for the published channel (authored/canonical —
   the trust boundary), `derived` for the derived channel (unreviewed
   pipeline output). A new `CompositionConfig { budget_tokens,
   channels }` rides the loaded pack beside `RedactionConfig`:
   `channels` is `published-and-derived` (the product default in all
   three embedded packs — derived is readable-per-policy by design,
   seed tech plan §2.2) or `published-only` — the "bank mode" switch
   FLOW-2's AC will flip, existing and testable today. The rule
   resolves *per candidate scope* from that scope's effective pack, so
   a subtree assigned a published-only pack composes pinned-only while
   the rest of the chain composes both: per-scope channel rules
   through the existing assignment machinery. The config never grants
   — `MemoryRead` was already decided per scope — it only narrows
   which channel of readable material composes, which is why an
   unconfigured stored pack defaulting to the product config is not a
   widening.
3. **The budget rides the same config; "per-scope" means the caller's
   placement scope.** `budget_tokens` defaults to 1,500 (seed §4.4).
   The budget in force for an inject is resolved at the caller's home
   scope (chain head) under its effective pack: assign a pack with a
   different budget at a team and that team's members inject under it.
   Stored packs configure both fields via a `composition` JSONB column
   (migration 0017) and `synveda policy apply --composition-budget
   --composition-channels`, hot-reloading with the pack (the
   ADR-0021 plumbing, reused wholesale). No per-node settings table:
   packs already have versioning, audit events on assignment, display
   routes, and break-glass.
4. **Token accounting is a deterministic estimator, applied to the
   rendered text.** `estimated_tokens = ceil(chars/4)` (Unicode scalar
   values), applied to exactly the lines the block will contain —
   preamble, scope headers, entry lines with their markers, and the
   watermark line's marginal growth per entry. Assembly is first-fit
   in priority order: an entry that does not fit the remaining budget
   is skipped and assembly continues (deterministic, and a long broad
   record cannot starve nearer scopes because nearer scopes were
   already placed). The estimator is a named seam; per-harness real
   tokenizers are an adapter concern and EVAL-4 measures the bias
   (reversal trigger below).
5. **Assembly order is the seed §4.4 gradient, totally ordered.**
   Scopes nearest-first (user > team > department > division > org);
   within a scope, pinned before derived; pinned ordered by newest
   `valid_from` then id; derived ordered by relevance rank when the
   caller supplies one (a ranked id list from CTX-1's hybrid engine —
   derived records absent from it do not compose; retrieval already
   swept the permitted corpus), else newest `valid_from` then id.
   Pinned material never depends on the task: canonical content
   composes regardless of relevance input. The valid-time instant `at`
   is an explicit input — records compose only if their valid window
   covers `at` — which both closes the determinism AC (no clock reads
   inside) and is the valid-time half of the as-of seam CTX-5 extends
   to transaction time.
6. **Conflict rules: the seed's resolution order over an exact-match
   Phase-1 predicate.** Two candidates conflict when their trimmed
   content is identical (the cross-scope duplicate: the same fact
   extracted at user and team scope). One winner per group by the
   seed §4.4 comparator — pinned beats derived, then nearer chain
   position, then newer `valid_from`, then newer `tx_from`, then
   smaller id — losers are dropped from the block entirely. The
   comparator is exported: MEM-5 replaces the *predicate* (embedding
   near-dup, supersession edges) and reuses the resolution.
7. **Watermarks are BLAKE3 version hashes until FLOW-1 mints real
   commits.** Every composed entry carries
   `blake3(record_id ‖ tx_from ‖ content)` — the content address of
   exactly the version that composed, recomputable from the bitemporal
   store forever. The block hash is BLAKE3 over the ordered entry
   hashes. The rendered block ends with one watermark line carrying
   the block hash and every composed record id, counted inside the
   budget (auditability is paid for honestly; ~10 tokens per record).
   The `ComposedBlock` struct carries the full per-entry watermark for
   CTX-3's single audit event (ADR-0019 decision 4). When FLOW-1
   lands, commit hashes take this field — same shape, real commits.
8. **`synveda_tokens_per_inject` is recorded in `compose`, always.**
   The histogram FND-5 pre-registered; a compose over an empty
   permitted set records 0 — an inject that composed nothing is data,
   not an omission. The constant moves to `synveda-retrieval` (the
   emitting crate, the ADR-0016 precedent of store-declared metric
   names) and the gateway's describe-side references it.

## Options considered

1. **Per-node settings table for budget/channels** — honest "per
   scope" reading, but invents a second config plane: new mutation
   surface, new audit vocabulary, new cache/freshness story, all
   duplicating what pack assignment already has. Rejected; packs carry
   composition config as they carry redaction config.
2. **Real tokenizer (tiktoken-rs or similar)** — closer counts, but
   model-specific vocabularies (the harness's model is unknown here),
   a heavier dependency on the hot path, and cross-model bias anyway.
   Rejected for now; the estimator is a seam, EVAL-4 measures.
3. **No conflict handling until MEM-5** — honest about detection
   quality, but the AC names conflict rules and cross-scope exact
   duplicates are real today (same fact extracted at two scopes).
   Rejected; exact-match predicate now, comparator reused later.
4. **Watermark as response metadata only (no in-text line)** — saves
   ~10 tokens/record, but the injected text itself would carry no
   binding to its provenance and seed §4.4 says the *block* is
   watermarked. Rejected; hash + ids ride the text, budget-counted.
5. **Relevance filtering applied to pinned material too** — uniform,
   but canonical content must not silently vanish because a task
   didn't mention it; pinned is the trust anchor PRMT-2 builds on.
   Rejected.
6. **Compose reading `records_versions` for transaction-time as-of**
   — the full bitemporal inject ("what did the agent know on March
   3rd"). Deferred to CTX-5 with the refs half (tech plan §2.5); the
   `at` input covers valid time now without a second query shape.
7. **A `hierarchy_nodes` column for budget** — lighter than a table,
   but still a second config plane with none of the pack machinery,
   and budget/channels belong together. Rejected with option 1.

## Consequences

- Positive: composition is policy-shaped end to end (scope set and
  channel rules from the same effective-pack resolution, one walk);
  deterministic by construction (explicit instant, total ordering, no
  clocks or map-order dependence); bank mode exists and is testable
  two phases early; watermarks are recomputable content addresses the
  audit event can carry today and FLOW-1 can upgrade in place; the
  budget/channel knobs get pack versioning, hot reload, audit, and
  CLI for free.
- Negative / accepted trade-offs: the token estimator is approximate
  (chars/4) — budgets are honest bounds on *estimated* tokens, and a
  harness tokenizing differently will see ±20–30%; the watermark line
  spends budget (~10 tokens/record at default budget ≈ 10% at 15
  records); the conflict predicate catches only exact trimmed
  duplicates until MEM-5; `kind` conflates authorship with channel
  until FLOW-2 (a pinned record is *treated as* published without a
  review having happened — the honest Phase-1 state, and why derived
  stays marked unreviewed).
- Reversal triggers: EVAL-4 measures estimator bias beyond what
  budget headroom absorbs → per-adapter tokenizer behind the
  estimator seam; CTX-4's index tier or watermark overhead measured
  material → short-id watermark scheme; FLOW-1/2 land → commit hashes
  replace version hashes and channel refs replace the kind stand-in
  (this ADR's decisions 2 and 7 are explicitly transitional); MEM-5
  lands → semantic predicate replaces exact-match, comparator reused.

## Compliance notes

- The PDP remains unbypassable: `composition_plan` is the product
  path's only scope producer for compose, every scope in it is a
  per-request Cedar `MemoryRead` allow, and the channel rule comes
  from the same effective-pack resolution that decided the scope.
  Tests use packs (embedded and stored), never a bypass (seed §2.2).
- Composition config can only narrow, never widen: an unreadable
  scope never reaches compose regardless of config, and `restricted`
  sensitivity stays structurally out (ADR-0024 decision 2's clamp is
  reused).
- Audit: CTX-2 adds no action vocabulary and no HTTP surface. The
  inject audit event — one chained event with the block's watermark
  and aggregated decisions — is CTX-3's emission point (ADR-0019
  decision 4); `ComposedBlock` carries exactly what it needs.
- Tenant isolation: the candidate query filters `tenant_id`
  explicitly and runs inside the caller's tenant transaction (RLS
  backstop, ADR-0009).
- Determinism: no wall-clock reads, no unordered iteration affecting
  output; the AC test asserts byte-identical re-composition.

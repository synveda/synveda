# ADR-0026: inject API — the session-start seam, degradation ladder, one chained audit event

- **Status**: Accepted
- **Date**: 2026-07-23
- **Feature(s)**: CTX-3
- **Deciders**: sujitn

## Context

CTX-3 is the read path's HTTP half: "Session-start contract;
warm-cache p99 <150ms; graceful degradation (partial context + warning
header rather than failure). AC: latency SLO under 1k concurrent
sessions; degradation modes tested." The tech plan §3 spells the hot
path end to end: JWT verify → tenant/scope resolution (cached) → Cedar
authorize → composition over the scope chain → hybrid retrieve within
candidates → budgeted assembly → watermark → audit event → return
block. Everything below the HTTP seam now exists: CTX-1's engine,
CTX-2's plan and compose. CTX-3 wires them to a route and finishes the
Phase-1 spine.

Forces at play:

- **The product path is already demonstrated.** The CTX-2 example
  (`compose_block.rs`) runs identity → placement chain (HIER-2 cache)
  → per-request decision inputs → `composition_plan` (one PDP walk) →
  `compose`. The route must be that path with retrieval wired in — not
  a parallel one (seed §2.2: no path bypasses the PDP).
- **The query-embedding obligation lands here.** ADR-0024 kept
  `synveda-retrieval` network-free — the query vector is the caller's
  input — and assigned the embed call to CTX-3 through the MEM-4
  `Embedder` seam. The embedder is currently constructed in `main` and
  owned by the pipeline worker alone; the request path has no handle.
- **No transaction spans a network call** (the MEM-3 rule, restated by
  ADR-0023). The embed call is a network call; compose and search are
  tenant-transaction reads.
- **The audit shape is pre-decided.** Inject chains ONE event carrying
  its watermarks with per-candidate `MemoryRead` decisions aggregated,
  never one row per candidate (ADR-0019 decision 4). AUD-1's read
  discipline: allowed read handlers commit — their decision is a chain
  row, appended in the handler's own tenant transaction with the
  chain-head lock taken last. The recorded p99 risk is that lock;
  ADR-0019 option 2 (a buffered appender for read-path decision
  events) is the recorded upgrade, gated on evidence.
- **Degradation is the feature, not an accident.** CTX-1 already
  defined the one-sided modes (no vector → sparse-only, no sidecar →
  dense-only; a lagging sidecar can miss, never leak). The AC wants
  the route to convert dependency failures into partial context plus a
  warning, and only honest emptiness into an empty block.
- **The SLO predates the feature.** Seed §10: inject p99 < 150ms at 1K
  concurrent sessions, excluding first-call cold cache. FND-5's HTTP
  histogram buckets already bracket 150ms. The house perf discipline
  (HIER-1, MEM-1, CTX-1): on Docker Desktop assert the MEDIAN, report
  the tails — fsync stalls own the upper percentiles and the audit
  append is a commit — EVAL-6 owns percentile SLO enforcement on
  production-shaped IO.
- **Callers are users and agents alike.** AUTH-3's base layer carves
  exactly one floor out of service confinement — the role-free
  own-chain `MemoryRead` — precisely so agents inherit composition.
  Quarantined and unplaced identities plan nothing (CTX-2's
  `composition_plan` tests).

## Decision

The gateway gains `POST /v1/inject` beside `/v1/observe`: the third
primitive, implemented as the CTX-2 product path with the hybrid
engine wired between plan and compose, degrading by a fixed ladder,
chaining one audit event in the compose transaction.

Decisions, specifically:

1. **Route and contract.** `POST /v1/inject`, authenticated like every
   `/v1` route. Request: `{ task?, session_id?, budget_tokens? }` —
   all optional; the identity comes from the token, the scopes from
   the plan. Response: `{ text, block_hash, record_ids, tokens,
   budget_tokens, as_of, degraded }`. The response is 200 with a
   (possibly empty) watermarked block whenever authn and tenant
   resolution succeed: an empty plan — quarantined caller, unplaced
   identity, every scope denied — composes the empty block. Policy
   outcomes are results, not errors; the inject surface leaks nothing
   about why a block is thin (the AUTH-2/MEM-2 posture), and the
   audit event records the real reason.
2. **The handler is the compose_block path, two transactions around
   the network call.** Transaction 1 gathers the per-request decision
   inputs — identity row, pack assignments, role bindings (deliberately
   per-request reads, ADR-0016 decision 6) — with the chain from
   `ScopeChainCache`; `composition_plan` then runs pure (one PDP walk,
   ADR-0025 decision 1). The embed call happens between transactions
   (the MEM-3 rule). Transaction 2 runs `hybrid_search`, `compose`,
   and the audit append — chain-head lock last (ADR-0019) — and
   commits. Warm, transaction 1 is three indexed reads and the chain
   is a cache hit; the SLO's "warm-cache" is HIER-2/HIER-3/pack state,
   and CTX-3 adds no cache of its own.
3. **Query embedding through the MEM-4 seam, deadline-bounded,
   degradable.** The configured `AnyEmbedder` moves into `AppState`
   (shared with the pipeline worker — one config, one model identity,
   ADR-0023). A present `task` is embedded under a read-path deadline
   (`SYNVEDA_INJECT_EMBED_TIMEOUT_MS`, default 100ms — inside the SLO
   with headroom; warm TEI short-query calls sit far under it). Error
   or deadline → the dense leg is dropped (`SearchRequest.vector =
   None`, CTX-1's sparse-only mode) and the response is marked
   degraded — never failed. An absent `task` skips retrieval entirely
   and composes unranked — recency-ordered derived under ADR-0025
   decision 5's else-branch — by design, not a degradation.
4. **The degradation ladder, every rung a 200.** (a) Embed failure →
   sparse-only retrieval, still ranked: `degraded: ["embedder"]`.
   (b) `hybrid_search` failure → compose with `relevance: None`
   (pinned material plus recency-ordered derived — the taskless
   shape): `degraded: ["retrieval"]`. (c) Store/compose failure → an
   honest 5xx; there is no partial block without Postgres.
   Degradations ride the `X-Synveda-Degraded` response header (comma
   list) and the body's `degraded` array. Inside `hybrid_search`,
   CTX-1's one-sided modes (sidecar missing → dense-only) remain
   internal and visible in metrics/traces — the header reports what
   changed the *composition input*, not per-leg mechanics.
5. **One chained audit event: `context.injected`, in-transaction.**
   `AuditAction::ContextInjected` (`"context.injected"`) — the
   feature's only new action vocabulary. Payload: the block hash and
   composed record ids (the CTX-2 watermark, ADR-0025 decision 7),
   the instant, tokens and budget, the aggregated per-scope decisions
   — allowed scope ids in gradient order, denied chain scope ids —
   the degradation list, `session_id` when given, conflict/budget-skip
   counts, and `task_hash` (BLAKE3) when a task was given: never the
   task text (the ADR-0021 content discipline — audit payloads carry
   no user content). `CompositionPlan` gains the per-scope decision
   summary the walk already holds (additive), so the handler never
   re-derives authorization facts. Append failure fails the inject —
   audit is a first-class output (seed §2.5) and reads commit their
   decision row (AUD-1); if the latency AC shows the append or
   chain-head lock dominating, ADR-0019 option 2 is the recorded
   upgrade, not a quiet best-effort downgrade.
6. **The valid-time instant is server-stamped and echoed.** The
   handler stamps `Utc::now()` once, passes it to compose as CTX-2's
   explicit `at`, and returns it as `as_of` in the response and the
   audit payload. Same instant + same database state → byte-identical
   block (the CTX-2 determinism AC), so the audit event is a
   recomputable claim about what the agent was told. The as-of
   *parameter* — transaction time, refs, "what did the agent know on
   March 3rd" — is CTX-5's surface (tech plan §2.5, ADR-0025).
7. **A request budget narrows, never widens.** Effective budget =
   `min(pack budget, request budget_tokens)`. The pack config remains
   the ceiling (ADR-0025 decision 3); the narrowing exists for
   adapters — pre-compact injection has whatever room the harness has
   left, and ADPT-1 needs to say so per call.
8. **No new Cedar vocabulary, no response cache.** Authorization is
   the plan — per-scope `MemoryRead`, packs stay at `@5`; agents
   arrive through the AUTH-3 floor with zero new grants. Every inject
   composes fresh: pack flips, hierarchy moves, and role changes
   govern the very next inject (the ADR-0014/0015/0016 freshness
   promises), which a block cache would break. Metrics:
   `synveda_context_injects_total{outcome}` (`ok` | `degraded` |
   `empty` | `error`, with the degradation detail on the existing
   per-leg/mode counters); latency rides the FND-5 HTTP histogram
   whose buckets already bracket the SLO; block size rides
   `synveda_tokens_per_inject`, recorded inside compose on every call.
9. **The latency AC follows the house discipline.** An `--ignored`
   gateway test seeds a multi-scope corpus, warms the caches with a
   first pass, then drives 1k concurrent sessions over reused
   connections: the MEDIAN is asserted under 150ms, p95/p99 are
   reported (the audit commit is a write; Docker Desktop's fsync owns
   the tails), and the same run reports the embed/search/compose/append
   split so the ADR-0019 option 2 trigger is measured, not guessed.
   EVAL-6 owns percentile SLO enforcement. The degradation AC is
   direct: mock TEI down → 200 + header, sparse results still ranked;
   sidecar directory wiped → 200, dense-or-unranked per CTX-1;
   quarantined caller → empty block, event chained; same-instant
   re-compose → byte-identical.

## Options considered

1. **`GET` with query parameters** — cacheable and "it's a read", but
   the task is user content in a URL (logs, proxies), the response is
   identity-bound and must not be cached anyway (decision 8), and
   observe already set the POST precedent for primitives. Rejected.
2. **Fail closed on embedder failure (503)** — simpler and arguably
   honest, but the AC's whole point is the opposite posture: the block
   without the dense leg is still correct governed context, only less
   task-relevant. Rejected.
3. **403 for quarantined callers** — leaks placement state through
   the data plane's most-called endpoint and breaks the silent
   session-start contract (harnesses would need an error branch for a
   policy outcome). Rejected; empty block, audited truthfully.
4. **Best-effort / buffered audit append now** — pre-builds ADR-0019
   option 2 before any evidence the chain append binds the read path.
   Rejected; in-transaction until decision 9's measurement says
   otherwise (the recorded trigger).
5. **Client-supplied query vector** — lets a power caller skip the
   embed round-trip, but couples every adapter to the corpus's model
   identity and opens a text/vector mismatch the server cannot check.
   Rejected here; CTX-5's recall surface may revisit for explicit
   deep queries.
6. **Standard `Warning:` header** — the AC's phrasing, but RFC 9111
   obsoleted it and intermediaries may strip it. A custom
   `X-Synveda-Degraded` plus the body field is the boring explicit
   choice. Rejected.
7. **Response-block cache keyed on (identity, instant bucket)** —
   attractive for the SLO, but every freshness promise this codebase
   has made (next-request pack/role/hierarchy effect) dies at a cache
   with TTL semantics. Rejected; CTX-4/CTX-6 own read-path shaping if
   EVAL-6 ever shows compose itself binding.

## Consequences

- Positive: the Phase-1 spine is complete — SSO → governed memory →
  governed context, end to end on one PDP; the degradation posture is
  explicit, tested, and visible to callers; the audit event is a
  recomputable claim (instant + watermark + decisions); agents inherit
  inject with zero new grants; no new caches, so every existing
  freshness promise holds unchanged.
- Negative / accepted trade-offs: the chain append rides the hot path
  (measured, with a recorded upgrade); slow-TEI environments silently
  run sparse-only under the embed deadline (visible in the header and
  metrics, but a misconfigured deadline looks like "retrieval is
  mediocre"); two pool acquisitions per inject; taskless injects are
  recency-ordered — relevance requires a task by construction.
- Reversal triggers: the latency AC (or EVAL-6) shows the append or
  chain-head lock dominating → buffered read-path appender (ADR-0019
  option 2); ~~CTX-5 lands → the as-of parameter extends decision 6~~
  **(ADR-0042 decision 7: the parameter landed on `recall` rather than on
  `inject` — two explicit instants, `as_of` for transaction time and
  `valid_at` for valid time, both defaulting to the request instant. This
  decision's server-stamped instant is unchanged; inject stays the silent,
  fast, present-tense primitive, and the time machine is the deep one)**;
  FLOW-1/2 land → commit hashes and channel refs replace the CTX-2
  stand-ins inside an unchanged response shape; EVAL-4/6 evidence →
  revisit options 5 and 7.

## Compliance notes

- The PDP remains unbypassable: `composition_plan` is the route's only
  scope producer; an empty plan composes the empty block; tests use
  packs (embedded and stored), never a bypass (seed §2.2).
- Tenant isolation: both transactions are tenant transactions
  (`rls::begin_tenant_tx`, ADR-0009); the sensitivity ceiling stays
  clamped at `internal` (the extraction floor) until AUTHZ-5.
- Audit: one new action type, `context.injected`, one chained event
  per inject with aggregated decisions (ADR-0019 decision 4; DoD #4).
  No user content in payloads — task hash only (ADR-0021).
- Observability: `gateway.inject` span wrapping the plan → embed →
  search → compose → append stages; the counters and histograms of
  decision 8 (DoD #3).
- Determinism: the instant is explicit and echoed; no clock reads
  below the seam (CTX-2's AC holds through the route).

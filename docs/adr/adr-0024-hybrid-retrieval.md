# ADR-0024: Hybrid retrieval — pgvector ANN + Tantivy sidecar, RRF fusion, authz-derived pushdown

- **Status**: Accepted
- **Date**: 2026-07-23
- **Feature(s)**: CTX-1
- **Deciders**: sujitn

## Context

CTX-1 is the read path's engine: "pgvector ANN + Tantivy BM25, RRF
fusion; always filtered by tenant+scope+sensitivity via authz-derived
predicate pushdown. AC: retrieval quality on fixture set; NO LLM calls
on read path; p99 <80ms at 1M records/tenant." CTX-2 (composition) and
CTX-3 (inject) consume it; CTX-5 (recall API) exposes it. MEM-4 left
`record_embeddings` populated and unindexed ("ANN indexing belongs to
CTX-1"), and the Cedar schema records the intended enforcement shape:
"CTX-1/2/3 ask exactly this question [`MemoryRead`] per candidate
scope."

Forces at play:

- **The PDP is never bypassed (seed §2.2), but a per-record decision
  sweep cannot run before the index.** An ANN/BM25 query must be
  *filtered at the index* or it wastes its budget retrieving records
  the caller may not see (and a top-k over unfiltered candidates leaks
  existence through ranking). The decision therefore has to be made
  *before* the query, over a candidate set of scopes — then pushed down
  as a predicate — rather than after, over retrieved rows.
- **`MemoryRead` permits exceed the caller's own chain.** The packs
  grant: own placement chain (all packs), the department subtree under
  `standard`, org-wide non-personal scopes under `open-collaboration`,
  and any viewer/contributor/curator-bound subtree. The full permitted
  *set* is enumerable only by sweeping every tenant scope through the
  PDP — at the AUTHZ-1 measured ~109µs/decision, a 1 000-scope tenant
  costs ~100ms: over the CTX-1 latency budget on its own.
- **Composition reads the chain (seed §4.4).** `inject` composes user >
  team > department > org — exactly the caller's placement chain, ≤
  hierarchy depth scopes, already resolved and cached (HIER-2), with
  Cedar entity fragments prebuilt per chain (HIER-3).
- **Tantivy is a sidecar, Postgres is the system of record.** BM25
  lives in process-local index files, not in Postgres (tech plan §1.1:
  Tantivy for BM25 quality without ParadeDB's AGPL). A sidecar can lag,
  diverge, or be lost; the engine must stay correct — never
  over-return — whatever the sidecar's state.
- **The bitemporal pair is the change feed.** Triggers stamp `tx_from`
  on every new current version and archive closed versions into
  `records_history` with `tx_to` (ADR-0006). Any committed insert,
  update, or temporal delete is visible in one of the two tables by
  transaction time — no new outbox table is needed to tail changes.
- **`record_embeddings.embedding` is typmod-less by design**
  (ADR-0023: models with different dimensions coexist). pgvector can
  only index a fixed-dimension expression, and sqlx's compile-checked
  macros take literal SQL only — no string-built SQL, ever
  (CLAUDE.md), so runtime-composed `::vector(N)` DDL/queries are out.
- **NO LLM calls on the read path** (the AC; seed §3 wants inject
  silent and fast). Query *embedding* is an embedding-model call (TEI),
  not an LLM call — but even that must not be the retrieval crate's
  concern, and the engine must degrade to lexical-only when no query
  vector is supplied.
- **The deterministic dev embedder's geometry is meaningless noise**
  (ADR-0023 decision 6: "CTX-1's retrieval-quality work never runs
  against it"). Quality claims need either a real model (network) or a
  fixture whose vector geometry is *constructed* to be meaningful.
- **Sensitivity is not yet a policy attribute.** The Cedar schema's
  `MemoryRead` context carries roles only; classification/ABAC arrives
  with AUTHZ-5. Extraction floors sensitivity at `internal` (MEM-3).
  The sensitivity filter must exist structurally now, with its policy
  source arriving later.
- **Perf ACs on Docker Desktop assert medians** (HIER-1 discipline,
  restated through MEM-1): the dev environment's virtualisation owns
  the tail; EVAL-6 owns percentile SLO enforcement on
  production-shaped IO.

## Decision

`synveda-retrieval` gains the hybrid engine: a per-tenant Tantivy BM25
index maintained by a watermark-polling indexer task tailing the
bitemporal pair, a pgvector HNSW dense leg, reciprocal-rank fusion in
Rust, and a mandatory `SearchFilter` — the PDP-derived scope set plus a
sensitivity ceiling — pushed down into both legs and re-verified in the
final hydration statement. The scope set comes from
`permitted_chain_scopes`: one `MemoryRead` decision per scope of the
caller's placement chain, through the existing facade.

Decisions, specifically:

1. **The candidate-scope universe is the caller's placement chain;
   each candidate is PDP-decided per request.** `permitted_chain_scopes`
   walks the resolved chain (HIER-2 cache) and asks the PDP
   `MemoryRead` per scope, using that scope's chain (a suffix of the
   caller's) as the resource chain — the assignments and role-binding
   rows the gateway's `gather` already fetches for the full chain serve
   every suffix, because the PDP consults only rows whose node is on
   the resource chain. ≤ depth decisions at µs each, prebuilt fragments
   (HIER-3) inherited through the same facade. Scopes the packs permit
   *beyond* the chain (bound subtrees, `standard`'s department subtree)
   are not in CTX-1's universe: they are recall's deep-query surface,
   and CTX-5 owns enumerating a broader candidate set (its recorded
   perf problem — see reversal trigger).
2. **The filter is mandatory and fails empty, and sensitivity is
   structurally capped.** `hybrid_search` takes `SearchFilter { scopes,
   max_sensitivity }`; an empty scope set returns no results without
   touching either index — there is no unfiltered code path to call.
   `restricted` records are never retrievable regardless of the
   requested ceiling (the cap clamps to `confidential`) until AUTHZ-5
   makes sensitivity a policy attribute; today's requested ceiling is
   the seam caller's choice, defaulting to `internal` (the extraction
   floor).
3. **One Tantivy index per tenant, under a configurable root
   directory.** Fields: `record_id` (raw string, stored), `scope_id`
   and `sensitivity` (raw strings, indexed — the pushdown terms),
   `content` (tokenised, indexed, not stored — content is hydrated from
   Postgres, never from the sidecar). Per-tenant directories keep BM25
   corpus statistics tenant-local (no cross-tenant term-frequency
   bleed), make tenant disposal a directory delete (TEN-5), and make
   the tenant filter structural: a query opens exactly one tenant's
   index. A schema-version file in each directory forces a rebuild when
   the index schema changes.
4. **The indexer is a gateway-embedded task polling a per-tenant
   transaction-time watermark.** Each sweep, per active tenant (the
   pack-refresher pattern), inside a tenant RLS transaction: collect
   ids from `records` with `tx_from > watermark − overlap` and from
   `records_history` with `tx_to > watermark − overlap`, re-read each
   id's current version, upsert (delete-term + add) present ids and
   delete absent ones, commit Tantivy, then persist the watermark
   beside the index (the watermark describes the local replica, so it
   lives and dies with the directory — deleting the index directory
   *is* the rebuild procedure). The overlap window (default 10s)
   re-scans idempotently to cover writers whose `tx_from` was stamped
   before a concurrent sweep read; a writer holding its transaction
   open longer than the overlap is outside the design (worker
   transactions are milliseconds) and recovery is the directory
   delete. Polling mirrors ADR-0022's transport decision;
   LISTEN/NOTIFY is the same recorded upgrade if measured lag matters.
5. **The dense leg is compile-checked per supported dimension, with
   HNSW partial expression indexes.** Migration 0016 creates
   `hnsw ((embedding::vector(16)) vector_cosine_ops) where dim = 16`
   (the deterministic embedder) and the same for 1024 (BGE-M3 dense) —
   the two shipped embedders. The dense query exists as one sqlx
   `query_as!` per supported dimension, dispatched on the query
   vector's length; an unsupported dimension is a clean `Invalid`
   error naming the supported set. A deployment pinning a custom-dim
   model adds its index and query variant as a reviewed diff — the
   same review that admits the model (per-tenant pinning and re-embed
   are already deferred to the tech plan §1.3 machinery). The query
   sets `hnsw.iterative_scan = relaxed_order` transaction-locally so
   scope/sensitivity post-filters keep yielding candidates instead of
   starving the limit.
6. **Fusion is RRF (k = 60) over the two legs' ranks, then one
   verify-and-hydrate statement.** Top-N per leg (default 50) →
   `score(d) = Σ 1/(60 + rank)` → fused top-k ids are re-read in the
   caller's tenant transaction with the scope/sensitivity predicate
   re-applied in SQL. The re-check makes sidecar staleness one-sided:
   a lagging Tantivy index can only *miss* (bounded by the poll
   interval), never resurface a deleted, re-scoped, or re-classified
   record, because Postgres current truth decides what hydrates. Both
   legs are optional-but-at-least-one: no query vector → BM25-only
   (the embedder-down degradation CTX-3 will lean on); an absent or
   cold tenant index → dense-only.
7. **The query embedding is the caller's input.** `hybrid_search`
   takes an optional pre-computed query vector; it never calls an
   embedder. The gateway (CTX-3) owns embedding the query through the
   MEM-4 `Embedder` seam — TEI or deterministic, an embedding-model
   round-trip, not an LLM. The retrieval crate keeps zero HTTP
   dependencies: "NO LLM calls on the read path" is structural (the
   crate cannot make a network call other than Postgres), not policed.
8. **Quality is measured on a fixture set with constructed geometry in
   CI, and with the real model behind an ignored hook.** The fixture
   ships documents, queries, and relevance judgments; CI-run vectors
   are synthetic topic-mixture unit vectors (meaningful geometry by
   construction, honouring ADR-0023's "never against `hash@1`"), so CI
   asserts the *engine*: fused nDCG/recall at least matching either
   leg alone, filters never leaking, deterministic ranking. The
   live-TEI variant of the same harness (gateway tests, `#[ignore]`,
   the MEM-3 live-LLM pattern) measures real-model quality; EVAL-4
   owns quality targets and their regression gates.
9. **The perf AC follows the established discipline:** an `#[ignore]`d
   load test seeds 1M records for one tenant (batched
   records+embeddings writes satisfying embed-or-fail), builds both
   indexes, then asserts the hybrid p50 against the 80ms budget over a
   `select 1` baseline and *reports* p95/p99 — Docker Desktop owns the
   tail; EVAL-6 owns percentile SLO enforcement (MEM-1 precedent). The
   test vacuums its own debt.

## Options considered

1. **Post-retrieval PDP filtering (decide per retrieved record)** —
   truest to "every read through the PDP" naively, but top-k retrieval
   over an unfiltered corpus starves under selective permissions,
   leaks existence via rank displacement, and burns the latency budget
   on candidates that get dropped. Rejected; the PDP decides *before*
   the query over scopes, and the Cedar schema already records
   per-candidate-scope as the intended shape.
2. **Full permitted-set enumeration per request (sweep every tenant
   scope through the PDP)** — honest to pack semantics beyond the
   chain, but O(tenant scopes) PDP calls bust the 80ms budget at
   ~1 000 scopes. Deferred to CTX-5 (recall), which needs it and can
   afford batch evaluation work.
3. **Postgres FTS as the lexical leg** — transactional (no sidecar
   sync at all), RLS-covered, but `ts_rank` is not BM25 and the tech
   plan explicitly chose Tantivy for BM25 quality. Rejected for the
   leg, kept in mind as the fallback if the sidecar's operational cost
   ever outweighs its quality edge (reversal trigger below).
4. **One shared Tantivy index with a `tenant_id` field** — fewer file
   handles, but cross-tenant corpus statistics bleed into BM25 scores,
   tenant disposal becomes a doc-level delete crawl, and isolation
   rides on a query term instead of structure. Rejected.
5. **Transactional outbox (PGMQ signals) feeding the indexer** —
   exactly-once delivery, no clock reasoning, but couples every
   records writer to a queue send, adds a queue with no consumer half
   the time, and still needs a rebuild path. The bitemporal pair
   already *is* a complete change feed; polling it covers every
   writer including break-glass SQL. Rejected.
6. **Runtime-composed DDL/queries for arbitrary embedding dimensions**
   — violates the no-string-SQL rule for the exact code path (the hot
   read path) where auditability matters most. Rejected; per-dimension
   compile-checked variants for the shipped embedders.
7. **Do nothing (dense-only via pgvector)** — no lexical recall for
   identifiers, error strings, names — the queries agent memory is
   *for*. Fails the feature outright.

## Consequences

- Positive: retrieval is policy-shaped before it touches an index;
  sidecar failure modes are one-sided (miss, never leak); the engine
  is network-free by construction; dev mode retrieves usefully through
  BM25 even while `hash@1` vectors are noise; tenant disposal and
  index rebuild are both directory deletes.
- Negative / accepted trade-offs: BM25 visibility lags writes by the
  poll interval (bounded staleness, dense leg unaffected); index
  directories duplicate redacted content terms outside Postgres —
  they must live on the same encrypted volume as the database in any
  deployment profile, per-tenant key coverage for them is a recorded
  TEN-4 obligation, and TEN-5 disposal must delete the tenant's
  directory; supported ANN dimensions are a closed set (16, 1024)
  until a reviewed diff extends it; records beyond the caller's chain
  that packs would permit are not retrievable until CTX-5.
- Reversal triggers: filtered ANN p50 > 80ms at 1M records/tenant on
  production-shaped IO → activate OPS-4 (Qdrant behind `VectorIndex`);
  measured index lag or idle-poll load matters → LISTEN/NOTIFY
  (ADR-0022's recorded upgrade); sidecar operational cost outweighs
  BM25 quality edge in the field → Postgres FTS fallback per option 3;
  CTX-5's broader universe needs batch PDP evaluation → revisit
  decision 1's chain-only universe.

## Compliance notes

- The PDP remains unbypassable: the engine's only entry takes a scope
  set, `permitted_chain_scopes` is its only producer in the product
  paths, and every scope in it is a per-request Cedar `MemoryRead`
  allow under the caller's effective pack. Tests use test packs, never
  a PDP bypass (seed §2.2).
- Tenant isolation is layered: per-tenant index directories
  (structural), explicit tenant filters in every SQL leg (correct even
  under the RLS-bypassing dev superuser), and the RLS backstop for the
  app role (ADR-0009).
- Audit: CTX-1 adds no action vocabulary and no HTTP surface; the
  read-path audit events (one chained event per inject/recall with
  aggregated per-candidate decisions, watermarked with record ids)
  are CTX-3/CTX-5 emission points on the seams AUD-1 recorded
  (ADR-0019 decision 4).
- Redaction: the sidecar indexes only post-MEM-2 redacted content
  (staging never holds raw findings), and vectors were already
  computed over the persisted redacted text (ADR-0023 decision 5), so
  neither leg can surface a secret the tables do not contain.

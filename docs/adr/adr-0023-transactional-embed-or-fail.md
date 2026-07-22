# ADR-0023: Transactional embed-or-fail — the embed stage, the sidecar table, the schema backstop

- **Status**: Accepted
- **Date**: 2026-07-22
- **Feature(s)**: MEM-4
- **Deciders**: sujitn

## Context

MEM-4 puts embedding (TEI serving BGE-M3, tech plan §1.3) inside the
ingestion transaction boundary: partial batch failure → retry, never
silent drop — the documented Mem0 failure mode, where a record is
committed first and embedded asynchronously, so an embedding failure
strands a memory that retrieval will never surface. The AC is a chaos
test: kill TEI mid-batch; zero lost or embedding-less records.

ADR-0022 anticipated exactly this feature: "embedding is *not* in the
commit transaction yet — records are born embedding-less until MEM-4
wraps the commit seam, which is exactly the seam (`commit` is one
function) it will wrap."

Forces at play:

- **"Inside the transaction" cannot mean "a transaction spanning a
  network call".** MEM-3 deliberately commits the load stage before any
  extractor runs and opens the write transaction only after extraction
  returns. Holding the tenant write transaction — and the archive row
  locks — across a TEI round-trip would reintroduce the shape MEM-3
  refused. The property the feature actually needs is atomicity of the
  *commit*: a record and its embedding exist together or not at all.
- **The retry machinery already exists.** ADR-0022 decision 6 gives
  every pre-commit failure the same flow: archive nothing, let the
  visibility timeout redeliver, dead-letter by `read_ct`. An embedding
  failure is a pre-commit failure like any other.
- **The vector must be computed over exactly the persisted text.**
  MEM-3 re-scans extractor output *inside* the commit loop. If
  embedding ran before that re-scan, a live-format secret echoed by an
  extractor would be embedded even though the stored content shows the
  placeholder — the secret's geometry would live on in vector space,
  recoverable by similarity probes. The scan must move ahead of the
  embed call.
- **The invariant should outlive this pipeline.** FLOW-1/2 will route
  pinned/promoted writes, MEM-5 will merge and supersede records —
  every future writer of `records` must inherit embed-or-fail, not
  re-discover it. Application discipline in one worker is not the
  repo's style; the append-only triggers (ADR-0006), the RLS backstop
  (ADR-0009), and the one-shot quarantine transition (ADR-0021) all
  enforce their invariants in the schema.
- **The store's tests write records directly.** The FND-4 bitemporal
  suite and the TEN-2 RLS suite call `records::insert`/`update` on
  plain pool connections (autocommit). Any schema-level enforcement
  must stay satisfiable by a single-statement writer.
- **Models change; dimensions differ.** The tech plan names BGE-M3
  (1024-d) or Qwen3-Embedding, per-tenant model pinning, and a
  re-embed workflow on model change. A network-free default is also
  needed (seed §2.1): dev, tests, and demos must not require a 2.3 GB
  model download. Fixed-dimension storage would wire one model's shape
  into the schema.
- **pgvector is in the image but not the schema.** No migration
  creates the `vector` extension yet; CTX-1 (hybrid retrieval) owns
  ANN indexing and query-time binding later.

## Decision

Embedding runs as a per-event stage between extract and commit;
vectors land atomically with their records under the archive-lock; a
sidecar `record_embeddings` table stores them; the store API and a
deferred constraint trigger make embedding-less records
unrepresentable; the embedder sits behind an `Embedder` trait with TEI
and deterministic implementations selected by environment.

1. **The embed stage: outside any transaction, per event.** After
   extraction (and re-scan, decision 5) the worker calls the embedder
   once per event with that event's candidate contents. A failure
   drops the event from the commit group — nothing archived, the
   visibility timeout redelivers, `read_ct` climbs toward the
   dead-letter threshold (ADR-0022 decision 6, unchanged). Events
   whose embeddings returned commit normally: TEI dying mid-batch
   costs only the un-embedded events a retry, and costs no event its
   records. Zero-candidate events skip the stage and commit (archive
   only), as before.
2. **Atomic commit: record + embedding in one statement.**
   `records::insert` and `records::update` gain a required
   `RecordEmbedding {model, vector}` parameter and write both tables
   in a single data-modifying-CTE statement (insert the record row,
   insert — or on update, upsert — the embedding row). One statement
   keeps the API a plain `PgExecutor` call, works in autocommit (the
   store suites), and means "record exists" and "embedding exists"
   are the same commit under the archive-lock: kill anything at any
   point and either both persist and the signal is consumed, or
   neither and it redelivers.
3. **Storage: a `record_embeddings` sidecar, not a column on the
   bitemporal pair.** Migration 0015 creates the `vector` extension
   and `record_embeddings(record_id PK → records ON DELETE CASCADE,
   tenant_id, model, dim, embedding vector, embedded_at)`, with
   `dim = vector_dims(embedding)` checked. The bitemporal
   current/history pair stays vector-free: history is provenance, and
   a re-embed on model change regenerates vectors rather than
   archiving stale ones. The column is typmod-less `vector` — models
   with different dimensions coexist (per-row `model` + `dim`); ANN
   index strategy over it belongs to CTX-1. This table is seed §4.2's
   `embedding_ref`. Tenant isolation per the ADR-0009 structural
   rule: forced RLS keyed to the tenant GUC, in the same migration.
   The app role gets SELECT/INSERT/UPDATE but no DELETE — an
   embedding row leaves only by its record's cascade (FK actions
   bypass RLS and role grants by Postgres semantics, so the cascade
   works; ad-hoc deletion that would strand a record does not).
4. **The schema backstop: a deferred constraint trigger.** A
   `DEFERRABLE INITIALLY DEFERRED` constraint trigger on `records`
   INSERT verifies at commit time that the embedding row exists.
   Defence in depth in the ADR-0006/0009 tradition: decision 2 makes
   the API unable to write an embedding-less record; the trigger
   makes raw SQL unable to commit one. Updates are covered by the API
   (the upsert refreshes the embedding whenever content is
   rewritten); a trigger-level staleness check is not attempted —
   MEM-5's supersession work inherits that obligation.
5. **Scan → embed → commit.** The extractor-output re-scan (ADR-0022
   decision 7) moves from the commit loop into the embed stage, ahead
   of the embed call: the vector is computed over the final,
   redacted, persisted text — never over content the scanner would
   have rewritten. A secret that never reaches `content` now also
   never reaches vector space. Side effect: the write transaction
   sheds the scan's CPU.
6. **The `Embedder` seam: one trait, two implementations, env
   selection.** `trait Embedder { fn method(); fn model(); async fn
   embed(&[String]) -> Result<Vec<Vec<f32>>> }`, dispatched through
   `AnyEmbedder` (the `AnyExtractor` shape: static, no `dyn`). `tei`:
   `POST {base}/embed` on text-embeddings-inference over `reqwest`,
   30 s timeout (under the 60 s visibility timeout, the ClaudeExtractor
   discipline), failures as `Error::Dependency { service: "tei" }` —
   the taxonomy's own example. A response whose vector count deviates
   from the input count is a dependency error, not a partial success.
   `deterministic`: BLAKE3 hash of the content expanded to a
   16-dimension L2-normalised vector, model `hash@1` — the
   zero-config, zero-network default for dev, tests, and demos; its
   geometry is meaningless and CTX-1's quality work never runs
   against it. Env: `SYNVEDA_EMBEDDER` (`deterministic` [default] |
   `tei`), `SYNVEDA_TEI_URL` (required for `tei`, the
   `SYNVEDA_VLLM_BASE_URL` precedent), `SYNVEDA_EMBEDDER_MODEL`
   (default `BAAI/bge-m3`, recorded per row). There is deliberately
   no `off`: embed-or-fail is unconditional wherever the worker runs.
   The model identity is config-declared, not probed from TEI's
   `/info` at startup — gateway boot must not couple to TEI
   availability (observe never blocks); a config/served-model
   mismatch is an ops error, accepted and recorded here.
7. **Audit and metrics extend, no new action.** The aggregated
   `memory.extracted` success payload gains the embedder method and
   model (counts per event were already there); embedding rides the
   existing exactly-once commit event rather than minting a second
   chain write per group (ADR-0019 decision 4's aggregation rule).
   New metrics: `synveda_embedder_requests_total{method, outcome}` and
   `synveda_embedder_request_seconds{method}`; the embed stage gets a
   tracing span. Failure paths reuse the existing
   `synveda_extraction_events_total{outcome="error"}` accounting.
8. **No new crate: SQL casts, not a pgvector client.** Vectors bind
   as `real[]` (`Vec<f32>`, a type sqlx checks natively) and cast
   `::real[]::vector` in the statement; reads that need dimensions
   use `vector_dims(...)`. The pgvector Rust crate is deferred to
   CTX-1, which will want it (or the same cast) for query-time
   binding — `deny.toml` stays untouched today.

## Options considered

1. **Embed stage + atomic commit + schema backstop (chosen)** — the
   transactional guarantee without a transaction spanning the network;
   the invariant survives future writers by schema, not convention.
   Con: `records::insert/update` signatures change and every store
   test call site follows — accepted as mechanical, and the churn *is*
   the point: an embedding-less write no longer typechecks.
2. **Call TEI inside the write transaction** — the literal reading of
   "inside the ingestion transaction boundary". Rejected: holds the
   tenant transaction, the archive row locks, and pool connections
   across a network call that can take seconds; MEM-3 explicitly
   refused this shape for extraction and embedding is no different.
3. **Embedding column on `records`/`records_history`** — no new
   table. Rejected: the structural rule would archive a dead vector
   into history on every update, typmod-less columns plus the
   trigger column lists complicate the pair, and re-embed-on-model-
   change becomes a bitemporal rewrite instead of a sidecar upsert.
4. **Commit records first, embed after, mark-and-sweep the gaps** —
   the Mem0 shape with a repair loop. Rejected on principle: the
   feature exists because "eventually embedded" degrades silently;
   a repair loop is monitoring debt pretending to be architecture.
5. **One TEI batch per commit group** — fewest round-trips. Rejected:
   one poisoned or oversized event's failure would retry the whole
   group (correct but wasteful), and per-event calls are what make
   the AC's "partial batch failure" semantics natural. The recorded
   upgrade if TEI round-trips ever dominate lag: coalesce calls,
   keep per-event failure attribution.
6. **Probe TEI `/info` at startup for the model identity** —
   provenance can never lie. Rejected: couples gateway boot to TEI
   availability (observe must keep admitting while TEI is down; the
   queue is the buffer). Config-declared model accepted instead.
7. **The pgvector Rust crate now** — typed vectors end to end. MIT,
   admissible; deferred anyway: the write path never materialises a
   stored vector in Rust, casts keep the sqlx macros compile-checked,
   and CTX-1 can adopt the crate when the read path needs it.
8. **UNIQUE/FK gymnastics instead of a constraint trigger** (e.g. a
   circular deferred FK from `records` to `record_embeddings`).
   Rejected: a circular FK needs the embedding row keyed back into
   `records` and confuses every future migration; the constraint
   trigger states the invariant in one function.

## Consequences

- Positive: a committed record cannot lack an embedding — by API
  shape, by schema, and under crash/redelivery/racing consumers via
  the archive-lock; the Mem0 failure mode is structurally absent, not
  monitored for. Secrets redacted from content are also absent from
  vector space. FLOW-1/2 and MEM-5 inherit the invariant for free.
  CTX-1 finds `vector` installed, populated, and model-tagged.
- Negative / accepted trade-offs: pipeline lag gains a TEI round-trip
  per event (bounded by the 30 s client timeout, observed via
  `synveda_extraction_lag_seconds` against the <60 s SLO). Records
  committed during the MEM-3 window remain embedding-less until the
  re-embed workflow (tech plan §1.3, lands with model-change work)
  backfills them — the constraint governs new writes only; dev
  databases predate no promises. The deterministic embedder's
  vectors are hash noise — honest placeholders, never a retrieval
  substrate. `records::insert/update` callers must produce an
  embedding, which is exactly the friction intended.
- Reversal trigger: measured extraction lag approaching the 60 s SLO
  with TEI round-trips dominating → coalesced batch calls with
  per-event failure attribution (option 5's recorded upgrade).
  Per-tenant model pinning or the re-embed workflow arriving →
  revisit decision 6's single config-declared model.

## Compliance notes

Seed §2.2 is untouched: the embed stage adds no read or write of
governed data — the PDP re-decision at commit (ADR-0022 decision 4)
still gates every record, and the embedding rides the same
transaction. Tenant isolation: `record_embeddings` carries forced RLS
keyed to the tenant GUC in the creating migration (ADR-0009
structural rule) and the TEN-2 suite's completeness guard covers it.
Audit: no new action type; the aggregated `memory.extracted` payload
names the embedder method and model, so the chain records how every
vector was produced (ADR-0019 decision 4). Redaction: decision 5
orders scan before embed — MEM-2's "no raw finding text downstream"
now extends to vector space. The AC is demonstrated by
`crates/synveda-gateway/tests/embedding.rs` (the chaos test: a mock
TEI killed mid-batch; partial commit, redelivery, recovery; zero
lost, zero embedding-less — plus the schema backstop refusing a
bare-SQL record), `crates/synveda-ingest/tests/embedder_http.rs`
(TEI client contract), and `demos/mem-4-embedding.sh`.

# ADR-0022: Extraction pipeline — PGMQ worker, archive-lock commits, the Extractor seam

- **Status**: Accepted
- **Date**: 2026-07-22
- **Feature(s)**: MEM-3
- **Deciders**: sujitn

## Context

MEM-3 is the observe queue's first consumer (ADR-0020 decision 7
forward-declared the contract: read the `{tenant_id, event_id}` signal,
open the tenant transaction, load the staging row, process, archive).
It classifies staged events into the six record classes
(`fact | decision | preference | procedure | entity | episode`),
summarises at write time, and commits `kind = derived` records whose
provenance carries the AC quadruple: source session, extraction method,
model version, confidence. The destination exists already — the
bitemporal `records` table (FND-4, ADR-0006), whose `provenance` shape
this pipeline owns (`synveda_store::records::RecordState`).

Forces at play:

- **The backlog says "Temporal workflow"; the workspace cannot honestly
  ship one.** The community Temporal Rust SDK is distributed as a git
  dependency (`deny.toml` refuses unknown-git sources) and its rustls
  stack pulls `ring`/`aws-lc`, whose licences this workspace
  deliberately excludes. The tech plan already hedges both ways:
  "Rust SDK (community) or activities via gRPC workers" (§1.1) and
  "PGMQ + a simple Rust worker covers SMB mode; Temporal required only
  for enterprise profile" (§6). The SMB deployment profile has no
  Temporal in its footprint at all.
- **Retries must not duplicate memories.** The queue is at-least-once:
  a visibility-timeout expiry mid-processing redelivers the message,
  and a crash between record insert and archive would otherwise
  extract twice. `records::insert` mints a fresh UUIDv7 per record, so
  nothing at the table level dedups a re-run. Exactly-once has to be
  engineered at the commit seam.
- **The write was authorized once, but time has passed.** Observe
  admitted the event under a `MemoryWrite` decision at the caller's
  home scope — possibly minutes ago. The owner may since have been
  quarantined, moved, or revoked. Seed §2.2 ("every read and write
  passes through a PDP") reads naturally as: the pipeline's own write
  re-decides, it does not replay a stale allow.
- **The worker runs outside every ambient context.** No task-local
  tenant (ADR-0008's reversal trigger names exactly this case: workers
  pass context explicitly), no ambient actor for audit events
  (`ActorKind` today is `subject` and `break_glass` — neither is
  honest attribution for a system pipeline), no request span.
- **LLM output is untrusted input.** Staged payloads are already
  redacted (ADR-0021) and `[REDACTED:*]` placeholders must survive as
  opaque tokens — but a model can echo memorised secrets or fabricate
  secret-shaped strings into extracted content, and that content is
  about to become long-lived, injectable memory.
- **The precision AC points at a Phase-2 feature.** "Precision on a
  labelled fixture set ≥ target (see EVAL-2)" — EVAL-2's dashboard and
  target-setting land in Phase 2. MEM-3 must ship the measurement seam
  and an honest interim bar without pretending the real quality gate
  exists. The pipeline must also be trivially pointable at a live LLM
  (Claude API, or vLLM for air-gapped) — the product path — without
  making the test suite network-dependent.
- **The neighbours are designed but not built.** MEM-4 will put TEI
  embedding inside the ingestion transaction (embed-or-fail); MEM-5
  inserts dedup/conflict between extraction and commit; FLOW-1/2
  replace the direct insert with a VedaFlow derived-channel commit;
  GRPH-2 adds graph-linking. MEM-3's shape must leave those seams
  open, not pre-build them.

## Decision

Extraction runs as a PGMQ-polling worker loop in `synveda-ingest`,
spawned by the gateway as an embedded task; stages are Temporal-shaped
activities; commits take the archive as an in-transaction lock; the
LLM sits behind an `Extractor` trait with Claude API, vLLM, and
deterministic implementations selected by environment.

1. **A simple Rust worker now; Temporal-shaped so enterprise can host
   it later.** The worker is a `tokio` interval loop (the pack
   refresher's shape): `pgmq.read` a batch of signals → group by
   tenant → process. The stages are activities in the Temporal sense —
   pure async functions over serde-serializable inputs/outputs
   (`ExtractionInput` → `ExtractionOutcome` → commit receipt) — and
   the per-signal orchestration is a function separate from the
   polling transport. The recorded mapping when the enterprise profile
   (OPS-2) adopts the Temporal SDK: signal consumption becomes
   workflow start, visibility timeout + `read_ct` become the retry
   policy, archive becomes workflow completion; the activities move
   unchanged. It lives embedded in the gateway (spawned from `main`,
   aborted on shutdown) rather than as a second binary: SMB mode stays
   one process, and the worker shares the gateway's pool, PDP, and —
   critically — its `ScopeChainCache`, so hierarchy-move invalidations
   reach extraction's authorization reads for free.
2. **Exactly-once commit: the archive is the lock.** `pgmq.archive`
   runs *inside* the tenant write transaction, before the record
   inserts. Archive deletes the queue row, so under concurrent
   redelivery the row lock serializes contenders: whoever commits
   first wins, and a competitor's archive then moves zero rows — it
   drops the message and inserts nothing. Commit therefore means
   "records exist AND the signal is consumed", atomically; failure
   before commit means neither, and the visibility timeout redelivers.
   A deliberately re-sent signal (break-glass `pgmq.send` for an
   already-processed event) is intentional reprocessing — additive by
   design, with MEM-5's dedup as the semantic net. `pgmq.a_observe`
   is the durable completion/dead-letter record; no new table.
3. **The `Extractor` seam: one trait, three implementations, env
   selection.** `trait Extractor { async fn extract(&ExtractionInput)
   -> ExtractionOutcome }`, dispatched through an `AnyExtractor` enum
   (no `async_trait`, no `dyn`). `claude`: the Anthropic Messages API
   over `reqwest` (forced tool-use for schema-shaped output; base URL
   overridable so tests and demos point at a local mock; key via
   `ANTHROPIC_API_KEY`, never logged). `vllm`: any OpenAI-compatible
   `/v1/chat/completions` endpoint — the air-gapped path. Both share
   one prompt (the six class definitions, summarise-at-write,
   placeholders-are-opaque) and one strict response parser; malformed
   output is `Error::Dependency` and retries under decision 2.
   `deterministic`: a rule-based classifier over `ObserveKind` +
   payload keywords with truncation-as-summary — the zero-config,
   zero-network default (seed §2.1) that keeps dev, demos, and tests
   self-contained. Confidence is model-elicited (the deterministic
   rules carry fixed per-rule values) and recorded as *uncalibrated*;
   EVAL-2 owns calibration. Provenance per record:
   `{event_id, session_id, method, model_version, confidence,
   extracted_at, redactions?}` — the AC quadruple plus traceability
   into the staging row and its finding summary (ADR-0021).
4. **The pipeline's write re-decides at commit time.** In the write
   transaction the worker re-reads the owner (`identities::by_id`),
   resolves its *current* placement chain through the shared cache,
   reads pack assignments and role bindings for that chain, and calls
   `Pdp::require(MemoryWrite, Scope(current_home))` — the same
   own-home floor observe used, now under current facts. Records land
   at the owner's current home (a mover's memories follow the mover).
   A deny archives the signal and chains a standalone
   `authz.decision`/deny event (ADR-0019 decision 4); no records; the
   staging row remains re-drivable provenance. Service-identity owners
   keep their confinement: `token_scope` is rebuilt from the chain
   (anchor above the personal leaf) exactly as the gateway seam does.
5. **Audit: `memory.extracted`, aggregated; a third actor kind.** One
   chained event per tenant commit-group — event ids, per-event record
   counts and classes, method, model version, denied/dead-letter
   membership — never one row per record (the `memory.observed`
   precedent; the inject aggregation rule, ADR-0019 decision 4).
   Dead-letters (decision 6) chain the same action with
   `outcome = failure`. Payloads carry counts only — confidence is a
   float and audit canonicalisation rejects floats; it lives in record
   provenance. The actor is the new `ActorKind::System`
   (`actor_kind = 'system'`, subject = the component, here
   `"extraction"`) — migration 0014 widens the check constraint; MEM-6
   sweeps and AUTH-4/5 sync jobs inherit the kind. Worker events carry
   no trace id for now (the gateway owns OTel; ADR-0007's shared-init
   note fires when a second binary appears).
6. **Failure flow: leave-and-retry, then dead-letter by count.** An
   extractor failure archives nothing — the visibility timeout
   redelivers and `read_ct` climbs. A signal arriving with
   `read_ct > SYNVEDA_EXTRACTION_MAX_READS` is dead-lettered without
   an extraction attempt: archived (the queue must drain; a poison
   message must not wedge the pipeline) and chained as
   `memory.extracted`/failure with the count. The staging row is
   untouched — re-driving a dead-letter is a break-glass re-send of
   its signal. A signal whose staging row is missing (disposal raced,
   or a bogus injection) archives with a warning, no chain event — no
   attributable operation occurred.
7. **Output hygiene: extracted content re-enters the scanner;
   sensitivity is floored.** Every candidate's content passes through
   `synveda_ingest::scan` before persist — an extractor that echoes or
   fabricates a live-format secret writes the placeholder, never the
   text (placeholders already in the input pass through untouched:
   the rules match live grammars, not `[REDACTED:*]`). Extractor
   findings are counted (`synveda_extraction_rescan_findings_total`)
   and noted in the audit payload. Sensitivity: the extractor may
   propose, the pipeline clamps to at least `internal` — auto-derived
   content is never `public` by construction; AUTHZ-5 brings real
   classification. Valid time: `valid_from = occurred_at` (when the
   observed thing held in the world), `valid_to` open; supersession
   and validity-window management are MEM-5's (ADR-0006: valid time is
   deliberately application-set, transaction time is the triggers').
8. **Precision: the harness ships now, the target is provisional.**
   A labelled fixture set (transcript-shaped observe events, expected
   class per extraction, `[REDACTED:*]` placeholders included) lives
   with `synveda-ingest`'s tests, with a per-class precision harness.
   The AC test asserts the deterministic extractor ≥ **0.8
   macro-averaged precision** — the provisional bar this ADR records.
   The same harness runs against live Claude or vLLM behind an
   `#[ignore]`d env-driven test: the hook EVAL-2 grows into a
   dashboard with real targets, hallucinated-memory rate, and
   calibration. The deferral is recorded in STATUS.

## Options considered

1. **PGMQ worker with Temporal-shaped activities (chosen)** — honest
   about the licence wall, sanctioned by the tech plan, keeps SMB one
   process, preserves the enterprise path as a hosting change rather
   than a redesign. Con: retry policy and dead-lettering are
   hand-built (decisions 2 and 6) — accepted as ~a hundred lines
   against an SDK we cannot legally vendor.
2. **Temporal Rust SDK now** — highest fidelity to the backlog line.
   Rejected: unknown-git dependency (denied), `ring`/`aws-lc` licence
   graph (excluded), and the SMB profile would carry a Temporal
   cluster it was explicitly promised not to need.
3. **Hand-rolled Temporal gRPC worker over the vendored `tonic`** —
   licence-clean, real Temporal semantics. Rejected: hundreds of lines
   of protocol code against "boring, explicit"; the polling worker is
   smaller than the protocol shim alone.
4. **A separate worker binary** — cleaner scaling story, anticipated
   by ADR-0007. Rejected for now: a second process in every dev,
   demo, and SMB flow, plus lifted telemetry init, bought nothing the
   embedded task lacks today; the split falls out naturally when
   Temporal hosting arrives (option recorded as the reversal path).
5. **Deterministic record ids (UUIDv5 over event id + index) instead
   of the archive-lock** — makes re-runs idempotent by collision.
   Rejected: breaks the UUIDv7 id discipline (ADR-0005), turns
   deliberate reprocessing into silent overwrites, and still needs
   the archive race handled; the lock solves redelivery at the seam
   where it occurs.
6. **Per-record or per-event audit rows** — finer forensics.
   Rejected: ADR-0019 decision 4 aggregates by operation, the observe
   batch precedent aggregates admission, and per-record rows would
   contend the per-tenant chain head in proportion to extraction
   volume.
7. **A dead-letter table** — queryable failures. Rejected:
   `pgmq.a_observe` already holds every archived message durably with
   its read count and timestamps; the chain records the failure with
   attribution. A table adds a second disposal obligation (MEM-6/
   TEN-5) for data the archive already carries.
8. **Skip re-authorization (the observe decision stands)** — one less
   PDP call. Rejected: seed §2.2 is unconditional, the staleness
   window is real (quarantine/revocation between ack and extraction),
   and the re-decide costs microseconds against an LLM call that
   costs seconds.

## Consequences

- Positive: the observe queue finally drains — MEM-1's "signals
  accumulate" debt is paid; exactly-once extraction holds under
  crash, redelivery, and racing consumers by construction rather than
  by dedup; extracted memories can never carry live secrets (the
  MEM-2 discipline extends around the LLM); the write path stays
  PDP-governed end to end; the enterprise Temporal adoption is a
  hosting decision, not a rewrite; `ActorKind::System` gives every
  future background job honest attribution.
- Negative / accepted trade-offs: the worker polls (default 1s) —
  idle polling costs a queue read per tick, and pipeline lag is
  bounded below by the interval (fine against the <60s SLO;
  LISTEN/NOTIFY is the recorded upgrade if lag or idle load ever
  matters). Confidence is self-reported and uncalibrated until
  EVAL-2. The deterministic extractor's summaries are truncations,
  not abstractions — honest but crude; the LLM impls are the product
  path. `reqwest` gains a TLS backend (`native-tls`: schannel on
  Windows, OpenSSL 3 on Linux — both admissible) for the Claude impl.
  Embedding is *not* in the commit transaction yet — records are
  born embedding-less until MEM-4 wraps the commit seam, which is
  exactly the seam (`commit` is one function) it will wrap.
- Reversal trigger: enterprise profile work (OPS-2) or a
  crates.io-published, licence-admissible Temporal Rust SDK →
  revisit decision 1 (the activities move; the worker becomes the
  SMB-only path). Sustained queue depth growth at MEM-1's load shape,
  or measured lag approaching the 60s SLO → LISTEN/NOTIFY wakeups
  and/or concurrent group processing before any architectural change.

## Compliance notes

Seed §2.2 holds: the pipeline's own write re-authorizes through
`Pdp::require` at current facts (decision 4); no code path from queue
to `records` skips the PDP. Tenant isolation: every staging read and
record write runs inside `rls::begin_tenant_tx` for the signal's
tenant; queue signals remain content-free (ADR-0020), and the worker
holds no cross-tenant state beyond the shared caches already deemed
safe (HIER-2/3). Audit: commits chain `memory.extracted`
in-transaction with decision context (ADR-0019 decision 1/4); denials
chain standalone decision events; dead-letters chain with
`outcome = failure`; migration 0014 extends the actor-kind vocabulary
without touching hashed history. Redaction (ADR-0021): staging
content is already redacted, placeholders are treated as opaque
tokens by every extractor, and decision 7 closes the LLM-echo hole so
the MEM-2 AC's "no raw finding text downstream" survives extraction.
The AC is demonstrated by `crates/synveda-ingest/tests/
extraction_precision.rs` (fixture precision ≥ the provisional target;
provenance quadruple on every candidate) and `crates/synveda-gateway/
tests/extraction.rs` (observe → worker → records end to end, with the
deny, dead-letter, and archive-race paths), and `demos/
mem-3-extraction.sh`.

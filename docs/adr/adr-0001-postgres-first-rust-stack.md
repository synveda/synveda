# ADR-0001: Postgres-first data platform, all-Rust services

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-6 (records decisions enacted by FND-1, FND-2)
- **Deciders**: sujitn

## Context

Synveda sells trustworthiness to regulated buyers (seed §1): it must run
on-prem and air-gapped, survive a bank's infrastructure review, and be
auditable end to end. That review board evaluates every moving part — each
extra engine is a separate backup story, HA story, patch cadence, and
licence to defend. The core path admits only MIT/Apache-2.0/PostgreSQL
licences (CLAUDE.md; tech plan §1), and the read path carries a hard SLO
(`inject` p99 < 150ms, seed §10). Deployment must scale down to "single
binary + Postgres" for SMB as well as up to regional data planes (seed §7).

## Decision

PostgreSQL 17 is the system of record for *everything* — records, hierarchy,
audit, versioning, queues (PGMQ), vectors (pgvector HNSW), and graph (Apache
AGE) — and all services are Rust: axum/tonic gateway, sqlx
compile-time-checked queries, Tantivy sidecar for BM25, Temporal only for
complex workflows, text-embeddings-inference for embeddings, Rauthy as the
bundled dev/SMB OIDC provider. Every deliberate non-choice (Elasticsearch,
Redis, Kafka, Neo4j, dedicated vector DB) is a door left open behind a
trait, not a dependency taken today.

## Options considered

1. **Postgres-first + Rust (chosen)** — one engine to operate, back up, and
   explain; transactional consistency between records, graph, queue, and
   audit with no sync pipelines; Rust gives a single static binary for the
   on-prem story and predictable hot-path latency. Cons: pgvector and AGE
   have known ceilings (post-filtered ANN, Cypher maturity) — accepted with
   explicit reversal triggers below.
2. **Best-of-breed polyglot** (Elasticsearch/OpenSearch + Redis + Kafka +
   Neo4j + Qdrant/Pinecone) — each component stronger in isolation, but the
   estate becomes five backup/HA/licence stories, cross-store consistency
   needs sync pipelines (exactly the "multi-database tax" the 2026 pgvector
   +AGE guidance warns against), Neo4j and several others fail the licence
   rule, and SMB "one command" deployment dies.
3. **Cloud-managed services** (RDS + managed queues + hosted vector DB) —
   least ops burden, but cloud-locked services in the core path are
   forbidden (seed §7): air-gapped and on-prem deployments are the target
   market, not an afterthought.
4. **All-in-one newcomers** (SurrealDB, Memgraph) — single-engine appeal,
   but BSL licences fail the constraint outright and their audit/HA maturity
   is unproven for regulated buyers.

## Consequences

- Positive: one backup/HA/DR story; records, graph edges, queue entries,
  and audit rows commit in one transaction; licence review is trivial;
  Tantivy avoids ParadeDB's AGPL while keeping BM25 quality; PGMQ keeps SMB
  deployments free of extra queue infrastructure.
- Negative / accepted trade-offs: pgvector metadata filtering is
  post-filter, not in-graph — and our ANN queries are *always* filtered
  (tenant + scope + sensitivity); mitigated by tenant partitioning + partial
  HNSW indexes (TEN-3). AGE Cypher performance is unproven at 10M+ edges.
  Temporal is Go, not Rust — isolated to the async plane, never on the read
  path.
- Reversal trigger: filtered ANN p99 > 200ms at ~5M vectors/tenant →
  promote the Qdrant adapter (OPS-4) to per-deployment default. AGE
  traversal benchmarks fail the GRPH-4 gate → the graph fallback ladder,
  per ADR-0004. PGMQ throughput ceiling → Temporal absorbs the ingestion
  buffer.
  **GRPH-4 settled 2026-07-25 (ADR-0029):** AGE traversal passed
  (2-hop 12.91ms median at 10M edges, slope 1.58×), so the fallback ladder
  stays unactivated and "AGE Cypher performance is unproven at 10M+ edges"
  above is now measured rather than assumed. The ladder itself was
  rewritten: the embedded engine originally named is no longer maintained
  and no licence-compatible property-graph replacement exists, so the
  fallback is indexed adjacency in Postgres, then a materialised k-hop
  closure table, with a second engine a last rung needing its own ADR. The single-engine claim held
  on both counts that matter here: Cypher writes roll back with the
  enclosing transaction, and forced RLS on AGE's label tables is honoured
  by Cypher traversals.

## Compliance notes

Single-engine storage means tenant isolation (TEN-2 row-level security),
bitemporal history (ADR-0006), and the AUD-1 hash chain all live under one
security and backup boundary — one place to prove encryption at rest, one
WAL to archive for point-in-time audit reconstruction. sqlx compile-time
checked queries (no string-built SQL, ever) are part of the audit story:
reviewers can enumerate every SQL statement in the binary.

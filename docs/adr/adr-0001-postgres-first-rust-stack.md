# ADR-0001: Postgres-first data platform and Rust services

- **Status**: Accepted; current stack clarified 2026-09-20
- **Date**: 2026-07-18
- **Feature(s)**: FND-6 (FND-1, FND-2)
- **Deciders**: sujitn

## Context

Synveda must remain portable for self-hosted and on-premises use. Each extra
data engine introduces synchronization, backup, tenant-isolation, encryption
and licence obligations. Knowledge changes, review state and audit evidence
need a transaction boundary contributors can inspect and test.

## Decision

Use PostgreSQL 17 as the authoritative store and Rust for the gateway, worker
and CLI. Product SQL is static and SQLx checked in `synveda-store`. The current
stack uses pgvector, PostgreSQL FTS, immutable Knowledge relations, leased jobs,
embedded Cedar and generic OIDC with bundled Keycloak. The React console and
client adapters consume the public API.

[ADR-0097](adr-0097-bounded-knowledge-graph-retrieval.md) owns bounded graph
retrieval; [ADR-0102](adr-0102-portable-reference-deployment.md) owns the portable
Compose deployment and separate worker; [ADR-0089](adr-0089-governed-runtime-configuration.md)
owns governed runtime configuration. The early AGE, Record graph, PGMQ,
Tantivy, Rauthy and Temporal designs are not today's stack. Their removal does
not relax the single-authority or transaction requirements.

## Options considered

1. **Postgres-first and Rust (chosen).** One authoritative data engine keeps
   atomic mutations, forced RLS and recovery boundaries coherent. pgvector,
   query planning and bounded traversal still need workload-specific evidence.
2. **Separate search, vector, graph and queue engines.** Rejected as the
   default: each adds another consistency, custody and operational boundary.
   A future measured requirement needs an ADR and dependency-licence review;
   no unimplemented adapter is promised.
3. **Mandatory cloud-managed services.** Rejected because they exclude offline
   and on-premises installations. Operators may supply compatible external
   PostgreSQL/OIDC through the existing deployment contract.
4. **One new multi-model database.** Rejected without equivalent transaction,
   RLS, recovery and licence evidence. A smaller component list alone does not
   establish trustworthiness.

## Consequences

- Positive: Knowledge, review and audit can commit together, and contributors
  can inspect one persistence and tenant-isolation model.
- Trade-off: database capacity and retrieval plan stability constrain scale;
  no HA, backup/PITR or production latency claim follows from the choice.
- Reversal trigger: measured supported workloads exceed PostgreSQL's relevant
  limits. Evaluate the bounded projection/index options first; OPS-4, CTX-7
  and EVAL-6 own the unresolved vector/planning/performance work.

## Compliance notes

Every product read/write retains Cedar enforcement, forced tenant RLS and the
applicable VedaFlow/audit boundary. Optional Apalis uses a separate,
non-authoritative transport database; it does not become a second product
store. Repository licence gates remain authoritative. Current operational
qualification belongs to [production readiness](../PRODUCTION_READINESS.md).

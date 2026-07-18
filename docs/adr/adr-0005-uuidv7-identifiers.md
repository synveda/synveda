# ADR-0005: UUIDv7 newtypes for all domain identifiers

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-3, FND-4
- **Deciders**: sujitn

(Numbers 0001–0004 are reserved for the ADRs listed in FND-6: stack,
Cedar-over-OPA, VedaFlow-in-Postgres, multi-graph AGE schema.)

## Context

FND-3 introduces `TenantId`, `ScopeId`, `IdentityId`, and `RecordId` in
`synveda-types`. Whatever representation they use is baked into the FND-4
bitemporal table schemas, every API payload, audit events, and the VedaFlow
commit metadata — changing it later is a data migration across the whole
estate. Constraints: identifiers must be generatable without a database
round-trip (observe pipeline commits records asynchronously, seed §3), must
not leak information across tenants, and must index well in Postgres, our
system of record (tech plan §1.1).

## Decision

All domain identifiers are UUID **version 7**, wrapped in per-concept Rust
newtypes that serialize transparently as canonical UUID strings.

## Options considered

1. **UUIDv7 newtypes (chosen)** — time-ordered, so b-tree/index locality in
   Postgres is good (no random-write amplification at scale); globally unique
   without coordination; native `uuid` Postgres column type; creation-time is
   recoverable from the ID which aids debugging. Con: embeds a coarse
   timestamp (acceptable: creation time is not sensitive and is recorded in
   audit events anyway).
2. **UUIDv4** — the previous default; uniform randomness fragments b-tree
   pages under insert-heavy load (observe pipeline is exactly that). No
   upside over v7 for us.
3. **BIGSERIAL / sequences** — smallest and fastest, but requires a DB
   round-trip to mint, leaks record counts across tenants, and complicates
   multi-region merge (seed §2.7 residency routing). Fails the constraints.
4. **ULID / KSUID** — same time-ordered property but non-standard column
   types in Postgres and weaker library/ecosystem support; UUIDv7 is the
   standardised (RFC 9562) equivalent.

## Consequences

- Positive: IDs mintable anywhere (gateway, ingest workers) with no
  coordination; stable `uuid` columns in FND-4; newtypes prevent
  cross-concept ID mix-ups at compile time.
- Negative / accepted trade-offs: 16 bytes vs 8 for bigint; creation
  timestamp visible to anyone who holds an ID.
- Reversal trigger: none anticipated; representation is isolated behind the
  newtypes, so a future change is a types-crate + migration concern, not an
  API rewrite.

## Compliance notes

No policy-enforcement effect. Tenant isolation never relies on ID secrecy —
every access passes the PDP (seed §2.2). Audit events gain slightly: v7 IDs
sort chronologically, matching audit-log order.

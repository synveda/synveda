# ADR-0145: Saved context uses an exact immutable policy snapshot

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: MEM-8, CPR-20
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

Current database anchors and grants do not prove the compiled Cedar source is
current. The background refresher releases its transaction before compilation;
its readiness lease bounds convergence age, not an exact stored pack revision.
Saved replay can therefore observe a committed policy change while evaluating
the prior compile. A cleared and recreated pack can reuse name/version 1.

## Decision

Read a bounded tenant policy inventory in the replay transaction and construct
a request-local frozen PDP over those exact sources and typed configurations.
Reuse a compiled `Arc` only when name, version, source bytes and configuration
all match under the same embedded schema/base instance. Otherwise compile the
exact source locally. The snapshot shares immutable embedded packs, owns its
tenant pack set and refuses policy installation/removal. Every replay decision
and composition threshold uses that same snapshot. Never repair the global
cache from a request or rely on name/version alone.

Limit the inventory to 64 packs and 1 MiB of source text. The store bounds rows
before returning text, and withholds text when the aggregate source bound is
exhausted. Limit concurrent snapshot construction to two per process and include
it within replay's five-second deadline. Cache hits reuse immutable compiles;
misses use bounded local CPU work. No network, model or second database pool is
introduced. Unavailable, malformed or over-limit current policy yields the
existing content-free saved-delivery conflict, retaining the original receipt.

Test packs use ordinary persisted tenant policy rows as well as their compiled
engine; a process-only test mutation cannot substitute for database authority.

This closes the stale-process-policy replay window after a committed change.
It does not serialize a policy mutation committing after the snapshot read,
nor establish full source withdrawal. The shared/exclusive authority gate,
source fences and final admission ordering remain MEM-8's next implementation.
Other public read paths retain their current policy-refresh contract.

## Options considered

1. **Exact frozen snapshot (chosen).** Reuses the embedded Cedar evaluator and
   immutable compiles while keeping the database policy documents authoritative.
2. **Fresh anchors with the mutable process PDP.** Does not observe exact current
   source and can change evaluator halfway through one proof.
3. **Compare only pack name/version.** Cannot distinguish clear/recreate or an
   incorrect compile under the same version.

## Consequences

Saved replay may compile current policy before the background cache converges.
The bounded compile adds local work; no model-quality or production-latency claim
follows. Large policy inventories refuse replay explicitly. Exact source and
configuration remain private; errors and telemetry contain no policy text.

## Compliance notes

The snapshot remains embedded Cedar over ordinary forced-RLS data. Static SQL
stays in synveda-store and metadata is generated on a fresh current database.
No source suppression, erasure privilege or independent authority is introduced.

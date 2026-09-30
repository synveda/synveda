# ADR-0129: Scale fenced Capture separately from singleton maintenance

- **Status**: Accepted
- **Date**: 2026-09-30
- **Feature(s)**: OPS-7
- **Deciders**: Synveda maintainers

## Context

The core worker now owns Capture, Knowledge indexing, relaxation expiry,
directory pull and native Skill validation (ADR-0102). Its Helm Deployment
still has one replica. Capture's PostgreSQL claim uses a process-unique owner,
an incremented attempt fence, database-time expiry and a terminal write before
candidate insertion (ADR-0083). A disposable Kind test with two worker pods
deleted the pod holding a blocked Capture call; a different pod reclaimed the
expired claim and committed one candidate. The test did not qualify the other
worker loops.

Knowledge indexing converges its durable sidecar insert by revision/model key,
but two workers can still call the external embedder for the same batch.
Directory pull has a per-tenant pass counter and absence-derived sealing
(ADR-0060), without a cross-process pass owner or fence. Running that loop on
two pods could count one directory snapshot twice and advance a leaver toward
an irreversible seal. A duplicate-safe row write is not a safe pass protocol.
The product needs a way to use Capture's proven parallelism without treating
every maintenance loop as parallel-safe or adding another queue authority.

## Decision

Keep exactly one **combined** core worker for maintenance and Capture. Permit
additional **capture-only** worker processes as a separate, explicit deployment
group. The combined process remains the Compose and single-replica Helm
default. A capture-only process runs the same database-authority sentinel,
initial Cedar convergence, policy freshness lease, private readiness and
bounded shutdown as the combined process, but starts only the Capture work
loop and its policy refresher. It does not run Knowledge indexing, directory
pull, relaxation expiry or Skill validation. Worker profile is a closed local
deployment setting, never a public job or tenant-controlled value.

The Helm chart may add a bounded count of capture-only pods after source and
Kind acceptance. It must retain one combined worker, keep the existing
`worker.replicas` refusal and reject values that create another maintenance
worker. One Capture loop performs at most one extractor call at a time; one
combined worker plus N capture-only pods therefore bounds simultaneous
Capture extractor calls at N+1. Operators must reserve that capacity at their
provider. A retry may repeat an external provider call; the fence guarantees
one durable candidate set, not exactly-once provider effects.

The first supported scale increment is at most two additional capture-only
pods, and the chart remains disabled until a direct three-worker claim,
provider-outage and pod-loss acceptance passes. Do not infer a general worker
replica count from this mode. Gateway scaling and its own traffic/key-rotation
acceptance remain separate OPS-7 decisions.

## Options considered

1. **Scale the existing combined Deployment.** Capture is fenced, but
   duplicate Knowledge provider calls and unfenced directory absence passes
   make this unsafe. Refused.
2. **Add durable leases to every maintenance family.** This could make each
   loop independently scalable, but directory reconciliation would need a
   fence across enumeration, mirror writes, absence accounting and audit.
   That is a larger schema and protocol change than the small-team capacity
   need justifies now. Defer until measured maintenance throughput or recovery
   requires it.
3. **Keep one combined worker and add capture-only pods.** Chosen: no new
   product state or transport, and the proven Capture fence remains the sole
   concurrent claim authority.

## Consequences

- Positive: Capture throughput can rise without multiplying directory passes
  or Knowledge provider calls. The one-worker deployment remains valid.
- Negative: maintenance work still pauses during its pod restart and has one
  process's throughput. Extra Capture pods consume database connections and
  provider capacity; the chart must count both.
- Rollback: remove capture-only pods, let their claims expire or drain, then
  continue on the combined worker. Fenced candidates remain readable.
- Reversal trigger: measured maintenance backlog exceeds its SLO or a
  directory-pull availability target requires concurrent owners; design and
  prove per-family fenced ownership then, rather than scaling the combined
  process by assumption.

## Compliance notes

Every process uses the exact ordinary worker PostgreSQL role, forced RLS,
embedded Cedar and the existing content-free audit contract. Capture-only
does not gain a database owner, privileged claim bypass, new queue or public
mutation path. The chart must not claim general worker HA from Capture-only
replicas.

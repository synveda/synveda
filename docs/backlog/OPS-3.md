---
title: "OPS-3: Residency routing"
labels:
  - epic:OPS
  - phase:3
size: L
---

# OPS-3: Residency routing

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** L

## Problem and evidence

Synveda currently deploys as one regional installation. There is no global
tenant-placement service, regional request router, tenant move protocol or
network-policy evidence that Postgres, embeddings, extraction, telemetry and
key operations remain in a selected jurisdiction. The gap is recorded under
SaaS deployment and residency in
[production readiness](../PRODUCTION_READINESS.md).

## Scope

- Define immutable tenant placement at admission and an audited, restart-safe
  move procedure when moves are supported.
- Route authenticated requests to the tenant's selected data plane before any
  tenant content is read.
- Keep Postgres, Knowledge embeddings, extraction inputs/outputs, tenant keys,
  audit evidence and content-bearing telemetry inside the permitted region.
- Fail closed when placement is unknown, conflicting or unavailable.
- Publish region health, placement drift and move progress without exposing
  tenant content.

## Non-goals

- No speculative global control plane inside the single-region gateway.
- No cross-region context composition or policy-safe content-summary shortcut.
- No claim that a replicated database alone provides jurisdictional isolation.
- No billing, multi-cloud abstraction or automatic failover that violates the
  selected residency boundary.

## Architecture seam

Tenant resolution remains the authority for one tenant per credential. A
separate routing/admission boundary may select a regional gateway before the
ordinary epoch-3 public API, Cedar, forced-RLS and VedaFlow path runs. Regional
provider, KMS, telemetry and object-store endpoints must be Configuration or
deployment inputs, never request-body choices.

## Acceptance criteria

- A tenant pinned to each supported region has no content-bearing network flow,
  storage object, backup, embedding request or trace outside that region under
  deny-by-default network tests.
- An unknown or unavailable placement refuses before tenant data access and
  emits content-free operational evidence.
- A move, if supported, quiesces writes, transfers database/key/audit state,
  verifies canonical evidence, changes routing atomically and has a measured
  rollback point.
- Support and break-glass access preserve the same regional boundary.
- Regional failure behaviour meets owner-approved RPO/RTO without silently
  failing over across jurisdictions.

## Required tests

- Multi-region integration environment with egress capture and explicit
  negative network policy.
- Cross-region identifier, backup, telemetry and provider-endpoint probes.
- Concurrent request tests at placement cutover and failure injection during
  every move phase.
- Restore/audit verification in the destination before traffic changes.

## Rollout and rollback

Begin with placement-only admission and no tenant moves. Add one region at a
time behind an allowlist. Preserve the previous route until destination
verification succeeds; rollback returns routing only while the source remains
authoritative and unmodified.

## Dependencies

Hosting topology, jurisdictions, subprocessors, KMS/object-store providers,
support access and regional RPO/RTO are owner decisions. OPS-5, OPS-7, TEN-5
and an accepted residency ADR are prerequisites for moves or failover.

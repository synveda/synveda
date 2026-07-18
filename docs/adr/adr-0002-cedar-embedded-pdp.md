# ADR-0002: Cedar embedded in-process as the PDP, not an OPA sidecar

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-6, AUTHZ-1 (facade), AUTHZ-6 (OpenFGA escape hatch)
- **Deciders**: sujitn

## Context

Policy is enforced, never advisory (seed §2.2): every read and write passes
through a Policy Decision Point, with no code path around it. That puts an
`authorize()` call on the `inject` hot path, which carries a p99 < 150ms SLO
at 1K concurrent sessions (seed §10) — a network round-trip per decision is
a material fraction of that budget before any retrieval work happens.
Policies themselves are governed assets: policy packs and lapses version
and flow through VedaFlow (tech plan §2.3), so the policy format must be
storable, diffable, and reviewable as data. Regulated buyers additionally
need decisions they can explain to an auditor.

## Decision

The PDP is **Cedar**, embedded in-process in the gateway, fronted by a
single internal `authorize(subject, action, resource, context)` facade.
Policy packs are versioned Cedar bundles per tenant with hot reload; the
tenancy hierarchy is materialised into Cedar's entity store (HIER-3).
Relationship checks use Cedar's entity hierarchy first; OpenFGA remains an
adapter path behind the same facade if ReBAC outgrows it.

## Options considered

1. **Cedar embedded (chosen)** — pure Rust, in-process, microsecond
   decisions with no network hop; formally verified evaluator (a genuine
   differentiator in front of a bank's review board); policies-as-data fits
   VedaFlow versioning and the policy_snapshot_hash recorded on every
   commit. Cons: younger ecosystem than OPA; ReBAC expressiveness at deep
   hierarchies is the known limit.
2. **OPA sidecar (Rego)** — the incumbent standard, powerful language, but
   it adds a Go sidecar and a network hop to every decision on the hot
   path, complicates the single-binary SMB deployment, and Rego's
   evaluation is not formally verified. Remains a supported adapter for
   shops that mandate OPA — the facade hides the engine.
3. **OpenFGA as primary** — best-in-class ReBAC, but it is relationship
   checks only (no attribute/condition policies like sensitivity, residency,
   time-of-day — AUTHZ-5 needs those), and it is another service hop.
   Kept as the escape hatch, proven by the AUTHZ-6 spike before it is
   needed.
4. **Hand-rolled RBAC in application code** — fails seed §2.2 structurally:
   policy scattered through code is advisory by construction, unreviewable
   as an asset, and un-lapsable without redeploys.

## Consequences

- Positive: authorization adds microseconds, not milliseconds, to inject;
  policy packs (`regulated-strict` / `standard` / `open-collaboration`) are
  ordinary versioned assets with review and rollback; the gateway stays one
  static binary; every decision logs its policy version (AUTHZ-1 AC).
- Negative / accepted trade-offs: we own entity-store synchronisation from
  the hierarchy (HIER-3) — a transactional-consistency obligation an
  external PDP would not impose; Cedar's ReBAC limits at deep or unusual
  hierarchy shapes may force the OpenFGA adapter earlier than planned.
- Reversal trigger: the AUTHZ-6 spike defines the conditions (hierarchy
  depth, relationship-query shapes, decision latency) under which the
  facade flips to OpenFGA for relationship checks. A customer mandate for
  OPA activates the OPA adapter for that deployment only — never a core
  change.

## Compliance notes

This ADR *is* the enforcement mechanism for seed §2.2: one facade, called
by the gateway on every request, with no alternative path to storage — the
crate dependency rule (types ← policy/store ← retrieval/ingest ← gateway)
makes bypass structurally visible in review. Every decision (allow and
deny) is audited with the policy-pack version in force (AUD-1). Tests use a
test policy pack, never a PDP bypass (CLAUDE.md).

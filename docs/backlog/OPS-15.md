---
title: "OPS-15: Operational signals and recovery runbooks"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-15: Operational signals and recovery runbooks

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

Synveda exports Prometheus and OTLP signals, and the Compose reference has an
optional private Collector/Prometheus profile. The gateway and workers expose
readiness and selected stage, queue, policy, audit and key metrics. These are
raw observations, not a supported availability or recovery contract. There
are no owner-approved SLOs, recording/alert rules, release markers, support
diagnostics or exercised P0/P1 runbooks. The gaps are PR-10 and the
operational rows in [production readiness](../PRODUCTION_READINESS.md).

## Scope

- Define a small, bounded SLI set for request availability and latency,
  ContextRun outcomes, Session delivery lag, Capture retry/queue age,
  Knowledge index lag, policy freshness, backup/restore age and dependency
  health. State each measurement's scope and blind spots.
- Emit one immutable release/schema/configuration identity marker for gateway
  and worker telemetry so an alert and runbook name the exact deployed bytes.
- Provide recording rules and alert candidates with a runbook for every P0/P1
  condition. Test collector loss separately from healthy zero-traffic periods.
- Offer tenant-safe aggregate diagnostic views for loading, partial, stale and
  degraded product states through the public API where customer action helps;
  use the existing PDP/RLS path and omit denied-resource counts.
- Exercise restore, provider outage, key loss and failed upgrade playbooks on
  isolated deployed artifacts, recording observed recovery times and owners.

## Non-goals

- No full internal SaaS operations console or remote support authority.
- No high-cardinality tenant, principal, token, Skill body or Knowledge
  content labels in hot metrics or exported traces.
- No availability, RPO/RTO or on-call promise inferred from source-only rules
  or an untested local dashboard.
- No replacement for OPS-5 recovery, OPS-6 upgrade, OPS-13 retry or OPS-14
  usage-budget ownership.

## Architecture seam

Instrument at existing gateway, worker and store boundaries using closed
outcomes and bounded cardinality. Keep raw observability optional and private
under the existing Compose/Helm contracts; alert rules consume exported
metrics and do not become product authority. A customer-facing health read
is a new public API operation only if it passes the route catalogue, OpenAPI,
generated console client, Cedar and forced-RLS checks. Support export must
apply a machine-checked sensitive-field allowlist before leaving the host.

## Acceptance criteria

- Every advertised SLO names its measurement formula or percentile, window,
  topology and measured threshold; traffic-free periods, absent scrape
  targets and stale gauges do not silently count as healthy.
- Synthetic policy-stale, provider-503, database-loss, backup-age and worker
  queue failures produce the expected alert with a release marker and a
  tested runbook; restored service clears the condition within the recorded
  bound.
- Default metrics/traces/logs and any support bundle contain no bearer,
  session ciphertext, policy source, Skill bundle or Knowledge body. Tenant
  diagnostics disclose only the caller's authorised aggregate state.
- One supported-host Compose run proves Collector-to-Prometheus delivery and
  telemetry-loss detection; any Helm claim needs a separate named-platform
  run. Alert thresholds are tuned from EVAL-6 load and OPS-5/OPS-6 drills.

## Required tests

- Deterministic metric/recording-rule tests for empty traffic, missing target,
  stale gauge, high cardinality and release-marker mismatch.
- Failure-injection smoke runs against the supported deployment profile,
  including alert fire/clear and runbook execution receipts.
- Security fixture scan of exported telemetry and tenant-scoped health API
  authorization/cross-tenant probes.
- Restore, provider, key and upgrade tabletop or live drills with recorded
  elapsed time, operator action and unresolved prerequisites.

## Rollout and rollback

Ship recording rules and dashboards without paging, measure their behavior
under normal and injected load, then enable alerts after an on-call owner and
thresholds are accepted. Rollback disables alert routing without removing raw
signals or deleting incident evidence. Keep runbooks versioned with the
release marker they describe.

## Dependencies

The deployment owner selects telemetry backend, region, retention and scrape
boundary. Product/operations owners select SLOs, on-call escalation, support
tiers and drill cadence. OPS-5 supplies off-host recovery, OPS-6 supplies a
published upgrade pair, OPS-13 supplies Capture queue-age state, OPS-14 owns
usage budgets and EVAL-6 supplies representative load.

Next: choose the first-offer SLI/SLO and on-call contract, prove the private
Compose telemetry path on a supported host, then publish dark recording rules
and tested runbooks before enabling paging.

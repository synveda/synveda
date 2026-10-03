---
title: "OPS-14: Request limits and tenant usage budgets"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-14: Request limits and tenant usage budgets

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

The gateway verifies credentials, resolves a tenant and applies embedded
Cedar decisions, but its public router has no general request-frequency or
concurrent-work limiter. The shared error taxonomy maps `RateLimited` to 429;
that response shape is not enforcement. Payload bounds limit one request but
do not prevent one tenant or principal from saturating PostgreSQL, extraction
providers or context composition. This is PR-07 in
[production readiness](../PRODUCTION_READINESS.md). The first small-team
Compose offer has one gateway; a medium-team Helm offer may need multiple
gateways and a shared limit.

## Scope

- Define a closed set of admitted operation classes and independent tenant,
  principal and pre-authentication abuse budgets. Bound request frequency and
  in-flight work before expensive database/provider calls while preserving
  a stable, non-oracular 429 response and retry guidance.
- Account for durable tenant usage separately from hot request metrics,
  starting with Session event bytes/rate and ContextRun concurrency. Charge
  only accepted work and make replay/idempotency accounting explicit.
- Keep aggregate limiter-denial, saturation and wait telemetry free of tenant,
  principal, token or resource labels. Provide an authorised tenant-scoped
  usage view through the normal PDP/RLS path.
- Define a bounded operator change/incident override as a governed,
  content-free-audited mutation. Configuration may narrow budgets but never
  bypass Cedar, forced RLS or audit.
- Qualify one-gateway limits first, then select and test a distributed limiter
  store before making any multi-replica fairness claim.

## Non-goals

- No billing, plan-tier or pricing product; usage evidence is not an invoice.
- No limit response that exposes another tenant's counts or denied resource
  existence.
- No in-process counter presented as a cluster-wide quota, and no unbounded
  per-identity Prometheus labels.
- No rate limit that authorises a request Cedar would deny.

## Architecture seam

Admission follows verified credential and tenant resolution for authenticated
routes, before costly work. Separate pre-authentication protection must avoid
using unverified tenant claims as keys. A bounded concurrency permit covers
the entire governed application future and is released on cancellation. The
durable usage ledger belongs in `synveda-store` behind static SQLx and forced
RLS; quota mutations use the public API, embedded PDP, VedaFlow when governed
state changes and content-free audit. An ADR must choose distributed limiter
storage, failure behavior and the accounting clock before implementation.

## Acceptance criteria

- Adversarial mixed-tenant load cannot make one tenant or principal exhaust
  another's admitted request or concurrency budget on the qualified topology.
  A legitimate caller receives stable 429 envelopes and bounded retry advice.
- Rate and concurrency counters remain bounded in memory/storage and recover
  safely after restart. A limiter-store outage has a declared fail-closed or
  conservative-degrade behavior; it never weakens authorization.
- Accepted Session/Context work charges exactly once under replay and
  cancellation; denied attempts do not create a cross-tenant count oracle.
- Tenant usage and operator budget changes are Cedar/RLS-scoped, auditable and
  do not expose bearer or content material.
- A two- or three-gateway test proves cluster-wide limits before Helm replica
  settings are lifted; the one-gateway result is labelled narrowly.

## Required tests

- Exact-role request and tenant-isolation tests for independent budgets,
  idempotent replay, cancellation, expiry and 429 envelopes.
- Mixed public-API load at the EVAL-6 declared agent/event/context envelope,
  including one noisy tenant and provider/database slowdown.
- Multi-gateway restart and limiter-store fault injection after its backend is
  selected; cardinality check for every metric label.
- Governed quota-change and emergency-override audit tests, including a
  foreign-tenant refusal.

## Rollout and rollback

Ship observe-only aggregate counters first, then enforce conservative
per-operation limits for the one-gateway reference after measuring legitimate
traffic. Preserve a tested, audited incident override. Keep Helm pinned until
the chosen distributed backend and cross-pod tests pass. Rollback may disable
new admission limits under incident procedure, but must retain usage/audit
records and cannot bypass PDP, RLS or governed mutation rules.

## Dependencies

The product owner must define operation classes, tenant/principal budgets,
burst windows, pre-authentication treatment, overage behavior and override
authority. The deployment owner must select the distributed limiter store and
its failure contract. EVAL-6 supplies representative load and OPS-7 supplies
multi-gateway acceptance. Record the state and failure semantics in an ADR
before implementation.

Next: select the first-offer one-gateway budget and operation classes, then
ship observe-only aggregate metrics and adversarial tenant-isolation tests.

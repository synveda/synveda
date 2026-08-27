---
title: "AUTHZ-6: Authorisation scale decision"
labels:
  - epic:AUTHZ
  - phase:4
size: S
marker: "de-risk"
---

# AUTHZ-6: Authorisation scale decision

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 4 · **Size:** S · **Marker:** de-risk

## Problem and evidence

The embedded Cedar PDP is the current authority and supports scope ancestry,
grant-derived roles, privacy, quarantine, service confinement and
attribute/time conditions. No measured decision shape currently exceeds it,
and there is no OpenFGA dependency or production service. The original
escape-hatch rationale in [ADR-0002](../adr/adr-0002-cedar-embedded-pdp.md)
predates the epoch-3 scope/grant model, so a spike needs a current trigger
rather than speculative abstraction.

## Scope

Run a time-boxed spike only after a named customer relationship query or
measured Cedar depth/latency limit is documented. Model that exact shared
relationship subset in OpenFGA, feed it the same immutable tenant/scope/grant
snapshot and compare decisions, latency, freshness, outage and operator cost.
Record whether it can be an evidence source behind the PDP seam; do not assume
it can replace Cedar's conditional forbids.

## Non-goals

- No replacement of Cedar, forced RLS, privacy forbids or VedaFlow.
- No dual-authority runtime in production during a research spike.
- No generic policy-engine interface before two exercised implementations have
  a sound shared contract.
- No network call on every decision without a reviewed availability/latency
  budget.
- No support claim from a local happy-path container.

## Architecture seam

synveda-policy keeps one decision contract and closed Action/Resource
vocabulary. Postgres remains authoritative for scopes and grants. A spike
adapter receives a content-free, tenant-qualified relationship projection and
runs shadow comparisons only; every public request still follows Cedar and
forced RLS.

## Acceptance criteria

- The trigger, unsupported Cedar query and measurable success threshold are
  recorded before implementation.
- A conformance corpus covers grants/inheritance, principal-scope privacy,
  group membership, revocation, archived/disabled identities and cross-tenant
  identifiers for the genuinely shared subset.
- The report names every Cedar rule that OpenFGA cannot express and forbids
  silently treating missing conditional logic as permit.
- Freshness, outage, bootstrap, backup, tenancy, operational cost and p50/p95/
  p99 latency are measured at representative scale.
- The outcome is an accepted ADR choosing reject, defer, evidence-only or a
  separately scoped implementation; the spike code is deleted if not needed.

## Required tests

- Deterministic decision-diff corpus with expected denies and forbids.
- Projection idempotency, revoke/disable freshness and cross-tenant tests.
- Network timeout, stale projection and service-unavailable failure tests.
- Load test against the registered decision budget.
- Mutation tests proving privacy and service-confinement differences are
  detected.

## Rollout and rollback

Shadow only, with Cedar authoritative and no user-visible behaviour change.
The spike has a fixed removal date. Any later production rollout requires its
own feature and ADR; rollback stops projection/comparison and deletes no
authoritative Postgres or audit state.

## Dependencies

A concrete relationship trigger or customer mandate, supported scale and
latency budget are required. Security/architecture owners decide whether a
second policy service is acceptable and which party owns its availability,
backup and schema lifecycle.

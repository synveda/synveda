---
title: "AUTHZ-7: Governed admin-plane mutation"
labels:
  - epic:AUTHZ
  - phase:4
size: M
---

# AUTHZ-7: Governed admin-plane mutation

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 4 · **Size:** M

## Problem and evidence

Configuration and policy relaxation effects already use typed VedaFlow
changes, while current scope, group, grant, invite and directory-access
administration is applied directly after PDP, ownership, RLS, idempotency and
audit checks. The old pack-assignment/role-binding description is obsolete.
The open question is narrower: which current authority-increasing or
large-blast-radius admin actions require review/separation of duties, and which
must remain immediate, especially revocation.

## Scope

- Inventory current public admin mutations and classify authority gained/lost,
  subtree blast radius, reversibility and emergency timing.
- Decide in an ADR which actions remain direct and which enter the common typed
  proposal/review/effect lifecycle from
  [ADR-0091](../adr/adr-0091-unified-artifact-approvals.md).
- If gated, define immutable references, expected revision/head, reviewer
  matrix, expiry/cancel and effect-time live re-authorisation.
- Preserve immediate fail-closed revocation, identity disable and emergency
  containment unless the accepted decision proves another safe path.
- Expose pending effects through the existing artifact-neutral review surface.

## Non-goals

- No proposal around every administrative write.
- No parallel access vocabulary or permission table.
- No bypass of Cedar, forced RLS, ownership checks or content-free audit.
- No change to Configuration/relaxation governance already delivered.
- No email delivery or generic workflow engine.

## Architecture seam

The current access and admin-scope handlers remain boundary validation and
PDP seams. Gated effects use VedaFlow's typed references and stale-head
preconditions, then call the same store mutation inside one tenant transaction.
[ADR-0072](../adr/adr-0072-groups-grants-and-invitations.md) remains the
grant/group vocabulary; the common review lifecycle owns review, not access.

## Acceptance criteria

- An accepted ADR records every current admin mutation, risk class and direct
  or reviewed decision before code changes.
- Gated actions cannot be self-authored/reviewed/effected where separation is
  required; stale state and lost authority refuse at effect time.
- Retries create one proposal/effect/audit result and disclose no denied
  subject/scope counts.
- Direct actions have an explicit compensating control, bounded authority and
  behavioural test.
- Revocation/disable remains effective on the next ordinary decision and is
  not blocked behind unavailable reviewers.
- OpenAPI, CLI and console distinguish pending review from applied effect
  without optimistic success.

## Required tests

- Golden mutation matrix across all current role keys and test policy packs.
- Self-review, stale-head, reviewer/effect-actor, revoke-during-review and
  concurrent-effect tests.
- Cross-tenant/foreign-identifier oracle tests for proposals and effects.
- Idempotency/audit-chain assertions for direct and governed outcomes.
- Console/CLI behaviour tests using generated operations only.

## Rollout and rollback

First emit a non-authoritative would-require-review classification metric with
bounded labels and inspect real operations. Enable one mutation family at a
time through governed Configuration. Rollback binds the prior rule/version;
already applied effects remain auditable and are reversed only by the ordinary
inverse action.

## Dependencies

Security/product owners must set separation-of-duties thresholds, emergency
revocation guarantees and reviewer roles. Any new review matrix is an
architectural decision. AUTH-6 and OPS-7 affect revocation propagation but do
not block the classification ADR.

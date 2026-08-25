---
title: "CPR-31: Governed auto-apply and policy relaxations"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-31: Governed auto-apply and policy relaxations

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Complete the one-path auto-apply invariant across Knowledge, Skills, trusted
Tools and governed Configuration, then replace the pre-cut `policy_lapses`
plane with immutable, versioned, time-boxed policy relaxations over the current
scope and Knowledge model. No predecessor row, route, CLI alias, DTO or record-read
translation survives.

## Acceptance criteria

- Knowledge, Skill, Tool and Configuration mutations all create a typed
  VedaFlow `apply` change before policy can auto-apply, retain it for review,
  or reject it; there is no personal fast path around PDP, immutable versions
  or audit.
- A relaxation has a stable aggregate id and immutable content-hashed
  versions. Its terms name one provisioned subject, one non-personal governed
  scope, the closed `knowledge.read` action, a sensitivity ceiling, requested
  start/end, hard expiry and a mandatory reason. Applied versions retain their
  creator, exact approver identities, effective Configuration version/digest
  and whether the matrix auto-applied them.
- Create, revise and revoke are typed `Policy/apply` VedaFlow commands. Each
  repeats ownership, `relaxation.write`, proposal, live approval, payload and
  current-version checks before moving authority. Personal policy may
  auto-apply the same change; stricter profiles return pending review or a
  terminal rejection.
- Authorization loads only unrevoked relaxations inside their effective
  window, applies the current governed Configuration ceiling, and feeds them
  into the embedded Cedar decision. Denied/private targets and service-token
  confinement remain invariant; no post-decision override exists.
- Public cursor-paginated APIs, generated console operations and public-HTTP
  CLI commands cover list/show/create/revise/revoke/history. Reads decide per
  aggregate; idempotency keys and revision preconditions are required.
- Hard expiry restores denial without a job. A background pass records each
  expiry once on the hash chain; revocation and expiry retain content-free
  evidence.
- The predecessor table, its store/type/route/effect surface and pack JSON setting
  are deleted without migrating old rows. New tenant tables use enabled and
  forced RLS and join the completeness gate.
- Focused domain, PDP, gateway, RLS, OpenAPI, CLI and console tests, the
  isolated acceptance demo, `make ci` and `make db-test` pass.

## Evidence

Delivered 2026-08-25 from `ed7d233` under accepted ADR-0090. Migration
`0056_governed_policy_relaxations` adds three forced-RLS tables and removes the
predecessor table and policy-pack setting without translation. The generated
contract contains 164 operations and 260 schemas; `synveda relaxation` and the
Scopes/Configuration console surfaces consume that public contract only.

Focused acceptance passes for the domain (210 unit tests plus 50 serde tests),
Cedar (3 relaxation cases and the complete pack suites), public gateway (2),
RLS/immutability/database-time expiry (2), OpenAPI parity (6), audit (27), CLI
(155 plus 5 MCP corpus cases), console (209 plus production build) and
retrieval (53). The isolated `demos/cpr-31-governed-relaxations.sh` proves
auto-apply, pending review, rejection, revision, revocation, exact-subject
Knowledge access and absence of the retired table. Complete `make ci` and
`make db-test` pass.

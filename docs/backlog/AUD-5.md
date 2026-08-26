---
title: "AUD-5: Compliance mapping doc"
labels:
  - epic:AUD
  - phase:4
size: M
marker: "Phase 3"
---

# AUD-5: Compliance mapping doc

**Epic:** AUD — Audit (functional requirement) · **Phase:** 4 · **Size:** M · **Marker:** Phase 3

## Problem and evidence

Synveda documents technical security boundaries and production gaps, but it
does not have an owner-reviewed mapping from a selected control framework to
implemented evidence, operating controls and residual gaps. Repository tests
cannot establish SOC 2, ISO 27001 or DORA compliance, and an agent must not
invent legal scope or certification claims.

## Scope

- With a named compliance owner, select the exact framework versions,
  organisational scope, deployment model and control owners.
- Map each applicable control to current code/ADR, executable evidence,
  operational procedure, evidence frequency and unresolved gap.
- Distinguish product capability from customer/operator responsibility and
  design effectiveness from recurring operating evidence.
- Link gaps to a small set of implementation-ready feature records and
  [production readiness](../PRODUCTION_READINESS.md).
- Establish review/version ownership so the mapping cannot silently become
  stale.

## Non-goals

- No certification, legal opinion or assertion of compliance.
- No copied framework text beyond licence/quotation limits.
- No fabricated policies, incidents, restore drills or control operation.
- No mapping to every framework before one scoped review proves the method.
- No duplicate source of truth for architecture, features or test results.

## Architecture seam

This is a governed documentation/evidence index, not runtime code. It links
accepted ADRs, SECURITY.md, operator runbooks, generated contracts, audit
exports and CI/drill artifacts by stable control/evidence IDs. Volatile counts
and results remain in their authoritative generated reports.

## Acceptance criteria

- The compliance owner approves framework/version, scope, responsibility
  boundary and applicability decisions.
- Every applicable control names an owner, current status, concrete evidence,
  test/monitor frequency, retention and gap; unsupported controls say so.
- Links resolve to current artifacts and no control relies only on marketing
  prose or a proposed ADR.
- An external checklist/reviewer samples evidence and records findings without
  the document claiming certification.
- Every P0/P1 gap maps to an existing backlog item or one approved new item,
  with no duplicate tickets.
- A scheduled review detects stale feature status, broken links and expired
  evidence.

## Required tests

- Internal link and referenced-feature/ADR status checks.
- Sample evidence-reproduction run for each control family.
- Review checklist proving owner, date, framework version and evidence expiry
  are present.
- Negative review for unsupported claims and product/operator responsibility
  confusion.
- External review when credentials/contracts permit; otherwise record it as
  unavailable.

## Rollout and rollback

Begin with one owner-selected framework and a draft explicitly marked not a
certification. Publish only after legal/compliance review. Supersede mappings
in place with version history in git; rollback restores the prior reviewed
version and withdraws any claim based on expired evidence.

## Dependencies

Legal/compliance owners must choose framework, scope, auditor, evidence
retention and publication language. OPS-5, AUD-3, AUD-4, AUTH-6 and other
readiness gaps supply operating evidence only after their acceptance passes.
A public licence and security-response process may also be prerequisites.

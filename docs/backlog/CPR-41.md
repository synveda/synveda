---
title: "CPR-41: One-command realistic product demo"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-41: One-command realistic product demo

**Epic:** CPR — Context-platform redesign · **Phase:** 5 · **Size:** XL

## Description

Package the PulseBoard personal/small-team acceptance story as a resumable
authenticated CLI walkthrough over the same public application API used by the
console and adapters. Keep governed mode available without making proposals
the default product vocabulary.

## Acceptance criteria

- `synveda demo start --profile personal|team|governed`, `demo status` and
  `demo reset --force` are real commands with no retired aliases.
- The walkthrough creates a workspace, project and repository, records and
  closes sessions, extracts and reviews candidates, separates private from
  project Knowledge, proves clean-session reuse, records an explicit
  supersession and links the current context trace.
- Team mode uses a distinct real principal when credentials exist; otherwise
  it issues a one-time invitation and explicitly declines to claim teammate
  verification. No invitation token enters the resume receipt.
- One release Skill, one MCP manifest and one OKF v0.2 troubleshooting bundle
  traverse their existing public governed flows. Pending review is reported as
  pending and never presented as activation, pinning or binding.
- All product data is seeded through supported public APIs and VedaFlow. No
  product-table insert, direct store path, deleted route, fake semantic label,
  profile/edition branch or second gateway exists.
- The first exact canonical Configuration and its first binding can resolve
  the fresh-tenant approval cycle only for a live administrator, with typed
  VedaFlow/PDP/audit evidence and transaction-serialised one-winner semantics.
- A mode-0600 atomic receipt makes steps idempotent and resumable. Reset
  archives only receipt-owned current objects and retains immutable/audit
  history.
- Focused CLI/configuration/concurrency and PulseBoard acceptance tests, the
  CPR-41 demo, demo drift gate, `make ci` and database suite pass. Any absent
  live second principal or semantic provider remains explicitly labelled.

## Evidence

Implementation starts from `036eab7` under ADR-0100. CLI demo tests pass 4/4;
the fresh-profile, edited-document and concurrent-bootstrap database suite
passes 4/4; and the consolidated PulseBoard team loop passes 1/1.
`demos/cpr-41-one-command-demo.sh`, the complete 87-script drift gate,
`make ci` and the fresh-scratch `make db-test` pass. The latter used
`synveda_test_44030`, which its harness removed on success. No current gateway,
unexpired Alice/Bob credentials or separately authenticated semantic provider
were available, so the packaged command is not misreported as a live run. The
resulting commit is recorded by CPR-42.

---
title: "AUTHZ-7: Governed admin-plane mutation"
labels:
  - epic:AUTHZ
  - phase:4
size: M
---

# AUTHZ-7: Governed admin-plane mutation

**Epic:** AUTHZ — Authorisation & policy (functional requirement) · **Phase:** 4 · **Size:** M

## Description

Pack assignment and role binding are direct `PUT`s (AUTHZ-2, AUTHZ-3) while every
content act is proposal-gated. Decide whether they gain an approval-matrix cell of
their own, and record the answer either way.

## Why this exists

Filed 2026-08-05 by CNSL-2 (ADR-0058 decision 9), which found it by building the
screen that renders a pack, its origin, the roles under it and the grants over it
on one page — at which point the next question a reader asks is who changed any of
it, and under what review.

All three packs grant `PolicyAssign` to `steward` and `org-admin` over the bound
subtree, and the decision deliberately skips the node's own assignment (ADR-0014
decision 4) so a restrictive pack cannot seal its own node. Together those mean
**one steward replaces a team's pack with one call and one signature,
permanently** — while the **lapse** that relaxes far less requires a reasoned,
time-boxed, dual-approved proposal that expires on its own (AUTHZ-4, ADR-0037).

Seed §2.3 has controls relaxed "through explicit, audited, time-boxable policy
relaxations". A pack assignment is explicit and audited (`policy.node.assigned`).
It is neither of the other two.

## What bounds it

Both bounds were verified when the finding was recorded, and both hold:

- **A pack flip cannot widen anyone's candidate universe.** The universe is the
  caller's placement chain and it widens by lapse and by nothing else (ADR-0037
  decision 13) — which is exactly why EVAL-5's governed-relaxation demo had to be
  a lapse rather than a pack flip.
- **A pack cannot reach below the invariant floor** (ADR-0032 decision 4,
  ADR-0051 decision 18, ADR-0052 decision 3).

What a pack assignment *does* move, for a whole subtree: approval counts,
sensitivity ceilings, scan thresholds and quality bars.

That is why this is filed as a governance question rather than as a hole, and why
it sits in Phase 4. If either bound is ever found not to hold, it belongs in front
of the Phase 3 procurement block rather than behind it.

## Acceptance criteria

- The decision recorded as an ADR **before** any implementation, per the standing
  rule that architectural choices get an ADR first.
- **If gated:** the admin-plane cell joins the role×action and approval golden
  tests under all three packs; `policy.node.assigned` and `role.bound` become
  proposal effects; and the explorer gains the write path CNSL-2 declined
  (ADR-0058 decision 9).
- **If left direct:** the seed §2.3 reading that permits it is written down, and
  the compensating control is named rather than assumed.

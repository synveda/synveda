---
title: "CNSL-2: Hierarchy & policy explorer"
labels:
  - epic:CNSL
  - phase:3
size: M
---

# CNSL-2: Hierarchy & policy explorer

**Epic:** CNSL — Admin console (Phase 3) · **Phase:** 3 · **Size:** M

## Description

visualise scopes, packs, roles, active lapses.

## Acceptance criteria

Written 2026-08-05 (ADR-0058). The feature arrived with four nouns and no
criteria. Three of the four already had read surfaces — scopes (HIER-1), packs
with their origin (AUTHZ-2), roles and lapses — so most of this is a second
renderer on the toolchain ADR-0056 bought. The criteria below are the parts
that are not.

- **The four nouns are answered for a node in one screen and by the CLI beside
  it.** ADR-0056 decision 9 — the console gets no endpoint the CLI does not have
  — is a standing decision, and `synveda policy` has no read verb while `lapse`
  is not a verb at all. A product whose lapse machinery is its whole answer to
  "strict by default, relaxable by design" (seed §2.3) has no terminal in which
  to ask what is currently relaxed.
- **A pack renders with where it came from, and so do roles.** Assigned here,
  assigned at a named ancestor, the tenant default, the embedded default — and
  the effective role set with each binding's origin node, in the same
  vocabulary. This is the asymmetry the feature closes: policy has served an
  origin since AUTHZ-2 and roles have served only the bindings at the node
  asked about, so the inheritance every reader needs was a walk each client did
  for itself.
- **"Active lapses" is answerable without already knowing which scope to ask
  about.** The scope-free list returns the standing set the caller may see, each
  lapse visible from **either** end under `PolicyRead` at that end — never under
  a tenant-wide grant nothing below an org-admin holds. That is what lets the
  steward of a *granted* scope list, and therefore revoke, a grant their own
  team holds; `at_target` had made a standing grant visible only to the side
  that disclosed it.
- **The standing set is the PDP's own predicate.** `active_for_scopes`,
  unrevoked and unexpired on the database's clock, so the screen and the PDP
  cannot disagree about which grants are live. The scoped form keeps returning
  expired and revoked rows, because "who could read what, when" is a question
  about history.
- **What the reader may do is the PDP's verdict, never a re-derivation of it.**
  The answer carries the pack `name@version` it was decided under, and is
  asserted to move when a lapse opens and when a pack is assigned — neither of
  which a role-derived answer can express.
- **A capability is a forecast rather than a grant.** Proved by a probe that
  answers yes, a pack change, and the same act then refused at its own seam. The
  enforcement point is unchanged; a client may decide what to *offer* and never
  what to *allow*.
- **The probe answers about the caller and takes no `subject`.** An explorer
  must not become an enumeration oracle for an organisation's role assignments,
  one 403 at a time. "Who holds what here" keeps `RoleRead` and its own denial.
- **A 10,000-node hierarchy renders** (HIER-1's own AC) without fetching a
  subtree or probing a node nobody looked at: children on expand, capabilities
  batched for the rendered set under a maximum the API declares, with the
  response naming what it did not answer rather than truncating silently.
- **The whole render chains one `authz.decision` per probe request**, pairs
  summarised, rather than one row per (node, action) — ADR-0019 decision 4's
  second sentence arriving on the admin plane. Asserted as a count on the chain
  rather than as a claim: a governance product whose audit log is mostly a
  record of people looking at it has made its own chain unreadable.
- **CNSL-1's deferral closes where it was sent.** The inbox offers the acts the
  reader may actually take, and a reader holding one role short is shown a
  refusal rather than a button that returns one.
- **The parity corpus takes four new cases** — effective roles with mixed
  origins, a pack inherited from two levels up, a standing lapse beside an
  expired one, and a capability set with at least one denial — asserted by both
  renderers and checked for teeth the way CNSL-1's was, by deleting each fact
  and naming which case fails and which do not.
- **Every act is on the chain**, with no third party's binding and no lapse
  reason in any probe payload, swept for.
- **Demo script.**

Deferred with recorded triggers (ADR-0058): **policy simulation** — "what would
this scope compose under `standard`" — because a forecast against a pack nobody
assigned is a second decision path through the PDP, and the honest version
decides against a supplied pack rather than the effective one; and **mutation
from the explorer**, because assigning a pack and binding a role are direct
routes where content is proposal-gated, and adding a second direct mutation
surface would settle by accident a question that is **AUTHZ-7's** to answer on
purpose — filed by this feature, which found that all three packs let one
steward replace a subtree's pack with one call and one signature while the lapse
that relaxes far less needs a reasoned, time-boxed, dual-approved proposal.

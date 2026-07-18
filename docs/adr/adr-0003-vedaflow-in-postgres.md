# ADR-0003: VedaFlow implements git semantics natively in Postgres, not on git repos

- **Status**: Accepted
- **Date**: 2026-07-18
- **Feature(s)**: FND-6, FLOW-1..8
- **Deciders**: sujitn

## Context

VedaFlow treats organisational knowledge like code: memories, context
packs, prompts, skills, and policies flow through propose → review →
approve → publish, with approval authority derived from the hierarchy
(tech plan §2). The mechanics are deliberately git-*like* — content
addressing, commits, refs, branches-with-meaning (derived/staged/
published). The question is the substrate. Approvals are authz-checked
actions; every commit must record the policy pack in force; audit entries
are hash-chained (AUD-1); tenant isolation is enforced by row-level
security (TEN-2); and `inject` reads published refs on a p99 < 150ms
budget. All of that must be transactional with the content itself.

## Decision

VedaFlow is implemented as native Postgres tables — content-addressed
(BLAKE3) `objects`, `trees`, `commits` (with author identity, signature,
and policy_snapshot_hash), `refs`, and `proposals` — not as bare git
repositories. Channels (`derived`/`staged`/`published` per scope per asset
type) are rows in `refs`. A **git bridge** (gitoxide) mirrors published
channels outward to real repos for GitHub/GitLab visibility — export
first, import later.

## Options considered

1. **Git semantics in Postgres (chosen)** — proposals, approvals, policy
   checks, audit chaining, RLS, and ref updates commit in one transaction
   with the content; refs are queryable with SQL alongside records and
   bitemporal history (as-of inject joins refs + ADR-0006 tables); no
   filesystem state to replicate or back up separately. Con: we own the
   object model and its maintenance (packing/GC when history grows).
2. **Bare git repos via gitoxide as the store** — battle-tested object
   model for free, but git has no row-level tenant isolation, no
   transactional link to Postgres records (two-phase commit between a repo
   and the database on every observe pipeline write), no SQL over refs for
   composition, and thousands-of-tenants × scopes × asset-types means a
   filesystem repo sprawl with its own backup/HA story — reopening
   everything ADR-0001 closed.
3. **External forge (GitHub/GitLab) as system of record** — inherits PR
   review UX, but puts a cloud service in the core path (forbidden, seed
   §7), caps approval logic at forge features (our approval matrix derives
   from asset × sensitivity × scope × policy pack), and makes every inject
   dependent on forge availability.
4. **Plain version-number tables, no content addressing** — simplest, but
   loses dedup of identical content, cheap lineage, and — decisive —
   commit-hash watermarks in injected context, which is what makes "what
   did the agent know on March 3rd" reproducible (tech plan §2.5).

## Consequences

- Positive: rollback is a ref move, fleet-wide on next session start;
  every published sentence of context traces to an author, an approval,
  and a recorded policy version; identical content dedups by hash;
  auditors read proposals, not database rows; the bank-mode switch
  (published-only injection) is a policy predicate over channels, not a
  code path.
- Negative / accepted trade-offs: we re-implement a subset of git
  (objects/trees/commits/refs) and must property-test it (FLOW-1 AC:
  identical content dedups; history immutable under concurrent writers);
  object growth needs a packing/GC story eventually; the git bridge is an
  extra component — but export-only, so its failure never blocks the core.
- Reversal trigger: none realistic for the storage substrate (the
  transactional-with-RLS requirement is structural). If bridge users need
  *import* (reviews happening on the forge flowing back), that is a FLOW-8
  extension proposal, not a substrate change.

## Compliance notes

Commits carry `policy_snapshot_hash` — an auditor can prove which policy
pack governed any published asset at creation time. Proposal state
transitions and approvals are authz-checked actions emitting AUD-1 events,
and RLS applies to all five tables, so a tenant's knowledge history is
isolated by the same mechanism as its records. Signatures on commits
survive the git bridge round-trip (FLOW-8 AC), keeping external mirrors
verifiable.

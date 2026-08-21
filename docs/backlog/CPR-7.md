---
title: "CPR-7: The hierarchy cutover — one scope tree"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-7: The hierarchy cutover — one scope tree

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Prompt 7 of the 33-prompt context-platform programme, and the one six earlier
records deferred to: the prompt that deletes the old fixed hierarchy whole.
`hierarchy_nodes`, `hierarchy_closure` and `role_bindings` leave the schema;
the rank vocabulary (`org`/`division`/`department`/`team`/`user`, `rank()`,
the child-outranks-parent rule, the root-must-be-an-org CHECK) leaves the
types; `/v1/hierarchy/*`, `synveda hierarchy`, `synveda role bind`, the
placement-based quarantine convention, the `synveda-{dept}-{team}` JIT
convention, `group_mappings`, the scope-chain cache and the console hierarchy
explorer leave the product.

What replaces them:

- **Six public admin routes** over governed scopes — `GET/POST
  /v1/admin/scopes`, `GET/PATCH /v1/admin/scopes/{scope_id}`, and
  `GET /v1/admin/scopes/{scope_id}/ancestors|descendants` — PDP-decided,
  audited, idempotent on creation, with pack assignment
  (`GET/PUT/DELETE …/policy`) and the VedaFlow curator file
  (`GET/PUT …/curators`) re-homed under the same prefix.
- **Five operator CLI commands** — `synveda scope list|show|create|move|tree`
  — driving those routes.
- **One decision-gathering path**: the gateway's old-plane gather is deleted;
  every route resolves chains from `scope_closure` and reads
  `context.roles` from grants alone.
- **Placement as identity**: an identity's scope is its own principal scope,
  minted at first login for users, services and directory identities alike;
  the first admin-group login mints the tenant's first grant (an
  `administrator` grant at the tenant root) — closing ADR-0073's recorded
  operator gap.
- **Grant keys everywhere**: the old `Role` vocabulary is deleted from
  proposal approvals, curator files, the VedaFlow approval matrix and every
  Cedar role list.

Decisions in ADR-0074.

## Why this exists

The old hierarchy is the last place the product insists its smallest unit is
an organisation. It is also a second scope model: two trees, two role
vocabularies, two chain resolvers, and a projection seam between them that
CPR-6 built and this prompt deletes. Every prompt since CPR-3 has recorded
the coexistence as debt — the identity plane that could not use the
membership model because `identities_scope_fk` pointed at the old tree, the
admin surface that administered the tree being deleted, the first-grant gap
whose break-glass door was a role binding on that tree.

Deleting it is not a refactor; it is the cutover the epoch exists for. No
data migrates, no alias survives, and a pre-cutover database is refused with
the reset instruction.

## Design

See ADR-0074 for the six decisions: one tree; one gather; placement is
identity not convention; the admin door is a grant; administration is public
and typed (with the two re-homed sub-surfaces); the approval matrix speaks
grant keys. The scope substrate's own design — shapes not ranks, where each
rule is enforced — is ADR-0070's and is unchanged by this feature.

## Acceptance criteria

- Every `/v1/hierarchy` route answers **404**, and every old scope kind
  (`org`, `division`, `department`, `team`, `user`) **fails validation by
  name** — asserted as negative API tests, not implied by route deletion.
- The admin routes create, rename, archive and **move** scopes; each mutation
  is PDP-decided against the scope it is about and audited, a move recording
  both ends; creation is idempotent under `Idempotency-Key`; a move into the
  scope's own subtree is refused; a cross-tenant move is unrepresentable.
- The memory plane — observe, inject, recall, channels, proposals, skills,
  prompts, lapses, quarantine review — decides over governed scope chains and
  grant role keys with the old chain cache and the old bindings gone.
- An identity's scope is a principal scope minted at first login — no
  hierarchy row, no quarantine node, no group convention — and a first-time
  `synveda-admins` login mints the tenant's first grant with no break-glass
  step.
- Pack assignment at a scope governs its subtree through `scope_closure`.
- The approval matrix, proposal approval records and curator files speak
  grant keys only; the two invariant floors read `administrator` and
  `reviewer`.
- The migration chain is rewritten in place (scope substrate at `0004`,
  `role_bindings` and the hierarchy deleted, epoch bumped) and a fresh
  database is the only database this build accepts.
- RLS completeness holds: `hierarchy_nodes`, `hierarchy_closure` and
  `role_bindings` leave the adversarial inventory and nothing unforced
  replaces them.
- The approval matrix's scope-kind cells **partition the five shapes**: no
  shape — the tenant root least of all — falls through to auto-approve
  because the re-vocabulary forgot to name it.

## Standing after this feature

**A grant does not yet widen what a session composes.** `composition_plan`
walks the caller's chain — their own scope outward to the tenant root — and
anchors reach it only as decision context. With placement gone that means an
agent's `inject` sees its own scope and the tenant root and nothing else:
joining a workspace gives that session nothing. Making the candidate set the
anchor set is the composition contract's re-cut, owed to Prompts 16–18
(Stage D). Until then, material meant for a reader has to live on that
reader's chain.


Four demos whose subject is the deleted model go with it (`hier-1`,
`hier-2`, `hier-3`, `authz-3`) and the programme's own `cpr-4`, `cpr-5`
and `cpr-6` are re-cut onto the grant bootstrap. **Forty-three Phase-3
demos still seed through `role bind`, `hierarchy_closure` inserts or
`/v1/hierarchy`, and will fail at that line.** They are not re-cut here:
each belongs to a subsystem a later prompt of this programme re-anchors,
and no CI target runs them — which is why the number is recorded rather
than left to be discovered.

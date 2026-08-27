# ADR-0074: The hierarchy cutover — one scope tree, admin surface, grant bootstrap

- **Status**: Accepted
- **Date**: 2026-08-20
- **Feature(s)**: CPR-7
- **Deciders**: Prompt 7 of the CPR programme

## Context

CPR-3 built the governed scope substrate, CPR-4 and CPR-5 built product surfaces
on it, and CPR-6 re-cut the PDP over it — and every one of them left the old
fixed hierarchy standing beside the new tree, explicitly untouched until "the
prompt that deletes it whole" (ADR-0070 decision 1; ADR-0073's record). Two
models therefore coexist: `hierarchy_nodes` with its rank vocabulary
(`org`/`division`/`department`/`team`/`user`, child-outranks-parent, the
root-must-be-an-org CHECK) deciding the memory plane's chains, and `scopes`
with five shapes deciding the workspace plane. CPR-6 projected the old tree
into the new vocabulary at the caller's edge (`ScopeNode::from_hierarchy`) so
both could decide; nothing synchronised them.

That coexistence is this programme's largest standing debt. An identity still
needs a `hierarchy_nodes` row (the FK `identities_scope_fk`), so the membership
model had to make a principal a token subject rather than an identity row
(ADR-0072 decision 4) — one whole subsystem contorted to avoid a
synchronisation ADR-0068 decision 3 forbids. The product's admin surface
(`/v1/hierarchy/*`, `synveda hierarchy`, the console explorer) administers the
tree being deleted. And nothing mints a tenant's first grant (ADR-0073's
recorded gap): the old break-glass door was an `org-admin` *role binding* on
the tree that is going away.

This prompt is the cutover the record promised: delete the old hierarchy
whole — tables, rank vocabulary, role bindings, routes, CLI, DTOs, fixtures,
and the compatibility projection — and put the admin surface and the
bootstrap on the governed scope model.

## Decision

1. **One tree.** `scopes` + `scope_closure` (ADR-0070) are the only scope
   tree. `hierarchy_nodes`, `hierarchy_closure`, `role_bindings`,
   `ScopeKind {org…user}` with `rank()`, `HierarchyNode`, `Role`,
   `RoleBinding`, `ScopeNode::from_hierarchy*` and the placement-based
   quarantine convention are deleted. Nothing translates; no row moves.
   The schema epoch bumps to 2 so every pre-cutover database is refused by
   the CPR-2 guard with the reset instruction rather than by a migration
   checksum error. Because a fresh database is required, the migrations are
   rewritten in place: the scope substrate moves to `0004` (where the
   hierarchy was), `identities` and `policy_pack_assignments` foreign-key
   `scopes`, `group_mappings` leaves `identities` with the convention it
   overrode, the `role_bindings` migration is deleted, and the
   migration count 43 → 41 with gaps at `0009` and `0040` (the
   `role_bindings` slot and the substrate's old home; Prompt 33 squashes
   what remains).

2. **One gather.** The gateway's two decision-gathering paths collapse into
   the governed one (ADR-0073): every route's resource chain comes from
   `scope_closure`, the caller's own chain starts at their principal scope,
   and `context.roles` carries grant role keys only. The `ScopeChainCache`
   (HIER-2) is deleted with its tree; chains resolve per request through
   `scopes::ancestors`, and the post-mutation cache-flush seam narrows to the
   PDP's entity invalidation. HIER-2's latency discipline transfers to the
   anchor resolver, which is now on every request's critical path.

3. **Placement is identity, not convention.** An identity's `scope_id` is its
   own principal-shaped scope, minted at first login by
   `ensure_principal_scope` (CPR-6) — for users, services and
   directory-projected identities alike. The `synveda-{dept}-{team}`
   convention, `group_mappings` overrides and the reserved `quarantine` scope
   are deleted: a principal with no grants can reach nothing beyond their own
   scope because the anchor model and the base-layer privacy floor (ADR-0073
   decision 4) already say so, which is the new model's version of
   AUTH-2's fail-closed placement. Unmapped no longer means quarantined; it
   means *ungranted*, and that is decided per action rather than per person.
   A directory user's scope is keyed by the directory's `externalId` and
   adopted at first login through the existing correspondence rule
   (ADR-0059 decision 4); the identity row, not the slug, is the binding the
   anchor resolver reads.

4. **The admin door is a grant.** The `synveda-admins` IdP-group convention
   now upserts an `administrator` grant at the tenant root scope at every
   login completion — additive only, first establishment audited, exactly the
   ADR-0015 decision 6 shape on the new noun. This replaces the tenant-wide
   `org-admin` binding and closes ADR-0073's recorded gap for the *operator*
   door (first login of an admin-group member mints the first grant a tenant
   holds); admission-level bootstrap (a tenant with no IdP admin group) is
   still break-glass: direct store seeding, as CPR-6's harnesses already do.

5. **Administration is public and typed.** Six routes under
   `/v1/admin/scopes` (list, create, get, patch, ancestors, descendants),
   derived from the CPR-3 store services, PDP-gated by re-named actions
   (`HierarchyCreate/Read/Update/Delete` → `ScopeCreate/Read/Update`; scope
   deletion stays a non-goal — archiving is a status), creation under the
   CPR-4 idempotency discipline, every mutation audited (`scope.created`,
   `scope.updated` with a change list; `hierarchy.node.*` events are
   deleted). Scope movement is a PATCH of `parent_scope_id` decided by the
   PDP against the destination and audited with both ends. **No VedaFlow
   step**: scope structure is placement, not knowledge — the governed
   artifact family is ADR-0068 decision 7's, and a review workflow over
   moving a folder would govern nothing the PDP does not already decide.
   Two adjacent sub-surfaces are re-homed rather than invented, because
   their capability must survive the tree they hung on:
   `GET/PUT/DELETE /v1/admin/scopes/{id}/policy` (pack assignment, AUTHZ-2)
   and `GET/PUT /v1/admin/scopes/{id}/curators` (the VedaFlow curator file,
   ADR-0032 decision 3). Five CLI commands (`synveda scope
   list|show|create|move|tree`) drive the routes.

6. **The approval matrix speaks grant keys.** `Role` (viewer, contributor,
   curator, steward, org-admin, auditor, security-reviewer, compliance) is
   deleted; the VedaFlow approval matrix, proposal approval records, curator
   files and every Cedar role list speak `RoleKey` (owner, member, viewer,
   reviewer, curator, administrator) exclusively. The two invariant floors
   re-vocabulary: `compliance` → `administrator`, `security-reviewer` →
   `reviewer`. One vocabulary, one decision point — ADR-0072 decision 2
   extended from storage into approval.

7. **The quarantine review plane is tenant-anchored.** Decision 3 puts every
   quarantined observe event at a `principal`-shaped scope, and a principal
   scope inherits nothing (ADR-0072) — so a verdict decided at the event's own
   scope was reachable by nobody except the person whose secret it was, which
   is the one reader review exists to exclude. `QuarantineRead` and
   `QuarantineReview` therefore decide at `Resource::Tenant`, which is where
   the queue's tenant-wide branch already decided and which matches the packs'
   own treatment of this control ("how a security control is reviewed does not
   loosen per pack"). A `scope_id` on the queue stays a **filter** — the
   uniform-404 ownership check still runs on it — and `Scope` stays in the
   action's schema so a stored pack can still write a narrower rule. The
   personal-scope privacy floor is left closed rather than carved: the plane
   moved out from under it instead.

8. **Your own scope is yours, and a proposal climbs by neighbourhood.** Two
   consequences of "a principal scope inherits nothing" met the memory plane
   and had to be answered.

   `ensure_principal_scope` now mints an `owner` grant for the principal at
   its own scope, in the same transaction — the rule CPR-5 already applies to
   every other thing somebody owns (creating a workspace or a project mints an
   `owner` grant for its creator). Without it the person whose memory it is
   held no role key there and could not publish, propose about or govern their
   own material, while the privacy floor happily let them read it. That was not
   a policy anybody wrote; it was the absence of the one grant the model mints
   everywhere else.

   And the `ProposalOpen` membership floor reads `principal in resource ||
   (resource in principal.tenant && resource in principal.ambit)`. Until CPR-6
   `principal in resource` was enough because placement made every ancestor of
   your home an entity parent; anchors are deliberately not parents (ADR-0073),
   so it narrowed to your own chain and the climb the floor exists for stopped
   being sayable. `ambit` — the parent of every scope you hold — is exactly one
   hop of the gradient and no more. The tenant guard is not decoration: without
   it an anchor naming a foreign scope would widen the ambit across the tenant
   boundary.

## What this cutover does **not** carry, and who does

**A grant does not widen what a session composes.** `composition_plan` walks
the caller's chain — their own scope outward to the tenant root — and anchors
reach it only as *context*, widening what a pack permits at a chain scope. So
since decision 3 removed placement, an agent's `inject` sees its own scope and
the tenant root and nothing else: joining a workspace gives that session
nothing. The candidate set becoming the anchor set is the composition
contract's re-cut, which §9 of the implementation record assigns to Prompts
16–18 (Stage D), and beginning it here would be beginning a later prompt. Until
then, material meant for a reader has to live on their chain — which is what
every fixture in this cutover was re-cut to do, and what `demos/cpr-7-scopes.sh`
demonstrates.

9. **`materialise` carries anchors and groups, not only `chain`.** Found
   verifying this cutover, not designed into it: `synveda-retrieval`'s
   `materialise` — the one call per `composition_plan` invocation that
   builds the Cedar entity batch every later decision reuses — supplied
   `AuthzContext::default()` (`anchors: &[]`, `groups: &[]`) rather than
   the caller's actual sets. `entities_over` (`synveda-policy`) bakes
   `principal.ambit`/`principal.anchors`/`principal.private` into the
   Principal *entity* at that one call; every subsequent per-scope
   decision supplies only Cedar's request `context` (roles, sensitivity,
   lapsed) and cannot repair an entity already built without them. The
   practical effect: `standard`'s ambit-sharing permit, the private-scope
   door, and every group-anchored grant were unreachable through any
   composition plan — inject, recall, or a lapse's widened read — since
   CPR-6 minted the anchor model those attributes serve. Decision 3 above
   depends on this (an identity's own scope reaching through composition
   at all is the `own_scope` permit, unaffected — but the *sharing*
   decisions 4 and 8 exist to widen do go through this path), so it is
   recorded here rather than left as an unattributed test fix. Fixed by
   passing `inputs.anchors`/`inputs.groups` into the materialise-time
   context; nothing about which entities exist or how a decision reads
   them changes otherwise.

## Options considered

1. **Keep both trees, admin surface over the old one** — zero deletion risk,
   but it is the status quo this programme exists to end: two vocabularies,
   a projection seam, and an identity plane that cannot use its own
   membership model. Rejected (ADR-0068 decision 3 forbids the
   synchronisation this requires).
2. **Delete the tables, keep the old `ScopeKind`/`Role` types as wire
   compatibility** — rejected outright: pre-1.0 hard cut; a serde alias here
   is exactly the "compatibility deserialisation for old scope kinds" the
   programme refuses.
3. **Full chain squash now** — rejected: Prompt 33's, for CPR-2's recorded
   reason (a squashed chain leaves the epoch guard without a pre-cut
   database shape to be tested against).

## Consequences

- Positive: one scope tree, one decision path, one role vocabulary; the
  identity plane un-contorts (an identity *is* a principal scope owner);
  the admin surface, the console explorer and the eval seeding all drive
  public, typed, OpenAPI-documented routes; the first-grant gap has an
  operator door.
- Negative / accepted trade-offs: the HIER-2 warm cache goes with its tree
  (chains resolve per request; inject's p50 budget absorbs it — the closure
  scan is one indexed read); `policy_pack_assignments` semantics are
  unchanged but a pre-cutover database cannot reach the new chain
  (reset, by design); the approval floors lose their specialist names until
  Prompt 27 re-cuts the matrix over artifact versions; JIT placement from
  IdP groups is gone — a person arrives at their own scope, and reaching
  anything is a grant (directory adapters rebuild placement as enterprise
  profile configuration in Prompt 29).
- Reversal trigger: none — this is a locked deletion (ADR-0068 decision 3).
  The performance trigger that would demand work: inject/recall chain
  resolution measurably exceeding its budget on production-shaped storage,
  which would call for caching the anchor/chain resolution (ADR-0073
  decision 7's deferred optimisation), never for the old tree's return.

## Compliance notes

- **Tenancy**: no table loses its tenant binding or forced RLS; `scopes`,
  `scope_closure` and `policy_pack_assignments` were already tenant-bound
  and forced, and the re-pointed FKs stay composite over `(tenant_id, …)` so
  a cross-tenant identity/assignment edge remains unrepresentable. The RLS
  completeness inventory drops `hierarchy_nodes`, `hierarchy_closure` and
  `role_bindings` and gains nothing.
- **PDP**: no governed read or mutation loses its decision; the number of
  decision *sites* is unchanged, and `context.roles` narrows to one
  vocabulary. The base-layer privacy floor and the service-identity
  confinement forbid are untouched.
- **Audit**: `hierarchy.node.*` and `role.bound/unbound` events are deleted
  with their actions; `scope.created/updated` replace them, and the admin
  grant's first establishment chains `access.granted` as CPR-5 already
  defined. The chain's shape (hash-chained, append-only) is unchanged.
- **Contract**: `/v1/hierarchy/*` is removed from the router with no alias;
  the OpenAPI document gains the admin scope plane; console types are
  regenerated from it. Old scope kinds fail validation by name.

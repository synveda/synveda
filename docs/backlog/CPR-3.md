---
title: "CPR-3: Generic governed scope substrate"
labels:
  - epic:CPR
  - phase:5
size: M
---

# CPR-3: Generic governed scope substrate

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** M

## Description

Prompt 3 of the 33-prompt context-platform programme. It builds the scope
model every later prompt attaches to — sessions, candidates, knowledge
versions, skills, tools, policy profiles and role bindings all hang off a
scope — and it builds it **without a public API**, deliberately: the governed
entry points arrive with the PDP re-cut (Prompt 5) and the routes after it.

Four pieces:

- **`scopes`** — a named node with a parent and a subtree: `id`, `tenant_id`,
  `kind`, `parent_scope_id`, `slug`, `display_name`, `status`, `attributes`,
  `created_by`, `created_at`, `updated_at`.
- **`scope_closure`** — every `(ancestor, descendant, distance)` pair
  including the distance-0 self-row, maintained transactionally by explicit
  store SQL (no triggers — ADR-0011 decision 2, kept).
- **The structural rules**, most of them as database facts: root shape, one
  root per tenant, the placement rule, sibling-slug uniqueness, no
  cross-tenant edge, no cycle.
- **Internal application services**: create, rename, move, ancestors,
  descendants, tenant root, path resolution.

Decisions in ADR-0070.

## Why this exists

The tenancy hierarchy this replaces encodes an organisation. `hierarchy_nodes`
carries `kind in ('org','division','department','team','user')`, a
row-local CHECK that the root **is** the org, and a store-side rule that a
child's kind must strictly outrank its parent's. A single person cannot hold a
record in this product without first declaring themselves a company containing
a team — not as a default they can ignore, but as a CHECK constraint, a
`ScopeKind::rank()` and a Cedar entity attribute.

What was ever load-bearing about that model is the part that stays: a scope is
what assets attach to, what the PDP decides about, and what a role binding
covers. The rank is not load-bearing. It is a taxonomy that four people
sharing agent context have no use for and cannot leave empty.

## Design

### Shapes, not ranks

`kind` survives with five values — `tenant`, `org_unit`, `workspace`,
`project`, `principal` — and one job: deciding which kinds may be a scope's
parent.

| kind | permitted parents |
|---|---|
| `tenant` | *(none — it is the root)* |
| `org_unit` | `tenant`, `org_unit` |
| `workspace` | `tenant`, `org_unit` |
| `project` | `workspace` |
| `principal` | `tenant` |

Nothing compares two kinds for order; there is no `rank()`. `org_unit` nests
inside itself, so a deployment with a division, a department and a team
expresses all three without the product having names for them — and a
deployment with none creates none. One person's whole tree is a `tenant`
scope and a `principal`.

ADR-0068 decision 4 says a scope has "no `kind`". Read literally that produces
an untyped node and the placement rule has nowhere to live: "a project inside
a principal" becomes representable, and the shape information moves into
`attributes`, where every consumer parses a convention and no constraint
checks one. ADR-0070 decision 1 records why the closed shape vocabulary is
what decision 4 was actually refusing — the ladder, not the label.

### Where each rule is enforced

The judgement this feature turns on. Every rule that can be a database fact is
one, because a rule that lives only in a function holds only for callers who
went through that function:

| rule | enforced by |
|---|---|
| tenant has no parent, and only tenant | `scopes_root_shape_check` |
| one tenant-root per tenant | `scopes_one_root_per_tenant` (partial unique index) |
| the placement rule | `scopes_placement_check` + the composite parent FK |
| sibling slugs unique under a parent | `scopes_sibling_slug_unique` |
| never moves across tenants | composite parent FK + immutability trigger + forced RLS |
| cycles are impossible | `scope_closure_self_row_check` + the closure PK |
| the refusal *says* what would have been legal | the store service |

The placement rule rides a denormalised `parent_kind` column with a composite
foreign key `(tenant_id, parent_scope_id, parent_kind) → (tenant_id, id,
kind)`. The copy cannot drift — a row whose `parent_kind` disagrees with its
parent's `kind` has no referent — and carrying `tenant_id` in the same key is
what makes a cross-tenant edge *unrepresentable* rather than merely refused.

Cycles: `check ((ancestor_id = descendant_id) = (distance = 0))`. A move's
relink cross-joins the destination's ancestry with the moved subtree, so a
destination inside that subtree produces `(X, X, distance > 0)` — the row the
CHECK refuses. The service checks descendancy first so the ordinary refusal
is an error with a sentence in it; the constraint is what holds when something
reaches the tables another way.

### No materialised path, no depth column

Both are derived from the closure. The old model stored them and rewrote every
descendant's copy on every move; a derived path cannot be stale, and
`resolve_path` is a single recursive walk down the adjacency in one statement,
so nothing changes under it between segments.

### Where governance attaches

The services live in `synveda-store` beside the hierarchy they replace, and
hold no authorisation. The PDP decision, the audit event and the VedaFlow
change attach at the API boundary later prompts add — a store function that
consulted the PDP would be a second decision point beside the one seed §2.2
puts on the request path.

## What this prompt deliberately does not do

- **It does not touch the old hierarchy.** `hierarchy_nodes`,
  `hierarchy_closure`, `ScopeKind {org…user}` and every consumer of them are
  exactly as they were. Prompt 6 deletes them whole.
- **It synchronises nothing.** No row of `hierarchy_nodes` becomes a row of
  `scopes`, in either direction, at any time. There is no translator, by
  decision (ADR-0068 decision 3).
- **It exposes no API.** No route, no CLI command, no console screen, no
  adapter. The only callers are tests.
- **It does not touch Cedar.** `Scope.kind` and `Principal.department` still
  carry the old vocabulary into the PDP; removing them is Prompt 5's, together
  with the policy profiles that replace the packs.
- **It emits no audit event**, because no new action is reachable. The three
  emission points are named in the module so Prompt 6 wires them rather than
  finds them.

## Acceptance criteria

- `scopes` and `scope_closure` exist, are tenant-bound, carry `ENABLE` +
  `FORCE ROW LEVEL SECURITY` with a tenant-isolation policy and
  least-privilege grants, and appear in the adversarial suite's completeness
  inventory.
- A tree is created, read and moved through the services, and the closure
  agrees with a recomputation from the adjacency after **every** operation.
- The placement rule holds for every pair of the five kinds — asserted as a
  matrix over the vocabulary, not as a handful of cases.
- Each refusal names the rule it enforced: a parentless non-tenant scope, a
  second tenant root, a nested tenant scope, a duplicate sibling slug, a
  malformed slug, display name or attributes bag, and a placement the tree
  does not admit.
- Org units nest to arbitrary depth (40 levels in the test), and a workspace
  and a project still hang off the deepest one.
- Every scope's path resolves back to that scope; a path naming nothing
  resolves to nothing; a malformed path is an error rather than a miss.
- Every read is tenant-filtered in SQL as well as by RLS, so another tenant's
  scope reads as absent rather than forbidden — on `get`, `children`,
  `ancestors`, `descendants`, `path`, `resolve_path`, `rename` and `move`'s
  destination.
- A scope cannot move across tenants, and the database refuses it to the owner
  role as well as the application role. `id`, `kind`, `slug`, `created_at` and
  `created_by` are immutable beside it.
- Cycles are impossible: refused as an error by the service (under itself,
  under its child, under its grandchild) and unrepresentable in the closure.
- The closure survives randomly generated operation histories — creates, moves
  and renames against a live tree, legal and illegal alike — checked against
  the recomputation after every step.
- Concurrent writers behave: two creates racing for one sibling slug admit
  exactly one and the loser gets a conflict; two moves of one scope serialise
  and the closure agrees with the last; and a create landing inside a moving
  subtree waits, then inherits the ancestry the move left behind.
- Demonstrated by `crates/synveda-store/tests/scopes.rs` (20) and the scope
  block of `crates/synveda-store/tests/rls.rs` (4). **No demo script**, and
  the absence is the point: a demo drives a surface, and this prompt
  deliberately adds none. `demos/` gets its scope demo when the routes do.

---
title: "CPR-4: Workspaces, projects & canonical repository identity"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-4: Workspaces, projects & canonical repository identity

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Prompt 4 of the 33-prompt context-platform programme, and the first one with a
**public surface**. CPR-3 built the scope substrate deliberately without an API;
two of its five shapes — `workspace` and `project` — were named by the
vocabulary and had nothing behind them. This puts something behind them.

Five pieces:

- **`workspaces`** — a collaboration space owning one `workspace`-shaped scope
  under the tenant root: `id`, `tenant_id`, `scope_id`, `slug`, `display_name`,
  `description`, `status`, `revision`, `created_by`, `created_at`, `updated_at`.
- **`projects`** — a unit of work owning one `project`-shaped scope under its
  workspace's, with `workspace_id` beside the same columns.
- **`project_repositories`** — what a project is *about*, addressed by
  **canonical identity**: `provider`, `canonical_uri`, `repository_owner`,
  `repository_name`, `default_branch`, `local_fingerprint`, `metadata`.
- **`idempotency_records`** — what makes retrying a creation safe.
- **Twelve routes**, an OpenAPI document derived from them, and the console's
  TypeScript generated from that document.

Decisions in ADR-0071.

## Why this exists

The programme's target is a product a single person can start using. What a
person starts with is not a scope tree — it is *a place to work*. "Workspace"
and "project" are the two nouns everybody already has, and until this feature
the product could represent neither: `scopes` could hold the shapes, but a
`workspace` scope was a row with a slug and no description, no lifecycle, no
uniqueness rule anybody had written down, and no way to reach it.

The alternative was to let those facts live in `scopes.attributes`, which is an
open bag nothing constrains. Four rules say why not: a workspace's slug must be
unique, its description bounded, its status a closed vocabulary, and its
revision monotonic. None of the four can live in a JSON bag — they would become
conventions every consumer re-parses and no constraint checks, which is the
failure ADR-0070 decision 1 already refused once.

## Design

### The subtype and its scope are one act

`workspaces::create` mints the scope and the row in the caller's transaction, in
that order. There is no compensating delete and there must not be one: a failure
between the two statements rolls the first back, so the outcomes are **both**
and **neither** and there is no third. `scope_id` is `NOT NULL` with a foreign
key, so a subtype without a scope is unrepresentable.

The tenant root is minted by the first thing that needs a parent
(`scopes::ensure_tenant_root`), from the `tenants` row — the boundary both
models share and neither owns. **Nothing reads `hierarchy_nodes`**, then or
ever (ADR-0068 decision 3).

### Where each rule lives

ADR-0070 decision 2's doctrine, one level up. Every rule that can be a database
fact is one:

| rule | enforced by |
|---|---|
| a workspace's scope is workspace-shaped | `workspaces_scope_fk` (composite, over a denormalised `scope_kind`) |
| a project's scope is project-shaped | `projects_scope_fk` |
| **the subtype's slug IS the scope's slug** | the same two keys, which carry `slug` |
| a project's scope sits under its workspace's | `projects_scope_parent_fk` over `workspace_scope_id` |
| one subtype per scope | `workspaces_scope_unique`, `projects_scope_unique` |
| a project never changes workspace | `projects_immutable_workspace` trigger |
| **a revision steps forward by exactly one** | `synveda_subtype_immutable_columns` trigger |
| a repository identity is never a path | `project_repositories_uri_check` + `synveda_types::repository` |

The third row is the one worth reading twice. Because the composite key carries
`slug`, a workspace and its scope are *one name*: a scope path and a product
path cannot diverge, and the day somebody adds a rename that changes one of
them it fails against the other.

### Canonical repository identity

A project is about code, and code is not where it is checked out. Two agents on
two laptops, a CI runner and a container see one repository at four paths, and
one person's checkout moves between `~/src` and `/tmp` in a week.

So the **canonical remote URI is the identity** whenever one exists.
`repository::identify` collapses the transport (`https`, `http`, `ssh`, `git`
all become `https`), the credential, the port, a `.git` suffix and a trailing
slash, and lower-cases the host:

```
git@github.com:Acme/payments.git
https://github.com/Acme/payments.git
ssh://git@github.com:22/Acme/payments
https://x-token:secret@github.com:443/Acme/payments/
                            ↓
      https://github.com/Acme/payments
```

A repository with **no** remote is identified by `git+fingerprint:<hex>`, built
from a stable content id the client computes (a git root-commit object id),
which survives every move a path does not. A path-shaped `remote_uri` is
**refused by name**, with a message saying what to send instead — and the same
two shapes are a CHECK constraint, so a row that reached the table another way
still cannot hold a path.

Dropping the credential is load-bearing rather than tidy: it is what makes
`canonical_uri` safe to store, return, log and put in an audit payload.

### Retryable creation, refused lost updates

This surface's first callers are adapters, a CLI and hooks — all of which retry.
So creation takes a **required** `Idempotency-Key` and update a **required**
`expected_revision`. Both are required rather than optional because an optional
guarantee is absent from exactly the clients that did not think about it.

Three details are the whole of whether the idempotency works: the stored
**digest** (so a key reused for a *different* request is a conflict rather than
the wrong resource returned as success), the **race** between the lookup and the
insert (which replays rather than conflicting — that caller is the timeout retry
this exists for, arriving early), and the fact that **a replay still takes the
PDP decision** (a replay that skipped it would be a cached authorisation).

### `GET /v1/me`, and why onboarding state is the server's

The one call a client makes first: principal, tenant, accessible workspaces and
projects, effective tenant-plane capabilities, and an onboarding state —
`blocked` | `needs_workspace` | `needs_project` | `ready`.

The state is computed here rather than inferred by each client from an empty
list, because "no workspaces exist" and "you may read no workspaces" are
different facts with the same shape.

### The first OpenAPI contract

`utoipa` derives `docs/api/openapi.json` from the gateway's own request and
response types; a test fails when the committed file and the tree disagree, when
a documented path is not mounted, or when a mounted path on this plane is not
documented; `scripts/generate-api-types.mjs` writes
`console/src/generated/api.ts` from that file and `make check-api-types` fails
when those disagree. Three artefacts, two checks, and the only one a human edits
is the Rust.

The document covers **this plane and says so in its own description**. The
fifty-four `/v1` paths that predate it are Prompt 19's.

## What this prompt deliberately does not do

- **It does not build the console shell.** The generated types exist and
  typecheck; no screen consumes them yet. That is Prompt 20.
- **It does not re-anchor the PDP.** Every decision here names
  `Resource::Tenant`, because the Cedar entity model still materialises `Scope`
  from `hierarchy_nodes` and a generic scope has no row in it. The six new
  actions already apply to `[Tenant, Scope]` in the schema, so Prompt 5 moves
  them with a route change rather than a contract change. **This is the largest
  thing the feature defers, and it is stated rather than implied.**
- **It touches nothing of the old hierarchy**, and synchronises nothing with it.
- **It brings no other route onto the contract.** Twelve operations out of a
  `/v1` surface with sixty-six.

## Acceptance criteria

- Creating a workspace creates its scope in one transaction under the tenant
  root, and a project's under its workspace's, with the tenant root minted on
  the way past from the `tenants` row — and no `hierarchy_nodes` row is read or
  written.
- **A failed creation leaves neither an orphan subtype nor an orphan scope**,
  asserted for both subtypes through the failure mode that fires *after* the
  scope insert.
- The structural rules hold against direct SQL, not only through the services: a
  workspace cannot own a project-shaped scope, a subtype's slug and its scope's
  slug cannot disagree, a project cannot move between workspaces, a project's
  scope cannot be moved out from under it, and a revision cannot be rewound or
  skipped.
- A stale `expected_revision` is a 409 that writes nothing and names the current
  revision; another tenant's subtype is a 404 rather than a revision oracle; an
  empty update is refused.
- An archived workspace takes no new projects, and a status change is mirrored
  onto the owned scope — both ways.
- A description can be set, cleared and left alone as three distinct requests; a
  blank one is refused rather than stored.
- One repository written four ways is one attachment and the second is a
  conflict (case-insensitively); a filesystem path is refused with a message
  naming what to send instead, and the CHECK refuses one that bypassed the
  service; a repository with no remote is identified by its fingerprint; a
  handle from one project cannot address another's; two projects may be about
  the same repository.
- A creation replayed with the same key returns the original resource with 200
  and creates nothing; the same key with a different body is a 409; a concurrent
  duplicate replays rather than conflicting; and the replay still takes the PDP
  decision.
- Every route denies without its action, and every mutation chains its event —
  with an update's event carrying the `expected_revision` it was applied under.
  `GET /v1/me` chains one summarised decision.
- All four tables join the adversarial RLS suite's completeness inventory, with
  a wrong-GUC read seeing nothing, a cross-tenant write rejected, the lifecycle
  working as `synveda_app`, and no DELETE on `workspaces` or `projects`.
- The OpenAPI document is derived from the handlers; every documented path is
  mounted and every mounted path on this plane is documented;
  `console/src/generated/api.ts` is generated from the document; both checks run
  in `make ci`.
- Demonstrated by `crates/synveda-store/tests/workspaces.rs`,
  `crates/synveda-gateway/tests/workspaces_api.rs`,
  `crates/synveda-gateway/tests/openapi.rs`, the CPR-4 block of
  `crates/synveda-store/tests/rls.rs`, and `demos/cpr-4-workspaces.sh`.

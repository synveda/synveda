# ADR-0071: workspaces and projects as subtypes of a governed scope, canonical repository identity, and the first OpenAPI contract

- **Status**: Accepted
- **Date**: 2026-08-17
- **Feature(s)**: CPR-4
- **Deciders**: sujitn

## Context

CPR-3 built the scope substrate and gave it **no API on purpose** (ADR-0070):
`scopes` and `scope_closure` exist, five shapes decide placement, and the only
callers are tests. Two of those five shapes — `workspace` and `project` — were
named by the vocabulary and had nothing behind them. This is the prompt that
puts something behind them, and it is the first prompt of the context-platform
programme with a public surface.

The forces:

- **Seed §2.1 — zero-config by default.** A person creating their first
  workspace must not first be asked to create a tenant scope. Anything that
  makes them is the "declare an organisation before the product will hold a
  record" the whole programme exists to remove.
- **ADR-0068 decision 1 — one domain model.** One person and a bank differ in
  the profile assigned to their scopes, never in which table serves them. No
  `personal_workspaces`, no `team_workspaces`, no mode branch.
- **ADR-0070 decision 2 — every structural rule that can be a database fact is
  one.** The substrate set that standard for itself; a subtype layered on top
  that held its rules in a function would be a hole in the same wall.
- **Seed §2.2 — policy is enforced, never advisory.** Every read and mutation
  here passes the PDP. But the PDP's entity model still describes the *old*
  hierarchy: `Scope` entities are materialised from `hierarchy_nodes` and its
  closure, which a generic scope has no row in. Prompt 5 re-cuts that; this
  prompt has to work correctly before it does.
- **Agents retry.** This surface's first caller is not a human clicking a
  button — it is an adapter, a CLI and a hook, all of which retry on a timeout.
  A creation that is not idempotent makes two workspaces the first time a
  network hiccups, and an update with no precondition silently loses whichever
  writer was slower.
- **A project is about code, and code is not where it is checked out.** Two
  agents on two laptops, a CI runner and a container see one repository at four
  paths, and one person's checkout moves between `~/src` and `/tmp` in a week.
- **There is no OpenAPI document** (ADR-0068's context), and the target
  invariant says the contract is authoritative and frontend types are generated
  from it. Prompt 19 owns the whole surface; this prompt adds routes, and
  adding routes to a product with no contract is how the condition persists.

## Decision

**1. Workspaces and projects are product-level subtypes of a governed scope,
and the scope is created in the same transaction.** A workspace owns exactly
one `workspace`-shaped scope under the tenant root; a project owns one
`project`-shaped scope under its workspace's. `scope_id` is `NOT NULL` with a
foreign key, so a subtype without a scope is unrepresentable; both rows are
written in one transaction, so the outcomes are **both** and **neither** and
there is no compensating delete anywhere. The tenant root itself is minted by
the first thing that needs a parent (`scopes::ensure_tenant_root`), from the
`tenants` row — never from `hierarchy_nodes`, which nothing here reads.

The rules that hold the two models together are foreign keys rather than
service code, following ADR-0070 decision 2:

| rule | enforced by |
|---|---|
| a workspace's scope is workspace-shaped | `workspaces_scope_fk` over a denormalised `scope_kind` |
| a project's scope is project-shaped | `projects_scope_fk` |
| the subtype's slug **is** the scope's slug | the same two keys, which carry `slug` |
| a project's scope sits under its workspace's | `projects_scope_parent_fk` over a denormalised `workspace_scope_id` |
| one subtype per scope | `workspaces_scope_unique`, `projects_scope_unique` |
| a project never changes workspace | `projects_immutable_workspace` trigger |
| a revision only ever steps forward by one | `synveda_subtype_immutable_columns` trigger |

The third of those is worth stating on its own: because the composite key
carries `slug`, a workspace and its scope are **one name**. A scope path and a
product path cannot diverge, and the day somebody adds a rename that changes one
of them, it fails against the other.

**2. `GET /v1/me` is the client's first call, and onboarding state is the
server's answer.** It returns the principal, the tenant, the accessible
workspaces and projects, the caller's effective tenant-plane capabilities, and
an onboarding state from a closed vocabulary: `blocked` | `needs_workspace` |
`needs_project` | `ready`. The state is computed here rather than inferred by
each client from an empty list, because "no workspaces exist" and "you may read
no workspaces" are different facts with the same shape, and a client that
guessed would get the second one wrong.

`/v1/me` chains one summarised `authz.decision` event, unlike
`whoami?capabilities=true`, which chains none. The difference is what is
disclosed: `whoami` answers about the caller, and this answers what exists. A
route that served the same inventory `GET /v1/workspaces` chains an event for,
without one, would be a documented way around the audit trail.

**3. Six new PDP actions, decided at the tenant until Prompt 5 moves them.**
`WorkspaceRead`, `WorkspaceCreate`, `WorkspaceUpdate`, `ProjectRead`,
`ProjectCreate`, `ProjectUpdate`, applying to `[Tenant, Scope]` in the Cedar
schema. Repository attachment takes `ProjectUpdate` and repository listing
`ProjectRead`, because what a project is *about* is part of what the project
is. There is deliberately **no delete action**: retiring a workspace is a status
transition under `WorkspaceUpdate`, because a workspace is what sessions,
versions and audit events name.

The routes decide at `Resource::Tenant`, and that is a **stated limitation
rather than a design**. The Cedar entity model materialises `Scope` from
`hierarchy_nodes`, so `Resource::Scope(workspace.scope_id)` would materialise an
entity with no chain and deny at every caller — failing closed, but for the
wrong reason. Deciding at the tenant is coarser than the target and is a real
decision on the real path: tenant-wide role bindings are what the shipped packs
resolve there. The schema already admits `Scope`, so Prompt 5 re-anchors these
six actions with a route change and not a contract change.

The three shipped packs price the four mutating actions with the rest of
structural administration (steward, org-admin) and the two reads with the
admin readers **plus the content roles** (viewer, contributor, curator). That
last clause is the one place this feature widens a pack, and CNSL-2's finding is
the argument: under every shipped pack `HierarchyRead` is admin-only, so a
curator — the role the review inbox exists for — could see no tree at all. A
workspace's *name* discloses nothing about what is in it; everything in it stays
behind the tiered reads.

**4. A repository's identity is its canonical remote URI, and a filesystem path
is never one.** `synveda_types::repository::identify` collapses the transports
(`https`, `http`, `ssh`, `git` all canonicalise to `https`), drops the
credential, drops the port, drops a `.git` suffix and a trailing slash, and
lower-cases the host — so `git@github.com:Acme/payments.git` and
`https://x-token:secret@github.com:443/Acme/payments/` are one identity. A
repository with **no** remote is identified by a `git+fingerprint:<hex>` URI
built from a stable content id the client computes (a git root-commit object
id), which survives every move a path does not. A path-shaped `remote_uri` is
**refused by name**, with a message saying what to send instead; the same two
shapes are a CHECK constraint, so a row that reached the table another way still
cannot hold a path.

Dropping the credential is not tidiness: it is what makes `canonical_uri` safe
to store, return, log and put in an audit payload, and a caller who pasted
`https://x-access-token:ghp_…@github.com/acme/repo` has handed the gateway a
live token.

**5. Updates carry a required revision precondition.** Every subtype has a
monotonic `revision`, and `PATCH` takes `expected_revision`. A mismatch is
`409 Conflict` and writes nothing. The precondition is **required**, not
optional: an optional one is absent from exactly the clients that needed it. The
monotonicity is a trigger rather than store code, because a precondition is
worth nothing if the number it names can be rewound or skipped by anything
holding a connection.

**6. Creations carry a required `Idempotency-Key`.** A key, the subject that
minted it, the operation, a BLAKE3-256 digest of the canonical request and the
resource produced, in `idempotency_records` — written **in the same transaction
as the creation**. Same key + same digest replays the original resource with
`200`; a fresh key creates and answers `201`; same key + different digest is
`409`. The header is required for the same reason the precondition is.

Three details that are the whole of whether this works:

- **The digest.** Storing only (key → resource) would answer a *different*
  request with the first one's resource, and report it as success.
- **The race.** Two concurrent requests with one key both miss the lookup; the
  second blocks on the primary key and then fails. That caller is the timeout
  retry this exists for, arriving early, so the route re-reads and replays
  rather than returning the conflict — otherwise the guarantee holds everywhere
  except under the conditions that produce a retry.
- **The replay still takes the PDP decision.** A replay is still a request to
  create, and a caller whose permission was revoked between the attempt and the
  retry must be refused. A replay that skipped the decision would be a cached
  authorisation.

**7. The OpenAPI document is derived from the handlers, and the frontend types
from the document.** `utoipa` derives `docs/api/openapi.json` from the request
and response types in the gateway;
`crates/synveda-gateway/tests/openapi.rs` fails when the committed file and the
tree disagree, when a documented path is not mounted, or when a path on this
plane is not documented; `scripts/generate-api-types.mjs` writes
`console/src/generated/api.ts` from that file and `make check-api-types` fails
when those disagree. Three artefacts, two checks, and the only one a human edits
is the Rust.

The document covers **this plane and says so in its own description**. The
fifty-four `/v1` paths that predate it are Prompt 19's, and its silence about
`/v1/observe` is a statement about the document rather than about the gateway.

## Options considered

**1. Workspaces as `scopes.attributes`, with no subtype table.** No new tables,
and it is what an open labelling bag invites. Refused because none of the four
rules a workspace actually has can live in one: a slug that must be unique, a
description that must be bounded, a status from a closed vocabulary, and a
revision that must be monotonic. They would all become conventions every
consumer re-parses and no constraint checks — which is the failure ADR-0070
decision 1 already refused once, arriving through a different door.

**2. A workspace *is* a scope: one table, a `kind` discriminator, nullable
subtype columns.** Fewer joins, and it is the classic single-table-inheritance
shape. Refused because every subtype column becomes nullable for four fifths of
the rows, so "a workspace has a description" stops being expressible as `NOT
NULL`, and because the scope table is read on every chain walk — widening it
with product fields prices governance by the product's growth.

**3. Deciding at `Resource::Scope(workspace.scope_id)` now, and materialising a
Cedar entity for generic scopes here.** The target model, taken early. Refused
because it is Prompt 5's whole content and doing half of it here would mean
either a second entity-materialisation path beside the existing one, or a
synchronisation between `hierarchy_nodes` and `scopes` that ADR-0068 decision 3
forbids outright. Tenant-anchored decisions are honest, coarse, and one route
change away from correct.

**4. Reusing `HierarchyCreate` / `HierarchyRead` / `HierarchyUpdate` for
workspaces.** Zero policy churn, and defensible — creating a workspace *is*
administering the scope tree. Refused on the audit chain rather than on the
packs: an event that said `hierarchy.create` when somebody made a workspace
would be a false statement in the one record this product asks people to trust,
and no amount of later renaming makes a written row true.

**5. Optional idempotency keys (Stripe's shape).** The industry default.
Refused because an optional guarantee is absent from precisely the clients that
did not think about it, and this surface's first callers are retrying agents. A
required header costs a client one UUID.

**6. `If-Match` / `ETag` for the update precondition.** The HTTP-standard
mechanism. Refused for a reason specific to this codebase: the error taxonomy
maps one-to-one onto status codes and has no `412` or `428` (ADR-0008), so an
`If-Match` failure would answer `409` anyway — an HTTP mechanism whose
distinctive status codes we would then not use. `expected_revision` in the body
says the same thing, maps onto `Invalid` and `Conflict` exactly, and appears in
the OpenAPI document as a field rather than as a header convention.

**7. A local filesystem path as a fallback identity when there is no remote.**
The obvious accommodation, and the one decision 4 exists to refuse. A path
differs per machine and changes when somebody moves a directory, so a project's
identity would depend on which client last reported it — two agents on one
repository would be two projects, and one agent that reorganised its home
directory would be a third.

**8. Hand-authoring the OpenAPI document.** No new dependency, and it is how
most projects start. Refused because a hand-authored document is a second
description of the surface, and this repository's own history is the argument:
ADR-0068 recorded two hand-written copies of one contract at the base commit,
and neither check nor test made them agree.

**9. `openapi-typescript` for the frontend types.** The standard tool. Refused
narrowly rather than on principle: the document is ours and uses a narrow known
subset of JSON Schema, the console's dependency list is policed by
`scripts/check-npm-licences.mjs` so a build-time dependency is a reviewed diff
either way, and a ~250-line generator that **exits non-zero on a shape it cannot
express** is easier to trust than one that emits `unknown` and moves on.

## Consequences

- **Positive.** A person's first act in this product is `POST /v1/workspaces`
  with a slug and a name. The tenant scope appears because something needed it,
  not because somebody was asked for it. One row shape serves one person and a
  bank.
- **Positive.** The rules a reviewer has to trust about the subtype/scope
  relationship are readable in one migration and most of them cannot be violated
  by anything holding a database connection.
- **Positive.** Retrying a creation is safe and losing an update is not
  possible, which is what makes this surface drivable by an adapter rather than
  only by a human.
- **Positive.** The product has a contract, and it is generated. The condition
  ADR-0068 recorded — two hand-written copies, no check — cannot recur on this
  plane.
- **Negative / accepted.** **Decisions are tenant-anchored.** A pack cannot yet
  say "steward of *this* workspace"; it can only say "steward of this tenant".
  Every decision is a real decision that fails closed, but the granularity is
  wrong until Prompt 5, and this is the largest single thing this feature
  defers.
- **Negative / accepted.** `scope_kind`, `workspace_scope_id` and the
  denormalised `slug` exist for constraints and are never read by application
  code. ADR-0070 accepted the same cost for `parent_kind`; this adds three more
  columns a reader will meet without the migration header and wonder about.
- **Negative / accepted.** The canonical URI **drops the port**, so a deployment
  running two git servers on one host and path, distinguished only by a port, is
  a case this collapses into one identity. It can keep the raw URI in
  `metadata`, and no product surface reads it.
- **Negative / accepted.** `idempotency_records` accumulates. Nothing prunes it;
  the index on `created_at` exists so that the retention plane's sweep is a
  range scan when it arrives.
- **Negative / accepted.** The OpenAPI document covers twelve operations out of
  a `/v1` surface with sixty-six. That is a bounded start, and the document says
  so; the risk is that somebody reads its silence as coverage, which the
  description is written to prevent.
- **Negative / accepted.** Archiving a workspace does not cascade to its
  projects. New projects are refused, and existing ones keep their own status —
  a cascade would be a bulk mutation with one audit event covering rows nobody
  named.
- **Reversal trigger.** If a client is ever found deriving onboarding state
  itself rather than reading `onboarding.state`, decision 2 has failed — the
  field is there so the rule lives in one place, and a second implementation of
  it is the drift it exists to prevent. Equally: if the canonicalisation in
  decision 4 ever needs a per-provider special case beyond a host-to-provider
  lookup, then "canonical" has become "canonical per host" and the identity
  belongs to the provider adapter rather than to `synveda-types`.

## Compliance notes

- **Tenancy.** `workspaces`, `projects`, `project_repositories` and
  `idempotency_records` carry `tenant_id`, `ENABLE` + `FORCE ROW LEVEL
  SECURITY`, a `*_tenant_isolation` policy and least-privilege grants, and all
  four join the adversarial suite's completeness inventory
  (`crates/synveda-store/tests/rls.rs`). Every read filters on `tenant_id` in
  SQL as well, because the services are also called on owner connections. An
  idempotency key is tenant-bound too: one tenant must not learn that another
  used a key it guessed.
- **Policy enforcement.** Every route takes a PDP decision before acting,
  including the idempotent-replay path. No store function consults the PDP. The
  six new actions are in the Cedar schema, in all three shipped packs, and in
  `Action::ALL`, `PROBED_AT_SCOPE` and `PROBED_AT_TENANT` — so the capability
  probe answers them and `every_action_is_classified_exactly_once` holds.
- **Audit.** Six new action types: `workspace.created`, `workspace.updated`,
  `project.created`, `project.updated`, `project.repository.attached`,
  `project.repository.detached`. Every mutation chains its semantic event in its
  own transaction; every allowed read chains the decision; `/v1/me` chains one
  summarised event. An update's event carries the `expected_revision` it was
  applied under, so the chain says why a refused writer's change is absent.
- **Secrets.** A canonical URI is credential-free by construction, which is what
  makes `repository_image` safe as an audit payload; `local_fingerprint` is
  reported to the chain as a boolean rather than a value. No response, log or
  audit payload on this plane carries a token, and `AttachRepositoryBody`
  refuses unknown fields, so a client that sent one under a name we do not read
  is told rather than silently obliged.

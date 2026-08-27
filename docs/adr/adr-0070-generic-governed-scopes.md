# ADR-0070: a generic governed scope substrate — shapes rather than ranks, and the structural rules in the database

- **Status**: Accepted
- **Date**: 2026-08-17
- **Feature(s)**: CPR-3
- **Deciders**: sujitn

## Context

ADR-0068 decision 4 committed to *generic governed scopes*: "a scope is a named
node with a parent and a subtree. It has no `kind`, no rank, and no five-value
vocabulary." Prompt 3 of the context-platform programme builds it. The decision
is locked; what this ADR settles is the six things the decision does not say,
each of which had a cheaper answer that would have quietly reintroduced what it
removes.

The forces:

- **Seed §2.1 — zero-config by default.** One person must be able to hold a
  record without declaring an organisation containing a team. Whatever the
  vocabulary is, "the deployment I want has one scope in it" has to be
  expressible.
- **Seed §2.2 — policy is enforced, never advisory.** A scope is what the PDP
  decides about. It must stay a resource the decision point can name, and the
  substrate must not become a second place where authorisation is decided.
- **Seed §8 / the crate rule.** `types ← crypto ← {policy, store, identity,
  audit, vedaflow} ← retrieval/ingest ← gateway`. A scope service that consults
  the PDP cannot live in `synveda-store`; a scope service that lives above the
  middle band cannot be reached by `synveda-store`'s own code.
- **Prompt 6 has not run.** The old `hierarchy_nodes` / `hierarchy_closure`
  model is still in the tree, still carries `ScopeKind {org, division,
  department, team, user}`, and is still what every existing feature reads.
  This prompt is explicitly told to leave it untouched and to synchronise
  nothing with it.
- **ADR-0011's closure design was right and is not what is being replaced.** A
  closure table maintained by explicit store SQL inside the caller's
  transaction, no triggers, ancestor and descendant queries as one index scan:
  that survives. What is being replaced is the vocabulary sitting on top of it.

## Decision

**1. `kind` stays, as a shape rather than a rank.** Five values — `tenant`,
`org_unit`, `workspace`, `project`, `principal` — and the *only* thing a kind
decides is which kinds may be its parent. There is no `rank()`, nothing
compares two kinds for order, and `org_unit` nests inside itself to arbitrary
depth. ADR-0068 decision 4 says a scope "has no `kind`"; taken literally that
produces an untyped node, and the placement rule then has nowhere to live —
"a project inside a project" and "a person's scope containing a department"
become representable and every consumer re-derives what a node is from its
attributes. What decision 4 refuses is the *rank*: the strictly-increasing
ladder, the root-must-be-an-org rule, and the requirement that an individual
model themselves as an organisation. All three are gone. A closed shape
vocabulary with a parent rule is the smallest thing that keeps the tree
decidable, and it is what makes "one person has a tenant scope and a
principal" a complete deployment.

**2. Every structural rule that can be a database fact is one.** The root
shape, one root per tenant, sibling-slug uniqueness, the placement rule, the
impossibility of a cross-tenant edge and the impossibility of a cycle are all
enforced by the schema. The placement rule rides a denormalised `parent_kind`
column with a composite foreign key `(tenant_id, parent_scope_id, parent_kind)
→ (tenant_id, id, kind)`, so the copy cannot drift and the rule is row-local.
Cycles are refused by `check ((ancestor_id = descendant_id) = (distance = 0))`
on the closure: the row a cycle would need cannot be written, and a move that
would create one aborts on the constraint even if it reached the tables
another way.

**3. A scope never moves across tenants, for every role.** The composite
parent key makes a cross-tenant *edge* unrepresentable. A `before update`
trigger makes `tenant_id`, `id`, `kind`, `slug`, `created_at` and `created_by`
immutable, which covers the role forced RLS does not: the owner, which is what
migrations, break-glass `psql` and a restore run as.

**4. The scope services live in `synveda-store`, and hold no authorisation.**
`create`, `rename`, `move_scope` and the reads are internal application
services in the storage crate, exactly where `synveda_store::hierarchy` sits
today. The PDP decision, the audit event and the VedaFlow change attach at the
API boundary the later prompts add. Nothing in this module decides
authorisation, and a store function that consulted the PDP would be a second
decision point beside the one seed §2.2 puts on the request path.

**5. No materialised `path` and no `depth` column.** Both are derived from the
closure on demand. The old model stored them and rewrote every descendant's
copy on every move; a derived path cannot be stale, and `resolve_path` is one
recursive walk down the adjacency in a single statement.

**6. `synveda_types::scope` is a module path, not a root re-export, until
Prompt 6.** The new `ScopeKind` and the old one are different types with the
same name. Callers write `synveda_types::scope::ScopeKind`, which says at the
import line which model the code is written against.

## Options considered

**1. A truly untyped node (decision 4, read literally).** Cheapest to build
and the one this ADR spends the most on refusing. Without a kind there is no
placement rule, so "a project inside a project inside a principal" is legal and
the tree stops meaning anything a UI or a policy profile can rely on. The
information does not disappear — it moves into `attributes`, where every
consumer parses a convention and no constraint checks one. That is the rank
vocabulary again, unenforced.

**2. Keep the placement rule in the store only, as the old model kept the rank
rule.** Precedent, and it is where the *messages* still live. Rejected as the
only enforcement: a rule that exists in one function holds for callers who went
through that function, and this substrate is about to acquire an import path,
an adapter and a CLI. The `parent_kind` denormalisation costs one column and
one index and turns the rule into something a `psql` session cannot violate.

**3. Enforce the placement rule with a trigger reading the parent row.**
Stronger than the store, weaker than the FK: a trigger is code, it runs per
row, and ADR-0011 decision 2 already refused triggers for closure maintenance
on the grounds that the maintenance should be readable in the caller's
transaction. The composite key does the same work declaratively.

**4. A separate `synveda-scopes` crate above the middle band, so the service
can hold the PDP call.** Architecturally tidier for Prompt 5 and premature
here: it adds a crate tier entry, a dependency-rule edit and a second home for
storage code, in exchange for a governance call this prompt is explicitly told
not to expose. The governed entry points land at the gateway with the routes,
which is where every other governed path in this product already is.

**5. Migrate `hierarchy_nodes` into `scopes`.** Refused by ADR-0068 decision 3
and option 3 of that ADR, restated here because this is the prompt where it
would have been convenient: a `team` and a `department` map to the same
`org_unit`, and the approval matrix priced them differently.

**6. Do nothing / keep the five ranks.** The condition the whole programme
exists to remove.

## Consequences

- **Positive.** A one-person deployment is a `tenant` scope and a `principal`,
  created in that order, and nothing asks for a division. A four-person team is
  a `tenant`, a `workspace` and four `principal`s. A bank is all five kinds
  nested as deep as it likes. One table, one set of services, no edition
  conditional.
- **Positive.** The rules a reviewer has to trust are readable in one
  migration, and most of them cannot be violated by anything holding a database
  connection.
- **Negative / accepted.** `parent_kind` is denormalised. It is never read by
  application code, it cannot drift (the FK), and `kind` is immutable (the
  trigger) — but it is a column that exists for a constraint, and a reader who
  meets it without the migration header will wonder.
- **Negative / accepted.** Two scope models are in the tree at once, and the
  name `ScopeKind` means two things depending on the import. This is temporary
  by construction: Prompt 6 deletes the old hierarchy and the root name becomes
  free.
- **Negative / accepted.** The substrate has no governed surface, so nothing
  audits a scope creation yet. Audit re-anchoring is Prompt 6 and the routes
  are Prompts 5 onward; until then the only writers are tests.
- **Negative / accepted.** `status` has a two-value vocabulary of which only
  `active` is ever written, because no service transitions it yet. It is on the
  domain model because the model states it; the transition arrives with the
  surface that needs it.
- **Reversal trigger.** If a policy profile ever needs to decide over a scope's
  *kind* — a rule whose subject is "every workspace" rather than "this scope's
  subtree" — then kind has become a rank in everything but name and decisions 1
  and 4 of ADR-0068 need revisiting together. Equally: if a deployment ever
  needs a shape this vocabulary cannot express, the answer is a new value with
  a parent rule, not an `attributes` convention; the day that convention
  appears in a query, this ADR has failed.

## Compliance notes

- **Tenancy.** `scopes` and `scope_closure` carry `tenant_id`, forced RLS and a
  `*_tenant_isolation` policy, and both join the adversarial suite's
  completeness inventory (`crates/synveda-store/tests/rls.rs`). Every read in
  the module also filters on `tenant_id` in SQL, because the services are
  called on owner connections where RLS does not bite. A scope of another
  tenant reads as absent rather than forbidden (ADR-0008).
- **Policy enforcement.** Unchanged, and deliberately untouched: no PDP call
  moves into the store, and the Cedar entity model still describes the old
  hierarchy. The PDP re-cut over generic scopes is Prompt 5 (ADR-0068
  decision 4's other half).
- **Audit.** No new action type is emitted, because no new action is reachable:
  the services have no route, no CLI command and no adapter. The audit
  emission points are `create`, `rename` and `move_scope`, and they are wired
  when the governed entry points land.
- **Secrets.** None touched. `attributes` is caller-supplied labelling and is
  never an authorisation input; it is bounded at 16 KiB so a governed row read
  on every chain walk stays a governed row.

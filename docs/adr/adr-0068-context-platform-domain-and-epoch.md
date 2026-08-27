# ADR-0068: one domain model, policy profiles, and a fresh schema epoch for the context platform

- **Status**: Accepted
- **Date**: 2026-08-17
- **Feature(s)**: CPR-1
- **Deciders**: sujitn

## Context

Ninety-six features and sixty-five deliveries built an enterprise memory and
context platform whose smallest unit of existence is an *organisation*. A
tenant's hierarchy root must be `kind = 'org'` — migration 0004 says so in a
row-local CHECK — and every node below it must be a `division`, a
`department`, a `team` or a `user`, with a strictly increasing rank enforced
in the store because the rule needs the parent row. Three named policy packs
sit under that, and two deployment profiles above it: `standard` and the SMB
compose file for a small shop, `regulated-strict` and the Helm chart for a
bank.

That model is correct for the customer it was designed for and wrong for the
one this programme is aimed at. A single person, or four people sharing
agent context, has no division and no department and never will; today they
must be told that they are an `org` with a `team` inside it before the
product will hold a single record. The rank vocabulary is not a default
they can ignore — it is a CHECK constraint, a `ScopeKind::rank()` and a
Cedar entity attribute, so a scope that is not one of five things is
unrepresentable rather than merely unconventional.

The forces:

- **Seed §2.1 — zero-config by default.** "No YAML before value" was written
  about an identity provider's group claims. For an individual it now reads
  as "no org chart before value", and the current model cannot honour it.
- **Seed §2.4 — separation of concerns is architectural.** If personal and
  enterprise became two runtimes, or one runtime with edition conditionals,
  every subsequent feature would be written twice and tested once.
- **Seed §2.2 — policy is enforced, never advisory.** Whatever replaces the
  rank vocabulary must still be a resource the PDP can decide about, and the
  decision must stay on the same path.
- **Pre-1.0.** There is no customer whose data must survive this change.
  There is exactly one thing that must survive it: the discipline that
  produced sixty-five features with an audit chain that verifies.
- **The `derived` channel has always been a quarantine.** Tech plan §6 says
  so outright — "the derived channel is quarantined by design; published is
  the trust boundary" — but the schema does not: `records` holds both, and
  which side of the boundary a row is on is a `vedaflow_refs` membership
  question answered by walking a commit tree. The product's most important
  distinction is its least structural one.
- **Nothing in the product is a session.** `session_id` is a `text` column on
  `observe_events`, a `String` on the inject and recall bodies, and an
  audit-correlation string. The thing an agent actually *does* — start,
  observe, recall, inject, stop — has no aggregate, so nothing can be asked
  about it, governed at it, or retained by it.
- **There is no OpenAPI contract.** ADPT-3 owns one and has not run. Every
  DTO is hand-written per handler and every console type is hand-written a
  second time in `console/src/api.mts`. Two hand-written copies of one
  contract is the condition this programme starts from.

## Decision

We take eight decisions, together, and lock them for the whole programme.
Later prompts implement them; no later prompt reopens them without a
superseding ADR.

**1. One domain model for personal, team and enterprise.** There is one set
of entities, one storage schema, one PDP model and one runtime. A single
person and a regulated bank differ in what their configuration says, never
in which code path serves them. There are no edition conditionals, no
`#[cfg]` features per tier, and no second implementation of any noun.

**2. Policy profiles rather than edition branches.** `personal`, `team` and
`enterprise` are policy/configuration profiles in the sense AUTHZ-2 already
established for packs: a named, versioned bundle assigned to a scope, in
force for the subtree, swappable without a deploy. They are the successor of
the `standard` / `regulated-strict` / `open-collaboration` triple, and they
carry the same guarantee — a profile can narrow or widen what a pack decides
and can never reach below the invariant floor.

**3. Fresh schema epoch, with no old-data migration.** The 38-migration
sequence is deleted and replaced by a new `0001`. Nothing translates old
rows into the new model, nothing dual-reads, and no compatibility view
exists. A database carrying the old epoch is **rejected at startup with a
reset instruction**, rather than upgraded, half-read, or silently accepted.

**4. Generic governed scopes replace fixed organisational ranks.** A scope
is a named node with a parent and a subtree. It has no `kind`, no rank, and
no five-value vocabulary. `ScopeKind`, `hierarchy_nodes_kind_check`, the
strictly-increasing-rank rule and the root-is-org rule all go. What survives
is the part that was ever load-bearing: a scope is what assets attach to,
what the PDP decides about, and what a role binding covers.

**5. Sessions are the root of agent runtime activity.** A session is a
first-class, tenant-bound aggregate with a stable id. Observed events,
extracted candidates, recalls, injections and their audit events all hang
off it. `session_id: String` as a correlation hint is deleted.

**6. Candidates are separated from published knowledge.** Two things, not
one thing with a column. A **candidate** is what a session produced and
nobody has stood behind; **knowledge** is what was reviewed and published.
The trust boundary becomes a table boundary, so a query cannot accidentally
compose across it and a reviewer cannot accidentally be looking at the wrong
side of it.

**7. Knowledge, skills and tools are immutable versions.** Every one of the
three carries a stable aggregate id and an immutable revision. Publishing
mints a new version; nothing edits a published one. History is what a
version chain *is*, not a table that shadows one.

**8. OKF and MCP are external-format adapters.** Neither is a domain model.
They live at the boundary, translate to and from the public application API,
and hold no privileged access. A change to either must be implementable
without touching a core crate.

## Options considered

**1. Keep the rank vocabulary and make the levels optional.** Cheapest, and
it is what the current schema half-does already — `division` is documented as
optional. It fails on the decision that matters: an individual is still
required to be an `org` containing a `user`, and the five-value CHECK is
still the thing a personal profile has to work around. An optional taxonomy
that cannot be empty is not optional.

**2. Two products — a personal build and an enterprise build.** Honest about
the difference in requirements and fatal for everything else: two PDP
models, two audit paths, two sets of tests, and the guarantee this product
sells (that the small deployment and the regulated one enforce policy the
same way) becomes an assertion nobody can check. Explicitly refused by seed
§2.4.

**3. Generic scopes with a migration from the old model.** The obvious
compromise, and the one this ADR spends the most of its credibility
refusing. Mapping five ranks onto an unranked tree is lossy in the direction
that matters (a `team` and a `department` become the same thing, and the
approval matrix priced them differently); mapping `records` onto
candidates-and-knowledge requires re-deciding, per row, a boundary that
currently lives in a commit tree; and every one of those decisions would be
taken by a migration script that nobody reviews as carefully as they review
a policy. Pre-1.0, with no customer data, the migrator is pure risk bought
with nothing.

**4. Model sessions as a view over `observe_events`.** Would avoid a table.
Also would mean a session cannot be retained, sealed, governed or asked
about except by aggregating rows that were written for another purpose, and
that a session with no observed events does not exist — which is exactly the
session an agent that only *read* had.

**5. Do nothing.** The product remains excellent for the organisation it was
built for, and unusable for one person, which is the market this programme
exists to reach.

## Consequences

- **Positive.** One code path per noun. A personal deployment and an
  enterprise one are the same binary with different configuration, which
  makes "policy is enforced identically at both ends" a testable claim
  rather than a slogan. The trust boundary becomes structural. The thing an
  agent does becomes a thing the product can name.
- **Positive.** The epoch cut removes 38 migrations of accumulated shape
  that no longer describes the target model, and removes them *before*
  anything depends on the new one.
- **Negative / accepted.** Sixty-five delivered features are re-cut against
  the new model over Prompts 2–33. Everything that is deleted is deleted for
  real — production code, tests, fixtures, documentation and examples — so
  the programme is not additive and cannot be half-adopted.
- **Negative / accepted.** Every existing database is a reset. This is
  acceptable exactly once, pre-1.0, and the guard exists so that it is loud
  rather than silent.
- **Negative / accepted.** The published claims that name the old model —
  the LongMemEval row in `docs/BENCHMARKS.md`, the latency figures in
  STATUS.md — describe a product that will not exist after the programme.
  They are measurements of a commit, they say which commit, and they are
  re-measured rather than retracted.
- **Negative / accepted.** `OKF` names a format that does not exist in this
  repository at the base commit. This ADR fixes only its *position* — an
  external format, reached through an adapter — and not its schema, which a
  later prompt defines. Deciding position before content is deliberate: the
  position is the part that constrains the architecture.
- **Reversal trigger.** If a policy profile ever needs a decision the PDP
  cannot express over generic scopes — a rule whose subject is genuinely
  "the department" and not "this scope's subtree" — then rank carried
  information the tree does not, and decisions 1 and 4 need revisiting
  together, not separately. Equally: if two profiles ever require different
  *behaviour* from the same endpoint rather than a different answer from the
  same decision point, decision 2 has failed and the branch it forbids has
  already been written somewhere.

## Compliance notes

- **Tenancy.** Every persisted domain entity in the new model is
  tenant-bound, and forced RLS remains on every one of them. The epoch cut
  does not relax this; it re-asserts it on a smaller table set.
- **Policy enforcement.** The PDP stays on the read and mutation path
  unchanged. Generic scopes change what a `Scope` entity *is*, not where the
  decision is taken. Personal policy may auto-apply a VedaFlow change; it
  may not bypass one, and there is no code path from any adapter to storage
  that skips the decision point (seed §2.2).
- **Audit.** The hash-chained log survives the epoch as a mechanism and is
  re-anchored on the new nouns. Sessions, candidates and versions each
  produce events; a fresh epoch starts a fresh chain, which is honest — the
  old chain verified over rows that no longer exist.
- **Secrets.** Unchanged and restated: no secret appears in an ordinary API
  response, a log line, or an audit payload. The key plane (ADR-0064) is
  orthogonal to this cut and survives it.
- **This ADR is `Accepted` on delivery of CPR-1, which is unusual here and
  correct.** The normal pattern is `Proposed` while the feature is in flight;
  CPR-1's whole deliverable *is* this decision plus the baseline record, so
  there is no interval in which the argument is settled and the consequence
  unpaid. What remains outstanding is Prompts 2–33, each of which is its own
  feature with its own ADR where it needs one — not an unpaid consequence of
  this one.

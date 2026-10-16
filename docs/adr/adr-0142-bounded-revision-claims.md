# ADR-0142: Compare bounded claims on exact Knowledge revisions

- **Status**: Accepted
- **Date**: 2026-10-14
- **Feature(s)**: CPR-37
- **Deciders**: Owner-authorized implementation; GPT-6 Astra planning review

## Context

The shared lexical classifier drops one-character tokens. A reproduced fixture
classifies `Retry cap 3` and `Retry cap 5` as duplicate. Proximity cannot establish
value equivalence, applicability or approval. Conflict classification preserves
both revisions, but duplicate/support hints can still misdirect review. The
existing immutable revision metadata, source links and conflict/VedaFlow workflow
provide the necessary persistence; another graph or model service does not.

## Decision

Support an optional `metadata.synveda_claims` object with `schema_version: 1`
and one to eight closed typed claims. Each declares a case-sensitive subject and
predicate, up to eight applicability conditions, assertion status (`asserted` or
`proposed`), a typed value constraint and exact UTF-8 body-byte evidence span.
Decimal values use an integer coefficient and scale of zero to nine; text,
boolean and semantic-version release triples are also supported. Constraints
are equality, inequality, ordered bounds or an inclusive/exclusive range.
Valid time and normalized provenance belong to the exact immutable revision.
Metadata declares a proposition; it does not establish truth, verified execution,
approval, entity-read authority or permission to publish.

Compare only matching subject/predicate and overlapping applicability and valid
time. Supported disjoint values imply a contradiction hint, future-effective
replacement implies transition, and unknown or partially covered comparisons
require review. Known time and byte units can be normalized with checked integer
arithmetic. Unsupported units require review; there is no universal unit parser.
Only fully covered equivalent asserted claims can supply a support hint.
Different environments, predicates or past disjoint intervals suppress lexical
fallback. Case and version components remain exact.

Untyped prose remains usable. Exact canonical content hashes alone justify an
automatic duplicate hint. Non-exact lexical relatedness can suggest a reviewable
supersession, opposite polarity or future transition; it cannot assert duplicate
or supporting truth. One-character terms remain in relatedness. No generative
work enters reads and no classifier publishes or resolves truth independently.

The retained merge acceptance exposed why weak shared words alone cannot imply
conflict: webhook identity and webhook retry instructions share an event-ID
phrase while saying complementary things. Unannotated similarity below 550
therefore supplies no relation unless literal numeric/unit terms differ at a
minimum similarity of 400. Such a difference suggests review, never a parsed
truth. Stronger unannotated proximity is also review-only. This preserves the
existing governed merge contract without implicitly resolving transitional
inputs or weakening its preconditions.

## Options considered

1. **Typed metadata plus conservative lexical hints (chosen).** Reuses exact
   revision identity, canonical hash, provenance, validation and existing review.
2. **Parse all prose or ask a model to decide truth.** Cannot reliably establish
   units, applicability or policy and adds hidden dependencies.
3. **Add a separate mutable claim graph.** Duplicates authoritative content and
   introduces another lineage/erasure contract.

## Consequences

- Positive: numeric/unit/comparator/version differences remain explicit review
  evidence and applicability can prevent false conflicts.
- Negative / accepted: fuzzy duplicates and unsupported supporting claims need
  more review. The existing bounded candidate pool is not exhaustive claim search.
- Reversal trigger: held-out precision/coverage evidence supports a separately
  reviewed asynchronous parser without weakening these authority boundaries.

## Compliance notes

Claims live inside the existing hashed immutable revision and inherit its
VedaFlow, Cedar, forced RLS and erasure contracts. Source details retain their
own authorization. Claim values do not enter metrics/audit; conflict evidence
uses the current bounded classifications/reason codes and exact revision IDs.
No migration, dependency, model service or compatibility store is introduced.

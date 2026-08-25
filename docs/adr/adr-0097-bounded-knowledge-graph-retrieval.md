# ADR-0097: ContextRun expands only the governed Knowledge-relation graph under explicit bounds

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-38 (subsumes the remaining product objective of GRPH-3; replaces the Record-era runtime of GRPH-1/GRPH-2)
- **Deciders**: Autonomous continuation of the context-platform programme

## Context

CPR-20 made current immutable `KnowledgeRevision` rows the only learned-context
universe and deliberately left `graph_version` absent. CPR-15 already made
`KnowledgeRelation` the append-only relationship claim between stable Knowledge
items. The repository nevertheless still compiles a second, unused graph:
`graph_vertices`, `graph_edges` and the old ingest linker are keyed to the
retired Record aggregate. No application reader calls it. Teaching ContextRun
to translate Knowledge into those vertices would create the forbidden dual
model and a new synchronisation path.

Graph expansion also widens a candidate universe. A tenant-RLS relation row is
not permission to disclose either endpoint, and filtering only after traversal
would retain denied ids, path shapes and counts in the diagnostic trace. The
planner needs hard hop, fan-out, candidate, time and token bounds; its scores
must distinguish retrieval evidence from graph evidence; and a graph failure
must leave the lexical/vector answer usable and explicitly degraded.

## Decision

1. **`KnowledgeRelation` is the one product graph.** ContextRun traverses the
   six supporting types `supports`, `references`, `derived_from`,
   `supersedes`, `transitions_to` and `related_to`. `contradicts` may attach an
   authorised warning but contributes zero supporting score; `duplicates`
   remains deduplication evidence and does not expand. The Record-backed graph
   tables, store API, ids/types, ingest linker and tests are deleted without a
   bridge or translator. The migration refuses non-empty predecessor or
   pre-native context rows with the reset instruction before dropping them.

2. **Expansion is anchor-first and bounded by governed Configuration.** The
   existing lexical, semantic, explicit-pin and freshness-aware pool is
   authorised first. At most ten leading selectable Knowledge anchors enter an
   expansion attempt. The effective immutable Configuration supplies enabled,
   maximum hops (never above two), fan-out per frontier node, total expanded
   candidates, wall-clock milliseconds and expansion token budget. All shipped
   templates enable the same implementation with explicit conservative values;
   a document may disable it by selecting a zeroed bound set. These settings
   narrow work and grant no authority.

3. **The four policy boundaries are explicit.** Anchor candidates have already
   passed exact `KnowledgeRead`; a separate tenant-RLS read transaction
   re-authorises each frontier before reading its bounded adjacency; every
   endpoint is decided before it enters the frontier; and the main planner
   transaction re-hydrates and re-authorises every expanded revision and path
   before it is ranked or persisted. Inspector reads perform the fourth fresh
   endpoint/revision decision before rendering an exact path. Any denial drops
   the whole affected path and contributes only the run's aggregate policy
   exclusion flag—never an id, edge, reason or count.

4. **One best supporting path is trace evidence, not hidden rank state.** A
   forced-RLS append-only `context_graph_steps` table records each retained
   step against its candidate. Full/redacted modes retain exact relation and
   immutable endpoint addresses; hashes-only retains only node/edge hashes;
   disabled retains no steps. Every candidate separately records anchor
   retrieval score, aggregate edge weight, hop penalty, freshness adjustment,
   current-state adjustment and final score. Step rows record direction,
   relation type, hop, weight and whether the edge is supporting or a warning.
   Context selections cite their exact candidate, so the inspector cannot
   infer a path by fuzzy content matching.

5. **Failure returns the anchor answer with an exact degradation.** Expansion
   uses its own tenant-RLS transaction, allowing a timeout or storage failure to
   roll back independently while the main lexical/vector plan continues.
   Time, fan-out, total-candidate and graph-token exhaustion are explicit
   degradation values. Storage unavailability records `graph_unavailable` and
   exposes no backend text. Successful attempts record
   `knowledge-relations-v1`; the planner becomes `knowledge-planner-v2`.

6. **Ranking remains deterministic and graph is additive.** An anchor's score
   is unchanged. A graph-only candidate inherits the originating anchor score,
   adds fixed integer relation weights, subtracts a fixed hop penalty, then adds
   its own freshness/current-state adjustments. The best score/path wins;
   identifiers break ties. Graph-only retrieval does not exist, traversal can
   never make a stale/superseded/transitional item current, and final assembly
   still applies the one context token budget.

## Options considered

1. **Bounded `KnowledgeRelation` expansion with typed trace evidence (chosen).**
   It extends the current aggregate and planner with no projection or second
   authority.
2. **Map Knowledge into the old named Record graph.** Rejected: it requires a
   dual write/backfill and preserves a graph whose vertices cannot cite the
   governed scope or immutable Knowledge revision.
3. **Fetch a whole relation component and filter after traversal.** Rejected:
   it is unbounded and persists a policy side channel before Cedar decides.
4. **Graph-only recall or a hidden rank boost.** Rejected: neither can explain
   why a revision arrived, and graph failure would become request failure.

## Consequences

- Positive: one immutable relationship model now powers the browser, conflict
  lifecycle and runtime retrieval; every expanded selection has a bounded,
  inspectable evidence path and a reproducible integer score.
- Negative / accepted trade-offs: expansion makes bounded repeated PDP and
  adjacency calls rather than one recursive SQL sweep; only one best supporting
  path is retained per candidate; exact paths disappear on later revocation.
- Reversal trigger: graph-expansion p95 exceeds ADR-0029's 150ms slice or the
  bounded repeated decisions dominate planner latency on production-shaped
  data → measure a policy-safe materialised two-hop projection before changing
  engines or widening a bound.

## Compliance notes

`context_graph_steps` is tenant-bound, forced-RLS and immutable. Candidate
generation and traversal do not grant authority; Cedar decisions precede every
retained endpoint and repeat on disclosure. Trace/audit metadata contains only
ids, hashes, relation vocabulary, bounds, scores and decision summaries—never
Knowledge content or relation metadata. This package creates no mutation path,
so VedaFlow remains the sole writer of Knowledge and its relations.

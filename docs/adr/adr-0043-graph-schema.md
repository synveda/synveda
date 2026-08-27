# ADR-0043: The graph is indexed adjacency in Postgres — ADR-0004's named graphs survive as a discriminator the API cannot omit, an edge is a bitemporal row of the `records` shape, and the product stops calling AGE

- **Status**: Superseded by ADR-0097
- **Date**: 2026-07-27
- **Feature(s)**: GRPH-1 (GRPH-2, GRPH-3 inherit)
- **Deciders**: sujitn

## Context

GRPH-1's text is "named graphs per tenant: entity, episode, provenance
(MAGMA-informed). Edges carry bitemporal validity", and its acceptance
criterion is "Cypher round-trip tests; edge supersession preserves
history". Both sentences were written under ADR-0004, and ADR-0029 has
since taken the first one apart and handed this ADR the pieces.

**This ADR exists because GRPH-4 said it should.** The gate's verdict
retained AGE on the criteria it was commissioned to test and then said,
in its own words: "the relational adjacency baseline cleared *every*
criterion, including both that AGE failed … the burden of proof has
moved: GRPH-1 should adopt plain indexed adjacency unless a concrete
requirement for Cypher-expressed traversal appears. That decision belongs
in GRPH-1's design ADR — G4's pre-registered consequence explicitly makes
graph partitioning a schema decision — and this report is the evidence in
front of it." ADR-0029 restated it as a live obligation rather than a
remark: "GRPH-1's design ADR is where the schema call belongs".

The evidence, measured 2026-07-25 on Postgres 17.10 + AGE 1.7.0
(docs/spikes/grph-4-age-traversal.md), medians at 10M edges:

| | AGE (disciplined form) | relational adjacency |
|---|---|---|
| 1-hop, 10 seeds | 9.35ms | **1.24ms** |
| 2-hop, 10 seeds | 12.91ms | **4.84ms** |
| single edge write | 10.42ms Cypher / 0.01ms direct insert | **1.46ms** |
| edge storage @1M | 173 MB | **68 MB** |
| bulk load @10M | 39.8s | **6.0s** |
| catalog cost @1,000 tenants | 48,000 relations (amended to 48) | **0** |
| statements | graph name must be a literal | bound parameters, sqlx-checked |

Forces at play:

- **Three of the four ways a competent person writes an AGE traversal are
  20×–2000× slower than the one that works, and nothing in the query text
  says which one you wrote.** `IN` lists scan the whole edge table
  (211.77ms at 10M, slope 7.09×); `*1..2` variable-length paths cost
  3.7s at 10M where the explicit two-hop pattern costs 0.43ms;
  `where id(x) = …` does not use the primary key (689ms). ADR-0029 made
  the disciplined forms binding on GRPH-1 and required a test that fails
  on a sequential scan over a label table — a gate that passes only for
  queries written one particular way is a property of the code, not of
  the engine.
- **The distinguishing feature of a graph engine is the slowest thing
  measured.** Variable-length traversal is what separates Cypher from a
  join, and it is unusable at both scales. Whatever AGE is buying here,
  it is not the thing graph engines exist to sell.
- **Nothing in the product calls AGE.** The only caller in the workspace
  is `crates/synveda-store/tests/graph_spike.rs`, the gate's own harness.
  This is a decision about what to build, not about what to remove — the
  cheapest moment there will ever be to take it.
- **The product requirement is bounded, and both engines are bounded the
  same way.** GRPH-3 is specified as "1–2 hop expansion in recall
  ranking; degradable (retrieval works with graph off)". Nobody has asked
  for unbounded depth. ADR-0029's fallback ladder records that its one
  live trigger — depth beyond 2 hops or genuinely variable-length paths —
  "is equally the one AGE cannot serve", so choosing AGE today does not
  buy an option on that requirement.
- **The repo already shipped an edge, relationally, for these reasons.**
  MEM-5's `record_supersessions` (migration 0024) is the feature text's
  "supersession edges", and ADR-0039 option 7 rejected the AGE form on
  ADR-0029's verdict: "graph names must be literals, so the statements
  cannot be sqlx-checked, which CLAUDE.md forbids outright; and this edge
  is read *by the write path* inside the record's own transaction. A
  relational edge table is rung one of that ADR's own fallback ladder."
  Its trigger (d) — "GRPH-2 landing → these rows are mirrored as graph
  edges for traversal, the table staying the system of record" — is
  addressed to this decision.
- **The repo also already ships graph-shaped data in plain Postgres.**
  HIER-1's `hierarchy_closure` is a closure table with a materialised
  path, measured at 57µs/691µs medians over 10k nodes, and its composite
  foreign key `(tenant_id, ancestor_id) → hierarchy_nodes (tenant_id,
  id)` makes a cross-tenant closure row unrepresentable. That is the
  house pattern for exactly this shape of problem.
- **`sqlx` compile-time checking is not a preference.** CLAUDE.md admits
  no string-built SQL, ever, and ADR-0001's compliance note sells it:
  "reviewers can enumerate every SQL statement in the binary". AGE's
  `cypher()` takes its graph name as a name constant and its parameter
  map as a bare `$n` bound as `agtype`, a type sqlx has no encoder for.
  ADR-0029's amendment (one shared graph set, tenant as a property) makes
  the graph name a compile-time constant and so survives the letter of
  the rule — but every Cypher statement remains an opaque string literal
  that sqlx checks nothing inside.
- **ADR-0004's semantic partitioning is a separate claim from its
  engine.** MAGMA's finding is that specialised graphs beat one
  homogeneous graph; that is about what edges mean, not about which
  process stores them. ADR-0029 already amended the instantiation and
  kept the partitioning. Nothing in the research digest (features §A1)
  requires Cypher.
- **ADR-0004's option 2 recorded the footgun that a discriminator
  invites.** It rejected "one homogeneous graph, label-discriminated"
  because "traversals cross semantic domains unless every query filters
  on labels (a leak-by-omission footgun, like the single-table bitemporal
  option rejected in ADR-0006)". A relational table with a `graph` column
  is exactly that shape, and this ADR has to answer it rather than
  inherit it quietly.
- **CTX-5 already built the seam graph expansion plugs into.** ADR-0042
  decision 12 fuses ranked ids and hands them to admission as
  `ComposeRequest.only`, "so the ranked set is *narrowed* by admission
  and never widened by it", and its option 12 refused a private edge
  shape in recall precisely because "GRPH-1 owns the schema decision".
  ADR-0042's reversal triggers name "GRPH-1/2 land → GRPH-3 adds a third
  leg to decision 12's fusion, feature-flagged and degradable."
- **As-of is now a product surface, not a promise.** CTX-5 ships
  transaction-time and valid-time recall over `records_versions`
  (ADR-0042 decisions 7–11). A graph whose edges cannot answer the same
  two axes with the same shape would make "what did the agent know on
  March 3rd" true of the corpus and false of its relationships.
- **`records.class` already names `entity` and `episode`** (migration
  0001), so the extraction pipeline can already produce the material
  GRPH-2 will resolve. The graph does not need to invent a parallel
  universe of things; it needs identity for them and claims between them.

## Decision

**The graph layer is indexed adjacency in Postgres.** GRPH-1 ships
migration 0026 with a vertex table and a bitemporal edge pair on the
ADR-0006 pattern, a traversal API in `synveda-store` whose only entry
point cannot express an undisciplined query, and no call to AGE from any
crate. ADR-0004's named graphs survive as a mandatory discriminator; its
engine choice does not.

Decisions, specifically:

1. **Rung one of ADR-0029's own ladder is adopted, on ADR-0029's own
   recommendation.** No new dependency, no new licence question, no
   second engine, no catalog cost; bound parameters and compile-checked
   statements everywhere; 3–8× faster on the traversal the product
   actually issues, at 2.5× less storage. The gate reserved the
   relational revival for a G1/G2 failure and those passed — so this ADR
   does not overturn ADR-0004 on the gate's own trigger. It overturns it
   on the evidence the gate gathered anyway and was explicit about
   handing forward.

2. **The named graphs survive as a `graph` discriminator, and the API
   cannot omit it.** `entity`, `episode`, `provenance` — ADR-0004's three,
   with its research argument intact and its per-tenant instantiation
   already gone (ADR-0029). ADR-0004 option 2's leak-by-omission
   objection is answered where this repo has answered it before: the way
   CTX-1 answered it for scopes. `graph::expand` takes a `Graph` enum by
   value, there is no default and no `Option`, and there is no other
   entry point — the ADR-0024 decision 1 discipline ("the retrieval
   engine's only entry takes a mandatory `SearchFilter`; there is no
   unfiltered code path"), applied to semantic domain instead of to
   tenancy. A traversal that does not name its graph does not compile.

3. **An edge is a bitemporal row of exactly the `records` shape.**
   `graph_edges` + `graph_edges_history` + a `graph_edges_versions` view,
   with `valid_from`/`valid_to` as application data and `tx_from`/`tx_to`
   written by triggers only. Not "inspired by" the records pattern — the
   same pattern, including migration 0001's structural rule that an
   alteration touches the table, the history table, the view and the
   trigger functions' explicit column lists in one migration. The trigger
   functions are written per table because they enumerate columns, and
   that is the one piece of duplication this decision accepts, in
   exchange for a graph that answers `as_of` through the same view shape
   `records_versions` gave CTX-5. GRPH-1's AC clause "edge supersession
   preserves history" is then a property of the schema rather than of the
   code that writes it.

4. **Supersession of an edge is a closed window plus a new row — MEM-5's
   rule, restated for edges.** A changed relation closes the prior edge's
   `valid_to` and inserts its replacement; nothing is deleted, and the
   history reads as-of. ADR-0039 decision-level semantics carry over
   unchanged, which is what lets a reader ask the graph the question
   MEM-5 made answerable of the corpus.

5. **Vertices are identity; edges are claims.** `graph_vertices` holds
   one row per thing the graph can talk about — a resolved entity, an
   episode, or a reference to something that already exists elsewhere
   (a record, an identity, a scope) — with `(tenant_id, graph, kind,
   key)` unique so GRPH-2's entity resolution has a place to converge.
   Vertices are **not** bitemporal: a vertex asserts that a thing exists
   and is named, which is not a claim about the world that can be
   superseded. Every revisable statement is an edge, and every edge has
   history. `records.class` already carries `entity` and `episode`, so a
   vertex may be backed by a record rather than duplicating it — GRPH-2
   decides when it is, and this ADR only makes it representable.

6. **Endpoints are vertices, so both ends carry a foreign key.** The
   polymorphic alternative — `(src_kind, src_id)` pairs with no
   referential integrity — buys one fewer table and costs cascade
   deletes, orphan sweeps and a class of dangling edge this product would
   then have to monitor for. Edges reference `graph_vertices` on both
   sides, and vertices reference the rest of the schema, so there is
   exactly one place where the graph joins the world.

7. **Cross-tenant edges are structurally impossible again.** Every FK is
   composite — `(tenant_id, src_id) → graph_vertices (tenant_id, id)`,
   the `hierarchy_closure` pattern — so an edge between two tenants
   cannot be represented, not merely cannot be inserted. This **recovers
   the guarantee ADR-0029's amendment had to downgrade**: under the
   shared-AGE-graph shape, "cross-tenant edges structurally impossible"
   became "enforced by RLS". Here it is structural *and* enforced, since
   decision 8 keeps the backstop as well.

8. **Forced RLS keyed to the TEN-2 GUC on every new table, and they join
   the adversarial suite.** No new isolation mechanism, no new argument:
   `rls::begin_tenant_tx` (ADR-0009), `tenant_id` on every row, and
   `crates/synveda-store/tests/rls.rs` gains the vertex, edge and
   edge-history tables — forged-tenant write rejected, no DELETE grant
   where the app role has none elsewhere, cascade behaviour asserted.
   TEN-5 tenant deletion is a predicated delete, which is the cost
   ADR-0029 already accepted and recorded when it amended ADR-0004; it is
   not a new one.

9. **The disciplined query forms are the only ones that exist — ADR-0029's
   obligation, honoured in the substrate that replaced the one it was
   written against.** The obligation was AGE-specific in its examples and
   general in its principle. `expand` takes a bounded seed set, a `Depth`
   **enum** (`One` | `Two`) rather than an integer, the graph, the tenant
   and the two instants, and emits the measured adjacency join with bound
   arrays. Unbounded depth is unrepresentable in the type, so the
   ladder's trigger-1 requirement cannot arrive by accident — it arrives
   as a compile error and then as an ADR. And the test ADR-0029 asked for
   is kept in its exact spirit: the AC suite reads `explain (format json)`
   for the shipped statements and **fails on a sequential scan over
   `graph_edges`**, because a plan that regresses silently is how the
   discipline dies on contact with the second contributor.

10. **No property bag.** Edges carry typed columns — `kind` (the relation
    type, checked non-empty), `method` and `confidence_permille` (the
    MEM-5 discipline: integers per mille, never floats, because a number
    jsonb or a client may reshape is a number that cannot be compared
    later), the two windows, and the provenance of the assertion. There
    is no `properties jsonb` in GRPH-1: an edge property nobody queries
    is a column nobody reviewed, and GRPH-2 adding the columns it needs
    as a reviewed diff is the CTX-1 discipline for shipped index
    variants, not a hardship.

11. **`record_supersessions` stays the system of record, and ADR-0039
    trigger (d) is discharged as a projection rather than a mirror.**
    With both structures relational, "mirroring rows into the graph"
    would be a dual write of one claim into two tables — the thing this
    product refuses everywhere else. The supersession edge keeps its own
    table because the write path reads it inside the record's own
    transaction (ADR-0039 option 7), and traversal reaches it through a
    projection into the edge model. GRPH-2 owns the wiring; GRPH-1 owns
    the rule that there is one system of record per claim.

12. **The graph is never a scope producer, and expansion runs before
    admission.** GRPH-3 hands `expand`'s output into ADR-0042 decision
    12's fused id list, which is narrowed by `admit` and never widened by
    it. So an edge cannot disclose a record its owner may not read, no
    Cedar vocabulary is added, and the AUTHZ-5 leak suite's answer is
    unchanged by the graph's existence. This is stated as a decision
    rather than left as an implementation habit, because it is the one
    property that keeps a knowledge graph from becoming a policy bypass.

13. **The AC's first clause is amended on the record, and the feature
    text with it.** "Cypher round-trip tests" names a mechanism this ADR
    removes; its substance survives whole and is restated as: *an edge
    written through the store API reads back through the traversal API
    with its kind, endpoints and validity intact, and a supersession
    closes the prior edge's window with both versions readable as-of.*
    `docs/SYNVEDA_FEATURES.md` GRPH-1 is amended to "Multi-graph schema"
    with that AC, in the same commit as this ADR's acceptance and by the
    same discipline ADR-0029 used to amend ADR-0004 in place — an
    amendment block, not a silent edit. The tech plan's technology table,
    its `observe` pipeline sketch and its AGE risk row are amended the
    same way. **The seed is deliberately left alone**: it is founding
    text, and this repo's practice is that ADRs supersede it rather than
    rewrite it — §6 and §8 still name OPA/OpenFGA two phases after
    ADR-0002 chose Cedar, and the seed's own instruction 3 ("every
    subsequent architectural choice gets its own numbered ADR") is the
    mechanism that makes that correct. So §7's stack list still names
    Apache AGE, and this ADR is what supersedes it. **A feature is done
    when its acceptance criteria pass** (CLAUDE.md), so the criteria have
    to say what is actually being claimed.

14. **AGE stays installed and stops being called.** It remains in the dev
    compose because `graph_spike.rs` is the evidence behind this ADR and
    must keep running; it stays out of every crate's code path; and no
    core-path licence question changes (it is an Apache-2.0 Postgres
    extension either way). Removing it from the image is deferred to
    OPS-1/OPS-2's profile work with the condition named — pulling it now
    costs a dev-environment change and buys nothing. What is **refused**
    is the middle position: no dual write, no "keep the AGE tables warm
    in case", no Cypher-over-SQL shim. One system of record per claim
    (decision 11), applied to the engine question.

15. **The measurement is re-taken on the shipped schema, not inherited
    from the spike.** The spike measured a synthetic adjacency table; the
    AC suite measures `graph_edges` as built — with its RLS predicate,
    its bitemporal columns, its composite FKs and its tenant index — at
    the spike's own shape (10 seeds, out-degree 10) and reports 1-hop and
    2-hop medians with tails, plus the plan assertion of decision 9. The
    house discipline applies unchanged: the median is asserted, the tails
    are reported, dev IO owns the upper percentiles, and EVAL-6 owns SLO
    enforcement on production-shaped hardware. ADR-0029's 300ms recall
    decomposition reserves a slice for graph expansion, and CTX-5's
    measurement showed the whole recall request at 17.1ms of it — so the
    number this AC produces is the first evidence about whether that
    slice is real.

## Options considered

1. **AGE with ADR-0029's amendment — one shared graph set, tenant as a
   property** (the incumbent, and the ADR-0004 decision this one
   overturns). It passed the gate's latency criteria with headroom, keeps
   ADR-0004 and the seed's stack diagram literally true, keeps openCypher
   available for operators and for a future console, and G6 proved forced
   RLS is honoured by Cypher traversals. Rejected on the balance of the
   spike's own evidence: 3–8× slower on the exact traversal GRPH-3 will
   issue, 2.5× the storage, 6.6× the bulk-load time, a hand-built GIN
   index AGE does not create for you, a label-sequence desynchronisation
   obligation after every bulk load, edge writes that must bypass Cypher
   entirely to meet G3 (0.01ms via direct insert against 7.90ms via
   `CREATE`) — and, decisively for this repo, statements sqlx cannot
   check a single token inside. The disciplined-forms tax is the deeper
   objection: ADR-0029 required a plan test to defend query forms whose
   penalty for being written naturally is 20×–2000×, which is a permanent
   review burden on every future contributor. Against all that, the thing
   Cypher uniquely offers — variable-length paths — is measured at 408ms
   (1M) and 3.7s (10M) and is unusable, so the option's distinguishing
   value is not available at any scale this product runs at.
2. **Three separate edge tables, one per named graph** — structural
   separation with no discriminator anyone can forget, and per-graph
   lifecycle becomes a table operation. Rejected: three copies of every
   statement, three history tables, three trigger-function pairs, and
   cross-graph work becomes a union in every caller — for a guarantee
   decision 2 obtains in the type system at the cost of one enum
   argument. Recorded as the shape to revisit if the graphs' schemas ever
   genuinely diverge.
3. **One edge table LIST-partitioned by `graph`** — the middle path:
   physical separation, automatic pruning, per-graph decay as a partition
   operation, one query builder. Genuinely attractive and rejected as
   premature: it triples the RLS policy surface (a policy on the parent
   does not govern a partition addressed directly), adds DDL the AC does
   not need, and buys pruning at volumes the spike cleared unpartitioned
   at 10M edges. Recorded as the first upgrade if per-graph lifecycle or
   volume asks for it.
4. **A materialised k-hop closure table now** — ADR-0029's ladder rung 2,
   and the HIER-1 pattern already in this repo. Rejected as solving a
   problem the measurement does not show: 2-hop adjacency is 4.84ms at
   10M edges, and a closure table trades write amplification and storage
   for a read that already clears budget by two orders of magnitude. It
   stays rung 2, with the trigger recorded.
5. **A dedicated graph engine** (Neo4j, Memgraph, IndraDB, SurrealDB) —
   ladder rung 3. Rejected as it was in ADR-0004 option 3 and re-checked
   in the spike: the mature engines are GPL or BSL and fail the core-path
   licence rule, IndraDB is MPL-2.0 and would need an explicit cargo-deny
   exception, Oxigraph is RDF rather than a property graph — and any of
   them reopens two-phase commit between the record write and the graph
   write, a second backup and encryption story, and the loss of the TEN-2
   RLS backstop. Needs its own ADR, and only for a requirement the first
   two rungs structurally cannot serve.
6. **Polymorphic endpoints — `(kind, id)` pairs, no vertex table** — one
   fewer table and no vertex rows for things that already exist
   elsewhere. Rejected in decision 6: it trades referential integrity for
   row count, and this product's habit is to make the bad state
   unrepresentable rather than to sweep for it (MEM-4's deferred
   constraint trigger is the reference case).
7. **A `graph_refs` column on `records`**, as seed §4.2's field list
   suggests. Rejected on ADR-0039 option 8's argument, which was made
   about `superseded_by` and applies unchanged: a column drags the whole
   ADR-0006 structural rule behind it — history table, view, both trigger
   functions — for a relationship that is many-to-many and cannot say
   why. `graph_refs` is answered by an indexed read on the edge table.
8. **Bitemporal edges as valid-time only, with no history pair** — half
   the schema, and enough for GRPH-3's ranking. Rejected: the AC says
   history, MEM-5 and MEM-6 both went out of their way to keep the
   corpus's as-of meaningful, and CTX-5 turned that into a surface
   customers call. A graph that forgets is a graph that cannot answer the
   demo the seed leads with.
9. **A property bag on edges** — flexible, and every graph library has
   one. Rejected in decision 10.
10. **Keep AGE installed and dual-write edges to it**, so a future Cypher
    requirement finds the data waiting. Rejected outright: two systems of
    record for one claim, with no reader today to prove the second one
    correct — the failure mode is discovering years later that the mirror
    drifted.
11. **Defer GRPH-1 until GRPH-3 has a measured requirement** — arguably
    the most disciplined option, since nothing consumes the graph yet.
    Rejected: MEM-5 already ships an edge table, GRPH-2 needs a schema to
    write into, ADR-0039's trigger (d) is outstanding, and the gate's
    evidence is at its freshest right now. Deferring also leaves the
    seed's `recall` definition — "hybrid retrieval + graph traversal" —
    unbacked for another phase.
12. **Do nothing and keep ADR-0004 as written** — per-tenant AGE graphs.
    Not available: ADR-0029 already amended it on measured criteria, and
    G5 makes the per-tenant form string-built SQL, which CLAUDE.md
    forbids outright.

## Consequences

- Positive: the graph joins the rest of the storage layer rather than
  sitting beside it — compile-checked statements, bound parameters, one
  backup story, one RLS mechanism, commit-together with records for free
  (ADR-0001's property preserved without G3's mitigation); traversal is
  3–8× faster than the retained alternative at 2.5× less storage, on
  measured evidence rather than expectation; the disciplined query forms
  become types instead of review notes, and unbounded depth is a compile
  error; cross-tenant edges are structurally impossible again, recovering
  what ADR-0029's amendment had to downgrade to enforcement; the edge
  pair answers `as_of` through the same view shape CTX-5 already reads,
  so the graph inherits ADR-0042's "as-of rewinds the corpus, never the
  authority" without restating it; GRPH-3 plugs into a seam that already
  exists; and the product carries no dependency on an extension it does
  not call.
- Negative / accepted trade-offs: **ADR-0004's central technology choice
  is overturned**, and the seed's §7 stack list and architecture diagram
  still name Apache AGE — superseded by this ADR rather than rewritten,
  which is how ADR-0002 has stood beside the seed's OPA/OpenFGA since
  Phase 0, with the tech plan and the feature text amended in place
  (decision 13); there is no
  Cypher, so ad-hoc graph exploration by an operator has no surface until
  a console feature builds one (CNSL-2/CNSL-4 inherit the question);
  variable-length traversal is structurally unavailable, which is a real
  ceiling even though the retained alternative could not serve it either;
  the vertex table costs a row per participating thing and one more join
  than a records-only edge would need; the `graph` discriminator is
  defended in Rust rather than in the schema until option 3's
  partitioning, so raw SQL can still cross a semantic boundary — the
  adversarial suite is where that gets asserted, not the type system; the
  trigger functions duplicate migration 0001's shape because they
  enumerate columns; and the second bitemporal pair doubles the surface
  that migration 0001's structural rule governs, which is a standing
  review obligation on every future graph migration.
- Reversal triggers: traversal depth beyond 2 hops, or genuinely
  variable-length paths, become a product requirement → a fresh ADR, and
  the question is "accept a licence exception and a second engine, or
  bound the requirement", exactly as the spike recorded — **not** a
  return to AGE, which cannot serve it either; 2-hop expansion medians
  breach the slice ADR-0029's 300ms decomposition reserves for graph
  expansion, re-measured by EVAL-6 on production-shaped IO → ladder rung
  2's materialised k-hop closure table (option 4), the HIER-1 pattern
  generalised; a single deployment's entity graph exceeds ~100M edges, an
  order beyond what the spike measured and where its 1.58× slope may not
  hold → re-measure before assuming either rung; per-graph lifecycle
  (MEM-6 decay per graph, per-graph export) or volume makes the
  discriminator hot → option 3's LIST partitioning, a migration rather
  than a redesign; the graphs' schemas genuinely diverge → option 2's
  three tables; an operator surface for ad-hoc graph queries is demanded
  → decided in a console ADR, on the record, rather than by quietly
  reviving Cypher.

## Compliance notes

- **The PDP stays unbypassable, and this is the feature where that
  sentence has to be earned.** A knowledge graph is the classic way a
  policy layer gets walked around: an edge is a disclosure that its
  endpoints exist and are related. Decision 12 answers it structurally —
  the graph produces candidate ids and never scopes, expansion happens
  *before* `admit`, and the fused set is narrowed by ADR-0042 decision
  12's admission rather than widened by it. No new Cedar action, no new
  scope producer, no path from an edge to a record body that skips
  `composition_plan`. EVAL-5's leak suite gains graph paths explicitly,
  which is ADR-0004's compliance note inherited rather than dropped.
- **Tenant isolation** is doubled rather than moved: composite foreign
  keys make a cross-tenant edge unrepresentable (decision 7) *and* forced
  RLS keyed to the TEN-2 GUC backstops it (decision 8), with all three
  tables joining `crates/synveda-store/tests/rls.rs`'s adversarial suite.
  TEN-5's tenant deletion is a predicated delete — ADR-0029's accepted
  cost, restated here so it is not rediscovered.
- **Audit**: GRPH-1 adds no action type. Edges are derived material
  written by the extraction pipeline inside the transaction whose events
  already chain (ADR-0022, ADR-0019 decision 4), so the DoD's
  "audit events emitted for any new action type" is satisfied by there
  being none. If a human ever authors or deletes an edge directly, that
  is a new action, a new grant and a new ADR — recorded here so it cannot
  arrive as an implementation detail.
- **SQL discipline**: every statement is static and sqlx-checked, with
  seed sets bound as arrays; there is no runtime-built SQL anywhere in
  the traversal path, which is the criterion (G5) AGE failed and the
  reason ADR-0001's "enumerate every SQL statement in the binary" claim
  survives this feature intact.
- **Determinism**: traversal results are totally ordered before they
  leave the store (edge kind, then vertex id), so a graph-ranked input
  cannot make CTX-2's byte-identical re-composition depend on plan order.
- **Observability**: `graph.expand` span with seed count, depth and
  graph; `synveda_graph_expansion_duration_seconds` and
  `synveda_graph_edges_total{graph}`; the plan assertion of decision 9
  runs in the AC suite rather than in production (DoD #3).

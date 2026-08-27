# ADR-0044: Linking runs inside the extraction commit — the extractor's mention list is the only entity source, resolution is a deterministic key the schema's unique constraint enforces, and the provenance graph is projected rather than written

- **Status**: Superseded by ADR-0097
- **Date**: 2026-07-28
- **Feature(s)**: GRPH-2 (GRPH-3 inherits)
- **Deciders**: sujitn

## Context

GRPH-2 is "ingestion links records→entities→episodes; entity resolution
against existing nodes", and its acceptance criterion is "entity dedup
precision on fixture set; orphan rate tracked". GRPH-1 built the place
those edges go and said, in ADR-0043 decision 5, that
`(tenant_id, graph, kind, key)` is unique "so GRPH-2's entity resolution
has a place to converge". This ADR decides what converges there, who
writes it, and when.

Forces at play:

- **The schema was designed for this feature and left it three explicit
  holes.** ADR-0043 decision 5 names the convergence key; decision 10
  says "GRPH-2 adding the columns it needs as a reviewed diff" is the
  expected shape rather than a property bag; decision 11 hands over
  `record_supersessions` with "GRPH-2 owns the wiring". Migration 0026's
  header repeats all three. This ADR is the reviewed diff.
- **The extractor seam already has a mention field, and it has always
  been for this.** `CandidateRecord.entities` is documented as "a forward
  seam for MEM-5 dedup and GRPH-2 graph-linking", the shared LLM prompt
  already asks for "proper names mentioned by a candidate", and the
  schema both LLM extractors request already carries it. What it does not
  have is an implementation on the deterministic path, which is the path
  dev, demos and every asserted test run on (ADR-0022 decision 3).
- **A graph link is derived material about a record, decided at the
  moment the record is written.** The pipeline already runs dedup inside
  the write transaction for exactly this reason (ADR-0039 decision 1: "in
  this transaction, before the insert, so a record and the window it
  closes commit together"), and MEM-4 put the vector in the same
  statement as the record so the corpus can never hold an unembedded row
  (ADR-0023 decision 2). A graph written by a later pass would be a
  second consistency question this product has spent three features
  refusing to open.
- **The AC is a *precision* criterion, and that word chooses the
  algorithm.** A resolver that merges aggressively buys recall with
  false identity: two people called Paris become one node, and every
  traversal through that node is wrong in a way no downstream ranking can
  detect. A resolver that merges only on evidence leaves entities
  unmerged, which costs recall and is visible, recoverable, and honest.
  MEM-5 made the same call for records and said so (ADR-0039).
- **`graph_vertices` has no scope column, deliberately** (ADR-0043
  decision 12 and migration 0026's header: "a scope column here would be
  an authorisation input the PDP never granted"). Everything this stage
  writes into a vertex is therefore readable by any tenant-scoped read,
  regardless of the sensitivity of the record it came from. That is a
  constraint on *what may be written*, and no previous ADR has stated it
  as one, because until this feature nothing wrote vertices.
- **MEM-2's redaction placeholders are opaque by contract** (ADR-0021):
  extractors "preserve them verbatim, never guess at what they hid", and
  ADR-0022 decision 7 re-scans extractor output so a fabricated secret
  never persists. A resolver that interned `[REDACTED:github-pat]` as a
  vertex key would give a secret a stable graph identity and converge two
  unrelated secrets onto one node.
- **Nothing reads the graph yet.** GRPH-3 is the first reader, and
  ADR-0042 decision 12 already fixes its shape: expansion produces
  candidate ids that `admit` narrows and never widens. So GRPH-2 can be
  judged on what it writes, and the reader's obligations stay the
  reader's.
- **The claim's identity is not yet enforced anywhere.** Migration 0026
  has no uniqueness on `(graph, kind, src, dst)`, so two identical
  `mentions` edges are representable. Every other write path in this
  repo made re-assertion structurally idempotent —
  `record_supersessions` on its primary key, VedaFlow objects on
  `(tenant_id, hash)`, `upsert_vertex` on the resolution key — and a
  linker is exactly the shape of code that gets re-driven.

## Decision

**Linking runs inside the extraction commit transaction, over the
mentions the extractor already returns, resolving them to a deterministic
key that the schema's unique constraint turns into identity.** Two graphs
are written — `entity` and `episode` — and the third, `provenance`, is
projected from `record_supersessions` rather than materialised.

Decisions, specifically:

1. **The stage is a step in `commit_group`, not a pass of its own.** It
   runs after the record loop and before the derived-channel commit, on
   the same transaction, so a record and every claim about it either both
   land or neither does. No new queue, no new worker, no new lag SLO, and
   no window in which the corpus holds a record the graph has never heard
   of. This is ADR-0039 decision 1's placement and ADR-0023 decision 2's
   reasoning, applied to a third kind of derived material.

2. **The extractor's `entities` list is the only source of entity
   mentions, and GRPH-2 gives the deterministic extractor an
   implementation of it.** One seam, three implementations, exactly as
   ADR-0022 decision 3 built it: the LLM extractors already fill the
   field from the shared prompt, and the rule-based one now fills it with
   a capitalised-run heuristic and a sentence-initial stoplist. The
   linker never re-reads content. That is what keeps "which extractor
   ran" a provenance question rather than a behavioural fork, and it is
   why the linker cannot drift from the text that was actually persisted
   (the rescan happens upstream of it, ADR-0022 decision 7).

3. **Resolution is normalisation to a key, and the unique constraint is
   the resolver.** `upsert_vertex` on `(tenant, graph, kind, key)` is
   already an insert-or-converge, so resolution costs one statement per
   mention and needs no read-then-write race to lose. The rules are
   deterministic, ordered and few: casefold, collapse whitespace, strip
   edge punctuation, strip a possessive, strip a leading article, strip a
   trailing corporate suffix. No fuzzy matching, no edit distance, no
   embeddings, no LLM.

4. **Precision is bought with the confidence column rather than with a
   threshold.** A mention that reached its key on casefolding, whitespace
   collapsing and punctuation at the ends of tokens is recorded at 1000
   per mille: none of those changes *which* string it is, so two mentions
   that agree are the same string. A mention that needed a word removed —
   a possessive, a leading article, a trailing corporate suffix — is
   recorded at 900: the key is then a claim about equivalence rather than
   an observation of identity. Both are written; GRPH-3 may rank on the
   difference, and nothing is silently dropped for being merely probable.
   Integers per mille, never floats — the MEM-5 discipline (ADR-0039
   decision 2).

5. **Entity vertices are untyped, and `kind` is `name`.** The
   deterministic path cannot tell a person from a product, the LLM seam
   returns a list of strings rather than typed entities, and `kind` is
   part of the resolution key — so a type derived by guesswork would
   *split* the convergence point this feature exists to build. The day an
   extractor returns types is the day the key gains one, with a merge
   written deliberately. Until then a name is a name.

6. **Two edge kinds, one per graph, and both are facts rather than
   inferences about the world.** In `entity`: `record --mentions--> name`,
   confidence per decision 4. In `episode`: `record --occurred_during-->
   session`, confidence 1000, because the session identifier is a
   property of the event the record was extracted from and not a
   judgement anyone made. "Records→entities→episodes" is then a two-hop
   path through a record vertex, which is exactly the shape `expand`
   measures at 23.4ms.

7. **A vertex is never written without the edge that justifies it, in the
   same transaction.** There are no orphan vertices, so every name in
   `graph_vertices` is reachable from a record through an edge whose
   `source_record_id` names that record — which means a reader can always
   find the scope and sensitivity that govern a name before rendering it.
   That is the property that makes decision 8 survivable.

8. **A vertex label is not a disclosure surface, and this is a rule, not
   a habit.** `graph_vertices` has no scope (ADR-0043 decision 12), so
   anything written into `key` or `label` is readable by any tenant-scoped
   read. Therefore: a record-backed vertex's key and label are its
   record id and nothing else — never its content, never its class,
   never a summary. A name vertex is backed by no record at all, even
   when a record of class `entity` is plainly *about* it: binding the
   name to whichever record happened to mention it first would privilege
   that record and make the vertex a disclosure of it, and ADR-0043
   decision 5 left this choice here. An entity vertex's key and label are
   the name itself,
   which is the one bounded disclosure a graph cannot avoid making and
   still be a graph; it is the surface form the extractor already
   returned as a proper noun, capped by the schema at 512 characters, and
   nothing else from the record crosses. Any future column on either
   table is subject to the same test: could its value be a sentence from
   a `restricted` record? Then it does not belong on an unscoped table.

9. **A redaction placeholder is never an entity, and a mention has to be
   true of the text that was stored.** A mention containing `[REDACTED:`
   is refused before normalisation and counted as refused: interning one
   would give a secret a stable graph identity and, worse, converge every
   secret that hit the same rule onto one node — a cross-scope join
   through a redaction, which is the opposite of what ADR-0021 bought.
   The second half closes the gap ADR-0022 decision 7 opens: extractor
   output is re-scanned before it is persisted, so a live-format secret
   that admission missed becomes a placeholder *after* the extractor
   named it. Where that rescan changed a candidate, mentions that are no
   longer a substring of the persisted content are dropped. The check
   runs only on changed candidates and nowhere else, because an
   unscanned candidate cannot carry a redacted mention and because an
   LLM legitimately returns normalised names that never appear verbatim.

10. **The resolver refuses everything the schema would refuse, so a
    mention can never put a commit at risk.** Empty after normalisation,
    longer than the schema's 512-character bound, a placeholder,
    stoplist-only: refused in Rust, counted, and never sent to SQL. What
    remains can only fail on a genuine invariant breach, and a
    transaction that aborts for one of those is correct — the signal
    redelivers, and the bug is visible. The mention list is capped at 32
    per candidate for the same reason `MAX_EXPANSION_SEEDS` is 64; the
    excess is counted rather than dropped in silence.

11. **A claim's identity is `(tenant, graph, kind, src, dst)` among open
    claims, enforced by a partial unique index, and assertion is
    idempotent.** Migration 0027 adds
    `graph_edges_open_claim_unique ... where valid_to is null`, and
    `graph::assert_edge` inserts `on conflict do nothing`. Re-asserting a
    claim that already holds writes nothing — no second row, no history
    row, no counter — so a re-drive, a redelivery or a future re-linker
    converges instead of accumulating. The partial predicate is what
    keeps supersession legal: a closed window leaves a row behind, and
    the replacement is the one open claim.

12. **A restatement absorbed by dedup contributes no edges.** ADR-0039
    decision 10 says a merge keeps the survivor's content, vector and
    address; the restatement's text is never persisted, so it asserts
    nothing about the world that the survivor does not already assert,
    and a mention extracted from text nobody stored is a claim nobody can
    audit. `reinforce` is an observation count, not a new source.

13. **Mention edges do not expire.** `valid_from` is the record's
    valid-from and `valid_to` is `None`, permanently. "This record
    mentions this name" is a property of text that does not change, and
    copying the record's own validity onto the edge would make the corpus
    and the graph two systems of record for one fact — the rule ADR-0043
    decision 11 states and this decision obeys. A record whose window
    closes is answered as closed by the corpus, which is where GRPH-3's
    candidates go anyway. When retention destroys the record, the
    cascade takes the edge with it.

14. **The `provenance` graph is projected from `record_supersessions`,
    never written into `graph_edges`** — ADR-0039's trigger (d) and
    ADR-0043 decision 11, discharged as they were specified.
    `graph::supersession_edges` reads the system-of-record table and
    returns the edge-shaped view of it, keyed by `RecordId` because the
    records are not vertices and inventing vertices for them would *be*
    the mirror this was supposed to avoid. GRPH-3 fuses that list with
    `expand`'s output; both are candidate record ids, which is the only
    currency ADR-0042 decision 12 accepts.

15. **Orphan rate is a measurement of the stage, not an error condition.**
    `synveda_graph_link_records_total{graph, outcome}` counts every
    committed record as `linked` or `orphan` per graph, so the entity
    graph's orphan rate (records from which no name resolved) and the
    episode graph's (records from an event with no usable session id) are
    separately visible. A record that links to nothing is a normal
    outcome — "the staging cluster restarted" names nothing — and the
    number is the evidence for whether the extractor's mention recall is
    worth improving. The AC reports it; EVAL-2 owns any target.

16. **No feature flag, no pack configuration.** GRPH-3 is the
    feature-flagged, degradable half (its own AC says so); making the
    *write* switchable would mean the flag's off state silently produces
    a corpus the graph can never describe, and turning it back on would
    need a backfill nobody has specified. Linking is two statements per
    mention inside a transaction that already does more work than that.
    The reversal trigger is a measurement, not a preference: if linking
    shows up in the extraction lag histogram, it becomes a pack switch
    with a backfill, on the record.

17. **Vertices are touched in a deterministic order.** Mentions are
    resolved, deduplicated by key and sorted before any statement runs,
    so two workers linking the same popular name approach the shared rows
    in the same order — the discipline `commit_group` already applies to
    scopes ("visited in id order so two workers touching the same two
    scopes cannot deadlock by approaching them from opposite ends").

## Options considered

1. **A separate post-commit linking pass** (a second worker on its own
   queue, or a Temporal activity after the extraction workflow) — the
   textbook shape, and it keeps the extraction transaction small.
   Rejected: it reintroduces the exact consistency gap ADR-0023 closed
   for embeddings and ADR-0039 closed for supersessions, needs its own
   exactly-once story, its own dead-letter threshold and its own lag SLO,
   and buys nothing measurable — the work is two indexed statements per
   mention against tables the transaction has not touched. Recorded as
   the shape to revisit if decision 16's trigger fires.
2. **The linker re-reads record content and finds its own mentions**,
   ignoring the extractor's list. Rejected: two mention extractors would
   exist, they would disagree, and the LLM path's better recall would be
   thrown away by the very stage that needs it. It also puts content
   parsing downstream of the rescan, where a placeholder-mangling bug
   would be invisible.
3. **Fuzzy resolution now** — edit distance, token-set ratio, or cosine
   over the entity name's embedding, with a threshold. Rejected on the
   AC's own word: precision. Every one of these merges distinct entities
   at some threshold, the failure is silent and permanent (a merged
   vertex cannot be unmerged without a lineage the schema does not keep),
   and none of them is measurable until there is a labelled corpus bigger
   than a fixture file. Recorded as the first upgrade, gated on EVAL-2
   producing that corpus, and it arrives as a new `method` and a lower
   confidence band rather than as a rewrite of decision 3.
4. **Typed entity vertices** (`person`, `org`, `system`), from an LLM
   that returns types. Rejected for now in decision 5: `kind` is in the
   resolution key, so a wrong or missing type splits the node this
   feature exists to converge, and no extractor returns types today.
5. **Materialise the provenance graph** — write a `supersedes` edge into
   `graph_edges` beside every `record_supersessions` row. Rejected in
   decision 14 and, before that, in ADR-0043 decision 11: it is a dual
   write of one claim, and the failure mode is discovering years later
   that the mirror drifted. The cost of the projection is that a
   traversal cannot cross from a supersession into the entity graph in
   one statement; the trigger for revisiting is a reader that actually
   needs to.
6. **A `graph_refs` column on `records`**, as seed §4.2's field list
   suggests, filled by this stage. Rejected on ADR-0043 option 7's
   argument, unchanged: a column drags migration 0001's whole structural
   rule behind it for a many-to-many relationship that cannot say why it
   exists.
7. **Link only records of class `entity` and `episode`.** Superficially
   tidy — those are the classes the seed names — and rejected because it
   is backwards: a `decision` record that names three systems is exactly
   what a knowledge graph is for, and `records.class` describes what the
   record asserts, not whether it mentions anything. Class is not
   consulted at all.
8. **One vertex per mention occurrence, with resolution deferred to read
   time.** Rejected: it makes every traversal pay for resolution, the
   convergence point ADR-0043 decision 5 built goes unused, and "entity
   resolution against existing nodes" is the feature text.
9. **Skip the deterministic mention heuristic and let the network-free
   path produce no edges.** Honest, and rejected: it would make the
   orphan rate 100% in dev, in every demo and in every asserted test, so
   the AC would measure the LLM's behaviour or nothing at all. The
   heuristic is documented as a floor, not as the product path — the same
   position ADR-0022 decision 3 takes for the rule-based extractor.
10. **Defer GRPH-2 until GRPH-3 has a measured requirement.** Rejected on
    ADR-0043 option 11's reasoning: GRPH-3 cannot be built against an
    empty graph, ADR-0039's trigger (d) is outstanding, and the schema
    was built for this.

## Consequences

- Positive: a record and every claim about it commit together, so the
  graph cannot lag or diverge from the corpus and there is no second
  exactly-once problem; resolution costs one statement per mention and
  cannot lose a read-then-write race, because the convergence point is a
  unique constraint rather than a query; re-drives are structurally
  idempotent, so the linker is safe to run again by design rather than by
  care; the precision the AC asks for is bought by refusing to guess, and
  what the resolver *did* guess is legible in a per-mille column a ranker
  can use; the provenance graph is discharged without a dual write, so
  `record_supersessions` remains the only place a supersession is
  recorded; the disclosure surface of an unscoped table is now governed
  by a stated rule instead of by whatever the next contributor writes
  into a label; and nothing above the store gained a new authorisation
  input, so AUTHZ-5's leak suite answers the same question it did before.
- Negative / accepted trade-offs: **recall is deliberately poor** — "IBM"
  and "International Business Machines" are two nodes, and so are "Jörg
  Müller" and "Jorg Muller", because no transliteration or abbreviation
  rule can be added without evidence; the deterministic mention
  heuristic is a capitalisation heuristic and will both miss lowercase
  entity names and occasionally intern a sentence-initial word the
  stoplist does not carry, which is the honest floor of a network-free
  path and is measured rather than claimed; entity names are readable
  tenant-wide regardless of the sensitivity of the record that produced
  them (decision 8 bounds this to names and decision 7 makes the
  governing record always findable, but it is a real widening and it is
  stated here rather than discovered later); the linking work is inside
  the extraction transaction, so a pathological mention list lengthens a
  transaction that holds an archive lock (decision 10's cap bounds it);
  a merged vertex cannot be unmerged, because vertices carry no lineage
  — this is why decision 3 refuses fuzzy matching, and it is also the
  reason a future typed or fuzzy resolver needs a merge story before it
  needs a threshold; and the provenance projection cannot be traversed
  by `expand`, so a caller that wants supersession lineage and entity
  adjacency in one answer makes two calls and fuses them.
- Reversal triggers: linking appears in `synveda_extraction_lag_seconds`
  or lengthens the write transaction measurably → decision 16 becomes a
  pack switch plus a backfill, with the backfill specified before the
  switch; EVAL-2 produces a labelled entity corpus large enough to
  measure a fuzzy resolver → decision 3's first upgrade, arriving as a
  new `method` name and a confidence band below 900 rather than as a
  change to the existing keys; an extractor begins returning typed
  entities → decision 5's key gains a type and the migration that adds it
  carries the merge; the entity-graph orphan rate stays near 1.0 on the
  LLM path (not just the deterministic one) → the mention prompt is the
  problem, not the resolver, and it is ADR-0022's text to change; a
  reader needs supersession lineage and adjacency in one traversal →
  decision 14 is revisited with a materialisation and a drift test, on
  the record; a name turns out to be too much to disclose tenant-wide for
  some deployment → the answer is a per-graph read grant or a scoped
  vertex table, which is an ADR, not a column.

## Compliance notes

- **The PDP is not consulted by this stage, and that is correct rather
  than convenient.** Linking runs *after* `authorize_owner` allowed the
  write, on records that decision already permitted, inside that
  decision's transaction. It creates no new resource, no new action and
  no new scope: an edge names a record that the write path was authorised
  to create. ADR-0043 decision 12's rule — the graph is never a scope
  producer — is preserved because nothing here writes a scope, and the
  read side stays GRPH-3's obligation: expansion feeds ADR-0042 decision
  12's fused id list, which `admit` narrows and never widens.
- **Audit**: no new action type, as ADR-0043's compliance note reserved
  ("edges are derived material written by the extraction pipeline inside
  the transaction whose events already chain"). The group's existing
  `memory.extracted` event gains a `graph` summary — vertices and edges
  asserted, mentions refused, orphans per graph — because an auditor
  holding that event should be able to see what the graph learned from
  it without a second event asserting the same fact (ADR-0019
  decision 4). Direct human authorship or deletion of an edge remains
  what ADR-0043 made it: a new action, a new grant and a new ADR.
- **Tenant isolation** is unchanged and doubly held: every statement runs
  inside `rls::begin_tenant_tx`, and migration 0026's composite foreign
  keys already make a cross-tenant or cross-graph edge unrepresentable.
  Migration 0027 adds an index and no new grant; the app role still holds
  no DELETE on either table.
- **Redaction**: decision 9 is a compliance property, not an
  implementation detail — a `[REDACTED:*]` placeholder must never become
  a graph identity, and the refusal is counted so the metric can show it
  happening. The rescan-substring rule is the same property one layer
  up, and it is the reason the AC suite observes a real secret through
  quarantine and release rather than asserting the rule in a unit test.
- **SQL discipline**: two new statements, both static and sqlx-checked —
  `assert_edge`'s conditional insert and `supersession_edges`' projection
  — with the mention list bound as parameters. No runtime-built SQL.
- **Determinism**: mentions are deduplicated and sorted before any
  statement runs (decision 17), so a group's writes are ordered by key
  rather than by extractor output order, and two runs over the same
  events touch the same rows in the same sequence.
- **Observability**: `ingest.linking.link` span with record, name and
  edge counts; `synveda_graph_link_records_total{graph, outcome}`,
  `synveda_graph_link_mentions_total{outcome}` and
  `synveda_graph_link_duration_seconds`; the store's existing
  `synveda_graph_edges_total{graph}` counts only claims actually
  asserted, which decision 11 makes meaningful.

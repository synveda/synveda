# ADR-0039: Dedup & conflict detection — supersession is a closed valid window plus an edge, nominated by two signals because one of them is not always meaningful, and decided by a judge tuned to refuse

- **Status**: Accepted
- **Date**: 2026-07-26
- **Feature(s)**: MEM-5
- **Deciders**: sujitn

## Context

MEM-5's text is "near-dup merge (embedding + minhash); contradiction
detection creates explicit supersession edges with validity windows
(Graphiti pattern) — never ADD-only", and its acceptance criterion is
"LongMemEval knowledge-update category score ≥ baseline; superseded facts
retrievable via as-of but excluded from current inject".

Six accepted ADRs defer to this one, which is the usual sign that the
design was half-settled by the features that left room for it. ADR-0020
decision 5 refused to build a second dedup mechanism at the observe seam
because "MEM-5 is semantic (embedding/minhash near-dup of content)".
ADR-0022 decision 7 set `valid_from = occurred_at` and left `valid_to`
open because "supersession and validity-window management are MEM-5's".
ADR-0023 decision 4 declined a staleness check on the embedding sidecar
because "MEM-5's supersession work inherits that obligation".
ADR-0025 decision 6 shipped an exact-trimmed-content conflict predicate
and **exported the comparator** so "MEM-5 replaces the *predicate*
(embedding near-dup, supersession edges) and reuses the resolution".
ADR-0031 decision 6 put `valid_to` inside the content address, "so closing
a record's window produces a different object, which is what makes a
superseded record fall off its published set (MEM-6 and MEM-5 inherit the
obligation to re-commit when they rewrite)". ADR-0033 decision 14 left
similarity-triggered promotion to "MEM-5's similarity signal".

Forces at play:

- **The store is ADD-only today, and the feature text names that as the
  thing to fix.** Two observations of the same fact become two records;
  an observation that updates a fact becomes a third record beside the
  one it contradicts, and both compose. CTX-2's conflict rules drop the
  loser *from one block* by exact content match (ADR-0025 decision 6) —
  a rendering rule, not a statement about what is true, and it cannot
  see "we deploy on Tuesdays" against "we deploy on Thursdays" at all.
- **The default embedder has no geometry, and it is what the AC runs
  on.** ADR-0023 decision 6 says so in its own doc comment: the hash
  embedder's "geometry carries no meaning: equal texts collide, similar
  texts do not attract". Dev, demos, every hermetic test, and `make eval`
  all run on it. A design that nominates duplicates only by vector
  neighbourhood would detect exactly nothing in the configurations this
  feature has to pass in, and would look like it worked because the
  suite is green.
- **Precision is worth more than recall here, and the asymmetry is not
  close.** A missed update leaves a stale fact composing beside a fresh
  one — bad, visible, and what the product already does. A wrong
  supersession removes a true fact from every future inject, silently.
  The first is a quality regression; the second is the memory system
  losing something the user told it.
- **Contradiction cannot be read off a similarity number.** "We deploy
  on Tuesdays" / "We deploy on Thursdays" and "Deploys go through make
  deploy" / "Tests go through make test" sit at nearly the same lexical
  distance, and one pair is an update while the other is two true facts.
  Similarity can *nominate* a pair. Something else has to decide it.
- **The bitemporal pair already answers the AC's second half.** Valid
  time is application data set deliberately (ADR-0006), every composition
  read filters `valid_from <= at and (valid_to is null or valid_to > at)`
  (ADR-0025 decision 5), and `records::as_of`/`as_of_bitemporal` read
  `records_versions`. Closing a window *is* "excluded from current inject
  but retrievable as-of" — there is nothing to invent, only something to
  write.
- **A pipeline that rewrites records touches governed surfaces.** A
  scope's `memory/published` tree binds bytes, not ids (ADR-0031
  decision 5), and a record whose window closes changes address. The
  pipeline must not be able to remove reviewed material from a scope's
  trust boundary as a side effect of somebody's session.
- **GRPH-1 has not landed, and ADR-0029 constrains what "edge" may
  mean.** The gate passed conditionally: AGE stays, but graph names must
  be literals (G5 failed), so Cypher cannot be compile-checked SQL, and
  CLAUDE.md admits none. The supersession edge is read by the write path
  and must commit with the record.

## Decision

**Dedup and conflict detection run inside the extraction worker's write
transaction, per candidate, in valid-time order.** A candidate is
compared against the records its own scope already holds, nominated by
**two independent signals — MinHash LSH over content and ANN over the
stored embedding** — and resolved into exactly one of three outcomes:
**merge** (no new record; the survivor is reinforced), **supersede** (the
older record's valid window is closed at the newer one's `valid_from`, an
explicit `record_supersessions` edge is written, and both changed
addresses are re-committed to `memory/derived`), or **insert**. The
contradiction judge is a conservative, explainable rule tuned to refuse,
behind a seam the LLM judge (the Graphiti reading of the feature text)
can take without reshaping anything.

Decisions, specifically:

1. **The stage is a step in the existing write transaction, not a pass of
   its own.** It runs after embed (it needs the candidate's vector) and
   before the insert, inside `commit_group`'s tenant transaction. Three
   reasons, all of them structural rather than stylistic: the archive-lock
   makes that transaction the exactly-once boundary (ADR-0022 decision 2),
   so a record and the window it closes commit together or neither does;
   candidates in one group must be able to see each other, and inside the
   transaction that is free, because a query sees the transaction's own
   inserts; and the owner's `MemoryWrite` decision has already been taken
   there, so nothing new is authorised.

   A post-commit sweep is refused in options. A record that is current for
   one second is a record that can be injected in that second.

2. **Two nomination signals, because one of them is not always
   meaningful.** The lexical leg is MinHash LSH over the record's
   normalised word set — content-derived, meaningful under every
   configuration, and the reason the AC can pass without a model. The
   semantic leg is approximate-nearest-neighbour over the stored
   embedding, which catches the paraphrase the lexical leg cannot and is
   only as good as the embedder in use. Neither is authoritative: both
   produce *candidates*, scored exactly afterwards.

   This is the feature text's "embedding + minhash" read as a union rather
   than a conjunction, and the union is what makes the two legs
   complementary: a restatement in different words is a vector neighbour
   with low Jaccard, and a one-word edit is a band collision the hash
   embedder puts nowhere near it.

   Either leg may be *absent* without the write failing. A vector whose
   dimension has no ANN index (ADR-0024 decision 5) — a custom embedder, a
   re-embed in flight — costs that candidate its semantic nomination and
   nothing else. Dedup is a stage of the write, not a gate on it: a record
   that cannot be compared is inserted, which is the pre-MEM-5 behaviour
   and the only safe fallback.

3. **`record_signatures` is a sidecar written in the same statement as the
   record.** Columns: `record_id`, `tenant_id`, `signature bigint[]` (96
   MinHash values) and `bands bigint[]` (24 band hashes over 4 rows each),
   with a GIN index on `bands`. Nomination is `bands && $1` — one indexed
   array-overlap predicate, no join table, no second row per band. The
   MinHash is over the **normalised word-unigram set** rather than
   character shingles: a knowledge update usually reorders and re-words
   around a changed value, and unigram Jaccard survives that where
   5-grams fall under the band threshold (the worked numbers are in
   options 5).

   The banding is 24 × 4, whose S-curve puts the collision threshold at
   `(1/24)^(1/4) ≈ 0.45` Jaccard — deliberately well below the duplicate
   band, because nomination must also reach the pairs that turn out to be
   *conflicts*, which are by construction less similar than duplicates.

   **No deferred constraint trigger**, unlike embeddings (ADR-0023
   decision 4), and the difference is honest: a record without an
   embedding is unrepresentable in the read path, while a record without a
   signature is merely invisible to the lexical nominator. The API writes
   it in one statement so it cannot go missing through the product path;
   records written before this migration are not backfilled, exactly as
   ADR-0023 recorded for the MEM-3 window.

4. **Three outcomes, tested in this order: identical, conflict,
   near-duplicate, else insert.** Order matters and this order is the
   decision. Identical normalised content is a duplicate with certainty.
   *Then* the judge is asked, because a long statement with one changed
   value is both a high-Jaccard near-duplicate and a knowledge update, and
   testing "near-duplicate" first would merge exactly the updates the AC
   measures. Only after the judge refuses does a similarity threshold
   (Jaccard ≥ 0.90 or cosine ≥ 0.97) call it a near-duplicate. Everything
   else inserts, which is the pipeline's behaviour today and stays the
   default outcome.

5. **The judge is a conjunction of refusals over the content words.**
   Given incoming `N` and neighbour `O`, both derived, at the same scope,
   same owner, same class, `O` unpublished, contents not identical, `N`
   strictly newer in valid time — `N` supersedes `O` iff all of:

   - **the frames overlap**: with tokens normalised, stopwords dropped and
     *value tokens* (anything containing a digit, plus weekday and month
     names) held out, the overlap coefficient `|F(N) ∩ F(O)| / min(|F(N)|,
     |F(O)|)` is ≥ 0.70;
   - **the leading frame token is shared** — a crude subject proxy, and
     the conjunct that separates "we deploy on Tuesdays / Thursdays" from
     "deploys go through make deploy / tests go through make test";
   - **something actually changed**: the frames differ or the value-token
     sets differ.

   The overlap coefficient rather than Jaccard because an update is
   routinely *longer* than the statement it replaces ("the stand-up is at
   09:30" → "the stand-up moved to 10:15 from this week"), and Jaccard
   charges for the added words twice. Value tokens are held out of the
   frame and compared separately because "same subject, changed number" is
   the shape of most knowledge updates, and because a frame that included
   the number would score the update as *less* similar the more clearly it
   was one.

   This rule is tuned to refuse. It will miss real updates — a subject
   named last, a passive voice, a re-worded subject, a language whose word
   order is not English's. That is the asymmetry from the context section
   applied on purpose, and the miss rate is unmeasured until EVAL-2.

6. **The judge is behind a named seam, and the LLM judge is the recorded
   upgrade, not a stub.** The verdict function takes the pair and the
   config and returns `Insert | Merge | Supersede`, with the signals it
   used carried on the verdict for the audit payload. A model-backed judge
   — the Graphiti pattern's actual mechanism, asking which existing
   statements a new one invalidates — becomes another mode behind that
   function, reusing MEM-3's HTTP extractor clients. It is not built here
   because the AC must pass hermetically and because a judge whose
   precision nobody has measured should not be the one that ships first.
   The reversal trigger is in consequences.

7. **Supersession closes a valid window and writes an edge. It never
   deletes and never edits content.** `records.valid_to := winner
   .valid_from` through a narrow UPDATE, plus a `record_supersessions`
   row carrying both ids, the reason, the judge's method, the signals as
   integers, and when it was decided. Three properties fall straight out
   and none of them needed new machinery: the record stops composing (the
   valid-window predicate every composition read already applies), it
   stays readable at an earlier instant (`records::as_of`,
   `as_of_bitemporal`, `records_versions`), and its content address
   changes (ADR-0031 decision 6 put `valid_to` inside the object), which
   is what removes it from a published set it was on.

   An edge table rather than a `superseded_by` column on `records`:
   supersession is many-to-one (one new fact may close several), the
   column would alter `records` and therefore `records_history`,
   `records_versions` and both archive trigger functions in one migration
   (migration 0001's structural rule), and an edge can carry *why* while a
   column can only carry *that*.

8. **A late-arriving older fact is inserted already closed, never
   dropped.** When the incoming candidate loses on valid time — the
   session that produced it observed something that held earlier — it is
   inserted with `valid_to` set to the newer record's `valid_from` and the
   edge is written in the other direction. "Never ADD-only" cuts both
   ways: the pipeline may not silently discard an observation because it
   arrived late. When `valid_from` ties exactly, nobody supersedes
   anybody: both stay current and CTX-2's comparator resolves the block.

9. **Five boundaries, each refusing a class of damage.** The pipeline may
   close a window only on a record that is (a) `derived` — pinned material
   is "authored/canonical, cannot be shadowed or decayed" by seed §4.2;
   (b) at the same scope, (c) owned by the same identity — a user's
   session may not close a fact somebody else asserted, and for derived
   material scope and owner move together anyway; (d) of the same class —
   a `procedure` must not close an `episode`; and (e) **not named by its
   scope's `memory/published` tree**. The last is the governance boundary
   and the one worth stating twice: reviewed material leaves the trust
   surface through a proposal (FLOW-3/FLOW-5) or a rollback (FLOW-7), never
   as a side effect of an extraction. A conflict against published content
   is a real and interesting event; this feature declines to resolve it and
   leaves it to the promotion path, and the metric counts it.

10. **A merge reinforces the survivor; it does not rewrite it.** The
    duplicate's observation is absorbed into the survivor's provenance —
    a merge count and the absorbing event ids — through a narrow UPDATE
    that touches no content, on exactly the `records::reclassify`
    precedent (ADR-0038 decision 9). No content change means the vector
    still describes the text it was computed over (ADR-0023 decision 4's
    rule holds without a re-embed) and the content address is unchanged,
    so a published set is untouched by a merge. The survivor's version
    history gains a row, which is the bitemporal store doing its job:
    "this fact was observed again, at this time, from this event" is
    exactly the staleness and usage signal MEM-6 and FLOW-4 asked for.

11. **The read path verifies the valid window, so a superseded record
    cannot hold a ranking slot.** `search::hydrate_verified` gains the
    valid-time instant and applies the same predicate the composition
    reads apply. Composition already refused superseded material — this is
    about the fused candidate list, where a stale fact that still ranks
    costs a live one its place in the top-k. It also keeps CTX-1's
    contract exactly as written (ADR-0024 decision 6): the sidecar index
    may miss, never resurface, and now "superseded" is one more way for
    current truth to disagree with a lagging index.

12. **`DedupConfig` rides the effective pack.** `mode` (`off` | `merge` |
    `supersede`, where supersede implies merge), the three thresholds, and
    the nomination depth. All three embedded packs take the product
    default — `supersede`, on, everywhere — because the seed's own
    conflict order already says "newer valid-time beats older" (§4.4) and
    a pack that hoarded contradictions would be the surprising one. It
    resolves through the same effective-pack walk as redaction (ADR-0021
    decision 3) and composition (ADR-0025 decision 2), so a tenant that
    wants the pipeline to stop closing windows writes a stored pack and
    gets versioning, hot reload, audit and CLI for free. No second config
    plane (ADR-0025 option 1's argument, unchanged).

13. **One new audit action, `memory.superseded`, chained once per commit
    group.** It carries the id pairs, the judge's method, the signals, the
    closed windows, and the reason — never content, like every other
    memory event. It is a separate action rather than a field on
    `memory.extracted` because it asserts a different fact: extraction
    says what was created, and this says what stopped being current, which
    is the question an auditor arrives with and should not have to
    reconstruct from another action's payload. Merges stay *inside*
    `memory.extracted`'s per-event entry, because a merge creates nothing
    and closes nothing — it is an outcome of the extraction, not an act on
    the store.

    Similarities ride as integer per-mille values. Audit canonicalisation
    rejects floats (ADR-0019 decision 2, and the reason confidence lives
    in record provenance instead); a hash chain over a value jsonb may
    reshape is not a hash chain.

14. **The AC's first half is a measured eval category, and EVAL-3 still
    owns the benchmark.** The scenario format gains an optional
    `category`, the report reduces accuracy per category, and the baseline
    gains a `knowledge_update` axis — so the gate can fail naming the
    category, which is what "score ≥ baseline" has to mean in a repo whose
    eval discipline is pre-registered gates (ADR-0028 decision 5). The
    scenarios are written in LongMemEval's knowledge-update shape: a fact
    is stated, later updated, and the probe must return the update and not
    the original.

    The **baseline the AC compares against is the product before this
    feature**, measured and recorded in STATUS: an ADD-only store composes
    both facts and therefore scores 0 on a category whose grading forbids
    the stale one. Running the real LongMemEval corpus end to end —
    dataset, haystacks, LLM judge — is EVAL-3, a Phase 3 feature, and
    pretending otherwise here would be claiming a published benchmark
    score for a suite of four scenarios.

15. **Metrics and tracing on the new path.** `synveda_dedup_decisions_total`
    labelled `outcome = insert | merge | supersede | superseded_on_arrival
    | refused_published`, `synveda_dedup_candidates` (nominated per
    candidate, by leg), and `synveda_dedup_seconds`. The refusal label is
    the one that earns its place: it is how a tenant discovers that its
    published material is being contradicted by sessions and that somebody
    should open a proposal.

## Options considered

1. **A post-commit dedup sweep (Temporal, or a second polling worker)** —
   symmetrical with FLOW-4's promotion sweep, and it would keep the write
   transaction thin. Rejected: it makes "never ADD-only" a promise about
   eventual state, and the window between commit and sweep is a window in
   which the contradiction is live in every inject. The sweep shape is
   right for decay (MEM-6, which is about the passage of time) and wrong
   for this (which is about a write).
2. **Exact content-hash dedup only** — cheap, certain, no thresholds, no
   judge. Rejected: MEM-1's idempotency key already covers exact
   *delivery* duplication (ADR-0020 decision 2), and CTX-2 already drops
   exact content duplicates from a block. Doing it a third time would add
   nothing the product does not have; the feature is the semantic case.
3. **Nominate by embedding alone** — one leg, one index, no new table.
   Rejected on the context section's second force: the default embedder
   has no geometry, so this detects nothing in dev, demos, the AC test, or
   `make eval`, while looking correct in a suite that runs on real TEI.
4. **Nominate by scanning the scope's recent records instead of an
   index** — no signature table, no LSH parameters, and honest at small
   scale. Rejected: a knowledge update routinely contradicts a fact from
   months ago, and "recent" is exactly the wrong filter for it. A cap
   would silently bound correctness by corpus size.
5. **MinHash over character 5-grams rather than word unigrams** — the
   textbook choice, robust to punctuation and morphology. Rejected on the
   numbers: "we deploy on tuesdays" against "we deploy on thursdays" is
   J ≈ 0.46 in 5-grams and J = 0.60 in unigrams, and the first sits on top
   of the 0.45 band threshold — the canonical knowledge update would be
   nominated or not depending on a word's length. Unigrams are order-blind,
   which costs nothing here because the judge decides, not the signal.
6. **An LLM judge in this feature** — the Graphiti pattern as actually
   implemented upstream, and better at the cases the rule misses.
   Deferred, not refused: the AC must pass without a network, the write
   path would gain a second model round-trip per candidate against a 60s
   lag SLO, and a judge nobody has measured should not be the one that
   first gets to close windows. The seam is decision 6 and the trigger is
   in consequences.
7. **Supersession as an Apache AGE edge** — the graph is where "edge"
   naturally lives, and GRPH-2 is coming. Rejected for now on ADR-0029's
   verdict: graph names must be literals, so the statements cannot be
   sqlx-checked, which CLAUDE.md forbids outright; and this edge is read
   *by the write path* inside the record's own transaction. A relational
   edge table is rung one of that ADR's own fallback ladder. GRPH-2 may
   mirror these rows into the graph for traversal; nothing here prevents
   it.
8. **A `superseded_by` column on `records`** — one fewer table, and the
   read path could filter on it directly. Rejected: many-to-one, it drags
   the whole ADR-0006 structural rule behind it (history table, view, both
   archive trigger functions), and it cannot say why.
9. **Dedup at the observe seam, before extraction** — earliest possible
   point, cheapest work discarded soonest. Rejected: the content a record
   holds does not exist yet at that seam; extraction and summarisation
   produce it. ADR-0020 decision 5 already decided this and this ADR
   agrees with it.
10. **Merge by rewriting the survivor's content to the newer wording** —
    keeps the freshest phrasing. Rejected: a content rewrite needs a
    re-embed (ADR-0023 decision 4), changes the content address, and
    therefore demotes published material to unreviewed as a side effect of
    somebody restating a fact in a session.
11. **Let the pipeline supersede published records** — it is the same
    contradiction and arguably the more important one. Refused as
    governance: FLOW-2's whole point is that published is the trust
    boundary and only review moves it. The refusal is counted, so the
    signal reaches the humans who can act on it.
12. **Cross-class supersession** — a fact re-extracted as a decision would
    then be caught. Rejected: class is the coarsest guard available and
    the damage from a `procedure` closing an `episode` is worse than the
    miss. Recorded as a known miss.

## Consequences

- Positive: the store stops being ADD-only, and it stops in the one place
  that can be atomic about it; "what did the agent know on date X" now has
  something interesting to answer, because facts have ends as well as
  beginnings; the AC's second half is a property of the schema rather than
  of application discipline; the feature works with no model configured,
  which is the configuration every demo and the eval harness run in; the
  contradiction rate becomes visible (metrics + one audit action) instead
  of being a thing nobody can count; MEM-6 inherits a reinforcement signal
  and CTX-5's as-of surface inherits a corpus where as-of means something.
- Negative / accepted trade-offs: the deterministic judge's recall is
  unmeasured and its precision is asserted by construction rather than
  measured — EVAL-2 is where both get numbers; supersession is refused
  entirely against published material, so a scope can hold a reviewed fact
  its own members have contradicted, and the only signal is a counter;
  every candidate now costs two nomination queries and a hydrate inside
  the write transaction (bounded, but real, against the <60s lag SLO); the
  signature sidecar is one more row per record and is not backfilled;
  `records_history` grows by one row per merge, which is the cost of
  recording reinforcement rather than dropping it.
- Reversal triggers: (a) EVAL-2 measures the deterministic judge's
  precision below 0.95 on a labelled conflict set → the LLM judge lands
  behind decision 6's seam and becomes the default for tenants with an
  extractor configured; (b) `refused_published` climbing at a real tenant
  → a supersession *proposal* (FLOW-3's shape: the pipeline opens it, a
  curator decides), which is the surface this ADR deliberately did not
  invent; (c) nomination cost showing up in
  `synveda_extraction_lag_seconds` → the ANN leg goes behind a config flag
  before the lexical one, since the lexical leg is the one the AC depends
  on; (d) GRPH-2 landing → these rows are mirrored as graph edges for
  traversal, the table staying the system of record; (e) a tenant needing
  supersession across scopes → it is a promotion, and FLOW-5 already knows
  how to climb.

## Compliance notes

- **The PDP stays unbypassable.** The stage adds no authorization path: it
  runs inside the `MemoryWrite` decision `authorize_owner` already took
  for that owner (ADR-0022 decision 4), and it reads and writes only
  records at the owner's own home scope. It cannot widen what a session
  may see, because it only ever *removes* material from current
  composition. Nothing here consults a policy pack for a decision — only
  for configuration, resolved through the same effective-pack walk as
  every other pack config.
- **Tenant isolation.** `record_signatures` and `record_supersessions` are
  tenant-scoped tables and get enabled + forced RLS, a policy, and grants
  in the same migration (ADR-0009); both join the adversarial suite and
  its structural completeness guard, and both cascade with their records.
  Every query in this feature filters `tenant_id` explicitly regardless —
  the backstop is a backstop.
- **Audit.** One action added (`memory.superseded`), chained inside the
  same write transaction as the records it describes, with the chain head
  taken last (ADR-0019 decision 1). Merges ride the existing
  `memory.extracted` event. No payload carries record content, and no
  payload carries a float.
- **Reversibility of a wrong decision.** A supersession is recorded, not
  destructive: the record, its content, and every version of it remain in
  `records`/`records_history`, and reopening a window is an ordinary
  bitemporal update. This feature ships no operator surface for that, and
  that gap is named here rather than left to be discovered — AUD-2's audit
  query surface and CNSL-2's console are where a curator would act on the
  `memory.superseded` trail.

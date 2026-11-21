# ADR-0164: Business entities are typed Knowledge named with standard vocabularies, and systems of record stay authoritative

- **Status**: Accepted, **amended 2026-11-20** (Amendment 1) and
  **2026-11-21** (Amendments 2 and 3)
- **Date**: 2026-11-16; accepted 2026-11-20
- **Feature(s)**: Typed business entities and relationships
- **Deciders**: sujitn

## Amendment 3 (2026-11-21): business links in graph expansion

Following business links in context-run expansion settled how decision 3's
traversal works.

1. **A separate switch.** Configuration's `context.graph.include_entity_links`
   turns it on. It is off by default and left out of canonical bytes when
   off, so existing Configuration hashes do not move. Every template ships it
   off, and it needs graph expansion enabled. Turning it off, or disabling a
   predicate, stops expansion following those links.
2. **One frontier under ADR-0097's bounds.**
   - Expansion reads each frontier item's relations and then its business
     links. The two kinds share the anchors, the two-hop limit, and the
     candidate, token and time budgets.
   - Fan-out applies to each kind: at most `fan_out_per_node` relations and
     at most `fan_out_per_node` links per item.
   - A path may mix relations and links, and the best score reaching an item
     wins whichever kind of edge carried it.
3. **Which links are followed.**
   - The link is valid at the run instant, as recorded now.
   - The caller may read its other end, decided afresh and within the
     request's sensitivity ceiling.
   - That end is not transitional, archived or erased. A stale or superseded
     end is recorded as excluded and expands no further, as a relation's
     would.
   - The source's scope still enables the predicate.
4. **A fixed weight.** On 2026-11-21 the owner chose 0.50 for every link, the
   weight of `supersedes`, below the evidence relations. Hop penalty,
   freshness and current-state scores apply as in ADR-0097.
5. **Hidden links leave no trace.** On 2026-11-21 the owner chose this over
   ADR-0097's aggregate policy flag, which relations keep.
   - Fan-out counts only links that pass, so `graph_fanout_truncated` reports
     only visible links left out.
   - None of these raises the run's policy flag: an unreadable link end, an
     item reached by a link and then denied, or a link step that no longer
     holds when the plan re-reads it. The path is dropped.
   - The time budget is the exception. Deciding hidden ends takes time, and a
     run that runs out records `graph_time_budget_exceeded` as any other does.
6. **Read decisions are taken in bulk.** Each level's link ends are decided
   together by the walk recall uses (Amendment 2 item 4). A body is read only
   for an end that passed, and an end whose revision moved on in between is
   left out.
7. **The trace names the link.**
   - Migration 0021, identical in both lineages, lets a `context_graph_steps`
     row follow a business link. A row holds exactly one of a relation type
     and a predicate, which hashes-only retention and the erasure scrub both
     keep, with only that kind's id. A link step always supports.
   - Views and audit show `entity_link_id` and `predicate`, and leave out
     `relation_type`.
   - A step's hash covers the link id, predicate, direction, both revisions
     with their content hashes, and the asserting revision.
   - A run that may follow links records the graph version
     `knowledge-relations-entity-links-v1`.
8. **Rechecks.**
   - Before a path is ranked, each of its link steps must still hold: both
     ends current at the step's revisions, the link recorded as the step
     names it and valid, its predicate enabled at the source's scope, and the
     step's three revisions readable.
   - Replaying a saved delivery checks the same at replay time, with both
     ends current active, so a delivery over a link that has since ended or
     been disabled is refused.
   - The run inspector shows a retained link step while the link is recorded
     and its revisions are readable, even after the link has ended.
9. **Erasure.** This reverses Amendment 1 item 7's "no erasure routine
   changed". Erasing either end still deletes its links by cascade. A row
   trigger on `entity_links` now clears the addresses of every step that
   followed a deleted link, keeping its hashes and predicate, as the erasure
   routine already does for relation steps. Tenant erasure deletes steps
   before links.

## Amendment 2 (2026-11-21): business links in recall

Showing links on recall hits (scope item 6) settled how recall reads them.

1. **Every hit carries its links.** Recall hits at summary and full depth
   carry their current business links: valid at the query instant, as
   recorded now.
   - Any hit can carry links, not only an entity. On 2026-11-21 the owner
     chose this, so a decision hit shows what it concerns and who decided.
   - Each link shows its predicate, which way it runs from the hit
     (`outbound` or `inbound`, as graph steps spell it), and the other end's
     id, Knowledge type, class and current title.
   - Evaluation queries carry no links, because they measure retrieval.
2. **A link shows only when three things hold:**
   - the caller may read its other end, decided afresh;
   - the other end is current active Knowledge at the query instant, as a
     hit must be;
   - the source scope's effective Configuration still enables the predicate.
     On 2026-11-21 the owner chose this, so emptying the vocabulary takes
     links out of recall. `GET /v1/knowledge/{id}/links` still shows the
     recorded history.
3. **Limits count only links that show.**
   - A hit shows at most 16 links, oldest first, and `links_truncated` says
     when more would show.
   - A limit or a flag over stored links would tell readers about links they
     cannot see, as Amendment 1 item 6 found for the links read.
4. **Read decisions are taken in bulk.**
   - The other ends' heads are read in one query, without their bodies.
   - The decision input is gathered once per scope and project, by the same
     step a single read uses. Each item still gets its own Cedar decision,
     and its historical evidence is rechecked when its revision carries
     some.
   - `GET /v1/knowledge/{id}/links` takes its decisions the same way.
   - Each round decides the next 17 links of every hit still short of its
     limit together, and a hit drops out once it has more than it shows.
     The worst case is still 1,000 decisions for a hit, and 100,000 for a
     query at its 100-hit limit.
5. **Audit.** The query's read event records how many links it showed. When
   the trace retention records results, each result also records the ids of
   the links it showed.

## Amendment 1 (2026-11-20): how business links are governed

Building the link table settled points decision 3 left open.

1. **Predicates have their own allowlist.** On 2026-11-20 the owner chose a
   Configuration `entity_predicates` list on the same pattern as
   `entity_classes`: it is empty by default, left out of the canonical
   document when empty, and can only narrow. A link needs three things from
   the source scope's effective Configuration: its predicate enabled, its ends
   fitting the predicate's endpoint rule, and every class endpoint it uses
   enabled.
2. **The endpoint rules.** "Agents" are `schema:Person`,
   `schema:Organization` and `prov:SoftwareAgent`; "organisations" are
   `schema:Organization` and `org:OrganizationalUnit`.

   | Predicate | Source | Target |
   |---|---|---|
   | `org:memberOf` | agents | organisations |
   | `org:headOf` | `schema:Person` | organisations |
   | `org:reportsTo` | `schema:Person`, `prov:SoftwareAgent` | `schema:Person` |
   | `org:subOrganizationOf` | organisations | organisations |
   | `schema:worksFor` | `schema:Person` | organisations |
   | `schema:owns` | `schema:Person`, organisations | `schema:Product`, `schema:Service`, `schema:SoftwareApplication`, `schema:Place`, `synveda:System` |
   | `prov:wasAttributedTo` | the `decision` type | agents |
   | `dcterms:subject` | the `decision`, `convention` and `warning` types | every class |
   | `synveda:customerOf` | `schema:Person`, `schema:Organization`, `synveda:Account` | organisations, `schema:Product`, `schema:Service` |
   | `synveda:dependsOn` | `schema:Product`, `schema:Service`, `schema:SoftwareApplication`, `synveda:System` | the same four |
   | `synveda:accountableFor` | `schema:Person`, organisations, `prov:SoftwareAgent` | `schema:Product`, `schema:Service`, `schema:SoftwareApplication`, `synveda:Account`, `synveda:Contract`, `synveda:System` |

   Every target is a class, so an empty class allowlist disables every link.
3. **Links are two Knowledge commands under the existing review.**
   - `link` asserts a link from the source revision the author inspected,
     which becomes the asserting revision.
   - `end_link` appends an end with an exclusive `valid_to` after the link's
     `valid_from`. A link ends once, and the same pair may be linked again.
     Ending stays possible after the predicate is disabled, so a rollback
     cannot strand a link.
   - Both name both ends as their targets, so forgetting either end clears
     their payloads and rejects them while open. The result envelope
     carries `entity_link_id`.
4. **Admission order.**
   - The target is authorised for reading first, and an absent target is
     refused with exactly the same error as an unreadable one.
   - Then writing the source is authorised, and the allowlists and the
     endpoint rule are checked. An end must fall after its link's start.
   - Lifecycle and an open link are the effect's to check, as for other
     Knowledge commands. A link to archived Knowledge, or a second end, opens
     a change that is rejected.
   - Admission reads the source's class at the expected revision, which the
     link pins, and refuses a revision the source does not have. It freezes
     the target revision it read into the command before the payload is
     hashed. The classes the allowlists and the endpoint rule checked
     therefore hold until the link is recorded, through review and any
     concurrent edit; a target that moves on, even by Verify, rejects the
     change.
   - When a reviewed change applies, authorisation runs again. The effect then
     checks that the source is still at the asserting revision and the target
     at the frozen one, both ends are active, stale or transitional, and
     neither has reached the link limit (item 6). If any check fails, the
     change is rejected. The effect locks both items until it commits, so
     neither can change, and no other link to either can be recorded, between
     its checks and the insert.
   - Whoever applies a reviewed link must be able to read its target. For
     anyone else the target is not found, and the change stays open.
   - The allowlists apply as reviewed, as for classes.
5. **Reviewers see ids, not content.** A reviewer of the source scope sees
   the target's id, the target revision admission read and the predicate in
   the review payload, as a merge reviewer sees input ids. The target's
   content and class are not shown; a time-ordered id does show when the
   target last changed.
6. **Reads are bitemporal and per end.**
   - An item's links can be read valid at an instant (`as_of`) as recorded
     by another (`as_known_at`).
   - A link is shown only when the reader may read both ends, and
     the count recorded in audit is the visible count.
   - An item can have at most 1,000 links in its lifetime, ended ones
     included, because rows are append-only and any instant can be read. The
     limit is enforced when a link is recorded, so a read always returns
     every link the reader may see and never refuses because of links they
     cannot see. The cost is a refusal to a writer, who can read the target
     and write the source, that one of the two has reached the limit. No
     lens can move it. Paging waits until an item needs more.
7. **Erasure cascades.** Links and their ends are deleted with the item at
   either end, inside governed Knowledge erasure and tenant erasure, by
   foreign-key cascade. The Knowledge erasure routine deletes relations by
   name but not links, so no erasure routine changed.

## Context

Teams want agents to answer questions such as "who owns the billing service",
"which customer is this contract for" and "who decided we stop retrying". That
needs named business things and the relationships between them. The product
already has most of the parts, but none of them carries business meaning.

**What the product has today**
- `KnowledgeType::Entity` ("a named thing and what is known about it"), with
  no class and no identifiers.
- `KnowledgeRelation`: append-only claims between items, typed by an
  *epistemic* vocabulary: `supports`, `contradicts`, `supersedes`,
  `derived_from`, `references`, `related_to`, `transitions_to`, `duplicates`.
  - It has forward-compatible `metadata` and records the asserting revision.
  - It has **no validity of its own**. Adjacency reads every relation whatever
    the asserting revision's state, and nothing ends a relation. A business
    link such as "reports to" changes over time, so this gap is real.
- ADR-0097 makes `KnowledgeRelation` the one product graph: bounded two-hop
  expansion with a Cedar decision on every endpoint. Apache AGE and the
  Record graph were removed, and a second engine needs new evidence.
- Extraction already returns `entities`, the proper names a candidate
  mentions, into candidate `metadata.entities`. No reader uses them.
- The organisational graph for authority already exists: identities, SCIM
  directory sync, governed scopes and Cedar entities. The open identity
  stitching plan holds that similarity never becomes identity.
- The trusted MCP catalogue (ADR-0086) governs which external MCP servers a
  project trusts, pins approved versions and advertises them to agents.
- OKF v0.2 exchange (ADR-0087) preserves unknown concept `type` values.

**External facts, checked on 2026-11-16**
- **SQL/PGQ:** committed to PostgreSQL in March 2026 and reverted on
  2026-09-07 (commit `b1f106c`, "Revert SQL Property Graph Queries"). It is
  not in PostgreSQL 19, and the product admits 14–18.
- **Mem0** removed graph memory from its open-source SDK in v3 with "no OSS
  replacement".
- **LinkML** is Apache-2.0 and generates JSON Schema, Pydantic, TypeScript,
  Rust, SQL DDL, SHACL, OWL and JSON-LD contexts.
- **Benchmarks:** no independent study found shows that ontology-constrained
  extraction improves conversational memory answers. GraphRAG-Bench finds
  graphs help multi-hop and summary questions, and tie or lose on single
  facts. The case for this feature is precision and governance, not a
  benchmark gain.
- **Graph-memory engines:** Graphiti (Apache-2.0) has the strongest
  agent-memory entity model, but it needs Neo4j (GPL-3.0), FalkorDB (SSPL) or
  Neptune.

## Decision

1. **Systems of record stay authoritative; Synveda holds governed knowledge
   *about* entities.**
   - CRM, CMDB, data catalogue and ontology products are reached through
     their own MCP servers, governed by the trusted catalogue.
   - An entity item may carry external identifiers so an agent can look the
     thing up there.
   - Synveda does not mirror their records or their ontologies.
2. **An entity is an `entity` item with a class from a governed vocabulary.**
   - The product declares a closed set of classes as compact IRIs:
     - schema.org: `Person`, `Organization`, `Product`, `Service`,
       `SoftwareApplication`, `Place`, `Event`
     - W3C ORG: `org:OrganizationalUnit`
     - PROV-O: `prov:SoftwareAgent`
     - product-namespaced terms where no standard term fits:
       `synveda:Account`, `synveda:Contract`, `synveda:System`
   - Governed Configuration's `entity_classes` selects the allowlist. It is
     empty by default, which disables classes, and an empty list is left out
     of the canonical document so existing Configuration hashes do not move.
     Configuration resolves by scope chain, so the allowlist that applies is
     the effective one at the item's scope; binding it at the tenant root
     makes it per tenant.
   - A revision declares its class in metadata as
     `synveda_entity: {"schema_version": 1, "class": "<compact IRI>"}`. The
     declaration is part of the reviewed revision, so its content hash covers
     it, and it needs no schema change.
     - Admission validates the shape with the rest of the content, including
       at insert.
     - Only an `entity` item may declare a class, and only a class its scope
       enables. That check runs after authorisation, so a refusal cannot
       describe the Configuration of a scope the caller may not write. It
       runs when a change is proposed; a pending change applies as reviewed,
       and Verify copies the current class.
     - Removing a class from the allowlist stops new declarations of it.
       Recorded classes stay on their immutable revisions and are still
       shown; an edit that keeps a disabled class is refused.
   - Reads show the declared class on each revision and on summary recall
     hits.
   - Aliases and external identifiers are free text that the admission scan
     and recall would also have to carry. They wait for a later
     `schema_version`.
   - Roles such as customer, owner and approver are relationships, not
     classes.
3. **A business link carries a governed predicate between two items.**
   - Predicates come from the same vocabulary. Each predicate states its
     allowed source and target endpoints. An endpoint is either an entity
     class or a Knowledge type, because some links start from items that
     have no class: a decision, a convention, a warning. Examples:
     - `org:memberOf`, `org:headOf`, `org:reportsTo`,
       `org:subOrganizationOf`: entity class to entity class
     - `schema:worksFor`, `schema:owns`: entity class to entity class
     - `prov:wasAttributedTo` for "who decided": from the `decision`
       Knowledge type to `schema:Person`, `schema:Organization` or
       `prov:SoftwareAgent`
     - `dcterms:subject` for "what a decision or convention is about": from
       the `decision`, `convention` or `warning` Knowledge type to any
       allowed entity class
     - product-namespaced `synveda:customerOf`, `synveda:dependsOn`,
       `synveda:accountableFor` where standards are silent
   - Admission checks the endpoint rule against the source item's Knowledge
     type and the target's class, or the reverse. So "what did we decide
     about service X" is the decisions with a `dcterms:subject` link to X,
     and their `prov:wasAttributedTo` links name who decided.
   - Links live in their own append-only `entity_links` table (option 2),
     not in `KnowledgeRelation`, which would need a business-link exception
     in each epistemic reader; Options considered gives the evidence.
   - The predicate is a column of the link. A link records its asserting
     revision and a valid-time window. Ending a link appends an end record,
     and every reader filters on it, so a link's lifetime does not depend on
     its asserting revision staying current.
   - Admission authorises the target before it checks the endpoint rule, so
     a refusal reveals neither the target's existence nor its class.
   - Traversal includes business links only when Configuration says so,
     through a second bounded path under ADR-0097's limits and per-endpoint
     Cedar, with a fixed weight so ranking stays deterministic.
4. **Entity resolution is deterministic and reviewable.**
   - Each mention in `metadata.entities` is matched within the reader-visible
     scope by exact external identifier, or by exact normalised name or
     alias.
   - A match becomes a *proposed* `references` link on the capture
     candidate, reviewed with it.
   - A model may propose a `duplicates` candidate. Nothing merges entities,
     and nothing links across scopes the reader cannot see.
5. **No link is authority.** No relation, extracted or reviewed, feeds Cedar
   or RLS. If access by business relationship is ever wanted, that is a
   separate authorisation decision, limited to links asserted by a system of
   record.
6. **Standards are mapped at the edge.**
   - JSON-LD export uses one `@context` and maps relations:
     - `derived_from` → `prov:wasDerivedFrom`
     - `supersedes` → `prov:wasRevisionOf`
     - `references` → `dcterms:references`
     - `related_to` → `dcterms:relation`
   - `supports`, `contradicts` and `transitions_to` have no standard
     equivalent and stay native.
   - Business predicates export as their own IRIs.
   - OKF export carries the entity class as the concept `type`.
   - Software agents described with A2A AgentCard fields map to
     `prov:SoftwareAgent`, and delegation maps to `prov:actedOnBehalfOf`.
7. **One declaration for the vocabulary.** The closed Rust types in the
   types crate are the declaration; LinkML is not used.
   - Classes, predicates and their endpoint rules are declared there, and the
     wire names agree with them by test.
   - The JSON-LD context and any JSON Schema are generated from the same
     types when export lands, with a test that the peers agree.

## Options considered

1. **Typed `entity` items plus a business relation type inside
   `KnowledgeRelation`.**
   - For: reuses one graph, one review path and one authorisation seam.
   - Against: needs relation lifetime, and widens a closed vocabulary that
     the graph planner reads.
   - Rejected on 2026-11-20, when mapping the relation code showed that
     graph evidence treats every relation except `contradicts` as
     supporting, the adjacency read lists relation types by hand and
     truncates fan-out by type name, the unique key has no place for a
     predicate, the type list is repeated in both migration lineages, and
     the table cannot record an end.
2. **A separate append-only `entity_links` table (chosen).**
   - For: keeps epistemic and business relations apart, and lifetime is
     native to the new table.
   - Against: a second adjacency path to bound and authorise, which strains
     ADR-0097's "one product graph".
3. **Embed Graphiti, Cognee or a graph database.**
   - Rejected: GPL or SSPL backends, or a second store with its own custody
     and authorisation.
4. **Federate everything, store no entities.**
   - Rejected: decisions, conventions and ownership claims made in agent work
     have no system of record to point at.
5. **Import a full ontology (FIBO, BFO/CCO, all of schema.org) with OWL or
   SHACL in core.**
   - Rejected: no evidence it helps, high operating cost. SHACL 1.2 and
     RDF 1.2 are not yet Recommendations.
6. **Use SQL/PGQ.** Rejected for now: it was reverted and not released, and
   the product supports PostgreSQL 14–18.

## Consequences

- **Positive:**
  - Agents can answer ownership and "who decided" questions with exact
    provenance.
  - Exports speak standard vocabularies.
  - Nothing new is trusted for authority.
- **Negative / accepted:**
  - Business predicates the standards lack are product-namespaced.
  - Links need review like other Knowledge.
  - The vocabulary is per tenant, so terms can drift between tenants.
- **Reversal trigger:** a pilot question shows no precision gain over plain
  recall.

## Compliance notes

- Every entity and link read repeats Knowledge read decisions on both
  endpoints. A relationship never grants access to either.
- Writes stay typed VedaFlow commands with content-free audit.
- Vocabulary selection is governed Configuration that can only narrow.
- Resolution runs inside the reader's authorised scope and never uses email
  or display-name similarity as identity.
- Licences: only compact IRIs are stored. No labels, definitions or
  vocabulary files from schema.org, W3C ORG, PROV-O or Dublin Core are
  copied or vendored; the owner accepted that basis on 2026-11-20. LinkML is
  not used.

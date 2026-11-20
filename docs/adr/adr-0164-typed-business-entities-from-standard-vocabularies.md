# ADR-0164: Business entities are typed Knowledge named with standard vocabularies, and systems of record stay authoritative

- **Status**: Accepted
- **Date**: 2026-11-16; accepted 2026-11-20
- **Feature(s)**: Typed business entities and relationships
- **Deciders**: sujitn

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

# MEM-11: Typed business entities and relationships

## Problem and evidence

Agents cannot answer business questions such as "who owns the billing service", "which customer is this contract for" or "who decided we stop retrying" from governed Knowledge.

What exists today:
- **Entities have no class.** `KnowledgeType::Entity` items carry no class and no external identifiers.
- **Relations are only epistemic.** `KnowledgeRelation` types say how claims relate (supports, contradicts, supersedes, derived_from, references, related_to, transitions_to, duplicates), not how business things relate.
- **Relations never end.** The store's adjacency read ignores whether the asserting revision is still current, and no command ends a relation.
- **Mentions are captured but unused.** Extraction writes the proper names a candidate mentions into candidate `metadata.entities` (`crates/synveda-ingest/src/capture_worker.rs`), and no reader uses them.

What exists to build on:
- the trusted MCP catalogue (ADR-0086), which can govern access to systems of record;
- OKF exchange (ADR-0087), which preserves unknown concept types.

Research on 2026-11-16, recorded in [ADR-0164](../adr/adr-0164-typed-business-entities-from-standard-vocabularies.md):
- Standard vocabularies cover classes and organisational structure well: schema.org, W3C ORG and PROV-O.
- No independent evidence shows ontology-constrained extraction improves memory answers.
- PostgreSQL SQL/PGQ was reverted on 2026-09-07 and is not in PostgreSQL 19.

## Scope

1. A governed per-tenant vocabulary of entity classes and business predicates, declared once in the repository. Each predicate states its allowed source and target endpoints, and an endpoint is an entity class or a Knowledge type. For example, `prov:wasAttributedTo` runs from a `decision` to a person, organisation or software agent, and `dcterms:subject` runs from a `decision`, `convention` or `warning` to an entity.
   - It starts from schema.org, W3C ORG and PROV-O terms.
   - It adds product-namespaced terms where those standards are silent.
2. An `entity` revision may carry a class, aliases and external identifiers. They are validated at admission and covered by the content hash.
3. Business links in a separate append-only `entity_links` table, each carrying a governed predicate, a valid-time window and an appended end record.
   - They are typed VedaFlow work under the existing review.
   - Traversal includes them only when Configuration enables it.
4. Deterministic mention resolution: exact external identifier or exact normalised name or alias, within the reader-visible scope.
   - Matches become proposed `references` links on capture candidates.
   - A model may propose only `duplicates` candidates.
5. JSON-LD export:
   - with one `@context`;
   - mapping relations to PROV-O and Dublin Core where an equivalent exists;
   - with the class as the OKF concept `type`.
6. Agent access: `recall` results show an entity's class and identifiers and its current business links, each endpoint authorised afresh.
7. A pilot on one real question ("who owns service X and what did we decide about it"), measuring answer precision with and without typed links.

### Current state (2026-11-20)

ADR-0164 is accepted. The owner chose a separate link table (option 2) and storing compact IRIs only, and LinkML is not used. Amendment 1 records how links are governed, including the owner's choice of a separate predicate allowlist.

**Entity classes (scope item 2, class only), in source:**
- The types crate declares twelve classes as compact IRIs. Governed Configuration's `entity_classes` enables a subset; it is empty by default and omitted from canonical bytes.
- A revision declares `metadata.synveda_entity` v1 with its class. The shape is validated with the rest of the content, including at insert.
- After authorisation, admission refuses a class on a non-`entity` item or one the scope's effective Configuration does not enable. An outsider gets the authorisation refusal, not the class refusal.
- Revisions and summary recall hits show `entity_class`, and `synveda recall` and the MCP `recall` text show it after the type.
- Pure, exact-role and CLI rendering tests cover the refusals, the outsider's authorisation refusal, display, and edit and Verify after the allowlist is emptied.
- OpenAPI, the console client and both SDK contracts are regenerated. The console keeps the field when it saves a document but does not edit it.

**Business links (scope item 3, without traversal), in source:**
- The types crate declares eleven predicates as compact IRIs, each with its source and target endpoints. Governed Configuration's `entity_predicates` enables a subset; it is empty by default and omitted from canonical bytes.
- Migration `0020`, identical in both lineages, adds the append-only `entity_links` and `entity_link_ends` tables with forced row security. Both cascade from the items they join, and the command check admits `link` and `end_link`.
- `POST /v1/knowledge/{id}/links` asserts a link and `POST /v1/knowledge/{id}/links/{link_id}/end` ends one, as reviewed Knowledge commands. The target is authorised before anything about it is checked, so an absent target and an unreadable one get the same refusal.
- `GET /v1/knowledge/{id}/links` reads an item's links valid at `as_of` as recorded by `as_known_at`. A link is shown only when the reader may read both ends.
- Exact-role tests cover:
  - the allowlist and endpoint refusals;
  - the identical refusal for an absent and an unreadable end;
  - ending and the bitemporal lens;
  - erasure of links and ends with either end;
  - reviewed links that apply as reviewed, and changes rejected once their link has ended or their target is archived.
- The tenant archive and erasure fixture holds a link and its end.

**Not yet built:**
- aliases and external identifiers;
- link traversal behind a Configuration switch, and links in `recall`;
- mention resolution;
- JSON-LD export and the OKF class;
- the policy-leak fuzz across classes, predicates, roles and scopes;
- the pilot.

**Release:** migration `0020` makes the next release the first published pair that runs a migration since the open transition (ADR-0156 Amendment 1) shipped, so that release owes the open-transition upgrade drill.

**Next action:** links in `recall` and a second bounded traversal path. `recall` shows each entity's current links with both ends authorised afresh. Traversal follows links only when Configuration enables it, under ADR-0097's bounds with a fixed weight. The item links read is bounded at 1,000 links and refuses beyond that, while the relation read stays unbounded. Neither is paged, so recall must bound how many links it reads and authorise their other ends in bulk. The policy-leak fuzz then covers both paths.

## Non-goals

- A second graph engine:
  - Apache AGE;
  - Neo4j (GPL-3.0), FalkorDB (SSPL), Graphiti or Cognee as an embedded store;
  - SQL/PGQ.
- Mirroring CRM, CMDB, Palantir, Fabric or other systems of record. They are federated through the trusted MCP catalogue.
- OWL reasoning or a SHACL engine in core.
- Adopting RDF 1.2 or SHACL 1.2 before they are W3C Recommendations.
- Importing FIBO, BFO/CCO or all of schema.org. Individual FIBO terms may be cherry-picked for finance tenants.
- Entity merges.
- Identity linking by email, display name or model similarity. Identity stitching stays in MEM-7.
- Any relationship acting as authority for Cedar or RLS.

## Architecture seam

**Types**
- Vocabulary, class and predicate types live in `synveda-types` with closed validation.
- A generated JSON Schema and JSON-LD context come from the same declaration, and a test checks that the peers agree.

**Storage**
- The entity class lives in revision metadata and needs no migration.
- The link table and its end records are additive forward migrations in `synveda-store`.
- All SQL is static and SQLx-checked.

**Gateway**
- Commands are typed Knowledge/VedaFlow commands.
- Traversal of business links is a second bounded path beside `context_api/graph.rs`, under ADR-0097's bounds, with a fixed weight.
- Resolution runs in the capture worker before candidate review.

**Clients**
- The MCP and SDK surfaces change only through the generated contracts.

## Acceptance criteria

**Entities and links**
- An administrator selects a vocabulary in Configuration. An `entity` revision with an unlisted class, or a link whose source or target does not fit its predicate's endpoint rule (entity class or Knowledge type), is refused at admission with a typed error.
- A decision links to the entity it concerns with `dcterms:subject` and to who decided with `prov:wasAttributedTo`.
- A business link can be ended, and an ended link stops contributing to traversal and recall at its end instant.
- Its history stays readable in the as-known lens.

**Resolution**
- A captured mention of an existing entity becomes a proposed `references` link only by exact identifier or exact normalised name or alias within the reader's scope.
- No path merges entities or links across scopes the reader cannot read.

**Authorisation**
- A reader without access to an endpoint finds no trace of the link, the endpoint or a count in recall, manifests, traversal traces or errors.

**Exchange**
- JSON-LD export is checked as a graph, not just against its `@context`. The exported document is expanded, and its classes and relations are compared with the expected set for a fixture tenant.
- An exported property the context does not map fails the export check, because JSON-LD expansion silently drops unmapped terms.
- OKF export carries the entity class.

**Pilot**
- The pilot question is answered with exact provenance.
- Precision with and without typed links is recorded.

## Required tests

**Pure tests**
- Vocabulary validation.
- Domain and range checks.
- Normalisation and exact matching.
- The JSON-LD mapping, compared on the expanded graph, with a negative case where an unmapped property must fail rather than vanish.
- Agreement between generated peers.

**Exact-role database tests**
- Admission refusal.
- Ending a link.
- Bitemporal reads.
- Traversal with business links on and off.
- Resolution within scope.

**Fuzz**
- Policy-leak fuzz across classes, predicates, roles and scopes, with a positive control.

**Pinning**
- A test that accepting a capture candidate preserves `metadata.entities` into the published revision.

**Exchange**
- Export round-trip through OKF.

## Rollout and rollback

An empty vocabulary disables the feature, and every built-in template ships empty.

- **Rollout:** a tenant opts in by selecting a vocabulary. Business-link traversal is a separate Configuration switch.
- **Rollback:** select an empty vocabulary or disable traversal. Existing revisions keep their recorded class, which reads still show, and an edit that keeps a disabled class is refused. Links stay immutable and inert.
- **Migrations:** they are additive under the forward-only upgrade contract.

## Dependencies

- **Pilot:** an owner must supply a real pilot question and representative data.

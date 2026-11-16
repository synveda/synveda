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

1. A governed per-tenant vocabulary of entity classes and business predicates, with allowed source and target classes for each predicate, declared once in the repository.
   - It starts from schema.org, W3C ORG and PROV-O terms.
   - It adds product-namespaced terms where those standards are silent.
2. An `entity` revision may carry a class, aliases and external identifiers. They are validated at admission and covered by the content hash.
3. One business relation type carrying a governed predicate, with a valid-time window and an explicit end command.
   - It is typed VedaFlow work under the existing review.
   - Traversal includes it only when Configuration enables it.
4. Deterministic mention resolution: exact external identifier or exact normalised name or alias, within the reader-visible scope.
   - Matches become proposed `references` links on capture candidates.
   - A model may propose only `duplicates` candidates.
5. JSON-LD export:
   - with one `@context`;
   - mapping relations to PROV-O and Dublin Core where an equivalent exists;
   - with the class as the OKF concept `type`.
6. Agent access: `recall` results show an entity's class and identifiers and its current business links, each endpoint authorised afresh.
7. A pilot on one real question ("who owns service X and what did we decide about it"), measuring answer precision with and without typed links.

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
- Revision fields and relation lifetime are additive forward migrations in `synveda-store`.
- All SQL is static and SQLx-checked.

**Gateway**
- Commands are typed Knowledge/VedaFlow commands.
- Traversal stays in `context_api/graph.rs` under ADR-0097's bounds, with a fixed weight for business links.
- Resolution runs in the capture worker before candidate review.

**Clients**
- The MCP and SDK surfaces change only through the generated contracts.

## Acceptance criteria

**Entities and links**
- An administrator selects a vocabulary in Configuration. An `entity` revision with an unlisted class, or a link whose predicate's source or target class does not fit, is refused at admission with a typed error.
- A business link can be ended, and an ended link stops contributing to traversal and recall at its end instant.
- Its history stays readable in the as-known lens.

**Resolution**
- A captured mention of an existing entity becomes a proposed `references` link only by exact identifier or exact normalised name or alias within the reader's scope.
- No path merges entities or links across scopes the reader cannot read.

**Authorisation**
- A reader without access to an endpoint finds no trace of the link, the endpoint or a count in recall, manifests, traversal traces or errors.

**Exchange**
- JSON-LD export validates against the published `@context`.
- OKF export carries the entity class.

**Pilot**
- The pilot question is answered with exact provenance.
- Precision with and without typed links is recorded.

## Required tests

**Pure tests**
- Vocabulary validation.
- Domain and range checks.
- Normalisation and exact matching.
- The JSON-LD mapping.
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
- **Rollback:** select an empty vocabulary or disable traversal. Existing revisions and links stay immutable and inert.
- **Migrations:** they are additive under the forward-only upgrade contract.

## Dependencies

- **Decision:** ADR-0164 must be accepted, including the choice between a business relation type inside `KnowledgeRelation` and a separate link table.
- **Legal:** confirm that using schema.org IRIs without vendoring its CC BY-SA 3.0 files is acceptable, and check the DCMI terms licence.
- **Tooling:** decide whether LinkML (Apache-2.0) is a build-time dependency, or whether the declaration is a repository-local format with its own generator.
- **Pilot:** an owner must supply a real pilot question and representative data.

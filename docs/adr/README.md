# Architecture decision record index

This is the current classification overlay for Synveda's ADRs. ADR bodies
remain the historical decision record and are not rewritten when the product
changes. The header-status column preserves each ADR's declared lifecycle
status, with long amendment prose compacted. `Current` means the rationale or
proposal has not been removed; it does not turn a `Proposed` header into an
accepted or implemented decision.

The template is intentionally excluded. Every other ADR appears exactly once.

## ADR-0001 through ADR-0034

| ADR | Concise title | Header status | Current classification | Features | Replacement/removal |
| --- | --- | --- | --- | --- | --- |
| [ADR-0001](adr-0001-postgres-first-rust-stack.md) | Postgres-first Rust stack | Accepted | Current | FND-6 (FND-1, FND-2) | — |
| [ADR-0002](adr-0002-cedar-embedded-pdp.md) | Embedded Cedar PDP | Accepted | Current | FND-6, AUTHZ-1, AUTHZ-6 | The application facade is elaborated by ADR-0012. |
| [ADR-0003](adr-0003-vedaflow-in-postgres.md) | VedaFlow in Postgres | Accepted; object model specified by ADR-0030 | Current | FND-6, FLOW-1..8 | Object storage is ADR-0030; typed review is ADR-0091. |
| [ADR-0004](adr-0004-multi-graph-age-schema.md) | Named Apache AGE graphs | Accepted; amended; engine choice superseded by ADR-0043 | Removed with Record/AGE graph (ADR-0097/CPR-38) | FND-6, GRPH-1..4 | ADR-0097 defines bounded `KnowledgeRelation` expansion without AGE. |
| [ADR-0005](adr-0005-uuidv7-identifiers.md) | UUIDv7 domain identifiers | Accepted | Current | FND-3, FND-4 | — |
| [ADR-0006](adr-0006-bitemporal-tables.md) | Bitemporal current/history tables | Accepted | Current | FND-4 | Knowledge history and temporal queries are refined by ADR-0080 and ADR-0096. |
| [ADR-0007](adr-0007-observability-baseline.md) | Tracing, OpenTelemetry and metrics baseline | Accepted; deferred clause landed | Current | FND-5 | — |
| [ADR-0008](adr-0008-tenant-resolution.md) | Token-derived tenant context | Accepted | Current | TEN-1 | — |
| [ADR-0009](adr-0009-rls-tenant-backstop.md) | Forced-RLS tenant backstop | Accepted | Current | TEN-2 | — |
| [ADR-0010](adr-0010-oidc-login.md) | OIDC code and PKCE login | Accepted | Current | AUTH-1 | — |
| [ADR-0011](adr-0011-hierarchy-store.md) | Fixed hierarchy closure store | Accepted | Removed with hierarchy store (ADR-0074/CPR-7) | HIER-1 | ADR-0070 and ADR-0074 replace it with the governed scope tree. |
| [ADR-0012](adr-0012-cedar-pdp-embedded.md) | Cedar facade and policy-pack store | Accepted | Current | AUTHZ-1 | Resource construction is re-cut over governed anchors by ADR-0073. |
| [ADR-0013](adr-0013-jit-provisioning.md) | JIT identity provisioning | Accepted | Current (partially superseded by ADR-0074 and ADR-0093) | AUTH-2 | Mapping/quarantine placement was removed; principal scopes, directory adoption and the administrator grant remain. |
| [ADR-0014](adr-0014-policy-packs.md) | Policy packs and composition | Accepted | Current (partially superseded by ADR-0089) | AUTHZ-2 | Per-node assignment was replaced by governed Configuration selection. |
| [ADR-0015](adr-0015-roles-role-bindings.md) | Legacy roles and role bindings | Accepted | Removed with role bindings (ADR-0074/CPR-7) | AUTHZ-3 | ADR-0072 scope grants and the closed `RoleKey` vocabulary replace it. |
| [ADR-0016](adr-0016-scope-chain-resolver.md) | Legacy scope-chain cache | Accepted | Removed with scope-chain cache (ADR-0074/CPR-7) | HIER-2 | Governed ancestors resolve per request; only PDP entity invalidation remains. |
| [ADR-0017](adr-0017-cedar-entity-sync.md) | Cedar entity fragments | Accepted | Current (partially superseded by ADR-0074) | HIER-3 | Entity fragments remain; governed scope chains replaced the legacy cache feed. |
| [ADR-0018](adr-0018-service-identities.md) | Service identities and confinement | Accepted | Current (partially superseded by ADR-0074) | AUTH-3 | Service placement now uses principal scopes and scope grants. |
| [ADR-0019](adr-0019-hash-chained-audit-log.md) | Hash-chained audit log | Accepted; amended | Current | AUD-1 | Query/export are extended by ADR-0045 and ADR-0092; ADR-0064 amendment 3 adds the narrow repairable key-provision witness. |
| [ADR-0020](adr-0020-observe-ingestion.md) | Idempotent event ingestion | Accepted; partially superseded by ADR-0078 | Current (partially superseded by ADR-0078) | MEM-1 | `/v1/observe`, PGMQ and the old buffer were removed; session events and the durable adapter spool preserve the delivery doctrine. |
| [ADR-0021](adr-0021-redaction-secret-scanning.md) | Admission redaction and secret scanning | Accepted | Current | MEM-2 | Capture and skill admission use the surviving fail-closed scanning rules. |
| [ADR-0022](adr-0022-extraction-pipeline.md) | Extraction pipeline | Accepted | Current (partially superseded by ADR-0083) | MEM-3 | Database-leased capture batches and reviewable candidates replace PGMQ and direct Record writes. |
| [ADR-0023](adr-0023-transactional-embed-or-fail.md) | Transactional embedding invariant | Accepted | Superseded by ADR-0080 and ADR-0082 | MEM-4 | Governance commits publish immutable revisions without waiting for embeddings; a bounded, idempotent worker converges the revision sidecar. |
| [ADR-0024](adr-0024-hybrid-retrieval.md) | Authorised lexical/vector fusion | Accepted | Current (partially superseded by ADR-0084) | CTX-1 | Knowledge retrieval keeps authorised fusion; the Record corpus and Tantivy sidecar were removed. |
| [ADR-0025](adr-0025-composition-engine.md) | Record-era context composition | Accepted; decisions 2 and 7 superseded by ADR-0031 | Superseded by ADR-0084 | CTX-2 | ADR-0084 owns immutable Knowledge planning and trace disclosure. |
| [ADR-0026](adr-0026-inject-api.md) | Injection degradation ladder | Accepted; partially superseded by ADR-0078 | Current (partially superseded by ADR-0078) | CTX-3 | `/v1/inject` was removed; bounded context-run degradation remains. |
| [ADR-0027](adr-0027-claude-code-adapter.md) | Claude Code adapter | Accepted; amended three times | Current | ADPT-1, ADPT-8, CPR-14 | Lifecycle evidence is ADR-0079; support claims are governed by ADR-0098. |
| [ADR-0028](adr-0028-eval-harness.md) | Unprivileged evaluation harness | Accepted | Current | EVAL-1 | Product outcome methodology is extended by ADR-0099. |
| [ADR-0029](adr-0029-graph-traversal-gate.md) | Apache AGE adoption gate | Accepted | Removed with Apache AGE graph (ADR-0043/GRPH-1) | GRPH-4, GRPH-1..3, MEM-5, CTX-5 | ADR-0097 carries the surviving explicit-bound and fallback requirements. |
| [ADR-0030](adr-0030-vedaflow-object-store.md) | VedaFlow content-addressed object store | Accepted | Current | FLOW-1 | — |
| [ADR-0031](adr-0031-vedaflow-channels.md) | VedaFlow channels and derived publication | Accepted | Current | FLOW-2 | — |
| [ADR-0032](adr-0032-vedaflow-proposals-approval-matrix.md) | VedaFlow proposal approval matrix | Accepted | Current (partially superseded by ADR-0091) | FLOW-3 | ADR-0091 replaces recorded proposal review decision 7 with one typed artifact lifecycle. |
| [ADR-0033](adr-0033-auto-promotion-rules.md) | Record-usage auto-promotion sweeper | Accepted | Removed with Record promotion pipeline (ADR-0083/CPR-18) | FLOW-4 | The audit projection, Record usage rules and automatic promotion worker have no current successor; typed auto-apply is a separate ADR-0091 path. |
| [ADR-0034](adr-0034-cross-scope-promotion.md) | Governed cross-scope promotion | Accepted | Current | FLOW-5 | — |

## ADR-0035 through ADR-0067

| ADR | Concise title | Header status | Current classification | Features | Replacement/removal |
| --- | --- | --- | --- | --- | --- |
| [ADR-0035](adr-0035-cli-review-flow.md) | Governed CLI review | Accepted | Current | FLOW-6 | ADR-0091 makes the review surface artifact-neutral. |
| [ADR-0036](adr-0036-rollback-and-pinning.md) | VedaFlow rollback and pinning | Accepted | Current | FLOW-7 | — |
| [ADR-0037](adr-0037-lapses.md) | Legacy policy lapses | Accepted | Superseded by ADR-0090 | AUTHZ-4 | Immutable, expiring policy relaxations replace the lapse table and worker. |
| [ADR-0038](adr-0038-abac-conditions.md) | Closed ABAC conditions and restricted floor | Accepted | Current | AUTHZ-5 | ADR-0090 adds the governed restricted-tier exception without weakening the floor. |
| [ADR-0039](adr-0039-dedup-and-conflict-detection.md) | Record deduplication and conflicts | Accepted | Superseded by ADR-0096 | MEM-5 | Durable Knowledge conflict sets and typed resolution replace Record-era judging. |
| [ADR-0040](adr-0040-decay-ttl-and-staleness.md) | Record decay and staleness | Accepted | Superseded by ADR-0096 | MEM-6 | Governed, version-evidenced Knowledge freshness replaces pack-time Record decay. |
| [ADR-0041](adr-0041-tiered-injection.md) | Tiered context rendering | Accepted | Current (partially superseded by ADR-0084) | CTX-4 | ContextRun selection and authored-context composition replace the inject-specific surface. |
| [ADR-0042](adr-0042-recall-api-and-mcp-tool.md) | Scoped recall query | Accepted; partially superseded by ADR-0078 | Current (partially superseded by ADR-0078) | CTX-5 | `/v1/recall` was removed; Knowledge/context queries and the generic MCP boundary carry the surviving query doctrine. |
| [ADR-0043](adr-0043-graph-schema.md) | Record adjacency graph | Superseded by ADR-0097 | Superseded by ADR-0097 | GRPH-1, GRPH-2, GRPH-3 | Bounded `KnowledgeRelation` traversal replaces the Record graph. |
| [ADR-0044](adr-0044-graph-linking.md) | Record extraction graph linking | Superseded by ADR-0097 | Superseded by ADR-0097 | GRPH-2, GRPH-3 | Explicit immutable Knowledge relations replace extractor-written graph edges. |
| [ADR-0045](adr-0045-audit-query-surface.md) | Tenant-complete audit query | Accepted | Current | AUD-2, AUD-3, AUD-4, CNSL-3 | ADR-0092 adds typed context-platform evidence and frozen-head export. |
| [ADR-0046](adr-0046-extraction-quality-suite.md) | Extraction quality gate | Accepted | Current | EVAL-2 | ADR-0099 incorporates it into the product outcome suite. |
| [ADR-0047](adr-0047-retrieval-and-injection-quality.md) | Retrieval and context quality gate | Accepted | Current (partially superseded by ADR-0099) | EVAL-4 | ContextRun delivery/use outcomes replace the deleted injection lens. |
| [ADR-0048](adr-0048-security-evals.md) | Zero-tolerance security evaluation | Accepted | Current | EVAL-5 | ADR-0099 retains these as explicit trust gates. |
| [ADR-0049](adr-0049-prompt-registry.md) | Governed prompt registry | Accepted | Current | PRMT-1 | — |
| [ADR-0050](adr-0050-context-packs.md) | Governed context packs | Accepted | Current (storage/composition amended by CPR-43) | PRMT-2 | Immutable `context_pack_chunks` and VedaFlow publication remain; chunks are authored ContextRun inputs, not Records or Knowledge-index entries. Configuration selects the effective authored-context policy (ADR-0089). |
| [ADR-0051](adr-0051-skills-registry.md) | Mutable Skill registry | Superseded by ADR-0085 | Superseded by ADR-0085 | SKIL-1 | Stable Skill aggregates and immutable versions replace mutable drafts. |
| [ADR-0052](adr-0052-skill-security-scanning-gate.md) | Whole-bundle Skill security scan | Accepted | Current | SKIL-2 | ADR-0085 re-anchors the unchanged gate on immutable Skill versions. |
| [ADR-0053](adr-0053-skill-quality-scoring.md) | Legacy Skill quality scoring | Superseded by ADR-0085 | Superseded by ADR-0085 | SKIL-3 | Version-bound scan/rubric evidence replaces checklist overrides. |
| [ADR-0054](adr-0054-skill-distribution.md) | Legacy Skill distribution set | Superseded by ADR-0085 | Superseded by ADR-0085 | SKIL-4 | Revisioned principal/project bindings replace channel-backed materialisation. |
| [ADR-0055](adr-0055-smb-profile-and-init.md) | Small-team installation and init | Accepted | Current (partially superseded by ADR-0074 and ADR-0095) | OPS-1 | Governed scopes replace hierarchy seeding; immutable Configuration replaces runtime profile branches. |
| [ADR-0056](adr-0056-admin-console-shell.md) | Console without a second authority | Accepted; amended | Current | CNSL-1, CNSL-2, CNSL-3, CNSL-4 | Product routing and generated-client boundaries are extended by ADR-0075 and ADR-0088; ADR-0102 adds a distinct explicit-development HTTP cookie mode without changing HTTPS. |
| [ADR-0057](adr-0057-generic-mcp-server.md) | Generic MCP adapter boundary | Accepted; amended twice | Current | ADPT-2, ADPT-3, ADPT-4 | Evidence-based client support is ADR-0098. |
| [ADR-0058](adr-0058-hierarchy-and-policy-explorer.md) | PDP-backed scope and policy explorer | Accepted | Current (partially superseded by ADR-0074 and ADR-0075) | CNSL-2 | Governed scopes replace the fixed hierarchy; forecasts remain non-authoritative. |
| [ADR-0059](adr-0059-scim-directory-sync.md) | SCIM directory projection | Accepted; amended twice | Current (partially superseded by ADR-0093) | AUTH-4, AUTH-5 | ADR-0093 replaces the separate SCIM group graph with shared identities, groups and grants. |
| [ADR-0060](adr-0060-directory-pull-sync.md) | Safe directory pull reconciliation | Accepted; amended | Current (partially superseded by ADR-0093) | AUTH-5 | ADR-0093 converges pull and SCIM onto one projection and secret boundary. |
| [ADR-0061](adr-0061-public-benchmark-adapters.md) | Governed LongMemEval adapter | Accepted; amended | Current | EVAL-3 | ADR-0099 separates delivery, use and outcome signals. |
| [ADR-0062](adr-0062-enterprise-profile-and-helm-chart.md) | Enterprise Helm deployment | Accepted; amended | Current (partially superseded by ADR-0095 and ADR-0102) | OPS-2 | Deployment shapes no longer select product behaviour; governed Configuration does. Compose is now the reference contract and Helm maps it with Kubernetes-native primitives. |
| [ADR-0063](adr-0063-tenant-partitioned-storage.md) | Tenant storage partitioning decision | Accepted; amended | Current (partially superseded by ADR-0080) | TEN-3 | Record-specific benchmark evidence is historical; the unpartitioned pgvector decision remains current for Knowledge. |
| [ADR-0064](adr-0064-per-tenant-envelope-keys.md) | Per-tenant envelope encryption | Accepted; amended three times | Current (partially superseded by ADR-0094) | TEN-4 | ADR-0094 adds stable secret identities and durable envelope rotation; amendment 3 defines repairable key-provision evidence. |
| [ADR-0065](adr-0065-release-and-distribution.md) | Release packaging and distribution | Accepted; amended eight times | Current | OPS-8 | — |
| [ADR-0066](adr-0066-beta-demo-profile.md) | Operator-seeded beta demo | Proposed; amended once | Current (partially superseded by ADR-0100) | OPS-9 | ADR-0100 provides the resumable public-API PulseBoard demo; externally dependent beta evidence remains open. |
| [ADR-0067](adr-0067-uninstall-and-cleanup.md) | Uninstall and cleanup | Proposed | Current | OPS-10 | — |

## ADR-0068 through ADR-0102

| ADR | Concise title | Header status | Current classification | Features | Replacement/removal |
| --- | --- | --- | --- | --- | --- |
| [ADR-0068](adr-0068-context-platform-domain-and-epoch.md) | Context-platform domain and epoch | Accepted | Current | CPR-1 | — |
| [ADR-0069](adr-0069-schema-epoch-and-local-reset.md) | Authoritative schema epoch and reset | Accepted | Current | CPR-2, CPR-43 | — |
| [ADR-0070](adr-0070-generic-governed-scopes.md) | Generic governed scopes | Accepted | Current | CPR-3 | — |
| [ADR-0071](adr-0071-workspaces-projects-and-repository-identity.md) | Workspaces, projects and repositories | Accepted | Current | CPR-4 | — |
| [ADR-0072](adr-0072-groups-grants-and-invitations.md) | Groups, grants and invitations | Accepted | Current | CPR-5 | — |
| [ADR-0073](adr-0073-governed-scope-anchors.md) | Governed scope anchors | Accepted | Current | CPR-6 | — |
| [ADR-0074](adr-0074-hierarchy-cutover.md) | One scope tree and grant bootstrap | Accepted | Current | CPR-7 | Replaces ADR-0011, ADR-0015 and ADR-0016. |
| [ADR-0075](adr-0075-console-product-shell.md) | Routed console product shell | Accepted | Current | CPR-8 | — |
| [ADR-0076](adr-0076-sessions-as-runtime-aggregate.md) | Sessions as runtime aggregate | Accepted | Current | CPR-10 | — |
| [ADR-0077](adr-0077-session-product-surface.md) | Session product surface | Accepted | Current | CPR-11 | — |
| [ADR-0078](adr-0078-durable-session-delivery.md) | Durable session delivery and route cutover | Accepted; amended by CPR-14 and CPR-42 | Current | CPR-12, CPR-42 | Replaces the observe/inject/recall transport surfaces while retaining bounded delivery doctrine. |
| [ADR-0079](adr-0079-live-claude-session-acceptance.md) | Claude lifecycle evidence tiers | Accepted | Current | CPR-14 | — |
| [ADR-0080](adr-0080-versioned-knowledge-aggregate.md) | Immutable versioned Knowledge | Accepted | Current | CPR-15 | Replaces the Record aggregate as the learned-context domain. |
| [ADR-0081](adr-0081-governed-knowledge-lifecycle.md) | VedaFlow-governed Knowledge changes | Accepted | Current | CPR-16 | — |
| [ADR-0082](adr-0082-public-knowledge-surface.md) | Public immutable Knowledge API | Accepted | Current | CPR-17 | — |
| [ADR-0083](adr-0083-session-capture-candidates.md) | Session capture candidates | Accepted | Current | CPR-18 | Replaces direct extraction publication and the Record writer. |
| [ADR-0084](adr-0084-explainable-knowledge-context-planning.md) | Explainable Knowledge context planning | Accepted | Current | CPR-20 | Replaces the Record-era composer (ADR-0025). |
| [ADR-0085](adr-0085-versioned-skill-catalogue.md) | Immutable governed Skill catalogue | Accepted | Current | CPR-23 | Replaces ADR-0051, ADR-0053 and ADR-0054; retains ADR-0052's scan gate. |
| [ADR-0086](adr-0086-trusted-mcp-catalogue.md) | Trusted immutable MCP catalogue | Accepted | Current | CPR-25 | — |
| [ADR-0087](adr-0087-okf-v0-2-exchange-boundary.md) | Bounded OKF v0.2 exchange | Accepted | Current | CPR-27 | — |
| [ADR-0088](adr-0088-public-contract-and-client-boundary.md) | Executable route inventory and public clients | Accepted | Current | CPR-29 | — |
| [ADR-0089](adr-0089-governed-runtime-configuration.md) | Governed immutable runtime Configuration | Accepted | Current | CPR-30 | Replaces mutable defaults, assignments and runtime profile branches. |
| [ADR-0090](adr-0090-governed-policy-relaxations.md) | Governed policy relaxations | Accepted | Current | CPR-31 | Replaces ADR-0037's lapse table and worker. |
| [ADR-0091](adr-0091-unified-artifact-approvals.md) | Unified typed artifact approval | Accepted | Current | CPR-32 | Partially supersedes ADR-0032's recorded-review decision. |
| [ADR-0092](adr-0092-context-platform-audit-export.md) | Context-platform audit query and export | Accepted | Current | CPR-33 | Extends ADR-0019 and ADR-0045. |
| [ADR-0093](adr-0093-directory-adapter-convergence.md) | Converged directory projection | Accepted | Current | CPR-34 | Partially supersedes ADR-0059 and ADR-0060. |
| [ADR-0094](adr-0094-context-platform-key-and-secret-plane.md) | Stable secret identities and rotation | Accepted | Current | CPR-35 | Extends and partially supersedes ADR-0064. |
| [ADR-0095](adr-0095-one-runtime-deployment-convergence.md) | One runtime across deployment shapes | Accepted | Current | CPR-36 | Partially supersedes ADR-0055 and ADR-0062. |
| [ADR-0096](adr-0096-conflict-freshness-and-temporal-knowledge.md) | Knowledge conflicts and freshness | Accepted | Current | CPR-37 | Replaces ADR-0039 and ADR-0040. |
| [ADR-0097](adr-0097-bounded-knowledge-graph-retrieval.md) | Bounded Knowledge graph retrieval | Accepted | Current | CPR-38, GRPH-3 | Replaces ADR-0043 and ADR-0044 and removes the remaining Record graph. |
| [ADR-0098](adr-0098-client-adapter-conformance.md) | Evidence-based client support | Accepted | Current | CPR-39 | — |
| [ADR-0099](adr-0099-context-platform-product-evaluation.md) | Product delivery, use and trust evaluation | Accepted | Current | CPR-40 | Incorporates the earlier evaluation gates under one outcome model. |
| [ADR-0100](adr-0100-public-api-pulseboard-demo.md) | Resumable public-API demo | Accepted | Current | CPR-41 | Partially supersedes ADR-0066's demo shape. |
| [ADR-0101](adr-0101-production-hardening-boundary.md) | Production-hardening boundary | Accepted | Current | CPR-44 | — |
| [ADR-0102](adr-0102-portable-reference-deployment.md) | Portable reference deployment contract | Accepted | Current (implementation open) | CPR-45 | Compose is the canonical single-host reference; Keycloak replaces Rauthy, workers separate and optional Apalis remains an adapter. Its clean-Engine amendment adds a fixed fake-only actor/start/settlement and private-root ownership boundary while live provider execution and exact cleanup remain open. |

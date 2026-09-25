# Architecture decision record index

Use current decisions for new work. This index identifies whether each record
still applies and points to its replacement where needed. Each ADR header owns
its formal status and feature mapping. The [technical architecture](../SYNVEDA_TECH_PLAN.md)
provides the overview; retired decision history remains in Git. Records are not
renumbered.

## ADR-0001 through ADR-0034

| ADR | Decision | Current classification | Replacement/removal |
| --- | --- | --- | --- |
| [ADR-0001](adr-0001-postgres-first-rust-stack.md) | Postgres-first Rust stack | Current | — |
| [ADR-0002](adr-0002-cedar-embedded-pdp.md) | Embedded Cedar PDP | Current | The application facade is elaborated by ADR-0012. |
| [ADR-0003](adr-0003-vedaflow-in-postgres.md) | VedaFlow in Postgres | Current | Object storage is ADR-0030; typed review is ADR-0091. |
| ADR-0004 → [replacement](adr-0097-bounded-knowledge-graph-retrieval.md) | Named Apache AGE graphs | Removed; rationale in ADR-0097 | ADR-0097 defines bounded `KnowledgeRelation` expansion without AGE. |
| [ADR-0005](adr-0005-uuidv7-identifiers.md) | UUIDv7 domain identifiers | Current | — |
| [ADR-0006](adr-0006-bitemporal-tables.md) | Bitemporal current/history tables | Current | Knowledge history and temporal queries are refined by ADR-0080 and ADR-0096. |
| [ADR-0007](adr-0007-observability-baseline.md) | Tracing, OpenTelemetry and metrics baseline | Current | — |
| [ADR-0008](adr-0008-tenant-resolution.md) | Token-derived tenant context | Current | — |
| [ADR-0009](adr-0009-rls-tenant-backstop.md) | Forced-RLS tenant backstop | Current | — |
| [ADR-0010](adr-0010-oidc-login.md) | OIDC code and PKCE login | Current | — |
| [ADR-0011](adr-0011-hierarchy-store.md) | Fixed hierarchy closure store | Removed with hierarchy store (ADR-0074/CPR-7) | ADR-0070 and ADR-0074 replace it with the governed scope tree. |
| [ADR-0012](adr-0012-cedar-pdp-embedded.md) | Cedar facade and policy-pack store | Current | Resource construction is re-cut over governed anchors by ADR-0073. |
| [ADR-0013](adr-0013-jit-provisioning.md) | JIT identity provisioning | Current (partially superseded by ADR-0074 and ADR-0093) | Mapping/quarantine placement was removed; principal scopes, directory adoption and the administrator grant remain. |
| [ADR-0014](adr-0014-policy-packs.md) | Policy packs and composition | Current (partially superseded by ADR-0089) | Per-node assignment was replaced by governed Configuration selection. |
| [ADR-0015](adr-0015-roles-role-bindings.md) | Legacy roles and role bindings | Removed with role bindings (ADR-0074/CPR-7) | ADR-0072 scope grants and the closed `RoleKey` vocabulary replace it. |
| [ADR-0016](adr-0016-scope-chain-resolver.md) | Legacy scope-chain cache | Removed with scope-chain cache (ADR-0074/CPR-7) | Governed ancestors resolve per request; only PDP entity invalidation remains. |
| [ADR-0017](adr-0017-cedar-entity-sync.md) | Cedar entity fragments | Current (partially superseded by ADR-0074) | Entity fragments remain; governed scope chains replaced the legacy cache feed. |
| [ADR-0018](adr-0018-service-identities.md) | Service identities and confinement | Current (partially superseded by ADR-0074) | Service placement now uses principal scopes and scope grants. |
| [ADR-0019](adr-0019-hash-chained-audit-log.md) | Hash-chained audit log | Current | Query/export are extended by ADR-0045 and ADR-0092; ADR-0064 amendment 3 adds the narrow repairable key-provision witness. |
| [ADR-0020](adr-0020-observe-ingestion.md) | Idempotent event ingestion | Current (partially superseded by ADR-0078) | `/v1/observe`, PGMQ and the old buffer were removed; session events and the durable adapter spool preserve the delivery doctrine. |
| [ADR-0021](adr-0021-redaction-secret-scanning.md) | Admission redaction and secret scanning | Current | Capture and skill admission use the surviving fail-closed scanning rules. |
| [ADR-0022](adr-0022-extraction-pipeline.md) | Extraction pipeline | Current (partially superseded by ADR-0083) | Database-leased capture batches and reviewable candidates replace PGMQ and direct Record writes. |
| [ADR-0023](adr-0023-transactional-embed-or-fail.md) | Transactional embedding invariant | Superseded by ADR-0080 and ADR-0082 | Governance commits publish immutable revisions without waiting for embeddings; a bounded, idempotent worker converges the revision sidecar. |
| [ADR-0024](adr-0024-hybrid-retrieval.md) | Authorised lexical/vector fusion | Current (partially superseded by ADR-0084) | Knowledge retrieval keeps authorised fusion; the Record corpus and Tantivy sidecar were removed. |
| [ADR-0025](adr-0025-composition-engine.md) | Record-era context composition | Superseded by ADR-0084 | ADR-0084 owns immutable Knowledge planning and trace disclosure. |
| [ADR-0026](adr-0026-inject-api.md) | Injection degradation ladder | Current (partially superseded by ADR-0078) | `/v1/inject` was removed; bounded context-run degradation remains. |
| [ADR-0027](adr-0027-claude-code-adapter.md) | Claude Code adapter | Current | Lifecycle evidence is ADR-0079; support claims are governed by ADR-0098. Ordinary exit retains native resume. |
| [ADR-0028](adr-0028-eval-harness.md) | Unprivileged evaluation harness | Current | Product outcome methodology is extended by ADR-0099. |
| ADR-0029 → [replacement](adr-0097-bounded-knowledge-graph-retrieval.md) | Apache AGE adoption gate | Removed; budget and fallback in ADR-0097 | ADR-0097 carries the surviving explicit-bound and fallback requirements. |
| [ADR-0030](adr-0030-vedaflow-object-store.md) | VedaFlow content-addressed object store | Current | — |
| [ADR-0031](adr-0031-vedaflow-channels.md) | VedaFlow channels and derived publication | Current | — |
| [ADR-0032](adr-0032-vedaflow-proposals-approval-matrix.md) | VedaFlow proposal approval matrix | Current (partially superseded by ADR-0091) | ADR-0091 replaces recorded proposal review decision 7 with one typed artifact lifecycle. |
| [ADR-0033](adr-0033-auto-promotion-rules.md) | Record-usage auto-promotion sweeper | Removed with Record promotion pipeline (ADR-0083/CPR-18) | The audit projection, Record usage rules and automatic promotion worker have no current successor; typed auto-apply is a separate ADR-0091 path. |
| [ADR-0034](adr-0034-cross-scope-promotion.md) | Governed cross-scope promotion | Current | — |

## ADR-0035 through ADR-0067

| ADR | Decision | Current classification | Replacement/removal |
| --- | --- | --- | --- |
| [ADR-0035](adr-0035-cli-review-flow.md) | Governed CLI review | Current | ADR-0091 makes the review surface artifact-neutral. |
| [ADR-0036](adr-0036-rollback-and-pinning.md) | VedaFlow rollback and pinning | Current | — |
| [ADR-0037](adr-0037-lapses.md) | Legacy policy lapses | Superseded by ADR-0090 | Immutable, expiring policy relaxations replace the lapse table and worker. |
| [ADR-0038](adr-0038-abac-conditions.md) | Closed ABAC conditions and restricted floor | Current | ADR-0090 adds the governed restricted-tier exception without weakening the floor. |
| [ADR-0039](adr-0039-dedup-and-conflict-detection.md) | Record deduplication and conflicts | Superseded by ADR-0096 | Durable Knowledge conflict sets and typed resolution replace Record-era judging. |
| [ADR-0040](adr-0040-decay-ttl-and-staleness.md) | Record decay and staleness | Superseded by ADR-0096 | Governed, version-evidenced Knowledge freshness replaces pack-time Record decay. |
| [ADR-0041](adr-0041-tiered-injection.md) | Tiered context rendering | Current (partially superseded by ADR-0084) | ContextRun selection and authored-context composition replace the inject-specific surface. |
| [ADR-0042](adr-0042-recall-api-and-mcp-tool.md) | Scoped recall query | Current (partially superseded by ADR-0078) | `/v1/recall` was removed; Knowledge/context queries and the generic MCP boundary carry the surviving query doctrine. |
| ADR-0043 → [replacement](adr-0097-bounded-knowledge-graph-retrieval.md) | Record adjacency graph | Removed; replaced by ADR-0097 | Bounded `KnowledgeRelation` traversal replaces the Record graph. |
| ADR-0044 → [replacement](adr-0097-bounded-knowledge-graph-retrieval.md) | Record extraction graph linking | Removed; candidate/graph boundary in ADR-0097 | Explicit immutable Knowledge relations replace extractor-written graph edges. |
| [ADR-0045](adr-0045-audit-query-surface.md) | Tenant-complete audit query | Current | ADR-0092 adds typed context-platform evidence and frozen-head export. |
| [ADR-0046](adr-0046-extraction-quality-suite.md) | Extraction quality gate | Current | ADR-0099 incorporates it into the product outcome suite. |
| [ADR-0047](adr-0047-retrieval-and-injection-quality.md) | Retrieval and context quality gate | Current (partially superseded by ADR-0099) | ContextRun delivery/use outcomes replace the deleted injection lens. |
| [ADR-0048](adr-0048-security-evals.md) | Zero-tolerance security evaluation | Current | ADR-0099 retains these as explicit trust gates. |
| [ADR-0049](adr-0049-prompt-registry.md) | Governed prompt registry | Current | — |
| [ADR-0050](adr-0050-context-packs.md) | Governed context packs | Current (storage/composition amended by CPR-43) | Immutable `context_pack_chunks` and VedaFlow publication remain; chunks are authored ContextRun inputs, not Records or Knowledge-index entries. Configuration selects the effective authored-context policy (ADR-0089). |
| [ADR-0051](adr-0051-skills-registry.md) | Mutable Skill registry | Superseded by ADR-0085 | Stable Skill aggregates and immutable versions replace mutable drafts. |
| [ADR-0052](adr-0052-skill-security-scanning-gate.md) | Whole-bundle Skill security scan | Current | ADR-0085 re-anchors the unchanged gate on immutable Skill versions. |
| [ADR-0053](adr-0053-skill-quality-scoring.md) | Legacy Skill quality scoring | Superseded by ADR-0085 | Version-bound scan/rubric evidence replaces checklist overrides. |
| [ADR-0054](adr-0054-skill-distribution.md) | Legacy Skill distribution set | Superseded by ADR-0085 | Revisioned principal/project bindings replace channel-backed materialisation. |
| [ADR-0055](adr-0055-smb-profile-and-init.md) | Small-team installation and init | Superseded by ADR-0102 | Canonical Compose owns bootstrap; the ADR-0055 contributor Rauthy profile, host gateway and lifecycle implementation are deleted, while `synveda init` is a reserved refusal. |
| [ADR-0056](adr-0056-admin-console-shell.md) | Console without a second authority | Current | Product routing and generated-client boundaries are extended by ADR-0075 and ADR-0088; ADR-0102 adds a distinct explicit-development HTTP cookie mode without changing HTTPS. |
| [ADR-0057](adr-0057-generic-mcp-server.md) | Generic MCP adapter boundary | Current | Evidence-based client support is ADR-0098. |
| [ADR-0058](adr-0058-hierarchy-and-policy-explorer.md) | PDP-backed scope and policy explorer | Current (partially superseded by ADR-0074 and ADR-0075) | Governed scopes replace the fixed hierarchy; forecasts remain non-authoritative. |
| [ADR-0059](adr-0059-scim-directory-sync.md) | SCIM directory projection | Current (partially superseded by ADR-0093) | ADR-0093 replaces the separate SCIM group graph with shared identities, groups and grants. |
| [ADR-0060](adr-0060-directory-pull-sync.md) | Safe directory pull reconciliation | Current (partially superseded by ADR-0093) | ADR-0093 converges pull and SCIM onto one projection and secret boundary. |
| [ADR-0061](adr-0061-public-benchmark-adapters.md) | Governed LongMemEval adapter | Current | ADR-0099 separates delivery, use and outcome signals. |
| [ADR-0062](adr-0062-enterprise-profile-and-helm-chart.md) | Enterprise Helm deployment | Current (partially superseded by ADR-0095 and ADR-0102) | Deployment shapes no longer select product behaviour; governed Configuration does. Compose is now the reference contract and Helm maps it with Kubernetes-native primitives. |
| [ADR-0063](adr-0063-tenant-partitioned-storage.md) | Tenant storage partitioning decision | Current (partially superseded by ADR-0080) | Record-specific benchmark evidence is historical; the unpartitioned pgvector decision remains current for Knowledge. |
| [ADR-0064](adr-0064-per-tenant-envelope-keys.md) | Per-tenant envelope encryption | Current (partially superseded by ADR-0094) | ADR-0094 adds stable secret identities and durable envelope rotation; amendment 3 defines repairable key-provision evidence. |
| [ADR-0065](adr-0065-release-and-distribution.md) | Release packaging and distribution | Current (partially superseded by ADR-0102) | Native/plugin/chart packaging remains; private Node and Unix/Windows client archives extend distribution without server downloads. Windows uses protected storage and PowerShell installation. Exact native plugin scope is preserved. The digest-bound reference replaces installed-profile mechanics. |
| [ADR-0066](adr-0066-beta-demo-profile.md) | Operator-seeded beta demo | Current (partially superseded by ADR-0100) | ADR-0100 provides the resumable public-API PulseBoard demo; externally dependent beta evidence remains open. |
| [ADR-0067](adr-0067-uninstall-and-cleanup.md) | Uninstall and cleanup | Current (partially superseded by ADR-0102) | Canonical Compose owns lifecycle and reset. Automatic artifact removal remains fail-closed until the installer persists a strict ownership receipt. |

## ADR-0068 through ADR-0117

| ADR | Decision | Current classification | Replacement/removal |
| --- | --- | --- | --- |
| [ADR-0068](adr-0068-context-platform-domain-and-epoch.md) | Context-platform domain and epoch | Current | — |
| [ADR-0069](adr-0069-schema-epoch-and-local-reset.md) | Authoritative schema epoch and reset | Current | — |
| [ADR-0070](adr-0070-generic-governed-scopes.md) | Generic governed scopes | Current | — |
| [ADR-0071](adr-0071-workspaces-projects-and-repository-identity.md) | Workspaces, projects and repositories | Current | Non-default ports are refused until a data-preserving schema change can represent them. |
| [ADR-0072](adr-0072-groups-grants-and-invitations.md) | Groups, grants and invitations | Current | — |
| [ADR-0073](adr-0073-governed-scope-anchors.md) | Governed scope anchors | Current | — |
| [ADR-0074](adr-0074-hierarchy-cutover.md) | One scope tree and grant bootstrap | Current | Replaces ADR-0011, ADR-0015 and ADR-0016. |
| [ADR-0075](adr-0075-console-product-shell.md) | Routed console product shell | Current | — |
| [ADR-0076](adr-0076-sessions-as-runtime-aggregate.md) | Sessions as runtime aggregate | Current | — |
| [ADR-0077](adr-0077-session-product-surface.md) | Session product surface | Current | — |
| [ADR-0078](adr-0078-durable-session-delivery.md) | Durable session delivery and route cutover | Current | Replaces the observe/inject/recall transport surfaces while retaining bounded delivery and resumable native bindings. |
| [ADR-0079](adr-0079-live-claude-session-acceptance.md) | Claude lifecycle evidence tiers | Current | — |
| [ADR-0080](adr-0080-versioned-knowledge-aggregate.md) | Immutable versioned Knowledge | Current | Replaces the Record aggregate as the learned-context domain. |
| [ADR-0081](adr-0081-governed-knowledge-lifecycle.md) | VedaFlow-governed Knowledge changes | Current | — |
| [ADR-0082](adr-0082-public-knowledge-surface.md) | Public immutable Knowledge API | Current | — |
| [ADR-0083](adr-0083-session-capture-candidates.md) | Session capture candidates | Current | Replaces direct extraction publication and the Record writer. |
| [ADR-0084](adr-0084-explainable-knowledge-context-planning.md) | Explainable Knowledge context planning | Current | Replaces the Record-era composer (ADR-0025). |
| [ADR-0085](adr-0085-versioned-skill-catalogue.md) | Immutable governed Skill catalogue | Current | Replaces ADR-0051, ADR-0053 and ADR-0054; retains ADR-0052's scan gate. |
| [ADR-0086](adr-0086-trusted-mcp-catalogue.md) | Trusted immutable MCP catalogue | Current | — |
| [ADR-0087](adr-0087-okf-v0-2-exchange-boundary.md) | Bounded OKF v0.2 exchange | Current | — |
| [ADR-0088](adr-0088-public-contract-and-client-boundary.md) | Executable route inventory and public clients | Current | — |
| [ADR-0089](adr-0089-governed-runtime-configuration.md) | Governed immutable runtime Configuration | Current | Replaces mutable defaults, assignments and runtime profile branches. |
| [ADR-0090](adr-0090-governed-policy-relaxations.md) | Governed policy relaxations | Current | Replaces ADR-0037's lapse table and worker. |
| [ADR-0091](adr-0091-unified-artifact-approvals.md) | Unified typed artifact approval | Current | Partially supersedes ADR-0032's recorded-review decision. |
| [ADR-0092](adr-0092-context-platform-audit-export.md) | Context-platform audit query and export | Current | Extends ADR-0019 and ADR-0045. |
| [ADR-0093](adr-0093-directory-adapter-convergence.md) | Converged directory projection | Current | Partially supersedes ADR-0059 and ADR-0060. |
| [ADR-0094](adr-0094-context-platform-key-and-secret-plane.md) | Stable secret identities and rotation | Current | Extends and partially supersedes ADR-0064. |
| [ADR-0095](adr-0095-one-runtime-deployment-convergence.md) | One runtime across deployment shapes | Current | Partially supersedes ADR-0055 and ADR-0062. |
| [ADR-0096](adr-0096-conflict-freshness-and-temporal-knowledge.md) | Knowledge conflicts and freshness | Current | Replaces ADR-0039 and ADR-0040. |
| [ADR-0097](adr-0097-bounded-knowledge-graph-retrieval.md) | Bounded Knowledge graph retrieval | Current | Replaces ADR-0043 and ADR-0044 and removes the remaining Record graph. |
| [ADR-0098](adr-0098-client-adapter-conformance.md) | Evidence-based client support | Current | — |
| [ADR-0099](adr-0099-context-platform-product-evaluation.md) | Product delivery, use and trust evaluation | Current | Incorporates the earlier evaluation gates under one outcome model. |
| [ADR-0100](adr-0100-public-api-pulseboard-demo.md) | Resumable public-API demo | Current | Partially supersedes ADR-0066's demo shape. |
| [ADR-0101](adr-0101-production-hardening-boundary.md) | Production-hardening boundary | Current | — |
| [ADR-0102](adr-0102-portable-reference-deployment.md) | Portable reference deployment contract | Current (live validation pending) | Compose is the canonical single-host reference; Keycloak replaces Rauthy, workers are separate and optional Apalis remains a leaf adapter. ADR-0105 replaces provider simulation with direct Compose acceptance; OPS-12 derives a candidate consumer graph with private named-volume initialization and paired logical recovery. Confirmed hosts installation can renew a verified device-only witness change. |
| ADR-0103 → [replacement](adr-0105-direct-compose-acceptance.md) | Cooperative aggregate live-provider reservation | Removed; replaced by ADR-0105 | The non-executing provider-reservation fixture was deleted; a supported Docker engine is an operator-owned prerequisite. |
| ADR-0104 → [replacement](adr-0105-direct-compose-acceptance.md) | Indivisible live-provider effect generation | Removed; replaced by ADR-0105 | The fixture-only provider-effect grammar was deleted; acceptance now exercises the canonical Compose graph directly. |
| [ADR-0105](adr-0105-direct-compose-acceptance.md) | Direct Docker Compose acceptance | Current | Supersedes ADR-0103/0104 and removes container-engine simulation. Logical recovery, Operations and the Apalis canary are implemented; live Docker/upgrade/external acceptance remains, while S3/WAL-PITR stays with OPS-5. |
| [ADR-0106](adr-0106-authenticated-client-task-boundary.md) | Explicit task identity and public client interoperability | Current | Amends ADR-0057's launch identity; preserves public API enforcement and ADR-0098 evidence gates. SDK build metadata and repository Apache-2.0 licence/notice are checked through installed packages; runtime test evidence is separate from public support policy. |
| [ADR-0107](adr-0107-copilot-context-adapter.md) | Copilot context and observations | Current | Native CLI 1.0.83 start/resume and the shared SDK task are verified for the named source-build setup. Text/tool events reuse the bounded reader and spool; runtime exit retains task identity. ADR-0065 packages the existing runtime. |
| [ADR-0108](adr-0108-ci-gate-ownership-and-build-caching.md) | CI gate ownership and Kind build caching | Current | CI Result permits only declared skips; shared native CLI/Docker validation retains full candidate acceptance. Tag publication requires exact-source full CI, retained candidate reports and native anonymous distribution checks. |
| [ADR-0109](adr-0109-small-team-kubernetes-release.md) | Small-team Kubernetes release boundary | Current (portable increment implemented) | External PostgreSQL/OIDC, verified TLS and file-mounted Secrets extend the existing chart; single application replicas and optional preinstalled CNPG remain. Starter/platform/release qualification is still open. |
| [ADR-0110](adr-0110-persistent-starter-and-team-admission.md) | Persistent starter and explicit team admission | Current (implemented) | Locked optional Keycloak, retained existing CNPG and disjoint issuer admission; four ownership combinations passed on Kind. No new operator or role model. |
| [ADR-0111](adr-0111-restricted-kubernetes-portability.md) | Restricted Kubernetes portability | Current | One chart, assigned IDs, v3 post-renderer, edge Routes and opt-in NetworkPolicies; live platform qualification remains separate. |
| [ADR-0112](adr-0112-small-team-operational-evidence.md) | Small-team operational evidence and recovery | Current | Native logical recovery, existing-job interruption, immutable Helm image overlays and explicit publication/upgrade evidence boundaries. |
| [ADR-0113](adr-0113-static-public-site-and-brand.md) | Static public site and canonical brand | Current | Public HTML/CSS and derived assets are isolated from the product runtime; a pinned WASM renderer and both Linux architectures protect exact export checks. |
| [ADR-0114](adr-0114-console-brand-and-navigation.md) | Console branding and task navigation | Current | Shared canonical assets, responsive navigation and existing-project onboarding retain the public API and authority model. |
| [ADR-0115](adr-0115-prebuilt-container-release-verification.md) | Prebuilt container release verification | Current (hosted verification pending) | Native anonymous pulls gate release announcements; full candidate drills precede immutable two-registry copying and public chart parity. |
| [ADR-0116](adr-0116-native-consumer-commands.md) | Native consumer lifecycle and setup | Current (candidate only) | Native commands retain Compose/API/installer authority with private ownership receipts, project observation consent and conflict-safe removal. |
| [ADR-0117](adr-0117-client-platform-boundaries.md) | Client platform and private-state boundaries | Current (private-storage candidate) | CLI/hooks share path fixtures and Windows credential/receipt/spool ACL and identity checks. A bounded local CLI protocol serves Windows hooks. Repository/vendor edits and deployment witnesses still refuse. |
| [ADR-0118](adr-0118-governed-context-optimisation.md) | Governed context optimisation inside ContextRun | Current (rollout evidence pending) | CTX-8; off remains the comparison path while Compose and held-out quality gates remain open. |
| [ADR-0119](adr-0119-session-checkpoints-from-verified-ledger-evidence.md) | Checkpoints derived from immutable Session evidence | Current (rollout evidence pending) | CTX-6; deterministic Claude compact/restart replay, capture, scoped retention, Session policy matrix and four synthetic restart task probes pass; live task quality remains open. |

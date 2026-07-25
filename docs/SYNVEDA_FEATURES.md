# SYNVEDA — Research Digest & Feature Backlog v1 (July 2026)

Companion to SYNVEDA_SEED.md and SYNVEDA_TECH_PLAN.md.
Part A: what the 2026 research and ecosystem actually says, and what it changes in our design.
Part B: the plan decomposed into runnable features with acceptance criteria.

═══════════════════════════════════════════════════════════════════
PART A — RESEARCH DIGEST (state of the art, mid-2026)
═══════════════════════════════════════════════════════════════════

## A1. Memory architectures

**Temporal knowledge graphs won the argument.** Zep's Graphiti (arXiv 2501.13956) is the
canonical production system: every fact/edge carries a validity window (valid_at/invalid_at)
plus ingestion time — bitemporal — so new facts supersede old ones without losing history.
Results: 94.8% DMR (vs MemGPT 93.4%), ~63.8% LongMemEval vs Mem0's 49% — a 15-point gap
attributed directly to temporal fact modelling over flat vector storage. Graphiti crossed
20k+ GitHub stars in 2026; retrieval is hybrid (embeddings + BM25 + graph traversal) with
**no LLM calls at retrieval time** — that's how they hit ~300ms P95.
→ **Validates Synveda's bitemporal records + hybrid retrieval + no-LLM-on-read-path design.
   We must put validity windows on graph edges, not just records.**

**Multi-graph is the research frontier.** MAGMA (arXiv 2601.03236) tops LoCoMo (judge 0.7 vs
A-MEM 0.58, MemoryOS 0.553) by maintaining multiple specialised graphs (semantic, episodic,
causal, entity) rather than one homogeneous graph. 2026 surveys taxonomise graph memory into
knowledge/temporal/hyper/hierarchical/hybrid variants.
→ **Design AGE schema as multiple named graphs per tenant from day one (entity graph,
   episode graph, provenance graph) — cheap now, expensive to retrofit.**

**Token efficiency is now a first-class benchmark axis.** Mem0's 2026 state-of-memory report
scores systems on accuracy × latency × tokens-per-query; their new algorithm reports 92.5
LoCoMo / 94.4 LongMemEval at ~6.9k tokens/query, and the field explicitly calls out that
"accurate but 26k tokens/query" is not production-viable. Recent pipelines (semantic
compression papers) report 30× inference-token reductions via write-time compression +
intent-aware retrieval scope planning.
→ **Confirms summarise-at-write + budgeted injection. Add tokens-per-inject as a tracked
   SLO metric and eval dimension.**

**Known failure modes to design against** (from Mem0/mem-framework field reports):
silent memory loss on partial batch-embedding failure; dedup that only runs when a flag is
set; ADD-only stores that rely entirely on retrieval-time ranking for conflict resolution;
memory staleness; cross-session identity resolution. Hardest open problems per the 2026
reports: cross-session identity, temporal abstraction at scale, staleness.
→ **Features below include: transactional embed-or-fail ingestion, always-on dedup,
   explicit conflict/supersession records, staleness/decay jobs, identity stitching.**

**Benchmarks that matter:** LoCoMo (Snap; 35 sessions/300 turns; single-hop, multi-hop,
temporal, open-domain), LongMemEval (~115k-token histories; 10 categories incl. knowledge
update, temporal reasoning, abstention, contradiction resolution), DMR (saturated, legacy),
HaluMem (hallucinated-memory detection). Eval frameworks score five dimensions jointly:
accuracy, latency, tokens, recall, abstention.
→ **Synveda ships an eval harness that runs LoCoMo + LongMemEval against the whole pipeline,
   plus enterprise-specific evals (policy-leak tests, cross-tenant isolation tests).**

## A2. Context engineering

The 2026 consensus stack (Anthropic's framing: "smallest set of high-signal tokens"):
**progressive disclosure** for what loads when; **compression** (hybrid sliding window: last
N turns raw, older summarised); **routing** to the right source; **tool management** because
MCP tool schemas alone can eat 24%+ of a context window (MCP added Tool Search in Jan 2026 to
lazy-load tool catalogs). Layered, not alternatives.
→ **Synveda's inject is precisely the progressive-disclosure layer for org knowledge.
   Feature: tiered injection — index/metadata tier always, bodies on demand via recall,
   mirroring the skills model. Also: expose Synveda itself through ONE recall tool, never a
   tool catalog.**

## A3. Skills — the ecosystem shift that changes our product

Agent Skills went from Anthropic feature (Oct 2025) to **open standard (agentskills.io, 18
Dec 2025)** to ecosystem default: ~40 client platforms including GitHub Copilot, Cursor,
OpenAI Codex, Gemini CLI; anthropics/skills at ~149k stars by June 2026. MCP itself was
donated to the Linux Foundation (Dec 2025). Skills use 3-level progressive disclosure
(metadata ~80 tokens → SKILL.md body 275–8,000 tokens → bundled files on demand).

Enterprise motion validates Synveda's thesis directly: Anthropic added **org-wide skill
administration** (central provisioning, default-enable) for Team/Enterprise in Feb 2026 —
"companies want tailored workflows with audit trails and clean separation between teams."
Meanwhile the open catalogs have a quality/security crisis: SkillsBench scores the average
public skill 6.2/12, two-million-skill scraped catalogs are "mostly noise," and unaudited
third-party skills are recognised as real supply-chain risk. Curated, reviewed, versioned
skill registries with rollback are explicitly the emerging battleground (registries,
security scanning, quality scores "the way buyers demand security scores").
→ **Synveda's skills registry must be agentskills.io-spec-compliant (portable to 40+
   clients), and VedaFlow review + security-scan + provenance is exactly the governance
   layer the ecosystem lacks. This is the sharpest wedge in the whole product. Add: skill
   quality scoring (SkillsBench-style) and static security scanning as pipeline gates.**

## A4. Storage & relationships

**pgvector vs Qdrant, 2026 verdict:** pgvector with HNSW is fine to ~10M vectors and often
"fast enough" (5–8ms at moderate scale) — the standard guidance is "pgvector if you have
Postgres, Qdrant if you don't." The sharp edge: **pgvector metadata filtering is post-filter
on the candidate set, not inside the HNSW graph** — low-selectivity filters (e.g. tenant or
scope filters!) degrade badly; Qdrant's filtered search is best-in-class and it's Rust,
Apache-2.0, single binary, with scalar/product quantisation for memory-constrained deploys.
→ **Our queries are ALWAYS filtered (tenant + scope + sensitivity). Mitigations in features:
   partial HNSW indexes per tenant partition, partition-by-tenant tables, and the Qdrant
   adapter promoted from "someday" to Phase 3 with a benchmark gate deciding default at
   ~5M vectors/tenant. Also evaluate pg_diskann-style disk indexes for large tenants.**

**Apache AGE**: active Apache top-level project; the pgvector+AGE single-engine pattern
(vectors + openCypher graph in one Postgres, no sync pipelines, "no multi-database tax") is
now an explicitly promoted architecture (e.g. Microsoft Azure PostgreSQL guidance, 2026).
Kuzu is the notable embedded-graph alternative appearing in memory stacks.
→ **AGE choice holds. Keep graph features additive/degradable. Kuzu noted as embedded
   fallback if AGE Cypher perf disappoints (spike in Phase 2).**

**AuthZ**: MCP only got "its missing enterprise authorization layer" in mid-2026 —
authorization for agent access is immature everywhere, which again is our gap to own. Cedar
(Rust, in-process, formally verified) for policy + entity hierarchy; OpenFGA adapter path
for extreme ReBAC depth. No change from tech plan; conviction increased.

═══════════════════════════════════════════════════════════════════
PART B — FEATURE BACKLOG
═══════════════════════════════════════════════════════════════════

Legend: each feature is independently runnable/demoable. Size: S (≤2 days), M (≤1 wk),
L (≤2 wks). "AC" = acceptance criteria. Dependencies by ID. IDs are stable — use them in
issues/branches (e.g. `feat/AUTH-3`).

──────────────────────────────────────────────
EPIC FND — Foundation
──────────────────────────────────────────────
FND-1  Workspace scaffold (S)
  Rust workspace per tech plan §8 + pnpm workspace; empty crates compile; CI: fmt,
  clippy -D warnings, test, deny (licence check). AC: `cargo build --workspace` green in CI.
FND-2  Dev environment (S)
  docker-compose: Postgres 17 + pgvector + AGE + PGMQ, Rauthy, Temporal, TEI (BGE-M3),
  Jaeger. AC: `make dev-up && make smoke` passes.
FND-3  synveda-types + error model (S)
  Tenant/Scope/Identity/RecordId newtypes, sensitivity enum, error taxonomy.
  AC: types crate has zero internal deps; serde round-trip tests.
FND-4  Migrations & bitemporal base tables (M)
  sqlx migrations; records with (tx_from, tx_to, valid_from, valid_to); triggers for
  tx-time maintenance. AC: as-of query returns historical row states; property tests.
FND-5  Observability baseline (S)
  OTel tracing through gateway→core→store; Prometheus metrics incl. tokens_per_inject.
  AC: single trace visible in Jaeger spanning an end-to-end request.
FND-6  ADRs 0001–0004 (S)
  Stack; Cedar-over-OPA; VedaFlow-in-Postgres; multi-graph AGE schema. AC: merged.

──────────────────────────────────────────────
EPIC TEN — Multi-tenancy (functional requirement)
──────────────────────────────────────────────
TEN-1  Tenant model & resolution (M)
  Tenant table, per-request resolution from token claims; tenant context propagated via
  tower middleware + task-local. AC: request without resolvable tenant → 401; traces
  carry tenant_id.
TEN-2  Postgres row-level security as backstop (M)
  RLS policies on every tenant-scoped table keyed to a session GUC set per connection.
  Defence-in-depth: app bug cannot cross tenants. AC: adversarial test suite — direct SQL
  with wrong tenant GUC returns zero rows on every table.
TEN-3  Tenant-partitioned storage layout (M)
  Declarative partitioning by tenant hash for records/embeddings; partial HNSW indexes per
  partition (mitigates pgvector post-filtering). AC: filtered ANN query plan shows partition
  pruning; benchmark vs unpartitioned recorded.
TEN-4  Per-tenant encryption keys (M)
  Envelope encryption; key ref per tenant; KMS trait (local dev impl + AWS/GCP/Vault impls
  later). AC: tenant export is unreadable without that tenant's key.
TEN-5  Tenant lifecycle (M)
  Create/suspend/export/delete workflows (Temporal); delete produces signed destruction
  certificate; export = portable archive (records+assets+audit). AC: GDPR-style erasure
  E2E test; export re-imports into a fresh instance.
TEN-6  Cross-tenant isolation test harness (M) [continuous]
  Fuzzing suite that attempts cross-tenant reads via API, recall, inject composition, and
  graph traversal. AC: runs in CI nightly; any leak fails the build. (This is also an
  evaluation deliverable — see EVAL-5.)

──────────────────────────────────────────────
EPIC AUTH — Authentication & identity (functional requirement)
──────────────────────────────────────────────
AUTH-1 OIDC login (code+PKCE) (M)
  Any compliant IdP; Rauthy bundled for dev/SMB. JWKS cache, rotation handling.
  AC: login via Rauthy and via a mock Entra config both yield a Synveda session.
AUTH-2 JIT user provisioning from claims (M)
  First login: map groups/claims → hierarchy nodes via mapping rules (convention defaults
  `synveda-{dept}-{team}`, override table). AC: new user lands in correct team scope with
  zero admin action; unmapped users land in quarantine scope with no read rights.
AUTH-3 Service identities (M)
  OAuth2 client-credentials; short-lived scoped tokens; every headless agent is an identity
  at a hierarchy node. AC: agent token with team scope cannot call org-scope endpoints.
AUTH-4 SCIM 2.0 server (L)
  Users+Groups endpoints; joiner/mover/leaver; leaver seals personal scope (retention-held,
  unreadable by default). AC: SCIM conformance tests; mover's memories re-scope per policy.
AUTH-5 Directory sync fallback (M)
  Scheduled pull sync (Temporal) for IdPs without SCIM push. AC: drift converges ≤ sync
  interval; deletions handled as leavers.
AUTH-6 Session & token hygiene (S)
  Refresh rotation, revocation list, device-bound options. AC: revoked token rejected ≤30s.

──────────────────────────────────────────────
EPIC AUTHZ — Authorisation & policy (functional requirement)
──────────────────────────────────────────────
AUTHZ-1 Cedar PDP embedded (M)
  `authorize(subject, action, resource, ctx)` facade; entities materialised from hierarchy;
  policy store per tenant, hot-reload. AC: µs-level decision benchmark; decision + policy
  version logged for every call.
AUTHZ-2 Policy packs (M)
  `regulated-strict` / `standard` / `open-collaboration` as versioned Cedar bundles applied
  per node; inheritance with override rules. AC: switching a team's pack changes inject
  composition in the next session; golden tests per pack.
AUTHZ-3 Roles & role bindings (M)
  viewer/contributor/curator/steward/org-admin/auditor/security-reviewer/compliance; bound
  per node, inherited downward. AC: full role×action matrix golden-tested.
AUTHZ-4 Lapses (controlled relaxation) (L)
  Lapse = time-boxed policy override proposal; reason mandatory; dual approval under
  regulated-strict; Temporal timer auto-revert; all transitions audited.
  AC: E2E — lapse grants cross-team read, expiry restores denial, audit shows full story.
AUTHZ-5 ABAC conditions (M)
  Sensitivity, residency, channel (published/derived), time-of-day, purpose-of-use as Cedar
  context. AC: `restricted` records never injected without compliance-granted permission,
  proven by leak-test suite.
AUTHZ-6 OpenFGA adapter spike (S) [de-risk]
  Prove the facade can back onto FGA for deep ReBAC; document trigger conditions for switch.
  AC: spike report + conformance test suite passing on both engines for the shared subset.

──────────────────────────────────────────────
EPIC HIER — Hierarchy & scopes
──────────────────────────────────────────────
HIER-1 Hierarchy store (M)
  Closure table + materialised path; org→division→department→team→user with configurable
  depth; CRUD via admin API. AC: 10k-node hierarchy; ancestor/descendant queries <1ms.
HIER-2 Scope chain resolver (S)
  Given identity → ordered scope chain for composition (user→…→org), cached with
  invalidation on hierarchy change. AC: cache invalidation test; p99 <0.5ms warm.
HIER-3 Cedar entity sync (M)
  Hierarchy changes stream into Cedar entity store transactionally. AC: move a team between
  departments → authz decisions reflect it in the same transaction boundary.

──────────────────────────────────────────────
EPIC MEM — Memory core (write path)
──────────────────────────────────────────────
MEM-1  observe API + PGMQ buffer (M)
  Batched transcript/event ingestion; ack <20ms; idempotency keys. AC: load test 1k events/s
  on dev hardware; duplicate delivery does not duplicate memories.
MEM-2  Redaction & secret scanning (M)
  PII patterns + gitleaks-derived secret rules; modes deny/redact/quarantine per policy pack.
  AC: seeded secrets never reach storage in any mode; quarantine review queue works.
MEM-3  Extraction pipeline (L)
  Temporal workflow: classify into fact/decision/preference/procedure/entity/episode;
  Extractor trait (Claude API + vLLM impls); summarise-at-write. AC: extraction precision
  measured on a labelled fixture set ≥ target (see EVAL-2); every record carries provenance
  (session, method, model version, confidence).
MEM-4  Transactional embed-or-fail (M)
  Embedding (TEI/BGE-M3) inside the ingestion transaction boundary; partial batch failure →
  retry, never silent drop (the documented Mem0 failure mode). AC: chaos test kills TEI
  mid-batch; zero lost or embedding-less records.
MEM-5  Always-on dedup & conflict detection (L)
  Near-dup merge (embedding + minhash); contradiction detection creates explicit
  supersession edges with validity windows (Graphiti pattern) — never ADD-only.
  AC: LongMemEval knowledge-update category score ≥ baseline; superseded facts retrievable
  via as-of but excluded from current inject.
MEM-6  Decay, TTL & staleness (M)
  Retention per policy pack; staleness scoring; pinned exempt; Temporal sweep jobs.
  AC: retention policy change re-evaluates existing records; audit trail of expiries.
MEM-7  Identity stitching (M)
  Same human across harnesses/service accounts → one identity spine (research: hardest open
  problem; we scope it to enterprise reality where IdP is ground truth). AC: memories from
  Claude Code and API SDK sessions for the same IdP subject compose together.

──────────────────────────────────────────────
EPIC CTX — Context engine (read path)
──────────────────────────────────────────────
CTX-1  Hybrid retrieval (L)
  pgvector ANN + Tantivy BM25, RRF fusion; always filtered by tenant+scope+sensitivity via
  authz-derived predicate pushdown. AC: retrieval quality on fixture set; NO LLM calls on
  read path; p99 <80ms at 1M records/tenant.
CTX-2  Composition engine (L)
  Scope-gradient assembly (user>team>dept>org), pinned-first, conflict rules, token budget
  (default 1.5k, per-scope configurable); channel rules (published + policy-permitted
  derived). AC: deterministic given same inputs; every block watermarked with commit hashes
  + record IDs; tokens_per_inject metric emitted.
CTX-3  inject API (M)
  Session-start contract; warm-cache p99 <150ms; graceful degradation (partial context +
  warning header rather than failure). AC: latency SLO under 1k concurrent sessions;
  degradation modes tested.
CTX-4  Tiered injection / progressive disclosure (M)
  Inject carries a compact index of available deeper assets (names+descriptions, skills-style
  ~80 tokens each); bodies fetched via recall. AC: token cost of index tier measured;
  agent can navigate index→body in a live Claude Code session.
CTX-5  recall API + MCP tool (M)
  Explicit deep query: hybrid + graph traversal + as-of; results labelled with channel,
  provenance, validity. Exposed as ONE MCP tool. AC: MCP client E2E; as-of returns
  historically accurate context (`--as-of` demo).
CTX-6  Session compression assist (M)
  Optional pre-compact hook support: hybrid sliding window summarisation of session history
  into observe events. AC: PreCompact in Claude Code produces derived memories; probe-based
  eval shows key facts survive compression.

──────────────────────────────────────────────
EPIC GRPH — Knowledge graph & relationships
──────────────────────────────────────────────
GRPH-1 Multi-graph AGE schema (M)
  Named graphs per tenant: entity, episode, provenance (MAGMA-informed). Edges carry
  bitemporal validity. AC: Cypher round-trip tests; edge supersession preserves history.
GRPH-2 Graph-linking stage (M)
  Ingestion links records→entities→episodes; entity resolution against existing nodes.
  AC: entity dedup precision on fixture set; orphan rate tracked.
GRPH-3 Graph-augmented recall (M)
  1–2 hop expansion in recall ranking; degradable (retrieval works with graph off).
  AC: multi-hop question set improves vs vector-only baseline; feature-flagged.
GRPH-4 AGE performance spike / Kuzu fallback assessment (S) [de-risk, Phase 2 gate]
  AC: report with traversal benchmarks at 1M/10M edges; go/no-go criteria recorded as ADR.

──────────────────────────────────────────────
EPIC FLOW — VedaFlow (git-style governance)
──────────────────────────────────────────────
FLOW-1 Object store (M)
  BLAKE3 content-addressed objects/trees/commits/refs in Postgres; commits record author
  identity, signature, and policy-pack snapshot hash. AC: property tests — identical content
  dedups; history immutable under concurrent writers.
FLOW-2 Channels (M)
  derived/staged/published refs per scope per asset type; inject reads published (+ derived
  per policy). AC: "bank mode" switch (published-only) flips composition instantly.
FLOW-3 Proposals & approval matrix (L)
  Proposal lifecycle; required approvals resolved from asset×sensitivity×scope×pack;
  approvals are authz-checked actions; CODEOWNERS-style curator files per scope.
  AC: full matrix golden tests; a memory promotion team→published E2E with 1 curator;
  restricted asset requires compliance + dual approval.
FLOW-4 Auto-promotion rules (M)
  Rule engine: e.g. procedure recalled >N times by ≥3 members → open proposal. AC: rule
  fires in soak test; proposals carry evidence (usage stats) for the reviewer.
FLOW-5 Cross-scope promotion (M)
  Team→dept→org climbs with each level's approvers. AC: E2E of knowledge climbing two
  levels with distinct approver sets; denial at any level audited with reason.
FLOW-6 CLI review flow (M)
  `synveda proposal list/show/review/approve/reject`; diff rendering for text assets.
  AC: full review possible without console.
FLOW-7 Rollback & pinning (S)
  Ref rollback; agents heal next session; assets pinnable to a commit per scope.
  AC: bad-prompt rollback demo <60s to fleet-wide effect.
FLOW-8 Git bridge — export (M)
  gitoxide mirror of published channels to a bare repo / GitHub for visibility & PR-culture
  review. AC: published history round-trips to a real git repo with signatures preserved.

──────────────────────────────────────────────
EPIC SKIL — Skills registry (spec-compliant + governed)
──────────────────────────────────────────────
SKIL-1 agentskills.io-compliant model (M)
  SKILL.md + frontmatter + bundled files as a VedaFlow asset type; validate against the open
  spec; import from anthropics/skills format. AC: a skill authored in Synveda installs and
  runs unmodified in Claude Code and one other client (Cursor or Codex).
SKIL-2 Security scanning gate (M)
  Static analysis of skill scripts (secret patterns, network egress, dangerous calls);
  scan report attached to proposal; security-reviewer role required for executable skills.
  AC: seeded-malicious skill cannot reach published; report renders in review.
SKIL-3 Skill quality scoring (M)
  SkillsBench-style rubric scoring (automated + reviewer checklist) stored on the version.
  AC: score displayed at review and in the registry; low-score publish requires override.
SKIL-4 Scope-targeted distribution (M)
  Skills attach to hierarchy nodes; inject index tier lists skills available to this
  identity; adapter materialises them into the harness (Claude Code plugin dir).
  AC: user in team A sees team A's skills; team B's are absent; org skills present for both.
SKIL-5 Skill usage telemetry (S)
  Trigger counts, success signals per skill version feeding FLOW-4 evidence and EVAL-4.
  AC: usage dashboard per scope.

──────────────────────────────────────────────
EPIC PRMT — Prompt & context-pack registry
──────────────────────────────────────────────
PRMT-1 Prompt templates as assets (M)
  Versioned, variable-schema'd templates; draft→review→publish; consumed via API/SDK by id +
  channel. AC: prompt change behind review; consumer pins channel or commit.
PRMT-2 Context packs (M)
  Curated doc bundles (conventions, glossaries) pinned to scopes; chunked+embedded on
  publish; composed by CTX-2 as pinned material. AC: pack update re-embeds atomically;
  inject reflects new pack next session.
PRMT-3 A/B channels for prompts (S)
  Staged rollout: % of sessions on candidate version; metrics comparison feeding promotion.
  AC: two-version experiment with automatic report.

──────────────────────────────────────────────
EPIC AUD — Audit (functional requirement)
──────────────────────────────────────────────
AUD-1  Hash-chained audit log (M)
  Append-only, BLAKE3-chained per tenant; every authz decision, inject (with commit-hash
  watermarks), recall, observe, proposal transition, policy change, lapse, admin action.
  AC: tamper test — mutating any historic row breaks chain verification.
AUD-2  Audit query & auditor role surface (M)
  Search by actor/resource/time/action; auditor role read-only incl. denials; answer
  "who could see X on date D" and "what did agent A know at time T". AC: both questions
  answerable via one API call each (uses bitemporal + refs).
AUD-3  WORM export (M)
  Scheduled signed export (S3 object-lock compatible target); verification tool.
  AC: exported chain independently verifiable offline.
AUD-4  SIEM streaming (S)
  CEF/OTLP audit event stream for Splunk/Sentinel. AC: events arrive in a dev Splunk with
  correct schema.
AUD-5  Compliance mapping doc (M) [Phase 3]
  SOC2/ISO27001/DORA control mapping referencing concrete features. AC: reviewed by an
  external checklist; gaps ticketed.

──────────────────────────────────────────────
EPIC EVAL — Evaluation (functional requirement)
──────────────────────────────────────────────
EVAL-1 Eval harness skeleton (M)
  Rust runner + fixtures; executes scenario suites against a live stack; CI-integrated with
  regression gates on the five axes: accuracy, latency, tokens, recall, abstention.
  AC: `make eval` runs the scenario suite against a live stack and reports all five axes as
  machine-readable JSON plus a human summary; a committed baseline gates the run; a real
  product change that degrades quality (a bank-mode pack flip withholding derived memory)
  fails the gate naming the axis, the baseline, the measurement, and the delta; nightly
  workflow; demo script.
EVAL-2 Extraction quality suite (M)
  Labelled transcript fixtures → precision/recall per memory class; hallucinated-memory rate
  (HaluMem-style). AC: dashboard; gate on regression >2pts.
EVAL-3 Public benchmark adapters (L)
  LoCoMo + LongMemEval run end-to-end through Synveda (observe→inject/recall→judge).
  AC: reproducible scores published in repo; tracked per release. (Marketing artefact too —
  every credible 2026 memory system publishes these.)
EVAL-4 Retrieval & injection quality (M)
  Fixture Q&A per scope; probe-based compression eval (CTX-6); tokens-per-inject trend.
  AC: composition changes show measurable quality effect before merge.
EVAL-5 Security evals (M)
  Policy-leak suite (restricted content never crosses sensitivity/scope under 10k generated
  query variants); cross-tenant fuzz (TEN-6); prompt-injection-via-memory suite (a memory
  containing instructions must not alter agent behaviour when injected — content is data,
  wrapped and labelled). AC: nightly; zero-tolerance gate.
EVAL-6 Load & latency suite (M)
  k6/vegeta profiles for inject/observe/recall SLOs at SMB and enterprise shapes.
  AC: SLO report per release.

──────────────────────────────────────────────
EPIC ADPT — Adapters & SDKs
──────────────────────────────────────────────
ADPT-1 Claude Code adapter (L)
  TS plugin: SessionStart/PreCompact/Stop hooks → inject/observe; MCP recall tool; skills
  materialisation (SKIL-4); zero-config after `synveda login`. AC: fresh machine to
  personalised session in <2 minutes; demo script.
ADPT-2 Generic MCP server (M)
  recall (+ policy-gated write) for any MCP client. AC: works in Claude Desktop + one
  non-Anthropic client.
ADPT-3 REST/gRPC API + OpenAPI (M)
  The three primitives + admin surface; versioned; API keys for service identities.
ADPT-4 Python & TS SDKs (M)
  Thin typed clients; LangGraph memory interface shim; OpenAI Agents SDK shim.
  AC: LangGraph example app persists and recalls across sessions.
ADPT-5 Importers (M) [Phase 4]
  claude-mem, Cognee, mem0 export formats → Synveda records with provenance=imported.
  AC: round-trip fidelity report per source.

──────────────────────────────────────────────
EPIC OPS — Deployment & operations
──────────────────────────────────────────────
OPS-1  SMB profile (M)
  Single gateway binary + Postgres + Rauthy + TEI compose; `synveda init` seeds org.
  AC: laptop → working governed memory in <10 minutes, documented.
OPS-2  Helm chart / enterprise profile (L)
  HA Postgres (CloudNativePG), Temporal cluster, optional Qdrant, customer IdP wiring.
  AC: kind-cluster CI install test.
OPS-3  Residency routing (L)
  Region-pinned data planes; global control plane; cross-region inject returns policy-safe
  summaries only. AC: EU-pinned tenant's embeddings never leave EU plane (verified by
  network policy test).
OPS-4  Qdrant adapter behind VectorIndex trait (M)
  Benchmark gate vs partitioned pgvector at 1M/5M/20M vectors with realistic filters
  decides per-deployment default. AC: benchmark report; conformance suite passes both.
OPS-5  Backup/restore & DR (M)
  PITR playbook; restore drill in CI monthly. AC: RPO/RTO documented and tested.
OPS-6  Zero-downtime migration discipline (S)
  Expand/contract migration checklist enforced by CI lint. AC: demo migration under load.

──────────────────────────────────────────────
EPIC CNSL — Admin console (Phase 3)
──────────────────────────────────────────────
CNSL-1 Proposals inbox (hero screen) (L) — review queue with diffs, scan reports, quality
  scores, evidence; approve/reject. AC: full review parity with CLI.
CNSL-2 Hierarchy & policy explorer (M) — visualise scopes, packs, roles, active lapses.
CNSL-3 Audit explorer (M) — AUD-2 surfaced; "what did agent know at T" as a UI query.
CNSL-4 Memory browser (M) — per-scope records with provenance, channel, validity; manual
  pin/retire (as proposals). AC: no direct-mutation path exists — everything is a proposal.

──────────────────────────────────────────────
Sequencing (features → phases)
──────────────────────────────────────────────
Phase 0 (wk 1):    FND-1..6
Phase 1 spine (wk 2–5):  TEN-1,2 · AUTH-1 · HIER-1 · AUTHZ-1 · AUTH-2 · AUTHZ-2,3 ·
                         HIER-2,3 · AUTH-3 · AUD-1 · MEM-1,2,3,4 · CTX-1,2,3 ·
                         ADPT-1 (minimal) · EVAL-1
   (Order within the phase is topological, not epic-grouped: HIER-1 precedes
   AUTHZ-1/AUTH-2 — Cedar entities and JIT provisioning need hierarchy nodes;
   AUTH-3's scope-enforcement AC needs the PDP; AUD-1 precedes the data path so
   MEM/CTX are born audited.)
   → Demo: SSO login → auto-scoped → live Claude Code session writes and receives governed
     memory, fully audited.
Phase 2 governance (wk 6–10): FLOW-1..7 · AUTHZ-4,5 · MEM-5,6 · CTX-4,5 · GRPH-1,2,4 ·
                         AUD-2 · EVAL-2,4,5 · PRMT-1,2
   → Demo: promotion pipeline, lapse lifecycle, as-of inject, bank-mode switch.
Phase 3 enterprise (wk 11–16): AUTH-4,5 · TEN-3,4,5,6 · SKIL-1..4 · GRPH-3 · AUD-3,4 ·
                         EVAL-3,6 · OPS-1..4 · CNSL-1,2 · ADPT-2,3 · CTX-6 · FLOW-8
   → Demo: Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor,
     LoCoMo/LongMemEval scores published, Helm install.
Phase 4 ecosystem: ADPT-4,5 · PRMT-3 · SKIL-5 · MEM-7 · OPS-5,6 · CNSL-3,4 · AUD-5 · AUTHZ-6
──────────────────────────────────────────────

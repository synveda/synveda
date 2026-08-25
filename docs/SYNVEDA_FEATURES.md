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
→ **Design the graph schema as multiple named graphs from day one (entity graph, episode
   graph, provenance graph) — cheap now, expensive to retrofit. The semantic partitioning is
   the research finding and it stands; the engine that carries it is a separate question,
   settled against AGE by GRPH-1/ADR-0043, and the per-tenant instantiation was already
   dropped by GRPH-4/ADR-0029.**

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
→ **Synveda ships an eval harness that runs LongMemEval against the whole pipeline,
   plus enterprise-specific evals (policy-leak tests, cross-tenant isolation tests).**
   (Read "LoCoMo + LongMemEval" until 2026-08-07. LoCoMo remains a benchmark that matters —
   the paragraph above is unchanged and still true — but its corpus is CC BY-NC 4.0, so we
   may not publish a score from it. That is EVAL-7, not EVAL-3. A licence bounds what we can
   claim, not what the field measures.)

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
The embedded-graph alternatives that appeared in memory stacks have thinned out: the
notable one is no longer maintained, and the mature property-graph engines are GPL or BSL,
failing the core-path licence rule.
→ **Keep graph features additive/degradable. The fallback if AGE Cypher perf disappoints is
   inside Postgres — indexed adjacency, then a materialised k-hop closure table — not a
   second engine (spike in Phase 2; settled by GRPH-4/ADR-0029: AGE passed on latency, and
   the fallback ladder was rewritten there). Updated 2026-07-27 (GRPH-1/ADR-0043): "AGE
   choice holds" no longer does — the spike's own relational baseline beat it on every
   measured axis including the two criteria AGE failed, so rung 1 of that ladder is what
   ships and no crate calls AGE. The single-engine argument is untouched: the graph is
   still in Postgres, still transactional with records.**

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
TEN-3  Dense-leg retrieval benchmark (M)
  (Amended 2026-08-10 by ADR-0063 decision 4, after the benchmark its own AC asked for came
  back negative. Read "Tenant-partitioned storage layout — declarative partitioning by tenant
  hash for records/embeddings; partial HNSW indexes per partition (mitigates pgvector
  post-filtering). AC: filtered ANN query plan shows partition pruning; benchmark vs
  unpartitioned recorded." The AC's second clause is a comparison, and a comparison allowed
  to say no said no: in the regime the gate was written for, recall is already 1.000 at 1.65ms
  p95 on an exact scan that never touches the HNSW index, so partitioning by tenant has
  nothing there to improve. The first clause cannot be shown by a deployment that does not
  partition, so it is amended rather than satisfied. The partitioning half is TEN-7, as LIST.)
  A recall-and-latency harness for the dense leg over a seeded corpus, at stated sizes and
  tenant counts, in both filter regimes; arms recorded with the corpus, the pgvector version
  and the commit in each row. AC: recall@10 against exact search and p50/p95 for every arm,
  three runs each, rows published and re-checked by CI; the plan each arm ran shown at
  EXPLAIN rather than assumed.
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
TEN-7  LIST partitioning per tenant (L)
  The partitioning half TEN-3 measured and declined, as LIST rather than HASH — a hash
  partition holds an arbitrary set of tenants, so it can be neither dropped for one nor
  pinned for one, which is what every feature that wants partitioning actually wants. Costs
  `records` the meaning of its own primary key (composite with tenant_id), drags the
  bitemporal triple and both archive triggers with it, and is an operator-run offline
  repartition rather than a migration. AC: whichever trigger reopened it, met and measured;
  filtered ANN query plan shows partition pruning at EXPLAIN (ANALYZE) with partitions
  actually removed; TEN-3's harness re-run across the change; every partition carries its own
  enabled and forced RLS and ADR-0009's completeness guard covers relkind 'p'.

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
  DELETED WHOLE by CPR-7 on 2026-08-20 (ADR-0074 decisions 1 and 6). Authority is a grant of
  one of six role keys at a governed scope, inherited by its subtree — CPR-5 and CPR-6.
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
AUTHZ-7 Governed admin-plane mutation (M)
  Pack assignment and role binding are direct `PUT`s (AUTHZ-2, AUTHZ-3) while every content act
  is proposal-gated. Decide whether they gain an approval-matrix cell of their own, and record
  the answer either way. Filed by CNSL-2 (ADR-0058 decision 9), which found it by building the
  screen that renders a pack, its origin and the grants over it on one page: all three packs
  grant `PolicyAssign` to steward/org-admin over the bound subtree and the decision skips the
  node's own assignment (ADR-0014 decision 4), so one steward replaces a team's pack with one
  call and one signature, permanently — while the **lapse** that relaxes far less requires a
  reasoned, time-boxed, dual-approved proposal that expires on its own. Seed §2.3 has controls
  relaxed "through explicit, audited, time-boxable policy relaxations"; a pack assignment is
  explicit and audited and is neither of the other two. Bounded, and the bounds hold: a pack
  flip cannot widen anyone's candidate universe (ADR-0037 decision 13 — which is why EVAL-5's
  relaxation demo had to be a lapse) and cannot reach below the invariant floor (ADR-0032
  decision 4, ADR-0051 decision 18, ADR-0052 decision 3). It changes approval counts,
  sensitivity ceilings, scan thresholds and quality bars for a whole subtree.
  AC: the decision recorded as an ADR before any implementation; if gated, the admin-plane cell
  joins the role×action and approval golden tests under all three packs, `policy.node.assigned`
  and `role.bound` become proposal effects, and the explorer gains the write path CNSL-2
  declined; if left direct, the seed §2.3 reading that permits it is written down and the
  compensating control named.

──────────────────────────────────────────────
EPIC HIER — Hierarchy & scopes
──────────────────────────────────────────────
HIER-1 Hierarchy store (M)
  Closure table + materialised path; org→division→department→team→user with configurable
  depth; CRUD via admin API. AC: 10k-node hierarchy; ancestor/descendant queries <1ms.
  DELETED WHOLE by CPR-7 on 2026-08-20 (ADR-0074 decision 1). The closure-table shape
  survives in `scopes` + `scope_closure`; the rank vocabulary does not.
HIER-2 Scope chain resolver (S)
  Given identity → ordered scope chain for composition (user→…→org), cached with
  invalidation on hierarchy change. AC: cache invalidation test; p99 <0.5ms warm.
  DELETED WHOLE by CPR-7 on 2026-08-20 (ADR-0074 decision 2). Chains resolve per request
  through `scope_closure`; the warm cache went with the tree it cached.
HIER-3 Cedar entity sync (M)
  Hierarchy changes stream into Cedar entity store transactionally. AC: move a team between
  departments → authz decisions reflect it in the same transaction boundary.
  RE-CUT onto governed scopes by CPR-7 on 2026-08-20: read "move a scope between org units".

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
CTX-7  Dense-leg plan stability (M)
  The dense leg is a prepared statement on a long-lived pool, so PostgreSQL plans it against
  real parameters five times per connection and then substitutes a generic plan — which drops
  the HNSW index and scans the whole allowed slice exactly. Rule on which plan the read path
  should run, and make it a decision rather than a default. AC: the plan the dense leg runs is
  asserted rather than assumed; recall and p50/p95 recorded for both plans at 1024 dimensions;
  CTX-1's own latency AC re-read against whichever plan its test is actually measuring.

──────────────────────────────────────────────
EPIC GRPH — Knowledge graph & relationships
──────────────────────────────────────────────
GRPH-1 Multi-graph schema (M)
  Named graphs — entity, episode, provenance (MAGMA-informed) — carried as a mandatory
  discriminator over a bitemporal edge pair in Postgres. Edges carry bitemporal validity.
  AC: an edge written through the store API reads back through the traversal API with its
  kind, endpoints and validity intact; a supersession closes the prior edge's window with
  both versions readable as-of; the shipped statements' plans contain no sequential scan
  over the edge table.
  Amended 2026-07-27 (ADR-0043): the title was "Multi-graph AGE schema" and the criterion
  named Cypher round-trip tests. GRPH-4/ADR-0029 measured relational adjacency 3–8× faster
  than AGE at 2.5× less storage and handed the schema call to this feature's design ADR;
  the substance of the criterion survives, the Cypher mechanism does not.
GRPH-2 Graph-linking stage (M)
  Ingestion links records→entities→episodes; entity resolution against existing nodes.
  AC: entity dedup precision on fixture set; orphan rate tracked.
GRPH-3 Graph-augmented recall (M)
  1–2 hop expansion in recall ranking; degradable (retrieval works with graph off).
  AC: multi-hop question set improves vs vector-only baseline; feature-flagged.
GRPH-4 AGE performance spike / graph fallback assessment (S) [de-risk, Phase 2 gate]
  Benchmark AGE Cypher traversal at the scales ADR-0001 and ADR-0004 both flag as unproven,
  and decide whether the multi-graph AGE schema survives. Assess the fallback the two ADRs
  name as their reversal trigger, and record the conditions that would activate it.
  AC: report with traversal benchmarks at 1M/10M edges; go/no-go criteria recorded as ADR
  — recorded *before* the benchmark runs, since a spike that fixes its thresholds after
  seeing the numbers can only ratify the decision it was commissioned to test.

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
  AC: a rewind can only install a state the channel has held — a proposal commit and an
  orphaned publication are both refused by name (ADR-0036 decisions 1–2).
  AC: a pinned scope serves its pinned commit while publications keep landing; the block's
  watermark says so; releasing the pin catches every reader up on the next session.
FLOW-8 Git bridge — export (M)
  gitoxide mirror of published channels to a bare repo / GitHub for visibility & PR-culture
  review. AC: published history round-trips to a real git repo with signatures preserved.

──────────────────────────────────────────────
EPIC SKIL — Skills registry (spec-compliant + governed)
──────────────────────────────────────────────
SKIL-1 agentskills.io-compliant model (M)
  SKILL.md + frontmatter + bundled files as a VedaFlow asset type; validate against the open
  spec; import from anthropics/skills format. AC: a skill authored at a scope reaches a client
  only through the review the pack in force asks for — and under *every* pack that is two
  distinct people, one of them a security-reviewer, because the invariant floor has priced
  executable content at a security reviewer since FLOW-3 but never at a second *person*, so the
  one separation "skills are treated like code because they are" exists to draw collapsed into
  one signature under `standard`, in a cell nothing could reach until this feature; "installs
  unmodified" is a hash comparison rather than a claim — one published commit materialises into
  each client's own skills root, the trees are byte-identical to each other file for file, every
  file's content address recomputes to the address the commit named, and the bundle directory
  holds exactly the reviewed files and nothing else, the install receipt living *outside* it
  because a receipt inside the bundle is the modification the criterion forbids, which is what
  makes this the first governed read surface that cannot watermark what it serves; the
  frontmatter is parsed by a **strict subset** of YAML that refuses what it cannot represent —
  anchors, aliases, tags, a second document, anything outside the spec's key vocabulary — rather
  than by a general parser, because the reviewed meaning and the loaded meaning must be the same
  meaning and two YAML parsers disagreeing is exactly how they stop being; the spec's own rules
  are enforced where they can fail, at authoring, naming the offender: a name that is not one
  lower-case hyphenated segment (a *stricter* grammar than the product's own, imported from the
  spec, or the product would admit at the first step what a third-party client refuses at the
  last), a frontmatter `name` disagreeing with the skill's name, a missing or empty
  `description`, no SKILL.md at all, and a bundled path that a materialisation would let escape
  its directory — `..`, absolute, a reserved device name, a trailing dot or space, or a
  case-fold collision with a sibling, the last being the one a case-preserving filesystem turns
  into silent overwriting; every materialised file is non-executable, so a governed bundle
  cannot arrive carrying a mode nobody reviewed; a skill's content never becomes a record —
  ADR-0049 option 4's "fetched by name where a record is ranked by relevance" holds here where
  PRMT-2 inverted it for packs, because the client's own progressive disclosure is the loader
  and ranking a SKILL.md body into a block would spend the budget doing that job twice; import
  reads an anthropics/skills directory and refuses a symlink, a bundle with no SKILL.md and a
  file over the bound rather than importing part of one; resolution walks the caller's own
  placement chain nearest-first and skips the scopes the PDP refuses, so a team's version
  overrides the org's, a nearer copy nobody may read does not shadow the further readable one,
  and a name nothing publishes is the uniform 404 rather than an existence oracle — and because
  the name is also the installed directory name, that gradient is a filesystem fact rather than
  a policy one; a consumer names a channel and follows publications or names a commit and keeps
  what it installed, and a FLOW-7 rewind refuses the pinned read naming both commits, PRMT-1's
  rule inherited whole, which is what makes a receipt reproducible and a rollback still mean
  sixty seconds; a bundle carrying a live credential is stopped at authoring by MEM-2's scanner,
  so no secret reaches a client's disk; `SkillRead` and `SkillWrite` join the role×action golden
  matrix under all three packs and the service-identity confinement list, which discharges
  ADR-0036 decision 3 for the **last** of the three kinds it refused by name and leaves none;
  every act is on the chain — `skill.authored`, `skill.resolved`, and the same
  `vedaflow.channel.published` a memory publication emits with `asset` reading `skill` — with no
  SKILL.md text and no file content in any payload, swept for; demo script. Deferred with a
  recorded trigger: the behavioural half of "runs" — whether a model reaches for the skill it
  was served — because it measures a model's disposition rather than the product's bytes and
  would fail when a model changed rather than when the code did, which is EVAL-5's own deferral
  arriving from the distribution side; the demo runs `claude` and `codex` live against the
  materialised bundles when the binaries are present, and reports the skip with its reason when
  they are not.
  Written 2026-08-03 (SKIL-1, ADR-0051): the feature text named three clauses and a one-line AC
  whose verb ("runs") belongs to a model rather than to this product. The load-bearing parts are
  that the **format is somebody else's** — the first governed asset whose bytes must leave
  Synveda untouched, which costs the watermark every other read surface carries and buys
  portability in its place — and the discovery that the floor's skill rule required the
  security-reviewer *role* without ever requiring a second *signature*.
SKIL-2 Security scanning gate (M)
  Static analysis of skill scripts (secret patterns, network egress, dangerous calls);
  scan report attached to proposal; security-reviewer role required for executable skills.
  AC: seeded-malicious skill cannot reach published; report renders in review.
  Written 2026-08-03 (SKIL-2, ADR-0052): two of the three clauses arrived already discharged
  — "secret patterns" has been MEM-2's scanner at this seam since ADR-0051 decision 14, and
  the security-reviewer requirement is the invariant floor's second rule, on every skill
  rather than only an executable one, because decision 8 refused the executable bit and there
  is no such thing as a non-executable skill here. The load-bearing parts are that a skill's
  **prose is executable** — a SKILL.md instructing an agent to fetch and run something is the
  same attack with the model as the interpreter, so "skill scripts" is the wrong scope — and
  that **"cannot reach published" is not the whole boundary**, because a draft is installable
  by anyone the pack lets read skills at that scope. The gate therefore sits at authoring,
  where a refused bundle is never stored, and again at publication, where the rule table is
  re-applied to bytes an approval already bound.
SKIL-3 Skill quality scoring (M)
  SkillsBench-style rubric scoring (automated + reviewer checklist) stored on the version.
  AC: score displayed at review and in the registry; low-score publish requires override.
  Written 2026-08-03 (SKIL-3, ADR-0053): the score is **two halves that are never averaged** —
  a rubric recomputed from the bytes and a checklist a person supplies — because summing them
  lets each hide the other, and "stored on the version" is true of exactly one of them. The
  checklist is keyed by a **digest of the bundle's object addresses**, so an edit beneath a
  review finds nothing rather than inheriting answers about content nobody read; that is
  ADR-0032 decision 6 applied to the one review artefact with no address check of its own, and
  it needs no invalidation logic at all. Two findings changed the design: the rubric's heaviest
  check fired on 29 of 37 real bundles and had to be narrowed and repriced, and the override —
  first a field on the publish request — deadlocked, because `curator` holds the reads that
  publishing a skill takes and `steward` holds the override and no content read, so nobody
  could publish a below-bar bundle under any pack. It is now a separate governed act the
  publish seam looks up, which is ADR-0032 decision 9's own separation one seam later.
SKIL-4 Scope-targeted distribution (M)
  Skills attach to hierarchy nodes; inject index tier lists skills available to this
  identity; adapter materialises them into the harness (Claude Code plugin dir).
  AC: user in team A sees team A's skills; team B's are absent; org skills present for both
  — asserted on **both** surfaces the feature adds, because a block that advertised a
  capability the registry will not serve is a worse failure than either alone; and asserted
  as three *mechanisms* rather than three absences, since the org's skills arrive because
  the org is on both chains, team A's because team A is on one, and team B's are absent
  because team B is on no chain that reader has — a suite that asserted all three the same
  way would pass for a build that decided nothing. The available set is `GET /v1/skills`
  with no scope, the plural of the resolve route's own chain walk and the *same* walk, with
  the gradient applied **after** the PDP filter so a nearer copy nobody may read does not
  shadow the readable one behind it; the materialisation is a **reconcile** that removes as
  well as writes, bounded by this product's own install receipts and pointed at a root it
  created rather than a person's `~/.claude/skills`, because a rollback that stops at the
  network is not a rollback.
  Written 2026-08-03 (SKIL-4, ADR-0054): the load-bearing discovery is that **an
  advertisement is not a demotion** — CTX-4's index tier is safe because a line is taken
  only when it is strictly cheaper than the body it replaces, and a skill has no body in a
  block and never will, so this is new content with no second operand and needs its own
  bound, its own placement (last, displacing nothing) and its own off switch. What makes it
  worth its tokens is the other half of the feature: what a session materialises is loaded
  at the *next* one, so the block is current where the folder is behind.
SKIL-5 Skill usage telemetry (S)
  Trigger counts, success signals per skill version feeding FLOW-4 evidence and EVAL-4.
  AC: usage dashboard per scope.

──────────────────────────────────────────────
EPIC PRMT — Prompt & context-pack registry
──────────────────────────────────────────────
PRMT-1 Prompt templates as assets (M)
  Versioned, variable-schema'd templates; draft→review→publish; consumed via API/SDK by id +
  channel. AC: a prompt authored at a scope through POST /v1/prompts reaches a consumer only
  through the review the pack in force asks for — under the default pack the direct publish route
  refuses it by name, short of the steward and the curator the `prompt` cell has priced at two
  distinct people since FLOW-3, and the same two approvals through POST /v1/proposals carry it;
  "prompt change behind review" is measured from the reader's side and never at the writing
  surface — the draft is edited under its own published version, the author's own draft read
  returns the edit, and the consumer keeps being served the reviewed bytes at the reviewed commit
  until a second proposal lands; a consumer names a channel and follows publications, or names a
  commit and keeps the version it was built against while the channel moves on, and when a FLOW-7
  rewind takes that commit off the channel's first-parent line the pinned read is refused naming
  both commits rather than served or silently upgraded, because "<60s to fleet-wide effect"
  (FLOW-7) and a pin that outlives a withdrawal cannot both be true; a pin freezes bytes and never
  authority, so the same pinned read stops resolving when the pack behind it changes (CTX-4's
  handle rule); resolution walks the caller's own placement chain nearest-first and skips the
  scopes the PDP refuses, so a team's version overrides the org's, a nearer copy nobody may read
  does not shadow the further one that is readable, and a name nothing publishes is the uniform
  404 rather than an existence oracle; the variable schema is enforced where it can fail — a
  template whose placeholders and declared variables disagree is refused at authoring naming the
  offender, and rendering refuses a missing required value and an undeclared one — rather than
  returned beside the template and checked by nobody; PromptRead and PromptWrite join the
  role×action golden matrix under all three packs and the service-identity confinement list, which
  is what makes a rewind of `prompt/published` decidable and discharges ADR-0036 decision 3's
  "refused by name until PRMT-1 brings their read action"; every act is on the chain —
  `prompt.authored`, `prompt.resolved`, and the same `vedaflow.channel.published` a memory
  publication emits with `asset` reading `prompt` — with no template text in any payload, swept
  for; demo script.
  Written 2026-08-02 (PRMT-1, ADR-0049): the feature text named two clauses of AC, one of which
  ("consumer pins channel or commit") ADR-0036 decision 12 had already refused in the only reading
  it had then. The load-bearing parts are the pin's shape — a request parameter rather than a
  stored decision, which a rewind refuses rather than outlives — and the discovery that the first
  authored asset needed no channel shape, no proposal effect and no approval rule, because FLOW-3
  priced prompts two features before anything could open one.
PRMT-2 Context packs (M)
  Curated doc bundles (conventions, glossaries) pinned to scopes; chunked+embedded on
  publish; composed by CTX-2 as pinned material. AC: a pack authored at a scope reaches a
  session only through the review the pack in force asks for — and under `regulated-strict`
  at a department, division or org that is a curator *and* a steward, two distinct people,
  where FLOW-3 had left the cell at one curator, because publishing a bundle into every
  session must not be cheaper than publishing one memory record at the same scope;
  "re-embeds atomically" is measured from the reader's side — no inject ever composes half a
  pack, the previous version composes in full until the new one is entirely embedded *and*
  published, and the new one in full thereafter; "next session" is satisfied as "next call",
  because the pack channel is read live on the composition path; pack content composes as
  pinned material, **ranked**, and what does not fit is named in the index tier rather than
  dropped — a block that cannot hold the runbook says the runbook exists, names it, and
  hands back a recall handle that resolves; `ContextPackRead` admits pack chunks and
  `MemoryRead` never does, so a reader who holds no readable memory at a scope still
  receives that scope's conventions, decided per scope inside the plan walk composition
  already runs; a published document that is edited demotes its own chunks, ADR-0031
  decision 5 reaching chunks through the document address the channel names; a rewind
  restores the previous version by moving a ref with no re-embedding, and a pin freezes what
  the pack channel serves, which is what discharges ADR-0036 decision 3 for the second of
  the three kinds it refused by name and leaves `skill`; a document carrying a live
  credential is stopped at authoring by MEM-2's scanner running ahead of the embedder, so no
  secret reaches vector space; `ContextPackRead` and `ContextPackWrite` join the role×action
  golden matrix under all three packs and the service-identity confinement list; every act
  is on the chain — `context_pack.authored`, `context_pack.quarantined`, and the same
  `vedaflow.channel.published` a memory publication emits with `asset` reading
  `context-pack` — with served chunks watermarked inside `context.injected` and no document
  text in any payload, swept for; demo script.
  Written 2026-08-02 (PRMT-2, ADR-0050): the feature text named two clauses of AC, and both
  turned out to be about the *read* half rather than the write half — which is what makes
  this feature unlike PRMT-1. A prompt is fetched by name and composes into nothing; a
  context pack is the first authored asset whose content has to enter the corpus CTX-1 ranks
  and CTX-2 assembles, so ADR-0049 option 4's third reason for refusing "prompts as memory
  records" inverts here and its published chunks *are* pinned records. The load-bearing
  parts are that decision — which inherits both retrieval legs, the tier check, recall, the
  retention exemption and the supersession exemption rather than re-earning them — and the
  discovery that FLOW-3 had priced `context-pack` at one curator at every scope kind, in a
  cell tech plan §2.4 left empty and nothing could reach until this feature.
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
  AC: `make eval` runs the scenario suite against a live stack and reports all five axes
  (accuracy, latency, tokens, recall, abstention) as machine-readable JSON plus a human
  summary; a committed baseline gates the run; a real product change that degrades quality
  (a bank-mode pack flip withholding derived memory) fails the gate naming the axis, the
  baseline, the measurement, and the delta; nightly workflow; demo script.
  Written 2026-07-25 (EVAL-1, ADR-0028): the feature text specified a runner and gates but
  no criteria. The gate is the load-bearing part — a harness that reports without failing is
  a dashboard, and the five axes only mean something if a real regression trips them.
EVAL-2 Extraction quality suite (M)
  Labelled transcript fixtures → precision/recall per memory class; hallucinated-memory rate
  (HaluMem-style). AC: one labelled corpus under evals/fixtures/extraction/ read by both the
  eval harness and MEM-3's unit test, so a format change breaks both loudly; `make eval`
  reports per-class precision and recall for every RecordClass the corpus exercises, plus
  macro averages, measured over the real observe→extract→serve path and never over seeded
  records; the report carries produced/expected/matched per class, the unmatched-record list,
  and the pipeline's own committed counts read from the audit chain, so a shortfall between
  what was committed and what a reader is served is its own number rather than absorbed into
  recall; `hallucination_rate` measured from fixture-declared bait and gated at zero; a real
  product change that degrades quality (a retention horizon cutting served records while the
  pipeline still commits them) fails the gate naming the axis, the baseline, the measurement,
  and the delta, and the attribution column says why; the >2pt tolerance is a declared slack
  in the committed baseline, not a rule in code; deterministic macro precision ≥0.90; the
  live-model run measures the same corpus on demand against its own baseline, recording the
  model the API served; nightly workflow; demo script.
  Written 2026-07-30 (EVAL-2, ADR-0046): the feature text named a dashboard and a threshold
  but no axis, no path, and no artefact. The lens is the load-bearing part — extraction
  quality is a property of a record set, and an inject block cannot express one.
EVAL-3 Public benchmark adapters (L)
  LongMemEval runs end-to-end through Synveda (observe→inject/recall→judge). LoCoMo was
  named here until 2026-08-07 and is now EVAL-7: its LICENSE.txt is CC BY-NC 4.0, which
  withholds exactly the use this AC's own parenthetical describes, so a score we may not
  quote has no acceptance criterion. LongMemEval is MIT.
  AC: reproducible scores published in repo; tracked per release. (Marketing artefact too —
  every credible 2026 memory system publishes these.) Two tiers, because the benchmark
  publishes two metrics with different reproducibility: a DETERMINISTIC retrieval gate —
  did the block bind the evidence sessions the instance names in answer_session_ids, graded
  by record identity as EVAL-4 grades, reproducible from bytes, on the nightly, failing with
  the axis, the baseline, the measurement and the delta — and a MODEL-JUDGED end-to-end QA
  accuracy that is the published figure and gates nothing, run on demand against its own
  baseline, off the merge path because a gate that fails when a model changed rather than
  when the code changed is an alarm nobody keeps (ADR-0028 decision 6); the judge is a
  `Judge` seam with a deterministic default, the shape `Extractor` already has, which is
  also the seam ADR-0053 option 9's SkillJudge becomes an implementation of; THE JUDGE IS
  MEASURED BEFORE IT MEASURES — scored against EVAL-2's unmatched-record list (the labelled
  set ADR-0046 option 6 named when it deferred this) and LongMemEval's 500 reference
  answers, with its agreement rate published as a first-class axis beside the product's
  score, because a number produced by an unmeasured judge is a second opinion with a decimal
  point; the baseline is keyed to BOTH models the score depends on — the reader that answers
  from the block and the judge that grades the answer — recorded from what the API served
  rather than the alias requested, since a memory benchmark figure quoted without its reader
  model is reproducible by nobody including us; a declared slice on the routine path and the
  full 500 behind its own target, with the instance count, slice interval, abstention
  exclusion and skip count stated on every run; one actor per instance, because the
  32-record sweep cap is a rule and not a limit to raise; everything at the `user` tier,
  because LongMemEval has no teams and synthesising a hierarchy would measure a fiction;
  scores accumulate as rows in evals/scores/ and docs/BENCHMARKS.md, each carrying both
  model versions and the commit.
  Written 2026-08-07 (EVAL-3, ADR-0061): the feature arrived with five other ADRs' deferrals
  already naming its judge, so most of its design was written before it started. The
  load-bearing decisions are the two that were not — that the judge owes its own measurement
  first, and that a benchmark score is never a measurement of the memory system alone.
EVAL-4 Retrieval & injection quality (M)
  Fixture Q&A per scope; probe-based compression eval (CTX-6); tokens-per-inject trend.
  AC: one Q&A corpus under evals/fixtures/qa/ whose material sits at four scope tiers because
  the suite promoted it there through the governed path — seeded at an actor's home through
  /v1/observe, then climbed to a team, a department and the org through POST /v1/proposals
  and real approvals, because records land at the caller's home scope and a service identity
  is a leaf under its anchor, so no other arrangement can put material above a leaf; one
  corpus seeded once and asked many times, so every question in a file measures the same
  corpus; grading joins seed to block by record identity (observe's event_id → the recall
  sweep's provenance.event_id → record_id → its position in the block's record_ids and
  tiers), never by string containment, because an index entry carries a truncated head and
  "demoted" and "absent" are otherwise the same measurement; `make eval` reports
  `qa_answer_rate` and `qa_body_rate` per scope tier and over the corpus, and
  `tokens_per_answer` as the exchange rate a composition change actually moves, with the gap
  between answer rate and body rate reported as the index tier's displacement; every question
  declares needs: lexical | semantic, and semantic questions are skipped and counted rather
  than scored zero on a run whose embedder cannot rank; `retrieval_precision` reads only the
  blocks something bound, and is gated on both paths at different values because on the
  deterministic one it is the sparse leg alone; the dense leg is measured against live TEI on
  the nightly and gated against evals/baseline-retrieval.json, whose floors are measurements
  of today rather than round numbers; `estimator_bias_p95` (CTX-2's ceil(chars/4) against a real
  tokenizer, declared model-specific) and `staleness_p50_permille` (MEM-6's unvalidated
  heuristic) are measured and reported, gated by nothing on the first run; the deterministic
  gate runs on the pull-request path — a Postgres-backed eval job in ci.yml that fails the
  merge on a breach, with the other jobs left database-free; a real composition change that
  degrades quality (a department's budget_tokens narrowed through the governed pack path, on
  a fresh tenant per phase) fails the gate naming the axis, the baseline, the measurement and
  the delta, and the scope tier that fell says which end of the gradient paid for it; nightly
  workflow; demo script. Deferred with a recorded trigger: the probe-based compression eval,
  because CTX-6 is Phase 3 and unbuilt — an axis for it would be permanently absent, which
  the harness treats as a coverage breach, or permanently zero, which reads as coverage.
  Written 2026-07-31 (EVAL-4, ADR-0047): the feature text named three clauses, one of which
  has no product to measure, and an AC with no axis in it. The load-bearing parts are the
  lens — the block, which EVAL-2 rejected for exactly the properties that make it right
  here — and the discovery that a per-scope corpus has to be promoted rather than placed.
EVAL-5 Security evals (M)
  Policy-leak suite (restricted content never crosses sensitivity/scope under 10k generated
  query variants); cross-tenant fuzz (TEN-6); prompt-injection-via-memory suite (a memory
  containing instructions must not alter agent behaviour when injected — content is data,
  wrapped and labelled).
  AC: one security corpus under evals/fixtures/security/ in which every (record, reader) pair
  declares readable or forbidden, refused at parse time when a pair is undeclared or declared
  twice, because an undeclared pair is an unmeasured boundary and a security suite that skips
  one silently is the failure mode it exists to prevent; the corpus is governed into place
  rather than seeded — material enters at its author's leaf through /v1/observe, climbs through
  POST /v1/proposals and each level's real approvers, and reaches `restricted` through a
  classify proposal the author opens at their own home scope and two distinct approvers sign,
  one of them holding `compliance`, because that is the only mechanism in the product that
  mints the tier; `make eval-security` asks every reader every generated variant over both
  query-shaped read surfaces — POST /v1/inject and POST /v1/recall's query form — and asks each
  reader the sweep form and the ids form naming every record it must not have, since recall's
  universe is wider than inject's by design (ADR-0024) and the ids form needs no retrieval to
  succeed and only a refusal to fail; `security_leaks_sensitivity`, `security_leaks_scope` and
  `security_leaks_tenant` are COUNTS gated at zero and never rates, because a rate divides a
  leak by a denominator the run chooses and three decimal places then round one leak in ten
  thousand to zero; `security_probes` and `security_variants` are gated with FLOORS, 10k
  variants being that floor on the nightly, because a one-sided gate with a free denominator
  passes by measuring less and nothing in the report would look wrong; `security_controls` is
  gated at 1.0 — every declared-readable pair actually reaching its reader — so a run of zeros
  is a measurement rather than an empty corpus, a dead pipeline or an expired bearer; a leak is
  graded by record identity AND by distinctive phrase and a disagreement between them is
  reported as its own defect, because a block whose text carries material its watermark does
  not name is not the same failure as one that served the wrong record; the cross-tenant half
  runs against a second admitted tenant with its own hierarchy, actors and corpus, and TEN-6's
  remaining scope — the store seam TEN-2 already fuzzes, and graph traversal, which has no
  caller-facing surface until GRPH-3 — is recorded rather than left to be rediscovered; the
  prompt-injection half is `security_unattributed_lines` gated at zero, the invariant that every
  non-empty line of a composed block is the preamble, a section header, the index legend, the
  watermark or an entry line and that the entry lines number exactly `record_ids.len()`, so a
  record's content cannot forge a scope header, an entry no record backs, a marker on a line of
  its own or a watermark — which takes a renderer that folds whitespace in rendered content
  rather than an extractor that happens to, plus the preamble line that says the entries are
  recorded material and not instructions, labelled in the ADR as a mitigation addressed to the
  guest rather than counted as a control; `security_marker_echoes` (content reproducing
  ` [confidential]` or `(recall <id>)` inline, with no newline needed) is measured and gated by
  nothing on the first run; a real, governed relaxation that opens a disclosure — a lapse
  proposed on the disclosing side, approved by two distinct stewards and time-boxed, granting a
  sibling team read of the vault team's material, on a fresh tenant — fails the gate naming the
  axis, the baseline, the measurement and the delta, while `security_leaks_sensitivity` and
  `security_leaks_tenant` hold at zero on the same run and hold for two DIFFERENT reasons: the
  confidential record is withheld by the grant's own declared tier ceiling, and the `restricted`
  one by something no grant can reach at all, since it lives at a personal leaf and the base
  layer's one permit carries `resource.kind != "user"`; a lapse rather than a pack flip because
  a pack cannot open a sibling team's material into anybody's block — the candidate universe is
  the caller's placement chain and widens by lapse and by nothing else (ADR-0037 decision 13),
  which is a good product property and a demo that would have proved nothing; nightly at the full variant budget against
  evals/baseline-security.json, and a deterministic every-k-th slice on the pull-request path
  against evals/baseline.json, because a product that blocks a merge on a token count and not on
  a disclosure has recorded its priorities backwards; demo script. Deferred with a recorded
  trigger: the behavioural half of the injection suite — whether a model reading the block obeys
  an instruction inside it — because it measures a joint property of the product's framing and
  one model's susceptibility and would fail when a model changed rather than when the code did;
  it rides the model-backed judge EVAL-3 must build and ADR-0046 option 6 already deferred.
  Written 2026-07-31 (EVAL-5, ADR-0048): the feature text named three suites and four words of
  AC. The load-bearing parts are the shape of the gate — counts and floors, because zero
  tolerance over a rate is a gate that rounds and over a free denominator is a gate that passes
  by measuring less — and the finding that the block's structure was forgeable by the content it
  carries, held back only by one extractor's whitespace handling.
EVAL-6 Load & latency suite (M)
  k6/vegeta profiles for inject/observe/recall SLOs at SMB and enterprise shapes.
  AC: SLO report per release.
EVAL-7 A second public benchmark (M)
  Filed 2026-08-07 by EVAL-3/ADR-0061 decision 1. LoCoMo was named in EVAL-3 until that ADR
  read its licence: snap-research/locomo's LICENSE.txt is Creative Commons
  Attribution-NonCommercial 4.0 International, which grants rights "for NonCommercial
  purposes only" and defines NonCommercial as not "primarily intended for or directed
  towards commercial advantage" — which is precisely what EVAL-3's own AC calls the score
  ("Marketing artefact too"). Nothing in the build would have caught it: CLAUDE.md's licence
  rule covers the core path and cargo-deny enforces it over crates, and a corpus is data.
  ADR-0061's compliance note closes that gap with `make check-corpus-licences`; this feature
  is the corpus it cost us. Two paths, either sufficient: written permission from Snap
  Research for commercial benchmark use, recorded beside the corpus — or a
  permissively-licensed substitute in LoCoMo's slot, which needs finding and licence-checking
  before this feature can commit to one.
  AC: a second published benchmark score under EVAL-3's two-tier discipline, arriving in
  EVAL-3's corpus format or with the reason it cannot recorded (ADR-0047 trigger (f),
  inherited rather than escaped); its licence permits the use, and the permission is in the
  repository rather than in somebody's memory of an email.

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
ADPT-6 LlamaIndex memory adapter (M) [Phase 4]
  Synveda behind LlamaIndex's memory and retriever interfaces; governed recall as a
  retriever, writes host-owned (ADR-0057 decision 6) so the same turn is not also
  stored by ADPT-2's tool. AC: example app persists and recalls across sessions, and
  a run with the adapter and the MCP server both configured writes each turn once.
ADPT-7 Semantic Kernel memory connector (M) [Phase 4]
  Synveda behind SK's memory abstraction and recall as a plugin; .NET and Python
  surfaces over one governed corpus, writes host-owned on ADPT-6's rule.
  AC: example app persists and recalls across sessions on both surfaces, and the
  two answer the same corpus for one identity.
ADPT-8 Observation that survives a session that does not wait (M) [Phase 4]
  Filed 2026-08-13 by testing ADPT-1's plugin in a real Claude Code session on
  v0.1.3. The measured defect was a headless `claude -p` which injected and
  never observed because every write hook was async. Delivered 2026-08-24 by
  CPR-14: Stop and PreCompact synchronously cross only the atomic local-spool
  boundary and return before credentials or network, while SessionEnd, the next
  SessionStart and explicit flush own delivery. An installed authenticated
  Claude Code 2.1.241 run proved one context run, four ordered authentic
  user/tool/assistant events, normal close and a verifying session audit chain;
  Stop took 8ms and made no event request. AC: the real headless session's
  activity reaches the chain without gateway latency entering an interactive
  turn; the host-killed-before-any-hook tail remains stated.

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
OPS-7  Gateway horizontal scale (L)
  Filed 2026-08-10 by OPS-2/ADR-0062 decision 5, which found that the chart could not
  honestly offer a second gateway replica. Most of what one needs is already there and in
  the database: the audit chain appends under a `for update` on `audit_chain_heads`, the
  promotion sweep takes `watermark_for_update` against exactly two sweepers, the lapse
  sweep's stamp is an idempotency key, PGMQ's archive-lock makes racing extraction
  consumers safe, and console sessions are a table. Two things are not. `LoginFlow` parks
  pending logins and CLI handoff codes in memory and says "single-replica only until
  OPS-2" in its own module doc, so a callback landing on another pod is a 401 for a login
  the IdP completed. `ScopeChainCache` is invalidated in-process, tenant-wide, with no TTL
  and no eviction, so a hierarchy move handled by one replica leaves every other replica
  composing against the ancestry the mover left — indefinitely, and looking exactly like a
  policy decision rather than a stale one. Three parts: the login and handoff store moves
  to Postgres beside the console sessions already there; scope-chain invalidation gets a
  cross-process transport (LISTEN/NOTIFY, or a generation column polled beside the pack
  refresher that already polls); and the writing loops get a ruling — they are safe on
  every replica, but N replicas is N times the sweep load on one database. Until this
  lands, ADR-0062 pins the chart at one replica and refuses an override.
  AC: a kind-cluster test at three replicas — a login that completes across pods, and a
  hierarchy move visible to every replica's composition within a stated bound — plus the
  retention sweep's concurrency verified rather than assumed.
OPS-8  Release & distribution (M)
  Filed 2026-08-11. OPS-1 built an installer that cannot leave this laptop and said so in
  its own source three times: `init` resolves its compose file relative to the working
  directory and errors "run `synveda init` from a Synveda checkout"; `gateway_binary()`
  looks only in `target/`, under a comment reading "a release ships this binary"; and
  `repo_root()` carries the other half — "a released binary would carry its own profile".
  Nothing is published: both our images build from source at install time, there is no
  release job and no tag, and the Helm chart names two images nobody outside this laptop
  can pull. A tagged GitHub Release with public GHCR images fixes all of it — prebuilt
  `synveda` and `synveda-gateway` for darwin-arm64 and linux-x86_64, the console bundle,
  a self-contained profile bundle under `deploy/release/`, and one `curl | sh` — so the
  prerequisite list drops to Docker. It ships binaries *and* images rather than images
  alone because ADR-0055 decision 8 forecloses the Docker-only shape: the bundled Rauthy's
  issuer is a `localhost` URL, RFC 6761 makes that the container itself, and the default
  install therefore runs the gateway as a host process by measurement rather than by
  preference. It also closes a CNSL-1 gap nobody had reason to notice — the host-gateway
  path never set `SYNVEDA_CONSOLE_DIR`, so `/console/` has 404'd for everyone without a
  checkout and a `pnpm` build. No Windows, no upgrade path, no package manager, no code
  signing; each has a reversal trigger in ADR-0065 rather than a plan.
  AC: on a scratch HOME with no checkout and no Rust toolchain, install → `init --demo` →
  `login` → a governed recall inside OPS-1's ten-minute budget with the image pull in the
  clock; installed from the packaged bundle rather than the tree, so a drifted bundle
  fails; OPS-1's break-glass invariant re-asserted from the installed path; `/console/`
  serving; and an unsupported platform refused by name rather than guessed at.
OPS-9  Beta demo profile (L)
  Filed 2026-08-13. OPS-8 made the product installable by somebody else and stopped one
  step short of being *showable* to them: `init --demo` seeds four people into the bundled
  IdP and then **prints** the commands that would build ACME's scopes, so a tester logs in
  to an empty product — no scopes, no memory, no proposals, an empty console — and the
  governance story that is this product's whole differentiator is invisible until they
  hand-run a dozen verbs nobody handed them. The printing is not a defect: ADR-0055
  decisions 1 and 2 refuse to let an installer create governed objects under a break-glass
  actor, and an installer that seeded an org would show the hierarchy under the wrong
  subject. So the seeding moves to where it can be governed rather than being abolished —
  `synveda demo seed`, run *after* login, under the operator's own bearer, through the same
  routes the CLI drives, which is the one shape that produces a living organisation without
  carving an exception into the invariant that produced the emptiness. It ships in the
  binary rather than as a `demos/` script because the audience is exactly the person who
  installed by `curl | sh` and has no checkout, which is also why the 60 acceptance scripts
  in `demos/` cannot serve here: each seeds its own scratch state, tears it down, and none
  of them is a tour. What it seeds is a *living* org — scopes, packs in deliberate contrast,
  role bindings so a demo person's first login lands somewhere with authority, a memory
  corpus, a published skill with channel history, a proposal left pending so the console
  inbox has something in it, and an active lapse — with the audit chain falling out of the
  seeding rather than being seeded. Content that needs a second author is written by service
  identities (AUTH-3) rather than by impersonating the demo people, because login is
  code+PKCE and turning on ROPC to let a seeder log in as Alice would weaken authentication
  for a demo convenience. Carries the beta's honesty with it: `docs/BETA.md` is the tour and
  the limits in one file, so the thing a tester is invited to try and the list of what does
  not work cannot drift apart.
  AC: on a scratch HOME with no checkout and no Rust toolchain, install → `init --demo` →
  `login` → `demo seed` → a recall that returns seeded memory, a console a tester **signs
  in to** rather than merely 200s, an inbox holding the pending proposal, and a chain that
  verifies with exactly one break-glass event; `demo seed` twice changes nothing on the
  second run and refuses a tenant that was not demo-marked; and the MCP client registry's
  extension point is demonstrated by configuring a client the release has never heard of.
OPS-10 Uninstall & cleanup (M)
  Filed 2026-08-13. OPS-8 made the product installable by a stranger and gave them no way
  to remove it: there is no `uninstall.sh`, no `mcp uninstall`, no `plugin uninstall`, and
  nothing anywhere in the repository matches the word. A beta asks people to try something
  on their own machine, so "how do I get rid of this" is a question the product has to
  answer before it is asked, and answering it in prose is not the same as answering it in
  a script — the footprint spans three tiers with different owners. What `install.sh`
  wrote is ours and removable exactly (`$SYNVEDA_HOME/{bin,console,profile,plugin}` and
  the CLI, wherever the sudo fallback put it). What `init` created is a **deployment** —
  containers and four named volumes — and destroying those volumes is the only way to
  remove a tenant's memory, because TEN-5 means a tenant row cannot be deleted; so the
  data is the one thing uninstall must never take by default, and the flag that takes it
  says what it is. And what the operator later asked us to write into *somebody else's*
  config — a `synveda` key in Claude Desktop, Cursor, Zed, VS Code, Windsurf or Continue,
  and a plugin inside Claude Code's own cache — is theirs, so removal is the exact mirror
  of `mcp install`: take out the one key we own and write every other byte back as found.
  That mirror is the feature's real content, because a shell script cannot do it: it is
  the same CST splice, and it belongs in the CLI beside the verb that made the mess.
  AC: after `uninstall.sh`, nothing of ours is on PATH or under `$SYNVEDA_HOME` and no
  container of ours is running, asserted from a scratch HOME; the data volumes survive by
  default and are named in the output, with `--purge` removing them and saying that memory
  is what it removed; `synveda mcp uninstall --client <c>` removes exactly the `synveda`
  entry and leaves an adjacent server, comments and layout byte-identical; `plugin
  uninstall` leaves `claude plugin list` without ours; and the whole thing is idempotent —
  a second run finds nothing and exits 0 rather than failing on what it already removed.

──────────────────────────────────────────────
EPIC CNSL — Admin console (Phase 3)
──────────────────────────────────────────────
CNSL-1 Proposals inbox (hero screen) (L) — review queue with diffs, scan reports, quality
  scores, evidence; approve/reject. AC: full review parity with CLI.
CNSL-2 Hierarchy & policy explorer (M) — visualise scopes, packs, roles, active lapses.
  AC: the four nouns are answered for a node in one screen and by the CLI beside it, because
  ADR-0056 decision 9 — the console gets no endpoint the CLI does not have — is a standing
  decision and `synveda policy` has no read verb while `lapse` is not a verb at all, so the
  machinery that is this product's whole answer to "strict by default, relaxable by design"
  has no terminal in which to ask what is currently relaxed; a pack renders with **where it
  came from** — assigned here, assigned at a named ancestor, the tenant default, the embedded
  default — and roles render the same way, the effective set with each binding's origin node,
  which is the asymmetry this feature closes: policy has served an origin since AUTHZ-2 and
  roles have served only the bindings at the node asked about, so the inheritance every reader
  needs was a walk each client did for itself and the two admin planes disagreed about how to
  say "this came from above"; "active lapses" is answerable **without already knowing which
  scope to ask about**, the scope-free list returning the standing set the caller may see with
  each lapse visible from **either** end under `PolicyRead` at that end rather than under a
  tenant-wide grant nothing below an org-admin holds, which is what lets the steward of a
  granted scope list — and therefore revoke — a grant their own team holds, where `at_target`
  had made a standing grant visible only to the side that disclosed it; the standing set is
  `active_for_scopes`' own predicate on the database's own clock, so the screen and the PDP
  cannot disagree about which grants are live, while the scoped form keeps returning expired
  and revoked rows because "who could read what, when" is a question about history; **what the
  reader may do is the PDP's own verdict and never a re-derivation of it** — the answer carries
  the pack `name@version` it was decided under, is asserted to move when a lapse opens and when
  a pack is assigned, neither of which a role-derived answer can express, and is proved to be a
  **forecast rather than a grant** by a probe that answers yes, a pack change, and the same act
  then refused at its own seam, since the enforcement point is unchanged and a client may decide
  what to offer and never what to allow; the probe answers about the caller and takes no
  `subject`, so an explorer cannot become an enumeration oracle for an organisation's role
  assignments one 403 at a time, and "who holds what here" keeps `RoleRead` and its own denial;
  a **10,000-node** hierarchy (HIER-1's own AC) renders without fetching a subtree or probing a
  node nobody looked at — children on expand, capabilities batched for the rendered set under a
  maximum the API declares, with the response naming what it did not answer rather than
  truncating silently — and the whole render chains **one** `authz.decision` per probe request
  with the pairs summarised rather than one row per (node, action), which is ADR-0019 decision
  4's second sentence (CTX-2's per-candidate sweep aggregating into the request-level event)
  arriving on the admin plane, asserted as a count on the chain rather than as a claim, because
  a governance product whose audit log is mostly a record of people looking at it has made its
  own chain unreadable; CNSL-1's deferral closes where it was sent — the inbox offers the acts
  the reader may actually take, and a reader holding one role short is shown a refusal rather
  than a button that returns one; the parity corpus takes four new cases — effective roles with
  mixed origins, a pack inherited from two levels up, a standing lapse beside an expired one,
  and a capability set with at least one denial — asserted by both renderers and checked for
  teeth the way CNSL-1's was, by deleting each fact and naming which case fails and which do
  not; every act is on the chain, with no third party's binding and no lapse reason in any probe
  payload, swept for; demo script. Deferred with a recorded trigger: **policy simulation** —
  "what would this scope compose under `standard`" — because a forecast against a pack nobody
  assigned is a second decision path through the PDP and the honest version decides against a
  supplied pack rather than the effective one; and **mutation from the explorer**, because
  assigning a pack and binding a role are direct routes where content is proposal-gated, and a
  second direct mutation surface would settle by accident a question that is AUTHZ-7's to answer
  on purpose — this feature's own finding, that all three packs let one steward replace a
  subtree's pack with one call and one signature while the lapse relaxing far less needs a
  reasoned, time-boxed, dual-approved proposal.
  Written 2026-08-05 (CNSL-2, ADR-0058): the feature text named four nouns and no criteria.
  Three of the four already had surfaces, so the load-bearing parts are the two that did not
  answer the question actually being asked — a role that will not say where it came from, and a
  lapse that can only be found from the side that granted it — plus the discovery that the
  reader's own capabilities cannot be derived client-side at all without producing a second
  implementation of "may I" that disagrees with the PDP immediately rather than eventually.
CNSL-3 Audit explorer (M) — AUD-2 surfaced; "what did agent know at T" as a UI query.
CNSL-4 Knowledge browser (M) — delivered and subsumed by CPR-17's generated-contract
  Knowledge Browser. Scope-filtered immutable revisions expose independently authorised
  provenance and validity/history; create/edit/verify/merge/supersede/archive/restore/forget
  are typed VedaFlow changes. The old raw-record and channel pin/retire nouns are deleted.
  AC: no direct-mutation path exists and no hand-written console contract exists.

──────────────────────────────────────────────
EPIC CPR — Context platform redesign (Phase 5)
──────────────────────────────────────────────
CPR-1  Implementation baseline & locked decisions (M)
  Filed 2026-08-17. The first prompt of a 33-prompt programme that re-cuts this product for
  an individual and a small team without producing a second one. It writes down what the
  repository *is* at the commit the programme starts from, records the eight decisions the
  programme may not reopen, and changes no runtime behaviour. Two things have to exist
  before a hard cut starts and neither is code. The first is an **inventory**: sixty-five
  delivered features is more shape than anybody holds in their head, and a redesign that has
  not written down what it is cutting discovers the parts it forgot one compile error at a
  time — so the record is exhaustive and boring on purpose, every route, verb, table, RLS
  policy, Cedar action and console screen, plus the part that earned its place, each
  adapter's *actual* verification level, which for three of them is lower than the feature
  list implies (a headless Claude Code session never observes; no real Cursor frame has ever
  been replayed; the SCIM corpus is transcribed from published tables rather than taken from
  a live tenant). The second is a **lock**: eight decisions later prompts implement and may
  not relitigate, because every one of them is the kind that gets quietly reversed under
  pressure at prompt 19 — one domain model rather than two, policy profiles rather than
  edition branches, a fresh schema epoch rather than a migrator, generic governed scopes
  rather than five organisational ranks, sessions as a real aggregate rather than a
  correlation string, the candidate/knowledge boundary as a table boundary rather than a
  column, immutable knowledge/skill/tool versions, and OKF and MCP as external-format
  adapters at the boundary. They are ADR-0068, which argues each against the cheaper option
  it refuses. The baseline also states the **MVP checkpoint** — what must be true after
  Prompt 20 — so "are we there yet" is a list somebody reads rather than a judgement
  somebody makes. It deliberately does not file the other 32 prompts as features: each is
  filed by the prompt that runs it, which is how this backlog stays a record of what was
  found rather than a forecast.
  AC: docs/implementation/synveda-context-platform.md records the base commit SHA, CI status,
  migration head, the public HTTP route inventory, the CLI command inventory, the console
  route and navigation inventory, domain entities and tenant-bound tables, the RLS-protected
  table inventory, the Cedar/PDP entity and action model, the observe/inject/recall paths,
  the hierarchy and role-binding implementation, the record/proposal/quarantine/skill/
  context-pack models, client adapters with their actual verification level, an explicit
  deletion map from old concepts to target concepts, the ordered programme of Prompts 1–33,
  and the MVP checkpoint after Prompt 20; ADR-0068 records the eight decisions with options
  and a reversal trigger; the complete suite is run and its result recorded accurately with
  pre-existing failures named; and **no product runtime behaviour changes** — the diff is
  documentation only and `make ci` is green after it as it was before.
CPR-2  Fresh schema epoch, startup guard & local reset (M)
  Filed 2026-08-17 by Prompt 2 of the CPR programme. ADR-0068 decision 3 committed to a
  fresh schema epoch with no old-data migration and said what must follow: a database
  carrying the old epoch is **rejected at startup with a reset instruction**. This is the
  mechanism, and it exists because the decision as written had three holes. Nothing in the
  database says which model its rows are in — 38 migrations and no marker of any kind, so
  a binary pointed at an old database cannot tell it from a new one and `MIGRATOR.run`
  will happily bring it forward, which is the silent acceptance the decision forbids,
  available by running the command the documentation already tells operators to run.
  `_sqlx_migrations` is not that marker: it answers "which of *this binary's* migrations
  have run", a question about the current chain, which Prompt 33's squash makes
  meaningless as evidence and which moves on every ordinary release. And there was no
  reset — `uninstall.sh --purge` destroys the Docker volumes, which is both too much
  (Temporal's two databases share `pg-data`) and the wrong shape, so the instruction the
  guard is meant to print named nothing. CPR-2 adds `schema_metadata` (epoch, migration
  head, creation time, the product version that created it), a preflight that refuses to
  migrate a pre-cut database *before* the migrator touches it, a boot guard the gateway
  refuses to start past, the same check on `/readyz` because the gateway may start without
  a database, and `synveda reset --database --force` — which drops and recreates the
  application database, not the volume and not the installation. ADR-0069.
  AC: a fresh empty database bootstraps to the current epoch and the marker records the
  epoch, the migration head, the moment and the release; a current-epoch database starts
  normally and re-migrating keeps its provenance; a database from before the cut is refused
  by the gateway at startup, by `/readyz` and by the migrator, which writes nothing — the
  rows it refused are still exactly there; missing and malformed markers are refused;
  every refusal prints `synveda reset --database --force` verbatim, and the one refusal
  that must not (a database from a *newer* build) says to upgrade instead; `reset` requires
  both flags, refuses a database that is not on this machine, builds a working current-epoch
  database, carries **zero** rows across, is idempotent, and preserves the KEK, the profile,
  the console bundle, stored logins and every other database on the server; and no
  old-to-new data migrator exists — asserted structurally (the epoch migration is pure DDL,
  no `.down.sql`) as well as behaviourally.
CPR-3  Generic governed scope substrate (M)
  Filed 2026-08-17 by Prompt 3 of the CPR programme. ADR-0068 decision 4 said generic scopes
  replace fixed organisational ranks; this builds the substrate, with no public API on it
  yet. `scopes` + `scope_closure`: a named node with a parent and a subtree, tenant-bound,
  forced RLS, closure maintained transactionally by explicit store SQL. Five **shapes** —
  `tenant`, `org_unit`, `workspace`, `project`, `principal` — where the only thing a shape
  decides is which shapes may be its parent, so `org_unit` nests inside itself to arbitrary
  depth and a person's whole deployment is a tenant scope and a principal. The rank is what
  goes: no `rank()`, no strictly-increasing ladder, no root-must-be-an-org, nothing anywhere
  comparing two kinds for order. The judgement worth reading is *where each rule lives*
  (ADR-0070 decision 2): every structural rule that can be a database fact is one, so the
  placement rule rides a composite foreign key over a denormalised `parent_kind` rather than
  a store-side check the next caller can go around, a cycle is refused by a CHECK on the
  closure's own self-row rather than only by the descendant test a move makes first, and
  cross-tenant immobility holds for the owner role — migrations, break-glass psql, a restore
  — and not only for the role RLS binds. Internal services for create, rename, move,
  ancestors, descendants, tenant root and path resolution; no route, no CLI command, no
  adapter, and no PDP call inside the store (the governed entry points attach at the API
  boundary later prompts add). The old hierarchy is untouched and **nothing is synchronised
  with it**: no row of `hierarchy_nodes` becomes a row of `scopes`, and Prompt 6 deletes it
  whole. ADR-0070.
  AC: a scope tree is created, read and moved through the services with the closure agreeing
  with the adjacency after every operation; the placement rule holds for every pair of the
  five kinds, asserted as a matrix rather than as cases; a parentless non-tenant scope, a
  second tenant root, a nested tenant scope, a duplicate sibling slug, a malformed slug,
  display name or attribute bag, and a placement the tree does not admit are each refused
  with the rule they broke; org units nest to arbitrary depth and a workspace and project
  still hang off the deepest one; a scope's path resolves back to that scope and a path that
  names nothing resolves to nothing while a malformed one is an error; every read is
  tenant-filtered in SQL as well as by RLS, so another tenant's scope reads as absent rather
  than forbidden, on every surface including `move`'s destination; a scope cannot move across
  tenants and the database refuses it to the owner role too, with slug, kind and provenance
  immutable beside it; cycles are impossible — refused as an error by the service and
  unrepresentable in the closure; the closure survives randomly generated operation
  histories, checked against a recomputation from the adjacency after every step; concurrent
  writers behave — two creates racing one sibling slug admit exactly one, two moves of one
  scope serialise, and a create landing inside a moving subtree waits and inherits the
  ancestry the move left; and both tables join the adversarial RLS suite's completeness
  inventory with a wrong-GUC read seeing nothing, a cross-tenant write rejected, the whole
  lifecycle working as `synveda_app`, and the app role holding no DELETE.
CPR-4  Workspaces, projects & canonical repository identity (L)
  Filed 2026-08-17 by Prompt 4 of the CPR programme. CPR-3 built the scope substrate and gave
  it no API on purpose; two of its five shapes — `workspace` and `project` — were named by
  the vocabulary and had nothing behind them. This puts something behind them, and it is the
  programme's first public surface. `workspaces`, `projects` and `project_repositories` as
  **product-level subtypes of a governed scope**: a workspace owns one workspace-shaped scope
  under the tenant root, a project owns one project-shaped scope under its workspace's, and
  both are created **in the same transaction as their scope**, so the outcomes are both and
  neither — there is no compensating delete anywhere. The tenant root is minted by the first
  thing that needs a parent, from the `tenants` row and never from the old hierarchy, so a
  person's first act is `POST /v1/workspaces` and nobody is asked to declare an organisation.
  One row shape for one person and for a bank: no personal/team tables and no mode branch
  (ADR-0068 decision 1). The rules holding the subtype and its scope together are foreign
  keys rather than service code, ADR-0070 decision 2 applied one level up — the scope's shape,
  the project's scope sitting under its workspace's, and the fact that a subtype's slug **is**
  its scope's slug, so a product path and a scope path cannot diverge. Twelve routes: `GET
  /v1/me` (the client's first call — principal, tenant, accessible workspaces and projects,
  effective capabilities, and an onboarding state the **server** computes rather than each
  client inferring from an empty list), the workspace and project CRUD, and repository
  attach/list/detach. Creation takes a **required** `Idempotency-Key` and update a **required**
  `expected_revision`, because this surface's first callers are retrying agents rather than
  people clicking buttons. Repository identity is canonical: transports, credentials, ports
  and `.git` collapse, so four ways of writing one repository are one row — and a filesystem
  path is **refused by name**, in the type layer with a message and in a CHECK constraint
  behind it, because a path differs per machine and changes when somebody moves a directory.
  A repository with no remote gets a `git+fingerprint:<hex>` identity from a stable content id
  the client computes. Six new Cedar actions, in all three packs; decisions are anchored at
  the tenant until Prompt 5 re-cuts the PDP over generic scopes, which is the largest thing
  this feature defers and is stated rather than implied. And the product gets its **first
  OpenAPI contract**, derived from the handlers rather than written beside them, with the
  console's TypeScript generated from it. ADR-0071.
  AC: creating a workspace creates its scope in one transaction under the tenant root, and a
  project's under its workspace's, with the tenant root minted on the way past — and a failed
  creation leaves **neither** an orphan subtype nor an orphan scope, asserted for both
  subtypes through the failure mode that fires after the scope insert; a subtype's slug and
  its scope's slug cannot disagree, a workspace cannot own a project-shaped scope, a project
  cannot move between workspaces and its scope cannot be moved out from under it — each
  refused against direct SQL, not only through the services; a revision cannot be rewound or
  skipped by anything holding a connection, a stale `expected_revision` is a 409 that writes
  nothing, and another tenant's subtype is a 404 rather than a revision oracle; an archived
  workspace takes no new projects and a status change is mirrored onto the owned scope both
  ways; a description can be set, cleared and left alone as three distinct requests; one
  repository written four ways is one attachment and the second is a conflict, a filesystem
  path is refused with a message naming what to send instead, a repository with no remote is
  identified by its fingerprint, a handle from one project cannot address another's, and two
  projects may be about the same repository; a creation replayed with the same key returns the
  original resource with 200 rather than creating a second, the same key with a different body
  is a 409, a concurrent duplicate replays rather than conflicting, and **the replay still
  takes the PDP decision**; every route denies without the action and chains its event, with
  an update's event carrying the precondition it was applied under; all four tables join the
  adversarial RLS suite's completeness inventory; and the OpenAPI document is derived from the
  handlers, every documented path is mounted, every mounted path on this plane is documented,
  and `console/src/generated/api.ts` is generated from the document with both checks in `make
  ci`.

CPR-5  Membership, groups, grants & invitations (L)
  Filed 2026-08-18 by Prompt 5 of the CPR programme. CPR-4 gave the platform workspaces and
  projects and left one thing conspicuously absent: **nobody is in them**. A workspace had a
  name, a scope and no members, and the only way anybody could act on it was a role binding
  on a node of the *old* hierarchy — a different tree Prompt 6 deletes whole. This is the
  membership model, and it is one model for a person working alone, four people sharing agent
  context, and a company with a directory (ADR-0068 decision 1): `groups`, `group_members`,
  `scope_grants` and `pending_invites`, where a grant gives a **subject** — a principal or a
  group — a **role key** at a scope, and the scope's subtree inherits it. Creating a workspace
  or a project mints an `owner` grant for its creator in the same transaction, because a
  collaboration space nobody is a member of is not one. The judgement worth reading is what is
  **not** here: there is no permission table, and there must not be one (ADR-0072 decision 2).
  Six role keys — `owner`, `member`, `viewer`, `reviewer`, `curator`, `administrator` — and
  nothing anywhere says what any of them may do, because the Cedar packs decide that and a
  second mapping would be a second decision point that disagrees with the first the day
  somebody edits one. Inheritance is the scope tree rather than a fan-out: a workspace grant
  reaches every project inside it through `scope_closure` at read time, with **no per-project
  row** to keep consistent. The one place it stops is a `principal`-shaped scope, which is
  somebody's own — no ancestor reaches in, not the tenant root and not a workspace owner.
  A principal is a **token subject** rather than an identity row, for ADR-0015 decision 2's
  reason and one sharper: an `identities` row in this tree still needs a `hierarchy_nodes`
  node, so a membership model keyed on it would need the model it replaces. Invitations are
  how a small team actually onboards: an expiring, one-time, revocable token minted and hashed
  exactly like the provisioning credential (ADR-0059 decision 13's shape), returned **once**
  with a copyable URL, and redeemed with the recipient's *own* credential — no email delivery
  anywhere in the product. Fourteen routes; four new Cedar actions, and the packs grade
  membership reads differently on purpose. Every decision is still anchored at the tenant, and
  **grants are not yet a PDP input** — the largest thing this feature defers, and stated
  rather than implied. ADR-0072.
  AC: creating a workspace or a project makes its creator the `owner` in the creating
  transaction, with the source no route hands out; a grant at a workspace is in force at every
  project inside it and **writes no row there**, and a project-only grant reaches neither its
  workspace nor a sibling; a `principal`-shaped scope inherits nothing, asserted against the
  widest grant the model can express; a grant to a group resolves to its members, following
  them as the group changes with no grant written, and an archived or empty group resolves to
  nobody; a grant has exactly one subject, is never edited, and an `invite`-sourced one names
  its invitation — each refused against direct SQL as well as through the services; a group's
  slug, source and provenance are immutable and its revision cannot be rewound or skipped by
  anything holding a connection; a stale `expected_revision` is a 409 that writes nothing,
  membership included; an invitation is one-time (a retry by the same principal replays, a
  second person is refused), expires **without anything running**, cannot be reopened after
  either terminal state, and its token is stored only as a 32-byte hash and appears in exactly
  one response — swept for in the audit chain and absent; a replayed invitation creation is a
  409 saying the token cannot be re-served rather than a 200 with it missing; redeeming needs
  the token rather than a role under every pack, while the invariant floor still refuses a
  quarantined principal and every service identity; removing a member touches only what was
  written at that scope and refuses inherited, group-derived and directory-managed authority
  with the place to go; every route denies without its action, refuses without a credential,
  chains its event, and a replay still takes the PDP decision; all four tables join the
  adversarial RLS suite's completeness inventory; and the OpenAPI document grows to
  twenty-six operations with the console's types regenerated from it.

CPR-6  Governed scope anchors: the PDP re-cut (L)
  Filed 2026-08-19 by Prompt 6 of the CPR programme. CPR-3, CPR-4 and CPR-5 each built a piece
  of the governed scope model and each recorded the same debt: **the decision point still
  described the old hierarchy**. Twenty-six routes anchored every decision at
  `Resource::Tenant` because a governed scope had no chain in the Cedar entity graph, and
  ADR-0072 decision 3 went further — the role keys a grant carries were stored, resolved and
  served, and *never reached Cedar at all*, because `context.roles` was the old hierarchy's
  binding vocabulary. So the product had a complete membership model that decided nothing: a
  workspace `owner` grant was a governed record of authority, and what actually let somebody
  administer a workspace was a role binding on the tree this programme is deleting. This is
  that cut. A **scope-anchor resolver** answers "where does this request stand" from six
  inputs — the caller's own scope, the selected project, the selected workspace, the
  organisation-unit relationships above them, the tenant root, and every scope a direct or
  group grant names — and returns an **ordered set**, most specific first, rather than the one
  fixed organisation chain the old model assumed. That assumption was wrong in two ways at
  once: a caller can stand in several places none of which contains the others, and a
  placement chain runs *upward* where a grant runs *downward*. The Cedar entity model is
  rewritten around seven entities — `Tenant`, `Scope`, `Principal`, `Group`, `ScopeGrant`,
  `Workspace`, `Project` — each of the four new ones parented to the scope it belongs to, so a
  decision names the thing it is about: a read names the workspace, a project creation names
  the workspace it would land in, and a revocation names **the grant**. `Principal.department`
  is deleted with the rank vocabulary it read, and `Scope.kind` carries the five shapes
  instead of the five rungs; a test asserts directly that nesting the same tree four levels
  deeper changes no verdict. **Personal principal-scope privacy** becomes a base-layer forbid
  no pack can drop — with a short, closed governance carve-out and, for the first time, a door:
  a grant written *directly at* somebody's own scope reaches it, so "share my own notes with
  you" is finally sayable. `GET /v1/me` forecasts what the caller may do **at each anchor**
  from real PDP decisions, never from a plan or an edition. And the SCIM boundary projects
  directory users and groups onto the same four tables a person working alone uses —
  principals, groups, group members, grants — with no enterprise membership table anywhere.
  The old hierarchy APIs stay until the prompt that deletes them, and they are no longer
  *required* by PDP evaluation: they project their rows into the decision point's one scope
  vocabulary at the caller's edge. ADR-0073.
  AC: an anchor set is ordered most-specific-first, merges one scope into one anchor however
  many ways it became applicable, and orders by structure rather than by rank — nesting the
  same tree deeper changes no verdict, asserted over every probed action; a workspace grant is
  in force at that workspace's projects with no row written there and reaches neither a
  sibling workspace nor the tenant; a **project-only** grant reaches the project and refuses
  every read, update and administration of the workspace above it; a grant naming a group
  reaches its members, and membership of a group with no grant naming it confers nothing;
  revoking a grant refuses the very next decision with nothing invalidated, and revoking is
  itself a decision that names the grant; a profile assigned at an organisation unit governs
  everything beneath it however deep, and a grant written there reaches the same subtree;
  **nobody reaches into somebody else's own scope** — not a tenant-root owner, under no pack,
  at no tier, for content or membership — while their own scope is theirs and a grant written
  directly at it reaches it; a foreign tenant's chain, anchor and entity grant nothing, and a
  chain spliced across two tenants launders nothing; the capability block for an anchor is the
  set of decisions it forecasts, moves with the grant and the profile and with nothing else,
  and forecasts nothing at all for somebody holding nothing; and `/v1/me` mints the caller's
  own scope, serves its anchors and names how many the bound dropped.

CPR-7  The hierarchy cutover: one scope tree (XL)
  Filed 2026-08-20 by Prompt 7 of the CPR programme. Six prompts built the governed
  scope model — substrate, workspaces, membership, the re-cut decision point — while the
  old fixed hierarchy stood beside it, explicitly untouched until "the prompt that deletes
  it whole" (ADR-0070, ADR-0073's records). This is that prompt. The old tree is deleted
  **whole**: `hierarchy_nodes`, `hierarchy_closure` and `role_bindings` leave the schema,
  the rank vocabulary (`org`/`division`/`department`/`team`/`user`, `rank()`, the
  child-outranks-parent rule, the root-must-be-an-org CHECK) leaves the types, and
  `/v1/hierarchy/*`, `synveda hierarchy`, `synveda role bind`, the placement-based
  quarantine convention, the `synveda-{dept}-{team}` JIT convention, `group_mappings`
  and the console's hierarchy explorer leave the product — replaced by six public
  admin routes over governed scopes (`/v1/admin/scopes` list/create/get/patch/ancestors/
  descendants), five operator CLI commands (`synveda scope list|show|create|move|tree`),
  and pack assignment plus the VedaFlow curator file re-homed under the same prefix. One
  decision-gathering path remains (the governed one); `context.roles` carries grant role
  keys only, and the old `Role` vocabulary — bindings, proposals' approval records,
  curator files, every Cedar role list — is deleted with the VedaFlow approval matrix
  re-vocabularied onto the six grant keys. Placement becomes identity: an identity's
  scope is its own principal scope, minted at first login for users, services and
  directory identities alike, and "unmapped" means *ungranted* rather than quarantined,
  decided per action by the anchor model and the base-layer privacy floor rather than
  per person by placement. The `synveda-admins` convention now mints an `administrator`
  grant at the tenant root — the operator door ADR-0073 recorded as missing. The schema
  epoch bumps so every pre-cutover database is refused with the reset instruction, and
  the migrations are rewritten in place (the scope substrate moves to `0004`) rather
  than translated. ADR-0074.
  AC: every `/v1/hierarchy` route answers 404 and every old scope kind
  (`org`, `division`, `department`, `team`, `user`) fails validation by name, asserted
  as negative API tests; the admin routes create, rename, archive and **move** scopes
  with each mutation PDP-decided against the scope it is about, audited with both ends
  of a move, and idempotent on creation; a move is refused into its own subtree and a
  cross-tenant move is unrepresentable; the memory plane (observe/inject/recall,
  channels, proposals, skills, prompts) decides over governed scope chains and grant
  role keys with the old chain cache and the old bindings gone; an identity's scope is
  a principal scope minted at first login — no hierarchy row, no quarantine node — and
  a first-time admin-group login mints the tenant's first grant without any break-glass
  step; pack assignment at a scope governs its subtree through `scope_closure`; the
  approval matrix, proposal approvals and curator files speak grant keys only; and a
  fresh database is the only database this build accepts.

CPR-8  The console product shell & first-run onboarding (L)
  Filed 2026-08-21 by Prompt 8 of the CPR programme. Six prompts built a context platform —
  governed scopes, workspaces, projects, membership, invitations, the re-cut decision point —
  and every one of them is reachable only from the CLI. The console still resolves its session
  with `whoami` and mounts the proposals inbox and the scope explorer one after the other with
  no navigation at all, which is the right first screen for somebody reviewing other people's
  publications and the wrong one for the person who just installed the product and has no
  proposals, one scope and no way to create a workspace. This replaces that entry point with a
  **route-based product shell**: a primary menu that is the product (Home, Sessions, Knowledge,
  New Learnings, Skills, Tools, People, Settings), shown to everybody unconditionally, and an
  **advanced** menu that is governance (Reviews, Scopes, Policies, Audit, Service identities),
  shown only where the caller's capability forecast offers the plane behind it. Beside it: a
  workspace and project switcher over a selection persisted per browser and reconciled against
  `/v1/me` on every load; route-level loading and error states from one query/cache layer; a
  typed client generated from the OpenAPI contract, which now also emits the runtime path table
  and a compile-time `Idempotency-Key` obligation on the eight operations whose document
  requires one; a **People** page answering *why* somebody may act here — workspace members,
  project-only members, pending and settled invitations, each row carrying its role, access
  source, the scope its grant is written at, the group it came through and whether a directory
  owns it, with invite/revoke/remove offered exactly where the API would accept them; and
  **first-run onboarding** — workspace, project, repository, agent client, connection
  instructions, connection check — whose personal/team question **seeds** a policy pack and a
  membership posture and records no edition anywhere, because ADR-0068 decision 1 forbids one
  and a wizard asking "is this just you?" is the friendliest door that branch could arrive
  through. The proposals inbox moves to Advanced ▸ Reviews and the scope explorer to
  Advanced ▸ Scopes, unchanged in substance. No npm dependency added: routing and the cache are
  written here, because the shipped bundle's licence gate has no exception mechanism and the
  page is served under `default-src 'none'`. ADR-0075.
  AC: the primary menu carries all eight items for every caller including one with an empty
  capability map and no primary route is gated; the advanced menu appears only where the
  forecast offers a plane, is absent heading-and-all for a caller with none, and carries
  exactly the planes offered; a guarded route reached directly explains the missing action
  rather than redirecting, and an unknown path renders a not-found page rather than Home; the
  selection survives a reload, falls back when it names a workspace the caller lost, drops a
  project belonging to another workspace, and degrades in a browser that stores nothing; every
  contract-covered call goes through the generated client, an operation the document marks
  idempotent cannot be sent without a key and one it does not mark cannot carry one; the People
  page splits workspace from project-only membership by each row's own `inherited` flag, names
  the group and the directory together where both are true, and offers remove only on a direct
  non-directory grant; onboarding walks its six steps, produces a seeding plan carrying no
  edition field, reports a refused seeding step with the plane that can finish it without
  blocking, and runs a connection check that passes without a repository, fails on an
  unreadable project and states what it cannot verify; a plane with no API says it is not built
  and what it waits on rather than rendering an empty list; and `make check-api-types` passes
  with no dependency added.

CPR-9  The foundation audit: hardening the scope and access cutover (M)
  Filed 2026-08-22 by Prompt 9 of the CPR programme, which was asked to audit Prompts 1–7
  rather than build on them. Eight prompts delivered a schema epoch, a governed scope tree,
  workspaces and projects, membership and invitations, a re-cut decision point and a console —
  each with its own green suite, and each suite proving that its own plane works. Nobody had
  yet asked the opposite question of all of them at once: what does a caller learn, or fail to
  learn, that their grants do not say? This is that pass. It adds an adversarial suite that
  probes every per-object route with **valid identifiers from another tenant** — real
  workspace, project, scope, group, grant and invitation ids, minted by that tenant's own
  administrator — and asserts each is not merely refused but **indistinguishable from an id
  nobody ever minted**, in status and in error kind, because a caller who can tell the two
  apart can enumerate another tenant's inventory a uuid at a time; probes a second workspace
  inside **one** tenant, where RLS cannot help and only the PDP and the anchor resolver stand;
  and probes somebody else's `principal` scope with the tenant administrator, the one caller
  who reaches everything else. It checks the three channels a denial leaks through when the
  status code is right — counts, error text and the navigation capabilities the console builds
  its menu from.
  Three defects it found, all of them cutover residue rather than design faults. **A grant at a
  workspace did not reach the listings**: `GET /v1/workspaces` and `/v1/me` took one decision
  at the tenant root and applied it to every row, so a caller granted `member` at a workspace —
  who holds nothing at the root — was served an empty list, a `workspace_count` of zero and an
  `onboarding.state` of `needs_workspace`, while the `anchors` block of that same response said
  `workspace.read: true` at that workspace. The listing and the navigation capabilities
  disagreed and the console renders both, so an invited member was sent to the first-run wizard
  to create the workspace they had just been added to. Listings now decide **per row against
  the row**, which is the decision the per-object route already took. And **two client/server
  contracts had drifted apart**: `synveda login` still required an `identity.quarantined` field
  CPR-7 deleted, so every login failed to parse the session it had just been handed, after the
  browser round trip and after the code exchange; `synveda whoami --capabilities` still read
  the `roles`/`role_assign` shape CPR-7 renamed and dropped, so the flag could not parse a
  single response. Both are hand-written DTOs on routes Prompt 19 has not put on the contract,
  and nothing on either side checked they still agreed.
  It also widens the no-data-migrator guard from the epoch migration to **the whole chain**,
  with function bodies skipped so an audit trigger is not mistaken for a translation, and pins
  the three inherited pre-epoch upgrade statements by name and by the reason each is
  unreachable — a fourth fails the build. No epoch bump: the three cannot run on any database
  the guard admits, and deleting them would trade a checksum error for the reset instruction on
  every existing deployment for no behavioural change. Prompt 33's squash removes them.
  AC: a valid workspace, project, scope, group, grant or invitation id from another tenant is a
  404 with the same error kind as a fictional one on every per-object route, and no listing,
  `/v1/me` field or onboarding tally names it; an invitation minted in one tenant cannot be
  redeemed from another **and survives the attempt**, still spendable by its rightful
  recipient; a member of one workspace sees exactly that workspace and its project in
  `/v1/me` and `GET /v1/workspaces`, with the two agreeing, and is refused the other workspace
  and its project; the capability probe answers a scope the caller holds nothing at with every
  verdict false and no `scope_path`, `pack` or role, and answers somebody else's `principal`
  scope the same way for a tenant administrator; a caller who holds nothing is answered rather
  than errored; `synveda login` and `synveda whoami --capabilities` parse what the gateway
  serves, pinned from both sides so neither can drift alone; every tenant-bound table is
  enabled + forced with a policy and the four hierarchy tables are absent; the scope closure
  carries a self row per scope, no cross-tenant pair, no cycle and a distance-1 edge per parent
  pointer; and no migration in the chain runs DML outside a function body but the three pinned
  statements.

CPR-10  The session ledger and runtime API (XL)
  Filed 2026-08-23 by Prompt 10 of the CPR programme, and the first of Stage B. ADR-0068
  decision 5 in one line: sessions are the root of agent runtime activity, and
  `session_id: String` as a correlation hint is deleted. What exists before this is that
  string — `observe_events.session_id`, an opaque harness identifier with a length CHECK and
  nothing else, copied into an audit payload by `/v1/inject` and `/v1/recall` and read back by
  nothing. Four things follow, and each is something this product claims to do and cannot: a
  run an agent only *read* in does not exist (ADPT-8 measured it — a headless Claude Code run,
  three sessions, three `inject.ok`, **zero** `observe.done`, exit 0); a run cannot be
  governed, because there is no resource for the PDP to decide about; a run cannot be
  retained, ended or audited; and the console cannot show one, which is why CPR-8 had to
  render a placeholder on the page it put first in the primary menu.
  What it adds: three tables — `sessions` (one run: its workspace, optionally its project,
  the **derived** governed scope it is decided at, who opened it, the client and version and
  installation, the harness's own id, the agent, the model, the repository and branch, a task
  summary, a five-state lifecycle and a bounded metadata bag), `session_events` (immutable,
  append-only, ordered, idempotent by the client's own `client_event_id`, over a closed
  twelve-name vocabulary spanning lifecycle, messages, tools, files, commands, skills, context
  requests and adapter warnings) and `session_context_runs` (one act of composing context for
  a run, with the rendered block and its watermark). Seven routes — open, list, get, append
  events, end, timeline, context-runs — all on the OpenAPI contract from the day they exist.
  Two Cedar actions (`SessionRead`, `SessionWrite`), a `Session` entity parented to the scope
  it runs at, permits in all three shipped packs (@18 → @19), four audit action types, and the
  console's Sessions page, which is the first of CPR-8's four planned pages to get a plane.
  Six decisions worth reading, all in ADR-0076. **The governed scope is derived, never
  submitted**: three columns and two composite foreign keys hold `scope_id =
  coalesce(project_scope_id, workspace_scope_id)` as a row-local fact, because a client that
  could name the scope could name one its workspace is not in. **Five states, because the
  close is two-phase**: an adapter learns a run is over at a hook that must return quickly and
  usually still has events buffered, so `ending` means *no new work, I am flushing* and still
  accepts them — and `abandoned` (nobody closed it) is kept apart from `failed` (it broke)
  because the two call for different things. **No revision and no `expected_revision`**: a
  precondition stops a lost update, and ending has one target state, so two concurrent ends
  are one transition and one refusal that already names the state the run is in. **Two
  idempotency mechanisms, not redundant**: opening and composing take the header; appending
  is idempotent per *event*, because a redelivered batch overlapping a previous one by three
  of ten must append seven and report `duplicate` for three, which a request-level key cannot
  express. **Nothing on the wire carries a tenant or an acting principal**, and a body naming
  one is refused rather than ignored — a server that silently dropped the field would behave
  correctly and teach every client author that it works. And **the timeline is a projection**
  over the two tables, never a third: a materialised transcript would be a second copy of
  `session_events` that disagrees the first time one is written and the other is not.
  The old observe/inject/recall routes are untouched, and nothing bridges or synchronises the
  two: Prompt 11 re-cuts the observe path onto sessions and deletes the string.
  AC: an agent opens a run whose governed scope is the project's, derived with no request
  naming it; a body naming `tenant_id`, `principal_id` or `scope_id` is a 400 and the stored
  principal is the token's; opening replays 200 on the same key, 409 on the same key with a
  different body, and 400 with no key; a redelivered batch appends only what is new, reports
  `duplicate` for the rest at their **original** positions, and a batch repeating an id inside
  itself is refused by name; `ending` still accepts buffered events while a closed run accepts
  none, never reopens and never changes how it closed — at the API and against direct SQL;
  `POST …/context-runs` composes through the existing retrieval engine and persists the
  identity and the block; the timeline merges both sources in one order and no timeline table
  exists; every route refuses a caller who holds nothing with a 403 naming the action; a
  member granted at one project sees that project's runs and no others, with the listing and
  the per-object route agreeing; another tenant's session id is a 404 with the same error kind
  as a fictional one; a tenant with no governed scopes is answered rather than errored; a
  session's `metadata` never reaches the audit chain and its size does; an append chains one
  event however many it carried, with counts, the sequence range and the per-type breakdown
  and never the events; an administrator is offered neither `session.read` nor `session.write`
  at somebody else's `principal` scope; and all three tables are tenant-bound with forced RLS,
  with no UPDATE or DELETE on the two append-only ones and no DELETE on `sessions`.

CPR-11  The session product experience (L)
  Filed 2026-08-24 by Prompt 11 of the CPR programme. CPR-10 made a run a governed record —
  three tables, seven routes, two Cedar actions, an audit chain — and a console page that
  listed the newest runs and expanded one in place. What it did not make is a record somebody
  with a question about a particular run can use, and the four gaps are holes rather than
  polish. **A run older than one answer was unreachable**: the listing set `truncated: true`
  when there was more and could not say where to continue, so a deployment whose agents open a
  few hundred runs a week had no API path to last Tuesday's run at all — and nothing narrowed
  by who ran a thing, which client, or when. **A timeline reported one clock**: `received_at`
  has been stored since migration 0044 and served nowhere, while the adapters this product
  ships spool to disk when the gateway is unreachable and flush later — so an hour of a
  transcript arrives at once, an hour late, and a reader sees a perfectly plausible transcript
  with no sign any of it was recovered. **There was no way to see what was actually said, and
  no way to stop people seeing it**: a payload was echoed to the client that wrote it and read
  by no route, and the moment one exists it is the largest disclosure on this plane. **And a
  run said how it stopped and never why.**
  What it adds: keyset pagination (`cursor` in, `next_cursor` out, `truncated` deleted rather
  than kept beside it) with four more filters — `client_name`, `principal_id` and a half-open
  `started_after`/`started_before` day range; `received_at` and a server-computed `delayed` on
  every timeline event entry; a new route `GET /v1/sessions/{id}/events/{event_id}` behind a
  **new Cedar action** `SessionDiagnostics`, strictly narrower than each pack's own
  `SessionRead` (packs @19 → @20); `sessions.end_reason` (migration 0045) set at close and
  carried into the chain; and the console's session product surface — a filter bar, Load more,
  a route per run at `/console/sessions/{id}`, an ordered timeline showing both clocks and
  marking what did not arrive live, a warning banner and per-entry warning marks, repository
  and branch, and a policy-authorised payload expansion offered from the caller's forecast at
  the run's own scope. Routing gains one level of `:param` pattern so a run has a real,
  linkable, refreshable URL.
  Five decisions in ADR-0077. **The cursor follows the last candidate a page considered, not
  the last row it served**: rows are decided one at a time after they are scanned (CPR-9), so a
  cursor on the last served row would end the listing whenever a whole page was denied while
  readable rows sat below it — which is why a page may be empty and still carry a cursor, and
  the schema says so. **A keyset, not an offset**: an offset skips and repeats whenever a run
  is opened between two requests, which on this table is every request. **Lateness is one flag
  and not three**: a spooled batch, a replay after a crash and a wrong clock produce the same
  two instants, so the server reports the gap and refuses to name a cause. **A payload is its
  own authority**, because a pack must be able to let a project's members follow what their
  agents did without handing every one of them everybody's prompts — and the chain records
  which event was expanded and never what was in it. **An end reason is not a task summary**:
  one is what the run was about, the other is what broke.
  AC: a listing pages through every run exactly once, newest first, and the walk terminates; a
  cursor the listing did not issue is a 400 rather than a silent restart; `truncated` is gone
  from the response rather than kept beside `next_cursor`; the four new filters narrow, the
  client filter is exact rather than a prefix, and an inverted date window is a 400; every
  timeline event entry carries both instants and a context run carries neither; an event
  delivered two hours after it happened is `delayed` and a live one is not; an `adapter.warning`
  is counted in `event_counts` and its own sentence reaches the entry; a caller granted `member`
  at a project reads that run's timeline and is refused `session.diagnostics` by name on the
  same run, while an administrator gets the bytes; a timeline carries no payload text at all;
  the audit chain records that an event was expanded and contains none of its content; an event
  id from another run — or an id nobody minted — is the same 404; a close records a reason, it
  survives a re-read, it reaches the `session.ended` payload, and one over its bound is refused
  rather than truncated; and the console renders an active run, a completed one, a failed one
  with its reason, a delayed entry with both clocks and the gap, a delivery warning in a banner
  and in place, a refusal with not one fact about the run in it, and another tenant's id exactly
  as it renders a fictional one.

CPR-12  Durable Claude session delivery (XL)
  Filed 2026-08-23 by Prompt 12 of the CPR programme. CPR-10 and CPR-11 built the record and
  made it usable; nothing wrote to it. The Claude Code adapter still posted to the global
  `POST /v1/observe` with a `session_id` string it invented, composed through `POST /v1/inject`,
  and buffered to a spool whose only durability was the process staying alive: a Stop hook wrote
  a file and fired a delivery, and a delivery that failed was **gone** — no attempt count, no
  acknowledgement state, nothing for a later hook to retry. The three global routes were
  therefore not merely superseded, they were the only writers of a plane the product had
  stopped describing, and the two planes could not both be true.
  What it adds: **a durable local spool** with a versioned format — spool version, client
  installation id, Synveda session id, client event id, sequence, event type, occurred time,
  payload, payload hash, delivery attempts, last attempt time and acknowledgement state —
  persisted atomically (write to a temp file, fsync, rename) and never read in its predecessor's
  format. Hooks own delivery: SessionStart opens or resumes a run and retries the backlog, Stop
  records fast and starts a delivery, SessionEnd flushes within a bounded budget, and the next
  SessionStart retries whatever is still unacknowledged. Three diagnostic commands —
  `synveda session flush`, `synveda session spool status` and
  `synveda session spool purge --acknowledged`. Context injection is a context run. And the
  cutover itself: `/v1/observe`, `/v1/inject` and `/v1/recall` deleted with their DTOs, their
  staging table, their queue and their quarantine table, extraction re-anchored on
  `session_events`, and every caller — gateway, ingest worker, CLI, MCP server, eval harness —
  moved or refused by name.
  Eight decisions in ADR-0078. **One write seam**, because two ways in is two authorisation
  paths and one of them is always the one nobody re-checked. **A session event is the
  extraction unit**, so the thing a policy decides about and the thing a memory is attributed
  to are the same row. **A memory lands at the scope the run was decided at**, not at the
  submitter's home — which is the difference between a workspace remembering what happened in
  it and every agent accumulating a private pile. **Quarantine is a withheld signal, never a
  mutated event**: the row a caller redelivers must hash to what it sent. **The spool hashes
  with SHA-256 rather than BLAKE3**, because the thing that has to verify it is Node and Node
  has no BLAKE3 — a chain hash and a spool checksum answer different questions. **Only
  acknowledged events may be automatically deleted**, so `purge` requires `--acknowledged` and
  offers no `--all`. **And the event-loss boundary is real, bounded and documented**: a host
  that dies before any lifecycle hook runs takes the un-flushed tail with it, and the honest
  number is "since the last Stop", not "nothing".
  AC: a spool survives a kill -9 mid-write and reads back as either the old state or the new
  one and never as a truncated file; a redelivered batch appends only what is new and answers
  `duplicate` for the rest, at their original sequence positions; an event whose payload hash
  does not match its payload is refused rather than stored; SessionEnd returns inside its
  budget with the spool flushed or the backlog intact; a SessionStart after a failed delivery
  retries it and acknowledges it; `spool purge --acknowledged` deletes only acknowledged
  entries and `spool purge` alone is refused; a memory extracted from a run lands at the run's
  scope and is readable by a workspace member who is not its author; `/v1/observe`, `/v1/inject`
  and `/v1/recall` are 404 by name; and the whole round trip — hook writes, gateway appends,
  worker extracts, context run composes — runs against a live stack.

CPR-13  The demo corpus re-point (L)
  Filed 2026-08-23 by Prompt 12, which went looking for the demos it had to re-point and found
  that **45 of the 67 shell scripts under `demos/` were dead**, and had been since
  CPR-7 (2026-08-20). That
  prompt deleted `synveda role bind`, `synveda hierarchy` and `/v1/hierarchy/*` whole — which
  was right, and is what ADR-0074 decided — and re-pointed the code, the tests and the docs.
  It did not re-point the demos, and nothing said so: CPR-7, CPR-8, CPR-9, CPR-10 and CPR-11
  all record clean runs, because no gate runs a demo. So the acceptance evidence for most of
  Phases 1–3 currently consists of scripts that exit non-zero on their fourth line.
  This is not CPR-12's to fix. Its 32 observe/inject/recall call sites live almost entirely
  inside those same 45 files, so re-pointing them onto the session plane would produce scripts
  that are still dead one command earlier — and re-pointing the placement half means rewriting
  each script's whole setup narrative on the governed scope model, which is a prompt, not a
  side effect. CPR-12 fixed the four that were live (`cpr-10-sessions`, `eval-2-extraction`,
  `eval-4-qa`, `ops-2-helm-install` with its client fixture) and deleted `ctx-5-recall`, whose
  subject no longer exists.
  What it adds: every remaining demo re-pointed onto workspaces, scopes and grants, and — the
  half that matters more — **a gate that fails when a demo names a command or route the product
  does not have**, so this cannot happen invisibly a fourth time. `make check-backlog`,
  `check-adr-status`, `check-api-types` and `check-benchmarks` are the precedent: each exists
  because a document drifted from the tree once.
  AC: no script under `demos/` names a CLI subcommand `synveda --help` does not list or a
  `/v1` path absent from `docs/api/openapi.json`; the gate catches a deliberately reintroduced
  dead command; and a representative demo from each of MEM, CTX, FLOW, AUTHZ and ADPT runs
  green against a live stack.
  Delivered 2026-08-24 after the core MVP surfaces existed: 49 affected scripts were rewritten
  as concise current-model narratives over isolated epoch-2 Postgres and focused acceptance
  seams: 18,528 affected-script lines became 504 plus a 52-line shared harness, a net reduction
  of 17,972. `make check-demos` recursively checks all 73 shell
  scripts against freshly built recursive Clap help and the generated OpenAPI paths; four
  parser/gate tests include deliberately dead command, path and binary-alias fixtures. The MEM,
  CTX, FLOW, AUTHZ and authentic-frame ADPT representatives all pass. No product contract,
  schema, PDP or audit vocabulary changes.

CPR-14  Live Claude Code session acceptance gate (L)
  Filed 2026-08-23 by the post-CPR-12 acceptance handover. CPR-12 made the session plane the
  only adapter path and made delivery durable, but its evidence stopped one layer short of the
  product claim: adapter functions, recorded hook payloads and gateway tests do not prove that
  a current installed Claude Code client discovers the marketplace, invokes those hooks, takes
  context through a session context run, appends its own activity and closes the same run. The
  old live evidence belongs to the deleted observe/inject plane and cannot prove the replacement.
  What it adds: a separately runnable **live-client gate** that packages and installs the plugin
  through `synveda plugin install`, asks Claude Code itself to report the plugin, four hooks and
  MCP server, and drives a deterministic headless `claude -p` session against a fresh epoch-2
  deployment; plus a **replay/live-gateway gate** for credential-free CI, using genuine,
  versioned Claude Code frames whose manifest binds every fixture to capture provenance,
  sanitisation and a SHA-256. Both tiers use the built hook child process and public session
  routes. The live tier alone may close the live criterion; without a current authenticated
  executable its runner exits 77 (`make` reports recipe `Error 77`) and the feature remains open
  rather than promoting replay into live evidence.
  The replay deliberately loses an acknowledgement: it takes the gateway away after a turn is
  durable, proves the unacknowledged spool survives, restores the gateway, commits one pending
  event without changing local acknowledgement state, then lets the next SessionStart redeliver
  an overlapping batch. The server must answer `duplicate` at the original sequence for the
  first and `appended` at the next sequence for the second, with one stored row per client event
  id. It also pins private spool/capture permissions, message-free timeline summaries, separately
  authorised diagnostics, content-free audit/log evidence and the boundary CPR-12 documented:
  a host killed before any hook writes may lose the in-flight tail.
  AC: the real authenticated executable, installed plugin and real hooks create or resume exactly
  one governed session; context arrives through `POST /v1/sessions/{id}/context-runs`; authentic
  user, assistant and tool activity reaches ordered `session_events`; normal completion flushes
  and ends the run; timeline and verifying audit chain show actions but no message content; spool
  ids, SHA-256s, server BLAKE3 hashes, attempts and acknowledgements agree; the outage/lost-ack
  replay stores every event exactly once and purge removes acknowledged entries while retaining
  pending ones; exact Claude Code, plugin, Synveda and OS versions and SessionStart, Stop,
  SessionEnd, append, context-run and recovery durations are recorded; and ordinary CI runs the
  schema-validated authentic-frame replay under its own name without claiming a live client ran.
  Delivered 2026-08-24 against installed authenticated Claude Code 2.1.241 and plugin 0.2.0:
  the real client reported four hooks plus one MCP server enabled, composed one context run,
  persisted four ordered user/tool/assistant events and ended the same governed session.

CPR-15  Versioned Knowledge aggregate and provenance (XL)
  Filed 2026-08-24 by the autonomous continuation of the context-platform programme. The
  session plane is now real and live-verified, but what extraction produces is still the old
  mutable `records` model: stable record identity, trigger-copied history, provenance in a JSON
  bag and an embedding tied to the mutable row. ADR-0068 locked a different model — candidates
  and published Knowledge are different aggregates, and Knowledge has stable item ids plus
  immutable revisions — so this feature creates that persistence boundary without bridging the
  two.
  What it adds: `KnowledgeItem`, the stable aggregate head carrying tenant, governed scope,
  optional project and owner, type, origin, lifecycle and current revision;
  `KnowledgeRevision`, immutable Markdown content with summary, canonical tags, sensitivity,
  integer confidence, valid time, database transaction time, stale-after, verification,
  extension metadata and canonical BLAKE3 hash; independently scoped `KnowledgeSource`
  provenance, linked many-to-many so merge can retain every source; and append-only
  `KnowledgeRelation` claims over the eight initial edge types. The aggregate head uses an
  ADR-0006 current/history pair so lifecycle, scope and current-pointer state remain bitemporal
  without mutating content revisions. A security-invoker current projection and stored lexical
  search document are the retrieval seam.
  This is persistence only: no HTTP, CLI or adapter mutation exists until the next package wraps
  it in the PDP, VedaFlow and audit chain. It reads and writes no old record table. ADR-0080
  carries the exact record/storage/API/browser deletion checklist the governed lifecycle and
  public Knowledge packages must complete; no dual write, fallback or translator may stand in.
  AC: stable item identity survives revision append; old revisions and head-state history remain
  immutable and queryable; current projection follows exactly the current revision; all seven
  source types and eight relations round-trip; every revision has at least one source; a
  separately scoped private source is omitted from a visible-source read; all new tenant tables
  are forced-RLS and cross-tenant ids are invisible; both views are security-invoker; source and
  relation constraints refuse cross-tenant or mismatched revision claims; a Knowledge write
  changes no old record table; and focused tests, `make ci` and `make db-test` pass.

CPR-16  Governed Knowledge mutation lifecycle (XL)
  Filed 2026-08-24 by the autonomous continuation. Extend the existing VedaFlow proposal
  and approval engine with a typed Knowledge `apply` effect: create, edit, verify,
  supersede, merge, archive, restore and forget all produce one content-addressed change,
  pass the PDP, resolve the effective approval matrix and chain audit evidence. A
  permissive policy may auto-apply the change; a strict one leaves the exact bytes pending
  review. No command writes a VedaFlow-independent Knowledge mutation.
  The immutable VedaFlow object binds a content-free manifest and canonical payload hash;
  the erasable effect projection retains the pending payload so authorised forget can
  remove plaintext without destroying governance history. Add one reusable durable
  operation ledger, with erasure as its first consumer. Forget evaluates the retention/
  legal-hold seam, removes authorised Knowledge, command payload and index plaintext, and
  leaves only ids, timestamps and hashes in an append-only tombstone.
  AC: every command returns its change id and applied/pending-review/rejected outcome;
  auto-apply, review, rejection and stale preconditions are exact; edits and verification
  append immutable revisions; supersession writes an explicit relation; merge retains all
  source provenance; archive/restore preserve history; erasure is durable, held or
  content-free and retry-safe; all new tenant tables are forced-RLS; every transition is
  content-free audited; the gateway starts no old extraction, promotion or retention
  writer and no Knowledge command touches records; the two controlled old-plane seams are
  named for the read/context cutover rather than hidden; focused tests, a runnable demo,
  `make ci` and `make db-test` pass. ADR-0081.

CPR-17  Public Knowledge API, search and browser (XL)
  Filed 2026-08-24 by the autonomous continuation. Put the CPR-15/16 aggregate and
  command seam behind one generated public application contract: current/detail/history,
  independently governed provenance and relations, usage, create/edit/verify/supersede/
  merge/archive/restore/forget, cursor pagination and the complete Knowledge Browser.
  Search uses the immutable current revision's stored lexical document and a forced-RLS
  revision embedding sidecar for a real configured semantic model. The deterministic
  zero-config hash remains an honest lexical-only degradation, never a semantic claim.
  Every candidate, source scope and relation endpoint is decided before disclosure; a
  denied row advances the cursor but leaks no object, edge or count.
  This is the public noun cutover: delete the record classification route/CLI/eval call,
  generic proposal inputs naming `record_ids`, raw-record console DTOs and fixtures. The
  record-backed context composer remains internal only until the explainable context
  planner; there is no bridge, fallback, alias or dual read. AC: all thirteen route
  groups are mounted and generated;
  creation is idempotent and mutations require revision preconditions through VedaFlow;
  every requested filter and current-active default is exact; lexical and semantic modes
  are honest; history/source/relationship isolation holds; the browser uses only generated
  operations; embedding RLS/erasure and content-free read audit hold; focused tests, demo,
  `make ci` and `make db-test` pass. ADR-0082.

CPR-18  Session-based capture batches and reviewable candidates (XL)
  Filed 2026-08-24 by the autonomous continuation. Replace the final internal
  session-event-to-record extraction writer with `CaptureBatch` and
  `CaptureCandidate` aggregates. Explicit requests and session end select
  potentially durable information, classify it, retain exact source event ids,
  compare it with policy-visible current Knowledge and record duplicate,
  conflict and possible-supersession matches. Model output is validated and
  becomes candidates only: an unreviewed candidate is never active Knowledge.
  Candidate and batch acceptance, edit-and-accept, merge, replace and dismiss
  are ordinary public application commands. Every publishing action calls
  CPR-16's Knowledge command layer and therefore creates a VedaFlow change;
  replace is governed supersession rather than deletion. Repeated extraction
  of the same session and request is idempotent, and a retryable failed batch
  has one durable address rather than duplicate candidate rows.
  AC: the two aggregates and all seven candidate states have tenant-bound,
  forced-RLS persistence; source event links prove same-session/same-tenant
  provenance and cannot be forged; extraction creates candidates only and
  writes zero Knowledge or record rows; duplicate/conflict/supersession matches
  include only independently PDP-visible Knowledge; the nine capture route
  groups are generated, cursor-paginated where applicable and use the common
  error envelope; accept, edit, merge, replace, dismiss and whole-batch accept
  are idempotent and record decision metadata; accepted actions return their
  VedaFlow change and Knowledge ids, including pending-review outcomes; repeated
  extraction is idempotent; the old record extraction writer and its runtime
  tables/queue assumptions are deleted; audit evidence is content-free; and
  focused tests, a runnable demo, `make ci` and `make db-test` pass. ADR-0083.

CPR-19  New Learnings lightweight review workflow (L)
  Filed 2026-08-24 by the autonomous continuation. Replace the New Learnings
  placeholder with the ordinary personal/team review surface over CPR-18's
  generated capture contract. Group candidates by their durable batch, filter
  by project, session and decision state, show batch progress, exact source
  conversation evidence, proposed type and placement, policy-visible duplicate,
  conflict and supersession hints, fresh existing-Knowledge comparisons and the
  resulting Knowledge address. Offer accept, edit-and-accept, merge, replace,
  scope change and dismiss; replace is governed supersession and no candidate
  action publishes outside CPR-16's VedaFlow command seam. Private principal,
  project and workspace placement are named distinctly, and the scope picker
  contains only anchors whose `/v1/me` forecast offers `knowledge.write`; every
  request still meets the gateway's exact PDP decision. A pending outcome points
  to Advanced Reviews and remains explicitly outside active Knowledge. Keep that
  comprehensive review engine and delete the planned/duplicate candidate product
  surface rather than creating a second proposal inbox.
  AC: the page uses only generated capture, session and Knowledge operations;
  project/session/state filters and both collection cursors are handled; batches
  show honest loaded/review progress; exact source events resolve to timeline
  previews and raw payload is offered only under `session.diagnostics`; every
  match is re-read through the public Knowledge API before comparison; all six
  actions are present and their generated idempotent wire bodies carry exact
  revision preconditions where required; unavailable publication scopes cannot
  be selected; personal/project/workspace wording is explicit; applied,
  pending-review, rejected and dismissed outcomes cannot be confused; Advanced
  Reviews remains the sole comprehensive proposal surface; the placeholder is
  deleted; and focused helper/rendering tests plus the production build and
  `make ci` pass. No ADR: ADR-0075, ADR-0081, ADR-0082 and ADR-0083 already lock
  the shell, command, read and candidate boundaries this package consumes.

CPR-20  Explainable Knowledge context planning and scoped query (XL)
  Filed 2026-08-24 by the autonomous continuation. Replace the final
  record-backed runtime reader with a durable, explainable planner over current
  immutable Knowledge revisions. Persist the visible bounded candidates,
  selections, exact revision addresses, reason and integer score components,
  requested/actual budget, retrieval/index versions, degradation, rendering
  hash, completion and governed trace-retention mode. Denied Knowledge leaves
  no object-shaped trace and every retained reference is re-authorised on read.
  Keep separately governed context packs and skill advertisements as authored
  inputs; never translate Knowledge through records or admit an unreviewed
  capture candidate without an explicit future governed channel rule.
  Add generated run list/detail and exact-revision feedback operations, a
  session-scoped ordinary Knowledge query, and a separately authorised
  SessionDiagnostics evaluation lens for query, enumeration and exact-id
  benchmarks. Neither surface restores tenant-global `/v1/recall` or direct
  store access. AC: only current active Knowledge selects; stale, superseded,
  duplicate and token-budget exclusions are explicit when visible; full,
  redacted, hashes-only and disabled traces disclose exactly their configured
  shape; policy-denied candidates leak no id/title/edge/reason/count; feedback
  binds one run, selection and immutable revision; Knowledge usage derives
  from re-authorised selections; retrieval, selection, delivery and feedback
  are distinct traced/metered/content-free audited events; context packs and
  skills survive the cutover; old runtime record composition and recall
  tombstones are deleted; focused tests, demo, `make ci` and `make db-test`
  pass. ADR-0084.

CPR-21  Context Inspector and outcome feedback (L)
  Filed 2026-08-24 by the autonomous continuation. Build the linkable Context
  Inspector over CPR-20's generated, freshly re-authorised run detail. Show the
  retained task, exact immutable selected revisions and provenance, planning
  lifecycle state, reasons, integer score components, rank, token cost and total
  budget, visible exclusions, implementation/index versions, degradation and
  rendered hash without widening any trace-retention mode. Offer the five
  explicit feedback types against one exact selection and revision through the
  generated idempotent command; retrieval alone is never helpfulness. Link each
  session timeline context entry to the inspector and summarise it as “Synveda
  supplied N knowledge items” without repeating the task on the broader session
  surface.
  AC: `/console/context-runs/{id}` is refreshable and uses only generated
  operations; selected current Knowledge, visible source evidence, reasons,
  scores, rank and tokens are legible; stale/superseded and token-budget
  exclusions are not confused with current selection or policy denial;
  full/redacted/hashes-only/disabled modes state what was retained and never
  invent withheld ids/content; policy denial exposes only the aggregate server
  message; feedback carries exact run/selection/revision identity and one
  idempotency key, then refreshes the detail; the session timeline links the
  exact run with the required summary; existing project-isolation and denied-
  Knowledge gateway cases remain green; no schema, policy, audit or OpenAPI
  operation is added; and helper/rendering, focused gateway, build, `make ci`
  and `make db-test` gates pass. No ADR: ADR-0075, ADR-0077 and ADR-0084 already
  lock every consumed boundary.

CPR-22  Core individual and small-team MVP acceptance (L)
  Filed 2026-08-24 by the autonomous continuation. Prove the complete
  PulseBoard loop over one public application runtime: Alice's real session
  events become reviewable candidates; two project items and one principal-
  private preference publish only through Knowledge VedaFlow changes; Bob
  reuses the project revisions from a clean session without seeing the private
  one; Bob's captured correction explicitly supersedes the old convention; and
  a third clean run plus the Context Inspector show the replacement and why it
  was selected while retaining the obsolete item only as history.
  AC: one database-backed public-API scenario asserts the workspace/project/
  grant, three session records, exact event evidence, candidate decisions,
  VedaFlow changes, immutable Knowledge revisions and sources, private PDP
  isolation, explicit supersession relation/current projection, context
  selections and visible inspector/timeline contract; the audit chain verifies
  with allowed decisions and all semantic transitions but no content; no old
  record is written and deleted global runtime paths remain 404; no schema,
  route, DTO, Cedar/audit vocabulary or second implementation is added; an
  isolated runnable demo, focused suites, `make ci` and `make db-test` pass.
  No ADR: this is the acceptance composition of ADR-0070 through ADR-0084.

CPR-23  Immutable skill versions, bindings and usage (XL)
  Filed 2026-08-24 by the autonomous continuation. Replace the mutable
  draft-plus-`skill/published` registry with stable Skill aggregates, immutable
  content-addressed SkillVersion rows, project/principal SkillBindings,
  evidence-labelled SkillUsageEvents and controlled SkillTestRuns. Extend the
  existing Agent Skills parser, scanners, quality rubric, VedaFlow object store
  and approval engine; do not introduce a second registry or rewrite bundle
  bytes. Pin the official unversioned Agent Skills specification to its tested
  upstream commit, add its `compatibility` field and exact name grammar,
  preserve extension metadata, and keep declared tools metadata-only.
  Install, update, bind, disable, enable, pin, unpin and rollback become typed
  VedaFlow `apply` changes with live PDP/precondition checks at execution.
  Distribution and ContextRun advertisement resolve the same enabled bindings;
  rollback changes a binding, never history. Eight usage stages distinguish
  host observation from model report. The gateway's validation sandbox parses
  and scans but never executes a bundled script; controlled-client evidence is
  labelled separately. AC: immutable/version digest and old-row mutation
  refusals; pending/applied/rejected changes; stale version/binding conflicts;
  project/principal pin, disable and rollback; PDP/RLS tenant and private-scope
  isolation; exact version/file/provenance/scan APIs; usage idempotency and
  evidence separation; safe test runs; generated contract and CLI cutover; no
  skill channel/draft residue; focused tests, demo, `make ci` and `make db-test`
  pass. ADR-0085.

CPR-24  Skills Library product experience (L)
  Filed 2026-08-24 by the autonomous continuation. Replace the obsolete
  mutable-Skill listing and Skill-only proposal renderer with one generated-
  contract Skills Library over CPR-23's stable aggregates, immutable versions,
  revisioned personal/project bindings, controlled tests and evidence-labelled
  usage. This is a product surface, not another registry: it adds no schema,
  API, policy or audit vocabulary and makes no decision the gateway owns.
  AC: the catalogue and linkable detail show installed Skills, current and
  available exact versions, digest, source/provenance, manifest extensions,
  compatibility, declared tools, quality and scan evidence; the file browser
  reads exact immutable bytes; personal and selected-project binding cards
  show enabled/following/pinned state and offer create, enable/disable, pin,
  unpin and rollback only when the real anchor forecasts `skill.write`; session
  availability is the generated resolver's answer; installation and complete-
  bundle updates carry idempotency and stale-head preconditions and every
  mutation reports its VedaFlow change/outcome; fixture testing names its
  validation sandbox, states that it executes no scripts and distinguishes
  controlled-client history; recent usage keeps host-observed and model-
  reported evidence separate; declared tools are explicitly non-authoritative;
  the console makes only generated Skill calls; old hand-written `skillsAt`,
  Skill checklist/quality review UI, CLI renderer and fixture corpus are
  deleted while artifact-neutral Advanced Reviews remains; focused pure and
  real-component acceptance, CLI regression, production build and `make ci`
  pass. No ADR: ADR-0075 and ADR-0085 already lock the consumed boundaries.

CPR-25  Trusted MCP server catalogue and project bindings (XL)
  Filed 2026-08-24 by the autonomous continuation. Add stable ToolServer
  aggregates, immutable ToolServerVersions, raw and normalised
  CapabilitySnapshots, revisioned exact-version project ToolBindings and
  immutable read-only ToolTestRuns. Treat MCP as the external discovery/config
  format around this governed product model, not as execution authority and
  not as a second generic MCP adapter. Pin the stable official MCP 2026-07-28
  specification and its stateless `server/discover`, per-request metadata,
  stdio and Streamable HTTP model; never add HTTP+SSE or session-state
  semantics. Import explicit manifests and one entry from supported client
  configuration, register Streamable HTTP metadata, and admit stdio discovery
  only as evidence reported by an authorised trusted local adapter. Preserve
  raw metadata and deterministic normalisation; source, digest, transport,
  authentication, requested-permission, tool, resource or prompt changes mint
  a quarantined immutable version. Approval and every binding transition are
  typed VedaFlow apply changes; project bindings always pin an exact approved
  version, so a changed server never becomes active silently. Credentials are
  secret references only. Generate client configuration without a secret value
  and record only read-only discovery/list connectivity tests; universal tool
  execution is outside this feature.
  AC: immutable version/digest and diff evidence; unchanged discovery is
  idempotent; quarantine then applied/pending/rejected approval; a new approved
  version leaves an existing project binding unchanged; revisioned bind,
  disable, repin and remove; exact tools/resources/prompts and schemas; current
  2026-07-28 pin with no HTTP+SSE; generated secret-free config; read-only tests
  reject any execution method; PDP/RLS tenant/project isolation; content-free
  audit; generated OpenAPI; authentic stateless discovery fixtures; focused
  tests, demo, `make ci` and `make db-test` pass. ADR-0086.

CPR-26  MCP Tools catalogue product experience (L)
  Filed 2026-08-25 by the autonomous continuation. Replace the Tools
  placeholder with one generated-contract product surface over CPR-25's
  stable catalogue, immutable versions, quarantined discovery evidence,
  exact project bindings and discovery-only tests. This feature adds no
  execution authority, secret resolver, schema, API, policy or audit
  vocabulary: ADR-0075 and ADR-0086 already lock those boundaries.
  AC: a policy-aware catalogue and stable detail route show source, immutable
  version/digest, transport, protocol, authentication kind, secret-reference
  presence without its value, trust/validation state, discovered tools,
  resources and prompts with schemas, approved-version differences, exact
  project bindings, last discovery, latest read-only health evidence and
  generated client configuration; import and discovery retain bounded raw
  metadata while approval links to the common VedaFlow review plane; binding
  create, enable, disable, exact repin and removal call only generated public
  operations and retain their revision preconditions; changed quarantined
  versions are unmistakable and cannot be bound before approval; the test
  reporter names its trusted adapter harness and offers discovery/list methods
  only; declared capabilities visibly grant no authority; no secret-reference
  value appears in rendered output or frontend snapshots; the obsolete Tools
  placeholder is deleted; focused pure and real-component acceptance,
  production build and `make ci` pass. No ADR.

CPR-27  OKF v0.2 knowledge exchange adapter (XL)
  Filed 2026-08-25 by the autonomous continuation. Implement one versioned
  external-format boundary for the official Open Knowledge Format v0.2 over
  the existing Knowledge and capture domains. Directory, zip, tar and
  explicitly identified checked-out Git sources become immutable import
  artifacts, a deterministic dry-run plan and reviewable capture candidates;
  an import never publishes Knowledge. Export selects current visible
  Knowledge and emits one deterministic v0.2 bundle without creating a second
  Synveda knowledge format.
  AC: pin the exact canonical OKF v0.2 specification revision and refuse v0.1;
  validate bounded UTF-8 Markdown/YAML with a required non-empty `type`, while
  preserving unknown types and frontmatter extension metadata; retain source
  revision, logical path, content hash, provenance, generation, verification,
  lifecycle and staleness; map internal links to proposed relations and report
  additions, updates, duplicates and conflicts before materialising candidates;
  unchanged reimport is idempotent; no import path calls the Knowledge mutation
  layer; deterministic export has stable paths, ordering, links and round-trip
  metadata; path traversal, symlink escape, archive expansion abuse, binary or
  executable content, SSRF and private-address redirects are refused; import
  jobs, artifacts and mappings are tenant-bound, forced-RLS, PDP-gated and
  audited without content; generated public API, focused fixtures, demo,
  `make ci` and `make db-test` pass. ADR-0087.

CPR-28  OKF import and export product workflows (L)
  Filed 2026-08-25 by the autonomous continuation. Add the public-API CLI and
  generated-contract project console over CPR-27's immutable v0.2 plans,
  candidate-only materialisation and deterministic export. The local CLI owns
  filesystem traversal and output writes; the gateway continues to receive
  inert bytes and grants no server-path, Git-process or execution authority.
  AC: `synveda okf validate <path>` and `inspect <path>` locally apply the
  exact pinned v0.2 adapter and expose validation, source shape, artifact
  metadata and unknown fields; `okf import <path> --project <id> --dry-run`
  creates and renders the immutable public dry-run only, while the non-dry-run
  form idempotently materialises its reviewable candidates; `okf export
  --project <id> --output <path>` calls the public export operation and writes
  its exact stable bundle atomically without path escape or silent overwrite;
  checked-out Git input retains an explicit source revision and no command is
  run; a project console surface shows source/validation/revision, additions,
  updates, duplicates, conflicts, progress/history, resulting candidates and
  deterministic export selection/status/summary using generated operations and
  types only; unknown OKF types and extension metadata remain visible and pass
  an import/export round trip; no scheduled Git sync or competing bundle format
  is added; focused CLI/component acceptance, production build, demo and
  `make ci` pass. No ADR; ADR-0087 fixes the boundary.

CPR-29  Public contract and client convergence (XL)
  Filed 2026-08-25 by the autonomous continuation. Complete the generated
  OpenAPI 3.1 application contract over every authenticated production `/v1`
  operation, then remove the handwritten console calls and storage-coupled
  ordinary CLI paths that the contract exposes. Re-cut the generic MCP and
  Claude adapters against the same public session, Knowledge, context, Skill
  and Tool-binding vocabulary without adding adapter-private authority.
  AC: one route declaration is the executable router inventory and the
  contract test proves exact method/path equality in both directions; every
  operation has a unique generated identifier, the common error envelope,
  authentication, idempotency and revision-precondition metadata where the
  server requires them, and consistent bounded pagination where the existing
  collection supports it; generated console operations/types cover proposals,
  capabilities, policy assignment, lapses, audit, channels, prompts, packs,
  quarantine, service identities, directory administration, SCIM credentials,
  Knowledge, capture, context, Skills, Tools and OKF; no governed console call
  remains hand-written; ordinary service-identity and audit CLI commands call
  the public API while only documented database/bootstrap/key/secret operator
  actions retain store access; generic MCP uses only current public session,
  Knowledge/context, available-Skill and project Tool-binding reads, Claude
  remains public-API-only, and neither adapter imports a core service layer;
  duplicate client DTOs and obsolete route-private business rules are deleted;
  focused gateway/contract/console/CLI/adapter acceptance, `make ci` and the
  relevant database suite pass. ADR-0088.

CPR-30  Governed runtime configuration artifacts (XL)
  Filed 2026-08-25 by the autonomous continuation. Replace mutable policy-pack
  assignment and ad-hoc runtime knobs with stable configuration aggregates,
  immutable content-hashed versions and revisioned nearest-scope bindings.
  Personal, team and enterprise are canonical documents over one runtime, not
  editions or branches. AC: complete validated documents cover the policy-pack
  selector, capture/extraction, context budget/channels/trace retention,
  type-aware freshness, Skill/Tool advertisement and allowed provider
  families; create, publish, bind, pin/unpin, enable/disable and rollback are
  typed VedaFlow Configuration/apply changes with live PDP, payload,
  version/revision and audit checks; capture and context cite the exact
  effective version/digest and all runtime consumers enforce it; old default
  and scope-assignment tables and direct mutation routes are deleted without
  translation or dual write; generated public API, console and HTTP CLI cover
  templates, list/show/history/compare/effective/create/publish/bind/rollback;
  all new tenant tables are forced-RLS; focused tests, demo, `make ci` and
  `make db-test` pass. ADR-0089.

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
Phase 3 enterprise (wk 11–16): SKIL-1..4 · OPS-1 · CNSL-1 · ADPT-2 · CNSL-2 ·
                         AUTH-4,5 · EVAL-3 · OPS-2 · TEN-3,4 · OPS-8 · OPS-9 · OPS-10 ·
                         TEN-5,6 ·
                         AUD-3,4 · GRPH-3 · EVAL-6 · CTX-7 · OPS-3,4 · ADPT-3 ·
                         CTX-6 · FLOW-8
   (Reordered 2026-08-04. Order within the phase is by demo-readiness, not epic-grouped
   and — unlike Phase 1's — not topological, because nothing here blocks anything else:
   every dependency Phase 3 has was met by Phase 2. The original order scattered this
   phase's own demo goal across slots 1, 2, 10, 13 and 18, so the phase could not run
   its demo until the phase was nearly over. The five features that goal names —
   AUTH-4,5 (Entra/Okta), ADPT-2 (Cursor), EVAL-3 (published scores), OPS-2 (Helm) —
   now sit in the front block with the two that make any of it showable at all: OPS-1,
   because until `synveda init` exists there is no instance that survives a restart and
   every demo is a script that seeds its own scratch state and tears it down; and
   CNSL-1, the hero screen (tech plan §5), because FLOW-6 gave the review flow full CLI
   parity and a terminal undersells a governance product to the people who buy it.
   CNSL-2 rides directly behind CNSL-1 while the frontend toolchain CNSL-1 must choose
   is warm. What moved back is the block a customer asks for at procurement rather than
   at a demo — TEN-3..6, AUD-3,4 — plus GRPH-3, EVAL-6, OPS-3,4, ADPT-3, CTX-6 and
   FLOW-8. Nothing is cut and the phase's contents are unchanged.)
   (CTX-7 added 2026-08-10 by TEN-3/ADR-0063, whose benchmark found it by disagreeing with
   itself: the same arm measured 0.341 recall at 5.9ms and 0.878 at 50.9ms on two runs, and
   the variable was not the arm. Placed in Phase 3, beside EVAL-6, because it is a
   performance claim this product has already published rather than one it has yet to make —
   CTX-1's AC is "p99 <80ms at 1M records/tenant" and nothing has established which plan
   that number was measured against. It is not a governance hole: the generic plan is exact,
   so it returns *better* answers, more slowly and with worse scaling.)
   (OPS-8 added 2026-08-11, and placed in this phase's demo block rather than in Phase 4
   with the rest of OPS, because it is the only feature here whose absence stops the phase
   demo from happening on anybody else's machine. Every goal this phase names is something
   somebody is meant to *watch* — Entra and Okta live, skills into Claude Code and Cursor,
   published scores, a Helm install — and until there is a release to download, watching it
   costs a Rust toolchain and a cold release build of Cedar, Tantivy and sqlx. It sits
   behind OPS-2 because the chart's images are two of the things a release publishes.)
   (OPS-9 added 2026-08-13, directly behind OPS-8 and for the same reason one step further
   on. OPS-8 asked "can somebody else install this?" and answered it; the question nobody
   had asked is what that somebody *sees* once they have. They see an empty product, because
   ADR-0055 decisions 1 and 2 correctly refuse to seed governed objects from an installer,
   and the seeding was never moved anywhere else — it was left as printed instructions,
   which is a design that assumes the reader is us. Every goal this phase names is something
   somebody is meant to watch, and OPS-8 established that watching it no longer costs a Rust
   toolchain; this establishes that it no longer costs a tour guide. It is also the feature
   that turns the phase demo into a **beta**, which is a different artefact: a demo is driven
   by whoever built it and a beta is driven by somebody who did not, so the limits stop being
   things we remember to mention and become a file — which is why `docs/BETA.md` carries the
   tour and the standing gaps together rather than in two documents that would drift.)
   → Demo: Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor,
     LongMemEval scores published, Helm install.
     (Read "LoCoMo/LongMemEval" until 2026-08-07. EVAL-3/ADR-0061 decision 1 found LoCoMo's
     corpus is CC BY-NC 4.0 and cannot back a published commercial claim; the second
     benchmark is EVAL-7. A goal naming a score we may not quote is a goal that cannot be
     met, so the goal moved rather than the honesty.)
Phase 4 ecosystem: ADPT-4,5,6,7,8 · PRMT-3 · SKIL-5 · MEM-7 · OPS-5,6,7 · CNSL-3,4 · AUD-5 · AUTHZ-6,7 · EVAL-7 · TEN-7
   (AUTHZ-7 added 2026-08-05 by CNSL-2/ADR-0058 decision 9, which found the asymmetry it
   names while building the explorer. Placed here rather than in Phase 3 because its two
   bounds hold — a pack flip widens no candidate universe and cannot reach below the
   invariant floor — so it is a governance question rather than a hole; if that reading is
   ever wrong, it belongs in front of the Phase 3 procurement block, not behind it.
   EVAL-7 added 2026-08-07 by EVAL-3/ADR-0061 decision 1. Here rather than Phase 3 because
   neither of its two paths is work we control: one waits on a grant from a third party, the
   other on a corpus that may not exist yet. EVAL-3 publishes a benchmark score without it,
   so the phase's demo goal is met — this is the second data point, not the first.
   OPS-7 added 2026-08-10 by OPS-2/ADR-0062 decision 5. Here rather than in Phase 3 because
   the phase's demo goal asks for a Helm install and OPS-2 is one: the enterprise profile's
   HA is the data plane's, which is what the feature text says, and the chart refuses the
   configuration it cannot honour rather than offering it with a warning. It moves forward
   the moment a deployment cannot serve its request rate from one gateway, or cannot accept
   a restart-shaped upgrade — and if it moves, it belongs beside OPS-6, since both are
   about an upgrade nobody notices.
   ADPT-8 added 2026-08-13 by running ADPT-1's plugin in a real Claude Code session on
   v0.1.3. Here rather than in Phase 3 because the interactive path — the one the phase's
   demo goal names, and the one a person uses — observes correctly, measured the same day:
   `observe.done events=5 accepted=5`. It moves forward the moment anybody drives Claude
   Code non-interactively and expects the chain to show it, which is CI, a scripted agent,
   or an evaluation harness — and that is a *when*, not an *if*, since ADPT-1's own demo
   is a script. What it must not become is a warning in a README: the gap is silent,
   returns exit 0, and reads exactly like a session that was observed.)
Phase 5 context platform (redesign): CPR-1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30
   (Added 2026-08-17. Its own phase rather than a slot in Phase 4, because it is not the
   next feature — it is the programme that re-cuts the model every feature above was built
   on, for an audience none of them was: one person, or four sharing agent context, who
   today must declare themselves an `org` containing a `team` before this product will hold
   a single record. Thirty-three ordered prompts, of which this files the first; the rest
   are filed by the prompts that run them, because a backlog that forecasts thirty-two
   features stops being a record of what was found. The decisions are locked in ADR-0068
   and the running record — inventory, deletion map, prompt order, and the MVP checkpoint
   after Prompt 20 — is docs/implementation/synveda-context-platform.md.)
   → Demo: a fresh deployment from nothing to a session, observed events, candidates, a
     published knowledge version, a context assembly that cites it, a recall that serves it,
     and a verifying audit chain — with one person's deployment and a team's differing only
     in the policy profile assigned to their scopes.
──────────────────────────────────────────────

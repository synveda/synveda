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
  LoCoMo + LongMemEval run end-to-end through Synveda (observe→inject/recall→judge).
  AC: reproducible scores published in repo; tracked per release. (Marketing artefact too —
  every credible 2026 memory system publishes these.)
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
Phase 3 enterprise (wk 11–16): SKIL-1..4 · OPS-1 · CNSL-1 · ADPT-2 · CNSL-2 ·
                         AUTH-4,5 · EVAL-3 · OPS-2 · TEN-3,4,5,6 · AUD-3,4 · GRPH-3 ·
                         EVAL-6 · OPS-3,4 · ADPT-3 · CTX-6 · FLOW-8
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
   → Demo: Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor,
     LoCoMo/LongMemEval scores published, Helm install.
Phase 4 ecosystem: ADPT-4,5 · PRMT-3 · SKIL-5 · MEM-7 · OPS-5,6 · CNSL-3,4 · AUD-5 · AUTHZ-6
──────────────────────────────────────────────

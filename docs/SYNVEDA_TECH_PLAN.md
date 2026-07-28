# SYNVEDA — Technical Plan v1

Companion to SYNVEDA_SEED.md. This document fixes the technology stack (open source,
Postgres-first, Rust-native where sensible), and specifies **VedaFlow** — the git-style
propose/review/approve workflow for all knowledge assets.

---

## 1. Stack decisions

Guiding rules: (a) Postgres is the system of record for everything until proven otherwise,
(b) prefer Rust-native components, (c) permissive licences (MIT/Apache-2.0) in the core path;
anything AGPL/BSL is opt-in and isolated behind a trait.

### 1.1 Core data platform — PostgreSQL 17

One database engine for records, hierarchy, audit, versioning, queues, and (initially) vectors
and graph. This is a feature: one backup story, one HA story, one thing to explain to a bank's
infrastructure review board.

| Concern | Choice | Licence | Rationale / scale-out path |
|---|---|---|---|
| System of record | **PostgreSQL 17** | PostgreSQL | Boring, auditable, runs anywhere incl. air-gapped |
| Vector search | **pgvector** (HNSW) | PostgreSQL | Fine to ~10–50M vectors per tenant shard. Scale-out: **Qdrant** (Rust, Apache-2.0) behind the same `VectorIndex` trait. Note: VectorChord/pgvecto.rs is Rust and faster but AGPL — optional adapter only |
| Sparse / lexical | Postgres FTS + **Tantivy** (Rust, MIT) sidecar via `synveda-retrieval` | MIT | BM25 quality without ParadeDB's AGPL. Hybrid fusion (RRF) done in Rust |
| Graph | **Indexed adjacency in plain Postgres** (bitemporal edge pair; named graphs as a mandatory discriminator) | — | Amended 2026-07-27 by GRPH-1/ADR-0043, was **Apache AGE**: the GRPH-4 spike measured adjacency 3–8× faster at 2.5× less storage, and AGE's `cypher()` takes a name constant its statements cannot be sqlx-checked inside. Still transactional with records, still one engine. Ladder: materialised k-hop closure table (the HIER-1 pattern), then a dedicated engine with its own ADR and a licence exception (candidates: **IndraDB**, Rust, MPL; avoid SurrealDB/Memgraph — BSL) |
| Hierarchy | Plain Postgres (closure table + materialised path) | — | No graph DB needed for tenancy |
| Queue (simple) | **PGMQ** (Postgres extension) | PostgreSQL | For observe-event ingestion buffer — no extra infra for SMB deployments |
| Workflow (complex) | **Temporal** | MIT | Extraction pipelines, directory sync, retention jobs, approval timers. Go-based but the best-in-class; Rust SDK (community) or activities via gRPC workers |
| Bitemporal versioning | Native tables (`tx_from/tx_to`, `valid_from/valid_to`) + triggers | — | No extension dependency; queryable "as-of" both dimensions |

### 1.2 Identity & policy — Rust-first

| Concern | Choice | Licence | Rationale |
|---|---|---|---|
| Authorisation (PDP) | **Cedar** (embedded) | Apache-2.0 | Amazon's policy language, **pure Rust, in-process** — no network hop on the hot read path; formally verified evaluator; policies-as-data suits VedaFlow versioning |
| Relationship checks | Cedar entity hierarchy (first choice); **OpenFGA** (Apache-2.0) adapter if ReBAC outgrows Cedar | Apache-2.0 | Start with one engine. The `authorize()` facade hides the choice |
| Why not OPA | Rego is powerful but adds a Go sidecar + network hop on every inject; Cedar embeds in the gateway binary | | OPA remains a supported adapter for shops that mandate it |
| OIDC provider (bundled dev/SMB) | **Rauthy** (Rust, Apache-2.0) | Apache-2.0 | Single-binary Rust OIDC server for SMB "batteries included" mode |
| Enterprise IdP | Bring-your-own: Entra ID, Okta, Keycloak, Zitadel — standard OIDC + SCIM 2.0 | — | Synveda is an OIDC *client*, never the source of truth for identity |
| Secrets/PII detection | Rust regex+ML pipeline; **gitleaks** ruleset port for secrets | MIT | Runs in `synveda-ingest` before persistence |

### 1.3 Services & runtime — all Rust

| Component | Tech |
|---|---|
| Gateway/API | **axum** + tonic (gRPC), tower middleware for authN/PDP/rate-limit/audit |
| ORM/queries | **sqlx** (compile-time checked SQL — auditability again) |
| Embeddings serving | **text-embeddings-inference** (Hugging Face, Apache-2.0, Rust) serving **BGE-M3** (dense+sparse) or **Qwen3-Embedding**; per-tenant model pinning, re-embed workflow on model change |
| Summarisation/extraction LLM | Pluggable: Claude API, or self-hosted via vLLM for air-gapped; behind `Extractor` trait |
| Observability | OpenTelemetry (traces on every inject/recall with record-ID watermarks), Prometheus, Grafana |
| Packaging | Single static gateway binary + Postgres = SMB mode. Helm chart with regional data planes = enterprise mode |

### 1.4 Explicit non-choices

- **No Elasticsearch/OpenSearch** (JVM estate, Tantivy covers it), **no Redis** initially
  (Postgres + moka in-process cache), **no Kafka** (PGMQ then Temporal), **no Neo4j** (licence),
  **no SurrealDB/Memgraph** (BSL). Every one of these is a door left open behind a trait, not a
  dependency taken today.

---

## 2. VedaFlow — git-style governance for knowledge assets

The insight: **treat organisational knowledge exactly like code**. Memories, context packs,
prompts, skills, and *policies themselves* flow through propose → review → approve → publish,
with approval authority derived from the hierarchy. Nothing reaches an agent that wasn't either
(a) auto-derived under policy, or (b) explicitly reviewed at the right level.

### 2.1 Model — git semantics in Postgres, not git repos

Git-*like*, implemented natively in Postgres rather than on bare git repos, because approvals,
policy checks, audit chaining, and row-level tenant isolation must be transactional with the
content. (A `git bridge` using **gitoxide** (Rust, MIT/Apache-2.0) mirrors published branches to
real repos for teams who want GitHub/GitLab visibility — export first, import later.)

Core tables (all content-addressed, BLAKE3):

```
objects   (hash, tenant, kind, content, size)          -- immutable blobs
trees     (hash, entries[])                            -- directory-like grouping per scope
commits   (hash, tree, parents[], author_identity,
           message, signature, policy_snapshot_hash)   -- every commit records WHICH policy
refs      (tenant, scope, name, commit_hash)           --   pack was in force when created
proposals (id, scope, source_ref, target_ref, state,
           required_approvals[], obtained_approvals[])
```

### 2.2 Channels (branches with meaning)

Every scope (org / department / team / user) has three standing channels per asset type:

- **`derived`** — auto-committed by the ingestion pipeline. Agents' extracted memories land
  here continuously. Readable per policy, clearly watermarked as unreviewed.
- **`staged`** — proposals under review live here.
- **`published`** — the trusted channel. `inject` composes **from `published` (+ `derived`
  where policy allows)**. Regulated-strict policy packs can restrict injection to
  `published`-only for designated scopes — that single switch is the "bank mode".

### 2.3 The lifecycle

```
agent session ──observe──▶ extraction ──▶ commit to {user|team}/derived      (automatic)
                                              │
                                   promotion proposal                        (human or
                                              ▼                               rule-driven)
                                        {scope}/staged ──review──▶ {scope}/published
                                              ▲
manual authoring (prompt, skill, ────────────┘
context pack, pinned memory, policy)
```

- **Promotion rules** can auto-open proposals: e.g. "a `procedure` memory recalled >N times
  across ≥3 team members → propose promotion to team/published".
- **Cross-scope promotion** (team → department → org) is a proposal against the higher scope,
  requiring that scope's approvers. This is how tribal knowledge climbs the org gradient with
  governance at each step.
- **Policy packs and lapses are themselves assets** flowing through VedaFlow — a lapse *is* a
  proposal with mandatory dual approval and an expiry commit scheduled by Temporal.

### 2.4 Approval matrix (CODEOWNERS, generalised)

Required approvals resolve from **(asset type × sensitivity × target scope × policy pack)**:

| Example | Required |
|---|---|
| Memory → `team/published`, internal | 1 × team `curator` |
| Prompt → `department/published` | 1 × dept `steward` + 1 × any `curator` (peer review) |
| Skill (executable!) → any `published` | steward + **security-reviewer role**; skills are treated like code because they are |
| Anything `restricted` sensitivity | + `compliance` role, dual approval |
| Policy lapse under regulated-strict | 2 × steward at target scope + auto-expiry mandatory |
| SMB `standard` pack | most of the above collapses to single-approver or auto-approve |

Reviews happen in the admin console or via a CLI (`synveda proposal review 142 --approve`),
and the git bridge means they can *also* surface as GitHub PRs for engineering-culture teams.

### 2.5 What this buys, concretely

- **Reproducibility**: `inject` responses cite commit hashes → "what did the agent know on
  March 3rd" is `synveda inject --as-of 2026-03-03` (bitemporal + refs).
- **Rollback**: bad prompt shipped? `refs` move back one commit; every consuming agent heals
  on next session start.
- **Blame/lineage**: every published sentence of context traces to an author or a source
  session, through an approval, under a recorded policy version.
- **Audit story**: the auditor reads proposals, not database rows.

---

## 3. Read/write paths (end-to-end)

**`inject`** (hot path, target p99 <150ms):
JWT verify → tenant/scope resolution (cached) → Cedar authorize (in-process, ~µs) →
composition engine reads `published` refs for scope chain (org→…→user) + policy-permitted
`derived` → hybrid retrieve within candidates (pgvector + Tantivy, RRF fusion) → budgeted
assembly (specificity gradient, pinned-first) → watermark with commit hashes → audit event →
return block.

**`observe`** (never blocks):
Gateway authZ → PGMQ enqueue (ack <20ms) → Temporal workflow: redact/secret-scan → extract
(classify into fact/decision/procedure/…) → dedup & conflict-detect against existing records →
summarise → embed (TEI) → graph-link (adjacency tables, ADR-0043) → **commit to `derived`** →
maybe auto-open promotion proposal.
_Amended 2026-07-28 (ADR-0044, GRPH-2): the arrow before "commit to `derived`" is sequence, not
a transaction boundary — graph-link runs **inside** the write transaction, after the records it
describes exist and before the channel commit, so a record and every claim about it land
together or not at all._

**`recall`** (explicit tool):
Same authZ → richer retrieval incl. graph traversal + as-of queries → results carry
provenance + channel labels so the agent can weigh derived vs published.

---

## 4. Deployment profiles

| | SMB ("one command") | Enterprise regulated |
|---|---|---|
| Footprint | `docker compose up`: gateway binary, Postgres (pgvector+AGE+PGMQ), Rauthy, TEI | Helm: HA Postgres (Patroni/CloudNativePG), Qdrant option, Temporal cluster, customer IdP, regional data planes |
| Policy pack | `standard`, single-approver | `regulated-strict`, dual approval, published-only injection |
| Residency | single region | control plane global, data planes pinned per division/region |
| Keys | single KMS key | per-tenant keys, HSM/KMS pluggable, WORM audit export |

---

## 5. Revised build order

- **Phase 0 (wk 1)**: workspace scaffold per seed §8 + `synveda-vedaflow` crate added; compose
  file with Postgres(+extensions), Rauthy, Temporal; ADR-0001 (stack), ADR-0002 (Cedar over
  OPA), ADR-0003 (VedaFlow in Postgres, not git repos).
- **Phase 1 (wk 2–4) — the spine**: OIDC login → auto-provision hierarchy → observe →
  extraction → commit to `derived` → inject from scope chain with Cedar checks → hash-chained
  audit. Claude Code adapter live.
- **Phase 2 (wk 5–8) — VedaFlow**: objects/commits/refs/proposals, channels, approval matrix,
  promotion proposals, CLI review flow, prompts & context packs as first authored asset types,
  policy packs + lapses through VedaFlow, bitemporal as-of inject.
- **Phase 3 (wk 9–12) — enterprise surface**: SCIM + Entra/Okta, skills registry (with
  security-review gate), admin console v1 (proposals inbox = the hero screen), Qdrant adapter,
  residency routing, git bridge (export), Helm.
- **Phase 4 — ecosystem**: SDKs, LangGraph/OpenAI shims, importers (claude-mem, Cognee, mem0),
  benchmark + retrieval-quality eval harness, SOC2 control mapping doc.

---

## 6. Key risks & mitigations

- **pgvector ceiling** → trait-isolated from day one; Qdrant adapter is Phase 3, not a rewrite.
- **Cedar ReBAC limits** at deep hierarchies → `authorize()` facade; OpenFGA adapter path proven
  by a spike in Phase 2, before it's needed.
- **Graph layer earning its place** → graph features are additive (retrieval works without
  graph-links); degrade gracefully. (Was "AGE maturity"; ADR-0043 removed the engine risk by
  removing the engine, and the mitigation is unchanged — GRPH-3 is feature-flagged.)
- **Temporal operational weight for SMB** → PGMQ + a simple Rust worker covers SMB mode;
  Temporal required only for enterprise profile.
- **Extraction quality** (garbage memories poison trust) → derived channel is quarantined by
  design; published channel is the trust boundary; eval harness in Phase 4 measures extraction
  precision continuously.

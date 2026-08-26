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

One database engine for Knowledge, sessions, scopes, audit, versioning, durable jobs, and (initially) vectors
and graph. This is a feature: one backup story, one HA story, one thing to explain to a bank's
infrastructure review board.

| Concern | Choice | Licence | Rationale / scale-out path |
|---|---|---|---|
| System of record | **PostgreSQL 17** | PostgreSQL | Boring, auditable, runs anywhere incl. air-gapped |
| Vector search | **pgvector** (HNSW) | PostgreSQL | Fine to ~10–50M vectors per tenant shard. Scale-out: **Qdrant** (Rust, Apache-2.0) behind the same `VectorIndex` trait. Note: VectorChord/pgvecto.rs is Rust and faster but AGPL — optional adapter only |
| Sparse / lexical | Tenant-bound **Postgres FTS** | PostgreSQL | Lexical rank stays transactionally aligned with current Knowledge revisions; hybrid fusion with pgvector uses RRF in the gateway |
| Graph | **Indexed adjacency in plain Postgres** (bitemporal edge pair; named graphs as a mandatory discriminator) | — | Amended 2026-07-27 by GRPH-1/ADR-0043, was **Apache AGE**: the GRPH-4 spike measured adjacency 3–8× faster at 2.5× less storage. Relationship claims remain transactional with Knowledge. Ladder: materialised bounded expansion, then a dedicated engine with its own ADR and a licence exception (candidates: **IndraDB**, Rust, MPL; avoid SurrealDB/Memgraph — BSL) |
| Governed scopes | Plain Postgres (`scopes` + closure table) | — | Five parent-shapes, no organisational rank and no graph DB needed for tenancy |
| Durable jobs | Leased tenant-bound Postgres tables | PostgreSQL | Capture, erasure, import/export, skill/tool tests and re-encryption share one observable idempotent operation model |
| Workflow (complex) | **Temporal** | MIT | Optional boundary for future long-running cross-service workflows; current product mutations remain in the database-backed job model |
| Bitemporal versioning | Native tables (`tx_from/tx_to`, `valid_from/valid_to`) + triggers | — | No extension dependency; queryable "as-of" both dimensions |

### 1.2 Identity & policy — Rust-first

| Concern | Choice | Licence | Rationale |
|---|---|---|---|
| Authorisation (PDP) | **Cedar** (embedded) | Apache-2.0 | Amazon's policy language, **pure Rust, in-process** — no network hop on the hot read path; formally verified evaluator; policies-as-data suits VedaFlow versioning |
| Relationship checks | Cedar entity hierarchy (first choice); **OpenFGA** (Apache-2.0) adapter if ReBAC outgrows Cedar | Apache-2.0 | Start with one engine. The `authorize()` facade hides the choice |
| Why not OPA | Rego is powerful but adds a Go sidecar + network hop on every context decision; Cedar embeds in the gateway binary | | OPA remains a possible adapter for shops that mandate it, not a shipped runtime |
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
| Observability | OpenTelemetry on session, capture, Knowledge and ContextRun paths; Prometheus; Grafana/Jaeger |
| Packaging | Single static gateway binary + Postgres = SMB mode. Helm chart with regional data planes = enterprise mode |

### 1.4 Explicit non-choices

- **No Elasticsearch/OpenSearch** (JVM estate; Postgres FTS is the current lexical leg), **no Redis** initially
  (Postgres + moka in-process cache), **no Kafka** (leased Postgres jobs, with Temporal as an extension point), **no Neo4j** (licence),
  **no SurrealDB/Memgraph** (BSL). Every one of these is a door left open behind a trait, not a
  dependency taken today.

---

## 2. VedaFlow — git-style governance for knowledge assets

The insight: **treat organisational knowledge exactly like code**. Memories, context packs,
prompts, skills, and *policies themselves* flow through propose → review → approve → publish,
with approval authority derived from governed scopes and grants. Nothing reaches an agent that wasn't either
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
proposals (id, scope, source_ref, target_ref, state, commit_hash,
           typed_artifact_references[], proposer, close_actor)
proposal_approvals (proposal, commit_hash, approver, verdict, roles)
```

### 2.2 Channels and typed aggregate effects

Prompt and context-pack authored assets retain standing VedaFlow channels at
a governed scope:

- **`derived`** — machine-produced material, readable only where policy permits
  an explicitly unreviewed channel.
- **`staged`** — proposals under review live here.
- **`published`** — the trusted authored bundle.

Knowledge is not a channel member and is never published by attaching a record
id to a commit. A proposal instead carries a content-free, hash-bound typed
aggregate effect. Applying it creates or advances a stable Knowledge item and
its immutable revisions; personal auto-apply still creates and executes that
same proposal.

Skills use the same typed-effect shape: a stable Skill points at immutable
content-addressed versions and explicit project/principal bindings select or
pin what is advertised. There is no `skill/published` ref, mutable draft or
channel-wide Skill rollback. A binding revision is the distribution switch.
Tool servers and bindings, governed Configuration and policy relaxations use
that same proposal/change row. Its content-free typed references name the
stable artifact, operation, exact version or digest and stale-head
precondition; OKF publication carries both the Knowledge and import-artifact
references rather than opening an import-specific review queue.

### 2.3 The lifecycle

```
session events ──capture──▶ reviewable candidate ──accept/edit/merge/replace──┐
                                                                              │
manual Knowledge create/edit/verify/forget ──────────────────────────────────┤
                                                                              ▼
                                               typed VedaFlow change ──▶ Knowledge revision

prompt / context-pack authoring ──▶ staged review ──▶ published ref

Skill install/update/bind/rollback ──▶ typed VedaFlow change ──▶ immutable version/binding
```

- Capture output is a candidate, never active Knowledge. Accept, edit, merge or
  replace calls the same Knowledge command layer as manual authoring.
- Cross-scope publication is decided at the destination scope and uses that
  scope's approval matrix. A proposal cannot carry authority from its source.
- Policy profiles and time-boxed relaxations are governed artifact families;
  they never create a second authorisation path.

### 2.4 Approval matrix (CODEOWNERS, generalised)

Required approvals resolve from **(asset type × sensitivity × target scope × policy pack)**:

| Example | Required |
|---|---|
| Knowledge → project scope, internal | policy may auto-apply or require a project `curator` |
| Prompt → workspace `published` | 1 × `administrator` + 1 × `curator` (peer review) |
| Skill version or binding change | live pack matrix, including the invariant security-reviewer requirement; skills are treated like code because they are |
| Anything `restricted` sensitivity | + distinct `reviewer`; the author cannot review |
| Policy relaxation under regulated-strict | distinct approvers, separate effect actor + hard expiry mandatory |
| SMB `standard` pack | most of the above collapses to single-approver or auto-approve |

Reviews happen in Advanced → Reviews or via the proposal CLI. A verdict binds
the exact commit the reviewer read; the gateway refuses a moved commit before
recording the act. Rules can require the reviewer to differ from the author and
the effect actor to differ from both author and counting reviewers. Applying or
publishing remains a separate PDP-authorised act and repeats artifact revision
checks. The author cancels through the one withdrawal transition; rejection is
a reviewer verdict with a reason. The git bridge may also surface authored
channel reviews as pull requests for engineering-culture teams.

### 2.4.1 Approval threat boundary

Typed references, approval counts and separation constraints narrow an
otherwise authorised act; none grants authority. Proposal reads, verdicts and
effects independently cross Cedar at the target scope, and forced RLS confines
the common rows by tenant. References and ordinary audit events contain only
ids, operations, versions/digests, reasons and policy evidence—never Knowledge
body text, Skill files, Tool credentials or Configuration documents. A denied
artifact cannot be discovered by probing the family filter because listing
still authorises and renders each visible proposal under its own scope.

### 2.5 What this buys, concretely

- **Reproducibility**: context selections cite immutable revision ids and
  rendered hashes; valid-time and transaction-time lenses answer what was true
  and what the system knew at an instant.
- **Rollback**: bad prompt shipped? `refs` move back one commit; every consuming agent heals
  on next session start.
- **Blame/lineage**: every published sentence of context traces to an author or a source
  session, through an approval, under a recorded policy version.
- **Audit story**: the auditor reads the tenant-complete hash chain through
  `AuditRead`, not application tables. Typed artifact/context filters cite
  canonical events; bitemporal Knowledge answers distinguish semantic valid
  time from transaction/as-known time; a frozen-prefix export carries every
  canonical hash input for offline verification. These are recorded decisions,
  never a historical Cedar replay, and ordinary evidence resolves no content
  or secret.

---

## 3. Read/write paths (end-to-end)

**Append a session event** (never blocks the agent on downstream work): JWT
verify → derive tenant/principal/project from bearer and session → ownership
check → Cedar decision → idempotent immutable append → content-free audit →
asynchronous capture trigger.

**Capture**: select real session events → redact/validate model output → compare
with visible current Knowledge → create duplicate/conflict/supersession hints →
persist reviewable candidates only. Acceptance calls a typed VedaFlow Knowledge
command; extraction never writes active Knowledge directly.

**Compose a session context run** (hot path, target p99 <150ms): ownership and
PDP → current Knowledge lexical/semantic anchors → policy filtering → bounded
selection under the token budget → rendered block + immutable revision ids +
reason codes + hash → audit. Denied candidates never enter trace details.

**Scoped query/recall**: the same current/as-of Knowledge rules with a separate
authorisation and pagination contract for deep query or evaluation. There is no
global `/v1/recall` route and no direct-store adapter path.

---

## 4. Deployment profiles

| | SMB ("one command") | Enterprise regulated |
|---|---|---|
| Footprint | `docker compose up`: gateway binary, Postgres + pgvector, Rauthy, TEI and optional Temporal | Helm: one gateway replica, CloudNativePG + pgvector, optional TEI/Temporal, customer IdP |
| Policy pack | `standard`, single-approver | `regulated-strict`, dual approval, published-only context |
| Residency | single region | control plane global, data planes pinned per division/region |
| Keys | local deployment KEK wrapping deployment and per-tenant DEKs | the same shipped local provider; cloud KMS/HSM/CMK and WORM custody are extension points, not current support |

---

## 5. Revised build order

- **Phases 0–2 — delivered foundation and governance proof**: workspace,
  Postgres-first stack, OIDC, embedded Cedar, RLS, hash-chained audit and
  VedaFlow objects/commits/refs/proposals. Their fixed hierarchy, record and
  global runtime-route implementations are replaced rather than preserved.
- **Phase 3 — paused enterprise surface**: the delivered skill, directory,
  console, deployment and key-plane foundations are re-anchored by explicit
  Phase 5 packages before the remaining enterprise backlog resumes.
- **Directory boundary (CPR-34/ADR-0093)**: SCIM push and scheduled pull
  project onto one identity/principal and the shared Group graph. Membership
  is identity-keyed; provider source/resource ids remain provenance; only a
  separately PDP-governed `scope_grants` assignment turns a directory group
  into product authority. There is no directory-only permission model.
- **Phase 4 — ecosystem**: SDKs, adapters, import/export, telemetry, DR and
  scale-out work follows the public context-platform contract.
- **Phase 5 — context platform hard cut (current)**: generic governed scopes;
  workspace/project/session runtime; stable Knowledge and immutable revisions;
  capture candidates; explainable retrieval; versioned skills, tools and
  configuration; one generated application contract; security/evaluation/demo
  gates; then one clean pre-1.0 baseline schema. ADR-0068 locks the programme.

---

## 6. Key risks & mitigations

- **pgvector ceiling** → trait-isolated from day one; Qdrant adapter is Phase 3, not a rewrite.
- **Cedar ReBAC limits** at deep hierarchies → `authorize()` facade; OpenFGA adapter path proven
  by a spike in Phase 2, before it's needed.
- **Graph layer earning its place** → graph features are additive (retrieval works without
  graph-links); degrade gracefully. (Was "AGE maturity"; ADR-0043 removed the engine risk by
  removing the engine, and the mitigation is unchanged — GRPH-3 is feature-flagged.)
- **Temporal operational weight** → no core path forks by deployment profile:
  sessions and capture run in the gateway/Postgres runtime; Temporal remains
  extension infrastructure until a feature proves a workflow needs it.
- **Extraction quality** (garbage memories poison trust) → derived channel is quarantined by
  design; published channel is the trust boundary; eval harness in Phase 4 measures extraction
  precision continuously.

# SYNVEDA — Project Seed Prompt

> Feed this document to your coding agent as the founding context for all work on Synveda.
> It defines what the product is, the invariants that must never be violated, the architecture,
> the domain model, and the build order. When in doubt, this document wins.

---

## 1. Identity

**Synveda** (Greek *syn*, "together" + Sanskrit *veda*, "knowledge") is a
**memory and context management platform for AI agents**. One tenant-bound,
policy-enforced runtime serves an individual, a small team and a future
multi-region regulated enterprise; profiles change governed policy and
configuration, never the domain model or binary.

**Positioning**: the hybrid of claude-mem (seamless hook-driven context injection for Claude Code)
and Cognee (organisational knowledge/memory engine), rebuilt from scratch as a trustworthy product:
identity-aware, scope-aware, audit-first, policy-enforced at a central decision point — not
policy-suggested in a config file.

**One-line pitch**: *Shared knowledge for agent fleets — governed like a bank, effortless like a consumer app.*

**What it is NOT**: not an agent framework, not an orchestrator, not a vector DB wrapper, not a
RAG pipeline product. It is the memory/context/skills control plane that any harness plugs into.

---

## 2. Product principles (non-negotiable)

1. **Zero-config by default.** A user logs in and gets a principal scope;
   supported product flows create workspaces/projects and grants. Directory
   claims may supply groups and grants through an adapter, but the core never
   invents a fixed organisation from IdP shape. No YAML before value.
2. **Policy is enforced, never advisory.** Every read and write passes through a Policy Decision
   Point (PDP). There is no code path from harness to storage that bypasses it.
3. **Strict by default, relaxable by design.** The default policy is deny-first
   and fully audited. A relaxation names one provisioned subject, exact
   non-personal scope, permission, tier and hard expiry; it is a governed,
   audited VedaFlow change and Configuration may only narrow it.
4. **Separation of concerns is architectural, not stylistic.** Transport adapters know nothing of
   policy. Policy knows nothing of storage. Storage knows nothing of identity. Each layer is
   independently testable and replaceable.
5. **Audit is a first-class output.** Decisions, context delivery and query,
   writes and policy changes produce content-minimised evidence in the
   tamper-evident tenant audit chain.
6. **The harness is a guest.** Claude Code, MCP clients, LangGraph, custom SDKs — all consume the
   same three primitives. Supporting a new harness must never require touching the core.
7. **Data residency is a placement and routing constraint, not a label.** The
   current runtime is single-region and makes no residency-routing claim. Any
   future regional control plane must enforce tenant placement under the same
   PDP, RLS, audit and key boundaries.

---

## 3. The three integration capabilities

Every harness integration reduces to:

| Capability | Direction | Description |
|-------------|-----------|-------------|
| Session context | read | Compose a token-budgeted block for an authenticated session/task through a session-scoped context run. |
| Scoped Knowledge query | read | Query current or temporal Knowledge through a project- or session-scoped, independently authorised API. |
| Session events | write | Append idempotent immutable transcript/tool/decision events; downstream capture never blocks the append. |

---

## 4. Domain model

### 4.1 Governed scopes

```
Tenant
├── Org unit                 (optional and recursively nestable)
├── Workspace
│   └── Project
└── Principal                (personal privacy boundary)
```

- Modelled as `scopes` + `scope_closure` in PostgreSQL. A scope kind is a
  parent-shape constraint, never an organisational rank.
- Workspaces, projects and principals are product-level subtypes that own one
  governed scope. A principal scope inherits no grant from above.
- Directory adapters map external principals, groups, memberships and access
  onto the same shared aggregates; directory structure is not the core tree.
- Knowledge, skills, prompts, tools, policy and governed configuration attach
  to these scopes and are decided by the embedded Cedar PDP.

### 4.2 Knowledge aggregates

Knowledge has stable identity and immutable content revisions. Every item
carries:

- `id`, tenant, governing scope, optional project and optional owning principal
- `type`: `fact` | `decision` | `preference` | `procedure` | `entity` |
  `episode` | `convention` | `warning` | `reference`
- an immutable current revision with title, Markdown body, summary, tags,
  confidence, verification, hashes and extensible metadata
- normalised, independently authorised provenance sources and explicit
  relations between stable items
- `sensitivity`: `public` | `internal` | `confidential` | `restricted` (drives policy)
- valid time plus database transaction time, with current and historical
  projections
- lifecycle: active, stale, superseded, archived, erasure-pending or erased

Every mutation is a typed VedaFlow change. Auto-apply is a policy outcome, not
a path around the change, immutable revision, PDP or audit chain.

### 4.3 Managed artifact families

Synveda manages artifact families with the same scope/policy/audit machinery:

1. **Knowledge** — stable aggregates and immutable revisions (above)
2. **Context packs** — curated, versioned bundles (docs, conventions, glossaries) pinned to scopes
3. **Skills** — versioned skill definitions (SKILL.md-style) distributed to agents by scope
4. **Prompts** — versioned prompt templates with approval workflow (draft → review → published)
5. **Tools and governed configuration** — versioned registries and bindings

All families: versioned, scope-attached, policy-gated, auditable, with rollback.

### 4.4 Scope composition (the read path contract)

A session context run composes current policy-visible Knowledge under a
configurable token budget. Candidate retrieval and selection remain distinct;
superseded or archived revisions are not current truth, and every selected
revision retains source evidence and an explainable address. Specific scopes
may outrank wider scopes only inside the governed selection policy; a deeper
forbid always overrides a wider permit.

---

## 5. Identity & access

- **SSO**: OIDC (auth code + PKCE) against any compliant IdP. First login
  provisions the identity and its own principal-shaped scope.
- **SCIM 2.0** for lifecycle (joiners/movers/leavers), implemented as an
  adapter onto shared principals, groups, memberships and scope grants.
- **Service identities** for headless agents: OAuth2 client-credentials with scoped, short-lived
  tokens; every agent runs *as* an identity at a governed scope, never as a shared key.
- **Role keys** granted at a scope and inherited by its subtree: `owner`,
  `member`, `viewer`, `reviewer`, `curator`, `administrator`. There is no
  permission table; Cedar policy packs decide what a key permits.

---

## 6. Policy engine

- **PDP**: Cedar embedded in the gateway, fronted by one internal
  `authorize(subject, action, resource, context)` seam. No policy sidecar or
  second permission mapping participates in a decision.
- **Policy packs** — versioned bundles applied per governed scope:
  - `regulated-strict` (default): own-chain or explicitly granted reads;
    confidential content needs an explicit content role and restricted content
    needs a governed relaxation. Strict capture scanning/redaction is the
    matching Configuration default.
  - `standard`: working-tier reads extend one governed-scope step around the
    caller's actual grants; no organisational rank is inferred.
  - `open-collaboration`: tenant-wide non-personal reads below the restricted
    tier; the personal-scope privacy floor still applies.
- **Controlled relaxations**: a subject may request an exact scoped, reasoned,
  time-boxed policy change. It is a governed artifact with hard expiry,
  approvals and audit, never a second authorisation path.
- **Residency policies**: a future hosting control-plane concern tracked by
  OPS-3. The current deployment is single-region and makes no cross-region
  routing claim.
- **Redaction pipeline**: session capture scans bounded event evidence before
  creating reviewable candidates. Session-event persistence and candidate
  admission are separate policy boundaries.

---

## 7. Architecture

```
┌──────────────── Harness adapters (public-API clients) ────────────┐
│ Claude Code adapter (TS hooks + spool) │ `synveda mcp` (Rust CLI)  │
└────────────────────────────┬───────────────────────────────────────┘
                             │  sessions, Knowledge and context APIs
┌────────────────────────────▼───────────────────────────────────────┐
│ GATEWAY (Rust, axum)                                                │
│  AuthN → tenant resolution → ownership → Cedar PDP → RLS transaction│
│  → typed effect and content-minimised audit                         │
└────────────────────────────┬───────────────────────────────────────┘
┌────────────────────────────▼───────────────────────────────────────┐
│ CORE (Rust)                                                         │
│  read: Knowledge search + budgeted context planning                 │
│        hybrid retrieval (dense + sparse + bounded graph)            │
│  write: immutable session events + typed VedaFlow commands          │
└──────┬──────────────────────────────────────────┬──────────────────┘
┌──────▼──────────────┐                 ┌─────────▼──────────────────┐
│ POSTGRES 17         │                 │ LEASED DATABASE WORKERS     │
│ Knowledge, sessions,│                 │ capture, index convergence, │
│ scopes, versions,   │                 │ import and re-encryption     │
│ audit, jobs, FTS,   │                 │ run in the gateway process   │
│ pgvector, relations │                 │ and remain restart-safe      │
└─────────────────────┘                 └─────────────────────────────┘
Cross-cutting: embedded Cedar PDP · standards-based OIDC · OTel traces/metrics
Deploy: source/installed Compose or Helm, currently one gateway replica
```

**Language decisions**: core/gateway in **Rust** (single static binary, on-prem friendly,
latency-critical read path). Claude Code adapter in **TypeScript** (hooks ecosystem).
The admin console is React and uses the generated OpenAPI client. Public Rust,
TypeScript and Python SDKs remain open work; deleted stubs are not support.

> **Footnote, added by ADPT-2 (ADR-0057, amended 2026-08-05).** The *generic MCP
> server* in the adapters row ships as `synveda mcp`, a subcommand of the Rust CLI,
> rather than as its own TypeScript package: the official TS MCP SDK does not
> implement the `2026-07-28` revision and the Rust one does. It stays in this row by
> **behaviour** — a gateway client over `/v1` holding a bearer, three primitives only
> — but it now lives in a binary that also links `synveda-store`, `synveda-identity`,
> `synveda-policy` and `synveda-audit` for its dev-bootstrap commands. So for that one
> adapter the arrow's *three primitives only* is a **review obligation rather than a
> structural guarantee**, and `crates/synveda-cli/src/mcp.rs` carries a test that fails
> on any reference to a core crate. The `claude-code` adapter no longer speaks MCP
> itself either; its `mcpServers` entry launches the same binary.

**Dependency licensing/stack constraint**: the shipped core path admits only
the repository's approved permissive dependency licences. PostgreSQL,
pgvector, Cedar, Rauthy and the Rust/TypeScript runtime are current; optional
engines and hosting services require a separate accepted decision. This
constraint does not choose a licence for Synveda itself.

---

## 8. Separation of concerns — module map

Monorepo, Rust workspace + pnpm workspace:

```
synveda/
├── crates/
│   ├── synveda-types        # domain types, IDs, errors — zero deps on other crates
│   ├── synveda-crypto       # envelope keys and audit-safe crypto boundaries
│   ├── synveda-policy       # embedded Cedar facade + policy pack loader
│   ├── synveda-store        # Postgres/pgvector Knowledge and platform state
│   ├── synveda-retrieval    # hybrid search, rerank, composition engine
│   ├── synveda-ingest       # extraction, redaction, embeddings and capture worker logic
│   ├── synveda-audit        # tamper-evident audit log (hash-chained), export
│   ├── synveda-vedaflow     # immutable objects, commits, refs and proposals
│   ├── synveda-identity     # OIDC, SCIM and directory adapters
│   ├── synveda-okf          # pure bounded OKF v0.2 exchange adapter
│   ├── synveda-gateway      # axum HTTP application plane and DB-leased workers
│   ├── synveda-cli          # admin/dev CLI (synveda init, synveda policy apply, ...)
│                            #   + `synveda mcp`: the generic MCP server (see §7 footnote)
│   └── synveda-eval         # unprivileged public-API evaluation client
├── adapters/
│   └── claude-code/         # TS: SessionStart/PreCompact/Stop hooks; its MCP
│                            #   entry launches `synveda mcp` (ADR-0057 decision 4)
├── console/                 # React application generated from OpenAPI
├── policies/                # Cedar policy packs
├── deploy/                  # Compose and single-region Helm shapes
└── docs/ (ADRs — every architectural decision gets an ADR from day one)
```

Dependency rule: `types ← crypto ← {policy, store, identity, audit, vedaflow}
← retrieval/ingest ← gateway`; `synveda-okf` is a types-only format leaf.
Nothing imports "upward". Adapters and future SDKs depend only on the public
API, never on crates. The check enumerates the CLI's local bootstrap exceptions
and keeps the evaluation crate dependency-free.

---

## 9. Build order (vertical slices, each independently demoable)

**Phase 0 — Skeleton (delivered)**
Repo scaffold, CI, `synveda-types`, ADR-0001, Docker Compose and the
Postgres-first development stack.

**Phase 1 — The original spine (delivered, runtime model replaced in Phase 5)**
OIDC, Cedar, RLS, audit and the first Claude Code slice proved the thesis. The
fixed hierarchy, global observe/inject/recall routes and record aggregate from
that slice are deleted by the pre-1.0 Phase 5 cut; their invariants survive on
governed scopes, sessions and Knowledge.

**Phase 2 — Governance depth (delivered, later re-cut)**
Policy packs, governed relaxations, sensitivity, redaction, audit query,
authored context packs and bitemporal Knowledge queries survive on the current
scope/Knowledge model.

**Phase 3 — Enterprise surface (paused, partially delivered)**
SCIM, directory projection, Skills, Tools, the console and Helm foundations are
implemented. Live Entra/Okta evidence, scale-out, tenancy operations and
regional routing remain open in the backlog.

**Phase 4 — Ecosystem (open)**
SDKs, importers and additional harness adapters are not current product claims.
The committed evaluation suite covers the implemented context platform.

**Phase 5 — Context platform hard cut (current runtime)**
One scope tree and role vocabulary; workspace/project/session runtime; stable
Knowledge with immutable revisions and provenance; capture candidates;
explainable context planning; versioned skills/tools/configuration; public API
and generated console client; adversarial acceptance; one clean pre-1.0 schema.
The second verified client remains externally blocked under CPR-39.

---

## 10. Non-functional requirements

- Current engineering budgets are context p99 below 150 ms, session-event ack
  below 20 ms and capture lag below 60 seconds under their documented local
  test conditions. They size timeouts, buckets and regression tests; they are
  not production SLOs. EVAL-6 must establish supported workload, hardware and
  production p50/p95/p99 objectives before any service claim.
- Audit is append-only and hash-chained in Postgres. WORM custody and SIEM
  delivery remain AUD-3/AUD-4 work.
- Tenant envelope keys protect the implemented secret/content paths. Complete
  storage encryption, KMS/HSM custody and restore ceremonies remain readiness
  requirements rather than current claims.
- Pre-1.0 schema-epoch hard cuts are explicit: old databases are refused with
  a reset instruction, with no data migrator or compatibility reader. After
  1.0, migration/compatibility policy requires its own accepted ADR.
- SOC 2 / ISO 27001 mapping remains open as AUD-5.
- PDP, RLS, VedaFlow, erasure and context selection require adversarial and
  behaviour-level tests. Coverage percentages are reported only when measured.

---

## 11. Instructions to the coding agent

Follow `AGENTS.md`, the current feature record and the accepted ADRs for the
area being changed. Do not restart the historical build order. When prose
conflicts with executable code, generated contracts or current accepted ADRs,
fix the prose or raise the decision before implementation; §2 remains the
product invariant.

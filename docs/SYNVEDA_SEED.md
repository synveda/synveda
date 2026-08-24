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
3. **Strict by default, relaxable by design.** The default policy pack assumes a regulated
   environment (deny-first, full audit, no cross-team reads). Administrators can *lapse* controls
   per org/department/team through explicit, audited, time-boxable policy relaxations — never by
   editing code.
4. **Separation of concerns is architectural, not stylistic.** Transport adapters know nothing of
   policy. Policy knows nothing of storage. Storage knows nothing of identity. Each layer is
   independently testable and replaceable.
5. **Audit is a first-class output.** Every decision (allow/deny), injection, recall, write, and
   policy change is recorded in a tamper-evident log. Synveda should be the *easiest* system in
   the estate to take through an audit.
6. **The harness is a guest.** Claude Code, MCP clients, LangGraph, custom SDKs — all consume the
   same three primitives. Supporting a new harness must never require touching the core.
7. **Data residency is a routing concern.** Multi-region orgs pin data to regions by policy;
   the control plane is global, the data plane is regional.

---

## 3. The three primitives

Every harness integration reduces to:

| Primitive   | Direction | Description |
|-------------|-----------|-------------|
| `inject`    | read      | "Give me a token-budgeted context block for this identity + session + task." Entered through the session-scoped context-run API. Silent, fast (<150ms p99 target). |
| `recall`    | read      | Explicit deep query: hybrid retrieval + bounded graph traversal, richer and slower. Exposed only through a project- or session-scoped application API and adapters over it. |
| `observe`   | write     | "Here is what happened" (transcript deltas, tool results, decisions). Entered as idempotent immutable session events and processed async — never blocks the session. |

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
  - `regulated-strict` (default): deny-first; no cross-team read without explicit grant; all
    writes classified; export blocked; retention enforced; PII redaction on ingest.
  - `standard`: team-shares-by-default within department; lighter classification.
  - `open-collaboration`: org-wide read for non-restricted content.
- **Controlled relaxations**: a subject may request an exact scoped, reasoned,
  time-boxed policy change. It is a governed artifact with hard expiry,
  approvals and audit, never a second authorisation path.
- **Residency policies**: data-plane region pinning per node; cross-region `inject` returns only
  metadata-safe summaries unless policy allows replication.
- **Redaction pipeline**: configurable PII/secret detection on `observe` before persistence
  (deny, redact, or quarantine-for-review).

---

## 7. Architecture

```
┌──────────────── Harness adapters (thin, stateless) ────────────────┐
│ claude-code adapter (TS: hooks + MCP)  │  generic MCP server        │
│ REST/gRPC SDKs (Rust, TS, Python)      │  LangGraph/OpenAI shims    │
└────────────────────────────┬───────────────────────────────────────┘
                             │  three primitives only
┌────────────────────────────▼───────────────────────────────────────┐
│ GATEWAY (Rust, axum)                                                │
│  AuthN (OIDC/JWT) → tenant resolution → PDP check → rate limits     │
│  → audit event emission (every request, every decision)             │
└────────────────────────────┬───────────────────────────────────────┘
┌────────────────────────────▼───────────────────────────────────────┐
│ CORE (Rust)                                                         │
│  read: Knowledge search + budgeted context planning                 │
│        hybrid retrieval (dense + sparse + bounded graph)            │
│  write: immutable session events; typed VedaFlow commands           │
└──────┬──────────────────────────────────────────┬──────────────────┘
┌──────▼──────────────┐                 ┌─────────▼──────────────────┐
│ STORAGE             │                 │ ASYNC PLANE (Temporal)      │
│ Postgres: Knowledge,│                 │ capture candidates, index   │
│  sessions, scopes,  │                 │ convergence, durable erase  │
│  audit + versions   │                 │ directory/import jobs       │
│ pgvector (default)  │                 │ consolidation, decay/TTL,   │
│ Qdrant (scale-out)  │                 │ re-embedding, directory sync│
│ indexed graph edges │                 └─────────────────────────────┘
└─────────────────────┘
Cross-cutting: embedded Cedar PDP · standards-based OIDC · OTel traces/metrics
Deploy: single binary + Postgres for SMB │ Helm chart, regional data planes for enterprise
```

**Language decisions**: core/gateway in **Rust** (single static binary, on-prem friendly,
latency-critical read path). Claude Code adapter in **TypeScript** (hooks ecosystem). SDKs:
Rust, TS, Python. Admin console: React (later phase; API-first until then).

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

**Licensing/stack constraint**: permissive or self-hostable OSS only — PostgreSQL, pgvector,
Apache AGE, Qdrant, OPA, OpenFGA, Temporal, Keycloak-compatible OIDC. No cloud-locked services
in the core path.

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
│   ├── synveda-ingest       # extraction, redaction, dedup, summarisation (Temporal activities)
│   ├── synveda-audit        # tamper-evident audit log (hash-chained), export
│   ├── synveda-vedaflow     # immutable objects, commits, refs and proposals
│   ├── synveda-identity     # OIDC, SCIM and directory adapters
│   ├── synveda-gateway      # axum HTTP/gRPC; the ONLY binary that speaks to the outside
│   └── synveda-cli          # admin/dev CLI (synveda init, synveda policy apply, ...)
│                            #   + `synveda mcp`: the generic MCP server (see §7 footnote)
├── adapters/
│   └── claude-code/         # TS: SessionStart/PreCompact/Stop hooks; its MCP
│                            #   entry launches `synveda mcp` (ADR-0057 decision 4)
├── sdks/ (rust, typescript, python)
├── policies/                # Cedar policy packs
├── deploy/ (docker-compose single-node, helm multi-region)
└── docs/ (ADRs — every architectural decision gets an ADR from day one)
```

Dependency rule: `types ← crypto ← {policy, store, identity, audit, vedaflow}
← retrieval/ingest ← gateway`.
Nothing imports "upward". Adapters and SDKs depend only on the public API, never on crates.

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

**Phase 2 — Governance depth (weeks 5–8)**
Policy packs, lapses with approval + expiry, sensitivity classification, redaction pipeline,
auditor role + audit export, pinned assets (context packs), bitemporal queries ("what did the
agent know on date X" — the killer regulated-industry demo).

**Phase 3 — Enterprise surface (weeks 9–12)**
SCIM, real IdP integrations (Entra/Okta), skills + prompt registries with approval workflow,
admin console v1, Qdrant scale-out option, multi-region data plane routing, Helm chart.

**Phase 4 — Ecosystem**
Python/TS SDKs polished, LangGraph/OpenAI shims, benchmark suite (injection latency, retrieval
quality), migration importers (claude-mem, Cognee, mem0 export formats).

**Phase 5 — Context platform hard cut (in progress)**
One scope tree and role vocabulary; workspace/project/session runtime; stable
Knowledge with immutable revisions and provenance; capture candidates;
explainable context planning; versioned skills/tools/configuration; public API
and generated clients; adversarial acceptance; one clean pre-1.0 schema.

---

## 10. Non-functional requirements

- `inject` p99 < 150ms at 1K concurrent sessions (excluding first-call cold cache)
- `observe` ack < 20ms (enqueue only); pipeline lag SLO < 60s
- Audit log: append-only, hash-chained, exportable to WORM storage
- All data encrypted at rest (per-tenant keys, KMS-pluggable) and in transit (mTLS internal)
- Pre-1.0 schema-epoch hard cuts are explicit: old databases are refused with
  a reset instruction, with no data migrator or compatibility reader. After
  1.0, migration/compatibility policy requires its own accepted ADR.
- SOC 2 / ISO 27001 control mapping documented from Phase 2 (design for it, don't retrofit)
- Test bar: policy engine and composition engine at 100% branch coverage; property-based tests
  for conflict resolution; every PDP decision path has a golden test

---

## 11. Instructions to the coding agent

1. Read this document fully before any code. Ask clarifying questions only where two sections
   conflict; otherwise proceed.
2. Start with Phase 0. Produce the workspace scaffold exactly per §8, with compiling empty
   crates, CI (fmt, clippy -D warnings, test), and docker-compose.
3. Write ADR-0001 summarising §2 and §7 decisions. Every subsequent architectural choice gets
   its own numbered ADR.
4. Never introduce a path around the PDP, even in tests — use a test policy pack instead.
5. Prefer boring technology and explicit code over cleverness. This system's selling point is
   trustworthiness.
6. Each phase ends with a runnable demo script under `demos/` proving the slice works.

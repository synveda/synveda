# SYNVEDA — Project Seed Prompt

> Feed this document to your coding agent as the founding context for all work on Synveda.
> It defines what the product is, the invariants that must never be violated, the architecture,
> the domain model, and the build order. When in doubt, this document wins.

---

## 1. Identity

**Synveda** (Greek *syn*, "together" + Sanskrit *veda*, "knowledge") is an **enterprise memory and
context management platform for AI agents**. It gives organisations a governed, multi-tenant,
policy-enforced layer for the memory, context, skills, and prompts their agents depend on —
from a 10-person SMB to a multi-region regulated bank.

**Positioning**: the hybrid of claude-mem (seamless hook-driven context injection for Claude Code)
and Cognee (organisational knowledge/memory engine), rebuilt from scratch as an enterprise product:
SSO-native, hierarchy-aware, audit-first, policy-enforced at a central decision point — not
policy-suggested in a config file.

**One-line pitch**: *Shared knowledge for agent fleets — governed like a bank, effortless like a consumer app.*

**What it is NOT**: not an agent framework, not an orchestrator, not a vector DB wrapper, not a
RAG pipeline product. It is the memory/context/skills control plane that any harness plugs into.

---

## 2. Product principles (non-negotiable)

1. **Zero-config by default.** A user logs in with SSO and everything works. Their scopes, teams,
   policies, and injection defaults are derived automatically from the identity provider's claims
   and group memberships. No YAML before value.
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
| `inject`    | read      | "Give me a token-budgeted context block for this identity + session + task." Called at session start / pre-compact. Silent, fast (<150ms p99 target). |
| `recall`    | read      | Explicit deep query: hybrid retrieval + graph traversal, richer and slower. Exposed as a single MCP tool / API endpoint. |
| `observe`   | write     | "Here is what happened" (transcript deltas, tool results, decisions). Queued, processed async — never blocks the session. |

---

## 4. Domain model

### 4.1 Tenancy hierarchy

```
Organisation
└── Division / Region        (optional levels — hierarchy depth is configurable)
    └── Department
        └── Team
            └── User
```

- Modelled as a **closure-table / materialised-path hierarchy in PostgreSQL**, mirrored into
  **OpenFGA relationship tuples** for authorisation checks.
- Hierarchy is **provisioned automatically** from the IdP: SCIM 2.0 push and/or scheduled
  directory sync (Entra ID, Okta, Google Workspace, generic OIDC+LDAP). Group-to-node mapping
  rules are configurable, with sane conventions out of the box (e.g. `synveda-{dept}-{team}`).
- Every node in the hierarchy is a **scope** to which memories, skills, prompts, and policies attach.

### 4.2 Memory records

The atomic unit. Every record carries:

- `id`, `tenant_id`, `scope` (node in hierarchy), `owner` (user or service identity)
- `kind`: `derived` (extracted by the pipeline) | `pinned` (authored/canonical — cannot be
  shadowed or decayed; the Shruti/Smriti split)
- `class`: `fact` | `decision` | `preference` | `procedure` | `entity` | `episode`
- `content` (summarised at write time), `embedding_ref`, `graph_refs`
- `provenance`: source session, extraction method, model version, confidence
- `sensitivity`: `public` | `internal` | `confidential` | `restricted` (drives policy)
- `temporal`: valid-from / valid-to (bitemporal: transaction time + valid time)
- `ttl` / decay policy reference

### 4.3 Beyond memory: the four managed asset types

Synveda manages four asset classes with the same scope/policy/audit machinery:

1. **Memories** — derived and pinned records (above)
2. **Context packs** — curated, versioned bundles (docs, conventions, glossaries) pinned to scopes
3. **Skills** — versioned skill definitions (SKILL.md-style) distributed to agents by scope
4. **Prompts** — versioned prompt templates with approval workflow (draft → review → published)

All four: versioned, scope-attached, policy-gated, auditable, with rollback.

### 4.4 Scope composition (the read path contract)

On `inject`, context is composed by **specificity gradient**: user > team > department > division > org,
with pinned records taking priority within each level, under a configurable token budget
(default 1,500 tokens). Conflicts resolve by (1) pinned beats derived, (2) more specific scope
beats less specific, (3) newer valid-time beats older. Every injected block is watermarked with
record IDs for auditability.

---

## 5. Identity & access

- **SSO**: OIDC (auth code + PKCE) against any compliant IdP; SAML bridge for legacy. First
  login auto-provisions the user into the hierarchy from claims/groups.
- **SCIM 2.0** for lifecycle (joiners/movers/leavers). A leaver's personal scope is sealed
  (retained per retention policy, no longer readable by default).
- **Service identities** for headless agents: OAuth2 client-credentials with scoped, short-lived
  tokens; every agent runs *as* an identity in the hierarchy, never as a shared key.
- **Roles** (per node, inherited downward): `viewer`, `contributor`, `curator` (can pin/approve),
  `steward` (policy + membership for subtree), `org-admin`, `auditor` (read-only including audit
  logs, cannot touch content).

---

## 6. Policy engine

- **PDP**: OPA for attribute/condition policies + OpenFGA for relationship checks. Both fronted
  by a single internal `authorize(subject, action, resource, context)` API so the engines are
  swappable.
- **Policy packs** — versioned bundles applied per node:
  - `regulated-strict` (default): deny-first; no cross-team read without explicit grant; all
    writes classified; export blocked; retention enforced; PII redaction on ingest.
  - `standard`: team-shares-by-default within department; lighter classification.
  - `open-collaboration`: org-wide read for non-restricted content.
- **Lapses (controlled relaxation)**: a steward may apply a scoped, reasoned, time-boxed override
  ("allow team X to read team Y's `procedure` records for 30 days — reason: joint incident
  review"). Lapses require a second approver in `regulated-strict`, are fully audited, and
  auto-expire. This is the mechanism that lets one product serve both an SMB and a bank.
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
│  read: composition engine (scope gradient, budget, conflict rules)  │
│        hybrid retrieval (dense + sparse + graph)                    │
│  write: enqueue observe events; command handlers for pinned assets  │
└──────┬──────────────────────────────────────────┬──────────────────┘
┌──────▼──────────────┐                 ┌─────────▼──────────────────┐
│ STORAGE             │                 │ ASYNC PLANE (Temporal)      │
│ Postgres: records,  │                 │ ingestion → extraction →    │
│  hierarchy, audit,  │                 │ dedup/conflict → summarise  │
│  versions (bitemporal)│               │ → embed → graph-link        │
│ pgvector (default)  │                 │ consolidation, decay/TTL,   │
│ Qdrant (scale-out)  │                 │ re-embedding, directory sync│
│ Apache AGE (graph)  │                 └─────────────────────────────┘
└─────────────────────┘
Cross-cutting: OPA/OpenFGA (PDP) · Keycloak-compatible OIDC · OTel traces/metrics
Deploy: single binary + Postgres for SMB │ Helm chart, regional data planes for enterprise
```

**Language decisions**: core/gateway in **Rust** (single static binary, on-prem friendly,
latency-critical read path). Claude Code adapter in **TypeScript** (hooks ecosystem). SDKs:
Rust, TS, Python. Admin console: React (later phase; API-first until then).

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
│   ├── synveda-policy       # authorize() facade over OPA/OpenFGA; policy pack loader
│   ├── synveda-store        # storage traits + Postgres/pgvector/AGE impls
│   ├── synveda-retrieval    # hybrid search, rerank, composition engine
│   ├── synveda-ingest       # extraction, redaction, dedup, summarisation (Temporal activities)
│   ├── synveda-audit        # tamper-evident audit log (hash-chained), export
│   ├── synveda-identity     # OIDC, SCIM, directory sync, hierarchy provisioning
│   ├── synveda-gateway      # axum HTTP/gRPC; the ONLY binary that speaks to the outside
│   └── synveda-cli          # admin/dev CLI (synveda init, synveda policy apply, ...)
├── adapters/
│   ├── claude-code/         # TS: SessionStart/PreCompact/Stop hooks + MCP recall tool
│   └── mcp-server/          # generic MCP server exposing recall (+ scoped write)
├── sdks/ (rust, typescript, python)
├── policies/                # policy packs as versioned OPA bundles + FGA models
├── deploy/ (docker-compose single-node, helm multi-region)
└── docs/ (ADRs — every architectural decision gets an ADR from day one)
```

Dependency rule: `types ← {policy, store, identity, audit} ← retrieval/ingest ← gateway`.
Nothing imports "upward". Adapters and SDKs depend only on the public API, never on crates.

---

## 9. Build order (vertical slices, each independently demoable)

**Phase 0 — Skeleton (week 1)**
Repo scaffold, CI, `synveda-types`, ADR-0001 (this document distilled), docker-compose with
Postgres + OPA + Temporal. `synveda init` boots a dev org.

**Phase 1 — The spine (weeks 2–4)**
One vertical slice, end to end: OIDC login → auto-provisioned hierarchy from mock IdP groups →
`observe` ingests a transcript → extraction to memory records → `inject` returns a budgeted,
policy-checked context block → hash-chained audit entries for every step. Claude Code adapter
wired to a live session. *This slice is the demo and the proof of the thesis.*

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

---

## 10. Non-functional requirements

- `inject` p99 < 150ms at 1K concurrent sessions (excluding first-call cold cache)
- `observe` ack < 20ms (enqueue only); pipeline lag SLO < 60s
- Audit log: append-only, hash-chained, exportable to WORM storage
- All data encrypted at rest (per-tenant keys, KMS-pluggable) and in transit (mTLS internal)
- Zero-downtime migrations; backwards-compatible API within a major version
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

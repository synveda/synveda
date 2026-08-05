# Synveda

**Shared knowledge for agent fleets — governed like a bank, effortless like a consumer app.**

Synveda is a memory and context platform for AI agents, built for organisations
rather than for one laptop. Agents log in with your company SSO, receive the
context their team has earned, contribute what they learn back, and every byte of
it is policy-checked and audited on the way in and on the way out.

---

## The problem

An AI agent finishes a session and forgets everything. Plenty of tools fix that
for a single developer. None of them answer the questions that appear the moment
fifty agents are doing it across a company:

- **Whose memory is this?** A contractor's agent should not inherit the payments
  team's incident history.
- **Who approved that?** An agent is now repeating a "team convention" to
  everyone. Who reviewed it, and when?
- **What did it know in March?** A regulator asks what the agent was told on a
  given date. You need an answer that survives cross-examination.

Synveda is the layer that answers those three questions. It is **not** an agent
framework, an orchestrator, or a vector-DB wrapper — it is the control plane that
any agent harness plugs into.

---

## How it works

### Three primitives

Every integration — Claude Code, an MCP client, a custom SDK — reduces to these:

| | What it does | When |
|---|---|---|
| **`observe`** | "Here's what happened." Transcript deltas, tool results, decisions. | Continuously. Queued and processed async — never blocks the session. |
| **`inject`** | "Give me a token-budgeted context block for this person, this session, this task." | At session start / pre-compact. Silent and fast. |
| **`recall`** | An explicit, deeper search: hybrid retrieval plus graph traversal, plus "as of last March". | When the agent asks. |

### Knowledge is treated like code

This is the idea the whole product turns on. Every memory, prompt, context pack
and skill flows through **propose → review → approve → publish** — a system
called **VedaFlow**, implemented with git-like semantics natively in Postgres.

Each scope (org, department, team, user) has three standing channels:

- **`derived`** — what the extraction pipeline wrote automatically. Readable per
  policy, clearly watermarked as unreviewed.
- **`staged`** — proposals under review.
- **`published`** — the trusted channel.

Restricting `inject` to `published` only is a single policy switch — that switch
is "bank mode". A bad prompt shipped? Move the ref back one commit; every
consuming agent heals on its next session start.

### Governance is enforced, not suggested

- **Every read and write passes a Policy Decision Point.** Cedar, compiled into
  the gateway binary — no network hop, no sidecar. There is no code path from a
  harness to storage that goes around it, not even in tests.
- **Strict by default, relaxable by design.** The default pack assumes a
  regulated environment. A steward can grant a scoped, reasoned, time-boxed
  *lapse* ("let team X read team Y's procedures for 30 days — joint incident
  review"), with dual approval and automatic expiry. That mechanism is why one
  product can serve both a 10-person shop and a multi-region bank.
- **Audit is an output, not a log file.** Every decision, injection, recall,
  write and policy change lands in a hash-chained log that detects tampering
  even by an attacker holding database credentials.

---

## Project status

**Phases 0–2 are complete. Phase 3 (enterprise surface) is in progress.**
54 of 86 planned features are done, each one demonstrated by a runnable script in
[`demos/`](demos/) and covered by an acceptance test.

It installs: `synveda init` takes a laptop to working governed memory —
see [docs/INSTALL.md](docs/INSTALL.md).

| Phase | Scope | State |
|---|---|---|
| **0 — Foundation** | Workspace, dev environment, types, bitemporal schema, observability | ✅ 6/6 |
| **1 — The spine** | SSO → auto-provisioned hierarchy → observe → extraction → inject → audit, live in Claude Code | ✅ 21/21 |
| **2 — Governance** | VedaFlow, lapses, dedup, decay, recall, graph, audit queries, prompts, context packs, eval gates | ✅ 22/22 |
| **3 — Enterprise** | SCIM, real IdPs, skills registry, console, Helm, residency, Qdrant | 🚧 1/25 |
| **4 — Ecosystem** | SDKs, importers, shims, telemetry, DR | ⬜ 0/11 |

One further feature (AUTH-6, session and token hygiene) is unscheduled — 86 in
total. The one Phase 3 item finished so far is SKIL-1, the skills registry.

Full detail, feature by feature: [`docs/backlog/STATUS.md`](docs/backlog/STATUS.md).

### What works today

- **Zero-config onboarding** — OIDC login (auth code + PKCE); first login places
  the user in the org hierarchy from their IdP groups. No YAML before value.
- **Hierarchy and policy** — org → department → team → user, with policy packs
  (`regulated-strict`, `standard`, `open-collaboration`), eight roles
  (`viewer`, `contributor`, `curator`, `steward`, `org-admin`, `auditor`,
  `security-reviewer`, `compliance`), ABAC conditions, time-boxed lapses, and
  Postgres row-level security as a backstop.
- **The write path** — `observe` → secret scan and redaction → extraction into
  classified records → embedding → graph-linking → commit to `derived`, with
  dedup and conflict detection, decay and TTL.
- **The read path** — `inject` composes along the specificity gradient
  (user beats team beats department beats org, pinned beats derived, newer beats
  older) under a token budget, watermarked with the record IDs it used.
  `recall` goes deeper: hybrid dense + sparse retrieval, graph traversal, as-of
  queries.
- **VedaFlow end to end** — objects, commits, refs, proposals, an approval matrix,
  auto-promotion rules, cross-scope promotion, rollback and pinning, and a CLI
  review flow that needs no console.
- **Audit** — a tamper-evident chain, plus a query surface that answers
  *"who could see X on date D"* and *"what did agent A know at time T"*.
- **Governed assets** — prompt templates, context packs, and an
  agentskills.io-compliant skills registry where publishing executable code
  requires a steward *and* a security reviewer, two distinct people.
- **A live Claude Code integration** — hooks plus an MCP recall tool.
- **A quality gate in CI** — extraction, retrieval, injection and security evals
  with committed baselines; the security gate is zero-tolerance on leaks.

### Measured, on a laptop

These are real numbers from the acceptance suites, not targets. Dev hardware —
treat them as shape, not as an SLO.

| What | Result |
|---|---|
| Policy decision (4-level chain, release build) | median **33µs**, p99 46µs |
| Scope-chain resolve, warm | median **800ns** |
| `inject` at 1,000 concurrent sessions | p50 **18.6ms**, p99 24ms (budget: 150ms) |
| Graph traversal | 1-hop 1.17ms, 2-hop 23.4ms (gate: 50ms) |
| Extraction over a 50-fixture labelled corpus | macro precision **0.983**, recall 0.914 — the deterministic extractor; a live model reads 0.820/0.783 against the same corpus, which is mostly the corpus's exact-match predicate penalising paraphrase |
| Clean machine → personalised Claude Code session | **1.5s** (budget: 120s) |

### What is *not* built yet

Being explicit, so nothing here misleads:

- **No production deployment.** Docker Compose dev environment only — the
  single-binary SMB profile and the Helm chart are Phase 3 (OPS-1, OPS-2).
- **No admin console.** API and CLI only (CNSL-1..4 are Phase 3/4).
- **No SCIM, no live Entra/Okta.** OIDC works against bundled Rauthy and a mock
  Entra; the real integrations are AUTH-4/AUTH-5.
- **No standalone MCP server** (ADPT-2). The recall MCP tool currently ships
  inside the Claude Code adapter.
- **No Python/TS SDKs** (ADPT-4) and **no importers** from claude-mem, Cognee or
  mem0 (ADPT-5).
- **No per-tenant encryption keys, WORM export or SIEM streaming**
  (TEN-4, AUD-3, AUD-4).
- **No published benchmark scores** — LoCoMo/LongMemEval adapters are EVAL-3.
- **No LICENSE file yet.** Dependencies are constrained to MIT / Apache-2.0 /
  PostgreSQL in the core path (enforced by `cargo-deny`), but the project's own
  licence is not yet declared.

---

## Try it

You need Docker and GNU Make. On Windows, run `make` from Git Bash.

```sh
make dev-up   # Postgres 17 (pgvector + PGMQ), Rauthy, Temporal, TEI (BGE-M3), Jaeger
make smoke    # end-to-end health check of every service
make dev-down # stop; state persists in named volumes
```

The first `dev-up` builds the Postgres image and downloads the BGE-M3 embedding
model (~2.3 GB), so allow a few minutes.

Then run any of the 49 demos in [`demos/`](demos/). Each one is self-contained —
it brings up what it needs, seeds a scratch database, and prints what it proves.
Good places to start:

```sh
sh demos/ctx-3-inject.sh          # the read path: compose a watermarked block,
                                  # then kill the embedder mid-demo and watch it
                                  # degrade gracefully instead of failing
sh demos/flow-3-proposals.sh      # VedaFlow: propose, review, approve, publish
sh demos/aud-2-audit-query.sh     # "who could see X on date D", answered from
                                  # the chain with the database URL unset
sh demos/adpt-1-claude-code.sh    # a clean machine to a personalised session
```

Other useful targets:

```sh
make ci          # exactly what CI runs: fmt, clippy -D warnings, test, build,
                 # cargo-deny, dependency-rule check, eval parse, TS build+test
make eval        # the eval harness against a live stack, gated by baselines
```

---

## Repo map

```
crates/
  synveda-types       domain types, IDs, errors — depends on no other crate
  synveda-policy      the Cedar PDP facade, policy packs, roles, lapses
  synveda-store       Postgres: records, hierarchy, audit, bitemporal versions
  synveda-vedaflow    objects, trees, commits, refs, proposals
  synveda-retrieval   hybrid search, fusion, the composition engine
  synveda-ingest      redaction, extraction, dedup, embedding, graph-linking
  synveda-identity    OIDC, JIT provisioning, hierarchy sync
  synveda-audit       the hash-chained log
  synveda-gateway     axum HTTP — the only binary that faces the outside world
  synveda-cli         synveda login / proposal review / channel rollback / mcp / ...
  synveda-eval        the eval harness and its gates
adapters/
  claude-code/        hooks (TypeScript); its MCP entry launches `synveda mcp`
sdks/                 rust, typescript, python — stubs, Phase 4
policies/             Cedar policy packs
deploy/compose/       the dev environment
demos/                55 runnable acceptance demos, one per feature
evals/                corpora, scenarios, and the committed baselines CI gates on
docs/                 the seed, the tech plan, the backlog, and 58 ADRs
```

**Dependency rule:** `types ← {policy, store, identity, audit} ← retrieval/ingest
← gateway`. Nothing imports upward; adapters and SDKs depend only on the public
API. `make check-deps` enforces it.

---

## The stack, and why

Postgres-first, Rust-native, permissively licensed. One database engine for
records, hierarchy, audit, versions, queues, vectors and graph — one backup
story, one HA story, one thing to explain to a bank's infrastructure review
board.

| Concern | Choice |
|---|---|
| System of record | PostgreSQL 17 |
| Vectors | pgvector (HNSW); Qdrant behind the same trait when it outgrows that |
| Lexical search | Postgres FTS + a Tantivy sidecar, fused with RRF |
| Graph | Indexed adjacency in plain Postgres — measured 3–8× faster than Apache AGE at 2.5× less storage ([ADR-0043](docs/adr/adr-0043-graph-schema.md)) |
| Queue / workflow | PGMQ for ingestion; Temporal for the heavy pipelines |
| Authorisation | Cedar, embedded in the gateway ([ADR-0002](docs/adr/adr-0002-cedar-embedded-pdp.md)) |
| Identity | OIDC client, never the source of truth; Rauthy bundled for dev/SMB |
| Gateway | axum + tower; sqlx with compile-time-checked SQL, never string-built |
| Embeddings | text-embeddings-inference serving BGE-M3 |
| Observability | OpenTelemetry traces, Prometheus metrics, Jaeger in dev |

---

## Documentation

Read in this order:

1. **[docs/SYNVEDA_SEED.md](docs/SYNVEDA_SEED.md)** — what the product is and the
   invariants that must never be violated. §2 is law.
2. **[docs/SYNVEDA_TECH_PLAN.md](docs/SYNVEDA_TECH_PLAN.md)** — stack decisions
   and the VedaFlow design.
3. **[docs/SYNVEDA_FEATURES.md](docs/SYNVEDA_FEATURES.md)** — the feature backlog.
   Every piece of work maps to a feature ID.
4. **[docs/backlog/STATUS.md](docs/backlog/STATUS.md)** — where everything stands,
   including what each finished feature actually proved and what it left standing.
5. **[docs/adr/](docs/adr/)** — 51 architecture decision records. Every
   architectural choice is written down *before* it is implemented.

---

## Working on it

- Every task references a feature ID. Branch `feat/<ID>`, commit
  `"FND-1: scaffold rust workspace"`.
- A feature is done only when its acceptance criteria in `SYNVEDA_FEATURES.md`
  pass, demonstrated by a test or a script in `demos/`.
- Architectural choices get an ADR before implementation.
- `cargo fmt` and `clippy -D warnings` clean before any commit; `make ci` locally
  equals CI green.
- Never create a code path that bypasses the PDP — not even in a test. Use a test
  policy pack instead.

Full rules in [CLAUDE.md](CLAUDE.md).

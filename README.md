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

Everything an agent does belongs to a **run** — a governed record naming the
workspace it happened in — and the two primitives hang off it:

| | What it does | When |
|---|---|---|
| **append an event** | "Here's what happened." Messages, tool results, file changes, commands. | Continuously. Spooled locally, delivered async — never blocks the session. |
| **a context run** | "Give me a token-budgeted context block for this person, this run, this question." | At session start / pre-compact, or whenever the agent asks. | 

`POST /v1/sessions/{id}/events` and `POST /v1/sessions/{id}/context-runs`. The
run is in the path, so a memory lands where the *work* happened rather than
wherever the writer's own account sits.

### Knowledge is treated like code

This is the idea the whole product turns on. Every memory, prompt, context pack
and skill flows through **propose → review → approve → publish** — a system
called **VedaFlow**, implemented with git-like semantics natively in Postgres.

Each scope has three standing channels:

- **`derived`** — what the extraction pipeline wrote automatically. Readable per
  policy, clearly watermarked as unreviewed.
- **`staged`** — proposals under review.
- **`published`** — the trusted channel.

Restricting a context run to `published` only is a single policy switch — that switch
is "bank mode". A bad prompt shipped? Move the ref back one commit; every
consuming agent heals on its next session start.

### Governance is enforced, not suggested

- **Every read and write passes a Policy Decision Point.** Cedar, compiled into
  the gateway binary — no network hop, no sidecar. There is no code path from a
  harness to storage that goes around it, not even in tests.
- **Strict by default, relaxable by design.** The default pack assumes a
  regulated environment. An administrator can grant a scoped, reasoned,
  time-boxed *lapse* ("let team X read team Y's procedures for 30 days —
  joint incident review"), with dual approval and automatic expiry. That mechanism is why one
  product can serve both a 10-person shop and a multi-region bank.
- **Audit is an output, not a log file.** Every decision, composition,
  write and policy change lands in a hash-chained log that detects tampering
  even by an attacker holding database credentials.

---

## Project status

**Phases 0–2 are complete. Phase 3 (enterprise surface) is in progress.**
63 of 94 planned features are done, each one demonstrated by a runnable script in
[`demos/`](demos/) and covered by an acceptance test.

It installs, on somebody else's machine, with Docker as the only prerequisite:

```sh
curl -fsSL https://raw.githubusercontent.com/synveda/synveda/main/scripts/install.sh | sh
synveda init --demo
synveda login
synveda plugin install            # Claude Code: hooks + MCP, one command
```

An admin console comes with it, at `http://127.0.0.1:8120/console/` — since
CPR-8 a product shell with first-run onboarding, workspace and project
switchers, a People page and the governance surfaces under **Advanced**. See
[docs/INSTALL.md](docs/INSTALL.md).

| Phase | Scope | State |
|---|---|---|
| **0 — Foundation** | Workspace, dev environment, types, bitemporal schema, observability | ✅ 6/6 |
| **1 — The spine** | SSO → provisioned own-scope → append → extraction → compose → audit, live in Claude Code | ✅ 21/21 |
| **2 — Governance** | VedaFlow, lapses, dedup, decay, recall, graph, audit queries, prompts, context packs, eval gates | ✅ 22/22 |
| **3 — Enterprise** | SCIM, real IdPs, skills registry, console, Helm, release & distribution, residency, Qdrant | 🚧 14/27 |
| **4 — Ecosystem** | SDKs, importers, shims, telemetry, DR, gateway scale | ⬜ 0/17 |
| **5 — Context platform** | The redesign: fresh epoch, governed scopes, workspaces, membership, the PDP, the hierarchy cutover, the console shell, the session ledger and its product surface | 🚧 11/33 |

One further feature (AUTH-6, session and token hygiene) is unscheduled — **108
in total, 75 delivered** (`docs/backlog/STATUS.md` is the count `make ci`
checks). Phase 5 is the 33-prompt context-platform redesign, in flight on
`feat/context-platform-mvp`; Phase 3 is paused mid-phase behind it. The fourteen Phase 3 items finished are the skills registry
and its governance (SKIL-1 through SKIL-4), the installable single binary
(OPS-1), the admin console's proposals inbox and scope explorer (CNSL-1,
CNSL-2), the generic MCP server (ADPT-2), the SCIM server with its
directory-sync fallback (AUTH-4, AUTH-5), the LongMemEval benchmark adapter
(EVAL-3), the Helm chart and enterprise profile (OPS-2) — which is where the
gateway stopped connecting to Postgres as a superuser, so the tenant isolation
backstop is enforced against a deployment rather than bypassed by one — the
dense-leg retrieval benchmark that declined its own proposal (TEN-3), and
per-tenant envelope keys (TEN-4).

Full detail, feature by feature: [`docs/backlog/STATUS.md`](docs/backlog/STATUS.md).
Published benchmark scores, and what they do and do not measure:
[`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).

### What works today

- **Zero-config onboarding** — OIDC login (auth code + PKCE); first login
  mints the user's own principal scope. No YAML before value.
- **Scopes and policy** — one governed tree (tenant, org unit, workspace,
  project, principal — shapes, not ranks), administered through
  `/v1/admin/scopes` and `synveda scope`, with policy packs
  (`regulated-strict`, `standard`, `open-collaboration`), six role keys
  granted at a scope and inherited by its subtree (`owner`, `member`,
  `viewer`, `reviewer`, `curator`, `administrator`), ABAC conditions,
  time-boxed lapses, and Postgres row-level security as a backstop.
- **The write path** — appending a session event → secret scan and redaction → extraction into
  classified records → embedding → graph-linking → commit to `derived`, with
  dedup and conflict detection, decay and TTL.
- **The read path** — a context run composes along the specificity gradient
  (the nearer scope beats the wider one, pinned beats derived, newer beats
  older) under a token budget, watermarked with the record IDs it used.
  `synveda recall` and the `recall` MCP tool ask the same question from a
  terminal or an agent. (The deeper form — hybrid retrieval *widened past the
  caller's own chain*, graph traversal and as-of queries — is out of service:
  `/v1/recall` was deleted with the observe cutover, and Prompt 18 of the
  context-platform programme re-cuts it over the governed scope model.)
- **VedaFlow end to end** — objects, commits, refs, proposals, an approval matrix,
  auto-promotion rules, cross-scope promotion, rollback and pinning, and a CLI
  review flow that needs no console.
- **Audit** — a tamper-evident chain, plus a query surface that answers
  *"who could see X on date D"* and *"what did agent A know at time T"*.
- **Governed assets** — prompt templates, context packs, and an
  agentskills.io-compliant skills registry where publishing executable code
  requires a `reviewer` and two distinct approvers.
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
| a context run at 1,000 concurrent sessions | p50 **18.6ms**, p99 24ms (budget: 150ms) |
| Graph traversal | 1-hop 1.17ms, 2-hop 23.4ms (gate: 50ms) |
| Extraction over a 50-fixture labelled corpus | macro precision **0.983**, recall 0.914 — the deterministic extractor; a live model reads 0.820/0.783 against the same corpus, which is mostly the corpus's exact-match predicate penalising paraphrase |
| Clean machine → personalised Claude Code session | **1.5s** (budget: 120s) |

### What is *not* built yet

Being explicit, so nothing here misleads:

- **The Kubernetes deployment is one gateway replica.** The Helm chart installs
  into a kind cluster and asserts a governed round trip, a CloudNativePG
  failover and a live RLS backstop (OPS-2) — but it refuses to render a second
  gateway replica, by a rendering error rather than a warning, until OPS-7
  moves login state out of process memory. Its upgrade is therefore
  restart-shaped.
- **No signed binaries, and no Windows.** The release ships macOS arm64 and
  Linux x86_64, unsigned and un-notarized (OPS-8); the checksums prove a
  download arrived intact, not who built it. There is no upgrade path and no
  package manager — reinstalling is how you upgrade.
- **No live Entra or Okta tenant has been replayed.** The SCIM 2.0 server is
  built (AUTH-4) and `synveda directory` syncs from a terminal (AUTH-5), but the
  vendor corpus both are tested against is transcribed from Microsoft's and
  Okta's published tables — nothing has yet handled a frame from a real tenant.
- **No real Cursor frame either.** The generic MCP server ships as `synveda mcp`
  (ADPT-2), and its acceptance corpus was recorded from Claude Desktop and Zed.
  Cursor remains an install target rather than a measured one.
- **No live Claude Code session has composed or appended.** The plugin now
  installs into Claude Code and is proven to *load* there — `✔ enabled`, four
  hooks, one MCP server (OPS-8). What runs the hooks in ADPT-1's acceptance
  demo is still that demo's own driver, replaying recorded payloads. Until
  OPS-8 the plugin had never loaded at all: its manifest declared two keys
  Claude Code discovers on its own, and nothing noticed because a harness that
  replaces the harness cannot.
- **Three console pages have no plane behind them.** CPR-8 made the console a
  product shell — routes, switchers, People, first-run onboarding, and the
  proposals inbox (CNSL-1) and scope explorer (CNSL-2) re-homed under
  **Advanced** — and CPR-10 gave **Sessions** its plane, so Knowledge, New
  Learnings and Tools are what remain, waiting on Prompts 11–15 of the
  context-platform programme. Each page says so rather than rendering an empty
  list. Seven console surfaces also still call hand-written paths, because the
  OpenAPI contract covers the context-platform plane only until Prompt 19.
- **No Python/TS SDKs** (ADPT-4) and **no importers** from claude-mem, Cognee or
  mem0 (ADPT-5).
- **No per-tenant encryption keys, WORM export or SIEM streaming**
  (TEN-4, AUD-3, AUD-4).
- **One published benchmark score, over 10 of LongMemEval's 500 instances**
  ([`docs/BENCHMARKS.md`](docs/BENCHMARKS.md)) — a first data point rather than a
  benchmark claim, and the row says which slice it covers.
  `make eval-longmemeval-full` is the run somebody schedules. LoCoMo is EVAL-7
  and is blocked on a licence, not on effort: its corpus is CC BY-NC 4.0, which
  withholds the published commercial claim a score would be.
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

The 65 demos in [`demos/`](demos/) are each self-contained — one brings up what
it needs, seeds a scratch database, and prints what it proves. Good places to
start:

```sh
sh demos/cpr-10-sessions.sh       # a run opened, appended to, composed for and
                                  # closed through the public API, with a
                                  # timeline over it and a chain that verifies
sh demos/cpr-5-access.sh          # groups, grants and invitations
sh demos/cpr-6-anchors.sh         # where a request stands, and what decides it
```

> **Most of the other demos do not currently run.** 43 of the 65 scripts under
> `demos/` call `synveda role bind` or `synveda hierarchy`, which the scope
> cutover deleted three prompts ago — see `docs/backlog/CPR-13.md`. The three
> above are among the 22 that are current.

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
  synveda-store       Postgres: records, scopes, audit, bitemporal versions
  synveda-vedaflow    objects, trees, commits, refs, proposals
  synveda-retrieval   hybrid search, fusion, the composition engine
  synveda-ingest      redaction, extraction, dedup, embedding, graph-linking
  synveda-identity    OIDC, JIT provisioning, directory sync
  synveda-audit       the hash-chained log
  synveda-gateway     axum HTTP — the only binary that faces the outside world
  synveda-cli         synveda login / proposal review / channel rollback / mcp / ...
  synveda-eval        the eval harness and its gates
adapters/
  claude-code/        hooks (TypeScript); its MCP entry launches `synveda mcp`
sdks/                 rust, typescript, python — stubs, Phase 4
policies/             Cedar policy packs
deploy/compose/       the dev environment
console/              the admin console (React); served from the gateway's origin
demos/                66 runnable acceptance demos, one per feature
evals/                corpora, scenarios, and the committed baselines CI gates on
docs/                 the seed, the tech plan, the backlog, and 71 ADRs
docs/api/openapi.json the API contract — generated from the gateway's handlers
```

**Dependency rule:** `types ← {policy, store, identity, audit} ← retrieval/ingest
← gateway`. Nothing imports upward; adapters and SDKs depend only on the public
API. `make check-deps` enforces it.

---

## The stack, and why

Postgres-first, Rust-native, permissively licensed. One database engine for
records, scopes, audit, versions, queues, vectors and graph — one backup
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
5. **[docs/adr/](docs/adr/)** — 71 architecture decision records. Every
   architectural choice is written down *before* it is implemented.
6. **[docs/api/openapi.json](docs/api/openapi.json)** — the API contract, and it
   is **generated**: `utoipa` derives it from the gateway's own request and
   response types, a test fails when the committed file and the tree disagree,
   and `console/src/generated/api.ts` is generated from it in turn
   ([ADR-0071](docs/adr/adr-0071-workspaces-projects-and-repository-identity.md)
   decision 7). It covers the context-platform plane — `/v1/me`, workspaces,
   projects and repositories, the access plane, the scope admin plane and the
   session ledger — and says so in its own description; the rest of `/v1`
   joins it later in the programme.
7. **[docs/implementation/synveda-context-platform.md](docs/implementation/synveda-context-platform.md)**
   — the Phase 5 context-platform redesign, on `feat/context-platform-mvp`:
   the base-commit inventory, the deletion map from old concepts to new, the
   ordered 33-prompt programme and its running record. A **pre-1.0 hard cut**
   — fresh schema epoch, no old-data migration, old databases rejected with a
   reset instruction — with the decisions locked in
   [ADR-0068](docs/adr/adr-0068-context-platform-domain-and-epoch.md). Nothing
   on `main` has changed yet.

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

# Synveda

Synveda is a governed memory and context control plane for AI agents. It gives
individuals and teams durable Knowledge without allowing a harness to bypass
identity, tenant isolation, policy, review or audit.

Synveda is not an agent framework, orchestrator or vector-database wrapper.
Harnesses use the public session, capture, Knowledge and context APIs; the
gateway remains the authority boundary.

> **Production status: not ready.** The context-platform behaviour has strong
> deterministic, tenancy and product evidence, but release artefact parity,
> backup/PITR and key-custody restore evidence are P0 gaps. The gateway is also
> single-replica and restart-shaped. See
> [Production readiness](docs/PRODUCTION_READINESS.md) for evidence and exit
> criteria. Passing CI is not a production-readiness claim.

## Why it exists

An agent session forgets what happened when it ends. Persisting text is easy;
answering these questions is not:

- Who may read this Knowledge, at this scope, now?
- Which immutable revision and provenance were actually supplied?
- Who proposed, reviewed and applied a change?
- What was known at a past valid time and transaction time?
- Can a tenant or principal be denied without leaking that another resource
  exists?

Synveda answers those questions with one product model:

- **Sessions** are governed run records with ordered immutable events.
- **Capture** freezes exact session evidence into reviewable candidates.
- **Knowledge** has stable identities, immutable revisions, normalised sources,
  explicit relations, conflict/freshness state and governed lifecycle changes.
- **Context runs** select exact authorised Knowledge revisions under configured
  candidate, graph, time and token bounds and retain an explainable trace.
- **VedaFlow** governs Knowledge, Configuration, relaxations, Skills, Tools and
  other authored assets through typed propose/review/apply effects.
- **Cedar and forced RLS** decide and backstop every tenant-bound read/write.
- **Audit** records a tenant-complete, content-minimised, hash-chained evidence
  trail and supports frozen-head offline verification.

## Current capabilities

The current schema epoch and public contract support:

- OIDC/PKCE login, JIT principal scopes, service identities, directory
  projection, groups, grants, invitations and scoped administrator operations;
- workspaces, projects and canonical repository identities;
- the complete session plane, durable adapter spooling, capture batches and New
  Learnings review;
- Knowledge create, edit, verify, merge, supersede, archive, restore and forget
  through typed VedaFlow commands;
- lexical search, optional TEI semantic fusion, bitemporal queries, conflict
  resolution, freshness and bounded two-hop Knowledge-relation expansion;
- immutable Configuration versions and governed scope bindings for policy,
  capture, context budgets, trace retention, freshness and provider allowlists;
- immutable Skill and MCP Tool versions, scans/discovery evidence, governed
  bindings, drift quarantine and evidence-labelled usage/tests;
- bounded OKF v0.2 validation, dry-run planning, candidate-only import and
  independently re-authorised export;
- a generated React console for Sessions, Context, Knowledge, Learnings,
  Reviews, Configuration, People, Skills, Tools and exchange workflows;
- an exact executable route catalogue, generated OpenAPI and generated console
  client used by ordinary CLI, MCP and console application operations.

The generated [OpenAPI contract](docs/api/openapi.json) is authoritative for
the application plane. Code generation and parity checks fail CI when handlers,
router inventory, OpenAPI or the console client disagree.

## Client support

`adapters/registry.json` is the support authority. The
[generated client-support matrix](docs/CLIENT_SUPPORT.md) is its checked
projection and distinguishes configuration, authentic captured frames,
deterministic replay and live verification.

Claude Code 2.1.241 is the only verified lifecycle. Other clients remain at
their evidenced registry level; a connection recipe or generic MCP
configuration is not lifecycle support.

## Known production gaps

The current top-level gaps are deliberately explicit:

- the release workflow does not publish the chart and CNPG-compatible image
  pair named by Helm as one signed artefact set;
- no production backup, WAL archive, PITR, restore drill, RPO or RTO exists;
- Helm can now reference an externally owned local key Secret, but custody,
  KEK rotation and joint database/key restore have not passed a production
  ceremony;
- one gateway replica is enforced because login/handoff and invalidation have
  process-local state; workers have no complete drain/backoff/dead-letter SLO;
- general rate limits, quotas, revocation bounds and a complete tenant
  suspend/export/import/erase lifecycle are absent;
- binaries are unsigned; releases have no SBOM/provenance or tested rollback;
- live Entra/Okta evidence, a second verified harness, production load/soak and
  operational dashboards/runbooks remain open;
- the repository has no `LICENSE` grant, while generated OpenAPI metadata says
  `Proprietary`; that label is not complete distribution/use terms and requires
  an owner/legal decision. Dependency policy does not license Synveda itself.

The implementation-ready P0/P1 register is maintained in
[docs/PRODUCTION_READINESS.md](docs/PRODUCTION_READINESS.md), not duplicated
here.

## Try it locally

For source development you need the pinned Rust toolchain, Node/pnpm, Docker and
GNU Make:

```sh
make dev-up
make smoke
make dev-down
```

The first start builds the Postgres image and may download the optional BGE-M3
embedding model. Named volumes persist until explicitly removed.

For the installed local profile and its key-custody warning, follow
[docs/INSTALL.md](docs/INSTALL.md). Release archives are currently unsigned;
verify checksums, and do not treat the installed profile as production-ready.
The accepted target for the Docker-first portable reference is
[docs/DEPLOYMENT_CONTRACT.md](docs/DEPLOYMENT_CONTRACT.md); CPR-45 remains open
until its clean-volume, identity, worker and recovery acceptance passes.

Runnable feature acceptance lives under [`demos/`](demos/). Useful current
entry points include:

```sh
sh demos/cpr-10-sessions.sh
sh demos/cpr-17-knowledge-browser.sh
sh demos/cpr-18-session-capture.sh
sh demos/cpr-30-governed-configuration.sh
sh demos/cpr-38-bounded-graph.sh
```

The demo drift gate checks named CLI commands and application routes against
the built CLI help and generated OpenAPI. Deleted pre-cut routes and nouns are
negative tests, not compatibility aliases.

## Quality gates

```sh
make ci                  # pull-request-equivalent Rust/TS/docs/deploy/eval gates
make db-test             # complete Postgres suite in a disposable database
make eval-check          # deterministic scenario/corpus/baseline validation
make eval                # live-stack product evaluation
make claude-acceptance   # deterministic authentic-frame replay
make claude-acceptance-live # installed authenticated client, when available
```

Live, model-backed and proprietary-client gates run only when their documented
services and credentials exist. An unavailable prerequisite is recorded as
unavailable, never converted to pass.

Measured results and their limits are in
[docs/BENCHMARKS.md](docs/BENCHMARKS.md). They are development/evaluation
evidence, not service-level objectives.

## Architecture

Synveda is a Rust 2024 workspace on the pinned toolchain, with React/TypeScript
for the console. PostgreSQL 17 is the system of record; pgvector is the optional
dense leg, ordinary tenant-bound PostgreSQL supports lexical and Knowledge
relation queries, and Cedar is embedded in the gateway.

The dependency direction is enforced:

```text
types ← crypto ← {policy, store, identity, audit, vedaflow}
      ← {retrieval, ingest} ← gateway
```

Adapters depend only on the public API. SQL remains in `synveda-store`, static
and sqlx compile-time checked. The schema is the single epoch-3
`0001_context_platform.sql` baseline; pre-cut databases are refused with a
destructive-reset instruction and no compatibility migrator.

Repository layout:

```text
crates/       13 Rust crates: domain, trust, persistence and application layers
adapters/     client integrations and conformance fixtures
console/      generated-contract React application
policies/     Cedar policy packs
deploy/       development, release/installed and Helm shapes
demos/        runnable acceptance evidence
evals/        scenarios, corpora and committed baselines
docs/         current contracts, feature inventory/open briefs, ADRs and OpenAPI
scripts/      generation and CI consistency checks
```

Deployment shapes implement one provider-neutral contract; they do not select
product editions. Docker Compose is the accepted single-host reference target,
and later Helm work maps the same commands, configuration, OIDC, OTLP and
backup semantics to native primitives rather than translating Compose YAML.

## Documentation

- [Product principles and invariants](docs/SYNVEDA_SEED.md)
- [Technical plan](docs/SYNVEDA_TECH_PLAN.md)
- [Feature inventory and open work](docs/backlog/STATUS.md)
- [Production readiness](docs/PRODUCTION_READINESS.md)
- [Security model and residual limits](docs/SECURITY.md)
- [Install and local operations](docs/INSTALL.md)
- [Client support](docs/CLIENT_SUPPORT.md)
- [Benchmarks and evaluation limits](docs/BENCHMARKS.md)
- [ADR index](docs/adr/README.md)
- [Schema hard-cut inventory](docs/implementation/context-hard-cut-inventory.md)
- [Generated OpenAPI](docs/api/openapi.json)

## Contributing

All work maps to a feature ID and acceptance evidence. Architectural decisions
are recorded before implementation. See [AGENTS.md](AGENTS.md) for repository
rules and [CONTRIBUTING.md](CONTRIBUTING.md) for the human workflow.

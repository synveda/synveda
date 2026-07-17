# Synveda

Enterprise memory & context management platform for AI agents — a governed,
multi-tenant, policy-enforced layer for the memory, context, skills, and prompts
that agent fleets depend on. Rust workspace + TypeScript adapters. Postgres-first.
Governed by VedaFlow.

Required reading, in order:

1. [docs/SYNVEDA_SEED.md](docs/SYNVEDA_SEED.md) — product principles & invariants (§2 is law)
2. [docs/SYNVEDA_TECH_PLAN.md](docs/SYNVEDA_TECH_PLAN.md) — stack decisions & VedaFlow design
3. [docs/SYNVEDA_FEATURES.md](docs/SYNVEDA_FEATURES.md) — feature backlog; all work maps to a feature ID

Status: Phase 0 (Foundation). See [docs/backlog/STATUS.md](docs/backlog/STATUS.md).

## Dev environment

```sh
make dev-up   # docker compose: Postgres 17(+pgvector+AGE+PGMQ), Rauthy, Temporal, TEI (BGE-M3), Jaeger
make smoke    # end-to-end health check of all six services
make dev-down # stop (state persists in named volumes)
```

Requires Docker and GNU Make; on Windows run make from Git Bash. The compose
project lives in [deploy/compose/](deploy/compose/). First `dev-up` builds the
Postgres image and downloads the BGE-M3 embedding model (~2.3 GB).

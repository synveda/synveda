# Backlog status

## Phase 0 — Foundation

| Feature | Status | Notes |
|---|---|---|
| FND-1 Workspace scaffold | **Done** (2026-07-16) | 10 crates + pnpm workspace; CI (fmt, clippy -D warnings, test, build, cargo-deny, layering check). Demo: `demos/fnd-1-scaffold.sh`. AC: `cargo build --workspace` green in CI |
| FND-2 Dev environment | Pending | docker-compose + `make dev-up` / `make smoke` |
| FND-3 synveda-types + error model | Pending | |
| FND-4 Migrations & bitemporal base tables | Pending | |
| FND-5 Observability baseline | Pending | |
| FND-6 ADRs 0001–0004 | Pending | Stack; Cedar-over-OPA; VedaFlow-in-Postgres (incl. synveda-vedaflow tier placement); multi-graph AGE schema |

Phase 1+ not started — blocked on FND completion and `make dev-up && make smoke`
passing (CLAUDE.md, current phase).

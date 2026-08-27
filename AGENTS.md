# AGENTS.md

Common instructions for coding agents in this repository. Tool-specific files
may add a small delta; they must not duplicate or override these rules.

## Product

Synveda is a Postgres-first memory and context control plane for AI agents. It
governs sessions, capture, immutable Knowledge, context composition, Skills,
Tools and configuration. It is not an agent framework, orchestrator or vector
database wrapper. Trustworthiness is the product.

The invariants in `docs/SYNVEDA_SEED.md` §2 are law:

- every read and write is decided by the embedded Cedar PDP;
- Postgres forced RLS is the tenant-isolation backstop;
- governed mutations use VedaFlow and retain content-free audit evidence;
- policy profiles may narrow behaviour but never bypass the PDP, RLS or audit;
- tests use test policy packs and ordinary tenant transactions, never bypasses.

## Read before changing code

Read these in order, then the ADRs and open brief for the feature in scope:

1. `docs/SYNVEDA_SEED.md`
2. `docs/SYNVEDA_TECH_PLAN.md`
3. `docs/backlog/STATUS.md`
4. `docs/PRODUCTION_READINESS.md`
5. `docs/adr/README.md`

For schema hard-cut work also read
`docs/implementation/context-hard-cut-inventory.md`, ADR-0068 and ADR-0069.
For client claims, `adapters/registry.json` and generated
`docs/CLIENT_SUPPORT.md` are authoritative.

Current executable code, tests, generated contracts and accepted ADRs outrank
historical prose. Git retains implementation history; current documents should
describe only current contracts, evidence and open risks.

## Work discipline

- Every task maps to a feature ID. The normal branch is `feat/<ID>` and every
  commit subject includes the ID.
- A feature is complete only when its acceptance criteria pass through a test
  or runnable script under `demos/`.
- Write an ADR from `docs/adr/adr-0000-template.md` before making an
  architectural choice. A shipped feature's ADR must not remain `Proposed`.
- Preserve the crate direction enforced by `make check-deps`:
  `types ← crypto ← {policy, store, identity, audit, vedaflow}
  ← {retrieval, ingest} ← gateway`. `synveda-okf` is a types-only format leaf
  consumed at the gateway/CLI boundary. The CLI's local bootstrap exceptions
  and the dependency-free eval crate are enumerated by the checker. Adapters
  and SDKs use the public API.
- Keep SQL in `synveda-store`. Use sqlx compile-time checked static queries;
  never construct SQL strings.
- Make items private by default. Export only a real cross-module or cross-crate
  contract; prefer `pub(crate)` to accidental public APIs.
- Prefer explicit control flow, closed state vocabularies and bounded work.
  Validate untrusted input at the boundary and preserve causal errors without
  leaking secrets or resource existence.
- Production request, worker, adapter and store paths must not use unjustified
  `unwrap`, `expect`, `panic!`, `todo!` or `unimplemented!`.
- Do not add compatibility paths for pre-1.0 Record, hierarchy or schema-era
  models. Epoch-1, epoch-2 and markerless databases are refused with reset
  guidance; there is no old-data translator.
- Licences allowed in the core dependency path are MIT, Apache-2.0 and
  PostgreSQL. `cargo-deny` and the npm/corpus/image inventories enforce the
  repository policy; they do not choose a licence for Synveda itself.
- Prefer small, reviewable changes. A large module is a review signal, not an
  instruction to split by line count. Extract a cohesive responsibility with a
  narrow seam and behaviour tests.
- Comments explain security, protocol, ordering, resource or operational
  rationale. Delete history narration and comments that merely restate code.

## Generated and checked artefacts

Never hand-edit:

- `docs/api/openapi.json`
- `console/src/generated/api.ts`
- `.sqlx/query-*.json`

Refresh OpenAPI and the console client with:

```sh
SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi
node scripts/generate-api-types.mjs
```

Regenerate SQLx metadata from a current database using the repository's
documented prepare command; review the old/new query hashes. Do not accept
unrelated cache churn.

## Definition of done

Every delivered feature includes:

1. acceptance evidence and focused tests;
2. tracing spans and bounded-cardinality metrics for new paths;
3. audit events for new action types;
4. generated contract/SQLx artefacts when their sources changed;
5. an updated open brief and `docs/backlog/STATUS.md`; on delivery, retain the
   current contract in tests/ADRs/docs and remove the planning brief;
6. accepted or superseded ADR status where applicable;
7. clean formatting, strict Clippy and relevant repository gates.

Do not weaken a gate or rewrite a benchmark baseline to make a change green.
Distinguish product failure, stale test, missing service, unavailable credential
and unsupported platform.

## Current architecture boundary

- The repository serves schema epoch 3 from the single
  `0001_context_platform.sql` baseline. Record storage, Tantivy, PGMQ, AGE and
  the development migration chain are gone.
- `synveda_types::scope::ScopeKind` and `synveda_types::access::RoleKey` are the
  only scope/role vocabularies. Placement is identity; authority is an
  inheritable grant decided through Cedar.
- Sessions are the only adapter runtime plane. Capture freezes exact session
  evidence into candidates; only typed Knowledge/VedaFlow commands publish.
- The executable route catalogue, OpenAPI and generated console client must be
  exact peers. Ordinary CLI, MCP and console operations are public-API clients.
- Configuration, relaxations, Knowledge, Skills and Tools use stable aggregates
  with immutable versions and governed bindings/effects.
- Current production-readiness verdict is recorded in
  `docs/PRODUCTION_READINESS.md`. Passing CI is not a production-readiness
  claim. Only evidence meeting `adapters/registry.json` may support a client.

## Commands

```sh
make dev-up              # Postgres, Rauthy, Temporal, TEI, Jaeger
make smoke               # end-to-end health check
make dev-down            # stop; named-volume state persists
make ci                  # the pull-request gate
make db-test             # full database suite against DATABASE_URL
make eval-check          # deterministic eval/corpus/baseline checks
make eval                # live-stack evaluation
make claude-acceptance   # deterministic authentic-frame replay
make claude-acceptance-live # installed authenticated client, when available
```

Before a commit, at minimum run `cargo fmt --all --check`, relevant focused
tests and strict Clippy for changed Rust crates. Run the complete gates in
proportion to the change and record prerequisites that were unavailable.

## Repository map

- `crates/` — 13 Rust crates: types, crypto, policy, store, identity, audit,
  vedaflow, retrieval, ingest, OKF, gateway, CLI and evaluation
- `adapters/` — client adapters; Claude hooks launch the public `synveda mcp`
- `console/` — React console using only the generated application contract
- `policies/` — Cedar policy packs
- `deploy/` — development, installed/release and Helm deployment shapes
- `demos/` — runnable acceptance evidence
- `evals/` — corpora, scenarios and committed baselines
- `docs/` — current contracts, feature state/evidence, ADRs and generated API
- `scripts/` — generation and CI consistency checks

## Durable project memory

Repository artefacts, not harness checkpoints, carry state:

- `docs/backlog/STATUS.md` owns feature identity and delivered/open state. Only
  open work has a linked implementation brief; git retains delivered history.
- `docs/adr/` owns architectural rationale; `docs/adr/README.md` indexes what is
  current, superseded or removed.
- `docs/PRODUCTION_READINESS.md` owns production gaps and their exit criteria.
- Generated OpenAPI, client support and benchmark reports own their measured
  contracts. Do not copy their volatile counts into agent instructions.

Before handing off unfinished work, update the open brief and STATUS with
the exact blocker and next action. A local resume file may help a running tool,
but it is never the project's durable source of truth.

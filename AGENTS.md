# AGENTS.md

Repository instructions for coding agents. Tool-specific files may add a small
delta; they must not duplicate or weaken these rules. The human workflow and
exact source commands live in [CONTRIBUTING.md](CONTRIBUTING.md) and
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md); AI assistance is optional.

## Product and authority

Synveda is a Postgres-first memory and context control plane for AI agents,
not an agent framework, orchestrator or vector database wrapper. The mandatory
invariants in `docs/SYNVEDA_SEED.md` section 2 apply to all changes:

- embedded Cedar decides every read and write;
- PostgreSQL forced RLS backstops tenant isolation;
- governed mutations use VedaFlow and retain content-free audit evidence;
- configuration may narrow behavior, never bypass PDP, RLS or audit;
- tests use ordinary tenant transactions and test policy packs, never bypasses.

Before changing code, read in order: `docs/SYNVEDA_SEED.md`,
`docs/SYNVEDA_TECH_PLAN.md`, `docs/backlog/STATUS.md`,
`docs/PRODUCTION_READINESS.md`, `docs/adr/README.md`, then the relevant open
brief and current ADRs. Code, tests, generated contracts and current accepted
ADRs outrank historical prose. `adapters/registry.json` and generated
`docs/CLIENT_SUPPORT.md` govern client-support claims; passing CI does not
establish HA, SaaS, disaster recovery or enterprise certification.

## Current boundaries

- Schema epoch 3 is the single `0001_context_platform.sql` baseline. Earlier
  schemas are refused with reset guidance; no compatibility migrator exists.
- `synveda_types::scope::ScopeKind` and `synveda_types::access::RoleKey` are the
  only scope/role vocabularies. Placement is identity; Cedar-governed grants
  are authority.
- Sessions are the adapter runtime plane. Capture freezes Session evidence;
  only typed Knowledge/VedaFlow commands publish.
- Route catalogue, OpenAPI and generated console client are exact peers.
  CLI, MCP and console use the public API except documented local bootstrap.
- Compose is the portable single-host reference: generic OIDC/PKCE with
  Keycloak, separate gateway/worker, file-backed secrets and OTLP. Helm maps
  the same contract with Kubernetes-native primitives.
- Apalis is an optional deployment leaf; product operations/outbox rows remain
  authoritative. No core/public crate imports Apalis or its provider types.

## Engineering discipline

- Map every change to a feature ID and include it in commit subjects. Small
  fixes use an existing ID; no new brief or ADR is needed unless scope or an
  architectural decision changes. Only open features retain backlog briefs.
- Record architectural choices with `docs/adr/adr-0000-template.md` before
  implementation. Shipped decisions must not remain Proposed.
- Preserve `make check-deps` direction:
  `types <- crypto <- {policy, store, identity, audit, vedaflow}
  <- {retrieval, ingest} <- gateway`. OKF is types-only; adapters/SDKs use
  the public API. Apalis's non-authoritative transport database is the sole
  accepted SQL placement exception and must not query product state.
- Keep authoritative product SQL static, SQLx checked and in `synveda-store`.
  Prefer private items (`pub(crate)` for real internal seams), explicit
  control flow, closed state vocabularies and bounded work. Validate untrusted
  input at the boundary; preserve causal errors without secrets or existence leaks.
- No unjustified production `unwrap`, `expect`, `panic!`, `todo!` or
  `unimplemented!`. No pre-1.0 schema, Record or hierarchy compatibility paths.
- Core dependency licences are MIT, Apache-2.0 or PostgreSQL; repository gates
  and accepted exceptions remain authoritative. Preserve notices/attribution.
- Comments explain security, protocol, ordering, resource or operational
  rationale. Git carries implementation history, not current prose.

## Validation and completion

Never hand-edit `docs/api/openapi.json`, `console/src/generated/api.ts` or
`.sqlx/query-*.json`. Follow [generated-contract commands](docs/DEVELOPMENT.md#generated-contracts),
using a fresh current database for SQLx metadata and reviewing every query hash.

Before a commit, run formatting, strict Clippy for changed Rust crates and
focused tests. Use [the validation guide](docs/DEVELOPMENT.md#validation) for
proportional deeper gates. Do not weaken tests or baselines. Distinguish missing
services/credentials/platform support from product failures and passing checks.
Follow `deploy/compose/README.md` for lifecycle prerequisites and its exact
confirmation for destructive reset; never reset unrelated retained data.

A delivered feature has runnable acceptance, focused tests, required telemetry
and audit, current generated artifacts, accepted/superseded decisions and an
updated feature inventory. Preserve its contract in code/tests/operator docs;
remove only its completed planning brief. STATUS owns feature state, the ADR
index owns decision classification, and PRODUCTION_READINESS owns readiness.
Before handing off unfinished work, update its open brief and STATUS with the
exact blocker and next action. Local recovery files are never project truth.

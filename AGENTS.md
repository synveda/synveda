# AGENTS.md

Repository instructions for coding agents. Tool-specific files may add a small
delta; they must not duplicate or weaken these rules.

## Product and invariants

Synveda is a Postgres-first memory and context control plane for AI agents. It
governs Sessions, Capture, immutable Knowledge, Context, Skills, Tools and
configuration. It is not an agent framework, orchestrator or vector database
wrapper. Trustworthiness is the product.

The invariants in `docs/SYNVEDA_SEED.md` section 2 are mandatory:

- the embedded Cedar PDP decides every read and write;
- PostgreSQL forced RLS is the tenant-isolation backstop;
- governed mutations use VedaFlow and retain content-free audit evidence;
- configuration may narrow behaviour but never bypass PDP, RLS or audit;
- tests use ordinary tenant transactions and test policy packs, never bypasses.

## Authoritative context

Read, in order, before changing code:

1. `docs/SYNVEDA_SEED.md`
2. `docs/SYNVEDA_TECH_PLAN.md`
3. `docs/backlog/STATUS.md`
4. `docs/PRODUCTION_READINESS.md`
5. `docs/adr/README.md`
6. the open brief and relevant current ADRs for the feature in scope

Executable code and tests, generated contracts and accepted current ADRs
outrank historical prose. `adapters/registry.json` and generated
`docs/CLIENT_SUPPORT.md` govern client-support claims.

## Current boundaries

- Schema epoch 3 is the single `0001_context_platform.sql` baseline. Earlier
  schemas are refused with reset guidance; there is no compatibility migrator.
- `synveda_types::scope::ScopeKind` and `synveda_types::access::RoleKey` are the
  only scope and role vocabularies. Placement is identity; Cedar-governed grants
  are authority.
- Sessions are the adapter runtime plane. Capture freezes Session evidence;
  only typed Knowledge/VedaFlow commands publish.
- The route catalogue, OpenAPI and generated console client are exact peers.
  CLI, MCP and console code use the public API unless a documented local
  bootstrap boundary requires otherwise.
- Docker Compose is the portable single-host reference deployment. It uses
  Keycloak through generic OIDC/OAuth 2.0 + PKCE, separate gateway and worker
  processes, file-backed secrets and OTLP. Helm implements the same contract
  with Kubernetes-native primitives.
- The optional Apalis crate is a replaceable deployment leaf. Synveda
  operations/outbox rows remain authoritative; no core or public crate imports
  Apalis.
- Passing CI is not evidence of HA, SaaS readiness, disaster recovery or
  enterprise certification. `docs/PRODUCTION_READINESS.md` owns those claims.

## Engineering discipline

- Map every change to a feature ID; include it in each commit subject. Only
  open features retain a brief under `docs/backlog/`.
- Record an architectural choice from `docs/adr/adr-0000-template.md` before
  implementation. A shipped decision must not remain `Proposed`.
- Preserve the dependency direction enforced by `make check-deps`:
  `types <- crypto <- {policy, store, identity, audit, vedaflow}
  <- {retrieval, ingest} <- gateway`. The OKF crate is a types-only boundary;
  adapters and SDKs consume the public API.
- Keep authoritative product SQL in `synveda-store`, static and SQLx checked.
  The accepted Apalis leaf exception is limited to its separate,
  non-authoritative transport database; it must not query product state or
  leak provider schema or types into core or public crates.
- Prefer private items (`pub(crate)` for real crate-internal seams), explicit
  control flow, closed state vocabularies and bounded work. Validate untrusted
  input at the boundary and preserve causal errors without leaking secrets or
  resource existence.
- Production paths must not use unjustified `unwrap`, `expect`, `panic!`,
  `todo!` or `unimplemented!`.
- Do not add pre-1.0 schema, Record or hierarchy compatibility paths.
- Core dependency licences are MIT, Apache-2.0 or PostgreSQL. Repository
  licence gates remain authoritative.
- Comments explain security, protocol, ordering, resource or operational
  rationale. Git, not current prose, carries implementation history.

## Generated artefacts

Never hand-edit `docs/api/openapi.json`, `console/src/generated/api.ts` or
`.sqlx/query-*.json`.

Refresh OpenAPI and the console client with:

```sh
SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi
node scripts/generate-api-types.mjs
```

Regenerate SQLx metadata from a fresh current database with the repository's
documented prepare command, then review every changed query hash. Do not accept
unrelated cache churn.

## Definition of done

A delivered feature has runnable acceptance evidence, focused tests, required
tracing/metrics and audit events, current generated artefacts, an accepted or
superseded ADR where applicable, and an updated feature inventory. Preserve the
current contract in code, tests, ADRs and operator docs; remove only its
planning brief on delivery. Do not weaken gates or rewrite baselines to pass.
Distinguish product failure from a missing service, credential or unsupported
platform.

Before a commit, run at least formatting, strict Clippy for changed Rust crates
and focused tests. Run broader gates in proportion to risk:

```sh
make ci                       # complete pull-request gate
make db-test                  # fresh exact-role PostgreSQL suite
make check-deploy             # deployment and packaging contracts
make compose-config           # canonical Compose render matrix
make compose-acceptance       # live single-host acceptance
make claude-acceptance        # deterministic authentic-frame replay
make claude-acceptance-live   # real installed client, when available
make eval-check               # deterministic evaluation contracts
```

Follow `deploy/compose/README.md` for issuer, DNS/hosts and secret prerequisites.
Lifecycle entry points are `make compose-up`, `make compose-smoke` and
`make compose-down`; destructive reset requires the exact confirmation enforced
by `make compose-reset`.

## Durable project memory

- `docs/backlog/STATUS.md` owns feature identity and delivery state.
- `docs/adr/README.md` classifies current, superseded and removed decisions.
- `docs/PRODUCTION_READINESS.md` owns readiness gaps and exit criteria.
- Generated OpenAPI, client support and benchmark reports own measured claims.

Before handing off unfinished work, update the open brief and STATUS with the
exact blocker and next action. Local recovery files are never project truth.

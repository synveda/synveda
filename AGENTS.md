# AGENTS.md

Instructions for any coding agent working in this repository — ZCode, Codex,
Cursor, Claude Code, or a plain model in a terminal. This file is the
agent-neutral entry point. `CLAUDE.md` carries the same rules for Claude Code
plus a fuller running narrative; when the project's state moves, both files
move. Durable project memory is the git-tracked documents named under
"Memory model" — never a harness's session state.

## Project

Synveda — enterprise memory & context management platform for AI agents.
Rust workspace + TypeScript adapters and console. Postgres-first. Knowledge
is governed by VedaFlow (propose → review → approve → publish, git-like
semantics natively in Postgres). It is a control plane, not an agent
framework, orchestrator, or vector-DB wrapper: any harness plugs into the
same three primitives — `observe` (write, async), `inject` (read,
token-budgeted), `recall` (read, deep) — and every read and write passes an
embedded Cedar Policy Decision Point. This product sells trustworthiness.

## Required reading (in order, before any task)

1. `docs/SYNVEDA_SEED.md` — product principles & invariants. §2 is law.
2. `docs/SYNVEDA_TECH_PLAN.md` — stack decisions & the VedaFlow design.
3. `docs/SYNVEDA_FEATURES.md` — the feature backlog. ALL work maps to a
   feature ID.
4. `docs/backlog/STATUS.md` — what is done, what each feature found, and
   what it left standing.
5. `docs/implementation/synveda-context-platform.md` — the Phase 5
   context-platform redesign: the base-commit inventory, the deletion map,
   the ordered 33-prompt programme and its running record. Required before
   any CPR work; ADR-0068's eight decisions are locked and no prompt
   reopens them.

## Working rules

- Every task references a feature ID (e.g. FND-1). Branch: `feat/<ID>`.
  Commit messages include the ID: "FND-1: scaffold rust workspace".
- A feature is done ONLY when its acceptance criteria in
  SYNVEDA_FEATURES.md pass, demonstrated by a test or a runnable demo
  script under `demos/`.
- Never create a code path that bypasses the PDP (seed §2.2), even in
  tests — use a test policy pack instead.
- Architectural choices get an ADR in `docs/adr/` (copy
  `0000-template.md`) BEFORE implementation. Once the feature ships, the
  ADR must not still read `Proposed` — `make check-adr-status` fails the
  build on that drift.
- Crate dependency rule (seed §8; tech plan §5 adds synveda-vedaflow,
  ADR-0064 adds synveda-crypto):
  `types ← crypto ← {policy, store, identity, audit} ← retrieval/ingest ← gateway`.
  Nothing imports upward. Adapters/SDKs depend only on the public API.
  `make check-deps` enforces it.
- Licences: MIT/Apache-2.0/PostgreSQL only in the core path; `cargo-deny`
  enforces.
- Prefer boring, explicit code over cleverness.
- `cargo fmt` + `clippy -D warnings` must be clean before any commit.
- sqlx compile-time checked queries only; no string-built SQL, ever.
- Generated files are generated: never hand-edit `docs/api/openapi.json`
  or `console/src/generated/api.ts`. To refresh both:
  `SYNVEDA_WRITE_OPENAPI=1 cargo test -p synveda-gateway --test openapi`
  then `node scripts/generate-api-types.mjs`; `make check-api-types`
  verifies the pair.

## Definition of done (every feature)

1. Acceptance criteria met and demonstrated
2. Tests written (unit + the AC test)
3. Tracing spans + metrics on new paths
4. Audit events emitted for any new action type
5. `docs/backlog/STATUS.md` updated

## Current state

Phase 5 — context platform redesign, since 2026-08-17, on
`feat/context-platform-mvp`. Phase 3 is paused mid-phase, not finished —
OPS-9, OPS-10, TEN-5,6, AUD-3,4, GRPH-3, EVAL-6, CTX-7, OPS-3,4, ADPT-3,
CTX-6 and FLOW-8 are still open, and no live Entra/Okta tenant or real
Cursor frame has been replayed. **107 features filed, 74 delivered**;
STATUS.md and `make check-backlog` are the authority on the count — the
headline number has drifted four times, the fourth being this file itself,
which still read 104/71 after CPR-8 filed the 105th. That is why the
counting trail in CLAUDE.md exists. Append to it — and to this line —
when filing a feature.

Load-bearing facts about Phase 5:

- **Pre-1.0 hard cut** (ADR-0068/0069): a fresh schema epoch, no old-data
  migration, no compatibility shims. Old databases are refused with a
  reset instruction — **your dev database will be refused**; reset it
  (`synveda reset --database --force`). Since CPR-7 the epoch is **2**
  and the chain was rewritten in place (the scope substrate sits at
  `0004`; 43 → 41 migrations). The chain is not squashed yet (Prompt 33).
- **One tree, one vocabulary** since CPR-7 (ADR-0074): the old hierarchy,
  role bindings, the rank vocabulary, `/v1/hierarchy/*`, `synveda
  hierarchy`, `synveda role bind` and the placement conventions are
  deleted whole — negative tests assert the 404s and the old kinds
  failing validation by name. `synveda_types::scope::ScopeKind` is the
  only `ScopeKind`; `synveda_types::access::RoleKey` is the only role
  vocabulary.
- CPR-3 (ADR-0070): `scopes` + `scope_closure` — a named node with a
  parent and a subtree, where `kind` is a **shape** deciding only which
  shapes may be its parent (`tenant`, `org_unit`, `workspace`, `project`,
  `principal`).
- CPR-7 (ADR-0074): placement is identity — an identity's scope is its
  own principal scope (minted at first login; `externalId`-keyed for
  directory identities; under the operator's anchor for services), and
  "unmapped" means *ungranted*. The `synveda-admins` IdP group upserts an
  `administrator` grant at the tenant root — the operator door. Six
  public admin routes at `/v1/admin/scopes` (a **move** is decided at
  both ends and audited with both; no delete — retiring is a status
  transition) and five CLI commands (`synveda scope
  list|show|create|move|tree`).
- CPR-4 (ADR-0071): `workspaces`, `projects`, `project_repositories` as
  product-level subtypes of a governed scope. Creation takes a required
  `Idempotency-Key`; update takes a required `expected_revision`. A
  repository's identity is its **canonical remote URI** — a filesystem
  path is never one.
- CPR-5 (ADR-0072): `groups`, `group_members`, `scope_grants`,
  `pending_invites`. A grant gives a subject (principal or group) a role
  key at a scope, and the subtree inherits it with no row written there.
  Six role keys, **no permission table**. A principal-shaped scope
  inherits nothing. Invitations are one-time, expiring, revocable tokens
  returned **once**; no email delivery anywhere.
- CPR-6 (ADR-0073): a grant decides. `synveda_store::anchors::resolve`
  answers where a request stands as an ordered set of anchors; Cedar has
  seven entities, each subtype parented to the scope it owns. Personal
  principal-scope privacy is a base-layer forbid no pack can drop. The
  ownership check runs **before** the decision (a made-up id is 404, not
  403). Since CPR-7 there is **one gather** and `context.roles` carries
  grant keys only. A login with the `synveda-admins` IdP group mints a
  tenant's first grant; a dev-token tenant seeds it by hand once
  (INSTALL.md's SQL).
- The OpenAPI contract covers the context-platform plane (`/v1/me`,
  workspaces, projects, repositories, the access plane and the six admin
  scope routes — 32 operations); the rest of `/v1` joins it at
  Prompt 19.
- CPR-10 (ADR-0076): **a run is a record**. `sessions`, `session_events` and
  `session_context_runs` replace `session_id: text`; the governed scope a run
  is decided at is derived from its workspace and project by composite keys
  and a CHECK, and no body may name a tenant, an acting principal or a scope.
  Five states (the close is two-phase), events immutable and ordered by a
  server-assigned `sequence` and idempotent per event, a timeline projected
  over two tables and merged rather than sorted. `/v1/observe`, `/v1/inject`
  and `/v1/recall` are untouched; Prompt 11 re-cuts them.
- CPR-9 (no ADR — the foundation audit of Prompts 1–7): **a listing decides
  per row.** `GET /v1/workspaces` and `/v1/me` took one decision at the
  tenant root and applied it to every row, so a caller granted `member` at a
  workspace saw nothing and was told `needs_workspace`, while the same
  response's `anchors` block said `workspace.read: true` there. They now
  decide about the row, under the row's own chain and pack, with **no fast
  path** for a caller permitted at the root (a forbid overrides a permit at
  any depth). Two CLI surfaces CPR-7 had silently broken are repaired and
  **pinned from both sides** — `synveda login` required a deleted
  `identity.quarantined`, so every login failed to parse its own session,
  and `synveda whoami --capabilities` read the deleted `roles`/`role_assign`
  shape. `crates/synveda-gateway/tests/foundation_audit.rs` is the
  adversarial suite: valid ids from another tenant, a second workspace in
  one tenant, and somebody else's principal scope, each checked against
  counts, error kinds and the navigation capabilities. The
  no-data-migrator guard now scans the whole migration chain, not the epoch
  file alone.

## Commands

- `make dev-up` — start Postgres(+pgvector+PGMQ), Rauthy, Temporal, TEI, Jaeger
- `make smoke` — end-to-end health check
- `make dev-down` — stop; state persists in named volumes
- `make ci` — exactly what .github/workflows/ci.yml runs; green here == green there
- `make db-test` — the full suite against DATABASE_URL (tests needing Postgres skip without it)
- `make eval` — the eval harness against a live stack, gated by evals/baseline.json
- `make eval-check` — parse suite + corpora + baseline with no stack (part of `make ci`)
- `make eval-retrieval / eval-security / eval-extraction-live` — the nightly
  and live-model gates, each with its own baseline

## Repo map

```
crates/         12 Rust crates — types, crypto, policy, store, identity, audit,
                vedaflow, retrieval, ingest, gateway, cli, eval
adapters/       claude-code hooks (TypeScript); its MCP entry launches `synveda mcp`
console/        admin console (React), served from the gateway origin at /console/
policies/       Cedar policy packs
deploy/         compose dev environment; helm chart
demos/          runnable acceptance demos, one per feature — a feature is not
                done without one
evals/          corpora, scenarios, and the committed baselines CI gates on
docs/           the seed, tech plan, features, backlog/, adr/, api/, implementation/
sdks/           rust, typescript, python — Phase 4 stubs
scripts/        install, packaging, and the CI checkers (check-backlog,
                check-adr-status, check-deps, generate-api-types, ...)
```

## Memory model — how context survives across harnesses

This project treats agent memory as a repository artefact, not a session
feature. Nothing important lives in any harness's checkpoint: a cold
session that reads the documents below has everything a warm one did.

- **Per-feature state** — `docs/backlog/STATUS.md` plus one file per
  feature in `docs/backlog/`. Hand-maintained; `make check-backlog`
  fails the build if SYNVEDA_FEATURES.md, the per-feature files and
  STATUS.md disagree. Adding a feature is three edits: SYNVEDA_FEATURES.md
  (Part B *and* the Sequencing line), `docs/backlog/<ID>.md`, and the
  checklist line in STATUS.md.
- **Decisions** — `docs/adr/`, numbered, written *before* implementation.
  `make check-adr-status` asserts no shipped feature's ADR still reads
  `Proposed`.
- **The running record** — `docs/implementation/synveda-context-platform.md`
  §10: every CPR prompt appends what it implemented, what changed in the
  schema and the API, what it deleted, what it tested, and the commit
  hash. Write it as part of the prompt, not after.
- **The phase narrative** — CLAUDE.md "Current phase" is the Claude Code
  mirror of this file's Current state, with the full counting trail. When
  state moves, both files move; the durable records above win any
  disagreement.
- **Session handoff** — before ending long work, make sure the running
  record and STATUS.md carry what the next session needs: what was done,
  what changed, what is half-finished, and the commit. Start the next
  session from the required reading above. Harness-local checkpoints
  (e.g. `.claude/RESUME.md`) are conveniences, not memory.

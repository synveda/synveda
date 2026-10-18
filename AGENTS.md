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

- Schema epoch 3 is the byte-pinned published `0001_context_platform.sql`
  baseline plus additive forward migrations; `crates/synveda-store/src/epoch.rs`
  pins the head. Other schemas are refused with reset guidance, never upgraded.
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
- Client adapters: read the adapter's README and ADRs first. For Claude Code,
  `make claude-acceptance` is deterministic replay through a live gateway and
  `make claude-acceptance-live` needs an installed, authenticated client
  (exit 77 means unavailable, not pass). Never present replay, generated
  configuration or a logged-out client as live verification.

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
- No pre-1.0 compatibility paths.
- Core dependency licences are MIT, Apache-2.0 or PostgreSQL; repository gates
  and accepted exceptions remain authoritative. Preserve notices/attribution.

### Architecture

- Functional core, imperative shell: decisions, validation and planning are
  pure and unit-tested; I/O (store, HTTP, workers) stays at the edges.
- Fail closed: unknown, unanswered or malformed means refuse, never allow.
- One source of truth: derive peers (routes, OpenAPI, clients, SQLx metadata)
  from one declaration and test that they agree.
- Retries are safe (idempotency keys, replay) and outputs deterministic; tests
  never order events with sleeps.
- Do not over-engineer: no layers, flags or abstractions for hypothetical
  needs; the simplest design that meets today's contract wins.
- Reuse before writing: prefer a maintained open-source library that passes
  the licence and dependency gates to a hand-rolled reimplementation.

### Rust (NASA/JPL "Power of 10", adapted)

1. Simple control flow: no recursion over untrusted or unbounded data; early
   returns over deep nesting; no control flow hidden in macros or `Drop`.
2. Bound everything: every loop, retry, queue, channel, page, batch, buffer,
   stream and request has an explicit limit and timeout, and exceeding it
   fails closed with a typed error.
3. Allocate in proportion to validated, capped input; stream rather than
   collect unbounded data.
4. Small functions with one job, about a screen (70 lines) at most.
5. Encode invariants in types first (newtypes, closed enums, private
   constructors); otherwise `debug_assert!` internal invariants and return
   typed errors for anything input can reach. No unjustified production
   `unwrap`, `expect`, `panic!`, `todo!` or `unimplemented!`.
6. Smallest scope: private by default, `pub(crate)` for real seams, no global
   mutable state.
7. Check every result: never discard a `Result` or `#[must_use]` value
   without a stated reason; propagate causal errors without secrets or
   existence leaks; validate untrusted input at the boundary.
8. Functions and generics before macros; `macro_rules!` only to remove real
   duplication with one obvious expansion.
9. Restricted indirection: production crates `#![forbid(unsafe_code)]`;
   prefer closed enums to `dyn` and function pointers; no `Rc<RefCell<_>>`
   webs.
10. Zero warnings: rustfmt, Clippy and Rustdoc with `-D warnings`, cargo-deny
    and the repository gates; every `#[allow]` states its reason.

### Comments

- Default to none. Code shows *how*; a comment carries only a *why* the code
  cannot: a non-obvious constraint or invariant, deliberate deviation, gotcha,
  workaround or cross-file sync obligation.
- Never narrate the code (steps, names, types, block ends) or the change
  ("fixed", "now", "as requested"). A comment must read correctly to someone
  who never saw the diff; history belongs in the commit message.
- Delete a comment that restates a decision the code already shows, even if
  it links a doc.
- The substance must survive with any link removed. Never cite point-in-time
  artifacts (specs, briefs, section numbers, design drafts); system-level
  rationale goes in a maintained doc, not inline. Feature and ADR IDs,
  tickets and permalinks may trail as breadcrumbs.
- Razor every comment you keep: one non-obvious fact in the fewest words,
  without mechanism, downstream use or justification chains. Suspect any block
  over two lines; the right answer is sometimes none.
- A one-line summary on a public item is fine; restating a clear line is not.
- TODOs need no ID, but never replace in-scope work.

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

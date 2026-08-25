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
public session/event, context-run and scoped Knowledge APIs, and every read
and write passes an embedded Cedar Policy Decision Point. This product sells
trustworthiness.

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
OPS-9, OPS-10, TEN-5,6, AUD-3,4, EVAL-6, CTX-7, OPS-3,4, ADPT-3,
CTX-6 and FLOW-8 are still open, and no live Entra/Okta tenant or real
  Cursor frame has been replayed. **135 features filed, 105 delivered**;
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
- CPR-29 (ADR-0088): the generated OpenAPI contract is the complete
  authenticated application plane — established at 156 operations and
  extended to **171 operations by CPR-37** — from `/v1/me` through governance,
  audit, Knowledge, capture, context, Skills, Tools, OKF and Configuration. One
  executable route catalogue builds the router and supplies the inventory an
  exact parity test compares with OpenAPI. The console has no hand-written
  application operations; ordinary service/audit CLI commands and the generic
  MCP adapter are public-API clients. Only documented local bootstrap,
  reset/migration, key custody and break-glass operations retain direct local
  authority.
- CPR-30 (ADR-0089): runtime configuration is a stable aggregate with
  immutable content-hashed versions and revisioned nearest-scope bindings.
  The `personal`, `team` and `enterprise` profiles are canonical documents,
  not runtime branches. Policy-pack selection, capture rules, context budget
  and channels, trace retention, type-aware freshness, Skill/Tool
  advertisement and provider allowlists all resolve from the same exact
  version. Every mutation is a typed VedaFlow `Configuration/apply` change;
  capture/context record version evidence; all four tables are forced-RLS.
  The mutable default/assignment tables and routes are deleted without
  translation, and no binding falls back to them.
- CPR-31 (ADR-0090): policy relaxation is one stable aggregate with immutable
  versions naming an exact provisioned subject, non-personal scope,
  `knowledge.read` permission, tier and hard expiry. Create, revise and revoke
  are typed `Policy/apply` VedaFlow changes; open collaboration may auto-apply
  the same persisted change that standard retains for review or rejection.
  Database time ends authority, current Configuration can only narrow it, and
  Cedar still decides with privacy, quarantine and service confinement above
  the permit. Three tables are forced-RLS; the predecessor table, effect,
  routes, CLI/DTOs and pack setting are deleted without translation.
- CPR-32 (ADR-0091): one VedaFlow proposal/review lifecycle governs every
  artifact family. Mandatory immutable typed references bind stable ids,
  operations, exact versions/digests and stale-head preconditions; verdicts
  name the inspected commit. Live matrix rules may forbid author self-review
  and require the effect actor to differ from author and reviewers, while
  Cedar remains the authority. Advanced Reviews filters, decides, cancels and
  executes this common lifecycle; New Learnings and session quarantine remain
  their distinct candidate and admission boundaries.
- CPR-33 (ADR-0092): audit query extends the existing tenant-complete chain,
  not a replay or second store. Typed artifact/session/context filters,
  distinct valid/as-known Knowledge evidence and exact effective
  Configuration/relaxation/Skill/Knowledge references remain content-free
  behind tenant-root `AuditRead`. Frozen-head cursor export contains every
  canonical hash input and verifies offline; its own reads land after the
  frozen prefix. Hashes-only evidence stays explicitly unresolved without an
  invented address. The public contract had **165 operations** at that
  checkpoint.
- CPR-34 (ADR-0093): directory users retain source-owned adapter attributes
  while projecting one-to-one onto identities and principal scopes;
  directory groups are the shared Group aggregate and membership keys stable
  identities before login. SCIM push and complete/partial Entra/Okta pull use
  one idempotent projection with stable provider ids and complete-pass-only
  retirement. Directory access assignments are ordinary source-bearing group
  grants behind `MembershipGrant`, RLS and the access audit chain; the wire
  cannot name one. Removing membership, disabling an identity or archiving a
  group withdraws authority at the next ordinary Cedar decision. The old SCIM
  group graph is deleted without translation. Fixtures are deterministic
  captured/transcribed evidence, not live vendor verification. The public
  contract has **167 operations**.
- CPR-35 (ADR-0094): a local tenant secret is a stable UUIDv7 aggregate with
  governing scope, closed kind, logical revision, key generation and
  active/revoked state. Internal Tool and directory references resolve the
  same forced-RLS row and fail closed on every use; value or DEK rotation does
  not rewrite immutable Tool history. Durable re-encryption jobs advance only
  envelopes, while the hard-cut sealed export now carries Knowledge heads,
  immutable history, provenance, relations and audit under
  `synveda-context-export-2`. The old name row, Record export and old archive
  reader are gone; cloud KMS, HSM and customer-managed keys remain unclaimed.
- CPR-36 (ADR-0095): installed-host, source/release Compose and Helm are
  infrastructure shapes around one gateway, schema epoch and generated public
  contract; personal/team/enterprise behaviour remains immutable governed
  Configuration data. Local init converges `synveda_gateway` as a LOGIN,
  non-superuser, non-BYPASSRLS member of `synveda_app`, and Helm verifies the
  same boundary; migration/tenant admission keep the admin identity. The dead
  `init --demo` people and packaged ACME seeder are deleted without aliases,
  and `make check-deploy` renders every shape, checks current routes/roles and
  proves repeat packaging cannot retain removed assets. The chart still pins
  one gateway replica and release upgrades remain restart-shaped.
- CPR-37 (ADR-0096): bounded post-PDP classification creates durable
  forced-RLS conflict sets with exact Knowledge-revision or capture-candidate
  evidence. Ambiguous heads are `transitional` and invisible to ordinary
  retrieval until revision-aware VedaFlow resolution chooses support,
  separate truth, supersession, future transition or archive; denied members
  suppress the whole public set. Freshness is an evaluated view of the exact
  governed Configuration version, not a second mutable policy plane, and
  public queries compose valid time, as-known transaction time, history and
  transitional state. The Knowledge Browser exposes comparison, staleness,
  future transition and resolution; the contract has **171 operations / 272
  schemas**.
- CPR-38 (ADR-0097): immutable `KnowledgeRelation` is the only runtime graph.
  ContextRun v2 expands at most two hops from already-authorised current
  lexical/semantic anchors under governed fan-out, candidate, time and token
  bounds, then re-authorises every endpoint and path before persistence and
  rendering. Six supporting relation types rank; contradiction is a
  zero-weight warning. Forced-RLS trace steps retain exact or hashes-only
  evidence by the existing retention mode, and every failure/bound falls back
  to lexical/vector results with a named degradation. The Record graph and
  ingest linker are deleted without translation; GRPH-3 closes by subsumption.
  The contract remains **171 operations / 274 schemas**.
- CPR-10 (ADR-0076): **a run is a record**. `sessions`, `session_events` and
  `session_context_runs` replace `session_id: text`; the governed scope a run
  is decided at is derived from its workspace and project by composite keys
  and a CHECK, and no body may name a tenant, an acting principal or a scope.
  Five states (the close is two-phase), events immutable and ordered by a
  server-assigned `sequence` and idempotent per event, a timeline projected
  over two tables and merged rather than sorted. `/v1/observe`, `/v1/inject`
  and `/v1/recall` are untouched. CPR-10 forecast that Prompt 11 would re-cut
  them; it did not — Prompt 11 turned out to be CPR-11 — so the observe re-cut
  is **open and unscheduled** (§10 of the implementation document).
- CPR-11 (ADR-0077): **that record is usable**. `GET /v1/sessions` is
  keyset-paginated (`cursor` in, `next_cursor` out, `truncated` **deleted**),
  and the cursor follows the last candidate a page *considered* rather than
  the last row it served — so a page may be empty and still carry one. Four
  more filters (client, principal, and a half-open day range). Timeline event
  entries carry `received_at` beside `occurred_at` and a server-computed
  `delayed`, which is one flag and not three: a spool replay, a crash replay
  and a wrong clock are indistinguishable from two instants, so the server
  reports the gap and names no cause. A raw payload is its own authority —
  `GET /v1/sessions/{id}/events/{event_id}` under `SessionDiagnostics`,
  strictly narrower than each pack's own `SessionRead` (**packs @19 → @20**).
  `sessions.end_reason` (migration `0045`) says *why* a run stopped. And a run
  has an address: `/console/sessions/{id}`.
- CPR-12 (ADR-0078): **the session plane is the only adapter plane**.
  `/v1/observe`, `/v1/inject` and `/v1/recall` are deleted with the old
  storage and queue; extraction consumes `session_events`; context enters
  through `POST /v1/sessions/{id}/context-runs`; and the Claude adapter writes
  a versioned, atomic local spool that retries per-event idempotently.
- CPR-14 (ADR-0079) is delivered at all three evidence tiers. Genuine Claude
  Code 2.1.220/2.1.241 frames run through the built hook, public session API,
  PDP, current Postgres, timeline and audit in CI, including outage and
  lost-ack recovery. On 2026-08-24 the installed authenticated Claude Code
  **2.1.241** client loaded plugin **0.2.0**, reported four hooks and one MCP
  server enabled, composed one context run, appended four authentic ordered
  events and ended the same run. Stop and PreCompact synchronously cross only
  the atomic local-spool boundary; SessionEnd/next SessionStart deliver. This
  also closes ADPT-8 without changing the host-killed-before-any-hook tail.
- CPR-15 (ADR-0080) is delivered: stable Knowledge heads, immutable
  revisions, normalised independently scoped sources, explicit relations and
  a bitemporal current projection. It creates no application mutation path
  and neither reads nor writes the old record model.
- CPR-16 (ADR-0081) is delivered: the eight Knowledge commands reuse the
  VedaFlow proposal/approval engine, with policy auto-apply, live
  re-authorisation, immutable revisions, content-free audit and durable
  held-or-completed erasure rather than a second workflow. The gateway no
  longer starts the old promotion or retention writers; CPR-17 deleted the
  public record seam and CPR-18 deleted the extraction writer.
- CPR-17 (ADR-0082) is delivered and also subsumes CNSL-4: thirteen public
  Knowledge operation groups expose current immutable revisions, filtered
  cursor listing, lexical search and TEI-only semantic fusion. Every item,
  source and relation endpoint is decided independently; all writes use
  CPR-16's VedaFlow command seam. The proposal classification route/CLI/eval
  call, public record proposal fields, record channel publication/aliases and
  raw-record browser fixtures are deleted. CPR-20 subsequently deleted the
  temporary record-backed context composer.
- CPR-18 (ADR-0083) is delivered: terminal or explicit session capture freezes
  an exact event snapshot into a restart-safe batch. Extraction creates only
  reviewable candidates with same-session evidence and independently decided
  Knowledge matches; accept/edit/merge/replace enter CPR-16's VedaFlow command
  seam, while dismiss publishes nothing. Candidate and match disclosure is
  re-authorised, candidate decisions are retry-safe, and Knowledge erasure
  scrubs candidate plaintext. The PGMQ `session_events` queue and the old
  record/embed/dedup/link extraction writer are deleted. The generated
  contract grew to 62 operations; CPR-20 subsequently replaced the remaining
  record-backed context reader.
- CPR-19 is delivered with no new ADR or backend: New Learnings groups capture
  candidates by batch, previews exact session evidence, freshly reads visible
  Knowledge comparisons and offers scope-safe accept/edit/merge/replace/
  dismiss through generated commands. Only anchors forecasting
  `knowledge.write` enter its private/project/workspace picker; exact PDP
  decisions still happen at the gateway. Applied results link to Knowledge and
  pending ones remain explicitly unpublished under Advanced Reviews. The
  console suite is **165/165** and the planned placeholder is gone.
- CPR-20 (ADR-0084) is delivered: context runs now plan over current immutable
  Knowledge revisions and retain independently authorised candidates,
  selections, score/reason detail and revision-specific feedback under four
  trace-retention modes. Five generated operations add run list/detail,
  feedback and distinct session-scoped ordinary/diagnostics Knowledge query
  lenses (67 operations total). The runtime record composer and recall
  tombstones are deleted; no global recall route or translation layer returns.
  Context packs and skill advertisements remain separately governed authored
  inputs, and their aggregate historical block is masked when exact authored
  authority cannot be reconstructed.
- CPR-21 is delivered: `/console/context-runs/{id}` is the generated-API
  Context Inspector over CPR-20's re-authorised trace and exact-revision
  feedback. Session context entries link it and carry only a freshly visible
  `Synveda supplied N knowledge items` summary; full/redacted/hashes-only/
  disabled remain honest, and no schema, Cedar/audit action or parallel
  telemetry model was added.
- CPR-22 is delivered: the isolated PulseBoard acceptance composes the complete
  personal/team MVP over public APIs — session evidence, reviewable candidates,
  VedaFlow Knowledge, clean teammate reuse, principal privacy, explicit
  supersession and the Context Inspector — with a verifying content-free audit
  chain, zero record writes and all three deleted global runtime routes still
  404. It is deterministic application acceptance; CPR-14 remains the genuine
  live Claude Code evidence.
- CPR-13 is delivered after that MVP checkpoint: 49 affected demo scripts are
  concise current-scope/session/capture/Knowledge/context narratives, 17,972
  lines smaller after copied retired setup was deleted. `make check-demos`
  recursively validates all 75 shell scripts against freshly built Clap help
  and generated OpenAPI paths, including binary aliases; MEM, CTX, FLOW,
  AUTHZ and the authentic-frame ADPT live-Postgres representatives pass.
- CPR-23 (ADR-0085) is delivered: stable Skill aggregates own immutable,
  content-addressed versions; project/principal bindings follow current or pin
  an exact version and revisioned disable/rollback changes distribution rather
  than history. Install, update and binding effects all pass through typed
  VedaFlow changes with live PDP, scan/rubric, RLS and content-free audit
  evidence. Eight usage stages separate host observation from model report,
  the controlled validation harness runs no bundle code, declared tools grant
  nothing, and context advertises the same exact resolved versions the public
  API serves. The old mutable drafts, `skill/published` distribution and
  special checklist/override mutation paths are gone.
- CPR-24 is delivered: the generated Skills Library is the only Skill product
  surface. Its catalogue and stable detail address expose exact immutable
  versions, files, provenance, scans, personal/project bindings, controlled
  tests and evidence-labelled usage; capability forecasts hide unavailable
  controls while every mutation still enters CPR-23's VedaFlow API. The last
  hand-written Skill request and mutable-Skill branch of console/CLI proposal
  review are deleted; Advanced Reviews remains artifact-neutral.
- CPR-25 (ADR-0086) is delivered: the trusted MCP catalogue has stable server
  identities, immutable raw/normalised version evidence, quarantined drift,
  exact revisioned project bindings and discovery-only test reports. Every
  approval/binding transition is a typed VedaFlow Tool change under the PDP,
  all six tables are forced-RLS and audit carries ids/digests/counts rather
  than descriptions or secrets. MCP `2026-07-28` is the pinned stateless
  contract; the gateway never launches imported stdio commands, proxies
  `tools/call`, treats descriptions as authority or emits a credential value.
- CPR-26 is delivered: the generated Tools catalogue and stable server address
  expose immutable discovery/schema evidence, approved-head diffs, obvious
  quarantine linked to common VedaFlow review, exact revisioned project
  bindings, named discovery-only adapter reports and masked configuration.
  Capability forecasts hide unavailable controls, declared metadata grants no
  authority, and the deleted `Planned` page has no successor placeholder.
- CPR-27 (ADR-0087) is delivered: canonical OKF v0.2 is a version-pinned leaf
  adapter over the shared Knowledge/capture domains. Bounded, path-safe inputs
  produce immutable dry-run plans and reviewable candidates only; acceptance
  still enters VedaFlow, and deterministic export independently re-authorises
  current Knowledge. Four import tables are forced-RLS, audit is content-free,
  unknown v0.2 metadata survives and there is no v0.1 fallback, network fetch,
  script execution or direct publication path.
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

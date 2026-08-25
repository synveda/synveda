# Synveda

Enterprise memory & context management platform for AI agents.
Rust workspace + TypeScript adapters. Postgres-first. Governed by VedaFlow.

## Required reading (in order, before any task)
1. docs/SYNVEDA_SEED.md        — product principles & invariants (§2 is law)
2. docs/SYNVEDA_TECH_PLAN.md   — stack decisions & VedaFlow design
3. docs/SYNVEDA_FEATURES.md    — feature backlog; ALL work maps to a feature ID
4. docs/backlog/STATUS.md      — what is done, what each feature found, what it left standing
5. docs/implementation/synveda-context-platform.md
                               — the Phase 5 context-platform redesign: the base-commit
                                 inventory, the deletion map, the ordered programme and its
                                 running record. Required before any CPR work; ADR-0068's
                                 eight decisions are locked and no prompt reopens them.

## Working rules
- Every task references a feature ID (e.g. FND-1). Branch: feat/<ID>.
  Commit messages include the ID: "FND-1: scaffold rust workspace".
- A feature is done ONLY when its acceptance criteria in SYNVEDA_FEATURES.md pass,
  demonstrated by a test or a runnable demo script under demos/.
- Never create a code path that bypasses the PDP (seed §2.2), even in tests —
  use a test policy pack instead.
- Architectural choices get an ADR in docs/adr/ (copy 0000-template.md) BEFORE
  implementation.
- Crate dependency rule (seed §8; tech plan §5 adds synveda-vedaflow, ADR-0064
  adds synveda-crypto): types ← crypto ← {policy, store, identity, audit}
  ← retrieval/ingest ← gateway. Nothing imports upward. Adapters/SDKs depend only
  on the public API.
- Licences: MIT/Apache-2.0/PostgreSQL only in the core path. cargo-deny enforces.
- Prefer boring, explicit code over cleverness. This product sells trustworthiness.
- cargo fmt + clippy -D warnings must be clean before any commit.
- sqlx compile-time checked queries only; no string-built SQL, ever.

## Definition of done (every feature)
1. Acceptance criteria met and demonstrated
2. Tests written (unit + the AC test)
3. Tracing spans + metrics on new paths
4. Audit events emitted for any new action type
5. docs/backlog/STATUS.md updated

## Current phase
Phase 5 — Context platform redesign, since 2026-08-17. Phase 3 is paused
mid-phase, not finished: OPS-9, OPS-10, TEN-5,6, AUD-3,4, GRPH-3, EVAL-6,
CTX-7, OPS-3,4, ADPT-3, CTX-6 and FLOW-8 are still open, and the phase's
demo goal is met except for the two live-tenant claims. What moved is the
audience. Everything above Phase 5 was built for an organisation — and
until CPR-7 a tenant's hierarchy root *had* to be `kind = 'org'`. Phase 5
re-cuts that as 33 ordered prompts on `feat/context-platform-mvp`, with the
decisions locked in ADR-0068 and the running record in
docs/implementation/synveda-context-platform.md. **Prompts 1–13 are
delivered**. The external CPR-14
acceptance gate is delivered at replay/live-gateway and real-client tiers.
CPR-15, the versioned Knowledge persistence aggregate, is delivered.
CPR-16, the governed Knowledge mutation lifecycle, is delivered.
CPR-17, the generated public Knowledge API/search/browser hard cut, is
delivered and also closes CNSL-4 by subsumption.
CPR-18, the session capture-batch and reviewable-candidate cutover, is
delivered; the old record extraction writer and its PGMQ queue are gone.
CPR-19, the New Learnings lightweight review workflow, is delivered over that
generated candidate contract; its scope-safe actions all enter VedaFlow and
the placeholder is gone.
CPR-20, the explainable Knowledge context planner and scoped-query cutover, is
delivered; current immutable Knowledge, re-authorised traces and exact feedback
replace the final runtime record reader under accepted ADR-0084.
CPR-21, the generated Context Inspector and outcome-feedback product surface,
is delivered with no new ADR, schema, policy/audit action or API operation.
CPR-22, the core personal/team MVP acceptance gate, is delivered: its isolated
PulseBoard cross-session scenario, complete CI and full database suite pass.
CPR-13, deliberately sequenced after the final MVP surfaces it documents, is
delivered: 49 affected demos now teach the current platform and a recursive
generated-help/OpenAPI drift gate covers all 75 shell scripts in `make ci`.
CPR-23, the immutable Skill catalogue and governed binding cutover, is
delivered under ADR-0085; the old draft/channel distribution path is gone.
CPR-24, the generated Skills Library and mutable-Skill review cutover, is
delivered; exact versions, files, bindings, controlled tests and usage evidence
now share one policy-aware product surface, while Advanced Reviews is
artifact-neutral.
CPR-25, the trusted MCP catalogue and exact-binding backend, is delivered under
ADR-0086; immutable discovery evidence and quarantined drift use the stable
stateless MCP 2026-07-28 contract, while the gateway executes no imported tool.
CPR-26, the generated MCP Tools catalogue product experience, is delivered;
the last placeholder is gone and approval remains in common Advanced Reviews.
CPR-27, the OKF v0.2 knowledge exchange adapter, is delivered under ADR-0087;
the canonical specification is pinned to `ad30107`, imports materialise
reviewable candidates only and deterministic export re-authorises current
Knowledge.
CPR-28, the public-API OKF CLI and generated project-console workflow, is
delivered; local paths never become gateway authority and imports still stop
at New Learnings.
CPR-29, the exact generated application contract and client convergence, is
delivered under ADR-0088; the 156-operation authenticated application contract,
executable router, generated console client and ordinary CLI/adapter boundary
are now exact.
CPR-30, governed runtime Configuration, is delivered under ADR-0089; immutable
documents and revisioned scope bindings select policy, capture, context,
freshness, advertisement and providers over one runtime.
CPR-31, governed policy relaxation, is delivered under ADR-0090; immutable
exact-subject `knowledge.read` versions replace the pre-cut mutable plane, and
personal auto-apply is an outcome of the same VedaFlow/PDP/audit path stricter
profiles retain for review.
CPR-32, unified approvals, is delivered under ADR-0091; immutable typed
artifact references, exact-commit verdicts and configurable separation of
duties now span Knowledge, Skills, Tools, Configuration, relaxations and OKF
through one generated Advanced Reviews lifecycle.
**It is a pre-1.0 hard
cut**: a fresh schema epoch, no old-data migration, no compatibility shims,
and old databases rejected with a reset instruction. Since CPR-2 that is
enforced rather than planned (ADR-0069), and since CPR-7 the epoch is **2**
(ADR-0074): the migration chain was rewritten in place — the scope
substrate sits at `0004` where the hierarchy was, and `role_bindings` and
the old hierarchy files left it (43 → 41 migrations) — so a database at
epoch 1 is refused with the reset instruction by the guard rather than by
a checksum error. **Your dev database will be refused** — reset it
(`synveda reset --database --force`). The chain is not squashed yet; that
is Prompt 33.

Since CPR-3 the governed scope model (ADR-0070): `scopes` + `scope_closure`,
a named node with a parent and a subtree, where `kind` is a **shape**
deciding only which shapes may be its parent — `tenant`, `org_unit`,
`workspace`, `project`, `principal` — so an `org_unit` nests inside itself
to any depth and one person's whole tree is a tenant scope and a principal.
Since CPR-7 **it is the only tree** (ADR-0074): the old hierarchy, role
bindings, the rank vocabulary, `/v1/hierarchy/*`, `synveda hierarchy`,
`synveda role bind` and the placement conventions are deleted whole, with
negative tests asserting the 404s and the old kinds failing validation by
name. Placement is identity (decision 3): an identity's scope is its own
`principal`-shaped scope — minted at first login for users, keyed by
`externalId` for directory identities (adopted at login through the
correspondence rule), under the operator's anchor for services — and
"unmapped" means *ungranted*, decided per action by the anchor model rather
than per person by a flag. The `synveda-admins` IdP-group convention
upserts an `administrator` grant at the tenant root (decision 4) — the
operator door CPR-6 recorded as missing. The admin surface is public
(decision 5): six `/v1/admin/scopes` routes (list/create/get/patch/
ancestors/descendants, with pack assignment at `…/policy` and curator
files at `…/curators` re-homed under it) and five CLI commands
(`synveda scope list|show|create|move|tree`). A **move** is decided at
both ends and audited with both. There is no delete: retiring a scope is a
status transition. And **one role vocabulary** (decision 6): grant keys
only, everywhere — proposals' approvals, curator files, the approval
matrix (steward/org-admin/compliance → administrator, security-reviewer →
reviewer) and every Cedar role list.

Since CPR-4 the substrate has a surface (ADR-0071): `workspaces`,
`projects` and `project_repositories` as **product-level subtypes** of a
governed scope, each owning one scope **created in the same transaction as
itself**, with the tenant root minted by the first thing that needs a
parent — so a person's first act is `POST /v1/workspaces` and nobody is
asked to declare an organisation. Two rules bind creation and update on
this plane: **a required `Idempotency-Key`** (same key + same request
replays with 200; same key + a different body is 409) and **a required
`expected_revision`** (a mismatch is 409 and writes nothing). A
repository's identity is its **canonical remote URI** — transports,
credentials, ports and `.git` collapse — and **a filesystem path is never
one**, refused by name in `synveda_types::repository` and by a CHECK
behind it.

This product has an **OpenAPI contract** since CPR-4: `docs/api/openapi.json`
is derived by `utoipa` from the gateway's own handlers, a test fails when
the committed file and the tree disagree, and `console/src/generated/api.ts`
is generated from that file (`make check-api-types`). **Never hand-edit
either.** To refresh both: `SYNVEDA_WRITE_OPENAPI=1 cargo test -p
synveda-gateway --test openapi` then `node scripts/generate-api-types.mjs`.
The document covers the complete authenticated application plane — **164
operations** from `/v1/me` through workspaces/projects/repositories, access,
governance, policy, audit, sessions, Knowledge, capture, context, immutable
Skills, trusted MCP Tools, OKF, Configuration and policy relaxations. Since
CPR-29 one route catalogue constructs
the executable router and exposes the method/path inventory that the OpenAPI
test compares exactly in both directions; the console has no hand-written
application operation, and ordinary service/audit CLI and generic MCP paths
are public-API clients. Since CPR-8 the generator also emits
the **runtime** path/method table beside the type table and marks every
operation whose document requires an `Idempotency-Key`, so the console's
client requires the key at compile time and no hand-written copy of a path
exists.

Since CPR-5 those workspaces have members (ADR-0072): `groups`,
`group_members`, `scope_grants` and `pending_invites`, where a grant gives
a **subject** — a principal or a group — a **role key** at a scope and the
scope's subtree inherits it, so a workspace grant reaches its projects
with **no row written there**. Creating a workspace or a project mints an
`owner` grant for its creator. Six role keys and **no permission table** —
what a key permits is the Cedar packs', and a second mapping would be a
second decision point. A `principal`-shaped scope **inherits nothing**:
nobody's own scope is reachable from above. A principal is a **token
subject**, not an `identities` row. Invitations are one-time, expiring,
revocable tokens returned **once** with a copyable URL and redeemed with
the recipient's own bearer — no email delivery anywhere.

Since CPR-6 **a grant decides** (ADR-0073). The PDP is re-cut over the
governed scope model: `synveda_store::anchors::resolve` answers "where
does this request stand" as an **ordered set** — the caller's own scope,
the selected project, the selected workspace, the organisation units
above them, the tenant root, and every scope a direct or group grant
names. Cedar has **seven** entities (`Tenant`, `Scope`, `Principal`,
`Group`, `ScopeGrant`, `Workspace`, `Project`), each subtype parented to
the scope it owns, so a decision **names what it is about**. `Scope.kind`
is the five shapes, and `standard` shares by `principal.ambit` — the
parent of what you hold. **Personal principal-scope privacy is a
base-layer forbid** no pack can drop, with one door: a grant written
*directly at* somebody's own scope reaches it. `GET /v1/me` mints the
caller's `principal` scope and forecasts `PROBED_AT_SCOPE` **at each
anchor** from real decisions. Since CPR-7 there is **one gather** and
`context.roles` carries grant keys only; the composition plane
(`MemoryReadInputs`) receives `anchors`/`groups` and no bindings. The
ownership check runs **before** the decision on every per-object route, so
a made-up id is a 404 rather than a 403. The first-grant gap stands for
admission-level bootstrap only: a login with the `synveda-admins` IdP
group mints the tenant's first grant; a dev-token tenant seeds it by hand
once (INSTALL.md's SQL), and the harnesses and demos that need it seed it
explicitly and say so.

Phases 0, 1 and 2 are complete; SKIL-1 through SKIL-4, OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4, AUTH-5, EVAL-3, OPS-2, TEN-3, TEN-4 and OPS-8 are the Phase 3
features done. 98 of 129 features delivered — see docs/backlog/STATUS.md for
what each one proved and what it left standing. (The total read 86 until
2026-08-05, when it was corrected to the 88 STATUS.md and `make
check-backlog` had both said for some time; AUTHZ-7 was filed the same day by
CNSL-2/ADR-0058, making it 89, EVAL-7 on 2026-08-07 by EVAL-3/ADR-0061,
making it 90, OPS-7 on 2026-08-10 by OPS-2/ADR-0062, making it 91, CTX-7 and
TEN-7 the same day by TEN-3/ADR-0063, making it 93, and OPS-8 filed and
delivered on 2026-08-11, making it 94. ADPT-8 was filed on 2026-08-13 by
running ADPT-1's plugin in a real Claude Code session, making it 95, and
OPS-9 the same day by asking what somebody who takes OPS-8's install actually
sees, making it 96 — this trail had stopped at 94 while the headline said 95,
which is the drift the trail exists to prevent. **OPS-10 was filed the same
day as OPS-9**, by asking how that same stranger removes it, making it 97;
the trail missed it and the headline read 96 against a checker that had said
97 since — the identical drift, one entry later. CPR-1 filed and delivered
2026-08-17, making it 98, **CPR-2 the same day**, making it 99, and
**CPR-3 the same day**, making it 100. **CPR-4 the same day**, making it 101 —
which the trail missed again, so the headline read 100 against a checker that
had said 101 since: the third time this exact drift has been recorded, and the
reason the trail exists. CPR-5 filed and delivered 2026-08-18, making it 102,
CPR-6 on 2026-08-19, making it 103 with 70 delivered, and **CPR-7 on
2026-08-20**, making it **104 with 71 delivered** — the hierarchy cutover,
which deletes whole subsystems six earlier prompts had recorded as standing
beside the new model — **CPR-8 on 2026-08-21**, making it **105 with 72
delivered**: the console product shell, the first prompt of this programme
whose deliverable is a screen, and the one that makes six prompts of platform
reachable by somebody who is not holding a terminal — and **CPR-9 on
2026-08-22**, making it **106 with 73 delivered**: the foundation audit, the
first prompt asked to check its predecessors rather than build on them — and
**CPR-10 on 2026-08-23**, making it **107 with 74 delivered**: the session
ledger, the first of Stage B and the prompt that makes what an agent *does* a
governed record — and **CPR-11 on 2026-08-24**, making it **108 with 75
delivered**: the session product experience, which turns that record into one
somebody with a question can use — and **CPR-12 on 2026-08-23**, making it
**109 with 76 delivered**: durable Claude session delivery, the prompt that
makes the session plane the only one by deleting the three global routes that
were still this product's actual write path. It filed **CPR-13** the same day,
making it **110 with 76 delivered** — the demo corpus re-point, which exists
because CPR-12 went looking for the demos it had to change and found 43 of 65
already dead. Prompts 13–33 of its programme are filed by the prompts that run
them, so this number will keep moving. **CPR-14 was filed on 2026-08-23**, making
it **111 with 76 delivered**: the live Claude Code session acceptance gate.
On 2026-08-24 the authenticated installed **2.1.241** client completed that
gate, delivering CPR-14 and simultaneously closing ADPT-8 at **111 with 78
delivered**: Stop now crosses only the synchronous local-spool boundary and
SessionEnd/next SessionStart deliver, proved by a real four-event run. **The
headline above read 70 of 103
against a trail that had said 108 since CPR-11 and a checker that had said 110
since CPR-12 filed** — the same drift, a fourth time, and once again found by
reading the trail rather than by any gate.) **CPR-15 was filed and delivered
on 2026-08-24**, making it **112 with 79 delivered**: the stable Knowledge
aggregate and immutable revision/provenance boundary, with no bridge to
`records`. **CPR-16 was filed and delivered the same day**, making it **113
with 80 delivered**: one VedaFlow-governed lifecycle and durable erasure seam,
with the old extractor, promotion and retention runtime writers stopped.
**CPR-17 was filed and delivered the same day**, making it **114 with 81
delivered**: current immutable Knowledge reached the generated public API and
the console, with per-object/source/edge decisions and honest lexical/semantic
search, while the public raw-record classification and channel seams were
deleted. It simultaneously subsumed the already-filed **CNSL-4 Knowledge
browser**, making it **114 with 82 delivered** rather than leaving the replaced
Memory-browser objective falsely open.
**CPR-18 was filed and delivered the same day**, making it **115 with 83
delivered**: exact session-event snapshots now become restart-safe reviewable
capture batches and candidates; publication enters CPR-16's VedaFlow Knowledge
command seam, while the old record extraction writer and PGMQ queue are
deleted.
**CPR-19 was filed and delivered the same day**, making it **116 with 84
delivered**: New Learnings now groups those candidates with exact source
evidence and fresh Knowledge comparisons, offers only policy-forecast
private/project/workspace destinations, and sends every decision through the
generated VedaFlow-backed capture commands rather than a second review model.
**CPR-20 was filed and delivered the same day**, making it **117 with 85
delivered**: its Knowledge-only explainable planner removes the final runtime
record reader, persists re-authorised trace/feedback evidence under four
retention modes and adds separately authorised session-scoped ordinary and
evaluation query lenses without restoring global recall.
**CPR-21 was filed and delivered the same day**, making it **118 with 86
delivered**: its linkable generated Context Inspector renders those traces
under all four retention modes, binds explicit feedback to one revision and
links a freshly visibility-counted, content-free session timeline summary
without adding a second telemetry model.
**CPR-22 was filed and delivered the same day**, making it **119 with 87
delivered**: the isolated PulseBoard acceptance proves the session → candidates
→ VedaFlow Knowledge → clean teammate context → explicit supersession →
inspector loop with Alice's private preference absent; complete CI and the full
fresh-scratch database suite pass.
**CPR-13 was delivered after that checkpoint**, making it **119 with 88
delivered**: 49 affected scripts and their shared harness are 17,972 lines
smaller after stale copied setup was deleted,
all 73 shell demos are checked against fresh recursive CLI help and generated
OpenAPI, and the required MEM, CTX, FLOW, AUTHZ and ADPT representatives pass
against isolated current Postgres (the ADPT run uses authentic captured frames;
CPR-14 remains the distinct genuine-client evidence).
**CPR-23 was filed next**, making it **120 with 88 delivered**: it replaces the
mutable draft/channel skill registry with stable immutable versions, governed
project/principal bindings, evidence-labelled usage and controlled test runs.
It was delivered the same day, making it **120 with 89 delivered**: the public
contract grows to 85 operations, exact bound versions reach context, and the
old mutable/channel/checklist-override paths are deleted without translation.
**CPR-24 was filed next**, making it **121 with 89 delivered**: it replaces the
last mutable-Skill console/CLI review residue with the generated Skills Library
over CPR-23's exact versions, bindings, controlled tests and usage evidence.
It was delivered the same day, making it **121 with 90 delivered**: the
generated Library owns the linkable product surface, and Advanced Reviews is
again artifact-neutral.
**CPR-25 was filed next**, making it **122 with 90 delivered**: it adds the
trusted MCP server catalogue and exact-version project bindings under the
stable stateless MCP 2026-07-28 contract.
It was delivered on 2026-08-25, making it **122 with 91 delivered**: immutable
raw/normalised discovery evidence, quarantined schema/source drift, typed
VedaFlow approval and exact project bindings now share one PDP/RLS/audit path;
the gateway neither launches imported stdio commands nor resolves secrets.
**CPR-26 was filed next**, making it **123 with 91 delivered**: it replaces
the console's Tools placeholder with the generated catalogue, immutable
version comparison, common VedaFlow review link, exact project bindings,
discovery-only evidence and secret-safe configuration.
It was delivered the same day, making it **123 with 92 delivered**: the stable
Tools address makes quarantined drift and exact distribution inspectable while
adding no execution proxy, secret resolver or parallel reviewer surface.
**CPR-27 was filed next**, making it **124 with 92 delivered**: it pins the
canonical OKF v0.2 specification and adds one external-format boundary for
bounded import planning into reviewable candidates and deterministic export,
never a second Knowledge domain or publication path.
It was delivered the same day, making it **124 with 93 delivered**: immutable
dry-run evidence, candidate-only materialisation, normalised provenance and
PDP-filtered deterministic export share the existing RLS/VedaFlow/audit path;
the adapter adds no v0.1 fallback, network fetch or execution authority.
**CPR-28 was filed next**, making it **125 with 93 delivered**: it adds the
local-path CLI and generated-contract project console over CPR-27 without
adding another format, publication path or scheduled synchronisation model.
It was delivered the same day, making it **125 with 94 delivered**: local
validation/inspection and atomic export share the pinned adapter, while import
and all governed state changes remain on the public API and existing
CaptureCandidate/VedaFlow path.
**CPR-29 was filed next**, making it **126 with 94 delivered**: it completes
the authenticated `/v1` contract and deletes handwritten or storage-coupled
ordinary clients rather than treating the generated surface as a newer-plane
island.
It was delivered the same day, making it **126 with 95 delivered**: one
catalogue now constructs all 156 authenticated application operations and is
checked exactly against OpenAPI; the console consumes generated operations,
while ordinary service/audit CLI and generic MCP/Claude clients reach product
state only through the public gateway boundary.
**CPR-30 was filed next**, making it **127 with 95 delivered**: it replaces
mutable policy assignment and ad-hoc runtime settings with immutable governed
configuration versions and nearest-scope bindings under ADR-0089.
It was delivered the same day, making it **127 with 96 delivered**: canonical
personal/team/enterprise documents now select policy, capture, context,
freshness, Skill/Tool advertisement and providers through immutable versions;
every binding/version change enters typed VedaFlow, and the mutable assignment
tables and routes are gone.
**CPR-31 was filed next**, making it **128 with 96 delivered**: it audits the
one-path auto-apply invariant and replaces the pre-cut lapse plane with
immutable, exact-subject, time-boxed `Policy/apply` relaxations.
It was delivered the same day, making it **128 with 97 delivered**: the old
row/effect/config plane is deleted without translation; stable aggregates and
immutable versions retain exact subject/scope/permission/tier/window,
approvers and Configuration evidence; Cedar and database time decide the
window, while open collaboration auto-applies only by completing the same
typed VedaFlow change standard retains for review or rejection.
**CPR-32 was filed next**, making it **129 with 97 delivered**: it extends the
one VedaFlow review across every context-platform artifact family with typed
aggregate/version references, configurable author/reviewer/effect-actor
separation, commit-preconditioned verdicts and one comprehensive Advanced
Reviews lifecycle.
It was delivered the same day, making it **129 with 98 delivered**: every
proposal now carries immutable typed artifact addresses, both verdicts bind
the inspected commit, stricter profiles separate author, reviewer and effect
actor, and generated Advanced Reviews completes that one common lifecycle.

Since CPR-9 a **listing decides per row**. The audit of Prompts 1–7 found that
`GET /v1/workspaces` and `/v1/me` took one decision at the tenant root and
applied it to every row, so a caller granted `member` at a workspace — who
holds nothing at the root — was served an empty list and an
`onboarding.state` of `needs_workspace`, while the `anchors` block of the same
response said `workspace.read: true` at that workspace. Listings now decide
about the row, under the row's own chain and pack, with **no fast path** for a
caller permitted at the root (a forbid overrides a permit at any depth). Two
CLI surfaces that CPR-7 had silently broken are repaired and pinned from both
sides: `synveda login` required an `identity.quarantined` the server had
deleted — so **every login failed to parse its own session** — and `synveda
whoami --capabilities` read the deleted `roles`/`role_assign` shape. And the
no-data-migrator guard now scans **the whole migration chain** rather than the
epoch file alone, skipping function bodies and pinning the three inherited
pre-epoch statements by name; they are unreachable (a pre-cut database never
reaches the migrator) and deleting them would trade the reset instruction for
a checksum error on every existing database, so the epoch stays at **2** and
Prompt 33's squash removes them.

Since CPR-10 **a run is a record** (ADR-0076). `sessions`, `session_events`
and `session_context_runs` replace `session_id: text` as the answer to "what
has this agent been doing": a run names a workspace and optionally a project,
and the governed scope it is decided at is **derived** from those by two
composite foreign keys and a CHECK, never sent by a client — which is also the
rule for the tenant and the acting principal, and a body naming either is
refused rather than ignored. Five states, because closing is two-phase: an
adapter says `ending` at a hook that must return fast and still has events
buffered, and `ending` still accepts them. Events are immutable, ordered by a
**server-assigned** `sequence`, and idempotent by the client's own
`client_event_id` — so a redelivered batch appends only what is new and
answers `duplicate` for the rest, at their original positions, which is why
this route takes no `Idempotency-Key` while opening a run and composing
context both do. A **timeline is a projection** over two tables and never a
third, merged rather than sorted so a skewed client clock can misplace a
context run and can never reorder a transcript.
`POST /v1/sessions/{id}/context-runs` is the **final public shape** of the
context endpoint. CPR-10 initially called the old retrieval engine and left
`/v1/observe`, `/v1/inject` and `/v1/recall` untouched; CPR-11 did not perform
the forecast cutover. CPR-12 subsequently deleted that parallel runtime, and
CPR-20 replaced the final internal record reader with the explainable current-
Knowledge planner without changing this endpoint's address.

Since CPR-11 that record is **usable** (ADR-0077). `GET /v1/sessions` is
keyset-paginated — `cursor` in, `next_cursor` out, and `truncated` **deleted**
rather than kept beside it — and the cursor follows the **last candidate a
page considered**, not the last row it served, because rows are decided one at
a time after they are scanned: a page may therefore be empty and still carry a
cursor. Four more filters (`client_name`, `principal_id` and a half-open
`started_after`/`started_before`). Every timeline event entry carries
`received_at` beside the client's `occurred_at` and a server-computed
`delayed`, which is **one flag and not three** — a spooled batch, a replay
after a crash and a wrong clock produce the same two instants, so the server
reports the gap and names no cause. A raw payload is its own authority:
`GET /v1/sessions/{id}/events/{event_id}` under **`SessionDiagnostics`**,
strictly narrower than each pack's own `SessionRead` (**packs @19 → @20**), and
the chain records which event was expanded and never what was in it. A close
carries an **`end_reason`** (migration `0045`), which is not `task_summary`.
And a run has an **address**: `/console/sessions/{id}`, reached by one level of
`:param` in the console's route table.

Since CPR-12 **that record is the only one** (ADR-0078). The Claude Code
adapter, the extraction pipeline, the CLI, the MCP server and the eval
harness all move onto the session plane, and `/v1/observe`, `/v1/inject`
and `/v1/recall` are **deleted** with `observe_events`, the `observe`
queue, `observe_quarantine` and `ObserveKind` (migration `0046`). Which
closes the divergence CPR-11 left open in §10 of the implementation
record. `POST /v1/sessions/{id}/events` is the only write seam. Since CPR-18,
a frozen event snapshot is the **capture unit**, so seven of the thirteen
types are `capture_eligible()` and the rest are ordered and auditable but not
candidate input. Extraction creates reviewable candidates only; accepting one
enters VedaFlow before it can become Knowledge. **Shared Knowledge is proposed
at the scope the run was decided at**, not the submitter's home — while a
preference defaults to the submitter's private principal scope.
Delivery is durable: a **versioned local spool**, one file per run, written
temp → `fsync` → `rename`, holding attempt counts and an acknowledgement
state, retried by the next `SessionStart` and never read in its
predecessor's format. Hooks own delivery; the CLI diagnoses
(`synveda session flush | spool status | spool purge --acknowledged`, and
`purge` has no `--all`). The spool hashes with **SHA-256, not BLAKE3**,
because the thing that verifies it is Node. **The event-loss boundary is
documented rather than closed**: a host killed before any lifecycle hook
runs loses the turn since the last `Stop`; nothing that reached the spool
is ever lost. Since CPR-20, `synveda recall` and the `recall` MCP tool call the
ordinary session-scoped Knowledge query rather than composing a context run;
the separately authorised evaluation lens supplies exact query, enumeration
and revision-id reads. The extraction, security and QA-index suites therefore
have the correct replacement seam, but Prompt 30 owns reproducible reseeding,
remeasurement and baseline changes; `make ci`'s `eval-check` remains parse-only
and green until then.

CPR-14 makes the missing join executable without blurring its evidence tiers
(ADR-0079). `make claude-acceptance` replays genuine, versioned Claude Code
2.1.220/2.1.241 frames through the built hook child, public session routes,
epoch-2 Postgres, the PDP, ingestion, timeline and verifying audit chain. It
also takes the gateway away with two entries pending, restores it, deliberately
loses one acknowledgement and proves the overlap answers `duplicate` at the
original sequence before appending the new event exactly once. Fixture bytes
are schema-validated, provenance-bound and hashed. `make
claude-acceptance-live` is separate: it packages and installs through the
supported marketplace path, asks Claude Code itself for the plugin/hook/MCP
state and invokes real `claude -p`. On 2026-08-24 the authenticated installed
**2.1.241** client passed: plugin **0.2.0** was enabled with four hooks and one
MCP server, SessionStart composed one context run, real user/Read/result/
assistant activity persisted as four ordered events, and SessionEnd closed the
same run. Stop is synchronous only through its 8ms atomic local-spool write;
the 28ms append happened at SessionEnd. The remaining loss boundary is a host
killed before any lifecycle hook receives the in-flight turn.

**CPR-12 also found that 43 of the 65 scripts under `demos/` do not run**,
and have not since CPR-7 deleted `synveda role bind` and
`synveda hierarchy` three prompts earlier — four prompts recorded clean
runs in between, because no gate runs a demo. Filed as **CPR-13**, whose
larger half is `make check-demos`. **Do not trust a demo under `demos/`
to run** unless it is one of the 22 CPR-13 lists as current.

Phase 3 was reordered on 2026-08-04 by demo-readiness (see the Sequencing
note in SYNVEDA_FEATURES.md): the demo block leads — OPS-1, CNSL-1, ADPT-2,
CNSL-2, AUTH-4,5, EVAL-3, OPS-2 — and TEN-3..6, AUD-3,4 and the rest follow.

The product is installable since OPS-1: `synveda init` — see docs/INSTALL.md.
Since OPS-8 it is installable **by somebody else**: `curl … install.sh | sh`
puts a release's CLI, gateway binary, console and compose profile on a
machine whose only prerequisite is Docker (ADR-0065). It ships binaries as
well as images because ADR-0055 decision 8 forecloses the Docker-only shape
— the bundled issuer is a `localhost` URL and RFC 6761 makes that the
container itself — so the default install runs the gateway on the host.
`init` now finds its profile by explicit > checkout > installed bundle, so a
contributor's tree still wins. The release also carries the **Claude Code
plugin** as a marketplace — `synveda plugin install` drives `claude plugin`
rather than editing its state — which found that the plugin had **never
loaded** in Claude Code: `plugin.json` declared `hooks` (a duplicate load)
and an inline `mcpServers` (silently ignored), and `~/.claude/plugins/<name>/`
is not a path Claude Code reads. ADPT-1's demo could not see any of it
because it is its own harness (ADR-0027 amendment). The installer itself
touches nothing under `~/.claude` or any client's config. Not signed, not
notarized, no Windows, no upgrade path: reinstalling is how you upgrade, and
the release workflow is the one thing that can break without a red build to
say so.
Since AUTH-4 it syncs a directory: `/scim/v2` for Entra and Okta, with
`synveda scim token issue` for the credential (ADR-0059). Nothing has
replayed a frame from a live Entra or Okta tenant yet — the vendor corpus is
transcribed from their published tables.
Since TEN-4 it has a key plane: `SYNVEDA_KMS_KEY` (mint one with `synveda kms
keygen`) wraps a data key per tenant plus one for the deployment, and the
console session tokens, the outbound directory credential and `synveda tenant
export` are sealed under them (ADR-0064). Unset means `Kms::Disabled` — `/v1`
serves exactly as before and only the surfaces needing a key refuse. Two
things this does **not** do: `records`/`record_embeddings`/the Tantivy
sidecars are not sealed (there is no BM25 or HNSW over ciphertext — that is
the volume's job, decision 7), so destroying a tenant's key is not erasure;
and the KEK lives in deployment configuration, so this defends a dumped table
and a stolen archive rather than an operator who can read the environment.

Since CNSL-1 it has a browser: the gateway serves the admin console from its
own origin at `/console/`, which needs `pnpm --filter @synveda/console build`.
A missing bundle is not a boot failure — the route 404s and the rest of the
product runs, because a static asset must not be a dependency of the audit
log (crates/synveda-gateway/src/console.rs).
Since CPR-8 that browser is a **product shell** (ADR-0075): a route table with
a **primary** menu shown to everybody (Home, Sessions, Knowledge, New
Learnings, Skills, Tools, People, Settings) and an **advanced** menu shown only
where the caller's capability forecast offers the plane (Reviews, Scopes,
Policies, Audit, Service identities) — the proposals inbox and the scope
explorer re-homed there, unchanged in substance. Beside it: workspace and
project switchers over a selection persisted per browser and reconciled
against `/v1/me`; one query/cache layer and therefore one loading and one
error state per route; a typed client over the generated contract; a
**People** page that answers *why* somebody may act here; and **first-run
onboarding** — workspace, project, repository, agent client, connection
instructions, connection check. The personal/team question **seeds** a policy
pack and a membership posture and records **no edition anywhere** (ADR-0068
decision 1). No npm dependency was added: routing and the cache are written
in-repo. One primary page has no plane yet (Tools) and says so — Sessions got
one at CPR-10/11, Knowledge at CPR-17 and New Learnings at CPR-19; seven surfaces
still call hand-written paths in `console/src/api.mts`
until Prompt 19 puts the rest of `/v1` on the contract.

Phase demo goal: Entra/Okta live, spec-compliant governed skills into Claude
Code + Cursor, LongMemEval scores published, Helm install.
(LongMemEval scores ARE published since 2026-08-09 — docs/BENCHMARKS.md, QA
0.300 and retrieval recall 0.357 over 10 of 500 instances, with the corpus
digest, both model versions and the commit in the row. Ten instances is a
first data point rather than a benchmark claim; `make eval-longmemeval-full`
is the run somebody schedules.)
(Read "LoCoMo/LongMemEval" until 2026-08-07, when EVAL-3/ADR-0061 found
LoCoMo's corpus is CC BY-NC 4.0 — a licence that withholds exactly the
published commercial claim this goal names. The second benchmark is EVAL-7.)
(ADPT-2 recorded its acceptance corpus from Claude Desktop and **Zed** —
Cursor stays an `install` target because this goal names it, but nothing has
replayed a real Cursor frame. See ADR-0057 amendment 2.)
(Helm install IS done since 2026-08-10 — `deploy/helm/synveda`, installed
into a kind cluster by `demos/ops-2-helm-install.sh`, which asserts a
governed round trip, a CloudNativePG failover and a live RLS backstop
rather than readiness. Two limits belong beside the claim: the gateway is
**one replica** and the chart refuses to render a second until OPS-7, and
the test's issuer is the bundled Rauthy at a Service DNS name — so this
profile has not met a live Entra or Okta tenant either. It is also the
first thing that ever asked the gateway *image* to serve, which closes
ADR-0055 decision 8's open residue.)

## Commands
- make dev-up      — start Postgres(+pgvector+PGMQ), Rauthy, Temporal, TEI, Jaeger
- make smoke       — end-to-end health check
- make dev-down    — stop; state persists in named volumes
- make ci          — exactly what .github/workflows/ci.yml runs; green here == green there
- make db-test     — the full suite against DATABASE_URL (tests needing Postgres skip without it)
- make eval        — the eval harness against a live stack, gated by evals/baseline.json
- make eval-check  — parse suite + corpora + baseline with no stack (part of `make ci`)
- make eval-retrieval / eval-security / eval-extraction-live — the nightly and
  live-model gates, each with its own baseline

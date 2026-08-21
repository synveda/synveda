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
docs/implementation/synveda-context-platform.md. **It is a pre-1.0 hard
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
The document covers the context-platform plane — `/v1/me`,
workspaces/projects/repositories, the access plane, and the six admin
scope routes (32 operations) — and says so in its own description; the
rest of `/v1` joins it at Prompt 19. Since CPR-8 the generator also emits
the **runtime** path/method table beside the type table and marks the eight
operations whose document requires an `Idempotency-Key`, so the console's
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
features done. 70 of 103 features delivered — see docs/backlog/STATUS.md for
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
beside the new model — and **CPR-8 on 2026-08-21**, making it **105 with 72
delivered**: the console product shell, the first prompt of this programme
whose deliverable is a screen, and the one that makes six prompts of platform
reachable by somebody who is not holding a terminal. Prompts 9–33 of its
programme are filed by the prompts that run them, so this number will keep
moving.)

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
in-repo. Four primary pages have no plane yet (Sessions, Knowledge, New
Learnings, Tools) and say so; seven surfaces still call hand-written paths in
`console/src/api.mts` until Prompt 19 puts the rest of `/v1` on the
contract.

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

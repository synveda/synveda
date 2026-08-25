# Synveda context-platform redesign — implementation record

The programme that re-cuts Synveda from an enterprise memory platform whose
smallest unit is an organisation into a context platform an individual can
use, without producing a second product. Feature **CPR-1**; decisions locked
in **ADR-0068**.

This document is the programme's running record. It is written once at the
baseline (Prompt 1) and appended to by every prompt after it: what was
implemented, what changed in the schema and the API, what was deleted, what
was tested, and the commit hash. It is required reading before any prompt in
this programme.

---

## Autonomous continuation queue

This journal was opened on **2026-08-24** for the autonomous continuation
that began at `6eb3e3bdf01d035c79caca3ccc3e0b0d1cdee4ff`. The remote
`origin/feat/context-platform-mvp` branch and the local branch were identical
and the worktree was clean after a fresh fetch and fast-forward-only pull.
CPR-14 was reconstructed from code, tests, durable evidence and both commits:
its Claude Code 2.1.241 run is genuine live-client evidence, not replay.

The objectives below are journal entries, not another specification. A
feature commit's hash is written by the following checkpoint, following the
programme convention established in Prompt 1.

| Objective | Feature | Status | Start SHA | Result SHA | Focused tests | `make ci` | `make db-test` | Live/demo/evaluation evidence | Blockers |
|---|---|---|---|---|---|---|---|---|---|
| Versioned Knowledge aggregate, immutable revisions, normalised provenance and current projection | CPR-15 | **complete** | `6eb3e3b` | `874aa51` | types 5/5; store DB 5/5; RLS completeness PASS | PASS | PASS | isolated `demos/cpr-15-knowledge-aggregate.sh` PASS | none |
| Governed create/edit/verify/supersede/merge/archive/restore/forget and durable erasure | CPR-16 | **complete** | `874aa51` | `f2a7c5c` | gateway lifecycle 3/3; policy approvals 6/6, packs 7/7, PDP 11/11; RLS completeness PASS | PASS | PASS | isolated `demos/cpr-16-knowledge-lifecycle.sh` PASS: 19 governed changes, zero old records | none |
| Public Knowledge API, lexical/semantic search, generated-client browser and raw-record product cutover | CPR-17 | **complete** | `f2a7c5c` | `2d845b0` | gateway public API 1/1; OpenAPI 5/5; console 151/151; RLS 84/84 | PASS | PASS | isolated `demos/cpr-17-knowledge-browser.sh` PASS: one Knowledge item, zero old records | none |
| Session-based capture batches, reviewable candidates and governed acceptance actions | CPR-18 | **complete** | `2d845b0` | `e778a60` | gateway 3/3; Claude lifecycle 2/2; ingest 64/64; OpenAPI 5/5; console 151/151; RLS 84/84 | PASS | PASS | isolated `demos/cpr-18-session-capture.sh` PASS: 8 candidates, 8 governed changes, zero old records/queue | none |
| New Learnings lightweight candidate review and scope-safe governed decisions | CPR-19 | **complete** | `e778a60` | `e90dac9` | console pure 8/8; component acceptance 6/6; complete console 165/165; production build PASS | PASS | N/A — console-only | real-component server-rendered acceptance covers evidence, comparisons, all actions, denial and applied/pending outcomes | none |
| Explainable Knowledge context planning, trace retention, feedback and scoped query/evaluation lenses | CPR-20 | **complete** | `e90dac9` | `8ed8aa6` | context 3/3; audit 13/13; packs 10/10; sessions 22/22; OpenAPI 5/5; console 165/165; RLS guards PASS | PASS | PASS | isolated `demos/cpr-20-context-planning.sh` PASS: 55 Knowledge, 47 plans, 75 selections, 2 feedback, zero records | none |
| Linkable Context Inspector, retention-aware evidence and exact revision outcome feedback | CPR-21 | **complete** | `8ed8aa6` | `8cdd1ee` | console helpers 7/7; component 6/6; complete console 179/179; context 3/3; sessions 22/22; production build PASS | PASS | PASS | in-app browser unavailable; real-component SSR covers full/redacted/hashes-only/disabled/refusal and the production bundle builds | none |
| Core personal/team PulseBoard loop across sessions, capture, governed Knowledge, privacy, supersession and inspector evidence | CPR-22 | **complete** | `8cdd1ee` | `c9e647d` | consolidated DB acceptance 1/1; capture 4/4; context 3/3; console 179/179 | PASS | PASS | isolated `demos/cpr-22-mvp-acceptance.sh` PASS: 3 sessions, 5 candidates, 4 changes, 3 current + 1 superseded Knowledge, 2 runs, 3 selections, zero records | none |
| Re-point the executable demo corpus and gate CLI/OpenAPI drift | CPR-13 | **complete** | `c9e647d` | `9b8ad04` | checker fixtures 4/4; shell syntax PASS; generated inventory 73/73 | PASS | N/A — no persisted behaviour changed | MEM sessions 22/22 + load 1/1; CTX 1/1; FLOW 4/4; AUTHZ 2/2; ADPT authentic-frame 2/2 | none |
| Immutable Agent Skills versions, project/principal bindings, evidence-labelled usage and controlled test runs | CPR-23 | **complete** | `9b8ad04` | `89b5f79` | gateway 1/1; RLS/immutability 1/1 + completeness 1/1; OpenAPI 5/5; policy packs 7/7; CLI 157/157; console 179/179 | PASS | PASS (`synveda_test_80706`) | official unversioned Agent Skills spec pinned to upstream `69ef37e`; isolated `demos/cpr-23-versioned-skills.sh` PASS | none |
| Generated-API Skills Library, bindings, exact files/tests/usage and legacy Skill review-screen cutover | CPR-24 | **complete** | `89b5f79` | `07ce9f3` | helpers/components 10/10; shared review 5/5; console 186/186; CLI 151/151; production build PASS | PASS | N/A — console/client-only | no in-app browser exposed; real-component SSR and production bundle PASS | none |
| Trusted MCP server catalogue, immutable versions/snapshots, exact project bindings, generated configuration and read-only tests | CPR-25 | **complete** | `07ce9f3` | `9845186` | types 5/5; gateway unit 3/3 + public DB 1/1; policy PASS; RLS 1/1; OpenAPI 5/5; console 186/186 | PASS | PASS (`synveda_test_88082`) | official stable MCP 2026-07-28 pinned to `5f5440b`; isolated `demos/cpr-25-tool-registry.sh` PASS; deterministic report is not live-server evidence | none |
| Generated-API MCP Tools catalogue, immutable evidence comparison, VedaFlow review linkage, exact bindings and secret-safe configuration | CPR-26 | **complete** | `9845186` | `98f5bcd` | helpers/components 10/10; complete console 196/196; production build PASS | PASS | N/A — console/client-only | no in-app browser exposed; real-component SSR and production bundle PASS | none |
| Versioned OKF v0.2 validation, import planning/candidates and deterministic Knowledge export | CPR-27 | **complete** | `98f5bcd` | `0dbf163` | adapter 6/6; types 1/1; store 1/1; gateway 1/1; capture 4/4; OpenAPI 5/5; RLS 1/1; console 197/197 | PASS | PASS (`synveda_test_1177`) | canonical v0.2 pinned to `ad30107`; isolated `demos/cpr-27-okf-v02.sh` PASS; no remote fetch/live-host claim | none |
| Public-API OKF CLI and generated project-console import/export workflows | CPR-28 | **complete** | `0dbf163` | `683a17d` | adapter 6/6; CLI 150/150; console 207/207; public API 1/1; production build PASS | PASS | N/A — client/pure-validation only | isolated `demos/cpr-28-okf-workflows.sh` PASS: real local fixture + public lifecycle + generated console; no remote-host claim | none |
| Exact generated public contract and console/CLI/generic-MCP client convergence | CPR-29 | **complete** | `683a17d` | `b33ba51` | OpenAPI 6/6; service 5/5; audit 13/13; CLI 156/156 + corpus 5/5; MCP 44/44; console 208/208; Claude adapter 98/98 | PASS | PASS | isolated `demos/cpr-29-public-contract.sh` PASS; generated API + 78-script demo gate PASS | none |
| Versioned governed runtime configuration, templates and scope bindings | CPR-30 | **complete** | `b33ba51` | `ed7d233` | domain 4/4; API 1/1; capture 4/4; context 3/3; approvals 6/6; packs 7/7; PDP 11/11; RLS 83/83; OpenAPI 6/6; console 210/210 | PASS | PASS | isolated `demos/cpr-30-governed-configuration.sh` PASS: 2 artifacts, 3 versions, 2 bindings, 6 audited applies, zero assignment tables; 79-script demo gate PASS | none |
| Governed auto-apply audit and versioned exact-subject policy-relaxation successor | CPR-31 | **complete** | `ed7d233` | `9281951` | types 210/210 + serde 50/50; policy relaxation 3/3; API 2/2; RLS 83/83; OpenAPI 6/6; audit 27/27; CLI 155/155 + MCP 5/5; console 209/209; retrieval 53/53 | PASS | PASS (`synveda_test_35856`) | isolated `demos/cpr-31-governed-relaxations.sh` PASS: 2 aggregates, 3 immutable versions, 5 governed changes, zero predecessor tables; 79-script demo gate PASS | none |
| One typed VedaFlow approval lifecycle across Knowledge, Skills, Tools, Configuration, relaxations and OKF publication | CPR-32 | **complete** | `9281951` | `cf52f34` | types 212/212 + serde 50/50; policy 77/77; VedaFlow 73/73 + store 10/10; gateway family suites 27/27; OpenAPI 6/6; console 210/210; store policy packs 5/5 + RLS 83/83 | PASS | PASS (`synveda_test_43866`) | isolated `demos/cpr-32-unified-approvals.sh` PASS: 81 typed proposals, 7 families, 23 exact-commit reviews, regulated three-person separation, zero audited content; 80-script demo gate PASS | none |
| Policy-authorised context-platform audit questions and deterministic offline-verifiable export | CPR-33 | **complete** | `cf52f34` | `3c61e5e` | audit 23/23 + tamper 7/7; gateway 16/16; terminal refs 5/5; CLI audit 4/4; OpenAPI 6/6; console 212/212; RLS completeness PASS | PASS | PASS (`synveda_test_51591`) | isolated `demos/cpr-33-audit-export.sh` PASS: 7 self-audited export reads, 49 typed artifact events, one payload index; 81-script demo gate PASS | none |
| Directory push/pull convergence on shared principals, Groups, memberships and grants | CPR-34 | **complete** | `3c61e5e` | next checkpoint | connectors 5/5; store access 30/30 + anchors 13/13 + sync 8/8; gateway access 18/18 + sync 9/9 + SCIM 10/10 + anchors 9/9; OpenAPI 6/6; console 212/212; RLS 83/83 | PASS | PASS (fresh disposable database; removed on success) | isolated `demos/cpr-34-directory-convergence.sh` PASS: 3 shared directory groups, 6 chained transitions, identity-keyed membership, zero mirror tables; 82-script demo gate PASS; Entra/Okta fixtures remain captured/transcribed | no live Entra/Okta tenant available; no live claim made |

**Exact next objective:** file and complete CPR-35 from the CPR-34 checkpoint:
re-anchor tenant envelope keys and secret references on schema epoch 2 and the
new Knowledge, Tool, provider, import/export, directory and deployment artifact
families; prove rotation, stale-reference, cross-tenant and serialization/log
boundaries without adding customer-managed keys or an HSM claim.

### Starting-point objective map

- **Delivered and retained:** the one governed scope tree; workspace/project
  subtypes; groups, grants and invitations; per-object PDP decisions; the
  session ledger; the session-event write seam; durable Claude spool; public
  session/context endpoint; and genuine CPR-14 live acceptance.
- **Deleted and still absent:** the fixed hierarchy, old role bindings and
  global `/v1/observe`, `/v1/inject` and `/v1/recall` routes.
- **Present only as replaced implementation:** `records`, record embeddings,
  record supersession, record-centred retrieval, quarantine/proposal product
  surfaces and the extraction commit path. CPR-15 does not read, translate or
  dual-write them; the lifecycle and public Knowledge packages complete their
  controlled cutover.
- **Absent at this SHA:** Knowledge aggregates, capture batches/candidates,
  New Learnings, explainable context planning, a true scoped recall/query
  lens, versioned skill bindings, a trusted MCP catalogue, OKF, governed
  configuration artifacts and the later convergence packages.
- **Truthful external limitations:** no live Entra/Okta tenant and no authentic
  Cursor lifecycle replay. Three evaluation paths deliberately refuse to
  report until a real Knowledge query lens replaces the deleted sweep; a
  budgeted context run is not used as an enumeration substitute.

---

## 1. Baseline

| | |
|---|---|
| **Base commit** | `92ffa890ee330eb31bce71d5fba08624dcd88a22` (`main`, "Merge pull request #47 from synveda/feat/OPS-9-v0.2.0") |
| **Branch** | `feat/context-platform-mvp`, cut from that commit |
| **Workspace version** | `0.2.0` |
| **Date** | 2026-08-17 |
| **Rust toolchain** | `rust-toolchain.toml` |
| **Features delivered** | 64 of 97 at the base commit; 65 of 98 with CPR-1 (`docs/backlog/STATUS.md`) |
| **Phase** | 3 (Enterprise) paused mid-phase; this programme is Phase 5 |

### 1.1 CI status at the base commit

`make ci` is the repository's full gate: `fmt lint test build deny
check-deps check-backlog check-adr-status check-corpus-licences
check-chart-images check-benchmarks check-ann-bench chart-lint eval-check
ts-build check-npm-licences ts-test`.

**Result: green, and `make db-test` green beside it.** Both runs, what they
do and do not cover, and the `--ignored` suites neither of them runs, are in
§8.1. **No pre-existing failures.**

### 1.2 Migration head

`crates/synveda-store/migrations/` holds **38** migrations, embedded at
compile time by `sqlx::migrate!()` into `synveda_store::MIGRATOR`
(`crates/synveda-store/src/lib.rs:62`). The head is:

```
0038_envelope_keys.sql
```

They create **49 tables** and **2 views** (`records_versions`,
`graph_edges_versions`). There is no down-migration, no reset guard, and no
epoch marker: a database at any prefix of this sequence is accepted and
brought forward.

---

## 2. Public HTTP route inventory (base commit)

From `crates/synveda-gateway/src/app.rs::router`, plus the two merged
sub-routers. Three planes: unauthenticated ops/auth, the tenant-resolved
`/v1` plane, and `/scim/v2` which authenticates with a provisioning
credential instead of a bearer.

### 2.1 Ops and auth plane (no tenant middleware)

| Method | Path |
|---|---|
| GET | `/healthz` |
| GET | `/readyz` |
| GET | `/metrics` |
| GET | `/auth/login` |
| GET | `/auth/callback` |
| POST | `/auth/cli/exchange` |
| POST | `/auth/refresh` |
| POST | `/auth/console/logout` |
| GET | `/console/*` (static bundle; absent bundle ⇒ the route is not mounted at all) |

### 2.2 `/v1` plane (bearer + `tenant::resolve_tenant` middleware)

| Method | Path | Plane |
|---|---|---|
| GET | `/v1/whoami` | identity |
| POST | `/v1/hierarchy/nodes` | hierarchy admin |
| GET | `/v1/hierarchy/root` | hierarchy admin |
| GET/PATCH/DELETE | `/v1/hierarchy/nodes/{id}` | hierarchy admin |
| GET | `/v1/hierarchy/nodes/{id}/children` | hierarchy admin |
| GET | `/v1/hierarchy/nodes/{id}/ancestors` | hierarchy admin |
| GET | `/v1/hierarchy/nodes/{id}/descendants` | hierarchy admin |
| GET | `/v1/hierarchy/nodes/{id}/capabilities` | capability probe |
| GET | `/v1/capabilities` | capability probe (batch) |
| GET | `/v1/policy/packs` | policy admin |
| GET/PUT/DELETE | `/v1/policy/default` | policy admin |
| GET/PUT/DELETE | `/v1/hierarchy/nodes/{id}/policy` | policy admin |
| GET/PUT/DELETE | `/v1/roles/bindings` | role admin (tenant-wide) |
| GET/PUT/DELETE | `/v1/hierarchy/nodes/{id}/roles` | role admin (node) |
| GET/PUT | `/v1/hierarchy/nodes/{id}/curators` | CODEOWNERS-style approvers |
| POST | `/v1/observe` | **primitive** (raised body limit) |
| POST | `/v1/inject` | **primitive** |
| POST | `/v1/recall` | **primitive** |
| GET | `/v1/quarantine` | redaction review |
| POST | `/v1/quarantine/{event_id}/release` | redaction review |
| POST | `/v1/quarantine/{event_id}/reject` | redaction review |
| GET | `/v1/audit/events` | audit query |
| GET | `/v1/audit/disclosures` | audit query |
| GET | `/v1/audit/knowledge` | audit query |
| GET | `/v1/audit/verify` | audit query |
| GET | `/v1/channels/{scope_id}` | VedaFlow channels |
| POST | `/v1/channels/{scope_id}/publish` | VedaFlow channels |
| GET | `/v1/channels/{scope_id}/history` | VedaFlow channels |
| POST | `/v1/channels/{scope_id}/rollback` | VedaFlow channels |
| POST | `/v1/channels/{scope_id}/pin` | VedaFlow channels |
| POST | `/v1/channels/{scope_id}/unpin` | VedaFlow channels |
| GET/POST | `/v1/proposals` | VedaFlow proposals |
| GET | `/v1/proposals/{id}` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/approve` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/reject` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/withdraw` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/checklist` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/quality-override` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/publish` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/classify` | VedaFlow proposals |
| POST | `/v1/proposals/{id}/lapse` | lapse effect |
| GET/POST | `/v1/lapses` | lapse plane |
| POST | `/v1/lapses/{id}/revoke` | lapse plane |
| GET/POST | `/v1/prompts` | prompt registry |
| GET | `/v1/prompts/{*name}` | prompt registry (wildcard, after the collection route) |
| GET/POST | `/v1/context-packs` | context-pack registry (**no** resolve-by-name route) |
| GET/POST | `/v1/skills` | skills registry |
| GET | `/v1/skills/{name}` | skills registry |
| GET/POST | `/v1/service-identities` | service identities |
| GET/DELETE | `/v1/service-identities/{id}` | service identities |
| GET/POST | `/v1/scim/credentials` | directory credentials |
| POST | `/v1/scim/credentials/{id}/revoke` | directory credentials |
| GET | `/v1/directory/sync` | directory pull sync |
| POST | `/v1/directory/seal-authorisations` | directory pull sync |

### 2.3 `/scim/v2` plane (provisioning credential, `require_credential`)

`GET /ServiceProviderConfig`, `GET /ResourceTypes`, `GET /Schemas`,
`GET|POST /Users`, `GET|PUT|PATCH|DELETE /Users/{id}`,
`GET|POST /Groups`, `GET|PUT|PATCH|DELETE /Groups/{id}`.

### 2.4 Contract status

**There is no OpenAPI document.** Every request and response DTO is
hand-written per handler in `crates/synveda-gateway/src/*.rs`; the console
hand-writes a second copy of the subset it consumes in
`console/src/api.mts`. ADPT-3 ("REST/gRPC API + OpenAPI") is filed, in Phase
3, and unstarted. The target invariant — *the OpenAPI contract is
authoritative and frontend types are generated from it* — is therefore new
work, not a repair.

---

## 3. CLI command inventory (base commit)

`crates/synveda-cli/src/main.rs`, 24 top-level commands. Two families, and
the split matters to the redesign: **gateway clients** hold a bearer written
by `synveda login` and call `/v1`, so the PDP decides and the gateway
chains; **store-level plumbing** opens `DATABASE_URL` directly and is
documented as dev bootstrap or break-glass.

| Command | Subcommands | Kind |
|---|---|---|
| `init` | — | installer (migrations, tenant admission, issuer, stack) |
| `login` | — | browser OIDC, writes a profile |
| `whoami` | — | gateway |
| `recall` | — | gateway |
| `hierarchy` | `create list show root policy roles capabilities` | gateway |
| `proposal` | `list show review approve reject withdraw publish override-quality checklist classify` | gateway |
| `channel` | `status history rollback pin unpin` | gateway |
| `prompt` | `list show author propose` | gateway |
| `context-pack` | `list author propose` | gateway |
| `skill` | `list show import install available sync propose` | gateway (+ `install` writes a client's disk) |
| `lapse` | `list` | gateway |
| `scim` | `token {issue list revoke}` | gateway |
| `directory` | `status set-credential clear-credential authorise-seals` | gateway |
| `auth` | `token logout` | local credential store |
| `mcp` | *(bare = the server)*, `install uninstall` | adapter |
| `plugin` | `install uninstall` | adapter (drives `claude plugin`) |
| `db` | `migrate` | store |
| `kms` | `keygen` | local |
| `tenant` | `create key{provision rotate status} export export-open export-describe` | store |
| `token` | `issue` | store (dev HS256) |
| `policy` | `apply clear` | store |
| `role` | `bind unbind list` | store (break-glass) |
| `service` | `register remove list` | store (break-glass) |
| `audit` | `verify tail` | store (read-only) |

---

## 4. Console route and navigation inventory (base commit)

There is **no router and no navigation**. `console/src/main.tsx` mounts one
`App`, served from the gateway's own origin under `/console/`; the whole
console is a single page whose sections appear conditionally on the outcome
of `GET /v1/whoami`:

| Section | Component | Reads |
|---|---|---|
| Signed-in header + sign-out | `App.tsx` | `/v1/whoami`, `POST /auth/console/logout` |
| Sign-in prompt (unauthenticated) | `App.tsx` | link to `/auth/login` |
| Proposals inbox (the hero screen) | `Inbox.tsx` → `Review.tsx` | `/v1/proposals`, `/v1/proposals/{id}`, approve/reject |
  *(CPR-8: `Inbox.tsx` is now `Reviews.tsx`, mounted at Advanced ▸ Reviews.)*
| Hierarchy & policy explorer | `Explorer.tsx` | `/v1/hierarchy/root`, `…/children`, `…/policy`, `…/roles`, `…/capabilities`, `/v1/lapses` |
  *(CPR-7 re-cut it onto `/v1/admin/scopes`; CPR-8 renamed it `Scopes.tsx` and mounted it at Advanced ▸ Scopes.)*

Supporting modules: `api.mts` (fetch + outcome classification + the
hand-written response types), `review.mts`, `explorer.mts`, `diff.mts`,
`text.mts`. Tests: `api.test.mts`, `review.test.tsx`, `explorer.test.mts`,
`explorer.parity.test.tsx`, `diff.test.mts`, `text.test.mts`, plus the
gateway-side parity corpus in
`crates/synveda-gateway/tests/console_parity.rs`.

A missing bundle is not a boot failure: `console_routes()` mounts nothing and
`/console/` 404s (`crates/synveda-gateway/src/app.rs:184`).

---

## 5. Domain model at the base commit

### 5.1 Entities (`crates/synveda-types`)

| Concept | Type(s) | Vocabulary |
|---|---|---|
| Tenant | `Tenant`, `TenantStatus` | `active` \| `suspended` |
| Scope | `HierarchyNode`, `ScopeKind` | **`org` \| `division` \| `department` \| `team` \| `user`**, ranked 0–4 |
| Identity | `Identity`, `IdentityKind`, `IdentityStatus` | human / service; provisioned / quarantined / departed |
| Role | `Role`, `RoleBinding` | `viewer contributor curator steward org-admin auditor security-reviewer compliance` |
| Memory record | `RecordKind`, `RecordClass` | `derived` \| `pinned`; `fact decision preference procedure entity episode` |
| Sensitivity | `Sensitivity`, `ScopeTier` | `public internal confidential restricted` |
| Channel | `Channel` | `derived` \| `staged` \| `published` |
| Asset | `AssetKind` | memory / prompt / context-pack / skill / policy |
| Proposal | `ProposalState`, `ProposalView`, `ProposalEffect`, `Verdict` | open/rejected/withdrawn/published (+ rendered `approved`); effects `published` \| `lapse` \| `classify` |
| Approval | `ApprovalMatrix`, `ApprovalRule`, `RoleRequirement`, `Outstanding` | resolved from (asset × sensitivity × scope × pack) |
| Observe | `ObserveKind`, `QuarantineState` | — |
| Composition | `CompositionConfig`, `InjectChannels`, `IndexTier`, `EntryTier`, `SkillIndex` | — |
| Lapse | `Lapse`, `LapseAction`, `LapseTerms`, `LapseConfig`, `LapseOutcome` | closed action vocabulary (`memory.read`) |
| Registries | `PromptTemplate`/`PromptName`, `PackDocument`/`DocumentChunk`, `SkillBundle`/`SkillFile`/`Frontmatter` | each with its own `*Channel` |
| Policy | `PolicyAssignment`, `PackConfig` | packs `regulated-strict` \| `standard` \| `open-collaboration` |
| Graph | `Graph`, `Depth` | named graphs |
| Other | `DedupConfig`, `RetentionConfig`, `PromotionRule`, `RedactionConfig`, `MoverConfig`, `SkillScanConfig`, `SkillQualityConfig`, `DirectoryUser`/`DirectoryGroup`/`ScimCredential` | — |

**A session is not in this table**, and that is the finding rather than an
omission of this document: `session_id` is a `text` column on
`observe_events`, an `Option<String>` on `InjectBody` and `RecallBody`, and a
correlation field on audit events. Nothing owns it.

### 5.2 Tenant-bound tables and RLS

**49 tables. 46 carry `tenant_id` with `ENABLE` + `FORCE ROW LEVEL SECURITY`
and a `*_tenant_isolation` policy** keyed on the session GUC set by
`synveda_store::rls::begin_tenant_tx` (ADR-0009):

```
audit_chain_heads          audit_log                  context_pack_chunks
context_pack_documents     context_packs              directory_sync_state
graph_edges                graph_edges_history        graph_vertices
group_mappings             hierarchy_closure          hierarchy_nodes
identities                 memory_usage               observe_events
observe_quarantine         policy_lapses              policy_pack_assignments
policy_pack_defaults       policy_packs               promotion_watermarks
prompts                    record_embeddings          record_signatures
record_supersessions       records                    records_history
role_bindings              scim_credentials           scim_group_members
scim_groups                scim_users                 skill_files
skill_quality_overrides    skill_reviews              skills
tenant_keys                tenant_secrets             vedaflow_commit_parents
vedaflow_commits           vedaflow_objects           vedaflow_proposal_approvals
vedaflow_proposals         vedaflow_refs              vedaflow_tree_entries
vedaflow_trees
```

The **three without RLS**, each deliberately:

- `tenants` — the registry of the boundary itself.
- `console_sessions` — has no `tenant_id`, and migration 0034's header is the
  argument for why it must not (the session is resolved *before* a tenant is).
- `deployment_keys` — one key plane per deployment, not per tenant
  (ADR-0064).

`vedaflow_refs` carries a second policy, `vedaflow_refs_only_pins_are_deletable`
(migration 0021).

The adversarial suite is `crates/synveda-store/tests/rls.rs`, which every
feature adding a table extends.

### 5.3 Cedar / PDP entity and action model

Schema: `crates/synveda-policy/src/synveda.cedarschema`. Base layer:
`base.cedar`. Packs: `crates/synveda-policy/src/packs/{regulated-strict,
standard, open-collaboration}.cedar`. The facade is
`synveda_policy::authorize`, in-process (ADR-0002).

**Entities (3):**

- `Tenant`
- `Principal in [Tenant, Scope]` — `tenant`, `quarantined`, `home?`,
  `department?`, `token_scope?`
- `Scope in [Tenant, Scope]` — `tenant`, `kind`, `sealed`

**Actions (34)**, with their wire names:

```
hierarchy.create      hierarchy.read        hierarchy.update    hierarchy.delete
memory.read           memory.write          memory.classify
prompt.read           prompt.write
context_pack.read     context_pack.write
skill.read            skill.write           skill.quality.override
quarantine.read       quarantine.review
policy.read           policy.assign
role.read             role.assign
service_identity.read service_identity.manage
audit.read
directory.manage      directory.seal.authorise
channel.read          channel.publish       channel.rollback    channel.pin
proposal.read         proposal.open         proposal.review
lapse.grant           lapse.revoke
```

Roles reach decisions as **request context** (`context.roles`), never as
entity attributes. Five actions carry context beyond `roles`: `MemoryRead`
(`lapsed` **and** `sensitivity`), `PromptRead`, `ContextPackRead` and
`SkillRead` (`sensitivity`), and `RoleAssign` (`grant`). Every one of those
attributes is **required rather than optional**, deliberately: Cedar drops a
policy that errors on a missing attribute, so an optional one would make a
base-layer forbid silently stop existing. `AuditRead`, `DirectoryManage` and
`DirectorySealAuthorise` apply to `Tenant` only — deliberately, so a
subtree-scoped answer is unrepresentable rather than merely unimplemented.

`Principal.department` and `Scope.kind` are the two places where the fixed
organisational rank vocabulary reaches the PDP; ADR-0068 decision 4 removes
both.

---

## 6. Existing paths and subsystems

### 6.1 observe → extract → embed → commit

`POST /v1/observe` (`observe.rs`) — batch of `{idempotency_key, kind,
payload, occurred_at}` under a `session_id` string. All-or-nothing
validation; `MemoryWrite` decided at the caller's home scope; the MEM-2
redaction/secret scan runs inline and may **deny**, **redact** or
**quarantine** each event; accepted events land in `observe_events` and PGMQ.
Response is per-event outcomes. Then, asynchronously: extraction into
`records` (classified into `RecordClass`), dedup and supersession
(`record_signatures`, `record_supersessions`), graph-link inside the write
transaction, embed-or-fail into `record_embeddings`, and a commit onto the
scope's `memory/derived` channel. ADR-0020, 0021, 0022, 0023, 0024, 0039,
0044.

### 6.2 inject

`POST /v1/inject` (`inject.rs`) — `{task?, session_id?, budget_tokens?,
max_sensitivity?}`. Composition plan (a PDP sweep per candidate scope **and
sensitivity tier**), hybrid retrieval (pgvector + Tantivy, RRF), budgeted
assembly by specificity gradient with pinned-first, tiering into `body` and
`index` entries, a BLAKE3 watermark, and exactly one chained
`context.injected` event with the per-scope decisions aggregated. Degrades to
sparse-only with an `X-Synveda-Degraded` header. ADR-0025, 0026, 0038, 0040,
0041, 0054.

### 6.3 recall

`POST /v1/recall` (`recall.rs`) — three modes: `ids` (the handles an index
entry printed), `query` (hybrid retrieval across every scope the caller's
policy admits, wider than inject composes from), and `sweep` (neither, with
an instant). Bitemporal `as_of` / `valid_at`. Entries carry `scope_id`,
`channel`, `kind`, `class`, `sensitivity`, full content and provenance. The
plan is re-decided per call — a handle is a name, not a capability. ADR-0041,
0042.

### 6.4 Hierarchy and role bindings

`hierarchy_nodes` is the adjacency ground truth plus a materialised `path`
(display only, never an authorisation input); `hierarchy_closure` holds every
`(ancestor, descendant, distance)` pair including distance-0 self-rows and is
what ancestor/descendant queries scan. Closure maintenance is explicit store
code inside the caller's transaction, no triggers (ADR-0011). Constraints
that encode the rank vocabulary: `hierarchy_nodes_kind_check` (five values),
`hierarchy_nodes_root_is_org_check`, one-root-per-tenant, and the
child-outranks-parent rule enforced in the store because it needs the parent
row.

`role_bindings` are strictly additive; `scope_id IS NULL` binds tenant-wide.
Effective roles are resolved by the PDP from binding rows and passed as
`context.roles`. `ScopeChainCache` (HIER-2) caches the chain and is
invalidated in-process — which is one of the two reasons OPS-7 exists.

### 6.5 Records, proposals, quarantine, skills, context packs

- **Records.** `records` + `records_history` + the `records_versions` view:
  bitemporal (`tx_from/tx_to`, `valid_from/valid_to`), tenant-bound,
  `kind ∈ {derived, pinned}`, `class`, `sensitivity`, provenance quadruple,
  embedding row in `record_embeddings` written in the same transaction.
- **Channels.** `vedaflow_refs` rows named `{asset-kind}/{channel}` per
  scope, over a content-addressed BLAKE3 object store (`vedaflow_objects`,
  `_trees`, `_tree_entries`, `_commits`, `_commit_parents`). `published`
  membership is a commit's tree; `derived` membership is the complement.
  **`staged` is never written.**
- **Proposals.** `vedaflow_proposals` + `_approvals`. Four stored states, a
  fifth rendered (`approved`). Three effects: `published`, `lapse`,
  `classify`. Required approvals resolve from the pack's approval matrix
  (asset × sensitivity × target scope), with an invariant floor no pack can
  reach below.
- **Quarantine.** `observe_quarantine` — pending / released / rejected, a
  one-shot column-bound review, adjudicated by `security-reviewer` under
  `QuarantineReview`.
- **Skills.** `skills` + `skill_files` (agentskills.io bundle, immutable
  content addresses recomputed by the client), `skill_reviews` (checklists,
  digest-keyed so an edit invalidates them) and `skill_quality_overrides`;
  the scan and quality thresholds ride the effective pack as its `scan` and
  `quality` JSONB columns, and the cached score rides `skills.quality_score`
  + `rubric_version`. `critical` scan findings are on the invariant floor and
  no pack or role can wave them through.
- **Context packs.** `context_packs` + `context_pack_documents` +
  `context_pack_chunks`. Server-side chunking, secret scan and embedding;
  content reaches a session through `/v1/inject` as ranked pinned material,
  which is why there is no resolve-by-name route.
- **Prompts.** `prompts`, resolved by name along the caller's placement chain
  nearest-first, or from a named scope, or from a pinned commit.

### 6.6 Client adapters, and their actual verification level

This is the part of the baseline most worth being exact about, because the
repository's own record is exact about it.

| Adapter | What it is | Verified how | Verification level |
|---|---|---|---|
| Claude Code plugin (`adapters/claude-code`) | TS hooks (`SessionStart`, `Stop`, `PreCompact`, `SessionEnd`) + a marketplace manifest; its MCP entry launches `synveda mcp` | `demos/adpt-1-claude-code.sh` end to end against a live gateway; recorded-payload driver over 16 fixture cases; unit suites | **Live, with a known hole.** ADPT-8: a *headless* run (`claude -p`) injects and never observes — three sessions, three `inject.ok`, zero `observe.done`, exit 0. Only `session-start` is synchronous. Interactive sessions observe correctly (measured `events=5 accepted=5`). |
| Generic MCP server (`synveda mcp`) | Rust CLI subcommand, stdio JSON-RPC, tools `recall` and `remember`; a `/v1` client holding the login bearer | `crates/synveda-cli/tests/mcp_corpus.rs` against a **recorded** corpus | **Recorded frames from Claude Desktop and Zed.** Cursor is an `install` target because the phase goal names it; **no real Cursor frame has ever been replayed** (ADR-0057 amendment 2). |
| SCIM `/scim/v2` | Entra and Okta provisioning | `crates/synveda-gateway/tests/scim.rs` against a **transcribed** vendor corpus | **No live tenant.** The corpus is transcribed from Entra's and Okta's published attribute tables; nothing has replayed a frame from a live tenant. |
| Directory pull sync | Scheduled fallback for tenants without SCIM push | `crates/synveda-identity/tests/directory_connectors.rs` | Same — connector-level, no live tenant. |
| TypeScript SDK (`sdks/typescript`) | One `index.ts` | — | Placeholder. |
| Rust SDK, Python SDK | READMEs only | — | Placeholder; ADPT-4. |

The one structural note: `synveda mcp` lives in a binary that also links
`synveda-store`, `synveda-identity`, `synveda-policy` and `synveda-audit`
for its dev-bootstrap commands, so "adapters use public APIs only" is held
there by a **test that fails on any reference to a core crate**
(`crates/synveda-cli/src/mcp.rs`) rather than by the crate graph. The target
invariant is the same rule; the programme should decide whether it stays a
test or becomes a boundary.

---

## 7. Deletion map — old concepts to target concepts

Every row is a hard cut. Nothing in the left column is preserved behind an
alias, a serde rename, a fallback read or a compatibility view; nothing in
the right column is populated by translating the left.

| # | Old concept (base commit) | Target concept | Notes |
|---|---|---|---|
| 1 | `ScopeKind {org, division, department, team, user}`, `rank()`, `hierarchy_nodes_kind_check`, `hierarchy_nodes_root_is_org_check`, child-outranks-parent | **Generic governed scope**: a named node with a parent and a subtree, no rank vocabulary | ADR-0068 decision 4. `Principal.department` and `Scope.kind` leave the Cedar schema with it. |
| 2 | Tenant ⇒ organisation; org root minted from the tenant slug on first login | Tenant remains the isolation boundary; the root scope is a scope like any other | The identity between "tenant" and "organisation" is what makes an individual model themselves as a company. |
| 3 | Policy packs `regulated-strict` / `standard` / `open-collaboration` | **Policy profiles** `personal` / `team` / `enterprise` | ADR-0068 decision 2. Same mechanism (assigned, versioned, subtree-inherited, invariant floor); a different vocabulary and different defaults. |
| 4 | SMB compose profile vs enterprise Helm profile as deployment *editions* | One runtime; deployment shape is configuration | ADR-0068 decisions 1 and 2. No edition conditionals anywhere. |
| 5 | `records` + `records_history` + `records_versions`, `RecordKind {derived, pinned}` | **Candidates** (session-produced, unreviewed) and **knowledge versions** (published, immutable) — two tables | ADR-0068 decision 6. The trust boundary becomes structural. |
| 6 | `Channel {derived, staged, published}` and per-scope `vedaflow_refs` for memory | Candidate/knowledge separation carries the boundary; publication mints a knowledge version | `staged` is deleted outright: nothing ever wrote it. |
| 7 | `observe_events` + `session_id: text` as a correlation string | **Session** as a first-class tenant-bound aggregate; events, candidates, recalls and injections hang off it | ADR-0068 decision 5. |
| 8 | `observe_quarantine` as its own review plane | Candidate lifecycle state on the session/candidate path | The scan itself (rules, modes, never-log-matched-text) survives unchanged. |
| 9 | `prompts`, `context_packs` (+documents, +chunks), `skills` (+files) as three registries with three `*Channel` enums | One immutable **versioned-artifact** family: knowledge, skill and tool versions with a stable aggregate id and an immutable revision | ADR-0068 decision 7. |
| 10 | `skill_reviews`, `skill_quality_overrides`, and the pack's `scan`/`quality` JSONB columns, all keyed to the skills registry | The same gates, expressed over artifact versions | The `critical`-is-un-overridable rule is invariant and survives verbatim. |
| 11 | `record_signatures`, `record_supersessions`, `memory_usage`, `promotion_watermarks` | Re-derived on the candidate → knowledge path | No data carries over. |
| 12 | `graph_vertices`, `graph_edges`, `graph_edges_history` | Re-derived over knowledge versions | Named-graph partitioning (ADR-0043) survives as a design. |
| 13 | `policy_lapses` | Time-boxed relaxation expressed as a governed VedaFlow change against a profile | Personal policy may *auto-apply* such a change; it may not bypass one. |
| 14 | `scim_users`, `scim_groups`, `scim_group_members`, `scim_credentials`, `directory_sync_state` | Enterprise-profile **directory adapter** | Not a core domain model. |
| 15 | Hand-written DTOs per handler + a hand-written second copy in `console/src/api.mts` | **OpenAPI is authoritative**; frontend types are generated from it | The one place two hand-written copies of one contract exist today. |
| 16 | MCP tool surface embedded in the CLI binary; Claude Code plugin with its own hook contract | **External-format adapters** over the public application API only | ADR-0068 decision 8. |
| 17 | *(no equivalent)* | **OKF** import/export adapter | New. The format is named by the programme and specified by a later prompt; ADR-0068 fixes only its position. |
| 18 | 38 migrations, no epoch marker, no reset guard | One `0001` epoch plus a **startup guard that rejects an old database with a reset instruction** | ADR-0068 decision 3. |

**Carried forward unchanged in kind** (re-anchored on the new nouns, not
deleted): tenancy and forced RLS; the Cedar in-process PDP and the
`authorize()` facade; the hash-chained audit log; VedaFlow's content-
addressed object store, proposals and approval matrix; hybrid retrieval and
the composition contract; the redaction/secret-scan rules; the envelope key
plane (ADR-0064); OIDC login, JIT provisioning and service identities.

---

## 8. Test suite at the base commit

### 8.1 Run record

Run on macOS/darwin 25.5.0, on `feat/context-platform-mvp`. **CPR-1's diff
is documentation only** — not one line of Rust, TypeScript, SQL or
configuration changed — so the code-exercising targets ran against
base-commit code byte for byte, and this is therefore the base commit's CI
status as well as the branch's.

**`make ci` — PASS (exit 0).** The full gate, every target green: `fmt`,
`lint` (clippy `-D warnings`), `test` (`cargo test --workspace`), `build`,
`deny`, `check-deps`, `check-backlog`, `check-adr-status`,
`check-corpus-licences`, `check-chart-images`, `check-benchmarks`,
`check-ann-bench`, `chart-lint`, `eval-check`, `ts-build`,
`check-npm-licences`, `ts-test` (74 adapter tests + the console suite).

**`make db-test` — PASS (exit 0).** The workspace suite against a scratch
database of its own (`scripts/db-test.sh`: create, extensions, migrate,
run, drop). This is not redundant with `make ci`: without `DATABASE_URL` the
Postgres-dependent tests **skip**, so `cargo test --workspace` alone is not
evidence about the store, the RLS suite, or any gateway test that needs a
database. `make ci` is the gate; this is the coverage.

**Pre-existing failures: none.** Both gates are green at the baseline, so
every failure the programme produces from here is its own.

**Not run, and named rather than assumed.** The `--ignored` suites are
opt-in by design and neither gate runs them: live-TEI retrieval quality
(`retrieval_live`), inject latency at 1,000 concurrent sessions
(`inject_latency`), the 1M-record retrieval latency bench (`latency`), the
ANN bench (`ann_bench`), the installed-skill corpora (`skill_corpus`,
`skill_corpus_rubric`), and live-model extraction. Their absence from this
record is not a pass.

**What the two runs report, and the trap in it.** Both report **107 test
binaries, 1,397 passed, 0 failed, 10 ignored** — identical totals. That is
not evidence that `make ci` covered the database: the Postgres-dependent
tests skip *inside the test body* and still count as passed, so
`crates/synveda-store/tests/rls.rs` reports "67 passed" in both runs while
only the `db-test` one actually opened a connection. A green `cargo test
--workspace` with no `DATABASE_URL` therefore looks exactly like a green one
with a database behind it. Worth writing down here because the programme is
about to rewrite every one of those tests, and the number it will be
tempted to compare against is the one that means less.

Numbers are in §8.2.

### 8.2 Measured counts

| | |
|---|---|
| Cargo test binaries | 107 (1,397 passed, 0 failed, 10 ignored) |
| Rust integration test files | 84 (`crates/*/tests/*.rs`), plus in-crate unit tests |
| Console TS test files | 6 (`console/src/*.test.*`) |
| Claude Code adapter TS test files | 10 (`adapters/claude-code/src/*.test.mts`) |
| Demo scripts | 64 (`demos/*.sh`) |
| Migrations | 38 |
| Tables / views | 49 / 2 |
| RLS-forced tables | 46 |
| Cedar actions | 34 |
| Cedar entity types | 3 |
| `/v1` route paths | 54 |
| `/scim/v2` route paths | 7 |
| Ops/auth route paths | 8 (+ the `/console` static mount) |
| CLI top-level commands | 24 |
| ADRs | 67 at the base commit; 68 with ADR-0068 |

---

## 9. The programme — Prompts 1–33

Thirty-three ordered prompts. The ordering below is what this baseline
commits to; it is derived from ADR-0068's eight locked decisions and the
deletion map in §7, in dependency order. **Each prompt's own text is
authoritative when it arrives** — where a prompt diverges from this list, the
divergence is recorded in §10 rather than silently absorbed.

### Stage A — the epoch (Prompts 1–6)

| # | Prompt |
|---|---|
| 1 | **Baseline.** This document, ADR-0068, CPR-1, the branch, the recorded test state. No runtime change. |
| 2 | **Fresh schema epoch.** Delete the 38-migration sequence; new `0001`; the startup guard that rejects an old database with a reset instruction. |
| 3 | **Generic governed scopes.** Scope tree, closure, tenant binding, forced RLS; `ScopeKind` and every rank constraint deleted. |
| 4 | **Identity, membership and role bindings** over generic scopes. |
| 5 | **Policy profiles and the PDP re-cut.** `personal`/`team`/`enterprise`; the Cedar entity and action model over generic scopes. |
| 6 | **Audit chain re-anchored** on the new nouns; a fresh chain for a fresh epoch. |

### Stage B — runtime and knowledge (Prompts 7–12)

| # | Prompt |
|---|---|
| 7 | **Sessions** as the root of agent runtime activity: the aggregate, its lifecycle, its audit. |
| 8 | **Session events** and the observe path onto sessions. |
| 9 | **Candidates** — extraction output, separated from published knowledge. |
| 10 | **Knowledge versions** — stable aggregate id, immutable revision, content addressing. |
| 11 | **Candidate → knowledge promotion** through VedaFlow, with the approval matrix over the new artifact family. |
| 12 | **Redaction and secret scanning** re-anchored on the session/candidate path. |

### Stage C — skills, tools and governed configuration (Prompts 13–15)

| # | Prompt |
|---|---|
| 13 | **Skill versions** — immutable, scanned, quality-gated. |
| 14 | **Tool versions** — the tool registry, immutable, governed. |
| 15 | **Governed configuration as versioned artifacts** — profiles, budgets, injection rules, through VedaFlow. |

### Stage D — retrieval and context (Prompts 16–18)

| # | Prompt |
|---|---|
| 16 | **Embedding and hybrid retrieval** over knowledge and candidates. |
| 17 | **Context assembly** — the read path over scopes and profiles. |
| 18 | **Recall** over the new model. |

### Stage E — contract and console (Prompts 19–20)

| # | Prompt |
|---|---|
| 19 | **OpenAPI as the authoritative contract**; the generator; frontend types generated from it. |
| 20 | **Console on generated types** — sessions, candidates, knowledge, review. **← MVP checkpoint** |

### Stage F — adapters and external formats (Prompts 21–24)

| # | Prompt |
|---|---|
| 21 | **MCP adapter** over the public application API. |
| 22 | **Claude Code adapter** over the public application API. |
| 23 | **OKF import/export adapter.** |
| 24 | **CLI re-cut** onto the public application API. |

### Stage G — governance depth (Prompts 25–28)

| # | Prompt |
|---|---|
| 25 | **Personal policy auto-apply** — auto-applies a VedaFlow change, never bypasses one. |
| 26 | **Relaxations** (the lapse successor) as governed changes on the new model. |
| 27 | **Approval matrix and review surfaces** over the full artifact family. |
| 28 | **Audit query and export** over the new model. |

### Stage H — enterprise and operations (Prompts 29–33)

| # | Prompt |
|---|---|
| 29 | **Directory adapters** (SCIM push, scheduled pull) as enterprise-profile adapters. |
| 30 | **Key plane and secret handling** on the new epoch. |
| 31 | **Deployment as configuration** — install, compose, chart, one runtime. |
| 32 | **Evaluation harness** on the new model; benchmarks re-measured. |
| 33 | **Programme close** — documentation, release, the standing gaps stated. |

### 9.1 The MVP checkpoint (after Prompt 20)

After Prompt 20 the product must stand up on its own, with no adapter and no
enterprise surface. Concretely, at that checkpoint:

1. **One runtime.** A single person and a team run the same binary, the same
   schema and the same decision point, differing only in the policy profile
   assigned to their scopes. No edition conditional exists in the tree.
2. **A fresh epoch, guarded.** A database from before the cut is refused at
   startup with a reset instruction. No migrator, no dual read.
3. **Generic scopes.** A scope has a parent and a subtree and no rank. An
   individual's deployment has whatever scopes they made, including one.
4. **Sessions are real.** Every runtime act is attributable to a session
   aggregate, and a session with no observed events still exists.
5. **Candidates and knowledge are separate tables.** Nothing composes across
   the boundary by accident; publication mints an immutable version.
6. **The governed path holds.** Every read and mutation passes the PDP; every
   knowledge, skill, tool and governed-configuration mutation passes VedaFlow;
   important state changes chain audit events.
7. **Tenancy holds.** Every persisted domain table is tenant-bound with
   forced RLS, tested adversarially.
8. **The contract is authoritative.** The OpenAPI document is generated from
   the server, the console's types are generated from the document, and no
   hand-written second copy of a DTO exists.
9. **It is demonstrable.** A demo script under `demos/` takes a fresh
   deployment from nothing to: a session, observed events, candidates,
   a published knowledge version, a context assembly that cites it, a recall
   that serves it, and a verifying audit chain.

What is **not** in the MVP checkpoint, deliberately: MCP, Claude Code, OKF,
the re-cut CLI, directory sync, the key plane's re-anchoring, the Helm
profile, and the benchmark re-measurement. Those are Prompts 21–33.

---

## 10. Prompt record

One entry per prompt: what was implemented, schema/domain changes, API and
frontend changes, deletions, tests, and the resulting commit hash.

### Prompt 1 — Baseline (CPR-1)

- **Implemented.** Branch `feat/context-platform-mvp` from
  `92ffa890ee330eb31bce71d5fba08624dcd88a22`. This document. ADR-0068 with
  the eight locked decisions. Feature CPR-1 filed in
  `docs/SYNVEDA_FEATURES.md` (new epic CPR, new Sequencing phase),
  `docs/backlog/CPR-1.md` and `docs/backlog/STATUS.md`. CLAUDE.md points at
  this document as required reading.
- **Schema/domain changes.** None. No migration was added, altered or
  removed; no type changed.
- **API and frontend changes.** None. No route, DTO, CLI command or console
  component was touched.
- **Deleted.** Nothing. The deletion map in §7 is a plan; Prompt 2 begins
  executing it.
- **Tests.** No test added — this prompt adds no behaviour to test. The
  existing suite was run to record the baseline: `make ci` PASS, `make
  db-test` PASS, no pre-existing failures (§8.1).
- **Commit.** `chore(programme): baseline context platform redesign` on
  `feat/context-platform-mvp`.
- **Commit hash.** `db01e5e28e13cc61b39a3e3be288504b389f153d`. A commit cannot
  contain its own hash, so this line was written by **Prompt 2**, the first
  entry that could see it — rather than left as a placeholder that would read
  as an oversight. Every later entry records its own hash the same way:
  written by the prompt after it.

### Prompt 2 — Fresh schema epoch, startup guard & local reset (CPR-2)

- **Implemented.** The schema epoch as an enforced fact rather than a stated
  intention (ADR-0069). `schema_metadata` — one row carrying the epoch, the
  migration head, the creation timestamp and the product version that created
  it. `synveda_store::epoch` with four surfaces: `read`, `verify` (the
  guard), `preflight` (refuses to migrate a pre-cut database) and `stamp`
  (writes the marker after a successful migration). `synveda_store::reset`
  with `recreate`, which drops and rebuilds the application database at the
  current epoch. `synveda reset --database --force` on top of it. Guards
  wired at four seams: gateway boot (refuses to start, exits non-zero),
  `/readyz` (503), `synveda_store::migrate` (refuses before the migrator
  runs) and `connect_current_epoch` in the CLI (every store-level command
  except `db migrate` and `reset`).
- **Divergence from §9.** §9's Prompt 2 reads *"Delete the 38-migration
  sequence; new `0001`"*. The prompt as it arrived says the opposite —
  *"Do not squash the full migration chain yet. That happens in the final
  cutover"* — and the prompt's own text is authoritative (§9's preamble).
  Recorded here rather than absorbed, because the substitution is a good one
  and worth having in the record: squashing here would put the epoch marker
  and the whole of the new model in one commit, leaving the guard with **no
  pre-cut database to be tested against**. Keeping the 38 migrations means a
  pre-cut database is a fixture the tests build, refuse, and reset — which is
  the difference between an enforced epoch and an asserted one. The squash
  moves to Prompt 33.
- **Schema/domain changes.** One migration, `0039_schema_epoch.sql`: creates
  `schema_metadata` (`id` boolean single-row PK, `epoch`, `migration_head`,
  `created_at`, `created_by_version`, `updated_at`), read-only for
  `synveda_app`. **50 tables, 2 views; 46 still RLS-forced.**
  `schema_metadata` carries no `tenant_id`, so it is structurally exempt like
  `console_sessions` and `deployment_keys` — and structurally *must* be: the
  guard reads it before a tenant is resolved, so a tenant-keyed policy would
  evaluate false and hide the marker from the check that exists to read it.
  The row is written by Rust, not by the migration: two of its four facts —
  the creating release and the head reached — are only available to the
  running binary. No domain type changed.
- **API and frontend changes.** No route added or removed. `/readyz` gains
  the epoch check and now answers 503 for a database that answers `SELECT 1`
  but is at the wrong epoch. **One CLI command added**, `synveda reset`
  (25 top-level commands). No console change.
- **Deleted.** Nothing yet — this prompt adds the guard the deletions in §7
  will be performed behind. The one thing it *removes* is the ability to
  bring a pre-cut database forward, which was reachable at the base commit by
  running `synveda db migrate`.
- **Tests.** `crates/synveda-store/tests/epoch.rs` (10, each on a scratch
  database of its own): a fresh empty database bootstraps; a current-epoch
  database starts normally and keeps its provenance across a re-migration; a
  pre-cut database is refused **and not touched**; a marker with no row is
  refused; a marker of another shape and one with a blank provenance are both
  refused; an older epoch and a newer epoch are refused differently; reset
  builds a working current database, carries zero rows across and is
  idempotent; reset builds a database that was not there; reset refuses a
  name it will not quote; and `no_old_to_new_data_migrator_exists`, which
  checks the epoch migration statement by statement for DML and the chain for
  `.down.sql`. `crates/synveda-gateway/tests/observability.rs` gains
  `readyz_refuses_a_database_that_is_not_at_this_schema_epoch`. Unit tests in
  `epoch.rs` (every refusal names the reset command; the one that must not),
  `reset.rs` in the store (the identifier grammar, against
  `synveda"; drop database temporal; --`) and `reset.rs` in the CLI (local
  only; neither flag alone; the password is never printed).
  `demos/cpr-2-schema-epoch.sh` drives the **boot** refusal against a real
  gateway binary — the only check no in-process test can reach.
- **Run record.** `make ci` PASS, `make db-test` PASS. The `.sqlx` cache
  gained 5 entries and lost none.
- **Commit.** `feat(schema): enforce fresh schema epoch` on
  `feat/context-platform-mvp`.
- **Commit hash.** `050d798f8825ee3a1cadabeca4d80f3def167665`, written by
  Prompt 3 on Prompt 1's rule.

### Prompt 3 — Generic governed scope substrate (CPR-3)

- **Implemented.** The first piece of the new domain model: `scopes` +
  `scope_closure`, tenant-bound with forced RLS, and the internal application
  services over them — create, rename, move, ancestors, descendants, tenant
  root, children, path and path resolution (`synveda_store::scopes`). The
  domain type is `synveda_types::scope::{Scope, ScopeKind, ScopeStatus}` with
  the slug/name/attributes/path validators beside it. Decisions in **ADR-0070**.
- **The one place this reads ADR-0068 rather than transcribing it.** Decision 4
  says a scope "has no `kind`". This keeps a `kind` and removes the *rank*:
  five shapes (`tenant`, `org_unit`, `workspace`, `project`, `principal`) whose
  only job is deciding which shapes may be a parent. No `rank()`, no
  strictly-increasing ladder, no root-must-be-an-org, and nothing anywhere
  comparing two kinds for order — `org_unit` nests inside itself to arbitrary
  depth, and one person's entire tree is a tenant scope and a principal.
  ADR-0070 decision 1 argues it: an untyped node leaves the placement rule with
  nowhere to live, so "a project inside a principal" becomes representable and
  the shape information moves into `attributes`, where every consumer parses a
  convention and no constraint checks one. That is the rank vocabulary again,
  unenforced. Recorded here rather than absorbed, per §9's preamble.
- **Schema/domain changes.** One migration, `0040_scopes.sql`. **52 tables, 2
  views; 48 RLS-forced** (both new tables carry `ENABLE` + `FORCE`, a
  `*_tenant_isolation` policy and least-privilege grants — no DELETE on
  `scopes`, no UPDATE on `scope_closure`). Where each structural rule lives is
  the feature's substance: the root shape, one-root-per-tenant and sibling-slug
  uniqueness are constraints; the **placement rule** is a row-local CHECK over a
  denormalised `parent_kind` kept honest by a composite foreign key
  `(tenant_id, parent_scope_id, parent_kind) → (tenant_id, id, kind)`, which
  also makes a **cross-tenant edge unrepresentable**; **cycles** are refused by
  `check ((ancestor_id = descendant_id) = (distance = 0))` on the closure — the
  exact row a move's relink would write if its destination were inside the
  subtree; and a `before update` trigger makes `id`, `tenant_id`, `kind`,
  `slug`, `created_at` and `created_by` immutable, which is what extends
  "a scope never moves across tenants" to the **owner** role that forced RLS
  does not bind. No materialised `path` and no `depth` column: both are derived
  from the closure, so a move stops rewriting every descendant's copy and a
  path cannot be stale. `synveda-types` gains `serde_json` as a dependency (the
  `attributes` bag) and exposes `scope` as a module rather than re-exporting it,
  because the old `ScopeKind` still owns the root name until Prompt 6.
- **API and frontend changes.** **None, deliberately.** No route, no CLI
  command, no console screen, no adapter and no PDP call inside the store — the
  governed entry points (a decision before the call, an audit event after it,
  VedaFlow where the change is governed) attach at the API boundary Prompts 5–6
  add, exactly where they attach today for the hierarchy this replaces. One
  metric is described in the gateway's telemetry registry
  (`synveda_scope_mutations_total`), and its series is expected to be *absent*
  rather than zero until a route reaches the services.
- **Deleted.** Nothing. This prompt is the only one in the deletion map's first
  row that adds before it removes: §7 row 1 is executed in two halves, and the
  half that deletes `ScopeKind {org…user}`, `hierarchy_nodes`,
  `hierarchy_closure`, `rank()`, `hierarchy_nodes_kind_check` and
  `hierarchy_nodes_root_is_org_check` is Prompt 6. Nothing synchronises the two
  models in either direction, at any time: no row of `hierarchy_nodes` becomes
  a row of `scopes`, and no code reads one to write the other.
- **Tests.** `crates/synveda-store/tests/scopes.rs` (20): the closure agrees
  with a recomputation from the adjacency after every operation in every test;
  the placement rule asserted as a **matrix over all 25 pairs** of the
  vocabulary rather than as cases; the root rules (a parentless non-tenant
  scope, a second root, a nested tenant scope); sibling slugs; malformed slug,
  display name and attributes; another tenant's scope absent rather than
  forbidden on every surface including `move`'s destination; a cross-tenant
  update refused to the owner role; slug/kind/provenance immutability; cycles
  refused by the service and unrepresentable in the closure; org units nested
  **40 deep** with a workspace and project still hanging off the deepest;
  subtree moves; ineligible moves; rename touching only the display name; path
  round-trips; and three concurrency tests. Plus a **property test** —
  randomly generated create/move/rename histories against a live tree, legal
  and illegal alike, with the closure recomputed and compared after every
  step. `crates/synveda-store/tests/rls.rs` gains the scope block (4) and both
  tables join the completeness inventory: 67 → 71 tests. Unit tests in
  `synveda-types` (10) and `synveda-store` (1).
- **What one test does not prove, stated rather than implied.**
  `a_create_inside_a_moving_subtree_waits_for_the_move` still passes with
  `move_scope`'s subtree lock removed — the relink's foreign keys already take
  a share lock on every subtree member, which blocks the create for a reason
  nobody designed. The lock stays (the rule should be *a move owns its subtree*,
  not *a foreign key happens to*), the narrower window where the incidental
  lock is not enough ends in a spurious conflict for the move rather than in
  corruption, and both the module doc and the test say so.
- **Run record.** On the final tree, against a live Postgres:
  `crates/synveda-store/tests/scopes.rs` **20/20**, `tests/rls.rs` **71/71**
  (67 before this feature), `tests/hierarchy.rs` green and unchanged,
  `synveda-types` unit tests **160/160**. `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` clean. The node gates
  green: `check-backlog`, `check-adr-status`, `check-crate-deps`,
  `check-corpus-licences`, `check-chart-images`, `check-benchmarks`,
  `check-ann-bench`. The `.sqlx` cache gained 28 entries and lost none.
- **The two full gates did not complete on the machine this was written on, and
  the cause is the machine.** Bitdefender's on-access scanner runs `codesign
  -vv -R=notarized --check-notarization` against every freshly built binary:
  **measured at ~51 seconds per test binary, on an idle scanner**, which is
  ~1.5 hours of pure overhead across the workspace's ~107 test binaries, and it
  wedged individual suites (`cedar_entity_sync`, `mcp_corpus`) in `_dyld_start`
  for 20+ minutes at a stretch. A `make db-test` run of this feature's code
  reached 29 suites with **zero failures** before being stopped. Recorded here
  rather than papered over: `make ci` and `make db-test` are this repository's
  gates, they have not been seen green on this commit, and the next machine to
  run them should exclude `target/` from on-access scanning first. Nothing in
  this feature is `#[ignore]`d, excluded or weakened to get a green.
- **Commit.** `feat(scopes): add governed scope substrate` on
  `feat/context-platform-mvp`.
- **Commit hash.** `9ff9631c83dfb68a87a38187263c86174f4eaf89`, written by
  Prompt 4 on Prompt 1's rule.

### Prompt 4 — Workspaces, projects & canonical repository identity (CPR-4)

- **Implemented.** The programme's **first public surface** (ADR-0071).
  `workspaces`, `projects` and `project_repositories` as product-level subtypes
  of a governed scope — each owning one scope of the matching shape, created in
  the same transaction as itself — plus `idempotency_records`, twelve `/v1`
  routes, six Cedar actions, six audit action types, and the product's first
  **OpenAPI contract**, generated from the handlers with the console's
  TypeScript generated from it in turn.
- **Divergence from §9.** §9's Prompt 4 reads *"Identity, membership and role
  bindings over generic scopes"*. The prompt as it arrived is workspaces and
  projects, and the prompt's own text is authoritative (§9's preamble).
  Recorded rather than absorbed, because the substitution moves a dependency:
  identity/membership was ordered before the PDP re-cut (Prompt 5) so that
  bindings would exist over generic scopes when the packs were re-anchored, and
  this prompt instead lands a surface that **needs** the PDP re-cut and does not
  have it. The consequence is stated in the decisions below rather than hidden:
  every decision on this plane is anchored at `Resource::Tenant`, and Prompt 5
  now has two things to carry rather than one. What the substitution buys is a
  product somebody can use at all — Prompt 3 built a scope substrate with no
  API, and a second prompt with no API would have left three consecutive
  prompts of infrastructure.
- **Schema/domain changes.** One migration, `0041_workspaces_projects.sql`.
  **56 tables, 2 views; 52 RLS-forced** (all four new tables carry `ENABLE` +
  `FORCE`, a `*_tenant_isolation` policy and least-privilege grants — no DELETE
  on `workspaces` or `projects`, DELETE on `project_repositories` because
  detaching is the API's own verb, no UPDATE on `idempotency_records`). Domain
  types: `synveda_types::workspace::{Workspace, Project, LifecycleStatus}`,
  `synveda_types::repository::{ProjectRepository, RepositoryProvider,
  RepositoryIdentity, identify}`, and `WorkspaceId` / `ProjectId` /
  `RepositoryId`. `synveda_store` gains `workspaces`, `projects`,
  `repositories` and `idempotency`; `scopes` gains `ensure_tenant_root` and
  `set_status`.

  Where each rule lives is again the substance, and it is ADR-0070 decision 2
  applied one level up: **the subtype's scope shape, the project's scope sitting
  under its workspace's, and the fact that a subtype's slug *is* its scope's
  slug are all foreign keys**, over three denormalised columns (`scope_kind`,
  `workspace_scope_id`, and `slug` itself). The third is the one nobody would
  think to make structural, and it is the one that matters most: a product path
  and a scope path cannot diverge. Revisions step forward by exactly one, and a
  project never changes workspace, both by trigger — so both hold for the owner
  role, which is what migrations and break-glass psql run as.
- **API and frontend changes.** **Twelve routes added** — `GET /v1/me`, the
  workspace and project CRUD, and repository attach/list/detach (54 → 66 `/v1`
  route paths). Creation takes a **required** `Idempotency-Key`; update takes a
  **required** `expected_revision`. No CLI command and no console screen: the
  console shell is Prompt 20, and the generated types typecheck without a screen
  consuming them. `docs/api/openapi.json` and `console/src/generated/api.ts` are
  new, both generated, both checked.
- **Deleted.** Nothing. This prompt adds the first surface over the substrate
  Prompt 3 built; the deletion map's rows are executed by Prompts 5, 6 and after.
  Nothing is synchronised with the old hierarchy in either direction, and a
  test asserts that a tenant which has used this plane has **zero**
  `hierarchy_nodes` rows.
- **Tests.** `crates/synveda-store/tests/workspaces.rs` (21): the scopes the
  model claims; the tenant root minted once from the `tenants` row with no
  `hierarchy_nodes` row to have come from; **a failed creation leaving neither
  an orphan subtype nor an orphan scope**, for both subtypes, through the
  failure mode that fires *after* the scope insert; the structural rules against
  direct SQL (a workspace owning a project-shaped scope, a slug disagreeing with
  its scope's, a rewound or skipped revision, a project moved between
  workspaces, a project's scope moved out from under its workspace); revision
  preconditions from both ends; another tenant's subtype absent rather than
  conflicting; archive/restore mirrored onto the scope; the three-case
  description; and the repository properties — one repository written four ways
  is one attachment, a path refused before it reaches a row *and* refused by the
  CHECK when the service is bypassed, a fingerprint identity, a handle scoped to
  its project, two projects about one repository.
  `crates/synveda-gateway/tests/workspaces_api.rs` (23): the whole path from
  nothing to a project with a repository with `/v1/me` narrating it; the
  idempotency guarantees including the reordered body and the per-subject key;
  **the replay that still takes the PDP decision**, with the binding revoked
  between the two calls; the precondition; every route denied without its action
  and refused without a credential; the credential swept out of the response and
  the chain; and the absent delete verb.
  `crates/synveda-gateway/tests/openapi.rs` (5), which needs no database: the
  committed document is the tree's, the document declares exactly this plane,
  every documented path is mounted (401 rather than 404), a path this plane does
  not declare is not mounted, and the document is generatable (unique operation
  ids, resolvable refs, a declared security scheme, a taxonomy body on every
  4xx). `crates/synveda-store/tests/rls.rs` gains the CPR-4 block (5) and all
  four tables join the completeness inventory: 71 → 76 tests. Unit tests in
  `synveda-types` (repository canonicalisation, 16), the gateway's idempotency
  seam and DTOs, and both store modules. `demos/cpr-4-workspaces.sh` drives the
  whole thing against a real gateway and a real database.
- **Run record.** *(written by Prompt 5, on Prompt 1's rule.)* Re-run on Prompt
  5's machine against a live Postgres, on CPR-4's code as committed:
  `crates/synveda-store/tests/workspaces.rs` **21/21**,
  `crates/synveda-gateway/tests/workspaces_api.rs` **23/23**,
  `crates/synveda-gateway/tests/openapi.rs` **5/5**, `tests/rls.rs` **76/76**.
  `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D
  warnings` clean; `check-api-types` and `check-backlog` green.
- **Commit.** `feat(workspaces): add workspaces, projects and repository
  identity` on `feat/context-platform-mvp`. (The prompt's proposed message named
  "workspace and project model"; the committed one names the repository plane
  too, and this record says the committed one.)
- **Commit hash.** `165e54ae732beea5b01cd73989e3e3afc23b6595`, written by
  Prompt 5 on Prompt 1's rule. Two follow-ups belong beside it, because the
  branch has them and a record that omitted them would not reconstruct:
  `9e443f3555ff00b095fea5393783702955bef9cc` (untracking the `.sqlx` offline
  cache) and `f5248ee209aa82b6f70d6821cbc219e92b3a6225` (re-tracking it) — the
  cache is required for a build without a database, so it is checked in.

### Prompt 5 — Membership, groups, grants & invitations (CPR-5)

- **Implemented.** The membership model (ADR-0072): `groups`, `group_members`,
  `scope_grants` and `pending_invites`, the resolution that turns them into "who
  may act here", fourteen `/v1` routes, four Cedar actions, seven audit action
  types, and the `owner` grant that creating a workspace or a project now mints
  for its creator in the creating transaction. CPR-4 shipped workspaces that
  **nobody was in**; this is who is in them.
- **Divergence from §9.** §9's Prompt 5 reads *"Policy profiles and the PDP
  re-cut"*. The prompt as it arrived is membership and access assignment — which
  is §9's Prompt 4, displaced when Prompt 4 turned out to be workspaces — and
  the prompt's own text is authoritative (§9's preamble). So the programme has
  now swapped two entries and is one behind on the PDP re-cut rather than two
  prompts off its plan: the ordering is 4=workspaces, 5=membership, and the PDP
  re-cut is the next one. Recorded rather than absorbed, because the same
  dependency has now been deferred twice and the second deferral is what makes
  it a pattern: decision 3 below is CPR-4's tenant-anchoring debt plus a new
  one, and both are owed to the same prompt.
- **The decision this prompt is mostly about is one it refused to make.** There
  is **no permission table**, and ADR-0072 decision 2 forbids one arriving.
  Six role keys — `owner`, `member`, `viewer`, `reviewer`, `curator`,
  `administrator` — and nothing in the schema or in `synveda-types` says what any
  of them may do. Every product with roles eventually grows a
  `role_permissions` table because it is inspectable and looks like
  configuration; it is refused here because it is a **second decision point**,
  and seed §2.2 permits one. The cost is decision 3.
- **Schema/domain changes.** One migration, `0042_access.sql`. **60 tables, 2
  views; 56 RLS-forced** (all four new tables carry `ENABLE` + `FORCE`, a
  `*_tenant_isolation` policy and least-privilege grants — no DELETE on `groups`,
  no UPDATE on `scope_grants`, UPDATE on `pending_invites` narrowed by a trigger
  to exactly two transitions). Domain types:
  `synveda_types::access::{RoleKey, SubjectKind, GrantSubject, GrantSource,
  GroupSource, Group, GroupMember, ScopeGrant, PendingInvite, InviteStatus,
  inherits_into}` plus `GroupId` / `GrantId` / `InviteId`.
  `synveda_identity::invite` mints and hashes the token;
  `synveda_store::access` is the services and the resolution.

  Where each rule lives is again the substance. A grant has exactly one subject,
  is **never edited** (a trigger that refuses every update, beside a grant that
  withholds `UPDATE` — the same rule said twice, deliberately: the grant is what
  the app role cannot do, the trigger is what nobody can), and only an
  `invite`-sourced one may name an invitation. An invitation is one-time because
  `pending` is the only status anything may leave and it may only be left once —
  a trigger, not a `SELECT … FOR UPDATE` in one function. And **expiry is a
  property of the decision rather than of a job** (ADR-0037 decision 4): there is
  no stored `expired` status, so an invitation stops working at the instant it
  says it will whether or not any sweep has run.
- **Inheritance is the scope tree, and nothing is materialised.** `members_of`
  walks `scope_closure` upward, so a workspace grant is in force at every project
  inside it — resolved at read time, with **zero rows written at the project**,
  which a test asserts directly. The one place the walk stops is a
  `principal`-shaped scope, which is somebody's own: no ancestor reaches in, and
  the rule is in the resolution SQL and in
  `synveda_types::access::inherits_into` rather than at each caller.
- **A principal is a token subject, and the reason is this programme's law.**
  ADR-0015 decision 2 already argued the general case (a grant may precede first
  login). The case that decided it is narrower: an `identities` row in this tree
  still requires a `hierarchy_nodes` node, because `identities_scope_fk` points
  there — so a membership model keyed on identities would have needed the model
  it replaces, in every test, every demo and every deployment. That is a
  synchronisation between the two models, which ADR-0068 decision 3 forbids
  outright. The cost is that a member list carries subjects rather than names,
  and it is not paid down here: joining `identities` for a display name would be
  reaching into the old model for cosmetics.
- **API and frontend changes.** **Fourteen operations added across ten paths**
  (66 → 76 `/v1` route paths; the OpenAPI document 12 → 26 operations across 17
  paths). Creation takes a required `Idempotency-Key` and the group update a
  required `expected_revision`, unchanged from CPR-4 — with **two deliberate
  exceptions on the invitation path, each a decision**: a replayed invitation
  creation is a **409 saying the token cannot be re-served** rather than a 200
  with the field missing, because the original response carried something that no
  longer exists; and redeeming takes no `Idempotency-Key`, because a one-time
  token already is one (a retry by the principal who redeemed it replays with
  200, anybody else is a 409). `docs/api/openapi.json` and
  `console/src/generated/api.ts` regenerated. No CLI command and no console
  screen: Prompts 24 and 20.
- **One route puts a secret in a URL path, and the mitigation is in the tree.**
  `POST /v1/invites/{invite_token}/accept` is the prompt's route shape. A trace is
  an ordinary log, so `crate::app::make_request_span` records the matched *route
  pattern* rather than the URI for that route, from an explicit `SECRET_IN_PATH`
  list rather than a heuristic — a heuristic that decides what looks like a
  secret will one day decide wrong in the permissive direction.
- **Deleted.** Nothing. The old `role_bindings` plane is untouched and **nothing
  is translated into a grant or dual-written**: no row of `role_bindings` becomes
  a `scope_grants` row, and no code reads one to write the other. The two
  vocabularies deliberately overlap on two words (`viewer`, `curator`) and mean
  different things, and a unit test says so.
- **Tests.** `crates/synveda-store/tests/access.rs` (**30**): the inheritance
  properties (a workspace grant in force at the project with **no row written**
  there, a project grant reaching neither its workspace nor a sibling, nearest
  first); principal-private isolation against a tenant-root grant — the widest
  thing the model can express — with a direct grant at the private scope still
  applying; group resolution following membership with no grant written, and an
  archived or empty group resolving to nobody; the structural rules **against
  direct SQL** (one subject, never edited, the invite shape, group slug/source/
  provenance immutability, a revision that cannot be rewound or skipped, a
  terminal invitation that cannot be reopened, terms that cannot be re-pointed);
  invitation one-time-ness with the same-principal replay and the second-person
  refusal; expiry with **nothing having run**; the redemption of an invitation
  for access already held consuming the invitation rather than erroring; an
  unknown token and a foreign one producing **byte-identical** refusals; and
  every read tenant-filtered. `crates/synveda-gateway/tests/access_api.rs`
  (**17**): the owner grant on creation; the whole invitation path with the
  token appearing once; **a sweep of the entire audit chain for the invitation
  secret**; the group grant and its `via_group` attribution; the idempotency
  guarantees including the invitation-creation 409; the stale precondition
  writing nothing; the inherited-member removal refused with the scope named;
  every route denied without its action and refused without a credential, as a
  sweep over all thirteen PDP-gated operations; the replay that still takes the
  decision, with the binding revoked between the two calls; and cross-tenant
  404s. `crates/synveda-policy/tests/access.rs` (**7**): the four actions across
  all three packs, the membership-read gradient the packs differ on, the
  quarantine floor, and a confined service identity refused all four.
  `crates/synveda-store/tests/rls.rs` gains the CPR-5 block (**5**) and all four
  tables join the completeness inventory: 76 → 81 tests.
  `crates/synveda-gateway/tests/openapi.rs` extended to 26 operations and 17
  paths, including three sibling paths this plane deliberately does **not**
  mount. Unit tests in `synveda-types` (access vocabularies, validators, the
  isolation predicate over the whole shape vocabulary — 188 total, up from 174),
  `synveda-identity` (5, including a refusal that must not echo the presented
  token), `synveda-store` and the gateway's views.
  `demos/cpr-5-access.sh` drives the whole thing against a real gateway and a
  real database with two people's tokens.
- **A pre-existing failure this prompt found, and fixed.**
  `crates/synveda-gateway/tests/explorer.rs::the_explorer_parity_corpus_is_what_the_gateway_serves`
  was **failing on the branch before this prompt touched it**. CNSL-2 records
  the gateway's capability answers into `console/fixtures/explorer/*.json` so
  the console's renderer is tested against what the server actually serves;
  CPR-4 added six actions to `Action::PROBED_AT_SCOPE` and did not re-record
  them, and CPR-4's own gates never completed on the machine it was written on
  (§ Prompt 4), so nothing said so. Re-recorded here with
  `SYNVEDA_RECORD_FIXTURES=1`: the fixture gained CPR-4's six *and* CPR-5's two,
  which is what a diff that nobody looked at for a prompt looks like. Worth
  writing down because the cause is the same one CPR-3 recorded — a gate that
  does not finish is a gate that reports nothing — and this is the first time it
  has cost the programme a real regression rather than only a missing green.
- **A pre-existing failure this prompt did not cause and fixed anyway.**
  `cargo deny check advisories` was red on the base tree: **RUSTSEC-2026-0258**,
  `h2` unbounded empty DATA frames, patched in 0.4.16 against a lockfile pinned
  at 0.4.15. An advisory published against a pinned dependency arrives from
  outside a diff, so this is `cargo update -p h2 --precise 0.4.16` — one line of
  `Cargo.lock` plus the `windows-sys` entries cargo re-normalised on the way
  (Windows-only, and this workspace does not target Windows). `cargo deny check`
  is green after it.
- **A defect in this prompt's own test harness, which only a full-workspace run
  shows.** `crates/synveda-gateway/tests/access_api.rs` first built a pool per
  test — a 2-connection bootstrap pool *and* a 4-connection application pool,
  both held for the test's life. Alone that is fine; in `cargo test --workspace`
  it exhausted Postgres's `max_connections` and eight tests failed with
  `PoolTimedOut` at the line that opens the bootstrap pool. The bootstrap pool
  is now **one** connection and is **closed** before the test body runs, and the
  application pool is two. Recorded because the first attempted fix was wrong in
  an instructive way: a process-wide shared pool, which is what the store suites
  use, **cannot** be used here — `#[tokio::test]` builds a runtime per test, and
  a sqlx pool carries a background task bound to the runtime that created it, so
  sharing one across them left 6 of 17 tests failing and the suite taking 184s
  instead of 4s. The per-test pool is right; only its size was wrong. CPR-4's
  suite has the same shape and was left as it is — it passes, and changing a
  passing suite's harness is not this prompt's to do.
- **A pre-existing failure this prompt did *not* fix, and the reason.**
  `pnpm -r test` fails four of the Claude Code adapter's 74 tests
  (`adapters/claude-code/src/skills.test.mts`) with `ENOENT` on the argv file a
  spawned fake CLI is supposed to write into a temp directory. **It fails
  identically on the base tree**, verified by stashing this prompt's diff, so it
  is this machine rather than this change — the same on-access scanner that
  makes the Rust gates take hours. Repairing an adapter's test harness is
  outside a membership feature, and inventing a skip would be exactly the
  "hide a failure with an ignore" this programme forbids. Named here so the next
  machine to run `make ci` knows which red is inherited. The console's 51 tests
  pass.
- **Run record.** On the final tree, against a live Postgres:
  `crates/synveda-store/tests/access.rs` **30/30**,
  `crates/synveda-gateway/tests/access_api.rs` **17/17**,
  `crates/synveda-policy/tests/access.rs` **7/7**,
  `crates/synveda-store/tests/rls.rs` **81/81** (76 before this feature),
  `crates/synveda-gateway/tests/openapi.rs` **5/5**,
  `crates/synveda-gateway/tests/workspaces_api.rs` **23/23** (CPR-4's, unchanged
  by the owner grant this prompt adds to its creation path),
  `crates/synveda-gateway/tests/explorer.rs` **9/9** after the re-record, and
  `synveda-types` unit tests **188/188**. `demos/cpr-5-access.sh` **green end to
  end** against a real gateway, a real database and two people's tokens.
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo build --workspace` and `cargo deny check` all clean. The
  node gates green: `check-api-types`, `check-backlog`, `check-adr-status`,
  `check-deps`, `check-corpus-licences`, `check-chart-images`,
  `check-benchmarks`, `check-ann-bench`, `check-npm-licences`, `chart-lint`,
  `eval-check`, `ts-build`. `ts-test` is the inherited red named above. The
  `.sqlx` cache gained 29 entries and lost none.

  **`cargo test --workspace` is slow on this machine for the reason CPR-3
  recorded, and it is worth restating with a fresh measurement.** Bitdefender's
  on-access scanner runs `codesign -vv -R=notarized --check-notarization`, as
  root, against every freshly built binary in `target/debug/deps` — **measured
  here at 9.3 seconds warm for one binary**, against CPR-3's ~51 seconds cold —
  which is tens of minutes to over an hour across the workspace's binaries. It
  is the machine and not the tree: the fix is to exclude `target/` from
  on-access scanning, and until somebody does, a full run is an overnight job
  rather than a loop. Two of this prompt's three inherited/introduced failures
  above were only visible in such a run, which is the argument for paying it.
- **Commit.** `feat(access): add groups grants and invitations` on
  `feat/context-platform-mvp`.
- **Commit hash.** `e20f9fa379823ed285262383607c95ae1351ecb4`, written by
  Prompt 6 on Prompt 1's rule.

### Prompt 6 — Governed scope anchors: the PDP re-cut (CPR-6)

- **Implemented.** The decision point, re-cut over the governed scope model
  (ADR-0073). `synveda_store::anchors` — the scope-anchor resolver;
  `synveda_types::anchor` — `AnchorSet`, `ScopeAnchor`, `AnchorSource`;
  `synveda_policy::{ScopeNode, ResourceEntity}` and a **seven-entity** Cedar
  model; `Resource` gains `Workspace`, `Project`, `Group` and `Grant`; the
  twenty-six routes CPR-4 and CPR-5 anchored at the tenant now name the thing
  they are about; `Principal.department` is deleted with the rank vocabulary;
  personal principal-scope privacy becomes a base-layer forbid **with a door**;
  `GET /v1/me` forecasts per anchor from real decisions; and the SCIM boundary
  projects directory groups onto `groups` + `group_members`.
- **Divergence from §9.** None worth recording. §9's Prompt 5 read *"Policy
  profiles and the PDP re-cut"*, and CPR-5 took the membership half of Prompt 4;
  this prompt is the PDP re-cut arriving one entry late, which the record
  already predicted ("the PDP re-cut is the next one"). The **policy-profile
  rename** (`personal`/`team`/`enterprise`, ADR-0068 decision 2) is *not* here:
  the prompt's own text does not ask for it and says so by omission — it names
  the anchors, the entity model, the rank removal, `/v1/me` and SCIM. Recorded
  so the next reader does not look for it.
- **Schema/domain changes.** One migration, `0043_scope_anchors.sql`, which adds
  **no table**: `scopes.principal_id`, present exactly on a `principal`-shaped
  scope, unique per tenant, immutable (the CPR-3 trigger extended rather than
  duplicated). **60 tables, 2 views; 56 RLS-forced**, unchanged — the RLS
  completeness inventory is untouched because nothing new is stored.
  `synveda_types::anchor` is a new module; `synveda_types::scope::Scope` gains
  `principal_id`; `synveda_store::scopes` gains `principal_scope` and
  `ensure_principal_scope`; `synveda_store::anchors` is new;
  `synveda_store::access` gains `sync_directory_group` and
  `retire_directory_group`.
- **API and frontend changes.** **No route added or removed** (76 `/v1` route
  paths, unchanged). What changed is what each decision *names*: a read or an
  update names the workspace or the project, a project creation names its
  workspace, curating a group names the group, revoking names the grant, and
  the tenant-plane calls name the tenant **root scope**. `GET /v1/me` grows
  `anchors` and `anchors_not_answered`; `TenantCapabilities` grows `role_keys`.
  `docs/api/openapi.json` and `console/src/generated/api.ts` regenerated (26
  operations, 33 schemas). No CLI command and no console screen.
- **The ownership check moved in front of the decision** on every per-object
  route, because deciding *about* a workspace requires having fetched it. That
  is ADR-0012 decision 7's order and the hierarchy plane's, so it is a
  convergence — but it is a behaviour change and two existing tests asserted the
  old order (a made-up id used to be a 403 and is now a 404).
- **Deleted.** `Principal.department` and `nearest_department`; the rank
  vocabulary from the Cedar schema (`Scope.kind` is now the five shapes);
  `principal.home`, renamed to `own_scope`; **every pack's
  `resource.kind != "user"` clause** — the privacy floor moved to the base layer
  where no pack can drop it and where a direct grant can lift it;
  `standard`'s four `principal.department` permits; and the two dead
  `workspace_scope`/`project_scope` helpers in the access plane. Nothing was
  translated: no row of `hierarchy_nodes` became a scope, and the one function
  that reads the old vocabulary — `ScopeNode::from_hierarchy` — writes nothing
  and is deleted with the table.
- **Two findings that changed the design mid-flight.**
  1. **The obvious re-cut of `standard` was dead policy.** Replacing
     `principal.department` with `resource in principal.anchors` looked right and
     asserted nothing: a grant reaches its own subtree under *every* pack,
     through `context.roles`, so the permit could never be the reason anything
     was allowed. What makes `standard` different is one step **outward**, so it
     reads `principal.ambit` — the parent of every held scope, minus the tenant
     root. Found by a test that failed for the right reason.
  2. **The privacy floor was in the wrong place and strictly too strong.** Every
     pack restated `resource.kind != "user"` per permit, which refused even a
     grant somebody deliberately wrote at their own scope — so "share my notes
     with you" was unsayable by the only person entitled to say it. Moving it to
     the base layer fixed both halves at once. The governance carve-out
     (`PolicyAssign`, `RoleRead`, the structural and service-identity planes)
     came from a *third* failure: a blanket forbid stopped an administrator
     assigning a retention profile to a personal scope, which is governance
     rather than disclosure.
- **A gap this feature found and did not close.** **Nothing mints a tenant's
  first grant.** A brand-new tenant has a root scope, no grants and no bindings,
  so nobody can create the first workspace — every shipped pack prices
  `WorkspaceCreate` at an admin role or an `owner` grant. CPR-4's own suite bound
  `org-admin` for the same reason and the programme's headline claim ("a person's
  first act is `POST /v1/workspaces`") is still not true end to end. What changed
  here is that a **grant** now works where only a binding did; where the first
  one comes from is admission's, and both the gateway suite's harness and
  `demos/cpr-6-anchors.sh` seed it explicitly and say why.
- **Tests.** `crates/synveda-policy/tests/anchors.rs` (**17**): the nine named
  properties, decided by the real PDP against the real packs — personal
  principal-scope privacy across all three packs and all four tiers (and its
  two halves: your own scope is yours, and a *direct* grant reaches it where an
  inherited one does not); project sharing; workspace inheritance with the
  project anchor proven not-direct; project-only access refused every verb
  upward; group-derived access, and a group with no grant conferring nothing;
  revocation refusing the next decision, and a grant being a resource a decision
  can name; organisation-unit policy inheritance (the profile *and* the grant,
  two levels down); **`depth_is_not_authority`**, which nests the same tree four
  levels deeper and asserts every probed action decides identically; no
  cross-tenant entity injection, including a chain **spliced** across two
  tenants; and effective capabilities moving with the grant and the profile and
  with nothing else. `crates/synveda-store/tests/anchors.rs` (**13**): the six
  inputs, the ordering, the merge, inheritance with **zero rows written**,
  project-only reach, group resolution and archived-group collapse, revocation,
  tenant filtering, and the `principal_id` rules **against direct SQL**.
  `crates/synveda-gateway/tests/anchors_api.rs` (**9**): the whole path on a
  grant alone with `role_bindings` empty, project-only access at the HTTP
  surface, revocation on the next request, group access arriving and leaving,
  `/v1/me` minting and serving the caller's own scope, its capability block
  moving with the grant, nobody else's own scope ever being an anchor, and
  cross-tenant 404s. `demos/cpr-6-anchors.sh` drives all eight claims against a
  real gateway, a real database and three tokens, with **no `role bind`
  anywhere**.
- **Tests changed rather than added, and why.** `packs.rs`'s golden matrix for
  `standard` moved (a caller with no grant reads their own chain, not their
  department) and `standard_shares_within_the_department_only` became
  `standard_shares_within_what_you_hold`; `sensitivity.rs`'s department test
  became a neighbourhood test; `entity_sync.rs`'s two HIER-3 tests were
  re-expressed over the membership floor, because the property they exist for
  (a reshaped chain is never answered from a stale fragment) survives the rank
  and the rule they used to demonstrate it with does not; the three packs bumped
  `@16 → @17` in five assertions; `access_api.rs`'s denial sweep now uses real
  ids for the two admin-grant routes, because those resolve what they are about
  before they decide; and `workspaces_api.rs` stopped asserting that a fresh
  tenant has *no* scope tree, because `/v1/me` now mints the caller's own scope
  and therefore the root — the claim it protected ("nobody is asked to declare
  an organisation") is unchanged and is what it now asserts.
- **Run record.** On the final tree, against a live Postgres:
  `crates/synveda-policy` **98/98** across eleven binaries,
  `crates/synveda-store/tests/anchors.rs` **13/13**,
  `crates/synveda-gateway/tests/anchors_api.rs` **9/9**,
  `crates/synveda-store/tests/rls.rs` **83/83** (81 before this feature),
  `tests/access_api.rs` **17/17**, `tests/workspaces_api.rs` **23/23**,
  `tests/openapi.rs` **5/5**, `tests/explorer.rs` **9/9**,
  `tests/authz_hierarchy.rs` **2/2**, `tests/hierarchy_admin.rs` **4/4**,
  `tests/scim.rs` **14/14**, `tests/policy_routes.rs` **2/2**,
  `tests/roles_routes.rs` **2/2**. `demos/cpr-6-anchors.sh` green end to end.
  The `.sqlx` cache gained 17 entries and lost 10.
- **A harness constraint this prompt hit, restated because it costs time.**
  Running several database-backed suites in one `cargo test` invocation exhausts
  the dev Postgres's 100 connections: each gateway suite opens a bootstrap pool
  and an application pool per test, and `cargo test` runs binaries in parallel.
  Every failure that produced was `connect to DATABASE_URL` at the harness line,
  never an assertion, and every suite passes when run alone. `anchors.rs`'s pool
  is two connections for that reason (CPR-5's finding, applied before it bit).
- **Commit.** `refactor(auth): use governed scope anchors` on
  `feat/context-platform-mvp`.
- **Commit hash.** `a286f4b6d2d90addff81fc3a58a22fcda067edf0`, written by
  Prompt 7, on Prompt 1's rule.

### Prompt 7 — The hierarchy cutover: one scope tree (CPR-7)

- **Implemented.** The prompt six records deferred to: the old fixed
  hierarchy deleted **whole** and the governed scope model left standing
  alone (ADR-0074). What left the schema: `hierarchy_nodes`,
  `hierarchy_closure`, `role_bindings`, `group_mappings`. What left the
  types: `ScopeKind {org…user}` with `rank()`, `HierarchyNode`, `Role`,
  `RoleBinding`, `Identity::quarantined`. What left the product:
  `/v1/hierarchy/*` (no alias — negative API tests assert the 404s),
  `synveda hierarchy`, `synveda role bind`, the `synveda-{dept}-{team}`
  JIT convention, the placement-based quarantine convention, the HIER-2
  chain cache and the console hierarchy explorer. What arrived: **six
  public admin routes** (`GET/POST /v1/admin/scopes`, `GET/PATCH
  /v1/admin/scopes/{id}`, `…/ancestors`, `…/descendants`), **five CLI
  commands** (`synveda scope list|show|create|move|tree`), and two
  re-homed sub-surfaces under the same prefix — per-scope pack assignment
  (`…/policy`) and the VedaFlow curator file (`…/curators`).
- **Divergence from §9.** §9's Prompt 7 reads *"Sessions"*. The prompt as
  it arrived is the hierarchy cutover — §7 deletion-map row 1's second
  half, which CPR-3 had assigned to "Prompt 6" and CPR-6 then displaced —
  and the prompt's own text is authoritative (§9's preamble). Recorded
  rather than absorbed because the swap is the programme settling into its
  real dependency order: the memory plane's re-anchoring (Prompts 7–12 as
  numbered) needs the tree it hangs off to be the only tree first, and
  every CPR since 3 has paid a projection seam or a contortion (the
  identity plane that could not use its own membership model) that this
  prompt removes. Sessions follow.
- **Schema/domain changes.** No new tables. The chain is **rewritten in
  place**: the scope substrate (CPR-3's migration 0040, merged with
  0043's `principal_id`) becomes `0004` where the hierarchy was;
  `identities`/`group_mappings`-free `0007` foreign-keys `scopes`;
  `policy_pack_assignments` foreign-keys `scopes`; `0009_role_bindings`
  and the old `0004`/`0040` files are deleted — **43 → 41 migrations,
  56 tables** (60 minus the four that left), RLS-forced count unchanged
  and the completeness inventory drops exactly the four. The epoch bumps
  **1 → 2**, so every pre-cutover database is refused by the CPR-2 guard
  with the reset instruction rather than by a checksum error — which is
  the guard doing what ADR-0069 decision 3 built it for. One substrate
  rule changed: a `principal` may nest under `tenant`, `org_unit` **or
  `workspace`** (ADR-0074 decision 3) — a service identity's confinement
  anchor is tree position, so the shape must admit it. `scopes::NewScope`
  gains `set_attributes`; `ScopeNode` (the PDP's) gains an immutable
  `slug` — its one display field, carried because the composition plane
  renders a section header per scope and a slug cannot go stale.
- **API and frontend changes.** Six admin routes on the OpenAPI contract
  (26 → **32 operations**, 17 → 21 paths); `PATCH` carries
  rename/archive/**move** (`parent_scope_id`), a move decided at both ends
  and audited with both; creation idempotent under `Idempotency-Key`. The
  `Hierarchy*` Cedar actions became `ScopeCreate/Read/Update` (no delete —
  retiring a scope is a status transition); `RoleRead`/`RoleAssign` left
  with the bindings, and the base layer's escalation guard with them.
  `context.roles` carries **grant keys only**. The console explorer re-cut
  onto the scope plane (tree/pack/lapse/capability panels; the roles panel
  gone with the bindings, the parity corpus case with it);
  `console/src/generated/api.ts` regenerated (39 schemas). `synveda scope`
  replaces `synveda hierarchy`; `synveda service register` mints a
  principal scope under the operator's anchor; `synveda role` is gone.
- **The identity plane un-contorted.** Placement is identity (ADR-0074
  decision 3): an identity's `scope_id` is its own principal scope —
  minted by `ensure_principal_scope` at first login, by SCIM projection
  keyed on the directory's `externalId` (adopted at login through
  ADR-0059 decision 4's correspondence rule, the identity row read before
  any subject-keyed fallback so one person is never two scopes), and by
  service registration under the operator's anchor. Quarantine is only
  ever "not provisioned". The `synveda-admins` convention upserts an
  `administrator` grant at the tenant root — the operator door ADR-0073
  recorded as missing. The reconciler's `apply_placement` (group-driven
  moves, pack-boundary sealing) is **deleted**: belonging is directory
  groups and grants, and placement-as-configuration is Prompt 29's.
- **One gather.** The gateway's two decision-gathering paths collapsed
  into the governed one: every route's resource chain comes from
  `scope_closure`, every caller's own chain starts at their identity's
  scope, `context.roles` is grant keys only, and `ScopeNode::from_hierarchy`
  — CPR-6's projection bridge — is deleted unreplaced. The composition
  plane (`MemoryReadInputs`) gained `anchors`/`groups` and lost
  `role_bindings`: the read path finally decides with the grants CPR-6
  made resolvable, which is the widening `standard`'s `principal.ambit`
  rule was waiting for. The HIER-2 warm cache went with its tree; chains
  resolve per request, and the post-mutation seam narrowed to
  `pdp.flush_entities` (`invalidate_scopes`).
- **The approval matrix speaks grant keys** (ADR-0074 decision 6):
  proposals' recorded approvals, curator files' `role:` entries, the
  embedded matrices and every Cedar role list. `steward`/`org-admin`/
  `compliance` → `administrator`; `security-reviewer` → `reviewer`; the
  floors unchanged in substance (restricted ⇒ administrator + 2 distinct,
  any skill ⇒ reviewer + 2 distinct), Prompt 27 named for the specialist
  names' return. Packs bumped **@17 → @18**.
- **Deleted.** Four tables; three type modules (`types/hierarchy.rs`,
  `types/role.rs` wholesale); four store modules (`hierarchy`, `scope_chain`,
  `role_bindings`, `group_mappings`); two gateway route modules
  (`hierarchy`, `roles`); the CLI hierarchy module and the `role` command;
  ten test suites whose subject was the old model (store `hierarchy`,
  `scope_chain`, `role_bindings`; gateway `hierarchy_admin`,
  `roles_routes`, `scope_chain_routes`; policy `roles`; the rls blocks for
  the four tables; the identities convention block); the console roles
  panel and its parity case; the scope-chain cache metrics. Nothing was
  translated: no `hierarchy_nodes` row became a scope, in either
  direction, at any time.
- **Tests.** New: `crates/synveda-gateway/tests/admin_scopes_api.rs` (**7**)
  — the six routes walked end to end, the **negative half** (seventeen
  `/v1/hierarchy` method-paths all 404; all five old kinds 400 by name),
  idempotency incl. the key-reuse 409, the move (lands, cycle-refused,
  both ends in the audit event), the ungranted caller (denied the reads
  and every mutation — `ScopeRead` is owner/administrator under every
  shipped pack — foreign tenant 404), and no-credential 401s.
  Re-cut: every store/policy/gateway suite that seeded or decided through
  the old model (seeding rebuilt on `scopes::create`/`ensure_*`;
  binding-based assertions re-expressed as grant anchors; golden
  matrices over the grant-key vocabulary; `rls.rs` inventory minus four).
  `demos/cpr-7-scopes.sh` drives the whole thing against a real gateway
  and a real database — no `role bind`, no `/v1/hierarchy`, anywhere in
  it.
- **The re-vocabulary lost the tenant root, and a weakened test hid it.**
  The scope-kind cells of the approval matrix are the one place the
  translation could silently drop a row, and it did. The old rule split
  `[org, division, department]` (SHARED) from `[team, user]` (LOCAL); the
  new one read `SHARED = [org_unit]` and `LOCAL = [principal, workspace,
  project]`, which leaves **`tenant` in neither** — so under
  `regulated-strict` a memory published at an **org unit** took a curator
  and an administrator, two distinct people, while the same memory
  published at the **tenant root**, the widest audience the product has
  and the one scope on every member's own chain, auto-approved. The
  partition test that exists to catch exactly this
  (`memory_rules_partition_the_scope_kinds`, "so no cell falls through to
  auto-approve by accident rather than by decision") had had `Tenant`
  dropped from the shapes it iterates, and `cross_scope.rs` had been
  rewritten to *assert* the hole ("a tenant-root publication needs no
  approvals"), which is what left its second approver unused and was the
  thread that led here. `SHARED` is now `[Tenant, OrgUnit]` — the root
  carries the `org` row it replaced — the partition test iterates all
  five shapes again, `cross_scope.rs` asks the root for its own curator
  **and** its own administrator, and the golden matrix carries the
  restored cells. Recorded at length because the failure mode is the one
  a hard cut is most exposed to: not a rule that broke, a rule that
  stopped being asked.
- **Two more things the cutover claimed and had not finished.** The
  `synveda-{dept}-{team}` convention was recorded as deleted in three
  places while `synveda_identity::mapping` still exported
  `CONVENTION_PREFIX`, `ConventionCandidate`, `convention_candidates` and
  `personal_slug` — dead the moment placement became identity, invisible
  to `dead_code` because they are `pub` in a library crate. The module is
  now the one convention that survives (`ADMIN_GROUP`,
  `contains_admin_group`) and its tests say what the prefix no longer
  means. And a move was "audited with both ends" (ADR-0074 decision 5)
  while its event carried `moved_to` alone; the origin parent is read
  before the move rewrites it and lands as `moved_from`, with the test
  asserting both ids rather than the presence of one.
- **The old demo corpus is standing, and named rather than left to be
  discovered.** Four demos whose *subject* is what this prompt deletes are
  deleted with it — `hier-1-hierarchy.sh`, `hier-2-scope-chain.sh`,
  `hier-3-cedar-entity-sync.sh`, `authz-3-roles.sh` (69 → 65) — and the
  programme's own three, `cpr-4`, `cpr-5` and `cpr-6`, are re-cut onto the
  grant bootstrap. **Forty-three Phase-3 demos still seed through
  `role bind`, `hierarchy_closure` inserts or `/v1/hierarchy`, and will
  fail at that line.** They are not re-cut here: each belongs to a
  subsystem Stage B onward re-anchors, and re-cutting them blind — no CI
  target runs them — would be forty-three unverifiable edits. `make ci` does not
  cover them, which is exactly why the number is written down. STATUS.md
  carries the same note beside HIER-1, HIER-2, HIER-3 and AUTHZ-3, whose
  entries now say what replaced them rather than pointing at files that no
  longer exist.
- **Run record.** Three production defects the suite re-cut surfaced and
  the prompt fixed: a move's destination decision was taken without the
  destination's own chain in the Cedar context (the entity the decision
  named was absent, so every legal move read as a denial) — it now
  gathers at the destination; an idempotent replay decided with no anchor
  at all (empty chain, empty roles) — it now anchors at the parent, as
  the original create did; and the capability/lapse surfaces rendered a
  scope's bare slug where they promise a `scope_path` — both now render
  the slug chain. `permits_parent` disagreed with the substrate's CHECK
  about a principal's permitted parents (Rust said the tenant root only;
  the schema and ADR-0074 say tenant, org unit or workspace) — the Rust
  rule was the bug, and the widened rule is what makes
  `POST /v1/service-identities` able to mint an agent's scope under the
  operator's anchor at all. The explorer parity corpus re-recorded
  (three cases; the roles case died with the bindings) and the console
  suite passes against it (49/49).
  **The suite the cutover re-cut had never been run against it**, and
  running it found forty-three failures across seventeen gateway suites.
  Three were production defects and are fixed here: the approval matrix's
  tenant-root hole above; a `synveda-admins` login by an
  **already-provisioned** subject wrote the bootstrap grant and then
  returned without committing, so the operator door of ADR-0074 decision 4
  silently failed for every directory-synced admin (the `bound` branch of
  `provision_once` now commits, and
  `the_admin_door_opens_on_a_later_login_and_the_grant_is_committed`
  is the regression test); and `IdentitySummary.scope_path` promised a slug
  chain and served a bare slug, beside a `quarantined` field that could no
  longer be anything but `false` and is deleted.
  Four pack rules had been re-vocabularied into holes and are restored,
  each recorded on `EMBEDDED_PACKS`' `@18` line: the SHARED cell (above),
  `DirectoryManage`/`DirectorySealAuthorise` (the old `org-admin` was
  mapped to `owner` alone, locking every directory operator out),
  `ProposalOpen`'s membership floor (ADR-0074 decision 8 — anchors are not
  entity parents, so `principal in resource` stopped reaching the scope
  above and the FLOW-5 climb became unsayable), and the quarantine review
  plane (decision 7 — every quarantined event now lands on a private scope
  that inherits nothing, so a verdict anchored there was reachable by
  nobody). Two more model gaps were closed the same way: every principal
  scope now carries an `owner` grant at itself (decision 8), and SCIM's
  create-time uniqueness ignores sealed rows, which a rehire needs.
  The rest was fixture breakage from the incomplete re-cut — teams mapped
  to `org_unit` (SHARED) where they had been LOCAL, test packs still
  reading the deleted `principal.home` attribute, proposals opened at
  scopes the actor held nothing at, and material seeded at scopes no
  reader's chain reaches.
- **Finishing the run found a fourth production defect, upstream of the
  other three.** `synveda-retrieval::authz::materialise` — the entity
  batch every `composition_plan` call (`inject`, `recall`, lapses) decides
  against — built its `AuthzContext` with `..AuthzContext::default()`,
  which is `anchors: &[]` and `groups: &[]`. `entities_over`
  (`synveda-policy`) bakes `principal.ambit`/`principal.anchors`/
  `principal.private` into the Principal *entity* at that call, once; the
  per-scope `AuthzContext` every later decision builds only supplies
  Cedar's request `context` map (roles, sensitivity, lapsed) and cannot
  repair an entity already materialised without it. So every decision
  taken through a composition plan — not only the widened candidates this
  prompt's own standing-gap note is about — evaluated `standard`'s
  `principal.ambit` sharing permit, the private-scope door, and any
  group-anchored grant against an entity that could never satisfy any of
  them, silently, since CPR-6. Recall's own widened universe (CTX-5,
  ADR-0042 decision 2) is what surfaced it:
  `a_query_reaches_material_the_chain_never_composes` asserts exactly the
  ambit permit this bug denied. Fixed by passing `inputs.anchors` and
  `inputs.groups` into the materialise-time context; nothing else in the
  decision path changes, because nothing else needed to.
- **Three more, smaller.** A rehire under the *same* directory resource
  collided on `principal_id`'s per-tenant uniqueness (migration 0043):
  `ensure_principal_scope` finds-or-creates by the raw anchor, and a
  reactivated SCIM user computes the identical anchor its departed self
  already holds — `place()` now disambiguates by appending the fresh
  identity id when the natural anchor is already taken, so "a rehire is a
  new person" (the reconciler's own words) is what happens rather than a
  409. `identities::rescope`/`seal_scope_as_former_self` — the
  group-driven-move machinery ADR-0074 decision 3 deleted the caller of —
  were dead (zero references outside their own definitions) and are
  deleted with it. And two test suites (`skills.rs`, `tiered.rs`) needed
  more than a scope swap: `skills.rs`'s gradient tests (nearest-copy
  shadowing, "own team's skills, the org's, never another's") assert CTX-2's
  multi-level walk, which needs a real placement chain under a caller —
  restored via the shape `POST /v1/service-identities` already mints an
  agent's scope under its operator's anchor with (`scopes::create` at
  `kind: Principal, parent_scope_id: Some(anchor)`, the same pattern
  `inject.rs`'s `seed_agent` already used), rather than the root-only
  `ensure_principal_scope`.
- **A fresh full run is what caught the last one.** After every gateway
  suite was green individually, a clean `make db-test` (fresh scratch
  database, whole workspace, no filter) found one more:
  `synveda-store/tests/anchors.rs`'s
  `the_callers_own_scope_sorts_first_and_inherits_nothing` asserted a
  caller's own scope carries **no** roles until a grant names it directly
  — true before decision 8, false after (every principal scope now
  carries its own `owner` grant at itself, from the moment it is minted).
  The fix is the test's, not the product's: the tenant-root probe grant
  now uses a role decision 8 does not already imply (`administrator`
  rather than `owner`), so "a wide grant does not reach a private scope"
  and "the scope's own baseline grant" stay distinguishable. Recorded as
  its own bullet because it is the shape every other fix in this record
  took, found by the one thing that reliably finds it: running the whole
  suite fresh rather than trusting suites verified in isolation to still
  agree with each other. `make db-test` is green on the final tree;
  `make ci` is the last gate (§ below).
- **The largest thing this prompt does not carry.** `composition_plan`
  still walks the caller's **chain** — their own scope outward to the
  tenant root — and anchors reach it only as decision *context*. With
  placement gone, that means an agent's `inject` sees its own scope and
  the tenant root and nothing else: **joining a workspace gives that
  session nothing**. Making the candidate set the anchor set is the
  composition contract's re-cut, which §9 assigns to Prompts 16–18, and
  beginning it here would be beginning a later prompt. Every fixture in
  this cutover was re-cut to the model as it stands — material a reader
  must see lives on that reader's chain — and ADR-0074 records it under
  "What this cutover does not carry".
- **Commit.** `refactor(scopes): remove fixed hierarchy model` on
  `feat/context-platform-mvp`.
- **Commit hash.** `b2c174d8d131540052e87b9883c7147c685d6a0f`. Written by
  **Prompt 8**, on Prompt 1's rule.

### Prompt 8 — The console product shell & first-run onboarding (CPR-8)

- **Implemented.** The console re-cut from a governance entry point into a
  route-based product shell (ADR-0075), and the first prompt of this
  programme whose deliverable is a screen rather than a plane. Six prompts
  built a context platform reachable only from the CLI; `App.tsx` still
  resolved its session with `whoami` and mounted the proposals inbox and the
  scope explorer one after the other, with no navigation at all. What
  arrived: a **route table** with two menus — a primary menu shown to
  everybody (Home, Sessions, Knowledge, New Learnings, Skills, Tools,
  People, Settings) and an advanced menu shown only where the capability
  forecast offers the plane (Reviews, Scopes, Policies, Audit, Service
  identities) — client-side routing over the History API, workspace and
  project switchers over a persisted-and-reconciled selection, one
  query/cache layer with one loading and one error state per route, a typed
  client over the generated contract, a People page, first-run onboarding,
  and honest pages for the four planes that have no API yet.
- **Divergence from §9.** §9's Prompt 8 reads *"Session events and the
  observe path onto sessions"*, and §9's Prompt 7 read *"Sessions"* before
  CPR-7 took that slot for the hierarchy cutover. The prompt as it arrived
  is the console shell — §9's Prompt 20, "Console on generated types" —
  and the prompt's own text is authoritative (§9's preamble). Recorded
  rather than absorbed, because the swap is larger than CPR-7's: it moves a
  Stage E prompt in front of the whole of Stages B–D. The programme's
  numbered order is not renumbered here; Stage B's memory-plane work is
  unstarted and the prompts that run it will say so. What the swap costs is
  visible in this entry's own standing gaps — four primary pages with no
  plane behind them, and seven surfaces still on hand-written paths because
  Prompt 19 has not run — and what it buys is that the platform CPR-3
  through CPR-7 built is reachable by somebody who is not holding a
  terminal.
- **Schema/domain changes.** **None.** No migration was added, altered or
  removed; no table, type or store service changed. The epoch stays at 2.
- **API and frontend changes.** One **contract defect fixed at the source**:
  `list_scopes` declared `parent_id` as a `Path` parameter on
  `/v1/admin/scopes`, a route with no such placeholder — `utoipa`'s
  `IntoParams` defaults to `Path` and nothing had declared otherwise. It is
  now `#[into_params(parameter_in = Query)]`, with `docs/api/openapi.json`
  and `console/src/generated/api.ts` regenerated. Invisible while every
  caller was hand-written; the first thing a generated client trips over.
  The **generator** gained two things: it emits the runtime path/method
  table (`OPERATIONS`) beside the type table, so no hand-written second copy
  of a path exists anywhere in the console, and an `idempotent: true` flag
  for the eight operations whose document requires an `Idempotency-Key` —
  which `client.mts` turns into a required argument at compile time and
  refuses at runtime for an untyped caller, both ways round. Operation and
  schema counts unchanged (32 operations, 39 schemas). No `/v1` route was
  added, changed or removed, and no console-only route exists (ADR-0056
  decision 9 standing).
- **Frontend.** New: `routes.mts`, `Router.tsx`, `client.mts`, `cache.mts`,
  `Query.tsx`, `selection.mts`, `Shell.tsx`, `people.mts`,
  `onboarding.mts`, and the pages `Home`, `People`, `Settings`, `Skills`,
  `Planned`, `Onboarding`, `Policies`, `Audit`, `ServiceIdentities`.
  Renamed: `Inbox.tsx` → `Reviews.tsx`, `Explorer.tsx` → `Scopes.tsx`, both
  unchanged in substance and now behind their own capability. `App.tsx`
  rewritten. `styles.css` extended with the shell layout — and one existing
  rule corrected: `.banner` carried the error colours unconditionally,
  because until now there was only ever one kind of banner, which would have
  made "the chain verifies" and "your session expired" look identical the
  moment a second arrived.
- **Deleted.** `App.tsx`'s direct mounting of `Inbox` and `Explorer`, and
  the shell that had no navigation. `api.mts`'s `whoami` and its `WhoAmI`
  type — superseded by `GET /v1/me`, which answers five things where
  `whoami` answered two, and keeping it would be keeping a second answer to
  the question every page starts with. The hand-written path strings for
  every contract-covered route (they are the generated table now).
- **Four decisions worth reading, all in ADR-0075.** **The primary menu is
  unconditional** (decision 1): a navigation that grew and shrank with a
  role would teach every reader a different shape for one application.
  **The personal/team question seeds and does not brand** (decision 6): it
  chooses a policy pack and a membership posture and records nothing — no
  column, no field, nothing that branches — because ADR-0068 decision 1
  forbids an edition and a wizard asking "is this just you?" is the
  friendliest possible door for that branch to arrive through;
  `onboarding.test.mts` asserts the plan carries no `kind`/`edition`/`tier`
  field **by name**. **The seeding is best-effort and reported**: a first
  caller holds an `owner` grant on what they just created and may hold
  nothing that permits `policy.assign`, so a refusal is a sentence pointing
  at Advanced ▸ Policies and never blocks the wizard — silently skipping it
  is the one unacceptable option, because somebody would leave believing
  their workspace was governed the way they chose. And **a plane with no
  API gets an honest page rather than an empty list** (decision 7): an empty
  list is indistinguishable from a plane that works and had a quiet week,
  which is precisely the wrong thing to tell somebody whose agent has been
  running all week.
- **Nothing here decides anything.** ADR-0058 decision 2 restated at the top
  of `routes.mts`, where somebody editing the table will meet it: the
  capability map chooses what to **offer**, every act still takes its own
  decision at its own seam, and a reader who reaches a guarded route through
  a stale forecast or a typed URL gets an explanation naming the missing
  action — not a redirect, because a redirect to Home tells somebody their
  link was wrong when in fact their role was.
- **No dependency was added.** Routing and the cache are written in-repo
  (~200 tested lines between them) rather than installed, because the
  shipped bundle's licence gate has no exception mechanism
  (`scripts/check-npm-licences.mjs`) and the page is served under
  `default-src 'none'` with `connect-src 'self'`. The checker reports **3
  shipped packages before and after**. The reversal trigger is in ADR-0075:
  nesting, per-route loaders or route-level code splitting, and a router is
  the right answer.
- **Tests.** **49 → 121** in the console suite. New: `routes.test.mts` (9)
  — the two menus asserted item by item, including that *no* primary route
  is gated and that a caller with nothing sees no Advanced heading at all;
  `client.test.mts` (11) — the wire shape, and the two refusals that matter
  (an idempotent operation without a key, an unfilled path placeholder);
  `cache.test.mts` (11) — the four rules, including the one manual testing
  cannot catch (a watched key refetches on invalidation, an unwatched one is
  dropped); `selection.test.mts` (11) — every reconciliation case, plus a
  browser that refuses to store anything; `onboarding.test.mts` (11) — the
  seeding plan's absent edition field, the refusal sentence, the connection
  check's honest verdict; `shell.test.tsx` (9) and `people.test.tsx` (10) —
  rendering, through `react-dom/server` + `toText`, the convention CNSL-1
  established. The CNSL-1 and CNSL-2 parity corpora are untouched and still
  pass: the pure helpers they render through did not move.
- **Two defects the new tests found in this feature's own code.** `slugFrom`
  ran NFKD and then treated the combining mark it produces as a separator,
  so "Ünicode Name" proposed `u-nicode-name` — the decomposition was
  actively harmful without the mark-stripping step beside it. And
  `accessSource` **replaced** the group clause with the directory clause
  when both were true, which drops the actionable half: "managed by your
  directory" tells a reader they cannot change it here, and *which group*
  tells them what to change instead. Both fixed; both are now asserted.
- **What this prompt does not carry.** Four primary pages have no plane
  behind them — Sessions, Knowledge, New Learnings and Tools are waiting on
  Prompts 9–15, and each page says so. Seven surfaces still call
  hand-written paths (`api.mts`'s labelled group): proposals, capabilities,
  policy packs, lapses, audit, skills, service identities — Prompt 19 puts
  the rest of `/v1` on the contract and deletes them. There is no
  group-management screen; People manages grants and invitations, and a
  group screen belongs beside the directory adapter (Prompt 29). And there
  is still no browser test runner, so what is covered is *which facts
  appear* and not the switchers' `onChange`, the wizard's step transitions
  or the mutation round trips — those are asserted at the seam below them,
  which is where the logic lives.
- **No demo script, and the reason is written down.** CLAUDE.md's definition
  of done takes "a test **or** a runnable demo script"; the 72-test console
  suite is the demonstration and `make ci` runs it (`ts-test`), where no CI
  target runs a demo. A console demo would need a browser flow against a
  live Rauthy and a live stack (`cnsl-1-proposals-inbox.sh` is 542 lines of
  exactly that), and Docker was not reachable in this environment — so
  writing one would have produced an unverifiable script, which is the same
  judgement CPR-7 made about the forty-three demos it declined to re-cut
  blind. Two other clauses of the definition of done are **vacuous here and
  stated rather than skipped**: this feature adds no server path, so there
  is no span or metric to add, and no new action type, so there is no audit
  event to emit. Every route it calls already chains the events it chained.
- **Commit.** `feat(console): add workspace product shell and onboarding` on
  `feat/context-platform-mvp`.
- **Commit hash.** `6cc4a39` (`feat/context-platform-mvp`). Written by
  **Prompt 9**, on Prompt 1's rule.

### Prompt 9 — The foundation audit (CPR-9)

- **Implemented.** An audit of Prompts 1–7 rather than a ninth plane on top of
  them. Prompts 1–8 each shipped green, and each suite proved that its own
  plane worked; none had asked the question that spans all of them — **what
  does a caller learn, or fail to learn, that their grants do not say?** This
  prompt asks it, with an adversarial suite
  (`crates/synveda-gateway/tests/foundation_audit.rs`, 6 tests), fixes the
  three defects it found, and widens the one guard that was checking a
  fortieth of what it claims.
- **Divergence from §9.** §9's Prompt 9 reads *"Candidates — extraction output,
  separated from published knowledge"*. The prompt as it arrived is an audit
  of the completed foundation, with "do not start session or knowledge work"
  in its own text, and the prompt's own text is authoritative (§9's preamble).
  Recorded rather than absorbed because this is the third consecutive
  divergence and the reason differs from the first two: CPR-7 and CPR-8 moved
  *other prompts* forward, while this one inserts work §9 never planned. Stage
  B remains unstarted; the numbered order is not renumbered.
- **Schema/domain changes.** **None.** No migration added, altered or removed;
  the epoch stays at **2**. That is a decision rather than an absence — see
  the guard bullet below.
- **API and frontend changes.** No route added, changed or removed; the
  OpenAPI document and `console/src/generated/api.ts` are byte-identical
  (`make check-api-types` passes). What changed is **what two existing
  listings return**: `GET /v1/workspaces` and `GET /v1/me` now serve the rows
  the caller may read rather than all-or-nothing on a single tenant-root
  verdict, and `GET /v1/workspaces/{id}/projects` filters the same way behind
  its existing gate. Two CLI surfaces were repaired (below). No console source
  changed: the shell already read `/v1/me`, and the fix is that `/v1/me` now
  answers correctly.
- **Defect 1 — a grant at a workspace did not reach the listings.** The
  listings took **one** decision, at the tenant root, and applied its verdict
  to every row. For an administrator, who holds a grant at the root, that is
  the right answer by accident. For a member it was wrong in the direction
  that matters: a caller granted `member` at one workspace holds nothing at
  the root, so the decision denied, the listing came back **empty**, and
  `/v1/me` reported `workspace_count: 0` and `onboarding.state:
  needs_workspace` — while the `anchors` block of that same response said
  `workspace.read: true` at that workspace. Two answers to one question in one
  payload, and CPR-8's console renders both, so an invited member was sent to
  the first-run wizard to create the workspace they had just been added to.
  Fixed by deciding **per row against the row**, which is the decision
  `GET /v1/workspaces/{id}` already took and passed: one gather, one
  materialised entity batch, one Cedar evaluation per row under that row's own
  chain and pack assignments (`workspaces::decide_each`, shaped after
  `capabilities::at_anchors` and for its reason — the effective pack is the
  resource's, ADR-0014 decision 3). **Unbounded**, and that was a correction
  made to this feature's own first cut: it capped at 128 rows with a
  `not_answered` count, which would have silently dropped the 129th workspace
  from an administrator who can see it today — introducing, in an audit, the
  exact class of failure the audit exists to find. The cost is three indexed
  reads per row, and the batched `scope_closure` read that would remove them is
  recorded in the function rather than pre-written. There is deliberately **no
  fast path** for a caller permitted at the root: "permitted above ⇒ permitted
  below" is not a property Cedar has — a forbid overrides a permit at any
  depth and a stored pack may write one — so a shortcut there would be a
  second decision point quietly disagreeing with the first. The route still
  refuses a caller who holds nothing at the root *and* nothing below it, so
  the outsider contract `every_route_denies_without_the_action` pins is
  unchanged.
- **Defect 2 — two client/server contracts had drifted apart**, both on routes
  Prompt 19 has not yet put on the contract, so both sides are hand-written
  and nothing checked they agreed. **`synveda login` could not parse a
  successful login**: CPR-7 deleted `identity.quarantined` from the gateway's
  session response (placement is identity, so it could only ever be `false`)
  and the CLI kept requiring it — serde has no default for a missing field, so
  every login failed *after* the browser round trip, *after* the code
  exchange, with the credential already minted. And **`synveda whoami
  --capabilities` could not parse any response**: CPR-7 renamed `roles` to
  `role_keys` and deleted `role_assign` with the binding vocabulary; the CLI
  read the old shape. Plain `synveda whoami` shares the route and never asks
  for the block, which is why nothing noticed. Both fixed and **pinned from
  each side** — the CLI parses a literal of what the gateway serves, the
  gateway asserts the exact key set it serialises, and each test names the
  other's file — so the server cannot drop a field the CLI needs without one
  of them going red.
- **Defect 3 — the no-data-migrator guard checked one file of forty-one.**
  `no_old_to_new_data_migrator_exists` scanned the epoch migration, which is
  the file a translator written *today* would live in, and left unchecked the
  forty where ones written *before* the cut already were. It now scans the
  whole chain, skipping dollar-quoted function bodies so `0001`'s and `0026`'s
  history triggers are not mistaken for translations, and pins the **three**
  inherited pre-epoch upgrade statements by name: `0008`'s
  `update policy_packs … '-legacy'` and `insert into policy_pack_defaults …
  select`, and `0038`'s `delete from console_sessions`.
- **Why those three are pinned rather than deleted, and why the epoch did not
  bump.** They are **unreachable**: `epoch::preflight` refuses a pre-cut
  database before the migrator runs, so the only databases that reach
  migrations 8 and 38 are fresh ones, where the tables those statements touch
  are empty at that point in the chain. Deleting them was tried and measured —
  editing an applied migration changes its checksum, and an existing epoch-2
  database then fails with `migration 8 was previously applied but has been
  modified`, which is a checksum error where ADR-0069 decision 3 promises the
  reset instruction. Doing it properly therefore means bumping the epoch to 3:
  a reset for every deployment and every developer, to remove statements that
  cannot run, in a chain Prompt 33 squashes anyway. Pinned instead, with the
  reasoning in the test rather than here — and the value of the list is not
  its three entries but that a **fourth** fails the build, which was verified
  by adding one.
- **Verified sound, independently of the suites that assert it.** 52
  tenant-bound tables, every one `ENABLE` + `FORCE` with at least one policy,
  read straight out of `pg_class`/`pg_policy` rather than through
  `rls.rs`'s own inventory; the four exempt tables are the documented
  structural ones and `hierarchy_nodes`, `hierarchy_closure`, `role_bindings`
  and `group_mappings` are absent. The scope closure holds over the ~23,000
  scopes the suite produces: a distance-0 self row per scope, no cross-tenant
  pair, no cycle, a distance-1 edge per parent pointer, one parentless
  `tenant`-shaped root per tenant, `principal_id` present exactly on
  `principal`-shaped scopes, and only the five shapes present. The privacy
  forbid holds against a tenant administrator; the capability probe serves no
  `scope_path`, `pack` or roles at a scope the caller holds nothing at; an
  invitation refuses a cross-tenant redemption **and survives it**, still
  spendable by its rightful recipient; and the console's `offersRoute` fails
  closed on a missing capability key.
- **Residue classified.** Every match of the audit's search terms
  (`Division`, `Department`, `hierarchy rank`, `RoleBinding`, `serde(alias)`,
  `legacy`, `compat`, `fallback`, `dual read`, `dual write`, `/v1/hierarchy`)
  was classified. `RoleBinding`, `serde(alias)`, `hierarchy rank` and `dual
  write` match **no code at all**: what they match is prose — ADR-0074
  recording that role bindings were deleted, three ADRs (0039, 0043, 0044)
  each *refusing* a dual write by name, and this section's own list of the
  terms. `/v1/hierarchy` has two matches in production source, both comments
  narrating the deletion, plus the negative tests that assert the 404s. The `legacy`, `compat` and `fallback` matches
  are the MCP protocol era, the pack-compilation fallback, the SPA fallback
  and the correspondence-rule ordering — all live mechanisms with those
  names, none of them old-model compatibility. What *was* deleted is stale
  prose a reader would act on: `synveda init` told operators to run `synveda
  hierarchy list` and claimed the first login provisions an org root and binds
  `org-admin`; `synveda whoami` pointed at `synveda hierarchy capabilities`;
  `--demo` printed its org units as "what the IdP groups resolve to", which no
  group has done since placement became identity; and `ScopeId`'s own doc
  comment still defined a scope as a rung of the
  `org`/`division`/`department`/`team`/`user` ladder. The remaining matches
  are narration, negative assertions, and test-local variable names for org
  units.
- **The headline count had drifted a fourth time, in the other file.**
  `AGENTS.md` states the feature count beside CLAUDE.md's and says "when the
  project's state moves, both files move". It still read **104 filed, 71
  delivered** — CPR-8's own numbers, never applied there — against a checker
  that had said 105 since. CLAUDE.md's counting trail exists because this
  drift has now happened four times; the fourth was the file that carries the
  rule. Both are now 106/73, and `AGENTS.md`'s line says to update itself.
- **The 43 demos were counted again rather than trusted.** CPR-7 recorded that
  forty-three Phase-3 demos still seed through `role bind`,
  `hierarchy_closure` or `/v1/hierarchy`. Re-counted here over non-comment
  lines: 44 files match, one of which is `demos/cpr-7-scopes.sh` asserting the
  404s on purpose. **Forty-three.** The number in STATUS.md is exactly right,
  and they stay for the prompts that re-anchor their subsystems.
- **Tests.** New: `crates/synveda-gateway/tests/foundation_audit.rs` (**6**) —
  the cross-tenant table (21 method-paths, each compared against a fictional
  control for status *and* error kind), the invitation that survives the
  attempt, the member who sees their workspace and not the other, the probe
  that offers no plane it cannot name, the administrator refused at somebody
  else's own scope, and the caller who holds nothing being answered rather
  than errored. Widened:
  `crates/synveda-store/tests/epoch.rs::no_old_to_new_data_migrator_exists`
  (one file → forty-one, with `top_level_statements`). New unit tests:
  `the_session_shape_is_the_one_the_gateway_serves` and
  `a_session_without_a_refresh_token_still_parses` (CLI),
  `the_cli_session_shape_is_the_one_the_cli_parses` (gateway `auth`),
  `the_tenant_capability_block_is_the_shape_the_cli_parses` (gateway
  `capabilities`).
- **Run record.** `make ci` **PASS** and `make db-test` **PASS** — the latter
  on a fresh scratch database (`synveda_test_8767`), the whole workspace, no
  filter: **113 suites green, 0 failed**. That is the run CPR-7's record names
  as the one that finds what suites verified in isolation miss, and it found
  nothing here. Individually along the way: `synveda-gateway` (49 binaries),
  `synveda-store` (18, incl. `rls` 76 and `epoch` 10), `synveda-policy` (84),
  `synveda-cli` (150) and the console suite (121). **No test was
  weakened to make this pass**: the outsider's 403 on `GET /v1/workspaces`,
  the cross-tenant 404s and every existing access-plane assertion are
  unchanged, and the listing fix was verified against them rather than around
  them. Three of the six new tests failed on first run against the code as it
  stood, which is what produced defect 1; two more failed on **my own** test's
  wrong assumptions (an invitation redemption answers 201, and a malformed
  token is refused by grammar before the lookup so it is not a control for a
  well-formed unknown one) and were corrected in the test, not the product.
- **What this prompt does not carry.** It fixes what a *listing* discloses and
  leaves what a *session composes* alone: CPR-7's standing note — anchors
  reach `composition_plan` as decision context rather than as the candidate
  set, so joining a workspace still gives that session no material — is
  Prompts 16–18's, and touching it here would have been starting them. The
  three pinned migration statements leave with Prompt 33's squash. The 43
  demos stay. And the per-scope capability probe still has no CLI verb: CPR-7
  deleted `synveda hierarchy capabilities` and did not replace it, so
  `synveda whoami` now names the console page that renders it rather than a
  command that no longer exists — filing the replacement verb is the CLI
  re-cut's (Prompt 24).
- **Commit.** `test(foundation): harden scope and access cutover` on
  `feat/context-platform-mvp`.
- **Commit hash.** `1caebba` (`feat/context-platform-mvp`). Written by
  **Prompt 10**, on Prompt 1's rule. Attempted in place first, which is why
  the rule exists: writing the hash into the commit changes the hash, and the
  amended entry then named a commit that no longer existed.

### Prompt 10 — The session ledger and runtime API (CPR-10)

- **Implemented.** ADR-0068 decision 5, in full: sessions become the root of
  agent runtime activity, and the correlation string stops being the only
  thing that knows a run happened. Three tables, seven routes, two Cedar
  actions, four audit action types, a console page, and a demo that drives all
  of it against a live gateway.
- **Divergence from §9.** §9's Prompt 10 reads *"Knowledge versions — stable
  aggregate id, immutable revision, content addressing"*. The prompt as it
  arrived is Stage B's **first** item — §9's Prompt 7, "Sessions as the root of
  agent runtime activity" — merged with its Prompt 8 (session events) and
  reaching forward into Prompt 17 for one endpoint's *shape*. The prompt's own
  text is authoritative (§9's preamble). Recorded rather than absorbed, and
  the reason is different from the last three divergences: those moved *other*
  prompts forward, this one **starts the stage §9 planned**, one prompt wider
  than §9 cut it. Stage B's remaining items — candidates, knowledge versions,
  promotion, redaction — are unstarted and keep their §9 order. The numbered
  order is not renumbered.
- **Schema/domain changes.** Migration **`0044_sessions.sql`**, three tables,
  41 → **42** migrations. The epoch stays at **2**: this is an addition to a
  chain nothing has shipped against, not a change to one.
  - `sessions` — one run. Its workspace (required), its project (optional),
    the **derived** governed scope it is decided at, the token subject that
    opened it, the client and its version and installation id, the harness's
    own `external_session_id`, the agent, the model, the repository and
    branch, a task summary, a five-state lifecycle, `started_at` /
    `ended_at` / `last_observed_at`, and a bounded metadata bag.
  - `session_events` — immutable, append-only, ordered, idempotent. Twelve
    event types as a CHECK, the client's declared `event_schema_version`, its
    own `client_event_id`, a **server-assigned** `sequence`, both
    `occurred_at` and `received_at`, a bounded payload and the server's
    BLAKE3 digest of it.
  - `session_context_runs` — one act of composing context for a run: the
    query, the rendered block, its hash, tokens against budget, the entry
    count and which legs degraded.
  - Two unique indexes added to existing tables, in this migration because
    they exist for its keys: `projects (tenant_id, id, workspace_id,
    scope_id)` and `project_repositories (tenant_id, project_id, id)`.
- **The anchor is a row-local fact, not a service's discipline.** This is the
  part worth reading twice. `sessions` carries `workspace_scope_id` and
  `project_scope_id`, each pinned to its owner by a **composite** foreign key,
  and `scope_id` held equal to `coalesce(project_scope_id,
  workspace_scope_id)` by a CHECK — so "a session is decided at its project's
  scope, or its workspace's when it has no project" is enforced against
  anything holding a connection. `projects.workspace_scope_id` is the same
  device one plane up (migration 0041) and exists for the same reason. The
  first cut had one `scope_id` and two contradictory foreign keys over it; the
  contradiction was found by writing the migration out and reading it, before
  it reached a database.
- **API and frontend changes.** Seven routes, all on the OpenAPI contract from
  the day they exist — **32 → 39 operations, 39 → 52 schemas**, with
  `docs/api/openapi.json` and `console/src/generated/api.ts` regenerated. No
  existing route changed. `/v1/observe`, `/v1/inject` and `/v1/recall` are
  untouched and still take the old string; **nothing bridges them**, in either
  direction, and Prompt 11 deletes it.
  - `POST /v1/sessions` (`Idempotency-Key`) · `GET /v1/sessions` ·
    `GET /v1/sessions/{id}` · `POST /v1/sessions/{id}/events` ·
    `POST /v1/sessions/{id}/end` · `GET /v1/sessions/{id}/timeline` ·
    `POST /v1/sessions/{id}/context-runs` (`Idempotency-Key`).
- **`POST …/context-runs` is the final shape and today's minimum depth.** It
  decides `SessionWrite` at the session, then calls the **existing** retrieval
  engine — `composition_plan`, the embed seam, `hybrid_search`, `compose` —
  and persists the identity and the rendered block. Two sources feed the
  universe: the caller's own chain as `chain`, and the **session's** scope
  chain as CTX-5's `candidates` (ADR-0042 decision 2), which is the mechanism
  built for exactly this. Handing the session's chain in as `chain` instead
  would have silently made the caller's own notes unreachable from every
  project session, which is the kind of thing that reads correct and is not.
  Lapses are deliberately not gathered here — a narrowing, never a widening —
  because a second relaxation path is Prompt 26's to build once. `inject.rs`
  was **not** refactored to share the orchestration: the prompt says leave the
  old routes untouched, and Prompt 18 re-cuts this endpoint's internals
  anyway.
- **Policy.** `SessionRead` and `SessionWrite`, a `Session` entity parented to
  the scope it runs at (`tenant` + `scope`, and deliberately **no principal**
  — the ownership distinction a pack might write is already expressed by
  *which scope the run was opened at*), and permits in all three packs,
  **@18 → @19**. Reading a run is priced with the **content** reads and never
  with `ProjectRead`: a project's name discloses nothing, and a run's timeline
  is a transcript of what somebody and their agent did, said, read and
  changed. The action applies to `[Scope, Session]` and nothing else — a
  listing is decided at the scope it is anchored at, each row as the session
  it is about — which is what lets every clause stay uniform over the union
  except the membership floor, written as two clauses per pack with the reason
  beside them: `principal in resource` walks *up* from the caller's own scope
  and can never reach a `Session`, which hangs *below* one.
- **Privacy holds without a new rule.** `SessionRead` and `SessionWrite` are
  **not** on the base layer's governance carve-out, so a run opened at
  somebody's own `principal` scope is unreachable by a tenant administrator —
  the one caller who reaches everything else. Asserted by name
  (`a_run_at_somebody_s_own_scope_is_not_the_administrator_s_to_read`), because
  the mistake worth pinning is somebody adding these two actions to that list.
- **One defect this feature's own tests found in its own code.**
  `SessionEventType` derived `serde(rename_all = "snake_case")` beside an
  `as_str()` of `message.user`, so the API **answered with one spelling and
  refused the other** — a request body naming `message.user` was a 400 quoting
  twelve names nobody would send. Four integration tests failed at once, which
  is the good version of that mistake. Fixed with per-variant renames and a
  unit test that walks every variant asserting serde and `as_str` agree; the
  original unit test had asserted the *divergence* and rationalised it, which
  is the more interesting failure and is recorded in the test that replaced it.
- **A second defect, and it was not this feature's.** `payload_hash` hashed
  `Value::to_string()` on the belief that `serde_json::Map` is a `BTreeMap`, so
  an event re-sent with its keys in a different order would have got a
  different digest. It is an `IndexMap` here: **`cedar-policy-core` enables
  `serde_json/preserve_order`**, and Cargo unifies features across a workspace.
  What makes this worth recording is where it led. CPR-4 had already written
  the same recursion inside the gateway's idempotency seam — with a comment
  saying it was a no-op "today", kept only against the day somebody turned the
  flag on. The flag was already on when that comment was written. The
  mechanism was right and its stated reason was wrong, which is the failure a
  comment is worst at catching. The canonicaliser now lives once, in
  `synveda_types::json`, both callers use it, the gateway's private copy is
  deleted and its comment corrected. The behaviour also **changes with the
  build's scope** — `cargo test -p synveda-types` has no Cedar in its graph and
  the two encodings match there, `cargo test --workspace` unifies the feature
  in and they do not — which is recorded in the module and is why nothing
  asserts the raw strings differ.
- **Two idempotency mechanisms, and the reason they are not redundant.**
  Opening a run and composing a context run each take a required
  `Idempotency-Key`. Appending events does not: its unit is the **event**,
  keyed by the client's own `client_event_id`, because a redelivered batch
  overlapping a previous one by three of ten must append seven and answer
  `duplicate` for three — at their *original* positions — and a request-level
  key cannot express that. A batch that repeats an id **inside itself** is
  refused by name, because the two would race for a position and one would
  silently become the other's duplicate.
- **Ordering is serialised per session, deliberately.** `append_events` takes
  the session's row lock before reading `max(sequence)`. The optimistic
  alternative is more code, is only faster when two clients append to *one*
  run at once, and has to get its retry right to be correct at all.
- **What the chain carries, and what it refuses to.** Four action types.
  `session.opened` records the run's shape and **`metadata_bytes`** — never
  the metadata, because an agent's environment is where credentials live and
  that bag is where a harness would put an environment. Asserted by putting a
  `ghp_`-shaped value in and sweeping the **whole** chain for it, not just the
  event that would obviously carry it. An append chains **one** event however
  many it carried, with counts, the sequence range and the per-type breakdown
  — so "what did that agent actually do" is answerable from the chain without
  reading the events, and a hundred-turn run is not written twice.
- **The listing decides per row against the row** (CPR-9's rule, from the
  start rather than retrofitted) and is **bounded and says so**: at most 500
  candidates, newest first, with `truncated` on the envelope. That is a
  different thing from the cap CPR-9 refused — a complete inventory of
  workspaces silently losing rows — because this is a recency-ordered feed of
  an unbounded event-like table where "the most recent N" is a well-defined
  answer. A tenant with **no governed scopes at all** is answered with an
  empty list rather than a denial: the Cedar action admits no `Tenant`
  resource (a run always happens somewhere), so there is nothing to decide
  about and nothing disclosed.
- **Console.** `Sessions.tsx` and `sessions.mts` — the first of CPR-8's four
  planned pages to get a plane behind it, and the first plane in this
  programme driven entirely through the **generated** client from day one
  rather than from Prompt 19. Opening a run fetches its timeline under its own
  cache key, because a transcript is the largest thing on this plane and the
  one a reader asks for least often. There is deliberately **no "start a
  session" button**: a run is opened by an agent from a harness, and a browser
  opening one would create a run that never ran. `Planned.tsx` lost its
  `sessions` entry, which is what that page is for — an entry there is a debt
  with a name, paid by deleting the entry rather than editing it.
- **Tests.** New: `crates/synveda-gateway/tests/sessions_api.rs` (**14**) —
  the whole path once (open → append → compose → timeline → two-phase close →
  refused), the identity rules a client may not submit, both idempotency
  mechanisms in one test because the pair *is* the design, every route
  refusing a caller who holds nothing, the project member who sees one run of
  two with the listing and the per-object route agreeing, another tenant's id
  answering exactly as a fictional one, the places a run may not be opened,
  the forward-only lifecycle, the metadata that never reaches the chain, the
  one-event-per-batch chain, the context run's watermark, the bare tenant, and
  the filters and their bounds. New in `crates/synveda-store/tests/rls.rs`
  (**5**) — cross-tenant blindness including a search for the victim's
  *transcript text*, the cross-tenant refusal, the missing UPDATE/DELETE
  privileges on the two append-only tables, the row rules against **direct
  SQL**, and the lifecycle end to end under RLS. New unit tests: 8 in
  `synveda_types::session`, 3 in `synveda_store::sessions`, 6 in
  `synveda_gateway::sessions`. Console: `sessions.test.mts` (**7**),
  121 → **128**.
- **Run record.** `make ci` **PASS** and `make db-test` **PASS**, the latter on
  a fresh scratch database over the whole workspace with no filter. Three
  things failed on the way and each is recorded above or here rather than
  smoothed over: the event-type spelling (four integration tests at once), the
  payload digest (one unit test), and — only under `db-test` — **CNSL-2's
  explorer parity corpus**, which is a recording of what the capability probe
  serves and now has two more actions in it. That last one is the reason
  `db-test` exists beside `ci`: the parity test skips without a database, so
  `cargo test --workspace` was green while a committed fixture disagreed with
  the gateway. Re-recorded with `SYNVEDA_RECORD_FIXTURES=1`, and the diff read
  before accepting it — exactly two lines, `session.read` and `session.write`,
  both `true` under a test pack that permits every action to a `viewer`. **No
  test was weakened**: the outsider's 403s, the cross-tenant 404s and every
  existing access-plane assertion are unchanged, and the listing and privacy
  properties were verified against them rather than around them. The demo also
  failed three times before it passed, and two of those were the demo's own
  fault (a grant body shape, and fixture events dated in the future); the
  third was not — see the seeding note below.
- **A third defect, in the demo's seeding, and it is not this feature's
  either.** A scope inserted by raw SQL has **no self-row in
  `scope_closure`** — closure maintenance is store code inside the caller's
  transaction with no trigger behind it (ADR-0011) — and the anchor resolver
  joins that table to find a grant. So the break-glass block CPR-7's demo
  established seeds a root scope whose administrator grant reaches nothing,
  and the first `POST /v1/workspaces` is a 403 quoting the pack. CPR-5's demo
  already knew this and seeds the closure row; CPR-7's does not. This demo
  seeds it, with the reason written where the next script to copy the block
  will read it. **`demos/cpr-7-scopes.sh` is left alone** — it is another
  feature's demo and fixing it is not this prompt's — and it is reported here
  so that the next prompt to touch it knows.
- **What this prompt does not carry.** The observe path is untouched: Prompt
  11 re-cuts it onto sessions and deletes `observe_events.session_id`. There
  is no CLI verb for this plane — the CLI re-cut is Prompt 24, and a run is
  opened by an agent rather than by somebody at a terminal. Candidates do not
  exist yet, so nothing a run produced is attributable to it beyond its own
  events. The context run holds a rendered block and a watermark and **not**
  the per-scope explainability Prompt 18 adds behind the same endpoint. And
  the 43 Phase-3 demos are still unchanged, for CPR-7's reason.
- **Commit.** `feat(sessions): add session ledger and runtime API` on
  `feat/context-platform-mvp`.
- **Commit hash.** `16b83e4249fd47eb2866cd57ac3c2bb5b0e55183`.

### Prompt 11 — The session product experience (CPR-11)

- **Divergence from §9, and from Prompt 10's own forecast.** §9's Prompt 11
  reads *"Candidate → knowledge promotion through VedaFlow"*; CPR-10's record,
  CLAUDE.md and `synveda_types::session`'s module note all say Prompt 11
  re-cuts the observe path and deletes `observe_events.session_id`. **The
  prompt as it arrived is neither.** It is the session *product* experience:
  the console surface over CPR-10's ledger, and the API work that surface
  needs to exist. §9's preamble makes a prompt's own text authoritative, so
  this is recorded rather than absorbed — and recorded more loudly than the
  earlier divergences, because this one **leaves a forecast standing in three
  files**. Two of the three are corrected here rather than left to read as an
  oversight — CLAUDE.md and `synveda_types::session`'s module note now say the
  observe re-cut is open and unscheduled. The **third is deliberately left
  standing**: the same sentence is in `0044_sessions.sql`'s header, and a
  migration's bytes are its checksum, so correcting a comment there would
  trade one stale sentence for a `VersionMismatch` on every database that has
  already run it. CPR-9 made the same call about three unreachable pre-epoch
  statements, for the same reason, and Prompt 33's squash is where both get
  cleaned up. CPR-10's entry above is also left exactly as written: a record of
  what a prompt believed is worth more than a tidy one.
  **The observe re-cut is unstarted.** `/v1/observe`, `/v1/inject` and
  `/v1/recall` are untouched, `observe_events.session_id` is still a `text`
  column, and nothing bridges the two models in either direction.
- **Implemented.** CPR-10 made a run a governed record. This makes it a record
  somebody with a question can use: keyset pagination and four more filters, a
  timeline that reports both clocks, a payload behind its own authority, an end
  reason, and the console pages over all of it — a filter bar, Load more, and a
  route per run.
- **Schema/domain changes.** Migration **`0045_session_end_reason.sql`**, one
  nullable column, 42 → **43** migrations. The epoch stays at **2**: an
  addition to a chain nothing has shipped against. `sessions.end_reason`, ≤ 500
  characters, and a CHECK that forbids one on an `active` row — a reason is
  part of a close, so a row carrying one while still running would be a state
  nothing wrote. It is **not** `task_summary`: that is what the run was
  *about*, set at open; overloading it would make the two indistinguishable the
  first time a client set both.
- **API and frontend changes.** 39 → **40** operations, 52 schemas, with
  `docs/api/openapi.json` and `console/src/generated/api.ts` regenerated from
  the handlers.
  - `GET /v1/sessions` — `cursor` in, `next_cursor` out, **`truncated`
    deleted**. Plus `client_name`, `principal_id`, `started_after`,
    `started_before`.
  - `GET /v1/sessions/{id}/timeline` — every event entry gains `received_at`
    and a server-computed `delayed`.
  - `GET /v1/sessions/{id}/events/{event_id}` — **new**, behind
    `SessionDiagnostics`.
  - `POST /v1/sessions/{id}/end` — takes `end_reason`; the view serves it and
    the chain carries it.
  - Console: `Session.tsx` (new), `Sessions.tsx` (re-cut), `sessions.mts`
    (re-cut), one level of `:param` in `routes.mts`/`Router.tsx`/`App.tsx`, and
    the styles for both pages.
- **The cursor follows the last candidate a page considered, not the last row
  it served.** This is the part worth reading twice. Rows on this plane are
  decided one at a time against the row (CPR-9), **after** they are scanned. A
  cursor on the last row *served* would re-scan every denied row between two
  served ones — and, worse, a page whose candidates were **all** denied would
  serve nothing, carry no cursor, and end the listing while readable rows sat
  below it. So `page()` walks the scanned candidates, keeps up to `limit` of
  them, and returns the key of the last one it looked at. The consequence is a
  shape clients must handle and the schema states: **a page may be empty and
  still carry a cursor.** The alternative — keep scanning until the page is
  full — is unbounded work driven by rows the caller cannot read.
- **A keyset, not an offset**, and the ordering moved with it: `started_at
  desc, id desc` rather than `started_at desc, id asc`, so the resume
  predicate is one row comparison the index can seek to rather than two
  disjuncts. An offset would skip and repeat whenever a run was opened between
  two requests, which on a table a fleet of agents writes to all night is every
  request.
- **Lateness is one flag and not three.** A locally spooled batch, a replay
  after a crash and a machine whose clock is an hour out produce **the same two
  instants**, and the server cannot tell them apart. So `delayed` reports that
  the gap exceeded a minute and the console reports the gap itself —
  "recovered or delayed — reached this deployment 1h 30m later" — and neither
  names a cause. Skew the other way is deliberately not late: a `received_at`
  earlier than the `occurred_at` it claims is something else, and calling it
  late would be a second wrong answer on top of the clock's. The threshold
  lives on the server so that "did not arrive live" means one thing across the
  console, the CLI and anything else that reads a timeline.
- **A payload is its own authority.** `SessionDiagnostics` — the fourteenth
  Cedar action on this programme's planes — because a timeline says *that* a
  message was sent and a payload is what was said, byte for byte. Permitted in
  all three packs, **@19 → @20**, and **strictly narrower than each pack's own
  `SessionRead`**: `regulated-strict` and `standard` take a governance key
  (`reviewer`, `owner`, `administrator`) where a timeline also admits `viewer`,
  `curator` and `member`, and `standard` deliberately does **not** extend it by
  `principal.ambit` — sharing one step outward is a decision about a reading
  surface, and a neighbouring project's raw prompts are not a default under any
  pack. `open-collaboration` reads runs tenant-wide *role-free* and requires
  any grant at all here, so the narrowing is real even there. It is **not** on
  `base.cedar`'s governance carve-out, so personal-scope privacy reaches it
  exactly as it reaches the other two, with no new rule.
  The split is asserted with **one caller holding one half and not the other**
  (`a_payload_takes_diagnostics_and_a_timeline_does_not`), which is the only
  form of that assertion that means anything — and the same test walks the
  timeline asserting no entry carries payload text, which is what fails the day
  somebody adds one "for convenience".
- **What the chain carries.** The diagnostic read chains one
  `authz.decision` naming the event, its type, its sequence and its payload
  **digest** — and never the payload. An audit log that copied every prompt
  somebody read would be a second, unbounded transcript store with weaker
  access rules than the first. Asserted by putting `hunter2` in an event,
  expanding it, and sweeping the whole chain for the string.
- **Console.** A run has an address. CPR-8's route table was flat literals;
  this adds one level of `:param`, `matchRoute` returns `{ id, params }`, and
  `hrefOf` throws on a placeholder nothing filled rather than emitting a
  literal `:session_id` that 404s on click. The detail page reads the id out of
  the address bar and from nowhere else, so Back, refresh and a pasted link all
  land on the same run. The payload control is offered from the caller's
  forecast **at that run's own scope** when `/v1/me` reported one — a caller
  may hold the plane in one project and not another, and the tenant-wide figure
  would render a control that 403s in half the places it appears. Nothing is
  fetched until the control is clicked.
- **Two things the console does that are easy to get wrong.** The accumulated
  page list is computed **in render** from `seen + this page`, not pushed into
  state from an effect — an effect that appends runs twice under StrictMode and
  shows every row twice. And `appendPage` de-duplicates by id, because a reader
  who clicks Load more twice before the first answer lands sends one cursor
  twice and is served one page twice.
- **Deleted.** `SessionList.truncated` — gone from the response, not kept
  beside `next_cursor`, and a test asserts its absence rather than only
  `next_cursor`'s presence. `synveda_store::sessions::list`'s ascending
  tiebreak. CPR-10's expander-based session row (`ul.sessions button.row` and
  the in-place `Timeline` inside `Sessions.tsx`), replaced by a link and a
  route. `matchRoute`'s `RouteId | null` return.
- **Tests.** New in `crates/synveda-gateway/tests/sessions_api.rs` (**7**,
  21 total) — the whole pagination walk rather than one hop (a cursor that
  repeats a row, skips one or never clears only shows in a full traversal),
  the three bad cursors, the four filters with the exact-match and inverted-window
  cases, the end reason through the API and into the chain and over its bound,
  both clocks with a two-hour-late event beside a live one and a context run
  that carries neither, the warning in `event_counts` and in its entry, the
  payload split with one caller, and an event id from another run answering
  exactly as a fictional one. New in `console/src/sessions.test.tsx` (**9**) —
  the six scenarios the prompt names, plus the two empty-list sentences and the
  Load more affordance, all through `renderToStaticMarkup` over a primed cache
  so no request is ever made. 12 new derivations in `sessions.test.mts`, the
  parameterised route in both directions in `routes.test.mts`, and the
  operation count in `client.test.mts`.
- **Run record, and it has one honest gap.** `make ci` **PASS** — every step,
  including `cargo test --workspace` with no `DATABASE_URL` (the step CI
  actually runs), `fmt`, `clippy -D warnings`, `deny`, `check-deps`,
  `check-api-types`, `check-backlog` (108 features agree), `check-adr-status`,
  `check-corpus-licences`, `check-chart-images`, `check-benchmarks`,
  `check-ann-bench`, `chart-lint`, `eval-check`, `ts-build`,
  `check-npm-licences` and `ts-test` (console **149/149**, adapter 74/74).
  `crates/synveda-gateway/tests/sessions_api.rs` against a live Postgres:
  **21/21**.
  The DB-backed suite ran **571 passed, 1 failed**, and the one failure is
  named here rather than smoothed over: **CNSL-2's explorer parity corpus**
  (`console/fixtures/explorer`), which is a *recording* of what the capability
  probe serves and now has one more action in it — `session.diagnostics`, from
  `Action::PROBED_AT_SCOPE`. It is the same expected re-record CPR-10 hit for
  `session.read`/`session.write`, and it is closed by one command:
  `SYNVEDA_RECORD_FIXTURES=1 make db-test`, reading the diff before accepting
  it — it should be exactly one line per recorded `actions` map.
  **It is not closed in this commit, and the reason is environmental.** The
  Docker daemon on the machine this ran on wedged partway through: every
  `docker` call hung, and the Postgres container went with it — the port kept
  accepting TCP and the backend stopped answering queries, which is why the
  571/1 figure comes from running the suite directly against a freshly created,
  fully migrated database rather than through `make db-test` (that target
  shells out to `docker compose exec` for its scratch database). The fixture
  was **not** hand-edited to the bytes it will have: a corpus written by hand
  is exactly the drift the parity test exists to catch, and asserting a value
  nobody observed would be worse than a red test somebody can see.
- **No demo script.** CPR-9's precedent: a prompt whose acceptance criteria are
  discharged by tests does not need one, and a new demo could not have been run
  against a live stack here anyway. The 30 acceptance assertions live in
  `sessions_api.rs` and `sessions.test.tsx`.
- **Left standing, deliberately.** `demos/cpr-7-scopes.sh`'s missing closure
  row (CPR-10 reported it and left it; still another feature's demo). The
  observe path, whole. `console/src/api.mts`'s seven hand-written surfaces,
  until Prompt 19.
- **Commit.** `feat(console): add session timeline` on
  `feat/context-platform-mvp`.
- **Commit hash.** `6150f34feab9cd0be1a748f9c51d9bdb2d41abd3`, written by
  Prompt 12 on Prompt 1's rule.

### Prompt 12 — Durable Claude session delivery (CPR-12)

- **Divergence from §9.** §9's Prompt 12 reads *"Redaction and secret scanning
  re-anchored on the session/candidate path"*. The prompt as it arrived is the
  **adapter cutover and durable delivery**, which subsumes it: the scan seam
  moved onto `session_events` as part of the move, so the §9 item is delivered
  inside this one rather than skipped. What this prompt adds beyond §9's
  forecast is the durable spool and the deletion of the three global routes —
  work §9 had spread across its Prompt 8 (*"session events and the observe path
  onto sessions"*), which CPR-11's record left open and unscheduled. **That gap
  is now closed.** §9's Prompts 9, 10 and 11 — candidates, knowledge versions
  and promotion — remain open and unstarted.

- **Implemented.** The Claude Code integration moves wholly onto the session
  API, and delivery becomes durable.

  The spool is a versioned envelope per run under
  `$XDG_CONFIG_HOME/synveda/spool/`, carrying spool version, client
  installation id, Synveda session id and, per entry, client event id,
  sequence, event type, occurred time, payload, payload hash, delivery
  attempts, last attempt time and acknowledgement state. Every write is
  temp-file → `fsync` → `rename`, so a kill mid-write leaves the old file or
  the new one and never half of either. The previous format has **no reader**.

  Hooks own delivery. `SessionStart` opens or resumes a run and retries the
  unacknowledged backlog; `Stop` records the turn and starts a delivery within
  a 2s budget; `PreCompact` records so a compaction does not swallow the turn;
  `SessionEnd` performs a bounded synchronous flush within 3s. Whatever does
  not go stays spooled and the next `SessionStart` retries it.

  The CLI diagnoses and does not deliver on a schedule: `synveda session
  flush`, `synveda session spool status`, `synveda session spool purge
  --acknowledged`.

  Context injection is `POST /v1/sessions/{id}/context-runs`.

- **Schema/domain changes.** Migration **`0046_session_ingestion.sql`**,
  43 → **44** migrations. The epoch stays at **2**: this chain has shipped
  against nothing.

  Added: `memory.asserted` to `session_events_type_check`;
  `session_events.redactions` with an array CHECK; a
  `session_events_tenant_id_unique` constraint so the quarantine table can
  carry a composite foreign key; the PGMQ queue `session_events`;
  `session_event_quarantine`, with composite foreign keys to `session_events`,
  `sessions` and `scopes` so a quarantined row cannot outlive or drift from
  what it is about; a one-shot review trigger; guarded-delete triggers behind
  the `synveda.retention_purge` transaction-local flag; and
  `session_context_runs.skills`.

  Rebuilt: `audit_log_disclosure_idx`, to include `session.context.composed`.

  Dropped: `observe_events`, `observe_quarantine`, the
  `synveda_observe_quarantine_*` functions, and the `observe` PGMQ queue.

  In types: `SessionEventType::MemoryAsserted` and `carries_memory()`, which
  decides which of the thirteen types enqueue extraction work — seven do.
  `ObserveKind` is deleted; `synveda-types/src/observe.rs` is now
  `quarantine.rs` and keeps only `QuarantineState`.

- **API and frontend changes.** Deleted: `POST /v1/observe`, `POST /v1/inject`,
  `POST /v1/recall`. No route was added — the session plane already had the
  seams, which is the point of ADR-0076 decision 7 having declared the context
  run final. The OpenAPI document and `console/src/generated/api.ts` are
  regenerated; the contract stays at **40 operations** because three
  deletions were three routes never on it.

  `synveda recall` and the `recall` MCP tool now compose a context run and say
  in their own help what they lost: the by-id tier and the bitemporal read.

- **Deleted.** `crates/synveda-gateway/src/{observe,inject,recall}.rs`;
  `crates/synveda-store/src/observe.rs`;
  `crates/synveda-store/tests/observe_queue.rs`;
  `crates/synveda-gateway/tests/recall.rs`;
  `adapters/claude-code/src/flush.mts`; `demos/ctx-5-recall.sh`; and roughly
  270 lines of recall-only gathers from `authz.rs`.

  Renamed rather than deleted, because their subject survived the move:
  `tests/inject.rs` → `context_runs.rs`, `tests/observe.rs` →
  `session_ingest_load.rs`, `tests/observe_redaction.rs` →
  `session_redaction.rs`, `tests/tiered.rs` → `index_tier.rs`,
  `tests/inject_latency.rs` → `context_run_latency.rs`.

- **Tests.** Full Rust workspace **1,676 passed, 0 failed, 9 ignored**.
  `synveda-eval` 159 passed. Adapter 87 passed. `cargo fmt --check` and
  `clippy -D warnings` clean. `.sqlx` regenerated (42 written, 44 removed).

  New: `crates/synveda-cli/src/spool.rs` — 13 unit tests over the format, the
  canonical hash and the atomic write. `crates/synveda-gateway/tests/
  session_seed.rs` — a shared fixture harness, included by `#[path]` rather
  than published, because a test helper that becomes a module becomes an API.

- **Three findings worth the record.**

  1. **A 1.9× regression on a stated SLO, and a wrong first hypothesis.** The
     append seam missed MEM-1's <20ms ack budget at 35.6ms after the move. The
     first hypothesis was a per-session row lock — plausible, because the new
     seam serialises per run where the old one did not. It was **wrong**:
     spreading the load across many runs moved the median 0.3ms. The cause was
     per-row inserts, one statement per event where the old path batched. Three
     statements — pre-filter existing client event ids, one `unnest` insert
     assigning contiguous sequences, read back — put it at 17–19ms. The wrong
     hypothesis is recorded in the test's own comment, because the next person
     to see this number will have the same idea.

  2. **A pre-existing security hole in CPR-11's timeline.** CPR-11 shipped a
     unit test asserting a timeline summary **is** the message text and an
     integration test asserting a timeline carries **no** payload text. Both
     were green, because they exercised different paths — and the one that was
     passing on the real path was the wrong one. Messages now summarise as
     `message.user (N characters)`. Two further CPR-11 tests could never have
     passed: one asserted 200 where a fresh context run answers 201, and one
     sent `+00:00` unencoded in a query string, where it decodes to a space and
     is a 400.

  3. **45 of the 67 shell scripts under `demos/` were dead**, and had been
     since CPR-7 — three prompts ago. That
     prompt deleted `synveda role bind`, `synveda hierarchy` and
     `/v1/hierarchy/*` whole — correctly — and re-pointed the code, the tests,
     the CLI and the docs. It did not re-point the demos, and **nothing said
     so**: four prompts have recorded clean runs since, because no gate runs a
     demo. 28 of CPR-12's own 32 observe/inject/recall demos are inside those
     45, so re-pointing their call sites would produce scripts still dead one
     command earlier. Filed as **CPR-13**, whose larger half is the gate.

- **Left standing, deliberately, and each with a reason.**

  - **Three eval suites fail by name rather than measure.** The extraction,
    security and QA-index suites enumerated a corpus through `/v1/recall`'s
    **sweep**. A context run cannot stand in: it ranks and budgets where a
    sweep enumerates, so what it left out would be a property of the budget
    rather than of extraction. The committed lens (`GET /v1/audit/events?
    action=memory.extracted`) cannot stand in either — the chain carries
    per-event *counts*, not the record text every per-class score reads. Their
    seed legs are fully re-pointed onto the session plane and their sweep legs
    return a named refusal, so `make eval-extraction-live` and
    `make eval-security` fail with the reason rather than reporting a number
    measured against a different question. `make ci`'s `eval-check` is
    parse-only and is green. Prompt 18 re-cuts recall; Prompt 32 re-measures.
  - **`RecallSweepRequest` and `RecallIdsRequest` are kept as tombstones** in
    the eval client with no route behind them. Deleting them means deleting
    three suites' worth of EVAL-2/4/5 structure for a surface that returns at
    Prompt 18.
  - **The demo corpus, minus five.** CPR-12 fixed the four demos that were
    live and deleted `ctx-5-recall.sh`. See CPR-13.
  - **`console/src/api.mts`'s seven hand-written surfaces**, until Prompt 19.

- **Commit.** `feat(adapter): make Claude session delivery durable` on
  `feat/context-platform-mvp`.
- **Commit hash.** `a065830349eff6c7ebac9b8e979e479690c8419b`, written by
  this acceptance prompt on Prompt 1's rule.

### External acceptance objective — Live Claude Code session gate (CPR-14)

- **Numbering and scope.** The external handover called this the next prompt;
  the repository already reserves **CPR-13** for the demo-corpus re-point, so
  the next free feature id is **CPR-14**. No CPR-13 work is included here. The
  candidate, Knowledge, New Learnings, explainable-context, skills, MCP-registry,
  OKF and graph objectives remain unstarted.

- **State: implementation and replay complete; live-client acceptance
  pending.** CPR-12 proved adapter functions and the new server seams. This
  adds the missing acceptance join, split into the three evidence tiers
  ADR-0079 names: captured/mock, replay/live-gateway, and live-client. A lower
  tier never reports the one above it. CPR-14 therefore remains open and the
  delivered count stays at 76 until an authenticated real client completes the
  last tier.

- **Authentic fixture contract.** `adapters/claude-code/fixtures/manifest.json`
  binds every committed hook frame and transcript to a genuine Claude Code
  capture, its exact version, capture provenance, a sanitisation declaration
  and SHA-256. The corpus covers **2.1.220** and **2.1.241**. A committed schema
  and adapter test check the manifest shape, complete on-disk coverage, hashes,
  version fields, synthetic paths and a credential/personal-path denylist.
  Raw live capture is opt-in through an absolute `SYNVEDA_CAPTURE_DIR`, writes
  a 0700 path and 0600 frames, logs only the event name and byte count, and
  lives inside the live runner's disposable scratch directory.

- **Adapter seams closed.** `project_id` joins `workspace_id` in
  `.synveda/config.json`, `SYNVEDA_PROJECT` and the recorded-payload driver's
  `--project`: project list order is not an identity, and a session that should
  compose at a project could not previously say which one. The configured
  pair is stored before the first open; once the server answers, its stored
  placement wins. Authentic 2.1.241 transcript bytes also found that a
  `tool_use` block emitted no `tool.invoked` event — only the later result was
  kept. Invocation ids are now stable, payloads bounded, and the user,
  invocation, result and assistant event families are all preserved.

  The Node adapter and Rust CLI now create every spool path component at 0700
  and every file/temp file at 0600 independently of umask. The previous code
  constrained the final file on one path and inherited ambient permissions for
  parent directories and Node temporary files.

- **Replay/live-gateway acceptance.** `crates/synveda-gateway/tests/
  claude_lifecycle.rs` runs the built `node dist/hook.mjs <mode>` child over the
  authentic frames against an epoch-2 database and the real gateway. Tenant
  admission uses the system seam; production JIT provisioning with the
  `synveda-admins` group mints the principal scope and first administrator
  grant; workspace and project are public HTTP creates; and the non-empty seed
  block is produced by a public seed session/event plus the real ingestion
  worker. No governed application table is inserted directly.

  The first SessionStart opens one project-scoped run and persists a context
  run. Stop appends user, tool invocation, tool result and assistant activity.
  The gateway then disappears after the next two events are durable: four
  entries are acknowledged and two remain pending with attempts and local
  SHA-256s intact. After restore, the test appends the first pending event
  through the public route but deliberately leaves the local acknowledgement
  unchanged — a server commit with a lost answer. The next SessionStart sends
  the overlapping two-event batch, receiving `duplicate` at original server
  sequence **5** and `appended` at sequence **6**. Six unique client event ids
  produce six rows exactly once; the server's own BLAKE3 hashes are present.
  Normal headless SessionEnd drains and closes the run with reason `other`.

  The timeline is six events plus two context runs, in server order. Its
  summaries carry type and length rather than transcript text, and the exact
  gateway response is the golden fixture the console renders. The audit chain
  contains the session actions and verifies; a sweep over every nested payload
  finds none of the fixture's user, assistant or tool content. CLI spool tests
  separately prove `purge --acknowledged` removes acknowledged entries and
  retains pending ones.

- **Live runner, and the result is not a live run.** `make
  claude-acceptance-live` creates isolated HOME/Claude/Synveda/XDG state,
  builds the adapter and CLI, packages plugin **0.2.0** through
  `scripts/package-plugin.sh`, installs through the real `synveda plugin
  install` → `claude plugin` path, asks Claude Code itself for the enabled
  plugin/four hooks/one MCP server, and would then invoke deterministic real
  `claude -p`, inspect persistence/audit and write a content-free version report
  under `target/`. It cleans its temporary credentials, config and captures.

  On 2026-08-23 the executable resolved on `PATH` as **Claude Code 2.1.241**,
  but `claude auth status` answered `{"loggedIn":false,"authMethod":"none",
  "apiProvider":"firstParty"}` and no isolated-run credential was set. The
  runner printed that prerequisite and exited **77 before packaging or invoking
  a session** (`make` reported recipe `Error 77` and exited 2). The real
  executable did not run a session; the plugin was not installed into the
  isolated HOME; no live result or live timing exists.

- **Versions.** Replay fixtures: Claude Code **2.1.220** and **2.1.241**.
  Installed but unauthenticated live prerequisite: Claude Code **2.1.241**.
  Plugin and Synveda workspace: **0.2.0**. Node **v24.18.0**, pnpm **11.13.1**,
  rustc/cargo **1.96.0**. Host: **macOS 26.5.2 (25F84)**, Darwin **25.5.0**,
  arm64.

- **Schema and API.** **No migration, epoch change, Cedar action, audit action,
  public HTTP route or OpenAPI change.** The public session contract remains 40
  operations. The changed contracts are the adapter's optional `project_id` /
  `SYNVEDA_PROJECT`, the versioned fixture manifest/schema, the private raw
  capture opt-in, `make claude-acceptance`, `make claude-acceptance-live`, and
  CI's explicitly named Postgres-backed `claude-replay` job.

- **Measurements.** On the final replay/live-gateway run: SessionStart **81ms**,
  Stop **62ms**, SessionEnd flush/close **54ms**, append **9ms**, two context
  runs **15ms** and **12ms**, bounded backlog recovery **70ms**. Every value is
  below its existing ceiling (8s start, 5s Stop, 8s SessionEnd, configured
  request deadline and 2s recovery budget); no limit moved. The dedicated
  1,000-event append load test remains green against its <20ms product SLO and
  completed in **10.52s** wall time.

- **Security checks.** Session wire deserialisation still refuses tenant and
  acting-principal fields; scope is still server-derived. The focused session
  suite proves a project member sees only that project's runs, a cross-tenant
  id is the same 404 as fiction, and somebody else's principal scope remains
  private. Timeline summaries contain no message text; diagnostics remain a
  separate `SessionDiagnostics` decision. Fixture, log and whole-audit-payload
  sweeps find no credentials or transcript content. Spool and capture modes are
  asserted 0700/0600. Setup uses supported identity/product/session paths and
  no test policy pack or direct governed-table mutation bypasses the PDP.

- **Tests and exact results.** The inherited handover tree's first baseline
  `make ci` stopped at `cargo fmt --check` on its unfinished
  `claude_lifecycle.rs`; the first `make db-test` could not reach Docker. After
  preserving and completing that work, OrbStack/Postgres became available.
  Focused results: adapter **96 passed, 0 failed**; CLI spool **10/10**; console
  **150/150**; audit/context/extraction/load/redaction/session gateway suites
  **43/43**; CPR-14 replay **1 passed**, live **1 ignored**. Final `make ci`
  **PASS** (Rust workspace including doctests **1,679 passed, 0 failed, 10
  ignored**; console **150/150**; adapter **96/96**, plus every repository
  checker). Final `make db-test` **PASS** on scratch database
  `synveda_test_8147`, with the same **1,679 passed, 0 failed, 10 ignored**;
  the scratch database was dropped by the harness. The first non-escalated CI
  rerun's two CLI failures were the managed sandbox refusing loopback binds
  with `EPERM`; the permitted rerun is the recorded product result.

- **Limitations and remaining work.** The host-killed-before-any-hook boundary
  is unchanged: that in-flight tail may be lost, and nothing here claims
  otherwise. A genuine authenticated installed-client run is still required to
  close CPR-14, replace the no-live-session support statement, and record its
  version/timings. Live Entra/Okta and Cursor evidence remain absent. The three
  intentionally blocked eval suites still fail by name and were not changed to
  measure a different question. CPR-13 remains untouched. The next bounded
  objective is therefore **run `make claude-acceptance-live` with a current
  isolated Claude credential and review the persisted version report**; it is
  acceptance completion, not candidate or Knowledge work.

- **Commit.** `test(adapter): verify live Claude session lifecycle` on
  `feat/context-platform-mvp`.
- **Commit hash.** `02b986ba68c9a867abdd9aa5c2746740669fa6d2`, written by
  the live-closure continuation below on Prompt 1's rule.

### External acceptance objective — CPR-14 live-client closure

- **Selected feature and state.** **CPR-14**, because CPR-13 remains reserved
  for the demo-corpus re-point. The installed-client tier passed on
  2026-08-24, so CPR-14 is delivered. The defect it exposed is exactly
  ADPT-8's subject, and the fix meets all four of that feature's criteria; it
  closes at the same time. The repository is therefore **111 features filed,
  78 delivered**. Candidate extraction, Knowledge, New Learnings,
  explainable context, skills, the MCP registry, OKF and graph work remain
  unstarted.

- **Implementation.** The live runner checks native Claude authentication with
  `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and
  `CLAUDE_CODE_OAUTH_TOKEN` unset, so an exported stale credential cannot
  shadow a valid native login. On macOS it transfers only the default
  `Claude Code-credentials` Keychain payload into the isolated Claude profile
  through a private 0600 `mktemp` file, then removes it on success, failure or
  signal. The Rust harness already removes isolated HOME, Synveda config and
  raw captures from `Drop`, including assertion failures.

  The authentic invocation now places the prompt before variadic
  `--allowedTools`, uses the stable JSON output contract, advertises only Read
  with `--tools Read --allowedTools Read`, and uses no permission bypass. Its
  failure report is content-free: status, category, safe envelope field names,
  enum/numeric fields, byte lengths, SHA-256s, denial counts and safe tool
  names. A test proves a credential error containing private result text and a
  private tool input is classified without either string appearing.

  The lifecycle fix amends ADR-0027 and ADR-0078. Claude Code's successful
  headless teardown kills unfinished async hooks. Stop and PreCompact are now
  synchronous only through transcript conversion plus atomic local spool save;
  they return before credential resolution or network I/O. SessionEnd owns the
  bounded flush/close, with the next SessionStart and explicit CLI flush as
  recovery. The live child sets
  `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS=8000`: Claude Code's default overall
  SessionEnd budget is 1.5s and a plugin hook's own timeout does not raise it,
  while the adapter's existing flush remains bounded at 3s. Adapter logs add
  content-free per-hook and per-append durations.

- **Live evidence.** The real installed authenticated executable ran; no hook
  function or manufactured invocation substitutes for it. Claude Code
  **2.1.241** installed plugin **0.2.0** through `synveda plugin install`'s
  marketplace path and reported `synveda@synveda` enabled, **four hooks** and
  **one MCP server**. One deterministic real Read turn emitted SessionStart,
  Stop and SessionEnd frames, opened exactly one project-scoped Synveda run,
  composed exactly one context run, persisted four ordered
  `message.user`/`tool.invoked`/`tool.result`/`message.assistant` events, flushed
  the final tail and ended with reason `other`. The timeline, separately
  authorised diagnostic payload, client ids, local SHA-256s, server BLAKE3
  hashes, acknowledgement state, sequence order and verifying audit-chain
  assertions passed. Summaries and audit/log evidence carry no message text.

- **Replay and outage evidence.** The schema-validated authentic 2.1.220/
  2.1.241 fixture corpus remains the ordinary-CI tier. Stop first leaves four
  entries durable and unacknowledged with `delivery_attempts = 0` and no event
  request; a supported next SessionStart drains them. A second two-event turn
  reaches the spool before the gateway is stopped. The private 0700/0600 spool
  remains intact and unacknowledged during the outage. After restore, the
  harness commits the first pending event through the public append route but
  deliberately loses the local acknowledgement; the next SessionStart sends
  the two-event overlap and receives `duplicate` at original sequence **5**
  plus `appended` at **6**. Six client ids produce six rows exactly once. CLI
  spool tests prove acknowledged purge removes acknowledged entries while
  retaining pending ones.

- **Versions.** Live: Claude Code **2.1.241**, plugin **0.2.0**, Synveda
  **0.2.0**, Node **v24.18.0**, pnpm **11.13.1**, rustc/cargo **1.96.0**.
  Host: macOS **26.5.2 (25F84)**, Darwin **25.5.0**, arm64. Replay provenance
  remains Claude Code **2.1.220** and **2.1.241**.

- **Schema, API and contracts.** **No migration, epoch, Cedar action, audit
  action, public HTTP route, OpenAPI or generated-client change.** The private
  adapter contract changes are: Stop and PreCompact registrations no longer
  carry `async: true`; both end at the atomic spool; SessionEnd/next start own
  delivery; the live runner has isolated native-credential handoff and the
  host SessionEnd budget; and the acceptance report/log carries content-free
  hook, append and context durations.

- **Defects discovered.** The variadic `--allowedTools` option consumed the
  prompt when it followed the option; `--include-hook-events` is not valid with
  JSON output; broad bypass flags were unnecessary; an exported invalid Claude
  credential shadowed a valid native login; isolating either HOME or
  `CLAUDE_CONFIG_DIR` changed the Keychain namespace in 2.1.241; the first
  diagnostic classifier matched the ever-present field name
  `permission_denials` and labelled unrelated failures as permission errors;
  Claude's result envelope can say `subtype=success` and `is_error=true` with
  zero permission denials; the host's SessionEnd budget is independent of the
  plugin timeout; and the live run resolved ADPT-8's old ambiguity — async
  Stop did not complete under successful `-p`, leaving the run active with
  zero events. One model invocation returned a transient generic API error;
  the unmodified retry passed. After the boundary moved, one replay assertion
  still queried immediately after Stop; the corrected test uses the supported
  next-SessionStart delivery path.

- **Measurements.** Final live: client **5,526ms**, SessionStart **72ms**,
  local-only Stop **8ms**, SessionEnd flush/close **53ms**, append **28ms**,
  context run **15ms**. Final replay: SessionStart **76ms**, Stop child process
  **31ms**, SessionEnd **55ms**, append **10ms**, context runs **15/13ms** and
  bounded backlog recovery **72ms**. No ceiling moved. The live append was a
  single cold sample, so the dedicated release gate was rerun: **10,000 events
  in 9.91s (1,009/s), ack p50 13.09ms, p95 15.75ms, p99 17.05ms**, with a
  165.88µs link baseline and 22.16ms local budget. It passed, so the 28ms live
  observation is not a steady-state regression.

- **Security.** Session bodies still cannot supply tenant, acting principal or
  governed scope. The focused suite proves project-row decisions,
  cross-project filtering, cross-tenant 404 equivalence and personal-scope
  privacy. Timeline summaries carry types and lengths rather than message
  text; raw diagnostics remain behind `SessionDiagnostics`; fixture and audit
  sweeps find no credentials/private content; spool, capture and credential
  handoff permissions are restrictive; logs/report carry ids, counts, hashes
  and durations rather than bodies; and setup uses production JIT plus public
  workspace/project/session paths with no direct governed-table mutation or
  test-policy PDP bypass.

- **Tests and exact results.** Adapter **96/96**; CLI spool **10/10**; console
  **150/150**; focused session/ingestion/redaction/context/audit/timeline/
  lifecycle gateway set **63 passed, 0 failed, 1 live ignored**; deterministic
  replay **1 passed**; installed-client live **1 passed**; release append load
  **1 passed** with the measurements above. `make ci` **PASS**: Rust workspace
  plus doctests **1,680 passed, 0 failed, 10 ignored**, adapter **96/96**,
  console **150/150**, and every checker. `make db-test` **PASS** on
  `synveda_test_21449` with the same **1,680 passed, 0 failed, 10 ignored**;
  the successful scratch database was dropped. The first replay after the
  Stop-boundary change failed one stale immediate-delivery assertion and kept
  `synveda_test_20275`; this was a harness failure, not lost product data.

- **Limitations and remaining work.** The real proof is exact to Claude Code
  2.1.241; future client versions run the same separate gate. A host killed
  before any lifecycle hook writes the in-flight turn can still lose that
  tail. Live Entra/Okta and real Cursor evidence remain absent. The three
  intentionally blocked eval suites were not changed to measure another
  question. CPR-13 remains untouched. The next bounded objective is
  **CPR-13, the demo-corpus re-point and `make check-demos` gate**.

- **Commit.** `fix(adapter): preserve headless Claude turns` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the next prompt on Prompt 1's rule.

### Prompt 13 objective — Versioned Knowledge aggregate (CPR-15)

- **Selected feature and state.** **CPR-15**, because CPR-13 remains reserved
  for the demo-corpus re-point and CPR-14 was consumed by the live Claude
  acceptance gate. The package is delivered. It is the persistence boundary
  locked by ADR-0068, not a rename or synchronisation of the old `records`
  aggregate.

- **Decision.** ADR-0080 separates stable item identity, immutable content,
  independently authorised provenance and explicit relationship claims.
  `knowledge_items` is a bitemporal aggregate head; an optimistic head
  precondition moves it between immutable `knowledge_revisions`. Valid time
  belongs to content, transaction time belongs to the database, and the head's
  current/history interval records when a revision or lifecycle was current.
  Sources are many-to-many and scoped independently so a shared result cannot
  disclose a private session or locator. The canonical BLAKE3 envelope hashes
  integer confidence, normalised tags and recursively ordered metadata rather
  than ids or database timestamps.

- **Schema and domain.** Migration `0047_knowledge` takes the chain from 44 to
  **45 migrations** without changing schema epoch **2**. It creates six
  tenant-bound, enabled-and-forced-RLS tables: `knowledge_items`,
  `knowledge_items_history`, `knowledge_revisions`, `knowledge_sources`,
  `knowledge_revision_sources` and `knowledge_relations`. It adds
  `knowledge_item_versions` and `knowledge_current` as `security_invoker`
  views, a stored language-neutral lexical document and its GIN index,
  tenant-qualified foreign keys, a deferred every-revision-has-source
  constraint, and owner-level append-only guards. A session-event source must
  name a real event whose session-derived scope is exactly the source scope.
  Rust types cover nine Knowledge types, four origins, six lifecycle states,
  seven source types and eight relation types, all closed and schema-matched.

- **Store seam.** Instrumented transaction-scoped primitives create sources,
  items and first revisions, append with an expected-current precondition,
  transition lifecycle, read current/as-known/history, filter sources by the
  already PDP-authorised scope set, and add/read relations. New write paths
  carry tracing spans and `synveda_knowledge_mutations_total`. No public
  route, Cedar action or audit action exists yet, so the primitives are not a
  second application service and no caller can publish around CPR-16.

- **Hard-cut boundary.** No code reads, copies, translates or dual-writes an
  old record. ADR-0080's deletion checklist names the record tables, store
  entry points, direct extraction commit, DTO/browser terminology and
  record-shaped query branches that CPR-16/17 must remove once governed
  mutations and public reads replace them. Semantic vectors remain on the old
  aggregate until the Knowledge retrieval cutover; no pretend embedding was
  attached to a revision.

- **Security and correctness evidence.** Five Postgres acceptance tests prove
  immutable revision and relation rows, exact current and as-known projection,
  stale-revision rejection, all seven sources and eight relations, separately
  filtered private provenance, session-event scope-confusion refusal,
  lifecycle history and cross-tenant invisibility across all six tables and
  both views. The dynamic RLS inventory includes every new object. Five type
  tests cover closed vocabularies, validation and canonical hashing. Creating
  Knowledge leaves every old record table unchanged.

- **Runnable evidence and findings.** `demos/cpr-15-knowledge-aggregate.sh`
  runs the focused database and RLS evidence in a disposable database and
  removes it. Its first run against the persistent development database was
  correctly refused because that database still records removed migration
  `0009`; the demo was changed to isolation rather than resetting somebody's
  state. A first full database run found a test-fixture error: its two
  supposedly equivalent canonical hash inputs had different `Utc::now()`
  values. Cloning the input before reordering metadata made the assertion test
  only the intended property; no product assertion was weakened.

- **Tests and exact results.** Focused type tests **5/5**, focused store
  database tests **5/5**, RLS completeness **PASS**, and the isolated demo
  **PASS**. `make db-test` **PASS**. `make ci` **PASS**, including Rust,
  clippy `-D warnings`, SQLx offline metadata, dependency/licence/ADR/backlog
  checks, console and adapter suites. The first managed-sandbox CI invocation
  could not bind loopback for two existing CLI tests (`EPERM`); the permitted
  rerun is the product result.

- **Limitations and next work.** This package intentionally cannot mutate or
  read Knowledge through a supported application API. It has no VedaFlow or
  audit event to bypass: CPR-16 is the only planned entry point and must add
  create/edit/verify/supersede/merge/archive/restore/forget, durable erasure
  work and the deletion of direct old record mutation paths. CPR-17 then owns
  public reads/search/browser and completes the old record cutover. Live
  Entra/Okta and authentic Cursor evidence remain unrelated external gaps.

- **Commit.** `feat(knowledge): add versioned knowledge aggregate (CPR-15)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-16 checkpoint on Prompt 1's rule.

### Prompt 14 objective — Governed Knowledge mutation lifecycle (CPR-16)

- **Selected feature and state.** **CPR-16** is delivered. It is the one
  governed mutation seam over CPR-15's aggregate, not a Knowledge-specific
  proposal inbox or a rename of record promotion. The preceding CPR-15
  feature commit is `874aa51`.

- **Decision.** ADR-0081 extends the existing VedaFlow proposal vocabulary
  with `AssetKind::Knowledge`, row-effect `apply` and terminal `applied`.
  Every command stores an erasable canonical typed payload in
  `knowledge_changes` and an immutable content-free manifest in VedaFlow; the
  manifest binds command kind, stable ids, expected revisions and a BLAKE3
  payload hash. The proposal id is the change id. The effective pack's one
  approval matrix decides auto-apply versus pending review, and the public
  apply seam independently re-hashes the typed payload, verifies the exact
  command, target ids and digest in the proposal's immutable manifest, then
  re-runs object ownership, every PDP decision and all lifecycle/revision
  preconditions against current state.

- **Domain and schema.** Eight closed commands cover create, edit, verify,
  supersede, merge, archive, restore and forget, with a common result carrying
  `applied`, `pending_review` or `rejected` plus the resulting stable item,
  revision and durable-operation addresses. Migration
  `0048_knowledge_lifecycle` takes the chain to **46 migrations** at schema
  epoch **2**. It adds four tenant-bound, enabled-and-forced-RLS tables:
  `knowledge_changes`, reusable `durable_operations`, content-free
  `knowledge_erasure_tombstones` and `knowledge_index_invalidations`. SQLx
  offline metadata was regenerated from a fresh migrated database.

- **Command semantics.** Create writes the first immutable revision. Edit and
  verify append with an exact current-revision precondition. Supersede creates
  a replacement, records `supersedes` and closes the old current state. Merge
  carries every distinct source, records `derived_from` for every input and
  supersedes those inputs. Archive/restore move lifecycle without changing
  content identity. A stale or invalid reviewed command rolls its partial
  effect back to a savepoint and closes the real proposal `rejected`; review
  never turns old authority or an old head into a current write.

- **Erasure.** Forget first creates a durable operation and evaluates the
  retention/legal-hold seam. A hold blocks the operation, retains content and
  terminally rejects the change. An authorised operation marks
  `erasure_pending`, rejects every other open change naming the aggregate,
  removes revisions, relations, exclusive source descriptors, future
  embedding/index state and all affected typed command payloads, then removes
  the head. The retained tombstone, VedaFlow objects and audit entries contain
  ids, timestamps and hashes only. Retrieval invalidation is explicit, and
  retry/lease state is reusable by later import, re-index and re-encryption
  jobs.

- **PDP, VedaFlow, audit and observability.** Cedar gains separately decidable
  `knowledge.read`, `knowledge.write` and `knowledge.forget` actions plus the
  `KnowledgeItem` entity. Every input and output scope of merge/supersession is
  decided; made-up and foreign ids fail ownership before policy. Generic
  proposal creation rejects `apply`, so only the typed command service can
  create the effect. Proposal detail verifies and renders the erasable typed
  payload, and the CLI can apply the already-reviewed proposal through the
  gateway. Six lifecycle/erasure audit actions chain decisions and transitions
  without content. New paths carry tracing and
  `synveda_knowledge_lifecycle_acts_total`.

- **Hard-cut boundary and deletions.** The gateway no longer starts the old
  session-event extractor, promotion engine or retention sweep, so an ordinary
  running process cannot manufacture, publish or destroy records behind the
  Knowledge lifecycle. No Knowledge command reads or writes `records`. Two
  controlled old-plane seams remain and are named rather than hidden:
  VedaFlow-governed record classification supports the restricted-tier
  adversarial proof until CPR-17 deletes its route/CLI/eval client. This
  checkpoint originally forecast that CPR-18 would also move composition off
  `records`; CPR-18 removed the extraction writer instead, and the later
  explainable context planner owns that read cutover. The feature record
  contains the exact deletion list.

- **Security and acceptance evidence.** Three database-backed gateway tests
  cover personal auto-apply with an Applied proposal, immutable edit/verify,
  stale rejection, another principal's private scope, archive/restore,
  supersession, merge provenance, regulated pending review, exact typed review
  rendering, approval plus public apply, policy drift, allowed erasure, held
  erasure, closure of competing open changes, content-free retained evidence
  and cross-tenant RLS for all four new tables. Approval **6/6**, pack **7/7**,
  PDP **11/11**, VedaFlow **79/79**, type **212/212**, leak **2/2** and RLS
  completeness pass. The full database suite includes the lifecycle tests and
  fresh epoch/bootstrap proof.

- **Runnable evidence and findings.** `demos/cpr-16-knowledge-lifecycle.sh`
  runs in an isolated database and reports **19 governed changes, zero old
  records**. The first `make db-test` found a checked explorer fixture that
  predated `knowledge.write`/`knowledge.forget`; it was re-recorded from the
  gateway and its two-line semantic diff reviewed. The next run found one
  rollback assertion pinned to the older substring `has no channel`; it now
  pins the production's more precise `has no VedaFlow channel` refusal and the
  focused test passes. Neither finding changed an authority or weakened an
  assertion.

- **Tests and exact results.** Focused tests above and the isolated demo
  **PASS**. `make db-test` **PASS** against a fresh migrated scratch database.
  `make ci` **PASS**, including Rust/clippy `-D warnings`, SQLx offline
  compilation, dependency/licence/backlog/ADR/API drift, Helm, evaluation
  parsing, console **150/150** and Claude adapter **96/96**. The first managed
  sandbox CI invocation could not bind loopback in two existing CLI tests;
  the permitted rerun is the product result.

- **Limitations and next work.** CPR-16 deliberately adds the governed command
  layer before exposing Knowledge CRUD/search. CPR-17 owns the generated
  public contract and Knowledge Browser and deletes the raw-record product
  surface and classification seam. CPR-18 then moves retrieval and context
  composition to current Knowledge revisions and removes the final controlled
  record projection. Live Entra/Okta and authentic Cursor evidence remain
  unrelated external gaps.

- **Commit.** `feat(knowledge): govern knowledge lifecycle with VedaFlow
  (CPR-16)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-17 checkpoint on Prompt 1's rule.

### Prompt 15 objective — Public Knowledge API, search and browser (CPR-17)

- **Selected feature and state.** **CPR-17** is delivered from `f2a7c5c` and
  also subsumes the already-filed **CNSL-4** browser objective. This is the
  public/read/browser half of CPR-15/16, not a record facade: Knowledge is the
  only public product noun, every write enters CPR-16's typed VedaFlow command
  layer and no DTO translates a record into Knowledge.

- **Decision.** ADR-0082 makes the immutable current Knowledge revision the
  read authority. Ordinary listing and search decide every item under its own
  scope chain and pack; source descriptors are decided under their own scopes,
  and a relation is disclosed only when both endpoints are readable. Cursors
  bind the normalised filter/query and advance over the last candidate
  considered rather than the last row served, so an all-denied page can still
  make progress without leaking a count. Creation uses an idempotency key;
  existing-head changes use the exact revision precondition the reader saw.

- **Schema and retrieval.** Migration `0049_knowledge_search` takes the chain
  to **47 migrations** at schema epoch **2**. The tenant-bound
  `knowledge_revision_embeddings` sidecar is enabled-and-forced RLS, keyed by
  immutable revision and model, indexed only at the reviewed 16/1024 vector
  dimensions and cascades under authorised erasure. Lexical search uses the
  current revision's stored weighted document. Configured TEI supplies the
  semantic leg and bounded reciprocal-rank fusion; the deterministic hash
  embedder is never queried or described as semantic and yields an explicit
  lexical-only degradation. A restart-safe background sweep indexes revision
  content outside the database transaction and inserts idempotently.

- **API and console.** The handler-derived OpenAPI document grows from 40 to
  **53 operations**, adding all thirteen Knowledge collection/item/history/
  source/usage/lifecycle operation groups with the common error envelope,
  cursor/filter schemas, idempotency metadata and revision preconditions. The
  generated TypeScript operation table is the only Knowledge client contract.
  `/console/knowledge` and `/console/knowledge/{item_id}` provide search and
  filters, current content, immutable history, independently visible
  provenance, relationships, verification and create/edit/merge/supersede/
  archive/restore/forget flows. Usage is truthfully empty until the later
  context-planning selection producer exists; mutation history is not
  relabelled agent use.

- **Hard-cut deletions.** The proposal classification route, `synveda proposal
  classify` and the eval caller are deleted. Generic public proposals reject
  the removed `record_ids` and `effect` fields. Channel publication no longer
  accepts record ids; memory channel history, rollback, pin and unpin aliases
  are refused in favour of explicit authored asset kinds. Raw-record review
  fixtures and seven record-oriented public integration suites are deleted,
  with their still-valid lapse regressions retained in a narrow suite. The one
  remaining record plane is internal session-event extraction/context
  composition owned by CPR-18; it is not exposed, translated or dual-written.

- **PDP, RLS, audit and observability.** Ownership precedes policy so a real id
  in another tenant and fiction are the same 404. Item, source and edge
  decisions happen before data or counts enter a response. The new sidecar is
  in the explicit and dynamic forced-RLS inventories. Read audit events carry
  ids, filter hashes, counts and retrieval mode rather than content, query text
  or source locators. Index and API paths carry tracing plus bounded metrics.

- **Acceptance evidence and findings.** The public database case proves
  idempotent create/conflict, filters and cursors, lexical search, honest
  semantic degradation, immutable detail/history, private-source omission,
  edit/verify, merge, supersession, archive/restore/forget, another tenant's
  real id, embedding erasure and the removed route/payload/channel shapes.
  `demos/cpr-17-knowledge-browser.sh` runs it in a disposable database, runs
  OpenAPI and console contracts and reports **one Knowledge item, zero old
  records**. The full database gate first found obsolete record-oriented
  suites and a missing explicit RLS inventory entry; deleting the replaced
  suites, retaining independent lapse coverage and adding the sidecar to the
  inventory made the complete gate green. CI then caught two pagination
  helpers over the clippy argument limit; their shared explicit page context
  removed the duplication without a lint exemption. The managed sandbox
  cannot bind the two existing CLI loopback tests; the permitted identical CI
  rerun is the product result.

- **Tests and exact results.** Gateway public Knowledge **1/1**, OpenAPI
  **5/5**, console **151/151**, dynamic RLS **84/84**, and the isolated demo
  **PASS**. `make db-test` **PASS** against a fresh scratch database. `make ci`
  **PASS**, including Rust/clippy `-D warnings`, SQLx offline metadata,
  dependency/licence/backlog/ADR/API drift, Helm, deterministic eval parsing,
  Claude adapter **96/96** and console **151/151**.

- **Limitations and next work.** CPR-17 is a browser/search surface, not the
  final agent retrieval plane. CPR-18 must replace internal record extraction
  with reviewable session-derived capture candidates; the later explainable
  context package moves composition to current Knowledge revisions and adds
  the scoped recall/query lenses. Live Entra/Okta and authentic Cursor evidence
  remain unrelated external gaps. CPR-13 stays reserved until the surfaces its
  demos must teach exist.

- **Commit.** `feat(console): add knowledge browser and search (CPR-17)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-18 checkpoint on Prompt 1's rule.

### Prompt 16 objective — Session capture batches and reviewable candidates (CPR-18)

- **Selected feature and state.** **CPR-18** is delivered from `2d845b0`.
  It replaces the last automatic session-event-to-record writer; it is not a
  second Knowledge command service or a model-output publication path. The
  preceding CPR-17 feature commit is
  `2d845b0f8a43d66f802286df922b820bf1bf25cf`.

- **Decision.** ADR-0083 makes an exact ordered session-event snapshot the
  idempotency unit. Explicit capture and terminal close canonicalise eligible
  event ids/types/payload hashes into one BLAKE3 snapshot digest. A durable
  per-tenant lease owns extraction, and a candidate decision intent is stored
  before its Knowledge command runs. Session authority decides extraction;
  Knowledge authority independently decides every destination, merge input,
  supersession endpoint and disclosed match.

- **Domain and schema.** The closed batch states are `pending`, `running`,
  `completed` and `failed`; the seven candidate outcomes are `pending`,
  `accepted`, `edited_and_accepted`, `merged`, `replaced`, `dismissed` and
  `failed`. Migration `0050_capture_candidates` takes the chain to **48
  migrations** at schema epoch **2** and adds six tenant-bound,
  enabled-and-forced-RLS tables: batches, frozen batch events, candidates,
  candidate sources, independently visible matches and append-only decision
  intents/results. Composite foreign keys make a fictional, cross-session or
  cross-tenant source unrepresentable. Owner-level triggers preserve frozen
  evidence and legal transitions; authorised Knowledge erasure scrubs
  candidate/request plaintext while leaving only ids and hashes. SQLx
  all-target metadata was regenerated from a fresh migration-0050 database.

- **Extraction boundary.** The deterministic, Claude and OpenAI-compatible
  extractor implementations now return bounded proposed Knowledge — type,
  title, Markdown body, summary, tags, sensitivity, confidence and metadata.
  The worker re-authorises the session principal before model work, rescans
  output for secrets, applies the Knowledge validators, retrieves a bounded
  current lexical neighbourhood and makes an exact `KnowledgeRead` decision
  before comparing each item. It persists duplicate, conflict and possible-
  supersession hints only for visible items and writes no Knowledge, records,
  vectors, graph edges or VedaFlow channels.

- **API and command semantics.** OpenAPI grows from 53 to **62 operations**:
  create/list/detail batches, whole-batch accept, list candidates and
  candidate accept/merge/replace/dismiss. Collections use bound opaque
  cursors and the common envelope; retryable acts require idempotency keys.
  Accept/edit creates Knowledge, merge carries the candidate's real source
  evidence into the merged aggregate, replace invokes explicit governed
  supersession and dismiss creates no Knowledge. The decision row binds the
  canonical request hash and caller before execution; CPR-16's idempotency
  ledger then converges lost acknowledgements, while the winner of the
  terminal transition alone audits. Whole-batch acceptance derives stable
  child keys and records its parent only after every child, so an interrupted
  batch resumes rather than duplicates.

- **PDP, disclosure and evidence.** Reading candidate plaintext requires both
  `SessionRead` on the exact source run and `KnowledgeRead` at the proposed
  destination; preferences default to the session principal's private scope,
  while shared facts default to the governed run scope/project. Match rows are
  re-authorised on every response, so a revoked grant cannot leak an item id,
  revision, reason or count. Three new content-free audit actions record batch
  creation/completion and candidate decisions; worker and API paths carry
  spans plus bounded batch/candidate/extractor counters and duration metrics.

- **Hard-cut deletions.** The PGMQ `session_events` queue, queue helpers, old
  record/embed/dedup/graph-link/channel commit worker and five direct-active
  extraction integration suites are deleted. Session append is once again
  only an immutable ledger write; capture is explicit or terminal. There is
  no record-to-Knowledge bridge or dual write. The old record-backed context
  composer remains an explicitly read-only seam until the explainable
  Knowledge context planner replaces it; the deterministic Claude acceptance
  now asserts the resulting empty composition rather than manufacturing an
  old record behind the public path.

- **Acceptance evidence and findings.** The three database-backed capture API
  cases cover exact-snapshot replay, terminal capture, candidate-only output,
  all decision kinds, strict-profile pending review, stale/key conflicts,
  merge provenance, governed erasure, append-only evidence, cross-session
  source refusal, another tenant's real ids and match re-authorisation after a
  sibling-project grant. The first broad run exposed two honest contract
  updates: CPR-14's timeline golden still claimed the removed dual write had
  supplied one record, and the console's exact idempotent-operation inventory
  omitted all six new mutation groups. Both fixtures now pin the generated
  zero-entry context and 62-operation contract; no product assertion was
  disabled.

- **Tests and runnable evidence.** Capture gateway **3/3**, deterministic
  Claude lifecycle **2/2** (the separately named installed-client test remains
  opt-in), Knowledge lifecycle **4/4**, session redaction **2/2**, ingest
  **64/64**, types **213/213**, audit **20/20**, OpenAPI **5/5**, console
  **151/151**, Claude adapter **96/96** and RLS **84/84** pass.
  `demos/cpr-18-session-capture.sh` passes in an isolated database and reports
  **8 candidates, 8 governed changes, zero old records and zero old queues**.
  `make db-test` **PASS** against a fresh scratch database and `make ci`
  **PASS**, including clippy `-D warnings`, SQLx offline compilation,
  dependency/licence/backlog/ADR/API drift, Helm and deterministic eval parse.

- **Limitations and next work.** No new external model or proprietary-client
  claim was needed for this storage/application cutover; CPR-14's genuine
  Claude Code 2.1.241 evidence remains the current live-client result. CPR-19
  owns the New Learnings candidate experience. The later context-planning
  package owns Knowledge-backed selection and the scoped recall/query lenses;
  until it lands, accepted Knowledge is deliberately not translated into the
  temporary record composer. CPR-13 stays reserved until those product
  surfaces exist. Live Entra/Okta and authentic Cursor evidence remain
  unrelated external gaps.

- **Commit.** `feat(capture): extract reviewable session learnings (CPR-18)`
  on `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-19 checkpoint on Prompt 1's rule.

### Prompt 17 objective — New Learnings lightweight review workflow (CPR-19)

- **Selected feature and state.** **CPR-19** is delivered from `e778a60`.
  It is the ordinary personal/team presentation of CPR-18's durable candidate
  boundary, not another inbox or proposal service. The preceding CPR-18
  feature commit is `e778a6041bc6b56621c9aeb313ca2757da2b9471`.

- **Decision.** No new ADR was needed. ADR-0075 fixes the product shell and
  generated-client rule; ADR-0081 fixes the one governed Knowledge mutation
  seam; ADR-0082 fixes fresh per-object Knowledge reads; and ADR-0083 fixes
  candidate-only capture plus the public decision contract. The console uses
  capability forecasts only to decide which controls to offer. Every read and
  mutation still reaches the gateway, where ownership and the PDP decide.

- **Product surface.** `/console/learnings` now lists cursor-paginated capture
  batches and candidates, filters both by project and exact session, filters
  candidates by decision state, groups cards under their durable batch and
  reports honest loaded progress. Each card distinguishes proposed content,
  type, confidence and sensitivity; private, project and workspace placement;
  duplicate, conflict and possible-supersession matches; decision status; and
  the resulting Knowledge or review destination.

- **Evidence and comparison.** Exact candidate source event ids are joined to
  the public session timeline for a conversation preview. Raw payload remains
  an on-demand, separately authorised read and is offered only when the exact
  run's forecast includes `session.diagnostics`. Every match comparison is
  fetched afresh through generated `get_knowledge`; a revoked grant therefore
  becomes the gateway's refusal instead of a title or body cached in frontend
  state.

- **Governed decisions.** Accept, edit-and-accept, merge, replace,
  change-scope-and-accept and dismiss use only the 62-operation generated
  public client. Retryable decisions carry idempotency keys; merge and replace
  carry the exact existing revision precondition supplied by extraction;
  changed placement sends explicit optional-field nulls. Replace invokes the
  governed supersession command and retains history. Dismiss publishes
  nothing. Applied outcomes link to Knowledge, while pending outcomes link to
  Advanced Reviews and explicitly remain inactive until publication.

- **Scope safety and removed residue.** Destination choices are limited to
  the relevant principal, project and workspace anchors whose forecast offers
  `knowledge.write`; the gateway repeats the authoritative decision. The
  stale New Learnings placeholder and its “not built” assertion are deleted.
  Advanced Reviews remains the sole comprehensive VedaFlow review surface;
  no capture-specific proposal, quarantine or review model was added.

- **Schema, API and security boundaries.** This package changes no migration,
  SQLx metadata, route, OpenAPI schema, Cedar action or audit vocabulary.
  CPR-18 remains the database/PDP/RLS/audit authority for every candidate
  read and decision. The generated contract remains **62 operations**, schema
  epoch **2** remains at **48 migrations**, and no frontend path can publish
  Knowledge directly.

- **Tests and exact results.** Eight pure cases pin collection filters,
  policy-filtered placements, all generated wire operations, explicit nulls,
  revision preconditions, grouping/progress and outcome wording. Six rendered
  component acceptance cases cover evidence and comparisons, every action,
  denied destinations, read-only sessions, applied and pending-review results
  and dismissal. The production TypeScript/Vite build and complete console
  suite **165/165** pass. `make ci` **PASS**, including Rust tests, clippy
  `-D warnings`, dependency/licence/backlog/ADR/generated-API gates, Helm,
  deterministic evaluation parsing, console **165/165** and Claude adapter
  **96/96**. `make db-test` is N/A because no database-backed behaviour
  changed; CPR-18's passing database evidence is not relabelled as a rerun.

- **Limitations and next work.** No external client or model claim is part of
  this console package. CPR-20 now owns the Knowledge-backed explainable
  context planner, scoped recall/query surfaces and deletion of the temporary
  record composer. CPR-13 remains reserved until that final runtime read
  surface exists. Live Entra/Okta and authentic Cursor evidence remain
  unrelated external gaps.

- **Commit.** `feat(console): add New Learnings workflow (CPR-19)` on
  `feat/context-platform-mvp`.
- **Commit hash.** `e90dac9c9f36e747c380b377f524dd383b7603ce`.

### Prompt 18 objective — explainable Knowledge context planning and scoped query (CPR-20)

- **Selected feature and state.** **CPR-20** is delivered from `e90dac9`.
  It removes the final production read of the replaced record model. The
  preceding CPR-19 feature commit is
  `e90dac9c9f36e747c380b377f524dd383b7603ce`.

- **Decision.** Accepted ADR-0084 makes one distinction load-bearing: a
  context run is budgeted delivery, while ordinary deep query and privileged
  corpus evaluation are separate session-scoped reads. All three share the
  same current Knowledge and exact PDP seam; none restores tenant-global
  `/v1/recall`, direct-store adapter access or a record translation layer.
  A trace is governed content in its own right, so a denied candidate is never
  persisted and every retained address is re-authorised before disclosure.

- **Schema and hard-cut boundary.** Migration `0051_context_planning` extends
  `session_context_runs` with the derived project, as-of instant, requested and
  actual budgets, retrieval/embedding/index/graph versions, completion and
  degradation states, query/render hashes, retention mode and policy-exclusion
  marker. `context_candidates`, `context_selections` and `context_feedback`
  retain bounded visible evidence and exact immutable revision outcomes. All
  are tenant-bound, forced-RLS and append-only to the application role, with
  tenant-qualified keys binding session, run, item and revision. The migration
  contains no `INSERT`, `UPDATE` or old-row translator: an opaque pre-cut run
  has no planner marker and every application query filters it out; a CHECK
  requires native rows to carry the complete planner shape. Schema epoch **2**
  now has **49 migrations**.

- **Planner and explanations.** Query composition uses bounded lexical and
  configured semantic retrieval over `knowledge_current`; queryless starts use
  bounded recency plus current conventions/preferences. Every exact candidate
  and source receives a fresh decision before persistence or rendering.
  Selected rows retain revision id, rank, token cost, eleven reason codes and
  separate integer keyword, semantic, freshness, pin, graph, current-state and
  final contributions. Stale and superseded revisions are visible exclusions
  only when authorised and are never selected as current. Graph absence is an
  explicit degradation/version fact until the bounded graph package.

- **Trace retention and side channels.** `full`, `redacted`, `hashes_only` and
  `disabled` have distinct storage and response assertions. Denied Knowledge
  creates no id, title, edge, reason or count; a run may report only one
  aggregate policy-exclusion message. The core delivery row retains exact bytes
  for lost-ack replay, but list reads mask them and detail reads apply retention
  plus fresh session/item/source decisions. Context packs and skill
  advertisements still compose through their own PDP actions and share the
  budget. Because this old core row cannot identify exact authored versions,
  any trace read after an authored input contributed masks the whole rendered
  block, its hash/tokens and skill list rather than treating a historical block
  as authority.

- **Public API and clients.** The generated OpenAPI contract grows **62 → 67
  operations**. The existing `POST /v1/sessions/{id}/context-runs` keeps its
  address and delivery semantics; new operations are cursor-paginated
  `GET /v1/context-runs`, re-authorised `GET /v1/context-runs/{id}`, idempotent
  exact-selection `POST /v1/context-runs/{id}/feedback`, ordinary
  `POST /v1/sessions/{id}/knowledge-query` and diagnostics-only
  `POST /v1/sessions/{id}/knowledge-evaluation`. The evaluation cursor advances
  over the last candidate considered, including denied rows, without disclosing
  their number. CLI and generic MCP recall use the ordinary public query;
  extraction/security/QA clients have the true enumeration/id lens needed for
  later remeasurement rather than abusing a context budget.

- **Audit and observability.** Retrieval, selection and feedback have separate
  spans and metrics. Three new hash-chain actions bring the vocabulary to
  **88**; their payloads contain ids, hashes, counts, versions and decisions,
  never query, rendered text, Knowledge content, source locator or event
  payload. Audit's “what did this agent know” fold now resolves exact immutable
  Knowledge selections and bitemporal revisions instead of old record entries.
  Knowledge usage uses the same re-authorised selection evidence.

- **Deleted production and test residue.** The record-backed composer and
  hydration/authorisation path, gateway `index_tier.rs` suite, record-shaped
  audit projection and temporary `RecallSweepRequest`/`RecallIdsRequest`
  refusal tombstones are deleted. No application query reads `records` or
  `record_embeddings`; no old row is dual-written or translated. Authored
  context-pack summaries no longer print dead record recall handles.

- **Tests and exact results.** Focused database-backed results: context runs
  **3/3**, audit query **13/13**, context packs **10/10**, sessions **22/22**,
  skills **24/24**, OpenAPI **5/5**, RLS trace immutability/completeness **2/2**;
  console **165/165**, CLI recall **2/2** and MCP **14/14** also pass.
  `make ci` **PASS** and the full fresh-scratch `make db-test` **PASS**,
  including the 1k-event load gate. The isolated runnable
  `demos/cpr-20-context-planning.sh` **PASS** reports **55 Knowledge items, 47
  plans, 75 immutable selections, 2 feedback rows and zero old records**.

- **Limitations and next work.** No live external client or model claim is made.
  The original extraction/security/QA benchmark questions now have a real
  evaluation lens, but Prompt 30 owns reproducible reseeding, measurement and
  baseline changes; this package does not relabel old refusal results. Bounded
  graph expansion, unreviewed-channel configuration and exact immutable skill
  binding identities remain their filed packages. CPR-21 now owns the Context
  Inspector presentation over these public traces.

- **Commit.** `feat(context): add explainable context planning (CPR-20)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-21 checkpoint on Prompt 1's rule.

### Prompt 19 objective — Context Inspector and explicit outcome feedback (CPR-21)

- **Selected feature and state.** **CPR-21** is delivered from `8ed8aa6`. It
  implements the external Prompt 19 objective without a new ADR, schema,
  Cedar action, audit action or OpenAPI operation. ADR-0075 continues to own
  the generated-client console shell, ADR-0077 the session timeline and
  ADR-0084 the re-authorised trace/feedback contract.

- **Inspector product surface.** `/console/context-runs/{id}` is a stable,
  refreshable route backed only by generated `get_context_run` and
  `create_context_feedback` operations. Full traces render the retained task,
  delivered block, exact Knowledge revision, current/stale/superseded state,
  rank, token charge, reason codes, keyword/embedding/freshness/pin/current-
  state/final scores and independently visible provenance. Requested and
  governed budgets, actual token use, retrieval/embedding/index/graph
  versions, degradation and rendered hash remain explicit. Graph is labelled
  `not run` rather than assigned an invented contribution before the bounded-
  graph package.

- **Retention and feedback.** Full, redacted, hashes-only and disabled modes
  state what was retained instead of treating absent content as empty.
  Hashes-only rows expose no Knowledge link or feedback control; disabled
  says selection detail was not retained and makes no claim that delivery
  selected nothing. The five feedback acts are user-initiated only and send
  one exact selection plus immutable revision under the run's generated
  idempotent operation. Selection alone creates no helpfulness assertion.

- **Timeline disclosure repair.** A context timeline entry now says `Synveda
  supplied N knowledge items`, links the exact run to the inspector and no
  longer repeats the task, rendered-entry total or token total on the broader
  `SessionRead` surface. The count is not copied from the historical run row:
  full/redacted selection addresses are freshly decided as Knowledge reads,
  denied rows add no count, and at most the aggregate current-policy notice is
  appended. A project reader of another principal's private selection sees
  zero visible items and no private id, revision, content or reason.

- **Tests and exact results.** Pure inspector/wire rules **7/7**, real-
  component acceptance **6/6**, complete console **179/179**, context API
  **3/3**, sessions API **22/22**, production Vite build **PASS**. `make ci`
  **PASS** and full fresh-scratch `make db-test` **PASS**, including the 1k-
  event load gate. The in-app browser runtime exposed no connected browser,
  so no interactive visual claim is made; the same production component is
  rendered in every retention/refusal acceptance case and the real bundle
  builds successfully.

- **Commit.** `feat(console): add context inspector and feedback (CPR-21)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-22 checkpoint under the programme's
  next-checkpoint convention.

### Prompt 20 objective — core individual/small-team MVP acceptance (CPR-22)

- **Selected feature and state.** **CPR-22** is delivered from `8cdd1ee`. It
  records CPR-21's commit as `8cdd1ee` and makes no architecture change: the
  package composes the public seams and accepted ADR-0070 through ADR-0084 in
  one adversarial product scenario.

- **PulseBoard loop.** The database-backed acceptance uses only the documented
  identity/root-grant test bootstrap beneath the gateway. Alice creates the
  workspace/project and Bob's grant through public operations, then every
  product act uses the public session, event, capture, Knowledge and context
  API. Four Alice events freeze to four candidate-only proposals. She publishes
  the webhook identity and request-header convention at the project, keeps the
  quick-test preference at her principal scope and dismisses the incidental
  detail. Each publication is an ordinary applied Knowledge VedaFlow change;
  the dismissal has no change and extraction itself has no active item.

- **Clean team reuse and correction.** A fresh Bob session receives both exact
  project revisions with Alice's session-event provenance. His rendered block,
  candidate/selection trace, Knowledge detail and scoped query disclose no id
  or content from her personal preference. Bob then records the `traceparent`
  correction as another event and capture candidate and resolves it with the
  public replace command against the exact inspected `X-Request-Id` revision.
  The replacement owns an immutable `supersedes` relation; the old aggregate
  remains history. A third fresh run selects the replacement and excludes the
  old item explicitly as `superseded`, never as current truth.

- **Evidence across the boundaries.** The generated context detail used by the
  Inspector contains the replacement revision, exact source event, reason
  codes, rank, token charge, retrieval version and rendered hash. Its session
  timeline has the exact link target and content-free `Synveda supplied N
  knowledge items` summary. Database assertions finish at three sessions, five
  events/candidates/decisions, four Knowledge items/revisions/VedaFlow changes,
  three active and one superseded head, two context runs, three selections and
  zero record writes. `/v1/observe`, `/v1/inject` and `/v1/recall` remain 404.
  The tenant audit chain verifies and contains allowed PDP decisions plus every
  session/capture/Knowledge/context transition without the three content
  sentinels.

- **Tests and exact results.** Consolidated MVP AC **1/1**, complete capture
  integration **4/4**, context **3/3**, complete real-component console
  **179/179**. The isolated `demos/cpr-22-mvp-acceptance.sh` passed with the
  counts above. `make ci` **PASS** and full fresh-scratch `make db-test`
  **PASS**, including the 1k-event ingestion gate. The first restricted CI
  invocation was denied permission to bind two loopback test listeners; the
  unchanged unrestricted invocation passed. This deterministic test is not a
  live-client claim; CPR-14 remains the genuine Claude Code 2.1.241 run.

- **Schema and contract.** No migration, route, DTO, Cedar action or audit
  action moved. Schema epoch **2** remains **49 migrations** and OpenAPI remains
  **67 operations**.

#### Core MVP checkpoint

- One runtime serves a personal user and a project team: **proved** by Alice,
  Bob and their shared workspace/project/session API.
- Sessions produce reviewable candidates and candidates remain separate from
  active Knowledge: **proved** before the first decision and by dismissal.
- Accepted Knowledge has immutable revisions and exact provenance: **proved**
  for all four published aggregates and their session-event sources.
- A clean session and teammate reuse project Knowledge: **proved** by Bob's
  second session; private Knowledge stays private across detail, query, plan
  and rendered context.
- Superseded Knowledge is not current or supplied: **proved** by the explicit
  edge/current projection and third clean run.
- Context selection is explainable: **proved** by the generated Inspector
  detail and timeline address.
- The complete path is PDP-, VedaFlow-, RLS- and audit-governed: **proved** by
  per-object decisions, one change per mutation, forced-RLS full suite and a
  verifying content-free chain.

- **Commit.** `test(mvp): verify cross-session team knowledge loop (CPR-22)`
  on `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-13 checkpoint under the programme's
  next-checkpoint convention.

### Demo-corpus convergence objective — current platform re-point (CPR-13)

- **Selected feature and state.** The already-reserved **CPR-13** is delivered
  from `c9e647d`, after CPR-22 supplied the final Knowledge/capture/context
  surfaces the demos must teach. It records CPR-22's commit as
  `c9e647d6332457735e8c2b05b43690f9e7b2dc2d`.

- **Authoritative inventory and deletion.** The generated-contract scan found
  **49 affected scripts**, six more than CPR-12's route-name estimate because
  the acceptance criterion also rejects real server paths missing from the
  generated contract. Those scripts contained **18,528 lines** of copied
  hierarchy, role-binding, record, global runtime, IdP and hand-written route
  setup. They are now short, feature-specific current-model narratives over a
  shared isolated epoch-2 Postgres harness: 18,528 affected-script lines became
  504 plus a 52-line helper, a net reduction of **17,972 lines**. No
  compatibility command, old route, record-to-Knowledge translation or direct
  database seeding was added.

- **Preserved teaching map.** MEM now demonstrates idempotent ordered session
  events, redaction, capture candidates, current Knowledge indexing/matches and
  governed erasure. CTX demonstrates current-revision planning, budgets,
  session-scoped delivery and trace retention. FLOW demonstrates the one
  VedaFlow Knowledge change ledger, auto-apply, pending review and immutable
  correction. AUTH/AUTHZ/TEN demonstrate principal scopes, groups, grants,
  anchors and per-row decisions. ADPT uses public clients and authentic Claude
  frames. The audit, console, evaluation, graph, operations, prompt and skill
  scripts point at their corresponding current focused suites. Historical
  filenames remain feature-evidence addresses, not aliases or runtime support.

- **Drift gate.** `scripts/check-demos.mjs` recursively scans all **73** shell
  scripts without executing them. It removes unquoted comments, joins continued
  lines, skips heredoc fixtures and explanatory output, distinguishes Rauthy's
  external `/auth/v1` contract, recognises both literal `synveda` and common
  built-binary aliases, and checks command positions against recursive Clap
  help plus production paths against generated OpenAPI. Cargo first refreshes
  the binary, so a stale executable cannot bless removed source. Four tests
  deliberately inject dead command/path and alias cases and pin the safe
  exclusions. `make check-demos` runs both test and corpus passes and is a
  prerequisite of `make ci`.

- **Representative live-database evidence.** `demos/mem-1-observe.sh` passes
  sessions **22/22** and the 10,000-event load gate at **1,006 events/s** (ack
  p50 29.36ms, p95 34.65ms, p99 36.12ms); `ctx-3-inject.sh` passes the current
  Knowledge planner **1/1**; `flow-3-proposals.sh` passes lifecycle/VedaFlow
  **4/4**; `authz-2-policy-packs.sh` passes current-scope decisions **2/2**;
  and `adpt-1-claude-code.sh` builds the real hook and passes **2/2** using
  authentic captured Claude Code 2.1.241 frames. Its separately named live
  proprietary-client test remains ignored in this run; CPR-14's genuine live
  2.1.241 evidence is not relabelled replay.

- **Schema and architecture.** This documentation/test package adds no ADR,
  migration, route, DTO, Cedar action, audit action, metric or runtime path.
  Schema epoch **2** remains **49 migrations** and OpenAPI remains **67
  operations**. `make db-test` is therefore not required; the five isolated
  Postgres demos are package evidence, not a claim that persistence changed.
  Complete `make ci` **PASS**, including the new four-case checker test and
  73/73 generated inventory gate, Rust fmt/clippy/tests/build/licences,
  dependency/ADR/backlog/API/benchmark/chart/evaluation checks, console
  **179/179** and Claude adapter **96/96**.

- **Commit.** `test(demos): re-point demo corpus and gate drift (CPR-13)` on
  `feat/context-platform-mvp`.
- **Commit hash.** `9b8ad04f68b2aa2f1b90d9b066b926327fd1f9ba`.

### Prompt 21 objective — immutable Skill versions, bindings and usage (CPR-23)

- **Selected feature and specification.** **CPR-23** is delivered from
  `9b8ad04`. ADR-0085 is accepted. The official Agent Skills format remains
  unversioned; this implementation pins the tested contract to upstream
  `agentskills/agentskills@69ef37e9424c0a7ea9dd2293b559e43ec8176379`,
  observed 2026-08-24, rather than inventing a protocol number. Required and
  optional frontmatter, the published name grammar, extension metadata and
  exact bundle bytes are fixture-pinned; `allowed-tools` stays a declaration
  and creates no Cedar authority.

- **Domain and schema hard cut.** Migration `0052_versioned_skills.sql` drops
  the mutable draft `skills`/`skill_files` rows plus their special
  `skill_reviews`/`skill_quality_overrides`, with no translation, and creates
  stable Skill aggregates, immutable ordinal versions and files, revisioned
  project/principal bindings, typed VedaFlow Skill effects, append-only usage
  evidence and immutable test runs. All seven tables are tenant-bound with
  enabled/forced RLS and composite ownership constraints; immutable history
  has no application update/delete grant. Schema epoch **2** now has **50
  migrations** and **630** checked SQLx query descriptions.

- **One governed mutation path.** Install, content update, binding creation,
  enable/disable/pin/unpin and rollback all open an `AssetKind::Skill`
  VedaFlow `Apply` change whose typed command is payload-hash bound. Apply
  repeats target ownership, `SkillWrite`, `ProposalOpen`, revision/current-
  version preconditions, exact object reconstruction and current scanner and
  rubric gates. Auto-apply therefore still creates/applies a change; stricter
  packs retain a pending review. Stale competing changes reject without moving
  a head or binding. Five semantic audit actions cover change, usage and test
  transitions without file/frontmatter/tool/output content.

- **Read, distribution and evidence.** Eighteen generated public operations
  expose catalogues, exact versions/files, binding history/control,
  PDP-filtered availability, eight usage stages and controlled tests. The
  OpenAPI inventory grows **67 → 85** and the generated TypeScript client has
  **119 schemas**. Context composition and client materialisation resolve the
  same enabled binding set and retain binding id, version id, digest and
  object address. Usage is idempotent per client event and distinguishes
  `host_observed` from `model_reported`. The built-in
  `validation_sandbox` parses, rescans and scores stored bytes but never runs a
  script; an external controlled-client harness remains explicitly labelled.

- **Deleted implementation.** `SkillChannel`, `ChannelRef::skill`,
  `skill/published`, mutable draft/file CRUD, skill publish/channel pin and
  rewind, direct-store CLI operations, special checklist/quality-override
  actions/routes/tables and duplicate skill telemetry leave production. The
  existing content-addressed VedaFlow object store, common proposal review
  engine, scanner/rubric and client-side materialisation remain because they
  serve the new aggregate. CPR-24 immediately replaces the still-offline old
  admin Skill review renderer/fixture corpus with the Skills Library; those
  fixtures are not a served backend path.

- **Tests and exact results.** Gateway end-to-end **1/1** proves pending
  review, approval/apply, two immutable versions, stale rejection, exact file
  bytes, binding/follow-current/rollback, eight-stage model, idempotent usage,
  a non-executing idempotent test run, content-free audit and cross-tenant 404.
  Forced-RLS/immutability **1/1**, RLS completeness **1/1**, policy packs
  **7/7**, OpenAPI **5/5**, CLI **157/157**, console **179/179**, generated API
  drift and workspace clippy with warnings denied all pass. The isolated
  `demos/cpr-23-versioned-skills.sh` reports one aggregate, two versions, one
  binding, one usage event and one validation run. Complete `make ci` **PASS**
  and full `make db-test` **PASS** against disposable
  `synveda_test_80706`, which was removed on success.

- **Commit.** `feat(skills): add immutable versions bindings and usage
  (CPR-23)` on `feat/context-platform-mvp`.
- **Commit hash.** `89b5f790a1268e55d8e0df849032ac06a954fd97`.

### Prompt 22 objective — Skills Library product experience (CPR-24)

- **Selected feature and architecture.** **CPR-24** is delivered from
  `89b5f79`. ADR-0075 and ADR-0085 already fix the generated-client product
  shell, immutable Skill aggregate, binding and controlled-harness boundaries,
  so no new ADR or backend variation was introduced. Schema epoch **2** stays
  at **50 migrations**, OpenAPI stays at **85 operations**, and
  `make db-test` is not applicable to this console/client-only hard cut.

- **One generated-contract Library.** `/console/skills` now lists installed
  immutable heads and separately asks CPR-23's exact availability resolver
  what a personal or selected-project session would receive. A stable
  `/console/skills/{skill_id}` address exposes every visible immutable version,
  exact file bytes, bundle digest, provenance and source revision, manifest
  extensions, client compatibility, scanner evidence and quality score. Tool
  declarations are labelled as metadata and explicitly grant no authority.

- **Governed controls and evidence.** Complete-bundle installation and update,
  personal/project bind, enable, disable, exact pin, follow-current and
  rollback all call the generated idempotent VedaFlow operations and report
  `applied`, `pending_review` or `rejected` without pretending a proposal moved
  active state. Binding writes retain revision preconditions. Capability
  forecasts decide which controls are offered, never whether an operation is
  allowed. Validation names the in-process `validation_sandbox`, says no script
  is executed, and keeps controlled-client runs distinct. Usage remains tied to
  one immutable version and visibly separates host-observed evidence from model
  self-report.

- **Deleted implementation.** The last hand-written Skill request in
  `api.mts`, the mutable-Skill scan/checklist/quality branch in console and CLI
  proposal review, ten stale fixture files, their fixture-corpus tests and dead
  styles are removed. Advanced Reviews remains as the artifact-neutral common
  VedaFlow surface; immutable scan and quality evidence now lives where it is
  actionable, on the Skills Library version.

- **Tests and exact results.** Pure and real-component Skills acceptance
  **10/10**, artifact-neutral shared review **5/5**, complete console
  **186/186**, CLI **151/151**, both TypeScript compilations, generated-client
  drift and the production Vite build pass. Complete `make ci` **PASS** across
  Rust fmt/clippy/tests/build/licences, dependency/backlog/ADR/OpenAPI/demo/
  corpus/chart/benchmark/evaluation checks and all TypeScript workspaces. The
  initial sandboxed run failed only because two CLI tests could not bind a
  loopback listener (`Operation not permitted`); the unrestricted full run
  passed them. The in-app Browser exposed no browser instance, so interactive
  browser QA was unavailable; real-component server rendering plus the
  production bundle is the recorded UI evidence.

- **Commit.** `feat(console): add Skills Library (CPR-24)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-25 checkpoint under the programme's
  next-checkpoint convention.

### Prompt 23 objective — trusted MCP server catalogue (CPR-25)

- **Selected feature and specification.** **CPR-25** is delivered from
  `07ce9f3`; it records CPR-24's commit as
  `07ce9f3b32d67c4a50e83ff8fed38d6abdd7983f`. ADR-0086 is accepted. The
  official stable MCP contract verified on 2026-08-24 is stateless
  `2026-07-28`, pinned to the specification release commit
  `5f5440bb26a62e2cf3440b92da5a667efa03b267`. Stdio and Streamable HTTP are
  the accepted transports; retired HTTP+SSE, protocol sessions and invented
  future revisions are refused.

- **Immutable trust model.** Migration `0053_tool_registry.sql` creates stable
  ToolServer identities, immutable ordinal ToolServerVersions, immutable raw
  and deterministically normalised CapabilitySnapshots, revisioned exact-
  version project ToolBindings, typed Tool changes and immutable ToolTestRuns.
  A canonical digest covers source, transport, authentication, secret-reference
  identity, requested permissions and tool/resource/prompt metadata. Any drift
  creates a quarantined version; the same digest returns the existing evidence.
  All six tables are tenant-bound with enabled/forced RLS and composite tenant
  ownership. Database triggers make history immutable and prevent a current
  pointer or binding from naming a version whose VedaFlow change was not
  applied. Schema epoch **2** now has **51 migrations** and **655** checked
  SQLx query descriptions.

- **One governed mutation path.** Registration, supported-client import,
  discovery drift, binding creation, disable/re-enable, exact repin and removal
  use `AssetKind::Tool` VedaFlow Apply changes. Apply repeats target ownership,
  `ToolWrite`, `ProposalOpen`, payload hash, current head, exact approval and
  revision preconditions. Regulated policy leaves Tool changes pending for two
  distinct reviewers plus the executable-boundary reviewer; applying another
  server version never moves an existing project binding. `ToolRead` and
  `ToolWrite` are carried through every pack, capability forecast and service-
  token confinement rule. Five audit actions retain ids, digests, counts,
  method names and outcomes rather than descriptions, schemas or credentials.

- **Public discovery and configuration plane.** Sixteen generated operations
  grow OpenAPI **85 → 101** and the TypeScript contract to **139 schemas**.
  They import a bounded manifest/client entry, list and inspect exact versions,
  compare normalised capabilities, govern exact bindings, generate a client
  configuration containing reference placeholders only and record immutable
  discovery/list test evidence. Collection routes are cursor-paginated and
  decide every returned row. The gateway never launches an imported stdio
  command, resolves a secret reference or accepts `tools/call`; descriptions,
  annotations, requested permissions and declared schemas grant no authority.
  The existing `synveda mcp` process remains a distinct public-API adapter.

- **Threat and deletion result.** Client config import rejects credential-
  shaped environment/header/token content without echo. Source/auth/schema
  substitution is visible as quarantined digest drift. Foreign and fictional
  identifiers produce the same absence, and the cross-tenant ambit regression
  test prevents a scoped service permit escaping its tenant. No retired
  transport, session compatibility shim, mutable-version replacement,
  follow-current Tool binding, gateway code-execution seam, execution proxy,
  plaintext secret field or duplicate MCP adapter was added. The capability
  explorer fixtures were re-recorded only to add the two real Tool forecasts.

- **Tests and exact results.** Tool domain validation **5/5**, gateway boundary
  units **3/3**, public database lifecycle **1/1**, complete policy PASS
  (packs **7/7**, approvals **6/6**, service confinement **4/4**), forced-RLS
  completeness **1/1**, OpenAPI **5/5**, generated client drift and console
  **186/186** pass. The isolated `demos/cpr-25-tool-registry.sh` reports one
  stable server, two immutable versions/snapshots, one exact binding, four
  governed changes and one non-executing test report. Full `make db-test`
  **PASS** against disposable `synveda_test_88082`; complete `make ci` **PASS**
  after its warnings-as-errors and generated-client golden findings were fixed
  rather than suppressed. The deterministic discovery report is fixture
  evidence, not a live external-server or proprietary-client claim.

- **Commit.** `feat(tools): add trusted MCP server registry (CPR-25)` on
  `feat/context-platform-mvp`.
- **Commit hash.** `9845186b4dfed7a61c59e997f3c31c85b8840dba`.

### Prompt 24 objective — MCP Tools catalogue product experience (CPR-26)

- **Selected feature and architecture.** **CPR-26** is delivered from
  `9845186`; it records CPR-25's commit as
  `9845186b4dfed7a61c59e997f3c31c85b8840dba`. No new ADR was needed:
  ADR-0075 already makes the generated contract the console boundary and
  ADR-0086 already fixes immutable MCP evidence, VedaFlow approval, exact
  bindings, secret references and the no-execution line.

- **One inspection surface.** `/console/tools` replaces the sole remaining
  `Planned` page with a cursor-paginated catalogue, selected-project import and
  secret-safe generated configuration. `/console/tools/{server_id}` is the
  stable address for source, exact version/digest, MCP 2026-07-28, transport,
  authentication kind, reference presence, trust, last discovery, honest
  metadata-validation/executable-scan state and the complete normalised
  tools/resources/prompts plus descriptions, arguments and JSON schemas.
  Selecting a quarantined version produces a blocking visual state and a
  deterministic diff against the exact approved head; its VedaFlow change
  links to artifact-neutral Advanced Reviews rather than growing a second
  approval UI.

- **Exact project distribution and evidence.** Binding controls list approved
  versions only and call the generated idempotent create/update operations for
  enable, disable, exact repin, remove and restoration with the binding's
  revision precondition. Approval alone therefore still moves no project.
  Discovery reports compare against the exact approved head. Connection-test
  evidence names a trusted local or remote adapter, its version, outcome,
  latency and the closed discovery/list method set; the screen states that it
  records an adapter report and that the gateway did not connect or execute.

- **Secret and authority boundary.** The descriptor exposes authentication
  kind and reference presence, never the reference identifier. Generated
  configuration masks reference identifiers, and every extensible JSON value
  is defensively sanitised before entering markup; component fixtures plant
  both opaque-reference and plaintext credential sentinels and assert neither
  reaches the rendered snapshot. CPR-25's public database acceptance already
  proves credential-shaped imports do not echo, persist in audit or enter
  generated configuration as plaintext. Descriptions, requested permissions
  and schemas are visibly labelled non-authoritative, and no `tools/call`
  control or method exists.

- **Contract and deletion result.** The page uses CPR-25's sixteen generated
  operations and DTOs only. OpenAPI stays **101 operations**, schema epoch stays
  **2**, and the migration chain stays **51** files. The placeholder component,
  placeholder test and dead prose are deleted; no duplicate Tool DTO, API,
  reviewer, secret resolver, local command runner or execution proxy appears.

- **Tests and exact results.** CPR-26 helpers and real-component acceptance
  **10/10**; complete console **196/196**; generated client, backlog and demo
  drift checks PASS; production TypeScript/Vite build PASS (66 modules,
  386.50 kB JavaScript / 109.11 kB gzip, 18.54 kB CSS / 4.23 kB gzip).
  Complete `make ci` **PASS**. `make db-test` is N/A because no persisted,
  policy, RLS or database-backed behaviour changed. No in-app browser session
  was exposed, so real-component SSR plus the production bundle is the honest
  UI evidence and no interactive visual-run claim is made.

- **Commit.** `feat(console): add MCP tool registry experience (CPR-26)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-27 checkpoint under the programme's
  next-checkpoint convention.

### Prompt 25 objective — OKF v0.2 knowledge exchange adapter (CPR-27)

- **Selected feature and specification boundary.** **CPR-27** is delivered
  from `98f5bcd`; it records CPR-26's commit as
  `98f5bcdac7d3313c99cd4bd27ecd6243189a6be3`. ADR-0087 is Accepted and pins
  the still-current canonical Open Knowledge Format v0.2 source to
  `GoogleCloudPlatform/open-knowledge-format` commit
  `ad30107c31c06aec8a7d5636e0d1058118604e6f`. The historical
  `knowledge-catalog/okf` location is frozen and redirects there. This package
  implements only v0.2 behind a `KnowledgeFormatAdapter`; a v0.1 `format`
  fallback is a hard validation error, while unknown v0.2 concept types and
  extension metadata are retained rather than invented into Synveda enums.

- **Pure bounded adapter.** New leaf crate `synveda-okf` validates directory,
  zip, tar, tar-gzip and explicitly identified checked-out Git-tree bytes with
  one set of path and size rules. It normalises logical paths, rejects absolute
  and parent traversal, symlinks/hardlinks/special entries, case collisions,
  unsupported binary or executable material and archive expansion beyond the
  documented entry/per-file/total limits. It parses UTF-8 Markdown plus YAML
  frontmatter, requires a non-empty OKF `type`, inspects reserved v0.2 files
  without treating them as concepts, retains source revision and extensions,
  resolves only bounded internal Markdown links, and performs no URL fetch,
  redirect, Git command, plugin or script execution. Credential-bearing or
  private-address remote source declarations fail before persistence and never
  echo their value.

- **Immutable plans and candidate-only materialisation.** Migration
  `0054_okf_imports.sql` adds `import_jobs`, `import_artifacts`,
  `import_mappings` and `capture_candidate_import_artifacts`, all tenant-bound,
  forced-RLS, indexed and protected by immutable-row triggers where history is
  evidence. An import stores canonical source identity/revision/digest and
  immutable artifacts, then classifies additions, updates, duplicates and
  conflicts in a dry-run mapping. Repeating identical source bytes under the
  same idempotency key returns the same plan. A separate materialise act creates
  capture candidates and proposed relations only; the source XOR constraint
  permits either immutable session-event evidence or immutable OKF artifact
  evidence. Session capture workers claim only session batches. No import path
  inserts a Knowledge head/revision or changes its current projection.

- **One governed publication and provenance path.** The existing candidate
  decision commands remain the sole publication boundary: every accepted OKF
  candidate enters the CPR-16 typed VedaFlow Knowledge effect under a fresh PDP
  decision, precondition and content-free audit event. Acceptance normalises
  both the immutable import artifact and declared OKF source references into
  `KnowledgeSource`; URL/document/repository references therefore retain
  provenance without turning an external declaration into fetch authority.
  Internal links map to proposed Knowledge relations only after their target is
  resolved within the validated artifact set. New audit actions record ids,
  digests, counts and outcomes, never source or Knowledge content.

- **Public import/export contract.** Five generated operations provide
  project-scoped plan creation, cursor-paginated job listing, exact job detail,
  idempotent materialisation and deterministic project export. Every project,
  job, artifact, candidate, Knowledge item and Knowledge source is owned first
  and decided at its exact governing scope; foreign ids remain indistinguishable
  from fictional ids. Export includes only current Knowledge independently
  authorised through the PDP, assigns stable deterministic paths and ordering,
  and preserves source, verification, staleness, relation and extension
  evidence. OpenAPI grows **101 → 106 operations** and **139 → 150 schemas**;
  generated TypeScript follows it. New Learnings now renders immutable OKF
  artifact provenance instead of fabricating a session address and forecasts
  `knowledge.write` at that candidate's destination.

- **Schema, deletion and threat result.** Schema epoch remains **2**; the chain
  is now **52 migration files**, **669** checked SQLx query descriptions and
  **87** forced-RLS tenant tables (92 tables and four views in the fresh-schema
  inventory). Nothing reads, writes or translates the retired record plane.
  No direct publication, dual write, compatibility format, scheduled Git sync,
  remote fetch, SSRF-capable redirect, unbounded decompressor, symlink escape,
  gateway execution seam or content-bearing audit path was added. CPR-28 owns
  user-facing filesystem CLI and console workflows; this backend accepts inert
  bytes through the public API rather than granting the gateway host paths.

- **Tests and exact results.** Pure adapter unit/integration **6/6**, shared
  import types **1/1**, store import persistence **1/1**, public database-backed
  OKF lifecycle **1/1**, capture regressions **4/4**, OpenAPI **5/5**,
  forced-RLS completeness **1/1**, complete console **197/197**, SQLx prepare
  check, crate-dependency, generated-client, backlog and demo-drift checks pass.
  The isolated `demos/cpr-27-okf-v02.sh` reports one job, three artifacts, two
  mappings, two reviewable candidates, one VedaFlow-published Knowledge item
  and two normalised sources. Complete `make ci` **PASS** and full
  `make db-test` **PASS** against disposable `synveda_test_1177`. The demo is
  deterministic local archive/API evidence and is not represented as a remote
  Git host, network-source or live third-party verification.

- **Commit.** `feat(okf): add v0.2 knowledge exchange adapter (CPR-27)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the CPR-28 checkpoint under the programme's
  next-checkpoint convention.

### Prompt 26 objective — OKF CLI and console (CPR-28)

- **Selected feature and boundary.** **CPR-28** is delivered from `0dbf163`;
  it records CPR-27's commit as
  `0dbf163d67dc1aba78de5f79089a47e5c989de48`. Accepted ADR-0087 already fixes
  one v0.2 adapter, inert-byte public API and candidate-only publication, so
  this client package adds no ADR, schema, Cedar action or audit action.

- **Filesystem-owning public client.** New `synveda okf
  validate|inspect|import|export` commands share the exact `synveda-okf` leaf
  adapter. A directory is enumerated with bounded canonical paths and no
  `.git` administration bytes; zip, tar and tar-gzip remain bounded inert
  input; an explicit revision labels an already checked-out tree without
  running Git. Validate and inspect are local. Import sends canonical bytes to
  the public project plan operation under a content-derived stable idempotency
  key and only invokes the separate candidate materialisation operation when
  not in dry-run mode. Export accepts only a pinned, internally consistent
  response, then writes a mode-0700 staging tree and atomically renames it to a
  new output directory; traversal, duplicate/non-bytewise paths, inconsistent
  hashes/digest and overwrite fail before publication.

- **Generated-contract product surface.** `/console/okf` is the primary
  project **Import / Export** page. It packages either an explicit folder or
  one archive, and calls only the generated plan/list/detail/materialise/export
  operations. It shows validation and source revision, immutable history and
  progress, additions/updates/duplicates/conflicts, exact artifacts,
  producer-defined types, unknown extension frontmatter and proposed links.
  Candidate materialisation links the same New Learnings workflow used by
  sessions and never claims publication. Export selects current project
  Knowledge through the generated collection and renders stable logical paths,
  file/content hashes, exact Markdown downloads and the bundle digest.
  Project-scope capability forecasts remove unavailable controls while the
  gateway remains authoritative.

- **Round trip and hard boundary.** The real PulseBoard fixture contains the
  unknown `pulseboard-practice` type, `x-owner`, `x-retention-class`, an
  official decision and an internal relation. Pure adapter tests and the
  database-backed CPR-27 API test prove the type/extensions survive
  inspect → plan → candidate → VedaFlow accept → export. There is still no
  server path, Git process, remote fetch, content execution, direct Knowledge
  publication, scheduled synchronisation, Synveda-only bundle format or
  compatibility reader.

- **Contract/schema result.** The generated contract remains **106 operations**
  and **150 schemas**; epoch **2** remains **52 migration files**, **669** SQLx
  descriptions and **87** forced-RLS tenant tables. No database-backed
  behavior changed, so `make db-test` is not applicable; the public API
  lifecycle is nevertheless rerun against a disposable database in the demo.
  README, beta and install guidance now name the exact CLI and console flows.

- **Tests and exact results.** Pure adapter **6/6**, focused CLI OKF **3/3**,
  complete CLI **150/150**, console helpers/components/generated request shape
  **10/10**, complete console **207/207**, clippy, crate layering, generated
  API, backlog and the **77-script** demo drift gate pass. The production
  console builds at 68 modules, 402.53 kB JavaScript (113.56 kB gzip) and
  18.84 kB CSS (4.29 kB gzip). Isolated
  `demos/cpr-28-okf-workflows.sh` validates and inspects the real local fixture,
  passes the public import/materialise/accept/export test **1/1** and reruns the
  generated console acceptance. Complete `make ci` **PASS**. This is local and
  deterministic public-API evidence, not remote Git-host or live third-party
  verification.

- **Commit.** `feat(console): add OKF import and export (CPR-28)` on
  `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository convergence objective — public contract and client cutover (CPR-29)

- **Selected feature and decision.** **CPR-29** is delivered from `683a17d`;
  it records CPR-28's commit as
  `683a17d30a812d160781cccf16c8633e9251f425`. Accepted ADR-0088 fixes the
  boundary: one authenticated application contract and one executable route
  catalogue, with the unauthenticated login exchange, operational health and
  metrics, and standards-defined `/scim/v2` protocol remaining deliberately
  separate. Governed `/v1/scim/credentials` administration is part of the
  application contract.

- **One executable and documented inventory.** New `routes.rs` owns each
  method, path and handler once; the same declaration constructs the Axum
  router and exposes the inventory the OpenAPI acceptance suite compares in
  both directions. The hand-maintained route list and the separate SCIM
  credential merge are deleted. All previously undocumented governance,
  capability, policy, directory, audit, channel, prompt/pack, lapse,
  quarantine, proposal, service-identity and SCIM-credential handlers now have
  generated request/response schemas, bearer security, common error envelopes
  and truthful precondition metadata. The contract grows **106 → 156
  operations** and **150 → 238 schemas**; a source guard also proves no
  authenticated `/v1` route is mounted outside the catalogue.

- **Generated console boundary.** `console/src/api.mts` now owns transport and
  browser-session mechanics only. Reviews, Scopes, Policies, Audit, Service
  identities and Onboarding call generated operations and consume generated
  types; handwritten wire DTOs and application wrappers are deleted. The
  generator now renders multiple-member `allOf` as TypeScript intersections
  and null-only schemas correctly. The cutover exposed and fixed two client
  assumptions: the default-policy response is its documented object rather
  than a scope assignment, and audit's Recent view must first resolve the
  chain head before requesting a forward cursor page. Generated artifacts are
  current and the transport test fails if another handwritten application
  operation appears.

- **CLI and adapter cutover.** Ordinary `synveda service` registration,
  listing and revocation and `synveda audit` verification/tail now use bearer-
  authenticated public routes; their modules carry source guards against store
  authority. Direct local database access remains only for documented
  bootstrap/reset/migration, key and secret custody, and break-glass policy-
  pack operations. The generic MCP server resolves workspace/project through
  `/v1/me`, opens and appends to public sessions, queries scoped current
  Knowledge, and advertises only exact available Skill and approved project
  Tool version/digest evidence. Imported tool command/configuration and secret
  references never enter its tool surface, and declared tools explicitly grant
  no authority. Project selection survives generated client configuration.
  The Claude adapter gains a contract test proving every route it calls exists
  in OpenAPI and that no store, SQLx or retired global runtime route appears.

- **Deletion and schema result.** The remaining handwritten console
  application client and duplicate DTOs, direct-store service/audit CLI paths,
  MCP-private `/v1/me` and session response models, and the router's second
  SCIM merge are deleted. No compatibility route, DTO alias, fallback reader,
  dual read/write or storage-coupled adapter was added. Epoch **2** remains
  **52 migration files**, **669** checked SQLx descriptions and **87**
  forced-RLS tenant tables; no schema, Cedar action, policy pack or audit
  action changed.

- **Tests and exact results.** OpenAPI parity/source guards **6/6**, service
  identities **5/5**, audit query **13/13**, focused CLI service **1/1**,
  audit **2/2** and MCP **44/44**, complete CLI **156/156** plus authentic MCP
  corpus **5/5**, complete console **208/208**, Claude adapter **98/98**,
  clippy, crate layering, generated API, backlog, ADR and **78-script** demo
  drift checks pass. The production console builds at 68 modules, 404.70 kB
  JavaScript (113.93 kB gzip) and 18.84 kB CSS (4.29 kB gzip). Isolated
  `demos/cpr-29-public-contract.sh` runs the database-backed public identity
  and audit paths plus client-boundary guards and passes. Complete `make ci`
  **PASS** and full disposable-Postgres `make db-test` **PASS**. This package
  claims public-contract and deterministic client evidence; it adds no new
  live external-client claim.

- **Commit.** `refactor(api): complete public contract and client cutover
  (CPR-29)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository programme objective — governed runtime configuration artifacts (CPR-30)

- **Selected feature and decision.** **CPR-30** is delivered from `b33ba51`;
  it records CPR-29's commit as
  `b33ba51c0101c171f1be43e209002c1cd21a127a`. Accepted ADR-0089 fixes one
  model: a stable Configuration aggregate, immutable complete versions and
  revisioned nearest-scope bindings. `personal`, `team` and `enterprise` are
  canonical source documents copied into ordinary governed history, never an
  edition switch or runtime branch. With no binding the immutable enterprise
  document is the fail-safe, not a mutable hidden row.

- **Schema and hard cut.** Migration `0055_governed_configuration` adds
  `configuration_artifacts`, `configuration_versions`,
  `configuration_bindings` and `configuration_changes`, with tenant-qualified
  ownership, exact current/pinned-version constraints, immutable-version and
  append-only binding-history triggers, enabled and forced RLS and indexed
  nearest-scope resolution. It drops `policy_pack_defaults` and
  `policy_pack_assignments` without reading or translating them. Context
  candidate/selection rows now identify their configured channel and carry
  exactly one address family: immutable Knowledge item/revision or reviewable
  CaptureCandidate. Epoch **2** now has **53 migration files**, **687** checked
  SQLx descriptions and **91** forced-RLS tenant tables.

- **One governed mutation path.** Create, publish, bind, enable/disable,
  pin/unpin and rollback are typed `Configuration/apply` VedaFlow changes.
  Before proposal and again before effect, the gateway checks aggregate/scope
  ownership, `ConfigurationWrite`, `ProposalOpen`, the live approval matrix,
  canonical payload hash, expected head and expected binding revision. A
  permissive matrix may auto-apply, but never skips the proposal, immutable
  version or audit chain. Four content-free configuration audit actions carry
  ids, revisions, digests, template provenance and deciding pack. The embedded
  packs advance to version 22 because their complete action and approval
  matrices now cover Configuration.

- **Exact runtime evidence.** The complete document selects the Cedar pack;
  explicit/session-end capture rules and bounds; context budget, channels and
  trace retention; type-aware freshness; Skill/Tool advertisement; and allowed
  external-provider families. Capture freezes version/digest with its event
  snapshot, and workers reload that immutable version. Context planning records
  the same evidence and obeys its cap and channel set. The optional unreviewed
  channel separately re-authorises source session/import evidence and proposed
  destination before retaining an address or text, labels rendered content
  `[UNREVIEWED CANDIDATE]`, and cannot receive immutable-revision feedback.
  Disabling current Knowledge really performs no Knowledge search; an empty
  scope set cannot widen into an unfiltered read.

- **Generated product surfaces and deletions.** Six net operations grow the
  exact authenticated contract **156 → 162 operations** and **238 → 255
  schemas**: templates; cursor-paginated artifacts, versions and bindings;
  comparison; exact effective resolution; and idempotent revision-preconditioned
  mutations. `synveda configuration templates|list|show|effective|compare|
  create|publish|bindings|bind|update-binding|rollback` is a public-HTTP client
  with no store authority. Advanced Configuration and scope-effective detail
  use generated operations/types. The mutable default/scope assignment API,
  old Policies screen, assignment fixtures and direct setters are deleted; an
  in-memory `PolicyAssignment` exists only as the resolved document projected
  into the embedded PDP.

- **Gate findings.** Full database execution caught four real convergence
  defects and one incomplete inventory: a shared scope-read audit payload had
  renamed canonical `op`; address-bearing AUD-2 fixtures relied implicitly on
  trace retention; three lifecycle fixtures bound before minting their root;
  service confinement still probed the deleted policy-default route; and the
  RLS coverage list omitted the four already-forced tables. Each is corrected
  at its contract boundary and pinned by the focused suite rather than hidden
  or excluded.

- **Tests and exact results.** Configuration domain **4/4**, public database
  API **1/1**, capture **4/4**, context **3/3**, policy approvals **6/6**, packs
  **7/7**, PDP **11/11**, audit events **3/3**, audit queries **13/13**,
  service identities **5/5**, forced RLS **83/83**, OpenAPI **6/6** and complete
  console **210/210** pass. Generated API/SQLx, dependency, licence, backlog,
  ADR and **79-script** demo-drift checks pass. The console production bundle
  is 418.15 kB JavaScript (116.92 kB gzip) and 18.84 kB CSS (4.29 kB gzip).
  Isolated `demos/cpr-30-governed-configuration.sh` passes with two artifacts,
  three versions, two bindings, six audited applications and zero replaced
  assignment tables. Complete `make ci` and uninterrupted disposable-Postgres
  `make db-test` **PASS**. This is executable local application evidence, not a
  live external-provider claim.

- **Commit.** `feat(configuration): version governed runtime profiles
  (CPR-30)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository governance objective — governed auto-apply and policy relaxations (CPR-31)

- **Selected feature and decision.** **CPR-31** is delivered from `ed7d233`;
  it records CPR-30's commit as
  `ed7d233879e96bebf8030c1d3d135fd5df2a2cbe`. Accepted ADR-0090 fixes the
  successor boundary: a relaxation is one stable aggregate with immutable
  reviewed versions, not an alternate authorisation path. Each version names
  one provisioned identity, its frozen principal spelling, one exact
  non-principal scope, the closed `knowledge.read` action, a sensitivity
  ceiling, requested window, database-time hard expiry, reason, creator,
  exact approvers and the effective Configuration version/digest.

- **One governed mutation path.** Create, revise and revoke are typed
  `Policy/apply` VedaFlow effects. Before proposal and again before effect,
  the gateway checks aggregate and tenant ownership, exact subject and scope,
  `RelaxationWrite`, `ProposalOpen`, the live pack matrix, canonical payload,
  current Configuration bounds and stale-head preconditions. Open
  collaboration may satisfy that same persisted change with zero explicit
  approvals; standard retains it for an administrator; regulated retains it
  for two distinct administrators. Rejection remains terminal and applies no
  version. Existing Knowledge, Capture/OKF, Skill, Tool and Configuration
  acceptance suites were rerun to prove that their personal auto-apply paths
  still persist typed VedaFlow history, immutable versions and content-free
  audit rather than branching around governance.

- **Authorisation and expiry.** Request gathering selects only database-time
  active rows matching the authenticated principal and the current
  Configuration's narrowed action/duration controls. Cedar receives those
  immutable matches as `context.relaxed` and alone decides the exact
  Knowledge read; personal-scope privacy, quarantine and service-identity
  confinement remain base-layer forbids. Expiry therefore withdraws authority
  without relying on a worker. The sweep records one content-free,
  hash-chained expiry event and never extends or recreates permission.

- **Schema and hard cut.** Migration `0056_governed_policy_relaxations` adds
  `policy_relaxations`, `policy_relaxation_versions` and
  `policy_relaxation_changes`, with tenant-qualified ownership, immutable
  version and append-only change triggers, enabled and forced RLS, exact
  current-head constraints and bounded indexes. It drops `policy_lapses`, its
  effect type, policy-pack setting, functions, store module and public routes
  without reading or translating old rows. Epoch **2** now has **54 migration
  files**, **693** checked SQLx query descriptions and **91** tenant tables in
  the forced-RLS completeness inventory.

- **Generated product surfaces.** Two net operations grow the exact
  authenticated contract **162 → 164 operations** and **255 → 260 schemas**:
  cursor-paginated governed relaxation reads plus idempotent create,
  revision-preconditioned revise and revoke commands. `synveda relaxation
  list|show|create|revise|revoke` is an HTTP-only client. Advanced Scopes uses
  generated operations and types to show the exact subject, target, action,
  immutable versions, governance outcome and expiry/revocation state;
  Configuration links to that single surface. The old lapse demo is replaced
  by the governed-relaxation acceptance narrative.

- **Gate findings.** Database execution found that the new closed action
  vocabulary parsed `knowledge.read` but its derived serializer emitted
  `knowledge_read`; the enum now pins the canonical dotted spelling and
  explicitly rejects the underscore form. That correction necessarily moved
  a Configuration digest, so the explorer fixture was regenerated through
  its recorder and immediately replayed. The RLS completeness inventory also
  had all three new forced tables but listed them on the wrong side of
  `policy_packs`; its sorted-order assertion is restored. None of these
  failures was suppressed or excluded.

- **Tests and exact results.** Types **210/210** plus serde **50/50**, policy
  relaxation **3/3**, public lifecycle **2/2**, audit **27/27**, service
  confinement regression **1/1**, forced RLS **83/83**, OpenAPI **6/6**,
  complete CLI **155/155** plus authentic MCP corpus **5/5**, retrieval
  **53/53** (one deliberate load test ignored), complete console **209/209**
  and production build pass. Generated API/SQLx, dependency, licence, backlog,
  ADR and **79-script** demo-drift checks pass. The console bundle is 418.50 kB
  JavaScript (116.95 kB gzip) and 18.85 kB CSS (4.29 kB gzip). Isolated
  `demos/cpr-31-governed-relaxations.sh` passes with two stable aggregates,
  three immutable versions, five governed changes and no `policy_lapses`
  table. Complete final-byte `make ci` and full disposable-Postgres
  `make db-test` **PASS**, the latter against `synveda_test_35856`. This is
  deterministic local product evidence and adds no external-provider claim.

- **Commit.** `feat(governance): add governed auto-apply and relaxations
  (CPR-31)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository governance objective — unified approvals and review (CPR-32)

- **Selected feature and decision.** **CPR-32** is delivered from `9281951`;
  it records CPR-31's commit as
  `92819516ee35abf3f5a0fe6cd8c0658f666269af`. Accepted ADR-0091 keeps one
  VedaFlow proposal/change rather than building review systems per noun. A
  closed `ArtifactFamily` and validated `ArtifactReference` bind each proposal
  to stable aggregate or binding ids, the exact operation, immutable
  version/digest and any stale-head precondition. Knowledge additionally names
  immutable OKF import evidence where applicable; authored multi-member
  proposals carry one reference per member.

- **One live approval calculation.** The inherited pack matrix and nearest
  curator requirements still resolve on every act. Rules now monotonically
  add an author self-review prohibition and, where configured, require the
  effect actor to differ from both the author and every counting reviewer.
  `open-collaboration` retains intentional personal auto-apply only where the
  complete live requirement is empty; `standard` separates author and
  reviewer; `regulated-strict` also separates the executor. Cedar remains the
  source of authority: these rules narrow otherwise allowed combinations and
  never grant an action.

- **Commit- and revision-bound lifecycle.** Approve and reject both require
  the exact proposal commit the reviewer inspected, with a stale commit
  rejected before a review or close row is written. Rejection still requires a
  reason; an author barred from reviewing cancels through the existing
  proposer withdrawal semantics. Apply/publish repeats ownership, PDP, live
  matrix, separation, canonical payload and artifact revision checks. Every
  production caller—Knowledge and OKF, Skills, Tool servers/bindings,
  Configuration, policy relaxations, Prompts, Context Packs and the pre-cut
  authored-Memory path—constructs the common typed evidence.

- **Schema and security.** Migration `0057_unified_artifact_approvals` adds a
  mandatory bounded JSONB reference array, database validator, family GIN
  index and immutability trigger to `vedaflow_proposals`. It creates no second
  table, default, backfill or old-row translation. The proposal remains
  tenant-bound under enabled and forced RLS, and opened/reviewed/closed audit
  metadata carries ids, hashes, references and separation flags without
  artifact content or secrets. Epoch **2** now has **55 migration files**,
  **693** checked SQLx descriptions and **91** tenant tables in the forced-RLS
  completeness inventory.

- **Generated product surface.** The exact authenticated application contract
  remains **164 operations** and grows **260 → 262 schemas** for typed
  references and lifecycle entries. Proposal listing accepts an artifact
  family filter; list/detail responses expose references, live requirement,
  approvals and deterministic content-free timeline. Advanced Reviews uses
  only generated operations/types to inspect the family and exact version,
  submit commit-bound verdicts, cancel as proposer and execute an approved
  effect. New Learnings remains the lightweight capture decision surface and
  session-event quarantine remains pre-extraction admission control; neither
  is falsely counted as an artifact review implementation.

- **Gate finding.** The first complete database run reached two old direct-SQL
  test fixtures that omitted the newly required references. Production callers
  and focused gateway suites were already correct; the fixtures now construct
  the same typed Configuration, relaxation and authored-Memory addresses, so
  RLS tests reach the tenant boundary instead of failing early on a non-null
  constraint. The corrected policy-pack **5/5** and forced-RLS **83/83** suites
  pass, and regenerated SQLx metadata compiles fully offline.

- **Tests and exact results.** Types **212/212** plus serde **50/50**, policy
  **77/77**, VedaFlow **73/73** plus object-store **10/10**, public
  Configuration **1/1**, Knowledge **4/4**, OKF **1/1**, relaxations **3/3**,
  Skills **1/1**, Tools **1/1**, Context Packs **10/10**, Prompts **6/6**,
  OpenAPI **6/6**, complete console **210/210**, policy-pack store **5/5** and
  forced RLS **83/83** pass. Generated API/SQLx, dependency, licence, backlog,
  ADR and **80-script** demo-drift checks pass. Isolated
  `demos/cpr-32-unified-approvals.sh` passes with 81 typed proposals across
  seven families, 23 exact-commit review acts, regulated three-person
  separation and zero audited artifact content. Complete final-byte `make ci`
  and full disposable-Postgres `make db-test` **PASS**, the latter against
  `synveda_test_43866`. This is deterministic local evidence and adds no live
  external-provider claim.

- **Commit.** `feat(governance): extend approvals across artifact families
  (CPR-32)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository governance objective — audit query and export (CPR-33)

- **Selected feature and decision.** **CPR-33** is delivered from `cf52f34`;
  it records CPR-32's commit as
  `cf52f34b4d408ef147310041f9367b1e445b4162`. Accepted ADR-0092 extends the
  existing tenant-complete hash chain rather than creating a replay engine,
  search projection or second audit truth. Tenant-root `AuditRead`, forced RLS
  and one content-free audit event after each answer remain the boundary.

- **Current-noun questions.** The cursor-keyset event query accepts exact
  typed artifact family/id/version, session and context-run filters using JSON
  containment; unknown families, incomplete dependent filters, control
  characters and invalid cursors fail rather than becoming empty findings.
  Terminal applied, rejected, superseded, archived, restored, forgotten,
  expired, Skill, Tool, Configuration and relaxation evidence repeats the
  immutable typed artifact reference. Context retrieval, selection and
  composition record the effective Configuration aggregate, binding and
  binding scope, immutable version/hash/policy pack, every gathered relaxation
  version, exact selected Knowledge revisions in address-retaining modes and
  exact advertised Skill bindings/versions.

- **Bitemporal and retention honesty.** `GET /v1/audit/knowledge` separates
  semantic `valid_at` from delivery/transaction `as_known_at`, accepts a
  reverse sequence cursor and folds the latest retained identity in chain
  order. It joins only content-free immutable revision interval/hash evidence;
  content remains behind a separate Knowledge decision. Erased, malformed and
  hashes-only evidence is returned under `unresolved`. A hashes-only row keeps
  its content hash but never grows a synthetic Knowledge item/revision id, and
  an empty address page still advances by the last event considered.

- **Frozen export and clients.** `GET /v1/audit/export` captures sequence and
  head hash before auditing its own read, then walks that fixed contiguous
  prefix at at most 1,000 rows per page. The envelope pins canonical format,
  hash rule, tenant-bound genesis and frozen head; the offline verifier
  recomputes every content/link hash and refuses mutation, gaps, reordering,
  incompleteness or another tenant. `synveda audit events|knowledge|export`
  uses public HTTP only; export verifies before an atomic no-overwrite write,
  while `verify-export` needs neither profile nor database. Advanced Audit
  uses generated operations to filter and download the same frozen snapshot.
  This is deterministic offline evidence, not SIEM delivery or WORM storage;
  AUD-3/AUD-4 remain explicit extensions.

- **Schema and generated contract.** Migration
  `0058_context_audit_export` adds one tenant-leading JSONB containment index
  and no data translator, table or canonical audit-byte change. Schema epoch
  **2** now has **56 migration files**, **700** checked SQLx query descriptions
  and **91** tenant tables in the forced-RLS completeness inventory. One net
  export operation grows the authenticated contract **164 → 165 operations**
  and typed audit evidence grows **262 → 264 schemas**; generated TypeScript is
  the console's only application contract.

- **Gate findings.** A final retention audit found that addressless
  hashes-only composition entries were being discarded before they could
  reach the promised unresolved set; parsing and folding now key those rows by
  retained content hash without inventing identity, with both pure and real
  context-run regressions. Strict Clippy also reduced the atomic writer's saved
  error propagation to the direct `?` form after temporary-file cleanup. The
  first unprivileged CI attempt could not bind the two existing loopback test
  listeners; the unchanged gate passed with local-loopback permission. The
  long-lived pre-hard-cut developer database was not reset or translated when
  it refused migration 0057's mandatory typed refs; compilation and all
  database evidence used fresh epoch-2 databases as the hard cut requires.

- **Tests and exact results.** Audit unit **23/23** plus tamper **7/7**,
  gateway audit **16/16**, terminal-reference regressions across Knowledge,
  Skills, Tools, Configuration and relaxations **5/5**, focused CLI audit
  **4/4**, complete CLI **157/157** plus MCP corpus **5/5**, OpenAPI **6/6**,
  complete console **212/212**, and forced-RLS completeness pass. Generated
  API/SQLx, strict Clippy, dependency, licence, backlog, ADR and **81-script**
  demo-drift checks pass. Isolated `demos/cpr-33-audit-export.sh` passes with
  seven self-audited export reads, 49 typed artifact events and exactly one
  tenant-leading payload index. Complete final-byte `make ci` and full
  disposable-Postgres `make db-test` **PASS**, the latter against
  `synveda_test_51591`. This is deterministic local evidence and adds no live
  external-provider claim.

- **Commit.** `feat(audit): query and export the context platform chain
  (CPR-33)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

### Repository enterprise objective — directory adapter convergence (CPR-34)

- **Selected feature and decision.** **CPR-34** is delivered from `3c61e5e`;
  it records CPR-33's commit as
  `3c61e5e0fa35f8e9a0056f1e7d53a19bfe43debc`. Accepted ADR-0093 completes
  ADR-0059's post-epoch successor: a directory user retains protocol/source
  attributes around one tenant-owned Identity and principal scope, while a
  directory group is the shared Group aggregate rather than a second access
  graph. External directory facts state identity and membership; they grant
  no product authority until the separately governed assignment command
  passes `MembershipGrant` at the exact scope.

- **One projection and removal semantics.** `groups` now retains directory
  source, stable provider resource id and optional protocol external id;
  `group_members` keys stable `IdentityId`, so membership is complete before
  first login and the effective-authority query emits only an active identity
  with a bound principal. SCIM push and scheduled Entra/Okta pull call the same
  atomic replacement projection. Pull snapshots carry stable group ids and
  member external ids; a complete pass may retire a missing source-owned
  Group, while an incomplete pass establishes presence and concludes nothing
  about absence. Group retirement, membership removal and identity disable
  therefore withdraw access on the next ordinary anchor resolution with no
  copied grants, directory policy table or stale cache.

- **Governed assignments and ownership.** Two generated public operations
  create and revoke source-evidenced, group-subject `scope_grants`. Creation
  requires `Idempotency-Key`; both directions resolve the owned scope before
  deciding, use the tenant RLS transaction and chain the existing
  `access.granted`/`access.revoked` actions without payloads or credentials.
  Ordinary group and grant mutation routes refuse directory-owned rows by
  name. Identical external ids are source- and tenant-qualified, and a
  source-less lookup with multiple matches fails closed rather than choosing
  one.

- **Hard cut, schema and contract.** Migration
  `0059_directory_adapter_convergence` refuses affected old directory/group
  rows with the exact `synveda reset --database --force` instruction, then
  deletes `scim_groups`, `scim_group_members` and the mirror DTO vocabulary
  without translation. The migration rebuilds identity-keyed membership,
  source-qualifies directory users and extends immutable directory evidence on
  shared Groups and grants. The refusal was proven on a disposable pre-cut
  database. Schema epoch **2** now has **57 migration files**, **694** checked
  SQLx descriptions and **89** tenant tables in the forced-RLS completeness
  inventory. The exact contract grows **165 → 167 operations** and
  **264 → 266 schemas**; regenerated TypeScript remains the console's only
  product contract.

- **Gate finding.** The first full database run found one CPR-6 HTTP fixture
  still placing a token subject in the now-UUID group member body. It was not
  made compatible: the fixture now provisions the stable Identity, while the
  group write still traverses the public PDP-governed route. The focused
  regression and complete nine-test anchor API suite pass. Strict Clippy and
  the generated-console idempotency inventory also caught and fixed one stale
  test reference each before the final gates.

- **Tests and exact results.** Identity connector fixtures **5/5**; store
  access **30/30**, anchors **13/13** and directory sync **8/8**; gateway
  access **18/18**, directory sync **9/9**, SCIM **10/10** and anchors **9/9**;
  OpenAPI **6/6**, complete console **212/212**, forced RLS **83/83**, full
  offline workspace compilation and SQLx prepare/check pass. Isolated
  `demos/cpr-34-directory-convergence.sh` passes with three shared directory
  groups, six chained transitions, identity-keyed membership and zero old
  mirror tables. The complete **82-script** demo-drift gate, final-byte
  `make ci` and full disposable-Postgres `make db-test` pass; the successful
  scratch database was removed by the harness. Entra/Okta evidence remains
  explicitly captured/transcribed fixture evidence because no live vendor
  tenant was available; this package makes no live-verification claim.

- **Commit.** `refactor(directory): use principals groups and scope grants
  (CPR-34)` on `feat/context-platform-mvp`.
- **Commit hash.** Written by the next checkpoint under the programme's
  next-checkpoint convention.

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
| Hierarchy & policy explorer | `Explorer.tsx` | `/v1/hierarchy/root`, `…/children`, `…/policy`, `…/roles`, `…/capabilities`, `/v1/lapses` |

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
- **Commit hash.** Written by Prompt 8, on Prompt 1's rule.

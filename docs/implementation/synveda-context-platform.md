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
- **Commit hash.** Written by Prompt 4, on Prompt 1's rule.

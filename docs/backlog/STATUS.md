# Feature inventory

142 features in this index. This file is authoritative for feature identity,
phase and delivered/open state. Delivered names identify historical slices;
current contracts live in code, generated artefacts and accepted ADRs, while git
retains their implementation evidence. Open entries link to current briefs.

110 delivered; 32 open. The inventory and open-brief shape are checked in CI.

## Phase 0 — Foundation (wk 1)

- [x] FND-1: Workspace scaffold — delivered 2026-07-16
- [x] FND-2: Dev environment — delivered 2026-07-17
- [x] FND-3: synveda-types + error model — delivered 2026-07-18
- [x] FND-4: Migrations & bitemporal base tables — delivered 2026-07-18
- [x] FND-5: Observability baseline — delivered 2026-07-18
- [x] FND-6: ADRs 0001–0004 — delivered 2026-07-18

## Phase 1 — The spine (wk 2–5)

- [x] TEN-1: Tenant model & resolution — delivered 2026-07-18
- [x] TEN-2: Postgres row-level security as backstop — delivered 2026-07-18
- [x] AUTH-1: OIDC login (code+PKCE) — delivered 2026-07-18
- [x] HIER-1: Hierarchy store — delivered 2026-07-18; ADR-0074
- [x] AUTHZ-1: Cedar PDP embedded — delivered 2026-07-18
- [x] AUTH-2: JIT user provisioning from claims — delivered 2026-07-18
- [x] AUTHZ-2: Policy packs — delivered 2026-07-19
- [x] AUTHZ-3: Roles & role bindings — delivered 2026-07-19; ADR-0074
- [x] HIER-2: Scope chain resolver — delivered 2026-07-19; ADR-0074
- [x] HIER-3: Cedar entity sync — delivered 2026-07-19
- [x] AUTH-3: Service identities — delivered 2026-07-19
- [x] AUD-1: Hash-chained audit log — delivered 2026-07-19
- [x] MEM-1: observe API + PGMQ buffer — delivered 2026-07-19
- [x] MEM-2: Redaction & secret scanning — delivered 2026-07-19
- [x] MEM-3: Extraction pipeline — delivered 2026-07-22
- [x] MEM-4: Transactional embed-or-fail — delivered 2026-07-22
- [x] CTX-1: Hybrid retrieval — delivered 2026-07-23
- [x] CTX-2: Composition engine — delivered 2026-07-23
- [x] CTX-3: inject API — delivered 2026-07-23
- [x] ADPT-1: Claude Code adapter — delivered 2026-07-25
- [x] EVAL-1: Eval harness skeleton — delivered 2026-07-25

## Phase 2 — Governance (wk 6–10)

- [x] FLOW-1: Object store — delivered 2026-07-25; ADR-0030
- [x] FLOW-2: Channels — delivered 2026-07-25; ADR-0031
- [x] FLOW-3: Proposals & approval matrix — delivered 2026-07-25; ADR-0032
- [x] FLOW-4: Auto-promotion rules — delivered 2026-07-25; ADR-0033
- [x] FLOW-5: Cross-scope promotion — delivered 2026-07-25; ADR-0034
- [x] FLOW-6: CLI review flow — delivered 2026-07-25; ADR-0035
- [x] FLOW-7: Rollback & pinning — delivered 2026-07-25; ADR-0036
- [x] AUTHZ-4: Lapses (controlled relaxation) — delivered 2026-07-26; ADR-0037
- [x] AUTHZ-5: ABAC conditions — delivered 2026-07-26; ADR-0038
- [x] MEM-5: Always-on dedup & conflict detection — delivered 2026-07-26; ADR-0039
- [x] MEM-6: Decay, TTL & staleness — delivered 2026-07-26; ADR-0040
- [x] CTX-4: Tiered injection / progressive disclosure — delivered 2026-07-27; ADR-0041
- [x] CTX-5: recall API + MCP tool — delivered 2026-07-27; ADR-0042
- [x] GRPH-1: Multi-graph schema — delivered 2026-07-28; ADR-0043
- [x] GRPH-2: Graph-linking stage — delivered 2026-07-28; ADR-0044
- [x] GRPH-4: AGE performance spike / graph fallback assessment — delivered 2026-07-25; ADR-0029
- [x] AUD-2: Audit query & auditor role surface — delivered 2026-07-28; ADR-0045
- [x] EVAL-2: Extraction quality suite — delivered 2026-07-30; ADR-0046
- [x] EVAL-4: Retrieval & injection quality — delivered 2026-07-31; ADR-0047
- [x] EVAL-5: Security evals — delivered 2026-07-31; ADR-0048
- [x] PRMT-1: Prompt templates as assets — delivered 2026-08-02; ADR-0049
- [x] PRMT-2: Context packs — delivered 2026-08-03; ADR-0050

## Phase 3 — Enterprise (wk 11–16)

- [x] SKIL-1: agentskills.io-compliant model — delivered 2026-08-03; ADR-0051
- [x] SKIL-2: Security scanning gate — delivered 2026-08-03; ADR-0052
- [x] SKIL-3: Skill quality scoring — delivered 2026-08-03; ADR-0053
- [x] SKIL-4: Scope-targeted distribution — delivered 2026-08-03; ADR-0054
- [x] OPS-1: SMB profile — delivered 2026-08-04; ADR-0055
- [x] CNSL-1: Proposals inbox (hero screen) — delivered 2026-08-04; ADR-0056
- [x] ADPT-2: Generic MCP server — delivered 2026-08-05; ADR-0057
- [x] CNSL-2: Hierarchy & policy explorer — delivered 2026-08-05; ADR-0058
- [x] AUTH-4: SCIM 2.0 server — delivered 2026-08-05; ADR-0059
- [x] AUTH-5: Directory sync fallback — delivered 2026-08-07; ADR-0060
- [x] EVAL-3: Public benchmark adapters — delivered 2026-08-09; ADR-0061
- [x] OPS-2: Helm chart / enterprise profile — delivered 2026-08-10; ADR-0062
- [x] TEN-3: Dense-leg retrieval benchmark — delivered 2026-08-10; ADR-0063
- [x] TEN-4: Per-tenant encryption keys — delivered 2026-08-11; ADR-0064
- [x] OPS-8: Release & distribution — delivered 2026-08-11; ADR-0065
- [ ] [OPS-9: Release-shaped beta acceptance](OPS-9.md) — open
- [ ] [OPS-10: Uninstall & cleanup](OPS-10.md) — open
- [ ] [TEN-5: Tenant lifecycle](TEN-5.md) — open
- [ ] [TEN-6: Cross-tenant isolation test harness](TEN-6.md) — open
- [ ] [AUD-3: External immutable audit retention](AUD-3.md) — open
- [ ] [AUD-4: SIEM streaming](AUD-4.md) — open
- [x] GRPH-3: Graph-augmented recall — delivered; ADR-0097
- [ ] [EVAL-6: Load & latency suite](EVAL-6.md) — open
- [ ] [CTX-7: Dense-leg plan stability](CTX-7.md) — open
- [ ] [OPS-3: Residency routing](OPS-3.md) — open
- [ ] [OPS-4: Vector index scale decision](OPS-4.md) — open
- [ ] [ADPT-3: Additional API transport decision](ADPT-3.md) — open
- [ ] [CTX-6: Session compression assist](CTX-6.md) — open
- [ ] [FLOW-8: Git bridge — export](FLOW-8.md) — open

## Phase 4 — Ecosystem

- [ ] [ADPT-4: Python & TS SDKs](ADPT-4.md) — open
- [ ] [ADPT-5: Source-format converters](ADPT-5.md) — open
- [ ] [ADPT-6: LlamaIndex memory adapter](ADPT-6.md) — open
- [ ] [ADPT-7: Semantic Kernel memory connector](ADPT-7.md) — open
- [x] ADPT-8: Observation that survives a session that does not wait — delivered 2026-08-24; ADR-0027
- [ ] [PRMT-3: Prompt experiment evidence](PRMT-3.md) — open
- [ ] [SKIL-5: Authentic Skill usage reporting](SKIL-5.md) — open
- [ ] [MEM-7: Identity stitching](MEM-7.md) — open
- [ ] [OPS-5: Backup/restore & DR](OPS-5.md) — open
- [ ] [OPS-6: Upgrade and rollback discipline](OPS-6.md) — open
- [ ] [OPS-7: Gateway horizontal scale](OPS-7.md) — open
- [ ] [CNSL-3: Audit temporal and disclosure views](CNSL-3.md) — open
- [x] CNSL-4: Knowledge browser — delivered 2026-08-24; ADR-0082
- [ ] [AUD-5: Compliance mapping doc](AUD-5.md) — open
- [ ] [AUTHZ-6: Authorisation scale decision](AUTHZ-6.md) — open
- [ ] [AUTHZ-7: Governed admin-plane mutation](AUTHZ-7.md) — open
- [ ] [TEN-7: Tenant storage partition decision](TEN-7.md) — open
- [ ] [EVAL-7: A second public benchmark](EVAL-7.md) — open

## Phase 5 — Context platform redesign

- [x] CPR-1: Implementation baseline & locked decisions — delivered 2026-08-17; ADR-0068
- [x] CPR-2: Fresh schema epoch, startup guard & local reset — delivered 2026-08-17; ADR-0069
- [x] CPR-3: Generic governed scope substrate — delivered 2026-08-17; ADR-0070
- [x] CPR-4: Workspaces, projects & canonical repository identity — delivered 2026-08-17; ADR-0071
- [x] CPR-5: Membership, groups, grants & invitations — delivered 2026-08-18; ADR-0072
- [x] CPR-6: Governed scope anchors — the PDP re-cut — delivered 2026-08-19; ADR-0073
- [x] CPR-7: The hierarchy cutover — one scope tree — delivered 2026-08-20; ADR-0074
- [x] CPR-8: The console product shell & first-run onboarding — delivered 2026-08-21; ADR-0075
- [x] CPR-9: The foundation audit — hardening the scope and access cutover — delivered 2026-08-22
- [x] CPR-10: The session ledger and runtime API — delivered 2026-08-23; ADR-0076
- [x] CPR-11: The session product experience — delivered 2026-08-24; ADR-0077
- [x] CPR-12: Durable Claude session delivery — delivered 2026-08-23; ADR-0078
- [x] CPR-13: The demo corpus re-point — delivered 2026-08-24
- [x] CPR-14: Live Claude Code session acceptance gate — delivered 2026-08-24; ADR-0079
- [x] CPR-15: Versioned Knowledge aggregate and provenance — delivered 2026-08-24; ADR-0080
- [x] CPR-16: Governed Knowledge mutation lifecycle — delivered 2026-08-24; ADR-0081
- [x] CPR-17: Public Knowledge API, search and browser — delivered 2026-08-24; ADR-0082
- [x] CPR-18: Session-based capture batches and reviewable candidates — delivered 2026-08-24; ADR-0083
- [x] CPR-19: New Learnings lightweight review workflow — delivered 2026-08-24
- [x] CPR-20: Explainable Knowledge context planning and scoped query — delivered 2026-08-24; ADR-0084
- [x] CPR-21: Context Inspector and outcome feedback — delivered 2026-08-24
- [x] CPR-22: Core individual and small-team MVP acceptance — delivered 2026-08-24
- [x] CPR-23: Immutable skill versions, bindings and usage — delivered 2026-08-24; ADR-0085
- [x] CPR-24: Skills Library product experience — delivered 2026-08-24
- [x] CPR-25: Trusted MCP server catalogue and project bindings — delivered 2026-08-25; ADR-0086
- [x] CPR-26: MCP Tools catalogue product experience — delivered 2026-08-25
- [x] CPR-27: OKF v0.2 knowledge exchange adapter — delivered 2026-08-25; ADR-0087
- [x] CPR-28: OKF import and export product workflows — delivered 2026-08-25
- [x] CPR-29: Public contract and client convergence — delivered 2026-08-25; ADR-0088
- [x] CPR-30: Governed runtime configuration artifacts — delivered 2026-08-25; ADR-0089
- [x] CPR-31: Governed auto-apply and policy relaxations — delivered 2026-08-25; ADR-0090
- [x] CPR-32: Unified approvals across governed artifacts — delivered 2026-08-25; ADR-0091
- [x] CPR-33: Context-platform audit query and deterministic export — delivered 2026-08-25; ADR-0092
- [x] CPR-34: Directory adapter convergence — delivered 2026-08-25; ADR-0093
- [x] CPR-35: Context-platform key and secret convergence — delivered 2026-08-25; ADR-0094
- [x] CPR-36: One-runtime deployment convergence — delivered 2026-08-25; ADR-0095
- [x] CPR-37: Conflict, supersession and freshness engine — delivered 2026-08-25; ADR-0096
- [x] CPR-38: Bounded graph-augmented retrieval — delivered 2026-08-25; ADR-0097
- [ ] [CPR-39: Second verified client](CPR-39.md) — open
- [x] CPR-40: Context-platform product and trust evaluation — delivered 2026-08-26; ADR-0099
- [x] CPR-41: One-command realistic product demo — delivered 2026-08-26; ADR-0100
- [x] CPR-42: Context-platform security and product-integrity audit — delivered 2026-08-26; ADR-0078
- [x] CPR-43: Final context-platform hard cut — delivered 2026-08-26; ADR-0069
- [x] CPR-44: Production hardening and maintainability cut — delivered 2026-08-26; ADR-0101
- [ ] [CPR-45: Docker-first portable reference deployment](CPR-45.md) — open

CPR-45's current identity/database checkpoint has fresh revision-2 authority
fingerprints, 657 validated SQLx records and a passing complete exact-role
database gate. Its independently reviewed collision-resistant fixture
allocator and fresh deterministic authentic-frame lifecycle also pass. The
canonical development graph now has bounded `up`, `smoke`, `down` and
exact-confirmation `reset` actions that invoke project-scoped secret/issuer
generation, audited tenant convergence, optimized Keycloak realm/profile/demo
convergence, exact issuer diagnostics and containerized gateway/worker startup.
One exact-project lock spans preparation and Docker mutation; deterministic
tests cover signal/process-group cleanup, stale-lock refusal, network/IPAM and
asset drift, reset ownership, and atomic issuer publication. The Keycloak
profile keeps unmanaged attributes disabled and its ownership markers
admin-only; drifted marker provenance fails closed. Development source builds
now refuse ambient BuildKit/Buildx/Bake routing before helpers or locking, pin
the local default builder behind fresh private state, require one running
embedded `docker` driver/node at endpoint `default`, preserve registry auth
opaquely, exclude its config and lifecycle temporaries from the source context,
and separate build from every no-build startup/recovery path. The next blocker
now has a reversible, exact-confirmation `.test` host ownership ceremony whose
scratch acceptance proves collision/drift refusal, same-inode metadata
retention, and strict-prefix and sidecar-stage interruption recovery. No host
change has been made. The current source now adds a fresh-project initial-asset
absence gate, closed proxy assertions before the first RUN in every deployment
image stage, and a pinned sandboxed no-capture Playwright fixture. Its exact
development overlay waits for one Keycloak authorization-code/PKCE S256
administrator login and logout before ordinary smoke; deterministic contract,
lifecycle and injected-flow tests pass, but the container has not run. The next
slice now has an immutable canonical candidate/plan receipt binding the clean
tracked index, actual effective Docker context, deployment inputs and exact
fixture selection plus a private non-secret synthetic proxy template. Its
complete run publishes through a no-replace hard-linked active receipt; inert
pre-publication crash residue grants no provider authority and remains pending
final cleanup. Planning reaches no Docker/provider/host authority.
The new version-2 append-only receipt machine closes all intent/result schemas,
collision-safe cleanup-only failure branches and success-only manifest
eligibility. Receipt and manifest publication use no-replace artifact staging.
The append-only mutation journal keeps permanent, gap-free slots, closes and
per-slot recovery claims; each generation binds its source/result endpoints,
prior close and exact owner/recovery authority. Final journal names are never
deleted or reused. Concurrent fake writers, append versus finalize, tamper,
partial/complete/linked stages, stale names, capacity exhaustion and a
cooperatively removed live close stage are covered. That close retries only
after exact authority and endpoint revalidation. Generic append cannot own
provider-create, provider-cleanup or finalization evidence. The synchronous
closed-data fake remains rollback. A separate controlled fake path holds the
slot across a private root plan/reservation, mirrored inode/mode/UID owner,
durable actor launch/witness, one-way start, fixed-child effect/outcome and
ESRCH settlement. The supervisor reasserts the actual slot before deciding;
the actor validates the complete root inventory and digest-bound authority.
Only the actor signals its group, and directly owned children exit on parent
IPC loss. Settlement and the mutation close bind exact optional effect/outcome
evidence. Exact confirmation recovers pre-reservation, marker/mirror,
post-outcome, terminal-receipt and close crashes without replay or stored-PGID
signalling; launch-without-witness stays blocked. The contract-derived adapter
accepts no caller function, command, path, environment or provider selection,
and generic append cannot create preflight evidence. Only test fixtures expose
it; no supported lifecycle target, Docker/Colima action, cleanup settlement or
finalization is enabled. Next add the fixed Colima adapter with isolated short
Colima/Lima/cache roots, private Docker state, exact pre-intent absence and
provider-owned deletion evidence. No code may set or repurpose `HOME`; use
explicit provider paths and `--mount none`. TLS private-registry,
remote-builder canary, live proxy, browser and
exact-cleanup evidence follow; only then is the privileged mapping installed
for the first live run. Linux/reference HTTPS follows before the legacy
Rauthy/Temporal callers
can be cut over and deleted. Backup/restore, upgrade and Apalis remain open
CPR-45 slices.

## Unscheduled — not listed in the Sequencing section

- [ ] [AUTH-6: Session & token hygiene](AUTH-6.md) — open

# Feature inventory

148 features in this index. This file owns feature identity and delivered/open
state. Delivered names identify historical slices; current behavior comes from
code, generated contracts and accepted decisions. Only open features retain
implementation briefs.

114 delivered; 34 open. CI checks the counts, IDs and open-brief contract.

The current [v0.4.3 release](https://github.com/synveda/synveda/releases/tag/v0.4.3)
is available for self-hosted evaluation. [Production readiness](../PRODUCTION_READINESS.md)
tracks deployment and recovery gaps; the generated [client support matrix](../CLIENT_SUPPORT.md)
tracks tested agent versions. The release history and implementation diaries
remain in Git and the linked release evidence.

## Phase 0 — Foundation

- [x] FND-1: Workspace scaffold — delivered
- [x] FND-2: Dev environment — delivered
- [x] FND-3: synveda-types + error model — delivered
- [x] FND-4: Migrations & bitemporal base tables — delivered
- [x] FND-5: Observability baseline — delivered
- [x] FND-6: Foundational architecture decisions — delivered
- [x] FND-7: Public website and canonical brand assets — delivered

## Phase 1 — Core runtime

- [x] TEN-1: Tenant model & resolution — delivered
- [x] TEN-2: Postgres row-level security as backstop — delivered
- [x] AUTH-1: OIDC login (code+PKCE) — delivered
- [x] HIER-1: Hierarchy store — delivered
- [x] AUTHZ-1: Cedar PDP embedded — delivered
- [x] AUTH-2: JIT user provisioning from claims — delivered
- [x] AUTHZ-2: Policy packs — delivered
- [x] AUTHZ-3: Roles & role bindings — delivered
- [x] HIER-2: Scope chain resolver — delivered
- [x] HIER-3: Cedar entity sync — delivered
- [x] AUTH-3: Service identities — delivered
- [x] AUD-1: Hash-chained audit log — delivered
- [x] MEM-1: observe API + PGMQ buffer — delivered
- [x] MEM-2: Redaction & secret scanning — delivered
- [x] MEM-3: Extraction pipeline — delivered
- [x] MEM-4: Transactional embed-or-fail — delivered
- [x] CTX-1: Hybrid retrieval — delivered
- [x] CTX-2: Composition engine — delivered
- [x] CTX-3: inject API — delivered
- [x] ADPT-1: Claude Code adapter — delivered
- [x] EVAL-1: Eval harness skeleton — delivered

## Phase 2 — Governance

- [x] FLOW-1: Object store — delivered
- [x] FLOW-2: Channels — delivered
- [x] FLOW-3: Proposals & approval matrix — delivered
- [x] FLOW-4: Auto-promotion rules — delivered
- [x] FLOW-5: Cross-scope promotion — delivered
- [x] FLOW-6: CLI review flow — delivered
- [x] FLOW-7: Rollback & pinning — delivered
- [x] AUTHZ-4: Lapses (controlled relaxation) — delivered
- [x] AUTHZ-5: ABAC conditions — delivered
- [x] MEM-5: Always-on dedup & conflict detection — delivered
- [x] MEM-6: Decay, TTL & staleness — delivered
- [x] CTX-4: Tiered injection / progressive disclosure — delivered
- [x] CTX-5: recall API + MCP tool — delivered
- [x] GRPH-1: Multi-graph schema — delivered
- [x] GRPH-2: Graph-linking stage — delivered
- [x] GRPH-4: AGE performance spike / graph fallback assessment — delivered
- [x] AUD-2: Audit query & auditor role surface — delivered
- [x] EVAL-2: Extraction quality suite — delivered
- [x] EVAL-4: Retrieval & injection quality — delivered
- [x] EVAL-5: Security evals — delivered
- [x] PRMT-1: Prompt templates as assets — delivered
- [x] PRMT-2: Context packs — delivered

## Phase 3 — Deployment foundations

- [x] SKIL-1: agentskills.io-compliant model — delivered
- [x] SKIL-2: Security scanning gate — delivered
- [x] SKIL-3: Skill quality scoring — delivered
- [x] SKIL-4: Scope-targeted distribution — delivered
- [x] OPS-1: SMB profile — delivered
- [x] CNSL-1: Proposals inbox (hero screen) — delivered
- [x] ADPT-2: Generic MCP server — delivered
- [x] CNSL-2: Hierarchy & policy explorer — delivered
- [x] AUTH-4: SCIM 2.0 server — delivered
- [x] AUTH-5: Directory sync fallback — delivered
- [x] EVAL-3: Public benchmark adapters — delivered
- [x] OPS-2: Helm chart / enterprise profile — delivered
- [x] TEN-3: Dense-leg retrieval benchmark — delivered
- [x] TEN-4: Per-tenant encryption keys — delivered
- [x] OPS-8: Release & distribution — delivered
- [ ] [OPS-9: Release-shaped beta acceptance](OPS-9.md) — open
- [ ] [OPS-10: Uninstall & cleanup](OPS-10.md) — open
- [ ] [TEN-5: Tenant lifecycle](TEN-5.md) — open
- [ ] [TEN-6: Cross-tenant isolation test harness](TEN-6.md) — open
- [ ] [AUD-3: External immutable audit retention](AUD-3.md) — open
- [ ] [AUD-4: SIEM streaming](AUD-4.md) — open
- [x] GRPH-3: Graph-augmented recall — delivered
- [ ] [EVAL-6: Load & latency suite](EVAL-6.md) — open
- [ ] [CTX-7: Dense-leg plan stability](CTX-7.md) — open
- [ ] [OPS-3: Residency routing](OPS-3.md) — open
- [ ] [OPS-4: Vector index scale decision](OPS-4.md) — open
- [ ] [ADPT-3: Additional API transport decision](ADPT-3.md) — open
- [ ] [CTX-6: Session compression assist](CTX-6.md) — open
- [ ] [CTX-8: Governed context optimisation](CTX-8.md) — open

- [ ] [FLOW-8: Git bridge — export](FLOW-8.md) — open

CTX-6 now has an exact-role tested deterministic Claude checkpoint/restart path, a passing source demo, direct capture-candidate and scoped retention probes, and a passing create/read/use/capture policy matrix including revocation and foreign-tenant comparisons. Four predeclared synthetic task probes pass in the fresh-database 23-scenario product suite: 12/12 Session facts and four exact Knowledge bodies retained, provenance intact, local assisted preview p95 below the 500 ms guard. A separate combined-mode gate preserves checkpoint source attribution and exact required Knowledge in both off and conservative modes, omitting optional restart text under a tight shared budget. An opt-in export now prepares the actual paired synthetic ContextRun blocks against a fixed JSON-answer rubric without calling a model, and a capped no-tools runner passes a fake-client end-to-end test. This does not measure model task success or provider usage. A live proprietary-client run remains open. CTX-8's conservative path is implemented and its optional learned backend has a no-go promotion decision. An isolated hostname handoff and resolver check passed without resetting retained interop state, and the original mapping was restored. The product source image now selects its verified pinned Rust compiler without fetching development-only components, but Cargo index/archive endpoints time out from this host and Docker, with a fresh direct host HTTPS timeout on 2026-09-25, so full Compose/browser acceptance and CTX-8 held-out model quality evidence remain open. Next restore Cargo source-build egress within the documented trust boundary, rerun paired deployment checks, then run the bounded factual/model and live Claude probes when credentials and spend permission are available. The open briefs hold the exact evidence and limits.

## Phase 4 — Clients and operations

- [ ] [ADPT-4: Python & TS SDKs](ADPT-4.md) — open
- [ ] [ADPT-5: Source-format converters](ADPT-5.md) — open
- [ ] [ADPT-6: LlamaIndex memory adapter](ADPT-6.md) — open
- [ ] [ADPT-7: Semantic Kernel memory connector](ADPT-7.md) — open
- [x] ADPT-8: Observation that survives a session that does not wait — delivered
- [x] ADPT-9: GitHub Copilot CLI adapter — delivered
- [ ] [PRMT-3: Prompt experiment evidence](PRMT-3.md) — open
- [ ] [SKIL-5: Authentic Skill usage reporting](SKIL-5.md) — open
- [ ] [MEM-7: Identity stitching](MEM-7.md) — open
- [ ] [OPS-5: Backup/restore & DR](OPS-5.md) — open
- [ ] [OPS-6: Upgrade and rollback discipline](OPS-6.md) — open
- [ ] [OPS-7: Gateway horizontal scale](OPS-7.md) — open
- [ ] [OPS-11: Small-team Kubernetes release](OPS-11.md) — open
- [ ] [OPS-12: Consumer installation and harness setup](OPS-12.md) — open
- [ ] [CNSL-3: Audit temporal and disclosure views](CNSL-3.md) — open
- [x] CNSL-4: Knowledge browser — delivered
- [x] CNSL-5: Console theme and everyday usability — delivered
- [ ] [AUD-5: Compliance mapping doc](AUD-5.md) — open
- [ ] [AUTHZ-6: Authorisation scale decision](AUTHZ-6.md) — open
- [ ] [AUTHZ-7: Governed admin-plane mutation](AUTHZ-7.md) — open
- [ ] [TEN-7: Tenant storage partition decision](TEN-7.md) — open
- [ ] [EVAL-7: A second public benchmark](EVAL-7.md) — open

## Phase 5 — Context platform

- [x] CPR-1: Implementation baseline & locked decisions — delivered
- [x] CPR-2: Fresh schema epoch, startup guard & local reset — delivered
- [x] CPR-3: Generic governed scope substrate — delivered
- [x] CPR-4: Workspaces, projects & canonical repository identity — delivered
- [x] CPR-5: Membership, groups, grants & invitations — delivered
- [x] CPR-6: Governed scope anchors — the PDP re-cut — delivered
- [x] CPR-7: The hierarchy cutover — one scope tree — delivered
- [x] CPR-8: The console product shell & first-run onboarding — delivered
- [x] CPR-9: The foundation audit — hardening the scope and access cutover — delivered
- [x] CPR-10: The session ledger and runtime API — delivered
- [x] CPR-11: The session product experience — delivered
- [x] CPR-12: Durable Claude session delivery — delivered
- [x] CPR-13: The demo corpus re-point — delivered
- [x] CPR-14: Live Claude Code session acceptance gate — delivered
- [x] CPR-15: Versioned Knowledge aggregate and provenance — delivered
- [x] CPR-16: Governed Knowledge mutation lifecycle — delivered
- [x] CPR-17: Public Knowledge API, search and browser — delivered
- [x] CPR-18: Session-based capture batches and reviewable candidates — delivered
- [x] CPR-19: New Learnings lightweight review workflow — delivered
- [x] CPR-20: Explainable Knowledge context planning and scoped query — delivered
- [x] CPR-21: Context Inspector and outcome feedback — delivered
- [x] CPR-22: Core individual and small-team MVP acceptance — delivered
- [x] CPR-23: Immutable skill versions, bindings and usage — delivered
- [x] CPR-24: Skills Library product experience — delivered
- [x] CPR-25: Trusted MCP server catalogue and project bindings — delivered
- [x] CPR-26: MCP Tools catalogue product experience — delivered
- [x] CPR-27: OKF v0.2 knowledge exchange adapter — delivered
- [x] CPR-28: OKF import and export product workflows — delivered
- [x] CPR-29: Public contract and client convergence — delivered
- [x] CPR-30: Governed runtime configuration artifacts — delivered
- [x] CPR-31: Governed auto-apply and policy relaxations — delivered
- [x] CPR-32: Unified approvals across governed artifacts — delivered
- [x] CPR-33: Context-platform audit query and deterministic export — delivered
- [x] CPR-34: Directory adapter convergence — delivered
- [x] CPR-35: Context-platform key and secret convergence — delivered
- [x] CPR-36: One-runtime deployment convergence — delivered
- [x] CPR-37: Conflict, supersession and freshness engine — delivered
- [x] CPR-38: Bounded graph-augmented retrieval — delivered
- [x] CPR-39: Second verified client — delivered
- [x] CPR-40: Context-platform product and trust evaluation — delivered
- [x] CPR-41: One-command realistic product demo — delivered
- [x] CPR-42: Context-platform security and product-integrity audit — delivered
- [x] CPR-43: Final context-platform hard cut — delivered
- [x] CPR-44: Production hardening and maintainability cut — delivered
- [ ] [CPR-45: Docker-first portable reference deployment](CPR-45.md) — open

## Unscheduled — not listed in the Sequencing section

- [ ] [AUTH-6: Session & token hygiene](AUTH-6.md) — open

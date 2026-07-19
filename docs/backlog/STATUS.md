# Backlog status

86 features parsed from docs/SYNVEDA_FEATURES.md — one file per
feature in this directory. Phases per the Sequencing section. Regenerate with
`node scripts/generate-backlog.mjs` (preserves done-marks listed in the script).

Phase 1+ must not start until FND is complete and `make dev-up && make smoke`
passes (CLAUDE.md, current phase).

## Phase 0 — Foundation (wk 1)

- [x] [FND-1: Workspace scaffold](FND-1.md) — done 2026-07-16, demo: demos/fnd-1-scaffold.sh
- [x] [FND-2: Dev environment](FND-2.md) — done 2026-07-17, demo: demos/fnd-2-dev-env.sh
- [x] [FND-3: synveda-types + error model](FND-3.md) — done 2026-07-18, AC test: crates/synveda-types/tests/serde_roundtrip.rs
- [x] [FND-4: Migrations & bitemporal base tables](FND-4.md) — done 2026-07-18, AC test: crates/synveda-store/tests/bitemporal.rs, demo: demos/fnd-4-bitemporal.sh
- [x] [FND-5: Observability baseline](FND-5.md) — done 2026-07-18, AC test: crates/synveda-gateway/tests/observability.rs, demo: demos/fnd-5-observability.sh
- [x] [FND-6: ADRs 0001–0004](FND-6.md) — done 2026-07-18, demo: demos/fnd-6-adrs.sh (adr-0001..0004 in docs/adr/)

_Phase 0 complete: exit gate `make dev-up && make smoke` passed 2026-07-18
(all services healthy incl. AGE/PGMQ/pgvector, Rauthy, Temporal, TEI BGE-M3,
Jaeger). Phase 1 may start._

## Phase 1 — The spine (wk 2–5)

_Phase demo goal: SSO login → auto-scoped → live Claude Code session writes and receives governed memory, fully audited._

- [x] [TEN-1: Tenant model & resolution](TEN-1.md) — done 2026-07-18, AC test: crates/synveda-gateway/tests/tenant_resolution.rs, demo: demos/ten-1-tenant-resolution.sh
- [x] [TEN-2: Postgres row-level security as backstop](TEN-2.md) — done 2026-07-18, AC test: crates/synveda-store/tests/rls.rs, demo: demos/ten-2-rls.sh
- [x] [AUTH-1: OIDC login (code+PKCE)](AUTH-1.md) — done 2026-07-18, AC test: crates/synveda-gateway/tests/oidc_login.rs (mock Entra), demo: demos/auth-1-oidc-login.sh (live Rauthy)
- [x] [HIER-1: Hierarchy store](HIER-1.md) — done 2026-07-18, AC test: crates/synveda-store/tests/hierarchy.rs (10k nodes; ancestors/descendants medians 57µs/691µs over baseline), demo: demos/hier-1-hierarchy.sh
- [x] [AUTHZ-1: Cedar PDP embedded](AUTHZ-1.md) — done 2026-07-18, AC tests: crates/synveda-policy/tests/decision_benchmark.rs (facade incl. entity materialisation, 4-level chain: median 109µs, p99 177µs), crates/synveda-policy/tests/pdp.rs (decision + pack version on every call), crates/synveda-gateway/tests/authz_hierarchy.rs (route gate + hot reload), demo: demos/authz-1-cedar-pdp.sh
- [x] [AUTH-2: JIT user provisioning from claims](AUTH-2.md) — done 2026-07-18, AC test: crates/synveda-gateway/tests/jit_provisioning.rs (mock IdP: team mapping, quarantine + PDP denial, override precedence, fail-closed bearer), demo: demos/auth-2-jit-provisioning.sh (live Rauthy)
- [x] [AUTHZ-2: Policy packs](AUTHZ-2.md) — done 2026-07-19, AC tests: crates/synveda-policy/tests/packs.rs (golden matrix per pack; composition switch at the MemoryRead seam), crates/synveda-gateway/tests/policy_routes.rs (per-node assignment governs the next request; inheritance, origin display, self-rescue), demo: demos/authz-2-policy-packs.sh
- [x] [AUTHZ-3: Roles & role bindings](AUTHZ-3.md) — done 2026-07-19, AC tests: crates/synveda-policy/tests/roles.rs (full role×action matrix per pack; escalation guard; subtree boundaries; privacy floor), crates/synveda-gateway/tests/roles_routes.rs (bindings govern the next request; delegation; uniform 404), crates/synveda-gateway/tests/jit_provisioning.rs (admin-group bootstrap), demo: demos/authz-3-roles.sh
- [ ] [HIER-2: Scope chain resolver](HIER-2.md)
- [ ] [HIER-3: Cedar entity sync](HIER-3.md)
- [ ] [AUTH-3: Service identities](AUTH-3.md)
- [ ] [AUD-1: Hash-chained audit log](AUD-1.md)
- [ ] [MEM-1: observe API + PGMQ buffer](MEM-1.md)
- [ ] [MEM-2: Redaction & secret scanning](MEM-2.md)
- [ ] [MEM-3: Extraction pipeline](MEM-3.md)
- [ ] [MEM-4: Transactional embed-or-fail](MEM-4.md)
- [ ] [CTX-1: Hybrid retrieval](CTX-1.md)
- [ ] [CTX-2: Composition engine](CTX-2.md)
- [ ] [CTX-3: inject API](CTX-3.md)
- [ ] [ADPT-1: Claude Code adapter](ADPT-1.md)
- [ ] [EVAL-1: Eval harness skeleton](EVAL-1.md)

_Order revised 2026-07-18 (was TEN → AUTH → AUTHZ → HIER → MEM → CTX →
AUD): the epic-grouped sequence was not a valid topological order. HIER-1
now precedes AUTHZ-1 and AUTH-2 (Cedar entities and JIT provisioning need
hierarchy nodes to exist); AUTH-3 follows AUTHZ-1 (its scope-enforcement AC
is a PDP decision); AUD-1 moves ahead of MEM-1 so the data path is born
audited and the ADR-0008/0009 emission-point retrofit stays bounded to the
identity features._

_TEN-1 deferral (ADR-0008): tenant-resolution decisions are an audit
emission point; events are wired when AUD-1's hash-chained log lands. Until
then they are visible in traces and `synveda_tenant_resolutions_total` only._

_TEN-2 deferrals (ADR-0009): RLS-backstop trips (SQLSTATE 42501 →
`Error::Internal`) are an AUD-1 emission point; data-path features must
reach tenant-scoped tables via `synveda_store::rls::begin_tenant_tx`, and
deployment profiles (OPS-1/OPS-2) must connect as a non-superuser
`synveda_app` login — the dev compose superuser bypasses RLS._

_HIER-1 deferrals (ADR-0011): hierarchy CRUD (create/rename/move/delete)
is an audit emission point, wired when AUD-1 lands — until then visible in
traces and `synveda_hierarchy_operations_total`. The `/v1/hierarchy/*`
admin routes' PDP gate — AUTHZ-1's first obligation — was discharged
2026-07-18: every handler authorizes through the Cedar facade
(ADR-0012 decision 7)._

_AUTHZ-1 deferrals (ADR-0012): every PDP decision is an AUD-1 emission
point — until the hash-chained log lands, decisions are visible in the
decision log (pack name@version + determining policies, every call) and
`synveda_authz_decisions_total`. The `bootstrap` pack was retired
2026-07-19: AUTHZ-2 replaced it with the embedded product packs
(`regulated-strict` is the zero-config default; roles still arrive with
AUTHZ-3). Stored-pack propagation lags up to
`SYNVEDA_POLICY_REFRESH_SECS` (default 5s, poll-based) until VedaFlow
policy commits drive event-based reload._

_AUTH-2 deferrals (ADR-0013): identity provisioning
(`identity.provisioned`) is an AUD-1 emission point — until then visible
in the `identity.provision` span and `synveda_jit_provisions_total`.
Group-mapping overrides are store-managed until an admin surface
exists; placement is first-login-final — movers/leavers arrive with
AUTH-4/5, and release from quarantine is the existing PDP-gated
hierarchy move. The quarantine forbid now lives in the base layer
compiled into every pack (ADR-0014 decision 2); an IdP subject that
never completed a login is quarantined at the PDP seam (fail closed).
Dev HS256 subjects kept tenant-wide admin semantics until AUTHZ-3
landed roles (2026-07-19): an unbound subject now holds no
administrative power._

_AUTHZ-2 deferrals (ADR-0014): pack assignment/default mutations are
AUD-1 emission points — until then visible in traces and
`synveda_policy_operations_total`. `MemoryRead` is the composition
seam; the AC's "inject composition changes next session" is
demonstrated at that seam and re-demonstrated end-to-end when
CTX-1/2/3 land on it. Governed handlers read the placement chain and
chain assignments per request until HIER-2/3 cache them. Who may
assign was tenant-wide until AUTHZ-3 narrowed it to steward/org-admin
(2026-07-19); `standard`'s department
sharing collapses to strict where the hierarchy skips the department
level; `open-collaboration`'s "non-restricted content" qualifier is
AUTHZ-5 classification — the personal-scope exclusion is the current
privacy floor. A tenant default naming a custom pack that omits
`PolicyAssign` locks the tenant's policy plane; the store-level CLI is
the break-glass (node-level assignments cannot seal themselves —
ADR-0014 decision 4)._

_AUTHZ-3 deferrals (ADR-0015): binding mutations and the JIT
admin-group binding are AUD-1 emission points — until then visible in
traces and `synveda_role_operations_total`. Group-driven bindings are
additive-only (`synveda-admins` upserts tenant-wide org-admin at every
login; an admin-group subject with no team mapping is placed under the
org root, never quarantine); revocation stays explicit until AUTH-4/5
bring mover/leaver sync, and richer group→role mapping rules defer with
them. `synveda role bind` is the bootstrap and the break-glass — a
tenant that revokes its last org-admin recovers there. The embedded
packs bumped to `@2`; governed requests now also read the subject's
bindings for the resource chain per request until HIER-2/3 cache them.
Roles whose actions land later are marker rows in the golden matrix
until those features extend it: curator approvals (FLOW-3),
security-reviewer (SKIL-2), compliance (AUTHZ-5/FLOW-3), auditor's
audit surface (AUD-2), contributor writes (MEM-1)._

## Phase 2 — Governance (wk 6–10)

_Phase demo goal: promotion pipeline, lapse lifecycle, as-of inject, bank-mode switch._

- [ ] [FLOW-1: Object store](FLOW-1.md)
- [ ] [FLOW-2: Channels](FLOW-2.md)
- [ ] [FLOW-3: Proposals & approval matrix](FLOW-3.md)
- [ ] [FLOW-4: Auto-promotion rules](FLOW-4.md)
- [ ] [FLOW-5: Cross-scope promotion](FLOW-5.md)
- [ ] [FLOW-6: CLI review flow](FLOW-6.md)
- [ ] [FLOW-7: Rollback & pinning](FLOW-7.md)
- [ ] [AUTHZ-4: Lapses (controlled relaxation)](AUTHZ-4.md)
- [ ] [AUTHZ-5: ABAC conditions](AUTHZ-5.md)
- [ ] [MEM-5: Always-on dedup & conflict detection](MEM-5.md)
- [ ] [MEM-6: Decay, TTL & staleness](MEM-6.md)
- [ ] [CTX-4: Tiered injection / progressive disclosure](CTX-4.md)
- [ ] [CTX-5: recall API + MCP tool](CTX-5.md)
- [ ] [GRPH-1: Multi-graph AGE schema](GRPH-1.md)
- [ ] [GRPH-2: Graph-linking stage](GRPH-2.md)
- [ ] [GRPH-4: AGE performance spike / Kuzu fallback assessment](GRPH-4.md)
- [ ] [AUD-2: Audit query & auditor role surface](AUD-2.md)
- [ ] [EVAL-2: Extraction quality suite](EVAL-2.md)
- [ ] [EVAL-4: Retrieval & injection quality](EVAL-4.md)
- [ ] [EVAL-5: Security evals](EVAL-5.md)
- [ ] [PRMT-1: Prompt templates as assets](PRMT-1.md)
- [ ] [PRMT-2: Context packs](PRMT-2.md)

## Phase 3 — Enterprise (wk 11–16)

_Phase demo goal: Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor, LoCoMo/LongMemEval scores published, Helm install._

- [ ] [AUTH-4: SCIM 2.0 server](AUTH-4.md)
- [ ] [AUTH-5: Directory sync fallback](AUTH-5.md)
- [ ] [TEN-3: Tenant-partitioned storage layout](TEN-3.md)
- [ ] [TEN-4: Per-tenant encryption keys](TEN-4.md)
- [ ] [TEN-5: Tenant lifecycle](TEN-5.md)
- [ ] [TEN-6: Cross-tenant isolation test harness](TEN-6.md)
- [ ] [SKIL-1: agentskills.io-compliant model](SKIL-1.md)
- [ ] [SKIL-2: Security scanning gate](SKIL-2.md)
- [ ] [SKIL-3: Skill quality scoring](SKIL-3.md)
- [ ] [SKIL-4: Scope-targeted distribution](SKIL-4.md)
- [ ] [GRPH-3: Graph-augmented recall](GRPH-3.md)
- [ ] [AUD-3: WORM export](AUD-3.md)
- [ ] [AUD-4: SIEM streaming](AUD-4.md)
- [ ] [EVAL-3: Public benchmark adapters](EVAL-3.md)
- [ ] [EVAL-6: Load & latency suite](EVAL-6.md)
- [ ] [OPS-1: SMB profile](OPS-1.md)
- [ ] [OPS-2: Helm chart / enterprise profile](OPS-2.md)
- [ ] [OPS-3: Residency routing](OPS-3.md)
- [ ] [OPS-4: Qdrant adapter behind VectorIndex trait](OPS-4.md)
- [ ] [CNSL-1: Proposals inbox (hero screen)](CNSL-1.md)
- [ ] [CNSL-2: Hierarchy & policy explorer](CNSL-2.md)
- [ ] [ADPT-2: Generic MCP server](ADPT-2.md)
- [ ] [ADPT-3: REST/gRPC API + OpenAPI](ADPT-3.md)
- [ ] [CTX-6: Session compression assist](CTX-6.md)
- [ ] [FLOW-8: Git bridge — export](FLOW-8.md)

## Phase 4 — Ecosystem

- [ ] [ADPT-4: Python & TS SDKs](ADPT-4.md)
- [ ] [ADPT-5: Importers](ADPT-5.md)
- [ ] [PRMT-3: A/B channels for prompts](PRMT-3.md)
- [ ] [SKIL-5: Skill usage telemetry](SKIL-5.md)
- [ ] [MEM-7: Identity stitching](MEM-7.md)
- [ ] [OPS-5: Backup/restore & DR](OPS-5.md)
- [ ] [OPS-6: Zero-downtime migration discipline](OPS-6.md)
- [ ] [CNSL-3: Audit explorer](CNSL-3.md)
- [ ] [CNSL-4: Memory browser](CNSL-4.md)
- [ ] [AUD-5: Compliance mapping doc](AUD-5.md)
- [ ] [AUTHZ-6: OpenFGA adapter spike](AUTHZ-6.md)

## Unscheduled — not listed in the Sequencing section

- [ ] [AUTH-6: Session & token hygiene](AUTH-6.md)

# Backlog status

86 features parsed from docs/SYNVEDA_FEATURES.md — one file per
feature in this directory. Phases per the Sequencing section. Regenerate with
`node scripts/generate-backlog.mjs` (preserves done-marks listed in the script).

Phase 1+ must not start until FND is complete and `make dev-up && make smoke`
passes (CLAUDE.md, current phase).

## Phase 0 — Foundation (wk 1)

- [x] [FND-1: Workspace scaffold](FND-1.md) — done 2026-07-16, demo: demos/fnd-1-scaffold.sh
- [ ] [FND-2: Dev environment](FND-2.md)
- [ ] [FND-3: synveda-types + error model](FND-3.md)
- [ ] [FND-4: Migrations & bitemporal base tables](FND-4.md)
- [ ] [FND-5: Observability baseline](FND-5.md)
- [ ] [FND-6: ADRs 0001–0004](FND-6.md)

## Phase 1 — The spine (wk 2–5)

_Phase demo goal: SSO login → auto-scoped → live Claude Code session writes and receives governed memory, fully audited._

- [ ] [TEN-1: Tenant model & resolution](TEN-1.md)
- [ ] [TEN-2: Postgres row-level security as backstop](TEN-2.md)
- [ ] [AUTH-1: OIDC login (code+PKCE)](AUTH-1.md)
- [ ] [AUTH-2: JIT user provisioning from claims](AUTH-2.md)
- [ ] [AUTH-3: Service identities](AUTH-3.md)
- [ ] [AUTHZ-1: Cedar PDP embedded](AUTHZ-1.md)
- [ ] [AUTHZ-2: Policy packs](AUTHZ-2.md)
- [ ] [AUTHZ-3: Roles & role bindings](AUTHZ-3.md)
- [ ] [HIER-1: Hierarchy store](HIER-1.md)
- [ ] [HIER-2: Scope chain resolver](HIER-2.md)
- [ ] [HIER-3: Cedar entity sync](HIER-3.md)
- [ ] [MEM-1: observe API + PGMQ buffer](MEM-1.md)
- [ ] [MEM-2: Redaction & secret scanning](MEM-2.md)
- [ ] [MEM-3: Extraction pipeline](MEM-3.md)
- [ ] [MEM-4: Transactional embed-or-fail](MEM-4.md)
- [ ] [CTX-1: Hybrid retrieval](CTX-1.md)
- [ ] [CTX-2: Composition engine](CTX-2.md)
- [ ] [CTX-3: inject API](CTX-3.md)
- [ ] [AUD-1: Hash-chained audit log](AUD-1.md)
- [ ] [ADPT-1: Claude Code adapter](ADPT-1.md)
- [ ] [EVAL-1: Eval harness skeleton](EVAL-1.md)

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

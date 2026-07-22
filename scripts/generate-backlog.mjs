#!/usr/bin/env node
// Generates docs/backlog/<ID>.md (one file per feature) and docs/backlog/STATUS.md
// from Part B of docs/SYNVEDA_FEATURES.md.
//
// Phases are transcribed from the "Sequencing (features → phases)" section below;
// the script fails if the transcription and the parsed features drift apart, and
// labels any feature absent from Sequencing as phase:unscheduled.
//
// Usage: node scripts/generate-backlog.mjs

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const SRC = "docs/SYNVEDA_FEATURES.md";
const OUT = "docs/backlog";

// ── Transcribed from the Sequencing section ─────────────────────────────────
const PHASES = [
  {
    n: 0,
    title: "Phase 0 — Foundation (wk 1)",
    demo: null,
    ids: ["FND-1", "FND-2", "FND-3", "FND-4", "FND-5", "FND-6"],
  },
  {
    n: 1,
    title: "Phase 1 — The spine (wk 2–5)",
    demo:
      "SSO login → auto-scoped → live Claude Code session writes and receives governed memory, fully audited.",
    ids: [
      "TEN-1", "TEN-2",
      "AUTH-1", "HIER-1", "AUTHZ-1", "AUTH-2",
      "AUTHZ-2", "AUTHZ-3",
      "HIER-2", "HIER-3", "AUTH-3",
      "AUD-1",
      "MEM-1", "MEM-2", "MEM-3", "MEM-4",
      "CTX-1", "CTX-2", "CTX-3",
      "ADPT-1", "EVAL-1",
    ],
  },
  {
    n: 2,
    title: "Phase 2 — Governance (wk 6–10)",
    demo: "promotion pipeline, lapse lifecycle, as-of inject, bank-mode switch.",
    ids: [
      "FLOW-1", "FLOW-2", "FLOW-3", "FLOW-4", "FLOW-5", "FLOW-6", "FLOW-7",
      "AUTHZ-4", "AUTHZ-5",
      "MEM-5", "MEM-6",
      "CTX-4", "CTX-5",
      "GRPH-1", "GRPH-2", "GRPH-4",
      "AUD-2",
      "EVAL-2", "EVAL-4", "EVAL-5",
      "PRMT-1", "PRMT-2",
    ],
  },
  {
    n: 3,
    title: "Phase 3 — Enterprise (wk 11–16)",
    demo:
      "Entra/Okta live, spec-compliant governed skills into Claude Code + Cursor, LoCoMo/LongMemEval scores published, Helm install.",
    ids: [
      "AUTH-4", "AUTH-5",
      "TEN-3", "TEN-4", "TEN-5", "TEN-6",
      "SKIL-1", "SKIL-2", "SKIL-3", "SKIL-4",
      "GRPH-3",
      "AUD-3", "AUD-4",
      "EVAL-3", "EVAL-6",
      "OPS-1", "OPS-2", "OPS-3", "OPS-4",
      "CNSL-1", "CNSL-2",
      "ADPT-2", "ADPT-3",
      "CTX-6", "FLOW-8",
    ],
  },
  {
    n: 4,
    title: "Phase 4 — Ecosystem",
    demo: null,
    ids: [
      "ADPT-4", "ADPT-5",
      "PRMT-3", "SKIL-5", "MEM-7",
      "OPS-5", "OPS-6",
      "CNSL-3", "CNSL-4",
      "AUD-5", "AUTHZ-6",
    ],
  },
];

// Features already delivered (kept checked in STATUS.md across regenerations).
const DONE = new Map([
  ["FND-1", "done 2026-07-16, demo: demos/fnd-1-scaffold.sh"],
  ["FND-2", "done 2026-07-17, demo: demos/fnd-2-dev-env.sh"],
  ["FND-3", "done 2026-07-18, AC test: crates/synveda-types/tests/serde_roundtrip.rs"],
  ["FND-4", "done 2026-07-18, AC test: crates/synveda-store/tests/bitemporal.rs, demo: demos/fnd-4-bitemporal.sh"],
  ["FND-5", "done 2026-07-18, AC test: crates/synveda-gateway/tests/observability.rs, demo: demos/fnd-5-observability.sh"],
  ["FND-6", "done 2026-07-18, demo: demos/fnd-6-adrs.sh (adr-0001..0004 in docs/adr/)"],
  ["TEN-1", "done 2026-07-18, AC test: crates/synveda-gateway/tests/tenant_resolution.rs, demo: demos/ten-1-tenant-resolution.sh"],
  ["TEN-2", "done 2026-07-18, AC test: crates/synveda-store/tests/rls.rs, demo: demos/ten-2-rls.sh"],
  ["AUTH-1", "done 2026-07-18, AC test: crates/synveda-gateway/tests/oidc_login.rs (mock Entra), demo: demos/auth-1-oidc-login.sh (live Rauthy)"],
  ["HIER-1", "done 2026-07-18, AC test: crates/synveda-store/tests/hierarchy.rs (10k nodes; ancestors/descendants medians 57µs/691µs over baseline), demo: demos/hier-1-hierarchy.sh"],
  ["AUTHZ-1", "done 2026-07-18, AC tests: crates/synveda-policy/tests/decision_benchmark.rs (facade incl. entity materialisation, 4-level chain: median 109µs, p99 177µs), crates/synveda-policy/tests/pdp.rs (decision + pack version on every call), crates/synveda-gateway/tests/authz_hierarchy.rs (route gate + hot reload), demo: demos/authz-1-cedar-pdp.sh"],
  ["AUTH-2", "done 2026-07-18, AC test: crates/synveda-gateway/tests/jit_provisioning.rs (mock IdP: team mapping, quarantine + PDP denial, override precedence, fail-closed bearer), demo: demos/auth-2-jit-provisioning.sh (live Rauthy)"],
  ["AUTHZ-2", "done 2026-07-19, AC tests: crates/synveda-policy/tests/packs.rs (golden matrix per pack; composition switch at the MemoryRead seam), crates/synveda-gateway/tests/policy_routes.rs (per-node assignment governs the next request; inheritance, origin display, self-rescue), demo: demos/authz-2-policy-packs.sh"],
  ["AUTHZ-3", "done 2026-07-19, AC tests: crates/synveda-policy/tests/roles.rs (full role×action matrix per pack; escalation guard; subtree boundaries; privacy floor), crates/synveda-gateway/tests/roles_routes.rs (bindings govern the next request; delegation; uniform 404), crates/synveda-gateway/tests/jit_provisioning.rs (admin-group bootstrap), demo: demos/authz-3-roles.sh"],
  ["HIER-2", "done 2026-07-19, AC tests: crates/synveda-store/tests/scope_chain.rs (invalidation serves the fresh chain after a move; warm resolve median 800ns, p99 ≤1.5µs over 10k samples — 300× under the 0.5ms bound), crates/synveda-gateway/tests/scope_chain_routes.rs (a move governs the very next request through the cache), demo: demos/hier-2-scope-chain.sh"],
  ["HIER-3", "done 2026-07-19, AC tests: crates/synveda-gateway/tests/cedar_entity_sync.rs (a team moved between departments governs the very next decision: the moving steward's authority leaves with it over HTTP; the department MemoryRead follows it at the composition seam), crates/synveda-policy/tests/entity_sync.rs (a warm fragment never survives a reshaped chain, both directions), demo: demos/hier-3-cedar-entity-sync.sh"],
  ["AUTH-3", "done 2026-07-19, AC tests: crates/synveda-gateway/tests/service_identities.rs (client-credentials grant end to end against a mock IdP; a team-anchored agent holding tenant-wide org-admin is denied every org-scope endpoint; unregistered clients quarantined; lifetime cap; PDP-gated registration; next-request revocation), crates/synveda-policy/tests/service_scope.rs (the base-layer confinement forbid across the action vocabulary; the own-chain MemoryRead floor survives; roles cannot widen past the token scope), demo: demos/auth-3-service-identities.sh (live Rauthy)"],
  ["AUD-1", "done 2026-07-19, AC test: crates/synveda-audit/tests/tamper.rs (a database-credentialed attacker suppresses triggers and rewrites history: every hashed column, row removal, relinking, and head attacks all break verification at the named seq), emission tests: crates/synveda-gateway/tests/audit_events.rs (mutation/read/denial/suspended-tenant/token-rejection each chain one event and the chain verifies), crates/synveda-store/tests/rls.rs (audit tables join the adversarial RLS suite), demo: demos/aud-1-audit-log.sh"],
  ["MEM-1", "done 2026-07-19, AC tests: crates/synveda-gateway/tests/observe.rs (duplicate delivery admits nothing twice — response, staging table, queue, and audit chain all agree; 1k events/s sustained with the ack median inside the 20ms-plus-link-tax budget), crates/synveda-store/tests/rls.rs (observe buffer joins the adversarial RLS suite; PGMQ grants proven under synveda_app), crates/synveda-policy/tests/{packs,roles,service_scope}.rs (the MemoryWrite floor and grant golden-tested), demo: demos/mem-1-observe.sh"],
  ["MEM-2", "done 2026-07-19, AC tests: crates/synveda-gateway/tests/observe_redaction.rs (seeded secrets swept for across staging, quarantine, audit, and both PGMQ tables under all three modes — zero hits; the review queue E2E: security-reviewer's first live action, owner denied self-release, release sends the standard signal, one-shot 409), crates/synveda-ingest/tests/redaction.rs (every rule + validators + the scanner-output-never-contains-matched-text discipline), crates/synveda-store/tests/rls.rs (observe_quarantine joins the adversarial suite; column-bound one-shot review), crates/synveda-policy/tests/{packs,roles}.rs (quarantine plane golden-tested; redaction config rides the effective pack), demo: demos/mem-2-redaction.sh"],
  ["MEM-3", "done 2026-07-22, AC tests: crates/synveda-ingest/tests/extraction_precision.rs (labelled fixture set, per-class precision; deterministic macro 0.958 ≥ the provisional 0.8 target; the `#[ignore]`d live-LLM hook runs the same harness against Claude or vLLM), crates/synveda-gateway/tests/extraction.rs (observe → worker → records with the provenance quadruple on every record; archive-lock exactly-once under redelivery; a released quarantined event extracts identically; a since-quarantined owner is denied at commit; retries exhaust into an audited dead-letter; an extractor echoing a live-format secret persists only the placeholder), crates/synveda-ingest/tests/extractor_http.rs (Claude/vLLM request contracts against local mocks), crates/synveda-store/tests/observe_queue.rs (visibility timeout, redelivery, archive-as-lock), demo: demos/mem-3-extraction.sh"],
]);

// Phase-level notes appended after a phase's checklist (kept across
// regenerations, like DONE).
const PHASE_NOTES = new Map([
  [
    0,
    "_Phase 0 complete: exit gate `make dev-up && make smoke` passed 2026-07-18\n" +
      "(all services healthy incl. AGE/PGMQ/pgvector, Rauthy, Temporal, TEI BGE-M3,\n" +
      "Jaeger). Phase 1 may start._",
  ],
  [
    1,
    "_Order revised 2026-07-18 (was TEN → AUTH → AUTHZ → HIER → MEM → CTX →\n" +
      "AUD): the epic-grouped sequence was not a valid topological order. HIER-1\n" +
      "now precedes AUTHZ-1 and AUTH-2 (Cedar entities and JIT provisioning need\n" +
      "hierarchy nodes to exist); AUTH-3 follows AUTHZ-1 (its scope-enforcement AC\n" +
      "is a PDP decision); AUD-1 moves ahead of MEM-1 so the data path is born\n" +
      "audited and the ADR-0008/0009 emission-point retrofit stays bounded to the\n" +
      "identity features._\n" +
      "\n" +
      "_TEN-1 deferral (ADR-0008): closed 2026-07-19 — AUD-1 chains\n" +
      "`tenant.resolution.denied` when a verified token names a suspended tenant\n" +
      "(ADR-0019 decision 6). Successful resolutions stay implicit (every\n" +
      "subsequent chained event proves one); unauthenticated failures carry no\n" +
      "attributable subject and remain in traces and\n" +
      "`synveda_tenant_resolutions_total`._\n" +
      "\n" +
      "_TEN-2 deferrals (ADR-0009): the audit half closed 2026-07-19 — backstop\n" +
      "trips (SQLSTATE 42501, now marked via `rls::backstop_error` and classified\n" +
      "by `rls::is_backstop_trip`) chain as `store.rls.denied` at the gateway's\n" +
      "respond seam (AUD-1, ADR-0019 decision 5). Still standing: data-path\n" +
      "features must reach tenant-scoped tables via\n" +
      "`synveda_store::rls::begin_tenant_tx`, and deployment profiles\n" +
      "(OPS-1/OPS-2) must connect as a non-superuser `synveda_app` login — the\n" +
      "dev compose superuser bypasses RLS._\n" +
      "\n" +
      "_HIER-1 deferrals (ADR-0011): the audit half closed 2026-07-19 — every\n" +
      "hierarchy mutation chains `hierarchy.node.{created,updated,deleted}` with\n" +
      "pre/post images in the mutation's own transaction (AUD-1, ADR-0019). The\n" +
      "`/v1/hierarchy/*` admin routes' PDP gate — AUTHZ-1's first obligation —\n" +
      "was discharged 2026-07-18: every handler authorizes through the Cedar\n" +
      "facade (ADR-0012 decision 7)._\n" +
      "\n" +
      "_AUTHZ-1 deferrals (ADR-0012): the audit half closed 2026-07-19 with\n" +
      "ADR-0019 decision 4's shape — one chained event per audited operation:\n" +
      "mutations embed their decision context (pack@version, determining\n" +
      "policies, roles), denials and allowed admin-plane reads chain standalone\n" +
      "`authz.decision` events. The per-call decision log and\n" +
      "`synveda_authz_decisions_total` continue unchanged and remain the\n" +
      "full-fidelity record of every individual PDP call. The `bootstrap` pack was retired\n" +
      "2026-07-19: AUTHZ-2 replaced it with the embedded product packs\n" +
      "(`regulated-strict` is the zero-config default; roles still arrive with\n" +
      "AUTHZ-3). Stored-pack propagation lags up to\n" +
      "`SYNVEDA_POLICY_REFRESH_SECS` (default 5s, poll-based) until VedaFlow\n" +
      "policy commits drive event-based reload._\n" +
      "\n" +
      "_AUTH-2 deferrals (ADR-0013): the audit half closed 2026-07-19 —\n" +
      "`identity.provisioned` chains in the provisioning transaction whenever an\n" +
      "identity row is created (mapped/admin/quarantined placements; `existing`\n" +
      "logins chain nothing — ADR-0019 decision 6).\n" +
      "Group-mapping overrides are store-managed until an admin surface\n" +
      "exists; placement is first-login-final — movers/leavers arrive with\n" +
      "AUTH-4/5, and release from quarantine is the existing PDP-gated\n" +
      "hierarchy move. The quarantine forbid now lives in the base layer\n" +
      "compiled into every pack (ADR-0014 decision 2); an IdP subject that\n" +
      "never completed a login is quarantined at the PDP seam (fail closed).\n" +
      "Dev HS256 subjects kept tenant-wide admin semantics until AUTHZ-3\n" +
      "landed roles (2026-07-19): an unbound subject now holds no\n" +
      "administrative power._\n" +
      "\n" +
      "_AUTHZ-2 deferrals (ADR-0014): the audit half closed 2026-07-19 — pack\n" +
      "assignment/default mutations chain `policy.{default,node}.*` events, and\n" +
      "the CLI's `policy apply/clear` chain `policy.pack.{applied,cleared}` as\n" +
      "break-glass (AUD-1). `MemoryRead` is the composition\n" +
      "seam; the AC's \"inject composition changes next session\" is\n" +
      "demonstrated at that seam and re-demonstrated end-to-end when\n" +
      "CTX-1/2/3 land on it. Governed handlers' placement and resource\n" +
      "chains are cached since HIER-2 (ADR-0016); chain assignments stay\n" +
      "per-request reads by design (ADR-0016 decision 6). Who may\n" +
      "assign was tenant-wide until AUTHZ-3 narrowed it to steward/org-admin\n" +
      "(2026-07-19); `standard`'s department\n" +
      "sharing collapses to strict where the hierarchy skips the department\n" +
      "level; `open-collaboration`'s \"non-restricted content\" qualifier is\n" +
      "AUTHZ-5 classification — the personal-scope exclusion is the current\n" +
      "privacy floor. A tenant default naming a custom pack that omits\n" +
      "`PolicyAssign` locks the tenant's policy plane; the store-level CLI is\n" +
      "the break-glass (node-level assignments cannot seal themselves —\n" +
      "ADR-0014 decision 4)._\n" +
      "\n" +
      "_AUTHZ-3 deferrals (ADR-0015): the audit half closed 2026-07-19 —\n" +
      "binding mutations chain `role.{bound,unbound}`, and the JIT admin-group\n" +
      "upsert chains `role.bound` on its first establishment only (repeat logins\n" +
      "are no-op upserts, ADR-0019 decision 6). Group-driven bindings are\n" +
      "additive-only (`synveda-admins` upserts tenant-wide org-admin at every\n" +
      "login; an admin-group subject with no team mapping is placed under the\n" +
      "org root, never quarantine); revocation stays explicit until AUTH-4/5\n" +
      "bring mover/leaver sync, and richer group→role mapping rules defer with\n" +
      "them. `synveda role bind` is the bootstrap and the break-glass — a\n" +
      "tenant that revokes its last org-admin recovers there. The embedded\n" +
      "packs bumped to `@2`; governed requests also read the subject's\n" +
      "bindings for the resource chain per request — kept per-request by\n" +
      "design since HIER-2 (ADR-0016 decision 6).\n" +
      "Roles whose actions land later are marker rows in the golden matrix\n" +
      "until those features extend it: curator approvals (FLOW-3),\n" +
      "security-reviewer (SKIL-2), compliance (AUTHZ-5/FLOW-3), auditor's\n" +
      "audit surface (AUD-2), contributor writes (MEM-1 — closed 2026-07-19:\n" +
      "`MemoryWrite` at bound non-personal scopes for contributor/curator,\n" +
      "ADR-0020 decision 3)._\n" +
      "\n" +
      "_HIER-2 notes (ADR-0016): scope chains are cached in-process,\n" +
      "invalidated post-commit by the hierarchy-mutating handlers; the gateway\n" +
      "is the hierarchy's only production writer, so any future out-of-process\n" +
      "writer (AUTH-4 SCIM sidecar, AUTH-5 directory sync, break-glass SQL)\n" +
      "must bring an invalidation channel — LISTEN/NOTIFY is the recorded\n" +
      "upgrade path, a gateway restart the manual recovery. Pack assignments,\n" +
      "role bindings, and identity rows deliberately stay per-request reads\n" +
      "(they carry ADR-0014/0015's next-request freshness promises); the\n" +
      "\"until HIER-2/3 cache them\" deferrals close as chains-only. CTX-2's\n" +
      "composition engine should consume `synveda_store::ScopeChainCache`\n" +
      "rather than re-reading closure rows._\n" +
      "\n" +
      "_HIER-3 notes (ADR-0017): Cedar entity fragments are cached per chain\n" +
      "inside the PDP, valid exactly for the chain shape they were built from\n" +
      "— freshness is inherited from the HIER-2 chain cache, so ADR-0012's\n" +
      "\"per-request entity building repeats work HIER-3 will cache\" deferral\n" +
      "closes. The gateway's mutation seams now call one helper\n" +
      "(`AppState::invalidate_hierarchy`) that flushes chains and fragments\n" +
      "together; future hierarchy writers (AUTH-4/5) call it — never the two\n" +
      "caches individually — and the ADR-0016 LISTEN/NOTIFY upgrade path\n" +
      "covers both. The principal entity stays per-request (identity freshness,\n" +
      "ADR-0016 decision 6). `Entities::from_entities` still runs per decision\n" +
      "over the small merged set; revisit only if CTX-1's inject budget shows\n" +
      "it dominating (ADR-0017's reversal trigger). CTX-2/3's per-candidate\n" +
      "`MemoryRead` sweep inherits prebuilt fragments through the same\n" +
      "facade._\n" +
      "\n" +
      "_AUTH-3 deferrals (ADR-0018): the audit half closed 2026-07-19 —\n" +
      "registration/revocation chain `service_identity.{registered,revoked}`,\n" +
      "and seam token rejections chain `auth.token.rejected` at the respond\n" +
      "seam (AUD-1, ADR-0019 decision 5). Tokens are IdP-issued\n" +
      "(client-credentials; Rauthy mints them as `sub: null` + `azp`, covered\n" +
      "by a bearer-only azp fallback in the verifier); per-issuer\n" +
      "`service_audiences` must list the agents' audiences. A token's scope is\n" +
      "exactly the registered anchor subtree — per-token narrowing via OAuth\n" +
      "scope claims is deferred (ADR-0018 option 8); re-anchoring an agent is\n" +
      "the existing PDP-gated hierarchy move of its personal leaf; secret\n" +
      "lifecycle stays IdP-side; agents can never act on the tenant plane\n" +
      "(the revisit trigger is recorded in ADR-0018). The embedded packs\n" +
      "bumped to `@3` (the service-identity plane joins the admin permits);\n" +
      "the base layer now carries the confinement forbid, whose one carve-out\n" +
      "is the role-free own-chain `MemoryRead` floor — CTX-1/2/3 inherit\n" +
      "agent composition through it. `synveda service` is the dev\n" +
      "bootstrap and break-glass._\n" +
      "\n" +
      "_AUD-1 notes (ADR-0019): one BLAKE3 chain per tenant; appends run inside\n" +
      "the operation's own tenant transaction (mutations are atomic with their\n" +
      "events; read handlers now commit — their allowed decision is a chain row),\n" +
      "deny-path events run in a short dedicated transaction at the per-plane\n" +
      "`respond` seams, best-effort (`synveda_audit_append_failures_total`; the\n" +
      "original error is never masked). The CLI break-glass audits itself\n" +
      "(actor kind `break_glass`, OS-user attribution). `synveda audit\n" +
      "verify/tail` is the operator surface until AUD-2's query API. Forward\n" +
      "obligations: MEM-1's observe and CTX-1/2/3's inject/recall are emission\n" +
      "points on the same seams — inject chains ONE event carrying its\n" +
      "commit-hash watermarks with per-candidate `MemoryRead` decisions\n" +
      "aggregated, never one row per candidate (ADR-0019 decision 4); if CTX-3's\n" +
      "latency AC shows the chain-head lock or synchronous append dominating,\n" +
      "the recorded upgrade is a buffered appender for read-path decision events\n" +
      "only (ADR-0019 option 2). Chain anchoring beyond the database (signed\n" +
      "export, offline verification) is AUD-3; the auditor-role read surface is\n" +
      "AUD-2; audit-row retention/erasure semantics land with TEN-5._\n" +
      "\n" +
      "_MEM-1 notes (ADR-0020): observe writes land at the caller's personal\n" +
      "(home) scope only — the API takes no scope; placement decides. Content\n" +
      "stages in the RLS-forced, app-append-only `observe_events` table inside\n" +
      "the caller's tenant transaction; the PGMQ `observe` queue carries\n" +
      "content-free `{tenant_id, event_id}` signals. Idempotency is\n" +
      "buffer-level (`unique (tenant_id, idempotency_key)`, first-writer-wins,\n" +
      "duplicate = 202 with the original ids): what never enters twice can\n" +
      "never be extracted twice, so the AC holds structurally before MEM-2/3\n" +
      "exist. `MemoryWrite` joined the vocabulary (packs bumped to `@4`): the\n" +
      "role-free own-home floor plus the contributor/curator grant at bound\n" +
      "non-personal scopes — pack-uniform; writes beyond home always take an\n" +
      "explicit grant. The base layer is untouched (an agent's home leaf lies\n" +
      "inside its anchor subtree). Forward obligations: the queue has no\n" +
      "consumer until MEM-2/3 — signals accumulate and the pipeline must\n" +
      "archive them; staging rows are immutable provenance whose\n" +
      "retention/disposal lands with MEM-6/TEN-5, which must honour the\n" +
      "idempotency horizon (ADR-0020); redaction-before-persistence (seed §6)\n" +
      "is honestly not yet true — staging holds pre-redaction content under\n" +
      "RLS until MEM-2 inserts itself between buffer and extraction. The load\n" +
      "AC asserts the sustained rate and the ack MEDIAN against the 20ms\n" +
      "budget plus the measured dev-database link tax (the HIER-1 discipline\n" +
      "for IO-crossing perf ACs; Docker Desktop's fsync stalls own the upper\n" +
      "percentiles, which are reported only) — EVAL-6 owns percentile SLO\n" +
      "enforcement on production-shaped IO, and ADR-0019 option 2's buffered\n" +
      "appender remains the recorded upgrade if per-tenant chain serialisation\n" +
      "ever binds real burst traffic._\n" +
      "\n" +
      "_MEM-2 notes (ADR-0021): scanning runs in the observe ack path, before\n" +
      "the staging insert — ADR-0020's \"redaction-before-persistence is not\n" +
      "yet true\" debt is paid; staging only ever holds redacted content, and\n" +
      "the raw finding text has no representation anywhere (placeholder +\n" +
      "rule id only, in tables, responses, metrics, and audit payloads\n" +
      "alike). Modes are per category per pack (`RedactionConfig\n" +
      "{secrets, pii}` × deny/redact/quarantine): embedded configs are\n" +
      "compiled in (strict = secrets quarantine + PII redact; standard/open =\n" +
      "redact both), stored packs configure via `policy_packs.redaction`\n" +
      "(`synveda policy apply --redaction-secrets … --redaction-pii …`) and\n" +
      "hot-reload with the pack; unconfigured stored packs get the strict\n" +
      "config (fail safe). Quarantined events stage signal-less behind\n" +
      "`observe_quarantine` (RLS-forced, column-level UPDATE grants, one-shot\n" +
      "pending→released|rejected transition trigger); release sends the\n" +
      "standard `{tenant_id, event_id}` signal so the MEM-3 consumer contract\n" +
      "is unchanged; reject leaves the staging row provenance-only. The\n" +
      "review plane is `QuarantineRead`/`QuarantineReview` (packs @5),\n" +
      "granted pack-uniformly to steward/org-admin/security-reviewer — the\n" +
      "security-reviewer marker's first live actions — with auditor excluded\n" +
      "(content) and no owner self-release; this is the recorded oversight\n" +
      "carve-out of the personal-scope privacy floor, bounded by redaction.\n" +
      "Forward obligations: `observe_quarantine` and staging retention share\n" +
      "one disposal horizon (MEM-6/TEN-5, ADR-0020/0021); MEM-3 extraction\n" +
      "must treat `[REDACTED:*]` placeholders as opaque tokens; ruleset\n" +
      "precision/recall measurement lands with EVAL-2's labelled fixtures\n" +
      "(the recorded trigger for an ML pass behind the `Ruleset` seam); an\n" +
      "event at a since-deleted scope is unreviewable via the API (uniform\n" +
      "404) and awaits disposal. The scan is spawn_blocking CPU, O(payload\n" +
      "bytes); the MEM-1 load AC shape stays the asserted ack bound and\n" +
      "still passes with the seam in place — EVAL-6 owns percentile SLOs._\n" +
      "\n" +
      "_MEM-3 notes (ADR-0022): extraction runs as a PGMQ-polling worker embedded\n" +
      "in the gateway (`SYNVEDA_EXTRACTOR`: `deterministic` by default, `claude` /\n" +
      "`vllm` / `off`), its stages Temporal-shaped — serializable activity I/O,\n" +
      "orchestration split from the polling transport — so the enterprise profile\n" +
      "(OPS-2) can host the same stages under the Temporal SDK later; the SDK\n" +
      "itself is deferred (git-distributed, ring/aws-lc licence graph — deny.toml\n" +
      "refuses both). Exactly-once is the archive-lock: `pgmq.archive` runs inside\n" +
      "the tenant write transaction before the record inserts, so redelivery and\n" +
      "racing consumers cannot duplicate records; a deliberately re-sent signal is\n" +
      "intentional reprocessing, with MEM-5's dedup as the semantic net, and\n" +
      "`pgmq.a_observe` is the dead-letter/completion record (no new table). The\n" +
      "pipeline's write re-decides `MemoryWrite` at the owner's *current* home\n" +
      "under current facts (a mover's memories follow the mover; a\n" +
      "since-quarantined owner is denied); denials archive and chain standalone\n" +
      "decision events under the new `system` actor kind (migration 0014 — MEM-6\n" +
      "sweeps and AUTH-4/5 sync inherit it). Extractor output re-enters the MEM-2\n" +
      "scanner before persisting — an LLM echoing a live-format secret writes the\n" +
      "placeholder — and sensitivity is floored at `internal` until AUTHZ-5\n" +
      "brings classification. Confidence is model-elicited and uncalibrated; the\n" +
      "provisional macro-precision target (≥0.8 on the labelled fixtures) and the\n" +
      "`--ignored` live-LLM measurement stand in until EVAL-2 owns the real\n" +
      "target, dashboard, and calibration. Forward obligations: MEM-4 wraps the\n" +
      "one commit seam with embed-or-fail; MEM-5 inserts dedup between extract\n" +
      "and commit; FLOW-1/2 replace the direct records insert with the\n" +
      "derived-channel commit; the <60s pipeline-lag SLO is evidenced by\n" +
      "`synveda_extraction_lag_seconds` (EVAL-6 owns SLO enforcement); and\n" +
      "LISTEN/NOTIFY replaces polling if idle load or measured lag ever matters\n" +
      "(ADR-0022's recorded upgrade)._",
  ],
]);

// ── Parse Part B ─────────────────────────────────────────────────────────────
const lines = readFileSync(SRC, "utf8").split(/\r?\n/);

const epicRe = /^EPIC ([A-Z]+) — (.+)$/;
// "FND-1  Workspace scaffold (S)" / "TEN-6  Cross-tenant isolation test harness (M) [continuous]"
const blockRe = /^([A-Z]+-\d+)\s+(.+)\s\((S|M|L)\)(?:\s+\[([^\]]+)\])?\s*$/;
// "CNSL-2 Hierarchy & policy explorer (M) — visualise scopes, packs, roles, active lapses."
const inlineRe = /^([A-Z]+-\d+)\s+(.+?)\s\((S|M|L)\)\s+—\s+(.*)$/;

const features = new Map();
let currentEpic = null;
let current = null;

function close() {
  if (current) {
    features.set(current.id, current);
    current = null;
  }
}

for (const raw of lines) {
  const line = raw.trimEnd();
  if (/^─+$/.test(line) || line.startsWith("Sequencing")) {
    close();
    if (line.startsWith("Sequencing")) currentEpic = null;
    continue;
  }
  const epicM = line.match(epicRe);
  if (epicM) {
    close();
    currentEpic = { code: epicM[1], title: epicM[2] };
    continue;
  }
  if (!currentEpic) continue;
  const inlineM = line.match(inlineRe);
  const blockM = inlineM ? null : line.match(blockRe);
  if (inlineM || blockM) {
    close();
    const m = inlineM ?? blockM;
    current = {
      id: m[1],
      name: m[2],
      size: m[3],
      marker: inlineM ? null : (m[4] ?? null),
      epic: currentEpic.code,
      epicTitle: currentEpic.title,
      body: inlineM ? [m[4]] : [],
    };
    continue;
  }
  if (current && /^\s{2,}\S/.test(raw)) {
    current.body.push(line.trim());
    continue;
  }
  if (line !== "") close();
}
close();

// ── Validate against the phase transcription ────────────────────────────────
const phaseOf = new Map();
for (const p of PHASES) {
  for (const id of p.ids) {
    if (phaseOf.has(id)) throw new Error(`duplicate in phase map: ${id}`);
    phaseOf.set(id, p.n);
  }
}
const missing = [...phaseOf.keys()].filter((id) => !features.has(id));
if (missing.length) {
  throw new Error(`Sequencing transcription lists unparsed features: ${missing.join(", ")}`);
}
const unscheduled = [...features.keys()].filter((id) => !phaseOf.has(id));
if (unscheduled.length) {
  console.warn(`WARN: not in Sequencing, labelled phase:unscheduled: ${unscheduled.join(", ")}`);
}

// ── Emit one file per feature ────────────────────────────────────────────────
mkdirSync(OUT, { recursive: true });

function splitBody(bodyLines) {
  const body = bodyLines.join(" ").replace(/\s+/g, " ").trim();
  const i = body.search(/\bAC:\s/);
  if (i === -1) return { desc: body, ac: null };
  return {
    desc: body.slice(0, i).trim(),
    ac: body.slice(i).replace(/^AC:\s*/, "").trim(),
  };
}

for (const f of features.values()) {
  const { desc, ac } = splitBody(f.body);
  const phase = phaseOf.get(f.id);
  const phaseLabel = phase === undefined ? "unscheduled" : String(phase);
  const out = [
    "---",
    `title: ${JSON.stringify(`${f.id}: ${f.name}`)}`,
    "labels:",
    `  - epic:${f.epic}`,
    `  - phase:${phaseLabel}`,
    `size: ${f.size}`,
    ...(f.marker ? [`marker: ${JSON.stringify(f.marker)}`] : []),
    "---",
    "",
    `# ${f.id}: ${f.name}`,
    "",
    `**Epic:** ${f.epic} — ${f.epicTitle} · **Phase:** ${phaseLabel} · **Size:** ${f.size}` +
      (f.marker ? ` · **Marker:** ${f.marker}` : ""),
    "",
    "## Description",
    "",
    desc,
    "",
    "## Acceptance criteria",
    "",
    ac ?? "_No acceptance criteria specified in SYNVEDA_FEATURES.md._",
    "",
  ];
  writeFileSync(path.join(OUT, `${f.id}.md`), out.join("\n"));
}

// ── STATUS.md checklist ──────────────────────────────────────────────────────
const status = [
  "# Backlog status",
  "",
  `${features.size} features parsed from docs/SYNVEDA_FEATURES.md — one file per`,
  "feature in this directory. Phases per the Sequencing section. Regenerate with",
  "`node scripts/generate-backlog.mjs` (preserves done-marks listed in the script).",
  "",
  "Phase 1+ must not start until FND is complete and `make dev-up && make smoke`",
  "passes (CLAUDE.md, current phase).",
  "",
];
for (const p of PHASES) {
  status.push(`## ${p.title}`, "");
  if (p.demo) status.push(`_Phase demo goal: ${p.demo}_`, "");
  for (const id of p.ids) {
    const f = features.get(id);
    const done = DONE.get(id);
    status.push(
      `- [${done ? "x" : " "}] [${id}: ${f.name}](${id}.md)${done ? ` — ${done}` : ""}`,
    );
  }
  status.push("");
  const note = PHASE_NOTES.get(p.n);
  if (note) status.push(note, "");
}
if (unscheduled.length) {
  status.push("## Unscheduled — not listed in the Sequencing section", "");
  for (const id of unscheduled) {
    status.push(`- [ ] [${id}: ${features.get(id).name}](${id}.md)`);
  }
  status.push("");
}
writeFileSync(path.join(OUT, "STATUS.md"), status.join("\n"));

console.log(`wrote ${features.size} feature files + STATUS.md to ${OUT}/`);

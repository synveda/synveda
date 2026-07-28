# Backlog status

86 features parsed from docs/SYNVEDA_FEATURES.md — one file per
feature in this directory. Phases per the Sequencing section. Regenerate with
`node scripts/generate-backlog.mjs` (preserves done-marks listed in the script).

Note (2026-07-25): the per-feature **narrative paragraphs** below are
hand-written here and are *not* in the generator's `PHASE_NOTES`, so a
regeneration would discard everything from FLOW-3's paragraph onward. Add
the done-mark to `DONE` in the script as usual; if you ever need to
regenerate, lift the paragraphs out of git first.

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
- [x] [HIER-2: Scope chain resolver](HIER-2.md) — done 2026-07-19, AC tests: crates/synveda-store/tests/scope_chain.rs (invalidation serves the fresh chain after a move; warm resolve median 800ns, p99 ≤1.5µs over 10k samples — 300× under the 0.5ms bound), crates/synveda-gateway/tests/scope_chain_routes.rs (a move governs the very next request through the cache), demo: demos/hier-2-scope-chain.sh
- [x] [HIER-3: Cedar entity sync](HIER-3.md) — done 2026-07-19, AC tests: crates/synveda-gateway/tests/cedar_entity_sync.rs (a team moved between departments governs the very next decision: the moving steward's authority leaves with it over HTTP; the department MemoryRead follows it at the composition seam), crates/synveda-policy/tests/entity_sync.rs (a warm fragment never survives a reshaped chain, both directions), demo: demos/hier-3-cedar-entity-sync.sh
- [x] [AUTH-3: Service identities](AUTH-3.md) — done 2026-07-19, AC tests: crates/synveda-gateway/tests/service_identities.rs (client-credentials grant end to end against a mock IdP; a team-anchored agent holding tenant-wide org-admin is denied every org-scope endpoint; unregistered clients quarantined; lifetime cap; PDP-gated registration; next-request revocation), crates/synveda-policy/tests/service_scope.rs (the base-layer confinement forbid across the action vocabulary; the own-chain MemoryRead floor survives; roles cannot widen past the token scope), demo: demos/auth-3-service-identities.sh (live Rauthy)
- [x] [AUD-1: Hash-chained audit log](AUD-1.md) — done 2026-07-19, AC test: crates/synveda-audit/tests/tamper.rs (a database-credentialed attacker suppresses triggers and rewrites history: every hashed column, row removal, relinking, and head attacks all break verification at the named seq), emission tests: crates/synveda-gateway/tests/audit_events.rs (mutation/read/denial/suspended-tenant/token-rejection each chain one event and the chain verifies), crates/synveda-store/tests/rls.rs (audit tables join the adversarial RLS suite), demo: demos/aud-1-audit-log.sh
- [x] [MEM-1: observe API + PGMQ buffer](MEM-1.md) — done 2026-07-19, AC tests: crates/synveda-gateway/tests/observe.rs (duplicate delivery admits nothing twice — response, staging table, queue, and audit chain all agree; 1k events/s sustained with the ack median inside the 20ms-plus-link-tax budget), crates/synveda-store/tests/rls.rs (observe buffer joins the adversarial RLS suite; PGMQ grants proven under synveda_app), crates/synveda-policy/tests/{packs,roles,service_scope}.rs (the MemoryWrite floor and grant golden-tested), demo: demos/mem-1-observe.sh
- [x] [MEM-2: Redaction & secret scanning](MEM-2.md) — done 2026-07-19, AC tests: crates/synveda-gateway/tests/observe_redaction.rs (seeded secrets swept for across staging, quarantine, audit, and both PGMQ tables under all three modes — zero hits; the review queue E2E: security-reviewer's first live action, owner denied self-release, release sends the standard signal, one-shot 409), crates/synveda-ingest/tests/redaction.rs (every rule + validators + the scanner-output-never-contains-matched-text discipline), crates/synveda-store/tests/rls.rs (observe_quarantine joins the adversarial suite; column-bound one-shot review), crates/synveda-policy/tests/{packs,roles}.rs (quarantine plane golden-tested; redaction config rides the effective pack), demo: demos/mem-2-redaction.sh
- [x] [MEM-3: Extraction pipeline](MEM-3.md) — done 2026-07-22, AC tests: crates/synveda-ingest/tests/extraction_precision.rs (labelled fixture set, per-class precision; deterministic macro 0.958 ≥ the provisional 0.8 target; the `#[ignore]`d live-LLM hook runs the same harness against Claude or vLLM), crates/synveda-gateway/tests/extraction.rs (observe → worker → records with the provenance quadruple on every record; archive-lock exactly-once under redelivery; a released quarantined event extracts identically; a since-quarantined owner is denied at commit; retries exhaust into an audited dead-letter; an extractor echoing a live-format secret persists only the placeholder), crates/synveda-ingest/tests/extractor_http.rs (Claude/vLLM request contracts against local mocks), crates/synveda-store/tests/observe_queue.rs (visibility timeout, redelivery, archive-as-lock), demo: demos/mem-3-extraction.sh
- [x] [MEM-4: Transactional embed-or-fail](MEM-4.md) — done 2026-07-22, AC tests: crates/synveda-gateway/tests/embedding.rs (the chaos test: a mock TEI killed mid-batch — the embedded event commits atomically with its vector, the rest redeliver and commit on recovery, zero lost and zero embedding-less at every phase; the schema backstop refuses a raw-SQL embedding-less commit; the deterministic zero-config path), crates/synveda-ingest/tests/embedder_http.rs (TEI request contract and failure taxonomy against local mocks — error status, count mismatch, empty vector, dead endpoint all `Dependency`), crates/synveda-store/tests/rls.rs (record_embeddings joins the adversarial suite: tenant-isolated, forged-tenant write rejected, app role holds no DELETE, record delete cascades), crates/synveda-store/tests/bitemporal.rs (every write now carries its embedding through the one-statement API), demo: demos/mem-4-embedding.sh (real TEI/BGE-M3 stopped and restarted mid-pipeline)
- [x] [CTX-1: Hybrid retrieval](CTX-1.md) — done 2026-07-23, AC tests: crates/synveda-retrieval/tests/quality.rs (the fixture set, each leg blind to half the corpus by construction: sparse-only 0.500, dense-only 0.500, hybrid 1.000 recall@6), crates/synveda-gateway/tests/retrieval_live.rs (`--ignored`: the same fixtures through live TEI/BGE-M3 — sparse-only 0.500, hybrid 0.792 recall@6, MRR 1.0), crates/synveda-retrieval/tests/latency.rs (`--ignored`: 1M records/tenant, median asserted under the 80ms budget, tails reported — the HIER-1/MEM-1 discipline), crates/synveda-retrieval/tests/hybrid.rs (fusion order end to end; adversarial no-leak on both legs at once; one-sided staleness; degradation modes; watermark/overlap/rebuild), crates/synveda-retrieval/tests/permitted_scopes.rs (the PDP-derived predicate: own chain, quarantine, unplaced, the service-identity floor), demo: demos/ctx-1-hybrid-retrieval.sh (observe → pipeline → real-TEI vectors → sidecar convergence → live quality harness)
- [x] [CTX-2: Composition engine](CTX-2.md) — done 2026-07-23, AC tests: crates/synveda-retrieval/tests/compose.rs (deterministic: byte-identical re-composition at the same instant, unrelated writes notwithstanding; the watermark: BLAKE3 version hashes + record ids on every block, block hash recomputable from the entry hashes; tokens_per_inject recorded on every compose including the zero-entry one; plus gradient/pinned-first assembly, first-fit budget, channel rules, the seed §4.4 conflict matrix, valid-time as-of, the sensitivity clamp, relevance ranking), crates/synveda-retrieval/tests/composition_plan.rs (the PDP sweep: per-scope channel rules and the home-scope budget from effective packs; bank-mode subtree inheritance; quarantine/unplaced plan nothing), crates/synveda-store/tests/policy_packs.rs (the composition config rides the stored pack; re-apply clears; garbage json reads unconfigured), demo: demos/ctx-2-composition.sh (observe → pipeline → seeded scope material → the compose example over the real product path — identity → HIER-2 chain → PDP plan → compose — then the bank-mode pack flip governs the very next compose)
- [x] [CTX-3: inject API](CTX-3.md) — done 2026-07-23, AC tests: crates/synveda-gateway/tests/inject.rs (the degradation matrix: embedder down → sparse-only still ranked + the warning header; broken sidecar → unranked compose + the header; contract rejections stay honest errors — plus the full product path with the watermark and exactly one `context.injected` event, aggregated decisions, task as hash only; taskless recency; quarantined/unplaced → the empty block, still audited; budget narrowing never widening; the bank-mode pack governing the very next inject), crates/synveda-gateway/tests/inject_latency.rs (`--ignored`: 1,000 concurrent sessions arriving at 50/s — p50 18.6ms, p95 22ms, p99 24ms against the 150ms budget, median asserted, tails + stage split reported; the closed-loop saturation probe prints the per-tenant chain-lock ceiling every run), demo: demos/ctx-3-inject.sh (real TEI end to end; TEI stopped mid-demo degrades the same inject to sparse-only with the header and recovers on restart; audit tail + verify)
- [x] [ADPT-1: Claude Code adapter](ADPT-1.md) — done 2026-07-25, AC test: demos/adpt-1-claude-code.sh (a clean HOME to a personalised session in 1.5s of the 120s budget: the prebuilt plugin enabled, `synveda login` through live Rauthy with AUTH-2 placing a first-time identity, a watermarked block composed from team memory the user never configured, the turn observed back, then `context.injected` + `memory.observed` joined under one `claude-code:<id>` in a verifying chain, the access token renewing itself, and the observed turn returning as memory in the next session), adapters/claude-code/src/driver.test.mts + `node dist/driver.mjs` (the recorded-payload driver over fixtures/, sixteen cases against a mock and against the live gateway — dead gateway, 401, degraded header, oversized tool result, replayed batch, cursor resume after a failed flush, damaged transcript line, unreadable payload; every one exits 0), adapters/claude-code/src/{hook,events,transcript,spool,credentials,log}.test.mts (the handler, mapping, parser, spool, and CLI-seam suites), crates/synveda-gateway/tests/cli_login.rs (the loopback allowlist, the single-use state-bound handoff, the refresh grant, no token in any redirect URL)
- [x] [EVAL-1: Eval harness skeleton](EVAL-1.md) — done 2026-07-25, AC test: demos/eval-1-harness.sh (the suite green and the gate holding, then the bank-mode switch thrown for real — a published-only pack assigned at the org — and the very next run measuring the same product answering worse: recall 1.0 → 0.0, accuracy 1.0 → 0.5, the gate failing with the axis, the baseline, the measurement, and the delta; then the pack withdrawn and the suite green again, which is what makes the failure a measurement rather than a broken demo), crates/synveda-eval (20 unit tests: the scenario format refusing unknown fields and dangling keys, per-axis reduction over the scenarios that measure each axis, nearest-rank percentiles, floor and ceiling breaches, a bounded metric that stopped being measured, baseline updates keeping each bound on its own side, and the grading rules — recall, leak, abstention, and the budget invariant), `make eval` (the live run), `make eval-check` (suite and baseline parse with no stack, in `make ci`), nightly: .github/workflows/eval.yml

_Phase 1 complete 2026-07-25. The phase demo goal — "SSO login →
auto-scoped → live Claude Code session writes and receives governed
memory, fully audited" — is `demos/adpt-1-claude-code.sh` end to end, and
`demos/eval-1-harness.sh` is the phase's other half: the same spine, now
measured on five axes with a gate that fails when it gets worse. Phase 2
(VedaFlow) may start._

_Order revised 2026-07-18 (was TEN → AUTH → AUTHZ → HIER → MEM → CTX →
AUD): the epic-grouped sequence was not a valid topological order. HIER-1
now precedes AUTHZ-1 and AUTH-2 (Cedar entities and JIT provisioning need
hierarchy nodes to exist); AUTH-3 follows AUTHZ-1 (its scope-enforcement AC
is a PDP decision); AUD-1 moves ahead of MEM-1 so the data path is born
audited and the ADR-0008/0009 emission-point retrofit stays bounded to the
identity features._

_TEN-1 deferral (ADR-0008): closed 2026-07-19 — AUD-1 chains
`tenant.resolution.denied` when a verified token names a suspended tenant
(ADR-0019 decision 6). Successful resolutions stay implicit (every
subsequent chained event proves one); unauthenticated failures carry no
attributable subject and remain in traces and
`synveda_tenant_resolutions_total`._

_TEN-2 deferrals (ADR-0009): the audit half closed 2026-07-19 — backstop
trips (SQLSTATE 42501, now marked via `rls::backstop_error` and classified
by `rls::is_backstop_trip`) chain as `store.rls.denied` at the gateway's
respond seam (AUD-1, ADR-0019 decision 5). Still standing: data-path
features must reach tenant-scoped tables via
`synveda_store::rls::begin_tenant_tx`, and deployment profiles
(OPS-1/OPS-2) must connect as a non-superuser `synveda_app` login — the
dev compose superuser bypasses RLS._

_HIER-1 deferrals (ADR-0011): the audit half closed 2026-07-19 — every
hierarchy mutation chains `hierarchy.node.{created,updated,deleted}` with
pre/post images in the mutation's own transaction (AUD-1, ADR-0019). The
`/v1/hierarchy/*` admin routes' PDP gate — AUTHZ-1's first obligation —
was discharged 2026-07-18: every handler authorizes through the Cedar
facade (ADR-0012 decision 7)._

_AUTHZ-1 deferrals (ADR-0012): the audit half closed 2026-07-19 with
ADR-0019 decision 4's shape — one chained event per audited operation:
mutations embed their decision context (pack@version, determining
policies, roles), denials and allowed admin-plane reads chain standalone
`authz.decision` events. The per-call decision log and
`synveda_authz_decisions_total` continue unchanged and remain the
full-fidelity record of every individual PDP call. The `bootstrap` pack was retired
2026-07-19: AUTHZ-2 replaced it with the embedded product packs
(`regulated-strict` is the zero-config default; roles still arrive with
AUTHZ-3). Stored-pack propagation lags up to
`SYNVEDA_POLICY_REFRESH_SECS` (default 5s, poll-based) until VedaFlow
policy commits drive event-based reload._

_AUTH-2 deferrals (ADR-0013): the audit half closed 2026-07-19 —
`identity.provisioned` chains in the provisioning transaction whenever an
identity row is created (mapped/admin/quarantined placements; `existing`
logins chain nothing — ADR-0019 decision 6).
Group-mapping overrides are store-managed until an admin surface
exists; placement is first-login-final — movers/leavers arrive with
AUTH-4/5, and release from quarantine is the existing PDP-gated
hierarchy move. The quarantine forbid now lives in the base layer
compiled into every pack (ADR-0014 decision 2); an IdP subject that
never completed a login is quarantined at the PDP seam (fail closed).
Dev HS256 subjects kept tenant-wide admin semantics until AUTHZ-3
landed roles (2026-07-19): an unbound subject now holds no
administrative power._

_AUTHZ-2 deferrals (ADR-0014): the audit half closed 2026-07-19 — pack
assignment/default mutations chain `policy.{default,node}.*` events, and
the CLI's `policy apply/clear` chain `policy.pack.{applied,cleared}` as
break-glass (AUD-1). `MemoryRead` is the composition
seam; the AC's "inject composition changes next session" is
demonstrated at that seam and re-demonstrated end-to-end when
CTX-1/2/3 land on it. Governed handlers' placement and resource
chains are cached since HIER-2 (ADR-0016); chain assignments stay
per-request reads by design (ADR-0016 decision 6). Who may
assign was tenant-wide until AUTHZ-3 narrowed it to steward/org-admin
(2026-07-19); `standard`'s department
sharing collapses to strict where the hierarchy skips the department
level; `open-collaboration`'s "non-restricted content" qualifier is
AUTHZ-5 classification — the personal-scope exclusion is the current
privacy floor. A tenant default naming a custom pack that omits
`PolicyAssign` locks the tenant's policy plane; the store-level CLI is
the break-glass (node-level assignments cannot seal themselves —
ADR-0014 decision 4)._

_AUTHZ-3 deferrals (ADR-0015): the audit half closed 2026-07-19 —
binding mutations chain `role.{bound,unbound}`, and the JIT admin-group
upsert chains `role.bound` on its first establishment only (repeat logins
are no-op upserts, ADR-0019 decision 6). Group-driven bindings are
additive-only (`synveda-admins` upserts tenant-wide org-admin at every
login; an admin-group subject with no team mapping is placed under the
org root, never quarantine); revocation stays explicit until AUTH-4/5
bring mover/leaver sync, and richer group→role mapping rules defer with
them. `synveda role bind` is the bootstrap and the break-glass — a
tenant that revokes its last org-admin recovers there. The embedded
packs bumped to `@2`; governed requests also read the subject's
bindings for the resource chain per request — kept per-request by
design since HIER-2 (ADR-0016 decision 6).
Roles whose actions land later are marker rows in the golden matrix
until those features extend it: security-reviewer's skill approval
(SKIL-2) — the last one. Closed since: contributor
writes (MEM-1, 2026-07-19 — `MemoryWrite` at bound non-personal scopes
for contributor/curator, ADR-0020 decision 3); security-reviewer's first
live action (MEM-2, 2026-07-19 — the quarantine review plane); curator's
"can pin/approve" (FLOW-2, 2026-07-25 — the channel plane, then FLOW-3's
proposal review); **compliance** (FLOW-3, 2026-07-25 — `ProposalRead`
and `ProposalReview`, with the invariant approval floor requiring the
role on everything `restricted`, ADR-0032 decision 4); and **auditor**
(AUD-2, 2026-07-28 — `AuditRead`, the role's first and only live action,
on the read-only admin permit whose comment had named the feature since
AUTHZ-2, ADR-0045 decision 1)._

_HIER-2 notes (ADR-0016): scope chains are cached in-process,
invalidated post-commit by the hierarchy-mutating handlers; the gateway
is the hierarchy's only production writer, so any future out-of-process
writer (AUTH-4 SCIM sidecar, AUTH-5 directory sync, break-glass SQL)
must bring an invalidation channel — LISTEN/NOTIFY is the recorded
upgrade path, a gateway restart the manual recovery. Pack assignments,
role bindings, and identity rows deliberately stay per-request reads
(they carry ADR-0014/0015's next-request freshness promises); the
"until HIER-2/3 cache them" deferrals close as chains-only. CTX-2's
composition engine should consume `synveda_store::ScopeChainCache`
rather than re-reading closure rows._

_HIER-3 notes (ADR-0017): Cedar entity fragments are cached per chain
inside the PDP, valid exactly for the chain shape they were built from
— freshness is inherited from the HIER-2 chain cache, so ADR-0012's
"per-request entity building repeats work HIER-3 will cache" deferral
closes. The gateway's mutation seams now call one helper
(`AppState::invalidate_hierarchy`) that flushes chains and fragments
together; future hierarchy writers (AUTH-4/5) call it — never the two
caches individually — and the ADR-0016 LISTEN/NOTIFY upgrade path
covers both. The principal entity stays per-request (identity freshness,
ADR-0016 decision 6). `Entities::from_entities` still runs per decision
over the small merged set; revisit only if CTX-1's inject budget shows
it dominating (ADR-0017's reversal trigger). CTX-2/3's per-candidate
`MemoryRead` sweep inherits prebuilt fragments through the same
facade._

_AUTH-3 deferrals (ADR-0018): the audit half closed 2026-07-19 —
registration/revocation chain `service_identity.{registered,revoked}`,
and seam token rejections chain `auth.token.rejected` at the respond
seam (AUD-1, ADR-0019 decision 5). Tokens are IdP-issued
(client-credentials; Rauthy mints them as `sub: null` + `azp`, covered
by a bearer-only azp fallback in the verifier); per-issuer
`service_audiences` must list the agents' audiences. A token's scope is
exactly the registered anchor subtree — per-token narrowing via OAuth
scope claims is deferred (ADR-0018 option 8); re-anchoring an agent is
the existing PDP-gated hierarchy move of its personal leaf; secret
lifecycle stays IdP-side; agents can never act on the tenant plane
(the revisit trigger is recorded in ADR-0018). The embedded packs
bumped to `@3` (the service-identity plane joins the admin permits);
the base layer now carries the confinement forbid, whose one carve-out
is the role-free own-chain `MemoryRead` floor — CTX-1/2/3 inherit
agent composition through it. `synveda service` is the dev
bootstrap and break-glass._

_AUD-1 notes (ADR-0019): one BLAKE3 chain per tenant; appends run inside
the operation's own tenant transaction (mutations are atomic with their
events; read handlers now commit — their allowed decision is a chain row),
deny-path events run in a short dedicated transaction at the per-plane
`respond` seams, best-effort (`synveda_audit_append_failures_total`; the
original error is never masked). The CLI break-glass audits itself
(actor kind `break_glass`, OS-user attribution). `synveda audit
verify/tail` was the operator surface until AUD-2's query API landed
(2026-07-28, ADR-0045); both remain as the direct-to-store break-glass
for an operator who has lost the gateway, which is the split ADR-0045
decision 11 draws. Forward
obligations: MEM-1's observe and CTX-1/2/3's inject/recall are emission
points on the same seams — inject chains ONE event carrying its
commit-hash watermarks with per-candidate `MemoryRead` decisions
aggregated, never one row per candidate (ADR-0019 decision 4); if CTX-3's
latency AC shows the chain-head lock or synchronous append dominating,
the recorded upgrade is a buffered appender for read-path decision events
only (ADR-0019 option 2). Chain anchoring beyond the database (signed
export, offline verification) is AUD-3; the auditor-role read surface is
AUD-2; audit-row retention/erasure semantics land with TEN-5._

_MEM-1 notes (ADR-0020): observe writes land at the caller's personal
(home) scope only — the API takes no scope; placement decides. Content
stages in the RLS-forced, app-append-only `observe_events` table inside
the caller's tenant transaction; the PGMQ `observe` queue carries
content-free `{tenant_id, event_id}` signals. Idempotency is
buffer-level (`unique (tenant_id, idempotency_key)`, first-writer-wins,
duplicate = 202 with the original ids): what never enters twice can
never be extracted twice, so the AC holds structurally before MEM-2/3
exist. `MemoryWrite` joined the vocabulary (packs bumped to `@4`): the
role-free own-home floor plus the contributor/curator grant at bound
non-personal scopes — pack-uniform; writes beyond home always take an
explicit grant. The base layer is untouched (an agent's home leaf lies
inside its anchor subtree). Forward obligations: the queue has no
consumer until MEM-2/3 — signals accumulate and the pipeline must
archive them; staging rows are immutable provenance whose
retention/disposal lands with MEM-6/TEN-5, which must honour the
idempotency horizon (ADR-0020); redaction-before-persistence (seed §6)
is honestly not yet true — staging holds pre-redaction content under
RLS until MEM-2 inserts itself between buffer and extraction. The load
AC asserts the sustained rate and the ack MEDIAN against the 20ms
budget plus the measured dev-database link tax (the HIER-1 discipline
for IO-crossing perf ACs; Docker Desktop's fsync stalls own the upper
percentiles, which are reported only) — EVAL-6 owns percentile SLO
enforcement on production-shaped IO, and ADR-0019 option 2's buffered
appender remains the recorded upgrade if per-tenant chain serialisation
ever binds real burst traffic._

_MEM-2 notes (ADR-0021): scanning runs in the observe ack path, before
the staging insert — ADR-0020's "redaction-before-persistence is not
yet true" debt is paid; staging only ever holds redacted content, and
the raw finding text has no representation anywhere (placeholder +
rule id only, in tables, responses, metrics, and audit payloads
alike). Modes are per category per pack (`RedactionConfig
{secrets, pii}` × deny/redact/quarantine): embedded configs are
compiled in (strict = secrets quarantine + PII redact; standard/open =
redact both), stored packs configure via `policy_packs.redaction`
(`synveda policy apply --redaction-secrets … --redaction-pii …`) and
hot-reload with the pack; unconfigured stored packs get the strict
config (fail safe). Quarantined events stage signal-less behind
`observe_quarantine` (RLS-forced, column-level UPDATE grants, one-shot
pending→released|rejected transition trigger); release sends the
standard `{tenant_id, event_id}` signal so the MEM-3 consumer contract
is unchanged; reject leaves the staging row provenance-only. The
review plane is `QuarantineRead`/`QuarantineReview` (packs @5),
granted pack-uniformly to steward/org-admin/security-reviewer — the
security-reviewer marker's first live actions — with auditor excluded
(content) and no owner self-release; this is the recorded oversight
carve-out of the personal-scope privacy floor, bounded by redaction.
Forward obligations: `observe_quarantine` and staging retention share
one disposal horizon (MEM-6/TEN-5, ADR-0020/0021); MEM-3 extraction
must treat `[REDACTED:*]` placeholders as opaque tokens; ruleset
precision/recall measurement lands with EVAL-2's labelled fixtures
(the recorded trigger for an ML pass behind the `Ruleset` seam); an
event at a since-deleted scope is unreviewable via the API (uniform
404) and awaits disposal. The scan is spawn_blocking CPU, O(payload
bytes); the MEM-1 load AC shape stays the asserted ack bound and
still passes with the seam in place — EVAL-6 owns percentile SLOs._

_MEM-3 notes (ADR-0022): extraction runs as a PGMQ-polling worker embedded
in the gateway (`SYNVEDA_EXTRACTOR`: `deterministic` by default, `claude` /
`vllm` / `off`), its stages Temporal-shaped — serializable activity I/O,
orchestration split from the polling transport — so the enterprise profile
(OPS-2) can host the same stages under the Temporal SDK later; the SDK
itself is deferred (git-distributed, ring/aws-lc licence graph — deny.toml
refuses both). Exactly-once is the archive-lock: `pgmq.archive` runs inside
the tenant write transaction before the record inserts, so redelivery and
racing consumers cannot duplicate records; a deliberately re-sent signal is
intentional reprocessing, with MEM-5's dedup as the semantic net, and
`pgmq.a_observe` is the dead-letter/completion record (no new table). The
pipeline's write re-decides `MemoryWrite` at the owner's *current* home
under current facts (a mover's memories follow the mover; a
since-quarantined owner is denied); denials archive and chain standalone
decision events under the new `system` actor kind (migration 0014 — MEM-6
sweeps and AUTH-4/5 sync inherit it). Extractor output re-enters the MEM-2
scanner before persisting — an LLM echoing a live-format secret writes the
placeholder — and sensitivity is floored at `internal` until AUTHZ-5
brings classification. Confidence is model-elicited and uncalibrated; the
provisional macro-precision target (≥0.8 on the labelled fixtures) and the
`--ignored` live-LLM measurement stand in until EVAL-2 owns the real
target, dashboard, and calibration. Forward obligations: MEM-4 wraps the
one commit seam with embed-or-fail; MEM-5 inserts dedup between extract
and commit; FLOW-1/2 replace the direct records insert with the
derived-channel commit; the <60s pipeline-lag SLO is evidenced by
`synveda_extraction_lag_seconds` (EVAL-6 owns SLO enforcement); and
LISTEN/NOTIFY replaces polling if idle load or measured lag ever matters
(ADR-0022's recorded upgrade)._

_MEM-4 notes (ADR-0023): embedding is a per-event stage between extract and
commit — outside any transaction (the MEM-3 rule: no transaction spans a
network call) — and the vectors land atomically with their records under
the archive-lock: `records::insert/update` now REQUIRE a `RecordEmbedding`
and write both tables in one data-modifying-CTE statement, and migration
0015's deferred constraint trigger makes an embedding-less record
impossible to commit even from raw SQL (the Mem0 failure mode is
structurally absent, not monitored for). Storage is the `record_embeddings`
sidecar (typmod-less `vector`, per-row model + dim, FK cascade, forced RLS,
no app-role DELETE) — never a column on the bitemporal pair; ANN indexing
belongs to CTX-1, which will find `vector` installed and populated. The
extractor-output re-scan moved ahead of the embed call so vectors are
computed over exactly the persisted redacted text — a secret absent from
content is also absent from vector space. The `Embedder` seam
(`SYNVEDA_EMBEDDER`: `deterministic` [default] | `tei`, no `off`) mirrors
the Extractor seam; TEI failures are `Dependency` → the existing
redelivery/dead-letter flow; the model identity is config-declared
(`SYNVEDA_EMBEDDER_MODEL`), not probed — gateway boot never couples to TEI.
Forward obligations: records from the MEM-3 window remain embedding-less
until the re-embed workflow (tech plan §1.3 model-change machinery) owns
the backfill; MEM-5's supersession work inherits the update-path
re-embed-on-rewrite obligation the upsert already satisfies mechanically;
per-tenant model pinning revisits the single config-declared model;
coalesced TEI batch calls (per-event failure attribution preserved) are the
recorded upgrade if embed round-trips ever dominate the <60s lag SLO._

_CTX-1 notes (ADR-0024): the retrieval engine's only entry takes a
mandatory `SearchFilter` — an empty scope set returns nothing without
touching an index; there is no unfiltered code path — and the scope set
is produced by `permitted_chain_scopes`: one PDP `MemoryRead` decision
per scope of the caller's placement chain (the Cedar schema's recorded
per-candidate-scope contract), suffix chains as resource chains, reusing
the full-chain assignment/binding rows the gateway already gathers.
The candidate universe is the chain (seed §4.4's composition contract);
scopes packs permit beyond it — bound subtrees, `standard`'s department
subtree — are CTX-5's recall surface, with the broader-universe
enumeration (and its batch-PDP perf problem) recorded there. Sensitivity
is a structural ceiling clamped below `restricted` until AUTHZ-5 makes
it a policy attribute. The lexical leg is a per-tenant Tantivy sidecar
(BM25 stats tenant-local; disposal = directory delete, a TEN-5
obligation; the directory must share the database's encryption-at-rest
story — recorded for TEN-4) converged by a watermark poller tailing the
bitemporal pair with a 10s overlap window; a stale sidecar is one-sided
by construction — fused candidates re-verify scope and sensitivity
against current Postgres truth at hydration, so lag can miss but never
resurface or leak. The dense leg is pgvector HNSW with iterative scans
(partial expression indexes per shipped dimension: 16 deterministic,
1024 BGE-M3; a custom-dim model adds its index and compile-checked query
variant as a reviewed diff). Query embedding is the caller's input — the
retrieval crate has no HTTP dependency, so "NO LLM calls on the read
path" is structural; no vector degrades to BM25-only (CTX-3's
embedder-down mode), no sidecar index degrades to dense-only. Stopwords
are stripped at the tokenizer (RRF is rank-based; an "and"-grade match
must not hand out ranks). Forward obligations: CTX-2 composes over
`permitted_chain_scopes` + `ScopeChainCache` (ADR-0016); CTX-3 owns the
inject seam, its audit event (one chained event, aggregated decisions,
record-id watermarks — ADR-0019 decision 4), and the query-embedding
call through the MEM-4 `Embedder` seam; the latency AC asserts the
MEDIAN with tails reported — EVAL-6 owns percentile SLOs; EVAL-4 owns
real quality targets over the fixture harness; LISTEN/NOTIFY replaces
the indexer's polling if measured lag matters (ADR-0022's recorded
upgrade, restated)._

_CTX-2 notes (ADR-0025): composition consumes the CTX-1 predicate —
`composition_plan` is one PDP walk producing the allowed chain scopes
with per-scope channel rules and the home-scope budget, and `compose`
is deterministic by construction (the valid-time instant is an explicit
input, every ordering is total, no clock reads or map-order effects).
`RecordKind` is the pre-FLOW-2 channel stand-in — pinned composes as
the published channel, derived is always marked unreviewed — and the
pack-carried `CompositionConfig` (budget default 1500;
`published-and-derived` | `published-only`) rides
`policy_packs.composition` (migration 0017) with CLI flags
(`synveda policy apply --composition-budget --composition-channels`)
and hot reload: bank mode exists and is testable two phases early, and
FLOW-2's AC will flip this same switch. The config never grants —
`MemoryRead` is decided per scope before composition — so an
unconfigured pack defaulting to the product config is not a widening.
Watermarks are BLAKE3 version hashes over (id, tx_from, content) —
recomputable content addresses FLOW-1's commit hashes supersede in
place; the block hash and record ids ride the rendered text inside the
budget. Conflict rules implement seed §4.4's resolution order (pinned
beats derived, nearer scope, newer valid time) over an exact
trimmed-content predicate; MEM-5 replaces the predicate (near-dup,
supersession) and reuses the exported comparator. Token accounting is
the `ceil(chars/4)` estimator seam — budgets bound estimated tokens;
EVAL-4 owns bias measurement and per-harness tokenizers are an adapter
concern. `synveda_tokens_per_inject` (the name FND-5 reserved, now
declared in the emitting crate) records on every compose, empty
included. Forward obligations: CTX-3 owns the inject seam — its single
chained audit event carries this watermark with aggregated decisions
(ADR-0019 decision 4) — plus the query-embedding call (MEM-4 seam) and
the hybrid→relevance wiring; CTX-5 extends the explicit instant to
transaction-time as-of; PRMT-2's context packs arrive as pinned
material; FLOW-1/2 landing replaces both stand-ins (recorded reversal
triggers in ADR-0025)._

_CTX-3 notes (ADR-0026): `POST /v1/inject` is the CTX-2 product path
with the hybrid engine wired between plan and compose — two tenant
transactions bracket the MEM-4 embed call (no transaction spans a
network call), and the caller's `budget_tokens` narrows the pack budget,
never widens it. Policy outcomes are results: a quarantined, unplaced,
or fully-denied caller receives the empty block (200, watermarked,
audited with every denial in the decisions) — the surface is not a
placement oracle. Dependency failures degrade instead of failing:
embed error/deadline (`SYNVEDA_INJECT_EMBED_TIMEOUT_MS`, default 100ms)
→ sparse-only, still ranked; retrieval error → unranked compose; both
marked in `X-Synveda-Degraded` and the body — only a store failure is a
5xx. Each inject chains ONE `context.injected` event in the compose
transaction (ADR-0019 decision 4): block hash, per-entry version-hash
watermarks, the instant, aggregated per-scope decisions (the plan now
returns them), degradations, and the task as a BLAKE3 hash — never
task text. Latency evidence (dev hardware, the house discipline):
p50 18.6ms / p99 24ms at the 50/s herd, stage split plan 4.5ms /
embed 10µs / search 3.3ms / compose 2.1ms / audit 5.8ms. STANDING
TRIGGERS with measured evidence: (1) the per-tenant chain-head lock
serializes append commits — the saturation probe measures a ~160/s
per-tenant inject ceiling (p50 ~190ms saturated, audit stage ~70ms) —
ADR-0019 option 2 (buffered read-path appender) is the recorded
upgrade if real deployments approach that per-tenant session-start
rate, and the probe prints the number every latency run; (2) the CTX-1
sidecar indexer sweeps EVERY active tenant per cycle — on the
long-lived dev database (2,868 leftover test tenants) a full cycle
takes minutes, so a just-admitted tenant's sparse leg lags that long
(the demo runs on a scratch database because of it) — ADR-0022/0024's
LISTEN/NOTIFY (or a dirty-tenant filter) now has dev-scale evidence
and becomes load-bearing at production tenant counts. Forward
obligations: ADPT-1 consumes the route (session-start + pre-compact,
narrowing the budget to the harness's remaining room); CTX-5 owns the
as-of parameter (transaction time + refs) and the recall surface;
EVAL-4 owns quality over the inject path; EVAL-6 owns percentile SLO
enforcement on production-shaped IO._

_ADPT-1 in progress (ADR-0027), landing in steps. Step 1 (2026-07-24):
the `adapters/claude-code` plugin — `SessionStart` → `/v1/inject` as
`additionalContext`, `Stop`/`PreCompact`/`SessionEnd` → `/v1/observe`
over a durable per-session cursor that advances only on a 2xx (MEM-1's
idempotency makes at-least-once effectively exact), every hook exiting 0
by construction. Note that "PreCompact → inject" from the feature text is
not implementable — `PreCompact` has no context-injection output — so
post-compaction re-injection is `SessionStart` with `source: "compact"`
(ADR-0027 decision 2). Step 2 (2026-07-24): the credential half. The
gateway grew the three surfaces decision 5 named — `cli_redirect_uri` +
`cli_state` on `/auth/login`, a one-time 60-second state-bound handoff
code on the callback, and `POST /auth/cli/exchange` / `POST
/auth/refresh` — all reusing AUTH-1 unchanged, so a CLI login is
PKCE-verified, tenant-resolved, and AUTH-2-provisioned exactly like a
browser one. `synveda login` and `synveda auth token --json` are the
CLI half; the adapter's `resolveBearer` now shells out to the latter and
holds no OAuth at all. Three readings the ADR left to implementation,
recorded here because they are behaviour: (1) "state-bound" is
CLI-minted — `synveda login` sends its own `cli_state`, which returns on
the loopback redirect (so the CLI can tell its own callback from any
other local process reaching an ephemeral port) and is required again at
redemption, so a leaked code alone redeems nothing; (2) a CLI-resolved
credential's own `gateway_url` overrides `.synveda/config.json` —
`synveda login` is what binds a machine to a gateway, and a
`gateway_url` in a checked-out repository must not be able to redirect
someone's bearer to a host of the repository's choosing (an explicit
`SYNVEDA_TOKEN` keeps the configured gateway); (3) `synveda auth logout`
joins the two subcommands decision 4 named, because a credential you
cannot revoke locally is not one this product should write. Every way a
login can fail now 302s back to the loopback with an `error`, rather than
leaving the terminal to time out. Deferrals: refresh chains no audit
event — it mints a bearer without provisioning or mutating anything, and
attributing one would mean verifying the returned token to learn its
tenant (ADR-0027's compliance note stands: no new action types in
ADPT-1); the handoff store is in-memory like AUTH-1's pending logins, so
single-replica until OPS-2; `SYNVEDA_TOKEN` remains as the explicit
override for CI and demos. Step 3 (2026-07-25) closed the AC — see the
entry below._

_ADPT-1 step 3 (2026-07-25): the AC itself. `demos/adpt-1-claude-code.sh`
splits at the person it is a claim about: the estate (a tenant, the
acme/eng/platform hierarchy, and team memory an operator wrote before
anyone arrives) is untimed, and what a developer does on a machine that
has never seen Synveda is timed — enable the prebuilt plugin, `synveda
login` against live Rauthy (driven headlessly, PoW and all), and a
session that receives its watermarked block and observes its turn back.
**1.5s of the 120s budget**, alice's first-ever login and AUTH-2
placement included. Then, untimed: `context.injected` and
`memory.observed` joined under one `claude-code:<id>` in a chain that
verifies, the access token renewing itself with no login, the observed
turn coming back as memory in the next session, and the
recorded-payload driver run against that same live gateway. Every hook
runs the command `hooks/hooks.json` registers, with recorded payloads on
stdin, so what is timed is the product.

The driver (decision 14) lives in `adapters/claude-code/src/driver.mts`
over `fixtures/`, and runs both against its own mock (in `npm test`) and
against a live gateway (the demo's last section): sixteen cases — dead
gateway, 401, degraded header, oversized tool result, replayed batch,
cursor resume after a failed flush, damaged transcript line, unreadable
payload — every one asserting exit 0 first. Three things it found, all
fixed here:

1. **An unreadable payload used to inject anyway.** `hooks.json` names
   the mode, so the entry point could dispatch without a payload — and
   would then compose for a session it could not name, in a project whose
   `.synveda/config.json` it could not read, silently overriding a
   `disabled: true` that was right there on disk. A hook with no input now
   does nothing. Only the live run could catch this: a mock is asked
   nothing when the client asks nothing.
2. **The observe envelope never carried the model.** Decision 8 names it,
   but only `SessionStart` payloads have one, and the flush hooks are
   `Stop`/`PreCompact`/`SessionEnd`. The spool now carries it across, and
   the harness version rides from the transcript entry — the two halves of
   decision 8's "harness and model" that were missing.
3. **A pre-emptive refresh the issuer refuses no longer costs a session
   its memory.** Rauthy will not honour a refresh token until the access
   token is inside its last minute — `issued_at + lifetime - 60s` — which
   is the same instant `REFRESH_SKEW_SECS` fires the refresh, so which of
   the two clocks is ahead decided whether a hook got a bearer. `synveda
   auth token` now falls back to the stored token whenever it is still
   valid, and fails only when nothing usable is left.

One factual correction to ADR-0027, recorded in the ADR: `PreCompact`
*does* carry a `transcript_path` (all four payloads are built from one
envelope — verified against 2.1.220 while recording the fixtures). The
spool fallback stays: the payload is another program's internal format,
and the spool holds the cursor regardless. Deferrals: the degraded-inject
case is mock-only, because producing a live degradation means stopping
TEI, which is CTX-3's demo; subagent (sidechain) turns still go
unobserved (decision 8); and the demo runs the deterministic embedder and
rule-based extractor, so the real-TEI path stays CTX-1/CTX-3's to prove._

_EVAL-1 (2026-07-25, ADR-0028): the feature arrived with no acceptance
criteria, so they were written first (SYNVEDA_FEATURES.md and
docs/backlog/EVAL-1.md) and the gate is the load-bearing half — a harness
that reports without failing is a dashboard. `crates/synveda-eval`
depends on no Synveda crate at all, and `check-crate-deps.mjs` holds it
to that empty set: an eval that can link the store can seed and read
around the PDP and would then report quality the product cannot deliver.
It speaks `/v1` with each actor's own bearer, seeds through
`/v1/observe` and waits for the real pipeline, and grades a single probe
per scenario. Scenarios are JSON under `evals/scenarios/`, so EVAL-2/4/5
add coverage by adding files.

Three things the first live runs settled, each recorded because it is a
limit rather than a preference: (1) **retrieval precision cannot be
honestly measured here.** The suite runs the deterministic hash embedder
(no model server, so the nightly stays cheap — decision 6), and its
geometry carries no meaning by construction (ADR-0023 decision 6), so the
dense leg ranks by nothing; a "keep the irrelevant record out" scenario
failed for exactly that reason and was rewritten to assert what the
deterministic path does guarantee — reachability under a task, inside the
requested budget. Precision with real embeddings is EVAL-4's, over live
TEI, as CTX-1's own quality suite already says. (2) **A block that
outspends its requested budget now fails every scenario**, not just the
ones that thought to ask: ADR-0026 decision 7's narrowing rule is an
invariant, so the runner checks it for free. (3) **`--update-baseline`
leaves headroom on cost ceilings** (half again) and none on quality
floors: a ceiling pinned to the last measurement fails on the next run's
jitter, and a gate that cries wolf nightly is a gate someone turns off.

Deferrals: the gate is nightly rather than per-PR, which is a trade —
a regression is caught within a day, and pull requests keep the
database-free CI that makes them fast; EVAL-4's composition scenarios are
the stated trigger to move it. `make ci` runs `eval-check`, which parses
the suite and the baseline with no stack at all, so a scenario that would
have measured nothing still fails on every pull request. The suite's four
scenarios exercise one tenant on a scratch database with dev-mode bearers
(ADR-0008); the OIDC path is AUTH-1's and ADPT-1's to prove, and both
do._

## Phase 2 — Governance (wk 6–10)

_Phase demo goal: promotion pipeline, lapse lifecycle, as-of inject, bank-mode switch._

- [x] [FLOW-1: Object store](FLOW-1.md) — done 2026-07-25, ADR-0030, AC tests: crates/synveda-vedaflow/tests/object_store.rs (both properties over a live Postgres — dedup across objects, trees, and commits, with the row count pinned to the number of *distinct* (kind, content) pairs, and the same bytes in two tenants proven to be two rows at one address; immutability under 8 genuinely concurrent writers on their own connections, asserting every landed commit is reachable from the head, the chain is root + every commit, and no commit row exists that the head cannot reach — the headline run reports the compare-and-swaps actually lost, and fails if none were, since a race nobody lost tested nothing; plus the append-only grants and triggers, a trigger-suppressing attacker caught by `verify`, fast-forward vs. force, and the foreign keys refusing a dangling tree, parent, or entry), crates/synveda-store/tests/rls.rs (the six VedaFlow tables join the adversarial suite and its completeness guard), crates/synveda-vedaflow (27 unit tests: the length-prefixed encodings, per-kind domain separation, parent order, the policy-snapshot canonical form, and the signer seam), demo: demos/flow-1-object-store.sh
- [x] [FLOW-2: Channels](FLOW-2.md) — done 2026-07-25, ADR-0031, AC test: crates/synveda-gateway/tests/channels.rs (the bank-mode switch over real refs, end to end on product surfaces: extracted memories land on `{scope}/memory/derived`, a curator publishes one through `POST /v1/channels/{scope}/publish` under the PDP, inject renders it unmarked while the rest still says unreviewed, a `published-only` pack becomes the tenant default, and the very next inject — same token, same session, no restart — composes the published record alone and cites the commit the curator made; plus the PDP gate, the read requirement that keeps a curator out of a teammate's personal scope, whole-request refusal of another scope's record, the `vedaflow.channel.published` event carrying ids and addresses but never content, and `GET /v1/channels`), crates/synveda-retrieval/tests/compose.rs (12 tests on real channels — including authored-but-unpublished material failing bank mode, and an edit demoting a published record to unreviewed), crates/synveda-gateway/tests/extraction.rs (the derived-channel commit lands in the pipeline's own write transaction), crates/synveda-policy/tests/{roles,packs,pdp}.rs (the channel plane joins the role×action matrix at pack `@6`), demo: demos/flow-2-channels.sh
- [x] [FLOW-3: Proposals & approval matrix](FLOW-3.md) — done 2026-07-25, ADR-0032, AC tests: crates/synveda-policy/tests/approvals.rs (the **full matrix golden**: 3 packs × 5 asset kinds × 4 sensitivities × 5 scope kinds = 300 cells rendered canonically against tests/golden/approval-matrix.txt, so a wrong requirement and a wrong *absence* of one both fail and the diff names the cell; plus the packs proven to actually differ where tech plan §2.4 says they do, the floor holding under a pack written specifically to author it away, and an unsatisfiable matrix refused at install rather than discovered at review), crates/synveda-gateway/tests/proposals.rs (the **team→published promotion with 1 curator** over the product surfaces — a contributor opens, the response states what the pack requires and that it is unmet, the contributor cannot run the effect, the curator reads the content and approves, and the publication is a merge commit whose second parent is the proposal; and **restricted → compliance + dual approval**, refused on the direct route by name, refused again for a principal holding *both* roles because two distinct approvers means two people, then carried by curator + compliance — with the deciding compliance vote unable to publish, which is the case that decided against auto-publishing; plus approvals binding bytes, a curator file adding a named approver without granting them anything, rejection/withdrawal, and the uniform 404), crates/synveda-store/tests/rls.rs (the two new tables join the adversarial suite and its completeness guard: a forged approval on another tenant's proposal, the append-only review log, and the one permitted open → closed transition), crates/synveda-types (18 unit tests on the counting rule) and crates/synveda-vedaflow (curator-file parsing, the one-wildcard glob, and approvals that never carry to another commit), demo: demos/flow-3-proposals.sh
- [x] [FLOW-4: Auto-promotion rules](FLOW-4.md) — done 2026-07-25, ADR-0033, AC tests: crates/synveda-gateway/tests/promotion.rs (the **soak**, over real product surfaces with a real signal — nothing writes a usage counter, every recall is an actual `POST /v1/inject` whose `context.injected` event the engine folds out of the audit chain: two recalls open nothing, the third crosses the rule's threshold and a proposal appears with nobody deciding to, targeting the scope the material already sits on, proposed under the *owner's* identity rather than a system principal; and the **evidence**, which is checked rather than displayed — `evidence_is_checkable_against_the_chain` re-derives the recall and distinct-member counts from the hash-chained events in the `[from_seq, to_seq]` range the evidence names, without consulting the projection that produced it; plus a ten-round soak that never proposes the same bytes twice, a rejection binding those bytes and an edit freeing them, the projection discarded and refolded from seq 1 to the identical counts, a quarantined owner proposing nothing, an unconfigured pack promoting nothing while still sweeping usage, and — pinning the fact ADR-0033 decision 8 rests on — twenty injects by a teammate adding neither a member nor a recall to someone else's personal record), crates/synveda-store/tests/rls.rs (the two new tables join the adversarial suite and its completeness guard: a forged usage row, a rewound watermark that would refold a victim's chain and double their evidence, and the DELETE grant that makes the rebuild an operation rather than an aspiration), crates/synveda-types (16 unit tests on the rule vocabulary: every threshold load-bearing, the sensitivity ceiling, and refusal at install of a rule that asks nothing or could never fire), demo: demos/flow-4-auto-promotion.sh (on a scratch database, the gateway's own background loop — not a test harness — crossing the threshold, then the evidence re-derived from the chain in SQL, idempotence under a continuing soak, a curator refused because publishing needs MemoryRead on material nobody else can read, and the owner publishing her own through the ordinary FLOW-3 path into a merge commit whose second parent is the proposal a rule opened)
- [x] [FLOW-5: Cross-scope promotion](FLOW-5.md) — done 2026-07-25, ADR-0034, AC test: crates/synveda-gateway/tests/cross_scope.rs (the **two-level climb with distinct approver sets**, asserted from the reader's side because that is what makes it a promotion rather than a row: a platform-team runbook reaches Engineering and then ACME, and between the hops a payments-team member — who could not read platform before and still cannot — starts receiving it in her own \`POST /v1/inject\`, sectioned under the department and unmarked, while a member of *another department* still gets nothing until the second hop lands; each level refuses the level below by name, because bindings inherit downward and never up, and each publication takes what the pack asks at that scope kind — one curator at a team, a curator **and** a steward at a department or the org, with the steward unable to run the effect since steward reads no content in any pack; the **denial audited with reason** is the org's rejection between the hops, carrying its mandatory reason and both scopes, with the chain verifying over all of it; plus the direction rule refusing sideways and downward by name, the disclosure rule — a team curator cannot climb a teammate's personal memory and the owner can climb her own, then the curator reviews content she cannot read at its source — and the two senses in which a scope holds material, including the second hop proposed by a department that holds the record only by publishing it, and an edit that takes it out of both at once), crates/synveda-retrieval/tests/compose.rs (\`an_ancestors_published_channel_admits_a_record_living_below_it\`: the read-path half in isolation — the same record composes nothing when only a sibling team published it and composes as reviewed at the *department's* gradient position and section once the department does, surviving bank mode, with the record never moving), crates/synveda-gateway/tests/proposals.rs + crates/synveda-retrieval/tests/compose.rs (FLOW-2/3/4's suites unchanged and green: when source and target are the same scope both composition substitutions are identities), demo: demos/flow-5-cross-scope.sh (the runbook climbing \`acme/eng/platform → acme/eng → acme\` over the HTTP surfaces, with the two readers' injects before and after each hop, and the trail printed as a table whose from/to columns are the climb)
- [x] [FLOW-6: CLI review flow](FLOW-6.md) — done 2026-07-25, ADR-0035, AC demo: demos/flow-6-cli-review.sh (**the whole review from a terminal**, and shaped so the claim cannot be fudged: from the moment a proposal exists, every governed act is `synveda proposal ...` and `DATABASE_URL` is *unset* for all of it — cora lists her queue, reads one in full with its requirement and its effect, approves, and runs that effect; the runbook is then edited and re-proposed and the review renders it as an `update` with the published version beside it and one line marked out of three; `synveda proposal review` walks the queue oldest-first and takes three verdicts in one command — skip, reject with the empty reason refused and re-asked, approve — while the same command over `/dev/null` casts nothing at all; then the refusals a reviewer meets in the product's own words: a contributor denied `ProposalReview`, a team curator's tenant-wide listing denied with `--scope` named, a `restricted` record that one curator cannot carry, compliance reading content it holds no `MemoryRead` for and then unable to publish what it just decided; and the trail, where all twelve acts carry `actor_kind=subject` under the reviewer's own subject with **not one break-glass row**, chain verifying), AC tests: crates/synveda-gateway/tests/review_surface.rs (what the CLI cannot invent: `add`/`update`/`none` read off the *target's* tree rather than the record's row, the old side being the object the tree names now and the new side the object the **proposal** names — asserted specifically for a record edited under its own review, where the two differ; a `compliance` reviewer proven to compose nothing from `POST /v1/inject` at that scope and shown both sides of the change anyway, which is ADR-0035 decision 8 as a test rather than a paragraph; both scope paths on a climb through the listing and the detail; and a climb's baseline being the scope it would land on — the department `add`s what the team has already published, and only a second climb of the same bytes is the no-op), crates/synveda-cli (33 unit tests: the LCS line diff — hunk headers, context merging, identical texts producing nothing — the field-wise renderer refusing to show a sensitivity change as no change, the prompt whose EOF casts nothing and whose rejection is re-asked until it says why, the tenant-wide denial that names `--scope`, and refusals rendered from the shared taxonomy), crates/synveda-vedaflow (the batched object read the detail route uses so its statement count does not grow with the member set)
- [x] [FLOW-7: Rollback & pinning](FLOW-7.md) — done 2026-07-25, ADR-0036, AC demo: demos/flow-7-rollback.sh (**a bad instruction live in a fleet to not one agent receiving it in 0.2s of the 60s budget**, and the clock is on the incident rather than the estate: the line is authored at a team and climbs to Engineering through review — a curator *and* a steward, because that is how bad content actually becomes trusted — two engineers in different teams and a headless agent all receive it unmarked, and then one operator runs `synveda channel history` and `synveda channel rollback` and the same three agents' next sessions are different, with no second approval, no restart, and nothing to wait out; the two readers deliberately do **not** get the same answer, which is the honest half — the payments engineer loses the line entirely because the record lives off her chain, the platform engineer gets it back as `[unreviewed]`, because what a rewind removes is trust rather than content; then the trail (one `vedaflow.channel.rolled_back` carrying both commits, the record that left, the reason, and no record text, chain verifying), the refusals in the product's own words — a proposal commit *reachable from the head* and refused by name, a rewind forward refused with the way back named, a reader denied `channel.rollback`, and the steward who approved the publication denied `memory.read` — and the pin: the team holds its readers, publishes anyway, the block cites the frozen commit with `pinned=true`, a rewind under it is refused because readers would not heal, releasing catches the reader up, and a rewind at the *source* leaves the department's publication standing), AC tests: crates/synveda-gateway/tests/rollback.rs (8 tests over the product surfaces: the AC end to end with both readers asserted separately; **a proposal commit is reachable and still refused** — including the case that nearly slipped through, a channel's *first* publication, where the proposal sits at ordinal 0 and FLOW-3's own AC pins it there; a rewind that never advances and never guesses what it is leaving (`from == to`, a stale `from` as `Conflict`, and the forward move refused); log channels and asset kinds with no read action refused by name; the two decisions a rewind takes, with the privacy floor keeping a curator out of a teammate's personal channel through `MemoryRead` and no clause about personal scopes anywhere; the pin holding what readers compose while the channel keeps moving, with the watermark, the listing, the refusal, and both audit events; a pin refused at a state the channel never held; and **a climbed record surviving its source's rewind**, which discharges ADR-0034 reversal trigger (c) with the answer recorded rather than assumed), crates/synveda-store/tests/rls.rs (`only_pins_can_be_deleted_and_only_in_their_own_tenant`: the restrictive policy makes an unqualified channel-ref delete a legal statement matching nothing, a cross-tenant unpin matches nothing, a pin releases, and the trigger raises for anyone bypassing RLS), crates/synveda-vedaflow/tests/object_store.rs (`a_side_parent_is_reachable_and_is_not_on_the_first_parent_line`: the substrate distinction the whole feature rests on, asserted against `is_ancestor` in the same test), crates/synveda-vedaflow + crates/synveda-cli (unit tests: pin names that can never parse as channels and always match migration 0021's `pin/%`, set-channel-only operations, and the CLI's channel-name splitting)
- [x] [AUTHZ-4: Lapses (controlled relaxation)](AUTHZ-4.md) — done 2026-07-26, ADR-0037, AC demo: demos/authz-4-lapses.sh (**a cross-team read the pack forbids, opened by two stewards and closed by the clock**, asserted from the reader's side throughout: bea on payments receives nothing of platform's, then receives its published runbook under a section the block marks `[lapse]`, then receives nothing again ten seconds later with **nobody acting** — no revocation, no restart, no operator, because the read path's own predicate is what closes the window; the unpublished draft beside it never travels, which is the honest half — a lapse discloses what the target *stands behind*, not its corpus; then the trail, where the proposal, both approvals, the grant with its window, four of bea's injects and one `policy.lapse.expired` under `actor_kind=system` read in order on one verifying chain with no record text anywhere; the refusals in the product's own words — a personal scope denied by the PDP before the surface sees it, a target the grantee already composes, an action outside the closed vocabulary refused at the wire, a 45-day window against regulated-strict's 30-day ceiling naming both numbers, and the reader herself denied when she asks for the access she wants; and revocation, where a security-reviewer ends a standing grant he could never have opened, which is the whole reason `LapseGrant` and `LapseRevoke` are two actions), AC tests: crates/synveda-gateway/tests/lapses.rs (5 tests over the product surfaces: the AC end to end with a **real** wall-clock expiry — no injected clock, because a duration is seconds with no minimum precisely so this can be demonstrated rather than asserted; one steward refused at the effect with the outstanding requirement named; published-only disclosure with an unreviewed sibling record proven absent; a security-reviewer who revokes but cannot propose, whose revoked grant then gets no expiry event because its ending is already on the chain; the four surface refusals, including the privacy floor at both layers — the PDP stops a steward reaching another principal's personal scope, and the surface stops the owner themselves; and the listing keeping ended grants, because "who could read this scope's material in March" is the question it exists for), crates/synveda-policy/tests/lapses.rs (8 tests on the permit: the decision flipping on one row and back; a grant reaching its target and neither its neighbours nor its subtree; no grant opening a personal scope under any pack; a scope-shaped grantee reaching everyone under it and nobody else; a zero-ceiling pack ending standing grants on the very next request; **a forbid still beating the base layer's first permit** — quarantine, and a service identity that cannot be widened past its anchor; the closed vocabulary refusing to turn a read grant into a write; and the plan and the permit sharing one containment rule), crates/synveda-store/tests/rls.rs (`a_grant_cannot_be_forged_resurrected_or_extended` — a forged tenant, an un-revocation, and the attack the table exists for: an `expires_at` pushed forward, which would make a 30-day grant permanent while the proposal, the approvals and the chain all still said 30 days; plus `an_expiry_can_only_be_chained_once`), crates/synveda-types (16 unit tests on the vocabulary and the ceiling) and crates/synveda-vedaflow (5 on the reviewed terms' canonical form, where every term is in the address so an approval can never carry to different ones)
- [x] [AUTHZ-5: ABAC conditions](AUTHZ-5.md) — done 2026-07-26, ADR-0038, AC test: crates/synveda-gateway/tests/leak.rs (**the tier is earned, then never reaches a reader without the signature that earned it**: the record becomes `restricted` through the only path that can mint it — a classification proposal the invariant floor priced at `compliance` plus two distinct approvers, refused by name when one curator tries to run it — is published through that same floor, and is then asked back under 40-plus generated query variants (every word, every adjacent pair, reversed and upper-cased, plus the taskless session start) by four readers including a steward above the scope and *the record's own author*, with the working-tier corpus proven to travel so the sweep is not vacuous; then a two-steward lapse that declared only the working tier changes nothing about the top one, a lapse that declares `restricted` is refused with both stewards' approvals in hand until compliance signs — which is the AC's "compliance-granted permission", reached by the floor rather than by any rule this feature wrote — the reader receives it marked twice (`[lapse]` section, `[restricted]` line), a different department's reader never does, and the window closes with nobody acting; plus `confidential_material_takes_an_explicit_grant_and_a_binding_is_one`, where membership alone does not reach the tier, a content-role binding does on the very next request, and a caller asking for `internal` gets less rather than more), demo: demos/authz-5-abac.sh (the same arc over HTTP with the trail printed: `memory.classified` carrying `internal -> restricted`, the grant carrying the ceiling its approvers signed for, both expiries under `actor_kind=system`, chain verifying over 36 events; and the refusals in the product's own words — a classify proposal with no tier, a publication naming one it would not move, a classification pushed through the publish route, and the reader asking for her own access), AC tests: crates/synveda-policy/tests/sensitivity.rs (9 tests on the per-tier decision: membership reads the working tiers and stops, an explicit binding or one's own home reaches `confidential`, `restricted` denied under every pack at every scope *including the reader's own* to a principal holding every role, only a lapse that declared the tier lifts it and only at the target it named, `open-collaboration` reading the org at `confidential` and never above, quarantine and service-identity confinement still beating the base layer's permit, and a read decided without a tier refused rather than defaulted), crates/synveda-retrieval (the per-scope predicate: one scope's tier set never leaking into another's on the same chain, the plan carrying what the walk decided, the engine returning exactly the pairs it was handed — including the top tier when the plan names it, which is what makes the refusal policy's rather than a constant's), crates/synveda-store/tests/rls.rs (`a_grant_cannot_be_forged_resurrected_or_extended` gains the tier attack: a `max_sensitivity` raised after approval, which would widen an `internal` grant past what its approvers signed while the proposal, the approvals and the chain all still said `internal`), crates/synveda-gateway/tests/extraction.rs (`an_extractor_can_never_mint_the_top_tier`: a mock Claude proposing `restricted` persists `confidential`), and the latency AC re-measured with four times the decisions — p50 13.44ms, plan stage 4.42ms against CTX-3's 4.5ms at a quarter of the work
- [x] [MEM-5: Always-on dedup & conflict detection](MEM-5.md) — done 2026-07-26, ADR-0039, AC test: crates/synveda-gateway/tests/dedup.rs (**both halves of the AC over the real product path**, never a seeded row: alice states a fact, restates it — one record, `provenance.merged.count=1`, where an ADD-only store made two — then states its replacement, and the very next `POST /v1/inject` carries `ledger-live` and *not* `ledger-archive`, while the composition engine's own valid-time query asked at Monday still answers `ledger-archive` and `records_versions` still holds the open-ended version the record had before the close; plus the governance boundary — a *published* fact contradicted is refused, counted in `refused_published`, and left composing for a human — the late-arrival case, where an observation that reaches the pipeline after the fact that replaced it lands with its window already shut rather than being dropped, and a pack that turns the feature off and gets the pre-MEM-5 store back exactly), eval axis: `knowledge_update` in evals/baseline.json (floor 1.0, measured 1.000 with the gate held; evals/scenarios/05,06 in LongMemEval's knowledge-update shape — and with dedup off, the state the AC test's last case puts the product in and the state it was in before this feature, an ADD-only store composes the superseded fact beside its replacement, both scenarios' `must_not_contain` fails, and the axis reads 0.0), unit suites: crates/synveda-ingest/src/dedup.rs (14 tests: the changed-value update supersedes, the changed-*subject* one does not, two facts about one subject both survive, a paraphrase only cosine can see, the published refusal, the valid-time tie, and every mode bound), crates/synveda-store/src/dedup.rs (6 tests: the frame/value split, determinism of the signature across processes, the canonical updates sharing a band and unrelated statements not, and the full-rewording blind spot recorded rather than assumed away), crates/synveda-store/tests/rls.rs (`record_signatures` and `record_supersessions` join the adversarial suite and its completeness guard — including the one that matters here: the LSH nominator is a similarity oracle, and asked for another tenant's corpus with that corpus's own bands and placement it must answer nothing), demo: demos/mem-5-dedup.sh
- [x] [MEM-6: Decay, TTL & staleness](MEM-6.md) — done 2026-07-26, ADR-0040, AC test: crates/synveda-gateway/tests/retention.rs (**both halves of the AC over the real product path**, never a seeded record: alice's ninety-day-old session summary composes, a steward applies a retention schedule, and the **very next inject** stops carrying it with nobody acting, nothing restarted and no sweep having run — while the record is still in the store, because nothing was ever stamped on it and enforcement is the read path's; then the sweep expires it and chains `memory.expired` under `actor_kind=system` naming the horizon, the class, the id and the age in whole days, with no record content anywhere in the payload and the chain verifying; plus the bitemporal half, where the expired record's version is still in `records_versions` — the temporal delete FND-4 built, which is what keeps "what did the agent know in April" answerable; pinned material of the same age exempt from the read cut *and* the sweep, by seed §4.2 rather than by a pack field; the **second horizon**, where the destruction stage takes the history the expiry deliberately left and the as-of question that had an answer stops having one, audited as `memory.disposed`; the observe staging plane disposed of on its own horizon — the obligation ADR-0020 and ADR-0021 both parked here, with the extracted records untouched; and a pack that turns the feature off getting the pre-MEM-6 product back exactly, horizons set and ignored), read-path tests: crates/synveda-retrieval/tests/compose.rs (the per-scope cut removing one scope's own material and nothing else, a horizon never reaching pinned material, staleness ageing a *ranked* record out of its place while unranked order stays recency, and the two clocks proven distinct — a MEM-5 restatement refreshes staleness without moving the retention clock), crates/synveda-store/tests/rls.rs (the destruction path joins the adversarial suite: DELETE on `records_history` refused without the named flag, refused across tenants *with* it — the flag opens the trigger, never the isolation policy — and an UPDATE still refused under it, because "destroyed" and "altered" are different words; plus the staging plane's new grants, where the FK forces markers before payloads and a marker delete outside a declared disposal raises), crates/synveda-types (9 unit tests on the vocabulary: the product config expiring nothing, `off` ignoring horizons rather than lacking them, a cutoff as the horizon subtracted from the instant asked at, every class answered and none inventable, and staleness halving at the half-life and never exceeding fresh), demo: demos/mem-6-retention.sh (on a scratch database, the gateway's own sweep — not a test harness — with the schedule applied over the CLI, the trail printed, and the chain verifying)
- [x] [CTX-4: Tiered injection / progressive disclosure](CTX-4.md) — done 2026-07-27, ADR-0041, AC test: crates/synveda-gateway/tests/tiered.rs (**both halves of the AC over the real product surfaces**, never a block a harness composed: alice works six sessions, a budget too small for the corpus is applied, and `POST /v1/inject` is asked twice — once under `index_tier: off`, which is the product exactly as it behaved before CTX-4, and once with it on — so the measurement is a *difference* rather than a number: at a 240-token budget, records named 1 → 2, block tokens 80 → 217, the index tier costing 122 tokens or 56% of the block, which is the AC's "token cost of index tier measured" and what discharges ADR-0025's index-overhead reversal trigger — read honestly, and recorded in ADR-0041 rather than rounded off: the tier is *expensive* at a tight budget and under 8% at the seed §4.4 default, because its cost is a flat ~90 tokens per named record against whatever the body would have been; then the navigation, asserted as a round trip with nothing carried between the two calls but the id the block printed — the handle goes back to `POST /v1/recall` and the body comes out in full with its channel, provenance and validity labels; and the decision the whole surface rests on, `a_handle_stops_resolving_when_the_decision_behind_it_changes`, where the same id in the same session stops being served once the pack behind it is replaced, with nothing revoked because there is nothing to revoke — a handle is a name rather than a capability; plus the uniform refusal, where a nonexistent id, another tenant's id and a denied id are indistinguishable in the response *and* on the chain, so a recall never becomes an oracle for "does this record exist", and the 32-id cap), read-path tests: crates/synveda-retrieval/tests/compose.rs (6 tests on the tier itself: material that does not fit named rather than dropped, carrying its handle and joining the watermark because a disclosure the watermark does not cover is one nobody can audit; a short record **never** demoted, because naming it would spend budget to say less — the one rule that keeps a mechanism built for assets that do not exist yet from making today's corpus worse; `off` restoring the pre-CTX-4 bytes exactly, and a corpus with nothing to demote composing byte-identically either way; an index entry keeping every trust marker through the elision, `[confidential]` and `[unreviewed]` alike; the tier never naming what the plan excluded, not even by id; and CTX-2's byte-identical determinism AC re-asserted *while* the tier demotes, because a determinism proof over a path the feature does not take proves nothing about the feature), architecture: `retrieval::admit` extracted so composition and recall share one admission decision — the plan's tiers, channels, horizons and conflict rules — rather than recall re-deciding what a block already decided (seed §2.2), with the 286-test workspace suite green across the refactor, demo: demos/ctx-4-tiered-injection.sh (on a scratch database, six records through the real observe → extract path, the tier applied to the running gateway between two injects, `synveda recall` run with `DATABASE_URL` **unset** so the body can only have come through the gateway under the PDP, the same handle refused after the policy changes, and the trail printed — `context.injected` naming each entry's tier, `context.recalled` carrying counts but never the refused ids, no record content in either payload, chain verifying over 25 events)
- [x] [CTX-5: recall API + MCP tool](CTX-5.md) — done 2026-07-27, ADR-0042, AC tests: crates/synveda-gateway/tests/recall.rs (**the widening asserted from the reader's side**: one corpus, one identity, and the real `standard` pack — `POST /v1/inject` returns nothing of the sibling team's material because ADR-0024 fixed its universe at the chain, and `POST /v1/recall` returns it, which is the department permit `standard` has carried since AUTHZ-2 and nothing in the product could exercise; `regulated-strict` over the identical corpus still refuses it, so the widening is more scopes *asked* rather than more allowed; a `viewer` binding widens the universe on the very next call; **as-of** returns what was known then and the correction now, a bare instant sweeps material the live corpus has since retired that a query cannot rank, a withdrawn binding is not carried back by naming an instant at which it stood, and a reclassification reaches its own history so the AUTHZ-5 leak suite cannot be walked around with a timestamp; the query form is not an existence oracle; and `--ignored` `the_plan_stage_fits_the_budget_adr_0029_derived` asserts the plan stage inside the **15ms** ADR-0029 pre-registered for it — 13.2ms at the shipped cap, from 378ms before the batch materialisation), adapters/claude-code/src/mcp.test.mts (the protocol frame by frame: handshake, exactly one tool, the failure posture inverted from the hooks — an agent that asked is *told*), demo: demos/ctx-5-recall.sh (inject vs recall over one pack, the as-of pair, the tier boundary a withdrawn grant leaves behind, and a real MCP client speaking JSON-RPC over stdio to the real server against the live gateway)
- [x] [GRPH-1: Multi-graph schema](GRPH-1.md) — done 2026-07-28, ADR-0043, migration 0026, AC tests: crates/synveda-store/tests/graph.rs (**all three clauses**: an edge written through the store API read back through the traversal API with kind, endpoints and validity intact; a supersession closing the prior window with both versions readable as-of *and* the traversal answering differently at the two instants; and the plan guard, which explains **the statements the crate ships** — found in `src/graph.rs` by the `-- shipped-traversal:` marker each carries, so it cannot drift from a copy — and fails on a sequential scan over either edge table, all four planning as index scans on both legs; plus the cross-graph refusal on the read *and* write side, undirected 2-hop reach with the third ring excluded, the seed bound, vertex convergence, and the no-op supersession that inserts nothing; `--ignored` `traversal_medians_on_the_shipped_schema` re-takes ADR-0029 G1 on the built schema under RLS at 1M edges — 1-hop median 1.17ms, 2-hop 23.4ms against the 50ms median threshold, tails reported against the 150ms expansion slice), crates/synveda-store/tests/rls.rs (the three tables in the adversarial suite since the migration), crates/synveda-types/tests/serde_roundtrip.rs (`Graph` and `Depth` refuse anything outside their vocabulary, integers included), demo: demos/grph-1-graph-schema.sh
- [x] [GRPH-2: Graph-linking stage](GRPH-2.md) — done 2026-07-28, ADR-0044, migration 0027, AC tests: crates/synveda-ingest/tests/entity_resolution.rs (**the dedup precision half**, pairwise over the labelled fixture set: 0.973 against a 0.95 provisional target, recall 0.837 reported and not asserted, with the set carrying its own ceiling — `Paris` is two different things sharing one name — and a second test pinning the false merges to *exactly* that pair, so an over-eager rule fails before the threshold's slack absorbs it; plus the refusals a redaction placeholder, a pronoun and an over-long key all get, and the confidence tier that reports what normalisation did), crates/synveda-gateway/tests/graph_linking.rs (**the other half over the real product path**, never a seeded row: two sessions a month apart spell one company two ways and converge on **one** vertex — resolution against nodes that already exist, done by the unique constraint rather than by a lookup — each record hangs off its own session in the episode graph, a 2-hop `expand` walks record → name → the other record, and the graph's work rides the group's existing `memory.extracted` event because GRPH-2 adds no action type; plus the orphan rate counted per graph and asserted on the metric a dashboard reads, a real secret taken through quarantine and release with neither the key nor the placeholder reaching an unscoped vertex, and the provenance projection proved to be a projection — `graph_edges` holds no `supersedes` row and the provenance graph holds no vertex), crates/synveda-store/tests/graph.rs (`asserting_a_claim_that_already_holds_writes_nothing`: migration 0027's partial unique index makes re-assertion a no-op with no second row and no history row, while a different relation still lands and a *superseded* one may be asserted again — the predicate is partial so supersession's second half stays legal), crates/synveda-ingest/src/linking.rs (9 unit tests on the resolver's rules), demo: demos/grph-2-graph-linking.sh
- [x] [GRPH-4: AGE performance spike / graph fallback assessment](GRPH-4.md) — done 2026-07-25, report: docs/spikes/grph-4-age-traversal.md, criteria + verdict: ADR-0029, harness: crates/synveda-store/tests/graph_spike.rs (`--ignored`), demo: demos/grph-4-graph-spike.sh
- [x] [AUD-2: Audit query & auditor role surface](AUD-2.md) — done 2026-07-28, ADR-0045, migration 0028, AC test: crates/synveda-gateway/tests/audit_query.rs (**both questions over the real product path**, and the point is that nothing seeds an audit row: disclosures exist because alice and bob called `POST /v1/inject` and the chain recorded what they got. **Q1** — one `GET /v1/audit/disclosures` names exactly the readers the chain records being *served* the record, with the version, channel, tier and seq each of them actually got, and never the payments reader. **Q2** — one `GET /v1/audit/knowledge` folds to one row per record with the version last delivered and the number of occasions behind it, and the AC's "uses bitemporal + refs" is asserted rather than described: every id in the answer is resolved through `records::as_of` at the instant asked at, so the audit answer and the corpus agree; a companion test pins the instant as load-bearing by asking the same call before her first session and getting nothing. Plus the two lists proven separate with the reason carried *in the response body*; the refusals — a subtree-bound auditor denied on all four routes while the same role held tenant-wide passes, and the subject of the answers denied herself; no record content in any response, swept for whole bodies *and* distinctive fragments; the uniform empty answer that keeps the surface from being an existence oracle; a truncated page reporting itself with a cursor that advances; and dana's own query appearing in the next query's results. The suite runs under the real `regulated-strict` default and installs no permissive pack — a blanket pack would grant `AuditRead` to everyone and make every refusal in the file vacuous), crates/synveda-policy/tests/roles.rs (`the_audit_plane_admits_exactly_the_read_only_admin_roles_and_only_tenant_wide`: all 8 roles × 3 packs, allowed tenant-wide for steward/org-admin/auditor and denied for the same role bound at a subtree), crates/synveda-policy/tests/service_scope.rs (`AuditRead` joins the tenant-plane denial list, so no service identity reads the trail however it is bound), crates/synveda-audit (11 unit tests: the fold's last-wins-by-seq, absence reported as absence across three payload generations, the action taxonomy's uniqueness and column bounds), crates/synveda-gateway/src/audit_query.rs (4 unit tests: the vocabulary round-trips, a typo is refused, a limit over the cap is refused rather than trimmed), demo: demos/aud-2-audit-query.sh (the auditor's whole half with **DATABASE_URL unset**, so every answer can only have come through the gateway under the PDP: both questions, the two lists with the break-glass bootstrap bind and the two governed ones side by side in the authority half, the three refusals in the product's own words, a content sweep returning zero, dana's seven own audit reads on the chain she is reading, and `valid=true over 30 events`)
- [ ] [EVAL-2: Extraction quality suite](EVAL-2.md)
- [ ] [EVAL-4: Retrieval & injection quality](EVAL-4.md)
- [ ] [EVAL-5: Security evals](EVAL-5.md)
- [ ] [PRMT-1: Prompt templates as assets](PRMT-1.md)
- [ ] [PRMT-2: Context packs](PRMT-2.md)

_FLOW-4 (2026-07-25, ADR-0033) landed with a finding that constrains
FLOW-5 and PRMT-1 rather than FLOW-4. The tech plan's illustrative rule —
"a procedure recalled >N times across ≥3 team members" — **cannot fire on
anything the write path produces**, and not by an oversight: a derived
record lands at its owner's personal node, a service identity is placed
"like a user" (ADR-0018 decision 2, a `ScopeKind::User` leaf under its
anchor), and composition never leaves the caller's own chain —
`permitted_chain_scopes` decides `MemoryRead` once per *chain node*, so
another member's personal scope is never a candidate the PDP then
rejects. A distinct-member count over anything `observe` → extraction
writes is therefore identically 1, at two independent layers.

What ships is the engine plus the rules that *can* fire — the
`min_distinct_members: 1` case, which is a product case rather than a
weakened one: under bank mode a scope's derived material does not compose
at all, so promoting a member's own well-used memory to their own
published channel is the difference between a record existing and a
record counting. Multi-member rules need material at a shared scope,
which arrives with the first authoring path (PRMT-1, context packs) or
with FLOW-5's climb; the threshold is already a number in a pack, so
neither needs an engine change.

Two bugs the acceptance suite found are worth naming because both were
concurrency, not logic: two overlapping sweeps read the same watermark
and folded the same events twice, inflating every count they produced
(fixed by a `for update` on the watermark row — the AUD-1 chain-head
pattern); and an idle tenant was paying a write and a row lock per pass
just to discover it had nothing to do (fixed by an unlocked head-vs-
watermark check before either). The second matters on the shared dev
database, where a pass visits thousands of leftover test tenants — which
is also why the demo runs on a scratch database, the discipline EVAL-1
recorded for the same reason._

_FLOW-5 (2026-07-25, ADR-0034): cross-scope promotion — the climb is an
**ordinary proposal whose target is a strict ancestor of its source**.
Same table, same matrix resolved at the target and only there, same
lifecycle, same audit actions; two things are added and nothing else.
Opening a climb takes a second Cedar decision, `MemoryRead` at the
**source**, and a climb's members must be material the source scope holds
— records living there, or records its published channel names at their
current address. That second sense is what lets a department propose
onward what a team climbed into it, and it is why the ladder the tech
plan draws (team → department → org) falls out without being enforced:
hop two's source held the material because hop one published it there.
The feature added **no migration, no Cedar action, no audit action, and
no table** — `source_scope_id` has been stored and immutable since
migration 0019, which is the strongest available evidence that ADR-0032
decision 17 put the boundary where it said it did.

**The disclosure question ADR-0032 deferred has one answer: the
proposer's read at the source, taken once, recorded under their name.**
A reviewer at the target sees content they may not be able to read at its
source, and that is the disclosure the proposer made rather than a leak.
The obvious alternative — make every reviewer hold `MemoryRead` at the
source — was refused for two independent reasons, either decisive:
`compliance` holds no content read in any pack, so the invariant floor's
own role could never review a `restricted` climb; and nobody but the
owner reads a personal scope, so a user's own memory could never climb to
their team. The privacy floor then does the rest with no clause about
personal scopes anywhere in the promotion code: Bob cannot climb Alice's
note because no pack permits him `MemoryRead` there, and Alice can
because the self permit does.

**The read path is where the feature becomes real, and it is the one
place FLOW-5 changed behaviour rather than adding a surface.** A scope's
published channel may now name a record that lives *below* it, so
composition fetches published members by id (tree membership is the
predicate, not residence) and gives an entry the gradient position and
section of the **nearest planned scope whose tree names it at its current
address**. Derived material keeps its scope predicate exactly. When
source and target are the same scope both substitutions are identities,
which is why FLOW-2's and CTX-2's suites pass unchanged — and it is worth
naming what moved: the published tree now does authorisation work the
record row used to do, so a bug in tree membership is a disclosure bug
where before it was a trust-label bug. The address check keeps that
narrow, and the PDP still decides `MemoryRead` once per planned scope
before any of it runs.

Two things the acceptance work turned up. The approver *sets* are the
pack's, not a test fixture's: `regulated-strict` asks for one curator at
a team but a curator **and** a steward, two distinct people, at a
department or the org — so a two-level climb needs four principals, and
the steward cannot run the effect because steward reads no content in any
pack. And publish-time refusals are now uniformly `Conflict` rather than
`Invalid`: a member deleted, edited, or given up by its source between
approval and publication is the world moving under a well-formed request,
not a bad request, and each names the record.

Still standing: **rules do not climb.** ADR-0033 reversal trigger (e) is
half discharged — the same-scope constraint left the proposal surface, so
it is now a property of the rule engine — and the other half waits on a
rule target expression, for the reason decision 1 gives: a rule acts
under the material owner's authority, and an owner who configured no
target has decided nothing about disclosing upward. The direct publish
route stays same-scope (a restriction, never a relaxation). Everything
that climbs accumulates at the org root, where `MAX_CHANNEL_MEMBERS`
(10,000) is the standing bound and ADR-0031's subtree sharding is the
recorded upgrade. And a rewind at a source (FLOW-7) will not un-publish
at the target — a climbed record survives its source's rollback, which is
FLOW-7's decision to make, not this ADR's._

_FLOW-6 (2026-07-25, ADR-0035): the CLI review flow. The feature adds no
capability — FLOW-3 shipped every route it calls — so the whole of it is
two questions: **who is a reviewer to this binary**, and **what do they
have to be shown**.

**A reviewer is a governed principal, not an operator.** `synveda
proposal` opens no database connection, issues no SQL, and has no
`--database-url`; every verb is an HTTP call under the bearer `synveda
login` stored. That is not a style preference. This CLI already has a
store-backed half — `db migrate`, `tenant create`, `policy apply`, `role
bind` — which exists because a database with no usable gateway must still
be bootstrapped, and which audits itself as break-glass with OS-user
attribution. A review has no such moment. Approving is an act whose
authority is `ProposalReview`, whose count is the approval matrix, and
whose event is chained by the gateway; a CLI that inserted the approval
row would be the counting rule acting as authority, from a laptop, and it
would have to invent an identity, since `vedaflow_proposal_approvals`
names an `IdentityId` and the roles held at the target and the break-glass
actor has neither. The demo makes the claim un-fudgeable by **unsetting
`DATABASE_URL`** before the review begins, and the trail at the end shows
twelve governed acts with `actor_kind=subject` and not one break-glass
row.

**What a reviewer is shown is the effect on the target's channel, not the
proposal's contents.** Publishing is keyed by record id, so a proposal
either admits a record the channel does not name (`add`), replaces the
version it does name (`update`), or changes nothing (`none`) — read off
the *target's* tree, which for a climb is the ancestor's. The `update`
case is the one a review surface exists for and the one that had no
representation at all: FLOW-3's own AC establishes that the way to
republish edited content is a new proposal, so the channel is holding the
old version precisely when the review matters most, and
`GET /v1/proposals/{id}` returned the record's current text and nothing to
compare it with. It now returns both sides as object bytes.

Three readings the implementation settled, recorded because they are
behaviour. (1) **The new side is the object the proposal names, never the
record's row.** Once a record has been edited under its own review those
are two different texts, and showing the row would be showing content
nobody proposed; `unchanged` marks the drift and `proposed` keeps naming
what the approvals bind. (2) **The diff is field-wise with a line diff for
the text.** A memory object is canonical JSON with sorted keys — the form
`MemoryAsset::canonical_bytes` chose two features ago, in a comment that
says "FLOW-6 renders diffs of it" — and diffing those bytes as text would
render a multi-line content edit as one enormous escaped line, the worst
rendering of the most important case, while a content-only diff would
render a raised `sensitivity` or a closed `valid_to` as no change at all.
(3) **`publish` and `withdraw` joined the five named verbs**, because a
curator who can approve but not run the effect still has to leave the
terminal, and the deciding approval deliberately does not publish
(ADR-0032 decision 9). `open` deliberately did **not** join them: opening
is the proposer's act, the AC is about review, and the demo opens its
proposals the way proposals actually arrive — a contributor's POST, or a
FLOW-4 rule with nobody deciding to.

**One content-visibility widening, made on purpose.** Showing the old side
means a `ProposalRead` holder sees what a publication would overwrite, and
`compliance` holds no `MemoryRead` in any pack. It is admitted for the
reason ADR-0034 decision 1 admitted the proposed side — a review of a
change that hides one side of the change is not a review — and refusing it
would make the one role the invariant floor requires on everything
`restricted` approve replacements sight unseen. Bounded by the proposal's
own member set, the target's own channel, and the scope the reviewer
already reviews for; `review_surface.rs` asserts it against a compliance
principal proven, in the same test, to compose nothing from `POST
/v1/inject` at that scope.

Deferrals and standing friction, all recorded in ADR-0035: a curator bound
at one team is denied the tenant-wide listing (a *tenant*-resource
decision the packs grant to tenant-wide review roles only) and must pass
`--scope` — FLOW-3's boundary, not this feature's, and the CLI names the
flag in the refusal rather than leaving someone to read a Cedar file. The
detail response now carries up to three texts per member where it carried
one, bounded by `MAX_PROPOSAL_MEMBERS` (200) and `MAX_OBJECT_BYTES` as
before; a `?diff=` param is the recorded first move if size ever becomes
the reason someone cannot review, ahead of pagination. The line diff is
hand-written (LCS over lines, ~100 lines) rather than a dependency,
because the core path's licence rule makes even a small one a reviewed
diff — its correctness is ours, and the unit tests are the mitigation.
PRMT-1's prompts and SKIL-1's skill bundles will need a per-asset-kind
renderer behind the same seam, which is a new ADR rather than a widened
`match`; until then a non-object payload falls back to a plain text diff
rather than rendering nothing. And CNSL-1 landing does not retire this
surface — a console that recomputes the diff instead of reading these
fields is the bug.

One thing worth naming that was not this feature's: the demo pattern
every FLOW/CTX demo shares — start a gateway, poll `/healthz`, proceed —
**cannot tell its own gateway from someone else's**. A leftover process
from an earlier session held the port, the new gateway died on bind, and
`healthz` answered from a stranger signed with a different dev secret; the
symptom was a 401 on the first API call, twenty lines from the cause. This
demo now checks the child process is alive before and after the poll and
fails with the port named. The other demos still have it._

_FLOW-7 (2026-07-25, ADR-0036): rollback & pinning. Almost all of the
mechanism has existed since FLOW-1 — `force_update_ref` is a separate
function by name so "no rollback is ever a typo", and composition reads the
published channel per request with no cache, so "agents heal next session"
was a property to demonstrate rather than a feature to build. What did not
exist was the answer to the only question that matters: **what is a rewind
allowed to install?**

**A rewind may only install a state the channel has actually held.** The
target must be a strict *first-parent* ancestor of the head, which is not
the same as reachable, and the difference is the whole feature. Since FLOW-3
a publication through review is a merge commit whose second parent is the
proposal commit, so FLOW-1's `is_ancestor` — the fast-forward test, run
backwards — accepts commits whose trees are *proposed* member sets that may
never have been approved. `write_channel` has put the head first in every
parent list since FLOW-2, so walking ordinal 0 enumerates exactly the states
the ref has been in, and every one of them cleared the approval matrix when
it was installed. **That is why a rewind resolves no approvals of its own**
— it takes `ChannelRollback` plus the asset kind's read action and nothing
more — and it is not a convenience: `regulated-strict` asks for a curator
and a steward at a department, and a product whose answer to "a bad
instruction is reaching every agent right now" is "convene two people" has
not shipped rollback. The two decisions are load-bearing on each other, and
the ADR says so rather than leaving it to be inferred.

Two things the work turned up, both recorded because they are behaviour.
(1) **The rule needed one more clause than ordinal 0.** A channel's *first*
publication has no head to be its first parent, so a reviewed one puts the
proposal commit at ordinal 0 — a shape ADR-0032 decision 10 chose and
FLOW-3's own AC pins ("head first (there is none — this is the channel's
first commit), then the proposal"). Walking ordinal 0 alone would have
offered a proposal commit as a rewind target on exactly the channels that
have published least. The walk now stops at any commit a proposal names,
which is a fact the schema already stores and needs no marker column; the
acceptance suite covers both ordinals. (2) **A rewind rewinds; it never
advances.** Not an oversight: `write_channel` mints its commit before
attempting the compare-and-swap and retries three times, so a contended
channel leaves orphan commits parented on a head they never replaced, and
admitting first-parent *descendants* would make it possible to install a
member set no publication ever installed. Recovery from a mistaken rewind
is therefore publishing, which resolves the matrix again — the right price
for re-admitting content across the trust boundary.

**A pin freezes what a channel *serves* without moving where it points.**
Publications keep landing, the head keeps advancing, readers stay — and the
publish response names the standing pin, because a curator who publishes
and sees no effect has to be told why. It is a ref
(`pin/{asset}/{channel}`, the namespace ADR-0031 decision 1 reserved for
it), so there is no new table; the read path left-joins it and coalesces in
the query it was already running, so `inject` gains no round trip.
`ChannelWatermark` carries `pinned` and the inject response now returns its
channel citations, because tech plan §2.5's "inject responses cite commit
hashes" was true only of the audit event, and a citation only an auditor
can reach is not one. Reader-side pinning — a scope holding an *ancestor's*
channel for its own members, which is what PRMT-1's "consumer pins"
phrasing suggests — was refused for two reasons in the ADR: no action in
the vocabulary expresses "govern what someone else's channel serves me",
and it would make a scope's channel resolve differently per caller, so
"what did this scope publish on date D" would stop having one answer.

**Exactly one thing decides what readers see**, so a rewind of a pinned
channel is refused with `Conflict` naming the pin. The asymmetry with
publish is deliberate: a publication's contract ("this channel now holds
these records") stays true under a pin, and a rewind's contract is the
FLOW-7 sentence itself — every consuming agent heals on next session start
— which under a pin is false.

Migration 0021 is the only schema change and it grants exactly one thing:
DELETE on `vedaflow_refs`, narrowed by a *restrictive* policy to names
beginning `pin/` and by a trigger that raises for anyone bypassing RLS. A
pin is the first ref that is a standing decision rather than a pointer into
history, and a decision that cannot be reversed is not one this product
should write; a channel pointer stays undeletable, and a truncate trigger
closes the statement that would take every pointer at once.

**ADR-0034 reversal trigger (c) is discharged: a climbed record survives
its source's rollback.** The department admitted it under the department's
approvers, and a cascade would hand a team curator a veto over a decision
the org's own stewards made — precisely what ADR-0034 decision 5 refused
when it declined to enforce a ladder, pointed downhill. What changes for
the reader is which scope's section the line appears under, which is true
rather than cosmetic. The cost is that a rewind at one scope is a partial
remedy by design, and an operator who wants a record gone everywhere
rewinds at each scope that admitted it.

One thing the demo makes explicit rather than hiding: **a rewind moves the
trust boundary; it does not delete.** Under the default pack a rewound
record composes again as `[unreviewed]` for readers whose chain it lives
on, and disappears entirely only for readers it was reaching *through* the
rewound channel. Both are asserted, in the test and in the demo, because a
demo that showed only the second reader would be claiming a feature this is
not. Deferrals, all in ADR-0036: commits abandoned by a rewind stay in the
store unreachable from any ref (`verify` still recomputes them — it walks
rows, not reachability — and FLOW-8 will have to decide whether they belong
in a git mirror); a pin's reason lives in the audit chain rather than on
the ref, so "why is this pinned" needs AUD-2's query surface; reinstating a
rewound state without re-review would need a reflog, which is a table and
its own ADR; and prompts and skills are refused by name rather than
governed by memory's read action until PRMT-1 and SKIL-1 bring theirs._

_AUTHZ-4 (2026-07-26, ADR-0037): lapses. Seed §6 calls this "the
mechanism that lets one product serve both an SMB and a bank", and what
shipped is mostly the discovery that two earlier features had already
written it. `regulated-strict.cedar`'s header said a content-role binding
"*is* the seed's explicit grant for cross-team read; **AUTHZ-4 lapses add
the time-boxed variant**"; `AssetKind::Policy`'s doc comment named lapses;
and every embedded pack has carried a `policy` approval rule since FLOW-3,
`regulated-strict`'s written straight off tech plan §2.4's lapse row — 2 ×
steward, two distinct — with nothing until now that ever resolved against
it. So a lapse is **an ordinary FLOW-3 proposal whose asset is `policy`
and whose effect is a grant row**: no new asset kind, no new approval
rule, no new proposal action, and the 300-cell approval-matrix golden
byte-identical afterwards.

**Expiry is a query predicate, not a job**, and that is the decision the
rest hangs off. The feature text names a Temporal timer; nothing in the
workspace depends on Temporal, but that is a sequencing accident and not
the objection. The objection is that a job which fails to run leaves a
cross-team read standing, which is the worst possible failure for the one
feature whose entire promise is that access ends by itself.
`lapses::active_for_scopes` selects `revoked_at is null and expires_at >
now()` against the database's own clock and `authz::gather` calls it per
request; the sweep only chains `policy.lapse.expired`, and the demo's
trail is complete because the *grant* event records the window it opened.
Durations are seconds with no minimum, so the AC and the demo both observe
a real expiry rather than a simulated one — the acceptance test sleeps
through a four-second window and the demo through ten.

**The finding that shaped the read path: a permit that is never asked
grants nothing.** ADR-0024 fixed the inject candidate universe at the
caller's own chain, deliberately, because a pack's permits cannot be
enumerated. The consequence — which this feature was the first to run into
— is that ADR-0015's "explicit grant" for cross-team read has *never
reached inject*: the PDP would permit it and nothing asks. A lapse that
produced only a Cedar permit would have satisfied the letter of its AC at
the PDP seam and changed nothing a reader sees. So the universe widens by
lapse and by nothing else, and the asymmetry is principled rather than
convenient: a lapse **enumerates** (one row naming one target), is bounded
in time, carries a mandatory reason, and cleared a dual-approval matrix,
where a binding is durable, needs no approval, and would bring the derived
channel with it. Content-role bindings and `standard`'s department sharing
stay on CTX-5's deep-query surface; decision 13 records the shape the
change would take when somebody decides they want it.

**A lapse admits the target's published channel and nothing else**, which
is what let the whole read-path change be one plan entry. Published
members are fetched by id and uncapped (ADR-0031 decision 9), so hybrid
retrieval, the scope predicate, the index and the sidecar never learn what
a lapse is — the same property that let FLOW-5 admit a record living below
its publisher. The cost is stated rather than hidden: a lapse over a scope
that has published nothing discloses nothing, and the remedy is to
publish, which is a review.

Two refusals worth naming because both were nearly the other thing. The
seed's own example narrows to "`procedure` records", and that qualifier is
**refused rather than stored**: `MemoryRead` decides once per scope with
no record in hand, and a stored narrowing nothing applies is a widening
wearing a narrowing's name — the single most dangerous thing this feature
could have shipped. And `base.cedar` gained the product's first *permit*,
which weakens the sentence that file used to justify itself: it is no
longer "these are the things no pack can escape" but "these are the things
no pack can change". It is there because an override a pack can neutralise
by omission means "lapse" means different things in different tenants
(ADR-0014 decision 6's rule); every forbid still beats it, so quarantine
holds and a service identity cannot be widened past its anchor, and a pack
refuses the mechanism outright by setting its ceiling to zero — enforced
at decision time, so the flip ends standing grants on the very next
request.

Three things the acceptance work turned up. `vedaflow_proposals` carried
`check (target_channel = 'published')` from when publishing was a
proposal's only possible effect, so migration 0022 widens it and the column
now names the **effect** — a `ProposalEffect` vocabulary rather than a
`Channel`, because a lapse has no channel and storing `published` on a row
that publishes nothing is the paper-over the schema refused. The demo's
first run failed at exactly the right place: every principal was a service
identity, and the base layer would not widen bea's token past her anchor —
correct behaviour, demonstrating the wrong feature, so the reader became a
user and the property became a comment. And the demo's trail was racing
the sweep by about a second; it now waits for the *event* rather than
sleeping a fixed interval, because a demo that sometimes prints a trail
without its last line is showing the scheduler's luck.

Still standing: rules do not lapse and lapses do not climb — a lapse's
target is one scope, by decision 8, and the material below it reaches a
reader through what that scope published. The record-type qualifier waits
on AUTHZ-5, which is also when a lapse can declare a sensitivity ceiling
above `internal` and the invariant floor engages by itself with no
lapse-specific rule anywhere. `LapseConfig` carries one number today; a
channel rule would be a new ADR rather than a key, because it changes what
an approver is consenting to._

_AUTHZ-5 (2026-07-26, ADR-0038): ABAC conditions. The feature text names
five — sensitivity, residency, channel, time-of-day, purpose-of-use — and
**one shipped**, each of the others refused or deferred by name rather than
in bulk. Channel is already decided per scope by the pack's composition
config at the same seam in the same walk, and a Cedar half would let a pack
permit what its own config withholds with no defined resolution order.
Residency needs a second region, and seed §6's cross-region rule turns out
to be a *degradation* — "metadata-safe summaries" — rather than a denial,
which is recorded so OPS-3 does not rediscover it. Time-of-day would be a
second clock in a product that deliberately put expiry in a row read at
decision time, and a time-based denial returns a smaller block the reader
cannot distinguish from an empty one. Purpose-of-use is refused as a
widening permanently: it is the reader authorising their own read, and this
product's answer is older than the ADR — a disclosure is initiated on the
disclosing side, and a lapse already carries a reason a reviewer at the
target consented to.

**A closed vocabulary is decidable without the record**, and that is the
whole design. The `MemoryRead` seam decides once per scope with nothing in
hand — the constraint ADR-0037 decision 6 refused to paper over — but there
are exactly four tiers, so the seam can be asked about each of them before
anything is fetched. The composition walk asks four times per scope and
keeps the answers as a *set*; `ScopeTier` (a scope-and-tier pair) becomes
the read path's predicate unit, and the three scope-keyed store queries
match `(scope_id, sensitivity) in (select * from unnest($n::uuid[],
$m::text[]))`. What a single ceiling could never express is now the ordinary
case: a reader's own home admits `confidential` while the team one level up
admits only the working tiers, on the same chain, in the same block. The
rule generalises and is stated as one — an attribute whose domain is small
and closed can join the decision before the fetch; one that is per-record
and open cannot, and is refused rather than stored. Record *class* is closed
too and stays refused anyway, on a product judgement rather than a
mechanical limit: an extractor assigns it with uncalibrated confidence, and
a disclosure narrowed by a label a model chose is a control that only looks
like one.

**The AC's "compliance-granted permission" is not a mechanism this feature
wrote.** `base.cedar` forbids `MemoryRead` at `restricted` unless a standing
lapse covers it; `LapseTerms` gained a declared ceiling; the matrix resolves
at that ceiling, so declaring `restricted` pulls in ADR-0032's invariant
floor — the `compliance` role and two distinct approvers, under every pack.
ADR-0037 decision 14 predicted exactly this ("the floor engages by itself,
with no lapse-specific rule anywhere") and that is what happened. The
product now has one non-negotiable rule about the tier in both directions,
signed by the same role: nothing reaches a published channel at `restricted`
without it, nothing *becomes* `restricted` without it (the classification
proposal resolves at `max(current, proposed)`, so a declassification is
priced at the tier it is leaving), and nothing reaches a reader without a
grant that cleared it.

**Two corrections the implementation forced, both caught by tests rather
than by review.** The first: asking the *governance* guards — publish, a
climb's disclosure, a rewind, a pin, a reclassification — at the material's
own tier reads better in a diff and is wrong. FLOW-3's restricted-publication
AC failed immediately, because `restricted` is forbidden to every reader
without a lapse, so a tier-following guard makes restricted material
unpublishable by *anyone* — which strands the invariant floor's own cell and
leaves a restricted lapse with nothing to disclose, since a lapse admits only
what the target published. Those guards ask *whose* material it is and stay
at the working tier; the matrix prices the tier. Decision 10 states it as the
rule it is: **governing material is not composing it**, which is ADR-0035
decision 8's sentence seen from the other side. The second correction was in
a test the leak suite inherited: a content-role binding at *another* team
does not bring that team's material into inject, because the candidate
universe is the caller's chain and widens by lapse and by nothing else
(ADR-0037 decision 13) — so the tier rule is about scopes a reader already
composes, and the fixture moved onto her own team.

**The engine stopped clamping.** `allowed_sensitivities` capped every read
below `restricted` regardless of what any pack said; a clamp is a decision
nobody took, so it is gone and the retrieval crate executes the plan it is
handed. `crates/synveda-retrieval/tests/hybrid.rs`'s "restricted is never
retrievable (AUTHZ-5 owns lifting this)" is now its opposite by name — a
pair set naming the top tier surfaces it, one naming it at the *wrong* scope
surfaces nothing — and what keeps a real reader out is policy. Published
members are the one place the pair is not enforced in SQL: that read has no
scope predicate by design (a tree may name a record living below it,
ADR-0034 decision 6), so the tier is checked in Rust against the *naming*
scope's set, with the union pushed down as the hard ceiling.

Four times the decisions on the hot path, and **the plan stage did not
move**: p50 13.44ms / p99 30.42ms against the 150ms budget, plan stage
4.42ms over 1,050 calls where CTX-3 recorded 4.5ms at a quarter of the work.
The four asks at one scope differ in one context attribute and share their
entity graph, so HIER-3's fragment cache absorbs them — decision 1's claim,
now measured. Reversal trigger (a) is nowhere near tripping, and option 6
(validate tier-monotonicity at install, short-circuit top-down) stays the
recorded upgrade.

Still standing: `restricted` at a reader's own personal scope is invisible
to them without a grant, deliberately (decision 7) — the tier means what it
says, including for the author, and the only way a record carries it is a
proposal two people approved. The extraction prompt no longer offers the top
tier and the pipeline clamps at `confidential` (decision 8), so every
`restricted` record in the product has a compliance signature behind it. A
tenant wanting more than four tiers gets labels beside the tier rather than
instead of it, and that is a new ADR because the floor and the matrix key on
the order. `synveda_authz_decisions_total` now counts up to 4× per inject —
the metric's meaning is unchanged, but any dashboard reading it as "injects"
was already wrong and is now wrong by a bigger factor. And EVAL-5 owns what
the leak suite grows into: 10k variants nightly, the cross-tenant fuzz
(TEN-6), and the prompt-injection-via-memory half._

_MEM-5 (2026-07-26, ADR-0039): dedup & conflict detection. Six accepted
ADRs deferred to this one, which is the usual sign the design was
half-settled by the features that left room for it: ADR-0020 refused a
second dedup mechanism at the observe seam, ADR-0022 left `valid_to` open
and said so, ADR-0023 declined an embedding staleness check, ADR-0025
shipped an exact-match conflict predicate and *exported the comparator*,
ADR-0031 put `valid_to` inside the content address on purpose, and
ADR-0033 left similarity-triggered promotion here. What they were all
waiting for turns out to need no new read path at all: **supersession is
`valid_to` moving, and every composition read already filters on it.**
Closing a window makes a fact stop composing, keeps it addressable at the
instant it held, and changes its content address — three properties, none
of them written by this feature, all of them consequences of decisions
taken between FND-4 and FLOW-2. The code that had to be written is the
part that decides *which* window to close.

**Two nomination signals, because one of them is not always meaningful.**
The feature text says "embedding + minhash" and the temptation is to read
it as one mechanism with a second opinion. It is not: the default embedder
is a BLAKE3 hash whose geometry carries no meaning at all — ADR-0023
decision 6 says so in its own doc comment — and it is what dev, demos,
every hermetic test and `make eval` run on. A design that nominated only
by vector neighbourhood would detect exactly nothing in the configurations
the AC has to pass in, while looking correct in a suite that runs against
real TEI. So the lexical leg is load-bearing and the semantic leg is the
upgrade, not the reverse. Everything the AC test proves, it proves through
MinHash alone.

**The frame is what gets hashed, and that decision was found by a failing
test.** The first cut hashed the full token set, and the canonical
knowledge update — "the stand-up is at 09:30" against "the stand-up moved
to 10:15" — came out at J≈0.4, sitting on top of the band threshold, so
whether the product noticed a fact had changed depended on a coin flip.
Hashing the *frame* instead (content words, stopwords and value tokens
held out) puts the pairs worth judging at 0.6–1.0 and unrelated ones near
zero, and the scoring still uses the full set — two statements whose
frames match because only a number changed are precisely *not* duplicates.
One definition of "frame", used by the index and by the judge, which is
also why it lives in `synveda-store` beside the columns it encodes while
the judge lives in `synveda-ingest` beside the pack config that tunes it.

**The judge is a conjunction of refusals and it is honest about being
one.** Similarity can nominate a pair; it cannot decide one. "We deploy on
Tuesdays / Thursdays" and "deploys go through make deploy / tests go
through make test" sit at nearly the same lexical distance and one pair is
an update while the other is two true facts. Three conjuncts separate
them — frame overlap by coefficient (an update is routinely *longer* than
what it replaces, and Jaccard charges for the added words twice), a shared
leading frame word as a crude subject proxy, and something actually having
changed. It will miss a subject named last, a passive voice, a language
whose word order is not English's. That is the asymmetry applied on
purpose: a missed update leaves a stale fact composing beside a fresh one,
which is what the product already did; a wrong supersession removes a true
fact from every future inject, silently. The model-backed judge — the
Graphiti pattern's actual mechanism — is decision 6's named seam and
EVAL-2's measurement is its trigger.

**The boundary that took the most thought is the one against published
material.** A contradiction against a record a scope has reviewed is a
real and interesting event, and the pipeline refuses to act on it: reviewed
content leaves the trust boundary through a proposal or a rollback, never
as a side effect of somebody's session. So the estate can hold a published
fact its own members have contradicted, which is uncomfortable and correct
— and the refusal is counted and chained rather than swallowed, because a
refusal nobody can see is a refusal nobody can act on. FLOW-3's shape (the
pipeline opens a supersession *proposal*) is the recorded next move, and
deliberately not invented here.

Two things worth knowing that the AC does not say. **Never ADD-only cuts
both ways**: an observation that reaches the pipeline after the fact that
replaced it is inserted with its window already shut, not dropped — the
whole design is order-independent in valid time, which is what makes it
safe against a replayed spool or a clock that was behind. And **an
unindexed vector dimension degrades the stage rather than failing the
write**: found by the MEM-4 chaos test, whose mock TEI serves a dimension
with no HNSW index, and fixed the only way that is defensible — dedup is a
stage of the write, not a gate on it.

Still standing: the deterministic judge's recall is unmeasured (EVAL-2);
there is no operator surface to reverse a wrong supersession, though the
data model makes one ordinary — the record, every version of it, and an
edge saying why are all still there, and AUD-2/CNSL-2 are where a curator
would act on the `memory.superseded` trail; supersession never crosses a
scope, an owner, or a class, and each of those is a recorded miss rather
than an oversight; `record_signatures` is not backfilled for records
written before migration 0024, exactly as ADR-0023 recorded for the MEM-3
window; and the real LongMemEval corpus with its judge is still EVAL-3 —
what shipped is the category, measured on this suite's fixtures, gated at
a floor the product could not have met a day ago.

One fix outside the feature, found by it: `evals/lib.sh` built the
workspace *after* pointing `DATABASE_URL` at its empty scratch database,
so sqlx's compile-time checks validated every query against a schema that
did not exist. It passed only while the build cache happened to be warm,
and the first workspace change since EVAL-1 landed broke `make eval`
outright. The build is now `SQLX_OFFLINE=true`, against the committed
`.sqlx` data — which is what CI compiles against, and for the same
reason._

_MEM-6 (2026-07-26, ADR-0040): decay, TTL & staleness. The acceptance
criterion turns out to be a statement about *when* retention is read
rather than about what it does, and the whole design falls out of taking
that literally: **nothing is stamped on a record.** No `expires_at`, no
per-record TTL, no sweep bookkeeping table — a record's fate is a function
of facts it already carries (class, kind, valid time) and the pack in
force at its scope *now*. "A retention policy change re-evaluates existing
records" is therefore structural rather than a backfill, and a backfill is
exactly what it would have been: a job over every record in every tenant
that fails silently when it misses rows. Seed §4.2's "`ttl` / decay policy
**reference**" is read as the reference it says it is, and the referent is
the scope's pack.

**Two enforcement points, and the earlier one is the read path.** The
composition plan already carried per-scope channel rules (CTX-2) and
per-scope tier sets (AUTHZ-5); it now carries per-scope horizons, and
`compose_candidates` refuses material past them in SQL. The sweep is
*disposal*, not enforcement — ADR-0037 decision 4's shape applied a second
time, for the same reason: the interval between a policy change and the
next sweep must not be a window in which expired material is still
injectable. The demo is the proof: a schedule applied over the CLI changes
the very next inject, with nobody acting and nothing restarted.

**Two horizons, because one cannot serve both.** *Expire* is the FND-4
temporal delete: the record leaves the live corpus, its version archives,
the CTX-1 sidecar drops the document through the change feed it already
tails, and `records_versions` keeps answering — which is what the seed's
own killer demo needs. *Destroy* is the history rows themselves, past a
second and longer horizon, and it is the first thing in the product that
removes recorded content from the database. A product that only expires
keeps every payload forever behind an as-of query, which is not retention;
one that only destroys cannot answer what the agent knew last March. The
destruction path is a **named flag** the append-only trigger honours
(`synveda.retention_purge`) rather than a `SECURITY DEFINER` function,
because the function would run as the owner and bypass RLS — trading a
trigger migration 0001 itself calls "not a security boundary" for a hole
in the boundary that is one. The adversarial suite asserts the
consequence: with the flag set and the grant held, a purge naming another
tenant's history matches nothing.

**The staging plane's disposal closes the oldest parked obligation in the
repo.** ADR-0020, ADR-0021 and migration 0012's own comment all named
MEM-6/TEN-5; `observe_events` has held every payload ever observed,
pre-extraction, since MEM-1. It is disposed of on its own horizon (the one
number an embedded pack does set — 7 days under `regulated-strict`, 30
elsewhere), markers before payloads because the FK says so, with pending
reviews that aged out counted separately in the event. The honest cost is
named rather than discovered: disposal frees `(tenant_id,
idempotency_key)`, so **MEM-1's admission guarantee is worth exactly as
long as this plane is kept** — days, with a validated one-day floor,
against adapters that retry in seconds.

**No embedded pack names a record TTL, and that is the deliberate part.**
An upgrade that silently deletes a tenant's memory is the one surprise
this product must never spring (ADR-0033 decision 6's fail-safe, restated),
so the machinery is on, the horizons are unset, and a schedule is a
decision somebody takes. Pinned material is exempt from all of it by seed
§4.2 — one clause in the candidate query and one branch in the scorer, not
a pack field that could re-admit it.

**Staleness scores; it does not label.** Exponential decay over time since
*last assertion* — MEM-5's merge stamp is that signal, which ADR-0039
decision 10 predicted this feature would want — computed in the engine
from the instant CTX-2 already takes, so the determinism AC never sees a
clock. A ranked record that has halved in freshness sorts as though it
ranked twice as far down, within its gradient position and never across
one; unranked order is already recency, so decay adds nothing there. The
score rides the inject response and the audit event as integer per mille;
the rendered block gets no `[stale]` marker, because the labels there are
trust statements and an age is not one. **Retention runs from first
assertion, staleness from last** — two clocks, two questions, and a
restatement moves only the second.

Deferrals and forward obligations: expiry is not reversible through any
product surface, and past the destruction horizon not at all — which is
what destruction means, and why no embedded pack sets one; a published
tree can name a record a horizon has removed, inert by construction
(`compose_members` already refuses it) because a commit is an authored act
and a sweep has no author; ADR-0031's "re-commit when they rewrite"
obligation is discharged with *nothing to do*, since a log channel's tree
is exactly the write it recorded and an expiry changes no address;
`pgmq.a_observe` and `audit_log` retention are deliberately out of scope
(TEN-5, against AUD-3's anchoring — an append-only hash chain a background
loop can delete from is not tamper-evident); the staleness half-life
defaults are a product guess until EVAL-4 measures them, and staleness
does not reach RRF fusion, which is recorded there too; and the sweep's
per-tenant cost is bounded by an idle look — one transaction and three
indexed reads for a tenant with nothing to dispose of — the FLOW-4/AUTHZ-4
lesson, which this feature's own demo re-learned on the shared dev
database and answered the same way they did, with a scratch database per
run._

_CTX-4 (2026-07-27, ADR-0041): tiered injection. **The defect
progressive disclosure actually fixes is not token efficiency — it is
silence.** ADR-0025's first-fit assembly skipped an entry that exceeded
the remaining budget and carried on; the count reached the audit event as
`skipped_over_budget` and reached the caller as nothing at all. An agent
that does not know a runbook exists cannot ask for it, and a thin block
is indistinguishable from an empty corpus. So a candidate whose body does
not fit is now offered its **index line** instead of being dropped — the
body truncated at 320 characters with the record id as the handle,
rendered through the same code a body line uses, so a body and its index
form can never disagree about a trust marker.

**The index tier is the same permitted set rendered shallow, and there is
no new Cedar action.** Index candidates are the candidates `compose`
already fetched under the plan: the same per-scope `MemoryRead`
decisions, the same tiers, the same channel rules, the same retention cut
and pinned exemption. A "may list but not read" verdict was the honest
reading and the wrong one — a name and a description are content, and a
second, weaker decision is a second leak surface across every pack × role
× scope × tier cell the AUTHZ-5 suite covers.

**A demotion happens only when it saves.** Both renderings are estimated,
the index line is tried only when the body does not fit, and taken only
when it is strictly cheaper. That one rule is what let a mechanism built
for assets that do not exist yet ship against a corpus made entirely of
assets that do: the median memory record is summarised at write time
(seed §4.2, MEM-3) and is never demoted, because demoting it would spend
budget to say less. `AssetKind` has five variants and one populated —
when PRMT-1/2 and SKIL-1 bring assets carrying an authored name and
description, the index slot renders those, which is why this is a
per-kind rendering seam rather than a memory-record special case.

**A handle is a name, not a capability.** `POST /v1/recall` takes ids and
re-runs `composition_plan` exactly as inject does, so an id the current
plan does not admit is not served — including one that sat in a block
composed five minutes ago under a role the caller has since lost. Signed
expiring handles were the obvious performance answer and were rejected
outright: they would be the first construct in the product to outlive the
decision that minted it, and every freshness promise since ADR-0014 dies
there. The plan walk is ~100µs; the promise is worth more. Refusals are
uniform and silent — a missing id, another tenant's id, a denied scope, a
tier not earned, a horizon passed and a channel bank mode closed are one
outcome, absence — so recall never becomes an existence oracle, and a
request naming ten ids of which three are inadmissible serves seven
rather than refusing the whole.

**The measurement is the acceptance criterion, so it is a number rather
than a paragraph.** One corpus composed twice at a deliberately tight
240-token budget: with the tier `off`, one record named and 80 tokens
spent; with `demote`, two records named and 217, of which the tier cost
**122 — 56% of the block**. That is ~90 for the index line, ~23 for the
legend, ~9 for the id the watermark grew by. Read honestly the tier is
expensive at that budget and under 8% at the seed §4.4 default of 1,500,
because the cost is a fixed ~90 per named record against whatever the
body would have been: nothing at all for the 15-token records MEM-3's
write-time summarisation produces, roughly 4× for a 360-token runbook,
one to two orders of magnitude for the context packs and skills PRMT-2
and SKIL-1 will bring. **ADR-0025's short-id trigger is discharged with a
number rather than closed** — the id is 9 of the 122, so that scheme
would recover about 7% of the tier's cost in exchange for read-path state
or a prefix oracle. The line to watch is the 90, and its lever,
`index_entry_chars`, is already a pack field.

Two things worth naming. The legend — the one line saying what
`(recall <id>)` means — is charged to the first demotion, so a block with
no index entry never pays for it and stays byte-identical to today's; and
it **must not contain the parenthesised form it describes**, which the
first draft did and its own acceptance test caught, because an agent
scanning the block for `(recall …)` finds the legend first and goes
looking for a record called `<id>`. And ADR-0027's option 6 was
reconsidered on the record and re-rejected: tiering landing was the
trigger for revisiting a per-prompt inject at `UserPromptSubmit`, and the
answer is no — the index tier *reduces* the need for per-prompt
injection, and recall is one call for one body rather than a whole
recomposition.

Deferrals and forward obligations: `index_tier: off` restores the
pre-CTX-4 product byte-identically, which is a test rather than a claim
(the MEM-5/MEM-6 discipline); a truncated body is a poor description
until authored assets exist, and inventing one would be the model call
the read path structurally cannot make — MEM-3 owns it at write time;
option 9's always-index threshold waits behind the pack knob for bodies
that far exceed their index lines; option 4's separate index budget is
the answer if EVAL-4 shows demotions displacing bodies that mattered; a
demotion can displace a smaller, lower-priority body, which is the seed
§4.4 gradient doing its job and still a real change in what a fixed
budget buys; recall pays inject's decision cost without its retrieval leg
and is deliberately uncapped in body size; and the 32-id cap is a blunt
instrument against corpus exfiltration where a rate limit is the sharp
one — AUTH-6's. `context.recalled` is the third primitive's first chained
event (seed §2.2 principle 5), and `context.injected` gains a per-entry
tier, so "was that agent given the payments runbook, or only told it
exists" is answered by reading the chain rather than by re-deriving
rendered widths from a corpus that has since moved._

_CTX-5 (2026-07-27, ADR-0042): recall becomes a query. **Most of this
feature shipped in CTX-4, on purpose** — the route, the audit action, and
a `RecallEntry` already carrying scope, channel, kind, class,
sensitivity, provenance, the valid window, the object address and
staleness, which is the feature text's "results labelled with channel,
provenance, validity" already done. What did not exist was a way to ask a
question rather than name an answer, a universe wider than the chain, and
a second time axis. So `POST /v1/recall` takes `{ids}` xor `{query}` xor
neither-with-an-instant, under one audit action, one 32-entry ceiling and
one universe: a second way in to one surface rather than a second
surface.

**The chain-only universe was a cost decision in CTX-1 and had become a
functional defect.** The packs grant beyond the chain and say so in
Cedar — `standard` permits the department, every pack permits a content
role's bound subtree — and under ADR-0024's universe none of those reads
could be *performed*: grants nothing in the product could exercise,
carried since CTX-1 and hit from the other side by AUTHZ-4, which found
the same wall and answered it by widening on lapses alone. The AC asserts
the fix from the reader's side rather than at the PDP: one corpus, one
identity, the real `standard` pack, and `POST /v1/inject` returns nothing
of the sibling team's material while `POST /v1/recall` returns it — with
`regulated-strict` over the identical corpus still refusing, which is
what makes the widening **more scopes asked rather than more allowed**.

**The universe is the scopes that can contribute to this request, and
every one of them is decided.** The ids form decides the scopes that hold
or publish the named ids; the query form decides the tenant's *occupied*
scopes — those holding at least one record, plus those whose
`memory/published` ref is non-empty. That narrowing is cost with no
semantic content: `admit` reaches records only through a scope-predicated
derived sweep and a published-member read, so a scope that holds nothing
and publishes nothing contributes the empty set whether the PDP allowed
it or denied it. Inferring the universe from a pack's shape would have
been far cheaper and was refused outright — a second model of what the
policies mean, living beside the policies, silently under-returning the
moment a custom pack grants something the heuristic does not know about.

**The broad plan is AUTHZ-4's lapse mechanism generalised rather than a
new one**: `LapsedScope`'s lapse becomes an `Option` and its candidates
come from occupancy instead of `lapsed_scopes`, and the per-scope body —
four `MemoryRead` asks, the target's own effective pack for its channel
rule, its own horizons, its own tier config — is byte-for-byte the loop
that was already there, so composition, inject and both recall forms keep
sharing exactly one answer to "may this caller see this record". A
widened universe needed a widened *binding* set, which is the one thing
the design missed: `gather` reads the caller's bindings on their own
chain, right for inject and silently wrong here, since a binding at a
scope off that chain is exactly the grant an administrator issues to
widen someone's reach — one of the two ADR-0024 left unreachable. Recall
now reads every binding the subject holds in the tenant, while
`effective_roles_at` still admits one only at a resource whose own chain
contains its scope, so what widened is what is *read*, never what a
decision considers. It was found by its own acceptance test failing.

**As-of is two explicit instants, and it rewinds the corpus but never the
authority.** `as_of` is transaction time and `valid_at` is valid time —
FND-4's two axes, no third concept — both defaulting to the request
instant, which makes CTX-4's surface a special case of this one rather
than a sibling. The PDP decides with the caller's *current* identity,
roles, packs, lapses and placement whatever the instant says: a leaver, a
demoted user, a caller whose lapse closed last night reads nothing
historical, because there is no historical permission to inherit. That is
ADR-0041's "a handle is a name, not a capability" restated for time, and
it draws the line to AUD-2 — CTX-5 answers *what was there*, AUD-2
answers *who could see it*, from the chain that recorded the decisions as
they were taken.

**The axis is fact versus judgment, and it decides all three.** Where
material lived is a fact about the past, so a version is attributed to
its own `scope_id` and that scope is decided. What material *is*, and
whether the organisation *stands behind* it, are judgments — revisable,
and a revision governs every read including historical ones. So
**classification is retroactive**: a version is admitted at the strictest
tier its record has carried at or after the instant, which means the
AUTHZ-5 leak suite cannot be walked around with a timestamp, while a
declassification does not retroactively expose the history written when
the record was classified. At `as_of = now` the maximum degenerates to
the current tier, so today's behaviour is unchanged. And the
`memory/published` tree is read at its current state, never rewound: the
symmetric reading was tempting and would undo FLOW-7, whose entire claim
is that a rolled-back instruction reaches not one further agent. A record
published in March and withdrawn since is still *served* as-of March if
the caller may read its scope — as derived material, marked unreviewed,
which is the true statement that nobody stands behind it any more.

**A bare instant sweeps; a query with an instant ranks the survivors.**
The search indexes hold current truth by construction (ADR-0024 decision
4) — the sidecar re-reads each changed id's current version, HNSW is over
live embeddings — so a since-expired record cannot be ranked, only swept
or named. Rather than pretend a query leg and a time machine compose for
free, the two shapes are honestly different: `{as_of}` alone reads
`records_versions` through the same admission and is the complete answer
to "what did the agent know on March 3rd", which is the `--as-of` demo's
shape. As-of reaches expired material and never destroyed material —
MEM-6's two horizons inherited rather than retaken, which is what keeps
retention real. And the degradation posture inverts from inject's: an
embedder failure still degrades to sparse-only, but a retrieval failure
is an honest 5xx, because inject's caller cannot see an error and
recall's caller asked the question.

**The measured number moved three times, and each move was a different
lesson.** The plan stage over 512 occupied teams cost 378ms
re-materialising Cedar's entity store per decision; **one materialisation
per walk took it to 120ms** — 3.1×, a decision written into the ADR
before the code existed and the only reason the feature was ever
plausible, since Cedar's per-decision cost is dominated by building the
entity store rather than by evaluating policy against it. The next two
optimisations — resolving the pack once per scope, reading assignments
once per request — bought nothing measurable, which said the cap and not
the code was the lever left. At the shipped **32-scope cap the plan stage
is 13.2ms**, inside the **15ms** ADR-0029 pre-registered for it before
anyone could tune to the result, and the whole request is 17.1ms of that
gate's 300ms — so the slice GRPH-3 will claim for graph expansion is
measured as still being there. Roughly 7.7ms of the 13.2 is fixed cost
(identity, chain, occupancy, assignment and binding reads) on Docker
Desktop's virtualised fsync, which makes 32 a dev-hardware number.

Stated plainly, because it is a real limitation on the day this ships
rather than a theoretical one: **32 occupied scopes is a small universe
for an enterprise product**, and a tenant wider than that gets a
genuinely incomplete answer to a query. It is reported — `truncated`,
with both counts, in the response and on the audit chain — and candidates
are ordered nearest-first, so what drops is the farthest material rather
than an arbitrary slice. `SYNVEDA_RECALL_MAX_SCOPES` is the lever for an
operator who has measured their own plan histogram.

**One MCP tool, over the route that already exists.** `recall` ships as a
third entry point in `adapters/claude-code` beside the hook and the
driver, registered through the `mcpServers` slot ADR-0027 reserved for
it — that trigger discharged as configuration rather than restructuring —
with `{query?, ids?, as_of?, valid_at?, limit?}` in one tool rather than
one per shape, so there is no second place for the id/query exclusivity
to be got wrong. The JSON-RPC framing is written directly rather than
taken as a dependency: the plugin is `tsc`-built with no bundler and no
runtime dependencies precisely so enabling it needs no install step, and
the surface actually needed is `initialize`, `tools/list`, `tools/call`
and one notification. Its bearer comes from the same `synveda` CLI seam
the hooks already shell to, and results are rendered by the block's own
line renderer plus a watermark, so an agent that has read an inject block
does not learn a second format. ADR-0033's trigger (d) is discharged
alongside: the promotion sweep's action set gains `context.recalled` —
the stronger signal that ADR wrote a trigger for — and its evidence names
which signal it counted, which `promotion.rs` was written to assert.

Deferrals and forward obligations: graph traversal is **not** here —
GRPH-1 owns the schema decision ADR-0029 pre-registered the criteria for,
and GRPH-3 is where a third leg joins the fused id list, feature-flagged
and degradable, without touching admission; indexing historical versions
in the sidecar is the recorded upgrade if as-of queries ever need to rank
since-expired material; option 3's pack-declared universe is the lever if
the shape of real tenants breaks the cap, after EVAL-6 re-derives it on
production-shaped IO where most of today's fixed cost disappears; the
caller-supplied query vector was revisited from ADR-0026 and re-rejected,
because a text/vector mismatch is unverifiable server-side and returns
plausible nonsense on the deep surface; the id cap and the scope cap stay
blunt instruments against corpus exfiltration where a rate limit is the
sharp one — still AUTH-6's; the hand-written JSON-RPC loop is protocol
code the project now maintains, with the SDK plus a bundler recorded
against protocol churn or a second transport; and ADPT-2's standalone
server may take option 9's Rust binary, with this TS entry point becoming
a thin alias._

_GRPH-4 (2026-07-25, ADR-0029): the phase gate ran first, because it is
the only Phase 2 item that can invalidate an Accepted ADR and the schema
is the expensive thing to move. Its criteria were written and committed
before the harness existed — a spike that fixes its thresholds after
seeing the numbers can only ratify the decision it was commissioned to
test, and ADR-0004 was already Accepted.

**AGE passed the latency gate and failed three of the other four
criteria.** Traversal speed — the thing ADR-0001 and ADR-0004 both flagged
as unproven — is not AGE's problem: 2-hop expansion from a 10-seed set
costs 12.91ms median at 10M edges against a 50ms threshold, with a 1M→10M
slope of 1.58×. What failed was catalog cost (48 relations per tenant's
three graphs → 48,000 at 1,000 tenants, against a 25,000 ceiling), the SQL
discipline (`cypher()` takes its graph name as a *name constant*, so a
per-tenant graph name can only reach the statement as runtime-built text —
which CLAUDE.md forbids and ADR-0001's "enumerate every SQL statement in
the binary" rules out), and the edge write at 10.42ms against a 10ms
bound.

Both failures that matter share one cause and one fix. **ADR-0004 is
amended: named graphs stay, per-tenant instantiation goes** — one shared
entity/episode/provenance set with `tenant_id` as a property and forced
RLS keyed to the TEN-2 GUC, which G6 verified is honoured by Cypher
traversals across two tenants and returns nothing at all when the GUC is
unset. TEN-5 tenant deletion and MEM-6 per-graph decay become predicated
rather than structural; that is the recorded cost. The write failure is
mitigated rather than traded: direct inserts into AGE's label tables run
at 0.01ms against Cypher `CREATE`'s 7.90ms, inside the same transaction,
so ADR-0001's commit-together property is kept.

Three obligations are binding on GRPH-1/2: edges are written as
label-table inserts, never Cypher `CREATE`; the disciplined query forms
are the only ones the code can emit, defended by a test that fails on a
sequential scan over a label table; and the amended tenant-property schema
carries an index on the tenant property. The discipline is load-bearing
because **three of the four ways a competent person would write these
queries are 20×–2000× slower than the one that works**: `IN` lists scan
the whole edge table (211.77ms at 10M, slope 7.09×), `*1..2`
variable-length paths cost 408ms at 1M and 3.7s at 10M where the explicit
two-hop pattern costs 0.43ms, and `WHERE id(x) = …` does not use the
primary key (689ms). Nothing in the query text tells you which one you
wrote.

The fallback ladder was **not** activated and was rewritten anyway: the
embedded engine ADR-0001 and ADR-0004 originally named is no longer
maintained, and no licence-compatible property-graph replacement exists
(the mature engines are GPL or BSL). The ladder is now inside Postgres —
indexed adjacency, then a materialised k-hop closure table on the HIER-1
pattern — with a second engine a last rung needing its own ADR and a
cargo-deny exception. Its one live trigger is a requirement for depth
beyond 2 hops or genuinely variable-length paths, which AGE cannot serve
either.

Recorded and deliberately not resolved here: the relational adjacency
baseline cleared **every** criterion, including both AGE failed, and was
3–8× faster on the traversals themselves (1-hop 1.24ms vs 9.35ms, 2-hop
4.84ms vs 12.91ms at 10M) with 2.5× less storage and 6.7× faster bulk
load. The pre-registered rule reserved ADR-0004's option-4 revival for a
G1/G2 failure, which did not occur, so this gate does not overturn it on
that basis — but GRPH-1's design ADR is where the schema call belongs, and
the burden of proof has moved onto AGE._

_GRPH-1 (2026-07-28, ADR-0043): the graph is indexed adjacency in
Postgres, and **ADR-0004's central technology choice is overturned** —
on the evidence its own gate gathered and handed forward, not on the
trigger that gate reserved. What survives is the part ADR-0004 was
actually right about: the three named graphs, now a discriminator the API
cannot omit. `graph::expand` takes a `Graph` **by value** and a `Depth`
**enum**, so a traversal that does not name its semantic domain does not
compile and one that wants three hops cannot be written — the ADR-0024
discipline ("the only entry point takes a mandatory filter; there is no
unfiltered code path") applied to meaning instead of to tenancy. An edge
is a bitemporal row of exactly the records shape, so "both versions
readable as-of" is a property of the schema: the demo closes a window
with a plain `UPDATE` that never touches the store API and the history
row appears anyway, because the trigger put it there.

**The plan guard is the piece worth describing, because the obvious way
to build it is worthless.** Decision 9 asks the AC suite to explain the
shipped statements and fail on a sequential scan; a test that explains a
*copy* of the SQL proves only that the copy is fast. So each statement
carries a `-- shipped-traversal:` marker, and the test reads
`src/graph.rs` through `include_str!`, extracts the four by that marker,
and explains those — with a second assertion that the number of
`sqlx::query_as!` calls inside `expand` equals the number of markers
found, so a fifth traversal cannot be added without joining the guard.
All four plan as index scans on both legs, on `graph_edges` and on
`graph_edges_history` alike.

**Two findings, both from the fixtures rather than the feature.** First,
the plan guard is *scale-dependent*, which is exactly the trait a plan
guard must not have: below roughly 25,000 edges a sequential scan is the
correct plan for the second hop's join, the planner is right, and the
first version of this test passed only because the shared dev database
happened to hold a million rows from an earlier run. It now seeds 200,000
— an order past the crossover, ~8× margin — and says so in the test, so
the next contributor who sees a seq scan on a nearly empty table knows
what they are looking at. Second, that fixture originally resolved
endpoints by joining the edge series against the vertex table on a
computed key; the planner chose a nested loop and 200,000 edges took **five
and a half minutes**, with a cost that depended on the vertex count. Ids
are now derived arithmetically from the ordinal — no join to plan — and
the whole suite runs in 2.7s.

Decision 15's re-measurement, on the built schema under RLS at 1M edges
and the spike's own shape: **1-hop median 1.17ms, 2-hop 23.4ms**, against
ADR-0029 G1's 50ms median threshold, with p95 1.4ms/25.2ms reported
against the 150ms slice the recall decomposition reserves. The spike's
0.84ms/2.05ms are printed beside them and are **not** a like-for-like
comparison — it measured a directed join projecting one bigint column,
while this expands undirected at both hops and returns whole edge rows,
so the 2-hop shape answers a question roughly four times larger. Most of
that 23.4ms is materialising ~4,000 rows, not finding them: the same
traversal counted rather than projected executes in ~7ms server-side.
**That is a standing note for GRPH-3**, which will decide how much of an
edge it actually needs on the recall path; the slice has 6× headroom
either way.

Deferrals and forward obligations: `MAX_EXPANSION_SEEDS` (64) is a bound
rather than a tuning knob, and the *fan-out* past the frontier is
unbounded by design — GRPH-3 owns ranking, and a cap here would be a
silent truncation this repo refuses. Spans are named `store.graph.*` for
consistency with every other store span rather than the ADR's literal
`graph.expand`; the fields it asks for (seed count, depth, graph) are all
there. `record_supersessions` stays the system of record and the
projection into the edge model is GRPH-2's (ADR-0039 trigger (d) is
discharged as a projection, not a mirror). Nothing above the store reads
the graph yet: GRPH-2 writes the edges, GRPH-3 hands `expand`'s output
into ADR-0042 decision 12's fused id list — **narrowed by `admit`, never
widened by it**, which is the one property that keeps a knowledge graph
from becoming a policy bypass, and EVAL-5's leak suite gains graph paths
explicitly. AGE remains installed for `graph_spike.rs`'s evidence and is
called by nothing (the demo counts the `cypher(` call sites in the
workspace: zero); removing it from the image is OPS-1/OPS-2's, with the
condition named. Direct human authorship or deletion of an edge is
reserved for "a new action, a new grant and a new ADR"; today the app
role holds no DELETE on either table._

_GRPH-2 (2026-07-28, ADR-0044): the graph-linking stage. Linking is a
**step of the extraction commit**, not a pass of its own — it runs after
the record loop and before the channel commit, on the same transaction,
so a record and every claim about it either both land or neither does.
That is ADR-0039's placement for the closed window and ADR-0023's for the
vector, applied to a third kind of derived material, and it means there
is no second exactly-once problem, no second lag SLO, and no window in
which the corpus holds a record the graph has never heard of.

**Resolution is the schema's unique constraint, not a lookup.**
`upsert_vertex` on `(tenant, graph, kind, key)` is insert-or-converge, so
there is no read-then-write race to lose and no "check whether this
entity exists" query anywhere in the stage — which is exactly what
ADR-0043 decision 5 built the key for. The rules are deterministic and
few (casefold, collapse whitespace, strip edge punctuation, strip a
possessive, a leading article, a trailing corporate suffix) and the AC's
number is **pairwise precision 0.973 on the labelled fixture set**
against a 0.95 provisional target, with recall 0.837 reported and not
asserted. The fixture set carries its own failures deliberately: `Paris`
is two different things with one name, which no surface-form resolver can
split, and `PostgreSQL`/`International Business Machines`/`Jorg Muller`
are equivalences it refuses to guess at. A second test pins the false
merges to *exactly* that one pair, so a rule that starts over-merging
fails before the threshold's slack absorbs it.

Two design points are worth carrying forward. **Nothing but a name ever
reaches a vertex.** `graph_vertices` has no scope by ADR-0043 decision
12, so a key or a label is readable by any tenant-scoped read: a
record-backed vertex's key and label are its record id and nothing else,
a name vertex is backed by no record at all (binding it to whichever
record mentioned it first would privilege that record), and a mention
carrying a `[REDACTED:` marker is refused before normalisation. The
rescan gap is closed too — ADR-0022 decision 7 re-scans extractor output,
so where a rescan *changed* a candidate, mentions no longer present in
the persisted text are dropped; the AC observes a real secret through
quarantine and release rather than asserting that in a unit test. And
**a claim's identity is now enforced**: migration 0027's partial unique
index on `(tenant, graph, kind, src, dst) where valid_to is null` makes
`assert_edge` idempotent, so a re-drive writes nothing and reports
`held` — with the predicate partial precisely so supersession's second
half stays legal.

`provenance` is **projected, never written** (ADR-0039's trigger (d),
discharged as ADR-0043 decision 11 specified): `graph::supersession_edges`
reads `record_supersessions` and returns the edge-shaped view keyed by
`RecordId`, and the AC asserts that `graph_edges` holds no `supersedes`
row and the provenance graph holds no vertex. Minting vertices for
records that already exist would have been the mirror the projection
exists to avoid.

Deferrals and forward obligations: the deterministic extractor now fills
`entities` (ruleset `builtin@2`) with a capitalised-run heuristic and a
sentence-opener stoplist — a floor for the network-free path, not the
product path, and a stoplist rather than a position rule so the failure
is data a contributor can extend rather than behaviour they must argue
with. **Orphan rate is a measurement, not an error**: it is counted per
graph on `synveda_graph_link_records_total{graph, outcome}`, because "no
name resolved" and "no usable session id" are facts about different
pipelines. Fuzzy or embedding-backed resolution is deliberately absent
and gated on EVAL-2 producing a corpus bigger than a fixture file; when
it lands it arrives as a new `method` and a confidence band below 900,
not as a change to the existing keys, because a merged vertex carries no
lineage and cannot be unmerged. There is no feature flag: GRPH-3 owns the
degradable half, and an off switch on the *write* would leave a corpus
the graph could never describe without a backfill nobody has specified —
the trigger is the extraction lag histogram, not a preference._

_AUD-2 (2026-07-28, ADR-0045): the audit query surface. Unusually for a
feature this late, most of its design was decided by other features that
needed the answer to exist — ADR-0042 decision 8 drew the line ("CTX-5
answers *what was there*, AUD-2 answers *who could see it*"), ADR-0038
decision 13 put the permitted tier set into `context.injected`, ADR-0041
decision 9 put `tier` on every entry, ADR-0036 sent "why is this pinned"
here rather than build a pin log, and ADR-0019 decision 7 left the
user-or-service join to query time. **No emission point changed**, which
is the useful evidence that ADR-0019 decision 4's one-event-per-operation
discipline was applied with the auditor in mind each time.

**The surface answers from the chain as recorded, never from a replay.**
ADR-0042 option 5 rejected reconstructing historical authority for the
read path *and named AUD-2 as where the question belongs*; the objection
is stronger on this side, where being evidence is the whole value. So
"who could see X on date D" returns **two lists it refuses to merge**:
`disclosed` (who the chain records being served it, with the version,
channel, tier and staleness each reader got) and `authority` (the pack,
bindings, lapses and classifications that governed the window). Merging
them means deciding, and deciding over reconstructed inputs is the replay
again. The reason rides the response body, not only the ADR, so a
consumer cannot read one list as the other. Historical bindings are the
sharpest case: `role_bindings` is a current-state table and an unbound
role leaves no row, so `role.bound` on the chain is the *only* record
that anyone held a role on a given day.

**Tenant-complete or refused, and the schema is the enforcement.**
`AuditRead` declares `resource: [Tenant]` and omits `Scope` entirely, so
a subtree-scoped audit request fails schema validation rather than
relying on a handler to remember. An event's `resource` is a display
string by AUD-1's specification — a scope for some actions, a binding or
a tenant or `"scope none"` for others — so a subtree answer could only be
"the events we could attribute to your subtree", which silently omits the
rest, and silent omission is the one property an audit answer must never
have. Two things fall out free: a subtree-bound auditor holds nothing
(bindings inherit downward, the tenant plane is above every node), and no
service identity can read the trail however bound, because AUTH-3's
confinement forbid already covers everything outside the anchor subtree.
Packs bumped to `@11`, on the read-only admin permit whose comment has
named this feature since AUTHZ-2 — `auditor` stops being a marker row in
the golden matrix.

**Migration 0028 adds indexes and not one column.** `audit_log.hash`
covers a canonical serialisation of the row, so a column *inside* that
form invalidates every row written since AUD-1 and one *outside* it is an
audit field the audit chain does not protect. Four indexes, all
tenant-leading; the disclosure index is `gin (tenant_id, payload
jsonb_path_ops)` through `btree_gin`, **partial** to `context.injected`
and `context.recalled`. Measured on the dev chain at 56k events: 4.0MB
against a 49MB heap, and every query shape plans as an index scan with
disclosure containment at 0.33ms. The cost of the partial predicate is
named rather than discovered — record ids that `memory.superseded`,
`memory.expired` and `vedaflow.channel.published` name are not reachable
by containment, so "everything the chain says about record X" is not a
query this feature ships, only "who was served record X"; widening is a
reviewed diff that rebuilds in place (ADR-0024's rule, restated by
migration 0027).

Deferrals and forward obligations, none of them discovered later:
**(1) ADR-0045 decision 11 is half-discharged** — `synveda audit
verify`/`tail` stay direct-to-store break-glass as designed, but the
`events`/`disclosures`/`knowledge` subcommands through the gateway are
not built; the demo uses `curl` with `DATABASE_URL` unset, which
demonstrates the same property. **(2) The authority half returns raw
events, not a folded state.** Folding needs the `PolicyNodeAssigned`
overload handled — `curators.rs` emits it for curator files and
promotion rules, `policy.rs` for pack assignments, with different payload
shapes — and CNSL-2 is where a console would want the fold. **(3) The
200ms median budget decision 12 pre-registers is unmeasured**: the AC
suite runs on a fixture-sized chain, and nothing yet exercises 1M events.
EVAL-6 owns percentile SLOs, but the median assert is this feature's and
is still owed. **(4) One code path is covered by reasoning rather than by
a test**: `disclosures` computes truncation from the rows the SQL limit
returned rather than from the disclosures they yielded, because a full
page that loses a row to the entry filter is still a full page — no test
produces an event that matches containment without a matching entry,
since doing so would need a hand-forged payload and the suite's rule is
no seeded rows. **(5) Payload shapes are versioned by whatever feature
was live**: an entry written before FLOW-2 carries `version_hash` and no
`object_hash`, one before AUTHZ-5 has no `tier`. Every extracted field is
an `Option` that stays `None` rather than taking a default, and the two
hashes are kept distinct rather than folded — a content address and a
version hash are different claims, and reporting one as the other would
be this surface inventing a fact about the past. **(6) The chain grows on
read**: an allowed admin-plane read chains its own `authz.decision`, so
an estate with heavy audit use will find `AuditRead` decisions a visible
share of the trail, and it is why the pages are cursor-paginated. AUD-3's
WORM export and AUD-4's SIEM stream consume the same reads; CNSL-3
surfaces "what did the agent know at T" over this API rather than over
the store._

_FLOW-1 (2026-07-25, ADR-0030): the object store — the substrate ADR-0003
committed to, and only the substrate. Six `vedaflow_*` tables in migration
0018, all queries in `synveda-vedaflow`, every operation inside the
caller's `begin_tenant_tx` transaction. That is the AUD-1 split (ADR-0019)
and it is what makes ADR-0003's central claim true in code: a commit, the
records it describes, and the audit event attesting to it either all land
or none do.

Both acceptance properties are enforced rather than tested for, which is
the point of the schema work. **Dedup is the primary key**: the address
`(tenant_id, hash)` *is* the key, so a second write of identical content
conflicts with the first and reports `deduplicated` — the property test
pins the row count to the number of distinct (kind, content) pairs,
however many times each was written. **Immutability is grants plus
triggers**: the five history tables give `synveda_app` SELECT and INSERT
only and raise on every UPDATE/DELETE/TRUNCATE, owner included; refs are
the one mutable table and hold no DELETE. What a trigger cannot stop — a
principal who disables triggers, the AUD-1 attacker — `verify` catches by
recomputing every address from the row it is stored under, and the demo
runs that attack and prints both hashes.

Three readings the ADR settles because they are behaviour, not taste.
(1) **The tenant is not in the hash.** It is in the primary key instead.
Putting it in the address would make dedup true only in the trivial sense
and would break the two things a content address is for: an auditor
recomputing it from bytes, and FLOW-8 exporting it. (2) **The asset kind
is** — identical bytes as a prompt and as a skill are two different
objects, because FLOW-3 resolves approvals from asset type and a skill is
executable where a prompt is not. `AssetKind` joined `synveda-types` for
it. (3) **Racing a ref is a result, not an error.** `update_ref` states
the commit it expects to replace and reports `Raced` when it finds
another; the caller re-reads and re-parents. Under 8 concurrent writers
the headline test loses 100-odd compare-and-swaps and drops nothing: a
lost race rolls back its own objects, tree, and commit, so there is no
unreachable garbage either. Fast-forward is the default and
`force_update_ref` is a separate function by name, so FLOW-7's rollback
can never be a typo.

Deferrals, all recorded in ADR-0030: **no audit action, no route, no CLI
verb** — the object store has no product surface until FLOW-2, and
inventing `vedaflow.ref.updated`'s actor, resource, and decision context
ahead of the surface that produces them would be guessing; the counters
this crate emits (`synveda_vedaflow_{objects,trees,commits}_written_total`,
`_ref_updates_total`, `_verifications_total`) are described in the
gateway's recorder when FLOW-2 wires the gateway to this crate. Packing
and GC stay open, as ADR-0003 anticipated. Signing-key management is
configuration-shaped (`Signer::Unsigned` is the default and writes NULL,
because a commit nobody signed should say so); TEN-4's per-tenant keys are
its natural home. The ancestry walk behind the fast-forward check is
O(history) worst case — depth 1 in the overwhelmingly common case — with a
generation number as the recorded upgrade if FLOW-4/5's automated
promotions make ref moves hot. `vedaflow_refs` carries no foreign key to
`hierarchy_nodes`: an `on delete cascade` would be a ref-deletion path
around the withheld DELETE grant, and disposal is TEN-5's._

_FLOW-2 (2026-07-25, ADR-0031): channels — `vedaflow_refs` rows named
`{asset-kind}/{channel}`, materialising on first write, no migration and
no bootstrap. Three transitional stand-ins are discharged here: ADR-0025
decisions 2 and 7 (the `RecordKind` channel stand-in and the version-hash
watermark) and ADR-0022's "FLOW-1/2 replace the direct records insert
with the derived-channel commit".

**Published and staged are sets; derived is a log.** A publish commit's
tree is the channel's entire membership, so "what is published here" is
one indexed read for the whole scope chain. A derived commit's tree holds
only what that commit added, because a full-membership tree per commit
costs one row per record in the corpus on *every* extraction batch. The
asymmetry is safe because derived membership is never enumerated: a
record is derived material unless it is published.

**Publication binds bytes, not ids.** A tree entry names the object
address of exactly the version that was reviewed, and composition
recomputes that address from the record it is about to serve. Edit a
published record and it composes as unreviewed again — the alternative,
membership by id, would let anyone with `memory.write` rewrite text under
a published id and have it serve as reviewed content. It costs one hash
comparison and no extra query.

`RecordKind` goes back to meaning what seed §4.2 says: authored versus
pipeline-derived. **Authorship is not review** — a pinned record nobody
published does not survive bank mode, which is a deliberate behaviour
change and the reason several CTX-2/CTX-3 tests move in this diff.
Channel is tier 0 of conflict resolution (above seed §4.4's list, which
predates channels), published material composes regardless of the task
where pinned material used to, and bank mode *removes* the derived query
rather than filtering its results.

The publish route takes **two** decisions, and the second one decided
what FLOW-3 is for. `ChannelPublish` says who may publish here;
`MemoryRead` says whether they may read what they are about to declare
reviewed. Since the pipeline lands every record at its owner's personal
scope (ADR-0020 decision 3) and the privacy floor (ADR-0015 decision 4)
denies a team curator any read there, **a curator cannot reach into a
teammate's personal scope to publish it** — so the user→team climb tech
plan §2.3 describes has to be a proposal the owner opens, not a
reach-in. FLOW-2 ships no way around that. A user holding a curator
binding can publish their own memories to their own channel, because the
membership floor grants them that read.

Deferrals and forward obligations: retraction has no surface until
FLOW-7's rewind; `staged` has no writer until FLOW-3, so its ref is
genuinely absent rather than manufactured empty; published sets are
capped at 10,000 members per scope with subtree sharding as the recorded
upgrade; MEM-5 and MEM-6 inherit the obligation to re-commit when they
rewrite or close a record, since `valid_to` is inside the address;
FLOW-8's export covers `published` cleanly and would need the snapshot
question reopened for `derived`; records written before this feature have
no derived commit and are simply unpublished material, with no backfill
attempted. Signing stays `Unsigned` — key management is still TEN-4's._

_FLOW-3 (2026-07-25, ADR-0032): proposals and the approval matrix — the
review ADR-0031 wrote its own reversal trigger against ("FLOW-3 landing →
publication moves behind proposals"). It is discharged in the form that
keeps **one** matrix rather than two paths: the direct
`POST /v1/channels/{scope}/publish` resolves the same requirement a
proposal does, with the acting principal counting as the only approver.
A curator publishing internal memory under `regulated-strict` still works
— the matrix asks for one curator and one curator acted — and a
`restricted` record refuses, names the missing role, and points at the
proposal route. The direct route did not become a hole to close; it
became the degenerate case where one approval is enough, which is why
FLOW-2's acceptance walk still passes unchanged.

**A proposal is a commit plus a row.** The commit is the reviewed
content, a tree naming every member at the address of exactly the version
proposed; the row is workflow, and a trigger permits it one transition,
open → closed. It gets no ref: a ref names a moving head, and one ref per
proposal would leave a permanent pointer per closed proposal in a table
that deliberately holds no DELETE grant. That also settles `staged` —
FLOW-3 is not its writer after all, for a reason now known rather than
pending: a set channel cannot express withdrawal, and "what is open here"
is one indexed query that stays correct where a set would drift.

**Approvals bind the commit, and publication rechecks the bytes.**
ADR-0031 decision 5 one layer up: approve, edit, publish is the attack,
and publishing recomputes every address from the record as it stands then
and refuses with a `Conflict` naming the record that moved. The review
surface shows the drift before anyone tries.

**The floor is the `base.cedar` pattern applied to configuration.** Two
rules merge into every matrix — embedded pack, stored pack, or none at
all: anything `restricted` needs `compliance` and two distinct approvers;
any `skill` needs `security-reviewer`. There is deliberately no API that
resolves a matrix without them. `compliance` and `security-reviewer` stop
being marker roles here.

Three readings the ADR settles because they are behaviour, not taste.
(1) **No self-approval ban.** It was the obvious rule and the wrong one:
on the direct route the actor is necessarily the only approver, so a ban
would make the two paths disagree about the same matrix.
`distinct_approvers` expresses separation of duties precisely and
identically at both surfaces, and one person holding two required roles
satisfies both role lines while still counting as one identity.
(2) **The deciding approval does not publish.** Auto-publishing would run
under system authority exactly when a `compliance` reviewer casts the
deciding vote — a role with no publish grant in any pack — and no
spelling of that is not a PDP bypass. `POST /v1/proposals/{id}/publish`
takes the same two decisions the direct route takes.
(3) **`approved` is a rendering, not a stored state.** Requirements
resolve live (a pack switch governs the very next request, ADR-0014
decision 3), so a stored `approved` would need a background re-evaluator
or it would be a lie; the stored column holds only what happened.

**Curator files add requirements and grant nothing.** A per-scope
CODEOWNERS-shaped file, stored as an `AssetKind::Policy` object under a
`curators` ref, resolved nearest-ancestor-first like pack assignment. A
named subject still has to pass `ProposalReview`, so a file naming
someone the pack denies makes a proposal unsatisfiable rather than making
that person an approver — `CompositionConfig`'s "the config never grants"
rule, on the other side of the boundary. Written through `PolicyAssign`
rather than a new action: the steward who can swap the whole pack can
obviously edit the file its matrix composes with.

The publication of a proposal is a **merge commit**, `[channel head,
proposal commit]`, so tech plan §2.5's "every published sentence traces
to an author through an approval" is a fact about the commit graph rather
than a join, and FLOW-8 carries it into a real repository for free.

Deferrals and forward obligations: **there is no revise verb** — a
revision is a new proposal, and the machinery exists (approvals name a
commit) but the surface does not, because a proposal whose content
changes under its approvals is a review nobody consented to. FLOW-3 is
same-scope; **FLOW-5 relaxes `source_scope_id = target_scope_id`** and
inherits the disclosure question a climb raises, which is why
`ProposalRead` is shaped like `MemoryRead` now. `MAX_OPEN_PROPOSALS`
(500 per scope) is the reviewer-DoS bound FLOW-4's rule engine will press
on. The curator file's glob language is one wildcard on purpose and will
need real path semantics when SKIL-1 and PRMT-1 bring path-named
entries — the shape is accepted now so that growth is not a format
change. Rejected and withdrawn proposals leave unreferenced commits,
which is the packing/GC question ADR-0030 left open and does not worsen
in kind. `PackConfig` replaced the three positional config arguments
`policy_packs::apply` and `Pdp::install_source` were growing. Signing
stays `Unsigned`; key management is still TEN-4's._

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

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
until those features extend it: auditor's audit surface (AUD-2), and
security-reviewer's skill approval (SKIL-2). Closed since: contributor
writes (MEM-1, 2026-07-19 — `MemoryWrite` at bound non-personal scopes
for contributor/curator, ADR-0020 decision 3); security-reviewer's first
live action (MEM-2, 2026-07-19 — the quarantine review plane); curator's
"can pin/approve" (FLOW-2, 2026-07-25 — the channel plane, then FLOW-3's
proposal review); and **compliance** (FLOW-3, 2026-07-25 — `ProposalRead`
and `ProposalReview`, with the invariant approval floor requiring the
role on everything `restricted`, ADR-0032 decision 4)._

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
verify/tail` is the operator surface until AUD-2's query API. Forward
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
- [x] [GRPH-4: AGE performance spike / graph fallback assessment](GRPH-4.md) — done 2026-07-25, report: docs/spikes/grph-4-age-traversal.md, criteria + verdict: ADR-0029, harness: crates/synveda-store/tests/graph_spike.rs (`--ignored`), demo: demos/grph-4-graph-spike.sh
- [ ] [AUD-2: Audit query & auditor role surface](AUD-2.md)
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

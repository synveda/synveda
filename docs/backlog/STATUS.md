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
- [ ] [EVAL-1: Eval harness skeleton](EVAL-1.md)

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
until those features extend it: curator approvals (FLOW-3),
security-reviewer (SKIL-2), compliance (AUTHZ-5/FLOW-3), auditor's
audit surface (AUD-2), contributor writes (MEM-1 — closed 2026-07-19:
`MemoryWrite` at bound non-personal scopes for contributor/curator,
ADR-0020 decision 3)._

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

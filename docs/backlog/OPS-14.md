---
title: "OPS-14: Request limits and tenant usage budgets"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-14: Request limits and tenant usage budgets

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

Before the single-gateway slice the public router had no request-frequency
or concurrent-work limiter: `RateLimited` mapped to 429 without enforcement,
and one tenant could saturate the database pool and slow every other tenant.
This is PR-07 in [production readiness](../PRODUCTION_READINESS.md). The
small-team Compose offer runs one gateway; a medium-team Helm offer may need
several gateways and a shared limit.

[ADR-0159](../adr/adr-0159-single-gateway-admission-and-tenant-usage-budgets.md)
records the implemented single-gateway contract:

- Every route-catalogue entry declares one closed cost class (`read`,
  `search`, `session_ingest`, `context_compose`, `mutation`, `bulk`); an
  unclassified route does not compile and an unknown match is refused.
- A pre-authentication layer charges login-plane requests and credential
  failures to the network source and refuses an exhausted source before any
  credential is read. `X-Forwarded-For` is believed only from trusted
  proxies (loopback, private and link-local peers unless
  `SYNVEDA_TRUSTED_PROXIES` says otherwise); when every hop is trusted, the
  hop nearest the proxy names the client, so private clients keep separate
  budgets behind a proxy that forwards the client address. IPv6 is keyed by
  /64. The bundled Compose proxy does not forward it yet, so Compose clients
  share the unattributed source budget.
- After credential verification and before the tenant lookup, each request
  is charged to its verified principal and tenant per class and then takes
  bounded principal, tenant and process concurrency with one deadline and a
  waiting-room bound. Permits last until the response is built and are
  released on success, error and cancellation.
- Identity tables are hard-capped; rested idle entries are forgotten and
  overflow loses only that level's fairness. Refusals share one
  `rate_limited` body with `Retry-After` 1–60 s. `SYNVEDA_ADMISSION_MODE=observe`
  is the rollback.
- Additive migration `0017` (byte-identical in both lineages, measured
  catalogue fingerprints, regenerated SQLx metadata) adds forced-RLS
  `tenant_usage` and `tenant_usage_budgets`. Stored Session event payload
  bytes and created ContextRuns are charged by one conditional upsert at the
  end of the accepting transaction, per database UTC day; duplicates,
  replays, refusals and rollbacks charge nothing.
- `GET /v1/usage` (`UsageRead`) and expiring, narrowing-only
  `PUT|DELETE /v1/usage/budgets/{meter}` (`UsageBudgetManage`, content-free
  `tenant.usage.budget_changed` audit) are tenant-scoped; the shipped packs
  grant them to tenant `owner`/`administrator` and refuse service tokens.
- Aggregate metrics carry only stage, class, outcome, meter and table labels.
  Helm exposes `gateway.admission.{mode,trustedProxies,limits}`.

Evidence is listed under [the single-gateway checkpoint](#single-gateway-checkpoint-2026-10-27).

### Single-gateway checkpoint 2026-10-27

Source tree on top of `479fa3ff` (not a release). Exact-role fixtures:

- `crates/synveda-gateway/tests/request_limits.rs` (8 cases): alice's
  ContextRun principal budget refuses her third run while bob still runs and
  then spends the tenant budget; both refusals are byte-identical 429s with
  `Retry-After`; another tenant and another class are unaffected; refused
  runs create neither a run nor usage. The same limiter over an unreachable
  database still answers 429 for an over-budget caller, while an admitted
  caller reaches the database and fails as storage, so admission precedes
  database work. Garbage, malformed and wrong-key tokens spend only their
  source and allocate no principal or tenant state; once spent, a valid token
  from that source is refused identically; 256 distinct failing sources stay
  within a 64-entry cap. A request whose body never completes holds its
  permits, a second caller waits 300 ms and is refused, and aborting the first
  releases them. Duplicate batches, a concurrent identical batch, a replayed
  key and four racing creations with one key charge once; a 403 and a 409
  charge nothing. A narrowing override refuses the next run with
  `Retry-After: 60`; a zero byte budget refuses an append and stores
  nothing; clearing it resumes. Members, unplaced principals and bad inputs
  are refused; another tenant's override never applies; changes and PDP
  denials are audited without event content. Observe mode admits and
  records usage past every budget; a new limiter (a restart) resets hot
  budgets while usage and overrides persist.
- `crates/synveda-store/tests/usage.rs` (4 cases): charges land only within
  the effective limit, overrides only narrow and stop at expiry, 24
  concurrent charges against a limit of 16 land exactly 16, and the runtime
  role can neither see another tenant's usage nor delete its own.
- `crates/synveda-store/tests/epoch.rs` adds published and portable
  `0016`-to-`0017` upgrades. Measured fingerprints: ACL 451 rows (+11 grants),
  forced-RLS inventory 139 rows (+2 tables), routines and triggers unchanged.
  `EXPLAIN` keeps the charge on the primary-key arbiter and the history read
  on one primary-key range.
- 37 limiter unit tests use Tokio's paused clock; `claude_lifecycle` now runs
  authentic hooks under the enforced defaults and passed.
- The full exact-role gate passed on the final tree: 134 binaries, 2,204
  passed, 0 failed, 37 ignored, serial phases executed. An earlier run caught
  two defects fixed before it: a catalogued path requested with an
  undeclared method returned 503 instead of 405, and the forced-RLS coverage
  list lacked the new tables. Two deployment test cases fail identically on
  an untouched `479fa3ff` worktree on this host and are recorded, not caused,
  here.
- The deployment check found that, with the default trusted networks, a
  private client's own forwarded address was skipped as another proxy, so
  every private client behind a proxy shared the unattributed budget. The hop
  nearest the proxy now names the client when every hop is trusted; two
  limiter tests fail on the old rule and pass on the fix.
- On the pinned Caddy 2.11.4 with the shipped Caddyfile and development
  sites, two client containers, forged headers and the host all reached echo
  upstreams for the gateway and Keycloak with no `X-Forwarded-For`, `-Host`,
  `-Proto` or `-Port`: the shared upstream snippet sets them, and its
  `X-Forwarded-*` deletion runs after the sets. Canonical Compose clients
  therefore share the unattributed source budget. A site-level strip
  delivered one hop naming each client and removed forged headers in the same
  drill; it awaits a live Compose acceptance run. Canonical Compose could not
  start on this host because its hostname ownership receipt is refused.
- `SYNVEDA_DB_TEST_TASK=postgres-14` and `postgres-18` passed on the final
  tree with the official 14.24 and 18.6 images; 15 and 16 were not rerun for
  this additive migration.

`crates/synveda-gateway/tests/admission_load.rs` (a manual, ignored
measurement: `SYNVEDA_ADMISSION_LOAD_SECS=20 bash scripts/db-test.sh -p
synveda-gateway --test admission_load -- --ignored --nocapture`, so a shared
CI runner cannot turn timing into a red gate) ran four paced agents in
one tenant (an append and a ContextRun every 600 ms each) against 24 clients
in another tenant that ignore `Retry-After`, through one debug-build gateway
with an eight-connection pool, on one macOS ARM64/OrbStack host. Two paired
rounds against a worktree of `479fa3ff` (identical harness, no limiter):

| Phase | Measured append p99, before → after | Measured ContextRun p99 | Pool saturated |
| --- | --- | --- | --- |
| Noisy | 155–213 → 49–50 ms | 387–404 → 184–264 ms | 100% → 0.7% |
| Noisy, database CPU load | 227–240 → 71–107 ms | 487–492 → 193–242 ms | 99.8–100% → 0.4–0.9% |
| Noisy, stalled provider | 246–247 → 67 ms | 444–452 → 398–421 ms | 82–83% → 0.3–0.4% |

The measured tenant received only 2xx in every phase; the flooding tenant
was held to 34–44 admitted requests per second (103–136 without limits) and
refused 0.8–1.1 million times per 20-second phase without a database round
trip. `GET /v1/me` (2,000 requests) showed no detectable change: p50
13.4–14.6 ms and p99 18.3–20.9 ms against 13.6 ms and 18.7–20.6 ms. With
nothing else running, the measured tenant's quiet tail rose (append p95
70–79 ms against 33–39 ms) while p50 did not; the same tree in `observe`
mode, which runs every admission and charge but never queues, matched the
baseline, so that rise is bounded queueing at the tenant caps when four
lockstep agents start together. A Session append adds one statement and a
delivered ContextRun two; other routes add none. These are local debug-build
figures, not a service objective. Details are in
[the source acceptance record](../../demos/evidence/ops14-request-limits-source-acceptance.json).

## Scope

- Define a closed set of admitted operation classes and independent tenant,
  principal and pre-authentication abuse budgets. Bound request frequency and
  in-flight work before expensive database/provider calls while preserving
  a stable, non-oracular 429 response and retry guidance.
- Account for durable tenant usage separately from hot request metrics,
  starting with Session event bytes/rate and ContextRun concurrency. Charge
  only accepted work and make replay/idempotency accounting explicit.
- Keep aggregate limiter-denial, saturation and wait telemetry free of tenant,
  principal, token or resource labels. Provide an authorised tenant-scoped
  usage view through the normal PDP/RLS path.
- Define a bounded operator change/incident override as a governed,
  content-free-audited mutation. Configuration may narrow budgets but never
  bypass Cedar, forced RLS or audit.
- Qualify one-gateway limits first, then select and test a distributed limiter
  store before making any multi-replica fairness claim.

## Non-goals

- No billing, plan-tier or pricing product; usage evidence is not an invoice.
- No limit response that exposes another tenant's counts or denied resource
  existence.
- No in-process counter presented as a cluster-wide quota, and no unbounded
  per-identity Prometheus labels.
- No rate limit that authorises a request Cedar would deny.

## Architecture seam

Admission follows verified credential and tenant resolution for authenticated
routes, before costly work. Separate pre-authentication protection must avoid
using unverified tenant claims as keys. A bounded concurrency permit covers
the entire governed application future and is released on cancellation. The
durable usage ledger belongs in `synveda-store` behind static SQLx and forced
RLS; quota mutations use the public API, embedded PDP and content-free audit.
ADR-0159 fixes the single-gateway state (process memory, reset on restart),
the accounting clock (database UTC day) and narrowing-only, expiring
overrides outside VedaFlow. A multi-gateway topology needs a new decision on
distributed limiter storage and its failure behaviour.

## Acceptance criteria

- Adversarial mixed-tenant load cannot make one tenant or principal exhaust
  another's admitted request or concurrency budget on the qualified topology.
  A legitimate caller receives stable 429 envelopes and bounded retry advice.
- Rate and concurrency counters remain bounded in memory/storage and recover
  safely after restart. A limiter-store outage has a declared fail-closed or
  conservative-degrade behavior; it never weakens authorization.
- Accepted Session/Context work charges exactly once under replay and
  cancellation; denied attempts do not create a cross-tenant count oracle.
- Tenant usage and operator budget changes are Cedar/RLS-scoped, auditable and
  do not expose bearer or content material.
- A two- or three-gateway test proves cluster-wide limits before Helm replica
  settings are lifted; the one-gateway result is labelled narrowly.

## Required tests

- Exact-role request and tenant-isolation tests for independent budgets,
  idempotent replay, cancellation, expiry and 429 envelopes.
- Mixed public-API load at the EVAL-6 declared agent/event/context envelope,
  including one noisy tenant and provider/database slowdown.
- Multi-gateway restart and limiter-store fault injection after its backend is
  selected; cardinality check for every metric label.
- Governed quota-change and emergency-override audit tests, including a
  foreign-tenant refusal.

## Rollout and rollback

Observe mode and enforcement share one code path; enforcement is the default
for the one-gateway reference after measuring legitimate traffic. The audited
override narrows a tenant meter and expires. Keep Helm pinned to one gateway
until a distributed backend and cross-pod tests pass. Rollback is
`SYNVEDA_ADMISSION_MODE=observe` (Helm `gateway.admission.mode`), which keeps
usage and audit records and cannot bypass PDP, RLS or governed mutation
rules; `0017` is forward-only and a binary rollback is ADR-0156's restore
path.

## Dependencies

The product owner must adopt or replace the provisional class budgets,
source budget, identity caps and daily ceilings in ADR-0159, decide whether
any budget raise beyond the deployment ceiling is wanted and, if so, whether
it needs VedaFlow review rather than a gateway setting, and decide whether
`UsageRead` should reach roles beyond `owner`/`administrator`. EVAL-6 still
owns the declared agent/event/context envelope; this slice measured a
synthetic same-machine workload. The deployment owner must select a
distributed limiter store and its failure contract before OPS-7 lifts the
one-gateway pin.

Next: owner adoption of the provisional values, an EVAL-6 envelope run
against the enforced defaults, then a distributed limiter decision with
two- or three-gateway fault-injection evidence.

# ADR-0159: Single-gateway request admission and durable tenant usage budgets

- **Status**: Accepted. The admission layers, durable meters and governed
  overrides are source-, exact-role- and same-machine load-tested on one
  gateway process; the numeric values below are a conservative proposal
  until the owner adopts them. Multi-gateway fairness is not decided here
- **Date**: 2026-10-27
- **Feature(s)**: Request limits and tenant usage budgets
- **Deciders**: Owner-authorized implementation

## Context

The gateway verified credentials, resolved the tenant and applied embedded
Cedar decisions, but nothing bounded how often or how concurrently a caller
could start work. `Error::RateLimited` mapped to 429 without any limiter
behind it. Payload bounds limited one request, not a stream of them. A
same-machine measurement before this decision (one gateway, the reference
eight-connection pool, 24 concurrent clients in one tenant) raised another
tenant's Session event append p99 from 10.6 ms to 231 ms and its ContextRun p99
from 58 ms to 487 ms, cut its throughput sevenfold and held the pool saturated
99.7% of the time.

The supported first offer runs one gateway and one combined worker. No shared
limiter store is qualified, and the product deliberately runs no Redis or
second coordination service. PostgreSQL is the only durable authority.
Durable usage must survive restart and replay without double charging, while
hot admission counters must stay off the database and free of per-tenant
metric labels.

## Decision

1. **Closed operation classes from the executable catalogue.** Every entry in
   the route catalogue declares one of `read`, `search`, `session_ingest`,
   `context_compose`, `mutation` or `bulk` (`GET handler as Read`); the macro
   generates the class table from the same declaration as the router, so an
   unclassified route does not compile and a unit test proves every
   operation, including Axum's `{*wildcard}` spelling and implicit `HEAD`,
   resolves. A catalogued path requested with a method it does not declare
   runs no handler, so it keeps the router's 405 without admission; any other
   unclassified match is refused with 503.

2. **Pre-authentication source budget.** A layer outside tenant resolution
   charges one unit per `/auth/*` request and one per 401 on `/v1/*` and
   SCIM to the request's network source, and refuses a source whose budget is
   spent before any credential is read, so valid and invalid credentials from
   it get the same 429. The source is the socket peer. `X-Forwarded-For` is
   read only when the peer is a trusted proxy, taking the nearest untrusted
   hop and examining at most sixteen. Private clients share the trusted
   networks with proxies, so when every hop is trusted the hop nearest the
   peer is the client (the trusted peer wrote it). A request without a peer
   address, or from a trusted proxy that names no readable hop within
   sixteen, shares one install-wide unattributed budget. Trusted proxies
   default to loopback, private and link-local networks, so a proxy or
   ingress that forwards the client address attributes each client, public
   or private, without configuration, and a public peer can never choose
   its own source. `SYNVEDA_TRUSTED_PROXIES` replaces that list and an
   empty environment value trusts no proxy (the chart can narrow the list
   but not empty it). A client that connects directly from a trusted
   network can name any source; behind a proxy that appends to a
   client-written header, a client on a trusted network can name a public
   hop as its source; and behind several private proxies every private
   client collapses onto the proxy next to the peer. Naming only the
   proxies' own networks removes all three. A forged header can choose
   only a source key, never authority. IPv6 sources are keyed by /64.
   Successful authenticated requests spend nothing here. The login plane
   also has a process concurrency bound with a two-second wait. Static
   console assets and redirects are not screened.

3. **Verified-identity admission before database work.** After the
   credential verifies and before the active-tenant lookup or any handler,
   the request is charged to its verified principal (a digest of tenant and
   subject) and tenant for its class, under one lock so neither is charged
   for a request the other refuses. It then takes bounded concurrency at the
   principal, tenant and process levels, in that order, so a queue forms at
   the narrowest full level and never holds a wider level's slot while it
   waits. Waiting shares one per-class deadline and a per-class waiting-room
   bound; past either, the request is refused. Concurrency is bounded per
   class only, so one tenant's six class limits add up to more than the
   reference pool. Keys never come from an unverified token. Configuration
   validation requires `principal ≤ tenant < process` concurrency and
   principal rates at most the tenant's, so one tenant cannot fill a
   class's process share.

4. **Permits for the whole admitted future.** Tokio semaphore permits live in
   the tenant-resolution middleware until the handler's response is built,
   covering request-body extraction, policy decisions, database work and
   response rendering. Success, error, cancellation and the existing policy
   freshness cancellation all drop them. No handler streams a response body
   today; a future streaming handler must carry the permits in its body.

5. **Bounded identity memory.** Principal (16,384), tenant (4,096) and source
   (16,384) tables have hard caps. An entry whose rate cells are rested and
   whose concurrency slots nobody holds or awaits is forgotten without loss.
   A full table is swept at most once per second. A principal or tenant that
   still finds no room is admitted without that level's budget (its other
   levels still apply); a source that finds no room shares the unattributed
   budget. Malformed or foreign credentials cannot allocate identity state.

6. **One non-oracular refusal.** Every limiter refusal is the existing
   `{"kind":"rate_limited"}` envelope with one fixed message and a
   `Retry-After` of 1 to 60 seconds: the exact wait for a rate refusal,
   rounded up, and one second for a concurrency refusal. Other 429s from the
   taxonomy carry 60 seconds. Neither names the level, another tenant or a
   resource. Admission follows authentication (401 stays 401) and precedes
   Cedar, so an over-budget caller receives 429 for a resource it may not
   see, and an under-budget caller still receives the existing 403/404.

7. **Observe or enforce.** `SYNVEDA_ADMISSION_MODE=observe` admits everything,
   counts what enforcement would refuse and records durable usage without
   refusing it; `enforce` is the default. This is the incident rollback for
   admission; it removes no usage or audit evidence.

8. **Durable tenant usage.** Additive migration `0017` creates
   `tenant_usage(tenant_id, meter, usage_day, units)` and
   `tenant_usage_budgets`, both with forced tenant RLS, appended
   byte-identically to both lineages. The meters are `session_event_bytes`
   (the compact JSON length of each newly stored, post-redaction payload, the
   measure that bounds one payload) and `context_runs` (created runs; previews
   and replays are not runs). The accounting day is the database's UTC date.
   One conditional upsert, the last statement of the transaction that accepts
   the work and after its audit event, adds the units only if the effective
   limit admits them; otherwise the transaction rolls back with the work, its
   audit event and its idempotency record. Duplicates, replays, refusals and
   failures therefore charge nothing, concurrent completions serialise on
   the day's row, and the lock order is always audit head, then usage day.
   A ContextRun reads its standing in the authorization transaction before
   embedding or retrieval and refuses early; that check is advisory and the
   charge decides. No durable reservation exists: the in-process permit is
   the only reservation, so a crash cannot strand one. The runtime role may
   insert a day's row and add to its units but cannot delete it.

9. **Expiring narrowing overrides.** The deployment ceiling per tenant and
   day comes from configuration (provisionally 1 GiB of event bytes and
   100,000 ContextRuns). `PUT /v1/usage/budgets/{meter}` sets an override
   with a daily limit (zero pauses the meter), a mandatory reason and a hard
   expiry of one minute to ninety days; `DELETE` clears it. Charging applies
   `least(override, ceiling)`, so an override can never raise a budget.
   Changing one requires the new tenant-only Cedar action `UsageBudgetManage`;
   `GET /v1/usage` requires `UsageRead`. The shipped packs grant both to the
   tenant-wide `owner` and `administrator` keys, and service-token
   confinement refuses both. Each change records one content-free
   `tenant.usage.budget_changed` event with the meter, limits, expiries and
   reason. Overrides are administrative state like grants, not VedaFlow
   artifacts; they narrow and expire, and the governed admin-plane decision
   owns any change to that boundary.

10. **Aggregate telemetry only.** `synveda_admission_decisions_total{stage,
    class, outcome}`, `synveda_admission_wait_seconds{class}`,
    `synveda_admission_in_flight{class}`, `synveda_admission_capacity{class}`,
    `synveda_admission_identities{table}`, `synveda_admission_enforced`,
    `synveda_tenant_usage_charges_total{meter, outcome}` and
    `synveda_tenant_usage_requests_total{op, outcome}` use closed label
    vocabularies. No tenant, principal, address, token, route or resource
    label exists. Per-tenant detail is the authorised usage view.

11. **One gateway only.** All hot state is process memory: it resets when the
    process restarts and is not shared with any other gateway. Durable usage
    and overrides survive restart. Running several gateways multiplies the
    hot budgets by the replica count; the chart keeps one gateway, and no
    cluster-wide limit is claimed until the horizontal-scale work selects
    and qualifies a shared mechanism.

The provisional per-class rates, bursts, concurrency, waits and identity
caps are listed in the
[deployment contract](../DEPLOYMENT_CONTRACT.md#request-admission-and-tenant-usage).

## Options considered

1. **A maintained keyed limiter crate (for example `governor`).** It
   supplies the rate arithmetic but its keyed store grows without a hard
   identity cap and offers no lossless eviction tied to in-flight work, which
   is the requirement that matters here; it would add six crates for about
   twenty lines of arithmetic. The rate cell is a small pure function over an
   injected clock; Tokio supplies every semaphore and timeout.
2. **Tower `RateLimit` / `ConcurrencyLimit` layers.** Process-global only; no
   per-tenant or per-principal fairness. Rejected.
3. **Redis or another shared store now.** Adds a mandatory service to the
   one-gateway offer without a qualified multi-gateway topology to justify
   it. Deferred to the horizontal-scale decision.
4. **Counting usage from stored rows on demand.** Unbounded scans on the hot
   path, and erasing source events would erase usage. Rejected for a daily
   counter charged in the accepting transaction.
5. **Durable reservations before expensive work.** A crash strands them and
   needs a sweeper; the in-process permit already bounds concurrent work.
   Rejected.
6. **Budget overrides as governed Configuration.** Configuration resolves per
   resource scope, so a project binding could contradict a tenant budget,
   and it would put a lookup on every request. Rejected for a tenant-wide,
   expiring, narrowing-only override.
7. **Immediate refusal at a full principal or tenant level.** Turns ordinary
   bursts (console page loads, several agents starting at once) into 429s
   and invites immediate retries. Rejected for a short bounded queue.

## Consequences

- Positive: one tenant's flood is refused at its own budget instead of
  queueing on the shared pool; refused requests cost a signature check, not
  a database round trip; accepted work is charged exactly once; operators
  can pause a runaway tenant meter with an audited, expiring override.
- Negative: every authenticated request takes one short process mutex and up
  to three semaphore permits; a Session append adds one statement and a
  ContextRun two (standing and charge) to existing transactions. A request
  over budget at the tenant level is refused for every principal in that
  tenant. Overflowed identities lose that level's fairness. Console-cookie
  requests still read the cookie row before verification. SCIM is screened
  only before authentication.
- Rollback: `observe` mode disables refusal without losing evidence. The
  schema is forward-only; rolling back the binary is ADR-0156's restore path,
  and an older binary refuses the `0017` head.
- Reversal trigger: owner-selected values differ, measured legitimate traffic
  is refused, or the horizontal-scale work runs more than one gateway; that
  work must choose a shared store and failure behaviour in a new decision.

## Compliance notes

Admission never authorises: Cedar still decides every operation, forced RLS
still confines every row, and an override can only lower a budget. Hot keys
come only from verified claims. Usage rows are tenant-RLS, content-free and
undeletable by the runtime role; tenant archive and erasure include both new
tables. The usage view and override changes are tenant-scoped through the
token, refused to service tokens and audited without content.

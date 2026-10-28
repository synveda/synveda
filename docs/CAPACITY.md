# Capacity envelope

This page states the workload one Synveda installation is measured to carry,
how it was measured, what limits it and when to scale up. It is a **planning
proposal**: the workload and budgets below are a labelled proposal pending
owner approval, and every figure applies only to the recorded hardware,
build and database. [Production readiness](PRODUCTION_READINESS.md) owns the
readiness verdict; this page is its capacity evidence.

## The proposed first-offer envelope

| Axis | Proposed envelope |
| --- | --- |
| Deployment | The canonical single-host shape: one gateway and one core worker from one product build, 8 database connections each (the product default), bundled PostgreSQL 17.11 with pgvector 0.8.7 at image defaults, Keycloak OIDC. No replica of either process. |
| Tenants | One working tenant plus a second, independently loaded tenant on the same installation. |
| Concurrent agents | 24 agents in the working tenant (for example ten people running two or three agents each) and 6 in the second tenant. Headcount alone is not a capacity parameter; agents and their call rates are. |
| Session events | 12 appends per agent per minute (4.8 per second per tenant), 1.2 KB typical, 12 KB for 5% of events; a Session ends after 120 events (about ten minutes) and is frozen for Capture. About 0.72 GB of event payload per tenant per day, under the 1 GiB daily usage ceiling. |
| Context and search | One ContextRun per agent per minute; one Knowledge search per agent every four minutes. |
| Governed mutations | One private Knowledge item per agent every ten minutes and one edit every twenty, each through two named approvals and a distinct publisher; accepted Capture candidates take the same path. |
| Knowledge corpus | Proposed: up to 10,000 current items per tenant (about 15,000 revisions with history), 30 projects, with the context planner's admissible scopes holding about 13% of items; at most 0.25% of items behind a withdrawn source. Measured: with a shared corpus, ContextRun p50 is 0.7 s at 1,000 items and 1.3 s at 10,000, far outside the ContextRun budget, which the 1x workload's small private corpora met only in part. |
| Embeddings | BGE-M3, 1024 dimensions, through TEI. Provider latency is measured separately; the envelope assumes a provider answering a single query embedding inside the gateway's 100 ms deadline. |
| Retention | Product defaults; storage growth is reported per hour so an operator can size retention. |

## Proposed budgets at 1x

| Signal | Budget |
| --- | --- |
| Session event append | p95 ≤ 50 ms, p99 ≤ 100 ms |
| ContextRun | p95 ≤ 100 ms, p99 ≤ 150 ms (the seed's engineering budget) |
| Knowledge search | p99 ≤ 1,000 ms |
| Unexpected outcomes | ≤ 1 per 1,000 requests (5xx, timeouts, transport errors, or a refusal the workload did not expect) |
| Isolation | zero cross-tenant reads |

Budgets are assessed in steady 1x phases, with the second tenant allowed to
run hard. Burst, provider-fault, database-fault, recovery and 10x phases are
reported, not assessed.

## How it is measured

Three instruments keep capacity, retrieval correctness, permission filtering
and provider latency separately attributable:

1. **Dense retrieval plans** (`demos/ctx-7-dense-plans.sh`,
   [ADR-0160](adr/adr-0160-stable-dense-plans-and-cached-cleanup-authority.md)).
   Two tenants of 10,000 and 50,000 items share the one HNSW index;
   clustered 1024-dimension vectors, revisions with history, archived and
   stale heads and withdrawn session sources. Exact ground truth, recall,
   plan class at executions 1 and 6, and per-shape latency of the dense
   statement alone.
2. **ContextRun and Knowledge search at corpus scale** (the same demo's
   `context` step). The full public request path — authentication, Cedar,
   lexical and dense legs, selection, rendering and persistence — over the
   same corpora, one client, with a loopback embedder that maps a query's
   topic words to the corpus geometry.
3. **Mixed workload and faults** ([`synveda-eval load`](../evals/load/README.md)).
   Agents in two tenants through the public API with reviewed governance,
   Capture, cross-tenant refusal probes and stale reviewed edits, through
   baseline, burst, noisy-tenant, provider slowdown/stall/outage, database
   CPU contention, recovery and sustained phases; 10x as 240 agents.

Latency is measured from each call's scheduled time, so time spent waiting
behind the agent's previous call is charged, not omitted; service time and
generator lag are reported separately. Server-side attribution comes from
the gateway's own histograms (HTTP time by route, admission wait, pool
acquire, ContextRun planner stages) and the worker's (Capture queue wait,
extractor time, Knowledge indexing lag); provider time from the stub
provider's own service histogram.

## Dense retrieval at corpus scale

The dense leg is an exact, item-driven scan: no plan in any measured shape
used the HNSW index, recall@10 and recall@limit were 1.0, and each statement
plans with its arguments on every execution
([ADR-0160](adr/adr-0160-stable-dense-plans-and-cached-cleanup-authority.md)).
Its cost therefore grows with the rows inside the admissible scope set.
Latency of the dense statement alone, p50 / p99 over 32 executions, one M5 Max
host, PostgreSQL 17.11 and pgvector 0.8.7 at fixture defaults
([evidence](../demos/evidence/ctx7-dense-plan-stability.json)):

| Shape (limit) | Rows in scope | 10,000 items | 50,000 items |
| --- | --- | --- | --- |
| Context planner scope chain (96) | about 13% | 55 / 97 ms | 268 / 273 ms |
| One project (96) | about 3% | 13 / 14 ms | 66 / 68 ms |
| Knowledge search, no scope allowlist (1,000) | 100% | 403 / 411 ms | 2,095 / 2,110 ms |
| Planner chain as known at an earlier instant (96) | about 13% of history | 59 / 61 ms | 293 / 297 ms |

The per-row cost is dominated by the restrictive source-eligibility policies
(about 20 µs per row per policy on this host after migration 0018; before it,
each row behind a withdrawn source added about 35 ms).

### Full ContextRuns over the same corpora

The planner does much more than the dense leg: lexical and dense candidate
generation, hydration, freshness and conflict checks, Cedar decisions,
bounded graph expansion, selection and rendering, and persistence. One
client composing ContextRuns in a Session whose scope chain admits about 13%
of the tenant's items, through the public router built with the optimised
`ingest-load` profile, 60 runs per size after 5 warm-up runs:

| Tenant items | ContextRun p50 / p95 / p99 | Knowledge search p50 / p95 / p99 | Mean planner stage time (retrieve / select / graph / authorize) |
| --- | --- | --- | --- |
| 1,000 | 708 / 859 / 1,248 ms | 192 / 222 / 944 ms | 397 / 292 / 30 / 8 ms |
| 3,000 | 831 / 1,003 / 1,234 ms | 316 / 338 / 347 ms | 491 / 306 / 25 / 6 ms |
| 10,000 | 1,320 / 8,054 / 10,662 ms | 864 / 1,914 / 3,455 ms | 1,502 / 810 / 47 / 10 ms |
| 50,000 | 2,708 / 2,913 / 2,980 ms | 3,852 / 4,041 / 4,158 ms | 2,243 / 316 / 22 / 5 ms |

Once a Session's admissible Knowledge fills the planner's candidate caps
(96 per leg), each ContextRun costs about 0.7–0.8 s on this host, almost all
of it in candidate retrieval and selection; Cedar decisions stay under
10 ms. Beyond that, retrieval grows with the tenant's rows (2.2 s at 50,000
items) while selection stays near 0.3 s. The 10,000-item tail is not
explained by the measurement; intermittent background work on the freshly
seeded tables is the likely cause, and the 50,000-item run had no such
tail. In the 1x load runs the measured tenant held about 490 Knowledge
items in all, created privately by 24 agents, so each Session likely
admitted a few dozen at most, which would explain why their ContextRuns
were an order of magnitude faster. Even those small corpora met the
ContextRun budgets only in part (below); with a shared corpus ContextRun
p95 is 0.86–1.0 s at 1,000–3,000 items, about nine to ten times the p95
budget. Reducing per-candidate work is the first lever for a larger shared
corpus.

## Mixed workload at 1x

`first-offer-1x`, release builds, one gateway and one worker with 8 database
connections each, PostgreSQL 17.11 in the fixture container, the stub
provider, on one Apple M5 Max host (18 cores, 36 GB; OrbStack 18 vCPU,
16 GiB) on AC power with sleep prevented. Measured-tenant latency from each
call's scheduled time, p50 / p95 / p99 ms (samples)
([evidence](../demos/evidence/eval6-load-capacity.json)):

| Phase | Load | Append p50/p95/p99 ms (n) | ContextRun p50/p95/p99 ms (n) | Knowledge search p50/p95/p99 ms (n) | Unexpected | Assessed |
| --- | --- | --- | --- | --- | --- | --- |
| startup_warmup | 1x (warm-up) | 18 / 30 / 56 (597) | 38 / 61 / 66 (48) | 26 / 45 / 45 (14) | 0 | no |
| baseline | 1x | 17 / 28 / 55 (2860) | 47 / 86 / 179 (264) | 26 / 38 / 42 (57) | 0 | yes |
| burst | 3x | 14 / 23 / 35 (1707) | 66 / 88 / 110 (165) | 24 / 38 / 108 (45) | 0 | no |
| burst_recovery | 1x (warm-up) | 21 / 373 / 2863 (859) | 84 / 1744 / 4059 (56) | 44 / 2673 / 2773 (21) | 0 (+5 limited) | no |
| noisy_neighbour | 1x, second tenant 20x | 13 / 512 / 4118 (2862) | 107 / 2679 / 7360 (227) | 30 / 782 / 7778 (50) | 2 (+36 limited) | yes |
| provider_slow | 1x, provider +60 ms | 15 / 38 / 199 (1470) | 192 / 426 / 477 (126) | 93 / 102 / 108 (27) | 0 | no |
| provider_stall | 1x, provider +2000 ms | 14 / 34 / 233 (818) | 219 / 454 / 644 (73) | 124 / 207 / 207 (15) | 0 | no |
| provider_outage | 1x, provider 100% 503 | 14 / 26 / 54 (882) | 106 / 189 / 602 (66) | 24 / 40 / 40 (15) | 0 | no |
| database_contention | 1x, database CPU contention | 18 / 31 / 130 (1472) | 182 / 288 / 351 (133) | 41 / 79 / 103 (24) | 0 | no |
| fault_recovery | 1x (warm-up) | 14 / 25 / 61 (1429) | 127 / 156 / 170 (128) | 36 / 76 / 106 (28) | 0 | no |
| sustained | 1x | 15 / 25 / 55 (5864) | 150 / 196 / 229 (475) | 43 / 62 / 84 (125) | 0 | yes |

Against the proposed budgets: in this run the baseline phase meets every
budget except ContextRun p99 (179 ms against 150 ms); the sustained phase at the end of
the run misses both ContextRun budgets (p95 196, p99 229 ms) while appends
and search stay well inside theirs; the noisy-tenant phase misses all
latency budgets. Isolation held in every phase: 236 cross-tenant Session
reads were all refused. Unexpected outcomes were zero outside the
noisy-tenant phase (2 in 3,835 there); admission refused 36 measured
calls there and 5 during burst recovery.

What the run shows:

- **ContextRun cost grows with accumulated data.** The planner's `retrieve`
  stage rose from about 15 ms to about 95 ms mean over the run, with
  `select` and `graph_expand` following, while `authorize` (Cedar) stayed at
  about 8 ms and `embed` under 1 ms outside provider faults. In the
  sustained phase alone ContextRun p50 rose from 139 to 157 ms in 18
  minutes as pending Capture candidates grew from 24 to 29 MB, Session
  events to 82 MB and Knowledge revisions to 4 MB installation-wide. Which
  accumulation dominates is not yet isolated; the soak must answer it.
- **One noisy tenant can take the pool.** With the second tenant at 20
  times its rate, pool acquire p99 reached 1.7 s and admission waits about
  2 s, and the measured tenant's ContextRun p99 reached 7.4 s (0.95 s in
  the earlier run below). Per-tenant class limits sum above the
  8-connection pool, as the deployment contract warns; the default limits
  do not isolate latency at this load in either run.
- **Provider faults degrade predictably.** A 60 ms provider delay adds
  about the delay to ContextRuns (p50 192 ms); a stall beyond the 100 ms
  embedding deadline falls back to lexical after the deadline (p50 219 ms);
  an outage fails fast to lexical (p50 106 ms). Appends are unaffected.
  Indexing paused during the outage and its backlog drained within one
  window afterwards (lag p95 277 s, then about 1 s).
- **Database CPU contention** (four CPU-bound queries) raised ContextRun p50
  to 182 ms; appends stayed at p95 31 ms.
- **Burst recovery is noisier than the burst.** After three times the rate,
  ended Sessions' Capture and review work clustered: admission waits near
  2 s and pool acquire p99 654 ms for a few minutes.
- **Resources.** Gateway RSS stayed between 47 and 66 MB and the worker's
  between 29 and 44 MB with no upward trend; at steady 1x the gateway used
  about 0.02–0.03 cores and the worker under 0.01. Capture queue wait p95
  stayed near 1 s.
- **Storage.** In the sustained phase the database grew about 160 MB per
  hour for the measured tenant alone, about 3.9 GB per day at 1x; the audit
  log and Session events were each about a quarter of that growth. No
  time-based purge of Session events or audit exists, so growth is linear
  until an owner retention decision; the usage ceiling counts only stored
  event payload (about 0.72 GB per day here).

### Repeatability

An earlier run of the same profile and seed, identical except for the
`assess` flags, was interrupted when the host slept during its
provider-outage phase. Its phases that finished before the sleep give a
second sample (measured tenant, p50 / p95 / p99 ms):

| Phase | Append, earlier run | Append, run above | ContextRun, earlier run | ContextRun, run above | Pool acquire p99, earlier / above |
| --- | --- | --- | --- | --- | --- |
| baseline | 20 / 29 / 42 | 17 / 28 / 55 | 56 / 100 / 120 | 47 / 86 / 179 | 4 / 10 ms |
| burst | 16 / 27 / 37 | 14 / 23 / 35 | 71 / 101 / 113 | 66 / 88 / 110 | 3 / 2 ms |
| burst_recovery | 20 / 30 / 49 | 21 / 373 / 2,863 | 88 / 132 / 186 | 84 / 1,744 / 4,059 | 3 / 654 ms |
| noisy_neighbour | 11 / 20 / 84 | 13 / 512 / 4,118 | 86 / 131 / 950 | 107 / 2,679 / 7,360 | 16 / 1,737 ms |
| provider_slow | 17 / 27 / 44 | 15 / 38 / 199 | 179 / 219 / 223 | 192 / 426 / 477 | 4 / 5 ms |
| provider_stall | 17 / 28 / 46 | 14 / 34 / 233 | 214 / 254 / 264 | 219 / 454 / 644 | 4 / 8 ms |

Medians agree within about 25%. Tails under contention do not: the same
noisy-tenant and burst-recovery work saturated the pool in one run and not
the other, with the same request counts (15,652 and 15,653 noisy-tenant
calls) and no other job of this measurement running. The earlier run met
the baseline ContextRun budget (p99 120 ms) and the later one did not. The
cause is not isolated: the container VM also ran unrelated workloads,
including a local Kubernetes cluster with a pod in a restart loop, so
outside contention cannot be excluded. p99 figures under contention on this
host are a range, not a point, and a reviewed baseline needs repeated runs
on dedicated hardware.

## Ten times the declared load

`first-offer-10x`: 240 agents in one tenant, the same rates and host. It is
recorded, not gated:

| Phase | Load | Append p50/p95/p99 ms (n) | ContextRun p50/p95/p99 ms (n) | Knowledge search p50/p95/p99 ms (n) | Unexpected | Assessed |
| --- | --- | --- | --- | --- | --- | --- |
| startup_warmup | 10x, empty data (warm-up) | 12 / 19 / 31 (8566) | 30 / 49 / 70 (764) | 18 / 26 / 32 (188) | 0 | no |
| sustained_10x | 10x (240 agents) | 14 / 2400 / 5206 (43018) | 79 / 3594 / 7119 (3702) | 103 / 3537 / 6697 (843) | 0 (+4147 limited) | no |
| recovery | 10x agents winding down (warm-up) | 18 / 1386 / 2644 (8623) | 135 / 559 / 1577 (696) | 152 / 486 / 1307 (191) | 0 (+61 limited) | no |

The first three minutes, with little accumulated data, held 1x-like
latency. Under sustained 10x the gateway admitted about 43 appends and 3.9
ContextRuns per second, refused 4,147 calls with `429` (about 9% of
appends), and its tails reached seconds: pool acquire p99 1.2 s, admission
waits of 2–3 s in every class, append p99 5.2 s and ContextRun p99 7.1 s.
Gateway CPU stayed near 0.18 cores and its memory flat at about 60 MB, so
the binding constraint is the eight-connection pool behind per-tenant class
concurrency, not gateway CPU. The database grew about 1.1 GB per hour. Ten
times the envelope is not supported on this shape.

## Capacity summary

| Axis | Measured on the reference host | Limiting resource | Scale-up trigger |
| --- | --- | --- | --- |
| Session events | 4.8/s per tenant at p95 ≤ 28 ms (1x); about 43/s admitted with seconds-long tails (10x) | Gateway pool (8) and tenant class concurrency | Pool acquire p99 above 50 ms or any `session_ingest` admission wait above 250 ms at steady load |
| ContextRun | 0.4/s at p95 86–196 ms with small private corpora (1x); 0.7–0.8 s p50 once shared Knowledge fills the planner's candidate caps; 2.7 s at 50,000 items | Planner candidate retrieval and selection, then the pool | ContextRun p95 above 100 ms at steady load, or any Session admitting enough Knowledge to fill the planner's candidate caps |
| Knowledge corpus | Dense leg 55 ms at 10,000 items (planner chain), 268 ms at 50,000 | Exact item-driven scan cost per admissible row | More than about 10,000 current items per tenant, where the dense leg alone passes 50 ms in the planner chain and 400 ms in broad search: reopen the vector index scale decision |
| Second tenant | Isolation held; latency isolation failed at 20 times (measured ContextRun p99 0.95–7.4 s across two runs) | Shared pool; per-tenant limits sum above it | Any tenant sustaining more than the pool's worth of concurrent work: lower per-tenant concurrency or raise the pool |
| Storage | About 3.9 GB/day per 24-agent tenant, no purge | Disk; retention is undecided | Free space below 30 days of measured growth |
| Memory | Gateway 47–66 MB, worker 29–44 MB RSS, flat | Not limiting | RSS rising across checkpoints in a soak |

Measured against the proposal, the first offer supports one 24-agent tenant
(with a lightly loaded second tenant) on hardware at least as capable as the
measured host for Session ingest, search and governance. The ContextRun
budget is not met: with small private corpora its p95 held at baseline in
both runs, but p99 held only in the earlier run's baseline and both missed
in the sustained phase; a shared 1,000–10,000-item corpus costs 0.7–1.3 s
per ContextRun. ContextRun planning is the envelope's limiting resource and
needs either an owner decision on a looser budget or planner work that
reduces per-candidate cost. Every figure is from one macOS host with
host-process binaries and a stub provider; it is not a Linux, Compose-image,
live-model or multi-gateway result.

## Unrun and open

- The 24-hour soak: `make eval-load LOAD_PROFILE=soak-24h
  EVAL_LOAD_REPORT=<path>` on a host that stays awake about 25 hours (on AC
  power; a closed lid on battery sleeps the host and invalidates a run) with
  roughly 30 GB free. It must show no unbounded RSS, connection, queue-age
  or indexing-lag trend, and a storage slope explained by retention. The
  earlier 1x run that the host's sleep interrupted appears only in the
  repeatability comparison, for phases that finished before the sleep.
- A reviewed baseline: none exists in `evals/load/baselines/ledger.json`;
  thresholds are proposals until an observation run on production-shaped
  Linux hardware is reviewed.
- Owner decisions: the envelope and budgets above, retention, and whether a
  tenant's concurrency should be bounded below the pool.

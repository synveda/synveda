# Load and latency suite

`synveda-eval load` drives the versioned public API with ordinary service
identities in two tenants and writes a resumable, content-free report. It is
the measurement behind the [capacity envelope](../../docs/CAPACITY.md); the
dense-retrieval plan evidence it depends on comes from
`demos/ctx-7-dense-plans.sh`.

## What one run does

`evals/load.sh <profile>` enters a fresh exact-role database fixture, builds
the gateway, worker and harness with `EVAL_PRODUCT_PROFILE` (`release` by
default), starts a loopback stub embedding provider and points the gateway and
worker at it, provisions `load-NNNN` agents in the measured tenant and
`load-b-NNNN` agents in the second tenant through the governed public API,
then runs the profile's phases in checkpoint windows.

Each agent acts sequentially, like a harness's hooks: it opens a Session,
appends events, composes ContextRuns, searches Knowledge, creates and edits
private Knowledge through the reviewed VedaFlow path (two named approvers and
a distinct publisher), ends and freezes its Session for Capture, accepts
candidates once Capture completes, repeats an edit against a replaced
revision (expected `409`) and reads the other tenant's Sessions (expected
`404`/`403`). A `2xx` on that cross-tenant probe is recorded as an isolation
breach and fails every assessment.

Arrivals are seeded Poisson schedules per agent, phase and window. Latency is
measured from the scheduled send time (`response`), from the actual send
(`service`) and between the two (`lag`), so a slow call delays and is charged
to the next one instead of silently thinning the load. A window that closes
before an agent reaches a scheduled call counts it as `unsent`.

After every window the harness records:

- per-operation outcome counts and p50/p95/p99/max with sample counts, for
  the measured and the noisy tenant separately;
- gateway and worker Prometheus deltas: HTTP time by route, admission wait,
  pool acquire time, ContextRun planner stages, Capture queue wait,
  extractor time and Knowledge indexing lag, plus pool and slot gauges;
- the stub provider's request count, errors and service time;
- gateway and worker RSS and accumulated CPU;
- the latest database sample: size, connections by role and state, audit,
  Session event, Capture, Knowledge and embedding relation sizes.

Reports name operations, counts and timings only. They hold no bearer,
request body, Knowledge text or content identifier.

## Profiles

| Profile | Agents | Purpose | Duration |
| --- | --- | --- | --- |
| `smoke` | 3 + 2 | Proves the harness, report schema and required metric names; budgets are loose | about 2.5 min |
| `first-offer-1x` | 24 + 6 | The declared envelope: baseline, burst, noisy tenant, provider slow/stall/outage, database CPU contention, recovery and sustained traffic | 68 min |
| `first-offer-10x` | 240 | Ten times the declared concurrency, recorded honestly rather than gated | 21 min |
| `soak-24h` | 24 + 6 | 24 hours after a 15-minute warm-up, 15-minute checkpoints | 24 h 15 min |

Budgets are assessed only in completed, non-warm-up phases at multiplier 1
without an injected provider or database fault; the noisy tenant may run.
Faulted and 10x phases are reported, not assessed. Isolation is assessed in
every phase.

## Commands

The wrapper needs Docker with Compose, Node.js, Python 3 and the PostgreSQL
`psql` client on the host; it refuses to start without `psql` or `python3`.


```sh
make eval-load-smoke                                # a few minutes
make eval-load LOAD_PROFILE=first-offer-1x EVAL_LOAD_REPORT=/tmp/1x.json
make eval-load LOAD_PROFILE=first-offer-10x EVAL_LOAD_REPORT=/tmp/10x.json
make eval-load LOAD_PROFILE=soak-24h EVAL_LOAD_REPORT=/tmp/soak.json
```

The soak needs a host that stays awake for about 25 hours and roughly 30 GB
of free container storage. `EVAL_LOAD_RESUME_ATTEMPTS` (default 3, at most 20)
lets the wrapper restart an interrupted harness with `--resume` against the
same live stack; the resumed run refuses a checkpoint written by any other
profile and continues from its last completed window, keeping the phase's
histograms. Against an operator-managed deployment, call the harness directly
with an environment file naming `load-NNNN`/`load-b-NNNN` actors and the
reviewers, and pass `--resume` to continue:

```sh
synveda-eval load --env env.json --profile evals/load/profiles/soak-24h.json \
  --report soak.json --environment facts.json --worker-url http://127.0.0.1:8152 \
  [--provider-url …] [--fault-hook …] [--resume]
```

`synveda-eval load-check --report <report>` refuses an incomplete run, a
profile that does not match its digest, a missing environment axis, a phase
without append, ContextRun or Knowledge search samples, a phase that never
observed the required gateway metrics, and any baseline under
`evals/load/baselines/` whose bytes differ from its entry in `ledger.json` or
that has no entry. Baselines are added only with a reviewer and date in the
ledger after an observation run has established the thresholds; an existing
baseline is never edited, only superseded by a new file.

## Limits of the measurement

The fixture uses fixture-issued HS256 bearers, so RS256/JWKS verification is
not exercised. The stub provider performs no inference; its delay and error
share model provider behaviour, not model latency. The gateway and worker are
host processes beside a containerised PostgreSQL, not the Compose reference
images, and every run is one machine. Figures apply only to the recorded
hardware, build and database.

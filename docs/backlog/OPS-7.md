---
title: "OPS-7: Gateway horizontal scale"
labels:
  - epic:OPS
  - phase:4
size: L
---

# OPS-7: Gateway horizontal scale

**Epic:** OPS — Deployment & operations · **Phase:** 4 · **Size:** L

## Problem and evidence

Helm pins one gateway replica and Recreate. ADR-0126 moves pending OIDC login
and CLI handoff state into a deployment-key-sealed PostgreSQL ledger. Stored Cedar
policy packs are compiled process-locally and refreshed on a timer; ADR-0127
adds a provisional gateway freshness lease, while cross-replica latency and
worker expiry remain unqualified. Scope ancestry,
identity, grants, groups, assignments and Configuration are already read in
ordinary request transactions, and cached Cedar fragments compare the exact
supplied scope shape. Capture, Knowledge indexing,
directory pull and relaxation expiry now run in a separate supervised core
worker, but only one worker replica is supported and concurrent-worker recovery
has not been proved. Worker SIGTERM withdraws readiness and performs a bounded
cancel/join. Before the source lifecycle slice, gateway readiness remained
true during graceful request shutdown. Claimed-work termination remains
unproved. These gaps are recorded in
[production readiness](../PRODUCTION_READINESS.md) and the one-replica refusal
is governed by [ADR-0062](../adr/adr-0062-enterprise-profile-and-helm-chart.md).

On 2026-09-29, [ADR-0124](../adr/adr-0124-withdraw-gateway-admission-before-http-shutdown.md)
added a separate one-way request-admission signal. A normal SIGTERM now
withdraws `/readyz`, rejects new application requests and keeps the authority
sentinel alive until the HTTP server drains; only then do supervised tasks stop
and telemetry flush. Helm's next readiness probe removes the endpoint, and
Compose has an outer stop margin. A real exact-role gateway subprocess test
observed 503 readiness while HTTP remained live, refused new work and exited
inside its configured 15-second bound. A synthetic in-flight request completed
after admission withdrawal. This is single-process source evidence, not a
three-pod, load or interrupted-provider drain result.

The senior Rust review then found a separate startup window: the database
authority gate could open before the first stored policy-pack load completed.
[ADR-0125](../adr/adr-0125-gate-gateway-on-policy-convergence.md) now binds
readiness and application admission to successful policy convergence for the
current authority generation. Synthetic route tests prove the initial refusal,
opening and generation mismatch; the exact-role process path remains the
deployment check. This closes first-load ordering only.

[ADR-0126](../adr/adr-0126-persist-one-time-login-ledger.md) appends migration
`0003` for hashed one-time selectors,
database-clock TTLs and atomic consume. The protocol crate uses a narrow async
ledger; only tests can select its memory implementation. Exact-role mock-IdP
acceptance begins an OIDC login on one gateway instance and completes it on
another, and begins a CLI login on one, completes its callback on another and
redeems the handoff on a third. Replays fail. The 25-case epoch suite proves
the exact `0002` prefix advances without rewriting its SQLx rows or a tenant.
This is cross-process source acceptance, not a three-pod routing, load, key
rotation or database-failover result. It left post-start policy freshness,
multi-worker ownership and the full termination sequence open.

[ADR-0127](../adr/adr-0127-expire-stale-gateway-policy-convergence.md)
now makes a gateway's policy-ready generation expire after 30 seconds without
a complete successful sweep. A five-second sweep deadline and a 1–15-second
poll interval bound individual attempts; failures do not renew the lease.
Expiry removes readiness and cancels governed HTTP work, and a later successful
sweep can reopen the same authority generation. The sweep publishes duration
and closed outcome metrics. Unit route acceptance covers expiry, in-flight
cancel and recovery; an exact-role test with two independent policy engines
in one process covers a stored revision reaching each engine only after its
own sweep, and a failed compile leaving last-good loaded without counting as
convergence. This is source behavior, not a measured 30-second end-to-end
mutation bound under load.

[ADR-0128](../adr/adr-0128-expire-stale-worker-policy-convergence.md) now
applies the same provisional lease to the core worker and optional Apalis
leaf. Core readiness drops at expiry; its supervisor cancels and joins
governed futures, then retries initial convergence before taking work. Apalis
refuses dispatch and execution, withdraws readiness and exits nonzero for a
fresh convergence on restart. Synthetic core-worker tests cover readiness,
in-flight cancellation and bounded join. A real claimed Capture interruption,
optional Apalis delivery interruption and provider effects have not been
qualified. A disposable one-node Kind run then passed the normal Helm install,
CloudNativePG primary promotion and post-failover context read. The
`demos/ops-7-three-gateway.sh` probe temporarily scaled that deployment to
three ready gateway pods and forced JSON login from pod A to callback on B,
CLI login from A to callback on B and redemption on C, replay refusals and
direct authenticated reads from every pod. It restored one replica afterward.
The same harness applied two test-pack revisions through the audited CLI under
an ordinary gateway role and observed each of the three local compiles via
the closed reload metric: maximum database-timestamp-to-observation lag was
1.065 and 4.945 seconds. It checked readiness and cleared the pack. A further
one-node run adopted the canonical team Configuration through the public API,
opened and reviewed a project Configuration with a second identity, and
selected a uniquely named test pack. Three direct pod requests permitted
`ScopeUpdate` under revision 1, then each pod refused it with the revision-2
policy reason; the maximum observed database-timestamp-to-denial lag was
5.259 seconds. The probe repeated all three denials, disabled the binding
through the governed API and revoked the disposable review grant. The pack
and immutable Configuration version remain in this disposable cluster as a
legal rollback target; deleting the pack would violate the store's history
constraint. The harness restored one gateway replica. These are light-load,
one-node observations, not a mutation bound under sustained production traffic.
A subsequent test-only store helper committed an invalid Cedar pack through an
ordinary gateway-role tenant transaction with content-free audit, then cleared
it after 42 seconds. With three gateways serving sustained direct authenticated
reads, 486 paired request/readiness samples saw all pods close by 28.996 seconds
and reopen by 45.747 seconds from monitor baseline. Each recorded failed
policy sweeps while its database-authority gauge remained ready and its
authority-unavailable count did not change. The same probe restored one gateway
replica; the core worker was ready afterward. A table-lock experiment was not
counted as lease evidence because it also closed the database-authority gate.
The invalid-pack result qualifies the provisional source lease under this
one-node light load, not a production traffic envelope, claimed-worker
interruption or cross-AZ behavior. Key rotation during login, multi-node loss
and a rolling upgrade remain untested. Next exercise claimed work under
expiry, then pod loss and rolling traffic before lifting either replica limit.

## Scope

- Retain the durable one-time login and CLI handoff contract under real
  cross-pod routing, key rotation and restart.
- Prove scope, grant, identity and Configuration mutations against fresh
  request reads on every replica. Bound compiled policy-pack refresh with a
  fail-closed staleness rule; introduce a notification or generation transport
  only if measured polling cannot meet that bound.
- Prove every core-worker job's multi-replica ownership, lease, idempotency and
  provider-concurrency behaviour before lifting the one-worker limit.
- Withdraw gateway readiness on termination, drain requests, and prove that
  core workers finish or safely release claimed work, flush telemetry and exit
  within a bounded grace period.
- Lift the chart refusal only after a three-replica acceptance passes.

## Non-goals

- No distributed cache, second queue product or orchestration framework merely
  to lift the replica limits.
- No weakening of fresh PDP decisions, forced RLS or live re-authorisation on
  idempotent replay.
- No claim that CloudNativePG replication makes the request plane available.
- No unbounded sticky-session dependency presented as high availability.

## Architecture seam

The one-time login ledger belongs beside durable console sessions in
synveda-store. Fresh
request transactions and exact-shape fragment comparison supply scope/grant
correctness; local entity flush is cache hygiene. Compiled policy packs require
a measured cross-process refresh bound and fail-closed expiry. Existing durable
batch/job tables remain the worker seam.
Gateway readiness owns request drain. The core-worker supervisor owns task
cancellation and bounded join; each durable aggregate owns its claim/recovery
semantics. Helm owns the separate replica and termination settings.

## Acceptance criteria

- Three replicas complete a login begun on one pod and callback/handoff on
  another; the same state/code cannot be redeemed twice.
- A scope move, grant revoke, identity disable and Configuration/policy change
  become visible to every replica within the documented bound, with no stale
  permit after the bound.
- Three gateway replicas and separately scaled core-worker replicas produce one
  durable effect, respect provider concurrency and recover every lease after
  pod loss.
- SIGTERM makes the affected process's readiness fail first, drains gateway
  requests, and lets core workers finish or safely release claimed work before
  exiting inside the pod grace period.
- Sustained traffic loses a pod and a rolling upgrade without incorrect
  decisions or avoidable login failure.
- The chart accepts only tested replica counts and no longer requires Recreate.

## Required tests

- Three-pod kind acceptance with cross-pod login and direct per-pod requests.
- Mutation/invalidation latency tests for every authority-changing family.
- Worker race, lost-ack, lease-expiry, provider-outage and pod-kill tests.
- Connection-pool/load/soak evidence at the maximum supported replica count.
- Exact shutdown subprocess and Kubernetes termination-sequence tests.

## Rollout and rollback

Ship bounded policy refresh while still pinned to one replica, observe lag,
then canary two and three replicas. Retain one-replica/Recreate as the
rollback until the full acceptance is stable. Rollback must leave durable
state readable and may reduce replicas without discarding jobs.

## Dependencies

The owner must define availability, policy-pack staleness, drain and provider-
concurrency limits. Choose a generation/notification mechanism only if the
measured refresh path needs one; decide worker ownership in an ADR. OPS-5 and
OPS-6 provide recovery and upgrade discipline.

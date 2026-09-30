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
adds a provisional gateway freshness lease. Cross-replica latency and bounded
Capture-only scaling have one-node Kind evidence, but production-load and
general worker-family bounds remain unqualified. Scope ancestry,
identity, grants, groups, assignments and Configuration are already read in
ordinary request transactions, and cached Cedar fragments compare the exact
supplied scope shape. Capture, Knowledge indexing,
directory pull and relaxation expiry now run in a separate supervised core
worker. One combined worker is supported, with up to two optional capture-only
pods; other maintenance loops have no concurrent owner. Worker SIGTERM
withdraws readiness and performs a bounded cancel/join. Claimed Capture has
one-node policy-expiry, pod-loss and provider-503 recovery evidence; provider
exactly-once effects and general worker ownership remain unproved.
These gaps are recorded in
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
one-node light load, not a production traffic envelope or cross-AZ behavior.
A follow-on Kind drill kept a real Capture batch claimed inside a deliberately
blocked extractor, then applied the invalid pack. The core worker's private
readiness fell and the first provider socket was cancelled 28 seconds after
the fault, while its database-authority gauge stayed ready and failed-policy
sweep counters rose. No second provider call began while policy remained stale.
After the pack was cleared, the worker reclaimed the expired claim; before
the second blocked call was released, the batch still had zero candidates.
It then completed with two attempts, two provider calls (one cancelled) and
exactly one durable candidate. This proves one claimed Capture recovery on
one node, not provider exactly-once effects or multi-worker ownership.
A second Kind run temporarily scaled the core worker to two pods. Only one
worker reached the blocked extractor for a newly claimed Capture batch. The
probe identified that owner from the provider connection and deleted its pod;
the call was cancelled while the other worker remained ready. A different pod
reclaimed the expired claim, with no candidate committed before the second
provider call finished. The batch again completed in two attempts with one
candidate and one cancelled provider call. This qualifies Capture's fenced
claim under one owner-pod loss, not all core-worker jobs or a shared provider
quota. Knowledge indexing currently permits duplicate external embedding
calls while its durable insert converges; directory pull has no cross-worker
pass owner to prevent duplicate absence accounting. Decide those ownership
contracts before supporting two worker replicas. Both
`demos/ops-7-worker-recovery.sh policy-expiry` and
`demos/ops-7-worker-recovery.sh pod-loss` are repeatable against the isolated
`kind-synveda-ops7` context and restore the chart's one-replica baseline.
[ADR-0129](../adr/adr-0129-scale-fenced-capture-with-singleton-maintenance.md)
chooses a combined singleton plus optional capture-only workers instead of
scaling the unfenced maintenance loops. The source worker now accepts only
`combined` (default) or `capture-only`: both retain the same authority and
policy lifecycle, while capture-only omits maintenance task startup and its
embedder/KMS configuration. A real subprocess accepts capture-only with an
invalid maintenance embedder and still closes readiness during a database
outage; the combined profile refuses that embedder. The source chart renders
zero to two separate
capture-only pods while retaining one combined worker. It rejects a third
extra pod and counts each pool in the bundled/CNPG connection budget. A
source-image, one-node Kind run made three distinct worker pods hold three
Capture calls, deleted one capture-only owner, completed the two surviving
calls, and observed a different pod reclaim the expired claim. Each batch
completed with one candidate; there were four provider calls, one cancelled,
and attempt counts `1,1,2`. A separate 503 from the same provider was retried
once by Capture and completed with one candidate. The repeatable
`demos/ops-7-worker-recovery.sh capture-scale` restores the default release
values. This qualifies the bounded Capture-only chart option in a disposable
one-node cluster. Provider-wide concurrency, sustained load, cross-node pod
loss, the other maintenance loops and a rolling mixed-version upgrade remain
open; `worker.replicas` stays refused.

The first claimed-Capture run exposed a Helm/CNPG defect: after the baseline
primary promotion,
the restarted standby could not reconnect because the chart's bootstrap had
revoked PUBLIC CONNECT on the `postgres` maintenance database without granting
CNPG's reserved `streaming_replica` role CONNECT. The primary and product pods
served, but CNPG reported only one of two instances ready and a Helm upgrade
waited. A narrow grant on the disposable primary restored both instances;
the source chart now grants the reserved role during the required administrator
bootstrap Job, after CNPG creates it. A fresh chart-rendered two-instance
cluster passed that bootstrap, proved PUBLIC and product roles still lacked
maintenance CONNECT, and returned to two ready instances after its replica
was deleted. Contract checks and the operator runbook cover the ACL. A
retained-cluster Helm upgrade to the fixed source image passed its bootstrap
Job with two ready CNPG instances. In a second primary-loss drill, a graceful
pod delete left the old primary terminating under CNPG's 30-minute grace and
promotion pending while its WAL receiver remained active. A crash-style
deletion of that disposable pod let CNPG promote the other instance and return
to two ready instances. The three-gateway probe then passed cross-pod JSON and
CLI login with replay refusal, two policy-pack revisions reaching all pods in
3.917 and 4.937 seconds, and `ScopeUpdate` changing from 200 to a
revision-specific 403 on all three pods within 5.249 seconds. The saved Helm
values and one-gateway/one-worker image baseline were restored. These are
one-node observations without a timed recovery objective or planned
switchover qualification. Key rotation during login, multi-node loss and
a rolling upgrade also remain untested. Next prove other worker-family
ownership and pod loss under traffic before lifting the combined-worker or
gateway replica refusal.

The [2026-09-30 Rust review](../RUST_ARCHITECTURE_REVIEW_2026-09-30.md)
found no enforced active-tenant count behind the serial policy sweep's
five-second whole-sweep deadline. At a sufficient tenant count, or after one
invalid pack, the shared 30-second lease closes governed work for unrelated
tenants. Measure and enforce the supported tenant/pack envelope, then test
per-tenant failure isolation without allowing a stale Cedar decision. The
source now exports unlabeled active-tenant count and per-tenant stored-pack
count alongside existing sweep duration/outcome metrics. This makes the work
envelope measurable; it neither enforces a limit nor isolates a failed tenant.
Next measure at the owner-selected maximum before changing admission. The
same review found that a transient deployment-key provisioning error was not
retried until the database authority generation changed. The source gateway
now retries deployment-key provisioning and unwrap checks with a five-second
attempt deadline and two-second pause while authority remains open. It marks
application readiness only after that key and stored policy packs are ready;
an intentionally disabled KMS retains bearer-only readiness. A disposable
exact-role process test holds the deployment-key table through one timeout,
observes closed readiness and then recovery after release. A second process
with the wrong KEK remains unready despite the stored row. Content-free ready
and attempt metrics distinguish these outcomes. This is single-process local
evidence; live OIDC/KMS faults, key rotation and cross-pod behavior remain
open.

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
- Three gateway replicas and bounded capture-only workers produce one durable
  Capture candidate set per batch and recover its fenced lease after pod loss.
  Any future combined-worker replica count must first prove every maintenance
  loop's ownership; provider concurrency needs an explicit quota.
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

# Kubernetes operations

Use the [installation guide](README.md) and save the exact chart, values,
immutable image overlays and Secret custody references. This runbook covers
one gateway and one combined worker, with optional capture-only capacity and
planned maintenance. It does not establish HA,
WAL/PITR, zero downtime or a promised RPO/RTO. Compose keeps its existing
[backup/restore/upgrade entry points](../../compose/README.md).

The optional [CNPG object-store backup candidate](BACKUP.md) adds chart
plumbing for physical base backup and WAL through the Barman Cloud plugin. It
does not change this runbook's recovery evidence until a live isolated PITR and
joint identity/key drill pass.

## Health, logs and telemetry

Application liveness is `/healthz`; readiness is `/readyz`. Readiness requires
the current database epoch, ordinary role, forced-RLS/ACL contract and policy
convergence. The authority proof has a five-second deadline and repeats every
15 seconds. A transient outage closes the gate and retries; a conclusive role,
schema or database-identity refusal exits. Restart loops need diagnosis, not
weaker readiness or an elevated database role.

`/version` returns the release the gateway or worker binary was built as, as
plain text. Like `/healthz` it is ungated and private: the ingress and the
Compose proxy do not expose it.

The gateway also waits for its first successful stored policy-pack load for
the current authority generation. A recovered database generation requires a
new load before requests resume, even if the process still holds an older
compiled pack. An unavailable or invalid stored pack keeps the gateway
unready; inspect the policy refresh logs rather than routing around the gate.
After startup, a complete successful sweep renews a provisional 30-second
freshness lease. An incomplete, failed or five-second timed-out sweep cannot
renew it; expiry withdraws readiness and cancels governed HTTP work. The
`SYNVEDA_POLICY_REFRESH_SECS` range is 1–15 seconds (default 5). Inspect
`synveda_policy_pack_refresh_sweeps_total` and
`synveda_policy_pack_refresh_seconds` when readiness drops. The core worker
uses the same provisional lease: expiry withdraws readiness, cancels governed
work and retries initial convergence before resuming. An interrupted claim
requires a lease/fence recovery check for each job family. In one disposable
Kind run, a
claimed Capture request was cancelled on policy expiry, then its fenced second
attempt committed one candidate after policy recovery. A separate two-worker
Kind run deleted the pod owning a blocked Capture claim; another pod reclaimed
it and committed one candidate. The chart retains one combined worker;
capture-only pods may add fenced Capture throughput but do not run the other
job families. Provider-wide quotas remain operator-owned.

## Optional Capture capacity

The source chart accepts `worker.captureOnlyReplicas: 0..2`; zero is the
default. It always keeps exactly one combined worker because Knowledge
indexing and directory pull have no multi-process owner. Each extra pod uses
the ordinary worker database role, the same authority and policy readiness
gates, and only the fenced Capture loop. It has no OIDC, KMS or embedder
Secret mount. `worker.replicas` remains refused. The published v0.4.3 chart
does not include this source candidate.

Reserve `worker.capture.providerConcurrency` extractor calls at the provider
and `worker.dbMaxConnections` database connections per extra pod, plus enough
capacity for a replacement during rollout. The
chart rejects a bundled/CNPG connection budget that is too small; an external
database and provider need their own operator-enforced quotas. If using
NetworkPolicy, `networkPolicy.extraEgress.worker` also applies to capture-only
pods, so permit the chosen extractor endpoint there. Check each private
`/readyz` through a pod exec and inspect Capture metrics before increasing
the count. To roll back, set `worker.captureOnlyReplicas: 0` using the saved
values and wait for interrupted claims to expire and be reclaimed by the
combined worker. A repeated external provider call is possible; the attempt
fence permits one durable candidate set. This is Capture capacity, not
general maintenance-worker availability or gateway HA.

## Capture retries and provider capacity

Source builds after v0.4.7 schedule Capture retries in PostgreSQL and cap
external extractor calls across every worker pod
([ADR-0158](../../../docs/adr/adr-0158-durable-capture-backoff-and-provider-slots.md)).
The values below render identically into the combined and Capture-only
Deployments; they are provisional defaults until the product owner adopts
them.

| Value | Default | Meaning |
| --- | --- | --- |
| `worker.capture.providerConcurrency` | `3` | Installation-wide concurrent calls to one extractor endpoint (1–64). |
| `worker.capture.retryBaseSeconds` | `30` | First retry delay before ±50% deterministic jitter. |
| `worker.capture.retryMaxDelaySeconds` | `1800` | Cap for any delay, including a provider `Retry-After`. |
| `worker.capture.retryMaxAgeSeconds` | `7200` | A retry cycle closes when its next attempt would start later than this after the cycle began. |
| `extractor.timeoutSeconds` | `45` | Per-call client deadline; a stalled call is a retryable timeout. |

Only timeouts, connection loss, HTTP 429, 503/529 and 408/500/502/504 are
retried, at most five attempts per cycle. Authorisation, Configuration,
source, 4xx and malformed-output failures close after one attempt. A worker
reserves a provider slot in the same transaction as its claim; when every
slot is held, the claim rolls back without spending an attempt. A crashed
holder's slot returns when its 60-second Capture lease expires. A worker pod
that is terminated gracefully keeps its slot while its call drains for up to
`worker.shutdownSeconds`. A call that finishes or reaches
`extractor.timeoutSeconds` in that time frees the slot with its attempt; one
cut off by the drain deadline frees it at lease expiry. Another installation using the same provider account
is not counted; reserve capacity for each one.

Inspect work through the public API; never edit Capture rows directly. The
database refuses a reset attempt fence or a reopened batch outside the
retry command.

```sh
synveda session retry-capture BATCH_UUID --idempotency-key RETRY_REQUEST_KEY
```

`GET /v1/capture-batches/BATCH_UUID` reports `state`, `attempts` of
`max_attempts`, the content-free `error_code`, `next_attempt_at` while a retry
is scheduled and `terminal_reason` once it stopped. The retry command needs
`session.write` on the source session, works only on a failed session batch
within seven days of failure and at most twice per batch, and refuses a batch
whose frozen source was withdrawn or whose effective Configuration changed
since it was frozen; capture the session again instead. Each retry is
audited as `capture.batch.retry_requested`.

Watch these worker metrics (no tenant or principal labels):

- `synveda_capture_batches_total{outcome=…}` — `retry_scheduled`,
  `permanent_failure`, `attempts_exhausted`, `retry_age_exhausted`,
  `provider_saturated`, `completed`, `lease_lost`;
- `synveda_capture_queue_wait_seconds{attempt="first"|"retry"}` — database
  time from eligibility to claim;
- `synveda_capture_retry_cycle_seconds{outcome}` — how long a cycle that
  needed a retry took to recover or close;
- `sum(synveda_capture_provider_slots_held)` against
  `synveda_capture_provider_slot_limit`, and
  `synveda_capture_saturated_wait_seconds` for work waiting on capacity.

Sustained `provider_saturated` with a rising saturated wait means the ceiling
is too low for the offered work or calls are stalling; a burst of
`attempts_exhausted` or `retry_age_exhausted` means the provider stayed
unavailable longer than one cycle. After the provider recovers, retry the
affected batches with the command above.

OIDC login now parks its PKCE state and CLI handoff in the same deployment-key
plane as console sessions. A missing deployment key refuses `/auth/login`
before redirect; restore the matching key with PostgreSQL so unexpired logins
can complete after a gateway restart. The isolated cross-process test exercises
this source path. A one-node Kind drill used three ready pods to route
JSON and CLI login across them, refuse replays and read the same identity from
each pod after baseline database failover. It left the chart at one replica;
two audited test-pack revisions then compiled on all three pods within
5 seconds of their database timestamps at light load. The harness cleared
its pack and restored one replica. Key rotation, decision-level restrictive
policy latency under sustained load and multi-node loss remain unqualified.

After a database outage, Kubernetes' Ready status can lag the application's
current gate. Probe `/readyz` directly, then validate a fresh login and governed
read before reopening traffic. The external drill also observed Keycloak using
one stale JDBC connection after the database returned: a fresh login retry
recovered. Its report counts those failures within a 90-second test bound;
readiness alone is not a sign-in guarantee. Escalate persistent failures to the
identity administrator; do not replace credentials or weaken token checks.

Gateway SIGTERM withdraws `/readyz` and refuses new application work first.
With the default 30-second shutdown bound, it leaves up to ten seconds for
the next five-second Helm readiness probe to remove the endpoint; Helm accepts
one failed probe. It then stops HTTP admission, waits for in-flight requests
while the authority sentinel stays live, stops background tasks and flushes
telemetry. A conclusive database-authority refusal still cancels affected
requests immediately. Shorter configured bounds reduce the pre-drain window;
the single-process source test is not evidence of a rolling upgrade or
multi-pod availability. Shutdown also withdraws worker readiness and cancels
supervised tasks. Capture
cancellation leaves its fenced claim for the existing 60-second lease expiry;
the next worker retries. A request may reach an external provider more than
once. Database claim fencing/idempotency does not promise exactly-once external
effects. The default worker join is 75 seconds, gateway 30, with another ten
seconds of Kubernetes termination grace. Keep those bounds aligned.

The existing Operations console displays policy-visible aggregates from public
APIs. It is not a cluster-admin dashboard. TEI, extractor or OTLP availability
is distinct from application health; deterministic/lexical operation needs no
model service. `otel.endpoint` and private metrics can feed the organisation's
existing monitoring endpoint without installing a dashboard stack.

Private checks, for the release/namespace `synveda`:

```sh
kubectl -n synveda get pods,jobs,pvc
kubectl -n synveda get events --field-selector type=Warning
kubectl -n synveda logs deployment/synveda --tail=100
kubectl -n synveda logs deployment/synveda-worker --tail=100
kubectl -n synveda exec deployment/synveda-worker -- curl --fail --silent http://127.0.0.1:8121/readyz
kubectl -n synveda exec deployment/synveda-worker -- curl --fail --silent http://127.0.0.1:8121/version
kubectl -n synveda port-forward service/synveda 8120:8120
# In a second terminal; the management endpoint stays on local loopback:
curl --fail --silent http://127.0.0.1:8120/readyz
curl --fail --silent http://127.0.0.1:8120/metrics
curl --fail --silent http://127.0.0.1:8120/version
```

## Request admission and tenant usage

Source builds after v0.4.7 bound request rate and concurrency inside the one
gateway pod and charge durable tenant usage in PostgreSQL
([ADR-0159](../../../docs/adr/adr-0159-single-gateway-admission-and-tenant-usage-budgets.md)).
The budgets live in that pod's memory, reset when it restarts and are not
shared; the chart keeps one gateway replica, and a second gateway would
double every budget.

| Value | Default | Meaning |
| --- | --- | --- |
| `gateway.admission.mode` | `enforce` | `observe` admits everything and only counts what would be refused; use it as the incident rollback. Usage and audit records are kept either way. |
| `gateway.admission.trustedProxies` | `[]` | CIDR networks whose `X-Forwarded-For` names the client. Empty keeps the product default of loopback, private and link-local peers, which attributes each client behind an in-cluster ingress controller that forwards the client address. Pods that call the Service directly can write any source into the header. Naming only the controller's (and any load balancer's) networks stops that and also gives clients behind an appending controller their own source budgets. |
| `gateway.admission.limits` | `""` | Optional JSON document replacing provisional per-class, source, identity-cap and daily-ceiling values ([deployment contract](../../../docs/DEPLOYMENT_CONTRACT.md#request-admission-and-tenant-usage)). |

The defaults were chosen for an eight-connection gateway pool; the chart's
default `gateway.dbMaxConnections` of 20 leaves more headroom. Concurrency is
bounded per class, not across classes: one tenant may run 28 requests at once
over all six classes and the gateway 59. When raising limits, keep the summed
tenant concurrency of the classes a tenant uses together below the pool size.
Refusals are HTTP 429 with `Retry-After`; watch
`synveda_admission_decisions_total{outcome!="admitted"}` and
`synveda_admission_wait_seconds` for legitimate traffic being queued or
refused. A tenant `owner` or `administrator` reads usage with
`GET /v1/usage` and may pause or narrow a meter with an expiring, audited
`PUT /v1/usage/budgets/{meter}`; raising a ceiling is a chart value change.

## CNPG replica maintenance access

The packaged CNPG bootstrap closes PUBLIC access to the `postgres` and
`template1` databases. CNPG also uses its reserved `streaming_replica` role
for a certificate-authenticated connection to `postgres`; see the
[CNPG 1.30 security contract](https://cloudnative-pg.io/docs/1.30/security/).
CNPG creates that role after its initial `postInitSQL` PUBLIC revoke. The
required administrator bootstrap Job grants the reserved role CONNECT on
`postgres` only and verifies the remaining restrictions. A fresh
chart-rendered two-instance cluster passed bootstrap and replica restart in
an isolated Kind run. An older retained cluster without this grant can leave
a restarted replica unready with `permission denied for database "postgres"`,
even while the primary and Synveda processes serve requests.

Check `kubectl -n synveda get cluster/synveda-pg -o wide` and the unready
replica's `postgres` container logs. If that exact error appears, have the
database operator connect locally as `postgres` on the *current primary* and
run `GRANT CONNECT ON DATABASE postgres TO streaming_replica;`. Before and
after the repair, verify `has_database_privilege` on `postgres`: the reserved
replica role should change from false to true, while `synveda_gateway` and
`synveda_worker` remain false. Wait for the CNPG Cluster to report every
instance ready before treating Helm `--wait` or failover as healthy. Do not
grant CONNECT to PUBLIC or any product role. Include this ACL in restore
verification. The fixed bootstrap Job converges a retained cluster when an
upgrade uses an image containing the fix; the previously published v0.4.3
image does not. Repair the existing ACL before an upgrade that waits for all
replicas, or first use the fixed image with a controlled upgrade sequence. A
retained one-node Kind release upgraded to the fixed source image and passed
bootstrap with two ready instances. In a subsequent loss drill, a normal
primary Pod delete remained in CNPG's 30-minute termination grace while
failover waited for WAL receivers to stop; an intentional crash-style deletion
in that disposable cluster allowed promotion, two ready instances and fresh
three-gateway login/authorization. This does not establish a planned
switchover procedure, node-loss tolerance or a recovery-time target. Separate
planned maintenance from crash drills in the operator runbook.

Keep logs at `info` initially. Do not log HTTP authorization/cookie headers,
Secret manifests, credential URLs or Session/Knowledge bodies. Backup files and
Keycloak administration output are sensitive even when ordinary product audit
is content-free. Do not enable shell tracing around credential operations.
Use counters for requests, Capture outcomes, authority checks, database pools
and operation backlog; low-cardinality labels are operational signals, not a
tenant activity export. An unavailable telemetry backend can lose buffered
spans; it must not restart the application.

## Complete recovery set

| Preserve with the same generation | Why |
|---|---|
| Full Synveda PostgreSQL database, schema and SQLx metadata | Identities/grants, Sessions and payloads, Knowledge revisions, VedaFlow objects, Skills, secret envelopes, jobs/outbox and audit all live here |
| Full packaged Keycloak PostgreSQL database | Users, stable subjects, passwords/credentials, signing keys, realm/client configuration and service identities; a realm configuration export alone is insufficient |
| Database role/owner/ACL contract and credential custody | `pg_dump` does not back up cluster roles; restore exact separate owner/runtime roles before loading the archive |
| Original KMS KEK and reference; any external KMS custody | Needed to open deployment and tenant envelopes, sealed console sessions and pending OIDC/CLI handoffs; none can be reconstructed from ciphertext |
| Issuer/client configuration and client secrets, provider credentials, CA/TLS keys, saved values and artifact digests | Preserve canonical issuer and subjects; keep signing-key history through Keycloak DB or external IdP recovery |
| Adapter/client spool or unpublished local work where used | Lives on the client; it has not necessarily reached the server backup |

There is no separate authoritative server object bucket/filesystem in this
release. VedaFlow/Skill objects and durable Session payloads are database rows.
TEI model cache can be downloaded again from the same pinned model/revision;
retain offline model artifacts when downloads cannot be repeated. PostgreSQL
indexes/statistics can be rebuilt from rows (`pg_restore` rebuilds indexes;
run `ANALYZE`). Ephemeral pod files, compiled console assets and optional queue
transport state are rebuildable. Regenerating a KEK, identity subject or audit
history is not reconstruction. External database and identity administrators
own their backups, roles, retention, restores and issuer continuity; obtain
their validation evidence before treating the installation as recoverable.

## Bounded write-quiescing backup

Reserve a maintenance window. Pause integrations/SCIM writers and automation,
block new traffic at the existing edge, and exclude concurrent Helm/provider
upgrades. Gateway reads can append audit, so stopping writes in the UI alone
is insufficient. For a dedicated packaged installation:

```sh
kubectl -n synveda scale deployment/synveda deployment/synveda-worker --replicas=0
kubectl -n synveda wait pod -l app.kubernetes.io/component=gateway --for=delete --timeout=180s
kubectl -n synveda wait pod -l app.kubernetes.io/component=worker --for=delete --timeout=180s
# Use the StatefulSet name from kubectl get statefulset; starter normally uses synveda-keycloak.
kubectl -n synveda get statefulset
kubectl -n synveda scale statefulset/synveda-keycloak --replicas=0
kubectl -n synveda wait pod -l app.kubernetes.io/name=keycloakx --for=delete --timeout=180s
kubectl -n synveda get jobs -l app.kubernetes.io/component=install
```

The fixture sets `fullnameOverride: keycloak` and therefore scales
`statefulset/keycloak`; use the actual rendered name. All installation Jobs
must have completed or been explicitly stopped. Do not let GitOps immediately
reconcile the temporary scale-down. Coordinate an externally operated IdP's
quiescence with its owner; do not stop a shared service without that agreement.
If any stop/deadline fails, abandon this backup attempt and diagnose it; never
continue and call the two archives consistent.

On the administrator's workstation, choose a **new private directory outside
the database PVC and node failure domain**. The following are the native
commands exercised by the fixture (which uses a private host scratch directory).
The DBA runs them with an approved backup/admin identity; the gateway and
worker must never receive that credential. Bundled PostgreSQL and CNPG provide
local `postgres` access inside their database pod. External DBAs use their own
service/pgpass files with verified TLS:

```sh
umask 077
: "${RECOVERY_DIR:?set a new private backup directory on the administrator host}"
mkdir "$RECOVERY_DIR"
# Bundled StatefulSet, for fullnameOverride: synveda:
PGPOD=synveda-pg-0
# For the explicit CNPG mode, use this instead:
# PGPOD=$(kubectl -n synveda get cluster/synveda-pg -o jsonpath='{.status.currentPrimary}')
kubectl -n synveda exec "$PGPOD" -- psql -X -qAt -U postgres -d postgres -c "SELECT count(*) FROM pg_stat_activity WHERE datname IN ('synveda','keycloak')"
# Require zero connections to both databases before proceeding.
kubectl -n synveda exec "$PGPOD" -- timeout 120 pg_dump -U postgres -d synveda --format=custom --create --lock-wait-timeout=10s > "$RECOVERY_DIR/synveda.dump"
kubectl -n synveda exec "$PGPOD" -- timeout 120 pg_dump -U postgres -d keycloak --format=custom --create --lock-wait-timeout=10s > "$RECOVERY_DIR/keycloak.dump"
pg_restore --list "$RECOVERY_DIR/synveda.dump" > /dev/null
pg_restore --list "$RECOVERY_DIR/keycloak.dump" > /dev/null
(cd "$RECOVERY_DIR" && shasum -a 256 synveda.dump keycloak.dump > SHA256SUMS)
```

Check **each** exit status; a timeout/nonzero result leaves an invalid partial
archive. Use matching PostgreSQL 17 client tools. Keep writers stopped until
both archives, the selected Secret custody references and configuration have
been recorded. Record UTC start/end, source revision, tenant/issuer, database
version, archive hashes and last verified audit head. Do not save raw Secrets
in the ordinary evidence report. Protect the archives using the organisation's
authenticated encryption/backup tooling, escrow keys separately, and copy to
an access-controlled off-host destination with owned retention. Verify that
copy and its decryption before counting it as backup protection. The fixture's
unencrypted temporary files are test evidence only and are removed on cleanup.

Resume Keycloak first and wait for it, then scale gateway/worker to one and
wait for readiness. Remove the edge maintenance restriction last. Reconcile
GitOps only with those original replica values. Keep the last successful
backup when a later attempt fails; never apply retention cleanup automatically.

## Restore and move starter data to external services

Restore into a **new empty namespace/database server**, with a maintenance
window and the source intact. The measured fixture restores starter and
external archives into a clean independent PostgreSQL namespace, then runs
the same chart in external mode with packaged Keycloak. This is a data transfer,
not a claim of CNPG physical recovery/PITR or managed-cloud compatibility.
The drill reuses retained original Secret material; it does not simulate loss
of key custody or the whole cluster. An independently protected, recoverable
key/configuration copy remains an operator prerequisite.

1. Verify/decrypt both archives and their checksums on the recovery workstation.
   Pin the original PostgreSQL major/extension and Keycloak versions. The DBA
   provisions the exact original role names, passwords, ownership and ACL
   contract on the clean target, including separate `keycloak` ownership.
   Application migration and both writers must remain stopped.
2. Verify that the target's two databases have **zero user tables and zero
   connections**. Independently verify context, namespace and server identity.
   Do not point the following administrator command at an existing team.
3. Load both archives with PostgreSQL 17 tools. In the fixture, the new provider
   is `deployment/postgres` and the two empty bootstrap databases are replaced:

```sh
: "${RESTORE_NAMESPACE:?set the newly created disposable restore namespace}"
kubectl -n "$RESTORE_NAMESPACE" exec deployment/postgres -- psql -X -qAt -U postgres -d synveda -c "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema')"
# Repeat the emptiness check for keycloak; both counts MUST be zero.
# Destructive to the two verified empty databases only. Never use on the source.
kubectl -n "$RESTORE_NAMESPACE" exec -i deployment/postgres -- timeout 120 pg_restore -U postgres -d postgres --clean --if-exists --create --exit-on-error < "$RECOVERY_DIR/synveda.dump"
kubectl -n "$RESTORE_NAMESPACE" exec -i deployment/postgres -- timeout 120 pg_restore -U postgres -d postgres --clean --if-exists --create --exit-on-error < "$RECOVERY_DIR/keycloak.dump"
kubectl -n "$RESTORE_NAMESPACE" exec deployment/postgres -- psql -X -U postgres -d synveda -c ANALYZE
kubectl -n "$RESTORE_NAMESPACE" exec deployment/postgres -- psql -X -U postgres -d keycloak -c ANALYZE
```

4. Restore original KMS and issuer/client material from protected files using
   the installation guide's Secret commands. Update database **locations** and
   CAs in the credential files/values for the restored server, preserving exact
   usernames/ownership and original issuer/subjects. Do not change tenant UUID.
   A full Keycloak DB preserves subjects and signing keys; importing a realm
   template or recreating users does not. Keep source Keycloak stopped while
   the restored identity uses that issuer.
5. Point a controlled private resolver/proxy at the restore namespace while
   retaining the canonical issuer hostname, certificate and callback. The
   fixture reroutes only its own private proxy; it changes no host DNS. Install
   the original verified chart/image set with normal preflight, migration and
   tenant convergence. No migration metadata, RLS or policy checks are bypassed.
6. Sign in freshly as owner and member; verify stable subjects and membership,
   viewer/stranger/cross-workspace denial, a real Knowledge/Context or Skill,
   a stored Session payload, representative retrieval and audit verification.
   Open a pre-backup sealed console session to prove the restored KEK. Record
   backup point, data checks and elapsed time. Do not promote on readiness alone.
7. After verification, either restore original routing and discard only the
   disposable target, or schedule a deliberate cutover to external services.
   Allow only one set of writers and keep the source/backup through the agreed
   recovery window. Changing `postgres.mode`, a URL or `keycloak.enabled` never
   copies data. Changing the tenant's issuer/provider subjects is unsupported;
   it is not a settings edit.

For CNPG-native physical recovery and object-store/WAL retention use the
[provider's recovery procedure](https://cloudnative-pg.io/docs/1.30/recovery/)
as a separately qualified DBA operation. See the committed
[starter](../../../demos/evidence/ops11-operations-cnpg-packaged.json) and
[external](../../../demos/evidence/ops11-operations-external-external.json) evidence and the
[release record](../../../docs/backlog/OPS-11.md) for actual outcomes. Logical
dumps follow PostgreSQL's [backup](https://www.postgresql.org/docs/17/app-pgdump.html)
and [restore](https://www.postgresql.org/docs/17/app-pgrestore.html) semantics.

## Upgrade and failure recovery

The [upgrade guide](../../../docs/UPGRADING.md) owns the compatibility table
and the recovery direction for every phase. Published v0.4.6 → v0.4.7 is the
first published pair. Its Kubernetes route is **not yet qualified**: this
chart's ordering is rendered and contract-checked, but no published-chart
cluster drill has run. Older pairs keep their refusals: v0.4.0's different
`0001` checksum, and every epoch before 3.

Every chart upgrade is a planned interruption. `Recreate` stops the previous
gateway, worker and capture-only pods before new ones start. The install Job is
not a hook, so its migrate container can start while those pods are still in
their termination grace. From this chart onward, the migrator drains them: when
a run would change an installed schema, it waits until no gateway or worker
database session remains, for the longer of `gateway.shutdownSeconds` and
`worker.shutdownSeconds` plus 40 seconds
(`SYNVEDA_MIGRATION_WRITER_DRAIN_SECONDS`). If a session is still connected
after that, the Job attempt fails with phase `writer-drain` and changes nothing,
and the Job's `backoffLimit` retries it. A held lock fails within its bound,
with phase `boundary-lock` or `schema-change`, and `55P03`. New pods refuse
readiness until the new head is stamped.

Published v0.4.7 predates the drain. For v0.4.6 → v0.4.7, stop the previous
writers yourself:

1. Keep the previous chart, values and images.
2. Run the target's `synveda db migrate --check` with the **migrator**
   file/role configuration in an isolated one-off pod. Never substitute the
   gateway credential. Head `0010` is reported as upgradeable; nothing is
   written.
3. Scale the gateway, worker and capture-only Deployments to zero and wait for
   their pods to be deleted. As any database role, confirm that
   `select count(*) from pg_catalog.pg_stat_activity where usename in ('<gateway>', '<worker>');`
   returns 0.
4. Take the joint database, identity and key recovery set with the
   [bounded write-quiescing backup](#bounded-write-quiescing-backup). With
   packaged Keycloak (`keycloak.enabled`), that procedure also scales it to
   zero and requires zero sessions on both databases. With an external identity
   provider, coordinate its quiescence and recovery point with its owner
   instead. Skip the procedure's resume step: keep the gateway, worker and any
   packaged Keycloak at zero until `helm upgrade`. Restore-test the set on a
   separate target. A set taken while the previous release serves misses its
   later writes.
5. Run `helm upgrade` with the saved values. The Job applies `0011` and `0012`
   and stamps `0012`. Do not rely on the upgrade to undo the scale-down. Confirm
   that the gateway and worker Deployments, any capture-only Deployment and,
   with packaged Keycloak, its StatefulSet are at their rendered replica
   counts. Scale back any left at zero, Keycloak first, and wait for it before
   you judge gateway readiness.
6. Save the release revision and Job completion status. Repeat the public
   functional checks before you reopen traffic.

`helm rollback` changes Kubernetes manifests; it does not undo committed SQL,
restore deleted data or downgrade Keycloak/PostgreSQL. Roll back an application
only when its compatibility check accepts the current database and its provider
versions remain unchanged. Once the migrate container has started an
installed-schema change, never roll back: a previous gateway may accept an
interrupted stamp until the next Job attempt repairs it. Keep maintenance
active and roll forward with a verified fix, or restore the complete
pre-upgrade recovery set to a new target. Never reverse irreversible SQL by
editing migration history.

PostgreSQL major changes and Keycloak changes are separate maintenance work.
Keep their digests fixed on application-only upgrades. One-instance CNPG uses
`unsupervised` and may restart on an image change; Keycloak uses `OnDelete` and
requires an explicit pod replacement after a reviewed template change. Follow
provider compatibility/upgrade guidance and rehearse on restored data; this
release does not qualify major-version migration or provider downgrade.

Moving a retained database from pgvector 0.8.6 or earlier to the patched 0.8.7
is its own maintenance step. Follow the
[retained pgvector transition](../../../docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition).
In bundled and CNPG modes the new PostgreSQL image is the package replacement,
and the pod replacement is the required restart. The DBA's
`ALTER EXTENSION vector UPDATE TO '0.8.7'` follows the application upgrade.
After that update, a release without ADR-0153 refuses readiness, so
`helm rollback` alone cannot return to it. Restore the pre-transition
recovery set instead.

## Safe uninstall and explicit purge

`helm uninstall synveda -n synveda --wait --timeout 5m` removes application
Deployments/Services/Jobs, exposure/policies and packaged Keycloak resources.
The starter's `postgres.retain: true` keeps the CNPG Cluster running, its
operator-owned PVCs and generated database Secrets. Operator-created KMS,
OIDC, TLS and database Secrets remain. Independently operated providers remain
untouched. Reinstall with the same release/namespace, tenant, issuer, credentials
and provider images. The live fixture compares PVC UIDs and credential hashes
across this operation and verifies login/content after reinstall.

Outside the starter, `postgres.retain` defaults to **false**: uninstall can
delete the Cluster and its owned data/Secrets. An optional chart-created TEI
cache PVC is deleted by uninstall; an explicitly existing cache claim remains.
Inspect `helm get manifest` and the actual PVC ownerReferences/PV reclaim
policy before choosing uninstall. `Delete` may destroy backing storage;
`Retain` leaves storage for manual recovery. Neither is a backup. Namespace
deletion can remove even a Helm-retained Cluster, claims and all Secrets.

**DESTRUCTIVE PURGE — permanently removes every resource and Secret in the
named dedicated namespace, including retained claims.** Obtain a verified
off-host backup and explicit ownership confirmation first. This is never called
by an installer, upgrade or normal uninstall. Independently owned dependencies
outside the namespace require their administrators' separate decommissioning.

```sh
: "${PURGE_NAMESPACE:?set the dedicated namespace to destroy}"
: "${CONFIRM_PURGE:?set exactly purge:<namespace>:all-data-and-secrets}"
test "$CONFIRM_PURGE" = "purge:$PURGE_NAMESPACE:all-data-and-secrets" || exit 1
kubectl delete namespace "$PURGE_NAMESPACE" --wait=true --timeout=180s
```

## Tenant lifecycle Job

Set `tenantLifecycle.enabled=true`, `tenantLifecycle.command` (`suspend`,
`resume`, `inventory` or `erase`), `tenantLifecycle.tenant` and
`tenantLifecycle.reason`, then run `helm upgrade` with the saved values. The
revision runs one Job named `<release>-tenant-<command>-<revision>`:

- it receives the migrator Secret, the role ConfigMap and the KMS Secret;
- it uses the install Job's placement and its own NetworkPolicy, which
  allows DNS and the database only;
- it does not retry; to retry, upgrade again. An interrupted erase continues;
- `erase` also needs `tenantLifecycle.force=true` and
  `tenantLifecycle.fixtureRules=true`, and is irreversible.

Read the result with `kubectl -n synveda logs job/<name>`, then set
`enabled=false` on the next upgrade. Clear `install.tenant` before suspending
or erasing the tenant it names: the install Job's convergence refuses a
tenant whose status or name changed, which stops the upgrade. Archive export and import are not
wrapped, because no product container in this chart receives an
administrator credential. See the
[tenant lifecycle guide](../../../docs/TENANT_LIFECYCLE.md).

## Troubleshooting without printing credentials

| Symptom | Check and action |
|---|---|
| Issuer/redirect mismatch or login loop | Compare the public origin, exact `/auth/callback`, discovery issuer and provider client settings privately. Preserve issuer/subject identity; fix DNS/proxy/client configuration, never token verification. |
| Internal CA/hostname failure | Verify SAN/chain and mount the right PostgreSQL or OIDC PEM Secret. Use `openssl s_client -connect HOST:443 -servername HOST -CAfile private/ca.pem -verify_return_error`; do not use insecure TLS switches. Roll pods after CA changes. |
| PostgreSQL unavailable | Check network/DNS, verified TLS and bounded `database-preflight` logs. DBA verifies three distinct ordinary logins, target identity and peer isolation. |
| Extension ownership/catalogue refusal | Follow the [extension diagnostic path](#extension-ownership-and-catalogue-refusals). Public extensions belong to declared trusted administrators; migration/readiness perform the full proof after role/target preflight. |
| Migration Job failure | `helm history synveda -n synveda`; inspect the matching `job/synveda-install-REVISION` preflight/migrate logs and Job status. Check a competing migration/lock holder with the DBA. Keep maintenance active; preserve data and use compatibility/restore guidance. |
| Insufficient quota | `kubectl -n synveda get resourcequota,limitrange` and warning events. Request headroom for the install Job and provider pods; do not remove memory limits blindly. |
| Pending PVC | `kubectl -n synveda describe pvc CLAIM`; inspect events, storage class, capacity, access mode and CSI permissions with the storage owner. Never delete a populated claim to unstick scheduling. |
| ImagePullBackOff | Inspect pod events, actual image digest/architecture and pull-Secret **name** in the PodSpec. Verify namespace-local registry access privately. Missing candidate images are a release failure. |
| Restricted SCC rejection | Inspect OpenShift admission events and required SCC; use the assigned-ID overlay and, for v3, its post-renderer. Check CSI/CNPG qualification. Do not grant privileged/anyuid SCC to pass. |
| Blocked egress | Check enforcing NetworkPolicies, real selectors/CIDRs/ports, DNS and proxy/NO_PROXY coverage. OIDC intentionally bypasses HTTP proxy settings; allow its canonical endpoint directly. |
| Restart loop | Inspect container exit code, OOMKilled/events and bounded `kubectl logs POD --previous --tail=100`. Correct quota, Secret mounts, authority or configuration errors. Health is separate from optional telemetry export warnings. |

Never paste `kubectl get secrets -o yaml`, `helm get all` containing private
operator inputs, raw database URLs or unredacted provider diagnostics into a
support report. Supply version/profile, timestamps, pod status and content-free
error codes instead.

### Extension ownership and catalogue refusals

Have the DBA follow the [ownership rule](CONFIGURATION.md#extension-ownership),
[managed-provider procedure](CONFIGURATION.md#managed-postgresql-provisioning)
and protected read-only inventory in the [DBA handoff](CONFIGURATION.md#dba-handoff).
Identify the failing stage before changing privileges:

1. **Bundled/CNPG `database-bootstrap`:** public extension `extowner` must
   equal the bootstrap administrator's `session_user`; the bootstrap also
   requires `current_user = session_user`. Its administrator credential is
   separate from the migrator credential. `SET ROLE synveda_migrator` changes
   the effective role and cannot satisfy this bootstrap contract.
2. **`database-preflight` or external `database-probe`:** inspect ordinary
   roles, declared administrator/grantor identities, cluster identity, peer
   isolation, TLS and connectivity. This stage does not prove the full
   extension catalogue. A Complete probe still needs normal migration and
   runtime readiness.
3. **`migrate` or runtime authority/readiness:** inspect the intended database's
   exact versions, schemas and owners. `vector` and `btree_gin` in `public`
   belong to a declared administrator; the migrator owns application objects
   and has no role memberships. Verify member ownership as well: a provider's
   trusted installation can assign members a different owner despite successful
   creation. Normal migration and runtime checks are the authoritative proof.

Provider allowlisting, temporary privileges or `CREATE EXTENSION IF NOT EXISTS`
do not repair an existing ownership/catalogue mismatch. Removing temporary
provider membership from a migrator that created an extension leaves the
extension migrator-owned and refused. Preserve retained data, credentials and
recovery material; have the provider DBA diagnose the exact unmet contract.
Do not elevate an application role, add event triggers or edit the catalogue
to pass. A provider that cannot meet the full contract remains unqualified.

`synveda db migrate` prints a content-free warning while the server's
installed pgvector package is older than 0.8.7, or while the catalogue still
uses a legacy profile. Treat it as the next step of the
[retained transition](../../../docs/DEPLOYMENT_CONTRACT.md#retained-pgvector-transition).
It does not block migration.

## Installation-stage recovery

Run commands on the workstation with the **saved** `CONTEXT`, `NAMESPACE`,
`RELEASE`, `CHART` and `PREPARED`. Start with an allowlisted summary:

```sh
node "$CHART/examples/operator.mjs" diagnose --prepared "$PREPARED" > synveda-support.json
```

It includes only workload names/phases, container names/states/restarts,
known reason codes, exit codes, Job counters/conditions and PVC class/size/phase.
It excludes annotations, messages/events, logs, environments, Secrets,
credential URLs, tokens/cookies and Session/Knowledge content. Unknown reason
strings are classified rather than copied. API permission failure names the
required list permission. Read raw logs/events privately; never append them
unreviewed to this support file.

| Failing stage/symptom | Likely cause | Exact next action | Retry |
| --- | --- | --- | --- |
| Preparation conflict/partial file | Different selections, changed recovery material, interrupted derived output | Preserve the directory; restore original `state.json`/overlay/files. If only a derived output is absent, repeat the original preparation. Remove an empty lock only after its process has stopped. | Original `prepare.mjs` command with identical inputs |
| API/RBAC refusal | Namespaced installer lacks a listed permission | Request exactly that resource/verb from the namespace owner. Namespace/StorageClass listing is cluster-wide; obtain the specified administrator input when unavailable. | `operator.mjs preflight` |
| Pending PVC | Missing class, quota/capacity, scheduling or storage driver | `kubectl --context "$CONTEXT" -n "$NAMESPACE" describe pvc "$RELEASE-pg-data"` privately; confirm class/provisioner with storage owner. WaitForFirstConsumer needs a scheduled consumer, so inspect its scheduling too. Preserve populated claims. | Preflight then original Helm command |
| ImagePullBackOff/ErrImagePull | Wrong digest/platform, registry refusal or missing pull Secret | Inspect private Pod events. Confirm verified inventory/digest and architecture, namespace-local pull Secret keys and Pod registry egress. A source tag or workstation pull does not prove Pod pullability. | Correct referenced pull Secret/registry access, then original Helm command |
| CreateContainerConfigError / Secret/key failure | Secret missing in this namespace, wrong key or conflicting credential | Restore the original matching protected Secret/key. `operator.mjs secrets` creates only absent generated Secrets and refuses conflicting bytes before writing. External Secrets remain their owner's responsibility. | Secrets action, preflight, then original Helm command |
| Quota / LimitRange / FailedCreate | Missing install/replacement/verification headroom or required CPU limit | `kubectl --context "$CONTEXT" -n "$NAMESPACE" get resourcequota,limitrange`; use the printed per-recipe requests and limits. Resolve prior failed temporary probe Pods explicitly after diagnosis. | Preflight then original Helm command |
| `database-bootstrap` | Bundled/CNPG provisioning or administrative credential/authority refusal | Inspect bounded logs from that init container privately. Check exact separate credentials, owner/ACL/extension contract and current provider readiness. Do not grant elevated authority to a runtime role. | Correct provider input; original Helm command creates a new revision Job |
| `database-preflight` / external probe | Role, same-server proof, forbidden database, TLS/CA/hostname or pod connectivity refusal | Give the DBA its content-free reason/SQLSTATE. Verify the [DBA handoff](CONFIGURATION.md#dba-handoff), CA/SAN and Pod network path. Never weaken the authority verifier. | New external `database-probe`, require Complete, then original Helm command |
| `migrate` | Wrong schema/checksum/owner role, extension ownership/catalogue refusal or competing migration | Inspect bounded migrate logs privately; DBA checks lock, [extension proof](#extension-ownership-and-catalogue-refusals) and exact compatibility. Use the migrator-only `db migrate --check` before a planned upgrade. Keep writers quiesced for recovery. | Verified compatible artifact and original Helm command; no SQLx history edits |
| `tenant` | Original tenant/key material missing or conflicting | Restore the original tenant ID, KMS KEK/reference and issuer binding together. Do not generate a new tenant/key to bypass refusal. | Original Helm command after matching recovery is restored |
| Readiness/TLS | Schema/key/policy gate, wrong CA/chain/SAN or startup | `operator.mjs verify` plus allowlisted diagnosis; privately inspect the exact failing container and trust files. Check expiry/hostname and certificate owner's chain. No insecure TLS switch. | Correct input, planned restart/reapply, then verify |
| OIDC login/callback | Issuer/callback/audience/group mismatch, DNS, CA or provider SSO | Compare the [OIDC worksheet](CONFIGURATION.md#external-oidc-worksheet) and chosen human subject privately. Register exact callback; restore canonical issuer. Fresh private browser login avoids stale SSO. | First-login procedure and `whoami --capabilities`; readiness alone is insufficient |

To locate the current install Job, `kubectl --context "$CONTEXT" -n
"$NAMESPACE" get jobs -l app.kubernetes.io/component=install` shows its revision
suffix. The failing init/container in diagnosis names the stage. Private logs
use `kubectl ... logs job/JOB-NAME -c STAGE --tail=30`; replace both names with
those actual names. Never paste a credential URL or provider token if a log
contains one. The normal order stays bootstrap → authority preflight → migration
→ tenant. Correction and a new Helm revision re-execute it under bounded waits.
A Helm failure does not undo a completed migration or destroy retained data.

After infrastructure recovery, repeat authenticated sign-in, selected-project
access and the [first-use outcome](FIRST_USE.md). Record those separately from
Job Complete, `/readyz` or `helm test` success. No automatic purge, credential
regeneration or namespace deletion is an installation recovery action.

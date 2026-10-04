# Kubernetes operations

Use the [installation guide](README.md) and save the exact chart, values,
immutable image overlays and Secret custody references. This runbook covers
one gateway and one combined worker, with optional capture-only capacity and
planned maintenance. It does not establish HA,
WAL/PITR, zero downtime or a promised RPO/RTO. Compose keeps its existing
[backup/restore/upgrade entry points](../../compose/README.md).

The optional [OPS-5 CNPG object-store backup candidate](BACKUP.md) adds chart
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

Reserve one extractor call and `worker.dbMaxConnections` database connections
per extra pod, plus enough capacity for a replacement during rollout. The
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

OIDC login now parks its PKCE state and CLI handoff in the same deployment-key
plane as console sessions. A missing deployment key refuses `/auth/login`
before redirect; restore the matching key with PostgreSQL so unexpired logins
can complete after a gateway restart. The isolated cross-process test exercises
this source path. An OPS-7 one-node Kind drill used three ready pods to route
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
kubectl -n synveda port-forward service/synveda 8120:8120
# In a second terminal; the management endpoint stays on local loopback:
curl --fail --silent http://127.0.0.1:8120/readyz
curl --fail --silent http://127.0.0.1:8120/metrics
```

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
   copies data. Changing the tenant's issuer/provider subjects is unsupported
   pending MEM-7; it is not a settings edit.

For CNPG-native physical recovery and object-store/WAL retention use the
[provider's recovery procedure](https://cloudnative-pg.io/docs/1.30/recovery/)
as a separately qualified DBA operation. See the committed
[starter](../../../demos/evidence/ops11-operations-cnpg-packaged.json) and
[external](../../../demos/evidence/ops11-operations-external-external.json) evidence and the
[OPS-11 record](../../../docs/backlog/OPS-11.md) for actual outcomes. Logical
dumps follow PostgreSQL's [backup](https://www.postgresql.org/docs/17/app-pgdump.html)
and [restore](https://www.postgresql.org/docs/17/app-pgrestore.html) semantics.

## Upgrade and failure recovery

There is **no supported upgrade between published versions**. Public v0.2.0 has
the retired schema. Published v0.4.0 and v0.4.3 both use epoch 3 and baseline
revision 3, but their `0001` checksums differ and remain incompatible. The
source candidate now preserves the exact v0.4.3 baseline and adds `0002` under
ADR-0121 and `0003` under ADR-0126; its read-only check identifies exact
`0001` and `0002` heads as upgradeable
after quiescence, not ready for the new binary to serve. Keep the current
installation and its data until the published-artifact and joint recovery drill
qualifies this path. Same-source Helm migration reruns and retained reinstall
are continuity checks, not cross-release upgrades. OPS-6 owns the declared
compatibility window.
CI and Release retain four-mode install/upgrade/reinstall checks and the two
end-to-end recovery drills. Passing those checks does not establish migration
compatibility between two different published versions.

For a candidate pair whose compatibility has been demonstrated: take and
restore-test the joint backup first, retain the complete previous artifacts,
compare rendered changes, and quiesce the installation. Run the candidate's
`synveda db migrate --check` with the **migrator** file/role configuration in an
isolated one-off pod before normal installation; never substitute the gateway
credential. This command proves schema/authority compatibility without DDL.
The normal bounded install Job then serializes migrations using SQLx's advisory
lock and repeats tenant/key admission. Save the release revision and completion
status and repeat the public functional checks before reopening traffic.

`helm rollback` changes Kubernetes manifests; it does not undo committed SQL,
restore deleted data or downgrade Keycloak/PostgreSQL. Roll back an application
only when its compatibility check accepts the current database and its provider
versions remain unchanged. Otherwise keep maintenance active and roll forward
with a verified fix, or restore the complete pre-upgrade recovery set to a new
target. Never reverse irreversible SQL by editing migration history.

PostgreSQL major changes and Keycloak changes are separate maintenance work.
Keep their digests fixed on application-only upgrades. One-instance CNPG uses
`unsupervised` and may restart on an image change; Keycloak uses `OnDelete` and
requires an explicit pod replacement after a reviewed template change. Follow
provider compatibility/upgrade guidance and rehearse on restored data; this
release does not qualify major-version migration or provider downgrade.

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

## Troubleshooting without printing credentials

| Symptom | Check and action |
|---|---|
| Issuer/redirect mismatch or login loop | Compare the public origin, exact `/auth/callback`, discovery issuer and provider client settings privately. Preserve issuer/subject identity; fix DNS/proxy/client configuration, never token verification. |
| Internal CA/hostname failure | Verify SAN/chain and mount the right PostgreSQL or OIDC PEM Secret. Use `openssl s_client -connect HOST:443 -servername HOST -CAfile private/ca.pem -verify_return_error`; do not use insecure TLS switches. Roll pods after CA changes. |
| PostgreSQL unavailable/extension refusal | Check network/DNS and bounded `database-preflight` logs. DBA verifies PostgreSQL 17, exact extension versions/owners, three distinct roles and provider identity proof. Do not grant superuser/BYPASSRLS to the application. |
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
| `migrate` | Wrong schema/checksum/owner role or competing migration | Inspect bounded migrate logs privately; DBA checks lock and exact compatibility. Use the migrator-only `db migrate --check` before a planned upgrade. Keep writers quiesced for recovery. | Verified compatible artifact and original Helm command; no SQLx history edits |
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

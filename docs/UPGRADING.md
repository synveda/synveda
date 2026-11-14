# Upgrade and roll back a release

This guide is the upgrade contract for an installed Synveda deployment. It
covers the supported release pairs, the planned interruption, the installed-host
command, Helm ordering, and the recovery direction for every phase.
[ADR-0156](adr/adr-0156-quiesced-release-upgrade-and-restore-rollback.md)
records the decision. [ADR-0121](adr/adr-0121-preserve-v0-4-3-through-forward-migration.md)
and [ADR-0152](adr/adr-0152-postgresql-14-to-18-portable-lineage.md) define the
two schema lineages it relies on.

Every upgrade is a **planned maintenance interruption**. One gateway and one
core worker serve at a time, and no previous binary serves beside a new one.
There is no zero-downtime, mixed-version or down-migration path. Rollback
after a schema change means restoring the recovery set made before it.

## Compatibility table

| Starting release | Server and extension | Ledger lineage and head | Target | Target head | Interruption | Recovery direction | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| v0.4.6 (`58009f38`) | PostgreSQL 17 only. Bundled 17.11 with pgvector 0.8.6, or external 17 with a 0.8.2–0.8.6 catalogue. | Published `0001`–`0010` | v0.4.7 (`42308fd6`) | Published `0011`–`0012`, stamp `0012` | Gateway, worker and bundled Keycloak stop from writer stop until target readiness. | Before the `migrating` phase: restart v0.4.6, or resume. After it: roll forward with v0.4.7, or restore the pre-upgrade set with the v0.4.6 bundle into an empty installation. | Published bundles and images with the candidate launcher on one Linux Docker host. The published-commands route below was also drilled with both published launchers. See [evidence](#evidence). Kubernetes is not yet qualified. |
| v0.4.7 (`42308fd6`), upgraded from v0.4.6 or installed fresh | PostgreSQL 17 on the published lineage; 14–18 on the portable lineage. pgvector 0.8.7, or a dated legacy profile. | Published `0001`–`0012`, or portable `0011`–`0012` | v0.4.8 (`5715df5f`) | `0013`–`0018` on the same lineage, stamp `0018` | Same | Before the `migrating` phase: restart v0.4.7, or resume. After it: roll forward with v0.4.8, or restore the pre-upgrade set with the v0.4.7 bundle into an empty installation. | Source tests advance both lineages from `0012` to `0018`. Published bundles, images and launchers upgraded both lineages on one Linux arm64 Docker engine: a fresh v0.4.7 installation (portable), and a v0.4.7 upgraded from v0.4.6 (published), on both the legacy 0.8.6 and the 0.8.7 catalogue. See [evidence](#evidence). Kubernetes is not yet qualified. |
| v0.4.8 (`5715df5f`), upgraded from v0.4.7 or installed fresh | PostgreSQL 17 on the published lineage; 14–18 on the portable lineage. pgvector 0.8.7, or a dated legacy profile. | Published `0001`–`0018`, or portable `0011`–`0018` | v0.4.9 (`4f692bda`) | `0019` on the same lineage, stamp `0019` | Same | Before the `migrating` phase: restart v0.4.8, or resume. After it: roll forward with v0.4.9, or restore the pre-upgrade set with the v0.4.8 bundle into an empty installation. | Source tests advance both lineages from `0018` to `0019`. The published pair has not been drilled. Kubernetes is not yet qualified. |
| v0.4.3 to v0.4.5 (`0001`–`0009` heads) | PostgreSQL 17 | Published prefixes | v0.4.7 | `0012` | Same | Same | None qualified. The v0.4.7 migrator advances each exact prefix in source tests, but no published pair from these releases has been drilled, including through v0.4.6. |
| v0.4.0 | PostgreSQL 17 | Different `0001` checksum | None | — | — | Refused before any write. Keep the installation. | Source tests |
| Before schema epoch 3 (v0.2.0 and earlier) | — | No marker | None | — | — | Refused with reset guidance only. There is no data migrator. | Source tests |

The table is closed. A combination outside it is unsupported, even if the
migrator happens to accept it.

Rules that hold for every row:

- **The PostgreSQL major stays the same.** A server-major upgrade is separate
  DBA work, done first with the server's own tooling. The application
  migration never changes the major.
- **Extensions are DBA work.** A bundled PostgreSQL image may replace the
  native pgvector package on restart. That is the first half of the
  [retained pgvector transition](DEPLOYMENT_CONTRACT.md#retained-pgvector-transition),
  and both releases admit it. The DBA's `ALTER EXTENSION vector UPDATE TO
  '0.8.7'` waits until the target release is accepted. After that update,
  rollback means a restore only.
- **The Keycloak version stays the same.** v0.4.6 to v0.4.9 all ship
  Keycloak 26.7.2. The v0.4.7 image was rebuilt with inputs that differ only by
  a comment in a copied build helper; the v0.4.8 and v0.4.9 inputs are
  unchanged from v0.4.7.
- **Lineages never mix.** A published ledger stays on the published lineage
  and a portable one on the portable lineage. A marker that names a head
  before or after the committed ledger is refused as drift without writes.

## What protects the database during an upgrade

The migrator, not only the procedure, enforces the ordering:

| Phase | Bound | On failure |
| --- | --- | --- |
| `boundary-lock` | SQLx's advisory lock, server-side `lock_timeout` 60 s | `55P03`. Another migrator holds the boundary. Nothing changed. Rerun. |
| `preflight` | Exact marker and SQLx ledger proof | Drift or an unknown head is refused without writes or reset advice. |
| `writer-drain` | Installed-schema changes only. Waits for zero sessions of the gateway and worker logins, 120 s by default (`SYNVEDA_MIGRATION_WRITER_DRAIN_SECONDS`, 1–600). | Previous writers are still connected. Nothing changed. Stop them, then rerun. |
| `authority` | Head-specific role, ACL, extension, routine and forced-RLS proof | Refused before DDL. |
| `schema-change` | Each migration in its own transaction. Lock waits are bounded at 10 s. | `55P03` names the migration. The transaction rolls back. Stop the blocking session, such as a backup or a long query, then rerun. |
| `stamp` | Post-migration authority proof, then the marker stamp | A committed ledger with a trailing stamp is resumed by the next run, which repairs only the stamp. |

`synveda db migrate` prints the starting and stamped heads with each wait and
duration. The gateway and worker re-verify the stamp on startup and every 15
seconds. They refuse to serve a head they do not support, in either
direction. Migrations need no data backfill in this window. A future release
that needs one must make it bounded and restart-safe before it is added here.

Published v0.4.7 predates the writer drain and the lock bounds, so the
procedures below stop previous writers explicitly. The drill showed why the
server-side bound matters. Killing the published v0.4.7 migrator client during
a lock wait left its server backend waiting and still holding the boundary
until PostgreSQL ended it.

In the published v0.4.7 → v0.4.8 drill, both protections fired from the
v0.4.8 launcher. A session held a read lock on `public.scopes`, and `0014`
then failed after 10 seconds with `55P03`. `0013` stayed committed and the
marker stayed at `0012`. A planted session under the gateway login then held
the next run in `writer-drain` for 120 seconds. That run refused with the
database unchanged. The same command then resumed to completion.

## Installed host (Docker evaluation bundle)

Prerequisites: the installed release's extracted bundle and its private state
(`$SYNVEDA_HOME/state`), the verified target bundle, and enough disk space for
one logical backup. Copy the installed bundle's `evaluation.json` values
(port, subnet and `demoAccounts`) into the target bundle unchanged;
preparation refuses a changed installation. Keep the installed bundle; its recovery utility restores
the pre-upgrade set.

From the **target** bundle:

```sh
SYNVEDA_UPGRADE_FROM=/path/to/synveda-reference-0.4.8 \
SYNVEDA_CONFIRM_UPGRADE=upgrade:synveda-evaluation:before-0-4-9 \
  ./synveda-compose upgrade before-0-4-9
```

v0.4.8 is the first release that ships the command, so v0.4.7 → v0.4.8 is
the first pair that can use it. The published pair was drilled on both
lineages: from a fresh v0.4.7 installation (portable), and from a v0.4.7 that
was itself upgraded from v0.4.6 (published). v0.4.8 → v0.4.9 uses the same
command and has not been drilled from the published bundles. The command runs
these phases, each recorded with a timestamp in `state/upgrades/<backup-id>`:

1. `started`: checks that the installation runs the release in
   `SYNVEDA_UPGRADE_FROM` and that the backup id is new.
2. `writers-stopped`: stops the gateway, worker and Keycloak with the installed
   release's own Compose graph.
3. `backed-up`: makes the logical PostgreSQL and Keycloak set with the
   installed release's PostgreSQL image and recovery utility. Writers stay
   stopped, so the set is the exact pre-upgrade state.
4. `prepared`: prepares the target state, recreates PostgreSQL with the target
   image and reruns the database and identity bootstrap.
5. `migrating`: runs the target's `database-preflight` and read-only
   `migration-check`, then one ordinary `migrate`.
6. `migrated`, then `complete`: starts the target and waits for every health
   check.

Rerunning the same command resumes from the last completed phase. Before the
schema change it refuses to resume if a gateway or worker is running again,
because the backup may then be stale. Start a new upgrade with a new backup id
instead.

### v0.4.6 to v0.4.7 with only the published commands

The published v0.4.7 launcher has no `upgrade` action. The equivalent safe
order uses commands both bundles ship:

1. Announce maintenance and stop clients.
2. From the v0.4.6 bundle: `./synveda-compose backup before-0-4-7`. It stops
   the gateway, worker and Keycloak for the dump, then restarts the whole
   stack when it finishes.
3. As soon as the backup returns, from the v0.4.6 bundle:
   `./synveda-compose stop`, which stops every service.
4. From the v0.4.7 bundle, with the same `evaluation.json` values:
   `./synveda-compose up`. PostgreSQL starts first,
   and the v0.4.7 migrator runs `0011` and `0012` before the gateway and worker
   start.
5. Accept the target: sign in, check the expected workspace and run your
   public API checks.

This route cannot keep writers stopped across the backup. Between the restart
that ends step 2 and step 3, the v0.4.6 gateway, worker and Keycloak run again.
Anything they write in that interval, such as background work, is not in the
set, and a restore of the set loses it. Keeping clients stopped from step 1 leaves only
the services' own background work in that interval. In the published-lineage
drill, v0.4.6 gateway and worker sessions were back for about 14 seconds, and
the gateway reported ready for about 4 of them, before `stop` took effect. The
`upgrade` command
closes the gap, keeping writers stopped from before the backup until the
migration, for pairs whose target bundle ships it.

Never run the v0.4.7 `up` while the v0.4.6 gateway or worker is running. That
`up` runs the new migrator before it replaces the old writers.

## Recovery by phase

| Interrupted at | Database state | Supported recovery |
| --- | --- | --- |
| `started` or `writers-stopped` | Unchanged | Rerun the command, or restart with the installed bundle's `up`. |
| `backed-up` or `prepared`: preflight, check or bootstrap failure | Schema unchanged. Bundled PostgreSQL may run the target image, which both releases admit. | Fix the cause and rerun, or restart with the installed bundle's `up`. A restart discards this attempt; use a new backup id next time. |
| `migrating`: lock wait, cancelled client, lost database | The migration transaction rolled back, or committed up to a migration boundary. | Roll forward: rerun the same command. Do not restart the installed release; in the drill, v0.4.6 refused the changed schema at its migrator and at its gateway's authority proof. |
| `migrating`: committed migrations, trailing stamp | Ledger at the target head, marker at the old head | Rerun. The migrator repairs only the stamp. Do not start the installed release: its readiness depends on catalogue proofs, not on the marker. |
| `migrated`: new process start failure | Target schema | Rerun, or fix the configuration and run `up`. |
| After `complete`, target not accepted | Target schema | Restore the pre-upgrade set with the installed bundle into an empty installation, after backing up the upgraded state if you need it. |

A restore uses the installed bundle and needs an absent database volume:

```sh
./synveda-compose backup after-0-4-9            # from the target bundle, optional
SYNVEDA_CONFIRM_RESET=delete:synveda-evaluation:postgres-data \
  ./synveda-compose reset                       # from the target bundle
SYNVEDA_CONFIRM_RESTORE=restore:synveda-evaluation:before-0-4-9 \
  ./synveda-compose restore before-0-4-9        # from the installed bundle
```

Keep the installed bundle, its images and the `before-0-4-9` set until the
target release is accepted and a new recovery set of the upgraded installation
passes its own verification.

## Kubernetes

The chart keeps `Recreate`, one gateway and one combined worker. The
[operations guide](../deploy/helm/synveda/OPERATIONS.md#upgrade-and-failure-recovery)
has the v0.4.6 → v0.4.7 steps: scale the writers to zero, confirm no runtime
sessions, take the recovery set, then `helm upgrade`. From the chart that carries the writer drain
onward, the install Job's migrator waits out the previous pods' termination
grace by itself. `helm rollback` never reverses committed SQL. No published
chart pair has been drilled on a cluster yet.

## External PostgreSQL

The DBA owns the server major, extensions, backups and restores. Before the
upgrade, confirm that the target admits the server major and extension
profile (see [PostgreSQL compatibility](DEPLOYMENT_CONTRACT.md#postgresql-compatibility)).
Stop the gateway and worker first, together with a bundled or packaged Keycloak
when the deployment runs one. Confirm that no session of the gateway, worker or
Keycloak login remains in `pg_catalog.pg_stat_activity`. With an external
identity provider, coordinate its quiescence and recovery point with its owner;
do not stop a shared service without that agreement. Then take the provider's
joint backup of the product database, identity state and key material. Keep
those services stopped and run the target's `synveda db preflight`,
`synveda db migrate --check` and `synveda db migrate` as the migrator. Rollback
after the schema change is the provider's restore of that backup onto the
previous release.

## Proposed support window and outage budget

No owner decision sets these yet. The proposal: support an upgrade from the
previous published release only, and plan a maintenance window of 15 minutes
for the bundled single-host route. Two clean runs have been measured, each
on a small dataset:

- **v0.4.6 → v0.4.7**, on a linux/amd64 Docker Desktop VM. Writers were down
  for 377.9 seconds, from writer stop until the new gateway was ready, and 384
  seconds passed until every service was healthy.
- **v0.4.7 → v0.4.8**, on a linux/arm64 engine. The readiness gap was 225
  seconds, sampled every 2 seconds, and the total was 230 seconds.

| Step | v0.4.6 → v0.4.7, seconds | v0.4.7 → v0.4.8, seconds |
| --- | --- | --- |
| Stop writers and Keycloak | 13 | 13 |
| Logical backup | 2 | 1 |
| Prepare the target | 1 | 0 |
| Recreate PostgreSQL, bootstrap, preflight, read-only check | 14 | 9 |
| `migrate` | 1 (`0011` 6.3 ms, `0012` 3.6 ms of DDL) | 1 (`0013`–`0018`, 0.3 s in the migrator) |
| Start Keycloak, converge the realm, start the gateway and worker | 353 | 206 |

A restore of the pre-upgrade set took 382 and 218 seconds. Keycloak's restart
dominates both, and both datasets were small, so DDL time on a large
installation is not established. The owner still has to choose the window,
the budget and the minimum rehearsal dataset.

## Evidence

The [published-pair upgrade drill](../demos/evidence/ops6-published-upgrade-drill.json)
records artifact identities, phase timestamps, injected failures, digests and
measurements for the v0.4.6 → v0.4.7 run on one Windows/Docker Desktop
Linux VM. It is single-host evidence: a small product dataset, logical backup
on the same host, no off-host recovery set and no Kubernetes drill.

The [v0.4.7 → v0.4.8 upgrade drill](../demos/evidence/ops12-048-upgrade-drill.json)
used both releases' published bundles, images and launchers. It ran on one
Linux arm64 Docker engine inside a container on macOS, and its v0.4.7
installation was fresh, so it covers the portable lineage only. It records:

- the lock-bound and writer-drain refusals, and the resumes after them;
- the v0.4.7 gateway, check and migrator refusing `0018`;
- an interrupted stamp, where the v0.4.8 migrator changed only the marker;
- a restore of the pre-upgrade set with the v0.4.7 bundle;
- a clean, timed second upgrade. Its before and after snapshots match apart
  from the ledger and the marker.

Its limits are the same as those of the first drill.

The [published-lineage drill](../demos/evidence/ops6-published-lineage-drill.json)
ran the whole published path on the same kind of engine, with each release's
own bundle, images and launcher. A fresh v0.4.6 installation (published
`0001`–`0010`, pgvector 0.8.6) was seeded with v0.4.6's own fixtures. It then:

- upgraded to v0.4.7 with only the published commands, as described above.
  Every data table and the audit prefix were unchanged, and the v0.4.6 writers'
  return between the backup and `stop` was measured;
- upgraded to v0.4.8 with the `upgrade` command on the legacy 0.8.6 catalogue,
  in 244 seconds end to end. Before verify, the snapshots matched apart from
  the ledger and the marker;
- showed the v0.4.7 gateway, check and migrator refusing `0018`, and an
  interrupted stamp repaired by the v0.4.8 migrator alone;
- ran the DBA's `ALTER EXTENSION vector UPDATE TO '0.8.7'`, a no-op migrate and
  a verified restart;
- restored the pre-upgrade set with the v0.4.7 bundle. The logical restore
  recreated the catalogue at 0.8.7, and v0.4.7 accepted it;
- upgraded again with a held table lock and a planted writer. Each refused
  without loss, and the same command then resumed to complete.

Its limits are the same as those of the first drill.

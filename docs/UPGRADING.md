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
| v0.4.6 (`58009f38`) | PostgreSQL 17 only. Bundled 17.11 with pgvector 0.8.6, or external 17 with a 0.8.2–0.8.6 catalogue. | Published `0001`–`0010` | v0.4.7 (`42308fd6`) | Published `0011`–`0012`, stamp `0012` | Gateway, worker and bundled Keycloak stop from writer stop until target readiness. | Before the `migrating` phase: restart v0.4.6, or resume. After it: roll forward with v0.4.7, or restore the pre-upgrade set with the v0.4.6 bundle into an empty installation. | Published bundles and images with the candidate launcher on one Linux Docker host. See [evidence](#evidence). Kubernetes is not yet qualified. |
| v0.4.7, fresh install on PostgreSQL 14–18 | 14–18. pgvector 0.8.7, or a dated legacy profile. | Portable `0011`–`0012` | The next release | Byte-identical appended copies | Same | Same | No published successor yet |
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
- **The Keycloak version stays the same.** v0.4.6 and v0.4.7 both ship
  Keycloak 26.7.2. The bundled image was rebuilt; its inputs differ only by
  a comment in a copied build helper.
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

## Installed host (Docker evaluation bundle)

Prerequisites: the installed release's extracted bundle and its private state
(`$SYNVEDA_HOME/state`), the verified target bundle, and enough disk space for
one logical backup. Copy the installed bundle's `evaluation.json` values
(port, subnet and `demoAccounts`) into the target bundle unchanged;
preparation refuses a changed installation. Keep the installed bundle; its recovery utility restores
the pre-upgrade set.

From the **target** bundle:

```sh
SYNVEDA_UPGRADE_FROM=/path/to/synveda-reference-0.4.6 \
SYNVEDA_CONFIRM_UPGRADE=upgrade:synveda-evaluation:before-0-4-7 \
  ./synveda-compose upgrade before-0-4-7
```

The command is in the source after v0.4.7 and stays a candidate until a
release includes it. It runs these phases, each recorded with a timestamp in
`state/upgrades/<backup-id>`:

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
2. From the v0.4.6 bundle: `./synveda-compose backup before-0-4-7`. The backup
   restarts the stack when it finishes. Any write after that point is not in
   the set.
3. From the v0.4.6 bundle: `./synveda-compose stop`, which stops every
   service.
4. From the v0.4.7 bundle, with the same `evaluation.json` values:
   `./synveda-compose up`. PostgreSQL starts first,
   and the v0.4.7 migrator runs `0011` and `0012` before the gateway and worker
   start.
5. Accept the target: sign in, check the expected workspace and run your
   public API checks.

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
./synveda-compose backup after-0-4-7            # from the target bundle, optional
SYNVEDA_CONFIRM_RESET=delete:synveda-evaluation:postgres-data \
  ./synveda-compose reset                       # from the target bundle
SYNVEDA_CONFIRM_RESTORE=restore:synveda-evaluation:before-0-4-7 \
  ./synveda-compose restore before-0-4-7        # from the installed bundle
```

Keep the installed bundle, its images and the `before-0-4-7` set until the
target release is accepted and a new recovery set of the upgraded installation
passes its own verification.

## Kubernetes

The chart keeps `Recreate`, one gateway and one combined worker. The
[operations guide](../deploy/helm/synveda/OPERATIONS.md#upgrade-and-failure-recovery)
has the v0.4.6 → v0.4.7 steps: scale the writers to zero, confirm no runtime
sessions, then `helm upgrade`. From the chart that carries the writer drain
onward, the install Job's migrator waits out the previous pods' termination
grace by itself. `helm rollback` never reverses committed SQL. No published
chart pair has been drilled on a cluster yet.

## External PostgreSQL

The DBA owns the server major, extensions, backups and restores. Before the
upgrade, confirm that the target admits the server major and extension
profile (see [PostgreSQL compatibility](DEPLOYMENT_CONTRACT.md#postgresql-compatibility)).
Take the provider's joint backup of the product database, identity state and
key material, and stop the gateway and worker. Then run the target's
`synveda db preflight`, `synveda db migrate --check` and `synveda db migrate`
as the migrator. Rollback after the schema change is the provider's restore of
that backup onto the previous release.

## Proposed support window and outage budget

No owner decision sets these yet. The proposal: support an upgrade from the
previous published release only, and plan a maintenance window of 15 minutes
for the bundled single-host route. In the drill's clean run, writers were down
for 377.9 seconds, from writer stop until the new gateway was ready, and 384
seconds passed until every service was healthy:

| Step | Seconds |
| --- | --- |
| Stop writers and Keycloak | 13 |
| Logical backup | 2 |
| Prepare the target | 1 |
| Recreate PostgreSQL, bootstrap, preflight, read-only check | 14 |
| `migrate` (`0011` 6.3 ms and `0012` 3.6 ms of DDL) | 1 |
| Start Keycloak, converge the realm, start the gateway and worker | 353 |

A restore of the pre-upgrade set took 382 seconds. Keycloak's restart
dominates both, and the dataset was small, so DDL time on a large installation
is not established. The owner still has to choose the window, the budget and
the minimum rehearsal dataset.

## Evidence

The [published-pair upgrade drill](../demos/evidence/ops6-published-upgrade-drill.json)
records artifact identities, phase timestamps, injected failures, digests and
measurements for the v0.4.6 → v0.4.7 run on one Windows/Docker Desktop
Linux VM. It is single-host evidence: a small product dataset, logical backup
on the same host, no off-host recovery set and no Kubernetes drill.

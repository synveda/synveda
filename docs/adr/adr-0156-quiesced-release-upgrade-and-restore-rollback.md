# ADR-0156: Quiesced release upgrades with a migrator writer drain and restore-based rollback

- **Status**: Accepted, **amended three times**: 2026-11-18 (Amendment 1),
  2026-11-25 (Amendment 2) and 2026-11-27 (Amendment 3)
- **Date**: 2026-10-23
- **Feature(s)**: Upgrade and rollback discipline, Docker-first portable reference deployment, Small-team Kubernetes release
- **Deciders**: Synveda maintainers

## Amendment 3 (2026-11-27): the launcher states only the direction

The published v0.4.11 → v0.4.12 drill
([record](../../demos/evidence/ops12-0412-upgrade-drill.json)) upgraded a pair
that runs no migration. v0.4.11's gateway and worker then accepted the
upgraded database, although the launcher's completion message said the
installed release cannot serve it, as its message after a failure in
`migrated` also claimed. A review then found that a preparation failure on a
resume from `migrating` or `migrated` said no schema change had run and
directed a restart of the installed release.

10. **Failure and completion messages claim only what the launcher
    observed, such as a passing read-only check, or what holds by design,
    such as every release refusing an open transition.** From `migrating`
    onward, each gives decision 5's direction, and none says that a migration
    ran or that the installed release cannot serve or refuses the database,
    because the launcher does not know whether a migration committed. A
    preparation failure on a resume runs no check, because the target is not
    yet prepared. This replaces decision 9's sentence on `migrated`.

## Amendment 2 (2026-11-25): a passing check means no migration is open

The published v0.4.10 → v0.4.11 drill
([record](../../demos/evidence/ops12-0411-upgrade-drill.json)) found two
messages that did not follow the database. The v0.4.11 read-only check passed
an open transition whose ledger was still at the stamped head, and refused one
in which `0020` had committed, with the same advice for both. After a refused
writer drain, the launcher said the schema change had begun, although nothing
had committed and the installed release's check still passed.

8. **The read-only check refuses every open transition.** The reference
   runtime's image switch starts the candidate's services on a pass, so a pass
   must mean a completely stamped head. An open transition is refused without
   writes, whatever its ledger, with the advice already given once a migration
   inside it has committed: finish it with this image's `migrate` after
   stopping every writer, or restore the pre-upgrade set. A stamped earlier
   head still passes as upgradeable.
9. **After a failure in `migrating`, the launcher runs that check.** A pass
   means no migration is open, at either the installed head or the target
   head, and the launcher directs a rerun without claiming that the schema
   change began. A refusal, or PostgreSQL failing to start, keeps the restore
   alternative and says only that a migration may be open. A failure in
   `migrated` still says the schema change began. The direction stays
   roll-forward. Whether the launcher may offer the installed release without
   a restore when nothing committed is not decided here.

## Amendment 1 (2026-11-18): an open transition, not a trailing stamp

The published v0.4.8 → v0.4.9 drill
([record](../../demos/evidence/ops12-049-upgrade-drill.json)) found what the
first consequence below predicted. With the `0019` ledger under a trailing
`0018` stamp, the published v0.4.8 gateway passed its authority proof and its
readiness probe. `0019` changes only CHECK constraints, which that proof does
not cover. Runtime roles deliberately cannot read the SQLx ledger, so the
marker is the only thing the installed release consults.

7. **The migrator opens a transition before changing an installed schema.**
   After the writer drain and the pre-migration authority proof, a run that
   would change an installed schema under a stamped head rewrites that head
   as `<stamped>..<target>`, for example `0019..0020`, in its own committed
   transaction. The same 10-second bound limits both its lock wait and the
   statement, and a timeout rolls it back. A lost commit acknowledgement is
   reported as unknown, and the rerun reads the marker either way. A
   stamp-only repair writes it too. The stamp replaces it with the target
   head at the end. A rerun with the same target rewrites the same value;
   a later image that recognises the ledger rewrites it toward its own head.
   - A run under an unstamped marker drains writers but writes no
     transition: it has no head to name, and every release already refuses
     an unstamped marker.
   - A fresh database with no tables writes nothing, and neither does an
     unchanged current head. So the idempotent migrate on every start never
     takes a serving installation out of service.
   - Every release from v0.3.0 through v0.4.9 accepts only a four-digit head
     (checked in source; earlier releases predate this epoch's marker). So
     each one's gateway, worker, migrator and read-only check refuse a
     transition before serving, with no reset advice. The published v0.4.8
     and v0.4.9 images were tried against a hand-set transition. The
     protection therefore covers the very next pair, whose installed release
     predates this decision.
   - This build reports an open transition as `InterruptedMigration`, with
     guidance to rerun the upgrade or restore the pre-upgrade set. Its
     migrator classifies the ledger from the stamped head, as before, so a
     resumed run finishes the remaining migrations or repairs only the stamp.
     It refuses a transition toward a head newer than its own, without
     writes, as the same interrupted migration, before it reads a ledger
     that may hold the newer migrator's rows. An image whose head is below
     the target cannot finish it; the image that opened it, or a later one
     that recognises the ledger, can. The read-only check names the target
     head rather than advising the operator to keep the running image.
   - A lock refusal now leaves the transition in place even when the failing
     migration rolled back whole. The installed release then refuses a
     database whose schema is unchanged. That is accepted: the phase's
     recovery direction was already roll-forward. It also makes the launcher's
     after-schema-change message true for that case. The message stays wrong
     only when a first attempt fails before the transition commits: a refused
     writer drain, or a transition write rejected before its commit.

Option 2 below is not reopened. It wrote the target head before the DDL that
reaches it, which the lineage proof correctly treats as drift. A transition
names the stamped head it starts from, and the lineage proof reads that head.

## Context
[ADR-0121](adr-0121-preserve-v0-4-3-through-forward-migration.md) made
published schema heads upgradeable after quiescence, and
[ADR-0152](adr-0152-postgresql-14-to-18-portable-lineage.md) added the portable
lineage and published convergence `0011`. Published v0.4.6 (head `0010`,
PostgreSQL 17 only) and v0.4.7 (head `0012`, PostgreSQL 14–18) are the first
genuine published pair: `0011` drops the two Knowledge views that v0.4.6 reads.

Three gaps remained. The deployment, not the migrator, was trusted to stop the
previous release's writers. Between a committed migration and its marker
stamp, a previous binary's epoch check still accepts the database. Helm's
install Job is not a hook and starts beside the terminating pods under
`Recreate`. Lock waits had no bound: `pg_advisory_lock` and the DDL's
`ACCESS EXCLUSIVE` request waited forever. The latter also queues every later
reader. Failures named no phase. The evaluation launcher had no upgrade
action. A new bundle's `up` ran the new migrator while the old gateway and
worker served, and the old bundle's `backup` restarted writers when it
finished.

## Decision
1. **The migrator drains previous writers.** When a run would change an
   installed schema, the migrator first counts the other sessions of the
   declared gateway and worker logins in its database, from
   `pg_stat_activity`. That means a stamped older head on either lineage, or
   any committed prefix whose stamp trails it. It waits up to a bounded writer
   drain, 120 seconds by default, under its advisory lock. If sessions remain,
   it refuses with no DDL, no stamp and no reset advice. A fresh database and
   an unchanged current head skip the drain, so the idempotent rerun on every
   start still succeeds beside live processes. `usename` and `datname` are
   visible to every role, so the ordinary migrator needs no extra privilege.
2. **Bounded, phase-named boundary.** The migrator sets a server-side
   `lock_timeout` of 60 seconds for the advisory boundary lock. Once it holds
   the boundary it lowers this to 10 seconds for each migration's lock waits.
   A wait past either limit fails with `55P03`. The migration's own transaction
   rolls back, and the same command retries. Errors name one closed phase:
   `session`, `boundary-lock`, `preflight`, `writer-drain`, `authority`,
   `schema-change` or `stamp`. They also carry the SQLSTATE and migration
   number, never bound values. `synveda db migrate` prints the starting and
   stamped heads with each wait and duration.
   `SYNVEDA_MIGRATION_WRITER_DRAIN_SECONDS` (1–600) sizes the drain for a
   deployment. SQLx's advisory lock stays the only concurrency mechanism.
3. **Helm sizing, not new ordering machinery.** The install Job keeps its
   order and its non-hook placement. Its migrate container receives a drain
   of the longer of the gateway and worker shutdown values plus 40 seconds.
   That covers `Recreate` termination grace. The container timeout grows by
   the drain. New pods refuse readiness until the stamp moves.
4. **One installed-host command.** `./synveda-compose upgrade <backup-id>`
   runs from the target bundle with `SYNVEDA_UPGRADE_FROM` naming the
   installed bundle and an exact confirmation. It proceeds in this order:
   1. Stop the previous gateway, worker and Keycloak with the installed graph.
   2. Make the logical recovery set with the installed release's own
      PostgreSQL image and recovery utility, without restarting writers.
   3. Prepare the target and start only PostgreSQL.
   4. Run the read-only `migration-check`, then one ordinary `migrate`.
   5. Start the target and wait for readiness.

   A private receipt records each phase and its time. Rerunning the same
   command resumes from the last completed phase. Before any schema change it
   refuses a resume if a gateway or worker is running again. Failures state
   the safe direction for the phase reached.
5. **Recovery direction is fixed by phase.** Before `migrating`, the database
   is still the installed release's: restart the installed bundle or resume.
   From `migrating` onward the only directions are roll-forward or a restore of
   the pre-upgrade set with the installed bundle into an empty installation.
   `helm rollback` and image rollback never undo committed SQL. The DBA's
   pgvector catalogue update stays after the rollback window
   ([ADR-0153](adr-0153-default-to-patched-pgvector-0-8-7.md)).
6. **Qualified pairs, not a promised window.** An upgrade is a planned
   maintenance interruption with no mixed-version serving. The
   [compatibility table](../UPGRADING.md#compatibility-table) lists each pair
   and the evidence tier it reached. The PostgreSQL major and the Keycloak
   version stay unchanged across the pair. Earlier pairs keep their refusals.
   The support window, outage budget and minimum rehearsal dataset are owner
   decisions. The guide records proposed values with the measurements behind
   them; no proposal is accepted here.

## Options considered
1. **Document writer shutdown only** — rejected: it leaves the stamp gap open
   and depends on every operator and every orchestrator honouring it.
2. **Stamp a "migrating" head before DDL so old binaries refuse** — rejected:
   it puts the marker ahead of committed DDL, which the lineage proof treats
   as drift.
3. **Revoke runtime logins or terminate their backends** — rejected: the
   ordinary migrator has neither CREATEROLE nor `pg_signal_backend`, and
   granting either widens the role contract.
4. **A Helm pre-upgrade hook or a Kubernetes API wait** — rejected: a
   pre-upgrade hook runs while the old pods still serve, and an API wait needs
   a ServiceAccount token the chart deliberately withholds.
5. **Zero-downtime expand/contract** — deferred: one gateway and `Recreate`
   remain the supported topology.

## Consequences
- Positive: a previous writer cannot serve against DDL it predates while the
  migrator is in charge, on Compose, Helm or a manual run. Lock waits end within
  stated bounds and name their cause. The installed-host upgrade is one
  resumable command, and its recovery set is restorable by the installed
  release.
- Observed for the first pair: with the `0012` ledger under a trailing `0010`
  stamp, the published v0.4.6 gateway passed its epoch check but was refused
  by its runtime role/catalogue proof. That proof caught this pair only
  because `0011` drops views and `0012` replaces routines. An additive change
  that leaves those fingerprints alone would pass it, so the drain stays the
  guarantee.
- Negative / accepted trade-offs: the drain is a check, not a lock. A previous
  process started after the drain passes, for example by an operator rollback
  during an interrupted stamp, is outside it. The procedure forbids that, and
  later stamp verification makes those processes exit. From Amendment 1, the
  open transition makes their next marker check refuse them. Until that check,
  up to 15 seconds, such a process can serve, and a schema change can begin in
  that interval. A blocked schema change
  fails after 10 seconds instead of waiting, so long readers such as a
  concurrent dump must be stopped first. Published v0.4.7 predates the drain,
  so the v0.4.6 → v0.4.7 upgrade relies on the command's explicit writer stop.
  Every outage is a maintenance interruption; there is no mixed-version
  serving.
- Reversal trigger: rolling multi-replica serving, or a migration that needs a
  resumable backfill longer than the planned interruption. Either requires an
  expand/contract design and a new decision.

## Compliance notes
Cedar, forced RLS, VedaFlow and audit paths are unchanged. The drain reads only
role names and a count from `pg_stat_activity`; no query text, state or
customer content is read or logged. Runtime roles gain no privilege, and no
migration file, checksum or baseline changes. Amendment 1 writes only the
marker row, which the migrator already stamps; it adds no grant, column or
migration.

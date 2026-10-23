# ADR-0156: Quiesced release upgrades with a migrator writer drain and restore-based rollback

- **Status**: Accepted
- **Date**: 2026-10-23
- **Feature(s)**: Upgrade and rollback discipline, Docker-first portable reference deployment, Small-team Kubernetes release
- **Deciders**: Synveda maintainers

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
  later stamp verification makes those processes exit. A blocked schema change
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
migration file, checksum or baseline changes.

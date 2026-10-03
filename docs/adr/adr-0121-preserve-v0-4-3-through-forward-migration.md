# ADR-0121: Preserve the v0.4.3 epoch-3 baseline through a forward migration

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-6
- **Deciders**: Synveda maintainers

## Context

Published v0.4.3 and the current source both stamp epoch 3, baseline revision
3, but embed different `0001_context_platform.sql` bytes. SQLx correctly
refuses to apply the source image to an installed v0.4.3 database. ADR-0120
froze the current source bytes while this choice was open. A first supported
small-team release needs a real data-preserving path from the most recent
evaluation release, without claiming that earlier epoch-3 checksums or pre-epoch
databases can be translated.

The source-only differences are additive CTX-6/CTX-8 schema: checkpoint event
types and provenance, a checkpoint lookup index, and an additional context
reason code. The release still has one gateway and worker, so the upgrade is a
planned interruption. OPS-5 must provide a verified joint database, issuer and
key recovery set before customer data crosses this boundary.

## Decision

Treat the exact published v0.4.3 `0001_context_platform.sql` bytes as the
immutable epoch-3 revision-3 baseline. Move the subsequent source-only schema
additions into transactional `0002_context_restart_and_excerpt.sql`. Pin both
migration files by digest, and add only append-only forward migrations after
them. Never rewrite a published SQLx ledger or silently accept another checksum
under the same migration number.

The candidate's read-only migration check distinguishes an exact v0.4.3 head
from its own exact current head. A v0.4.3 head is **upgradeable after the
deployment is quiesced**, not ready for the new binary to serve. Before DDL,
the migrator verifies the marker, exact SQLx ledger prefix, narrow deployment
role/extension authority and expected old schema shape; after DDL it verifies
the full current catalogue and stamps head `0002`. Fresh installation follows
the same two-file chain. A crash after any committed SQLx prefix may resume
only when that exact prefix and marker state are proved; a drifted or unknown
database remains refused without writes. Runtime readiness requires the exact
current head, not merely epoch and baseline revision. The migrator's read-only
check proves the full SQLx ledger under its separate credential before DDL;
ordinary runtime roles do not read that table on every readiness probe.

There is no mixed-version serving promise for this first pair. Stop all old
gateway and worker processes, make and independently verify the OPS-5 joint
recovery set, run read-only check and migration, then start the new image.
Rollback after `0002` restores the verified joint set; it does not run a down
migration or start the v0.4.3 binary against the advanced database. The
deployed v0.4.3 binary cannot be retroactively given the new head guard, so
quiescence and deployment sequencing are mandatory.

## Options considered

1. **Fresh production installation only** — avoids the migration design but
   leaves v0.4.3 evaluation data behind. Keep this as an explicit fallback if
   a rehearsal finds a data-preservation defect, not the default route.
2. **Retain the published v0.4.3 baseline and append `0002`** — chosen; SQLx
   can validate the released checksum and migrate data in place.
3. **Keep the source baseline and rewrite installed `_sqlx_migrations`** —
   rejected; it would falsely claim that published DDL ran and erase the
   reliable identity of the schema.
4. **Serve both versions during DDL** — rejected for this pair. The single
   gateway/worker deployment and old binary's head guard do not support that
   claim.

## Consequences

- The ADR-0120 source-only digest pin is superseded by a released-baseline pin
  and a separate forward-migration pin. Published history stays intact.
- A v0.4.0 epoch-3 database still has a different `0001` checksum and is not
  silently upgraded. It requires an explicit separately tested path or an
  owner-approved export/import into fresh storage.
- The migration may hold locks on populated context/session tables. The
  maintenance window and failure/restore time must be measured on
  production-shaped data before support is claimed.
- OPS-6 remains open until a published N-1/N pair passes data, key, audit,
  failure, rollback and compatibility drills from released artifacts.
- Revisit the default preservation path if an isolated v0.4.3 rehearsal cannot
  retain identities, immutable history, key access and the audit prefix.

## Compliance notes

`0002` changes only schema. Existing content remains under embedded Cedar,
forced RLS, VedaFlow and content-free audit; no migrator path serves requests
or becomes a tenant-data policy bypass. Migration and restore are explicit
deployment-administrator operations. Functional acceptance after migration
uses ordinary authenticated public APIs and ordinary tenant transactions.

# ADR-0112: Small-team operational evidence and recovery

- **Status**: Accepted
- **Date**: 2026-09-20
- **Feature(s)**: OPS-11; coordinates OPS-5, OPS-6 and OPS-8
- **Deciders**: Owner's operability and release-evidence request

## Context

The same application chart already passes the four database/identity ownership
combinations. Retained reinstall and a same-source Helm update do not prove
backup recovery, a prior-release upgrade or interruption of claimed work.
Published v0.2.0 predates the current deployment contract. A version label alone
cannot establish compatibility or artifact availability.

## Decision

Extend the existing disposable Kind fixture, ordinary public-API team workload
and release workflow. Use PostgreSQL 17 logical archives and existing provider
tools for a bounded writer-quiesced recovery ceremony. Stop gateway, worker,
packaged Keycloak and concurrent installation jobs before backing up both
databases; preserve database owners/ACLs, issuer/subjects, KMS material and
operator configuration. Restore only into a verified empty target, then run
normal preflight/migration and public sign-in, membership, content, retrieval
and audit checks. External administrators own their corresponding recovery set.
Backups leave the working PVC and require encrypted off-host custody before
being treated as recovery protection. A realm configuration export is not a
database backup. No backup service, operator or schema compatibility path is added.

Keep one gateway and worker. Exercise cancellation of a real claimed Capture
batch, its existing lease expiry and retry, and inspect completed candidates.
This proves a fenced database effect, never exactly-once provider execution.
Health continues to describe process and mandatory database authority; optional
provider/telemetry diagnostics do not become restart triggers.

Package a digest-bound Helm values overlay beside the existing chart and
reference artifacts, retain checksums, and reuse BuildKit provenance. Publication
remains tag-controlled. No artifact is called installable until its complete
registry set has been pulled and exercised. Keep prior-source upgrade evidence
distinct from a supported published N-1 release; refuse older schema epochs.

## Options considered

1. **Extend existing acceptance and native recovery tools** — selected; makes
   commands executable without another product control plane.
2. **Invent a backup operator, installer or migration framework** — adds
   ownership and failure modes outside the small-team scope.
3. **Call retained PVCs or a same-source rerun recovery/upgrade evidence** —
   rejected; neither exercises data reconstruction or release compatibility.

## Consequences

The live two-migrator drill found that SQLx released its DDL lock before the
epoch-stamp transaction. Concurrent stamps then failed serialization. Hold an
outer acquisition of that same reentrant SQLx PostgreSQL advisory lock across
preflight, DDL and stamping. Use a dedicated owned connection, closed on every
result and dropped on cancellation, so neither the outer lock nor SQLx's inner
error-path lock can leak back into the pool. The reset boundary consumes its
already-proved connection rather than reacquiring a potentially different target.
No new lock vocabulary, schema, retry engine or migration baseline is added.

- Planned downtime and an operator-owned recovery set remain required.
- Logical restore is a measured point-in-time full recovery, not WAL/PITR or
  an RPO/RTO promise. OPS-5 retains production retention and recurring drills.
- Application rollback is allowed only after compatibility checks; database
  changes require roll-forward or restore when the old binary is incompatible.
- Revisit this ceremony if measured data size makes its downtime unacceptable;
  qualify native provider backups rather than adding an application service.

## Compliance notes

Backup/restore are explicit database-administrator operations. All functional
acceptance still uses real OIDC, the public API, Cedar, forced RLS, VedaFlow and
content-free audit. Fixtures own only newly created clusters/namespaces. Reports
contain no credentials, payload text, database dumps or recovery keys.

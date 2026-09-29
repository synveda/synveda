# ADR-0120: Freeze the epoch-3 source baseline before upgrade work

- **Status**: Superseded by ADR-0121
- **Date**: 2026-09-29
- **Feature(s)**: OPS-6
- **Deciders**: Synveda maintainers

## Context

ADR-0069 deliberately permits pre-1.0 hard cuts within epoch 3. Published
v0.4.0 and v0.4.3 both stamp baseline revision 3, but their sole migration
has different SQLx checksums. The current source baseline differs again from
v0.4.3 after CTX-6/CTX-8 schema additions. SQLx correctly refuses these
pairs, so neither a same-source reinstall nor a shared epoch/revision number
is evidence of a data-preserving upgrade. OPS-6 needs a stable starting point
before it can implement and qualify a forward migration.

The baseline currently embeds every tenant table, forced-RLS policy, trigger
and authority catalogue. Rewriting it after a production installation would
make an installed database's SQLx ledger irreconcilable with the new binary.
The repository already checks that there is exactly one epoch-3 migration,
but that inventory check does not notice changed bytes.

## Decision

Pin the exact current `0001_context_platform.sql` bytes in the existing
`check-context-hard-cut` gate. It is the **source candidate** baseline at
commit `8630320`; the SHA-256 is
`07b4cee36190496d75d5633ca0f616ea99ebb317c2d159ec7e4d31bde4b048a5`.
The gate fails if the file changes, even when the migration count remains one.
Changing the digest is an explicit architecture/release review, not a routine
way to satisfy CI.

This pin does not promote the source candidate, establish a compatible
published release pair or authorize a reset of customer data. v0.4.x remains
self-hosted evaluation with its existing hard-cut refusal. OPS-6 must next
choose the durable release baseline and compatibility window, design
append-only migrations without editing the pinned baseline, and qualify an
actual published N-1/N pair with OPS-5 recovery. The single-baseline
assumption in the present epoch guard and hard-cut tests must be revised in
that future change, with a new accepted ADR before implementation.

## Options considered

1. **Pin the current source baseline** — selected to stop further accidental
   drift while the release and recovery contract is still open.
2. **Pin v0.4.3 immediately and turn current schema additions into `0002`** —
   would create a possible forward path, but requires a new migration/marker
   policy and database-backed upgrade acceptance; the current one-baseline
   guard rejects it. Do this as a reviewed OPS-6 implementation, not as a
   checksum-only file move.
3. **Continue editing `0001` without a byte guard** — rejected because
   another release could repeat the v0.4.0/v0.4.3 checksum mismatch while
   retaining the same epoch and revision labels.

## Consequences

- A baseline schema edit now fails the fast gate until its migration and
  compatibility decision is explicit.
- Existing evaluation databases with earlier checksums remain refused;
  this change neither rewrites their SQLx ledger nor supplies a translator.
- The pin alone is insufficient for the production readiness verdict. A
  published upgrade pair, failure recovery and owner-approved window remain
  OPS-6 work.
- Revisit the source candidate if a pre-production defect demands a new
  baseline; record its compatibility consequence and requalify the release.

## Compliance notes

The check is repository-only and touches no tenant data. Future migrations
must retain the embedded Cedar, forced-RLS, governed VedaFlow and content-free
audit boundaries; a checksum match is not proof of those behaviours.

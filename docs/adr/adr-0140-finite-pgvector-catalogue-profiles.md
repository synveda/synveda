# ADR-0140: Admit finite pgvector catalogue profiles for external databases

- **Status**: Accepted
- **Date**: 2026-10-10
- **Feature(s)**: OPS-11, CPR-45
- **Deciders**: Synveda maintainers

## Context

Issue #72 identifies Azure Flexible Server's advertised pgvector 0.8.2 as
incompatible with the reference 0.8.6 pin. The owner requests source support
for 0.8.2 and subsequently 0.8.3, 0.8.4 and 0.8.5 while keeping live Azure
qualification outside this implementation.
Version strings alone cannot establish extension authority: epoch 3 also
requires bounded member, executable, operator and support-function proof.
This extends [ADR-0102's external deployment contract](adr-0102-portable-reference-deployment.md)
without changing [ADR-0121's published migration baseline](adr-0121-preserve-v0-4-3-through-forward-migration.md).

The upstream 0.8.2-to-0.8.6 SQL comparison contains only empty version-update
scripts. A genuine digest-pinned 0.8.2 installation has the same 237 members,
36 access operators and 54 support functions as the existing contract.
Its metadata includes a different version, so its fingerprint must remain
separate. The fingerprint does not attest native binaries or vendor backports.

The reviewed upstream multi-architecture input is
`pgvector/pgvector:0.8.2-pg17-bookworm@sha256:feb68f4f15446397d8cac7f4fe48fe4586de83160d1fc48b46283312d1a33966`.
The native ARM64 installation produces vector's catalogue digest
`327a24aaa912c72a161947332ad96f30431eba9e9a6e300419bda3c3a743812c`.
An independent read-only inventory comparison changes only its version-bearing
metadata to obtain the existing 0.8.6 digest; no installed catalogue/version
is rewritten. The upstream `v0.8.2...v0.8.6` comparison likewise adds only empty
SQL update scripts. [The image inventory](../../deploy/helm/IMAGES.md) retains
the input's PostgreSQL licence and its fixture-only purpose.

Genuine digest-pinned 0.8.3, 0.8.4 and 0.8.5 installations independently produce
the same member/operator/support counts. Their installation SQL has identical
SHA-256 `7fb5bb279ef83bf9204bfac7405bb5c9a05e49f5ac7d64d1eb4464d103b80f32`;
the intervening upstream update scripts contain no DDL. Read-only comparison
of each inventory changes only the version-bearing metadata to reproduce the
existing 0.8.6 digest; no installed catalogue or version is rewritten.

| Version | Upstream `pg17-bookworm` image index SHA-256 | Catalogue SHA-256 |
| --- | --- | --- |
| 0.8.3 | `be400b50812ab2cc908ed78593fda2e51e3b45fe774fa637f1c7b16e68531d95` | `ac60015ce1363b2e7a8e3361cad9dec990b2d6ecc6dbe1960a7b01a283a7af14` |
| 0.8.4 | `dd467f03ca5c5581222490e5217e48a262864ccb659be559f8491bbafdc97da0` | `315535f951b8aea94d1dc841c5f800d95458feef94b9071962bdd1ae27e0a83c` |
| 0.8.5 | `d2ef61f42ef767baa5a1475393303cc235bcd92febd9d7014eddb48b41f3bad0` | `b49e95e1bab2a81603ba73722a0ebd2daed082208a27fdd2d64170df51b99a6f` |

## Decision

Admit exactly pgvector 0.8.2, 0.8.3, 0.8.4, 0.8.5 and 0.8.6, each under its
own reviewed PostgreSQL 17 catalogue fingerprint. Preserve btree_gin 1.3,
plpgsql 1.0, expected schemas, trusted owners, role isolation, Cedar, forced
RLS and audit. No version range,
suffix, provider exception or unchecked profile is admitted.

Bundled artifacts and fresh administrative creation remain pinned to 0.8.6.
An external DBA may preinstall any admitted version; migration and retained
bootstrap verify it without updating or downgrading it. External destructive
reset remains outside the supported external-service lifecycle. Published
migrations and application catalogue fingerprints remain immutable.

Use digest-pinned genuine upstream 0.8.2, 0.8.3, 0.8.4 and 0.8.5 binaries in
the disposable database harness, preserving the reference PostgreSQL 17.11
base and normal role, migration and tenant-transaction paths. The fixture is
never a release image.
Test fresh/repeated migration, retained current schema, current/historical
Knowledge, cosine retrieval at both dimensions and real catalogue drift.

## Options considered

1. **Finite reviewed profiles** — chosen; existing services can pass the same
   mandatory authority proof without floating bundled dependencies.
2. **A minimum such as >= 0.7.0** — rejected; it admits unreviewed catalogues
   and native versions, and cannot satisfy the exact fingerprint contract.
3. **Change the default package or downgrade an existing server** — rejected;
   neither is required for existing-database compatibility.
4. **Retain the single-version refusal** — preserves the current limitation
   but does not meet the owner's compatibility request.

## Consequences

- Positive: genuine installations of all five versions can satisfy runtime
  admission, while catalogue and executable drift still fail closed before
  application DDL.
- Accepted trade-off: this is source compatibility for controlled evaluation,
  not production or managed-provider certification. Upstream 0.8.3/0.8.4 fix
  HNSW vacuum corruption and concurrent-insert errors present in 0.8.2.
  All admitted versions are affected by the IVFFlat index-build vulnerability
  fixed in 0.8.7. Passing regressions cannot prove those known defects absent.
  Synveda creates HNSW indexes and grants runtime roles no DDL; that narrows
  exposure but does not establish a patched native dependency.
- Native package/backport provenance, patched-release admission and retained
  extension upgrade/recovery qualification remain separate work. No automatic
  extension update, downgrade or mixed-binary serving promise is added.
- Reversal trigger: a provider supplies a patched native release/backport with
  verified provenance, or an observed 0.8.2 defect invalidates the controlled
  evaluation envelope. Review the finite set and fixtures before changing it.

## Compliance notes

All product reads and writes retain embedded Cedar and ordinary forced-RLS
tenant transactions; governed mutations retain VedaFlow and content-free
audit. Fixture administrators only provision or deliberately tamper with their
own disposable databases. Supporting 0.8.2 removes one Azure version blocker;
provider-owned objects, roles, cluster identity, TLS and recovery still need
independent qualification before an Azure support claim.

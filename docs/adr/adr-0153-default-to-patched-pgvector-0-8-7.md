# ADR-0153: Default to patched pgvector 0.8.7 and keep earlier profiles as a dated legacy exception

- **Status**: Accepted
- **Date**: 2026-10-21
- **Feature(s)**: Small-team Kubernetes release, Upgrade and rollback discipline, Consumer installation and harness setup, Docker-first portable reference deployment
- **Deciders**: Synveda maintainers

## Context

[Upstream issue #1036](https://github.com/pgvector/pgvector/issues/1036)
(CVE-2026-103484) reports that a database user able to create an IVFFlat
index can write out of bounds in pgvector 0.8.6 and earlier, which can lead to
arbitrary code execution. 0.8.7 (tag `v0.8.7`, commit
`f37c13f68b57d2c3472b2214fbcff699d6d34876`, 2026-10-01) fixes it in native C
code (`ivfbuild.c`, `ivfutils.c` and hardened index entry points). The same
release also fixes a duplicate-neighbour race between HNSW insert and vacuum.
Synveda's runtime roles have no DDL and the product creates only HNSW indexes.
That does not make an affected native library safe for other principals on
the same server.
[ADR-0140](adr-0140-finite-pgvector-catalogue-profiles.md) names a patched
native release as its reversal trigger. Every bundled image and per-major
fixture still installs 0.8.6.

Measured inputs (2026-10-21):

- The PGDG source package `pgvector_0.8.7-1.pgdg12+1` unpacks to the same tree
  as the GitHub `v0.8.7` tag. Its only Debian patch removes `-march=native`.
  Signed PGDG Bookworm binaries `0.8.7-1.pgdg12+1` exist for PostgreSQL 14–18
  on AMD64 and ARM64.
- The CloudNativePG `17.11-202610190957-standard-bookworm` index
  (`sha256:0d0e407ca1d986022651671e830022ccfe496994a8b99fe925b5ae3f0264cb1a`)
  carries `postgresql-17-pgvector 0.8.7-1.pgdg12+1`. `dpkg --verify` passes, and
  its `vector.so` is byte-identical to the signed PGDG `.deb` on both
  architectures (AMD64 `bf8d50bf…`, ARM64 `73e4ea8c…`).
- The 0.8.7 installation SQL adds 147 `COMMENT ON` statements to the 0.8.6
  script (149 against 2) and changes one SQL comment line. The
  `0.8.6--0.8.7` update script contains exactly those 147 statements. The
  extension fingerprint reads no `pg_description`.
  Clean installations measured with the unchanged fingerprint SQL therefore
  differ from 0.8.6 only by version metadata. The fingerprint is
  `22db8fc904ddcfa595031167851aee528cd34bd0d62fd09855b2a68f506a2de3` (234
  members) on 14–16 and `7ffe1da9f74811daba20d0923a6233b23f04badee88627630fd21bd4e704f9e0`
  (237 members) on 17–18, with 36 access operators and 54 support functions.
  The same values result after `ALTER EXTENSION … UPDATE` from a retained 0.8.6
  catalogue, on AMD64 and on emulated ARM64. The same method first reproduced
  every pinned 0.8.6, btree_gin and plpgsql digest.
- The 0.8.7 package ships only the `vector--0.8.7.sql` install script. A
  0.8.6 catalogue cannot be created on a 0.8.7 server; it can only be retained.

## Decision

1. **Patched default.** Bundled Compose, the PITR variant, bundled Helm, the
   CNPG derivative, `synveda db setup-sql`, the image's `database-setup-sql`
   role and local reset create or verify pgvector **0.8.7**. Every per-major
   fixture installs the genuine PGDG 0.8.7 package. The CNPG base moves to the
   measured image above. Package and image pins stay readable and
   digest-bound.
2. **Two new measured profiles.** Add `vector` 0.8.7 on 14–16 and on 17–18
   with the digests above. Every earlier row, bound, owner/role/schema proof and
   byte-pinned migration is unchanged. No schema migration is needed: the
   application catalogue, ACL, routine, trigger and forced-RLS fingerprints are
   independent of the extension version.
3. **Legacy exception.** Keep the 0.8.2–0.8.5 (17) and 0.8.6 (14–18) profiles
   admitted as a legacy compatibility exception:
   - *Scope:* preinstalled external databases and retained installations in
     transition, with the native package replaced. Bundled artifacts and fresh
     creation never select them.
   - *Production support:* requires native pgvector 0.8.7 or a vendor backport
     with verifiable provenance. Any installation whose native library is ≤ 0.8.6
     without such a backport is affected and limited to controlled evaluation.
   - *Expiry:* 2027-01-31. The first release after that date removes the legacy
     rows unless the maintainers record a renewal with a new date here.
   - *Remediation owner:* the deployment owner's database administrator
     replaces the native package and runs the catalogue update. The Synveda
     maintainers own the profile set and its removal. A named security
     owner and severity SLA are still open in production readiness.
4. **Separate native and catalogue transitions.** Package replacement is the
   security fix and needs a PostgreSQL restart, so that no backend keeps the old
   library mapped. `ALTER EXTENSION vector UPDATE TO '0.8.7'` adds only
   comments and moves the catalogue onto the patched profile before the
   exception expires. It is a deliberate DBA step. Ordinary startup, bootstrap,
   setup SQL and migration never update or downgrade an extension.
5. **Refusals and warnings say what to do.** An unmeasured version or
   fingerprint names the recommended 0.8.7 profile and the owner's
   `ALTER EXTENSION` step. `synveda db migrate`, including `--check`, also
   prints a warning that never blocks. It fires while the installed package's
   control-file default is older than 0.8.7 or the catalogue still uses a
   legacy profile. That reading comes from the package's files, not the loaded
   library. Neither message carries a connection detail or secret, and neither
   runs on a request path.

The retained-data ceremony is in the
[deployment contract](../DEPLOYMENT_CONTRACT.md#retained-pgvector-transition).

## Options considered

1. **Patched default with finite legacy rows and a dated exception** — chosen.
   New installations get the fix. Retained and provider databases keep one
   measured path forward without a silent support claim.
2. **Remove 0.8.2–0.8.6 immediately** — rejected. It breaks the owner-requested
   Azure 0.8.2 compatibility. It also refuses retained installations whose
   native library is already patched but whose catalogue update is pending.
3. **Update the extension automatically at startup** — rejected. It gives
   ordinary startup DBA authority, and the catalogue update is not the
   security fix.
4. **Add `pg_description` to the fingerprint** — rejected. It would change
   every immutable historical digest, and comments carry no executable
   meaning, so authority gains nothing.

## Consequences

- Positive: every artifact Synveda builds or selects now carries the patched
  native library. A measured, transactional catalogue step moves a retained
  database onto the patched profile without schema change.
- Accepted limit: the fingerprint still attests catalogue definitions and the
  `$libdir/vector` reference, not the loaded binary. A 0.8.6 catalogue on 0.8.7
  native code and on 0.8.6 native code look the same. Patch status is a
  package fact for the DBA or provider to verify.
- Accepted limit: after the catalogue update, a release that lacks the 0.8.7
  rows (every published release up to v0.4.6 and the v0.4.7 candidate) refuses
  readiness. Rollback past that step means restoring the pre-transition
  recovery set onto the previous artifacts. An old binary cannot simply be
  restarted.
- Reversal trigger: a newer upstream or vendor fix, a changed 0.8.7 catalogue,
  or the exception expiry. Measure clean installations and update the finite
  rows. Never copy a live customer fingerprint.

## Compliance notes

Cedar, forced RLS, VedaFlow, content-free audit, exact role admission,
ownership separation and drift refusal are unchanged. No runtime role gains
DDL, superuser, CREATEROLE or BYPASSRLS. Fixture administrators provision,
update and deliberately tamper only with their own disposable clusters. Every
product acceptance case uses the ordinary runtime roles.

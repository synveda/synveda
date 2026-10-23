# ADR-0154: Joint PITR recovery with sealed custody and KEK replacement

- **Status**: Accepted
- **Date**: 2026-10-22
- **Feature(s)**: OPS-5, CPR-45
- **Deciders**: Synveda maintainers

## Context

The first supported self-installed deployment is single-host Compose with the
bundled PostgreSQL 17 cluster. That one cluster holds both the `synveda` and
`keycloak` databases, and a file-backed local KEK wraps every deployment and
tenant data key. [ADR-0123](adr-0123-opt-in-compose-physical-backup.md)
added an opt-in pgBackRest image that archives encrypted WAL and full base
backups to an operator-owned S3-compatible bucket over verified TLS. It had no
restore command, no record of which keys and credentials a backup needs, and
no way to replace the KEK. The paired logical tools in
[ADR-0112](adr-0112-small-team-operational-evidence.md) remain unencrypted,
same-host and writer-quiesced, and two dumps are not one transaction.

A local pgBackRest 2.59.1 rehearsal (2026-10-22) found three behaviours that
any restore must work around:

- `verify` does not report a WAL segment missing from the middle of the
  archive, and it exits 0 even when it reports a corrupt object.
- A restore to the end of the archive across such a gap promotes silently
  at the gap and loses every later commit.
- A restore to a named target beyond the gap fills the target volume and only
  then fails with "recovery ended before configured recovery target was
  reached".

A physical copy also brings the source's `pg_authid` with it, so the restored
cluster accepts only the source's database role credentials.

## Decision

1. **Recovery mode.** For bundled Compose PostgreSQL, the supported recovery
   mode is pgBackRest base backups plus continuous WAL in one encrypted S3
   repository. One WAL stream covers both databases, so one recovery target
   gives a consistent product and identity point without pausing writers.
   Paired logical recovery stays as a planned-interruption fallback and is
   never called PITR. Use CNPG/Barman ([ADR-0122](adr-0122-provider-owned-object-store-backup-seam.md))
   only for an actual CNPG deployment. Customer-operated PostgreSQL keeps its
   own recovery service.
2. **Custody bundle.** Each `pitr-backup` generation produces
   `custody.svrc`. It holds the KEK and its reference, the Keycloak
   convergence credential, the source cluster's role credentials and the
   repository cipher passphrase. It is sealed with the existing XChaCha20-Poly1305
   envelope under a separate 256-bit custodian key and the new
   `recovery.custody` purpose. The clear header contains identifiers only:
   source/backup identity, product and PostgreSQL image references, the
   repository locator, backup label, LSN/WAL range, system identifier, server
   and pgvector versions, migration head, ledger digest, KEK references and
   custodian label. The header's SHA-256 is the envelope row key, so a
   replaced header or a body moved under another header does not open. The
   bundle never goes to the bucket. The custodian moves it off-host by their
   own route. Bucket credentials are not escrowed, because the owner can
   reissue them.
3. **Restore ceremony.** `pitr-restore` targets only a fresh, confirmed
   project. Before it changes any target state it opens the custody bundle and
   requires the selected product and PostgreSQL images to match the header.
   It reads `info`, `verify` and the WAL listing and refuses any non-zero
   verify counter. It proves every segment from the base backup's start to
   the target exists on one timeline. Supported targets are `latest` and a
   restore mark recorded by `pitr-mark`. The restore then installs the escrowed
   credentials into the target, restores with `archive-mode=off`, starts
   PostgreSQL with archiving off and waits for promotion. After that it reruns
   authority convergence, runs the read-only `migration-check`, the restored
   tenant/audit/key verifier and wrong-key refusal, and finishes with browser
   acceptance. The target records a restore marker. Every later PITR action on
   it is refused until its repository configuration names a different path.
   A restored cluster keeps the source system identifier, so without this it
   could archive into the source stanza.
4. **Retention.** Every base backup still runs with `--no-expire-auto`.
   Retention is an explicit operator expire, run only after an independent
   restore has verified a newer generation. No command deletes the last
   verified generation.
5. **KEK replacement.** `synveda kms rewrap` reads the retiring KEK from
   `SYNVEDA_KMS_PREVIOUS_KEY[_FILE]`/`…_REF` and the replacement from the
   ordinary KMS settings. It rewraps every deployment and tenant generation,
   retired ones included, by compare-and-set from the previous reference. It
   refuses a row on any third reference, resumes after interruption, appends a
   content-free `tenant.key.rewrapped` event per changed tenant, and finishes
   only after every generation opens under the replacement. Product writers
   are stopped while it runs. Generations taken before the replacement keep
   their own custody bundle with the previous KEK. `recovery-verify` now
   requires every generation to name and open under the supplied KEK, so a
   stale or incomplete custody set is refused.

## Options considered

1. **pgBackRest PITR with a sealed custody bundle** — selected; one tool owns
   WAL, encryption and object-store protocol, and one Synveda envelope owns key
   custody.
2. **Encrypt and copy the paired logical archives off-host** — still not
   PITR, still two independent dumps, and still a writer outage per backup.
3. **Escrow the custody files unencrypted beside the bucket** — keys and
   database would then fail together. A SHA-256 list detects accidents but not
   replacement.
4. **A new KMS/HSM framework** — no customer requirement names a provider. The
   existing `Kms` boundary stays the seam.

## Consequences

- Positive: one consistent product and identity recovery point. A missing
  segment, corrupt object, wrong passphrase, untrusted CA, revoked credential,
  mismatched image or stale KEK is refused before the target changes. Reports
  and logs carry identifiers and timings, never credentials or content.
- Negative / accepted: RPO is bounded by WAL archive latency, with the
  60-second `archive_timeout` as a ceiling rather than an objective. RTO grows
  with database size and WAL replay. Physical restore preserves the source's
  database role credentials, so their rotation is a separate act after
  recovery. Only 16 MiB WAL segments and one timeline per generation are
  supported. Time and LSN targets are not offered.
- Reversal trigger: a live S3-compatible provider that fails integrity, TLS or
  restore objectives, or measured RTO above the owner's objective, reopens the
  tool choice. A customer that selects a KMS/HSM provider gets that provider
  behind `Kms`, with this ceremony retained for local custody.

## Compliance notes

Backup, custody, restore and rewrap are deployment-administrator acts. They
never serve application requests. A restored deployment is admitted only
through ordinary OIDC, Cedar, forced RLS, VedaFlow and content-free audit, and
the operator reconciles current external disables and policy before opening
traffic. A historical restore does not prove present-day authorization.
Rewrap audit events carry KEK references and counts only.

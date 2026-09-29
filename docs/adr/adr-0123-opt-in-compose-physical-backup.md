# ADR-0123: Use pgBackRest for opt-in Compose physical recovery

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-5
- **Deciders**: Synveda maintainers

## Context

The first proposed self-hosted offer is single-host Compose. Its paired,
writer-paused logical archives verify one fresh restore but have neither
continuous WAL nor point-in-time recovery. The optional CNPG chart already
binds an operator-owned Barman Cloud ObjectStore under ADR-0122; Compose has no
CNPG operator and must not import that Kubernetes control plane. The selected
first destination is S3-compatible object storage, including AWS S3, with a
route to Azure Blob and GCS. Backups need encryption before they leave the host
and a separately recoverable Synveda/Keycloak/KMS/issuer set.

[pgBackRest](https://pgbackrest.org/user-guide.html) provides PostgreSQL base
backups, WAL archiving and PITR with S3, Azure and GCS repositories. Its
[repository cipher](https://pgbackrest.org/configuration.html#section-repository/option-repo-cipher-type)
encrypts client-side and its source is [MIT licensed](https://github.com/pgbackrest/pgbackrest).
It runs as deployment tooling in the PostgreSQL image; no Rust crate or public
application API needs to know the backup repository.

## Decision

Add a separate, opt-in pgBackRest-capable Compose PostgreSQL image and overlay.
An operator-owned private configuration file selects one S3 repository,
endpoint, credentials and client-side cipher passphrase. The Compose lifecycle
validates the selected mode and file before mutation, mounts it only in the
PostgreSQL service, and enables WAL archiving there. The opt-in service joins
the existing application-egress network to reach its object store; the default
PostgreSQL service remains internal-only. A bounded operator command
creates/checks the stanza and takes a physical base backup without automatic
expiry. The application
retains its existing single-instance, planned-interruption contract; backup
availability is not high availability.

Do not put repository credentials or cipher passphrase in Compose environment,
Git, Helm values, command arguments or logs. The passphrase and the matching
KMS/issuer recovery set are escrowed separately from the database repository.
No default destination, retention, RPO/RTO or deletion policy is invented.
Restore is always into new storage and a distinct project; an ordinary public
API, RLS, Knowledge and audit verifier decides whether the recovered generation
is usable. A repository listing or successful backup command alone is not an
OPS-5 acceptance result.

The initial source slice supports S3-compatible repositories with verified
TLS. A private object-store CA is an optional second project-private file,
mounted only into PostgreSQL and staged beside the backup configuration in
runtime tmpfs. Its use is explicit in the validated repository configuration;
TLS verification cannot be disabled. Azure/GCS can be added by extending the
deployment validator after their exact configuration and recovery tests,
without changing application code. The existing logical backup remains the
default reference until physical restore and joint custody pass a live drill.

## Options considered

1. **pgBackRest in an opt-in PostgreSQL image** — selected for physical/WAL
   recovery, client-side encryption, S3-compatible/AWS support, later Azure/GCS
   providers and MIT licence.
2. **WAL-G** — also supports several providers, but its available encryption
   modes and additional binary packaging would require a separate custody
   decision. pgBackRest gives the first offer one repository cipher contract.
3. **Logical dump plus S3 copy** — rejected as the only production mechanism:
   a sequence of dumps is not WAL/PITR and is not atomic across independent
   database owners.
4. **Run CNPG/Barman inside Compose** — rejected because it imports an
   unnecessary Kubernetes operator into the portable single-host reference.

## Consequences

- The binary, configuration file and archive command must be pinned and tested
  with the exact PostgreSQL image. WAL must be monitored: repository failure
  can retain WAL locally until storage fills.
- The configuration file lives beside the project's managed secret directory
  because that directory rejects unknown entries. A root-owned entrypoint
  stages a postgres-readable copy in the existing runtime tmpfs before the
  stock PostgreSQL entrypoint drops privileges.
- The optional private CA file follows the same ownership and tmpfs staging
  rule. It is never mounted into application or bootstrap services.
- The operator must schedule base backups, monitor archive age/capacity and
  retain an independently restored generation before enabling expiry.
- A new project cannot be brought up against recovered data until the
  PostgreSQL, Keycloak/issuer and KMS materials match and the ordinary
  authority/public-function checks pass. No in-place recovery command is added.
- If a live S3-compatible provider fails integrity, TLS or restore objectives,
  revisit this tool selection before advertising Compose recovery support.

## Compliance notes

The backup process reads PostgreSQL physical files with deployment authority.
It does not make application authorization decisions or bypass Cedar, forced
RLS, VedaFlow or content-free audit when serving restored traffic. Reports
contain object identities and timings only, never repository credentials or
product content.

# ADR-0122: Use a provider-owned object store for CloudNativePG backups

- **Status**: Accepted
- **Date**: 2026-09-29
- **Feature(s)**: OPS-5
- **Deciders**: Synveda maintainers

## Context

The optional CloudNativePG deployment has no WAL archive or scheduled physical
backup. Its quiesced logical restore proves a narrower recovery path, and the
Compose reference has the same production gap. OPS-5 needs off-host base
backups and WAL, isolated PITR, and joint database/identity/key verification.
The first destination is an operator-owned S3-compatible object store, including
AWS S3. The backup transport should admit later Azure Blob or GCS selection
without changing Synveda's database or application authority.

CloudNativePG's current Barman Cloud integration is a separate plugin. An
`ObjectStore` resource owns provider location, credentials, encryption and
retention; the `Cluster` references it for WAL and `ScheduledBackup` references
the plugin for base backups. This avoids maintaining a second backup engine in
the application and preserves existing deployment ownership boundaries.

## Decision

Add an opt-in Helm `postgres.backup` contract for `postgres.mode=cnpg` only.
The operator provisions a same-namespace Barman Cloud `ObjectStore`, bucket,
credentials, TLS trust, encryption and key custody outside this chart. The chart
accepts its exact name and an explicit six-field backup schedule, checks the
plugin API at render time, attaches it as the cluster WAL archiver, and creates
a `ScheduledBackup` using `method: plugin`. The schedule owns no backup CRs, so
removing the chart's schedule cannot cascade-delete retained recovery evidence.
No default backup destination, schedule, retention or RPO/RTO is invented.

The first documented ObjectStore shape uses a TLS S3-compatible endpoint,
Secret references and explicit encryption for both base data and WAL. AWS S3
uses the same provider contract, normally with its native endpoint and an
operator-owned IAM or Secret credential. Provider-specific ObjectStore fields
remain outside Synveda's values; a future Azure/GCS configuration changes that
resource, not the chart seam. The chart does not render credentials or claim to
verify the provider's encryption, reachability, lifecycle or custody.

This increment is configuration plumbing, not OPS-5 acceptance. A backup only
counts after the source and object-store status are inspected, a preselected
point is restored into a new cluster, the Keycloak/issuer and KMS recovery set
is paired with it, and public-API/audit/RLS/Knowledge checks pass. Compose and
external PostgreSQL still need their own qualified backup mechanism.

## Options considered

1. **Use the Barman Cloud plugin and external ObjectStore** — selected for its
   continuous physical backup/WAL and S3/Azure/GCS provider interface.
2. **Use the deprecated in-tree `barmanObjectStore` field** — rejected because
   the current operator documents the plugin as its forward path.
3. **Put S3 credentials and endpoint directly in Synveda values** — rejected;
   it couples the chart to one provider and risks placing credentials in values.
4. **Treat quiesced logical dumps or retained PVCs as PITR** — rejected; neither
   provides continuous WAL restore to a selected instant.

## Consequences

- A chart render can refuse an absent plugin API or incomplete opt-in values.
  Live installation still requires an already installed compatible operator,
  plugin, ObjectStore and secret; render checks do not prove those resources.
- The operator must keep a verified backup generation and separately protected
  identity/key material before setting retention or retiring a cluster.
- Rolling back this chart change removes new scheduling and WAL attachment but
  must not delete retained object-store data or prior backup resources.
- Revisit the provider seam if live restore shows Barman Cloud cannot meet the
  selected S3-compatible store's integrity, encryption or recovery objectives.

## Compliance notes

Backup software reads a physical database copy under deployment-administrator
authority. It does not serve application requests or create a Cedar/RLS bypass.
Restored acceptance uses ordinary OIDC/public API, Cedar, forced RLS, VedaFlow
and the content-free audit chain. ObjectStore manifests and reports contain no
plaintext credentials, session payloads or Knowledge bodies.

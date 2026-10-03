# CloudNativePG object-store backup candidate

This is the OPS-5 **source candidate** for the optional CNPG deployment. It
renders a continuous WAL archive and a scheduled physical base backup through
the [Barman Cloud plugin](https://cloudnative-pg.io/plugin-barman-cloud/docs/usage/).
No published Synveda release includes this chart change, and no live PITR or
production RPO/RTO has been qualified. The existing [quiesced logical recovery
runbook](OPERATIONS.md#bounded-write-quiescing-backup) remains separate.

The first target is an operator-owned S3-compatible service, including AWS S3.
The plugin's provider interface also supports Azure Blob and GCS; changing
provider changes its `ObjectStore`, not the Synveda chart fields. This chart
does not install the CNPG operator, plugin, object store, bucket or credentials.
The [example ObjectStore](examples/s3-backup-objectstore.yaml) is a template,
not a ready-to-apply backup policy.

## Prepare before enabling

1. The database owner installs a compatible CNPG operator and Barman Cloud
   plugin with their CRDs. The [plugin installation guide](https://cloudnative-pg.io/plugin-barman-cloud/docs/installation/)
   requires the plugin beside the operator and documents its TLS prerequisite.
   The chart's optional CNPG fixture uses operator 1.30.0; the plugin version
   and combined deployment still need live qualification.
2. Select a bucket/prefix outside the source cluster's failure domain. Record
   the provider, region, TLS CA, S3 access method, encryption/key custody,
   retention, cost owner and alert/drill owner in the installation's private
   operations record. Pre-create and restrict the bucket and Secret or workload
   identity. The example requires HTTPS and requests SSE `AES256` for **both**
   base data and WAL. For AWS S3, omit `endpointURL` and use the provider's
   native endpoint; `aws:kms` can be selected in the ObjectStore when its key
   custody and restore permissions are arranged. Prove encryption at the
   provider, rather than inferring it from Helm output.
3. Provision the `ObjectStore` in the Synveda namespace and verify its Secret,
   endpoint trust and access before enabling the chart setting. Keep its
   `retentionPolicy` unset until at least one independent PITR is verified.
   [Barman retention](https://cloudnative-pg.io/plugin-barman-cloud/docs/retention/)
   can delete obsolete generations after a later successful backup; the
   deletion policy needs its own owner review.
4. Escrow the **same generation** of Synveda/Keycloak databases, KMS KEK and
   key reference, issuer/client configuration and signing-key history, TLS/CA,
   DB role credentials and exact image/chart digests. A physical PostgreSQL
   backup covers both databases only when they share that CNPG cluster. An
   external IdP, KMS or PostgreSQL owner supplies their matching recovery set.
   Backup of a database without its identity/key material is not recoverable.

Enable only after these resources exist, with an operator-selected six-field
CNPG schedule (seconds first):

```yaml
postgres:
  mode: cnpg
  backup:
    enabled: true
    objectStoreName: synveda-backups
    schedule: "0 0 2 * * *" # example only; owner chooses actual schedule
```

`helm template` refuses a missing plugin API or incomplete opt-in values.
Use `--api-versions postgresql.cnpg.io/v1 --api-versions barmancloud.cnpg.io/v1`
only for an **offline render**; this flag does not install either CRD. The
rendered `ScheduledBackup` has `backupOwnerReference: none`, so removal of the
schedule does not cascade-delete its Backup resources. Object-store retention
and bucket lifecycle are separate and may still delete data.

## Qualification and recovery boundary

Inspect `kubectl -n synveda get cluster,objectstore,scheduledbackup,backup` and
the relevant operator/plugin conditions. Confirm a completed base backup and
WAL objects in the remote bucket. A green schedule alone is insufficient.
Before claiming coverage, choose two committed write points around an audit
event and Knowledge revision, then recover each target into a **new empty
cluster** through CNPG's
[plugin recovery source](https://cloudnative-pg.io/plugin-barman-cloud/docs/usage/#restoring-a-cluster)
and [`recoveryTarget.targetTime`](https://cloudnative-pg.io/docs/1.30/recovery/).
Name the original cluster as the external source `serverName`, retain exact
PostgreSQL/extension and application image versions, and keep the new cluster
isolated from production traffic. Never perform PITR in place or let normal
Helm `initdb` overwrite a recovered cluster.

Pair each restored database with the matching protected issuer/Keycloak and
KMS recovery set. Run the ordinary database role/epoch preflight, then the
[public functional checks](OPERATIONS.md#restore-and-move-starter-data-to-external-services):
login, Session append, Knowledge history/search, context, governed mutation,
forced-RLS inventory, frozen audit-prefix equality and a new valid audit
append. Rebuild/verify indexes and measure from target decision to complete
public-API service recovery. Record target time, last archived WAL, achieved
data loss, duration and failures without logging content or secrets. The
restore drill must include missing/wrong keys, incomplete backup, corrupt WAL,
provider outage and expired credential cases before production promotion.

The source pilot does **not** define retention, RPO/RTO, or a monthly drill. It
also does not add WAL/PITR to Compose or external PostgreSQL. Those deployment
owners need their own tested implementation and the same joint recovery proof.

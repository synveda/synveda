# Customer recipes

These files ship inside the chart archive. They are ordinary Helm values;
`ci/` fixtures are unnecessary. Follow the [linear installation guide](../README.md)
for context selection, verification and installation.

<!-- chart-package-source-status:start -->
The current files and
native utilities are source candidates, absent from the published v0.4.3 archive.
<!-- chart-package-source-status:end -->

Start with local evaluation for one person, shared HTTPS with bundled services
for a team without service owners, or existing CNPG only when its operator
already runs. PostgreSQL and identity ownership remain independent:

| Database | Identity | Maintained customer values |
| --- | --- | --- |
| Bundled persistent PostgreSQL | Bundled Keycloak | `bundled-database.json` + `bundled-identity.json` |
| Bundled persistent PostgreSQL | Existing OIDC | `bundled-database.json` |
| Existing PostgreSQL | Bundled Keycloak | `external-database.json` + `bundled-identity.json`, with its separate identity database inputs |
| Existing PostgreSQL | Existing OIDC | `external-database.json` |
| Existing CNPG | Bundled Keycloak | `existing-cnpg.json` + `bundled-identity.json` |
| Existing CNPG | Existing OIDC | `existing-cnpg.json` |

Preparation selects these same files, derives release/namespace/service names,
and records the operator's choices. Tests render these files directly and from
an extracted chart. Non-secret output can be reviewed and used with plain Helm
or GitOps. Secrets and `state.json` must stay outside Git.

## Existing services

Keep the guide's exported `CHART`, `CONTEXT`, `NAMESPACE`, `RELEASE`, `IMAGES`,
`PREPARED`, `ARCHITECTURE`, `APP_URL`, `INGRESS_CLASS` and TLS Secret names.
Commands here run on the workstation. Replace service endpoints once in your
ordinary JSON values file. Complete the [DBA handoff](../CONFIGURATION.md#dba-handoff)
and [OIDC worksheet](../CONFIGURATION.md#external-oidc-worksheet) first.

For **external database and external identity**:

```sh
export SERVICE_VALUES="$PWD/existing-services.json"
cp "$CHART/examples/external-database.json" "$SERVICE_VALUES"
# Edit SERVICE_VALUES: actual PostgreSQL host, CA, three Secret names and role contract.
# Create the private issuer file using the OIDC worksheet; never put it in Helm values.
export ISSUER_FILE="$HOME/.synveda-issuer/issuers.json"
node "$CHART/examples/prepare.mjs" --route shared --database external --identity external \
  --context "$CONTEXT" --namespace "$NAMESPACE" --release "$RELEASE" \
  --app-url "$APP_URL" --ingress-class "$INGRESS_CLASS" --app-tls-secret application-tls \
  --values "$SERVICE_VALUES" --issuer-file "$ISSUER_FILE" --images "$IMAGES" --output "$PREPARED"
```

For **bundled database and external identity**, remove `--values` unless you
need ordinary advanced values, use `--database bundled`, and add
`--storage-class "$STORAGE_CLASS" --storage-size 20Gi`. The issuer file is
still required. There is no Keycloak administrator, identity ingress or
identity database in this combination.

For **external database and bundled identity**, use the shared bundled
identity command in the main guide with `--database external --values
"$SERVICE_VALUES"`, removing storage arguments. Add this ordinary object to
that JSON file, with actual endpoints/references:

```json
{
  "keycloak": {
    "databaseCaExistingSecret": "identity-postgres-ca",
    "database": {
      "hostname": "identity-postgres.example.test",
      "port": 5432,
      "existingSecret": "identity-postgres-password"
    }
  }
}
```

This is an additional object alongside `postgres`, gateway/worker and other
references, not a replacement document. The identity DBA creates the separate
`keycloak` database owned by only `keycloak`, with its own password and verified
TLS. If both databases share a server, the application role contract must name
`keycloak` as a forbidden database and isolated peer role. Preparation derives
that restriction for the declared identical host; declare it yourself for two
DNS names pointing to one server. No external credential or administrative
extension is generated or applied by preparation.

For **existing CNPG**, select `--route cnpg`. Merge the verified CNPG image
reference into the same release overlay as described in the main guide. Its
operator creates the migrator owner and CA Secrets; do not pre-create them.
Use `postgres.storage.size/storageClass` for CNPG and
`postgres.bundled.size/storageClass` for bundled PostgreSQL. Non-default fields
for the other mode are refused; Helm's merged default fields cannot reveal
whether a caller explicitly repeated a default.

## Preparation and retry

`node "$CHART/examples/prepare.mjs" ...` is the single entry point. The
`prepare-local.sh` compatibility wrapper invokes it natively and takes the same
explicit `--context` and `--images` inputs. Neither needs Docker.

The output directory is mode 0700 and files mode 0600, owned by the current
ordinary Unix user. Identifiers/passwords/CA/KMS material are committed to
`state.json` before derived outputs. Repeat the same inputs to resume a missing
output or after an interruption. A conflicting file or different issuer,
context, release, namespace, image evidence or values stops without replacing
credentials. Restore matching files from recovery; do not delete private state
for a retained installation. A leftover empty `prepare.lock` may be removed
only after confirming its process has stopped.

For direct Helm use without this entry point, follow the
[manual equivalent](../MANUAL.md). Installation, retry and uninstall use the
same ordinary chart contract. See the [runbook](../OPERATIONS.md) for recovery.

`s3-backup-objectstore.yaml` is a source-only optional backup example. Its
bucket/endpoint/Secret placeholders require independent ownership and the
[backup guide](../BACKUP.md); it is not a default first-install dependency.

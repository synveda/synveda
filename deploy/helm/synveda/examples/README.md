# Customer recipes

These files ship inside the chart archive. They are ordinary Helm values;
`ci/` fixtures are unnecessary. Follow the [linear installation guide](../README.md)
for context selection, verification and installation.

<!-- chart-package-source-status:start -->
These recipes and native utilities ship in the controlled evaluation v0.4.4
chart. Use its matching verified archive and image overlay.
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

### Reuse existing services on Azure

For a small team with an existing AKS cluster, select **external PostgreSQL +
external OIDC** using the recipe above. Reuse its namespace, ingress, DNS/TLS,
database and identity owners. Synveda installs one gateway serving the console,
one worker and the bounded installation Job, with ordinary namespaced resources.
This combination creates no PostgreSQL, Keycloak, CNPG operator or application
data PVC. Preparation runs natively without Docker. AKS and Azure service
combinations still require live qualification; see [platform evidence](../PORTABILITY.md#environments-and-evidence).

**Check the database first.** As checked on 2026-10-10,
[Microsoft lists pgvector 0.8.2 for PostgreSQL 17](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-versions#vector).
The source compatibility candidate admits that exact 0.8.2 catalogue profile;
released v0.4.4 still refuses it. Use an application image and CLI containing
[ADR-0140](../../../../docs/adr/adr-0140-finite-pgvector-catalogue-profiles.md)
for evaluation, and read the known native maintenance/security limits in the
compatibility contract. External mode can reuse a database hosted
in Azure only when it meets the same [compatibility contract](../../../../docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility)
and [provisioning requirements](../CONFIGURATION.md#provisioning-and-privileges).
The [issue #72 plan](../../../../docs/backlog/OPS-11.md#issue-72-postgresql-extension-compatibility-plan)
records the separate patched-version and provider qualification work.

1. **Inspect the existing database without changing it.** The DBA uses a
   private `pg_service.conf` and password file with `sslmode=verify-full` and
   the issuing CA, as in the [DBA handoff](../CONFIGURATION.md#dba-handoff).
   The service below must select the intended application database:

   ```sh
   psql 'service=synveda-provisioning' -X -v ON_ERROR_STOP=1 <<'SQL'
   SHOW server_version;
   SELECT name, version, installed
   FROM pg_available_extension_versions
   WHERE name IN ('vector', 'btree_gin')
   ORDER BY name, version;
   SELECT e.extname, e.extversion, n.nspname,
          pg_get_userbyid(e.extowner) AS owner
   FROM pg_extension e
   JOIN pg_namespace n ON n.oid = e.extnamespace
   ORDER BY e.extname;
   SQL
   ```

   If the database does not exist yet, inspect available versions through the
   DBA's maintenance connection first; installed extensions belong to that
   selected database. Source admission requires PostgreSQL 17, `vector` 0.8.2 or 0.8.6
   and `btree_gin` 1.3 in `public`, and `plpgsql` 1.0 in `pg_catalog`, plus
   the required ownership/catalogue proof. A version match alone is insufficient.
   Do not downgrade a patched server to satisfy the pin or edit its catalogue.

2. **Complete the service-owner handoff.** After the version gate, the DBA
   provisions the exact ordinary roles and supplies three distinct credential
   URL Secrets plus the PostgreSQL CA Secret. Each URL includes
   `sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt`.
   Edit `SERVICE_VALUES` with the actual host, Secret references and role
   contract; the example administrator `postgres` is not an Azure mapping.
   Provider-owned extension owners, administrator/grantor memberships,
   `pg_control_system()` access and effective denial of maintenance/peer
   database CONNECT must also pass. On a shared server, the DBA arranges that
   contract without copying the dedicated-server revocations onto other users.

   Flexible Server's [extension allow list](https://learn.microsoft.com/en-us/azure/postgresql/extensions/how-to-allow-extensions)
   is managed under **Settings → Parameters → azure.extensions** in the Azure
   portal. Preserve the existing selection when allowing `vector` and
   `btree_gin`. Allowlisting does not supply a missing version or establish
   compatibility; Microsoft [does not permit custom extension packages](https://learn.microsoft.com/en-us/azure/postgresql/extensions/how-to-create-extensions).

   For a new Synveda database offering 0.8.2, the declared extension
   administrator preinstalls `vector` with
   `CREATE EXTENSION vector WITH SCHEMA public VERSION '0.8.2';`
   and `btree_gin` 1.3 before application migration. Keep a verified existing
   installation; the application performs no automatic version change.

   The identity owner completes the [OIDC worksheet](../CONFIGURATION.md#external-oidc-worksheet):
   public authorization-code/S256 PKCE client, exact issuer and API audience,
   callback `$APP_URL/auth/callback`, required groups and stable tenant UUIDv7.
   Entra ID needs real token, group-claim and human-admission qualification;
   no working Entra recipe or managed-identity database authentication is claimed.

3. **Prepare the application.** Use the installation guide's verified matching
   chart/image overlay and exported inputs, then run the external/external
   preparation command above. Review its `values.json` and preserve the private
   `PREPARED` directory and original KMS/issuer recovery material. Keep one
   gateway, one worker and the deterministic starter providers. An existing
   private OTLP/gRPC collector is optional through `otel.endpoint`; its
   transport and forwarding contract is in [configuration](../CONFIGURATION.md).

4. **Validate, probe and install.** Run the guide's [offline, Secrets, API
   preflight and Helm lint commands](../README.md#5-validate-and-install)
   sequentially. Ensure Pods can reach the database and issuer through the
   existing private network/firewall; workstation reachability is insufficient.
   Then run:

   ```sh
   node "$CHART/examples/operator.mjs" database-probe \
     --prepared "$PREPARED" --architecture "$ARCHITECTURE"
   ```

   Require the printed context/namespace-pinned `kubectl wait` command to
   complete. This probe checks connectivity, TLS, roles and database identity;
   migration performs the complete extension proof before application DDL.
   Only after these prerequisites pass, install:

   ```sh
   helm upgrade --install "$RELEASE" "$CHART" \
     --kube-context "$CONTEXT" -n "$NAMESPACE" \
     -f "$PREPARED/values.json" -f "$PREPARED/release-images.yaml" \
     --wait --wait-for-jobs --timeout 15m
   ```

5. **Verify first use.** Follow [browser sign-in](../README.md#6-open-the-console-and-sign-in),
   `synveda whoami --capabilities`, the governed Knowledge/context workflow and
   [private readiness verification](../README.md#8-verify-retry-stop-and-uninstall-safely).
   For failure, run `node "$CHART/examples/operator.mjs" diagnose --prepared "$PREPARED"`,
   correct the named prerequisite and retry with the original files.

For a team with an existing Azure VM and container engine instead of a cluster,
use [Compose with existing infrastructure](../../../compose/PREBUILT.md#use-existing-infrastructure)
and select `SYNVEDA_POSTGRES_MODE=external` and `SYNVEDA_OIDC_MODE=external`.
Follow its protected-file, issuer and reference-HTTPS inputs. From the extracted
release bundle run `./synveda-compose config`, `./synveda-compose up` and
`./synveda-compose smoke`; a source checkout uses the guide's `make compose-*`
equivalents. This omits bundled PostgreSQL/Keycloak while retaining the canonical
proxy, private collector and lifecycle services. The same database compatibility
gate applies. Neither route provisions Azure infrastructure.

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

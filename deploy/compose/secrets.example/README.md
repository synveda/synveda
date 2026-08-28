# Canonical Compose secret files

Generate the bundled-development set with:

```sh
deploy/compose/scripts/generate-secrets.sh
```

The script writes only to the ignored `secrets` sibling directory, uses
operating-system entropy, sets mode `0600`, refuses any existing target unless
`--force` is explicit, and prints filenames rather than values. The directory
and its contents are ignored by Git and the Docker build context.

Required core filenames:

- `synveda_migrator_database_url`
- `synveda_gateway_database_url`
- `synveda_worker_database_url`
- `synveda_kms_key`
- `synveda_kms_key_ref`

Bundled PostgreSQL and Keycloak additionally use:

- `postgres_owner_password`
- `synveda_migrator_password`
- `synveda_gateway_password`
- `synveda_worker_password`
- `keycloak_database_password`
- `keycloak_admin_username`
- `keycloak_admin_password`

Reference certificate-file mode additionally requires operator-supplied
`tls_cert` and `tls_key`. The generator never invents a certificate.

External PostgreSQL operators replace the three role-specific database URL
files with separately provisioned least-privilege connections. Owner,
migration, bootstrap and backup credentials must never be copied into the
gateway or worker files.

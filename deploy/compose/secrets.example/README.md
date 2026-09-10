# Canonical Compose secret files

Generate the bundled-development set with:

```sh
deploy/compose/scripts/generate-secrets.sh
```

The script writes the credential set to the ignored `secrets` sibling and
creates project-scoped, ignored database-authority and Keycloak public-gate
state below `runtime/synveda-<runtime>/`. It uses operating-system entropy,
sets private directory/file modes to `0700`/`0600`, and prints filenames rather
than values. All three roots are ignored by Git and the Docker build context.

An existing credential set is refused unless `--force` is paired with the
exact project confirmation, for example:

```sh
SYNVEDA_CONFIRM_SECRET_REPLACEMENT=synveda-development \
  deploy/compose/scripts/generate-secrets.sh --force
```

The old set is moved to the project runtime directory as `previous-secrets`.
This preserves recovery material; it is not an automatic credential-rotation
workflow, and a second replacement is refused while that preserved set exists.

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
- `keycloak_convergence_admin_password`

Reference certificate-file mode additionally requires an operator-supplied
leaf-first PEM fullchain (leaf plus any intermediates, with the trust root
omitted) in `tls_cert` and its matching unencrypted PEM private key in
`tls_key`. The generator creates neither file and never invents a certificate.

External PostgreSQL operators replace the three role-specific database URL
files with separately provisioned least-privilege connections. Owner,
migration, bootstrap and backup credentials must never be copied into the
gateway or worker files.

# Canonical Docker Compose deployment (CPR-45)

This directory is being cut over additively from the still-current contributor
stack in `docker-compose.yml`. The canonical files define the provider-neutral
graph, isolated networks, role-specific secret mounts, container security
baseline, database/role convergence and runtime database-authority preflight.
It is not yet the executable reference deployment: Keycloak realm convergence,
the exact issuer diagnostic and clean-volume login acceptance remain open, so
there is intentionally no canonical `up`, `smoke`, `down` or `reset` action.

`make dev-up` and `make smoke` remain contributor-only Rauthy/Temporal paths.
The `synveda init` lifecycle and installed release profile are withdrawn, not
advanced-operator alternatives. None is Docker-reference evidence.

## File selection

The reviewed wrapper selects files in this order:

1. `compose.yaml`;
2. exactly one of `compose.dev.yaml` or `compose.reference.yaml`;
3. `compose.postgres.yaml` when PostgreSQL is bundled;
4. `compose.keycloak.yaml` when OIDC is bundled;
5. `compose.keycloak-postgres.yaml` or
   `compose.keycloak-external-postgres.yaml` for the selected Keycloak database;
6. `compose.external-postgres.yaml` when PostgreSQL is external;
7. `compose.external.yaml` when either dependency is external.

The defaults are bundled development. Validate the complete eight-row matrix
without starting or pulling images:

```sh
make compose-config
```

To validate one operator selection, create role-specific secrets and the
private database-authority directory, prepare a real issuer file from
`configs/oidc/issuers.example.json`, and run:

```sh
deploy/compose/scripts/generate-secrets.sh
SYNVEDA_OIDC_ISSUERS_FILE=/absolute/path/to/issuers.json \
  deploy/compose/scripts/compose.sh config
```

Bootstrap refuses before mutation if the owner, migrator, gateway or worker
password files contain equal values. The bundled-Keycloak/shared-PostgreSQL
row includes the Keycloak database password in that same five-way comparison;
external-OIDC rows do not mount it into the Synveda bootstrap.

Development is explicit HTTP at `app.synveda.test` and
`auth.synveda.test`, normally through loopback port 8080. Add both names to
the host resolver; `.localhost` is deliberately not used across container
namespaces. Reference validation requires HTTPS, non-test DNS, certificate
files and digest-pinned product/PostgreSQL/Keycloak images.

The selector bounds and protects the issuer file but does not parse it. The
selected issuer and that file must pass the still-open product-owned exact
issuer diagnostic before this graph may be started or treated as acceptance.
Bundled PostgreSQL selects the exact checked-in role contract for its OIDC
topology. External PostgreSQL requires the operator to set
`SYNVEDA_DATABASE_ROLES_FILE` to an explicit contract naming every existing
peer and maintenance database whose CONNECT denial must eventually be proved.
Those rows are configuration tests, not startable deployments: every database
bootstrap command with `SYNVEDA_POSTGRES_BUNDLED_CLUSTER=false` refuses before
mounted-secret reads or `psql` until an authenticated-TLS bootstrap contract is
implemented. Bundled Keycloak with external PostgreSQL therefore cannot yet
create or converge its database, even if a provider has pre-provisioned the
roles and administrator URL.

## Current limits

This additive checkpoint is not install, login, backup, restore, upgrade,
Linux/Desktop portability or controlled-use evidence. It does not change the
current support matrix or production-readiness verdict. External PostgreSQL is
configuration shape only: the compiled SQLx driver and bootstrap transport have
no accepted authenticated-TLS path.
The Collector currently discards traces through its private `nop` exporter;
the optional bounded observability profile is still open.

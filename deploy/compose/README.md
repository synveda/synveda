# Canonical Docker Compose deployment (CPR-45)

This directory is being cut over additively from the still-current contributor
stack in `docker-compose.yml`. The canonical files define the provider-neutral
graph, isolated networks, role-specific secret mounts, container security
baseline, database/role convergence and runtime database-authority preflight.
The graph now includes fail-closed Keycloak realm convergence and the
product-owned exact issuer diagnostic. It is not yet the executable reference
deployment: the canonical lifecycle wrapper and clean-volume login acceptance
remain open, so there is intentionally no canonical `up`, `smoke`, `down` or
`reset` action.

`make dev-up` and `make smoke` remain contributor-only Rauthy/Temporal paths.
The `synveda init` lifecycle and installed release profile are withdrawn, not
advanced-operator alternatives. None is Docker-reference evidence.

## File selection

The reviewed wrapper selects files in this order:

1. `compose.yaml`;
2. exactly one of `compose.dev.yaml` or `compose.reference.yaml`;
3. in development, `compose.postgres.dev.yaml` and/or
   `compose.keycloak.dev.yaml` for the bundled identity and database-bootstrap images;
4. `compose.postgres.yaml` when PostgreSQL is bundled;
5. `compose.keycloak.yaml` when OIDC is bundled;
6. `compose.keycloak-postgres.yaml` or
   `compose.keycloak-external-postgres.yaml` for the selected Keycloak database;
7. `compose.external-postgres.yaml` when PostgreSQL is external;
8. `compose.external.yaml` when either dependency is external.

The defaults are bundled development. Validate the complete eight-row matrix
without starting or pulling images:

```sh
make compose-config
```

To validate one bundled-OIDC operator selection, create role-specific secrets,
the private project state directories and the credential-free static-tenant
issuer contract, then render:

```sh
deploy/compose/scripts/generate-secrets.sh
deploy/compose/scripts/generate-issuer.sh
SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24 \
  deploy/compose/scripts/compose.sh config
```

The issuer generator writes `runtime/<project>/issuers.json` atomically with
mode 0600 and never prints its content. Its default bootstrap tenant UUIDv7 can
be replaced with `SYNVEDA_BOOTSTRAP_TENANT_ID`; replacing an existing file
requires both `--force` and the exact project confirmation. External OIDC
continues to require an operator-supplied issuer file.

Development selection now includes source builds for the product, proxy,
PostgreSQL and optimized Keycloak images. This closes the local image build
graph but does not make the still-config-only wrapper an executable lifecycle.
The image also exposes an exact, audited `tenant-converge` command for the
future bootstrap job; no canonical service invokes it in this checkpoint.

Replace the example pool with a canonical private `/24` chosen for this exact
project. The selector deterministically divides it into ten `/28` networks;
reference and `acceptance-*` projects refuse an implicit pool. A project suffix
isolates names and volumes but not address space, so concurrent or retained
projects need distinct recorded pools. The unsuffixed development default is a
convenience only and can collide with Docker daemon pools, VPNs or provider
routes.

`compose config` is deliberately daemon-independent and therefore does not
claim a free pool. Before a future canonical `up`, the lifecycle preflight must
check interval overlap against every Engine network and accept a rerun only
when the exact project-owned names, labels and IPAM match. It must never remove
a conflicting network. Host/VPN route inspection and functional external-
dependency smoke tests are separately required, especially on Docker Desktop
where bridge routes live inside its VM.

Bootstrap refuses before mutation if the owner, migrator, gateway or worker
password files contain equal values. The bundled-Keycloak/shared-PostgreSQL
row includes the Keycloak database password in that same five-way comparison;
external-OIDC rows do not mount it into the Synveda bootstrap.

Bundled-OIDC development is explicit HTTP at `app.synveda.test` and
`auth.synveda.test`, normally through loopback port 8080. Add both names to
the host resolver. A selected `SYNVEDA_DEV_HTTP_PORT` from 1024 through 65535,
excluding Caddy's reserved development HTTPS convention port 8443, is both
Caddy's container listener and the loopback-published port. Browser and
containers therefore use the exact same issuer authority; host-only port
translation is refused. `.localhost` is deliberately not used across container
namespaces. External-OIDC development maps only the application name locally;
the provider issuer retains its real DNS and edge. Reference acceptance
requires HTTPS, non-test DNS, certificate files whose SANs cover both bundled
hosts (only the application host for external OIDC), and digest-pinned
product/proxy/Collector plus provider/bootstrap images selected by the row.

The selector bounds and protects the issuer file but does not parse it. The
rendered graph runs the product-owned exact issuer diagnostic before the
gateway; a configuration render alone is not diagnostic or acceptance
evidence.
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
Linux/Docker-Desktop portability or controlled-use evidence. It does not change the
current support matrix or production-readiness verdict. External PostgreSQL is
configuration shape only: the compiled SQLx driver and bootstrap transport have
no accepted authenticated-TLS path.
The Collector currently discards traces through its private `nop` exporter;
the optional bounded observability profile is still open.

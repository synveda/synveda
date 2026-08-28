# Canonical Docker Compose deployment (CPR-45)

This directory is being cut over additively from the still-current contributor
stack in `docker-compose.yml`. The canonical files already define the intended
provider-neutral graph, isolated networks, role-specific secret mounts and
container security baseline. They are currently a **static configuration
contract only**: database/role convergence, Keycloak realm convergence and the
exact issuer diagnostic have not landed, so there is intentionally no
canonical `up`, `smoke`, `down` or `reset` action yet.

`make dev-up`, `make smoke`, `synveda init` and the installed release profile
remain transitional Rauthy/Temporal paths until the replacement passes clean
volume login and restart acceptance. They are not Docker-reference evidence.

## File selection

The reviewed wrapper selects files in this order:

1. `compose.yaml`;
2. exactly one of `compose.dev.yaml` or `compose.reference.yaml`;
3. `compose.postgres.yaml` when PostgreSQL is bundled;
4. `compose.keycloak.yaml` when OIDC is bundled;
5. `compose.external.yaml` when either dependency is external.

The defaults are bundled development. Validate the complete eight-row matrix
without starting or pulling images:

```sh
make compose-config
```

To validate one operator selection, create role-specific secrets, prepare a
real issuer file from `configs/oidc/issuers.example.json`, and run:

```sh
deploy/compose/scripts/generate-secrets.sh
SYNVEDA_OIDC_ISSUERS_FILE=/absolute/path/to/issuers.json \
  deploy/compose/scripts/compose.sh config
```

Development is explicit HTTP at `app.synveda.test` and
`auth.synveda.test`, normally through loopback port 8080. Add both names to
the host resolver; `.localhost` is deliberately not used across container
namespaces. Reference validation requires HTTPS, non-test DNS, certificate
files and digest-pinned product/PostgreSQL/Keycloak images.

The selector bounds and protects the issuer file but does not parse it. The
selected issuer and that file must pass the still-open product-owned exact
issuer diagnostic before this graph may be started or treated as acceptance.

## Current limits

This additive checkpoint is not install, login, backup, restore, upgrade,
Linux/Desktop portability or controlled-use evidence. It does not change the
current support matrix or production-readiness verdict. External PostgreSQL is
configuration shape only: the compiled SQLx driver has no accepted TLS path.
The Collector currently discards traces through its private `nop` exporter;
the optional bounded observability profile is still open.

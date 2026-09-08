# Synveda deployment contract

Status: current contract for CPR-45 and ADR-0102, as amended by ADR-0105.

This document defines the application/deployment boundary shared by direct
binary execution, Docker Compose and later Kubernetes packaging. The
authoritative implementation is the product image, its commands, the
configuration readers, the database schema and the generated public API.

## Principles

- Synveda is one application with governed configuration profiles. There are
  no personal, team or enterprise editions in the runtime.
- Deployment mechanisms may change how values and files are supplied, but not
  their meaning.
- PostgreSQL, OIDC/OAuth 2.0 and OTLP are provider-neutral boundaries.
- Cedar, forced RLS, VedaFlow and content-free audit remain authoritative in
  every deployment mode.
- Docker Compose is the executable single-host reference topology. It is not
  an HA or disaster-recovery claim.
- Helm implements this contract with Kubernetes primitives; it is not generated
  from Compose.

## Canonical Compose assembly

Operators invoke deploy/compose/scripts/compose.sh; they do not select
fragments by hand. The selector validates the closed runtime/provider matrix
and assembles these files:

| File | Responsibility |
| --- | --- |
| compose.yaml | provider-neutral proxy, convergence jobs, gateway, worker and Collector |
| compose.dev.yaml | local builds and loopback HTTP |
| compose.reference.yaml | bounded reference resources, ports 80/443 and certificate secrets |
| compose.postgres.yaml | bundled PostgreSQL and Synveda role/database bootstrap |
| compose.keycloak.yaml | bundled optimized Keycloak and realm convergence |
| compose.keycloak-postgres.yaml | shared-server bootstrap ordering |
| compose.external-postgres.yaml | external PostgreSQL mounts and labels |
| compose.external.yaml | external provider labels |
| compose.demo.yaml | short-lived demo users |
| compose.browser-acceptance*.yaml | isolated browser acceptance runner |

Implemented selectors are development or reference,
bundled or external PostgreSQL and bundled or external OIDC. External
PostgreSQL is configuration-renderable but canonical start/reset is still
refused. Demo and browser-acceptance are implemented profiles. Other optional
profiles are not part of the current executable contract until their services
and acceptance tests land.

## Images and commands

| Image | Commands or role |
| --- | --- |
| Synveda product | gateway, worker, database-preflight, migrate, tenant-converge, issuer-diagnostic |
| PostgreSQL 17 + pgvector | bundled database and the bounded database bootstrap |
| optimized Keycloak 26.7.2 | start --optimized and idempotent realm convergence |
| Caddy 2.11.4 | public reverse proxy |
| OpenTelemetry Collector Contrib 0.159.0 | private OTLP receiver |
| Playwright 1.62.1 | disposable browser acceptance only |

The product image contains both gateway and worker binaries. The selected
command, not a deployment-specific image or code branch, chooses the process.
Runtime base images are digest pinned in their Dockerfiles. Development tags
are local conveniences; reference evidence requires immutable release
references.

## Service graph

The bundled reference graph is:

    postgres
    ├── database-bootstrap ── database-preflight ── migrate ── tenant-convergence
    └── keycloak-database-bootstrap ── keycloak ── keycloak-realm-convergence
                 └──────────────────── database-preflight

    keycloak-realm-convergence ── proxy ── issuer-diagnostic
    tenant-convergence + issuer-diagnostic ── gateway
    tenant-convergence + issuer-diagnostic ── worker
    gateway + worker ── otel-collector

The bootstrap, preflight, migration, tenant convergence and issuer diagnostic
services are bounded jobs. Proxy, PostgreSQL, Keycloak, realm convergence,
gateway, worker and Collector are long-running.

## Ports and health

Only the proxy publishes host ports.

| Mode or service | Contract |
| --- | --- |
| development proxy | configured high port, bound to 127.0.0.1 |
| reference proxy | TCP 80 and 443 |
| gateway | private port 8120; /healthz, /readyz, /metrics |
| worker | private loopback port 8121; /healthz, /readyz, /metrics |
| Keycloak application | private port 8080 |
| Keycloak management | private port 9000 |
| OTLP | private ports 4317 and 4318 |
| Collector health | private loopback port 13133 |
| PostgreSQL | private port 5432 |

The proxy does not publish application metrics. Keycloak management, worker
health, Collector receivers and PostgreSQL are never public routes.

## Configuration

The Compose selector validates and derives the runtime settings. Its
.env.example contains only non-secret defaults and placeholders.

| Setting | Meaning |
| --- | --- |
| SYNVEDA_COMPOSE_RUNTIME | development or reference |
| SYNVEDA_POSTGRES_MODE | bundled or external |
| SYNVEDA_OIDC_MODE | bundled or external |
| SYNVEDA_COMPOSE_PROFILES | closed comma-separated optional profile set |
| SYNVEDA_APP_HOST | browser-visible application DNS name |
| SYNVEDA_AUTH_HOST | browser-visible bundled issuer DNS name |
| SYNVEDA_PUBLIC_SCHEME | development http or reference https |
| SYNVEDA_DEV_HTTP_PORT | loopback development port |
| SYNVEDA_TLS_MODE | reference currently supports files |
| SYNVEDA_OIDC_ISSUER | exact external issuer URL |
| SYNVEDA_OIDC_ISSUERS_FILE | mounted provider-neutral issuer document |
| SYNVEDA_DATABASE_ROLES_FILE | mounted database role contract |
| SYNVEDA_COMPOSE_IPV4_POOL | explicit private /24 for reference/evidence |
| SYNVEDA_PRODUCT_IMAGE | immutable product image reference |
| SYNVEDA_POSTGRES_IMAGE | immutable bundled PostgreSQL image reference |
| SYNVEDA_KEYCLOAK_IMAGE | immutable bundled Keycloak image reference |
| SYNVEDA_CADDY_IMAGE | immutable proxy image reference |
| SYNVEDA_OTEL_COLLECTOR_IMAGE | immutable Collector image reference |
| OTEL_EXPORTER_OTLP_ENDPOINT | OTLP/gRPC destination used by application processes |

The application processes receive DATABASE_URL_FILE,
SYNVEDA_KMS_KEY_FILE, SYNVEDA_KMS_KEY_REF_FILE,
SYNVEDA_OIDC_ISSUERS_FILE, their listen address and the public URL.
Standard outbound proxy/custom-CA support is an external-dependency gap; no
provider-specific value enters a domain crate or public DTO.

Object storage and SMTP are not active runtime dependencies in this reference
checkpoint. If enabled later, they must be expressed through provider-neutral
S3-compatible and SMTP settings without changing public or domain contracts.

## Secret files

generate-secrets.sh creates mode-0600 files and refuses overwrite unless the
operator explicitly requests it. Direct secret environment variables are
rejected by the selector.

Bundled mode uses these files under the selected secret directory:

- postgres_owner_password
- synveda_migrator_password
- synveda_gateway_password
- synveda_worker_password
- synveda_migrator_database_url
- synveda_gateway_database_url
- synveda_worker_database_url
- keycloak_database_password
- keycloak_admin_username
- keycloak_admin_password
- keycloak_convergence_admin_password
- synveda_kms_key
- synveda_kms_key_ref
- tls_cert and tls_key in reference mode
- demo credentials only when the demo profile is selected

The Keycloak entrypoint reads upstream-required values from mounted files,
exports them only to its child, and execs Keycloak without printing them.
Secrets must not appear in Compose YAML, committed .env files, image layers,
logs or environment manifests.

## PostgreSQL ownership and persistent data

One bundled PostgreSQL server may host both products, but isolation is
database- and role-based:

- synveda is owned/migrated through the Synveda bootstrap and migrator roles;
- gateway and worker use separate least-privilege runtime roles;
- keycloak is owned by a dedicated keycloak login;
- Synveda runtime roles cannot connect to the Keycloak database;
- Keycloak has no privilege on Synveda data.

The postgres-data named volume is the bundled persistent database state.
Issuer projection, database authority and public realm gates are bounded
operator-owned runtime files, not independent data stores. The Synveda KMS key
is separate recovery material and must be protected with the database backup.

Migration ownership remains separate: Synveda runs migrate; Keycloak owns its
schema lifecycle. Keycloak realm export is not a database backup.

## OIDC contract

Synveda consumes standard discovery, authorization-code flow with PKCE S256,
JWKS and exact issuer/audience/algorithm validation. The application contains
no Keycloak-specific authorization branch.

Bundled mode provisions realm synveda, public PKCE clients, required claims
and the synveda-admins seed group. That group is used only for first
administrator admission; Synveda grants and Cedar remain authoritative.

The issuer string is identical in discovery, tokens and application
configuration. Proxy network aliases make the browser-visible authority
resolvable inside the Compose network. External mode omits Keycloak and uses
the same issuer document and diagnostic.

## Reverse proxy and TLS

Development is explicitly insecure HTTP on loopback with .test hostnames.
Reference mode requires operator DNS, HTTPS and mounted certificate/key files.
ACME automation is not yet implemented.

The proxy overwrites forwarding headers and strips caller-supplied identity and
distributed-tracing headers. It bounds headers, request bodies and upstream
timeouts. The bundled identity virtual host exposes only the realm discovery,
authorization, token, key, logout, account, login-action and static-resource
paths; administration and management remain private.

## Worker and operations

Gateway and worker are separate long-running processes with distinct database
credentials, readiness and shutdown bounds. Existing capture and maintenance
work owned by the worker must not move back into the gateway.

The experimental Apalis canary is not implemented yet. Its completion contract
is one non-destructive Skill validation operation, a provider-neutral
operation/attempt/outbox model with forced RLS, opaque task payloads and a leaf
adapter. The existing execution path remains the default and rollback.
Apalis identifiers or status vocabulary must not enter core crates or the
public API.

Temporal has no executable consumer and is not part of this deployment.

## Telemetry

Application processes emit traces through OTLP to the private Collector. The
Collector applies memory limiting and batching and currently terminates traces
at a no-op exporter. Application metrics are available only on private
Prometheus endpoints.

External OTLP export, a bounded local observability backend and the
customer-safe Operations route remain open CPR-45 slices. No prompt, message,
Knowledge body, credential or unbounded tenant/user label may enter telemetry.

## Backup and restore

Backup/restore is not implemented in the current graph. The minimal reference
completion is:

- PostgreSQL 17 pg_dump custom-format backups of both synveda and keycloak;
- separately protected Synveda KMS-key recovery material;
- an operator-owned local-filesystem target;
- integrity checks and restore into fresh, isolated volumes/network;
- verification of Keycloak data, Synveda data, role isolation, audit
  continuity, correct-key decryption and wrong-key failure.

This first establishes logical recovery validation. Bounded WAL/PITR and
S3-compatible target acceptance remain open CPR-45 slices. Even after those
pass, off-host retention policy, recurring drills and owned RPO/RTO remain
production work and no disaster-recovery claim follows.

## Lifecycle commands

Implemented:

    make compose-config
    make compose-secrets
    make compose-issuer
    make compose-hosts-plan
    make compose-hosts-install
    make compose-resolver-check
    make compose-up
    make compose-browser-acceptance
    make compose-acceptance
    make compose-smoke
    make compose-restart-gateway
    make compose-down
    make compose-reset

compose-acceptance requires a fresh suffixed bundled project and the exact
demo/browser profiles. Under one lock and deadline it performs browser login,
restarts each of the six long-running product/provider services independently,
runs the full smoke after every restart, and repeats browser login. It leaves
the successful stack running. compose-reset requires an exact confirmation
token and retains the project's secrets, issuer document and KMS key. Paired
backup/restore targets and upgrade smoke remain to be implemented. Live targets
must report an unavailable prerequisite distinctly from a passing test.

## Security and network boundary

Services use explicit networks, non-root users where upstream images permit,
read-only roots where compatible, dropped capabilities, no-new-privileges,
PID/resource bounds and Tini/Compose init handling. No service is privileged,
and no service mounts the Docker socket. Application runtime services never
receive owner database credentials; only the database server and bounded
bootstrap/recovery jobs may receive them.

Internal networks isolate application, Synveda data, Keycloak data, identity
management and telemetry. Explicit egress networks exist only for components
that require discovery/export access.

## Supported modes and limits

The completed and live-validated target is intended for local development,
demonstrations and a controlled single-host evaluation. The current
implementation remains validation-pending and does not establish:

- high availability or tolerance of host loss;
- zero-downtime upgrades;
- production SaaS readiness;
- multi-region operation;
- enterprise compliance certification;
- complete disaster recovery.

Before promotion, Helm must map the same image commands to Deployments/Jobs,
Secrets or external secret managers, Services/Ingress, NetworkPolicies,
security contexts, PVC/external PostgreSQL, external OIDC, external OTLP and
operator-owned backup facilities. Multi-replica prerequisites, disruption
budgets, topology spread, OpenShift arbitrary UID, offline/private-registry
distribution, customer CA/proxy, KMS and FIPS requirements remain explicit
promotion gaps.

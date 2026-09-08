# Canonical Docker Compose deployment

This directory is Synveda's canonical single-host deployment for CPR-45. It
runs the gateway and worker as separate processes with PostgreSQL, bundled
Keycloak, a reverse proxy and a private OpenTelemetry Collector.

It supports development and reference configuration. Live browser,
reference-HTTPS, recovery and upgrade evidence is still required before this
implementation can be called validated for controlled single-host use. It is
not an HA, disaster-recovery, hosted-SaaS or enterprise-certification claim.

Use deploy/compose/scripts/compose.sh through the Make targets. Do not assemble
Compose fragments manually.

## Prerequisites

- Docker Engine 28 or newer, reached through its local Unix socket;
- Docker Compose 2.33.1 or newer;
- Node.js 22 or newer;
- OpenSSL;
- a non-root Unix operator.

Hosts install/remove additionally requires root-owned, non-writable,
ACL-free Node 22 at /usr/bin/node or /usr/local/bin/node. Linux also requires
the fixed system getfacl binary from the acl package.

The lifecycle refuses remote Docker contexts because its loopback publication
and resolver checks describe the Engine host. Development also requires the
selected .test hostnames to resolve only to 127.0.0.1.

The default names are app.synveda.test and auth.synveda.test. The browser,
gateway, discovery document and tokens use the same issuer authority.

## Development hostname setup

Preview and inspect the owned hosts-file block:

    make compose-hosts-plan
    make compose-hosts-status

Install exactly the displayed block after reviewing the checkout and the
confirmation printed by the plan. For the default project:

    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-install

Run only that target with the required privilege escalation; do not run Docker,
the lifecycle or Make generally as root. Flush the active resolver cache, then
verify both the owned text and operating-system resolution:

    make compose-hosts-status
    make compose-resolver-check

The manager owns at most one marked block in /etc/hosts, refuses drift or
foreign equivalent rows, and keeps a private recovery record. Reference mode
uses operator DNS and never edits /etc/hosts.

## Default lifecycle

Development with bundled PostgreSQL and bundled Keycloak is the default:

    make compose-config
    make compose-up
    make compose-smoke
    make compose-restart-gateway
    make compose-down

compose-up:

1. validates the host, Docker socket, network range and inputs;
2. generates or validates project-scoped secrets and issuer configuration;
3. renders and validates the exact Compose graph;
4. builds development images when required;
5. creates the databases and roles;
6. runs Synveda migration and tenant convergence;
7. starts production-mode Keycloak and converges its realm;
8. diagnoses the exact public issuer;
9. starts separate gateway and worker processes.

Re-running compose-up is convergent and does not rotate secrets.

compose-smoke checks the expected services and completed jobs, public health
and console routes, OIDC discovery and the refusal of management/metrics
routes. It is not a browser-login test.

`make compose-acceptance` is the fresh-project gate. It requires the explicit
acceptance suffix and private `/24`, runs the real browser login, restarts
PostgreSQL, Keycloak, Collector, worker, gateway and proxy one at a time, runs
the full smoke after each, then repeats browser login. One project lock and
one bounded deadline cover the run. Success leaves the stack running for
inspection and later recovery/upgrade gates; reset remains separately
confirmed.

compose-down stops containers and preserves the PostgreSQL volume and generated
project inputs. The default ignored state directory is
deploy/compose/runtime/synveda-development.

## Secrets

Generate secrets explicitly when preparing a deployment:

    make compose-secrets
    make compose-issuer

The generator writes mode-0600 secret files below a mode-0700 project
directory, refuses overwrite unless explicitly forced and never prints secret
values. The checked-in .env.example contains no usable credential.

Do not put credentials, database URLs, KMS key material or client secrets in
Compose YAML, a committed environment file, an image layer or a shell command.
The canonical graph consumes mounted secret files.

## Bundled identity

Keycloak 26.7.2 is built as an optimized, pinned image and runs start
--optimized. Health and metrics are enabled on its private management port.
Only the required realm discovery, login, token, JWKS, logout, account and
static-resource routes are available through the public proxy.

The realm convergence service creates or reconciles:

- realm synveda;
- public authorization-code clients with PKCE S256;
- exact redirect origins;
- sub, email, name, groups and API-audience claims;
- the synveda-admins group;
- bounded token/session and brute-force settings.

The group seeds only the first Synveda administrator. Synveda grants, Cedar,
forced RLS, VedaFlow and audit remain authoritative.

Keycloak uses database keycloak and role keycloak. Synveda uses database
synveda with separate migrator, gateway and worker roles. Neither product role
has access to the other product's database.

## Demo and browser acceptance

The demo profile adds two short-lived, convergence-owned users:

    SYNVEDA_COMPOSE_PROFILES=demo make compose-up
    SYNVEDA_COMPOSE_PROFILES=demo make compose-smoke

Their passwords are generated into project-scoped secret files and should be
read only through a local password-input mechanism. The administrator belongs
to synveda-admins; the member receives no Keycloak domain role.

The existing isolated browser acceptance starts from an explicitly fresh,
suffixed project. If the ordinary development block is installed, stop that
project and hand off the one owned hosts-file block first:

    make compose-down
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove

Then select and install the acceptance project:

    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    make compose-hosts-plan
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development-acceptance-local:app.synveda.test:auth.synveda.test \
      make compose-hosts-install
    make compose-hosts-status
    make compose-resolver-check
    make compose-acceptance

It exercises real authorization-code login, PKCE S256, issuer/audience claims
and first-administrator admission through the same proxy authority used by
containers. It requires a real Docker Engine; deterministic fixture tests are
not live evidence. After inspecting the result, stop and reset that exact
project, then remove its owned hosts block:

    make compose-down
    SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-local make compose-reset
    unset SYNVEDA_COMPOSE_PROFILES
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development-acceptance-local:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove

Unset the acceptance selectors before returning to the ordinary project.
Reinstall its hosts block only when continuing development:

    unset SYNVEDA_COMPOSE_PROJECT_SUFFIX SYNVEDA_COMPOSE_IPV4_POOL
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-install
    make compose-resolver-check

## Reference HTTPS

Reference mode publishes only ports 80 and 443 and requires real operator DNS,
an explicit private /24, mounted certificate files and immutable image
references:

    export SYNVEDA_COMPOSE_RUNTIME=reference
    export SYNVEDA_PUBLIC_SCHEME=https
    export SYNVEDA_APP_HOST=app.example.com
    export SYNVEDA_AUTH_HOST=auth.example.com
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.44.0/24
    make compose-secrets

Place a leaf-first PEM chain in the selected project's tls_cert file and the
matching unencrypted PEM private key in tls_key. Both must be mode 0600 and
readable by the configured runtime UID/GID. Then supply matching
digest-addressed product, PostgreSQL, Keycloak and proxy image references. Use
one published environment manifest once release publication provides it:

    make compose-config
    make compose-up
    make compose-smoke

Certificate preflight checks format, chain adjacency, key match, hostnames and
remaining validity. It does not establish public trust, DNS ownership or
revocation status. ACME and automatic renewal are not implemented.

## External OIDC

The same product image runs without bundled Keycloak:

    make compose-down
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove
    export SYNVEDA_OIDC_MODE=external
    export SYNVEDA_OIDC_ISSUER=https://identity.example.com/realms/example
    export SYNVEDA_OIDC_ISSUERS_FILE=/absolute/path/to/issuers.json
    make compose-hosts-plan
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:- \
      make compose-hosts-install
    make compose-hosts-status
    make compose-resolver-check
    make compose-config
    make compose-up

The mounted issuer document owns the provider-neutral client ID, API audience,
supported algorithms, login scopes and static bootstrap-tenant binding. The
issuer diagnostic validates discovery, exact issuer, PKCE S256, JWKS and the
configured algorithm/audience contract before gateway readiness.

In development, the hosts manager owns only the application .test name in this
mode. The external provider retains its own DNS name. If no bundled hosts block
exists, skip its removal; if one exists, remove it before changing selectors so
the exact ownership confirmation remains valid. Reference mode uses operator
DNS and does not run the hosts targets.

## External PostgreSQL

External PostgreSQL rows render and validate through the same role and secret
file contract. Canonical up and reset currently refuse this mode because its
authenticated transport/bootstrap acceptance is unfinished. A successful
configuration render is not a live external-database claim.

## Network and edge contract

The selector divides one operator-selected private /24 into fixed internal and
egress networks and refuses foreign overlap. Only Caddy publishes host ports.

The proxy removes caller-supplied Forwarded, X-Forwarded-*, X-Real-IP,
identity, original-path and tracing/baggage headers before adding its own
bounded forwarding values. It applies request-body, header and upstream
timeouts. PostgreSQL, Keycloak management, worker health, metrics, OTLP
receivers, recovery jobs and operator UIs remain private.

Containers use non-root users where supported, read-only roots where
compatible, dropped capabilities, no-new-privileges, PID/resource bounds and
init/signal handling. Nothing is privileged and no service mounts the Docker
socket.

## Reset

Reset is destructive and requires the exact confirmation emitted by the
lifecycle:

    make compose-reset

It acts only on the validated project containers, networks, PostgreSQL volume
and transient database-authority/Keycloak-gate state. It deliberately retains
the project's secrets, issuer document and KMS key. Review the target and
supply the requested confirmation. Use compose-down when database state must
also be retained.

## Current completion gaps

The following work remains before the Docker reference can be called
implemented:

- paired logical backups of Synveda and Keycloak, isolated restore with the
  Synveda KMS key, and the bounded S3-compatible/WAL recovery path;
- one experimental forced-RLS operation/outbox and opaque-ID Apalis canary;
- a bounded local telemetry backend and customer-safe Operations route;
- canonical release/installer cutover and Rauthy deletion after live Keycloak
  browser acceptance;
- deterministic upgrade/rollback and external-dependency contract checks.

A general dashboard platform, ACME, HA, Helm promotion, signed provenance and
enterprise controls remain later work. Same-host backup and optional S3/PITR
acceptance do not establish disaster recovery or an owned RPO/RTO.

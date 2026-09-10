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
| compose.otlp-external.yaml | external OTLP export through the private Collector |
| compose.demo.yaml | short-lived demo users |
| compose.browser-acceptance*.yaml | isolated browser acceptance runner |
| compose.observability.yaml | optional private Collector metrics fan-in and loopback Prometheus UI |
| compose.apalis.yaml | optional private Apalis queue, one-shot migration and canary worker |
| compose.apalis.dev.yaml | development build wiring for the Apalis product commands |
| compose.backup.yaml | private, profile-gated logical backup one-shot |
| compose.restore.yaml | private isolated restore and key-verification one-shots |

Implemented selectors are development or reference, bundled or external
PostgreSQL, bundled or external OIDC, and discarded or external OTLP traces.
The executable external-PostgreSQL combination currently requires external
OIDC and operator-provisioned database roles. Compose applies Synveda schema
migrations and tenant convergence but does not provision, reset, back up or
restore that server. Demo, browser-acceptance, observability and experimental
Apalis are implemented profiles. Apalis currently requires bundled PostgreSQL
and bundled OIDC; backup, restore and upgrade actions refuse that profile.
The standalone gateway-restart action is also refused; full acceptance owns the
proved Apalis restart sequence.

## Packaged reference

The release artifact is `synveda-reference-<version>.tar.gz`. It contains only
the reference Compose closure, its fixed configuration and lifecycle scripts,
plus `environment.json`, `version`, `source-sha` and an executable
`synveda-compose` launcher. The environment manifest binds the source SHA to
six first-party image identities: product, single-host PostgreSQL, optimized
Keycloak, reference proxy, browser acceptance and CloudNativePG PostgreSQL. It
also records the digest-pinned Collector and Prometheus inputs.

The archive contains no development overlay, Dockerfile, database-test fixture,
local `.env`, secret, runtime state, backup, retired identity provider or
workflow-scheduler asset. Its launcher fixes reference HTTPS semantics and
immutable image references while leaving DNS, certificates and supported
external dependency settings operator-owned.

The installer stores immutable releases below
`$SYNVEDA_HOME/reference/releases/<version>-<source-sha>` and repoints
`$SYNVEDA_HOME/reference/current` at the selected release. Mutable deployment
inputs live below `$SYNVEDA_HOME/state/synveda-reference`; database and recovery
secret archives live under `$SYNVEDA_HOME/backups/{database,secrets}/synveda-reference`.
Upgrades preserve both roots. Automatic artifact removal remains fail-closed
until OPS-10 adds a strict installer-owned receipt; operators use the installed
launcher for `down` and confirmation-gated `reset`.

## Images and commands

| Image | Commands or role |
| --- | --- |
| Synveda product | gateway, worker, apalis-worker, apalis-migrate, database-preflight, migrate, migration-check, tenant-converge, issuer-diagnostic, and `probe {gateway\|worker\|apalis-worker} {live\|ready}` |
| PostgreSQL 17 + pgvector | bundled database, bounded bootstrap and logical backup/restore entrypoints |
| optimized Keycloak 26.7.2 | start --optimized and idempotent realm convergence |
| Caddy 2.11.4 | public reverse proxy |
| OpenTelemetry Collector Contrib 0.159.0 | private OTLP receiver |
| Prometheus 3.13.3 distroless | optional bounded local metrics store and loopback operator UI |
| Playwright 1.62.1 | disposable browser acceptance only |

The product image contains the closed process/operator command set above. The
selected command, not a deployment-specific image or code branch, chooses the
process.
Runtime base images are digest pinned in their Dockerfiles. Development tags
are local conveniences; reference evidence requires immutable release
references.

The restore overlay invokes
`synveda db recovery-verify --tenant <uuid> [--expect-key-refusal]` directly as
a private one-shot. It is a read-only post-restore verifier, not a public API
or a long-running product role. The PostgreSQL image exposes only the fixed
`synveda-logical-backup backup` and `synveda-logical-restore restore`
entrypoints for the two recovery one-shots.

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
    observability: gateway + worker metrics ── otel-collector ── prometheus
    optional traces: gateway + worker ── otel-collector ── external OTLP
    apalis: apalis-postgres ── apalis-migrate ── apalis-worker
                                      tenant-convergence ──┘

The bootstrap, preflight, migration, tenant convergence, issuer diagnostic,
Apalis migration and recovery services are bounded jobs. Proxy, PostgreSQL,
Keycloak, realm convergence, gateway, worker and Collector are long-running;
the Apalis queue and worker are long-running only when selected.

## Ports and health

Only the proxy publishes public host ports. The optional observability profile
also publishes its Prometheus operator UI on host loopback.

| Mode or service | Contract |
| --- | --- |
| development proxy | configured high port, bound to 127.0.0.1 |
| reference proxy | TCP 80 and 443 |
| gateway | private port 8120; /healthz, /readyz, /metrics |
| worker | private loopback port 8121 by default; observability explicitly binds it on private Compose networks without publishing it; /healthz, /readyz, /metrics |
| Apalis worker | optional private port 8122; /healthz, /readyz, /metrics |
| Keycloak application | private port 8080 |
| Keycloak management | private port 9000 |
| OTLP | private ports 4317 and 4318 |
| Collector health | private loopback port 13133 |
| Prometheus UI | optional port 9090 by default, bound to 127.0.0.1 |
| PostgreSQL | private port 5432 |
| Apalis PostgreSQL | optional private port 5432 on the data network only |

The proxy does not publish application metrics. Keycloak management, worker
health, Collector receivers and PostgreSQL are never public routes. Prometheus
is an operator endpoint, not a customer route; remote inspection requires an
operator-controlled loopback tunnel.

## Configuration

The Compose selector validates and derives the runtime settings. Its
.env.example contains only non-secret defaults and placeholders.

| Setting | Meaning |
| --- | --- |
| SYNVEDA_COMPOSE_RUNTIME | development or reference |
| SYNVEDA_POSTGRES_MODE | bundled or external |
| SYNVEDA_OIDC_MODE | bundled or external |
| SYNVEDA_OTLP_MODE | discard or external |
| SYNVEDA_COMPOSE_PROFILES | closed comma-separated optional profile set |
| SYNVEDA_EXECUTION_PROVIDER | `postgres` by default; the Apalis overlay sets exact `apalis` for Skill validation only |
| SYNVEDA_APP_HOST | browser-visible application DNS name |
| SYNVEDA_AUTH_HOST | browser-visible bundled issuer DNS name |
| SYNVEDA_PUBLIC_SCHEME | development http or reference https |
| SYNVEDA_DEV_HTTP_PORT | loopback development port |
| SYNVEDA_INSECURE_DEVELOPMENT_HTTP | exact `true` admits plaintext application/OIDC origins only for explicit development or disposable tests; Helm maps `gateway.insecureDevelopmentHttp` to this and defaults false |
| SYNVEDA_TLS_MODE | reference currently supports files |
| SYNVEDA_OIDC_ISSUER | exact external issuer URL |
| SYNVEDA_OIDC_ISSUERS_FILE | mounted provider-neutral issuer document |
| SYNVEDA_DATABASE_ROLES_FILE | mounted database role contract |
| SYNVEDA_DATABASE_EXPECTED_HOST | external PostgreSQL DNS authority asserted by preflight |
| SYNVEDA_DATABASE_EXPECTED_PORT | external PostgreSQL canonical TCP port asserted by preflight |
| SYNVEDA_DATABASE_EXPECTED_NAME | external PostgreSQL database asserted by preflight |
| SYNVEDA_BOOTSTRAP_TENANT_ID | UUIDv7 bound into backup and required unchanged at restore |
| SYNVEDA_COMPOSE_IPV4_POOL | explicit private /24 for reference/evidence |
| SYNVEDA_PRODUCT_IMAGE | immutable product image reference |
| SYNVEDA_PRODUCT_STARTING_IMAGE | upgrade-smoke-only immutable starting product image reference |
| SYNVEDA_POSTGRES_IMAGE | immutable bundled PostgreSQL image reference |
| SYNVEDA_KEYCLOAK_IMAGE | immutable bundled Keycloak image reference |
| SYNVEDA_CADDY_IMAGE | immutable proxy image reference |
| SYNVEDA_OTEL_COLLECTOR_IMAGE | immutable Collector image reference |
| SYNVEDA_PROMETHEUS_IMAGE | exact digest-pinned optional Prometheus image |
| SYNVEDA_PROMETHEUS_PORT | optional loopback operator port |
| SYNVEDA_OTLP_EXPORT_ENDPOINT | non-secret external OTLP/gRPC DNS authority and port |
| SYNVEDA_WORKER_ALLOW_NON_LOOPBACK_HEALTH | false by default; exact true permits only an unspecified worker health bind for a deployment-owned private network |
| OTEL_EXPORTER_OTLP_ENDPOINT | OTLP/gRPC destination used by application processes |
| SYNVEDA_BACKUP_ID | optional explicit immutable logical-backup identifier |
| SYNVEDA_RESTORE_SOURCE_PROJECT | exact source project recorded by the recovery set |
| SYNVEDA_CONFIRM_RESTORE | exact source-project:backup-id:target-project approval |
| SYNVEDA_DATABASE_BACKUP_ROOT | absolute private database-archive root |
| SYNVEDA_RECOVERY_SECRETS_ROOT | separate absolute private recovery-secret root |

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
- apalis_owner_password and apalis_runtime_password
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

External PostgreSQL uses the same three role-URL and two KMS files plus
`postgres_root_ca`. Its secret directory and issuer document are prepared by
the operator; the bundled generator refuses this mode. The mode-0700 secret
directory is scoped to the exact Compose project and contains the required
mode-0700 `oidc-directory` child. The separately mounted issuer file has a
private parent and may not be nested under that secret directory. Every role
URL must use the asserted DNS host, port and database, exactly one
`sslmode=verify-full`, and exactly
`sslrootcert=/run/secrets/postgres_root_ca`. Client certificates, `hostaddr`,
socket routing and libpq TLS aliases are outside this contract.

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

The postgres-data named volume is the bundled persistent database state. The
optional apalis-data volume holds disposable transport state only; the Synveda
operation/outbox rows remain authoritative and this queue volume is not part of
logical recovery. Selecting the native provider is the rollback if the queue is
lost or disabled. The isolated Apalis bootstrap owner remains the queue
cluster's superuser and is mounted only into PostgreSQL and the one-shot
migrator; the long-running Apalis worker receives only its converged runtime
role password and the normal Synveda worker DSN. The
optional prometheus-data volume is disposable operational history, bounded to
72-hour and 1-GB TSDB block-retention thresholds (whichever triggers first).
Those thresholds are not a volume quota because WAL, head-block and compaction
data can use additional space; the volume is not part of product recovery.
Issuer projection, database authority and public realm gates are bounded
operator-owned runtime files, not independent data stores. The Synveda KMS key
is separate recovery material and must be protected with the database backup.

Logical recovery writes two separate operator-owned roots. Each backup ID has
a mode-0700 database-archive directory containing `synveda.dump`,
`keycloak.dump` and `manifest.json`, and a mode-0700 recovery-secret directory
containing `synveda_kms_key`, `synveda_kms_key_ref`,
`keycloak_convergence_admin_password` and its linked `manifest.json`. These
are sensitive persistent operator data, not container volumes or image
contents.

Migration ownership remains separate: Synveda runs migrate; Keycloak owns its
schema lifecycle. Keycloak realm export is not a database backup.

In external-PostgreSQL mode, the provider/operator supplies PostgreSQL 17,
database `synveda`, `btree_gin` 1.3 and `vector` 0.8.6 in `public`, the declared
database owner, NOLOGIN capability role `synveda_app`, and the least-privilege
migrator, gateway and worker logins with the exact memberships, ownership and
ACL shape required by the role contract. The bounded `database-preflight`
checks the declared endpoint, role contract and TLS settings before migration,
gateway or worker startup. The CA is added to the native TLS trust calculation;
this is hostname-and-chain verification, not exclusive certificate pinning.
The current SQLx native-TLS path is documented for one PEM root certificate
until a larger bundle is live-proven. External backup, restore, reset and
bundled-Keycloak database bootstrap remain refused.

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

The experimental Apalis 0.7.4 canary implements only non-executing
`skill_validation@1`. The operation, attempt and outbox rows are tenant-bound
under forced RLS and the operation/outbox commit is transactional. The queue
payload is exactly the untrusted tenant routing hint, Synveda operation ID and
operation version; the worker re-resolves and re-authorises the immutable
tenant/Skill association in a tenant transaction before writing through the
normal executor. Stale acknowledged delivery is resubmitted after a bounded
timeout, while operation leases and result linkage fence duplicate effects.
The default PostgreSQL execution path remains the rollback. Apalis task IDs,
status vocabulary and SQL types stay in the leaf adapter and do not enter core
crates or the public API.
The exact stable crates are `apalis` 0.7.4 (MIT OR Apache-2.0) and
`apalis-sql` 0.7.4 (MIT). They declare no crate MSRV; Synveda validates them
under its pinned Rust 1.96 toolchain and Cargo Deny policy.

Temporal has no executable consumer and is not part of this deployment.

The customer-safe `/console/operations` route is an application view, not an
infrastructure or identity-administration surface. For the selected project it
makes four independent, bounded calls through the generated public API: recent
durable operations, Sessions, context runs and Capture batches. Each call
retains its own PDP/RLS, loading, empty and failure semantics. The view renders
only safe lifecycle fields, progress and content-free error codes, omits
separately protected candidate counts and queue-provider identifiers, labels
its snapshot as potentially stale and states that dependency/worker heartbeat,
latency/token, Knowledge/index, Skill/MCP, backup and provider-health aggregates
are not available rather than inferring them from probes or policy-filtered
rows.

## Telemetry

Application processes emit traces through OTLP to the private Collector. The
Collector applies memory limiting and batching. Discard mode terminates traces
at a no-op exporter. External mode sends traces over TLS to one validated
DNS-authority endpoint using the Collector image's public CA trust and a
bounded in-memory queue/retry window. It does not yet support a private CA,
mTLS or an authentication header, and Collector readiness does not prove that
the remote backend received a trace. With the optional observability profile,
the Collector
also scrapes the private gateway and worker metrics endpoints and exposes one
private fan-in target to Prometheus. Smoke requires gateway authority ready,
worker ready and a fresh worker heartbeat from samples newer than the smoke
start. Prometheus applies 72-hour and 1-GB TSDB block-retention thresholds,
whichever triggers first, and exposes only a loopback operator UI. This is a
block-retention policy, not a disk quota.

The external exporter carries traces only; local application metrics are not
forwarded. The local backend is infrastructure visibility, not the tenant-safe
Operations product. No prompt, message, Knowledge body, credential or unbounded
tenant/user label may enter telemetry.

## Backup and restore

The bundled-provider reference implements one operator-invoked logical
recovery path. `compose-backup` verifies the running graph, pauses the gateway,
worker and Keycloak writers, then uses the PostgreSQL 17 tools paired with the
server to create custom-format archives of the `synveda` and `keycloak`
databases. Synveda is dumped through the bounded cluster-owner recovery
identity because forced RLS prevents an ordinary runtime role from producing a
complete archive; Keycloak is dumped through its dedicated database owner.
The writers are resumed and rechecked before success is reported.

The database pair and separately stored KMS key, KMS reference and surviving
Keycloak convergence credential are SHA-256 linked and never overwritten.
`compose-restore-smoke` requires an exact
`source-project:backup-id:fresh-target-project` confirmation. It verifies the
pair, restores it into a new private PostgreSQL volume and network, reconverges
the database authorities, then verifies the restored tenant and complete audit
chain through the ordinary gateway database role before normal tenant
convergence can create product state. It opens the current tenant data key with
the recovered KMS key, proves that a synthetic wrong key receives the exact
authenticated-decryption refusal, converges the normal identity/application
graph and runs browser acceptance. The target is left private and running for
inspection.

This is a planned-interruption, same-PostgreSQL-17 logical recovery check. The
restore smoke deliberately requires the exact PostgreSQL image reference
recorded by backup; it is not a general archive-compatibility promise. The
two database dumps are not one cross-database transaction, so application
writers are paused; independently connected writers are outside that pause.
Logical archives and recovery secrets are sensitive and are not encrypted by
this tool. Their hashes detect accidental change but are not signatures or an
authenticated manifest. The check opens the restored tenant data key; it does
not claim that Knowledge bodies are application-encrypted.

WAL archive/PITR, S3-compatible transfer, scheduling, retention, off-host
custody, recurring drills and owned RPO/RTO remain open production work. A
same-host copy is recovery-validation evidence, not disaster recovery.
The experimental Apalis profile is refused for backup and restore: these
commands protect authoritative product and identity data, not disposable queue
transport state.

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
    make compose-backup
    make compose-restore-smoke
    make compose-upgrade-smoke
    make compose-smoke
    make compose-restart-gateway
    make compose-down
    make compose-reset

compose-acceptance requires a fresh suffixed bundled project and the exact
demo/browser profiles, optionally plus Apalis. Under one lock and deadline it
performs browser login, seeds the existing two-principal public-API team demo,
creates and polls one `skill_validation@1` operation, reopens and checks the
active receipt, then runs the fixed product/provider restart matrix and the full
smoke after every restart. The normal matrix has six services; selecting Apalis
adds its worker as a seventh restart while its queue remains running.
The gate repeats browser login and verifies the existing receipt against live
product rows. It leaves the successful stack running. Identity admission, the
operation and the demo rows are restart-state witnesses. With the same profiles
selected, compose-down removes the disposable browser credential-and-receipt
volume after exact ownership checks while retaining product data; confirmed
compose-reset also removes it and any selected Apalis transport volume.
Reset retains the project's secrets, issuer document and KMS key. Paired
logical backup/isolated restore are implemented for bundled PostgreSQL and
bundled Keycloak. External PostgreSQL plus external OIDC can start the same
product graph; its deterministic wiring tests do not constitute a live
provider result. External-provider recovery and upgrade smoke remain open.
Live targets must report an unavailable prerequisite distinctly from a passing
test.

compose-upgrade-smoke is the bounded reference-mode application-image check.
It requires an existing suffixed bundled project, the exact demo/browser
profiles and distinct digest-addressed starting and candidate product images.
The candidate first verifies the current epoch, embedded SQLx ledger and full
migrator authority in one read-only repeatable-read transaction. The lifecycle
then image-transitions only gateway and worker through candidate, starting
rollback and final candidate checkpoints, while rerunning the disposable
browser-acceptance service. Each checkpoint proves exact image reference
and image ID, public smoke, Keycloak browser login and the existing product
receipt. Ordinary failure restores the last fully verified image when that can
be proved; uncertain mutation retains the project lock. This does not migrate
schema or providers and is not general N-1, downgrade or zero-downtime
evidence.

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

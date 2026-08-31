# Synveda deployment contract

Status: **accepted architecture; implementation open under CPR-45**. This file
defines the contract the Docker reference, direct binaries and later Helm must
implement. Until `CPR-45` closes with executable evidence, the existing install
instructions describe the currently shipped profile and this document is not a
claim that the reference topology has passed.

## Contract principles

- There is one Synveda application, schema, public API and governed
  Configuration model. Deployment shape and provider do not select product
  editions or domain behavior.
- Every application read/write still crosses Cedar; forced RLS remains the
  tenant backstop; governed mutations use VedaFlow and content-free audit.
- PostgreSQL, OIDC/OAuth 2.0, S3-compatible backup storage, OTLP and OCI are
  the provider interfaces. Cloud product names do not enter domain crates or
  public DTOs.
- Images and commands are immutable inputs recorded with source and deployment
  digests. A tag alone is not reference acceptance evidence.
- A health result states only what it tests. Downstream telemetry or optional
  services never make the application unready.

## Compose file sets

The canonical directory is `deploy/compose/`. Runtime-mode overlays and
dependency-provider fragments are orthogonal:

```text
compose.yaml             provider-neutral proxy, product, migration and Collector graph
compose.reference.yaml   HTTPS, resource bounds and reference restart policy
compose.dev.yaml         source builds, explicit HTTP and loopback operator UI
compose.postgres.dev.yaml local PostgreSQL image build, development only
compose.keycloak.dev.yaml local optimized Keycloak and database-bootstrap helper builds, development only
compose.postgres.yaml    bundled PostgreSQL and idempotent role/database bootstrap
compose.keycloak.yaml    bundled Keycloak, isolated database bootstrap and realm convergence
compose.keycloak-postgres.yaml             bundled shared-cluster ordering/secret bridge
compose.external-postgres.yaml             external Synveda database egress bridge
compose.keycloak-external-postgres.yaml    external Keycloak database egress bridge
compose.external.yaml    external-provider labels, no provider services
compose.apalis.yaml      planned experimental routing plus dispatcher/executor; not yet present
compose.db-test.yaml     isolated database acceptance fixture, not an operator topology
docker-compose.yml       legacy Rauthy/Temporal development stack retained only until cutover acceptance
.env.example             non-secret selectors and hostnames only
configs/                 proxy, Collector, identity and monitoring config
secrets.example/         filenames and generation instructions, never values
scripts/                 bounded bootstrap, validation and acceptance helpers
README.md                exact file-set commands and limitations
```

Exactly one runtime overlay is required: `compose.reference.yaml` or
`compose.dev.yaml`. They are mutually exclusive; a validation script rejects
their simultaneous use. Provider fragments are then selected independently:

| Mode | Required file set after `compose.yaml` |
|---|---|
| bundled reference | `compose.reference.yaml`, `compose.postgres.yaml`, `compose.keycloak.yaml`, `compose.keycloak-postgres.yaml` |
| bundled development | `compose.dev.yaml`, `compose.postgres.dev.yaml`, `compose.keycloak.dev.yaml`, `compose.postgres.yaml`, `compose.keycloak.yaml`, `compose.keycloak-postgres.yaml` |
| bundled PostgreSQL, external OIDC | one runtime overlay, `compose.postgres.yaml`, `compose.external.yaml` |
| external PostgreSQL, bundled Keycloak (configuration only) | one runtime overlay, `compose.keycloak.yaml`, `compose.keycloak-external-postgres.yaml`, `compose.external-postgres.yaml`, `compose.external.yaml` |
| fully external (configuration only while PostgreSQL is external) | one runtime overlay, `compose.external-postgres.yaml`, `compose.external.yaml` |

For every development row, selection inserts `compose.postgres.dev.yaml`
before the provider fragments when PostgreSQL is bundled and
`compose.keycloak.dev.yaml` when OIDC is bundled. Reference rows select no
source-build fragment and require digest-addressed images.

Base `compose.yaml` owns product mounts, database preflight and issuer
diagnostics. `compose.external.yaml` adds provider labels only and never defines
a service named `postgres` or `keycloak`; the two external-PostgreSQL bridges
attach only the required egress. External PostgreSQL rows currently prove
configuration shape only.
The database bootstrap refuses before mounted-input reads or `psql` whenever
`SYNVEDA_POSTGRES_BUNDLED_CLUSTER=false`: the compiled SQLx surface and
bootstrap transport do not yet have an accepted authenticated-TLS contract.
Consequently bundled Keycloak with external PostgreSQL and the fully external
PostgreSQL row are not startable product modes. Their checked configuration
still requires an explicit topology-specific database-role contract so the
future implementation cannot infer or silently omit peer databases. Fully
external OIDC omits Keycloak and its database bootstrap entirely.

Optional services use only these profiles: `semantic`, `observability`,
`apalis-board`, `demo` and `backup-test`. Apalis execution is activated by the
explicit `compose.apalis.yaml` fragment, not a profile: that fragment atomically
changes the one per-kind routing key and starts its dispatcher/executor. The
configuration gate rejects a routed kind without its services or those
services without matching routing. The proxy, gateway, core worker, migration,
identity diagnostic and Collector are never profile-gated.
Bundled PostgreSQL and Keycloak are likewise unprofiled when their fragments
are selected. The repository requires Docker Compose 2.33.1 or newer for
deterministic `gw_priority` handling and tests
every row of the table with `docker compose config`, the expected service set,
ports, secret mounts and dependency graph. Reference and development use fixed
project names (`synveda-reference` and `synveda-development`); acceptance runs
use a bounded, validated suffix to isolate resources.

Merge semantics are intentionally simple: base declares no host ports or
provider service-name dependencies; each runtime overlay owns published ports
and lifecycle/resource settings for base services only; and each provider
fragment owns its provider's common non-root/security/restart/resource policy
plus health dependencies. Provider policy is deliberately identical in
development and reference so runtime overlays never declare service stubs that
would accidentally create bundled services in external mode. No override
relies on list append order.

## Images and commands

Every release environment manifest records the source SHA, deployment-file
digest, OCI index digest and accepted platform digests.

| Image role | Required contents | Commands |
|---|---|---|
| Synveda product | Current: `synveda`, `synveda-gateway`, `synveda-worker`, `synveda-oidc-diagnostic`; console static bundle and embedded policies. Planned CPR-45: experimental Apalis adapter, disabled by default | Current launcher roles: `gateway`; `worker`; `issuer-diagnostic`; `database-preflight`; `migrate`; `tenant-converge`; `probe gateway\|worker live\|ready`. Planned, not yet shipped: `operation-dispatcher`; `apalis-worker` |
| PostgreSQL | PostgreSQL 17, pgvector, `btree_gin`, pinned pgBackRest | server entrypoint; database/role bootstrap; `synveda-backup` archive/check/create/verify/expire/restore commands |
| Keycloak | Official Keycloak 26.7.2 optimized build with PostgreSQL, health and metrics; no added provider/package; reviewed standalone convergence/readback helpers | `kc.sh start --optimized` through a secret-file entrypoint; long-running `synveda-realm-supervise` controller with a bounded generation-fenced `synveda-realm-converge` child |
| Reverse proxy | Pinned Apache-2.0 Caddy release and reviewed configuration | `caddy run --config /etc/caddy/Caddyfile --adapter caddyfile` |
| Telemetry | Pinned OTel Collector Contrib | `otelcol-contrib --config=/etc/otelcol/config.yaml` |
| Optional visibility | Prometheus, Jaeger and Perses at reviewed digests | upstream commands with bounded storage/retention |
| Experimental executor | Exact same Synveda product image digest; adapter-only `apalis`/`apalis-sql` 0.7.4 dependency | `synveda-container operation-dispatcher`; `synveda-container apalis-worker` |

The product image has a role-neutral
`ENTRYPOINT ["/usr/local/bin/synveda-container"]` and defaults to `gateway`.
The entrypoint validates a closed command vocabulary and immediately `exec`s
the selected binary; it does not interpret deployment type or print secrets.
`worker` owns the current core maintenance/capture/indexing work,
`operation-dispatcher` submits only outbox rows routed to Apalis, and
`apalis-worker` executes only the declared experimental operation kind. Image
health is not hard-coded to the gateway: Compose and later Kubernetes attach a
role-specific probe to each service.

`tenant-converge` delegates to the provider-neutral `synveda tenant converge`
command. It admits one exact UUIDv7/slug/name tuple through the existing
migrator-authority, forced-RLS and `tenant.created` audit path, is idempotent
only for that exact active tenant, and requires encryption-key provisioning to
succeed. An existing wrapped key is not sufficient: every convergence proves
current-key unwrap custody, reads the authoritative generation-one KEK
reference, and serializes one exact content-free provision witness on the
tenant audit-chain head. The audit crate exposes this as a key-provision-only
repair operation rather than a generic idempotent append, so ordinary governed
mutations retain the same-transaction audit rule. The command is an image
contract; the canonical topology does not yet select a bootstrap job, so it is
preparation rather than install evidence.

Capture remains its existing durable aggregate rather than becoming an Apalis
task. Its current claim is fenced by tenant, batch, process-unique owner and
attempt number. The core worker renews the lease independently during provider
calls, re-proves it after preflight before the first provider disclosure,
abandons results after renewal loss, bounds renewal shutdown, makes the guarded
terminal transition before candidate writes, and audits an expired exhausted
attempt. This row-level safety does not by itself prove process drain or
multi-worker availability; those claims require the worker lifecycle
acceptance below.

Stateless images run with the validated numeric
`SYNVEDA_RUNTIME_UID:SYNVEDA_RUNTIME_GID` recorded in the environment manifest;
the Compose selector defaults these values to the operator's current ids,
refuses zero and verifies that private directories and mode-0600 files have
that exact owner. The secret generator must therefore run as the same non-root
operator that renders the deployment. This lets bind-mounted files remain
readable without relying on Compose's unimplemented local-secret uid/gid/mode
remapping. The product image remains compatible with an arbitrary non-root UID
for later OpenShift packaging.
Runtime roots are read-only with explicit `/tmp` tmpfs/writable data mounts.
Base image tags may appear only beside verified digests in build arguments or
inventories. Apalis does not create a second product image or deployment
contract.

## Services and ports

Only the reverse proxy publishes host ports in reference mode.

| Service | Container port | Exposure | Health/readiness |
|---|---:|---|---|
| reverse proxy | selected development port (default 8080); 80, 443 reference | public reference; loopback-only development | process/config health; upstream probes are separate |
| gateway | 8120 | private application network | current `/healthz` process and `/readyz` PostgreSQL + schema epoch; reference target adds drain-state withdrawal before shutdown |
| worker | 8121 | bound to container loopback for self-health only; not host/network published | `/healthz` process; `/readyz` lifecycle, scheduler heartbeat, database/schema and exact runtime-role state; permanent epoch/role drift faults the supervised process; `/metrics` is transitional |
| PostgreSQL | 5432 | private data networks | `pg_isready` plus schema/role sentinels |
| Keycloak frontend | 8080 | not host-published; reverse proxy is the only configured public route | public flow is probed through proxy |
| Keycloak management | 9000 | not host-published or routed; reachable only by private network peers | `/health/started`, `/health/live`, `/health/ready`; metrics private |
| OTel Collector | 4317, 4318 | private telemetry network | private Collector health extension/internal telemetry |
| TEI (`semantic`) | 80 | private semantic network | upstream `/health` after model load |
| Prometheus | 9090 | loopback/operator route only | upstream readiness |
| Jaeger UI | 16686 | loopback/operator route only | upstream health |
| Perses | 8080 | loopback/operator route only | upstream health |
| Apalis board | implementation-defined | operator-only, disabled by default | never a customer/public route |

Development binds explicit HTTP only to loopback. Its validated unprivileged
port from 1024 through 65535, excluding Caddy's reserved development HTTPS
convention port 8443, is identical inside and outside the proxy so the browser,
issuer diagnostic and gateway use one exact issuer authority; a host-only port
translation is invalid. Reference/playground binds 80/443 and
requires configured DNS and HTTPS. Certificate-file mode is the
only current render; ACME remains an unimplemented target mode. PostgreSQL,
Keycloak management/admin/master realm, worker health, receivers, metrics,
dashboards, board and backup operations are never public. pgBackRest is
embedded in the PostgreSQL image for POSIX/S3 repository operation; the
reference opens no pgBackRest daemon port.

## Service graph and lifecycle

The provider-neutral target graph is deterministic:

```text
bundled postgres healthy -> database bootstrap complete -> Synveda migrate complete
external postgres diagnostic ---------------------------> Synveda migrate complete
Synveda migrate complete -> core worker healthy

bundled Keycloak database sentinel -> Keycloak ready -> realm convergence
realm convergence -> reverse-proxy config healthy -> bundled issuer diagnostic
external reverse-proxy config healthy + external issuer over application-egress
                                         -> external issuer diagnostic
Synveda migrate complete + issuer diagnostic complete -> gateway ready -> usable app route

Apalis schema migration complete -> operation dispatcher + Apalis executor
product processes and Keycloak -> Collector -> optional visibility/external OTLP
```

The current additive checkpoint implements the bundled database bootstrap,
fail-closed realm convergence and the issuer-diagnostic service contract. The
canonical selector remains configuration-only, so this graph is not yet
clean-start acceptance evidence. Its external-PostgreSQL bootstrap path
deliberately stops before runtime startup until authenticated TLS and
pre-provisioned-provider acceptance are implemented.

The issuer diagnostic retries retryable OIDC availability through one bounded
deadline without naming a provider. Database preflight and migration have
bounded operations but do not poll their dependency. Provider fragments add
`depends_on` health/completion conditions when the dependency is bundled.
Gateway is not started until migration and issuer diagnostics succeed. In
bundled-Keycloak mode the proxy starts only after convergence; in external-OIDC
mode it may start immediately. Proxy health proves only that Caddy accepted the
closed route configuration. The product issuer diagnostic, not proxy health,
proves discovery, exact issuer and usable JWKS. Bundled mode reaches the
browser-visible issuer through the proxy alias; external mode reaches the
provider directly over `application-egress` and has no Synveda identity vhost.
The application route can remain unavailable until gateway readiness. Collector
or visibility failure never gates product readiness.

Reference long-running services, including the Keycloak convergence
supervisor, use `restart: unless-stopped`; one-shot migration, database
bootstrap and issuer-diagnostic jobs use `restart: "no"` and idempotent
locks/sentinels. Backup jobs will use the same one-shot policy when that slice
lands. Development defaults to `restart: "no"` so failures remain visible.
Every long-running process has init/signal forwarding, bounded stop grace,
PID/resource limits and a tested drain path. The core worker and Apalis
executor are mutually exclusive for the `skill_validation@1` operation route:
`SYNVEDA_OPERATION_PROVIDER_SKILL_VALIDATION=postgres` is the default;
`compose.apalis.yaml` changes that single value to `apalis` and starts the
dispatcher/executor in the same rendered project. The core worker never claims
an operation routed to Apalis.

The existing synchronous `POST
/v1/skills/{id}/versions/{version_id}/tests` contract remains unchanged as the
experiment's control and rollback. The new provider-neutral operation endpoint
records operation/outbox state and uses either the core PostgreSQL worker or
Apalis delivery; both call the same extracted validation function. Removing
`compose.apalis.yaml` restores PostgreSQL delivery, while the existing
synchronous route remains usable throughout the experiment.

## Container security baseline

Every reference container, including bootstrap, migration, convergence,
backup and restore jobs, uses a non-root UID where its upstream image supports
one, `cap_drop: [ALL]`, `security_opt:
[no-new-privileges:true]`, a read-only root filesystem, `init: true`, a tmpfs
for required transient paths, bounded memory/CPU/PIDs, health checks and
explicit networks. One-shot jobs omit health checks where successful exit is
their health contract. The only permitted root-at-start exceptions are the
official PostgreSQL ownership transition during first initialization and a
no-network volume-initialisation job for exact named volumes. Each drops all
capabilities then adds only the proved `CHOWN`, `DAC_OVERRIDE`, `FOWNER`,
`SETGID` and/or `SETUID` subset it needs, has a read-only root and bounded
resources, and exits before application work starts. Its final server process
and every application process run non-root with no effective capability.

The topology forbids privileged containers, Docker socket mounts, host network
or PID/IPC namespaces, host devices, broad host-directory mounts and owner
database credentials in application containers. Volumes and networks carry
Compose project/contract labels. Reset and restore scripts enumerate resources
with those labels, show the exact target set, and require
`SYNVEDA_CONFIRM_RESET=<project-name>` or
`SYNVEDA_CONFIRM_RESTORE=<restore-project-name>`; they never resolve a broad
path or unbounded project prefix.

## Networks

- `public-edge`: reverse proxy only, plus explicit public ingress.
- `app-backend`: proxy, gateway and issuer diagnostic.
- `identity-backend`: proxy, Keycloak and the realm-convergence supervisor.
- `synveda-data`: PostgreSQL, Synveda database bootstrap/preflight/migration,
  gateway and worker; future backup jobs attach only when implemented.
- `keycloak-data`: PostgreSQL, Keycloak database bootstrap and Keycloak.
- `telemetry`: gateway, worker and Collector.
- `keycloak-management`: Keycloak, the realm-convergence supervisor and
  Collector; management proof/scrape traffic, with no published port.
- `application-egress`: issuer diagnostic, gateway and worker; external
  PostgreSQL mode also attaches database preflight and migration.
- `identity-egress`: only bundled-Keycloak database bootstrap and Keycloak in
  the explicit external-PostgreSQL row.
- `telemetry-egress`: Collector only.
- Optional isolated `semantic`, `operations` and `restore-test` networks.

Compose consumes one operator-facing `SYNVEDA_COMPOSE_IPV4_POOL=A.B.C.0/24`.
The selector accepts only a canonical RFC1918 `/24` and derives fixed `/28`
slots: identity `.0`, public `.16`, application `.32`, Synveda data `.48`,
Keycloak data `.64`, Keycloak management `.80`, telemetry `.96`, application
egress `.112`, identity egress `.128` and telemetry egress `.144`. The `.160`,
`.176` and `.192` slots are reserved for semantic, operations and restore-test;
three slots remain spare. Every rendered network has an explicit subnet and
gateway. The intended public or provider-facing attachment has
`gw_priority: 1` on each multi-homed service; ordinary network `priority` is
not a default-route control.

Identity uses gateway `+1`, the proxy's sole static address `+2`, and the upper
`+8/29` dynamic range. This keeps Keycloak and convergence allocations away
from the trusted proxy address without marking that same address as an IPAM
auxiliary address. No other service has a static container address.

The baked pool is an unsuffixed development convenience, not a collision-free
allocation. Reference and acceptance-suffixed projects require an explicit
pool, and concurrent or retained projects require distinct recorded pools.
Configuration rendering stays daemon-independent. The future lifecycle `up`
boundary must first compare numeric CIDR intervals against all Engine networks,
accept only exact label-owned same-project reruns, and refuse stale or foreign
overlaps without deleting anything; Docker network creation remains the final
race authority. Operators must also avoid host/VPN and external-dependency
routes. Docker Desktop keeps bridge routes inside its VM, so host-route
inspection alone is insufficient and functional dependency smoke tests remain
required.

PostgreSQL is the only service joining both data networks in the shared-server
reference. Gateway and Keycloak never share a data network. Docker networks
are service-level, not per-port ACLs: a peer on a Keycloak network can reach
any Keycloak listener, so privacy claims mean no host publication and no proxy
route; configuration/inspection tests assert both. The proxy owns the
public application and issuer aliases on the networks that need them so
gateway and browser use identical issuer bytes without routing the gateway
through host loopback. Caddy's administration endpoint is disabled. The current
certificate-file mode mounts only the selected certificate and private-key
files read-only. Future ACME mode requires a dedicated persistent
`caddy-data` volume and acceptance before it can be selected.
`internal: true` removes external container egress; it is not a security
boundary against an authorised host/Engine operator.

## Persistent data and ownership

| Data | Persistent unit | Owner/lifecycle |
|---|---|---|
| PostgreSQL cluster | named data volume | PostgreSQL OS identity; physical backup recovery unit |
| Synveda database/schema | database inside cluster | Synveda migration owner; runtime gateway/worker roles are non-owner `synveda_app` members |
| Keycloak database/schema | separate database inside cluster | Keycloak login owns its schema and migrations; no Synveda membership |
| Experimental Apalis tables | separate `synveda_jobs` database | one-shot Apalis migrator; dispatcher/worker get narrower runtime roles |
| Backup repository | named POSIX volume or external S3-compatible repository | pgBackRest/operator identity; no application container credential |
| Synveda KEK/key reference | mounted secret outside PostgreSQL | deployment operator; backed up and restored separately |
| Keycloak realm/client/group | Keycloak database | converged through supported Admin API; export is not backup |
| Optional TEI cache | named model cache | derived and replaceable |
| Optional metrics/traces | bounded named volumes or memory | evaluation only; never system of record |

Database bootstrap revokes default `PUBLIC CONNECT` and `TEMPORARY`, grants
only intended roles, and tests connection denial in both directions. Owner,
superuser and `BYPASSRLS` credentials never reach gateway or worker.
Before any persistent mutation, bootstrap also proves that the PostgreSQL
owner, Synveda migrator, gateway and worker passwords are pairwise distinct.
When bundled Keycloak shares that server, the same locked Synveda preflight
includes the Keycloak database password and proves all five values are
pairwise distinct; the Keycloak branch independently rechecks owner versus
Keycloak. Refusals are content-free and external-OIDC mode does not mount a
Keycloak credential.
The closed role contract names both `forbidden_databases` (Synveda roles may
not connect to them) and `isolated_peer_roles` (those roles may not connect
to the selected Synveda database), so an inherited grant cannot silently
reopen either edge.

## Migration contract

1. PostgreSQL initialization creates databases and narrowly scoped roles from
   secret files. CloudNativePG leaves its application database in the exact
   closed/null-ACL bootstrap state; the shared convergence transaction
   publishes the explicit terminal ACL before reopening it. Initialization
   does not run application requests.
2. `synveda db migrate` runs once under the Synveda migration owner, protected
   by the existing advisory lock and epoch checks. The current embedded chain
   is exactly the transactional epoch-3 baseline
   `0001_context_platform.sql`. If SQLx commits that baseline and its exact
   checksum but the process exits before the separate marker transaction,
   restart recognises only the empty marker plus exact sole success ledger,
   repeats the full authority proof and stamps it. Every partial, additional,
   failed or checksum-drifted state remains refused.
3. Gateway and worker start only after the schema sentinel is readable through
   their ordinary runtime roles. Bootstrap convergence connects through each
   configured runtime credential and refuses read-only targets, different
   PostgreSQL cluster identities, database OIDs or live postmaster start
   markers; a valid epoch alone does not bind one deployment to one live
   writable database primary. Bundled Keycloak bootstrap atomically publishes
   a content-free, mode-0600 witness containing its database OID, cluster
   system identifier and postmaster start. Database preflight reads that same
   authority directory read-only for all three Synveda credentials. An
   ordinary PostgreSQL restart makes the witness stale and blocks startup
   until idempotent Keycloak bootstrap republishes it.
4. Keycloak owns and automatically migrates only its database. Upgrade first
   takes a verified backup and follows the pinned release's supported path.
5. Apalis schema setup is a one-shot migration command; workers never migrate
   on boot.
6. Backup tools operate as PostgreSQL/operator identities, never an application
   database role.

Schema epoch 1, epoch 2 and markerless historical databases remain refused
with reset guidance. This deployment contract adds no old-data translator.

## Configuration and secret files

Non-secret selectors may be direct environment values. A sensitive setting
uses `NAME_FILE` with a mounted file. Where a direct form remains for direct
binary compatibility, setting both forms is a startup error. Files must be
regular, non-symlinked where the platform can prove it, non-empty, bounded and
readable only by the target service. Diagnostics name the setting/path but
never its value.

Compose always uses the file form for sensitive values. Within a container the
target filename is stable even when the Compose secret object is role-specific;
for example, gateway's `synveda_gateway_database_url` and worker's
`synveda_worker_database_url` are each mounted as `/run/secrets/database_url`
only in their respective service.

Local Compose secret sources are generated mode `0600`, owned by the validated
non-root operator UID/GID. Stateless services run with those same numeric ids;
PostgreSQL's native entrypoint reads its own password before dropping to the
upstream database UID. The Compose files do not claim `uid`, `gid` or `mode`
remapping for file-backed secrets. Linux and Docker Desktop acceptance compares
host and in-container ownership/readability and proves that another service,
UID and unmounted path cannot read the sentinel. An external secret manager may
materialise the same per-service paths, but does not change setting meaning.

| Concern | Direct/non-secret key | File key and target |
|---|---|---|
| public/listen URL | `SYNVEDA_PUBLIC_URL`, `SYNVEDA_LISTEN_ADDR` | none |
| process database | `DATABASE_URL` for direct binaries only | `DATABASE_URL_FILE=/run/secrets/database_url` |
| Apalis queue database | non-secret pool/concurrency settings | `SYNVEDA_APALIS_DATABASE_URL_FILE=/run/secrets/apalis_database_url` |
| issuer set | issuer file path only | `SYNVEDA_OIDC_ISSUERS_FILE=/etc/synveda/oidc/issuers.json` (sensitive read-only config; credential values forbidden) |
| issuer directory credential | path reference inside the selected issuer entry | Entra `client_secret_file` or Okta `api_token_file`, each below `/run/secrets/oidc_directory/` |
| local KMS | `SYNVEDA_KMS_PROVIDER=local` | `SYNVEDA_KMS_KEY_FILE=/run/secrets/kms_key`; `SYNVEDA_KMS_KEY_REF_FILE=/run/secrets/kms_key_ref` |
| extraction | `SYNVEDA_EXTRACTOR=deterministic|claude|vllm`, `SYNVEDA_EXTRACTOR_MODEL`, `SYNVEDA_ANTHROPIC_BASE_URL`, `SYNVEDA_VLLM_BASE_URL` | `ANTHROPIC_API_KEY_FILE=/run/secrets/anthropic_api_key`; current vLLM adapter has no credential setting |
| embeddings | `SYNVEDA_EMBEDDER=deterministic|tei`, `SYNVEDA_EMBEDDER_MODEL`, `SYNVEDA_TEI_URL` | none in the current TEI adapter |
| Skill-validation delivery | `SYNVEDA_OPERATION_PROVIDER_SKILL_VALIDATION=postgres|apalis` | queue database files; `apalis` is accepted only with the atomic Apalis Compose fragment |
| application OTLP | `OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4317` | none; the private hop is unauthenticated only inside `telemetry` |
| external OTLP | `SYNVEDA_EXTERNAL_OTLP_ENDPOINT` | `SYNVEDA_EXTERNAL_OTLP_CA_FILE=/run/secrets/otel_ca`; `SYNVEDA_EXTERNAL_OTLP_HEADERS_FILE=/run/secrets/otel_headers` |
| custom CA | reserved; not accepted by the current application schema | no supported application mount until OIDC/provider clients prove explicit CA loading |
| outbound proxy | ambient proxy variables are deliberately ignored by OIDC and CLI clients | explicit proxy-file settings remain reserved and rejected until a bounded consumer and no-proxy contract ship |
| object store | `SYNVEDA_OBJECT_STORE_ENDPOINT`, `SYNVEDA_OBJECT_STORE_REGION`, `SYNVEDA_OBJECT_STORE_BUCKET`, `SYNVEDA_OBJECT_STORE_PATH_STYLE` | `SYNVEDA_OBJECT_STORE_ACCESS_KEY_FILE=/run/secrets/object_store_access_key`; `SYNVEDA_OBJECT_STORE_SECRET_KEY_FILE=/run/secrets/object_store_secret_key`; `SYNVEDA_OBJECT_STORE_SESSION_TOKEN_FILE=/run/secrets/object_store_session_token`; rejected unless an accepted feature enables the interface |
| SMTP | reserved `SYNVEDA_SMTP_HOST`, `PORT`, `FROM` | reserved `SYNVEDA_SMTP_USERNAME_FILE` and `PASSWORD_FILE`; all are rejected until an accepted consumer exists |
| reference TLS | `SYNVEDA_TLS_MODE=files` currently; `acme` reserved and rejected, public hostnames | `SYNVEDA_TLS_CERT_FILE=/run/secrets/tls_cert`; `SYNVEDA_TLS_KEY_FILE=/run/secrets/tls_key` |
| Keycloak database | `KC_DB_URL`, `KC_DB_USERNAME` | `KC_DB_PASSWORD_FILE=/run/secrets/keycloak_database_password` |
| Keycloak bootstrap | none | `KC_BOOTSTRAP_ADMIN_USERNAME_FILE=/run/secrets/keycloak_admin_username`; `KC_BOOTSTRAP_ADMIN_PASSWORD_FILE=/run/secrets/keycloak_admin_password` |
| Keycloak convergence | direct password values are forbidden | `SYNVEDA_KEYCLOAK_CONVERGENCE_PASSWORD_FILE=/run/secrets/keycloak_convergence_admin_password` |
| PostgreSQL bootstrap | `POSTGRES_USER`, database/role names | `POSTGRES_PASSWORD_FILE=/run/secrets/postgres_owner_password`; exact role files `/run/secrets/synveda_migrator_password`, `/run/secrets/synveda_gateway_password`, `/run/secrets/synveda_worker_password`, `/run/secrets/keycloak_database_password`, `/run/secrets/apalis_migrator_password`, `/run/secrets/apalis_runtime_password` |
| backup | `SYNVEDA_BACKUP_REPOSITORY=posix|s3`, `SYNVEDA_BACKUP_POSIX_PATH`, `SYNVEDA_BACKUP_S3_ENDPOINT`, `SYNVEDA_BACKUP_S3_REGION`, `SYNVEDA_BACKUP_S3_BUCKET`, `SYNVEDA_BACKUP_S3_PATH_STYLE` | `SYNVEDA_BACKUP_REPOSITORY_KEY_FILE=/run/secrets/pgbackrest_repository_key`; `SYNVEDA_BACKUP_S3_ACCESS_KEY_FILE=/run/secrets/backup_s3_access_key`; `SYNVEDA_BACKUP_S3_SECRET_KEY_FILE=/run/secrets/backup_s3_secret_key`; `SYNVEDA_BACKUP_S3_SESSION_TOKEN_FILE=/run/secrets/backup_s3_session_token` |
| demo identities | non-secret usernames only under `demo` | `SYNVEDA_DEMO_ADMIN_PASSWORD_FILE=/run/secrets/demo_admin_password`; `SYNVEDA_DEMO_MEMBER_PASSWORD_FILE=/run/secrets/demo_member_password` |

Every Synveda PostgreSQL URL uses the `postgres` or `postgresql` scheme and
explicitly names its database in the path or effective `dbname` parameter; it
never inherits a target from `PGDATABASE`. Fragments and query keys not
understood by the pinned SQLx PostgreSQL driver are refused before the driver
parses them. This is both fail-closed configuration and a secrecy boundary:
SQLx 0.8.6 otherwise logs an ignored query key together with its value.
Diagnostics name only the application-owned setting and never the URL, parser
detail or query value.

Upstreams without native file support use a reviewed entrypoint that reads
only its allowlisted `/run/secrets` paths into the child environment, unsets
temporary shell variables and `exec`s the upstream process. It performs no
shell tracing and never prints a value. Credentials are forbidden in endpoint
URLs, Compose labels, health commands and environment manifests.

The target issuer schema hard-cuts the current deployment-level
`directory_sync.client_secret`/`api_token` values into the file references
listed above. `issuers.json` may describe provider, tenant binding and file
path, but parsing fails if it contains a credential value. Per-tenant encrypted
directory credential aggregates remain the preferred runtime source and are
unchanged.

Secret visibility is deny-by-default:

| Service/job | Sensitive files it may mount |
|---|---|
| PostgreSQL server | PostgreSQL owner password; pgBackRest repository/S3 credentials needed by `archive_command` |
| database bootstrap | PostgreSQL owner plus the fixed role-password set it converges; the bundled shared-cluster Synveda bootstrap additionally receives the Keycloak database password only to prove the complete set is pairwise distinct |
| database preflight | all three Synveda migrator/gateway/worker database URL files so it can prove their exact independent authorities; never the PostgreSQL owner password |
| Synveda migrate | Synveda migration-owner database URL |
| issuer diagnostic | issuer configuration only; credential values are forbidden in that sensitive mounted file |
| gateway | gateway runtime database URL, issuer config, Synveda KMS reference/key, only gateway-used provider credentials; application custom-CA loading is not yet supported |
| core worker | worker runtime database URL, Synveda KMS reference/key, only credentials required by its owned work |
| Keycloak | Keycloak database password and first-start bootstrap administrator files |
| Keycloak convergence | first-start bootstrap administrator files and the persistent scoped convergence-identity password; demo user passwords only under `demo` |
| operation dispatcher | Synveda dispatcher database URL and Apalis runtime database URL; no provider/KMS content secret |
| Apalis executor | Synveda operation-worker database URL and Apalis runtime database URL; only canary-required key material |
| Apalis migration | Apalis migration-owner database URL only |
| Collector | external OTLP CA/auth files only |
| reverse proxy | TLS certificate/private key or ACME account state, never application/database credentials |
| backup/restore | database backup operator and repository credentials; recovery verifier alone additionally mounts the restored Synveda KMS key |

No service receives the complete secret set. Owner, bootstrap, migration and
backup credentials are absent from gateway, core worker, dispatcher and Apalis
executor containers.

`.env.example` contains no password, token, key, confidential DSN or usable
demo credential. The generator uses OS entropy, `umask 077`, mode `0600`, and
prints filenames only. Replacement requires both `--force` and the exact
project confirmation; the old set is preserved as `previous-secrets` and this
is explicitly not a credential-rotation workflow.

## OIDC contract

- The issuer URL is an exact immutable value reachable from browser, gateway,
  CLI flow, discovery, JWKS, token exchange and callback handling.
- Development uses `app.synveda.test` and `auth.synveda.test` with explicit
  host mapping and Docker aliases. `.localhost` is not used between network
  namespaces: RFC 6761 section 6.3 reserves it for loopback in each resolver,
  so a gateway container may resolve its own loopback rather than the proxy.
  A Docker alias cannot be accepted as a portable override of that rule. The
  host/container resolver diagnostic proves the `.test` mapping on Linux and
  Docker Desktop. Reference/playground uses operator DNS and HTTPS.
- Authorization code flow, state, nonce and PKCE S256 are mandatory. Implicit
  and resource-owner password grants are disabled.
- Discovery must return the exact issuer, S256 support and an allowed signing
  algorithm. Synveda allow-lists RS256/384/512 and validates signature,
  issuer, audience, time claims, subject and configured tenant binding.
- Discovery cannot prove client-registration-specific token-endpoint
  behaviour. The diagnostic therefore does not infer public-client support
  from `token_endpoint_auth_methods_supported`; a real authorization-code
  exchange is required before a provider is claimed conforming.
- The public client has exact redirect URI/origin and an explicit
  `synveda-api` audience mapper. `sub` is mandatory. Email/name follow the current JIT
  validation contract. The configured group claim is optional for generic
  authentication: absence maps to an empty set and can never seed the first
  administrator.
- `synveda-admins` is a one-time, race-safe initial-administrator signal only.
  Later authority is a governed Synveda grant; provider roles never become
  application roles.
- The bundled identity route forwards only discovery; authorization, token,
  JWKS and logout endpoints; logout confirmation; realm login actions; realm
  account pages; and required `/resources/*` assets. Other realm paths,
  `/admin`, the master realm, health, metrics and management remain private.
- External OIDC mounts the same issuer schema and runs the same gateway image.
  It renders no Synveda identity host or bundled-provider route. Provider
  support is earned by the common conformance suite, not its name.

The bundled provider is realm `synveda` with public client `synveda`.
Standard flow and PKCE method S256 are enabled; direct-access grants, implicit
flow, client service accounts and client secrets are disabled. Its only
redirect is `${SYNVEDA_PUBLIC_URL}/auth/callback` and its only web origin is
the exact public origin. The CLI continues through the gateway's bounded
loopback handoff; the IdP never accepts a wildcard loopback redirect. The
`groups` protocol mapper emits the leaf name `synveda-admins`, which the
provider-neutral group parser recognises, and an access-token-only audience
mapper emits `synveda-api`. `synveda-admins` is a bootstrap group, not an
application-role catalogue. The current checkpoint does not create a target-
realm user: the planned `demo` profile has no user-convergence service yet, and
there is no accepted manual administration path. A deterministic first-user or
demo-user provisioning boundary is therefore required before clean-volume
browser-login acceptance.

Keycloak is built from the official 26.7.2 image with `--db=postgres`, health
and metrics, and no preview feature/provider. It runs only
`start --optimized`. Runtime sets a full fixed `KC_HOSTNAME`, private HTTP on
8080 behind the proxy, management port 9000, `KC_PROXY_HEADERS=xforwarded` and
an explicit `KC_PROXY_TRUSTED_ADDRESSES` containing only the proxy's derived
fixed identity-network address as one `/32`. The non-secret
`SYNVEDA_COMPOSE_IPV4_POOL` is validated before all network addresses are
derived; the identity dynamic range excludes the proxy and no other service may
declare a static address. The
reference limit is 2 GiB memory and its documented host minimum includes that
bound. The exact upstream manifest digest and each built platform digest are
release inputs; a version tag without captured digest cannot pass acceptance.

Reference realm defaults are a five-minute access token, 30-minute SSO idle,
eight-hour SSO maximum, refresh-token rotation with zero reuse, offline access
disabled for the public client, brute-force detection enabled, login/user
events retained for seven days, and admin events recorded without
representations. Development may explicitly relax TLS for its HTTP origin;
reference/playground requires external TLS. Changes to these values are
versioned convergence input, not mutable console folklore.

The public proxy allowlists the exact discovery, authorization, token, JWKS,
logout, logout-confirmation, login-action, account and static-resource paths
required by the login/account flow. It rejects every other realm path,
`/admin/*`, `/realms/master/*`, health, metrics and management paths. The
browser administration console has no host mapping or supported route in
reference mode. No interactive Keycloak administration surface or canonical
operator command is accepted yet; convergence owns the required realm state.
Any later operator route requires its own authenticated boundary. A long-lived
supervisor uses supported Admin APIs and mounted administrator files to launch
one bounded, idempotent convergence child for each Keycloak process generation,
verify the resulting managed projection and monitor management readiness. It
never treats realm export as backup.

The shared marker used by Caddy is a fail-closed startup/readiness sequencing
aid, not an authorization boundary against the Keycloak process: Keycloak and
the supervisor deliberately share its writer identity. Before every Keycloak
start, the entrypoint atomically selects a fresh empty generation and withdraws
the previous marker. A convergence child captures that exact generation and
may publish only while it remains current; the supervisor withdraws readiness
on startup, shutdown, failed convergence or lost management readiness and
retires old generations. Convergence validates the complete candidate file
before its atomic rename, so Caddy never observes an unvalidated or stale-child
publication candidate. A compromised identity service could still mint
accepted identities or alter this shared gate, so the mechanism is process-
generation sequencing and availability evidence, not authentication isolation.
Live restart and dependency-loss acceptance remain required.

Convergence retires the temporary bootstrap administrator by exact user ID and
retains one narrowly scoped master-realm identity for idempotent repair after
restart. Its direct role set is exactly the three management roles on the
master realm's `synveda-realm` client plus direct
`master-realm:view-users`. Keycloak expands that read-only audit concession to
the effective `view-users`, `query-users` and `query-groups` role closure.
The latter is a read-only self-audit concession: it can view all master-realm
user and group metadata, so convergence proves the complete master user
inventory is exactly the temporary bootstrap identity plus the permanent
identity before retirement, and exactly the permanent identity afterwards.
It also proves the permanent user's security-relevant identity projection
(ID, username, enabled state, email-verification state, empty attributes and
required actions, and no service-account or user-storage federation link), no
external-identity links visible through Keycloak's Admin API, one password
credential, no groups, no realm roles, exactly four direct client roles and
the exact six-role effective closure on every first, no-op and restart run.
Keycloak filters links whose identity-provider
alias no longer exists, so that API cannot disprove an orphaned provider-owned database row;
operator-side master-realm/database mutation is an explicit residual rather
than an exact-empty claim. Non-authoritative profile metadata outside the
enumerated projection is likewise not claimed to be absent.

The permanent identity has no supported public route. A short-lived
`admin-cli` OpenID direct grant is validated for exact issuer, subject,
audience, exact scope membership, bounded token timing and public-key
signature. The response `session_state` and access/ID `sid` values must be one
identical Keycloak 26.7.2 secure session identifier: exactly 24 unpadded
URL-safe Base64 characters derived from the provider's 18-byte session-ID
generator. User, client, role and credential resource identifiers remain
UUIDs. Target-realm access and the enumerated master user/credential reads must
succeed. Keycloak 26.7.2's default fine-grained-admin-v2 evaluator must refuse
the filtered `admin-cli` client query with 403, proving that the identity lacks
master client-list authority; master client-session statistics, realm/user
creation, self-update and role-mapping mutation must likewise return 403.
Keycloak deliberately permits every
master admin role to read a reduced master-realm representation and general
provider/feature/theme/crypto server metadata; those version-sensitive bodies
are neither fetched nor treated as an absence oracle. The management surface
remains private, the master realm remains dedicated, and this concession must
be reassessed before adding other master users or custom providers. The helper
checks the exact nine-field outer token contract, then decodes the returned
internal refresh JWT as a pinned shape/drift check. That check requires the
HS512/JWT header, `Refresh` type, default provider, exact issuer/client/session
bindings, and the seven-scope refresh set: the three visible response scopes
plus Keycloak's hidden default `web-origins`, `acr`, `roles` and `basic`
scopes. The fresh dedicated master realm has refresh rotation disabled, so the
exact ten-claim projection excludes `reuse_id`, `aud_x` and `sub`; any operator
or version drift fails closed. The check binds the 1800-second refresh lifetime
to the access token and correlates `exp - refresh_expires_in` to the bounded
request/response window. Its HS512 payload cannot be verified through the
public JWKS and decoded internal claims are never an authorisation input. The
helper then revokes the refresh token, proves refresh refusal, and
revokes any replacement refresh token returned by a contradictory success
response. Each failure is reduced to a closed, content-free stage label:
`token-envelope` ends before cleanup is possible, while `token-contract` and
`refresh-contract` run only inside the cleanup guard. Probe inputs exist only
in the Java child's command-scoped environment. Captured stdout content is
never read; stderr is read only after its exact 39-byte bound is proved and is
accepted only when it equals the fixed refusal line. Neither captured stream is
emitted. Normal cleanup verifies deletion, while a deletion failure maps to
`runtime-output` and the final convergence cleanup retries removal. A
session cleanup failure overrides the earlier proof stage. No custom proof
client exists. This is an explicit reference-deployment security surface, not
a Synveda application client or domain-authorisation mechanism. Cleanup is
established immediately after a bounded HTTP-200 JSON object yields one
syntactically usable refresh token and before either exact token contract.
An oversize, malformed, duplicate-key or unextractable response cannot supply a
safe token to revoke; server-side session expiry is the bounded residual for
that pre-extraction failure.

Each exact persisted `kcadm` password-session configuration is likewise
revoked and refresh-refused before overwrite or deletion. A closure failure
withdraws the readiness marker, attempts target-realm quarantine, deletes the
container-local credential material and requires server-side session expiry;
the convergence container tmpfs is not a durable recovery channel. A killed CLI can also
receive a server session before it persists the refresh token. Local file
inspection cannot close or disprove either residual, so acceptance does not
claim zero administrative sessions. The 210-second Compose stop grace exceeds
the 56-second maximum deferred foreground proof plus the 110-second worst-case
cleanup bound with explicit margin.

The common conformance suite proves discovery, byte-exact issuer, PKCE S256,
JWKS rotation, accepted algorithm, audience, callback and first-admin signal;
it rejects wrong issuer, wrong audience, disallowed algorithm,
expired/not-yet-valid tokens and untrusted forwarded headers. A valid token
with a missing/empty group claim authenticates as an ordinary identity but is
proved unable to seed administration. It repeats login after Keycloak, proxy
and gateway restarts. Provider-specific success cannot replace any negative
case.

## Reverse proxy and trust boundary

The proxy accepts only configured hosts; removes the standard `Forwarded`
header; overwrites `X-Forwarded-For`, `X-Forwarded-Host`,
`X-Forwarded-Proto` and `X-Forwarded-Port`; strips untrusted identity and
`X-Original-*` headers; and removes
untrusted `traceparent`, `tracestate`, `baggage`, B3, Jaeger and OpenTracing
variants. It bounds body size, header size, upstream timeout and idle lifetime.
Reference TLS currently uses certificate files; bundled mode requires both
application and identity hostnames in the certificate SAN set, while external
OIDC requires only the application hostname. ACME, secure-header public-route
acceptance and cookie-origin browser acceptance remain open. Proxy
configuration never turns a header into a principal.

## Operation and worker contract

Gateway serves synchronous APIs and in one tenant transaction records the
Synveda operation, immutable authorization evidence, idempotency key and outbox
row. The provider-neutral operation stores tenant, requester principal
reference, closed kind and version, request digest/preconditions, requested and
authorised state, policy/profile evidence references, progress, attempts, next
retry, cancellation, terminal/dead-letter state, safe error code and audit
references. It never stores a bearer token or provider credential.

Queue payloads contain only an envelope version, tenant ID, Synveda operation
ID and content-free correlation value. The worker treats every field as
untrusted, opens an ordinary tenant-scoped forced-RLS transaction, resolves the
operation, and verifies its immutable tenant, kind/version, authorisation
state, request digest/preconditions, cancellation state and current attempt
fence. Cross-tenant IDs and malformed envelopes fail uniformly without
resource-existence disclosure.

Cedar still decides worker reads and writes. Its execution action authorises a
narrow worker service identity to consume the immutable capability represented
by an already-authorised operation of an assigned kind. The worker does not
rerun the requester's potentially changed grant, substitute its own domain
authority or reinterpret the original command. Cancellation is an explicit
operation transition. The domain effect passes through its normal
Cedar/VedaFlow/store/audit path with that operation capability; only the
current fenced attempt may commit the one effect and terminal state.

Dispatch claims use `FOR UPDATE SKIP LOCKED`, unique leases/fences and bounded
expiries. Submit/ack uncertainty is duplicate delivery by design. A dispatcher
may resubmit an uncertain envelope; idempotency and the attempt fence, not an
Apalis acknowledgement, decide whether an effect may commit. Apalis task IDs
and statuses remain adapter-private and never enter public APIs, audit actions
or telemetry labels.

The initial operation kind is `skill_validation`, version `1`, and invokes the
existing inert `validation_sandbox` Skill test. Inline execution is the default
and rollback path. Stable exact pins are `apalis = 0.7.4` and
`apalis-sql = 0.7.4`; 1.0 is release-candidate, stable 0.7.4 does not provide
Synveda's business idempotency/fencing/attempt state, and its undeclared MSRV
is an explicit build gate. Core/public crates import no Apalis type.

Acceptance injects: commit then dispatcher crash; submission then
acknowledgement-write failure; duplicate dispatch; two dispatchers; two
workers; executor SIGTERM; transient retry; terminal failure; cancellation;
restart recovery; cross-tenant operation ID; malformed payload; stale fence;
and rollback to inline. Each case asserts one effect or no effect as required,
ordinary RLS/PDP/audit evidence, content-free payloads and bounded retry.

## OpenTelemetry contract

The current gateway and core worker emit traces over OTLP/gRPC. The worker's
Prometheus `/metrics` listener is loopback-private, but the gateway's
unauthenticated `/metrics` route shares its application listener and is exposed
by the transitional Compose host port and Helm ingress. That is an explicit
pre-reference gap, not private monitoring evidence. The reference target cuts
application metrics over to OTLP/gRPC, so gateway, core worker, operation
dispatcher and Apalis executor have the private Collector as their only
telemetry egress seam. Keycloak metrics are scraped only on its private
management network. The Collector, not an application process, exposes a
private Prometheus exporter for the optional local backend. Current application
`/metrics` surfaces are transitional cutover seams, not part of the reference
target. Health endpoints remain separate from telemetry.

The Collector owns receiver limits, memory limiting, attribute
allowlisting/redaction, sampling, batching, bounded queues/retry and external
OTLP TLS/auth. Backend outage never changes gateway/worker readiness. Product
logs are bounded structured stdout without content; external log forwarding,
when configured, terminates at the Collector contract and requires no domain
code branch.

Telemetry excludes request bodies, prompts, messages, Knowledge, Skill files,
credentials, provider response bodies, raw paths, database statements and
arbitrary error text. Metric labels use closed vocabularies and never tenant,
principal, resource/operation ID, endpoint, model, worker ID or authored name.
The public proxy discards incoming trace/baggage headers and each public request
starts a new trusted trace context; internal propagation is allowlisted.
Local visibility has bounded storage/retention and is an evaluation tool, not
an SLO/on-call claim.

The `observability` profile uses pinned Prometheus, Jaeger and Perses images and
ships provisioned views for gateway latency/errors, worker last-seen,
operation/outbox age, retries/terminal failures, PostgreSQL errors, Keycloak
readiness/login failures, Session delivery, Capture and Knowledge-index lag,
context latency/token counts, Skill/MCP test failures and backup age/result.
The UI is loopback-only in development and available only through an
authenticated operator route in reference mode. Retention and disk limits are
finite; no dashboard is evidence of an SLO or high availability.

## Customer-safe Operations surface

The console `Operations` route consumes generated public-API types only. Its
API is tenant-scoped, RLS-protected and independently Cedar-authorised for each
aggregate. It returns bounded status vocabularies, timestamps, durations,
counts and public Synveda operation IDs; it does not query Prometheus, Keycloak
administration or Apalis tables from the browser.

The surface covers dependency status; recent operations, progress,
retry/dead-letter state and worker last-seen; recent Sessions; context latency
and token counts; Capture lag; Knowledge freshness/conflict/index health;
unhealthy Skill and MCP tests; latest backup/restore-test status; and degraded
external providers. Infrastructure jobs may submit only signed-internal,
content-free check results through a narrow service identity and audited
action. Loading, empty, degraded, stale and failed states are explicit and
tested.

It never returns raw Session/Knowledge/Skill/Tool content, prompts, secrets,
provider bodies, denied-resource counts, cross-tenant totals, Keycloak admin
data, database/queue credentials or Apalis task IDs. This customer view is not
the complete hosted-SaaS support or operator console.

## Object and backup storage

Synveda's current application object store is PostgreSQL-backed VedaFlow. This
contract does not falsely claim an external S3 application object store.
S3-compatible configuration delivered by CPR-45 is the pgBackRest repository
interface:
endpoint, region, bucket, path style, mounted CA and mounted credentials. A
future application object-store provider must receive its own accepted domain
contract without cloud-specific DTOs.

The exact reference tool is pgBackRest 2.59.1 (MIT). It is built into the
pinned PostgreSQL image and exposed through a versioned, deployment-neutral
`synveda-backup` command contract. PostgreSQL `archive_command` invokes
`synveda-backup archive-push`; operator backup commands execute in the database
container or an isolated restore job and require no pgBackRest daemon/listener.
The deterministic repository is a local POSIX named volume encrypted by
pgBackRest. Optional S3-compatible storage uses the same commands and evidence
schema. The repository cipher key and Synveda KEK are separate and neither is
stored in the recovery manifest.

## Backup and restore commands

`synveda-backup` has the closed commands `check`, `archive-push`, `create`,
`verify`, `status`, `expire` and `restore`. `create` accepts `full|diff|incr`;
the Compose acceptance always begins with a full backup. `restore` requires an
explicit target directory/volume and restore point and refuses a running source
cluster. Make targets are wrappers over this same interface, not an alternate
backup implementation:

- `make compose-backup` validates secret files, creates/checks the stanza
  idempotently, forces a WAL switch, takes a full backup, verifies repository
  and archive, and writes a content-free environment/recovery manifest.
- `make compose-restore-smoke` requires
  `SYNVEDA_CONFIRM_RESTORE=<restore-project-name>`, restores to freshly labelled
  volumes and an isolated network, boots the exact backed-up versions, then
  proves both databases, cross-role denial, schema epoch, forced RLS, Keycloak
  login, a governed public product lifecycle, Knowledge/index integrity, an
  encrypted tenant-secret open with the correct Synveda KEK, explicit failure
  with a wrong KEK, and frozen audit-prefix continuity. Current Knowledge rows
  are not described as application-envelope encrypted; backup-repository
  encryption and tenant-secret envelope encryption are distinct claims.
- PITR acceptance commits markers in both databases, restores to points before
  and after them, and verifies the expected paired cluster state. Destructive
  recovery never targets the source volume and cleanup names only resolved,
  labelled restore-test resources.
- `expire` runs only after the newest retained full chain has passed repository
  verification and an isolated restore test under the same environment
  manifest. Retention is versioned and bounded; an operator can always decline
  expiration.

The recovery manifest records source SHA, OCI/deployment digests, pgBackRest
version/stanza/backup label, PostgreSQL system identifier, database identities,
WAL coordinates, timestamps and the Synveda KMS key reference. It contains no
password, repository cipher key, KEK, bearer token, content or Keycloak realm
export. The KEK and repository key have a separately documented custody and
recovery path.

A shared PostgreSQL server is one physical recovery unit. Separate Synveda and
Keycloak RPO/PITR requires separate clusters. Same-host POSIX backup is portable
validation evidence, not host-loss disaster recovery.

## Restart, upgrade and rollback

The future `upgrade-from.json` under the canonical Compose test fixtures is the
sole N-1 test input.
Before acceptance it must contain literal prior test-build source SHA,
deployment digest, product/PostgreSQL/Keycloak image and platform digests,
schema epoch/migration head, Keycloak version, and configuration-schema
version. `previous`, mutable tags and a fixture built from the current SHA are
rejected. The planned CPR-45 fixture is an earlier, cleanly accepted Compose
test build with Keycloak 26.7.1 and schema epoch 3 before the forward operations
migration; it is test evidence, not a released-version support promise.

`make compose-upgrade-smoke` performs planned maintenance in this order:

1. boot the exact N-1 fixture, converge it twice, create governed sentinel
   state in both databases, and capture its labelled volume identities;
2. take and isolated-restore-test a joint backup plus Synveda key bundle;
3. stop public ingress, drain/stop gateway and workers, then stop Keycloak;
4. start the same PostgreSQL major/storage image, upgrade Keycloak only along
   the documented 26.7.1 → 26.7.2 path, and re-run realm convergence;
5. run the new Synveda migration exactly once under its migration owner, then
   start core/experimental workers, gateway and proxy in dependency order;
6. prove volume identities, role denial, issuer/login/CLI flow, tenant data,
   Session/Knowledge/audit sentinels, operation recovery and configuration
   validation, then prove a third convergence run is a no-op.

The restart matrix separately sends a graceful restart to proxy, gateway,
core worker, PostgreSQL, Keycloak and Collector, plus dispatcher/Apalis
executor/visibility services when enabled. After each restart it checks the
service-specific drain/recovery property and repeats the relevant public
lifecycle; a container merely returning to `running` is insufficient.

Compatible rollback is allowed only when the environment manifests prove that
neither Synveda nor Keycloak persisted a forward migration. Otherwise the
launcher refuses the old image with exact restore guidance. Keycloak database
downgrade is never attempted: rollback restores the paired pre-upgrade cluster,
repository/key material and exact old images. Synveda epoch-1/epoch-2,
markerless and unknown migration heads remain refused rather than translated.
There is no zero-downtime claim.

The full governed product lifecycle runs in both explicit development HTTP and
reference HTTPS modes. External-OIDC mode renders and runs its diagnostics
deterministically; a live provider is recorded unavailable unless actual
credentials and reachability exist.

## Runtime residue gate

After cutover, `make check-runtime-residue` requires zero active Rauthy or
Temporal service/image/config/environment/dependency/script/fixture/support
references. It scans runtime source, `Cargo.toml`/lockfiles, Make/scripts,
active deploy/release assets, README/install/security/architecture docs and
generated client/support contracts. Historical `docs/adr/**` decision records
are the only broad path allowlist; the gate's own encoded search vocabulary and
explicit negative-test fixtures are narrow line allowlists. The delivered
CPR-45 brief is removed under normal backlog discipline. Historical prose may
say why a component was removed, but cannot make it selectable, documented as
supported or present in a rendered service graph.

## Supported dependency modes

| Dependency | Bundled reference | External mode contract | Claim boundary while CPR-45 is open |
|---|---|---|---|
| PostgreSQL | PostgreSQL 17 + pgvector + pgBackRest | mounted DSN/CA and schema/migration ownership | external TLS support remains unclaimed until SQLx verify-full tests pass |
| OIDC | Keycloak 26.7.2 | issuer JSON file; custom CA/proxy remain open | only providers passing a live code exchange and conformance are named supported |
| telemetry | private Collector, optional local backends | Collector exporter to external OTLP | application configuration is unchanged |
| backup storage | encrypted POSIX volume | S3-compatible endpoint/region/bucket | backup interface only, not application object storage |
| embeddings | deterministic or optional TEI | configured HTTP provider | exact model/platform evidence is separate |
| extraction | deterministic, Claude or vLLM seam | configured endpoint/key file | live model claim requires credentialed evidence |
| registry | public OCI refs at digests | private registry prefix, CA and pull credentials | offline support needs a verified no-network bundle |

Configuration-schema tests may validate unavailable external modes. They are
not substitutes for live provider support evidence.

## Single-host limits

The reference has one host, one public proxy, one gateway, one core worker and
one PostgreSQL server. Restart policies reduce manual recovery time but do not
survive host or volume loss. Neither gateway nor worker is HA. Login/handoff and
some caches remain process-local until their separate scale feature lands.
There is no zero-downtime upgrade, multi-region routing, certified compliance,
owned RPO/RTO, general rate-limit/quota plane, complete tenant lifecycle,
24-hour mixed soak or general client-support claim.

Allowed labels are `development`, `reference` and `playground`. Never label
this topology HA, production SaaS, host-loss resilient, disaster-recovery
complete or enterprise certified.

## Future Helm mapping

| Contract concept | Compose reference | Later Kubernetes/OpenShift implementation |
|---|---|---|
| public edge | Caddy service, 80/443 | Ingress/Gateway API/OpenShift Route and customer certificate |
| gateway/worker | separate services from one image | separate Deployments with probes/drain and replicas only after OPS-7 |
| migration/convergence | one-shot migration and bounded long-running Keycloak convergence controller | migration remains a Job; convergence needs a controller/operator or a generation-fenced reconciliation design, never an application init shortcut |
| PostgreSQL | shared server, isolated DBs/roles | external PostgreSQL or CloudNativePG; Keycloak Operator/external DB as selected |
| secrets | mounted Compose files | External Secrets/Vault/CSI/projected Secrets and customer KMS |
| networks | explicit bridge networks | default-deny NetworkPolicies and reviewed ingress/egress |
| telemetry | private Collector | Collector/agent with external OTLP egress |
| backup | `synveda-backup` in the PostgreSQL container plus isolated restore job | a Job invokes the same `synveda-backup` command for a self-managed/CNPG data plane; an external managed database is explicitly externally owned until an adapter preserves the same command/evidence contract |
| identity | bundled Keycloak or external OIDC | customer IdP or Keycloak Operator; no domain change |
| persistence | named volumes | storage classes/PVCs with arbitrary-UID and SELinux evidence |
| supply chain | digest manifest | signed OCI chart/images, private registry and offline verified bundle |

OpenShift arbitrary UID, seccomp, service-account token suppression, PDB,
topology spread, private registry, offline installation, NetworkPolicies,
backup operator, FIPS and customer KMS remain later acceptance work. Helm is
not mechanically generated from Compose.

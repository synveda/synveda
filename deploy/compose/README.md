# Canonical Docker Compose deployment

This directory is Synveda's canonical single-host deployment for CPR-45. It
runs the gateway and worker as separate processes with PostgreSQL, bundled
Keycloak, a reverse proxy and a private OpenTelemetry Collector. Optional
settings add a bounded local Prometheus operator view, external trace export or
one experimental Apalis-backed Skill-validation worker.

It supports development and reference configuration. Logical backup, isolated
restore and same-schema product upgrade commands are implemented, but live
browser, reference-HTTPS, recovery and upgrade evidence is still required
before this implementation can be called validated for controlled single-host use. It is not an HA,
disaster-recovery, hosted-SaaS or enterprise-certification claim.

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
acceptance suffix and private `/24`, runs the real browser login, then uses two
real CLI login profiles to seed the existing public-API PulseBoard team
scenario. This includes Alice issuing and Bob redeeming a one-time workspace
invitation and one durable non-executing Skill validation. A second start
reopens and checks the active receipt; it does not repeat the mutations. The
gate then restarts PostgreSQL, Keycloak, Collector, worker, gateway and proxy
one at a time, runs the full smoke after each, repeats browser login and
verifies the existing receipt against live product rows. One project lock and
one bounded deadline cover the run. The identity admission, operation and
PulseBoard rows are persisted-state witnesses across the matrix.
Success leaves the stack running for inspection and later recovery/upgrade
gates; reset remains separately confirmed.

compose-down stops containers and preserves the PostgreSQL volume, any selected
Apalis transport volume and generated project inputs. When the
browser-acceptance profile is selected, down removes
that profile's disposable credential-and-receipt volume after checking its
exact ownership; it does not retain login tokens at rest. Confirmed reset with
the same profiles also removes it when stopping a running project. The default
ignored state directory is deploy/compose/runtime/synveda-development.

## Optional local metrics

The observability profile adds one private metrics path without changing the
application telemetry contract:

    export SYNVEDA_COMPOSE_PROFILES=observability
    make compose-up
    make compose-smoke

The Collector scrapes the gateway and worker privately. Prometheus reads only
the Collector fan-in and publishes its operator UI at
`http://127.0.0.1:${SYNVEDA_PROMETHEUS_PORT:-9090}`. Smoke requires the gateway
authority and worker readiness gauges to equal one and the worker heartbeat to
be no more than five seconds old, all from samples newer than the smoke start.
The profile applies 72-hour and 1-GB TSDB block-retention thresholds (whichever
triggers first). WAL, head-block and compaction overhead mean this is not a
volume or disk quota. The disposable `prometheus-data` volume is not part of
product recovery.

Use the same profile selector for `make compose-down` or confirmed
`make compose-reset`. Down preserves the metrics volume; reset validates and
removes it before the product database volume. On a remote host, reach the UI
only through an operator-controlled loopback tunnel. This profile is separate
from `compose-acceptance`, is not the tenant-safe Operations page, and carries
no production monitoring or alerting claim.

## Experimental Apalis canary

The disabled-by-default `apalis` profile changes only the
`skill_validation@1` execution transport. Synveda's forced-RLS operation,
attempt and transactional outbox rows remain authoritative; the ordinary core
worker remains the default rollback path. The profile adds one private
PostgreSQL queue, one bounded migration job and one private Apalis worker. It
has no public port or board:

    export SYNVEDA_COMPOSE_PROFILES=apalis
    make compose-up
    make compose-smoke

To execute the canary through the full fresh-project gate and include its
worker in the restart matrix:

    export SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance,apalis
    make compose-acceptance

The queue payload contains only an untrusted tenant routing identifier, the
Synveda operation ID and operation version. The worker rechecks tenant
association and current authority under forced RLS before applying the normal
Skill-validation effect. Duplicate delivery is fenced by the Synveda
operation lease and immutable test-result link; stale acknowledged delivery is
eligible for bounded resubmission.

The queue cluster's bootstrap owner is an isolated superuser used only by its
PostgreSQL process and one-shot migration service. The long-running worker
receives only the converged queue runtime password and Synveda's ordinary
worker DSN. The `apalis-data` volume is disposable transport state and is not
included in logical backup. Backup, restore, upgrade and standalone gateway
restart actions refuse this experimental profile. Select the default `postgres`
provider and stop the profile to roll back; there is no automatic queue
reconstruction or recovery claim. If a consumer remains unavailable, stale
deliveries are resubmitted at a bounded rate and may grow the disposable queue;
disable the profile until the consumer is repaired.

## Secrets

Generate secrets explicitly when preparing a bundled-PostgreSQL deployment:

    make compose-secrets
    make compose-issuer

The generator writes mode-0600 secret files below a mode-0700 project
directory, refuses overwrite unless explicitly forced and never prints secret
values. It refuses external PostgreSQL because those credentials and its CA
are operator-owned. The lifecycle's `--if-missing` preparation adds a missing
complete Apalis secret pair to a pre-Apalis set without rotating existing
credentials, but refuses a partial pair. The checked-in .env.example contains
no usable credential.

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

    SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance make compose-down
    SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance \
      SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-local make compose-reset
    SYNVEDA_CONFIRM_HOSTS_REMOVE=remove:127.0.0.1:synveda-development-acceptance-local:app.synveda.test:auth.synveda.test \
      make compose-hosts-remove

Unset the acceptance selectors before returning to the ordinary project.
Reinstall its hosts block only when continuing development:

    unset SYNVEDA_COMPOSE_PROJECT_SUFFIX SYNVEDA_COMPOSE_IPV4_POOL
    SYNVEDA_CONFIRM_HOSTS_INSTALL=install:127.0.0.1:synveda-development:app.synveda.test:auth.synveda.test \
      make compose-hosts-install
    make compose-resolver-check

## Logical backup and isolated restore

Recovery currently supports the bundled PostgreSQL and bundled Keycloak modes
only. Keep the exact source runtime, project suffix, profiles, hosts and image
selectors used to start the stack. If the source uses a non-default
`SYNVEDA_BOOTSTRAP_TENANT_ID`, retain that exact UUID for backup and restore;
the manifest binds it and a mismatch is refused. A backup verifies and smokes
the running graph, pauses the gateway, worker and Keycloak writers, creates separate
PostgreSQL 17 custom archives, snapshots the KMS key/reference and surviving
Keycloak convergence credential into a separate root, resumes the writers and
smokes again:

    export SYNVEDA_COMPOSE_PROFILES=demo,browser-acceptance
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    export SYNVEDA_BACKUP_ID=20260908-reference-1
    make compose-backup

If `SYNVEDA_BACKUP_ID` is omitted, the lifecycle uses a UTC-second identifier.
An ID is immutable and is never overwritten. Default mode-0700 roots are:

    deploy/compose/backups/database/<source-project>/<backup-id>
    deploy/compose/backups/secrets/<source-project>/<backup-id>

Both roots are ignored by Git and the image build context, but they still live
on the same host. For operator storage, set absolute existing mode-0700
`SYNVEDA_DATABASE_BACKUP_ROOT` and `SYNVEDA_RECOVERY_SECRETS_ROOT` paths owned
by the runtime UID/GID. They must be canonically non-overlapping with one
another and with active project inputs. Preserve both roots: the second
contains the KMS key and Keycloak credential needed to use the database pair.

Restore always targets a new suffixed project and private `/24`. It refuses
in-place recovery and requires the exact source, backup ID and derived target
project in one confirmation:

    export SYNVEDA_RESTORE_SOURCE_PROJECT=synveda-development-acceptance-local
    export SYNVEDA_BACKUP_ID=20260908-reference-1
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-restore1
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24
    export SYNVEDA_CONFIRM_RESTORE=synveda-development-acceptance-local:20260908-reference-1:synveda-development-acceptance-restore1
    make compose-restore-smoke

The command verifies both linked sets and the exact PostgreSQL image reference
before Docker mutation. It installs the recovered KMS key/reference and
Keycloak convergence password into the fresh target's normal secret set,
restores both databases, rebuilds planner statistics and reruns the existing
database authorities. Before tenant convergence can create product state, it
verifies the source bootstrap tenant and full audit chain through the ordinary
gateway database role, opens its current tenant data key, and proves an
unrelated key gets the expected cryptographic refusal. It then converges the
normal graph and completes the private browser login. The proxy publishes no
host port in this mode. Success leaves the target running for inspection.

For a reference-HTTPS restore, select `reference` and the target suffix first,
run `make compose-secrets`, and install the target certificate files exactly as
described below before invoking the restore. The source recovery manifests
must still match the selected immutable PostgreSQL image.

To remove the isolated target, retain its target suffix, private pool and
profiles, then use the ordinary exact-project commands:

    make compose-down
    SYNVEDA_CONFIRM_RESET=synveda-development-acceptance-restore1 make compose-reset

Reset removes the restored volume but deliberately retains the target secret
set. Remove retained inputs separately only after confirming they are no
longer needed.

This is a planned-interruption logical recovery check, not an online backup or
cross-database atomic snapshot. It pauses only the canonical application
writers; independently connected database writers are not fenced. The
archives and recovery-secret set are sensitive and are not encrypted by this
tool. SHA-256 links detect accidental alteration, not malicious replacement,
and are not signatures. The KMS check proves tenant-data-key unwrap; Synveda
does not claim application-level encryption of Knowledge bodies. Same-host
storage is validation evidence, not disaster recovery. Scheduling, encrypted
off-host retention, S3 transfer, WAL/PITR, RPO/RTO and recurring drills remain
OPS-5 production work.

If interruption reports that the exact-project lock was retained during
writer stop or backup, assume the four writers may still be stopped. Do not
delete the lock while its recorded PID or a bounded lifecycle child is alive.
Inspect the named Docker project and host process tree; once the recorded owner
and its children are conclusively gone, remove only
`/tmp/.synveda-compose-locks-<uid>/<source-project>.lock`. With the original
profiles still selected, run `make compose-down`; this removes the uncertain
containers/networks and the disposable browser credential-and-receipt volume,
but preserves the PostgreSQL volume and project inputs. For the
acceptance-project example above, then set `SYNVEDA_COMPOSE_PROFILES=demo`, run
`make compose-up`, and require `make compose-smoke` to pass. Never recursively
remove the lock directory. If process or Docker mutation state is uncertain,
leave the lock in place and escalate to the host operator.

## Same-schema product upgrade smoke

`make compose-upgrade-smoke` exercises a bounded application-image change
against an already accepted reference project. It requires bundled PostgreSQL
and Keycloak, the exact `demo,browser-acceptance` profiles, a project suffix,
reference TLS inputs and immutable image digests. Keep every ordinary
reference selector identical to the running project and set:

    export SYNVEDA_COMPOSE_RUNTIME=reference
    export SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-local
    export SYNVEDA_COMPOSE_IPV4_POOL=10.231.45.0/24
    export SYNVEDA_PRODUCT_STARTING_IMAGE=registry.example/synveda/product@sha256:<starting-digest>
    export SYNVEDA_PRODUCT_IMAGE=registry.example/synveda/product@sha256:<candidate-digest>
    make compose-upgrade-smoke

The references must resolve to different local image IDs. Under one project
lock and deadline, the command proves the running starting image, public
smoke, Keycloak browser login and existing product receipt. It pulls the
candidate by exact digest and runs `synveda db migrate --check` in a read-only
repeatable-read transaction. That check verifies the current schema epoch,
embedded migration ledger, database authority and forced-RLS catalogue without
running the SQLx migrator or changing persistent database state.

Only the long-running product services gateway and worker are
image-transitioned; the disposable browser-acceptance service is rerun at
each checkpoint. The command verifies candidate, starting rollback and final
candidate in turn; every checkpoint repeats exact container image identity,
public smoke, browser login and the existing product receipt. Success leaves
the candidate running. An ordinary failed transition
attempts to restore the last fully verified image but still returns failure.
A timeout, signal uncertainty or failed restoration retains the exact-project
lock for operator inspection.

This is planned-interruption, current-schema application compatibility
evidence. It does not upgrade PostgreSQL, Keycloak, volumes or schema; it does
not establish a general N-1 support window, downgrade safety, zero downtime or
Helm/release parity. Those remain OPS-6 work.

## Reference HTTPS

Reference mode publishes public traffic only on ports 80 and 443 and requires
real operator DNS, an explicit private /24, mounted certificate files and
immutable image references. The optional metrics UI remains loopback-only:

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

Create the mounted file as mode 0600 with the provider's exact values. This is
the minimal single-tenant shape; add provider-required scopes only when they
are needed to place the configured group claim in tokens:

```json
[
  {
    "issuer": "https://identity.example.com/realms/example",
    "client_id": "synveda",
    "audience": "synveda-api",
    "algorithms": ["RS256"],
    "tenant": {
      "static": {
        "tenant_id": "019b53c0-7c00-7000-8000-000000000045"
      }
    },
    "groups_claim": "groups",
    "login_scopes": ["openid", "profile", "email", "groups"]
  }
]
```

The API audience must differ from the login client ID. The provider must emit
the configured audience and group claim; `synveda-admins` is used only for the
one-time bootstrap described below, not ongoing Synveda authorisation.

In development, the hosts manager owns only the application .test name in this
mode. The external provider retains its own DNS name. If no bundled hosts block
exists, skip its removal; if one exists, remove it before changing selectors so
the exact ownership confirmation remains valid. Reference mode uses operator
DNS and does not run the hosts targets.

## External PostgreSQL

The executable external mode currently pairs external PostgreSQL with external
OIDC. The provider/operator must first provide PostgreSQL 17, create database
`synveda`, install `btree_gin` 1.3 and `vector` 0.8.6 in `public`, and create
the database owner plus the exact protected role shape: NOLOGIN capability role
`synveda_app` and the least-privilege migrator, gateway and worker logins with
the memberships, ownership and ACLs required by the selected role contract.
It must also provide a server certificate for the database DNS authority.
Compose validates this authority and applies Synveda migrations; it does not
provision, reset, back up or restore an externally owned cluster. Bundled
Keycloak with external PostgreSQL remains outside this slice.

Prepare an absolute mode-0700 secret directory owned by the configured runtime
UID:GID. Its path must end in the exact selected project and `/secrets`, for
example `/absolute/private/synveda-development/secrets`; a reference or
suffixed acceptance project uses its own exact project name. Create a
non-symlink mode-0700 `oidc-directory` child for the product's empty directory
secret. Put these non-symlink mode-0600 files in the secret directory:

- `synveda_migrator_database_url`;
- `synveda_gateway_database_url`;
- `synveda_worker_database_url`;
- `synveda_kms_key` and `synveda_kms_key_ref`; and
- `postgres_root_ca`, currently one PEM root certificate.

Each role URL has this exact TLS shape, with the password percent-encoded for a
URL and the role name varied per file:

    postgresql://synveda_gateway:<encoded-password>@database.example.com:5432/synveda?sslmode=verify-full&sslrootcert=/run/secrets/postgres_root_ca

Prepare the external issuer document separately as described above. Its parent
must also be a non-symlink mode-0700 directory owned by the runtime UID:GID and
the file must be non-symlink, nonempty and mode 0600. It must not sit inside
the secret directory. Then set:

    export SYNVEDA_POSTGRES_MODE=external
    export SYNVEDA_OIDC_MODE=external
    export SYNVEDA_SECRETS_DIR=/absolute/private/synveda-development/secrets
    export SYNVEDA_DATABASE_ROLES_FILE=$PWD/deploy/compose/configs/database/roles.external-oidc.json
    export SYNVEDA_DATABASE_EXPECTED_HOST=database.example.com
    export SYNVEDA_DATABASE_EXPECTED_PORT=5432
    export SYNVEDA_DATABASE_EXPECTED_NAME=synveda
    make compose-config
    make compose-up
    make compose-smoke

The preflight checks all three URLs against the declared endpoint and requires
hostname-and-CA verification before migration or either runtime starts. Native
TLS adds the mounted CA to normal platform trust; it is not exclusive
certificate pinning. A deterministic graph check is not a live external-
database claim. External acceptance, reset and recovery remain explicitly
refused.

## External OTLP traces

The application always sends traces to the private Collector. To forward them
to an external public-PKI OTLP/gRPC receiver, set one non-secret DNS authority:

    export SYNVEDA_OTLP_MODE=external
    export SYNVEDA_OTLP_EXPORT_ENDPOINT=otel.example.com:4317
    make compose-up

The Collector uses TLS, bounded batching, an in-memory queue and a one-minute
retry window. The endpoint accepts no scheme, path, credentials or IP address.
Private CA, authentication-header and mTLS delivery are not implemented in
this slice. Metrics remain local when the observability profile is selected,
and normal smoke proves Collector configuration rather than remote receipt.

## Network and edge contract

The selector divides one operator-selected private /24 into fixed internal and
egress networks and refuses foreign overlap. Only Caddy publishes public host
ports; the optional Prometheus operator UI binds only to host loopback.

The proxy removes caller-supplied Forwarded, X-Forwarded-*, X-Real-IP,
identity, original-path and tracing/baggage headers before adding its own
bounded forwarding values. It applies request-body, header and upstream
timeouts. PostgreSQL, Keycloak management, worker health, application metrics,
OTLP receivers and recovery jobs remain private. Prometheus is the only
profile-owned operator UI and is loopback-only.

Containers use non-root users where supported, read-only roots where
compatible, dropped capabilities, no-new-privileges, PID/resource bounds and
init/signal handling. Nothing is privileged and no service mounts the Docker
socket.

## Reset

Reset is destructive and requires the exact confirmation emitted by the
lifecycle:

    make compose-reset

It acts only on the validated project containers, networks, PostgreSQL volume,
profile-selected browser credential-and-receipt, Prometheus or Apalis transport
volume and transient database-authority/Keycloak-gate state. It deliberately
retains the project's secrets, issuer document and KMS key. Use the same
profile selector that created profile-owned volumes, review the target and
supply the requested confirmation. Use compose-down when product/database
state must be retained; the disposable browser-acceptance credentials are
still removed.

## Current validation gaps

The Docker reference implementation covers the source,
deterministic-contract and packaged-release boundaries. It still requires live
execution of clean development and reference HTTPS installs, the
restart/Apalis matrix, paired logical backup and isolated restore, and
same-schema product upgrade on Linux and one Docker Desktop platform.

A general dashboard platform, ACME, HA, Helm promotion, signed provenance,
S3/WAL-PITR recovery and enterprise controls remain later production work.
Same-host logical recovery does not establish disaster recovery or an owned
RPO/RTO.

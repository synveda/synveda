# Deployment

Synveda has one context-platform runtime. Direct binaries, source/release
Compose services and Helm Deployments use the same product commands, schema
epoch, generated `/v1` contract, embedded Cedar PDP, VedaFlow effects and
hash-chained audit path (CPR-36, ADR-0095, ADR-0102). The gateway is the public
request process; the private core worker owns scheduled Capture, Knowledge
index, relaxation-expiry and optional directory-pull work.

`personal`, `team` and `enterprise` are not deployment editions. They are
canonical Configuration documents copied into immutable governed versions and
bound to scopes after login. Deployment files may choose infrastructure size,
OIDC wiring, supported model implementations, secret references and telemetry;
they do not select policy, capture rules, context budgets, trace retention,
freshness or Skill/Tool advertisement.

- `compose/` is the contributor/single-node infrastructure: Postgres with the
  development extensions, bundled Rauthy, optional TEI and Jaeger. It also
  contains the product Dockerfile. During the CPR-45 cutover, `make dev-up`
  starts contributor services and `synveda init` starts the profiled gateway
  plus the Compose worker. This transitional layout is replaced, not retained,
  when canonical Keycloak Compose acceptance passes.
- `release/` is the pull-only single-node manifest installed under
  `~/.synveda/profile`. `scripts/package-release.sh` substitutes one release
  version and includes no source build or retired demo seeder.
- `helm/` is the Kubernetes infrastructure: separate gateway and worker
  Deployments from the same image, CloudNativePG, optional TEI, ingress and
  external IdP/secret wiring. The CloudNativePG operator is deliberately a
  separately installed cluster dependency.

## Bootstrap boundary

Both `synveda init` and the Helm install job do only the operations for which no
authenticated product principal exists yet:

1. apply the current schema chain;
2. provision/grant distinct gateway and worker runtime LOGINs;
3. optionally admit the first tenant;
4. establish deployment key/issuer material.

The first `synveda-admins` login creates the tenant root, the caller's principal
scope and its root `administrator` grant. Workspaces, projects, sessions,
capture decisions, Knowledge and Configuration are public-API/PDP/VedaFlow/
audit acts after that. No deployment script inserts those tables directly.

## Runtime database roles and forced RLS

Migrations create `synveda_app` as a NOLOGIN capability role. Runtime-role
sentinels require it to remain non-elevated and membership-free, own no
database, and own no schema, relation or routine in the selected Synveda
database. Local application processes use fixed,
distinct LOGINs that inherit only that capability:

- `synveda init` converges `synveda_gateway` and `synveda_worker` as LOGIN,
  INHERIT, owner of no database and no schema, relation or routine in the
  selected Synveda database, non-superuser and non-`BYPASSRLS`, each with
  `synveda_app` as its only, inheriting, non-admin membership, then verifies
  each resolved host-side runtime connection;
- the Helm install job converges the fixed `synveda_worker` role from a
  separately owned Secret, connects through its mounted URL, and refuses a
  live PostgreSQL primary instance/database target different from the install
  connection;
  the worker verifies that exact role and epoch again before readiness.

The current Helm gateway remains an explicit pre-reference gap: CloudNativePG's
generated application Secret is also the database owner. The chart refuses to
reuse it for the worker, but a later migrator/runtime credential cutover must
give the gateway a separate runtime Secret whose login owns no database and no
schema, relation or routine in the selected Synveda database before Helm satisfies the locked deployment
contract. No current evidence claims otherwise.

When `DATABASE_URL` is explicit—even when its host is loopback—`init` treats it
as an operator-owned bootstrap target and also requires both
`SYNVEDA_GATEWAY_DATABASE_URL` and `SYNVEDA_WORKER_DATABASE_URL`. Only the
implicit bundled default derives and converges the fixed development
credentials. The explicit URLs must name different, separately provisioned
roles on their actual target servers.
Init connects through each URL and verifies LOGIN/INHERIT, exact session
identity, ownership of no database and no schema, relation or routine in the
selected Synveda database, non-elevation and sole `synveda_app` membership; it
also compares the cluster system identifier, database OID and live postmaster
start marker with the bootstrap connection and requires a writable primary,
refusing a different live primary instance or read-only installation. The
marker is used only for the concurrent bootstrap comparison, not persisted
across restarts. For the implicit bundled default,
init instead checks the derived `localhost` URLs; the
container services use the Compose `postgres` alias, and the worker rechecks
its real session at runtime. The gateway has no equivalent runtime sentinel
yet. Diagnostics redact credentials.

The worker's default supervised join is 75 seconds. Both transitional Compose
manifests give it an 85-second outer stop grace; the installed release also
uses `restart: unless-stopped` so a deliberate non-zero critical-task exit is
visible and restarted. Helm derives its termination grace as the configured
worker join plus ten seconds.

`make check-deploy` renders both transitional Compose manifests and Helm,
asserts distinct process commands/credentials and private worker probes,
packages the release twice and checks the upgrade-shaped replacement. The
CPR-36 database acceptance test also proves a runtime login with no tenant GUC
cannot read tenant data. The kind acceptance script asserts the worker role
facts before a governed round trip and repeats its private readiness check
after CloudNativePG primary failover.

## Why the Compose gateway may run on the host

The bundled Rauthy issuer is `http://localhost:8100/auth/v1/`. An OIDC issuer
identifier must be the same URL for the browser, discovery document, token and
gateway; RFC 6761 resolves `localhost` to each caller's own loopback. The
default installed gateway therefore still runs as a host process during this
transition; the core worker is already a Compose service. An external issuer
has a mutually reachable DNS name and `synveda init --issuer ...` runs both
processes as separate services from the same product image. Canonical Keycloak
Compose removes this workaround by giving browser and containers one exact
proxy issuer name.

## Embeddings

`deterministic` is a lexical-only development implementation. `tei` serves
BGE-M3 and is the meaningful semantic option. Upstream's amd64 image is version
tagged; its arm64 image is pinned by commit because no versioned arm64 tag is
published. The two tested builds produce the same 1024-dimensional model output
to float32 rounding (cosine `1.000000000`, maximum absolute difference `7e-8`,
measured 2026-07-26).

Knowledge embedding rows retain model and dimension. A model change converges a
separately labelled sidecar; an old vector is never reinterpreted as output from
a new model. TEI's cache is persistent in Compose and Helm because a cold
BGE-M3 download is about 2.3 GB.

## Honest operating limits

- Helm runs one gateway and one core-worker replica with `Recreate`. Pending
  login state and cross-process cache invalidation have not passed OPS-7; the
  chart refuses replica settings. CloudNativePG provides a replicated data
  plane, not request or worker HA, and an application upgrade has a brief
  outage. Worker SIGTERM has bounded cancellation/join evidence at idle;
  interruption during claimed Capture work and two-worker execution remain
  open.
- Compose is a local single-node shape with explicit development database
  credentials. It is not a production secret-management example.
- The chart has no Qdrant, Temporal consumer, backup promise, external HSM or
  customer-managed-key implementation. Provider credentials are Secret
  references; rendered diagnostics must not contain values.
- Release binaries are unsigned and un-notarized; shipped binaries are macOS
  arm64 and Linux x86_64 only. There is no Windows build, zero-downtime gateway
  upgrade guarantee or old-schema translator.

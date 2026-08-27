# Deployment

Synveda has one context-platform runtime. The host binary, source/release
Compose service and Helm Deployment run the same gateway, schema epoch,
generated `/v1` contract, embedded Cedar PDP, VedaFlow effects and hash-chained
audit path (CPR-36, ADR-0095).

`personal`, `team` and `enterprise` are not deployment editions. They are
canonical Configuration documents copied into immutable governed versions and
bound to scopes after login. Deployment files may choose infrastructure size,
OIDC wiring, supported model implementations, secret references and telemetry;
they do not select policy, capture rules, context budgets, trace retention,
freshness or Skill/Tool advertisement.

- `compose/` is the contributor/single-node infrastructure: Postgres with the
  development extensions, bundled Rauthy, optional TEI and Jaeger. It also
  contains the gateway Dockerfile. `make dev-up` starts contributor services;
  `synveda init` starts the profiled gateway.
- `release/` is the pull-only single-node manifest installed under
  `~/.synveda/profile`. `scripts/package-release.sh` substitutes one release
  version and includes no source build or retired demo seeder.
- `helm/` is the Kubernetes infrastructure: the same gateway image,
  CloudNativePG, optional TEI, ingress and external IdP/secret wiring. The
  CloudNativePG operator is deliberately a separately installed cluster
  dependency.

## Bootstrap boundary

Both `synveda init` and the Helm install job do only the operations for which no
authenticated product principal exists yet:

1. apply the current schema chain;
2. provision/grant a least-privilege gateway LOGIN;
3. optionally admit the first tenant;
4. establish deployment key/issuer material.

The first `synveda-admins` login creates the tenant root, the caller's principal
scope and its root `administrator` grant. Workspaces, projects, sessions,
capture decisions, Knowledge and Configuration are public-API/PDP/VedaFlow/
audit acts after that. No deployment script inserts those tables directly.

## Forced RLS in every deployed shape

Migrations create `synveda_app` as a NOLOGIN capability role and grant each new
table only the privileges its runtime paths need. The gateway never connects as
the database owner:

- `synveda init` converges local `synveda_gateway` as LOGIN, non-superuser,
  non-BYPASSRLS and a member of `synveda_app`; the host and Compose gateway DSNs
  use it;
- CloudNativePG generates the Helm login and the install job grants it the same
  membership; the admin Secret exists only in migration/tenant-admission
  containers.

For a separately provisioned Postgres login, set
`SYNVEDA_GATEWAY_DATABASE_URL` before `synveda init`. Init verifies that the
named role already has LOGIN, is neither superuser nor BYPASSRLS, and inherits
`synveda_app`; it refuses to start the gateway when any fact is false. The
credential is written only to the deployment's mode-0600 environment file and
is redacted from diagnostics.

`make check-deploy` renders both Compose manifests and Helm, rejects an owner
DSN or removed runtime surface, packages the release twice and checks the
upgrade-shaped replacement. The CPR-36 database acceptance test also proves a
runtime login with no tenant GUC cannot read tenant data. The kind acceptance
test asserts the same role facts in a running chart before a governed round
trip and CloudNativePG primary failover.

## Why the Compose gateway may run on the host

The bundled Rauthy issuer is `http://localhost:8100/auth/v1/`. An OIDC issuer
identifier must be the same URL for the browser, discovery document, token and
gateway; RFC 6761 resolves `localhost` to each caller's own loopback. The
default installed gateway therefore runs as a host process. An external issuer
has a mutually reachable DNS name and `synveda init --issuer ...` uses the same
gateway image. This changes process placement, not product behaviour.

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

- Helm runs one gateway replica with `Recreate`. Pending login state and
  cross-process cache invalidation have not passed OPS-7; the chart refuses a
  replicas value. CloudNativePG provides a replicated data plane, not gateway
  HA, and a gateway upgrade has a brief outage.
- Compose is a local single-node shape with explicit development database
  credentials. It is not a production secret-management example.
- The chart has no Qdrant, Temporal consumer, backup promise, external HSM or
  customer-managed-key implementation. Provider credentials are Secret
  references; rendered diagnostics must not contain values.
- Release binaries are unsigned and un-notarized; shipped binaries are macOS
  arm64 and Linux x86_64 only. There is no Windows build, zero-downtime gateway
  upgrade guarantee or old-schema translator.

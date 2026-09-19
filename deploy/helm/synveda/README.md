# Synveda on an existing Kubernetes cluster

OPS-11 extends the existing application chart. One product image runs the
API and its existing console host, one private core worker, and a bounded
installation Job. The base Service is ClusterIP. PostgreSQL and OIDC are
required; semantic embeddings and model extraction are optional. No new queue,
operator, identity framework or cluster-scoped resource is installed.

The default is **external PostgreSQL and external OIDC**, one organisation
(tenant) and namespace per installation, with one gateway and worker using
`Recreate`. Keycloak is the tested OIDC provider. `postgres.mode: cnpg` retains
the existing integration with a separately installed CloudNativePG operator;
it defaults to one database instance. The optional
[persistent starter](STARTER.md) adds upstream-packaged Keycloak with a separate
database/user, retained storage and explicit owner/team onboarding. All four
provider combinations use this chart. OpenShift qualification remains OPS-11
work.

## Image and values

Use an image built from this checkout, or an image whose registry pull and
source revision you have verified. The default GHCR coordinates are the existing
release contract, **not evidence of a published usable release**. Build locally:

```sh
docker build -t synveda/product:ops11 -f deploy/compose/product/Dockerfile .
```

Load that image into Kind for local testing, or tag/push it to your own registry
for an existing cluster. `image.repository` plus `image.tag`, or
`image.repository` plus `image.digest: sha256:…`, selects the same artifact for
all application processes and Job stages. Tag and digest are mutually exclusive.
`imagePullSecrets` accepts existing namespace-local registry Secrets.

Start from [ci/external-values.yaml](ci/external-values.yaml), a non-secret
specimen, and save your actual configuration as `team-values.yaml`. Replace its
example DNS names and Secret references, select your actual image, and configure
one `install.tenant.id` (a UUIDv7), `slug` and `name`. The issuer document below
uses that same ID. IDs are operator inputs, never generated on an upgrade.
Empty tenant settings mean admission is already managed by the operator.

The closed [values.schema.json](values.schema.json) checks names, bounds,
required external settings and mode selection. Helm also refuses unsupported
replicas, simultaneous tag/digest, ignored provider settings, missing requested
APIs and inconsistent public URL/TLS configuration. Schema validation runs on
lint, render, install and upgrade. No setting bypasses runtime authority checks.

## External PostgreSQL

Supply `postgres.external.host`, `port`, `database`, the migrator Secret/key,
the root CA Secret/key and the exact `roles` document. Supply separate
`gateway.databaseExistingSecret` and `worker.databaseExistingSecret` values.
Each Secret's `DATABASE_URL` key contains one complete SQLx URL, including its
**own** username and percent-encoded password. `databaseUrlSecretKey` and
`migratorUrlSecretKey` can select different key names. The chart neither renders
nor reconstructs those credentials. External mode uses no password-only keys,
CNPG resource, bootstrap administrator credential, database PVC or Kubernetes API
client.

Every external URL must include these exact parameters:

```text
sslmode=verify-full&sslrootcert=/run/secrets/synveda-postgres/ca.crt
```

`caExistingSecret` / `caSecretKey` mount the issuing PEM root at that path.
The server presents its certificate/chain and its SAN must cover the declared
host. SQLx's native TLS backend checks both the trust chain and hostname; the
root augments normal platform trust rather than creating exclusive pinning.
There is no hostname-based inference that a server is local or trusted, and no
external plaintext switch. CA/host verification applies on every gateway,
worker, migration and tenant-command connection, including automatic restarts.
See the pinned [SQLx PostgreSQL TLS modes](https://docs.rs/sqlx/0.8.6/sqlx/postgres/enum.PgSslMode.html).

For optional mutual TLS, supply `clientExistingSecret`, `clientCertSecretKey`
and `clientKeySecretKey`, and add both URL parameters:

```text
sslcert=/run/secrets/synveda-postgres-client/tls.crt&sslkey=/run/secrets/synveda-postgres-client/tls.key
```

Use a PEM client certificate and unencrypted PKCS#8 key. Server verification and
separate role passwords remain required. The pair is transport identity, not a
replacement for database roles or tenant authority. Missing/mismatched files,
weaker TLS modes and undeclared endpoint changes fail closed without printing
the URL. A different namespace, on-premises host or cloud service uses the same
contract.

### Provisioning and privileges

The validated server is **PostgreSQL 17.11**; the runtime targets PostgreSQL 17.
Other major versions are unqualified. The current authority contract requires
`vector` **0.8.6**, `btree_gin` **1.3**, and standard `plpgsql` **1.0** in their
expected schemas/ownership. No AGE, PGMQ, Redis, separate vector database,
object store, pooler or model server is needed for the lexical core.

An external database administrator must provision the database, extensions and
roles **before installation**. Ordinary startup never creates administrative
extensions or roles in external mode. The existing reference for the exact
contract is [the shared database bootstrap](../../compose/postgres/synveda-database-bootstrap)
and [runtime role verifier](../../../crates/synveda-store/src/runtime_role.rs):

- The migrator is an ordinary, non-superuser owner of only the application
  database/public schema and resulting application objects, with no elevated
  role memberships.
- `synveda_app` is the NOLOGIN capability role. Gateway and worker are distinct
  non-owner LOGIN roles that inherit only that capability; neither may have
  `BYPASSRLS`, superuser, role/database-creation or replication privileges.
- Database/schema ACLs, trusted administrator identities, exact administrator
  grantor pairs and forbidden/peer databases must match `roles`. Runtime
  checks include the cluster identity proof and the installed function, grant
  and forced-RLS catalogue. These are mandatory, including on managed servers.

Managed-service branding does not establish compatibility with this exact
contract. An ordinary database user need not be a superuser, but the provider
must expose the required administrative provisioning and read-only identity
proof. Run the existing `database-preflight` before promotion; do not relax the
verifier to accommodate a managed service. Budget the two runtime pools plus
migration/operator headroom against the provider's connection limit.

CNPG mode alone runs the existing bounded administrator bootstrap, then the
same ordinary-role preflight/migrator. Its existing private in-cluster URLs use
the driver's default transport; **that mode does not claim verify-full**.
Keep that traffic within a trusted private cluster network. Use the verified
external contract when transport verification is required. CNPG installs no
operator and refuses rendering when its API is missing; offline rendering must
explicitly declare `--api-versions postgresql.cnpg.io/v1`. The default external
lint needs no operator or CRD.

## Identity, membership and Secrets

Create an issuer JSON file with the actual canonical issuer URL, `client_id`,
a distinct API `audience`, `algorithms: ["RS256"]`, the static tenant ID and
`groups_claim: "groups"`. `login_scopes` defaults to the existing browser-flow
scopes; configure the provider to emit those claims. Register exactly
`<gateway.publicUrl>/auth/callback` for the existing authorization-code + S256
PKCE flow, with the public origin as the allowed web origin. No wildcard
redirect or Keycloak administrator credential is used by the application.
The issuer must be byte-identical in discovery and tokens and reachable by
browsers and pods. A confidential client's `client_secret`, if used, stays in
the server-side issuer Secret, never the console bundle.

`oidc.caExistingSecret` / `caSecretKey` optionally mount one organisation PEM
root. `SYNVEDA_OIDC_CA_CERT_FILE` augments the same client's trust for discovery,
JWKS and token exchange; certificate and hostname verification remain enabled.
Install that CA in your users' browsers/clients separately. HTTP is refused
unless `gateway.insecureDevelopmentHttp: true` explicitly selects disposable
private development access. A shared installation uses an HTTPS public URL.

The first eligible `synveda-admins` group login uses the existing audited
bootstrap path. Subsequent external authentication creates a principal, not
workspace membership or administrative authority. Invitations, membership,
grants, Knowledge publication and configuration changes still use the public
API, Cedar, forced RLS, VedaFlow and content-free audit.

Create Secrets from private operator files, for example:

```sh
kubectl -n synveda create secret generic synveda-oidc --from-file=SYNVEDA_OIDC_ISSUERS=./private/issuers.json
kubectl -n synveda create secret generic synveda-gateway-db --from-file=DATABASE_URL=./private/gateway-database-url
kubectl -n synveda create secret generic synveda-worker-db --from-file=DATABASE_URL=./private/worker-database-url
kubectl -n synveda create secret generic synveda-migrator-db --from-file=DATABASE_URL=./private/migrator-database-url
kubectl -n synveda create secret generic synveda-postgres-ca --from-file=ca.crt=./private/postgres-ca.pem
kubectl -n synveda create secret generic synveda-kms --from-file=SYNVEDA_KMS_KEY=./private/kms-key --from-file=SYNVEDA_KMS_KEY_REF=./private/kms-key-ref
```

These commands assume the namespace and actual private files already exist.
Keep the KMS key (64 hex characters) and stable reference together and backed up
separately from the database. Rotating a Secret does **not** rotate encrypted
state: use the existing key-management ceremony; replacing the KEK arbitrarily
makes wrapped keys unreadable.

All runtime credentials use the existing bounded `*_FILE` loader. Projected
files are readable by arbitrary non-root UIDs, only in containers mounting that
projection; no fixed UID/GID or service-account token is needed by the product.
The chart never generates Secrets or copies their values into Helm release
configuration. After externally rotating passwords, issuer credentials, CAs or
extractor keys, roll the affected Deployment: startup reads are not hot reloads.
Coordinate database password changes and rollout in a maintenance window;
`helm upgrade` alone does not detect an existing Secret's changed contents.
Changing the role ConfigMap through values does trigger a rollout.

The optional existing TEI workload has its own resource, scheduling, annotation,
cache and non-root pod settings. Its upstream image defaults to root, so
`tei.podSecurityContext` supplies UID/fsGroup 1000 on ordinary Kubernetes;
platforms assigning these identities can remove those two values with `null`.
Its model cache is the only application-chart PVC in external mode when enabled.
TEI remains optional and its outage does not gate core gateway/worker readiness.

## Exposure, installation and health

For a shared team, configure the HTTPS public origin and either an existing
private TLS proxy or `ingress.enabled`, matching `ingress.host`, controller
class/annotations and a TLS Secret covering that host. If TLS terminates ahead
of Ingress, explicitly set `ingress.externalTlsTermination: true`. The chart
installs no controller. Ingress forwards only `/console`, `/auth`, `/v1` and
`/scim/v2`; `/metrics` and worker management have no public ingress route. There
was no Gateway API implementation to reuse. Keep private access explicit when
Ingress is disabled; registering a public URL does not itself expose a Service.

After provisioning the dependencies and namespace-local Secrets:

```sh
helm lint deploy/helm/synveda --strict -f team-values.yaml
helm template synveda deploy/helm/synveda -n synveda -f team-values.yaml > rendered.yaml
helm upgrade --install synveda deploy/helm/synveda -n synveda -f team-values.yaml --wait --wait-for-jobs --timeout 15m
kubectl -n synveda logs job/synveda-install-1 -c database-preflight
kubectl -n synveda logs job/synveda-install-1 -c migrate
```

Use the current release revision in the Job name after an upgrade. The Job is a
normal revision-scoped resource, **not a Helm hook**: all resources can exist
before Helm waits. Up to 12 bounded preflight attempts precede migration; the
migration command has a 300-second process deadline and the Job has a default
900-second total deadline. SQLx's PostgreSQL advisory lock serializes migration
DDL; the existing epoch guard refuses old schemas without modifying them.
Tenant convergence is idempotent by the preselected ID. The same command works
on a fresh database and on a same-epoch upgrade/rerun; this is not an old-epoch
migration or rollback promise.

Startup probes allow ten minutes for legitimate initialization. Liveness checks
only the local process. Existing readiness proves database/schema/role authority
before serving requests or claiming work. Optional embedding, extraction and
telemetry outages do not become liveness failures. Gateway grace is its
configured shutdown bound plus ten seconds; worker grace is its existing join
bound plus ten seconds. Readiness withdrawal, worker cancellation and database
lease recovery retain their existing semantics. Claimed-work SIGTERM recovery
and multi-worker operation remain separately unqualified.

## Reproducible acceptance

The external fixture builds these exact Dockerfiles into local `:ops11` tags,
creates a disposable Kind cluster, provisions independent PostgreSQL and
Keycloak in `synveda-dependencies`, and installs only the application release
in `synveda-test`:

```sh
POSTGRES_MODE=external bash demos/ops-2-helm-install.sh
```

Its exact Helm values are [external-values.yaml](../../../demos/fixtures/ops-2/external-values.yaml),
and its installation command is:

```sh
helm upgrade --install synveda deploy/helm/synveda -n synveda-test -f demos/fixtures/ops-2/external-values.yaml --wait --wait-for-jobs --timeout 15m
```

The fixture deliberately uses private HTTP for the application and private-CA
HTTPS for Keycloak; PostgreSQL requires verified TLS and a client certificate.
Only its separate administrator bootstrap uses the private provisioning
exception. No CNPG API is installed. The existing CNPG/failover test remains
`bash demos/ops-2-helm-install.sh` without the external selector.
`KEEP=1` retains the disposable cluster for diagnosis; `REUSE=1` may select an
empty precreated `synveda-ops11` cluster and never deletes that cluster.
`SKIP_BUILD=1` is for images already
built from the exact current source, never an acceptance shortcut.

Measured results and remaining qualifications are recorded in
[OPS-11](../../../docs/backlog/OPS-11.md). This chart does not establish cloud,
OpenShift, HA, disaster recovery, published artifact availability or a Kubernetes
minor-version support window.

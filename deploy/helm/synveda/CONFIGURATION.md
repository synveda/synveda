# Kubernetes configuration reference

Use the [installation guide](README.md) and save one non-secret values file.
[values.yaml](values.yaml) is the complete field reference;
[values.schema.json](values.schema.json) rejects unknown or inapplicable inputs.
Helm values never carry passwords, keys or tokens.

| Change for every installation | Meaning |
|---|---|
| `gateway.publicUrl`, exposure host and TLS Secret | Actual HTTPS application origin; exact callback is this origin plus `/auth/callback` |
| `install.tenant.id`, `slug`, `name` | Stable UUIDv7 and organisation names; use the same UUID in the issuer Secret on every upgrade |
| `image.repository` with `image.digest` | Product artifact from the verified release; clear `image.tag` when using a digest |
| `postgres.mode` | `external`, `bundled` (one persistent instance), or `cnpg` with a preinstalled operator |
| Database Secret names and external host/CA/roles | Three distinct credentials: migrator, gateway and worker |
| `oidc.existingSecret`, optional `oidc.caExistingSecret` | Exact trusted issuer/client/audience and optional private root |
| `kms.existingSecret` | Original 64-hex KEK plus stable reference; keep a protected recovery copy |
| Packaged Keycloak URL, Secrets, trusted proxy addresses | Required only with `keycloak.enabled`; a separate database/owner |
| CNPG storage size/class, registry pull Secrets | Real cluster/provider inputs; no universal class name |

Leave one gateway/worker, `Recreate`, security contexts, health paths, role
checks and migration ordering alone. Provider selection cannot copy data.
Keep `embedder: deterministic` and `extractor.kind: deterministic` for the
measured lexical starter; TEI and model extraction are optional. Configuration
inside the product governs provider admission independently of these values.

`otel.endpoint` accepts the existing OTLP/gRPC endpoint. An organisation's
private collector can forward to its authenticated/TLS backend; no collector
or dashboard is installed by this chart. The current application transport is
private plaintext gRPC; do not point it across an untrusted network or assume
`outbound.caBundleExistingSecret` configures OTLP TLS. An empty endpoint retains
the runtime's localhost default and reports bounded export failures when no
collector exists; it does not change liveness/readiness. Gateway `/metrics`
remains private. Worker health/metrics bind loopback; operator access uses
`kubectl exec`, not an additional public Service.

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

CNPG and bundled modes run the existing bounded administrator bootstrap, then the
same ordinary-role preflight/migrator. Its existing private in-cluster URLs use
the driver's default transport; **that mode does not claim verify-full**.
Keep that traffic within a trusted private cluster network. Use the verified
external contract when transport verification is required. CNPG installs no
operator and refuses rendering when its API is missing; offline rendering must
explicitly declare `--api-versions postgresql.cnpg.io/v1`. The default external
lint needs no operator or CRD.

## Identity and Secret formats

Create an issuer JSON file with the actual canonical issuer URL, `client_id`,
a distinct API `audience`, `algorithms: ["RS256"]`, the static tenant ID and
`groups_claim: "groups"`. `login_scopes` defaults to the existing browser-flow
scopes; configure the provider to emit those claims. Register exactly
`<gateway.publicUrl>/auth/callback` for the existing authorization-code + S256
PKCE flow, with the public origin as the allowed web origin. No wildcard
redirect or Keycloak administrator credential is used by the application.
The canonical issuer is byte-identical in discovery and tokens. The login
client is **public**, with authorization code + S256 PKCE; this trust entry has
no `client_secret` field. Service credentials use the separately documented
service-identity flow, not a secret in browser JavaScript.

An optional `discovery_url` selects a provider's explicit backchannel.
Discovery must still return the canonical issuer, its authorization endpoint
must retain the public origin, and token/JWKS endpoints must retain the
configured backchannel origin. HTTPS issuers cannot downgrade to HTTP.
Loopback evaluation explicitly permits private HTTP; external HTTPS uses
trusted certificates. This implements Keycloak's supported dynamic backchannel,
not arbitrary forwarded-header trust. Console Sign out revokes Synveda's sealed
session and cookie; provider SSO remains until its own logout or browser-session
closure. No provider logout redirect is registered or implicitly performed.

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

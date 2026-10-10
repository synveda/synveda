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
expected schemas/ownership. The [PostgreSQL compatibility contract](../../../docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility)
defines the exact catalogue and provider qualification boundary. These are
mandatory for lexical operation as well. No AGE, PGMQ, Redis, separate vector database,
object store, pooler or model server is needed for the lexical core.

An external database administrator must provision the database, extensions and
roles **before installation**. Ordinary startup never creates administrative
extensions or roles in external mode. The existing reference for the exact
provisioning is [the shared database bootstrap](../../compose/postgres/synveda-database-bootstrap)
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
verifier to accommodate a managed service. This role/target probe must still
be followed by migration and full runtime extension proof. Budget the two runtime pools plus
migration/operator headroom against the provider's connection limit.

The [Azure reuse walkthrough](examples/README.md#reuse-existing-services-on-azure)
checks available extensions before installation and reuses existing database,
OIDC and cluster owners. Azure Flexible Server's documented PostgreSQL 17 /
pgvector 0.8.2 combination is currently refused; the recipe does not qualify it.

CNPG and bundled modes run the existing bounded administrator bootstrap, then the
same ordinary-role preflight/migrator. The bundled preparation recipe supplies
verify-full runtime URLs and its generated private CA; the administrator
bootstrap remains private cluster traffic. CNPG owner/runtime URLs use the
driver's default transport and do not claim verify-full. Keep bootstrap/CNPG
traffic within a trusted private cluster network. External services always use
the verified TLS contract. CNPG installs no
operator and refuses rendering when its API is missing; offline rendering must
explicitly declare `--api-versions postgresql.cnpg.io/v1`. The default external
lint needs no operator or CRD.

### Optional CNPG object-store backup

`postgres.backup` defaults off. For CNPG only, set `enabled: true`, an exact
same-namespace `objectStoreName`, and an explicit six-field `schedule` after
the operator has installed the Barman Cloud plugin and an encrypted, accessible
ObjectStore. The chart attaches WAL archiving and schedules physical base
backups; it does not create provider credentials or select retention. An
external/bundled database keeps its own backup owner. Follow the
[OPS-5 backup candidate](BACKUP.md) before using this setting. Offline Helm
renders also need `--api-versions barmancloud.cnpg.io/v1`.

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

## DBA handoff

Give this section to the existing PostgreSQL service owner before installation.
Check [PostgreSQL compatibility](../../../docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility)
first; version strings alone do not prove trusted extension catalogues,
ownership or provider authority.
The example below provisions a **new dedicated PostgreSQL 17 database/server**
with administrator `postgres`. It is not a convergence script for an existing
team database. On a shared server, revoking PUBLIC maintenance-database access
can affect other consumers: the DBA must arrange an equivalent effective-denial
contract and declare the actual administrator/member/grantor identities before
proceeding. Never run this recipe against an unrelated retained database.

| DBA supplies | Required result |
| --- | --- |
| Server and extension versions | PostgreSQL 17 (qualified 17.11); vector 0.8.6 and btree_gin 1.3 in `public`; plpgsql 1.0 in `pg_catalog`; no extra extension/operator dependencies |
| Ownership | `synveda_migrator` owns only `synveda` and its public schema/application objects; extensions retain the trusted administrator owner |
| Runtime roles | Distinct `synveda_gateway` and `synveda_worker`; neither owns schema/data; both inherit only NOLOGIN `synveda_app` |
| Authority | All four application roles: no superuser, BYPASSRLS, CREATEROLE, CREATEDB or replication; no privileged/default/global-object grants; ordinary logins can read the required catalogue/cluster identity proof |
| Isolation | PUBLIC has no database CONNECT/TEMP or public-schema access; runtime roles have CONNECT only to the application database; declared forbidden databases/identity peers stay inaccessible |
| TLS | Actual host/SAN, issuing PEM root, verified chain, SCRAM passwords; each URL uses verify-full and the fixed mounted CA path; optional client certificate/key separately |
| Capacity and recovery | Connection budget for gateway/worker pools plus migration/operator headroom, durable storage/backups, separately recoverable exact roles, original credential files and TLS custody |

The DBA uses PostgreSQL 17 `psql` from an approved administration host with a
mode-0600 password file and `pg_service.conf`. Its service contains the actual
host/port/admin/database, `sslmode=verify-full` and its host CA path. No password
or credential URL appears in these commands. A private local server socket is
also acceptable for the server owner. The provider's password ceremony must
prevent plaintext credentials from being captured in SQL/client logs.

Create the empty roles and database once:

```sh
psql 'service=synveda-provisioning' -X -v ON_ERROR_STOP=1 <<'SQL'
CREATE ROLE synveda_app NOLOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE synveda_migrator LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE synveda_gateway LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE synveda_worker LOGIN INHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
GRANT synveda_app TO synveda_gateway, synveda_worker WITH ADMIN FALSE, INHERIT TRUE, SET TRUE;
CREATE DATABASE synveda OWNER synveda_migrator TEMPLATE template0 ENCODING 'UTF8';
REVOKE CONNECT, TEMPORARY ON DATABASE postgres, template1 FROM PUBLIC;
SET ROLE synveda_migrator;
REVOKE ALL ON DATABASE synveda FROM PUBLIC;
GRANT CREATE, CONNECT, TEMPORARY ON DATABASE synveda TO synveda_migrator;
GRANT CONNECT ON DATABASE synveda TO synveda_gateway, synveda_worker, postgres;
RESET ROLE;
\connect synveda
ALTER SCHEMA public OWNER TO synveda_migrator;
CREATE EXTENSION btree_gin WITH SCHEMA public VERSION '1.3';
CREATE EXTENSION vector WITH SCHEMA public VERSION '0.8.6';
SET ROLE synveda_migrator;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO synveda_app;
RESET ROLE;
SQL
```

Success is no SQL error on an empty target. A partial creation or pre-existing
role stops. Preserve its credentials/catalogue; the DBA inspects the exact
created objects and completes the missing grants/extension steps deliberately.
Do not drop a role/database to make this block succeed. The application
migration grants its own table/function privileges and forced RLS; the DBA must
not invent blanket table grants or a default-privilege bypass.

In a private interactive `psql 'service=synveda-provisioning' -X` session,
set three distinct passwords using `\password synveda_migrator`,
`\password synveda_gateway` and `\password synveda_worker`. Record them once in
protected files or the organisation's secret manager and hand off complete
SQLx URLs under three different Secret names, plus the CA. Repeat installation
with those same credentials. PostgreSQL's native `\password` input avoids a
plaintext password in arguments/history; the DBA still owns server logging and
credential custody. Never grant `pg_read_all_data` or `pg_write_all_data`.

The DBA can inspect these non-secret facts before handoff:

```sh
psql 'service=synveda-provisioning' -X -v ON_ERROR_STOP=1 <<'SQL'
SELECT version();
SELECT rolname,rolcanlogin,rolsuper,rolcreatedb,rolcreaterole,rolreplication,rolbypassrls
FROM pg_roles WHERE rolname IN ('synveda_app','synveda_migrator','synveda_gateway','synveda_worker');
SELECT member.rolname, granted.rolname, m.admin_option,m.inherit_option,m.set_option
FROM pg_auth_members m JOIN pg_roles member ON member.oid=m.member
JOIN pg_roles granted ON granted.oid=m.roleid
WHERE member.rolname LIKE 'synveda_%' OR granted.rolname LIKE 'synveda_%';
\connect synveda
SELECT e.extname,e.extversion,n.nspname,pg_get_userbyid(e.extowner) AS owner
FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace ORDER BY e.extname;
SELECT r, d, has_database_privilege(r,d,'CONNECT') FROM
unnest(ARRAY['synveda_gateway','synveda_worker','synveda_app','synveda_migrator']) r
CROSS JOIN unnest(ARRAY['synveda','postgres','template1']) d;
SQL
```

Gateway/worker should CONNECT to `synveda` only. The NOLOGIN capability role
need not have direct CONNECT. These inspection queries are a handoff check,
not a replacement for Synveda's exact authority proof.

After protected Secrets and normal offline/API preflight pass, the installer
runs the explicit **temporary in-cluster Job** on the workstation:

```sh
node "$CHART/examples/operator.mjs" database-probe --prepared "$PREPARED" --architecture "$ARCHITECTURE"
```

Run the exact context/namespace-pinned `kubectl wait` command it prints and
require Complete before Helm installation. The Job uses the chart's existing
`database-preflight` product container with only three ordinary credential/CA
mounts and the expected role document. It checks pod connectivity, verified TLS,
all three principals, the same writable database generation and the role/target
authority contract. It does not prove the complete extension catalogue;
migration performs that proof before application DDL. It performs no
administrator bootstrap or DDL. Job/ConfigMap creation
is a Kubernetes mutation, not a read-only API check. Default request is
100m/128Mi with a 1Gi memory limit and a 120-second deadline. The selected
network policy and provider ingress must admit that probe exactly as they admit
installation. Temporary Job history expires; delete its printed ConfigMap only
after inspection. On failure, give the DBA the bounded content-free error,
correct the declared contract/CA/network input, and create a fresh probe.

A managed provider that refuses the cluster identity/catalogue proof remains
unqualified even if those inspection queries or `SELECT 1` pass. Ordinary Helm
installation repeats the same verifier before migration. The
[full role reference](../../compose/postgres/synveda-database-bootstrap) explains
catalogue edge cases; the human recipe does not require reading it.

Bundled identity on an external server also needs a separate `keycloak` owner
and database, with no access to Synveda. On a new dedicated identity target the
DBA creates an ordinary `keycloak` LOGIN role with the same narrowing flags,
a `keycloak` database/public schema owned by it, revokes PUBLIC database/schema
access, and sets its unique password privately with `\password keycloak`.
It receives no `synveda_app` membership. If co-located, both sides must deny
effective CONNECT into the other's database and the application contract must
list that existing peer/forbidden database. The identity owner supplies its
own host/CA/password Secret and recovery evidence; the chart never provisions
an external identity database.

## External OIDC worksheet

The identity owner fills these values before installing. This is the existing
authorization-code/public-client contract; Synveda uses verified provider
subjects, not email as authority.

| Field | Required value/check |
| --- | --- |
| Canonical issuer | Exact HTTPS issuer returned by discovery and signed tokens, without a trailing slash; preserve across recovery |
| Public login client | Actual `client_id`, authorization code enabled, S256 PKCE mandatory; no browser/client secret or implicit/password grant |
| Callback and web origin | Exactly `$APP_URL/auth/callback` and `$APP_URL`; no wildcard redirects |
| API audience | Distinct `audience` included in the signed access token, for example `synveda-api`; optional service audiences are a separate service identity contract |
| Signing and claims | `algorithms: ["RS256"]`; `openid profile email` scopes; stable `sub`, groups array at the selected `groups_claim` |
| Tenant binding | One stable UUIDv7 stored once in `tenant.static.tenant_id`, identical to `install.tenant.id`; generate below rather than copy a fixture identity |
| First administrator | Assign only the selected stable human subject the exact `synveda-admins` group; verify durable Synveda authority and remove temporary membership as in FIRST_LOGIN |
| Trust and reachability | HTTPS DNS/chain/SAN trusted by browsers, CLI and gateway Pods; private provider CA Secret if needed; optional explicit backchannel cannot downgrade HTTPS or change issuer/endpoint origins |

On the workstation, generate the tenant identifier once into a private file:

```sh
export ISSUER_DIR="$HOME/.synveda-issuer"
(
  set -eu
  umask 077
  test -d "$ISSUER_DIR" || mkdir -m 700 "$ISSUER_DIR"
  node --input-type=module <<'NODE'
import { existsSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
const { uuid7, atomicPrivate, privatePath } = await import(pathToFileURL(process.env.CHART + '/examples/prepare.mjs'));
privatePath(process.env.ISSUER_DIR, true);
const path = process.env.ISSUER_DIR + '/tenant-id';
if (!existsSync(path)) atomicPrivate(path, uuid7() + '\n');
privatePath(path);
if (!/^[a-f0-9-]{36}\n$/.test(readFileSync(path,'utf8'))) throw Error('tenant file incomplete; restore original');
console.log('Stable tenant identifier ready. Read it into the private issuer document.');
NODE
)
```

Create mode-0600 `$ISSUER_DIR/issuers.json`, replacing the issuer, client,
audience and the generated `tenant-id` value once. Keep the file private even
though these fields are non-secret trust configuration:

```json
[{
  "issuer": "https://identity.example.com/realms/team",
  "client_id": "synveda",
  "audience": "synveda-api",
  "algorithms": ["RS256"],
  "groups_claim": "groups",
  "login_scopes": ["openid", "profile", "email"],
  "tenant": {"static": {"tenant_id": "VALUE_FROM_PRIVATE_TENANT_ID_FILE"}}
}]
```

Use `--identity external --issuer-file "$ISSUER_DIR/issuers.json"` with
preparation. It reuses that tenant binding; a conflicting explicit tenant ID
is refused. Add `oidc.caExistingSecret` in ordinary values only when its root
must augment platform trust. Provider owners validate discovery, callback and
claims privately; never paste tokens or cookies into a shared report. API
preflight checks the Secret's exact issuer/static binding. A fresh real browser
login and `synveda whoami --capabilities` establish human admission. A
workstation discovery response cannot establish Pod connectivity.

# Persistent small-team starter

[starter-values.yaml](starter-values.yaml) enables one PostgreSQL instance and
one Keycloak instance in the existing Synveda release. Gateway, console, worker,
Cedar, forced RLS and the PostgreSQL job engine are unchanged. This is a
single-instance installation with maintenance downtime. It needs backups and
operator custody of its encryption keys; it is not HA or disaster recovery.

## Choose the providers

| PostgreSQL | Identity | Values and ownership |
|---|---|---|
| Packaged | Packaged | Starter preset: existing CNPG operator manages one server; Synveda and Keycloak use separate databases/users |
| External | Packaged | Start with `ci/external-values.yaml`, enable the `keycloak` settings below; the database administrator provisions both databases |
| Packaged | External | Starter PostgreSQL settings, `keycloak.enabled: false`; supply the external issuer Secret and remove unused Keycloak settings |
| External | External | Existing `ci/external-values.yaml`; no stateful provider belongs to the application release |

These selections describe ownership, not data migration. Switching them does
not copy databases, identities or encryption keys. A move requires a separate
backup/restore and identity-continuity plan. Keep the canonical issuer unchanged
when moving the same Keycloak realm; changing the issuer in an existing tenant
is unsupported pending MEM-7.

The chart installs **no operator**. Packaged PostgreSQL requires the existing
CloudNativePG API/operator, separately maintained by the cluster administrator.
The starter acceptance pins CNPG 1.30.0. Its Cluster belongs to this Helm release
but has `helm.sh/resource-policy: keep`. The optional Keycloak StatefulSet is
the unmodified upstream codecentric `keycloakx` **7.3.2** dependency, pinned in
[Chart.lock](Chart.lock) and vendored under `charts/`. No wrapper or second
application chart is involved.

### Distribution and version boundary

Verified on 2026-09-19:

| Dependency | Selected version and evidence |
|---|---|
| Keycloak chart | [codecentric 7.3.2](https://github.com/codecentric/helm-charts/releases/tag/keycloakx-7.3.2), released 2026-09-17; downloaded without credentials; archive SHA-256 `4ebb630c1f842245f7953be4f80ed080d1979ded25b31d48052f43f012e3c1d7`; Apache-2.0 license retained in `third-party/` |
| Keycloak image | Existing [optimized Dockerfile](../../compose/keycloak/Dockerfile), official `quay.io/keycloak/keycloak:26.7.2` pinned by digest; public distribution verified; Apache-2.0. Chart 7.3.2 advertises 26.7.4, but this preset explicitly retains 26.7.2. Qualifying the newer provider patch is a separate maintenance action. |
| PostgreSQL | Existing [CNPG image](../postgres/Dockerfile): official pinned PostgreSQL 17.11 plus pgvector 0.8.6; PostgreSQL licence, with Apache-2.0 CNPG components. The existing Compose database image remains unchanged. |

Keycloak [supports PostgreSQL 14–18](https://www.keycloak.org/server/db);
Synveda's qualified intersection here is **17.11**, with `vector` 0.8.6,
`btree_gin` 1.3 and `plpgsql` 1.0 in the Synveda database. Keycloak uses its own
`keycloak` database/owner without those product extensions. Sharing one server
does not share credentials, schema ownership or application access. Runtime
Synveda roles cannot connect to `keycloak`; Keycloak cannot connect to Synveda
or maintenance databases. The existing bootstrap and runtime verifier enforce
these boundaries.

Synveda's example GHCR image coordinates are a release contract, not a claim
that a tested public release is available. Build from this checkout and publish
to your registry, or independently pull-verify the complete candidate artifact
set. Save exact tags/digests in your operator values:

```sh
docker build -t YOUR_REGISTRY/synveda:YOUR_REVISION -f deploy/compose/product/Dockerfile .
docker build -t YOUR_REGISTRY/cnpg-postgres:17.11-YOUR_REVISION -f deploy/helm/postgres/Dockerfile .
docker build -t YOUR_REGISTRY/keycloak:26.7.2-YOUR_REVISION -f deploy/compose/keycloak/Dockerfile .
helm dependency build deploy/helm/synveda
```

For private registries, create namespace-local registry Secrets and set
`imagePullSecrets` for product/install/CNPG images, plus
`keycloak.imagePullSecrets` for the upstream identity pod. Each is a list of
`{name: YOUR_REGISTRY_SECRET}` references. Registry pull verification remains
part of qualifying your published artifact set; Kind acceptance loads local
source-built images.

## Prepare one installation

1. Provision the namespace, CNPG operator if selected, persistent StorageClass,
   private cluster networking and an HTTPS ingress/proxy. No ingress controller,
   certificate issuer or monitoring stack is installed by this chart.
2. Copy the preset to your private operator configuration. Set the product and
   provider images, `install.tenant.id` to a preselected UUIDv7, tenant `slug`
   and `name`, application/identity HTTPS origins, actual TLS Secrets and ingress
   class. Use that tenant ID and exact issuer in the issuer Secret below.
3. Set `postgres.storage.storageClass` and `size` (20 GiB initially). CNPG uses
   filesystem volumes with `ReadWriteOnce`; choose a CSI driver that supports
   PostgreSQL fsync and the required ownership/mount permissions. No team
   template uses `hostPath`. Expansion depends on the StorageClass; shrinking
   and arbitrary existing-PVC adoption are not supported by this preset. Use
   the operator's documented recovery procedure for existing database data.
4. Set `keycloak.proxyTrustedAddresses` to the actual ingress source addresses
   or bounded CIDRs. Leave the Service private. Only `/realms/synveda` and
   `/resources` are public; master realm, `/admin`, health and metrics remain
   private. Backend HTTP requires a trusted private network; this chart does
   not yet supply NetworkPolicy or pod-to-pod TLS.

Generate distinct private credentials; never paste their values into Helm
values, source control or shell arguments. For the shared CNPG preset:

```sh
umask 077
mkdir private
printf '%s\n' 'local:team-primary' > private/kms-key-ref
python3 - <<'PY'
from pathlib import Path
from secrets import token_hex
for name in ('gateway-password', 'worker-password', 'keycloak-password',
             'keycloak-admin-password', 'kms-key'):
    Path(f'private/{name}').write_text(token_hex(32))
for role in ('gateway', 'worker'):
    password = Path(f'private/{role}-password').read_text().strip()
    Path(f'private/{role}-url').write_text(
        f'postgresql://synveda_{role}:{password}@synveda-pg-rw:5432/synveda\n')
PY
```

Create `private/keycloak-admin-username` with your operator-selected temporary
Keycloak bootstrap administrator name, without a trailing newline. Keycloak's
native Secret environment values are exact bytes; its username and passwords
must not contain line endings. No username or password is supplied by the
preset. Create `private/issuers.json`, replacing both the URL and tenant ID:

```json
[{
  "issuer": "https://auth.example.com/realms/synveda",
  "client_id": "synveda",
  "audience": "synveda-api",
  "service_audiences": ["synveda-agents"],
  "algorithms": ["RS256"],
  "groups_claim": "groups",
  "tenant": {"static": {"tenant_id": "YOUR-PRESELECTED-UUIDV7"}}
}]
```

The starter has one issuer and one organisation. A verified stable subject
identifies a principal within that issuer's tenant. Admission rejects two
issuers bound to the same tenant, and rejects mixing a claim-bound issuer with
another issuer. This is the narrower safe admission contract, **not** persistent
federated identity linking. The database still keys subjects within a tenant;
operators must not replace its issuer or enable email-based directory adoption.
Emails and display names never grant team access. Full issuer migration/linking
and directory reconciliation remain MEM-7 work.

Create operator-owned Secrets (namespace `synveda`, release `synveda`):

```sh
kubectl -n synveda create secret generic synveda-gateway-db --from-file=DATABASE_URL=private/gateway-url --from-file=password=private/gateway-password
kubectl -n synveda create secret generic synveda-worker-db --from-file=DATABASE_URL=private/worker-url --from-file=password=private/worker-password
kubectl -n synveda create secret generic synveda-keycloak-db --from-file=password=private/keycloak-password
kubectl -n synveda create secret generic synveda-keycloak-admin --from-file=username=private/keycloak-admin-username --from-file=password=private/keycloak-admin-password
kubectl -n synveda create secret generic synveda-oidc --from-file=SYNVEDA_OIDC_ISSUERS=private/issuers.json
kubectl -n synveda create secret generic synveda-kms --from-file=SYNVEDA_KMS_KEY=private/kms-key --from-file=SYNVEDA_KMS_KEY_REF=private/kms-key-ref
```

CNPG generates the separate migrator/superuser Secrets and database CA. The
installation Job copies only its required bootstrap files to private memory,
converges both databases with the existing bootstrap, proves the peer database,
then runs ordinary-role preflight/migration and tenant admission. Gateway and
worker receive neither bootstrap nor Keycloak credentials. Keycloak receives
only its own database and initial administrator Secret references. Kubernetes
Secret encryption at rest and namespace RBAC remain operator responsibilities.

For **external PostgreSQL with packaged Keycloak**, use the external values
specimen rather than merging CNPG-only settings. Provision `keycloak` with its
separate owner/password through the database administrator; configure
`keycloak.database.hostname`, `port`, `existingSecret` and
`databaseCaExistingSecret`. JDBC requires `verify-full` and a hostname-valid
certificate. Keep its database/user exactly `keycloak`. If both applications
share the external server, declare `keycloak` in Synveda's
`postgres.external.roles.forbidden_databases` and `isolated_peer_roles`.
Synveda's migrator/gateway/worker URLs and CA follow the
[external database contract](README.md#external-postgresql).

Install from the saved values; Helm must wait for the ordinary install Job:

```sh
helm lint deploy/helm/synveda --strict -f team-values.yaml
helm template synveda deploy/helm/synveda -n synveda -f team-values.yaml --api-versions postgresql.cnpg.io/v1 > rendered.yaml
helm upgrade --install synveda deploy/helm/synveda -n synveda -f team-values.yaml --wait --wait-for-jobs --timeout 15m
kubectl -n synveda get pods,pvc
```

Keycloak uses production `start --optimized --cache=local --import-realm`.
Native import creates the realm/client/group only when the realm is absent;
it skips an existing realm and never replaces team users on upgrade. Registration
and email reset are disabled. No sample content or team accounts are imported.
Make later realm changes through Keycloak's administration tools.

## First use and membership

Use the [Keycloak Admin CLI](https://www.keycloak.org/docs/latest/server_admin/#admin-cli)
over private pod access, or your organisation's restricted administration path.
For example, select the Keycloak pod and authenticate interactively:

```sh
kubectl -n synveda get pods -l app.kubernetes.io/name=keycloakx
kubectl -n synveda exec -it KEYCLOAK_POD -- /opt/keycloak/bin/kcadm.sh config credentials --config /tmp/team-admin.config --server http://localhost:8080 --realm master --user YOUR_BOOTSTRAP_ADMIN
```

The CLI prompts for the administrator password. Its temporary config contains
credentials: keep it private and delete it after administration. Follow
Keycloak's [bootstrap administrator recovery guidance](https://www.keycloak.org/server/bootstrap-admin-recovery)
to establish durable private administration and remove the temporary bootstrap
account. The Secret is an initial creation input, not a password rotation API.

Create an operator-selected human in realm `synveda`, with a privately delivered
temporary password and required password change, or configure your existing
trusted identity federation. Assign **only that selected initial owner** to the
existing `synveda-admins` group. The first verified login carrying that group
receives the existing one-time tenant administrator grant; an unauthenticated
visitor or ordinary OIDC user cannot claim it. The durable bootstrap marker
prevents later group additions or re-login from recreating revoked authority.
Remove the bootstrap group assignment after confirming the Synveda grant.

| Team term | Existing Synveda role | Meaning |
|---|---|---|
| Initial organisation administrator | `administrator` at tenant root | Configure the installation's tenant and manage governed access |
| Workspace owner | `owner` | Creator owns the workspace; scope descendants inherit its grant |
| Team administrator | `administrator` at workspace | Manage that team's access, subject to Cedar policy |
| Member | `member` | Work with Sessions, context and governed publication at granted scope |
| Read-only member | `viewer` | Read allowed content; publication and membership changes are denied |

1. Sign in at the application `/console/` as the selected owner. In Configuration,
   select **Organisation**, create from the **team** template and bind it at
   the tenant root. Then create a workspace and project.
   Descendants inherit this organisation configuration; membership remains
   scoped. Policy and any required later reviews remain active.
2. Create the second person's identity in Keycloak without the bootstrap group.
   In the existing **People** screen, create a bounded member invitation and
   privately share its one-time link. They sign in as themselves and accept.
   SMTP is unnecessary. An invitation is a bearer admission secret; its optional
   email is descriptive, not proof of the accepting identity. For exact-subject
   admission, use People/direct grants with the verified principal subject.
3. Publish a Knowledge convention in the project, for example an explicitly
   opt-in test release schedule. Complete any required VedaFlow review. A
   second user can read it only after admission. Ordinary authentication may
   create that person's own scope; it does not reveal your team's content.
4. Connect the existing supported CLI/MCP integration using that user's own
   `synveda login`, or the scoped service credential below. Open a Session in
   this project and use MCP `recall` to retrieve the published convention.
5. In People, remove the member grant. To change a role, revoke its exact grant
   and grant the replacement role; review inherited/group grants too. Repeat
   context/Knowledge/search/Skills calls with the existing credential. They
   must no longer disclose the removed workspace's content. Already delivered
   content cannot be recalled. The live fixture exercises this same API path.

The public equivalents are `POST /v1/workspaces/{id}/invites`, authenticated
`POST /v1/invites/{token}/accept`, `POST /v1/admin/grants`, and
`DELETE /v1/admin/grants/{id}`. Writes that declare `Idempotency-Key` require it.
No SQL, email-domain rule or second role vocabulary is involved.

### Agent credentials and revocation

Use an existing OIDC confidential service client with client-credentials enabled,
interactive/direct-password flows disabled, exact `synveda-agents` audience and
**300-second** access-token lifetime. Keep the Keycloak `basic` client scope so
tokens contain stable `sub`; do not attach interactive bootstrap groups. Read
its service-account subject through private provider administration. An owner
registers that subject with `POST /v1/service-identities` and the workspace
`scope_id`, then grants only the needed existing role at the project scope.
The placement vocabulary permits a service principal beneath a workspace,
tenant or org unit, not beneath a project. Registration
alone is not a team role. Synveda rejects an unregistered service subject.

Obtain its short-lived token from the issuer token endpoint using
`grant_type=client_credentials`, keeping the client secret in a private file or
secret store. Use the existing CLI `SYNVEDA_TOKEN` input, set by a private process
launcher, with `SYNVEDA_GATEWAY` and `synveda mcp --session SESSION_ID`. See
[client support](../../../docs/CLIENT_SUPPORT.md) for supported integration
claims. Never give an agent the owner's browser token. The token's registered
workspace confines all requests even if a caller supplies another workspace ID;
the separate project grant determines authority within that boundary.

Delete the service registration with `DELETE /v1/service-identities/{id}` to
deny subsequent authenticated requests, then disable/rotate its IdP client.
Revoking the Synveda member grant or service registration is checked on the
next governed request in the one gateway. In contrast, IdP disable/logout alone
does not invalidate an already issued JWT: budget its remaining lifetime plus
the validator's **30-second** clock leeway (up to 330 seconds here). There is no
claim of immediate global JWT revocation or recall of in-flight responses.
Console/CLI refresh and account-session inventory have their separate AUTH-6
qualification gaps.

## Maintenance, backup and retained reinstall

Save the complete values and image digests. The starter pins PostgreSQL and
Keycloak images independently of the application version. Keycloak uses
`OnDelete`: a changed StatefulSet template does not restart the existing pod.
For one PostgreSQL instance CNPG requires `unsupervised`; changing its image
starts maintenance automatically. Never change that value/image as a side
effect of an application-only upgrade. Review `helm diff` or rendered changes,
back up first, then explicitly schedule provider changes and Keycloak restart.
Do not assume application rollback can undo provider/schema migration.

Before a provider upgrade or destructive operation, stop new application work,
quiesce gateway/worker and Keycloak, and take consistent backups of **both**
databases using PostgreSQL/CNPG tools. Separately escrow the KMS key/reference,
operator database/IdP Secrets, exact issuer/realm configuration, saved values and
artifact digests. Encrypt archives, transfer them off-host and restrict access.
Retaining plaintext logical archives on the same node is not a backup policy.
Restore to an isolated target with the original keys and issuer, then verify
login, memberships, Knowledge, audit and key decryption before promotion.
Use [CNPG recovery documentation](https://cloudnative-pg.io/docs/1.30/recovery/)
for physical recovery/PITR design. This slice does not implement or qualify a
scheduled backup, PITR, joint restore drill, RPO or RTO; OPS-5/OPS-6 remain open.

For an intentional **retained reinstall** in the same namespace:

```sh
kubectl -n synveda get cluster synveda-pg
kubectl -n synveda get pvc
helm uninstall synveda -n synveda --wait
# Keep the namespace, retained Cluster/PVCs, CNPG-generated credentials and
# every operator-owned Secret. The PostgreSQL instance remains running.
helm upgrade --install synveda deploy/helm/synveda -n synveda -f team-values.yaml --wait --wait-for-jobs --timeout 15m
```

Use the same release/namespace, tenant ID, issuer, provider images and Secrets.
The retained Cluster keeps its Helm ownership annotations; do not adopt it into
a different release by editing them. Keycloak's recreated pod reads the same
database, and the normal install Job re-proves authority without regenerating
credentials. Deleting the namespace or Cluster can delete operator-owned PVCs;
the StorageClass/PV reclaim policy then determines whether backing storage is
destroyed. `Retain` at the PV layer still needs manual recovery and key custody.
Do not use namespace deletion or a reset command as an uninstall procedure for
team data.

## Acceptance and remaining qualification

Run the opt-in matrix in a disposable Kind cluster with Docker, Kind, Helm,
kubectl, Node, Ruby and OpenSSL available:

```sh
make chart-lint
STARTER_MATRIX=1 bash demos/ops-2-helm-install.sh
```

The fixture builds source images, uses an isolated kubeconfig, HTTPS/private CA,
real PKCE, private Keycloak administration, ordinary product APIs and the real
CLI MCP server. Each of the four combinations tests bootstrap, invitation,
viewer/stranger/foreign-workspace denial, governed publication, service expiry
settings, four concurrent agents (400 events and 80 context runs), worker
Capture, pod recreation, same-source upgrade, retained reinstall, stable
credentials and unchanged-token revocation. It writes content-free observed
CPU/memory peaks to `demos/evidence/ops11-starter.json` only after success.
`STARTER_CASE` selects one named combination; `KEEP=1` retains private diagnostic
state. `REUSE=1` may use a selected empty fixture cluster; existing namespaces
are refused. `SKIP_BUILD=1` is only for images built from the exact current source.

Initial requests are starting points: gateway 500m/512 MiB, worker 250m/256 MiB,
Keycloak 500m/768 MiB, plus PostgreSQL, operator and Kubernetes overhead. Select
database resources for your workload; the fixture's small database request is
250m/512 MiB. The deterministic lexical/extraction workload does not qualify
semantic model inference, burst capacity, long-running ingestion or retention.

OpenShift still needs a real restricted-SCC run: assigned UID/fsGroup, image and
secret permissions, CNPG operator compatibility, CSI mount/expand/reclaim,
private admin access, ingress/Route TLS and proxy addresses, network boundaries,
registry access, restart/upgrade and joint restore. No SCC exception, HA,
managed-cloud compatibility or production readiness follows from Kind startup.
See [OPS-11](../../../docs/backlog/OPS-11.md) for measured results and exact
outstanding work.

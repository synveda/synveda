# Deploy to Kubernetes

The existing chart runs one gateway/console and one private worker. Select
bundled or existing application PostgreSQL independently of bundled Keycloak
or existing OIDC. Bundled PostgreSQL is a persistent namespaced StatefulSet;
CNPG is an explicit alternative requiring an operator you already manage.
All modes preserve Cedar, forced RLS, VedaFlow, audit and the migration contract.

<!-- installation-version: 0.4.0; publication: unreleased -->
**0.4.0 is unreleased.** The v0.3.0 build is separate and does not contain the
installation changes documented here. Download/pull commands are pending
qualification and publication of the exact matching artifacts.

Start with the [complete loopback evaluation recipe](examples/README.md).
It prepares private Secrets in a short-lived container, installs no cluster-wide
infrastructure, and exposes the console through explicit loopback port-forwards.
For your existing infrastructure use the configuration below.

## Dependency ownership

| Application database | Identity | Starting values |
|---|---|---|
| Bundled | Bundled Keycloak | [local recipe](examples/README.md) or [bundled DB](ci/bundled-values.yaml) + [Keycloak](examples/bundled-keycloak.yaml) |
| Existing | Existing OIDC | [external](ci/external-values.yaml) |
| Existing | Bundled Keycloak | [external](ci/external-values.yaml) + [separate identity DB](examples/external-database-keycloak.yaml) |
| Bundled | Existing OIDC | [bundled DB](ci/bundled-values.yaml) |

Read [configuration](CONFIGURATION.md) for exact database privileges, issuer
and Secret formats; [operations](OPERATIONS.md) for maintenance and recovery;
[portability](PORTABILITY.md) for existing ingress, Gateway API and OpenShift
configuration. Generic OIDC compatibility is not verified support for every IdP.

## Cluster administrator prerequisites

The administrator supplies a namespace, installer RBAC, quotas, DNS, trusted TLS,
private networking and registry access. Bundled PostgreSQL needs persistent storage. Only `postgres.mode=cnpg` needs
a preinstalled CNPG operator. Loopback evaluation needs neither DNS nor ingress TLS. The chart creates no CRD/operator, ingress
controller, certificate issuer, storage class or monitoring stack.

The candidate qualification uses Kind 0.32.0, Kubernetes and kubectl 1.36.1,
Helm 4.2.3, PostgreSQL 17.11 with vector 0.8.6 and btree_gin 1.3, and Keycloak
26.7.2 via locked keycloakx 7.3.2. Earlier explicit CNPG evidence used operator
1.30.0; the bundled candidate installs no operator. Containers are Linux arm64
on macOS/OrbStack. The four-mode fixture uses private-CA HTTPS; the local recipe
uses loopback port-forwarding. Real ingress,
OpenShift, cloud services and a general Kubernetes minor-version window remain
unqualified. Structural API validation is distinct from execution evidence.

- Create a project/namespace and grant its installer namespaced access to
  Deployments, StatefulSets, Jobs, Pods/log/exec, Services, ConfigMaps, Secrets,
  ServiceAccounts, PVCs and the selected Ingress/Route/NetworkPolicy resources.
  CNPG mode additionally requires access to its namespaced Cluster resource.
- Set quota for database/provider pods plus gateway, worker and the temporary
  installation Job; allow replacement pods/PVC provisioning. See resource
  observations in [OPS-11](../../../docs/backlog/OPS-11.md). The tested engine
  exposed 18 CPUs and 16.8 GB RAM; that is test context, not a minimum.
- Select a persistent CSI StorageClass supporting PostgreSQL fsync and the
  cluster's UID/fsGroup rules. Record expansion and PV reclaim policies.
  Retained PVCs are not a backup. External mode without TEI needs no app PVC.
- Provision two DNS names and certificates for packaged identity, or one app
  name plus the organisation's canonical issuer. Both pods and browsers must
  resolve and trust the same issuer. Keep management, master realm and metrics
  private. Supply the existing ingress class or Route integration explicitly.
- Allow DNS, PostgreSQL, identity and optional approved model/telemetry egress.
  NetworkPolicy needs an enforcing CNI and real selectors/CIDRs; see the small
  [generic/cloud](examples/managed-kubernetes.yaml),
  [OpenShift](examples/openshift.yaml) and [network](examples/network-policy.yaml)
  overlays. They validate renders only until run on the named target.

## Namespace installer and artifact acquisition

Current starter requests are gateway **500m / 512Mi**, worker **250m / 256Mi**,
CNPG **1 CPU / 2Gi** and Keycloak **500m / 1280Mi**, plus installation jobs and
cluster/ingress overhead. The four-agent short workload observed Keycloak near
1Gi; its request was raised from 768Mi while retaining the 2Gi limit. These are
initial reservations, not throughput or capacity guarantees; see the measured
[starter](../../../demos/evidence/ops11-operations-cnpg-packaged.json) and
[external](../../../demos/evidence/ops11-operations-external-external.json) reports.

Use Helm 4.2.3 and a namespace-scoped kubeconfig supplied by the administrator.
These checks print capability/status information, not credentials:

```sh
export NAMESPACE=synveda
kubectl config current-context
kubectl auth can-i create deployments -n "$NAMESPACE"
kubectl auth can-i create secrets -n "$NAMESPACE"
kubectl auth can-i create jobs -n "$NAMESPACE"
kubectl -n "$NAMESPACE" get resourcequota,limitrange
kubectl get storageclass
```

Obtain the **candidate's** chart archive, SHA256SUMS, image overlays and CLI/
plugin archives from the release owner. Verify their checksums before unpacking;
checksums establish byte integrity, not publisher authenticity. BuildKit image
provenance/SBOM generation is prepared, not a verified signing service. No Rust
compiler, Dockerfile inspection or source edits are part of installation.

The following commands require those files in the current directory and an
owner-selected candidate version. They are not a claim that such a public tag
exists today:

```sh
: "${RELEASE_VERSION:?set the candidate version supplied by the release owner}"
sha256sum --check SHA256SUMS
mkdir chart
# The archive contains synveda/ and its unchanged locked Keycloak dependency.
tar -xzf "synveda-$RELEASE_VERSION.tgz" -C chart
export CHART="$PWD/chart/synveda"
cp "synveda-images-$RELEASE_VERSION.yaml" release-images.yaml
```

On macOS, use `shasum -a 256 -c SHA256SUMS`. Save the verified archive, overlays,
checksums and source revision with operator configuration. The publishing
workflow additionally pushes/pulls the chart at
`oci://ghcr.io/synveda/charts/synveda`; use that address only after a verified
publication. Private image registries need existing namespace-local pull
Secrets in `imagePullSecrets` and, for packaged identity,
`keycloak.imagePullSecrets`. Create them through your secret manager or
`kubectl create secret generic registry-access --type=kubernetes.io/dockerconfigjson
--from-file=.dockerconfigjson=private/registry-config.json -n "$NAMESPACE"`;
do not pass registry passwords in shell arguments.

## Choose the preset

| Database / identity | Starting values | Ownership |
|---|---|---|
| External / external | `ci/external-values.yaml` | Database and identity administrators own both services and their recovery |
| CNPG / packaged | `starter-values.yaml` | Existing operator manages one persistent server and two isolated databases |
| External / packaged | External values plus `keycloak` from starter | DBA provisions both databases; application release owns Keycloak |
| CNPG / external | Starter with `keycloak.enabled: false` | CNPG owns product DB; identity administrator owns identities |

Copy the selected file from `$CHART` to `team-values.yaml`. Replace every example
host, tenant and Secret reference. For CNPG only, append the candidate's
`synveda-cnpg-image-$RELEASE_VERSION.yaml` to `release-images.yaml` (its top-level
`postgres` key is distinct). Keep database and identity image digests unchanged
on later application-only upgrades. For external services, complete
[database provisioning](CONFIGURATION.md#external-postgresql) and
[OIDC setup](CONFIGURATION.md#identity-and-secret-formats) first; the application
never provisions or repairs an independently operated provider.

For the starter, prepare the two non-secret values files with:

```sh
cp "$CHART/starter-values.yaml" team-values.yaml
cat "synveda-cnpg-image-$RELEASE_VERSION.yaml" >> release-images.yaml
```

For external services, instead copy `$CHART/ci/external-values.yaml` and keep
the product image overlay without the CNPG addition. The commands below use
release and namespace `synveda`; keep those names consistent in values, Secrets
and administrative checks.

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
   private. Backend HTTP requires a trusted private network; this chart offers opt-in NetworkPolicy; it does not supply pod-to-pod TLS.

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
[external database contract](CONFIGURATION.md#external-postgresql).

Install from the saved values; Helm must wait for the ordinary install Job:

```sh
helm lint "$CHART" --strict -f team-values.yaml -f release-images.yaml
helm template synveda "$CHART" -n synveda -f team-values.yaml -f release-images.yaml --api-versions postgresql.cnpg.io/v1 > rendered.yaml
helm upgrade --install synveda "$CHART" -n synveda -f team-values.yaml -f release-images.yaml --wait --wait-for-jobs --timeout 15m
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

## Verification and measured commands

After Helm completes, inspect statuses and the revision-scoped installation
Job; `helm history synveda -n synveda` gives its revision. Normal migration is
bounded, advisory-locked and repeatable. `/healthz` reports the process;
`/readyz` reports mandatory database/schema/role authority. Optional provider
outages do not become liveness failures. Health and metrics stay off public
Ingress/Route paths. The [operations runbook](OPERATIONS.md) has private checks.

The following exact source-fixture command is executable with Docker, Kind,
Helm, kubectl, Node 22+, Ruby, Python 3 and OpenSSL. It creates its own clean
cluster and namespaces, generates synthetic credentials, runs the real chart,
then removes only its test resources. It requires no team kubeconfig or secrets:

```sh
BUNDLED_MATRIX=1 PORTABILITY=1 OPERATIONS=1 STARTER_MATRIX=1 bash demos/ops-2-helm-install.sh
OPERATIONS=1 STARTER_MATRIX=1 STARTER_CASE=cnpg-packaged bash demos/ops-2-helm-install.sh
OPERATIONS=1 STARTER_MATRIX=1 STARTER_CASE=external-external bash demos/ops-2-helm-install.sh
POSTGRES_MODE=external bash demos/ops-2-helm-install.sh
```

The first command exercises all four bundled/external combinations without an
operator. The next two commands package and extract the chart, then exercise
each ownership endpoint through all day-two drills and write a separate report.
The explicit CNPG fixture installs its operator only inside its disposable Kind
cluster; it requires authorisation for that cluster-wide test infrastructure.
Omit `STARTER_CASE` to run both sequentially. The last command retains the database TLS/client-certificate and
OIDC negative matrix. `PORTABILITY=1 STARTER_MATRIX=1` additionally exercises
all four ownership combinations under simulated restricted IDs. These use
local source images, not a verified public release. `KEEP=1` retains diagnostic
state; `REUSE=1` requires empty selected fixture namespaces. Results and exact
executed profiles live in [OPS-11](../../../docs/backlog/OPS-11.md).

Release qualification uses `scripts/qualify-kubernetes-release.mjs` with the
extracted candidate bundle, packaged chart archive and report path. It disables
builds, imports the manifest-bound images and also exercises the shipped local
preparation and browser port-forward recipe. The
[installation record](../../../docs/backlog/CPR-45.md#installation-mission-2026-09-20)
and content-free reports distinguish local candidates from published artifacts.

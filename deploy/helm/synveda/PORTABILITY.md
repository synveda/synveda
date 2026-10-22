# Kubernetes and OpenShift portability (OPS-11)

Use the existing chart and [external](README.md) or [starter](STARTER.md)
installation instructions. These overlays add no operator, ingress controller,
cloud provisioning, service mesh, edition, job engine or tenant-isolation path.
One gateway and one worker remain intentional; upgrades interrupt service.

## Environments and evidence

| Environment | Exact available version / policy | Evidence boundary |
|---|---|---|
| Local bundled candidate | Kind 0.32.0, Kubernetes/kubectl 1.36.1, Helm 4.2.3; Linux arm64 on macOS 26.6.2/OrbStack Engine 29.4.0 | Four bundled/external combinations; restricted local browser fixture assigns UID 1000900000 to product, Jobs, Keycloak **and PostgreSQL**; [candidate report](../../../demos/evidence/ops11-release-candidate.json) |
| Earlier operator starter | Same Kind/server/host, with kubectl 1.33.9/kustomize 5.6.0 | Historical CNPG matrix with PSA `restricted` pinned to `v1.33`; [report](../../../demos/evidence/ops11-portability.json). That kubectl version is outside the supported server skew; new qualification uses 1.36.1. |
| OpenShift 4.19 target | Kubernetes 1.32.0 structural schemas; `restricted-v2` target | Render/schema checked only; no cluster, patch version or admitted SCC available |
| OpenShift 4.20 target | Kubernetes 1.33.0 structural schemas; `restricted-v3` target with `hostUsers: false` | Render/schema checked only; no cluster, patch version, SELinux/user-namespace or storage evidence available |
| Self-managed Kubernetes | The exact local Kind version above | No other distribution, storage or ingress implementation qualified |
| EKS / AKS / GKE | No account, cluster version or provider combination supplied | Common [managed-Kubernetes overlay](examples/managed-kubernetes.yaml) render/schema checked; cloud examples remain unqualified |
| Customer-operated PostgreSQL on Kind | Kind 0.32.0, Kubernetes 1.36.1, kindnet; PGDG PostgreSQL 14.24 + pgvector 0.8.7; Keycloak stand-in on its own PostgreSQL 17.11 | Source-built images in hosted CI; external database and OIDC, DBA setup script, refusals, enforced NetworkPolicy and retained reinstall. See [the enterprise route](#first-enterprise-target-external-postgresql-and-oidc). Not a provider, OpenShift or published-artifact result |

Database admission is a separate [PostgreSQL compatibility contract](../../../docs/DEPLOYMENT_CONTRACT.md#postgresql-compatibility).
Azure Flexible Server's advertised PostgreSQL 17/pgvector 0.8.2 combination
fits a source version profile but is not provider-qualified when the Kubernetes
overlay renders. Released v0.4.4 still refuses 0.8.2. Other managed
database support cannot be inferred from a matching version or cluster brand.

The named OpenShift minors are API/policy targets, not a support promise.
[4.19's SCC documentation](https://docs.redhat.com/en/documentation/openshift_container_platform/4.19/html-single/authentication_and_authorization/index)
describes restricted-v2; [4.20's documentation](https://docs.redhat.com/en/documentation/openshift_container_platform/4.20/html/authentication_and_authorization/managing-pod-security-policies)
adds restricted-v3 and user namespaces. Upgraded clusters and grants can select
different policies. Inspect the actual cluster and admitted pods; never infer
the SCC from the chart or from a simulated UID.

## Platform prerequisites and admission

A platform administrator supplies the project/namespace, quota, working storage,
existing router/Ingress controller, DNS/certificates and an enforcing CNI.
CNPG mode additionally needs the already-installed operator. The team installs
namespace resources and operator-owned Secrets; runtime service accounts have
no cluster RBAC and no mounted API token. Packaged Keycloak requires its
chart-owned token-free ServiceAccount because the upstream PodSpec cannot
override automount for an arbitrary external account. CNPG instance-manager/operator access
is a separate upstream boundary, not an application credential.

Before installing into an **authorised disposable project**, record:

```sh
oc version
oc get clusterversion version -o jsonpath='{.status.desired.version}'
oc get namespace "$PROJECT" -o jsonpath='{.metadata.annotations}'
oc get scc restricted-v2 restricted-v3 -o yaml
```

If SCC inspection is unavailable to the installer, obtain that read-only
evidence from the administrator. Record the UID and supplemental-group ranges,
SCC grants, user-namespace/CSI capability and CNI implementation. No `anyuid`,
privileged SCC, custom broad SCC or root/chown initializer is required or
requested by this chart. Do not change namespace ranges to fit an image.

Layer [examples/openshift.yaml](examples/openshift.yaml) over your completed
values. It requires restricted-v2 on the application, install Job and packaged
Keycloak. `platform: openshift` removes numeric UID/GID/fsGroup from application
and optional TEI contexts; Keycloak already omits them and refuses numeric
overrides in this mode. Admission supplies the identity and volume group.

For **restricted-v3**, the unchanged upstream Keycloak chart has no PodSpec
`hostUsers` setting. Use the included local Helm post-renderer on **every**
install and upgrade. It patches all rendered Pods, Deployments, StatefulSets,
DaemonSets and Jobs, including init containers' enclosing pods; it sets
`hostUsers: false` and requires restricted-v3. Helm 4.2.3 uses a named plugin:

```sh
HELM_PLUGINS="$PWD/deploy/helm/synveda/post-renderers" \
helm upgrade --install synveda deploy/helm/synveda -n "$PROJECT" \
  -f private/team-values.yaml -f deploy/helm/synveda/examples/openshift.yaml \
  --post-renderer synveda-openshift-v3 --wait --wait-for-jobs --timeout 15m
```

`HELM_PLUGINS` above is command-scoped and does not install a global plugin.
For Helm 3, pass the executable
`deploy/helm/synveda/post-renderers/openshift-v3/render.sh` to `--post-renderer`;
that invocation is documented, not locally tested. Both need `kubectl kustomize`
but make no cluster call while rendering. The chart archive includes the files.

The post-renderer cannot patch CNPG's future operator-created pods. CNPG on
OpenShift, especially restricted-v3 with storage/user namespaces, remains an
independent **unqualified combination**. Start OpenShift qualification with
external PostgreSQL. Do not claim full starter support from that result or
work around operator admission using wider SCC grants.

Inspect `openshift.io/scc`, resolved security contexts, `hostUsers` and SELinux
labels for every admitted pod, including install/init and dependency pods.
The required-SCC annotation prevents choosing a broader SCC silently. The
optional TEI overlay should also require the selected SCC in `tei.podAnnotations`;
the v3 renderer applies it automatically.

## Filesystems and dependency boundaries

| Workload | Writes and restrictions |
|---|---|
| Gateway / worker | Same product image; gateway serves immutable console files directly, with no nginx/cache/PID runtime. Image HOME and XDG cache are under `/tmp`, backed by emptyDir. Port 8120; worker health stays loopback 8121. No application PVC. |
| Migration / tenant Job | Same product image and separate database authority stages. Product stages get bounded `/tmp`. CNPG bootstrap uses its existing narrow memory mounts for copied inputs and authority evidence. Private witness directories clear inherited setgid and select the process's own group before the unchanged strict ownership proof. No root startup. |
| Packaged Keycloak 26.7.2 / chart 7.3.2 | Existing optimized image, native production command; read-only root, writable bounded `/tmp` and `/opt/keycloak/data`; realm import and CA mounts read-only. Persistent identity is in PostgreSQL, not the temporary data directory. HTTP 8080; management 9000 is private. Optional outbound federation/SMTP needs separate provider/trust qualification. |
| Bundled PostgreSQL 17.11 | Retained data PVC, memory-backed socket and `/tmp`, read-only Secret mounts. Only its TLS private key is copied into memory and owned by the assigned UID. The local browser fixture passes with UID 1000900000 and restricted admission; OpenShift SCC, SELinux and CSI remain unqualified. |
| CNPG 1.30.0 / PostgreSQL 17.11 + pgvector 0.8.6 (recorded before ADR-0153; the 0.8.7 base is not yet Kind-qualified) | Existing operator owns instance pods, database PVCs, identity and API access. SQL bootstrap client is covered by the application Job checks; that does not qualify database pods under an OpenShift SCC. Storage class/resources remain existing CNPG values. Importing an existing database volume is not supported by an arbitrary PVC-name switch. |
| Optional TEI cpu-1.8.1 | Nonprivileged 8080, read-only root, HOME `/tmp`, HF_HOME `/data`, writable cache PVC. `tei.cache.existingClaim` reuses a preloaded claim; storageClass applies only to a newly created claim. Volume group/CSI access and model startup must be tested on the target. A `/data/...` model can be preloaded for offline use. |
| Tests/helpers | No application Helm hook test existed. The dependency's optional Selenium test assumes UID 1200 and a writable/browser runtime; enabling it is refused. Use the existing real PKCE/team/MCP acceptance fixture. Fixture pods and provider-bootstrap helpers are included in the restricted Kind run; they are not installed into customer projects. |

Application and selected dependency containers retain non-root execution,
RuntimeDefault seccomp, ALL capabilities dropped and no privilege escalation.
No image permission changes or edits to account files were needed for the
product/Keycloak path. Preserve the locked dependency archive.

## Exposure, TLS and canonical identity

Select one chart exposure mechanism: ClusterIP/private access, Ingress, or
Route. Route and either packaged Ingress are mutually exclusive. There is no
active Gateway API path in this application chart; the unused dependency's
HTTPRoute and Route remain refused. Controllers are platform prerequisites.

The Route option emits path-scoped Routes for `/auth`, `/v1`, `/console` and
`/scim/v2`, plus `/realms/synveda` and `/resources` for packaged Keycloak. It
does not expose metrics, readiness, Keycloak administration or the master realm.
Use `/console/` as the entry URL. All Routes use **edge TLS and Redirect**:
TLS ends at the router and router-to-pod traffic is HTTP. Re-encrypt and
passthrough are deliberately absent because these backend listeners are HTTP.

Use the organisation-managed default router certificate, or set
`route.existingCertificateSecret` and, for packaged identity,
`route.keycloakCertificateSecret`. Each names an existing same-namespace
`kubernetes.io/tls` Secret, never PEM/private-key values in Helm. The chart emits
only namespace Roles scoped to those Secret names and RoleBindings for
`route.routerServiceAccount` (default `openshift-ingress/router`). Verify the
real router account, the cluster's external-certificate feature availability,
custom-host permission and rotation behaviour. This follows the
[4.19 Route certificate contract](https://docs.redhat.com/en/documentation/openshift_container_platform/4.19/html/ingress_and_load_balancing/routes).
No certificate-manager/controller is installed. General Ingress retains its
controller-specific class/annotations and TLS Secret settings; configure its
HTTPS redirect using that controller's documented mechanism.

Set `gateway.publicUrl` and Keycloak's `publicUrl` to exact HTTPS origins. The
issuer JSON and provider metadata must name the same public issuer from browser,
gateway and worker networks. Register exactly `<publicUrl>/auth/callback`.
DNS hairpin/routing and router CA trust must work from pods too. Keycloak uses
fixed hostname and explicit `proxyTrustedAddresses`; allow only actual router
source ranges, never all clients. The Route replaces forwarded headers. Axum's
application redirect/origin contract derives from configured publicUrl rather
than arbitrary forwarded host/scheme headers. The acceptance fixture tests
spoofed headers both at the private API and its HTTPS proxy.

## Outbound trust, proxies and offline boundaries

| Actual client | Trust / proxy contract |
|---|---|
| SQLx: preflight, migrations, gateway, worker | Existing external `verify-full` CA/client-certificate mounts; no HTTP proxy. Managed databases must satisfy the exact role, ownership, extension and forced-RLS contracts. CNPG retains its existing internal transport contract. |
| OIDC discovery, JWKS, code exchange | Existing `oidc.caExistingSecret` adds one private PEM root; Linux native OpenSSL trust also applies. `.no_proxy()` deliberately ignores HTTP/HTTPS/ALL_PROXY, including NO_PROXY. Give the canonical issuer direct verified reachability; an intercepting proxy is not an OIDC compatibility mode. |
| TEI/vLLM/Claude HTTP and Entra/Okta directory clients | Product uses reqwest/native OpenSSL with verification enabled and default environment proxy handling. `outbound.caBundleExistingSecret` mounts a PEM bundle as SSL_CERT_FILE; include required organisation/public roots. `outbound.proxyExistingSecret` selects HTTP_PROXY, HTTPS_PROXY, NO_PROXY keys only. Direct URLs with credentials are not written to shared values. Actual provider egress/private-CA/proxy execution remains provider-specific evidence. |
| Optional TEI downloads | Same outbound settings also reach TEI; SSL_CERT_FILE and REQUESTS_CA_BUNDLE are set. Model startup/download/trust behaviour is unverified here; preload `/data/...` and mirror the image for disconnected qualification. |
| Packaged Keycloak JDBC | Existing separate `databaseCaExistingSecret` and verify-full. Extra Java trust for optional external providers uses upstream Keycloak `extraEnv`/`extraVolumes`/`extraVolumeMounts` and `KC_TRUSTSTORE_PATHS`; it is not inherited from the Rust HTTP bundle. |
| OTLP gRPC | Current compiled transport is private HTTP to an operator-owned Collector; the chart refuses an unsupported direct HTTPS endpoint. Put remote verified TLS/authentication/proxy/trust at that Collector boundary. An empty endpoint stays loopback; core requests/work proceed without a collector or remote telemetry account. |

NO_PROXY should include `localhost,127.0.0.1,::1,.svc,.cluster.local` plus your
actual internal domains/addresses and service/network ranges as supported by
each client. Kubernetes does not automatically inject a cluster-wide proxy into
these pods. Image pulls use the node/runtime's own proxy and CA configuration,
not application env. Product/Jobs/TEI/CNPG use `imagePullSecrets`; the unchanged
subchart uses `keycloak.imagePullSecrets`. All images have existing configurable
registry references; mirror and pull-test every enabled image independently.
Build-time proxy refusal in the Compose images is unchanged.

Deterministic capture and lexical context need no model egress or remote control
plane. Real Claude extraction, external vLLM/TEI, model downloads, directory
pull and remote telemetry are optional and require explicit reachable endpoints.
No claim that every workflow is offline is made.

## NetworkPolicy

Policies are off by default. First inventory the actual endpoints and test an
enforcing CNI in a disposable namespace. [The example](examples/network-policy.yaml)
uses documentation-only addresses; replace them before installation. Rules
select each gateway, worker, install, Keycloak and TEI workload independently.
Workers/Jobs accept no inbound traffic. TEI accepts only the release's runtime
pods. Required ingress, DNS (TCP **and** UDP 53), database and canonical issuer
allowances must be supplied before enabling policies. Existing job execution
uses PostgreSQL and needs no new queue connection.

`extraEgress.{gateway,worker,install,keycloak,tei}` holds native `to` plus `ports`
rules for optional providers, directory pull, proxies, local OTLP and downloads.
With CNPG, select its pods in `networkPolicy.database`; the chart adds SQL and
replication rules around those database pods. Explicit `postgresIngress` must
allow the actual operator (instance-manager port 8000), and `postgresEgress`
must allow the actual API server endpoint/port. Additional monitoring/backup
destinations belong there when configured. Operator namespace policy remains
the platform administrator's responsibility.

Use namespaceSelector and podSelector **in the same peer** to constrain both.
For external services use owned stable CIDRs or an approved egress proxy; standard
NetworkPolicy cannot track DNS names. Do not copy transient cloud IPs. Account
for service DNAT, node-local DNS, host-network routers, dual stack and the chosen
CNI's treatment of node traffic. NetworkPolicy filters L3/L4, not HTTP paths.
Namespace peers allowed to reach Keycloak HTTP must therefore be trusted; path
restriction is enforced at the public router. Before declaring success, test
fresh startup, login, Capture, restarts and migrations, then prove an unrelated
pod and an undeclared egress destination fail. Kind 0.32.0's default kindnet
(`kindnetd:v20260528-9350166c`) enforces ingress and egress policies: a local
probe on 2026-10-22 saw traffic pass, time out under a deny policy and pass
again after removal, with DNS still allowed. The customer-operated PostgreSQL
acceptance below asserts the same on every run. Kind enforcement says nothing
about your CNI; repeat the probes on the target.

## First enterprise target: external PostgreSQL and OIDC

The first enterprise route is the smallest one: the customer's existing
PostgreSQL 14–18 server and OIDC provider, this chart with one gateway and one
worker, and no packaged Keycloak or CNPG. It uses the existing
[external values](ci/external-values.yaml), [OpenShift overlay](examples/openshift.yaml)
and an adapted [NetworkPolicy example](examples/network-policy.yaml). There is no
separate configuration system.

**Rehearsal.** The same route runs end to end on a disposable Kind cluster:

```sh
STARTER_MATRIX=1 STARTER_CASE=external-external POSTGRES_MAJOR=14 \
  demos/ops-2-helm-install.sh
```

`POSTGRES_MAJOR` takes 14–18. The fixture builds a genuine PGDG server for that
major and provisions it only by running the product image's `database-setup-sql`
output as the declared administrator, twice. Before Helm runs, it checks five
refusals against the empty database, with no credential in any output:

| Case | Expected refusal |
| --- | --- |
| Extension removed by the DBA | `db migrate` names vector 0.8.7 and btree_gin 1.3 and creates no relation; rerunning the script restores the extension |
| Provider ADMIN-only grant declared | 14/15: cannot be represented; 16+: the catalogue lacks the declared pair. No relation created |
| Migrator URL naming another database | `db preflight`: refused before connecting as an invalid URL, because the parser binds the expected endpoint and database |
| Untrusted server certificate | `db preflight`: connection failed |
| PostgreSQL 13 server | The setup script stops before any change; `db migrate` names the unsupported major and creates no relation |

It then installs the chart and runs the team, MCP and load workloads. Next it
enables NetworkPolicies through a Helm upgrade, which reruns the installation
Job under them. With the policies on, a pod in another namespace cannot reach
the gateway, the gateway cannot reach an undeclared service, and login, reads,
context and audit verification still work. Finally it restarts the providers,
upgrades, uninstalls and reinstalls with retained Secrets, and revokes access.
It checks again that no Synveda role gained an elevated attribute, schema
ownership or extra membership. The content-free report is
`demos/evidence/ops11-external-postgres-<major>.json`. This is source-image
evidence on Kind only.

**Minimum target information.** Live qualification needs exactly these inputs
from the owner. Leave every value out of shared files and tickets.

| Area | Required input |
| --- | --- |
| Authorisation | An authorised disposable namespace or project, and named people who may run the steps below |
| Platform | Distribution and exact version; for OpenShift the admitted SCC, UID and supplemental-group ranges and SELinux/user-namespace settings; the CNI and whether it enforces NetworkPolicy |
| Exposure | Public HTTPS host, router or Ingress class and its ServiceAccount, the TLS Secret name and its issuing chain |
| Images | The release chart and image digests, and the registry mirror and pull Secret if images cannot be pulled directly |
| PostgreSQL | Host, port and database name matching the certificate SAN; exact server version; provider; the PEM root CA; installed or installable vector (0.8.7 package or provider statement) and btree_gin 1.3; the provisioning administrator and, on 16+, any provider ADMIN-only member/grantor pairs; an egress CIDR; a connection budget |
| OIDC | Exact issuer, public client ID with PKCE S256, API audience and the registered callback `<publicUrl>/auth/callback`; the issuer's PEM root; an egress CIDR reached directly, with no HTTP proxy; two test identities (owner and teammate) and the first-administrator group assignment |
| Recovery | Who owns the database backup and the offline copy of the KMS key |

**Live sequence.** Run on the target, in this order:

1. Record the platform and pod admission inputs with the commands in
   [prerequisites](#platform-prerequisites-and-admission).
2. The DBA renders the script with the release image
   (`docker run --rm -e SYNVEDA_DATABASE_ROLES_FILE=/r/roles.json -v "$PWD/roles.json:/r/roles.json:ro" <image@digest> database-setup-sql > setup.sql`),
   then follows the [DBA handoff](CONFIGURATION.md#dba-handoff). The DBA
   records the extension versions, owners and package or provider statement.
3. Run the [ordinary-role probe](CONFIGURATION.md#ordinary-role-installation-checks).
   Repeat it once with an untrusted CA and once with a URL naming another
   database. Each must fail with the fixed message, and no credential may
   appear.
4. Install with `-f private/team-values.yaml -f examples/openshift.yaml`, the
   v3 post-renderer when restricted-v3 applies, and a NetworkPolicy file built
   from the actual DNS, router, database and issuer peers.
5. Sign in the owner through the [first-login procedure](FIRST_LOGIN.md), admit
   the teammate through People, and complete [first use](FIRST_USE.md) through
   an approved Knowledge revision and a context run that cites it. Record
   subjects, the revision ID and `synveda audit verify --json`.
6. From a pod in another namespace, `curl` the gateway Service; from the
   gateway, `curl` an undeclared address. Both must time out while step 5
   still works.
7. Restart the gateway and worker, then uninstall and reinstall with the same
   values and Secrets. Repeat the step 5 checks: same subjects, the same
   revision, a readable console session and a valid audit chain.
8. Inspect every pod's `openshift.io/scc`, UID, groups, SELinux context and
   read-only root, and check the Route redirect, certificate chain and
   private-path refusals.

Steps that would need a destructive or second server (an unsupported major, a
removed extension) stay on the rehearsal unless the DBA provides a disposable
server. Record the source commit, chart and image digests, server and
extension builds, IdP, CNI, CSI, router, TLS termination, role names and every
result. Only that record can add the target to the support matrix, and only
for exactly what it ran.

## Repeatable checks

```sh
make chart-lint
# Install kubeconform 0.7.0 outside the repository; uses pinned official Route schemas.
make chart-schema
# Operator-free matrix in its own disposable Kind cluster/private kubeconfig.
BUNDLED_MATRIX=1 PORTABILITY=1 STARTER_MATRIX=1 bash demos/ops-2-helm-install.sh
make check-deploy
make compose-config
```

The schema runner uses Kubernetes 1.32.0/1.33.0/1.36.1 schemas and pinned
OpenShift API commits/checksums, including packaged identity, Routes/certificate
RBAC, NetworkPolicies and TEI PVCs. No missing schemas are ignored. Structural
validation does not run SCC/PSA, CEL, router admission or storage/CNI checks.
The existing fixture uses real protocol login over a private-CA HTTPS proxy;
it is not a browser UI or OpenShift router test.

Real OpenShift qualification must record the exact cluster patch, admitted SCC,
UID/group/SELinux/hostUsers for all pods, first install/migrations/test-client
results, browser and pod issuer resolution, Route HTTP redirect/HTTPS certificate
and renewal, private-path refusals, persistent writes/Capture, pod restart and
same-source upgrade. Repeat with packaged Keycloak and separately qualified CNPG
and TEI before widening the matrix. Managed-cloud acceptance additionally names
the actual database extensions/roles, IdP and CSI driver. Published pulls,
restore, N-1 upgrade, HA and certification remain separate work.

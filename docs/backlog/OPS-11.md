# OPS-11: Small-team Kubernetes release

## Problem and evidence

The Docker MVP is owner-validated. The 2026-09-19 audit at `1194c0a` found an
existing working Helm foundation, but no external-database branch, persistent
Keycloak starter or qualified OpenShift installation. The compact evidence,
operator inputs and four-stage plan are maintained in the
[deployment guide](../../deploy/README.md#kubernetes-release-contract).
CPR-45 retains its separate Docker qualification work; OPS-2 remains the
delivered chart foundation.

## Scope

Extend the existing chart in order: core/external services; persistent starter
and onboarding; OpenShift portability; operations, documentation and published
release acceptance. Database and identity modes must be independently usable.

## Non-goals

No new authentication framework, job engine, operator, service mesh, cloud
provisioning, SaaS billing, application HA or old-epoch compatibility path.

## Architecture seam

[ADR-0109](../adr/adr-0109-small-team-kubernetes-release.md) retains one product
image, gateway-served UI, private worker, PostgreSQL and generic OIDC/PKCE.
The existing CNPG integration is optional and uses a preinstalled operator.

### Portable chart increment — 2026-09-19

The first slice is implemented in the existing chart: external PostgreSQL and
OIDC by default, optional CNPG, a closed values schema, image digest selection,
file-mounted credentials, private-CA OIDC trust and verified database TLS with
optional client certificates. Runtime connections independently enforce the
declared database endpoint/TLS contract. The normal revision Job runs bounded
preflight, advisory-locked migration and idempotent tenant/key admission; its
issuer binding can exist before Helm waits. Gateway/worker probes, grace,
resources and scheduling preserve the existing runtime boundaries. Optional
Ingress admits only application routes; optional TEI has non-root settings.

Operator instructions, exact Secret formats and known provider constraints are
in the [chart guide](../../deploy/helm/synveda/README.md). The separate external
acceptance uses source-built product/PostgreSQL/Keycloak images, a disposable
Kind cluster without CNPG, independent providers in another namespace and the
[committed test values](../../demos/fixtures/ops-2/external-values.yaml).

```sh
POSTGRES_MODE=external bash demos/ops-2-helm-install.sh
```

The chart creates no database, identity server, administrator credential or
application PVC in the default external/lexical shape. Its external PostgreSQL
contract retains exact ordinary migrator/runtime roles, forced RLS and provider
identity proofs; managed services must meet that contract without exemptions.

## Acceptance criteria

Both presets and all four database/identity combinations pass rendered
contracts and real login, membership, Session/Capture, Knowledge/Context and
restart checks. External mode renders no CNPG object or superuser bootstrap;
runtime pods receive only their own non-owner database credentials.
OpenShift uses its restricted assigned UID without an SCC exception. Public
metrics/admin refusal, separate role authority, retained data/keys and isolated
restore are tested. Published chart and image digests match the tested source.

## Required tests

Retain `make ci`, `make db-test`, `make chart-lint`, `make check-deploy`,
`make compose-config`, `make claude-acceptance` and `make eval-check`.
Extend the existing `demos/ops-2-helm-install.sh` and Compose acceptance,
backup, restore and upgrade targets; run live tests only on explicitly selected
disposable targets. Unavailable checks remain unverified.

### Measured portable acceptance — 2026-09-19

The external fixture passed on Kind 0.32.0, Kubernetes v1.36.1 Linux arm64,
Helm 4.2.3 and macOS/OrbStack, with source-built PostgreSQL 17.11, Keycloak
26.7.2 and the current product image. The application uses explicitly private
HTTP in this fixture; Keycloak uses HTTPS/private CA and all product database
connections require `verify-full` plus a client certificate. The existing
Keycloak setup command uses a separate private administration endpoint.

The selected empty cluster was `synveda-ops11`; locally built images were
loaded with `REUSE=1 KEEP=1 SKIP_BUILD=1` during diagnosis. The final clean run
passed all steps and the cluster was removed afterward. The exact Helm command,
run with the fixture's isolated kubeconfig and committed external values, was:

```sh
helm upgrade --install synveda deploy/helm/synveda -n synveda-test -f demos/fixtures/ops-2/external-values.yaml --wait --wait-for-jobs --timeout 15m
```

| Check | Result |
|---|---|
| Clean database and Helm wait ordering | Passed; normal revision Job admitted the tenant and encryption key, gateway and worker became ready |
| External provider ownership | Passed; independent provider namespace, no CNPG API, provider/administrator resources absent from application release |
| Real Keycloak authorization code + PKCE | Passed; existing protocol fixture drove login, public Session/context operations and audit verification |
| Database authentication/transport refusals | Passed for wrong password, untrusted CA and certificate hostname mismatch; runtime connections observed using TLS |
| Identity refusals | Passed for untrusted CA, noncanonical issuer and an actual bearer with a mismatched audience |
| Runtime restart and upgrade | Passed; gateway/worker restart, authenticated access, normal Helm upgrade and migration/tenant rerun with audit verification |
| Schema and chart contracts | Passed strict lint/minimal/full renders, missing/ambiguous/ignored-value refusals, file/digest/probe contracts and reproducible packaged external/CNPG renders |
| Rust checks | Formatting and strict Clippy for store/identity/gateway passed; 61 focused database-URL/OIDC/runtime-config tests passed |
| Exact-role database suite | `make db-test` passed; disposable databases removed |
| Deployment contracts | All `make check-deploy` components passed, including the 16-row Compose matrix; the local HTTP fixture was rerun outside the filesystem sandbox after its socket bind was denied |
| Existing Compose smoke | Passed against `synveda-development-acceptance-interop`; no stack reconfiguration |
| Repository consistency | Dependency, backlog, ADR and documentation gates passed |

The exact Compose smoke selection was:

```sh
SYNVEDA_COMPOSE_RUNTIME=development SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-interop SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24 make compose-smoke
```

This is a same-source/same-epoch upgrade, not an N-1/schema upgrade. The browser
fixture proves the real protocol flow, not a new Playwright UI qualification.
The existing CNPG failover run, complete `make ci`, `make claude-acceptance` and
`make eval-check` were not rerun for this slice. The remaining platform,
provider, published-artifact and operational gaps below are unchanged.

### Persistent starter and onboarding increment — 2026-09-19

[ADR-0110](../adr/adr-0110-persistent-starter-and-team-admission.md) retains the
same chart and preinstalled CNPG operator. The
[starter preset](../../deploy/helm/synveda/starter-values.yaml) pins PostgreSQL
independently from application releases, retains its Cluster/PVCs and enables
the unchanged codecentric Keycloak chart 7.3.2 with the existing optimized
26.7.2 image. Both databases share one server with separate owners, passwords
and explicit cross-database refusals. External database/identity choices remain
independent. No operator, broker, identity framework or job engine was added.

The [operator walkthrough](../../deploy/helm/synveda/STARTER.md) covers private
administration, explicit first-owner selection, organisation configuration,
public invitations/grants, read-only membership, existing CLI/MCP access and
revocation. It installs no team users or sample data. The Configuration screen
offers a tenant-root target only when the existing PDP forecast permits it.
MEM-7's narrow admission guard refuses simultaneous issuers for one tenant;
persistent issuer-qualified linking, directory adoption and issuer replacement
remain unsupported. Service clients use the existing registered-subject flow,
a workspace anchor, project grant and 300-second IdP token. Registration/grant
removal is distinct from IdP-only disable, which retains the JWT lifetime plus
30-second validation leeway.

The source changes are the chart dependency/lock, preset, validation, realm
import and shared bootstrap templates; the focused console Configuration and
OIDC admission checks; and the opt-in matrix under
`demos/fixtures/ops-2/starter*.mjs`, dispatched by the existing Helm demo.
`scripts/check-starter-contract.mjs` is part of `make chart-lint`. Compose
images, manifests, authority guards and the schema baseline remain unchanged.

All four combinations passed live on Kind 0.32.0 / Kubernetes v1.36.1,
Helm 4.2.3, Linux arm64 containers and macOS/OrbStack. Source images were built
from this checkout, then reused for the final clean-namespace matrix:

```sh
STARTER_MATRIX=1 REUSE=1 SKIP_BUILD=1 bash demos/ops-2-helm-install.sh
```

The selected disposable cluster was `synveda-ops11-starter`; the fixture used
an isolated kubeconfig. The cluster and its private diagnostic credentials were
removed after acceptance; the pre-existing Compose stack was retained. The
reproducible clean-cluster entry point is
`STARTER_MATRIX=1 bash demos/ops-2-helm-install.sh`, which also builds the images.
The [content-free report](../../demos/evidence/ops11-starter.json) records every
combination, measurement window and assertion result. Each passed fresh install,
real code/PKCE and console-cookie login, explicit owner bootstrap, invitation,
viewer and stranger checks, cross-workspace refusal, governed publication,
registered service/MCP retrieval, planned pod recreation, same-source upgrade,
retained uninstall/reinstall, stable credentials/subjects, role change and
unchanged-token membership/service revocation. Audit verification passed with
526 events per case. Post-revocation API checks completed in 177–197 ms; this is
an observation in one gateway, not a global revocation SLA.

Each workload used four concurrent agent Sessions, 400 events and 80 context
requests. All 400 events completed Capture and produced 256 bounded candidates.
Eight kubelet samples per case covered approximately 12.3–12.7 seconds, every
two seconds including the window endpoints. The shared Docker engine exposed
18 CPUs and 16,817,168,384 bytes of RAM; the existing Compose stack was running.
The short, deterministic lexical/extraction run does not measure semantic
models, steady-state capacity, disk growth, queue saturation or a soak.

| Database / identity | Workload seconds | Gateway mCPU / MiB | Worker mCPU / MiB | PostgreSQL mCPU / MiB | Keycloak mCPU / MiB |
|---|---:|---:|---:|---:|---:|
| CNPG / packaged | 12.51 | 89 / 41.0 | 42 / 21.8 | 84 / 251.2 | 1253 / 1034.9 |
| External / packaged | 12.32 | 114 / 51.9 | 33 / 40.9 | 109 / 180.7 | 1176 / 1041.7 |
| CNPG / external | 12.26 | 68 / 65.1 | 19 / 18.3 | 73 / 222.1; provider 13 / 62.5 | 186 / 996.2 |
| External / external | 12.50 | 91 / 35.5 | 9 / 44.1 | 128 / 205.4 | 226 / 772.2 |

These are independent observed per-pod CPU and memory working-set peaks, rounded
to mCPU (1000 mCPU = one core) and MiB; they must not be summed as a simultaneous
peak. The fixture TLS proxy used 2–8 mCPU and 12.9–44.9 MiB. Test clients, the
CNPG operator and Kubernetes/node overhead are excluded. Provider startup was
recent, so the results are not steady-state measurements. Keycloak's observed
memory exceeded its initial 768 MiB request while remaining below its 2 GiB
limit; size requests for the actual installation. All initial resources are
starting points, not capacity guarantees.

The HTTPS boundary was a private-CA fixture proxy, not a qualified ingress
controller or OpenShift Route. External providers were independent fixture
releases/namespaces, not managed-cloud services. Source images were locally
loaded, so registry pull and amd64/OpenShift evidence remain outstanding.
Retained reinstall reused the same release, namespace, issuer, tenant, Secrets
and PVC identities. It is neither an N-1 upgrade nor a backup/restore test.

The focused and existing gates passed with these commands from the repository
root (the console build runs in `console`):

```sh
cargo fmt --all -- --check
cargo clippy -p synveda-identity --all-targets -- -D warnings
cargo test -p synveda-identity --lib oidc::tests
console/node_modules/.bin/tsc -p console/tsconfig.test.json
node --test --test-timeout=60000 'console/dist-test/*.test.mjs' 'console/dist-test/*.test.js'
(cd console && node_modules/.bin/tsc -p tsconfig.json --noEmit && node_modules/.bin/vite build)
make chart-lint
make check-deploy
make db-test
make check-deps check-backlog check-adr-status check-docs
SYNVEDA_COMPOSE_RUNTIME=development SYNVEDA_COMPOSE_PROJECT_SUFFIX=acceptance-interop SYNVEDA_COMPOSE_IPV4_POOL=10.231.46.0/24 make compose-smoke
```

There were 37 focused OIDC tests and 252 console tests, with no failures or
ignored tests in those two runs. The fresh exact-role database suite passed;
`check-deploy` included the 16-row Compose configuration matrix. The Compose
smoke used the existing stack without reconfiguration. Complete `make ci`,
`make claude-acceptance`, installed-client replay, `make eval-check`, the older
CNPG failover fixture and a new Playwright browser run were not repeated.

### Restricted portability increment — 2026-09-19

[ADR-0111](../adr/adr-0111-restricted-kubernetes-portability.md) and the
[portability guide](../../deploy/helm/synveda/PORTABILITY.md) extend this same
chart with assigned-ID contexts, explicit seccomp, bounded migration temporary
mounts, opt-in NetworkPolicies and path-scoped edge Routes. Routes redirect to
HTTPS, replace forwarding headers and reference organisation TLS Secrets with
router-only namespace read RBAC. Ingress remains configurable and mutually
exclusive. A local Helm 4 post-renderer adds user namespaces and required
restricted-v3 admission to all rendered workloads, including the unchanged
locked Keycloak dependency. Native HTTP CA/proxy inputs and an existing TEI
cache claim are configurable; the guide distinguishes every actual client's
trust/proxy behaviour. No controller, SCC privilege or product behaviour was
added. Cedar, forced RLS, VedaFlow, audit and the job engine are unchanged.

The first arbitrary-UID attempt found a real CNPG/packaged-Keycloak bootstrap
failure: fsGroup mounts propagate setgid onto the private authority directory.
The strict witness checks refused it. The fix changes only the two
process-created private directories: select the process's primary group and
clear inherited special bits using mode 00700. Ownership/audit guards, images
and sensitive account files remain unchanged. The failed Helm wait was
cancelled, its disposable cluster removed, and acceptance restarted cleanly.

| Check | Result |
|---|---|
| `make chart-lint` | Passed existing chart/starter checks and new rendered security, exposure, certificate RBAC, NetworkPolicy, cache and refusal checks |
| `KUBECONFORM=/tmp/ops11-tools/kubeconform make chart-schema` | Passed 27/27 resources for each OpenShift 4.19/4.20 API target and 7/7 for standard Kubernetes 1.36.1; zero skipped/errors; kubeconform built from v0.7.0 |
| `make check-deploy` | Passed 352 tests, zero skipped/failures, canonical 16-row Compose matrix and packaging/convergence checks |
| Existing Compose smoke | Passed on `synveda-development-acceptance-interop`, Engine 29.4.0/Compose 5.1.2, without reconfiguration |
| Formatting/repository checks | Formatting, dependency, backlog, ADR and documentation gates passed; no Rust source changed |
| TEI cpu-1.8.1 | Passed network-disabled Docker execution at UID 1000900000, read-only root, /tmp and /data writes and router `--help`; linux/amd64 under arm64 emulation; no model loaded |
| Restricted Kind lifecycle | All four PostgreSQL/identity ownership combinations passed; 400 captured events, 80 context runs and audit verification through 526 events per case; install, login, service/MCP, pod recreation, upgrade, retained reinstall and unchanged-token revocation |

The final run used Kind 0.32.0/Kubernetes v1.36.1, Helm 4.2.3 and kubectl
1.33.9 on macOS 26.6.2 arm64/OrbStack, with Linux arm64 application/provider
containers, Keycloak 26.7.2/chart 7.3.2, CNPG 1.30.0, PostgreSQL 17.11 and
pgvector 0.8.6. The disposable cluster was
`synveda-ops11-starter-portability`, using an isolated kubeconfig. The
[content-free report](../../demos/evidence/ops11-portability.json) retains all
four cases and the explicit PSA/simulation limits. The exact run reused the
previously built images because this increment changes no image or Rust source:

```sh
PORTABILITY=1 STARTER_MATRIX=1 SKIP_BUILD=1 CLUSTER=synveda-ops11-starter-portability bash demos/ops-2-helm-install.sh
```

Tested Docker image IDs (local images, not registry-pull attestations):

| Image | SHA-256 ID |
|---|---|
| `synveda/product:ops11-starter` | `4fbeb8e6c05c81d75aad28d95d22a8c300518775b3aaef1cda61275c99134cad` |
| `synveda/keycloak:ops11` | `b9d09ac44c2c0a41b921d2f8697b7056e4f14c09ecd30e96e9620be76100f3e9` |
| `synveda/cnpg-postgres:17.11-ops11-starter` | `d11b1a36de2a5f62d20333276458a373eea691d86c12a04747fda9d4bb8e13fe` |
| `synveda/postgres:ops11` | `6fd4155977029b393063b8e784a1e92802868175d4cf6d8c53ad97a07d368d03` |

The schema runner pins official OpenShift API commits
`1ebdd7bba8e6c503a1f61c6d467ed877783826e1` (4.19) and
`05da40ea98557f8351483a39db3d0e7d9076bc8f` (4.20), with SHA-256 verification,
paired with Kubernetes 1.32.0 and 1.33.0 structural schemas. Tests include
packaged Keycloak, TEI PVC, Routes and certificate RBAC. These are structural
results, not OpenShift admission. No OpenShift CLI, kubeconfig context or
authorised project, nor a managed-cloud target, was available. No shared
cluster was contacted.

Reproduce the extended existing fixture with:

```sh
PORTABILITY=1 STARTER_MATRIX=1 bash demos/ops-2-helm-install.sh
```

It enforces namespace PSA `restricted` at `v1.33`, simulates UID 1000900000 for
gateway/worker/install and Keycloak, and checks read-only application paths,
temporary writes and absent runtime API tokens. It tests spoofed forwarded
headers directly and through the private-CA HTTPS fixture proxy. CNPG database
pods retain the operator's UID 26; external fixture PostgreSQL uses UID 999.
This is not an SCC, SELinux, user-namespace, Route, CNI or CSI simulation.

The current exact blocker is the absence of a real authorised OpenShift or
managed-cloud target. Next action: supply a disposable project and exact
cluster patch/policy/storage/CNI/router inventory, then run the guide's live
matrix. Restricted-v3 with CNPG-generated pods, Route TLS/certificate rotation,
real browser UI login, enforcing NetworkPolicy allow/deny probes, TEI model
startup, actual provider private-CA/proxy egress, managed providers and published
pulls remain unverified. Complete `make ci`, the exact-role database suite,
old CNPG failover, native client replay and joint restore were not repeated
for this chart-only increment.

### PR schema gate repair — 2026-09-20

PR #52's schema check failed on Linux because kubeconform expands
`{{.ResourceKind}}` to lowercase, while the runner wrote `Route.json`. The
case-insensitive macOS filesystem hid the mismatch. A read-only Linux arm64
container with Node 22.23.2, Helm 4.2.3 and kubeconform 0.7.0 reproduced all six
missing-Route-schema errors. The runner now writes `route.json` and includes
the failed command, stdout, stderr and spawn errors in its assertion message.
The unchanged strict matrix passes on Linux and macOS: 27/27 resources for
each OpenShift target and 7/7 for Kubernetes, with zero errors or skips.
`make chart-lint` and formatting also pass. Platform and release qualification
remain open as recorded above.

## Rollout and rollback

Keep one gateway/worker and the existing CNPG failover path. Promote a candidate
only after login, restart and joint database/key restore. Roll back using a
compatible complete artifact set; no automatic data deletion or schema reset.

## Dependencies

Next action after starter acceptance: qualify restricted OpenShift and the
operations/release matrix. A real ingress controller, OpenShift assigned
UID/storage, managed-cloud providers, TEI model startup and registry-backed installation remain unqualified. The
local claimed-work recovery and joint restore increment is recorded below. No published
candidate image/chart set has pull evidence. Keycloak 26.7.4 is available through
the selected chart's metadata; its provider upgrade is deliberately separate
from the tested, fixed 26.7.2 image.
Recovery/custody and upgrade work coordinate with OPS-5/OPS-6; publication reuses
OPS-8. These remaining scopes keep OPS-11 open.

### Operability and release-evidence increment — 2026-09-20

[ADR-0112](../adr/adr-0112-small-team-operational-evidence.md) extends the
existing Kind/team fixture with actual claimed-Capture SIGTERM, concurrent
advisory-locked migration, repeated login/ingestion/context workloads, joint
logical recovery into a clean namespace and existing-key/identity verification.
No product authority or job engine is replaced. The old starter instructions
now link to one Kubernetes installation guide, configuration reference and
operations runbook; the stale pre-implementation deployment audit is removed
from current operator prose.

Release packaging now emits immutable Helm image overlays from the same six
image digests as the existing Compose reference. The tag-only workflow prepares
OCI chart push/pull verification and BuildKit provenance/SBOM generation; dry
runs still publish nothing. New CI jobs retain Compose/CNPG gates and separately
exercise starter/external operational acceptance and the existing TLS/OIDC
negative matrix. Hosted workflow execution remains unverified.

Anonymous GitHub release inventory on 2026-09-20 returned [public v0.2.0](https://github.com/synveda/synveda/releases/tag/v0.2.0)
(published 2026-08-16) with native, console, plugin and old profile archives;
there is no current chart/reference bundle. Its source lacks the epoch-3
baseline. Therefore no supported published N-1 upgrade pair exists. Do not
substitute a same-source rerun for that requirement or reset existing team data.

The live migration race initially failed with a serialization error because
SQLx released its DDL lock before the epoch stamp. The fix holds the same
reentrant lock through preflight, migration and stamping, then closes the owned
connection; cancellation cannot return a locked connection to the pool. The
fresh exact-role database suite and strict store Clippy passed after the fix.
No baseline, SQLx query cache, OpenAPI or generated client changed.

The final [starter](../../demos/evidence/ops11-operations-cnpg-packaged.json)
and [external](../../demos/evidence/ops11-operations-external-external.json)
runs both passed. Each used fresh namespaces/databases and an extracted, locally
packaged `0.2.0-ops11-local` chart with the rebuilt product image
`sha256:1644705aeed1977533bdbf4f50eb5decdcbe0bc830152bc6499e5372952d48f5`.
Kind 0.32.0 / Kubernetes 1.36.1 Linux arm64, Helm 4.2.3, kubectl 1.33.9,
CNPG 1.30.0, PostgreSQL 17.11 (vector 0.8.6, btree_gin 1.3), Keycloak 26.7.2
and keycloakx 7.3.2 ran on macOS 26.6.2 arm64 / OrbStack Docker 29.4.0,
Compose 5.1.2. Build caches and source images were reused; no developer team
state was seeded into either database. The external cluster was reused only
after deleting its failed diagnostic namespaces; the final databases were new.
The shared engine exposed 18 CPUs / 16,817,168,384 bytes RAM, with the existing
Compose stack and an independent fixture cluster present during part of the run.

Each case ran one measured workload plus five repetitions: **2,400 events,
480 context runs and 24 ended agent Sessions**, excluding setup/fault checks.
All completed Capture. Each real blocked-provider interruption cancelled one
HTTP request, exited zero, then completed in two attempts with one candidate
and two provider calls. Gateway SIGTERM exited zero and was ready in 4.26 /
5.32 seconds respectively; in-flight HTTP drain was not exercised.

| Profile | Five-round elapsed s | SIGTERM to Capture recovery s | Quiesce + archives s | Fresh restore + verification s | Frozen audit events |
|---|---:|---:|---:|---:|---:|
| CNPG / packaged | 66.96 | 59.78 | 2.56 | 32.99 | 2390 |
| External / external | 65.08 | 59.65 | 2.11 | 35.89 | 2417 |

Restore time includes fresh provider provisioning, native archive load, chart
installation and public verification. Archives were about 1.5 MB Synveda and
0.27 MB Keycloak; hashes and exact UTC recovery points are in the reports.
Original Secret material was retained separately. These are local logical
recovery observations, not an RPO/RTO, off-host-custody, full-cluster-loss or
PITR promise. Both restores proved fresh login, stable subjects, membership,
representative Knowledge/Context retrieval, a durable Session payload, original
sealed console-session decryption and frozen audit-prefix equality. Both
sources then resumed, survived pod recreation, same-source Helm migration
reruns and retained uninstall/reinstall; unchanged-token member/service
revocation and MCP denial passed.

The external outage returned readiness 503 with liveness 200. Actual authority
gates reopened after 10.99 seconds from database resume; the complete public
flow recovered in 12.83 seconds, including **one** failed login while Keycloak
discarded a dead JDBC connection. Login retry was bounded at 90 seconds; access,
subject and content assertions were not retried. Kubernetes Ready status alone
was insufficient evidence of current application or identity readiness.

Initial workload samples (mCPU / MiB, independent per-pod peaks):

| Profile | Gateway | Worker | PostgreSQL | Keycloak |
|---|---:|---:|---:|---:|
| CNPG / packaged | 15 / 32.3 | 10 / 16.1 | 87 / 219.1 | 33 / 607.8 |
| External / external | 98 / 49.0 | 40 / 30.6 | 106 / 198.2 | 1690 / 1004.0 |

Kubelet samples every two seconds cover the initial 13.34 / 13.16 second
workloads, excluding clients, operator and node overhead. They are not summed
simultaneous peaks, steady-state capacity or a long soak. The test CNPG request
was 250m / 512Mi with a 2Gi PVC; the operator preset keeps 1 CPU / 2Gi and a
20Gi PVC. Keycloak ran with its former 768Mi request / 2Gi limit. Its observed
1,004 MiB peak informed a final **1280Mi request**, retaining the 2Gi limit;
that reservation-only adjustment is render/package-checked. Other defaults
remain gateway 500m / 512Mi and worker 250m / 256Mi. Rehearse actual quotas,
data size and model providers before sizing a team installation.

Exact final live invocations used cached, already-built images:

```sh
OPERATIONS=1 STARTER_MATRIX=1 STARTER_CASE=cnpg-packaged SKIP_BUILD=1 KEEP=1 CLUSTER=synveda-ops11-starter-final-cnpg bash demos/ops-2-helm-install.sh
OPERATIONS=1 STARTER_MATRIX=1 STARTER_CASE=external-external SKIP_BUILD=1 KEEP=1 REUSE=1 CLUSTER=synveda-ops11-starter-final-external bash demos/ops-2-helm-install.sh
POSTGRES_MODE=external SKIP_BUILD=1 CLUSTER=synveda-ops11-ops-negative bash demos/ops-2-helm-install.sh
```

For a new checkout use the installation guide's commands without `SKIP_BUILD`,
`KEEP` or `REUSE`. Only task-owned clusters/namespaces were removed; private
scratch archives, keys and kubeconfigs are removed after recording evidence.
Fixture diagnosis also corrected an event-read route, shutdown-log scheduling
assumption, stale ConfigMap metadata, packaged dependency path and recovery
readiness timing; no failed run is presented as a passing release.

Static and focused gates passed: formatting, strict Clippy for `synveda-store`,
`make db-test`, `make chart-lint`, `make chart-schema` (27/27 resources for each
OpenShift structural target and 7/7 for Kubernetes), `make check-release-parity`
(23 tests and four packaged immutable provider renders), workflow Actionlint
1.7.7, dependency/backlog/ADR checks, and all `make check-deploy` components.
The latter's local HTTP binding test was rerun outside the filesystem sandbox
after a bind permission error; no product gate was weakened. Existing Compose
smoke passed against `synveda-development-acceptance-interop` without rebuilding
or reconfiguring it. The independent external fixture passed its TLS/mTLS,
credential/CA/hostname, exact issuer and wrong-audience refusals. Full hosted
CI, real OpenShift/cloud, native client replay and registry-backed release
acceptance did not run in this increment.

The published v0.4.0 installation increment is recorded in
[CPR-45](CPR-45.md#installation-mission-2026-09-20). Native Linux AMD64/ARM64
runners anonymously pulled the exact image set and exercised all four ownership
modes, operational recovery and real browser port-forward login with the
packaged chart. The anonymous OCI chart matches the public archive; both
checksummed Kubernetes reports are release attachments. Publication recovered
the original qualified bytes after an upload-only failure; no tag/image was rebuilt.

Remaining blockers: an explicit supported prior release and the unavailable
real OpenShift/cloud targets. Next: supply a disposable platform target and
declare the compatible upgrade pair, then qualify those exact configurations.
Kind restricted-UID tests do not establish SCC, router, CNI/CSI or managed-provider
support. The local work used only owned disposable fixtures.

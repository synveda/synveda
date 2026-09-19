# OPS-11: Small-team Kubernetes release

## Problem and evidence

The Docker MVP is owner-validated. The 2026-09-19 audit at `1194c0a` found an
existing working Helm foundation, but no external-database branch, persistent
Keycloak starter or qualified OpenShift installation. The compact evidence,
operator inputs and four-stage plan are maintained in the
[deployment guide](../../deploy/README.md#small-team-kubernetes-release-contract).
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

## Rollout and rollback

Keep one gateway/worker and the existing CNPG failover path. Promote a candidate
only after login, restart and joint database/key restore. Roll back using a
compatible complete artifact set; no automatic data deletion or schema reset.

## Dependencies

Next action after starter acceptance: qualify restricted OpenShift and the
operations/release matrix. A real ingress controller, OpenShift assigned
UID/storage, managed-cloud providers, TEI model startup, claimed-work worker
recovery and joint database/key restore remain unqualified. No published
candidate image/chart set has pull evidence. Keycloak 26.7.4 is available through
the selected chart's metadata; its provider upgrade is deliberately separate
from the tested, fixed 26.7.2 image.
Recovery/custody and upgrade work coordinate with OPS-5/OPS-6; publication reuses
OPS-8. These remaining scopes keep OPS-11 open.

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

## Rollout and rollback

Keep one gateway/worker and the existing CNPG failover path. Promote a candidate
only after login, restart and joint database/key restore. Roll back using a
compatible complete artifact set; no automatic data deletion or schema reset.

## Dependencies

Next action after the portable increment: implement the separately scoped
persistent starter/onboarding slice, then qualify restricted OpenShift and the
operations/release matrix. The starter, all four database/identity combinations,
shared HTTPS ingress, OpenShift assigned UID/storage, managed-cloud providers,
TEI model startup, claimed-work worker recovery and joint database/key restore
remain unqualified. No published candidate image/chart set has pull evidence.
Recovery/custody and upgrade work coordinate with OPS-5/OPS-6; publication reuses
OPS-8. These remaining scopes keep OPS-11 open.

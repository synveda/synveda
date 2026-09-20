# ADR-0109: Small-team Kubernetes release boundary

- **Status**: Accepted
- **Date**: 2026-09-19
- **Feature(s)**: OPS-11; reuses OPS-2 and CPR-45
- **Deciders**: Project owner scope; repository deployment audit

## Context

The owner has validated the local Docker MVP. The next milestone serves small
teams on existing Kubernetes clusters, including OpenShift. The existing Helm
chart already runs separate gateway and worker processes, but requires CNPG,
has no bundled identity mode and has not qualified restricted OpenShift pods.
The evidence and implementation order live in [the deployment guide](../../deploy/README.md#small-team-kubernetes-release-contract).

## Decision

Extend `deploy/helm/synveda` with external-services and persistent-starter
presets, sharing Compose's images, commands, configuration and bootstrap
boundaries. Database and identity selection are independent: external
PostgreSQL or the existing CNPG integration, and external OIDC (Keycloak first)
or bundled optimized Keycloak with the existing convergence process.

Default to one organisation/tenant per installation and namespace, one trusted
issuer, one gateway and one core worker with `Recreate`. The starter uses one
persistent CNPG instance on clusters where the operator is already installed;
retain the existing replicated database option and failover test. The chart
installs no operator or cluster-scoped infrastructure. The first OpenShift
qualification uses external PostgreSQL; CNPG on OpenShift is a separate,
unverified combination. These are implementation targets, not shipped presets.

## Options considered

1. **Extend the existing chart and reuse preinstalled CNPG** — selected; keeps
   the current database authority and failover evidence without another DB
   lifecycle implementation.
2. **Add a second chart, operator or database engine** — unnecessary for the
   stated release; increases installation and recovery obligations.
3. **Publish the current chart unchanged** — cannot meet independent service
   selection, starter onboarding or the OpenShift boundary.

## Consequences

- The external preset needs no CNPG CRD or application PVC. Starter storage
  and Keycloak recovery remain explicit operator responsibilities.
- Application upgrades interrupt service; OPS-7 alone can lift replica limits
  after cross-pod login, authority visibility and worker-concurrency evidence.
- Revisit the starter mechanism only if a concrete target cannot use either
  conforming external PostgreSQL or its already-installed CNPG integration.

## Portable chart increment (OPS-11)

The first increment defaults to external PostgreSQL and external OIDC. CNPG
remains an explicit optional mode, with one database instance by default and
its existing failover fixture selecting two. No identity server or operator
is added to the application chart in this increment.

External database credentials remain complete SQLx URLs in three distinct
operator-owned Secrets. The declared endpoint, database and role contract are
checked by the existing product preflight. External connections require
`verify-full` and a mounted root CA; optional client certificate/key files
supplement, never replace, server verification. Database/extension/role
provisioning stays outside external application startup. The existing CNPG
bootstrap is retained only for CNPG mode. Issuer, KMS and extractor credentials
use the same bounded file loader as Compose. An explicit OIDC root CA augments
platform trust for discovery, JWKS and the existing token exchange client.

The normal revision-named Job remains the installation mechanism. Bounded
preflight retries precede SQLx's database advisory-locked migration and
idempotent tenant convergence. A preselected tenant UUID allows its issuer
Secret to exist before `helm install --wait --wait-for-jobs`; no hook waits on
resources not yet created. Startup probes tolerate initialization, liveness
checks only the local process, and existing schema/authority readiness and
worker drain remain authoritative. ClusterIP is the base exposure; an explicit
Ingress uses the cluster's existing controller and HTTPS contract. Unknown or
inapplicable settings are refused rather than silently ignored.

## Compliance notes

Tenant/workspace isolation, Cedar, forced RLS, VedaFlow and audit are unchanged.
Initial administrator admission reuses the single-use verified group mapping;
subsequent workspace ownership and membership use the public API. Presets add
no behaviour profiles, default credentials or model-provider requirement.

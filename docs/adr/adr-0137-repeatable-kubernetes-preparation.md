# ADR-0137: Repeatable preparation for the existing Kubernetes chart

- **Status**: Accepted
- **Date**: 2026-10-03
- **Feature(s)**: OPS-11, OPS-12, OPS-8; CPR-8, CPR-39
- **Deciders**: Owner's fresh-install request; repository implementation

## Context

Human installers need usable recipes without Docker, CI fixtures or knowledge
of database bootstrap internals. The current local utility hard-codes names,
ports and tenant identity. Manual retries can replace recovery credentials.
Browser project access also cannot establish that a client delivered context.

## Decision

Extend the chart's release-packaged Node preparation utility. Its only output
contract is ordinary Helm values and separately protected, operator-owned
Secret material. Persist the selected inputs and generated credentials once,
before deriving outputs; refuse conflicting inputs or changed outputs. An
interrupted output write can be retried using that original private state.
No resource is applied by preparation. Native Node 22+ and OpenSSL replace the
container requirement. The executable and image overlay come from the same
publisher-verified chart release; source candidates are explicitly separate.

Ship customer values recipes in the chart and consume them in chart tests.
Keep offline rendering deterministic. A separate operator command distinguishes
offline checks, namespaced read-only API checks, installation-stage inspection
and private readiness verification. An explicit external-database probe creates
a temporary Job from the chart's same three-role preflight container, with only
its credential/CA mounts and role ConfigMap; it performs no bootstrap or DDL.
Creating that Job is a Kubernetes mutation, separate from read-only API checks.
The normal install Job performs administrator bootstrap, three-role authority
preflight, migration and tenant admission in that order. It is a mutating
workload, never called read-only.

Console setup choices come from registry onboarding metadata, including manual
routes. Persist only non-secret setup progress scoped to principal and project.
Report browser access, user-confirmed setup and authenticated client evidence
separately; a Session created without events or context is not an observed
client operation. No browser check proves vendor loading or live capability.

## Options considered

1. **Extend the packaged utility and chart** — selected; inspectable output,
   no second deployment authority or new CLI/server dependencies.
2. **New operator or installer framework** — duplicates lifecycle ownership.
3. **Helm credential generation or Secret lookups** — breaks recovery and
   deterministic GitOps rendering.

## Consequences

- Workstations need Node and OpenSSL for preparation, Helm/kubectl for install,
  and trusted GitHub CLI plus curl/tar for publisher verification.
- Private state, both databases, issuer identity and original encryption keys
  remain an operator recovery set. No automatic reset or credential rotation.
- Unavailable cluster-wide reads require specific administrator input; they
  cannot establish StorageClass, CNI, registry or pod connectivity support.
- Qualification on a named self-managed distribution and three unfamiliar
  engineers remains required before timing or platform claims.
- Revisit the workstation dependency only if a qualified native command can
  implement this same preparation contract with less maintenance.

## Compliance notes

No change to Cedar, forced RLS, VedaFlow, audit, role separation, one-time group
admission, shared Compose configuration or retained uninstall. Preparation
does not contact the product or grant authority. Health tests hold no user
credentials; functional verification uses the existing authenticated APIs.

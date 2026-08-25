# ADR-0095: deployment shapes select infrastructure, governed Configuration selects behaviour

- **Status**: Accepted
- **Date**: 2026-08-25
- **Feature(s)**: CPR-36
- **Deciders**: autonomous context-platform continuation

## Context

Synveda already builds one gateway image and one host gateway binary, but its
deployment surfaces still describe three products. Compose is called the SMB
profile, Helm is called the enterprise profile, and the release bundle carries
an ACME seeder that invokes deleted hierarchy, policy-assignment, global
observe and global recall APIs. Worse, both Compose gateways connect as the
database owner, so forced row-level security is inert in the two deployment
paths most likely to be used by an individual or small team. Helm alone uses a
non-superuser member of `synveda_app`.

CPR-30 made personal, team and enterprise complete immutable Configuration
documents selected by governed scope bindings. Keeping the deployment labels
would now create a second, unaudited profile selector beside that domain. The
pre-1.0 cut also rules out retaining the dead release seeder until a later demo
can replace it.

## Decision

1. **There is one application runtime.** The installed host process, Compose
   service and Helm Deployment run the gateway built from the same workspace,
   against schema epoch 2 and the same generated `/v1` contract. No deployment
   value, image, binary or environment branch selects personal, team or
   enterprise behaviour.

2. **Deployment configuration is infrastructure only.** Compose and Helm may
   size Postgres, choose an OIDC issuer, inject secret references, select a
   supported extractor/embedder implementation and configure telemetry. A
   runtime policy profile, capture rule, context budget, trace mode, freshness
   rule or Skill/Tool advertisement rule is an immutable Configuration version
   and governed binding, created after login through the public API and
   VedaFlow. With no binding, the existing enterprise document remains the
   conservative fail-safe; an installer does not silently widen it.

3. **Every deployed gateway observes forced RLS.** Migrations continue to
   create the NOLOGIN capability role `synveda_app`. `synveda init`, while
   connected with the local bootstrap owner, converges a separate fixed
   `synveda_gateway` LOGIN role, removes superuser/BYPASSRLS/role-creation
   attributes, grants it membership of `synveda_app`, and verifies the result.
   Host and container gateways use that role. Helm keeps CloudNativePG's
   generated login and grants the same membership in its install job. Domain
   migrations and tenant admission still use the bootstrap/admin identity and
   never hand it to the gateway.

4. **Bootstrap remains deliberately narrow.** Installation may migrate the
   schema, admit one tenant, provision deployment key material and establish
   the database login needed to enforce RLS. The first authenticated login
   still creates the tenant root, principal scope and operator grant. Every
   workspace, project, session, Knowledge item and Configuration binding is a
   public-API/PDP/VedaFlow/audit act after that login.

5. **The obsolete packaged demo is deleted, not emulated.** `synveda init
   --demo`, the bundled people/groups, `deploy/release/demo`, and the packager's
   copy/assertions are removed together. Their script cannot be repaired by
   aliases because its model no longer exists. The executable feature demos
   already exercise the current session/capture/Knowledge/context plane; the
   later one-command PulseBoard package will add the replacement product demo
   through public APIs.

6. **Deployment gates assert artefacts, not labels.** A CI check renders both
   Compose files and the Helm chart, rejects owner-role gateway DSNs and removed
   production paths/nouns, packages the release twice to prove deterministic
   replacement, and asserts the bundle contains no retired seeder. Database
   acceptance verifies the converged Compose login is LOGIN, non-superuser,
   non-BYPASSRLS, a `synveda_app` member, and unable to read tenant data without
   the tenant GUC. Existing chart lint and kind/failover tests remain the live
   Kubernetes evidence.

7. **Operational limits stay explicit.** The chart remains one gateway replica
   with `Recreate` until OPS-7 closes process-local login and cache state. Helm
   proves CloudNativePG failover, not gateway HA. Compose is a local/single-node
   deployment with development database credentials. Binaries remain unsigned;
   there is no promised Windows build, zero-downtime gateway upgrade, external
   HSM, or old-schema translator.

## Options considered

1. **Add a deployment `profile=personal|team|enterprise` switch.** Rejected:
   it would be a second runtime policy selector outside immutable Configuration,
   VedaFlow and audit.
2. **Keep Compose on the owner role because it is local.** Rejected: a personal
   deployment is where private-scope isolation matters most, and a backstop
   tested only in Helm is not one runtime.
3. **Keep or shim the ACME seeder until the new demo lands.** Rejected: it calls
   APIs and nouns deliberately removed by the hard cut. Shipping a known-dead
   executable is a false support claim; compatibility routes would be worse.
4. **Create the gateway LOGIN role in a migration.** Rejected: credentials and
   LOGIN identities are deployment-owned (ADR-0009). Migrations create only the
   NOLOGIN capability role; each deployment provisions its own login.

## Consequences

- Positive: all supported deployment shapes exercise the same PDP, VedaFlow,
  audit and forced-RLS data path; behaviour differences are inspectable,
  versioned rows rather than edition branches; a release cannot silently ship
  the removed runtime again.
- Negative / accepted: existing local Compose deployments must run `synveda
  init` once to converge the new gateway login before starting the deployed
  service; the old `init --demo` tour disappears before its PulseBoard
  replacement lands; Helm remains single-gateway and restart-shaped.
- Reversal trigger: OPS-7 proves multi-replica login and cache correctness ->
  lift the chart refusal with that evidence. A supported external secret or
  database provider -> implement it behind the existing deployment boundary,
  never as a product profile branch.

## Compliance notes

- **PDP/VedaFlow/audit:** no domain write moved into bootstrap. Runtime profile
  selection remains the CPR-30 public Configuration lifecycle.
- **RLS:** the deployed process never receives the bootstrap/admin DSN; the
  acceptance test checks both role attributes and a tenantless read refusal.
- **Secrets:** Helm keeps generated Secret references. Compose's fixed password
  is explicitly local-development material already present in that profile and
  is not printed in diagnostics; no customer credential enters a manifest,
  log, audit row or generated client.

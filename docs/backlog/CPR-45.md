---
title: "CPR-45: Docker-first portable reference deployment"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-45: Docker-first portable reference deployment

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Problem and evidence

Synveda has one application runtime, schema and public API, but complete live
evidence for a portable single-host installation is still missing. The
canonical Compose graph, digest-bound reference archive, logical recovery,
same-schema upgrade and experimental Apalis canary are implemented and covered
by deterministic contracts. They have not all run together from an empty
published installation on Linux and Docker Desktop.

Static configuration cannot prove that an operator can pull the artifacts,
sign in through Keycloak, use the product, restart it, recover it and upgrade
it. Until those gates pass, the verdict is “Docker reference implemented; live
validation pending.”

[ADR-0102](../adr/adr-0102-portable-reference-deployment.md) makes Compose the
canonical single-host deployment contract. Helm implements the same contract
with Kubernetes primitives; it is not generated from Compose.
[ADR-0105](../adr/adr-0105-direct-compose-acceptance.md) requires direct
acceptance against an operator-supplied supported Docker engine rather than a
non-executing engine simulation.

The reference topology is Caddy, PostgreSQL 17, optimized production-mode
Keycloak, separate gateway and worker commands from one product image, and a
private OpenTelemetry Collector. Synveda and Keycloak use separate databases
and roles. Keycloak stays behind the generic OIDC/OAuth2 + PKCE boundary and is
the only bundled identity provider. Temporal is absent.

## Scope

- Keep one provider-neutral configuration, image, command, schema, health,
  public API, OIDC, OTLP, object-store and recovery contract.
- Keep only the reverse proxy public; database, identity management, worker,
  metrics, OTLP and recovery services stay private.
- Use mounted files for secrets and distinct non-owner gateway/worker database
  roles under the existing Cedar, forced-RLS, VedaFlow and audit invariants.
- Support bundled or external OIDC and bundled or external PostgreSQL with the
  same product image; external dependencies remain operator-owned.
- Exercise a clean two-principal public-API scenario across the fixed restart
  matrix.
- Back up both product databases with separately held KMS/identity recovery
  material and restore only into a fresh isolated target.
- Keep `skill_validation@1` as the sole optional Apalis 0.7.4 canary behind the
  provider-neutral operation/attempt/outbox model and native-worker rollback.
- Keep OpenTelemetry as the application telemetry interface, with one bounded
  optional local metrics view and a customer-safe Operations route.
- Package the reference graph with immutable image identities and a source-SHA
  environment manifest.

## Non-goals

- No HA, host-loss tolerance, multi-region operation or zero-downtime upgrade.
- No production SaaS, disaster-recovery, compliance or signed-provenance claim.
- No complete Helm promotion, scheduler product or Apalis board.
- No migration of Sessions, Capture, Knowledge mutation, PDP, VedaFlow or audit
  authority into a queue provider.
- No Keycloak-specific Synveda principal, role, grant, tenant or policy model.
- No automatic container-engine installation or VM/provider receipt layer.

## Architecture seam

`deploy/compose/scripts/compose.sh` selects the closed development/reference,
PostgreSQL, OIDC and OTLP matrix. Core services are not profile-gated. Optional
profiles are demo/browser acceptance, bounded local observability and the
experimental Apalis canary. Reference mode requires real DNS, HTTPS certificate
files, an explicit private network and immutable images; development is
explicit loopback HTTP with managed `.test` names.

The release workflow builds five deployment images—product, single-host
PostgreSQL, optimized Keycloak, proxy and CloudNativePG-compatible
PostgreSQL—plus the browser-acceptance fixture. The reference archive contains
only the reference runtime closure and records every image identity in
`environment.json`. The installer stores immutable releases
under `reference/releases`, repoints `reference/current`, and preserves
separate state and backup roots. Automatic artifact removal remains fail-closed
until OPS-10 adds a strict installer ownership receipt.

The operation ledger and outbox are authoritative tenant data under forced
RLS. Apalis receives only an untrusted tenant routing identifier, Synveda
operation ID and version; its worker resolves and reauthorises product state in
a tenant-scoped transaction. The separate queue database is disposable
transport state and is excluded from recovery.

Logical recovery pauses canonical writers, creates separate Synveda and
Keycloak PostgreSQL archives, links them to a separate KMS/identity recovery
set and restores only into a confirmed fresh project. It proves the audit chain,
tenant key and wrong-key refusal before normal convergence. This is same-host
planned-interruption validation, not PITR or disaster recovery.

The documentation audit consolidates source-checkout operation in
`deploy/compose/README.md`, keeps packaged installation distinct, and routes
beginner navigation through the root documentation index. It does not add live
deployment evidence or change the remaining acceptance boundary.

### Remaining live acceptance

1. Run clean development and reference HTTPS acceptance on Linux and Docker
   Desktop, including exact issuer/browser login and the restart matrix.
2. Run logical backup and isolated restore with the recovered KMS key and
   post-restore Keycloak login on both supported host classes.
3. Run the native and Apalis Skill-validation paths through duplicate,
   cancellation, restart and rollback cases on the live reference.
4. Run the same-schema two-image upgrade/rollback smoke using published digest
   references.
5. Run deterministic external-dependency modes and at least one live external
   OIDC/PostgreSQL/OTLP combination when credentials exist.
6. Publish one candidate archive and manifest-bound image set, then install and
   pull it from an empty supported host.

## Acceptance criteria

- Every supported selector passes Compose configuration without rendering a
  secret; fresh bundled mode converges PostgreSQL, Keycloak, gateway, worker,
  proxy and Collector.
- Browser authorization-code + PKCE S256 login proves exact issuer, JWKS,
  audience, algorithm, callback and administrator-group handling; negative
  token cases fail.
- Separate database roles cannot cross product boundaries and forced RLS holds
  for runtime work.
- The public-API scenario creates a workspace/project, redeems a second-member
  invitation, records a Session/Capture candidate, accepts Knowledge and reuses
  it with provenance after every service restart.
- Backup/restore binds both databases and recovery keys, verifies audit/key
  evidence and refuses a wrong key without overwriting the source.
- Native and optional Apalis delivery produce one authorised canary effect
  under duplicate/restart/cancellation cases; queue payloads stay opaque.
- Operations and telemetry disclose no content, secret, cross-tenant data or
  provider-internal task identity.
- A digest-bound current-schema image upgrade preserves product evidence,
  restores the last verified image on ordinary failure and refuses an unsafe
  candidate before transition.
- Confirmed reset removes only the exact Compose project's owned resources.
- No active Rauthy, Temporal or second Compose lifecycle remains.

## Required tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
make ci
make db-test
make claude-acceptance
make check-deploy
make compose-config
make compose-acceptance
make compose-backup
make compose-restore-smoke
make compose-upgrade-smoke
```

Unavailable live services or credentials are reported as unavailable, never
relabeled as pass.

## Rollout and rollback

The identity and release hard cuts are complete; there is no retired-provider
compatibility mode. Rollback uses a previous complete environment manifest,
not mixed current/old images. The Apalis fragment is disabled by default and
the native PostgreSQL worker is its rollback. Restore always targets a fresh,
confirmed project.

## Dependencies

Completion needs a supported Docker Engine, Linux and Docker Desktop hosts,
browser trust for the selected issuer, and one published candidate. Production
S3/WAL-PITR, encrypted off-host retention and recurring recovery drills remain
OPS-5. Signing/provenance, HA, hosted SaaS and broader on-prem/Helm promotion
remain separate readiness work.

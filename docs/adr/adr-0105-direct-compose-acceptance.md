# ADR-0105: Direct Docker Compose acceptance

- **Status**: Accepted
- **Date**: 2026-09-08
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Context

CPR-45 already has a canonical Compose graph for PostgreSQL, Keycloak, the
reverse proxy, the gateway, the worker and the OpenTelemetry Collector. It
also accumulated a separate clean-engine subsystem intended to prepare and
describe a disposable Colima/Docker provider. That subsystem contains tens of
thousands of lines of receipt, reservation, process and recovery fixtures, but
its supported commands do not start Docker and provide no deployment
acceptance evidence.

The reference deployment needs evidence from the product topology itself. It
does not need to provision or prove the internals of the operator's container
engine. Docker Desktop, Docker Engine or another explicitly supported local
engine is a prerequisite owned by the operator.

## Decision

Synveda will validate the reference deployment by invoking the canonical
Docker Compose graph directly against an already-running supported engine.
Acceptance begins with an empty, uniquely named Compose project, runs the real
Keycloak browser flow and product smoke path, exercises bounded restarts, and
ends with an explicitly confirmed project-scoped reset.

The clean-engine receipt, Colima-provider, reservation and four-process fixture
subsystem is removed. ADR-0103 and ADR-0104 are superseded. Their provider
state formats are test scaffolding, not product or deployment compatibility
contracts, so no reader, translator or retained compatibility command is
provided.

Acceptance evidence has three explicit classes:

1. `compose-config` validates files and rendered configuration without a
   daemon.
2. deterministic tests validate scripts and failure handling without claiming
   a running deployment.
3. `compose-acceptance` requires a real engine and services. An unavailable
   engine or platform prerequisite is reported as unavailable, never as pass.

The completion programme remains deliberately bounded:

- reverse proxy, PostgreSQL, production-mode Keycloak, gateway, worker and a
  private OpenTelemetry Collector;
- PostgreSQL-native full logical backup and isolated restore of both product
  databases, with the Synveda KMS key supplied and verified separately;
- one disabled-by-default `skill_validation@1` operation delivered through an
  exact-pinned Apalis 0.7.4 leaf adapter;
- a minimal optional local observability profile and customer-safe Operations
  route using bounded aggregates;
- restart, upgrade/rollback and external-dependency contract acceptance; and
- the same image, commands, configuration meanings and public API used by
  later Helm deployments.

Logical backup is the initial portable recovery mechanism. It uses the
PostgreSQL 17 `pg_dump`/`pg_restore` tools already paired with the server. It
does not claim WAL archiving, point-in-time recovery, disaster recovery or an
off-host copy. CPR-45 subsequently adds the portable S3-compatible target and
WAL/PITR path required by its original acceptance contract; owned RPO/RTO and
recurring production recovery drills remain OPS-5 work.

OpenTelemetry traces through the private Collector establish the core
application seam. CPR-45 still includes one bounded optional local backend and
a customer-safe Operations route; it does not expand into a general dashboard
platform or the complete SaaS support console.

## Options considered

1. **Continue the clean-engine provider model** — rejected because it tests a
   simulated provider-control protocol rather than the running product.
2. **Make Synveda install and supervise Docker or Colima** — rejected because
   container-engine ownership belongs to the host operator and materially
   widens Synveda's privilege boundary.
3. **Direct Compose acceptance against an operator-supplied engine** — chosen
   because it is the shortest path to browser, restart and recovery evidence
   from the actual deployment.
4. **Implement physical recovery before logical restore** — rejected as the
   first recovery slice. Portable logical restore lands first, followed by the
   bounded WAL/PITR and S3-compatible contract already required by CPR-45.

## Consequences

- Positive: repository complexity falls sharply and every remaining
  acceptance command corresponds to a real product lifecycle.
- Positive: no Synveda command needs host-VM, process-tree or container-engine
  provisioning authority.
- Positive: backup and the experimental executor can land as narrow,
  independently removable slices.
- Negative / accepted trade-off: a working supported container engine is an
  explicit prerequisite and its installation is outside Synveda.
- Negative / accepted trade-off: bounded S3-compatible and WAL/PITR acceptance
  still does not establish disaster recovery, owned RPO/RTO, HA, SaaS
  readiness, signed releases or Helm production readiness.
- Reversal trigger: if direct Compose cannot produce repeatable clean-volume
  acceptance on Linux and one Docker Desktop platform, the reference remains
  unvalidated; a smaller supported platform matrix is documented rather than
  reintroducing a provider simulator.

## Compliance notes

This decision changes deployment acceptance only. It does not alter Cedar,
forced RLS, VedaFlow, audit, tenant identity or the public API. The Apalis task
payload remains an opaque operation reference; Synveda's tenant-bound
operation row is authoritative. Backup credentials are never mounted into the
gateway, worker or telemetry containers. KMS material is mounted read-only only
into the product processes whose cryptographic work requires it.

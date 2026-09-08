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

Synveda has one application runtime, schema and public API, but it does not yet
have complete current-source evidence for a portable single-host installation.
The canonical Compose graph exists, while the installed release profile and
legacy contributor loop still carry Rauthy. Backup/restore and the experimental
Apalis canary are not implemented, and clean-volume Keycloak browser
acceptance has not run on the current source.

Static configuration checks are useful but do not prove that a user can sign
in, use the product, restart it, back it up or restore it.

### Decisions

[ADR-0102](../adr/adr-0102-portable-reference-deployment.md) defines Compose as
the canonical single-host deployment contract. [ADR-0105](../adr/adr-0105-direct-compose-acceptance.md)
removes the abandoned container-engine simulation and requires direct
acceptance against an operator-supplied supported Docker engine.

The reference topology is:

- Caddy as the only public service;
- PostgreSQL 17 with separate Synveda and Keycloak databases and roles;
- optimized production-mode Keycloak behind the generic OIDC/PKCE boundary;
- one Synveda product image with separate gateway and worker commands;
- a private OpenTelemetry Collector; and
- an optional, disabled-by-default Apalis 0.7.4 canary using opaque operation
  references only.

Development is explicit loopback HTTP. Reference/playground mode is HTTPS with
operator-owned DNS and certificate files. Neither mode is HA, disaster
recovery, production SaaS or enterprise certification evidence.

## Scope

- Keep the canonical Compose graph reproducible and provider-neutral.
- Run gateway and worker in containers with distinct non-owner database roles.
- Complete current-source browser login through bundled Keycloak, then remove
  Rauthy without a compatibility mode.
- Provide external OIDC configuration using the same gateway image and issuer
  semantics.
- Supply secrets through mounted files and reject direct/file ambiguity.
- Add a thin live acceptance and restart runner; unavailable prerequisites are
  never reported as pass.
- Add a PostgreSQL-native full logical backup of both databases and an isolated
  restore using separately supplied KMS key material.
- Add a portable S3-compatible target and WAL/PITR acceptance without claiming
  owned production RPO/RTO.
- Add one forced-RLS `skill_validation@1` operation/outbox and an optional
  leaf Apalis adapter. Keep the ordinary PostgreSQL execution path as rollback.
- Keep OpenTelemetry as the application interface and add one bounded optional
  local backend plus a customer-safe Operations route.
- Add data-preserving restart, upgrade/rollback and external-dependency
  contract acceptance.
- Keep Temporal absent.

## Non-goals

- No Docker/Colima installation, VM supervision or provider receipt engine.
- No high availability, node-loss tolerance, multi-region operation or
  zero-downtime upgrades.
- No disaster-recovery claim from same-host backup, or owned production RPO/RTO;
  OPS-5 owns those operational requirements and recurring drills.
- No Apalis board, general scheduler or migration of Capture, Sessions,
  Knowledge mutation, PDP, VedaFlow or audit.
- No general dashboard platform or complete SaaS support console.
- No full Helm promotion, SaaS controls, signing/provenance or compliance claim.
- No Keycloak-specific Synveda roles, grants, tenants or policy behavior.

## Architecture seam

The canonical files under `deploy/compose/` select bundled or external
PostgreSQL/OIDC modes for development and reference runtimes. The base graph
contains proxy, database preflight, migration, tenant convergence, issuer
diagnostic, gateway, worker and Collector services. Bundled fragments add
PostgreSQL, database bootstrap, Keycloak database bootstrap, Keycloak and realm
convergence.

Only the proxy publishes ports: loopback development HTTP or reference 80/443.
PostgreSQL, Keycloak management, worker health and OTLP remain private.
Long-running services are non-root where their upstream permits it, drop
capabilities, use read-only roots and have bounded health/restart behavior.

The product image has the closed commands `gateway`, `worker`,
`issuer-diagnostic`, `database-preflight`, `migrate` and
`tenant-converge`. Gateway and worker consume different mounted DSNs.
Runtime role checks refuse owner, superuser, `BYPASSRLS`, wrong-database and
authority-drift conditions before work.

The bundled Keycloak image is pinned, optimized with PostgreSQL, health and
metrics, and runs `start --optimized`. Realm convergence creates the public
PKCE client, audience/group claims and one-time `synveda-admins` admission
signal. The management port is private. Caddy exposes only required identity
paths and overwrites forwarding, identity and tracing headers.

The gateway runs in its container. The issuer is exact across browser,
discovery, tokens and containers. Development uses managed `.test` host
mappings; reference mode uses real DNS.

The core worker owns Capture, Knowledge indexing, relaxation expiry and
optional directory pull. PostgreSQL remains the authority for their existing
leased work. Temporal runtime assets have been removed after a no-consumer
inventory.

### Remaining implementation slices

1. Add `make compose-acceptance` as a thin wrapper around the existing real
   browser fixture and a bounded gateway/worker/Keycloak/PostgreSQL/proxy/
   Collector restart matrix.
2. Add full logical database backup plus isolated restore and KMS-key checks,
   then the bounded S3-compatible and WAL/PITR path.
3. Add the minimal `operation`, `operation_attempt` and
   `operation_outbox` schema/API for `skill_validation@1` under forced RLS.
4. Add the exact-pinned Apalis leaf adapter and optional Compose fragment.
5. Add the bounded local observability profile and customer-safe Operations
   route.
6. Add upgrade/rollback and external-dependency contract acceptance.
7. Run current-source development and reference acceptance on Linux and one
   Docker Desktop platform.
8. After the Keycloak gate passes, replace release/install assets with the
   canonical pull-only graph and delete all Rauthy residue.

## Acceptance criteria

- `docker compose config` validates every supported selector using no secret
  value in rendered output.
- A clean development project generates secrets, starts the full bundled graph
  and completes browser authorization-code + PKCE S256 login.
- The exact issuer, JWKS, audience, algorithm, callback and administrator-group
  rules pass; wrong issuer/audience, missing group and expired tokens fail.
- Gateway, worker, Keycloak, PostgreSQL, proxy and Collector each restart while
  persisted product state remains usable.
- A workspace, project, member invitation, Session, Capture candidate,
  accepted Knowledge item and clean-session context reuse complete through
  public APIs.
- Gateway and worker roles cannot access Keycloak; the Keycloak role cannot
  connect to Synveda; forced RLS remains enabled for tenant data.
- A full backup contains both databases and no plaintext credential. An
  isolated restore with the matching KMS key opens encrypted data and verifies
  the audit chain; a wrong key fails closed. The optional S3 target and
  WAL/PITR path pass their bounded configuration and recovery checks.
- The default PostgreSQL worker and optional Apalis adapter execute the same
  bounded skill validation once under duplicate dispatch, restart and
  cancellation tests. Queue payloads contain only operation ID/version.
- External OIDC renders and boots without a bundled Keycloak service using the
  same product image.
- The optional local backend shows bounded content-free gateway, worker,
  operation and recovery telemetry; Operations shows only authorised
  tenant-safe aggregates and all loading/degraded states.
- Upgrade from the previous test build preserves volumes and data, permits only
  a declared compatible rollback and refuses an unsafe rollback.
- Confirmed reset removes only the exact Compose project's resources.
- No active Temporal or, after identity cutover, Rauthy residue remains.

## Required tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
make check-deps
make check-deploy
make compose-config
make compose-acceptance
make compose-backup
make compose-restore-smoke
make compose-upgrade-smoke
```

Database and live Compose gates report unavailable when their prerequisites do
not exist. No deterministic test is relabelled as live evidence.

## Rollout and rollback

The canonical graph is additive until current-source Keycloak acceptance
passes. Rauthy is then deleted in one reviewed cut with no compatibility mode.
The Apalis fragment is disabled by default; removing it and selecting the
PostgreSQL operation provider is the rollback. Restore always targets a fresh,
confirmed project and never overwrites the source deployment.

## Dependencies

Live completion needs a supported Docker Engine, Linux and one Docker Desktop
host, browser trust for the selected development/reference issuer, and a
database-capable environment for SQLx metadata and forced-RLS acceptance.
Production PITR/off-host retention remains OPS-5; release signing/provenance
and published artifact verification remain production-readiness work.

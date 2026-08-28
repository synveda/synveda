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

Synveda has one application runtime and strong request/data trust boundaries,
but its single-host packaging is not yet an executable product reference. At
programme start the Compose files exposed infrastructure directly, bundled
Rauthy, retained an unused Temporal development service, ran the default
gateway on the host to work around a loopback issuer, and kept background work
inside the gateway. The core worker is now a separate process, but the other
packaging gaps remain: there is no joint database/key restore, release-image
parity, private Collector topology or clean-volume platform acceptance. Static
deployment convergence is valuable but does not prove that a user can install,
sign in, use, back up, restore or upgrade the product.

[ADR-0102](../adr/adr-0102-portable-reference-deployment.md) fixes the target
architecture. [The deployment contract](../DEPLOYMENT_CONTRACT.md) fixes its
provider-neutral commands, configuration and external dependency seams.

## Scope

- Make Docker Compose the canonical executable single-host reference topology
  for development, individual use and controlled small-team evaluation.
- Replace bundled Rauthy with production-mode Keycloak while retaining generic
  OIDC/OAuth 2.0 authorization-code and PKCE semantics.
- Run the gateway and background workers as separate commands in the same
  immutable product image.
- Put a sanitising reverse proxy in front of application and identity routes;
  keep databases, management, telemetry, worker and backup surfaces private.
- Supply secrets through mounted files and prove separate Synveda/Keycloak
  databases and least-privilege roles on a shared PostgreSQL server.
- Keep OpenTelemetry as the application telemetry contract through a private
  Collector, with an optional bounded local evaluation stack.
- Add a tenant-authorised, content-free Operations surface through generated
  public APIs.
- Evaluate pinned stable `apalis`/`apalis-sql` 0.7.4 behind a provider-neutral
  operation/outbox adapter on the inert Skill `validation_sandbox` test,
  disabled by default. Stable 0.7.4 has no Synveda business-idempotency or
  fencing contract and declares no MSRV, so Synveda state remains authoritative.
- Remove Rauthy and unused Temporal residue after their replacements pass.
- Add portable local pgBackRest backup, WAL/PITR and isolated restore evidence
  for PostgreSQL state plus separately held Synveda key material.
- Record exact source/deployment/image inputs and exercise restart, upgrade,
  external-dependency and clean-volume acceptance.

## Non-goals

- No high availability, node-loss tolerance, multi-region operation or
  zero-downtime upgrade claim.
- No production SaaS readiness, enterprise compliance certification, FIPS or
  customer-HSM claim.
- No Keycloak-specific role, grant, tenant or policy model in Synveda.
- No Apalis dependency in domain/public API crates and no replacement of PDP,
  RLS, VedaFlow, audit or Synveda operation state.
- No migration of Session ingestion, context planning, Capture publication,
  Knowledge mutation, tenant erasure or the whole background pipeline to
  Apalis.
- No MCP connectivity canary in this first experiment. It would add outbound
  network, SSRF, credential, custom-CA/proxy and process-execution boundaries;
  the existing MCP test surface records bounded trusted-adapter evidence and
  deliberately performs no connection from the gateway.
- No Compose-to-Kubernetes generator and no full Helm promotion in this slice.
- No claim that same-host backup is disaster recovery or that an external
  provider is supported from configuration-only evidence.
- No compatibility mode for Rauthy, Temporal or pre-epoch-3 databases.

## Architecture seam

Compose, direct binaries and later Helm use the same images, commands,
configuration meanings, schema, public API, OIDC semantics, OTLP endpoint and
backup evidence contract. Deployment mechanisms only change how values and
secrets arrive.

PostgreSQL remains business-state authority. The gateway authorises and records
governed operations; workers re-establish tenant context and use ordinary
forced-RLS transactions. Cedar authorises the worker against the immutable
already-authorised operation capability rather than reinterpreting the
requester's later grants. An operation and its outbox row commit together.
Apalis, when enabled for the canary, receives opaque operation references only
and is reconstructible delivery state.

The existing synchronous Skill-test endpoint remains unchanged as the
experiment's control/rollback. The new operation path selects exactly one
delivery provider with
`SYNVEDA_OPERATION_PROVIDER_SKILL_VALIDATION=postgres|apalis`; the explicit
Apalis Compose fragment changes routing and starts its two processes
atomically. Both paths call the same bounded validation function.

The initial reference may place Synveda, Keycloak and an experimental Apalis
database on one PostgreSQL server, but each has its own database, role and
migration owner. Physical backup therefore restores the server as one recovery
unit; logical authority and application access remain isolated.

Capture's existing durable job is safe for a restartable worker. Its
incremented attempt is the fence; renewal, completion
and failure require the exact tenant/batch/owner/attempt tuple and a lease that
is live at statement time. Stale completion checks precede candidate writes,
the lease is re-proved after preflight and before provider disclosure, lost
renewal abandons dependency output, renewal shutdown is bounded, and an
expired final attempt becomes an audited terminal failure. The core worker now
runs that loop separately from the gateway and has strict boot, private
readiness, readiness withdrawal and bounded cancellation/join. Real SIGTERM
during claimed Capture work and two-worker execution remain required; idle
process shutdown plus row fencing do not imply those results.

### Current implementation evidence

The product image contains separate `synveda-gateway` and `synveda-worker`
binaries behind one closed role entrypoint; release archives also carry both
direct binaries. The gateway has no
domain maintenance loops: Capture, Knowledge indexing, relaxation expiry and
optional directory pull are supervised by the worker. It still owns request
state, policy refresh, pool monitoring and startup KMS provisioning. The worker
refuses work until schema epoch, exact non-elevated runtime role that owns no
database and no schema/relation/routine in the selected Synveda database,
writable-primary state and initial
policy convergence pass, binds health/readiness/metrics to loopback, and treats an
unexpected critical task exit as fatal. A supervised authority sentinel
continues to re-prove epoch and runtime role; a conclusive refusal faults the
process, cancels every loop and exits non-zero rather than merely changing
readiness.

For the implicit bundled default, init verifies derived host-side `localhost`
URLs while Compose supplies the `postgres` alias to the containers. Any
explicit `DATABASE_URL`, including a loopback one, requires an explicit
runtime-URL pair; init verifies those exact host-side URLs against the bootstrap
cluster identity, database OID, live postmaster generation and writable-primary
state. The generation marker is compared only during bootstrap and is not
persisted across a database restart. The transitional raw
`.env`/Compose handoff has no accepted byte-for-byte evidence for arbitrary URL
characters. The worker therefore independently proves the resolved container
session's exact role/epoch/writable-primary state at boot and while running;
init's host-side check alone is not container-resolution evidence. The gateway
has no equivalent boot-time role sentinel yet. Helm
renders a private worker Deployment from the
same image and a separately owned Secret, and its install job refuses a mounted
worker URL targeting a different live primary instance/database. Helm's gateway still uses
the CloudNativePG application-owner Secret, and the transitional Compose
manifests still use a monolithic rendered environment file (the other runtime
DSN is explicitly masked in each service); both are explicit gaps before the
locked secret/role-isolation contract is satisfied.

The shared product database-URL boundary accepts only `postgres`/`postgresql`,
requires an explicit database path or effective `dbname`, and rejects fragments
or query keys not consumed by pinned SQLx before SQLx can log an ignored value.
Content-free unit, reset and real gateway/worker process sentinels prove wrong
schemes, ambient database fallback and unknown query secrets are refused
without disclosure.

Deterministic evidence covers worker boot outage, live/not-ready semantics,
exact role, any-schema ownership, elevation, unexpected membership and
membership-administration refusal, live `BYPASSRLS` drift causing non-zero
exit, bounded idle SIGTERM, configuration bounds and private Helm probes.
Compose leaves an 85-second outer stop window around the worker's 75-second
default join. Model-provider clients used by Capture and embedding validate
absolute credential-free HTTP(S) base URLs, refuse redirects, redact credential
debug output and expose only closed transport/status/parse errors. This evidence
does not yet cover clean canonical Compose, Keycloak, claimed-work process
interruption, multi-worker operation, backup/restore or upgrade.

## Acceptance criteria

- From a clean checkout and empty named volumes, secret generation, image
  build/pull and Compose configuration are deterministic and expose only the
  reverse proxy.
- Bundled Keycloak starts with `start --optimized`; its realm/client/group are
  converged idempotently; browser, gateway and CLI observe the exact same
  issuer; PKCE S256, audience, algorithm, JWKS and negative token cases pass.
- Only the first qualifying `synveda-admins` login seeds initial Synveda
  administration; later administrators are governed Synveda grants.
- Gateway and worker have separate processes, health/readiness, bounded work
  and graceful shutdown; gateway contains no newly introduced maintenance
  loops.
- Capture renewal runs during blocking extractor calls; wrong-owner,
  wrong-attempt, expired and same-owner-reclaimed executors cannot renew,
  fail or retain candidates, including when a caught conflict is deliberately
  committed. Expiry during preflight causes zero provider calls, blocked
  renewal is cancellable, and an expired final attempt becomes an inspectable
  audited failure.
- Synveda and Keycloak cross-database connections fail. Synveda gateway/worker
  roles own no database, schema, relation or routine and are non-superuser and
  non-`BYPASSRLS`; Keycloak owns only
  its own database/schema and has no Synveda access. The complete forced-RLS
  inventory passes.
- Direct/file secret ambiguity is refused; rendered configuration, logs,
  telemetry, manifests and image contents contain no sentinel secret.
- The experimental Skill validation operation passes duplicate, crash,
  two-dispatcher/two-worker, retry, cancellation, restart, malformed-envelope,
  cross-tenant and inline-rollback tests.
- A private Collector and optional local metrics/traces UI show bounded,
  content-free gateway, worker, operation/outbox age/retry/dead-letter,
  database, Keycloak/login, Session/delivery, Capture lag, context latency/token,
  Knowledge freshness/index, Skill/MCP test and backup signals. The Operations
  page handles loading, empty, degraded, stale and failure states without
  content, secrets, denied counts or provider task IDs.
- A full backup plus WAL restores both databases into isolated volumes with
  the correct key bundle; Keycloak login, encrypted tenant-secret opening,
  Knowledge/index integrity, forced RLS and the frozen audit prefix pass;
  wrong keys fail closed without falsely claiming Knowledge envelope
  encryption.
- The committed literal N-1 fixture upgrades Keycloak 26.7.1 → 26.7.2 and
  Synveda epoch-3 migration head `0001` → `0002`; per-service restart and
  idempotent convergence preserve labelled volumes and sentinels, unsafe
  rollback is refused, and migrated rollback uses the paired backup rather
  than a Keycloak/database downgrade. No zero-downtime claim is made.
- External OIDC/DB/OTLP/S3/custom-CA/proxy/private-registry configuration is
  validated with the same product image; live support is claimed only for
  providers actually exercised.
- Rauthy and unsupported Temporal tracked residue reach zero after cutover.
- Clean reference acceptance passes on Linux Docker and at least one Docker
  Desktop platform before the verdict can be “validated for controlled
  single-host use.”
- The complete lifecycle also passes in explicit development HTTP mode;
  deterministic external-OIDC diagnostics are not reported as live-provider
  conformance.
- The real Compose lifecycle creates a workspace/project and second member,
  ingests one verified-harness Session, captures/accepts Knowledge, reuses it
  with provenance in a clean Session and proves private/cross-tenant isolation.
- Every compatible container has `no-new-privileges`, no effective capability,
  bounded CPU/memory/PIDs, deterministic names and no privileged mode, Docker
  socket, host namespace or undeclared host port.

## Required tests

- Keep all current Rust, TypeScript, RLS, PDP, audit, generated-contract,
  adapter, licence, release and evaluation gates.
- Add `make compose-config`, `compose-up`, `compose-smoke`, `compose-down`,
  `compose-reset`, `compose-acceptance`, `compose-backup`,
  `compose-restore-smoke` and `compose-upgrade-smoke`.
- Add deterministic OIDC/provider-file/configuration and Keycloak realm
  convergence tests plus live browser/container/CLI conformance.
- Add container inspection tests for user, capabilities, read-only roots,
  ports, networks, secrets, forwarded headers and private management paths.
- Add operation/outbox/attempt forced-RLS and failure-matrix tests.
- Keep deterministic Capture lease tests for statement-time expiry, renewal,
  same-owner reclaim, stale-result containment, one winning completion and
  final-attempt terminalisation; do not infer exactly-once provider calls,
  graceful process drain or HA from row fencing alone.
- Distinguish operation/outbox commit then dispatcher crash, submit then
  acknowledgement-write failure, duplicate dispatch, two dispatchers, two
  workers and worker SIGTERM; none may create a duplicate governed effect.
- Add telemetry field/label allowlists and content/secret sentinel scans.
- Add isolated full/WAL/PITR restore, correct/wrong key and database isolation
  tests.
- Add a checked literal `upgrade-from.json`, the complete ordered
  migration/restart/rollback matrix from the deployment contract, volume
  identity checks and refusal of mutable/current-image fixtures.
- Add `make check-runtime-residue`: zero active Rauthy/Temporal
  runtime/config/support references, with historical ADRs and narrow negative
  fixtures as the only allowlists.
- Run proprietary live harness acceptance only when its real executable and
  credentials exist; otherwise record the missing prerequisite.
- Keep one runnable CPR-45 acceptance script under `demos/`; static Compose
  rendering is a separate gate and cannot satisfy the feature by itself.

## Rollout and rollback

Land architecture, static topology and configuration gates first. Bring up
Keycloak alongside the old development state only during local cutover; delete
Rauthy after the new identity acceptance passes. Keep the existing synchronous
Skill test as the Apalis experiment's rollback. Retain the last verified
database/key backup and exact image manifest through upgrade testing. A failed
reference rollout returns to the preceding commit and its untouched volumes;
it never translates old schema eras or fabricates a downgrade.

## Dependencies

OPS-5 owns production backup/DR beyond same-host validation. OPS-6 owns the
post-1.0 compatibility window. OPS-7 owns horizontal gateway correctness.
OPS-8/OPS-9 own published release and hosted evaluation evidence. TEN-5/TEN-6,
AUTH-6, EVAL-6 and CPR-39 remain independent lifecycle, isolation, token,
capacity and second-client work. Owners must still choose public DNS,
certificates, off-host storage, RPO/RTO, custody, release signing, supported
platforms and legal licence terms.

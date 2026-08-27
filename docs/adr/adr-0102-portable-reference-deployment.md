# ADR-0102: Portable reference deployment contract

- **Status**: Accepted
- **Date**: 2026-08-27
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Context

Synveda has converged on one context-platform runtime, schema and public API,
but its deployment evidence is split between a contributor Compose file, a
Rauthy-based installed profile and an external-IdP Helm chart. The bundled
issuer forces the default gateway onto the host; stale Temporal services have
no executable consumer; background work runs inside the gateway; secrets are
environment values; and no joint database/key restore exists. These shapes do
not yet provide one reproducible product contract or justify availability,
recoverability or SaaS claims.

The deployment must preserve the trust boundary: Cedar decides every
application act, forced RLS is the database backstop, governed mutations use
VedaFlow and content-free audit, and deployment/provider selection grants no
domain authority.

## Decision

Docker Compose is the canonical executable single-host **reference**
deployment. Direct binaries, Compose and later Helm use the same product
images, application commands, configuration meanings, health endpoints,
schema, public API, OIDC semantics, OTLP interface and backup evidence
contract. Helm will implement that contract with Kubernetes-native resources;
it will not be generated from Compose.

The reference bundles an optimized production-mode Keycloak and replaces
Rauthy completely after conformance. Synveda remains a generic OIDC/OAuth 2.0
authorization-code + PKCE client: Keycloak groups may signal the one-time first
administrator bootstrap, but all continuing roles and grants are Synveda data
decided by Cedar.

Gateway and worker are separate commands in one immutable product image.
PostgreSQL remains business-state authority. Pinned stable
`apalis`/`apalis-sql` 0.7.4 is an optional, disabled-by-default delivery
experiment behind a transactional Synveda operation/outbox seam; it receives
opaque identifiers only, imports into no domain or public-contract crate and
retains the synchronous canary path as rollback. Stable 0.7.4 supplies no
Synveda business-idempotency/fencing guarantee and declares no MSRV, so those
properties remain Synveda contracts and the exact graph is compiled/tested on
the repository toolchain. Temporal is not a target and unused residue is
deleted.

The canary's provider-neutral operation path uses the per-kind routing key
`SYNVEDA_OPERATION_PROVIDER_SKILL_VALIDATION=postgres|apalis`. PostgreSQL
worker delivery is the default; one explicit Compose fragment changes routing
and starts the Apalis dispatcher/executor atomically. The existing synchronous
Skill-test API remains unchanged as the control and rollback, and all three
delivery call sites share one validation function. CPR-45 adds ordinary
forward migration `0002_portable_operations.sql` without changing schema
epoch 3.

The gateway persists immutable authorization evidence with the operation and
outbox. At execution Cedar permits a narrow worker identity to consume that
already-authorised operation capability under ordinary tenant RLS; the worker
does not rerun the requester's later grants, acquire domain authority from the
queue provider or reinterpret the command. Cancellation and precondition
failure are explicit operation transitions, and the effect still uses the
normal VedaFlow/store/audit path.

The canary wraps the existing inert Skill `validation_sandbox` test as
operation kind `skill_validation`, version `1`. MCP connectivity is not the
first canary: the current MCP test boundary deliberately records bounded
trusted-adapter evidence, while direct connectivity would add outbound
network, SSRF, credential, proxy/custom-CA and process-execution risks unrelated
to evaluating the queue adapter.

Secrets are mounted files or external references. The issuer schema no longer
embeds directory client secrets/tokens: it holds scoped credential-file
references. Local Compose uses the validated non-root operator UID/GID so
mode-0600 secret bind mounts work without unsupported ownership remapping, and
Linux plus Docker Desktop acceptance must prove the boundary. OpenTelemetry is the sole
application telemetry protocol through a private Collector. PostgreSQL,
OIDC, optional S3-compatible backup storage and OTLP are provider-neutral
external seams. A shared reference PostgreSQL server may host separate
Synveda, Keycloak and experimental queue databases/roles, but their access and
migration ownership never merge.

## Options considered

1. **Keep contributor and installed Compose separate** — preserves current
   scripts but keeps divergent topology, issuer and secret behavior.
2. **Use Helm as the source and generate Compose** — makes local validation
   depend on Kubernetes concepts and does not prove the single-host contract.
3. **Keep Rauthy or offer an IdP switch** — reduces immediate cutover work but
   creates a compatibility product the programme explicitly does not support.
4. **Use Apalis or Temporal as business authority** — duplicates Synveda
   operation/VedaFlow state and weakens transactional PDP/RLS/audit evidence.
5. **Selected approach** — one explicit deployment contract, Compose as its
   first executable reference, and replaceable adapters at external seams.

## Consequences

- A clean Docker host becomes the first place the complete product topology is
  validated before Kubernetes or hosted promotion.
- Only the reverse proxy is public. Management, data, worker, telemetry,
  optional board and backup surfaces are private by construction.
- The exact issuer must be reachable identically by browser and containers.
  Development uses reserved `.test` host mappings plus Docker aliases.
  Although `.localhost` was an illustrative programme hostname, RFC 6761
  reserves it for each resolver's own loopback; accepting a Docker DNS alias
  as an override would make the exact-issuer contract platform-dependent.
  Reference/playground uses real DNS and HTTPS.
- One physical PostgreSQL cluster means one WAL/PITR recovery unit even with
  correctly isolated databases and roles. Independent RPOs later require
  separate clusters.
- Same-host backup, single gateway/host and local dashboards are evaluation
  evidence, not DR, HA, SaaS or enterprise evidence.
- Keycloak/database downgrade and schema rollback remain constrained by the
  tested version window; zero downtime is not promised.
- Reversal trigger: if measured operation failure semantics cannot be made
  correct behind the provider-neutral outbox, remove Apalis and retain the
  Postgres worker path. If single-host Compose cannot reproduce the contract on
  Linux and a Docker Desktop platform, the reference topology is not accepted.

## Compliance notes

Deployment mode, IdP and executor never bypass Cedar, forced RLS, VedaFlow or
audit. Gateway/worker use ordinary non-owner roles; migrations and backup use
separate operator identities. Queue payloads, telemetry and operational APIs
contain no prompts, messages, Knowledge bodies, credentials or denied-resource
counts. Destructive reset/restore remains explicit and scoped to resolved
volumes.

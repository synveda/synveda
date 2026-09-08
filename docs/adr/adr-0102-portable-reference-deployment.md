# ADR-0102: Portable reference deployment contract

- **Status**: Accepted; amended by ADR-0105
- **Date**: 2026-08-27
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Amendment

ADR-0105 replaces the abandoned clean-engine provider planning, receipt,
reservation and process-effect work, and supersedes ADR-0103 and ADR-0104.
Synveda now treats a supported Docker engine as an operator-owned prerequisite
and tests the canonical Compose graph directly. No provider-state format from
that scaffolding is a compatibility contract.

ADR-0105 also narrows initial recovery evidence to PostgreSQL 17 logical full
backup and isolated restore for the Synveda and Keycloak databases plus the
separately supplied KMS key/reference and surviving Keycloak convergence
credential. That is the Docker-reference recovery contract. S3-compatible
encrypted retention, WAL/PITR, owned RPO/RTO, off-host retention policy and
recurring production drills remain OPS-5 work.

## Context

Synveda had one context-platform runtime and public API, but deployment assets
were split between a contributor Compose file, a Rauthy-based installed
profile and an external-IdP Helm chart. The bundled issuer forced the gateway
onto the host, stale Temporal services had no consumer, background work ran in
the gateway, secrets were passed as environment values, and no joint
database/key restore existed.

The deployment must preserve the product trust boundary: Cedar decides every
application act, forced RLS is the database backstop, governed mutations use
VedaFlow and content-free audit, and no infrastructure provider supplies
tenant or domain authority.

## Decision

Docker Compose is Synveda's canonical executable single-host **reference**
deployment. Direct binaries, Compose and later Helm use the same product
images, commands, configuration meanings, health endpoints, schema, public
API, OIDC semantics, OTLP interface and backup commands. Helm implements the
contract with Kubernetes-native resources; it is not generated from Compose.

The reference graph contains a reverse proxy, PostgreSQL, production-mode
Keycloak, a gateway, a separate worker and a private OpenTelemetry Collector.
Only the reverse proxy publishes public host ports. The optional local metrics
profile may publish its operator UI on host loopback only. Development may use
explicit loopback HTTP; reference/playground mode uses HTTPS and real DNS.

Keycloak replaces Rauthy after the existing browser and issuer conformance gate
passes. Synveda remains a generic OIDC/OAuth 2.0 authorization-code plus PKCE
client. Keycloak groups may seed the one-time first administrator only;
Synveda grants and Cedar remain authoritative. Bundled and external OIDC use
the same issuer configuration and verifier.

The exact issuer URL must be identical and reachable from the browser, gateway,
CLI login and discovery/token endpoints. Bundled development uses explicit
host mappings and a Docker network alias; reference mode uses operator-owned
DNS. The gateway always runs in its container.

A shared PostgreSQL server may host separate Synveda and Keycloak databases.
They use separate owners and logins. Gateway and worker are distinct ordinary
non-owner roles with access only to Synveda; Keycloak has access only to its
database. Deployment bootstrap owns database/role creation and extensions,
while the Synveda migrator owns only the Synveda database/schema lifecycle.

Gateway and worker are separate commands in one immutable product image.
PostgreSQL remains business-state authority. Existing Capture, indexing,
relaxation-expiry and directory-pull work runs in the worker; new long-running
work is not added to the gateway.

Pinned stable `apalis` and `apalis-sql` 0.7.4 are evaluated only for one
disabled-by-default `skill_validation@1` operation. A tenant-bound operation
and outbox row commit together. The adapter receives only operation ID and
version, imports into no domain/public API crate, and cannot replace Cedar,
RLS, VedaFlow, audit or the public operation model. The ordinary PostgreSQL
worker path remains the rollback.

Secrets are mounted files or external references. Direct/file ambiguity is a
startup error, values are never logged, and any upstream entrypoint that must
export a secret reads it only from `/run/secrets` and then `exec`s. The KMS
key is backed up and restored separately from PostgreSQL state.

OpenTelemetry is the application telemetry interface. The reference Collector
is private and may export to an external OTLP backend without application
changes. Optional local visualization is evaluation tooling, not a product
authority or production-readiness claim.

Temporal is not part of the target. Its unused runtime assets are removed.
There is no compatibility profile.

This ADR also permits distinct non-secure development cookie names when, and
only when, explicit development HTTP is enabled. HTTPS retains the secure
`__Host-` cookie contract. The OIDC client requests `offline_access` only
when its client-specific `login_scopes` configuration includes it;
provider-wide discovery advertising alone grants nothing.

## Options considered

1. **Keep contributor and installed Compose separate** — rejected because it
   preserves divergent issuer, secret and process semantics.
2. **Generate Compose from Helm or Helm from Compose** — rejected because each
   platform needs native lifecycle primitives while sharing the application
   contract.
3. **Retain Rauthy as an alternate bundled provider** — rejected because it
   creates a compatibility product with no target user need.
4. **Use Apalis or Temporal as business authority** — rejected because it
   duplicates Synveda operation, policy and audit state.
5. **Canonical Compose plus provider-neutral external seams** — chosen.

## Consequences

- A clean Docker host is the first place the whole single-host product is
  validated before Kubernetes or hosted promotion.
- Same-host backup, one gateway/host and local telemetry are controlled-use
  evidence, not HA or disaster recovery.
- A shared PostgreSQL server is one failure domain even though its databases
  and roles are isolated.
- Keycloak and database downgrade remain constrained by tested restore and
  upgrade paths; zero downtime is not promised.
- If Apalis failure semantics cannot preserve the Synveda outbox contract, its
  fragment and leaf adapter are removed while PostgreSQL execution remains.
- If the graph cannot pass on Linux and one Docker Desktop platform, the
  reference remains unvalidated rather than weakening its issuer or trust
  contract.

## Compliance notes

Deployment mode, identity provider and executor never bypass Cedar, forced
RLS, VedaFlow or audit. Queue payloads, telemetry and operational diagnostics
contain no prompts, messages, Knowledge bodies, credentials or denied-resource
counts. Reset and restore require exact project-scoped confirmation.

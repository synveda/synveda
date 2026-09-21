# ADR-0102: Portable reference deployment contract

- **Status**: Accepted; amended by ADR-0105, 2026-09-19 and 2026-09-21
- **Date**: 2026-08-27
- **Feature(s)**: CPR-45, OPS-12
- **Deciders**: Synveda maintainers

## Amendment

The 2026-09-21 consumer increment renders the existing evaluation fragments at
package time with Docker Compose itself. It does not maintain another service
graph. The generated root Compose file includes that rendered graph. A bounded,
networkless initialization service owns one private named installation volume;
services receive only their existing secret subset through read-only volume
subpaths. The initializer drops to the runtime UID before invoking the existing
secret and issuer preparation. No Docker socket is mounted.

The consumer project uses `synveda-local`, separating its named-volume layout
from the existing launcher's host state. No implicit transition of retained
state between those layouts is supported. Initialization refuses missing keys
beside retained PostgreSQL data and binds its state to the release, project and
issuer options. It preserves existing secrets and compares projected bytes on
retry. Normal Compose shutdown keeps both named volumes. This path remains a
local candidate until fresh-start, browser, recreation and paired-recovery
acceptance qualifies it; the published launcher remains the supported path in
the meantime. Existing external-provider and logical-recovery contracts remain
authoritative.

OPS-12 extends that same logical recovery ceremony to the consumer volumes.
The candidate's recovery command stops the entire source graph, dumps the pair
with the existing PostgreSQL tools, and links the original sealed installation
configuration through `evaluation-recovery.mjs` and `recovery-set.mjs`. Backups
live in a separate retained recovery volume; their format and encryption limits
are unchanged. A restore names source, backup and target in the existing exact
confirmation, refuses any retained target assets, and explicitly rebinds only
the project in the recovered installation identity. The source must be down;
the target uses the same issuer/network options with no published proxy port.
Before normal tenant convergence, the ordinary recovery verifier checks the
restored tenant, audit chain and key unwrap, including a wrong-key refusal.
Recovery holds an installation operation marker; interruption retains it for
operator inspection. Direct Compose operations must not run concurrently with
recovery. The host-state launcher continues to address only its own layout.

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

The 2026-09-19 development retry found an unchanged hosts mapping and backup
whose persisted filesystem device number alone had changed. Read-only status
and normal lifecycle checks continue to require the complete saved witness.
An explicitly confirmed, privileged hosts installation may renew that witness
only after reading the protected backup and verifying its nonce, selection,
source-to-installed digest, exact installed hosts bytes and metadata, and every
other saved backup witness field. It preserves the hosts inode and backup.
Under the existing mutation lock, it removes only the verified public receipt
and reuses interrupted-install recovery to publish a fresh one. A failure in
between leaves startup refused and the same confirmed installation retryable.
Removal, unknown drift and unowned mappings retain their existing refusals.

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
and outbox row commit together. The adapter receives only an untrusted tenant
routing identifier, operation ID and version, then rechecks the immutable
tenant association and current authority under forced RLS. It imports into no
domain/public API crate and cannot replace Cedar, RLS, VedaFlow, audit or the
public operation model. Its separate private queue database uses an isolated
bootstrap owner only for setup and a converged least-privilege runtime role for
the long-running worker. The ordinary PostgreSQL worker path remains the
rollback, and queue data is outside product recovery.

Because that database is provider-owned disposable transport state,
`synveda-apalis` rather than `synveda-store` owns its queue library and bounded
bootstrap and role-convergence SQL. This is a narrow SQL-placement exception:
the leaf cannot query Synveda product state or export provider schema or types,
and deterministic contracts pin its fixed queue authority, least privilege and
opaque task shape. Its one-shot migrator may use server-side `format(%I, %L)`
only to quote fixed role identifiers and the mounted credential safely.

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

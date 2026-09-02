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

This ADR amends ADR-0027 decision 6: `login_scopes` is the client-specific
authority for requesting `offline_access`. Discovery advertising that
provider-wide scope is necessary but never sufficient. The bundled public
Keycloak client omits it and uses ordinary authorization-code refresh tokens;
live identity acceptance must prove that behavior.

This ADR also amends ADR-0056 decision 2 for one explicitly selected
development-only case. HTTPS retains the exact
`__Host-synveda_console`/`__Host-synveda_login`, `Secure`, host-only,
`HttpOnly`, `SameSite`, path and lifetime contract. When startup validation
accepts a non-loopback plaintext public URL only because
`SYNVEDA_INSECURE_DEVELOPMENT_HTTP=true`, the gateway instead uses the
distinct `synveda_console_dev` and `synveda_login_dev` names, sets no `Domain`
attribute, and omits only `Secure`. Any HTTP origin selects these names when
the flag is explicitly true; non-loopback HTTP additionally requires the flag
to pass startup validation. Origin enforcement, token re-verification,
duplicate-cookie refusal, `HttpOnly`, `SameSite`, path and lifetimes are
unchanged. HTTPS never selects this relaxation merely because the setting is
present, and development cookies cannot become HTTPS session credentials.

The reference bundles an optimized production-mode Keycloak and replaces
Rauthy completely after conformance. Synveda remains a generic OIDC/OAuth 2.0
authorization-code + PKCE client: Keycloak groups may signal the one-time first
administrator bootstrap, but all continuing roles and grants are Synveda data
decided by Cedar. The bootstrap is an insert-only forced-RLS tenant marker
committed with the first root-administrator grant. A grant created through any
other path consumes the same marker, and revocation never returns authority to
the identity provider.

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

Application migration runs under an ordinary `synveda_migrator`-equivalent
login which owns only the selected Synveda database and its `public` schema.
Deployment bootstrap separately owns role/database creation and extension
installation. Gateway and worker are distinct non-owner `synveda_app` members
with direct CONNECT only to Synveda; the same authority split applies to
Compose and Helm and is validated before either runtime becomes ready.

Deployment tenant convergence uses the key-plane repair contract in ADR-0064
amendment 3. It first proves current-key unwrap custody, then reads the
authoritative generation-1 row and converges its exact content-free
`tenant.key.provisioned` witness while holding the tenant audit-chain head.
This is the narrow repairable exception to same-transaction mutation/audit:
an external KMS call is never held inside the tenant-admission transaction,
and a crash after the key commit is repaired by the exact rerun. Historic
exact duplicate witnesses are retained without extension; malformed
generation-1 candidates fail closed, while a later-generation legacy event is
a different fact and cannot prevent generation-1 repair.

The clean-Engine provider seam now has two fake-only execution paths. The
original synchronous closed-data adapter remains the rollback. The controlled
path creates a private, receipt-reserved external filesystem root, mirrors its
inode/mode/UID-bound owner marker, publishes a durable launch record and starts
one detached actor which remains its process-group leader. The supervisor
reasserts the actual mutation slot immediately before its one-way decision;
the actor independently validates the root plan, complete root/leaf inventory,
owner mirror, launch and witness, including their digest-bound slot fields,
before its fixed fake child can start. Only the actor signals its own group;
its directly owned children also exit if their private parent IPC closes. A
provider result can close only after a negative PGID probe reports `ESRCH`.
The settlement binds the exact optional effect and outcome digests. After
settlement, the supervisor copies the exact fixed effect, when present, into
append-only state and publishes a provider identity that binds the slot,
intent, fixed-fake contract, root plan/owner, settlement and closed fake
resource dispositions. The mutation close and a passing receipt bind that
identity digest; the receipt also labels the evidence explicitly as
`controlled-fake`, so it cannot be reclassified as live provider evidence.
Immediately before linking a close, the publisher re-proves its authority,
result endpoints, operation evidence and staged inode/bytes.
Recovery never signals a stored PGID or replays a durable start. A
marker-before-mirror crash converges only an exact reservation-bound marker;
crashes after effect mirroring or identity publication converge the same
identity without replay, and a launch without a witness remains an explicit
blocker because its PGID was not durably recorded.

This is deterministic POSIX process/filesystem evidence for the fixed fake
command and actor-owned descendant, not Docker, Colima or live-provider
evidence. The controlled root has no deletion settlement yet, so controlled
runs cannot publish provider-cleanup or finalization evidence. Receipt schema
v3 separates deterministic-fixture from controlled-fake evidence and requires
future live evidence to add a distinct reviewed shape. It binds provider
success to its intent contract; receipt v1/v2 state is refused and regenerated.
Mutation-close schema v2 adds the operation-evidence
digest and remains a fresh-plan hard cut; v1 closes are refused, not translated.

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
  Bundled-OIDC development uses reserved `.test` host mappings plus a
  Docker alias for the identity host. External OIDC maps only the application
  host locally and retains the provider's DNS and edge.
  Although `.localhost` was an illustrative programme hostname, RFC 6761
  reserves it for each resolver's own loopback; accepting a Docker DNS alias
  as an override would make the exact-issuer contract platform-dependent.
  One reversible host-file helper owns the exact development block. It accepts
  only the root-owned, single-link mode-0644, ACL-free host file and ACL-free
  physical parent, then retains the existing inode and non-ACL security
  metadata by appending or truncating only the terminal suffix. An exact
  interrupted prefix is recoverable by the next confirmed action rather than
  claimed old-or-new atomic. Its fixed root-owned, non-writable, ACL-free Node
  runtime/path and empty environment reduce accidental inputs, but elevation
  still trusts the clean, reviewed operator-writable checkout. Ordinary
  preflight proves ownership before Docker contact, while reference mode,
  `down` and confirmed `reset` never acquire host-file authority. Unmarked or
  overlapping aliases are refused, not adopted.
  Reference/playground uses real DNS and HTTPS.
- Explicit plaintext development is not a secure transport. Its distinct
  host-only cookie names make the limitation usable for local validation
  without weakening or reusing the HTTPS cookie contract; it is never
  reference/playground evidence.
- One physical PostgreSQL cluster means one WAL/PITR recovery unit even with
  correctly isolated databases and roles. Independent RPOs later require
  separate clusters.
- Same-host backup, single gateway/host and local dashboards are evaluation
  evidence, not DR, HA, SaaS or enterprise evidence.
- Keycloak/database downgrade and schema rollback remain constrained by the
  tested version window; zero downtime is not promised.
- The controlled clean-Engine actor/root seam remains internal and fake-only.
  Colima 0.10.3 `--foreground` keeps only its controller in the actor group; it
  does not make the Lima host agent an owned descendant. A live adapter must
  therefore lock a truthful background-instance model (or a genuinely owned
  replacement), close its transitive helper and disk-image identity,
  distinguish the host-agent and Engine sockets, and publish exact provider
  deletion settlement. A public runner and source/image environment manifest
  remain required before it can support a Docker or Colima acceptance claim.
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

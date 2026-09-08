# Context-platform security model

This document records the current security boundary and repeatable adversarial
evidence for Synveda's context platform. It is not a claim that one process
remains secure after compromise of its host, database superuser or signing
authority.

## Trust boundaries

- The gateway is the only public application service. Console, CLI and
  adapters use its generated public API.
- A separate private worker runs Capture, Knowledge indexing, relaxation
  expiry and optional directory pull with an ordinary runtime role.
- Every application read and write is decided by Cedar. Navigation
  capabilities predict UI state but never grant server authority.
- Tenant tables enable and force PostgreSQL RLS. Each request and worker unit
  enters an ordinary tenant-scoped transaction.
- Governed mutations use typed VedaFlow changes and retain content-free,
  hash-chained audit evidence.
- Reset, schema migration, role bootstrap and first-operator bootstrap are
  explicit bounded operator exceptions, not application paths.
- Session events are untrusted client assertions. The gateway validates,
  redacts and binds them to an owned session, then assigns authoritative order
  and digest.
- Skill, Tool and OKF inputs are metadata or inert content at the gateway.
  Bundle scripts and local stdio commands never execute in the gateway.
- MCP connection tests admit only the exact read-only discovery method set.

The gateway and worker database logins are inheriting, non-elevated members of
the safe NOLOGIN synveda_app role. They own no database, schema, table or
routine and cannot administer role membership. The worker re-proves its role,
schema epoch and writable-primary target; conclusive drift is fatal, while a
transient outage withdraws readiness and retries.

Helm also mounts role-scoped gateway and worker DSNs and rejects migrator,
owner and superuser credentials at those runtime boundaries.

## Authorisation and disclosure order

Normal reads resolve tenant-owned existence, obtain a Cedar decision, then load
or project content. Retrieval rechecks policy before anchors, after expansion
and before rendering. Denied candidates do not appear by ID, title, edge,
score, count, reason or rendered-context fingerprint.

Normal writes authenticate, resolve tenant ownership, validate preconditions,
obtain a Cedar decision, enter VedaFlow, re-authorise at effect time, apply the
immutable state transition and append audit evidence in the same transaction
where applicable. Stale, made-up and cross-tenant identifiers cannot be used
as existence oracles.

HTTP request bodies inherit Axum's 2 MiB default limit unless a route installs
a tighter one. Domain DTOs and import/archive readers add field, entry,
artifact, expansion and total-size bounds where their risk is higher. No
production route uses an unbounded raw-body extractor as a bypass.

## Secrets and sensitive content

Tool, provider and per-tenant directory credentials are stable secret
references. APIs, generated clients, audit and console state expose reference
status/version metadata, never plaintext.

Knowledge source payloads and session event bodies require narrower authority.
Timelines and audit records carry identifiers, hashes, counts, timestamps and
content-free summaries. Governed forget removes authorised plaintext,
embeddings and owned source payloads while retaining a content-free tombstone
and chain evidence.

Capture and embedding clients use bounded requests, refuse redirects and map
transport, status and parse failures to closed diagnostic codes. Provider
responses and credentials are never copied into application errors.

The canonical Compose deployment uses mounted mode-0600 secret files for:

- migrator, gateway and worker PostgreSQL URLs;
- PostgreSQL/Keycloak bootstrap and runtime passwords;
- Keycloak bootstrap and convergence administrators;
- the Synveda KMS key and key reference;
- TLS private key material;
- demo credentials when the demo profile is selected.

The selector rejects equivalent direct secret environment variables. The
Keycloak entrypoint reads upstream-required values from files, exports them
only to its child, never prints them and execs the real process. Generated
manifests and Compose configuration must remain content-free.

The deployment-level directory connector in the issuer configuration remains a
temporary exception pending role-specific file projection. Helm also projects
one issuer Secret to both gateway and worker; this remains an on-prem gap.

## Docker reference boundary

The canonical Compose lifecycle assumes an operator-supplied local Docker
Engine. Synveda does not install or supervise Docker, Docker Desktop, Colima or
another VM/provider.

The lifecycle:

- requires a local Unix Docker socket and refuses remote contexts;
- validates a closed runtime/provider/profile vocabulary;
- binds every action to one exact Compose project;
- refuses overlapping foreign networks;
- keeps generated authority and secrets outside committed inputs;
- distinguishes unavailable Docker prerequisites from passing evidence;
- requires exact confirmation for hosts-file mutation and reset.

Docker CLI proxy defaults could put credential-bearing proxy URLs into
container metadata or implicit build arguments. The graph explicitly empties
upper- and lower-case HTTP, HTTPS, NO, FTP and ALL proxy variables for every
runtime service and development build. Render checks and asset inspection
enforce that closure without printing a rejected value.

Development builds refuse ambient BuildKit, Buildx and Bake controls, pin the
validated local Engine endpoint, use a private Buildx state directory and
select only the embedded local docker driver. Registry authentication remains
opaque. Docker/Compose/Buildx binaries, credential helpers, daemon mirrors,
daemon proxy/CA and embedded BuildKit policy are part of the trusted host.

The former fixture-only clean-engine/Colima preparation framework was removed
under ADR-0105 because it never executed a provider or established live
evidence. Direct lifecycle acceptance now tests the actual product graph.

## Container and network boundary

The reference graph applies:

- non-root runtime users where upstream images permit;
- read-only root filesystems where compatible;
- all capabilities dropped, with only the PostgreSQL entrypoint's required
  startup capabilities added;
- no-new-privileges;
- PID, memory and CPU bounds;
- init/signal handling and bounded shutdown;
- no privileged containers and no Docker socket mount.

Only Caddy publishes public ports. The optional Prometheus operator UI binds to
host loopback only. PostgreSQL, gateway/worker metrics, worker health, Keycloak
management, OTLP receivers and recovery services are private.
Separate networks isolate the public edge, application, Synveda data, Keycloak
data/management and telemetry. Only discovery/export components join explicit
egress networks.

Caddy removes untrusted Forwarded, X-Forwarded-*, X-Real-IP, original-path,
identity and distributed-tracing/baggage headers before adding its own values.
Header size, request size, read/write/idle time and upstream dial/response time
are bounded. The public Keycloak host exposes only required realm discovery,
authorization, token, JWKS, logout, login-action, account and static-resource
paths.

Development is explicit loopback HTTP. Reference mode requires operator DNS
and HTTPS certificate files. The lifecycle validates certificate structure,
chain adjacency, key match, SANs and remaining validity; it does not establish
public trust, DNS ownership or revocation status.

## Identity boundary

Synveda consumes standards-compliant OIDC/OAuth 2.0. It validates exact issuer,
audience and supported RSA algorithms, discovery, JWKS and PKCE S256. Provider
groups may seed first-administrator admission, but Synveda grants and Cedar
remain authoritative.

Bundled Keycloak uses a dedicated database and role. Synveda gateway/worker
roles cannot connect to it, and Keycloak has no Synveda-table privileges.
Keycloak application traffic passes through the public gate only after exact
realm convergence; its management port and administration surface remain
private.

Rauthy remains in withdrawn/transitional contributor, CLI init, install,
package, release, smoke and fixture paths until live current-source
Keycloak/browser acceptance passes. It is not a supported alternate provider
mode and must then be deleted rather than retained as a compatibility path.

## Worker and queue boundary

The private worker readiness surface proves schema epoch, exact role, writable
primary, initial policy convergence, process lifecycle and supervisor
heartbeat. Its direct-binary default accepts only loopback. The observability
overlay must set the exact non-loopback relaxation before an unspecified bind
is accepted, publishes no worker port and attaches no new network. Any other
address or flag value fails startup. A heartbeat is not proof that each work
loop progressed.

The experimental Apalis canary is still open. It may be accepted only with:

- a provider-neutral operation, attempt and transactional outbox model;
- forced RLS on tenant rows;
- the Synveda operation ID as the public identity;
- payloads containing only opaque identifiers and non-secret correlation
  metadata;
- immutable tenant verification and a tenant-scoped transaction in the
  worker;
- bounded concurrency, retry, timeout, cancellation and graceful shutdown;
- an existing non-Apalis execution path as rollback.

Apalis task IDs, status values and types must remain in a leaf adapter. Apalis
must never own tenant identity, business state, Cedar decisions or VedaFlow.

## Telemetry boundary

Logs, spans and metrics must not contain prompts, messages, Knowledge bodies,
credentials or unbounded tenant/principal labels. Caller-supplied trace and
baggage headers are stripped at the edge.

Gateway and worker traces use OTLP to a private Collector with memory limiting
and batching. Discard mode terminates traces at a no-op exporter. External mode
uses public-PKI TLS to one validated OTLP/gRPC DNS authority with bounded
in-memory queueing and retry. It has no private-CA, authentication-header or
mTLS contract, and readiness does not prove remote receipt.
The optional observability profile uses a closed Collector configuration to
scrape only the private gateway and worker metrics endpoints, then exposes one
private fan-in to a digest-pinned Prometheus with 72-hour/1-GB TSDB
block-retention thresholds and a loopback-only UI. Those thresholds are not a
disk quota. It carries no tenant-safe Operations or production monitoring claim.
The external exporter carries traces only; metrics remain on the private local
path. Telemetry still may not contain prompts, messages, Knowledge bodies,
credentials or unbounded tenant/principal labels.

## Backup boundary

Bundled-provider recovery uses writer-paused PostgreSQL 17 custom archives for
the Synveda and Keycloak databases. Synveda uses the bounded cluster-owner
recovery identity because forced RLS prevents an ordinary runtime role from
producing a complete archive; Keycloak uses its dedicated database owner.
Gateway, worker and Keycloak writers are paused, but independently connected
database writers are not fenced and the two dumps are not one cross-database
transaction.

The KMS key/reference and surviving Keycloak convergence password are copied
to a separate, canonically non-overlapping mode-0700 recovery root and SHA-256
linked to the database manifest. The archives and recovery secrets are
sensitive and are not encrypted by this tool. The hashes detect accidental alteration, not
malicious replacement; they are neither signatures nor an authenticated
manifest.

Restore requires an exact source/backup/fresh-target confirmation and exact
source tenant and PostgreSQL image identity before Docker mutation. It uses a
fresh private volume/network, installs the recovered secrets into that target,
reconverges the database authorities, then verifies the tenant and complete
audit chain through the ordinary gateway database role before normal tenant
convergence. It opens the current tenant data key, requires the exact
cryptographic refusal under a synthetic wrong key, and only then starts the
normal graph and completes browser login. Opening that key does not mean
Knowledge bodies are application-encrypted.

A backup on the same host is recovery-validation evidence, not disaster
recovery. Logical `pg_dump`/`pg_restore` does not provide WAL/PITR, encrypted
off-host retention, S3 transfer, scheduling, RPO/RTO or recurring restore
drills; those remain OPS-5 production work. Keycloak realm export is not a
database backup.

## Adversarial evidence inventory

make check-context-security pins these boundaries:

| Boundary | Primary evidence |
| --- | --- |
| forced-RLS completeness | crates/synveda-store/tests/rls.rs |
| cross-tenant IDs and principal-scope privacy | crates/synveda-gateway/tests/foundation_audit.rs |
| invitation replay and token secrecy | crates/synveda-gateway/tests/access_api.rs |
| Session actor/scope spoofing | crates/synveda-gateway/tests/sessions_api.rs |
| Capture source-event forgery | crates/synveda-gateway/tests/capture_api.rs |
| Knowledge disclosure and erasure | crates/synveda-gateway/tests/knowledge_lifecycle.rs |
| context side channels and graph paths | crates/synveda-gateway/tests/context_runs.rs |
| Skill path safety and inert validation | crates/synveda-types/src/skill.rs and crates/synveda-gateway/tests/skills.rs |
| MCP read-only testing and secret lifecycle | crates/synveda-gateway/src/tool_registry.rs and crates/synveda-gateway/tests/tools.rs |
| OKF traversal and expansion bounds | crates/synveda-okf/tests/okf_v02.rs |
| audit content minimisation | crates/synveda-gateway/tests/audit_query.rs |
| directory credential failure | crates/synveda-gateway/tests/directory_sync.rs |
| VedaFlow personal auto-apply | crates/synveda-gateway/tests/relaxations.rs |
| UI capability and denied-content handling | console/src/review.test.tsx and console/src/context.test.tsx |
| adapter tamper hold and deployment binding | adapters/claude-code/src/hook.test.mts and log.test.mts |

The gate also scans console/adapters for storage coupling, diagnostics for raw
exception propagation, Skill/Tool/OKF handlers for execution seams and MCP
testing for methods outside the permitted discovery set. Mutation tests prove
each scan class detects a violation.

## Residual and external limits

- A PostgreSQL superuser, compromised application process or compromised host
  is outside the isolation promised by RLS and Cedar.
- Audit is tamper-evident, not WORM. SIEM streaming, external transparency
  anchoring and customer-managed HSM keys are not implemented.
- The gateway metrics route shares the private application listener. Helm may
  still expose it through transitional ingress configuration and must close
  that gap before promotion.
- The contributor stack still publishes Jaeger/OTLP development ports; it is
  not the canonical reference topology.
- Custom CA and explicit outbound-proxy support are not yet implemented.
- A local adapter cannot observe a turn before its host hook runs and cannot
  authenticate state against a hostile process in the same local account.
- MCP local stdio execution belongs to the trusted client; the gateway is not a
  universal execution proxy.
- OKF remote import is bounded and rejects private targets/redirects; it is not
  a general synchroniser.
- Live Entra/Okta and real Cursor evidence remain unavailable. Client claims
  are governed by adapters/registry.json and generated docs/CLIENT_SUPPORT.md.

## Repeat the audit

    make check-context-security
    npm test --prefix adapters/claude-code
    cargo test -p synveda-cli session::tests
    cargo test -p synveda-okf --test okf_v02
    make ci
    make db-test
    make eval-product
    make eval-security

make ci also checks RLS coverage, dependency direction, generated OpenAPI and
console clients, audit-action completeness, migration/schema-epoch rules,
demo drift, licences and deterministic product gates. Database-backed and live
client evidence remains labelled separately; deterministic replay is never
reported as live.

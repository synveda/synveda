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
packaging gaps remain: there is no joint database/key restore, signed
multi-architecture release-image parity, or clean-volume cross-platform
acceptance. Static deployment convergence is valuable but does not prove that
a user can install, sign in, use, back up, restore or upgrade the product.

An executable inventory found no Temporal SDK, application consumer,
migration, release service, Helm service, demo or CI job. Its two legacy
services, configuration and self-referential smoke probe were therefore
deleted independently. The contributor lifecycle removes the retired project
containers as orphans without deleting volumes or old peer databases. That
removal does not satisfy the still-open Rauthy to Keycloak identity cutover.

[ADR-0102](../adr/adr-0102-portable-reference-deployment.md) fixes the target
architecture. [ADR-0103](../adr/adr-0103-cooperative-live-provider-reservation.md)
locks the implemented terminal no-spawn reservation. Accepted
[ADR-0104](../adr/adr-0104-indivisible-live-provider-effect-generation.md)
defines the sibling generation; its fixture-only pre-attempt,
conclusive-not-created attempted branch and explicitly acknowledged
missing-delivery terminal are implemented, while every provider-process and
live-identity branch remains absent. [The deployment
contract](../DEPLOYMENT_CONTRACT.md) fixes the provider-neutral commands,
configuration and external dependency seams.

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
- Keep the unused Temporal runtime residue deleted; remove Rauthy only after
  the Keycloak identity replacement passes live acceptance.
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
writable-primary state and initial policy convergence pass, binds
health/readiness/metrics to loopback, and treats an unexpected critical task
exit as fatal. A supervised authority sentinel
continues to re-prove epoch and runtime role; a conclusive refusal faults the
process, cancels every loop and exits non-zero rather than merely changing
readiness.

The public `synveda init` entrypoint is now an unconditional cutover refusal;
its private legacy lifecycle cannot discover a profile, start Compose, read or
write secrets, or contact a database. This hard withdrawal is necessary because
the Rauthy-era host/container URL split, raw `.env` credential handoff and
unbounded whole lifecycle cannot satisfy the locked reference contract. A
static mutant test pins the gate-only public boundary. Reopening it requires a
wrapper around the accepted deployment-owned lifecycle, not a legacy escape
hatch.

Canonical Compose and Helm use distinct migrator, gateway and worker
credentials. The bootstrap refuses reused owner/runtime credentials before
mutation; bundled shared-cluster mode extends that content-free comparison to
the Keycloak database credential. Their preflight binds all three sessions to one exact cluster,
database OID, authority contract and writable primary, while a peer-cluster
witness prevents a copied authority file from turning a second cluster into the
same trust domain. Gateway and worker each maintain a fail-closed runtime
authority gate. Helm still supplies issuer/KMS values through Secret-backed
environment variables rather than the file-mount contract, and the retained
transitional manifests still carry legacy environment handoff; neither is
reference acceptance.

The shared product database-URL boundary accepts only `postgres`/`postgresql`,
requires an explicit database path or effective `dbname`, and rejects fragments
or query keys not consumed by pinned SQLx before SQLx can log an ignored value.
Content-free unit, reset and real gateway/worker process sentinels prove wrong
schemes, ambient database fallback and unknown query secrets are refused
without disclosure.

The additive canonical Compose checkpoint now has a closed selector for all
eight development/reference and bundled/external PostgreSQL/OIDC rows. Static
evidence proves role-scoped mode-0600 file inputs, provider-specific service
sets, internal trust networks, explicit egress seams, one product image across
gateway/worker/migration, reverse-proxy-only host ports and no Rauthy/Temporal
entry in the new graph. `synveda db migrate`, reset and other executable direct
store commands resolve `DATABASE_URL` or bounded `DATABASE_URL_FILE` with
ambiguity and content-free failure tests; `init` is closed before resolution.
Development selection now closes the local build graph for the product,
proxy, PostgreSQL and optimized Keycloak images. A content-free issuer helper
atomically generates the exact project-scoped static-tenant contract, while
`synveda tenant converge` reuses migrator authority, tenant forced-RLS and the
normal `tenant.created` audit event to admit only one exact active UUIDv7,
requiring current-key unwrap custody before success. Its repairable
generation-one audit witness uses a key-provision-specific API, the stored KEK
reference and the serialized tenant chain head; arbitrary mutations cannot use
that exception to escape same-transaction audit. Explicit development HTTP now
uses names distinct from the HTTPS `__Host-` cookies and retains host-only,
HttpOnly, SameSite, lifetime, duplicate-rejection and origin protections.
The canonical wrapper now invokes those seams through bounded `up`, `smoke`,
gateway-only `restart-gateway`, `down` and exact-confirmation `reset` actions.
One private exact-project lock
spans authority-file generation and Docker mutation; complete network/IPAM and
retained-asset proofs are repeated around startup; catchable signals propagate
to a bounded process-group runner; and uncertain stale locks fail closed. The
gateway stays in its container. Deterministic lifecycle tests prove
concurrency refusal, re-entrant signal cleanup, forced termination, asset
substitution refusal and atomic issuer replacement, but this remains source
evidence rather than a clean browser lifecycle.
External-PostgreSQL rows remain configuration-only and the bootstrap now
refuses before mounted-input reads or SQL until an authenticated-TLS transport
exists; an ordinary pre-provisioned CREATEROLE/CREATEDB principal is covered by
a live no-mutation sentinel. Mounted database and Keycloak inputs are copied by
the same bounded, non-following descriptor helper before parsing, including
writerless-FIFO and symlink refusal tests.
Pinned Keycloak, Caddy, PostgreSQL and Collector configuration, exact
database-role/authority convergence, idempotent realm convergence and a
product-owned exact issuer diagnostic now exist. Reference certificate-file
mode now refuses unsafe, oversized or malformed PEM, key mismatch, unordered
or duplicate chains, missing DNS SAN coverage and certificates that cannot
remain valid through the bounded lifecycle before Compose rendering or startup
mutation. The Node 22-or-newer preflight accepts leaf-first leaf-and-intermediate
fullchains, refuses an included self-signed trust root and commits no test keys;
expiry never blocks `down` or `reset`. Trust anchors, revocation,
served-endpoint proof, renewal and ACME remain open. Every lifecycle Node
helper now starts through an explicit bundled-CA wrapper. Reference evidence
actions refuse ambient Node/OpenSSL trust or proxy activation before their
first process and project lock, while development and recovery scrub those
controls. The runtime smoke independently refuses non-HTTPS reference URLs.
Every canonical service now also defines all ten upper/lower Docker proxy
variables as exactly empty, every development build defines the matching empty
build arguments, and rendered plus post-create asset checks refuse missing,
non-empty, malformed or duplicate runtime entries without disclosure. The
post-create check requires the complete container/network/volume graph before
smoke and on both sides of gateway restart; deterministic contract failure
remains recoverable while uncertain inspection retains the project lock. This
is deterministic host/client-proxy closure, not public/browser trust, live
synthetic Docker-config evidence or an explicit custom-CA/proxy contract. The
development source-build path now also refuses recognised ambient
BuildKit/Buildx/Bake selectors before helpers or locking, requires the pinned
local Engine's exact `default` context, uses fresh private Buildx state and an
explicit default builder, and separates the build from all no-build startup and
gateway-recovery commands. It preserves registry authentication, never opens or
parses credential content, and never rewrites or prints either authentication
environment value, while resolving path metadata to refuse an effective config
directory or temporary root physically inside the source context. Installed
Docker plugins, credential helpers and daemon policy remain operator-trusted,
and the canary remote-builder/private registry case is not yet live evidence.
The explicit development resolver prerequisite now has a reversible ownership
ceremony rather than an instruction to edit `/etc/hosts` manually. One
repository root command hardcodes the target, binds confirmation to the exact
project and selected aliases, refuses unmanaged/foreign/drifted state, and
preserves the original bytes in a root-only adjacent recovery record. It uses
same-inode append/truncate mutation so xattrs, security labels and flags
survive; the supported host file/physical parent are ACL-free and the target is
root-owned, single-link mode `0644`. A killed append is recoverable only when it
is an exact expected-prefix state. Ordinary preflight proves the
raw-content-free ownership record and its world-readable-target integrity
digest before its first Docker endpoint query, then repeats the check with real
resolver validation. The elevation uses a fixed root-owned, non-writable,
ACL-free Node runtime/path and empty environment but still trusts the clean,
reviewed operator-writable checkout. Reference mode owns no host-file state,
external OIDC owns only the application alias, and `down`/confirmed `reset`
stay usable after resolver removal. Scratch-file acceptance covers idempotency,
interruption on sidecar publication and strict-prefix mutation, stale/active
cooperative locks, collision and drift, inode/xattr/POSIX metadata, ACL/mode
refusal, file-type/size constraints and a concurrent edit before mutation. The
host mapping remains uninstalled, so this is deterministic ceremony evidence,
not browser or cross-platform resolver acceptance.
The current pinned Keycloak image includes a review-locked complete 26.7.2
user-profile contract: upstream built-ins remain intact, unmanaged attributes
remain disabled, and the two demo ownership attributes are admin-only with
closed validators. Realm repair performs a full no-merge profile replacement
while closed, proves the exact readback before marker use and refuses marker
provenance after prior profile drift. A source-locked arm64 image set was built
from the complete 1,169-file input closure at source HEAD
`48704d8878d62036e53645f39d9c70549fe18b09`; the complete-input manifest hash
was `ce3b5ff12b4d77437bf23f95a4af1cb65cae4409d2777fa1f98c15475016be81`
and the five-image ledger hash was
`26e0b846bd466d4fd54f0292b1dbef1489ff0a650903757defe896ff973e87b6`.
On Docker Desktop the frozen v13h development graph then passed its guarded lifecycle in dependency
order: PostgreSQL; separate Synveda and Keycloak database bootstrap; repeated
Synveda bootstrap; preflight; initial and repeated migration; optimized
production-mode Keycloak; initial and repeated realm convergence; proxy;
initial and repeated issuer diagnostic; private Collector; separate worker;
and gateway. Every long-running service was healthy with zero restarts and
every one-shot exited zero on its expected image.

That private, uncommitted bundle is bounded exploratory evidence, not durable
deployment acceptance or proof of the current worktree. No committed
environment manifest or evidence artifact reproduces the run, and subsequent
identity, directory, proxy and harness hardening changed its source closure.

Content-safe public probes through the sole loopback proxy also passed exact
gateway liveness/readiness, security headers, OIDC discovery, issuer and
endpoint equality, PKCE S256, authorization-code and RS256 metadata predicates.
The required host resolver mapping had not been installed, so neither `.test`
authority resolved from the host and this is not browser-login acceptance. The
current source narrows the public identity matcher to the exact discovery,
authorization, token, JWKS, logout, login-action, account and static-resource
paths, but that revision post-dates the frozen candidate. The legacy
Rauthy lifecycle remains non-authoritative cutover residue until a later
candidate passes real browser login, callback/token/audience/group/admin
admission and the deletion gate. Temporal's unused legacy runtime has been
deleted after a no-consumer inventory.

Fresh database-authority evidence previously proved normalized pairwise
credential refusal, idempotent Synveda/Keycloak convergence, the fixed
owner/migrator/gateway/worker and separate Keycloak role topology, migration,
forced RLS, gateway/worker terminal authority drift, pre-open read-only
refusal, post-open Keycloak quarantine, crash-resumable closure and exact
cleanup. Deterministic evidence also covers worker authority, shutdown and
configuration boundaries. It does not yet cover real claimed-work SIGTERM,
multi-worker execution, browser login, Linux lifecycle, reference HTTPS,
external OIDC, backup/restore, upgrade or the Apalis canary.

Current source hardening requires explicit advertised PKCE S256, a mandatory
closed API audience disjoint from the login client, service-tainted credential
classification and active registered-service admission. Only a strict
`email_verified: true` claim can participate in directory adoption. Directory
correspondence workflows take a global tenant fence before their sorted
principal-grant fences; SCIM create/projection is atomic, stale PATCH/DELETE
snapshots are fenced, and ambiguous active user-email matches fail closed.
The identity suite passes 91 unit, nine connector and one proxy test. The
durable one-time administrator marker adds an eleventh uncached store query and
advances the epoch-3 baseline to revision 2. A fresh isolated preparation now
owns 657 validated SQLx records: ten additions and three stale removals relative
to the starting tree. The ACL, routine, trigger and forced-RLS authority
fingerprints were regenerated from the same revision and the complete fresh
dual-cluster `make db-test` gate passes. Database acceptance includes
deterministic PostgreSQL blocker-graph evidence for two distinct claimants,
exactly one committed grant, no deadlock, and rollback handing the claim to the
waiting transaction; sequential coverage retains the marker after grant
revocation and denies later provider-group escalation.

The first isolated SQLx-prepare candidate stopped before preparation because
macOS exposed its per-user temporary directory through the `/var` compatibility
symlink, which the private-path policy correctly refused. That failed fixture
is retained and must not be reused or inspected. The generic DB harness now
canonicalizes an existing temporary root with `pwd -P`, keeps every generated
secret/authority/gate leaf under its unique fixture, and has a source-only
post-generation sentinel regression. Later source-compilation and access/OIDC
fixture failures were likewise retained without reuse. The final fresh
database gate passed all ordinary workspace tests, every serial
administrator/drift suite, epoch/reset acceptance and exact success cleanup.
The collision-resistant database-network allocator has independently reviewed
source and fake-engine concurrency evidence. The latest fresh exact-role
database gate passed the complete live matrix and self-cleaned after exercising
restart readiness, OIDC 16/16, Capture 7/7 plus its deliberate serial case and
both directory-sync binaries 10/10. The prior Compose, issuer and deployment
convergence suites passed at their recorded checkpoint; the current slice adds
deterministic contract/lifecycle/profile mutants without converting them into
live deployment evidence. A separate fresh
deterministic authentic-frame Claude lifecycle passed 1/1 and self-cleaned,
and the complete post-repair `make ci` gate passes with the generated API and
657-record SQLx cache current.

The current deterministic browser-preparation slice introduces an explicit
`absent` asset state that is valid for a suffixed development acceptance
project or the exact reference browser-acceptance selection, and refuses every
exact-name container, network and volume before the first build or pull-only
start. All fourteen deployment image stages now execute one
closed assertion before their first RUN and refuse a non-empty upper/lower
HTTP, HTTPS, NO, FTP or ALL proxy build argument. Recursive image inventory now
interprets Docker global build arguments and stage aliases in declaration
order, refuses noncanonical Compose image/build keys and Dockerfile parser
directives, rejects every external deployment base without a readable tag and
full SHA-256 digest, and covers all deployment Dockerfiles and Compose build
callers, including the fixture-only pinned
Playwright/Chromium 1.62.1 base and its Apache-2.0 package, licence and reviewed
default-deny sandbox profile.

The exact `demo,browser-acceptance` overlay is bundled-only, with separate
development-HTTP/source-build and reference-HTTPS/digest-image selections. It
requires exact Docker-asset absence and adds one private non-root one-shot on
`app-backend`. Host-side `--if-missing` inputs are separately reviewed rather
than claimed fresh. Its driver validates one exact authorization-code/PKCE S256
request, Keycloak's exact issuer/session-state callback, administrator
admission and logout while refusing foreign or non-flow identity paths. It
captures no screenshots, content, HAR, trace, video or storage state and emits
no credentials, codes, tokens or cookies. The wrapper waits for the exact
container to exit zero before the ordinary runtime smoke. Deterministic
Compose-model, lifecycle, seccomp, secret-descriptor and injected-browser tests
pass. The reference path is deterministically rendered and fake-lifecycle
exercised only; no real Docker command, TLS endpoint, browser container or IdP
exchange ran, and mounted-secret ownership and platform trust remain open. The
documented development resolver handoff binds the single helper-owned
hosts block to the suffixed acceptance project and carries the exact suffix,
pool and profiles through down, confirmed reset and mapping removal. No Docker
command has run in this slice, so neither a live exchange nor
the mounted secret's effective uid/mode is deployment evidence.

Release/Helm parity preparation now rejects an unsafe or noncanonical release
version before it can enter workflow shell source, an archive path, a download
path or a substitution. The release workflow packages the existing chart and
has one closed native amd64/arm64 plan for exactly five first-party images:
product, single-host PostgreSQL, CloudNativePG-compatible PostgreSQL, optimized
bundled Keycloak and the capability-stripped reference proxy. Helm resolves the
product and CloudNativePG defaults from the same application version. The
Playwright image remains a fixture and the Collector an exact upstream input.
Daemon-free gates prove the five workflow image/Dockerfile/tag tuples, require
tag-plus-full-digest external Dockerfile bases, package the chart twice and
render the Helm image pair. No image was built and no tag, registry, OCI
descriptor, environment manifest, authenticated pull or live Helm install was
produced, so the release-parity P0 remains open.

The deterministic clean-Engine preparation publishes one immutable
canonical candidate and plan receipt under a user-owned external state root.
The candidate binds the clean commit/tree, stage-zero tracked index, actual
effective Docker context, deployment inputs, deployment-contract digest, exact
development/bundled selection, private `/24`, hosts, profiles and pinned
Distribution 3.1.1 fixture image. Context hashing covers types, modes, file
bytes/sizes, symlink targets and derived directories; included untracked/empty
entries and ignored inputs outside the closed exclusions fail closed. It
contains no host path, machine identity, Docker configuration, credential
derivative or raw command output. A private synthetic proxy template contains
only `.invalid` markers. The complete run is fsynced, renamed and published as
a no-replace hard-linked active receipt using exact device/inode identities.
Uncatchable pre-publication residue remains inert and must be removed by final
cleanup. Independently, every development build now requires byte-bounded,
content-free `buildx inspect` evidence for exactly one running embedded
`docker` builder at endpoint `default` before source mutation. Planning
contacts neither Docker nor Colima and does not install the resolver mapping,
so it closes no live acceptance criterion.

The deterministic executor boundary now validates an append-only version-6
receipt grammar from provider intent through registry, proxy, zero-read
builder, browser, project cleanup and provider cleanup. Every phase has a
closed content-free result schema; failures enter only an exact receipt-owned
cleanup branch, and a colliding foreign resource is excluded from that
authority across cleanup retries. Receipt and synthetic-manifest publication
use fsynced staging plus no-replace links. The fixture-only manifest schema is
explicitly non-live and controlled background evidence cannot become eligible.
Provider success carries an explicit evidence class and must bind the exact
provider-contract digest from its intent; receipt v1 through v5 state is a
fresh-plan hard cut. Mutation slot v1 through v6, recovery/root v1 through v5
and close v1 through v7
are also refused and require regeneration; there is no translator. Mutation
slot v7 binds the exact source receipt and
environment, previous close, operation kind, contract and canonical operation
plan. Recovery v6 binds that operation and its exact observed evidence/root
frontier; close v8 binds the result, owner or newest recovery authority and the
outer operation-settlement digest. The recovery root is v6. Permanent,
gap-free slots, settlements, closes and per-slot recovery claims are never
deleted or reused. Only unique staging aliases reconcile. Every final link
re-proves authority, endpoints, operation evidence and staged inode/bytes; a
cooperatively displaced alias retries within a fixed bound only after the same
checks. Generic append cannot own preflight, provider-create, provider-cleanup
or finalization evidence.

The provider seam retains two fixed fake-only paths. The synchronous
`deterministic-fake-provider-v1` adapter is the rollback. The former
state-integrated actor protocol and its fixtures are deleted. The controlled
path is now the separately supervised background fake: a canonical operation
plan binds the private provider base, evidence directory, exact root key and
ownership nonce before slot acquisition; the v4 slot and v4 intent become
durable before root mutation. The state owner alone publishes the inner create
authority and launches the repository-fixed controller/host-agent chain. No
caller function, command, environment or provider selector enters the adapter.

At create-authority publication and every inner root/controller/start/identity
checkpoint, the outer gate reopens the actual slot, operation plan, receipt
head and source closure before allowing the next effect.
Root/config/readiness/PID evidence uses fsynced
private stages and no-replace links. HMAC-bound child identities make negative
PID/PGID probes admissible only for the exact recorded process; elapsed time
never proves absence. The terminal identity binds the complete static root
identity, including device, inode, mode, path and UID, after both private
sockets are reauthenticated.

An immutable outer `background-create-settlement.v1` records either the exact
complete identity or a settleable exact residual. Passing receipt and close
bind this outer digest, never the inner provider-identity digest. A foreign
pre-owner root is recorded as `resource-collision`, preserved without reading
its leaves and becomes historical once settled; later foreign removal or
replacement cannot invalidate the state journal. Synveda-owned evidence stays
exact. Source drift is rechecked at intent, every effect gate, pass and close;
a stale staged intent is retired without effect, while drift after a complete
identity enters the closed execution-failure branch.

Recovery confirmation is read-only. After proving the recorded owner and any
newest recoverer absent, acquisition alone may reconcile exact mutation-stage
aliases, then appends a v6 claim bound to a fresh observation. Recovery never
launches, signals, deletes, repairs the inner chain or replays a durable
controller/start decision. Unauthenticated controller-launch and start-without-
PID windows remain permanently uncertain. A complete identity, an exact
proved-absent residual or the closed foreign collision can settle; a changed or
live/unidentifiable observation remains blocking. At most eight claims and 64
slots keep inspection bounded.

The same inner protocol remains directly exercisable only as a deterministic
process canary for the background-provider model. Its v5 contract records only
repository-owned Node controller/host-agent ancestry, short private Colima,
Lima, cache, Docker and temporary fixture roots, a closed helper path and the
closed environment-name set. The fixture inherits ambient `HOME` unchanged and
does not persist that environment value. Its toolchain evidence necessarily
records absolute runtime and fixture-source paths, which may reside beneath the
home directory. This contract defines no live-provider preparation, host
eligibility or start authority.

A separate, non-executing live-provider preparation contract is authoritative
for the next input boundary. Requirements schema
`synveda.clean-engine.colima-live-requirements.v5` pins the official
Colima 0.10.3 Darwin/arm64 release, Lima 2.2.0 Darwin/arm64 archive and extracted
wrapper/`limactl` bytes, release guest agent at
`share/lima/lima-guestagent.Linux-aarch64.gz`, release default template at
`share/lima/templates/default.yaml`, and the Colima-core 0.10.4 arm64 Docker
disk image by exact size and digest. The private Lima network baseline is mode
0600 with the user-v2 gateway `192.168.5.2`. The command pins
`--activate=false` and `--port-forwarder grpc`, with
`LIMA_SSH_PORT_FORWARDER=false` as the derived child setting. Its current
canonical digest is
`157bd8b6eaef32ffb57e733bc66420038594f5bb093ede40dcda0cae4770d6a6`.
It retains the exact v3 digest
`409bfc2fa03c57d151812c69c395d75c4cf7454f1262d2369f47c97646ebf265`
only as `legacy_preparation_contract_sha256`. The private observation v5 schema
binds exact staged paths, bytes, modes,
parents, the dynamically selected Docker CLI, SSH helpers, state-owner closure
and macOS probes, a closed environment, exact host build/boot inputs and
distinct source/receipt-owned disk inodes. `/bin/sh` and `/usr/sbin/ioreg` are
declared exact-OS-build trusted-boundary inputs. They are not yet individually
observed live.

Six receipt-owned mutation namespaces cover Colima cache, Colima home, Docker
config, Lima home, `HOME` and temporary files. `HOME` is exactly the private
`h` namespace; Lima home contains only the staged `_config` baseline and the
other five start empty. The canonical physical provider root is at most 21
UTF-8 bytes: the root, closed two-byte `/l` segment and pinned longest 80-byte
Lima Unix-socket suffix total 103 bytes, below Darwin's 104-byte refusal
threshold. Lexically overlong input is refused before filesystem access and the
resolved physical path is checked again before traversal; a short alias cannot
bypass the bound or canonical-path requirement. Pre-effect root observation v5
no-follow recursively samples the exact six-namespace baseline twice. Lima
home has exactly two baseline descendants, `_config` and
`_config/networks.yaml`; the other five roots have none. The traversal admits
at most 64 descendants in aggregate, depth 32 and 64 entries per directory,
with bounded names, relative identities, link targets, hashed bytes and elapsed
time. Its public result exposes only each baseline-set HMAC, sorted keyed opaque
relative identities and descriptor-digest/identity pairs plus
`observed-pristine` or `foreign-collision`; it exposes no raw name, path,
target or content. Unexpected descendants are opaque and are never adopted.
The source disk is a preselected local file; the observer downloads nothing
and imports no process-execution API. Its host record is preparation input, not
a live probe or admission fact. Public projection v5 contains no raw private
provider, component or `HOME` path and no fixture or profile identity; the
declared OS executable paths remain non-private OS-build-bound metadata. V1
through v4 requirements, observations, root observations and public projections
plus falsely relabelled two-target evidence are refused; only v5 is generated.

All three capability bits remain false: preparation cannot authorize execution,
lifecycle exposure or finalization. The module is not registered with mutation
state, receipts, cleanup, a lifecycle target or the environment finalizer. Its
fixture-only seams permit small deterministic files instead of committing the
official disk image; any future state adapter must accept only the production
requirements digest. Twenty-six deterministic preparation tests cover
pin/schema/generation drift, canonical UTF-8 root-length boundaries, lexical
and physical overflow, alias refusal, whole-namespace collisions, bounded
inventory, role/path/environment/HOME/host/file/disk replacement, private
projection, revalidation and unconditional authorization refusal. The shared
fixture creates an exact 21-byte canonical provider root beneath the physical
temporary directory on both Darwin and Linux; this is fixture evidence, not a
production allocator. No Docker, Colima or Lima process was
invoked. The existing before-root source-drift fixture now
waits for the durable create-authority marker rather than racing the intent
receipt's post-link source read; drift is still injected while the provider
root is absent and is refused by the next exact authority gate.

A new pure provider-adapter registry reserves
`colima-vz-docker-live-create-v1` and
`colima-vz-docker-live-cleanup-v1` plus distinct live evidence schema names. The
create operation contract digest is
`bbf65bbb13c58e31e15b040576253b32c1f07a62445d585c8436cc821d845fe4`;
cleanup is `3fd567d325cb0a6575109f03ddbb8eb0fff7331cd32f5a559c360ce7a4e8182b`
and binds the create digest. Registry digest
`4eda07f8bf439f631e1541717e5345fc41ec374c43580c406a129115c8366052`
accepts only an exact action, operation kind, operation-contract digest and
provider-class tuple. It compares all fields after the content-addressed key,
cannot select either fake class and returns only closed metadata. Create grants
state planning only through `mutation-journal-v7-plan-only`; execution,
provider recovery, lifecycle exposure and finalization remain false, while
cleanup remains wholly deny-only. The registry imports no process,
mutation-state, receipt or fake-provider module; state reaches it only through
the non-executing live-plan boundary. Fifteen deterministic tests cover tuple
confusion, contract and registry drift, capability escalation, fake relabelling,
immutability and content-free refusal.

The new `synveda.clean-engine.colima-live-provider-operation-plan.v1` binds the
active run/candidate/head, exact registry resolution, production requirements,
private preparation-observation digest and provider profile/resource. It
persists no private paths, command, environment, `HOME`, binding key or
credentials. The state owner embeds it in a `provider-plan` mutation slot v7
and owner close v8 after revalidating the production observation before slot
acquisition and again at the close link. Planning and both fake executors
therefore compete on the same atomic slot name. Completion changes no receipt
or environment, writes no provider evidence and blocks later mutation and
finalization except the exact inert intent successor. An abandoned slot can
only be explicitly recovered by an
`aborted-before-effect` journal close before retry; this grants no provider
recovery. Six plan-boundary and five focused state plan/race/crash tests cover
the new boundary. The supported lifecycle still exposes only `plan`, `status`
and `verify`.

The standalone fixture launcher creates a separately detached fake host agent
only after its pre-effect gates. Its recoverable fixture-only create authority
binds the canonical base/evidence directory identities, intended
receipt/slot-shaped inputs and ownership nonce before root creation. An
immutable controller-launch decision precedes the controller, and an immutable
start decision precedes the host agent. The controller reopens and validates
the exact authority, root owner, launch/witness chain, start decision and
host-agent configuration before spawning. Rewritten gate inputs and controller
death before the request leave no host-agent PID or socket. Distinct
authenticated host-agent and Engine sockets plus a bound Docker-context
endpoint then remain. Provider identity binds the exact creation-time root
and complete content-free inventory before retirement can obtain deletion
authority. After the controller group reports `ESRCH`, fresh
challenges prove the host agent and Engine still belong to the recorded
process instance. Retirement publishes a complete immutable inventory and
leaf-first action plan before any deletion, stops the host agent only through
its authenticated protocol, and uses individual `unlink`/`rmdir` actions after
revalidating the exact remaining subset. Append-only progress repairs an exact
delete-before-progress crash. Unknown leaves, aliases, symlinks, inode changes,
transplanted evidence and a recreated retired root are preserved and refused.
Provider-create and provider-cleanup evidence remain separate immutable heads.
Before recovered deletion progress, the exact parent is fsynced and absence is
rechecked, including the terminal provider root.
Planning after host-agent death or graceful expiry retains every exact
creation-bound non-socket and permits only the two recorded sockets to be stale
or absent.

This remains fixed-fake trusted-process POSIX evidence. The state-integrated
adapter accepts no caller function, command, environment, path or provider
selection; the standalone process canary accepts only validated private
fixture/evidence roots and bounded test controls. Legacy retirement v1 and
operation-evidence APIs remain fixture-only and
`state_integration: not-authorized`. Retirement v2 accepts only the exact
`mutation-journal-v2` creation chain. Its immutable plan, per-step progress and
inner settlement remain a separate head from creation evidence and grant no
result-receipt or close authority.

The controlled-background contract v5 binds only repository-owned Node
controller/host-agent fixture ancestry. Its module exports no live-named
preparation contract, host validator or start authorizer, so this two-role fake
cannot be selected as the later four-role live process-effect implementation.

The mutation-state owner now composes that seam through a dedicated controlled-
background cleanup operation plan, slot and intent. They bind the completed
create slot, outer create settlement and close, immutable provider identity,
source head, provider base and evidence-directory identities. Each synchronous
retirement checkpoint is reconstructed from journal state before authenticated
stop, stale-socket or leaf-first unlink/rmdir, stage mutation, publication and
final consumption. Exact effect authorization also binds whether absence was
recovered. Partial publication is repairable only when its declared digest and
inode bind the exactly reconstructable bytes; changed parent identities,
resources, source or inert state staging refuse.

After complete retirement, the owner publishes a distinct outer cleanup
settlement. Receipt v6 accepts only that digest as operation evidence for the
controlled cleanup pass, and close v8 binds the same digest; the inner
retirement settlement, provider identity, create settlement and unrelated
digests are rejected. Owner close is valid only before any recovery claim.
Action-dispatched cleanup recovery may abort an untouched pre-intent slot
without effect. Otherwise it holds the newest observation claim, reserves one
of the eight bounded claim positions for the final observation when required,
and appends that exact settled-prefix snapshot before publishing the outer
settlement. Recovery settlement and close must name the latest claim. Settled
observations cannot regress or change, and completion is reasserted again at
the final close link.

Concurrent private mutation publication cannot invalidate an otherwise valid
publisher through a stale directory read. The loader validates one captured,
sorted run inventory and accepts it only when the directory generation and
mutation-stage identities remain exact. Private-stage creation or retirement
and an append-only recognised final-name candidate discard the whole snapshot;
the next stable scan must prove its topology. Up to 16 supersessions are retried
before one final scan, and continued churn fails content-free with status 73.
Reconciliation similarly reloads after exact stage disappearance or same-inode
publication-link progress, and fsyncs before every zero-stage return. Same-name
identity replacement, unrecognised entry changes, final-name removal, capture
failure and every stable invalid topology remain hard failures. A pipe-barrier
regression fixes the former winner-69/loser-73 interleaving at winner 0/loser 73
with one cleanup slot, settlement and close.

Only tests and test fixture modules pass these observer hooks; no supported
lifecycle target exposes them. The authority-gated helpers are unsupported
internal composition hooks, not a JavaScript security boundary; owner-UID code
execution and journal mutation are one trusted-host boundary. Process probes do
not prove another PID namespace, and ACLs, xattrs, flags and bind mounts remain
trusted-host limits.
Controlled-background evidence is structurally ineligible for the synthetic
environment finalizer, so finalization remains blocked. No Docker, Colima,
resolver, registry, database, live-provider or host-management command ran in
this slice; only bounded repository-owned fake processes and sockets were used.

The live plan, intent and process-start decision boundary is now durably inert.
The state owner projects only content-free slot, close, plan and observation
digests. Intent candidates remain `provider-create`/
`requested-not-authorized` with a zero-entry logical prefix. Production intent
operation kind `colima-live-provider-intent-publication-v1` now has contract
digest `24c8d98eb13962fd7139f72f034b7b4f85b1474eb9b681b8db57f2881bdaed9d`;
the separate fixture digest is
`bf7681827912b6a848525145ffc3a8eebafa1239e68cf6c4ec6184b6045e8d6d`.
They authorize only `mutation-journal-v7-inert-intent-only` publication.

The successor `provider-start-decision` uses production operation kind
`colima-live-provider-start-decision-publication-v1` with contract digest
`358eb1f8be6d3ee632df8d262239c1ab09c9dcd544fb62607d3ed2d242a63e4c`;
the distinct fixture digest is
`8ae81955e5d4ce75b5d1c9f9fe72d0de5ae6d2827e6bc3f746e2c3513f0f6c73`.
Its publication plan is schema v2 and authorizes only
`mutation-journal-v7-inert-start-decision-only`. Both contracts bind the exact
completed intent and allow only `requested-not-executed-not-authorized` from an
`observed-pristine` namespace observation. They remain absent from the
provider-adapter registry.

The publisher reconstructs state/source and fresh namespace admission at four
boundaries: initial, immediately before slot link, after slot acquisition and
immediately before close link. Every canonical completed-intent projection,
intent completion/publication plan, root observation, admission and decision
publication plan must equal the initial pristine-namespace value. Slot v7 and
owner close v8 have result sequence zero and zero receipt, environment and
operation-evidence delta. They create no process, provider, registry, runtime,
receipt, environment or operation-settlement artifact. A pre-CAS collision leaves no
slot; a post-CAS collision owner-aborts that generation.

The journal grammar is ordered and class-closed: aborted plan generations before
one completed owner plan, aborted intent generations before one matching-class
completed owner intent, then aborted decision generations before one
matching-class completed owner decision.
Abandoned intent and decision slots accept only an all-zero recovery v6 claim
and `aborted-before-effect` recovery close; generic effect recovery refuses
them. Exact completed retry checks the preparation-observation and fixture-
requirements digests and returns historical non-authorizing state without
reading or claiming current namespace state.

Mutation slot v7, close v8, recovery v6 and recovery-root v6 are an indivisible
full-state hard cut. Slot v1-v6, close v1-v7 and recovery/root v1-v5 are refused
with reset-and-regenerate guidance; no migration or relabelling exists. Crash,
collision, source drift, competing writer, recovery, class-crossing, schema and
validly rehashed semantic-tamper tests cover the boundary.

The post-decision read-only boundary projects the exact completed decision,
including its slot, close, publication-plan, candidate and predecessor digests,
and re-observes state and namespaces in `S1/O1/S2/O2` order. Stable pristine
namespaces yield a process-start-effect candidate that explicitly denies every
process, adapter, root, publication, recovery, lifecycle and finalization
capability; stable collision yields `null`. Production and fixture schemas
remain distinct. The admission is not persisted and adds no journal action,
contract digest, registry row, receipt or provider artifact. Its explicit
authorization function always refuses.

ADR-0103 now locks and the state owner implements one cooperative aggregate
no-spawn reservation. Production operation kind
`colima-live-provider-reservation-publication-v1` has contract digest
`1dd6bd0e68e2b5991c7dd5f98230ec951ab3fa479746c2c695357f4c8b728ec6`;
the fixture-only operation kind
`colima-live-fixture-provider-reservation-publication-v1` has digest
`abb0b37f748540272149140d615e7569873bb204ffcfab14f08da38235542b6b`.
Both use `mutation-journal-v7-no-spawn-reservation-only` and remain absent from
the provider-adapter registry and supported lifecycle.

After the completed matching-class decision, aborted reservation generations
may precede at most one completed reservation, which must be the last slot. The
owner publishes the complete v7 slot before filesystem mutation, writes and
fsyncs one private mode-0600 immutable witness stage, and revalidates the bound
state and roots. Its plan binds one provider root, all six fixed namespace
identities and inventories, and one state run. All eight directory identities
must be distinct, current-UID, private and on one device. A hard link from the
stage to fixed basename `.synveda-clean-engine-provider-reservation` is the
single no-replace CAS. Exact bytes, device, inode, UID, mode, size and link count
are checked for the stage, marker and durable state witness.

While the exact marker/witness inode remains held with two links, the owner
brackets a fresh six-namespace observation with topology checks and requires it
to equal the plan. It then publishes the immutable
`retirement-authorized-without-process` settlement, retires only the verified
marker, fsyncs the provider root, proves the witness has one link, and closes v8
with the settlement digest. Receipt and environment do not advance, and no
provider, registry, runtime or evidence artifact appears.

Dedicated v6 recovery records bounded topology and settlement evidence. It can
retire an inert stage and abort, finish a partial witness publication, or relink
an exact standalone witness to the fixed marker before completing observation,
settlement and retirement. Claim history must be reachable, bind the
deterministic witness and match current local topology. Wrong type/mode/inode,
foreign replacement, extra hard links, backward topology, malformed or orphan
settlement, and namespace drift fail closed without adoption or cleanup. A
two-state-base race admits one CAS winner and one durable pre-effect abort;
owner/recovery interruption tests cover every owner publication boundary plus
the stage-retirement and multi-transition recovery gaps.

This is cooperative same-UID exclusion, not protection against hostile same-UID
code. `reservation_recovery_authorized` is true only for the exact marker/witness
grammar. Process start/spawn/signal/group ownership, adapter/effect execution,
general root mutation, provider-effect recovery, provider/evidence/runtime/
receipt/environment publication, cleanup, lifecycle and finalization remain
false. No Docker, Colima or Lima process ran; only repository-owned deterministic
fixtures and filesystem state were exercised. The logical supervisor label is
not a PID, PGID, ownership or liveness fact.

### Current effect-state slice and exact next action

[ADR-0104](../adr/adr-0104-indivisible-live-provider-effect-generation.md) is
accepted and its first persisted-state slice is implemented. Production
operation kind `colima-live-provider-effect-v1` remains
`production-deny-only-no-invoker`, is pinned to
`e57ab31606d0cf6e33a0fd45cc86335a6ca1288d9beb28839aeb45225f24df63`
and has no production publisher, recovery entry point, registry row or
lifecycle command. The separate fixture-only
`colima-live-fixture-provider-effect-v2` contract is pinned to
`fffed74545de0af992fbcdcf38f0ea7d203864755c1f63c24251a537e66b4b60`;
its recovery and explicit missing-delivery-resolution capabilities cannot
cross into the production tuple.

The earlier non-persisted prerequisite projection remains a separate inert
review artifact. Its production/fixture digests are
`81c496e201dbb75b17470f09d7cad2114d266641ca11cd8fbff25a992584b8da`
and `a38253aff84b502c683e64413e62b13307f50b05a5a420e3b175bb6c3733a57e`.
It defines no action, operation kind or contract, retains its independent
512-occurrence preflight, and is consumed by neither state nor the pure effect
grammar. It is not a compatibility or execution path.

The pure fixture grammar binds the exact completed decision, trusted v5
recursive baseline, witness/marker, causal graph, endpoints, inventories,
cleanup, terminal receipt and retirement without observation I/O. The state
owner persists both the fixture-only pre-attempt branch and one bounded
attempted branch using receipt v6, mutation slot v7, recovery/root v6 and close
v8. A sequence-bound private stage derives its witness from the physical inode
before the fixed marker CAS. It may retire before attempt with no receipt, stop
at an attempt fence without delivery, or consume the exact process-free
conclusive adapter result described below. Partial stages, foreign markers and
interrupted marker or receipt publication fail closed without adopting or
deleting foreign evidence.

An iterative closed-data preflight runs before validation or freezing. It
rejects proxies without invoking traps, cycles, depth above 16, more than 1,024
value occurrences, more than 64 entries in one container, and canonical UTF-8
above 64 KiB including the newline. Shared acyclic aliases are permitted and
counted per occurrence. Refusals are typed and content-free.

The generic receipt API still rejects `provider-effect-retired` before slot
acquisition. Only the exact current fixture owner or newest recovery authority
publishes that reserved phase; the pre-attempt completion publishes no
receipt. All effect mutation/recovery APIs are fixture-only, ADR-0103's
completed no-spawn branch remains terminal, and caller-held pure bytes carry no
state provenance or branch selection.

The persisted fixture plan now comes from the closed process-free blueprint v3
rather than label-derived synthetic commitments. Its fourth component is the
`conclusive-adapter`, whose v2 contract digest is
`b476c4f4c9258943fff3745abfc622e822684f95fa0b72d1a311a9cc86bed681`.
The blueprint fixes the state-owner-to-outer four-role topology, four planned
endpoint kinds and bounded runtime envelope, then projects exact invocation,
role, endpoint, adapter, start-attempt and quiescence commitments into the
existing effect plan. State opens Node, protocol, role and adapter bytes with
`O_NOFOLLOW`, compares descriptor identity before and after hashing, and
re-proves the exact source/component/blueprint binding at the authority,
attempt, event, marker-retirement and close boundaries. Role-key, challenge and
private-path commitments are domain-separated HMACs under the unpersisted
observation binding key. Public Ed25519 SPKI material plus content-free hashes
and opaque commitments enter durable state; no private key, raw path,
environment or adapter-private value is persisted.

The adapter is a pure state-owned fixture seam. For one closed input it returns
only `conclusive-not-created`, a zero child-handle identity,
`effect_possible: false` and safe error code `not-created`. It has no
filesystem, network, process, provider or runtime-publication capability. The
attempted owner path publishes exactly `start-authority`, `start-attempt`,
`launch-edge`, `delivery-result`, `create-settlement`, `cleanup-plan-page`,
`cleanup-plan`, `cleanup-progress`, `cleanup-settlement`, `terminal-receipt`
and `completion`. The terminal event embeds the receipt; the global receipt is
published byte-identically and durably before the exact marker is unlinked and
its bound provider-root descriptor fsynced. Completion and mutation close are
last.

Once the attempt fence exists, generic recovery is blocked until an exact
durable conclusive delivery event exists. Recovery never calls the adapter
executor; state exact-byte loads the module and recovery validates the durable
result. A separate test-only acknowledgement accepts only the exact two-event
authority/attempt prefix or that prefix plus the outer launch edge. It binds a
local same-UID confirmation to the complete content-free source, plan, slot,
witness, event, predecessor-recovery and pending-stage statement. The resolver
retires no pending stage, or only the exact confirmed inert stage or alias;
then it publishes a new recovery claim and an
`uncertain-start` terminal with `effect_possible: true`, and leaves the lease,
receipt head and two-link marker/witness unchanged. It publishes no synthetic
delivery, cleanup, receipt, completion or close. A pre-terminal crash that
changes the frontier requires a new confirmation; after terminal linkage the
embedded statement makes normal retry and crash/restart idempotent. This does
not authenticate a human, provide non-repudiation, create an atomic physical
snapshot or detect hostile same-UID rollback. After conclusive delivery the
ordinary recovery path revalidates the adapter result,
source/component/blueprint binding, opaque proposed-outer-node identity
commitment, provider root, all six namespace identities and the accepted event
prefix, then resumes only the missing suffix under the newest recovery claim.

The exact durable vocabulary is disposition
`acknowledge-indeterminate-effect-possible`, event variant
`operator-acknowledged-missing-delivery`, reason
`delivery-record-not-durable`, and confirmation provenance
`local-same-uid-explicit-confirmation-v1`.
Receipt-first, marker-first, embedded/global receipt mismatch,
malformed/crossed evidence and physical drift fail closed. Pending one- or
two-link receipt publication is reconciled only from canonical bytes matching
the embedded receipt and the observed staged inode. Repeated same-frontier
recoverers are retained. Existing durable prefix events and staged receipt
bytes retain their original publisher and identity; among recoverers only the
newest recovery authority may publish new suffix state, reconcile the receipt,
retire the marker or close.

This remains deterministic preparation, conclusive-not-created retirement and
acknowledged-indeterminate terminal evidence only. It does not launch or adopt
a provider process, prove that a
planned endpoint exists, retain a launch capability or make the standalone
four-process fixture an effect executor. That fixture deliberately retains its
truthful `fixture-supervisor` ancestry and imports no deployment module. No
persisted receipt/journal generation advances: receipt v6, slot v7,
recovery/root v6 and close v8 remain unchanged. The process-free blueprint
advances to v3; the fixture operation contract, fixture `uncertain-start` and
adapter contract/result advance to v2. The production operation contract and
digest remain v1 and unchanged.

A standalone deterministic preparation fixture validates
the closed four-role causal shape with four repository-owned Node processes:
pre-spawn fsynced edges, IPC-only role keys, authenticated private endpoints,
recursive quiescence, one signed recursive deadline, detach/reparent
observation and bounded cooperative shutdown. Its tests also bind an
authenticated partial-start frontier and refuse wrong keys, replay, wrong
roles, stale identities, wrong fences and a post-fence spawn request. The
fixture is not imported by deployment code and reports `state_integration`,
`provider_effect_evidence`, `live_provider_evidence` and
`lifecycle_authority` as false. It does not provide OS start/boot/PGID/session
identity, complete arbitrary-descendant observation,
the six-namespace inventory, crash recovery or any Colima/Lima/SSH/Engine
evidence. The recorded execution is Darwin-only; Linux execution remains an
open evidence requirement.

Next add authenticated live-role, endpoint, quiescence and recursive inventory
evidence without changing this process-free adapter into an executor. Then
cover live cleanup retry before considering a production invoker. No
production registration or supported lifecycle may precede independently
reviewed platform evidence.

Keep the production effect contract deny-only, absent from the adapter registry
and lifecycle-unexposed. Never reuse the generic mutation-slot inode,
ADR-0103's retired marker inode or its immutable no-spawn witness, and do not
branch through, relabel or import the class-closed controlled-background v5
fake. Finalization remains blocked until a separately reviewed live
environment-manifest schema exists. A later real invocation must use the closed
helper `PATH`, explicit provider roots, `--mount none`, the receipt-owned disk
copy and receipt-private `HOME`; it requires separate environment/platform
evidence and independent review before any supported lifecycle target exists.
Subsequent bounded steps add the globally clean Engine proof, TLS/bcrypt
private registry,
authenticated push/pull, synthetic Docker-client proxy activation, a zero-read
canary remote builder, browser run and exact cleanup that make the existing
environment finalizer eligible.
Do not use plaintext Basic auth, host-wide daemon trust changes or any prune.
Then request explicit
administrator approval for the exact `.test` block, install it from the clean
reviewed checkout with the fixed-runtime helper, flush the active resolver cache
and run the committed browser fixture against that separate clean Docker
endpoint. Prove the repository reaches only the pinned local default builder,
registry authentication still works, private Buildx state is removed, every
created container retains the ten exact empty proxy entries, and the real
browser completes authorization-code + PKCE without recording credentials,
codes, tokens, cookies, HAR, trace, video or screenshots.
Repeat the resolver/lifecycle contract on Linux, then exercise reference HTTPS.
Only after replacement acceptance may the Rauthy callers and assets be
deleted. Backup/isolated joint database-and-key restore, upgrade and the Apalis
canary remain subsequent slices.

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
- Unsupported Temporal runtime residue remains at zero. Rauthy tracked residue
  reaches zero only after live Keycloak cutover acceptance.
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
- Keep the focused Temporal runtime regression under `make check-deploy`.
  After Keycloak cutover, add `make check-runtime-residue` for zero active
  Rauthy runtime/config/support references, with historical ADRs and narrow
  negative fixtures as the only allowlists.
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

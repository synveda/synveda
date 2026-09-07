# ADR-0102: Portable reference deployment contract

- **Status**: Accepted; amended by ADR-0104
- **Date**: 2026-08-27
- **Feature(s)**: CPR-45
- **Deciders**: Synveda maintainers

## Amendment (2026-09-06): process effects are a sibling generation

ADR-0104 supersedes this ADR's later statements that an effect generation
would keep ADR-0103's reservation held. ADR-0103's no-spawn reservation is a
terminal sibling whose retired marker and immutable witness prove that no
process belonged to that generation. A process-capable generation instead
branches directly from the exact completed start decision, publishes a
distinct effect witness at the same fixed marker basename and holds only that
new marker/witness inode across its start-attempt and settlement protocol.
Neither sibling may extend, translate, relabel or reuse the other.

The current generic mutation-recovery claim is v6; the earlier v4 reference
below records an older intermediate state generation. ADR-0104's fixture-only
state generation and recovery are implemented, while production effect
publication and recovery remain absent. This amendment grants no production
process, registry, lifecycle, finalization or readiness authority.

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

The clean-Engine provider seam has two fake-only execution paths. The original
synchronous closed-data adapter remains the rollback. The superseded detached
actor path is removed. The controlled path is the lifecycle-unexposed
background process canary, wrapped in state-born authority without making its
inner process model the operation authority.

Receipt schema v6, mutation slot v7, mutation recovery/root v6 and mutation
close v8 are fresh-plan hard cuts; receipt v1-v5, slot v1-v6, recovery/root
v1-v5 and close v1-v7 are refused rather than translated. A canonical background-create
operation plan binds the private
provider base, evidence directory, root key and ownership nonce. The slot binds
that plan, operation kind and v4 process contract before intent or root
mutation. The inner create authority then binds the slot and intent. Passing
receipt and close use the immutable outer background-create settlement digest
as operation evidence, never the inner provider-identity digest.

The v4 process contract places six synchronous veto-only checkpoints: before
create-authority publication, root publication, controller spawn,
start-decision publication, start delivery and terminal identity publication.
At state integration, each checkpoint
reopens the actual slot, receipt/source head, operation plan and complete causal
evidence/root frontier. Private root/config/readiness/PID records use fsynced
stages and no-replace links; sockets begin under a restrictive umask. Complete
controller-readiness and host-agent PID records HMAC their causal
configuration, launch/start, process and toolchain identity before any
negative PID/PGID probe. Elapsed lifetime never proves absence. Terminal
identity is prevalidated only after both provider sockets are freshly
reauthenticated and it binds the static root identity, including device,
inode, mode, path and UID.

The outer settlement records only a complete identity or an exact settleable
residual. A root encountered before a valid owner marker is a foreign
`resource-collision`: its leaves are not inspected or adopted. Once the exact
collision and current Synveda evidence are durably settled, later foreign-root
removal or replacement is irrelevant historical state; Synveda-owned evidence
remains exact. Source closure is required at intent, pass and close. A stale
staged intent is retired before effect; drift after complete identity enters
the closed execution-failure branch.

This original decision recorded generic recovery v4; the current hard cut uses
v6. Recovery confirmation is read-only. Recovery
acquisition first proves the recorded owner and newest recoverer absent, then
may reconcile only exact mutation-stage aliases and append a v6 claim bound to
the fresh observation.
It never launches, signals, deletes, repairs the inner chain or replays a
durable controller/start decision. Controller launch without authenticated
readiness and start without authenticated PID remain permanently uncertain;
live or unidentifiable process state also blocks settlement. Permanent slots,
settlements, closes and bounded recovery claims are never deleted or reused.

Legacy inner retirement v1 remains fixture-only. It binds the complete
creation inventory, stops only through authenticated IPC and applies
individual leaf-first unlink/rmdir actions after exact revalidation; recovered
absence fsyncs the parent. Retirement v2 accepts only the exact state-born
create chain. The mutation owner composes it through a dedicated cleanup plan,
slot and intent bound to the completed create slot, outer create settlement and
close, provider identity, source head and parent-directory identities. The
create head remains immutable while exact state gates cover every process or
resource effect and durable publication frontier. Lower progress and
settlement remain a separate head and expose an operation-bound read-only
prefix observation.

The inner settlement grants no result-receipt or close authority. Only the
distinct outer cleanup settlement may bind receipt v6's controlled pass and
close v8. Action-dispatched recovery holds the newest observation claim and,
when retirement advances, publishes a reserved final settled-prefix claim
before outer settlement. Owner close permits no claims; recovery close must
name the latest claim. Pre-intent recovery is effect-free, settled history
cannot regress, and completed retirement is reasserted at the final close
publication. The authority-gated helpers are unsupported internal composition
hooks, not a JavaScript security boundary. Owner-UID code and journal writes
share one trusted-host boundary; ACLs, xattrs, flags, bind mounts and other PID
namespaces remain outside this deterministic evidence. No supported lifecycle
or finalization authority follows from this fixed-fake contract.

The controlled-background contract v5 binds only its repository-owned Node
controller/host-agent fixture ancestry, controlled environment and private
root layout. Its module exports no `COLIMA_LIVE_*` contract, host validator or
live-start authorizer, so the two-role fake cannot be substituted for a live
provider boundary. A separate
`synveda.clean-engine.colima-live-requirements.v5` record now pins
the official Darwin/arm64 release bytes, selected extracted Lima runtime files
and Colima-core 0.10.4 arm64 Docker disk image. The staged closure uses Lima's
release guest-agent and default-template locations, a mode-0600 user-v2 network
baseline with gateway `192.168.5.2`, `--activate=false` and the gRPC port
forwarder. Its private observation v5 closes staged and dynamic helper
identities, a toolchain-only environment, exact host build/boot inputs, command
expansion and distinct source/receipt-owned disk files. Six receipt-owned
mutation namespaces isolate Colima cache/home, Docker config, Lima home,
`HOME`, and temporary files. `HOME` is the empty private `h` namespace; Lima
home has the exact `_config` and `_config/networks.yaml` baseline descendants.
Each exact baseline is recursively sampled twice without following links under
a 64-descendant aggregate bound. Public evidence contains only keyed opaque
relative identities, descriptor digests and a set HMAC; raw names, paths,
targets and content remain private.
The canonical physical provider root is limited to 21 UTF-8 bytes. Together
with the closed two-byte `/l` segment and pinned longest 80-byte Lima socket
suffix, this stays at 103 bytes and below Darwin's 104-byte refusal threshold.
The lexical input is bounded before filesystem access and the resolved physical
path is bounded again before traversal; a symlink alias remains noncanonical.
`/bin/sh` and `/usr/sbin/ioreg` remain explicit exact-OS-build trusted inputs,
not live per-file observations. It performs no process execution, and its host
data is preparation input rather than a live probe. V1 through v4 requirements,
observations, root observations and public projections plus falsely relabelled
two-target evidence are refused; only v5 is generated. Execution,
lifecycle exposure and
finalization remain false, and no receipt, lifecycle or finalizer path accepts
it. Neither contract is Docker, Colima or supported lifecycle evidence.

A separate closed provider-adapter registry reserves fresh live create and
cleanup operation/evidence identities without selecting or importing the
controlled fake. Its lookup key is the exact action, operation kind,
operation-contract digest and
`colima-vz-docker-live` provider class. Both contracts bind the production
requirements digest; cleanup additionally binds the create-contract digest.
The create entry grants only state planning through
`mutation-journal-v7-plan-only`; execution, provider recovery, lifecycle
exposure and finalization remain false, cleanup remains wholly deny-only, and
the registry imports no process, state, receipt or fake-provider implementation.

The state owner embeds the exact live plan in a dedicated `provider-plan`
mutation slot v7. The plan binds the active run/candidate/head, registry tuple,
production requirements, private observation digest and provider
profile/resource, but persists no paths, command, environment, `HOME`, binding
key or credentials. The production observation is revalidated before slot
acquisition and at the owner-close v8 publication boundary. This action shares
the same slot CAS as fake-provider mutation, changes no receipt or environment,
produces no provider evidence and blocks all later mutation/finalization except
the exact inert intent successor while execution remains disabled. An
abandoned slot may only be explicitly closed
`aborted-before-effect`; this is journal repair, not provider recovery. The
supported lifecycle remains `plan|status|verify` and imports no live executor or
test fixture.

The next contract boundary is durable but remains inert. State projects the
newest completed owner plan into only its slot, close, embedded-plan and
preparation-observation digests. Separate structural helpers close the
candidate/plan and empty-prefix/candidate digest bindings, but authenticate
neither state nor observation. The candidate records only `provider-create`
and `requested-not-authorized`; its logical pre-effect prefix has zero entries.

The state owner alone may persist those values through the distinct
`provider-intent` successor and
`mutation-journal-v7-inert-intent-only`. Production and fixture operation
kinds, contracts, schemas and evidence classes are distinct; neither tuple is
registered with the provider adapter. Both contracts deny execution, receipt,
provider-effect recovery, cleanup, lifecycle and finalization authority.

Publication derives all admission values internally. It requires the initial
canonical pristine-namespace admission to equal the observations immediately
before the slot link, after slot acquisition and immediately before the close link.
Slot v7 contains only content-free state, and its owner close v8 commits with
zero receipt, environment and operation-evidence delta. Collision or drift
before slot publication leaves no permanent slot; after acquisition it
owner-aborts that generation. An abandoned intent can only append an all-zero
recovery v6 observation and close `aborted-before-effect`. Exact completed
retries return historical non-authorizing state without claiming current
namespace state.

No resource, process, VM, Engine, socket or context authority is inferred. The
logical supervisor label is not a PID/PGID, ownership or liveness fact.
Serialized values lose provenance and cannot be submitted as authority.

The next boundary is a durable but effect-free `provider-start-decision`,
separate for production and fixtures. It binds the exact completed intent slot,
close, publication plan and completed-plan projection, and its only decision is
`requested-not-executed-not-authorized`. Its publication-plan schema is v2 and
its state integration is `mutation-journal-v7-inert-start-decision-only`.
Process start/spawn/signal, adapter/effect execution, root mutation,
provider/evidence/runtime/receipt/environment publication, provider-effect
recovery, cleanup, lifecycle and finalization remain false.

Structural values alone authenticate neither state nor observation provenance.
The state owner reconstructs the predecessor exclusively from the validated
terminal intent slot and owner close, then reruns fresh `S1/O1/S2/O2` admission
at the initial, pre-slot-link, post-slot-acquisition and pre-close-link
boundaries. Completed-intent snapshots, namespace observations, derived
admissions and publication plans must remain byte-equal. Production and fixture
publishers hard-code their class and reject caller-supplied state, source, plan, projection,
root result, historical admission, candidate or authority.

A pre-CAS collision leaves no slot; a post-CAS collision aborts the generation.
Abandoned decisions admit only an all-zero recovery v6 claim and recovery close;
generic effect recovery refuses them. The journal admits only aborted plan,
intent and decision generations before each matching owner completion. Exact retry returns historical non-authorizing
state without re-reading roots. No process authority follows from persistence.

After that completed decision, a separate read-only seam may project its exact
slot, close, publication-plan, candidate and predecessor identities and perform
a current `S1/O1/S2/O2` state/namespace observation. Stable pristine namespaces
derive only a deny-only process-effect candidate with every authority flag
false; stable collision derives `null`. The result is not persisted and
introduces no journal action, operation kind, contract digest, registry entry or receipt. Its
authorization function always refuses.

ADR-0103 adds one persisted successor: a cooperative terminal no-spawn
reservation. Its v7 slot precedes filesystem mutation; a private fsynced witness
binds the exact state run, provider root and six namespace identities on one
device. One fixed provider-root hard-link CAS selects the owner. While the exact
two-link marker/witness inode remains held, bracketed fresh observation must
match the plan before immutable retirement authorization is published. The
verified marker is then retired, its parent fsynced, the one-link witness proved
and close v8 bound to the settlement digest. Dedicated v6 recovery may retire an
inert stage, finish partial witness publication or relink the exact standalone
witness. It grants reservation recovery only; all process/provider-effect,
receipt/environment/provider/runtime/evidence, cleanup, lifecycle and
finalization authorities remain false. The marker coordinates cooperating
same-UID owners and is not a hostile-principal boundary.

This original decision anticipated an indivisible effect cut that would keep
the no-spawn reservation held across start authority and effect. That
continuation model is superseded by the amendment above: ADR-0104 instead owns
a sibling generation with a distinct effect witness. Its production-unreachable
fixture state now persists the physical stage/witness/marker prefix, a
zero-receipt pre-attempt completion, an attempt fence and crash recovery through
receipt v6, slot v7, recovery/root v6 and close v8. It invokes no process and
cannot recover across the attempt fence. Complete recursive settlement and
causal ownership/recovery for the outer process, detached Lima hostagent,
usernet and SSH ControlMaster remain open, as do live macOS proxy observation
and per-file OS executable identity.

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
- The controlled clean-Engine seams remain internal and fake-only. The
  deterministic background create path distinguishes controller, host agent,
  Engine, both sockets and Docker context; its outer settlement is integrated
  with receipt v6 and close v8. Legacy retirement v1 remains fixture-only; the
  state owner composes retirement v2 through a dedicated cleanup slot, exact
  lower checkpoints, an outer settlement, action-dispatched recovery, receipt
  and close. Only the outer cleanup settlement is operation evidence. A live
  preparation contract now closes exact Docker/helper/disk/host input identities
  without executing them. A tuple-closed registry binds its production
  requirements digest through fresh create/cleanup operation schemas; create
  grants plan-state publication only, while all effect capabilities and all
  cleanup capabilities remain false. The state owner can journal the inert
  intent and decision through distinct non-provider contracts, then the
  ADR-0103 no-spawn reservation without gaining execution capability. This
  original consequence expected a later live state adapter to keep that
  reservation held. The amendment above supersedes that expectation: the
  adapter must branch from the exact completed decision into ADR-0104's
  distinct-witness sibling generation while closing causal process/socket/
  Engine/context ownership without weakening the immutable journal. A
  supported runner, dynamic-tree cleanup and source/image environment manifest
  remain required before this can support a Docker or Colima acceptance or
  finalization claim.
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

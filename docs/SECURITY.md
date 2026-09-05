# Context-platform security model

This document records the security boundary of the Phase 5 context platform
and the repeatable adversarial audit introduced by CPR-42. It describes what
the implementation and tests establish; it is not a claim that a single
process is secure after its host, database superuser or signing authority has
been fully compromised.

## Trust boundaries

- The gateway is the only public application service. Console, CLI and adapters
  use its public API. A separate private core worker runs Capture, Knowledge
  indexing, relaxation expiry and optional directory pull through ordinary
  runtime credentials; tightly scoped database reset, migration and
  first-operator bootstrap are documented operator exceptions.
- Tenant-bound tables enable and force PostgreSQL row-level security. The
  request transaction sets its tenant after authentication. Each worker unit
  re-enters an ordinary tenant-scoped transaction. Governed actions preserve
  their Cedar decision and audit boundary; derivative maintenance such as
  Knowledge indexing is constrained by the already-governed aggregate and
  forced RLS rather than inventing a second authorisation path. Tests use policy
  packs rather than bypassing the PDP.
- Local gateway and worker logins, and the Helm worker login, are verified as
  exact inheriting, non-elevated members of only the safe NOLOGIN `synveda_app`
  role, without membership administration or ownership of any database or of
  any schema, relation or routine in the selected Synveda database. The current Helm
  gateway is an explicit exception: it still uses CloudNativePG's
  database-owner application Secret and must be cut over before the reference
  contract passes. The worker re-proves its own runtime session and the schema
  epoch while running; conclusive drift is a fatal supervised task that cancels
  work and exits non-zero. A transient database outage withdraws readiness and
  is retried rather than mislabelled as authority drift.
- Cedar is the authority even when navigation capabilities predict what the UI
  should show. A hidden or enabled control never grants server authority.
- Knowledge, Skill, Tool server/binding, Configuration and relaxation writes
  enter the typed VedaFlow change path. Auto-apply is an outcome of that path,
  not an alternate mutation seam.
- Session events are untrusted client assertions. The gateway validates,
  redacts, binds them to an already-owned session and assigns ordering and the
  authoritative digest before persistence.
- Skill, Tool and OKF inputs are metadata or inert content at the gateway
  boundary. Bundle scripts and local stdio commands are never executed inside
  the gateway. MCP connection tests admit an exact read-only discovery-method
  set.
- Audit is content-minimised and hash-chained. This makes alteration or a
  broken chain detectable; it is not immutable WORM storage and does not make
  a database superuser unable to delete the whole chain.

## Authorisation and disclosure order

The normal read order is ownership/existence resolution within the tenant,
then Cedar authorisation, then content loading or projection. Retrieval
decides before anchors, before graph expansion, after expansion and before
rendering. Denied candidates are not persisted or returned in a trace by id,
title, edge, score, count, reason or rendered-context fingerprint. The only
permitted disclosure is an aggregate statement that policy exclusions
occurred.

The normal write order is authentication, tenant/resource ownership,
precondition validation, Cedar decision, typed VedaFlow change, effect-time
re-authorisation, immutable version/state transition and a hash-chained audit
event in the same transaction where applicable. A stale version, cross-tenant
identifier or made-up identifier cannot be used as an existence oracle.

## CPR-42 findings and fixes

| Finding | Impact | Resolution |
|---|---|---|
| CPR42-01: automatic Claude retry did not verify `payload_hash` | Accidental or local tampering could be delivered by a hook although explicit CLI flush refused it. | Every spool reader now validates the complete shape, unique ids, increasing sequence and payload hashes before any send. |
| CPR42-02: a refused spool looked missing | Malformed, unreadable or future-version bytes could be overwritten by a newly created spool. | Spool reads are tri-state: `missing`, `ready` or `held`; only `missing` permits creation and refused bytes remain untouched. |
| CPR42-03: a spool could cross gateway origins | Switching an authenticated profile could send one deployment's transcript to another. | First authenticated use pins the canonical gateway origin; automatic and CLI delivery hold on mismatch without rebinding. |
| CPR42-04: adapter diagnostics propagated exception messages | Parsers and subprocesses may quote rejected credential/configuration input into a local log. | Diagnostics retain only stable error classes and recursively redact secret-, payload- and transcript-bearing fields. |

The spool's SHA-256 hash is deliberately a corruption check, not an
authentication code. A hostile process with arbitrary write access to the same
local account can alter the payload and recompute the hash. The server remains
responsible for authentication, admission limits, redaction, PDP checks and
the authoritative digest; Synveda does not claim hostile same-account evidence
integrity.

## Adversarial evidence inventory

`make check-context-security` pins the following boundaries to executable
evidence and fails if a refactor silently removes one:

| Boundary | Primary evidence |
|---|---|
| Forced-RLS completeness | `crates/synveda-store/tests/rls.rs` |
| Cross-tenant id oracles and principal-scope privacy | `crates/synveda-gateway/tests/foundation_audit.rs` |
| Invitation replay and token secrecy | `crates/synveda-gateway/tests/access_api.rs` |
| Session actor/scope spoofing and cross-run ids | `crates/synveda-gateway/tests/sessions_api.rs` |
| Capture source-event forgery | `crates/synveda-gateway/tests/capture_api.rs` plus the composite frozen-event foreign key |
| Knowledge source disclosure and governed erasure | `crates/synveda-gateway/tests/knowledge_lifecycle.rs` |
| Context side channels, trace retention and graph-path leakage | `crates/synveda-gateway/tests/context_runs.rs` |
| Skill path safety, inert validation and declared-tool separation | `crates/synveda-types/src/skill.rs` and `crates/synveda-gateway/tests/skills.rs` |
| MCP read-only testing, version quarantine and secret lifecycle | `crates/synveda-gateway/src/tool_registry.rs` and `crates/synveda-gateway/tests/tools.rs` |
| OKF traversal, symlink, expansion, binary and remote-source limits | `crates/synveda-okf/tests/okf_v02.rs` |
| Audit content minimisation | `crates/synveda-gateway/tests/audit_query.rs` |
| Directory credential fail-closed behaviour | `crates/synveda-gateway/tests/directory_sync.rs` |
| VedaFlow-backed personal auto-apply | `crates/synveda-gateway/tests/relaxations.rs` |
| UI capability failure and denied-content rendering | `console/src/review.test.tsx` and `console/src/context.test.tsx` |
| Adapter tamper hold, deployment binding and diagnostics | `adapters/claude-code/src/hook.test.mts` and `log.test.mts` |

The gate additionally scans ordinary console and adapter code for storage
coupling, adapter diagnostics for raw exception propagation, Skill/Tool/OKF
metadata handlers for process-execution seams, and the MCP test implementation
for any method outside `server/discover`, `tools/list`, `resources/list` and
`prompts/list`. Its own mutation tests prove each class is detected.

HTTP request bodies inherit Axum's 2 MiB default body limit unless a route
installs a tighter limit. Domain DTOs and import/archive readers impose tighter
field, entry, artifact, expansion and total-size bounds where their risk is
higher. No production route uses an unbounded raw-body extractor as a bypass.

## Secrets and sensitive content

Tool, provider and per-tenant directory credentials are stable secret
references in ordinary domain records. APIs, generated clients, audit metadata
and console state expose reference status and version metadata, not plaintext.
Rotation changes the secret material behind a stable reference or runs an
explicit re-encryption job; it does not rewrite immutable artifact history. A
revoked or unusable per-tenant directory reference fails closed and cannot fall
back. The current deployment-level directory connector embedded in issuer
configuration remains an explicit exception; CPR-45 replaces its credential
value with a mounted file reference. Helm currently supplies the issuer Secret
to both gateway and worker pending that role-specific file cutover.

Knowledge source payloads and session event bodies require their own narrower
authority. Ordinary timelines and audit rows carry identifiers, hashes,
counts, timestamps and content-free summaries. Forget removes authorised
plaintext, embeddings and owned source payloads, invalidates retrieval and
retains a content-free tombstone and chain evidence.

Model-provider clients used by Capture and embedding use bounded requests,
refuse redirects and map transport, status and parsing failures to closed
diagnostic codes. Their response bodies and configured credentials are not
copied into application errors. The core worker's private readiness surface
proves schema epoch, its exact runtime role, a writable primary target, initial policy
convergence, process lifecycle and the supervisor heartbeat; that heartbeat is
not evidence that every work loop made progress.

Docker CLI proxy defaults can otherwise add credential-bearing proxy URLs to
new container metadata and implicit build arguments. The canonical Compose
graph explicitly empties the upper- and lower-case HTTP, HTTPS, NO, FTP and ALL
proxy names for every runtime service and every development build. Rendered
model checks reject absence or non-empty values, and converged-asset inspection
requires one exact empty entry per name in every created container without
printing a rejected value. This prevents ambient client configuration from
becoming application routing or stored container metadata; explicit custom-CA
and outbound-proxy support remain unimplemented.

Development builds additionally refuse recognised ambient BuildKit, Buildx and
Bake controls before helpers or project locking. They pin the already-proved
local Unix Engine endpoint, require its effective context to be `default`, use
a fresh mode-0700 Buildx state directory, and require bounded inspection to
show exactly one running `default` node using the embedded `docker` driver at
endpoint `default`. Remote/container/Kubernetes drivers, additional nodes,
driver options and daemon file/flag extensions fail content-free. The build
then uses the explicit default builder and startup uses only `--no-build`.
Reference startup and gateway recovery cannot
build. Registry authentication inputs are retained opaquely and are never
parsed or logged by the lifecycle. Only their effective directory location is
resolved; development builds refuse it, or the lifecycle temporary root, when
physically inside the repository so those files cannot join the source
context. When `DOCKER_CONFIG` is unset, the prospective path is derived from a
required accessible `HOME`; a present `config.json` must be regular and not a
symlink. No config content is opened. `DOCKER_CONFIG` is the portable
authentication path;
`DOCKER_AUTH_CONFIG` is merely byte-preserved when a client version does not
support it as a credential store. Docker/Compose/Buildx binaries and plugin
discovery, credential helpers, registry authentication, daemon mirrors, daemon
proxy/CA and embedded BuildKit policy remain within the trusted host boundary.
Clean-Engine preparation now writes an immutable content-free candidate and
receipt outside the checkout. It binds both the tracked index and actual
effective Docker context—portable paths, types, modes, file bytes and symlink
targets—and rejects included untracked/empty entries plus ignored files not
covered by the closed exclusions. Its private proxy template contains only
`.invalid` non-secret markers. Exact BigInt device/inode checks and a
no-replace hard-linked active receipt prevent rounded or partial publication.
Pre-publication crash residue is inert and grants no provider
authority, but the later final cleanup must remove it. Preparation creates no
Engine and carries no Docker auth. The version-4 synthetic receipt grammar
phase-binds collisions, replays retired cleanup authority and admits a final
manifest only after the exact success sequence. Provider success is explicitly
classified and binds its operation kind, plan and intent contract; v1/v2/v3
receipts fail closed. The
fixture finalizer emits an explicitly non-live synthetic schema and rejects
controlled-background-fake evidence. The append-only mutation journal uses
slot v2, recovery/root v2 and close v3. Permanent numbered slots bind exact
source receipt/environment endpoints, prior close, cooperative owner challenge,
operation kind, contract and plan. Permanent outer settlements bind the
observed provider frontier. Closes bind exact result endpoints, owner/recovery
authority and the outer settlement digest; per-slot recovery claims form a
gap-free prefix. Final journal names are never deleted or reused. Slot v1,
recovery/root v1 and close v1/v2 are fresh-plan hard cuts.
Unique fsynced stages and atomic no-replace links prevent partial final files;
only stage aliases reconcile. Every close link follows authority, result,
operation-evidence and staged-inode reproof. A displaced live close stage
retries only after those checks. An unrelated one-link alias
confers no authority and cannot block that close; its final no-replace link
loses. Generic append cannot own preflight, provider-create, provider-cleanup
or finalization evidence.

The synchronous fake remains rollback. The state-born controlled path executes
only the repository-fixed background controller/host-agent fixtures beneath a
short private root. Its canonical plan and mutation slot precede intent and
root mutation. Six synchronous veto-only gates cover create-authority, root,
controller, start-decision, start-delivery and identity publication. Each gate
reopens the slot, operation plan, source head and complete causal frontier.
The adapter accepts no caller function, command, environment, path or provider
selector.

Private root/config/readiness/PID artifacts publish through fsynced no-replace
links and sockets begin under a restrictive umask. HMAC-bound child records
tie PID/PGID probes to their configuration, start/launch, process instance and
toolchain; elapsed time and caller-supplied PIDs cannot prove absence. Terminal
identity follows fresh authenticated probes of both sockets and binds the
static root device/inode/mode/path/UID identity. Same-byte inode replacement is
therefore refused before settlement, receipt or close.

The outer settlement admits only a complete identity or an exact residual with
no live/unattested process. An unowned root is a foreign collision: only the
root itself is observed, leaves and sockets are not inspected, and no deletion
authority is gained. Once that exact collision is durably settled, later
foreign-root changes are historical while Synveda evidence remains exact.
Passing receipt and close bind the outer settlement, never the inner identity.
Source closure is rechecked at intent, all six effect gates, pass and close.

Recovery confirmation is non-mutating. Acquisition first proves the slot owner
and newest recoverer absent, then may reconcile only exact mutation-stage
aliases and append an observation-bound claim. It never launches, signals,
deletes, repairs the inner chain or replays a durable controller/start decision.
Launch without authenticated readiness and start without authenticated PID
remain permanently unattested; live/unidentifiable state refuses. A claim
after durable close makes the journal invalid and cannot reopen that generation;
exhausted claim/slot capacity also fails closed.

Legacy retirement v1 remains fixture-only. It authenticates stop, fsyncs
absence and performs individual leaf-first unlink/rmdir steps only after exact
inventory revalidation. The mutation-state owner composes retirement v2 with a
dedicated cleanup plan, slot and intent bound to the completed create slot,
outer create settlement and close, immutable provider identity, source head
and exact parent-directory identities. A closed state checkpoint is required
before authenticated shutdown delivery, every stale-socket or resource
removal, every publication mutation and final consumption. The checkpoint
reconstructs all fixed fields, resource and stage identities, including the
exact recovered-absence decision. Publication recovery binds declared, actual
and inode identities; the read-only prefix observation binds the expected
operation, residual inventory and process state.

The inner retirement settlement grants no receipt or close authority. Only a
distinct outer cleanup settlement can bind the controlled cleanup pass and
close; provider identity, create settlement, the inner settlement and unrelated
digests are refused. Owner close permits no recovery claims. Action-dispatched
recovery binds the latest claim, reserves capacity for a final settled-prefix
snapshot before destructive work when needed, and cannot rewrite a settled
observation. An untouched pre-intent recovery can only abort without effect.
Source, parent identities, completed retirement and inert-state absence are
reasserted at the final close publication.

The live-provider preparation module is a separate deny-by-construction input
boundary, not an extension of the fake. Its production record pins official
Colima/Lima/disk bytes and closes staged paths, file modes, dynamic helper roles,
the exact environment and host build/boot inputs. It reads regular one-link
files with no-follow opens and bounded streaming hashes, compares descriptor
and named identities before and after each read, requires a distinct private
receipt-owned disk copy, and rejects symlinked or writable private inputs. The
real `HOME` path is retained only as a keyed private binding plus physical
directory identity; raw `HOME` and all tool/provider paths are absent from the
public projection. Host data remains caller-supplied preparation evidence and
cannot attest VZ or authorize a process. The observer imports no child-process
API and every execution, lifecycle and finalization capability is false.

No supported lifecycle target exposes these fixtures and no Docker/Colima
effect is enabled. Controlled-background evidence remains ineligible for the
synthetic finalizer. The `*WithAuthorityGate` exports are internal composition
hooks, not a security boundary against arbitrary owner-UID JavaScript: owner-
UID code execution and journal mutation are one trusted-host boundary. Other
PID namespaces, ACLs, xattrs, file flags and bind mounts remain trusted-host
limits. A truthful live provider identity, TLS registry auth, zero-read builder
and destruction evidence remain pending.

## Residual and external limits

- A PostgreSQL superuser, compromised gateway or worker process, or compromised
  host is outside the isolation promised by RLS and the embedded PDP.
- Audit is tamper-evident, not WORM. SIEM streaming, external transparency
  anchoring and customer-managed HSM keys are not implemented.
- The worker metrics listener is loopback-private, but the current gateway's
  unauthenticated `/metrics` route shares its application listener and remains
  host/ingress-reachable in transitional Compose and Helm. CPR-45 must remove
  that scrape route when application metrics move behind the private Collector.
- Transitional Compose also host-publishes the Jaeger UI and OTLP receivers;
  these development/evaluation endpoints are not private telemetry evidence.
  The canonical reference must remove those host ports and accept telemetry
  only on its private network.
- The local adapter cannot observe a turn when the host dies before any hook,
  and it cannot authenticate state against an attacker controlling the same
  account. Corrupt or cross-gateway spool bytes are held, not repaired
  automatically.
- MCP local stdio execution belongs to a trusted local adapter/client. The
  gateway catalogue discovers, compares, approves and binds metadata but is
  not a universal execution proxy.
- Skill fixture testing is an explicitly named non-executing validation
  harness. Arbitrary bundle scripts never run in the gateway.
- OKF remote import rejects private-address targets and redirects, but it is a
  bounded import adapter rather than a general network fetcher or synchroniser.
- Live Entra/Okta verification and a real Cursor lifecycle remain unavailable;
  captured or contract-only evidence is labelled as such. Claude Code's named
  versions and installed-client evidence are recorded separately in the
  generated support matrix.

## Repeat the audit

```sh
make check-context-security
npm test --prefix adapters/claude-code
cargo test -p synveda-cli session::tests
cargo test -p synveda-okf --test okf_v02
make ci
make db-test
make eval-product
make eval-security
```

`make ci` also checks RLS coverage, dependency direction, generated OpenAPI and
console clients, audit-action completeness, migration/schema epoch rules,
demo drift, licences and the deterministic product gate. Database-backed and
live-client evidence is recorded in current open feature briefs and generated
client-support surfaces; deterministic replay is never relabelled as live.

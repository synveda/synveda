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

# ADPT-4: Python & TS SDKs

## Problem and evidence

The initial slice in `sdks/` now generates 15 operations and 52 schemas from the checked OpenAPI, with bounded HTTPX/Fetch clients, nine tests per language and a shared real-gateway acceptance workflow. Claude hook replay and both SDKs share authenticated Session identity, approved Skill bytes, pending proposals, workspace denial and correlated content-free audit. Public publication, wider operation coverage and a verified runtime/server matrix remain open. The shared Keycloak workflow passes. Framework-specific shims are not a substitute for supported base clients.

## Scope

The authorised initial slice and execution checkpoints are in
[the interoperability plan](../INTEROPERABILITY_PLAN.md), under ADR-0106.
It adds approved Skill retrieval and typed Knowledge proposal submission to
the shared authenticated scenario without closing the wider release scope.

- Generate or mechanically derive typed Python and TypeScript clients from the checked OpenAPI contract, then add a small maintained ergonomic layer.
- Cover OIDC bearer injection/refresh hooks, sessions and ordered events, context runs, Knowledge query, capture, keyset pagination, idempotency keys, typed errors, timeouts, cancellation, and user-agent/version reporting.
- Retry only operations whose public idempotency contract makes replay safe; expose response/audit correlation identifiers without logging content or credentials.
- Ship equivalent cross-session Knowledge examples and contract fixtures for both languages.
- Define supported language/runtime versions, package versioning, contract compatibility, release provenance, and deprecation policy.

## Non-goals

- Embedding Cedar, SQL, Synveda domain storage, model/tool execution, or adapter lifecycle logic in an SDK.
- Direct database access, static API-key invention, or retries for non-idempotent calls.
- Claiming LangGraph, LlamaIndex, Semantic Kernel, or gRPC support from the base clients alone.
- Hand-maintaining DTOs that can be generated from the authoritative contract.

## Architecture seam

Both SDKs are public REST clients over the generated OpenAPI schema and generated operation identifiers. Language-specific convenience types wrap, but do not fork, wire DTOs. Authentication stays pluggable at the HTTP boundary; the gateway remains the only PDP, RLS, VedaFlow, and audit enforcement point.

## Acceptance criteria

- Python and TypeScript complete the same create-session, append-events, run-context, capture/accept, end-session, and cross-session Knowledge scenario.
- Golden fixtures prove byte-equivalent paths, query encoding, pagination, errors, and idempotency semantics for both clients.
- Token refresh, timeout, cancellation, rate-limit, transient failure, and retry-after handling never expose credentials or duplicate governed effects.
- Package builds are reproducible, signed/provenanced, and tied to a checked OpenAPI digest and compatible server range.
- Published documentation states supported runtimes, authentication prerequisites, limitations, and upgrade policy.

## Required tests

- Generated-source drift and API-operation coverage checks.
- Unit tests for auth injection, redaction, pagination, time/date/enum encoding, error mapping, timeout, cancellation, and safe-retry classification.
- Shared mock-server golden suite run by Python and TypeScript.
- Live gateway end-to-end scenario with ordinary OIDC identities, deny/revoke, and idempotent replay.
- Package install/import smoke tests on every supported runtime and platform.

## Rollout and rollback

Publish pre-1.0 prereleases against a pinned server version, run the shared conformance suite, then promote only supported combinations. Rollback yanks or deprecates a broken package version without deleting it, publishes the fixed successor, and leaves the server contract unchanged; server-side feature discovery must let older clients fail clearly.

## Dependencies

The owner must approve Synveda's repository/package licence, PyPI/npm namespaces, signing/provenance custody, supported runtime matrix, release ownership, and compatibility window before public distribution. The clients depend on stable generated OpenAPI and test OIDC credentials. gRPC support, if accepted under ADPT-3, is separate.

**Current checkpoint (2026-09-12)**

`make sdk-check`, full CI and the fresh exact-role database suite pass at
`8f40237`; the explicit shared gateway workflow passed in that implementation
slice. Both SDKs additionally passed all eight tests on their minimum runtimes
in pinned Linux arm64 containers: Node 22.23.2 and Python 3.11.16. These are
runtime tests, not OIDC evidence or a complete platform support matrix.
Fresh Keycloak acceptance passed after the owned hosts handoff. Native Codex
and both SDKs then completed the shared authenticated workflow through the public
proxy on Session `01a096b9-e1c1-7a50-aa32-8c1db0efdba8`. Each retrieved allowed
context and the exact approved Skill, submitted an idempotent pending proposal,
received cross-workspace denial and correlated content-free audit. The gateway
returns its actual response trace ID because the proxy removes incoming trace
context; SDK success/error results prefer it. Nine tests per SDK and all 20
focused fresh exact-role DB tests pass. Capture, explicit task-owner end,
cross-session reuse and the audit chain also passed. The content-free result is
in `adapters/codex/fixtures/keycloak-qualification.json`.

CPR-33/ADPT-4 now corrects the Session audit filter through the existing shared
query. It matches lifecycle snapshots and delivery identities before pagination;
42 focused exact-role DB tests pass, including combined filters, Session/tenant
isolation and unchanged canonical hashes. Canonical Compose up/smoke and live
Keycloak queries returned all 23 saved events, including open/end, across 12
pages with unchanged prefix hashes and continued workspace denial; see
`docs/INTEROPERABILITY_PLAN.md`. Package ownership, licence, release
provenance and broader runtime/server decisions still require resolution before
publication.

The completed CPR-39 run at `8e90358` adds automatic compaction and native
outage/recovery on the same task used by both SDKs, plus live public-API grant
revocation/re-authorisation with one unchanged Python client and bearer.
The fixture `adapters/codex/fixtures/live-qualification.json` records 19 unique
events, 17 Capture candidates, explicit end, reuse and audit verification
through sequence 858. Existing product policy and SDK code needed no change.
Next: build and install the local wheel/npm archives on the declared minimum
runtimes, retaining the current unpublished status while owner decisions remain
open. Do not expand the API slice or add framework adapters to do that check.

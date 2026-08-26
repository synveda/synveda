# ADPT-4: Python & TS SDKs

## Problem and evidence

OpenAPI and the TypeScript console contract are generated from the executable REST catalogue, but `sdks/` contains no tracked Python or TypeScript SDK implementation. External integrations must currently hand-build HTTP, authentication, pagination, retries, and error handling, which invites contract drift and unsafe retry behaviour. Framework-specific shims are not a substitute for supported base clients.

## Scope

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

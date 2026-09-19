# ADPT-4: Python & TS SDKs

## Problem and evidence

The initial slice in `sdks/` now generates 15 operations and 52 schemas from the checked OpenAPI, with bounded HTTPX/Fetch clients, nine tests per language and a shared real-gateway acceptance workflow. Claude hook replay and both SDKs share authenticated Session identity, approved Skill bytes, pending proposals, workspace denial and correlated content-free audit. Installed packages expose verified build metadata, with two measured runtime pairs. Public publication, wider operation coverage and the public runtime/server support policy remain open. The shared Keycloak workflow passes. Framework-specific shims are not a substitute for supported base clients.

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

The package check from `265eb6b` found that a clean `npm pack` included only
`package.json`: importing the installed SDK failed with `ERR_MODULE_NOT_FOUND`.
The standard `prepack` hook now runs the existing compiler and packages only
runtime/type outputs. The Python wheel already worked; its existing Hatchling
backend now joins the hash-locked development dependencies for offline checks.
`make sdk-package-check` builds each archive twice, installs into isolated
consumers, verifies public types/resources and reuses all nine tests per SDK.
It passed on pinned Docker Linux arm64 Node 22.23.2/Python 3.11.16 and macOS
arm64 Node 24.18.0/Python 3.14.6, with identical archive hashes and no skips.
CI now invokes the same check; that remote job and the broader CI/database
suites were not rerun for this packaging-only batch. Exact hashes, corrected
environment failures and reproduction are in `docs/INTEROPERABILITY_PLAN.md`
and `sdks/README.md`.

**Compatibility increment (2026-09-19; starting at `06bcc096f7d56d5728e2522c21265c94fa07836e`)**

The generated contracts already carry the OpenAPI digest, but both HTTP clients
hard-code SDK version `0.1.0` in their identification header. Public imports do
not expose a package/API/contract tuple, and CI tests only Node 22/Python 3.11.
A package version bump could therefore misidentify the installed client.

Under the amended ADR-0106, derive the two package versions and OpenAPI version
through the existing generator, expose the three metadata constants and verify
the header through the installed-package wire suites. Add Node 24/Python 3.14
to the existing CI job while keeping real gateway acceptance on the minimum
pair. Record exact measured runtimes/platforms; no compatibility range or public
support promise is inferred. Reuse the package checks, their hash-locked inputs
and the documented offline Docker path.

Acceptance passed: isolated TypeScript/Python package-version and API-version
mutations each fail the existing generator drift check; unchanged inputs pass.
Installed public metadata equals each manifest and current OpenAPI, and actual
HTTP headers use the SDK version. Nine tests per SDK pass on pinned offline
Docker Linux arm64 Node 22.23.2/Python 3.11.16 and host macOS arm64 Node
24.18.0/Python 3.14.6, with zero skips and identical archive hashes. No API
expansion, protocol negotiation or framework adapter was needed.

CI declares both runtime pairs on Linux amd64, but its remote execution remains
unverified. SDK drift, formatting, dependency direction, adapter conformance,
generated console types, docs, backlog, ADR and CI YAML checks pass. Full CI,
fresh database, live Keycloak and Compose acceptance were
not rerun for this metadata/header change; prior authenticated evidence remains
dated to its original source. No Rust changed, so strict Clippy is inapplicable.
The [SDK guide](../../sdks/README.md#compatibility-and-release-boundary) records
the exact contract and measured matrix, separate from public support policy.

Next: obtain the owner decisions under Dependencies, then add only the selected
release path and support policy. No public package was published. ADPT-4
remains open for those decisions and its wider scope.

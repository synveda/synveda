# ADPT-6: LlamaIndex memory adapter

## Problem and evidence

The repository has no LlamaIndex adapter or verified LlamaIndex support. LlamaIndex APIs and package versions are external contracts, and configuring both a host memory adapter and Synveda's MCP write surface can double-record the same turn unless one component owns writes. ADR-0057 fixes the rule: a framework adapter owns session writes and any co-configured MCP launch is read/tool-only for that run.

## Scope

- Pin one supported LlamaIndex release and implement its documented memory/retriever seam over the Python SDK and public Synveda API.
- Map a host conversation to one Synveda Session; append ordered events with stable client event identifiers, request context/Knowledge through session-scoped APIs, and close/capture on the host lifecycle boundary.
- Make write ownership explicit in configuration and force co-configured Synveda MCP to `--writes host` or its current equivalent.
- Preserve exact Synveda source attribution, session identity, authorization errors, timeouts, cancellation, and retry/idempotency behaviour.
- Ship a runnable example and support evidence pinned to the real framework/client version.

## Non-goals

- Acting as a generic vector-store replacement, embedding Cedar, or bypassing session/capture/VedaFlow governance.
- Executing models or tools, inventing host lifecycle events, or claiming hooks the pinned LlamaIndex version does not expose.
- Direct store access, automatic identity stitching, or simultaneous writes by MCP and the adapter.
- Advertising support from a mock-only run.

## Architecture seam

The adapter is a Python public-API client above ADPT-4. LlamaIndex DTOs map at the package boundary to session events, context requests, and Knowledge results; no framework type enters Synveda core crates. The gateway remains responsible for OIDC identity, PDP, forced RLS, capture, VedaFlow, and audit.

## Acceptance criteria

- A pinned real LlamaIndex example persists one conversation and reuses authorized Knowledge in a later session with exact provenance.
- Running the adapter and Synveda MCP together records every host turn exactly once and still permits configured read/tool operations.
- Duplicate delivery, restart, timeout, and cancellation preserve ordering and create no duplicate session events, candidates, or governed effects.
- Deny, revoke, cross-tenant, and unavailable-gateway cases fail clearly without leaking resource existence or falling back to local ungoverned memory.
- Support documentation names framework/runtime versions, lifecycle gaps, write owner, authentication, and evidence level.

## Required tests

- Mapping and configuration unit tests against the pinned LlamaIndex interface.
- Shared SDK contract tests for auth, errors, pagination, timeout, cancellation, and safe replay.
- Double-write conformance test with adapter and MCP enabled together.
- Database-backed end-to-end example covering session create/events/context/capture/end and cross-session Knowledge reuse.
- Real pinned-package install and lifecycle smoke test; mocks supplement but do not replace it.

## Rollout and rollback

Publish as experimental for one pinned LlamaIndex/runtime pair, require explicit write-owner configuration, and promote only after the real example and conformance evidence pass. Rollback withdraws the affected version from support and disables its write path; existing session evidence and Knowledge remain governed and intact.

## Dependencies

ADPT-4 must provide the supported Python client and package/release policy. The owner must select the LlamaIndex version/runtime, approve its licence and dependency inventory, and provide a representative example. Any verified-client claim additionally follows ADR-0098 and the support-registry evidence rules.

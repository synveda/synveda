# ADPT-7: Semantic Kernel memory connector

## Problem and evidence

The repository has no Semantic Kernel connector or verified support. The intended Python and .NET surfaces must observe one governed Synveda corpus without forking identity, lifecycle, or retrieval semantics. Semantic Kernel's external interfaces evolve independently, and dual connector/MCP writers create the same duplicate-turn risk addressed by ADR-0057.

## Scope

- Pin supported Semantic Kernel Python and .NET releases and implement their documented memory/plugin seams over public Synveda APIs.
- Use one black-box connector contract for session creation, stable ordered event delivery, context/Knowledge query, capture, session end, errors, and idempotency in both languages.
- Resolve both surfaces to the same OIDC subject binding and governed scopes in the acceptance environment; do not infer that binding from email or display name.
- Make the framework connector the session-write owner and configure any co-resident Synveda MCP as read/tool-only.
- Ship equivalent runnable Python and .NET examples and record exact framework/runtime/client evidence.

## Non-goals

- A generic vector database, model/tool executor, direct store client, embedded policy engine, or alternate Knowledge model.
- Heuristic identity stitching, cross-tenant corpus sharing, or silent fallback to ungoverned local memory.
- Inventing lifecycle callbacks unavailable in the pinned framework version.
- Claiming parity or support from generated types or mocks alone.

## Architecture seam

Language packages translate Semantic Kernel contracts into the same versioned REST operations and shared golden wire fixtures. Python builds on ADPT-4; .NET uses a minimal generated/thin public client owned by this feature unless a separately supported base SDK is approved. All identity, PDP, RLS, capture, VedaFlow, and audit decisions remain in the gateway.

## Acceptance criteria

- Pinned real Python and .NET examples each persist a session and retrieve the same authorized Knowledge in a later session for one explicitly bound identity.
- The two surfaces produce equivalent ordered events, context sources, provenance, errors, and idempotent outcomes over the shared corpus.
- Connector plus MCP configuration records each turn once and makes write ownership visible.
- Deny, revoke, cross-tenant, restart, timeout, cancellation, and unavailable-gateway cases fail closed without local-memory fallback.
- Published support states exact framework/runtime versions, lifecycle limitations, authentication, and evidence level.

## Required tests

- Pinned-interface mapping tests for Python and .NET against one shared behavioural fixture set.
- Golden wire-contract tests for paths, DTOs, errors, pagination, time, cancellation, and safe replay.
- Double-write test with connector and MCP co-configured.
- Database-backed, two-language cross-session Knowledge scenario with ordinary OIDC identities and policy revoke.
- Real package install/build and lifecycle smoke tests on every advertised runtime/platform.

## Rollout and rollback

Release Python and .NET pairs as experimental against one server range; do not imply parity until both real examples pass. Promote support per language/version, not for the product name in general. Rollback removes the failing pair from the support matrix and disables writes while preserving governed server data and audit evidence.

## Dependencies

ADPT-4 supplies the Python SDK contract. The owner must approve the .NET client/package scope, NuGet/PyPI namespaces, Synveda licence, signing custody, supported Semantic Kernel/runtime versions, and test identity configuration. Verified-client claims additionally depend on ADR-0098 conformance evidence.

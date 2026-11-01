# ADPT-7: Microsoft Agent Framework context provider

## Problem and evidence

The repository has no Microsoft Agent Framework or Semantic Kernel connector
and no verified support for either. This brief originally targeted Semantic
Kernel. On 2026-11-01 the target moved to its successor:

- Microsoft Agent Framework 1.0 became generally available for .NET and Python
  on 2026-04-03
  ([announcement](https://devblogs.microsoft.com/agent-framework/microsoft-agent-framework-version-1-0/)).
  Its memory seam is the context provider; Microsoft documents providers for
  Mem0 (Python only), Redis, Neo4j, Cosmos DB and Foundry memory
  ([context providers](https://learn.microsoft.com/en-us/agent-framework/integrations/by-component/context-providers/)).
- Microsoft calls Agent Framework the successor to Semantic Kernel and commits
  to critical bug and security fixes for Semantic Kernel for at least a year
  after Agent Framework's general availability, so to at least April 2027
  ([Microsoft](https://devblogs.microsoft.com/agent-framework/semantic-kernel-and-microsoft-agent-framework/)).
  A new Semantic Kernel connector would start near the end of that window.

The intended Python and .NET surfaces must still observe one governed Synveda
corpus without forking identity, lifecycle or retrieval semantics, and a
context provider co-configured with Synveda's MCP server can record the same
turn twice, the hazard ADR-0057 decision 6 addresses.

## Scope

- Pin supported Agent Framework Python and .NET releases and confirm the exact
  context-provider interface, its invocation points and what it can observe
  in each language, before mapping anything.
- Implement a context provider over public Synveda APIs: map one agent thread
  to one Synveda Session, append ordered events with stable client event
  identifiers, supply context from session-scoped context runs, and capture
  and close at the thread's lifecycle boundary.
- Use one black-box contract for session creation, event delivery, context and
  Knowledge query, capture, session end, errors and idempotency in both
  languages.
- Resolve both surfaces to the same OIDC subject binding and governed scopes in
  the acceptance environment; do not infer that binding from email or display
  name.
- Make the provider the session-write owner and configure any co-resident
  Synveda MCP server as read/tool-only.
- Ship equivalent runnable Python and .NET examples and record exact
  framework, runtime and client evidence.

## Non-goals

- A new Semantic Kernel connector. A Semantic Kernel user can use the MCP
  server meanwhile; a Semantic Kernel request from a named customer reopens
  that decision.
- A generic vector database, model or tool executor, direct store client,
  embedded policy engine or alternate Knowledge model.
- Heuristic identity stitching, cross-tenant corpus sharing or silent fallback
  to ungoverned local memory.
- Inventing lifecycle callbacks that the pinned framework version does not
  expose.
- Claiming parity or support from generated types or mocks alone.

## Architecture seam

Language packages translate Agent Framework context-provider contracts into
the same versioned REST operations and shared golden wire fixtures. Python
builds on the Python SDK; .NET uses a minimal generated or thin public client
owned by this feature unless a separately supported base SDK is approved. All
identity, PDP, RLS, Capture, VedaFlow and audit decisions remain within the
Synveda gateway and core-worker application boundary. No framework type enters
a Synveda core crate.

## Acceptance criteria

- Pinned real Python and .NET examples each persist a session and retrieve the
  same authorized Knowledge in a later session for one explicitly bound
  identity.
- The two surfaces produce equivalent ordered events, context sources,
  provenance, errors and idempotent outcomes over the shared corpus.
- Provider plus MCP configuration records each turn once and makes write
  ownership visible.
- Deny, revoke, cross-tenant, restart, timeout, cancellation and
  unavailable-gateway cases fail closed without local-memory fallback.
- Published support states exact framework and runtime versions, lifecycle
  limitations, authentication and evidence level.

## Required tests

- Pinned-interface mapping tests for Python and .NET against one shared
  behavioural fixture set.
- Golden wire-contract tests for paths, DTOs, errors, pagination, time,
  cancellation and safe replay.
- Double-write test with the provider and MCP co-configured.
- Database-backed, two-language cross-session Knowledge scenario with ordinary
  OIDC identities and policy revoke.
- Real package install/build and lifecycle smoke tests on every advertised
  runtime and platform.

## Rollout and rollback

Release Python and .NET packages as experimental against one server range; do
not imply parity until both real examples pass. Promote support per language
and version, not for the product name in general. Rollback removes the failing
pair from the support matrix and disables writes while preserving governed
server data and audit evidence.

## Dependencies

The [Python & TS SDKs](ADPT-4.md) supply the Python client contract. The owner
must approve the .NET client and package scope, NuGet/PyPI namespaces, signing
custody, supported Agent Framework and runtime versions, and test identity
configuration. Verified-client claims also depend on ADR-0098 conformance
evidence. The [agent SDK session-store adapters](ADPT-11.md) share the
thread-to-Session mapping and should reuse one fixture set.

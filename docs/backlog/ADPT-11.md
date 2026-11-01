# ADPT-11: Agent SDK session-store adapters

## Problem and evidence

Agent SDKs now expose session storage as an integration point separate from
memory. The external facts below come from a survey checked on 2026-11-01:

| SDK | Seam | Reference backends |
| --- | --- | --- |
| Claude Agent SDK | `SessionStore` with `append` and `load` ([docs](https://code.claude.com/docs/en/agent-sdk/session-storage)) | S3, Redis, PostgreSQL |
| OpenAI Agents SDK | `Session` protocol, four methods ([docs](https://openai.github.io/openai-agents-python/sessions/)) | Redis, SQLAlchemy, MongoDB, Dapr, Conversations |
| Google ADK | `BaseMemoryService` (`add_memory`, `search_memory` and others) ([docs](https://adk.dev/sessions/memory/)) | Vertex Memory Bank |
| LangGraph | checkpointers and `BaseStore` | several |

Mem0 publishes adapters for LangChain and LangGraph, LlamaIndex, CrewAI,
AutoGen, Vercel, OpenAI Agents, ADK and Mastra
([Mem0](https://docs.mem0.ai/integrations)).

At `8a22b0e5` Synveda has an initial, unpublished Python and TypeScript SDK
slice generated from the public API, open briefs for [LlamaIndex](ADPT-6.md) and
[Microsoft Agent Framework](ADPT-7.md), and none for these four. The seed's
principle that the harness is a guest names LangGraph, and ADR-0057's
consequences already note that a framework shim and the MCP server must not
both write the same turn.

One design question decides the shape. A session store must return what was
appended so the framework can resume. Synveda scans and redacts every Session
event before it persists, under the effective pack's capture mode (`full`,
`redacted`, `hashes_only` or `disabled`) (`crates/synveda-gateway/src/sessions.rs`).
It therefore cannot promise to return exactly what a framework wrote.

## Scope

1. **ADR.** Decide, per SDK, between:
   - **Mirror.** The framework keeps its own session store for resume; the
     adapter wraps it, sends each appended item to a Synveda Session as
     evidence, and supplies context through the SDK's memory or context seam.
   - **Store.** Synveda is the session store, which requires a defined
     fidelity contract for redacted items and a resume path over the public
     API.

   The ADR also chooses the first SDK. The Claude Agent SDK's `append` and
   `load` map most directly onto Sessions; the OpenAI Agents SDK and ADK
   reach different users.
2. Implement the chosen adapter for the first SDK over the published Python or
   TypeScript SDK, with a runnable example.
3. Add the remaining SDKs one at a time, each with its own pinned version and
   evidence.

## Non-goals

- Replacing a framework's checkpointing of execution state (LangGraph
  checkpoints, tool state) with Synveda.
- Returning secret material the redaction policy removed.
- A Java SDK or other new base language.
- Claiming support for an SDK from mocks or generated types.

## Architecture seam

Adapters are thin packages over the published SDKs and public API. Framework
types stop at the package boundary and map to Session events, context runs and
Knowledge queries. The adapter owns Session writes for its run, so a
co-configured Synveda MCP server runs read-only (ADR-0057). No core crate
changes for a mirror; a store choice needs its own ADR for the resume
contract.

## Acceptance criteria

- The ADR records the per-SDK choice and fidelity contract.
- For each supported SDK, a pinned real example records a run once, resumes
  correctly under the chosen contract, and reuses approved Knowledge in a
  later run with exact provenance.
- Adapter and MCP server together record every turn once.
- Deny, revoke, cross-tenant and unavailable-gateway cases fail without
  falling back to ungoverned storage.
- The support matrix names SDK and runtime versions, the chosen mode and the
  evidence level.

## Required tests

- Mapping tests against each pinned SDK interface.
- Idempotent re-delivery, restart and cancellation tests.
- A double-write test with the MCP server co-configured.
- A redaction test showing what resume returns for a secret-bearing item
  under each capture mode.
- A database-backed end-to-end run per SDK.

## Rollout and rollback

Release each adapter as experimental for one pinned SDK version and promote it
after its real example passes. Rollback withdraws that SDK version from the
support matrix; recorded Sessions stay governed.

## Dependencies

The [Python & TS SDKs](ADPT-4.md) must be published first. The owner chooses
the first SDK and the mirror or store decision. The thread-to-Session mapping
and fixtures should be shared with the [LlamaIndex adapter](ADPT-6.md) and the
[Microsoft Agent Framework provider](ADPT-7.md).

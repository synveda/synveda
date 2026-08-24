---
title: "CPR-25: Trusted MCP server catalogue and project bindings"
labels:
  - epic:CPR
  - phase:5
size: XL
---

# CPR-25: Trusted MCP server catalogue and project bindings

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** XL

## Description

Add one governed catalogue for MCP server metadata and project distribution.
The existing `synveda mcp` generic server remains a thin application-API
adapter; it is not the catalogue and confers no authority on imported tools.

## Acceptance criteria

- Stable ToolServer aggregates own immutable ToolServerVersions. Raw and
  deterministically normalised CapabilitySnapshots retain the exact observed
  tools, resources and prompts, including descriptions and JSON schemas.
- The official stable MCP `2026-07-28` specification is pinned to release
  commit `5f5440bb26a62e2cf3440b92da5a667efa03b267`. The implementation models
  stateless `server/discover`, stdio and Streamable HTTP only; no HTTP+SSE or
  protocol-level session implementation is added.
- Explicit server manifests and one server entry from supported client config
  import through bounded validation. Streamable HTTP is metadata; stdio
  capability evidence is admitted only from an authorised trusted local
  adapter, and the gateway never launches an imported command.
- Any source, digest, transport, authentication, requested-permission, tool,
  resource or prompt change mints a quarantined immutable version. Unchanged
  discovery is idempotent. Version comparison identifies additions, removals
  and changed schemas/metadata.
- Version approval and all project binding transitions use typed VedaFlow
  changes with live PDP, precondition and audit checks. Bindings always pin an
  exact approved version, so approving another version cannot move them.
- Client configuration generation emits no plaintext credential. Authentication
  uses a bounded secret reference and neither APIs, logs, traces nor audit
  metadata contain a secret value.
- ToolTestRuns are immutable, name the exact version and harness, and accept
  only discovery/list operations. The catalogue is not a universal execution
  proxy and tool descriptions/permissions never become authorisation.
- Every tenant table has enabled and forced RLS plus completeness coverage;
  denied and cross-tenant identifiers disclose nothing. Generated OpenAPI,
  focused domain/store/gateway/policy/audit tests, authentic stateless fixture
  replay, demo, `make ci` and `make db-test` pass.

## Status

Done 2026-08-25 from `07ce9f3b32d67c4a50e83ff8fed38d6abdd7983f`
under accepted ADR-0086. Commit
`9845186b4dfed7a61c59e997f3c31c85b8840dba`.

## Implementation

- Migration `0053_tool_registry.sql` adds stable servers, immutable ordinal
  versions and raw/normalised capability snapshots, revisioned exact project
  bindings, typed VedaFlow Tool changes and immutable test evidence. All six
  tenant tables use enabled and forced RLS, tenant-qualified constraints and
  database triggers for history and approved-pointer invariants.
- `synveda-types` pins MCP `2026-07-28`, validates only stdio and Streamable
  HTTP metadata, normalises bounded tools/resources/prompts deterministically
  while preserving extensions, and treats descriptions and declared
  permissions as non-authoritative evidence.
- Registration, supported-client import, discovery drift and every binding
  transition enter `AssetKind::Tool` VedaFlow changes. Apply repeats ownership,
  PDP, proposal, payload-hash, current-version and revision checks. A newly
  approved version never moves an existing binding.
- Sixteen public operations list and inspect exact versions, compare
  capabilities, govern bindings, generate secret-reference-only client
  configuration and record a closed set of discovery/list tests. The gateway
  never launches imported stdio commands and exposes no execution proxy.
- `ToolRead` and `ToolWrite` are part of every policy pack, the service-token
  confinement floor and the generated capability explorer. Five audit actions
  record changes, tests and generated configuration using identifiers,
  digests, counts and outcomes rather than capability prose or secrets.

## Acceptance evidence

- Domain validation: `cargo test -p synveda-types tool_registry --lib` — 5/5.
- Complete policy suite: `cargo test -p synveda-policy` — PASS, including
  packs 7/7, approval matrix 6/6 and service confinement 4/4.
- Gateway unit boundary tests: `cargo test -p synveda-gateway --lib
  tool_registry` — 3/3.
- Public lifecycle/database acceptance: `cargo test -p synveda-gateway --test
  tools` — 1/1, including pending and applied review, idempotent discovery,
  drift quarantine/diff, exact binding/repin, immutable evidence, cross-tenant
  404 and audit secret absence.
- Forced-RLS completeness 1/1; OpenAPI inventory 5/5; generated API drift
  PASS; console generated-client contract 186/186.
- `demos/cpr-25-tool-registry.sh` PASS against isolated epoch-2 Postgres: one
  stable server, two immutable versions/snapshots, one exact binding, four
  governed changes and one non-executing test report.
- `make db-test` PASS against disposable `synveda_test_88082`; complete
  `make ci` PASS after the final documentation checkpoint.

The discovery report used by the deterministic acceptance is a protocol-shaped
fixture, not a live external-server claim. No proprietary client or credential
is required for this backend acceptance; authentic client conformance remains
the later adapter package.

---
title: "CPR-26: MCP Tools catalogue product experience"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-26: MCP Tools catalogue product experience

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Replace the console's Tools placeholder with the generated public product
surface for CPR-25's trusted MCP catalogue. The console presents immutable
discovery and binding state; it neither executes a tool nor resolves a secret.

## Acceptance criteria

- The catalogue and stable server detail address expose source, immutable
  versions and digests, transport, pinned MCP protocol, authentication kind,
  trust state, last discovery and exact tools, resources and prompts with their
  descriptions and JSON schemas.
- A quarantined changed version is visually distinct. Its deterministic diff
  against the approved version is visible, and approval enters the shared
  VedaFlow Advanced Reviews surface rather than a second reviewer workflow.
- Import and discovery use generated operations. Exact project bindings can be
  created, enabled, disabled, repinned and removed only when live capability
  forecasts offer the action; the gateway remains authoritative and a changed
  version cannot become active before approval.
- Latest discovery-only ToolTestRun evidence names the adapter harness and
  permitted methods. The UI never implies that descriptions, declared
  permissions or schemas authorise execution.
- Generated client configuration is inspectable without exposing credential
  material. Secret references appear only as present/absent status: their
  opaque values do not enter ordinary rendered output, logs or frontend state
  snapshots.
- The console consumes only generated CPR-25 operation and DTO types. The old
  Tools placeholder and its duplicate test/style residue are deleted.
- Focused helper and real-component acceptance tests cover import, discovery,
  inspection, comparison, review linkage, binding transitions, read-only test
  evidence, configuration and secret non-disclosure. Production build and
  `make ci` pass.

## Status

Done 2026-08-25 from `9845186b4dfed7a61c59e997f3c31c85b8840dba`.
The commit hash is written by the CPR-27 checkpoint.

## Architecture

No new ADR. ADR-0075 defines the generated-contract console boundary and
ADR-0086 defines immutable MCP evidence, VedaFlow approval, exact bindings,
secret references and the no-execution boundary.

## Implementation

- `/console/tools` is a policy-aware generated-API catalogue with bounded
  cursor paging, manifest and supported-client import, selected-project
  configuration and an honest no-project/read-only posture. A stable
  `/console/tools/{server_id}` address owns one server's immutable evidence.
- The detail page shows source, protocol, transport, authentication shape,
  digests, last discovery, explicit metadata-validation/executable-scan state,
  normalised tools/resources/prompts and JSON schemas. It compares the selected
  version with the exact approved head and makes a quarantined version a
  blocking visual state linked to the shared Advanced Reviews plane.
- Project controls admit approved versions only and use the generated create
  and revision-preconditioned update operations for enable, disable, exact
  repin, removal and restoration. Generated configuration retains exact
  binding/version/digest evidence while masking secret-reference identifiers.
- Discovery and read-only test-report forms name the trusted adapter boundary.
  Their method vocabulary is closed to `server/discover` and the three list
  operations; the UI says plainly that the gateway neither connects nor
  executes. Open JSON is defensively sanitised again before rendering.
- The sole `Planned` Tools placeholder and its placeholder acceptance are
  deleted. No new route, DTO copy, policy decision, audit action, schema or
  product authority was added.

## Acceptance evidence

- CPR-26 pure and real-component acceptance: 10/10, covering catalogue,
  stable detail, schema/diff evidence, VedaFlow linkage, binding actions,
  read-only reports, capability forecasts and secret/reference redaction.
- Complete console suite: 196/196; generated operation drift and all repository
  route/schema/client checks pass.
- Production TypeScript/Vite build PASS (66 modules; 386.50 kB JavaScript,
  109.11 kB gzip; 18.54 kB CSS, 4.23 kB gzip).
- Complete `make ci` PASS. `make db-test` is N/A because this package changes
  no schema, persistence, RLS, PDP or database-backed behaviour.
- No in-app browser session was exposed. Real-component SSR and the production
  bundle are the honest UI evidence; no interactive visual-run claim is made.

---
title: "CPR-24: Skills Library product experience"
labels:
  - epic:CPR
  - phase:5
size: L
---

# CPR-24: Skills Library product experience

**Epic:** CPR — Context platform redesign · **Phase:** 5 · **Size:** L

## Description

Make CPR-23's immutable Skill domain usable through one policy-aware Skills
Library. Replace the legacy mutable listing and the Skill-only branch of the
generic proposal renderer; preserve Advanced Reviews as the common VedaFlow
surface.

## Acceptance criteria

- The catalogue and stable detail route use generated public operations only
  and expose current/available immutable versions, digest, provenance, source,
  manifest extensions, client compatibility, quality and scan evidence.
- Exact version files are browsable without collapsing immutable history.
- Personal and selected-project bindings expose enabled, follow-current and
  exact-pin state. Create, enable/disable, pin, unpin and rollback use CPR-23's
  idempotent VedaFlow operations and revision preconditions.
- A capability forecast decides what the UI offers, never what the gateway
  allows; the exact session availability resolver remains authoritative.
- Installation and version update submit complete bundles, retain source
  evidence and display applied, pending-review and rejected outcomes.
- Fixture testing names the validation sandbox and states that it executes no
  scripts. Controlled-client runs remain distinctly labelled.
- Usage shows every recorded stage against its exact version and never
  collapses host-observed evidence into model self-report.
- Declared tools are visibly metadata and never presented as authorisation.
- The hand-written Skill request, mutable-Skill proposal scan/checklist/quality
  UI, CLI rendering branch, old fixture corpus and their dead styles/tests are
  deleted. Artifact-neutral Advanced Reviews remains.
- Pure and real-component acceptance, CLI regression, production build and
  `make ci` pass. `make db-test` is not required because this package changes
  no persisted or gateway behaviour.

## Status

Delivered 2026-08-24 from
`89b5f790a1268e55d8e0df849032ac06a954fd97`. ADR-0075 and ADR-0085 already
decide the generated-client, immutable-version, binding, policy and
controlled-harness boundaries; no new ADR was required.

The primary Skills route is now a generated-contract catalogue with exact
session availability, and every Skill has a stable detail address exposing
immutable versions, exact files, provenance, extension metadata, scanner
evidence, personal/project bindings, controlled tests and version-specific
usage. Installation, complete-bundle updates and binding changes render the
VedaFlow outcome rather than assuming active state moved. Capability forecasts
remove unavailable controls; the gateway still re-decides every operation and
the availability resolver remains the source of truth.

The mutable-Skill branch of the shared review page and CLI, its ten fixture
files, hand-written `skillsAt` request and dead presentation styles are
deleted. Advanced Reviews now renders only the artifact-neutral VedaFlow
contract, while scanner and quality evidence live with the immutable version
they describe in the Skills Library.

Acceptance evidence: Skills helpers/components **10/10**, shared review
**5/5**, complete console **186/186**, CLI **151/151**, both TypeScript checks,
the production Vite build and complete `make ci` pass. The first sandboxed CI
attempt could not bind two loopback listeners; the unrestricted CI run passed
those assertions and the full gate. `make db-test` is N/A because no schema,
store, gateway, RLS, policy or persisted behaviour changed. No in-app browser
was exposed in this environment, so interactive browser QA was unavailable;
real-component server-rendered acceptance and the production bundle are the
recorded UI evidence.

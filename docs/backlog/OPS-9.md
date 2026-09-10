---
title: "OPS-9: Release-shaped beta acceptance"
labels:
  - epic:OPS
  - phase:3
size: L
---

# OPS-9: Release-shaped beta acceptance

**Epic:** OPS — Deployment & operations · **Phase:** 3 · **Size:** L

## Problem and evidence

The public-API PulseBoard walkthrough and canonical Docker reference are
implemented and deterministic gates cover their source contracts. The release
workflow also builds a digest-bound reference archive with an environment
manifest. What remains open is evidence that an independent operator can pull
one published candidate onto an empty supported host, sign in, exercise the
product and cleanly hand the deployment back. A checkout build is not that
evidence.

## Scope

- Install one published reference archive and its manifest-bound images.
- Verify source SHA, archive checksum and resolved OCI image digests.
- Complete Keycloak login, console access and the two-principal PulseBoard
  scenario through generated public APIs.
- Record the exact OS, architecture, Docker, image and verified-client versions
  plus every unavailable external prerequisite.
- Stop the deployment while preserving state, then reinstall the same release
  and prove the persisted product evidence remains usable.

## Non-goals

- No demo-only API, direct database seeder or policy bypass.
- No production, HA, disaster-recovery, SaaS or client-support claim beyond the
  evidence actually run.
- No signing/provenance claim until those controls are implemented.
- No automatic deletion of data, keys, backups or client configuration.

## Architecture seam

Packaging owns only immutable artifacts and their source/image manifest. The
installed `synveda-compose` launcher owns the canonical reference selection;
mutable state and backups remain outside immutable releases. Demo operations
remain ordinary authenticated public-API calls subject to the gateway, Cedar,
forced RLS, VedaFlow and audit boundaries.

## Acceptance criteria

- An empty supported host installs a checksum-verified published candidate and
  pulls every manifest-bound image by digest.
- Bundled Keycloak login, the console and the existing two-principal product
  scenario succeed without source files or direct SQL.
- A stop/reinstall cycle preserves and reopens the scenario with the same keys.
- The evidence records artifact identities, elapsed time and cleanup result.
- Missing credentials, registry access, DNS or browser trust is reported as an
  unavailable prerequisite, never a pass.

## Required tests

- Keep deterministic release packaging, installer and CPR-41 product gates.
- Add a clean-host installed-reference run on every claimed OS/architecture.
- Run one authenticated live lifecycle for each client named verified in
  `adapters/registry.json`; captured or replay-only clients stay labelled.
- Mutation-test image-digest binding, console sign-in, second-principal reuse
  and persisted-state reopening.

## Rollout and rollback

Publish a release candidate first. Retain the previous complete environment
manifest and chart version; never mix image identities across manifests. A
failed beta run stops only its exact Compose project and preserves its state for
diagnosis or retry.

## Dependencies

Release publication, owner-approved distribution terms, supported platforms
and a real authenticated client are external prerequisites. The product owner
must define the beta audience, support channel, data-retention notice and
promotion evidence.

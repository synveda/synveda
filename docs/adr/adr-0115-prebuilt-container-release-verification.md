# ADR-0115: Verify prebuilt containers before announcing a release

- **Status**: Accepted
- **Date**: 2026-09-20
- **Feature(s)**: OPS-8, CPR-45; coordinates OPS-11
- **Deciders**: Owner's request for precompiled Docker deployment and release CI

## Context

The release workflow already builds five deployment images and one browser
fixture on native AMD64 and ARM64 runners. The packaged Compose launcher uses
immutable image digests and never builds source. However, the workflow joins
and announces these artifacts without pulling and executing the final images
on fresh runners. Registry write access does not prove anonymous read access.
The latest public release, v0.2.0, predates the current reference contract.

The server does not need the native CLI, console or client-plugin archives.
Requiring the all-artifact installer unnecessarily restricts a container host
to the smaller native-binary platform matrix.

## Decision

Retain the existing images, registry, canonical Compose graph and tag-only
publication. Separate artifact assembly from the GitHub Release announcement.
Between them, use fresh native AMD64 and ARM64 jobs to anonymously resolve and
pull the exact manifest-bound images, verify platform and source/version labels,
and run bounded, isolated executable/asset smoke checks. A failed architecture,
private package, missing image, wrong source or runtime failure blocks the
announcement. Retain the per-platform report as release evidence.

Document direct download and checksum verification of the existing reference
archive as the server-only installation path. Its launcher, operator DNS/TLS,
private state and pull-only lifecycle remain unchanged. Include that guide in
the archive. The packaged launcher exposes `secrets` and `issuer` by invoking
the existing locked generators with `--if-missing`, allowing TLS preparation
before the first start without a Makefile. It adds no replacement or force
mode. The native installer remains available for supported client hosts.
Dispatch dry runs publish nothing and explicitly skip registry verification;
their synthetic digests must never be presented as an installable release.

## Options considered

1. **Extend the current release boundary** — selected; tests the artifacts
   users download without creating another deployment or installer.
2. **Add a second simplified Compose stack or Docker-only installer** —
   duplicates identity, secret and lifecycle ownership for no runtime benefit.
3. **Only add Docker pull commands to the README** — cannot establish package
   visibility, both architectures or a complete current release.

## Consequences

- Release jobs need public GHCR package visibility and repository publishing
  access. Owners configure these settings; CI does not change visibility.
- Source/revision labels identify intended build inputs, not signed provenance.
  Digest pulls and executable checks do not establish OIDC login, data recovery,
  N-1 compatibility, Windows support or production readiness.
- A new unused version is required. No existing tag or image is republished by
  this implementation. An interrupted workflow may leave unannounced registry
  artifacts; the release owner inspects them before retrying.
- Revisit the archive-only instructions when measured installation failures
  justify an additional installer mode; keep one runtime contract.

## Compliance notes

There is no change to application code, Cedar, forced RLS, VedaFlow, audit,
tenant admission or secret handling. Image smoke containers have no network,
host mounts or deployment credentials. Full product acceptance remains with
the existing Compose and Helm suites.

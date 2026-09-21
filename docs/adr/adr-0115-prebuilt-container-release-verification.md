# ADR-0115: Verify prebuilt containers before announcing a release

- **Status**: Accepted; amended twice
- **Date**: 2026-09-20
- **Feature(s)**: OPS-8, CPR-45, OPS-12; coordinates OPS-11
- **Deciders**: Owner's request for precompiled Docker deployment and release CI

## Context

The release workflow already builds five deployment images and one browser
fixture on native AMD64 and ARM64 runners. The packaged Compose launcher uses
immutable image digests and never builds source. However, the workflow joins
and announces these artifacts without pulling and executing the final images
on fresh runners. Registry write access does not prove anonymous read access.
At the initial decision, the latest public release, v0.2.0, predated the
current reference contract. The amendment below records the next increment.

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

The original image-smoke increment changed no application code, Cedar, forced
RLS, VedaFlow, audit, tenant admission or secret handling. Its containers have no network,
host mounts or deployment credentials. Full product acceptance remains with
the existing Compose and Helm suites.

## Amendment — installation and evaluation contract (2026-09-20)

Accepted for CPR-45 / OPS-11 / OPS-8 at the owner's installation mission.
The in-flight 0.3.0 release is a separate immutable artifact set; the following
increment is unreleased until its own candidate passes acceptance.

Reuse the Compose fragments, database bootstrap, public-API demo and existing
chart. Containerize preparation, leaving Docker Compose itself on the host;
preparation receives only its deployment directory, never a Docker socket.
The released evaluation entry point uses prebuilt immutable images, loopback
exposure and persistent, generated-once private state. Source builds remain a
contributor workflow. Explicit dependency modes remain independent.

For localhost OIDC, support an explicitly configured discovery URL while
retaining the exact canonical issuer and all signature, audience, PKCE and
transport checks. Keycloak's supported dynamic backchannel advertises internal
token/JWKS endpoints; browser authorization retains the public localhost URL.
HTTP backchannels require the existing explicit development-HTTP setting.
External HTTPS deployments retain certificate and hostname verification.

Add `postgres.mode=bundled` to the existing chart: one namespaced persistent
PostgreSQL StatefulSet using the existing PostgreSQL image and bootstrap tools,
ordinary separate runtime roles and operator-owned Secret references. No
operator, CRD, cluster-wide RBAC or privileged init container is installed.
`cnpg` remains an explicit preinstalled-operator option. Bundled Keycloak has
its own database/credential contract regardless of application database mode.
Retain PVCs and independently back up identity state and encryption material;
neither bundling nor Kubernetes establishes HA or production readiness.

Keep the approved static site and brand. A small checked publication manifest
distinguishes published artifacts from pending instructions. Release and
platform claims require native runtime evidence, not chart renders or builds.

## Amendment — two registries, one build (2026-09-21)

Accepted for OPS-12 / OPS-8 at the owner's consumer installation request.
Extend this workflow rather than creating another publisher. Each existing
native architecture build exports the same result, including BuildKit SBOM and
provenance descriptors, to Docker Hub and GHCR. Docker Hub's namespace and login
are owner settings, never assumed to be `synveda`. Join and inspect both
registries independently; compare the complete child descriptor sets and record
each destination's own index digest. Consumer Compose and Helm overlays use
Docker Hub digests; GHCR remains the verified mirror. Upstream pins do not change.

Version and architecture tags are write-once: preflight refuses existing tags
and ambiguous lookup failures. Owners restrict publishing and enable registry
immutability where available. A partial publication requires owner investigation,
not an automatic rebuild/overwrite. Dispatch remains nonpublishing and its
synthetic manifest explicitly states that it cannot be installed. No rolling
alias is introduced. Published v0.4.0 and its instructions remain unchanged.

The native anonymous pull gate verifies both registries from empty credentials.
The existing complete Docker and Kind gates consume the Docker Hub bundle.
Publisher credentials are available only in protected publishing jobs. Final
checksums and their image inventory are authenticated by an identity-bound
GitHub build attestation, with a pinned action and explicit workflow/tag/source
verification instructions. Checksums alone remain corruption detection. This
does not claim binary code signing, notarization or completed hosted verification.

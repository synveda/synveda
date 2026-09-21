# Publishing a consumer release

OPS-8 / OPS-12; [ADR-0115](adr/adr-0115-prebuilt-container-release-verification.md).
The two-registry and attestation changes are configured, **not published or
qualified**. The published v0.4.0 remains unchanged and uses GHCR, its original
installer and its original checksum-only trust boundary. Never rerun publication
against that version. The [installation guide](../deploy/compose/PREBUILT.md)
continues to describe those existing downloads.

Published v0.4.0 uses `e59284619567d6a13b70ce3f3b3e81121b7621e6`.
Its original 15 public assets and OCI chart matched the qualified bytes on
anonymous download. Its upload-only failure was recovered from the original
workflow artifacts; that workflow remains failed while its installation jobs
and recovered release are verified. v0.3.0 also remains immutable.

## Native client candidates

OPS-12 adds `synveda-client-VERSION-TARGET.tar.gz` for native macOS/Linux x86_64
and arm64, plus `.zip` for Windows x86_64/arm64. Each contains the existing CLI and adapters plus private Node pinned
by upstream SHA-256 in [the runtime inventory](../scripts/node-runtimes.json),
with Synveda and complete Node licence notices. It contains no server binaries
or deployment bundle. The [candidate guide](CONSUMER_CLI.md#client-archive-candidate)
owns its explicit `SYNVEDA_INSTALL_MODE=client` and PowerShell instructions.

Each native job executes its extracted archive with no Node/Docker on the
installer PATH. Unix jobs replay existing Codex/Copilot fixtures using private
Node. Windows jobs require native Rust/Node storage interoperability, PowerShell
install/reinstall, hook entry-point smoke and unsafe ZIP/installer refusals.
Assembly requires all six `synveda-client-report-TARGET.json` reports
to match archive digests, source, target and runtime pin. Tagged builds also
require a clean source tree and matching CLI version. Reports enter SHA256SUMS
before attestation and accompany the release assets. A configured job is not
hosted execution evidence. [OPS-12](backlog/OPS-12.md) records local macOS arm64
archive evidence and the exact hosted Windows qualification results. Real
issuer/harness use of these archives and cold network download measurements
remain open. The two historical server binary archives
and system-Node plugin archive retain their existing contract.

## Owner setup

1. Select a Docker Hub namespace you own. Do not assume `synveda` is available.
   Create public repositories named `product`, `postgres`, `cnpg-postgres`,
   `keycloak`, `proxy` and `browser-acceptance`. The last image is required by
   the opt-in sample and acceptance/recovery tour. Keep the upstream Collector
   and Prometheus at their existing reviewed digest pins.
2. Enable immutable version tags on Docker Hub where available. Treat all
   version and architecture tags on both registries as write-once even where
   the registry does not enforce that policy. Restrict writers and protect
   GitHub `v*` tags. There is no `latest` or rolling alias in this release plan.
3. Create an expiring push credential with only read/write access to those six
   repositories. Prefer an organization token when the account supports it;
   otherwise use a dedicated publisher's personal access token, without delete
   permission. Set an owner and rotation reminder outside this repository.
   Never paste the credential into an issue, shell argument or committed file.
4. In GitHub repository variables set `DOCKERHUB_NAMESPACE` and
   `DOCKERHUB_USERNAME`. Create the protected environment named `release`,
   require an owner reviewer, restrict its deployment tags, and store
   `DOCKERHUB_TOKEN` as an environment secret. Make GHCR packages public and
   grant this repository write access. `release-dry-run` requires no secret and
   must not hold publisher credentials. Namespace is a repository variable
   because the read-only version job resolves it before publishing jobs begin.
5. Review the pinned `actions/attest` revision and approve GitHub OIDC as the
   publisher identity for `.github/workflows/release.yml` on protected tags.
   The final publishing job alone has attestation and OIDC permissions. No
   signing private key is stored in GitHub. This authenticates the checksum
   inventory; it is not macOS notarization, Authenticode or a complete archive
   SBOM. Those native distribution requirements remain OPS-12 work.
6. Authorize a new unused coordinated version and update the existing versioned
   product/chart/package contracts, including console/adapters and starter image
   versions. Regenerate OpenAPI, console client and SDK contract metadata after
   changing the workspace version. Require normal CI on the intended committed
   source; release qualification does not replace it. Run the nonpublishing `workflow_dispatch`
   first. It can use a synthetic version and namespace; it builds artifacts
   but never logs in, pushes, signs or creates a Release. Its inventory has
   `published: false` and cannot pass anonymous release verification.
7. After reviewing local and hosted results, separately authorize the new tag
   push. The existing `v*` workflow is the only publisher. Confirm the protected
   environment approvals, both native image reports, Docker and all four Kind
   ownership-mode reports, and final attestation verification. Only then update
   the publication manifest, README, site and UI to name the new release.

The workflow checks every existing image tag before writing. A missing secret,
private pull, authentication/rate-limit/network error, incomplete architecture,
lost BuildKit attestation or different mirror descriptor fails the release.
An interrupted publication can leave unannounced artifacts. Inspect and retain
those bytes; do not delete/rebuild/overwrite them automatically. Recover using
the original verified artifacts under an explicitly reviewed owner procedure,
or authorize a new version. Do not move a published Git tag.
For upload-only recovery, use the original `release-assets` and both
`release-verification-*` artifacts, verify all checksums/source/image identities,
and retain the authenticated final checksum inventory. The old checksum-only
v0.4.0 recovery is not a signing bypass for a new release.

Docker documents [multiple registry exports](https://docs.docker.com/build/ci/github-actions/push-multi-registries/),
[personal tokens](https://docs.docker.com/security/access-tokens/personal-access-tokens/)
and [organization tokens](https://docs.docker.com/security/access-tokens/organization-access-tokens/).
The workflow retains the existing reviewed Docker action pins rather than
upgrading them as part of this change.

## Artifacts and verification

Each native AMD64/ARM64 image is built once from the same source and lockfiles
and exported to both registries with source/revision/version labels, SBOM and
provenance. Each registry's final multi-platform index is independently
assembled, inspected and hashed. The versioned `synveda-registry-images-*.json`
records both destinations and their complete child descriptors. The consumer
reference archive, its `environment.json`, launcher and Helm overlays use
the **Docker Hub destination digests**. The chart's unconfigured source defaults
remain GHCR because the owner namespace is unknown until release assembly.
Use the release's immutable overlay when installing the packaged chart.

Native verification starts with an empty Docker credential store. It checks
both registries, including descriptor parity, anonymous digest pulls, labels
and executable assets. The full existing Docker lifecycle/browser login/sample/
paired-recovery and Kind checks then consume the Docker Hub bundle. The archive
also includes the plain-Compose candidate and `synveda-recovery`. Both native
jobs must run `qualify-release.mjs --consumer-candidate` against those extracted
bytes. The required `release-consumer-{amd64,arm64}.json` reports cover stopped
writers, a private empty-target restore, original sealed keys, audit/key
verification, real browser access and the original sample receipt. They join
the checksummed, attested release inventory; a missing or failed report blocks
publication. The host-state Docker and Kind gates remain required. These
reports distinguish image smoke from full deployment evidence; they do not
qualify Windows, Docker Desktop, OpenShift or an N-1 migration.

The full drills are `scripts/qualify-release.mjs` and
`scripts/qualify-kubernetes-release.mjs`. They require Docker, Helm 4.2.3,
Kind 0.32.0, kubectl 1.36.1, Node, Ruby, Python and OpenSSL on the qualification
runner. They do not build images. Configure public visibility and repository
Actions access for `ghcr.io/synveda/charts/synveda` too: anonymous OCI retrieval
must be byte-identical to the downloadable chart. The source checkout's
`assets/brand` and `assets/product` directories are never upload candidates.

After those gates, GitHub attests the final `SHA256SUMS`, which covers all
archives, overlays, destination digests and native reports. The downloadable
`SHA256SUMS.sigstore.json` carries that attestation. For a future approved
release, verify it with a current trusted GitHub CLI before trusting its
checksums. Supply the exact approved tag and source commit from the release:

```sh
gh attestation verify SHA256SUMS --bundle SHA256SUMS.sigstore.json \
  --repo synveda/synveda \
  --signer-workflow synveda/synveda/.github/workflows/release.yml \
  --source-ref "refs/tags/$RELEASE_TAG" --source-digest "$SOURCE_SHA" \
  --deny-self-hosted-runners
```

Then verify the desired downloaded archive against exactly its checksum entry
before extracting it. A checksum from an unauthenticated download does not
establish publisher identity. Failed or absent attestation is not permission to
skip verification. GitHub documents [attestation verification](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).

The current installer does not yet enforce this new attestation boundary;
integrating verification without imposing GitHub CLI or Docker on client-only
installation remains open. Do not advertise this source installer as an
authenticated consumer installer. Artifact authentication also does not prove
OS code signing, absence of vulnerabilities or safe data migration. An owner
incident response must revoke publishing access, identify affected immutable
digests/tags and publish a reviewed replacement; never silently replace bytes.

Homebrew and WinGet publication remain blocked until the native direct artifact
flow is qualified. Owners must supply signing/notarization identities where
needed and authorize package-manager submissions separately. No package-manager
install command or Docker Hub release is advertised before it exists.

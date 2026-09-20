# Publish a container release

The [Release workflow](../.github/workflows/release.yml) builds the existing
native clients, client hooks, console, Helm chart and Docker reference from
one source revision. A version tag publishes images to GHCR; a manual workflow
dispatch builds and packages a dry run without publishing anything.

As of 2026-09-20, the latest public release is v0.2.0. It contains the retired
profile archive and predates the current database schema. Do not reuse that
tag or claim it is compatible with the current reference launcher.

## Before the first current release

1. Select a new, unused version and update the workspace/chart version inputs
   together. `make check-release-parity` checks their agreement. Commit the
   intended source, including required brand assets, and require the normal CI
   gates for that revision. The release workflow is not a replacement for CI.
2. Confirm the repository's Actions token may publish packages and releases.
   Existing packages must grant this repository Actions access. The workflow
   sets the source, revision and version OCI labels on every first-party image.
3. In the Synveda organisation's package settings, make the six image packages
   below public. On first creation GitHub may make a package private even when
   the repository is public. New packages may need this change after the first
   push; the anonymous verification job will fail until it is done. Inspect the
   failure, change visibility, then rerun the failed jobs. CI deliberately does
   not change organisation visibility settings. See
   [GitHub's Container registry guide](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry).
4. Configure equivalent access/visibility for the OCI chart at
   `ghcr.io/synveda/charts/synveda` if distributing it publicly. The workflow
   verifies the chart with its publishing identity; this is separate from the
   anonymous image checks.
5. Run a manual **Release** dispatch against the intended revision. Its workflow
   artifacts contain **synthetic image digests**. They exercise packaging and
   checksums and cannot be installed. They are not published as a GitHub Release.

## What a tag does

Push the approved `v<version>` tag after the source version matches it. Image
builds run natively on Linux AMD64 and ARM64, with separate architecture cache
scopes. There is no `latest` tag or automatic deployment to a running server.

| Package | Purpose |
| --- | --- |
| `ghcr.io/synveda/product:<version>` | Gateway, worker, operator CLI and compiled console |
| `ghcr.io/synveda/postgres:<version>` | Single-host PostgreSQL with pgvector and bootstrap/recovery tools |
| `ghcr.io/synveda/keycloak:<version>` | Bundled OIDC provider and reviewed convergence helpers |
| `ghcr.io/synveda/proxy:<version>` | Reference reverse proxy |
| `ghcr.io/synveda/cnpg-postgres:17.11-synveda-<version>` | Optional Kubernetes PostgreSQL image |
| `ghcr.io/synveda/browser-acceptance:<version>` | Acceptance fixture; not required for normal server operation |

The `assemble` job joins the architecture tags and packages immutable image
digests in `environment.json` and the Helm image overlays. It checks archive
presence, writes SHA256SUMS, and pushes/pulls the OCI chart for byte comparison.

Two fresh `verify-images` runners use empty Docker credential directories. Each
checks archive checksums, resolves both platform descriptors, pulls all six
first-party and both pinned upstream images by digest, checks the native
architecture and release labels, and executes the relevant packaged tools.
Product checks include the compiled console and its font licence. Containers
have no network, host mounts or deployment credentials. A failed pull,
architecture, label or executable prevents the GitHub Release announcement.

`publish` runs only after both jobs succeed. It attaches the complete archive
set, Helm overlays and checksummed `release-images-amd64.json` and
`release-images-arm64.json` reports. BuildKit generates image SBOM/provenance
attestations; identity-bound signature verification remains unimplemented.

If a workflow fails after pushing images, registry artifacts may exist without
an announced release. Inspect the run before retrying. Never retag a released
version to different source, or suggest a partially published set to users.

## After publication

Download the release from an empty target using the
[prebuilt Docker guide](../deploy/compose/PREBUILT.md), check the manifest and
reports, configure DNS/TLS, and run the existing reference acceptance. Record
real PKCE login, a governed Session/Knowledge/Context flow, persistence,
recovery and compatibility evidence in [CPR-45](backlog/CPR-45.md). Qualify the
packaged chart separately under [OPS-11](backlog/OPS-11.md).

The image reports prove pullability and isolated executable/asset checks.
They do not prove a working deployment, Windows/WSL2 support, full Linux/Docker
Desktop parity, a supported N-1 upgrade, or production readiness. Do not remove
the [readiness gaps](PRODUCTION_READINESS.md) on the strength of a green image
job alone. Update the dated availability note in the root README and prebuilt
guide only once a compatible public release and its actual evidence exist.

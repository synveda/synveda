# Publish a container release

The [Release workflow](../.github/workflows/release.yml) builds the existing
native clients, client hooks, console, Helm chart and Docker reference from
one source revision. A version tag publishes images to GHCR; a manual workflow
dispatch builds and packages a dry run without publishing anything.

The **v0.3.0** workflow passed its builds and isolated image checks, but its
publication failed because the release page already existed. Keep that release
immutable. The additive Docker/Kubernetes installation
increment is prepared as **0.4.0**, following the existing pre-1.0 minor-version
policy. It is unpublished. A tag's existence or an in-progress release page
does not establish that its complete artifact set is installable.

## Before the first current release

1. Keep the workspace, console/adapters, chart and starter image versions
   aligned with the intended unused release tag. Regenerate OpenAPI, the console
   client and SDK contract metadata after changing the workspace version.
   `make check-release-parity` checks workspace/chart agreement. Commit the
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
   verifies identical chart bytes first with its publishing identity and then
   with an empty registry configuration on each qualification runner.
5. Run a manual **Release** dispatch against the intended revision. Its workflow
   artifacts contain **synthetic image digests**. They exercise packaging and
   checksums and cannot be installed. They are not published as a GitHub Release.

## What a tag does

Push the approved `v<version>` tag after the source version matches it. Image
builds run natively on Linux AMD64 and ARM64, with separate architecture cache
scopes. There is no `latest` tag or automatic deployment to a running server.

| Package | Purpose |
| --- | --- |
| `ghcr.io/synveda/product:<version>` | Gateway, worker, CLI, compiled console and one-shot preparation utility |
| `ghcr.io/synveda/postgres:<version>` | Compose/bundled-chart PostgreSQL with pgvector and bootstrap/recovery tools |
| `ghcr.io/synveda/keycloak:<version>` | Bundled OIDC provider and reviewed convergence helpers |
| `ghcr.io/synveda/proxy:<version>` | Reference reverse proxy |
| `ghcr.io/synveda/cnpg-postgres:17.11-synveda-<version>` | Optional Kubernetes PostgreSQL image |
| `ghcr.io/synveda/browser-acceptance:<version>` | Optional sample preparation and real-browser acceptance |

The `assemble` job joins the architecture tags and packages immutable image
digests in `environment.json` and the Helm image overlays. It checks archive
presence, writes SHA256SUMS, and pushes/pulls the OCI chart for byte comparison.

Two fresh `verify-images` runners use empty Docker credential directories. Each
checks archive checksums, resolves both platform descriptors, pulls all six
first-party and both pinned upstream images by digest, checks the native
architecture and release labels, and executes the relevant packaged tools.
Product checks include the compiled console and its font licence. These initial
smoke containers have no network, host mounts or deployment credentials.

The same runners then execute `scripts/qualify-release.mjs` against the extracted
archive: private preparation, real loopback console login/logout, governed
opt-in sample and repeat, retained-volume recreation, paired database/key backup,
explicit reset and fresh restore. `scripts/qualify-kubernetes-release.mjs` loads
the already-pulled native image bytes into disposable Kind, uses the packaged
chart for all four dependency combinations, drills migration serialization and
joint recovery for bundled/external ownership, and tests the documented
loopback port-forward with a real browser. Neither qualifier builds images.
Anonymous OCI chart retrieval must match the downloadable archive exactly.
Any failure prevents the GitHub Release announcement; no required test is
silently skipped. The commands need Docker, Helm 4.2.3, Kind 0.32.0, kubectl 1.36.1,
Node, Ruby, Python and OpenSSL on the **qualification runner**, not on a Docker
installer's host.

`publish` runs only after both jobs succeed. It attaches the complete archive
set, Helm overlays and checksummed `release-images-<arch>.json`,
`release-docker-<arch>.json` and `release-kubernetes-<arch>.json` reports.
BuildKit generates image SBOM/provenance
attestations; identity-bound signature verification remains unimplemented.

If a workflow fails after pushing images, registry artifacts may exist without
an announced release. Inspect the run before retrying. Never retag a released
version to different source, or suggest a partially published set to users.

## After publication

Download the release from an empty target using the
[prebuilt Docker guide](../deploy/compose/PREBUILT.md), check the manifest and
reports, and run the default loopback evaluation. Use the explicit reference
configuration for public DNS/TLS or existing services. Record
real PKCE login, a governed Session/Knowledge/Context flow, persistence,
recovery and compatibility evidence in [CPR-45](backlog/CPR-45.md). Qualify the
packaged chart separately under [OPS-11](backlog/OPS-11.md).

Image reports prove pullability and isolated executable/asset checks; Docker
and Kubernetes reports separately prove their executed installation scenarios.
None establishes macOS Docker Desktop, Windows/WSL2, OpenShift, a supported
published N-1 upgrade, or production readiness. Do not remove
the [readiness gaps](PRODUCTION_READINESS.md) on the strength of a green image
job alone. Change `docs/installation.json` to `published`, update the checked
README/guide markers and publish matching Pages copy only once the complete
compatible public release and its evidence exist. Main-branch copy currently
labels 0.4.0 download instructions as pending.

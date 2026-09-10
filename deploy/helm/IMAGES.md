# Container image inventory

Every image the canonical Compose graph or its deployment fixtures can
reference, every image the Helm chart can reference, every image the
release workflow publishes, and every base image the images we build are built
from. `scripts/check-chart-images.mjs` (in `make ci`) fails
the build when one of those surfaces names an image that is not on this list,
**tag included** — so a version bump is a diff somebody reads rather than a
silent change of what is installed.

Fixture-only deployment Dockerfiles are inventoried as well and are labelled
explicitly; their presence here is not a claim that those images ship in the
reference deployment.

The canonical Compose bundle and Helm chart are customer-facing surfaces, so
their release images have the same inventory requirement as their bases and
third-party runtime dependencies.

That is the modest half of the job. The check proves the list is complete;
it cannot prove a licence is admissible, because a container image carries
no machine-readable licence. A human reads those and records what they
read, here.

## Why this file exists

The repository licence rule is enforced by `cargo-deny` over crates,
`check-npm-licences` over packages and — since ADR-0061 —
`check-corpus-licences` over corpora. A Helm chart introduces a fourth
kind of artefact, and until this file nothing in the repository looked at
it. That is the same gap in the same shape as the one that let a CC BY-NC
corpus reach a published phase demo goal untouched by any check (EVAL-7).

The entry to read first on any bump is **text-embeddings-inference**: an
inference server's licence is exactly the kind that changes between
releases, and that image carries both a binary and a model.

## Images the canonical Compose graph and fixtures run

The development names below are locally built outputs, not pullable release
claims. Reference deployment replaces each Synveda-built name with a digest
from a matching environment manifest. The Collector is a direct third-party
runtime dependency and is pinned here exactly.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `synveda/product:dev` | canonical gateway, worker and one-shot product commands | ours | Development output of `deploy/compose/product/Dockerfile`; reference selects the same image contract by digest. |
| `synveda/postgres:17.11-dev` | bundled PostgreSQL and database bootstrap | ours over PostgreSQL-licensed PostgreSQL | Development output of `deploy/compose/postgres/Dockerfile`. |
| `synveda/keycloak:26.7.2-dev` | bundled Keycloak and realm convergence | ours over Apache-2.0 Keycloak | Optimized development output of `deploy/compose/keycloak/Dockerfile`. |
| `synveda/proxy:2.11.4-dev` | canonical reverse proxy | ours over Apache-2.0 Caddy | Development output of `deploy/compose/proxy/Dockerfile`. |
| `otel/opentelemetry-collector-contrib:0.159.0@sha256:1f2c54a30e713fac6b3ae77a1ec84010c2007e29ced8ec666214fc2f6739c1cc` | private core Collector | Apache-2.0 | Exact official Collector Contrib runtime; the application emits only OTLP to this private seam. |
| `prom/prometheus:v3.13.3-distroless@sha256:2e9a8ad75536755572d703e645fcc39c8104d9f0215d49d613db35194b0d8bc2` | optional private Compose metrics backend | Apache-2.0 | Exact official multi-platform distroless image. It receives only the Collector's private metrics fan-in, applies 72-hour/1-GB TSDB block-retention thresholds (not a disk quota), and publishes its operator UI on host loopback only. |
| `synveda/browser-acceptance:1.62.1-dev` | Compose acceptance fixture with the exact Synveda CLI and Playwright | Synveda's licence is not yet selected; fixture code and Playwright are Apache-2.0; bundled browsers and system components retain their upstream licences | Locally built no-capture one-shot used by reference acceptance, not a release product service. The CLI is copied from the same source build as the gateway/worker. Playwright's licence, upstream NOTICE and the seccomp provenance notice are retained in the image. |
| `synveda-db-test-postgres:local` | isolated database acceptance fixture | ours over PostgreSQL-licensed PostgreSQL | Local-only database-test build; never an operator topology. |

## Release image set

The release workflow builds native amd64 and arm64 images, joins their indexes
and records the resolved index digests in the packaged reference deployment's
`environment.json`. `<version>` represents that release input; this source
wiring is not evidence that a tag has actually been published or pull-tested.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/synveda/product:<version>` | canonical gateway, worker and one-shot commands | ours | The role-neutral product image also used by the chart. |
| `ghcr.io/synveda/postgres:<version>` | `postgres` | ours (see bases) | Postgres 17 with pgvector, from `deploy/compose/postgres/Dockerfile`. The same epoch-3 extension shape is used by dev, release and Helm. |
| `ghcr.io/synveda/cnpg-postgres:17.11-synveda-<version>` | Helm/CloudNativePG release input | ours (see bases) | The CloudNativePG data-plane image built from `deploy/helm/postgres/Dockerfile`; its tag begins with the real PostgreSQL version required for direct `imageName` validation and retains the Synveda release version as its suffix. |
| `ghcr.io/synveda/keycloak:<version>` | bundled reference identity provider | ours over Apache-2.0 Keycloak | The optimized production-mode Keycloak image built from `deploy/compose/keycloak/Dockerfile`; it adds no provider-specific product authority. |
| `ghcr.io/synveda/proxy:<version>` | reference reverse proxy | ours over Apache-2.0 Caddy | The Caddy image built from `deploy/compose/proxy/Dockerfile` with its inherited file capability removed before non-root runtime. |
| `ghcr.io/synveda/browser-acceptance:<version>` | release acceptance fixture | Synveda's licence is not yet selected; fixture code and Playwright are Apache-2.0; bundled browsers and system components retain their upstream licences | Digest-bound one-shot needed by reference acceptance, restore and upgrade smoke. It is not a product service. |

## The per-architecture embedder pins

Upstream publishes two TEI builds and versions only one of them. These pins
belong only to the isolated retrieval evaluation fixture selected by the
`Makefile`; TEI is not a core reference service.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `TEI_IMAGE_x86_64` | **read on every bump** | amd64 retrieval-evaluation server. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561` | `TEI_IMAGE_arm64` | **read on every bump** | Apple Silicon. There are no versioned arm64 tags, so this is pinned by *commit* rather than left on `cpu-arm64-latest` — which means a bump is a deliberate act and the licence at that commit is what applies. It agrees with the amd64 release to float32 rounding (cosine 1.000000000, max abs diff 7e-8, measured 2026-07-26), which is the property that matters when Knowledge revision vectors retain a model and dimension. |

## Images the chart runs

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/synveda/product:<appVersion>` | `image.repository` | ours | The product. Both binaries: the gateway serves, the CLI migrates and issues SCIM credentials. Built from `deploy/compose/product/Dockerfile`; the release workflow is configured to join its native amd64/arm64 builds under this GHCR coordinate. |
| `ghcr.io/synveda/cnpg-postgres:17.11-synveda-<appVersion>` | default `postgres.image` | ours (see bases) | Postgres for CloudNativePG plus pgvector and the shared content-free database bootstrap command. Built from the repository root with `deploy/helm/postgres/Dockerfile`; the leading version satisfies CloudNativePG's direct-image contract and the release workflow joins native amd64/arm64 builds under the paired Synveda application suffix. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `tei.image`, optional | **read on every bump** | The embedder, when `embedder: tei` and `tei.enabled`. Serves BAAI/bge-m3, whose weights are a separate licence from the server's. |

## Base images we build on

| Image | Built into | Licence | Notes |
|---|---|---|---|
| `ghcr.io/cloudnative-pg/postgresql:17.11-202608310816-standard-bookworm@sha256:e8ffaff9d17011fb71f264d857c3b4c54cb86ed0443a4ecb0298cb96be708d4e` | cnpg-postgres | Apache-2.0 (CNPG) over PostgreSQL-licensed Postgres and extensions | Exact multi-architecture CNPG PostgreSQL 17.11 standard Bookworm base. It already contains pgvector; the derivative verifies that package and adds only Synveda's support files. |
| `rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc` | gateway, Compose PostgreSQL, Keycloak and CloudNativePG mounted-input helper build stages | MIT/Apache-2.0 toolchain; build-only system compiler | Matches `rust-toolchain.toml` and the Debian 12 runtime ABI; also provides the digest-pinned native C compiler so no mutable apt compiler packages enter helper builds. |
| `node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5` | gateway console stage | MIT | Builds the console bundle. Never in the runtime stage. |
| `debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171` | gateway runtime stage | various, all Debian-main | Runtime: `ca-certificates` for OIDC discovery, `curl` for the healthcheck. |
| `postgres:17.11-bookworm@sha256:051f7b7b3abdd564d5d1bd1e8c4b9c1b6e77087d1dd22020ede611c096a272e0` | Compose PostgreSQL image | PostgreSQL | Exact multi-architecture upstream PostgreSQL 17.11 base for the bundled reference database. |
| `quay.io/keycloak/keycloak:26.7.2@sha256:9d1f1b2b7261ff53c66cb1092dfcdc34a5fb77e81f9e6a6e75b8b6a795de8067` | optimized Compose Keycloak image | Apache-2.0 | Exact multi-architecture Keycloak production base, used for both the optimized build and runtime stages. |
| `caddy:2.11.4-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648` | Compose reverse proxy image | Apache-2.0 over Alpine packages | Exact multi-architecture Caddy base with its file capability removed before runtime. |

## Fixture-only build images

This image is not part of the product or reference service graph. It builds a
one-shot development acceptance fixture and is inventoried because repository
deployment Dockerfiles are a closed surface.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `mcr.microsoft.com/playwright:v1.62.1-noble@sha256:dcc5531e97840b9b5e794f2814476b21571c5124a3fca2267d73041f56e7580e` | browser-acceptance fixture | Playwright is Apache-2.0; the bundled Chromium, Firefox, WebKit and Ubuntu/system components retain their upstream licences | Exact official multi-architecture Playwright fixture base. The matching `playwright-core` package and reviewed non-root Chromium sandbox profile are pinned to 1.62.1; fixture-only use is not a product-image licence claim. |

## Images the install test runs, and the chart never does

`make check-chart-images` does not scan `demos/` — the chart is what a
customer installs, and the test's scaffolding is not shipped to anyone. They
are recorded here anyway, because "not shipped" is a reason to hold a lower
bar, not none.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/synveda/keycloak:<appVersion>` | `demos/fixtures/ops-2/keycloak.yaml` | ours over Apache-2.0 Keycloak | The same optimized production-mode image as the reference deployment, at a private Service DNS name. |
| `postgres:17.11-bookworm@sha256:051f7b7b3abdd564d5d1bd1e8c4b9c1b6e77087d1dd22020ede611c096a272e0` | `demos/fixtures/ops-2/keycloak.yaml` | PostgreSQL | Disposable physically separate database proving the Helm fixture does not place Keycloak tables or authority in Synveda's CloudNativePG cluster. |
| `node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5` | `demos/fixtures/ops-2/client-pod.yaml` | MIT | Plays the browser half of `synveda login`. |
| CloudNativePG operator | applied by the demo, version pinned in it | Apache-2.0 | Installed separately by design; the chart renders a `Cluster` for it. |

## Extensions available in `synveda/cnpg-postgres`

Not images, and not covered by `cargo-deny` either, so they are recorded
in the same place:

| Extension | Version | Licence |
|---|---|---|
| pgvector | PGDG `postgresql-17-pgvector` 0.8.6-1.pgdg12+1, supplied by the pinned CNPG base | PostgreSQL |

Epoch 3 uses no other Postgres extension. Bounded graph expansion, capture
leasing and durable operations use ordinary tenant-bound tables.

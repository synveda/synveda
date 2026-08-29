# Container images we ship

Every image the Helm chart can reference, every image the **released
single-node profile** runs, and every base image the images we build are
built from. `scripts/check-chart-images.mjs` (in `make ci`) fails the build
when one of those surfaces names an image that is not on this list, **tag
included** — so a version bump is a diff somebody reads rather than a
silent change of what is installed.

The release profile joined with OPS-8 (ADR-0065 decision 9). It is the
stronger case, not the weaker one: the chart is what a customer's platform
team installs deliberately, and `deploy/release/` is what anybody installs
with one `curl | sh`. Adding it found four images no check had looked at,
one of them an inference server pinned by commit.

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

## Images the released single-node profile runs

`deploy/release/docker-compose.yml`, the bundle `scripts/install.sh` unpacks
under `~/.synveda/profile`. `<version>` is the release tag, substituted by
`scripts/package-release.sh` — inventoried as a placeholder for the same
reason the chart's `<appVersion>` is.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/synveda/gateway:<version>` | `gateway`, retained profile | ours | The product image also used by the chart. The profile lifecycle is withdrawn during CPR-45 and is not a default install. |
| `ghcr.io/synveda/postgres:<version>` | `postgres` | ours (see bases) | Postgres 17 with pgvector, from `deploy/compose/postgres/Dockerfile`. The same epoch-3 extension shape is used by dev, release and Helm. |
| `ghcr.io/sebadob/rauthy:0.35.2` | `rauthy` | Apache-2.0 | Cutover residue in the withdrawn release profile; not a current provider claim. |
| `jaegertracing/jaeger:2.19.0` | `jaeger` | Apache-2.0 | Traces on port 16686. FND-5's exporter targets it; the profile starts it because an install nobody can see inside is harder to trust. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `tei`, optional | **read on every bump** | The amd64 embedder, when `--embedder tei`. See the arm64 row below. |

## The per-architecture embedder pins

Upstream publishes two TEI builds and versions only one of them. The pins
live in the `Makefile` (`TEI_IMAGE_*`), which is what the contributor loop
resolves. The withdrawn release artifact carries an explicit/default image
selection for deterministic packaging evidence only.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/huggingface/text-embeddings-inference:cpu-arm64-sha-4150561` | `TEI_IMAGE_arm64` | **read on every bump** | Apple Silicon. There are no versioned arm64 tags, so this is pinned by *commit* rather than left on `cpu-arm64-latest` — which means a bump is a deliberate act and the licence at that commit is what applies. It agrees with the amd64 release to float32 rounding (cosine 1.000000000, max abs diff 7e-8, measured 2026-07-26), which is the property that matters when Knowledge revision vectors retain a model and dimension. |

## Images the chart runs

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `synveda/gateway:<appVersion>` | `image.repository` | ours | The product. Both binaries: the gateway serves, the CLI migrates and issues SCIM credentials. Built from `deploy/compose/gateway/Dockerfile`. |
| `synveda/enterprise-postgres:17` | `postgres.image` | ours (see bases) | Postgres for CloudNativePG plus pgvector and the shared content-free database bootstrap command. Built from the repository root with `deploy/helm/postgres/Dockerfile`; its schema and role contract match the Compose reference. |
| `ghcr.io/huggingface/text-embeddings-inference:cpu-1.8.1` | `tei.image`, optional | **read on every bump** | The embedder, when `embedder: tei` and `tei.enabled`. Serves BAAI/bge-m3, whose weights are a separate licence from the server's. |

## Base images we build on

| Image | Built into | Licence | Notes |
|---|---|---|---|
| `ghcr.io/cloudnative-pg/postgresql:17@sha256:fa6e2b2e14d19a109cc142cf857d328420bb7f1656b08c96e08be377692247ab` | enterprise-postgres | Apache-2.0 (CNPG) over PostgreSQL-licensed Postgres | Exact multi-architecture CNPG PostgreSQL 17 base; the helper is executed again in this final image. |
| `rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc` | gateway, Compose PostgreSQL and Keycloak mounted-input helper build stages | MIT/Apache-2.0 toolchain; build-only system compiler | Matches `rust-toolchain.toml`; also provides the digest-pinned native C compiler so no mutable apt compiler packages enter helper builds. |
| `rust:1.96.0-bullseye@sha256:7069898d5edfc11b0ba498ecefbcc5438f6390b3ce0be11a9750cf39cab7e02f` | CloudNativePG mounted-input helper build stage | MIT/Apache-2.0 toolchain; build-only system compiler | Matches the Debian 11 glibc ABI in the pinned CloudNativePG final image; a final-stage execution probe rejects ABI drift. |
| `node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5` | gateway console stage | MIT | Builds the console bundle. Never in the runtime stage. |
| `debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171` | gateway runtime stage | various, all Debian-main | Runtime: `ca-certificates` for OIDC discovery, `curl` for the healthcheck. |

## Images the install test runs, and the chart never does

`make check-chart-images` does not scan `demos/` — the chart is what a
customer installs, and the test's scaffolding is not shipped to anyone. They
are recorded here anyway, because "not shipped" is a reason to hold a lower
bar, not none.

| Image | Where | Licence | Why it is here |
|---|---|---|---|
| `ghcr.io/sebadob/rauthy:0.35.2` | `demos/fixtures/ops-2/idp.yaml` | Apache-2.0 | The test issuer, at a Service DNS name. Same version the dev compose runs. |
| `node:22-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5` | `demos/fixtures/ops-2/client-pod.yaml` | MIT | Plays the browser half of `synveda login`. |
| CloudNativePG operator | applied by the demo, version pinned in it | Apache-2.0 | Installed separately by design; the chart renders a `Cluster` for it. |

## Extensions compiled into `synveda/enterprise-postgres`

Not images, and not covered by `cargo-deny` either, so they are recorded
in the same place:

| Extension | Version | Licence |
|---|---|---|
| pgvector | PGDG `postgresql-17-pgvector` | PostgreSQL |

Epoch 3 uses no other Postgres extension. Bounded graph expansion, capture
leasing and durable operations use ordinary tenant-bound tables.
